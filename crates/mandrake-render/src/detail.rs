use mandrake_tag::detail_map::{DETAIL_MAP_ENTRY_COUNT, DetailMap, DetailMapEntry};

pub const DETAIL_WORD_MIXED: u32 = 0x4000_0000;

pub fn layer_word(a: u8, b: u8, c: u8) -> u32 {
    let mut layers = [a, b, c];
    layers.sort_unstable();
    let mut word = 0;
    let mut n = 0;
    for (i, &t) in layers.iter().enumerate() {
        if t == 0 || (i > 0 && layers[i - 1] == t) {
            continue;
        }
        let at = 3 * n;
        word |= u32::from(t) << (9 + 7 * n)
            | u32::from(a == t) << at
            | u32::from(b == t) << (at + 1)
            | u32::from(c == t) << (at + 2);
        n += 1;
    }
    if n == 0 {
        return DETAIL_WORD_MIXED;
    }
    if !(a == b && b == c) {
        word |= DETAIL_WORD_MIXED;
    }
    word
}

pub fn forced_word(entry: u8) -> u32 {
    u32::from(entry) << 9 | 7
}

pub fn layer_count(word: u32) -> usize {
    (0..3).filter(|l| word >> (3 * l) & 7 != 0).count()
}

pub fn layer_entry(word: u32, l: usize) -> u8 {
    (word >> (9 + 7 * l) & 0x7f) as u8
}

