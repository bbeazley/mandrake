use crate::app::frame_budget::{FrameScene, Lap, Mark, lap};
use crate::app::shell::{acquire_surface, clear_target};
use crate::app::{ClientApp, GpuWindowState, detach};
use crate::game_speed::{self, GameMode, GameSpeed};
use crate::interface_setup::world_frame;
use crate::landscape_edit::apply_landscape_edit;
use crate::model_instances::{animating_shadow_overlays, changed_animating_shadow_rects};
use crate::region_view::{minimap_outline, view_bars};
use crate::render_passes::{BarTargets, PassRule, WorldPasses, WorldTargets, encode_bars};
use crate::scenery_billboards::{
    ObjectShadow, SceneryPart, SceneryView, apply_reflection_ripples, generate_scenery_billboards,
};
use crate::screenshot::{self, Readback};
use crate::ui_scale;
use crate::world_assembly::{LocalOverlays, Myth2Gpu, Myth2Lighting, Myth2View, Myth2World};
use crate::{interface_state, media, minimap, window_surface};
use clap::ValueEnum;
use mandrake_render::{DrawInterface, InterfaceFrame};
use mandrake_view::billboard::BillboardInstance;
use mandrake_view::landscape::{PixelRect, ShadeOverlay, changed_overlay_rects};
use tracing::{info, warn};

const HOLD_FRAMES: u32 = 60;

fn budget_scene(sim: &mandrake_sim::Simulation, billboards: usize, models: usize) -> FrameScene {
    let (mut xs, mut ys): (Vec<i32>, Vec<i32>) = sim
        .units
        .values()
        .filter(|u| u.is_alive())
        .map(|u| sim.world_to_command_point(u.position))
        .unzip();
    xs.sort_unstable();
    ys.sort_unstable();
    let middle = |v: &[i32]| v.get(v.len() / 2).map_or(0.0, |&p| p as f32 / 512.0);
    FrameScene {
        tick: sim.current_tick,
        billboards,
        models,
        living: xs.len(),
        living_middle: [middle(&xs), middle(&ys)],
    }
}

#[derive(Default)]
struct LandscapeUpload {
    bytes: Vec<u8>,
    copies: Vec<(PixelRect, u64)>,
}

impl LandscapeUpload {
    fn padded_row(rect: PixelRect) -> usize {
        (rect.width as usize * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
    }

    fn add(&mut self, rect: PixelRect, rgba: &[u8]) {
        let row = rect.width as usize * 4;
        let padded = Self::padded_row(rect);
        self.copies.push((rect, self.bytes.len() as u64));
        for line in rgba.chunks_exact(row) {
            self.bytes.extend_from_slice(line);
            self.bytes.resize(self.bytes.len() + padded - row, 0);
        }
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) {
        if self.copies.is_empty() {
            return;
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Landscape Rectangles"),
            size: self.bytes.len() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, &self.bytes);
        for &(rect, offset) in &self.copies {
            let to = wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            };
            let size = wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 1,
            };
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(Self::padded_row(rect) as u32),
                        rows_per_image: None,
                    },
                },
                to,
                size,
            );
        }
    }
}

