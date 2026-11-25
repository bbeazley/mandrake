use crate::draw_site;
use crate::engine_rng::Unmodelled;
use crate::passability::{CELL_KILLED, MeshCells};

use mandrake_tag::model_hit::{HitGeometry, ModelHit, ModelPlacement, SurfaceTest};

pub fn rotation_matrix(yaw: u16, pitch: u16) -> [i16; 9] {
    mandrake_tag::model_hit::build_rotation_matrix(yaw, pitch)
}

pub fn model_matrix(r: &[i16; 9]) -> [i16; 9] {
    mandrake_tag::model_hit::build_rotation_matrix3d([r[0], r[3], r[6]], [r[2], r[5], r[8]])
        .map(|v| v >> 5)
}

pub fn transform_point(
    position: (i32, i32, i32),
    origin: (i16, i16, i16),
    point: (i16, i16, i16),
    m: &[i16; 9],
) -> (i32, i32, i32) {
    let w = mandrake_tag::model_hit::transform_geometry_point_to_world(
        [position.0, position.1, position.2],
        [origin.0, origin.1, origin.2],
        [point.0, point.1, point.2],
        m,
    );
    (w[0], w[1], w[2])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectHitModelRun {
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub offset: i32,
    pub found: Option<(i32, [i16; 3], i16, usize)>,
    pub hit: Option<ModelHit>,
}

thread_local! {
    pub static OBJECT_HIT_MODEL_LOG: std::cell::RefCell<Option<Vec<ObjectHitModelRun>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Clone, Debug)]
pub struct WorldModel {
    pub identifier: i16,
    pub permutation: i16,
    pub next_permutation: i16,
    pub blend: u16,
    pub transparency: u16,
    pub scale: i16,
    pub yaw: u16,
    pub pitch: u16,
    pub shape: std::sync::Arc<ModelShape>,
    pub placement: ModelPlacement,
    pub geometry: std::sync::Arc<HitGeometry>,
    pub animated: bool,
    pub animation: i16,
    pub geometry_tag: Option<mandrake_common::FourCC>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceMaterial {
    pub material: i16,
    pub transparency: u16,
}

pub fn surface_materials(geom: &mandrake_tag::model::GeometryTag) -> Vec<SurfaceMaterial> {
    geom.surfaces
        .iter()
        .map(|s| SurfaceMaterial {
            material: s.material_index,
            transparency: s.transparency,
        })
        .collect()
}

impl WorldModel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identifier: i16,
        position: [i32; 3],
        yaw: u16,
        pitch: u16,
        permutation: i16,
        geometry: std::sync::Arc<HitGeometry>,
        shape: impl Into<std::sync::Arc<ModelShape>>,
        animated: bool,
        animation: i16,
    ) -> Self {
        let shape = shape.into();
        Self {
            identifier,
            permutation,
            next_permutation: permutation,
            blend: 0,
            transparency: 0,
            scale: 0x200,
            yaw,
            pitch,
            shape,
            placement: ModelPlacement::new(position, yaw, pitch, &geometry),
            geometry,
            animated,
            animation,
            geometry_tag: None,
        }
    }
}

impl crate::engine_rng::EngineWorld {
    pub fn new_model_identifier(&mut self) -> i16 {
        let identifier = self.model_datums | 0x8000;
        self.model_datums = self.model_datums.wrapping_add(1) & 0x7fff;
        identifier as i16
    }

    pub fn set_model_identifier(&mut self, index: usize, wanted: i16) {
        if wanted == 0 {
            return;
        }
        let counter = (self.model_datums | 0x8000) as i16;
        let mut id = wanted;
        for (k, m) in self.models.iter().enumerate() {
            if k != index && m.identifier != 0 && m.identifier == id {
                id = counter;
            }
        }
        if let Some(m) = self.models.get_mut(index) {
            m.identifier = id;
        }
    }

