use crate::map_load::MODEL_ARCHIVES;
use crate::media;
use mandrake_common::FourCC;
use mandrake_render::{
    ModelMesh, ModelRenderer, TranslucentBatch, TranslucentTriangle,
    sort_translucent_triangles_on_screen,
};
use mandrake_sim::Simulation;
use mandrake_tag::collection::CollectionTag;
use mandrake_tag::model_anim::ModelAnimationShadowMaps;
use mandrake_tag::{
    GeometryTag, MarkerEntry, MeshTag, ModelAnimationPlayback, ModelAnimationTag, ModelTag,
    TagArchive, WorldPoint,
};
use mandrake_view::landscape::{OverlayMode, PixelRect, ShadeOverlay};
use mandrake_view::{
    camera::{Camera, Mat4},
    model::ModelVertex,
};
use std::collections::BTreeMap;
use tracing::{debug, info, trace, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ModelMeshKey {
    pub(crate) geometry: FourCC,
    pub(crate) permutation: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct ModelTagSource<'a> {
    pub(crate) small: &'a TagArchive,
    pub(crate) large: Option<&'a TagArchive>,
}

pub(crate) struct ModelBuild {
    pub(crate) key: ModelMeshKey,
    model: Option<ModelTag>,
    marker_permutation: usize,
    pub(crate) geom: GeometryTag,
    pub(crate) tex_tag: FourCC,
}

impl ModelTagSource<'_> {
    pub(crate) fn find(&self, group: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
        std::iter::once(self.small)
            .chain(MODEL_ARCHIVES.get().into_iter().flatten())
            .chain(self.large)
            .find_map(|a| {
                a.find_tag_by_id(FourCC::new(*group), id)
                    .and_then(|e| a.get_tag_data(e).ok())
            })
    }

    pub(crate) fn collection(&self, tex_tag: FourCC) -> Option<CollectionTag> {
        self.large
            .and_then(|la| {
                la.find_tag_by_id(FourCC::new(*b".256"), tex_tag)
                    .and_then(|e| la.get_tag_data(e).ok())
            })
            .or_else(|| {
                self.small
                    .find_tag_by_id(FourCC::new(*b".256"), tex_tag)
                    .and_then(|e| self.small.get_tag_data(e).ok())
            })
            .and_then(|d| CollectionTag::from_bytes(&d).ok())
    }

    pub(crate) fn model_key(
        &self,
        model_id: FourCC,
        marker_permutation: usize,
    ) -> (Option<ModelTag>, ModelMeshKey) {
        let model = self
            .find(b"mode", model_id)
            .and_then(|d| ModelTag::from_bytes(&d).ok());
        let key = ModelMeshKey {
            geometry: model.as_ref().map_or(model_id, |m| m.geometry_tag),
            permutation: model
                .as_ref()
                .map_or(0, |m| m.permutation_index(marker_permutation)),
        };
        (model, key)
    }

    pub(crate) fn build(
        &self,
        model: Option<ModelTag>,
        key: ModelMeshKey,
        marker_permutation: usize,
    ) -> Option<ModelBuild> {
        let geom = GeometryTag::from_bytes(&self.find(b"geom", key.geometry)?).ok()?;
        let tex_tag = match self.large {
            Some(large)
                if self
                    .small
                    .find_tag_by_id(FourCC::new(*b"core"), geom.texture_tag)
                    .is_none() =>
            {
                geom.resolve_texture_tag(large)
            }
            _ => geom.resolve_texture_tag(self.small),
        };
        Some(ModelBuild {
            key,
            model,
            marker_permutation,
            geom,
            tex_tag,
        })
    }
}

impl ModelBuild {
    fn permutation(&self) -> Option<&mandrake_tag::ModelPermutation> {
        self.model.as_ref()?.permutation(self.marker_permutation)
    }

    pub(crate) fn with_surface_transparency(mut self, transparency: &[u16]) -> Self {
        crate::model_effects::fade_surfaces(&mut self.geom, transparency);
        self
    }

    pub(crate) fn mesh(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &ModelRenderer,
        collection: Option<&CollectionTag>,
    ) -> ModelMesh {
        let label = format!("{}#{}", self.key.geometry, self.key.permutation);
        match collection {
            Some(c) => ModelMesh::from_geometry_with_collection(
                device,
                queue,
                renderer,
                &label,
                &self.geom,
                c,
                self.permutation(),
            ),
            None => ModelMesh::from_geometry(device, &label, &self.geom, None),
        }
    }

    pub(crate) fn triangles(&self, collection: Option<&CollectionTag>) -> Vec<TranslucentTriangle> {
        let (vertices, indices, mut submeshes, _) = ModelMesh::build_geometry_buffers_engine(
            &self.geom,
            collection,
            collection.and(self.permutation()),
            None,
        );
        for s in &mut submeshes {
            s.translucent = true;
        }
        TranslucentTriangle::collect(&vertices, &indices, &submeshes)
    }
}

