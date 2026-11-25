use std::collections::{BTreeMap, BTreeSet};

use mandrake_common::FourCC;
use mandrake_math::Angle512;
use mandrake_sim::engine_rng::{self, EngineObject, Unmodelled};
use mandrake_sim::views::ObjectShape;
use mandrake_sim::{Simulation, UnitEntity, UnitState};
use mandrake_tag::{
    CollectionSequence, MONSTER_SEQUENCE_SLOTS, MonsterSequenceSlot, SequenceEvents,
    SequenceHeader, SequencePlayback,
};
use tracing::{info, warn};

use crate::world_view::UnitPose;

use crate::minimap;
use crate::object_history::{MYTH2_STRIDE_JUMP, ObjectMotion, Stride, shows_history_shape};

#[derive(Clone, Debug, PartialEq)]
pub struct ViewSprite {
    pub atlas_index: usize,
    pub world_size: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShadowSprite {
    pub flags: u32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub width: u16,
    pub height: u16,
    pub pixels: std::sync::Arc<[u8]>,
}

#[derive(Clone, Debug, Default)]
pub struct FrameVisual {
    pub views: Vec<Option<ViewSprite>>,
    pub shadow: Option<ShadowSprite>,
    pub shadow_sources: Vec<Option<std::sync::Arc<crate::shadowgen::ShadowSource>>>,
}

#[derive(Clone, Debug)]
pub struct SequenceVisual {
    pub sequence: CollectionSequence,
    pub frames: Vec<FrameVisual>,
    pub object_bounds: mandrake_tag::SequenceWorldBounds,
}

#[derive(Clone, Debug, Default)]
pub struct UnitSpriteVisual {
    pub slots: [Option<usize>; MONSTER_SEQUENCE_SLOTS],
    pub sequences: BTreeMap<usize, SequenceVisual>,
    pub object: Option<mandrake_tag::ObjectDefinition>,
    pub definition: (u32, i16),
}

impl UnitSpriteVisual {
    pub fn sequence_for_slot(&self, slot: MonsterSequenceSlot) -> Option<&SequenceVisual> {
        self.sequences.get(&self.slot_index(slot)?)
    }

    pub fn slot_index(&self, slot: MonsterSequenceSlot) -> Option<usize> {
        self.slots
            .get(slot.0)
            .copied()
            .flatten()
            .or(self.slots[MonsterSequenceSlot::STANDING.0])
    }

    pub fn is_empty(&self) -> bool {
        self.sequences.is_empty()
    }
}

pub fn slot_for_state(state: UnitState) -> MonsterSequenceSlot {
    match state {
        UnitState::Moving => MonsterSequenceSlot::MOVING,
        UnitState::Idle | UnitState::Attacking | UnitState::Dead => MonsterSequenceSlot::STANDING,
    }
}

#[cfg(test)]
pub fn select_view(
    visual: &SequenceVisual,
    frame: usize,
    stored_view: u8,
    relative: u16,
    mirrored: bool,
) -> Option<(&ViewSprite, bool)> {
    let (stored, flip) = crate::corpse::object_view(visual, stored_view, relative, mirrored);
    let frame = visual.frames.get(frame % visual.frames.len().max(1))?;
    frame
        .views
        .get(stored)?
        .as_ref()
        .map(|s| (s, flip ^ mirrored))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineIdleUnit {
    pub object: EngineObject,
    pub mode: u8,
    pub anim_state: u8,
    pub mirrored: bool,
}

impl EngineIdleUnit {
    fn of(m: &engine_rng::EngineMonster) -> Self {
        Self {
            object: m.object.clone(),
            mode: m.mode,
            anim_state: m.anim_state,
            mirrored: m.mirrored,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.mode == 1
    }
}

pub struct EngineIdleModel {
    collections: std::sync::Arc<mandrake_sim::entities::TagIdMap<Vec<SequenceHeader>>>,
    units: BTreeMap<u32, EngineIdleUnit>,
    order: Vec<u32>,
    stop: Option<(u32, Unmodelled)>,
    ticks: u32,
    shapes: BTreeMap<u32, ObjectShape>,
    visibility: Option<Box<DrawVisibility>>,
    visible: std::sync::Arc<Vec<u32>>,
    mesh_cells: (i32, i32),
    game_type: i16,
    gameplay_version: i32,
    renames_read: usize,
    renames: Vec<(u32, u32)>,
}

impl Clone for EngineIdleModel {
    fn clone(&self) -> Self {
        Self {
            collections: std::sync::Arc::clone(&self.collections),
            units: self.units.clone(),
            order: self.order.clone(),
            stop: self.stop.clone(),
            ticks: self.ticks,
            shapes: self.shapes.clone(),
            visibility: None,
            visible: std::sync::Arc::clone(&self.visible),
            mesh_cells: self.mesh_cells,
            game_type: self.game_type,
            gameplay_version: self.gameplay_version,
            renames_read: self.renames_read,
            renames: self.renames.clone(),
        }
    }
}

struct DrawVisibility {
    buffers: minimap::Visibility,
    before_tick: Vec<minimap::VisibilityMonster>,
}

impl std::fmt::Debug for EngineIdleModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineIdleModel")
            .field("units", &self.units.len())
            .field("stop", &self.stop)
            .field("ticks", &self.ticks)
            .finish()
    }
}

impl EngineIdleModel {
    pub fn new(sim: &Simulation) -> Option<Self> {
        let world = &sim.engine;
        let seers = crate::world_view::myth2::visibility_monsters(world);
        let mut buffers = minimap::Visibility::new(world.mesh_cells.0, world.mesh_cells.1);
        buffers.build_initial(
            &seers,
            crate::world_view::myth2::local_team(world),
            world.game_options,
        );
        let visible = std::sync::Arc::new(buffers.visible.clone());
        let mut model = Self {
            collections: std::sync::Arc::new(world.collections.clone()),
            units: BTreeMap::new(),
            order: Vec::new(),
            stop: None,
            ticks: 0,
            shapes: BTreeMap::new(),
            visibility: Some(Box::new(DrawVisibility {
                buffers,
                before_tick: seers,
            })),
            visible,
            mesh_cells: world.mesh_cells,
            game_type: world.game_type,
            gameplay_version: world.formation_globals.gameplay_version,
            renames_read: world.renamed_monsters.len(),
            renames: Vec::new(),
        };
        model.read(world);
        model.read_shapes(sim);
        model.check_stop(sim);
        if model.stop.is_none() {
            info!(
                "Unit animation: engine model running for {} monsters",
                model.units.len()
            );
        }
        Some(model)
    }

    fn read(&mut self, world: &engine_rng::EngineWorld) {
        for m in &world.monsters {
            if let Some(id) = m.unit_key()
                && self.units.insert(id, EngineIdleUnit::of(m)).is_none()
            {
                self.order.push(id);
            }
        }
    }

