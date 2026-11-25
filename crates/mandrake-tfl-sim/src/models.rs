use mandrake_common::FourCC;

use crate::draw_site;
use crate::{Stop, TflSim};

pub const MAX_MODELS: usize = 0x80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInstance {
    pub identifier: i16,
    pub position: [i32; 3],
    pub a: [i16; 3],
    pub b: [i16; 3],
    pub mode: FourCC,
    pub permutation: i16,
    pub next_permutation: i16,
    pub transition: u16,
    pub yaw: u16,
    pub pitch: u16,
    pub fade: u16,
}

type SurfaceHit = ([i16; 3], [i16; 3]);

fn cross(a: [i16; 3], b: [i16; 3]) -> [i16; 3] {
    let (a0, a1, a2) = (i32::from(b[0]), i32::from(b[1]), i32::from(b[2]));
    let (b0, b1, b2) = (i32::from(a[0]), i32::from(a[1]), i32::from(a[2]));
    [
        ((b2 * a1 - b1 * a2) >> 14) as i16,
        ((b0 * a2 - a0 * b2) >> 14) as i16,
        ((a0 * b1 - b0 * a1) >> 14) as i16,
    ]
}

fn transform(m: &[i16; 9], p: [i32; 3], o: [i32; 3]) -> [i16; 3] {
    let d = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
    let m = m.map(i32::from);
    [
        ((m[6] * d[2] + m[3] * d[1] + m[0] * d[0]) >> 10) as i16,
        ((m[7] * d[2] + m[4] * d[1] + m[1] * d[0]) >> 10) as i16,
        ((m[8] * d[2] + m[5] * d[1] + m[2] * d[0]) >> 10) as i16,
    ]
}

fn identity(m: &[i16; 9]) -> bool {
    m[0] == 0x400 && m[4] == 0x400 && m[8] == 0x400
}

fn sin_cos(angle: u16, float_pitch: bool) -> (i16, i16) {
    match angle {
        0x0000 => (0, 0x4000),
        0x4000 => (0x4000, 0),
        0x8000 => (0, -0x4000),
        0xc000 => (-0x4000, 0),
        _ => {
            let mut a = f64::from(angle) * crate::commands::full_turn() * (1.0 / 65536.0);
            if float_pitch {
                a = f64::from(a as f32);
            }
            let (s, c) = a.sin_cos();
            ((s * 16384.0).trunc() as i16, (c * 16384.0).trunc() as i16)
        }
    }
}

pub(crate) fn angle_vectors(yaw: u16, pitch: u16) -> ([i16; 3], [i16; 3]) {
    let (s1, c1) = sin_cos(yaw, false);
    let (s2, c2) = sin_cos(pitch, true);
    let (s1, c1, s2, c2) = (i32::from(s1), i32::from(c1), i32::from(s2), i32::from(c2));
    (
        [s1 as i16, -c1 as i16, 0],
        [
            (-(c2 * c1) >> 14) as i16,
            (-(c2 * s1) >> 14) as i16,
            -s2 as i16,
        ],
    )
}

