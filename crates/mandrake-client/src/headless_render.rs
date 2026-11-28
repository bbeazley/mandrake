use crate::corpse;
use crate::headless::{
    HeadlessDrag, HeadlessInterface, HeadlessMarkers, UnitTickRecord, run_headless_drag,
    write_unit_draw_log,
};
use crate::hud;
use crate::interface_setup::world_frame;
use crate::landscape_edit::{apply_landscape_edit, apply_stain};
use crate::map_load::build_terrain_mesh;
use crate::media;
use crate::minimap;
use crate::model_instances::{
    animating_shadow_overlays, attach_model_media, follow_sim_model_animations,
    load_model_instances, log_region_culled_surfaces, tick_model_animations,
};
use crate::object_bounds;
use crate::particles;
use crate::projectile_local;
use crate::projfx_log;
use crate::region_view::{minimap_outline, view_bars};
use crate::render_passes::{
    BarTargets, PassRule, PassSkips, WorldPasses, WorldTargets, encode_bars,
};
use crate::scenery;
use crate::scenery_billboards::{
    ObjectShadow, SceneryPart, SceneryView, generate_connector_billboards,
    generate_scenery_billboards,
};
use crate::sprite_manager::ClientSpriteManager;
use crate::ui_scale;
use crate::unit_anim;
use crate::unit_anim::UnitAnimator;
use crate::world_assembly::{LocalOverlays, Myth2Gpu, Myth2View, Myth2World};
use mandrake_render::{
    BillboardRenderer, ModelRenderer, OverlayRenderer, RenderPipelineConfig, TerrainRenderer,
    TranslucentBatch,
};
use mandrake_sim::Simulation;
use mandrake_tag::collection::TerrainTexture;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_tag::{InterfaceColorTable, MeshConnectorList, MeshTag, TagArchive};
use mandrake_view::camera::Camera;
use mandrake_view::landscape::{LandscapeTexels, ShadeOverlay};
use std::path::Path;
use tracing::{debug, info};

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_headless_frame(
    mesh_tag: &MeshTag,
    connectors: &MeshConnectorList,
    (texture, detail): (
        Option<&TerrainTexture>,
        Option<&mandrake_render::detail::LandscapeDetail>,
    ),
    archive: &TagArchive,
    large_archive: Option<&TagArchive>,
    sim: &mut Simulation,
    width: u32,
    height: u32,
    camera: &Camera,
    output_path: &Path,
    ticks: u32,
    ticks_without_engine: u32,
    projfx_log: Option<projfx_log::ProjfxLog>,
    unit_states: &[UnitTickRecord],
    start_motions: unit_anim::UnitMotions,
    unit_draw_log: Option<&Path>,
    fraction: i32,
    interface_colors: Option<&InterfaceColorTable>,
    scenery_objects: &scenery::SceneryObjects,
    band_drag: Option<HeadlessDrag>,
    engine_idle: Option<unit_anim::EngineIdleModel>,
    interface: HeadlessInterface,
    hud: Option<&hud::Hud>,
    minimap: Option<&minimap::Minimap>,
    translucent_bars: bool,
    markers: HeadlessMarkers<'_>,
    (ui, ui_filter): (ui_scale::UiScale, ui_scale::UiFilter),
    (sampling, sprite_filter, fog, reflection_soft_edge, water_settings): (
        ui_scale::Sampling,
        Option<ui_scale::UiFilter>,
        &crate::fog::MapFog,
        bool,
        Option<&mandrake_view::water::WaterSettings>,
    ),
    particle_field: Option<&particles::ParticleField>,
    local_projectiles: Option<&projectile_local::LocalProjectiles>,
    landscape_edits: &[corpse::LandscapeEdit],
    mesh_ripple: &crate::mesh_ripple::MeshRipple,
    ten_bit: bool,
) -> anyhow::Result<()> {
    let camera = &crate::screenshot::frame_camera(camera);
    pollster::block_on(async {
        let (device, queue) = headless_device("Headless Device").await?;

        let config = RenderPipelineConfig {
            width,
            height,
            target_format: if ten_bit {
                mandrake_render::CANVAS_FORMAT
            } else {
                wgpu::TextureFormat::Rgba8UnormSrgb
            },
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        };

        let mut terrain_renderer = TerrainRenderer::new(&device, &queue, &config);
        terrain_renderer.set_detail(&device, &queue, detail);
        let cell_size = 1.0f32;

        let model_renderer = ModelRenderer::new(&device, &queue, &config);
        let (mut model_meshes, mut model_instances) = load_model_instances(
            &device,
            &queue,
            &model_renderer,
            archive,
            large_archive,
            mesh_tag,
        );
        let mut billboard_renderer = BillboardRenderer::new(&device, &queue, &config);
        let mut sprite_mgr = ClientSpriteManager::build(
            &device,
            &queue,
            &billboard_renderer.texture_bind_group_layout,
            archive,
            large_archive,
            mesh_tag,
            connectors,
            markers.collection,
            Some(&corpse::EngineDeathGroups::of(&sim.engine)),
            object_bounds::mons_postprocess_of(sim),
            &scenery_objects.team_colours,
        );
        sprite_mgr.projfx_log = projfx_log;
        sprite_mgr.interface_colors = interface_colors.cloned();
        sprite_mgr.scenery_objects = scenery_objects.clone();
        {
            let archives: Vec<&TagArchive> = large_archive
                .into_iter()
                .chain(std::iter::once(archive))
                .collect();
            attach_model_media(
                &device,
                &model_renderer,
                &mut model_instances,
                &model_meshes,
                &archives,
                &mut sprite_mgr.scenery_objects.media,
            );
        }
        follow_sim_model_animations(
            &mut model_instances,
            sim,
            &mut sprite_mgr.scenery_objects.media,
        );
        for _ in 0..ticks_without_engine {
            tick_model_animations(&mut model_instances);
        }
        let _ = crate::model_effects::follow_sim_model_effects(
            &mut model_instances,
            &mut model_meshes,
            sim,
            &mut crate::model_effects::EffectMeshes {
                device: &device,
                queue: &queue,
                renderer: &model_renderer,
                source: crate::model_instances::ModelTagSource {
                    small: archive,
                    large: large_archive,
                },
                cache: &mut crate::model_effects::EffectMeshCache::default(),
            },
            &mut sprite_mgr.scenery_objects.media,
        );
        let mut terrain_mesh = build_terrain_mesh(
            &device,
            mesh_ripple.mesh(mesh_tag),
            cell_size,
            sprite_mgr.scenery_objects.media.as_deref(),
            detail,
            mesh_ripple.lod(),
        );
        info!(
            "Media surface: {} animated vertices",
            terrain_mesh.media_vertices().len()
        );
        let mut media_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Headless Media Update Encoder"),
        });
        terrain_mesh.update_media(
            &device,
            &queue,
            &mut media_encoder,
            media::local_time(ticks),
            &mandrake_math::engine::trig_tables().sine,
        );
        queue.submit(std::iter::once(media_encoder.finish()));
        let mut water = match (water_settings, sprite_mgr.scenery_objects.media.as_deref()) {
            (Some(settings), Some(map)) => {
                let mut w = mandrake_render::water::WaterShading::new(
                    &device,
                    &queue,
                    &terrain_renderer,
                    mesh_ripple.mesh(mesh_tag),
                    cell_size,
                    map.surface,
                    settings,
                    billboard_renderer.interface_format,
                    &billboard_renderer,
                );
                w.update(&queue, media::local_time(ticks));
                w.update_terrain(&device, &queue, mesh_ripple.mesh(mesh_tag), &terrain_mesh);
                Some(w)
            }
            _ => None,
        };

        let mut scenery_animator = scenery::SceneryAnimator::new(
            mesh_tag,
            &sprite_mgr.scenery_objects,
            &sprite_mgr.scenery_visuals,
        );
        for _ in 0..ticks {
            scenery_animator.tick(&sprite_mgr.scenery_visuals);
        }
        let (mut all_billboards, _, object_shadows) = generate_scenery_billboards(
            mesh_tag,
            &sprite_mgr,
            media::local_time(ticks),
            SceneryView::new(mesh_tag, camera.eye_position(), &scenery_animator, fraction),
            SceneryPart::All,
        );
        all_billboards.extend(generate_connector_billboards(
            mesh_tag,
            connectors,
            &sprite_mgr,
        ));
        let mut unit_animator = UnitAnimator::with_engine_idle(engine_idle);
        unit_animator.seed_motions(start_motions);
        let mut draw_log = unit_draw_log
            .map(|p| std::fs::File::create(p).map(std::io::BufWriter::new))
            .transpose()?;
        for (t, tick) in unit_states.iter().enumerate() {
            unit_animator.replay_tick(tick.engine.clone(), tick.motions.clone());
            unit_animator.tick(tick.states.iter().copied(), &sprite_mgr.unit_visuals);
            if let Some(out) = draw_log.as_mut() {
                write_unit_draw_log(
                    out,
                    t as u32 + 1,
                    sim,
                    (camera, (width, height)),
                    &unit_animator,
                    &sprite_mgr,
                )?;
            }
        }
        unit_animator.set_fraction(fraction);
        let band = band_drag.and_then(|drag| {
            let (bounds, ids) = run_headless_drag(
                drag,
                sim,
                mesh_tag,
                camera,
                (width, height),
                &sprite_mgr,
                &unit_animator,
                (view_bars(&ui), media::local_time(ticks)),
            );
            for unit in sim.units.values_mut() {
                unit.selected = ids.contains(&unit.id);
            }
            info!(
                "Band drag {:?} -> {:?}: band {bounds:?}, {} unit(s) selected",
                drag.from,
                drag.to,
                ids.len()
            );
            bounds
        });
        sprite_mgr.update_team_cache(
            &device,
            &queue,
            &billboard_renderer.texture_bind_group_layout,
            sim,
        );
        let iface = interface.view(sim, camera, (width, height), &sprite_mgr, &unit_animator);
        let world = crate::world_view::myth2::drawn_world(sim, &unit_animator, &sprite_mgr);
        let bed = match (
            texture,
            water.as_ref(),
            water_settings,
            sprite_mgr.scenery_objects.media.as_deref(),
        ) {
            (Some(tex), Some(_), Some(settings), Some(map)) if settings.river_bed => {
                crate::water_bed::repainted_texture(tex, mesh_tag, &map.coverage)
            }
            _ => None,
        };
        let mut category2_batch = TranslucentBatch::new(&device);
        let mut reflection_batch = mandrake_render::ReflectionBatch::new(&device);
        let mut translucent_batch = TranslucentBatch::new(&device);
        let reflection_edge = reflection_soft_edge.then(|| {
            mandrake_render::ReflectionEdge::new(
                &device,
                &RenderPipelineConfig {
                    width,
                    height,
                    target_format: wgpu::TextureFormat::R8Unorm,
                    depth_format: Some(wgpu::TextureFormat::Depth32Float),
                },
            )
        });
        let mut gpu = Myth2Gpu {
            device: &device,
            queue: &queue,
            terrain_renderer: &terrain_renderer,
            terrain_mesh: &terrain_mesh,
            model_renderer: &model_renderer,
            billboard_renderer: &mut billboard_renderer,
            sprite_mgr: &sprite_mgr,
            model_instances: &model_instances,
            model_meshes: &model_meshes,
            category2_batch: &mut category2_batch,
            reflection_batch: &mut reflection_batch,
            translucent_batch: &mut translucent_batch,
            reflection_edge: reflection_edge.as_ref(),
        };
        let mut terrain_tex_bg = None;
        let scene = Myth2World::prepare(
            &Myth2View {
                mesh_tag,
                sim,
                world: &world,
                camera,
                size: (width, height),
                bars: view_bars(&ui),
                animator: &unit_animator,
                interface: &iface,
                local_time: media::local_time(ticks),
                band,
                gesture: None,
                markers: markers.sprites,
                particles: particle_field,
                local_projectiles,
                local_overlays: LocalOverlays::Whole,
                sampling,
                sprite_filter,
                fog: (fog, fog.seconds_now()),
                lighting: None,
            },
            &mut gpu,
            (all_billboards, object_shadows),
            &mut LocalRandom::new(0),
            &mut None,
            |object_shadows| {
                terrain_tex_bg = texture.map(|original| {
                    let tex = bed.as_ref().unwrap_or(original);
                    let mut texels = LandscapeTexels::new(tex, &mesh_tag.landscape_shading);
                    let mut stain_random = LocalRandom::new(0);
                    match &sprite_mgr.projfx_log {
                        Some(trace) => {
                            for s in trace.stains_before(sim.current_tick) {
                                apply_stain(
                                    &sprite_mgr,
                                    &mut texels,
                                    (s.collection, s.sequence),
                                    s.point,
                                    s.random,
                                    s.mirror,
                                );
                            }
                        }
                        None => {
                            for edit in landscape_edits {
                                apply_landscape_edit(
                                    &sprite_mgr,
                                    &mut texels,
                                    edit,
                                    &mut stain_random,
                                );
                            }
                        }
                    }
                    let overlays: Vec<ShadeOverlay<'_>> = object_shadows
                        .iter()
                        .map(ObjectShadow::overlay)
                        .chain(animating_shadow_overlays(&model_instances))
                        .collect();
                    let rgba = texels.rgba(&overlays);
                    if let Some(water) = water.as_mut() {
                        let painted = bed.as_ref().map(|_| {
                            crate::water_bed::painted_rgba(&rgba, &texels, &original.color_index)
                        });
                        water.set_landscape(
                            &queue,
                            mesh_tag,
                            &texels,
                            painted.as_deref().unwrap_or(&rgba),
                        );
                    }
                    terrain_renderer
                        .create_landscape_texture(
                            &device,
                            &queue,
                            texels.width,
                            texels.height,
                            &rgba,
                        )
                        .bind_group
                });
            },
        );
        let (region_view, view_region) = (scene.region_view.as_ref(), scene.view_region.as_ref());
        if let Some(r) = view_region {
            debug!(
                "View region rows {}..{}: {:?}",
                r.first_row, r.last_row, r.spans
            );
            log_region_culled_surfaces(
                &model_instances,
                &model_meshes,
                camera,
                width as f32 / height as f32,
                mesh_tag,
                r,
            );
        }
        let target = Offscreen::new(
            &device,
            (width, height),
            gpu.billboard_renderer.interface_format,
        );
        if let Some(frames) = std::env::var("M2_WATER_TIMING")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|_| device.features().contains(water_timing_features()))
        {
            let set = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("Water Timing"),
                ty: wgpu::QueryType::Timestamp,
                count: 3,
            });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Water Timing Resolve"),
                size: 24,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Water Timing Read"),
                size: 24,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let period = f64::from(queue.get_timestamp_period()) / 1.0e6;
            let (mut steps, mut worlds, mut cpu) = (Vec::new(), Vec::new(), Vec::new());
            let turn = std::env::var("M2_WATER_TIMING_TURN")
                .ok()
                .and_then(|v| v.parse::<f64>().ok());
            for frame_index in 0..frames {
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.write_timestamp(&set, 0);
                let started = std::time::Instant::now();
                if let Some(water) = water.as_mut() {
                    water.ensure_reflection_layer(&device, (width, height));
                    let seconds = f64::from(frame_index) / 60.0;
                    let wind = (sim.engine.wind.as_ref()).map(|w| w.velocity);
                    let wind = match turn {
                        Some(d) => {
                            let [x, y, z] = wind.unwrap_or([1 << 15, 0, 0]);
                            let (s, c) = (f64::from(frame_index) * d).to_radians().sin_cos();
                            let (x, y) = (f64::from(x), f64::from(y));
                            Some([(x * c - y * s) as i32, (x * s + y * c) as i32, z])
                        }
                        None => wind,
                    };
                    water.set_wind(&queue, seconds, wind);
                    water.step_ripples(&queue, &mut encoder, 1.0 / 60.0, &[]);
                }
                cpu.push(started.elapsed().as_secs_f64() * 1000.0);
                encoder.write_timestamp(&set, 1);
                scene.encode(
                    &gpu,
                    &mut encoder,
                    &target.views(),
                    terrain_tex_bg.as_ref(),
                    water.as_ref(),
                    WorldPasses {
                        rule: PassRule::Myth2,
                        clear: wgpu::Color::BLACK,
                        skip: PassSkips::default(),
                    },
                    None,
                );
                encoder.write_timestamp(&set, 2);
                encoder.resolve_query_set(&set, 0..3, &resolve, 0);
                encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, 24);
                queue.submit([encoder.finish()]);
                read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                let t: Vec<u64> = read
                    .slice(..)
                    .get_mapped_range()?
                    .chunks(8)
                    .map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()))
                    .collect();
                read.unmap();
                if frame_index >= 10 {
                    steps.push(t[1].saturating_sub(t[0]) as f64 * period);
                    worlds.push(t[2].saturating_sub(t[1]) as f64 * period);
                }
            }
            let median = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v.get(v.len() / 2).copied().unwrap_or(0.0)
            };
            eprintln!(
                "water timing {width}x{height}, {} frames: water steps {:.4} ms, world {:.4} ms (GPU medians), water CPU {:.4} ms (max {:.4})",
                worlds.len(),
                median(&mut steps),
                median(&mut worlds),
                median(&mut cpu),
                cpu.last().copied().unwrap_or(0.0)
            );
        }
        if let Some(water) = water.as_mut() {
            water.ensure_reflection_layer(&device, (width, height));
            let wind = (sim.engine.wind.as_ref()).map(|w| w.velocity);
            water.set_wind(&queue, f64::from(ticks) / 30.0, wind);
            water.flush_waves(&queue);
            water.set_wind(&queue, f64::from(ticks) / 30.0, wind);
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Offscreen Render Encoder"),
        });
        scene.encode(
            &gpu,
            &mut encoder,
            &target.views(),
            terrain_tex_bg.as_ref(),
            water.as_ref(),
            WorldPasses {
                rule: PassRule::Myth2,
                clear: wgpu::Color {
                    r: 0.1,
                    g: 0.15,
                    b: 0.22,
                    a: 1.0,
                },
                skip: PassSkips::default(),
            },
            None,
        );
        let interface_view = &target.interface;

        if let Some(hud) = hud {
            let mut overlay = OverlayRenderer::new(&device, billboard_renderer.interface_format);
            let mut images = hud::HudImages::default();
            images.update(&overlay, &device, &queue, hud);
            let mut map_images = minimap::MinimapImages::default();
            let map_frame = minimap.and_then(|mm| {
                let outline = minimap_outline(
                    camera,
                    mesh_tag,
                    sprite_mgr.scenery_objects.media.as_deref(),
                    region_view.zip(view_region),
                );
                info!(
                    "Overhead map outline: from {:?} to {:?} polygon {:?}",
                    outline.from, outline.to, outline.polygon
                );
                if let Ok(dir) = std::env::var("MYTH2_MINIMAP_DUMP") {
                    let lines: Vec<String> = mm
                        .map
                        .outline_segments(&outline)
                        .iter()
                        .map(|(a, b)| format!("L p0 {},{} p1 {},{}", a.0, a.1, b.0, b.1))
                        .collect();
                    let _ = std::fs::write(
                        Path::new(&dir).join(format!("outline.t{ticks}.txt")),
                        lines.join("\n") + "\n",
                    );
                }
                map_images.update(&overlay, &device, &queue, &mm.map);
                mm.map
                    .draws(&outline, world_frame(ui.logical.0, ui.logical.1).1)
            });
            let mut draws = map_frame
                .as_ref()
                .map(|f| map_images.draws(f))
                .unwrap_or_default();
            draws.extend(images.console_draws());
            draws.extend(images.score_draws());
            draws.extend(images.key_binding_draws());
            draws.extend(images.draws(hud, translucent_bars));
            let mut scaler = mandrake_render::InterfaceScaler::new(&device, overlay.format);
            encode_bars(
                BarTargets {
                    device: &device,
                    queue: &queue,
                    overlay: &mut overlay,
                    scaler: &mut scaler,
                    view: interface_view,
                    window: (width, height),
                },
                &mut encoder,
                (&ui, ui_filter.into()),
                &draws,
                &mut None,
            );
        }

        let unpadded = target.read(&device, &queue, encoder)?;

        if ten_bit {
            let rgba = crate::screenshot::canvas_to_ten_bit_rgba16(&unpadded);
            image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(width, height, rgba)
                .ok_or_else(|| anyhow::anyhow!("a {width}x{height} frame's pixels"))?
                .save(output_path)?;
        } else {
            image::save_buffer(
                output_path,
                &unpadded,
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )?;
        }

        Ok(())
    })
}