pub(crate) struct ModelInstance {
    pub(crate) mesh_key: ModelMeshKey,
    pub(crate) model_matrix: [[f32; 4]; 4],
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) animation: Option<ModelAnimationInstance>,
    core: FourCC,
    reflects: bool,
    reflection_tint: [u8; 8],
    reflection_bind_group: Option<wgpu::BindGroup>,
    pub(crate) marker_placement: ([i32; 3], u16, u16),
    pub(crate) placed: Option<usize>,
    pub(crate) in_region: std::cell::Cell<bool>,
    pub(crate) present: bool,
    pub(crate) mode: Option<FourCC>,
    pub(crate) record: Option<usize>,
    pub(crate) record_linked: bool,
    pub(crate) record_look: Option<crate::model_effects::RecordLook>,
    pub(crate) followed_key: Option<ModelMeshKey>,
    pub(crate) moved: bool,
    pub(crate) shade: [u8; 8],
    pub(crate) effect: Option<crate::model_effects::EffectDraw>,
    pub(crate) record_draws: Option<crate::model_effects::RecordDraws>,
    pub(crate) cells_released: bool,
}

impl ModelInstance {
    fn new(
        mesh_key: ModelMeshKey,
        model_matrix: [[f32; 4]; 4],
        bind_group: wgpu::BindGroup,
    ) -> Self {
        Self {
            mesh_key,
            model_matrix,
            bind_group,
            animation: None,
            core: FourCC::new([0; 4]),
            reflects: false,
            reflection_tint: mandrake_render::model::MODEL_SLOT0_TINT,
            reflection_bind_group: None,
            marker_placement: ([0; 3], 0, 0),
            placed: None,
            in_region: std::cell::Cell::new(true),
            present: true,
            mode: None,
            record: None,
            record_linked: false,
            record_look: None,
            followed_key: None,
            moved: false,
            shade: mandrake_view::media::SLOT0_IDENTITY,
            effect: None,
            record_draws: None,
            cells_released: false,
        }
    }

    pub(crate) fn draws(&self) -> impl Iterator<Item = (ModelMeshKey, u16)> + '_ {
        let (first, second) = match &self.record_draws {
            Some(d) => (d.first, d.second),
            None => ((self.mesh_key, 0), None),
        };
        std::iter::once(first).chain(second)
    }

    pub(crate) fn set_matrix(
        &mut self,
        renderer: &ModelRenderer,
        device: &wgpu::Device,
        matrix: [[f32; 4]; 4],
    ) {
        self.model_matrix = matrix;
        let (_buffer, bind_group) = renderer.create_shaded_bind_group(device, &matrix, self.shade);
        self.bind_group = bind_group;
    }

    pub(crate) fn opaque_mesh_key(&self) -> Option<ModelMeshKey> {
        match &self.effect {
            Some(e) if e.first.is_some() => None,
            _ => Some(self.mesh_key),
        }
    }
}

pub(crate) fn opaque_model_draws<'a>(
    instances: &'a [ModelInstance],
    meshes: &'a BTreeMap<ModelMeshKey, ModelMesh>,
) -> Vec<(&'a wgpu::BindGroup, &'a ModelMesh)> {
    instances
        .iter()
        .filter(|i| i.in_region.get())
        .filter_map(|i| Some((&i.bind_group, meshes.get(&i.opaque_mesh_key()?)?)))
        .collect()
}

