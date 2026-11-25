use crate::model_effects::RecordDraws;
use crate::tfl_game::FrameTime;
use crate::tfl_landscape::Waves;
use crate::tfl_map::TflMap;
use crate::tfl_objects::{TFL_CELL_SIZE, WORLD_PER_RENDER_UNIT};
use mandrake_common::FourCC;
use mandrake_render::{ModelMesh, ModelRenderer, ModelSubMesh, ModelSubmeshes, TranslucentBatch};
use mandrake_tag::collection::{CollectionTag, CorePermutations};
use mandrake_tag::mesh::MarkerEntry;
use mandrake_tag::model::{
    GEOMETRY_SURFACE_CATEGORY_2, GEOMETRY_SURFACE_DEPTH_FARTHER, GEOMETRY_SURFACE_DEPTH_NEAREST,
    GEOMETRY_SURFACE_TWO_SIDED,
};
use mandrake_tag::{GeometryTag, ModelPermutation};
use mandrake_view::{
    camera::Camera,
    model::{ModelVertex, TranslucentDrawList, TranslucentRun},
};
use std::collections::BTreeMap;
use tracing::{info, warn};

const MARKER_MODEL: i16 = 6;

struct Layer {
    kind: usize,
    transparency: u16,
    bind_group: wgpu::BindGroup,
    reflection_bind_group: wgpu::BindGroup,
    mesh: ModelMesh,
    shown: Vec<Option<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RecordLook {
    permutation: i16,
    next_permutation: i16,
    transition: u16,
    fade: u16,
    yaw: u16,
    pitch: u16,
}

struct Placed {
    index: usize,
    mode: FourCC,
    position: [i32; 3],
    angles: (u16, u16),
    matrix: [[f32; 4]; 4],
    layers: Vec<Layer>,
    reflects: bool,
    record: Option<usize>,
    look: Option<RecordLook>,
}

#[derive(Clone, Copy)]
struct Slot {
    first: u32,
    submesh: u32,
}

struct Kind {
    id: FourCC,
    geom: GeometryTag,
    collection: FourCC,
    palette: usize,
    permutation: Option<ModelPermutation>,
    materials: Vec<Option<usize>>,
    vertices: Vec<ModelVertex>,
    indices: Vec<u32>,
    slots: Vec<Slot>,
    ranges: Vec<(u32, u32, bool, bool)>,
    footprint: Vec<(i16, i16)>,
}

#[derive(Default)]
pub(crate) struct TflModels {
    kinds: Vec<Kind>,
    placed: Vec<Placed>,
    collections: BTreeMap<FourCC, Option<CollectionTag>>,
    palettes: Vec<Vec<[u8; 4]>>,
    textures: BTreeMap<(FourCC, usize, usize), wgpu::BindGroup>,
    waves: Option<Waves>,
    cells: (u32, u32),
    reflection_tint: [u8; 8],
    waves_time: Option<FrameTime>,
    pick: ModelPick,
    kind_ids: BTreeMap<(FourCC, usize), Option<usize>>,
    linked: bool,
}

pub(crate) fn tfl_model_axes(yaw: u16, pitch: u16) -> [[f32; 3]; 3] {
    let angle = |a: u16| f32::from(a) * std::f32::consts::TAU / 65536.0;
    let (s1, c1) = angle(yaw).sin_cos();
    let (s2, c2) = angle(pitch).sin_cos();
    let a = [s1, -c1, 0.0];
    let b = [-c2 * c1, -c2 * s1, -s2];
    let c = [
        a[2] * b[1] - a[1] * b[2],
        a[0] * b[2] - b[0] * a[2],
        b[0] * a[1] - a[0] * b[1],
    ];
    [a, c, b]
}

pub(crate) fn tfl_model_matrix(
    place: [i32; 3],
    centre: (i16, i16, i16),
    axes: [[f32; 3]; 3],
    cells: (u32, u32),
) -> [[f32; 4]; 4] {
    let [a, c, b] = axes;
    let half_w = cells.0 as f32 * TFL_CELL_SIZE * 0.5;
    let half_d = cells.1 as f32 * TFL_CELL_SIZE * 0.5;
    let w = WORLD_PER_RENDER_UNIT;
    let origin = [
        (place[0] + i32::from(centre.0)) as f32,
        (place[1] + i32::from(centre.1)) as f32,
        (place[2] + i32::from(centre.2)) as f32,
    ];
    let col = |k: usize| [-a[k], b[k], c[k], 0.0];
    [
        col(1),
        col(2),
        col(0),
        [
            half_w - origin[0] / w,
            origin[2] / w,
            origin[1] / w - half_d,
            1.0,
        ],
    ]
}

impl Kind {
    fn build(
        id: FourCC,
        geom: GeometryTag,
        collection: FourCC,
        palette: usize,
        permutation: Option<ModelPermutation>,
    ) -> Self {
        let (vertices, indices, submeshes, engine) = ModelMesh::build_geometry_buffers_grouped(
            &geom,
            None,
            None,
            None,
            ModelSubmeshes::ByMaterial,
        );
        let slots = engine
            .surfaces
            .iter()
            .map(|s| Slot {
                first: s.mesh_vertex,
                submesh: s.submesh,
            })
            .collect();
        let ranges = submeshes
            .iter()
            .map(|m| (m.index_offset, m.index_count, m.category2, m.translucent))
            .collect();
        Self {
            id,
            geom,
            collection,
            palette,
            permutation,
            materials: submeshes.iter().map(|m| m.material).collect(),
            vertices,
            indices,
            slots,
            ranges,
            footprint: Vec::new(),
        }
    }