    fn advance_visibility(&mut self, sim: &Simulation) {
        let Some(state) = self.visibility.as_deref_mut() else {
            return;
        };
        let world = &sim.engine;
        let time = sim.current_tick as i32 - 1;
        state.buffers.update(
            time,
            &state.before_tick,
            crate::world_view::myth2::local_team(world),
            world.game_options,
        );
        if time & 0x3f == 0 {
            self.visible = std::sync::Arc::new(state.buffers.visible.clone());
        }
        state.before_tick = crate::world_view::myth2::visibility_monsters(world);
    }

    pub fn object_visibility(
        &self,
        flags: u32,
        kind: u8,
        (x, y): (i32, i32),
    ) -> minimap::ObjectVisibility {
        let bits = minimap::visible_cell_bits(&self.visible, self.mesh_cells, x >> 9, y >> 9);
        minimap::local_object_visibility(flags, kind, bits, self.game_type, self.gameplay_version)
    }

    pub fn cell_bits(&self, x: i32, y: i32) -> u32 {
        minimap::visible_cell_bits(&self.visible, self.mesh_cells, x >> 9, y >> 9) & 3
    }

    fn read_shapes(&mut self, sim: &Simulation) {
        self.shapes.clear();
        for &id in sim.units.keys() {
            if let Some(shape) = sim.unit_shape(id) {
                self.shapes.insert(id, shape);
            }
        }
    }

    pub fn shape(&self, id: u32) -> Option<&ObjectShape> {
        self.shapes.get(&id)
    }

    fn check_stop(&mut self, sim: &Simulation) {
        if self.stop.is_some() {
            return;
        }
        if let Some(e) = sim.engine_stop.clone() {
            warn!(
                "Unit animation: the engine model stopped at tick {} ({e}); units keep their \
                 sequence, speed and phase from there (animate_object only, no idle draws)",
                self.ticks
            );
            self.stop = Some((self.ticks, e));
        }
    }

    pub fn running(&self) -> bool {
        self.stop.is_none()
    }

    pub fn observe(&mut self, sim: &Simulation) {
        self.follow_renames(sim);
        self.observe_after_renames(sim);
    }

    fn observe_after_renames(&mut self, sim: &Simulation) {
        self.advance_visibility(sim);
        match (self.stop.is_none(), &sim.engine) {
            (true, world) => {
                let stopped = sim.engine_stop.is_some();
                for m in &world.monsters {
                    let Some(id) = m.unit_key() else {
                        continue;
                    };
                    let now = EngineIdleUnit::of(m);
                    match self.units.get_mut(&id) {
                        Some(unit) if stopped && now == *unit => {
                            Self::animate(&self.collections, unit);
                        }
                        Some(unit) => *unit = now,
                        None => {
                            self.units.insert(id, now);
                            self.order.push(id);
                        }
                    }
                }
            }
            _ => {
                for id in &self.order {
                    if let Some(unit) = self.units.get_mut(id) {
                        Self::animate(&self.collections, unit);
                    }
                }
            }
        }
        self.ticks += 1;
        self.read_shapes(sim);
        self.check_stop(sim);
    }

    fn animate(
        collections: &mandrake_sim::entities::TagIdMap<Vec<SequenceHeader>>,
        unit: &mut EngineIdleUnit,
    ) {
        if let Some(headers) = collections.get(&unit.object.collection) {
            engine_rng::animate_object(&mut unit.object, headers);
        }
    }

    fn follow_renames(&mut self, sim: &Simulation) {
        let world = &sim.engine;
        let start = self.renames_read.min(world.renamed_monsters.len());
        for &(old, new) in &world.renamed_monsters[start..] {
            if let (Some(old), Some(new)) = (engine_rng::unit_key(old), engine_rng::unit_key(new))
                && !sim.units.contains_key(&old)
            {
                self.rename_unit(old, new);
                self.renames.push((old, new));
            }
        }
        self.renames_read = world.renamed_monsters.len();
    }

    fn rename_unit(&mut self, old: u32, new: u32) {
        self.order.retain(|&id| id != new);
        if let Some(at) = self.order.iter().position(|&id| id == old) {
            self.order[at] = new;
        }
        self.units.remove(&new);
        if let Some(unit) = self.units.remove(&old) {
            self.units.insert(new, unit);
        }
    }

    pub fn unit(&self, id: u32) -> Option<&EngineIdleUnit> {
        self.units.get(&id)
    }

