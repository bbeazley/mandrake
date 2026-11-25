use mandrake_common::FourCC;
use mandrake_tag::tfl::menus::{TFL_FIRST_LEVEL, TFL_TRAINING_LEVEL};
use tracing::info;

use super::{AfterFilm, AfterMap, OpenOverlay, Session, State};
use crate::action::{Action, GameEvent, GameStart, LevelStart, Request};
use crate::catalog::Catalog;
use crate::progress::Progress;
use crate::report::{EpiloguePage, PostgameReport};
use crate::rules::{self, PostgameChoice, PostgameRule};
use crate::screen::{
    Briefing, Cutscene, Difficulty, Films, LevelEntry, MenuTrack, NewGame, Paused, Postgame,
};
use crate::text::Text;
use crate::tfl::{
    AfterMovie, LevelCarry, NO_PLUGIN, TFL_MAX_VISITED, TflGame, TflMovie, TflPrologue,
    epilogue_level, solo_levels,
};

const TFL_MENU_MUSIC: [MenuTrack; 1] = [MenuTrack {
    tag: mandrake_tag::tfl::menus::TFL_MAIN_MENU_MUSIC,
    weight: 1,
}];

impl Session {
    pub fn tfl_set_menu_music(&mut self, on: bool) {
        self.catalog.menu_music = if on {
            TFL_MENU_MUSIC.to_vec()
        } else {
            Vec::new()
        };
        if matches!(self.state, State::MainMenu) {
            self.main_menu.music = self.catalog.menu_music.clone();
        }
    }

    pub fn new_tfl(
        mut game: TflGame,
        progress: Option<Progress>,
        data: crate::net::DataChecksums,
    ) -> Self {
        let progress = match progress {
            Some(progress) => {
                info!(
                    "TFL progress: {} levels played, from our file",
                    progress.visited.len()
                );
                game.prefs.visited = progress.visited.iter().map(|v| v.mesh).collect();
                game.prefs.difficulty = progress.difficulty;
                progress
            }
            None => game.imported_progress(),
        };
        let (net_mapset, net_max_teams) =
            crate::tfl::net_mapset_with_teams(&game.archive, b"Myth: The Fallen Lords");
        let catalog = Catalog {
            net_mapsets: vec![net_mapset],
            net_max_teams,
            difficulties: game
                .difficulties()
                .into_iter()
                .map(|name| Difficulty {
                    name: Text(name),
                    description: Text::default(),
                })
                .collect(),
            menu_music: TFL_MENU_MUSIC.to_vec(),
            ..Catalog::default()
        };
        let mut s = Self::new(catalog, progress);
        s.set_tfl(data);
        s.tfl = Some(Box::new(game));
        s.tfl_main_menu();
        s
    }

    pub fn tfl(&self) -> Option<&TflGame> {
        self.tfl.as_deref()
    }

    fn tfl_mut(&mut self) -> &mut TflGame {
        self.tfl.as_mut().expect("TFL's menus")
    }

