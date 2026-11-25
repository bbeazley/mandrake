pub(crate) use mandrake_view::model::*;

use crate::billboard::{TextureAtlas, frame_to_rgba_image};
use crate::camera::Mat4;
use crate::pipeline::RenderPipelineConfig;
use crate::vertex::VertexLayout;
use bytemuck::{Pod, Zeroable};
use mandrake_tag::{GeometryTag, ModelPermutation, collection::CollectionTag};
use wgpu::util::DeviceExt;

pub const MODEL_VERTEX_ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2,
    3 => Float32,
];

impl VertexLayout for ModelVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MODEL_VERTEX_ATTRIBS,
        }
    }
}

pub const MODEL_SLOT0_TINT: [u8; 8] = [0xff, 0, 0, 0, 0, 0, 0, 0];

pub const MODEL_TEXTURE_ANISOTROPY: u16 = 8;

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct ModelUniforms {
    pub model_matrix: [[f32; 4]; 4],
    pub tint: [f32; 4],
    pub shade_kpqr: [f32; 4],
    pub shade_xyz: [f32; 4],
}

impl Default for ModelUniforms {
    fn default() -> Self {
        Self {
            model_matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            tint: [1.0, 1.0, 1.0, 1.0],
            shade_kpqr: crate::media::tint_factors(MODEL_SLOT0_TINT).0,
            shade_xyz: crate::media::tint_factors(MODEL_SLOT0_TINT).1,
        }
    }
}

impl ModelUniforms {
    pub fn from_matrix(matrix: [[f32; 4]; 4]) -> Self {
        Self {
            model_matrix: matrix,
            ..Default::default()
        }
    }