pub(crate) fn attach_model_media(
    device: &wgpu::Device,
    renderer: &ModelRenderer,
    instances: &mut [ModelInstance],
    meshes: &BTreeMap<ModelMeshKey, ModelMesh>,
    archives: &[&TagArchive],
    map_media: &mut Option<std::sync::Arc<media::MapMedia>>,
) {
    let Some(arc) = map_media.as_mut() else {
        return;
    };
    for inst in instances.iter_mut() {
        let core = media::CoreRef::of_core(archives, inst.core).unwrap_or_default();
        let tint = core.resolve(arc.global_tint);
        let shade = mandrake_view::media::slot0_tint(tint.fraction, tint.color);
        inst.shade = shade;
        if shade != mandrake_view::media::SLOT0_IDENTITY {
            let (_buffer, bind_group) =
                renderer.create_shaded_bind_group(device, &inst.model_matrix, shade);
            inst.bind_group = bind_group;
        }
    }
    let mut placed_models = Vec::new();
    let mut placed = Vec::new();
    for (i, inst) in instances.iter().enumerate() {
        let Some(engine) = meshes.get(&inst.mesh_key).and_then(|m| m.engine.as_ref()) else {
            continue;
        };
        let (position, yaw, pitch) = inst.marker_placement;
        let mut model = media::PlacedModel::new(engine.clone(), position, yaw, pitch);
        if let Some(a) = &inst.animation {
            let frame = |k: &Option<ModelMeshKey>| {
                let engine = meshes.get(k.as_ref()?)?.engine.clone()?;
                Some(std::sync::Arc::new(engine))
            };
            model = model.with_animation(a.marker, a.frame_meshes.iter().map(frame).collect());
        }
        placed_models.push(model);
        placed.push(i);
    }
    let map = std::sync::Arc::make_mut(arc);
    let mut frame_cells_of: Vec<Vec<FrameCells>> = placed
        .iter()
        .map(|&i| frame_cells(&instances[i], meshes, map))
        .collect();
    let (reflects, overlapping) = map.claim_model_cells(placed_models);
    if overlapping > 0 {
        warn!(
            "{overlapping} models meet an earlier model's cells (the engine's new_model deletes them)"
        );
    }
    for (k, &i) in placed.iter().enumerate() {
        let inst = &mut instances[i];
        inst.placed = Some(k);
        inst.reflects = reflects[k];
        if let Some(a) = inst.animation.as_mut() {
            a.frame_cells = std::mem::take(&mut frame_cells_of[k]);
        }
        let core = media::CoreRef::of_core(archives, inst.core).unwrap_or_default();
        inst.reflection_tint = map.reflection_tint(core);
        let can_reflect = inst.reflects
            || inst
                .animation
                .as_ref()
                .is_some_and(|a| a.frame_cells.iter().any(|f| f.reflects));
        if can_reflect {
            let (_buffer, bind_group) = renderer.create_shaded_bind_group(
                device,
                &Mat4::IDENTITY.cols,
                inst.reflection_tint,
            );
            inst.reflection_bind_group = Some(bind_group);
        }
    }
    for inst in instances.iter().filter(|m| m.reflects) {
        debug!(
            "Model media: '{}' at {:?} reflects",
            inst.mesh_key.geometry, inst.marker_placement.0
        );
    }
    info!(
        "Model media: {} of {} models reflect",
        instances.iter().filter(|m| m.reflects).count(),
        instances.len()
    );
}

#[derive(Clone, Copy, Debug, Default)]
struct FrameCells {
    reflects: bool,
    bounds: [[i32; 3]; 2],
}

impl FrameCells {
    fn cells(&self) -> [[i32; 2]; 2] {
        mandrake_view::model_surfaces::bounds_cells(self.bounds.map(|c| c.map(|v| v as f32)))
    }
}

fn frame_cells(
    inst: &ModelInstance,
    meshes: &BTreeMap<ModelMeshKey, ModelMesh>,
    map: &media::MapMedia,
) -> Vec<FrameCells> {
    let Some(a) = inst.animation.as_ref() else {
        return Vec::new();
    };
    let (position, yaw, pitch) = inst.marker_placement;
    let (w, h) = (i32::from(map.cell_width), i32::from(map.cell_height));
    let flags = |col: i32, row: i32| {
        let i = (row.clamp(0, h - 1) * w + col.clamp(0, w - 1)) as usize;
        map.cells.get(i).map_or(0, |c| c.flags)
    };
    a.frame_meshes
        .iter()
        .map(|k| {
            let Some(engine) = k.and_then(|k| meshes.get(&k)?.engine.clone()) else {
                return FrameCells::default();
            };
            let placed = media::PlacedModel::new(engine, position, yaw, pitch);
            let bounds = placed.bounds();
            let rect =
                mandrake_view::model_surfaces::bounds_cells(bounds.map(|c| c.map(|v| v as f32)));
            FrameCells {
                reflects: mandrake_view::model_surfaces::model_reflects(
                    rect,
                    &flags,
                    &placed.geometry,
                ),
                bounds,
            }
        })
        .collect()
}

