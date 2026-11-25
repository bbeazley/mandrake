pub const MEDIA_REFLECTION_TINT_VALUE: usize = 0x58;
pub const MEDIA_REFLECTION_TINT_DATA: usize = 0x5a;
pub const MEDIA_RIPPLE_AMPLITUDE: usize = 0x6c;

pub const SUBMERGED_COPY_TRANSPARENCY: u16 = 0xe000;

pub fn surface_opacity(transparency: u16) -> f32 {
    ((0xffff - u32::from(transparency)) >> 8) as f32 / 255.0
}
pub const MEDIA_RIPPLE_FREQUENCY: usize = 0x72;

pub const MEDIA_WOBBLE_AMPLITUDE: usize = 0x68;
pub const MEDIA_WOBBLE_FREQUENCY: usize = 0x6e;
pub const MESH_HEADER_FLAGS: usize = 0x4c;
pub const MESH_FLAG_MODEL_CELLS_NOT_MEDIA: u32 = 0x1000_0000;
pub const CELL_FLAG_MEDIA_VERTEX: u16 = 0x0100;
pub const CELL_FLAG_MEDIA_VERTEX_SUBMERGED: u16 = 0x0400;

pub fn media_sine_index(phase: i32, frequency: i16) -> usize {
    let e = phase.wrapping_mul(i32::from(frequency)).wrapping_add(0x20);
    ((e as u32 & 0xffff) >> 6) as usize
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaWobble {
    pub amplitude: [i16; 3],
    pub frequency: [i16; 3],
}

impl MediaWobble {
    pub fn from_media_bytes(medi: &[u8]) -> Self {
        let be16 = |o: usize| {
            medi.get(o..o + 2)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
        };
        Self {
            amplitude: [0, 2, 4].map(|i| be16(MEDIA_WOBBLE_AMPLITUDE + i)),
            frequency: [0, 2, 4].map(|i| be16(MEDIA_WOBBLE_FREQUENCY + i)),
        }
    }

    pub fn offset(&self, col: i32, row: i32, local_time: i32, sine: &[i16; 1024]) -> [i32; 3] {
        let t = local_time
            .wrapping_add(col.wrapping_mul(0x179))
            .wrapping_add(row.wrapping_mul(0xd3));
        let mut out = [0; 3];
        for (axis, o) in out.iter_mut().enumerate() {
            let a = self.amplitude[axis];
            if a != 0 {
                let s = sine[media_sine_index(t, self.frequency[axis])];
                *o = (i32::from(a) * i32::from(s)) >> 14;
            }
        }
        out
    }
}

pub fn is_media_height_vertex(flags: u16, model_index: i16, mesh_flags: u32) -> bool {
    let media_branch = model_index == -1 || mesh_flags & MESH_FLAG_MODEL_CELLS_NOT_MEDIA == 0;
    media_branch && flags & CELL_FLAG_MEDIA_VERTEX != 0
}

pub fn is_media_vertex(flags: u16, model_index: i16, mesh_flags: u32) -> bool {
    is_media_height_vertex(flags, model_index, mesh_flags)
        && flags & CELL_FLAG_MEDIA_VERTEX_SUBMERGED != 0
}

pub fn mesh_vertex_height(
    physical_height: i16,
    media_height: i16,
    flags: u16,
    model_index: i16,
    mesh_flags: u32,
) -> i16 {
    if is_media_height_vertex(flags, model_index, mesh_flags) {
        media_height
    } else {
        physical_height
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MediaSurface {
    pub mesh_flags: u32,
    pub wobble: MediaWobble,
}

pub fn reflection_shading_slot(transfer_mode: u8) -> u8 {
    match transfer_mode {
        1 => 5,
        2 => 6,
        _ => 3,
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EngineViewAxes {
    origin: [i32; 3],
    up: [i16; 3],
    depth: [i16; 3],
    vertical_scale: i16,
    center_y: i16,
}

#[cfg(test)]
fn software_clip_row(view: &EngineViewAxes, x: i32, y: i32, media_height: i16) -> (i32, i32) {
    let d = [
        x.wrapping_sub(view.origin[0]),
        y.wrapping_sub(view.origin[1]),
        i32::from(media_height).wrapping_sub(view.origin[2]),
    ];
    let dot = |axis: [i16; 3]| -> i32 {
        (i32::from(axis[0]).wrapping_mul(d[0]))
            .wrapping_add(i32::from(axis[1]).wrapping_mul(d[1]))
            .wrapping_add(i32::from(axis[2]).wrapping_mul(d[2]))
    };
    let u = dot(view.up) >> 14;
    let depth = dot(view.depth) >> 14;
    let scaled = i32::from(view.vertical_scale).wrapping_mul(u);
    let row = if depth == 0 {
        i32::from(view.center_y)
    } else {
        i32::from(view.center_y).wrapping_add(scaled.wrapping_div(depth))
    };
    (u, row)
}

#[cfg(test)]
fn kept_view_up_range(u: i32, reflection: bool) -> (i32, i32) {
    if reflection {
        (u, -0x8000)
    } else {
        (0x7fff, u)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShadingSlot {
    pub kind: i16,
    pub value: i16,
    pub data: [u16; 4],
}

impl ShadingSlot {
    pub fn media_reflection(media: &[u8]) -> Option<Self> {
        let be16 = |o: usize| {
            media
                .get(o..o + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
        };
        let d = MEDIA_REFLECTION_TINT_DATA;
        Some(Self {
            kind: 1,
            value: be16(MEDIA_REFLECTION_TINT_VALUE)? as i16,
            data: [be16(d)?, be16(d + 2)?, be16(d + 4)?, be16(d + 6)?],
        })
    }
}

fn tint_byte(x: f32) -> u8 {
    let i = if x.is_nan() || x >= 2_147_483_648.0 || x < -2_147_483_648.0 {
        i32::MIN
    } else {
        x as i32
    };
    if i < 0 {
        0
    } else if i > 0xff {
        0xff
    } else {
        i as u8
    }
}

pub fn build_tinting_shader(
    slot: ShadingSlot,
    fraction: i16,
    color: [u16; 3],
    slot3: ShadingSlot,
) -> [u8; 8] {
    const S16: f32 = 1.0 / 65536.0;
    const S8: f32 = 1.0 / 256.0;
    let (lr, lg, lb) = (0.4_f32, 0.6_f32, 0.2_f32);
    let (a0, a1, a2, v9) = if slot.kind == 0 || slot.kind == 2 {
        (
            f32::from(slot.data[0]) * S16,
            f32::from(slot.data[1]) * S16,
            f32::from(slot.data[2]) * S16,
            f32::from(slot.value) * S8,
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    let tinted = match slot.kind {
        1 => Some(slot),
        2 => Some(ShadingSlot { kind: 2, ..slot3 }),
        _ => None,
    };
    let (v, r, g, b, lum, one_minus_v) = match tinted {
        Some(t) => {
            let v = f32::from(t.value) * S8;
            let r = f32::from(t.data[0]) * S16;
            let g = f32::from(t.data[1]) * S16;
            let b = f32::from(t.data[2]) * S16;
            let lum = (g * lg + r * lr) + b * lb;
            (v, r, g, b, lum, 1.0 - v)
        }
        None => (0.0, 0.0, 0.0, 0.0, 0.0, 1.0),
    };
    let p = S8 * f32::from(fraction);
    let c0 = f32::from(color[0]) * S16;
    let c1 = f32::from(color[1]) * S16;
    let c2 = f32::from(color[2]) * S16;
    let lum_a = (lr * a0 + lg * a1) + lb * a2;
    let one_minus_p = 1.0 - p;
    let pv = one_minus_p * v;
    let vp = v * p;
    let pv_bar = one_minus_p * one_minus_v;
    let lum_vp = lum * vp;
    let v9_term = v9 * pv_bar;
    let k = (pv_bar * (1.0 - v9)) * 255.0;
    let q = p * (lum_a * (v9 * one_minus_v) + one_minus_v * (1.0 - v9));
    let kp = r * pv + c0 * lum_vp;
    let kq = c1 * lum_vp + g * pv;
    let kr = pv * b + lum_vp * c2;
    let x = c0 * q + a0 * v9_term;
    let y = c1 * q + a1 * v9_term;
    let z = q * c2 + a2 * v9_term;
    [
        tint_byte(k),
        tint_byte(kp * 255.0),
        tint_byte(kq * 255.0),
        tint_byte(kr * 255.0),
        tint_byte(x * 255.0),
        tint_byte(y * 255.0),
        tint_byte(255.0 * z),
        0,
    ]
}

pub fn slot0_tint(fraction: i16, color: [u16; 3]) -> [u8; 8] {
    let slot = ShadingSlot {
        kind: -1,
        ..ShadingSlot::default()
    };
    build_tinting_shader(slot, fraction, color, slot)
}

pub const ENGINE_SHADING_SLOTS: [ShadingSlot; 8] = [
    ShadingSlot {
        kind: -1,
        value: 0,
        data: [0; 4],
    },
    ShadingSlot {
        kind: 0,
        value: 0xe0,
        data: [0xbae0, 0xca3c, 0xbd6f, 0],
    },
    ShadingSlot {
        kind: 0,
        value: 0xc0,
        data: [0xa3d6, 0x4a3d, 0xbd6f, 0],
    },
    ShadingSlot {
        kind: 1,
        value: 0x80,
        data: [0, 0, 0x8000, 0],
    },
    ShadingSlot {
        kind: 0,
        value: 0xc0,
        data: [0x4242, 0x3a3a, 0x3333, 0],
    },
    ShadingSlot {
        kind: 2,
        value: 0xe0,
        data: [0xbae0, 0xca3c, 0xbd6f, 0],
    },
    ShadingSlot {
        kind: 2,
        value: 0xc0,
        data: [0xa3d6, 0x4a3d, 0xbd6f, 0],
    },
    ShadingSlot {
        kind: 3,
        value: 0,
        data: [0; 4],
    },
];

pub fn slot_tint(slot: u8, fraction: i16, color: [u16; 3], slot3: ShadingSlot) -> [u8; 8] {
    let record = ENGINE_SHADING_SLOTS[usize::from(slot & 7)];
    let record = if slot & 7 == 3 { slot3 } else { record };
    build_tinting_shader(record, fraction, color, slot3)
}

pub const SLOT0_IDENTITY: [u8; 8] = [0xff, 0, 0, 0, 0, 0, 0, 0];

pub fn tint_factors(tint: [u8; 8]) -> ([f32; 4], [f32; 4]) {
    let f = |b: u8| f32::from(b) / 255.0;
    (
        [f(tint[0]), f(tint[1]), f(tint[2]), f(tint[3])],
        [f(tint[4]), f(tint[5]), f(tint[6]), 0.0],
    )
}

pub fn reflection_ripple(
    x: i32,
    y: i32,
    local_time: i32,
    amplitude: i16,
    frequency: i16,
    sine: &[i16; 1024],
) -> i32 {
    MediaWobble {
        amplitude: [0, 0, amplitude],
        frequency: [0, 0, frequency],
    }
    .offset(x >> 9, y >> 9, local_time, sine)[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_paralysis_slot_is_purple() {
        let none = ShadingSlot::default();
        assert_eq!(
            slot_tint(2, 0, [0; 3], none),
            [63, 0, 0, 0, 122, 55, 141, 0]
        );
        assert_eq!(
            slot_tint(0, 0x40, [0x8000; 3], none),
            slot0_tint(0x40, [0x8000; 3])
        );
    }

    #[test]
    fn slot0_tint_is_the_engines_table() {
        assert_eq!(
            slot0_tint(0xd9, [10093, 21654, 22937]),
            [0x26, 0, 0, 0, 0x21, 0x47, 0x4b, 0]
        );
        assert_eq!(
            slot0_tint(0x65, [0x322c, 0x3d70, 0x3126]),
            [0x9a, 0, 0, 0, 0x13, 0x18, 0x13, 0]
        );
        assert_eq!(
            slot0_tint(0xc0, [10086, 16934, 17694]),
            [0x3f, 0, 0, 0, 0x1d, 0x31, 0x33, 0]
        );
        assert_eq!(slot0_tint(0, [0xffff, 0x8000, 7]), SLOT0_IDENTITY);
    }

    #[test]
    fn wate_reflection_darkens_to_a_quarter() {
        let slot = ShadingSlot {
            kind: 1,
            value: 0xc0,
            data: [0; 4],
        };
        assert_eq!(
            build_tinting_shader(slot, 0, [0; 3], ShadingSlot::default()),
            [63, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn default_slot_is_identity_and_colorizes_by_fraction() {
        let none = ShadingSlot::default();
        assert_eq!(
            build_tinting_shader(none, 0, [0; 3], none),
            [255, 0, 0, 0, 0, 0, 0, 0]
        );
        let t = build_tinting_shader(none, 0x80, [0xffff, 0x8000, 0], none);
        assert_eq!(t[0], 127);
        assert_eq!(&t[4..], &[127, 63, 0, 0]);
    }

    #[test]
    fn tinted_media_adds_its_color_by_value() {
        let slot = ShadingSlot {
            kind: 1,
            value: 0x80,
            data: [0xffff, 0x8000, 0, 0],
        };
        let t = build_tinting_shader(slot, 0, [0; 3], ShadingSlot::default());
        assert_eq!(t, [127, 127, 63, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn reflection_slot_follows_transfer_mode() {
        assert_eq!(reflection_shading_slot(0), 3);
        assert_eq!(reflection_shading_slot(1), 5);
        assert_eq!(reflection_shading_slot(2), 6);
        assert_eq!(reflection_shading_slot(7), 3);
    }

    #[test]
    fn media_reflection_slot_from_tag_bytes() {
        let mut m = vec![0u8; 0x100];
        m[0x58..0x5a].copy_from_slice(&0x00c0u16.to_be_bytes());
        m[0x5a..0x5c].copy_from_slice(&0x1234u16.to_be_bytes());
        let s = ShadingSlot::media_reflection(&m).unwrap();
        assert_eq!(s.kind, 1);
        assert_eq!(s.value, 0xc0);
        assert_eq!(s.data, [0x1234, 0, 0, 0]);
    }

    #[test]
    fn clip_row_is_the_projected_media_surface() {
        let c = 0x2d41;
        let view = EngineViewAxes {
            origin: [0, 0, 1000],
            up: [c, 0, c],
            depth: [-c, 0, c],
            vertical_scale: 512,
            center_y: 384,
        };
        let (u, row) = software_clip_row(&view, 1000, 0, 0);
        assert_eq!(u, 0);
        assert_eq!(row, 384);
        let (u, row) = software_clip_row(&view, 800, 0, 0);
        assert_eq!(u, (-200 * c as i32) >> 14);
        let depth = (-(c as i32) * 800 + (c as i32) * -1000) >> 14;
        assert_eq!(row, 384 + (512 * u) / depth);
        assert!(row > 384, "nearer surface points are lower on the screen");
        assert_eq!(kept_view_up_range(u, false), (0x7fff, u));
        assert_eq!(kept_view_up_range(u, true), (u, -0x8000));
    }

    #[test]
    fn wate_wobble_from_tag_bytes() {
        let mut medi = vec![0u8; 256];
        medi[0x68..0x74].copy_from_slice(&[
            0x00, 0x33, 0x00, 0x33, 0x00, 0x66, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00,
        ]);
        let w = MediaWobble::from_media_bytes(&medi);
        assert_eq!(w.amplitude, [0x33, 0x33, 0x66]);
        assert_eq!(w.frequency, [0x100, 0x100, 0x200]);
        assert_eq!(MediaWobble::from_media_bytes(&[]), MediaWobble::default());
    }

    #[test]
    fn wobble_indexes_the_sine_table_per_vertex_and_tick() {
        let mut sine = [0i16; 1024];
        for (i, s) in sine.iter_mut().enumerate() {
            *s = i as i16;
        }
        let w = MediaWobble {
            amplitude: [0x4000, 0x4000, 0x4000],
            frequency: [0x100, 0x100, 0x200],
        };
        assert_eq!(w.offset(0, 0, 0, &sine), [0, 0, 0]);
        assert_eq!(w.offset(0, 0, 1, &sine), [4, 4, 8]);
        assert_eq!(media_sine_index(0x31f, 0x100), 124);
        assert_eq!(w.offset(1, 2, 0, &sine)[0], 124);
        assert_eq!(w.offset(1, 2, 0, &sine)[2], 248);
        assert_eq!(w.offset(3, 5, 7, &sine), w.offset(3, 5, 7 + 256, &sine));
        assert_eq!(
            w.offset(3, 5, 7, &sine)[2],
            w.offset(3, 5, 7 + 128, &sine)[2]
        );
        let mut half = [0x4000i16; 1024];
        half[0] = -0x4000;
        let wate = MediaWobble {
            amplitude: [0x33, 0, 0x66],
            frequency: [0x100, 0x100, 0x200],
        };
        assert_eq!(wate.offset(0, 0, 0, &half), [-0x33, 0, -0x66]);
        assert_eq!(wate.offset(0, 0, 1, &half), [0x33, 0, 0x66]);
    }

    #[test]
    fn media_vertices_need_both_flags() {
        assert!(is_media_vertex(0x3d22, -1, 0x0121_0000));
        assert!(!is_media_vertex(0x1900, -1, 0x0121_0000));
        assert!(!is_media_vertex(0x2100, -1, 0x0121_0000));
        assert!(!is_media_vertex(0x0400, -1, 0));
        assert!(is_media_vertex(0x0500, 3, 0));
        assert!(!is_media_vertex(0x0500, 3, MESH_FLAG_MODEL_CELLS_NOT_MEDIA));
        assert!(is_media_vertex(0x0500, -1, MESH_FLAG_MODEL_CELLS_NOT_MEDIA));
        assert_eq!(mesh_vertex_height(-896, -512, 0x3d22, -1, 0), -512);
        assert!(is_media_height_vertex(0x3160, -1, 0x0121_0000));
        assert_eq!(mesh_vertex_height(-408, -512, 0x3160, -1, 0), -512);
        assert_eq!(mesh_vertex_height(-408, -512, 0x2166, -1, 0), -512);
        assert!(!is_media_height_vertex(
            0x0100,
            3,
            MESH_FLAG_MODEL_CELLS_NOT_MEDIA
        ));
        assert_eq!(mesh_vertex_height(-408, -512, 0x1c00, -1, 0), -408);
    }

    #[test]
    fn ripple_indexes_the_sine_table_by_cell_and_time() {
        let mut sine = [0i16; 1024];
        for (i, s) in sine.iter_mut().enumerate() {
            *s = i as i16;
        }
        assert_eq!(
            reflection_ripple(512 + 5, 1024 + 7, 3, 102, 2, &sine),
            (102 * 25) >> 14
        );
        let mut sine = [0x4000i16; 1024];
        sine[25] = -0x4000;
        assert_eq!(reflection_ripple(517, 1031, 3, 102, 2, &sine), -102);
        assert_eq!(reflection_ripple(517, 1031, 3, 0, 2, &sine), 0);
    }
}