    pub fn from_translation_and_scale(pos: [f32; 3], scale: [f32; 3]) -> Self {
        Self {
            model_matrix: [
                [scale[0], 0.0, 0.0, 0.0],
                [0.0, scale[1], 0.0, 0.0],
                [0.0, 0.0, scale[2], 0.0],
                [pos[0], pos[1], pos[2], 1.0],
            ],
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct ModelSubMesh {
    pub index_offset: u32,
    pub index_count: u32,
    pub bitmap_index: Option<usize>,
    pub texture_bind_group: Option<wgpu::BindGroup>,
    pub two_sided: bool,
    pub translucent: bool,
    pub category2: bool,
    pub material: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelSubmeshes {
    #[default]
    ByBitmap,
    ByMaterial,
}

pub struct ModelMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    pub submeshes: Vec<ModelSubMesh>,
    pub texture_bind_group: Option<wgpu::BindGroup>,
    pub translucent_triangles: Vec<TranslucentTriangle>,
    pub engine: Option<crate::model_surfaces::EngineGeometry>,
    pub cpu_vertices: Vec<ModelVertex>,
    pub texture_words: Vec<u32>,
    pub depth_keys: Option<wgpu::Buffer>,
}

const DEPTH_KEY_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: core::mem::size_of::<f32>() as wgpu::BufferAddress,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &[wgpu::VertexAttribute {
        offset: 0,
        shader_location: 4,
        format: wgpu::VertexFormat::Float32,
    }],
};

impl ModelMesh {
    pub fn write_vertices(&self, queue: &wgpu::Queue, first: usize, vertices: &[ModelVertex]) {
        queue.write_buffer(
            &self.vertex_buffer,
            (first * core::mem::size_of::<ModelVertex>()) as u64,
            bytemuck::cast_slice(vertices),
        );
    }

    pub fn from_raw(
        device: &wgpu::Device,
        label: &str,
        vertices: &[ModelVertex],
        indices: &[u32],
    ) -> Self {
        Self::from_submeshes(device, label, vertices, indices, Vec::new())
    }

    pub fn from_submeshes(
        device: &wgpu::Device,
        label: &str,
        vertices: &[ModelVertex],
        indices: &[u32],
        submeshes: Vec<ModelSubMesh>,
    ) -> Self {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{} Vertex Buffer", label)),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{} Index Buffer", label)),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });

        let translucent_triangles = TranslucentTriangle::collect(vertices, indices, &submeshes);

        Self {
            vertex_buffer,
            index_buffer,
            num_indices: indices.len() as u32,
            submeshes,
            texture_bind_group: None,
            translucent_triangles,
            engine: None,
            cpu_vertices: vertices.to_vec(),
            texture_words: Vec::new(),
            depth_keys: None,
        }
    }

    pub fn with_depth_keys(mut self, device: &wgpu::Device) -> Self {
        self.depth_keys = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Model Depth Keys"),
            size: (self.cpu_vertices.len().max(1) * core::mem::size_of::<f32>())
                as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self
    }

    pub fn write_depth_keys(&self, queue: &wgpu::Queue, keys: &[f32]) {
        if let Some(buffer) = &self.depth_keys {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(keys));
        }
    }

    pub fn with_engine_geometry(
        mut self,
        engine: crate::model_surfaces::EngineGeometry,
        texture_words: Vec<u32>,
    ) -> Self {
        self.engine = Some(engine);
        self.texture_words = texture_words;
        self
    }

    pub fn with_texture_bind_group(mut self, bind_group: wgpu::BindGroup) -> Self {
        self.texture_bind_group = Some(bind_group);
        self
    }

    pub fn with_submeshes(mut self, submeshes: Vec<ModelSubMesh>) -> Self {
        self.submeshes = submeshes;
        self
    }

    pub fn build_geometry_buffers(
        geom: &GeometryTag,
        atlas: Option<&TextureAtlas>,
    ) -> (Vec<ModelVertex>, Vec<u32>, Vec<ModelSubMesh>) {
        let frames_meta: Option<(u32, u32, &[crate::billboard::AtlasFrame])> =
            atlas.map(|a| (a.width, a.height, a.frames.as_slice()));
        Self::build_geometry_buffers_with_frames(geom, frames_meta)
    }

    pub fn build_geometry_buffers_full(
        geom: &GeometryTag,
        collection: Option<&CollectionTag>,
        atlas_frames: Option<(u32, u32, &[crate::billboard::AtlasFrame])>,
    ) -> (Vec<ModelVertex>, Vec<u32>, Vec<ModelSubMesh>) {
        Self::build_geometry_buffers_permuted(geom, collection, None, atlas_frames)
    }

    pub fn build_geometry_buffers_permuted(
        geom: &GeometryTag,
        collection: Option<&CollectionTag>,
        permutation: Option<&ModelPermutation>,
        atlas_frames: Option<(u32, u32, &[crate::billboard::AtlasFrame])>,
    ) -> (Vec<ModelVertex>, Vec<u32>, Vec<ModelSubMesh>) {
        let (v, i, s, _) =
            Self::build_geometry_buffers_engine(geom, collection, permutation, atlas_frames);
        (v, i, s)
    }

    pub fn build_geometry_buffers_engine(
        geom: &GeometryTag,
        collection: Option<&CollectionTag>,
        permutation: Option<&ModelPermutation>,
        atlas_frames: Option<(u32, u32, &[crate::billboard::AtlasFrame])>,
    ) -> (
        Vec<ModelVertex>,
        Vec<u32>,
        Vec<ModelSubMesh>,
        crate::model_surfaces::EngineGeometry,
    ) {
        Self::build_geometry_buffers_grouped(
            geom,
            collection,
            permutation,
            atlas_frames,
            ModelSubmeshes::ByBitmap,
        )
    }

    pub fn build_geometry_buffers_grouped(
        geom: &GeometryTag,
        collection: Option<&CollectionTag>,
        permutation: Option<&ModelPermutation>,
        atlas_frames: Option<(u32, u32, &[crate::billboard::AtlasFrame])>,
        grouping: ModelSubmeshes,
    ) -> (
        Vec<ModelVertex>,
        Vec<u32>,
        Vec<ModelSubMesh>,
        crate::model_surfaces::EngineGeometry,
    ) {
        let mut slots = vec![(0u32, 0u32); geom.surfaces.len()];
        let mut vertices = Vec::with_capacity(geom.surfaces.len() * 3);
        let mut indices = Vec::with_capacity(geom.surfaces.len() * 3);
        let mut submeshes: Vec<ModelSubMesh> = Vec::new();

        let is_category2 = |s: &mandrake_tag::GeometrySurface| {
            s.flags & mandrake_tag::model::GEOMETRY_SURFACE_CATEGORY_2 != 0
        };
        let mut surface_order: Vec<usize> = (0..geom.surfaces.len()).collect();
        surface_order.sort_by_key(|&i| {
            let s = &geom.surfaces[i];
            (is_category2(s), s.is_translucent(), s.material_index)
        });

        for surf_idx in surface_order {
            let surface = &geom.surfaces[surf_idx];
            let n_raw = [
                surface.normal.1 as f32,
                surface.normal.2 as f32,
                surface.normal.0 as f32,
            ];
            let len = (n_raw[0] * n_raw[0] + n_raw[1] * n_raw[1] + n_raw[2] * n_raw[2]).sqrt();
            let normal = if len > 1e-4 {
                [n_raw[0] / len, n_raw[1] / len, n_raw[2] / len]
            } else {
                [0.0, 1.0, 0.0]
            };

            let mat = if surface.material_index >= 0 {
                geom.materials.get(surface.material_index as usize)
            } else {
                None
            };

            let bitmap_index = mat.and_then(|m| {
                if let Some(coll) = collection {
                    return geom.material_bitmap_index(
                        surface.material_index as usize,
                        coll,
                        permutation,
                    );
                }
                if m.texture_index >= 0 {
                    Some(m.texture_index as usize)
                } else {
                    None
                }
            });

            let two_sided = mat.map(|m| m.flags != 0).unwrap_or(false)
                || surface.vertices.iter().any(|sv| sv.flags != 0);

            let category2 = is_category2(surface);
            let translucent = surface.is_translucent() && !category2;
            let alpha = surface.opacity();

            let base_vertex = vertices.len() as u32;
            let index_offset = indices.len() as u32;

            for sv in &surface.vertices {
                let v = geom
                    .vertices
                    .get(sv.vertex_index as usize)
                    .copied()
                    .unwrap_or_default();

                let (vx, vy, vz) = v.to_renderer_space(geom.origin);

                let u_raw = sv.u_f32().clamp(0.0, 1.0);
                let v_raw = sv.v_f32().clamp(0.0, 1.0);

                let uv = if let (Some((_, _, frames)), Some(b_idx)) = (atlas_frames, bitmap_index) {
                    if let Some(frame) = frames.get(b_idx) {
                        let u = frame.uv_min[0] + u_raw * (frame.uv_max[0] - frame.uv_min[0]);
                        let v_coord = frame.uv_min[1] + v_raw * (frame.uv_max[1] - frame.uv_min[1]);
                        [u, v_coord]
                    } else {
                        [u_raw, v_raw]
                    }
                } else {
                    [u_raw, v_raw]
                };

                vertices.push(ModelVertex::new([vx, vy, vz], normal, uv).with_alpha(alpha));
            }

            indices.extend_from_slice(&[base_vertex, base_vertex + 1, base_vertex + 2]);

            let material = usize::try_from(surface.material_index).ok();
            let same_group = |last: &ModelSubMesh| match grouping {
                ModelSubmeshes::ByBitmap => last.bitmap_index == bitmap_index,
                ModelSubmeshes::ByMaterial => last.material == material,
            };
            if let Some(last) = submeshes.last_mut()
                && same_group(last)
                && last.two_sided == two_sided
                && last.translucent == translucent
                && last.category2 == category2
            {
                last.index_count += 3;
            } else {
                submeshes.push(ModelSubMesh {
                    index_offset,
                    index_count: 3,
                    bitmap_index,
                    texture_bind_group: None,
                    two_sided,
                    translucent,
                    category2,
                    material,
                });
            }
            slots[surf_idx] = (base_vertex, submeshes.len() as u32 - 1);
        }

        let mut engine = crate::model_surfaces::EngineGeometry::new(geom, &slots);
        if let Some(collection) = collection {
            engine.resolve_materials(geom, collection, permutation);
        }
        (vertices, indices, submeshes, engine)
    }

    pub fn build_geometry_buffers_with_frames(
        geom: &GeometryTag,
        atlas_frames: Option<(u32, u32, &[crate::billboard::AtlasFrame])>,
    ) -> (Vec<ModelVertex>, Vec<u32>, Vec<ModelSubMesh>) {
        Self::build_geometry_buffers_full(geom, None, atlas_frames)
    }

    pub fn from_geometry(
        device: &wgpu::Device,
        label: &str,
        geom: &GeometryTag,
        atlas: Option<&TextureAtlas>,
    ) -> Self {
        let frames_meta: Option<(u32, u32, &[crate::billboard::AtlasFrame])> =
            atlas.map(|a| (a.width, a.height, a.frames.as_slice()));
        let (vertices, indices, submeshes, engine) =
            Self::build_geometry_buffers_engine(geom, None, None, frames_meta);
        let texture_words = submeshes
            .iter()
            .map(|s| crate::model_surfaces::category2_texture_word(0, s.bitmap_index, 0))
            .collect();
        Self::from_submeshes(device, label, &vertices, &indices, submeshes)
            .with_engine_geometry(engine, texture_words)
    }

    pub fn from_geometry_with_collection(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &ModelRenderer,
        label: &str,
        geom: &GeometryTag,
        collection: &CollectionTag,
        permutation: Option<&ModelPermutation>,
    ) -> Self {
        let (vertices, indices, mut submeshes, engine) =
            Self::build_geometry_buffers_engine(geom, Some(collection), permutation, None);
        let color_table = permutation.map_or(0, |p| p.color_table);
        let texture_words = submeshes
            .iter()
            .map(|s| crate::model_surfaces::category2_texture_word(0, s.bitmap_index, color_table))
            .collect();

        let mut uploaded: std::collections::HashMap<usize, wgpu::BindGroup> =
            std::collections::HashMap::new();
        for sub in &mut submeshes {
            if let Some(b_idx) = sub.bitmap_index {
                if let Some(bg) = uploaded.get(&b_idx) {
                    sub.texture_bind_group = Some(bg.clone());
                } else if let Some(frame) = collection.frame(b_idx) {
                    let rgba_img = frame_to_rgba_image(frame, collection.palette());
                    let bg_label = format!("{} Material Bitmap #{}", label, b_idx);
                    let bg = renderer.create_texture_bind_group(
                        device,
                        queue,
                        &rgba_img,
                        Some(&bg_label),
                    );
                    uploaded.insert(b_idx, bg.clone());
                    sub.texture_bind_group = Some(bg);
                }
            }
        }

        Self::from_submeshes(device, label, &vertices, &indices, submeshes)
            .with_engine_geometry(engine, texture_words)
    }

    pub fn has_translucent(&self) -> bool {
        self.submeshes
            .iter()
            .any(|s| s.translucent && s.index_count > 0)
    }

    pub fn draw<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        if self.num_indices == 0 {
            return;
        }
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.num_indices, 0, 0..1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TranslucentTriangle {
    pub submesh: u32,
    pub indices: [u32; 3],
    pub positions: [[f32; 3]; 3],
}

