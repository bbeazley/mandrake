use crate::media::MediaSurface;
use crate::vertex::{OccluderVertex, TerrainVertex};
use core::ops::Range;
use mandrake_tag::mesh::{
    LOD_CELL_DRAWN_SHIFT, LOD_EDGE_SUBDIVIDED_MASK, LOD_MEDIA_SHIFT, MeshLod,
};
use wgpu::util::DeviceExt;

#[path = "mesh_ripple.rs"]
pub mod ripple;
pub use ripple::{
    LodCell, RippleUpload, VertexSource, make_cell_region_high_lod, recalculate_lod_for_cell_region,
};

fn cell_render_height(cell: &mandrake_tag::mesh::Myth2Cell, mesh_flags: u32) -> f32 {
    f32::from(crate::media::mesh_vertex_height(
        cell.physical_height,
        cell.media_height,
        cell.flags,
        cell.model_index,
        mesh_flags,
    )) / 512.0
}

pub fn is_media_triangle(flags: u16, triangle: usize) -> bool {
    flags & if triangle == 0 { 0x0800 } else { 0x1000 } != 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LodTriangle {
    pub corners: [(u32, u32); 3],
    pub media: bool,
}

const LOD_FAN_SIDES: [[(u32, u32); 4]; 4] = [
    [(1, 1), (2, 0), (1, 0), (0, 0)],
    [(1, 1), (2, 2), (2, 1), (2, 0)],
    [(1, 1), (0, 2), (1, 2), (2, 2)],
    [(1, 1), (0, 0), (0, 1), (0, 2)],
];

pub fn lod_block_triangles(word: u16, col: u32, row: u32) -> Vec<LodTriangle> {
    let bit = |b: u32| u32::from(word) >> b & 1 != 0;
    let tri = |corners: [(u32, u32); 3], media_bit: u32| LodTriangle {
        corners,
        media: bit(media_bit),
    };
    if word & LOD_EDGE_SUBDIVIDED_MASK == 0 {
        let v = if block_diagonal(col, row) == 0 {
            [(2, 2), (2, 0), (0, 0), (0, 2)]
        } else {
            [(2, 0), (0, 0), (0, 2), (2, 2)]
        };
        return vec![
            tri([v[0], v[1], v[2]], LOD_MEDIA_SHIFT),
            tri([v[2], v[3], v[0]], LOD_MEDIA_SHIFT),
        ];
    }
    let mut out = Vec::with_capacity(8);
    for (s, [c, a, mid, b]) in LOD_FAN_SIDES.iter().copied().enumerate() {
        let s = s as u32;
        if bit(s) {
            if bit(LOD_CELL_DRAWN_SHIFT + ((s + 1) & 3)) {
                out.push(tri([c, a, mid], LOD_MEDIA_SHIFT + 1 + 2 * s));
            }
            if bit(LOD_CELL_DRAWN_SHIFT + s) {
                out.push(tri([c, mid, b], LOD_MEDIA_SHIFT + 2 * s));
            }
        } else {
            out.push(tri([c, a, b], LOD_MEDIA_SHIFT + 2 * s));
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailHalf {
    pub corners: [(u32, u32); 3],
    pub media: bool,
    pub cell: (u32, u32),
    pub slot: usize,
    pub side: u32,
}

const DETAIL_HALF_CELLS: [[(u32, u32); 2]; 4] = [
    [(1, 0), (0, 0)],
    [(1, 1), (1, 0)],
    [(0, 1), (1, 1)],
    [(0, 0), (0, 1)],
];

pub fn detail_block_halves(word: u16) -> Vec<DetailHalf> {
    let bit = |b: u32| u32::from(word) >> b & 1 != 0;
    let mut out = Vec::with_capacity(8);
    for (s, [c, a, mid, b]) in LOD_FAN_SIDES.iter().copied().enumerate() {
        let s = s as u32;
        let [cell_a, cell_b] = DETAIL_HALF_CELLS[s as usize];
        if bit(LOD_CELL_DRAWN_SHIFT + ((s + 1) & 3)) {
            out.push(DetailHalf {
                corners: [c, a, mid],
                media: bit(LOD_MEDIA_SHIFT + 1 + 2 * s),
                cell: cell_a,
                slot: 0,
                side: s,
            });
        }
        if bit(LOD_CELL_DRAWN_SHIFT + s) {
            out.push(DetailHalf {
                corners: [c, mid, b],
                media: bit(LOD_MEDIA_SHIFT + 2 * s),
                cell: cell_b,
                slot: 1,
                side: s,
            });
        }
    }
    out
}

pub type AveragedVertex = ((u32, u32), [(u32, u32); 2]);

fn block_diagonal(col: u32, row: u32) -> u32 {
    ((row ^ col) >> 1) & 1
}

pub fn detail_block_averaged(word: u16, col: u32, row: u32) -> Vec<AveragedVertex> {
    let side = |s: usize| {
        let [_, a, mid, b] = LOD_FAN_SIDES[s];
        (mid, [a, b])
    };
    if word & LOD_EDGE_SUBDIVIDED_MASK == 0 {
        let centre = if block_diagonal(col, row) == 0 {
            [(2, 2), (0, 0)]
        } else {
            [(2, 0), (0, 2)]
        };
        let mut out: Vec<_> = (0..4).map(side).collect();
        out.push(((1, 1), centre));
        return out;
    }
    (0..4)
        .filter(|&s| u32::from(word) >> s & 1 == 0)
        .map(side)
        .collect()
}

pub fn detail_depth_group(word: u16, col: u32, row: u32, side: u32, slot: usize) -> u32 {
    if word & LOD_EDGE_SUBDIVIDED_MASK == 0 {
        let first = if block_diagonal(col, row) == 0 {
            side <= 1
        } else {
            side == 3 || side == 0
        };
        return u32::from(!first);
    }
    if u32::from(word) >> side & 1 == 0 {
        side
    } else {
        4 + 2 * side + slot as u32
    }
}

pub fn detail_block_word(
    mesh: &mandrake_tag::MeshTag,
    lod: Option<&MeshLod>,
    col: u32,
    row: u32,
) -> u16 {
    block_word(mesh, lod, col, row)
}

fn block_word(mesh: &mandrake_tag::MeshTag, lod: Option<&MeshLod>, col: u32, row: u32) -> u16 {
    lod.and_then(|l| l.word((col / 2) as usize, (row / 2) as usize))
        .unwrap_or_else(|| {
            let f = |dc: u32, dr: u32| {
                mesh.cell_at((col + dc) as usize, (row + dr) as usize)
                    .map_or(0, |c| c.flags)
            };
            0x00ff
                | mandrake_tag::mesh::lod_media_bits_from_cells(f(0, 0), f(1, 0), f(1, 1), f(0, 1))
        })
}

pub fn block_triangles(
    mesh: &mandrake_tag::MeshTag,
    lod: Option<&MeshLod>,
    col: u32,
    row: u32,
) -> impl Iterator<Item = ([(u32, u32); 3], bool)> {
    lod_block_triangles(block_word(mesh, lod, col, row), col, row)
        .into_iter()
        .map(move |t| (t.corners.map(|(dc, dr)| (col + dc, row + dr)), t.media))
}

pub fn landscape_triangles(
    mesh: &mandrake_tag::MeshTag,
    lod: Option<&MeshLod>,
) -> Vec<([u32; 3], bool)> {
    let (cols, rows) = (u32::from(mesh.cell_width), u32::from(mesh.cell_height));
    let vw = cols + 1;
    let mut out = Vec::with_capacity((cols * rows * 2) as usize);
    for by in 0..rows / 2 {
        for bx in 0..cols / 2 {
            let word = block_word(mesh, lod, 2 * bx, 2 * by);
            out.extend(wound_block_triangles(word, 2 * bx, 2 * by, vw));
        }
    }
    out
}

fn wound_block_triangles(
    word: u16,
    col: u32,
    row: u32,
    vw: u32,
) -> impl Iterator<Item = ([u32; 3], bool)> {
    lod_block_triangles(word, col, row)
        .into_iter()
        .map(move |t| {
            let p = t.corners.map(|(dc, dr)| (col + dc, row + dr));
            let turn = |a: (u32, u32), b: (u32, u32), c: (u32, u32)| {
                let (ax, ay) = (a.0 as i64, a.1 as i64);
                (b.0 as i64 - ax) * (c.1 as i64 - ay) - (b.1 as i64 - ay) * (c.0 as i64 - ax)
            };
            let p = if turn(p[0], p[1], p[2]) < 0 {
                [p[0], p[2], p[1]]
            } else {
                p
            };
            (p.map(|(c, r)| r * vw + c), t.media)
        })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OccluderRule {
    #[default]
    Myth2,
    TflBuckets,
}

pub struct TerrainMesh {
    pub occluder_rule: OccluderRule,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    pub num_media_indices: u32,
    pub occluder_buffer: wgpu::Buffer,
    pub num_occluder_vertices: u32,
    vertices: Vec<TerrainVertex>,
    occluders: Vec<OccluderVertex>,
    occluder_indices: Vec<u32>,
    occluder_depth: Vec<OccluderDepth>,
    media: Option<MediaSurface>,
    media_vertices: Vec<MediaVertex>,
    media_occluder_triangles: Vec<u32>,
    media_vertex_runs: Vec<Range<u32>>,
    media_occluder_runs: Vec<Range<u32>>,
    cell_size: f32,
    half_w: f32,
    half_d: f32,
    pub detail_buffer: Option<wgpu::Buffer>,
    media_mids: Vec<MediaMid>,
    ripple_cells: Option<ripple::RippleCells>,
    slots: Option<ripple::BlockSlots>,
    detail_vertices: Vec<crate::detail::DetailVertex>,
    vertex_sources: Option<Vec<VertexSource>>,
    media_time: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MediaMid {
    index: u32,
    corners: [Result<MediaVertex, [f32; 3]>; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaVertex {
    pub index: u32,
    pub col: i32,
    pub row: i32,
    pub media_height: i16,
}

impl TerrainMesh {
    pub fn from_raw(
        device: &wgpu::Device,
        label: &str,
        vertices: &[TerrainVertex],
        indices: &[u32],
    ) -> Self {
        Self::from_raw_with_occluders(device, label, vertices, indices, indices)
    }

    pub fn from_raw_with_occluders(
        device: &wgpu::Device,
        label: &str,
        vertices: &[TerrainVertex],
        indices: &[u32],
        occluder_indices: &[u32],
    ) -> Self {
        let depth = occluder_indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| OccluderDepth::corners(*t))
            .collect();
        Self::from_raw_with_occluder_depth(
            device,
            label,
            vertices,
            indices,
            occluder_indices,
            depth,
        )
    }

    fn from_raw_with_occluder_depth(
        device: &wgpu::Device,
        label: &str,
        vertices: &[TerrainVertex],
        indices: &[u32],
        occluder_indices: &[u32],
        occluder_depth: Vec<OccluderDepth>,
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

        let mut occluders =
            vec![<OccluderVertex as bytemuck::Zeroable>::zeroed(); occluder_indices.len() / 3 * 3];
        let all: Vec<u32> = (0..occluder_depth.len() as u32).collect();
        move_occluder_triangles(
            vertices,
            occluder_indices,
            &occluder_depth,
            &all,
            &mut occluders,
        );
        let occluder_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{} Occluder Buffer", label)),
            contents: bytemuck::cast_slice(if occluders.is_empty() {
                &EMPTY_OCCLUDER
            } else {
                &occluders[..]
            }),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            occluder_rule: OccluderRule::Myth2,
            vertex_buffer,
            index_buffer,
            num_indices: indices.len() as u32,
            num_media_indices: 0,
            occluder_buffer,
            num_occluder_vertices: occluders.len() as u32,
            vertices: vertices.to_vec(),
            occluders,
            occluder_indices: occluder_indices.to_vec(),
            occluder_depth,
            media: None,
            media_vertices: Vec::new(),
            media_occluder_triangles: Vec::new(),
            media_vertex_runs: Vec::new(),
            media_occluder_runs: Vec::new(),
            cell_size: 1.0,
            half_w: 0.0,
            half_d: 0.0,
            detail_buffer: None,
            media_mids: Vec::new(),
            ripple_cells: None,
            slots: None,
            detail_vertices: Vec::new(),
            vertex_sources: None,
            media_time: None,
        }
    }

    pub fn move_vertices(&mut self, queue: &wgpu::Queue, moved: &[(u32, [f32; 3])]) {
        let mut run: Option<(u32, u32)> = None;
        let write = |vertices: &[TerrainVertex], (a, b): (u32, u32)| {
            queue.write_buffer(
                &self.vertex_buffer,
                u64::from(a) * std::mem::size_of::<TerrainVertex>() as u64,
                bytemuck::cast_slice(&vertices[a as usize..b as usize]),
            );
        };
        for &(i, p) in moved {
            let Some(v) = self.vertices.get_mut(i as usize) else {
                continue;
            };
            v.position = p;
            run = match run {
                Some((a, b)) if b == i => Some((a, i + 1)),
                Some(r) => {
                    write(&self.vertices, r);
                    Some((i, i + 1))
                }
                None => Some((i, i + 1)),
            };
        }
        if let Some(r) = run {
            write(&self.vertices, r);
        }
    }

    pub fn move_ground_vertices(&mut self, queue: &wgpu::Queue, moved: &[(u32, [f32; 3])]) {
        if moved.is_empty() {
            return;
        }
        self.move_vertices(queue, moved);
        let mut changed: Vec<u32> = moved.iter().map(|&(i, _)| i).collect();
        changed.sort_unstable();
        let triangles: Vec<u32> = self
            .occluder_indices
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, t)| t.iter().any(|i| changed.binary_search(i).is_ok()))
            .map(|(k, _)| k as u32)
            .collect();
        move_occluder_triangles(
            &self.vertices,
            &self.occluder_indices,
            &self.occluder_depth,
            &triangles,
            &mut self.occluders,
        );
        let mut k = 0;
        while k < triangles.len() {
            let start = triangles[k];
            let mut end = start + 1;
            k += 1;
            while k < triangles.len() && triangles[k] == end {
                end += 1;
                k += 1;
            }
            queue.write_buffer(
                &self.occluder_buffer,
                u64::from(start) * 3 * std::mem::size_of::<OccluderVertex>() as u64,
                bytemuck::cast_slice(&self.occluders[start as usize * 3..end as usize * 3]),
            );
        }
    }

    pub fn media_vertices(&self) -> &[MediaVertex] {
        &self.media_vertices
    }

    pub fn is_plain_grid(&self) -> bool {
        self.vertex_sources.is_none() && self.detail_buffer.is_none()
    }

    pub(crate) fn drawn_indices(&self) -> Option<&[u32]> {
        self.slots
            .as_ref()
            .map(|s| &s.indices[..(self.num_indices as usize).min(s.indices.len())])
    }

    pub fn lowered_vertices(&self, mesh: &mandrake_tag::MeshTag) -> Vec<TerrainVertex> {
        let vw = usize::from(mesh.cell_width) + 1;
        let vh = usize::from(mesh.cell_height) + 1;
        let grid = |g: u32| -> Option<[f32; 3]> {
            let g = g as usize;
            if g >= vw * vh {
                return None;
            }
            let (c, r) = (g % vw, g / vw);
            let cell = mesh
                .cell_at(c.min(vw - 2), r.min(vh - 2))
                .copied()
                .unwrap_or_default();
            Some([
                self.half_w - c as f32 * self.cell_size,
                f32::from(cell.physical_height) / 512.0,
                r as f32 * self.cell_size - self.half_d,
            ])
        };
        self.vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let source = match &self.vertex_sources {
                    Some(sources) => sources.get(i).copied(),
                    None => Some(VertexSource::Grid(i as u32)),
                };
                let position = match source {
                    Some(VertexSource::Grid(g)) => grid(g),
                    Some(VertexSource::Mean(a, b)) => grid(a)
                        .zip(grid(b))
                        .map(|(a, b)| [0, 1, 2].map(|k| (a[k] + b[k]) * 0.5)),
                    None => None,
                };
                TerrainVertex {
                    position: position.unwrap_or(v.position),
                    ..*v
                }
            })
            .collect()
    }

    pub fn media_vertex_position(
        &self,
        v: &MediaVertex,
        local_time: i32,
        sine: &[i16; 1024],
    ) -> [f32; 3] {
        media_vertex_render_position(
            v,
            self.media,
            self.cell_size,
            self.half_w,
            self.half_d,
            local_time,
            sine,
        )
    }

    pub fn update_media(&mut self, queue: &wgpu::Queue, local_time: i32, sine: &[i16; 1024]) {
        if self.media_time == Some(local_time) {
            return;
        }
        self.media_time = Some(local_time);
        if (self.media_vertices.is_empty() && self.media_mids.is_empty())
            || !self.move_media(local_time, sine)
        {
            return;
        }
        for run in &self.media_vertex_runs {
            queue.write_buffer(
                &self.vertex_buffer,
                u64::from(run.start) * std::mem::size_of::<TerrainVertex>() as u64,
                bytemuck::cast_slice(&self.vertices[run.start as usize..run.end as usize]),
            );
        }
        for run in &self.media_occluder_runs {
            queue.write_buffer(
                &self.occluder_buffer,
                u64::from(run.start) * 3 * std::mem::size_of::<OccluderVertex>() as u64,
                bytemuck::cast_slice(&self.occluders[run.start as usize * 3..run.end as usize * 3]),
            );
        }
    }

    fn move_media(&mut self, local_time: i32, sine: &[i16; 1024]) -> bool {
        let (media, cell_size, half_w, half_d) =
            (self.media, self.cell_size, self.half_w, self.half_d);
        let position = |v: &MediaVertex| {
            media_vertex_render_position(v, media, cell_size, half_w, half_d, local_time, sine)
        };
        let moved = move_media_mids(&mut self.vertices, &self.media_mids, position);
        let moved_vertices = move_media_vertices(
            &mut self.vertices,
            &mut self.occluders,
            &self.occluder_indices,
            &self.occluder_depth,
            &self.media_vertices,
            &self.media_occluder_triangles,
            position,
        );
        if moved && !moved_vertices {
            move_occluder_triangles(
                &self.vertices,
                &self.occluder_indices,
                &self.occluder_depth,
                &self.media_occluder_triangles,
                &mut self.occluders,
            );
        }
        moved || moved_vertices
    }

    pub fn generate_grid<F>(
        cols: u32,
        rows: u32,
        cell_size: f32,
        height_fn: F,
    ) -> (Vec<TerrainVertex>, Vec<u32>)
    where
        F: Fn(u32, u32) -> f32,
    {
        let num_verts = ((cols + 1) * (rows + 1)) as usize;
        let mut vertices = Vec::with_capacity(num_verts);

        let half_w = (cols as f32 * cell_size) * 0.5;
        let half_d = (rows as f32 * cell_size) * 0.5;

        for r in 0..=rows {
            for c in 0..=cols {
                let x = c as f32 * cell_size - half_w;
                let z = r as f32 * cell_size - half_d;
                let y = height_fn(c, r);

                let u = c as f32 / cols as f32;
                let v = r as f32 / rows as f32;

                vertices.push(TerrainVertex {
                    position: [x, y, z],
                    uv: [u, v],
                    cell_flags: 0,
                });
            }
        }

        let num_indices = (cols * rows * 6) as usize;
        let mut indices = Vec::with_capacity(num_indices);
        let stride = cols + 1;

        for r in 0..rows {
            for c in 0..cols {
                let i0 = r * stride + c;
                let i1 = i0 + 1;
                let i2 = (r + 1) * stride + c;
                let i3 = i2 + 1;

                indices.push(i0);
                indices.push(i2);
                indices.push(i1);

                indices.push(i1);
                indices.push(i2);
                indices.push(i3);
            }
        }

        (vertices, indices)
    }

    pub fn create_grid<F>(
        device: &wgpu::Device,
        cols: u32,
        rows: u32,
        cell_size: f32,
        height_fn: F,
    ) -> Self
    where
        F: Fn(u32, u32) -> f32,
    {
        let (vertices, indices) = Self::generate_grid(cols, rows, cell_size, height_fn);
        Self::from_raw(device, "Terrain Grid", &vertices, &indices)
    }

    pub fn from_mesh_tag(
        device: &wgpu::Device,
        mesh: &mandrake_tag::MeshTag,
        cell_size: f32,
        media: Option<MediaSurface>,
        lod: Option<&MeshLod>,
    ) -> Self {
        let mesh_flags = media.map_or(0, |m| m.mesh_flags);
        let (vertices, indices, num_media_indices, occluder_indices) =
            landscape_geometry(mesh, cell_size, mesh_flags, lod);
        let mut padded = indices.clone();
        padded.resize(ripple::index_capacity(indices.len()), 0);
        let mut occluders_padded = occluder_indices.clone();
        occluders_padded.resize(
            occluder_indices.len() + 3 * ripple::SPARE_TRIANGLES as usize,
            0,
        );
        let mut terrain = Self::from_raw_with_occluders(
            device,
            "Terrain Grid from MeshTag",
            &vertices,
            &padded,
            &occluders_padded,
        );
        terrain.num_indices = indices.len() as u32;
        terrain.num_occluder_vertices = occluder_indices.len() as u32;
        let used = occluder_indices.len() / 3;
        terrain.occluder_depth[used..].fill(OccluderDepth::UNUSED);
        terrain.occluders[3 * used..].fill(bytemuck::Zeroable::zeroed());
        terrain.num_media_indices = num_media_indices;
        terrain.cell_size = cell_size;
        terrain.half_w = cols_half(mesh, cell_size).0;
        terrain.half_d = cols_half(mesh, cell_size).1;
        terrain.ripple_cells = Some(ripple::RippleCells::new(mesh, mesh_flags));
        if let Some(surface) = media {
            terrain.media = Some(surface);
            let plan = media_write_runs(
                mesh,
                surface.mesh_flags,
                vertices.len(),
                &indices,
                &occluder_indices,
                MEDIA_WRITE_GAP,
            );
            terrain.media_vertices = plan.vertices;
            terrain.media_occluder_triangles = plan.occluder_triangles;
            terrain.media_vertex_runs = plan.vertex_runs;
            terrain.media_occluder_runs = plan.occluder_runs;
        }
        terrain.install_slots(lod, indices, None);
        terrain
    }
}

impl TerrainMesh {
    pub fn from_mesh_tag_detailed(
        device: &wgpu::Device,
        mesh: &mandrake_tag::MeshTag,
        cell_size: f32,
        media: Option<MediaSurface>,
        lod: Option<&MeshLod>,
        detail: Option<&crate::detail::LandscapeDetail>,
    ) -> Self {
        let Some(detail) = detail else {
            return Self::from_mesh_tag(device, mesh, cell_size, media, lod);
        };
        let mesh_flags = media.map_or(0, |m| m.mesh_flags);
        let geometry = crate::detail::detail_landscape_geometry(
            mesh,
            cell_size,
            mesh_flags,
            lod,
            &detail.table,
        );
        let n = geometry.vertices.len() as u32;
        let indices: Vec<u32> = (0..n).collect();
        let num_media_indices = geometry.num_media_triangles * 3;
        let mut occluder_depth = detail_occluder_depth(mesh, &geometry);
        let spare = 3 * ripple::SPARE_TRIANGLES as usize;
        let mut vertices = geometry.vertices.clone();
        vertices.resize(vertices.len() + spare, bytemuck::Zeroable::zeroed());
        let mut padded = indices.clone();
        padded.resize(ripple::index_capacity(indices.len()), 0);
        let mut occluder_indices = indices[num_media_indices as usize..].to_vec();
        let num_occluders = occluder_indices.len() as u32;
        occluder_indices.resize(occluder_indices.len() + spare, 0);
        occluder_depth.resize(occluder_indices.len() / 3, OccluderDepth::UNUSED);
        let mut terrain = Self::from_raw_with_occluder_depth(
            device,
            "Terrain Grid from MeshTag (detail)",
            &vertices,
            &padded,
            &occluder_indices,
            occluder_depth,
        );
        terrain.num_indices = n;
        terrain.num_occluder_vertices = num_occluders;
        terrain.num_media_indices = num_media_indices;
        terrain.cell_size = cell_size;
        (terrain.half_w, terrain.half_d) = cols_half(mesh, cell_size);
        terrain.ripple_cells = Some(ripple::RippleCells::new(mesh, mesh_flags));
        let vw = u32::from(mesh.cell_width) + 1;
        let mut sources: Vec<VertexSource> = geometry
            .corners
            .iter()
            .map(|c| match c.averaged {
                Some([a, b]) => VertexSource::Mean(a.1 * vw + a.0, b.1 * vw + b.0),
                None => VertexSource::Grid(c.row * vw + c.col),
            })
            .collect();
        sources.resize(vertices.len(), VertexSource::Grid(0));
        terrain.vertex_sources = Some(sources);
        let mut attributes = detail.vertices(&geometry);
        attributes.resize(vertices.len(), bytemuck::Zeroable::zeroed());
        terrain.detail_buffer = Some(device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Terrain Detail Buffer"),
                contents: bytemuck::cast_slice(&attributes),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            },
        ));
        terrain.detail_vertices = attributes;
        if let Some(surface) = media {
            terrain.media = Some(surface);
            let plan = detail_media_plan(mesh, surface.mesh_flags, &geometry, &terrain);
            terrain.media_vertices = plan.0.vertices;
            terrain.media_occluder_triangles = plan.0.occluder_triangles;
            terrain.media_vertex_runs = plan.0.vertex_runs;
            terrain.media_occluder_runs = plan.0.occluder_runs;
            terrain.media_mids = plan.1;
        }
        let state = ripple::DetailState {
            table: detail.table.clone(),
            entry_layers: detail.entry_layers.clone(),
        };
        terrain.install_slots(lod, indices, Some(state));
        terrain
    }
}