    fn bitmap(&self, coll: &CollectionTag, m: usize, index: usize, time: u32) -> Option<usize> {
        let mat = self.geom.materials.get(m)?;
        let seq = coll.sequence(usize::try_from(mat.texture_index).ok()?)?;
        let view = match &self.permutation {
            Some(p) => p.material_view(m)?,
            None => 0,
        };
        let tpf = if seq.ticks_per_frame == 0 {
            1
        } else {
            i64::from(seq.ticks_per_frame)
        };
        let frame = (i64::from(time) + m as i64 + index as i64) / tpf;
        seq.bitmap_index(view, frame.max(0) as usize)
    }
}

impl TflModels {
    pub(crate) fn load(
        device: &wgpu::Device,
        renderer: &ModelRenderer,
        map: &TflMap,
        reflection_tint: [u8; 8],
    ) -> Self {
        let mut out = Self {
            reflection_tint,
            ..Self::default()
        };
        let cells = (map.tfl_mesh.cell_width, map.tfl_mesh.cell_height);
        out.waves = Waves::of(map);
        out.cells = cells;
        for (kind, index, marker) in out.read_markers(map) {
            let reflects = footprint_reflects(&out.kinds[kind], marker, map);
            out.place(device, renderer, kind, index, marker, reflects);
        }
        out.pick = ModelPick::of(
            &out.kinds,
            out.placed.iter().map(|p| (p.layers[0].kind, p.matrix)),
            cells,
            out.waves,
        );
        info!(
            "TFL: {} models of {} kinds",
            out.placed.len(),
            out.kinds.len()
        );
        out
    }

    pub(crate) fn pick(&self) -> &ModelPick {
        &self.pick
    }

    fn read_markers<'m>(&mut self, map: &'m TflMap) -> Vec<(usize, usize, &'m MarkerEntry)> {
        let mesh = &map.mesh;
        let mut out = Vec::new();
        for (index, marker) in mesh
            .markers
            .iter()
            .filter(|m| m.marker_type == MARKER_MODEL)
            .enumerate()
        {
            let Some(id) = mesh.palette_tag_for_marker(marker) else {
                continue;
            };
            let perm = marker.model_permutation();
            if let Some(kind) = self.kind_of(map, id, perm) {
                out.push((kind, index, marker));
            }
        }
        out
    }

    fn kind_of(&mut self, map: &TflMap, id: FourCC, perm: usize) -> Option<usize> {
        if let Some(&k) = self.kind_ids.get(&(id, perm)) {
            return k;
        }
        let k = self.read_kind(map, id, perm);
        self.kind_ids.insert((id, perm), k);
        k
    }

    fn read_kind(&mut self, map: &TflMap, id: FourCC, perm: usize) -> Option<usize> {
        let tag = |group: &[u8; 4], id: FourCC| {
            let e = map.archive.find_tag_by_id(FourCC::new(*group), id)?;
            Some((map.archive.get_tag_data(e).ok()?, e.header.version as u16))
        };
        let mut read = || -> Result<Kind, String> {
            let (d, v) = tag(b"mode", id).ok_or("no mode tag")?;
            let model = mandrake_tag::tfl::tfl_model(&d, v).map_err(|e| e.to_string())?;
            let gid = model.model.geometry_tag;
            let (d, v) = tag(b"geom", gid).ok_or(format!("no geom {gid}"))?;
            let geom = mandrake_tag::tfl::tfl_geometry(&d, v).map_err(|e| e.to_string())?;
            let (core_bytes, _) =
                tag(b"core", geom.texture_tag).ok_or(format!("no core {}", geom.texture_tag))?;
            let core = CorePermutations::from_bytes(&core_bytes).map_err(|e| e.to_string())?;
            let permutation = model.model.permutation(perm).cloned();
            let coll_id = core.collection_tag;
            let coll = self
                .collections
                .entry(coll_id)
                .or_insert_with(|| {
                    tag(b".256", coll_id).and_then(|(d, _)| CollectionTag::from_bytes_tfl(&d).ok())
                })
                .as_ref()
                .ok_or(format!("collection {coll_id} not read"))?;
            let colour_table = permutation.as_ref().map_or(0, |p| p.color_table);
            let p = if core.count > 0 {
                (i32::from(colour_table).rem_euclid(i32::from(core.count))) as usize
            } else {
                0
            };
            let mut palette = coll.permutation_palette(&core, p);
            if let Some(tint) = crate::tfl_objects::core_tint(&core_bytes) {
                crate::tfl_objects::tint_palette(&mut palette, tint);
            }
            self.palettes.push(palette);
            let mut kind = Kind::build(id, geom, coll_id, self.palettes.len() - 1, permutation);
            kind.footprint = model.cells.iter().map(|c| (c.x, c.y)).collect();
            Ok(kind)
        };
        match read() {
            Ok(k) => {
                info!(
                    "TFL model {id} (core {}, {} surfaces, {} materials)",
                    k.geom.texture_tag,
                    k.geom.surfaces.len(),
                    k.geom.materials.len()
                );
                self.kinds.push(k);
                Some(self.kinds.len() - 1)
            }
            Err(e) => {
                warn!("TFL model {id}: {e}; not drawn");
                None
            }
        }
    }

