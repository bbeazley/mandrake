use std::path::PathBuf;
use std::time::Instant;

use mandrake_dialogs::EngineDialogs;
use mandrake_session::{Action, Catalog, FrameInput, FrontEnd, GameEvent, Progress, Session};
use mandrake_tag::TagLookup;
use mandrake_tag::mesh_links::CampaignTags;
use mandrake_tag::tag_files::{INTERFACE_800X600, dialog_tag_files};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().expect("usage: films OUT.png FOLDER_PARENT"));
    let folder = PathBuf::from(args.next().expect("usage: films OUT.png FOLDER_PARENT"));
    let (mut row, mut time) = (None, 0usize);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--row" => row = args.next().and_then(|n| n.parse().ok()),
            "--time" => time = args.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            _ => panic!("unknown argument {a}"),
        }
    }
    let game = PathBuf::from(std::env::var("MYTH2_DATA_DIR").expect("MYTH2_DATA_DIR"));
    let started = Instant::now();
    let tags = CampaignTags::open(&game);
    let listing = mandrake_session::films::list_folder(&folder, &tags);
    println!(
        "{} films listed in {:.1} ms",
        listing.films.len(),
        started.elapsed().as_secs_f64() * 1e3
    );
    for f in &listing.films {
        println!(
            "  {:?}: mesh {} art {:?} plugins {:?} name {:?} start {}",
            f.name.to_plain_utf8(),
            f.game.mesh,
            f.game.art,
            f.game.art_plugins,
            f.game.map_name.as_ref().map(|n| n.to_plain_utf8()),
            f.game.starting_locations
        );
    }
    let mut session = Session::new(Catalog::default(), Progress::default());
    session.game_event(GameEvent::FilmsListed(listing.clone()));
    session.act(Action::OpenFilms);
    session.take_requests();
    session.game_event(GameEvent::FilmsListed(listing));
    if let Some(row) = row {
        session.act(Action::ChooseFilm(row));
    }
    let files = dialog_tag_files(&game, Some(INTERFACE_800X600));
    let mut front_end = EngineDialogs::new(TagLookup::open(&files));
    let size = front_end.canvas_size();
    let input = |events| FrameInput {
        now_ms: 0,
        size,
        events,
        sound_busy: false,
        shift_held: false,
    };
    let screen = session.screen();
    let started = Instant::now();
    front_end.frame(&screen, &input(&[]));
    println!("opened in {:.2} ms", started.elapsed().as_secs_f64() * 1e3);
    let bitmap = front_end.canvas();
    image::RgbaImage::from_raw(bitmap.width as u32, bitmap.height as u32, bitmap.to_rgba())
        .expect("the size matches")
        .save(&out)
        .expect("write the PNG");
    println!("{}: {}x{}", out.display(), bitmap.width, bitmap.height);
    if time > 0 {
        let started = Instant::now();
        for _ in 0..time {
            front_end.frame(&screen, &input(&[]));
        }
        println!(
            "a still frame {:.5} ms",
            started.elapsed().as_secs_f64() * 1e3 / time as f64
        );
    }
}
