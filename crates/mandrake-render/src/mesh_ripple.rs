use super::{
    DepthPoint, MEDIA_WRITE_GAP, MediaMid, MediaVertex, OccluderDepth, TerrainMesh,
    cell_render_height, media_vertex_render_position, move_occluder_triangles, write_runs,
};
use crate::detail::{DetailLayerTable, DetailVertex};
use crate::vertex::{OccluderVertex, TerrainVertex};
use core::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexSource {
    Grid(u32),
    Mean(u32, u32),
}

#[derive(Clone, Debug)]
pub(crate) struct RippleCells {
    pub cells: Vec<mandrake_tag::mesh::Myth2Cell>,
    pub width: usize,
    pub height: usize,
    pub mesh_flags: u32,
}

impl RippleCells {
    pub fn new(mesh: &mandrake_tag::MeshTag, mesh_flags: u32) -> Self {
        Self {
            cells: mesh.cells.clone(),
            width: usize::from(mesh.cell_width),
            height: usize::from(mesh.cell_height),
            mesh_flags,
        }
    }

    fn cell(&self, col: u32, row: u32) -> mandrake_tag::mesh::Myth2Cell {
        let cx = (col as usize).min(self.width - 1);
        let cy = (row as usize).min(self.height - 1);
        self.cells[cy * self.width + cx]
    }

    fn media_vertex(&self, index: u32, col: u32, row: u32) -> Option<MediaVertex> {
        if col as usize >= self.width || row as usize >= self.height {
            return None;
        }
        let cell = self.cells[row as usize * self.width + col as usize];
        crate::media::is_media_vertex(cell.flags, cell.model_index, self.mesh_flags).then_some(
            MediaVertex {
                index,
                col: col as i32,
                row: row as i32,
                media_height: cell.media_height,
            },
        )
    }

    fn word(&self, lod: Option<&mandrake_tag::mesh::MeshLod>, bx: u32, by: u32) -> u16 {
        lod.and_then(|l| l.word(bx as usize, by as usize))
            .unwrap_or_else(|| {
                let f = |dc: u32, dr: u32| {
                    let (c, r) = ((2 * bx + dc) as usize, (2 * by + dr) as usize);
                    if c < self.width && r < self.height {
                        self.cells[r * self.width + c].flags
                    } else {
                        0
                    }
                };
                0x00ff
                    | mandrake_tag::mesh::lod_media_bits_from_cells(
                        f(0, 0),
                        f(1, 0),
                        f(1, 1),
                        f(0, 1),
                    )
            })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RippleUpload {
    pub blocks: u32,
    pub vertices: u32,
    pub occluder_triangles: u32,
    pub indices: u32,
    pub writes: u32,
}

pub const CHUNK: u32 = 8;
const SPARE_CHUNKS: u32 = 1024;
const NONE: u32 = u32::MAX;
const MID: u32 = 1 << 31;

#[derive(Clone, Debug)]
pub(crate) struct DetailState {
    pub table: DetailLayerTable,
    pub entry_layers: Vec<Option<(u32, [f32; 2])>>,
}

#[derive(Debug)]
pub(crate) struct BlockSlots {
    bw: u32,
    bh: u32,
    words: Vec<u16>,
    detail: Option<DetailState>,
    pub(crate) indices: Vec<u32>,
    seg: Vec<u32>,
    home: Vec<u32>,
    spare_entries: u32,
    spare_occ: u32,
    entry_chunk: Vec<u32>,
    occ_chunk: Vec<u32>,
    free_entry_chunks: Vec<u32>,
    free_occ_chunks: Vec<u32>,
    occ_chunks_used: u32,
    uses: Vec<u16>,
    media_of: Vec<u32>,
    free_media: Vec<u32>,
    free_mids: Vec<u32>,
    media_run: Vec<bool>,
    media_occ: Vec<bool>,
}

struct BlockTri {
    corners: [BlockCorner; 3],
    word: u32,
    depth: Vec<(u32, [f32; 3])>,
}

#[derive(Clone, Copy)]
struct BlockCorner {
    grid: u32,
    col: u32,
    row: u32,
    v: usize,
    position: [f32; 3],
    averaged: Option<[(u32, u32); 2]>,
}

fn side_pair(s: usize) -> [(u32, u32); 2] {
    let [_, a, _, b] = super::LOD_FAN_SIDES[s];
    [a, b]
}

fn mean(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| (a[i] + b[i]) * 0.5)
}

impl BlockSlots {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bw: u32,
        bh: u32,
        words: Vec<u16>,
        counts: &[(u32, u32)],
        indices: Vec<u32>,
        grids: usize,
        detail: Option<DetailState>,
    ) -> Self {
        let nb = (bw * bh) as usize;
        let mut seg = Vec::with_capacity(2 * nb + 1);
        seg.push(0);
        for &(m, _) in counts {
            seg.push(seg.last().unwrap() + m);
        }
        for &(_, d) in counts {
            seg.push(seg.last().unwrap() + d);
        }
        let tris = *seg.last().unwrap();
        debug_assert_eq!(tris as usize * 3, indices.len());
        let mut uses = Vec::new();
        if detail.is_none() {
            uses = vec![0u16; grids];
            for &i in &indices {
                uses[i as usize] = uses[i as usize].saturating_add(1);
            }
        }
        let entries = if detail.is_some() {
            3 * (tris + SPARE_CHUNKS * CHUNK) as usize
        } else {
            grids
        };
        let occ_tris = (tris - seg[nb] + SPARE_CHUNKS * CHUNK) as usize;
        Self {
            bw,
            bh,
            words,
            detail,
            indices,
            home: seg.clone(),
            spare_entries: tris,
            spare_occ: tris - seg[nb],
            seg,
            entry_chunk: vec![NONE; nb],
            occ_chunk: vec![NONE; nb],
            free_entry_chunks: (0..SPARE_CHUNKS).rev().collect(),
            free_occ_chunks: (0..SPARE_CHUNKS).rev().collect(),
            occ_chunks_used: 0,
            uses,
            media_of: vec![NONE; entries],
            free_media: Vec::new(),
            free_mids: Vec::new(),
            media_run: vec![false; entries],
            media_occ: vec![false; occ_tris],
        }
    }