impl TranslucentTriangle {
    pub fn collect(
        vertices: &[ModelVertex],
        indices: &[u32],
        submeshes: &[ModelSubMesh],
    ) -> Vec<Self> {
        let mut out = Vec::new();
        for (s, sub) in submeshes.iter().enumerate() {
            if !sub.translucent {
                continue;
            }
            let start = sub.index_offset as usize;
            let end = (start + sub.index_count as usize).min(indices.len());
            for &idx in indices[start.min(end)..end].as_chunks::<3>().0 {
                let pos = |i: u32| {
                    vertices
                        .get(i as usize)
                        .map_or([0.0; 3], |v: &ModelVertex| v.position)
                };
                out.push(Self {
                    submesh: s as u32,
                    indices: idx,
                    positions: [pos(idx[0]), pos(idx[1]), pos(idx[2])],
                });
            }
        }
        out
    }
}

pub fn sort_translucent_triangles<'a>(
    instances: impl IntoIterator<Item = ([[f32; 4]; 4], &'a [TranslucentTriangle])>,
    view: &Mat4,
) -> TranslucentDrawList {
    sort_triangles(instances, view, None)
}

pub fn sort_translucent_triangles_on_screen<'a>(
    instances: impl IntoIterator<Item = ([[f32; 4]; 4], &'a [TranslucentTriangle])>,
    view: &Mat4,
    view_proj: &Mat4,
) -> TranslucentDrawList {
    sort_triangles(instances, view, Some(view_proj))
}

#[derive(Clone, Copy)]
struct KeyedTriangle {
    key: f32,
    instance: usize,
    submesh: u32,
    indices: [u32; 3],
    bounds: Option<[f32; 4]>,
}

#[allow(clippy::too_many_arguments)]
fn sort_triangles_into<'a>(
    instances: impl IntoIterator<Item = ([[f32; 4]; 4], &'a [TranslucentTriangle])>,
    view: &Mat4,
    view_proj: Option<&Mat4>,
    keyed: &mut Vec<KeyedTriangle>,
    order: &mut Vec<u64>,
    indices: &mut Vec<u32>,
    runs: &mut Vec<TranslucentRun>,
    keys: &mut Vec<f32>,
    bounds: &mut Vec<Option<[f32; 4]>>,
) {
    keyed.clear();
    for (i, (model_matrix, tris)) in instances.into_iter().enumerate() {
        if tris.is_empty() {
            continue;
        }
        let model = Mat4 { cols: model_matrix };
        let model_view = view.mul_mat4(&model);
        let model_view_proj = view_proj.map(|vp| vp.mul_mat4(&model));
        let c = model_view.cols;
        let distance = |p: [f32; 3]| -(c[0][2] * p[0] + c[1][2] * p[1] + c[2][2] * p[2] + c[3][2]);
        for tri in tris {
            let key = tri
                .positions
                .iter()
                .map(|&p| distance(p))
                .fold(f32::NEG_INFINITY, f32::max);
            let tri_bounds = model_view_proj
                .as_ref()
                .and_then(|mvp| crate::billboard::ndc_rect(mvp, tri.positions));
            keyed.push(KeyedTriangle {
                key,
                instance: i,
                submesh: tri.submesh,
                indices: tri.indices,
                bounds: tri_bounds,
            });
        }
    }
    order.clear();
    order.reserve(keyed.len());
    for (n, k) in keyed.iter().enumerate() {
        let bits = k.key.to_bits();
        let ascending = if bits >> 31 == 1 {
            !bits
        } else {
            bits | 1 << 31
        };
        order.push((u64::from(!ascending) << 32) | n as u64);
    }
    order.sort_unstable();

    indices.clear();
    indices.reserve(keyed.len() * 3);
    runs.clear();
    keys.clear();
    keys.reserve(keyed.len());
    bounds.clear();
    if view_proj.is_some() {
        bounds.reserve(keyed.len());
    }
    for &n in order.iter() {
        let entry = keyed[(n & 0xffff_ffff) as usize];
        keys.push(entry.key);
        if view_proj.is_some() {
            bounds.push(entry.bounds);
        }
        let start = indices.len() as u32;
        indices.extend_from_slice(&entry.indices);
        let submesh = entry.submesh as usize;
        match runs.last_mut() {
            Some(run) if run.instance == entry.instance && run.submesh == submesh => {
                run.indices.end = start + 3;
            }
            _ => runs.push(TranslucentRun {
                instance: entry.instance,
                submesh,
                indices: start..start + 3,
            }),
        }
    }
}

fn sort_triangles<'a>(
    instances: impl IntoIterator<Item = ([[f32; 4]; 4], &'a [TranslucentTriangle])>,
    view: &Mat4,
    view_proj: Option<&Mat4>,
) -> TranslucentDrawList {
    let mut keyed = Vec::new();
    let mut order = Vec::new();
    let mut list = TranslucentDrawList {
        indices: Vec::new(),
        runs: Vec::new(),
        keys: Vec::new(),
        bounds: Vec::new(),
        view_proj: view_proj.copied(),
    };
    sort_triangles_into(
        instances,
        view,
        view_proj,
        &mut keyed,
        &mut order,
        &mut list.indices,
        &mut list.runs,
        &mut list.keys,
        &mut list.bounds,
    );
    list
}

pub struct TranslucentBatch {
    index_buffer: wgpu::Buffer,
    capacity: usize,
    runs: Vec<TranslucentRun>,
    keys: Vec<f32>,
    bounds: Vec<Option<[f32; 4]>>,
    view_proj: Option<Mat4>,
    keyed_scratch: Vec<KeyedTriangle>,
    order_scratch: Vec<u64>,
    indices_scratch: Vec<u32>,
}

