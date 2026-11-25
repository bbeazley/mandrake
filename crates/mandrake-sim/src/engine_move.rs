use std::sync::OnceLock;

use mandrake_common::FourCC;
use mandrake_tag::{SequenceHeader, get_sequence};

use crate::draw_site;
use crate::engine_rng::Unmodelled;
use crate::passability::{MeshCells, MonsterPassability, move_monster};

pub const MIN_LEG: i32 = 0x54;
pub const EDGE_MARGIN: i32 = 0x3e0;

pub struct TrigTables {
    pub cosine: [i16; 1024],
    pub sine: [i16; 1024],
    pub tangent: [i32; 1024],
}

pub fn table_sin_cos(i: usize) -> (f64, f64) {
    let a = i as f64 * std::f64::consts::TAU * 0.000_976_562_5;
    a.sin_cos()
}

pub fn trig_tables() -> &'static TrigTables {
    static TABLES: OnceLock<TrigTables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut t = TrigTables {
            cosine: [0; 1024],
            sine: [0; 1024],
            tangent: [0; 1024],
        };
        for i in 0..1024usize {
            let (s, c) = match i {
                0x000 => (0, 0x4000),
                0x100 => (0x4000, 0),
                0x200 => (0, -0x4000),
                0x300 => (-0x4000, 0),
                _ => {
                    let (sin, cos) = table_sin_cos(i);
                    ((sin * 16384.0 + 0.5) as i32, (cos * 16384.0 + 0.5) as i32)
                }
            };
            t.sine[i] = s as i16;
            t.cosine[i] = c as i16;
            t.tangent[i] = match i {
                0x200 => 0,
                _ if c as i16 == 0 => i32::MIN,
                _ => (i32::from(s as i16) << 14) / i32::from(c as i16),
            };
        }
        t
    })
}

pub fn angle_index(angle: u16) -> usize {
    (angle.wrapping_add(0x20) >> 6) as usize
}

pub fn arctangent(x: i32, y: i32) -> u16 {
    if x == 0 {
        return if y < 0 { 0xc000 } else { 0x4000 };
    }
    let t = y.wrapping_shl(14) / x;
    if t == 0 {
        return if x < 0 { 0x8000 } else { 0 };
    }
    let base = if y > 0 { 0 } else { 512 } + if t < 0 { 256 } else { 0 };
    let tab = &trig_tables().tangent[base..];
    let (mut lo, mut count) = (1usize, 255usize);
    while count > 0 {
        let step = count >> 1;
        let mid = lo + step;
        if t > tab[mid] {
            lo = mid + 1;
            count -= step + 1;
        } else {
            count = step;
        }
    }
    if lo <= 0xff && tab[lo].wrapping_sub(t) > t.wrapping_sub(tab[lo - 1]) {
        lo -= 1;
    }
    (((lo + base) << 6) & 0xffff) as u16
}