    fn place(
        &mut self,
        device: &wgpu::Device,
        renderer: &ModelRenderer,
        kind: usize,
        index: usize,
        marker: &MarkerEntry,
        reflects: bool,
    ) {
        let k = &self.kinds[kind];
        let matrix = marker_matrix(k, marker, self.cells);
        let layer = Self::layer(
            device,
            renderer,
            k,
            kind,
            index,
            &matrix,
            (0, self.reflection_tint),
        );
        self.placed.push(Placed {
            index,
            mode: k.id,
            position: [marker.x, marker.y, marker.z],
            angles: (marker.yaw as u16, marker.pitch as u16),
            matrix,
            layers: vec![layer],
            reflects,
            record: None,
            look: None,
        });
    }

    fn layer(
        device: &wgpu::Device,
        renderer: &ModelRenderer,
        k: &Kind,
        kind: usize,
        index: usize,
        matrix: &[[f32; 4]; 4],
        (transparency, tint): (u16, [u8; 8]),
    ) -> Layer {
        let submeshes = k
            .ranges
            .iter()
            .map(|&(offset, count, category2, translucent)| ModelSubMesh {
                index_offset: offset,
                index_count: count,
                bitmap_index: None,
                texture_bind_group: None,
                two_sided: true,
                translucent,
                category2,
                material: None,
            })
            .collect::<Vec<_>>();
        let n = submeshes.len();
        let mesh = ModelMesh::from_submeshes(
            device,
            &format!("TFL model {}#{index}", k.id),
            &k.vertices,
            &k.indices,
            submeshes,
        )
        .with_depth_keys(device);
        let (bind_group, reflection_bind_group) =
            Self::bind_groups(device, renderer, matrix, transparency, tint);
        Layer {
            kind,
            transparency,
            bind_group,
            reflection_bind_group,
            mesh,
            shown: vec![None; n],
        }
    }

    fn bind_groups(
        device: &wgpu::Device,
        renderer: &ModelRenderer,
        matrix: &[[f32; 4]; 4],
        transparency: u16,
        tint: [u8; 8],
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let opacity = if transparency == 0 {
            1.0
        } else {
            mandrake_view::media::surface_opacity(transparency)
        };
        let model = renderer
            .create_model_bind_group(device, matrix, &[1.0, 1.0, 1.0, opacity])
            .1;
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let reflection = renderer
            .create_faded_bind_group(device, &identity, tint, opacity)
            .1;
        (model, reflection)
    }

