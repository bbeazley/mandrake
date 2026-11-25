use crate::interface_state;
use crate::media;
use crate::move_marker;
use crate::picking::{
    PickWorld, hover_pick, ray_terrain_intersection, sprite_window_rect, unproject_cursor,
};
use crate::region_view::engine_ground_pick;
use crate::selection_input;
use crate::sprite_manager::ClientSpriteManager;
use crate::unit_anim;
use crate::unit_anim::UnitAnimator;
use crate::unit_billboards::{drawn_surface_position, drawn_unit_bounds, drawn_unit_sprite};
use mandrake_common::FourCC;
use mandrake_math::{Fixed16, Vec3Fixed};
use mandrake_sim::{Simulation, UnitState};
use mandrake_tag::MeshTag;
use mandrake_tag::collection::CollectionTag;
use mandrake_view::{camera::Camera, selection};
use tracing::{debug, info};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HeadlessMoveClick {
    pub(crate) tick: u32,
    at: HeadlessClickPoint,
    shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HeadlessClickPoint {
    Pixel(f32, f32),
    World(i32, i32),
}

pub(crate) fn parse_move_click(s: &str) -> anyhow::Result<HeadlessMoveClick> {
    let mut parts = s.split(':');
    let tick = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("--move-click {s}: no tick"))?
        .parse()?;
    let point = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("--move-click {s}: no point"))?;
    let shift = match parts.next() {
        None => false,
        Some("shift") => true,
        Some(o) => anyhow::bail!("--move-click {s}: unknown modifier {o}"),
    };
    let at = match point.strip_prefix("world=") {
        Some(w) => {
            let (x, y) = parse_point(w)?;
            HeadlessClickPoint::World(x, y)
        }
        None => {
            let (x, y) = point
                .split_once(',')
                .ok_or_else(|| anyhow::anyhow!("--move-click {s}: X,Y"))?;
            HeadlessClickPoint::Pixel(x.trim().parse()?, y.trim().parse()?)
        }
    };
    Ok(HeadlessMoveClick { tick, at, shift })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HeadlessPoll {
    tick: u32,
    cursor: (f32, f32),
    left: bool,
    right: bool,
}

pub(crate) fn parse_mouse_schedule(s: &str) -> anyhow::Result<Vec<HeadlessPoll>> {
    let xy = |p: &str| -> anyhow::Result<(f32, f32)> {
        let (x, y) = p
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("--mouse: X/Y expected, got {p}"))?;
        Ok((x.trim().parse()?, y.trim().parse()?))
    };
    let mut polls = Vec::new();
    for event in s.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let (tick, what) = event
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("--mouse {event}: no tick"))?;
        let tick: u32 = tick.trim().parse()?;
        let (kind, rest) = what
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("--mouse {event}: no event"))?;
        let (button, at) = rest
            .split_once('@')
            .ok_or_else(|| anyhow::anyhow!("--mouse {event}: no position"))?;
        let right = match button {
            "left" => false,
            "right" => true,
            b => anyhow::bail!("--mouse {event}: unknown button {b}"),
        };
        let held = |tick, cursor| HeadlessPoll {
            tick,
            cursor,
            left: !right,
            right,
        };
        let up = |tick, cursor| HeadlessPoll {
            tick,
            cursor,
            left: false,
            right: false,
        };
        match kind {
            "click" => {
                let p = xy(at)?;
                polls.push(held(tick + 2, p));
                polls.push(up(tick + 3, p));
            }
            "drag" => {
                let (from, rest) = at
                    .split_once('-')
                    .ok_or_else(|| anyhow::anyhow!("--mouse {event}: X0/Y0-X1/Y1/N"))?;
                let (to, n) = rest
                    .rsplit_once('/')
                    .ok_or_else(|| anyhow::anyhow!("--mouse {event}: X0/Y0-X1/Y1/N"))?;
                let (a, b, n) = (xy(from)?, xy(to)?, n.trim().parse::<u32>()?.max(1));
                polls.push(held(tick + 2, a));
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let p = (
                        (a.0 + (b.0 - a.0) * t).trunc(),
                        (a.1 + (b.1 - a.1) * t).trunc(),
                    );
                    polls.push(held(tick + 2 + i, p));
                }
                polls.push(up(tick + 3 + n, b));
            }
            k => anyhow::bail!("--mouse {event}: unknown event {k}"),
        }
    }
    Ok(polls)
}

