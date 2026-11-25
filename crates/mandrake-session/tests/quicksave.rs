mod common;

use std::path::PathBuf;

use common::{BROKEN_PATH, LOSS, WILLOW, catalog, report, text};
use mandrake_session::films::{FilmGame, FilmPlugin};
use mandrake_session::progress::VisitedLevel;
use mandrake_session::{
    Action, GameEvent, LevelStart, NewGame, Overlay, Paused, Progress, Request, SaveFile,
    SaveFolder, SavedGames, Screen, Session,
};

fn save(name: &str, subgroup: &[u8; 4], modified: i64) -> SaveFile {
    SaveFile {
        path: PathBuf::from("saved games").join(name),
        name: text(name),
        modified,
        date: text("Nov 08, 2026 09:08 PM"),
        game: FilmGame {
            mode: 2,
            game_type: 13,
            mesh: BROKEN_PATH,
            difficulty: 2,
            plugins: vec![FilmPlugin {
                name: "Patch 1.8.5 Maps".to_owned(),
                checksum: 7,
            }],
            map_name: Some(text("Down a Broken Path")),
            plugins_present: true,
            ..FilmGame::default()
        },
        end_tick: 1533,
        veterans: Vec::new(),
        snapshot: None,
        subgroup: mandrake_common::FourCC(*subgroup),
    }
}

fn folder(saves: Vec<SaveFile>) -> SaveFolder {
    SaveFolder {
        any: !saves.is_empty(),
        saves,
        new_name: text("New Saved Game"),
    }
}

fn answer(s: &mut Session, folder: &mut SaveFolder) -> Vec<Request> {
    let mut rest = Vec::new();
    loop {
        let requests = s.take_requests();
        if requests.is_empty() {
            return rest;
        }
        for request in requests {
            match request {
                Request::ListSaves => s.game_event(GameEvent::SavesListed(folder.clone())),
                Request::DeleteSave { file_name } => {
                    folder.saves.retain(|f| f.name.0 != file_name);
                    folder.any = !folder.saves.is_empty();
                }
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

fn playing(folder: &SaveFolder) -> Session {
    let mut s = Session::new(catalog(), progress());
    s.game_event(GameEvent::SavesListed(folder.clone()));
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));
    s
}

fn paused(s: &Session) -> Paused {
    match s.screen() {
        Screen::Playing {
            overlay: Some(Overlay::Paused(paused)),
        } => paused.clone(),
        other => panic!("not paused: {other:?}"),
    }
}

fn open_over_game(s: &Session) -> SavedGames {
    match s.screen() {
        Screen::Playing {
            overlay: Some(Overlay::SavedGames(saves)),
        } => saves.clone(),
        other => panic!("not Saved Games over the game: {other:?}"),
    }
}

fn new_game_over_game(s: &Session) -> NewGame {
    match s.screen() {
        Screen::Playing {
            overlay: Some(Overlay::NewGame(new_game)),
        } => new_game.clone(),
        other => panic!("not New Game over the game: {other:?}"),
    }
}

#[test]
fn open_over_the_game_and_cancel_goes_on() {
    let mut saves = folder(vec![
        save("Quicksave 2", b"qui2", 9),
        save("Mine", b"mine", 8),
    ]);
    let mut s = playing(&saves);
    s.act(Action::OpenPause);
    s.take_requests();
    assert!(paused(&s).can_open, "`save_game_exists`");
    s.act(Action::OpenSavedGames);
    assert!(
        answer(&mut s, &mut saves).is_empty(),
        "the game stays still"
    );
    let shown = open_over_game(&s);
    assert_eq!(shown.names, [text("Quicksave 2"), text("Mine")]);
    assert_eq!(shown.selected, Some(0));
    assert!(shown.can_open && shown.delete);
    s.act(Action::ChooseSave(1));
    assert_eq!(open_over_game(&s).selected, Some(1));
    s.act(Action::Quicksave);
    s.act(Action::OpenPause);
    assert!(s.take_requests().is_empty());

    s.act(Action::Cancel);
    assert_eq!(s.take_requests(), [Request::SetPaused(false)]);
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));

    let mut one = folder(vec![save("Only", b"only", 1)]);
    let mut s = playing(&one);
    s.act(Action::OpenPause);
    s.act(Action::OpenSavedGames);
    answer(&mut s, &mut one);
    s.act(Action::DeleteSave);
    assert_eq!(
        open_over_game(&s).delete_save.map(|q| q.name),
        Some(text("Only"))
    );
    s.act(Action::ConfirmDeleteSave);
    assert!(answer(&mut s, &mut one).is_empty());
    assert!(open_over_game(&s).selected.is_none());
    s.act(Action::OpenSave);
    assert_eq!(s.take_requests(), [Request::SetPaused(false)]);
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));
    s.act(Action::OpenPause);
    assert!(!paused(&s).can_open, "no save left");
}

