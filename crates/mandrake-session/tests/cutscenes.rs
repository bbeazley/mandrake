mod common;

use std::path::PathBuf;

use common::{
    ALTERNATE, BROKEN_PATH, DRINKING, FINALE, LOSS, SALVATION, WILLOW, WIN, catalog, report, text,
};
use mandrake_common::FourCC;
use mandrake_session::films::{FilmGame, FilmPlugin};
use mandrake_session::rules::MESH_FILM_BEFORE_BRIEFING;
use mandrake_session::{
    Action, Catalog, GameEvent, Progress, Request, SaveFile, SaveFolder, Screen, Session,
};

fn film(name: &str) -> Option<String> {
    Some(name.to_owned())
}

fn films_catalog() -> Catalog {
    let mut catalog = catalog();
    for level in &mut catalog.levels {
        level.cutscenes = match level.mesh {
            WILLOW => [film("01.smk"), None, None],
            BROKEN_PATH => [film("04.smk"), None, film("lost.smk")],
            DRINKING => [film("03a.smk"), None, None],
            FINALE => [film("25.smk"), film("epilogue.smk"), None],
            _ => [None, None, None],
        };
    }
    catalog
}

fn session(catalog: Catalog) -> Session {
    Session::new(catalog, Progress::default())
}

fn plays(name: &str) -> Request {
    Request::PlayCutscene {
        name: name.to_owned(),
    }
}

fn take(s: &mut Session) -> Vec<Request> {
    s.take_requests()
        .into_iter()
        .filter(|r| !matches!(r, Request::SaveProgress(_)))
        .collect()
}

fn has_load(requests: &[Request]) -> bool {
    requests.iter().any(|r| matches!(r, Request::LoadGame(_)))
}

fn has_film(requests: &[Request]) -> bool {
    requests
        .iter()
        .any(|r| matches!(r, Request::PlayCutscene { .. }))
}

fn cutscene_name(s: &Session) -> String {
    match s.screen() {
        Screen::Cutscene(c) => c.name.clone(),
        other => panic!("no film: {other:?}"),
    }
}

fn start(catalog: Catalog, mesh: FourCC) -> Session {
    let mut s = session(catalog);
    s.act(Action::NewGame { all_levels: true });
    if mesh == FINALE {
        s.act(Action::ChooseCampaign {
            index: 1,
            all_levels: true,
        });
    }
    s.act(Action::ChooseLevel(mesh));
    take(&mut s);
    s.act(Action::StartLevel);
    s
}

fn play(s: &mut Session) {
    if matches!(s.screen(), Screen::Briefing(_)) {
        s.act(Action::BriefingDone);
    }
    if matches!(s.screen(), Screen::Cutscene(_)) {
        s.game_event(GameEvent::CutsceneOver);
    }
    assert_eq!(s.screen(), Screen::Loading);
    s.game_event(GameEvent::Loaded);
    take(s);
}

fn map_over(s: &mut Session, end_code: i16) -> Vec<Request> {
    s.game_event(GameEvent::MapOver {
        report: report(end_code),
        veterans: Vec::new(),
    });
    take(s)
}

#[test]
fn briefing_then_the_film_then_the_objectives() {
    let mut s = start(films_catalog(), WILLOW);
    assert!(matches!(s.screen(), Screen::Briefing(_)));
    assert!(!has_film(&take(&mut s)), "not during the briefing");

    s.act(Action::BriefingDone);
    assert_eq!(cutscene_name(&s), "01.smk");
    assert_eq!(take(&mut s), [plays("01.smk")], "no load yet");
    assert!(s.front_end_has_input(), "the skip key is the front end's");

    s.game_event(GameEvent::CutsceneOver);
    assert_eq!(s.screen(), Screen::Loading);
    let requests = take(&mut s);
    assert!(has_load(&requests) && !has_film(&requests));

    s.game_event(GameEvent::Loaded);
    assert!(matches!(
        s.screen(),
        Screen::Playing {
            overlay: Some(mandrake_session::Overlay::Hints(_))
        }
    ));
}

