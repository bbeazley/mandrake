//! Decodes a Smacker file and prints its header and decode speed:
//! `cargo run --release -p mandrake-smacker --example smk-bench -- film.smk`.

use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: smk-bench <film.smk>");
    let bytes = std::fs::read(&path).expect("read film");
    let smk = mandrake_smacker::Smacker::open(&bytes).expect("open film");
    println!("{:?}", smk.header());
    let start = Instant::now();
    let mut dec = smk.decoder();
    let (mut frames, mut samples) = (0u32, 0usize);
    while let Some(f) = dec.next_frame().expect("decode") {
        frames += 1;
        samples += f.audio.iter().map(|c| c.samples.len()).sum::<usize>();
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{frames} frames, {samples} audio samples in {:.3} s: {:.0} frames/s",
        secs,
        frames as f64 / secs
    );
}
