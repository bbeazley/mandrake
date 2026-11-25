mod common;

use common::{
    ALTERNATE, BONUS, BREACH, BROKEN_PATH, DRINKING, FINALE, LOSS, SALVATION, TUTORIAL, WILLOW,
    WIN, catalog, report, text, veteran,
};
use mandrake_common::FourCC;
use mandrake_rec::header::MonsterInitializer;
use mandrake_session::progress::VisitedLevel;
use mandrake_session::{
    Action, Catalog, GameEvent, GameStart, LevelStart, Overlay, PostgameExits, Progress, Request,
    Screen, Session,
};

fn session() -> Session {
    Session::new(catalog(), Progress::default())
}

fn veteran_player() -> Progress {
    Progress {
        visited: [WILLOW, SALVATION, BROKEN_PATH, BREACH]
            .map(|mesh| VisitedLevel {
                mesh,
                plugin_checksum: -1,
            })
            .to_vec(),
        last_level: Some(BROKEN_PATH),
        show_hints: false,
        ..Progress::default()
    }
}

fn at_new_game(progress: Progress) -> Session {
    let mut s = Session::new(catalog(), progress);
    s.act(Action::NewGame { all_levels: false });
    s.take_requests();
    s
}

fn at_briefing(progress: Progress) -> Session {
    let mut s = at_new_game(progress);
    s.act(Action::StartLevel);
    s.take_requests();
    s
}

fn at_loading(progress: Progress) -> Session {
    let mut s = at_briefing(progress);
    s.act(Action::BriefingDone);
    s.take_requests();
    s
}

fn playing(progress: Progress) -> Session {
    let mut s = at_loading(progress);
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s
}

fn paused(progress: Progress) -> Session {
    let mut s = playing(progress);
    s.act(Action::OpenPause);
    s.take_requests();
    s
}

fn map_over(s: &mut Session, end_code: i16, veterans: Vec<MonsterInitializer>) {
    s.game_event(GameEvent::MapOver {
        report: report(end_code),
        veterans,
    });
}

fn at_postgame(end_code: i16) -> Session {
    let mut s = playing(veteran_player());
    map_over(&mut s, end_code, Vec::new());
    s.take_requests();
    assert!(matches!(s.screen(), Screen::Postgame(_)));
    s
}

fn is_menu(s: &Session) -> bool {
    matches!(s.screen(), Screen::MainMenu(_))
}

fn is_playing(s: &Session) -> bool {
    matches!(s.screen(), Screen::Playing { overlay: None })
}

fn exits(s: &Session) -> PostgameExits {
    match s.screen() {
        Screen::Postgame(p) => p.exits.clone(),
        other => panic!("not the post-game screen: {other:?}"),
    }
}

fn new_game_level(s: &Session) -> Option<FourCC> {
    match s.screen() {
        Screen::NewGame(n) => n.level,
        other => panic!("not New Game: {other:?}"),
    }
}

fn load_of(requests: &[Request]) -> GameStart {
    let mut loads = requests.iter().filter_map(|r| match r {
        Request::LoadGame(start) => Some(start.clone()),
        _ => None,
    });
    let start = loads.next().expect("a LoadGame");
    assert!(loads.next().is_none());
    start
}

fn saved(requests: &[Request]) -> Option<Progress> {
    requests.iter().find_map(|r| match r {
        Request::SaveProgress(progress) => Some(progress.clone()),
        _ => None,
    })
}

#[test]
fn the_menu_comes_first_and_buttons_with_no_feature_are_disabled() {
    let s = session();
    let Screen::MainMenu(menu) = s.screen() else {
        panic!("not the menu");
    };
    assert!(menu.new_game && menu.tutorial);
    assert!(!menu.multiplayer && !menu.saved_games && !menu.films);
    assert!(!menu.intro && !menu.preferences);
    assert_eq!(menu.music[0].tag, FourCC(*b"mamm"));
    assert!(s.front_end_has_input());
}

#[test]
fn menu_new_game_opens_new_game_and_asks_nothing() {
    let mut s = session();
    s.act(Action::NewGame { all_levels: false });
    assert!(matches!(s.screen(), Screen::NewGame(_)));
    assert!(s.take_requests().is_empty());
}

#[test]
fn menu_tutorial_loads_00tm_with_no_briefing_and_no_veterans() {
    let mut s = playing(veteran_player());
    map_over(&mut s, WIN, vec![veteran(5)]);
    s.act(Action::QuitToMenu);
    s.take_requests();

    s.act(Action::Tutorial);
    assert_eq!(s.screen(), Screen::Loading);
    let start = load_of(&s.take_requests());
    assert_eq!(start.mesh, TUTORIAL);
    assert!(start.veterans.is_empty());
    assert!(start.plugins.is_empty());
    assert_eq!(start.difficulty, 2, "the tutorial keeps the difficulty");
    assert!(start.reset_rankings);
}

#[test]
fn menu_quit_exits() {
    let mut s = session();
    s.act(Action::Quit);
    assert_eq!(s.screen(), Screen::Quit);
    assert_eq!(s.take_requests(), [Request::Exit]);
}

