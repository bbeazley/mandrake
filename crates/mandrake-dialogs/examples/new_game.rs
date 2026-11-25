use std::path::PathBuf;

use mandrake_dialogs::EngineDialogs;
use mandrake_session::screen::Screen;
use mandrake_session::{Action, Catalog, FrameInput, FrontEnd, Progress, Session, UiEvent};
use mandrake_tag::TagLookup;
use mandrake_tag::mesh_links::CampaignTags;
use mandrake_tag::tag_files::{INTERFACE_800X600, dialog_tag_files};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().expect("usage: new_game OUT.png [flags]"));
    let (mut shift, mut stock, mut shipped, mut base_only) = (false, false, false, false);
    let mut clicks = Vec::new();
    let mut time = 0usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--shift" => shift = true,
            "--stock" => stock = true,
            "--shipped" => shipped = true,
            "--base-only" => base_only = true,
            "--time" => time = args.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            "--click" => {
                let arg = args.next().expect("a point X,Y");
                let (x, y) = arg.split_once(',').expect("a point X,Y");
                clicks.push((x.parse::<i32>().unwrap(), y.parse::<i32>().unwrap()));
            }
            _ => panic!("unknown argument {a}"),
        }
    }
    let game = std::env::var("MYTH2_DATA_DIR").expect("MYTH2_DATA_DIR");
    let files = dialog_tag_files(game.as_ref(), (!stock).then_some(INTERFACE_800X600));
    let mut front_end = EngineDialogs::new(TagLookup::open(&files));
    let mut catalog = Catalog::from_tags(&CampaignTags::open(game.as_ref()));
    if base_only {
        catalog.campaigns.truncate(1);
    }
    let mut session = Session::new(catalog, Progress::default());
    session.act(Action::NewGame { all_levels: shift });
    let size = front_end.canvas_size();
    let mut now_ms = 0;
    let mut run = |events: &[UiEvent], session: &mut Session, front_end: &mut EngineDialogs| {
        let Screen::NewGame(data) = session.screen() else {
            panic!("not on New Game");
        };
        let mut data = data.clone();
        if !shipped {
            data.tagsets = true;
            data.gameplay_options = true;
        }
        now_ms += 100;
        let input = FrameInput {
            now_ms,
            size,
            events,
            sound_busy: false,
            shift_held: shift,
        };
        for action in front_end.frame(&Screen::NewGame(&data), &input).actions {
            session.act(action);
        }
    };
    run(&[], &mut session, &mut front_end);
    for (x, y) in clicks {
        let press = [
            UiEvent::PointerMoved { x, y },
            UiEvent::Button {
                left: true,
                pressed: true,
            },
        ];
        run(&press, &mut session, &mut front_end);
        run(&[], &mut session, &mut front_end);
        let release = [UiEvent::Button {
            left: true,
            pressed: false,
        }];
        run(&release, &mut session, &mut front_end);
        for _ in 0..5 {
            run(&[], &mut session, &mut front_end);
        }
    }
    if time > 0 {
        let start = std::time::Instant::now();
        for i in 0..time {
            session.act(Action::ChooseDifficulty((i % 5) as i16));
            run(&[], &mut session, &mut front_end);
        }
        let redraw = start.elapsed().as_secs_f64() * 1e3 / time as f64;
        let start = std::time::Instant::now();
        for _ in 0..time {
            run(&[], &mut session, &mut front_end);
        }
        let still = start.elapsed().as_secs_f64() * 1e3 / time as f64;
        println!(
            "a redraw {redraw:.3} ms ({:.1}% of 4 ms), a still frame {still:.4} ms",
            redraw / 4.0 * 100.0
        );
    }
    let canvas = front_end.canvas();
    image::RgbaImage::from_raw(canvas.width as u32, canvas.height as u32, canvas.to_rgba())
        .expect("the canvas")
        .save(&out)
        .expect("the file");
}
