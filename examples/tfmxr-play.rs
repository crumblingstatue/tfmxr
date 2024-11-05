use {
    anyhow::Context,
    clap::Parser,
    console::Term,
    std::{
        io::Write as _,
        ops::ControlFlow,
        process::{Command, Stdio},
    },
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
    #[arg(short = 'r', long, default_value = "44100")]
    sample_rate: u32,
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
    let term = Term::stdout();
    let (send, recv) = std::sync::mpsc::channel();
    let th_handle = std::thread::spawn(move || {
        let mut builder = PlayerBuilder::new(args.mdat_path);
        if let Some(smpl) = args.smpl_path {
            builder.smpl_file(smpl);
        }
        let mut player = builder
            .starting_subsong(args.song)
            .sample_rate(args.sample_rate)
            .build()
            .context("Failed to create player")
            .unwrap();
        let mut cmd = Command::new("aplay");
        cmd.args([
            "-f",
            "s16_le",
            "-r",
            &args.sample_rate.to_string(),
            "-c",
            "2",
        ]);
        cmd.stdin(Stdio::piped());
        let mut handle = cmd.spawn().unwrap();
        let Some(stdin) = &mut handle.stdin else {
            eprintln!("failed to get stdin for aplay");
            handle.wait().unwrap();
            return;
        };
        player.play(|samples| {
            total += samples.len();
            eprint!(
                "[tfmxr] ({:.02}) {} bytes rendered (approx {} seconds)\r",
                begin.elapsed().as_secs_f32(),
                total,
                // 2 channels, s16le
                total / (args.sample_rate as usize * 2 * 2)
            );
            match stdin.write_all(bytemuck::cast_slice(samples)) {
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