#[test]
fn new_game_shows_the_campaigns_difficulties_and_the_lowest_open_level() {
    let s = at_new_game(Progress::default());
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    assert_eq!(n.campaigns.len(), 2);
    assert_eq!(n.campaign, 0);
    assert_eq!(n.difficulties.len(), 5);
    assert_eq!(n.difficulty, 2);
    assert_eq!(n.level, Some(WILLOW));
    assert_eq!(n.description, text("A description."));
    assert!(n.can_start);
    let open: Vec<_> = n.levels.iter().map(|l| (l.mesh, l.selectable)).collect();
    assert_eq!(
        open,
        [
            (WILLOW, true),
            (SALVATION, false),
            (BROKEN_PATH, false),
            (BREACH, false),
        ]
    );
}

#[test]
fn new_game_start_goes_to_the_briefing_clearing_the_veterans() {
    let mut s = playing(veteran_player());
    map_over(&mut s, WIN, vec![veteran(5)]);
    s.act(Action::QuitToMenu);
    s.act(Action::NewGame { all_levels: false });
    s.take_requests();

    s.act(Action::StartLevel);
    assert!(matches!(s.screen(), Screen::Briefing(_)));
    assert!(saved(&s.take_requests()).is_none(), "nothing new to save");

    s.act(Action::BriefingDone);
    assert!(load_of(&s.take_requests()).veterans.is_empty());
}

#[test]
fn new_game_start_remembers_the_choices_and_marks_the_level_visited() {
    let mut s = at_new_game(Progress::default());
    s.act(Action::ChooseDifficulty(3));
    s.act(Action::StartLevel);
    let progress = saved(&s.take_requests()).expect("a save");
    assert_eq!(progress.last_level, Some(WILLOW));
    assert_eq!(progress.difficulty, 3);
    assert!(progress.is_visited(WILLOW, -1));

    s.act(Action::BriefingDone);
    let start = load_of(&s.take_requests());
    assert_eq!((start.mesh, start.difficulty), (WILLOW, 3));
}

#[test]
fn new_game_cancel_goes_to_the_menu_keeping_the_choices_as_the_engine_does() {
    let mut s = at_new_game(Progress::default());
    s.act(Action::ChooseDifficulty(4));
    s.act(Action::Cancel);
    assert!(is_menu(&s));
    let progress = saved(&s.take_requests()).expect("a save");
    assert_eq!(progress.difficulty, 4);
    assert!(progress.visited.is_empty());

    s.act(Action::NewGame { all_levels: false });
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    assert_eq!(n.difficulty, 4);
}

#[test]
fn a_locked_level_doesnt_select_and_an_open_one_does() {
    let mut s = at_new_game(Progress::default());
    s.act(Action::ChooseLevel(SALVATION));
    assert_eq!(new_game_level(&s), Some(WILLOW));

    let mut s = at_new_game(Progress {
        visited: vec![VisitedLevel {
            mesh: SALVATION,
            plugin_checksum: -1,
        }],
        ..Progress::default()
    });
    s.act(Action::ChooseLevel(SALVATION));
    assert_eq!(new_game_level(&s), Some(SALVATION));
    s.act(Action::ChooseLevel(FourCC(*b"none")));
    assert_eq!(new_game_level(&s), Some(SALVATION));
}

#[test]
fn shift_at_new_game_lists_more_and_opens_everything() {
    let mut s = session();
    s.act(Action::NewGame { all_levels: true });
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    assert_eq!(n.levels[0].mesh, TUTORIAL);
    assert!(n.levels.iter().all(|l| l.selectable));
    assert!(
        n.levels.iter().all(|l| l.mesh != DRINKING),
        "alternates stay hidden"
    );
    s.act(Action::ChooseLevel(BREACH));
    assert_eq!(new_game_level(&s), Some(BREACH));
}

#[test]
fn changing_campaign_reads_all_levels_afresh() {
    let mut s = session();
    s.act(Action::NewGame { all_levels: true });
    s.act(Action::ChooseCampaign {
        index: 1,
        all_levels: false,
    });
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    assert_eq!(n.campaign, 1);
    assert_eq!(n.level, Some(BONUS));
    let rows: Vec<_> = n.levels.iter().map(|l| l.mesh).collect();
    assert_eq!(rows, [BONUS], "released: the hidden entry goes");

    s.act(Action::ChooseCampaign {
        index: 1,
        all_levels: true,
    });
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    let rows: Vec<_> = n.levels.iter().map(|l| l.mesh).collect();
    assert_eq!(rows, [BONUS, FINALE], "held: it's back");
    assert_eq!(n.level, Some(BONUS), "the choice stays");

    s.act(Action::ChooseCampaign {
        index: 9,
        all_levels: false,
    });
    assert_eq!(new_game_level(&s), Some(BONUS), "no such campaign");
}

