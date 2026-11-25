use crate::app::cutscene::{self, Cutscene};
use crate::app::input::WheelNotches;
use crate::app::menu_sound::{MenuSound, NO_MENU_MUSIC};
use crate::app::myth2_scene::Myth2Scene;
use crate::app::shell::{Shell, ShellGpu, ShellHost, acquire_surface, clear_target};
use crate::app::sound::sound_preference_flags;
use crate::args::{Args, BudgetOverlay, ScriptStep};
use crate::audio_out::AudioLink;
use crate::clock::Instant;
use crate::interface_config::{InterfaceConfig, MenuArt};
use crate::map_load::MeshQuery;
use crate::settings::{self, SavedProgress};
use crate::window_surface::{self, SurfaceTimeoutLog};
use mandrake_common::FourCC;
use mandrake_dialogs::EngineDialogs;
use mandrake_render::{DrawInterface, InterfaceFrame, InterfacePresenter};
use mandrake_session::{
    Action, Catalog, FrameInput, FrontEnd, GameEvent, Overlay, Progress, Request, Screen, Session,
    UiEvent, UiKey,
};
use mandrake_tag::TagLookup;
use mandrake_tag::mesh_links::CampaignTags;
use mandrake_tag::tag_files::{INTERFACE_800X600, dialog_tag_files, engine_tag_files};
use mandrake_view::interface_picture::InterfaceFilter;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

pub(super) const MENU_TITLE: &str = "Myth II: Soulblighter";

pub(crate) fn run_menu(args: Args) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut host = ShellHost::new(args);
    let mut game = Myth2Scene::new();
    game.start_menu(&mut host)?;
    crate::app::shell::run_event_loop(event_loop, Shell::new(host, Box::new(game)))?;
    Ok(())
}

pub(crate) trait WindowFrontEnd: FrontEnd + DrawInterface {}
impl<T: FrontEnd + DrawInterface> WindowFrontEnd for T {}

fn engine_front_end(
    game_dir: &Path,
    config: InterfaceConfig,
    ui_filter: Option<InterfaceFilter>,
    seed: u32,
) -> anyhow::Result<Box<dyn WindowFrontEnd>> {
    let interface = match config.menu_art {
        MenuArt::Interface800x600 => Some(INTERFACE_800X600),
        MenuArt::Stock => None,
    };
    let mut files = dialog_tag_files(game_dir, interface);
    if !files.missing_plugins.is_empty() {
        warn!(
            "menus: {:?} isn't installed in {}: using the stock art",
            files.missing_plugins,
            game_dir.display()
        );
        files = dialog_tag_files(game_dir, None);
    }
    anyhow::ensure!(
        files.complete(),
        "menus: the tags of {} are incomplete: {files:?}",
        game_dir.display()
    );
    let filter = ui_filter.unwrap_or_else(|| config.menu_filter.into());
    let game_dir = game_dir.to_path_buf();
    let art_cache: std::cell::RefCell<Option<(Vec<String>, TagLookup)>> =
        std::cell::RefCell::new(None);
    Ok(Box::new(DialogsWindow::new(
        EngineDialogs::new(TagLookup::open(&files))
            .with_menu_filter(filter)
            .with_random_seed(seed)
            .with_campaign_art(Box::new(move |plugins, id| {
                campaign_art(&game_dir, &art_cache, plugins, id)
            })),
    )))
}

struct DialogsWindow {
    dialogs: EngineDialogs,
    gpu: Option<InterfacePresenter>,
}

impl DialogsWindow {
    fn new(dialogs: EngineDialogs) -> Self {
        Self { dialogs, gpu: None }
    }
}

impl FrontEnd for DialogsWindow {
    fn frame(
        &mut self,
        screen: &Screen<'_>,
        input: &FrameInput<'_>,
    ) -> mandrake_session::FrameOutput {
        self.dialogs.frame(screen, input)
    }

    fn modeless_rect(&self) -> Option<[i32; 4]> {
        self.dialogs.modeless_rect()
    }
}

impl DrawInterface for DialogsWindow {
    fn draw(&mut self, frame: &mut InterfaceFrame<'_>) {
        let gpu = self
            .gpu
            .get_or_insert_with(|| InterfacePresenter::new(frame));
        let filter = self.dialogs.menu_filter();
        gpu.draw(frame, self.dialogs.picture(frame.size), filter);
    }
}

fn campaign_art(
    game_dir: &Path,
    cache: &std::cell::RefCell<Option<(Vec<String>, TagLookup)>>,
    plugins: &[String],
    id: FourCC,
) -> Option<Vec<u8>> {
    let mut cached = cache.borrow_mut();
    let matches = cached
        .as_ref()
        .is_some_and(|(keys, _)| keys.as_slice() == plugins);
    if !matches {
        let names: Vec<&str> = plugins.iter().map(String::as_str).collect();
        *cached = Some((
            plugins.to_vec(),
            TagLookup::open(&engine_tag_files(game_dir, &names)),
        ));
    }
    cached.as_ref()?.1.get(FourCC(*b".256"), id)
}

fn answer_to_unbuilt_screen(screen: &Screen<'_>) -> Option<Action> {
    match screen {
        Screen::CampaignEnd(_) => Some(Action::NarrationDone),
        _ => None,
    }
}

fn read_progress(path: Option<&Path>, game_dir: &Path) -> (Progress, bool) {
    let saved = match path {
        Some(path) => settings::load_progress(path, game_dir),
        None => Ok(SavedProgress::None),
    };
    let read = match saved {
        Ok(SavedProgress::Own(text)) => Progress::from_text(&text),
        Ok(SavedProgress::GamePreferences(bytes)) => Progress::import_game2(&bytes),
        Ok(SavedProgress::None) => Ok(Progress::default()),
        Err(e) => Err(e),
    };
    match read {
        Ok(progress) => (progress, path.is_some()),
        Err(e) => {
            warn!("progress: {e}; starting fresh, and it won't be saved over");
            (Progress::default(), false)
        }
    }
}

