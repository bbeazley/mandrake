mod common;

use std::path::{Path, PathBuf};

use common::{catalog, text};
use mandrake_common::FourCC;
use mandrake_session::{Action, Films, GameEvent, Progress, Request, Screen, Session};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("FILM_MANAGE_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")), PathBuf::from);
    let game = base.join(name);
    let _ = std::fs::remove_dir_all(&game);
    std::fs::create_dir_all(game.join("recordings")).unwrap();
    game
}

fn write_film(game: &Path, file: &str, header_name: &[u8]) {
    let mut data: Vec<u8> = (0..0xa28 + 16).map(|i| (i % 7) as u8).collect();
    data[0..2].copy_from_slice(&3u16.to_be_bytes());
    let bytes = mandrake_rec::writer::film_file(header_name, FourCC(*b"test"), &data);
    std::fs::write(game.join("recordings").join(file), bytes).unwrap();
}

fn files(game: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(game.join("recordings"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn run(s: &mut Session, game: &Path) {
    let tags = mandrake_tag::mesh_links::CampaignTags::open(game);
    loop {
        let requests = s.take_requests();
        if requests.is_empty() {
            return;
        }
        for request in requests {
            match request {
                Request::ListFilms => s.game_event(GameEvent::FilmsListed(
                    mandrake_session::films::list_folder(game, &tags),
                )),
                Request::DeleteFilm { file_name } => {
                    mandrake_session::films::delete_film(game, &file_name).unwrap();
                }
                Request::RenameFilm { file_name, name } => {
                    let now =
                        mandrake_session::films::rename_film(game, &file_name, &name.0).unwrap();
                    s.game_event(GameEvent::FilmRenamed(now));
                }
                other => panic!("not asked for here: {other:?}"),
            }
        }
    }
}

fn films(s: &Session) -> Films {
    match s.screen() {
        Screen::Films(films) => films.clone(),
        other => panic!("not Saved Films: {other:?}"),
    }
}

fn chosen(s: &Session) -> Option<String> {
    let f = films(s);
    f.selected.map(|row| f.names[row].to_plain_utf8())
}

fn open(game: &Path) -> Session {
    for (file, name) in [
        ("broken_path_victory.m2rec", &b"broken_path_victory"[..]),
        ("Last Recording", b"Last Recording"),
        ("level3.m2rec", b"level3"),
        ("MDL.m2rec", b"MDL"),
        ("willow_vic_new.m2rec", b"willow_vic_new"),
        (
            "bagrada2025_04_27__18_45_17_24.m2rec",
            b"bagrada2025_04_27__18_45_17_24",
        ),
    ] {
        write_film(game, file, name);
    }
    let mut s = Session::new(catalog(), Progress::default());
    let tags = mandrake_tag::mesh_links::CampaignTags::open(game);
    s.game_event(GameEvent::FilmsListed(
        mandrake_session::films::list_folder(game, &tags),
    ));
    s.act(Action::OpenFilms);
    run(&mut s, game);
    assert_eq!(
        chosen(&s).as_deref(),
        Some("bagrada2025_04_27__18_45_17_24")
    );
    s
}

fn choose(s: &mut Session, name: &str) {
    let row = films(s)
        .names
        .iter()
        .position(|n| n.0 == name.as_bytes())
        .unwrap();
    s.act(Action::ChooseFilm(row));
}

#[test]
fn delete_film_asks_then_deletes() {
    let game = scratch("film-manage-delete");
    let mut s = open(&game);
    assert!(films(&s).delete, "a film is chosen");
    s.act(Action::DeleteFilm);
    let question = films(&s).delete_film.expect("the question");
    assert_eq!(question.name, text("bagrada2025_04_27__18_45_17_24"));

    choose(&mut s, "MDL");
    assert_eq!(
        chosen(&s).as_deref(),
        Some("bagrada2025_04_27__18_45_17_24"),
        "the question is modal"
    );
    s.act(Action::Cancel);
    assert!(films(&s).delete_film.is_none());
    assert!(s.take_requests().is_empty(), "Cancel deletes nothing");

    choose(&mut s, "MDL");
    s.act(Action::DeleteFilm);
    assert_eq!(films(&s).delete_film.unwrap().name, text("MDL.m2rec"));
    s.act(Action::ConfirmDeleteFilm);
    run(&mut s, &game);
    assert!(!files(&game).contains(&"MDL.m2rec".to_string()));
    assert_eq!(chosen(&s).as_deref(), Some("level3"), "the row above");
    assert!(films(&s).delete_film.is_none());

    choose(&mut s, "bagrada2025_04_27__18_45_17_24");
    s.act(Action::DeleteFilm);
    s.act(Action::ConfirmDeleteFilm);
    run(&mut s, &game);
    assert_eq!(chosen(&s).as_deref(), Some("broken_path_victory"));
    assert_eq!(
        files(&game),
        [
            "Last Recording",
            "broken_path_victory.m2rec",
            "level3.m2rec",
            "willow_vic_new.m2rec"
        ]
    );
}

#[test]
fn file_names_are_only_names() {
    let game = scratch("film-manage-paths");
    write_film(&game, "a.m2rec", b"a");
    assert!(mandrake_session::films::delete_film(&game, b"../a.m2rec").is_err());
    assert!(mandrake_session::films::delete_film(&game, b"..").is_err());
    assert!(mandrake_session::films::rename_film(&game, b"../x", b"y").is_err());
    assert_eq!(files(&game), ["a.m2rec"]);
    assert_eq!(
        mandrake_session::films::rename_film(&game, b"a.m2rec", b"../y").unwrap(),
        b"___y.m2rec"
    );
    assert_eq!(files(&game), ["___y.m2rec"]);
}
