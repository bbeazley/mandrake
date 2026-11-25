use std::path::PathBuf;

use mandrake_common::FourCC;
use mandrake_rec::header::MonsterInitializer;

use crate::action::{Action, GameEvent, GameStart, LevelStart, Request};
use crate::catalog::{Catalog, Level};
use crate::films::{self, FilmFile, FilmFolder};
use crate::progress::Progress;
use crate::report::PostgameReport;
use crate::rules::{self, PostgameChoice, PostgameRule};
use crate::saves::{self, SaveFile, SaveFolder};
use crate::screen::{
    Briefing, Cutscene, DeleteFilm, DeleteSave, FilmDetails, Films, ForcedGameplay, Hints,
    INTRO_CUTSCENE, MainMenu, Narration, NewGame, Overlay, Paused, Postgame, RenameFilm, Restoring,
    SaveFilm, SaveGame, SavedGames, Screen,
};
#[path = "net_flow.rs"]
mod net_flow;
#[path = "tfl_flow.rs"]
mod tfl_flow;

pub const RESTORE_TICKS_PER_STEP: u32 = 0x708;

#[derive(Debug)]
pub struct Session {
    catalog: Catalog,
    progress: Progress,
    main_menu: MainMenu,
    state: State,
    game: GameSetup,
    veterans: Vec<MonsterInitializer>,
    veterans_backup: Vec<MonsterInitializer>,
    reset_rankings: bool,
    net_load: Option<GameStart>,
    mission_advances: u32,
    requests: Vec<Request>,
    progress_changed: bool,
    films: FilmFolder,
    film_row: Option<usize>,
    saves: SaveFolder,
    restoring: (crate::text::Text, u32),
    save_film_wanted: bool,
    film_saved: Option<Vec<u8>>,
    film_deleted: Option<usize>,
    renamed_tags: Vec<(Vec<u8>, crate::text::Text)>,
    rename_tag_name: Option<crate::text::Text>,
    last_saved: Option<SaveFile>,
    last_saved_deleted: Option<mandrake_common::FourCC>,
    last_saved_gone: bool,
    autosave_next: bool,
    networking: Option<Box<crate::net::Networking>>,
    net: Option<net_flow::NetGame>,
    show_ping: bool,
    game_kind: crate::net::GameKind,
    tfl_data: crate::net::DataChecksums,
    tfl: Option<Box<crate::tfl::TflGame>>,
}

#[derive(Debug)]
struct GameSetup {
    mesh: Option<FourCC>,
    plugin_checksum: i32,
    difficulty: i16,
    plugins: Vec<String>,
    film: Option<PathBuf>,
    restore: bool,
    start: LevelStart,
    net: Option<crate::net::NetGameStart>,
}

impl Default for GameSetup {
    fn default() -> Self {
        Self {
            mesh: None,
            plugin_checksum: -1,
            difficulty: 0,
            plugins: Vec::new(),
            film: None,
            restore: false,
            start: LevelStart::default(),
            net: None,
        }
    }
}

impl GameSetup {
    fn can_save_game(&self) -> bool {
        self.film.is_none() && self.net.is_none() && self.mesh != Some(rules::TUTORIAL)
    }
}

#[derive(Debug)]
enum State {
    MainMenu,
    NewGame(NewGame),
    Briefing(Briefing),
    Loading,
    Playing(Option<OpenOverlay>),
    CampaignEnd(Narration, AfterMap),
    Postgame(AfterMap),
    Films(Films),
    FilmReport(PostgameReport),
    SavedGames(SavedGames),
    Restoring(Restoring),
    Cutscene(Cutscene, AfterFilm),
    Net(net_flow::NetState),
    Quit,
}

#[derive(Debug)]
enum AfterFilm {
    MainMenu,
    Load,
    Show(Box<State>),
    Tfl(crate::tfl::AfterMovie),
}

#[derive(Debug)]
enum OpenOverlay {
    Hints(Hints),
    Paused(Paused),
    SaveGame(SaveGame),
    SavedGames(SavedGames),
    NewGame(NewGame),
    UnitTrading(crate::screen::UnitTrading),
}

#[derive(Debug)]
struct AfterMap {
    screen: Postgame,
    next_mesh: Option<FourCC>,
    skip_mesh: Option<FourCC>,
    network: Option<net_flow::NetEpilogue>,
}

impl Session {
    pub fn new(catalog: Catalog, progress: Progress) -> Self {
        let main_menu = MainMenu {
            new_game: true,
            tutorial: true,
            multiplayer: false,
            saved_games: false,
            films: false,
            intro: false,
            preferences: false,
            music: catalog.menu_music.clone(),
        };
        let game = GameSetup {
            difficulty: progress.difficulty,
            ..GameSetup::default()
        };
        Self {
            catalog,
            progress,
            main_menu,
            state: State::MainMenu,
            game,
            veterans: Vec::new(),
            veterans_backup: Vec::new(),
            reset_rankings: false,
            net_load: None,
            mission_advances: 0,
            requests: Vec::new(),
            progress_changed: false,
            films: FilmFolder::default(),
            film_row: None,
            saves: SaveFolder::default(),
            restoring: Default::default(),
            save_film_wanted: false,
            film_saved: None,
            film_deleted: None,
            renamed_tags: Vec::new(),
            rename_tag_name: None,
            last_saved: None,
            last_saved_deleted: None,
            last_saved_gone: false,
            autosave_next: false,
            networking: None,
            net: None,
            show_ping: true,
            game_kind: crate::net::GameKind::Myth2,
            tfl_data: [0, mandrake_net::tfl::NO_PLUGIN],
            tfl: None,
        }
    }

    pub fn set_tfl(&mut self, data: crate::net::DataChecksums) {
        self.game_kind = crate::net::GameKind::Tfl;
        self.tfl_data = data;
    }

    pub fn game_kind(&self) -> crate::net::GameKind {
        self.game_kind
    }