fn detail_occluder_depth(
    mesh: &mandrake_tag::MeshTag,
    geometry: &crate::detail::DetailGeometry,
) -> Vec<OccluderDepth> {
    let vw = u32::from(mesh.cell_width) + 1;
    let mut at_grid: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for (i, c) in geometry.corners.iter().enumerate() {
        at_grid.entry(c.row * vw + c.col).or_insert(i as u32);
    }
    geometry.depth[geometry.num_media_triangles as usize..]
        .iter()
        .map(|points| {
            OccluderDepth(
                points
                    .iter()
                    .map(|p| match at_grid.get(&p.grid) {
                        Some(&i) if geometry.vertices[i as usize].position == p.position => {
                            DepthPoint::Vertex(i)
                        }
                        _ => DepthPoint::Fixed(p.position),
                    })
                    .collect(),
            )
        })
        .collect()
}

fn detail_media_plan(
    mesh: &mandrake_tag::MeshTag,
    mesh_flags: u32,
    geometry: &crate::detail::DetailGeometry,
    terrain: &TerrainMesh,
) -> (MediaWritePlan, Vec<MediaMid>) {
    let vw = u32::from(mesh.cell_width) + 1;
    let by_grid: std::collections::HashMap<u32, MediaVertex> = media_vertices(mesh, mesh_flags)
        .into_iter()
        .map(|v| (v.index, v))
        .collect();
    let media_at = |(c, r): (u32, u32)| by_grid.get(&(r * vw + c)).copied();
    let mut vertices = Vec::new();
    let mut mids = Vec::new();
    for (i, corner) in geometry.corners.iter().enumerate() {
        let index = i as u32;
        match corner.averaged {
            None => {
                if let Some(v) = media_at((corner.col, corner.row)) {
                    vertices.push(MediaVertex { index, ..v });
                }
            }
            Some(pair) => {
                let corners = pair.map(|(c, r)| {
                    media_at((c, r)).ok_or_else(|| {
                        let g = (r * vw + c) as usize;
                        grid_position(terrain, mesh, g)
                    })
                });
                if corners.iter().any(|c| c.is_ok()) {
                    mids.push(MediaMid { index, corners });
                }
            }
        }
    }
    let n = geometry.vertices.len();
    let mut is_media = vec![false; n];
    for i in vertices
        .iter()
        .map(|v| v.index)
        .chain(mids.iter().map(|m| m.index))
    {
        is_media[i as usize] = true;
    }
    let occluder_triangles: Vec<u32> = terrain
        .occluder_indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&terrain.occluder_depth)
        .enumerate()
        .filter(|(_, (tri, depth))| {
            tri.iter().any(|&i| is_media[i as usize])
                || depth
                    .0
                    .iter()
                    .any(|p| matches!(*p, DepthPoint::Vertex(i) if is_media[i as usize]))
        })
        .map(|(t, _)| t as u32)
        .collect();
    let mut moved: Vec<u32> = (0..n as u32).filter(|&i| is_media[i as usize]).collect();
    moved.sort_unstable();
    let plan = MediaWritePlan {
        vertices,
        vertex_runs: write_runs(&moved, MEDIA_WRITE_GAP),
        occluder_runs: write_runs(&occluder_triangles, MEDIA_WRITE_GAP / 3),
        occluder_triangles,
    };
    (plan, mids)
}

