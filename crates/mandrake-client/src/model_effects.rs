use crate::model_instances::{ModelInstance, ModelMeshKey, ModelTagSource};
use mandrake_common::FourCC;
use mandrake_render::{ModelMesh, ModelRenderer, TranslucentTriangle};
use mandrake_sim::models::WorldModel;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, info};

pub(crate) struct EffectLayer {
    pub(crate) key: ModelMeshKey,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) triangles: Arc<Vec<TranslucentTriangle>>,
}

#[derive(Default)]
pub(crate) struct EffectDraw {
    pub(crate) first: Option<EffectLayer>,
    pub(crate) second: Option<EffectLayer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecordDraws<K = ModelMeshKey> {
    pub(crate) first: (K, u16),
    pub(crate) second: Option<(K, u16)>,
}

impl RecordDraws<i16> {
    pub(crate) fn of_record(
        next_permutation: i16,
        permutation: i16,
        transparency: u16,
        blend: u16,
    ) -> Self {
        Self {
            first: (next_permutation, transparency),
            second: (permutation != next_permutation).then_some((permutation, blend)),
        }
    }
}

impl<K: Copy> RecordDraws<K> {
    pub(crate) fn resolve<L>(self, mut find: impl FnMut(K) -> Option<L>) -> Option<RecordDraws<L>> {
        let first = (find(self.first.0)?, self.first.1);
        let second = self.second.and_then(|(k, t)| Some((find(k)?, t)));
        Some(RecordDraws { first, second })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RecordLook {
    next_permutation: i16,
    permutation: i16,
    blend: u16,
    transparency: u16,
    scale: i16,
    matrix: [i16; 9],
    mode: FourCC,
}

impl RecordLook {
    fn of(record: &WorldModel, mode: FourCC) -> Self {
        Self {
            next_permutation: record.next_permutation,
            permutation: record.permutation,
            blend: record.blend,
            transparency: record.transparency,
            scale: record.scale,
            matrix: record.placement.matrix,
            mode,
        }
    }
}

#[derive(Default)]
pub(crate) struct EffectMeshCache {
    keys: BTreeMap<(FourCC, i16), Option<ModelMeshKey>>,
    triangles: BTreeMap<ModelMeshKey, Arc<Vec<TranslucentTriangle>>>,
    surface_fades: BTreeMap<FourCC, Vec<u16>>,
}

pub(crate) struct EffectMeshes<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) renderer: &'a ModelRenderer,
    pub(crate) source: ModelTagSource<'a>,
    pub(crate) cache: &'a mut EffectMeshCache,
}

impl EffectMeshes<'_> {
    fn ensure(
        &mut self,
        meshes: &mut BTreeMap<ModelMeshKey, ModelMesh>,
        mode: FourCC,
        permutation: i16,
    ) -> Option<ModelMeshKey> {
        if let Some(&k) = self.cache.keys.get(&(mode, permutation)) {
            return k.filter(|k| meshes.contains_key(k));
        }
        let p = usize::try_from(permutation).unwrap_or(0);
        let (model, key) = self.source.model_key(mode, p);
        if meshes.contains_key(&key) {
            self.cache.keys.insert((mode, permutation), Some(key));
            return Some(key);
        }
        let fade = self.cache.surface_fades.get(&key.geometry);
        let built = self.source.build(model, key, p).map(|b| match fade {
            Some(t) => b.with_surface_transparency(t),
            None => b,
        });
        let got = built.map(|built| {
            let collection = self.source.collection(built.tex_tag);
            let mesh = built.mesh(self.device, self.queue, self.renderer, collection.as_ref());
            info!(
                "Model effect: loaded '{}' permutation {} (mode '{mode}')",
                key.geometry, key.permutation
            );
            meshes.insert(key, mesh);
            key
        });
        self.cache.keys.insert((mode, permutation), got);
        got
    }

