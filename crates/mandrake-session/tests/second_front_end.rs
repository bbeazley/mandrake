//! A small front end with no drawing, so that `FrontEnd` has two implementers before anything
//! depends on it (section 2). It walks the whole first path through the trait and section 2's frame
//! loop: menu, New Game, briefing, loading, play, a win, post-game, next level, a defeat, restart,
//! quit to menu, quit. It sees only `Screen`, as any front end does.

mod common;

use std::collections::VecDeque;

use common::{LOSS, SALVATION, WILLOW, WIN, catalog, report, veteran};
use mandrake_rec::header::MonsterInitializer;
use mandrake_session::{
    Action, FrameInput, FrameOutput, FrontEnd, GameEvent, GameStart, Overlay, Progress, Request,
    Screen, Session, SoundCue,
};

/// Plays the first path and quits when it comes back to the menu. It matches every screen with no
/// wildcard, so a new one has to be looked at here.
#[derive(Default)]
struct PathPlayer {
    menu_visits: u32,
    restarted: bool,
    played_sounds: Vec<SoundCue>,
}

impl FrontEnd for PathPlayer {
    fn frame(&mut self, screen: &Screen<'_>, input: &FrameInput<'_>) -> FrameOutput {
        let mut sounds = Vec::new();
        let action = match screen {
            Screen::MainMenu(menu) => {
                assert!(menu.new_game, "the path starts here");
                self.menu_visits += 1;
                Some(if self.menu_visits == 1 {
                    Action::NewGame {
                        all_levels: input.shift_held,
                    }
                } else {
                    Action::Quit
                })
            }
            Screen::NewGame(new_game) => {
                assert!(new_game.can_start);
                Some(Action::StartLevel)
            }
            Screen::Briefing(briefing) => {
                sounds.extend(briefing.narration.map(SoundCue::Narration));
                Some(Action::BriefingDone)
            }
            Screen::Playing {
                overlay: Some(Overlay::Hints(_)),
            } => Some(Action::CloseHints),
            Screen::Playing {
                overlay: Some(Overlay::Paused(_)),
            } => Some(Action::Resume),
            Screen::Playing {
                overlay:
                    Some(
                        Overlay::SaveGame(_)
                        | Overlay::SavedGames(_)
                        | Overlay::NewGame(_)
                        | Overlay::UnitTrading(_),
                    ),
            } => Some(Action::Cancel),
            Screen::CampaignEnd(_) => Some(Action::NarrationDone),
            Screen::Films(_) => Some(Action::Cancel),
            Screen::FilmReport(_) => Some(Action::CloseReport),
            Screen::SavedGames(_) => Some(Action::Cancel),
            // The first path never goes multiplayer: `tests/net.rs` walks those screens.
            Screen::Net(_) => Some(Action::Cancel),
            Screen::Restoring(_) | Screen::Cutscene(_) => None,
            Screen::Postgame(postgame) => {
                sounds.extend(postgame.ambient.map(SoundCue::Ambient));
                if postgame.exits.next {
                    Some(Action::NextLevel)
                } else if postgame.exits.restart && !self.restarted {
                    self.restarted = true;
                    Some(Action::RestartLevel)
                } else {
                    Some(Action::QuitToMenu)
                }
            }
            Screen::Playing { overlay: None } | Screen::Loading | Screen::Quit => None,
        };
        self.played_sounds.extend(sounds.iter().copied());
        FrameOutput {
            actions: action.into_iter().collect(),
            sounds,
        }
    }
}

/// What the client does for the session: answers its requests, some frames later.
struct FakeGame {
    /// How each game it runs will end, in order.
    endings: VecDeque<(i16, Vec<MonsterInitializer>)>,
    started: Vec<GameStart>,
    loading_frames: Option<u32>,
    frames_in_play: Option<u32>,
    paused: bool,
    exited: bool,
    saves: usize,
}

impl FakeGame {
    fn perform(&mut self, request: Request) {
        match request {
            Request::LoadGame(start) => {
                assert!(self.frames_in_play.is_none(), "the last game was dropped");
                self.started.push(start);
                self.loading_frames = Some(3);
            }
            Request::BeginPlay => self.frames_in_play = Some(0),
            Request::SetPaused(paused) => self.paused = paused,
            Request::DropGame => self.frames_in_play = None,
            Request::SaveProgress(_) => self.saves += 1,
            Request::ListFilms
            | Request::ListSaves
            | Request::SaveFilm { .. }
            | Request::DeleteFilm { .. }
            | Request::DeleteSave { .. }
            | Request::RenameFilm { .. }
            | Request::TakeSnapshot
            | Request::SaveGame { .. }
            | Request::Autosave
            | Request::Quicksave
            | Request::PlayCutscene { .. }
            | Request::Net(_)
            | Request::SaveNetPlayer(_)
            | Request::DescribeNetGame(_)
            | Request::NetChatTo(..) => {}
            Request::Exit => self.exited = true,
        }
    }

