use crate::{tfl_frame_parts, tfl_input, tfl_live, tfl_menu, tfl_sound};

use crate::app::cursor::{CursorBitmap, SoftwareCursor};
use crate::app::cutscene::{Cutscene, HdLookup};
use crate::app::game_scene::{FrameScene, GameScene, SceneFrame, SceneSetup};
use crate::app::input::ModelessRoute;
use crate::app::menu_sound::{MenuSound, NO_MENU_MUSIC};
use crate::app::shell::ShellHost;
use crate::app::shell_game::{LoadFlow, LoadWork};
use crate::app::view_settings::ViewSettings;
use crate::args::Args;
use crate::audio_out::AudioLink;
use crate::camera_control::MotionClock;
use crate::carnage_report::{EndStage, MapEnd};
use crate::controls::{ControlAction, Controls, HeldKeys};
use crate::move_marker::{LocalSprite, LocalSprites, TFL_MAX_LOCAL_SPRITES};
use crate::object_history::{self, FRACTION_ONE};
use crate::render_passes::WorldTargets;
use crate::tfl_game::TflReplay;
use crate::tfl_hud::{BarButton, TflHud, TflHudSetup, TflHudView};
use crate::tfl_map::TflMap;
use crate::tfl_map::load_tfl_map;
use crate::tfl_objects::TFL_CELL_SIZE;
use crate::tfl_view::{TflFrame, TflMarkers, TflScene, start_camera};
use crate::ui_scale::{UiFilter, UiScale};
use mandrake_common::FourCC;
use mandrake_dialogs::canvas::CanvasFit;
use mandrake_render::RenderPipelineConfig;
use mandrake_render::{
    InterfaceFrame, InterfaceScaler, OverlayDraw, OverlayImage, OverlayLayer, OverlayRenderer,
};
use mandrake_session::UiEvent;
use mandrake_sim::game_speed::{GameMode, GameSpeed};
use mandrake_view::camera::Camera;
use mandrake_view::camera::{CameraState, ObserverController};
use mandrake_view::interface_bitmap::InterfaceArt;
use mandrake_view::interface_picture::InterfaceFilter;
use mandrake_view::selection::state::{HoverPick, HoverTracker};
use std::time::Duration;

use crate::clock::Instant;
use tfl_frame_parts::Part;
use tfl_menu::dialogs::{CANVAS, MenuInput, TflDialogs, TflQuitDialog};
use tfl_menu::{ProgressFile, TflLaunch, TflMovie};
use tracing::{debug, info};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

pub(crate) fn opens_menus(args: &Args) -> bool {
    let mut a = args.clone();
    a.frame_budget = None;
    crate::args::wants_menu(&a) || a.play.is_some()
}

const NETWORK_GAME_MODE: i16 = 1;

const TICK: Duration = Duration::from_micros(33_333);

const CATCH_UP: Duration = Duration::from_millis(250);

fn catch_up_from(last_tick: Instant, now: Instant) -> Instant {
    match now.checked_sub(CATCH_UP) {
        Some(earliest) if earliest > last_tick => earliest,
        _ => last_tick,
    }
}

fn camera_alpha(since_tick: Duration) -> f32 {
    (since_tick.as_secs_f32() / TICK.as_secs_f32()).clamp(0.0, 1.0)
}

pub(crate) fn run_tfl_window(
    args: &Args,
    map: TflMap,
    replay: Option<TflReplay>,
) -> anyhow::Result<()> {
    run_window(args, map, replay, None)
}

pub(crate) fn run_tfl_net_window(
    args: &Args,
    game_dir: &std::path::Path,
    net: crate::tfl_net::TflNetWindow,
) -> anyhow::Result<()> {
    let mut net = net;
    let (map, replay) = net.load(game_dir)?;
    run_window(args, map, replay, Some(Box::new(net)))
}

fn run_window(
    args: &Args,
    map: TflMap,
    replay: Option<TflReplay>,
    net: Option<Box<crate::tfl_net::TflNetWindow>>,
) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let (controls, _) = crate::controls::tfl::load(args.conf.as_deref());
    let mut host = crate::app::shell::ShellHost::new(args.clone());
    let interp = host.view().sprite_interpolation;
    crate::sprite_interp::install(interp);
    println!(
        "TFL: sprite in-betweens {} (F9 toggles; cache {})",
        if interp { "on" } else { "off" },
        crate::sprite_interp::cache_dir("tfl").map_or("none".into(), |d| d.display().to_string())
    );
    let scene = host.make_scene(
        |args, setup| {
            let mut app = TflApp::new(args, setup, map, replay, controls.clone())?;
            if let Some(net) = net {
                app.start_network_game(net);
            }
            Ok(Box::new(app))
        },
        controls.clone(),
    )?;
    let shell = crate::app::shell::Shell::new(host, scene);
    let shell = crate::app::shell::run_event_loop(event_loop, shell)?;
    match shell.and_then(|mut s| s.take_failure()) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn tfl_cursor_from_archive(archive: &mandrake_tag::TagArchive) -> Option<CursorBitmap> {
    const CURSOR_SEQUENCE: i32 = 3;
    let collection = interface_collection(archive);
    let art = InterfaceArt::from_bytes(crate::tfl_map::tag_data(archive, b".256", collection)?)?;
    CursorBitmap::of_art(&art, CURSOR_SEQUENCE)
}

fn interface_collection(archive: &mandrake_tag::TagArchive) -> FourCC {
    let stin = FourCC::new(*b"stin");
    crate::tfl_map::tag_data(archive, b"inte", stin)
        .and_then(|d| mandrake_tag::interface::InterfaceDefinition::from_bytes(&d).ok())
        .and_then(|d| d.collection)
        .unwrap_or(stin)
}

struct Gpu<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    interface_format: wgpu::TextureFormat,
    ui: UiScale,
    ui_filter: InterfaceFilter,
    menu_filter: InterfaceFilter,
}

struct GameView {
    map: TflMap,
    camera: Camera,
    observer: ObserverController,
    replay: Option<TflReplay>,
    speed: GameSpeed,
    paused: bool,
    typed: tfl_input::TypedKeys,
    presses: Vec<(ControlAction, bool)>,
    obmo: ObmoCamera,
    camera_control: crate::camera_control::CameraControl,
    scene: Option<TflScene>,
    live: Option<Live>,
    quit: Option<TflQuitDialog>,
    quit_events: Vec<UiEvent>,
    net_pause: Option<mandrake_session::Paused>,
    net_trade: Option<mandrake_session::screen::UnitTrading>,
    modeless_press: bool,
    quit_gpu: Option<LiveGpu>,
    film_sound: Option<tfl_sound::TflLevelSound>,
    hud: Option<TflHud>,
    hud_gpu: Option<HudGpu>,
    world: u64,
    smooth: bool,
    sound_rules: tfl_sound::TflSoundRules,
    camera_before: CameraState,
    render_camera: Camera,
    motion: MotionClock,
    motion_tick: Option<u32>,
    fraction: i32,
    film_end: Option<u32>,
    end: MapEnd,
    end_announced: bool,
    hover_frames: bool,
    hover: HoverTracker,
    look: Option<crate::tfl_view::TflLook>,
    snapshots: tfl_menu::TflSnapshots,
    snapshot_copy: Option<Option<crate::screenshot::Readback>>,
    restoring: Option<u32>,
    film_ui: Option<tfl_input::FilmInterface>,
    film_mouse: Mouse,
    film_time: u64,
}

const RESTORE_FRAME_BUDGET: Duration = Duration::from_millis(100);

const END_GAME_TICKS: i32 = 300;

fn end_stage(replay: Option<&TflReplay>, film_end: Option<u32>) -> EndStage {
    let Some(sim) = replay
        .filter(|r| r.stop().is_none())
        .and_then(TflReplay::sim)
    else {
        return EndStage::Playing;
    };
    match sim.map_actions.game.ended_at {
        Some(at) => match END_GAME_TICKS - sim.tick.saturating_sub(at) as i32 {
            left if left <= 0 => EndStage::Over,
            left => EndStage::Live { ticks_left: left },
        },
        None if film_end.is_some_and(|end| sim.tick >= end) => EndStage::FilmFrozen,
        None => EndStage::Playing,
    }
}

fn report_follows(game: &GameView) -> bool {
    game.live.is_none()
        && game
            .replay
            .as_ref()
            .and_then(TflReplay::sim)
            .is_some_and(|s| s.game_options & mandrake_common::game_options::COOPERATIVE == 0)
}

fn steps_on(replay: Option<&TflReplay>, film_end: Option<u32>) -> bool {
    !matches!(
        end_stage(replay, film_end),
        EndStage::Over | EndStage::FilmFrozen
    )
}

fn live_viewer(sim: &mandrake_tfl_sim::TflSim) -> crate::tfl_hud_selection::Viewer {
    crate::tfl_hud_selection::Viewer {
        player: sim.local_player() as i8,
        team: sim.local_team().ok().flatten().unwrap_or(-1),
    }
}

struct HudGpu {
    overlay: OverlayRenderer,
    images: Vec<crate::hud::OverlayImageSlot>,
    scaler: InterfaceScaler,
    fps: crate::hud::FpsOverlay,
    minimap: crate::minimap::MinimapImages,
    minimap_revision: Option<u64>,
}

fn sound_log_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("M2_TFL_SOUND_LOG").is_some())
}

