use core::f32::consts::TAU;
use core::ops::Mul;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Mat4 {
    pub cols: [[f32; 4]; 4],
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub const ZERO: Self = Self {
        cols: [[0.0; 4]; 4],
    };

    pub fn look_at_rh(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Self {
        let f_unnorm = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
        let f = normalize3(f_unnorm);
        let r = normalize3(cross3(f, up));
        let u = cross3(r, f);

        Self {
            cols: [
                [r[0], u[0], -f[0], 0.0],
                [r[1], u[1], -f[1], 0.0],
                [r[2], u[2], -f[2], 0.0],
                [-dot3(r, eye), -dot3(u, eye), dot3(f, eye), 1.0],
            ],
        }
    }

    pub fn perspective_rh_zo(fov_y_rad: f32, aspect: f32, near: f32, far: f32) -> Self {
        let tan_half_fov = (fov_y_rad * 0.5).tan();
        let h = 1.0 / tan_half_fov;
        let w = h / aspect;
        let range_inv = 1.0 / (near - far);

        Self {
            cols: [
                [w, 0.0, 0.0, 0.0],
                [0.0, h, 0.0, 0.0],
                [0.0, 0.0, far * range_inv, -1.0],
                [0.0, 0.0, near * far * range_inv, 0.0],
            ],
        }
    }

    pub fn mul_mat4(&self, other: &Self) -> Self {
        let mut result = Self::ZERO;
        for c in 0..4 {
            for r in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += self.cols[k][r] * other.cols[c][k];
                }
                result.cols[c][r] = sum;
            }
        }
        result
    }

    pub const fn to_cols_array_2d(&self) -> [[f32; 4]; 4] {
        self.cols
    }

    pub fn to_cols_array(&self) -> [f32; 16] {
        let mut out = [0.0; 16];
        for c in 0..4 {
            for r in 0..4 {
                out[c * 4 + r] = self.cols[c][r];
            }
        }
        out
    }

    pub fn transform_point3(&self, p: [f32; 3]) -> [f32; 3] {
        let x = self.cols[0][0] * p[0]
            + self.cols[1][0] * p[1]
            + self.cols[2][0] * p[2]
            + self.cols[3][0];
        let y = self.cols[0][1] * p[0]
            + self.cols[1][1] * p[1]
            + self.cols[2][1] * p[2]
            + self.cols[3][1];
        let z = self.cols[0][2] * p[0]
            + self.cols[1][2] * p[1]
            + self.cols[2][2] * p[2]
            + self.cols[3][2];
        let w = self.cols[0][3] * p[0]
            + self.cols[1][3] * p[1]
            + self.cols[2][3] * p[2]
            + self.cols[3][3];

        if w.abs() > 1e-6 && (w - 1.0).abs() > 1e-6 {
            let inv_w = 1.0 / w;
            [x * inv_w, y * inv_w, z * inv_w]
        } else {
            [x, y, z]
        }
    }

    pub fn inverse(&self) -> Option<Self> {
        let m = self.to_cols_array().map(|v| v as f64);
        let mut inv = [0.0f64; 16];

        inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15]
            + m[9] * m[7] * m[14]
            + m[13] * m[6] * m[11]
            - m[13] * m[7] * m[10];

        inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15]
            - m[8] * m[7] * m[14]
            - m[12] * m[6] * m[11]
            + m[12] * m[7] * m[10];

        inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15]
            + m[8] * m[7] * m[13]
            + m[12] * m[5] * m[11]
            - m[12] * m[7] * m[9];

        inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14]
            - m[8] * m[6] * m[13]
            - m[12] * m[5] * m[10]
            + m[12] * m[6] * m[9];

        inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15]
            - m[9] * m[3] * m[14]
            - m[13] * m[2] * m[11]
            + m[13] * m[3] * m[10];

        inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15]
            + m[8] * m[3] * m[14]
            + m[12] * m[2] * m[11]
            - m[12] * m[3] * m[10];

        inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15]
            - m[8] * m[3] * m[13]
            - m[12] * m[1] * m[11]
            + m[12] * m[3] * m[9];

        inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14]
            + m[8] * m[2] * m[13]
            + m[12] * m[1] * m[10]
            - m[12] * m[2] * m[9];

        inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15]
            + m[5] * m[3] * m[14]
            + m[13] * m[2] * m[7]
            - m[13] * m[3] * m[6];

        inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15]
            - m[4] * m[3] * m[14]
            - m[12] * m[2] * m[7]
            + m[12] * m[3] * m[6];

        inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15]
            + m[4] * m[3] * m[13]
            + m[12] * m[1] * m[7]
            - m[12] * m[3] * m[5];

        inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14]
            - m[4] * m[2] * m[13]
            - m[12] * m[1] * m[6]
            + m[12] * m[2] * m[5];

        inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11]
            - m[5] * m[3] * m[10]
            - m[9] * m[2] * m[7]
            + m[9] * m[3] * m[6];

        inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11]
            + m[4] * m[3] * m[10]
            + m[8] * m[2] * m[7]
            - m[8] * m[3] * m[6];

        inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11]
            - m[4] * m[3] * m[9]
            - m[8] * m[1] * m[7]
            + m[8] * m[3] * m[5];

        inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10]
            + m[4] * m[2] * m[9]
            + m[8] * m[1] * m[6]
            - m[8] * m[2] * m[5];

        let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
        if det.abs() < 1e-12 {
            return None;
        }

        let inv_det = 1.0 / det;
        let mut cols = [[0.0f32; 4]; 4];
        for c in 0..4 {
            for r in 0..4 {
                cols[c][r] = (inv[c * 4 + r] * inv_det) as f32;
            }
        }
        Some(Self { cols })
    }
}

impl Mul<Mat4> for Mat4 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        self.mul_mat4(&rhs)
    }
}

impl Mul<&Mat4> for &Mat4 {
    type Output = Mat4;
    #[inline]
    fn mul(self, rhs: &Mat4) -> Mat4 {
        self.mul_mat4(rhs)
    }
}

