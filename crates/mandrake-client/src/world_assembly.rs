use crate::app::frame_budget::{FrameClock, GpuTimer, Lap, lap};
use crate::effects::{
    lightning_billboards, lightning_overlays, local_projectile_billboards_from_surfaces,
    local_projectile_overlays_from_surfaces, particle_billboards,
};
use crate::effects::{lightning_surfaces, local_surfaces_from};
use crate::fog::MapFog;
use crate::interface_state::InterfaceView;
use crate::lightning_draw::LightningSurfaces;
use crate::model_instances::ModelInstance;
use crate::model_instances::{
    ModelMediaDraws, ModelMeshKey, opaque_model_draws, prepare_model_media,
    prepare_translucent_models, translucent_model_instances,
};
use crate::move_marker::LocalSprites;
use crate::overlays::{MoveGesture, band_rectangle_overlays, facing_gesture_arrows};
use crate::particles::ParticleField;
use crate::projectile_local::LocalProjectiles;
use crate::projectile_local::LocalSurface;
use crate::region_view::frame_view_parts;
use crate::region_view::{mark_region_models, retain_region_objects};
use crate::render_passes::{
    ModelDraw, SceneLighting, SpriteAtlases, WorldDraws, WorldFiltering, WorldLook, WorldPasses,
    WorldTargets, encode_world, finish_sprites, world_uniforms,
};
use crate::scenery_billboards::ObjectShadow;
use crate::sprite_manager::ClientSpriteManager;
use crate::ui_scale::{Sampling, UiFilter};
use crate::unit_anim::UnitAnimator;
use crate::unit_billboards::generate_unit_billboards;
use crate::world_view::{ClientRules, WorldView};
use mandrake_render::water::WaterShading;
use mandrake_render::{
    BillboardRenderer, ModelMesh, ModelRenderer, ReflectionBatch, ReflectionEdge, TerrainMesh,
    TerrainRenderer, TranslucentBatch,
};
use mandrake_sim::Simulation;
use mandrake_tag::MeshTag;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::view_region::{RegionView, ViewRegion};
use mandrake_view::{
    billboard::{BillboardInstance, SpritePassOrder},
    camera::Camera,
    selection,
};
use std::collections::BTreeMap;

struct Myth2Frame<'a> {
    v: &'a Myth2View<'a>,
    sprite_mgr: &'a ClientSpriteManager,
    region: (Option<&'a RegionView>, Option<&'a ViewRegion>),
    lightning: &'a LightningSurfaces,
    local_overlays: &'a [LocalSurface],
    local_sprites: &'a [LocalSurface],
    models: &'a [ModelInstance],
}

#[derive(Default)]
pub(crate) struct WorldLists {
    pub(crate) objects: Vec<BillboardInstance>,
    pub(crate) local_projectiles: Vec<BillboardInstance>,
    pub(crate) lightning: Vec<BillboardInstance>,
    pub(crate) mesh_effects: Vec<BillboardInstance>,
    pub(crate) particles: Vec<BillboardInstance>,
    pub(crate) overlays: Vec<BillboardInstance>,
}

pub(crate) fn assemble_world(
    rules: &ClientRules,
    camera: &Camera,
    lists: WorldLists,
) -> (Vec<BillboardInstance>, SpritePassOrder) {
    let WorldLists {
        objects: mut billboards,
        local_projectiles,
        lightning,
        mesh_effects,
        particles,
        overlays,
    } = lists;
    let (first, second) = if rules.lightning_first {
        (lightning, local_projectiles)
    } else {
        (local_projectiles, lightning)
    };
    for list in [first, second, mesh_effects, particles, overlays] {
        billboards.extend(list);
    }
    let order = finish_sprites(&mut billboards, camera);
    (billboards, order)
}