fn walk_to_level(session: &mut Session, mesh: FourCC, difficulty: i16) -> anyhow::Result<()> {
    if mesh == mandrake_session::rules::TUTORIAL {
        session.act(Action::Tutorial);
        return Ok(());
    }
    session.act(Action::NewGame { all_levels: true });
    let campaigns = match session.screen() {
        Screen::NewGame(new_game) => new_game.campaigns.len(),
        _ => 0,
    };
    for index in 0..campaigns {
        session.act(Action::ChooseCampaign {
            index,
            all_levels: true,
        });
        let listed = matches!(session.screen(), Screen::NewGame(new_game)
            if new_game.levels.iter().any(|l| l.mesh == mesh && l.selectable));
        if listed {
            session.act(Action::ChooseLevel(mesh));
            session.act(Action::ChooseDifficulty(difficulty));
            session.act(Action::StartLevel);
            return Ok(());
        }
    }
    anyhow::bail!("--session: no campaign lists mesh {}", mesh.as_str())
}

fn walk_to_film(menu: &mut Menu, film: &Path) -> anyhow::Result<()> {
    let wanted = film
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("--replay {film:?} has no file name"))?
        .to_string_lossy()
        .as_bytes()
        .to_vec();
    menu.session.act(Action::OpenFilms);
    if menu.session.take_requests().contains(&Request::ListFilms) {
        menu.list_films();
    }
    let row = match menu.session.screen() {
        Screen::Films(_) => menu
            .tags
            .as_ref()
            .map(|tags| mandrake_session::films::list_folder(&menu.game_dir, tags))
            .and_then(|folder| folder.films.iter().position(|f| f.file_name == wanted)),
        _ => None,
    };
    let Some(row) = row else {
        anyhow::bail!(
            "--session: no film {film:?} in {}",
            menu.game_dir
                .join(mandrake_session::films::FOLDER)
                .display()
        );
    };
    menu.session.act(Action::ChooseFilm(row));
    menu.session.act(Action::ViewFilm);
    if !matches!(menu.session.screen(), Screen::Loading) {
        anyhow::bail!("--session: film {film:?} can't be played");
    }
    Ok(())
}

fn walk_to_save(menu: &mut Menu, save: &Path) -> anyhow::Result<()> {
    let wanted = save
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("--replay {save:?} has no file name"))?
        .to_string_lossy()
        .as_bytes()
        .to_vec();
    menu.session.act(Action::OpenSavedGames);
    if menu.session.take_requests().contains(&Request::ListSaves) {
        menu.list_saves();
    }
    let row = match menu.session.screen() {
        Screen::SavedGames(saves) => saves.names.iter().position(|n| n.0 == wanted),
        _ => None,
    };
    let Some(row) = row else {
        anyhow::bail!(
            "--session: no save {save:?} in {}",
            menu.game_dir
                .join(mandrake_session::saves::FOLDER)
                .display()
        );
    };
    menu.session.act(Action::ChooseSave(row));
    menu.session.act(Action::OpenSave);
    if !matches!(menu.session.screen(), Screen::Loading) {
        anyhow::bail!("--session: save {save:?} can't be loaded");
    }
    Ok(())
}

fn is_saved_game(path: &Path) -> bool {
    let mut head = [0u8; 40];
    crate::game_files::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok_and(|()| mandrake_session::saves::is_save(&head))
}

pub(crate) fn ui_event(event: &WindowEvent) -> Option<UiEvent> {
    let mut wheel = WheelNotches::default();
    let mut out = Vec::with_capacity(1);
    push_ui_event(event, &mut wheel, &mut out);
    out.into_iter().next()
}

fn push_ui_event(event: &WindowEvent, wheel: &mut WheelNotches, out: &mut Vec<UiEvent>) {
    match event {
        WindowEvent::CursorMoved { position, .. } => out.push(UiEvent::PointerMoved {
            x: position.x.floor() as i32,
            y: position.y.floor() as i32,
        }),
        WindowEvent::MouseInput { state, button, .. } => {
            let left = match button {
                MouseButton::Left => true,
                MouseButton::Right => false,
                _ => return,
            };
            out.push(UiEvent::Button {
                left,
                pressed: *state == ElementState::Pressed,
            });
        }
        WindowEvent::MouseWheel { delta, .. } => {
            let notches = wheel.push(*delta);
            let dir = if notches > 0 { 1 } else { -1 };
            for _ in 0..notches.unsigned_abs() {
                out.push(UiEvent::Wheel(dir));
            }
        }
        _ => {}
    }
}

fn ui_key(code: KeyCode) -> Option<UiKey> {
    use KeyCode as K;
    Some(match code {
        K::Enter | K::NumpadEnter => UiKey::Return,
        K::Escape => UiKey::Escape,
        K::Space => UiKey::Space,
        K::Tab => UiKey::Tab,
        K::Backspace => UiKey::Backspace,
        K::Delete => UiKey::Delete,
        K::ArrowLeft => UiKey::Left,
        K::ArrowRight => UiKey::Right,
        K::ArrowUp => UiKey::Up,
        K::ArrowDown => UiKey::Down,
        K::Home => UiKey::Home,
        K::End => UiKey::End,
        K::PageUp => UiKey::PageUp,
        K::PageDown => UiKey::PageDown,
        K::KeyA => UiKey::Char('a'),
        K::KeyB => UiKey::Char('b'),
        K::KeyC => UiKey::Char('c'),
        K::KeyD => UiKey::Char('d'),
        K::KeyE => UiKey::Char('e'),
        K::KeyF => UiKey::Char('f'),
        K::KeyG => UiKey::Char('g'),
        K::KeyH => UiKey::Char('h'),
        K::KeyI => UiKey::Char('i'),
        K::KeyJ => UiKey::Char('j'),
        K::KeyK => UiKey::Char('k'),
        K::KeyL => UiKey::Char('l'),
        K::KeyM => UiKey::Char('m'),
        K::KeyN => UiKey::Char('n'),
        K::KeyO => UiKey::Char('o'),
        K::KeyP => UiKey::Char('p'),
        K::KeyQ => UiKey::Char('q'),
        K::KeyR => UiKey::Char('r'),
        K::KeyS => UiKey::Char('s'),
        K::KeyT => UiKey::Char('t'),
        K::KeyU => UiKey::Char('u'),
        K::KeyV => UiKey::Char('v'),
        K::KeyW => UiKey::Char('w'),
        K::KeyX => UiKey::Char('x'),
        K::KeyY => UiKey::Char('y'),
        K::KeyZ => UiKey::Char('z'),
        K::Digit0 => UiKey::Char('0'),
        K::Digit1 => UiKey::Char('1'),
        K::Digit2 => UiKey::Char('2'),
        K::Digit3 => UiKey::Char('3'),
        K::Digit4 => UiKey::Char('4'),
        K::Digit5 => UiKey::Char('5'),
        K::Digit6 => UiKey::Char('6'),
        K::Digit7 => UiKey::Char('7'),
        K::Digit8 => UiKey::Char('8'),
        K::Digit9 => UiKey::Char('9'),
        _ => return None,
    })
}

