use crate::billboard::{
    BILLBOARD_FLAG_REFLECTION, BILLBOARD_FLAG_VELOCITY_QUAD, BillboardInstance,
};

const WORLD_ONE: f32 = crate::WORLD_UNITS_PER_CELL;

pub fn fade_ratio(count: u8, period: u8) -> Option<i32> {
    let (c, p) = (i32::from(count as i8), i32::from(period as i8));
    (p != 0).then(|| ((c << 16) / p).min(0xffff))
}

pub fn transfer_transparency(mode: u8, ratio: i32, sine: &[i16; 1024]) -> Option<u16> {
    match mode {
        1 => Some(ratio as u16),
        2 => Some(!ratio as u16),
        5 => Some(!(ratio >> 1) as u16),
        6 => Some(((ratio >> 1) as u16).wrapping_add(0x8000)),
        7 => {
            let i = ((ratio.wrapping_add(0x20) as u32 & 0xffff) >> 6) as usize;
            Some((sine[i] as u16).wrapping_add(0x7fff))
        }
        _ => None,
    }
}

pub fn bob_offset(flags: u32, object: i32, tick: i32, cosine: &[i16; 1024]) -> i32 {
    if flags & 0x800 == 0 && flags & 0x1020 != 0x1020 {
        return 0;
    }
    let i = (object as i16 as i32)
        .wrapping_mul(0x39)
        .wrapping_add(tick)
        .wrapping_shl(4)
        & 0x3f0;
    i32::from(cosine[i as usize] >> 9)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineViewRows {
    pub right: [i16; 3],
    pub up: [i16; 3],
    pub depth: [i16; 3],
}

impl EngineViewRows {
    pub fn from_camera_axes(right: [f32; 3], up: [f32; 3], forward: [f32; 3]) -> Self {
        let row = |a: [f32; 3], sign: f32| {
            [-a[0], a[2], a[1]].map(|c| (c * sign * 16384.0).round() as i16)
        };
        Self {
            right: row(right, 1.0),
            up: row(up, 1.0),
            depth: row(forward, -1.0),
        }
    }
}

pub fn velocity_direction(rows: &EngineViewRows, v: [i16; 3], reflection: bool) -> [i32; 3] {
    view_direction_and_length(rows, v, reflection).0
}

fn view_direction_and_length(
    rows: &EngineViewRows,
    v: [i16; 3],
    reflection: bool,
) -> ([i32; 3], i32) {
    let [vx, vy, vz] = v.map(i32::from);
    let dot = |r: [i16; 3], z: i32| {
        i32::from(r[0])
            .wrapping_mul(vx)
            .wrapping_add(i32::from(r[1]).wrapping_mul(vy))
            .wrapping_add(i32::from(r[2]).wrapping_mul(z))
            >> 14
    };
    let x = dot(rows.right, vz);
    let up = dot(rows.up, if reflection { -vz } else { vz });
    let d = dot(rows.depth, vz);
    let sq = |c: i32| c.wrapping_mul(c);
    let len = isqrt(sq(x).wrapping_add(sq(up)).wrapping_add(sq(d)));
    if len == 0 {
        return ([x, up, d], len);
    }
    ([(x << 16) / len, (up << 16) / len, (d << 16) / len], len)
}

fn isqrt(n: i32) -> i32 {
    mandrake_math::engine::isqrt(n as u32) as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceRect {
    pub top: i16,
    pub left: i16,
    pub right: i16,
    pub bottom: i16,
}

impl InstanceRect {
    pub fn from_sprite(size: [f32; 2], anchor: [f32; 2]) -> Self {
        let world = |v: f32| (v * WORLD_ONE).round() as i16;
        Self {
            top: world(anchor[1] * size[1]),
            left: world(-anchor[0] * size[0]),
            right: world((1.0 - anchor[0]) * size[0]),
            bottom: world(-(1.0 - anchor[1]) * size[1]),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VelocityQuad {
    pub direction: [i32; 3],
    pub a: [i32; 2],
    pub b: [i32; 2],
}

impl VelocityQuad {
    pub fn new(
        rows: &EngineViewRows,
        velocity: [i16; 3],
        rect: InstanceRect,
        scale: i32,
        reflection: bool,
    ) -> Self {
        let scaled = |e: i16| (i32::from(e) * scale) >> 8;
        let (top, bottom) = if reflection {
            (rect.bottom.wrapping_neg(), rect.top.wrapping_neg())
        } else {
            (rect.top, rect.bottom)
        };
        Self {
            direction: velocity_direction(rows, velocity, reflection),
            a: [scaled(rect.left), scaled(rect.right)],
            b: [scaled(top), scaled(bottom)],
        }
    }

    pub fn corner(&self, i: usize, j: usize) -> [i32; 3] {
        let [ux, uy, uz] = self.direction;
        let (a, b) = (self.a[i], self.b[j]);
        [
            (a.wrapping_mul(-ux).wrapping_add(b.wrapping_mul(-uy))) >> 16,
            (a.wrapping_mul(-uy).wrapping_add(b.wrapping_mul(ux))) >> 16,
            a.wrapping_mul(-uz) >> 16,
        ]
    }

    pub fn segment(
        rows: &EngineViewRows,
        velocity: [i16; 3],
        rect: InstanceRect,
        height_scale: i32,
        extra: i32,
        reflection: bool,
    ) -> Self {
        let scaled = |e: i16| (i32::from(e) * height_scale) >> 8;
        let (top, bottom) = if reflection {
            (rect.bottom.wrapping_neg(), rect.top.wrapping_neg())
        } else {
            (rect.top, rect.bottom)
        };
        let (direction, length) = view_direction_and_length(rows, velocity, reflection);
        Self {
            direction,
            a: [0, length + extra],
            b: [scaled(top), scaled(bottom)],
        }
    }

    pub fn instance(&self, mut sprite: BillboardInstance, reflection: bool) -> BillboardInstance {
        sprite.flags |= BILLBOARD_FLAG_VELOCITY_QUAD;
        sprite.flags &= !crate::billboard::BILLBOARD_FLAG_SCREEN_ALIGNED;
        if reflection {
            sprite.flags |= BILLBOARD_FLAG_REFLECTION;
        }
        sprite.size = self.a.map(|v| v as f32 / WORLD_ONE);
        sprite.anchor = self.b.map(|v| v as f32 / WORLD_ONE);
        let [x, y, z] = self.direction.map(|c| c as f32 / 65536.0);
        sprite.media = [x, y, z, 0.0];
        sprite
    }
}

pub const QUAD_TEXTURE_LOW: f32 = 64.0 / 65536.0;
pub const QUAD_TEXTURE_HIGH: f32 = 65472.0 / 65536.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_rounds_to_nearest_like_the_engine() {
        for (n, want) in [
            (0, 0),
            (1, 1),
            (2, 1),
            (3, 2),
            (6, 2),
            (7, 3),
            (12, 3),
            (13, 4),
            (15, 4),
            (16, 4),
            (110, 10),
            (111, 11),
            (0x4000_0000 + 0x8000, 0x8000),
            (0x4000_0000 + 0x8001, 0x8001),
        ] {
            assert_eq!(isqrt(n), want, "isqrt({n})");
        }
        assert_eq!(isqrt(-1), 0x1_0000);
    }

    const ROWS: EngineViewRows = EngineViewRows {
        right: [16384, 0, 0],
        up: [0, 0, 16384],
        depth: [0, -16384, 0],
    };
    const ARROW: InstanceRect = InstanceRect {
        top: 70,
        left: -160,
        right: 160,
        bottom: -10,
    };

    fn sine() -> [i16; 1024] {
        mandrake_math::engine::SINE
    }

    #[test]
    fn fade_curves_of_the_transfer_modes() {
        let s = sine();
        let r = fade_ratio(1, 28).unwrap();
        assert_eq!(r, 0x0924);
        assert_eq!(transfer_transparency(1, r, &s), Some(0x0924));
        assert_eq!(fade_ratio(28, 28), Some(0xffff), "clamped at 0xffff");
        assert_eq!(transfer_transparency(1, 0xffff, &s), Some(0xffff));
        assert_eq!(transfer_transparency(2, 0x4000, &s), Some(0xbfff));
        assert_eq!(transfer_transparency(5, 0x4000, &s), Some(0xdfff));
        assert_eq!(transfer_transparency(6, 0x4000, &s), Some(0xa000));
        assert_eq!(transfer_transparency(7, 0, &s), Some(0x7fff));
        assert_eq!(
            transfer_transparency(7, 0x4000, &s),
            Some(0x7fffu16.wrapping_add(16384))
        );
        for m in [0, 3, 4, 8] {
            assert_eq!(transfer_transparency(m, 0x8000, &s), None);
        }
        assert_eq!(fade_ratio(1, 0), None);
    }

    #[test]
    fn the_quad_lies_along_the_velocity_with_the_bitmap_left_edge_forward() {
        let q = VelocityQuad::new(&ROWS, [100, 0, 0], ARROW, 256, false);
        assert_eq!(q.direction, [65536, 0, 0]);
        assert_eq!(q.corner(0, 0), [160, 70, 0]);
        assert_eq!(q.corner(1, 0), [-160, 70, 0]);
        assert_eq!(q.corner(0, 1), [160, -10, 0]);
        assert_eq!(q.corner(1, 1), [-160, -10, 0]);
    }

    #[test]
    fn the_quad_pitches_with_the_velocity() {
        let q = VelocityQuad::new(&ROWS, [0, 0, 50], ARROW, 256, false);
        assert_eq!(q.direction, [0, 65536, 0]);
        assert_eq!(q.corner(0, 0), [-70, 160, 0]);
        assert_eq!(q.corner(1, 1), [10, -160, 0]);
    }

    #[test]
    fn the_quad_is_foreshortened_along_the_view_axis() {
        let q = VelocityQuad::new(&ROWS, [0, 100, 100], ARROW, 256, false);
        assert_eq!(q.direction, [0, 46479, -46479]);
        assert_eq!(q.corner(0, 0)[1], (-160 * -46479) >> 16);
        assert_eq!(q.corner(0, 0)[2], (-160 * 46479) >> 16);
        assert_eq!(q.corner(0, 0)[0], (70 * -46479) >> 16);
    }

    #[test]
    fn the_quad_scale_and_the_reflection() {
        let half = VelocityQuad::new(&ROWS, [100, 0, 0], ARROW, 128, false);
        assert_eq!(half.a, [-80, 80]);
        assert_eq!(half.b, [35, -5]);
        let r = VelocityQuad::new(&ROWS, [0, 0, 50], ARROW, 256, true);
        assert_eq!(r.b, [10, -70]);
        assert_eq!(r.direction, [0, -65536, 0]);
    }

    #[test]
    fn a_segment_runs_from_its_origin_past_its_far_end() {
        let q = VelocityQuad::segment(&ROWS, [-300, 0, 0], ARROW, 0x80, 16, false);
        assert_eq!(q.direction, [-65536, 0, 0]);
        assert_eq!(q.a, [0, 316]);
        assert_eq!(q.b, [35, -5]);
        assert_eq!(q.corner(0, 0), [0, -35, 0]);
        assert_eq!(q.corner(0, 1), [0, 5, 0]);
        assert_eq!(q.corner(1, 0), [316, -35, 0]);
        assert_eq!(q.corner(1, 1), [316, 5, 0]);
        let r = VelocityQuad::segment(&ROWS, [0, 0, -100], ARROW, 0x100, 0, true);
        assert_eq!(r.b, [10, -70]);
        assert_eq!(r.direction, [0, 65536, 0]);
        assert_eq!(r.a, [0, 100]);
    }

    #[test]
    fn a_still_object_has_no_direction() {
        let q = VelocityQuad::new(&ROWS, [0, 0, 0], ARROW, 256, false);
        assert_eq!(q.direction, [0, 0, 0]);
        assert_eq!(q.corner(0, 0), [0, 0, 0]);
    }

    #[test]
    fn bobbing_needs_its_flags() {
        let c: [i16; 1024] = std::array::from_fn(|i| {
            ((i as f64 * core::f64::consts::TAU / 1024.0).cos() * 16384.0 + 0.5) as i16
        });
        assert_eq!(bob_offset(0x20, 3, 100, &c), 0);
        assert_eq!(bob_offset(0x1000, 3, 100, &c), 0);
        let want = i32::from(c[((3 * 0x39 + 100) * 16) & 0x3f0] >> 9);
        assert_eq!(bob_offset(0x800, 3, 100, &c), want);
        assert_eq!(bob_offset(0x1020, 3, 100, &c), want);
    }

    #[test]
    fn view_rows_from_the_client_axes() {
        let rows =
            EngineViewRows::from_camera_axes([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(rows.right, [16384, 0, 0]);
        assert_eq!(rows.up, [0, 0, 16384]);
        assert_eq!(rows.depth, [0, -16384, 0]);
    }

    #[test]
    fn the_rect_of_a_sprite() {
        let r = InstanceRect::from_sprite([320.0 / 512.0, 80.0 / 512.0], [0.5, 70.0 / 80.0]);
        assert_eq!(r, ARROW);
    }

    #[test]
    fn large_velocity_wraps_squared_length_without_overflow_panic() {
        let v = [32767, 32767, 10000];
        let (dir, len) = view_direction_and_length(&ROWS, v, false);
        let expected_sq = 32767u32 * 32767 + 10000u32 * 10000 + 32767u32 * 32767;
        assert!(expected_sq > i32::MAX as u32);
        let expected_len = mandrake_math::engine::isqrt(expected_sq) as i32;
        assert_eq!(len, expected_len);
        assert_eq!(
            dir,
            [
                (32767 << 16) / expected_len,
                (10000 << 16) / expected_len,
                (-32767 << 16) / expected_len,
            ]
        );
    }

    #[test]
    fn reflected_quad_handles_min_i16_rect_bounds() {
        let extreme = InstanceRect {
            top: i16::MIN,
            left: -10,
            right: 10,
            bottom: i16::MIN,
        };
        let q = VelocityQuad::new(&ROWS, [100, 0, 0], extreme, 256, true);
        assert_eq!(q.b, [i32::from(i16::MIN), i32::from(i16::MIN)]);
        let s = VelocityQuad::segment(&ROWS, [100, 0, 0], extreme, 256, 0, true);
        assert_eq!(s.b, [i32::from(i16::MIN), i32::from(i16::MIN)]);
    }
}
