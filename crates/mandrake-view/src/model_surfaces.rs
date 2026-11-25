use crate::camera::Mat4;
use crate::media::MediaWobble;
use crate::model::{ModelVertex, TranslucentDrawList, TranslucentRun};
use mandrake_tag::CollectionTag;
use mandrake_tag::model::ModelPermutation;
use mandrake_tag::model::{
    GEOMETRY_SURFACE_CATEGORY_2, GEOMETRY_SURFACE_DEPTH_FARTHER, GEOMETRY_SURFACE_DEPTH_NEAREST,
    GEOMETRY_SURFACE_DEPTH_NEAREST_FARTHER, GEOMETRY_SURFACE_TWO_SIDED, GeometryTag,
};
use mandrake_tag::model_hit::{HitGeometry, HitSurface};

pub const WORLD_ONE: f32 = crate::WORLD_UNITS_PER_CELL;

pub const CELL_FLAG_REFLECTIVE: u16 = 0x2000;

pub fn material_resolves(
    geom: &GeometryTag,
    material: usize,
    collection: &CollectionTag,
    permutation: Option<&ModelPermutation>,
) -> bool {
    let Some(m) = geom.materials.get(material) else {
        return false;
    };
    let Ok(sequence) = usize::try_from(m.texture_index) else {
        return false;
    };
    let view = match permutation {
        Some(p) => p.material_view(material),
        None => Some(0),
    };
    let bitmap = view.and_then(|v| collection.sequence(sequence)?.bitmap_index(v, 0));
    bitmap.is_some_and(|b| collection.frame(b).is_some())
}

#[derive(Clone, Debug, PartialEq)]
pub struct EngineSurface {
    pub flags: u16,
    pub normal: [i16; 3],
    pub distance: i32,
    pub transparency: u16,
    pub has_material: bool,
    pub vertices: [u16; 3],
    pub draw_after: Vec<u16>,
    pub mesh_vertex: u32,
    pub submesh: u32,
}

impl EngineSurface {
    pub fn plane_side(&self, p: [f32; 3]) -> f32 {
        f32::from(self.normal[0]) * p[0]
            + f32::from(self.normal[1]) * p[1]
            + f32::from(self.normal[2]) * p[2]
            + self.distance as f32
    }