pub(crate) fn key_events(
    code: PhysicalKey,
    pressed: bool,
    repeat: bool,
    text: Option<&str>,
    held: ModifiersState,
    out: &mut Vec<UiEvent>,
) {
    if let PhysicalKey::Code(code) = code
        && let Some(key) = ui_key(code)
    {
        out.push(UiEvent::Key {
            key,
            pressed,
            repeat,
            shift: held.shift_key(),
            alt: held.alt_key(),
        });
    }
    if pressed && !held.alt_key() && !held.control_key() {
        out.extend(
            text.unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .map(UiEvent::Text),
        );
    }
}

pub(crate) struct Menu {
    session: Session,
    front_end: Box<dyn WindowFrontEnd>,
    events: Vec<UiEvent>,
    event_buf: Vec<UiEvent>,
    wheel_notches: WheelNotches,
    clock: Instant,
    timeouts: SurfaceTimeoutLog,
    sound: MenuSound,
    fade: Option<mandrake_render::ScreenFade>,
    pub(super) config: InterfaceConfig,
    game_dir: PathBuf,
    progress_path: Option<PathBuf>,
    script: VecDeque<ScriptStep>,
    pub(super) fail_next_load: bool,
    budget_frames: u32,
    shift_held: bool,
    front_end_dirty: bool,
    tags: Option<CampaignTags>,
    on_menu: bool,
    cutscene: Option<Cutscene>,
    audio: Option<AudioLink>,
    menu_filter: InterfaceFilter,
}

impl Menu {
    pub(super) fn new(args: &Args, audio: Option<AudioLink>) -> anyhow::Result<Self> {
        if let Some(error) = crate::args::session_error(args) {
            anyhow::bail!(error);
        }
        let config = crate::interface_config::load(args.conf.as_deref(), args.headless);
        let game_dir = args
            .archive
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| anyhow::anyhow!("--archive {:?} has no game folder", args.archive))?
            .to_path_buf();
        let seed = crate::clock::SystemTime::now()
            .duration_since(crate::clock::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let ui_filter = args.ui_filter.map(Into::into);
        let front_end = engine_front_end(&game_dir, config, ui_filter, seed)?;
        let start = Instant::now();
        let tags = CampaignTags::open(&game_dir);
        let mut catalog = Catalog::from_tags(&tags);
        if sound_preference_flags(&args.archive) & NO_MENU_MUSIC != 0 {
            catalog.menu_music.clear();
        }
        let path = settings::user_progress_path();
        let (progress, can_save) = read_progress(path.as_deref(), &game_dir);
        info!(
            "menus: {} campaigns and the player's progress in {} ms",
            catalog.campaigns.len(),
            start.elapsed().as_millis()
        );
        let mut menu = Self::with(front_end, catalog, progress);
        menu.sound = MenuSound::new(audio.clone(), game_dir.clone(), seed);
        menu.audio = audio;
        menu.menu_filter = ui_filter.unwrap_or_else(|| config.menu_filter.into());
        menu.config = config
            .with_cutscene_filter(args.cutscene_filter)
            .with_cutscene_hd(args.cutscene_hd.map(|hd| hd == crate::args::CutsceneHd::On));
        menu.game_dir = game_dir;
        menu.progress_path = path.filter(|_| can_save);
        menu.script = args.session_script.iter().copied().collect();
        menu.tags = Some(tags);
        menu.after_session();
        if let (true, Some(save)) = (args.session, &args.replay)
            && is_saved_game(save)
        {
            walk_to_save(&mut menu, save)?;
        } else if let (true, Some(film)) = (args.session, &args.replay) {
            walk_to_film(&mut menu, film)?;
        } else if args.session {
            let mesh = args
                .play
                .as_deref()
                .and_then(|name| MeshQuery::from_arg(name).id)
                .ok_or_else(|| anyhow::anyhow!("--session: --play takes a mesh's four letters"))?;
            walk_to_level(&mut menu.session, mesh, args.difficulty)?;
        }
        Ok(menu)
    }

    fn with(front_end: Box<dyn WindowFrontEnd>, catalog: Catalog, progress: Progress) -> Self {
        Self {
            session: Session::new(catalog, progress),
            front_end,
            events: Vec::new(),
            event_buf: Vec::new(),
            wheel_notches: WheelNotches::default(),
            clock: Instant::now(),
            timeouts: SurfaceTimeoutLog::default(),
            sound: MenuSound::new(None, PathBuf::new(), 0),
            fade: None,
            config: InterfaceConfig::defaults(),
            game_dir: PathBuf::new(),
            progress_path: None,
            script: VecDeque::new(),
            fail_next_load: false,
            budget_frames: 0,
            shift_held: false,
            front_end_dirty: false,
            tags: None,
            on_menu: false,
            cutscene: None,
            audio: None,
            menu_filter: InterfaceFilter::default(),
        }
    }

