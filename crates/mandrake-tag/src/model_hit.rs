use crate::model::{GEOMETRY_SURFACE_NOT_SOLID, GeometryTag};

const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
const EPS: f32 = 1.525_878_9e-5;

fn near(x: f32, c: f32) -> bool {
    let d = x - c;
    if 0.0 > d { d > -EPS } else { EPS > d }
}

fn trig(angle: u16) -> (i32, i32) {
    let a = (f32::from(angle) * TWO_PI) * EPS;
    let q = [
        0.0f32,
        TWO_PI * 0.25,
        (TWO_PI + TWO_PI) * 0.25,
        0.25 * (TWO_PI * 3.0),
    ];
    let exact = [(0x4000, 0), (0, 0x4000), (-0x4000, 0), (0, -0x4000)];
    for (k, &c) in q.iter().enumerate() {
        if near(a, c) {
            return exact[k];
        }
    }
    let (s, c) = f64::from(a).sin_cos();
    ((c * 16384.0) as i32, (s * 16384.0) as i32)
}

pub fn build_rotation_matrix(yaw: u16, pitch: u16) -> [i16; 9] {
    let (ca, sa) = trig(yaw);
    let (cb, sb) = trig(pitch);
    let (ca, sa, cb, sb) = (
        i32::from(ca as i16),
        i32::from(sa as i16),
        i32::from(cb as i16),
        i32::from(sb as i16),
    );
    [
        sa as i16,
        ((-(ca * sb)) >> 14) as i16,
        ((-(ca * cb)) >> 14) as i16,
        (-ca) as i16,
        ((-(sb * sa)) >> 14) as i16,
        ((-(cb * sa)) >> 14) as i16,
        0,
        cb as i16,
        (-sb) as i16,
    ]
}

pub fn build_rotation_matrix3d(u: [i16; 3], f: [i16; 3]) -> [i16; 9] {
    let a = u.map(i32::from);
    let b = f.map(i32::from);
    [
        u[0],
        ((b[1] * a[2] - b[2] * a[1]) >> 14) as i16,
        f[0],
        u[1],
        ((b[2] * a[0] - a[2] * b[0]) >> 14) as i16,
        f[1],
        u[2],
        ((b[0] * a[1] - a[0] * b[1]) >> 14) as i16,
        f[2],
    ]
}

pub fn model_world_matrix(yaw: u16, pitch: u16) -> [i16; 9] {
    let r = build_rotation_matrix(yaw, pitch);
    build_rotation_matrix3d([r[0], r[3], r[6]], [r[2], r[5], r[8]]).map(|v| v >> 5)
}

