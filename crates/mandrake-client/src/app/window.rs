use crate::app::ClientApp;
use crate::app::frame_budget::{BudgetOptions, FrameBudget};
use crate::app::myth2_scene::Myth2Scene;
use crate::app::shell::{Shell, ShellHost};
use crate::app::shell_game::MapRun;
use crate::app::sound::{load_map_sound, play_debug_sound};
use crate::args::Args;
use crate::audio_out::AudioOutput;
use crate::carnage_report::{self, EndStage, MapEnd};
use crate::clock::Instant;
use crate::effects::{load_local_projectiles, load_particle_field};
use crate::game_speed::{GameMode, GameSpeed};
use crate::interface_setup::{layout_interface, load_hud, load_minimap};
use crate::render_passes::create_depth_view;
use crate::start_camera::load_observer_physics_constants;
use crate::startup::Startup;
use crate::{settings, ui_scale};
use mandrake_rec::header::MonsterInitializer;
use tracing::{debug, info, warn};
use winit::dpi::PhysicalSize;
use winit::event_loop::{ControlFlow, EventLoop};

pub(crate) fn run_window(args: Args, start: Startup) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut host = ShellHost::new(args);
    let mut game = Myth2Scene::new();
    game.start_map(&mut host, start, MapRun::Direct);
    crate::app::shell::run_event_loop(event_loop, Shell::new(host, Box::new(game)))?;
    Ok(())
}

pub(crate) fn run_network_window(
    args: Args,
    start: Startup,
    net: crate::net_game::NetPlay,
) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let own_pause = crate::app::shell_menu::OwnPauseDialog::new(&args)
        .inspect_err(|e| warn!("the pause dialog won't open: {e:#}"))
        .ok();
    let mut host = ShellHost::new(args);
    #[cfg(not(target_arch = "wasm32"))]
    host.wake_on_net(&event_loop);
    let mut game = Myth2Scene::new();
    game.pending_net = Some(net);
    game.start_map(&mut host, start, MapRun::Direct);
    if let Some(app) = game.app.as_mut() {
        app.own_pause = own_pause;
    }
    let mut shell = Shell::new(host, Box::new(game));
    crate::app::guard::install();
    event_loop.run_app(&mut shell)?;
    Ok(())
}