    pub(crate) fn follow(
        &mut self,
        device: &wgpu::Device,
        renderer: &ModelRenderer,
        map: &TflMap,
        records: &[mandrake_tfl_sim::ModelInstance],
    ) {
        if !self.linked {
            self.linked = true;
            let mut taken = vec![false; records.len()];
            for p in &mut self.placed {
                p.record = records
                    .iter()
                    .enumerate()
                    .position(|(i, r)| !taken[i] && r.position == p.position && r.mode == p.mode);
                if let Some(r) = p.record {
                    taken[r] = true;
                }
            }
        }
        for n in 0..self.placed.len() {
            let Some(r) = self.placed[n].record.and_then(|r| records.get(r)) else {
                continue;
            };
            let look = RecordLook {
                permutation: r.permutation,
                next_permutation: r.next_permutation,
                transition: r.transition,
                fade: r.fade,
                yaw: r.yaw,
                pitch: r.pitch,
            };
            if self.placed[n].look == Some(look) {
                continue;
            }
            self.placed[n].look = Some(look);
            let mode = self.placed[n].mode;
            let draws = RecordDraws::of_record(
                look.next_permutation,
                look.permutation,
                look.fade,
                look.transition,
            )
            .resolve(|p| self.kind_of(map, mode, usize::try_from(p).ok()?));
            let Some(draws) = draws else {
                continue;
            };
            let p = &mut self.placed[n];
            let turned = p.angles != (look.yaw, look.pitch);
            if turned {
                let k = &self.kinds[draws.first.0];
                p.angles = (look.yaw, look.pitch);
                p.matrix = tfl_model_matrix(
                    p.position,
                    k.geom.origin,
                    tfl_model_axes(look.yaw, look.pitch),
                    self.cells,
                );
                self.pick.place(n, k, p.matrix);
                self.waves_time = None;
            }
            let mut old = std::mem::take(&mut p.layers);
            for (kind, transparency) in std::iter::once(draws.first).chain(draws.second) {
                let layer = match old.iter().position(|l| l.kind == kind) {
                    Some(i) => {
                        let mut l = old.swap_remove(i);
                        if turned || l.transparency != transparency {
                            l.transparency = transparency;
                            (l.bind_group, l.reflection_bind_group) = Self::bind_groups(
                                device,
                                renderer,
                                &p.matrix,
                                transparency,
                                self.reflection_tint,
                            );
                        }
                        l
                    }
                    None => {
                        self.waves_time = None;
                        Self::layer(
                            device,
                            renderer,
                            &self.kinds[kind],
                            kind,
                            p.index,
                            &p.matrix,
                            (transparency, self.reflection_tint),
                        )
                    }
                };
                p.layers.push(layer);
            }
        }
    }