pub(crate) fn log_region_culled_surfaces(
    instances: &[ModelInstance],
    meshes: &BTreeMap<ModelMeshKey, ModelMesh>,
    camera: &Camera,
    aspect: f32,
    mesh_tag: &MeshTag,
    region: &mandrake_view::view_region::ViewRegion,
) {
    use mandrake_view::model_surfaces::{SurfaceInstance, SurfaceRegion, WorldFrame};
    let surfaces: Vec<SurfaceInstance<'_>> = instances
        .iter()
        .filter(|i| i.in_region.get())
        .filter_map(|i| {
            let m = meshes.get(&i.mesh_key)?;
            Some(SurfaceInstance {
                model_matrix: i.model_matrix,
                geometry: m.engine.as_ref()?,
                texture_words: &m.texture_words,
            })
        })
        .collect();
    let region = SurfaceRegion {
        region,
        frame: WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height),
    };
    let (culled, on_screen) = mandrake_view::model_surfaces::region_culled_surfaces(
        &surfaces,
        &camera.view_matrix(),
        &camera.view_proj_matrix(aspect),
        &region,
    );
    info!(
        "Model surfaces outside the view region (render_geometry): {culled} on {} drawn \
         models, {on_screen} of them can reach the screen",
        surfaces.len()
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_model_media<'a>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    _renderer: &ModelRenderer,
    category2: &mut TranslucentBatch,
    reflections: &mut mandrake_render::ReflectionBatch,
    instances: &'a [ModelInstance],
    meshes: &'a BTreeMap<ModelMeshKey, ModelMesh>,
    camera: &Camera,
    map_media: Option<&media::MapMedia>,
    mesh_tag: &MeshTag,
    local_time: i32,
    region: Option<&mandrake_view::view_region::ViewRegion>,
) -> ModelMediaDraws<'a> {
    use mandrake_view::model_surfaces::{
        ReflectionMedia, SurfaceInstance, SurfaceRegion, WorldFrame, category2_draw_list,
        reflection_triangles,
    };
    let view = camera.view_matrix();
    let surface_region = region.map(|region| SurfaceRegion {
        region,
        frame: WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height),
    });
    let with_mesh: Vec<(&ModelInstance, &ModelMesh, u16)> = instances
        .iter()
        .filter(|i| i.in_region.get())
        .flat_map(|i| {
            i.draws()
                .filter_map(move |(key, t)| meshes.get(&key).map(|m| (i, m, t)))
        })
        .collect();
    let c2: Vec<(&ModelInstance, &ModelMesh)> = with_mesh
        .iter()
        .map(|&(i, m, _)| (i, m))
        .filter(|(_, m)| m.engine.as_ref().is_some_and(|e| e.has_category2()))
        .collect();
    let c2_instances: Vec<SurfaceInstance<'_>> = c2
        .iter()
        .map(|(i, m)| SurfaceInstance {
            model_matrix: i.model_matrix,
            geometry: m.engine.as_ref().expect("filtered"),
            texture_words: &m.texture_words,
        })
        .collect();
    category2.prepare(
        device,
        queue,
        category2_draw_list(&c2_instances, &view, surface_region.as_ref()),
    );
    let category2_draws = c2.iter().map(|(i, m)| (&i.bind_group, *m)).collect();

    let mut reflection_draws = Vec::new();
    let mut triangles = Vec::new();
    if let Some(map) = map_media.filter(|m| !m.no_reflections) {
        let refl: Vec<(&ModelInstance, &ModelMesh, u16, &wgpu::BindGroup)> = with_mesh
            .iter()
            .copied()
            .filter_map(|(i, m, t)| {
                (i.reflects && m.engine.is_some())
                    .then(|| i.reflection_bind_group.as_ref().map(|bg| (i, m, t, bg)))
                    .flatten()
            })
            .collect();
        let height = |x: i32, y: i32| map.cell_media_height(x, y);
        let media = ReflectionMedia {
            media_height: &height,
            wobble: map.surface.wobble,
            local_time,
            sine: &mandrake_math::engine::trig_tables().sine,
            transparency: map.reflection_transparency,
            frame: WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height),
        };
        let inputs: Vec<(SurfaceInstance<'_>, &[ModelVertex], u16)> = refl
            .iter()
            .map(|&(i, m, t, _)| {
                (
                    SurfaceInstance {
                        model_matrix: i.model_matrix,
                        geometry: m.engine.as_ref().expect("filtered"),
                        texture_words: &m.texture_words,
                    },
                    m.cpu_vertices.as_slice(),
                    t,
                )
            })
            .collect();
        triangles = reflection_triangles(
            &inputs,
            &view,
            camera.eye_position(),
            &media,
            surface_region.as_ref(),
        );
        reflection_draws = refl.iter().map(|&(_, m, _, bg)| (bg, m)).collect();
    }
    reflections.prepare(device, queue, &triangles);
    debug!(
        "Model media: {} category-2 surfaces, {} reflected surfaces",
        category2.keys().len(),
        triangles.len()
    );
    if tracing::enabled!(tracing::Level::TRACE) {
        let mut by_instance: std::collections::BTreeMap<usize, Vec<u16>> = Default::default();
        for t in &triangles {
            by_instance.entry(t.instance).or_default().push(t.surface);
        }
        for (i, mut surfaces) in by_instance {
            surfaces.sort_unstable();
            trace!("Model reflection instance {i}: surfaces {surfaces:?}");
        }
    }
    ModelMediaDraws {
        category2: category2_draws,
        reflections: reflection_draws,
    }
}

pub(crate) struct ModelMediaDraws<'a> {
    pub(crate) category2: Vec<(&'a wgpu::BindGroup, &'a ModelMesh)>,
    reflections: Vec<(&'a wgpu::BindGroup, &'a ModelMesh)>,
}

impl<'a> ModelMediaDraws<'a> {
    pub(crate) fn reflection_instances(&self) -> &[(&'a wgpu::BindGroup, &'a ModelMesh)] {
        &self.reflections
    }
}

pub(crate) struct TranslucentModel<'a> {
    matrix: [[f32; 4]; 4],
    triangles: &'a [TranslucentTriangle],
    bind_group: &'a wgpu::BindGroup,
    mesh: &'a ModelMesh,
}