    fn triangles(
        &mut self,
        key: ModelMeshKey,
        mode: FourCC,
        permutation: i16,
    ) -> Arc<Vec<TranslucentTriangle>> {
        if let Some(t) = self.cache.triangles.get(&key) {
            return t.clone();
        }
        let p = usize::try_from(permutation).unwrap_or(0);
        let (model, _) = self.source.model_key(mode, p);
        let fade = self.cache.surface_fades.get(&key.geometry);
        let tris = self
            .source
            .build(model, key, p)
            .map(|b| match fade {
                Some(t) => b.with_surface_transparency(t),
                None => b,
            })
            .map(|b| b.triangles(self.source.collection(b.tex_tag).as_ref()))
            .unwrap_or_default();
        let tris = Arc::new(tris);
        self.cache.triangles.insert(key, tris.clone());
        tris
    }

    fn layer(
        &mut self,
        meshes: &mut BTreeMap<ModelMeshKey, ModelMesh>,
        (mode, permutation, transparency): (FourCC, i16, u16),
        matrix: &[[f32; 4]; 4],
        shade: [u8; 8],
    ) -> Option<EffectLayer> {
        let key = self.ensure(meshes, mode, permutation)?;
        let opacity = mandrake_view::media::surface_opacity(transparency);
        let (_buffer, bind_group) =
            self.renderer
                .create_faded_bind_group(self.device, matrix, shade, opacity);
        Some(EffectLayer {
            key,
            bind_group,
            triangles: self.triangles(key, mode, permutation),
        })
    }
}

pub(crate) fn fade_surfaces(geom: &mut mandrake_tag::GeometryTag, transparency: &[u16]) {
    for (s, &t) in geom.surfaces.iter_mut().zip(transparency) {
        s.transparency = t;
    }
}

fn fade_changed(drawn: &[u16], now: &[mandrake_sim::models::SurfaceMaterial]) -> bool {
    drawn.len() != now.len() || drawn.iter().zip(now).any(|(&d, s)| d != s.transparency)
}

fn mesh_mode(
    instances: &[ModelInstance],
    cache: &EffectMeshCache,
    source: &ModelTagSource<'_>,
    key: ModelMeshKey,
) -> FourCC {
    if let Some((&(mode, _), _)) = cache.keys.iter().find(|(_, k)| **k == Some(key)) {
        return mode;
    }
    let mut candidates = Vec::new();
    for inst in instances {
        match &inst.animation {
            Some(a) => {
                for (f, k) in a.definition.frames.iter().zip(&a.frame_meshes) {
                    if *k == Some(key) {
                        candidates.push(f.model_tag);
                    }
                }
            }
            None => candidates.extend(inst.mode),
        }
    }
    candidates.dedup();
    candidates
        .into_iter()
        .find(|&m| source.model_key(m, key.permutation).1 == key)
        .unwrap_or(key.geometry)
}

pub(crate) fn follow_surface_fades(
    instances: &mut [ModelInstance],
    meshes: &mut BTreeMap<ModelMeshKey, ModelMesh>,
    world: &mandrake_sim::engine_rng::EngineWorld,
    loader: &mut EffectMeshes<'_>,
) {
    for (&tag, now) in &world.geometry_surfaces {
        let drawn = loader.cache.surface_fades.entry(tag).or_insert_with(|| {
            loader
                .source
                .find(b"geom", tag)
                .and_then(|d| mandrake_tag::GeometryTag::from_bytes(&d).ok())
                .map(|g| g.surfaces.iter().map(|s| s.transparency).collect())
                .unwrap_or_default()
        });
        if !fade_changed(drawn, now) {
            continue;
        }
        *drawn = now.iter().map(|s| s.transparency).collect();
        let transparency = drawn.clone();
        loader.cache.triangles.retain(|k, _| k.geometry != tag);
        let keys: Vec<ModelMeshKey> = meshes
            .keys()
            .filter(|k| k.geometry == tag)
            .copied()
            .collect();
        for key in keys {
            let mode = mesh_mode(instances, loader.cache, &loader.source, key);
            let (model, _) = loader.source.model_key(mode, key.permutation);
            let Some(built) = loader
                .source
                .build(model, key, key.permutation)
                .map(|b| b.with_surface_transparency(&transparency))
            else {
                continue;
            };
            let collection = loader.source.collection(built.tex_tag);
            let mesh = built.mesh(
                loader.device,
                loader.queue,
                loader.renderer,
                collection.as_ref(),
            );
            debug!(
                "Material fade: '{}' permutation {} (mode '{mode}') built again",
                key.geometry, key.permutation
            );
            meshes.insert(key, mesh);
        }
        for inst in instances.iter_mut() {
            let drawn_here = inst.mesh_key.geometry == tag
                || inst.record_draws.is_some_and(|d| {
                    d.first.0.geometry == tag || d.second.is_some_and(|s| s.0.geometry == tag)
                });
            if drawn_here {
                inst.record_look = None;
            }
        }
    }
}

