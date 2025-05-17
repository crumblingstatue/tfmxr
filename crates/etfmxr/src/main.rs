use {
    clap::Parser,
    cpal::{
        BufferSize, SampleRate, StreamConfig,
        traits::{DeviceTrait, HostTrait, StreamTrait},
    },
    eframe::egui,
    egui_file_dialog::FileDialog,
    std::{
        ops::ControlFlow,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc::Sender,
        },
    },
    tfmxr::{PlayerBuilder, PlayerCmd},
};

#[derive(clap::Parser)]
struct Args {
    mdat_path: Option<String>,
    #[arg(short = 's', long)]
    smpl_path: Option<String>,
    /// Song index
    #[arg(short = 't', long, default_value = "0")]
    song: u8,
    #[arg(short = 'r', long, default_value = "44100")]
    sample_rate: u32,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "eTFMXr",
        native_options,
        Box::new(|cc| Ok(Box::new(EtfmxrApp::new(cc, args)))),
    )
    .unwrap();
    Ok(())
}

struct EtfmxrApp {
    file_dialog: FileDialog,
    cpal_host: cpal::Host,
    cpal_stream: Option<cpal::Stream>,
    pl_msg_send: Option<Sender<PlayerCmd>>,
    /// Buffer of rendered samples
    rendered_buffer: Arc<Mutex<Vec<i16>>>,
    stop_playback: Arc<AtomicBool>,
    player_status: Arc<Mutex<PlayerStatus>>,
}

#[derive(Default)]
pub struct PlayerStatus {
    current_song_idx: u8,
}

impl EtfmxrApp {
    fn new(_cc: &eframe::CreationContext<'_>, mut cli_args: Args) -> Self {
        let mdat_path = cli_args.mdat_path.take();
        let mut this = Self {
            file_dialog: FileDialog::new(),
            cpal_host: cpal::default_host(),
            cpal_stream: None,
            pl_msg_send: None,
            rendered_buffer: Arc::new(Mutex::new(Vec::new())),
            stop_playback: Arc::new(AtomicBool::new(false)),
            player_status: Arc::default(),
        };
        if let Some(path) = mdat_path {
            this.play_song(path.into());
        }
        this
    }

    fn play_song(&mut self, path: std::path::PathBuf) {
        let mut player = match PlayerBuilder::new(path.to_str().unwrap()).build() {
            Ok(player) => player,
            Err(e) => {
                log::error!("Error playing song: {e}");
                return;
            }
        };
        let dev = self.cpal_host.default_output_device().unwrap();
        let (send, recv) = std::sync::mpsc::sync_channel(1);
        let (pl_send, pl_recv) = std::sync::mpsc::channel();
        self.pl_msg_send = Some(pl_send);
        let player_status = self.player_status.clone();
        std::thread::spawn(move || {
            player.play(|new, player| {
                {
                    let mut status = player_status.lock().unwrap();
                    status.current_song_idx = player.current_song_index();
                }
                if let Err(e) = send.send(new.to_vec()) {
                    log::error!("Send error: {e}");
                    return ControlFlow::Break(());
                }
                let msg = pl_recv.try_recv().ok();
                if msg.as_ref().is_some() {
                    log::debug!("Command: {msg:?}");
                }
                ControlFlow::Continue(msg)
            });
        });
        let rendered_buffer = self.rendered_buffer.clone();
        let stop_playback = self.stop_playback.clone();
        let stream = dev
            .build_output_stream(
                &StreamConfig {
                    channels: 2,
                    sample_rate: SampleRate(44_100),
                    buffer_size: BufferSize::Default,
                },
                move |dat: &mut [i16], _| {
                    let mut rendered_buffer = rendered_buffer.lock().unwrap();
                    while rendered_buffer.len() < dat.len() {
                        match recv.recv() {
                            Ok(dat) => {
                                rendered_buffer.extend(dat);
                            }
                            Err(e) => {
                                eprintln!("Error receiving data: {e}");
                                stop_playback.store(true, Ordering::Relaxed);
                                return;
                            }
                        }
                    }
                    let new = rendered_buffer.split_off(dat.len());
                    dat.copy_from_slice(&rendered_buffer);
                    *rendered_buffer = new;
                },
                |err| {
                    dbg!(err);
                },
                None,
            )
            .unwrap();
        stream.play().unwrap();
        self.cpal_stream = Some(stream);
    }
}

impl eframe::App for EtfmxrApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        if self.stop_playback.load(Ordering::Relaxed) {
            self.cpal_stream = None;
            self.pl_msg_send = None;
        }
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            let (ctrl, key_o) = ui.input(|inp| (inp.modifiers.ctrl, inp.key_pressed(egui::Key::O)));
            if ui.button("Open file").clicked() || (ctrl && key_o) {
                self.file_dialog.pick_file();
            }
            let status = self.player_status.lock().unwrap();
            let active_track = status.current_song_idx;
            ui.label(format!("Active track: {active_track}"));
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| match self.pl_msg_send.as_mut() {
                Some(pl_msg_send) => {
                    if ui.button("⏮").on_hover_text("Previous track").clicked()
                        && let Err(e) = pl_msg_send.send(PlayerCmd::Prev)
                    {
                        log::error!("Failed to send message ({:?}) to player: {e}", e.0);
                    }
                    let label = if self.cpal_stream.is_some() {
                        "⏹"
                    } else {
                        "▶"
                    };
                    if ui.button(label).clicked() {
                        self.stop_playback.store(true, Ordering::Relaxed);
                    }
                    if ui.button("⏭").on_hover_text("Next track").clicked()
                        && let Err(e) = pl_msg_send.send(PlayerCmd::Next)
                    {
                        log::error!("Failed to send message ({:?}) to player: {e}", e.0);
                    }
                }
                None => {
                    ui.label("No connection to player");
                }
            });
        });
        self.file_dialog.update(ctx);
        if let Some(path) = self.file_dialog.take_picked() {
            if let Some(parent) = path.parent() {
                self.file_dialog.config_mut().initial_directory = parent.to_owned();
            }
            self.play_song(path);
        }
    }
}