pub(crate) fn translucent_model_instances<'a>(
    instances: &'a [ModelInstance],
    meshes: &'a BTreeMap<ModelMeshKey, ModelMesh>,
) -> Vec<TranslucentModel<'a>> {
    let mut out = Vec::new();
    for inst in instances.iter().filter(|inst| inst.in_region.get()) {
        let layer = |l: &'a crate::model_effects::EffectLayer| {
            meshes.get(&l.key).map(|mesh| TranslucentModel {
                matrix: inst.model_matrix,
                triangles: l.triangles.as_slice(),
                bind_group: &l.bind_group,
                mesh,
            })
        };
        let effect = inst.effect.as_ref();
        match effect.and_then(|e| e.first.as_ref()) {
            Some(first) => out.extend(layer(first)),
            None => out.extend(
                meshes
                    .get(&inst.mesh_key)
                    .filter(|m| !m.translucent_triangles.is_empty())
                    .map(|mesh| TranslucentModel {
                        matrix: inst.model_matrix,
                        triangles: mesh.translucent_triangles.as_slice(),
                        bind_group: &inst.bind_group,
                        mesh,
                    }),
            ),
        }
        if let Some(second) = effect.and_then(|e| e.second.as_ref()) {
            out.extend(layer(second));
        }
    }
    out
}

pub(crate) fn prepare_translucent_models<'a>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &mut TranslucentBatch,
    models: &[TranslucentModel<'a>],
    camera: &Camera,
    aspect_ratio: f32,
) -> Vec<(&'a wgpu::BindGroup, &'a ModelMesh)> {
    let list = sort_translucent_triangles_on_screen(
        models.iter().map(|m| (m.matrix, m.triangles)),
        &camera.view_matrix(),
        &camera.view_proj_matrix(aspect_ratio),
    );
    batch.prepare(device, queue, list);
    models.iter().map(|m| (m.bind_group, m.mesh)).collect()
}

pub(crate) struct ModelAnimationInstance {
    pub(crate) definition: ModelAnimationTag,
    pub(crate) location: WorldPoint,
    marker: usize,
    pub(crate) frame_meshes: Vec<Option<ModelMeshKey>>,
    frame_cells: Vec<FrameCells>,
    claimed_frame: Option<usize>,
    pub(crate) playback: ModelAnimationPlayback,
    pub(crate) sim_model: i16,
    pub(crate) in_sim: bool,
    shadow_maps: Option<ModelAnimationShadowMaps>,
    deleted: bool,
    shown_shadow: std::cell::Cell<Option<Option<usize>>>,
}

impl ModelAnimationInstance {
    fn shadow_overlay(&self, position: [i32; 3], frame: usize) -> Option<ShadeOverlay<'_>> {
        let maps = self.shadow_maps.as_ref()?;
        (maps.width <= maps.row_bytes).then_some(())?;
        Some(ShadeOverlay {
            x: position[0] + i32::from(maps.offset[0]),
            y: position[1] + i32::from(maps.offset[1]),
            pixels: maps.frame(frame)?,
            width: maps.width,
            height: maps.height,
            row_bytes: maps.row_bytes,
            intensity: 0xffff,
            mode: OverlayMode::Min,
        })
    }
}

impl ModelInstance {
    fn show_frame(&mut self, has_model: bool) {
        let Some(anim) = self.animation.as_ref() else {
            return;
        };
        let frame = anim.playback.frame;
        let key = anim.frame_meshes.get(frame).copied().flatten();
        if let Some(key) = key {
            self.mesh_key = key;
        }
        self.present = has_model && key.is_some();
        if let Some(f) = anim.frame_cells.get(frame) {
            self.reflects = f.reflects;
        }
    }

    fn tick_animation(&mut self) -> Option<(FourCC, WorldPoint)> {
        let anim = self.animation.as_mut()?;
        let sound = anim.playback.tick(&anim.definition);
        let location = anim.location;
        self.show_frame(true);
        sound.map(|s| (s, location))
    }

    fn follow_datum(&mut self, datum: Option<&mandrake_sim::model_animations::ModelAnimation>) {
        let Some(anim) = self.animation.as_mut() else {
            return;
        };
        anim.in_sim = datum.is_some();
        anim.deleted = datum.is_none();
        anim.sim_model = datum.map_or(-1, |d| d.model);
        if let Some(d) = datum {
            anim.playback = d.playback;
        }
        self.show_frame(datum.is_some_and(|d| d.model != -1));
    }
}

pub(crate) fn animating_shadow_overlays(instances: &[ModelInstance]) -> Vec<ShadeOverlay<'_>> {
    instances
        .iter()
        .filter_map(|inst| {
            let a = inst.animation.as_ref().filter(|a| !a.deleted)?;
            a.shadow_overlay(inst.marker_placement.0, a.playback.frame)
        })
        .collect()
}