impl ClientApp {
    pub(crate) fn render_frame(
        &mut self,
        surface: Option<window_surface::FrameSurface<'_>>,
        front_end: Option<&mut dyn DrawInterface>,
    ) {
        if let Some(budget) = &mut self.frame_budget
            && budget.reached_seek_tick(self.sim.current_tick)
        {
            self.game_speed = GameSpeed::new(
                GameMode::Film,
                false,
                game_speed::parse_speed("1").unwrap_or_default(),
            );
        }
        let mut clock = self
            .frame_budget
            .as_mut()
            .map(|budget| budget.start_frame(self.sim.current_tick));
        self.update_physics_and_simulation();
        lap(&mut clock, Lap::Sim);

        let (Some(gpu), Some(window_surface::FrameSurface { surface, canvas })) =
            (&mut self.gpu_state, surface)
        else {
            return;
        };

        let Some(frame) = acquire_surface(
            surface,
            &gpu.device,
            &gpu.surface_config,
            &mut gpu.surface_timeouts,
        ) else {
            return;
        };

        lap(&mut clock, Lap::Acquire);

        let target = window_surface::frame_texture(canvas, &frame);
        let view = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(window_surface::SurfaceFormats::of(&gpu.surface_config).scene),
            ..Default::default()
        });

        if let Some(reload) = self.fog_reload.as_mut() {
            reload.poll(&mut self.fog);
        }
        if let Some(reload) = self.lighting_reload.as_mut() {
            reload.poll(&mut self.lighting);
        }
        self.fog_seconds = self.fog.seconds_now();
        let local_time = media::local_time(self.sim.current_tick);
        let window = (gpu.surface_config.width, gpu.surface_config.height);
        if gpu.media_time != Some(local_time) {
            gpu.media_time = Some(local_time);
            gpu.terrain_mesh.update_media(
                &gpu.queue,
                local_time,
                &mandrake_math::engine::trig_tables().sine,
            );
        }

        if let Some(water) = gpu.water.as_mut() {
            water.update(&gpu.queue, local_time);
            water.ensure_reflection_layer(&gpu.device, window);
            water.set_wind(
                &gpu.queue,
                self.machine_clock.elapsed().as_secs_f64(),
                (self.sim.engine.wind.as_ref()).map(|w| w.velocity),
            );
        }
        if gpu.team_cache_tick != Some(self.sim.current_tick) {
            gpu.team_cache_tick = Some(self.sim.current_tick);
            gpu.sprite_mgr.update_team_cache(
                &gpu.device,
                &gpu.queue,
                &gpu.billboard_renderer.texture_bind_group_layout,
                &self.sim,
            );
        }
        let (active_billboards, moving_shadows) = scenery_billboards_for_frame(
            gpu,
            &self.mesh_tag,
            local_time,
            self.render_camera.eye_position(),
            self.unit_animator.fraction(),
        );
        self.interface.hovered = self
            .hover
            .hovered()
            .filter(|id| self.sim.units.get(id).is_some_and(|u| u.is_alive()));
        self.interface.caps_lock = self
            .caps_lock_key
            .is_down()
            .then(|| interface_state::caps_lock_mode(self.controls.options.caps_lock));
        let world =
            crate::world_view::myth2::drawn_world(&self.sim, &self.unit_animator, &gpu.sprite_mgr);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Frame Encoder"),
            });
        if let Some(water) = gpu.water.as_mut().filter(|w| w.has_ripples()) {
            let now = self.machine_clock.elapsed().as_millis();
            let dt = gpu
                .ripple_clock
                .map_or(0.0, |last| now.saturating_sub(last) as f32 / 1000.0);
            gpu.ripple_clock = Some(now);
            let effects = gpu
                .sprite_mgr
                .scenery_objects
                .media
                .as_deref()
                .map_or(&[][..], |m| &m.effect_sprites[..]);
            let disturbances = gpu.ripple_sources.gather(&world, effects);
            water.step_ripples(&gpu.queue, &mut encoder, dt, &disturbances);
        }

        let mut myth2 = Myth2Gpu {
            device: &gpu.device,
            queue: &gpu.queue,
            terrain_renderer: &gpu.terrain_renderer,
            terrain_mesh: &gpu.terrain_mesh,
            model_renderer: &gpu.model_renderer,
            billboard_renderer: &mut gpu.billboard_renderer,
            sprite_mgr: &gpu.sprite_mgr,
            model_instances: &gpu.model_instances,
            model_meshes: &gpu.model_meshes,
            category2_batch: &mut gpu.category2_batch,
            reflection_batch: &mut gpu.reflection_batch,
            translucent_batch: &mut gpu.translucent_batch,
            reflection_edge: gpu.reflection_edge.as_ref(),
        };
        let lighting = &self.lighting;
        let scene = Myth2World::prepare(
            &Myth2View {
                mesh_tag: &self.mesh_tag,
                sim: &self.sim,
                world: &world,
                camera: &self.render_camera,
                size: window,
                bars: view_bars(&self.ui),
                animator: &self.unit_animator,
                interface: &self.interface,
                local_time,
                band: self.selection_machine.band_bounds(),
                gesture: self.move_gesture.as_ref(),
                markers: &self.local_sprites,
                particles: self.particle_field.as_ref(),
                local_projectiles: self.local_projectiles.as_ref(),
                local_overlays: LocalOverlays::InRegion,
                sampling: self.sampling,
                sprite_filter: self.sprite_filter,
                fog: (&self.fog, self.fog_seconds),
                lighting: Some(Myth2Lighting {
                    ambient_tint: lighting.ambient_tint,
                    enabled: lighting.enabled,
                    intensity: lighting.light_intensity,
                    static_lights: &gpu.static_lights,
                }),
            },
            &mut myth2,
            (active_billboards, moving_shadows),
            &mut self.lightning_random,
            &mut clock,
            |moving_shadows| {
                upload_landscape_changes(
                    LandscapeChangeContext {
                        device: &gpu.device,
                        queue: &gpu.queue,
                        sprite_mgr: &gpu.sprite_mgr,
                        model_instances: &gpu.model_instances,
                        static_shadows: &gpu.static_shadows,
                        landscape: &mut gpu.landscape,
                        scenery_shadow_rects: &mut gpu.scenery_shadow_rects,
                        landscape_edits_applied: &mut gpu.landscape_edits_applied,
                        stain_random: &mut gpu.stain_random,
                        moving_shadows: &mut gpu.moving_shadows,
                    },
                    &self.landscape_edits,
                    moving_shadows,
                    &mut encoder,
                );
            },
        );
        let (region_view, view_region) = (scene.region_view.as_ref(), scene.view_region.as_ref());

        let skips = self.skip_passes;

        let timer = gpu.gpu_timer.as_ref();
        if let Some(timer) = timer {
            timer.mark(&mut encoder, Mark::Start);
        }
        let interface_view = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(myth2.billboard_renderer.interface_format),
            ..Default::default()
        });
        scene.encode(
            &myth2,
            &mut encoder,
            &WorldTargets {
                color: &view,
                interface: &interface_view,
                depth: &gpu.depth_texture_view,
            },
            gpu.landscape.as_ref().map(|(_, t)| &t.bind_group),
            gpu.water.as_ref(),
            WorldPasses {
                rule: PassRule::Myth2,
                clear: wgpu::Color {
                    r: 0.12,
                    g: 0.16,
                    b: 0.24,
                    a: 1.0,
                },
                skip: skips.world,
            },
            timer,
        );
        let world_readback = self
            .snapshot_wanted
            .is_some()
            .then(|| Readback::encode(&gpu.device, &mut encoder, target));

        if let (Some(hud), world) = (self.hud.as_mut(), &self.sim.engine) {
            let ids = &self.selected_unit_ids;
            if self.hud_selection.as_ref() != Some(ids) {
                hud.set_selection(world, ids);
                self.hud_selection = Some(ids.clone());
            }
        }
        if let Some(hud) = self.hud.as_mut().filter(|_| !skips.interface) {
            hud.set_paused(self.sim_paused && self.hints_open != Some(true));
            hud.update_console(media::local_time(self.sim.current_tick));
            let keyed = self.scores.toggled
                || self
                    .controls
                    .is_held(crate::controls::ControlAction::ShowScores, &self.held_keys)
                || (self.caps_lock_key.is_down()
                    && self.controls.options.caps_lock == crate::controls::CapsLock::PlayerStats);
            hud.update_scores(&self.sim, &self.film_names, keyed);
            let keys = self.controls.is_held(
                crate::controls::ControlAction::ShowKeyBindings,
                &self.held_keys,
            );
            hud.set_key_bindings(keys.then_some(&self.controls));
            gpu.hud_images
                .update(&gpu.overlay, &gpu.device, &gpu.queue, hud);
            let now_ms = self.machine_clock.elapsed().as_millis() as i32;
            self.detach_menu
                .follow_mode(hud.console.selection_mode, now_ms);
            let menu = self
                .detach_menu
                .shown(now_ms)
                .filter(|m| m.0 == detach::MENU_TEAMMATES);
            let w = &self.sim.engine;
            if let (Some(_), Some(stats), Some(tags)) =
                (menu, self.sim.statistics(), hud.prompt_tags())
            {
                gpu.detach_menu_images.update(
                    &gpu.overlay,
                    &gpu.device,
                    &gpu.queue,
                    tags,
                    detach::menu_rows(w, stats.statistics),
                    hud.view_frame(),
                );
            }
            let ui = self.ui;
            let (size, origin) = world_frame(ui.logical.0, ui.logical.1);
            let map_frame = self.minimap.as_mut().and_then(|mm| {
                mm.update(&world, media::local_time(self.sim.current_tick), size);
                let outline = minimap_outline(
                    &self.render_camera,
                    &self.mesh_tag,
                    gpu.sprite_mgr.scenery_objects.media.as_deref(),
                    region_view.zip(view_region),
                );
                gpu.minimap_images
                    .update(&gpu.overlay, &gpu.device, &gpu.queue, &mm.map);
                mm.map.draws(&outline, origin)
            });
            let mut draws = Vec::with_capacity(64);
            if let Some(frame) = map_frame.as_ref() {
                draws.extend(gpu.minimap_images.draws(frame));
            }
            draws.extend(gpu.hud_images.console_draws());
            if let Some((_, band, text)) = menu {
                draws.extend(gpu.detach_menu_images.draws(band, text));
            }
            draws.extend(gpu.hud_images.score_draws());
            draws.extend(gpu.hud_images.fps_draws());
            draws.extend(gpu.hud_images.key_binding_draws());
            draws.extend(gpu.hud_images.draws(hud, self.translucent_bars));
            encode_bars(
                BarTargets {
                    device: &gpu.device,
                    queue: &gpu.queue,
                    overlay: &mut gpu.overlay,
                    scaler: &mut gpu.interface_scaler,
                    view: &interface_view,
                    window,
                },
                &mut encoder,
                (&ui, self.ui_mode.filter(self.ui_filter).into()),
                &draws,
                &mut None,
            );
        }

        if let Some(front_end) = front_end {
            draw_dialogs(
                DialogTargets {
                    device: &gpu.device,
                    queue: &gpu.queue,
                    scaler: &mut gpu.dialog_scaler,
                    view: &interface_view,
                    window,
                    format: gpu.billboard_renderer.interface_format,
                },
                &mut encoder,
                (
                    self.ui.dialogs(self.dialog_scale),
                    self.ui_mode.filter(self.ui_filter).into(),
                ),
                front_end,
            );
        }

        let end_fade = self.map_end.as_ref().map_or(0.0, |end| end.alpha());
        let fade = end_fade.max(self.shell_fade);
        if fade > 0.0 && !skips.fade {
            gpu.screen_fade.encode(&mut encoder, &interface_view, fade);
        }

        window_surface::present_canvas(canvas, &mut encoder, &frame);
        if let Some(timer) = gpu.gpu_timer.as_mut() {
            timer.mark(&mut encoder, Mark::Interface);
            timer.encode_resolve(&gpu.device, &mut encoder);
        }

        if let Some((tick, frames)) = &mut self.hold_at_tick
            && self.sim.current_tick >= *tick
            && *frames <= HOLD_FRAMES
        {
            *frames += 1;
            if *frames == HOLD_FRAMES {
                info!("held at tick {}", self.sim.current_tick);
                self.screenshot_requested = cfg!(not(target_arch = "wasm32"));
            }
        }
        let readback = std::mem::take(&mut self.screenshot_requested)
            .then(|| Readback::encode(&gpu.device, &mut encoder, &frame.texture));
        let command_buffer = encoder.finish();
        lap(&mut clock, Lap::Encode);
        let submission = gpu.queue.submit(std::iter::once(command_buffer));
        lap(&mut clock, Lap::Submit);
        if self.frame_budget.as_ref().is_some_and(|b| b.wait_for_gpu()) {
            let _ = gpu.device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: None,
            });
            lap(&mut clock, Lap::GpuWait);
        }
        gpu.queue.present(frame);
        lap(&mut clock, Lap::Present);
        if let Some(end) = self.map_end.as_mut() {
            end.frame_presented(end_fade);
        }
        let billboard_count = scene.frame.billboards.len();
        let mut billboards = scene.frame.billboards;
        billboards.clear();
        gpu.billboard_scratch = billboards;
        if let (Some(budget), Some(clock)) = (self.frame_budget.as_mut(), clock.take()) {
            let scene = budget_scene(
                &self.sim,
                billboard_count,
                gpu.model_instances
                    .iter()
                    .filter(|i| i.in_region.get())
                    .count(),
            );
            budget.end_frame(clock, scene, gpu.gpu_timer.as_mut(), &gpu.device);
        }
        if let Some(hud) = self.hud.as_mut() {
            hud.fps
                .frame_presented(self.machine_clock.elapsed().as_millis() as u32);
        }
        let world_pixels =
            world_readback.map(|copy| copy.and_then(|copy| copy.read(&gpu.device).ok()));
        let pixels = readback.map(|copy| {
            copy.ok_or_else(|| anyhow::anyhow!("this window's surface can't be read"))
                .and_then(|copy| copy.read(&gpu.device))
        });
        match pixels {
            Some(Ok((size, pixels))) => self.save_screenshot(size, &pixels),
            Some(Err(e)) => self.screenshot_failed(&e),
            None => {}
        }
        if let Some(read) = world_pixels {
            let rect = self.world_rect(window);
            let snapshot = read.map(|(size, pixels)| {
                super::save_game::snapshot_of_frame(size, &pixels.to_rgba8(), rect)
            });
            self.snapshot_drawn(snapshot);
        }
    }

    fn save_screenshot(&mut self, size: (u32, u32), pixels: &screenshot::Pixels) {
        let dir = screenshot::user_screenshot_dir();
        let info = self.shot_info(size);
        match screenshot::write_shot(&dir, &info, &screenshot::time_of_day(), size, pixels) {
            Ok(path) => {
                info!("Screenshot saved to {}", path.display());
                self.print_line(&screenshot::saved_line(&path));
            }
            Err(e) => self.screenshot_failed(&e),
        }
    }

    fn screenshot_failed(&mut self, error: &anyhow::Error) {
        warn!("Screenshot failed: {error:#}");
        self.print_line(&screenshot::failed_line(error));
    }

    fn shot_info(&self, size: (u32, u32)) -> screenshot::ShotInfo {
        let argv: Vec<String> = std::env::args().collect();
        let observer = self
            .camera_control
            .observer(&self.observer, self.camera_frame());
        let camera = &self.render_camera;
        let subtitle = self
            .hud
            .as_ref()
            .and_then(|hud| screenshot::newest_subtitle(hud.console_lines.lines()));
        let ui_scale = match self.ui_mode {
            ui_scale::UiScaleMode::Native => None,
            ui_scale::UiScaleMode::Lines768 => Some("768p".to_owned()),
            ui_scale::UiScaleMode::Zoom(percent) => Some(percent.to_string()),
        };
        screenshot::ShotInfo {
            film: self.replay_path.clone(),
            mesh: self.mesh_name.clone(),
            mesh_id: self.mesh_tag.mesh_id.to_string(),
            difficulty: Some(self.sim.engine.difficulty),
            tick: self.sim.current_tick,
            subtitle,
            size,
            observer: [
                observer.x,
                observer.y,
                observer.z,
                observer.facing,
                observer.pitch,
                observer.zoom,
            ],
            view_camera: [
                camera.target[0],
                camera.target[1],
                camera.target[2],
                camera.distance,
                camera.yaw,
                camera.pitch,
            ],
            game_speed: self.game_speed.speed(),
            paused: self.sim_paused,
            selection: self.selected_unit_ids.clone(),
            fraction: self.unit_animator.fraction(),
            sampling: self
                .sampling
                .to_possible_value()
                .map_or_else(String::new, |v| v.get_name().to_owned()),
            fog: self.fog.mode(),
            fog_seconds: self.fog_seconds,
            reflection_soft_edge: self.reflection_soft_edge,
            hover: self.interface.hovered.map(|_| self.current_cursor_pos),
            caps_lock: self.interface.caps_lock.is_some(),
            overhead_map: self
                .minimap
                .as_ref()
                .is_some_and(|mm| mm.map.drawn() && mm.map.state != minimap::MapState::FadingOut),
            ui_scale,
            argv,
            cwd: std::env::current_dir().ok(),
        }
    }
}