pub fn layer_weight(word: u32, l: usize, v: usize) -> bool {
    word >> (3 * l + v) & 1 != 0
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailLayerTable {
    pub width: u32,
    pub height: u32,
    pub entries: Vec<DetailMapEntry>,
    pub cell_entries: Vec<u8>,
    pub triangle_entries: Vec<u8>,
    pub words: Vec<[u32; 2]>,
}

pub fn merge_entries(map: &DetailMap) -> (Vec<DetailMapEntry>, Vec<u8>, Vec<u8>) {
    let n = DETAIL_MAP_ENTRY_COUNT;
    let mut first = [0u8; 256];
    for (i, e) in map.entries.iter().enumerate().take(n).skip(1) {
        if e.raw[..4] == [0xff; 4] {
            continue;
        }
        let j = (0..=i)
            .find(|&j| map.entries[j].raw[..4] == e.raw[..4] && map.entries[j].raw[4] == e.raw[4])
            .unwrap_or(i);
        first[i] = j as u8;
    }
    let mut cells: Vec<u8> = map
        .cell_entries
        .iter()
        .map(|&b| first[b as usize])
        .collect();
    let mut tris: Vec<u8> = map
        .triangle_entries
        .iter()
        .map(|&b| first[b as usize])
        .collect();
    let mut used = [false; DETAIL_MAP_ENTRY_COUNT];
    for &b in cells.iter().chain(&tris) {
        used[b as usize] = true;
    }
    used[0] = true;
    let mut renumber = [0u8; 256];
    let mut entries: Vec<DetailMapEntry> = Vec::with_capacity(n);
    for i in 0..n {
        if used[i] {
            renumber[i] = entries.len() as u8;
            entries.push(map.entries[i].clone());
        }
    }
    let mut cleared = [0u8; 64];
    cleared[..4].copy_from_slice(&[0xff; 4]);
    entries.resize(n, DetailMapEntry::from_bytes(&cleared));
    for b in cells.iter_mut().chain(tris.iter_mut()) {
        *b = renumber[*b as usize];
    }
    (entries, cells, tris)
}

type BlockTriangle = ((u32, u32), usize, usize, [(u32, u32); 2]);

const BLOCK_TRIANGLES: [BlockTriangle; 8] = [
    ((1, 0), 0, 0, [(2, 0), (1, 0)]),
    ((0, 0), 1, 1, [(1, 0), (0, 0)]),
    ((1, 1), 0, 1, [(2, 2), (2, 1)]),
    ((1, 0), 1, 1, [(2, 1), (2, 0)]),
    ((0, 1), 0, 1, [(0, 2), (1, 2)]),
    ((1, 1), 1, 0, [(1, 2), (2, 2)]),
    ((0, 0), 0, 0, [(0, 0), (0, 1)]),
    ((0, 1), 1, 0, [(0, 1), (0, 2)]),
];

impl DetailLayerTable {
    pub fn new(map: &DetailMap) -> Self {
        let (entries, cell_entries, triangle_entries) = merge_entries(map);
        let (w, h) = (map.width, map.height);
        let cell = |x: u32, y: u32| -> u8 {
            if x < w && y < h {
                cell_entries[(y * w + x) as usize]
            } else {
                0
            }
        };
        let mut words = vec![[0u32; 2]; (w * h) as usize];
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                let centre = cell(x + 1, y + 1);
                for &((dx, dy), slot, forced, [b, c]) in &BLOCK_TRIANGLES {
                    let (cx, cy) = (x + dx, y + dy);
                    if cx >= w || cy >= h {
                        continue;
                    }
                    let at = (cy * w + cx) as usize;
                    let force = triangle_entries[2 * at + forced];
                    words[at][slot] = if force != 0 {
                        forced_word(force)
                    } else {
                        layer_word(centre, cell(x + b.0, y + b.1), cell(x + c.0, y + c.1))
                    };
                }
            }
        }
        Self {
            width: w,
            height: h,
            entries,
            cell_entries,
            triangle_entries,
            words,
        }
    }

    pub fn word(&self, col: u32, row: u32, slot: usize) -> u32 {
        if col < self.width && row < self.height {
            self.words[(row * self.width + col) as usize][slot]
        } else {
            DETAIL_WORD_MIXED
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.words
            .iter()
            .flat_map(|w| w.iter().flat_map(|v| v.to_le_bytes()))
            .collect()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DetailVertex {
    pub coords: [[f32; 2]; 3],
    pub layers: u32,
    pub weights: [u8; 4],
}

impl DetailVertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        3 => Float32x2,
        4 => Float32x2,
        5 => Float32x2,
        6 => Uint32,
        7 => Unorm8x4,
    ];

    pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }

    pub fn new(
        word: u32,
        v: usize,
        (col, row): (u32, u32),
        entry_layer: impl Fn(u8) -> Option<(u32, [f32; 2])>,
    ) -> Self {
        let mut out = Self::zeroed_vertex();
        for l in 0..layer_count(word) {
            let Some((layer, [sx, sy])) = entry_layer(layer_entry(word, l)) else {
                continue;
            };
            out.coords[l] = [-(col as f32) * sx, row as f32 * sy];
            out.layers |= (layer & 0xff) << (8 * l);
            out.weights[l] = if layer_weight(word, l, v) { 255 } else { 0 };
        }
        out
    }

    fn zeroed_vertex() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DetailGeometry {
    pub vertices: Vec<crate::vertex::TerrainVertex>,
    pub corners: Vec<DetailCorner>,
    pub words: Vec<u32>,
    pub num_media_triangles: u32,
    pub depth: Vec<Vec<DepthPoint>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthPoint {
    pub grid: u32,
    pub position: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailCorner {
    pub col: u32,
    pub row: u32,
    pub v: usize,
    pub averaged: Option<[(u32, u32); 2]>,
}

pub fn detail_landscape_geometry(
    mesh: &mandrake_tag::MeshTag,
    cell_size: f32,
    mesh_flags: u32,
    lod: Option<&mandrake_tag::mesh::MeshLod>,
    table: &DetailLayerTable,
) -> DetailGeometry {
    let (cols, rows) = (u32::from(mesh.cell_width), u32::from(mesh.cell_height));
    let vw = cols + 1;
    let mut grid = crate::mesh::grid_vertices(mesh, cell_size, mesh_flags);
    let mut averaged: Vec<Option<[(u32, u32); 2]>> = vec![None; grid.len()];
    let index = |(c, r): (u32, u32)| (r * vw + c) as usize;
    let mut media: Vec<HalfOut> = Vec::new();
    let mut dry: Vec<HalfOut> = Vec::new();
    for by in 0..rows / 2 {
        for bx in 0..cols / 2 {
            let (col, row) = (2 * bx, 2 * by);
            let word = crate::mesh::detail_block_word(mesh, lod, col, row);
            let halves = crate::mesh::detail_block_halves(word);
            let read: Vec<[DepthPoint; 3]> = halves
                .iter()
                .map(|h| {
                    h.corners.map(|(dc, dr)| {
                        let g = index((col + dc, row + dr));
                        DepthPoint {
                            grid: g as u32,
                            position: grid[g].position,
                        }
                    })
                })
                .collect();
            let groups: Vec<u32> = halves
                .iter()
                .map(|h| crate::mesh::detail_depth_group(word, col, row, h.side, h.slot))
                .collect();
            for (i, half) in halves.iter().enumerate() {
                let tri_word = table.word(col + half.cell.0, row + half.cell.1, half.slot);
                let mut depth: Vec<DepthPoint> = Vec::with_capacity(6);
                for (j, points) in read.iter().enumerate() {
                    if groups[j] != groups[i] {
                        continue;
                    }
                    for p in points {
                        if !depth.iter().any(|q| q.grid == p.grid) {
                            depth.push(*p);
                        }
                    }
                }
                let mut c: [DetailCorner; 3] = core::array::from_fn(|v| {
                    let (dc, dr) = half.corners[v];
                    DetailCorner {
                        col: col + dc,
                        row: row + dr,
                        v,
                        averaged: None,
                    }
                });
                let p = |k: usize| (c[k].col as i64, c[k].row as i64);
                let (a, b, d) = (p(0), p(1), p(2));
                if (b.0 - a.0) * (d.1 - a.1) - (b.1 - a.1) * (d.0 - a.0) < 0 {
                    c.swap(1, 2);
                }
                let out = HalfOut {
                    corners: c,
                    word: tri_word,
                    depth,
                };
                if half.media {
                    media.push(out);
                } else {
                    dry.push(out);
                }
            }
            for ((mc, mr), [a, b]) in crate::mesh::detail_block_averaged(word, col, row) {
                let at = |(c, r): (u32, u32)| (col + c, row + r);
                let (a, b) = (at(a), at(b));
                let (pa, pb) = (grid[index(a)].position, grid[index(b)].position);
                let m = index(at((mc, mr)));
                grid[m].position = core::array::from_fn(|i| (pa[i] + pb[i]) * 0.5);
                averaged[m] = Some([a, b]);
            }
        }
    }
    let mut out = DetailGeometry {
        num_media_triangles: media.len() as u32,
        ..Default::default()
    };
    for h in media.into_iter().chain(dry) {
        out.words.push(h.word);
        out.depth.push(h.depth);
        for k in h.corners {
            let g = index((k.col, k.row));
            out.vertices.push(grid[g]);
            out.corners.push(DetailCorner {
                averaged: averaged[g],
                ..k
            });
        }
    }
    out
}

struct HalfOut {
    corners: [DetailCorner; 3],
    word: u32,
    depth: Vec<DepthPoint>,
}

#[derive(Clone, Debug)]
pub struct LandscapeDetail {
    pub table: DetailLayerTable,
    pub textures: Vec<(mandrake_common::FourCC, Pix0r1Image)>,
    pub entry_layers: Vec<Option<(u32, [f32; 2])>>,
}

use mandrake_tag::pix0r1::Pix0r1Image;

impl LandscapeDetail {
    pub fn new(
        map: &DetailMap,
        mut texture: impl FnMut(mandrake_common::FourCC) -> Option<Pix0r1Image>,
    ) -> Option<Self> {
        let table = DetailLayerTable::new(map);
        let mut textures: Vec<(mandrake_common::FourCC, Pix0r1Image)> = Vec::new();
        let mut entry_layers = vec![None; table.entries.len()];
        let mut used = [false; DETAIL_MAP_ENTRY_COUNT];
        for w in table.words.iter().flatten() {
            for l in 0..layer_count(*w) {
                used[layer_entry(*w, l) as usize] = true;
            }
        }
        for (i, e) in table.entries.iter().enumerate().skip(1) {
            let Some(id) = e.texture else {
                continue;
            };
            let layer = match textures.iter().position(|(t, _)| *t == id) {
                Some(l) => l,
                None => match texture(id) {
                    Some(img) => {
                        textures.push((id, img));
                        textures.len() - 1
                    }
                    None if used[i] => return None,
                    None => continue,
                },
            };
            let img = &textures[layer].1;
            entry_layers[i] = Some((layer as u32, e.texture_scale(img.width, img.height)));
        }
        Some(Self {
            table,
            textures,
            entry_layers,
        })
    }

    pub fn vertices(&self, geometry: &DetailGeometry) -> Vec<DetailVertex> {
        let layer = |e: u8| self.entry_layers.get(e as usize).copied().flatten();
        geometry
            .corners
            .iter()
            .enumerate()
            .map(|(i, c)| DetailVertex::new(geometry.words[i / 3], c.v, (c.col, c.row), layer))
            .collect()
    }

    pub fn array_size(&self) -> (u32, u32) {
        self.textures
            .iter()
            .fold((1, 1), |(w, h), (_, t)| (w.max(t.width), h.max(t.height)))
    }

    pub fn mip_chain(&self, i: usize) -> Vec<(u32, u32, Vec<u8>)> {
        let (w, h) = self.array_size();
        let t = &self.textures[i].1;
        let mut rgba: Vec<u8> = t
            .rgb
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        if (t.width, t.height) != (w, h) {
            let img = image::RgbaImage::from_raw(t.width, t.height, rgba).expect("dtex size");
            rgba = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle)
                .into_raw();
        }
        let mut levels = vec![(w, h, rgba)];
        loop {
            let (lw, lh, prev) = levels.last().expect("level 0");
            if *lw == 1 && *lh == 1 {
                break;
            }
            let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
            let (lw, lh) = (*lw as usize, *lh as usize);
            let mut next = vec![0u8; (nw * nh * 4) as usize];
            for y in 0..nh as usize {
                for x in 0..nw as usize {
                    for ch in 0..4 {
                        let mut sum = 0u32;
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            let sx = (2 * x + dx).min(lw - 1);
                            let sy = (2 * y + dy).min(lh - 1);
                            sum += u32::from(prev[(sy * lw + sx) * 4 + ch]);
                        }
                        next[(y * nw as usize + x) * 4 + ch] = ((sum + 2) / 4) as u8;
                    }
                }
            }
            levels.push((nw, nh, next));
        }
        levels
    }
}