#[test]
fn open_over_the_game_loads_the_save() {
    let mut saves = folder(vec![save("Quicksave 2", b"qui2", 9)]);
    let mut s = playing(&saves);
    s.act(Action::OpenPause);
    s.act(Action::OpenSavedGames);
    answer(&mut s, &mut saves);
    s.act(Action::OpenSave);
    let requests = answer(&mut s, &mut saves);
    assert_eq!(requests[0], Request::DropGame);
    let Request::LoadGame(start) = &requests[1] else {
        panic!("{requests:?}");
    };
    assert!(start.restore);
    assert_eq!(start.mesh, BROKEN_PATH);
    assert_eq!(
        start.film,
        Some(PathBuf::from("saved games").join("Quicksave 2"))
    );
    assert_eq!(s.screen(), Screen::Loading);
    s.game_event(GameEvent::Restoring { ticks: 1533 });
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s.act(Action::OpenPause);
    assert!(paused(&s).can_revert);
}

#[test]
fn revert_with_the_save_gone_opens_new_game_over_the_game() {
    let mut saves = folder(vec![
        save("Quicksave 5", b"qui5", 9),
        save("Other", b"othr", 8),
    ]);
    let mut s = playing(&saves);
    s.game_event(GameEvent::GameSaved(Some(saves.saves[0].clone())));
    s.act(Action::OpenPause);
    s.act(Action::OpenSavedGames);
    answer(&mut s, &mut saves);
    s.act(Action::DeleteSave);
    s.act(Action::ConfirmDeleteSave);
    answer(&mut s, &mut saves);
    assert_eq!(open_over_game(&s).names, [text("Other")]);
    s.act(Action::Cancel);
    s.take_requests();
    s.act(Action::OpenPause);
    s.take_requests();
    assert!(paused(&s).can_revert, "the subgroup is still remembered");

    s.act(Action::LastSavedGame);
    assert!(
        s.take_requests().is_empty(),
        "nothing loaded, the game still"
    );
    let shown = new_game_over_game(&s);
    assert_eq!(shown.level, Some(WILLOW));
    assert!(shown.can_start);
    s.act(Action::OpenPause);
    s.act(Action::Quicksave);
    assert!(s.take_requests().is_empty());
    s.act(Action::ChooseDifficulty(3));
    assert_eq!(new_game_over_game(&s).difficulty, 3);

    s.act(Action::Cancel);
    let requests = s.take_requests();
    assert!(
        requests.contains(&Request::SetPaused(false)),
        "{requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, Request::SaveProgress(p) if p.difficulty == 3)),
        "the dialog's choices kept on Cancel too: {requests:?}"
    );
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));
    s.act(Action::OpenPause);
    assert!(!paused(&s).can_revert, "forgotten");
    s.act(Action::Resume);
    s.take_requests();

    s.game_event(GameEvent::GameSaved(Some(save("Gone", b"gone", 1))));
    s.act(Action::OpenPause);
    s.act(Action::OpenSavedGames);
    let mut gone = folder(vec![save("Gone", b"gone", 1)]);
    answer(&mut s, &mut gone);
    s.act(Action::DeleteSave);
    s.act(Action::ConfirmDeleteSave);
    answer(&mut s, &mut gone);
    s.act(Action::Cancel);
    s.act(Action::OpenPause);
    s.take_requests();
    s.act(Action::LastSavedGame);
    new_game_over_game(&s);
    s.act(Action::StartLevel);
    let requests = s.take_requests();
    assert_eq!(requests.first(), Some(&Request::DropGame));
    let in_level = matches!(s.screen(), Screen::Briefing(_))
        || requests.iter().any(|r| {
            matches!(r, Request::LoadGame(start) if start.start == LevelStart::NewGame
                && start.reset_rankings && start.veterans.is_empty())
        });
    assert!(in_level, "{requests:?} {:?}", s.screen());
    s.act(Action::OpenPause);
    assert!(!matches!(s.screen(), Screen::Playing { .. }));
}

#[test]
fn the_epilogues_last_saved_game_with_the_save_gone() {
    let lost = |s: &mut Session, saves: &mut SaveFolder| {
        s.game_event(GameEvent::GameSaved(Some(saves.saves[0].clone())));
        s.act(Action::OpenPause);
        s.act(Action::OpenSavedGames);
        answer(s, saves);
        s.act(Action::DeleteSave);
        s.act(Action::ConfirmDeleteSave);
        answer(s, saves);
        s.act(Action::Cancel);
        s.game_event(GameEvent::MapOver {
            report: report(LOSS),
            veterans: Vec::new(),
        });
        s.take_requests();
        let Screen::Postgame(postgame) = s.screen() else {
            panic!("the epilogue: {:?}", s.screen());
        };
        assert!(postgame.exits.last_saved, "`[0x7b2eb4] != -1`");
        s.act(Action::LastSavedGame);
        assert!(s.take_requests().is_empty(), "nothing loaded");
        let Screen::NewGame(new_game) = s.screen() else {
            panic!("New Game: {:?}", s.screen());
        };
        assert!(new_game.no_menu, "over nothing");
    };
    let mut saves = folder(vec![save("Quicksave 5", b"qui5", 9)]);
    let mut s = playing(&saves);
    lost(&mut s, &mut saves);
    s.act(Action::Cancel);
    assert!(matches!(s.screen(), Screen::MainMenu(_)));

    let mut saves = folder(vec![save("Quicksave 5", b"qui5", 9)]);
    let mut s = playing(&saves);
    lost(&mut s, &mut saves);
    s.act(Action::StartLevel);
    assert!(
        matches!(s.screen(), Screen::Briefing(_) | Screen::Loading),
        "{:?}",
        s.screen()
    );
}