    #[cfg(test)]
    pub fn stop(&self) -> Option<&(u32, Unmodelled)> {
        self.stop.as_ref()
    }
}

fn engine_events(flags: u8) -> SequenceEvents {
    SequenceEvents {
        frame_changed: flags & 1 != 0,
        key_frame: flags & 2 != 0,
        wrapped: flags & 4 != 0,
    }
}

#[derive(Clone, Copy, Debug)]
enum UnitAnim {
    Playback(SequencePlayback),
    Engine {
        sequence: usize,
        frame: usize,
        view: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawnShape {
    pub sequence: usize,
    pub frame: usize,
    pub view: u8,
    pub shading_slot: u8,
}

#[derive(Clone, Debug, Default)]
pub struct UnitMotions {
    units: BTreeMap<u32, ObjectMotion>,
    ticks_per_frame: BTreeMap<u32, i8>,
    origin: [f32; 2],
    tick: i32,
}

impl UnitMotions {
    fn render_position(&self, world: [i32; 3]) -> [f32; 3] {
        let one = mandrake_tag::MarkerEntry::WORLD_ONE;
        [
            self.origin[0] - world[0] as f32 / one,
            world[2] as f32 / one,
            self.origin[1] + world[1] as f32 / one,
        ]
    }

    fn world_ground_point(&self, xz: [f32; 2]) -> (i32, i32) {
        let one = mandrake_tag::MarkerEntry::WORLD_ONE;
        (
            ((self.origin[0] - xz[0]) * one).round() as i32,
            ((xz[1] - self.origin[1]) * one).round() as i32,
        )
    }
}

pub(crate) fn client_facing_to_engine(facing: Angle512) -> u16 {
    ((256 - i32::from(facing.to_raw())).rem_euclid(512) as u16) << 7
}

fn fill_unit_motions(sim: &Simulation, engine: Option<&EngineIdleModel>, out: &mut UnitMotions) {
    let zero = sim.world_units_to_sim([0, 0, 0]);
    out.units.clear();
    out.ticks_per_frame.clear();
    for u in sim.units.values() {
        let mover = u.engine.as_ref();
        let (x, y) = mover.map_or_else(|| sim.world_to_command_point(u.position), |m| (m.x, m.y));
        let shape = engine.and_then(|e| e.shape(u.id));
        let facing = shape.map(|s| s.facing).or(mover.map(|m| m.facing));
        let scale = u.engine_object.map_or(0x100, |o| o.scale as i16);
        let tpf = engine
            .and_then(|e| e.unit(u.id))
            .map(|eu| eu.object.ticks_per_frame)
            .or_else(|| mover.map(|m| m.ticks_per_frame))
            .unwrap_or(1);
        let motion = ObjectMotion {
            world: [
                x,
                y,
                u.position.y.to_raw() / (65536 / mandrake_rec::WORLD_UNITS_PER_CELL),
            ],
            velocity: mover.map_or_else(
                || {
                    [u.velocity.x, u.velocity.z, u.velocity.y]
                        .map(|v| (v.to_raw() / (65536 / mandrake_rec::WORLD_UNITS_PER_CELL)) as i16)
                },
                |m| m.velocity,
            ),
            facing: facing.unwrap_or_else(|| client_facing_to_engine(u.facing)),
            elevation: 0,
            scale,
        };
        out.units.insert(u.id, motion);
        out.ticks_per_frame.insert(u.id, tpf);
    }
    out.origin = [zero.x.to_f32(), zero.z.to_f32()];
    out.tick = sim.current_tick as i32;
}

pub fn unit_motions(sim: &Simulation, engine: Option<&EngineIdleModel>) -> UnitMotions {
    let mut out = UnitMotions::default();
    fill_unit_motions(sim, engine, &mut out);
    out
}

pub fn follow_renamed(ids: &mut Vec<u32>, renamed: &[(u32, u32)]) {
    for &(old, new) in renamed {
        ids.retain(|&id| id != new);
        for id in ids.iter_mut() {
            if *id == old {
                *id = new;
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DrawnUnit {
    pub motion: Option<ObjectMotion>,
    pub shape: Option<DrawnShape>,
    pub world: [i32; 3],
    pub position: [f32; 3],
    pub facing: u16,
    pub flags: u32,
    pub mirrored: bool,
    pub bob_height: i32,
    pub visibility: Option<minimap::ObjectVisibility>,
}

#[derive(Default)]
pub struct UnitAnimator {
    states: BTreeMap<u32, UnitAnim>,
    engine: Option<EngineIdleModel>,
    diverged: BTreeSet<u32>,
    live: UnitMotions,
    history: BTreeMap<u32, ObjectMotion>,
    shape_history: BTreeMap<u32, DrawnShape>,
    renamed: Vec<(u32, u32)>,
    fraction: i32,
    strides: BTreeMap<u32, Stride>,
    glide: bool,
}

impl UnitAnimator {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_engine_idle(engine: Option<EngineIdleModel>) -> Self {
        Self {
            engine,
            ..Self::default()
        }
    }

    pub fn seed_motions(&mut self, motions: UnitMotions) {
        self.live = motions;
        self.strides.clear();
        let tick = self.live.tick.max(0) as u32;
        for (&id, m) in &self.live.units {
            self.strides
                .insert(id, Stride::start(i64::from(id), m.world, tick));
        }
    }

    pub fn observe(&mut self, sim: &Simulation) {
        self.renamed = match self.engine.as_mut() {
            Some(engine) => {
                let known = engine.renames.len();
                engine.follow_renames(sim);
                engine.renames[known..].to_vec()
            }
            None => Vec::new(),
        };
        for (old, new) in self.renamed.clone() {
            self.rename_unit(old, new);
        }
        self.record_history();
        if let Some(engine) = self.engine.as_mut() {
            engine.observe_after_renames(sim);
        }
        fill_unit_motions(sim, self.engine.as_ref(), &mut self.live);
        self.update_strides();
    }

    pub fn replay_tick(&mut self, engine: Option<EngineIdleModel>, motions: UnitMotions) {
        self.renamed = match (self.engine.as_ref(), engine.as_ref()) {
            (Some(before), Some(after)) => after
                .renames
                .get(before.renames.len()..)
                .unwrap_or_default()
                .to_vec(),
            _ => Vec::new(),
        };
        for (old, new) in self.renamed.clone() {
            if let Some(e) = self.engine.as_mut() {
                e.rename_unit(old, new);
            }
            self.rename_unit(old, new);
        }
        self.record_history();
        if engine.is_some() {
            self.engine = engine;
        }
        self.live = motions;
        self.update_strides();
    }

    fn update_strides(&mut self) {
        let tick = self.live.tick.max(0) as u32;
        self.strides
            .retain(|id, _| self.live.units.contains_key(id));
        for (&id, live) in &self.live.units {
            let tpf = self
                .engine
                .as_ref()
                .and_then(|e| e.unit(id))
                .map(|u| u.object.ticks_per_frame)
                .or_else(|| self.live.ticks_per_frame.get(&id).copied())
                .unwrap_or(1);
            let next = match self.strides.get(&id).copied() {
                Some(s) => s.next(i64::from(id), live.world, tpf, tick, MYTH2_STRIDE_JUMP),
                None => Stride::start(i64::from(id), live.world, tick),
            };
            self.strides.insert(id, next);
        }
    }

    fn rename_unit(&mut self, old: u32, new: u32) {
        for map in [&mut self.live.units, &mut self.history] {
            map.remove(&new);
            if let Some(m) = map.remove(&old) {
                map.insert(new, m);
            }
        }
        self.live.ticks_per_frame.remove(&new);
        if let Some(tpf) = self.live.ticks_per_frame.remove(&old) {
            self.live.ticks_per_frame.insert(new, tpf);
        }
        self.strides.remove(&new);
        if let Some(mut s) = self.strides.remove(&old) {
            s.identifier = i64::from(new);
            self.strides.insert(new, s);
        }
        self.shape_history.remove(&new);
        if let Some(s) = self.shape_history.remove(&old) {
            self.shape_history.insert(new, s);
        }
        self.states.remove(&new);
        if let Some(s) = self.states.remove(&old) {
            self.states.insert(new, s);
        }
        self.diverged.remove(&new);
        if self.diverged.remove(&old) {
            self.diverged.insert(new);
        }
    }

    pub fn renamed(&self) -> &[(u32, u32)] {
        &self.renamed
    }

    fn record_history(&mut self) {
        self.shape_history.clear();
        for &id in self.live.units.keys() {
            if let Some(shape) = self.live_shape(id) {
                self.shape_history.insert(id, shape);
            }
        }
        std::mem::swap(&mut self.history, &mut self.live.units);
        self.live.units.clear();
    }

    pub fn set_fraction(&mut self, fraction: i32) {
        self.fraction = fraction;
    }

    pub fn set_glide(&mut self, enabled: bool) {
        self.glide = enabled;
    }

    pub fn fraction(&self) -> i32 {
        self.fraction
    }

    fn glided_world(&self, id: u32) -> Option<[i32; 3]> {
        if !self.glide {
            return None;
        }
        let tick = self.live.tick.max(0) as u32;
        self.strides.get(&id)?.at_tick(tick, self.fraction)
    }

    pub fn drawn_motion(&self, id: u32) -> Option<ObjectMotion> {
        let live = self.live.units.get(&id)?;
        let mut motion = match self.history.get(&id) {
            Some(history) => ObjectMotion::interpolated(history, live, self.fraction),
            None => *live,
        };
        if let Some(world) = self.glided_world(id) {
            motion.world = world;
        }
        Some(motion)
    }

    pub fn drawn_position(&self, unit: &(impl UnitPose + ?Sized)) -> [f32; 3] {
        match self.drawn_motion(unit.id()) {
            Some(m) => self.live.render_position(m.world),
            None => unit.position(),
        }
    }

    pub fn drawn_world(&self, unit: &(impl UnitPose + ?Sized)) -> [i32; 3] {
        match self.drawn_motion(unit.id()) {
            Some(m) => m.world,
            None => {
                let [x, y, z] = self.drawn_position(unit);
                let (wx, wy) = self.live.world_ground_point([x, z]);
                [
                    wx,
                    wy,
                    (y * mandrake_tag::MarkerEntry::WORLD_ONE).round() as i32,
                ]
            }
        }
    }

    pub fn bob_height(&self, id: u32) -> i32 {
        self.engine
            .as_ref()
            .and_then(|e| e.unit(id))
            .map_or(0, |u| self.bob_height_for(u))
    }

    pub fn mirrored(&self, id: u32) -> bool {
        self.engine
            .as_ref()
            .and_then(|e| e.unit(id))
            .is_some_and(|u| u.mirrored)
    }

    pub fn camera_ground_point(&self, camera_eye: [f32; 3]) -> (i32, i32) {
        self.live.world_ground_point([camera_eye[0], camera_eye[2]])
    }

    pub fn relative_angle(&self, unit: &(impl UnitPose + ?Sized), camera_eye: [f32; 3]) -> u16 {
        let [x, y, _] = self.drawn_world(unit);
        crate::scenery::camera_relative_angle(
            (x, y),
            self.drawn_facing(unit),
            self.camera_ground_point(camera_eye),
        )
    }

    pub fn drawn_facing(&self, unit: &(impl UnitPose + ?Sized)) -> u16 {
        self.drawn_motion(unit.id())
            .map_or_else(|| unit.engine_facing(), |m| m.facing)
    }

    fn engine_object<'a>(
        &'a self,
        id: u32,
        visual: Option<&UnitSpriteVisual>,
    ) -> Option<&'a EngineIdleUnit> {
        let engine = self.engine.as_ref()?;
        let unit = engine.unit(id)?;
        let loaded =
            visual.is_none_or(|v| v.sequences.contains_key(&usize::from(unit.object.sequence)));
        let shown = if engine.running() {
            true
        } else {
            unit.is_idle() && !self.diverged.contains(&id)
        };
        (shown && !unit.object.no_shape && unit.object.sequence != 0xff && loaded).then_some(unit)
    }

    pub fn tick(
        &mut self,
        units: impl Iterator<Item = (u32, FourCC, UnitState)>,
        visuals: &BTreeMap<FourCC, UnitSpriteVisual>,
    ) -> Vec<(u32, SequenceEvents)> {
        let mut events = Vec::new();
        let mut seen = Vec::new();
        let running = self.engine.as_ref().is_some_and(EngineIdleModel::running);
        let mut units_vec: Vec<(u32, FourCC, UnitState)> = units.collect();
        units_vec.sort_by_key(|&(id, ..)| id);
        for (id, archetype, state) in units_vec {
            seen.push(id);
            let Some(visual) = visuals.get(&archetype) else {
                continue;
            };
            if let Some(shape) = self.engine.as_ref().and_then(|e| e.shape(id))
                && visual.sequences.contains_key(&usize::from(shape.sequence))
            {
                let ev = self
                    .engine
                    .as_ref()
                    .and_then(|e| e.unit(id))
                    .map(|u| engine_events(u.object.anim_flags))
                    .unwrap_or_default();
                self.states.insert(
                    id,
                    UnitAnim::Engine {
                        sequence: usize::from(shape.sequence),
                        frame: shape.frame.max(0) as usize,
                        view: shape.view,
                    },
                );
                if ev != SequenceEvents::default() {
                    events.push((id, ev));
                }
                continue;
            }
            if !running && state != UnitState::Idle && self.engine_object(id, None).is_some() {
                self.diverged.insert(id);
            }
            if (running || state == UnitState::Idle)
                && let Some(unit) = self.engine_object(id, Some(visual))
            {
                let ev = engine_events(unit.object.anim_flags);
                self.states.insert(
                    id,
                    UnitAnim::Engine {
                        sequence: usize::from(unit.object.sequence),
                        frame: unit.object.frame.max(0) as usize,
                        view: unit.object.view,
                    },
                );
                if ev != SequenceEvents::default() {
                    events.push((id, ev));
                }
                continue;
            }
            let Some(target) = visual.sequence_for_slot(slot_for_state(state)) else {
                continue;
            };
            let anim = self.states.entry(id).or_insert_with(|| {
                UnitAnim::Playback(SequencePlayback::start(
                    &target.sequence,
                    mandrake_tag::NORMAL_SPEED,
                    false,
                ))
            });
            let playback = match anim {
                UnitAnim::Playback(p) => p,
                UnitAnim::Engine { .. } => {
                    *anim = UnitAnim::Playback(SequencePlayback::start(
                        &target.sequence,
                        mandrake_tag::NORMAL_SPEED,
                        false,
                    ));
                    let UnitAnim::Playback(p) = anim else {
                        unreachable!()
                    };
                    p
                }
            };
            playback.switch_to(&target.sequence);
            let ev = playback.tick(&target.sequence);
            if ev != SequenceEvents::default() {
                events.push((id, ev));
            }
        }
        self.states.retain(|id, _| seen.binary_search(id).is_ok());
        events
    }

    pub fn tick_units<'a>(
        &mut self,
        units: impl Iterator<Item = &'a UnitEntity>,
        visuals: &BTreeMap<FourCC, UnitSpriteVisual>,
    ) -> Vec<(u32, SequenceEvents)> {
        self.tick(units.map(|u| (u.id, u.archetype, u.state)), visuals)
    }

    #[cfg(test)]
    pub fn frame_of(&self, id: u32) -> usize {
        match self.states.get(&id) {
            Some(UnitAnim::Playback(p)) => p.frame,
            Some(UnitAnim::Engine { frame, .. }) => *frame,
            None => self
                .engine_object(id, None)
                .map_or(0, |u| u.object.frame.max(0) as usize),
        }
    }

    pub fn sequence_of(&self, id: u32) -> Option<usize> {
        match self.states.get(&id) {
            Some(UnitAnim::Playback(p)) => Some(p.sequence),
            Some(UnitAnim::Engine { sequence, .. }) => Some(*sequence),
            None => self
                .engine_object(id, None)
                .map(|u| usize::from(u.object.sequence)),
        }
    }

    fn live_shape_with(&self, id: u32, engine_unit: Option<&EngineIdleUnit>) -> Option<DrawnShape> {
        let state = self.states.get(&id);
        let fallback = || {
            let engine = self.engine.as_ref()?;
            let unit = engine_unit?;
            let shown = if engine.running() {
                true
            } else {
                unit.is_idle() && !self.diverged.contains(&id)
            };
            (shown && !unit.object.no_shape && unit.object.sequence != 0xff).then_some(unit)
        };
        let (sequence, frame, view) = match state {
            Some(UnitAnim::Playback(p)) => (p.sequence, p.frame, 0),
            Some(UnitAnim::Engine {
                sequence,
                frame,
                view,
            }) => (*sequence, *frame, *view),
            None => {
                let u = fallback()?;
                (
                    usize::from(u.object.sequence),
                    u.object.frame.max(0) as usize,
                    u.object.view,
                )
            }
        };
        let shading_slot = engine_unit.map_or(0, |u| u.object.shading_slot);
        Some(DrawnShape {
            sequence,
            frame,
            view,
            shading_slot,
        })
    }

    fn live_shape(&self, id: u32) -> Option<DrawnShape> {
        let engine_unit = self.engine.as_ref().and_then(|e| e.unit(id));
        self.live_shape_with(id, engine_unit)
    }

    fn bob_height_for(&self, u: &EngineIdleUnit) -> i32 {
        let at = |tick: i32| {
            mandrake_view::projectile::bob_offset(
                u.object.flags,
                u.object.index,
                tick,
                &mandrake_math::engine::trig_tables().cosine,
            )
        };
        let live = at(self.live.tick);
        if self.fraction <= 0 || self.fraction > 0xffff {
            return live;
        }
        let next = at(self.live.tick.wrapping_add(1));
        i32::from(crate::object_history::interpolate_i16(
            live as i16,
            next as i16,
            self.fraction,
        ))
    }

    pub fn drawn(&self, unit: &(impl UnitPose + ?Sized)) -> DrawnUnit {
        let id = unit.id();
        let live_motion = self.live.units.get(&id);
        let motion = live_motion.map(|live| {
            let mut m = match self.history.get(&id) {
                Some(history) => ObjectMotion::interpolated(history, live, self.fraction),
                None => *live,
            };
            if let Some(world) = self.glided_world(id) {
                m.world = world;
            }
            m
        });
        let (position, world, facing) = match motion {
            Some(m) => (self.live.render_position(m.world), m.world, m.facing),
            None => {
                let pos = unit.position();
                let (wx, wy) = self.live.world_ground_point([pos[0], pos[2]]);
                let wz = (pos[1] * mandrake_tag::MarkerEntry::WORLD_ONE).round() as i32;
                (pos, [wx, wy, wz], unit.engine_facing())
            }
        };
        let engine = self.engine.as_ref();
        let engine_unit = engine.and_then(|e| e.unit(id));
        let shape = if shows_history_shape(self.fraction)
            && let Some(shape) = self.shape_history.get(&id)
        {
            Some(*shape)
        } else {
            self.live_shape_with(id, engine_unit)
        };
        let flags = engine_unit.map_or(0, |u| u.object.flags | u32::from(u.mirrored) << 17);
        let mirrored = engine_unit.is_some_and(|u| u.mirrored);
        let bob_height = engine_unit.map_or(0, |u| self.bob_height_for(u));
        let visibility = match (engine, engine_unit, live_motion) {
            (Some(engine), Some(u), Some(live)) => {
                let vis_flags = u.object.flags | if u.object.no_shape { 0x200 } else { 0 };
                Some(engine.object_visibility(
                    vis_flags,
                    u.object.kind,
                    (live.world[0], live.world[1]),
                ))
            }
            _ => None,
        };
        DrawnUnit {
            motion,
            shape,
            world,
            position,
            facing,
            flags,
            mirrored,
            bob_height,
            visibility,
        }
    }

    pub(crate) fn engine_model(&self) -> Option<&EngineIdleModel> {
        self.engine.as_ref()
    }

    pub fn hidden(&self, id: u32) -> bool {
        let no_shape = self
            .engine
            .as_ref()
            .and_then(|e| e.unit(id))
            .is_some_and(|u| u.object.no_shape);
        no_shape || self.visibility(id).is_some_and(|v| !v.drawn)
    }

    pub fn visibility(&self, id: u32) -> Option<minimap::ObjectVisibility> {
        let engine = self.engine.as_ref()?;
        let unit = engine.unit(id)?;
        let at = self.live.units.get(&id)?.world;
        let flags = unit.object.flags | if unit.object.no_shape { 0x200 } else { 0 };
        Some(engine.object_visibility(flags, unit.object.kind, (at[0], at[1])))
    }

    pub fn other_object_visibility(
        &self,
        flags: u32,
        kind: u8,
        at: (i32, i32),
    ) -> Option<minimap::ObjectVisibility> {
        let engine = self.engine.as_ref()?;
        Some(engine.object_visibility(flags, kind, at))
    }

    pub fn bounds_sequences(&self, id: u32) -> (Option<usize>, Option<usize>, i32) {
        let live = self.sequence_of(id);
        let history = self.shape_history.get(&id).map(|s| s.sequence);
        (history.or(live), live, self.fraction)
    }

    pub fn drawn_shape(&self, id: u32) -> Option<DrawnShape> {
        if shows_history_shape(self.fraction)
            && let Some(shape) = self.shape_history.get(&id)
        {
            return Some(*shape);
        }
        self.live_shape(id)
    }

    pub fn drawn_sequence_of(&self, id: u32) -> Option<usize> {
        self.drawn_shape(id).map(|s| s.sequence)
    }

    #[cfg(test)]
    pub fn drawn_frame_of(&self, id: u32) -> usize {
        self.drawn_shape(id).map_or(0, |s| s.frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence(
        index: usize,
        frames: u16,
        ticks: i16,
        views: u16,
        flags: u32,
    ) -> CollectionSequence {
        CollectionSequence {
            index,
            name: format!("seq{index}"),
            flags,
            pixels_to_world: 1,
            num_views: views,
            frames_per_view: frames,
            ticks_per_frame: ticks,
            key_frame: -1,
            loop_frame: 0,
            bitmap_indices: vec![0; frames as usize * views as usize],
            shadow_indices: vec![-1; frames as usize],
            bounds: Default::default(),
        }
    }

    fn visual(seq: CollectionSequence) -> SequenceVisual {
        let stored =
            mandrake_tag::displayed_views(seq.num_views as usize).max(seq.num_views as usize);
        let frames = (0..seq.frames_per_view as usize)
            .map(|f| FrameVisual {
                views: (0..stored)
                    .map(|v| {
                        Some(ViewSprite {
                            atlas_index: f * 100 + v,
                            world_size: [1.0, 1.0],
                        })
                    })
                    .collect(),
                shadow: None,
                shadow_sources: Vec::new(),
            })
            .collect();
        SequenceVisual {
            object_bounds: seq.bounds.world(),
            sequence: seq,
            frames,
        }
    }

    fn unit_visual() -> UnitSpriteVisual {
        let mut v = UnitSpriteVisual::default();
        v.slots[MonsterSequenceSlot::STANDING.0] = Some(6);
        v.slots[MonsterSequenceSlot::MOVING.0] = Some(7);
        v.sequences.insert(6, visual(sequence(6, 2, 30, 8, 2)));
        v.sequences.insert(7, visual(sequence(7, 8, 2, 8, 2)));
        v
    }

    #[test]
    fn states_map_to_standing_and_moving_slots() {
        assert_eq!(
            slot_for_state(UnitState::Idle),
            MonsterSequenceSlot::STANDING
        );
        assert_eq!(
            slot_for_state(UnitState::Moving),
            MonsterSequenceSlot::MOVING
        );
        assert_eq!(
            slot_for_state(UnitState::Attacking),
            MonsterSequenceSlot::STANDING
        );
        assert_eq!(
            slot_for_state(UnitState::Dead),
            MonsterSequenceSlot::STANDING
        );
    }

    #[test]
    fn empty_slot_falls_back_to_standing() {
        let v = unit_visual();
        let seq = v.sequence_for_slot(MonsterSequenceSlot(5)).unwrap();
        assert_eq!(seq.sequence.index, 6);
    }

    #[test]
    fn walking_unit_plays_walk_frames_at_authored_rate() {
        let mut visuals = BTreeMap::new();
        let arch = FourCC::new(*b"wam1");
        visuals.insert(arch, unit_visual());
        let mut anim = UnitAnimator::new();
        let mut frames = Vec::new();
        for _ in 0..6 {
            anim.tick([(1, arch, UnitState::Moving)].into_iter(), &visuals);
            frames.push(anim.frame_of(1));
        }
        assert_eq!(frames, vec![0, 1, 1, 2, 2, 3]);
        assert_eq!(anim.sequence_of(1), Some(7));
    }

    #[test]
    fn switching_state_restarts_in_the_new_sequence() {
        let mut visuals = BTreeMap::new();
        let arch = FourCC::new(*b"wam1");
        visuals.insert(arch, unit_visual());
        let mut anim = UnitAnimator::new();
        for _ in 0..40 {
            anim.tick([(1, arch, UnitState::Idle)].into_iter(), &visuals);
        }
        assert_eq!(anim.sequence_of(1), Some(6));
        anim.tick([(1, arch, UnitState::Moving)].into_iter(), &visuals);
        assert_eq!(anim.sequence_of(1), Some(7));
        assert_eq!(anim.frame_of(1), 0);
    }

    #[test]
    fn removed_units_drop_their_state() {
        let mut visuals = BTreeMap::new();
        let arch = FourCC::new(*b"wam1");
        visuals.insert(arch, unit_visual());
        let mut anim = UnitAnimator::new();
        anim.tick([(1, arch, UnitState::Idle)].into_iter(), &visuals);
        anim.tick(std::iter::empty(), &visuals);
        assert_eq!(anim.sequence_of(1), None);
    }

    #[test]
    fn unit_facing_the_camera_uses_view_zero() {
        let seq = visual(sequence(7, 8, 2, 8, 2));
        let (sprite, flip) = select_view(&seq, 0, 0, 0, false).unwrap();
        assert_eq!(sprite.atlas_index, 0);
        assert!(!flip);
    }

    #[test]
    fn unit_facing_away_uses_the_opposite_view() {
        let seq = visual(sequence(7, 8, 2, 8, 2));
        let (sprite, _) = select_view(&seq, 0, 0, 0x8000, false).unwrap();
        assert_eq!(sprite.atlas_index, 4);
    }

    #[test]
    fn views_beyond_the_stored_half_are_mirrored_when_flagged() {
        let seq = visual(sequence(7, 8, 2, 8, 3));
        let (sprite, flip) = select_view(&seq, 0, 0, 0xc000, false).unwrap();
        assert_eq!(sprite.atlas_index, 2);
        assert!(flip);
    }

    #[test]
    fn a_mirrored_object_negates_the_angle_and_mirrors_the_sprite() {
        let seq = visual(sequence(7, 8, 2, 8, 3));
        let (s, flip) = select_view(&seq, 0, 0, 0x2000, false).unwrap();
        assert_eq!((s.atlas_index, flip), (1, false));
        let (s, flip) = select_view(&seq, 0, 0, 0x2000, true).unwrap();
        assert_eq!((s.atlas_index, flip), (1, false));
        let (s, flip) = select_view(&seq, 0, 0, 0x8000, false).unwrap();
        assert_eq!((s.atlas_index, flip), (4, false));
        let (s, flip) = select_view(&seq, 0, 0, 0x8000, true).unwrap();
        assert_eq!((s.atlas_index, flip), (4, true));
    }

    #[test]
    fn a_stored_view_sequence_ignores_the_camera() {
        let seq = visual(sequence(7, 8, 2, 8, 4));
        for relative in [0u16, 0x4000, 0x8000] {
            let (s, _) = select_view(&seq, 0, 3, relative, false).unwrap();
            assert_eq!(s.atlas_index, 3);
        }
    }

    #[test]
    fn a_frame_draws_the_history_copy_by_fraction() {
        let mut animator = UnitAnimator::new();
        let id = 9;
        let at = |x: i32, facing: u16| ObjectMotion {
            world: [x, 0, 0],
            facing,
            ..Default::default()
        };
        let mut before = UnitMotions::default();
        before.units.insert(id, at(100, 0x1000));
        animator.seed_motions(before.clone());
        animator.states.insert(
            id,
            UnitAnim::Engine {
                sequence: 3,
                frame: 1,
                view: 0,
            },
        );
        let mut after = before.clone();
        after.units.insert(id, at(200, 0x2000));
        animator.replay_tick(None, after);
        animator.states.insert(
            id,
            UnitAnim::Engine {
                sequence: 3,
                frame: 2,
                view: 0,
            },
        );
        assert_eq!(animator.drawn_motion(id).unwrap().world[0], 100);
        assert_eq!(animator.drawn_frame_of(id), 1);
        animator.set_fraction(0x8000);
        assert_eq!(animator.drawn_motion(id).unwrap().world[0], 150);
        assert_eq!(animator.drawn_motion(id).unwrap().facing, 0x1800);
        assert_eq!(animator.drawn_frame_of(id), 2);
        animator.set_fraction(crate::object_history::FRACTION_ONE);
        assert_eq!(animator.drawn_motion(id).unwrap().world[0], 200);
    }

    #[test]
    fn a_new_unit_is_drawn_live() {
        let mut animator = UnitAnimator::new();
        let mut after = UnitMotions::default();
        after.units.insert(
            5,
            ObjectMotion {
                world: [300, 0, 0],
                ..Default::default()
            },
        );
        animator.replay_tick(None, after);
        assert_eq!(animator.drawn_motion(5).unwrap().world[0], 300);
    }

    #[test]
    fn a_moving_unit_plays_walk_frames_then_stands() {
        let arch = FourCC::new(*b"wam1");
        let mut visuals = BTreeMap::new();
        visuals.insert(arch, unit_visual());
        let id = 1;
        let mut anim = UnitAnimator::new();
        let mut walk_frames = std::collections::BTreeSet::new();
        for _ in 0..60 {
            anim.tick([(id, arch, UnitState::Moving)].into_iter(), &visuals);
            assert_eq!(anim.sequence_of(id), Some(7), "moving unit must play walk");
            walk_frames.insert(anim.frame_of(id));
        }
        assert_eq!(
            walk_frames.len(),
            8,
            "every walk frame is shown: {walk_frames:?}"
        );
        anim.tick([(id, arch, UnitState::Idle)].into_iter(), &visuals);
        assert_eq!(anim.sequence_of(id), Some(6), "arrived unit stands again");
    }

    #[test]
    fn frame_selects_that_frames_sprite() {
        let seq = visual(sequence(7, 8, 2, 8, 2));
        let (sprite, _) = select_view(&seq, 3, 0, 0, false).unwrap();
        assert_eq!(sprite.atlas_index, 300);
    }

    use mandrake_math::BungieRng;
    use mandrake_sim::engine_rng::{
        EngineMonster, EngineRng, EngineWorld, MonsterDefinition, SiteCounts,
    };

    const IDLE_COLLECTION: FourCC = FourCC::new(*b"tidl");
    const IDLE_ID: u32 = 7;

    fn idle_headers() -> Vec<SequenceHeader> {
        [3, 2, 4, 2]
            .into_iter()
            .map(|frames| SequenceHeader {
                number_of_views: 1,
                frames_per_view: frames,
                ticks_per_frame: 1,
                key_frame: -1,
                loop_frame: 0,
                ..Default::default()
            })
            .collect()
    }

    fn idle_definition(secondary: bool) -> MonsterDefinition {
        let mut mons = vec![0u8; 0x224];
        let words: [(usize, i16); 4] = if secondary {
            [(0x08, 0), (0x1c, 1), (0x1e, 2), (0x20, 3)]
        } else {
            [(0x08, 0), (0x1c, -1), (0x1e, -1), (0x20, -1)]
        };
        for (o, v) in words {
            mons[o..o + 2].copy_from_slice(&v.to_be_bytes());
        }
        MonsterDefinition::from_bytes(FourCC::new(*b"tmon"), &mons).unwrap()
    }

    fn idle_object() -> EngineObject {
        let mut o = EngineObject::new(0, IDLE_COLLECTION);
        o.sequence = 0;
        o.ticks_per_frame = 3;
        o
    }

    fn idle_sim(seed: u32, secondary: bool) -> Simulation {
        let mut world = EngineWorld::default();
        world.collections.insert(IDLE_COLLECTION, idle_headers());
        world.monster_definitions.push(idle_definition(secondary));
        world.monsters.push(EngineMonster {
            index: 0,
            identifier: IDLE_ID as i16,
            first_draw: 0,
            definition: 0,
            object: idle_object(),
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position: (0, 0),
            flags: 0,
            flags_8: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team: 0,
            owner: 0,
            facing: 0,
            impassable: 0,
            neighbours: 0,
            formation: Default::default(),
            cell_link: 0,
            flying_height: 0,
        });
        let mut sim = Simulation::new(seed);
        sim.set_engine_world(world);
        sim
    }

    #[test]
    fn a_monster_with_no_shape_is_hidden() {
        let mut sim = idle_sim(1, false);
        let mut animator = engine_animator(&sim);
        assert!(!animator.hidden(IDLE_ID));
        assert!(!animator.hidden(IDLE_ID + 1), "not a monster of the model");
        let set_no_shape = |sim: &mut Simulation, no_shape| {
            sim.engine.monsters[0].object.no_shape = no_shape;
        };
        set_no_shape(&mut sim, true);
        animator.observe(&sim);
        assert!(animator.hidden(IDLE_ID));
        set_no_shape(&mut sim, false);
        animator.observe(&sim);
        assert!(!animator.hidden(IDLE_ID));
    }

    fn engine_animator(sim: &Simulation) -> UnitAnimator {
        UnitAnimator::with_engine_idle(Some(EngineIdleModel::new(sim).unwrap()))
    }

    fn shape(sequence: u8, frame: i8) -> ObjectShape {
        ObjectShape {
            collection: IDLE_COLLECTION,
            sequence,
            frame,
            view: 0,
            facing: 0,
            permutation: 0,
            mirrored: false,
            z: 0,
        }
    }

    #[test]
    fn the_reported_shape_is_drawn_per_event() {
        let sim = idle_sim(1, false);
        let visuals = idle_visuals();
        let archetype = FourCC::new(*b"tmon");
        for (event, state, seq, frame) in [
            ("attack", UnitState::Attacking, 2u8, 4i8),
            ("flinch", UnitState::Idle, 3, 0),
            ("celebrate", UnitState::Idle, 1, 2),
        ] {
            let mut animator = engine_animator(&sim);
            animator
                .engine
                .as_mut()
                .unwrap()
                .shapes
                .insert(IDLE_ID, shape(seq, frame));
            animator.tick([(IDLE_ID, archetype, state)].into_iter(), &visuals);
            assert_eq!(
                animator.sequence_of(IDLE_ID),
                Some(usize::from(seq)),
                "{event}"
            );
            assert_eq!(animator.frame_of(IDLE_ID), frame as usize, "{event}");
        }
        let mut animator = engine_animator(&sim);
        animator
            .engine
            .as_mut()
            .unwrap()
            .shapes
            .insert(IDLE_ID, shape(90, 1));
        animator.tick(
            [(IDLE_ID, archetype, UnitState::Idle)].into_iter(),
            &visuals,
        );
        assert_ne!(animator.sequence_of(IDLE_ID), Some(90));
    }

    fn idle_visuals() -> BTreeMap<FourCC, UnitSpriteVisual> {
        let mut v = UnitSpriteVisual::default();
        v.slots[MonsterSequenceSlot::STANDING.0] = Some(0);
        for (slot, seq) in [(0x1c - 0x08) / 2, (0x1e - 0x08) / 2, (0x20 - 0x08) / 2]
            .map(MonsterSequenceSlot)
            .into_iter()
            .zip(1..)
        {
            v.slots[slot.0] = Some(seq);
        }
        for (i, h) in idle_headers().iter().enumerate() {
            v.sequences.insert(
                i,
                visual(sequence(
                    i,
                    h.frames_per_view as u16,
                    h.ticks_per_frame,
                    1,
                    0,
                )),
            );
        }
        BTreeMap::from([(FourCC::new(*b"tmon"), v)])
    }

    struct ReferenceIdle {
        rng: BungieRng,
        sites: SiteCounts,
        object: EngineObject,
        state: u8,
        def: MonsterDefinition,
        headers: Vec<SequenceHeader>,
    }

    impl ReferenceIdle {
        fn new(seed: u32, secondary: bool) -> Self {
            Self {
                rng: BungieRng::new(seed),
                sites: SiteCounts::default(),
                object: idle_object(),
                state: 0x1a,
                def: idle_definition(secondary),
                headers: idle_headers(),
            }
        }

        fn tick(&mut self) {
            engine_rng::animate_object(&mut self.object, &self.headers);
            let mut rng = EngineRng {
                rng: &mut self.rng,
                sites: &mut self.sites,
            };
            let f = self.object.anim_flags;
            let d = &self.def;
            let next = match self.state {
                0x1b if f & 4 != 0 => Some((d.idle_sequence, 0x100, 0x1a)),
                0x1d if f & 4 != 0 => Some((d.idle_sequence_1e, 0x100, 0x1c)),
                0x1a | 0x1c if f & 1 != 0 => {
                    let r = rng.draw(mandrake_sim::draw_site::IDLE_CHANCE);
                    if r <= 0x3fff && d.idle_sequence_1e != -1 {
                        Some(if self.state == 0x1a {
                            (d.idle_sequence_1c, 0x100, 0x1d)
                        } else {
                            (d.idle_sequence_20, 0x100, 0x1b)
                        })
                    } else {
                        let r2 = rng.draw(mandrake_sim::draw_site::IDLE_SEQUENCE);
                        Some((
                            i16::from(self.object.sequence),
                            0x300 + (i32::from(r2) >> 9),
                            self.state,
                        ))
                    }
                }
                _ => None,
            };
            if let Some((seq, speed, state)) = next {
                self.state = state;
                engine_rng::set_object_shape(
                    &mut self.object,
                    &self.headers,
                    i32::from(seq),
                    speed,
                    0x21,
                    &mut rng,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn engine_idle_switches_to_the_secondary_animations_and_back() {
        let seed = 0x1234;
        let visuals = idle_visuals();
        let arch = FourCC::new(*b"tmon");
        let mut sim = idle_sim(seed, true);
        let mut anim = engine_animator(&sim);
        let mut reference = ReferenceIdle::new(seed, true);
        let mut states = std::collections::BTreeSet::new();
        let mut transitions = std::collections::BTreeSet::new();
        let mut speeds = std::collections::BTreeSet::new();
        for t in 0..600 {
            let before = reference.state;
            reference.tick();
            sim.step();
            anim.observe(&sim);
            anim.tick([(IDLE_ID, arch, UnitState::Idle)].into_iter(), &visuals);
            let engine = anim.engine.as_ref().unwrap().unit(IDLE_ID).unwrap();
            assert_eq!(engine.object, reference.object, "tick {t}: engine object");
            assert_eq!(engine.anim_state, reference.state, "tick {t}: state");
            assert_eq!(
                anim.sequence_of(IDLE_ID),
                Some(usize::from(reference.object.sequence)),
                "tick {t}"
            );
            assert_eq!(anim.frame_of(IDLE_ID), reference.object.frame as usize);
            states.insert(reference.state);
            speeds.insert(reference.object.ticks_per_frame);
            if before != reference.state {
                transitions.insert((before, reference.state));
            }
        }
        assert!(anim.engine.as_ref().unwrap().stop().is_none());
        assert_eq!(states, [0x1a, 0x1b, 0x1c, 0x1d].into());
        assert_eq!(
            transitions,
            [(0x1a, 0x1d), (0x1d, 0x1c), (0x1c, 0x1b), (0x1b, 0x1a)].into()
        );
        assert_eq!(speeds, [1, 3].into());
        assert_eq!(
            sim.engine_sites, reference.sites,
            "same draws at the same sites"
        );
        assert!(reference.sites.at(&[mandrake_sim::draw_site::IDLE_CHANCE]) > 50);
        assert!(
            reference
                .sites
                .at(&[mandrake_sim::draw_site::OBJECT_SHAPE_FRAME])
                > 50
        );
    }

    #[test]
    fn engine_idle_without_secondaries_only_restarts_the_loop() {
        let seed = 99;
        let visuals = idle_visuals();
        let arch = FourCC::new(*b"tmon");
        let mut sim = idle_sim(seed, false);
        let mut anim = engine_animator(&sim);
        let mut reference = ReferenceIdle::new(seed, false);
        for _ in 0..300 {
            reference.tick();
            sim.step();
            anim.observe(&sim);
            anim.tick([(IDLE_ID, arch, UnitState::Idle)].into_iter(), &visuals);
            assert_eq!(reference.state, 0x1a);
            assert_eq!(anim.sequence_of(IDLE_ID), Some(0));
            assert_eq!(anim.frame_of(IDLE_ID), reference.object.frame as usize);
        }
        assert_eq!(
            reference.sites.at(&[mandrake_sim::draw_site::IDLE_CHANCE]),
            reference
                .sites
                .at(&[mandrake_sim::draw_site::IDLE_SEQUENCE])
        );
    }

    #[test]
    fn after_the_engine_stop_a_unit_the_client_moves_leaves_the_engine_idle() {
        let visuals = idle_visuals();
        let arch = FourCC::new(*b"tmon");
        let mut sim = idle_sim(5, true);
        let mut anim = engine_animator(&sim);
        let tick = |sim: &mut Simulation, anim: &mut UnitAnimator, state| {
            sim.step();
            anim.observe(sim);
            anim.tick([(IDLE_ID, arch, state)].into_iter(), &visuals);
        };
        for _ in 0..10 {
            tick(&mut sim, &mut anim, UnitState::Idle);
        }
        assert!(matches!(anim.states[&IDLE_ID], UnitAnim::Engine { .. }));
        tick(&mut sim, &mut anim, UnitState::Moving);
        assert!(matches!(anim.states[&IDLE_ID], UnitAnim::Engine { .. }));
        sim.engine_stop = Some(Unmodelled::new(0, "test stop"));
        tick(&mut sim, &mut anim, UnitState::Idle);
        assert!(matches!(anim.states[&IDLE_ID], UnitAnim::Engine { .. }));
        tick(&mut sim, &mut anim, UnitState::Moving);
        tick(&mut sim, &mut anim, UnitState::Idle);
        assert!(matches!(anim.states[&IDLE_ID], UnitAnim::Playback(_)));
        assert_eq!(anim.sequence_of(IDLE_ID), Some(0));
    }

    #[test]
    fn a_selection_follows_the_renamed_units() {
        let mut ids = vec![5, 10269, 10270, 7];
        follow_renamed(&mut ids, &[(10269, 0x8000)]);
        assert_eq!(ids, [5, 0x8000, 10270, 7]);
        follow_renamed(&mut ids, &[(10270, 0x8000)]);
        assert_eq!(ids, [5, 0x8000, 7]);
    }
}