#[test]
fn a_skipped_film_goes_on_as_an_ended_one_and_other_actions_are_ignored() {
    let mut s = start(films_catalog(), WILLOW);
    s.act(Action::BriefingDone);
    take(&mut s);
    for action in [Action::BriefingDone, Action::OpenPause, Action::Cancel] {
        s.act(action);
    }
    s.game_event(GameEvent::Loaded);
    assert_eq!(cutscene_name(&s), "01.smk");
    assert!(take(&mut s).is_empty());

    s.game_event(GameEvent::CutsceneOver);
    assert!(has_load(&take(&mut s)));
    s.game_event(GameEvent::CutsceneOver);
    assert_eq!(s.screen(), Screen::Loading);
}

#[test]
fn a_missing_film_is_over_at_once() {
    let mut s = start(films_catalog(), WILLOW);
    s.act(Action::BriefingDone);
    assert_eq!(take(&mut s), [plays("01.smk")]);
    s.game_event(GameEvent::CutsceneOver);
    assert!(has_load(&take(&mut s)));
}

#[test]
fn a_level_with_no_film_loads_after_its_briefing() {
    let mut s = start(films_catalog(), SALVATION);
    s.act(Action::BriefingDone);
    let requests = take(&mut s);
    assert!(has_load(&requests) && !has_film(&requests));
}

#[test]
fn a_level_with_no_briefing_still_plays_its_film() {
    let mut s = start(films_catalog(), BROKEN_PATH);
    play(&mut s);
    map_over(&mut s, ALTERNATE);
    s.act(Action::NextLevel);
    assert_eq!(cutscene_name(&s), "03a.smk");
    let requests = take(&mut s);
    assert!(requests.contains(&plays("03a.smk")) && !has_load(&requests));
    s.game_event(GameEvent::CutsceneOver);
    assert!(has_load(&take(&mut s)));
}

#[test]
fn mesh_flag_2_plays_the_film_before_the_briefing() {
    let mut catalog = films_catalog();
    for level in &mut catalog.levels {
        if level.mesh == WILLOW {
            level.flags |= MESH_FILM_BEFORE_BRIEFING;
        }
    }
    let mut s = start(catalog, WILLOW);
    assert_eq!(cutscene_name(&s), "01.smk");
    assert!(take(&mut s).contains(&plays("01.smk")));
    s.game_event(GameEvent::CutsceneOver);
    assert!(matches!(s.screen(), Screen::Briefing(_)));
    s.act(Action::BriefingDone);
    let requests = take(&mut s);
    assert!(has_load(&requests) && !has_film(&requests), "once only");
}

#[test]
fn restart_level_plays_no_film() {
    let mut s = start(films_catalog(), WILLOW);
    play(&mut s);
    s.act(Action::CloseHints);
    s.act(Action::OpenPause);
    s.act(Action::RestartLevel);
    let requests = take(&mut s);
    assert!(has_load(&requests) && !has_film(&requests));
    assert_eq!(s.screen(), Screen::Loading);

    s.game_event(GameEvent::Loaded);
    take(&mut s);
    let requests = map_over(&mut s, LOSS);
    assert!(!has_film(&requests), "Willow Creek has no loss film");
    assert!(matches!(s.screen(), Screen::Postgame(_)));
    s.act(Action::RestartLevel);
    let requests = take(&mut s);
    assert!(has_load(&requests) && !has_film(&requests));
}