fn grid_position(terrain: &TerrainMesh, mesh: &mandrake_tag::MeshTag, g: usize) -> [f32; 3] {
    let vw = usize::from(mesh.cell_width) + 1;
    let (c, r) = ((g % vw) as u32, (g / vw) as u32);
    let cx = (c as usize).min(mesh.cell_width as usize - 1);
    let cy = (r as usize).min(mesh.cell_height as usize - 1);
    let cell = mesh.cell_at(cx, cy).copied().unwrap_or_default();
    let mesh_flags = terrain.media.map_or(0, |m| m.mesh_flags);
    [
        terrain.half_w - c as f32 * terrain.cell_size,
        cell_render_height(&cell, mesh_flags),
        r as f32 * terrain.cell_size - terrain.half_d,
    ]
}

fn media_vertex_render_position(
    v: &MediaVertex,
    media: Option<MediaSurface>,
    cell_size: f32,
    half_w: f32,
    half_d: f32,
    local_time: i32,
    sine: &[i16; 1024],
) -> [f32; 3] {
    let [dx, dy, dz] = media.map_or([0; 3], |m| m.wobble.offset(v.col, v.row, local_time, sine));
    let wx = (v.col * 512 + dx) as f32 / 512.0;
    let wy = (v.row * 512 + dy) as f32 / 512.0;
    [
        half_w - wx * cell_size,
        (i32::from(v.media_height) + dz) as f32 / 512.0,
        wy * cell_size - half_d,
    ]
}