pub const DETAIL_TEXTURE_ANISOTROPY: u16 = 8;

pub struct DetailTextures {
    pub texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
}

impl DetailTextures {
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Detail Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
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
        })
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        detail: &LandscapeDetail,
    ) -> Self {
        let (w, h) = detail.array_size();
        let layers = detail.textures.len().max(1) as u32;
        let mips = 32 - w.max(h).leading_zeros();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Detail Texture Array"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: layers,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for i in 0..detail.textures.len() {
            for (level, (lw, lh, data)) in detail.mip_chain(i).into_iter().enumerate() {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: i as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * lw),
                        rows_per_image: Some(lh),
                    },
                    wgpu::Extent3d {
                        width: lw,
                        height: lh,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Detail Texture Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: DETAIL_TEXTURE_ANISOTROPY,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Detail Texture Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self {
            texture,
            bind_group,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_of_three_entries() {
        assert_eq!(layer_word(5, 5, 5), 5 << 9 | 7);
        assert_eq!(layer_word(0, 0, 0), DETAIL_WORD_MIXED);
        assert_eq!(
            layer_word(3, 2, 3),
            DETAIL_WORD_MIXED | 2 << 9 | 0b010 | 3 << 16 | 0b101 << 3
        );
        assert_eq!(layer_word(0, 4, 0), DETAIL_WORD_MIXED | 4 << 9 | 0b010);
        let w = layer_word(9, 1, 4);
        assert_eq!(layer_count(w), 3);
        assert_eq!([0, 1, 2].map(|l| layer_entry(w, l)), [1, 4, 9]);
        assert!(layer_weight(w, 0, 1) && layer_weight(w, 1, 2) && layer_weight(w, 2, 0));
        assert_eq!(layer_word(3, 3, 3), 0x0000_0607);
        assert_eq!(layer_count(0x4003_022a), 2);
    }
}