    pub(super) fn list_films(&mut self) {
        let Some(tags) = &self.tags else {
            self.session.game_event(GameEvent::FilmsListed(
                mandrake_session::FilmFolder::default(),
            ));
            return;
        };
        let started = Instant::now();
        let folder = mandrake_session::films::list_folder(&self.game_dir, tags);
        debug!(
            "menus: {} films in {} ms",
            folder.films.len(),
            started.elapsed().as_millis()
        );
        self.front_end_dirty = true;
        self.session.game_event(GameEvent::FilmsListed(folder));
    }

    pub(super) fn describe_net_game(&self, game: &mut mandrake_session::FilmGame) {
        if let Some(tags) = &self.tags {
            mandrake_session::films::FilmTags::new(tags).look_up(game);
        }
    }

    pub(super) fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    pub(super) fn last_recording_name(&self) -> Vec<u8> {
        self.tags.as_ref().map_or_else(
            || b"Last Recording".to_vec(),
            |tags| mandrake_session::films::FilmTags::new(tags).last_recording_name(),
        )
    }

    pub(super) fn save_film(&mut self, name: &mandrake_session::Text) -> Vec<Request> {
        let last = self.last_recording_name();
        let saved = match mandrake_session::films::save_last_film(&self.game_dir, &last, &name.0) {
            Ok(file) => {
                info!("Save Film: {}", String::from_utf8_lossy(&file));
                Some(file)
            }
            Err(e) => {
                warn!("Save Film: {} wasn't saved: {e}", name.to_plain_utf8());
                None
            }
        };
        self.game_event(GameEvent::FilmSaved(saved))
    }

    pub(super) fn delete_film(&mut self, file_name: &[u8]) {
        let shown = String::from_utf8_lossy(file_name).into_owned();
        match mandrake_session::films::delete_film(&self.game_dir, file_name) {
            Ok(()) => info!("Delete Film: {shown}"),
            Err(e) => warn!("Delete Film: {shown} wasn't deleted: {e}"),
        }
    }

    pub(super) fn delete_save(&mut self, file_name: &[u8]) {
        let shown = String::from_utf8_lossy(file_name).into_owned();
        match mandrake_session::saves::delete_save(&self.game_dir, file_name) {
            Ok(()) => info!("Delete Saved Game: {shown}"),
            Err(e) => warn!("Delete Saved Game: {shown} wasn't deleted: {e}"),
        }
    }

    pub(super) fn rename_film(
        &mut self,
        file_name: &[u8],
        name: &mandrake_session::Text,
    ) -> Vec<Request> {
        let shown = String::from_utf8_lossy(file_name).into_owned();
        let now = match mandrake_session::films::rename_film(&self.game_dir, file_name, &name.0) {
            Ok(now) => {
                info!("Rename Film: {shown} is {}", String::from_utf8_lossy(&now));
                now
            }
            Err(e) => {
                warn!("Rename Film: {shown} wasn't renamed: {e}");
                file_name.to_vec()
            }
        };
        self.game_event(GameEvent::FilmRenamed(now))
    }

    pub(super) fn save_game(&mut self, name: &[u8], data: Option<Vec<u8>>) -> (bool, Vec<Request>) {
        let shown = String::from_utf8_lossy(name).into_owned();
        let written = match data {
            Some(data) => mandrake_session::saves::write_save(&self.game_dir, name, &data)
                .map_err(|e| e.to_string()),
            None => Err("this map isn't recorded".to_owned()),
        };
        let saved = match written {
            Ok(path) => {
                info!("Save Game: {}", path.display());
                let zone = mandrake_session::saves::LocalZone::system();
                self.tags.as_ref().and_then(|tags| {
                    mandrake_session::saves::read_save(
                        &path,
                        &mandrake_session::films::FilmTags::new(tags),
                        &zone,
                    )
                })
            }
            Err(e) => {
                warn!("Save Game: {shown} wasn't saved: {e}");
                None
            }
        };
        let ok = saved.is_some();
        (ok, self.game_event(GameEvent::GameSaved(saved)))
    }

    pub(super) fn quicksave_name(&mut self) -> Vec<u8> {
        let folder = self
            .tags
            .as_ref()
            .map(|tags| mandrake_session::saves::list_folder(&self.game_dir, tags))
            .unwrap_or_default();
        mandrake_session::saves::quicksave_name(&folder)
    }

    pub(super) fn list_saves(&mut self) {
        let Some(tags) = &self.tags else {
            self.session.game_event(GameEvent::SavesListed(
                mandrake_session::SaveFolder::default(),
            ));
            return;
        };
        let started = Instant::now();
        let folder = mandrake_session::saves::list_folder(&self.game_dir, tags);
        debug!(
            "menus: {} saves in {} ms",
            folder.saves.len(),
            started.elapsed().as_millis()
        );
        self.front_end_dirty = true;
        self.session.game_event(GameEvent::SavesListed(folder));
    }

    fn after_session(&mut self) {
        let on_menu = matches!(self.session.screen(), Screen::MainMenu(_));
        if on_menu && !self.on_menu {
            self.list_films();
            self.list_saves();
            let intro = !self.game_dir.as_os_str().is_empty()
                && cutscene::found(&self.game_dir, mandrake_session::INTRO_CUTSCENE);
            self.session.game_event(GameEvent::IntroFound(intro));
        }
        self.on_menu = on_menu;
    }

