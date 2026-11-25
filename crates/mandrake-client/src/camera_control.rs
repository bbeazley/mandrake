use core::f32::consts::{FRAC_PI_2, PI, TAU};

use mandrake_math::engine::{arctangent, distance2d};
use mandrake_sim::camera_actions::{CameraAction, CameraMove, CameraTarget, SUBJECT_CAMERA_RADIUS};
use mandrake_sim::engine_rng::angular_difference;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::camera::ObserverController;
use mandrake_view::model_surfaces::WorldFrame;

pub const FLAG_LOCK: u16 = 1;
pub const FLAG_MOVE: u16 = 2;
pub const FLAG_TRACK: u16 = 4;

pub const START_HEIGHT: f32 = 8.0;
pub const START_PITCH: f32 = f32::from_bits(0x40b9_fad7);
const ORBIT_PIVOT: f64 = 6272.0;
const CELL: f32 = 1.0 / 512.0;
const ONE_16: f32 = 1.0 / 65536.0;
const SNAPSHOT_RANGE_SQUARED: f32 = 4.0;
pub const INTERPOLATION_PREFERENCE: bool = true;
const LERP_MS: f32 = 8533.333;

const VIEW_ZOOM_PITCH: f32 = f32::from_bits(0x3e20_cf6e);
const VIEW_PRINCIPAL_SLOPE: f32 = f32::from_bits(0xbe12_4925);
const VIEW_HALF_ANGLE_DEGREES: f32 = 25.0;

fn cvtt(v: f32) -> i32 {
    if v.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&v) {
        i32::MIN
    } else {
        v as i32
    }
}

fn cvtt64(v: f32) -> i64 {
    if v.is_nan() || !(-9.223_372e18..9.223_372e18).contains(&v) {
        i64::MIN
    } else {
        v as i64
    }
}

fn real_signed_angular_difference(a: f32, b: f32) -> f32 {
    let tau = f64::from(TAU);
    let mut d = (f64::from(b - a) % tau) as f32;
    if d >= PI {
        d -= TAU;
    }
    if -PI >= d {
        d += TAU;
    }
    d
}

fn real_angle_normalize(a: f32) -> f32 {
    let r = (f64::from(a) % f64::from(TAU)) as f32;
    if 0.0 > a { r + TAU } else { r }
}

pub const fn user_camera_locked_by(flags: u16) -> bool {
    flags != 0
}