impl TflSim {
    pub(crate) fn add_models(&mut self) -> Result<(), Stop> {
        let palettes = self.data.mesh.palettes.clone();
        let markers = self.data.mesh.markers.clone();
        let mut model = 0i16;
        for m in markers.iter().filter(|m| m.marker_type == 6) {
            let Some(p) = palettes
                .iter()
                .filter(|p| p.marker_type == 6)
                .nth(m.palette_index.max(0) as usize)
            else {
                continue;
            };
            let (a, b) = angle_vectors(m.yaw as u16, m.pitch as u16);
            let (a0, a1, a2) = (i32::from(b[0]), i32::from(b[1]), i32::from(b[2]));
            let (b0, b1, b2) = (i32::from(a[0]), i32::from(a[1]), i32::from(a[2]));
            let c = [
                ((b2 * a1 - b1 * a2) >> 14) as i16,
                ((b0 * a2 - a0 * b2) >> 14) as i16,
                ((a0 * b1 - b0 * a1) >> 14) as i16,
            ];
            let mut mat = [0i32; 9];
            for k in 0..3 {
                mat[3 * k] = i32::from(a[k] >> 4);
                mat[3 * k + 1] = i32::from(c[k] >> 4);
                mat[3 * k + 2] = i32::from(b[k] >> 4);
            }
            let ids: Vec<i16> = self.model_instances.iter().map(|m| m.identifier).collect();
            let identifier = match self.model_pool.allocate(|k| k < ids.len()) {
                Some((k, salt)) => self
                    .model_pool
                    .set_identifier(k, m.marker_id, &ids)
                    .unwrap_or(salt),
                None => {
                    return Err(Stop::new(draw_site::ADD_MODELS, "more than 128 models"));
                }
            };
            let permutation = i16::from_be_bytes([m.type_data[4], m.type_data[5]]);
            self.model_instances.push(ModelInstance {
                identifier,
                position: [m.x, m.y, m.z],
                a,
                b,
                mode: p.tag_id,
                permutation,
                next_permutation: permutation,
                transition: 0,
                yaw: m.yaw as u16,
                pitch: m.pitch as u16,
                fade: 0,
            });
            let mode = self.data.def_arc(b"mode", p.tag_id)?;
            let geom_id = mode.tag(4).ok_or_else(|| {
                Stop::new(draw_site::ADD_MODELS, format!("mode {}: no geom", p.tag_id))
            })?;
            let geom = self.data.def_arc(b"geom", geom_id)?;
            let centre = [
                i32::from(geom.i16(0x10)),
                i32::from(geom.i16(0x12)),
                i32::from(geom.i16(0x14)),
            ];
            let count = mode.i16(0x0e).max(0) as usize;
            let at = 0x40 + mode.u32(0x28) as usize;
            let identity = mat[0] == 0x400 && mat[4] == 0x400 && mat[8] == 0x400;
            for k in 0..count {
                let e = at + 8 * k;
                let (ex, ey, flags) = (mode.i16(e), mode.i16(e + 2), mode.i16(e + 4));
                let p3 = [
                    i32::from((i32::from(ex) * 0x400 + 0x200) as i16),
                    i32::from((i32::from(ey) * 0x400 + 0x200) as i16),
                    0,
                ];
                let local: [i32; 3] = if identity {
                    [
                        i32::from((p3[0] - centre[0]) as i16),
                        i32::from((p3[1] - centre[1]) as i16),
                        i32::from((p3[2] - centre[2]) as i16),
                    ]
                } else {
                    let d = [p3[0] - centre[0], p3[1] - centre[1], p3[2] - centre[2]];
                    [
                        i32::from(((mat[6] * d[2] + mat[3] * d[1] + mat[0] * d[0]) >> 10) as i16),
                        i32::from(((mat[7] * d[2] + mat[4] * d[1] + mat[1] * d[0]) >> 10) as i16),
                        i32::from(((mat[8] * d[2] + mat[5] * d[1] + mat[2] * d[0]) >> 10) as i16),
                    ]
                };
                let wx = local[0] + m.x + centre[0];
                let wy = local[1] + m.y + centre[1];
                let cell = self.cell_index(wx, wy);
                let c = &mut self.cells[cell];
                if flags & 1 != 0 {
                    c.flags |= 0x40;
                }
                if flags & 6 != 0 {
                    let t = ((flags as u8 & 4) | 0x38) >> 2;
                    c.byte_5 = (t << 4) | t;
                }
                c.flags |= 0x80;
                let head = self.cell_heads[cell];
                if head < 0 {
                    self.cell_heads[cell] = -2 - model;
                } else {
                    let mut o = head as usize;
                    while self.objects[o].next_in_cell >= 0 {
                        o = self.objects[o].next_in_cell as usize;
                    }
                    self.objects[o].next_in_cell = -2 - model;
                }
            }
            model += 1;
        }
        Ok(())
    }
}

impl TflSim {
    fn model_matrices(m: &ModelInstance) -> ([i16; 9], [i16; 9]) {
        let c = cross(m.a, m.b);
        let mut mat = [0i16; 9];
        let mut inv = [0i16; 9];
        for k in 0..3 {
            mat[3 * k] = m.a[k] >> 4;
            mat[3 * k + 1] = c[k] >> 4;
            mat[3 * k + 2] = m.b[k] >> 4;
            inv[k] = m.a[k] >> 4;
            inv[3 + k] = c[k] >> 4;
            inv[6 + k] = m.b[k] >> 4;
        }
        (mat, inv)
    }

    pub(crate) fn model_collision(
        &mut self,
        from: [i32; 3],
        to: &mut [i32; 3],
        radius: i16,
    ) -> Result<Option<[i16; 3]>, Stop> {
        let m1 = self.model_at(from[0] >> 10, from[1] >> 10);
        if let Some(m) = m1
            && let Some(n) = self.model_hit(m as usize, from, to, radius)?
        {
            return Ok(Some(n));
        }
        let m2 = self.model_at(to[0] >> 10, to[1] >> 10);
        if let Some(m) = m2
            && m2 != m1
        {
            return self.model_hit(m as usize, from, to, radius);
        }
        Ok(None)
    }

