use {
    clap::Parser,
    cpal::{
        traits::{DeviceTrait, HostTrait, StreamTrait},
        BufferSize, SampleRate, StreamConfig,
    },
    eframe::egui,
    egui_file_dialog::FileDialog,
    egui_inspect::Inspect,
    std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    tfmxr::{
        rendering::{available_sound_data, try_to_makeblock, AudioCtx, BUFSIZE},
        song, PlayerBuilder, TfmxPlayer,
    },
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
    /// Buffer of rendered samples
    rendered_buffer: Arc<Mutex<Vec<i16>>>,
    stop_playback: Arc<AtomicBool>,
    player: Option<Arc<Mutex<TfmxPlayer>>>,
    audio: Arc<Mutex<AudioCtx>>,
}

impl EtfmxrApp {
    fn new(_cc: &eframe::CreationContext<'_>, mut cli_args: Args) -> Self {
        let mdat_path = cli_args.mdat_path.take();
        let mut this = Self {
            file_dialog: FileDialog::new(),
            cpal_host: cpal::default_host(),
            cpal_stream: None,
            rendered_buffer: Arc::new(Mutex::new(Vec::new())),
            stop_playback: Arc::new(AtomicBool::new(false)),
            player: None,
            audio: Arc::new(Mutex::new(AudioCtx::new())),
        };
        if let Some(path) = mdat_path {
            this.play_song(path.into());
        }
        this
    }

    fn play_song(&mut self, path: std::path::PathBuf) {
        self.stop_playback.store(false, Ordering::SeqCst);
        let mut player = match PlayerBuilder::new(path.to_str().unwrap()).build() {
            Ok(player) => player,
            Err(e) => {
                log::error!("Error playing song: {e}");
                return;
            }
        };
        player.tfmx.init();
        song::start_song(player.song_idx, 0, &player.header, &mut player.tfmx);
        let player = Arc::new(Mutex::new(player));
        self.player = Some(player.clone());
        let dev = self.cpal_host.default_output_device().unwrap();
        let rendered_buffer = self.rendered_buffer.clone();
        let audio = self.audio.clone();
        let stream = dev
            .build_output_stream(
                &StreamConfig {
                    channels: 2,
                    sample_rate: SampleRate(44_100),
                    buffer_size: BufferSize::Default,
                },
                move |dat: &mut [i16], _| {
                    let mut player = player.lock().unwrap();
                    let player = &mut *player;
                    let mut audio = audio.lock().unwrap();

                    let mut rendered_buffer = rendered_buffer.lock().unwrap();
                    while try_to_makeblock(
                        &player.header,
                        &mut audio,
                        &mut player.tfmx,
                        &player.sample_buf,
                        player.ch_on,
                    ) != Some(0)
                    {}
                    while rendered_buffer.len() < dat.len() {
                        if try_to_makeblock(
                            &player.header,
                            &mut audio,
                            &mut player.tfmx,
                            &player.sample_buf,
                            player.ch_on,
                        )
                        .is_some()
                        {
                            let mut total_len = available_sound_data(&audio);
                            while total_len > 0 {
                                let mut len = total_len;
                                if audio.btail + len > BUFSIZE {
                                    len = BUFSIZE - audio.btail;
                                }
                                let end_idx = (audio.bhead + len).min(audio.buf.len());

                                let buf = &audio.buf[audio.bhead..end_idx];
                                rendered_buffer.extend_from_slice(buf);

                                audio.btail = (audio.btail + len) % BUFSIZE;

                                total_len -= len;
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
        }
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            let (ctrl, key_o) = ui.input(|inp| (inp.modifiers.ctrl, inp.key_pressed(egui::Key::O)));
            if ui.button("Open file").clicked() || (ctrl && key_o) {
                self.file_dialog.select_file();
            }
            if let Some(player) = &self.player {
                let active_track = player.lock().unwrap().song_idx;
                ui.label(format!("Active track: {active_track}"));
            }
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(player) = &mut self.player else {
                ui.label("Player inactive");
                return;
            };
            let player = &mut *player.lock().unwrap();
            ui.horizontal(|ui| {
                if ui.button("⏮").on_hover_text("Previous track").clicked() {
                    player.song_idx -= 1;
                    player.tfmx = player.clean_tfmx.clone();
                    player.tfmx.init();
                    song::start_song(player.song_idx, 0, &player.header, &mut player.tfmx);
                }
                let label = if self.cpal_stream.is_some() {
                    "⏹"
                } else {
                    "▶"
                };
                if ui.button(label).clicked() {
                    self.stop_playback.store(true, Ordering::Relaxed);
                }
                if ui.button("⏭").on_hover_text("Next track").clicked() {
                    player.song_idx += 1;
                    player.tfmx = player.clean_tfmx.clone();
                    player.tfmx.init();
                    song::start_song(player.song_idx, 0, &player.header, &mut player.tfmx);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.label("What the fuck");
                    player.inspect_mut(ui, 0);
                });
        });
        self.file_dialog.update(ctx);
        if let Some(path) = self.file_dialog.take_selected() {
            if let Some(parent) = path.parent() {
                self.file_dialog.config_mut().initial_directory = parent.to_owned();
            }
            self.play_song(path);
        }
    }
}