    fn move_waves(&mut self, queue: &wgpu::Queue, time: FrameTime) {
        let Some(waves) = self.waves else { return };
        if self.waves_time == Some(time) {
            return;
        }
        self.waves_time = Some(time);
        let half_w = self.cells.0 as f32 * TFL_CELL_SIZE * 0.5;
        let half_d = self.cells.1 as f32 * TFL_CELL_SIZE * 0.5;
        let w = WORLD_PER_RENDER_UNIT;
        for (p, layer) in self
            .placed
            .iter()
            .flat_map(|p| p.layers.iter().map(move |l| (p, l)))
        {
            let k = &self.kinds[layer.kind];
            let Some(first) = k
                .geom
                .surfaces
                .iter()
                .zip(&k.slots)
                .filter(|(s, _)| s.flags & GEOMETRY_SURFACE_CATEGORY_2 != 0)
                .map(|(_, slot)| slot.first as usize)
                .min()
            else {
                continue;
            };
            let m = p.matrix;
            let moved: Vec<ModelVertex> = k.vertices[first..]
                .iter()
                .map(|v| {
                    let l = v.position;
                    let r: [f32; 3] = std::array::from_fn(|i| {
                        m[0][i] * l[0] + m[1][i] * l[1] + m[2][i] * l[2] + m[3][i]
                    });
                    let (x, y) = (((half_w - r[0]) * w) as i32, ((r[2] + half_d) * w) as i32);
                    let [dx, dy, dz] = waves.offset_at(time, x, y);
                    let d = [
                        r[0] - dx as f32 / w - m[3][0],
                        r[1] + dz as f32 / w - m[3][1],
                        r[2] + dy as f32 / w - m[3][2],
                    ];
                    let mut out = *v;
                    out.position =
                        std::array::from_fn(|j| m[j][0] * d[0] + m[j][1] * d[1] + m[j][2] * d[2]);
                    out
                })
                .collect();
            layer.mesh.write_vertices(queue, first, &moved);
        }
    }

    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &ModelRenderer,
        time: FrameTime,
    ) {
        self.move_waves(queue, time);
        let time = time.shape_time();
        for (index, p) in self.placed.iter_mut().flat_map(|p| {
            let index = p.index;
            p.layers.iter_mut().map(move |l| (index, l))
        }) {
            let k = &self.kinds[p.kind];
            let Some(coll) = self.collections.get(&k.collection).and_then(Option::as_ref) else {
                continue;
            };
            let mesh = &mut p.mesh;
            for (s, m) in k.materials.iter().enumerate() {
                let bitmap = m.and_then(|m| k.bitmap(coll, m, index, time));
                if p.shown[s] == bitmap && mesh.submeshes[s].texture_bind_group.is_some() {
                    continue;
                }
                p.shown[s] = bitmap;
                let sub = &mut mesh.submeshes[s];
                sub.bitmap_index = bitmap;
                sub.texture_bind_group = bitmap.and_then(|b| {
                    let key = (k.collection, k.palette, b);
                    if let Some(bg) = self.textures.get(&key) {
                        return Some(bg.clone());
                    }
                    let frame = coll.frame(b)?;
                    let img =
                        mandrake_render::frame_to_rgba_image(frame, &self.palettes[k.palette]);
                    let bg = renderer.create_texture_bind_group(
                        device,
                        queue,
                        &img,
                        Some(&format!("TFL {} bitmap {b}", k.collection)),
                    );
                    self.textures.insert(key, bg.clone());
                    Some(bg)
                });
            }
        }
    }

    pub(crate) fn sort(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batch: &mut TranslucentBatch,
        first_pass: &mut TranslucentBatch,
        camera: &Camera,
    ) {
        let eye = camera.eye_position();
        let view = camera.view_matrix().cols;
        let mut keyed: Vec<(f32, usize, Slot)> = Vec::new();
        let mut first: Vec<(f32, usize, Slot)> = Vec::new();
        let mut instance = 0;
        let mut depth_keys: Vec<f32> = Vec::new();
        for (n, p) in self.placed.iter().enumerate() {
            let k = &self.kinds[p.layers[0].kind];
            let keys = self.pick.keys(n, eye, &view);
            depth_keys.clear();
            depth_keys.resize(k.vertices.len(), 0.0);
            for (key, slot) in keys.iter().zip(&k.slots) {
                if let Some(key) = *key {
                    let first = slot.first as usize;
                    if let Some(v) = depth_keys.get_mut(first..first + 3) {
                        v.fill(key);
                    }
                }
            }
            for layer in &p.layers {
                layer.mesh.write_depth_keys(queue, &depth_keys);
            }
            for ((key, slot), s) in keys.iter().zip(&k.slots).zip(&k.geom.surfaces) {
                let Some(key) = *key else { continue };
                let list = if s.flags & GEOMETRY_SURFACE_CATEGORY_2 != 0 {
                    &mut first
                } else {
                    &mut keyed
                };
                for (l, layer) in p.layers.iter().enumerate() {
                    if layer
                        .shown
                        .get(slot.submesh as usize)
                        .copied()
                        .flatten()
                        .is_some()
                    {
                        list.push((key, instance + l, *slot));
                    }
                }
            }
            instance += p.layers.len();
        }
        batch.prepare(device, queue, Self::draw_list(keyed));
        first_pass.prepare(device, queue, Self::draw_list(first));
    }

    fn draw_list(mut keyed: Vec<(f32, usize, Slot)>) -> TranslucentDrawList {
        keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut list = TranslucentDrawList::default();
        for (key, instance, slot) in keyed {
            let start = list.indices.len() as u32;
            list.indices
                .extend([slot.first, slot.first + 1, slot.first + 2]);
            list.keys.push(key);
            let submesh = slot.submesh as usize;
            match list.runs.last_mut() {
                Some(run) if run.instance == instance && run.submesh == submesh => {
                    run.indices.end = start + 3;
                }
                _ => list.runs.push(TranslucentRun {
                    instance,
                    submesh,
                    indices: start..start + 3,
                }),
            }
        }
        list
    }

    pub(crate) fn draws(&self) -> Vec<(&wgpu::BindGroup, &ModelMesh)> {
        self.placed
            .iter()
            .flat_map(|p| p.layers.iter().map(|l| (&l.bind_group, &l.mesh)))
            .collect()
    }

    pub(crate) fn reflect(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batch: &mut mandrake_render::ReflectionBatch,
        camera: &Camera,
    ) {
        let eye = camera.eye_position();
        let view = camera.view_matrix().cols;
        let half_w = self.cells.0 as f32 * TFL_CELL_SIZE * 0.5;
        let half_d = self.cells.1 as f32 * TFL_CELL_SIZE * 0.5;
        let w = WORLD_PER_RENDER_UNIT;
        let mut out: Vec<(f32, mandrake_view::model_surfaces::ReflectionTriangle)> = Vec::new();
        let mut instance = 0;
        for (n, p) in self.placed.iter().enumerate().filter(|(_, p)| p.reflects) {
            let k = &self.kinds[p.layers[0].kind];
            let keys = self.pick.mirrored_keys(n, eye, &view);
            let m = p.matrix;
            let corner = |s: &mandrake_tag::GeometrySurface, v: &ModelVertex| -> ModelVertex {
                let l = v.position;
                let r: [f32; 3] = std::array::from_fn(|i| {
                    m[0][i] * l[0] + m[1][i] * l[1] + m[2][i] * l[2] + m[3][i]
                });
                let mut q = [r[0], -r[1], r[2]];
                if s.flags & GEOMETRY_SURFACE_CATEGORY_2 != 0
                    && let (Some(waves), Some(time)) = (self.waves, self.waves_time)
                {
                    let (x, y) = (((half_w - r[0]) * w) as i32, ((r[2] + half_d) * w) as i32);
                    let [dx, dy, dz] = waves.offset_at(time, x, y);
                    q = [
                        q[0] - dx as f32 / w,
                        q[1] + dz as f32 / w,
                        q[2] + dy as f32 / w,
                    ];
                }
                ModelVertex::new(q, [0.0; 3], v.uv).with_alpha(v.alpha)
            };
            for ((key, slot), (si, s)) in keys
                .iter()
                .zip(&k.slots)
                .zip(k.geom.surfaces.iter().enumerate())
            {
                let Some(key) = *key else { continue };
                let first = slot.first as usize;
                let Some(vs) = k.vertices.get(first..first + 3) else {
                    continue;
                };
                let vertices = [corner(s, &vs[0]), corner(s, &vs[1]), corner(s, &vs[2])];
                for (l, layer) in p.layers.iter().enumerate() {
                    if layer
                        .shown
                        .get(slot.submesh as usize)
                        .copied()
                        .flatten()
                        .is_some()
                    {
                        out.push((
                            key,
                            mandrake_view::model_surfaces::ReflectionTriangle {
                                instance: instance + l,
                                submesh: slot.submesh,
                                surface: si as u16,
                                vertices,
                                key,
                            },
                        ));
                    }
                }
            }
            instance += p.layers.len();
        }
        out.sort_by(|a, b| b.0.total_cmp(&a.0));
        let triangles: Vec<_> = out.into_iter().map(|(_, t)| t).collect();
        batch.prepare(device, queue, &triangles);
    }

    pub(crate) fn reflection_draws(&self) -> Vec<(&wgpu::BindGroup, &ModelMesh)> {
        self.placed
            .iter()
            .filter(|p| p.reflects)
            .flat_map(|p| p.layers.iter().map(|l| (&l.reflection_bind_group, &l.mesh)))
            .collect()
    }
}