#[inline]
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[inline]
fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len_sq = dot3(v, v);
    if len_sq > 1e-8 {
        let inv_len = 1.0 / len_sq.sqrt();
        [v[0] * inv_len, v[1] * inv_len, v[2] * inv_len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

pub const MYTH_HALF_FOV_DEG: f32 = 25.0;

pub mod myth_observer {
    pub const START_HEIGHT: f32 = 8.0;
    pub const MIN_TERRAIN_CLEARANCE: f32 = 2.0;
    pub const START_PITCH: f32 = 5.811_87 - core::f32::consts::TAU;
    pub const START_ZOOM: f32 = 0.0;
    pub const ZOOM_DISTANCE: f32 = 10.0;
    pub const ZOOM_PITCH: f32 = 0.157_04;

    pub fn eye_and_pitch(
        col: f32,
        row: f32,
        facing: f32,
        zoom: f32,
        ground_height: impl Fn(f32, f32) -> f32,
    ) -> ([f32; 3], f32) {
        let pitch = START_PITCH + ZOOM_PITCH * zoom.clamp(-1.0, 0.0);
        let len = zoom * ZOOM_DISTANCE;
        let (sp, cp) = pitch.sin_cos();
        let (sf, cf) = facing.sin_cos();
        let ec = col + len * cp * cf;
        let er = row + len * cp * sf;
        let ez = (START_HEIGHT + len * sp).max(ground_height(ec, er) + MIN_TERRAIN_CLEARANCE);
        ([ec, er, ez], pitch)
    }

    pub const ORBIT_PIVOT_DISTANCE: f32 = 12.25;
    pub const WORLD_UNITS_PER_CELL: f32 = crate::WORLD_UNITS_PER_CELL;
    pub const MAX_ZOOM: f32 = 0.999_984_7;
    pub const MIN_ZOOM_STANDARD: f32 = -0.999_984_7;
    pub const MIN_ZOOM_EXTENDED: f32 = -5.0;
    pub const ZOOM_PRESETS: [f32; 5] = [-5.0, -3.295_34, -2.021_65, -1.0, 0.0];
    pub const WHEEL_NOTCH_TICKS: u32 = 10;
    pub const EDGE_SCROLL_ZONE: f32 = 5.0;

    pub const MOVE_SCALE: f32 = 1.0 / 512.0;
    pub const ANGLE_SCALE: f32 = core::f32::consts::TAU / 65536.0;
    pub const ZOOM_SCALE: f32 = 1.0 / 65536.0;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct AxisConstants {
        pub negative_max: i32,
        pub negative_acceleration: i32,
        pub negative_deceleration: i32,
        pub positive_max: i32,
        pub positive_acceleration: i32,
        pub positive_deceleration: i32,
    }

    impl AxisConstants {
        const fn symmetric(max: i32, acceleration: i32, deceleration: i32) -> Self {
            Self {
                negative_max: max,
                negative_acceleration: acceleration,
                negative_deceleration: deceleration,
                positive_max: max,
                positive_acceleration: acceleration,
                positive_deceleration: deceleration,
            }
        }

        pub fn step(&self, v: f32, axis: f32, scale: f32) -> f32 {
            let magnitude = axis.abs();
            if axis > 0.0 {
                (magnitude * self.positive_max as f32 * scale)
                    .min(v + self.positive_acceleration as f32 * scale)
            } else if axis < 0.0 {
                (-(magnitude * self.negative_max as f32 * scale))
                    .max(v - self.negative_acceleration as f32 * scale)
            } else if v < 0.0 {
                (v + self.negative_deceleration as f32 * scale).min(0.0)
            } else {
                (v - self.positive_deceleration as f32 * scale).max(0.0)
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ObserverPhysicsConstants {
        pub forward: AxisConstants,
        pub strafe: AxisConstants,
        pub turn: AxisConstants,
        pub orbit: AxisConstants,
        pub unknown_0x60: AxisConstants,
        pub unknown_0x78: AxisConstants,
        pub zoom: AxisConstants,
    }

    impl ObserverPhysicsConstants {
        pub const RECORDS_SIZE: usize = 7 * 24;

        pub const SHIPPED_TAG: Self = Self {
            forward: AxisConstants::symmetric(195, 39, 32),
            strafe: AxisConstants::symmetric(195, 39, 32),
            turn: AxisConstants::symmetric(654, 163, 163),
            orbit: AxisConstants::symmetric(654, 163, 163),
            unknown_0x60: AxisConstants::symmetric(17, 5, 5),
            unknown_0x78: AxisConstants::symmetric(34, 34, 34),
            zoom: AxisConstants::symmetric(5242, 655, 655),
        };

        pub const BUILT_IN: Self = Self {
            forward: AxisConstants::symmetric(42, 4, 4),
            strafe: AxisConstants::symmetric(42, 4, 4),
            turn: AxisConstants::symmetric(68, 4, 4),
            orbit: AxisConstants::symmetric(546, 78, 91),
            unknown_0x60: AxisConstants::symmetric(8, 2, 2),
            unknown_0x78: AxisConstants::symmetric(34, 34, 34),
            zoom: AxisConstants::symmetric(10922, 1560, 1560),
        };

        pub fn from_tag_bytes(data: &[u8]) -> Option<Self> {
            if data.len() < Self::RECORDS_SIZE {
                return None;
            }
            let int =
                |i: usize| i32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            let axis = |at: usize| AxisConstants {
                negative_max: int(at),
                negative_acceleration: int(at + 4),
                negative_deceleration: int(at + 8),
                positive_max: int(at + 12),
                positive_acceleration: int(at + 16),
                positive_deceleration: int(at + 20),
            };
            Some(Self {
                forward: axis(0x00),
                strafe: axis(0x18),
                turn: axis(0x30),
                orbit: axis(0x48),
                unknown_0x60: axis(0x60),
                unknown_0x78: axis(0x78),
                zoom: axis(0x90),
            })
        }
    }

    impl Default for ObserverPhysicsConstants {
        fn default() -> Self {
            Self::SHIPPED_TAG
        }
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct ObserverAxes {
        pub forward: f32,
        pub left: f32,
        pub turn_left: f32,
        pub orbit_left: f32,
        pub zoom_in: f32,
    }

    impl ObserverAxes {
        pub fn clamped(self) -> Self {
            Self {
                forward: self.forward.clamp(-1.0, 1.0),
                left: self.left.clamp(-1.0, 1.0),
                turn_left: self.turn_left.clamp(-1.0, 1.0),
                orbit_left: self.orbit_left.clamp(-1.0, 1.0),
                zoom_in: self.zoom_in.clamp(-1.0, 1.0),
            }
        }
    }

    pub fn next_zoom_preset(zoom: f32, zoom_in: bool) -> f32 {
        if zoom_in {
            ZOOM_PRESETS
                .iter()
                .copied()
                .find(|&p| p > zoom)
                .unwrap_or(zoom)
        } else {
            ZOOM_PRESETS
                .iter()
                .rev()
                .copied()
                .find(|&p| p < zoom)
                .unwrap_or(zoom)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObserverLimits {
    pub half_width: f32,
    pub half_height: f32,
    pub width: i16,
    pub height: i16,
    pub margins: [i16; 4],
}

impl ObserverLimits {
    pub fn from_mesh_bytes(data: &[u8], width: u16, height: u16) -> Option<Self> {
        let at = |o: usize| data.get(o..o + 2).map(|b| i16::from_be_bytes([b[0], b[1]]));
        Some(Self {
            half_width: f32::from(width) * 0.5,
            half_height: f32::from(height) * 0.5,
            width: width as i16,
            height: height as i16,
            margins: [at(0x74)?, at(0x76)?, at(0x78)?, at(0x7a)?],
        })
    }

    pub fn edges(&self, facing: f32) -> [f32; 4] {
        use core::f32::consts::{FRAC_PI_2, PI};
        let inset = |margin: i16, edge: f32| {
            let m = f32::from(margin);
            if margin <= 4 {
                return m;
            }
            let cos = f64::from(real_angular_difference(facing, edge)).cos();
            ((f64::from(m) - (1.0 - cos) * f64::from(myth_observer::ORBIT_PIVOT_DISTANCE)) as f32)
                .max(4.0)
        };
        let (w, h) = (f32::from(self.width), f32::from(self.height));
        [
            inset(self.margins[0], 3.0 * FRAC_PI_2),
            inset(self.margins[1], PI),
            (h - 1.0) - inset(self.margins[2], FRAC_PI_2),
            (w - 1.0) - inset(self.margins[3], 0.0),
        ]
    }
}

fn real_angular_difference(a: f32, b: f32) -> f32 {
    let d = b - a;
    let mut r = (f64::from(d) % f64::from(TAU)) as f32;
    if d < 0.0 {
        r += TAU;
    }
    if r >= core::f32::consts::PI {
        TAU - r
    } else {
        r
    }
}

#[derive(Clone, Copy, Debug)]
struct LimitedPass {
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    rotation: f32,
}

impl LimitedPass {
    fn hold_at_edges(&mut self, [low_y, low_x, high_y, high_x]: [f32; 4], orbit: bool) {
        const CELL: f32 = 1.0;
        let mut scale_x = CELL;
        let mut common = CELL;
        if self.dx <= 0.0 && self.x < low_x + CELL {
            self.x = self.x.max(low_x);
            scale_x = self.x - low_x;
            if orbit {
                common = scale_x;
            }
        }
        if self.dx >= 0.0 && high_x - CELL < self.x {
            self.x = self.x.min(high_x);
            scale_x = high_x - self.x;
            if orbit {
                common = scale_x;
            }
        }
        let high = |y: f32, dy: f32| dy >= 0.0 && high_y - CELL < y;
        let mut scale_y = None;
        if self.dy <= 0.0 && self.y < low_y + CELL {
            self.y = self.y.max(low_y);
            scale_y = Some(self.y - low_y);
            if self.dy == 0.0 && high(self.y, self.dy) {
                self.y = self.y.min(high_y);
                scale_y = Some(high_y - self.y);
            }
        } else if high(self.y, self.dy) {
            self.y = self.y.min(high_y);
            scale_y = Some(high_y - self.y);
        }
        match scale_y {
            Some(sy) => {
                self.dy *= sy;
                if orbit {
                    let least = scale_x.min(sy);
                    self.dx *= least;
                    self.rotation *= least;
                } else {
                    self.dx *= scale_x;
                }
            }
            None => {
                self.dx *= scale_x;
                self.dy *= common;
                if orbit {
                    self.rotation *= scale_x.min(common);
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObserverController {
    pub origin: [f32; 3],
    pub yaw: f32,
    pub base_elevation: f32,
    pub zoom: f32,
    pub min_zoom: f32,
    pub velocity: [f32; 5],
    pub constants: myth_observer::ObserverPhysicsConstants,
    pub zoom_locked: bool,
}

impl ObserverController {
    pub fn from_camera(camera: &Camera) -> Self {
        Self {
            origin: camera.eye_position(),
            yaw: camera.yaw,
            base_elevation: camera.pitch,
            zoom: 0.0,
            min_zoom: myth_observer::MIN_ZOOM_EXTENDED,
            velocity: [0.0; 5],
            constants: myth_observer::ObserverPhysicsConstants::default(),
            zoom_locked: false,
        }
    }

    fn forward(yaw: f32) -> [f32; 2] {
        [-yaw.sin(), -yaw.cos()]
    }

    pub fn step(&mut self, axes: myth_observer::ObserverAxes) {
        self.integrate(axes);
        self.advance();
    }

    pub fn integrate(&mut self, axes: myth_observer::ObserverAxes) {
        use myth_observer::{ANGLE_SCALE, MOVE_SCALE, ZOOM_SCALE};
        let a = axes.clamped();
        let c = self.constants;
        let [fwd, left, turn, orbit, zoom] = self.velocity;
        self.velocity = [
            c.forward.step(fwd, a.forward, MOVE_SCALE),
            c.strafe.step(left, a.left, MOVE_SCALE),
            c.turn.step(turn, a.turn_left, ANGLE_SCALE),
            c.orbit.step(orbit, a.orbit_left, ANGLE_SCALE),
            if self.zoom_locked {
                0.0
            } else {
                c.zoom.step(zoom, a.zoom_in, ZOOM_SCALE)
            },
        ];
    }

    pub fn advance(&mut self) {
        self.advance_within(None, None);
    }

    pub fn advance_within(
        &mut self,
        limits: Option<&ObserverLimits>,
        explored: Option<&dyn Fn(i32, i32, u16) -> bool>,
    ) {
        let [fwd, left, turn, orbit, zoom] = self.velocity;
        let edges =
            limits.map(|l| l.edges((self.yaw - core::f32::consts::FRAC_PI_2).rem_euclid(TAU)));

        let f = Self::forward(self.yaw);
        let l = [f[1], -f[0]];
        self.motion_pass(
            limits.zip(edges),
            explored,
            false,
            f[0] * fwd + l[0] * left,
            f[1] * fwd + l[1] * left,
            0.0,
        );
        let r = myth_observer::ORBIT_PIVOT_DISTANCE;
        let f0 = Self::forward(self.yaw);
        let f1 = Self::forward(self.yaw + orbit);
        let (dx, dz) = if orbit != 0.0 {
            (r * (f0[0] - f1[0]), r * (f0[1] - f1[1]))
        } else {
            (0.0, 0.0)
        };
        self.motion_pass(limits.zip(edges), explored, true, dx, dz, orbit);
        self.yaw = (self.yaw + turn).rem_euclid(TAU);
        self.set_zoom(self.zoom + zoom);
        if self.zoom <= self.min_zoom || self.zoom >= myth_observer::MAX_ZOOM {
            self.velocity[4] = 0.0;
        }
    }

    fn motion_pass(
        &mut self,
        limits: Option<(&ObserverLimits, [f32; 4])>,
        explored: Option<&dyn Fn(i32, i32, u16) -> bool>,
        orbit_pass: bool,
        dx: f32,
        dz: f32,
        rotation: f32,
    ) {
        let (mut dx, mut dz, mut rotation) = (dx, dz, rotation);
        if let Some((limits, edges)) = limits {
            let mut p = LimitedPass {
                x: limits.half_width - self.origin[0],
                y: self.origin[2] + limits.half_height,
                dx: -dx,
                dy: dz,
                rotation,
            };
            p.hold_at_edges(edges, orbit_pass);
            self.origin[0] = limits.half_width - p.x;
            self.origin[2] = p.y - limits.half_height;
            (dx, dz, rotation) = (-p.dx, p.dy, p.rotation);
            if let Some(explored) = explored {
                let wu = myth_observer::WORLD_UNITS_PER_CELL;
                let facing = (self.yaw - core::f32::consts::FRAC_PI_2 + rotation).rem_euclid(TAU);
                let step = ((facing / TAU) * 65536.0) as i32 as u16;
                let target = (((p.x + p.dx) * wu) as i32, ((p.y + p.dy) * wu) as i32);
                if !explored(target.0, target.1, step) {
                    (dx, dz, rotation) = (0.0, 0.0, 0.0);
                }
            }
        }
        self.origin[0] += dx;
        self.origin[2] += dz;
        self.yaw = (self.yaw + rotation).rem_euclid(TAU);
    }

    pub fn orbit_by(&mut self, angle: f32) {
        let r = myth_observer::ORBIT_PIVOT_DISTANCE;
        let f0 = Self::forward(self.yaw);
        let f1 = Self::forward(self.yaw + angle);
        self.origin[0] += r * (f0[0] - f1[0]);
        self.origin[2] += r * (f0[1] - f1[1]);
        self.yaw = (self.yaw + angle).rem_euclid(TAU);
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(self.min_zoom, myth_observer::MAX_ZOOM);
    }

    pub fn cycle_zoom_preset(&mut self, zoom_in: bool) {
        self.set_zoom(myth_observer::next_zoom_preset(self.zoom, zoom_in));
        self.velocity[4] = 0.0;
    }

    pub fn pan(&mut self, delta_right: f32, delta_forward: f32) {
        let f = Self::forward(self.yaw);
        let r = [self.yaw.cos(), -self.yaw.sin()];
        self.origin[0] += f[0] * delta_forward + r[0] * delta_right;
        self.origin[2] += f[1] * delta_forward + r[1] * delta_right;
    }

    pub fn stop(&mut self) {
        self.velocity = [0.0; 5];
    }

    pub fn eye_and_elevation(&self) -> ([f32; 3], f32) {
        let elevation =
            self.base_elevation - myth_observer::ZOOM_PITCH * self.zoom.clamp(-1.0, 0.0);
        let len = self.zoom * myth_observer::ZOOM_DISTANCE;
        let (se, ce) = elevation.sin_cos();
        let f = Self::forward(self.yaw);
        (
            [
                self.origin[0] + len * ce * f[0],
                self.origin[1] - len * se,
                self.origin[2] + len * ce * f[1],
            ],
            elevation,
        )
    }

    pub fn apply_to(&self, camera: &mut Camera, ground_height: impl Fn(f32, f32) -> f32) {
        let (mut eye, elevation) = self.eye_and_elevation();
        eye[1] = eye[1].max(ground_height(eye[0], eye[2]) + myth_observer::MIN_TERRAIN_CLEARANCE);
        let ground_y = ground_height(self.origin[0], self.origin[2]);
        camera.place_eye(eye, self.yaw, elevation, ground_y);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraState {
    pub target: [f32; 3],
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineView {
    eye: [i32; 3],
    right: [i32; 3],
    up: [i32; 3],
    forward: [i32; 3],
}

impl EngineView {
    const EYE_TOLERANCE: f64 = 0.02;

    pub fn new(eye: [f32; 3], facing: u16, pitch: u16) -> Self {
        let wu = |c: f32| f64::from(c) * f64::from(crate::selection::WORLD_UNITS_PER_CELL);
        let t = Self::EYE_TOLERANCE;
        let eye = [
            -((wu(eye[0]) - t).ceil() as i32),
            (wu(eye[2]) + t).floor() as i32,
            (wu(eye[1]) + t).floor() as i32,
        ];
        let (sa, ca) = Self::sin_cos_shorts(facing);
        let (sb, cb) = Self::sin_cos_shorts(pitch);
        let shifted = |a: i32, b: i32| (-(a * b)) >> 14;
        Self {
            eye,
            right: [sa, -ca, 0],
            up: [shifted(ca, sb), shifted(sb, sa), cb],
            forward: [shifted(ca, cb), shifted(cb, sa), -sb],
        }
    }

    fn sin_cos_shorts(step: u16) -> (i32, i32) {
        const ONE: i32 = 0x4000;
        match step {
            0 => (0, ONE),
            0x4000 => (ONE, 0),
            0x8000 => (0, -ONE),
            0xc000 => (-ONE, 0),
            _ => {
                let radians = f32::from(step) * TAU / 65536.0;
                let (sin, cos) = f64::from(radians).sin_cos();
                ((sin * f64::from(ONE)) as i32, (cos * f64::from(ONE)) as i32)
            }
        }
    }

    pub fn view_coords(&self, world: [f32; 3]) -> [i32; 3] {
        let wu = |c: f32| {
            (f64::from(c) * f64::from(crate::selection::WORLD_UNITS_PER_CELL)).round() as i32
        };
        let d = [
            -wu(world[0]) - self.eye[0],
            wu(world[2]) - self.eye[1],
            wu(world[1]) - self.eye[2],
        ];
        let row = |r: [i32; 3]| {
            r[0].wrapping_mul(d[0])
                .wrapping_add(r[1].wrapping_mul(d[1]))
                .wrapping_add(r[2].wrapping_mul(d[2]))
                >> 14
        };
        [row(self.right), row(self.up), row(self.forward)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameProjector {
    view: EngineView,
    focal: f32,
    center: [i32; 2],
    viewport: [i32; 2],
}

impl FrameProjector {
    pub fn project(&self, world: [f32; 3]) -> Option<crate::selection::FrameProjection> {
        let [vx, vy, vz] = self.view.view_coords(world);
        if vz >= -0x22 {
            return None;
        }
        let k = self.focal / -vz as f32;
        Some(crate::selection::FrameProjection {
            origin: [
                self.center[0] as f32 + vx as f32 * k,
                self.center[1] as f32 - vy as f32 * k,
            ],
            pixels_per_unit: k,
            vertical_pixels_per_unit: k,
            viewport: self.viewport,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub target: [f32; 3],
    pub distance: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub principal_offset_y: f32,
    pub near: f32,
    pub far: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub min_pitch: f32,
    pub max_pitch: f32,
}

impl Camera {
    pub const DEFAULT_FOV_Y: f32 = 45.0 * (core::f32::consts::PI / 180.0);

    pub const ENGINE_NEAR: f32 = 0.5;

    pub const ENGINE_FAR: f32 = 360.0;

    pub fn myth_fov_y() -> f32 {
        2.0 * (0.75 * MYTH_HALF_FOV_DEG.to_radians().tan()).atan()
    }

    pub const MYTH_PRINCIPAL_OFFSET_Y: f32 = 2.0 / 7.0;

    pub fn from_myth_observer(eye: [f32; 3], facing: f32, pitch: f32, ground_y: f32) -> Self {
        let mut cam = Self::new([0.0; 3], 1.0, 0.0, 0.5);
        cam.place_eye(eye, facing + core::f32::consts::FRAC_PI_2, -pitch, ground_y);
        cam.fov_y = Self::myth_fov_y();
        cam.principal_offset_y = Self::MYTH_PRINCIPAL_OFFSET_Y;
        cam
    }

    pub fn place_eye(&mut self, eye: [f32; 3], yaw: f32, elevation: f32, ground_y: f32) {
        let drop = (eye[1] - ground_y).max(0.5);
        let distance = drop / elevation.sin().max(1e-3);
        let horiz = distance * elevation.cos();
        self.target = [
            eye[0] - horiz * yaw.sin(),
            eye[1] - distance * elevation.sin(),
            eye[2] - horiz * yaw.cos(),
        ];
        self.distance = distance;
        self.yaw = yaw.rem_euclid(TAU);
        self.pitch = elevation;
    }

    pub fn new(target: [f32; 3], distance: f32, yaw: f32, pitch: f32) -> Self {
        let min_distance = 3.0;
        let max_distance = 150.0;
        let min_pitch = 15.0 * (core::f32::consts::PI / 180.0);
        let max_pitch = 85.0 * (core::f32::consts::PI / 180.0);
        Self {
            target,
            distance: distance.clamp(min_distance, max_distance),
            yaw: yaw.rem_euclid(TAU),
            pitch: pitch.clamp(min_pitch, max_pitch),
            fov_y: Self::DEFAULT_FOV_Y,
            principal_offset_y: 0.0,
            near: Self::ENGINE_NEAR,
            far: Self::ENGINE_FAR,
            min_distance,
            max_distance,
            min_pitch,
            max_pitch,
        }
    }

    pub fn eye_position(&self) -> [f32; 3] {
        let cos_p = self.pitch.cos();
        let sin_p = self.pitch.sin();
        let cos_y = self.yaw.cos();
        let sin_y = self.yaw.sin();

        [
            self.target[0] + self.distance * cos_p * sin_y,
            self.target[1] + self.distance * sin_p,
            self.target[2] + self.distance * cos_p * cos_y,
        ]
    }

    pub fn ground_forward(&self) -> [f32; 3] {
        [-self.yaw.sin(), 0.0, -self.yaw.cos()]
    }

    pub fn ground_right(&self) -> [f32; 3] {
        [self.yaw.cos(), 0.0, -self.yaw.sin()]
    }

    pub fn view_matrix(&self) -> Mat4 {
        let eye = self.eye_position();
        Mat4::look_at_rh(eye, self.target, [0.0, 1.0, 0.0])
    }

    pub fn view_axes(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let eye = self.eye_position();
        let f = normalize3([
            self.target[0] - eye[0],
            self.target[1] - eye[1],
            self.target[2] - eye[2],
        ]);
        let r = normalize3(cross3(f, [0.0, 1.0, 0.0]));
        (r, cross3(r, f), f)
    }

    pub fn projection_matrix(&self, aspect_ratio: f32) -> Mat4 {
        let mut proj = Mat4::perspective_rh_zo(self.fov_y, aspect_ratio, self.near, self.far);
        proj.cols[2][1] -= self.principal_offset_y;
        proj
    }

    pub fn view_proj_matrix(&self, aspect_ratio: f32) -> Mat4 {
        let proj = self.projection_matrix(aspect_ratio);
        let view = self.view_matrix();
        proj * view
    }

    pub fn orbit(&mut self, yaw_delta: f32, pitch_delta: f32) {
        self.yaw = (self.yaw + yaw_delta).rem_euclid(TAU);
        self.pitch = (self.pitch + pitch_delta).clamp(self.min_pitch, self.max_pitch);
    }

    pub fn zoom(&mut self, distance_delta: f32) {
        self.distance =
            (self.distance + distance_delta).clamp(self.min_distance, self.max_distance);
    }

    pub fn pan_local(&mut self, forward_delta: f32, right_delta: f32) {
        let fwd = self.ground_forward();
        let right = self.ground_right();

        self.target[0] += fwd[0] * forward_delta + right[0] * right_delta;
        self.target[2] += fwd[2] * forward_delta + right[2] * right_delta;
    }

    pub fn pan(&mut self, delta_right: f32, delta_forward: f32) {
        self.pan_local(delta_forward, delta_right);
    }

    pub fn set_target(&mut self, target: [f32; 3]) {
        self.target = target;
    }

    pub fn current_state(&self) -> CameraState {
        CameraState {
            target: self.target,
            distance: self.distance,
            yaw: self.yaw,
            pitch: self.pitch,
        }
    }

    pub fn state(&self) -> CameraState {
        self.current_state()
    }

    pub fn interpolate(&self, prev: &CameraState, alpha: f32) -> Camera {
        let alpha = alpha.clamp(0.0, 1.0);
        let inv_alpha = 1.0 - alpha;

        let target = [
            prev.target[0] * inv_alpha + self.target[0] * alpha,
            prev.target[1] * inv_alpha + self.target[1] * alpha,
            prev.target[2] * inv_alpha + self.target[2] * alpha,
        ];

        let distance = prev.distance * inv_alpha + self.distance * alpha;

        let mut diff = (self.yaw - prev.yaw).rem_euclid(TAU);
        if diff > core::f32::consts::PI {
            diff -= TAU;
        }
        let yaw = (prev.yaw + diff * alpha).rem_euclid(TAU);

        let pitch = prev.pitch * inv_alpha + self.pitch * alpha;

        let mut result = self.clone();
        result.target = target;
        result.distance = distance;
        result.yaw = yaw;
        result.pitch = pitch;
        result
    }

    pub fn engine_view(&self) -> EngineView {
        const TURN: f64 = 65536.0;
        const STEP_TOLERANCE: f64 = 0.03;
        let tau = f64::from(TAU);
        let step = |radians: f64| {
            ((radians.rem_euclid(tau) / tau * TURN + STEP_TOLERANCE).floor() as i64 & 0xffff) as u16
        };
        let facing = step(f64::from(self.yaw) - f64::from(core::f32::consts::FRAC_PI_2));
        let pitch = step(-f64::from(self.pitch));
        EngineView::new(self.eye_position(), facing, pitch)
    }

    pub fn object_projection_center(&self, width: u32, height: u32) -> [i32; 2] {
        [
            width as i32 / 2,
            height as i32 / 2 + (-(self.principal_offset_y * 0.5) * height as f32 + 0.5) as i32,
        ]
    }

    pub fn frame_projector(&self, width: u32, height: u32) -> FrameProjector {
        let h = height.max(1) as f32;
        FrameProjector {
            view: self.engine_view(),
            focal: (h * 0.5) / (self.fov_y * 0.5).tan(),
            center: self.object_projection_center(width, height),
            viewport: [width as i32, height as i32],
        }
    }

    pub fn frame_projection(
        &self,
        world: [f32; 3],
        width: u32,
        height: u32,
    ) -> Option<crate::selection::FrameProjection> {
        self.frame_projector(width, height).project(world)
    }

    pub fn screen_ray(
        &self,
        cursor_x: f32,
        cursor_y: f32,
        width: u32,
        height: u32,
    ) -> ([f32; 3], [f32; 3]) {
        let w = width.max(1) as f32;
        let h = height.max(1) as f32;
        let ndc_x = (2.0 * cursor_x / w) - 1.0;
        let ndc_y = 1.0 - (2.0 * cursor_y / h);
        let aspect = w / h;

        let eye = self.eye_position();
        if let Some(inv_vp) = self.view_proj_matrix(aspect).inverse() {
            let near_world = inv_vp.transform_point3([ndc_x, ndc_y, 0.0]);
            let far_world = inv_vp.transform_point3([ndc_x, ndc_y, 1.0]);
            let dir = normalize3([
                far_world[0] - near_world[0],
                far_world[1] - near_world[1],
                far_world[2] - near_world[2],
            ]);
            (eye, dir)
        } else {
            let dir = normalize3([
                self.target[0] - eye[0],
                self.target[1] - eye[1],
                self.target[2] - eye[2],
            ]);
            (eye, dir)
        }
    }

    pub fn ray_ground_plane(
        &self,
        cursor_x: f32,
        cursor_y: f32,
        width: u32,
        height: u32,
        plane_y: f32,
    ) -> Option<[f32; 3]> {
        let (origin, dir) = self.screen_ray(cursor_x, cursor_y, width, height);
        if dir[1].abs() < 1e-6 {
            return None;
        }
        let t = (plane_y - origin[1]) / dir[1];
        if t < 0.0 {
            return None;
        }
        Some([origin[0] + t * dir[0], plane_y, origin[2] + t * dir[2]])
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new(
            [0.0, 0.0, 0.0],
            30.0,
            0.0,
            45.0 * (core::f32::consts::PI / 180.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_view_axes_span_unforeshortened_screen_rect() {
        let mut cam = Camera::new([10.0, 0.0, 5.0], 12.0, 0.7, 0.9);
        cam.fov_y = Camera::myth_fov_y();
        cam.principal_offset_y = Camera::MYTH_PRINCIPAL_OFFSET_Y;
        let (w, h) = (3840u32, 2160u32);
        let (r, u, f) = cam.view_axes();
        assert!(dot3(r, u).abs() < 1e-6 && dot3(r, f).abs() < 1e-6 && dot3(u, f).abs() < 1e-6);
        assert!(u[1] > 0.0 && f[1] < 0.0);
        let p = [10.5, 0.2, 4.0];
        let fp = cam.frame_projection(p, w, h).unwrap();
        let q = [p[0] + u[0], p[1] + u[1], p[2] + u[2]];
        let fq = cam.frame_projection(q, w, h).unwrap();
        let tolerance = 2.0 * fp.pixels_per_unit + 0.05;
        assert!((fq.pixels_per_unit - fp.pixels_per_unit).abs() < 1e-3);
        let dy = fp.origin[1] - fq.origin[1];
        assert!((dy - 512.0 * fp.pixels_per_unit).abs() < tolerance, "{dy}");
        assert!((fq.origin[0] - fp.origin[0]).abs() < tolerance);
    }

    fn le3e_t60_camera() -> Camera {
        let eye = [64.0 - 11547.0 / 512.0, 8.0, 45198.0 / 512.0 - 64.0];
        Camera::from_myth_observer(eye, 5.326173, 5.81187 - TAU, 0.0)
    }

    #[test]
    fn test_engine_view_is_the_reference_rotation() {
        let view = le3e_t60_camera().engine_view();
        assert_eq!(view.right, [-13393, -9436, 0]);
        assert_eq!(view.up, [4284, -6081, 14597]);
        assert_eq!(view.forward, [-8407, 11932, 7439]);
    }

    #[test]
    fn test_engine_view_coords_are_the_reference_ones() {
        let cam = le3e_t60_camera();
        let world = [64.0 - 14828.0 / 512.0, 50.0 / 512.0, 42149.0 / 512.0 - 64.0];
        assert_eq!(cam.engine_view().view_coords(world), [-927, -1616, -5742]);
    }

    #[test]
    fn test_engine_view_axes_follow_the_camera() {
        for (yaw, pitch) in [(0.3, 0.5), (2.1, 0.47), (4.0, 0.9), (5.9, 0.2)] {
            let mut cam = Camera::new([10.0, 0.0, 5.0], 12.0, yaw, pitch);
            cam.pitch = pitch;
            let (r, u, f) = cam.view_axes();
            let view = cam.engine_view();
            let client = |a: [f32; 3]| [-a[0], a[2], a[1]].map(|c| c * 16384.0);
            for (engine, float) in [
                (view.right, client(r)),
                (view.up, client(u)),
                (view.forward, client(f).map(|c| -c)),
            ] {
                for (e, c) in engine.iter().zip(float) {
                    assert!((*e as f32 - c).abs() < 4.0, "{engine:?} {c}");
                }
            }
        }
    }

    #[test]
    fn test_engine_rotation_is_exact_on_the_axes() {
        assert_eq!(EngineView::sin_cos_shorts(0), (0, 0x4000));
        assert_eq!(EngineView::sin_cos_shorts(0x4000), (0x4000, 0));
        assert_eq!(EngineView::sin_cos_shorts(0x8000), (0, -0x4000));
        assert_eq!(EngineView::sin_cos_shorts(0xc000), (-0x4000, 0));
    }

    #[test]
    fn test_frame_projection_scale_matches_projection() {
        let mut cam = Camera::new([10.0, 0.0, 5.0], 12.0, 0.7, 0.5);
        cam.fov_y = Camera::myth_fov_y();
        cam.principal_offset_y = Camera::MYTH_PRINCIPAL_OFFSET_Y;
        let (w, h) = (3840u32, 2160u32);
        let p = [10.5, 0.2, 4.0];
        let fp = cam.frame_projection(p, w, h).unwrap();
        assert_eq!(fp.viewport, [3840, 2160]);
        let right = cam.ground_right();
        let q = [p[0] + right[0], p[1], p[2] + right[2]];
        let fq = cam.frame_projection(q, w, h).unwrap();
        let tolerance = 2.0 * fp.pixels_per_unit + 0.05;
        let dx = fq.origin[0] - fp.origin[0];
        assert!((dx - 512.0 * fp.pixels_per_unit).abs() < tolerance, "{dx}");
        assert!((fq.origin[1] - fp.origin[1]).abs() < tolerance);
        let eye = cam.eye_position();
        let fwd = cam.ground_forward();
        let behind = [eye[0] - fwd[0], eye[1], eye[2] - fwd[2]];
        assert!(cam.frame_projection(behind, w, h).is_none());
    }

    #[test]
    fn test_object_projection_center_is_whole() {
        let mut cam = Camera::new([10.0, 0.0, 5.0], 12.0, 0.7, 0.5);
        cam.principal_offset_y = Camera::MYTH_PRINCIPAL_OFFSET_Y;
        assert_eq!(cam.object_projection_center(1024, 768), [512, 64 + 211]);
    }

    #[test]
    fn test_object_projection_center_shift_from_principal_point() {
        let mut cam = Camera::new([10.0, 0.0, 5.0], 12.0, 0.7, 0.5);
        cam.principal_offset_y = Camera::MYTH_PRINCIPAL_OFFSET_Y;
        for (w, h, shift) in [
            (1024, 768, [0.0, 5.0 / 7.0]),
            (1280, 720, [0.0, 6.0 / 7.0]),
            (1920, 1080, [0.0, 9.0 / 7.0]),
            (1025, 768, [-0.5, 5.0 / 7.0]),
        ] {
            let exact = [
                w as f32 * 0.5,
                h as f32 * 0.5 * (1.0 - cam.principal_offset_y),
            ];
            let [cx, cy] = cam.object_projection_center(w, h);
            assert!((cx as f32 - exact[0] - shift[0]).abs() < 1e-3, "{w}x{h}");
            assert!((cy as f32 - exact[1] - shift[1]).abs() < 1e-3, "{w}x{h}");
        }
    }

    #[test]
    fn test_identity_matrix() {
        let id = Mat4::IDENTITY;
        let p = [3.0, 4.0, 5.0];
        assert_eq!(id.transform_point3(p), p);
    }

    #[test]
    fn test_look_at_and_projection() {
        let eye = [0.0, 10.0, 20.0];
        let target = [0.0, 0.0, 0.0];
        let up = [0.0, 1.0, 0.0];

        let view = Mat4::look_at_rh(eye, target, up);
        let transformed_target = view.transform_point3(target);
        assert!(transformed_target[2] < 0.0);
        assert!(transformed_target[0].abs() < 1e-4);

        let proj = Mat4::perspective_rh_zo(Camera::DEFAULT_FOV_Y, 16.0 / 9.0, 0.5, 1000.0);
        let vp = proj * view;

        let p_near = [
            0.0,
            10.0 - 0.5 * (10.0 / 22.36068),
            20.0 - 0.5 * (20.0 / 22.36068),
        ];
        let p_ndc = vp.transform_point3(p_near);
        assert!(p_ndc[2] >= -0.05 && p_ndc[2] <= 0.1);
    }

    #[test]
    fn test_camera_orbit_pan_zoom() {
        let mut cam = Camera::default();
        assert_eq!(cam.min_distance, 3.0);
        assert_eq!(cam.max_distance, 150.0);

        let initial_eye = cam.eye_position();
        assert!((initial_eye[1] - (30.0 * 45.0f32.to_radians().sin())).abs() < 1e-3);

        cam.orbit(core::f32::consts::PI * 0.5, 0.0);
        assert!((cam.yaw - core::f32::consts::PI * 0.5).abs() < 1e-4);

        cam.zoom(-10.0);
        assert_eq!(cam.distance, 20.0);
        cam.zoom(-500.0);
        assert_eq!(cam.distance, 3.0);
        cam.zoom(500.0);
        assert_eq!(cam.distance, 150.0);

        let target_before = cam.target;
        cam.pan_local(10.0, 5.0);
        assert_ne!(cam.target, target_before);

        cam.orbit(0.0, 10.0);
        assert!(cam.pitch <= cam.max_pitch);
        cam.orbit(0.0, -20.0);
        assert!(cam.pitch >= cam.min_pitch);
    }

    #[test]
    fn test_screen_ray_exact_inverse_projection() {
        let cam = Camera::new([12.5, 1.5, -8.0], 24.0, -0.65, 0.58);
        let width = 1920u32;
        let height = 1080u32;
        let aspect = width as f32 / height as f32;
        let vp = cam.view_proj_matrix(aspect);

        for world_pt in [
            cam.target,
            [15.0, 0.0, -5.0],
            [8.0, 2.0, -12.0],
            [20.0, -1.0, -3.0],
        ] {
            let ndc = vp.transform_point3(world_pt);
            let cursor_x = (ndc[0] + 1.0) * 0.5 * width as f32;
            let cursor_y = (1.0 - ndc[1]) * 0.5 * height as f32;

            let hit = cam
                .ray_ground_plane(cursor_x, cursor_y, width, height, world_pt[1])
                .expect("Ray should intersect plane of projected point");

            assert!(
                (hit[0] - world_pt[0]).abs() < 1e-3,
                "X mismatch: got {}, expected {}",
                hit[0],
                world_pt[0]
            );
            assert!(
                (hit[2] - world_pt[2]).abs() < 1e-3,
                "Z mismatch: got {}, expected {}",
                hit[2],
                world_pt[2]
            );
        }
    }

    #[test]
    fn test_frame_projector_matches_frame_projection() {
        let mut cam = Camera::new([10.0, 0.0, 5.0], 12.0, 0.7, 0.5);
        cam.fov_y = Camera::myth_fov_y();
        cam.principal_offset_y = Camera::MYTH_PRINCIPAL_OFFSET_Y;
        let (w, h) = (1920u32, 1080u32);
        let projector = cam.frame_projector(w, h);
        let eye = cam.eye_position();
        let fwd = cam.ground_forward();
        for p in [
            [10.5, 0.2, 4.0],
            [8.0, -1.0, 6.5],
            eye,
            [eye[0] - fwd[0], eye[1], eye[2] - fwd[2]],
        ] {
            assert_eq!(projector.project(p), cam.frame_projection(p, w, h));
        }
    }

    #[test]
    fn test_observer_physics_subtick_interpolation() {
        let cam = Camera::new([10.0, 2.0, 5.0], 30.0, 0.2, 0.6);
        let prev_state = CameraState {
            target: [0.0, 0.0, 0.0],
            distance: 20.0,
            yaw: 0.0,
            pitch: 0.4,
        };

        let c0 = cam.interpolate(&prev_state, 0.0);
        assert_eq!(c0.target, prev_state.target);
        assert_eq!(c0.distance, prev_state.distance);
        assert_eq!(c0.yaw, prev_state.yaw);
        assert_eq!(c0.pitch, prev_state.pitch);

        let c1 = cam.interpolate(&prev_state, 1.0);
        assert_eq!(c1.target, cam.target);
        assert_eq!(c1.distance, cam.distance);
        assert_eq!(c1.yaw, cam.yaw);
        assert_eq!(c1.pitch, cam.pitch);

        let c_mid = cam.interpolate(&prev_state, 0.5);
        assert!((c_mid.target[0] - 5.0).abs() < 1e-4);
        assert!((c_mid.distance - 25.0).abs() < 1e-4);
        assert!((c_mid.yaw - 0.1).abs() < 1e-4);
        assert!((c_mid.pitch - 0.5).abs() < 1e-4);

        let cam_wrap = Camera::new([0.0, 0.0, 0.0], 20.0, 0.1, 0.5);
        let prev_wrap = CameraState {
            target: [0.0, 0.0, 0.0],
            distance: 20.0,
            yaw: TAU - 0.1,
            pitch: 0.5,
        };
        let c_wrap = cam_wrap.interpolate(&prev_wrap, 0.5);
        assert!(
            c_wrap.yaw.abs() < 1e-3 || (c_wrap.yaw - TAU).abs() < 1e-3,
            "Interpolated yaw should take shortest arc through zero: got {}",
            c_wrap.yaw
        );
    }

    #[test]
    fn test_myth_observer_start_eye_and_pitch() {
        let flat = |_: f32, _: f32| 1.0;
        let (eye, pitch) = myth_observer::eye_and_pitch(50.0, 70.0, 1.0, 0.0, flat);
        assert_eq!(eye, [50.0, 70.0, myth_observer::START_HEIGHT]);
        assert!((pitch.to_degrees() + 27.0).abs() < 0.01);

        let (eye_out, pitch_out) = myth_observer::eye_and_pitch(50.0, 70.0, 0.0, -1.0, flat);
        assert!(eye_out[0] < 50.0 && eye_out[2] > myth_observer::START_HEIGHT);
        assert!((pitch_out.to_degrees() + 36.0).abs() < 0.01);
        let (_, pitch_in) = myth_observer::eye_and_pitch(50.0, 70.0, 0.0, 0.5, flat);
        assert_eq!(pitch_in, myth_observer::START_PITCH);

        let high = |_: f32, _: f32| 7.5;
        let (eye_hi, _) = myth_observer::eye_and_pitch(0.0, 0.0, 0.0, 0.0, high);
        assert_eq!(eye_hi[2], 9.5);
    }

    fn myth_camera() -> Camera {
        Camera::from_myth_observer([3.0, 8.0, -4.0], 1.1, myth_observer::START_PITCH, 0.0)
    }

    #[test]
    fn test_obpc_axis_step_accelerates_clamps_and_decays() {
        use myth_observer::{AxisConstants, ObserverPhysicsConstants};
        let c: AxisConstants = ObserverPhysicsConstants::SHIPPED_TAG.forward;
        let s = myth_observer::MOVE_SCALE;
        let mut v = 0.0;
        for tick in 1..=6 {
            v = c.step(v, 1.0, s);
            let expect = (39.0 * tick as f32).min(195.0) * s;
            assert!((v - expect).abs() < 1e-6, "tick {tick}: {v} != {expect}");
        }
        assert!((c.step(v, 0.5, s) - 97.5 * s).abs() < 1e-6);
        let mut d = v;
        let mut ticks = 0;
        while d > 0.0 {
            d = c.step(d, 0.0, s);
            ticks += 1;
        }
        assert_eq!(d, 0.0);
        assert_eq!(ticks, 7);
        assert!((c.step(0.0, -1.0, s) + 39.0 * s).abs() < 1e-6);
        assert_eq!(c.step(-s, 0.0, s), 0.0);
    }

    #[test]
    fn test_obpc_tag_bytes_parse() {
        use myth_observer::ObserverPhysicsConstants;
        let mut bytes = Vec::new();
        for v in [
            195, 39, 32, 195, 39, 32, 195, 39, 32, 195, 39, 32, 654, 163, 163, 654, 163, 163, 654,
            163, 163, 654, 163, 163, 17, 5, 5, 17, 5, 5, 34, 34, 34, 34, 34, 34, 5242, 655, 655,
            5242, 655, 655,
        ] {
            bytes.extend_from_slice(&i32::to_be_bytes(v));
        }
        bytes.extend_from_slice(&[0u8; 88]);
        assert_eq!(
            ObserverPhysicsConstants::from_tag_bytes(&bytes),
            Some(ObserverPhysicsConstants::SHIPPED_TAG)
        );
        assert_eq!(
            ObserverPhysicsConstants::from_tag_bytes(&bytes[..100]),
            None
        );
    }

    #[test]
    fn test_observer_controller_reproduces_start_camera() {
        let cam = myth_camera();
        let ctl = ObserverController::from_camera(&cam);
        let mut out = Camera {
            fov_y: cam.fov_y,
            principal_offset_y: cam.principal_offset_y,
            ..Camera::default()
        };
        ctl.apply_to(&mut out, |_, _| 0.0);
        for i in 0..3 {
            assert!((out.eye_position()[i] - cam.eye_position()[i]).abs() < 1e-4);
            assert!((out.target[i] - cam.target[i]).abs() < 1e-4);
        }
        assert!((out.yaw - cam.yaw).abs() < 1e-6 && (out.pitch - cam.pitch).abs() < 1e-6);
    }

    #[test]
    fn test_observer_orbit_circles_pivot_and_turn_rotates_in_place() {
        let cam = myth_camera();
        let pivot = |c: &ObserverController| {
            let f = c.yaw;
            let r = myth_observer::ORBIT_PIVOT_DISTANCE;
            [c.origin[0] - r * f.sin(), c.origin[2] - r * f.cos()]
        };
        let mut ctl = ObserverController::from_camera(&cam);
        let p0 = pivot(&ctl);
        for _ in 0..20 {
            ctl.step(myth_observer::ObserverAxes {
                orbit_left: 1.0,
                ..Default::default()
            });
        }
        let p1 = pivot(&ctl);
        assert!((p0[0] - p1[0]).abs() < 1e-3 && (p0[1] - p1[1]).abs() < 1e-3);
        assert!(
            (ctl.yaw - cam.yaw).abs() > 0.5,
            "orbit should change the heading"
        );
        let expected: f32 = (0..20)
            .map(|t| ((t + 1) as f32 * 163.0).min(654.0) * myth_observer::ANGLE_SCALE)
            .sum();
        assert!(((ctl.yaw - cam.yaw).rem_euclid(TAU) - expected).abs() < 1e-4);

        let mut turn = ObserverController::from_camera(&cam);
        let origin = turn.origin;
        for _ in 0..10 {
            turn.step(myth_observer::ObserverAxes {
                turn_left: -1.0,
                ..Default::default()
            });
        }
        assert_eq!(turn.origin, origin);
        assert!((turn.yaw - cam.yaw).rem_euclid(TAU) > core::f32::consts::PI);
    }

    #[test]
    fn test_observer_move_directions_match_camera_axes() {
        let cam = myth_camera();
        let mut ctl = ObserverController::from_camera(&cam);
        ctl.step(myth_observer::ObserverAxes {
            left: 1.0,
            ..Default::default()
        });
        let d = [
            ctl.origin[0] - cam.eye_position()[0],
            ctl.origin[2] - cam.eye_position()[2],
        ];
        let right = cam.ground_right();
        let step = 39.0 * myth_observer::MOVE_SCALE;
        assert!((d[0] + right[0] * step).abs() < 1e-5 && (d[1] + right[2] * step).abs() < 1e-5);

        let mut ctl = ObserverController::from_camera(&cam);
        ctl.step(myth_observer::ObserverAxes {
            forward: 1.0,
            ..Default::default()
        });
        let fwd = cam.ground_forward();
        let d = [
            ctl.origin[0] - cam.eye_position()[0],
            ctl.origin[2] - cam.eye_position()[2],
        ];
        assert!((d[0] - fwd[0] * step).abs() < 1e-5 && (d[1] - fwd[2] * step).abs() < 1e-5);
    }

    #[test]
    fn test_observer_zoom_limits_presets_and_pitch() {
        use myth_observer::next_zoom_preset;
        assert_eq!(next_zoom_preset(0.3, false), 0.0);
        assert_eq!(next_zoom_preset(0.0, false), -1.0);
        assert_eq!(next_zoom_preset(-1.0, false), -2.021_65);
        assert_eq!(next_zoom_preset(-4.0, false), -5.0);
        assert_eq!(next_zoom_preset(-5.0, false), -5.0);
        assert_eq!(next_zoom_preset(-5.0, true), -3.295_34);
        assert_eq!(next_zoom_preset(-0.5, true), 0.0);
        assert_eq!(next_zoom_preset(0.5, true), 0.5);

        let cam = myth_camera();
        let mut ctl = ObserverController::from_camera(&cam);
        ctl.min_zoom = myth_observer::MIN_ZOOM_STANDARD;
        ctl.cycle_zoom_preset(false);
        ctl.cycle_zoom_preset(false);
        assert_eq!(ctl.zoom, myth_observer::MIN_ZOOM_STANDARD);
        ctl.set_zoom(-1.0);
        let (eye, elevation) = ctl.eye_and_elevation();
        assert!((elevation - cam.pitch - myth_observer::ZOOM_PITCH).abs() < 1e-3);
        let back = ((eye[0] - ctl.origin[0]).powi(2)
            + (eye[1] - ctl.origin[1]).powi(2)
            + (eye[2] - ctl.origin[2]).powi(2))
        .sqrt();
        assert!((back - myth_observer::ZOOM_DISTANCE * 0.999_984_7).abs() < 1e-3);
        for _ in 0..200 {
            ctl.step(myth_observer::ObserverAxes {
                zoom_in: 1.0,
                ..Default::default()
            });
        }
        assert_eq!(ctl.zoom, myth_observer::MAX_ZOOM);
    }

    #[test]
    fn test_myth_projection_fov_and_principal_point() {
        assert!((Camera::myth_fov_y().to_degrees() - 38.548).abs() < 0.01);

        let eye = [3.0, 8.0, -4.0];
        let facing = 228.0f32.to_radians();
        let pitch = myth_observer::START_PITCH;
        let cam = Camera::from_myth_observer(eye, facing, pitch, 0.0);
        let e = cam.eye_position();
        for i in 0..3 {
            assert!((e[i] - eye[i]).abs() < 1e-3, "eye {e:?} != {eye:?}");
        }
        assert!((cam.pitch + pitch).abs() < 1e-5);
        assert!(cam.target[1].abs() < 1e-3);

        let fwd = cam.ground_forward();
        assert!((fwd[0] + facing.cos()).abs() < 1e-5);
        assert!((fwd[2] - facing.sin()).abs() < 1e-5);

        let vp = cam.view_proj_matrix(16.0 / 9.0);
        let ndc = vp.transform_point3(cam.target);
        assert!(ndc[0].abs() < 1e-4);
        assert!((ndc[1] - 2.0 / 7.0).abs() < 1e-4);

        let (_, dir) = cam.screen_ray(960.0, 540.0 - 1080.0 / 7.0, 1920, 1080);
        let axis = normalize3([
            cam.target[0] - e[0],
            cam.target[1] - e[1],
            cam.target[2] - e[2],
        ]);
        assert!(dot3(dir, axis) > 0.99999);
    }
}
