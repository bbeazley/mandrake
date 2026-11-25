#[path = "../../mandrake-common/tests/support/require_data.rs"]
mod test_data;

mod common;

use std::path::PathBuf;

use common::{BROKEN_PATH, LOSS, WILLOW, catalog, report, text, veteran};
use mandrake_session::films::{FilmGame, FilmPlugin};
use mandrake_session::{
    Action, GameEvent, Overlay, Progress, Request, SaveFile, SaveFolder, Screen, Session,
};

fn save(name: &str, mesh: mandrake_common::FourCC, found: bool, end_tick: u32) -> SaveFile {
    SaveFile {
        path: PathBuf::from("saved games").join(name),
        name: text(name),
        modified: 0,
        date: text("Oct 30, 2026 09:08 PM"),
        game: FilmGame {
            mode: 2,
            game_type: 13,
            mesh,
            difficulty: 4,
            plugins: vec![FilmPlugin {
                name: "Patch 1.8.5 Maps".to_owned(),
                checksum: 7,
            }],
            map_name: found.then(|| text("A level")),
            plugins_missing: !found,
            plugins_present: found,
            ..FilmGame::default()
        },
        end_tick,
        veterans: vec![veteran(3)],
        snapshot: None,
        subgroup: mandrake_common::FourCC(name.as_bytes()[name.len() - 4..].try_into().unwrap()),
    }
}

fn folder() -> SaveFolder {
    SaveFolder {
        saves: vec![
            save("Quicksave 2", BROKEN_PATH, true, 14147),
            save("Quicksave 9", WILLOW, false, 10),
        ],
        any: true,
        new_name: text("New Saved Game"),
    }
}

fn answer(session: &mut Session, folder: &SaveFolder) -> Vec<Request> {
    let mut rest = Vec::new();
    let mut requests = session.take_requests();
    while !requests.is_empty() {
        for request in requests {
            match request {
                Request::ListSaves => session.game_event(GameEvent::SavesListed(folder.clone())),
                other => rest.push(other),
            }
        }
        requests = session.take_requests();
    }
    rest
}

fn at_menu(folder: &SaveFolder) -> Session {
    let mut s = Session::new(catalog(), Progress::default());
    s.game_event(GameEvent::SavesListed(folder.clone()));
    s
}

fn saved_games(session: &Session) -> mandrake_session::SavedGames {
    match session.screen() {
        Screen::SavedGames(saves) => saves.clone(),
        other => panic!("not Saved Games: {other:?}"),
    }
}

#[test]
fn saved_games_is_off_until_the_folder_has_a_save() {
    let mut s = at_menu(&SaveFolder::default());
    let Screen::MainMenu(menu) = s.screen() else {
        panic!("the menu");
    };
    assert!(!menu.saved_games);
    s.act(Action::OpenSavedGames);
    assert!(matches!(s.screen(), Screen::MainMenu(_)), "a disabled item");
    s.game_event(GameEvent::SavesListed(folder()));
    let Screen::MainMenu(menu) = s.screen() else {
        panic!("the menu");
    };
    assert!(menu.saved_games);
}

#[test]
fn menu_saved_games_load_restore_play() {
    let folder = folder();
    let mut s = at_menu(&folder);
    s.act(Action::OpenSavedGames);
    answer(&mut s, &folder);
    let shown = saved_games(&s);
    assert_eq!(shown.names, [text("Quicksave 2"), text("Quicksave 9")]);
    assert_eq!(shown.selected, Some(0), "the first row");
    assert_eq!(shown.date, text("Oct 30, 2026 09:08 PM"));
    assert!(shown.can_open && shown.delete, "Delete is never disabled");

    s.act(Action::ChooseSave(1));
    assert!(!saved_games(&s).can_open);
    s.act(Action::OpenSave);
    assert!(matches!(s.screen(), Screen::SavedGames(_)));
    assert!(s.take_requests().is_empty());

    s.act(Action::ChooseSave(0));
    s.act(Action::OpenSave);
    let requests = s.take_requests();
    let load = requests.iter().find_map(|r| match r {
        Request::LoadGame(start) => Some(start),
        _ => None,
    });
    let start = load.expect("a load");
    assert_eq!(start.film, Some(PathBuf::from("saved games/Quicksave 2")));
    assert!(start.restore);
    assert_eq!((start.mesh, start.difficulty), (BROKEN_PATH, 4));
    assert_eq!(start.plugins, ["Patch 1.8.5 Maps"]);
    assert_eq!(start.veterans, [veteran(3)], "the save's own table");
    assert!(!start.reset_rankings);
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, Request::SaveProgress(p) if p.difficulty == 4))
    );
    assert!(matches!(s.screen(), Screen::Loading));

    let fractions: Vec<i16> = [1, 1800, 1801, 14147]
        .into_iter()
        .map(|ticks| {
            s.game_event(GameEvent::Restoring { ticks });
            match s.screen() {
                Screen::Restoring(r) => {
                    assert_eq!(r.name, text("Quicksave 2"));
                    assert!(!r.network);
                    r.fraction
                }
                other => panic!("restoring: {other:?}"),
            }
        })
        .collect();
    assert_eq!(fractions, [32, 32, 64, 256]);

    s.game_event(GameEvent::Loaded);
    assert_eq!(
        s.take_requests(),
        [Request::BeginPlay],
        "no hints after a restore"
    );
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));
    s.act(Action::OpenPause);
    let Screen::Playing {
        overlay: Some(Overlay::Paused(paused)),
    } = s.screen()
    else {
        panic!("paused");
    };
    assert!(paused.can_restart, "a live game");
    s.act(Action::Resume);
    s.take_requests();

    s.game_event(GameEvent::MapOver {
        report: report(LOSS),
        veterans: Vec::new(),
    });
    assert!(
        matches!(s.screen(), Screen::Postgame(_)),
        "the epilogue: {:?}",
        s.screen()
    );
}

#[test]
fn cancel_goes_back_to_the_menu() {
    let folder = folder();
    let mut s = at_menu(&folder);
    s.act(Action::OpenSavedGames);
    answer(&mut s, &folder);
    s.act(Action::Cancel);
    assert!(matches!(s.screen(), Screen::MainMenu(_)));
    assert!(s.take_requests().is_empty());
}