#[allow(clippy::too_many_arguments)]
fn play_sounds(
    sound: &mut tfl_sound::TflLevelSound,
    map: &crate::tfl_map::TflMap,
    scene: Option<&mut TflScene>,
    replay: &mut Option<TflReplay>,
    observer: &ObserverController,
    stepped: u32,
    voices: Vec<mandrake_tfl_sim::SoundEvent>,
    hud: Option<&mut TflHud>,
) {
    let log = sound_log_enabled();
    if log {
        for v in &voices {
            if let mandrake_tfl_sim::SoundKind::MonsterVoice(kind) = v.kind {
                tracing::debug!(
                    "TFL voice: tick {}: kind {kind} {:?} by object {:?}",
                    v.tick,
                    v.sound,
                    v.object
                );
            }
        }
    }
    let mut events = voices;
    events.extend(
        replay
            .as_mut()
            .map(TflReplay::take_sound_events)
            .unwrap_or_default(),
    );
    let news = replay
        .as_mut()
        .and_then(TflReplay::sim_mut)
        .map(|sim| (sim.tick, crate::tfl_net::interface_news(sim)));
    let mut hud = hud;
    if let (Some((tick, news)), Some(hud)) = (news.as_ref(), hud.as_deref_mut()) {
        for sketch in &news.sketches {
            if hud.sketch(sketch.flags, &sketch.points) {
                events.push(crate::tfl_net::game_sound(
                    *tick,
                    crate::tfl_net::SKETCH_SOUND_SITE,
                    crate::tfl_net::SKETCH_SOUND,
                ));
            }
        }
    }
    let cells = (
        map.tfl_mesh.cell_width as i32,
        map.tfl_mesh.cell_height as i32,
    );
    let view = TflHudView::of_observer(observer.origin, observer.yaw, cells);
    let height = (observer.origin[1] * crate::tfl_objects::WORLD_PER_RENDER_UNIT) as i32;
    let listener = sound.listener([view.observer.0, view.observer.1, height], view.facing);
    let local = scene.map(TflScene::take_local_sounds).unwrap_or_default();
    sound.set_observer(replay.as_ref().is_some_and(TflReplay::observer));
    sound.tick_with_local(
        &events,
        &local,
        replay.as_ref().and_then(TflReplay::sim),
        &map.archive,
        &listener,
        stepped,
    );
    let captions = sound.take_captions();
    if let Some(hud) = hud {
        for text in &captions {
            hud.print(text);
        }
        for chat in news.iter().flat_map(|(_, n)| &n.chats) {
            hud.chat(chat.player, &chat.text);
        }
    }
    for p in sound.take_played() {
        if log {
            tracing::debug!(
                "TFL sound: tick {}: {} on channel {} at {:?}, listener {:?}, volume {:#x} attenuated {:#x} left {:#x} right {:#x}",
                p.tick,
                p.sound,
                p.channel,
                p.location.map(|w| (w.x, w.y)),
                (listener.location.x, listener.location.y, listener.facing),
                p.volumes.volume,
                p.volumes.attenuated,
                p.volumes.left,
                p.volumes.right
            );
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum LiveGame {
    Solo(tfl_live::TflLive),
    Net(Box<crate::tfl_net::TflNetWindow>),
}

impl LiveGame {
    fn tick(&self) -> u32 {
        match self {
            Self::Solo(l) => l.tick(),
            Self::Net(n) => n.tick(),
        }
    }

    fn stop(&self) -> Option<String> {
        match self {
            Self::Solo(l) => l.stop().map(ToString::to_string),
            Self::Net(n) => n.stop().map(str::to_string),
        }
    }

    fn give(&mut self, command: mandrake_rec::CommandPacket) -> bool {
        match self {
            Self::Solo(l) => l.give(command),
            Self::Net(n) => crate::tfl_net::give(&mut n.play.net, &command),
        }
    }

    fn solo(&self) -> Option<&tfl_live::TflLive> {
        match self {
            Self::Solo(l) => Some(l),
            Self::Net(_) => None,
        }
    }

    fn solo_mut(&mut self) -> Option<&mut tfl_live::TflLive> {
        match self {
            Self::Solo(l) => Some(l),
            Self::Net(_) => None,
        }
    }

    fn net(&self) -> Option<&crate::tfl_net::TflNetWindow> {
        match self {
            Self::Solo(_) => None,
            Self::Net(n) => Some(n),
        }
    }

    fn net_mut(&mut self) -> Option<&mut crate::tfl_net::TflNetWindow> {
        match self {
            Self::Solo(_) => None,
            Self::Net(n) => Some(n),
        }
    }

    fn is_net(&self) -> bool {
        matches!(self, Self::Net(_))
    }
}

struct Live {
    game: LiveGame,
    selection: tfl_input::TflSelection,
    sound: tfl_sound::TflLevelSound,
    mouse: Mouse,
    time: u64,
    local_sprites: LocalSprites,
    marker_collection: FourCC,
    game_time: i32,
    level: FourCC,
    difficulty: i16,
}

impl Live {
    fn start(
        view: &GameView,
        mut game: LiveGame,
        args: &Args,
        audio: Option<AudioLink>,
        (level, difficulty): (FourCC, i16),
    ) -> Self {
        let sources = view
            .replay
            .as_ref()
            .and_then(TflReplay::sim)
            .map(|s| s.sound_sources().to_vec())
            .unwrap_or_default();
        let mut sound = tfl_sound::TflLevelSound::open(
            &view.map,
            &sources,
            audio.filter(|_| !args.mute),
            crate::app::sound::sound_preference_flags(&args.archive),
            view.sound_rules,
        );
        if let LiveGame::Solo(solo) = &mut game {
            sound.set_restored_ambient_volumes(&solo.take_restored_ambient_volumes());
        }
        if game.is_net() {
            sound.load_myth2_whisper(&args.archive);
        }
        println!("TFL game: sound: {}", sound.summary());
        let mut selection = tfl_input::TflSelection::with_rules(selection_rules(args));
        selection.strings = tfl_input::SelectionStrings::of_map(&view.map);
        selection.chat_allowed = game.is_net();
        let game_time = game.tick() as i32;
        Self {
            game,
            selection,
            sound,
            mouse: Mouse::default(),
            time: 0,
            local_sprites: LocalSprites::with_capacity(TFL_MAX_LOCAL_SPRITES),
            marker_collection: interface_collection(&view.map.archive),
            game_time,
            level,
            difficulty,
        }
    }
}

const SKETCH_COMMAND: u8 = 12;

pub(crate) fn sketch_payload(flags: u16, points: &[u16]) -> Vec<u8> {
    let mut p = flags.to_be_bytes().to_vec();
    p.extend((points.len() as i16).to_be_bytes());
    for w in points {
        p.extend(w.to_be_bytes());
    }
    p
}

fn net_news(
    net: &crate::tfl_net::TflNetWindow,
    frame: crate::tfl_net::WindowFrame,
    mut hud: Option<&mut TflHud>,
    sounds: &mut Vec<mandrake_tfl_sim::SoundEvent>,
) {
    let tick = net.tick();
    for (chat, shown) in frame.chats {
        let name = usize::try_from(chat.player)
            .ok()
            .and_then(|p| net.names.get(p))
            .cloned()
            .unwrap_or_default();
        println!(
            "TFL game: chat from player {} ({name}){} at tick {tick}: {}{}",
            chat.player,
            if chat.team_only { " to its team" } else { "" },
            String::from_utf8_lossy(&chat.text),
            if shown {
                format!(", sound {:#x}", crate::tfl_net::chat_sound(chat.team_only))
            } else {
                " (not shown: another team's)".into()
            }
        );
        if shown {
            if let Some(h) = hud.as_deref_mut() {
                h.chat(chat.player, &chat.text);
            }
            sounds.push(crate::tfl_net::game_sound(
                tick,
                crate::tfl_net::CHAT_SOUND_SITE,
                crate::tfl_net::chat_sound(chat.team_only),
            ));
        }
    }
    for (player, name) in frame.dropped {
        println!("TFL game: player {player} ({name}) left the game: dropped at tick {tick}");
        if let Some(h) = hud.as_deref_mut() {
            h.print(&net.left_line(&name));
        }
    }
}

pub(crate) fn drawn_world(
    replay: Option<&TflReplay>,
    fraction: i32,
) -> crate::world_view::WorldView {
    drawn_world_with_live(replay, fraction, None)
}

fn drawn_world_with_live(
    replay: Option<&TflReplay>,
    fraction: i32,
    live: Option<&crate::world_view::tfl::ObjectHistory>,
) -> crate::world_view::WorldView {
    replay.map_or_else(Default::default, |r| {
        crate::world_view::tfl::pick_view(r, fraction, live)
    })
}

#[derive(Default)]
struct Mouse {
    cursor: (i32, i32),
    left: bool,
    right: bool,
    left_pressed: bool,
    right_pressed: bool,
    actions: crate::selection_input::ActionBuilder,
    press_cursor: Option<(i32, i32)>,
    shift: bool,
    alt: bool,
    option: bool,
    control: bool,
    attack_ground: bool,
    recall_preset: bool,
    number: Option<usize>,
    center_held: bool,
    overhead_map: bool,
}

pub(crate) const TFL_MOVE: crate::camera_control::MoveRules = crate::camera_control::MoveRules {
    line: 0x40,
    far: 0xf00,
    maxz: false,
    at_least_one: false,
    cell_shift: 10,
    arctangent: mandrake_tfl_sim::trig::arctangent,
};

#[derive(Debug)]
struct ObmoCamera {
    movement: Option<crate::camera_control::MoveBlock>,
    tracking: mandrake_tfl_sim::tracking::Tracking,
    random: mandrake_tag::sound_manager::LocalRandom,
    tracked_tick: Option<u32>,
}

impl Default for ObmoCamera {
    fn default() -> Self {
        Self {
            movement: None,
            tracking: Default::default(),
            random: mandrake_tag::sound_manager::LocalRandom::new(0),
            tracked_tick: None,
        }
    }
}

impl ObmoCamera {
    fn consume(
        &mut self,
        sim: &mut mandrake_tfl_sim::TflSim,
        observer: &ObserverController,
        cells: (i32, i32),
    ) {
        use mandrake_tfl_sim::procs::CameraMoveTarget;
        for event in sim.take_camera_movements() {
            let view = TflHudView::of_observer(observer.origin, observer.yaw, cells);
            let from = crate::camera_control::ObserverLocation {
                point: view.observer,
                facing: view.facing,
            };
            let (to, facing) = match event.target {
                CameraMoveTarget::Point { point, facing } => (point, facing),
                CameraMoveTarget::Monster { position, .. } => {
                    crate::camera_control::subject_target_as(from, position, cells, &TFL_MOVE)
                }
            };
            let mv = mandrake_sim::camera_actions::CameraMove {
                target: mandrake_sim::camera_actions::CameraTarget::Point { point: to, facing },
                line: event.line,
                angu: event.angu,
                time: event.time,
                maxz: false,
            };
            let block = crate::camera_control::MoveBlock::start_as(
                from,
                (observer.zoom * 65536.0) as i32,
                to,
                facing,
                &mv,
                &TFL_MOVE,
            );
            println!(
                "TFL game: obmo {} at tick {}: the camera to {to:?} facing {facing:#x} in {} ticks",
                event.identifier, event.tick, block.duration
            );
            self.movement = Some(block);
        }
    }

    fn flags(replay: Option<&TflReplay>, live: bool) -> u16 {
        replay.and_then(TflReplay::sim).map_or(0, |s| {
            if live {
                s.camera_control_flags
            } else {
                s.camera_control_flags & crate::camera_control::FLAG_TRACK
            }
        })
    }

    fn control(
        &mut self,
        flags: u16,
        observer: &mut ObserverController,
        map: &TflMap,
        tick: u32,
    ) -> u16 {
        use crate::camera_control::{FLAG_LOCK, FLAG_MOVE};
        if flags == FLAG_LOCK {
        } else if flags & FLAG_MOVE != 0 && self.step(observer, map, tick) {
            return 0;
        }
        flags
    }

    fn track_poll(
        &mut self,
        sim: &mandrake_tfl_sim::TflSim,
        speed: i32,
        observer: &mut ObserverController,
        map: &TflMap,
        axes: &mut mandrake_view::camera::myth_observer::ObserverAxes,
    ) {
        use mandrake_tfl_sim::tracking::TrackingPoll;
        match self.tracking.poll(sim, speed) {
            TrackingPoll::Hold => {}
            TrackingPoll::Move { dx, dy } => {
                let cells = (
                    map.tfl_mesh.cell_width as i32,
                    map.tfl_mesh.cell_height as i32,
                );
                let view = TflHudView::of_observer(observer.origin, observer.yaw, cells);
                let to = (
                    view.observer.0 + i32::from(dx),
                    view.observer.1 + i32::from(dy),
                );
                if to != view.observer {
                    place_observer(observer, map, to, view.facing);
                }
            }
            TrackingPoll::Turn { left } => axes.orbit_left = if left { 1.0 } else { -1.0 },
        }
    }

    fn step(&mut self, observer: &mut ObserverController, map: &TflMap, _tick: u32) -> bool {
        let Some(b) = self.movement.as_mut() else {
            return true;
        };
        let cells = (
            map.tfl_mesh.cell_width as i32,
            map.tfl_mesh.cell_height as i32,
        );
        let own = TflHudView::of_observer(observer.origin, observer.yaw, cells).facing;
        let step = b.advance();
        place_observer(observer, map, step.point, step.facing.unwrap_or(own));
        observer.zoom = step.zoom as f32 / 65536.0;
        let over = b.done();
        if over {
            println!(
                "TFL game: obmo move over: point {:?} facing {:#x} zoom {} height {} pitch {}",
                step.point,
                step.facing.unwrap_or(own),
                observer.zoom,
                observer.origin[1],
                observer.base_elevation
            );
        }
        over
    }
}

fn centre_observer_on(observer: &mut ObserverController, map: &TflMap, point: [i32; 3]) {
    let cells = (
        map.tfl_mesh.cell_width as i32,
        map.tfl_mesh.cell_height as i32,
    );
    let view = TflHudView::of_observer(observer.origin, observer.yaw, cells);
    let (to, facing) = mandrake_tfl_sim::tracking::centre_observer(point, view.observer, cells);
    place_observer(observer, map, to, facing);
}

fn track(game: &mut GameView) {
    let GameView {
        obmo,
        scene,
        replay,
        observer,
        map,
        ..
    } = game;
    let Some(sim) = replay.as_mut().and_then(TflReplay::sim_mut) else {
        return;
    };
    let events = sim.take_interest_events();
    let sim: &mandrake_tfl_sim::TflSim = sim;
    let ObmoCamera {
        tracking,
        random: own,
        tracked_tick,
        ..
    } = obmo;
    let random = match scene.as_mut() {
        Some(s) => s.local_random_mut(),
        None => own,
    };
    let mut draw = |_site: u32| random.next_u16();
    let flags = sim.camera_control_flags;
    let mut asks = Vec::new();
    let last = match *tracked_tick {
        Some(t) => t,
        None => {
            asks.extend(tracking.new_map(flags, sim, &mut draw));
            sim.tick
        }
    };
    for e in &events {
        if let Some(a) = tracking.event(e, &mut draw) {
            tracing::debug!(
                "TFL game: tracking: tick {} object {} (interest {}, site {:#x})",
                e.tick,
                e.object,
                e.interest,
                e.site
            );
            asks.push(a);
        }
    }
    *tracked_tick = Some(sim.tick);
    if flags & crate::camera_control::FLAG_TRACK != 0 {
        let ticks = sim.tick.saturating_sub(last) as i32;
        let before = tracking.object;
        let chosen = tracking.update(ticks, sim, &mut draw);
        if !chosen.is_empty() {
            tracing::debug!(
                "TFL game: tracking: tick {} chose object {} (was {before}, interest {})",
                sim.tick,
                tracking.object,
                tracking.interest
            );
        }
        asks.extend(chosen);
    }
    for mandrake_tfl_sim::tracking::ObserverRequest::Center(p) in asks {
        centre_observer_on(observer, map, p);
    }
}

fn eye_behind((x, y): (i32, i32), facing: u16) -> (i32, i32) {
    let trig = mandrake_tfl_sim::trig::trig();
    let i = ((i32::from(facing.wrapping_add(0x8000)) + 0x20) >> 6 & 0x3ff) as usize;
    (
        ((i32::from(trig.cosine[i]) * 0x1880) >> 14) + x,
        ((i32::from(trig.sine[i]) * 0x1880) >> 14) + y,
    )
}

fn place_observer(observer: &mut ObserverController, map: &TflMap, (ex, ey): (i32, i32), f: u16) {
    let cells = (map.tfl_mesh.cell_width, map.tfl_mesh.cell_height);
    let per = crate::tfl_objects::WORLD_PER_RENDER_UNIT;
    let half = |c: u32| c as f32 * crate::tfl_objects::TFL_CELL_SIZE * 0.5;
    observer.origin[0] = half(cells.0) - ex as f32 / per;
    observer.origin[2] = ey as f32 / per - half(cells.1);
    observer.yaw = (f32::from(f) / 65536.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2)
        .rem_euclid(std::f32::consts::TAU);
    observer.velocity = [0.0; 5];
}

fn center_observer(
    observer: &mut mandrake_view::camera::ObserverController,
    map: &crate::tfl_map::TflMap,
    (x, y): (i32, i32),
) {
    let edge = |o: usize| {
        map.tag(b"mesh", map.mesh.mesh_id)
            .and_then(|d| {
                d.get(o..o + 2)
                    .map(|b| i32::from(i16::from_be_bytes([b[0], b[1]])))
            })
            .unwrap_or(0)
    };
    let (top, left, bottom, right) = (edge(0x74), edge(0x76), edge(0x78), edge(0x7a));
    let (w, h) = (
        map.tfl_mesh.cell_width as i32,
        map.tfl_mesh.cell_height as i32,
    );
    let facing =
        crate::tfl_hud::TflHudView::of_observer(observer.origin, observer.yaw, (w, h)).facing;
    let eye = |f: u16| eye_behind((x, y), f);
    let inside = |(ex, ey): (i32, i32)| {
        left <= ex >> 10 && ex >> 10 < w - right && top <= ey >> 10 && ey >> 10 < h - bottom
    };
    let mut turn = 0i32;
    let placed = loop {
        let f = facing.wrapping_add(turn as u16);
        if inside(eye(f)) {
            break (eye(f), f);
        }
        turn = if turn > 0 {
            -turn - 0x800
        } else if turn < 0 {
            -turn
        } else {
            -0x800
        };
        if turn < -0x8000 {
            let at = (
                x.clamp(left * 0x400, (w - right).max(left) * 0x400),
                y.clamp(top * 0x400, (h - bottom).max(top) * 0x400),
            );
            break (at, mandrake_tfl_sim::trig::arctangent(x - at.0, y - at.1));
        }
    };
    let (eye, f) = placed;
    place_observer(observer, map, eye, f);
    crate::camera_control::reset_height_and_pitch(observer);
    observer.zoom = observer.zoom.min(0.0);
}

pub(crate) fn place_camera_at(map: &TflMap, observer: &ObserverController, camera: &mut Camera) {
    let mesh = &map.mesh;
    let half_w = mesh.cell_width as f32 * TFL_CELL_SIZE * 0.5;
    let half_d = mesh.cell_height as f32 * TFL_CELL_SIZE * 0.5;
    observer.apply_to(camera, |x, z| {
        mesh.terrain_height_at((half_w - x) / TFL_CELL_SIZE, (z + half_d) / TFL_CELL_SIZE)
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn center_selection(
    sim: &mandrake_tfl_sim::TflSim,
    selected: &[usize],
    camera: &Camera,
    size: (u32, u32),
    control: &mut crate::camera_control::CameraControl,
    observer: &mut ObserverController,
    map: &TflMap,
    in_view_test: bool,
) -> bool {
    let cells = (map.tfl_mesh.cell_width, map.tfl_mesh.cell_height);
    let frame = crate::tfl_view::world_frame(cells);
    let state = control.observer(observer, frame);
    let Some(point) = tfl_input::selection_focus(
        sim,
        selected,
        state.zoom_factor(),
        in_view_test.then_some((camera, size, cells)),
    ) else {
        return false;
    };
    let angle = state.location().facing.wrapping_sub(0x8000);
    control.set_observer_focus(
        point,
        angle,
        crate::camera_control::view_slope(size.0, size.1.max(1)),
        sim.tick as i32,
        observer,
        frame,
    );
    println!(
        "TFL game: center selection on {point:?}: observer origin {:?} yaw {}",
        observer.origin, observer.yaw
    );
    true
}

fn feed_mouse(mouse: &mut Mouse, event: &WindowEvent) {
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            mouse.cursor = (position.x.floor() as i32, position.y.floor() as i32);
        }
        WindowEvent::MouseInput { state, button, .. } => {
            let left = match button {
                MouseButton::Left => true,
                MouseButton::Right => false,
                _ => return,
            };
            if *state == ElementState::Pressed {
                mouse.press(left);
            } else if left {
                mouse.left = false;
            } else {
                mouse.right = false;
            }
        }
        WindowEvent::ModifiersChanged(m) => {
            mouse.shift = m.state().shift_key();
            mouse.alt = m.state().alt_key();
        }
        _ => {}
    }
}

impl Mouse {
    fn action(&mut self, time: u64, size: (u32, u32)) -> tfl_input::MouseAction {
        let cursor = self.press_cursor.take().unwrap_or(self.cursor);
        let clamp = |v: i32, size: u32| v.clamp(0, size.saturating_sub(1) as i32) as i16;
        let poll = crate::selection_input::PollState {
            cursor: (clamp(cursor.0, size.0), clamp(cursor.1, size.1)),
            left: self.left || self.left_pressed,
            right: self.right || self.right_pressed,
            alt: self.alt,
            option: self.option,
            control: self.control,
            shift: self.shift,
            clear_selection: false,
            attack_ground: self.attack_ground,
        };
        let a = tfl_input::MouseAction {
            recall_preset: self.recall_preset,
            number: self.number,
            center_held: self.center_held,
            ..tfl_input::MouseAction::polled(&mut self.actions, time, poll)
        };
        self.left_pressed = false;
        self.right_pressed = false;
        a
    }

    fn press(&mut self, left: bool) {
        self.press_cursor = Some(self.cursor);
        if left {
            self.left = true;
            self.left_pressed = true;
        } else {
            self.right = true;
            self.right_pressed = true;
        }
    }
}

pub(crate) struct LiveGpu {
    dialog_overlay: OverlayRenderer,
    dialog: Option<OverlayImage>,
    revision: u64,
    scaler: InterfaceScaler,
}

fn selection_rules(args: &Args) -> crate::selection_input::SelectionRules {
    let interface = crate::interface_config::load(args.conf.as_deref(), false);
    let choice = interface.with_selection(args.selection).selection;
    println!("TFL game: selection by {choice:?}'s rules");
    choice.tfl_rules()
}

fn speed_mode(game_mode: i16) -> GameMode {
    match game_mode {
        0 => GameMode::Film,
        NETWORK_GAME_MODE => GameMode::Network,
        _ => GameMode::Solo,
    }
}

pub(crate) fn game_start_camera(map: &TflMap, args: &Args, replay: Option<&TflReplay>) -> Camera {
    let network_team = replay
        .and_then(TflReplay::sim)
        .filter(|s| s.game_mode == NETWORK_GAME_MODE)
        .and_then(|s| s.local_team().ok().flatten());
    match network_team {
        Some(team) => crate::tfl_view::network_start_camera(map, args, team),
        None => start_camera(map, args),
    }
}

impl GameView {
    fn modal(&self) -> bool {
        self.quit
            .as_ref()
            .is_some_and(|q| q.modeless_rect().is_none())
    }

    fn modeless_rect(&self) -> Option<[i32; 4]> {
        self.quit.as_ref().and_then(TflQuitDialog::modeless_rect)
    }

    fn new_with_tags(
        args: &Args,
        view: &ViewSettings,
        map: TflMap,
        replay: Option<TflReplay>,
        tfl_tags: Option<std::sync::Arc<mandrake_tag::TagArchive>>,
    ) -> Self {
        let hud = tfl_tags
            .or_else(|| {
                crate::tfl_game::chosen_game(args)
                    .1
                    .and_then(|dir| mandrake_tag::tag_files::open_tfl_tags(&dir).ok())
                    .map(std::sync::Arc::new)
            })
            .map(|archive| {
                let setup = match replay
                    .as_ref()
                    .and_then(TflReplay::sim)
                    .filter(|s| s.game_mode == NETWORK_GAME_MODE)
                {
                    Some(sim) => TflHudSetup::live_network(
                        &map,
                        sim.local_player(),
                        sim.local_team().ok().flatten(),
                    ),
                    None => TflHudSetup::of_map(&map),
                };
                let mut hud = TflHud::new(archive, setup);
                let interface = crate::interface_config::load(args.conf.as_deref(), false);
                hud.set_status_bar(interface.status_bar);
                hud.set_translucent_bars(view.translucent_bars);
                hud
            });
        let camera = game_start_camera(&map, args, replay.as_ref());
        let film_end = map.film.as_ref().map(|f| f.end_tick);
        let mut observer = ObserverController::from_camera(&camera);
        observer.constants = crate::start_camera::load_observer_physics_constants(&map.archive);
        let mode = replay
            .as_ref()
            .and_then(TflReplay::sim)
            .map_or(if map.film.is_some() { 0 } else { 2 }, |s| s.game_mode);
        let teams = crate::start_camera::StartTeams {
            mode,
            ..crate::start_camera::StartTeams::solo()
        };
        observer.min_zoom = crate::start_camera::start_min_zoom(&[], &teams, false);
        let mut camera_control = crate::camera_control::CameraControl::new_for_map(0, false);
        let mesh_data = map.tag(b"mesh", map.mesh.mesh_id);
        let speed = GameSpeed::for_map(
            speed_mode(mode),
            map.mesh.mesh_id,
            mesh_data.as_deref().unwrap_or_default(),
            args.game_speed,
        );
        match mesh_data {
            Some(data) => camera_control.set_mesh_edges(
                &data,
                (map.mesh.cell_width, map.mesh.cell_height),
                TFL_CELL_SIZE,
            ),
            None => info!("TFL: no mesh tag read; the camera has no edges"),
        }
        let look = crate::tfl_view::TflLook::new(args, &map, view, false);
        Self {
            map,
            smooth: view.smoothing,
            sound_rules: view.sound_rules,
            hover_frames: view.hover_frames,
            hover: HoverTracker::default(),
            look: Some(look),
            snapshots: tfl_menu::TflSnapshots::default(),
            snapshot_copy: None,
            restoring: None,
            film_ui: None,
            film_mouse: Mouse::default(),
            film_time: 0,
            camera_before: camera.current_state(),
            render_camera: camera.clone(),
            motion: MotionClock::default(),
            motion_tick: None,
            fraction: FRACTION_ONE,
            film_end,
            end: MapEnd::new(false),
            end_announced: false,
            camera,
            observer,
            replay,
            speed,
            paused: false,
            typed: tfl_input::TypedKeys::default(),
            presses: Vec::new(),
            obmo: ObmoCamera::default(),
            camera_control,
            scene: None,
            live: None,
            quit: None,
            quit_events: Vec::new(),
            net_pause: None,
            net_trade: None,
            modeless_press: false,
            quit_gpu: None,
            film_sound: None,
            hud,
            hud_gpu: None,
            world: 0,
        }
    }

    fn sniff_poll_with(&mut self, held: &mut Vec<u16>, key: i16) -> bool {
        let presses = std::mem::take(&mut self.presses);
        held.extend(presses.iter().filter_map(|(a, _)| a.engine_id()));
        held.sort_unstable();
        held.dedup();
        let action = mandrake_tfl_sim::sniffing::InputAction {
            key,
            ..mandrake_tfl_sim::sniffing::InputAction::with_actions(held)
        };
        let cleared = self
            .replay
            .as_mut()
            .is_some_and(|r| !r.sniff_action(&action));
        if cleared {
            if key != -1 || !presses.is_empty() {
                println!(
                    "TFL game: the sniffer cleared the poll (key {key}, presses {:?})",
                    presses
                        .iter()
                        .map(|(a, _)| a.config_name())
                        .collect::<Vec<_>>()
                );
            }
        } else {
            press_actions(self, presses);
        }
        cleared
    }

    fn step_observer(
        &mut self,
        axes: mandrake_view::camera::myth_observer::ObserverAxes,
        shift: bool,
    ) {
        self.observer.zoom_locked = shift;
        self.camera_before = self.camera.current_state();
        crate::camera_control::step_free_observer(
            &self.camera_control,
            &mut self.observer,
            axes,
            None,
        );
    }

    fn place_camera(&mut self) {
        place_camera_at(&self.map, &self.observer, &mut self.camera);
    }

    fn begin_frame(&mut self, since_tick: Duration, now_ms: u32) {
        let alpha = camera_alpha(since_tick);
        self.render_camera = if self.smooth && alpha < 1.0 {
            self.camera.interpolate(&self.camera_before, alpha)
        } else {
            self.camera.clone()
        };
        self.fraction = match self.replay.as_ref() {
            Some(r) if self.smooth => {
                let speed = self
                    .speed
                    .speed()
                    .min(mandrake_sim::game_speed::NORMAL_SPEED);
                if self.motion_tick != Some(r.tick()) {
                    self.motion_tick = Some(r.tick());
                    let late = since_tick.as_millis().min(u128::from(now_ms)) as u32;
                    self.motion.fraction(r.tick(), now_ms - late, speed);
                }
                object_history::frame_fraction(self.motion.fraction(r.tick(), now_ms, speed))
            }
            _ => FRACTION_ONE,
        };
    }
}

struct Menus {
    session: mandrake_session::Session,
    progress: Option<ProgressFile>,
    dialogs: TflDialogs,
    sound: MenuSound,
    events: Vec<UiEvent>,
    clock: Instant,
    modifiers: ModifiersState,
    g_held: bool,
    gpu: Option<MenuGpu>,
    audio: Option<AudioLink>,
    cutscene_hd: bool,
    movie: Option<MoviePlaying>,
    on_black: bool,
    pending: Option<TflLaunch>,
    net: crate::tfl_net::menu::TflMenuNet,
}

struct MoviePlaying {
    movie: TflMovie,
    film: Option<Cutscene>,
}

type MenuGpu = mandrake_render::MenuCanvasGpu;

impl Menus {
    fn new(
        game_dir: &std::path::Path,
        args: &Args,
        audio: Option<AudioLink>,
        tfl_tags: Option<std::sync::Arc<mandrake_tag::TagArchive>>,
    ) -> anyhow::Result<Self> {
        let (mut session, progress) = tfl_menu::open_session(
            game_dir,
            tfl_live::our_films_dir(),
            crate::user_dirs::config_file(tfl_menu::TFL_PROGRESS_FILE),
        )?;
        let archive = mandrake_tag::tag_files::open_tfl_tags(game_dir)
            .map_err(|e| anyhow::anyhow!("TFL tags in {}: {e}", game_dir.display()))?;
        println!(
            "TFL menus: the main menu (the splash and the intro aren't shown: TFL 1.5 goes \
             straight to the main menu on this install)"
        );
        let audio = audio.filter(|_| !args.mute);
        let audio_for_movies = audio.clone();
        let cutscene_hd = crate::interface_config::load(args.conf.as_deref(), false)
            .with_cutscene_hd(args.cutscene_hd.map(|hd| hd == crate::args::CutsceneHd::On))
            .cutscene_hd;
        let flags = crate::app::sound::sound_preference_flags(&args.archive);
        session.tfl_set_menu_music(flags & NO_MENU_MUSIC == 0);
        if let Some((level, code)) = std::env::var("M2_TFL_EPILOGUE")
            .ok()
            .and_then(|v| v.split_once(':').map(|(l, c)| (l.to_owned(), c.to_owned())))
            && let (Ok(level), Ok(code)) = (<[u8; 4]>::try_from(level.as_bytes()), code.parse())
        {
            println!(
                "TFL menus: M2_TFL_EPILOGUE: {} ended with code {code}",
                FourCC(level)
            );
            let difficulty = session.tfl().map_or(2, |t| t.prefs.difficulty);
            session.tfl_level_over(FourCC(level), difficulty, code, None);
        }
        if let Some(level) = &args.play {
            session
                .tfl_walk_to_level(level, args.difficulty)
                .map_err(anyhow::Error::msg)?;
        }
        tfl_menu::list_saves(&mut session, tfl_live::save::saves_dir().as_deref());
        let pending = tfl_menu::take_launch(&mut session, progress.as_ref());
        let dialogs = TflDialogs::new(archive);
        let net_tags = match tfl_tags {
            Some(tags) => tags,
            None => std::sync::Arc::new(
                mandrake_tag::tag_files::open_tfl_tags(game_dir)
                    .map_err(|e| anyhow::anyhow!("TFL tags in {}: {e}", game_dir.display()))?,
            ),
        };
        let net = crate::tfl_net::menu::TflMenuNet::new(
            net_tags,
            true,
            crate::settings::user_settings_path(),
        );
        Ok(Self {
            session,
            progress,
            dialogs,
            sound: MenuSound::tfl(
                audio,
                mandrake_tag::tag_files::open_tfl_tags(game_dir)
                    .map_err(|e| anyhow::anyhow!("TFL tags in {}: {e}", game_dir.display()))?,
                crate::clock::SystemTime::now()
                    .duration_since(crate::clock::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos()),
            ),
            events: Vec::new(),
            clock: Instant::now(),
            modifiers: ModifiersState::empty(),
            g_held: false,
            gpu: None,
            audio: audio_for_movies,
            cutscene_hd,
            movie: None,
            on_black: false,
            pending,
            net,
        })
    }

    fn pump(&mut self, game: Option<&mut crate::tfl_net::TflNetWindow>) -> Option<TflLaunch> {
        crate::tfl_net::menu::pump(
            &mut self.session,
            self.progress.as_ref(),
            &mut self.net,
            game,
        )
    }

    fn clear_held_input(&mut self) {
        self.modifiers = ModifiersState::empty();
        self.g_held = false;
    }

    fn movie_frame(&mut self, movie: TflMovie) -> Option<TflLaunch> {
        self.events.clear();
        let playing = match &mut self.movie {
            Some(p) if p.movie == movie => p,
            _ => {
                self.sound.stop();
                let film = self
                    .session
                    .tfl()
                    .and_then(|t| t.movie_data(movie.id))
                    .and_then(|data| {
                        Cutscene::from_data(
                            &format!("moov {}", movie.id),
                            data,
                            self.audio.clone(),
                            HdLookup::tfl_movie(movie.id, self.cutscene_hd),
                        )
                    });
                println!(
                    "TFL menus: movie {}{}{}",
                    movie.id,
                    if movie.skippable {
                        ""
                    } else {
                        " (can't be skipped)"
                    },
                    if film.is_none() { ": won't play" } else { "" }
                );
                self.movie.insert(MoviePlaying { movie, film })
            }
        };
        if let Some(film) = &mut playing.film {
            film.update(crate::clock::Instant::now());
            if !film.is_over() {
                return None;
            }
        }
        self.movie = None;
        println!("TFL menus: movie {} over", movie.id);
        self.session
            .game_event(mandrake_session::GameEvent::CutsceneOver);
        let launch = tfl_menu::take_launch(&mut self.session, self.progress.as_ref());
        self.on_black = launch.is_some();
        launch
    }

    fn now_ms(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    fn fade_sound(&mut self) {
        let now_ms = self.now_ms();
        self.sound.cue(
            mandrake_session::SoundCue::FadeOut {
                ticks: mandrake_dialogs::screens::campaign_end::SOUND_FADE_TICKS,
            },
            now_ms,
        );
    }

    fn frame(&mut self) -> Option<TflLaunch> {
        if let Some(launch) = self.pending.take() {
            return Some(launch);
        }
        if let mandrake_session::Screen::Cutscene(mandrake_session::Cutscene {
            movie: Some(movie),
            ..
        }) = self.session.screen()
        {
            return self.movie_frame(*movie);
        }
        self.net.step(&mut self.session);
        if let Some(launch) = self.pump(None) {
            return Some(launch);
        }
        let now_ms = self.now_ms();
        self.sound.update(now_ms);
        let input = MenuInput {
            events: &self.events,
            now_ms,
            shift: self.modifiers.shift_key(),
            g_held: self.g_held,
            music_busy: self.sound.busy(),
        };
        let out = self.dialogs.frame(&self.session, &input);
        self.events.clear();
        for cue in out.sounds {
            self.sound.cue(cue, now_ms);
        }
        for action in out.actions {
            self.session.act(action);
            if let Some(launch) = self.pump(None) {
                return Some(launch);
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct KeyIn<'a> {
    pressed: bool,
    repeat: bool,
    text: Option<&'a str>,
}

fn net_quit_dialog(host: bool) -> mandrake_session::Paused {
    mandrake_session::Paused {
        network: Some(mandrake_session::screen::NetPause {
            originator: host,
            rehost_items: false,
            can_rehost: false,
        }),
        ..mandrake_session::Paused::default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct SpriteCacheOverlay {
    generator: crate::sprite_interp_gen::TflSpriteCacheGen,
    screen: Option<mandrake_dialogs::tfl::TflProgressScreen>,
    gpu: Option<MenuGpu>,
    pending_game: Option<(TflMap, Option<TflReplay>)>,
}

pub(crate) struct TflApp {
    args: Args,
    view: ViewSettings,
    audio: Option<AudioLink>,
    depth: Option<((u32, u32), wgpu::TextureView)>,
    size: (u32, u32),
    exit: bool,
    load: LoadFlow<TflLaunch>,
    fade: Option<mandrake_render::ScreenFade>,
    cursor: Option<SoftwareCursor>,
    system_arrow: bool,
    mouse: Option<(i32, i32)>,
    ui: Option<UiScale>,
    game: Option<GameView>,
    report_backdrop: Option<GameView>,
    menus: Option<Menus>,
    controls: Controls,
    keys: HeldKeys,
    caps_lock_key: crate::interface_state::CapsLockKey,
    menu_filter: InterfaceFilter,
    cutscene_filter: InterfaceFilter,
    ui_filter: Option<UiFilter>,
    wheel: crate::controls::WheelState,
    last_tick: Instant,
    started: Instant,
    parts: Option<tfl_frame_parts::FrameParts>,
    tfl_tags: Option<std::sync::Arc<mandrake_tag::TagArchive>>,
    quit_cache: Option<TflQuitDialog>,
    held_actions: Vec<u16>,
    #[cfg(not(target_arch = "wasm32"))]
    sprite_gen: Option<SpriteCacheOverlay>,
}

impl TflApp {
    fn open_quit_dialog(
        cache: &mut Option<TflQuitDialog>,
        dir: Option<std::path::PathBuf>,
    ) -> Option<TflQuitDialog> {
        if let Some(quit) = cache.take() {
            return Some(quit);
        }
        dir.and_then(|d| mandrake_tag::tag_files::open_tfl_tags(&d).ok())
            .and_then(TflQuitDialog::open)
    }

    fn close_quit_dialog(cache: &mut Option<TflQuitDialog>, quit: &mut Option<TflQuitDialog>) {
        if let Some(mut dialog) = quit.take() {
            dialog.frame_screen(&mandrake_session::Screen::Playing { overlay: None }, &[], 0);
            *cache = Some(dialog);
        }
    }

    fn caps_lock(&self) -> Option<mandrake_view::selection::state::CapsLock> {
        self.caps_lock_key
            .is_down()
            .then(|| crate::interface_state::caps_lock_mode(self.controls.options.caps_lock))
    }

    fn new(
        args: &Args,
        setup: SceneSetup,
        map: TflMap,
        replay: Option<TflReplay>,
        controls: Controls,
    ) -> anyhow::Result<Self> {
        let game_dir = crate::tfl_game::chosen_game(args).1;
        let tfl_tags = game_dir
            .as_deref()
            .and_then(|dir| mandrake_tag::tag_files::open_tfl_tags(dir).ok())
            .map(std::sync::Arc::new);
        #[cfg(not(target_arch = "wasm32"))]
        let sprite_gen_init = if !cfg!(test)
            && crate::sprite_interp::installed()
            && setup.view.sprite_interpolation
            && let (Some(archive), Some(cache_dir)) =
                (tfl_tags.clone(), crate::sprite_interp::cache_dir("tfl"))
            && let Some(generator) =
                crate::sprite_interp_gen::TflSpriteCacheGen::start(archive.clone(), cache_dir)
        {
            let screen = mandrake_dialogs::tfl::TflProgressScreen::new(archive);
            Some((generator, screen))
        } else {
            None
        };
        #[cfg(not(target_arch = "wasm32"))]
        let defer_game = sprite_gen_init.is_some();
        #[cfg(target_arch = "wasm32")]
        let defer_game = false;
        let (menus, game, pending_game) = if opens_menus(args) {
            let dir = game_dir
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("TFL menus: no TFL data folder"))?;
            (
                Some(Menus::new(
                    dir,
                    args,
                    setup.audio.clone(),
                    tfl_tags.clone(),
                )?),
                None,
                None,
            )
        } else if defer_game {
            (None, None, Some((map, replay)))
        } else {
            (
                None,
                Some(GameView::new_with_tags(
                    args,
                    &setup.view,
                    map,
                    replay,
                    tfl_tags.clone(),
                )),
                None,
            )
        };
        #[cfg(not(target_arch = "wasm32"))]
        let sprite_gen = sprite_gen_init.map(|(generator, screen)| SpriteCacheOverlay {
            generator,
            screen,
            gpu: None,
            pending_game,
        });
        #[cfg(target_arch = "wasm32")]
        let _ = pending_game;
        let cursor = tfl_tags.as_deref().and_then(tfl_cursor_from_archive);
        if cursor.is_none() {
            info!("TFL: no cursor in the interface collection; the system's shows");
        }
        let system_arrow = shows_system_cursor(cursor.is_some());
        Ok(Self {
            args: args.clone(),
            audio: setup.audio,
            system_arrow,
            cursor: cursor.map(SoftwareCursor::new),
            mouse: None,
            ui: None,
            depth: None,
            size: (args.width, args.height),
            exit: false,
            load: LoadFlow::Idle,
            fade: None,
            game,
            report_backdrop: None,
            menus,
            controls,
            keys: HeldKeys::default(),
            caps_lock_key: Default::default(),
            menu_filter: setup.menu_filter,
            cutscene_filter: setup.cutscene_filter,
            ui_filter: setup.view.ui_filter,
            view: setup.view,
            wheel: Default::default(),
            last_tick: Instant::now(),
            started: Instant::now(),
            parts: args
                .frame_budget
                .map(|s| tfl_frame_parts::FrameParts::new(s, args.frame_budget_wait)),
            tfl_tags,
            quit_cache: None,
            held_actions: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            sprite_gen,
        })
    }

    fn start_network_game(&mut self, net: Box<crate::tfl_net::TflNetWindow>) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(overlay) = self.sprite_gen.take()
            && let Some((map, replay)) = overlay.pending_game
        {
            self.game = Some(GameView::new_with_tags(
                &self.args,
                &self.view,
                map,
                replay,
                self.tfl_tags.clone(),
            ));
        }
        let Some(view) = self.game.as_mut() else {
            tracing::error!("TFL network game: no map to play it on");
            return;
        };
        let p = &net.play.new_game.parameters;
        let level = (p.mesh, p.difficulty);
        println!(
            "TFL network game: {} (game type {}, difficulty {}), local player {} of {}",
            view.map.mesh_name,
            p.game_type,
            p.difficulty,
            net.play.new_game.local_player,
            net.play.new_game.players.len()
        );
        view.live = Some(Live::start(
            view,
            LiveGame::Net(net),
            &self.args,
            self.audio.clone(),
            level,
        ));
        view.film_end = None;
        self.keys = HeldKeys::default();
        self.last_tick = Instant::now();
    }

    fn window_title(&self) -> String {
        let Some(game) = &self.game else {
            return "Myth: The Fallen Lords".to_string();
        };
        let mut t = format!("Myth: The Fallen Lords - {}", game.map.mesh_name);
        if let Some(live) = &game.live {
            t += &match live.game.stop() {
                None => format!(" - tick {}", live.game.tick()),
                Some(s) => format!(" - stopped at tick {}: {s}", live.game.tick()),
            };
            if live.game.is_net() {
                t += " (network game)";
            } else if game.quit.is_some() || game.paused {
                t += " (paused)";
            }
            return t;
        }
        if let Some(r) = &game.replay {
            t += &format!(" - {}", r.status());
            if r.stop().is_none() {
                if game.paused || game.quit.is_some() {
                    t += " (paused)";
                } else if game.speed.speed() != mandrake_sim::game_speed::NORMAL_SPEED {
                    t += &format!(" ({}/256 speed)", game.speed.speed());
                }
            }
        }
        t
    }

    fn press(&mut self, key: crate::controls::MythKey) {
        let shift = self.keys.modifiers().shift;
        let Some(game) = self.game.as_mut() else {
            return;
        };
        game.typed.latch(key);
        if let Some(hud) = game.hud.as_mut() {
            display_chat_press(
                hud,
                &self.controls,
                &self.keys,
                key,
                self.caps_lock_key.is_down(),
            );
        }
        let actions = self.controls.actions_for_press(key, &self.keys);
        self.double_tap_event(key, &actions, false);
        let Some(game) = self.game.as_mut() else {
            return;
        };
        for action in actions {
            match action {
                ControlAction::Quicksave => {
                    if let (Some(_), Some(menus)) = (game.live.as_ref(), self.menus.as_mut()) {
                        menus.session.act(mandrake_session::Action::Quicksave);
                    }
                }
                a => {
                    if !press_now(game, a, shift) {
                        game.presses.push((a, shift));
                    }
                }
            }
        }
    }
}

impl TflApp {
    fn double_tap_event(
        &mut self,
        key: crate::controls::MythKey,
        actions: &[ControlAction],
        repeat: bool,
    ) {
        let Some(live) = self.game.as_mut().and_then(|g| g.live.as_mut()) else {
            return;
        };
        let mods = self.keys.modifiers();
        let held: Vec<u16> = ControlAction::ALL
            .iter()
            .filter(|&&a| self.controls.is_held(a, &self.keys))
            .filter_map(|a| a.engine_id())
            .collect();
        let words = crate::local_input::double_tap_words(
            (live.mouse.left, live.mouse.right),
            (mods.alt, mods.ctrl, mods.shift),
            repeat,
            &held,
        );
        live.selection.machine.presets.key_event(
            key,
            words,
            actions.iter().filter_map(|a| a.preset_index()),
        );
    }
}

fn display_chat_press(
    hud: &mut TflHud,
    controls: &crate::controls::Controls,
    keys: &HeldKeys,
    key: crate::controls::MythKey,
    caps_lock: bool,
) {
    let held = controls.is_held(ControlAction::DisplayChat, keys);
    let shift_up = key == crate::controls::MythKey(0x48) && keys.modifiers().shift;
    let caps = caps_lock && controls.options.caps_lock == crate::controls::CapsLock::ChatBacklog;
    hud.display_chat_action(held, shift_up, caps);
}

fn press_now(game: &mut GameView, action: ControlAction, shift: bool) -> bool {
    match action {
        ControlAction::GameSpeedDown | ControlAction::GameSpeedUp
            if game.live.as_ref().is_some_and(|l| l.game.is_net()) => {}
        ControlAction::GameSpeedDown | ControlAction::GameSpeedUp => {
            if let Some(index) = game
                .speed
                .adjust(action == ControlAction::GameSpeedUp, shift)
            {
                info!("Game speed: {}/256 (index {index})", game.speed.speed());
                if let Some(hud) = game.hud.as_mut() {
                    hud.show_speed_popup(index, game.speed.polls());
                }
            }
        }
        ControlAction::ToggleFpsDisplay => {
            if let Some(hud) = game.hud.as_mut() {
                hud.fps.toggle();
            }
        }
        ControlAction::ToggleScores => {
            if let Some(hud) = game.hud.as_mut() {
                hud.toggle_scores();
            }
        }
        _ => return false,
    }
    true
}

fn press_actions(game: &mut GameView, presses: Vec<(ControlAction, bool)>) {
    for (action, shift) in presses {
        {
            match action {
                ControlAction::PauseGame if game.live.as_ref().is_some_and(|l| l.game.is_net()) => {
                }
                ControlAction::PauseGame => {
                    game.paused = !game.paused;
                    info!("Game {}", if game.paused { "PAUSED" } else { "RESUMED" });
                }
                ControlAction::OverheadMap => match game.live.as_mut() {
                    Some(live) => live.mouse.overhead_map = true,
                    None => game.film_mouse.overhead_map = true,
                },
                a if a.preset_index().is_some() => {
                    if let (Some(live), Some(sim), Some(number)) = (
                        game.live.as_mut(),
                        game.replay.as_ref().and_then(TflReplay::sim),
                        a.preset_index(),
                    ) {
                        let busy = live.mouse.left || live.mouse.right;
                        if live.selection.number_key(sim, number, shift, busy) {
                            live.local_sprites.remove_local_sprites(
                                crate::move_marker::RemoveMode::ClearUnlessTimed,
                                live.game_time,
                            );
                        }
                        println!(
                            "TFL game: {} (formation {}, mode {:?})",
                            a.config_name(),
                            live.selection.formation,
                            live.selection.mode
                        );
                    }
                }
                a => {
                    if let (Some(live), Some(sim), Some(order)) = (
                        game.live.as_mut(),
                        game.replay.as_ref().and_then(TflReplay::sim),
                        tfl_input::keys::key_order(a, shift),
                    ) {
                        println!("TFL game: {} ({order:?})", a.config_name());
                        live.selection.key(sim, order);
                        for c in std::mem::take(&mut live.selection.orders) {
                            live.game.give(c);
                        }
                        if std::mem::take(&mut live.selection.clear_markers) {
                            live.local_sprites.remove_local_sprites(
                                crate::move_marker::RemoveMode::ClearUnlessTimed,
                                live.game_time,
                            );
                        }
                    }
                }
            }
        }
    }
}

impl TflApp {
    fn wheel_notch(&mut self, up: f32) {
        use crate::controls::WheelEffect;
        if up == 0.0 {
            return;
        }
        let shift = self.keys.modifiers().shift;
        match self
            .wheel
            .notch(up > 0.0, shift, self.controls.options.mouse_wheel)
        {
            WheelEffect::ZoomPreset(zoom_in) => {
                if let Some(game) = self.game.as_mut() {
                    game.observer.cycle_zoom_preset(zoom_in);
                }
            }
            WheelEffect::CyclePresets { up } => {
                if let Some(live) = self.game.as_mut().and_then(|g| g.live.as_mut()) {
                    live.selection.machine.presets.wheel_notch(up);
                }
            }
            WheelEffect::NotImplemented(_) => {
                debug!(
                    "TFL: mouse_wheel = \"{}\" does nothing in TFL",
                    self.controls.options.mouse_wheel.as_str()
                );
            }
            WheelEffect::None | WheelEffect::Hold => {}
        }
    }

    fn current_ui(&self) -> UiScale {
        self.ui.unwrap_or_else(|| {
            UiScale::new(
                crate::ui_scale::UiScaleMode::Native,
                self.size.0,
                self.size.1,
            )
        })
    }

    fn bar_mouse(&mut self) -> Option<BarMouse<'_>> {
        let ui = self.current_ui();
        let (Some(game), Some((x, y))) = (self.game.as_mut(), self.mouse) else {
            return None;
        };
        if game.modal() || game.hud.is_none() {
            return None;
        }
        let (lx, ly) = ui.to_logical(x as f32, y as f32);
        let logical = (ui.logical.0 as i32, ui.logical.1 as i32);
        Some((game, logical, (lx, ly)))
    }

    fn bar_click(&mut self) -> bool {
        let Some((game, logical, (lx, ly))) = self.bar_mouse() else {
            return false;
        };
        game.hud
            .as_mut()
            .is_some_and(|hud| hud.press(logical, lx, ly))
    }

    fn bar_moved(&mut self) {
        if let Some((game, logical, (lx, ly))) = self.bar_mouse()
            && let Some(hud) = game.hud.as_mut().filter(|h| h.tracking())
        {
            hud.moved(logical, lx, ly);
        }
    }

    fn bar_release(&mut self) -> bool {
        let Some((game, logical, (lx, ly))) = self.bar_mouse() else {
            return false;
        };
        let Some(hud) = game.hud.as_mut().filter(|h| h.tracking()) else {
            return false;
        };
        match hud.release(logical, lx, ly) {
            Some(BarButton::Pause) => {
                game.paused = !game.paused;
                info!("Game {}", if game.paused { "PAUSED" } else { "RESUMED" });
                println!("TFL status bar: pause ({lx}, {ly})");
            }
            Some(BarButton::Sketch) => {
                let on = hud.toggle_sketch();
                println!(
                    "TFL status bar: sketch ({lx}, {ly}): {}",
                    if on { "on" } else { "off" }
                );
            }
            Some(BarButton::EraseSketch) => {
                hud.erase_sketch();
                println!("TFL status bar: the sketch erased");
            }
            Some(BarButton::OverheadMap) => {
                hud.toggle_map();
                println!(
                    "TFL status bar: overhead map ({lx}, {ly}): {}",
                    if hud.map_shown() { "shown" } else { "hidden" }
                );
            }
            None => {}
        }
        true
    }

    fn load_clock(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn step_load(&mut self) {
        let now = self.load_clock();
        if matches!(self.load, LoadFlow::Idle) && self.game.is_none() {
            match self.menus.as_mut().and_then(Menus::frame) {
                Some(TflLaunch::Quit) => self.launch(TflLaunch::Quit),
                Some(launch)
                    if self
                        .menus
                        .as_mut()
                        .is_some_and(|m| std::mem::take(&mut m.on_black)) =>
                {
                    self.load = LoadFlow::Start(launch);
                }
                Some(launch) => {
                    if let Some(menus) = self.menus.as_mut() {
                        menus.fade_sound();
                    }
                    self.load = LoadFlow::begin(launch, now);
                }
                None => {}
            }
        } else if let Some(menus) = self.menus.as_mut() {
            menus.events.clear();
            let now_ms = menus.now_ms();
            menus.sound.update(now_ms);
        }
        match self.load.step(now) {
            LoadWork::Start(launch) => {
                self.game = None;
                self.launch(launch);
            }
            LoadWork::Loaded => {
                self.last_tick = Instant::now();
                let net = self
                    .game
                    .as_ref()
                    .and_then(|g| g.live.as_ref())
                    .is_some_and(|l| l.game.is_net());
                if let Some(menus) = self.menus.as_mut()
                    && self.game.is_some()
                    && !net
                {
                    menus
                        .session
                        .game_event(mandrake_session::GameEvent::Loaded);
                }
            }
            LoadWork::AttachGpu => {
                if self.game.as_ref().is_some_and(|g| g.restoring.is_some()) {
                    self.load.restore();
                    self.restore_step();
                }
            }
            LoadWork::Restore => self.restore_step(),
            LoadWork::None | LoadWork::ForMap => {}
        }
    }

    fn restore_step(&mut self) {
        let (Some(game), Some(menus)) = (self.game.as_mut(), self.menus.as_mut()) else {
            self.load.restored();
            return;
        };
        let Some(end) = game.restoring else {
            self.load.restored();
            return;
        };
        let replay = &mut game.replay;
        let step = game
            .live
            .as_mut()
            .and_then(|l| l.game.solo_mut())
            .map(|live| {
                live.restore_step(replay, end, RESTORE_FRAME_BUDGET)
                    .map(|over| (over, live.tick()))
            });
        match step {
            Some(Ok((over, tick))) => {
                menus
                    .session
                    .game_event(mandrake_session::GameEvent::Restoring { ticks: tick });
                if over {
                    println!("TFL game: restored to tick {tick}");
                    game.restoring = None;
                    game.motion_tick = None;
                    if let Some(mut old) = game.live.take() {
                        old.sound.stop();
                        let at = (old.level, old.difficulty);
                        let live = Live::start(game, old.game, &self.args, self.audio.clone(), at);
                        game.live = Some(live);
                    }
                    self.load.restored();
                }
                let now_ms = menus.now_ms();
                let input = MenuInput {
                    events: &[],
                    now_ms,
                    shift: false,
                    g_held: false,
                    music_busy: false,
                };
                let _ = menus.dialogs.frame(&menus.session, &input);
            }
            Some(Err(e)) => self.restore_failed(e),
            None => self.restore_failed("not a solo game".to_owned()),
        }
    }

    fn snapshot_read(&mut self, gpu: &Gpu<'_>, _size: (u32, u32)) {
        let (Some(game), Some(menus)) = (self.game.as_mut(), self.menus.as_mut()) else {
            return;
        };
        let Some(copy) = game.snapshot_copy.take() else {
            return;
        };
        let snapshot = copy
            .and_then(|c| c.read(gpu.device).ok())
            .map(|(size, pixels)| {
                crate::app::save_game::snapshot_of_frame(
                    size,
                    &pixels.to_rgba8(),
                    (0, 0, size.0, size.1),
                )
            });
        if snapshot.is_none() {
            tracing::warn!("TFL game: the frame couldn't be read for the snapshot");
        }
        let lines = tfl_menu::snapshot_drawn(
            &mut menus.session,
            &mut game.snapshots,
            snapshot,
            tfl_live::save::saves_dir().as_deref(),
        );
        if let Some(hud) = game.hud.as_mut() {
            for line in &lines {
                hud.print(line.as_bytes());
            }
        }
    }

    fn restore_failed(&mut self, error: String) {
        tracing::error!("TFL menus: the save didn't restore: {error}");
        self.load = LoadFlow::Idle;
        self.end_live_game();
        self.game = None;
        if let Some(menus) = self.menus.as_mut() {
            menus.clear_held_input();
            menus
                .session
                .game_event(mandrake_session::GameEvent::LoadFailed(error));
            menus.dialogs.reset();
        }
    }

    fn launch(&mut self, launch: TflLaunch) {
        if let TflLaunch::Net(mesh) = launch {
            self.launch_network_game(mesh);
            return;
        }
        let Some(menus) = self.menus.as_mut() else {
            return;
        };
        let Some(dir) = menus.session.tfl().map(|t| t.game_dir.clone()) else {
            return;
        };
        let loaded = match &launch {
            TflLaunch::Quit => {
                println!("TFL menus: Quit");
                self.exit = true;
                return;
            }
            TflLaunch::Net(_) => unreachable!("launched above"),
            TflLaunch::Level { .. } | TflLaunch::Restore(_) => {
                let started = match &launch {
                    TflLaunch::Level { level, difficulty } => {
                        println!("TFL menus: level {level} at difficulty {difficulty}");
                        load_tfl_map(&dir, None, Some(level.as_str().trim_end())).map(|mut map| {
                            let mut game = tfl_live::film::SoloGame::now(*level, *difficulty);
                            let script = tfl_live::LiveScript::from_env(*level);
                            if let Some(s) = &script {
                                game.seed = s.seed;
                                game.difficulty = s.difficulty;
                            }
                            let carry = tfl_menu::carry(&menus.session);
                            let (mut live, replay) =
                                tfl_live::TflLive::start_with(&dir, &mut map, game, &carry);
                            if let Some(s) = script {
                                live.follow(s);
                            }
                            (map, live, replay, None)
                        })
                    }
                    TflLaunch::Restore(path) => {
                        println!("TFL menus: restoring {}", path.display());
                        std::fs::read(path)
                            .ok()
                            .and_then(|b| tfl_live::save::TflSave::from_bytes(&b))
                            .and_then(|saved| saved.game().map(|g| (saved, g)))
                            .ok_or_else(|| anyhow::anyhow!("{} isn't a TFL save", path.display()))
                            .and_then(|(saved, game)| {
                                let mut map =
                                    load_tfl_map(&dir, None, Some(game.mesh.as_str().trim_end()))?;
                                let (live, replay, end) =
                                    tfl_live::TflLive::begin_restore(&dir, &mut map, &saved)
                                        .map_err(anyhow::Error::msg)?;
                                Ok((map, live, replay, Some((saved.carry(), end))))
                            })
                    }
                    _ => unreachable!("matched above"),
                };
                match started {
                    Ok((map, mut live, replay, restored)) => {
                        let (level, difficulty) = (live.game.mesh, live.game.difficulty);
                        let restoring = restored.map(|(carry, end)| {
                            menus.session.tfl_restored(Box::new(carry));
                            end
                        });
                        live.record_to(tfl_live::our_films_dir());
                        menus.sound.stop();
                        let mut view = GameView::new_with_tags(
                            &self.args,
                            &self.view,
                            map,
                            replay,
                            self.tfl_tags.clone(),
                        );
                        view.live = Some(Live::start(
                            &view,
                            LiveGame::Solo(live),
                            &self.args,
                            self.audio.clone(),
                            (level, difficulty),
                        ));
                        view.film_end = None;
                        view.restoring = restoring;
                        self.game = Some(view);
                        tfl_menu::list_saves(
                            &mut menus.session,
                            tfl_live::save::saves_dir().as_deref(),
                        );
                        self.keys = HeldKeys::default();
                        self.last_tick = Instant::now();
                        return;
                    }
                    Err(e) => Err(e),
                }
            }
            TflLaunch::Film(path) => {
                println!("TFL menus: replaying {}", path.display());
                load_tfl_map(&dir, Some(path), None).map(|m| {
                    let replay = TflReplay::new(&dir, &m);
                    (m, replay)
                })
            }
        };
        match loaded {
            Ok((map, replay)) => {
                menus.sound.stop();
                self.game = Some(GameView::new_with_tags(
                    &self.args,
                    &self.view,
                    map,
                    replay,
                    self.tfl_tags.clone(),
                ));
                self.keys = HeldKeys::default();
                self.last_tick = Instant::now();
            }
            Err(e) => {
                tracing::error!("TFL menus: {e}");
                menus
                    .session
                    .game_event(mandrake_session::GameEvent::LoadFailed(e.to_string()));
                menus.dialogs.reset();
            }
        }
    }

    fn launch_network_game(&mut self, mesh: FourCC) {
        let Some(menus) = self.menus.as_mut() else {
            return;
        };
        let Some(dir) = menus.session.tfl().map(|t| t.game_dir.clone()) else {
            return;
        };
        println!("TFL menus: the network game's map {mesh}");
        match menus.net.take_game(&self.args, &dir) {
            Ok((net, map, replay)) => {
                menus.sound.stop();
                menus
                    .session
                    .game_event(mandrake_session::GameEvent::Loaded);
                self.game = Some(GameView::new_with_tags(
                    &self.args,
                    &self.view,
                    map,
                    replay,
                    self.tfl_tags.clone(),
                ));
                self.start_network_game(net);
            }
            Err(e) => {
                tracing::error!("TFL menus: the network game's map: {e}");
                menus
                    .session
                    .game_event(mandrake_session::GameEvent::LoadFailed(e.to_string()));
                let _ = menus.pump(None);
                menus.dialogs.reset();
            }
        }
    }

    fn net_game_to_menus(&mut self, report: Option<mandrake_session::PostgameReport>) {
        let Some(menus) = self.menus.as_mut() else {
            return;
        };
        let Some(mut live) = self.game.as_mut().and_then(|g| g.live.take()) else {
            return;
        };
        live.sound.stop();
        let LiveGame::Net(net) = live.game else {
            return;
        };
        let game = self.game.take();
        menus.clear_held_input();
        menus.dialogs.reset();
        menus.net.hold(net);
        if let Some(report) = report {
            let next = game.as_ref().and_then(|g| {
                let header = g
                    .map
                    .tag(b"mesh", g.map.mesh.mesh_id)
                    .and_then(|d| tfl_menu::TflLevelHeader::from_mesh(&d))?;
                match report.end_code {
                    1 => header.next,
                    2 => header.alternative,
                    _ => None,
                }
            });
            if let Some(next) =
                next.filter(|&n| Some(n) != game.as_ref().map(|g| g.map.mesh.mesh_id))
            {
                println!("TFL network game: the next map is {next}");
                menus.session.game_event(mandrake_session::GameEvent::Net(
                    mandrake_session::net::NetEvent::NextMap(next),
                ));
            }
            menus
                .session
                .game_event(mandrake_session::GameEvent::MapOver {
                    report,
                    veterans: Vec::new(),
                });
        } else {
            println!("TFL network game: back to the menus");
        }
        let _ = menus.pump(None);
    }

    fn finish_game(&mut self) {
        let net = self
            .game
            .as_mut()
            .and_then(|g| g.live.as_mut())
            .and_then(|l| l.game.net_mut());
        let menus_net = self.menus.is_some();
        if let Some(net) = net {
            if net.reported && !menus_net {
                if net.may_leave() {
                    self.end_live_game();
                    self.exit = true;
                }
                return;
            }
            net.reported = true;
        }
        let is_net = self
            .game
            .as_ref()
            .and_then(|g| g.live.as_ref())
            .is_some_and(|l| l.game.is_net());
        let carry = self
            .game
            .as_mut()
            .filter(|g| g.live.is_some())
            .and_then(|g| g.replay.as_mut())
            .and_then(TflReplay::sim_mut)
            .map(|sim| {
                let stats = sim.epilogue().ok().flatten();
                (stats, tfl_live::TflCarry::at_map_end(sim))
            });
        let Some(game) = self.game.as_ref() else {
            return;
        };
        let tick = game.replay.as_ref().map_or(0, TflReplay::tick);
        let reason = game
            .replay
            .as_ref()
            .and_then(TflReplay::sim)
            .map_or(0, |s| s.map_actions.game.end_reason);
        let alternative = game
            .map
            .tag(b"mesh", game.map.mesh.mesh_id)
            .and_then(|d| tfl_menu::TflLevelHeader::from_mesh(&d))
            .is_some_and(|h| h.alternative.is_some());
        let word = match reason {
            0 => "no end",
            _ => crate::carnage_report::result_word(reason, alternative),
        };
        println!(
            "TFL game over at tick {tick}: {word} (end code {reason}) on {}",
            game.map.mesh_name
        );
        let live = game.live.as_ref().map(|l| (l.level, l.difficulty));
        let level = live.map_or(game.map.mesh.mesh_id, |l| l.0);
        let difficulty = live.map_or(0, |l| l.1);
        let sim = game.replay.as_ref().and_then(TflReplay::sim);
        let pages = sim
            .filter(|s| is_net || s.game_options & mandrake_common::game_options::COOPERATIVE == 0)
            .and_then(|s| tfl_menu::carnage_pages(&game.map.archive, s));
        let make_report = |film: bool, report_pages| mandrake_session::PostgameReport {
            map: mandrake_session::Text(game.map.mesh_name.clone().into_bytes()),
            result_word: mandrake_session::Text(word.as_bytes().to_vec()),
            end_tick: tick as i32,
            end_code: reason,
            film,
            pages: report_pages,
            ..mandrake_session::PostgameReport::default()
        };
        let film_report =
            (live.is_none() && pages.is_some()).then(|| make_report(true, pages.clone()));
        let (stats, carry) = match carry {
            Some((stats, carry)) => (stats, Some(carry)),
            None => (
                game.replay
                    .as_ref()
                    .and_then(TflReplay::sim)
                    .and_then(|s| s.epilogue().ok().flatten()),
                None,
            ),
        };
        if let Some(stats) = &stats {
            let page = tfl_menu::epilogue_page(&game.map.archive, level, difficulty, stats);
            let mut out = std::io::stdout().lock();
            let _ = crate::carnage_report::write_epilogue(&mut out, &page);
        }
        if is_net {
            if menus_net {
                let mut report = make_report(false, pages);
                if let (Some(stats), Some(menus)) = (&stats, self.menus.as_ref())
                    && let Some(tfl) = menus.session.tfl()
                {
                    report.epilogue = Some(tfl_menu::epilogue_page(
                        &tfl.archive,
                        level,
                        difficulty,
                        stats,
                    ));
                }
                self.net_game_to_menus(Some(report));
            }
            return;
        }
        let now = self.load_clock();
        match (live, self.menus.is_some()) {
            (Some((level, difficulty)), true) => {
                self.end_live_game();
                self.game = None;
                let menus = self.menus.as_mut().expect("the menus");
                menus.clear_held_input();
                menus.dialogs.reset();
                match carry {
                    Some(Ok(carry)) => menus.session.tfl_map_over(Box::new(carry)),
                    Some(Err(stop)) => {
                        tracing::error!("TFL game: the veterans weren't kept: {stop}")
                    }
                    None => {}
                }
                let page = stats
                    .as_ref()
                    .zip(menus.session.tfl())
                    .map(|(s, tfl)| tfl_menu::epilogue_page(&tfl.archive, level, difficulty, s));
                menus
                    .session
                    .tfl_level_over(level, difficulty, reason, page);
                if let Some(launch) =
                    tfl_menu::take_launch(&mut menus.session, menus.progress.as_ref())
                {
                    self.load = LoadFlow::begin(launch, now);
                }
            }
            (None, true) => {
                let mut game = self.game.take();
                if let Some(sound) = game.as_mut().and_then(|g| g.film_sound.as_mut()) {
                    sound.stop();
                }
                if film_report.is_some() {
                    println!("TFL film: the carnage report (pocr)");
                    self.report_backdrop = game.map(|mut g| {
                        g.hover_frames = false;
                        g.quit = None;
                        g
                    });
                }
                let menus = self.menus.as_mut().expect("the menus");
                menus.clear_held_input();
                menus.session.tfl_film_over(film_report);
                menus.dialogs.reset();
            }
            (_, false) => {
                self.end_live_game();
                self.exit = true;
            }
        }
    }

    fn leave_game(&mut self) {
        self.end_live_game();
        self.game = None;
        match self.menus.as_mut() {
            Some(menus) => {
                menus.clear_held_input();
                menus.session.tfl_back_to_menu();
                menus.dialogs.reset();
            }
            None => self.exit = true,
        }
    }

    fn end_live_game(&mut self) {
        let Some(live) = self.game.as_mut().and_then(|g| g.live.as_mut()) else {
            return;
        };
        live.sound.stop();
        let LiveGame::Solo(solo) = &mut live.game else {
            if let Some(LiveGame::Net(net)) = self
                .game
                .as_mut()
                .and_then(|g| g.live.take())
                .map(|l| l.game)
            {
                net.leave();
            }
            return;
        };
        match solo.write_film() {
            Some(Ok(path)) => println!(
                "TFL game: ended at tick {}; recorded as {}",
                solo.tick(),
                path.display()
            ),
            Some(Err(e)) => tracing::error!("TFL game: the film wasn't saved: {e}"),
            None => tracing::error!("TFL game: no folder for the film"),
        }
    }

    fn live_key(&mut self, code: KeyCode, event: KeyIn<'_>) -> bool {
        let is_net = self
            .game
            .as_ref()
            .and_then(|g| g.live.as_ref())
            .is_some_and(|l| l.game.is_net());
        let dir = match self.menus.as_ref() {
            Some(m) => m.session.tfl().map(|t| t.game_dir.clone()),
            None if is_net => crate::tfl_game::chosen_game(&self.args).1,
            None => None,
        };
        let loading = !matches!(self.load, LoadFlow::Idle);
        let held = self.keys.modifiers();
        let Some(game) = self.game.as_mut() else {
            return false;
        };
        if game.live.is_none() && self.menus.is_none() {
            return false;
        }
        let pressed = event.pressed;
        if game.quit.is_some() && !game.modal() {
            return code == KeyCode::Escape;
        }
        if game.quit.is_some() {
            let quicksave = pressed
                && !event.repeat
                && game.live.is_some()
                && crate::app::input::myth_key(code).is_some_and(|k| {
                    self.controls
                        .actions_for_press(k, &self.keys)
                        .contains(&ControlAction::Quicksave)
                });
            if quicksave && let Some(menus) = self.menus.as_mut() {
                menus.session.act(mandrake_session::Action::Quicksave);
                return true;
            }
            let (shift, alt) = game
                .live
                .as_ref()
                .map_or((held.shift, false), |l| (l.mouse.shift, l.mouse.alt));
            let mut modifiers = ModifiersState::empty();
            modifiers.set(ModifiersState::SHIFT, shift);
            modifiers.set(ModifiersState::ALT, alt);
            modifiers.set(ModifiersState::CONTROL, held.ctrl);
            crate::app::shell_menu::key_events(
                PhysicalKey::Code(code),
                pressed,
                event.repeat,
                event.text,
                modifiers,
                &mut game.quit_events,
            );
            return true;
        }
        if code == KeyCode::Escape && pressed && !event.repeat {
            if loading {
                return true;
            }
            game.quit = Self::open_quit_dialog(&mut self.quit_cache, dir);
            match (game.quit.is_some(), self.menus.as_mut()) {
                (true, Some(menus)) => {
                    menus.session.act(mandrake_session::Action::OpenPause);
                    if matches!(
                        menus.session.screen(),
                        mandrake_session::Screen::Playing {
                            overlay: Some(mandrake_session::Overlay::Paused(_))
                        }
                    ) {
                        println!(
                            "TFL game: {}the quit dialog open",
                            if is_net { "" } else { "paused, " }
                        );
                    } else {
                        tracing::warn!(
                            "TFL game: the session opened no quit dialog on {:?}",
                            menus.session.screen()
                        );
                        Self::close_quit_dialog(&mut self.quit_cache, &mut game.quit);
                    }
                }
                (true, None) if is_net => {
                    let host = game
                        .live
                        .as_ref()
                        .and_then(|l| l.game.net())
                        .is_some_and(|n| n.play.is_host());
                    game.net_pause = Some(net_quit_dialog(host));
                    println!("TFL network game: the quit dialog open");
                }
                _ => {
                    tracing::error!("TFL game: no quit dialog (bina qudi); back to the menus");
                    self.leave_game();
                }
            }
            return true;
        }
        false
    }

    fn text_key(&mut self, code: KeyCode, event: &winit::event::KeyEvent) -> bool {
        let Some(live) = self.game.as_mut().and_then(|g| g.live.as_mut()) else {
            return false;
        };
        if !live.selection.input.typing() {
            return false;
        }
        let key = crate::app::input::myth_key(code);
        if event.state == ElementState::Released {
            if let Some(key) = key {
                self.keys.release(key);
            }
            return true;
        }
        let ch = event
            .text
            .as_deref()
            .map(str::as_bytes)
            .filter(|b| b.len() == 1 && (0x20..0x7f).contains(&b[0]))
            .map(|b| b[0]);
        if let Some(key) = key {
            live.selection.text_key(key.0, ch);
        }
        true
    }

    fn live_mouse(&mut self, event: &WindowEvent) {
        let ui = self.current_ui();
        let Some(game) = self.game.as_mut() else {
            return;
        };
        let fit = quit_fit(game.quit.as_ref(), ui);
        let Some(live) = game.live.as_mut() else {
            if game.quit.is_some() {
                match event {
                    WindowEvent::CursorMoved { position, .. } => {
                        let (x, y) =
                            fit.to_canvas(position.x.floor() as i32, position.y.floor() as i32);
                        game.quit_events.push(UiEvent::PointerMoved { x, y });
                    }
                    WindowEvent::MouseInput { state, button, .. } => {
                        let left = match button {
                            MouseButton::Left => true,
                            MouseButton::Right => false,
                            _ => return,
                        };
                        let pressed = *state == ElementState::Pressed;
                        game.quit_events.push(UiEvent::Button { left, pressed });
                    }
                    _ => {}
                }
            } else if game.replay.is_some() {
                feed_mouse(&mut game.film_mouse, event);
            }
            return;
        };
        let modeless = game.quit.as_ref().and_then(TflQuitDialog::modeless_rect);
        let route = match modeless {
            Some(rect) => {
                let at = match event {
                    WindowEvent::CursorMoved { position, .. } => {
                        (position.x.floor() as i32, position.y.floor() as i32)
                    }
                    _ => live.mouse.cursor,
                };
                crate::app::input::modeless_route(
                    event,
                    fit.to_canvas(at.0, at.1),
                    rect,
                    &mut game.modeless_press,
                )
            }
            None if game.quit.is_some() => ModelessRoute::Dialog,
            None => {
                game.modeless_press = false;
                ModelessRoute::Game
            }
        };
        let dialog = route != ModelessRoute::Game;
        let to_game = route != ModelessRoute::Dialog;
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x.floor() as i32, position.y.floor() as i32);
                live.mouse.cursor = p;
                if dialog {
                    let (x, y) = fit.to_canvas(p.0, p.1);
                    game.quit_events.push(UiEvent::PointerMoved { x, y });
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let left = match button {
                    MouseButton::Left => true,
                    MouseButton::Right => false,
                    _ => return,
                };
                let pressed = *state == ElementState::Pressed;
                if dialog {
                    game.quit_events.push(UiEvent::Button { left, pressed });
                }
                if !to_game {
                    return;
                }
                if pressed {
                    live.mouse.press(left);
                } else if left {
                    live.mouse.left = false;
                } else {
                    live.mouse.right = false;
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                live.mouse.shift = m.state().shift_key();
                live.mouse.alt = m.state().alt_key();
            }
            _ => {}
        }
    }

    fn quit_frame(&mut self) {
        let now = self.started.elapsed().as_millis() as u64;
        let Some(menus) = self.menus.as_mut() else {
            self.net_quit_frame(now);
            return;
        };
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if let Some(net) = game.live.as_mut().and_then(|l| l.game.net_mut()) {
            for event in net.play.take_session_events() {
                menus
                    .session
                    .game_event(mandrake_session::GameEvent::Net(event));
            }
            let may_open = game.quit.is_none()
                && matches!(
                    menus.session.screen(),
                    mandrake_session::Screen::Playing { overlay: None }
                );
            for event in net.trading_events(game.replay.as_ref(), may_open) {
                menus
                    .session
                    .game_event(mandrake_session::GameEvent::Net(event));
            }
        }
        let trading = matches!(
            menus.session.screen(),
            mandrake_session::Screen::Playing {
                overlay: Some(mandrake_session::Overlay::UnitTrading(_))
            }
        );
        if trading && game.quit.is_none() {
            let dir = menus.session.tfl().map(|t| t.game_dir.clone());
            game.quit = Self::open_quit_dialog(&mut self.quit_cache, dir);
            println!("TFL network game: untr open");
        }
        if let Some(quit) = game.quit.as_mut() {
            let events = std::mem::take(&mut game.quit_events);
            for action in quit.frame(&menus.session, &events, now) {
                menus.session.act(action);
            }
        }
        let launch = match game.live.as_ref().and_then(|l| l.game.solo()) {
            Some(live) => tfl_menu::take_game_requests(
                &mut menus.session,
                menus.progress.as_ref(),
                live,
                tfl_live::save::saves_dir,
                &mut game.snapshots,
            ),
            None => {
                let net = game.live.as_mut().and_then(|l| l.game.net_mut());
                menus.pump(net)
            }
        };
        let is_net = game.live.as_ref().is_some_and(|l| l.game.is_net());
        if is_net
            && launch.is_none()
            && matches!(menus.session.screen(), mandrake_session::Screen::Net(_))
        {
            self.net_game_to_menus(None);
            return;
        }
        let screen = menus.session.screen();
        let playing = matches!(screen, mandrake_session::Screen::Playing { .. });
        let loading = matches!(
            screen,
            mandrake_session::Screen::Loading | mandrake_session::Screen::Restoring(_)
        );
        let open = matches!(
            screen,
            mandrake_session::Screen::Playing { overlay: Some(_) }
        );
        if launch.is_none() && (playing || loading) {
            if !open && game.quit.is_some() {
                println!("TFL game: resumed");
                Self::close_quit_dialog(&mut self.quit_cache, &mut game.quit);
                if let Some(live) = game.live.as_mut() {
                    live.mouse = Mouse {
                        cursor: live.mouse.cursor,
                        ..Mouse::default()
                    };
                }
            }
            return;
        }
        if self.load.drop_game(launch.is_some()) {
            println!("TFL game: Quit");
            self.leave_game();
        } else if let Some(launch) = launch {
            println!("TFL game: ended for {launch:?}");
            menus.dialogs.reset();
            Self::close_quit_dialog(&mut self.quit_cache, &mut game.quit);
            self.end_live_game();
            self.load = LoadFlow::begin(launch, self.load_clock());
        }
    }

    fn net_quit_frame(&mut self, now: u64) {
        let Some(game) = self.game.as_mut() else {
            return;
        };
        if Self::net_trade_frame(game, &self.args, &mut self.quit_cache, now) {
            return;
        }
        let (Some(quit), Some(paused)) = (game.quit.as_mut(), game.net_pause.as_ref()) else {
            return;
        };
        let events = std::mem::take(&mut game.quit_events);
        let screen = mandrake_session::Screen::Playing {
            overlay: Some(mandrake_session::Overlay::Paused(paused)),
        };
        let actions = quit.frame_screen(&screen, &events, now);
        for action in actions {
            match action {
                mandrake_session::Action::Resume => {
                    println!("TFL network game: the quit dialog closed");
                    Self::close_quit_dialog(&mut self.quit_cache, &mut game.quit);
                    game.net_pause = None;
                    if let Some(live) = game.live.as_mut() {
                        live.mouse = Mouse {
                            cursor: live.mouse.cursor,
                            ..Mouse::default()
                        };
                    }
                    return;
                }
                mandrake_session::Action::QuitToMenu => {
                    println!("TFL network game: Quit");
                    self.leave_game();
                    return;
                }
                other => tracing::debug!(?other, "TFL network game: not in its quit dialog"),
            }
        }
    }

    fn net_trade_frame(
        game: &mut GameView,
        args: &Args,
        quit_cache: &mut Option<TflQuitDialog>,
        now: u64,
    ) -> bool {
        use mandrake_session::net::NetEvent;
        let Some(net) = game.live.as_mut().and_then(|l| l.game.net_mut()) else {
            return false;
        };
        for event in net.trading_events(game.replay.as_ref(), game.quit.is_none()) {
            match event {
                NetEvent::UnitTrading(trade) => {
                    let dir = crate::tfl_game::chosen_game(args).1;
                    game.quit = Self::open_quit_dialog(quit_cache, dir);
                    if game.quit.is_some() {
                        game.net_trade = Some(*trade);
                        println!("TFL network game: untr open");
                    }
                }
                NetEvent::TradingOver => {
                    if let Some(trade) = game.net_trade.take() {
                        net.trade_units(&trade.counts);
                        Self::close_quit_dialog(quit_cache, &mut game.quit);
                        println!("TFL network game: untr closed at the planning time's end");
                    }
                }
                _ => {}
            }
        }
        let (Some(quit), Some(trade)) = (game.quit.as_mut(), game.net_trade.as_mut()) else {
            return false;
        };
        let events = std::mem::take(&mut game.quit_events);
        let actions = quit.frame_screen(
            &mandrake_session::Screen::Playing {
                overlay: Some(mandrake_session::Overlay::UnitTrading(trade)),
            },
            &events,
            now,
        );
        for action in actions {
            match action {
                mandrake_session::Action::SetTradeCounts(counts)
                    if counts.len() == trade.counts.len() =>
                {
                    trade.counts = counts;
                }
                mandrake_session::Action::Ok | mandrake_session::Action::Cancel => {
                    if action == mandrake_session::Action::Ok {
                        net.trade_units(&trade.counts);
                    }
                    println!("TFL network game: untr closed ({action:?})");
                    Self::close_quit_dialog(quit_cache, &mut game.quit);
                    game.net_trade = None;
                    return false;
                }
                other => tracing::debug!(?other, "TFL network game: not in untr"),
            }
        }
        true
    }

    fn toggle_overhead_map(
        mouse: &mut Mouse,
        hud: Option<&mut TflHud>,
        cleared: bool,
        locked: bool,
        quit_open: bool,
        label: &str,
    ) {
        if std::mem::take(&mut mouse.overhead_map)
            && !cleared
            && !locked
            && !quit_open
            && let Some(hud) = hud
        {
            hud.toggle_map();
            println!(
                "{label}: Overhead Map: {}",
                if hud.map_shown() { "shown" } else { "hidden" }
            );
        }
    }

    fn clear_action(action: tfl_input::MouseAction) -> tfl_input::MouseAction {
        tfl_input::MouseAction {
            time: action.time,
            cursor: action.cursor,
            sniffed_away: true,
            ..Default::default()
        }
    }

    fn pick_camera_and_fraction<'a>(
        smooth: bool,
        render_camera: &'a Camera,
        camera: &'a Camera,
        fraction: i32,
    ) -> (&'a Camera, i32) {
        if smooth {
            (render_camera, fraction)
        } else {
            (camera, FRACTION_ONE)
        }
    }

    fn center_on_selection(
        game: &mut GameView,
        selected: &[usize],
        size: (u32, u32),
        in_view_test: bool,
    ) {
        let Some(sim) = game.replay.as_ref().and_then(TflReplay::sim) else {
            return;
        };
        let camera = if game.smooth {
            &game.render_camera
        } else {
            &game.camera
        };
        if center_selection(
            sim,
            selected,
            camera,
            size,
            &mut game.camera_control,
            &mut game.observer,
            &game.map,
            in_view_test,
        ) {
            place_camera_at(&game.map, &game.observer, &mut game.camera);
            game.camera_before = game.camera.current_state();
            game.render_camera = game.camera.clone();
        }
    }

    fn sync_common_mouse(mouse: &mut Mouse, controls: &Controls, keys: &HeldKeys) {
        mouse.center_held = controls.is_held(ControlAction::CenterSelection, keys);
        mouse.option = keys.is_down(crate::controls::MythKey::OPTION);
        mouse.control = keys.is_down(crate::controls::MythKey::CONTROL);
    }

    fn live_tick_with(
        game: &mut GameView,
        size: (u32, u32),
        ui: UiScale,
        cleared: bool,
        locked: bool,
        live_history: Option<&crate::world_view::tfl::ObjectHistory>,
    ) {
        let quit_open = game.modal();
        let Some(live) = game.live.as_mut() else {
            return;
        };
        live.time += 1;
        let paused = game.paused || quit_open;
        if !paused {
            live.game_time = live.game_time.wrapping_add(1);
        }
        let mut action = live.mouse.action(live.time, size);
        Self::toggle_overhead_map(
            &mut live.mouse,
            game.hud.as_mut(),
            cleared,
            locked,
            quit_open,
            "TFL game",
        );
        if cleared {
            action = Self::clear_action(action);
        }
        let (lx, ly) = ui.to_logical(action.cursor.0 as f32, action.cursor.1 as f32);
        let poll = game
            .hud
            .as_mut()
            .filter(|_| !quit_open && !locked)
            .and_then(|hud| {
                hud.map_mouse(
                    (ui.logical.0 as i32, ui.logical.1 as i32),
                    action.left_held || action.left_pressed,
                    (lx, ly),
                    live.game_time,
                )
            });
        if let Some(hud) = game.hud.as_mut() {
            for (flags, points) in hud.take_sketch_commands() {
                live.game.give(crate::tfl_live::film::command(
                    SKETCH_COMMAND,
                    &sketch_payload(flags, &points),
                ));
            }
        }
        let on_map = match poll {
            Some(crate::tfl_minimap::MapPoll::Centre(p)) => Some(p),
            _ => None,
        };
        if let Some(point) = on_map {
            let o = &game.observer;
            let before = (o.origin[1], o.base_elevation, o.zoom);
            center_observer(&mut game.observer, &game.map, point);
            if action.left_pressed {
                let o = &game.observer;
                println!(
                    "TFL overhead map: press ({lx}, {ly}) -> world {point:?}: height, pitch, \
                     zoom {before:?} -> {:?}",
                    (o.origin[1], o.base_elevation, o.zoom)
                );
            }
        }
        if poll.is_none()
            && !quit_open
            && let Some(sim) = game.replay.as_ref().and_then(TflReplay::sim)
        {
            let (camera, fraction) = Self::pick_camera_and_fraction(
                game.smooth,
                &game.render_camera,
                &game.camera,
                game.fraction,
            );
            let world = drawn_world_with_live(game.replay.as_ref(), fraction, live_history);
            let view = tfl_input::View {
                camera,
                size,
                mesh: &game.map.mesh,
                world: &world,
                models: game.scene.as_ref().map(TflScene::model_pick),
            };
            if let Some(order) = live.selection.update(sim, &view, action) {
                println!(
                    "TFL game: tick {}: move {:?} to {:?} (flags {:#x}, formation {}, facing {:#06x})",
                    live.game.tick(),
                    order.monsters,
                    order.waypoints,
                    order.flags,
                    order.formation,
                    order.facing
                );
                let payload = tfl_live::film::movement_payload(
                    order.flags,
                    order.formation,
                    order.facing,
                    &order.monsters,
                    &order.waypoints,
                );
                live.game
                    .give(tfl_live::film::movement_command(0, 0, &payload));
            }
            if std::mem::take(&mut live.selection.clear_markers) {
                live.local_sprites.remove_local_sprites(
                    crate::move_marker::RemoveMode::ClearUnlessTimed,
                    live.game_time,
                );
            }
            if let Some(point) = live.selection.attack_marker.take() {
                crate::move_marker::attack_ground_marker_of(
                    &mut live.local_sprites,
                    live.marker_collection,
                    point,
                    live.game_time,
                );
            }
            if let Some(request) = live.selection.markers.take() {
                tfl_input::ground_markers(
                    &mut live.local_sprites,
                    sim,
                    live.marker_collection,
                    &request,
                    live.game_time,
                );
            }
            for c in std::mem::take(&mut live.selection.orders) {
                println!(
                    "TFL game: tick {}: order type {} {:02x?}",
                    live.game.tick(),
                    c.command_type.to_u8(),
                    c.raw_payload
                );
                live.game.give(c);
            }
        }
        let center = live.selection.center.take();
        let popup = live.selection.preset_popup.take();
        if let Some(sim) = game.replay.as_ref().and_then(TflReplay::sim)
            && let (Some(preset), Some(hud)) = (popup, game.hud.as_mut())
        {
            hud.show_preset_popup(
                &live
                    .selection
                    .strings
                    .insp
                    .first()
                    .cloned()
                    .unwrap_or_default(),
                sim,
                &live.selection.selected,
                preset,
                live_viewer(sim),
                game.speed.polls(),
            );
        }
        let center_selected = center
            .filter(|_| !locked && !quit_open)
            .map(|in_view_test| (in_view_test, live.selection.selected.clone()));
        if let Some((in_view_test, selected)) = center_selected {
            Self::center_on_selection(game, &selected, size, in_view_test);
        }
        let Some(live) = game.live.as_mut() else {
            return;
        };
        let asked = std::mem::take(&mut live.selection.voices);
        let voices = match game.replay.as_mut() {
            Some(r) if !asked.is_empty() => r.monsters_voices(&asked),
            _ => Vec::new(),
        };
        if let (Some(hud), Some(r)) = (game.hud.as_ref(), game.replay.as_mut()) {
            r.set_overhead_map_state(hud.map_state());
        }
        let mut stepped = 0;
        let mut voices = voices;
        match &mut live.game {
            LiveGame::Solo(solo) => {
                for _ in 0..game.speed.advance(1, paused) {
                    if !steps_on(game.replay.as_ref(), None) || !solo.step(&mut game.replay) {
                        break;
                    }
                    stepped += 1;
                }
            }
            LiveGame::Net(net) => {
                let frame = net.frame(&mut game.replay, 1);
                stepped = frame.stepped;
                net_news(net, frame, game.hud.as_mut(), &mut voices);
            }
        }
        play_sounds(
            &mut live.sound,
            &game.map,
            game.scene.as_mut(),
            &mut game.replay,
            &game.observer,
            stepped,
            voices,
            game.hud.as_mut(),
        );
        if let Some(sim) = game.replay.as_ref().and_then(TflReplay::sim) {
            live.selection.forget_removed(sim);
            if let Some(hud) = game.hud.as_mut() {
                hud.update_selection(sim, &live.selection.selected, live_viewer(sim));
            }
        }
        if let Some(hud) = game.hud.as_mut() {
            hud.set_prompts(
                &live.selection.prompts,
                &live.selection.input,
                live.time as i32,
            );
            for m in std::mem::take(&mut live.selection.messages) {
                hud.print(&m);
            }
        }
    }

    fn film_poll_with(
        game: &mut GameView,
        size: (u32, u32),
        ui: UiScale,
        rules: impl FnOnce() -> crate::selection_input::SelectionRules,
        cleared: bool,
        locked: bool,
        live_history: Option<&crate::world_view::tfl::ObjectHistory>,
    ) {
        let quit_open = game.quit.is_some();
        game.film_time += 1;
        let mut action = game.film_mouse.action(game.film_time, size);
        Self::toggle_overhead_map(
            &mut game.film_mouse,
            game.hud.as_mut(),
            cleared,
            locked,
            quit_open,
            "TFL film",
        );
        if cleared {
            action = Self::clear_action(action);
        }
        if quit_open || locked {
            return;
        }
        let (lx, ly) = ui.to_logical(action.cursor.0 as f32, action.cursor.1 as f32);
        let (centre, center) = {
            let Some(sim) = game.replay.as_ref().and_then(TflReplay::sim) else {
                return;
            };
            let (camera, fraction) = Self::pick_camera_and_fraction(
                game.smooth,
                &game.render_camera,
                &game.camera,
                game.fraction,
            );
            let world = drawn_world_with_live(game.replay.as_ref(), fraction, live_history);
            let view = tfl_input::View {
                camera,
                size,
                mesh: &game.map.mesh,
                world: &world,
                models: game.scene.as_ref().map(TflScene::model_pick),
            };
            let map = &game.map;
            let film = game.film_ui.get_or_insert_with(|| {
                tfl_input::FilmInterface::new(rules(), tfl_input::SelectionStrings::of_map(map))
            });
            let time = i32::try_from(game.film_time).unwrap_or(i32::MAX);
            let centre = film.poll(
                sim,
                &view,
                game.hud.as_mut(),
                action,
                (ui.logical.0 as i32, ui.logical.1 as i32),
                (lx, ly),
                time,
            );
            let center = film
                .selection
                .center
                .take()
                .map(|in_view_test| (in_view_test, film.selection.selected.clone()));
            (centre, center)
        };
        if let Some(point) = centre {
            center_observer(&mut game.observer, &game.map, point);
            if action.left_pressed {
                println!(
                    "TFL film: overhead map: press ({lx}, {ly}) -> world {point:?}: observer origin {:?}",
                    game.observer.origin
                );
            }
        } else if let Some((in_view_test, selected)) = center {
            Self::center_on_selection(game, &selected, size, in_view_test);
        }
        if action.left_pressed
            && let Some(film) = game.film_ui.as_ref()
            && !film.selection.selected.is_empty()
        {
            println!("TFL film: looking at {:?}", film.selection.selected);
        }
    }

    fn tick(&mut self) {
        self.quit_frame();
        let now = Instant::now();
        let mut ticks = 0;
        let held = !matches!(self.load, LoadFlow::Idle);
        if let Some(game) = self.game.as_mut() {
            self.last_tick = catch_up_from(self.last_tick, now);
            while !held && now.duration_since(self.last_tick) >= TICK {
                if game.live.is_some()
                    && let Some(sim) = game.replay.as_mut().and_then(TflReplay::sim_mut)
                {
                    let cells = (
                        game.map.tfl_mesh.cell_width as i32,
                        game.map.tfl_mesh.cell_height as i32,
                    );
                    game.obmo.consume(sim, &game.observer, cells);
                }
                let flags = ObmoCamera::flags(game.replay.as_ref(), game.live.is_some());
                let locked = crate::camera_control::user_camera_locked_by(flags);
                let edge = if locked || game.modal() {
                    Default::default()
                } else {
                    crate::controls::tfl::edge_scroll(
                        self.mouse.map(|(x, y)| (x as f32, y as f32)),
                        self.size,
                        &self.controls.options,
                        game.live
                            .as_ref()
                            .map_or(game.film_mouse.left, |l| l.mouse.left),
                    )
                };
                let edge_actions = crate::controls::tfl::edge_actions(&edge);
                let mut axes =
                    crate::controls::sum_axes(&[self.controls.keyboard_axes(&self.keys), edge]);
                let keys = &self.keys;
                let key = game.typed.poll(|k| keys.is_down(k));
                if key == 0x0c
                    && let Some(sim) = game.replay.as_mut().and_then(TflReplay::sim_mut)
                {
                    let on = sim.camera_control_flags & crate::camera_control::FLAG_TRACK == 0;
                    sim.camera_control_flags ^= crate::camera_control::FLAG_TRACK;
                    let list = game
                        .map
                        .tag(b"stli", mandrake_common::FourCC(*b"inct"))
                        .and_then(|d| {
                            mandrake_tag::tfl::TflStringList::from_tag(
                                mandrake_common::FourCC(*b"stli"),
                                1,
                                &d,
                            )
                            .ok()
                        })
                        .map(|l| l.strings)
                        .unwrap_or_default();
                    let get = |i: usize| list.get(i).cloned().unwrap_or_default();
                    let mut text = get(0);
                    text.push(b' ');
                    text.extend(get(if on { 1 } else { 2 }));
                    println!(
                        "TFL game: object tracking {} at tick {}: {:?}",
                        if on { "on" } else { "off" },
                        sim.tick,
                        String::from_utf8_lossy(&text)
                    );
                    if let Some(hud) = game.hud.as_mut() {
                        hud.print(&text);
                    }
                }
                let cleared = if !locked && !game.modal() {
                    self.held_actions.clear();
                    self.held_actions.extend(
                        ControlAction::ALL
                            .iter()
                            .filter(|&&a| {
                                a != ControlAction::OverheadMap
                                    && self.controls.is_held(a, &self.keys)
                            })
                            .chain(&edge_actions)
                            .filter_map(|a| a.engine_id()),
                    );
                    game.sniff_poll_with(&mut self.held_actions, key)
                } else {
                    let presses = std::mem::take(&mut game.presses);
                    press_actions(game, presses);
                    false
                };
                if cleared || locked {
                    axes = Default::default();
                }
                if locked {
                    let tick = game.replay.as_ref().map_or(0, TflReplay::tick);
                    let after = game
                        .obmo
                        .control(flags, &mut game.observer, &game.map, tick);
                    if after != flags
                        && let Some(sim) = game.replay.as_mut().and_then(TflReplay::sim_mut)
                    {
                        sim.camera_control_flags = after;
                    }
                    use crate::camera_control::{FLAG_MOVE, FLAG_TRACK};
                    if after & FLAG_TRACK != 0
                        && after & FLAG_MOVE == 0
                        && let Some(sim) = game.replay.as_ref().and_then(TflReplay::sim)
                    {
                        let speed = if game.paused {
                            0
                        } else {
                            i32::from(game.speed.speed())
                        };
                        game.obmo
                            .track_poll(sim, speed, &mut game.observer, &game.map, &mut axes);
                    }
                }
                let wheel = self.wheel.tick(self.controls.options.mouse_wheel);
                if !locked {
                    axes = crate::controls::sum_axes(&[axes, wheel]);
                }
                game.step_observer(axes, self.keys.modifiers().shift);
                if edge != Default::default() {
                    debug!(
                        "TFL edge scroll: forward {} left {} -> observer origin {:?} yaw {}",
                        edge.forward, edge.left, game.observer.origin, game.observer.yaw
                    );
                }
                game.place_camera();
                let before = game
                    .replay
                    .as_ref()
                    .filter(|_| game.smooth)
                    .map(|r| (r.tick(), crate::world_view::tfl::history(r)));
                let ui = self.ui.unwrap_or_else(|| {
                    UiScale::new(
                        crate::ui_scale::UiScaleMode::Native,
                        self.size.0,
                        self.size.1,
                    )
                });
                let live_history = before.as_ref().map(|(_, h)| h);
                if let Some(live) = game.live.as_mut() {
                    live.mouse.attack_ground = self
                        .controls
                        .is_held(ControlAction::AttackGround, &self.keys);
                    live.mouse.recall_preset = self
                        .controls
                        .is_held(ControlAction::RecallSetPreset, &self.keys);
                    live.mouse.number = ControlAction::PRESET_SELECTIONS
                        .iter()
                        .position(|&a| self.controls.is_held(a, &self.keys));
                    Self::sync_common_mouse(&mut live.mouse, &self.controls, &self.keys);
                    let options = &self.controls.options;
                    live.selection.alternate_recall = options.alternate_preset_recall;
                    live.selection.machine.left_drag_bands = options.left_click_doesnt_gesture;
                    live.selection.machine.right_click =
                        crate::app::input::right_click_mode(options.right_click);
                    Self::live_tick_with(game, self.size, ui, cleared, locked, live_history);
                    track(game);
                    if let Some(sim) = game.replay.as_mut().and_then(TflReplay::sim_mut) {
                        game.obmo.consume(
                            sim,
                            &game.observer,
                            (
                                game.map.tfl_mesh.cell_width as i32,
                                game.map.tfl_mesh.cell_height as i32,
                            ),
                        );
                    }
                } else if game.replay.is_some() {
                    Self::sync_common_mouse(&mut game.film_mouse, &self.controls, &self.keys);
                    Self::film_poll_with(
                        game,
                        self.size,
                        ui,
                        || selection_rules(&self.args),
                        cleared,
                        locked,
                        live_history,
                    );
                    let Some(r) = game.replay.as_mut() else {
                        continue;
                    };
                    if let Some(hud) = game.hud.as_ref() {
                        r.set_overhead_map_state(hud.map_state());
                    }
                    let mut stepped = 0;
                    let paused = game.paused || game.quit.is_some();
                    for _ in 0..game.speed.advance(1, paused) {
                        if !steps_on(Some(r), game.film_end) || !r.step() {
                            break;
                        }
                        stepped += 1;
                    }
                    track(game);
                    if let (Some(film), Some(sim)) = (
                        game.film_ui.as_mut(),
                        game.replay.as_ref().and_then(TflReplay::sim),
                    ) {
                        film.selection.forget_removed(sim);
                        if let Some(hud) = game.hud.as_mut() {
                            hud.update_selection(
                                sim,
                                &film.selection.selected,
                                crate::tfl_hud_selection::Viewer::FILM,
                            );
                        }
                    }
                    let Some(r) = game.replay.as_mut() else {
                        continue;
                    };
                    let sound = game.film_sound.get_or_insert_with(|| {
                        let sources = r
                            .sim()
                            .map(|s| s.sound_sources().to_vec())
                            .unwrap_or_default();
                        let link = self.audio.clone().filter(|_| !self.args.mute);
                        let flags = crate::app::sound::sound_preference_flags(&self.args.archive);
                        let sound = tfl_sound::TflLevelSound::open(
                            &game.map,
                            &sources,
                            link,
                            flags,
                            game.sound_rules,
                        );
                        println!("TFL film: sound: {}", sound.summary());
                        sound
                    });
                    play_sounds(
                        sound,
                        &game.map,
                        game.scene.as_mut(),
                        &mut game.replay,
                        &game.observer,
                        stepped,
                        Vec::new(),
                        game.hud.as_mut(),
                    );
                }
                if let (Some((tick, objects)), Some(r)) = (before, game.replay.as_mut())
                    && r.tick() != tick
                {
                    r.record_history(objects);
                }
                if let Some(hud) = game.hud.as_mut() {
                    let sim = match game.live.as_ref() {
                        Some(live) => game
                            .replay
                            .as_ref()
                            .and_then(TflReplay::sim)
                            .map(|s| (live.game.tick(), s)),
                        None => game
                            .replay
                            .as_ref()
                            .and_then(|r| r.sim().map(|s| (r.tick(), s))),
                    };
                    let view = sim.map_or_else(crate::world_view::WorldView::default, |(t, s)| {
                        crate::world_view::tfl::interface_view(s, t)
                    });
                    hud.update_owned(view);
                    if let Some((_, s)) = sim {
                        hud.update_scores(s);
                    }
                }
                self.last_tick += TICK;
                ticks += 1;
                game.world += 1;
            }
            if let Some(net) = game.live.as_mut().and_then(|l| l.game.net_mut()) {
                if ticks == 0 {
                    let frame = net.frame(&mut game.replay, 0);
                    let mut sounds = Vec::new();
                    net_news(net, frame, game.hud.as_mut(), &mut sounds);
                }
                if net.host_gone && self.menus.is_none() {
                    self.exit = true;
                }
            }
            game.place_camera();
            game.begin_frame(
                now.duration_since(self.last_tick),
                self.started.elapsed().as_millis() as u32,
            );
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn step_sprite_cache_gen(&mut self, frame: &mut SceneFrame<'_>) -> bool {
        let Some(overlay) = self.sprite_gen.as_mut() else {
            return false;
        };
        let step = overlay.generator.step(
            frame.device,
            frame.queue,
            std::time::Duration::from_millis(25),
        );
        match step {
            crate::sprite_interp_gen::StepResult::Working => {
                let (status, frac) = overlay.generator.status();
                if let Some(screen) = overlay.screen.as_mut() {
                    screen.update(&status, frac);
                    let menu_gpu = overlay.gpu.get_or_insert_with(|| {
                        MenuGpu::new(&InterfaceFrame {
                            device: frame.device,
                            queue: frame.queue,
                            encoder: frame.encoder,
                            target: frame.interface,
                            format: frame.formats.interface,
                            size: frame.size,
                        })
                    });
                    menu_gpu.draw(
                        &mut InterfaceFrame {
                            device: frame.device,
                            queue: frame.queue,
                            encoder: frame.encoder,
                            target: frame.interface,
                            format: frame.formats.interface,
                            size: frame.size,
                        },
                        screen.menu_canvas(),
                        self.menu_filter,
                    );
                }
                true
            }
            crate::sprite_interp_gen::StepResult::Done { generated, elapsed } => {
                println!(
                    "TFL: generated {generated} sprite interpolation cache files in {:.2}s",
                    elapsed.as_secs_f64()
                );
                let pending = self.sprite_gen.take().and_then(|o| o.pending_game);
                if let Some((map, replay)) = pending {
                    self.game = Some(GameView::new_with_tags(
                        &self.args,
                        &self.view,
                        map,
                        replay,
                        self.tfl_tags.clone(),
                    ));
                }
                self.last_tick = Instant::now();
                false
            }
            crate::sprite_interp_gen::StepResult::Failed(err) => {
                tracing::warn!(
                    "TFL: sprite interpolation cache generation failed ({err}); continuing with drawn frames"
                );
                let pending = self.sprite_gen.take().and_then(|o| o.pending_game);
                if let Some((map, replay)) = pending {
                    self.game = Some(GameView::new_with_tags(
                        &self.args,
                        &self.view,
                        map,
                        replay,
                        self.tfl_tags.clone(),
                    ));
                }
                self.last_tick = Instant::now();
                false
            }
        }
    }

    #[allow(unused_mut)]
    fn draw(&mut self, mut frame: SceneFrame<'_>) {
        self.size = frame.size;
        self.ui = Some(frame.ui);
        #[cfg(not(target_arch = "wasm32"))]
        if self.step_sprite_cache_gen(&mut frame) {
            return;
        }
        if let Some(p) = self.parts.as_mut() {
            if self.game.is_none() {
                p.restart();
            }
            p.begin();
            if p.waits() {
                let _ = frame.device.poll(wgpu::PollType::wait_indefinitely());
            }
            p.lap(Part::GpuWait);
            if p.done() {
                let what = match self.game.as_ref() {
                    Some(g) if g.live.is_some() => "live game",
                    Some(g) if g.replay.is_some() => "film",
                    _ => "map",
                };
                p.print(frame.size, what);
                self.parts = None;
                self.exit = true;
            }
        }
        if self
            .game
            .as_ref()
            .is_some_and(|g| g.end.finished(self.load_clock()))
        {
            self.finish_game();
        }
        self.step_load();
        let fade = self.load.alpha(self.load_clock());
        self.tick();
        let end_fade = match self.game.as_mut() {
            Some(g) => {
                let stage = end_stage(g.replay.as_ref(), g.film_end);
                if stage != EndStage::Playing && !g.end_announced {
                    g.end_announced = true;
                    println!(
                        "TFL game: the end at tick {}: {stage:?}",
                        g.replay.as_ref().map_or(0, TflReplay::tick)
                    );
                }
                g.end.update(stage, self.started.elapsed().as_secs_f64());
                if self.menus.is_some() && report_follows(g) {
                    0.0
                } else {
                    g.end.alpha()
                }
            }
            None => 0.0,
        };
        let fade = fade.max(end_fade);
        lap(&mut self.parts, Part::Tick);
        let size = frame.size;
        let gpu = Gpu {
            device: frame.device,
            queue: frame.queue,
            interface_format: frame.formats.interface,
            ui: frame.ui,
            ui_filter: frame.ui.filter(self.ui_filter).into(),
            menu_filter: self.menu_filter,
        };
        let encoder = frame.encoder;
        let interface = frame.interface;
        let mouse = self.mouse;
        let now_ms = self.started.elapsed().as_millis() as u32;
        let caps_lock = self.caps_lock();
        let restoring = self.game.as_ref().is_some_and(|g| g.restoring.is_some());
        self.snapshot_read(&gpu, size);
        let film_report = self
            .menus
            .as_ref()
            .is_some_and(|m| matches!(m.session.screen(), mandrake_session::Screen::FilmReport(_)));
        if !film_report || self.game.is_some() {
            self.report_backdrop = None;
        }
        let backdrop = self.report_backdrop.is_some();
        if backdrop {
            self.game = self.report_backdrop.take();
        }
        match (self.game.as_mut(), self.menus.as_mut()) {
            (_, Some(menus)) if restoring => {
                draw_menus(&gpu, menus, encoder, interface, size, self.cutscene_filter);
            }
            (Some(game), _) => {
                if self.depth.as_ref().is_none_or(|(s, _)| *s != size) {
                    let view = crate::render_passes::create_depth_view(gpu.device, size.0, size.1);
                    self.depth = Some((size, view));
                }
                let Some((_, depth)) = self.depth.as_ref() else {
                    return;
                };
                let scene = game.scene.get_or_insert_with(|| {
                    let mut scene = TflScene::new(
                        gpu.device,
                        gpu.queue,
                        &RenderPipelineConfig {
                            width: size.0,
                            height: size.1,
                            target_format: frame.formats.scene,
                            depth_format: Some(wgpu::TextureFormat::Depth32Float),
                        },
                        gpu.interface_format,
                        &game.map,
                        game.replay.as_ref(),
                    );
                    if let Some(look) = game.look.take() {
                        scene.set_look(look);
                    }
                    scene.set_frame_per_prepare();
                    scene
                });
                let sim = game.replay.as_ref().and_then(TflReplay::sim);
                let selection = game
                    .live
                    .as_ref()
                    .map(|l| &l.selection)
                    .or_else(|| game.film_ui.as_ref().map(|f| &f.selection));
                let framed_monsters = selection
                    .map(tfl_input::TflSelection::framed_monsters)
                    .unwrap_or_default();
                let hovered = match sim {
                    Some(_) if game.hover_frames => {
                        let pointer = mouse.filter(|_| {
                            game.quit
                                .as_ref()
                                .is_none_or(|q| q.modeless_rect().is_some())
                        });
                        let (camera, fraction) = if game.smooth {
                            (&game.render_camera, game.fraction)
                        } else {
                            (&game.camera, FRACTION_ONE)
                        };
                        let replay = game.replay.as_ref();
                        let mesh = &game.map.mesh;
                        game.hover.update(now_ms, pointer.is_some(), || {
                            let world = drawn_world(replay, fraction);
                            let view = tfl_input::View {
                                camera,
                                size,
                                mesh,
                                world: &world,
                                models: None,
                            };
                            let slot = tfl_input::monster_under(&view, pointer?)?;
                            Some(HoverPick {
                                id: slot as u32,
                                selected: framed_monsters.contains(&slot),
                                monster: true,
                            })
                        });
                        game.hover.hovered().map(|s| s as usize)
                    }
                    _ => None,
                };
                let ground_sprites = game
                    .live
                    .as_ref()
                    .map_or(&[][..], |l| l.local_sprites.drawn(l.game_time));
                scene.set_markers(
                    game.hud.as_ref().and_then(TflHud::interface_colors),
                    tfl_markers_with(
                        selection,
                        sim,
                        &framed_monsters,
                        ground_sprites,
                        hovered,
                        caps_lock,
                    ),
                );
                scene.set_glide(game.smooth);
                scene.prepare_changed(
                    gpu.device,
                    gpu.queue,
                    &game.map,
                    game.replay.as_ref(),
                    &game.render_camera,
                    game.world,
                    game.fraction,
                );
                if let Some(replay) = game.replay.as_mut() {
                    let (stains, marks, orders, starts) = scene.consumed();
                    replay.trim_consumed(stains, marks, orders, starts);
                }
                lap(&mut self.parts, Part::Prepare);
                scene.encode(
                    gpu.device,
                    gpu.queue,
                    encoder,
                    &WorldTargets {
                        color: frame.color,
                        interface,
                        depth,
                    },
                    size,
                    &game.render_camera,
                );
                if game.snapshots.wanted() && game.snapshot_copy.is_none() {
                    game.snapshot_copy = Some(crate::screenshot::Readback::encode(
                        gpu.device,
                        encoder,
                        frame.color.texture(),
                    ));
                }
                lap(&mut self.parts, Part::Encode);
                let now_ms = self.started.elapsed().as_millis() as u32;
                if let Some(hud) = game.hud.as_mut() {
                    hud.set_held_keys(&self.controls, &self.keys, self.caps_lock_key.is_down());
                    hud.update_speed_popup(game.speed.polls());
                }
                let outline = game
                    .hud
                    .as_ref()
                    .is_some_and(|h| h.map_state() != 0)
                    .then(|| scene.view_outline(&game.render_camera, size));
                draw_hud(
                    &gpu,
                    game,
                    encoder,
                    interface,
                    size,
                    now_ms,
                    &mut self.parts,
                    outline.as_ref(),
                );
                draw_quit(&gpu, game, encoder, interface, size);
                lap(&mut self.parts, Part::Live);
            }
            (None, Some(menus)) => {
                draw_menus(&gpu, menus, encoder, interface, size, self.cutscene_filter);
            }
            (None, None) => {}
        }
        if backdrop {
            self.report_backdrop = self.game.take();
            if let (Some(game), Some(menus)) = (self.report_backdrop.as_mut(), self.menus.as_mut())
            {
                let report_gpu = game
                    .quit_gpu
                    .get_or_insert_with(|| LiveGpu::new(gpu.device, gpu.interface_format));
                report_gpu.draw_report(
                    (gpu.device, gpu.queue),
                    &mut menus.dialogs,
                    encoder,
                    (interface, size),
                    (gpu.menu_filter, gpu.ui),
                );
            }
        }
        if fade > 0.0 {
            self.fade
                .get_or_insert_with(|| {
                    mandrake_render::ScreenFade::new(gpu.device, gpu.interface_format)
                })
                .encode(encoder, interface, fade);
        }
        let now = self.load_clock();
        self.load.frame_drawn(now);
        if let Some(g) = self.game.as_mut() {
            g.end.frame_presented(end_fade);
        }
        let movie = self.menus.as_ref().is_some_and(|m| m.movie.is_some());
        if let (Some(cursor), Some(at), false) = (self.cursor.as_mut(), self.mouse, movie) {
            cursor.draw(
                gpu.device,
                gpu.queue,
                gpu.interface_format,
                encoder,
                interface,
                &gpu.ui,
                at,
                gpu.ui_filter,
            );
        }
        lap(&mut self.parts, Part::Cursor);
    }

    fn menu_event(&mut self, event: &WindowEvent) {
        let ui = self.current_ui();
        let Some(menus) = self.menus.as_mut() else {
            return;
        };
        if let Some(playing) = menus.movie.as_mut() {
            if playing.movie.skippable
                && crate::app::cutscene::is_skip(event)
                && let Some(film) = playing.film.as_mut()
            {
                film.skip();
            }
            return;
        }
        let fit = if self.report_backdrop.is_some() {
            quit_dialog_fit(ui)
        } else {
            CanvasFit::new(self.size, CANVAS)
        };
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = fit.to_canvas(position.x.floor() as i32, position.y.floor() as i32);
                menus.events.push(UiEvent::PointerMoved { x, y });
            }
            WindowEvent::ModifiersChanged(m) => menus.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if event.physical_key == PhysicalKey::Code(KeyCode::KeyG) {
                    menus.g_held = pressed;
                }
                crate::app::shell_menu::key_events(
                    event.physical_key,
                    pressed,
                    event.repeat,
                    event.text.as_deref(),
                    menus.modifiers,
                    &mut menus.events,
                );
            }
            WindowEvent::Focused(false) => menus.clear_held_input(),
            other => menus.events.extend(crate::app::shell_menu::ui_event(other)),
        }
    }
}

type BarMouse<'a> = (&'a mut GameView, (i32, i32), (i32, i32));

#[allow(clippy::too_many_arguments)]
fn draw_hud(
    gpu: &Gpu<'_>,
    game: &mut GameView,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    size: (u32, u32),
    now_ms: u32,
    parts: &mut Option<tfl_frame_parts::FrameParts>,
    outline: Option<&crate::minimap::OutlineView>,
) {
    let paused = game.paused;
    let trading = game.modeless_rect();
    let Some(hud) = game.hud.as_mut() else {
        return;
    };
    hud.set_paused(paused);
    hud.set_trading(trading.map(|r| untr_drop(r, gpu.ui)));
    let now = match game.live.as_ref() {
        Some(live) => u64::from(live.game.tick()),
        None => game.replay.as_ref().map_or(0, |r| u64::from(r.tick())),
    };
    let ui = gpu.ui;
    let logical = (ui.logical.0 as i32, ui.logical.1 as i32);
    hud.fps.frame_presented(now_ms);
    let HudGpu {
        overlay,
        images,
        scaler,
        fps,
        minimap,
        minimap_revision,
    } = game.hud_gpu.get_or_insert_with(|| HudGpu {
        overlay: OverlayRenderer::new(gpu.device, gpu.interface_format),
        images: Vec::new(),
        scaler: InterfaceScaler::new(gpu.device, gpu.interface_format),
        fps: crate::hud::FpsOverlay::default(),
        minimap: Default::default(),
        minimap_revision: None,
    });
    let fps_frame = hud.view_frame(logical);
    let mut fps_buf = [0u8; 32];
    let fps_text = hud.fps.write_text(&mut fps_buf);
    fps.update(
        overlay,
        gpu.device,
        gpu.queue,
        fps_text,
        fps_frame,
        |text| hud.fps_image(logical, text),
    );
    let map_frame = hud
        .map_frame(logical, outline)
        .map(|(frame, map, revision)| {
            if *minimap_revision != Some(revision) {
                minimap.update(overlay, gpu.device, gpu.queue, map);
                *minimap_revision = Some(revision);
            }
            frame
        });
    let layers = hud.layers(logical, now);
    lap(parts, Part::HudLayers);
    images.resize_with(layers.len().max(images.len()), Default::default);
    for (slot, l) in images.iter_mut().zip(&layers) {
        slot.sync(
            overlay,
            gpu.device,
            gpu.queue,
            l.revision,
            (l.image.width as u32, l.image.height as u32),
            || l.rgba(),
        );
    }
    let map_draws = map_frame
        .as_ref()
        .map(|f| minimap.draws(f))
        .unwrap_or_default();
    let map_box = map_draws.iter().map(|d| d.dst).reduce(|a, b| {
        let (x0, y0) = (a[0].min(b[0]), a[1].min(b[1]));
        let (x1, y1) = (
            (a[0] + a[2]).max(b[0] + b[2]),
            (a[1] + a[3]).max(b[1] + b[3]),
        );
        [x0, y0, x1 - x0, y1 - y0]
    });
    let map_count = map_draws.len();
    let draws: Vec<_> = map_draws
        .into_iter()
        .chain(images.iter().zip(&layers).filter_map(|(slot, l)| {
            let image = slot.image()?;
            Some(OverlayDraw::at(l.layer, image, l.x, l.y, l.alpha))
        }))
        .chain(fps.draw())
        .collect();
    if !ui.is_scaled() {
        overlay.encode(gpu.device, gpu.queue, encoder, target, size, &draws);
    } else {
        let layer = scaler.layer_view(gpu.device, ui.logical);
        overlay.encode_cleared(gpu.device, gpu.queue, encoder, layer, ui.logical, &draws);
        let content: Vec<[i32; 4]> = map_box
            .into_iter()
            .chain(draws[map_count..].iter().map(|d| d.dst))
            .collect();
        scaler.composite_content(gpu.queue, encoder, target, size, gpu.ui_filter, &content);
    }
    lap(parts, Part::HudUpload);
}

fn lap(parts: &mut Option<tfl_frame_parts::FrameParts>, part: Part) {
    if let Some(p) = parts.as_mut() {
        p.lap(part);
    }
}

fn draw_menus(
    gpu: &Gpu<'_>,
    menus: &mut Menus,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    size: (u32, u32),
    cutscene_filter: InterfaceFilter,
) {
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("TFL Menus Clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    if let Some(playing) = menus.movie.as_mut() {
        if let Some(film) = playing.film.as_mut() {
            film.draw(
                gpu.device,
                gpu.queue,
                encoder,
                target,
                gpu.interface_format,
                size,
                cutscene_filter,
            );
        }
        return;
    }
    let menu_gpu = menus.gpu.get_or_insert_with(|| {
        MenuGpu::new(&InterfaceFrame {
            device: gpu.device,
            queue: gpu.queue,
            encoder,
            target,
            format: gpu.interface_format,
            size,
        })
    });
    menu_gpu.draw(
        &mut InterfaceFrame {
            device: gpu.device,
            queue: gpu.queue,
            encoder,
            target,
            format: gpu.interface_format,
            size,
        },
        menus.dialogs.menu_canvas(),
        gpu.menu_filter,
    );
}

impl GameScene for TflApp {
    fn opening_title(&self) -> Option<String> {
        Some(self.title())
    }

    fn finish(&mut self) {
        self.end_live_game();
    }

    fn hides_system_cursor(&self) -> bool {
        !self.system_arrow
    }

    fn on_event(
        &mut self,
        host: &mut ShellHost,
        event_loop: &winit::event_loop::ActiveEventLoop,
        event: WindowEvent,
    ) {
        host.scene_event(self, event_loop, event);
    }

    fn on_wait(&mut self, host: &mut ShellHost, event_loop: &winit::event_loop::ActiveEventLoop) {
        host.scene_wait(self, event_loop);
        let net = self
            .game
            .as_ref()
            .and_then(|g| g.live.as_ref())
            .is_some_and(|l| l.game.is_net());
        if net && host.hidden() {
            self.tick();
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                crate::clock::Instant::now() + TICK / 4,
            ));
        }
    }
}

impl FrameScene for TflApp {
    fn title(&self) -> String {
        self.window_title()
    }

    fn window_event(&mut self, event: &WindowEvent, size: (u32, u32)) {
        self.size = size;
        if let (WindowEvent::Focused(focused), Some(live)) = (
            event,
            self.game
                .as_ref()
                .and_then(|g| g.live.as_ref())
                .filter(|l| l.game.is_net()),
        ) {
            info!(
                "net: the window {} the focus at tick {}",
                if *focused { "has" } else { "lost" },
                live.game.tick()
            );
        }
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = Some((position.x.floor() as i32, position.y.floor() as i32));
                self.bar_moved();
            }
            WindowEvent::CursorLeft { .. } => self.mouse = None,
            WindowEvent::Focused(false) => self.mouse = None,
            _ => {}
        }
        if self.game.is_none() {
            self.menu_event(event);
        }
        match event {
            WindowEvent::KeyboardInput { event, .. } if self.game.is_some() => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let key = KeyIn {
                    pressed: event.state == ElementState::Pressed,
                    repeat: event.repeat,
                    text: event.text.as_deref(),
                };
                if self.live_key(code, key) {
                    return;
                }
                if self.text_key(code, event) {
                    return;
                }
                if code == KeyCode::F9 && event.state == ElementState::Pressed && !event.repeat {
                    let on = crate::sprite_interp::toggle();
                    println!("TFL: sprite in-betweens {}", if on { "on" } else { "off" });
                    if let Some(game) = self.game.as_mut() {
                        game.world += 1;
                    }
                    return;
                }
                if code == KeyCode::Escape && event.state == ElementState::Pressed {
                    if self.menus.is_some() {
                        self.leave_game();
                    } else {
                        self.exit = true;
                    }
                    return;
                }
                if let Some(key) = crate::app::input::myth_key(code) {
                    match event.state {
                        ElementState::Pressed if !event.repeat => {
                            if key.0 == mandrake_view::selection::state::CAPS_LOCK_KEY as u8 {
                                self.caps_lock_key.press();
                            }
                            self.keys.press(key);
                            self.press(key);
                        }
                        ElementState::Pressed => {
                            self.keys.press(key);
                            if self.game.as_ref().is_some_and(|g| g.live.is_some()) {
                                let actions = self.controls.actions_for_press(key, &self.keys);
                                self.double_tap_event(key, &actions, true);
                            }
                        }
                        ElementState::Released => self.keys.release(key),
                    }
                }
            }
            WindowEvent::Focused(false) => {
                self.keys = HeldKeys::default();
                self.wheel.clear();
                if let Some(game) = self.game.as_mut() {
                    game.observer.stop();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } if self.bar_click() => {}
            WindowEvent::MouseInput {
                state: ElementState::Released,
                ..
            } if self.bar_release() => {}
            WindowEvent::CursorMoved { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::ModifiersChanged(_)
                if self.game.as_ref().is_some_and(|g| {
                    g.live.is_some() || g.quit.is_some() || g.replay.is_some()
                }) =>
            {
                self.live_mouse(event);
            }
            WindowEvent::MouseWheel { delta, .. } if self.game.is_some() => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                self.wheel_notch(up);
            }
            _ => {}
        }
    }

    fn frame(&mut self, frame: SceneFrame<'_>) {
        #[cfg(target_arch = "wasm32")]
        {
            static FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if n.is_power_of_two() || n.is_multiple_of(1800) {
                info!(
                    "web: TFL frame {n}, menus {}, game {}",
                    self.menus.is_some(),
                    self.game.is_some()
                );
            }
        }
        self.draw(frame);
    }

    fn wants_exit(&self) -> bool {
        self.exit
    }

    fn submitted(&mut self, submit_ms: f32, present_ms: f32) {
        if let Some(p) = self.parts.as_mut() {
            p.add(Part::Submit, submit_ms);
            p.add(Part::Present, present_ms);
        }
    }

    fn shot_info(&self, size: (u32, u32)) -> crate::screenshot::ShotInfo {
        let game = self.game.as_ref();
        let tick = game.map_or(0, |g| match g.live.as_ref() {
            Some(live) => live.game.tick(),
            None => g.replay.as_ref().map_or(0, TflReplay::tick),
        });
        let o = game.map(|g| &g.observer);
        let camera = game.map(|g| {
            if g.smooth {
                &g.render_camera
            } else {
                &g.camera
            }
        });
        let hud = game.and_then(|g| g.hud.as_ref());
        let ui_scale = self.ui.and_then(|ui| match ui.zoom {
            Some(percent) => Some(percent.to_string()),
            None if ui.logical != ui.window => Some("768p".to_owned()),
            None => None,
        });
        crate::screenshot::ShotInfo {
            film: self.args.replay.clone(),
            mesh: game.map_or_else(|| "tfl-menus".to_owned(), |g| g.map.mesh_name.clone()),
            mesh_id: game.map_or_else(String::new, |g| g.map.mesh.mesh_id.to_string()),
            difficulty: game
                .and_then(|g| g.replay.as_ref())
                .and_then(TflReplay::sim)
                .map(|s| s.difficulty),
            tick,
            subtitle: hud.and_then(|h| crate::screenshot::newest_subtitle(h.console_log())),
            size,
            observer: o.map_or([0.0; 6], |o| {
                [
                    o.origin[0],
                    o.origin[1],
                    o.origin[2],
                    o.yaw,
                    o.base_elevation,
                    o.zoom,
                ]
            }),
            view_camera: camera.map_or([0.0; 6], |c| {
                [
                    c.target[0],
                    c.target[1],
                    c.target[2],
                    c.distance,
                    c.yaw,
                    c.pitch,
                ]
            }),
            game_speed: game.map_or(256, |g| g.speed.speed()),
            paused: game.is_some_and(|g| g.paused),
            selection: game
                .and_then(|g| g.live.as_ref())
                .map_or_else(Vec::new, |l| {
                    l.selection.selected.iter().map(|&s| s as u32).collect()
                }),
            fraction: game.map_or(0, |g| g.fraction),
            sampling: clap::ValueEnum::to_possible_value(&self.view.sampling)
                .map_or_else(String::new, |v| v.get_name().to_owned()),
            fog: if self.view.extras.fog {
                crate::args::Fog::On
            } else {
                crate::args::Fog::Off
            },
            fog_seconds: 0.0,
            reflection_soft_edge: self.view.reflection_soft_edge,
            hover: game
                .filter(|g| g.hover.hovered().is_some())
                .and(self.mouse)
                .map(|(x, y)| (x as f32, y as f32)),
            caps_lock: self.caps_lock().is_some(),
            overhead_map: hud.is_some_and(TflHud::map_shown),
            ui_scale,
            argv: std::env::args().collect(),
            cwd: std::env::current_dir().ok(),
        }
    }

    fn print_line(&mut self, line: &str) {
        if let Some(hud) = self.game.as_mut().and_then(|g| g.hud.as_mut()) {
            hud.print(line.as_bytes());
        }
    }
}