    fn nb(&self) -> usize {
        (self.bw * self.bh) as usize
    }

    fn segment(&self, k: usize) -> &[u32] {
        &self.indices[3 * self.seg[k] as usize..3 * self.seg[k + 1] as usize]
    }

    fn entry_pool(&self, b: usize) -> Vec<u32> {
        let nb = self.nb();
        let mut out: Vec<u32> = (self.home[b]..self.home[b + 1])
            .chain(self.home[nb + b]..self.home[nb + b + 1])
            .collect();
        if self.entry_chunk[b] != NONE {
            let s = self.spare_entries + self.entry_chunk[b] * CHUNK;
            out.extend(s..s + CHUNK);
        }
        out
    }

    fn occ_pool(&self, b: usize) -> Vec<u32> {
        let nb = self.nb();
        let base = self.home[nb];
        let mut out: Vec<u32> = (self.home[nb + b] - base..self.home[nb + b + 1] - base).collect();
        if self.occ_chunk[b] != NONE {
            let s = self.spare_occ + self.occ_chunk[b] * CHUNK;
            out.extend(s..s + CHUNK);
        }
        out
    }

    fn entry_home(&self, b: usize) -> u32 {
        let nb = self.nb();
        self.home[b + 1] - self.home[b] + self.home[nb + b + 1] - self.home[nb + b]
    }

    fn occ_home(&self, b: usize) -> u32 {
        let nb = self.nb();
        self.home[nb + b + 1] - self.home[nb + b]
    }

    fn fit_entries(&mut self, b: usize, n: u32) -> bool {
        let home = self.entry_home(b);
        if n <= home {
            if self.entry_chunk[b] != NONE {
                self.free_entry_chunks.push(self.entry_chunk[b]);
                self.entry_chunk[b] = NONE;
            }
            return true;
        }
        if self.entry_chunk[b] == NONE {
            match self.free_entry_chunks.pop() {
                Some(c) => self.entry_chunk[b] = c,
                None => return false,
            }
        }
        n <= home + CHUNK
    }

    fn fit_occluders(&mut self, b: usize, n: u32) -> bool {
        let home = self.occ_home(b);
        if n <= home {
            if self.occ_chunk[b] != NONE {
                self.free_occ_chunks.push(self.occ_chunk[b]);
                self.occ_chunk[b] = NONE;
            }
            return true;
        }
        if self.occ_chunk[b] == NONE {
            match self.free_occ_chunks.pop() {
                Some(c) => {
                    self.occ_chunk[b] = c;
                    self.occ_chunks_used = self.occ_chunks_used.max(c + 1);
                }
                None => return false,
            }
        }
        n <= home + CHUNK
    }
}

pub(crate) const SPARE_TRIANGLES: u32 = SPARE_CHUNKS * CHUNK;

pub(crate) fn block_counts(word: u16, col: u32, row: u32, detail: bool) -> (u32, u32) {
    let media: Vec<bool> = if detail {
        super::detail_block_halves(word)
            .iter()
            .map(|h| h.media)
            .collect()
    } else {
        super::lod_block_triangles(word, col, row)
            .iter()
            .map(|t| t.media)
            .collect()
    };
    let m = media.iter().filter(|&&m| m).count() as u32;
    (m, media.len() as u32 - m)
}

fn blocks_of_vertex(col: u32, row: u32, bw: u32, bh: u32, out: &mut Vec<u32>) {
    let range = |x: u32, n: u32| {
        let lo = x.saturating_sub(1) / 2;
        let hi = (x / 2).min(n.saturating_sub(1));
        lo..=hi
    };
    for by in range(row, bh) {
        for bx in range(col, bw) {
            out.push(by * bw + bx);
        }
    }
}

impl TerrainMesh {
    pub(super) fn install_slots(
        &mut self,
        lod: Option<&mandrake_tag::mesh::MeshLod>,
        indices: Vec<u32>,
        detail: Option<DetailState>,
    ) {
        let Some(cells) = self.ripple_cells.as_ref() else {
            return;
        };
        let (bw, bh) = (cells.width as u32 / 2, cells.height as u32 / 2);
        let mut words = Vec::with_capacity((bw * bh) as usize);
        let mut counts = Vec::with_capacity((bw * bh) as usize);
        for by in 0..bh {
            for bx in 0..bw {
                let w = cells.word(lod, bx, by);
                words.push(w);
                counts.push(block_counts(w, 2 * bx, 2 * by, detail.is_some()));
            }
        }
        let grids = (cells.width + 1) * (cells.height + 1);
        let mut slots = BlockSlots::new(bw, bh, words, &counts, indices, grids, detail);
        for (j, v) in self.media_vertices.iter().enumerate() {
            slots.media_of[v.index as usize] = j as u32;
        }
        for (j, m) in self.media_mids.iter().enumerate() {
            slots.media_of[m.index as usize] = MID | j as u32;
        }
        for run in &self.media_vertex_runs {
            slots.media_run[run.start as usize..run.end as usize].fill(true);
        }
        for &t in &self.media_occluder_triangles {
            slots.media_occ[t as usize] = true;
        }
        self.slots = Some(slots);
    }

    fn entry_source(&self, i: u32) -> VertexSource {
        self.vertex_sources
            .as_ref()
            .map_or(VertexSource::Grid(i), |s| s[i as usize])
    }

    fn grid_render_position(&self, cells: &RippleCells, g: u32) -> [f32; 3] {
        let vw = cells.width as u32 + 1;
        let (c, r) = (g % vw, g / vw);
        let cell = cells.cell(c, r);
        [
            self.half_w - (c as f32 * self.cell_size),
            cell_render_height(&cell, cells.mesh_flags),
            (r as f32 * self.cell_size) - self.half_d,
        ]
    }