    pub fn screen(&self) -> Screen<'_> {
        if let State::Playing(_) = &self.state
            && let Some(r) = self.net.as_ref().and_then(net_flow::NetGame::restoring)
        {
            return Screen::Restoring(r);
        }
        match &self.state {
            State::MainMenu => Screen::MainMenu(&self.main_menu),
            State::NewGame(new_game) => Screen::NewGame(new_game),
            State::Briefing(briefing) => Screen::Briefing(briefing),
            State::Loading => Screen::Loading,
            State::Playing(overlay) => Screen::Playing {
                overlay: overlay.as_ref().map(|overlay| match overlay {
                    OpenOverlay::Hints(hints) => Overlay::Hints(hints),
                    OpenOverlay::Paused(paused) => Overlay::Paused(paused),
                    OpenOverlay::SaveGame(prompt) => Overlay::SaveGame(prompt),
                    OpenOverlay::SavedGames(saves) => Overlay::SavedGames(saves),
                    OpenOverlay::NewGame(new_game) => Overlay::NewGame(new_game),
                    OpenOverlay::UnitTrading(trading) => Overlay::UnitTrading(trading),
                }),
            },
            State::CampaignEnd(narration, _) => Screen::CampaignEnd(narration),
            State::Postgame(after) if after.network.is_some_and(|e| e.hidden()) => Screen::Loading,
            State::Postgame(after) => Screen::Postgame(&after.screen),
            State::Films(films) => Screen::Films(films),
            State::FilmReport(report) => Screen::FilmReport(report),
            State::SavedGames(saves) => Screen::SavedGames(saves),
            State::Restoring(restoring) => Screen::Restoring(restoring),
            State::Cutscene(cutscene, _) => Screen::Cutscene(cutscene),
            State::Net(state) => {
                Screen::Net(net_flow::screen(state, self.net.as_ref(), self.show_ping))
            }
            State::Quit => Screen::Quit,
        }
    }

    pub fn act(&mut self, action: Action) {
        let tfl_net = self.tfl.is_some() && self.tfl_networked(Some(&action));
        if self.tfl.is_some() && !tfl_net {
            self.tfl_act(action);
            self.save_progress_if_changed();
            return;
        }
        if tfl_net {
            self.myth2_act(action);
            self.tfl_after_network();
            return;
        }
        self.myth2_act(action);
    }

    fn myth2_act(&mut self, action: Action) {
        let Some(action) = self.net_act(action) else {
            self.save_progress_if_changed();
            return;
        };
        match &self.state {
            State::MainMenu => self.main_menu_act(action),
            State::NewGame(_) | State::Playing(Some(OpenOverlay::NewGame(_))) => {
                self.new_game_act(action);
            }
            State::Briefing(_) => match action {
                Action::BriefingDone => self.briefing_done(),
                action => tracing::debug!(?action, screen = "briefing", "session: ignored"),
            },
            State::Playing(Some(OpenOverlay::SaveGame(_))) => self.save_game_overlay_act(action),
            State::SavedGames(_) | State::Playing(Some(OpenOverlay::SavedGames(_))) => {
                self.saved_games_act(action);
            }
            State::Playing(_) => self.playing_act(action),
            State::CampaignEnd(..) => match action {
                Action::NarrationDone => {
                    let State::CampaignEnd(_, after) =
                        std::mem::replace(&mut self.state, State::MainMenu)
                    else {
                        unreachable!("matched above");
                    };
                    self.state = State::Postgame(after);
                    self.net_epilogue_opened();
                }
                action => tracing::debug!(?action, screen = "campaign end", "session: ignored"),
            },
            State::Postgame(_) => self.postgame_act(action),
            State::Films(_) => self.films_act(action),
            State::FilmReport(_) => match action {
                Action::CloseReport => self.state = State::MainMenu,
                action => tracing::debug!(?action, screen = "film report", "session: ignored"),
            },
            state => {
                tracing::debug!(?action, screen = state.name(), "session: ignored");
            }
        }
        self.save_progress_if_changed();
    }

    fn main_menu_act(&mut self, action: Action) {
        match action {
            Action::NewGame { all_levels } => self.open_new_game(all_levels),
            Action::Tutorial => self.start_tutorial(),
            Action::OpenFilms if self.main_menu.films => {
                self.state = State::Films(Films::default());
                self.requests.push(Request::ListFilms);
            }
            Action::OpenSavedGames if self.main_menu.saved_games => {
                self.state = State::SavedGames(SavedGames::default());
                self.requests.push(Request::ListSaves);
            }
            Action::ReplayIntro if self.main_menu.intro => {
                self.play_cutscene(INTRO_CUTSCENE.to_owned(), AfterFilm::MainMenu);
            }
            Action::Quit => {
                self.state = State::Quit;
                self.requests.push(Request::Exit);
            }
            action => tracing::debug!(?action, screen = "main menu", "session: ignored"),
        }
    }

    fn new_game_act(&mut self, action: Action) {
        match (&mut self.state, action) {
            (
                State::NewGame(_) | State::Playing(Some(OpenOverlay::NewGame(_))),
                Action::ChooseCampaign { index, all_levels },
            ) => self.choose_campaign(index, all_levels),
            (
                State::NewGame(_) | State::Playing(Some(OpenOverlay::NewGame(_))),
                Action::ChooseLevel(mesh),
            ) => self.choose_level(mesh),
            (
                State::NewGame(new_game) | State::Playing(Some(OpenOverlay::NewGame(new_game))),
                Action::ChooseDifficulty(difficulty),
            ) => {
                if usize::try_from(difficulty).is_ok_and(|d| d < new_game.difficulties.len()) {
                    new_game.difficulty = difficulty;
                } else {
                    tracing::debug!(difficulty, "session: no such difficulty");
                }
            }
            (State::NewGame(_), Action::StartLevel) => self.start_new_game(),
            (State::Playing(Some(OpenOverlay::NewGame(new_game))), Action::StartLevel)
                if new_game.level.is_some() =>
            {
                self.requests.push(Request::DropGame);
                self.start_new_game();
            }
            (State::NewGame(_), Action::Cancel) => {
                self.remember_new_game_choices();
                self.set_last_saved(None);
                self.state = State::MainMenu;
            }
            (State::Playing(overlay @ Some(OpenOverlay::NewGame(_))), Action::Cancel) => {
                let Some(OpenOverlay::NewGame(new_game)) = overlay.take() else {
                    unreachable!("matched above");
                };
                self.remember_choices_of(&new_game);
                self.set_last_saved(None);
                self.requests.push(Request::SetPaused(false));
            }
            (State::Playing(Some(OpenOverlay::NewGame(_))), action) => {
                tracing::debug!(?action, "session: New Game is open over the game");
            }
            (state, action) => {
                tracing::debug!(?action, screen = state.name(), "session: ignored");
            }
        }
    }

    fn playing_act(&mut self, action: Action) {
        match (&mut self.state, action) {
            (State::Playing(overlay @ None), Action::OpenPause) => {
                let tfl_net = self
                    .game
                    .net
                    .as_ref()
                    .is_some_and(|n| n.kind == crate::net::GameKind::Tfl);
                let owner = self.net.as_ref().is_some_and(net_flow::NetGame::owner);
                *overlay = Some(OpenOverlay::Paused(Paused {
                    can_save: self.game.can_save_game()
                        || (self.game.net.is_some()
                            && self
                                .net
                                .as_ref()
                                .is_some_and(net_flow::NetGame::save_allowed)),
                    can_open: self.saves.any && self.game.net.is_none(),
                    can_revert: self.last_saved.is_some()
                        && self.game.film.is_none()
                        && self.game.net.is_none(),
                    can_restart: self.game.film.is_none() && self.game.net.is_none(),
                    replay: self.game.film.is_some(),
                    network: self.game.net.as_ref().map(|net| crate::screen::NetPause {
                        originator: net.host || owner,
                        rehost_items: (net.host || owner)
                            && !tfl_net
                            && !self.net.as_ref().is_some_and(net_flow::NetGame::game_over),
                        can_rehost: !tfl_net,
                    }),
                }));
                if !tfl_net {
                    self.requests.push(Request::SetPaused(true));
                }
            }
            (State::Playing(None), Action::OpenHints) => self.open_hints(),
            (
                State::Playing(None | Some(OpenOverlay::Paused(_) | OpenOverlay::Hints(_))),
                Action::Quicksave,
            ) if self.game.can_save_game() => {
                self.requests.push(Request::Quicksave);
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::OpenSavedGames)
                if paused.can_open =>
            {
                self.state = State::Playing(Some(OpenOverlay::SavedGames(SavedGames::default())));
                self.requests.push(Request::ListSaves);
            }
            (State::Playing(overlay @ Some(OpenOverlay::Hints(_))), Action::CloseHints) => {
                *overlay = None;
                self.requests.push(Request::SetPaused(false));
            }
            (State::Playing(Some(OpenOverlay::Hints(hints))), Action::SetShowHints(show)) => {
                hints.show_at_start = show;
                if self.progress.show_hints != show {
                    self.progress.show_hints = show;
                    self.progress_changed = true;
                }
            }
            (State::Playing(Some(OpenOverlay::Hints(_))), Action::SetHintIndex(index)) => {
                if self.progress.hint_index != index {
                    self.progress.hint_index = index;
                    self.progress_changed = true;
                }
            }
            (State::Playing(overlay @ Some(OpenOverlay::Paused(_))), Action::Resume) => {
                *overlay = None;
                if !self
                    .game
                    .net
                    .as_ref()
                    .is_some_and(|n| n.kind == crate::net::GameKind::Tfl)
                {
                    self.requests.push(Request::SetPaused(false));
                }
            }
            (
                State::Playing(Some(OpenOverlay::Paused(paused))),
                action @ (Action::LastSavedGame | Action::RestartLevel),
            ) if paused
                .network
                .is_some_and(|n| n.rehost_items && n.can_rehost)
                && self
                    .game
                    .net
                    .as_ref()
                    .is_some_and(|n| n.kind.commands().forced_end.is_some()) =>
            {
                self.state = State::Playing(None);
                self.requests.push(Request::SetPaused(false));
                let kind = if action == Action::LastSavedGame {
                    2
                } else {
                    3
                };
                self.requests
                    .push(Request::Net(crate::net::NetRequest::ForceEnd(kind)));
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::RestartLevel)
                if paused.can_restart =>
            {
                self.requests.push(Request::DropGame);
                self.restart_level();
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::SaveGame)
                if paused.can_save =>
            {
                self.open_save_game();
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::LastSavedGame)
                if paused.can_revert && self.last_saved_gone =>
            {
                self.revert_to_missing_save_in_game();
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::LastSavedGame)
                if paused.can_revert =>
            {
                self.requests.push(Request::DropGame);
                self.revert_to_last_saved_game();
            }
            (State::Playing(Some(OpenOverlay::Paused(_))), Action::QuitToMenu) => {
                self.game.film = None;
                self.game.restore = false;
                self.requests.push(Request::DropGame);
                self.state = State::MainMenu;
            }
            (state, action) => {
                tracing::debug!(?action, screen = state.name(), "session: ignored");
            }
        }
    }

    pub(super) fn save_game_overlay_act(&mut self, action: Action) {
        let is_tfl = self.tfl.is_some();
        match (&mut self.state, action) {
            (State::Playing(Some(OpenOverlay::SaveGame(prompt))), Action::SaveGameAs(name))
                if prompt.replace.is_none() =>
            {
                self.save_game_as(name);
            }
            (State::Playing(Some(OpenOverlay::SaveGame(prompt))), Action::ReplaceSave) => {
                if let Some(name) = prompt.replace.take() {
                    self.save_game(name);
                }
            }
            (State::Playing(Some(OpenOverlay::SaveGame(prompt))), Action::Cancel)
                if prompt.replace.is_some() =>
            {
                prompt.replace = None;
            }
            (State::Playing(overlay @ Some(OpenOverlay::SaveGame(_))), Action::Cancel) => {
                *overlay = None;
                if !is_tfl {
                    self.requests.push(Request::SetPaused(false));
                }
            }
            (State::Playing(Some(OpenOverlay::SaveGame(_))), action) => {
                tracing::debug!(?action, "session: Save Game's prompt is open");
            }
            _ => {}
        }
    }

    fn postgame_act(&mut self, action: Action) {
        match (&mut self.state, action) {
            (State::Postgame(after), Action::SaveFilmAs(name))
                if after.screen.save_film.is_some() =>
            {
                self.save_film_as(name);
            }
            (State::Postgame(after), Action::Cancel) if after.screen.save_film.is_some() => {
                after.screen.save_film = None;
            }
            (State::Postgame(after), _) if after.screen.save_film.is_some() => {
                tracing::debug!("session: the Save Film prompt is open");
            }
            (State::Postgame(after), Action::SaveFilm) if after.screen.exits.save_film => {
                self.want_save_film();
            }
            (State::Postgame(after), Action::NextLevel) if after.screen.exits.next => {
                let mesh = after.next_mesh;
                self.autosave_next = mesh.is_some();
                self.change_level(mesh);
            }
            (State::Postgame(after), Action::SkipLevel) if after.screen.exits.skip => {
                let mesh = after.skip_mesh;
                self.change_level(mesh);
            }
            (State::Postgame(after), Action::RestartLevel) if after.screen.exits.restart => {
                self.restart_level();
            }
            (State::Postgame(after), Action::LastSavedGame) if after.screen.exits.last_saved => {
                self.revert_to_last_saved_game();
            }
            (State::Postgame(_), Action::QuitToMenu) => self.state = State::MainMenu,
            (state, action) => {
                tracing::debug!(?action, screen = state.name(), "session: ignored");
            }
        }
    }

    fn films_act(&mut self, action: Action) {
        match (&mut self.state, action) {
            (State::Films(films), Action::SaveFilmAs(name)) if films.save_film.is_some() => {
                self.save_film_as(name);
            }
            (State::Films(films), Action::Cancel) if films.save_film.is_some() => {
                films.save_film = None;
            }
            (State::Films(films), _) if films.save_film.is_some() => {
                tracing::debug!("session: the Save Film prompt is open");
            }
            (State::Films(films), Action::ConfirmDeleteFilm) if films.delete_film.is_some() => {
                self.delete_film();
            }
            (State::Films(films), Action::Cancel) if films.delete_film.is_some() => {
                films.delete_film = None;
            }
            (State::Films(films), _) if films.delete_film.is_some() => {
                tracing::debug!("session: Delete Film's question is open");
            }
            (State::Films(films), Action::CloseNotice)
                if films.rename_film.as_ref().is_some_and(|r| r.exists) =>
            {
                if let Some(prompt) = &mut films.rename_film {
                    prompt.exists = false;
                }
            }
            (State::Films(films), _) if films.rename_film.as_ref().is_some_and(|r| r.exists) => {
                tracing::debug!("session: the notice over Rename Film is open");
            }
            (State::Films(films), Action::RenameFilmAs(name)) if films.rename_film.is_some() => {
                self.rename_film_as(&name);
            }
            (State::Films(films), Action::Cancel) if films.rename_film.is_some() => {
                films.rename_film = None;
                self.renamed(None);
            }
            (State::Films(films), _) if films.rename_film.is_some() => {
                tracing::debug!("session: Rename Film's prompt is open");
            }
            (State::Films(films), Action::DeleteFilm) if films.delete => self.ask_delete_film(),
            (State::Films(films), Action::RenameFilm) if films.rename => self.open_rename_film(),
            (State::Films(films), Action::SaveFilm) if films.save_last => self.want_save_film(),
            (State::Films(_), Action::ChooseFilm(row)) => self.choose_film(Some(row)),
            (State::Films(films), Action::ViewFilm) if films.can_view && !films.find_plugins => {
                let row = films.selected;
                if let Some(film) = row.and_then(|row| self.films.films.get(row)).cloned() {
                    self.film_row = row;
                    self.play_film(&film);
                }
            }
            (State::Films(films), Action::ReplayLastFilm) if films.last_film => {
                if let Some(film) = self.films.last.clone() {
                    self.play_film(&film);
                }
            }
            (State::Films(_), Action::Cancel) => self.state = State::MainMenu,
            (state, action) => {
                tracing::debug!(?action, screen = state.name(), "session: ignored");
            }
        }
    }

    pub(super) fn saved_games_act(&mut self, action: Action) {
        let Some(saves) = self.saved_games_mut() else {
            return;
        };
        let question = saves.delete_save.is_some();
        let (selected, can_open, delete) = (saves.selected, saves.can_open, saves.delete);
        match action {
            Action::ConfirmDeleteSave if question => self.delete_save(),
            Action::Cancel if question => {
                if let Some(saves) = self.saved_games_mut() {
                    saves.delete_save = None;
                }
            }
            _ if question => tracing::debug!("session: Delete's question is open"),
            Action::DeleteSave if delete => self.ask_delete_save(),
            Action::ChooseSave(row) => self.choose_save(Some(row)),
            Action::OpenSave if selected.is_none() => self.leave_saved_games(),
            Action::OpenSave if can_open => {
                if let Some(save) = selected.and_then(|row| self.saves.saves.get(row)).cloned() {
                    if matches!(self.state, State::Playing(_)) {
                        self.requests.push(Request::DropGame);
                    }
                    if self.tfl.is_some() {
                        self.tfl_continue_saved_game(&save);
                    } else {
                        self.continue_saved_game(&save);
                    }
                }
            }
            Action::Cancel => self.leave_saved_games(),
            action => tracing::debug!(?action, "session: Saved Games is open"),
        }
    }

    pub fn game_event(&mut self, event: GameEvent) {
        if self.tfl.is_some() {
            if matches!(event, GameEvent::Net(_)) || self.tfl_networked(None) {
                self.myth2_game_event(event);
                self.tfl_after_network();
                return;
            }
            self.tfl_event(event);
            self.save_progress_if_changed();
            return;
        }
        self.myth2_game_event(event);
    }

    fn myth2_game_event(&mut self, event: GameEvent) {
        let Some(event) = self.net_game_event(event) else {
            self.save_progress_if_changed();
            return;
        };
        match (&self.state, event) {
            (_, GameEvent::FilmsListed(folder)) => self.films_listed(folder),
            (_, GameEvent::IntroFound(found)) => self.main_menu.intro = found,
            (State::Cutscene(..), GameEvent::CutsceneOver) => self.cutscene_over(),
            (_, GameEvent::SavesListed(folder)) => self.saves_listed(folder),
            (_, GameEvent::FilmSaved(saved)) => self.film_saved(saved),
            (State::Films(_), GameEvent::FilmRenamed(file_name)) => self.renamed(Some(file_name)),
            (_, GameEvent::GameSaved(saved)) => self.game_saved(saved),
            (State::Playing(Some(OpenOverlay::SaveGame(_))), GameEvent::Snapshot(snapshot)) => {
                self.snapshot_taken(snapshot);
            }
            (State::Loading | State::Restoring(_), GameEvent::Loaded) => self.begin_play(),
            (State::Loading | State::Restoring(_), GameEvent::Restoring { ticks })
                if self.game.restore =>
            {
                self.restoring(ticks);
            }
            (State::Loading | State::Restoring(_), GameEvent::LoadFailed(error)) => {
                tracing::error!(%error, "session: the level didn't load");
                self.autosave_next = false;
                self.game.film = None;
                self.game.restore = false;
                self.state = State::MainMenu;
            }
            (State::Playing(_), GameEvent::MissionAdvanced(advances)) => {
                self.mission_advances = advances;
            }
            (State::Playing(_), GameEvent::MapOver { report, veterans }) => {
                self.map_over(report, veterans);
            }
            (state, event) => {
                tracing::debug!(?event, screen = state.name(), "session: ignored");
            }
        }
        self.save_progress_if_changed();
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    fn films_listed(&mut self, folder: FilmFolder) {
        self.main_menu.films = folder.any;
        self.films = folder;
        if let State::Films(films) = &self.state {
            let prompt = films.save_film.clone();
            let saved = self
                .film_saved
                .take()
                .and_then(|name| self.films.films.iter().position(|f| f.file_name == name));
            let deleted = self.film_deleted.take().map(|row| row.saturating_sub(1));
            let row = saved
                .or(deleted)
                .or_else(|| self.film_row.take())
                .filter(|&row| row < self.films.films.len())
                .or_else(|| (!self.films.films.is_empty()).then_some(0));
            self.choose_film(row);
            if let State::Films(films) = &mut self.state {
                films.save_film = prompt;
            }
        }
        if std::mem::take(&mut self.save_film_wanted) {
            self.open_save_film();
        }
    }

    fn want_save_film(&mut self) {
        self.save_film_wanted = true;
        self.requests.push(Request::ListFilms);
    }

    fn open_save_film(&mut self) {
        if !self.films.last_exists {
            tracing::info!("session: no last recording to save");
            return;
        }
        let prompt = SaveFilm {
            names: self.films.names.clone(),
            name: self.films.untitled.clone(),
            last_name: self.films.last_name.clone(),
        };
        match &mut self.state {
            State::Postgame(after) => after.screen.save_film = Some(prompt),
            State::Films(films) => films.save_film = Some(prompt),
            state => tracing::debug!(screen = state.name(), "session: no Save Film here"),
        }
    }

    fn save_film_as(&mut self, name: crate::text::Text) {
        let is_tfl = self.tfl.is_some();
        let prompt = match &mut self.state {
            State::Postgame(after) => &mut after.screen.save_film,
            State::Films(films) => &mut films.save_film,
            _ => return,
        };
        let Some(open) = prompt else {
            return;
        };
        let ok = if is_tfl {
            !name.0.eq_ignore_ascii_case(films::tfl::LAST_RECORDING_FILE)
                && !name.0.eq_ignore_ascii_case(&open.last_name.0)
                && saves::save_name_ok(&name.0)
        } else {
            films::film_name_ok(&name.0, &open.last_name.0)
        };
        if !ok {
            tracing::debug!(name = %name.to_plain_utf8(), "session: not a film's name");
            return;
        }
        *prompt = None;
        self.requests.push(Request::SaveFilm { name });
    }

    fn chosen_film(&self) -> Option<(usize, &FilmFile)> {
        let State::Films(films) = &self.state else {
            return None;
        };
        let row = films.selected?;
        Some((row, self.films.films.get(row)?))
    }

    fn ask_delete_film(&mut self) {
        let Some((_, film)) = self.chosen_film() else {
            return;
        };
        let name = self
            .renamed_tags
            .iter()
            .find(|(file, _)| *file == film.file_name)
            .map_or_else(
                || films::found_tag_name(&film.file_name),
                |(_, n)| n.clone(),
            );
        if let State::Films(films) = &mut self.state {
            films.delete_film = Some(DeleteFilm { name });
        }
    }

    fn delete_film(&mut self) {
        let Some((row, film)) = self.chosen_film() else {
            return;
        };
        let file_name = film.file_name.clone();
        if let State::Films(films) = &mut self.state {
            films.delete_film = None;
        }
        self.renamed_tags.retain(|(file, _)| *file != file_name);
        self.film_deleted = Some(row);
        self.requests.push(Request::DeleteFilm { file_name });
        self.requests.push(Request::ListFilms);
    }

    fn open_rename_film(&mut self) {
        let Some((_, film)) = self.chosen_film() else {
            return;
        };
        let initial = if self.tfl.is_some() {
            film.file_name.clone()
        } else {
            films::display_name(&film.file_name)
        };
        let prompt = RenameFilm {
            names: self.films.names.clone(),
            name: crate::text::Text(initial),
            exists: false,
        };
        if let State::Films(films) = &mut self.state {
            films.rename_film = Some(prompt);
        }
    }

    fn rename_film_as(&mut self, name: &crate::text::Text) {
        let is_tfl = self.tfl.is_some();
        let ok = if is_tfl {
            !name.0.eq_ignore_ascii_case(films::tfl::LAST_RECORDING_FILE)
                && saves::save_name_ok(&name.0)
        } else {
            films::rename_name_ok(&name.0)
        };
        if !ok {
            tracing::debug!(name = %name.to_plain_utf8(), "session: not a film's name");
            return;
        }
        let Some((_, film)) = self.chosen_film() else {
            return;
        };
        let file_name = film.file_name.clone();
        let exists = name.0 != file_name
            && self
                .films
                .file_names
                .iter()
                .any(|f| f.eq_ignore_ascii_case(&name.0));
        let State::Films(films) = &mut self.state else {
            return;
        };
        if exists {
            if let Some(prompt) = &mut films.rename_film {
                prompt.exists = true;
            }
            return;
        }
        films.rename_film = None;
        if name.0 == file_name {
            self.renamed(None);
            return;
        }
        self.rename_tag_name = if is_tfl {
            mandrake_rec::writer::save_names(&name.0)
                .map(|(tag_name, _, _)| crate::text::Text(tag_name))
        } else {
            mandrake_rec::writer::film_names(&name.0)
                .map(|(tag_name, _, _)| crate::text::Text(tag_name))
        };
        self.requests.push(Request::RenameFilm {
            file_name,
            name: name.clone(),
        });
    }

    fn renamed(&mut self, file_name: Option<Vec<u8>>) {
        let old = self.chosen_film().map(|(_, f)| f.file_name.clone());
        let tag_name = self.rename_tag_name.take();
        if let (Some(new), Some(tag_name)) = (&file_name, tag_name)
            && Some(new) != old.as_ref()
        {
            self.renamed_tags
                .retain(|(file, _)| Some(file) != old.as_ref());
            self.renamed_tags.push((new.clone(), tag_name));
        }
        self.film_saved = file_name.or(old);
        self.requests.push(Request::ListFilms);
    }

    fn film_saved(&mut self, saved: Option<Vec<u8>>) {
        let Some(file_name) = saved else {
            return;
        };
        match &mut self.state {
            State::Postgame(after) => after.screen.exits.save_film = false,
            State::Films(_) => {
                self.film_saved = Some(file_name);
                self.requests.push(Request::ListFilms);
            }
            _ => {}
        }
    }

    fn choose_film(&mut self, row: Option<usize>) {
        let State::Films(films) = &mut self.state else {
            return;
        };
        let row = row.filter(|&row| row < self.films.films.len());
        let chosen = row.map(|row| &self.films.films[row]);
        *films = Films {
            names: self.films.films.iter().map(|f| f.name.clone()).collect(),
            selected: row,
            details: chosen.map(|f| FilmDetails {
                name: films::short_name(&f.name),
                game: f.game.clone(),
            }),
            can_view: chosen.is_some_and(|f| f.game.map_name.is_some() || f.game.plugins_missing),
            find_plugins: chosen
                .is_some_and(|f| f.game.map_name.is_none() || !f.game.plugins_present),
            last_film: self.films.last.is_some(),
            save_last: self.films.last.is_some(),
            delete: chosen.is_some(),
            rename: chosen.is_some_and(|f| f.file_name != self.films.last_name.0),
            save_film: None,
            delete_film: None,
            rename_film: None,
        };
    }

    fn play_film(&mut self, film: &FilmFile) {
        let plugins: Vec<String> = film.game.plugins.iter().map(|p| p.name.clone()).collect();
        let plugin_checksum = self.plugin_checksum_for_mesh(
            film.game.mesh,
            film.game.plugins.iter().rev().map(|p| p.checksum),
            &plugins,
        );
        self.game = GameSetup {
            mesh: Some(film.game.mesh),
            plugin_checksum,
            difficulty: film.game.difficulty,
            plugins,
            film: Some(film.path.clone()),
            restore: false,
            start: LevelStart::ChangeMap,
            net: None,
        };
        self.load_level();
    }

    fn saves_listed(&mut self, folder: SaveFolder) {
        self.main_menu.saved_games = folder.any;
        self.saves = folder;
        if let Some(subgroup) = self.last_saved_deleted.take() {
            match self.saves.saves.iter().find(|s| s.subgroup == subgroup) {
                Some(save) => self.last_saved = Some(save.clone()),
                None => self.last_saved_gone = true,
            }
        }
        if let State::Playing(Some(OpenOverlay::SaveGame(prompt))) = &mut self.state {
            prompt.names = self.saves.saves.iter().map(|s| s.name.clone()).collect();
        }
        if self.saved_games_mut().is_some() {
            let row = (!self.saves.saves.is_empty()).then_some(0);
            self.choose_save(row);
        }
    }

    fn saved_games_mut(&mut self) -> Option<&mut SavedGames> {
        match &mut self.state {
            State::SavedGames(saves) | State::Playing(Some(OpenOverlay::SavedGames(saves))) => {
                Some(saves)
            }
            State::Net(net_flow::NetState::GameOptions(o)) => o.picker.as_deref_mut(),
            _ => None,
        }
    }

    fn leave_saved_games(&mut self) {
        if let State::Playing(overlay) = &mut self.state {
            *overlay = None;
            self.requests.push(Request::SetPaused(false));
        } else {
            self.state = State::MainMenu;
        }
    }

    fn choose_save(&mut self, row: Option<usize>) {
        if self.saved_games_mut().is_none() {
            return;
        }
        let row = row.filter(|&row| row < self.saves.saves.len());
        let chosen = row.map(|row| &self.saves.saves[row]);
        let shown = SavedGames {
            names: self.saves.saves.iter().map(|s| s.name.clone()).collect(),
            selected: row,
            snapshot: chosen.and_then(|s| s.snapshot.clone()),
            date: chosen.map(|s| s.date.clone()).unwrap_or_default(),
            can_open: chosen.is_some_and(|s| s.game.plugins_present && s.game.map_name.is_some()),
            delete: true,
            delete_save: None,
        };
        if let Some(saves) = self.saved_games_mut() {
            *saves = shown;
        }
    }

    fn ask_delete_save(&mut self) {
        let selected = self.saved_games_mut().and_then(|saves| saves.selected);
        let Some(save) = selected.and_then(|row| self.saves.saves.get(row)) else {
            return;
        };
        let question = DeleteSave {
            name: films::found_tag_name(&save.name.0),
        };
        if let Some(saves) = self.saved_games_mut() {
            saves.delete_save = Some(question);
        }
    }

    fn delete_save(&mut self) {
        let Some(saves) = self.saved_games_mut() else {
            return;
        };
        saves.delete_save = None;
        let selected = saves.selected;
        let Some(save) = selected.and_then(|row| self.saves.saves.get(row)) else {
            return;
        };
        if self
            .last_saved
            .as_ref()
            .is_some_and(|l| l.path == save.path)
        {
            self.last_saved_deleted = Some(save.subgroup);
        }
        self.requests.push(Request::DeleteSave {
            file_name: save.name.0.clone(),
        });
        self.requests.push(Request::ListSaves);
    }

    fn set_last_saved(&mut self, save: Option<SaveFile>) {
        self.last_saved = save;
        self.last_saved_deleted = None;
        self.last_saved_gone = false;
    }

    fn revert_to_missing_save_in_game(&mut self) {
        let new_game = self.new_game_dialog(false);
        self.state = State::Playing(Some(OpenOverlay::NewGame(new_game)));
    }

    fn continue_saved_game(&mut self, save: &SaveFile) {
        let plugins: Vec<String> = save.game.plugins.iter().map(|p| p.name.clone()).collect();
        let plugin_checksum = self.plugin_checksum_for_mesh(
            save.game.mesh,
            save.game.plugins.iter().rev().map(|p| p.checksum),
            &plugins,
        );
        self.game = GameSetup {
            mesh: Some(save.game.mesh),
            plugin_checksum,
            difficulty: save.game.difficulty,
            plugins,
            film: Some(save.path.clone()),
            restore: true,
            start: LevelStart::ChangeMap,
            net: None,
        };
        if self.progress.difficulty != save.game.difficulty {
            self.progress.difficulty = save.game.difficulty;
            self.progress_changed = true;
        }
        self.veterans.clone_from(&save.veterans);
        self.veterans_backup.clone_from(&save.veterans);
        self.reset_rankings = false;
        self.restoring = (save.name.clone(), save.end_tick);
        self.set_last_saved(Some(save.clone()));
        self.load_level();
    }

    fn revert_to_last_saved_game(&mut self) {
        if self.last_saved_gone {
            let mut new_game = self.new_game_dialog(false);
            new_game.no_menu = true;
            self.state = State::NewGame(new_game);
            return;
        }
        match self.last_saved.clone() {
            Some(save) => self.continue_saved_game(&save),
            None => tracing::debug!("session: no saved game to revert to"),
        }
    }

    fn open_save_game(&mut self) {
        let name = match &self.last_saved {
            Some(save) if !save.name.0.starts_with(saves::AUTOSAVE_PREFIX) => save.name.clone(),
            _ => self.saves.new_name.clone(),
        };
        let prompt = SaveGame {
            names: self.saves.saves.iter().map(|s| s.name.clone()).collect(),
            name,
            snapshot: None,
            replace: None,
        };
        self.state = State::Playing(Some(OpenOverlay::SaveGame(prompt)));
        self.requests.push(Request::TakeSnapshot);
        self.requests.push(Request::ListSaves);
    }

    fn snapshot_taken(&mut self, snapshot: mandrake_rec::saved_game::Snapshot) {
        if let State::Playing(Some(OpenOverlay::SaveGame(prompt))) = &mut self.state {
            prompt.snapshot = Some(snapshot);
        }
    }

    fn save_game_as(&mut self, name: crate::text::Text) {
        if !saves::save_name_ok(&name.0) {
            tracing::debug!(name = %name.to_plain_utf8(), "session: not a save's name");
            return;
        }
        let file_name = mandrake_rec::writer::save_names(&name.0).map(|(_, file, _)| file);
        let taken = self
            .saves
            .saves
            .iter()
            .any(|s| Some(&s.name.0) == file_name.as_ref());
        match (&mut self.state, file_name) {
            (State::Playing(Some(OpenOverlay::SaveGame(prompt))), Some(file)) if taken => {
                prompt.replace = Some(crate::text::Text(file));
            }
            _ => self.save_game(name),
        }
    }

    fn save_game(&mut self, name: crate::text::Text) {
        self.state = State::Playing(None);
        self.requests.push(Request::SaveGame { name });
        self.requests.push(Request::SetPaused(false));
    }

    fn game_saved(&mut self, saved: Option<SaveFile>) {
        let Some(save) = saved else {
            return;
        };
        self.main_menu.saved_games = true;
        self.saves.any = true;
        self.set_last_saved(Some(save));
    }

    fn restoring(&mut self, ticks: u32) {
        let (name, end_tick) = &self.restoring;
        let steps = i64::from(end_tick / RESTORE_TICKS_PER_STEP + 1);
        let done = i64::from(ticks.div_ceil(RESTORE_TICKS_PER_STEP)).min(steps);
        let restoring = Restoring {
            name: name.clone(),
            fraction: ((done << 8) / steps) as i16,
            network: false,
        };
        self.state = State::Restoring(restoring);
    }

    pub fn front_end_has_input(&self) -> bool {
        !matches!(self.state, State::Playing(None))
            || self.net.as_ref().is_some_and(|n| n.restoring().is_some())
    }

    fn open_new_game(&mut self, all_levels: bool) {
        let new_game = self.new_game_dialog(all_levels);
        self.state = State::NewGame(new_game);
    }

    fn new_game_dialog(&self, all_levels: bool) -> NewGame {
        let campaign = self
            .progress
            .last_campaign
            .and_then(|checksum| {
                self.catalog
                    .campaigns
                    .iter()
                    .position(|c| c.checksum == checksum)
            })
            .unwrap_or(0);
        let mut new_game = NewGame {
            campaigns: self
                .catalog
                .campaigns
                .iter()
                .map(|c| c.name.clone())
                .collect(),
            campaign,
            difficulties: self.catalog.difficulties.clone(),
            difficulty: self.progress.difficulty,
            ..NewGame::default()
        };
        self.fill_level_list(&mut new_game, self.progress.last_level, all_levels);
        new_game
    }

    fn new_game_mut(&mut self) -> Option<&mut NewGame> {
        match &mut self.state {
            State::NewGame(new_game) | State::Playing(Some(OpenOverlay::NewGame(new_game))) => {
                Some(new_game)
            }
            _ => None,
        }
    }

    fn new_game_ref(&self) -> Option<&NewGame> {
        match &self.state {
            State::NewGame(new_game) | State::Playing(Some(OpenOverlay::NewGame(new_game))) => {
                Some(new_game)
            }
            _ => None,
        }
    }

    fn fill_level_list(&self, new_game: &mut NewGame, current: Option<FourCC>, all_levels: bool) {
        let list = self
            .catalog
            .campaigns
            .get(new_game.campaign)
            .map(|campaign| {
                rules::level_list(
                    &campaign.entries,
                    &self.progress.visited,
                    current,
                    all_levels,
                )
            })
            .unwrap_or_default();
        new_game.levels = list.levels;
        self.set_chosen_level(new_game, list.selected);
    }

    fn set_chosen_level(&self, new_game: &mut NewGame, mesh: Option<FourCC>) {
        let campaign = self.catalog.campaigns.get(new_game.campaign);
        let level = mesh.and_then(|mesh| {
            campaign
                .and_then(|c| self.catalog.level_of_campaign(mesh, c.checksum))
                .or_else(|| self.catalog.level(mesh))
        });
        let entry = mesh.and_then(|mesh| campaign?.entries.iter().find(|e| e.mesh == mesh));
        new_game.level = mesh;
        new_game.can_start = mesh.is_some();
        new_game.description = level.map(|l| l.description.clone()).unwrap_or_default();
        new_game.preview = level.and_then(|l| l.preview);
        new_game.splash = level.and_then(|l| l.splash);
        new_game.art_plugins = campaign.map(|c| c.plugins.clone()).unwrap_or_default();
        new_game.forced_gameplay = entry.and_then(|e| {
            if e.flags & rules::ENTRY_FORCES_ANTI_CLUMP != 0 {
                Some(ForcedGameplay::AntiClump)
            } else if e.flags & rules::ENTRY_FORCES_TFL != 0 {
                Some(ForcedGameplay::Tfl)
            } else {
                None
            }
        });
        new_game.tfl_conflict = campaign
            .filter(|c| c.checksum != -1)
            .map(|c| c.name.clone());
    }

    fn choose_campaign(&mut self, index: usize, all_levels: bool) {
        if index >= self.catalog.campaigns.len() {
            tracing::debug!(index, "session: no such campaign");
            return;
        }
        let Some(mut new_game) = self.new_game_mut().map(std::mem::take) else {
            unreachable!("only called on New Game");
        };
        new_game.campaign = index;
        let current = new_game.level.or(self.progress.last_level);
        self.fill_level_list(&mut new_game, current, all_levels);
        if let Some(shown) = self.new_game_mut() {
            *shown = new_game;
        }
    }

    fn choose_level(&mut self, mesh: FourCC) {
        let Some(mut new_game) = self.new_game_mut().map(std::mem::take) else {
            unreachable!("only called on New Game");
        };
        if new_game
            .levels
            .iter()
            .any(|l| l.mesh == mesh && l.selectable)
        {
            self.set_chosen_level(&mut new_game, Some(mesh));
        } else {
            tracing::debug!(mesh = %mesh.as_str(), "session: this level can't be chosen");
        }
        if let Some(shown) = self.new_game_mut() {
            *shown = new_game;
        }
    }

    fn remember_new_game_choices(&mut self) {
        if let Some(new_game) = self.new_game_ref().cloned() {
            self.remember_choices_of(&new_game);
        }
    }

    fn remember_choices_of(&mut self, new_game: &NewGame) {
        let Some(mesh) = new_game.level else {
            return;
        };
        let campaign = self.catalog.campaigns.get(new_game.campaign);
        let (last_level, last_campaign, difficulty) = (
            Some(mesh),
            campaign.map(|c| c.checksum),
            new_game.difficulty,
        );
        if self.progress.last_level != last_level
            || self.progress.last_campaign != last_campaign
            || self.progress.difficulty != difficulty
        {
            self.progress.last_level = last_level;
            self.progress.last_campaign = last_campaign;
            self.progress.difficulty = difficulty;
            self.progress_changed = true;
        }
    }

    fn start_new_game(&mut self) {
        let Some(new_game) = self.new_game_ref() else {
            return;
        };
        let Some(mesh) = new_game.level else {
            tracing::debug!("session: no level to start");
            return;
        };
        let campaign = self.catalog.campaigns.get(new_game.campaign);
        self.game = GameSetup {
            mesh: Some(mesh),
            plugin_checksum: campaign.map_or(-1, |c| c.checksum),
            difficulty: new_game.difficulty,
            plugins: campaign.map(|c| c.plugins.clone()).unwrap_or_default(),
            film: None,
            restore: false,
            start: LevelStart::NewGame,
            net: None,
        };
        self.remember_new_game_choices();
        self.set_last_saved(None);
        self.veterans.clear();
        self.reset_rankings = true;
        self.enter_level();
    }

    fn start_tutorial(&mut self) {
        self.game.mesh = Some(rules::TUTORIAL);
        self.game.plugin_checksum = -1;
        self.game.plugins.clear();
        self.game.film = None;
        self.game.restore = false;
        self.game.start = LevelStart::ChangeMap;
        self.veterans.clear();
        self.reset_rankings = true;
        self.enter_level();
    }

    fn change_level(&mut self, mesh: Option<FourCC>) {
        let Some(mesh) = mesh else {
            self.state = State::MainMenu;
            return;
        };
        self.game.mesh = Some(mesh);
        self.game.start = LevelStart::ChangeMap;
        self.enter_level();
    }

    fn enter_level(&mut self) {
        self.veterans_backup.clone_from(&self.veterans);
        let level = self.current_level();
        let visit = level.map(|l| (l.mesh, l.plugin_checksum));
        let briefing = level.filter(|l| rules::has_briefing(l)).map(|l| Briefing {
            art: l.pre_art,
            art_plugins: self.game.plugins.clone(),
            title: l.title.clone(),
            text: l.briefing_text.clone().unwrap_or_default(),
            captions: l.captions.clone(),
            narration: l.narration,
        });
        if let Some((mesh, checksum)) = visit {
            self.progress_changed |= self.progress.mark_visited(mesh, checksum);
        }
        let film = self
            .current_level()
            .and_then(|l| rules::level_film(l, rules::PREGAME_FILM))
            .map(str::to_owned);
        match (briefing, film) {
            (Some(briefing), Some(film)) if self.film_before_briefing() => {
                self.play_cutscene(film, AfterFilm::Show(Box::new(State::Briefing(briefing))));
            }
            (Some(briefing), _) => self.state = State::Briefing(briefing),
            (None, Some(film)) => self.play_cutscene(film, AfterFilm::Load),
            (None, None) => self.load_level(),
        }
    }

    fn film_before_briefing(&self) -> bool {
        self.current_level()
            .is_some_and(|l| l.flags & rules::MESH_FILM_BEFORE_BRIEFING != 0)
    }

    fn briefing_done(&mut self) {
        let film = self
            .current_level()
            .filter(|_| !self.film_before_briefing())
            .and_then(|l| rules::level_film(l, rules::PREGAME_FILM))
            .map(str::to_owned);
        match film {
            Some(film) => self.play_cutscene(film, AfterFilm::Load),
            None => self.load_level(),
        }
    }

    fn play_cutscene(&mut self, name: String, after: AfterFilm) {
        self.requests
            .push(Request::PlayCutscene { name: name.clone() });
        self.state = State::Cutscene(Cutscene { name, movie: None }, after);
    }

    fn cutscene_over(&mut self) {
        let State::Cutscene(_, after) = std::mem::replace(&mut self.state, State::MainMenu) else {
            return;
        };
        match after {
            AfterFilm::MainMenu | AfterFilm::Tfl(_) => {}
            AfterFilm::Load => self.load_level(),
            AfterFilm::Show(state) => {
                self.state = *state;
                self.net_epilogue_opened();
            }
        }
    }

    fn restart_level(&mut self) {
        self.veterans.clone_from(&self.veterans_backup);
        self.game.start = LevelStart::Restart;
        self.load_level();
    }

    fn load_level(&mut self) {
        if let Some(start) = self.net_load.take() {
            self.requests.push(Request::LoadGame(start));
            self.state = State::Loading;
            return;
        }
        let Some(mesh) = self.game.mesh else {
            tracing::error!("session: no level to load");
            self.state = State::MainMenu;
            return;
        };
        self.requests.push(Request::LoadGame(GameStart {
            start: self.game.start,
            mesh,
            difficulty: self.game.difficulty,
            plugins: self.game.plugins.clone(),
            veterans: self.veterans.clone(),
            reset_rankings: std::mem::take(&mut self.reset_rankings),
            film: self.game.film.clone(),
            restore: self.game.restore,
            net: self.game.net.clone(),
        }));
        self.state = State::Loading;
    }

    fn begin_play(&mut self) {
        self.mission_advances = 0;
        self.requests.push(Request::BeginPlay);
        self.state = State::Playing(None);
        if std::mem::take(&mut self.autosave_next) && !self.game.restore {
            self.requests.push(Request::Autosave);
        }
        if std::mem::take(&mut self.game.restore) {
            self.game.film = None;
            return;
        }
        if self.progress.show_hints && self.game.film.is_none() {
            self.open_hints();
        }
    }

    fn open_hints(&mut self) {
        let hints = self
            .current_level()
            .filter(|l| !l.no_hints_dialog)
            .map(|l| self.hints_for(l));
        if let Some(hints) = hints {
            self.requests.push(Request::SetPaused(true));
            self.state = State::Playing(Some(OpenOverlay::Hints(hints)));
        }
    }

    fn hints_for(&self, level: &Level) -> Hints {
        let general = self.catalog.general_hints.clone();
        let last = level.objectives.0.split(|b| *b == b'\r').count() as i64 - 1;
        Hints {
            objectives: level.objectives.clone(),
            first_open_objective: i64::from(self.mission_advances).min(last).max(0) as i32,
            tab_names: self.catalog.hint_tabs.clone(),
            first_general: self.progress.first_hint(general.len() as i32),
            general,
            level: level.hints.clone(),
            show_at_start: self.progress.show_hints,
        }
    }

    fn map_over(&mut self, report: PostgameReport, veterans: Vec<MonsterInitializer>) {
        if self.game.film.take().is_some() {
            self.requests.push(Request::DropGame);
            self.state = State::FilmReport(report);
            return;
        }
        self.veterans = veterans;
        self.requests.push(Request::DropGame);
        let rule = rules::postgame(
            self.current_level(),
            report.end_code,
            self.last_saved.is_some(),
        );
        match rule {
            PostgameRule::ToMenu => self.state = State::MainMenu,
            PostgameRule::RestartFresh => {
                self.game.start = LevelStart::ChangeMap;
                self.veterans.clear();
                self.reset_rankings = true;
                self.enter_level();
            }
            PostgameRule::Dialog(choice) | PostgameRule::Undrawn(choice) => {
                self.open_postgame(report, choice, None)
            }
        }
    }

    fn open_postgame(
        &mut self,
        report: PostgameReport,
        choice: PostgameChoice,
        network: Option<net_flow::NetEpilogue>,
    ) {
        if let Some(mesh) = choice.mark_visited {
            let checksum = self
                .level_in_game(mesh)
                .or_else(|| self.current_level())
                .map_or(-1, |l| l.plugin_checksum);
            self.progress_changed |= self.progress.mark_visited(mesh, checksum);
        }
        let after = AfterMap {
            screen: Postgame {
                report,
                art: Some(choice.art),
                art_sequence: choice.art_sequence,
                ambient: Some(choice.ambient),
                exits: choice.exits,
                save_film: None,
            },
            next_mesh: choice.next_mesh,
            skip_mesh: choice.skip_mesh,
            network,
        };
        let next = if choice.campaign_end {
            State::CampaignEnd(self.catalog.campaign_end.clone(), after)
        } else {
            State::Postgame(after)
        };
        let index = if choice.won {
            rules::WIN_FILM
        } else {
            rules::LOSS_FILM
        };
        let film = self
            .current_level()
            .and_then(|l| rules::level_film(l, index))
            .map(str::to_owned);
        match film {
            Some(film) => self.play_cutscene(film, AfterFilm::Show(Box::new(next))),
            None => self.state = next,
        }
    }

    fn plugin_checksum_for_mesh(
        &self,
        mesh: FourCC,
        checksums: impl IntoIterator<Item = i32>,
        names: &[String],
    ) -> i32 {
        for checksum in checksums {
            if checksum != -1 && self.catalog.level_of_campaign(mesh, checksum).is_some() {
                return checksum;
            }
        }
        for name in names.iter().rev() {
            if let Some(c) = self.catalog.campaigns.iter().find(|c| {
                c.checksum != -1
                    && (c.plugins.iter().any(|p| p.eq_ignore_ascii_case(name))
                        || c.name.0.eq_ignore_ascii_case(name.as_bytes()))
            }) && self.catalog.level_of_campaign(mesh, c.checksum).is_some()
            {
                return c.checksum;
            }
        }
        -1
    }

    fn level_in_game(&self, mesh: FourCC) -> Option<&Level> {
        self.catalog
            .level_of_campaign(mesh, self.game.plugin_checksum)
            .or_else(|| self.catalog.level(mesh))
    }

    fn current_level(&self) -> Option<&Level> {
        self.level_in_game(self.game.mesh?)
    }

    fn save_progress_if_changed(&mut self) {
        if std::mem::take(&mut self.progress_changed) {
            self.requests
                .push(Request::SaveProgress(self.progress.clone()));
        }
    }
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::MainMenu => "main menu",
            State::NewGame(_) => "new game",
            State::Briefing(_) => "briefing",
            State::Loading => "loading",
            State::Playing(_) => "playing",
            State::CampaignEnd(..) => "campaign end",
            State::Postgame(_) => "postgame",
            State::Films(_) => "films",
            State::FilmReport(_) => "film report",
            State::SavedGames(_) => "saved games",
            State::Restoring(_) => "restoring",
            State::Cutscene(..) => "cutscene",
            State::Net(state) => net_flow::name(state),
            State::Quit => "quit",
        }
    }
}
