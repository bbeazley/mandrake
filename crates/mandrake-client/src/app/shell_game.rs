use crate::app::ClientApp;
use crate::app::input::{DialogAsk, ModelessRoute, modeless_route};
use crate::app::myth2_scene::Myth2Scene;
use crate::app::recording::FilmFile;
use crate::app::save_game::SnapshotTaken;
use crate::app::shell::{Shell, ShellHost};
use crate::app::shell_menu::MENU_TITLE;
use crate::args::{self, Args};
use crate::clock::Instant;
use crate::film_report;
use crate::interface_config;
use crate::map_load::MeshQuery;
use crate::startup::{start_game, start_level};
use mandrake_session::{Action, GameEvent, GameStart, LevelStart, PostgameReport, Request};
use mandrake_sim::players::SoloStart;
use std::time::Duration;
use tracing::{error, info, warn};
use winit::dpi::PhysicalPosition;
use winit::event::{DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};

pub(crate) const FADE_OUT_SECONDS: f64 =
    mandrake_dialogs::fade::MYTH_II.load_fade_out_ms as f64 / 1000.0;
pub(crate) const FADE_IN_SECONDS: f64 =
    mandrake_dialogs::fade::MYTH_II.load_fade_in_ms as f64 / 1000.0;
const REBUILD_TICKS: u32 = 300;
const REBUILD_LIMIT: Duration = Duration::from_secs(30);
const RESTORE_FRAME_BUDGET: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapRun {
    Direct,
    Session,
    Rebuild,
}

#[derive(Debug, PartialEq)]
pub(crate) enum LoadWork<G = GameStart> {
    None,
    Start(G),
    ForMap,
    AttachGpu,
    Restore,
    Loaded,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) enum LoadFlow<G = GameStart> {
    #[default]
    Idle,
    FadingOut {
        game: G,
        since: f64,
    },
    Start(G),
    ForMap,
    AttachGpu,
    Restoring,
    FadingIn {
        since: Option<f64>,
    },
}

impl<G> LoadFlow<G> {
    pub(crate) fn begin(game: G, now: f64) -> Self {
        Self::FadingOut { game, since: now }
    }

    pub(crate) fn drop_game(&mut self, reloads: bool) -> bool {
        if !reloads {
            *self = Self::Idle;
        }
        !reloads
    }

    pub(crate) fn step(&mut self, now: f64) -> LoadWork<G> {
        match std::mem::take(self) {
            Self::Idle => LoadWork::None,
            Self::FadingOut { game, since } if now - since < FADE_OUT_SECONDS => {
                *self = Self::FadingOut { game, since };
                LoadWork::None
            }
            Self::FadingOut { game, .. } => {
                *self = Self::Start(game);
                LoadWork::None
            }
            Self::Start(game) => {
                *self = Self::ForMap;
                LoadWork::Start(game)
            }
            Self::ForMap => {
                *self = Self::AttachGpu;
                LoadWork::ForMap
            }
            Self::AttachGpu => {
                *self = Self::FadingIn { since: None };
                LoadWork::AttachGpu
            }
            Self::Restoring => {
                *self = Self::Restoring;
                LoadWork::Restore
            }
            Self::FadingIn { since } => match since {
                Some(from) if now - from >= FADE_IN_SECONDS => LoadWork::Loaded,
                since => {
                    *self = Self::FadingIn { since };
                    LoadWork::None
                }
            },
        }
    }

    pub(crate) fn restore(&mut self) {
        if matches!(self, Self::FadingIn { since: None }) {
            *self = Self::Restoring;
        }
    }

    pub(crate) fn restored(&mut self) {
        if matches!(self, Self::Restoring) {
            *self = Self::FadingIn { since: None };
        }
    }

    pub(crate) fn frame_drawn(&mut self, now: f64) {
        if let Self::FadingIn {
            since: since @ None,
        } = self
        {
            *since = Some(now);
        }
    }

