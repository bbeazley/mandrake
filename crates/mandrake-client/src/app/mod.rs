pub(crate) mod bar_reservation;
pub(crate) mod camera;
pub(crate) mod cursor;
pub(crate) mod cutscene;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod cutscene_hd;
pub(crate) mod detach;
pub(crate) mod fps_cap;
pub(crate) mod frame_budget;
pub(crate) mod game_scene;
pub(crate) mod gpu;
pub(crate) mod guard;
pub(crate) mod input;
pub(crate) mod menu_sound;
pub(crate) mod myth2_scene;
pub(crate) mod orders;
pub(crate) mod recording;
pub(crate) mod render;
pub(crate) mod save_game;
pub(crate) mod shell;
pub(crate) mod shell_game;
pub(crate) mod shell_menu;
pub(crate) mod shell_net;
pub(crate) mod sound;
pub(crate) mod tick;
pub(crate) mod view_settings;
pub(crate) mod window;

use crate::app::input::right_click_mode;
use crate::clock::Instant;
use crate::controls::{ControlAction, Controls, HeldKeys, NotImplementedLog, WheelState};
use crate::model_instances::{ModelInstance, ModelMeshKey};
use crate::overlays::MoveGesture;
use crate::scenery_billboards::{ObjectShadow, ReflectionRipple};
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim::UnitAnimator;
use crate::{
    audio_out, camera_control, carnage_report, corpse, hud, interface_state, map_sound, minimap,
    move_marker, particles, projectile_local, scenery, selection_input, ui_scale, unit_anim,
    window_surface,
};
use mandrake_common::FourCC;
use mandrake_rec::Recording;
use mandrake_render::{
    BillboardRenderer, LandscapeTexture, ModelMesh, ModelRenderer, OverlayRenderer, TerrainMesh,
    TerrainRenderer, TranslucentBatch,
};
use mandrake_sim::Simulation;
use mandrake_tag::collection::{CollectionTag, TerrainTexture};
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_tag::{InterfaceColorTable, MeshConnectorList, MeshTag, TagArchive, WorldPoint};
use mandrake_view::camera::ObserverController;
use mandrake_view::camera::myth_observer::ObserverPhysicsConstants;
use mandrake_view::landscape::LandscapeTexels;
use mandrake_view::{
    billboard::BillboardInstance,
    camera::{Camera, CameraState},
    selection,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use winit::window::Window;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ResolvedPassSkips {
    pub(crate) world: crate::render_passes::PassSkips,
    pub(crate) interface: bool,
    pub(crate) fade: bool,
}

impl ResolvedPassSkips {
    pub(crate) fn from_arg(arg: Option<&str>) -> Self {
        let mut skips = Self::default();
        for name in arg.into_iter().flat_map(|s| s.split(',')) {
            match name {
                "all" => {
                    skips.world = crate::render_passes::PassSkips {
                        scene: true,
                        category4: true,
                        opaque: true,
                        sprites: true,
                    };
                    skips.interface = true;
                    skips.fade = true;
                }
                "scene" => skips.world.scene = true,
                "category4" => skips.world.category4 = true,
                "opaque" => skips.world.opaque = true,
                "sprites" => skips.world.sprites = true,
                "interface" => skips.interface = true,
                "fade" => skips.fade = true,
                _ => {}
            }
        }
        skips
    }
}

pub(crate) struct ClientApp {
    mesh_tag: MeshTag,
    connectors: MeshConnectorList,
    mesh_name: String,
    terrain_texture: Option<TerrainTexture>,
    landscape_detail: Option<std::sync::Arc<mandrake_render::detail::LandscapeDetail>>,
    archive: TagArchive,
    large_archive: Option<TagArchive>,
    sim: Simulation,
    replay: Option<Recording>,
    replay_path: Option<PathBuf>,
    restore: Option<crate::startup::SavedGameStart>,
    restore_queued: bool,
    recording: Option<recording::LiveRecording>,
    stop_report: Option<String>,
    test_stop_at: Option<u32>,
    pub(crate) net: Option<crate::net_game::NetPlay>,
    pub(crate) net_strings: Vec<Vec<u8>>,
    snapshot_wanted: Option<save_game::SnapshotUse>,
    snapshot: Option<mandrake_rec::saved_game::Snapshot>,
    snapshot_taken: Option<save_game::SnapshotTaken>,
    pub(crate) film_names: Vec<(mandrake_session::Text, mandrake_session::Text)>,
    title_tick: Option<u32>,
    title_updated: Instant,
    sim_paused: bool,
    hold_at_tick: Option<(u32, u32)>,
    pub(crate) skip_passes: ResolvedPassSkips,
    stop_line_key: Option<(bool, bool, u32, bool)>,
    last_tick_time: Instant,
    physics_accumulator: f32,
    width: u32,
    height: u32,
    window: Option<Arc<Window>>,
    gpu_state: Option<GpuWindowState>,
    occluded: bool,
    audio: Option<audio_out::AudioLink>,
    map_sound: Option<map_sound::MapSound>,
    sound_archives: Vec<TagArchive>,
    pending_animation_sounds: Vec<(FourCC, WorldPoint)>,
    camera: Camera,
    prev_camera_state: CameraState,
    render_camera: Camera,
    current_cursor_pos: (f32, f32),
    mouse_left_down: bool,
    mouse_right_down: bool,
    selected_unit_ids: Vec<u32>,
    move_gesture: Option<MoveGesture>,
    walking_orders: BTreeSet<u32>,
    unit_animator: UnitAnimator,
    controls: Controls,
    held_keys: HeldKeys,
    wheel: WheelState,
    wheel_notches: input::WheelNotches,
    not_implemented: NotImplementedLog,
    observer: ObserverController,
    camera_control: camera_control::CameraControl,
    game_speed: crate::game_speed::GameSpeed,
    particle_field: Option<particles::ParticleField>,
    local_projectiles: Option<projectile_local::LocalProjectiles>,
    lightning_random: mandrake_tag::sound_manager::LocalRandom,
    landscape_edits: Vec<corpse::LandscapeEdit>,
    cursor_in_window: bool,
    selection_machine: selection_input::SelectionMachine,
    selection_actions: selection_input::ActionBuilder,
    pressed_since_poll: (bool, bool),
    pending_presses: Vec<(ControlAction, bool)>,
    interface: interface_state::InterfaceView,
    hover: selection::state::HoverTracker,
    caps_lock_key: interface_state::CapsLockKey,
    scores: crate::scores::ScoreDisplay,
    detach_menu: detach::ConsoleMenu,
    focused: bool,
    machine_clock: Instant,
    interface_colors: Option<InterfaceColorTable>,
    scenery_objects: scenery::SceneryObjects,
    hud: Option<hud::Hud>,
    hud_selection: Option<Vec<u32>>,
    minimap: Option<minimap::Minimap>,
    translucent_bars: bool,
    ui_mode: ui_scale::UiScaleMode,
    ui_filter: Option<ui_scale::UiFilter>,
    sampling: ui_scale::Sampling,
    fog: crate::fog::MapFog,
    fog_seconds: f32,
    lighting: crate::lighting::MapLighting,
    fog_reload: Option<crate::fog::FogReload>,
    lighting_reload: Option<crate::lighting::LightingReload>,
    reflection_soft_edge: bool,
    water: Option<mandrake_view::water::WaterSettings>,
    sprite_filter: Option<ui_scale::UiFilter>,
    ui: ui_scale::UiScale,
    dialog_scale: u16,
    settings_path: Option<PathBuf>,
    min_window: Option<(u32, u32)>,
    interface_collection: Option<CollectionTag>,
    local_sprites: move_marker::LocalSprites,
    screenshot_requested: bool,
    frame_budget: Option<frame_budget::FrameBudget>,
    map_end: Option<carnage_report::MapEnd>,
    quit_requested: bool,
    granted_window_size: Option<winit::dpi::PhysicalSize<u32>>,
    map_over: Option<mandrake_session::PostgameReport>,
    held_for_begin_play: bool,
    session_map: bool,
    dialog_ask: Option<input::DialogAsk>,
    own_pause: Option<shell_menu::OwnPauseDialog>,
    hints_open: Option<bool>,
    shell_fade: f32,
    force_end: Option<crate::args::ForceEnd>,
    sound_load: std::time::Duration,
}

pub(crate) struct GpuWindowState {
    surface_config: wgpu::SurfaceConfiguration,
    surface_timeouts: window_surface::SurfaceTimeoutLog,
    terrain_renderer: TerrainRenderer,
    terrain_mesh: TerrainMesh,
    mesh_ripple: crate::mesh_ripple::MeshRipple,
    media_time: Option<i32>,
    pub(crate) landscape: Option<(LandscapeTexels, LandscapeTexture)>,
    pub(crate) static_shadows: Vec<ObjectShadow>,
    pub(crate) static_lights: Vec<mandrake_render::PointLight>,
    moving_shadows: Vec<ObjectShadow>,
    landscape_edits_applied: usize,
    stain_random: LocalRandom,
    depth_texture_view: wgpu::TextureView,
    reflection_edge: Option<mandrake_render::ReflectionEdge>,
    water: Option<mandrake_render::water::WaterShading>,
    ripple_sources: crate::water_ripples_input::RippleSources,
    ripple_clock: Option<u128>,
    model_renderer: ModelRenderer,
    model_meshes: BTreeMap<ModelMeshKey, ModelMesh>,
    model_instances: Vec<ModelInstance>,
    model_effect_meshes: crate::model_effects::EffectMeshCache,
    translucent_batch: TranslucentBatch,
    category2_batch: TranslucentBatch,
    reflection_batch: mandrake_render::ReflectionBatch,
    billboard_renderer: BillboardRenderer,
    pub(crate) sprite_mgr: ClientSpriteManager,
    team_cache_tick: Option<u32>,
    pub(crate) scenery_billboards: Vec<BillboardInstance>,
    billboard_scratch: Vec<BillboardInstance>,
    pub(crate) scenery_ripples: Vec<ReflectionRipple>,
    pub(crate) scenery_animator: scenery::SceneryAnimator,
    pub(crate) scenery_shadow_rects: Vec<mandrake_view::landscape::PixelRect>,
    overlay: OverlayRenderer,
    hud_images: hud::HudImages,
    detach_menu_images: detach::MenuImages,
    minimap_images: minimap::MinimapImages,
    interface_scaler: mandrake_render::InterfaceScaler,
    dialog_scaler: mandrake_render::InterfaceScaler,
    gpu_timer: Option<frame_budget::GpuTimer>,
    screen_fade: mandrake_render::ScreenFade,
    queue: wgpu::Queue,
    device: wgpu::Device,
}

impl ClientApp {
    #[allow(clippy::too_many_arguments)]
    fn new(
        mesh_tag: MeshTag,
        connectors: MeshConnectorList,
        mesh_name: String,
        terrain_texture: Option<TerrainTexture>,
        archive: TagArchive,
        large_archive: Option<TagArchive>,
        sim: Simulation,
        replay: Option<Recording>,
        width: u32,
        height: u32,
        camera: Camera,
        interface_colors: Option<InterfaceColorTable>,
        scenery_objects: scenery::SceneryObjects,
        controls: Controls,
        observer_constants: ObserverPhysicsConstants,
        engine_idle: Option<unit_anim::EngineIdleModel>,
    ) -> Self {
        let prev_camera_state = camera.current_state();
        let render_camera = camera.clone();
        let camera_control = camera_control::CameraControl::new_for_map(
            sim.game_option_flags(),
            camera_control::INTERPOLATION_PREFERENCE,
        );
        let mut observer = ObserverController::from_camera(&camera);
        observer.constants = observer_constants;
        let right_click = right_click_mode(controls.options.right_click);
        let mut unit_animator = UnitAnimator::with_engine_idle(engine_idle);
        unit_animator.seed_motions(unit_anim::unit_motions(&sim, None));
        Self {
            mesh_tag,
            connectors,
            mesh_name,
            terrain_texture,
            archive,
            large_archive,
            sim,
            replay,
            replay_path: None,
            restore: None,
            restore_queued: false,
            recording: None,
            stop_report: None,
            test_stop_at: std::env::var("M2_TEST_STOP_AT_TICK")
                .ok()
                .and_then(|t| t.parse().ok()),
            net: None,
            net_strings: Vec::new(),
            snapshot_wanted: None,
            snapshot: None,
            snapshot_taken: None,
            film_names: Vec::new(),
            landscape_detail: None,
            title_tick: None,
            title_updated: Instant::now(),
            sim_paused: false,
            hold_at_tick: None,
            skip_passes: ResolvedPassSkips::default(),
            stop_line_key: None,
            last_tick_time: Instant::now(),
            physics_accumulator: 0.0,
            width,
            height,
            window: None,
            gpu_state: None,
            occluded: false,
            camera,
            prev_camera_state,
            render_camera,
            current_cursor_pos: (0.0, 0.0),
            mouse_left_down: false,
            mouse_right_down: false,
            selected_unit_ids: Vec::new(),
            move_gesture: None,
            walking_orders: Default::default(),
            unit_animator,
            controls,
            held_keys: HeldKeys::default(),
            wheel: WheelState::default(),
            wheel_notches: input::WheelNotches::default(),
            not_implemented: NotImplementedLog::default(),
            camera_control,
            game_speed: crate::game_speed::GameSpeed::default(),
            particle_field: None,
            local_projectiles: None,
            lightning_random: mandrake_tag::sound_manager::LocalRandom::new(0),
            landscape_edits: Vec::new(),
            observer,
            cursor_in_window: false,
            selection_machine: selection_input::SelectionMachine::new(right_click),
            selection_actions: selection_input::ActionBuilder::default(),
            pressed_since_poll: (false, false),
            pending_presses: Vec::new(),
            interface: interface_state::InterfaceView::default(),
            hover: selection::state::HoverTracker::default(),
            caps_lock_key: interface_state::CapsLockKey::default(),
            scores: crate::scores::ScoreDisplay::default(),
            detach_menu: detach::ConsoleMenu::default(),
            focused: true,
            machine_clock: Instant::now(),
            interface_colors,
            scenery_objects,
            audio: None,
            map_sound: None,
            sound_archives: Vec::new(),
            pending_animation_sounds: Vec::new(),
            hud: None,
            hud_selection: None,
            minimap: None,
            translucent_bars: false,
            ui_mode: ui_scale::UiScaleMode::Native,
            ui_filter: None,
            sampling: ui_scale::Sampling::default(),
            fog: crate::fog::MapFog::default(),
            fog_seconds: 0.0,
            lighting: crate::lighting::MapLighting::default(),
            fog_reload: None,
            lighting_reload: None,
            reflection_soft_edge: false,
            water: None,
            sprite_filter: None,
            ui: ui_scale::UiScale::new(ui_scale::UiScaleMode::Native, width, height),
            dialog_scale: ui_scale::DIALOG_SCALE_DEFAULT,
            settings_path: None,
            min_window: None,
            interface_collection: None,
            local_sprites: move_marker::LocalSprites::new(),
            screenshot_requested: false,
            frame_budget: None,
            map_end: None,
            quit_requested: false,
            granted_window_size: None,
            map_over: None,
            held_for_begin_play: false,
            session_map: false,
            dialog_ask: None,
            own_pause: None,
            hints_open: None,
            shell_fade: 0.0,
            force_end: None,
            sound_load: std::time::Duration::ZERO,
        }
    }
}
