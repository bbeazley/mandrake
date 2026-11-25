use crate::tfl_game::FrameTime;
use crate::tfl_landscape::{CELL_FIRST_PASS, CELL_NOT_DRAWN, CELL_WATER, CELL_WAVES};
use crate::tfl_map::TflMap;
use crate::tfl_objects::TFL_CELL_SIZE;
use mandrake_render::water::WaterShading;
use mandrake_render::water::ripples::Disturbance;
use mandrake_render::{BillboardRenderer, TerrainRenderer, TerrainVertex};
use mandrake_tag::MeshTag;
use mandrake_tag::tfl::TflCell;
use mandrake_view::media::{
    CELL_FLAG_MEDIA_VERTEX, CELL_FLAG_MEDIA_VERTEX_SUBMERGED, MediaSurface, MediaWobble,
};
use mandrake_view::water::WaterSettings;
use tracing::info;

const MEDIA_TRIANGLES: u16 = 0x1800;

const TICKS_PER_SECOND: f64 = 30.0;

pub(crate) fn water_mesh(map: &MeshTag, cells: &[TflCell]) -> MeshTag {
    let mut mesh = map.clone();
    for (c, t) in mesh.cells.iter_mut().zip(cells) {
        c.physical_height = t.physical_height;
        c.media_height = 0;
        c.model_index = -1;
        c.flags = 0;
        if t.flags & CELL_WATER != 0 && t.physical_height < 1 {
            c.flags |= CELL_FLAG_MEDIA_VERTEX;
            if t.flags & CELL_WAVES != 0 {
                c.flags |= CELL_FLAG_MEDIA_VERTEX_SUBMERGED;
            }
        }
        let drawn_at_zero = c.flags & CELL_FLAG_MEDIA_VERTEX != 0;
        if (t.flags & CELL_FIRST_PASS != 0 || drawn_at_zero) && t.flags & CELL_NOT_DRAWN == 0 {
            c.flags |= MEDIA_TRIANGLES;
        }
    }
    mesh
}

pub(crate) fn water_triangles(mesh: &MeshTag, cells: &[TflCell]) -> Vec<([u32; 3], bool)> {
    let (w, h) = (u32::from(mesh.cell_width), u32::from(mesh.cell_height));
    let vw = w + 1;
    mandrake_render::mesh::landscape_triangles(mesh, None)
        .into_iter()
        .filter(|(tri, _)| {
            let col = tri.iter().map(|i| i % vw).min().unwrap_or(0).min(w - 1);
            let row = tri.iter().map(|i| i / vw).min().unwrap_or(0).min(h - 1);
            cells
                .get((row * w + col) as usize)
                .is_none_or(|c| c.flags & CELL_NOT_DRAWN == 0)
        })
        .collect()
}

pub(crate) fn lowered_vertices(mesh: &MeshTag) -> Vec<TerrainVertex> {
    let mut plain = mesh.clone();
    for c in &mut plain.cells {
        c.flags = 0;
    }
    mandrake_render::mesh::grid_vertices(&plain, TFL_CELL_SIZE, 0)
}

pub(crate) fn media_surface(map: &TflMap) -> MediaSurface {
    let wobble = map
        .tfl_mesh
        .media_tag
        .and_then(|id| map.tag(b"medi", id))
        .map(|medi| MediaWobble::from_media_bytes(&medi))
        .unwrap_or_default();
    MediaSurface {
        mesh_flags: 0,
        wobble,
    }
}

pub(crate) fn has_water(cells: &[TflCell]) -> bool {
    cells
        .iter()
        .any(|c| c.flags & CELL_FIRST_PASS != 0 && c.flags & CELL_NOT_DRAWN == 0)
}

pub(crate) struct TflWater {
    shading: WaterShading,
    mesh: MeshTag,
    sources: crate::water_ripples_input::RippleSources,
    disturbances: Vec<Disturbance>,
    started: crate::clock::Instant,
    last_step: Option<f64>,
}

pub(crate) struct WaterGpu<'g> {
    pub(crate) device: &'g wgpu::Device,
    pub(crate) queue: &'g wgpu::Queue,
    pub(crate) terrain: &'g TerrainRenderer,
    pub(crate) billboards: &'g BillboardRenderer,
}