pub(crate) fn record_model_matrix(
    matrix: &[i16; 9],
    scale: i16,
    translation: [f32; 4],
) -> [[f32; 4]; 4] {
    let mut m = mandrake_view::rotation::port_model_matrix(
        matrix,
        [translation[0], translation[1], translation[2]],
    );
    let s = f32::from(scale) / 512.0;
    for col in m.iter_mut().take(3) {
        for v in col.iter_mut().take(3) {
            *v *= s;
        }
    }
    m
}

pub(crate) fn link_sim_records(
    instances: &mut [ModelInstance],
    world: &mandrake_sim::engine_rng::EngineWorld,
) {
    let mut taken = vec![false; world.models.len()];
    for inst in instances.iter_mut().filter(|i| i.animation.is_none()) {
        let (position, _, _) = inst.marker_placement;
        inst.record =
            world.models.iter().enumerate().position(|(i, r)| {
                !taken[i] && r.animation == -1 && r.placement.position == position
            });
        if let Some(r) = inst.record {
            taken[r] = true;
        }
    }
}

fn released_cells(
    cells: &mandrake_sim::MeshCells,
    bounds: [[i32; 3]; 2],
) -> impl Iterator<Item = usize> + '_ {
    let (b0, b1) = (bounds[0], bounds[1]);
    let x0 = (b0[0].min(b1[0]) >> 9) as i16;
    let x1 = ((b0[0].max(b1[0]) - 1) >> 9) as i16;
    let y0 = (b0[1].min(b1[1]) >> 9) as i16;
    let y1 = ((b0[1].max(b1[1]) - 1) >> 9) as i16;
    (y0..=y1)
        .flat_map(move |y| (x0..=x1).map(move |x| cells.cell_index(i32::from(x), i32::from(y))))
        .filter(|&i| cells.models.get(i) == Some(&-1))
}

fn release_deleted_cells(
    cells: &mandrake_sim::MeshCells,
    bounds: [[i32; 3]; 2],
    map: &mut Option<Arc<crate::media::MapMedia>>,
) -> bool {
    let Some(arc) = map.as_mut() else {
        return false;
    };
    let model_cells_count =
        arc.surface.mesh_flags & mandrake_view::media::MESH_FLAG_MODEL_CELLS_NOT_MEDIA != 0;
    let mut changed = false;
    for i in released_cells(cells, bounds) {
        if arc.cell_models.get(i).is_some_and(|&m| m != -1) {
            Arc::make_mut(arc).cell_models[i] = -1;
            changed |= model_cells_count;
        }
    }
    changed
}