impl TranslucentBatch {
    const INITIAL_CAPACITY: usize = 1024;

    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            index_buffer: Self::create_buffer(device, Self::INITIAL_CAPACITY),
            capacity: Self::INITIAL_CAPACITY,
            runs: Vec::new(),
            keys: Vec::new(),
            bounds: Vec::new(),
            view_proj: None,
            keyed_scratch: Vec::new(),
            order_scratch: Vec::new(),
            indices_scratch: Vec::new(),
        }
    }

    fn create_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Translucent Model Index Buffer"),
            size: (capacity * core::mem::size_of::<u32>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        list: TranslucentDrawList,
    ) {
        if list.indices.len() > self.capacity {
            self.capacity = list.indices.len().next_power_of_two();
            self.index_buffer = Self::create_buffer(device, self.capacity);
        }
        if !list.indices.is_empty() {
            queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&list.indices));
        }
        self.runs = list.runs;
        self.keys = list.keys;
        self.bounds = list.bounds;
        self.view_proj = list.view_proj;
    }

    pub fn prepare_sorted<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: impl IntoIterator<Item = ([[f32; 4]; 4], &'a [TranslucentTriangle])>,
        view: &Mat4,
    ) {
        self.prepare_sorted_inner(device, queue, instances, view, None);
    }

    pub fn prepare_sorted_on_screen<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: impl IntoIterator<Item = ([[f32; 4]; 4], &'a [TranslucentTriangle])>,
        view: &Mat4,
        view_proj: &Mat4,
    ) {
        self.prepare_sorted_inner(device, queue, instances, view, Some(view_proj));
    }

    fn prepare_sorted_inner<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: impl IntoIterator<Item = ([[f32; 4]; 4], &'a [TranslucentTriangle])>,
        view: &Mat4,
        view_proj: Option<&Mat4>,
    ) {
        sort_triangles_into(
            instances,
            view,
            view_proj,
            &mut self.keyed_scratch,
            &mut self.order_scratch,
            &mut self.indices_scratch,
            &mut self.runs,
            &mut self.keys,
            &mut self.bounds,
        );
        self.view_proj = view_proj.copied();
        if self.indices_scratch.len() > self.capacity {
            self.capacity = self.indices_scratch.len().next_power_of_two();
            self.index_buffer = Self::create_buffer(device, self.capacity);
        }
        if !self.indices_scratch.is_empty() {
            queue.write_buffer(
                &self.index_buffer,
                0,
                bytemuck::cast_slice(&self.indices_scratch),
            );
        }
    }

    pub fn keys(&self) -> &[f32] {
        &self.keys
    }

    pub fn view_proj(&self) -> Option<&Mat4> {
        self.view_proj.as_ref()
    }

    pub fn screen_bounds(&self, triangles: core::ops::Range<u32>) -> Option<[f32; 4]> {
        let rects = self
            .bounds
            .get(triangles.start as usize..triangles.end as usize)?;
        let mut out = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for r in rects {
            let r = (*r)?;
            out = [
                out[0].min(r[0]),
                out[1].min(r[1]),
                out[2].max(r[2]),
                out[3].max(r[3]),
            ];
        }
        Some(out)
    }

    pub fn runs(&self) -> &[TranslucentRun] {
        &self.runs
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReflectionRun {
    pub instance: usize,
    pub submesh: usize,
    pub vertices: core::ops::Range<u32>,
}

pub struct ReflectionBatch {
    vertex_buffer: wgpu::Buffer,
    capacity: usize,
    vertices: Vec<ModelVertex>,
    runs: Vec<ReflectionRun>,
    keys: Vec<f32>,
}

impl ReflectionBatch {
    const INITIAL_CAPACITY: usize = 768;

    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            vertex_buffer: Self::create_buffer(device, Self::INITIAL_CAPACITY),
            capacity: Self::INITIAL_CAPACITY,
            vertices: Vec::new(),
            runs: Vec::new(),
            keys: Vec::new(),
        }
    }

    fn create_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Model Reflection Vertex Buffer"),
            size: (capacity * core::mem::size_of::<ModelVertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        triangles: &[crate::model_surfaces::ReflectionTriangle],
    ) {
        self.vertices.clear();
        self.vertices.reserve(triangles.len() * 3);
        self.runs.clear();
        self.keys.clear();
        self.keys.reserve(triangles.len());
        for (k, t) in triangles.iter().enumerate() {
            self.vertices.extend_from_slice(&t.vertices);
            self.keys.push(t.key);
            let start = 3 * k as u32;
            match self.runs.last_mut() {
                Some(r) if r.instance == t.instance && r.submesh == t.submesh as usize => {
                    r.vertices.end = start + 3;
                }
                _ => self.runs.push(ReflectionRun {
                    instance: t.instance,
                    submesh: t.submesh as usize,
                    vertices: start..start + 3,
                }),
            }
        }
        if self.vertices.len() > self.capacity {
            self.capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = Self::create_buffer(device, self.capacity);
        }
        if !self.vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        }
    }

    pub fn keys(&self) -> &[f32] {
        &self.keys
    }
}

#[derive(Default)]
pub struct TrianglePassState {
    scene_bound: bool,
    index_bound: bool,
    model: Option<usize>,
}

impl TrianglePassState {
    pub fn scene_to_bind<'a>(&mut self, scene: &'a wgpu::BindGroup) -> Option<&'a wgpu::BindGroup> {
        (!core::mem::replace(&mut self.scene_bound, true)).then_some(scene)
    }
}

struct ModelPipelines {
    pipeline: wgpu::RenderPipeline,
    translucent_pipeline: wgpu::RenderPipeline,
    keyed_pipeline: wgpu::RenderPipeline,
    depth_only_pipeline: wgpu::RenderPipeline,
    category2_pipeline: wgpu::RenderPipeline,
    reflection_pipeline: wgpu::RenderPipeline,
    soft_reflection_pipeline: wgpu::RenderPipeline,
    reflection_format: wgpu::TextureFormat,
    uniform_bind_group_layout: wgpu::BindGroupLayout,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    model_bind_group_layout: wgpu::BindGroupLayout,
    default_texture_bind_group: wgpu::BindGroup,
    surface_sampler: wgpu::Sampler,
}

pub struct ModelRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub translucent_pipeline: wgpu::RenderPipeline,
    pub keyed_pipeline: wgpu::RenderPipeline,
    pub depth_only_pipeline: wgpu::RenderPipeline,
    pub category2_pipeline: wgpu::RenderPipeline,
    pub reflection_pipeline: wgpu::RenderPipeline,
    pub soft_reflection_pipeline: wgpu::RenderPipeline,
    pub reflection_format: wgpu::TextureFormat,
    pub uniform_bind_group_layout: wgpu::BindGroupLayout,
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub model_bind_group_layout: wgpu::BindGroupLayout,
    pub default_texture_bind_group: wgpu::BindGroup,
    surface_sampler: wgpu::Sampler,
    pub default_model_bind_group: wgpu::BindGroup,
    pub default_model_buffer: wgpu::Buffer,
}

