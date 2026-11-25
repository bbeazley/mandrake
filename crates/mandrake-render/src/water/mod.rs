mod bindings;
mod category4;
mod cells;
mod draw;
pub mod fft;
mod pipelines;
pub mod ripples;
mod shelter;
#[cfg(test)]
mod tests;
mod uniforms;

use bindings::*;
use category4::*;
use cells::*;
use mandrake_view::water::grid::*;
use mandrake_view::water::light::*;
use mandrake_view::water::settings::*;
use pipelines::*;
use shelter::*;
use uniforms::*;

pub use shelter::{FETCH_MAX, ShelterMap};

use crate::media::{MediaSurface, MediaWobble};
use crate::mesh::TerrainMesh;
use crate::pipeline::TerrainRenderer;
use crate::vertex::TerrainVertex;
use bytemuck::{Pod, Zeroable};

pub struct WaterShading {
    pipelines: WaterPipelines,
    physical: bool,
    uniform_buffer: wgpu::Buffer,
    bind_groups: [wgpu::BindGroup; 2],
    bindings: WaterBindings,
    layer: Option<wgpu::Texture>,
    category4: Option<WaterCategory4>,
    display_format: wgpu::TextureFormat,
    ripples: Option<crate::water::ripples::RippleField>,
    wind: Option<mandrake_view::water::wind::RippleWind>,
    fft: Option<crate::water::fft::FftWaves>,
    gust_drift: (Option<f64>, f64, f64, [f64; 2]),
    shelter: Option<Shelter>,
    min_wavelength: f32,
    floor_texture: wgpu::Texture,
    map_colors: wgpu::Texture,
    cell_light: wgpu::Texture,
    color_mode: WaterColorMode,
    color_radius: f32,
    light_direction: Option<[f32; 3]>,
    specular: f32,
    light_fit_min_r2: f32,
    light_fallback: Option<(f32, f32)>,
    lowered: Option<wgpu::Buffer>,
    full: Option<FullGrid>,
    water_indices: Option<WaterIndices>,
    full_media_hash: Option<u64>,
    uniforms: WaterUniforms,
    wobble: MediaWobble,
    mesh_flags: u32,
}