fn move_media_mids(
    vertices: &mut [TerrainVertex],
    mids: &[MediaMid],
    position_of: impl Fn(&MediaVertex) -> [f32; 3],
) -> bool {
    let mut moved = false;
    for mid in mids.iter().filter(|m| m.index != u32::MAX) {
        let [a, b] = mid
            .corners
            .map(|c| c.map_or_else(|p| p, |v| position_of(&v)));
        let p = core::array::from_fn(|i| (a[i] + b[i]) * 0.5);
        let at = &mut vertices[mid.index as usize].position;
        if *at != p {
            *at = p;
            moved = true;
        }
    }
    moved
}

fn move_media_vertices(
    vertices: &mut [TerrainVertex],
    occluders: &mut [OccluderVertex],
    occluder_indices: &[u32],
    occluder_depth: &[OccluderDepth],
    media_vertices: &[MediaVertex],
    media_occluder_triangles: &[u32],
    mut position_of: impl FnMut(&MediaVertex) -> [f32; 3],
) -> bool {
    let mut moved = false;
    for v in media_vertices.iter().filter(|v| v.index != u32::MAX) {
        let pos = position_of(v);
        if vertices[v.index as usize].position != pos {
            vertices[v.index as usize].position = pos;
            moved = true;
        }
    }
    if moved {
        move_occluder_triangles(
            vertices,
            occluder_indices,
            occluder_depth,
            media_occluder_triangles,
            occluders,
        );
    }
    moved
}