    pub fn model_identifier_to_model_index(&self, identifier: i16) -> i16 {
        self.models
            .iter()
            .position(|m| m.identifier != 0 && m.identifier == identifier)
            .map_or(-1, |k| k as i16)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModelGrid {
    width: i32,
    height: i32,
    cells: Vec<i16>,
    models: Vec<WorldModel>,
}

const POINT_TEST: SurfaceTest = SurfaceTest::Gameplay;

fn model_height(m: &WorldModel, x: i32, y: i32) -> i16 {
    mandrake_tag::model_hit::get_model_height_at_position(
        &m.placement,
        &m.geometry,
        x,
        y,
        POINT_TEST,
    )
}

pub fn model_height_at(
    cells: &MeshCells,
    models: &[WorldModel],
    x: i32,
    y: i32,
    site: u32,
) -> Result<Option<i16>, Unmodelled> {
    let (cx, cy) = (x >> 9, y >> 9);
    if cx < 0 || cy < 0 || cx >= cells.width || cy >= cells.height {
        return Ok(None);
    }
    let index = cells
        .models
        .get((cy * cells.width + cx) as usize)
        .copied()
        .unwrap_or(-1);
    if index == -1 {
        return Ok(None);
    }
    Ok(Some(model_height(model_record(models, index, site)?, x, y)))
}

fn model_record(models: &[WorldModel], index: i16, site: u32) -> Result<&WorldModel, Unmodelled> {
    models
        .get(index as usize)
        .ok_or_else(|| Unmodelled::new(site, format!("no record of model {index}")))
}

impl ModelGrid {
    pub fn new(cells: &MeshCells, models: &[WorldModel]) -> Self {
        Self {
            width: cells.width,
            height: cells.height,
            cells: cells.models.clone(),
            models: models.to_vec(),
        }
    }

    fn cell(&self, x: i32, y: i32) -> i16 {
        self.cells
            .get((y * self.width + x) as usize)
            .copied()
            .unwrap_or(-1)
    }

    fn model(&self, index: i16, site: u32) -> Result<&WorldModel, Unmodelled> {
        model_record(&self.models, index, site)
    }

    pub fn height_at(&self, x: i32, y: i32) -> Result<Option<i16>, Unmodelled> {
        let index = self.cell(x >> 9, y >> 9);
        if index == -1 {
            return Ok(None);
        }
        let m = self.model(index, draw_site::MODEL_HEIGHT)?;
        Ok(Some(model_height(m, x, y)))
    }

    fn segment_models(&self, a: [i32; 3], b: [i32; 3]) -> Vec<i16> {
        let x0 = (a[0].min(b[0]) >> 9).max(0);
        let y0 = (a[1].min(b[1]) >> 9).max(0);
        let x1 = ((a[0].max(b[0]) + 0x1ff) >> 9).min(self.width - 1);
        let y1 = ((a[1].max(b[1]) + 0x1ff) >> 9).min(self.height - 1);
        let mut found = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let m = self.cell(x, y);
                if m != -1 && !found.contains(&m) {
                    found.push(m);
                }
            }
        }
        found
    }

    pub fn object_hit_model(
        &self,
        a: [i32; 3],
        b: [i32; 3],
        offset: i32,
    ) -> Result<Option<ModelHit>, Unmodelled> {
        let indices = self.segment_models(a, b);
        if indices.is_empty() {
            return Ok(None);
        }
        let mut candidates = Vec::with_capacity(indices.len());
        for &i in &indices {
            candidates.push(self.model(i, draw_site::OBJECT_HIT_MODEL)?);
        }
        let found = mandrake_tag::model_hit::nearest_model_surface(
            candidates.iter().map(|m| (&m.placement, &*m.geometry)),
            a,
            b,
            POINT_TEST,
        );
        let hit = found.as_ref().map(|f| {
            let m = candidates[f.model];
            mandrake_tag::model_hit::object_hit_model_point(
                &m.placement,
                &m.geometry,
                f,
                a,
                b,
                offset,
            )
        });
        OBJECT_HIT_MODEL_LOG.with(|l| {
            if let Some(l) = l.borrow_mut().as_mut() {
                l.push(ObjectHitModelRun {
                    from: a,
                    to: b,
                    offset,
                    found: found.map(|f| (f.t, f.p0, indices[f.model], f.surface)),
                    hit,
                });
            }
        });
        Ok(hit)
    }