pub(crate) struct HeadlessMouse {
    polls: Vec<HeadlessPoll>,
    pub(crate) machine: selection_input::SelectionMachine,
    actions: selection_input::ActionBuilder,
    cursor: (f32, f32),
    selection: Option<Vec<u32>>,
}

pub(crate) struct HeadlessPicks {
    sprite_mgr: ClientSpriteManager,
    animator: UnitAnimator,
    media: Option<std::sync::Arc<media::MapMedia>>,
}

impl HeadlessPicks {
    pub(crate) fn new(
        archive: &mandrake_tag::TagArchive,
        large_archive: Option<&mandrake_tag::TagArchive>,
        mesh_tag: &MeshTag,
        connectors: &mandrake_tag::MeshConnectorList,
        sim: &Simulation,
        (media, engine_idle, start_motions): (
            Option<std::sync::Arc<media::MapMedia>>,
            Option<unit_anim::EngineIdleModel>,
            unit_anim::UnitMotions,
        ),
    ) -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()?;
        let config = mandrake_render::RenderPipelineConfig {
            width: 16,
            height: 16,
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        };
        let billboards = mandrake_render::BillboardRenderer::new(&device, &queue, &config);
        let sprite_mgr = ClientSpriteManager::build(
            &device,
            &queue,
            &billboards.texture_bind_group_layout,
            archive,
            large_archive,
            mesh_tag,
            connectors,
            None,
            Some(&crate::corpse::EngineDeathGroups::of(&sim.engine)),
            crate::object_bounds::mons_postprocess_of(sim),
            &Default::default(),
        );
        let models = mandrake_render::ModelRenderer::new(&device, &queue, &config);
        let (meshes, mut instances) = crate::model_instances::load_model_instances(
            &device,
            &queue,
            &models,
            archive,
            large_archive,
            mesh_tag,
        );
        let mut media = media;
        let archives: Vec<&mandrake_tag::TagArchive> =
            large_archive.into_iter().chain(Some(archive)).collect();
        crate::model_instances::attach_model_media(
            &device,
            &models,
            &mut instances,
            &meshes,
            &archives,
            &mut media,
        );
        let mut animator = UnitAnimator::with_engine_idle(engine_idle);
        animator.seed_motions(start_motions);
        Some(Self {
            sprite_mgr,
            animator,
            media,
        })
    }

    pub(crate) fn tick(&mut self, tick: &UnitTickRecord) {
        self.animator
            .replay_tick(tick.engine.clone(), tick.motions.clone());
        self.animator
            .tick(tick.states.iter().copied(), &self.sprite_mgr.unit_visuals);
    }
}