    pub(crate) fn for_budget(
        args: &Args,
        mesh: FourCC,
        which: BudgetOverlay,
    ) -> anyhow::Result<Self> {
        let mut menu = Self::new(
            &Args {
                session: false,
                session_script: Vec::new(),
                ..args.clone()
            },
            None,
        )?;
        menu.progress_path = None;
        let catalog = Catalog::from_tags(&CampaignTags::open(&menu.game_dir));
        menu.session = Session::new(catalog, Progress::default());
        walk_to_level(&mut menu.session, mesh, args.difficulty)?;
        menu.session.act(Action::BriefingDone);
        menu.session.game_event(GameEvent::Loaded);
        if which == BudgetOverlay::Paused {
            menu.session.act(Action::CloseHints);
            menu.session.act(Action::OpenPause);
        }
        menu.session.take_requests();
        Ok(menu)
    }

    pub(crate) fn budget_step(&mut self, size: (u32, u32), which: BudgetOverlay) -> bool {
        let ((w, h), (x, y)) = match which {
            BudgetOverlay::Hints if size.0 >= 660 && size.1 >= 450 => ((660, 450), (575, 404)),
            BudgetOverlay::Hints => ((440, 300), (383, 269)),
            BudgetOverlay::Paused => ((302, 192), (239, 166)),
        };
        let (x, y) = (size.0 as i32 / 2 - w / 2 + x, size.1 as i32 / 2 - h / 2 + y);
        self.budget_frames += 1;
        let press = self.budget_frames.is_multiple_of(15);
        let events: Vec<UiEvent> = if !press {
            Vec::new()
        } else if self.budget_frames % 30 == 15 {
            let down = UiEvent::Button {
                left: true,
                pressed: true,
            };
            vec![UiEvent::PointerMoved { x, y }, down]
        } else {
            let up = UiEvent::Button {
                left: true,
                pressed: false,
            };
            vec![UiEvent::PointerMoved { x: 0, y: 0 }, up]
        };
        let input = FrameInput {
            now_ms: self.clock.elapsed().as_millis() as u64,
            size,
            events: &events,
            sound_busy: false,
            shift_held: false,
        };
        let out = self.front_end.frame(&self.session.screen(), &input);
        debug_assert!(out.actions.is_empty(), "the press never completes");
        press
    }

    pub(super) fn game_event(&mut self, event: GameEvent) -> Vec<Request> {
        self.front_end_dirty = true;
        self.session.game_event(event);
        self.after_session();
        self.session.take_requests()
    }

    pub(super) fn overlay_open(&self) -> bool {
        matches!(self.session.screen(), Screen::Playing { overlay: Some(_) })
    }

    pub(super) fn hints_open(&self) -> bool {
        matches!(
            self.session.screen(),
            Screen::Playing {
                overlay: Some(Overlay::Hints(_))
            }
        )
    }

    pub(super) fn act(&mut self, action: Action) -> Vec<Request> {
        self.front_end_dirty = true;
        self.session.act(action);
        self.after_session();
        self.session.take_requests()
    }

    pub(super) fn over_the_game(&self) -> bool {
        matches!(self.session.screen(), Screen::Playing { .. })
    }

    pub(super) fn front_end(&mut self) -> &mut dyn mandrake_render::DrawInterface {
        self
    }

    pub(super) fn play_cutscene(&mut self, name: &str) -> Vec<Request> {
        self.sound.stop();
        self.cutscene = Cutscene::open(
            &self.game_dir,
            name,
            self.audio.clone(),
            self.config.cutscene_hd,
        );
        if self.cutscene.is_some() {
            return Vec::new();
        }
        self.game_event(GameEvent::CutsceneOver)
    }

    fn step_cutscene(&mut self) -> Vec<Request> {
        let Some(film) = &mut self.cutscene else {
            return Vec::new();
        };
        film.update(Instant::now());
        if !film.is_over() {
            return Vec::new();
        }
        self.cutscene = None;
        self.game_event(GameEvent::CutsceneOver)
    }

    pub(super) fn stop_sound(&mut self) {
        self.sound.stop();
    }

    pub(super) fn save_progress(&self, progress: &Progress) {
        let Some(path) = &self.progress_path else {
            return;
        };
        if let Err(e) = settings::save_progress(path, &self.game_dir, &progress.to_text()) {
            warn!("progress: {}: {e}", path.display());
        }
    }

    pub(super) fn has_input(&self) -> bool {
        self.session.front_end_has_input() && !self.modeless_open()
    }

    pub(super) fn modeless_open(&self) -> bool {
        self.cutscene.is_none()
            && matches!(
                self.session.screen(),
                Screen::Playing {
                    overlay: Some(Overlay::UnitTrading(_))
                }
            )
    }

    pub(super) fn modeless_rect(&self) -> Option<[i32; 4]> {
        self.modeless_open()
            .then(|| self.front_end.modeless_rect())
            .flatten()
    }

    pub(super) fn window_event(&mut self, event: &WindowEvent, held: ModifiersState) {
        if let Some(film) = &mut self.cutscene {
            if cutscene::is_skip(event) {
                film.skip();
            }
            return;
        }
        if let WindowEvent::KeyboardInput { event: key, .. } = event {
            key_events(
                key.physical_key,
                key.state == ElementState::Pressed,
                key.repeat,
                key.text.as_deref(),
                held,
                &mut self.events,
            );
        } else {
            push_ui_event(event, &mut self.wheel_notches, &mut self.events);
        }
    }

    fn run_front_end(&mut self, size: (u32, u32), now_ms: u64, shift_held: bool) {
        self.event_buf.clear();
        std::mem::swap(&mut self.events, &mut self.event_buf);
        let input = FrameInput {
            now_ms,
            size,
            events: &self.event_buf,
            sound_busy: self.sound.busy(),
            shift_held,
        };
        if let Screen::Briefing(briefing) = self.session.screen() {
            self.sound.set_plugins(&briefing.art_plugins);
        }
        let out = self.front_end.frame(&self.session.screen(), &input);
        self.event_buf.clear();
        for action in out.actions {
            debug!("menu: {action:?}");
            self.session.act(action);
        }
        for cue in out.sounds {
            self.sound.cue(cue, now_ms);
        }
    }