impl WaterShading {
    pub const SHADER_SRC: &'static str = concat!(
        include_str!("../../shaders/scene_uniforms.wgsl"),
        include_str!("../../shaders/filter_helpers.wgsl"),
        include_str!("../../shaders/terrain.wgsl"),
        include_str!("../../shaders/fog.wgsl"),
        include_str!("../../shaders/water.wgsl"),
        include_str!("../../shaders/water_waves.wgsl"),
        include_str!("../../shaders/water_bed.wgsl"),
        include_str!("../../shaders/water_surface.wgsl")
    );

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainRenderer,
        mesh: &mandrake_tag::MeshTag,
        cell_size: f32,
        surface: MediaSurface,
        settings: &WaterSettings,
        display_format: wgpu::TextureFormat,
        billboards: &crate::billboard::BillboardRenderer,
    ) -> Self {
        let (w, h, info) = grid_info(mesh, surface.mesh_flags);
        let wet: Vec<(u32, u32)> = (0..w * h)
            .filter(|&i| info[i as usize][3] > 0.5)
            .map(|i| (i % w, i / w))
            .collect();
        tracing::info!(
            "--water enhanced: {} media-height vertices, e.g. {:?}",
            wet.len(),
            wet.iter()
                .step_by((wet.len() / 8).max(1))
                .collect::<Vec<_>>()
        );
        let floor_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Water Floor Heights"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let wobble = surface.wobble;
        let mut uniforms = initial(mesh, cell_size, &wobble, settings, display_format);
        let floor_view = floor_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let ripples = if settings.ripples {
            crate::water::ripples::RippleField::new(
                device,
                queue,
                &floor_view,
                (w, h),
                &info,
                [uniforms.grid[0], uniforms.grid[1], cell_size],
                settings.meters_per_cell,
                settings.ripple_strength,
                settings.ripple_damping,
            )
        } else {
            None
        };
        if let Some(field) = &ripples {
            let r = field.region();
            uniforms.ripple = [1.0, r[0], r[1], r[2]];
        }
        let still = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Water Ripples Still"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let still = still.create_view(&wgpu::TextureViewDescriptor::default());
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Water Uniforms"),
            size: std::mem::size_of::<WaterUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let layout = layout(device);
        let map_colors = cell_texture(device, mesh, "Water Map Colours");
        let cell_light = cell_texture(device, mesh, "Water Cell Light");
        write_cell_texture(
            queue,
            &cell_light,
            mesh,
            &vec![
                128u8;
                usize::from(mesh.cell_width).max(1) * usize::from(mesh.cell_height).max(1) * 4
            ],
        );
        let (fft, fft_view) = if settings.wind_ripples {
            let fft = crate::water::fft::FftWaves::new(
                device,
                crate::water::fft::FftSettings {
                    min_wavelength: settings.ripple_min_wavelength,
                    max_wavelength: settings.ripple_max_wavelength,
                },
            );
            let view = fft.texture().create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
            (Some(fft), view)
        } else {
            let dummy = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Water Waves Unused"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = dummy.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
            (None, view)
        };
        let wave_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Water Wave Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let fetch = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Water Fetch"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let fetch_view = fetch.create_view(&wgpu::TextureViewDescriptor::default());
        let shelter = (settings.shelter && settings.wind_ripples)
            .then(|| ShelterMap::new(&info, (w, h), settings.meters_per_cell));
        let coverage = coverage(&info, (w as usize, h as usize), mesh);
        let initial: Vec<u8> = coverage.iter().flat_map(|&g| [255, g, 0, 255]).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &fetch,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &initial,
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
        let map_colors_view = map_colors.create_view(&wgpu::TextureViewDescriptor::default());
        let cell_light_view = cell_light.create_view(&wgpu::TextureViewDescriptor::default());
        let layer = reflection_layer_texture(device, display_format, (1, 1));
        let submerged = reflection_layer_texture(device, display_format, (1, 1));
        let submerged_height = reflection_layer_texture(device, SUBMERGED_HEIGHT_FORMAT, (1, 1));
        let bindings = WaterBindings {
            layout: layout.clone(),
            uniform: uniform_buffer.clone(),
            floor: floor_view.clone(),
            map_colors: map_colors_view,
            cell_light: cell_light_view,
            ripple: match &ripples {
                Some(field) => [field.views()[0].clone(), field.views()[1].clone()],
                None => [still.clone(), still],
            },
            fft: fft_view,
            sampler: wave_sampler,
            fetch: fetch_view,
            layer: layer.create_view(&wgpu::TextureViewDescriptor::default()),
            submerged: submerged.create_view(&wgpu::TextureViewDescriptor::default()),
            submerged_height: submerged_height.create_view(&wgpu::TextureViewDescriptor::default()),
        };
        let bind_groups = bindings.groups(device);
        let pipelines = WaterPipelines::new(device, terrain, &layout, settings, display_format);
        let water = Self {
            pipelines,
            physical: settings.physical,
            uniform_buffer,
            bind_groups,
            bindings,
            layer: settings.physical.then_some(layer),
            category4: settings
                .physical
                .then(|| WaterCategory4::new(device, billboards, terrain.config.depth_format)),
            display_format,
            ripples,
            fft,
            shelter: shelter.map(|map| Shelter::new(fetch, map, coverage)),
            min_wavelength: settings.ripple_min_wavelength,
            gust_drift: (
                None,
                f64::from(settings.gust_speed),
                f64::from(settings.gust_scale.max(1e-3)),
                [0.0; 2],
            ),
            wind: settings.wind_ripples.then(|| {
                mandrake_view::water::wind::RippleWind::new(
                    settings.meters_per_cell,
                    settings.ripple_calm,
                    settings.ripple_wind_scale,
                )
            }),
            floor_texture,
            map_colors,
            cell_light,
            color_mode: settings.color_mode,
            color_radius: settings.color_radius,
            light_direction: settings.light_direction,
            specular: settings.specular,
            light_fit_min_r2: settings.light_fit_min_r2,
            light_fallback: settings.light_fallback,
            lowered: None,
            full: None,
            water_indices: None,
            full_media_hash: None,
            uniforms,
            wobble,
            mesh_flags: surface.mesh_flags,
        };
        water.write_floor(queue, (w, h), &info);
        water
    }

    pub fn update_terrain(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &mandrake_tag::MeshTag,
        terrain: &TerrainMesh,
    ) {
        let vertices = terrain.lowered_vertices(mesh);
        if terrain.is_plain_grid() {
            let hash = media_flags_hash(mesh, self.mesh_flags);
            if self.full.is_some() && self.full_media_hash == Some(hash) {
                self.update_floor(queue, mesh);
                self.upload_lowered_vertices(device, queue, &vertices);
                return;
            }
            let full = crate::mesh::landscape_triangles(mesh, None);
            self.set_lowered(device, queue, mesh, &vertices, Some(&full));
            self.full_media_hash = Some(hash);
            return;
        }
        self.full_media_hash = None;
        let (w, h, info) = self.update_floor_returning(queue, mesh);
        self.full = None;
        self.upload_lowered_vertices(device, queue, &vertices);
        self.update_detail_water_indices(device, queue, terrain, &vertices, (w, h), &info);
    }

    pub fn set_lowered(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &mandrake_tag::MeshTag,
        vertices: &[TerrainVertex],
        full: Option<&[([u32; 3], bool)]>,
    ) {
        let (w, h, info) = self.update_floor_returning(queue, mesh);
        self.water_indices = None;
        self.full_media_hash = None;
        match full {
            Some(tris) => {
                let mut indices: Vec<u32> = Vec::with_capacity(tris.len() * 3);
                for (tri, media) in tris {
                    if *media {
                        indices.extend_from_slice(tri);
                    }
                }
                let media = indices.len() as u32;
                let mut far_dry: Vec<u32> = Vec::new();
                for (tri, media) in tris {
                    if *media {
                        continue;
                    }
                    if triangle_near_water(*tri, vertices, (w, h), &info) {
                        indices.extend_from_slice(tri);
                    } else {
                        far_dry.extend_from_slice(tri);
                    }
                }
                let near = indices.len() as u32;
                indices.extend_from_slice(&far_dry);
                let total = indices.len() as u32;
                let bytes: &[u8] = bytemuck::cast_slice(&indices);
                let buffer = match self.full.take() {
                    Some(existing) if existing.indices.size() >= bytes.len() as u64 => {
                        if !bytes.is_empty() {
                            queue.write_buffer(&existing.indices, 0, bytes);
                        }
                        existing.indices
                    }
                    _ => {
                        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("Water Full Grid Indices"),
                            size: (indices.len().max(1) * 4) as u64,
                            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        if !bytes.is_empty() {
                            queue.write_buffer(&buffer, 0, bytes);
                        }
                        buffer
                    }
                };
                self.full = Some(FullGrid {
                    indices: buffer,
                    media,
                    near,
                    total,
                });
            }
            None => self.full = None,
        }
        self.upload_lowered_vertices(device, queue, vertices);
    }

    fn upload_lowered_vertices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[TerrainVertex],
    ) {
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        match &self.lowered {
            Some(buffer) if buffer.size() == bytes.len() as u64 => {
                queue.write_buffer(buffer, 0, bytes);
            }
            _ => {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Water Lowered Terrain"),
                    size: bytes.len().max(4) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if !bytes.is_empty() {
                    queue.write_buffer(&buffer, 0, bytes);
                }
                self.lowered = Some(buffer);
            }
        }
    }

    fn update_detail_water_indices(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainMesh,
        vertices: &[TerrainVertex],
        grid_size: (u32, u32),
        info: &[[f32; 4]],
    ) {
        let Some(drawn) = terrain.drawn_indices() else {
            self.water_indices = None;
            return;
        };
        let media_end = (terrain.num_media_indices as usize).min(drawn.len());
        let mut indices: Vec<u32> = Vec::with_capacity(media_end + (drawn.len() - media_end) / 4);
        indices.extend_from_slice(&drawn[..media_end]);
        let media = indices.len() as u32;
        for &tri in drawn[media_end..].as_chunks::<3>().0 {
            if triangle_near_water(tri, vertices, grid_size, info) {
                indices.extend_from_slice(&tri);
            }
        }
        let near = indices.len() as u32;
        let bytes: &[u8] = bytemuck::cast_slice(&indices);
        let buffer = match self.water_indices.take() {
            Some(existing) if existing.indices.size() >= bytes.len() as u64 => {
                if !bytes.is_empty() {
                    queue.write_buffer(&existing.indices, 0, bytes);
                }
                existing.indices
            }
            _ => {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Water Detail Near Indices"),
                    size: (indices.len().max(1) * 4) as u64,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if !bytes.is_empty() {
                    queue.write_buffer(&buffer, 0, bytes);
                }
                buffer
            }
        };
        self.water_indices = Some(WaterIndices {
            indices: buffer,
            media,
            near,
        });
    }

    pub fn set_landscape(
        &mut self,
        queue: &wgpu::Queue,
        mesh: &mandrake_tag::MeshTag,
        texels: &crate::landscape::LandscapeTexels,
        rgba: &[u8],
    ) {
        let size = (texels.width, texels.height);
        self.set_map_colors(queue, mesh, rgba, size);
        if let Some(light) = cell_light(mesh, texels) {
            let n = (light.len() / 4).max(1) as f32;
            let mean: Vec<f32> = (0..3)
                .map(|k| {
                    light
                        .iter()
                        .skip(k)
                        .step_by(4)
                        .map(|&v| f32::from(v))
                        .sum::<f32>()
                        / n
                        / 127.5
                })
                .collect();
            tracing::info!(
                "--water enhanced: mean light on the cells from the baked shading {mean:.3?}"
            );
            let (cw, ch) = (usize::from(mesh.cell_width), usize::from(mesh.cell_height));
            let wet =
                |c: usize, r: usize| mesh.cell_at(c, r).is_some_and(|v| v.flags & 0x1800 != 0);
            let (mut under, mut shore) = (([0.0f32; 3], 0.0f32), ([0.0f32; 3], 0.0f32));
            for r in 0..ch {
                for c in 0..cw {
                    let near = (r.saturating_sub(2)..(r + 3).min(ch))
                        .any(|y| (c.saturating_sub(2)..(c + 3).min(cw)).any(|x| wet(x, y)));
                    let slot = if wet(c, r) {
                        &mut under
                    } else if near {
                        &mut shore
                    } else {
                        continue;
                    };
                    for k in 0..3 {
                        slot.0[k] += f32::from(light[(r * cw + c) * 4 + k]) / 127.5;
                    }
                    slot.1 += 1.0;
                }
            }
            let avg = |(sum, n): ([f32; 3], f32)| sum.map(|v| v / n.max(1.0));
            tracing::info!(
                "--water enhanced: baked light under the water {:.3?} ({} cells), on the dry shore within 2 cells {:.3?} ({} cells)",
                avg(under),
                under.1,
                avg(shore),
                shore.1
            );
            write_cell_texture(queue, &self.cell_light, mesh, &light);
        }
        if self.light_direction.is_none() {
            let fit = fit_light(mesh, texels.shade(), size);
            let usable = fit.filter(|f| {
                f.r2 >= self.light_fit_min_r2
                    && f.spread >= FLAT_SHADING_SPREAD
                    && f.direction[1] > 0.0
            });
            let fallback = self
                .light_fallback
                .map(|(elevation, azimuth)| light_from_angles(elevation, azimuth));
            let verdict = match (usable, fallback) {
                (Some(_), _) => "the fitted direction".to_string(),
                (None, Some(d)) => format!(
                    "poor fit or flat shading: the shared direction [{:.3}, {:.3}, {:.3}] (light_fallback)",
                    d[0], d[1], d[2]
                ),
                (None, None) => "poor fit or flat shading, directional highlights off".to_string(),
            };
            match fit {
                Some(f) => {
                    let d = f.direction;
                    let elevation = d[1].clamp(-1.0, 1.0).asin().to_degrees();
                    let azimuth = d[2].atan2(-d[0]).to_degrees();
                    tracing::info!(
                        "--water enhanced: light fitted to the baked shading: render-space direction [{:.3}, {:.3}, {:.3}] (elevation {elevation:.1} deg, map azimuth {azimuth:.1} deg), R2 {:.3}, shading spread {:.1} levels, {} dry cells: {verdict}",
                        d[0],
                        d[1],
                        d[2],
                        f.r2,
                        f.spread,
                        f.cells,
                    );
                }
                None => {
                    tracing::info!("--water enhanced: no light fit (too few dry cells): {verdict}")
                }
            }
            match usable.map(|f| f.direction).or(fallback) {
                Some(d) => self.uniforms.light = [d[0], d[1], d[2], self.specular],
                None => self.uniforms.light[3] = 0.0,
            }
        }
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }

    fn set_map_colors(
        &mut self,
        queue: &wgpu::Queue,
        mesh: &mandrake_tag::MeshTag,
        rgba: &[u8],
        (width, height): (u32, u32),
    ) {
        if self.color_mode != WaterColorMode::Map {
            return;
        }
        let Some(colors) = map_water_colors(
            mesh,
            self.mesh_flags,
            rgba,
            (width, height),
            self.color_radius,
        ) else {
            return;
        };
        write_cell_texture(queue, &self.map_colors, mesh, &colors);
        self.uniforms.absorb[3] = 1.0;
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }

    pub fn lowered_buffer(&self) -> Option<&wgpu::Buffer> {
        self.lowered.as_ref()
    }

    pub fn update_floor(&self, queue: &wgpu::Queue, mesh: &mandrake_tag::MeshTag) {
        let (w, h, floor) = grid_info(mesh, self.mesh_flags);
        self.write_floor(queue, (w, h), &floor);
    }

    fn update_floor_returning(
        &self,
        queue: &wgpu::Queue,
        mesh: &mandrake_tag::MeshTag,
    ) -> (u32, u32, Vec<[f32; 4]>) {
        let (w, h, floor) = grid_info(mesh, self.mesh_flags);
        self.write_floor(queue, (w, h), &floor);
        (w, h, floor)
    }

    fn write_floor(&self, queue: &wgpu::Queue, (w, h): (u32, u32), floor: &[[f32; 4]]) {
        if (w, h) != (self.floor_texture.width(), self.floor_texture.height()) {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.floor_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(floor),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 16),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn update(&mut self, queue: &wgpu::Queue, local_time: i32) {
        let phase =
            (local_time.wrapping_mul(i32::from(self.wobble.frequency[2])) as u32 & 0xffff) as f32;
        if self.uniforms.phase[0] != phase {
            self.uniforms.phase[0] = phase;
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
        }
    }

    pub fn set_wind(&mut self, queue: &wgpu::Queue, seconds: f64, wind: Option<[i32; 3]>) {
        if let Some(ripple_wind) = self.wind.as_mut() {
            let (u, windiness, direction) = ripple_wind.update(wind);
            match ripple_wind.take_log() {
                Some((true, line)) => tracing::info!("{line}"),
                Some((false, line)) => tracing::debug!("{line}"),
                None => {}
            }
            if let Some(fft) = self.fft.as_mut() {
                fft.update(queue, seconds, u, windiness, direction);
            }
            let k_lo = f64::from(self.uniforms.fetch[3]);
            let k_hi = std::f64::consts::TAU / f64::from(self.min_wavelength.max(1e-3));
            self.uniforms.spectrum = [
                crate::water::fft::developed_peak(u) as f32,
                crate::water::fft::saturation(u, k_lo, k_hi) as f32,
                crate::water::fft::band(crate::water::fft::CASCADES - 1).1 as f32,
                k_hi as f32,
            ];
            self.uniforms.fetch[1] = u as f32;
            self.uniforms.glint[3] = (seconds % 3600.0) as f32;
            if let Some(shelter) = self.shelter.as_mut() {
                shelter.update(queue, direction, windiness);
            }
            let (last, speed, scale, offset) = &mut self.gust_drift;
            let dt = last.map_or(0.0, |t| (seconds - t).clamp(0.0, 0.5));
            *last = Some(seconds);
            let step = u * *speed * dt / *scale;
            for (o, d) in offset.iter_mut().zip([direction.cos(), direction.sin()]) {
                *o = (*o - d * step).rem_euclid(4096.0);
            }
            self.uniforms.gust[2] = offset[0] as f32;
            self.uniforms.gust[3] = offset[1] as f32;
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
        }
    }

    pub fn ensure_reflection_layer(&mut self, device: &wgpu::Device, (width, height): (u32, u32)) {
        let Some(layer) = &self.layer else {
            return;
        };
        if (layer.width(), layer.height()) == (width.max(1), height.max(1)) {
            return;
        }
        let layer = reflection_layer_texture(device, self.display_format, (width, height));
        self.bindings.layer = layer.create_view(&wgpu::TextureViewDescriptor::default());
        self.bindings.submerged =
            reflection_layer_texture(device, self.display_format, (width, height))
                .create_view(&wgpu::TextureViewDescriptor::default());
        self.bindings.submerged_height =
            reflection_layer_texture(device, SUBMERGED_HEIGHT_FORMAT, (width, height))
                .create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_groups = self.bindings.groups(device);
        self.layer = Some(layer);
    }

    pub fn reflection_layer(&self) -> Option<&wgpu::TextureView> {
        self.layer.as_ref().map(|_| &self.bindings.layer)
    }

    pub fn submerged_layer(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView)> {
        self.layer
            .as_ref()
            .map(|_| (&self.bindings.submerged, &self.bindings.submerged_height))
    }

    pub fn flush_waves(&mut self, queue: &wgpu::Queue) {
        if let Some(fft) = self.fft.as_mut() {
            fft.flush(queue);
        }
        if let Some(shelter) = self.shelter.as_mut() {
            shelter.receive(queue, true);
        }
    }

    pub fn step_ripples(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        dt: f32,
        disturbances: &[crate::water::ripples::Disturbance],
    ) {
        if let Some(field) = self.ripples.as_mut() {
            field.step(queue, encoder, dt, disturbances);
        }
    }

    pub fn has_ripples(&self) -> bool {
        self.ripples.is_some()
    }
}