impl HeadlessMouse {
    pub(crate) fn new(polls: Vec<HeadlessPoll>) -> Self {
        Self {
            polls,
            machine: selection_input::SelectionMachine::default(),
            actions: selection_input::ActionBuilder::default(),
            cursor: (0.0, 0.0),
            selection: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn poll(
        &mut self,
        tick: u32,
        sim: &mut Simulation,
        mesh_tag: &MeshTag,
        camera: &Camera,
        viewport: (u32, u32),
        (map_media, bars): (Option<&media::MapMedia>, (i32, i32)),
        picks: Option<&HeadlessPicks>,
        select_ids: Option<&[u32]>,
        sprites: &mut move_marker::LocalSprites,
        (order_hide, pick_up_target): (
            &mut interface_state::OrderFlagHide,
            &mut selection::state::TargetHighlight,
        ),
        now: i32,
    ) {
        let p = self.polls.iter().find(|p| p.tick == tick).copied();
        if let Some(p) = p {
            self.cursor = p.cursor;
        }
        let action = self.actions.poll(selection_input::PollState {
            cursor: (self.cursor.0 as i16, self.cursor.1 as i16),
            left: p.is_some_and(|p| p.left),
            right: p.is_some_and(|p| p.right),
            ..Default::default()
        });
        let mut selection = self
            .selection
            .take()
            .unwrap_or_else(|| headless_selection(sim, select_ids));
        let outcome = {
            let mut world = PickWorld::myth2(
                sim,
                mesh_tag,
                camera,
                viewport,
                picks.map(|p| (&p.sprite_mgr, &p.animator)),
                selection.clone(),
                picks.and_then(|p| p.media.as_deref()).or(map_media),
                (bars, media::local_time(tick)),
            );
            self.machine.update(&action, &mut world, &mut selection)
        };
        let ids: std::collections::HashSet<u32> = selection.iter().copied().collect();
        for u in sim.units.values_mut() {
            u.selected = ids.contains(&u.id);
        }
        self.selection = Some(selection.clone());
        if p.is_some() || outcome != selection_input::Outcome::None {
            info!(
                "Mouse poll at tick {tick}: cursor {:?} flags {:#x} state {:?} -> {outcome:?}, \
                 selection {selection:?}",
                action.cursor,
                action.flags.bits(),
                self.machine.state(),
            );
        }
        if let selection_input::Outcome::PickUp {
            index,
            identifier,
            units,
        } = &outcome
        {
            sim.queue_local_pick_up(mandrake_sim::LocalPickUp {
                object_index: *index,
                object_identifier: *identifier,
                unit_ids: units.clone(),
            });
            order_hide.order(units);
            pick_up_target.pick_up(i32::from(*index));
            info!("Mouse pick-up at tick {tick} of object {index} ({identifier}) for {units:?}");
        }
        if let selection_input::Outcome::Move {
            point,
            facing,
            add_waypoint,
        } = outcome
        {
            let sent = move_marker::ground_click(
                sprites,
                sim,
                &selection,
                point,
                facing,
                add_waypoint,
                now,
            );
            if let Ok(Some(c)) = &sent {
                order_hide.order(&c.unit_ids);
            }
            info!("Mouse move at tick {tick} on world {point:?}: {sent:?}");
        }
    }
}

pub(crate) fn parse_hud_click(s: &str) -> anyhow::Result<(Option<u32>, (i32, i32))> {
    match s.split_once(':') {
        Some((t, p)) => Ok((Some(t.trim().parse()?), parse_point(p)?)),
        None => Ok((None, parse_point(s)?)),
    }
}

pub(crate) fn headless_selection(sim: &Simulation, select_ids: Option<&[u32]>) -> Vec<u32> {
    match select_ids {
        Some(ids) => ids
            .iter()
            .copied()
            .filter(|id| sim.units.get(id).is_some_and(|u| u.selected))
            .collect(),
        None => {
            let mut ids: Vec<u32> = sim
                .units
                .values()
                .filter(|u| u.selected)
                .map(|u| u.id)
                .collect();
            ids.sort_unstable();
            ids
        }
    }
}

pub(crate) fn parse_point(s: &str) -> anyhow::Result<(i32, i32)> {
    let (x, y) = s
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("expected X,Y, got {s:?}"))?;
    Ok((x.trim().parse()?, y.trim().parse()?))
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HeadlessInterface {
    pub(crate) caps_lock: Option<selection::state::CapsLock>,
    pub(crate) hover: Option<(f32, f32)>,
    pub(crate) command_targets: Vec<u32>,
    pub(crate) tfl_selection_boxes: bool,
    pub(crate) order_hide: interface_state::OrderFlagHide,
    pub(crate) pick_up_target: selection::state::TargetHighlight,
}

impl HeadlessInterface {
    pub(crate) fn view(
        &self,
        sim: &Simulation,
        camera: &Camera,
        viewport: (u32, u32),
        sprite_mgr: &ClientSpriteManager,
        animator: &UnitAnimator,
    ) -> interface_state::InterfaceView {
        let mut hover = selection::state::HoverTracker::default();
        if let Some((x, y)) = self.hover {
            hover.update(selection::state::HOVER_REFRESH_MS + 1, true, || {
                hover_pick(sim, camera, viewport, (x, y), Some((sprite_mgr, animator)))
            });
            info!("Hover at ({x}, {y}): {:?}", hover.hovered());
        }
        let selection: Vec<u32> = sim
            .units
            .values()
            .filter(|u| u.selected)
            .map(|u| u.id)
            .collect();
        let mut targets = self.pick_up_target.clone();
        if !self.command_targets.is_empty() {
            targets.command(&self.command_targets, &selection);
        }
        interface_state::InterfaceView {
            caps_lock: self.caps_lock,
            hovered: hover.hovered(),
            targets,
            tfl_selection_boxes: self.tfl_selection_boxes,
            order_hide: self.order_hide.clone(),
        }
    }
}

pub(crate) fn write_unit_draw_log(
    out: &mut impl std::io::Write,
    tick: u32,
    sim: &Simulation,
    (camera, viewport): (&Camera, (u32, u32)),
    animator: &UnitAnimator,
    sprite_mgr: &ClientSpriteManager,
) -> std::io::Result<()> {
    let eye = camera.eye_position();
    let cam = animator.camera_ground_point(eye);
    let mut units: Vec<&mandrake_sim::UnitEntity> = sim.units.values().collect();
    units.sort_by_key(|u| u.id);
    for unit in units {
        let Some(visual) = sprite_mgr.unit_visuals.get(&unit.archetype) else {
            continue;
        };
        let (Some(m), Some(shape)) = (
            animator.drawn_motion(unit.id),
            animator.drawn_shape(unit.id),
        ) else {
            continue;
        };
        let Some(sequence) = visual.sequences.get(&shape.sequence) else {
            continue;
        };
        let seq = &sequence.sequence;
        let mirrored = animator.mirrored(unit.id);
        let relative = animator.relative_angle(unit, eye);
        let view =
            mandrake_tag::view_for_relative_angle(relative, usize::from(seq.num_views), mirrored);
        let (stored, flip) = seq.resolve_view(view);
        writeln!(
            out,
            "{tick} U id={} seq={} frame={} view={view} stored={stored} flip={} mirrored={} \
             facing={} pos={},{},{} cam={},{} screen={:?} rect={} sprite={} view={}",
            unit.id,
            shape.sequence,
            shape.frame,
            u8::from(flip ^ mirrored),
            u8::from(mirrored),
            m.facing,
            m.world[0],
            m.world[1],
            m.world[2],
            cam.0,
            cam.1,
            camera
                .frame_projection(animator.drawn_position(unit), viewport.0, viewport.1)
                .map(|p| p.origin),
            {
                let position = drawn_surface_position(unit, animator);
                let bounds = drawn_unit_bounds(unit, visual, sequence, animator);
                camera
                    .frame_projection(position, viewport.0, viewport.1)
                    .map(|p| selection::projected_frame_rect(bounds, &p))
            }
            .map_or_else(
                || "-".to_string(),
                |r| format!("{},{},{},{}", r.left, r.right, r.top, r.bottom),
            ),
            drawn_unit_sprite(
                &crate::world_view::myth2::unit_view(sim, unit),
                &crate::world_view::myth2::header(sim),
                camera,
                animator,
                sprite_mgr,
            )
            .map_or_else(
                || "-".to_string(),
                |sprite| {
                    let [l, r, t, b] = sprite_window_rect(&sprite, camera, viewport);
                    format!("{l:.3},{r:.3},{t:.3},{b:.3}")
                },
            ),
            {
                let [x, y, z] = camera
                    .engine_view()
                    .view_coords(animator.drawn_position(unit));
                format!("{x},{y},{z}")
            },
        )?;
    }
    Ok(())
}

pub(crate) fn record_unit_tick(
    sim: &Simulation,
    engine_state: &mut Option<unit_anim::EngineIdleModel>,
    trace: &mut Vec<UnitTickRecord>,
) {
    if let Some(e) = engine_state.as_mut() {
        e.observe(sim);
    }
    let mut states: Vec<(u32, FourCC, UnitState)> = sim
        .units
        .values()
        .map(|u| (u.id, u.archetype, u.state))
        .collect();
    states.sort_by_key(|&(id, ..)| id);
    trace.push(UnitTickRecord {
        states,
        motions: unit_anim::unit_motions(sim, engine_state.as_ref()),
        engine: engine_state.clone(),
    });
}

pub(crate) struct UnitTickRecord {
    pub(crate) states: Vec<(u32, FourCC, UnitState)>,
    pub(crate) engine: Option<unit_anim::EngineIdleModel>,
    pub(crate) motions: unit_anim::UnitMotions,
}

#[allow(clippy::too_many_arguments)]
pub(crate) struct HeadlessMarkers<'a> {
    pub(crate) collection: Option<&'a CollectionTag>,
    pub(crate) sprites: &'a move_marker::LocalSprites,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn headless_move_click(
    click: &HeadlessMoveClick,
    sim: &mut Simulation,
    mesh_tag: &MeshTag,
    camera: &Camera,
    (width, height): (u32, u32),
    (map_media, bars, local_time): (Option<&media::MapMedia>, (i32, i32), i32),
    select_ids: Option<&[u32]>,
    sprites: &mut move_marker::LocalSprites,
    order_hide: &mut interface_state::OrderFlagHide,
    now: i32,
) -> anyhow::Result<()> {
    let point = match click.at {
        HeadlessClickPoint::World(x, y) => (x, y),
        HeadlessClickPoint::Pixel(x, y) if map_media.is_some() => engine_ground_pick(
            camera,
            mesh_tag,
            map_media,
            (width, height),
            bars,
            local_time,
            (x, y),
        )
        .ok_or_else(|| anyhow::anyhow!("--move-click at ({x}, {y}): no ground"))?,
        HeadlessClickPoint::Pixel(x, y) => {
            let (o, d) = unproject_cursor(x, y, width, height, camera);
            let [gx, gy, gz] = ray_terrain_intersection(o, d, mesh_tag)
                .ok_or_else(|| anyhow::anyhow!("--move-click at ({x}, {y}): no ground"))?;
            sim.world_to_command_point(Vec3Fixed::new(
                Fixed16::from_f32(gx),
                Fixed16::from_f32(gy),
                Fixed16::from_f32(gz),
            ))
        }
    };
    let selection = headless_selection(sim, select_ids);
    let sent = move_marker::ground_click(sprites, sim, &selection, point, None, click.shift, now);
    if let Ok(Some(c)) = &sent {
        order_hide.order(&c.unit_ids);
    }
    info!(
        "Move click at tick {} on world {point:?} (shift {}) for {selection:?}: {sent:?}; {} \
         local sprite(s), timer {:?}",
        click.tick,
        click.shift,
        sprites.entries().len(),
        sprites.timer(),
    );
    for s in sprites.entries() {
        debug!("  local sprite seq {} at {:?}", s.sequence, s.point);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HeadlessDrag {
    pub(crate) from: (f32, f32),
    pub(crate) to: (f32, f32),
    pub(crate) polls: u32,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_headless_drag(
    drag: HeadlessDrag,
    sim: &Simulation,
    mesh_tag: &MeshTag,
    camera: &Camera,
    viewport: (u32, u32),
    sprite_mgr: &ClientSpriteManager,
    animator: &UnitAnimator,
    (bars, local_time): ((i32, i32), i32),
) -> (Option<selection::PixelRect>, Vec<u32>) {
    let mut selection: Vec<u32> = sim
        .units
        .values()
        .filter(|u| u.selected)
        .map(|u| u.id)
        .collect();
    selection.sort_unstable();
    let mut machine = selection_input::SelectionMachine::default();
    let mut actions = selection_input::ActionBuilder::default();
    let point = |i: u32| {
        let t = i as f32 / drag.polls as f32;
        (
            (drag.from.0 + (drag.to.0 - drag.from.0) * t) as i16,
            (drag.from.1 + (drag.to.1 - drag.from.1) * t) as i16,
        )
    };
    let polls =
        std::iter::once((point(0), false)).chain((0..=drag.polls).map(|i| (point(i), true)));
    for (cursor, left) in polls {
        let action = actions.poll(selection_input::PollState {
            cursor,
            left,
            ..Default::default()
        });
        let mut world = PickWorld::myth2(
            sim,
            mesh_tag,
            camera,
            viewport,
            Some((sprite_mgr, animator)),
            selection.clone(),
            sprite_mgr.scenery_objects.media.as_deref(),
            (bars, local_time),
        );
        machine.update(&action, &mut world, &mut selection);
    }
    selection.sort_unstable();
    (machine.band_bounds(), selection)
}