impl ModelRenderer {
    pub const MODEL_SHADER_SRC: &'static str = concat!(
        include_str!("../shaders/scene_uniforms.wgsl"),
        include_str!("../shaders/filter_helpers.wgsl"),
        include_str!("../shaders/model.wgsl"),
        include_str!("../shaders/reflection_edge_fade.wgsl"),
        include_str!("../shaders/fog.wgsl")
    );

    fn create_pipeline(
        device: &wgpu::Device,
        config: &RenderPipelineConfig,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        label: &str,
        (blend, write_mask): (wgpu::BlendState, wgpu::ColorWrites),
        depth_write_enabled: bool,
    ) -> wgpu::RenderPipeline {
        Self::create_pipeline_with(
            device,
            config,
            layout,
            shader,
            label,
            (blend, write_mask),
            (depth_write_enabled, wgpu::CompareFunction::Less),
            ("vs_main", "fs_main"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_pipeline_with(
        device: &wgpu::Device,
        config: &RenderPipelineConfig,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        label: &str,
        (blend, write_mask): (wgpu::BlendState, wgpu::ColorWrites),
        (depth_write_enabled, depth_compare): (bool, wgpu::CompareFunction),
        (vertex_entry, fragment_entry): (&str, &str),
    ) -> wgpu::RenderPipeline {
        let depth_stencil = config.depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(depth_write_enabled),
            depth_compare: Some(depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        let keyed_buffers = [Some(ModelVertex::desc()), Some(DEPTH_KEY_LAYOUT)];
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex_entry),
                buffers: if vertex_entry == "vs_keyed" {
                    &keyed_buffers
                } else {
                    &keyed_buffers[..1]
                },
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment_entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.target_format,
                    blend: Some(blend),
                    write_mask,
                })],
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: crate::scene_constants(config.target_format),
                    ..Default::default()
                },
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, config: &RenderPipelineConfig) -> Self {
        Self::with_reflection_format(
            device,
            queue,
            config,
            config.target_format.remove_srgb_suffix(),
        )
    }

    pub fn with_reflection_format(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RenderPipelineConfig,
        reflection_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Model Scene Uniform Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Model Texture Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let model_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Model Uniform Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Model Pipeline Layout"),
            bind_group_layouts: &[
                Some(&uniform_bind_group_layout),
                Some(&texture_bind_group_layout),
                Some(&model_bind_group_layout),
            ],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Model WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(Self::MODEL_SHADER_SRC.into()),
        });

        let pipeline = Self::create_pipeline(
            device,
            config,
            &pipeline_layout,
            &shader,
            "Model Render Pipeline",
            (wgpu::BlendState::REPLACE, wgpu::ColorWrites::ALL),
            true,
        );
        let translucent_pipeline = Self::create_pipeline_with(
            device,
            &RenderPipelineConfig {
                target_format: reflection_format,
                ..*config
            },
            &pipeline_layout,
            &shader,
            "Model Translucent Render Pipeline",
            (wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::ALL),
            (false, wgpu::CompareFunction::LessEqual),
            (
                "vs_main",
                if reflection_format.is_srgb() {
                    "fs_main"
                } else {
                    "fs_media_display"
                },
            ),
        );
        let keyed_pipeline = Self::create_pipeline_with(
            device,
            &RenderPipelineConfig {
                target_format: reflection_format,
                ..*config
            },
            &pipeline_layout,
            &shader,
            "Model Keyed Translucent Render Pipeline",
            (wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::ALL),
            (false, wgpu::CompareFunction::LessEqual),
            (
                "vs_keyed",
                if reflection_format.is_srgb() {
                    "fs_main"
                } else {
                    "fs_media_display"
                },
            ),
        );
        let depth_only_pipeline = Self::create_pipeline(
            device,
            config,
            &pipeline_layout,
            &shader,
            "Model Depth-Only Pipeline",
            (wgpu::BlendState::REPLACE, wgpu::ColorWrites::empty()),
            true,
        );
        let category2_pipeline = Self::create_pipeline(
            device,
            config,
            &pipeline_layout,
            &shader,
            "Model Category 2 Pipeline",
            (wgpu::BlendState::REPLACE, wgpu::ColorWrites::ALL),
            false,
        );
        let reflection_pipeline = Self::create_pipeline_with(
            device,
            &RenderPipelineConfig {
                target_format: reflection_format,
                ..*config
            },
            &pipeline_layout,
            &shader,
            "Model Reflection Pipeline",
            (wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::ALL),
            (false, wgpu::CompareFunction::Always),
            (
                "vs_reflection",
                if reflection_format.is_srgb() {
                    "fs_main"
                } else {
                    "fs_media_display"
                },
            ),
        );
        let soft_reflection_pipeline = Self::create_pipeline_with(
            device,
            &RenderPipelineConfig {
                target_format: reflection_format,
                ..*config
            },
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Model Soft Reflection Pipeline Layout"),
                bind_group_layouts: &[
                    Some(&uniform_bind_group_layout),
                    Some(&texture_bind_group_layout),
                    Some(&model_bind_group_layout),
                    Some(&crate::reflection_edge::ReflectionEdge::bind_group_layout(
                        device,
                    )),
                ],
                immediate_size: 0,
            }),
            &shader,
            "Model Soft Reflection Pipeline",
            (wgpu::BlendState::ALPHA_BLENDING, wgpu::ColorWrites::ALL),
            (false, wgpu::CompareFunction::Always),
            (
                "vs_reflection",
                if reflection_format.is_srgb() {
                    "fs_main_soft"
                } else {
                    "fs_media_display_soft"
                },
            ),
        );

        let default_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Default Model Texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &default_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let default_view = default_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let default_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Model Default Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let default_texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Model Default Texture Bind Group"),
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&default_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&default_sampler),
                },
            ],
        });
        let surface_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Model Surface Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: MODEL_TEXTURE_ANISOTROPY,
            ..Default::default()
        });

        Self::with_pipelines(
            device,
            ModelPipelines {
                pipeline,
                translucent_pipeline,
                keyed_pipeline,
                depth_only_pipeline,
                category2_pipeline,
                reflection_pipeline,
                soft_reflection_pipeline,
                reflection_format,
                uniform_bind_group_layout,
                texture_bind_group_layout,
                model_bind_group_layout,
                default_texture_bind_group,
                surface_sampler,
            },
        )
    }

    pub fn sharing_pipelines(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> Self {
        Self::with_pipelines(
            device,
            ModelPipelines {
                pipeline: self.pipeline.clone(),
                translucent_pipeline: self.translucent_pipeline.clone(),
                keyed_pipeline: self.keyed_pipeline.clone(),
                depth_only_pipeline: self.depth_only_pipeline.clone(),
                category2_pipeline: self.category2_pipeline.clone(),
                reflection_pipeline: self.reflection_pipeline.clone(),
                soft_reflection_pipeline: self.soft_reflection_pipeline.clone(),
                reflection_format: self.reflection_format,
                uniform_bind_group_layout: self.uniform_bind_group_layout.clone(),
                texture_bind_group_layout: self.texture_bind_group_layout.clone(),
                model_bind_group_layout: self.model_bind_group_layout.clone(),
                default_texture_bind_group: self.default_texture_bind_group.clone(),
                surface_sampler: self.surface_sampler.clone(),
            },
        )
    }

    fn with_pipelines(device: &wgpu::Device, pipelines: ModelPipelines) -> Self {
        let ModelPipelines {
            pipeline,
            translucent_pipeline,
            keyed_pipeline,
            depth_only_pipeline,
            category2_pipeline,
            reflection_pipeline,
            soft_reflection_pipeline,
            reflection_format,
            uniform_bind_group_layout,
            texture_bind_group_layout,
            model_bind_group_layout,
            default_texture_bind_group,
            surface_sampler,
        } = pipelines;

        let default_uniforms = ModelUniforms::default();
        let default_model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Default Model Uniform Buffer"),
            contents: bytemuck::bytes_of(&default_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let default_model_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Default Model Uniform Bind Group"),
            layout: &model_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: default_model_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            translucent_pipeline,
            keyed_pipeline,
            depth_only_pipeline,
            category2_pipeline,
            reflection_pipeline,
            soft_reflection_pipeline,
            reflection_format,
            uniform_bind_group_layout,
            texture_bind_group_layout,
            model_bind_group_layout,
            default_texture_bind_group,
            surface_sampler,
            default_model_bind_group,
            default_model_buffer,
        }
    }

    pub fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        img: &image::RgbaImage,
        label: Option<&str>,
    ) -> wgpu::BindGroup {
        let non_empty = img.width() > 0 && img.height() > 0;
        let chain = non_empty.then(|| crate::billboard::sprite_mip_chain(img));
        let width = img.width().max(1);
        let height = img.height().max(1);
        let mip_level_count = chain.as_ref().map_or(1, |c| c.len() as u32);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: label.or(Some("Model Surface Texture")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        if let Some(chain) = chain {
            for (level, data) in chain.iter().enumerate() {
                let (w, h) = (data.width(), data.height());
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data.as_raw(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w * 4),
                        rows_per_image: Some(h),
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Model Surface Texture Bind Group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.surface_sampler),
                },
            ],
        })
    }

    pub fn create_model_bind_group(
        &self,
        device: &wgpu::Device,
        model_matrix: &[[f32; 4]; 4],
        tint: &[f32; 4],
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let uniforms = ModelUniforms {
            model_matrix: *model_matrix,
            tint: *tint,
            ..Default::default()
        };

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Model Instance Uniform Buffer"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Model Instance Bind Group"),
            layout: &self.model_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        (buffer, bind_group)
    }

    pub fn update_model_uniforms(
        &self,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        model_matrix: &[[f32; 4]; 4],
        tint: &[f32; 4],
    ) {
        let uniforms = ModelUniforms {
            model_matrix: *model_matrix,
            tint: *tint,
            ..Default::default()
        };
        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn render_translucent<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        model_bind_group: Option<&'a wgpu::BindGroup>,
        mesh: &'a ModelMesh,
    ) {
        if !mesh.has_translucent() {
            return;
        }
        render_pass.set_pipeline(&self.translucent_pipeline);
        render_pass.set_bind_group(0, scene_bind_group, &[]);
        render_pass.set_bind_group(
            2,
            model_bind_group.unwrap_or(&self.default_model_bind_group),
            &[],
        );
        let fallback_tex_bg = texture_bind_group
            .or(mesh.texture_bind_group.as_ref())
            .unwrap_or(&self.default_texture_bind_group);
        render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for sub in mesh
            .submeshes
            .iter()
            .filter(|s| s.translucent && s.index_count > 0)
        {
            let sub_tex_bg = sub.texture_bind_group.as_ref().unwrap_or(fallback_tex_bg);
            render_pass.set_bind_group(1, sub_tex_bg, &[]);
            render_pass.draw_indexed(
                sub.index_offset..(sub.index_offset + sub.index_count),
                0,
                0..1,
            );
        }
    }

    pub fn render_translucent_sorted<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        batch: &'a TranslucentBatch,
        instances: &[(&'a wgpu::BindGroup, &'a ModelMesh)],
    ) {
        let count = batch.keys.len() as u32;
        self.render_translucent_triangles(
            render_pass,
            scene_bind_group,
            batch,
            instances,
            0..count,
        );
    }

    pub fn render_translucent_triangles<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        batch: &'a TranslucentBatch,
        instances: &[(&'a wgpu::BindGroup, &'a ModelMesh)],
        triangles: core::ops::Range<u32>,
    ) {
        self.render_batch_triangles(
            &self.translucent_pipeline,
            render_pass,
            scene_bind_group,
            batch,
            instances,
            triangles,
            &mut TrianglePassState::default(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_sprite_triangles<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        batch: &'a TranslucentBatch,
        instances: &[(&'a wgpu::BindGroup, &'a ModelMesh)],
        triangles: core::ops::Range<u32>,
        state: &mut TrianglePassState,
    ) {
        self.render_batch_triangles(
            &self.translucent_pipeline,
            render_pass,
            scene_bind_group,
            batch,
            instances,
            triangles,
            state,
        );
    }

    pub fn render_category2<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        batch: &'a TranslucentBatch,
        instances: &[(&'a wgpu::BindGroup, &'a ModelMesh)],
    ) {
        let count = batch.keys.len() as u32;
        self.render_batch_triangles(
            &self.category2_pipeline,
            render_pass,
            scene_bind_group,
            batch,
            instances,
            0..count,
            &mut TrianglePassState::default(),
        );
    }

    pub fn render_reflection_triangles<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        batch: &'a ReflectionBatch,
        instances: &[(&'a wgpu::BindGroup, &'a ModelMesh)],
        triangles: core::ops::Range<u32>,
        soft_edge: Option<&'a wgpu::BindGroup>,
    ) {
        let (lo, hi) = (triangles.start * 3, triangles.end * 3);
        if batch.runs.is_empty() || lo >= hi {
            return;
        }
        match soft_edge {
            Some(edge) => {
                render_pass.set_pipeline(&self.soft_reflection_pipeline);
                render_pass.set_bind_group(3, edge, &[]);
            }
            None => render_pass.set_pipeline(&self.reflection_pipeline),
        }
        render_pass.set_bind_group(0, scene_bind_group, &[]);
        render_pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));
        let mut bound_instance = None;
        for run in &batch.runs {
            let vertices = run.vertices.start.max(lo)..run.vertices.end.min(hi);
            if vertices.is_empty() {
                continue;
            }
            let Some(&(bind_group, mesh)) = instances.get(run.instance) else {
                continue;
            };
            if bound_instance != Some(run.instance) {
                render_pass.set_bind_group(2, bind_group, &[]);
                bound_instance = Some(run.instance);
            }
            let tex_bg = mesh
                .submeshes
                .get(run.submesh)
                .and_then(|s| s.texture_bind_group.as_ref())
                .or(mesh.texture_bind_group.as_ref())
                .unwrap_or(&self.default_texture_bind_group);
            render_pass.set_bind_group(1, tex_bg, &[]);
            render_pass.draw(vertices, 0..1);
        }
    }

    pub fn create_shaded_bind_group(
        &self,
        device: &wgpu::Device,
        model_matrix: &[[f32; 4]; 4],
        shade: [u8; 8],
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        self.create_faded_bind_group(device, model_matrix, shade, 1.0)
    }

    pub fn create_faded_bind_group(
        &self,
        device: &wgpu::Device,
        model_matrix: &[[f32; 4]; 4],
        shade: [u8; 8],
        opacity: f32,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let (kpqr, xyz) = crate::media::tint_factors(shade);
        let uniforms = ModelUniforms {
            model_matrix: *model_matrix,
            tint: [1.0, 1.0, 1.0, opacity],
            shade_kpqr: kpqr,
            shade_xyz: xyz,
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Model Shaded Uniform Buffer"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Model Shaded Bind Group"),
            layout: &self.model_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        (buffer, bind_group)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_batch_triangles<'a>(
        &'a self,
        pipeline: &'a wgpu::RenderPipeline,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        batch: &'a TranslucentBatch,
        instances: &[(&'a wgpu::BindGroup, &'a ModelMesh)],
        triangles: core::ops::Range<u32>,
        state: &mut TrianglePassState,
    ) {
        let (lo, hi) = (triangles.start * 3, triangles.end * 3);
        if batch.runs.is_empty() || lo >= hi {
            return;
        }
        let keyed =
            core::ptr::eq(pipeline, &self.translucent_pipeline).then_some(&self.keyed_pipeline);
        let mut bound_pipeline: Option<&wgpu::RenderPipeline> = None;
        if !state.scene_bound {
            render_pass.set_bind_group(0, scene_bind_group, &[]);
            state.scene_bound = true;
        }
        if !state.index_bound {
            render_pass.set_index_buffer(batch.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            state.index_bound = true;
        }
        let mut bound_vertices: Option<&wgpu::Buffer> = None;
        let mut bound_texture: Option<&wgpu::BindGroup> = None;
        let first = batch.runs.partition_point(|run| run.indices.end <= lo);
        for run in batch.runs[first..]
            .iter()
            .take_while(|run| run.indices.start < hi)
        {
            let indices = run.indices.start.max(lo)..run.indices.end.min(hi);
            if indices.is_empty() {
                continue;
            }
            let Some(&(model_bind_group, mesh)) = instances.get(run.instance) else {
                continue;
            };
            let Some(sub) = mesh.submeshes.get(run.submesh) else {
                continue;
            };
            let (want, keys) = match (keyed, &mesh.depth_keys) {
                (Some(keyed), Some(keys)) => (keyed, Some(keys)),
                _ => (pipeline, None),
            };
            if !bound_pipeline.is_some_and(|p| core::ptr::eq(p, want)) {
                render_pass.set_pipeline(want);
                bound_pipeline = Some(want);
            }
            if state.model != Some(run.instance) {
                render_pass.set_bind_group(2, model_bind_group, &[]);
                state.model = Some(run.instance);
            }
            if !bound_vertices.is_some_and(|b| core::ptr::eq(b, &mesh.vertex_buffer)) {
                render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                if let Some(keys) = keys {
                    render_pass.set_vertex_buffer(1, keys.slice(..));
                }
                bound_vertices = Some(&mesh.vertex_buffer);
            }
            let tex_bg = sub
                .texture_bind_group
                .as_ref()
                .or(mesh.texture_bind_group.as_ref())
                .unwrap_or(&self.default_texture_bind_group);
            if !bound_texture.is_some_and(|b| core::ptr::eq(b, tex_bg)) {
                render_pass.set_bind_group(1, tex_bg, &[]);
                bound_texture = Some(tex_bg);
            }
            render_pass.draw_indexed(indices, 0, 0..1);
        }
    }

    pub fn render<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        model_bind_group: Option<&'a wgpu::BindGroup>,
        mesh: &'a ModelMesh,
    ) {
        self.render_opaque_with(
            &self.pipeline,
            render_pass,
            scene_bind_group,
            texture_bind_group,
            model_bind_group,
            mesh,
        );
    }

    pub fn render_depth_only<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        model_bind_group: Option<&'a wgpu::BindGroup>,
        mesh: &'a ModelMesh,
    ) {
        self.render_opaque_with(
            &self.depth_only_pipeline,
            render_pass,
            scene_bind_group,
            texture_bind_group,
            model_bind_group,
            mesh,
        );
    }

    fn render_opaque_with<'a>(
        &'a self,
        pipeline: &'a wgpu::RenderPipeline,
        render_pass: &mut wgpu::RenderPass<'a>,
        scene_bind_group: &'a wgpu::BindGroup,
        texture_bind_group: Option<&'a wgpu::BindGroup>,
        model_bind_group: Option<&'a wgpu::BindGroup>,
        mesh: &'a ModelMesh,
    ) {
        if mesh.num_indices == 0 {
            return;
        }

        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, scene_bind_group, &[]);
        let model_bg = model_bind_group.unwrap_or(&self.default_model_bind_group);
        render_pass.set_bind_group(2, model_bg, &[]);

        let fallback_tex_bg = texture_bind_group
            .or(mesh.texture_bind_group.as_ref())
            .unwrap_or(&self.default_texture_bind_group);

        if mesh.submeshes.is_empty() {
            render_pass.set_bind_group(1, fallback_tex_bg, &[]);
            mesh.draw(render_pass);
        } else {
            render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for sub in &mesh.submeshes {
                if sub.index_count == 0 || sub.translucent || sub.category2 {
                    continue;
                }
                let sub_tex_bg = sub.texture_bind_group.as_ref().unwrap_or(fallback_tex_bg);
                render_pass.set_bind_group(1, sub_tex_bg, &[]);
                render_pass.draw_indexed(
                    sub.index_offset..(sub.index_offset + sub.index_count),
                    0,
                    0..1,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billboard::AtlasFrame;
    use mandrake_common::FourCC;
    use mandrake_tag::model::{GeometryMaterial, GeometrySurface, GeometryVertex, SurfaceVertex};

    #[test]
    fn test_model_vertex_layout_stride_and_offsets() {
        assert_eq!(core::mem::size_of::<ModelVertex>(), 36);
        let desc = ModelVertex::desc();
        assert_eq!(desc.array_stride, 36);
        assert_eq!(desc.attributes.len(), 4);
        assert_eq!(desc.attributes[0].offset, 0);
        assert_eq!(desc.attributes[1].offset, 12);
        assert_eq!(desc.attributes[2].offset, 24);
        assert_eq!(desc.attributes[3].offset, 32);
        assert_eq!(ModelVertex::new([0.0; 3], [0.0; 3], [0.0; 2]).alpha, 1.0);
    }

    #[test]
    fn test_build_geometry_buffers_translucent_surfaces() {
        let sv = |u, v, i| SurfaceVertex {
            u,
            v,
            vertex_index: i,
            flags: 0,
        };
        let surface = |transparency| GeometrySurface {
            flags: 0,
            normal: (0, 0, 512),
            distance: 0,
            transparency,
            material_index: 0,
            vertices: [sv(0, 0, 0), sv(65535, 0, 1), sv(0, 65535, 2)],
            draw_after_list: -1,
        };
        let geom = GeometryTag {
            flags: 0,
            texture_tag: FourCC::new(*b"wite"),
            origin: (0, 0, 0),
            material_count: 1,
            vertex_count: 3,
            surface_count: 3,
            materials: vec![GeometryMaterial {
                name: "blade".into(),
                texture_index: 0,
                flags: 0,
            }],
            vertices: vec![
                GeometryVertex { x: 0, y: 0, z: 0 },
                GeometryVertex { x: 512, y: 0, z: 0 },
                GeometryVertex { x: 0, y: 512, z: 0 },
            ],
            surfaces: vec![surface(0x6665), surface(0), surface(0x6665)],
            draw_after_lists: Vec::new(),
            bounds: [[0; 3]; 2],
        };
        let (verts, _indices, submeshes) =
            ModelMesh::build_geometry_buffers_with_frames(&geom, None);
        assert_eq!(submeshes.len(), 2);
        assert!(!submeshes[0].translucent);
        assert_eq!(submeshes[0].index_count, 3);
        assert!(submeshes[1].translucent);
        assert_eq!(submeshes[1].index_count, 6);
        assert_eq!(verts[0].alpha, 1.0);
        for v in &verts[3..] {
            assert!((v.alpha - 153.0 / 255.0).abs() < 1e-6);
        }

        let (verts, indices, submeshes) =
            ModelMesh::build_geometry_buffers_with_frames(&geom, None);
        let tris = TranslucentTriangle::collect(&verts, &indices, &submeshes);
        assert_eq!(tris.len(), 2);
        assert!(tris.iter().all(|t| t.submesh == 1));
        assert_eq!(tris[0].indices, [3, 4, 5]);
        assert_eq!(tris[1].indices, [6, 7, 8]);
        assert_eq!(
            tris[0].positions,
            [verts[3].position, verts[4].position, verts[5].position]
        );
    }

    fn tri_at(submesh: u32, base: u32, d: f32) -> TranslucentTriangle {
        TranslucentTriangle {
            submesh,
            indices: [base, base + 1, base + 2],
            positions: [[0.0, 0.0, -d], [1.0, 0.0, -d], [0.0, 1.0, -d]],
        }
    }

    const IDENTITY: [[f32; 4]; 4] = Mat4::IDENTITY.cols;

    #[test]
    fn test_sort_translucent_triangles_interleaves_instances() {
        let a = [tri_at(0, 0, 10.0), tri_at(0, 3, 2.0)];
        let b = [tri_at(1, 0, 5.0)];
        let list =
            sort_translucent_triangles([(IDENTITY, &a[..]), (IDENTITY, &b[..])], &Mat4::IDENTITY);
        assert_eq!(list.indices, [0, 1, 2, 0, 1, 2, 3, 4, 5]);
        assert_eq!(list.keys, [10.0, 5.0, 2.0]);
        assert_eq!(
            list.runs,
            [
                TranslucentRun {
                    instance: 0,
                    submesh: 0,
                    indices: 0..3
                },
                TranslucentRun {
                    instance: 1,
                    submesh: 1,
                    indices: 3..6
                },
                TranslucentRun {
                    instance: 0,
                    submesh: 0,
                    indices: 6..9
                },
            ]
        );
    }

    #[test]
    fn test_sort_translucent_triangles_on_screen_gives_each_triangle_its_rectangle() {
        let near = [tri_at(0, 0, 2.0)];
        let behind = [tri_at(0, 0, -2.0)];
        let mut shifted = IDENTITY;
        shifted[3] = [1.0, 0.0, -2.0, 1.0];
        let vp = Mat4::perspective_rh_zo(1.0, 1.0, 0.1, 100.0);
        let list = sort_translucent_triangles_on_screen(
            [
                (IDENTITY, &near[..]),
                (shifted, &near[..]),
                (IDENTITY, &behind[..]),
            ],
            &Mat4::IDENTITY,
            &vp,
        );
        assert_eq!(list.keys, [4.0, 2.0, -2.0]);
        assert_eq!(list.view_proj, Some(vp));
        let ndc = |p: [f32; 3]| {
            let r = crate::billboard::ndc_rect(&vp, [p]).unwrap();
            [r[0], r[1]]
        };
        let rect = |pts: [[f32; 3]; 3]| {
            let p = pts.map(ndc);
            Some([
                p.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min),
                p.iter().map(|q| q[1]).fold(f32::INFINITY, f32::min),
                p.iter().map(|q| q[0]).fold(f32::NEG_INFINITY, f32::max),
                p.iter().map(|q| q[1]).fold(f32::NEG_INFINITY, f32::max),
            ])
        };
        let moved = near[0].positions.map(|[x, y, z]| [x + 1.0, y, z - 2.0]);
        assert_eq!(list.bounds.len(), 3);
        assert_eq!(list.bounds[2], None);
        for (got, want) in list
            .bounds
            .iter()
            .zip([rect(moved), rect(near[0].positions)])
        {
            let (got, want) = (got.unwrap(), want.unwrap());
            assert!(
                got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5),
                "{got:?} {want:?}"
            );
        }
        assert!(
            sort_translucent_triangles([(IDENTITY, &near[..])], &Mat4::IDENTITY)
                .bounds
                .is_empty()
        );
    }

    #[test]
    fn test_sort_translucent_triangles_farthest_vertex_and_transforms() {
        let slanted = TranslucentTriangle {
            submesh: 0,
            indices: [0, 1, 2],
            positions: [[0.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 1.0, -9.0]],
        };
        let flat = [tri_at(0, 3, 6.0)];
        let list = sort_translucent_triangles(
            [(IDENTITY, &[slanted][..]), (IDENTITY, &flat[..])],
            &Mat4::IDENTITY,
        );
        assert_eq!(list.runs[0].instance, 0);
        assert_eq!(list.runs[1].instance, 1);

        let tri = [tri_at(0, 0, 1.0)];
        let mut far = IDENTITY;
        far[3] = [0.0, 0.0, -20.0, 1.0];
        let list =
            sort_translucent_triangles([(IDENTITY, &tri[..]), (far, &tri[..])], &Mat4::IDENTITY);
        assert_eq!(list.runs.len(), 2);
        assert_eq!(list.runs[0].instance, 1);

        let view = Mat4::look_at_rh([0.0, 0.0, -50.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let list = sort_translucent_triangles([(IDENTITY, &tri[..]), (far, &tri[..])], &view);
        assert_eq!(list.runs[0].instance, 0);

        let list = sort_translucent_triangles(
            [
                (IDENTITY, &[][..]),
                (IDENTITY, &tri[..]),
                (IDENTITY, &tri[..]),
            ],
            &Mat4::IDENTITY,
        );
        let order: Vec<usize> = list.runs.iter().map(|r| r.instance).collect();
        assert_eq!(order, [1, 2]);
    }

    #[test]
    fn test_model_uniforms_size_and_alignment() {
        assert_eq!(core::mem::size_of::<ModelUniforms>(), 112);
        let def = ModelUniforms::default();
        let bytes = bytemuck::bytes_of(&def);
        assert_eq!(bytes.len(), 112);
    }

    #[test]
    fn test_model_wgsl_syntax_validation() {
        let src = ModelRenderer::MODEL_SHADER_SRC;
        let module = naga::front::wgsl::parse_str(src);
        assert!(
            module.is_ok(),
            "Model WGSL syntax validation failed: {:?}",
            module.err()
        );
    }

    #[test]
    fn test_model_uniforms_constructors() {
        let u = ModelUniforms::from_translation_and_scale([10.0, 5.0, -20.0], [2.0, 3.0, 4.0]);
        assert_eq!(u.model_matrix[0][0], 2.0);
        assert_eq!(u.model_matrix[1][1], 3.0);
        assert_eq!(u.model_matrix[2][2], 4.0);
        assert_eq!(u.model_matrix[3][0], 10.0);
        assert_eq!(u.model_matrix[3][1], 5.0);
        assert_eq!(u.model_matrix[3][2], -20.0);
        assert_eq!(u.model_matrix[3][3], 1.0);
        assert_eq!(u.tint, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn model_slot0_tint_is_identity() {
        let u = ModelUniforms::default();
        assert_eq!(u.shade_kpqr, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(u.shade_xyz, [0.0; 4]);
        let slot0 = crate::media::ShadingSlot::default();
        assert_eq!(
            crate::media::build_tinting_shader(slot0, 0, [0; 3], slot0),
            MODEL_SLOT0_TINT
        );
        let shade = |c: [f32; 3], a: f32| -> [f32; 3] {
            let (k, x) = (u.shade_kpqr, u.shade_xyz);
            let l = 0.4 * c[0] + 0.6 * c[1] + 0.2 * c[2];
            [0, 1, 2].map(|i| c[i] * k[0] + a * k[i + 1] + l * x[i])
        };
        assert_eq!(shade([0.25, 0.5, 0.75], 0.6), [0.25, 0.5, 0.75]);
    }

    #[test]
    fn test_build_geometry_buffers_uv_remapping_and_submeshes() {
        let geom = GeometryTag {
            flags: 0,
            texture_tag: FourCC::new(*b"wite"),
            origin: (0, 0, 0),
            material_count: 2,
            vertex_count: 3,
            surface_count: 2,
            materials: vec![
                GeometryMaterial {
                    name: "walls".into(),
                    texture_index: 0,
                    flags: 0,
                },
                GeometryMaterial {
                    name: "sails".into(),
                    texture_index: 1,
                    flags: 1,
                },
            ],
            vertices: vec![
                GeometryVertex { x: 0, y: 0, z: 0 },
                GeometryVertex { x: 512, y: 0, z: 0 },
                GeometryVertex { x: 0, y: 512, z: 0 },
            ],
            surfaces: vec![
                GeometrySurface {
                    flags: 0,
                    normal: (0, 0, 512),
                    distance: 0,
                    transparency: 0,
                    material_index: 1,
                    vertices: [
                        SurfaceVertex {
                            u: 0,
                            v: 0,
                            vertex_index: 0,
                            flags: 0,
                        },
                        SurfaceVertex {
                            u: 65535,
                            v: 0,
                            vertex_index: 1,
                            flags: 0,
                        },
                        SurfaceVertex {
                            u: 0,
                            v: 65535,
                            vertex_index: 2,
                            flags: 0,
                        },
                    ],
                    draw_after_list: -1,
                },
                GeometrySurface {
                    flags: 0,
                    normal: (0, 0, 512),
                    distance: 0,
                    transparency: 0,
                    material_index: 0,
                    vertices: [
                        SurfaceVertex {
                            u: 0,
                            v: 0,
                            vertex_index: 0,
                            flags: 0,
                        },
                        SurfaceVertex {
                            u: 65535,
                            v: 65535,
                            vertex_index: 1,
                            flags: 0,
                        },
                        SurfaceVertex {
                            u: 0,
                            v: 65535,
                            vertex_index: 2,
                            flags: 0,
                        },
                    ],
                    draw_after_list: -1,
                },
            ],
            draw_after_lists: Vec::new(),
            bounds: [[0; 3]; 2],
        };

        let frames = [
            AtlasFrame {
                index: 0,
                uv_min: [0.0, 0.0],
                uv_max: [0.5, 0.5],
                width: 64,
                height: 64,
                offset_x: 0,
                offset_y: 0,
            },
            AtlasFrame {
                index: 1,
                uv_min: [0.5, 0.5],
                uv_max: [1.0, 1.0],
                width: 64,
                height: 64,
                offset_x: 0,
                offset_y: 0,
            },
        ];

        let (verts, indices, submeshes) =
            ModelMesh::build_geometry_buffers_with_frames(&geom, Some((128, 128, &frames)));

        assert_eq!(verts.len(), 6);
        assert_eq!(indices.len(), 6);
        assert_eq!(submeshes.len(), 2);

        assert_eq!(submeshes[0].bitmap_index, Some(0));
        assert!(!submeshes[0].two_sided);
        assert_eq!(submeshes[0].index_offset, 0);
        assert_eq!(submeshes[0].index_count, 3);
        assert!((verts[1].uv[0] - 0.5).abs() < 1e-4);
        assert!((verts[1].uv[1] - 0.5).abs() < 1e-4);

        assert_eq!(submeshes[1].bitmap_index, Some(1));
        assert!(submeshes[1].two_sided);
        assert_eq!(submeshes[1].index_offset, 3);
        assert_eq!(submeshes[1].index_count, 3);
        assert!((verts[3].uv[0] - 0.5).abs() < 1e-4);
        assert!((verts[4].uv[0] - 1.0).abs() < 1e-4);

        assert_eq!(verts[1].position, [0.0, 0.0, 1.0]);
        assert_eq!(verts[0].normal, [0.0, 1.0, 0.0]);
    }
}
