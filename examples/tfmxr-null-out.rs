//! For miri testing

use {
    std::ops::ControlFlow,
    tfmxr::{PlayerBuilder, PlayerCmd},
};

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_env("RUST_LOG")
        .init();
    let mut player = PlayerBuilder::new(std::env::args().nth(1).expect("Need mdat file"))
        .build()
        .unwrap();
    let begin = std::time::Instant::now();
    eprintln!("Let's-a-go!");
    let mut song_total = 0;
    player.play(|samples, _player| {
        song_total += samples.len();
        let realtime = begin.elapsed().as_secs_f32();
        // 44khz, 2 channels, s16le
        let render_elapsed = song_total / (44_100 * 2 * 2);
        eprint!(
            "[tfmxr] ({realtime:.02}) {song_total} bytes rendered (approx {render_elapsed} seconds)\r",
        );
        // After 2 hours of rendered song, it's probably enough testing
        if render_elapsed > 7200 {
            eprintln!("Enough. Requesting next song...");
            song_total = 0;
            ControlFlow::Continue(Some(PlayerCmd::Next))
        } else {
            ControlFlow::Continue(None)
        }
    });
}