pub fn reset_height_and_pitch(controller: &mut ObserverController) {
    controller.origin[1] = START_HEIGHT;
    controller.base_elevation = TAU - START_PITCH;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObserverState {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub facing: f32,
    pub pitch: f32,
    pub zoom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObserverLocation {
    pub point: (i32, i32),
    pub facing: u16,
}

impl ObserverState {
    pub fn of(controller: &ObserverController, frame: WorldFrame) -> Self {
        let pitch = -controller.base_elevation;
        Self {
            x: frame.half_width - controller.origin[0],
            y: controller.origin[2] + frame.half_height,
            z: controller.origin[1],
            facing: (controller.yaw - FRAC_PI_2).rem_euclid(TAU),
            pitch: if pitch < 0.0 { pitch + TAU } else { pitch },
            zoom: controller.zoom,
        }
    }

    pub fn write(&self, controller: &mut ObserverController, frame: WorldFrame) {
        controller.origin = [
            frame.half_width - self.x,
            self.z,
            self.y - frame.half_height,
        ];
        controller.yaw = (self.facing + FRAC_PI_2).rem_euclid(TAU);
        controller.base_elevation = -(if self.pitch > PI {
            self.pitch - TAU
        } else {
            self.pitch
        });
        controller.zoom = self.zoom;
    }

    pub fn location(&self) -> ObserverLocation {
        ObserverLocation {
            point: (cvtt(self.x * 512.0), cvtt(512.0 * self.y)),
            facing: cvtt(self.facing / TAU * 65536.0) as u16,
        }
    }

    pub fn zoom_factor(&self) -> i32 {
        cvtt(65536.0 * self.zoom)
    }

    pub fn view_pitch_word(&self) -> u16 {
        let pitch = if self.zoom < 0.0 {
            let part = if -1.0 <= self.zoom {
                VIEW_ZOOM_PITCH * self.zoom
            } else {
                -VIEW_ZOOM_PITCH
            };
            self.pitch + part
        } else {
            self.pitch
        };
        cvtt(pitch / TAU * 65536.0) as u16
    }
}

pub fn view_slope(width: u32, height: u32) -> f32 {
    let tangent = f64::from(VIEW_HALF_ANGLE_DEGREES * (PI / 180.0)).tan();
    let aspect = (f64::from(width as i32) * 0.75 / f64::from(height as i32)) as f32;
    let horizontal = (f64::from(aspect) * tangent) as f32;
    horizontal * VIEW_PRINCIPAL_SLOPE
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Snapshot {
    active: bool,
    start: u32,
    end: u32,
    x: f32,
    y: f32,
    z: f32,
    pitch: f32,
    facing: f32,
    zoom: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct LerpState {
    key: u32,
    base: f32,
    last: f32,
    start_ms: u32,
    end_ms: u32,
    speed: i16,
}

impl LerpState {
    fn lerp(&mut self, key: u32, span: f32, now_ms: u32, speed: i16) -> f32 {
        let duration = if speed == 0 {
            0.0
        } else {
            span * LERP_MS / f32::from(speed)
        };
        if self.key != key {
            self.start_ms = now_ms;
            self.last = 0.0;
            self.base = 0.0;
            self.end_ms = cvtt64(now_ms as f32 + duration) as u32;
            self.speed = speed;
            self.key = key;
            return 0.0;
        }
        let length = self.end_ms.wrapping_sub(self.start_ms);
        let mut f = if length != 0 {
            (1.0 - self.base) * now_ms.wrapping_sub(self.start_ms) as f32 / length as f32
        } else {
            0.0
        };
        f += self.base;
        f = if 0.0 > f { 0.0 } else { f.min(1.0) };
        if speed == self.speed {
            self.last = f;
        } else {
            self.start_ms = now_ms;
            self.speed = speed;
            self.base = f;
            self.end_ms = cvtt64((1.0 - f) * duration + now_ms as f32) as u32;
        }
        self.key = key;
        f
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MotionClock(LerpState);

impl MotionClock {
    pub fn fraction(&mut self, tick: u32, now_ms: u32, speed: i16) -> f32 {
        self.0.lerp(tick, 1.0, now_ms, speed)
    }

    pub fn fraction_from_due(&mut self, tick: u32, now_ms: u32, late_ms: u32, speed: i16) -> f32 {
        if self.0.key != tick {
            self.0.lerp(tick, 1.0, now_ms - late_ms.min(now_ms), speed);
        }
        self.0.lerp(tick, 1.0, now_ms, speed)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveBlock {
    pub from: (i32, i32),
    pub from_facing: u16,
    pub to: (i32, i32),
    pub to_facing: u16,
    pub from_zoom: i32,
    pub to_zoom: i32,
    pub duration: i16,
    pub step: i16,
}

#[derive(Debug, Clone, Copy)]
pub struct MoveRules {
    pub line: i32,
    pub far: i32,
    pub maxz: bool,
    pub at_least_one: bool,
    pub cell_shift: u32,
    pub arctangent: fn(i32, i32) -> u16,
}

impl MoveRules {
    pub const MYTH2: Self = Self {
        line: 0x20,
        far: 0x780,
        maxz: true,
        at_least_one: true,
        cell_shift: 9,
        arctangent,
    };
}

impl MoveBlock {
    pub fn start(
        from: ObserverLocation,
        from_zoom: i32,
        point: (i32, i32),
        facing: u16,
        mv: &CameraMove,
    ) -> Self {
        Self::start_as(from, from_zoom, point, facing, mv, &MoveRules::MYTH2)
    }

    pub fn start_as(
        from: ObserverLocation,
        from_zoom: i32,
        point: (i32, i32),
        facing: u16,
        mv: &CameraMove,
        rules: &MoveRules,
    ) -> Self {
        let mut block = Self {
            from: from.point,
            from_facing: from.facing,
            to: point,
            to_facing: facing,
            from_zoom,
            to_zoom: if mv.maxz && rules.maxz {
                0xffff_0001_u32 as i32
            } else {
                0
            },
            duration: 0,
            step: 0,
        };
        let mut d = distance2d(from.point.0 - point.0, from.point.1 - point.1);
        let a = angular_difference(from.facing, facing);
        let mut line = mv.line.unwrap_or(rules.line);
        let far = distance2d(
            (from.point.0 >> 3) - (point.0 >> 3),
            (from.point.1 >> 3) - (point.1 >> 3),
        );
        if far > rules.far {
            line >>= 3;
            d = far;
        }
        let turn = mv.angu.map_or(0x200, |angu| ((angu as u32) << 6) as u16);
        let ticks_distance = if line != 0 { d.wrapping_div(line) } else { 0 };
        let ticks_angle = (a as u16).checked_div(turn).unwrap_or(0);
        let (td, ta) = (ticks_distance as i16, ticks_angle as i16);
        let n = if ta >= td { ta } else { td };
        block.duration = if n <= 0 && rules.at_least_one { 1 } else { n };
        if let Some(time) = mv.time {
            let mut t = time.wrapping_mul(30);
            if t < 0 {
                t = t.wrapping_add(0xffff);
            }
            if t >> 16 < i32::from(block.duration) {
                block.duration = 1;
            }
        }
        block
    }

    pub fn advance(&mut self) -> MoveStep {
        let b = *self;
        let t = if b.duration == 0 {
            1.0
        } else {
            f32::from(b.step) / f32::from(b.duration)
        };
        let point = (
            cvtt(b.to.0.wrapping_sub(b.from.0) as f32 * t + b.from.0 as f32),
            cvtt(b.to.1.wrapping_sub(b.from.1) as f32 * t + b.from.1 as f32),
        );
        let zoom = cvtt(b.to_zoom.wrapping_sub(b.from_zoom) as f32 * t + b.from_zoom as f32);
        let turn = cvtt(t * f32::from(angular_difference(b.from_facing, b.to_facing) as u16));
        let (from, to) = (i32::from(b.from_facing), i32::from(b.to_facing));
        let ahead = (from + turn) as u16;
        let behind = (from - turn) as u16;
        let facing = match from.cmp(&to) {
            core::cmp::Ordering::Greater => match from - to {
                d if d <= 0x7fff => Some(behind),
                0x8000 => None,
                _ => Some(ahead),
            },
            core::cmp::Ordering::Less => match to - from {
                d if d > 0x8000 => Some(behind),
                0x8000 => None,
                _ => Some(ahead),
            },
            core::cmp::Ordering::Equal => None,
        };
        self.step = b.step.wrapping_add(1);
        MoveStep {
            point,
            zoom,
            facing,
        }
    }

    pub fn done(&self) -> bool {
        self.step > self.duration
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveStep {
    pub point: (i32, i32),
    pub zoom: i32,
    pub facing: Option<u16>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shake {
    pub active: bool,
    pub mode: u16,
    pub intensity: i32,
    pub duration: i32,
    pub start: i32,
}

#[derive(Debug, Clone)]
pub struct CameraControl {
    pub flags: u16,
    pub keep_control: bool,
    pub movement: MoveBlock,
    pub shake: Shake,
    pub interpolate: bool,
    ring: [Snapshot; 4],
    ring_index: i32,
    lerp: LerpState,
    motion: MotionClock,
    observer: Option<(ObserverState, [f32; 6])>,
    random: LocalRandom,
    pub limits: Option<mandrake_view::camera::ObserverLimits>,
    pub explored_only: bool,
    view_pitch: Option<u16>,
}

fn controller_key(c: &ObserverController) -> [f32; 6] {
    [
        c.origin[0],
        c.origin[1],
        c.origin[2],
        c.yaw,
        c.base_elevation,
        c.zoom,
    ]
}

impl CameraControl {
    pub fn new_for_map(game_options: u32, interpolate: bool) -> Self {
        Self {
            flags: if game_options & mandrake_common::game_options::CAMERA_TRACKING != 0 {
                FLAG_TRACK
            } else {
                0
            },
            keep_control: false,
            movement: MoveBlock::default(),
            shake: Shake::default(),
            interpolate,
            ring: [Snapshot::default(); 4],
            ring_index: 0,
            lerp: LerpState::default(),
            motion: MotionClock::default(),
            observer: None,
            random: LocalRandom::new(0),
            limits: None,
            explored_only: false,
            view_pitch: None,
        }
    }

    pub fn set_mesh_limits(&mut self, data: &[u8], cells: (u16, u16)) {
        self.set_mesh_edges(data, cells, 1.0);
        self.explored_only = data.get(0x4c..0x50).is_some_and(|b| {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]]) & crate::minimap::MESH_FLAG_EXPLORED_MAP
                != 0
        });
    }

    pub fn set_mesh_edges(&mut self, data: &[u8], cells: (u16, u16), cell_size: f32) {
        self.limits = mesh_edges(data, cells, cell_size);
    }

    #[cfg(test)]
    pub fn motion_fraction(&mut self, tick: u32, now_ms: u32, speed: i16) -> f32 {
        self.motion.fraction(tick, now_ms, speed)
    }

    pub fn motion_fraction_from_due(
        &mut self,
        tick: u32,
        now_ms: u32,
        late_ms: u32,
        speed: i16,
    ) -> f32 {
        self.motion.fraction_from_due(tick, now_ms, late_ms, speed)
    }

    pub fn seed(
        &mut self,
        state: ObserverState,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) {
        self.set_observer(state, controller, frame);
    }

    pub fn after_tick(
        &mut self,
        sim: &mut mandrake_sim::Simulation,
        tick: i32,
        controller: &mut ObserverController,
        frame: WorldFrame,
        mesh_cells: (i32, i32),
    ) -> Vec<CameraAction> {
        let runs = take_camera_actions(sim);
        for run in &runs {
            self.apply(run, controller, frame, mesh_cells);
        }
        self.handle_map_action_camera_control(tick, controller, frame);
        runs
    }

    pub fn user_camera_locked(&self) -> bool {
        user_camera_locked_by(self.flags)
    }

    pub fn observer(&self, controller: &ObserverController, frame: WorldFrame) -> ObserverState {
        match self.observer {
            Some((state, key)) if key == controller_key(controller) => state,
            _ => ObserverState::of(controller, frame),
        }
    }

    fn set_observer(
        &mut self,
        state: ObserverState,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) {
        state.write(controller, frame);
        self.observer = Some((state, controller_key(controller)));
    }

    pub fn apply(
        &mut self,
        action: &CameraAction,
        controller: &ObserverController,
        frame: WorldFrame,
        mesh_cells: (i32, i32),
    ) {
        if let Some(flags) = action.control_flags {
            self.flags = flags as u16;
        }
        if let Some(keep) = action.keep_control {
            self.keep_control = keep;
        }
        if let Some(mv) = &action.movement {
            let observer = self.observer(controller, frame);
            let from = observer.location();
            let (point, facing) = match mv.target {
                CameraTarget::Point { point, facing } => (point, facing),
                CameraTarget::Monster { position, .. } => {
                    subject_target(from, position, mesh_cells)
                }
            };
            self.movement = MoveBlock::start(from, observer.zoom_factor(), point, facing, mv);
        }
        if let Some(shake) = &action.shake {
            self.set_observer_shaking(-1, shake.intensity, shake.duration, action.time);
        }
    }

    pub fn set_observer_shaking(&mut self, mode: i16, intensity: i32, duration: i32, now: i32) {
        let mode = if mode == -1 { 0 } else { mode as u16 };
        let s = &mut self.shake;
        s.active = true;
        s.start = now;
        s.mode = mode;
        let defaults = match mode {
            0 => Some((0x100, 0xf0)),
            1 => Some((0x80, 0x78)),
            _ => None,
        };
        if intensity != -1 {
            s.intensity = intensity;
        } else if let Some((i, _)) = defaults {
            s.intensity = i;
        }
        if duration != -1 {
            s.duration = duration;
        } else if let Some((_, d)) = defaults {
            s.duration = d;
        }
    }

    pub fn handle_map_action_camera_control(
        &mut self,
        tick: i32,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) {
        if self.flags & FLAG_MOVE == 0 {
            return;
        }
        let b = self.movement;
        if b.step > b.duration {
            return;
        }
        let observer = self.observer(controller, frame);
        let mut loc = observer.location();
        let step = self.movement.advance();
        loc.point = step.point;
        if let Some(f) = step.facing {
            loc.facing = f;
        }
        let zoom = step.zoom;
        self.set_actual_observer_location_and_zoom(
            loc,
            zoom as f32 * ONE_16,
            tick,
            controller,
            frame,
        );
        if self.movement.step > b.duration && !self.keep_control {
            self.flags = 0;
        }
    }

    pub fn set_actual_observer_location_and_zoom(
        &mut self,
        loc: ObserverLocation,
        zoom: f32,
        tick: i32,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) {
        let x = loc.point.0 as f32 * CELL;
        let y = loc.point.1 as f32 * CELL;
        let facing = f32::from(loc.facing) * TAU * ONE_16;
        let (index, prev) = self.next_ring_slot();
        let observer = self.observer(controller, frame);
        let (px, py, pz) = if prev.active {
            (prev.x, prev.y, prev.z)
        } else {
            (observer.x, observer.y, observer.z)
        };
        let (dx, dy, dz) = (x - px, y - py, START_HEIGHT - pz);
        let d2 = dz * dz + (dy * dy + dx * dx);
        let state = ObserverState {
            x,
            y,
            z: START_HEIGHT,
            facing,
            pitch: START_PITCH,
            zoom,
        };
        let queue = self.interpolate && SNAPSHOT_RANGE_SQUARED > d2;
        self.queue_or_set(index, prev, state, queue, tick, controller, frame);
    }

    pub fn set_observer_location(
        &mut self,
        loc: ObserverLocation,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) {
        let limits = self
            .limits
            .unwrap_or(mandrake_view::camera::ObserverLimits {
                half_width: frame.half_width,
                half_height: frame.half_height,
                width: (2.0 * frame.half_width) as i16,
                height: (2.0 * frame.half_height) as i16,
                margins: [0; 4],
            });
        let mut found = None;
        'turns: for turn in (0..=0x8000i32).step_by(0x800) {
            for t in [-turn, turn] {
                let word = (t + i32::from(loc.facing)) as u16;
                let back = word.wrapping_sub(0x8000);
                let facing = f32::from(word) * TAU * ONE_16;
                let angle = f64::from(f32::from(back) * TAU * ONE_16);
                let (sin, cos) = angle.sin_cos();
                let x = (f64::from(loc.point.0) + ORBIT_PIVOT * cos) as i32 as f32 * CELL;
                let y = (f64::from(loc.point.1) + ORBIT_PIVOT * sin) as i32 as f32 * CELL;
                let [low_y, low_x, high_y, high_x] = limits.edges(facing);
                if low_x < x && x < high_x && low_y < y && y < high_y {
                    found = Some((x, y, facing));
                    break 'turns;
                }
            }
        }
        let (x, y, facing) = found.unwrap_or_else(|| {
            let [low_y, low_x, high_y, high_x] = limits.margins.map(|m| i32::from(m) << 9);
            let clamp = |v: i32, low: i32, high: i32| if v < low { low } else { v.min(high) };
            let cx = clamp(loc.point.0, low_x, (i32::from(limits.width) << 9) - high_x);
            let cy = clamp(loc.point.1, low_y, (i32::from(limits.height) << 9) - high_y);
            let word = arctangent(loc.point.0 - cx, loc.point.1 - cy);
            (
                cx as f32 * CELL,
                cy as f32 * CELL,
                f32::from(word) * TAU * ONE_16,
            )
        });
        let zoom = self.observer(controller, frame).zoom;
        let (index, prev) = self.next_ring_slot();
        let state = ObserverState {
            x,
            y,
            z: START_HEIGHT,
            facing,
            pitch: START_PITCH,
            zoom,
        };
        self.queue_or_set(index, prev, state, false, 0, controller, frame);
    }

    fn next_ring_slot(&mut self) -> (i32, Snapshot) {
        let index = self.ring_index;
        self.ring_index = index.wrapping_add(1);
        (index, self.ring[(index.wrapping_sub(1) & 3) as usize])
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_or_set(
        &mut self,
        index: i32,
        prev: Snapshot,
        state: ObserverState,
        queue: bool,
        tick: i32,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) {
        if queue {
            let now = tick as u32;
            self.ring[(index & 3) as usize] = Snapshot {
                active: true,
                start: now,
                end: now.wrapping_add(1),
                x: state.x,
                y: state.y,
                z: state.z,
                pitch: state.pitch,
                facing: state.facing,
                zoom: state.zoom,
            };
            if prev.active
                && now > prev.start
                && state.x == prev.x
                && state.y == prev.y
                && prev.z == state.z
                && state.facing == prev.facing
                && prev.pitch == state.pitch
                && state.zoom == prev.zoom
            {
                self.ring[(index.wrapping_sub(1) & 3) as usize].end = now.wrapping_add(1);
                self.ring_index = index;
            }
            return;
        }
        self.ring = [Snapshot::default(); 4];
        self.set_observer(state, controller, frame);
        controller.stop();
    }

    pub fn view_built(&mut self, controller: &ObserverController, frame: WorldFrame) {
        self.view_pitch = Some(self.observer(controller, frame).view_pitch_word());
    }

    pub fn set_observer_focus(
        &mut self,
        point: (i32, i32, i32),
        angle: u16,
        slope: f32,
        tick: i32,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) {
        let current = self.observer(controller, frame);
        let word = self.view_pitch.unwrap_or_else(|| current.view_pitch_word());
        let elevation = f64::from(slope).atan() as f32;
        let view_angle = f32::from(word) * TAU * ONE_16 + elevation;
        let tangent = f64::from(real_angle_normalize(TAU - view_angle)).tan();
        let a = f32::from(angle) * TAU * ONE_16;
        let reach = (f64::from(current.z - point.2 as f32 * CELL) / tangent) as f32;
        let (sin, cos) = f64::from(a).sin_cos();
        let reach = f64::from(reach);
        let x = (f64::from(point.0 as f32 * CELL) + cos * reach) as f32;
        let y = (reach * sin + f64::from(point.1 as f32 * CELL)) as f32;
        let clamp = |v: f32, cells: f32| {
            if 0.0 > v {
                0.0
            } else if cells - 1.0 < v {
                cells - 1.0
            } else {
                v
            }
        };
        let x = clamp(x, 2.0 * frame.half_width);
        let y = clamp(y, 2.0 * frame.half_height);
        let facing = real_angle_normalize(a + PI);
        let (index, prev) = self.next_ring_slot();
        let (px, py, z, pitch, zoom) = if prev.active {
            (prev.x, prev.y, prev.z, prev.pitch, prev.zoom)
        } else {
            (current.x, current.y, current.z, current.pitch, current.zoom)
        };
        let (dx, dy) = (x - px, y - py);
        let d2 = dx * dx + dy * dy;
        let queue = self.flags & FLAG_TRACK != 0 && self.interpolate && SNAPSHOT_RANGE_SQUARED > d2;
        let state = ObserverState {
            x,
            y,
            z,
            facing,
            pitch,
            zoom,
        };
        self.queue_or_set(index, prev, state, queue, tick, controller, frame);
    }

    pub fn interpolate_observer(
        &mut self,
        now_ms: u32,
        speed: i16,
        controller: &mut ObserverController,
        frame: WorldFrame,
    ) -> bool {
        if !self.interpolate {
            return false;
        }
        let i = self.ring_index;
        let a = self.ring[(i.wrapping_sub(3) & 3) as usize];
        let b = self.ring[(i.wrapping_sub(2) & 3) as usize];
        let c = self.ring[(i.wrapping_sub(1) & 3) as usize];
        if !(a.active && b.active && c.active) || b.start <= a.start || b.start >= c.start {
            return false;
        }
        let span = b
            .start
            .wrapping_add(b.end)
            .wrapping_sub(a.end)
            .wrapping_sub(a.start) as f32
            * 0.5;
        let f = self.lerp.lerp(a.start, span, now_ms, speed);
        let state = ObserverState {
            x: (b.x - a.x) * f + a.x,
            y: (b.y - a.y) * f + a.y,
            z: (b.z - a.z) * f + a.z,
            pitch: real_angle_normalize(
                real_signed_angular_difference(a.pitch, b.pitch) * f + a.pitch,
            ),
            facing: real_angle_normalize(
                real_signed_angular_difference(a.facing, b.facing) * f + a.facing,
            ),
            zoom: f * (b.zoom - a.zoom) + a.zoom,
        };
        self.set_observer(state, controller, frame);
        true
    }

    pub fn shake_eye(&mut self, now: i32, eye: [f32; 3]) -> [f32; 3] {
        let s = &mut self.shake;
        if !s.active {
            return eye;
        }
        let elapsed = now.wrapping_sub(s.start) as u32;
        let duration = s.duration as u32;
        if elapsed > duration {
            s.active = false;
            return eye;
        }
        let f = match s.mode {
            0 => {
                let half = duration >> 1;
                if elapsed > half {
                    (duration - elapsed) as f32 / half as i32 as f32
                } else {
                    elapsed as i32 as f32 / half as i32 as f32
                }
            }
            1 => (duration - elapsed) as f32 / duration as f32,
            _ => 0.0,
        };
        let amplitude = cvtt(f * s.intensity as f32);
        let [mut x, mut y, mut z] = eye;
        if amplitude <= 0 {
            let off = amplitude.wrapping_neg() as f32 * CELL;
            x += off;
            y += off;
        } else {
            let mut draw = || {
                let r = i32::from(self.random.next_u16());
                (r.wrapping_mul(amplitude) >> 16) - amplitude
            };
            x += draw() as f32 * CELL;
            y += draw() as f32 * CELL;
            z += draw() as f32 * CELL;
        }
        [x, y, z]
    }
}

pub fn step_user_observer(
    control: &CameraControl,
    observer: &mut ObserverController,
    axes: mandrake_view::camera::myth_observer::ObserverAxes,
    visibility: Option<&crate::minimap::Visibility>,
) {
    if control.user_camera_locked() {
        observer.integrate(Default::default());
    } else {
        step_free_observer(control, observer, axes, visibility);
    }
}

pub fn step_free_observer(
    control: &CameraControl,
    observer: &mut ObserverController,
    axes: mandrake_view::camera::myth_observer::ObserverAxes,
    visibility: Option<&crate::minimap::Visibility>,
) {
    observer.integrate(axes);
    advance_user_observer(control, observer, visibility);
}

pub fn mesh_edges(
    data: &[u8],
    cells: (u16, u16),
    cell_size: f32,
) -> Option<mandrake_view::camera::ObserverLimits> {
    let l = mandrake_view::camera::ObserverLimits::from_mesh_bytes(data, cells.0, cells.1)?;
    if cell_size == 1.0 {
        return Some(l);
    }
    let units = |n: f32| (n * cell_size).round() as i16;
    let size = |n: u16| units(f32::from(n) - 1.0) + 1;
    Some(mandrake_view::camera::ObserverLimits {
        half_width: l.half_width * cell_size,
        half_height: l.half_height * cell_size,
        width: size(cells.0),
        height: size(cells.1),
        margins: l.margins.map(|m| units(f32::from(m))),
    })
}

fn advance_user_observer(
    control: &CameraControl,
    observer: &mut ObserverController,
    visibility: Option<&crate::minimap::Visibility>,
) {
    let on_explored = |x: i32, y: i32, facing: u16| {
        visibility.is_some_and(|v| v.view_on_explored_cell(x, y, facing))
    };
    let test: Option<&dyn Fn(i32, i32, u16) -> bool> =
        (control.explored_only && visibility.is_some()).then_some(&on_explored);
    observer.advance_within(control.limits.as_ref(), test);
}

pub fn shake_camera(
    control: &mut CameraControl,
    camera: &mut mandrake_view::camera::Camera,
    frame: WorldFrame,
    now: i32,
) {
    if !control.shake.active {
        return;
    }
    let e = camera.eye_position();
    let eye = [frame.half_width - e[0], e[2] + frame.half_height, e[1]];
    let s = control.shake_eye(now, eye);
    let d = [eye[0] - s[0], s[2] - eye[2], s[1] - eye[1]];
    for (t, d) in camera.target.iter_mut().zip(d) {
        *t += d;
    }
}

pub fn trigger_obmo(mesh_data: &[u8], index: usize) -> anyhow::Result<CameraAction> {
    use mandrake_sim::engine_rng::{EngineRng, EngineWorld, GAMEPLAY_VERSION_1_8_3, SiteCounts};
    use mandrake_sim::map_actions::{MapActionContext, MapActions};
    let mesh =
        mandrake_tag::MeshMapActions::from_mesh_bytes(mesh_data).map_err(anyhow::Error::msg)?;
    let mut actions = MapActions::from_mesh(&mesh);
    let tag = actions
        .actions
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("no map action {index}"))?
        .tag;
    anyhow::ensure!(
        &tag.to_be_bytes() == b"obmo",
        "map action {index} is not 'obmo'"
    );
    let mut world = EngineWorld::default();
    world.mode = 2;
    world.formation_globals.gameplay_version = GAMEPLAY_VERSION_1_8_3;
    let mut rng = mandrake_math::BungieRng::new(0);
    let mut sites = SiteCounts::default();
    actions
        .trigger(
            index,
            &mut MapActionContext {
                world: &mut world,
                rng: &mut EngineRng {
                    rng: &mut rng,
                    sites: &mut sites,
                },
                time: 0,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    actions
        .camera_actions
        .pop()
        .ok_or_else(|| anyhow::anyhow!("map action {index} recorded no camera run"))
}

pub fn take_camera_actions(sim: &mut mandrake_sim::Simulation) -> Vec<CameraAction> {
    (sim.engine.map_actions.as_mut())
        .map(|a| std::mem::take(&mut a.camera_actions))
        .unwrap_or_default()
}

pub fn subject_target(
    observer: ObserverLocation,
    position: (i32, i32),
    mesh_cells: (i32, i32),
) -> ((i32, i32), u16) {
    subject_target_as(observer, position, mesh_cells, &MoveRules::MYTH2)
}

pub fn subject_target_as(
    observer: ObserverLocation,
    position: (i32, i32),
    mesh_cells: (i32, i32),
    rules: &MoveRules,
) -> ((i32, i32), u16) {
    let p = closest_point_on_circle2d(position, SUBJECT_CAMERA_RADIUS, observer.point);
    let clamp = |v: i32, cells: i32| {
        let limit = cells << rules.cell_shift;
        if v < 0 {
            0
        } else if v >= limit {
            limit - 1
        } else {
            v
        }
    };
    let point = (clamp(p.0, mesh_cells.0), clamp(p.1, mesh_cells.1));
    let facing = (rules.arctangent)(
        position.0.wrapping_sub(observer.point.0),
        position.1.wrapping_sub(observer.point.1),
    );
    (point, facing)
}

pub fn closest_point_on_circle2d(center: (i32, i32), radius: i32, point: (i32, i32)) -> (i32, i32) {
    let dx = point.0.wrapping_sub(center.0);
    let dy = point.1.wrapping_sub(center.1);
    let s = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32;
    let d = mandrake_sim::projectiles::isqrt(s);
    if d == 0 {
        return point;
    }
    (
        dx.wrapping_mul(radius)
            .wrapping_div(d)
            .wrapping_add(center.0),
        dy.wrapping_mul(radius)
            .wrapping_div(d)
            .wrapping_add(center.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_camera_control_flag_locks_the_user_camera() {
        assert!(!user_camera_locked_by(0));
        for f in [FLAG_LOCK, FLAG_MOVE, FLAG_TRACK] {
            assert!(user_camera_locked_by(f));
        }
    }

    #[test]
    fn the_reset_height_and_pitch_are_both_games() {
        assert_eq!(0x1000 as f32 / 512.0, START_HEIGHT);
        let tfl_pitch = f32::from(0xecccu16) / 65536.0 * TAU;
        assert!((tfl_pitch - START_PITCH).abs() < 1e-4, "{tfl_pitch}");
        let frame = WorldFrame::new(64, 64);
        let mut c = ObserverController::from_camera(&mandrake_view::camera::Camera::default());
        c.origin[1] = 3.0;
        c.base_elevation = 1.0;
        let mut expected = c.clone();
        let mut state = ObserverState::of(&c, frame);
        state.z = START_HEIGHT;
        state.pitch = START_PITCH;
        state.write(&mut expected, frame);
        reset_height_and_pitch(&mut c);
        assert_eq!(c.origin[1], expected.origin[1]);
        assert!((c.base_elevation - expected.base_elevation).abs() < 1e-6);
    }

    #[test]
    fn the_motion_fraction_follows_the_game_speed() {
        use crate::game_speed::GAME_SPEEDS;
        let mut c = CameraControl::new_for_map(0, true);
        let speed = |index| GAME_SPEEDS[index];
        let s = speed(1);
        assert_eq!(c.motion_fraction(1, 1000, s), 0.0);
        let half = c.motion_fraction(1, 1000 + 267, s);
        assert!((half - 0.5).abs() < 0.01, "{half}");
        let s = speed(0);
        let held = c.motion_fraction(1, 2000, s);
        assert_eq!(c.motion_fraction(1, 9000, s), held);
        let s = speed(10);
        for (tick, ms) in [(2, 9016), (34, 9032), (66, 9048)] {
            assert_eq!(c.motion_fraction(tick, ms, s), 0.0);
        }
    }

    #[test]
    fn objects_follow_the_frame_time_evenly() {
        const TICK_INTERVAL: f32 = 1.0 / 30.0;
        let speed = mandrake_sim::game_speed::NORMAL_SPEED;
        let tick_ms = 1000.0 / 30.0;
        for hz in [60.0f64, 59.94, 75.0, 120.0, 144.0] {
            let (mut ours, mut engine) = (
                CameraControl::new_for_map(0, true),
                CameraControl::new_for_map(0, true),
            );
            let period = 1000.0 / hz;
            let offset = 0.37 * period;
            let (mut accumulator, mut last, mut tick) = (0.0f32, 0.0f64, 0u32);
            let (mut a, mut b) = (Vec::new(), Vec::new());
            for i in 0..(hz as usize * 3) {
                let t = offset + i as f64 * period;
                accumulator += (((t - last) / 1000.0) as f32).min(0.25);
                last = t;
                while accumulator >= TICK_INTERVAL {
                    accumulator -= TICK_INTERVAL;
                    tick += 1;
                }
                let now_ms = t as u32;
                let late_ms = (accumulator * 1000.0) as u32;
                let shown = |f: f32| {
                    let f = crate::object_history::frame_fraction(f);
                    (f64::from(tick) - 1.0 + f64::from(f) / 65536.0) * tick_ms - t
                };
                a.push(shown(
                    ours.motion_fraction_from_due(tick, now_ms, late_ms, speed),
                ));
                b.push(shown(engine.motion_fraction(tick, now_ms, speed)));
            }
            let spread = |v: &[f64]| {
                let (lo, hi) = v[hz as usize..]
                    .iter()
                    .fold((f64::MAX, f64::MIN), |(l, h), &x| (l.min(x), h.max(x)));
                hi - lo
            };
            let (a, b) = (spread(&a), spread(&b));
            println!(
                "{hz} Hz: objects' time against the frame's spreads {a:.2} ms (the engine's \
                 clock {b:.2} ms) in a frame of {period:.2} ms"
            );
            assert!(a <= 2.1, "{hz} Hz: {a} ms");
            assert!(a <= b + 0.01, "{hz} Hz: {a} ms against the engine's {b} ms");
            if hz == 75.0 || hz == 144.0 {
                assert!(a + 2.0 < b, "{hz} Hz: {a} ms against the engine's {b} ms");
            }
        }
    }
    use mandrake_view::camera::Camera;

    fn frame() -> WorldFrame {
        WorldFrame::new(192, 128)
    }

    fn controller() -> ObserverController {
        ObserverController::from_camera(&Camera::default())
    }

    fn observer_at(x: i32, y: i32, facing: u16) -> ObserverState {
        ObserverState {
            x: (x as f32 + 0.5) / 512.0,
            y: (y as f32 + 0.5) / 512.0,
            z: START_HEIGHT,
            facing: (f32::from(facing) + 0.5) * TAU / 65536.0,
            pitch: START_PITCH,
            zoom: 0.0,
        }
    }

    #[test]
    fn subject_targets_face_the_monster() {
        let obs = ObserverLocation {
            point: (10000, 4000),
            facing: 0,
        };
        let (p, f) = subject_target(obs, (30000, 4000), (192, 128));
        assert_eq!((p, f), ((30000 - 0x1400, 4000), 0));
        let (p, f) = subject_target(obs, (10000, 30000), (192, 128));
        assert_eq!((p, f), ((10000, 30000 - 0x1400), 0x4000));
        let (p, _) = subject_target(obs, (1000, 4000), (192, 128));
        assert_eq!(p, (1000 + 0x1400, 4000));
        let (p, _) = subject_target(
            ObserverLocation {
                point: (-9000, 4000),
                facing: 0,
            },
            (1000, 4000),
            (192, 128),
        );
        assert_eq!(p, (0, 4000));
        assert_eq!(closest_point_on_circle2d((5, 5), 0x1400, (5, 5)), (5, 5));
    }

    #[test]
    fn the_engine_observer_survives_the_controller() {
        let mut observer = controller();
        let mut control = CameraControl::new_for_map(0, true);
        let s = observer_at(77230, 17353, 20136);
        control.seed(s, &mut observer, frame());
        assert_eq!(control.observer(&observer, frame()), s);
        let back = ObserverState::of(&observer, frame());
        assert!((back.x - s.x).abs() < 1e-4 && (back.facing - s.facing).abs() < 1e-5);
        assert!((back.pitch - START_PITCH).abs() < 1e-5);
        observer.origin[0] += 1.0;
        assert_ne!(control.observer(&observer, frame()), s);
        assert_eq!(CameraControl::new_for_map(0x400, true).flags, FLAG_TRACK);
    }

    #[test]
    fn observer_stops_at_the_mesh_edge() {
        let mut control = CameraControl::new_for_map(0, true);
        control.limits = Some(mandrake_view::camera::ObserverLimits {
            half_width: 48.0,
            half_height: 48.0,
            width: 96,
            height: 96,
            margins: [4; 4],
        });
        let edges = control.limits.unwrap().edges(0.0);
        assert_eq!(edges, [4.0, 4.0, 91.0, 91.0]);
        let mut o = controller();
        o.origin = [0.0, 8.0, 0.0];
        o.yaw = std::f32::consts::FRAC_PI_2;
        o.velocity[0] = 1.0;
        for _ in 0..200 {
            advance_user_observer(&control, &mut o, None);
        }
        assert!((48.0 - o.origin[0] - 91.0).abs() < 0.01, "{:?}", o.origin);
        o.origin[0] = 48.0 - 95.0;
        o.velocity = [0.0; 5];
        advance_user_observer(&control, &mut o, None);
        assert_eq!(48.0 - o.origin[0], 91.0);
    }
}