pub fn distance2d(dx: i32, dy: i32) -> i32 {
    let in_range = |v: i32| (-0x7fff..=0x7ffe).contains(&v);
    if !in_range(dx) || !in_range(dy) {
        return 0x7fff;
    }
    let s = (dx * dx) as u32 as u64 + (dy * dy) as u32 as u64;
    let s = s as u32 as u64;
    let mut r = (s as f64).sqrt() as u64;
    while r * r > s {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= s {
        r += 1;
    }
    if s - r * r > r {
        r += 1;
    }
    (r as i32).min(0x7fff)
}

pub fn signed_angular_difference(from: u16, to: u16) -> i32 {
    let mut d = i32::from(to) - i32::from(from);
    if d >= 0x8000 {
        d -= 0x10000;
    }
    if d <= -0x8000 {
        d += 0x10000;
    }
    d
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MovementDefinition {
    pub tag: FourCC,
    pub per_tick: bool,
    pub speed: i16,
    pub turn_rate: u16,
    pub terrain_speed: [u8; 16],
    pub impassable: u16,
    pub flags: u32,
    pub slide_without_nudge: bool,
    pub walk_half_width: i16,
    pub walking_sequence: i16,
    pub turning_sequence: i16,
    pub walk_ticks_per_frame: i8,
    pub turn_ticks_per_frame: i8,
    pub field_198_clear: bool,
    pub run_sequence: i16,
    pub run_bonus: i8,
    pub field_198_bit4: bool,
    #[serde(default)]
    pub field_198_bit8: bool,
    pub pathfinding_radius: i16,
    pub field_82: i16,
    pub field_1b5: u8,
    pub class: i16,
}

impl MovementDefinition {
    pub fn path_view(&self) -> crate::pathfinding::DefinitionView {
        crate::pathfinding::DefinitionView {
            flags: self.flags,
            impassable: self.impassable,
            radius: self.pathfinding_radius,
            field_82: self.field_82,
            field_1b5: self.field_1b5,
            class: self.class,
        }
    }

    pub fn from_tags(
        tag: FourCC,
        mons: &[u8],
        headers: &[SequenceHeader],
        gameplay_version: i32,
    ) -> Result<Self, String> {
        if mons.len() < 0x224 {
            return Err(format!("mons {tag} too short ({} bytes)", mons.len()));
        }
        let be16 = |o: usize| i16::from_be_bytes([mons[o], mons[o + 1]]);
        let tpf = |s: i16| -> Result<i8, String> {
            if s == -1 {
                return Ok(1);
            }
            let h = get_sequence(headers, i32::from(s))
                .ok_or_else(|| format!("mons {tag}: no sequences in its collection"))?;
            let t = ((i32::from(h.ticks_per_frame) * 0x100) >> 8) as i8;
            Ok(if t <= 0 { 1 } else { t })
        };
        let walking_sequence = be16(0x0a);
        let turning_sequence = be16(0x10);
        let mut terrain_speed = [0u8; 16];
        terrain_speed.copy_from_slice(&mons[0x5c..0x6c]);
        let terrain = mandrake_tag::unit::MonsterTerrain::from_mons_bytes(mons)
            .map_err(|e| format!("mons {tag}: {e}"))?;
        let walk_half_width = if walking_sequence == -1 {
            0
        } else {
            get_sequence(headers, i32::from(walking_sequence))
                .map_or(0, |h| h.world_bounds.half_width)
        };
        Ok(Self {
            tag,
            per_tick: mons[3] & 1 != 0,
            speed: be16(0x7e),
            turn_rate: be16(0x7c) as u16,
            terrain_speed,
            impassable: terrain.impassable_mask(gameplay_version),
            flags: terrain.flags,
            slide_without_nudge: mons[0x198] & 8 != 0,
            walk_half_width,
            walking_sequence,
            turning_sequence,
            walk_ticks_per_frame: tpf(walking_sequence)?,
            turn_ticks_per_frame: tpf(turning_sequence)?,
            field_198_clear: mons[0x198] & 4 == 0,
            run_sequence: be16(0x22),
            run_bonus: mons[0x50] as i8,
            field_198_bit4: mons[0x198] & 4 != 0,
            field_198_bit8: mons[0x198] & 8 != 0,
            pathfinding_radius: be16(0x5a),
            field_82: be16(0x82),
            field_1b5: mons[0x1b5],
            class: be16(0x1b6),
        })
    }

    pub fn creation_speed(&self) -> Option<i16> {
        ((i32::from(self.speed) * 7) >> 8 == 0).then_some(self.speed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MoveBlock {
    pub site: u32,
    pub reason: String,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MoveMode {
    Idle = 1,
    Celebrating = 2,
    Taunting = 3,
    Walking = 4,
    Turning = 5,
    Holding = 6,
    Aiming = 7,
    Attacking = 8,
    Blocking = 9,
    PickingUp = 10,
    Held = 11,
    Stunned = 12,
    LowVitality = 13,
    Paralyzed = 14,
    Flinching = 15,
}

impl MoveMode {
    pub fn byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            1 => Self::Idle,
            2 => Self::Celebrating,
            3 => Self::Taunting,
            4 => Self::Walking,
            5 => Self::Turning,
            6 => Self::Holding,
            7 => Self::Aiming,
            8 => Self::Attacking,
            9 => Self::Blocking,
            10 => Self::PickingUp,
            11 => Self::Held,
            12 => Self::Stunned,
            13 => Self::LowVitality,
            14 => Self::Paralyzed,
            15 => Self::Flinching,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeState {
    Zero,
    Keep,
    Value(i8),
}

pub struct RecordFlags<'a> {
    pub flags: &'a mut u32,
    pub flags_8: &'a mut u32,
}

pub const NEW_LEG: u32 = 0x20;
pub const DETOUR: u32 = 0x80;
pub const BACKWARDS: u32 = 0x800;
pub const FINAL_FACING: u32 = 0x1000;
pub const DETOUR_SIDE: u32 = 0x10_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detour {
    pub detour: bool,
    pub side: bool,
}

impl Detour {
    pub fn write(self, mut f: RecordFlags<'_>) {
        f.set_detour(self.detour);
        f.set_detour_side(self.side);
    }
}

impl RecordFlags<'_> {
    fn set(word: &mut u32, bit: u32, on: bool) {
        if on {
            *word |= bit;
        } else {
            *word &= !bit;
        }
    }
    pub fn needs_leg(&self) -> bool {
        *self.flags & NEW_LEG != 0
    }
    pub fn set_needs_leg(&mut self, on: bool) {
        Self::set(self.flags, NEW_LEG, on);
    }
    pub fn detour(&self) -> bool {
        *self.flags & DETOUR != 0
    }
    pub fn set_detour(&mut self, on: bool) {
        Self::set(self.flags, DETOUR, on);
    }
    pub fn backwards(&self) -> bool {
        *self.flags & BACKWARDS != 0
    }
    pub fn set_backwards(&mut self, on: bool) {
        Self::set(self.flags, BACKWARDS, on);
    }
    pub fn final_facing_pending(&self) -> bool {
        *self.flags & FINAL_FACING != 0
    }
    pub fn set_final_facing_pending(&mut self, on: bool) {
        Self::set(self.flags, FINAL_FACING, on);
    }
    pub fn detour_side(&self) -> bool {
        *self.flags_8 & DETOUR_SIDE != 0
    }
    pub fn set_detour_side(&mut self, on: bool) {
        Self::set(self.flags_8, DETOUR_SIDE, on);
    }
}

pub trait ModeHooks {
    fn flags(&mut self) -> RecordFlags<'_>;

    fn change_mode(
        &mut self,
        mover: &mut EngineMover,
        mode: MoveMode,
        state: ModeState,
    ) -> Result<Option<(i8, i8)>, Unmodelled>;

    fn order_done(&mut self, action: i16) -> Result<(), Unmodelled>;

    fn walking_becomes(&self) -> Option<MoveMode> {
        None
    }

    fn running(&self) -> bool {
        false
    }

    fn set_state(&mut self, _state: u8) {}

    fn leg_gait(&mut self, _mover: &mut EngineMover) -> Result<(), Unmodelled> {
        Ok(())
    }

    fn under_order(&self) -> bool {
        true
    }

    fn end_order(&mut self) {}

    fn plan_leg(&mut self, mover: &mut EngineMover) -> Result<(), Unmodelled>;

    fn posed(&self) -> bool {
        false
    }

    fn step(
        &mut self,
        mover: &mut EngineMover,
        terrain: Option<&MeshCells>,
    ) -> Result<u32, Unmodelled> {
        let [vx, vy, _] = mover.velocity;
        let (r, new_leg) = mover.translate(vx, vy, terrain);
        if new_leg {
            self.flags().set_needs_leg(true);
        }
        Ok(r)
    }

    fn engaged(&mut self, _mover: &mut EngineMover) -> Result<bool, Unmodelled> {
        Ok(false)
    }

    fn finish(&mut self, _mover: &mut EngineMover) -> Result<bool, Unmodelled> {
        Ok(false)
    }

    fn aimed(&mut self, _mover: &mut EngineMover) -> Result<(), Unmodelled> {
        Ok(())
    }

    fn react(&mut self, _mover: &mut EngineMover, _r: u32) -> Result<(), Unmodelled> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EngineMover {
    pub definition: MovementDefinition,
    pub x: i32,
    pub y: i32,
    pub facing: u16,
    pub desired_facing: u16,
    pub speed: i16,
    pub aim_countdown: i8,
    pub target: (i32, i32),
    pub leg_budget: i16,
    pub waypoints: Vec<(i32, i32)>,
    pub waypoint: usize,
    pub cycle: bool,
    pub back_and_forth: bool,
    #[serde(default)]
    pub word_26: u16,
    pub action: i16,
    pub mode: MoveMode,
    pub animation_state: u8,
    pub velocity: [i16; 3],
    pub frame_tick: i8,
    pub ticks_per_frame: i8,
    pub frame_advanced: bool,
    pub walk_phase: i8,
    pub passability: MonsterPassability,
    pub block: Option<MoveBlock>,
    pub path: Option<crate::pathfinding::Path>,
    pub avoid: i16,
    pub monster_index: Option<usize>,
    #[serde(default)]
    pub bump_count: i16,
    #[serde(default)]
    pub pose_timer: i16,
    #[serde(default)]
    pub step_terrain: usize,
}

impl EngineMover {
    pub fn new(definition: MovementDefinition, x: i32, y: i32, facing: u16) -> Self {
        let speed = definition.creation_speed().unwrap_or(definition.speed);
        let mut block = None;
        if definition.creation_speed().is_none() {
            block = Some(MoveBlock {
                site: draw_site::MOVE_CREATION_SPEED,
                reason: format!(
                    "mons {} speed {} is jittered by a creation draw",
                    definition.tag, definition.speed
                ),
            });
        }
        if !definition.field_198_clear {
            block.get_or_insert(MoveBlock {
                site: draw_site::MOVE_DEFINITION_198,
                reason: format!("mons {} +0x198 bit 4 is set", definition.tag),
            });
        }
        let passability = MonsterPassability {
            impassable: definition.impassable,
            neighbours: 0,
            flies: definition.flags & mandrake_tag::unit::MONSTER_FLAG_FLYING != 0,
            slide_without_nudge: definition.slide_without_nudge,
            radius: None,
        };
        Self {
            definition,
            x,
            y,
            facing,
            desired_facing: 0,
            speed,
            aim_countdown: 0,
            target: (0, 0),
            leg_budget: 0,
            waypoints: Vec::new(),
            waypoint: 0,
            cycle: false,
            back_and_forth: false,
            word_26: 0,
            action: -1,
            mode: MoveMode::Idle,
            animation_state: 0,
            velocity: [0; 3],
            frame_tick: 0,
            ticks_per_frame: 1,
            frame_advanced: false,
            walk_phase: 0,
            passability,
            block,
            path: None,
            avoid: 0,
            monster_index: None,
            bump_count: 0,
            step_terrain: 0,
            pose_timer: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn monster_view(
        &self,
        index: i16,
        definition: i16,
        object: i16,
        flags: u32,
        flags_8: u32,
        team: i8,
    ) -> crate::pathfinding::MonsterView {
        crate::pathfinding::MonsterView {
            index,
            definition,
            flags,
            flags_8,
            object,
            team,
            target: -1,
            pick_up: -1,
            pick_up_identifier: 0,
            flying_height: 0,
            speed: self.speed,
            mode: self.mode.byte(),
        }
    }

    pub fn plan_with(
        &mut self,
        cells: &MeshCells,
        world: &dyn crate::pathfinding::ObstacleWorld,
        m: crate::pathfinding::MonsterView,
        me: crate::pathfinding::ObjectView,
    ) -> Result<(crate::pathfinding::AdjustTrace, Detour), Unmodelled> {
        use crate::pathfinding::{NewPath, PathRequest, new_path};
        let mut req = PathRequest {
            flags: 0,
            impassable: self.definition.impassable,
            object: i32::from(me.index),
            start: (self.x, self.y),
            goal: self.target,
        };
        self.path = None;
        let fire = world.fire_of(me.owner);
        let path = loop {
            match new_path(cells, &fire, &mut req, Some(me.radius))? {
                NewPath::Path(p) => break p,
                NewPath::Retry => {}
            }
        };
        self.path = Some(path);
        self.target = path.point;
        self.adjust(world, m)
    }

    pub fn adjust(
        &mut self,
        world: &dyn crate::pathfinding::ObstacleWorld,
        m: crate::pathfinding::MonsterView,
    ) -> Result<(crate::pathfinding::AdjustTrace, Detour), Unmodelled> {
        let mut state = crate::pathfinding::AdjustState {
            target: self.target,
            detour: m.flags & DETOUR != 0,
            detour_side: m.flags_8 & DETOUR_SIDE != 0,
            avoid: self.avoid,
        };
        let trace = crate::pathfinding::adjust_path_destination_for_obstacles(
            world,
            &m,
            self.desired_facing,
            self.path.as_ref(),
            &mut state,
        )?;
        self.target = state.target;
        self.avoid = state.avoid;
        Ok((
            trace,
            Detour {
                detour: state.detour,
                side: state.detour_side,
            },
        ))
    }

    pub fn place(&mut self, cells: &MeshCells) {
        self.passability.neighbours =
            cells.neighbour_mask(self.passability.impassable, self.x, self.y);
    }

    pub fn set_object_scale(&mut self, scale: i32) {
        self.passability.radius =
            Some(((i32::from(self.definition.walk_half_width) * scale) >> 8) as i16);
    }

    fn block(&mut self, site: u32, reason: impl Into<String>) {
        self.block.get_or_insert(MoveBlock {
            site,
            reason: reason.into(),
        });
    }

    pub fn has_velocity(&self) -> bool {
        self.velocity != [0; 3]
    }

    pub fn is_moving(&self, flags: u32) -> bool {
        self.mode != MoveMode::Idle || flags & NEW_LEG != 0
    }

    pub fn order(&mut self, waypoints: &[(i32, i32)], flags: u16, record: RecordFlags<'_>) {
        let facing = waypoints
            .last()
            .map(|&(x, y)| arctangent(x - self.x, y - self.y))
            .unwrap_or(self.facing);
        self.order_with(waypoints, flags, facing, -1, record);
    }

    pub fn order_with(
        &mut self,
        waypoints: &[(i32, i32)],
        flags: u16,
        facing: u16,
        action: i16,
        mut record: RecordFlags<'_>,
    ) {
        self.action = action;
        if waypoints.len() == 1 && waypoints[0] == (self.x, self.y) {
            return;
        }
        self.waypoints = waypoints.iter().take(4).copied().collect();
        self.waypoint = 0;
        record.set_needs_leg(true);
        record.set_detour(false);
        self.avoid = -1;
        let several = self.waypoints.len() > 1 && flags & 1 != 0;
        self.cycle = several;
        self.back_and_forth = several && flags & 2 != 0;
        record.set_backwards(false);
        record.set_final_facing_pending(true);
        self.word_26 = facing;
    }

    pub(crate) fn set_mode(
        &mut self,
        mode: MoveMode,
        state: ModeState,
        hooks: &mut dyn ModeHooks,
    ) -> Result<(), Unmodelled> {
        let mode = match mode {
            MoveMode::Walking => hooks.walking_becomes().unwrap_or(mode),
            _ => mode,
        };
        if mode == MoveMode::Walking && hooks.posed() {
            return Ok(());
        }
        let clock = hooks.change_mode(self, mode, state)?;
        self.mode = mode;
        self.velocity = [0; 3];
        if let Some((tick, tpf)) = clock {
            self.frame_tick = tick;
            self.walk_phase = tick;
            self.ticks_per_frame = tpf;
            self.frame_advanced = false;
        }
        if matches!(
            mode,
            MoveMode::Idle | MoveMode::Attacking | MoveMode::Celebrating | MoveMode::Taunting
        ) {
            self.waypoints.clear();
            let mut f = hooks.flags();
            f.set_needs_leg(false);
            f.set_detour(false);
            f.set_detour_side(false);
            self.path = None;
        }
        Ok(())
    }

    fn finish_mode(&mut self, hooks: &mut dyn ModeHooks) -> Result<(), Unmodelled> {
        if hooks.finish(self)? {
            return Ok(());
        }
        if self.waypoint < self.waypoints.len() {
            let state = if self.mode == MoveMode::Walking {
                ModeState::Keep
            } else {
                ModeState::Zero
            };
            if !matches!(self.mode, MoveMode::Walking | MoveMode::Turning) {
                hooks.flags().set_needs_leg(true);
            }
            self.set_mode(MoveMode::Walking, state, hooks)
        } else {
            self.set_mode(MoveMode::Idle, ModeState::Zero, hooks)
        }
    }

    pub fn animate(&mut self) {
        self.frame_advanced = false;
        if self.mode != MoveMode::Idle && self.ticks_per_frame > 0 {
            self.frame_tick = if self.frame_tick >= self.ticks_per_frame {
                1
            } else {
                self.frame_tick + 1
            };
            self.frame_advanced = self.frame_tick >= self.ticks_per_frame;
        }
    }

    pub fn reset_to_idle(&mut self, hooks: &mut dyn ModeHooks) -> Result<(), Unmodelled> {
        self.set_mode(MoveMode::Idle, ModeState::Zero, hooks)?;
        hooks.flags().set_final_facing_pending(false);
        Ok(())
    }

    pub fn start_leg_if_needed(&mut self, hooks: &mut dyn ModeHooks) -> Result<(), Unmodelled> {
        if !hooks.flags().needs_leg() {
            return Ok(());
        }
        hooks.flags().set_needs_leg(false);
        let Some(&wp) = self.waypoints.get(self.waypoint) else {
            if !hooks.flags().final_facing_pending() {
                return hooks.leg_gait(self);
            }
            hooks.flags().set_final_facing_pending(false);
            let facing = self.word_26;
            self.desired_facing = facing;
            hooks.leg_gait(self)?;
            if self.definition.turning_sequence != -1 {
                return self.set_mode(MoveMode::Turning, ModeState::Zero, hooks);
            }
            if self.leg_to((self.x, self.y), hooks)? {
                return Ok(());
            }
            return self.waypoint_reached(hooks);
        };
        hooks.leg_gait(self)?;
        if self.leg_to(wp, hooks)? {
            return Ok(());
        }
        self.waypoint_reached(hooks)
    }

    pub(crate) fn waypoint_reached(&mut self, hooks: &mut dyn ModeHooks) -> Result<(), Unmodelled> {
        let n = self.waypoints.len();
        if n <= 1 {
            self.cycle = false;
            hooks.flags().set_backwards(false);
        }
        if hooks.flags().backwards() {
            if self.waypoint == 0 {
                hooks.flags().set_backwards(false);
                self.waypoint = 1;
            } else {
                self.waypoint -= 1;
            }
            hooks.flags().set_needs_leg(true);
            return Ok(());
        }
        self.waypoint += 1;
        if self.waypoint < n {
            hooks.flags().set_needs_leg(true);
            return Ok(());
        }
        if self.cycle {
            self.waypoint = 0;
            if self.back_and_forth {
                hooks.flags().set_backwards(true);
                self.waypoint = n - 2;
            }
            hooks.flags().set_needs_leg(true);
            return Ok(());
        }
        self.waypoints.clear();
        if self.action != -1 && hooks.under_order() {
            hooks.order_done(self.action)?;
        }
        if hooks.flags().final_facing_pending() {
            hooks.flags().set_final_facing_pending(false);
            self.desired_facing = self.word_26;
            if self.definition.turning_sequence != -1 {
                self.set_mode(MoveMode::Turning, ModeState::Zero, hooks)?;
            }
        } else {
            self.finish_mode(hooks)?;
        }
        hooks.end_order();
        Ok(())
    }

    pub fn leg_to(
        &mut self,
        point: (i32, i32),
        hooks: &mut dyn ModeHooks,
    ) -> Result<bool, Unmodelled> {
        let run = hooks.running();
        if run && self.mode == MoveMode::Walking {
            self.animation_state = 0x13;
            hooks.set_state(0x13);
        }
        self.target = point;
        hooks.plan_leg(self)?;
        let t = self.target;
        let dist = distance2d(self.x - t.0, self.y - t.1);
        self.leg_budget = dist as i16;
        let (dx, dy) = (t.0 - self.x, t.1 - self.y);
        if dx | dy != 0 {
            self.desired_facing = arctangent(dx, dy);
        }
        if (dist as i16) > MIN_LEG as i16 || hooks.flags().detour() {
            let state = if run {
                ModeState::Value(0x13)
            } else if self.mode == MoveMode::Walking {
                ModeState::Keep
            } else {
                ModeState::Zero
            };
            self.set_mode(MoveMode::Walking, state, hooks)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn handler(
        &mut self,
        terrain: Option<&MeshCells>,
        hooks: &mut dyn ModeHooks,
    ) -> Result<(), Unmodelled> {
        if hooks.engaged(self)? {
            return Ok(());
        }
        let walking = self.mode == MoveMode::Walking;
        let per_tick = self.animation_state == 0x14 || self.definition.per_tick;
        if walking && self.leg_budget <= 0 {
            hooks.flags().set_needs_leg(true);
        }
        let (dx, dy) = (self.target.0 - self.x, self.target.1 - self.y);
        if self.aim_countdown > 0 {
            self.aim_countdown -= 1;
            if !self.frame_advanced && !per_tick {
                return Ok(());
            }
        } else {
            if !self.frame_advanced && !per_tick {
                return Ok(());
            }
            if walking && dx | dy != 0 {
                self.desired_facing = arctangent(dx, dy);
            }
            self.aim_countdown = 15;
        }
        let diff = signed_angular_difference(self.facing, self.desired_facing);
        let turn = i32::from(self.definition.turn_rate);
        let budget = if per_tick {
            turn
        } else {
            i32::from(self.frame_tick) * turn
        };
        if budget >= diff.abs() {
            let facing = self.desired_facing;
            match self.mode {
                MoveMode::Turning => self.finish_mode(hooks)?,
                MoveMode::Aiming => hooks.aimed(self)?,
                _ => {}
            }
            self.facing = facing;
        } else if self.definition.field_198_bit8
            && distance2d(self.x - self.target.0, self.y - self.target.1) > 0x2ff
        {
            let toward = if diff.abs() > 0x8000 {
                diff < 0
            } else {
                diff >= 0
            };
            self.facing = if toward {
                self.facing.wrapping_add(budget as u16)
            } else {
                self.facing.wrapping_sub(budget as u16)
            };
            if self.mode == MoveMode::Holding {
                self.set_mode(MoveMode::Turning, ModeState::Zero, hooks)?;
            }
        } else {
            self.facing = if diff > 0 {
                self.facing.wrapping_add(budget as u16)
            } else {
                self.facing.wrapping_sub(budget as u16)
            };
            if diff.abs() > 0x4000 && self.definition.turning_sequence != -1 {
                self.set_mode(MoveMode::Turning, ModeState::Zero, hooks)?;
            }
        }
        if self.mode != MoveMode::Walking {
            return Ok(());
        }
        if !hooks.flags().detour()
            && diff != 0
            && self.leg_budget <= 0
            && (!hooks.running() || self.animation_state == 0x13)
        {
            self.velocity = [0; 3];
            return Ok(());
        }
        let t = terrain.map(|t| t.terrain_type(self.x, self.y)).unwrap_or(4);
        self.step_terrain = t;
        let mut rate =
            (i32::from(self.definition.terrain_speed[t]) + 1).wrapping_mul(i32::from(self.speed));
        if self.animation_state == 0x13 && self.definition.run_sequence != -1 {
            let d = &self.definition;
            rate = rate.wrapping_add(if !d.field_198_bit4 {
                rate
            } else if d.run_bonus > 0 {
                (f64::from(d.run_bonus) / 100.0 * f64::from(rate)) as i32
            } else {
                rate / 5
            });
        }
        let d = if per_tick {
            rate >> 8
        } else {
            i32::from(self.frame_tick).wrapping_mul(rate) >> 8
        };
        let in_range = |v: i32| (-0x7fff..=0x7ffe).contains(&v);
        let tab = trig_tables();
        let (vx, vy) = if in_range(dx)
            && in_range(dy)
            && dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) < d.wrapping_mul(d)
        {
            self.leg_budget = 0;
            (dx as i16, dy as i16)
        } else {
            let i = angle_index(self.facing);
            self.leg_budget = self.leg_budget.wrapping_sub(d as i16);
            (
                (i32::from(tab.cosine[i]).wrapping_mul(d) >> 14) as i16,
                (i32::from(tab.sine[i]).wrapping_mul(d) >> 14) as i16,
            )
        };
        self.velocity = [vx, vy, 0];
        let r = hooks.step(self, terrain)?;
        if !per_tick {
            let i = angle_index(self.facing);
            let v = rate >> 8;
            self.velocity[0] = ((i32::from(tab.cosine[i]) * v) >> 14) as i16;
            self.velocity[1] = ((i32::from(tab.sine[i]) * v) >> 14) as i16;
        }
        hooks.react(self, r)
    }

    pub fn translate(&mut self, vx: i16, vy: i16, terrain: Option<&MeshCells>) -> (u32, bool) {
        let Some(cells) = terrain else {
            if vx != 0 || vy != 0 {
                self.x += i32::from(vx);
                self.y += i32::from(vy);
            }
            return (0, false);
        };
        match move_monster(cells, &mut self.passability, (self.x, self.y), (vx, vy)) {
            Ok(step) => {
                self.x = step.x;
                self.y = step.y;
                (step.flags, step.new_leg)
            }
            Err(e) => {
                self.block(e.site, e.reason);
                self.x += i32::from(vx);
                self.y += i32::from(vy);
                (0, false)
            }
        }
    }
}

pub(crate) fn set_movement(mover: &mut EngineMover, definition: MovementDefinition) {
    mover.definition = definition;
    mover.passability.flies = mover.definition.flags & mandrake_tag::unit::MONSTER_FLAG_FLYING != 0;
    mover.passability.slide_without_nudge = mover.definition.slide_without_nudge;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct PhaseHooks {
        flags: u32,
        flags_8: u32,
    }

    impl ModeHooks for PhaseHooks {
        fn flags(&mut self) -> RecordFlags<'_> {
            RecordFlags {
                flags: &mut self.flags,
                flags_8: &mut self.flags_8,
            }
        }

        fn plan_leg(&mut self, _mover: &mut EngineMover) -> Result<(), Unmodelled> {
            Ok(())
        }

        fn change_mode(
            &mut self,
            mover: &mut EngineMover,
            mode: MoveMode,
            _state: ModeState,
        ) -> Result<Option<(i8, i8)>, Unmodelled> {
            let d = &mover.definition;
            let changed = match (mover.mode, mode) {
                (_, MoveMode::Idle) => false,
                (a, b) if a == b => false,
                (MoveMode::Walking, MoveMode::Turning) | (MoveMode::Turning, MoveMode::Walking) => {
                    d.walking_sequence != d.turning_sequence
                }
                _ => true,
            };
            let tpf = match mode {
                MoveMode::Turning => d.turn_ticks_per_frame,
                _ => d.walk_ticks_per_frame,
            };
            Ok(changed.then_some((mover.walk_phase, tpf)))
        }

        fn order_done(&mut self, _action: i16) -> Result<(), Unmodelled> {
            Ok(())
        }
    }

    fn tick(m: &mut EngineMover, hooks: &mut PhaseHooks) {
        m.animate();
        m.start_leg_if_needed(hooks).unwrap();
        if matches!(m.mode, MoveMode::Walking | MoveMode::Turning) {
            m.handler(None, hooks).unwrap();
        }
    }

    fn def(per_tick: bool, speed: i16, tpf: i8) -> MovementDefinition {
        MovementDefinition {
            tag: FourCC::new(*b"test"),
            per_tick,
            speed,
            turn_rate: 1334,
            terrain_speed: [255; 16],
            impassable: 0,
            flags: 0,
            slide_without_nudge: false,
            walk_half_width: 0,
            walking_sequence: 1,
            turning_sequence: 2,
            walk_ticks_per_frame: tpf,
            turn_ticks_per_frame: 2,
            field_198_clear: true,
            run_sequence: -1,
            run_bonus: 0,
            field_198_bit4: false,
            field_198_bit8: false,
            pathfinding_radius: 0,
            field_82: 0,
            field_1b5: 0,
            class: 0,
        }
    }

    #[test]
    fn trig_tables_have_the_axis_constants_and_symmetry() {
        let t = trig_tables();
        assert_eq!((t.cosine[0], t.sine[0]), (0x4000, 0));
        assert_eq!((t.cosine[256], t.sine[256]), (0, 0x4000));
        assert_eq!((t.cosine[512], t.sine[512]), (-0x4000, 0));
        assert_eq!((t.cosine[768], t.sine[768]), (0, -0x4000));
        assert_eq!(
            (t.tangent[0], t.tangent[256], t.tangent[512]),
            (0, i32::MIN, 0)
        );
        assert_eq!((t.cosine[128], t.sine[128]), (11585, 11585));
        assert_eq!(t.cosine[384], -11584);
    }

    #[test]
    fn arctangent_matches_the_axes_and_rounds_to_table_angles() {
        assert_eq!(arctangent(0, 5), 0x4000);
        assert_eq!(arctangent(0, -5), 0xc000);
        assert_eq!(arctangent(0, 0), 0x4000);
        assert_eq!(arctangent(7, 0), 0);
        assert_eq!(arctangent(-7, 0), 0x8000);
        assert_eq!(arctangent(100, 100), 0x2000);
        assert_eq!(arctangent(-100, 100), 0x6000);
        assert_eq!(arctangent(-100, -100), 0xa000);
        assert_eq!(arctangent(100, -100), 0xe000);
        let t = trig_tables();
        for i in 0..1024 {
            let a = arctangent(i32::from(t.cosine[i]), i32::from(t.sine[i]));
            assert_eq!(a, (i << 6) as u16, "index {i}");
        }
    }

    #[test]
    fn distance2d_rounds_to_nearest_and_saturates() {
        assert_eq!(distance2d(3, 4), 5);
        assert_eq!(distance2d(1, 1), 1);
        assert_eq!(distance2d(2, 3), 4);
        assert_eq!(distance2d(0x7fff, 0), 0x7fff);
        assert_eq!(distance2d(-0x7fff, 0), 0x7fff);
        assert_eq!(distance2d(0x7ffe, 0x7ffe), 0x7fff);
    }

    #[test]
    fn walker_steps_speed_times_ticks_once_per_frame() {
        let mut m = EngineMover::new(def(false, 10, 3), 10_000, 10_000, 0);
        let mut h = PhaseHooks::default();
        m.order(&[(11_000, 10_000)], 0, h.flags());
        let mut xs = Vec::new();
        for _ in 0..12 {
            tick(&mut m, &mut h);
            xs.push(m.x);
        }
        assert_eq!(
            xs,
            [
                10_000, 10_000, 10_000, 10_030, 10_030, 10_030, 10_060, 10_060, 10_060, 10_090,
                10_090, 10_090
            ]
        );
        for _ in 0..200 {
            tick(&mut m, &mut h);
        }
        assert_eq!((m.x, m.y), (11_000, 10_000), "lands exactly on the target");
        assert_eq!(m.mode, MoveMode::Idle);
    }

    #[test]
    fn soaring_state_turns_and_steps_every_tick_and_sets_the_velocity() {
        let mut m = EngineMover::new(def(false, 28, 4), 0, 0, 0);
        let mut h = PhaseHooks::default();
        m.animation_state = 0x14;
        m.order(&[(0, 5000)], 0, h.flags());
        let mut facings = Vec::new();
        for _ in 0..4 {
            tick(&mut m, &mut h);
            facings.push(m.facing);
        }
        assert_eq!(facings, [1334, 2668, 4002, 5336]);
        assert!(m.has_velocity());
        let tab = trig_tables();
        let i = angle_index(m.facing);
        let d = (256 * 28) >> 8;
        assert_eq!(
            m.velocity,
            [
                ((i32::from(tab.cosine[i]) * d) >> 14) as i16,
                ((i32::from(tab.sine[i]) * d) >> 14) as i16,
                0
            ]
        );
        let mut w = EngineMover::new(def(false, 28, 4), 0, 0, 0);
        let mut hw = PhaseHooks::default();
        w.order(&[(0, 5000)], 0, hw.flags());
        let mut facings = Vec::new();
        for _ in 0..5 {
            tick(&mut w, &mut hw);
            facings.push(w.facing);
        }
        assert_eq!(facings, [0, 0, 0, 0, 4 * 1334]);
    }

    #[test]
    fn a_frame_step_leaves_the_velocity_of_one_tick_and_a_mode_change_zeroes_it() {
        let mut m = EngineMover::new(def(false, 10, 3), 10_000, 10_000, 0);
        let mut h = PhaseHooks::default();
        m.order(&[(11_000, 10_000)], 0, h.flags());
        for _ in 0..4 {
            tick(&mut m, &mut h);
        }
        assert_eq!(m.x, 10_030);
        assert_eq!(m.velocity, [10, 0, 0]);
        tick(&mut m, &mut h);
        assert_eq!(
            m.velocity,
            [10, 0, 0],
            "kept on the ticks without a frame advance"
        );
        m.reset_to_idle(&mut h).unwrap();
        assert!(!m.has_velocity());
    }

    #[test]
    fn per_tick_walker_moves_every_tick() {
        let mut m = EngineMover::new(def(true, 15, 3), 0, 0, 0x4000);
        let mut h = PhaseHooks::default();
        m.order(&[(0, 600)], 0, h.flags());
        tick(&mut m, &mut h);
        let y0 = m.y;
        tick(&mut m, &mut h);
        tick(&mut m, &mut h);
        assert_eq!(m.y - y0, 30, "15 per tick");
    }

    #[test]
    fn large_turn_switches_to_turning_in_place() {
        let mut m = EngineMover::new(def(false, 10, 3), 10_000, 10_000, 0);
        let mut h = PhaseHooks::default();
        m.order(&[(9_000, 10_000)], 0, h.flags());
        for _ in 0..4 {
            tick(&mut m, &mut h);
        }
        assert_eq!(m.mode, MoveMode::Turning);
        assert_eq!(m.x, 10_000, "no step while turning in place");
        for _ in 0..300 {
            tick(&mut m, &mut h);
        }
        assert_eq!((m.x, m.y), (9_000, 10_000));
    }

    #[test]
    fn single_waypoint_at_the_position_is_ignored() {
        let mut m = EngineMover::new(def(false, 10, 3), 5, 6, 0);
        let mut h = PhaseHooks::default();
        m.order(&[(5, 6)], 0, h.flags());
        assert!(!m.is_moving(h.flags));
        m.order(&[(5 + MIN_LEG, 6)], 0, h.flags());
        tick(&mut m, &mut h);
        assert_eq!(
            m.mode,
            MoveMode::Turning,
            "a leg of 84 units counts as reached"
        );
        assert_eq!(m.x, 5);
        for _ in 0..3 {
            tick(&mut m, &mut h);
        }
        assert_eq!(m.mode, MoveMode::Idle, "back to mode 1 after the turn");
        assert_eq!(m.x, 5);
    }

    #[derive(Default)]
    struct Recorder {
        modes: Vec<(MoveMode, ModeState)>,
        done: Vec<i16>,
        flags: u32,
        flags_8: u32,
    }

    impl ModeHooks for Recorder {
        fn flags(&mut self) -> RecordFlags<'_> {
            RecordFlags {
                flags: &mut self.flags,
                flags_8: &mut self.flags_8,
            }
        }

        fn change_mode(
            &mut self,
            mover: &mut EngineMover,
            mode: MoveMode,
            state: ModeState,
        ) -> Result<Option<(i8, i8)>, Unmodelled> {
            self.modes.push((mode, state));
            Ok((mover.mode != mode).then_some((1, 3)))
        }

        fn plan_leg(&mut self, _mover: &mut EngineMover) -> Result<(), Unmodelled> {
            Ok(())
        }

        fn order_done(&mut self, action: i16) -> Result<(), Unmodelled> {
            self.done.push(action);
            Ok(())
        }
    }

    fn run(m: &mut EngineMover, h: &mut Recorder, ticks: usize) {
        for _ in 0..ticks {
            m.animate();
            m.start_leg_if_needed(h).unwrap();
            if m.mode != MoveMode::Idle {
                m.handler(None, h).unwrap();
            }
        }
    }

    #[test]
    fn no_waypoints_turn_to_the_facing_where_the_monster_stands() {
        let mut m = EngineMover::new(def(false, 10, 3), 10_000, 10_000, 0);
        let mut h = Recorder::default();
        m.order_with(&[(20_000, 10_000)], 0, 0, -1, h.flags());
        run(&mut m, &mut h, 10);
        assert_eq!(m.mode, MoveMode::Walking);
        let (x, y) = (m.x, m.y);
        h.modes.clear();
        m.order_with(&[], 0x10, 0x4000, -1, h.flags());
        assert!(m.waypoints.is_empty());
        assert!(h.flags & NEW_LEG != 0);
        assert!(h.flags & FINAL_FACING != 0);
        assert_eq!(m.word_26, 0x4000);
        m.start_leg_if_needed(&mut h).unwrap();
        assert_eq!(m.mode, MoveMode::Turning);
        assert_eq!(m.desired_facing, 0x4000);
        assert!(h.flags & FINAL_FACING == 0);
        assert!(h.flags & NEW_LEG == 0);
        run(&mut m, &mut h, 60);
        assert_eq!((m.x, m.y), (x, y));
        assert_eq!(m.facing, 0x4000);
        assert_eq!(m.mode, MoveMode::Idle);
        assert_eq!(
            h.modes,
            [
                (MoveMode::Turning, ModeState::Zero),
                (MoveMode::Idle, ModeState::Zero)
            ]
        );
    }

    #[test]
    fn no_waypoints_without_a_turning_sequence_end_at_once() {
        let mut d = def(false, 10, 3);
        d.turning_sequence = -1;
        let mut m = EngineMover::new(d, 10_000, 10_000, 0);
        let mut h = Recorder::default();
        m.order_with(&[], 0x10, 0x4000, -1, h.flags());
        m.start_leg_if_needed(&mut h).unwrap();
        assert_eq!(m.desired_facing, 0x4000);
        assert!(h.flags & FINAL_FACING == 0);
        assert_eq!(m.mode, MoveMode::Idle);
        assert_eq!((m.x, m.y), (10_000, 10_000));
    }

    #[test]
    fn an_order_walks_turns_to_its_facing_and_returns_to_idle() {
        let mut m = EngineMover::new(def(false, 10, 3), 10_000, 10_000, 0);
        let mut h = Recorder::default();
        m.order_with(&[(10_300, 10_000)], 0, 0x4000, 77, h.flags());
        run(&mut m, &mut h, 200);
        assert_eq!((m.x, m.y), (10_300, 10_000));
        assert_eq!(m.facing, 0x4000);
        assert_eq!(m.mode, MoveMode::Idle);
        assert_eq!(
            h.done,
            [77],
            "set_map_action_success once, at the last waypoint"
        );
        assert_eq!(h.modes.first(), Some(&(MoveMode::Walking, ModeState::Zero)));
        let tail: Vec<_> = h.modes.iter().rev().take(2).rev().copied().collect();
        assert_eq!(
            tail,
            [
                (MoveMode::Turning, ModeState::Zero),
                (MoveMode::Idle, ModeState::Zero)
            ]
        );
        assert!(
            h.modes[1..h.modes.len() - 2]
                .iter()
                .all(|&x| x == (MoveMode::Walking, ModeState::Keep))
        );
    }

    #[test]
    fn the_drawn_phase_starts_the_walking_clock() {
        let mut m = EngineMover::new(def(false, 10, 3), 10_000, 10_000, 0);
        let mut h = Recorder::default();
        m.order(&[(11_000, 10_000)], 0, h.flags());
        let mut xs = Vec::new();
        for _ in 0..4 {
            run(&mut m, &mut h, 1);
            xs.push(m.x);
        }
        assert_eq!(m.walk_phase, 1);
        assert_eq!(xs, [10_000, 10_000, 10_030, 10_030]);
    }

    #[test]
    fn cyclic_and_back_and_forth_waypoints() {
        let wp = [(10_000, 10_000), (10_400, 10_000), (10_400, 10_400)];
        let visits = |flags: u16| {
            let mut m = EngineMover::new(def(false, 10, 1), 10_000, 9_000, 0);
            let mut h = Recorder::default();
            m.order(&wp, flags, h.flags());
            let mut seen = Vec::new();
            for _ in 0..2_000 {
                run(&mut m, &mut h, 1);
                if m.mode == MoveMode::Walking && seen.last() != Some(&m.waypoint) {
                    seen.push(m.waypoint);
                }
            }
            (seen, m.mode)
        };
        let (seen, mode) = visits(0);
        assert_eq!((seen, mode), (vec![0, 1, 2], MoveMode::Idle));
        let (seen, _) = visits(1);
        assert_eq!(&seen[..7], [0, 1, 2, 0, 1, 2, 0], "flag 1 cycles (0x200)");
        let (seen, _) = visits(3);
        assert_eq!(
            &seen[..7],
            [0, 1, 2, 1, 0, 1, 2],
            "flag 3 goes back and forth (0x400)"
        );
    }
}