struct MediaWritePlan {
    vertices: Vec<MediaVertex>,
    occluder_triangles: Vec<u32>,
    vertex_runs: Vec<Range<u32>>,
    occluder_runs: Vec<Range<u32>>,
}

fn media_write_runs(
    mesh: &mandrake_tag::MeshTag,
    mesh_flags: u32,
    num_vertices: usize,
    indices: &[u32],
    occluder_indices: &[u32],
    gap: u32,
) -> MediaWritePlan {
    let mut used = vec![false; num_vertices];
    for &i in indices {
        used[i as usize] = true;
    }
    let vertices: Vec<MediaVertex> = media_vertices(mesh, mesh_flags)
        .into_iter()
        .filter(|v| used[v.index as usize])
        .collect();
    let mut is_media = vec![false; num_vertices];
    for v in &vertices {
        is_media[v.index as usize] = true;
    }
    let occluder_triangles: Vec<u32> = occluder_indices
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, tri)| tri.iter().any(|&i| is_media[i as usize]))
        .map(|(t, _)| t as u32)
        .collect();
    let mut vert_indices: Vec<u32> = vertices.iter().map(|v| v.index).collect();
    vert_indices.sort_unstable();
    let vertex_runs = write_runs(&vert_indices, gap);
    let occluder_runs = write_runs(&occluder_triangles, gap / 3);
    MediaWritePlan {
        vertices,
        occluder_triangles,
        vertex_runs,
        occluder_runs,
    }
}