fn myth2_sprites(
    f: &Myth2Frame<'_>,
    mut objects: Vec<BillboardInstance>,
    mut shadows: Vec<ObjectShadow>,
) -> (WorldLists, Vec<ObjectShadow>) {
    let (region_view, view_region) = f.region;
    let fraction = f.v.animator.fraction();
    let aspect = f.v.size.0 as f32 / f.v.size.1 as f32;
    let (units, unit_shadows) = generate_unit_billboards(
        f.v.mesh_tag,
        f.v.world,
        f.v.camera,
        f.v.animator,
        f.sprite_mgr,
        f.v.size,
        f.v.interface,
        view_region,
    );
    objects.extend(units);
    shadows.extend(unit_shadows);
    shadows.extend(local_projectile_overlays_from_surfaces(
        f.local_overlays,
        f.sprite_mgr,
        f.v.camera,
        f.v.mesh_tag,
    ));
    shadows.extend(lightning_overlays(f.lightning, f.sprite_mgr, f.v.sim));
    retain_region_objects(&mut objects, view_region, f.v.mesh_tag);
    objects.extend(
        f.sprite_mgr
            .marker_billboards(f.v.markers, f.v.local_time, f.v.sim),
    );
    let mut overlays = Vec::new();
    if let Some(colors) = &f.sprite_mgr.interface_colors {
        if let Some(bounds) = f.v.band {
            overlays.extend(band_rectangle_overlays(bounds, colors, f.v.size));
        }
        if let Some(gesture) = f.v.gesture {
            overlays.extend(facing_gesture_arrows(gesture, f.v.world, colors));
        }
    }
    let particles = match (f.v.particles, region_view, view_region) {
        (Some(field), Some(view), Some(region)) => particle_billboards(
            field,
            f.sprite_mgr,
            f.v.sim,
            f.v.camera,
            f.v.mesh_tag,
            (view, region),
            fraction,
        ),
        _ => Vec::new(),
    };
    let lists = WorldLists {
        objects,
        local_projectiles: local_projectile_billboards_from_surfaces(
            f.local_sprites,
            f.sprite_mgr,
            f.v.camera,
            f.v.mesh_tag,
        ),
        lightning: lightning_billboards(
            f.lightning,
            f.sprite_mgr,
            f.v.camera,
            f.v.mesh_tag,
            aspect,
        ),
        mesh_effects: crate::mesh_effect_sprites::mesh_effect_billboards(
            f.sprite_mgr,
            f.v.sim,
            f.v.camera,
            f.v.mesh_tag,
            view_region,
            fraction,
            aspect,
        ),
        particles,
        overlays,
    };
    mark_region_models(
        f.models,
        view_region,
        f.sprite_mgr.scenery_objects.media.as_deref(),
        f.v.mesh_tag,
        f.v.sim.engine_stop.is_none().then_some(&f.v.sim.engine),
    );
    (lists, shadows)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalOverlays {
    InRegion,
    Whole,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Myth2Lighting<'a> {
    pub(crate) ambient_tint: [f32; 3],
    pub(crate) enabled: bool,
    pub(crate) intensity: f32,
    pub(crate) static_lights: &'a [mandrake_render::PointLight],
}

pub(crate) struct Myth2View<'a> {
    pub(crate) mesh_tag: &'a MeshTag,
    pub(crate) sim: &'a Simulation,
    pub(crate) world: &'a WorldView,
    pub(crate) camera: &'a Camera,
    pub(crate) size: (u32, u32),
    pub(crate) bars: (i32, i32),
    pub(crate) animator: &'a UnitAnimator,
    pub(crate) interface: &'a InterfaceView,
    pub(crate) local_time: i32,
    pub(crate) band: Option<selection::PixelRect>,
    pub(crate) gesture: Option<&'a MoveGesture>,
    pub(crate) markers: &'a LocalSprites,
    pub(crate) particles: Option<&'a ParticleField>,
    pub(crate) local_projectiles: Option<&'a LocalProjectiles>,
    pub(crate) local_overlays: LocalOverlays,
    pub(crate) sampling: Sampling,
    pub(crate) sprite_filter: Option<UiFilter>,
    pub(crate) fog: (&'a MapFog, f32),
    pub(crate) lighting: Option<Myth2Lighting<'a>>,
}

pub(crate) struct Myth2Gpu<'g, 'a> {
    pub(crate) device: &'g wgpu::Device,
    pub(crate) queue: &'g wgpu::Queue,
    pub(crate) terrain_renderer: &'g TerrainRenderer,
    pub(crate) terrain_mesh: &'g TerrainMesh,
    pub(crate) model_renderer: &'g ModelRenderer,
    pub(crate) billboard_renderer: &'g mut BillboardRenderer,
    pub(crate) sprite_mgr: &'g ClientSpriteManager,
    pub(crate) model_instances: &'a [ModelInstance],
    pub(crate) model_meshes: &'a BTreeMap<ModelMeshKey, ModelMesh>,
    pub(crate) category2_batch: &'g mut TranslucentBatch,
    pub(crate) reflection_batch: &'g mut ReflectionBatch,
    pub(crate) translucent_batch: &'g mut TranslucentBatch,
    pub(crate) reflection_edge: Option<&'g ReflectionEdge>,
}

pub(crate) struct WorldFeed<'f> {
    pub(crate) rules: &'f ClientRules,
    pub(crate) camera: &'f Camera,
    pub(crate) size: (u32, u32),
    pub(crate) look: WorldLook<'f>,
    pub(crate) lists: WorldLists,
    pub(crate) shadows: Vec<ObjectShadow>,
}