fn footprint_reflects(kind: &Kind, marker: &MarkerEntry, map: &TflMap) -> bool {
    let mesh = &map.tfl_mesh;
    let [a, c, _] = tfl_model_axes(marker.yaw as u16, marker.pitch as u16);
    let o = kind.geom.origin;
    let centre = [i32::from(o.0), i32::from(o.1), i32::from(o.2)];
    kind.footprint.iter().any(|&(x, y)| {
        let p = [
            i32::from((i32::from(x) * 0x400 + 0x200) as i16),
            i32::from((i32::from(y) * 0x400 + 0x200) as i16),
            0,
        ];
        let d = [0, 1, 2].map(|i| (p[i] - centre[i]) as f32);
        let dot = |v: [f32; 3]| v[0] * d[0] + v[1] * d[1] + v[2] * d[2];
        let wx = marker.x + centre[0] + dot(a).round() as i32;
        let wy = marker.y + centre[1] + dot(c).round() as i32;
        let (cx, cy) = (wx >> 10, wy >> 10);
        (0..mesh.cell_width as i32).contains(&cx)
            && (0..mesh.cell_height as i32).contains(&cy)
            && mesh.cells[(cy * mesh.cell_width as i32 + cx) as usize].flags & CELL_MODEL_REFLECTS
                != 0
    })
}

const CELL_MODEL_REFLECTS: u8 = 0x20;

fn marker_matrix(kind: &Kind, marker: &MarkerEntry, cells: (u32, u32)) -> [[f32; 4]; 4] {
    let axes = tfl_model_axes(marker.yaw as u16, marker.pitch as u16);
    tfl_model_matrix(
        [marker.x, marker.y, marker.z],
        kind.geom.origin,
        axes,
        cells,
    )
}

#[derive(Debug, Clone)]
pub(crate) struct PickSurface {
    pub(crate) corners: [[f32; 3]; 3],
    normal: [f32; 3],
    flags: u16,
    textured: bool,
}

#[derive(Debug, Default)]
pub(crate) struct ModelPick {
    models: Vec<(Vec<PickSurface>, Vec<Vec<u16>>)>,
    extents: Vec<[f32; 4]>,
    cells: (u32, u32),
    waves: Option<Waves>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ModelHit {
    pub(crate) key: f32,
    pub(crate) point: [i32; 3],
}

impl ModelPick {
    fn of(
        kinds: &[Kind],
        placed: impl Iterator<Item = (usize, [[f32; 4]; 4])>,
        cells: (u32, u32),
        waves: Option<Waves>,
    ) -> Self {
        let mut out = Self {
            models: Vec::new(),
            extents: Vec::new(),
            cells,
            waves,
        };
        for (kind, m) in placed {
            let model = Self::model(&kinds[kind], m);
            out.extents.push(extent(&model.0));
            out.models.push(model);
        }
        out
    }

    fn place(&mut self, n: usize, k: &Kind, m: [[f32; 4]; 4]) {
        if let Some(model) = self.models.get_mut(n) {
            *model = Self::model(k, m);
            if let Some(e) = self.extents.get_mut(n) {
                *e = extent(&model.0);
            }
        }
    }