    pub fn cell_heights(&self, cell: usize) -> Option<(i16, i16)> {
        let c = self.ripple_cells.as_ref()?.cells.get(cell)?;
        Some((c.physical_height, c.media_height))
    }

    pub fn cpu_buffers(&self) -> (&[TerrainVertex], &[OccluderVertex]) {
        (&self.vertices, &self.occluders)
    }

    pub fn cpu_indices(&self) -> (&[u32], &[DetailVertex]) {
        let indices = self.slots.as_ref().map_or(&[][..], |s| &s.indices[..]);
        (indices, &self.detail_vertices)
    }

    pub fn vertex_position(&self, i: u32) -> Option<[f32; 3]> {
        self.vertices.get(i as usize).map(|v| v.position)
    }

    pub fn entries_at(&self, col: u32, row: u32) -> Vec<u32> {
        let Some(cells) = self.ripple_cells.as_ref() else {
            return Vec::new();
        };
        let g = row * (cells.width as u32 + 1) + col;
        if self.vertex_sources.is_none() {
            return if (g as usize) < self.vertices.len() {
                vec![g]
            } else {
                Vec::new()
            };
        }
        let mut out: Vec<u32> = self
            .slots
            .as_ref()
            .map_or(&[][..], |s| &s.indices[..])
            .iter()
            .copied()
            .filter(|&i| self.entry_source(i) == VertexSource::Grid(g))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn apply_cell_heights(
        &mut self,
        queue: &wgpu::Queue,
        changes: &[(usize, i16, i16)],
        sine: &[i16; 1024],
    ) -> RippleUpload {
        let Some(grids) = self.set_cell_heights(changes) else {
            return RippleUpload::default();
        };
        let Some(slots) = self.slots.as_ref() else {
            return RippleUpload::default();
        };
        let (bw, bh) = (slots.bw, slots.bh);
        let vw = self.ripple_cells.as_ref().map_or(1, |c| c.width as u32 + 1);
        let mut blocks = Vec::new();
        for &g in &grids {
            blocks_of_vertex(g % vw, g / vw, bw, bh, &mut blocks);
        }
        let moved = if self.vertex_sources.is_none() {
            self.place_grid_vertices(&grids, sine)
        } else {
            Vec::new()
        };
        let mut writes = BlockWrites {
            vertices: moved,
            ..Default::default()
        };
        let built = self.rebuild_blocks(&mut blocks, &mut writes, sine);
        debug_assert!(built);
        self.upload(None, queue, writes, blocks.len())
    }

    pub fn apply_lod_words(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lod: Option<&mandrake_tag::mesh::MeshLod>,
        sine: &[i16; 1024],
    ) -> Option<RippleUpload> {
        let cells = self.ripple_cells.take()?;
        let Some(slots) = self.slots.as_mut() else {
            self.ripple_cells = Some(cells);
            return None;
        };
        let (bw, bh) = (slots.bw, slots.bh);
        let detail = slots.detail.is_some();
        let mut blocks = Vec::new();
        for by in 0..bh {
            for bx in 0..bw {
                let b = (by * bw + bx) as usize;
                let w = cells.word(lod, bx, by);
                if slots.words[b] != w {
                    slots.words[b] = w;
                    blocks.push(b as u32);
                    if detail {
                        if bx > 0 {
                            blocks.push(b as u32 - 1);
                        }
                        if bx + 1 < bw {
                            blocks.push(b as u32 + 1);
                        }
                        if by > 0 {
                            blocks.push(b as u32 - bw);
                        }
                        if by + 1 < bh {
                            blocks.push(b as u32 + bw);
                        }
                    }
                }
            }
        }
        self.ripple_cells = Some(cells);
        if blocks.is_empty() {
            return Some(RippleUpload::default());
        }
        let mut writes = BlockWrites::default();
        if !self.rebuild_blocks(&mut blocks, &mut writes, sine) {
            return None;
        }
        Some(self.upload(Some(device), queue, writes, blocks.len()))
    }

    fn set_cell_heights(&mut self, changes: &[(usize, i16, i16)]) -> Option<Vec<u32>> {
        let cells = self.ripple_cells.as_mut()?;
        let (w, h) = (cells.width, cells.height);
        let vw = w as u32 + 1;
        let mut grids = Vec::new();
        for &(i, physical, media) in changes {
            let Some(cell) = cells.cells.get_mut(i) else {
                continue;
            };
            if (cell.physical_height, cell.media_height) == (physical, media) {
                continue;
            }
            cell.physical_height = physical;
            cell.media_height = media;
            let (x, y) = ((i % w) as u32, (i / w) as u32);
            let xs: &[u32] = if x as usize == w - 1 {
                &[x, x + 1]
            } else {
                &[x]
            };
            let ys: &[u32] = if y as usize == h - 1 {
                &[y, y + 1]
            } else {
                &[y]
            };
            for &r in ys {
                for &c in xs {
                    grids.push(r * vw + c);
                }
            }
        }
        (!grids.is_empty()).then_some(grids)
    }

    fn place_grid_vertices(&mut self, grids: &[u32], sine: &[i16; 1024]) -> Vec<u32> {
        let (Some(cells), Some(slots)) = (self.ripple_cells.take(), self.slots.as_ref()) else {
            return Vec::new();
        };
        for &g in grids {
            let item = slots.media_of[g as usize];
            if item != NONE {
                let vw = cells.width as u32 + 1;
                let v = &mut self.media_vertices[item as usize];
                v.media_height = cells.cell(g % vw, g / vw).media_height;
                let v = *v;
                self.vertices[g as usize].position = self.media_position(&v, &cells, g, sine);
            } else {
                self.vertices[g as usize].position = self.grid_render_position(&cells, g);
            }
        }
        self.ripple_cells = Some(cells);
        grids.to_vec()
    }

    fn media_position(
        &self,
        v: &MediaVertex,
        cells: &RippleCells,
        g: u32,
        sine: &[i16; 1024],
    ) -> [f32; 3] {
        match self.media_time {
            Some(t) => media_vertex_render_position(
                v,
                self.media,
                self.cell_size,
                self.half_w,
                self.half_d,
                t,
                sine,
            ),
            None => self.grid_render_position(cells, g),
        }
    }
}

#[derive(Debug, Default)]
struct BlockWrites {
    vertices: Vec<u32>,
    occluders: Vec<u32>,
    media_entries: Vec<u32>,
    media_occluders: Vec<u32>,
    indices: Option<Range<u32>>,
}

impl TerrainMesh {
    fn rebuild_blocks(
        &mut self,
        blocks: &mut Vec<u32>,
        writes: &mut BlockWrites,
        sine: &[i16; 1024],
    ) -> bool {
        blocks.sort_unstable();
        blocks.dedup();
        let (Some(cells), Some(mut slots)) = (self.ripple_cells.take(), self.slots.take()) else {
            return false;
        };
        let mut segments: Vec<(usize, Vec<u32>)> = Vec::new();
        let mut ok = true;
        for &b in blocks.iter() {
            let built = if slots.detail.is_some() {
                self.rebuild_detail_block(&mut slots, &cells, b as usize, writes, sine)
            } else {
                self.rebuild_plain_block(&mut slots, &cells, b as usize, writes, sine)
            };
            match built {
                Some(s) => segments.extend(s),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        writes.occluders.sort_unstable();
        writes.occluders.dedup();
        move_occluder_triangles(
            &self.vertices,
            &self.occluder_indices,
            &self.occluder_depth,
            &writes.occluders,
            &mut self.occluders,
        );
        for &t in &writes.media_occluders {
            self.media_occluder_triangles.push(t);
        }
        writes.media_occluders.sort_unstable();
        self.media_occluder_runs
            .extend(write_runs(&writes.media_occluders, MEDIA_WRITE_GAP / 3));
        writes.media_entries.sort_unstable();
        writes.media_entries.dedup();
        self.media_vertex_runs
            .extend(write_runs(&writes.media_entries, MEDIA_WRITE_GAP));
        writes.indices = lay_out_segments(&mut slots, segments);
        self.ripple_cells = Some(cells);
        self.slots = Some(slots);
        ok
    }

    fn kill_media(&mut self, slots: &mut BlockSlots, e: u32) {
        let item = slots.media_of[e as usize];
        if item == NONE {
            return;
        }
        if item & MID != 0 {
            let i = item & !MID;
            self.media_mids[i as usize].index = NONE;
            slots.free_mids.push(i);
        } else {
            self.media_vertices[item as usize].index = NONE;
            slots.free_media.push(item);
        }
        slots.media_of[e as usize] = NONE;
    }

    fn add_media_vertex(
        &mut self,
        slots: &mut BlockSlots,
        v: MediaVertex,
        writes: &mut BlockWrites,
    ) {
        let e = v.index;
        let i = match slots.free_media.pop() {
            Some(i) => {
                self.media_vertices[i as usize] = v;
                i
            }
            None => {
                self.media_vertices.push(v);
                self.media_vertices.len() as u32 - 1
            }
        };
        slots.media_of[e as usize] = i;
        if !slots.media_run[e as usize] {
            slots.media_run[e as usize] = true;
            writes.media_entries.push(e);
        }
    }

    fn add_media_mid(&mut self, slots: &mut BlockSlots, mid: MediaMid, writes: &mut BlockWrites) {
        let e = mid.index;
        let i = match slots.free_mids.pop() {
            Some(i) => {
                self.media_mids[i as usize] = mid;
                i
            }
            None => {
                self.media_mids.push(mid);
                self.media_mids.len() as u32 - 1
            }
        };
        slots.media_of[e as usize] = MID | i;
        if !slots.media_run[e as usize] {
            slots.media_run[e as usize] = true;
            writes.media_entries.push(e);
        }
    }

    fn set_block_occluders(
        &mut self,
        slots: &mut BlockSlots,
        b: usize,
        old_dry: usize,
        dry: Vec<([u32; 3], OccluderDepth)>,
        writes: &mut BlockWrites,
    ) -> bool {
        let old_pool = slots.occ_pool(b);
        if !slots.fit_occluders(b, dry.len() as u32) {
            return false;
        }
        let pool = slots.occ_pool(b);
        for &t in &old_pool[..old_dry.min(old_pool.len())] {
            if !pool[..dry.len()].contains(&t) {
                let t = t as usize;
                self.occluder_indices[3 * t..3 * t + 3].copy_from_slice(&[0; 3]);
                self.occluder_depth[t] = OccluderDepth::UNUSED;
                writes.occluders.push(t as u32);
            }
        }
        for (j, (tri, depth)) in dry.into_iter().enumerate() {
            let t = pool[j];
            let media = tri.iter().any(|&i| slots.media_of[i as usize] != NONE)
                || depth.0.iter().any(
                    |p| matches!(*p, DepthPoint::Vertex(i) if slots.media_of[i as usize] != NONE),
                );
            if media && !slots.media_occ[t as usize] {
                slots.media_occ[t as usize] = true;
                writes.media_occluders.push(t);
            }
            let t = t as usize;
            self.occluder_indices[3 * t..3 * t + 3].copy_from_slice(&tri);
            self.occluder_depth[t] = depth;
            writes.occluders.push(t as u32);
        }
        true
    }

    fn rebuild_plain_block(
        &mut self,
        slots: &mut BlockSlots,
        cells: &RippleCells,
        b: usize,
        writes: &mut BlockWrites,
        sine: &[i16; 1024],
    ) -> Option<Vec<(usize, Vec<u32>)>> {
        let nb = slots.nb();
        let (bx, by) = (b as u32 % slots.bw, b as u32 / slots.bw);
        let vw = cells.width as u32 + 1;
        let (mut media, mut dry) = (Vec::new(), Vec::new());
        for (tri, m) in super::wound_block_triangles(slots.words[b], 2 * bx, 2 * by, vw) {
            if m {
                media.extend_from_slice(&tri);
            } else {
                dry.extend_from_slice(&tri);
            }
        }
        let (old_m, old_d) = (slots.segment(b).to_vec(), slots.segment(nb + b).to_vec());
        if old_m != media || old_d != dry {
            for &g in old_m.iter().chain(&old_d) {
                slots.uses[g as usize] -= 1;
            }
            for &g in media.iter().chain(&dry) {
                slots.uses[g as usize] += 1;
            }
            for &g in old_m.iter().chain(&old_d).chain(&media).chain(&dry) {
                let used = slots.uses[g as usize] > 0;
                let has = slots.media_of[g as usize] != NONE;
                if used == has {
                    continue;
                }
                if used {
                    let v = self
                        .media
                        .and_then(|_| cells.media_vertex(g, g % vw, g / vw));
                    let Some(v) = v else {
                        continue;
                    };
                    self.add_media_vertex(slots, v, writes);
                    self.vertices[g as usize].position = self.media_position(&v, cells, g, sine);
                } else {
                    self.kill_media(slots, g);
                    self.vertices[g as usize].position = self.grid_render_position(cells, g);
                }
                writes.vertices.push(g);
            }
        }
        let occluders = dry
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| (*t, OccluderDepth::corners(*t)))
            .collect();
        if !self.set_block_occluders(slots, b, old_d.len() / 3, occluders, writes) {
            return None;
        }
        let mut out = Vec::new();
        if old_m != media {
            out.push((b, media));
        }
        if old_d != dry {
            out.push((nb + b, dry));
        }
        Some(out)
    }

    fn rebuild_detail_block(
        &mut self,
        slots: &mut BlockSlots,
        cells: &RippleCells,
        b: usize,
        writes: &mut BlockWrites,
        sine: &[i16; 1024],
    ) -> Option<Vec<(usize, Vec<u32>)>> {
        let nb = slots.nb();
        let (media, dry) = self.detail_block(slots, cells, b);
        let (old_m, old_d) = (slots.segment(b).to_vec(), slots.segment(nb + b).to_vec());
        for &e in old_m.iter().chain(&old_d) {
            self.kill_media(slots, e);
        }
        let n = (media.len() + dry.len()) as u32;
        if !slots.fit_entries(b, n) {
            return None;
        }
        let pool = slots.entry_pool(b);
        let (cols, rows) = (cells.width as f32, cells.height as f32);
        let vw = cells.width as u32 + 1;
        let detail = slots.detail.take().expect("detail");
        let layer = |e: u8| detail.entry_layers.get(e as usize).copied().flatten();
        let mut segs = [Vec::new(), Vec::new()];
        let mut occluders = Vec::new();
        let nm = media.len();
        let media_on = self.media.is_some();
        for (k, tri) in media.iter().chain(&dry).enumerate() {
            let s = pool[k];
            let entries = [3 * s, 3 * s + 1, 3 * s + 2];
            for (c, &e) in tri.corners.iter().zip(&entries) {
                let cell = cells.cell(c.col, c.row);
                let mut position = c.position;
                match c.averaged {
                    None => {
                        if let Some(v) = media_on
                            .then(|| cells.media_vertex(e, c.col, c.row))
                            .flatten()
                        {
                            position = self.media_position(&v, cells, c.grid, sine);
                            self.add_media_vertex(slots, v, writes);
                        }
                    }
                    Some(pair) if media_on => {
                        let corners = pair.map(|(pc, pr)| {
                            let g = pr * vw + pc;
                            cells
                                .media_vertex(g, pc, pr)
                                .ok_or_else(|| self.grid_render_position(cells, g))
                        });
                        if corners.iter().any(|c| c.is_ok()) {
                            if self.media_time.is_some() {
                                let [a, b] = corners.map(|c| match c {
                                    Ok(v) => {
                                        let g = v.index;
                                        self.media_position(&v, cells, g, sine)
                                    }
                                    Err(p) => p,
                                });
                                position = mean(a, b);
                            }
                            self.add_media_mid(slots, MediaMid { index: e, corners }, writes);
                        }
                    }
                    Some(_) => {}
                }
                self.vertices[e as usize] = TerrainVertex {
                    position,
                    uv: [c.col as f32 / cols, c.row as f32 / rows],
                    cell_flags: u32::from(cell.flags),
                };
                if let Some(sources) = self.vertex_sources.as_mut() {
                    sources[e as usize] = match c.averaged {
                        Some([a, b]) => VertexSource::Mean(a.1 * vw + a.0, b.1 * vw + b.0),
                        None => VertexSource::Grid(c.grid),
                    };
                }
                self.detail_vertices[e as usize] =
                    DetailVertex::new(tri.word, c.v, (c.col, c.row), layer);
                writes.vertices.push(e);
            }
            segs[usize::from(k >= nm)].extend_from_slice(&entries);
            if k >= nm {
                occluders.push((entries, tri));
            }
        }
        let all: Vec<(u32, [f32; 3], u32)> = media
            .iter()
            .chain(&dry)
            .enumerate()
            .flat_map(|(k, t)| {
                let s = pool[k];
                t.corners
                    .iter()
                    .enumerate()
                    .map(move |(v, c)| (c.grid, c.position, 3 * s + v as u32))
            })
            .collect();
        let occluders: Vec<([u32; 3], OccluderDepth)> = occluders
            .into_iter()
            .map(|(entries, tri)| {
                let depth = tri
                    .depth
                    .iter()
                    .map(|&(g, p)| match all.iter().find(|a| a.0 == g) {
                        Some(&(_, at, e)) if at == p => DepthPoint::Vertex(e),
                        _ => DepthPoint::Fixed(p),
                    })
                    .collect();
                (entries, OccluderDepth(depth))
            })
            .collect();
        slots.detail = Some(detail);
        if !self.set_block_occluders(slots, b, old_d.len() / 3, occluders, writes) {
            return None;
        }
        let [m, d] = segs;
        let mut out = Vec::new();
        if old_m != m {
            out.push((b, m));
        }
        if old_d != d {
            out.push((nb + b, d));
        }
        Some(out)
    }

    fn detail_block(
        &self,
        slots: &BlockSlots,
        cells: &RippleCells,
        b: usize,
    ) -> (Vec<BlockTri>, Vec<BlockTri>) {
        let (bw, bh) = (slots.bw as i64, slots.bh as i64);
        let (bx, by) = (b as i64 % bw, b as i64 / bw);
        let (col, row) = (2 * bx as u32, 2 * by as u32);
        let vw = cells.width as u32 + 1;
        let word = slots.words[b];
        let neighbour = |s: usize| {
            let (dx, dy) = [(0, -1), (1, 0), (0, 1), (-1, 0)][s];
            let (x, y) = (bx + dx, by + dy);
            ((0..bw).contains(&x) && (0..bh).contains(&y)).then(|| {
                (
                    slots.words[(y * bw + x) as usize],
                    (2 * x as u32, 2 * y as u32),
                )
            })
        };
        let averages = |w: u16, s: usize| u32::from(w) >> s & 1 == 0;
        let orig = |(c, r): (u32, u32)| self.grid_render_position(cells, r * vw + c);
        let at = |o: (u32, u32), p: [(u32, u32); 2]| p.map(|(c, r)| (o.0 + c, o.1 + r));
        let side_of = |l: (u32, u32)| match l {
            (1, 0) => Some(0),
            (2, 1) => Some(1),
            (1, 2) => Some(2),
            (0, 1) => Some(3),
            _ => None,
        };
        let read = |l: (u32, u32)| -> [f32; 3] {
            let abs = (col + l.0, row + l.1);
            if let Some(s @ (0 | 3)) = side_of(l)
                && let Some((w, o)) = neighbour(s)
                && averages(w, (s + 2) % 4)
            {
                let [a, b] = at(o, side_pair((s + 2) % 4));
                return mean(orig(a), orig(b));
            }
            orig(abs)
        };
        let fin = |l: (u32, u32)| -> ([f32; 3], Option<[(u32, u32); 2]>) {
            let abs = (col + l.0, row + l.1);
            let pair = if l == (1, 1) {
                (word & mandrake_tag::mesh::LOD_EDGE_SUBDIVIDED_MASK == 0).then(|| {
                    let p = if super::block_diagonal(col, row) == 0 {
                        [(2, 2), (0, 0)]
                    } else {
                        [(2, 0), (0, 2)]
                    };
                    at((col, row), p)
                })
            } else if let Some(s) = side_of(l) {
                let own = averages(word, s).then(|| at((col, row), side_pair(s)));
                let other = neighbour(s)
                    .filter(|&(w, _)| averages(w, (s + 2) % 4))
                    .map(|(_, o)| at(o, side_pair((s + 2) % 4)));
                if s == 0 || s == 3 {
                    own.or(other)
                } else {
                    other.or(own)
                }
            } else {
                None
            };
            match pair {
                Some([a, b]) => (mean(orig(a), orig(b)), Some([a, b])),
                None => (orig(abs), None),
            }
        };
        let table = &slots.detail.as_ref().expect("detail").table;
        let halves = super::detail_block_halves(word);
        let reads: Vec<[(u32, [f32; 3]); 3]> = halves
            .iter()
            .map(|h| h.corners.map(|l| ((row + l.1) * vw + col + l.0, read(l))))
            .collect();
        let groups: Vec<u32> = halves
            .iter()
            .map(|h| super::detail_depth_group(word, col, row, h.side, h.slot))
            .collect();
        let (mut media, mut dry) = (Vec::new(), Vec::new());
        for (i, half) in halves.iter().enumerate() {
            let mut depth: Vec<(u32, [f32; 3])> = Vec::with_capacity(6);
            if !half.media {
                for (j, points) in reads.iter().enumerate() {
                    if groups[j] != groups[i] {
                        continue;
                    }
                    for p in points {
                        if !depth.iter().any(|q| q.0 == p.0) {
                            depth.push(*p);
                        }
                    }
                }
            }
            let mut corners: [BlockCorner; 3] = core::array::from_fn(|v| {
                let l = half.corners[v];
                let (position, averaged) = fin(l);
                let (c, r) = (col + l.0, row + l.1);
                BlockCorner {
                    grid: r * vw + c,
                    col: c,
                    row: r,
                    v,
                    position,
                    averaged,
                }
            });
            let p = |k: usize| (corners[k].col as i64, corners[k].row as i64);
            let (a, b, d) = (p(0), p(1), p(2));
            if (b.0 - a.0) * (d.1 - a.1) - (b.1 - a.1) * (d.0 - a.0) < 0 {
                corners.swap(1, 2);
            }
            let tri = BlockTri {
                corners,
                word: table.word(col + half.cell.0, row + half.cell.1, half.slot),
                depth,
            };
            if half.media {
                media.push(tri);
            } else {
                dry.push(tri);
            }
        }
        (media, dry)
    }

    fn upload(
        &mut self,
        device: Option<&wgpu::Device>,
        queue: &wgpu::Queue,
        mut writes: BlockWrites,
        blocks: usize,
    ) -> RippleUpload {
        let mut stats = RippleUpload {
            blocks: blocks as u32,
            ..Default::default()
        };
        writes.vertices.sort_unstable();
        writes.vertices.dedup();
        stats.vertices = writes.vertices.len() as u32;
        stats.occluder_triangles = writes.occluders.len() as u32;
        for run in write_runs(&writes.vertices, MEDIA_WRITE_GAP) {
            let r = run.start as usize..run.end as usize;
            queue.write_buffer(
                &self.vertex_buffer,
                u64::from(run.start) * size_of::<TerrainVertex>() as u64,
                bytemuck::cast_slice(&self.vertices[r.clone()]),
            );
            stats.writes += 1;
            if let Some(buffer) = self.detail_buffer.as_ref() {
                queue.write_buffer(
                    buffer,
                    u64::from(run.start) * size_of::<DetailVertex>() as u64,
                    bytemuck::cast_slice(&self.detail_vertices[r]),
                );
                stats.writes += 1;
            }
        }
        for run in write_runs(&writes.occluders, MEDIA_WRITE_GAP / 3) {
            queue.write_buffer(
                &self.occluder_buffer,
                u64::from(run.start) * 3 * size_of::<OccluderVertex>() as u64,
                bytemuck::cast_slice(&self.occluders[run.start as usize * 3..run.end as usize * 3]),
            );
            stats.writes += 1;
        }
        let Some(slots) = self.slots.as_ref() else {
            return stats;
        };
        if let Some(range) = writes.indices {
            let bytes = (slots.indices.len() * 4) as u64;
            if bytes > self.index_buffer.size() {
                let device = device.expect("a count changes only with the words");
                self.index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Terrain Grid from MeshTag Index Buffer"),
                    size: index_capacity(slots.indices.len()) as u64 * 4,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&slots.indices));
                stats.indices = slots.indices.len() as u32;
            } else {
                queue.write_buffer(
                    &self.index_buffer,
                    u64::from(range.start) * 4,
                    bytemuck::cast_slice(&slots.indices[range.start as usize..range.end as usize]),
                );
                stats.indices = range.end - range.start;
            }
            stats.writes += 1;
        }
        self.num_indices = slots.indices.len() as u32;
        self.num_media_indices = 3 * slots.seg[slots.nb()];
        self.num_occluder_vertices = 3 * (slots.spare_occ + slots.occ_chunks_used * CHUNK);
        stats
    }
}