    pub(super) fn step(
        &mut self,
        size: (u32, u32),
        shift_held: bool,
        tick: Option<u32>,
    ) -> Vec<Request> {
        self.shift_held = shift_held;
        self.front_end_dirty = false;
        let mut film_requests = self.step_cutscene();
        let now_ms = self.clock.elapsed().as_millis() as u64;
        self.sound.update(now_ms);
        if !matches!(self.session.screen(), Screen::Postgame(_)) {
            self.sound.end_ambient();
        }
        self.run_front_end(size, now_ms, shift_held);
        self.run_script(tick);
        if let Some(action) = answer_to_unbuilt_screen(&self.session.screen()) {
            info!("menu: {action:?}: this screen has no dialog yet");
            self.session.act(action);
            self.front_end_dirty = true;
        }
        self.after_session();
        film_requests.extend(self.session.take_requests());
        film_requests
    }

    fn run_script(&mut self, tick: Option<u32>) {
        let Some(&step) = self.script.front() else {
            return;
        };
        if step == ScriptStep::FailNextLoad {
            self.script.pop_front();
            self.fail_next_load = true;
            return;
        }
        let at = |from: u32| tick.is_some_and(|tick| tick >= from);
        let actions: &[Action] = match (self.session.screen(), step) {
            (Screen::MainMenu(_), ScriptStep::Tutorial) => &[Action::Tutorial],
            (Screen::MainMenu(_), ScriptStep::Quit) => &[Action::Quit],
            (Screen::MainMenu(_), ScriptStep::Intro) => &[Action::ReplayIntro],
            (Screen::Postgame(_), ScriptStep::Restart) => &[Action::RestartLevel],
            (Screen::Postgame(_), ScriptStep::Next) => &[Action::NextLevel],
            (Screen::Postgame(_), ScriptStep::Skip) => &[Action::SkipLevel],
            (Screen::Postgame(_), ScriptStep::Menu) => &[Action::QuitToMenu],
            (Screen::FilmReport(_), ScriptStep::Menu) => &[Action::CloseReport],
            (Screen::Playing { overlay: None }, ScriptStep::Pause(t)) if at(t) => {
                &[Action::OpenPause]
            }
            (Screen::Playing { overlay: None }, ScriptStep::PauseRestart(t)) if at(t) => {
                &[Action::OpenPause, Action::RestartLevel]
            }
            (Screen::Playing { overlay: None }, ScriptStep::PauseQuit(t)) if at(t) => {
                &[Action::OpenPause, Action::QuitToMenu]
            }
            (Screen::Playing { overlay: None }, ScriptStep::PauseResume(t)) if at(t) => {
                &[Action::OpenPause, Action::Resume]
            }
            _ => return,
        };
        info!("--session-script: {step:?}");
        self.script.pop_front();
        for action in actions {
            self.session.act(action.clone());
        }
        self.front_end_dirty = true;
    }

    pub(super) fn draw(&mut self, gpu: &ShellGpu, fade: f32) {
        let Some(frame) = acquire_surface(
            &gpu.surface,
            &gpu.device,
            &gpu.surface_config,
            &mut self.timeouts,
        ) else {
            return;
        };
        let format = crate::window_surface::SurfaceFormats::of(&gpu.surface_config).interface;
        let target = window_surface::frame_texture(gpu.canvas.as_ref(), &frame);
        let view = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(format),
            ..Default::default()
        });
        let size = (gpu.surface_config.width, gpu.surface_config.height);
        let submitted = self.encode(&gpu.device, &gpu.queue, &view, format, size, fade);
        let present = gpu.canvas.as_ref().map(|canvas| {
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Front End Canvas Encoder"),
                });
            window_surface::present_canvas(Some(canvas), &mut encoder, &frame);
            encoder.finish()
        });
        gpu.queue.submit(std::iter::once(submitted).chain(present));
        gpu.queue.present(frame);
    }

    fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        fade: f32,
    ) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Menu Frame"),
        });
        clear_target(&mut encoder, target, wgpu::Color::BLACK, "Menu Clear");
        match &mut self.cutscene {
            Some(film) => film.draw(
                device,
                queue,
                &mut encoder,
                target,
                format,
                size,
                self.config.cutscene_filter.into(),
            ),
            None => DrawInterface::draw(
                self,
                &mut InterfaceFrame {
                    device,
                    queue,
                    encoder: &mut encoder,
                    target,
                    format,
                    size,
                },
            ),
        }
        if fade > 0.0 {
            self.fade
                .get_or_insert_with(|| mandrake_render::ScreenFade::new(device, format))
                .encode(&mut encoder, target, fade);
        }
        encoder.finish()
    }

    fn sync_front_end(&mut self, size: (u32, u32)) {
        if !self.front_end_dirty {
            return;
        }
        self.front_end_dirty = false;
        let now_ms = self.clock.elapsed().as_millis() as u64;
        self.run_front_end(size, now_ms, self.shift_held);
    }
}

impl DrawInterface for Menu {
    fn draw(&mut self, frame: &mut InterfaceFrame<'_>) {
        self.sync_front_end(frame.size);
        self.front_end.draw(frame);
    }
}

pub(crate) struct OwnPauseDialog {
    front_end: Box<dyn WindowFrontEnd>,
    events: Vec<UiEvent>,
    event_buf: Vec<UiEvent>,
    wheel_notches: WheelNotches,
    clock: Instant,
    data: Option<mandrake_session::Paused>,
    pub(crate) held: ModifiersState,
    pub(crate) cursor_in_window: bool,
}