pub fn model_inverse_matrix(world: &[i16; 9]) -> [i16; 9] {
    let m = world;
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

pub fn transform_geometry_point_to_world(
    position: [i32; 3],
    origin: [i16; 3],
    point: [i16; 3],
    m: &[i16; 9],
) -> [i32; 3] {
    let d = [0, 1, 2].map(|k| i32::from(point[k]) - i32::from(origin[k]));
    let m = m.map(i32::from);
    let v = if m[0] == 0x200 && m[4] == 0x200 && m[8] == 0x200 {
        d
    } else {
        [0, 1, 2].map(|c| {
            let s = m[c]
                .wrapping_mul(d[0])
                .wrapping_add(m[3 + c].wrapping_mul(d[1]))
                .wrapping_add(m[6 + c].wrapping_mul(d[2]));
            ((s as u32) >> 9) as i32
        })
    };
    [0, 1, 2].map(|k| position[k].wrapping_add(i32::from(v[k] as i16)))
}

pub fn transform_world_point_to_geometry(
    position: [i32; 3],
    origin: [i16; 3],
    point: [i32; 3],
    inverse: &[i16; 9],
) -> [i16; 3] {
    let d = [0, 1, 2].map(|k| point[k].wrapping_sub(position[k]));
    let m = inverse.map(i32::from);
    let v = if m[0] == 0x200 && m[4] == 0x200 && m[8] == 0x200 {
        d.map(|v| v as i16)
    } else {
        [0, 1, 2].map(|c| {
            (m[c]
                .wrapping_mul(d[0])
                .wrapping_add(m[3 + c].wrapping_mul(d[1]))
                .wrapping_add(m[6 + c].wrapping_mul(d[2]))
                >> 9) as i16
        })
    };
    [0, 1, 2].map(|k| v[k].wrapping_add(origin[k]))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitSurface {
    pub flags: u16,
    pub normal: [i16; 3],
    pub distance: i32,
    pub vertices: [u16; 3],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HitGeometry {
    pub origin: [i16; 3],
    pub vertices: Vec<[i16; 3]>,
    pub surfaces: Vec<HitSurface>,
    pub bounds: [[i16; 3]; 2],
}

impl HitGeometry {
    pub fn new(geom: &GeometryTag) -> Self {
        Self {
            origin: [geom.origin.0, geom.origin.1, geom.origin.2],
            vertices: geom.vertices.iter().map(|v| [v.x, v.y, v.z]).collect(),
            surfaces: geom
                .surfaces
                .iter()
                .map(|s| HitSurface {
                    flags: s.flags,
                    normal: [s.normal.0, s.normal.1, s.normal.2],
                    distance: s.distance,
                    vertices: s.vertices.map(|v| v.vertex_index),
                })
                .collect(),
            bounds: geom.bounds,
        }
    }

    fn vertex(&self, i: u16) -> [i32; 3] {
        self.vertices
            .get(usize::from(i))
            .copied()
            .unwrap_or([0; 3])
            .map(i32::from)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceTest {
    Gameplay,
    Legacy,
}

impl SurfaceTest {
    pub fn inside(self, p: [i16; 3], s: &HitSurface, g: &HitGeometry) -> bool {
        let n = s.normal.map(i32::from);
        let p32 = p.map(i32::from);
        (0..3).all(|k| {
            let a = g.vertex(s.vertices[k]);
            let b = g.vertex(s.vertices[(k + 1) % 3]);
            let c = match self {
                Self::Gameplay => {
                    let d = [0, 1, 2].map(|i| p32[i] - a[i]);
                    let e = [0, 1, 2].map(|i| b[i] - a[i]);
                    [
                        d[1].wrapping_mul(e[2])
                            .wrapping_sub(d[2].wrapping_mul(e[1]))
                            >> 14,
                        d[2].wrapping_mul(e[0])
                            .wrapping_sub(e[2].wrapping_mul(d[0]))
                            >> 14,
                        e[1].wrapping_mul(d[0])
                            .wrapping_sub(e[0].wrapping_mul(d[1]))
                            >> 14,
                    ]
                }
                Self::Legacy => {
                    let d = [0, 1, 2].map(|i| i32::from((p32[i] - a[i]) as i16));
                    let e = [0, 1, 2].map(|i| i32::from((b[i] - a[i]) as i16));
                    [
                        (d[1] * e[2] - d[2] * e[1]) >> 9,
                        (d[2] * e[0] - d[0] * e[2]) >> 9,
                        (d[0] * e[1] - d[1] * e[0]) >> 9,
                    ]
                    .map(|v| i32::from(v as i16))
                }
            };
            c[0].wrapping_mul(n[0])
                .wrapping_add(c[1].wrapping_mul(n[1]))
                .wrapping_add(c[2].wrapping_mul(n[2]))
                >= 0
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPlacement {
    pub position: [i32; 3],
    pub matrix: [i16; 9],
    pub inverse: [i16; 9],
    pub bounds: [[i32; 3]; 2],
}

impl ModelPlacement {
    pub fn new(position: [i32; 3], yaw: u16, pitch: u16, geometry: &HitGeometry) -> Self {
        let matrix = model_world_matrix(yaw, pitch);
        let corner =
            |p: [i16; 3]| transform_geometry_point_to_world(position, geometry.origin, p, &matrix);
        Self {
            position,
            matrix,
            inverse: model_inverse_matrix(&matrix),
            bounds: [corner(geometry.bounds[0]), corner(geometry.bounds[1])],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelHit {
    pub point: [i32; 3],
    pub normal: [i16; 3],
    pub surface: usize,
}

fn down8(v: i32) -> i32 {
    if v < 0 {
        v.wrapping_add(0xff) >> 8
    } else {
        v >> 8
    }
}

fn clip_face(num: i32, den: i32, t_in: &mut i32, t_out: &mut i32) -> bool {
    if den == 0 {
        return num <= 0;
    }
    let (n, d) = if num.wrapping_abs() > 0x8000 {
        (down8(num), down8(den))
    } else {
        (num, den)
    };
    let t = if d == 0 {
        i32::MAX
    } else {
        n.wrapping_shl(16).wrapping_div(d)
    };
    if den > 0 {
        if t > *t_out {
            return false;
        }
        *t_in = (*t_in).max(t);
    } else {
        if t < *t_in {
            return false;
        }
        *t_out = (*t_out).min(t);
    }
    true
}

pub fn intersects_with_model_bounding_box(
    model: &ModelPlacement,
    a: [i32; 3],
    b: [i32; 3],
) -> bool {
    let (mut t_in, mut t_out) = (0i32, 0x10000i32);
    (0..3).all(|k| {
        let lo = model.bounds[0][k].min(model.bounds[1][k]);
        let hi = model.bounds[0][k].max(model.bounds[1][k]);
        let d = i32::from(b[k].wrapping_sub(a[k]) as i16);
        clip_face(lo.wrapping_sub(a[k]), d, &mut t_in, &mut t_out)
            && clip_face(a[k].wrapping_sub(hi), -d, &mut t_in, &mut t_out)
    })
}

fn plane_crossing(s: &HitSurface, p0: [i16; 3], p1: [i16; 3], delta: [i32; 3]) -> Option<i32> {
    let side = |p: [i16; 3]| {
        (0..3)
            .fold(0i32, |acc, k| {
                acc.wrapping_add(i32::from(s.normal[k]).wrapping_mul(i32::from(p[k])))
            })
            .wrapping_add(s.distance)
    };
    let (s0, s1) = (side(p0), side(p1));
    if s0 == 0 && s1 == 0 {
        return None;
    }
    if s0 != 0 && s1 != 0 && (s0 < 0) == (s1 < 0) {
        return None;
    }
    let den = (0..3).fold(0i32, |acc, k| {
        acc.wrapping_add(i32::from(s.normal[k]).wrapping_mul(delta[k]))
    });
    if den == 0 {
        return None;
    }
    let (mut num, mut den) = (s0.wrapping_neg(), den);
    if s0.wrapping_abs() > 0x40_0000 {
        num = down8(num);
        den = down8(den);
    }
    if den == 0 {
        return None;
    }
    let t = num.wrapping_shl(9).wrapping_div(den);
    (t as u32 <= 0x200).then_some(t)
}

fn nearest_surface(
    geometry: &HitGeometry,
    p0: [i16; 3],
    p1: [i16; 3],
    test: SurfaceTest,
    best_t: &mut i32,
) -> Option<(usize, [i16; 3])> {
    let delta = [0, 1, 2].map(|k| i32::from(p1[k]) - i32::from(p0[k]));
    let mut best = None;
    for (i, s) in geometry.surfaces.iter().enumerate() {
        if s.flags & GEOMETRY_SURFACE_NOT_SOLID != 0 {
            continue;
        }
        let Some(t) = plane_crossing(s, p0, p1, delta) else {
            continue;
        };
        if t >= *best_t {
            continue;
        }
        let p = [0, 1, 2].map(|k| p0[k].wrapping_add((delta[k].wrapping_mul(t) >> 9) as i16));
        if test.inside(p, s, geometry) {
            *best_t = t;
            best = Some((i, p));
        }
    }
    best
}

fn world_normal(model: &ModelPlacement, s: &HitSurface) -> [i16; 3] {
    let n = s.normal.map(i32::from);
    let m = model.matrix.map(i32::from);
    [0, 1, 2].map(|c| {
        (n[0]
            .wrapping_mul(m[c])
            .wrapping_add(n[1].wrapping_mul(m[3 + c]))
            .wrapping_add(n[2].wrapping_mul(m[6 + c]))
            >> 9) as i16
    })
}

pub fn object_hit_model_index(
    model: &ModelPlacement,
    geometry: &HitGeometry,
    a: [i32; 3],
    b: [i32; 3],
    test: SurfaceTest,
    offset: i32,
) -> Option<ModelHit> {
    if !intersects_with_model_bounding_box(model, a, b) {
        return None;
    }
    let to_geometry = |p: [i32; 3]| {
        transform_world_point_to_geometry(model.position, geometry.origin, p, &model.inverse)
    };
    let p1 = to_geometry(b);
    let p0 = to_geometry(a);
    let mut best_t = i32::MAX;
    let (surface, mut p) = nearest_surface(geometry, p0, p1, test, &mut best_t)?;
    let n = geometry.surfaces[surface].normal.map(i32::from);
    let normal = world_normal(model, &geometry.surfaces[surface]);
    if offset != 0 && normal[2] <= 0x3f37 {
        for k in 0..3 {
            p[k] = p[k].wrapping_add((n[k].wrapping_mul(offset) >> 14) as i16);
        }
    }
    Some(ModelHit {
        point: transform_geometry_point_to_world(model.position, geometry.origin, p, &model.matrix),
        normal,
        surface,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NearestModelSurface {
    pub t: i32,
    pub p0: [i16; 3],
    pub model: usize,
    pub surface: usize,
}

pub fn nearest_model_surface<'a>(
    candidates: impl IntoIterator<Item = (&'a ModelPlacement, &'a HitGeometry)>,
    a: [i32; 3],
    b: [i32; 3],
    test: SurfaceTest,
) -> Option<NearestModelSurface> {
    let mut best_t = i32::MAX;
    let mut p0_out = [0i16; 3];
    let mut found = None;
    for (i, (model, geometry)) in candidates.into_iter().enumerate() {
        if !intersects_with_model_bounding_box(model, a, b) {
            continue;
        }
        let to_geometry = |p: [i32; 3]| {
            transform_world_point_to_geometry(model.position, geometry.origin, p, &model.inverse)
        };
        let p0 = to_geometry(a);
        let p1 = to_geometry(b);
        p0_out = p0;
        if let Some((surface, _)) = nearest_surface(geometry, p0, p1, test, &mut best_t) {
            found = Some((i, surface));
        }
    }
    found.map(|(model, surface)| NearestModelSurface {
        t: best_t,
        p0: p0_out,
        model,
        surface,
    })
}

pub fn object_hit_model_point(
    model: &ModelPlacement,
    geometry: &HitGeometry,
    found: &NearestModelSurface,
    a: [i32; 3],
    b: [i32; 3],
    offset: i32,
) -> ModelHit {
    let s = &geometry.surfaces[found.surface];
    let mut point = b;
    let mut lo = 0u32;
    let mut hi: u32 = if found.t <= 0x1fe {
        (found.t + 1) as u32
    } else {
        0x200
    };
    while lo < hi {
        let mid = (lo + hi) >> 1;
        let at = [0, 1, 2].map(|k| {
            b[k].wrapping_sub(a[k])
                .wrapping_mul(mid as i32)
                .wrapping_shr(9)
                .wrapping_add(a[k])
        });
        if mid != 0 {
            let p1 = transform_world_point_to_geometry(
                model.position,
                geometry.origin,
                at,
                &model.inverse,
            );
            let delta = [0, 1, 2].map(|k| i32::from(p1[k]) - i32::from(found.p0[k]));
            if plane_crossing(s, found.p0, p1, delta).is_some() {
                hi = mid;
                continue;
            }
        }
        point = at;
        lo = mid + 1;
    }
    let normal = world_normal(model, s);
    if offset != 0 && normal[2] <= 0x3f37 {
        for k in 0..3 {
            let d = i32::from(normal[k]).wrapping_mul(offset) >> 14;
            point[k] = point[k].wrapping_add(d);
        }
    }
    ModelHit {
        point,
        normal,
        surface: found.surface,
    }
}

pub fn get_model_height_at_position(
    model: &ModelPlacement,
    geometry: &HitGeometry,
    x: i32,
    y: i32,
    test: SurfaceTest,
) -> i16 {
    object_hit_model_index(model, geometry, [x, y, 0x1000], [x, y, -0x1000], test, 0)
        .map_or(-0x1000, |h| h.point[2] as i16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_angles_are_exact() {
        assert_eq!(trig(0), (0x4000, 0));
        assert_eq!(trig(0x4000), (0, 0x4000));
        assert_eq!(trig(0x8000), (-0x4000, 0));
        assert_eq!(trig(0xc000), (0, -0x4000));
    }

    #[test]
    fn identity_marker_gives_the_unit_world_matrix() {
        let r = build_rotation_matrix(0, 0xc000);
        assert_eq!(r, [0, 0x4000, 0, -0x4000, 0, 0, 0, 0, 0x4000]);
        assert_eq!(
            model_world_matrix(0, 0xc000),
            [0, 0x200, 0, -0x200, 0, 0, 0, 0, 0x200]
        );
    }

    #[test]
    fn inverse_is_the_transpose() {
        let m = model_world_matrix(0xc000, 0xc000);
        assert_eq!(m, [-512, 0, 0, 0, -512, 0, 0, 0, 512]);
        assert_eq!(model_inverse_matrix(&m), m);
        let m = model_world_matrix(0, 0xc000);
        assert_eq!(model_inverse_matrix(&m), [0, -512, 0, 512, 0, 0, 0, 0, 512]);
    }

    #[test]
    fn transforms_round_trip() {
        let origin = [100, -200, 30];
        for (yaw, pitch) in [(0xc000, 0xc000), (0, 0xc000), (0x4000, 0xc000)] {
            let geometry = HitGeometry {
                origin,
                ..HitGeometry::default()
            };
            let m = ModelPlacement::new([39101, 83732, -1027], yaw, pitch, &geometry);
            for p in [[39000, 83800, -500], [39371, 84106, 431]] {
                let g = transform_world_point_to_geometry(m.position, origin, p, &m.inverse);
                let w = transform_geometry_point_to_world(m.position, origin, g, &m.matrix);
                assert_eq!(w, p, "{yaw:#x}");
            }
        }
    }

    fn floor() -> HitGeometry {
        let v = vec![[0, 0, 0], [512, 0, 0], [512, 512, 0], [0, 512, 0]];
        let up = [0, 0, 0x4000];
        HitGeometry {
            origin: [0, 0, 0],
            vertices: v,
            surfaces: vec![
                HitSurface {
                    flags: 0,
                    normal: up,
                    distance: 0,
                    vertices: [0, 2, 1],
                },
                HitSurface {
                    flags: 0,
                    normal: up,
                    distance: 0,
                    vertices: [0, 3, 2],
                },
            ],
            bounds: [[0, 0, -64], [512, 512, 64]],
        }
    }

    #[test]
    fn a_segment_through_the_floor_hits_it() {
        let g = floor();
        let m = ModelPlacement::new([1000, 2000, 0], 0x4000, 0xc000, &g);
        assert_eq!(m.matrix, [512, 0, 0, 0, 512, 0, 0, 0, 512]);
        for test in [SurfaceTest::Gameplay, SurfaceTest::Legacy] {
            let h = object_hit_model_index(&m, &g, [1100, 2100, 100], [1300, 2300, -100], test, 0)
                .expect("hit");
            assert_eq!(h.point, [1200, 2200, 0]);
            assert_eq!(h.normal, [0, 0, 0x4000]);
            assert!(
                object_hit_model_index(&m, &g, [1300, 2300, -100], [1100, 2100, 100], test, 0)
                    .is_some()
            );
            assert_eq!(
                object_hit_model_index(&m, &g, [1100, 2100, 60], [1300, 2300, 10], test, 0),
                None
            );
            assert_eq!(
                object_hit_model_index(&m, &g, [1100, 2100, 300], [1300, 2300, 100], test, 0),
                None
            );
            assert_eq!(
                object_hit_model_index(&m, &g, [1600, 2100, 50], [1700, 2100, -50], test, 0),
                None
            );
        }
        let mut g2 = g.clone();
        for s in &mut g2.surfaces {
            s.flags = GEOMETRY_SURFACE_NOT_SOLID;
        }
        assert_eq!(
            object_hit_model_index(
                &m,
                &g2,
                [1100, 2100, 100],
                [1300, 2300, -100],
                SurfaceTest::Gameplay,
                0
            ),
            None
        );
    }

    #[test]
    fn vertical_segment_height_grid() {
        let mut g = floor();
        for s in &mut g.surfaces {
            s.distance = -54 * 0x4000;
        }
        g.bounds = [[0, 0, 0], [512, 512, 64]];
        let m = ModelPlacement::new([0, 0, 0], 0x4000, 0xc000, &g);
        let h = object_hit_model_index(
            &m,
            &g,
            [100, 100, 0x1000],
            [100, 100, -0x1000],
            SurfaceTest::Gameplay,
            0,
        )
        .expect("hit");
        assert_eq!(h.point[2], 64);
    }

    #[test]
    fn nearest_model_surface_shares_t_and_keeps_the_last_p0() {
        let g = floor();
        let at = |z| ModelPlacement::new([1000, 2000, z], 0x4000, 0xc000, &g);
        let (low, high) = (at(0), at(40));
        let (a, b) = ([1100, 2100, 100], [1300, 2300, -100]);
        let r = nearest_model_surface([(&low, &g), (&high, &g)], a, b, SurfaceTest::Gameplay)
            .expect("hit");
        assert_eq!(r.model, 1);
        assert_eq!(r.p0, [100, 100, 60]);
        let r = nearest_model_surface([(&high, &g), (&low, &g)], a, b, SurfaceTest::Gameplay)
            .expect("hit");
        assert_eq!(r.model, 0);
        assert_eq!(r.t, (60 << 9) / 200);
        assert_eq!(r.p0, [100, 100, 100]);
        let mut none = g.clone();
        for s in &mut none.surfaces {
            s.flags = GEOMETRY_SURFACE_NOT_SOLID;
        }
        let r = nearest_model_surface([(&low, &g), (&low, &none)], a, b, SurfaceTest::Gameplay);
        assert_eq!(r.map(|r| r.model), Some(0));
        assert_eq!(
            nearest_model_surface([(&low, &none)], a, [1300, 2300, 90], SurfaceTest::Gameplay),
            None
        );
    }

    #[test]
    fn object_hit_model_point_stops_before_the_plane() {
        let g = floor();
        let m = ModelPlacement::new([1000, 2000, 0], 0x4000, 0xc000, &g);
        let (a, b) = ([1100, 2100, 100], [1300, 2300, -100]);
        let r = nearest_model_surface([(&m, &g)], a, b, SurfaceTest::Gameplay).expect("hit");
        assert_eq!(r.t, 0x100);
        let h = object_hit_model_point(&m, &g, &r, a, b, 32);
        assert_eq!(h.normal, [0, 0, 0x4000]);
        assert_eq!(h.point, [1198, 2198, 1]);
        let mut tilted = g.clone();
        for s in &mut tilted.surfaces {
            s.normal = [0, 0x2d41, 0x2d41];
        }
        let found = NearestModelSurface {
            t: 0x100,
            p0: [100, 100, 100],
            model: 0,
            surface: 0,
        };
        let plain = object_hit_model_point(&m, &tilted, &found, a, b, 0);
        let moved = object_hit_model_point(&m, &tilted, &found, a, b, 32);
        assert_eq!(plain.normal, [0, 0x2d41, 0x2d41]);
        assert_eq!(moved.point[0], plain.point[0]);
        assert_eq!(moved.point[1], plain.point[1] + ((0x2d41 * 32) >> 14));
        assert_eq!(moved.point[2], plain.point[2] + ((0x2d41 * 32) >> 14));
    }

    #[test]
    fn model_height_is_the_low_word_or_the_miss_value() {
        let g = floor();
        let m = ModelPlacement::new([1000, 2000, 0], 0x4000, 0xc000, &g);
        assert_eq!(
            get_model_height_at_position(&m, &g, 1100, 2100, SurfaceTest::Gameplay),
            0
        );
        assert_eq!(
            get_model_height_at_position(&m, &g, 3000, 2100, SurfaceTest::Gameplay),
            -0x1000
        );
    }

    #[test]
    fn box_faces() {
        let (mut a, mut b) = (0, 0x10000);
        assert!(!clip_face(1, 0, &mut a, &mut b));
        assert!(clip_face(0, 0, &mut a, &mut b));
        assert!(clip_face(50, 100, &mut a, &mut b));
        assert_eq!(a, 0x8000);
        assert!(clip_face(-75, -100, &mut a, &mut b));
        assert_eq!(b, 0xc000);
        assert!(!clip_face(90, 100, &mut a, &mut b));
        let (mut a, mut b) = (0, 0x10000);
        assert!(clip_face(0x9000, 0x12000, &mut a, &mut b));
        assert_eq!(a, (0x90 << 16) / 0x120);
        let (mut a, mut b) = (0, 0x10000);
        assert!(!clip_face(0x9000, 0x80, &mut a, &mut b));
        assert!(clip_face(0x9000, -0x80, &mut a, &mut b));
        assert_eq!(b, 0x10000);
    }
}
