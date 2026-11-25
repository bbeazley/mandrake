mod common;

use std::path::PathBuf;

use common::{BROKEN_PATH, LOSS, catalog, report, text};
use mandrake_common::FourCC;
use mandrake_session::progress::VisitedLevel;
use mandrake_session::{
    Action, FilmFile, FilmFolder, GameEvent, Progress, Request, SaveFilm, Screen, Session,
};

fn folder(last_exists: bool) -> FilmFolder {
    let last = FilmFile {
        path: PathBuf::from("recordings/Last Recording"),
        file_name: b"Last Recording".to_vec(),
        name: text("Last Recording"),
        game: mandrake_session::FilmGame {
            mesh: BROKEN_PATH,
            map_name: Some(text("Down a Broken Path")),
            plugins_present: true,
            ..mandrake_session::FilmGame::default()
        },
    };
    FilmFolder {
        films: vec![last.clone()],
        last: last_exists.then_some(last),
        any: true,
        last_exists,
        names: vec![text("Last Recording"), text("MDL")],
        file_names: vec![b"Last Recording".to_vec(), b"MDL.m2rec".to_vec()],
        last_name: text("Last Recording"),
        untitled: text("Untitled Replay"),
    }
}

fn answer(s: &mut Session, folder: &FilmFolder) -> Vec<Request> {
    let mut rest = Vec::new();
    loop {
        let requests = s.take_requests();
        if requests.is_empty() {
            return rest;
        }
        for request in requests {
            match request {
                Request::ListFilms => s.game_event(GameEvent::FilmsListed(folder.clone())),
                other => rest.push(other),
            }
        }
    }
}

fn at_postgame() -> Session {
    let progress = Progress {
        visited: vec![VisitedLevel {
            mesh: BROKEN_PATH,
            plugin_checksum: -1,
        }],
        last_level: Some(BROKEN_PATH),
        show_hints: false,
        ..Progress::default()
    };
    let mut s = Session::new(catalog(), progress);
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    s.game_event(GameEvent::MapOver {
        report: report(LOSS),
        veterans: Vec::new(),
    });
    s.take_requests();
    assert!(matches!(s.screen(), Screen::Postgame(_)));
    s
}

fn prompt(s: &Session) -> Option<SaveFilm> {
    match s.screen() {
        Screen::Postgame(p) => p.save_film.clone(),
        Screen::Films(f) => f.save_film.clone(),
        other => panic!("no prompt here: {other:?}"),
    }
}

fn save_film_enabled(s: &Session) -> bool {
    match s.screen() {
        Screen::Postgame(p) => p.exits.save_film,
        other => panic!("not the epilogue: {other:?}"),
    }
}

#[test]
fn the_epilogues_save_film_prompts_saves_and_is_disabled() {
    let mut s = at_postgame();
    assert!(save_film_enabled(&s));
    assert_eq!(prompt(&s), None);
    s.act(Action::SaveFilm);
    assert!(answer(&mut s, &folder(true)).is_empty());
    let shown = prompt(&s).expect("the prompt");
    assert_eq!(shown.name, text("Untitled Replay"));
    assert_eq!(shown.names, [text("Last Recording"), text("MDL")]);

    s.act(Action::SaveFilmAs(text("Last Recording")));
    s.act(Action::SaveFilmAs(text("a/b")));
    assert!(s.take_requests().is_empty());
    assert!(prompt(&s).is_some(), "the prompt stays");
    s.act(Action::QuitToMenu);
    assert!(matches!(s.screen(), Screen::Postgame(_)));

    s.act(Action::SaveFilmAs(text("my film")));
    assert_eq!(
        s.take_requests(),
        [Request::SaveFilm {
            name: text("my film")
        }]
    );
    assert_eq!(prompt(&s), None);
    s.game_event(GameEvent::FilmSaved(Some(b"my film.m2rec".to_vec())));
    assert!(!save_film_enabled(&s), "the engine disables item 9");
    s.act(Action::SaveFilm);
    assert!(answer(&mut s, &folder(true)).is_empty());
    assert_eq!(prompt(&s), None);
}

#[test]
fn cancel_closes_the_prompt_and_a_failed_save_leaves_save_film_on() {
    let mut s = at_postgame();
    s.act(Action::SaveFilm);
    answer(&mut s, &folder(true));
    s.act(Action::Cancel);
    assert_eq!(prompt(&s), None);
    assert!(matches!(s.screen(), Screen::Postgame(_)));
    s.act(Action::SaveFilm);
    answer(&mut s, &folder(true));
    s.act(Action::SaveFilmAs(text("x")));
    s.take_requests();
    s.game_event(GameEvent::FilmSaved(None));
    assert!(save_film_enabled(&s));
}