impl OwnPauseDialog {
    pub(crate) fn new(args: &Args) -> anyhow::Result<Self> {
        let config = crate::interface_config::load(args.conf.as_deref(), args.headless);
        let game_dir = args
            .archive
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| anyhow::anyhow!("--archive {:?} has no game folder", args.archive))?;
        let ui_filter = args.ui_filter.map(Into::into);
        Ok(Self {
            front_end: engine_front_end(game_dir, config, ui_filter, 0)?,
            events: Vec::new(),
            event_buf: Vec::new(),
            wheel_notches: WheelNotches::default(),
            clock: Instant::now(),
            data: None,
            held: ModifiersState::empty(),
            cursor_in_window: true,
        })
    }

    pub(crate) fn is_open(&self) -> bool {
        self.data.is_some()
    }

    pub(crate) fn open(&mut self, data: mandrake_session::Paused) {
        self.events.clear();
        self.wheel_notches.clear();
        self.data = Some(data);
    }

    pub(crate) fn close(&mut self, size: (u32, u32)) {
        self.data = None;
        self.events.clear();
        self.wheel_notches.clear();
        let screen = Screen::Playing { overlay: None };
        self.frame(&screen, size);
    }

    pub(crate) fn window_event(&mut self, event: &WindowEvent) {
        if let WindowEvent::ModifiersChanged(modifiers) = event {
            self.held = modifiers.state();
        } else if let WindowEvent::KeyboardInput { event: key, .. } = event {
            key_events(
                key.physical_key,
                key.state == ElementState::Pressed,
                key.repeat,
                key.text.as_deref(),
                self.held,
                &mut self.events,
            );
        } else {
            push_ui_event(event, &mut self.wheel_notches, &mut self.events);
        }
    }

    pub(crate) fn step(&mut self, size: (u32, u32)) -> Vec<Action> {
        let Some(data) = self.data.take() else {
            return Vec::new();
        };
        let screen = Screen::Playing {
            overlay: Some(Overlay::Paused(&data)),
        };
        let actions = self.frame(&screen, size);
        self.data = Some(data);
        actions
    }

    fn frame(&mut self, screen: &Screen<'_>, size: (u32, u32)) -> Vec<Action> {
        self.event_buf.clear();
        std::mem::swap(&mut self.events, &mut self.event_buf);
        let input = FrameInput {
            now_ms: self.clock.elapsed().as_millis() as u64,
            size,
            events: &self.event_buf,
            sound_busy: false,
            shift_held: false,
        };
        let actions = self.front_end.frame(screen, &input).actions;
        self.event_buf.clear();
        actions
    }

    pub(crate) fn front_end(&mut self) -> &mut dyn DrawInterface {
        &mut *self.front_end
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mandrake_session::FrameOutput;
    use std::cell::RefCell;
    use std::rc::Rc;
    use winit::event::{DeviceId, MouseScrollDelta};

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum FrontEndCall {
        Frame(String),
        Draw,
    }

    #[derive(Default)]
    struct Seen {
        shift: Vec<bool>,
        sizes: Vec<(u32, u32)>,
        events: Vec<UiEvent>,
        answers: Vec<Action>,
        calls: Vec<FrontEndCall>,
    }
    struct Fake(Rc<RefCell<Seen>>);
    impl FrontEnd for Fake {
        fn frame(&mut self, screen: &Screen<'_>, input: &FrameInput<'_>) -> FrameOutput {
            let screen_name = match screen {
                Screen::Loading => "loading",
                Screen::Playing {
                    overlay: Some(Overlay::Hints(_)),
                } => "hints",
                Screen::Playing { .. } => "playing",
                _ => "other",
            };
            let mut seen = self.0.borrow_mut();
            seen.calls
                .push(FrontEndCall::Frame(screen_name.to_string()));
            seen.shift.push(input.shift_held);
            seen.sizes.push(input.size);
            seen.events.extend_from_slice(input.events);
            FrameOutput {
                actions: std::mem::take(&mut seen.answers),
                sounds: Vec::new(),
            }
        }
    }
    impl DrawInterface for Fake {
        fn draw(&mut self, _frame: &mut InterfaceFrame<'_>) {
            self.0.borrow_mut().calls.push(FrontEndCall::Draw);
        }
    }

    fn menu() -> (Menu, Rc<RefCell<Seen>>) {
        let seen = Rc::new(RefCell::new(Seen::default()));
        let menu = Menu::with(
            Box::new(Fake(seen.clone())),
            Catalog::default(),
            Progress::default(),
        );
        (menu, seen)
    }

    #[test]
    fn budget_step_centers_click_for_screen_size() {
        let (mut m, seen) = menu();
        for _ in 0..15 {
            m.budget_step((1920, 1080), BudgetOverlay::Hints);
        }
        let pointer = seen
            .borrow()
            .events
            .iter()
            .find_map(|e| match *e {
                UiEvent::PointerMoved { x, y } => Some((x, y)),
                _ => None,
            })
            .expect("pointer event at frame 15");
        assert_eq!(pointer, (1205, 719));

        let (mut m, seen) = menu();
        for _ in 0..15 {
            m.budget_step((800, 600), BudgetOverlay::Hints);
        }
        let pointer = seen
            .borrow()
            .events
            .iter()
            .find_map(|e| match *e {
                UiEvent::PointerMoved { x, y } => Some((x, y)),
                _ => None,
            })
            .expect("pointer event at frame 15");
        assert_eq!(pointer, (645, 479));

        let (mut m, seen) = menu();
        for _ in 0..15 {
            m.budget_step((640, 480), BudgetOverlay::Hints);
        }
        let pointer = seen
            .borrow()
            .events
            .iter()
            .find_map(|e| match *e {
                UiEvent::PointerMoved { x, y } => Some((x, y)),
                _ => None,
            })
            .expect("pointer event at frame 15");
        assert_eq!(pointer, (483, 359));
    }

    #[test]
    fn quit_asks_the_shell_to_leave() {
        let (mut menu, seen) = menu();
        assert!(menu.has_input());
        assert_eq!(menu.step((800, 600), false, None), []);
        seen.borrow_mut().answers.push(Action::Quit);
        assert_eq!(menu.step((800, 600), false, None), [Request::Exit]);
    }

    #[test]
    fn new_game_opens() {
        let (mut menu, seen) = menu();
        seen.borrow_mut()
            .answers
            .push(Action::NewGame { all_levels: true });
        assert_eq!(menu.step((800, 600), true, None), []);
        assert!(matches!(menu.session.screen(), Screen::NewGame(_)));
    }

    #[test]
    fn front_end_syncs_before_drawing_when_screen_transitions_mid_frame() {
        let seen = Rc::new(RefCell::new(Seen::default()));
        let catalog = Catalog {
            levels: vec![mandrake_session::catalog::Level::new(
                mandrake_session::rules::TUTORIAL,
            )],
            ..Catalog::default()
        };
        let mut menu = Menu::with(Box::new(Fake(seen.clone())), catalog, Progress::default());
        seen.borrow_mut().answers.push(Action::Tutorial);
        let _ = menu.step((800, 600), false, None);
        assert_eq!(menu.session.screen(), Screen::Loading);

        let _ = menu.step((800, 600), false, None);
        assert_eq!(
            seen.borrow().calls.last(),
            Some(&FrontEndCall::Frame("loading".to_string()))
        );
        assert!(!menu.front_end_dirty);

        menu.game_event(GameEvent::Loaded);
        assert!(menu.hints_open());
        assert!(menu.overlay_open());
        assert!(menu.front_end_dirty);

        if let Some((device, queue)) = crate::tests::test_gpu() {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 800,
                    height: 600,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let target = texture.create_view(&Default::default());
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut frame = InterfaceFrame {
                device: &device,
                queue: &queue,
                encoder: &mut encoder,
                target: &target,
                format: wgpu::TextureFormat::Bgra8Unorm,
                size: (800, 600),
            };
            menu.front_end().draw(&mut frame);
            assert!(!menu.front_end_dirty);

            let calls = &seen.borrow().calls;
            let len = calls.len();
            assert!(len >= 2);
            assert_eq!(calls[len - 2], FrontEndCall::Frame("hints".to_string()));
            assert_eq!(calls[len - 1], FrontEndCall::Draw);
        } else {
            menu.sync_front_end((800, 600));
            assert!(!menu.front_end_dirty);
            assert_eq!(
                seen.borrow().calls.last(),
                Some(&FrontEndCall::Frame("hints".to_string()))
            );
        }
    }

    #[test]
    fn the_script_waits_for_its_screen_and_its_tick() {
        let (mut menu, seen) = menu();
        menu.script = [ScriptStep::PauseQuit(5), ScriptStep::Quit]
            .into_iter()
            .collect();
        assert_eq!(menu.step((800, 600), false, None), []);
        seen.borrow_mut().answers.push(Action::Tutorial);
        menu.step((800, 600), false, None);
        assert_eq!(
            menu.game_event(GameEvent::Loaded).first(),
            Some(&Request::BeginPlay)
        );
        menu.act(Action::CloseHints);
        menu.step((800, 600), false, Some(4));
        assert_eq!(menu.script.len(), 2);
        let requests = menu.step((800, 600), false, Some(5));
        assert_eq!(requests, [Request::SetPaused(true), Request::DropGame]);
        assert_eq!(menu.step((800, 600), false, None), [Request::Exit]);
    }

    #[test]
    fn progress_that_wont_read_isnt_saved_over() {
        let dir = std::env::temp_dir().join(format!("m2-menu-progress-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.txt");
        let game = dir.join("no game");
        assert_eq!(
            read_progress(Some(&path), &game),
            (Progress::default(), true)
        );
        std::fs::write(&path, "difficulty hard\n").unwrap();
        assert!(!read_progress(Some(&path), &game).1);
        std::fs::create_dir(path.with_extension("dir")).unwrap();
        assert!(!read_progress(Some(&path.with_extension("dir")), &game).1);
        assert_eq!(read_progress(None, &game), (Progress::default(), false));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_live_shift_and_the_size_reach_the_front_end() {
        let (mut menu, seen) = menu();
        menu.step((1920, 1080), false, None);
        menu.step((1920, 1080), true, None);
        assert_eq!(seen.borrow().shift, [false, true]);
        assert_eq!(seen.borrow().sizes[1], (1920, 1080));
    }

    #[test]
    fn window_events_are_queued_in_window_pixels_until_the_frame() {
        let (mut menu, seen) = menu();
        let device_id = DeviceId::dummy();
        for event in [
            WindowEvent::CursorMoved {
                device_id,
                position: winit::dpi::PhysicalPosition::new(844.75, 298.5),
            },
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Released,
                button: MouseButton::Middle,
            },
            WindowEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(0.0, -2.0),
                phase: winit::event::TouchPhase::Moved,
            },
        ] {
            menu.window_event(&event, ModifiersState::empty());
        }
        assert!(seen.borrow().events.is_empty());
        menu.step((3840, 2160), false, None);
        assert_eq!(
            seen.borrow().events,
            [
                UiEvent::PointerMoved { x: 844, y: 298 },
                UiEvent::Button {
                    left: true,
                    pressed: true
                },
                UiEvent::Wheel(-1),
            ]
        );
    }

    #[test]
    fn keys_carry_their_modifiers_and_only_plain_ones_type() {
        let key = |code, text, held| {
            let mut out = Vec::new();
            key_events(PhysicalKey::Code(code), true, false, text, held, &mut out);
            out
        };
        let alt_q = UiEvent::Key {
            key: UiKey::Char('q'),
            pressed: true,
            repeat: false,
            shift: false,
            alt: true,
        };
        assert_eq!(key(KeyCode::KeyQ, Some("q"), ModifiersState::ALT), [alt_q]);
        let shift_a = key(KeyCode::KeyA, Some("A"), ModifiersState::SHIFT);
        assert_eq!(shift_a[1], UiEvent::Text('A'));
        assert_eq!(
            key(KeyCode::Escape, Some("\u{1b}"), ModifiersState::empty()).len(),
            1
        );
        assert!(key(KeyCode::F1, None, ModifiersState::empty()).is_empty());
        assert_eq!(ui_key(KeyCode::NumpadEnter), Some(UiKey::Return));
    }
}