pub(crate) fn changed_animating_shadow_rects(
    instances: &[ModelInstance],
    map_w: u32,
    map_h: u32,
) -> Vec<PixelRect> {
    instances
        .iter()
        .filter_map(|inst| {
            let a = inst.animation.as_ref()?;
            let now = (!a.deleted).then_some(a.playback.frame);
            (a.shown_shadow.replace(Some(now)) != Some(now))
                .then(|| a.shadow_overlay(inst.marker_placement.0, 0))
                .flatten()
                .and_then(|o| o.pixel_rect(map_w, map_h))
        })
        .collect()
}

pub(crate) fn tick_model_animations(instances: &mut [ModelInstance]) -> Vec<(FourCC, WorldPoint)> {
    instances
        .iter_mut()
        .filter_map(ModelInstance::tick_animation)
        .collect()
}

pub(crate) fn follow_sim_model_animations(
    instances: &mut [ModelInstance],
    sim: &Simulation,
    map: &mut Option<std::sync::Arc<media::MapMedia>>,
) -> bool {
    let world = &sim.engine;
    let running = sim.engine_stop.is_none();
    for inst in instances.iter_mut() {
        let Some(marker) = inst.animation.as_ref().map(|a| a.marker) else {
            continue;
        };
        let datum = world
            .model_animations
            .animations
            .iter()
            .find(|d| d.marker == marker);
        if running || datum.is_some() {
            inst.follow_datum(datum);
        }
    }
    sync_map_cells(instances, world, map)
}

fn sync_map_cells(
    instances: &mut [ModelInstance],
    world: &mandrake_sim::engine_rng::EngineWorld,
    map: &mut Option<std::sync::Arc<media::MapMedia>>,
) -> bool {
    let mut changed = false;
    let Some(arc) = map.as_mut() else {
        return false;
    };
    let (w, h) = (world.cells.width, world.cells.height);
    let model_cells_count =
        arc.surface.mesh_flags & mandrake_view::media::MESH_FLAG_MODEL_CELLS_NOT_MEDIA != 0;
    for inst in instances.iter_mut() {
        let (Some(a), Some(placed)) = (inst.animation.as_mut(), inst.placed) else {
            continue;
        };
        let frame = a.playback.frame;
        if a.claimed_frame == Some(frame) || !a.in_sim {
            continue;
        }
        let tag_frame = |f: usize| a.definition.frames.get(f).map(|f| f.model_tag);
        let swapped = a
            .claimed_frame
            .is_none_or(|old| tag_frame(old) != tag_frame(frame));
        let old = a.claimed_frame.unwrap_or(0);
        a.claimed_frame = Some(frame);
        if !swapped {
            continue;
        }
        let sim_model = a.sim_model;
        for f in [old, frame] {
            let Some(rect) = a.frame_cells.get(f).map(FrameCells::cells) else {
                continue;
            };
            for row in rect[0][1].max(0)..=rect[1][1].min(h - 1) {
                for col in rect[0][0].max(0)..=rect[1][0].min(w - 1) {
                    let i = (row * w + col) as usize;
                    let want = match world.cells.models.get(i) {
                        Some(-1) | None => -1,
                        Some(&m) if m == sim_model => placed as i16,
                        Some(_) => continue,
                    };
                    let was = arc.cell_models.get(i).copied().unwrap_or(want);
                    if was != want {
                        std::sync::Arc::make_mut(arc).cell_models[i] = want;
                        changed |= model_cells_count && (was == -1) != (want == -1);
                    }
                }
            }
        }
        if sim_model == -1 {
            continue;
        }
        let Some(bounds) = a.frame_cells.get(frame).map(|f| f.bounds) else {
            continue;
        };
        let blocks = |axis: usize| {
            let (lo, hi) = (
                bounds[0][axis].min(bounds[1][axis]),
                bounds[0][axis].max(bounds[1][axis]),
            );
            (lo >> 10)..=((hi >> 10) + 1)
        };
        let cell_drawn = |col: i32, row: i32, bit: u16| {
            let i = (row.clamp(0, h - 1) * w + col.clamp(0, w - 1)) as usize;
            if world.cells.flags.get(i).copied().unwrap_or(0)
                & mandrake_sim::passability::CELL_KILLED
                == 0
            {
                bit
            } else {
                0
            }
        };
        for by in blocks(1) {
            for bx in blocks(0) {
                let (c, r) = (bx * 2, by * 2);
                let drawn = cell_drawn(c, r, 0x10)
                    | cell_drawn(c + 1, r, 0x20)
                    | cell_drawn(c + 1, r + 1, 0x40)
                    | cell_drawn(c, r + 1, 0x80);
                let Some(lod) = arc.lod.as_ref() else {
                    continue;
                };
                if bx < 0 || bx >= lod.block_width as i32 || by < 0 || by >= lod.block_height as i32
                {
                    continue;
                }
                let k = by as usize * lod.block_width + bx as usize;
                let word = (lod.words[k] & 0xff0f) | drawn;
                if word != lod.words[k] {
                    let lod = std::sync::Arc::make_mut(arc).lod.as_mut().expect("checked");
                    lod.words[k] = word;
                    changed = true;
                }
            }
        }
    }
    changed
}