#[test]
fn a_loss_plays_the_loss_film_before_the_epilogue() {
    let mut s = start(films_catalog(), BROKEN_PATH);
    play(&mut s);
    let requests = map_over(&mut s, LOSS);
    assert_eq!(requests, [Request::DropGame, plays("lost.smk")]);
    assert_eq!(cutscene_name(&s), "lost.smk");
    s.game_event(GameEvent::CutsceneOver);
    assert!(matches!(s.screen(), Screen::Postgame(p) if p.exits.restart));

    let mut s = start(films_catalog(), BROKEN_PATH);
    play(&mut s);
    assert_eq!(map_over(&mut s, WIN), [Request::DropGame]);
    assert!(matches!(s.screen(), Screen::Postgame(p) if p.exits.next));
}

#[test]
fn next_level_plays_the_next_levels_film() {
    let mut catalog = films_catalog();
    for level in &mut catalog.levels {
        if level.mesh == SALVATION {
            level.cutscenes[0] = film("cr01.smk");
        }
    }
    let mut s = start(catalog, WILLOW);
    play(&mut s);
    map_over(&mut s, WIN);
    s.act(Action::NextLevel);
    assert!(matches!(s.screen(), Screen::Briefing(_)));
    s.act(Action::BriefingDone);
    assert!(take(&mut s).contains(&plays("cr01.smk")));
}

#[test]
fn the_campaigns_end_plays_the_win_film_then_the_game_epilogue() {
    let mut s = start(films_catalog(), FINALE);
    assert!(matches!(s.screen(), Screen::Briefing(_)));
    s.act(Action::BriefingDone);
    assert!(take(&mut s).contains(&plays("25.smk")));
    play(&mut s);

    let requests = map_over(&mut s, WIN);
    assert_eq!(requests, [Request::DropGame, plays("epilogue.smk")]);
    assert_eq!(cutscene_name(&s), "epilogue.smk");
    s.game_event(GameEvent::CutsceneOver);
    let Screen::CampaignEnd(narration) = s.screen() else {
        panic!("no game epilogue: {:?}", s.screen());
    };
    assert_eq!(narration.text, text("It is over."));
    s.act(Action::NarrationDone);
    assert!(matches!(s.screen(), Screen::Postgame(_)));
}

fn game(mesh: FourCC) -> FilmGame {
    FilmGame {
        mode: 2,
        game_type: 13,
        mesh,
        difficulty: 3,
        plugins: vec![FilmPlugin {
            name: "Patch 1.8.5 Maps".to_owned(),
            checksum: 7,
        }],
        map_name: Some(text("A level")),
        plugins_present: true,
        ..FilmGame::default()
    }
}

#[test]
fn a_loaded_save_plays_no_pregame_film() {
    let save = SaveFile {
        path: PathBuf::from("saved games/Autosave"),
        name: text("Autosave"),
        modified: 0,
        date: text("Nov 10, 2026 01:00 AM"),
        game: game(BROKEN_PATH),
        end_tick: 30,
        veterans: Vec::new(),
        snapshot: None,
        subgroup: FourCC(*b"save"),
    };
    let folder = SaveFolder {
        saves: vec![save],
        any: true,
        new_name: text("New Saved Game"),
    };
    let mut s = session(films_catalog());
    s.game_event(GameEvent::SavesListed(folder.clone()));
    s.act(Action::OpenSavedGames);
    take(&mut s);
    s.game_event(GameEvent::SavesListed(folder));
    s.act(Action::OpenSave);
    let requests = take(&mut s);
    assert!(has_load(&requests) && !has_film(&requests));
    s.game_event(GameEvent::Loaded);
    take(&mut s);
    assert_eq!(
        map_over(&mut s, LOSS),
        [Request::DropGame, plays("lost.smk")]
    );
}

#[test]
fn the_intro_still_ends_at_the_menu() {
    let mut s = session(films_catalog());
    s.game_event(GameEvent::IntroFound(true));
    s.act(Action::ReplayIntro);
    assert_eq!(take(&mut s), [plays("prologue.smk")]);
    s.game_event(GameEvent::CutsceneOver);
    assert!(matches!(s.screen(), Screen::MainMenu(_)));
}