impl ClientApp {
    pub(crate) fn for_map(shell: &ShellHost, args: &Args, start: Startup, run: MapRun) -> Self {
        let (width, height) = shell.map_size();
        let Startup {
            interface,
            mesh_tag,
            mesh_name,
            mesh_data,
            terrain_texture,
            landscape_detail,
            archive,
            large_archive,
            connectors,
            ambient_tag,
            replay_recording,
            restore,
            film_plugin_names,
            film_names,
            initial_camera,
            start_state,
            interface_colors,
            interface_collection,
            scenery_objects,
            engine_idle,
            map_load_seed,
            sim,
        } = start;
        let film_plugins: Vec<&str> = film_plugin_names.iter().map(String::as_str).collect();
        let observer_constants = load_observer_physics_constants(&archive);
        info!("Starting interactive desktop window with tactical camera & unit orders...");
        info!(
            "Camera: W/S move, Z/X move left/right, A/D turn, Q/E orbit, C/V zoom (Shift: presets), \
         screen edges scroll, wheel zooms; Escape pauses, Ctrl+Q quits (see mandrake.default.toml's [controls])"
        );

        let mut app = ClientApp::new(
            mesh_tag,
            connectors,
            mesh_name,
            terrain_texture,
            archive,
            large_archive,
            sim,
            replay_recording,
            width,
            height,
            initial_camera,
            interface_colors,
            scenery_objects,
            shell.controls.clone(),
            observer_constants,
            engine_idle,
        );
        if let Some(state) = start_state {
            app.seed_observer(state);
        }
        app.film_names = film_names;
        app.landscape_detail = landscape_detail;
        app.observer.min_zoom = crate::start_camera::start_min_zoom(
            &mesh_data,
            &crate::start_camera::StartTeams::of(&app.sim),
            restore.is_some(),
        );
        app.restore = restore;
        let mode = if app.replay.is_some() {
            GameMode::Film
        } else if app.sim.is_network_game() {
            GameMode::Network
        } else {
            GameMode::Solo
        };
        app.game_speed =
            GameSpeed::for_map(mode, app.mesh_tag.mesh_id, &mesh_data, args.game_speed);
        info!("Game speed: {}/256", app.game_speed.speed());
        let observer = app
            .camera_control
            .observer(&app.observer, app.camera_frame())
            .location()
            .point;
        app.particle_field = load_particle_field(
            &app.archive,
            app.large_archive.as_ref(),
            &app.mesh_tag,
            &app.sim,
            observer,
        );
        app.local_projectiles = load_local_projectiles(
            &app.archive,
            app.large_archive.as_ref(),
            &app.sim,
            &app.mesh_tag,
        );
        app.interface.tfl_selection_boxes = args.tfl_selection_boxes;
        let view = &shell.view;
        app.translucent_bars = view.translucent_bars;
        app.settings_path = shell.settings_path.clone();
        app.ui_mode = shell.ui_mode;
        app.ui_filter = view.ui_filter;
        app.sampling = view.sampling;
        app.fog = crate::fog::MapFog::at_map_load(args, &app.mesh_tag);
        app.lighting =
            crate::lighting::MapLighting::at_map_load(args, &app.mesh_tag, &app.mesh_name);
        app.fog_reload = crate::fog::FogReload::at_map_load(args, &app.mesh_tag, &app.fog);
        app.lighting_reload =
            crate::lighting::LightingReload::at_map_load(args, &app.mesh_tag, &app.mesh_name);
        app.reflection_soft_edge = view.reflection_soft_edge;
        app.water = view.water_on(app.mesh_tag.mesh_id.as_str()).then(|| {
            crate::water_config::load(args.conf.as_deref(), false, app.mesh_tag.mesh_id.as_str())
        });
        app.sprite_filter = view.sprite_filter;
        app.ui = ui_scale::UiScale::new(app.ui_mode, width, height);
        if let ui_scale::UiScaleMode::Zoom(chosen) = app.ui_mode {
            info!(
                "UI zoom: {}% in this {width}x{height} window ({chosen}% chosen)",
                app.ui.zoom.unwrap_or(100)
            );
        }
        app.interface_collection = interface_collection;
        let (logical_w, logical_h) = app.ui.logical;
        app.hud = load_hud(
            &args.archive,
            &film_plugins,
            Some(&mesh_data),
            &app.mesh_tag,
            &app.sim,
            map_load_seed,
            ((logical_w, logical_h), interface),
        );
        app.camera_control.set_mesh_limits(
            &mesh_data,
            (app.mesh_tag.cell_width, app.mesh_tag.cell_height),
        );
        app.minimap = load_minimap(
            &args.archive,
            &film_plugins,
            Some(&mesh_data),
            &app.mesh_tag,
            &app.sim,
            logical_w,
            logical_h,
        );
        app.frame_budget = args
            .frame_budget
            .filter(|_| run != MapRun::Rebuild)
            .map(|seconds| {
                let mut budget = FrameBudget::new(BudgetOptions {
                    seconds,
                    from_tick: args.frame_budget_tick,
                    scan_ticks: args.frame_budget_scan,
                    wait: args.frame_budget_wait,
                    follow: args.frame_budget_follow,
                });
                budget.gpu_resized(width, height);
                budget
            });
        if app.frame_budget.is_some()
            && let Some(ids) = &args.select_ids
        {
            app.selected_unit_ids = ids.clone();
        }
        app.held_for_begin_play = run == MapRun::Session;
        app.hold_at_tick = args.hold_at_tick.map(|tick| (tick, 0));
        app.skip_passes = crate::app::ResolvedPassSkips::from_arg(args.skip_passes.as_deref());
        app.replay_path = app.replay.as_ref().and_then(|_| args.replay.clone());
        if run != MapRun::Rebuild {
            app.force_end = args.force_end;
        }
        if run != MapRun::Rebuild && args.film_report.is_none() && args.frame_budget.is_none() {
            let mut map_end = MapEnd::new(carnage_report::has_alternative_next_level(&mesh_data));
            if let Some(game_dir) = args.archive.parent().and_then(std::path::Path::parent) {
                map_end = map_end.with_tag_files(game_dir.to_path_buf(), film_plugin_names.clone());
            }
            app.map_end = Some(map_end);
        }
        app.audio = shell.audio.as_ref().map(AudioOutput::link);
        let sound_start = Instant::now();
        load_map_sound(&mut app, args, ambient_tag);
        app.sound_load = sound_start.elapsed();
        if run != MapRun::Rebuild {
            play_debug_sound(&app, args);
        }
        app
    }

    pub(crate) fn begin_play(&mut self) {
        info!("BeginPlay at tick {}", self.sim.current_tick);
        self.held_for_begin_play = false;
        if let Some(budget) = &mut self.frame_budget {
            budget.begin_play();
        }
        self.hold_clock();
    }

    pub(crate) fn hold_clock(&mut self) {
        self.last_tick_time = Instant::now();
        self.physics_accumulator = 0.0;
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        let changed = self.sim_paused != paused;
        self.sim_paused = paused;
        if changed && self.net.is_some() && self.net_coop_host() {
            self.queue_system_command(mandrake_sim::net_game::PAUSE, i16::from(paused));
        }
    }