pub(crate) fn index_capacity(n: usize) -> usize {
    n + n / 8 + 3 * (SPARE_TRIANGLES as usize)
}

fn lay_out_segments(
    slots: &mut BlockSlots,
    mut segments: Vec<(usize, Vec<u32>)>,
) -> Option<Range<u32>> {
    if segments.is_empty() {
        return None;
    }
    segments.sort_unstable_by_key(|s| s.0);
    let same_lengths = segments
        .iter()
        .all(|(k, t)| t.len() == slots.segment(*k).len());
    let first = 3 * slots.seg[segments[0].0];
    if same_lengths {
        let mut end = first;
        for (k, t) in &segments {
            let start = 3 * slots.seg[*k] as usize;
            slots.indices[start..start + t.len()].copy_from_slice(t);
            end = (start + t.len()) as u32;
        }
        return Some(first..end);
    }
    let n = slots.seg.len() - 1;
    let mut indices = Vec::with_capacity(slots.indices.len() + 3 * CHUNK as usize * segments.len());
    indices.extend_from_slice(&slots.indices[..first as usize]);
    let mut seg = Vec::with_capacity(n + 1);
    seg.extend_from_slice(&slots.seg[..=segments[0].0]);
    let mut next = segments.iter().peekable();
    for k in segments[0].0..n {
        match next.peek() {
            Some((j, t)) if *j == k => {
                indices.extend_from_slice(t);
                next.next();
            }
            _ => indices.extend_from_slice(slots.segment(k)),
        }
        seg.push((indices.len() / 3) as u32);
    }
    slots.indices = indices;
    slots.seg = seg;
    Some(first..slots.indices.len() as u32)
}
fn clamp_block(x: i32, n: i32) -> i32 {
    if x < 0 {
        0
    } else if x >= n {
        n - 1
    } else {
        x
    }
}