#[test]
fn a_campaign_is_remembered_and_its_plugins_load_and_carry_to_the_next_level() {
    let mut s = at_new_game(Progress::default());
    s.act(Action::ChooseCampaign {
        index: 1,
        all_levels: false,
    });
    s.act(Action::StartLevel);
    let progress = saved(&s.take_requests()).expect("a save");
    assert_eq!(progress.last_campaign, Some(77));
    assert!(progress.is_visited(BONUS, 77));
    s.act(Action::BriefingDone);
    assert_eq!(load_of(&s.take_requests()).plugins, ["A plugin"]);

    s.game_event(GameEvent::Loaded);
    s.act(Action::CloseHints);
    map_over(&mut s, WIN, Vec::new());
    s.take_requests();
    s.act(Action::NextLevel);
    s.act(Action::BriefingDone);
    assert_eq!(load_of(&s.take_requests()).plugins, ["A plugin"]);

    let mut s = at_new_game(progress);
    s.take_requests();
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    assert_eq!(n.campaign, 1);
    s.act(Action::Cancel);
}

#[test]
fn a_bad_difficulty_is_ignored() {
    let mut s = at_new_game(Progress::default());
    s.act(Action::ChooseDifficulty(5));
    s.act(Action::ChooseDifficulty(-1));
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    assert_eq!(n.difficulty, 2);
}

#[test]
fn start_needs_a_level() {
    let mut s = Session::new(Catalog::default(), Progress::default());
    s.act(Action::NewGame { all_levels: false });
    let Screen::NewGame(n) = s.screen() else {
        panic!("not New Game");
    };
    assert!(!n.can_start);
    s.act(Action::StartLevel);
    assert!(matches!(s.screen(), Screen::NewGame(_)));
    assert!(s.take_requests().is_empty());
}

#[test]
fn briefing_shows_the_levels_tag_data_with_nothing_loaded_yet() {
    let s = at_briefing(Progress::default());
    let Screen::Briefing(b) = s.screen() else {
        panic!("not the briefing");
    };
    assert_eq!(b.art, Some(WILLOW));
    assert_eq!(b.text, text("A briefing."));
    assert_eq!(b.narration, Some(FourCC(*b"na01")));
    assert_eq!(b.title, common::text("l1  "));
}

#[test]
fn briefing_done_goes_to_loading_with_load_game() {
    let mut s = at_briefing(Progress::default());
    s.act(Action::BriefingDone);
    assert_eq!(s.screen(), Screen::Loading);
    assert_eq!(load_of(&s.take_requests()).mesh, WILLOW);
    assert!(s.front_end_has_input());
}

#[test]
fn loading_loaded_goes_to_play_with_begin_play() {
    let mut s = at_loading(veteran_player());
    s.game_event(GameEvent::Loaded);
    assert!(is_playing(&s));
    assert_eq!(s.take_requests(), [Request::BeginPlay]);
    assert!(!s.front_end_has_input());
}

#[test]
fn loading_failure_goes_to_the_menu() {
    let mut s = at_loading(Progress::default());
    s.game_event(GameEvent::LoadFailed("no such map".to_owned()));
    assert!(is_menu(&s));
    assert!(s.take_requests().is_empty());
}

#[test]
fn hints_open_at_the_start_of_play_paused_and_close_unpaused() {
    let mut s = at_loading(Progress::default());
    s.game_event(GameEvent::Loaded);
    assert_eq!(
        s.take_requests(),
        [Request::BeginPlay, Request::SetPaused(true)]
    );
    let Screen::Playing {
        overlay: Some(Overlay::Hints(hints)),
    } = s.screen()
    else {
        panic!("no hints");
    };
    assert_eq!(hints.objectives, text("Objectives."));
    assert_eq!(hints.general, [text("A general hint.")]);
    assert_eq!(hints.level, [text("A level hint.")]);
    assert!(hints.show_at_start);
    assert!(s.front_end_has_input());

    s.act(Action::CloseHints);
    assert!(is_playing(&s));
    assert_eq!(s.take_requests(), [Request::SetPaused(false)]);
}

#[test]
fn the_help_button_opens_the_hints_where_they_were_left() {
    let mut s = at_loading(Progress::default());
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s.act(Action::SetHintIndex(1));
    let saved = s.take_requests();
    assert!(
        matches!(&saved[..], [Request::SaveProgress(p)] if p.hint_index == 1),
        "{saved:?}"
    );
    s.act(Action::CloseHints);
    s.take_requests();
    s.game_event(GameEvent::MissionAdvanced(9));
    s.act(Action::OpenHints);
    assert_eq!(s.take_requests(), [Request::SetPaused(true)]);
    let Screen::Playing {
        overlay: Some(Overlay::Hints(hints)),
    } = s.screen()
    else {
        panic!("no hints");
    };
    assert_eq!(hints.first_general, 1);
    assert_eq!(hints.first_open_objective, 0, "one objective, always open");
    s.act(Action::CloseHints);
    s.take_requests();
    s.act(Action::OpenPause);
    s.act(Action::OpenHints);
    assert!(matches!(
        s.screen(),
        Screen::Playing {
            overlay: Some(Overlay::Paused(_))
        }
    ));
}

