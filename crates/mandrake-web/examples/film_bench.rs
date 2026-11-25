//! Native side of [`mandrake_web::film_replay::replay`]: runs the same replay as the browser
//! bench, from the game folder on disk, to compare tick time, draws and memory with wasm.
//!
//! `cargo run --release -p mandrake-web --example film_bench -- FILM.m2rec [GAME_DIR]`
//! (GAME_DIR defaults to `MYTH2_DATA_DIR`). Prints the summary line and peak RSS.

use std::path::{Path, PathBuf};
use std::time::Instant;

use mandrake_web::film_replay;

fn main() {
    let mut args = std::env::args().skip(1);
    let film_path = PathBuf::from(
        args.next()
            .expect("usage: film_bench FILM.m2rec [GAME_DIR]"),
    );
    let game = args
        .next()
        .or_else(|| std::env::var("MYTH2_DATA_DIR").ok())
        .expect("no game dir: pass it or set MYTH2_DATA_DIR");
    let game = Path::new(&game);
    let film = std::fs::read(&film_path).unwrap();

    let start = Instant::now();
    let now = || start.elapsed().as_secs_f64() * 1000.0;

    let names = |dir: &str| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(game.join(dir))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let tags = names("tags");
    let plugins: Vec<(String, Vec<u8>)> = names("plugins")
        .into_iter()
        .map(|n| {
            use std::io::Read;
            let mut head = vec![0; mandrake_tag::SBMonoHeader::SIZE];
            let mut f = std::fs::File::open(game.join("plugins").join(&n)).unwrap();
            let got = f.read(&mut head).unwrap_or(0);
            head.truncate(got);
            (n, head)
        })
        .collect();
    let film_plugins = film_replay::film_plugins(&film).unwrap();
    let order = film_replay::order_tag_files(
        &tags,
        &plugins,
        &film_plugins,
        &mandrake_tag::tagset::chosen_metaserver(game),
    )
    .unwrap();

    // The same list as the engine's, from the directory.
    let refs: Vec<&str> = film_plugins.iter().map(String::as_str).collect();
    let want: Vec<PathBuf> = mandrake_tag::tag_files::engine_tag_files(game, &refs).paths;
    let got: Vec<PathBuf> = order.iter().map(|p| game.join(p)).collect();
    assert_eq!(got, want, "order_tag_files differs from engine_tag_files");

    let t0 = now();
    let mut bytes = 0usize;
    let archives: Vec<Vec<u8>> = order
        .iter()
        .map(|p| {
            let b = std::fs::read(game.join(p)).unwrap();
            bytes += b.len();
            b
        })
        .collect();
    let read_ms = now() - t0;
    eprintln!("files: {}", order.join(", "));
    let r = film_replay::replay(&film, archives, &now).unwrap();
    let hwm = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(|l| l.split_whitespace().nth(1).unwrap_or("0").to_string())
        })
        .unwrap_or_default();
    println!(
        "native {} files={} file_bytes={bytes} read_ms={read_ms:.1} peak_rss_kb={hwm}",
        r.summary(),
        order.len()
    );
}
