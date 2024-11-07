use {
    anyhow::Context,
    clap::Parser,
    console::Term,
    std::{io::Write as _, ops::ControlFlow},
    tfmxr::{PlayerBuilder, PlayerCmd},
};

#[derive(clap::Parser)]
struct Args {
    mdat_path: String,
    #[arg(short = 's', long)]
    smpl_path: Option<String>,
    /// Song index
    #[arg(short = 't', long, default_value = "0")]
    song: u8,
}

enum Msg {
    Cmd(PlayerCmd),
    End,
}

fn main() {
    let args = Args::parse();
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
    let begin = std::time::Instant::now();
    let mut total = 0;
    let term = Term::stderr();
    let (send, recv) = std::sync::mpsc::channel();
    let th_handle = std::thread::spawn(move || {
        let mut builder = PlayerBuilder::new(args.mdat_path);
        if let Some(smpl) = args.smpl_path {
            builder.smpl_file(smpl);
        }
        let mut player = builder
            .starting_subsong(args.song)
            .build()
            .context("Failed to create player")
            .unwrap();
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        player.play(|samples, _player| {
            total += samples.len();
            eprint!(
                "[tfmxr] ({:.02}) {} bytes rendered (approx {} seconds)\r",
                begin.elapsed().as_secs_f32(),
                total,
                // 44khz, 2 channels, s16le
                total / (44_100 * 2 * 2)
            );
            match lock.write_all(bytemuck::cast_slice(samples)) {
                Ok(()) => {
                    if let Ok(msg) = recv.try_recv() {
                        match msg {
                            Msg::Cmd(cmd) => return ControlFlow::Continue(Some(cmd)),
                            Msg::End => return ControlFlow::Break(()),
                        }
                    }
                    ControlFlow::Continue(None)
                }
                Err(e) => {
                    eprintln!("Error writing samples: {e}");
                    ControlFlow::Break(())
                }
            }
        });
    });
    loop {
        match term.read_char() {
            Ok(ch) => match ch {
                '<' => send.send(Msg::Cmd(PlayerCmd::Prev)).unwrap(),
                '>' => send.send(Msg::Cmd(PlayerCmd::Next)).unwrap(),
                'r' => send.send(Msg::Cmd(PlayerCmd::RestartSong)).unwrap(),
                'l' => send
                    .send(Msg::Cmd(PlayerCmd::ToggleLoopCurrentSong))
                    .unwrap(),
                'q' => {
                    send.send(Msg::End).unwrap();
                    th_handle.join().unwrap();
                    break;
                }
                'b' => {
                    send.send(Msg::Cmd(PlayerCmd::ToggleBlend)).unwrap();
                }
                '1'..'9' => {
                    send.send(Msg::Cmd(PlayerCmd::ToggleCh(ch as u8 - b'1')))
                        .unwrap();
                }
                _ => {}
            },
            Err(e) => eprintln!("{e}"),
        }
    }
}
