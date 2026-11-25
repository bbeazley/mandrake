#![allow(dead_code)]

#[path = "../../../mandrake-common/tests/support/require_data.rs"]
pub mod test_data;

use std::path::{Path, PathBuf};

use mandrake_common::FourCC;
use mandrake_dialogs::EngineDialogs;
use mandrake_session::films::{FilmTags, parse_game, read_header, short_name};
use mandrake_session::{FilmDetails, Films, FrameInput, FrontEnd, Screen, Text, UiEvent};
use mandrake_tag::font::Bitmap32;
use mandrake_tag::mesh_links::CampaignTags;
use mandrake_tag::tag_files::{INTERFACE_800X600, dialog_tag_files, engine_tag_files};
use mandrake_tag::{Rect, TagLookup};

pub fn game() -> Option<PathBuf> {
    let game = test_data::require_game()?;
    test_data::require_data(game.join("plugins").join(INTERFACE_800X600)).then_some(game)
}

pub fn game_if(ok: impl FnOnce(&Path) -> bool, missing: &str) -> Option<PathBuf> {
    let game = test_data::require_game()?;
    let ok = ok(&game);
    if !ok {
        eprintln!("Notice: {missing}, skipping");
    }
    ok.then_some(game)
}

pub fn game_with_film() -> Option<PathBuf> {
    game_if(
        |game| {
            game.join("plugins").join(INTERFACE_800X600).is_file()
                && game.join("recordings/broken_path_victory.m2rec").is_file()
        },
        "the plugin or the film isn't installed",
    )
}

pub fn front_end(game: &Path) -> EngineDialogs {
    EngineDialogs::new(TagLookup::open(&dialog_tag_files(
        game,
        Some(INTERFACE_800X600),
    )))
}

pub fn front_end_with_art(game: &Path) -> EngineDialogs {
    let files = dialog_tag_files(game, Some(INTERFACE_800X600));
    let game_dir = game.to_path_buf();
    EngineDialogs::new(TagLookup::open(&files)).with_campaign_art(Box::new(move |plugins, id| {
        let names: Vec<&str> = plugins.iter().map(String::as_str).collect();
        TagLookup::open(&engine_tag_files(&game_dir, &names)).get(FourCC(*b".256"), id)
    }))
}

pub fn shots(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../screenshots")
        .join(sub)
}

pub fn reference_shot(sub: &str, file: &str) -> image::RgbImage {
    image::open(shots(sub).join(file))
        .expect("the engine's frame")
        .to_rgb8()
}

pub fn dump(ours: &Bitmap32, file: &str) {
    if let Some(dir) = std::env::var_os("M2_DIALOG_DUMP") {
        image::RgbaImage::from_raw(ours.width as u32, ours.height as u32, ours.to_rgba())
            .expect("the canvas")
            .save(Path::new(&dir).join(file))
            .expect("the file");
    }
}

pub fn differing_in(ours: &Bitmap32, theirs: &image::RgbImage, rect: Rect) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for y in rect.top..rect.bottom {
        for x in rect.left..rect.right {
            let p = ours.get(x, y);
            let mine = [(p >> 16) as u8, (p >> 8) as u8, p as u8];
            if theirs.get_pixel(x as u32, y as u32).0 != mine {
                out.push((x, y));
            }
        }
    }
    out
}

pub fn differing(
    front_end: &EngineDialogs,
    sub: &str,
    frame: &str,
    file: &str,
    rect: Rect,
) -> Vec<(i32, i32)> {
    let ours = front_end.canvas();
    dump(ours, &format!("ours-{frame}.png"));
    differing_in(ours, &reference_shot(sub, file), rect)
}

pub fn extent(diff: &[(i32, i32)]) -> Option<Rect> {
    let (x0, y0) = diff.first()?;
    let mut r = Rect::new(*y0, *x0, y0 + 1, x0 + 1);
    for &(x, y) in diff {
        r = Rect::new(
            r.top.min(y),
            r.left.min(x),
            r.bottom.max(y + 1),
            r.right.max(x + 1),
        );
    }
    Some(r)
}

pub fn dialog_rect(front_end: &EngineDialogs, id: FourCC) -> Rect {
    front_end
        .stack()
        .iter()
        .find(|d| d.id == id)
        .expect("the dialog is open")
        .bounds()
}

pub fn frame(front_end: &mut EngineDialogs, screen: &Screen<'_>, now_ms: u64, events: &[UiEvent]) {
    let input = FrameInput {
        now_ms,
        size: front_end.canvas_size(),
        events,
        sound_busy: false,
        shift_held: false,
    };
    front_end.frame(screen, &input);
}

pub fn broken_path(game: &Path, built: bool) -> Films {
    let bytes = std::fs::read(game.join("recordings/broken_path_victory.m2rec")).unwrap();
    let mut film = parse_game(read_header(&bytes).expect("a recording"));
    FilmTags::new(&CampaignTags::open(game)).look_up(&mut film);
    let names = [
        "bagrada2025_04_27__18_45_17_24",
        "broken_path_victory",
        "Last Recording",
        "level3",
        "MDL",
        "salvation_def_new",
        "willow_vic_new",
    ];
    Films {
        names: names.iter().map(|n| Text(n.as_bytes().to_vec())).collect(),
        selected: Some(1),
        details: Some(FilmDetails {
            name: short_name(&Text(b"broken_path_victory".to_vec())),
            game: film,
        }),
        can_view: true,
        find_plugins: false,
        last_film: true,
        save_last: built,
        delete: built,
        rename: built,
        save_film: None,
        delete_film: None,
        rename_film: None,
    }
}
