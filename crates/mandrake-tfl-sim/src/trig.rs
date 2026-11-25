use std::sync::OnceLock;

pub struct Trig {
    pub cosine: [i16; 1024],
    pub sine: [i16; 1024],
    pub tangent: [i32; 1024],
}

pub fn table_sin_cos(i: usize) -> (f64, f64) {
    let a = i as f64 * (1.0f64.atan() * 8.0) * 0.000_976_562_5;
    a.sin_cos()
}

fn fnv(words: impl IntoIterator<Item = u64>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for w in words {
        for x in w.to_le_bytes() {
            h = (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

pub fn float_target_hashes() -> Vec<(&'static str, u64)> {
    let t = trig();
    vec![
        (
            "tfl.atan1.raw",
            fnv([
                1.0f64.atan().to_bits(),
                crate::commands::full_turn().to_bits(),
            ]),
        ),
        (
            "tfl.trig.sin_cos.raw",
            fnv((0..1024).filter(|i| i & 0xff != 0).flat_map(|i| {
                let (s, c) = table_sin_cos(i);
                [s.to_bits(), c.to_bits()]
            })),
        ),
        (
            "tfl.trig.tables",
            fnv(t
                .sine
                .iter()
                .chain(&t.cosine)
                .map(|&v| v as u16 as u64)
                .chain(t.tangent.iter().map(|&v| v as u32 as u64))),
        ),
    ]
}

pub fn trig() -> &'static Trig {
    static T: OnceLock<Trig> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = Trig {
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
            t.tangent[i] = if c as i16 == 0 {
                i32::MIN
            } else {
                (i32::from(s as i16) << 14) / i32::from(c as i16)
            };
        }
        t
    })
}

pub fn index(angle: i32) -> usize {
    ((angle + 0x20) >> 6 & 0x3ff) as usize
}

pub fn arctangent(dx: i32, dy: i32) -> u16 {
    if dx == 0 {
        return if dy >= 0 { 0x4000 } else { 0xc000 };
    }
    let tan = dy.wrapping_shl(14).wrapping_div(dx);
    if tan == 0 {
        return if dx >= 0 { 0 } else { 0x8000 };
    }
    let t = &trig().tangent;
    let mut si: u16 = if dy > 0 { 1 } else { 0x201 };
    if tan < 0 {
        si = si.wrapping_add(0x100);
    }
    let at = |k: u16| t[usize::from(k) & 0x3ff];
    let mut prev = tan.wrapping_sub(at(si.wrapping_sub(1)));
    let mut left = 0xff;
    loop {
        let cur = tan.wrapping_sub(at(si));
        if (prev <= 0 && cur >= 0) || (prev >= 0 && cur <= 0) {
            if prev.unsigned_abs() < cur.unsigned_abs() {
                si = si.wrapping_sub(1);
            }
            return si.wrapping_shl(6);
        }
        left -= 1;
        si = si.wrapping_add(1);
        prev = cur;
        if left < 1 {
            return si.wrapping_shl(6);
        }
    }
}

pub fn rotate(p: (i32, i32), centre: (i32, i32), angle: i32) -> (i32, i32) {
    let k = index(angle);
    let (c, s) = (i32::from(trig().cosine[k]), i32::from(trig().sine[k]));
    let (dx, dy) = (p.0 - centre.0, p.1 - centre.1);
    (
        ((s * dy + c * dx) >> 14) + centre.0,
        ((c * dy - s * dx) >> 14) + centre.1,
    )
}

pub fn angle_difference(a: i32, b: i32) -> u32 {
    let d = (a.wrapping_sub(b) & 0xffff) as u32;
    if d > 0x7fff { 0x10000 - d } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_axes_and_the_arctangent() {
        let t = trig();
        assert_eq!((t.cosine[0], t.sine[0]), (0x4000, 0));
        assert_eq!((t.cosine[0x200], t.sine[0x300]), (-0x4000, -0x4000));
        assert_eq!(arctangent(0, 5), 0x4000);
        assert_eq!(arctangent(0, -5), 0xc000);
        assert_eq!(arctangent(5, 0), 0);
        assert_eq!(arctangent(-5, 0), 0x8000);
        assert_eq!(arctangent(100, 100), 0x2000);
        assert_eq!(arctangent(-100, 100), 0x6000);
        assert_eq!(arctangent(-100, -100), 0xa000);
        assert_eq!(arctangent(100, -100), 0xe000);
    }
}