#[test]
fn the_show_hints_choice_is_saved_and_the_next_level_obeys_it() {
    let mut s = at_loading(Progress::default());
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s.act(Action::SetShowHints(false));
    let Screen::Playing {
        overlay: Some(Overlay::Hints(hints)),
    } = s.screen()
    else {
        panic!("no hints");
    };
    assert!(!hints.show_at_start);
    let progress = saved(&s.take_requests()).expect("a save");
    assert!(!progress.show_hints);

    s.act(Action::SetShowHints(false));
    assert!(s.take_requests().is_empty(), "no change, no save");

    s.act(Action::CloseHints);
    map_over(&mut s, WIN, Vec::new());
    s.act(Action::NextLevel);
    s.act(Action::BriefingDone);
    s.take_requests();
    s.game_event(GameEvent::Loaded);
    assert!(is_playing(&s));
    assert_eq!(s.take_requests(), [Request::BeginPlay, Request::Autosave]);
}

#[test]
fn a_mesh_with_its_no_hints_flag_opens_none() {
    let mut catalog = catalog();
    for level in &mut catalog.levels {
        level.no_hints_dialog = true;
    }
    let mut s = Session::new(catalog, Progress::default());
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.take_requests();
    s.game_event(GameEvent::Loaded);
    assert!(is_playing(&s));
    assert_eq!(s.take_requests(), [Request::BeginPlay]);
}

#[test]
fn pause_doesnt_open_over_the_hints() {
    let mut s = at_loading(Progress::default());
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s.act(Action::OpenPause);
    assert!(matches!(
        s.screen(),
        Screen::Playing {
            overlay: Some(Overlay::Hints(_))
        }
    ));
    assert!(s.take_requests().is_empty());
}

#[test]
fn play_opens_pause_on_open_pause() {
    let mut s = playing(veteran_player());
    s.act(Action::OpenPause);
    let Screen::Playing {
        overlay: Some(Overlay::Paused(p)),
    } = s.screen()
    else {
        panic!("not paused");
    };
    assert!(p.can_restart);
    assert!(p.can_save && !p.can_open && !p.can_revert);
    assert_eq!(s.take_requests(), [Request::SetPaused(true)]);
    assert!(s.front_end_has_input());
}

#[test]
fn map_over_goes_to_the_postgame_screen_and_drops_the_game() {
    let mut s = playing(veteran_player());
    let the_report = report(WIN);
    s.game_event(GameEvent::MapOver {
        report: the_report.clone(),
        veterans: Vec::new(),
    });
    let Screen::Postgame(p) = s.screen() else {
        panic!("not the post-game screen");
    };
    assert_eq!(p.report, the_report);
    assert_eq!(p.art, Some(FourCC(*b"post")));
    assert_eq!(p.art_sequence, 0);
    assert_eq!(p.ambient, Some(FourCC(*b"wisc")));
    assert!(s.take_requests().contains(&Request::DropGame));
}

#[test]
fn map_over_on_the_tutorial_goes_to_the_menu_with_no_dialog() {
    let mut s = session();
    s.act(Action::Tutorial);
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    s.act(Action::CloseHints);
    s.take_requests();
    map_over(&mut s, WIN, Vec::new());
    assert!(is_menu(&s));
    assert_eq!(s.take_requests(), [Request::DropGame]);
}

#[test]
fn map_over_on_a_mesh_the_catalog_hasnt_got_goes_to_the_menu() {
    let mut s = Session::new(Catalog::default(), Progress::default());
    s.act(Action::Tutorial);
    assert_eq!(s.screen(), Screen::Loading);
    s.game_event(GameEvent::Loaded);
    assert!(is_playing(&s), "no hints to show");
    map_over(&mut s, WIN, Vec::new());
    assert!(is_menu(&s));
}

#[test]
fn map_over_reaches_past_an_open_dialog() {
    let mut s = paused(veteran_player());
    map_over(&mut s, LOSS, Vec::new());
    assert!(matches!(s.screen(), Screen::Postgame(_)));
}

#[test]
fn a_win_marks_the_next_level_visited_before_the_dialog() {
    let mut s = playing(veteran_player());
    map_over(&mut s, WIN, Vec::new());
    assert_eq!(saved(&s.take_requests()), None, "already visited");

    let mut s = at_loading(Progress::default());
    s.game_event(GameEvent::Loaded);
    s.act(Action::CloseHints);
    s.take_requests();
    map_over(&mut s, LOSS, Vec::new());
    assert_eq!(saved(&s.take_requests()), None, "a loss marks nothing");

    let mut s = at_loading(Progress::default());
    s.game_event(GameEvent::Loaded);
    s.act(Action::CloseHints);
    s.take_requests();
    map_over(&mut s, WIN, Vec::new());
    let progress = saved(&s.take_requests()).expect("a save");
    assert!(progress.is_visited(SALVATION, -1));
}