    pub(crate) fn alpha(&self, now: f64) -> f32 {
        match self {
            Self::Idle | Self::Restoring => 0.0,
            Self::FadingOut { since, .. } => (((now - since) / FADE_OUT_SECONDS) as f32).min(1.0),
            Self::Start(_) | Self::ForMap | Self::AttachGpu | Self::FadingIn { since: None } => 1.0,
            Self::FadingIn { since: Some(from) } => {
                (1.0 - ((now - from) / FADE_IN_SECONDS) as f32).clamp(0.0, 1.0)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LoadTimes {
    begun: Option<Instant>,
    start_game: Duration,
    for_map: Duration,
    sound: Duration,
    attach_gpu: Duration,
    dropped: Duration,
}

impl LoadTimes {
    fn line(&self) -> String {
        format!(
            "drop {} ms, start_game {} ms, for_map {} ms (sound {} ms), attach_gpu {} ms",
            self.dropped.as_millis(),
            self.start_game.as_millis(),
            self.for_map.as_millis(),
            self.sound.as_millis(),
            self.attach_gpu.as_millis(),
        )
    }
}

fn level_args(base: &Args, game: &GameStart) -> Args {
    let mut args = base.clone();
    args.mesh = None;
    match &game.film {
        Some(film) => {
            args.play = None;
            args.replay = Some(film.clone());
            args.force_end = None;
            args.game_options = None;
        }
        None => {
            args.play = Some(game.mesh.as_str().to_owned());
            args.replay = None;
            args.film_report = None;
            args.difficulty = game.difficulty;
            args.plugin = game.plugins.clone();
            args.game_options = Some(mandrake_sim::players::solo_game_options(
                match game.start {
                    LevelStart::NewGame => SoloStart::NewGame,
                    LevelStart::ChangeMap => SoloStart::Map { restart: false },
                    LevelStart::Restart => SoloStart::Map { restart: true },
                },
                game.mesh.0,
            ));
        }
    }
    args
}

fn rebuild_args(base: &Args, mesh: &str) -> Args {
    let mut args = base.clone();
    args.play = Some(mesh.to_owned());
    args.mesh = None;
    args.replay = None;
    args.input_film = None;
    args.seed = None;
    args.film_report = None;
    args.frame_budget = None;
    args.force_end = None;
    args
}

impl Myth2Scene {
    fn load_clock(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub(super) fn session_frame(
        &mut self,
        host: &mut ShellHost,
        event_loop: &ActiveEventLoop,
        draw: bool,
    ) {
        let session_started = Instant::now();
        let window = host.map_size();
        let logical = self.app.as_ref().map(|app| app.dialog_ui().logical);
        let tick = self.app.as_ref().map(|app| app.sim.current_tick);
        let shift = host.modifiers.shift_key();
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let size = logical.filter(|_| menu.over_the_game()).unwrap_or(window);
        let requests = menu.step(size, shift, tick);
        self.perform(host, requests, event_loop);
        self.net_step(host, event_loop);
        if host.window().is_none() {
            return;
        }
        self.step_load(host, event_loop);
        if host.window().is_none() {
            return;
        }
        self.pass_on_dialog_ask(host, event_loop);
        if host.window().is_none() {
            return;
        }
        self.follow_input_owner(host);
        self.follow_dialog(host);
        let pre_draw = session_started.elapsed();
        if draw {
            let first_of_map = matches!(self.load, LoadFlow::FadingIn { since: None });
            let started = Instant::now();
            self.draw_session_frame(host);
            if first_of_map {
                info!(
                    "The level's first frame: {} ms",
                    started.elapsed().as_millis()
                );
            }
        }
        let post_started = Instant::now();
        let now = self.load_clock();
        self.load.frame_drawn(now);
        self.pass_on_snapshot(host, event_loop);
        if draw
            && let Some(app) = self.app.as_mut()
            && let Some(budget) = app.frame_budget.as_mut()
        {
            budget.note_session(pre_draw + post_started.elapsed());
        }
    }

    fn pass_on_snapshot(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        let Some(taken) = self.app.as_mut().and_then(ClientApp::take_snapshot) else {
            return;
        };
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let requests = match taken {
            SnapshotTaken::Prompt(snapshot) => menu.game_event(GameEvent::Snapshot(snapshot)),
            SnapshotTaken::Save { name, data, line } => {
                let (saved, requests) = menu.save_game(&name, Some(data));
                if saved && let Some(app) = self.app.as_mut() {
                    app.print_line(line);
                }
                requests
            }
        };
        self.perform(host, requests, event_loop);
    }

    fn draw_session_frame(&mut self, host: &ShellHost) {
        let alpha = self.load.alpha(self.load_clock());
        let map_shows = !matches!(
            self.load,
            LoadFlow::ForMap | LoadFlow::AttachGpu | LoadFlow::Restoring
        );
        let Self { app, menu, .. } = self;
        let (Some(menu), Some(gpu)) = (menu.as_mut(), host.gpu()) else {
            return;
        };
        match app.as_mut().filter(|app| app.has_gpu_state() && map_shows) {
            Some(app) => {
                app.shell_fade = alpha;
                let dialog = menu.overlay_open();
                app.redraw(Some(gpu.frame_surface()), dialog.then(|| menu.front_end()));
            }
            None => menu.draw(gpu, alpha),
        }
    }

    fn follow_input_owner(&mut self, host: &ShellHost) {
        let has_input = self.menu.as_ref().is_some_and(|menu| menu.has_input());
        if has_input == self.front_end_had_input {
            return;
        }
        self.front_end_had_input = has_input;
        if let Some(app) = self.app.as_mut() {
            if has_input {
                app.input_taken();
            } else {
                app.input_given(host.window_facts);
            }
        }
    }

    pub(super) fn modeless_event(&mut self, host: &ShellHost, event: WindowEvent, rect: [i32; 4]) {
        let Self {
            app,
            menu,
            modeless_press,
            ..
        } = self;
        let (window_facts, modifiers) = (&host.window_facts, &host.modifiers);
        let (Some(app), Some(menu)) = (app.as_mut(), menu.as_mut()) else {
            return;
        };
        let pointer = app
            .dialog_ui()
            .to_logical(window_facts.cursor.0, window_facts.cursor.1);
        let route = modeless_route(&event, pointer, rect, modeless_press);
        if route != ModelessRoute::Game {
            let mapped = app.event_for_dialog(&event);
            menu.window_event(mapped.as_ref().unwrap_or(&event), *modifiers);
        }
        if route != ModelessRoute::Dialog {
            app.window_event(event);
        }
    }

    fn pass_on_dialog_ask(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        let Some(ask) = app.take_dialog_ask() else {
            return;
        };
        let advances =
            (app.sim.engine.map_actions.as_ref()).map_or(0, |actions| actions.mission_advances);
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let requests = match ask {
            DialogAsk::Pause => menu.act(Action::OpenPause),
            DialogAsk::Quicksave => menu.act(Action::Quicksave),
            DialogAsk::Hints => {
                let mut requests = menu.game_event(GameEvent::MissionAdvanced(advances));
                requests.extend(menu.act(Action::OpenHints));
                requests
            }
        };
        self.perform(host, requests, event_loop);
    }

    fn follow_dialog(&mut self, host: &ShellHost) {
        let Self {
            app,
            menu,
            dialog_ui,
            ..
        } = self;
        let (window_facts, modifiers) = (&host.window_facts, &host.modifiers);
        let (Some(app), Some(menu)) = (app.as_mut(), menu.as_mut()) else {
            *dialog_ui = None;
            return;
        };
        app.set_hints_open(menu.hints_open());
        let ui = menu.overlay_open().then_some(app.dialog_ui().logical);
        if ui.is_some() && ui != *dialog_ui {
            let (x, y) = app
                .dialog_ui()
                .to_logical(window_facts.cursor.0, window_facts.cursor.1);
            menu.window_event(
                &WindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position: PhysicalPosition::new(f64::from(x), f64::from(y)),
                },
                *modifiers,
            );
        }
        *dialog_ui = ui;
    }

    pub(super) fn perform(
        &mut self,
        host: &mut ShellHost,
        requests: Vec<Request>,
        event_loop: &ActiveEventLoop,
    ) {
        let reloads = requests.iter().any(|r| matches!(r, Request::LoadGame(_)));
        for request in requests {
            match request {
                Request::LoadGame(game) => {
                    if game.reset_rankings {
                        host.args.carried_rankings.clear();
                        host.args.carried_initializers.clear();
                    }
                    self.load = LoadFlow::begin(game, self.load_clock());
                }
                Request::BeginPlay => {
                    if let Some(app) = self.app.as_mut() {
                        app.begin_play();
                    }
                }
                Request::SetPaused(paused) => {
                    if let Some(app) = self.app.as_mut() {
                        app.set_paused(paused);
                    }
                }
                Request::DropGame => {
                    if self.load.drop_game(reloads) {
                        self.pending_map = None;
                        self.drop_map(host);
                    }
                }
                Request::SaveProgress(progress) => {
                    if let Some(menu) = &self.menu {
                        menu.save_progress(&progress);
                    }
                }
                Request::PlayCutscene { name } => {
                    if let Some(menu) = self.menu.as_mut() {
                        let requests = menu.play_cutscene(&name);
                        self.perform(host, requests, event_loop);
                    }
                }
                Request::ListFilms => {
                    if let Some(menu) = self.menu.as_mut() {
                        menu.list_films();
                    }
                }
                Request::ListSaves => {
                    if let Some(menu) = self.menu.as_mut() {
                        menu.list_saves();
                    }
                }
                Request::SaveFilm { name } => {
                    if let Some(menu) = self.menu.as_mut() {
                        let requests = menu.save_film(&name);
                        self.perform(host, requests, event_loop);
                    }
                }
                Request::DeleteFilm { file_name } => {
                    if let Some(menu) = self.menu.as_mut() {
                        menu.delete_film(&file_name);
                    }
                }
                Request::DeleteSave { file_name } => {
                    if let Some(menu) = self.menu.as_mut() {
                        menu.delete_save(&file_name);
                    }
                }
                Request::RenameFilm { file_name, name } => {
                    if let Some(menu) = self.menu.as_mut() {
                        let requests = menu.rename_film(&file_name, &name);
                        self.perform(host, requests, event_loop);
                    }
                }
                Request::TakeSnapshot => {
                    if let Some(app) = self.app.as_mut() {
                        app.want_snapshot();
                    }
                }
                Request::SaveGame { name } => {
                    let data = self.app.as_ref().and_then(ClientApp::saved_game_now);
                    if let Some(menu) = self.menu.as_mut() {
                        let (_, requests) = menu.save_game(&name.0, data);
                        self.perform(host, requests, event_loop);
                    }
                }
                Request::Autosave => {
                    let now = crate::clock::SystemTime::now()
                        .duration_since(crate::clock::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64);
                    let zone = mandrake_session::saves::LocalZone::system();
                    let name = mandrake_session::saves::autosave_name(now, &zone);
                    if let Some(app) = self.app.as_mut() {
                        app.save_named(name, "Autosave created.");
                    }
                }
                Request::Quicksave => {
                    let name = self.menu.as_mut().map(|menu| menu.quicksave_name());
                    if let (Some(app), Some(name)) = (self.app.as_mut(), name) {
                        info!("Quicksave: {}", String::from_utf8_lossy(&name));
                        app.save_named(name, "Quicksave created.");
                    }
                }
                Request::Exit => host.quit(self, event_loop),
                request @ (Request::Net(_)
                | Request::SaveNetPlayer(_)
                | Request::DescribeNetGame(_)
                | Request::NetChatTo(..)) => {
                    self.net_request(host, request, event_loop);
                }
            }
        }
    }

    fn step_load(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        let now = self.load_clock();
        match self.load.step(now) {
            LoadWork::None => {}
            LoadWork::Start(game) => {
                self.load_times = LoadTimes {
                    begun: Some(Instant::now()),
                    dropped: self.drop_map(host),
                    ..LoadTimes::default()
                };
                if let Some(menu) = self.menu.as_mut() {
                    menu.stop_sound();
                }
                let mut args = level_args(&host.args, &game);
                if let Some(menu) = self.menu.as_mut().filter(|menu| menu.fail_next_load) {
                    menu.fail_next_load = false;
                    args.play = Some("!!!!".to_owned());
                }
                let interface = self.menu.as_ref().map(|menu| menu.config);
                let started = Instant::now();
                let interface =
                    interface.unwrap_or_else(interface_config::InterfaceConfig::defaults);
                let wanted = match &args.play {
                    Some(p) => MeshQuery::from_arg(p).id,
                    None => Some(game.mesh),
                };
                if let (Some(path), Some(_)) = (&args.film_report, &game.film)
                    && let Err(e) = film_report::start(path)
                {
                    error!("--film-report {}: {e}", path.display());
                }
                let level = match &game.net {
                    Some(net) => {
                        let role = if net.host {
                            mandrake_sim::net_game::NetRole::Host
                        } else {
                            mandrake_sim::net_game::NetRole::Joiner
                        };
                        let film = match self.pending_net.as_mut() {
                            Some(play) => {
                                if net.tfl.is_none()
                                    && let Some(theirs) = play.endpoint_game()
                                    && theirs.0 != net.new_game_data.0
                                {
                                    let at = theirs
                                        .0
                                        .iter()
                                        .zip(&net.new_game_data.0)
                                        .position(|(a, b)| a != b);
                                    warn!(
                                        "net: the session's new-game data isn't the endpoint's \
                                         (first difference at byte {at:?}): out of sync ahead"
                                    );
                                }
                                play.begin_map(Some(net.new_game_data.clone()));
                                play.film_header()
                            }
                            None => mandrake_net::client::film(
                                net.new_game_data.clone(),
                                &[],
                                0,
                                b"Network Game",
                            ),
                        };
                        crate::startup::start_network_level(&args, interface, film, role)
                    }
                    None => start_level(&args, interface, &game.veterans),
                };
                match level {
                    Ok(startup) if Some(startup.mesh_tag.mesh_id) == wanted => {
                        self.pending_map = Some(startup);
                        self.pending_args = Some(args);
                    }
                    Ok(startup) => self.load_failed(
                        host,
                        format!(
                            "mesh {:?} isn't installed: the load found {}",
                            args.play, startup.mesh_name
                        ),
                        event_loop,
                    ),
                    Err(e) => self.load_failed(host, format!("{e:#}"), event_loop),
                }
                self.load_times.start_game = started.elapsed();
            }
            LoadWork::ForMap => self.build_map(host, MapRun::Session),
            LoadWork::AttachGpu => {
                let started = Instant::now();
                self.attach_map(host);
                self.load_times.attach_gpu = started.elapsed();
                let whole = self
                    .load_times
                    .begun
                    .map_or(Duration::ZERO, |b| b.elapsed());
                let (width, height) = host.map_size();
                info!(
                    "The level is loaded at {width}x{height} in {} ms: {}",
                    whole.as_millis(),
                    self.load_times.line()
                );
                if self.app.as_ref().is_some_and(ClientApp::restoring) {
                    self.load.restore();
                }
            }
            LoadWork::Restore => self.restore_step(host, event_loop),
            LoadWork::Loaded => {
                if let Some(app) = self.app.as_mut() {
                    app.shell_fade = 0.0;
                }
                if let Some(menu) = self.menu.as_mut() {
                    let requests = menu.game_event(GameEvent::Loaded);
                    self.perform(host, requests, event_loop);
                }
            }
        }
    }

    fn restore_step(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        let Some(app) = self.app.as_mut() else {
            self.load.restored();
            return;
        };
        let started = Instant::now();
        let (ticks, over) = app.restore_step(RESTORE_FRAME_BUDGET);
        if over {
            app.finish_restore();
            info!(
                "The saved game is restored in {} ms (this frame's share)",
                started.elapsed().as_millis()
            );
            self.load.restored();
        }
        if let Some(menu) = self.menu.as_mut() {
            let requests = menu.game_event(GameEvent::Restoring { ticks });
            self.perform(host, requests, event_loop);
        }
    }

    fn build_map(&mut self, host: &ShellHost, run: MapRun) {
        let Some(startup) = self.pending_map.take() else {
            return;
        };
        let level_args = self.pending_args.take();
        let map_args = level_args.as_ref().unwrap_or(&host.args);
        let started = Instant::now();
        let mut app = ClientApp::for_map(host, map_args, startup, run);
        app.session_map = run == MapRun::Session;
        if app.sim.is_network_game()
            && let Some(net) = self.pending_net.take()
        {
            app.net_strings = crate::net_game::networking_strings(&host.args.archive);
            app.net = Some(net);
        }
        app.set_console_cache(self.console_cache.clone());
        if let (MapRun::Session, Some(args), Some(menu)) = (run, &level_args, &self.menu) {
            let file = FilmFile::new(menu.game_dir(), menu.last_recording_name());
            app.start_recording(args, menu.game_dir(), Some(file));
        }
        self.load_times.sound = app.sound_load;
        self.app = Some(app);
        self.load_times.for_map = started.elapsed();
    }

    fn load_failed(&mut self, host: &mut ShellHost, error: String, event_loop: &ActiveEventLoop) {
        error!("the level didn't load: {error}");
        self.load = LoadFlow::Idle;
        self.pending_map = None;
        self.pending_args = None;
        self.drop_map(host);
        if let Some(menu) = self.menu.as_mut() {
            let requests = menu.game_event(GameEvent::LoadFailed(error));
            self.perform(host, requests, event_loop);
        }
    }

    pub(super) fn map_over(
        &mut self,
        host: &mut ShellHost,
        report: PostgameReport,
        event_loop: &ActiveEventLoop,
    ) {
        let (veterans, rankings) = self.app.as_ref().map_or_else(
            || (Vec::new(), Vec::new()),
            |app| (app.veterans_at_end(), app.sim.permanent_rankings()),
        );
        host.args.carried_rankings = rankings;
        host.args.carried_initializers = veterans.clone();
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let requests = menu.game_event(GameEvent::MapOver { report, veterans });
        self.drop_map(host);
        self.perform(host, requests, event_loop);
    }

    pub(super) fn drop_map(&mut self, host: &mut ShellHost) -> Duration {
        let Some(mut app) = self.app.take() else {
            return Duration::ZERO;
        };
        let started = Instant::now();
        if let Some(gpu) = host.gpu() {
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
        host.ui_mode = app.ui_mode;
        self.console_cache = app.console_cache();
        app.finish_recording();
        if let Some(mut net) = app.net.take() {
            if app.sim.is_network_game() {
                let sim = std::mem::replace(&mut app.sim, mandrake_sim::Simulation::new(0));
                net.postgame = Some(Box::new(sim));
            }
            self.pending_net = Some(net);
        }
        if let Some(audio) = &host.audio {
            audio.link().audio.stop_all();
        }
        if let Some(window) = host.window() {
            window.set_min_inner_size(None::<winit::dpi::PhysicalSize<u32>>);
            crate::app::shell::set_window_title(window, MENU_TITLE);
        }
        drop(app);
        if let Some(gpu) = host.gpu() {
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        }
        release_freed_heap();
        started.elapsed()
    }

    pub(super) fn rebuild_step(&mut self, host: &mut ShellHost, event_loop: &ActiveEventLoop) {
        if host.gpu().is_none() {
            return;
        }
        let Some(rebuilds) = self.rebuilds.as_ref() else {
            return;
        };
        let Some(mesh) = rebuilds.mesh() else {
            self.rebuilds = None;
            if let Err(e) = self.start_own_run(host) {
                host.failed = Some(e);
                host.quit(self, event_loop);
            }
            return;
        };
        let Some(app) = self.app.as_ref() else {
            if let Err(e) = self.start_rebuild(host, &mesh) {
                host.failed = Some(e);
                host.quit(self, event_loop);
            }
            return;
        };
        let played = rebuilds
            .since
            .is_some_and(|since| since.elapsed() >= REBUILD_LIMIT);
        if app.sim.current_tick < REBUILD_TICKS && !played {
            return;
        }
        self.finish_rebuild(host, &mesh);
    }

    fn start_rebuild(&mut self, host: &mut ShellHost, mesh: &str) -> anyhow::Result<()> {
        let args = rebuild_args(&host.args, mesh);
        let started = Instant::now();
        let startup = start_game(&args)?;
        self.load_times = LoadTimes {
            start_game: started.elapsed(),
            ..LoadTimes::default()
        };
        self.pending_map = Some(startup);
        self.build_map(host, MapRun::Rebuild);
        let started = Instant::now();
        self.attach_map(host);
        self.load_times.attach_gpu = started.elapsed();
        if let Some(rebuilds) = self.rebuilds.as_mut() {
            rebuilds.since = Some(Instant::now());
        }
        Ok(())
    }

    fn finish_rebuild(&mut self, host: &mut ShellHost, mesh: &str) {
        let ticks = self.app.as_ref().map_or(0, |app| app.sim.current_tick);
        let sounding = |host: &ShellHost| {
            host.audio
                .as_ref()
                .map_or(0, |audio| audio.link().audio.sounding_channel_count())
        };
        let before = sounding(host);
        let playing = Self::gpu_counts(host);
        self.load_times.dropped = self.drop_map(host);
        let after = sounding(host);
        let counts = Self::gpu_counts(host);
        let Some(rebuilds) = self.rebuilds.as_mut() else {
            return;
        };
        rebuilds.done += 1;
        eprintln!(
            "[rebuild] cycle {}/{} {mesh}: ticks {ticks}; {}; channels {before} then {after}; \
             rss {:.1} MB; {}; playing {playing}; dropped {counts}",
            rebuilds.done,
            rebuilds.count,
            self.load_times.line(),
            rss_mb(),
            memory_split(),
        );
    }

    fn gpu_counts(host: &ShellHost) -> String {
        let Some(gpu) = host.gpu() else {
            return String::new();
        };
        #[cfg(target_arch = "wasm32")]
        let objects = "no report on the web".to_owned();
        #[cfg(not(target_arch = "wasm32"))]
        let objects = gpu.instance.generate_report().map_or_else(
            || "no report".to_owned(),
            |r| {
                let h = r.hub;
                format!(
                    "buffers {} textures {} views {} samplers {} bind_groups {} bgls {} pipelines \
                     {} layouts {} shaders {} query_sets {} encoders {} cmd_bufs {}",
                    h.buffers.num_allocated,
                    h.textures.num_allocated,
                    h.texture_views.num_allocated,
                    h.samplers.num_allocated,
                    h.bind_groups.num_allocated,
                    h.bind_group_layouts.num_allocated,
                    h.render_pipelines.num_allocated,
                    h.pipeline_layouts.num_allocated,
                    h.shader_modules.num_allocated,
                    h.query_sets.num_allocated,
                    h.command_encoders.num_allocated,
                    h.command_buffers.num_allocated,
                )
            },
        );
        let memory = gpu.device.generate_allocator_report().map_or_else(
            || "no allocator report".to_owned(),
            |r| {
                format!(
                    "gpu_allocated {:.1} MB gpu_reserved {:.1} MB blocks {} allocations {}",
                    r.total_allocated_bytes as f64 / 1e6,
                    r.total_reserved_bytes as f64 / 1e6,
                    r.blocks.len(),
                    r.allocations.len(),
                )
            },
        );
        format!("{objects} {memory}")
    }

    fn start_own_run(&mut self, host: &mut ShellHost) -> anyhow::Result<()> {
        if args::wants_menu(&host.args) {
            self.start_menu(host)?;
            return Ok(());
        }
        if let Some(path) = &host.args.film_report {
            film_report::start(path)?;
        }
        let startup = start_game(&host.args)?;
        self.start_map(host, startup, MapRun::Direct);
        self.attach_map(host);
        Ok(())
    }
}

pub(crate) struct Rebuilds {
    meshes: Vec<String>,
    count: u32,
    done: u32,
    since: Option<Instant>,
}

impl Rebuilds {
    fn mesh(&self) -> Option<String> {
        (self.done < self.count)
            .then(|| self.meshes[self.done as usize % self.meshes.len()].clone())
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn release_freed_heap() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> std::ffi::c_int;
    }
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn release_freed_heap() {}

fn rss_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1e3
}

fn memory_split() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 1e3
    };
    format!(
        "anon {:.1} file {:.1}{}",
        field("RssAnon:"),
        field("RssFile:"),
        malloc_in_use()
    )
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn malloc_in_use() -> String {
    #[repr(C)]
    struct MallInfo2 {
        arena: usize,
        ordblks: usize,
        smblks: usize,
        hblks: usize,
        hblkhd: usize,
        usmblks: usize,
        fsmblks: usize,
        uordblks: usize,
        fordblks: usize,
        keepcost: usize,
    }
    unsafe extern "C" {
        fn dlsym(
            handle: *mut std::ffi::c_void,
            symbol: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_void;
    }
    let sym = unsafe { dlsym(std::ptr::null_mut(), c"mallinfo2".as_ptr()) };
    if sym.is_null() {
        return String::new();
    }
    let mallinfo2: extern "C" fn() -> MallInfo2 = unsafe { std::mem::transmute(sym) };
    let info = mallinfo2();
    format!(
        " malloc_used {:.1} malloc_free {:.1}",
        (info.uordblks + info.hblkhd) as f64 / 1e6,
        info.fordblks as f64 / 1e6
    )
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn malloc_in_use() -> String {
    String::new()
}

pub(crate) fn run_rebuilds(args: Args) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let rebuilds = Rebuilds {
        meshes: args.rebuild_first.clone(),
        count: args.rebuild_count,
        done: 0,
        since: None,
    };
    let mut game = Myth2Scene::new();
    game.rebuilds = Some(rebuilds);
    let shell = Shell::new(ShellHost::new(args), Box::new(game));
    let shell = crate::app::shell::run_event_loop(event_loop, shell)?;
    shell
        .and_then(|mut shell| shell.take_failure())
        .map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn myth_iis_fades_are_its_game_fades() {
        use mandrake_dialogs::fade::{LevelEndFade, MYTH_II};
        assert_eq!(
            MYTH_II.level_end,
            LevelEndFade::ToBlack {
                live_ticks: crate::carnage_report::LIVE_FADE_TICKS,
                film_ms: (crate::carnage_report::FILM_FADE_SECONDS * 1000.0) as u32,
            }
        );
        assert_eq!((FADE_OUT_SECONDS, FADE_IN_SECONDS), (2.0, 0.5));
        assert_eq!(MYTH_II.epilogue, None);
    }

    #[test]
    fn a_rebuild_has_none_of_the_runs_tools() {
        let base = Args::try_parse_from([
            "mandrake",
            "--replay",
            "a.m2rec",
            "--film-report",
            "a",
            "--plugin",
            "Patch 1.8.5 Maps",
        ])
        .unwrap();
        let args = rebuild_args(&base, "le3e");
        assert_eq!(args.play.as_deref(), Some("le3e"));
        assert_eq!(args.replay, None);
        assert_eq!(args.film_report, None);
        assert_eq!(args.plugin, ["Patch 1.8.5 Maps"], "reaches a plugin's maps");
    }

    #[test]
    fn rebuilds_alternate_their_meshes_and_stop_at_the_count() {
        let mut r = Rebuilds {
            meshes: vec!["le3e".to_owned(), "cr01".to_owned()],
            count: 3,
            done: 0,
            since: None,
        };
        let mut seen = Vec::new();
        while let Some(mesh) = r.mesh() {
            seen.push(mesh);
            r.done += 1;
        }
        assert_eq!(seen, ["le3e", "cr01", "le3e"]);
    }
}
