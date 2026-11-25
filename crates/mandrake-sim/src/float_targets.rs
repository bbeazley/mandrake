use crate::engine_move::{table_sin_cos, trig_tables};
use crate::particles::transition_cos;

#[derive(Clone, Copy)]
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.bytes(&v.to_le_bytes());
    }
}

const PI_BITS: u32 = 0x4049_0fdb;

fn model_hit_angle(angle: u16) -> f64 {
    const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
    const EPS: f32 = 1.525_878_9e-5;
    f64::from((f32::from(angle) * TWO_PI) * EPS)
}

pub fn engine_sin_cos_raw() -> u64 {
    let mut h = Fnv::default();
    for i in (0..1024).filter(|i| i & 0xff != 0) {
        let (s, c) = table_sin_cos(i);
        h.u64(s.to_bits());
        h.u64(c.to_bits());
    }
    h.0
}

pub fn engine_tables() -> u64 {
    let t = trig_tables();
    let mut h = Fnv::default();
    t.sine.iter().for_each(|&v| h.i16(v));
    t.cosine.iter().for_each(|&v| h.i16(v));
    t.tangent.iter().for_each(|&v| h.i32(v));
    h.0
}

pub fn particle_cos(stride: u32) -> (u64, u64) {
    let (mut raw, mut steps) = (Fnv::default(), Fnv::default());
    let mut bits = 0u32;
    loop {
        let c = transition_cos(f32::from_bits(bits));
        raw.u64(c.to_bits());
        steps.i32(((1.0 - c) * 65536.0) as i32 >> 1);
        steps.i32(((c + 1.0) * 65536.0) as i32 >> 1);
        if bits == PI_BITS {
            break;
        }
        bits = bits.saturating_add(stride).min(PI_BITS);
    }
    (raw.0, steps.0)
}

pub fn model_sin_cos_raw() -> u64 {
    let mut h = Fnv::default();
    for a in 0..=u16::MAX {
        let (s, c) = model_hit_angle(a).sin_cos();
        h.u64(s.to_bits());
        h.u64(c.to_bits());
    }
    h.0
}

pub fn model_rotation() -> u64 {
    let mut h = Fnv::default();
    for a in 0..=u16::MAX {
        mandrake_tag::model_hit::build_rotation_matrix(a, 0)
            .iter()
            .for_each(|&v| h.i16(v));
    }
    h.0
}

pub fn hashes(particle_stride: u32) -> Vec<(&'static str, u64)> {
    let (raw, steps) = particle_cos(particle_stride);
    vec![
        ("engine_move.sin_cos.raw", engine_sin_cos_raw()),
        ("engine_move.tables", engine_tables()),
        ("particles.cos.raw", raw),
        ("particles.cos.steps", steps),
        ("model_hit.sin_cos.raw", model_sin_cos_raw()),
        ("model_hit.rotation", model_rotation()),
    ]
}