impl TflWater {
    pub(crate) fn new(
        gpu: &WaterGpu<'_>,
        map: &TflMap,
        cells: &[TflCell],
        settings: &WaterSettings,
        texels: Option<&mandrake_view::landscape::LandscapeTexels>,
    ) -> Option<Self> {
        if !has_water(cells) {
            info!("TFL --water enhanced: no water on {}", map.mesh_name);
            return None;
        }
        let mesh = water_mesh(&map.mesh, cells);
        let surface = media_surface(map);
        let mut shading = WaterShading::new(
            gpu.device,
            gpu.queue,
            gpu.terrain,
            &mesh,
            TFL_CELL_SIZE,
            surface,
            settings,
            gpu.billboards.interface_format,
            gpu.billboards,
        );
        shading.set_lowered(
            gpu.device,
            gpu.queue,
            &mesh,
            &lowered_vertices(&mesh),
            Some(&water_triangles(&mesh, cells)),
        );
        if let Some(texels) = texels {
            shading.set_landscape(gpu.queue, &mesh, texels, &texels.rgba(&[]));
        }
        info!(
            "TFL --water enhanced on {}: wobble {:?}",
            map.mesh_name, surface.wobble
        );
        Some(Self {
            shading,
            mesh,
            sources: Default::default(),
            disturbances: Vec::new(),
            started: crate::clock::Instant::now(),
            last_step: None,
        })
    }

    pub(crate) fn shading(&self) -> &WaterShading {
        &self.shading
    }

    pub(crate) fn follow_ground(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cells: &[TflCell],
    ) {
        let mesh = water_mesh(&self.mesh, cells);
        self.shading.set_lowered(
            device,
            queue,
            &mesh,
            &lowered_vertices(&mesh),
            Some(&water_triangles(&mesh, cells)),
        );
        self.mesh = mesh;
    }

    pub(crate) fn has_ripples(&self) -> bool {
        self.shading.has_ripples()
    }

    pub(crate) fn gather(&mut self, world: &crate::world_view::WorldView) {
        if self.shading.has_ripples() {
            self.disturbances = self.sources.gather(world, &[]);
        }
    }

    pub(crate) fn before_passes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        (size, time): ((u32, u32), FrameTime),
        fixed: bool,
    ) {
        self.shading.update(queue, time.live as i32);
        self.shading.ensure_reflection_layer(device, size);
        let seconds = if fixed {
            f64::from(time.live) / TICKS_PER_SECOND
        } else {
            self.started.elapsed().as_secs_f64()
        };
        self.shading.set_wind(queue, seconds, None);
        if fixed {
            self.shading.flush_waves(queue);
            self.shading.set_wind(queue, seconds, None);
        }
        if self.shading.has_ripples() {
            let dt = if fixed {
                (1.0 / TICKS_PER_SECOND) as f32
            } else {
                self.last_step
                    .map_or(0.0, |last| (seconds - last).clamp(0.0, 0.1) as f32)
            };
            self.last_step = Some(seconds);
            let disturbances = std::mem::take(&mut self.disturbances);
            self.shading.step_ripples(queue, encoder, dt, &disturbances);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(h: i16, flags: u8) -> TflCell {
        TflCell {
            physical_height: h,
            flags,
            ..TflCell::default()
        }
    }

    #[test]
    fn tfls_cells_are_myth2_media_cells() {
        let cells = [
            cell(-840, CELL_WATER | CELL_WAVES | CELL_FIRST_PASS),
            cell(260, CELL_WATER | CELL_FIRST_PASS),
            cell(-300, 0),
            cell(-300, CELL_FIRST_PASS | CELL_NOT_DRAWN),
        ];
        let id = mandrake_common::FourCC::new(*b"test");
        let map = MeshTag {
            mesh_id: id,
            landscape_tag: id,
            media_tag: id,
            submesh_w: 2,
            submesh_h: 2,
            cell_width: 2,
            cell_height: 2,
            total_cells: 4,
            mesh_offset: 0,
            mesh_size: 0,
            cells: vec![Default::default(); 4],
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
        let mesh = water_mesh(&map, &cells);
        let f: Vec<u16> = mesh.cells.iter().map(|c| c.flags).collect();
        assert_eq!(f, vec![0x1d00, 0x1800, 0, 0]);
        assert_eq!(mesh.cells[0].physical_height, -840);
        assert_eq!(mesh.cells[0].media_height, 0);
        let tris = water_triangles(&mesh, &cells);
        assert_eq!(tris.len(), 6);
        assert_eq!(tris.iter().filter(|(_, m)| *m).count(), 4);
        let lowered = lowered_vertices(&mesh);
        assert_eq!(lowered.len(), 9);
        assert_eq!(lowered[0].position[1], -840.0 / 512.0);
        assert!(has_water(&cells));
        assert!(!has_water(&cells[2..3]));
    }
}
