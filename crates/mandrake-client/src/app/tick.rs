use crate::app::ClientApp;
use crate::carnage_report::EndStage;
use crate::clock::Instant;
use crate::controls::ControlAction;
use crate::effects::{build_ambient_local_projectiles, update_particle_field};
use crate::model_instances::update_model_animations;
use crate::region_view::view_bars;
use crate::scenery_billboards::refresh_static_scenery;
use crate::{camera_control, film_report, local_input, media, object_history, unit_info};
use tracing::{debug, info, warn};

const TITLE_TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

fn overhead_map_state(state: crate::minimap::MapState) -> i16 {
    use crate::minimap::MapState;
    match state {
        MapState::Hidden => 0,
        MapState::FadingIn => 1,
        MapState::Shown => 2,
        MapState::FadingOut => 3,
    }
}

pub(crate) fn window_title(mesh_name: &str, film_tick: Option<u32>) -> String {
    match film_tick {
        Some(tick) => format!("Myth II: Soulblighter - {mesh_name} - tick {tick}"),
        None => format!("Myth II: Soulblighter - {mesh_name}"),
    }
}

impl ClientApp {
    fn log_finished_orders(&mut self) {
        let terrain = self.sim.engine_terrain.as_ref();
        self.walking_orders.retain(|id| {
            let Some(m) = self.sim.units.get(id).and_then(|u| u.engine.as_ref()) else {
                return false;
            };
            if self.sim.unit_is_moving(*id) {
                return true;
            }
            let (kind, closed) = terrain.map_or((None, None), |c| {
                let t = c.terrain_type(m.x, m.y);
                (Some(t), Some(m.definition.impassable >> t & 1 != 0))
            });
            info!(
                "Unit {id} '{}' ended its move order at world ({}, {}): terrain type {kind:?}, \
                 closed to it: {closed:?}",
                m.definition.tag, m.x, m.y
            );
            false
        });
    }

    pub(crate) fn log_local_sprites(&self) {
        let seqs: Vec<i16> = self
            .local_sprites
            .entries()
            .iter()
            .map(|s| s.sequence)
            .collect();
        info!(
            "Local sprites: {} (sequences {seqs:?}), timer {:?}",
            seqs.len(),
            self.local_sprites.timer()
        );
        for s in self.local_sprites.entries() {
            debug!("  local sprite seq {} at {:?}", s.sequence, s.point);
        }
    }

    pub(crate) fn title(&self) -> String {
        let tick = self.replay.as_ref().map(|_| self.sim.current_tick);
        window_title(&self.mesh_name, tick)
    }

    pub(crate) fn update_title_tick(&mut self) {
        let tick = self.sim.current_tick;
        if self.replay.is_none()
            || self.title_tick == Some(tick)
            || self.title_updated.elapsed() < TITLE_TICK_INTERVAL
        {
            return;
        }
        if let Some(window) = &self.window {
            crate::app::shell::set_window_title(window, &self.title());
            self.title_tick = Some(tick);
            self.title_updated = Instant::now();
        }
    }

    fn apply_force_end(&mut self) {
        let Some(force) = self.force_end.filter(|f| self.sim.current_tick >= f.tick) else {
            return;
        };
        self.force_end = None;
        let tick = self.sim.current_tick as i32;
        let world = &mut self.sim.engine;
        world.game.winner = if force.win { 0 } else { 1 };
        if let Err(e) = world.game.end_game(0, tick, &mut world.game_options) {
            warn!("--force-end: {e}");
        }
        if force.win {
            world.game.end_code = 1;
        }
        info!("--force-end: {force:?} at tick {tick}");
    }