const fn shows_system_cursor(drawn: bool) -> bool {
    !drawn
}

pub(crate) fn tfl_markers(
    selection: Option<&tfl_input::TflSelection>,
    sim: Option<&mandrake_tfl_sim::TflSim>,
    ground: &[LocalSprite],
    hovered: Option<usize>,
    caps_lock: Option<mandrake_view::selection::state::CapsLock>,
) -> TflMarkers {
    let monsters = selection
        .map(tfl_input::TflSelection::framed_monsters)
        .unwrap_or_default();
    tfl_markers_with(selection, sim, &monsters, ground, hovered, caps_lock)
}

fn tfl_markers_with(
    selection: Option<&tfl_input::TflSelection>,
    sim: Option<&mandrake_tfl_sim::TflSim>,
    monsters: &[usize],
    ground: &[LocalSprite],
    hovered: Option<usize>,
    caps_lock: Option<mandrake_view::selection::state::CapsLock>,
) -> TflMarkers {
    let mut framed = Vec::new();
    if let Some(sim) = sim {
        for &slot in monsters {
            if let Some(m) = sim.monsters.get(slot) {
                framed.push(TflFrame {
                    object: m.object,
                    monster: true,
                    hovered_only: false,
                    frame: true,
                });
            }
        }
        if let Some(object) = selection.and_then(tfl_input::TflSelection::framed_object)
            && sim.objects.get(object).is_some_and(|o| o.identifier != 0)
        {
            framed.push(TflFrame {
                object,
                monster: false,
                hovered_only: false,
                frame: true,
            });
        }
        if let Some(slot) = hovered.filter(|s| !monsters.contains(s) && tfl_input::live(sim, *s))
            && let Some(m) = sim.monsters.get(slot)
        {
            framed.push(TflFrame {
                object: m.object,
                monster: true,
                hovered_only: true,
                frame: true,
            });
        }
        if let Some(mode) = caps_lock {
            let hovered = hovered.filter(|s| tfl_input::live(sim, *s));
            let mut is_framed = vec![false; sim.monsters.len()];
            for &slot in monsters {
                if let Some(flag) = is_framed.get_mut(slot) {
                    *flag = true;
                }
            }
            for (slot, &already_framed) in is_framed.iter().enumerate() {
                if already_framed || hovered == Some(slot) || !tfl_input::live(sim, slot) {
                    continue;
                }
                if let Some(frame) = caps_lock_frame(sim, slot, mode) {
                    framed.push(frame);
                }
            }
        }
    }
    TflMarkers {
        framed,
        band: selection.and_then(|s| s.band),
        gesture: selection.and_then(|s| s.gesture.clone()),
        ground: ground.to_vec(),
    }
}