#[derive(Default)]
pub(crate) struct WorldModels<'a> {
    pub(crate) category2: Vec<ModelDraw<'a>>,
    pub(crate) media: Option<ModelMediaDraws<'a>>,
    pub(crate) reflections: Vec<ModelDraw<'a>>,
    pub(crate) opaque: Vec<ModelDraw<'a>>,
    pub(crate) translucent: Vec<ModelDraw<'a>>,
}

pub(crate) struct WorldGpu<'g> {
    pub(crate) terrain_renderer: &'g TerrainRenderer,
    pub(crate) terrain_mesh: &'g TerrainMesh,
    pub(crate) model_renderer: &'g ModelRenderer,
    pub(crate) billboard_renderer: &'g BillboardRenderer,
    pub(crate) atlases: SpriteAtlases<'g>,
    pub(crate) category2: &'g TranslucentBatch,
    pub(crate) reflections: &'g ReflectionBatch,
    pub(crate) soft_edge: Option<&'g ReflectionEdge>,
    pub(crate) translucent: &'g TranslucentBatch,
}

pub(crate) struct WorldFrame<'a> {
    pub(crate) billboards: Vec<BillboardInstance>,
    sprite_order: SpritePassOrder,
    billboard_buf: wgpu::Buffer,
    models: WorldModels<'a>,
}

impl<'a> WorldFrame<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain_renderer: &TerrainRenderer,
        billboard_renderer: &mut BillboardRenderer,
        feed: WorldFeed<'_>,
        clock: &mut Option<FrameClock>,
        landscape: impl FnOnce(Vec<ObjectShadow>),
        models: impl FnOnce() -> WorldModels<'a>,
    ) -> Self {
        terrain_renderer
            .update_uniforms(queue, &world_uniforms(feed.camera, feed.size, &feed.look));
        let (billboards, sprite_order) = assemble_world(feed.rules, feed.camera, feed.lists);
        lap(clock, Lap::Billboards);
        landscape(feed.shadows);
        lap(clock, Lap::LandscapeUpload);
        let billboard_buf = billboard_renderer.upload_instances(device, queue, &billboards);
        let models = models();
        lap(clock, Lap::Prepare);
        Self {
            billboards,
            sprite_order,
            billboard_buf,
            models,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &self,
        gpu: &WorldGpu<'_>,
        encoder: &mut wgpu::CommandEncoder,
        targets: &WorldTargets<'_>,
        landscape: Option<&wgpu::BindGroup>,
        water: Option<&WaterShading>,
        passes: WorldPasses,
        timer: Option<&GpuTimer>,
    ) {
        let reflection_draws: &[ModelDraw<'_>] = self.models.media.as_ref().map_or(
            self.models.reflections.as_slice(),
            ModelMediaDraws::reflection_instances,
        );
        encode_world(
            encoder,
            targets,
            &WorldDraws {
                terrain_renderer: gpu.terrain_renderer,
                terrain_mesh: gpu.terrain_mesh,
                landscape,
                model_renderer: gpu.model_renderer,
                billboard_renderer: gpu.billboard_renderer,
                atlases: SpriteAtlases {
                    atlas: gpu.atlases.atlas,
                    team: gpu.atlases.team,
                },
                billboard_buf: &self.billboard_buf,
                billboards: &self.billboards,
                sprite_order: &self.sprite_order,
                category2: gpu.category2,
                category2_draws: &self.models.category2,
                reflections: gpu.reflections,
                reflection_draws,
                soft_edge: gpu.soft_edge,
                water,
                opaque_models: &self.models.opaque,
                translucent: gpu.translucent,
                translucent_draws: &self.models.translucent,
            },
            passes,
            timer,
        );
    }
}