fn water_timing_features() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
}

pub(crate) async fn headless_device(label: &str) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to find suitable GPU adapter: {e}"))?;
    Ok(adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some(label),
            required_features: if std::env::var_os("M2_WATER_TIMING").is_some() {
                adapter.features() & water_timing_features()
            } else {
                wgpu::Features::empty()
            },
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        })
        .await?)
}

pub(crate) struct Offscreen {
    texture: wgpu::Texture,
    pub(crate) color: wgpu::TextureView,
    pub(crate) interface: wgpu::TextureView,
    pub(crate) depth: wgpu::TextureView,
}

impl Offscreen {
    pub(crate) fn new(
        device: &wgpu::Device,
        (width, height): (u32, u32),
        interface_format: wgpu::TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Offscreen Color Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if interface_format == mandrake_render::CANVAS_FORMAT {
                interface_format
            } else {
                wgpu::TextureFormat::Rgba8UnormSrgb
            },
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[interface_format],
        });
        let color = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let interface = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(interface_format),
            ..Default::default()
        });
        let depth = crate::render_passes::create_depth_view(device, width, height);
        Self {
            texture,
            color,
            interface,
            depth,
        }
    }

    pub(crate) fn views(&self) -> WorldTargets<'_> {
        WorldTargets {
            color: &self.color,
            interface: &self.interface,
            depth: &self.depth,
        }
    }

    pub(crate) fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
    ) -> anyhow::Result<Vec<u8>> {
        let (width, height) = (self.texture.width(), self.texture.height());
        let pixel = self.texture.format().block_copy_size(None).unwrap_or(4);
        let bytes_per_row = (width * pixel).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Staging Buffer"),
            size: u64::from(bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(std::iter::once(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv()??;
        let data = slice.get_mapped_range()?;
        let mut pixels = Vec::with_capacity((width * height * pixel) as usize);
        for row in data.chunks_exact(bytes_per_row as usize) {
            pixels.extend_from_slice(&row[..(width * pixel) as usize]);
        }
        drop(data);
        staging.unmap();
        Ok(pixels)
    }
}