#[test]
fn no_last_recording_no_prompt() {
    let mut s = at_postgame();
    s.act(Action::SaveFilm);
    answer(&mut s, &folder(false));
    assert_eq!(prompt(&s), None);
}

#[test]
fn save_last_film_saves_and_chooses_the_new_film() {
    let mut s = Session::new(catalog(), Progress::default());
    s.game_event(GameEvent::FilmsListed(folder(true)));
    s.act(Action::OpenFilms);
    answer(&mut s, &folder(true));
    let Screen::Films(films) = s.screen() else {
        panic!("Saved Films");
    };
    assert!(films.save_last && films.last_film);
    s.act(Action::SaveFilm);
    answer(&mut s, &folder(true));
    assert!(prompt(&s).is_some());
    s.act(Action::SaveFilmAs(text("kept")));
    let requests = answer(&mut s, &folder(true));
    assert_eq!(requests, [Request::SaveFilm { name: text("kept") }]);
    let mut after = folder(true);
    let mut kept = after.films[0].clone();
    kept.file_name = b"kept.m2rec".to_vec();
    kept.name = text("kept");
    after.films.insert(0, kept);
    s.game_event(GameEvent::FilmSaved(Some(b"kept.m2rec".to_vec())));
    answer(&mut s, &after);
    let Screen::Films(films) = s.screen() else {
        panic!("Saved Films");
    };
    assert_eq!((films.selected, films.save_film.clone()), (Some(0), None));
    assert_eq!(films.details.as_ref().unwrap().name, text("kept"));

    let mut none = folder(false);
    none.last = None;
    s.game_event(GameEvent::FilmsListed(none));
    let Screen::Films(films) = s.screen() else {
        panic!("Saved Films");
    };
    assert!(!films.save_last);
}

#[test]
fn films_are_saved_as_the_engine_saves_them() {
    let game = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("save-film-game");
    let _ = std::fs::remove_dir_all(&game);
    let data: Vec<u8> = (0..0xa28 + 16).map(|i| (i % 7) as u8).collect();
    let mut data = data;
    data[0..2].copy_from_slice(&3u16.to_be_bytes());
    let mut seeds = [1u16, 2, 3, 4].into_iter().cycle();
    let last =
        mandrake_session::films::write_last_recording(&game, b"Last Recording", &data, || {
            seeds.next().unwrap()
        })
        .unwrap();
    let bytes = std::fs::read(&last).unwrap();
    assert_eq!(&bytes[4..19], b"Last Recording\0");
    assert_eq!(&bytes[36..44], b"reco!\"#$");
    assert_eq!(&bytes[64..], &data[..]);

    let save = |name: &[u8]| {
        mandrake_session::films::save_last_film(&game, b"Last Recording", name).unwrap()
    };
    assert_eq!(save(b"Untitled Replay"), b"Untitled Replay.m2rec");
    assert_eq!(save(b"Untitled Replays"), b"Untitled Replays.m2rec");
    assert_eq!(save(b"Untitled Replay"), b"Untitled Replay.m2rec");
    let head = |file: &str| std::fs::read(game.join("recordings").join(file)).unwrap();
    let first = head("Untitled Replay.m2rec");
    assert_eq!(FourCC(first[40..44].try_into().unwrap()), FourCC(*b"unre"));
    assert_eq!(&first[64..], &data[..]);
    assert_eq!(
        first[..64],
        mandrake_rec::writer::tag_header(b"Untitled Replay", FourCC(*b"reco"), FourCC(*b"unre"), 3)
    );
    let second = head("Untitled Replays.m2rec");
    assert_eq!(FourCC(second[40..44].try_into().unwrap()), FourCC(*b"unrf"));

    let tags = mandrake_tag::mesh_links::CampaignTags::open(&game);
    let listed = mandrake_session::films::list_folder(&game, &tags);
    assert!(listed.last_exists && listed.any);
    assert_eq!(
        listed.names,
        [
            text("Last Recording"),
            text("Untitled Replay"),
            text("Untitled Replays")
        ]
    );
    assert!(mandrake_session::films::save_last_film(&game, b"Nothing Here", b"x").is_err());
}