fn match_neighbour_edges(words: &mut [u16], bx: i32, by: i32, bw: i32, bh: i32) {
    let at = |x: i32, y: i32| (y * bw + x) as usize;
    let mut w = words[at(bx, by)];
    let up = if by != 0 { words[at(bx, by - 1)] } else { 0 };
    let down = if by + 1 < bh {
        words[at(bx, by + 1)]
    } else {
        0
    };
    let left = if bx != 0 { words[at(bx - 1, by)] } else { 0 };
    let right = if bx + 1 < bw {
        words[at(bx + 1, by)]
    } else {
        0
    };
    if w & 1 == 0 && up & 4 != 0 {
        w |= 1;
    }
    if w & 2 == 0 && right & 8 != 0 {
        w |= 2;
    }
    if w & 4 == 0 && down & 1 != 0 {
        w |= 4;
    }
    if w & 8 == 0 && left & 2 != 0 {
        w |= 8;
    }
    words[at(bx, by)] = w;
}

pub fn make_cell_region_high_lod(
    lod: &mut mandrake_tag::mesh::MeshLod,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) {
    let (bw, bh) = (lod.block_width as i32, lod.block_height as i32);
    if bw == 0 || bh == 0 || lod.words.len() < (bw * bh) as usize {
        return;
    }
    let words = &mut lod.words;
    let (min_x, max_x) = (x0.min(x1) >> 1, x0.max(x1) >> 1);
    let (min_y, max_y) = (y0.min(y1) >> 1, y0.max(y1) >> 1);
    for by in clamp_block(min_y, bh)..=clamp_block(max_y, bh) {
        for bx in clamp_block(min_x, bw)..=clamp_block(max_x, bw) {
            words[(by * bw + bx) as usize] |= 0xf;
        }
    }
    let left = clamp_block(min_x - 1, bw);
    let right = clamp_block(max_x + 1, bw);
    let (top, side_from) = if min_y - 1 < 0 {
        (0, 1)
    } else if min_y > bh {
        (bh - 1, bh)
    } else {
        (min_y - 1, min_y)
    };
    let bottom = clamp_block(max_y + 1, bh);
    for bx in left..=right {
        match_neighbour_edges(words, bx, top, bw, bh);
        match_neighbour_edges(words, bx, bottom, bw, bh);
    }
    for by in side_from..bottom {
        match_neighbour_edges(words, left, by, bw, bh);
        match_neighbour_edges(words, right, by, bw, bh);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LodCell {
    pub height: i16,
    pub flags: u16,
    pub model: i16,
}

#[allow(clippy::too_many_arguments)]
pub fn recalculate_lod_for_cell_region(
    lod: &mut mandrake_tag::mesh::MeshLod,
    mesh_width: i32,
    mesh_height: i32,
    cell: impl Fn(i32, i32) -> LodCell,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) {
    let (bw, bh) = (lod.block_width as i32, lod.block_height as i32);
    if bw == 0 || bh == 0 || lod.words.len() < (bw * bh) as usize {
        return;
    }
    let bx0 = clamp_block((x0.min(x1) >> 1) - 1, bw);
    let bx1 = clamp_block((x0.max(x1) >> 1) + 1, bw);
    let by0 = clamp_block((y0.min(y1) >> 1) - 1, bh);
    let by1 = clamp_block((y0.max(y1) >> 1) + 1, bh);
    let curved =
        |a: i16, b: i16, mid: i16| (i32::from(a) + i32::from(b) - 2 * i32::from(mid)).abs() > 0x80;
    for by in by0..=by1 {
        for bx in bx0..=bx1 {
            let (x, y) = (2 * bx, 2 * by);
            let x2 = (x + 2).min(mesh_width - 1);
            let y2 = (y + 2).min(mesh_height - 1);
            let c00 = cell(x, y);
            let c10 = cell(x + 1, y);
            let c11 = cell(x + 1, y + 1);
            let c01 = cell(x, y + 1);
            let c20 = cell(x2, y);
            let c21 = cell(x2, y + 1);
            let c22 = cell(x2, y2);
            let c12 = cell(x + 1, y2);
            let c02 = cell(x, y2);
            let mut w: u16 = 0;
            if curved(c20.height, c00.height, c10.height) {
                w |= 1;
            }
            if curved(c22.height, c20.height, c21.height) {
                w |= 2;
            }
            if curved(c02.height, c22.height, c12.height) {
                w |= 4;
            }
            if curved(c00.height, c02.height, c01.height) {
                w |= 8;
            }
            if curved(c10.height, c12.height, c11.height)
                || curved(c01.height, c21.height, c11.height)
            {
                w = 0xf;
            }
            for (c, edges) in [(c00, 9), (c10, 3), (c11, 6), (c01, 0xc)] {
                if c.model != -1 {
                    w |= edges;
                }
            }
            for (c, edges, drawn) in [
                (c00, 9, 0x10),
                (c10, 3, 0x20),
                (c11, 6, 0x40),
                (c01, 0xc, 0x80),
            ] {
                w |= if c.flags & 0x4000 != 0 { edges } else { drawn };
            }
            for (c, b0800, b1000) in [
                (c00, 0x8000, 0x0100),
                (c10, 0x0200, 0x0400),
                (c11, 0x1000, 0x0800),
                (c01, 0x4000, 0x2000),
            ] {
                if c.flags & 0x0800 != 0 {
                    w |= b0800;
                }
                if c.flags & 0x1000 != 0 {
                    w |= b1000;
                }
            }
            for (media, edge) in [(0x0300, 1), (0x0c00, 2), (0x3000, 4), (0xc000, 8)] {
                if w & media != 0 {
                    w |= edge;
                }
            }
            if w & 0xfff0 == 0xfff0 {
                w = 0xfff0;
            }
            lod.words[(by * bw + bx) as usize] = w;
        }
    }
    for by in by0..=by1 {
        for bx in bx0..=bx1 {
            match_neighbour_edges(&mut lod.words, bx, by, bw, bh);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_tag::mesh::MeshLod;

    fn lod(bw: usize, bh: usize, word: u16) -> MeshLod {
        MeshLod {
            block_width: bw,
            block_height: bh,
            words: vec![word; bw * bh],
        }
    }

    #[test]
    fn high_lod_subdivides_the_region_and_matches_the_ring() {
        let mut l = lod(8, 8, 0x00f0);
        make_cell_region_high_lod(&mut l, 6, 6, 9, 9);
        let w = |x: usize, y: usize| l.words[y * 8 + x];
        for (x, y) in [(3, 3), (4, 3), (3, 4), (4, 4)] {
            assert_eq!(w(x, y), 0x00ff);
        }
        assert_eq!(w(2, 3), 0x00f2, "left neighbour: its right edge");
        assert_eq!(w(5, 4), 0x00f8, "right neighbour: its left edge");
        assert_eq!(w(3, 2), 0x00f4, "upper neighbour: its bottom edge");
        assert_eq!(w(4, 5), 0x00f1, "lower neighbour: its top edge");
        assert_eq!(w(2, 2), 0x00f0, "corner: no shared edge");
        assert_eq!(w(0, 0), 0x00f0);
    }

    #[test]
    fn recalculate_follows_heights_models_and_media() {
        let mut l = lod(4, 4, 0x00ff);
        let flat = |_: i32, _: i32| LodCell {
            model: -1,
            ..Default::default()
        };
        recalculate_lod_for_cell_region(&mut l, 8, 8, flat, 0, 0, 7, 7);
        assert!(l.words.iter().all(|&w| w == 0x00f0));
        let bump = |x: i32, y: i32| LodCell {
            height: if (x, y) == (3, 3) { 0x100 } else { 0 },
            model: -1,
            ..Default::default()
        };
        recalculate_lod_for_cell_region(&mut l, 8, 8, bump, 0, 0, 7, 7);
        assert_eq!(
            l.words[4 + 1],
            0x00ff,
            "block (1, 1) holds the bump in its centre"
        );
        assert_eq!(
            l.words[4 + 2],
            0x00f8,
            "its right neighbour matches the shared edge"
        );
        let water = |_: i32, _: i32| LodCell {
            flags: 0x1800,
            model: -1,
            ..Default::default()
        };
        recalculate_lod_for_cell_region(&mut l, 8, 8, water, 0, 0, 7, 7);
        assert!(l.words.iter().all(|&w| w == 0xfff0));
        let model = |x: i32, y: i32| LodCell {
            model: if (x, y) == (0, 0) { 3 } else { -1 },
            ..Default::default()
        };
        recalculate_lod_for_cell_region(&mut l, 8, 8, model, 0, 0, 7, 7);
        assert_eq!(l.words[0], 0x00f9);
    }
}