    pub fn faces(&self, eye: [f32; 3]) -> bool {
        self.plane_side(eye) > 0.0 || self.flags & GEOMETRY_SURFACE_TWO_SIDED != 0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineGeometry {
    pub origin: [i16; 3],
    pub vertices: Vec<[i16; 3]>,
    pub surfaces: Vec<EngineSurface>,
    pub bounds: [[i16; 3]; 2],
}

impl EngineGeometry {
    pub fn new(geom: &GeometryTag, slots: &[(u32, u32)]) -> Self {
        let surfaces = geom
            .surfaces
            .iter()
            .enumerate()
            .map(|(i, s)| EngineSurface {
                flags: s.flags,
                normal: [s.normal.0, s.normal.1, s.normal.2],
                distance: s.distance,
                transparency: s.transparency,
                has_material: usize::try_from(s.material_index)
                    .ok()
                    .and_then(|m| geom.materials.get(m))
                    .is_some_and(|m| m.texture_index != -1),
                vertices: s.vertices.map(|v| v.vertex_index),
                draw_after: geom.draw_after(i),
                mesh_vertex: slots.get(i).map_or(0, |s| s.0),
                submesh: slots.get(i).map_or(0, |s| s.1),
            })
            .collect();
        Self {
            origin: [geom.origin.0, geom.origin.1, geom.origin.2],
            vertices: geom.vertices.iter().map(|v| [v.x, v.y, v.z]).collect(),
            surfaces,
            bounds: geom.bounds,
        }
    }

    pub fn resolve_materials(
        &mut self,
        geom: &GeometryTag,
        collection: &CollectionTag,
        permutation: Option<&ModelPermutation>,
    ) {
        for (surface, s) in self.surfaces.iter_mut().zip(&geom.surfaces) {
            surface.has_material = usize::try_from(s.material_index)
                .is_ok_and(|m| material_resolves(geom, m, collection, permutation));
        }
    }

    pub fn has_category2(&self) -> bool {
        self.surfaces
            .iter()
            .any(|s| s.flags & GEOMETRY_SURFACE_CATEGORY_2 != 0)
    }

    pub fn hit_geometry(&self) -> HitGeometry {
        HitGeometry {
            origin: self.origin,
            vertices: self.vertices.clone(),
            surfaces: self
                .surfaces
                .iter()
                .map(|s| HitSurface {
                    flags: s.flags,
                    normal: s.normal,
                    distance: s.distance,
                    vertices: s.vertices,
                })
                .collect(),
            bounds: self.bounds,
        }
    }

    pub fn to_geometry(&self, p: [f32; 3]) -> [f32; 3] {
        [
            p[2] * WORLD_ONE + f32::from(self.origin[0]),
            p[0] * WORLD_ONE + f32::from(self.origin[1]),
            p[1] * WORLD_ONE + f32::from(self.origin[2]),
        ]
    }

    pub fn from_geometry(&self, g: [i16; 3]) -> [f32; 3] {
        [
            (f32::from(g[1]) - f32::from(self.origin[1])) / WORLD_ONE,
            (f32::from(g[2]) - f32::from(self.origin[2])) / WORLD_ONE,
            (f32::from(g[0]) - f32::from(self.origin[0])) / WORLD_ONE,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldFrame {
    pub half_width: f32,
    pub half_height: f32,
}

impl WorldFrame {
    pub fn new(cell_width: u16, cell_height: u16) -> Self {
        Self {
            half_width: f32::from(cell_width) * 0.5,
            half_height: f32::from(cell_height) * 0.5,
        }
    }

    pub fn to_engine(&self, p: [f32; 3]) -> [f32; 3] {
        [
            (self.half_width - p[0]) * WORLD_ONE,
            (p[2] + self.half_height) * WORLD_ONE,
            p[1] * WORLD_ONE,
        ]
    }

    pub fn from_engine(&self, e: [f32; 3]) -> [f32; 3] {
        [
            self.half_width - e[0] / WORLD_ONE,
            e[2] / WORLD_ONE,
            e[1] / WORLD_ONE - self.half_height,
        ]
    }
}

pub fn surface_depth(flags: u16, depths: [f32; 3]) -> f32 {
    let mut d = if flags & GEOMETRY_SURFACE_DEPTH_NEAREST != 0 {
        depths.iter().copied().fold(f32::NEG_INFINITY, f32::max)
    } else {
        depths.iter().copied().fold(f32::INFINITY, f32::min)
    };
    if flags & GEOMETRY_SURFACE_DEPTH_FARTHER != 0 {
        d -= 512.0;
    }
    if flags & GEOMETRY_SURFACE_DEPTH_NEAREST_FARTHER == GEOMETRY_SURFACE_DEPTH_NEAREST_FARTHER {
        d -= 384.0;
    }
    d
}

pub fn apply_draw_after(surfaces: &[EngineSurface], depths: &mut [Option<f32>]) {
    for (i, s) in surfaces.iter().enumerate() {
        if depths[i].is_none() {
            continue;
        }
        for &o in &s.draw_after {
            if let (Some(other), Some(own)) = (depths.get(o as usize).copied().flatten(), depths[i])
                && other >= own
            {
                depths[i] = Some(other + 1.0);
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct SurfaceInstance<'a> {
    pub model_matrix: [[f32; 4]; 4],
    pub geometry: &'a EngineGeometry,
    pub texture_words: &'a [u32],
}

#[derive(Clone, Copy)]
pub struct SurfaceRegion<'a> {
    pub region: &'a crate::view_region::ViewRegion,
    pub frame: WorldFrame,
}

impl SurfaceRegion<'_> {
    pub fn flags_vertex(&self, p: [f32; 3], depth: f32) -> bool {
        let e = self.frame.to_engine(p);
        depth < -34.0
            && self
                .region
                .vertex_inside(e[0].floor() as i32 >> 9, e[1].floor() as i32 >> 9)
    }
}

fn in_region(
    region: Option<&SurfaceRegion<'_>>,
    s: &EngineSurface,
    world: &dyn Fn(u16) -> [f32; 3],
    depth: &dyn Fn(u16) -> f32,
) -> bool {
    region.is_none_or(|r| {
        s.vertices
            .iter()
            .any(|&v| r.flags_vertex(world(v), depth(v)))
    })
}

pub fn region_culled_surfaces(
    instances: &[SurfaceInstance<'_>],
    view: &Mat4,
    view_proj: &Mat4,
    region: &SurfaceRegion<'_>,
) -> (usize, usize) {
    let (mut culled, mut on_screen) = (0, 0);
    let eye_world = view
        .inverse()
        .map_or([0.0; 3], |v| v.transform_point3([0.0; 3]));
    for inst in instances {
        let g = inst.geometry;
        let model = Mat4 {
            cols: inst.model_matrix,
        };
        let Some(inverse) = model.inverse() else {
            continue;
        };
        let eye = g.to_geometry(inverse.transform_point3(eye_world));
        let model_view = view.mul_mat4(&model);
        let local = |v: u16| {
            g.vertices
                .get(v as usize)
                .map_or([0.0; 3], |&g3| g.from_geometry(g3))
        };
        let depth_of = |v: u16| model_view.transform_point3(local(v))[2] * WORLD_ONE;
        let world_of = |v: u16| model.transform_point3(local(v));
        let clip = |v: u16| {
            let p = world_of(v);
            let c = &view_proj.cols;
            let row = |r: usize| c[0][r] * p[0] + c[1][r] * p[1] + c[2][r] * p[2] + c[3][r];
            [row(0), row(1), row(2), row(3)]
        };
        for s in &g.surfaces {
            if !s.has_material
                || s.flags & GEOMETRY_SURFACE_CATEGORY_2 != 0
                || !s.faces(eye)
                || in_region(Some(region), s, &world_of, &depth_of)
            {
                continue;
            }
            culled += 1;
            let corners = s.vertices.map(clip);
            let outside = |f: &dyn Fn(&[f32; 4]) -> bool| corners.iter().all(f);
            let hidden = outside(&|c| c[0] < -c[3])
                || outside(&|c| c[0] > c[3])
                || outside(&|c| c[1] < -c[3])
                || outside(&|c| c[1] > c[3])
                || outside(&|c| c[2] < 0.0);
            on_screen += usize::from(!hidden);
        }
    }
    (culled, on_screen)
}

pub fn category2_texture_word(collection: u8, bitmap: Option<usize>, color_table: i16) -> u32 {
    let bitmap = bitmap.map_or(0x3fff, |b| b as u32 & 0x3fff);
    u32::from(collection) | (bitmap << 8) | ((color_table as u32 & 0x7f) << 22)
}

pub fn category2_draw_list(
    instances: &[SurfaceInstance<'_>],
    view: &Mat4,
    region: Option<&SurfaceRegion<'_>>,
) -> TranslucentDrawList {
    let eye_world = view
        .inverse()
        .map_or([0.0; 3], |v| v.transform_point3([0.0; 3]));
    let mut keyed: Vec<(u32, f32, usize, usize)> = Vec::new();
    let mut verts: Vec<([f32; 3], f32)> = Vec::new();
    let mut depths: Vec<Option<f32>> = Vec::new();
    for (i, inst) in instances.iter().enumerate() {
        let g = inst.geometry;
        if !g.has_category2() {
            continue;
        }
        let model = Mat4 {
            cols: inst.model_matrix,
        };
        let Some(inverse) = model.inverse() else {
            continue;
        };
        let eye = g.to_geometry(inverse.transform_point3(eye_world));
        let model_view = view.mul_mat4(&model);
        verts.clear();
        verts.extend(g.vertices.iter().map(|&g3| {
            let local = g.from_geometry(g3);
            let world = if region.is_some() {
                model.transform_point3(local)
            } else {
                [0.0; 3]
            };
            let depth = model_view.transform_point3(local)[2] * WORLD_ONE;
            (world, depth)
        }));
        let fallback_world = if region.is_some() {
            model.transform_point3([0.0; 3])
        } else {
            [0.0; 3]
        };
        let fallback_depth = model_view.transform_point3([0.0; 3])[2] * WORLD_ONE;
        let world_of = |v: u16| verts.get(v as usize).map_or(fallback_world, |p| p.0);
        let depth_of = |v: u16| verts.get(v as usize).map_or(fallback_depth, |p| p.1);
        depths.clear();
        depths.extend(g.surfaces.iter().map(|s| {
            if !s.has_material || !s.faces(eye) || !in_region(region, s, &world_of, &depth_of) {
                return None;
            }
            let d = surface_depth(s.flags, s.vertices.map(depth_of));
            (d < 0.0).then_some(d)
        }));
        apply_draw_after(&g.surfaces, &mut depths);
        for (si, s) in g.surfaces.iter().enumerate() {
            if s.flags & GEOMETRY_SURFACE_CATEGORY_2 == 0 {
                continue;
            }
            if let Some(d) = depths[si] {
                let word = inst
                    .texture_words
                    .get(s.submesh as usize)
                    .copied()
                    .unwrap_or(0);
                keyed.push((word, d, i, si));
            }
        }
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let mut list = TranslucentDrawList::default();
    for (_, d, i, si) in keyed {
        let s = &instances[i].geometry.surfaces[si];
        let start = list.indices.len() as u32;
        list.indices
            .extend_from_slice(&[s.mesh_vertex, s.mesh_vertex + 1, s.mesh_vertex + 2]);
        list.keys.push(-d / WORLD_ONE);
        let submesh = s.submesh as usize;
        match list.runs.last_mut() {
            Some(run) if run.instance == i && run.submesh == submesh => run.indices.end = start + 3,
            _ => list.runs.push(TranslucentRun {
                instance: i,
                submesh,
                indices: start..start + 3,
            }),
        }
    }
    list
}

pub fn reflection_transparency(model: u16, media: u16) -> u16 {
    !(((u32::from(!model)) * u32::from(!media)) >> 16) as u16
}

pub struct ReflectionMedia<'a> {
    pub media_height: &'a dyn Fn(i32, i32) -> i32,
    pub wobble: MediaWobble,
    pub local_time: i32,
    pub sine: &'a [i16; 1024],
    pub transparency: u16,
    pub frame: WorldFrame,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReflectionTriangle {
    pub instance: usize,
    pub submesh: u32,
    pub surface: u16,
    pub vertices: [ModelVertex; 3],
    pub key: f32,
}

fn reflected_vertex_and_water_height(p: [f32; 3], media: &ReflectionMedia<'_>) -> ([f32; 3], f32) {
    let (ix, iy) = (p[0].floor() as i32, p[1].floor() as i32);
    let (col, row) = (ix >> 9, iy >> 9);
    let m = (media.media_height)(ix, iy) as f32;
    let d = p[2] - m;
    let mut out = [p[0], p[1], m - d];
    if d > 0.0 {
        let o = media.wobble.offset(col, row, media.local_time, media.sine);
        out[0] += o[0] as f32;
        out[1] += o[1] as f32;
        out[2] += o[2] as f32;
    }
    (out, media.frame.from_engine([p[0], p[1], m])[1])
}

pub fn reflected_vertex(p: [f32; 3], media: &ReflectionMedia<'_>) -> [f32; 3] {
    reflected_vertex_and_water_height(p, media).0
}

#[cfg(test)]
fn water_height(p: [f32; 3], media: &ReflectionMedia<'_>) -> f32 {
    reflected_vertex_and_water_height(p, media).1
}

pub fn reflection_triangles(
    instances: &[(SurfaceInstance<'_>, &[ModelVertex], u16)],
    view: &Mat4,
    eye: [f32; 3],
    media: &ReflectionMedia<'_>,
    region: Option<&SurfaceRegion<'_>>,
) -> Vec<ReflectionTriangle> {
    let frame = media.frame;
    let eye_e = frame.to_engine(eye);
    let eye_m = (media.media_height)(eye_e[0].floor() as i32, eye_e[1].floor() as i32) as f32;
    let mirrored_eye = frame.from_engine([eye_e[0], eye_e[1], 2.0 * eye_m - eye_e[2]]);
    let mut out: Vec<ReflectionTriangle> = Vec::new();
    let mut verts: Vec<([f32; 3], f32, f32)> = Vec::new();
    let mut depths: Vec<Option<f32>> = Vec::new();
    for (i, (inst, cpu, draw_transparency)) in instances.iter().enumerate() {
        let g = inst.geometry;
        let model = Mat4 {
            cols: inst.model_matrix,
        };
        let Some(inverse) = model.inverse() else {
            continue;
        };
        let geo_eye = g.to_geometry(inverse.transform_point3(mirrored_eye));
        verts.clear();
        verts.extend(g.vertices.iter().map(|&v| {
            let e = frame.to_engine(model.transform_point3(g.from_geometry(v)));
            let (refl_e, water_y) = reflected_vertex_and_water_height(e, media);
            let world = frame.from_engine(refl_e);
            let depth = view.transform_point3(world)[2] * WORLD_ONE;
            (world, water_y, depth)
        }));
        let depth = |v: u16| verts.get(v as usize).map_or(0.0, |p| p.2);
        let world_of = |v: u16| verts.get(v as usize).map_or([0.0; 3], |p| p.0);
        depths.clear();
        depths.extend(g.surfaces.iter().map(|s| {
            if !s.has_material
                || s.flags & GEOMETRY_SURFACE_CATEGORY_2 != 0
                || !s.faces(geo_eye)
                || !in_region(region, s, &world_of, &depth)
            {
                return None;
            }
            let d = surface_depth(s.flags, s.vertices.map(depth));
            (d < 0.0).then_some(d)
        }));
        apply_draw_after(&g.surfaces, &mut depths);
        let model_t = reflection_transparency(*draw_transparency, media.transparency);
        for (si, s) in g.surfaces.iter().enumerate() {
            let Some(d) = depths[si] else { continue };
            let t = s.transparency.max(model_t);
            let alpha = f32::from(((0xffffu32 - u32::from(t)) >> 8) as u8) / 255.0;
            let corner = |k: usize| {
                let mv = cpu
                    .get(s.mesh_vertex as usize + k)
                    .copied()
                    .unwrap_or(ModelVertex::new([0.0; 3], [0.0; 3], [0.0; 2]));
                let (p, surface) = verts
                    .get(s.vertices[k] as usize)
                    .map_or(([0.0; 3], 0.0), |v| (v.0, v.1));
                ModelVertex::new(p, [0.0, surface, 0.0], mv.uv).with_alpha(alpha)
            };
            out.push(ReflectionTriangle {
                instance: i,
                submesh: s.submesh,
                surface: si as u16,
                vertices: [corner(0), corner(1), corner(2)],
                key: -d / WORLD_ONE,
            });
        }
    }
    out.sort_by(|a, b| b.key.total_cmp(&a.key));
    out
}

pub fn model_reflects(
    cells: [[i32; 2]; 2],
    cell_flags: &dyn Fn(i32, i32) -> u16,
    geometry: &EngineGeometry,
) -> bool {
    for row in cells[0][1]..=cells[1][1] {
        for col in cells[0][0]..=cells[1][0] {
            if cell_flags(col, row) & CELL_FLAG_REFLECTIVE != 0 {
                return true;
            }
        }
    }
    geometry.has_category2()
}

pub fn bounds_cells(corners_world: [[f32; 3]; 2]) -> [[i32; 2]; 2] {
    let axis = |a: usize| {
        let (lo, hi) = (
            corners_world[0][a].min(corners_world[1][a]) as i32,
            corners_world[0][a].max(corners_world[1][a]) as i32,
        );
        (lo >> 9, (hi - 1) >> 9)
    };
    let (x0, x1) = axis(0);
    let (y0, y1) = axis(1);
    [[x0, y0], [x1, y1]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(flags: u16, normal: [i16; 3], distance: i32, vertices: [u16; 3]) -> EngineSurface {
        EngineSurface {
            flags,
            normal,
            distance,
            transparency: 0,
            has_material: true,
            vertices,
            draw_after: Vec::new(),
            mesh_vertex: 0,
            submesh: 0,
        }
    }

    #[test]
    fn surface_depth_rules() {
        let d = [-14270.0, -13981.0, -13973.0];
        assert_eq!(surface_depth(0, d), -14270.0);
        assert_eq!(surface_depth(0x8, d), -14270.0);
        assert_eq!(surface_depth(0x10, d), -13973.0);
        assert_eq!(surface_depth(0x4, d), -14782.0);
        assert_eq!(surface_depth(0x90, d), -13973.0 - 384.0);
    }

    #[test]
    fn draw_after_moves_nearer() {
        let mut s = vec![surface(0, [0, 0, 1], 0, [0, 1, 2]); 3];
        s[2].draw_after = vec![0, 1];
        let mut d = vec![Some(-100.0), Some(-50.0), Some(-200.0)];
        apply_draw_after(&s, &mut d);
        assert_eq!(d, vec![Some(-100.0), Some(-50.0), Some(-49.0)]);
        let mut d = vec![Some(-100.0), None, Some(-200.0)];
        apply_draw_after(&s, &mut d);
        assert_eq!(d[2], Some(-99.0));
    }

    #[test]
    fn reflection_transparency_combines() {
        assert_eq!(reflection_transparency(0, 0), 1);
        assert_eq!(reflection_transparency(0, 0x8000), 0x8001);
        assert_eq!(((0xffff - 0x8001u32) >> 8) as u8, 127);
    }

    #[test]
    fn reflected_vertex_mirrors_about_the_cell_media() {
        let sine = {
            let mut s = [0i16; 1024];
            for (i, v) in s.iter_mut().enumerate() {
                *v = ((i as f32 * core::f32::consts::TAU / 1024.0).sin() * 16384.0) as i16;
            }
            s
        };
        let height = |_x: i32, _y: i32| -1536;
        let media = ReflectionMedia {
            media_height: &height,
            wobble: MediaWobble {
                amplitude: [0, 0, 0],
                frequency: [0, 0, 0],
            },
            local_time: 0,
            sine: &sine,
            transparency: 0,
            frame: WorldFrame::new(10, 10),
        };
        assert_eq!(water_height([1000.0, 2000.0, -1000.0], &media), -3.0);
        assert_eq!(
            reflected_vertex([1000.0, 2000.0, -1000.0], &media),
            [1000.0, 2000.0, -2072.0]
        );
        let media = ReflectionMedia {
            wobble: MediaWobble {
                amplitude: [51, 51, 102],
                frequency: [256, 256, 512],
            },
            local_time: 35,
            ..media
        };
        let w = MediaWobble {
            amplitude: [51, 51, 102],
            frequency: [256, 256, 512],
        }
        .offset(1, 3, 35, &sine);
        assert_eq!(
            reflected_vertex([1000.0, 2000.0, -1000.0], &media),
            [
                1000.0 + w[0] as f32,
                2000.0 + w[1] as f32,
                -2072.0 + w[2] as f32
            ]
        );
        assert_eq!(
            reflected_vertex([1000.0, 2000.0, -1600.0], &media),
            [1000.0, 2000.0, -1472.0]
        );
    }

    #[test]
    fn facing_test() {
        let up = surface(0, [0, 0, 0x4000], -0x4000 * 10, [0, 1, 2]);
        assert!(up.faces([0.0, 0.0, 20.0]));
        assert!(!up.faces([0.0, 0.0, 5.0]));
        let two = surface(
            GEOMETRY_SURFACE_TWO_SIDED,
            [0, 0, 0x4000],
            -0x4000 * 10,
            [0, 1, 2],
        );
        assert!(two.faces([0.0, 0.0, 5.0]));
    }

    #[test]
    fn height_query_takes_the_top_surface() {
        let g = EngineGeometry {
            origin: [0; 3],
            vertices: vec![
                [0, 0, 100],
                [0, 512, 100],
                [512, 0, 100],
                [0, 0, 300],
                [0, 512, 300],
                [512, 0, 300],
            ],
            surfaces: vec![
                surface(0, [0, 0, 0x4000], -0x4000 * 100, [0, 1, 2]),
                surface(0, [0, 0, 0x4000], -0x4000 * 300, [3, 4, 5]),
            ],
            bounds: [[0, 0, 100], [512, 512, 300]],
        };
        let m =
            mandrake_tag::model_hit::ModelPlacement::new([0; 3], 0x4000, 0xc000, &g.hit_geometry());
        let h = |g: &EngineGeometry, x, y| {
            mandrake_tag::model_hit::get_model_height_at_position(
                &m,
                &g.hit_geometry(),
                x,
                y,
                mandrake_tag::model_hit::SurfaceTest::Gameplay,
            )
        };
        assert_eq!(h(&g, 100, 100), 304);
        assert_eq!(h(&g, 400, 400), -4096);
        assert_eq!(h(&g, 600, 100), -4096);
        let mut g2 = g.clone();
        g2.surfaces[1].flags = mandrake_tag::model::GEOMETRY_SURFACE_NOT_SOLID;
        assert_eq!(h(&g2, 100, 100), 112);
    }

    #[test]
    fn bounds_cells_rule() {
        assert_eq!(
            bounds_cells([[1024.0, 2048.0, 0.0], [512.0, 4096.0, 9.0]]),
            [[1, 4], [1, 7]]
        );
    }

    #[test]
    fn world_frame_round_trip() {
        let f = WorldFrame::new(320, 192);
        let e = [105216.0, 52480.0, 300.0];
        let p = f.from_engine(e);
        let back = f.to_engine(p);
        for k in 0..3 {
            assert!((back[k] - e[k]).abs() < 1e-2);
        }
    }

    #[test]
    fn from_geometry_does_not_overflow_i16_subtraction() {
        let g = EngineGeometry {
            origin: [i16::MIN, i16::MAX, i16::MIN],
            ..Default::default()
        };
        let p = g.from_geometry([i16::MAX, i16::MIN, i16::MAX]);
        assert_eq!(
            p[0],
            (f32::from(i16::MIN) - f32::from(i16::MAX)) / WORLD_ONE
        );
        assert_eq!(
            p[1],
            (f32::from(i16::MAX) - f32::from(i16::MIN)) / WORLD_ONE
        );
        assert_eq!(
            p[2],
            (f32::from(i16::MAX) - f32::from(i16::MIN)) / WORLD_ONE
        );
    }

    #[test]
    fn unmapped_surfaces_are_skipped_in_category2_reflections_and_region_cull() {
        let mut s_cat2 = surface(
            GEOMETRY_SURFACE_CATEGORY_2 | GEOMETRY_SURFACE_TWO_SIDED,
            [0, 0, 0x4000],
            0,
            [0, 1, 2],
        );
        s_cat2.has_material = false;
        let mut s_refl = surface(GEOMETRY_SURFACE_TWO_SIDED, [0, 0, 0x4000], 0, [0, 1, 2]);
        s_refl.has_material = false;
        let g = EngineGeometry {
            origin: [0; 3],
            vertices: vec![[0, 0, 512], [512, 0, 512], [0, 512, 512]],
            surfaces: vec![s_cat2, s_refl],
            bounds: [[0, 0, 0], [512, 512, 512]],
        };
        let inst = SurfaceInstance {
            model_matrix: Mat4::IDENTITY.cols,
            geometry: &g,
            texture_words: &[1],
        };
        let eye = [0.0, 5.0, 10.0];
        let view = Mat4::look_at_rh(eye, [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]);
        let proj = Mat4::perspective_rh_zo(1.0, 1.0, 0.5, 100.0);
        let vp = proj * view;

        let c2 = category2_draw_list(&[inst], &view, None);
        assert!(c2.indices.is_empty());

        let sine = [0i16; 1024];
        let height = |_x: i32, _y: i32| 0;
        let media = ReflectionMedia {
            media_height: &height,
            wobble: MediaWobble::default(),
            local_time: 0,
            sine: &sine,
            transparency: 0,
            frame: WorldFrame::new(10, 10),
        };
        let refl = reflection_triangles(&[(inst, &[], 0)], &view, eye, &media, None);
        assert!(refl.is_empty());

        let empty_region = crate::view_region::ViewRegion::default();
        let sr = SurfaceRegion {
            region: &empty_region,
            frame: WorldFrame::new(10, 10),
        };
        assert_eq!(region_culled_surfaces(&[inst], &view, &vp, &sr), (0, 0));
    }
}