pub(crate) fn caps_lock_frame(
    sim: &mandrake_tfl_sim::TflSim,
    slot: usize,
    mode: mandrake_view::selection::state::CapsLock,
) -> Option<TflFrame> {
    use mandrake_view::selection::state as st;
    let m = sim.monsters.get(slot)?;
    let local_team = sim.local_team().ok().flatten().unwrap_or(-1);
    let context = st::GameContext {
        game_mode: u16::from(sim.game_mode == 1),
        ..st::GameContext::single_player(local_team, crate::tfl_live::LOCAL_PLAYER as i8)
    };
    let alliances = st::Alliances::new(16);
    let monster = st::MonsterInterface {
        team: m.team as i8,
        player: m.player,
        flags: 0,
        flags8: 0,
        definition_flags: 0,
        definition_class: 0,
    };
    let state = st::overlay_state(
        &context,
        &alliances,
        st::OverlayObject {
            flags: 0,
            monster: Some(&monster),
        },
        Some(mode),
    )?;
    Some(TflFrame {
        object: m.object,
        monster: true,
        hovered_only: true,
        frame: state.frame,
    })
}

fn untr_drop(rect: [i32; 4], ui: UiScale) -> i32 {
    let fit = untr_fit(ui);
    let bottom = f64::from(fit.origin.1) + f64::from(rect[1] + rect[3]) * fit.factor;
    (bottom * f64::from(ui.logical.1) / f64::from(ui.window.1.max(1)))
        .ceil()
        .max(0.0) as i32
}