    fn step(&mut self, session: &mut Session) {
        if let Some(frames) = &mut self.loading_frames {
            if *frames == 0 {
                self.loading_frames = None;
                session.game_event(GameEvent::Loaded);
            } else {
                *frames -= 1;
            }
        }
        if self.paused {
            return;
        }
        if let Some(frames) = &mut self.frames_in_play {
            *frames += 1;
            if *frames == 5 {
                let (end_code, veterans) = self.endings.pop_front().expect("a game too many");
                session.game_event(GameEvent::MapOver {
                    report: report(end_code),
                    veterans,
                });
            }
        }
    }
}

fn label(screen: &Screen<'_>) -> &'static str {
    match screen {
        Screen::MainMenu(_) => "menu",
        Screen::Cutscene(_) => "cutscene",
        Screen::NewGame(_) => "new game",
        Screen::Loading => "loading",
        Screen::Briefing(_) => "briefing",
        Screen::Playing { overlay: None } => "play",
        Screen::Playing {
            overlay: Some(Overlay::Hints(_)),
        } => "hints",
        Screen::Playing {
            overlay: Some(Overlay::Paused(_)),
        } => "paused",
        Screen::Playing {
            overlay: Some(Overlay::SaveGame(_)),
        } => "save game",
        Screen::Playing {
            overlay: Some(Overlay::SavedGames(_)),
        } => "saved games over the game",
        Screen::Playing {
            overlay: Some(Overlay::NewGame(_)),
        } => "new game over the game",
        Screen::Playing {
            overlay: Some(Overlay::UnitTrading(_)),
        } => "unit trading",
        Screen::CampaignEnd(_) => "campaign end",
        Screen::Postgame(_) => "postgame",
        Screen::Films(_) => "films",
        Screen::FilmReport(_) => "film report",
        Screen::SavedGames(_) => "saved games",
        Screen::Restoring(_) => "restoring",
        Screen::Net(_) => "multiplayer",
        Screen::Quit => "quit",
    }
}

#[test]
fn the_whole_first_path_through_the_trait() {
    let mut session = Session::new(catalog(), Progress::default());
    let mut front_end = PathPlayer::default();
    let mut game = FakeGame {
        endings: VecDeque::from([
            (WIN, vec![veteran(1)]),
            (LOSS, vec![veteran(2)]),
            (LOSS, vec![veteran(3)]),
        ]),
        started: Vec::new(),
        loading_frames: None,
        frames_in_play: None,
        paused: false,
        exited: false,
        saves: 0,
    };

    let mut path = vec![label(&session.screen())];
    for frame in 0..200 {
        // Section 2's one frame, without a window.
        let input = FrameInput {
            now_ms: frame * 16,
            size: (800, 600),
            events: &[],
            sound_busy: false,
            shift_held: false,
        };
        let out = FrontEnd::frame(&mut front_end, &session.screen(), &input);
        for action in out.actions {
            session.act(action);
        }
        for request in session.take_requests() {
            game.perform(request);
        }
        game.step(&mut session);
        for request in session.take_requests() {
            game.perform(request);
        }

        let now = label(&session.screen());
        if path.last() != Some(&now) {
            path.push(now);
        }
        if game.exited {
            break;
        }
    }

    assert_eq!(
        path,
        [
            "menu", "new game", "briefing", "loading", "hints", "play", "postgame", // the win
            "briefing", // next level
            "loading", "hints", "play", "postgame", // the defeat
            "loading",  // restart, with no briefing
            "hints", "play", "postgame", // the defeat again
            "menu", "quit",
        ]
    );
    assert!(game.exited);
    assert!(game.endings.is_empty());

    let levels: Vec<_> = game.started.iter().map(|s| s.mesh).collect();
    assert_eq!(levels, [WILLOW, SALVATION, SALVATION]);
    let veterans: Vec<_> = game.started.iter().map(|s| s.veterans.clone()).collect();
    assert_eq!(
        veterans,
        [vec![], vec![veteran(1)], vec![veteran(1)]],
        "the win's survivors carry, and restart restores them"
    );
    assert!(game.saves > 0);
    assert!(!front_end.played_sounds.is_empty());
}