fn media_flags_hash(mesh: &mandrake_tag::MeshTag, mesh_flags: u32) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mix = |h: &mut u64, v: u64| {
        *h ^= v;
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(&mut h, u64::from(mesh.cell_width));
    mix(&mut h, u64::from(mesh.cell_height));
    mix(&mut h, u64::from(mesh_flags));
    for cell in &mesh.cells {
        mix(
            &mut h,
            u64::from(cell.flags & 0x1900) | (u64::from(cell.model_index as u16) << 16),
        );
    }
    h
}

pub(super) fn vertex_near_water(v: &TerrainVertex, (w, h): (u32, u32), info: &[[f32; 4]]) -> bool {
    if w == 0 || h == 0 {
        return false;
    }
    let last_c = (w - 1) as i32;
    let last_r = (h - 1) as i32;
    let gx = (v.uv[0] * last_c as f32 * 2.0).round() * 0.5;
    let gy = (v.uv[1] * last_r as f32 * 2.0).round() * 0.5;
    let c0 = (gx.floor() as i32).clamp(0, last_c) as usize;
    let c1 = (gx.ceil() as i32).clamp(0, last_c) as usize;
    let r0 = (gy.floor() as i32).clamp(0, last_r) as usize;
    let r1 = (gy.ceil() as i32).clamp(0, last_r) as usize;
    let w = w as usize;
    info.get(r0 * w + c0).is_some_and(|p| p[2] > 0.5)
        || info.get(r0 * w + c1).is_some_and(|p| p[2] > 0.5)
        || info.get(r1 * w + c0).is_some_and(|p| p[2] > 0.5)
        || info.get(r1 * w + c1).is_some_and(|p| p[2] > 0.5)
}

pub(super) fn triangle_near_water(
    tri: [u32; 3],
    vertices: &[TerrainVertex],
    grid_size: (u32, u32),
    info: &[[f32; 4]],
) -> bool {
    tri.iter().any(|&i| {
        vertices
            .get(i as usize)
            .is_some_and(|v| vertex_near_water(v, grid_size, info))
    })
}

struct FullGrid {
    indices: wgpu::Buffer,
    media: u32,
    near: u32,
    total: u32,
}

struct WaterIndices {
    indices: wgpu::Buffer,
    media: u32,
    near: u32,
}