    pub fn tfl_level(&self) -> Option<(FourCC, i16)> {
        self.tfl()?.level
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    pub fn tfl_carry(&self) -> Option<&dyn LevelCarry> {
        self.tfl()?.carry.as_deref()
    }

    pub fn tfl_carry_serial(&self) -> u32 {
        self.tfl().map_or(0, |t| t.carry_serial)
    }

    pub fn tfl_map_over(&mut self, carry: Box<dyn LevelCarry>) {
        let (veterans, permanent) = carry.counts();
        info!(
            "TFL game over: {veterans} veterans and {permanent} permanent rankings for the next level"
        );
        self.tfl_mut().set_carry(Some(carry));
    }

    pub fn tfl_restored(&mut self, carry: Box<dyn LevelCarry>) {
        self.tfl_mut().set_carry(Some(carry));
    }

    pub fn tfl_back_to_menu(&mut self) {
        self.game.film = None;
        self.game.restore = false;
        self.tfl_main_menu();
    }

    pub fn tfl_back_to_films(&mut self) {
        self.game.film = None;
        self.tfl_films();
    }

    pub fn tfl_film_over(&mut self, report: Option<PostgameReport>) {
        match report {
            Some(report) => {
                self.game.film = None;
                self.state = State::FilmReport(report);
            }
            None => self.tfl_back_to_films(),
        }
    }

    pub fn tfl_list_films(&mut self) {
        let folder = self.tfl_mut().film_folder();
        self.tfl_event(GameEvent::FilmsListed(folder));
    }

    pub fn tfl_walk_to_level(&mut self, level: &str, difficulty: i16) -> Result<(), String> {
        self.tfl_main_menu();
        if level.as_bytes() == TFL_TRAINING_LEVEL.0 {
            self.act(Action::Tutorial);
            return Ok(());
        }
        self.act(Action::NewGame { all_levels: true });
        let State::NewGame(new_game) = &self.state else {
            return Err("--play: New Game has no list of levels".to_owned());
        };
        let wanted = level.to_lowercase();
        let found = new_game
            .levels
            .iter()
            .find(|l| l.mesh.0 == level.as_bytes())
            .or_else(|| {
                new_game
                    .levels
                    .iter()
                    .find(|l| l.name.to_plain_utf8().to_lowercase().contains(&wanted))
            })
            .map(|l| l.mesh);
        let Some(level_id) = found else {
            self.tfl_main_menu();
            return Err(format!("--play: no solo level of TFL is {level:?}"));
        };
        self.act(Action::ChooseLevel(level_id));
        self.act(Action::ChooseDifficulty(difficulty));
        self.act(Action::StartLevel);
        Ok(())
    }

    pub fn tfl_level_over(
        &mut self,
        level: FourCC,
        difficulty: i16,
        end_reason: i16,
        epilogue: Option<EpiloguePage>,
    ) {
        let header = epilogue_level(&self.tfl_mut().archive, level);
        let can_revert = self.last_saved.is_some();
        let rule = rules::postgame_with(
            &rules::TFL_EPILOGUE,
            header.as_ref(),
            end_reason,
            can_revert,
        );
        info!("TFL game over: {level}, end code {end_reason}: {rule:?}");
        self.game.film = None;
        match rule {
            PostgameRule::ToMenu => self.tfl_main_menu(),
            PostgameRule::RestartFresh => self.tfl_start_level(level, difficulty),
            PostgameRule::Undrawn(choice) => {
                let default = if choice.won {
                    Action::NextLevel
                } else {
                    Action::LastSavedGame
                };
                self.tfl_after_epilogue(level, difficulty, choice, default);
            }
            PostgameRule::Dialog(choice) => {
                let page = Postgame {
                    report: PostgameReport {
                        end_code: end_reason,
                        epilogue,
                        ..PostgameReport::default()
                    },
                    art: Some(choice.art),
                    art_sequence: choice.art_sequence,
                    ambient: Some(choice.ambient),
                    exits: choice.exits.clone(),
                    save_film: None,
                };
                let tfl = self.tfl_mut();
                tfl.level = Some((level, difficulty));
                tfl.epilogue = Some(choice.clone());
                self.state = State::Postgame(AfterMap {
                    screen: page,
                    next_mesh: choice.next_mesh,
                    skip_mesh: choice.skip_mesh,
                    network: None,
                });
            }
        }
        self.save_progress_if_changed();
    }

    pub(super) fn tfl_networked(&self, action: Option<&Action>) -> bool {
        self.net.is_some()
            || matches!(self.state, State::Net(_))
            || (matches!(self.state, State::MainMenu)
                && matches!(
                    action,
                    Some(Action::Multiplayer | Action::JoinByAddress { .. })
                ))
    }

    pub(super) fn tfl_after_network(&mut self) {
        if self.net.is_none() && matches!(self.state, State::MainMenu) {
            self.game.film = None;
            self.game.restore = false;
            self.tfl_main_menu();
        }
    }

    pub(super) fn tfl_act(&mut self, action: Action) {
        info!("TFL menus: {action:?}");
        match (&mut self.state, action) {
            (State::MainMenu, Action::NewGame { all_levels }) => {
                self.set_last_saved(None);
                let last = self.progress.last_level;
                let tfl = self.tfl_mut();
                self.state = State::NewGame(tfl_new_game(tfl, all_levels, last));
            }
            (State::MainMenu, Action::Tutorial) => {
                self.set_last_saved(None);
                let tfl = self.tfl_mut();
                tfl.set_carry(None);
                let difficulty = tfl.prefs.difficulty;
                self.tfl_start_level(TFL_TRAINING_LEVEL, difficulty);
            }
            (State::MainMenu, Action::ReplayIntro) if self.main_menu.intro => {
                let movie = TflMovie {
                    id: mandrake_tag::tfl::menus::TFL_INTRO_MOVIE,
                    skippable: true,
                };
                self.tfl_movie(movie, AfterMovie::MainMenu);
            }
            (State::MainMenu, Action::OpenFilms) => self.tfl_films(),
            (State::MainMenu, Action::OpenSavedGames) if self.main_menu.saved_games => {
                self.state = State::SavedGames(crate::SavedGames::default());
                self.requests.push(Request::ListSaves);
            }
            (State::MainMenu, Action::Quit) => {
                self.state = State::Quit;
                self.requests.push(Request::Exit);
            }
            (
                State::NewGame(new_game) | State::Playing(Some(OpenOverlay::NewGame(new_game))),
                Action::ChooseLevel(mesh),
            ) => {
                if new_game.levels.iter().any(|l| l.mesh == mesh) {
                    new_game.level = Some(mesh);
                }
            }
            (
                State::NewGame(new_game) | State::Playing(Some(OpenOverlay::NewGame(new_game))),
                Action::ChooseDifficulty(difficulty),
            ) => {
                new_game.difficulty = difficulty;
            }
            (State::NewGame(new_game), Action::StartLevel) => {
                let (level, difficulty) = (new_game.level, new_game.difficulty);
                self.set_last_saved(None);
                let Some(level) = level else {
                    self.tfl_main_menu();
                    return;
                };
                self.tfl_start_chosen_level(level, difficulty);
            }
            (State::Playing(Some(OpenOverlay::NewGame(new_game))), Action::StartLevel) => {
                let (level, difficulty) = (new_game.level, new_game.difficulty);
                if let Some(level) = level {
                    self.requests.push(Request::DropGame);
                    self.set_last_saved(None);
                    self.tfl_start_chosen_level(level, difficulty);
                }
            }
            (State::NewGame(_), Action::Cancel) => {
                self.set_last_saved(None);
                self.tfl_main_menu();
            }
            (State::Playing(overlay @ Some(OpenOverlay::NewGame(_))), Action::Cancel) => {
                *overlay = None;
                self.set_last_saved(None);
            }
            (State::Playing(Some(OpenOverlay::NewGame(_))), action) => {
                tracing::debug!(?action, "TFL menus: New Game is open over the game");
            }
            (State::Briefing(_), Action::BriefingDone) => {
                if let Some((level, difficulty)) = self.tfl_mut().level {
                    self.tfl_enter_level(level, difficulty);
                }
            }
            (State::Films(films), action) if tfl_films_own(films, &action) => match action {
                Action::ViewFilm if films.can_view && !films.find_plugins => {
                    let row = films.selected;
                    if let Some(film) = row.and_then(|row| self.films.films.get(row)) {
                        let path = film.path.clone();
                        self.film_row = row;
                        self.tfl_load_film(path);
                    }
                }
                Action::ReplayLastFilm if films.last_film => {
                    if let Some(film) = &self.films.last {
                        let path = film.path.clone();
                        self.tfl_load_film(path);
                    }
                }
                Action::Cancel => self.tfl_main_menu(),
                action => tracing::debug!(?action, "TFL menus: not on Saved Films now"),
            },
            (State::Films(_), action) => {
                self.myth2_act(action);
                self.tfl_film_rules();
            }
            (State::Postgame(after), action)
                if after.screen.save_film.is_some()
                    || matches!(action, Action::SaveFilm | Action::SaveFilmAs(_)) =>
            {
                self.myth2_act(action);
            }
            (State::FilmReport(_), Action::CloseReport) => self.tfl_main_menu(),
            (
                State::Postgame(after),
                action @ (Action::NextLevel
                | Action::SkipLevel
                | Action::RestartLevel
                | Action::LastSavedGame
                | Action::QuitToMenu),
            ) => {
                let exits = &after.screen.exits;
                let shown = match action {
                    Action::NextLevel => exits.next,
                    Action::SkipLevel => exits.skip,
                    Action::RestartLevel => exits.restart,
                    Action::LastSavedGame => exits.last_saved,
                    _ => exits.quit,
                };
                if !shown {
                    tracing::debug!(?action, "TFL menus: not on the epilogue");
                    return;
                }
                let tfl = self.tfl_mut();
                if let (Some((level, difficulty)), Some(choice)) = (tfl.level, tfl.epilogue.take())
                {
                    self.tfl_after_epilogue(level, difficulty, choice, action);
                }
            }
            (State::Playing(None), Action::OpenPause) => {
                let film = self.game.film.is_some();
                let paused = Paused {
                    can_save: self.tfl_can_save(),
                    can_open: self.saves.any && !film,
                    can_revert: self.last_saved.is_some() && !film,
                    can_restart: !film,
                    replay: film,
                    network: None,
                };
                self.state = State::Playing(Some(OpenOverlay::Paused(paused)));
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::OpenSavedGames)
                if paused.can_open =>
            {
                self.state =
                    State::Playing(Some(OpenOverlay::SavedGames(crate::SavedGames::default())));
                self.requests.push(Request::ListSaves);
            }
            (State::SavedGames(_) | State::Playing(Some(OpenOverlay::SavedGames(_))), action) => {
                self.saved_games_act(action)
            }
            (State::Playing(overlay @ Some(OpenOverlay::Paused(_))), Action::Resume) => {
                *overlay = None;
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::RestartLevel)
                if paused.can_restart =>
            {
                if let Some((level, difficulty)) = self.tfl_mut().level {
                    self.requests.push(Request::DropGame);
                    self.tfl_load_level(level, difficulty);
                }
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::SaveGame)
                if paused.can_save =>
            {
                self.open_save_game();
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::LastSavedGame)
                if paused.can_revert && self.last_saved_gone =>
            {
                let last = self.progress.last_level;
                let tfl = self.tfl_mut();
                self.state =
                    State::Playing(Some(OpenOverlay::NewGame(tfl_new_game(tfl, false, last))));
            }
            (State::Playing(Some(OpenOverlay::Paused(paused))), Action::LastSavedGame)
                if paused.can_revert =>
            {
                if let Some(save) = self.last_saved.clone() {
                    self.requests.push(Request::DropGame);
                    self.tfl_continue_saved_game(&save);
                }
            }
            (State::Playing(None | Some(OpenOverlay::Paused(_))), Action::Quicksave)
                if self.game.film.is_none()
                    && self
                        .tfl
                        .as_ref()
                        .and_then(|t| t.level)
                        .is_some_and(|(level, _)| level != TFL_TRAINING_LEVEL) =>
            {
                self.requests.push(Request::Quicksave);
            }
            (State::Playing(Some(OpenOverlay::SaveGame(_))), action) => {
                self.save_game_overlay_act(action);
            }
            (State::Loading | State::Restoring(_) | State::Playing(_), Action::QuitToMenu) => {
                self.tfl_back_to_menu();
            }
            (state, action) => {
                tracing::debug!(?action, screen = state.name(), "TFL menus: ignored");
            }
        }
    }

    pub(super) fn tfl_event(&mut self, event: GameEvent) {
        match (&self.state, event) {
            (State::Cutscene(_, AfterFilm::Tfl(then)), GameEvent::CutsceneOver) => {
                match then.clone() {
                    AfterMovie::MainMenu => self.tfl_main_menu(),
                    AfterMovie::Level { level, difficulty } => {
                        self.tfl_load_level(level, difficulty)
                    }
                    AfterMovie::Epilogue {
                        level,
                        difficulty,
                        choice,
                        action,
                    } => self.tfl_leave_epilogue(level, difficulty, &choice, action),
                }
            }
            (State::Loading | State::Restoring(_), GameEvent::Loaded) => {
                self.requests.push(Request::BeginPlay);
                let autosave = std::mem::take(&mut self.autosave_next) && !self.game.restore;
                if std::mem::take(&mut self.game.restore) {
                    self.game.film = None;
                }
                self.state = State::Playing(None);
                if autosave && self.tfl_can_save() {
                    self.requests.push(Request::Autosave);
                }
            }
            (_, GameEvent::FilmsListed(folder)) => {
                self.films_listed(folder);
                self.tfl_film_rules();
            }
            (_, GameEvent::FilmSaved(saved)) => {
                self.film_saved(saved);
                self.tfl_film_rules();
            }
            (State::Films(_), GameEvent::FilmRenamed(file_name)) => {
                self.renamed(Some(file_name));
                self.tfl_film_rules();
            }
            (_, GameEvent::SavesListed(folder)) => self.saves_listed(folder),
            (_, GameEvent::GameSaved(saved)) => self.game_saved(saved),
            (State::Loading | State::Restoring(_), GameEvent::Restoring { ticks })
                if self.game.restore =>
            {
                self.restoring(ticks);
            }
            (State::Playing(Some(OpenOverlay::SaveGame(_))), GameEvent::Snapshot(snapshot)) => {
                self.snapshot_taken(snapshot);
            }
            (
                State::Loading | State::Restoring(_) | State::Playing(_),
                GameEvent::LoadFailed(error),
            ) => {
                tracing::error!(%error, "TFL menus: the level didn't load");
                self.autosave_next = false;
                self.tfl_back_to_menu();
            }
            (state, event) => {
                tracing::debug!(?event, screen = state.name(), "TFL menus: ignored");
            }
        }
    }

    fn tfl_main_menu(&mut self) {
        self.autosave_next = false;
        let tfl = self.tfl_mut();
        let (films, intro) = (tfl.has_films(), tfl.has_intro());
        self.main_menu = crate::MainMenu {
            new_game: true,
            tutorial: true,
            films,
            intro,
            saved_games: self.saves.any,
            multiplayer: self.networking.is_some(),
            preferences: false,
            music: self.catalog.menu_music.clone(),
        };
        self.state = State::MainMenu;
    }

    fn tfl_films(&mut self) {
        self.state = State::Films(Films::default());
        self.tfl_list_films();
    }

    fn tfl_film_rules(&mut self) {
        let ours = match (&self.state, &self.tfl) {
            (State::Films(films), Some(tfl)) => films
                .selected
                .and_then(|row| self.films.films.get(row))
                .is_some_and(|f| tfl.is_our_film(&f.path)),
            _ => return,
        };
        if let State::Films(films) = &mut self.state
            && !ours
        {
            films.delete = false;
            films.rename = false;
        }
    }

    fn tfl_mark_visited(&mut self, level: FourCC) {
        if self
            .progress
            .mark_visited_up_to(level, NO_PLUGIN, TFL_MAX_VISITED)
        {
            info!("TFL progress: {level} played");
            self.tfl_mut().prefs.visited.push(level);
            self.progress_changed = true;
        }
    }

    fn tfl_start_level(&mut self, level: FourCC, difficulty: i16) {
        self.tfl_mark_visited(level);
        let tfl = self.tfl_mut();
        tfl.level = Some((level, difficulty));
        match TflPrologue::of_level(&tfl.archive, level) {
            Some(prologue) => {
                self.state = State::Briefing(Briefing {
                    art: Some(prologue.pictures),
                    art_plugins: Vec::new(),
                    title: Text(prologue.title),
                    text: Text(prologue.text),
                    captions: prologue.caption.into_iter().map(Text).collect(),
                    narration: prologue.music,
                });
            }
            None => self.tfl_enter_level(level, difficulty),
        }
    }

    fn tfl_enter_level(&mut self, level: FourCC, difficulty: i16) {
        match self.tfl_mut().level_movie(level, rules::PREGAME_FILM) {
            Some(movie) => self.tfl_movie(movie, AfterMovie::Level { level, difficulty }),
            None => self.tfl_load_level(level, difficulty),
        }
    }

    fn tfl_movie(&mut self, movie: TflMovie, then: AfterMovie) {
        let cutscene = Cutscene {
            name: format!("moov {}", movie.id),
            movie: Some(movie),
        };
        self.state = State::Cutscene(cutscene, AfterFilm::Tfl(then));
    }

    fn tfl_load_level(&mut self, level: FourCC, difficulty: i16) {
        self.tfl_mut().level = Some((level, difficulty));
        self.game.mesh = Some(level);
        self.game.difficulty = difficulty;
        self.game.film = None;
        self.game.restore = false;
        self.requests.push(Request::LoadGame(GameStart {
            start: LevelStart::default(),
            mesh: level,
            difficulty,
            plugins: Vec::new(),
            veterans: Vec::new(),
            reset_rankings: false,
            film: None,
            restore: false,
            net: None,
        }));
        self.state = State::Loading;
    }

    fn tfl_start_chosen_level(&mut self, level: FourCC, difficulty: i16) {
        let tfl = self.tfl_mut();
        tfl.prefs.difficulty = difficulty;
        tfl.set_carry(None);
        if self.progress.difficulty != difficulty || self.progress.last_level != Some(level) {
            self.progress.difficulty = difficulty;
            self.progress.last_level = Some(level);
            self.progress_changed = true;
        }
        self.tfl_start_level(level, difficulty);
    }

    fn tfl_can_save(&self) -> bool {
        self.game.film.is_none()
            && self
                .tfl()
                .and_then(|t| t.level)
                .is_some_and(|(level, _)| level != TFL_TRAINING_LEVEL)
    }

    pub(super) fn tfl_continue_saved_game(&mut self, save: &crate::saves::SaveFile) {
        let (level, difficulty) = (save.game.mesh, save.game.difficulty);
        info!(
            "TFL: reverting to {} ({level}, tick {})",
            save.name.to_plain_utf8(),
            save.end_tick
        );
        self.tfl_mut().level = Some((level, difficulty));
        self.game.mesh = Some(level);
        self.game.difficulty = difficulty;
        self.game.film = Some(save.path.clone());
        self.game.restore = true;
        self.restoring = (save.name.clone(), save.end_tick);
        self.set_last_saved(Some(save.clone()));
        self.requests.push(Request::LoadGame(GameStart {
            start: LevelStart::default(),
            mesh: level,
            difficulty,
            plugins: Vec::new(),
            veterans: Vec::new(),
            reset_rankings: false,
            film: Some(save.path.clone()),
            restore: true,
            net: None,
        }));
        self.state = State::Loading;
    }

    fn tfl_load_film(&mut self, path: std::path::PathBuf) {
        self.game.film = Some(path.clone());
        self.requests.push(Request::LoadGame(GameStart {
            start: LevelStart::default(),
            mesh: FourCC([0; 4]),
            difficulty: self.game.difficulty,
            plugins: Vec::new(),
            veterans: Vec::new(),
            reset_rankings: false,
            film: Some(path),
            restore: false,
            net: None,
        }));
        self.state = State::Loading;
    }

    fn tfl_after_epilogue(
        &mut self,
        level: FourCC,
        difficulty: i16,
        choice: PostgameChoice,
        action: Action,
    ) {
        let index = if choice.won {
            rules::WIN_FILM
        } else {
            rules::LOSS_FILM
        };
        match self.tfl_mut().level_movie(level, index) {
            Some(movie) => self.tfl_movie(
                movie,
                AfterMovie::Epilogue {
                    level,
                    difficulty,
                    choice,
                    action,
                },
            ),
            None => self.tfl_leave_epilogue(level, difficulty, &choice, action),
        }
    }

    fn tfl_leave_epilogue(
        &mut self,
        level: FourCC,
        difficulty: i16,
        choice: &PostgameChoice,
        action: Action,
    ) {
        let to = match action {
            Action::NextLevel => {
                self.autosave_next = choice.next_mesh.is_some();
                choice.next_mesh
            }
            Action::SkipLevel => choice.skip_mesh,
            Action::LastSavedGame if self.last_saved_gone => {
                let last = self.progress.last_level;
                let tfl = self.tfl_mut();
                let mut new_game = tfl_new_game(tfl, false, last);
                new_game.no_menu = true;
                self.state = State::NewGame(new_game);
                return;
            }
            Action::LastSavedGame if self.last_saved.is_some() => {
                if let Some(save) = self.last_saved.clone() {
                    self.tfl_continue_saved_game(&save);
                }
                return;
            }
            Action::RestartLevel | Action::LastSavedGame => {
                self.tfl_load_level(level, difficulty);
                return;
            }
            _ => None,
        };
        match to {
            Some(next) => self.tfl_start_level(next, difficulty),
            None => self.tfl_main_menu(),
        }
    }
}

fn tfl_films_own(films: &Films, action: &Action) -> bool {
    films.save_film.is_none()
        && films.delete_film.is_none()
        && films.rename_film.is_none()
        && matches!(
            action,
            Action::ViewFilm | Action::ReplayLastFilm | Action::Cancel
        )
}

fn tfl_new_game(tfl: &TflGame, all_levels: bool, last: Option<FourCC>) -> NewGame {
    let levels: Vec<LevelEntry> = solo_levels(&tfl.archive, &tfl.prefs, all_levels)
        .into_iter()
        .map(|l| LevelEntry {
            mesh: l.id,
            name: Text(l.name),
            selectable: true,
            listed: true,
            order: 0,
        })
        .collect();
    let listed = |mesh: FourCC| levels.iter().any(|l| l.mesh == mesh).then_some(mesh);
    let level = last.and_then(listed).or_else(|| listed(TFL_FIRST_LEVEL));
    NewGame {
        levels,
        level,
        difficulties: tfl
            .difficulties()
            .into_iter()
            .map(|name| Difficulty {
                name: Text(name),
                description: Text::default(),
            })
            .collect(),
        difficulty: tfl.prefs.difficulty,
        can_start: true,
        ..NewGame::default()
    }
}