#[test]
fn an_end_code_with_no_dialog_starts_the_level_again_without_veterans() {
    let mut s = playing(veteran_player());
    map_over(&mut s, 0, vec![veteran(5)]);
    assert!(matches!(s.screen(), Screen::Briefing(_)));
    s.act(Action::BriefingDone);
    assert!(load_of(&s.take_requests()).veterans.is_empty());
}

#[test]
fn paused_resume_goes_back_to_play() {
    let mut s = paused(veteran_player());
    s.act(Action::Resume);
    assert!(is_playing(&s));
    assert_eq!(s.take_requests(), [Request::SetPaused(false)]);
}

#[test]
fn paused_restart_goes_to_loading_with_no_briefing() {
    let mut s = paused(veteran_player());
    s.act(Action::RestartLevel);
    assert_eq!(s.screen(), Screen::Loading);
    let requests = s.take_requests();
    assert_eq!(requests[0], Request::DropGame);
    assert_eq!(load_of(&requests).mesh, BROKEN_PATH);
    s.game_event(GameEvent::Loaded);
    assert!(is_playing(&s));
}

#[test]
fn paused_quit_goes_to_the_menu_and_drops_the_game() {
    let mut s = paused(veteran_player());
    s.act(Action::QuitToMenu);
    assert!(is_menu(&s));
    assert_eq!(s.take_requests(), [Request::DropGame]);
}

fn at_finale(end_code: i16) -> Session {
    let mut s = Session::new(
        catalog(),
        Progress {
            show_hints: false,
            ..Progress::default()
        },
    );
    s.act(Action::NewGame { all_levels: true });
    s.act(Action::ChooseCampaign {
        index: 1,
        all_levels: true,
    });
    s.act(Action::ChooseLevel(FINALE));
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    s.take_requests();
    map_over(&mut s, end_code, Vec::new());
    s.take_requests();
    s
}

#[test]
fn map_over_on_the_last_level_goes_to_the_campaign_end_then_postgame() {
    let mut s = at_finale(WIN);
    let Screen::CampaignEnd(n) = s.screen() else {
        panic!("not the campaign's end: {:?}", s.screen());
    };
    assert_eq!(n.narration, Some(FourCC(*b"naep")));
    assert_eq!(n.text, text("It is over."));

    s.act(Action::NextLevel);
    assert!(matches!(s.screen(), Screen::CampaignEnd(_)), "not yet");

    s.act(Action::NarrationDone);
    assert!(matches!(s.screen(), Screen::Postgame(_)));
    assert!(s.take_requests().is_empty());

    assert!(exits(&s).next);
    s.act(Action::NextLevel);
    assert!(is_menu(&s));
}

#[test]
fn a_loss_on_the_last_level_has_no_campaign_end() {
    let s = at_finale(LOSS);
    assert!(matches!(s.screen(), Screen::Postgame(_)));
}

#[test]
fn postgame_next_goes_to_the_next_levels_briefing_and_marks_it() {
    let mut s = at_postgame(WIN);
    s.act(Action::NextLevel);
    let Screen::Briefing(b) = s.screen() else {
        panic!("not the briefing");
    };
    assert_eq!(b.art, Some(BREACH));
    assert!(
        s.take_requests().is_empty(),
        "visited already, and no load yet"
    );
    s.act(Action::BriefingDone);
    assert_eq!(load_of(&s.take_requests()).mesh, BREACH);
}

#[test]
fn postgame_next_with_the_alternate_result_goes_to_the_alternate_with_no_briefing() {
    let mut s = at_postgame(ALTERNATE);
    s.act(Action::NextLevel);
    assert_eq!(s.screen(), Screen::Loading);
    let requests = s.take_requests();
    assert_eq!(load_of(&requests).mesh, DRINKING);
    assert!(saved(&requests).expect("a save").is_visited(DRINKING, -1));
}

#[test]
fn postgame_next_with_no_link_goes_to_the_menu() {
    let mut s = at_new_game(Progress {
        last_level: Some(BREACH),
        show_hints: false,
        ..veteran_player()
    });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    map_over(&mut s, WIN, Vec::new());
    s.take_requests();
    s.act(Action::NextLevel);
    assert!(is_menu(&s));
    assert!(s.take_requests().is_empty());
}

#[test]
fn postgame_skip_goes_to_the_next_link_when_its_offered() {
    let mut catalog = catalog();
    let level = catalog
        .levels
        .iter_mut()
        .find(|l| l.mesh == BROKEN_PATH)
        .expect("in the catalog");
    level.alternate = None;
    let mut s = Session::new(catalog, veteran_player());
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    s.game_event(GameEvent::Loaded);
    map_over(&mut s, ALTERNATE, Vec::new());
    assert!(exits(&s).skip);
    s.take_requests();

    s.act(Action::NextLevel);
    assert!(
        matches!(s.screen(), Screen::Postgame(_)),
        "next isn't offered"
    );

    s.act(Action::SkipLevel);
    let Screen::Briefing(b) = s.screen() else {
        panic!("not the briefing");
    };
    assert_eq!(b.art, Some(BREACH));
}