    pub(crate) fn force_network_endgame(&mut self, kind: i16) {
        let host = self.net.as_ref().is_some_and(|net| net.is_host());
        let (over, gameplay) = {
            (
                self.sim.engine.game.flags & mandrake_sim::game_state::FLAG_GAME_OVER != 0,
                self.sim.engine.formation_globals.gameplay_version,
            )
        };
        if host
            && self.sim.is_network_game()
            && !over
            && gameplay > mandrake_common::game_options::GAMEPLAY_1_7_2
        {
            let queued = self.sim.force_network_endgame(kind);
            info!(
                "net: the host's forced end, kind {kind}, at tick {} (queued: {queued})",
                self.sim.current_tick
            );
        } else {
            let line = if kind == 2 {
                "Cannot rehost at this time."
            } else {
                "Cannot restart at this time."
            };
            info!("console: {line}");
            self.print_line(line);
        }
    }

    pub(crate) fn veterans_at_end(&self) -> Vec<MonsterInitializer> {
        self.sim.monster_initializers()
    }

    pub(crate) fn quit(&mut self) {
        self.quit_requested = true;
    }

    pub(crate) fn finish_map(&mut self) {
        if matches!(EndStage::of(&self.sim), EndStage::Live { .. }) {
            return;
        }
        if !self.sim.game_over() && !matches!(EndStage::of(&self.sim), EndStage::FilmFrozen) {
            return;
        }
        info!(
            "The map's end reached black at tick {}",
            self.sim.current_tick
        );
        let mut model = None;
        if let Some(end) = self.sim.game_end() {
            let map = self
                .hud
                .as_ref()
                .and_then(|hud| hud.map_name(self.mesh_tag.string_list_tag))
                .unwrap_or_else(|| self.mesh_name.clone());
            let alternative = self.map_end.as_ref().is_some_and(MapEnd::alternative);
            let tags = self.map_end.as_ref().and_then(MapEnd::open_tags);
            let (report, printed) = carnage_report::print(
                &self.sim,
                &map,
                end,
                alternative,
                tags.as_ref(),
                &self.film_names,
            );
            if let Err(e) = printed {
                warn!("the carnage report wasn't printed: {e}");
            }
            model = Some(report);
        }
        self.map_over = Some(model.unwrap_or_default());
    }

    pub(crate) fn standings(&self) -> mandrake_session::PostgameReport {
        let map = self
            .hud
            .as_ref()
            .and_then(|hud| hud.map_name(self.mesh_tag.string_list_tag))
            .unwrap_or_else(|| self.mesh_name.clone());
        let lookup = self.map_end.as_ref().and_then(MapEnd::open_tags);
        let tags = |group: [u8; 4], id: mandrake_common::FourCC| {
            lookup
                .as_ref()
                .and_then(|l| l.get(mandrake_common::FourCC(group), id))
        };
        let header = carnage_report::ReportHeader {
            map: &map,
            end: mandrake_sim::game_state::GameEnd {
                tick: self.sim.current_tick as i32,
                winner: -1,
                code: 0,
            },
            alternative_next_level: false,
            film: false,
            names: &self.film_names,
        };
        let strings = carnage_report::ReportStrings::load(&tags);
        let view = self.sim.statistics();
        carnage_report::build_report(view.as_ref(), &header, &strings, &tags)
    }

    pub(crate) fn release_window(&mut self) {
        self.gpu_state = None;
        self.own_pause = None;
        self.window = None;
    }

    pub(crate) fn set_screen_size(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        if let Some(budget) = &mut self.frame_budget {
            budget.gpu_resized(self.width, self.height);
        }
        self.ui = ui_scale::UiScale::new(self.ui_mode, self.width, self.height);
        layout_interface(
            self.hud.as_mut(),
            self.minimap.as_mut().map(|m| &mut m.map),
            &self.ui,
        );
        let (lw, lh) = self.ui.logical;
        if self.ui.is_scaled() {
            info!(
                "UI scale {:?}: window {}x{}, logical screen {}x{}, s = {}",
                self.ui_mode, self.width, self.height, lw, lh, self.ui.factor
            );
        }
        self.sync_min_window();
    }

    pub(crate) fn sync_min_window(&mut self) {
        let min = self.ui.zoom.map(ui_scale::min_window);
        if min != self.min_window {
            self.min_window = min;
            if let Some(window) = &self.window {
                window.set_min_inner_size(min.map(|(w, h)| PhysicalSize::new(w, h)));
            }
        }
    }