    fn follow_renamed_units(&mut self) {
        let renamed = self.unit_animator.renamed().to_vec();
        if renamed.is_empty() {
            return;
        }
        crate::unit_anim::follow_renamed(&mut self.selected_unit_ids, &renamed);
        if let Some(ids) = self.hud_selection.as_mut() {
            crate::unit_anim::follow_renamed(ids, &renamed);
        }
        crate::unit_anim::follow_renamed(&mut self.interface.order_hide.units, &renamed);
        let mut orders: Vec<u32> = self.walking_orders.iter().copied().collect();
        crate::unit_anim::follow_renamed(&mut orders, &renamed);
        self.walking_orders = orders.into_iter().collect();
        if let Some(gpu) = self.gpu_state.as_ref() {
            let mut media = gpu.sprite_mgr.unit_media.borrow_mut();
            for &(old, new) in &renamed {
                media.remove(&new);
                if let Some(m) = media.remove(&old) {
                    media.insert(new, m);
                }
            }
        }
        info!("Units re-keyed by a panic: {renamed:?}");
    }

    fn set_overhead_map_state(&mut self) {
        let state = self
            .minimap
            .as_ref()
            .map_or(0, |mm| overhead_map_state(mm.map.state));
        if let Some(actions) = self.sim.engine.map_actions.as_mut() {
            actions.overhead_map_state = state;
        }
    }