pub(crate) fn quit_fit(quit: Option<&TflQuitDialog>, ui: UiScale) -> CanvasFit {
    if quit.is_some_and(|q| q.modeless_rect().is_some()) {
        untr_fit(ui)
    } else {
        quit_dialog_fit(ui)
    }
}

pub(crate) fn untr_fit(ui: UiScale) -> CanvasFit {
    let mut fit = quit_dialog_fit(ui);
    fit.origin.1 = 0;
    fit
}

pub(crate) fn quit_dialog_fit(ui: UiScale) -> CanvasFit {
    crate::ui_scale::dialog_fit(ui, CANVAS)
}

fn draw_quit(
    gpu: &Gpu<'_>,
    game: &mut GameView,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    size: (u32, u32),
) {
    let Some(quit) = game.quit.as_ref() else {
        return;
    };
    let quit_gpu = game
        .quit_gpu
        .get_or_insert_with(|| LiveGpu::new(gpu.device, gpu.interface_format));
    quit_gpu.draw_quit(
        (gpu.device, gpu.queue),
        quit,
        encoder,
        (target, size),
        (gpu.menu_filter, gpu.ui),
    );
}

impl LiveGpu {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            dialog_overlay: OverlayRenderer::new(device, format),
            dialog: None,
            revision: u64::MAX,
            scaler: InterfaceScaler::new(device, format),
        }
    }

    pub(crate) fn draw_quit(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        quit: &TflQuitDialog,
        encoder: &mut wgpu::CommandEncoder,
        (target, size): (&wgpu::TextureView, (u32, u32)),
        (filter, ui): (InterfaceFilter, UiScale),
    ) {
        let Self {
            dialog_overlay,
            dialog,
            revision,
            scaler,
        } = self;
        if *revision != quit.revision() {
            let rgba = quit.canvas().to_rgba();
            match dialog.as_ref() {
                Some(i) => OverlayRenderer::update_image(queue, i, &rgba),
                None => {
                    *dialog =
                        Some(dialog_overlay.create_image(device, queue, CANVAS.0, CANVAS.1, &rgba));
                }
            }
            *revision = quit.revision();
        }
        if let Some(i) = dialog.as_ref() {
            let b = quit.bounds();
            let mut draw = OverlayDraw::at(OverlayLayer::Dialog, i, b.left, b.top, 0xff);
            draw.src = [b.left, b.top, b.right - b.left, b.bottom - b.top];
            draw.dst = draw.src;
            let layer = scaler.layer_view(device, CANVAS);
            dialog_overlay.encode_cleared(device, queue, encoder, layer, CANVAS, &[draw]);
            let fit = quit_fit(Some(quit), ui);
            scaler.composite_into(
                queue,
                encoder,
                target,
                Some([fit.origin.0, fit.origin.1, fit.size.0, fit.size.1]),
                size,
                filter,
            );
        }
    }

    pub(crate) fn draw_report(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        dialogs: &mut TflDialogs,
        encoder: &mut wgpu::CommandEncoder,
        (target, size): (&wgpu::TextureView, (u32, u32)),
        (filter, ui): (InterfaceFilter, UiScale),
    ) {
        let Some(b) = dialogs.stack().iter().map(|d| d.bounds()).reduce(|a, b| {
            mandrake_tag::Rect::new(
                a.top.min(b.top),
                a.left.min(b.left),
                a.bottom.max(b.bottom),
                a.right.max(b.right),
            )
        }) else {
            return;
        };
        let Self {
            dialog_overlay,
            dialog,
            revision,
            scaler,
        } = self;
        let canvas = dialogs.menu_canvas();
        if *revision != canvas.revision {
            let rgba = canvas.canvas.to_rgba();
            match dialog.as_ref() {
                Some(i) => OverlayRenderer::update_image(queue, i, &rgba),
                None => {
                    *dialog =
                        Some(dialog_overlay.create_image(device, queue, CANVAS.0, CANVAS.1, &rgba));
                }
            }
            *revision = canvas.revision;
        }
        let Some(i) = dialog.as_ref() else {
            return;
        };
        let mut draw = OverlayDraw::at(OverlayLayer::Dialog, i, b.left, b.top, 0xff);
        draw.src = [b.left, b.top, b.right - b.left, b.bottom - b.top];
        draw.dst = draw.src;
        let layer = scaler.layer_view(device, CANVAS);
        dialog_overlay.encode_cleared(device, queue, encoder, layer, CANVAS, &[draw]);
        let fit = quit_dialog_fit(ui);
        scaler.composite_into(
            queue,
            encoder,
            target,
            Some([fit.origin.0, fit.origin.1, fit.size.0, fit.size.1]),
            size,
            filter,
        );
    }
}

