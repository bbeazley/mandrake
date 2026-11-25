use crate::draw_site;
use crate::engine_move::trig_tables;
use crate::engine_rng::Unmodelled;
use mandrake_common::game_options;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormationDefinition {
    pub fields: [i16; 20],
}

impl FormationDefinition {
    pub const STATIC_DEFAULT: Self = Self {
        fields: [
            0x0004, 0x0280, 0x0008, 0x0280, 0x0004, 0x0400, 0x0004, 0x0400, 0x0280, 0x0300, 0x0180,
            0x0600, 0x0300, 0x4000, 0x5555, 0x0100, 0x0200, 0x0200, 0x0300, 0x0000,
        ],
    };

    pub fn from_tag(data: &[u8]) -> Option<Self> {
        if data.len() < 0x28 {
            return None;
        }
        let mut fields = [0i16; 20];
        for (k, f) in fields.iter_mut().enumerate() {
            *f = i16::from_be_bytes([data[2 * k], data[2 * k + 1]]);
        }
        Some(Self { fields })
    }

    fn at(&self, offset: usize) -> i32 {
        i32::from(self.fields[offset / 2])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormationGlobals {
    pub game_options_high: u16,
    pub gameplay_version: i32,
}

fn cos_sin(angle: i32) -> (i32, i32) {
    let t = trig_tables();
    let i = (((angle as u32).wrapping_add(0x20) & 0xffff) >> 6) as usize;
    (i32::from(t.cosine[i]), i32::from(t.sine[i]))
}

const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);

pub type Slot = ((i32, i32), u16);

pub fn build_formation(
    def: &FormationDefinition,
    globals: FormationGlobals,
    formation: i16,
    point: (i32, i32),
    facing: u16,
    count: i32,
) -> Result<Vec<Slot>, Unmodelled> {
    if count <= 0 {
        return Ok(Vec::new());
    }
    if count == 1 {
        return Ok(vec![(point, facing)]);
    }
    let f = i32::from(facing);
    let mut out = Vec::with_capacity(count as usize);
    match formation {
        0 | 10 | 11 | 13 => rows(def.at(0x00), def.at(0x02), point, f, count, &mut out),
        1 => rows(def.at(0x04), def.at(0x06), point, f, count, &mut out),
        2 => rows(def.at(0x08), def.at(0x0a), point, f, count, &mut out),
        3 => staggered(def, globals, point, f, count, &mut out),
        4 => {
            let mut side = crate::projectiles::isqrt(count as u32);
            if side.wrapping_mul(side) < count {
                side += 1;
            }
            rows(side, def.at(0x10), point, f, count, &mut out);
        }
        5 => scattered(def, point, f, count, &mut out),
        6 => arc(def, globals, def.at(0x1a), point, f, count, &mut out)?,
        7 => arc(def, globals, def.at(0x1c), point, f, count, &mut out)?,
        8 => wedge(def, globals, point, f, count, &mut out),
        9 => circle(def, point, f, count, &mut out),
        _ => {
            return Err(Unmodelled::new(
                draw_site::FORMATION_OUTSIDE_TABLE,
                format!("formation {formation} outside the shape table [0x85fa60]"),
            ));
        }
    }
    Ok(out)
}

fn rows(width: i32, spacing: i32, p: (i32, i32), facing: i32, count: i32, out: &mut Vec<Slot>) {
    let (c, s) = cos_sin(facing);
    let (mut row_x, mut row_y) = (0i32, 0i32);
    let mut left = count;
    while left > 0 {
        let n = width.min(left);
        let lat0 = -((n - 1).wrapping_mul(spacing) / 2);
        for j in 0..n.max(0) {
            let lat = lat0.wrapping_add(j.wrapping_mul(spacing));
            let x = (row_x.wrapping_sub(s.wrapping_mul(lat)) >> 14).wrapping_add(p.0);
            let y = (c.wrapping_mul(lat).wrapping_add(row_y) >> 14).wrapping_add(p.1);
            out.push(((x, y), facing as u16));
        }
        left -= width;
        row_x = row_x.wrapping_add(c.wrapping_mul(-spacing));
        row_y = row_y.wrapping_add(s.wrapping_mul(-spacing));
        if width <= 0 {
            break;
        }
    }
}

fn staggered(
    def: &FormationDefinition,
    g: FormationGlobals,
    p: (i32, i32),
    facing: i32,
    count: i32,
    out: &mut Vec<Slot>,
) {
    let (c, s) = cos_sin(facing);
    let width = def.at(0x0c);
    let spacing = def.at(0x0e);
    let half = spacing / 2;
    let mut row = 0i32;
    let mut left = count;
    while left > 0 {
        let capacity = if row & 1 != 0 { width + 1 } else { width };
        let n = capacity.min(left);
        let mut lat = (-half).wrapping_mul(n - 1);
        if row & 1 != 0
            && n & 1 == 0
            && !(g.game_options_high
                & mandrake_common::game_options::high(mandrake_common::game_options::TFL)
                != 0
                && n == 2)
        {
            lat = lat.wrapping_sub(half);
        }
        let back = row.wrapping_mul(spacing);
        let (bx, by) = (back.wrapping_mul(c), back.wrapping_mul(s));
        for _ in 0..n.max(0) {
            let x = (bx.wrapping_sub(s.wrapping_mul(lat)) >> 14).wrapping_add(p.0);
            let y = (c.wrapping_mul(lat).wrapping_add(by) >> 14).wrapping_add(p.1);
            out.push(((x, y), facing as u16));
            lat = lat.wrapping_add(spacing);
        }
        left -= capacity;
        row -= 1;
    }
}

fn scattered(
    def: &FormationDefinition,
    p: (i32, i32),
    facing: i32,
    count: i32,
    out: &mut Vec<Slot>,
) {
    let d = def.at(0x12);
    let mut seed = (p.0 ^ p.1) as u32;
    let size = (crate::projectiles::isqrt(count as u32) + 2).wrapping_mul(d);
    let min2 = d.wrapping_mul(d);
    let half = size >> 1;
    let (x0, y0) = (p.0.wrapping_sub(half), p.1.wrapping_sub(half));
    let step = |s: &mut u32| {
        *s = s.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
        *s
    };
    for i in 0..count {
        let mut attempt = 1i32;
        let point = loop {
            if i == 0 {
                break p;
            }
            let (x, y) = if size != 0 {
                let x = (step(&mut seed) % size as u32) as i32 + x0;
                let y = (step(&mut seed) % size as u32) as i32 + y0;
                (x, y)
            } else {
                let x = (step(&mut seed) as i32).wrapping_add(p.0);
                let y = (step(&mut seed) as i32).wrapping_add(p.1);
                (x, y)
            };
            let close = out.iter().any(|&((ox, oy), _)| {
                let (dx, dy) = (x.wrapping_sub(ox), y.wrapping_sub(oy));
                min2 > dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
            });
            if close && (attempt as i16) <= 0xb {
                attempt += 1;
                continue;
            }
            break (x, y);
        };
        out.push((point, facing as u16));
    }
}

fn arc(
    def: &FormationDefinition,
    g: FormationGlobals,
    arc: i32,
    p: (i32, i32),
    facing: i32,
    count: i32,
    out: &mut Vec<Slot>,
) -> Result<(), Unmodelled> {
    if g.gameplay_version <= game_options::GAMEPLAY_1_3 {
        return Err(Unmodelled::new(
            draw_site::ARC_FORMATION_OLD_GAMEPLAY,
            "arc formation for gameplay <= game_options::GAMEPLAY_1_3",
        ));
    }
    let a = i64::from(def.fields[0x16 / 2]);
    let b = i64::from(def.fields[0x18 / 2]);
    let last = i64::from(count - 1);
    let two_pi = TWO_PI as i64;
    let mut total = (b * last) << 16;
    let div = i64::from(arc) * two_pi;
    if div == 0 {
        return Err(Unmodelled::new(
            draw_site::ARC_FORMATION_DIVISION_BY_ZERO,
            "arc formation: division by zero",
        ));
    }
    let radius = (total / div).max(a);
    let div = two_pi * radius;
    if div == 0 {
        return Err(Unmodelled::new(
            draw_site::ARC_FORMATION_RADIUS_ZERO,
            "arc formation: radius 0",
        ));
    }
    total /= div;
    let (c, s) = cos_sin(facing);
    let cx = (((i64::from(c) * radius) >> 14) as i32).wrapping_add(p.0);
    let cy = (((i64::from(s) * radius) >> 14) as i32).wrapping_add(p.1);
    let start = facing.wrapping_sub((total >> 1) as i32);
    let mut acc = 0i64;
    for _ in 0..count {
        let angle = ((acc / last) as i32).wrapping_add(start);
        acc += total;
        let (ac, as_) = cos_sin(angle);
        let x = cx.wrapping_sub(((i64::from(ac) * radius) >> 14) as i32);
        let y = cy.wrapping_sub(((i64::from(as_) * radius) >> 14) as i32);
        out.push(((x, y), angle as u16));
    }
    Ok(())
}

fn wedge(
    def: &FormationDefinition,
    g: FormationGlobals,
    p: (i32, i32),
    facing: i32,
    count: i32,
    out: &mut Vec<Slot>,
) {
    let (c, s) = cos_sin(facing);
    let half = count / 2;
    let odd = count & 1 != 0;
    let (m, r) = if odd {
        (half + 1, 0)
    } else {
        (half, def.at(0x1e))
    };
    let (bx, mut by) = p;
    if g.gameplay_version <= game_options::GAMEPLAY_1_7_2 {
        by = by.wrapping_add(((m / 2).wrapping_mul(def.at(0x22)).wrapping_mul(s)) >> 14);
    }
    let f = facing as u16;
    for k in 0..m.max(0) {
        if k == 0 && odd {
            out.push(((bx, by), f));
            continue;
        }
        let back = (-k).wrapping_mul(def.at(0x22));
        let lat = k.wrapping_mul(def.at(0x20)).wrapping_add(r);
        let cb = back.wrapping_mul(c);
        let sb = back.wrapping_mul(s);
        let p1 = (
            (cb.wrapping_sub(lat.wrapping_mul(s)) >> 14).wrapping_add(bx),
            (sb.wrapping_add(c.wrapping_mul(lat)) >> 14).wrapping_add(by),
        );
        let p2 = (
            (lat.wrapping_mul(s).wrapping_add(cb) >> 14).wrapping_add(bx),
            (sb.wrapping_sub(c.wrapping_mul(lat)) >> 14).wrapping_add(by),
        );
        out.push((p1, f));
        out.push((p2, f));
    }
}

fn circle(def: &FormationDefinition, p: (i32, i32), facing: i32, count: i32, out: &mut Vec<Slot>) {
    let r = ((def.at(0x24).wrapping_mul(count)) as f32 / TWO_PI) as i32;
    for i in 0..count {
        let angle = (i.wrapping_mul(0x10000) / count).wrapping_add(facing);
        let (c, s) = cos_sin(angle);
        let x = (c.wrapping_mul(r) >> 14).wrapping_add(p.0);
        let y = (s.wrapping_mul(r) >> 14).wrapping_add(p.1);
        out.push(((x, y), angle as u16));
    }
}

pub fn formation_is_line(def: &FormationDefinition, formation: i16, count: i32) -> bool {
    let n = count as i16;
    match formation {
        0 => def.fields[0] >= n,
        1 => n <= def.fields[2],
        2 => n <= def.fields[4],
        3 => n <= def.fields[6],
        6 | 7 => true,
        _ => false,
    }
}

pub fn heap_sort<T: Copy>(v: &mut [T], compare: impl Fn(&T, &T) -> i32) {
    let n = v.len();
    if n <= 1 {
        return;
    }
    let sift = |v: &mut [T], mut i: usize, size: usize| {
        loop {
            let mut j = 2 * i;
            if j > size {
                break;
            }
            if size > j && compare(&v[j - 1], &v[j]) < 0 {
                j += 1;
            }
            if compare(&v[i - 1], &v[j - 1]) >= 0 {
                break;
            }
            v.swap(i - 1, j - 1);
            i = j;
        }
    };
    for l in (1..=n / 2).rev() {
        sift(v, l, n);
    }
    let mut last = n - 1;
    loop {
        v.swap(0, last);
        if last == 1 {
            break;
        }
        sift(v, 1, last);
        last -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: FormationGlobals = FormationGlobals {
        game_options_high: 0x200c,
        gameplay_version: 0x726,
    };

    #[test]
    fn formation_3_matches_the_engine() {
        let slots = build_formation(
            &FormationDefinition::STATIC_DEFAULT,
            G,
            3,
            (10626, 69823),
            46912,
            6,
        )
        .unwrap();
        let pts: Vec<_> = slots.iter().map(|s| s.0).collect();
        assert_eq!(
            pts,
            [
                (9125, 70150),
                (10125, 69932),
                (11126, 69713),
                (12126, 69495),
                (9843, 71041),
                (10844, 70823)
            ]
        );
    }

    #[test]
    fn heap_sort_sorts() {
        let mut v = [5, 3, 9, 1, 1, 7, 2];
        heap_sort(&mut v, |a, b| a - b);
        assert_eq!(v, [1, 1, 2, 3, 5, 7, 9]);
    }
}