    fn game_tick(&mut self) {
        self.apply_force_end();
        self.inject_test_stop();
        if let Some(mm) = self.minimap.as_mut() {
            mm.before_tick(
                self.sim.current_tick,
                crate::world_view::myth2::minimap_view(&self.sim).as_ref(),
            );
        }
        if let Some(local) = self.local_projectiles.as_mut() {
            local.record_history();
        }
        self.set_overhead_map_state();
        let detach = super::detach::detach_this_tick(&self.sim);
        film_report::step(&mut self.sim);
        if detach {
            super::detach::log_owners(&self.sim);
        }
        self.camera_after_tick(self.sim.current_tick as i32 - 1);
        self.follow_unit();
        self.play_map_action_sounds();
        self.unit_animator.observe(&self.sim);
        self.follow_renamed_units();
        self.log_finished_orders();
        self.play_combat_events();
        if let Some(gpu) = self.gpu_state.as_ref() {
            self.unit_animator
                .tick_units(self.sim.units.values(), &gpu.sprite_mgr.unit_visuals);
        }
        if let Some(gpu) = self.gpu_state.as_mut() {
            let update = update_model_animations(
                &mut gpu.model_instances,
                &mut self.sim,
                &mut gpu.sprite_mgr.scenery_objects.media,
            );
            self.pending_animation_sounds.extend(update.sounds);
            let cells_released = crate::model_effects::follow_sim_model_effects(
                &mut gpu.model_instances,
                &mut gpu.model_meshes,
                &self.sim,
                &mut crate::model_effects::EffectMeshes {
                    device: &gpu.device,
                    queue: &gpu.queue,
                    renderer: &gpu.model_renderer,
                    source: crate::model_instances::ModelTagSource {
                        small: &self.archive,
                        large: self.large_archive.as_ref(),
                    },
                    cache: &mut gpu.model_effect_meshes,
                },
                &mut gpu.sprite_mgr.scenery_objects.media,
            );
            let ripple = gpu.mesh_ripple.follow(&self.mesh_tag, &self.sim.engine);
            let sine = &mandrake_math::engine::trig_tables().sine;
            let mut rebuild = update.landscape_changed || cells_released;
            if !rebuild && (ripple.lod_changed || !ripple.cells.is_empty()) {
                let start = Instant::now();
                let heights = gpu
                    .terrain_mesh
                    .apply_cell_heights(&gpu.queue, &ripple.cells, sine);
                let words = if ripple.lod_changed {
                    let upload = gpu.terrain_mesh.apply_lod_words(
                        &gpu.device,
                        &gpu.queue,
                        gpu.mesh_ripple.lod(),
                        sine,
                    );
                    rebuild = upload.is_none();
                    upload
                } else {
                    None
                };
                if let Some(water) = gpu.water.as_mut() {
                    water.update_terrain(
                        &gpu.device,
                        &gpu.queue,
                        gpu.mesh_ripple.mesh(&self.mesh_tag),
                        &gpu.terrain_mesh,
                    );
                }
                tracing::debug!(
                    target: "mesh_ripple",
                    "tick {}: {} cells moved, {heights:?}, LOD words {}{words:?} in {:?}",
                    self.sim.current_tick,
                    ripple.cells.len(),
                    if ripple.lod_changed { "changed " } else { "kept" },
                    start.elapsed()
                );
            }
            if rebuild {
                gpu.terrain_mesh = crate::map_load::build_terrain_mesh(
                    &gpu.device,
                    gpu.mesh_ripple.mesh(&self.mesh_tag),
                    1.0,
                    gpu.sprite_mgr.scenery_objects.media.as_deref(),
                    self.landscape_detail.as_deref(),
                    gpu.mesh_ripple.lod(),
                );
                let mut encoder =
                    gpu.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Rebuild Media Update Encoder"),
                        });
                gpu.terrain_mesh.update_media(
                    &gpu.device,
                    &gpu.queue,
                    &mut encoder,
                    media::local_time(self.sim.current_tick),
                    sine,
                );
                gpu.queue.submit(std::iter::once(encoder.finish()));
                if let Some(water) = gpu.water.as_mut() {
                    water.update_terrain(
                        &gpu.device,
                        &gpu.queue,
                        gpu.mesh_ripple.mesh(&self.mesh_tag),
                        &gpu.terrain_mesh,
                    );
                }
            }
            gpu.scenery_animator.tick(&gpu.sprite_mgr.scenery_visuals);
            if gpu
                .sprite_mgr
                .scenery_objects
                .follow(crate::world_view::myth2::scenery_view(&self.sim.engine))
            {
                refresh_static_scenery(
                    gpu,
                    &self.mesh_tag,
                    &self.connectors,
                    &self.sim,
                    media::local_time(self.sim.current_tick),
                    self.render_camera.eye_position(),
                );
            }
        }
        self.take_status_bar_flags();
    }

    pub(super) fn take_status_bar_flags(&mut self) {
        if self.selected_unit_ids.is_empty() {
            return;
        }
        let selected = unit_info::selected_monsters(&self.sim.engine, &self.selected_unit_ids);
        if self.sim.engine.take_selection_flags(&selected) {
            self.hud_selection = None;
        }
    }

    fn inject_test_stop(&mut self) {
        let Some(at) = self.test_stop_at.filter(|&t| self.sim.current_tick >= t) else {
            return;
        };
        self.test_stop_at = None;
        self.sim
            .engine_stop
            .get_or_insert(mandrake_sim::engine_rng::Unmodelled::new(
                0,
                format!("a stop injected at tick {at} (M2_TEST_STOP_AT_TICK)"),
            ));
    }

    fn after_ticks(&mut self, game_ticks: i32) {
        if game_ticks > 0 {
            self.checkpoint_recording();
        }
        self.report_stop();
        if game_ticks > 0
            && let Some(local) = self.local_projectiles.as_mut()
        {
            let engine = self.unit_animator.engine_model();
            let visible = engine.map(|e| move |x, y| e.cell_bits(x, y));
            local.update_from_sim(
                game_ticks,
                &self.sim,
                (
                    i32::from(self.mesh_tag.cell_width),
                    i32::from(self.mesh_tag.cell_height),
                ),
                visible.as_ref().map(|v| v as &dyn Fn(i32, i32) -> u32),
            );
            self.landscape_edits.extend(
                local
                    .take_stains()
                    .into_iter()
                    .map(crate::corpse::LandscapeEdit::Stain),
            );
        }
    }

    pub(crate) fn restoring(&self) -> bool {
        self.restore.is_some()
    }

    pub(crate) fn restore_step(&mut self, budget: std::time::Duration) -> (u32, bool) {
        let Some(end) = self.restore.as_ref().map(|r| r.end_tick) else {
            return (self.sim.current_tick, true);
        };
        let started = Instant::now();
        let audio = self.audio.take();
        let mut ticks = 0;
        let stopped =
            |sim: &mandrake_sim::Simulation| sim.ticks_stopped() || sim.engine_stop.is_some();
        loop {
            if self.sim.current_tick >= end && !self.restore_queued {
                self.queue_restore_commands();
            }
            let over = stopped(&self.sim) || (self.restore_queued && self.sim.save_loaded());
            if over || (ticks > 0 && started.elapsed() >= budget) {
                break;
            }
            self.game_tick();
            ticks += 1;
        }
        self.after_ticks(ticks);
        self.audio = audio;
        self.pending_animation_sounds.clear();
        let over = stopped(&self.sim) || (self.restore_queued && self.sim.save_loaded());
        (self.sim.current_tick, over)
    }

    fn queue_restore_commands(&mut self) {
        self.restore_queued = true;
        let observer = self
            .restore
            .as_ref()
            .and_then(|r| r.header.as_ref())
            .map(|h| mandrake_sim::save_restore::SaveLoaded {
                x: h.observer_x,
                y: h.observer_y,
                facing: h.observer_facing,
                zoom: h.observer_zoom,
            })
            .unwrap_or_default();
        self.sim.end_saved_game_replay(observer);
        self.sim.command_lead = mandrake_sim::LOCAL_COMMAND_LEAD;
        self.sim.halt_on_stop = true;
    }

    pub(crate) fn finish_restore(&mut self) {
        let Some(restore) = self.restore.take() else {
            return;
        };
        if !self.restore_queued {
            self.queue_restore_commands();
        }
        self.restore_queued = false;
        info!(
            "Restored to tick {} (the save's end: {}), seed {:#x}",
            self.sim.current_tick, restore.end_tick, self.sim.rng.state
        );
        if let Some(header) = restore.header {
            let count = usize::try_from(header.console_count).unwrap_or(0);
            let lines = header
                .console
                .iter()
                .take(count)
                .map(|l| crate::console::ConsoleLine {
                    player: l.kind,
                    stamp: 0,
                    text: l.text.clone(),
                })
                .collect();
            self.set_console_cache(lines);
        }
        if let Some(observer) = self.sim.save_loaded_observer.take() {
            self.set_saved_observer(observer);
        }
    }

    fn set_saved_observer(&mut self, observer: mandrake_sim::save_restore::SaveLoaded) {
        let location = camera_control::ObserverLocation {
            point: (observer.x, observer.y),
            facing: observer.facing as u16,
        };
        let zoom = observer.zoom as f32 / 65536.0;
        let frame = self.camera_frame();
        let interpolate = std::mem::replace(&mut self.camera_control.interpolate, false);
        self.camera_control.set_actual_observer_location_and_zoom(
            location,
            zoom,
            self.sim.current_tick as i32,
            &mut self.observer,
            frame,
        );
        self.camera_control.interpolate = interpolate;
        self.place_camera_at_observer();
        self.prev_camera_state = self.camera.current_state();
        self.render_camera = self.camera.clone();
    }

    pub(crate) fn tick_bodies(&mut self, polls: u32) -> i32 {
        let stopped = self.sim.ticks_stopped();
        let dialog_pause = self.sim_paused && self.net.is_none();
        let paused = dialog_pause || self.held_for_begin_play || stopped;
        if stopped {
            for _ in 0..polls {
                film_report::idle_poll(&self.sim);
            }
        }
        let ticks = self.game_speed.advance(polls, paused) as i32;
        match self.hold_at_tick {
            Some((tick, _)) => ticks.min(tick.saturating_sub(self.sim.current_tick) as i32),
            None => ticks,
        }
    }

    pub(crate) fn poll_network(&mut self) {
        let Some(net) = self.net.as_mut() else {
            return;
        };
        if !self.session_map && !net.is_loaded() {
            net.map_loaded();
        }
        let news = net.poll(&mut self.sim);
        if news.started {
            self.last_tick_time = Instant::now();
            self.physics_accumulator = 0.0;
        }
        if news.host_lost && !net.ended {
            self.net_console_line(crate::net_game::STRING_HOST_LOST, usize::MAX);
        }
    }

    fn show_system_events(&mut self) {
        if let Some(index) = self.sim.net_game_speed() {
            self.game_speed.set_network_index(index);
        }
        for event in self.sim.take_system_events() {
            match event {
                mandrake_sim::net_game::SystemEvent::Paused { paused, originator } => {
                    info!(
                        "net: {} at tick {}",
                        if paused { "paused" } else { "resumed" },
                        self.sim.current_tick
                    );
                    if !originator {
                        info!(
                            "console warning: {}",
                            if paused { "SERVER PAUSED" } else { "" }
                        );
                    }
                }
                mandrake_sim::net_game::SystemEvent::Speed { index, originator } => {
                    info!(
                        "net: game speed index {index} at tick {}",
                        self.sim.current_tick
                    );
                    if !originator && let Some(hud) = self.hud.as_mut() {
                        hud.show_speed_popup(index, self.game_speed.polls());
                    }
                }
            }
        }
    }

    fn net_console_line(&mut self, index: usize, player: usize) {
        let name = self
            .film_names
            .get(player)
            .map(|n| n.0.0.clone())
            .unwrap_or_default();
        let line = crate::net_game::networking_string(&self.net_strings, index, &name);
        info!("console: {}", String::from_utf8_lossy(&line));
        let now = media::local_time(self.sim.current_tick + 1);
        if let Some(hud) = self.hud.as_mut() {
            hud.console_lines.cputs(-1, &line, now);
        }
    }

    fn chat_line(&mut self, tick: u32, sender: i16, flags: u16, text: &[u8]) {
        let (shown, sound) = crate::net_game::chat_shown(&self.sim, sender, flags);
        info!(
            "chat from player {sender} (flags {flags}) at tick {tick}: {}{}",
            String::from_utf8_lossy(text),
            if shown {
                ""
            } else {
                " (not shown: another team's)"
            }
        );
        if !shown {
            return;
        }
        let now = media::local_time(self.sim.current_tick + 1);
        if let Some(hud) = self.hud.as_mut() {
            hud.append_chat(sender, text, flags & 1 != 0, now);
        }
        if let (Some(sound), w) = (sound, &mut self.sim.engine) {
            w.game.game_sound_requests.push(sound);
        }
    }

    fn alliance_lines(&mut self) {
        let w = &mut self.sim.engine;
        let changes = std::mem::take(&mut w.alliance_changes);
        if changes.is_empty() {
            return;
        }
        let (local, mode) = (w.local_team(), w.mode);
        let teams = w.players.teams.clone();
        let now = media::local_time(self.sim.current_tick + 1);
        for c in changes {
            info!(
                "alliance at tick {}: team {} {} team {}",
                self.sim.current_tick,
                c.team,
                if c.allied {
                    "allied with"
                } else {
                    "broke with"
                },
                c.other
            );
            let Some(hud) = self.hud.as_mut() else {
                continue;
            };
            let name = |t| crate::console::team_display_name(&teams, &self.film_names, t);
            if let Some(line) =
                crate::console::alliance_line(hud.alliance_strings(), &c, local, mode, name)
            {
                info!("console: {}", String::from_utf8_lossy(&line));
                hud.console_lines.cputs(-1, &line, now);
            }
        }
    }

    pub(crate) fn update_physics_and_simulation(&mut self) {
        self.poll_network();
        let muted = if self
            .net
            .as_ref()
            .is_some_and(|n| n.joining || n.restoring())
        {
            self.audio.take()
        } else {
            None
        };
        self.update_frame();
        if muted.is_some() {
            self.audio = muted;
            self.pending_animation_sounds.clear();
        }
    }

    fn update_frame(&mut self) {
        const TICK_INTERVAL: f32 = 1.0 / 30.0;

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick_time).as_secs_f32();
        self.last_tick_time = now;
        let dt = elapsed.min(0.25);
        self.physics_accumulator += dt;

        let polls = self.poll_input_steps(TICK_INTERVAL);
        let game_ticks = self.run_tick_bodies(polls);
        self.after_ticks(game_ticks);
        self.play_local_projectile_sounds();
        self.update_frame_effects(game_ticks);
        self.update_render_camera(TICK_INTERVAL);
        self.update_frame_sound(polls);
        self.update_map_end_and_stop_line();

        let frame = self.camera_frame();
        let now = self.sim.current_tick as i32;
        camera_control::shake_camera(
            &mut self.camera_control,
            &mut self.render_camera,
            frame,
            now,
        );
    }

    fn poll_input_steps(&mut self, tick_interval: f32) -> u32 {
        let mut polls: u32 = 0;
        while self.physics_accumulator >= tick_interval {
            self.prev_camera_state = self.camera.current_state();

            let polled = self.poll_action();
            let presses = std::mem::take(&mut self.pending_presses);
            let mut bound = [0u16; 64];
            let mut bound_len = 0;
            for id in ControlAction::ALL
                .iter()
                .filter(|&&a| self.controls.is_held(a, &self.held_keys))
                .filter_map(|a| a.engine_id())
                .chain(presses.iter().filter_map(|(a, _)| a.engine_id()))
            {
                if bound_len < bound.len() {
                    bound[bound_len] = id;
                    bound_len += 1;
                }
            }
            let bound = &bound[..bound_len];
            let accepted = local_input::sniff_polled_action(&mut self.sim, polled, bound);
            if accepted.is_none() {
                debug!(
                    "the tutorial sniffer cleared the action at t {} (flags {bound:?})",
                    polled.time
                );
            }
            self.step_observer(accepted.is_some());
            self.step_selection(accepted.unwrap_or_default());
            if accepted.is_some() {
                for (action, shift) in presses {
                    self.dispatch_press(action, shift);
                }
            }

            self.physics_accumulator -= tick_interval;
            polls += 1;
        }
        polls
    }

    fn run_tick_bodies(&mut self, polls: u32) -> i32 {
        let (game_ticks, mut frame) = match self.net.as_mut() {
            Some(net) => {
                let polls = if net.started { polls } else { 0 };
                let polls = net.catch_up_polls(&self.sim).unwrap_or(polls);
                (
                    0,
                    Some(net.frame(&mut self.sim, &mut self.game_speed, polls)),
                )
            }
            None => (self.tick_bodies(polls), None),
        };
        let gate = frame.map(|f| f.step);
        let mut ran = 0;
        loop {
            let more = match frame.as_mut() {
                Some(f) => f.next_body(&mut crate::net_game::SimNet(&mut self.sim)),
                None => ran < game_ticks,
            };
            if !more {
                break;
            }
            let dropped = if gate.is_some() {
                crate::net_game::dropped_players(&self.sim)
            } else {
                Vec::new()
            };
            let chats = crate::net_game::chats(&self.sim);
            let tick = self.sim.current_tick;
            self.game_tick();
            ran += 1;
            if let Some(net) = self.net.as_mut() {
                net.after_body(&mut self.sim);
            }
            for player in dropped {
                self.net_console_line(crate::net_game::STRING_PLAYER_LEFT, player);
            }
            for (sender, flags, text) in chats {
                self.chat_line(tick, sender, flags, &text);
            }
            self.alliance_lines();
        }
        if gate.is_some() {
            self.show_system_events();
        }
        if self.sim.is_network_game() && self.sim.game_over() {
            self.sim.end_recording();
        }
        if let (Some(step), Some(net)) = (gate, self.net.as_mut()) {
            if net.after_batch(&mut self.sim, step) {
                let focus = self.window.as_ref().map(|w| w.has_focus());
                info!(
                    "net: the window has the focus: {focus:?}, occluded: {}",
                    self.occluded
                );
            }
            if !net.forwards() && self.sim.engine.game.game_over {
                net.map_over();
            }
        }
        ran
    }

    fn update_frame_effects(&mut self, game_ticks: i32) {
        if let (Some(local), Some(gpu)) = (self.local_projectiles.as_mut(), self.gpu_state.as_ref())
        {
            build_ambient_local_projectiles(
                local,
                &self.sim,
                &self.mesh_tag,
                gpu.sprite_mgr.scenery_objects.media.as_deref(),
                &self.render_camera,
                (
                    (gpu.surface_config.width, gpu.surface_config.height),
                    view_bars(&self.ui),
                ),
                game_ticks,
            );
        }

        if game_ticks > 0
            && let Some(mut field) = self.particle_field.take()
        {
            let frame = self.camera_frame();
            let observer = self
                .camera_control
                .observer(&self.observer, frame)
                .location()
                .point;
            update_particle_field(&mut field, &self.sim, &self.mesh_tag, game_ticks, observer);
            self.particle_field = Some(field);
        }
    }

    fn update_render_camera(&mut self, tick_interval: f32) {
        let now_ms = self.machine_clock.elapsed().as_millis() as u32;
        let late_ms = (self.physics_accumulator * 1000.0) as u32;
        let fraction = self.camera_control.motion_fraction_from_due(
            self.sim.current_tick,
            now_ms,
            late_ms,
            self.game_speed.speed(),
        );
        self.unit_animator.set_glide(self.hold_at_tick.is_none());
        self.unit_animator
            .set_fraction(object_history::frame_fraction(fraction));
        if let Some(hud) = self.hud.as_mut() {
            hud.update_speed_popup(self.game_speed.polls());
        }

        if self.camera_control.user_camera_locked() {
            let now_ms = self.machine_clock.elapsed().as_millis() as u32;
            let frame = self.camera_frame();
            if self.camera_control.interpolate_observer(
                now_ms,
                self.game_speed.speed(),
                &mut self.observer,
                frame,
            ) {
                self.place_camera_at_observer();
            }
            self.prev_camera_state = self.camera.current_state();
            self.render_camera = self.camera.clone();
        } else {
            let alpha = (self.physics_accumulator / tick_interval).clamp(0.0, 1.0);
            self.render_camera = self.camera.interpolate(&self.prev_camera_state, alpha);
        }
    }

    fn update_frame_sound(&mut self, polls: u32) {
        if let Some(out) = &self.audio
            && polls > 0
            && let Some(map_sound) = self.map_sound.as_mut()
        {
            let listener = map_sound.listener(&self.render_camera);
            for (id, location) in std::mem::take(&mut self.pending_animation_sounds) {
                if let Some(index) = map_sound.manager.sound_index(id) {
                    map_sound.play(index, location, &listener, &out.audio);
                }
            }
            if let Some(field) = self.particle_field.as_ref() {
                map_sound.manager.particle_volume = field.ambient_volume();
            }
            map_sound.update(polls as i32, &listener, &out.audio);
        }
        if self.map_sound.is_none() {
            self.pending_animation_sounds.clear();
        }
    }

    fn update_map_end_and_stop_line(&mut self) {
        if let Some(end) = self.map_end.as_mut() {
            end.update(
                EndStage::of(&self.sim),
                self.machine_clock.elapsed().as_secs_f64(),
            );
        }
        if let Some(hud) = self.hud.as_mut() {
            let key = (
                self.sim.engine_stop.is_some(),
                self.sim.game_over(),
                self.sim.current_tick,
                self.stop_report.is_some(),
            );
            if self.stop_line_key != Some(key) {
                self.stop_line_key = Some(key);
                hud.stop_line = crate::console::stop_text(&self.sim);
                if let (Some(line), Some(suffix)) =
                    (hud.stop_line.as_mut(), self.stop_report.as_ref())
                    && self.sim.engine_stop.is_some()
                {
                    line.extend_from_slice(suffix.as_bytes());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::overhead_map_state;
    use crate::minimap::MapState;

    #[test]
    fn overhead_map_states_are_the_engines() {
        assert_eq!(overhead_map_state(MapState::Hidden), 0);
        assert_eq!(overhead_map_state(MapState::FadingIn), 1);
        assert_eq!(overhead_map_state(MapState::Shown), 2);
        assert_eq!(overhead_map_state(MapState::FadingOut), 3);
    }
}