    pub(crate) fn adjust_ui_zoom(&mut self, delta: i32) {
        let Some(level) = ui_zoom_press(self.ui_mode, &self.ui, delta, self.window.as_deref())
        else {
            return;
        };
        self.ui_mode = ui_scale::UiScaleMode::from_zoom(level);
        self.set_screen_size(self.width, self.height);
        let granted = ui_zoom_applied(
            level,
            (self.width, self.height),
            self.window.as_deref(),
            self.settings_path.as_deref(),
        );
        if let Some(granted) = granted {
            self.granted_window_size = granted;
        }
    }

    pub(crate) fn gpu_resized(&mut self, size: PhysicalSize<u32>) {
        if let Some(budget) = &mut self.frame_budget {
            budget.gpu_resized(size.width, size.height);
        }
        if let Some(gpu) = &mut self.gpu_state {
            let (width, height) = surface_extent(size);
            gpu.surface_config.width = width;
            gpu.surface_config.height = height;
            gpu.depth_texture_view = size_targets(&gpu.device, size, gpu.reflection_edge.as_mut());
        }
    }
}

pub(crate) fn surface_extent(size: PhysicalSize<u32>) -> (u32, u32) {
    (size.width.max(1), size.height.max(1))
}

pub(crate) fn size_targets(
    device: &wgpu::Device,
    size: PhysicalSize<u32>,
    edge: Option<&mut mandrake_render::ReflectionEdge>,
) -> wgpu::TextureView {
    let (width, height) = surface_extent(size);
    if let Some(edge) = edge {
        edge.resize(device, width, height);
    }
    create_depth_view(device, width, height)
}

fn zoom_screen(window: Option<&winit::window::Window>, own: (u32, u32)) -> (u32, u32) {
    let Some(window) = window else {
        return own;
    };
    if window.fullscreen().is_some() || window.is_maximized() {
        return own;
    }
    window.current_monitor().map_or(own, |monitor| {
        let size = monitor.size();
        (size.width.max(own.0), size.height.max(own.1))
    })
}

pub(crate) fn ui_zoom_press(
    mode: ui_scale::UiScaleMode,
    ui: &ui_scale::UiScale,
    delta: i32,
    window: Option<&winit::window::Window>,
) -> Option<u16> {
    let level = ui_scale::zoom_after_press(mode, ui, delta, zoom_screen(window, ui.window));
    if ui_scale::UiScaleMode::from_zoom(level) == mode && ui.zoom.unwrap_or(100) == level {
        debug!("UI zoom: stays at {level}%");
        return None;
    }
    info!("UI zoom: {level}%");
    Some(level)
}

pub(crate) fn ui_zoom_applied(
    level: u16,
    size: (u32, u32),
    window: Option<&winit::window::Window>,
    settings_path: Option<&std::path::Path>,
) -> Option<Option<PhysicalSize<u32>>> {
    let (min_w, min_h) = ui_scale::min_window(level);
    let mut granted = None;
    if level > 100 && (size.0 < min_w || size.1 < min_h) {
        let wanted = PhysicalSize::new(size.0.max(min_w), size.1.max(min_h));
        info!(
            "UI zoom: growing the window to {}x{}",
            wanted.width, wanted.height
        );
        granted = Some(window.and_then(|window| window.request_inner_size(wanted)));
    }
    if let Some(path) = settings_path
        && let Err(e) = settings::save_ui_zoom(path, level)
    {
        warn!("settings: {}: {e}; the level isn't saved", path.display());
    }
    granted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimised_window_keeps_a_one_pixel_depth_buffer() {
        assert_eq!(surface_extent(PhysicalSize::new(0, 0)), (1, 1));
        assert_eq!(surface_extent(PhysicalSize::new(0, 720)), (1, 720));
        assert_eq!(surface_extent(PhysicalSize::new(1280, 720)), (1280, 720));
        let Some((device, _queue)) = crate::tests::test_gpu() else {
            return;
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _view = size_targets(&device, PhysicalSize::new(0, 0), None);
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }

    #[test]
    fn resolved_pass_skips_parses_individual_and_all() {
        let none = crate::app::ResolvedPassSkips::from_arg(None);
        assert!(!none.world.scene && !none.interface && !none.fade);
        let some = crate::app::ResolvedPassSkips::from_arg(Some("scene,fade"));
        assert!(some.world.scene && !some.world.sprites && !some.interface && some.fade);
        let all = crate::app::ResolvedPassSkips::from_arg(Some("all"));
        assert!(
            all.world.scene
                && all.world.category4
                && all.world.opaque
                && all.world.sprites
                && all.interface
                && all.fade
        );
    }
}
