use crate::args::{self, Args};
use crate::effects::{
    build_ambient_local_projectiles, load_local_projectiles, load_particle_field,
    update_particle_field,
};
use crate::game_speed::{GAME_SPEEDS, NORMAL_SPEED};
use crate::headless::{
    HeadlessDrag, HeadlessInterface, HeadlessMarkers, HeadlessMouse, HeadlessPicks, UnitTickRecord,
    headless_move_click, headless_selection, parse_hud_click, parse_mouse_schedule,
    parse_move_click, parse_point, record_unit_tick,
};
use crate::headless_render::render_headless_frame;
use crate::interface_setup::{load_hud, load_minimap, world_frame};
use crate::region_view::view_bars;
use crate::startup::Startup;
use crate::{
    camera_control, command_bar, console, controls, corpse, film_report, hud, interface_state,
    map_sound, media, minimap, move_marker, object_history, projfx_log, ui_scale, unit_anim,
};
use mandrake_math::{Fixed16, Vec3Fixed};
use mandrake_tag::MarkerEntry;
use mandrake_view::camera::ObserverController;
use std::path::PathBuf;
use tracing::{info, warn};

pub(crate) fn run_headless(args: Args, start: Startup) -> anyhow::Result<()> {
    let Startup {
        interface,
        mesh_tag,
        mesh_data,
        terrain_texture,
        landscape_detail,
        archive,
        large_archive,
        connectors,
        film_plugin_names,
        film_names,
        initial_camera,
        start_state,
        interface_colors,
        interface_collection,
        mut scenery_objects,
        engine_idle,
        map_load_seed,
        mut sim,
        ..
    } = start;
    let film_plugins: Vec<&str> = film_plugin_names.iter().map(String::as_str).collect();
    let map_fog = crate::fog::MapFog::at_map_load_headless(&args, &mesh_tag);
    let output_path = args
        .screenshot
        .clone()
        .unwrap_or_else(|| PathBuf::from("myth2_rendered_terrain.png"));
    info!(
        "Executing headless GPU render ({}x{})...",
        args.width, args.height
    );

    if let Some(team) = args.select_team {
        for unit in sim.units.values_mut() {
            unit.selected = unit.team == team;
        }
    }
    if let Some(tag) = &args.select_unit {
        for unit in sim.units.values_mut() {
            unit.selected = unit.archetype.as_str() == tag.as_str();
        }
    }
    if let Some(ids) = &args.select_ids {
        for unit in sim.units.values_mut() {
            unit.selected = ids.contains(&unit.id);
        }
    }
    let move_clicks = args
        .move_click
        .iter()
        .map(|c| parse_move_click(c))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut local_sprites = move_marker::LocalSprites::new();
    let mut order_hide = interface_state::OrderFlagHide::default();
    let mut pick_up_target = mandrake_view::selection::state::TargetHighlight::default();
    let mut mouse = args
        .mouse
        .as_deref()
        .map(parse_mouse_schedule)
        .transpose()?
        .map(HeadlessMouse::new);
    let band_drag = match args.band_drag.as_deref() {
        None => None,
        Some(&[x0, y0, x1, y1]) => Some(HeadlessDrag {
            from: (x0, y0),
            to: (x1, y1),
            polls: args.band_drag_ticks.max(1),
        }),
        Some(v) => anyhow::bail!("--band-drag takes X0,Y0,X1,Y1, got {} values", v.len()),
    };
    for unit in sim.units.values().filter(|u| u.selected) {
        info!(
            "Selected unit {} '{}' (team {}) at ({:.2}, {:.2}, {:.2})",
            unit.id,
            unit.archetype,
            unit.team,
            unit.position.x.to_f32(),
            unit.position.y.to_f32(),
            unit.position.z.to_f32()
        );
    }

    if let Some(spec) = &args.place_unit {
        let [tag, x, y] = spec.as_slice() else {
            anyhow::bail!("--place-unit takes UNIT,X,Y, got {} values", spec.len());
        };
        let (x, y): (f32, f32) = (x.parse()?, y.parse()?);
        let (col, row) = (x / MarkerEntry::WORLD_ONE, y / MarkerEntry::WORLD_ONE);
        let render = [
            mesh_tag.cell_width as f32 * 0.5 - col,
            mesh_tag.terrain_height_at(col, row),
            row - mesh_tag.cell_height as f32 * 0.5,
        ];
        let unit = sim
            .units
            .values_mut()
            .filter(|u| u.archetype.as_str() == tag.as_str())
            .min_by_key(|u| u.id)
            .ok_or_else(|| anyhow::anyhow!("--place-unit: no '{tag}' unit"))?;
        unit.position = Vec3Fixed::new(
            Fixed16::from_f32(render[0]),
            Fixed16::from_f32(render[1]),
            Fixed16::from_f32(render[2]),
        );
        unit.waypoints.clear();
        if let Some(m) = unit.engine.as_mut() {
            m.x = x.round() as i32;
            m.y = y.round() as i32;
            if let Some(cells) = sim.engine_terrain.as_ref() {
                m.place(cells);
            }
        }
        info!(
            "Placed unit {} '{tag}' at world ({x}, {y}): render {render:?}",
            unit.id
        );
    }

    let mut unit_states: Vec<UnitTickRecord> = Vec::new();
    let mut engine_state = engine_idle.clone();
    let start_motions = unit_anim::unit_motions(&sim, engine_state.as_ref());
    let mut picks = mouse.as_ref().and_then(|_| {
        HeadlessPicks::new(
            &archive,
            large_archive.as_ref(),
            &mesh_tag,
            &connectors,
            &sim,
            (
                scenery_objects.media.clone(),
                engine_idle.clone(),
                start_motions.clone(),
            ),
        )
    });
    let ui_mode = args.ui_scale.unwrap_or_default();
    let ui = ui_scale::UiScale::new(ui_mode, args.width, args.height);
    if ui.is_scaled() {
        info!(
            "UI scale {:?} (not parity): logical screen {:?}, s = {}",
            ui_mode, ui.logical, ui.factor
        );
    }
    let (logical_w, logical_h) = ui.logical;
    let mut headless_minimap = if args.hud {
        load_minimap(
            &args.archive,
            &film_plugins,
            Some(&mesh_data),
            &mesh_tag,
            &sim,
            logical_w,
            logical_h,
        )
    } else {
        None
    };
    if let Some(mm) = headless_minimap.as_mut() {
        mm.update(
            &crate::world_view::myth2::units_view(&sim),
            media::local_time(0),
            world_frame(logical_w, logical_h).0,
        );
    }
    let hud_clicks = args
        .hud_click
        .iter()
        .map(|c| parse_hud_click(c))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut headless_hud = if args.hud {
        load_hud(
            &args.archive,
            &film_plugins,
            Some(&mesh_data),
            &mesh_tag,
            &sim,
            map_load_seed,
            ((logical_w, logical_h), interface),
        )
    } else {
        None
    };
    let camera_frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    let mesh_cells = (
        i32::from(mesh_tag.cell_width),
        i32::from(mesh_tag.cell_height),
    );
    let mut headless_observer = ObserverController::from_camera(&initial_camera);
    let mut camera_control = camera_control::CameraControl::new_for_map(
        sim.game_option_flags(),
        camera_control::INTERPOLATION_PREFERENCE,
    );
    if let Some(state) = start_state {
        camera_control.seed(state, &mut headless_observer, camera_frame);
    }
    let mut camera_runs = 0usize;
    if let Some(index) = args.trigger_obmo {
        let data = mesh_data.as_slice();
        warn!("--trigger-obmo with the engine model running: its own runs apply too");
        let run = camera_control::trigger_obmo(data, index)?;
        info!("--trigger-obmo {index}: {run:?}");
        camera_control.apply(&run, &headless_observer, camera_frame, mesh_cells);
        camera_runs += 1;
    }
    let mut particle_field = load_particle_field(
        &archive,
        large_archive.as_ref(),
        &mesh_tag,
        &sim,
        camera_control
            .observer(&headless_observer, camera_frame)
            .location()
            .point,
    );
    let mut local_projectiles =
        load_local_projectiles(&archive, large_archive.as_ref(), &sim, &mesh_tag);
    let ambient_camera = |observer: &ObserverController, moved: bool| {
        let mut camera = initial_camera.clone();
        if moved {
            let (half_w, half_d) = (camera_frame.half_width, camera_frame.half_height);
            observer.apply_to(&mut camera, |x, z| {
                mesh_tag.terrain_height_at(half_w - x, z + half_d)
            });
        }
        camera
    };
    if let Some(local) = local_projectiles.as_mut() {
        build_ambient_local_projectiles(
            local,
            &sim,
            &mesh_tag,
            scenery_objects.media.as_deref(),
            &ambient_camera(&headless_observer, camera_runs > 0),
            ((args.width, args.height), view_bars(&ui)),
            0,
        );
    }
    let mut landscape_edits = Vec::new();
    let mut mesh_ripple = crate::mesh_ripple::MeshRipple::new(
        scenery_objects
            .media
            .as_deref()
            .and_then(|m| m.lod.as_ref()),
    );
    let mut ticks_without_engine = 0;
    if args.ticks > 0 {
        info!("Advancing simulation by {} ticks...", args.ticks);
        if sim.engine_stop.is_some() {
            ticks_without_engine = args.ticks;
        }
        for tick in 1..=args.ticks {
            if let Some(mm) = headless_minimap.as_mut() {
                mm.before_tick(
                    sim.current_tick,
                    crate::world_view::myth2::minimap_view(&sim).as_ref(),
                );
            }
            if let Some(local) = local_projectiles.as_mut() {
                local.record_history();
            }
            film_report::step(&mut sim);
            if sim.engine_stop.is_some() && ticks_without_engine == 0 {
                ticks_without_engine = args.ticks - tick + 1;
            }
            for event in sim.drain_combat_events() {
                landscape_edits.extend(crate::world_view::myth2::landscape_edit(&event, &sim));
                if let Some(local) = local_projectiles.as_mut() {
                    local.start_for_event(&event);
                }
            }
            if let Some(local) = local_projectiles.as_mut() {
                let visible = engine_state.as_ref().map(|e| move |x, y| e.cell_bits(x, y));
                local.update_from_sim(
                    1,
                    &sim,
                    (
                        i32::from(mesh_tag.cell_width),
                        i32::from(mesh_tag.cell_height),
                    ),
                    visible.as_ref().map(|v| v as &dyn Fn(i32, i32) -> u32),
                );
                landscape_edits.extend(
                    local
                        .take_stains()
                        .into_iter()
                        .map(corpse::LandscapeEdit::Stain),
                );
            }
            {
                let world = &sim.engine;
                scenery_objects.follow(crate::world_view::myth2::scenery_view(world));
                mesh_ripple.follow(&mesh_tag, world);
            }
            let runs = camera_control.after_tick(
                &mut sim,
                tick as i32 - 1,
                &mut headless_observer,
                camera_frame,
                mesh_cells,
            );
            for run in &runs {
                info!("Camera action (obmo) at tick {}: {run:?}", run.time);
            }
            camera_runs += runs.len();
            if let Some(field) = particle_field.as_mut() {
                let observer = camera_control
                    .observer(&headless_observer, camera_frame)
                    .location()
                    .point;
                update_particle_field(field, &sim, &mesh_tag, 1, observer);
            }
            if camera_control.user_camera_locked() {
                let speed = GAME_SPEEDS[args.game_speed as usize];
                let now_ms = (u64::from(tick) * 1000 * u64::from(NORMAL_SPEED as u16)
                    / (30 * u64::from(speed.max(1) as u16))) as u32;
                camera_control.interpolate_observer(
                    now_ms,
                    speed,
                    &mut headless_observer,
                    camera_frame,
                );
            }
            if let Some(local) = local_projectiles.as_mut() {
                build_ambient_local_projectiles(
                    local,
                    &sim,
                    &mesh_tag,
                    scenery_objects.media.as_deref(),
                    &ambient_camera(&headless_observer, camera_runs > 0),
                    ((args.width, args.height), view_bars(&ui)),
                    1,
                );
            }
            let mut plays = console::take_sound_plays(&mut sim);
            {
                let world = &sim.engine;
                map_sound::resolve_nearest_visible(
                    &mut plays,
                    world,
                    headless_minimap.as_ref().map(|m| m.vis.visible.as_slice()),
                    None,
                );
            }
            if let Some(hud) = headless_hud.as_mut() {
                for text in hud.print_sound_plays(&plays) {
                    info!(
                        "Console line at tick {}: {:?}",
                        sim.current_tick - 1,
                        String::from_utf8_lossy(&text)
                    );
                }
            }
            if let Some(mm) = headless_minimap.as_mut() {
                let local = media::local_time(sim.current_tick);
                if args.minimap_toggle.contains(&tick) {
                    let shown = !mm.map.drawn() || mm.map.state == minimap::MapState::FadingOut;
                    mm.set_visible(shown, local);
                }
                mm.update(
                    &crate::world_view::myth2::units_view(&sim),
                    local,
                    world_frame(logical_w, logical_h).0,
                );
            }
            if tick == args.ticks {
                record_unit_tick(&sim, &mut engine_state, &mut unit_states);
                continue;
            }
            order_hide.update(1);
            pick_up_target.update(1);
            if let Some(mouse) = mouse.as_mut() {
                mouse.poll(
                    tick,
                    &mut sim,
                    &mesh_tag,
                    &initial_camera,
                    (args.width, args.height),
                    (scenery_objects.media.as_deref(), view_bars(&ui)),
                    picks.as_ref(),
                    args.select_ids.as_deref(),
                    &mut local_sprites,
                    (&mut order_hide, &mut pick_up_target),
                    media::local_time(tick) + 1,
                );
            }
            for click in move_clicks.iter().filter(|c| c.tick == tick) {
                headless_move_click(
                    click,
                    &mut sim,
                    &mesh_tag,
                    &initial_camera,
                    (args.width, args.height),
                    (
                        scenery_objects.media.as_deref(),
                        view_bars(&ui),
                        media::local_time(tick),
                    ),
                    args.select_ids.as_deref(),
                    &mut local_sprites,
                    &mut order_hide,
                    media::local_time(tick) + 1,
                )?;
            }
            for &(_, (x, y)) in hud_clicks.iter().filter(|(t, _)| *t == Some(tick)) {
                let (x, y) = ui.to_logical(x as f32, y as f32);
                if let Some(mm) = headless_minimap.as_mut() {
                    let origin = world_frame(logical_w, logical_h).1;
                    let (fx, fy) = (x - origin.0, y - origin.1);
                    if mm.map.takes(fx, fy) {
                        let target = mm.map.click(fx, fy, true);
                        mm.map.release();
                        if let Some(p) = target
                            && !camera_control.user_camera_locked()
                        {
                            let mut loc = camera_control
                                .observer(&headless_observer, camera_frame)
                                .location();
                            loc.point = p;
                            camera_control.set_observer_location(
                                loc,
                                &mut headless_observer,
                                camera_frame,
                            );
                            camera_runs += 1;
                            info!("Overhead map click at tick {tick}: view centre to {p:?}");
                        }
                        continue;
                    }
                }
                let Some(hud) = headless_hud.as_mut() else {
                    break;
                };
                hud.press(x, y);
                let event = hud.release(x, y);
                info!("HUD click at tick {tick} at ({x}, {y}): {event:?}");
                if let Some(hud::HudEvent::Command { kind }) = event
                    && let Some(action) = command_bar::bar_action(kind)
                {
                    let selection = headless_selection(&sim, args.select_ids.as_deref());
                    let look = mouse.as_ref().is_some_and(|m| m.machine.looking());
                    let outcome = command_bar::run_bar_action(
                        action,
                        false,
                        &selection,
                        look,
                        &mut sim,
                        Some(hud),
                        &mut local_sprites,
                        &mut order_hide,
                        media::local_time(tick) + 1,
                    );
                    info!("{action:?} for {selection:?}: {outcome:?}");
                }
            }
            record_unit_tick(&sim, &mut engine_state, &mut unit_states);
            if let (Some(picks), Some(last)) = (picks.as_mut(), unit_states.last()) {
                picks.tick(last);
            }
        }
        if let Some(e) = &sim.engine_stop {
            info!("Engine model stopped during the run: {e}");
        }
    }

    let run_controls = if args.caps_lock || args.hud {
        controls::load(args.conf.as_deref(), true).0
    } else {
        controls::Controls::defaults()
    };
    let headless_interface = HeadlessInterface {
        caps_lock: args
            .caps_lock
            .then(|| interface_state::caps_lock_mode(run_controls.options.caps_lock)),
        hover: match args.hover.as_deref() {
            None => None,
            Some(&[x, y]) => Some((x, y)),
            Some(v) => anyhow::bail!("--hover takes X,Y, got {} values", v.len()),
        },
        command_targets: args.command_targets.clone().unwrap_or_default(),
        tfl_selection_boxes: args.tfl_selection_boxes,
        order_hide,
        pick_up_target,
    };
    let headless_hud = if args.hud {
        let mut hud = headless_hud.take();
        if let Some(hud) = hud.as_mut() {
            hud.update_console(media::local_time(args.ticks));
            hud.stop_line = console::stop_text(&sim);
            let keyed = args.scores
                || (args.caps_lock
                    && run_controls.options.caps_lock == controls::CapsLock::PlayerStats);
            hud.update_scores(&sim, &film_names, keyed);
            hud.set_key_bindings(args.key_bindings.then_some(&run_controls));
            for row in hud.key_rows().unwrap_or_default() {
                info!(
                    "Key binding: {:?} {:?}",
                    String::from_utf8_lossy(&row.action),
                    String::from_utf8_lossy(&row.key)
                );
            }
            for line in hud.score_lines() {
                info!(
                    "Score line {}: {:?} {:?} {:?} fill {:?} {}",
                    line.row,
                    line.score.as_deref().map(String::from_utf8_lossy),
                    String::from_utf8_lossy(&line.name),
                    String::from_utf8_lossy(&line.right),
                    line.fill,
                    if line.active { "active" } else { "faded" }
                );
            }
            let lines: Vec<String> = hud
                .console_lines
                .lines()
                .iter()
                .map(|l| String::from_utf8_lossy(&l.text).into_owned())
                .collect();
            info!("Console lines at the render (newest first): {lines:?}");
        }
        if let (Some(hud), world) = (hud.as_mut(), &sim.engine) {
            let ids = args.select_ids.clone().unwrap_or_else(|| {
                let mut ids: Vec<u32> = sim
                    .units
                    .values()
                    .filter(|u| u.selected)
                    .map(|u| u.id)
                    .collect();
                ids.sort_unstable();
                ids
            });
            hud.set_selection(world, &ids);
            if let Some(info) = hud.unit_info().filter(|_| hud.has_selection()) {
                info!(
                    "HUD unit info: {:?} {:?}, description {:?}",
                    String::from_utf8_lossy(&info.record.name),
                    String::from_utf8_lossy(&info.record.kind),
                    String::from_utf8_lossy(&info.description)
                );
            }
        }
        if let Some(hud) = hud.as_mut() {
            for &(_, (x, y)) in hud_clicks.iter().filter(|(t, _)| t.is_none()) {
                let (x, y) = ui.to_logical(x as f32, y as f32);
                hud.press(x, y);
                info!("HUD click at ({x}, {y}): {:?}", hud.release(x, y));
            }
            if let Some(p) = &args.hud_press {
                let (x, y) = parse_point(p)?;
                let (x, y) = ui.to_logical(x as f32, y as f32);
                info!("HUD press held at ({x}, {y}): taken {}", hud.press(x, y));
            }
        }
        hud
    } else {
        None
    };
    let mut view_camera = initial_camera.clone();
    if camera_runs > 0 {
        let state = camera_control.observer(&headless_observer, camera_frame);
        info!(
            "Camera control at the render: flags {:#x}, move {:?}, observer {state:?} \
             (location {:?})",
            camera_control.flags,
            camera_control.movement,
            state.location()
        );
        let (half_w, half_d) = (camera_frame.half_width, camera_frame.half_height);
        headless_observer.apply_to(&mut view_camera, |x, z| {
            mesh_tag.terrain_height_at(half_w - x, z + half_d)
        });
    }
    camera_control::shake_camera(
        &mut camera_control,
        &mut view_camera,
        camera_frame,
        sim.current_tick as i32,
    );
    let projfx_log = args
        .projfx_log
        .as_ref()
        .map(|p| -> anyhow::Result<_> {
            let text = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("--projfx-log {p:?}: {e}"))?;
            projfx_log::ProjfxLog::parse(&text, &sim)
        })
        .transpose()?;
    if let (Some(path), Some(field)) = (&args.particle_state, particle_field.as_mut()) {
        let text = std::fs::read_to_string(path)?;
        let n = field
            .load_state(&text)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        info!(
            "{n} particles from {}; the simulation's wind {:?}, multiplier {:?}",
            path.display(),
            (sim.engine.wind.as_ref()).map(|w| w.velocity),
            (sim.engine.particles.as_ref()).map(|p| p.multiplier),
        );
    }
    film_report::finish(&sim);
    let view = crate::app::view_settings::ViewSettings::of(&args);
    let water_settings = (args.water == Some(args::Water::Enhanced))
        .then(|| crate::water_config::load(args.conf.as_deref(), true, mesh_tag.mesh_id.as_str()));
    render_headless_frame(
        &mesh_tag,
        &connectors,
        (terrain_texture.as_ref(), landscape_detail.as_deref()),
        &archive,
        large_archive.as_ref(),
        &mut sim,
        args.width,
        args.height,
        &view_camera,
        &output_path,
        args.ticks,
        ticks_without_engine,
        projfx_log,
        &unit_states,
        start_motions,
        args.unit_draw_log.as_deref(),
        object_history::frame_fraction(args.frame_fraction),
        interface_colors.as_ref(),
        &scenery_objects,
        band_drag,
        engine_idle,
        headless_interface,
        headless_hud.as_ref(),
        headless_minimap.as_ref(),
        view.translucent_bars,
        HeadlessMarkers {
            collection: interface_collection.as_ref(),
            sprites: &local_sprites,
        },
        (ui, ui_mode.filter(args.ui_filter)),
        (
            view.sampling,
            view.sprite_filter,
            &map_fog,
            view.reflection_soft_edge,
            water_settings.as_ref(),
        ),
        particle_field.as_ref(),
        local_projectiles.as_ref(),
        &landscape_edits,
        &mesh_ripple,
        args.ten_bit == Some(args::TenBit::On),
    )?;
    info!("SUCCESS: Rendered frame saved to {:?}", output_path);
    Ok(())
}
