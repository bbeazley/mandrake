#[path = "../../mandrake-common/tests/support/require_data.rs"]
mod test_data;

mod common;

use std::path::PathBuf;

use common::{BROKEN_PATH, catalog, text};
use mandrake_common::FourCC;
use mandrake_session::films::{FilmGame, FilmPlugin};
use mandrake_session::{
    Action, FilmFile, FilmFolder, GameEvent, Progress, Request, Screen, Session,
};

fn film(file: &str, mesh: FourCC, found: bool) -> FilmFile {
    FilmFile {
        path: PathBuf::from("recordings").join(file),
        file_name: file.as_bytes().to_vec(),
        name: text(file.trim_end_matches(".m2rec")),
        game: FilmGame {
            mode: 2,
            game_type: 13,
            mesh,
            difficulty: 3,
            plugins: vec![FilmPlugin {
                name: "Patch 1.8.5 Maps".to_owned(),
                checksum: 7,
            }],
            map_name: found.then(|| text("Down a Broken Path")),
            plugins_missing: !found,
            plugins_present: found,
            ..FilmGame::default()
        },
    }
}

fn folder() -> FilmFolder {
    FilmFolder {
        films: vec![
            film("broken_path_victory.m2rec", BROKEN_PATH, true),
            film("MDL.m2rec", FourCC(*b"mdl "), false),
        ],
        last: Some(film("Last Recording", BROKEN_PATH, true)),
        any: true,
        last_exists: true,
        names: vec![
            text("broken_path_victory"),
            text("Last Recording"),
            text("MDL"),
        ],
        file_names: vec![
            b"broken_path_victory.m2rec".to_vec(),
            b"Last Recording".to_vec(),
            b"MDL.m2rec".to_vec(),
        ],
        last_name: text("Last Recording"),
        untitled: text("Untitled Replay"),
    }
}

fn answer(session: &mut Session, folder: &FilmFolder) -> Vec<Request> {
    let mut rest = Vec::new();
    let mut requests = session.take_requests();
    while !requests.is_empty() {
        for request in requests {
            match request {
                Request::ListFilms => session.game_event(GameEvent::FilmsListed(folder.clone())),
                other => rest.push(other),
            }
        }
        requests = session.take_requests();
    }
    rest
}

fn at_menu(folder: &FilmFolder) -> Session {
    let mut s = Session::new(catalog(), Progress::default());
    s.game_event(GameEvent::FilmsListed(folder.clone()));
    s
}

fn films(session: &Session) -> mandrake_session::Films {
    match session.screen() {
        Screen::Films(films) => films.clone(),
        other => panic!("not Saved Films: {other:?}"),
    }
}

#[test]
fn saved_films_is_off_until_the_folder_has_a_recording() {
    let mut s = at_menu(&FilmFolder::default());
    let Screen::MainMenu(menu) = s.screen() else {
        panic!("the menu");
    };
    assert!(!menu.films);
    s.act(Action::OpenFilms);
    assert!(matches!(s.screen(), Screen::MainMenu(_)), "a disabled item");
    s.game_event(GameEvent::FilmsListed(folder()));
    let Screen::MainMenu(menu) = s.screen() else {
        panic!("the menu");
    };
    assert!(menu.films);
}

#[test]
fn replay_last_film_plays_the_last_recording() {
    let folder = folder();
    let mut s = at_menu(&folder);
    s.act(Action::OpenFilms);
    answer(&mut s, &folder);
    s.act(Action::ReplayLastFilm);
    let requests = s.take_requests();
    let [Request::LoadGame(start)] = requests.as_slice() else {
        panic!("one load: {requests:?}");
    };
    assert_eq!(start.film, Some(PathBuf::from("recordings/Last Recording")));

    s.game_event(GameEvent::Loaded);
    s.act(Action::OpenPause);
    s.act(Action::QuitToMenu);
    s.take_requests();
    s.act(Action::Tutorial);
    let requests = s.take_requests();
    let start = requests.iter().find_map(|r| match r {
        Request::LoadGame(start) => Some(start),
        _ => None,
    });
    assert_eq!(start.expect("a load").film, None);
}

#[test]
fn no_last_recording_no_replay() {
    let folder = FilmFolder {
        last: None,
        ..folder()
    };
    let mut s = at_menu(&folder);
    s.act(Action::OpenFilms);
    answer(&mut s, &folder);
    assert!(!films(&s).last_film);
    s.act(Action::ReplayLastFilm);
    assert!(s.take_requests().is_empty());
}