pub(crate) struct Myth2World<'a> {
    pub(crate) region_view: Option<RegionView>,
    pub(crate) view_region: Option<ViewRegion>,
    pub(crate) frame: WorldFrame<'a>,
}

impl<'a> Myth2World<'a> {
    pub(crate) fn prepare(
        v: &Myth2View<'_>,
        gpu: &mut Myth2Gpu<'_, 'a>,
        (scenery, scenery_shadows): (Vec<BillboardInstance>, Vec<ObjectShadow>),
        lightning_random: &mut LocalRandom,
        clock: &mut Option<FrameClock>,
        landscape: impl FnOnce(Vec<ObjectShadow>),
    ) -> Self {
        let sprite_mgr = gpu.sprite_mgr;
        let (region_view, view_region) = frame_view_parts(
            v.camera,
            v.mesh_tag,
            sprite_mgr.scenery_objects.media.as_deref(),
            v.size,
            v.bars,
            v.local_time,
        )
        .unzip();
        let local_surfaces = |region| {
            v.local_projectiles.map_or_else(Vec::new, |local| {
                local_surfaces_from(
                    local,
                    sprite_mgr,
                    v.sim,
                    (v.camera, v.mesh_tag),
                    region,
                    v.animator.fraction(),
                )
            })
        };
        let local_sprites = local_surfaces(view_region.as_ref());
        let local_overlays = match v.local_overlays {
            LocalOverlays::InRegion => None,
            LocalOverlays::Whole if sprite_mgr.local_projectile_sprites.has_overlays() => {
                Some(local_surfaces(None))
            }
            LocalOverlays::Whole => Some(Vec::new()),
        };
        let lightning = lightning_surfaces(
            sprite_mgr,
            v.sim,
            v.mesh_tag,
            view_region.as_ref(),
            lightning_random,
        );
        let lights = match v.lighting {
            Some(l) if l.enabled => crate::lighting::collect_scene_lights(
                &local_sprites,
                sprite_mgr,
                v.mesh_tag,
                v.camera.eye_position(),
                l.static_lights,
                l.intensity,
            ),
            _ => Vec::new(),
        };
        let look = WorldLook {
            filtering: WorldFiltering::Myth2 {
                sampling: v.sampling,
                sprite_filter: v.sprite_filter,
            },
            fog: Some(v.fog),
            lighting: v.lighting.map(|l| SceneLighting {
                ambient_tint: l.ambient_tint,
                enabled: l.enabled,
                lights: &lights,
            }),
        };
        let (lists, shadows) = myth2_sprites(
            &Myth2Frame {
                v,
                sprite_mgr,
                region: (region_view.as_ref(), view_region.as_ref()),
                lightning: &lightning,
                local_overlays: local_overlays.as_deref().unwrap_or(&local_sprites),
                local_sprites: &local_sprites,
                models: gpu.model_instances,
            },
            scenery,
            scenery_shadows,
        );
        let feed = WorldFeed {
            rules: &ClientRules::MYTH2,
            camera: v.camera,
            size: v.size,
            look,
            lists,
            shadows,
        };
        let models = || {
            let mut media = prepare_model_media(
                gpu.device,
                gpu.queue,
                gpu.model_renderer,
                gpu.category2_batch,
                gpu.reflection_batch,
                gpu.model_instances,
                gpu.model_meshes,
                v.camera,
                sprite_mgr.scenery_objects.media.as_deref(),
                v.mesh_tag,
                v.local_time,
                view_region.as_ref(),
            );
            let translucent_models =
                translucent_model_instances(gpu.model_instances, gpu.model_meshes);
            let translucent = prepare_translucent_models(
                gpu.device,
                gpu.queue,
                gpu.translucent_batch,
                &translucent_models,
                v.camera,
                v.size.0 as f32 / v.size.1 as f32,
            );
            WorldModels {
                category2: std::mem::take(&mut media.category2),
                media: Some(media),
                reflections: Vec::new(),
                opaque: opaque_model_draws(gpu.model_instances, gpu.model_meshes),
                translucent,
            }
        };
        let frame = WorldFrame::prepare(
            gpu.device,
            gpu.queue,
            gpu.terrain_renderer,
            gpu.billboard_renderer,
            feed,
            clock,
            landscape,
            models,
        );
        Self {
            region_view,
            view_region,
            frame,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &self,
        gpu: &Myth2Gpu<'_, '_>,
        encoder: &mut wgpu::CommandEncoder,
        targets: &WorldTargets<'_>,
        landscape: Option<&wgpu::BindGroup>,
        water: Option<&WaterShading>,
        passes: WorldPasses,
        timer: Option<&GpuTimer>,
    ) {
        let sprite_mgr = gpu.sprite_mgr;
        self.frame.encode(
            &WorldGpu {
                terrain_renderer: gpu.terrain_renderer,
                terrain_mesh: gpu.terrain_mesh,
                model_renderer: gpu.model_renderer,
                billboard_renderer: gpu.billboard_renderer,
                atlases: SpriteAtlases {
                    atlas: &sprite_mgr.atlas.bind_group,
                    team: sprite_mgr.team_cache.atlas.as_ref().map(|a| &a.bind_group),
                },
                category2: gpu.category2_batch,
                reflections: gpu.reflection_batch,
                soft_edge: gpu.reflection_edge,
                translucent: gpu.translucent_batch,
            },
            encoder,
            targets,
            landscape,
            water,
            passes,
            timer,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ObjectRule {
    pub(crate) own: bool,
    pub(crate) reflected: bool,
    pub(crate) surface_only: bool,
    pub(crate) half: bool,
    pub(crate) in_water: bool,
}

impl ObjectRule {
    pub(crate) const PLAIN: Self = Self {
        own: true,
        reflected: false,
        surface_only: false,
        half: false,
        in_water: false,
    };

    pub(crate) fn of(
        rules: &ClientRules,
        flags: u32,
        seen: Option<bool>,
        cell_flags: Option<u8>,
        media_pixel: impl FnOnce() -> bool,
    ) -> Option<Self> {
        let whole = seen?;
        let water_only = flags & rules.water_only_flag != 0;
        Some(Self {
            own: !water_only,
            reflected: Self::reflects(rules, cell_flags),
            surface_only: water_only,
            half: !whole,
            in_water: in_water(rules, cell_flags, media_pixel),
        })
    }

    pub(crate) fn under_model(self, model: Option<i32>, z: i32) -> Self {
        Self {
            reflected: self.reflected && model.is_none_or(|h| z < h),
            ..self
        }
    }

    pub(crate) fn reflects(rules: &ClientRules, cell_flags: Option<u8>) -> bool {
        cell_flags.is_some_and(|f| f & rules.reflect_cells != 0)
    }

    pub(crate) fn billboards(
        self,
        rules: &ClientRules,
        b: BillboardInstance,
        reflection: Option<BillboardInstance>,
        out: &mut Vec<BillboardInstance>,
    ) {
        let placed = |mut b: BillboardInstance| {
            if rules.bucket_depth {
                b.flags |= mandrake_view::billboard::BILLBOARD_FLAG_BUCKET_DEPTH;
            }
            if self.half {
                b.tint[3] *= 0.5;
            }
            b
        };
        let b = placed(b);
        if self.reflected {
            if self.surface_only {
                let mut r = b;
                r.flags |= mandrake_view::billboard::BILLBOARD_FLAG_MEDIA_EFFECT;
                out.push(r);
            } else {
                out.extend(reflection.map(placed));
            }
        }
        if self.own {
            out.push(if self.in_water {
                b.with_media_clip(0.0)
            } else {
                b
            });
        }
    }
}

pub(crate) fn in_water(
    rules: &ClientRules,
    cell_flags: Option<u8>,
    media_pixel: impl FnOnce() -> bool,
) -> bool {
    cell_flags.is_some_and(|f| f & rules.water_cells != 0) && media_pixel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_keep_the_scene_s_order() {
        let one = |x: f32| {
            let mut b = BillboardInstance::default();
            b.tint[1] = x;
            vec![b]
        };
        let lists = || WorldLists {
            objects: one(1.0),
            local_projectiles: one(2.0),
            lightning: one(3.0),
            mesh_effects: one(4.0),
            particles: one(5.0),
            overlays: Vec::new(),
        };
        let camera = Camera::default();
        let sizes = |rules| {
            let (b, _) = assemble_world(rules, &camera, lists());
            b.iter().map(|b| b.tint[1]).collect::<Vec<_>>()
        };
        assert_eq!(sizes(&ClientRules::MYTH2), [1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(sizes(&ClientRules::TFL), [1.0, 3.0, 2.0, 4.0, 5.0]);
    }
}