pub(crate) struct DialogTargets<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) scaler: &'a mut mandrake_render::InterfaceScaler,
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) window: (u32, u32),
    pub(crate) format: wgpu::TextureFormat,
}

pub(crate) fn draw_dialogs(
    to: DialogTargets<'_>,
    encoder: &mut wgpu::CommandEncoder,
    (dialogs, filter): (
        ui_scale::UiScale,
        mandrake_view::interface_picture::InterfaceFilter,
    ),
    front_end: &mut dyn DrawInterface,
) {
    let DialogTargets {
        device,
        queue,
        scaler,
        view,
        window,
        format,
    } = to;
    if !(dialogs.is_scaled() && dialogs.window == window) {
        front_end.draw(&mut InterfaceFrame {
            device,
            queue,
            encoder,
            target: view,
            format,
            size: window,
        });
        return;
    }
    let layer = scaler.layer_view(device, dialogs.logical);
    clear_target(
        encoder,
        layer,
        wgpu::Color::TRANSPARENT,
        "Dialog Layer Clear",
    );
    front_end.draw(&mut InterfaceFrame {
        device,
        queue,
        encoder,
        target: layer,
        format,
        size: dialogs.logical,
    });
    scaler.composite(queue, encoder, view, window, filter);
}

fn scenery_billboards_for_frame(
    gpu: &mut GpuWindowState,
    mesh_tag: &mandrake_tag::MeshTag,
    local_time: i32,
    eye: [f32; 3],
    fraction: i32,
) -> (Vec<BillboardInstance>, Vec<ObjectShadow>) {
    let mut active_billboards = std::mem::take(&mut gpu.billboard_scratch);
    active_billboards.clear();
    active_billboards.extend_from_slice(&gpu.scenery_billboards);
    if let Some(map_media) = gpu.sprite_mgr.scenery_objects.media.as_deref() {
        apply_reflection_ripples(
            &mut active_billboards,
            &gpu.scenery_ripples,
            map_media,
            local_time,
        );
    }
    let (mut dynamic_scenery, dynamic_ripples, moving_shadows) = generate_scenery_billboards(
        mesh_tag,
        &gpu.sprite_mgr,
        local_time,
        SceneryView::new(mesh_tag, eye, &gpu.scenery_animator, fraction),
        SceneryPart::Dynamic,
    );
    if let Some(map_media) = gpu.sprite_mgr.scenery_objects.media.as_deref() {
        apply_reflection_ripples(
            &mut dynamic_scenery,
            &dynamic_ripples,
            map_media,
            local_time,
        );
    }
    active_billboards.extend(dynamic_scenery);
    (active_billboards, moving_shadows)
}

