use crate::app::frame_budget::{self, GpuTimer};
use crate::app::shell::{ShellGpu, WindowFacts};
use crate::app::{ClientApp, GpuWindowState};
use crate::map_load::build_terrain_mesh;
use crate::model_instances::{
    attach_model_media, follow_sim_model_animations, load_model_instances,
};
use crate::render_passes::create_depth_view;
use crate::scenery_billboards::{
    ObjectShadow, SceneryPart, SceneryView, generate_connector_billboards,
    generate_scenery_billboards,
};
use crate::sprite_manager::ClientSpriteManager;
use crate::{corpse, hud, media, minimap, object_bounds, scenery, window_surface};
use mandrake_render::{
    BillboardRenderer, ModelRenderer, OverlayRenderer, RenderPipelineConfig, TerrainRenderer,
    TranslucentBatch,
};
use mandrake_tag::TagArchive;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::landscape::{LandscapeTexels, ShadeOverlay};
use std::sync::Arc;
use tracing::info;
use winit::window::Window;

pub(crate) struct MapRenderers {
    terrain: TerrainRenderer,
    models: ModelRenderer,
    billboards: BillboardRenderer,
    overlay: OverlayRenderer,
    scaler: mandrake_render::InterfaceScaler,
    screen_fade: mandrake_render::ScreenFade,
    reflection_edge: std::cell::OnceCell<mandrake_render::ReflectionEdge>,
}

impl MapRenderers {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RenderPipelineConfig,
        interface_format: wgpu::TextureFormat,
    ) -> Self {
        let overlay = OverlayRenderer::new(device, interface_format);
        Self {
            terrain: TerrainRenderer::new(device, queue, config),
            models: ModelRenderer::with_reflection_format(device, queue, config, interface_format),
            billboards: BillboardRenderer::with_interface_format(
                device,
                queue,
                config,
                interface_format,
            ),
            scaler: mandrake_render::InterfaceScaler::new(device, overlay.format),
            screen_fade: mandrake_render::ScreenFade::new(device, interface_format),
            overlay,
            reflection_edge: std::cell::OnceCell::new(),
        }
    }
}

impl ClientApp {
    pub(crate) fn attach_window(&mut self, window: Arc<Window>, facts: WindowFacts) {
        let size = window.inner_size();
        self.window = Some(window);
        self.occluded = facts.occluded;
        self.focused = facts.focused;
        self.cursor_in_window = facts.cursor_in_window;
        self.current_cursor_pos = facts.cursor;
        self.sync_min_window();
        let width = size.width.max(1);
        let height = size.height.max(1);
        if (width, height) != (self.width, self.height) {
            self.set_screen_size(width, height);
        }
    }

    pub(crate) fn has_gpu_state(&self) -> bool {
        self.gpu_state.is_some()
    }