#[test]
fn postgame_restart_goes_to_loading_with_no_briefing() {
    let mut s = at_postgame(LOSS);
    s.act(Action::RestartLevel);
    assert_eq!(s.screen(), Screen::Loading);
    let requests = s.take_requests();
    assert_eq!(load_of(&requests).mesh, BROKEN_PATH);
    assert!(
        !requests.contains(&Request::DropGame),
        "dropped at the map's end"
    );
}

#[test]
fn postgame_quit_goes_to_the_menu() {
    for end_code in [WIN, ALTERNATE, LOSS] {
        let mut s = at_postgame(end_code);
        s.act(Action::QuitToMenu);
        assert!(is_menu(&s));
        assert!(s.take_requests().is_empty());
    }
}

#[test]
fn exits_not_offered_do_nothing() {
    let mut s = at_postgame(WIN);
    s.act(Action::RestartLevel);
    s.act(Action::SkipLevel);
    assert!(matches!(s.screen(), Screen::Postgame(_)));

    let mut s = at_postgame(LOSS);
    s.act(Action::NextLevel);
    s.act(Action::SkipLevel);
    assert!(matches!(s.screen(), Screen::Postgame(_)));
    assert!(s.take_requests().is_empty());
}

#[test]
fn next_keeps_the_veterans_restart_restores_new_game_and_tutorial_clear() {
    let mut s = playing(veteran_player());
    map_over(&mut s, WIN, vec![veteran(1)]);
    s.act(Action::NextLevel);
    s.act(Action::BriefingDone);
    let start = load_of(&s.take_requests());
    assert_eq!(start.veterans, [veteran(1)], "next keeps what the end left");
    assert!(!start.reset_rankings, "next keeps the rankings");

    s.game_event(GameEvent::Loaded);
    map_over(&mut s, LOSS, vec![veteran(2)]);
    s.take_requests();
    s.act(Action::RestartLevel);
    let start = load_of(&s.take_requests());
    assert_eq!(
        start.veterans,
        [veteran(1)],
        "restart restores what the level began with"
    );
    assert!(!start.reset_rankings, "restart keeps the rankings");

    s.game_event(GameEvent::Loaded);
    s.act(Action::OpenPause);
    s.take_requests();
    s.act(Action::RestartLevel);
    let start = load_of(&s.take_requests());
    assert_eq!(
        start.veterans,
        [veteran(1)],
        "and so does a restart from the pause dialog"
    );
    assert!(!start.reset_rankings, "and keeps the rankings");

    s.game_event(GameEvent::Loaded);
    map_over(&mut s, WIN, vec![veteran(3)]);
    s.act(Action::QuitToMenu);
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::StartLevel);
    s.act(Action::BriefingDone);
    let start = load_of(&s.take_requests());
    assert!(start.veterans.is_empty());
    assert!(start.reset_rankings, "new game clears the rankings");

    s.game_event(GameEvent::Loaded);
    map_over(&mut s, LOSS, vec![veteran(4)]);
    s.take_requests();
    s.act(Action::RestartLevel);
    let start = load_of(&s.take_requests());
    assert!(
        start.veterans.is_empty(),
        "a new game's backup is empty too"
    );
    assert!(!start.reset_rankings, "restart keeps the rankings");
}

#[test]
fn each_level_says_how_the_shell_set_its_game_up() {
    let mut s = at_briefing(veteran_player());
    s.act(Action::BriefingDone);
    assert_eq!(load_of(&s.take_requests()).start, LevelStart::NewGame);

    s.game_event(GameEvent::Loaded);
    s.act(Action::OpenPause);
    s.act(Action::RestartLevel);
    assert_eq!(load_of(&s.take_requests()).start, LevelStart::Restart);

    s.game_event(GameEvent::Loaded);
    map_over(&mut s, WIN, Vec::new());
    s.act(Action::NextLevel);
    s.act(Action::BriefingDone);
    assert_eq!(load_of(&s.take_requests()).start, LevelStart::ChangeMap);

    s.game_event(GameEvent::Loaded);
    map_over(&mut s, LOSS, Vec::new());
    s.take_requests();
    s.act(Action::RestartLevel);
    assert_eq!(load_of(&s.take_requests()).start, LevelStart::Restart);

    s.game_event(GameEvent::Loaded);
    map_over(&mut s, 0, Vec::new());
    s.act(Action::BriefingDone);
    assert_eq!(load_of(&s.take_requests()).start, LevelStart::ChangeMap);

    let mut s = session();
    s.act(Action::Tutorial);
    assert_eq!(load_of(&s.take_requests()).start, LevelStart::ChangeMap);
}

#[test]
fn quit_stays_quit() {
    let mut s = session();
    s.act(Action::Quit);
    s.take_requests();
    s.act(Action::NewGame { all_levels: false });
    s.game_event(GameEvent::Loaded);
    assert_eq!(s.screen(), Screen::Quit);
    assert!(s.take_requests().is_empty());
}