pub(crate) struct ModelAnimationUpdate {
    pub(crate) sounds: Vec<(FourCC, WorldPoint)>,
    pub(crate) landscape_changed: bool,
}

pub(crate) fn update_model_animations(
    instances: &mut [ModelInstance],
    sim: &mut Simulation,
    map: &mut Option<std::sync::Arc<media::MapMedia>>,
) -> ModelAnimationUpdate {
    let mut sounds: Vec<(FourCC, WorldPoint)> = sim
        .take_model_animation_sounds()
        .into_iter()
        .map(|s| {
            let (x, y, z) = s.position;
            (s.sound, WorldPoint::new(x, y, z))
        })
        .collect();
    let mut landscape_changed = false;
    if sim.engine_stop.is_none() {
        landscape_changed = follow_sim_model_animations(instances, sim, map);
    } else {
        sounds.extend(tick_model_animations(instances));
    }
    ModelAnimationUpdate {
        sounds,
        landscape_changed,
    }
}

pub(crate) fn model_marker_position(mesh_tag: &MeshTag, marker: &MarkerEntry) -> (f32, f32, f32) {
    let (wx, _, wz) = mesh_tag.marker_world_position(marker, 1.0);
    (wx, marker.z as f32 / MarkerEntry::WORLD_ONE, wz)
}

pub(crate) fn marker_model_matrix(marker: &MarkerEntry, position: [f32; 3]) -> [[f32; 4]; 4] {
    let m = mandrake_tag::model_hit::model_world_matrix(marker.yaw as u16, marker.pitch as u16);
    mandrake_view::rotation::port_model_matrix(&m, position)
}

fn animation_marker(marker: &MarkerEntry) -> MarkerEntry {
    MarkerEntry {
        yaw: 0,
        ..marker.clone()
    }
}

pub(crate) fn frame_permutation(anim: &[u8], frame: usize) -> usize {
    let o = 0x12 + ModelAnimationTag::FRAME_SIZE * frame;
    anim.get(o..o + 2)
        .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]).max(0) as usize)
}

pub(crate) fn marker_placement(marker: &MarkerEntry) -> ([i32; 3], u16, u16) {
    (
        [marker.x, marker.y, marker.z],
        marker.yaw as u16,
        marker.pitch as u16,
    )
}