    pub(crate) fn attach_gpu(&mut self, shell: &ShellGpu) {
        let device = shell.device.clone();
        let queue = shell.queue.clone();
        let surface_config = shell.surface_config.clone();
        let (width, height) = (surface_config.width, surface_config.height);

        let gpu_timer = (self.frame_budget.is_some()
            && device.features().contains(frame_budget::timer_features()))
        .then(|| GpuTimer::new(&device, &queue));
        if let Some(budget) = &mut self.frame_budget {
            budget.surface = format!(
                "{}, soft edge {}, sampling {:?}",
                shell.budget_surface, self.reflection_soft_edge, self.sampling,
            );
            budget.gpu_resized(width, height);
        }

        let depth_view = create_depth_view(&device, width, height);

        let formats = crate::window_surface::SurfaceFormats::of(&surface_config);
        let render_config = RenderPipelineConfig {
            width,
            height,
            target_format: formats.scene,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        };

        let interface_format = formats.interface;
        let shared = shell
            .renderers
            .get_or_init(|| MapRenderers::new(&device, &queue, &render_config, interface_format));

        let reflection_edge = self.reflection_soft_edge.then(|| {
            shared
                .reflection_edge
                .get_or_init(|| mandrake_render::ReflectionEdge::new(&device, &render_config))
                .sharing_pipelines(&device, &render_config)
        });

        let mut terrain_renderer =
            shared
                .terrain
                .sharing_pipelines(&device, &queue, &render_config);
        terrain_renderer.set_detail(&device, &queue, self.landscape_detail.as_deref());
        let cell_size = 1.0f32;

        let model_renderer = shared.models.sharing_pipelines(&device, &queue);
        let translucent_batch = TranslucentBatch::new(&device);
        let overlay = shared.overlay.sharing_pipelines(&device);
        let category2_batch = TranslucentBatch::new(&device);
        let reflection_batch = mandrake_render::ReflectionBatch::new(&device);
        let (mut model_meshes, mut model_instances) = load_model_instances(
            &device,
            &queue,
            &model_renderer,
            &self.archive,
            self.large_archive.as_ref(),
            &self.mesh_tag,
        );

        let billboard_renderer = shared.billboards.sharing_pipelines(&device, &queue);
        let mut sprite_mgr = ClientSpriteManager::build(
            &device,
            &queue,
            &billboard_renderer.texture_bind_group_layout,
            &self.archive,
            self.large_archive.as_ref(),
            &self.mesh_tag,
            &self.connectors,
            self.interface_collection.as_ref(),
            Some(&corpse::EngineDeathGroups::of(&self.sim.engine)),
            object_bounds::mons_postprocess_of(&self.sim),
            &self.scenery_objects.team_colours,
        );
        sprite_mgr.interface_colors = self.interface_colors.clone();
        sprite_mgr.scenery_objects = self.scenery_objects.clone();
        {
            let archives: Vec<&TagArchive> = self
                .large_archive
                .iter()
                .chain(std::iter::once(&self.archive))
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
            &self.sim,
            &mut sprite_mgr.scenery_objects.media,
        );
        let mut model_effect_meshes = crate::model_effects::EffectMeshCache::default();
        let _ = crate::model_effects::follow_sim_model_effects(
            &mut model_instances,
            &mut model_meshes,
            &self.sim,
            &mut crate::model_effects::EffectMeshes {
                device: &device,
                queue: &queue,
                renderer: &model_renderer,
                source: crate::model_instances::ModelTagSource {
                    small: &self.archive,
                    large: self.large_archive.as_ref(),
                },
                cache: &mut model_effect_meshes,
            },
            &mut sprite_mgr.scenery_objects.media,
        );
        let terrain_mesh = build_terrain_mesh(
            &device,
            &self.mesh_tag,
            cell_size,
            sprite_mgr.scenery_objects.media.as_deref(),
            self.landscape_detail.as_deref(),
            None,
        );
        let mut water = match (&self.water, sprite_mgr.scenery_objects.media.as_deref()) {
            (Some(settings), Some(map)) => {
                let mut water = mandrake_render::water::WaterShading::new(
                    &device,
                    &queue,
                    &terrain_renderer,
                    &self.mesh_tag,
                    cell_size,
                    map.surface,
                    settings,
                    billboard_renderer.interface_format,
                    &billboard_renderer,
                );
                water.update_terrain(&device, &queue, &self.mesh_tag, &terrain_mesh);
                Some(water)
            }
            _ => None,
        };
        let mesh_ripple = crate::mesh_ripple::MeshRipple::new(
            sprite_mgr
                .scenery_objects
                .media
                .as_deref()
                .and_then(|m| m.lod.as_ref()),
        );
        let scenery_animator = scenery::SceneryAnimator::new(
            &self.mesh_tag,
            &sprite_mgr.scenery_objects,
            &sprite_mgr.scenery_visuals,
        );
        info!(
            "{} animating scenery objects (scen flag 0x40)",
            scenery_animator.count()
        );
        let (mut scenery_billboards, scenery_ripples, static_shadows) = generate_scenery_billboards(
            &self.mesh_tag,
            &sprite_mgr,
            media::local_time(self.sim.current_tick),
            SceneryView::new(
                &self.mesh_tag,
                self.render_camera.eye_position(),
                &scenery_animator,
                0,
            ),
            SceneryPart::Static,
        );
        let bed = match (
            self.terrain_texture.as_ref(),
            water.as_ref(),
            self.water.as_ref(),
            sprite_mgr.scenery_objects.media.as_deref(),
        ) {
            (Some(tex), Some(_), Some(settings), Some(map)) if settings.river_bed => {
                crate::water_bed::repainted_texture(tex, &self.mesh_tag, &map.coverage)
            }
            _ => None,
        };
        let landscape = self.terrain_texture.as_ref().map(|original| {
            let tex = bed.as_ref().unwrap_or(original);
            let mut texels = LandscapeTexels::new(tex, &self.mesh_tag.landscape_shading);
            texels.prepare_stains();
            let overlays: Vec<ShadeOverlay<'_>> =
                static_shadows.iter().map(ObjectShadow::overlay).collect();
            let rgba = texels.rgba(&overlays);
            if let Some(water) = water.as_mut() {
                let painted = bed
                    .as_ref()
                    .map(|_| crate::water_bed::painted_rgba(&rgba, &texels, &original.color_index));
                water.set_landscape(
                    &queue,
                    &self.mesh_tag,
                    &texels,
                    painted.as_deref().unwrap_or(&rgba),
                );
            }
            let texture = terrain_renderer.create_landscape_texture(
                &device,
                &queue,
                texels.width,
                texels.height,
                &rgba,
            );
            (texels, texture)
        });
        scenery_billboards.extend(generate_connector_billboards(
            &self.mesh_tag,
            &self.connectors,
            &sprite_mgr,
        ));

        let interface_scaler = shared.scaler.sharing_pipelines(&device);
        let dialog_scaler = shared.scaler.sharing_pipelines(&device);
        let screen_fade = shared.screen_fade.clone();
        let static_lights = crate::lighting::extract_static_torch_lights(
            &static_shadows,
            &self.sim,
            &self.mesh_tag,
        );
        self.gpu_state = Some(GpuWindowState {
            device,
            queue,
            surface_config,
            surface_timeouts: window_surface::SurfaceTimeoutLog::default(),
            terrain_renderer,
            terrain_mesh,
            mesh_ripple,
            media_time: None,
            landscape,
            static_shadows,
            static_lights,
            moving_shadows: Vec::new(),
            landscape_edits_applied: 0,
            stain_random: LocalRandom::new(0),
            depth_texture_view: depth_view,
            reflection_edge,
            water,
            ripple_sources: Default::default(),
            ripple_clock: None,
            model_renderer,
            model_meshes,
            model_effect_meshes,
            model_instances,
            translucent_batch,
            category2_batch,
            reflection_batch,
            billboard_renderer,
            sprite_mgr,
            team_cache_tick: None,
            scenery_billboards,
            billboard_scratch: Vec::new(),
            scenery_ripples,
            scenery_animator,
            scenery_shadow_rects: Vec::new(),
            overlay,
            hud_images: hud::HudImages::default(),
            detach_menu_images: super::detach::MenuImages::default(),
            minimap_images: minimap::MinimapImages::default(),
            interface_scaler,
            dialog_scaler,
            gpu_timer,
            screen_fade,
        });
        self.load_model_animation_sounds();
    }
}