struct LandscapeChangeContext<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    sprite_mgr: &'a crate::sprite_manager::ClientSpriteManager,
    model_instances: &'a [crate::model_instances::ModelInstance],
    static_shadows: &'a [ObjectShadow],
    landscape: &'a mut Option<(
        mandrake_view::landscape::LandscapeTexels,
        mandrake_render::LandscapeTexture,
    )>,
    scenery_shadow_rects: &'a mut Vec<PixelRect>,
    landscape_edits_applied: &'a mut usize,
    stain_random: &'a mut mandrake_tag::sound_manager::LocalRandom,
    moving_shadows: &'a mut Vec<ObjectShadow>,
}

fn upload_landscape_changes(
    ctx: LandscapeChangeContext<'_>,
    landscape_edits: &[crate::corpse::LandscapeEdit],
    moving_shadows: Vec<ObjectShadow>,
    encoder: &mut wgpu::CommandEncoder,
) {
    let shadows_changed = *ctx.moving_shadows != moving_shadows;
    if let Some((texels, texture)) = ctx.landscape {
        let mut dirty = std::mem::take(ctx.scenery_shadow_rects);
        for edit in &landscape_edits[*ctx.landscape_edits_applied..] {
            dirty.extend(apply_landscape_edit(
                ctx.sprite_mgr,
                texels,
                edit,
                ctx.stain_random,
            ));
        }
        *ctx.landscape_edits_applied = landscape_edits.len();
        let moving = if shadows_changed {
            let shown: Vec<ShadeOverlay<'_>> = ctx
                .moving_shadows
                .iter()
                .map(ObjectShadow::overlay)
                .collect();
            let moving: Vec<ShadeOverlay<'_>> =
                moving_shadows.iter().map(ObjectShadow::overlay).collect();
            dirty.extend(changed_overlay_rects(
                &shown,
                &moving,
                texels.width,
                texels.height,
            ));
            Some(moving)
        } else {
            None
        };
        dirty.extend(changed_animating_shadow_rects(
            ctx.model_instances,
            texels.width,
            texels.height,
        ));
        if !dirty.is_empty() {
            let moving = moving
                .unwrap_or_else(|| moving_shadows.iter().map(ObjectShadow::overlay).collect());
            let overlays: Vec<ShadeOverlay<'_>> = ctx
                .static_shadows
                .iter()
                .map(ObjectShadow::overlay)
                .chain(moving.iter().copied())
                .chain(animating_shadow_overlays(ctx.model_instances))
                .collect();
            let mut upload = LandscapeUpload::default();
            texels.rebuild_regions(&mut dirty, &overlays, |rect, rgba| upload.add(rect, rgba));
            upload.encode(ctx.device, ctx.queue, encoder, &texture.texture);
        }
        dirty.clear();
        *ctx.scenery_shadow_rects = dirty;
    }
    if shadows_changed {
        *ctx.moving_shadows = moving_shadows;
    }
}
