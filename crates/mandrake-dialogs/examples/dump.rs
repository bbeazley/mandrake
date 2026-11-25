use std::path::PathBuf;
use std::time::Instant;

use mandrake_dialogs::EngineDialogs;
use mandrake_session::{FrameInput, FrontEnd, MainMenu, Screen, UiEvent};
use mandrake_tag::TagLookup;
use mandrake_tag::tag_files::{INTERFACE_800X600, dialog_tag_files};

fn point(arg: Option<String>) -> (i32, i32) {
    let arg = arg.expect("a point X,Y");
    let (x, y) = arg.split_once(',').expect("a point X,Y");
    (x.parse().expect("X"), y.parse().expect("Y"))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().expect("usage: dump OUT.png [flags]"));
    let (mut stock, mut shipped, mut time) = (false, false, 0usize);
    let mut events = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--stock" => stock = true,
            "--as" => shipped = args.next().as_deref() == Some("shipped"),
            "--time" => time = args.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            "--hold" => {
                let (x, y) = point(args.next());
                events.push(UiEvent::PointerMoved { x, y });
                events.push(UiEvent::Button {
                    left: true,
                    pressed: true,
                });
            }
            "--move" => {
                let (x, y) = point(args.next());
                events.push(UiEvent::PointerMoved { x, y });
            }
            _ => panic!("unknown argument {a}"),
        }
    }
    let game = std::env::var("MYTH2_DATA_DIR").expect("MYTH2_DATA_DIR");
    let files = dialog_tag_files(game.as_ref(), (!stock).then_some(INTERFACE_800X600));
    assert!(files.complete(), "{files:?}");
    let mut front_end = EngineDialogs::new(TagLookup::open(&files));
    let menu = MainMenu {
        new_game: true,
        multiplayer: !shipped,
        saved_games: false,
        films: false,
        tutorial: true,
        intro: false,
        preferences: !shipped,
        music: Vec::new(),
    };
    let screen = Screen::MainMenu(&menu);
    let size = front_end.canvas_size();
    let input = |now_ms, events| FrameInput {
        now_ms,
        size,
        events,
        sound_busy: false,
        shift_held: false,
    };
    let started = Instant::now();
    front_end.frame(&screen, &input(0, &[]));
    println!("opened in {:.2} ms", started.elapsed().as_secs_f64() * 1e3);
    front_end.frame(&screen, &input(0, &events));
    let bitmap = front_end.canvas();
    image::RgbaImage::from_raw(bitmap.width as u32, bitmap.height as u32, bitmap.to_rgba())
        .expect("the size matches")
        .save(&out)
        .expect("write the PNG");
    println!("{}: {}x{}", out.display(), bitmap.width, bitmap.height);
    if time > 0 {
        timing(&mut front_end, &screen, size, time);
    }
}

fn timing(front_end: &mut EngineDialogs, screen: &Screen<'_>, size: (u32, u32), count: usize) {
    let (x, y) = if size.0 == 800 {
        (270, 298)
    } else {
        (216, 238)
    };
    let events = [
        [UiEvent::PointerMoved { x, y }],
        [UiEvent::PointerMoved {
            x: 0,
            y: size.1 as i32 - 1,
        }],
    ];
    let input = |events| FrameInput {
        now_ms: 0,
        size,
        events,
        sound_busy: false,
        shift_held: false,
    };
    let press = [UiEvent::Button {
        left: true,
        pressed: true,
    }];
    front_end.frame(screen, &input(&events[0]));
    front_end.frame(screen, &input(&press));
    let mut bytes = 0;
    let started = Instant::now();
    for i in 0..count {
        let before = front_end.revision();
        front_end.frame(screen, &input(&events[(i + 1) % 2]));
        assert_ne!(before, front_end.revision(), "that was a redraw");
        bytes += front_end.canvas().to_rgba().len();
    }
    let redraw = started.elapsed().as_secs_f64() * 1e3 / count as f64;
    let started = Instant::now();
    for _ in 0..count {
        front_end.frame(screen, &input(&[]));
    }
    let still = started.elapsed().as_secs_f64() * 1e3 / count as f64;
    println!(
        "{}x{}: redraw with to_rgba {redraw:.3} ms, a still frame {still:.5} ms ({} MB/frame)",
        size.0,
        size.1,
        bytes / count / (1 << 20)
    );
}