pub(crate) fn load_model_instances(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &ModelRenderer,
    small_archive: &TagArchive,
    large_archive: Option<&TagArchive>,
    mesh_tag: &MeshTag,
) -> (BTreeMap<ModelMeshKey, ModelMesh>, Vec<ModelInstance>) {
    let mut meshes = BTreeMap::new();
    let mut instances = Vec::new();
    let source = ModelTagSource {
        small: small_archive,
        large: large_archive,
    };
    let mut coll_cache: BTreeMap<FourCC, Option<CollectionTag>> = BTreeMap::new();
    let find_tag = |group: &[u8; 4], id: FourCC| source.find(group, id);
    let mut cores: BTreeMap<ModelMeshKey, FourCC> = BTreeMap::new();

    let mut ensure_model = |model_id: FourCC,
                            marker_permutation: usize,
                            meshes: &mut BTreeMap<ModelMeshKey, ModelMesh>|
     -> Option<ModelMeshKey> {
        let (model, key) = source.model_key(model_id, marker_permutation);
        if meshes.contains_key(&key) {
            return Some(key);
        }
        let built = source.build(model, key, marker_permutation)?;
        cores.insert(key, built.geom.texture_tag);
        let coll = coll_cache
            .entry(built.tex_tag)
            .or_insert_with(|| source.collection(built.tex_tag))
            .clone();
        let mesh = built.mesh(device, queue, renderer, coll.as_ref());
        info!(
            "Loaded textured 3D polygonal model '{}' (mode '{}', permutation {}, tex: '{}', {} vertices, {} surfaces, {} submeshes)",
            key.geometry,
            model_id,
            key.permutation,
            built.tex_tag,
            built.geom.vertices.len(),
            built.geom.surfaces.len(),
            mesh.submeshes.len()
        );
        meshes.insert(key, mesh);
        Some(key)
    };

    for marker in mesh_tag.model_markers() {
        let Some(tag_id) = mesh_tag.palette_tag_for_marker(marker) else {
            continue;
        };

        if let Some(mesh_key) = ensure_model(tag_id, marker.model_permutation(), &mut meshes) {
            let (wx, wy, wz) = model_marker_position(mesh_tag, marker);
            let model_matrix = marker_model_matrix(marker, [wx, wy, wz]);
            let tint = [1.0, 1.0, 1.0, 1.0];
            let (_buf, bind_group) = renderer.create_model_bind_group(device, &model_matrix, &tint);

            instances.push(ModelInstance {
                marker_placement: marker_placement(marker),
                mode: Some(tag_id),
                ..ModelInstance::new(mesh_key, model_matrix, bind_group)
            });
        }
    }

    for marker in mesh_tag.model_anim_markers() {
        let Some(tag_id) = mesh_tag.palette_tag_for_marker(marker) else {
            continue;
        };

        let bytes = find_tag(&ModelAnimationTag::TAG_TYPE, tag_id);
        let definition = bytes
            .as_deref()
            .and_then(|d| match ModelAnimationTag::from_bytes(d) {
                Ok(anim) => Some(anim),
                Err(e) => {
                    warn!("Failed to parse model animation '{}': {}", tag_id, e);
                    None
                }
            });
        let (Some(definition), Some(bytes)) = (definition, bytes) else {
            warn!(
                "Model animation marker references missing anim tag '{}'",
                tag_id
            );
            continue;
        };

        let frame_meshes: Vec<Option<ModelMeshKey>> = definition
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| ensure_model(f.model_tag, frame_permutation(&bytes, i), &mut meshes))
            .collect();

        let playback = ModelAnimationPlayback::new(&definition, 0);
        let start = frame_meshes.get(playback.frame).copied().flatten();
        let Some(mesh_key) = start.or_else(|| frame_meshes.iter().copied().flatten().next()) else {
            warn!(
                "Model animation '{}' has no loadable frame geometry",
                tag_id
            );
            continue;
        };

        let placed = animation_marker(marker);
        let (wx, wy, wz) = model_marker_position(mesh_tag, &placed);
        let model_matrix = marker_model_matrix(&placed, [wx, wy, wz]);
        let tint = [1.0, 1.0, 1.0, 1.0];
        let (_buf, bind_group) = renderer.create_model_bind_group(device, &model_matrix, &tint);

        info!(
            "Placed model animation '{}' at ({:.2}, {:.2}, {:.2}): {} frames, {} ticks/frame, flags {:#x}",
            tag_id,
            wx,
            wy,
            wz,
            definition.frame_count(),
            definition.ticks_per_displayed_frame(),
            definition.flags
        );

        instances.push(ModelInstance {
            marker_placement: marker_placement(&placed),
            present: start.is_some(),
            animation: Some(ModelAnimationInstance {
                marker: mesh_tag
                    .markers
                    .iter()
                    .position(|m| std::ptr::eq(m, marker))
                    .unwrap_or(usize::MAX),
                definition,
                location: WorldPoint::from_marker(marker),
                frame_meshes,
                frame_cells: Vec::new(),
                claimed_frame: None,
                playback,
                sim_model: -1,
                in_sim: false,
                shadow_maps: ModelAnimationShadowMaps::from_tag_data(&bytes),
                deleted: false,
                shown_shadow: std::cell::Cell::new(None),
            }),
            ..ModelInstance::new(mesh_key, model_matrix, bind_group)
        });
    }

    for inst in &mut instances {
        inst.core = cores.get(&inst.mesh_key).copied().unwrap_or(inst.core);
    }
    info!(
        "Loaded {} distinct 3D model meshes, placed {} instances across map",
        meshes.len(),
        instances.len()
    );
    (meshes, instances)
}

#[cfg(test)]
mod animating_shadow_tests {
    use super::*;

    fn animation(maps: ModelAnimationShadowMaps) -> ModelAnimationInstance {
        let definition = ModelAnimationTag {
            flags: 0,
            ticks_per_frame: 0,
            frames: Vec::new(),
            sounds: [None; 2],
        };
        ModelAnimationInstance {
            playback: ModelAnimationPlayback::new(&definition, 0),
            definition,
            location: WorldPoint::new(0, 0, 0),
            marker: 0,
            frame_meshes: Vec::new(),
            frame_cells: Vec::new(),
            claimed_frame: None,
            sim_model: -1,
            in_sim: true,
            shadow_maps: Some(maps),
            deleted: false,
            shown_shadow: std::cell::Cell::new(None),
        }
    }

    fn maps(width: i16, row_bytes: i16) -> ModelAnimationShadowMaps {
        ModelAnimationShadowMaps {
            width,
            height: 2,
            row_bytes,
            offset: [565, -64],
            pixels: (0..16).collect(),
        }
    }

    #[test]
    fn a_frame_is_one_mode_2_overlay_at_the_tag_offset() {
        let a = animation(maps(3, 4));
        let o = a.shadow_overlay([1000, 2000, 7], 1).expect("overlay");
        assert_eq!((o.x, o.y), (1565, 1936));
        assert_eq!(o.pixels, &[8, 9, 10, 11, 12, 13, 14, 15]);
        assert_eq!((o.width, o.height, o.row_bytes), (3, 2, 4));
        assert_eq!(o.mode, OverlayMode::Min);
        assert_eq!(a.shadow_overlay([0; 3], 2), None);
    }

    #[test]
    fn a_bitmap_wider_than_its_rows_has_no_overlay() {
        assert_eq!(animation(maps(5, 4)).shadow_overlay([0; 3], 0), None);
    }
}