fn all_actions() -> Vec<Action> {
    vec![
        Action::NewGame { all_levels: false },
        Action::Tutorial,
        Action::Quit,
        Action::ChooseCampaign {
            index: 0,
            all_levels: false,
        },
        Action::ChooseLevel(WILLOW),
        Action::ChooseDifficulty(0),
        Action::StartLevel,
        Action::Cancel,
        Action::BriefingDone,
        Action::OpenPause,
        Action::Resume,
        Action::CloseHints,
        Action::SetShowHints(false),
        Action::RestartLevel,
        Action::QuitToMenu,
        Action::NextLevel,
        Action::SkipLevel,
        Action::NarrationDone,
    ]
}

fn does_nothing(make: &dyn Fn() -> Session, action: &Action) -> bool {
    let mut s = make();
    let before = format!("{:?}", s.screen());
    s.act(action.clone());
    format!("{:?}", s.screen()) == before && s.take_requests().is_empty()
}

#[test]
fn only_a_screens_own_actions_do_anything() {
    let hints_open = || {
        let mut s = at_loading(Progress::default());
        s.game_event(GameEvent::Loaded);
        s.take_requests();
        s
    };
    let at_campaign_end = || at_finale(WIN);
    let quit = || {
        let mut s = session();
        s.act(Action::Quit);
        s.take_requests();
        s
    };
    let at_new_game_fresh = || at_new_game(Progress::default());
    let at_briefing_fresh = || at_briefing(Progress::default());
    let at_loading_fresh = || at_loading(Progress::default());
    let playing_plain = || playing(veteran_player());
    let paused_plain = || paused(veteran_player());
    let postgame_win = || at_postgame(WIN);

    type Maker<'a> = &'a dyn Fn() -> Session;
    let screens: [(&str, Maker, &[&str]); 9] = [
        ("menu", &session, &["NewGame", "Tutorial", "Quit"]),
        (
            "new game",
            &at_new_game_fresh,
            &["StartLevel", "Cancel", "ChooseDifficulty"],
        ),
        ("briefing", &at_briefing_fresh, &["BriefingDone"]),
        ("loading", &at_loading_fresh, &[]),
        ("playing", &playing_plain, &["OpenPause"]),
        ("hints", &hints_open, &["CloseHints", "SetShowHints"]),
        (
            "paused",
            &paused_plain,
            &["Resume", "RestartLevel", "QuitToMenu"],
        ),
        ("campaign end", &at_campaign_end, &["NarrationDone"]),
        ("postgame", &postgame_win, &["NextLevel", "QuitToMenu"]),
    ];
    for (name, make, own) in screens {
        for action in all_actions() {
            let label = format!("{action:?}");
            let variant = label.split([' ', '(']).next();
            let is_own = own.iter().any(|&o| Some(o) == variant);
            assert_eq!(does_nothing(make, &action), !is_own, "{action:?} on {name}");
        }
    }
    for action in all_actions() {
        assert!(does_nothing(&quit, &action), "{action:?} after quit");
    }
}

#[test]
fn game_events_that_dont_apply_are_ignored() {
    let mut s = session();
    s.game_event(GameEvent::Loaded);
    s.game_event(GameEvent::LoadFailed("no".to_owned()));
    map_over(&mut s, WIN, vec![veteran(1)]);
    assert!(is_menu(&s));
    assert!(s.take_requests().is_empty());

    let mut s = playing(veteran_player());
    s.game_event(GameEvent::Loaded);
    s.game_event(GameEvent::LoadFailed("no".to_owned()));
    assert!(is_playing(&s));
    assert!(s.take_requests().is_empty());

    let mut s = at_loading(Progress::default());
    map_over(&mut s, WIN, Vec::new());
    assert_eq!(s.screen(), Screen::Loading);
}

#[test]
fn replay_intro_plays_the_prologue_and_comes_back_to_the_menu() {
    let mut s = session();
    s.act(Action::ReplayIntro);
    assert!(matches!(s.screen(), Screen::MainMenu(menu) if !menu.intro));
    assert!(
        s.take_requests().is_empty(),
        "no film: the button does nothing"
    );

    s.game_event(GameEvent::IntroFound(true));
    assert!(matches!(s.screen(), Screen::MainMenu(menu) if menu.intro));
    s.act(Action::ReplayIntro);
    let Screen::Cutscene(cutscene) = s.screen() else {
        panic!("{:?}", s.screen());
    };
    assert_eq!(cutscene.name, "prologue.smk");
    assert_eq!(
        s.take_requests(),
        vec![Request::PlayCutscene {
            name: "prologue.smk".to_owned()
        }]
    );
    assert!(s.front_end_has_input(), "the skip key is the front end's");

    s.game_event(GameEvent::CutsceneOver);
    assert!(matches!(s.screen(), Screen::MainMenu(menu) if menu.intro));
    assert!(s.take_requests().is_empty());
}

