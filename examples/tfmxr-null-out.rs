//! For miri testing

use {std::ops::ControlFlow, tfmxr::PlayerBuilder};

fn main() {
    let mut player = PlayerBuilder::new(std::env::args().nth(1).expect("Need mdat file"))
        .build()
        .unwrap();
    let begin = std::time::Instant::now();
    eprintln!("Let's-a-go!");
    let mut total = 0;
    player.play(|samples| {
        total += samples.len();
        eprint!(
            "[tfmxr] ({:.02}) {} bytes rendered (approx {} seconds)\r",
            begin.elapsed().as_secs_f32(),
            total,
            // 44khz, 2 channels, s16le
            total / (44_100 * 2 * 2)
        );
        ControlFlow::Break(())
    });
}
