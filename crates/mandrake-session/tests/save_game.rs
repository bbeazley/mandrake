mod common;

use std::path::PathBuf;

use common::{LOSS, SALVATION, TUTORIAL, WILLOW, WIN, catalog, report, text, veteran};
use mandrake_session::films::FilmGame;
use mandrake_session::progress::VisitedLevel;
use mandrake_session::{
    Action, GameEvent, Overlay, Progress, Request, SaveFile, SaveFolder, SaveGame, Screen, Session,
    Snapshot, Text,
};

fn save(name: &str, end_tick: u32) -> SaveFile {
    SaveFile {
        path: PathBuf::from("saved games").join(name),
        name: text(name),
        modified: 0,
        date: Text::default(),
        game: FilmGame {
            mode: 2,
            game_type: 13,
            mesh: WILLOW,
            difficulty: 2,
            map_name: Some(text("Willow Creek")),
            plugins_present: true,
            ..FilmGame::default()
        },
        end_tick,
        veterans: vec![veteran(5)],
        snapshot: None,
        subgroup: mandrake_common::FourCC(name.as_bytes()[name.len() - 4..].try_into().unwrap()),
    }
}

fn folder() -> SaveFolder {
    SaveFolder {
        saves: vec![
            save("Quicksave 2", 14147),
            save("Autosave- 2026-11-03 06.50.51", 0),
        ],
        any: true,
        new_name: text("New Saved Game"),
    }
}

fn snapshot() -> Snapshot {
    Snapshot {
        width: 160,
        height: 120,
        rgba: vec![7; 160 * 120 * 4],
    }
}

fn answer(s: &mut Session, folder: &SaveFolder) -> Vec<Request> {
    let mut rest = Vec::new();
    loop {
        let requests = s.take_requests();
        if requests.is_empty() {
            return rest;
        }
        for request in requests {
            match request {
                Request::ListSaves => s.game_event(GameEvent::SavesListed(folder.clone())),
                Request::TakeSnapshot => s.game_event(GameEvent::Snapshot(snapshot())),
                other => rest.push(other),
            }
        }
    }
}

fn progress() -> Progress {
    Progress {
        visited: vec![VisitedLevel {
            mesh: WILLOW,
            plugin_checksum: -1,
        }],
        last_level: Some(WILLOW),
        show_hints: false,
        ..Progress::default()
    }
}

fn playing() -> Session {
    let mut s = Session::new(catalog(), progress());
    s.game_event(GameEvent::SavesListed(folder()));
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));
    s
}

fn prompt(s: &Session) -> Option<SaveGame> {
    match s.screen() {
        Screen::Playing {
            overlay: Some(Overlay::SaveGame(prompt)),
        } => Some(prompt.clone()),
        _ => None,
    }
}

fn paused(s: &Session) -> mandrake_session::Paused {
    match s.screen() {
        Screen::Playing {
            overlay: Some(Overlay::Paused(paused)),
        } => paused.clone(),
        other => panic!("not paused: {other:?}"),
    }
}

#[test]
fn pause_save_ok_saves_and_resumes() {
    let mut s = playing();
    s.act(Action::OpenPause);
    let p = paused(&s);
    assert!(p.can_save && !p.can_revert && !p.replay);
    assert!(p.can_open, "`save_game_exists`: the folder has saves");
    assert_eq!(s.take_requests(), [Request::SetPaused(true)]);
    s.act(Action::SaveGame);
    assert_eq!(
        s.take_requests(),
        [Request::TakeSnapshot, Request::ListSaves],
        "`render_screenshot` before the dialog"
    );
    s.game_event(GameEvent::SavesListed(folder()));
    s.game_event(GameEvent::Snapshot(snapshot()));
    let shown = prompt(&s).expect("the prompt");
    assert_eq!(shown.name, text("New Saved Game"));
    assert_eq!(
        shown.names,
        [text("Quicksave 2"), text("Autosave- 2026-11-03 06.50.51")]
    );
    assert_eq!(shown.snapshot, Some(snapshot()));
    assert_eq!(shown.replace, None);

    s.act(Action::Resume);
    s.act(Action::SaveGameAs(text("a/b")));
    assert!(prompt(&s).is_some());
    assert!(s.take_requests().is_empty());

    s.act(Action::SaveGameAs(text("Before the bridge")));
    assert_eq!(
        s.take_requests(),
        [
            Request::SaveGame {
                name: text("Before the bridge")
            },
            Request::SetPaused(false)
        ]
    );
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));
    s.game_event(GameEvent::GameSaved(Some(save("Before the bridge", 300))));

    s.act(Action::OpenPause);
    assert!(paused(&s).can_revert);
    s.act(Action::SaveGame);
    answer(&mut s, &folder());
    assert_eq!(prompt(&s).unwrap().name, text("Before the bridge"));
    s.act(Action::Cancel);
    assert_eq!(s.take_requests(), [Request::SetPaused(false)]);
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));

    s.act(Action::OpenPause);
    s.take_requests();
    s.act(Action::LastSavedGame);
    let requests = s.take_requests();
    assert_eq!(requests.first(), Some(&Request::DropGame));
    let start = requests.iter().find_map(|r| match r {
        Request::LoadGame(start) => Some(start.clone()),
        _ => None,
    });
    let start = start.expect("the save loads");
    assert_eq!(
        start.film,
        Some(PathBuf::from("saved games/Before the bridge"))
    );
    assert!(start.restore);
    assert_eq!(start.veterans, [veteran(5)]);
}