pub(crate) fn follow_sim_model_effects(
    instances: &mut [ModelInstance],
    meshes: &mut BTreeMap<ModelMeshKey, ModelMesh>,
    sim: &mandrake_sim::Simulation,
    loader: &mut EffectMeshes<'_>,
    map: &mut Option<Arc<crate::media::MapMedia>>,
) -> bool {
    let world = &sim.engine;
    let mut landscape_changed = false;
    follow_surface_fades(instances, meshes, world, loader);
    if instances.iter().any(|i| !i.record_linked) {
        link_sim_records(instances, world);
        for inst in instances.iter_mut() {
            inst.record_linked = true;
        }
    }
    for inst in instances.iter_mut() {
        let (record, mode) = match inst.animation.as_ref() {
            Some(a) => {
                let mode = a
                    .definition
                    .frames
                    .get(a.playback.frame)
                    .map(|f| f.model_tag);
                (usize::try_from(a.sim_model).ok(), mode)
            }
            None => (inst.record, inst.mode),
        };
        let (Some(record), Some(mode)) = (record.and_then(|r| world.models.get(r)), mode) else {
            continue;
        };
        if record.identifier == 0 {
            inst.present = false;
            inst.effect = None;
            inst.record_draws = None;
            if inst.animation.is_none() && !inst.cells_released {
                inst.cells_released = true;
                landscape_changed |=
                    release_deleted_cells(&world.cells, record.placement.bounds, map);
            }
            continue;
        }
        let look = RecordLook::of(record, mode);
        if inst.record_look == Some(look) && inst.followed_key == Some(inst.mesh_key) {
            continue;
        }
        let blending = look.permutation != look.next_permutation;
        let first_permutation = if blending && look.blend == 0 {
            look.permutation
        } else {
            look.next_permutation
        };
        let Some(first_key) = loader.ensure(meshes, mode, first_permutation) else {
            inst.record_look = Some(look);
            inst.record_draws = None;
            continue;
        };
        inst.record_draws = RecordDraws::of_record(
            look.next_permutation,
            look.permutation,
            look.transparency,
            look.blend,
        )
        .resolve(|p| loader.ensure(meshes, mode, p));
        let (_, yaw, pitch) = inst.marker_placement;
        let as_placed = !blending
            && look.transparency == 0
            && look.scale == 0x200
            && look.matrix == mandrake_tag::model_hit::model_world_matrix(yaw, pitch);
        inst.record_look = Some(look);
        inst.followed_key = Some(first_key);
        if as_placed && inst.effect.is_none() && !inst.moved {
            inst.mesh_key = first_key;
            continue;
        }
        let matrix =
            record_model_matrix(&record.placement.matrix, record.scale, inst.model_matrix[3]);
        let shade = inst.shade;
        let mut effect = EffectDraw::default();
        if look.transparency != 0 {
            effect.first = loader.layer(
                meshes,
                (mode, first_permutation, look.transparency),
                &matrix,
                shade,
            );
        }
        if blending && look.blend != 0 {
            effect.second =
                loader.layer(meshes, (mode, look.permutation, look.blend), &matrix, shade);
        }
        if effect.first.is_some() || effect.second.is_some() {
            debug!(
                "Model effect: record at {:?}, permutation {} over {} at blend {:#06x}, transparency {:#06x}",
                record.placement.position,
                look.permutation,
                look.next_permutation,
                look.blend,
                look.transparency
            );
        }
        inst.mesh_key = first_key;
        inst.followed_key = Some(first_key);
        inst.set_matrix(loader.renderer, loader.device, matrix);
        inst.moved = !as_placed;
        inst.effect = (effect.first.is_some() || effect.second.is_some()).then_some(effect);
    }
    landscape_changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_cells_are_the_sims_cleared_bounds() {
        let full = mandrake_sim::MeshCells {
            width: 8,
            height: 6,
            flags: vec![0; 48],
            models: vec![3; 48],
        };
        for bounds in [
            [[1100, 600, 0], [2048, 1900, 90]],
            [[2048, 1900, 90], [1100, 600, 0]],
            [[-300, 2500, 0], [700, 9000, 0]],
        ] {
            let mut cells = full.clone();
            mandrake_sim::models::clear_model_bounds_between(&mut cells, bounds[0], bounds[1]);
            let want: Vec<usize> = (0..48).filter(|&i| cells.models[i] == -1).collect();
            let mut got: Vec<usize> = released_cells(&cells, bounds).collect();
            got.sort_unstable();
            got.dedup();
            assert_eq!(got, want, "{bounds:?}");
            assert!(released_cells(&full, bounds).next().is_none());
        }
    }

    #[test]
    fn a_record_draws_its_next_permutation_then_its_blend() {
        let still = RecordDraws::of_record(2, 2, 0x4000, 0x1234);
        assert_eq!((still.first, still.second), ((2, 0x4000), None));
        let blending = RecordDraws::of_record(4, 0, 0, 0x8000);
        assert_eq!(
            (blending.first, blending.second),
            ((4, 0), Some((0, 0x8000)))
        );
        let named = |p: i16| (p != 3).then(|| format!("p{p}"));
        let both = blending.resolve(named).unwrap();
        assert_eq!(both.first, ("p4".to_string(), 0));
        assert_eq!(both.second, Some(("p0".to_string(), 0x8000)));
        let no_second = RecordDraws::of_record(4, 3, 0, 0x8000)
            .resolve(named)
            .unwrap();
        assert_eq!(no_second.second, None);
        assert!(RecordDraws::of_record(3, 4, 0, 0).resolve(named).is_none());
    }

    #[test]
    fn a_material_fade_draws_that_materials_surfaces_translucent() {
        use mandrake_tag::model::{
            GeometryMaterial, GeometrySurface, GeometryVertex, SurfaceVertex,
        };
        let sv = |u, v, i| SurfaceVertex {
            u,
            v,
            vertex_index: i,
            flags: 0,
        };
        let surface = |material_index| GeometrySurface {
            flags: 0,
            normal: (0, 0, 512),
            distance: 0,
            transparency: 0,
            material_index,
            vertices: [sv(0, 0, 0), sv(65535, 0, 1), sv(0, 65535, 2)],
            draw_after_list: -1,
        };
        let material = |name: &str| GeometryMaterial {
            name: name.into(),
            texture_index: 0,
            flags: 0,
        };
        let geom = mandrake_tag::GeometryTag {
            flags: 0,
            texture_tag: FourCC::new(*b"wite"),
            origin: (0, 0, 0),
            material_count: 2,
            vertex_count: 3,
            surface_count: 3,
            materials: vec![material("wall"), material("glow")],
            vertices: vec![
                GeometryVertex { x: 0, y: 0, z: 0 },
                GeometryVertex { x: 512, y: 0, z: 0 },
                GeometryVertex { x: 0, y: 512, z: 0 },
            ],
            surfaces: vec![surface(1), surface(0), surface(1)],
            draw_after_lists: Vec::new(),
            bounds: [[0; 3]; 2],
        };
        let mut sim = mandrake_sim::models::surface_materials(&geom);
        let drawn: Vec<u16> = geom.surfaces.iter().map(|s| s.transparency).collect();
        assert!(!fade_changed(&drawn, &sim));
        for s in sim.iter_mut().filter(|s| s.material == 1) {
            s.transparency = 0x8000;
        }
        assert!(fade_changed(&drawn, &sim));
        let now: Vec<u16> = sim.iter().map(|s| s.transparency).collect();
        let mut faded = geom.clone();
        fade_surfaces(&mut faded, &now);

        let (_, _, before) = ModelMesh::build_geometry_buffers_with_frames(&geom, None);
        assert!(before.iter().all(|s| !s.translucent));
        let (verts, indices, submeshes) =
            ModelMesh::build_geometry_buffers_with_frames(&faded, None);
        let opaque: u32 = submeshes
            .iter()
            .filter(|s| !s.translucent)
            .map(|s| s.index_count)
            .sum();
        let translucent: Vec<_> = submeshes.iter().filter(|s| s.translucent).collect();
        assert_eq!(opaque, 3, "material 0's surface");
        assert_eq!(translucent.iter().map(|s| s.index_count).sum::<u32>(), 6);
        let alpha = f32::from(((0xffff - 0x8000u32) >> 8) as u8) / 255.0;
        for s in &translucent {
            for &i in &indices[s.index_offset as usize..(s.index_offset + s.index_count) as usize] {
                assert!((verts[i as usize].alpha - alpha).abs() < 1e-6);
            }
        }
        for s in sim.iter_mut().filter(|s| s.material == 1) {
            s.transparency = 0xffff;
        }
        let end: Vec<u16> = sim.iter().map(|s| s.transparency).collect();
        fade_surfaces(&mut faded, &end);
        let (verts, _, submeshes) = ModelMesh::build_geometry_buffers_with_frames(&faded, None);
        let tris = TranslucentTriangle::collect(&verts, &indices, &submeshes);
        assert_eq!(tris.len(), 2);
    }

    #[test]
    fn a_record_matrix_scales_its_rotation() {
        let world = mandrake_tag::model_hit::model_world_matrix(0x4000, 0xc000);
        let at = [1.0, 2.0, 3.0, 1.0];
        let port = mandrake_view::rotation::port_model_matrix(&world, [1.0, 2.0, 3.0]);
        assert_eq!(record_model_matrix(&world, 0x200, at), port);
        let scaled = record_model_matrix(&world, 0x400, at);
        for c in 0..3 {
            for r in 0..3 {
                assert_eq!(scaled[c][r], port[c][r] * 2.0);
            }
        }
        assert_eq!(scaled[3], port[3]);
    }
}