const EMPTY_OCCLUDER: [OccluderVertex; 1] = [OccluderVertex {
    position: [0.0; 3],
    depth: [[0.0; 3]; crate::vertex::OCCLUDER_DEPTH_POINTS],
}];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepthPoint {
    Vertex(u32),
    Fixed([f32; 3]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccluderDepth(pub Vec<DepthPoint>);

impl OccluderDepth {
    pub fn corners(tri: [u32; 3]) -> Self {
        Self(tri.map(DepthPoint::Vertex).to_vec())
    }

    pub const UNUSED: Self = Self(Vec::new());
}

fn move_occluder_triangles(
    vertices: &[TerrainVertex],
    indices: &[u32],
    depth: &[OccluderDepth],
    triangles: &[u32],
    occluders: &mut [OccluderVertex],
) {
    let position = |i: u32| vertices.get(i as usize).map_or([0.0; 3], |v| v.position);
    let mut points = Vec::with_capacity(crate::vertex::OCCLUDER_DEPTH_POINTS);
    for &t in triangles {
        let t = t as usize;
        if depth[t].0.is_empty() {
            occluders[t * 3..t * 3 + 3].fill(bytemuck::Zeroable::zeroed());
            continue;
        }
        let tri = &indices[t * 3..t * 3 + 3];
        points.clear();
        points.extend(depth[t].0.iter().map(|p| match *p {
            DepthPoint::Vertex(i) => position(i),
            DepthPoint::Fixed(p) => p,
        }));
        let corners = OccluderVertex::with_depth(
            [position(tri[0]), position(tri[1]), position(tri[2])],
            &points,
        );
        occluders[t * 3..t * 3 + 3].copy_from_slice(&corners);
    }
}

const MEDIA_WRITE_GAP: u32 = 6;

fn write_runs(items: &[u32], gap: u32) -> Vec<Range<u32>> {
    let mut runs: Vec<Range<u32>> = Vec::new();
    for &i in items {
        match runs.last_mut() {
            Some(run) if i - run.end <= gap => run.end = i + 1,
            _ => runs.push(i..i + 1),
        }
    }
    runs
}

fn cols_half(mesh: &mandrake_tag::MeshTag, cell_size: f32) -> (f32, f32) {
    (
        f32::from(mesh.cell_width) * cell_size * 0.5,
        f32::from(mesh.cell_height) * cell_size * 0.5,
    )
}

pub fn landscape_geometry(
    mesh: &mandrake_tag::MeshTag,
    cell_size: f32,
    mesh_flags: u32,
    lod: Option<&MeshLod>,
) -> (Vec<TerrainVertex>, Vec<u32>, u32, Vec<u32>) {
    let vertices = grid_vertices(mesh, cell_size, mesh_flags);
    let tris = landscape_triangles(mesh, lod);
    let mut indices = Vec::with_capacity(tris.len() * 3);
    let mut occluder_indices = Vec::new();
    for (tri, media) in &tris {
        if *media {
            indices.extend_from_slice(tri);
        }
    }
    let num_media_indices = indices.len() as u32;
    for (tri, media) in &tris {
        if !*media {
            indices.extend_from_slice(tri);
            occluder_indices.extend_from_slice(tri);
        }
    }
    (vertices, indices, num_media_indices, occluder_indices)
}

pub fn grid_vertices(
    mesh: &mandrake_tag::MeshTag,
    cell_size: f32,
    mesh_flags: u32,
) -> Vec<TerrainVertex> {
    let cols = mesh.cell_width as u32;
    let rows = mesh.cell_height as u32;
    let vw = cols + 1;
    let vh = rows + 1;
    let (half_w, half_d) = cols_half(mesh, cell_size);
    let height = |c: &mandrake_tag::mesh::Myth2Cell| cell_render_height(c, mesh_flags);

    let mut vertices = Vec::with_capacity((vw * vh) as usize);
    for r in 0..vh {
        for c in 0..vw {
            let x = half_w - (c as f32 * cell_size);
            let z = (r as f32 * cell_size) - half_d;

            let cx = (c as usize).min(mesh.cell_width as usize - 1);
            let cy = (r as usize).min(mesh.cell_height as usize - 1);
            let cell = mesh.cell_at(cx, cy).copied().unwrap_or_default();
            let y = height(&cell);

            let u = c as f32 / cols as f32;
            let v = r as f32 / rows as f32;

            vertices.push(TerrainVertex {
                position: [x, y, z],
                uv: [u, v],
                cell_flags: cell.flags as u32,
            });
        }
    }
    vertices
}

pub fn media_vertices(mesh: &mandrake_tag::MeshTag, mesh_flags: u32) -> Vec<MediaVertex> {
    let vw = u32::from(mesh.cell_width) + 1;
    let mut out = Vec::new();
    for row in 0..mesh.cell_height as usize {
        for col in 0..mesh.cell_width as usize {
            let Some(cell) = mesh.cell_at(col, row) else {
                continue;
            };
            if crate::media::is_media_vertex(cell.flags, cell.model_index, mesh_flags) {
                out.push(MediaVertex {
                    index: row as u32 * vw + col as u32,
                    col: col as i32,
                    row: row as i32,
                    media_height: cell.media_height,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn moved_occluder_triangles_equal_building_them_again() {
        let vertex = |x: f32| TerrainVertex {
            position: [x, 2.0 * x, 3.0 * x],
            uv: [0.0; 2],
            cell_flags: 0,
        };
        let mut vertices: Vec<TerrainVertex> = (0..5).map(|i| vertex(i as f32)).collect();
        let indices = [0, 1, 2, 2, 3, 4, 4, 0, 1];
        let positions = |v: &[TerrainVertex]| v.iter().map(|v| v.position).collect::<Vec<_>>();
        let mut occluders = OccluderVertex::from_triangles(&positions(&vertices), &indices);
        vertices[3] = vertex(9.0);
        let depth: Vec<OccluderDepth> = indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| OccluderDepth::corners(*t))
            .collect();
        move_occluder_triangles(&vertices, &indices, &depth, &[1], &mut occluders);
        let want = OccluderVertex::from_triangles(&positions(&vertices), &indices);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&occluders),
            bytemuck::cast_slice::<_, u8>(&want),
        );
    }

    #[test]
    fn write_runs_cover_every_item_and_join_near_neighbours() {
        let runs = |items: &[u32], gap| -> Vec<(u32, u32)> {
            write_runs(items, gap)
                .into_iter()
                .map(|r| (r.start, r.end))
                .collect()
        };
        assert!(runs(&[], 4).is_empty());
        assert_eq!(runs(&[7], 4), [(7, 8)]);
        assert_eq!(runs(&[1, 2, 7, 13], 4), [(1, 8), (13, 14)]);
        assert_eq!(runs(&[1, 2, 3], 0), [(1, 4)]);
        assert_eq!(runs(&[1, 3], 0), [(1, 2), (3, 4)]);
    }

    #[test]
    fn media_triangles_do_not_occlude() {
        assert!(!is_media_triangle(0x0000, 0));
        assert!(!is_media_triangle(0x0000, 1));
        assert!(is_media_triangle(0x0800, 0));
        assert!(!is_media_triangle(0x0800, 1));
        assert!(!is_media_triangle(0x1000, 0));
        assert!(is_media_triangle(0x1000, 1));
        assert!(is_media_triangle(0x3922, 0) && is_media_triangle(0x3922, 1));
    }

    use super::*;

    #[test]
    fn test_terrain_grid_generation() {
        let (vertices, indices) =
            TerrainMesh::generate_grid(4, 4, 2.0, |c, r| (c as f32) + (r as f32));

        assert_eq!(vertices.len(), 25);
        assert_eq!(indices.len(), 96);
    }

    #[test]
    fn test_cell_render_height_elevation_and_bank_calibration() {
        let water_cell = mandrake_tag::mesh::Myth2Cell {
            physical_height: -700,
            normal: 0,
            flags: 0x3d00,
            first_object_index: -1,
            media_height: -512,
            model_index: -1,
        };
        assert_eq!(cell_render_height(&water_cell, 0), -512.0 / 512.0);

        let bank_cell = mandrake_tag::mesh::Myth2Cell {
            physical_height: -408,
            normal: 0,
            flags: 0x2100,
            first_object_index: -1,
            media_height: -512,
            model_index: -1,
        };
        assert_eq!(cell_render_height(&bank_cell, 0), -512.0 / 512.0);

        let windmill_cell = mandrake_tag::mesh::Myth2Cell {
            physical_height: -384,
            normal: 0,
            flags: 0,
            first_object_index: -1,
            media_height: 0,
            model_index: 768,
        };
        assert_eq!(cell_render_height(&windmill_cell, 0), -384.0 / 512.0);
    }

    #[test]
    fn test_water_shoreline_triangle_calibration_synthetic() {
        let mesh = mandrake_tag::MeshTag {
            mesh_id: mandrake_common::FourCC::new(*b"test"),
            landscape_tag: mandrake_common::FourCC::new(*b"land"),
            media_tag: mandrake_common::FourCC::new(*b"wate"),
            submesh_w: 1,
            submesh_h: 1,
            cell_width: 2,
            cell_height: 2,
            total_cells: 4,
            mesh_offset: 0,
            mesh_size: 48,
            cells: vec![
                mandrake_tag::mesh::Myth2Cell {
                    physical_height: 50,
                    normal: 0,
                    flags: 0x1800,
                    first_object_index: -1,
                    media_height: 0,
                    model_index: -1,
                },
                mandrake_tag::mesh::Myth2Cell {
                    physical_height: 50,
                    normal: 0,
                    flags: 0x0800,
                    first_object_index: -1,
                    media_height: 0,
                    model_index: -1,
                },
                mandrake_tag::mesh::Myth2Cell {
                    physical_height: -50,
                    normal: 0,
                    flags: 0x1000,
                    first_object_index: -1,
                    media_height: 0,
                    model_index: -1,
                },
                mandrake_tag::mesh::Myth2Cell {
                    physical_height: 50,
                    normal: 0,
                    flags: 0x2100,
                    first_object_index: -1,
                    media_height: 0,
                    model_index: -1,
                },
            ],
            palette_count: 0,
            palette_offset: 0,
            marker_count: 0,
            marker_offset: 0,
            palettes: Vec::new(),
            markers: Vec::new(),
            string_list_tag: None,
            overhead_tag: None,
            pregame_story_tag: None,
            postgame_tag: None,
            next_mesh_id: None,
            landscape_shading: Default::default(),
            reverb: None,
        };

        assert!(media_vertices(&mesh, 0).is_empty());
        let mut wet = mesh.clone();
        wet.cells[3].flags = 0x1d00;
        wet.cells[3].media_height = -512;
        assert_eq!(
            media_vertices(&wet, 0),
            [MediaVertex {
                index: 4,
                col: 1,
                row: 1,
                media_height: -512
            }]
        );
        wet.cells[3].model_index = 2;
        assert_eq!(media_vertices(&wet, 0).len(), 1);
        assert!(media_vertices(&wet, crate::media::MESH_FLAG_MODEL_CELLS_NOT_MEDIA).is_empty());

        let c00 = mesh.cell_at(0, 0).unwrap();
        assert!((c00.flags & 0x0800) != 0);
        assert!((c00.flags & 0x1000) != 0);

        let c10 = mesh.cell_at(1, 0).unwrap();
        assert!((c10.flags & 0x0800) != 0);
        assert!((c10.flags & 0x1000) == 0);

        let c01 = mesh.cell_at(0, 1).unwrap();
        assert!((c01.flags & 0x0800) == 0);
        assert!((c01.flags & 0x1000) != 0);

        let c11 = mesh.cell_at(1, 1).unwrap();
        assert!((c11.flags & 0x1800) == 0);
        assert_eq!(cell_render_height(c11, 0), 0.0);
        assert_eq!(cell_render_height(c00, 0), 50.0 / 512.0);
    }

    #[test]
    fn detail_block_is_always_the_fan() {
        let word = 0x00f0 | 0b1011 << LOD_MEDIA_SHIFT;
        assert_eq!(lod_block_triangles(word, 0, 0).len(), 2);
        let halves = detail_block_halves(word);
        assert_eq!(halves.len(), 8);
        let media: Vec<bool> = halves.iter().map(|h| h.media).collect();
        assert_eq!(media, [true, true, true, false, false, false, false, false]);
        assert_eq!(
            halves.iter().map(|h| (h.cell, h.slot)).collect::<Vec<_>>(),
            [
                ((1, 0), 0),
                ((0, 0), 1),
                ((1, 1), 0),
                ((1, 0), 1),
                ((0, 1), 0),
                ((1, 1), 1),
                ((0, 0), 0),
                ((0, 1), 1),
            ]
        );
        assert_eq!(halves[0].corners, [(1, 1), (2, 0), (1, 0)]);
        assert_eq!(halves[1].corners, [(1, 1), (1, 0), (0, 0)]);
        let halves = detail_block_halves(0x00e0);
        assert_eq!(halves.len(), 6);
        assert!(!halves.iter().any(|h| h.cell == (0, 0)));
    }

    #[test]
    fn media_mid_follows_its_corners() {
        let mut vertices = vec![TerrainVertex::new([0.0; 3], [0.0; 2], 0); 2];
        let water = MediaVertex {
            index: 9,
            col: 4,
            row: 2,
            media_height: -512,
        };
        let mids = [MediaMid {
            index: 1,
            corners: [Ok(water), Err([2.0, 0.0, 4.0])],
        }];
        let at = |h: f32| move |_: &MediaVertex| [0.0, h, 0.0];
        assert!(move_media_mids(&mut vertices, &mids, at(-1.0)));
        assert_eq!(vertices[1].position, [1.0, -0.5, 2.0]);
        assert!(!move_media_mids(&mut vertices, &mids, at(-1.0)));
        assert!(move_media_mids(&mut vertices, &mids, at(1.0)));
        assert_eq!(vertices[1].position, [1.0, 0.5, 2.0]);
        assert_eq!(vertices[0].position, [0.0; 3]);
    }

    #[test]
    fn detail_block_averages_unsubdivided_midpoints() {
        for (col, row, centre) in [
            (0, 0, [(2, 2), (0, 0)]),
            (2, 0, [(2, 0), (0, 2)]),
            (0, 2, [(2, 0), (0, 2)]),
            (2, 2, [(2, 2), (0, 0)]),
        ] {
            let moved = detail_block_averaged(0x00f0, col, row);
            assert_eq!(moved.len(), 5);
            assert_eq!(moved[4], ((1, 1), centre));
            let plain = lod_block_triangles(0x00f0, col, row);
            assert!(plain[0].corners.contains(&centre[0]) && plain[0].corners.contains(&centre[1]));
        }
        assert_eq!(
            detail_block_averaged(0x00f9, 0, 0),
            [((2, 1), [(2, 2), (2, 0)]), ((1, 2), [(0, 2), (2, 2)])]
        );
        assert!(detail_block_averaged(0x00ff, 0, 0).is_empty());
    }

    #[test]
    fn detail_halves_share_the_plain_triangles_depth() {
        for (col, row) in [(0, 0), (2, 0)] {
            let plain = lod_block_triangles(0x00f0, col, row);
            for h in detail_block_halves(0x00f0) {
                let g = detail_depth_group(0x00f0, col, row, h.side, h.slot) as usize;
                assert!(g < 2);
                let tri = plain[g].corners;
                let inside = |(c, r): (u32, u32)| {
                    tri.contains(&(c, r))
                        || [(0, 1), (1, 2), (2, 0)].iter().any(|&(i, j)| {
                            let (a, b) = (tri[i], tri[j]);
                            (a.0 + b.0, a.1 + b.1) == (2 * c, 2 * r)
                        })
                };
                assert!(h.corners.iter().all(|&p| inside(p)), "{col},{row} {h:?}");
            }
        }
        let groups: Vec<u32> = detail_block_halves(0x00f9)
            .iter()
            .map(|h| detail_depth_group(0x00f9, 0, 0, h.side, h.slot))
            .collect();
        assert_eq!(groups, [4, 5, 1, 1, 2, 2, 10, 11]);
    }

    #[test]
    fn coarse_block_is_two_triangles() {
        let t = lod_block_triangles(0x00f0, 0, 0);
        assert_eq!(
            t.iter().map(|t| t.corners).collect::<Vec<_>>(),
            [[(2, 2), (2, 0), (0, 0)], [(0, 0), (0, 2), (2, 2)]]
        );
        assert!(t.iter().all(|t| !t.media));
        let t = lod_block_triangles(0x01f0, 2, 0);
        assert_eq!(
            t.iter().map(|t| t.corners).collect::<Vec<_>>(),
            [[(2, 0), (0, 0), (0, 2)], [(0, 2), (2, 2), (2, 0)]]
        );
        assert!(t.iter().all(|t| t.media));
        assert_eq!(lod_block_triangles(0x00f0, 2, 2)[0].corners[0], (2, 2));
        assert_eq!(lod_block_triangles(0x00f0, 0, 2)[0].corners[0], (2, 0));
    }

    #[test]
    fn fan_block_skips_unsubdivided_midpoint() {
        let word = 0x0001 | 0x0010 | 0x0020 | 0x0080 | (1 << 9) | (1 << 12);
        let t = lod_block_triangles(word, 0, 0);
        let corners: Vec<_> = t.iter().map(|t| (t.corners, t.media)).collect();
        assert_eq!(
            corners,
            [
                ([(1, 1), (2, 0), (1, 0)], true),
                ([(1, 1), (1, 0), (0, 0)], false),
                ([(1, 1), (2, 2), (2, 0)], false),
                ([(1, 1), (0, 2), (2, 2)], true),
                ([(1, 1), (0, 0), (0, 2)], false),
            ]
        );
        let t = lod_block_triangles(0x00bf, 0, 0);
        assert_eq!(t.len(), 6);
        assert!(!t.iter().any(|t| t.corners == [(1, 1), (2, 2), (2, 1)]));
        assert!(!t.iter().any(|t| t.corners == [(1, 1), (1, 2), (2, 2)]));
        assert_eq!(lod_block_triangles(0x00ff, 0, 0).len(), 8);
    }
}