    fn model(k: &Kind, m: [[f32; 4]; 4]) -> (Vec<PickSurface>, Vec<Vec<u16>>) {
        let world = |v: [f32; 3], w: f32| -> [f32; 3] {
            std::array::from_fn(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2] + m[3][r] * w)
        };
        let surfaces = k
            .geom
            .surfaces
            .iter()
            .zip(&k.slots)
            .map(|(s, slot)| PickSurface {
                corners: std::array::from_fn(|c| {
                    world(k.vertices[(slot.first + c as u32) as usize].position, 1.0)
                }),
                normal: world(k.vertices[slot.first as usize].normal, 0.0),
                flags: s.flags,
                textured: usize::try_from(s.material_index)
                    .ok()
                    .and_then(|m| k.geom.materials.get(m))
                    .is_some_and(|m| m.texture_index >= 0),
            })
            .collect();
        let lists = (0..k.geom.surfaces.len())
            .map(|i| k.geom.draw_after(i))
            .collect();
        (surfaces, lists)
    }

    fn corners_at(&self, s: &PickSurface, time: u32) -> [[f32; 3]; 3] {
        let Some(waves) = self
            .waves
            .filter(|_| s.flags & GEOMETRY_SURFACE_CATEGORY_2 != 0)
        else {
            return s.corners;
        };
        let half_w = self.cells.0 as f32 * TFL_CELL_SIZE * 0.5;
        let half_d = self.cells.1 as f32 * TFL_CELL_SIZE * 0.5;
        let w = WORLD_PER_RENDER_UNIT;
        s.corners.map(|r| {
            let (x, y) = (((half_w - r[0]) * w) as i32, ((r[2] + half_d) * w) as i32);
            let [dx, dy, dz] = waves.offset(Waves::phase(time, x, y));
            [
                r[0] - dx as f32 / w,
                r[1] + dz as f32 / w,
                r[2] + dy as f32 / w,
            ]
        })
    }

    pub(crate) fn keys(&self, n: usize, eye: [f32; 3], view: &[[f32; 4]; 4]) -> Vec<Option<f32>> {
        self.keys_of(n, eye, view, false)
    }

    pub(crate) fn mirrored_keys(
        &self,
        n: usize,
        eye: [f32; 3],
        view: &[[f32; 4]; 4],
    ) -> Vec<Option<f32>> {
        self.keys_of(n, eye, view, true)
    }

    fn keys_of(
        &self,
        n: usize,
        eye: [f32; 3],
        view: &[[f32; 4]; 4],
        mirrored: bool,
    ) -> Vec<Option<f32>> {
        let Some((surfaces, lists)) = self.models.get(n) else {
            return Vec::new();
        };
        let m = |q: [f32; 3]| if mirrored { [q[0], -q[1], q[2]] } else { q };
        let unit = 1.0 / WORLD_PER_RENDER_UNIT;
        let depth =
            |q: [f32; 3]| -(view[0][2] * q[0] + view[1][2] * q[1] + view[2][2] * q[2] + view[3][2]);
        let mut keys: Vec<Option<f32>> = surfaces
            .iter()
            .map(|s| {
                let (normal, corners) = (m(s.normal), s.corners.map(m));
                let faces = (0..3)
                    .map(|a| normal[a] * (eye[a] - corners[0][a]))
                    .sum::<f32>()
                    > 0.0;
                if !faces && s.flags & GEOMETRY_SURFACE_TWO_SIDED == 0 {
                    return None;
                }
                let d = corners.map(depth);
                let mut key = if s.flags & GEOMETRY_SURFACE_DEPTH_NEAREST != 0 {
                    d.into_iter().fold(f32::INFINITY, f32::min)
                } else {
                    d.into_iter().fold(f32::NEG_INFINITY, f32::max)
                };
                if s.flags & GEOMETRY_SURFACE_DEPTH_FARTHER != 0 {
                    key += 0x400 as f32 * unit;
                }
                (key > 0.0).then_some(key)
            })
            .collect();
        for i in 0..keys.len() {
            let Some(mut key) = keys[i] else { continue };
            for &l in &lists[i] {
                if let Some(lk) = keys.get(usize::from(l)).copied().flatten()
                    && key >= lk
                {
                    key = lk - unit;
                }
            }
            keys[i] = Some(key);
        }
        keys
    }

