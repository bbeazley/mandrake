mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use common::{WILLOW, catalog, text};
use mandrake_common::FourCC;
use mandrake_session::progress::VisitedLevel;
use mandrake_session::{
    Action, GameEvent, Overlay, Progress, Request, SavedGames, Screen, Session, saves,
};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("SAVE_MANAGE_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")), PathBuf::from);
    let game = base.join(name);
    let _ = std::fs::remove_dir_all(&game);
    std::fs::create_dir_all(game.join(saves::FOLDER)).unwrap();
    game
}

fn write_save(game: &Path, file: &str, subgroup: &[u8; 4], time: u64) {
    let mut data: Vec<u8> = (0..0xa28 + 16).map(|i| (i % 7) as u8).collect();
    data[0..2].copy_from_slice(&3u16.to_be_bytes());
    let bytes = mandrake_rec::writer::save_file(file.as_bytes(), FourCC(*subgroup), &data);
    let path = game.join(saves::FOLDER).join(file);
    std::fs::write(&path, bytes).unwrap();
    let f = std::fs::File::options().write(true).open(&path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(time))
        .unwrap();
}

fn files(game: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(game.join(saves::FOLDER))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn listed(game: &Path) -> GameEvent {
    let tags = mandrake_tag::mesh_links::CampaignTags::open(game);
    GameEvent::SavesListed(saves::list_folder(game, &tags))
}

fn run(s: &mut Session, game: &Path) -> Vec<Request> {
    let mut rest = Vec::new();
    loop {
        let requests = s.take_requests();
        if requests.is_empty() {
            return rest;
        }
        for request in requests {
            match request {
                Request::ListSaves => s.game_event(listed(game)),
                Request::DeleteSave { file_name } => {
                    saves::delete_save(game, &file_name).unwrap();
                }
                other => rest.push(other),
            }
        }
    }
}

fn saved_games(s: &Session) -> SavedGames {
    match s.screen() {
        Screen::SavedGames(saves) => saves.clone(),
        other => panic!("not Saved Games: {other:?}"),
    }
}

fn chosen(s: &Session) -> Option<String> {
    let shown = saved_games(s);
    shown.selected.map(|row| shown.names[row].to_plain_utf8())
}

fn choose(s: &mut Session, name: &str) {
    let row = saved_games(s)
        .names
        .iter()
        .position(|n| n.to_plain_utf8() == name)
        .expect("listed");
    s.act(Action::ChooseSave(row));
    assert_eq!(chosen(s).as_deref(), Some(name));
}

fn fill(game: &Path) {
    for (file, subgroup, time) in [
        ("Quicksave 2", b"qui2", 1_793_354_903),
        ("Quicksave 3", b"qui3", 1_793_355_000),
        ("Quicksave 4", b"qui4", 1_793_356_000),
        ("Quicksave 5", b"qui5", 1_793_357_000),
        ("Autosave- 2026-11-03 06.50.51", b"aut1", 1_793_649_051),
    ] {
        write_save(game, file, subgroup, time);
    }
}

fn open_saved_games(s: &mut Session, game: &Path) {
    s.act(Action::OpenSavedGames);
    run(s, game);
    assert_eq!(
        chosen(s).as_deref(),
        Some("Autosave- 2026-11-03 06.50.51"),
        "the first row"
    );
}

fn at_menu(game: &Path) -> Session {
    let mut s = Session::new(catalog(), progress());
    s.game_event(listed(game));
    s
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

#[test]
fn delete_asks_then_deletes_and_the_first_row_is_chosen() {
    let game = scratch("delete");
    fill(&game);
    let mut s = at_menu(&game);
    open_saved_games(&mut s, &game);
    let shown = saved_games(&s);
    assert!(shown.delete, "Delete is never disabled");
    assert!(shown.delete_save.is_none());

    choose(&mut s, "Quicksave 3");
    s.act(Action::DeleteSave);
    let question = saved_games(&s).delete_save.expect("asked");
    assert_eq!(question.name, text("Quicksave 3"));
    s.act(Action::ChooseSave(0));
    s.act(Action::OpenSave);
    assert_eq!(chosen(&s).as_deref(), Some("Quicksave 3"));
    assert!(s.take_requests().is_empty());

    s.act(Action::Cancel);
    assert!(saved_games(&s).delete_save.is_none(), "closed");
    assert_eq!(chosen(&s).as_deref(), Some("Quicksave 3"), "still chosen");
    assert!(s.take_requests().is_empty(), "nothing deleted");
    assert_eq!(files(&game).len(), 5);

    s.act(Action::DeleteSave);
    s.act(Action::ConfirmDeleteSave);
    assert_eq!(
        run(&mut s, &game),
        Vec::<Request>::new(),
        "only the folder's requests"
    );
    assert_eq!(
        files(&game),
        [
            "Autosave- 2026-11-03 06.50.51",
            "Quicksave 2",
            "Quicksave 4",
            "Quicksave 5"
        ]
    );
    let shown = saved_games(&s);
    assert!(shown.delete_save.is_none());
    assert_eq!(
        shown.names,
        [
            text("Autosave- 2026-11-03 06.50.51"),
            text("Quicksave 5"),
            text("Quicksave 4"),
            text("Quicksave 2")
        ]
    );
    assert_eq!(
        chosen(&s).as_deref(),
        Some("Autosave- 2026-11-03 06.50.51"),
        "the first row, not the one above"
    );
}

#[test]
fn the_last_save_deleted_leaves_an_empty_list() {
    let game = scratch("last");
    write_save(&game, "Only", b"only", 1_793_354_903);
    let mut s = at_menu(&game);
    assert!(matches!(s.screen(), Screen::MainMenu(m) if m.saved_games));
    s.act(Action::OpenSavedGames);
    run(&mut s, &game);
    s.act(Action::DeleteSave);
    assert_eq!(saved_games(&s).delete_save.unwrap().name, text("Only"));
    s.act(Action::ConfirmDeleteSave);
    run(&mut s, &game);
    assert!(files(&game).is_empty());
    let shown = saved_games(&s);
    assert!(shown.names.is_empty() && shown.selected.is_none());
    assert!(shown.date.0.is_empty() && shown.snapshot.is_none());
    assert!(shown.delete, "still drawn enabled");
    s.act(Action::DeleteSave);
    assert!(saved_games(&s).delete_save.is_none(), "no row: no question");
    s.act(Action::OpenSave);
    assert!(
        matches!(s.screen(), Screen::MainMenu(m) if !m.saved_games),
        "{:?}",
        s.screen()
    );
}

#[test]
fn the_question_names_the_tag_as_found() {
    let game = scratch("long");
    let long = "A very long saved game name, longer than the tag's";
    write_save(&game, long, b"long", 1_793_354_903);
    let mut s = at_menu(&game);
    s.act(Action::OpenSavedGames);
    run(&mut s, &game);
    s.act(Action::DeleteSave);
    assert_eq!(saved_games(&s).delete_save.unwrap().name, text(&long[..31]));
}

#[test]
fn names_that_would_leave_the_folder_are_refused() {
    let game = scratch("refused");
    std::fs::write(game.join("outside"), b"x").unwrap();
    for name in [&b""[..], b".", b"..", b"../outside", b"a/b", b"a\0b"] {
        assert!(saves::delete_save(&game, name).is_err(), "{name:?}");
    }
    assert!(game.join("outside").is_file());
}

fn paused(s: &Session) -> mandrake_session::Paused {
    match s.screen() {
        Screen::Playing {
            overlay: Some(Overlay::Paused(paused)),
        } => paused.clone(),
        other => panic!("not paused: {other:?}"),
    }
}

fn saved_then_quit(game: &Path, name: &str) -> Session {
    let mut s = at_menu(game);
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    let tags = mandrake_tag::mesh_links::CampaignTags::open(game);
    let folder = saves::list_folder(game, &tags);
    let save = folder.saves.iter().find(|f| f.name == text(name)).cloned();
    s.game_event(GameEvent::GameSaved(save));
    s.act(Action::OpenPause);
    assert!(paused(&s).can_revert);
    s.act(Action::QuitToMenu);
    s.take_requests();
    s
}

fn delete(s: &mut Session, game: &Path, name: &str) {
    open_saved_games(s, game);
    choose(s, name);
    s.act(Action::DeleteSave);
    s.act(Action::ConfirmDeleteSave);
    run(s, game);
    s.act(Action::Cancel);
    assert!(matches!(s.screen(), Screen::MainMenu(_)));
}

fn in_tutorial(s: &mut Session) {
    s.act(Action::Tutorial);
    if matches!(s.screen(), Screen::Briefing(_)) {
        s.act(Action::BriefingDone);
    }
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s.act(Action::OpenPause);
    s.take_requests();
}

#[test]
fn revert_after_the_last_saved_game_is_deleted() {
    let game = scratch("revert");
    fill(&game);
    let mut s = saved_then_quit(&game, "Quicksave 5");
    delete(&mut s, &game, "Quicksave 5");
    in_tutorial(&mut s);
    assert!(paused(&s).can_revert, "the subgroup is still remembered");
    s.act(Action::LastSavedGame);
    assert!(s.take_requests().is_empty(), "nothing loaded");
    assert!(matches!(
        s.screen(),
        Screen::Playing {
            overlay: Some(Overlay::NewGame(_))
        }
    ));
    s.act(Action::Cancel);
    assert!(s.take_requests().contains(&Request::SetPaused(false)));
    assert!(matches!(s.screen(), Screen::Playing { overlay: None }));
    s.act(Action::OpenPause);
    assert!(!paused(&s).can_revert, "forgotten");
}

#[test]
fn another_save_of_the_subgroup_is_reverted_to() {
    let game = scratch("subgroup");
    fill(&game);
    write_save(&game, "Quicksave 5 again", b"qui5", 1_793_340_000);
    let mut s = saved_then_quit(&game, "Quicksave 5");
    delete(&mut s, &game, "Quicksave 5");
    in_tutorial(&mut s);
    s.act(Action::LastSavedGame);
    let requests = s.take_requests();
    let load = requests.iter().find_map(|r| match r {
        Request::LoadGame(start) => start.film.clone(),
        _ => None,
    });
    assert_eq!(
        load,
        Some(game.join(saves::FOLDER).join("Quicksave 5 again"))
    );
}

#[test]
fn deleting_another_save_keeps_revert() {
    let game = scratch("other");
    fill(&game);
    let mut s = saved_then_quit(&game, "Quicksave 5");
    delete(&mut s, &game, "Quicksave 2");
    in_tutorial(&mut s);
    s.act(Action::LastSavedGame);
    let requests = s.take_requests();
    assert!(requests.iter().any(|r| matches!(
        r,
        Request::LoadGame(start) if start.film == Some(game.join(saves::FOLDER).join("Quicksave 5"))
    )));
}