#[test]
fn plugin_campaign_reusing_base_mesh_id_uses_plugin_level_for_briefing_hints_visited_and_postgame()
{
    use mandrake_session::Difficulty;
    use mandrake_session::catalog::{Campaign, Level, ListEntry};
    use mandrake_session::rules::{ENTRY_LISTED, ENTRY_SOLO};

    const PLUGIN_CSUM: i32 = 0x1234_5678;
    const PLUGIN_NEXT: FourCC = FourCC(*b"p2  ");
    let tag = |s: &[u8; 4]| FourCC(*s);

    let mut base_l1 = Level::new(WILLOW);
    base_l1.title = text("Base Level 1");
    base_l1.pre_art = Some(tag(b"01pr"));
    base_l1.briefing_text = Some(text("Base briefing"));
    base_l1.post_art = Some(tag(b"01po"));
    base_l1.next = Some(SALVATION);
    base_l1.hints = vec![text("Base hint")];

    let mut plugin_l1 = Level::new(WILLOW);
    plugin_l1.plugin_checksum = PLUGIN_CSUM;
    plugin_l1.title = text("Plugin Level 1");
    plugin_l1.pre_art = Some(tag(b"p1pr"));
    plugin_l1.briefing_text = Some(text("Plugin briefing"));
    plugin_l1.post_art = Some(tag(b"p1po"));
    plugin_l1.next = Some(PLUGIN_NEXT);
    plugin_l1.hints = vec![text("Plugin hint")];

    let mut plugin_l2 = Level::new(PLUGIN_NEXT);
    plugin_l2.plugin_checksum = PLUGIN_CSUM;
    plugin_l2.title = text("Plugin Level 2");
    plugin_l2.pre_art = Some(tag(b"p2pr"));
    plugin_l2.briefing_text = Some(text("Plugin 2 briefing"));

    let catalog = Catalog {
        campaigns: vec![
            Campaign {
                name: text("Base"),
                checksum: -1,
                plugins: Vec::new(),
                entries: vec![common::entry(WILLOW, 1, ENTRY_SOLO | ENTRY_LISTED)],
            },
            Campaign {
                name: text("My Plugin"),
                checksum: PLUGIN_CSUM,
                plugins: vec!["My Plugin".to_owned()],
                entries: vec![
                    ListEntry {
                        plugin_checksum: PLUGIN_CSUM,
                        ..common::entry(WILLOW, 1, ENTRY_SOLO | ENTRY_LISTED)
                    },
                    ListEntry {
                        plugin_checksum: PLUGIN_CSUM,
                        ..common::entry(PLUGIN_NEXT, 2, ENTRY_SOLO | ENTRY_LISTED)
                    },
                ],
            },
        ],
        levels: vec![base_l1, Level::new(SALVATION), plugin_l1, plugin_l2],
        difficulties: vec![Difficulty {
            name: text("Normal"),
            description: text("Standard"),
        }],
        ..Catalog::default()
    };

    let mut s = Session::new(catalog, Progress::default());
    s.act(Action::NewGame { all_levels: false });
    s.act(Action::ChooseCampaign {
        index: 1,
        all_levels: false,
    });
    s.act(Action::StartLevel);

    let Screen::Briefing(briefing) = s.screen() else {
        panic!("expected plugin briefing, got {:?}", s.screen());
    };
    assert_eq!(briefing.title, text("Plugin Level 1"));
    assert_eq!(briefing.art, Some(tag(b"p1pr")));
    assert_eq!(briefing.text, text("Plugin briefing"));

    let reqs = s.take_requests();
    let saved_progress = reqs
        .iter()
        .find_map(|r| match r {
            Request::SaveProgress(p) => Some(p),
            _ => None,
        })
        .expect("progress saved");
    assert!(
        saved_progress
            .visited
            .iter()
            .any(|v| v.mesh == WILLOW && v.plugin_checksum == PLUGIN_CSUM),
        "plugin checksum should be marked visited, got {:?}",
        saved_progress.visited
    );

    s.act(Action::BriefingDone);
    s.take_requests();
    s.game_event(GameEvent::Loaded);
    s.act(Action::OpenHints);
    let Screen::Playing {
        overlay: Some(Overlay::Hints(hints)),
    } = s.screen()
    else {
        panic!("expected hints overlay, got {:?}", s.screen());
    };
    assert_eq!(hints.level, vec![text("Plugin hint")]);
    s.act(Action::CloseHints);

    map_over(&mut s, WIN, Vec::new());
    let Screen::Postgame(post) = s.screen() else {
        panic!("expected postgame, got {:?}", s.screen());
    };
    assert_eq!(post.art, Some(tag(b"p1po")));
    let reqs = s.take_requests();
    let saved_progress = reqs
        .iter()
        .find_map(|r| match r {
            Request::SaveProgress(p) => Some(p),
            _ => None,
        })
        .expect("next level progress saved");
    assert!(
        saved_progress
            .visited
            .iter()
            .any(|v| v.mesh == PLUGIN_NEXT && v.plugin_checksum == PLUGIN_CSUM),
        "plugin next level should be marked visited under plugin checksum, got {:?}",
        saved_progress.visited
    );

    s.act(Action::NextLevel);
    let Screen::Briefing(next_briefing) = s.screen() else {
        panic!("expected plugin level 2 briefing, got {:?}", s.screen());
    };
    assert_eq!(next_briefing.title, text("Plugin Level 2"));
}