    pub fn object_hit_model_old(
        &self,
        a: [i32; 3],
        b: [i32; 3],
        offset: i32,
    ) -> Result<Option<ModelHit>, Unmodelled> {
        let on_map = |x: i32, y: i32| {
            let (x, y) = (x as i16, y as i16);
            x >= 0 && i32::from(x) < self.width && y >= 0 && i32::from(y) < self.height
        };
        let test = |index: i16, site: u32| -> Result<Option<ModelHit>, Unmodelled> {
            let m = self.model(index, site)?;
            Ok(mandrake_tag::model_hit::object_hit_model_index(
                &m.placement,
                &m.geometry,
                a,
                b,
                POINT_TEST,
                offset,
            ))
        };
        let (ax, ay) = (a[0] >> 9, a[1] >> 9);
        let (bx, by) = (b[0] >> 9, b[1] >> 9);
        let first = if on_map(ax, ay) {
            self.cell(ax, ay)
        } else {
            -1
        };
        if first != -1
            && let Some(h) = test(first, draw_site::OBJECT_HIT_MODEL_FIRST)?
        {
            return Ok(Some(h));
        }
        if (ax, ay) == (bx, by) || !on_map(bx, by) {
            return Ok(None);
        }
        let second = self.cell(bx, by);
        if second == -1 || second == first {
            return Ok(None);
        }
        test(second, draw_site::OBJECT_HIT_MODEL_SECOND)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelShape {
    pub cells: Vec<(i16, i16, u16)>,
    pub origin: (i16, i16, i16),
    pub bounds: [(i16, i16, i16); 2],
    pub ground_vertices: Vec<(i16, i16, i16)>,
    pub permutation_count: i16,
}

impl ModelShape {
    pub fn from_tags(mode: &[u8], geom: &[u8]) -> Result<Self, String> {
        let be16 = |d: &[u8], o: usize| -> Result<i16, String> {
            d.get(o..o + 2)
                .map(|b| i16::from_be_bytes([b[0], b[1]]))
                .ok_or_else(|| format!("tag too short at {o:#x}"))
        };
        let be32 = |d: &[u8], o: usize| -> Result<usize, String> {
            d.get(o..o + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
                .ok_or_else(|| format!("tag too short at {o:#x}"))
        };
        let n = be16(mode, 0x0e)?.max(0) as usize;
        let base = be32(mode, 0x34)? + be32(mode, 0x28)?;
        let mut cells = Vec::with_capacity(n);
        for i in 0..n {
            let o = base + 8 * i;
            cells.push((
                be16(mode, o)?,
                be16(mode, o + 2)?,
                be16(mode, o + 4)? as u16,
            ));
        }
        let p = |o: usize| -> Result<(i16, i16, i16), String> {
            Ok((be16(geom, o)?, be16(geom, o + 2)?, be16(geom, o + 4)?))
        };
        let count = be16(mode, 0x0a)?.max(0) as usize;
        let words = be32(mode, 0x34)? + be32(mode, 0x10)?;
        let vertices = 0x80 + be32(geom, 0x28)?;
        let mut ground_vertices = Vec::new();
        for i in 0..count {
            let flag = mode
                .get(words + 4 * i + 3)
                .ok_or_else(|| format!("mode vertex word {i} past the tag"))?;
            if flag & 1 != 0 {
                ground_vertices.push(p(vertices + 6 * i)?);
            }
        }
        Ok(Self {
            cells,
            origin: p(0x10)?,
            bounds: [p(0x58)?, p(0x5e)?],
            ground_vertices,
            permutation_count: be16(mode, 0x0c)?,
        })
    }
}

fn disable_cell_media_animation(cells: &mut MeshCells, x: i32, y: i32) {
    for cx in x..=x + 1 {
        for cy in y..=y + 1 {
            let i = cells.cell_index(cx, cy);
            cells.flags[i] &= !0x400;
        }
    }
}

pub fn new_model_cells_for_version(
    cells: &mut MeshCells,
    index: i16,
    position: (i32, i32, i32),
    yaw: u16,
    pitch: u16,
    shape: &ModelShape,
    gameplay: i32,
) -> NewModelCells {
    let m = model_matrix(&rotation_matrix(yaw, pitch));
    let mut changed = Vec::new();
    for &(gx, gy, flags) in &shape.cells {
        let p = (
            ((i32::from(gx) << 9) as i16).wrapping_add(0x100),
            ((i32::from(gy) << 9) as i16).wrapping_add(0x100),
            0,
        );
        let w = transform_point(position, shape.origin, p, &m);
        let (cx, cy) = (w.0 >> 9, w.1 >> 9);
        let i = cells.cell_index(cx, cy);
        let low: u16 = if flags & 4 != 0 { 0xff } else { 0xee };
        if flags & 1 != 0 {
            if gameplay > crate::engine_rng::GAMEPLAY_VERSION_1_8_3 {
                disable_cell_media_animation(cells, cx, cy);
            }
            cells.flags[i] = ((cells.flags[i] | CELL_KILLED) & 0xff00) | low;
            changed.push((cx, cy));
        } else if flags & 6 != 0 {
            cells.flags[i] = (cells.flags[i] & 0xff00) | low;
            changed.push((cx, cy));
        }
    }
    let b0 = transform_point(position, shape.origin, shape.bounds[0], &m);
    let b1 = transform_point(position, shape.origin, shape.bounds[1], &m);
    let x0 = ((b0.0.min(b1.0)) as u32 >> 9) as i32;
    let x1 = (((b0.0.max(b1.0)) - 1) as u32 >> 9) as i32;
    let y0 = (b0.1.min(b1.1) >> 9) as i16;
    let y1 = ((b0.1.max(b1.1) - 1) >> 9) as i16;
    let mut taken = false;
    for y in y0..=y1 {
        for x in (x0 as i16)..=(x1 as i16) {
            let i = cells.cell_index(i32::from(x), i32::from(y));
            if cells.models[i] == -1 {
                cells.models[i] = index;
            } else {
                taken = true;
            }
        }
    }
    NewModelCells {
        changed,
        overlapped: taken,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewModelCells {
    pub changed: Vec<(i32, i32)>,
    pub overlapped: bool,
}

pub fn new_model_heights(
    cells: &MeshCells,
    heights: &mut [(i16, i16)],
    position: (i32, i32, i32),
    yaw: u16,
    pitch: u16,
    shape: &ModelShape,
) -> bool {
    let m = model_matrix(&rotation_matrix(yaw, pitch));
    let mut changed = false;
    for &v in &shape.ground_vertices {
        let w = transform_point(position, shape.origin, v, &m);
        if w.0 & 0x1ff != 0 || w.1 & 0x1ff != 0 {
            continue;
        }
        if let Some(h) = heights.get_mut(cells.cell_index(w.0 >> 9, w.1 >> 9)) {
            changed |= h.0 != w.2 as i16;
            h.0 = w.2 as i16;
        }
    }
    changed
}

pub fn delete_model_cell(
    cells: &mut MeshCells,
    position: (i32, i32, i32),
    yaw: u16,
    pitch: u16,
    shape: &ModelShape,
    entry: (i16, i16, u16),
) -> Option<(i32, i32)> {
    let m = model_matrix(&rotation_matrix(yaw, pitch));
    delete_model_cell_with_matrix(cells, position, &m, shape, entry)
}

pub fn delete_model_cell_with_matrix(
    cells: &mut MeshCells,
    position: (i32, i32, i32),
    m: &[i16; 9],
    shape: &ModelShape,
    (gx, gy, flags): (i16, i16, u16),
) -> Option<(i32, i32)> {
    let p = (
        ((i32::from(gx) << 9) as i16).wrapping_add(0x100),
        ((i32::from(gy) << 9) as i16).wrapping_add(0x100),
        0,
    );
    let w = transform_point(position, shape.origin, p, m);
    let (cx, cy) = (w.0 >> 9, w.1 >> 9);
    let i = cells.cell_index(cx, cy);
    if flags & 1 != 0 {
        cells.flags[i] = (cells.flags[i] & !CELL_KILLED & 0xff00) | 0x66;
    } else if flags & 6 != 0 {
        cells.flags[i] = (cells.flags[i] & 0xff00) | 0x66;
    } else {
        return None;
    }
    Some((cx, cy))
}

pub fn clear_model_bounds(
    cells: &mut MeshCells,
    position: (i32, i32, i32),
    yaw: u16,
    pitch: u16,
    shape: &ModelShape,
) {
    let m = model_matrix(&rotation_matrix(yaw, pitch));
    let b0 = transform_point(position, shape.origin, shape.bounds[0], &m);
    let b1 = transform_point(position, shape.origin, shape.bounds[1], &m);
    clear_model_bounds_between(cells, [b0.0, b0.1, b0.2], [b1.0, b1.1, b1.2]);
}

pub fn clear_model_bounds_between(cells: &mut MeshCells, b0: [i32; 3], b1: [i32; 3]) {
    let (b0, b1) = ((b0[0], b0[1]), (b1[0], b1[1]));
    let x0 = ((b0.0.min(b1.0)) >> 9) as i16;
    let x1 = (((b0.0.max(b1.0)) - 1) >> 9) as i16;
    let y0 = (b0.1.min(b1.1) >> 9) as i16;
    let y1 = ((b0.1.max(b1.1) - 1) >> 9) as i16;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = cells.cell_index(i32::from(x), i32::from(y));
            cells.models[i] = -1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_tag::model_hit::HitSurface;
    use std::sync::Arc;

    #[test]
    fn a_ground_vertex_on_a_cell_corner_sets_that_cells_height() {
        let cells = MeshCells {
            width: 4,
            height: 4,
            flags: vec![0; 16],
            models: vec![-1; 16],
        };
        let mut heights = vec![(0i16, 0i16); 16];
        let shape = ModelShape {
            cells: Vec::new(),
            origin: (0, 0, 0),
            bounds: [(0, 0, 0), (512, 512, 0)],
            ground_vertices: vec![(0, 0, -3), (0x100, 0x80, 7)],
            permutation_count: 1,
        };
        let (yaw, pitch) = (0, 0xc000);
        let m = model_matrix(&rotation_matrix(yaw, pitch));
        let d = transform_point((0, 0, 0), shape.origin, shape.ground_vertices[0], &m);
        let position = (1024 - d.0, 512 - d.1, 0);
        let on = transform_point(position, shape.origin, shape.ground_vertices[0], &m);
        assert_eq!((on.0, on.1), (1024, 512));
        let off = transform_point(position, shape.origin, shape.ground_vertices[1], &m);
        assert_ne!((off.0 & 0x1ff, off.1 & 0x1ff), (0, 0));
        assert!(new_model_heights(
            &cells,
            &mut heights,
            position,
            yaw,
            pitch,
            &shape
        ));
        let mut want = vec![(0i16, 0i16); 16];
        want[4 + 2] = (on.2 as i16, 0);
        assert_ne!(on.2, 0);
        assert_eq!(heights, want);
    }

    fn grid(animated: bool) -> ModelGrid {
        let g = HitGeometry {
            origin: [0, 0, 0],
            vertices: vec![[0, 0, 0], [512, 0, 0], [512, 512, 0], [0, 512, 0]],
            surfaces: [[0u16, 2, 1], [0, 3, 2]]
                .map(|vertices| HitSurface {
                    flags: 0,
                    normal: [0, 0, 0x4000],
                    distance: 0,
                    vertices,
                })
                .to_vec(),
            bounds: [[0, 0, -64], [512, 512, 64]],
        };
        let placement = ModelPlacement::new([512, 512, 0], 0x4000, 0xc000, &g);
        let mut cells = vec![-1; 16];
        cells[4 + 1] = 0;
        ModelGrid {
            width: 4,
            height: 4,
            cells,
            models: vec![WorldModel {
                placement,
                ..WorldModel::new(
                    1,
                    [512, 512, 0],
                    0x4000,
                    0xc000,
                    0,
                    Arc::new(g),
                    ModelShape {
                        cells: Vec::new(),
                        origin: (0, 0, 0),
                        bounds: [(0, 0, -64), (512, 512, 64)],
                        ground_vertices: Vec::new(),
                        permutation_count: 1,
                    },
                    animated,
                    -1,
                )
            }],
        }
    }

    #[test]
    fn segment_models_widen_the_box_by_the_rounded_up_end() {
        let g = grid(false);
        assert_eq!(g.segment_models([100, 100, 0], [510, 510, 0]), [0]);
        assert!(g.segment_models([-100, -100, 0], [0, 0, 0]).is_empty());
        assert_eq!(g.segment_models([-900, -900, 0], [9000, 9000, 0]), [0]);
    }

    #[test]
    fn object_hit_model_old_and_new_find_the_floor() {
        let g = grid(false);
        let (a, b) = ([700, 700, 50], [800, 800, -50]);
        let new = g.object_hit_model(a, b, 0).unwrap().expect("hit");
        assert_eq!(new.normal, [0, 0, 0x4000]);
        let old = g.object_hit_model_old(a, b, 0).unwrap().expect("hit");
        assert_eq!(old.point, [750, 750, 0]);
        assert_eq!(g.object_hit_model(a, [800, 800, 10], 0).unwrap(), None);
        assert_eq!(g.object_hit_model_old(a, [800, 800, 10], 0).unwrap(), None);
    }

    #[test]
    fn heights_and_animated_models() {
        for animated in [false, true] {
            let g = grid(animated);
            assert_eq!(g.height_at(700, 700).unwrap(), Some(0));
            assert_eq!(g.height_at(100, 100).unwrap(), None);
            assert!(
                g.object_hit_model([700, 700, 50], [800, 800, -50], 0)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn identity_marker_gives_the_unit_world_matrix() {
        let r = rotation_matrix(0, 0xc000);
        assert_eq!(r, [0, 0x4000, 0, -0x4000, 0, 0, 0, 0, 0x4000]);
        let m = model_matrix(&r);
        assert_eq!(m, [0, 0x200, 0, -0x200, 0, 0, 0, 0, 0x200]);
        assert_eq!(m, mandrake_tag::model_hit::model_world_matrix(0, 0xc000));
    }
}