    pub(crate) fn pick(
        &self,
        camera: &Camera,
        size: (u32, u32),
        cursor: (i32, i32),
        time: u32,
    ) -> Option<ModelHit> {
        let eye = camera.eye_position();
        let view = camera.view_matrix().cols;
        let vp = camera.view_proj_matrix(size.0 as f32 / size.1.max(1) as f32);
        let screen = |q: [f32; 3]| -> [f32; 2] {
            let n = vp.transform_point3(q);
            [
                (n[0] + 1.0) * 0.5 * size.0 as f32,
                (1.0 - n[1]) * 0.5 * size.1 as f32,
            ]
        };
        let depth =
            |q: [f32; 3]| -(view[0][2] * q[0] + view[1][2] * q[1] + view[2][2] * q[2] + view[3][2]);
        let p = [cursor.0 as f32, cursor.1 as f32];
        let mut best: Option<(f32, [f32; 3])> = None;
        for (n, (surfaces, _)) in self.models.iter().enumerate() {
            for (s, key) in surfaces.iter().zip(self.keys(n, eye, &view)) {
                let Some(key) = key else { continue };
                if !s.textured || best.is_some_and(|(b, _)| key > b) {
                    continue;
                }
                let corners = self.corners_at(s, time);
                if corners.iter().any(|&q| depth(q) <= 0.0) {
                    continue;
                }
                let on_screen = corners.map(screen);
                if on_screen
                    .iter()
                    .flatten()
                    .any(|c| !(-32768.0 < *c && *c < 32767.0))
                {
                    continue;
                }
                let Some(w) = barycentric(on_screen, p) else {
                    continue;
                };
                let at = std::array::from_fn(|i| {
                    w[0] * corners[0][i] + w[1] * corners[1][i] + w[2] * corners[2][i]
                });
                best = Some((key, at));
            }
        }
        let (key, r) = best?;
        let half_w = self.cells.0 as f32 * TFL_CELL_SIZE * 0.5;
        let half_d = self.cells.1 as f32 * TFL_CELL_SIZE * 0.5;
        let w = WORLD_PER_RENDER_UNIT;
        Some(ModelHit {
            key,
            point: [
                ((half_w - r[0]) * w) as i32,
                ((r[2] + half_d) * w) as i32,
                (r[1] * w) as i32,
            ],
        })
    }
}

impl ModelPick {}

impl ModelPick {
    pub(crate) fn height_at(&self, x: i32, y: i32) -> Option<i32> {
        let half_w = self.cells.0 as f32 * TFL_CELL_SIZE * 0.5;
        let half_d = self.cells.1 as f32 * TFL_CELL_SIZE * 0.5;
        let w = WORLD_PER_RENDER_UNIT;
        let p = [half_w - x as f32 / w, y as f32 / w - half_d];
        self.models
            .iter()
            .zip(&self.extents)
            .filter(|(_, e)| e[0] <= p[0] && p[0] <= e[2] && e[1] <= p[1] && p[1] <= e[3])
            .flat_map(|((s, _), _)| s)
            .filter_map(|s| {
                let c = s.corners;
                let t = c.map(|q| [q[0], q[2]]);
                let k = barycentric(t, p)?;
                Some(k[0] * c[0][1] + k[1] * c[1][1] + k[2] * c[2][1])
            })
            .reduce(f32::max)
            .map(|h| (h * w).round() as i32)
    }
}

fn extent(surfaces: &[PickSurface]) -> [f32; 4] {
    surfaces.iter().flat_map(|s| s.corners).fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |e, q| {
            [
                e[0].min(q[0]),
                e[1].min(q[2]),
                e[2].max(q[0]),
                e[3].max(q[2]),
            ]
        },
    )
}

fn barycentric(t: [[f32; 2]; 3], p: [f32; 2]) -> Option<[f32; 3]> {
    let [a, b, c] = t;
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
    if area.abs() < f32::EPSILON {
        return None;
    }
    let wa = ((b[0] - p[0]) * (c[1] - p[1]) - (c[0] - p[0]) * (b[1] - p[1])) / area;
    let wb = ((c[0] - p[0]) * (a[1] - p[1]) - (a[0] - p[0]) * (c[1] - p[1])) / area;
    let wc = 1.0 - wa - wb;
    (wa >= 0.0 && wb >= 0.0 && wc >= 0.0).then_some([wa, wb, wc])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quarter_turns_place_a_vertex_as_tfl_does() {
        let axes = tfl_model_axes(0x4000, 0xc000);
        for (r, want) in axes
            .iter()
            .zip([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
        {
            for (x, w) in r.iter().zip(want) {
                assert!((x - w).abs() < 1e-6, "{axes:?}");
            }
        }
        let m = tfl_model_matrix([1024, 2048, 512], (0x800, 0x800, 0x200), axes, (80, 80));
        let local = [2.0f32, 0.0, 1.0, 1.0];
        let r: Vec<f32> = (0..3)
            .map(|i| (0..4).map(|j| m[j][i] * local[j]).sum())
            .collect();
        let want = [80.0 - 3584.0 / 512.0, 1024.0 / 512.0, 5120.0 / 512.0 - 80.0];
        for (x, w) in r.iter().zip(want) {
            assert!((x - w).abs() < 1e-5, "{r:?} {want:?}");
        }
    }
}