#[test]
fn a_saves_name_asks_whether_to_replace_it() {
    let mut s = playing();
    s.act(Action::OpenPause);
    s.act(Action::SaveGame);
    answer(&mut s, &folder());
    s.act(Action::SaveGameAs(text("Quicksave 2")));
    assert_eq!(prompt(&s).unwrap().replace, Some(text("Quicksave 2")));
    assert!(s.take_requests().is_empty());
    s.act(Action::SaveGameAs(text("Other")));
    assert!(s.take_requests().is_empty(), "the question is modal");
    s.act(Action::Cancel);
    let shown = prompt(&s).expect("back at the prompt");
    assert_eq!(shown.replace, None);
    s.act(Action::SaveGameAs(text("Quicksave 2")));
    s.act(Action::ReplaceSave);
    assert_eq!(
        s.take_requests(),
        [
            Request::SaveGame {
                name: text("Quicksave 2")
            },
            Request::SetPaused(false)
        ]
    );
}

#[test]
fn an_autosave_isnt_the_boxs_name() {
    let mut s = playing();
    s.game_event(GameEvent::GameSaved(Some(save(
        "Autosave- 2026-11-03 06.50.51",
        0,
    ))));
    s.act(Action::OpenPause);
    s.act(Action::SaveGame);
    answer(&mut s, &folder());
    assert_eq!(prompt(&s).unwrap().name, text("New Saved Game"));
}

#[test]
fn next_level_autosaves_on_the_first_tick() {
    let mut s = playing();
    s.game_event(GameEvent::MapOver {
        report: report(WIN),
        veterans: vec![veteran(9)],
    });
    s.take_requests();
    s.act(Action::NextLevel);
    s.act(Action::BriefingDone);
    let requests = s.take_requests();
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, Request::LoadGame(g) if g.mesh == SALVATION))
    );
    assert!(!requests.contains(&Request::Autosave), "not before play");
    s.game_event(GameEvent::Loaded);
    assert_eq!(s.take_requests(), [Request::BeginPlay, Request::Autosave]);

    s.game_event(GameEvent::MapOver {
        report: report(LOSS),
        veterans: Vec::new(),
    });
    s.act(Action::RestartLevel);
    s.game_event(GameEvent::Loaded);
    assert!(!s.take_requests().contains(&Request::Autosave));
}

#[test]
fn last_saved_game_after_a_loss() {
    let mut s = playing();
    s.game_event(GameEvent::GameSaved(Some(save("Before the bridge", 300))));
    s.game_event(GameEvent::MapOver {
        report: report(LOSS),
        veterans: Vec::new(),
    });
    let Screen::Postgame(postgame) = s.screen() else {
        panic!("the epilogue");
    };
    assert!(postgame.exits.last_saved && postgame.exits.restart);
    s.take_requests();
    s.act(Action::LastSavedGame);
    let requests = s.take_requests();
    assert!(requests.iter().any(
        |r| matches!(r, Request::LoadGame(g) if g.restore && g.film == Some(PathBuf::from("saved games/Before the bridge")))
    ));

    let mut s = playing();
    s.game_event(GameEvent::GameSaved(Some(save("Before the bridge", 300))));
    s.act(Action::OpenPause);
    s.act(Action::QuitToMenu);
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::Cancel);
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    s.act(Action::OpenPause);
    assert!(!paused(&s).can_revert);
}

#[test]
fn the_tutorial_cant_be_saved() {
    let mut s = Session::new(catalog(), progress());
    s.act(Action::Tutorial);
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s.act(Action::OpenPause);
    assert!(!paused(&s).can_save);
    s.act(Action::SaveGame);
    assert!(prompt(&s).is_none());
    assert_eq!(TUTORIAL, mandrake_session::rules::TUTORIAL);
}