    fn model_hit(
        &mut self,
        k: usize,
        from: [i32; 3],
        to: &mut [i32; 3],
        radius: i16,
    ) -> Result<Option<[i16; 3]>, Stop> {
        let inst = self
            .model_instances
            .get(k)
            .cloned()
            .ok_or_else(|| Stop::new(draw_site::MODEL_HIT, format!("model {k} not made")))?;
        let mode = self.data.def_arc(b"mode", inst.mode)?;
        let geom_id = mode.tag(4).ok_or_else(|| {
            Stop::new(draw_site::MODEL_HIT, format!("mode {}: no geom", inst.mode))
        })?;
        let geom = self.data.def_arc(b"geom", geom_id)?;
        let centre = [geom.i16(0x10), geom.i16(0x12), geom.i16(0x14)];
        let c32 = centre.map(i32::from);
        let (mat, inv) = Self::model_matrices(&inst);
        let local = |p: [i32; 3]| -> [i16; 3] {
            let o = [
                inst.position[0] + c32[0],
                inst.position[1] + c32[1],
                inst.position[2] + c32[2],
            ];
            let mut v = if identity(&inv) {
                [
                    (p[0] as i16).wrapping_sub(o[0] as i16),
                    (p[1] as i16).wrapping_sub(o[1] as i16),
                    (p[2] as i16).wrapping_sub(o[2] as i16),
                ]
            } else {
                transform(&inv, p, o)
            };
            for (k, c) in v.iter_mut().enumerate() {
                *c = c.wrapping_add(centre[k]);
            }
            v
        };
        let end = local(*to);
        let start = local(from);
        let Some((s, mut hit)) = self.model_surface_hit(&geom, start, end)? else {
            return Ok(None);
        };
        let n = [s[0], s[1], s[2]].map(i32::from);
        let m = mat.map(i32::from);
        let normal = [
            ((n[1] * m[3] + n[0] * m[0] + n[2] * m[6]) >> 10) as i16,
            ((n[1] * m[4] + n[0] * m[1] + n[2] * m[7]) >> 10) as i16,
            ((n[1] * m[5] + n[0] * m[2] + n[2] * m[8]) >> 10) as i16,
        ];
        if normal[2] < 0x3f38 {
            let r = i32::from(radius);
            for k in 0..3 {
                hit[k] = hit[k].wrapping_add(((n[k] * r) >> 14) as i16);
            }
        }
        let d = if identity(&mat) {
            [
                hit[0].wrapping_sub(centre[0]),
                hit[1].wrapping_sub(centre[1]),
                hit[2].wrapping_sub(centre[2]),
            ]
        } else {
            transform(&mat, hit.map(i32::from), c32)
        };
        for k in 0..3 {
            to[k] = i32::from(d[k]) + inst.position[k] + c32[k];
        }
        Ok(Some(normal))
    }

    fn model_surface_hit(
        &self,
        geom: &crate::data::Def,
        start: [i16; 3],
        end: [i16; 3],
    ) -> Result<Option<SurfaceHit>, Stop> {
        const HEADER: usize = 0x60;
        let count = geom.i16(0x0c).max(0) as usize;
        let surfaces = HEADER + geom.u32(0x34) as usize;
        let vertices = HEADER + geom.u32(0x28) as usize;
        let dot = |n: [i32; 3], d: i32, p: [i16; 3]| {
            (n[2] * i32::from(p[2]) + n[1] * i32::from(p[1]) + n[0] * i32::from(p[0]))
                .wrapping_add(d)
        };
        let sign = |v: i32| {
            if v == 0 {
                0
            } else if v < 0 {
                -1
            } else {
                1
            }
        };
        let mut best: Option<(i32, [i16; 3], [i16; 3])> = None;
        let (s0, s1, s2) = (
            i32::from(start[0]),
            i32::from(start[1]),
            i32::from(start[2]),
        );
        let (e0, e1, e2) = (i32::from(end[0]), i32::from(end[1]), i32::from(end[2]));
        for k in 0..count {
            let at = surfaces + 0x40 * k;
            if geom.u16(at) & 0x40 != 0 {
                continue;
            }
            let n = [geom.i16(at + 2), geom.i16(at + 4), geom.i16(at + 6)];
            let n32 = n.map(i32::from);
            let d = geom.u32(at + 8) as i32;
            let d1 = dot(n32, d, start);
            let d2 = dot(n32, d, end);
            if sign(d1) == sign(d2) {
                continue;
            }
            let den = (n32[2] * (e2 - s2))
                .wrapping_add(n32[1] * (e1 - s1))
                .wrapping_add(n32[0] * (e0 - s0));
            if den == 0 {
                continue;
            }
            let t = d1.wrapping_mul(-0x400) / den;
            if !(0..=0x400).contains(&t) || best.is_some_and(|b| t >= b.0) {
                continue;
            }
            let p = [
                (((t * (e0 - s0)) >> 10) as i16).wrapping_add(start[0]),
                (((t * (e1 - s1)) >> 10) as i16).wrapping_add(start[1]),
                (((t * (e2 - s2)) >> 10) as i16).wrapping_add(start[2]),
            ];
            let v = |c: usize| {
                let i = geom.i16(at + c).max(0) as usize;
                let o = vertices + 6 * i;
                [geom.i16(o), geom.i16(o + 2), geom.i16(o + 4)]
            };
            if point_in_surface(n, [v(0x20), v(0x28), v(0x30)], p) {
                best = Some((t, n, p));
            }
        }
        Ok(best.map(|b| (b.1, b.2)))
    }
}