#[cfg(test)]
#[path = "tfl_net_escape_tests.rs"]
mod net_escape_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sketch_payload_is_flags_count_and_points() {
        assert_eq!(
            sketch_payload(0, &[0x1405, 0]),
            [0, 0, 0, 2, 0x14, 0x05, 0, 0]
        );
        assert_eq!(sketch_payload(2, &[]), [0, 2, 0, 0]);
    }

    #[test]
    fn f5_is_quicksave_in_tfl() {
        let c = crate::controls::tfl::tfl_defaults();
        let f5 = crate::controls::MythKey::from_name("F5").unwrap();
        let mut held = HeldKeys::default();
        held.press(f5);
        assert!(
            c.actions_for_press(f5, &held)
                .contains(&ControlAction::Quicksave)
        );
    }

    #[test]
    fn the_camera_factor_is_the_share_of_the_tick_elapsed() {
        assert_eq!(camera_alpha(Duration::ZERO), 0.0);
        for (us, alpha) in [(8_333, 0.25), (16_666, 0.5), (25_000, 0.75), (30_000, 0.9)] {
            let a = camera_alpha(Duration::from_micros(us));
            assert!((a - alpha).abs() < 1e-4, "{us} us: {a}");
        }
        assert_eq!(camera_alpha(TICK), 1.0);
        assert_eq!(camera_alpha(Duration::from_millis(500)), 1.0);
    }

    #[test]
    fn obmos_subject_target_is_tfls() {
        use crate::camera_control::{ObserverLocation, subject_target, subject_target_as};
        let obs = |x, y| ObserverLocation {
            point: (x, y),
            facing: 0,
        };
        let cells = (64, 64);
        assert_eq!(
            subject_target_as(obs(10000, 4000), (30000, 4000), cells, &TFL_MOVE),
            ((30000 - 0x1400, 4000), 0)
        );
        let near_edge = subject_target_as(obs(66000, 4000), (60000, 4000), cells, &TFL_MOVE);
        assert_eq!(near_edge, ((60000 + 0x1400, 4000), 0x8000));
        let myth2 = subject_target(obs(66000, 4000), (60000, 4000), cells);
        assert_eq!(myth2.0, ((64 << 9) - 1, 4000));
        let past = subject_target_as(obs(70000, 4000), (64000, 4000), cells, &TFL_MOVE);
        assert_eq!(past.0, ((64 << 10) - 1, 4000));
        let (_, f) = subject_target_as(obs(1000, 1000), (9000, 5000), cells, &TFL_MOVE);
        assert_eq!(f, mandrake_tfl_sim::trig::arctangent(8000, 4000));
    }

    #[test]
    fn one_cursor_shows() {
        assert!(!shows_system_cursor(true));
        assert!(shows_system_cursor(false));
    }

    #[test]
    fn the_sims_mode_picks_the_speed_mode() {
        assert_eq!(speed_mode(0), GameMode::Film);
        assert_eq!(speed_mode(NETWORK_GAME_MODE), GameMode::Network);
        assert_eq!(speed_mode(2), GameMode::Solo);
    }
}