fn point_in_surface(n: [i16; 3], v: [[i16; 3]; 3], p: [i16; 3]) -> bool {
    let (ax, ay, az) = (
        i32::from(n[0]).abs(),
        i32::from(n[1]).abs(),
        i32::from(n[2]).abs(),
    );
    let axis = if ay < ax && az < ax {
        2
    } else if ay <= ax || ay <= az {
        0
    } else {
        1
    };
    let s = |a: i16, b: i16| a.wrapping_sub(b);
    let (e1u, e2u, e1v, pu, e2v, pv) = match axis {
        0 => (
            s(v[1][0], v[0][0]),
            s(v[2][0], v[0][0]),
            s(v[1][1], v[0][1]),
            s(p[0], v[0][0]),
            s(v[2][1], v[0][1]),
            s(p[1], v[0][1]),
        ),
        1 => (
            s(v[1][0], v[0][0]),
            s(v[2][0], v[0][0]),
            s(v[1][2], v[0][2]),
            s(p[0], v[0][0]),
            s(v[2][2], v[0][2]),
            s(p[2], v[0][2]),
        ),
        _ => (
            s(v[1][1], v[0][1]),
            s(v[2][1], v[0][1]),
            s(v[1][2], v[0][2]),
            s(p[1], v[0][1]),
            s(v[2][2], v[0][2]),
            s(p[2], v[0][2]),
        ),
    };
    let (e1u, e2u, e1v, pu, e2v, pv) = (
        i32::from(e1u),
        i32::from(e2u),
        i32::from(e1v),
        i32::from(pu),
        i32::from(e2v),
        i32::from(pv),
    );
    let sum = if e1u == 0 {
        if e2u == 0 || e1v == 0 {
            return false;
        }
        let b = pu.wrapping_shl(10) / e2u;
        if !(0..=0x400).contains(&b) {
            return false;
        }
        let a = pv.wrapping_mul(0x400).wrapping_sub(e2v.wrapping_mul(b)) / e1v;
        if a < 0 {
            return false;
        }
        a + b
    } else {
        let den = e2v.wrapping_mul(e1u).wrapping_sub(e1v.wrapping_mul(e2u));
        if den == 0 {
            return false;
        }
        let b = (pv.wrapping_mul(e1u).wrapping_sub(e1v.wrapping_mul(pu))).wrapping_mul(0x400) / den;
        if !(0..=0x400).contains(&b) {
            return false;
        }
        let a = pu.wrapping_mul(0x400).wrapping_sub(e2u.wrapping_mul(b)) / e1u;
        if a < 0 {
            return false;
        }
        a + b
    };
    sum <= 0x400
}

impl TflSim {
    pub fn model_instances(&self) -> &[ModelInstance] {
        &self.model_instances
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_angle_truncates_far_from_an_integer() {
        for n in 0..=0xffffu32 {
            if n & 0x3fff == 0 {
                continue;
            }
            for float_pitch in [false, true] {
                let mut a = f64::from(n) * crate::commands::full_turn() * (1.0 / 65536.0);
                if float_pitch {
                    a = f64::from(a as f32);
                }
                for v in [a.sin() * 16384.0, a.cos() * 16384.0] {
                    assert!((v - v.round()).abs() > 1e-6, "angle {n:#x}: {v}");
                }
            }
        }
    }

    #[test]
    fn the_angle_vectors() {
        assert_eq!(
            angle_vectors(0x4000, 0xc000),
            ([0x4000, 0, 0], [0, 0, 0x4000])
        );
        assert_eq!(sin_cos(0xb000, false), (-0x3b20, -0x187d));
        assert_eq!(
            angle_vectors(0xb000, 0xc000),
            ([-0x3b20, 0x187d, 0], [0, 0, 0x4000])
        );
    }
}
