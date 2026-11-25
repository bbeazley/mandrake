use crate::app::view_settings::ViewSettings;
use crate::args::Args;
use crate::corpse::StainSequence;
use crate::headless_render::{Offscreen, headless_device};
use crate::object_history::FRACTION_ONE;
use crate::object_sprites::{DrawnObject, ObjectDraw, ObjectFrame, SurfaceReflection, object_draw};
use crate::overlays::{
    TflFrameStyle, TflMonsterOverlay, band_rectangle_overlays, facing_arrows_at,
    tfl_selection_overlays,
};
use crate::render_passes::{
    PassSkips, SceneLighting, SpriteAtlases, WorldFiltering, WorldLook, WorldPasses, WorldTargets,
};
use crate::scenery_billboards::ObjectShadow;
use crate::tfl_game::{FrameTime, TflReplay};
use crate::tfl_hud::{TflHud, TflHudSetup, TflHudView};
use crate::tfl_map::{TflMap, load_tfl_map};
use crate::tfl_models::TflModels;
use crate::tfl_objects::{
    TFL_CELL_SIZE, TflObjectSprites, TflSprites, WORLD_PER_RENDER_UNIT, tfl_sprites,
};
use crate::world_assembly::{ObjectRule, WorldFeed, WorldFrame, WorldGpu, WorldLists, WorldModels};
use crate::world_view::ClientRules;
use clap::Parser;
use mandrake_render::{
    BillboardRenderer, ModelRenderer, RenderPipelineConfig, TerrainMesh, TerrainRenderer,
    TranslucentBatch, create_atlas_from_images_with_offsets, grow_atlas_from_images_with_offsets,
};
use mandrake_tag::InterfaceColorTable;
use mandrake_tag::collection::CollectionTag;
use mandrake_tag::sound_manager::LocalRandom;
use mandrake_view::landscape::{LandscapeTexels, ShadeOverlay};
use mandrake_view::projectile::EngineViewRows;
use mandrake_view::selection::{ObjectBounds, PixelRect};
use mandrake_view::{billboard::BillboardInstance, camera::Camera};
use std::path::Path;
use tracing::info;

const LOCAL_TEAM: i16 = 0;

use crate::tfl_lightning::TflBolts;
use crate::tfl_local_projectiles::TflLocal;

const FIRST_VIEW_SIZE: (u32, u32) = (1024, 768);

pub(crate) fn run_tfl(args: &Args, game_dir: &Path) -> anyhow::Result<()> {
    let map = load_tfl_map(game_dir, args.replay.as_deref(), args.mesh.as_deref())?;
    let Some(out) = args.screenshot.as_deref() else {
        let replay = if crate::tfl_window::opens_menus(args) {
            None
        } else {
            TflReplay::new(game_dir, &map)
        };
        return crate::tfl_window::run_tfl_window(args, map, replay);
    };
    let mut replay = TflReplay::new(game_dir, &map);
    if let Some(r) = replay.as_mut() {
        r.run(args.ticks);
        println!("TFL film frame at {}", r.status());
    }
    let camera = start_camera(&map, args);
    let mut hud = mandrake_tag::tag_files::open_tfl_tags(game_dir)
        .ok()
        .map(|archive| TflHud::new(archive, TflHudSetup::of_map(&map)));
    if let Some(h) = hud.as_mut() {
        h.set_status_bar(crate::interface_config::load(args.conf.as_deref(), true).status_bar);
        let mut view = replay
            .as_ref()
            .and_then(|r| Some(crate::world_view::tfl::interface_view(r.sim()?, r.tick())))
            .unwrap_or_default();
        let tick = view.tick;
        view.tick = 0;
        h.update(&view);
        view.tick = tick;
        h.update(&view);
        if args.key_bindings {
            let controls = crate::controls::tfl::load(args.conf.as_deref()).0;
            h.set_key_bindings(Some(&controls));
            for row in h.key_rows().unwrap_or_default() {
                info!(
                    "Key binding: {:?} {:?}",
                    String::from_utf8_lossy(&row.action),
                    String::from_utf8_lossy(&row.key)
                );
            }
        }
    }
    render_tfl_frame(&map, replay.as_ref(), &camera, args, out, hud.as_mut())?;
    info!(
        "TFL frame of {} ('{}', mesh {}) written to {}",
        map.film
            .as_ref()
            .map_or_else(|| "no film".to_string(), |f| format!("film '{}'", f.name)),
        map.mesh_name,
        map.mesh.mesh_id,
        out.display()
    );
    Ok(())
}

pub(crate) fn start_camera(map: &TflMap, args: &Args) -> Camera {
    let teams = crate::start_camera::StartTeams {
        mode: 2,
        local_team: LOCAL_TEAM,
        players: Vec::new(),
    };
    crate::start_camera::initial_camera_with(&map.mesh, args, &teams, TFL_CELL_SIZE)
}

pub(crate) fn network_start_camera(map: &TflMap, args: &Args, local_team: i16) -> Camera {
    let teams = crate::start_camera::StartTeams {
        mode: 2,
        local_team,
        players: Vec::new(),
    };
    let mut args = args.clone();
    let legacy = args.target_x.is_some()
        || args.target_z.is_some()
        || args.zoom.is_some()
        || args.yaw.is_some()
        || args.pitch.is_some();
    if args.observer.is_none() && !legacy && tfl_start_marker(&map.mesh, local_team).is_none() {
        args.observer = Some(vec![
            f32::from(map.mesh.cell_width) * TFL_CELL_SIZE * 0.5,
            f32::from(map.mesh.cell_height) * TFL_CELL_SIZE * 0.5,
            0.0,
            mandrake_view::camera::myth_observer::START_ZOOM,
        ]);
    }
    crate::start_camera::initial_camera_with(&map.mesh, &args, &teams, TFL_CELL_SIZE)
}

pub(crate) fn tfl_start_marker(
    mesh: &mandrake_tag::MeshTag,
    local_team: i16,
) -> Option<&mandrake_tag::MarkerEntry> {
    mesh.observer_markers().find(|m| {
        mesh.palette_for_marker(m)
            .is_some_and(|p| p.team_index() == local_team)
    })
}

pub(crate) fn tfl_landscape_texels(
    tex: &mandrake_tag::collection::TerrainTexture,
) -> LandscapeTexels {
    let table = mandrake_tag::tfl::landscape_shading_table(
        &tex.palette16,
        tex.color_count,
        mandrake_tag::tfl::TFL_SHADING_LEVELS,
    );
    LandscapeTexels::with_table(tex, &table)
}

fn tfl_landscape_detail(
    args: &Args,
    map: &TflMap,
) -> Option<std::sync::Arc<mandrake_render::detail::LandscapeDetail>> {
    let vfs = mandrake_common::game_vfs();
    let packs: Vec<mandrake_tag::TagArchive> =
        crate::map_load::enhancement_packs(&args.archive, &args.plugin)
            .iter()
            .filter_map(|(_, p)| mandrake_tag::TagArchive::open(vfs, &p.to_string_lossy()).ok())
            .collect();
    let search: Vec<&mandrake_tag::TagArchive> =
        packs.iter().chain(std::iter::once(&map.archive)).collect();
    crate::map_load::load_landscape_detail(&search, &map.mesh)
}

pub(crate) struct TflLook {
    view: ViewSettings,
    fog: Option<crate::fog::MapFog>,
    lighting: Option<crate::lighting::MapLighting>,
    fog_reload: Option<crate::fog::FogReload>,
    lighting_reload: Option<crate::lighting::LightingReload>,
    detail: Option<std::sync::Arc<mandrake_render::detail::LandscapeDetail>>,
    water: Option<mandrake_view::water::WaterSettings>,
}

impl TflLook {
    pub(crate) fn new(args: &Args, map: &TflMap, view: &ViewSettings, headless: bool) -> Self {
        let extras = view.extras;
        let fog = extras
            .fog
            .then(|| {
                if headless {
                    crate::fog::MapFog::at_map_load_headless(args, &map.mesh)
                } else {
                    crate::fog::MapFog::at_map_load(args, &map.mesh)
                }
            })
            .filter(crate::fog::MapFog::has_height_fog);
        let lighting = extras.lighting.then(|| {
            let config = if headless && args.conf.is_none() {
                crate::lighting::LightingConfig::defaults()
            } else {
                let (config, _, diagnostics) = crate::lighting::load_native(args.conf.as_deref());
                for d in &diagnostics {
                    eprintln!("lighting: {d}");
                }
                config
            };
            crate::lighting::resolve_lighting_in(
                mandrake_tag::tfl::Game::Tfl,
                crate::args::Lighting::On,
                &config,
                &crate::lighting::flag_layer(args),
                map.mesh.mesh_id.as_str(),
            )
        });
        info!(
            "TFL look: detail shadows {}, fog {}, lighting {:?}, reflections' soft edge {}",
            extras.detail_shadows,
            fog.is_some(),
            lighting.filter(|l| l.enabled),
            view.reflection_soft_edge,
        );
        let lighting_reload = lighting.filter(|_| !headless).and_then(|_| {
            crate::lighting::LightingReload::for_tfl(args, map.mesh.mesh_id.as_str())
        });
        let fog_reload = fog
            .as_ref()
            .filter(|_| !headless)
            .and_then(|f| crate::fog::FogReload::at_map_load(args, &map.mesh, f));
        let detail = view
            .detail_textures
            .then(|| tfl_landscape_detail(args, map))
            .flatten();
        let mesh_id = map.mesh.mesh_id.as_str();
        let water = view
            .water_on(mesh_id)
            .then(|| crate::water_config::load(args.conf.as_deref(), headless, mesh_id));
        Self {
            view: view.clone(),
            fog,
            lighting,
            fog_reload,
            lighting_reload,
            detail,
            water,
        }
    }

    fn poll_reloads(&mut self) {
        if let (Some(reload), Some(fog)) = (self.fog_reload.as_mut(), self.fog.as_mut()) {
            reload.poll(fog);
        }
        if let (Some(reload), Some(lighting)) =
            (self.lighting_reload.as_mut(), self.lighting.as_mut())
        {
            reload.poll(lighting);
        }
    }

    fn lit(&self) -> Option<&crate::lighting::MapLighting> {
        self.lighting.as_ref().filter(|l| l.enabled)
    }

    pub(crate) fn tfl_1_5() -> Self {
        let mut view = ViewSettings::of(&Args::parse_from(["mandrake"]));
        view.reflection_soft_edge = false;
        view.extras = crate::args::TflExtras {
            detail_shadows: false,
            fog: false,
            lighting: false,
        };
        Self {
            view,
            fog: None,
            lighting: None,
            fog_reload: None,
            lighting_reload: None,
            detail: None,
            water: None,
        }
    }

    pub(crate) fn detail_shadows(&self) -> bool {
        self.view.extras.detail_shadows
    }

    fn world_look<'a>(&'a self, lights: &'a [mandrake_render::PointLight]) -> WorldLook<'a> {
        WorldLook {
            filtering: WorldFiltering::tfl(self.view.sampling, self.view.sprite_filter),
            fog: self.fog.as_ref().map(|f| (f, f.seconds_now())),
            lighting: self.lit().map(|l| SceneLighting {
                ambient_tint: l.ambient_tint,
                enabled: true,
                lights,
            }),
        }
    }
}

fn glide_allowed() -> bool {
    static ALLOWED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ALLOWED.get_or_init(|| std::env::var("M2_TFL_GLIDE").map_or(true, |v| v != "0"))
}

fn glide(drawn: &mut DrawnObject, replay: &TflReplay, fraction: i32) {
    let tick = replay.tick();
    let Some([x, y, z]) = replay
        .stride(drawn.object as usize)
        .and_then(|s| s.at_tick(tick, fraction))
    else {
        return;
    };
    let fraction = fraction.clamp(0, FRACTION_ONE);
    drawn.world = [x, y, i32::from(glide_height(z, drawn, tick, fraction))];
    drawn.position = crate::tfl_objects::world_render_position(drawn.world, replay.cells_across());
}

fn glide_height(z: i32, drawn: &DrawnObject, tick: u32, fraction: i32) -> i16 {
    let index = drawn.object as usize;
    let float_height_at = |t: u32| crate::world_view::tfl::drawn_height(0, drawn.flags, index, t);
    let (before, live) = (
        float_height_at(tick.saturating_sub(1)),
        float_height_at(tick),
    );
    let float_height = before + (((live - before) * fraction) >> 16);
    (z as i16).wrapping_add(float_height as i16)
}

pub(crate) struct TflScene {
    terrain_renderer: TerrainRenderer,
    model_renderer: ModelRenderer,
    billboard_renderer: BillboardRenderer,
    terrain_mesh: TerrainMesh,
    waves: Option<crate::tfl_landscape::TflWaves>,
    ground: Option<TflGround>,
    models: TflModels,
    sprites: TflSprites,
    atlas: mandrake_render::TextureAtlas,
    atlas_images: usize,
    drawn: Vec<(BillboardInstance, Option<BillboardInstance>, ObjectRule)>,
    cell_flags: Vec<mandrake_tag::tfl::TflCell>,
    ground_tick: Option<u32>,
    translucent_batch: TranslucentBatch,
    first_pass_batch: TranslucentBatch,
    prepared: Option<(u64, Camera, i32)>,
    cells: (u32, u32),
    model_reflections: mandrake_render::ReflectionBatch,
    reflection_tint: [u8; 8],
    ground_glow: bool,
    glow: Vec<crate::scenery_billboards::ObjectShadow>,
    lights: Vec<mandrake_render::PointLight>,
    fires: Option<crate::lighting::tfl::TflFires>,
    ripple: Option<crate::tfl_landscape::Waves>,
    interface_colors: Option<InterfaceColorTable>,
    markers: TflMarkers,
    framed: Vec<Framed>,
    weather: Option<TflWeather>,
    weather_tried: bool,
    look: TflLook,
    reflection_edge: Option<((u32, u32), mandrake_render::ReflectionEdge)>,
    detail_applied: bool,
    water: Option<crate::tfl_water::TflWater>,
    water_followed: bool,
    water_time: FrameTime,
    local: Option<TflLocal>,
    local_tried: bool,
    fixed_frames: bool,
    glide: bool,
    bolts: TflBolts,
    bolts_random: LocalRandom,
    view_size: (u32, u32),
    shadows: Option<Vec<ObjectShadow>>,
    sight_cache: Option<(u32, bool, Option<crate::world_view::ViewSight>)>,
    lit_projectiles: std::collections::HashSet<usize>,
}

struct TflGround {
    stains: TflStains,
    texture: mandrake_render::LandscapeTexture,
    shadows: Vec<ObjectShadow>,
}

impl TflGround {
    fn paint(
        &mut self,
        queue: &wgpu::Queue,
        map: &TflMap,
        replay: &TflReplay,
        before_mark: &mut dyn FnMut(Option<u32>, &mut LocalRandom),
    ) {
        let overlays: Vec<ShadeOverlay<'_>> =
            self.shadows.iter().map(ObjectShadow::overlay).collect();
        for rect in self.stains.paint(map, replay, before_mark) {
            self.texture.write_region(
                queue,
                rect,
                &self.stains.texels.region_rgba(rect, &overlays),
            );
        }
    }

    fn shade(&mut self, queue: &wgpu::Queue, shadows: Vec<ObjectShadow>) {
        if shadows.is_empty() && self.shadows.is_empty() {
            return;
        }
        let texels = &mut self.stains.texels;
        let old: Vec<ShadeOverlay<'_>> = self.shadows.iter().map(ObjectShadow::overlay).collect();
        let new: Vec<ShadeOverlay<'_>> = shadows.iter().map(ObjectShadow::overlay).collect();
        let mut dirty = mandrake_view::landscape::changed_overlay_rects(
            &old,
            &new,
            texels.width,
            texels.height,
        );
        let texture = &self.texture;
        texels.rebuild_regions(&mut dirty, &new, |rect, rgba| {
            texture.write_region(queue, rect, rgba);
        });
        drop((old, new));
        self.shadows = shadows;
    }
}

fn object_shadow(
    sprites: &TflSprites,
    sim: &mandrake_tfl_sim::TflSim,
    (o, rule): (&DrawnObject, &ObjectRule),
    detail: bool,
) -> Option<ObjectShadow> {
    if !rule.own || o.hidden || o.sequence == 0xff || o.flags & OBJECT_NO_SHADOW != 0 {
        return None;
    }
    let seq = sprites.decoded(o)?;
    let frame = seq
        .frames
        .get(usize::try_from(o.frame).unwrap_or(0) % seq.frames.len().max(1))?;
    let shadow = if detail {
        let scale = (i64::from(o.scale) * 7 / 10).clamp(0, 0xffff) as u16;
        crate::shadowgen::drawn_shadow(
            &seq.sequence,
            frame,
            &crate::shadowgen::ShadowObject {
                facing: o.facing,
                stored_view: o.view.into(),
                scale,
                flags: o.flags,
            },
        )?
    } else {
        frame.shadow.clone()?
    };
    let object = sim.objects.get(usize::try_from(o.object).ok()?)?;
    let (mode, count, period) = o.transfer;
    let place = mandrake_view::landscape::TflShadowPlace {
        above_ground: object.above_ground,
        drawn_height: o.world[2],
        in_water_top: rule.in_water.then(|| object.z + i32::from(object.top)),
        transfer: (
            mode,
            mandrake_view::projectile::fade_ratio(count, period).unwrap_or(0) as u16,
        ),
    };
    let intensity = mandrake_view::landscape::tfl_shadow_intensity(&place)?;
    Some(ObjectShadow::at(
        (o.world[0], o.world[1]),
        intensity,
        &shadow,
    ))
}

const OBJECT_NO_SHADOW: u32 = 0x20_0000;

type RemappedStain = (StainSequence, [u8; 256]);

pub(crate) struct TflStains {
    texels: LandscapeTexels,
    sequences: std::collections::BTreeMap<(mandrake_common::FourCC, i16), Option<RemappedStain>>,
    random: LocalRandom,
    painted: usize,
    marks_painted: usize,
}

impl TflStains {
    pub(crate) fn new(map: &TflMap, mut texels: LandscapeTexels) -> Self {
        match map
            .tag(b".256", map.tfl_mesh.landscape_tag)
            .and_then(|d| tag_blend_table(&d))
        {
            Some(table) => texels.set_blend_table(table),
            None => texels.prepare_stains(),
        }
        Self {
            texels,
            sequences: std::collections::BTreeMap::new(),
            random: LocalRandom::new(0),
            painted: 0,
            marks_painted: 0,
        }
    }

    pub(crate) fn paint(
        &mut self,
        map: &TflMap,
        replay: &TflReplay,
        before_mark: &mut dyn FnMut(Option<u32>, &mut LocalRandom),
    ) -> Vec<mandrake_view::landscape::PixelRect> {
        let (stains, marks) = (
            replay.stains_from(self.painted),
            replay.marks_from(self.marks_painted),
        );
        let mut dirty = Vec::new();
        if stains.is_empty() && marks.is_empty() {
            before_mark(None, &mut self.random);
            return dirty;
        }
        let sim = replay.sim();
        let is_media = |x: i32, y: i32| sim.is_some_and(|s| s.media_pixel(x, y).unwrap_or(false));
        let (mut si, mut mi) = (0usize, 0usize);
        loop {
            let stain_next = match (stains.get(si), marks.get(mi)) {
                (Some(s), Some(m)) => s.order < m.order(),
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if stain_next {
                before_mark(Some(stains[si].order), &mut self.random);
                dirty.extend(self.paint_stain(map, &stains[si], &is_media));
                si += 1;
            } else {
                before_mark(Some(marks[mi].order()), &mut self.random);
                dirty.extend(self.paint_mark(&marks[mi], &is_media));
                mi += 1;
            }
        }
        before_mark(None, &mut self.random);
        info!(
            "TFL marks: {} stains, {} blood and footprints painted at tick {}",
            si,
            mi,
            replay.tick()
        );
        self.painted += si;
        self.marks_painted += mi;
        dirty
    }

    fn paint_stain(
        &mut self,
        map: &TflMap,
        s: &mandrake_tfl_sim::stains::Stain,
        is_media: &dyn Fn(i32, i32) -> bool,
    ) -> Option<mandrake_view::landscape::PixelRect> {
        let collection = s.collection?;
        let texels = &self.texels;
        let stain = self
            .sequences
            .entry((collection, s.sequence))
            .or_insert_with(|| {
                let stain = map
                    .tag(b".256", collection)
                    .and_then(|d| CollectionTag::from_bytes_tfl(&d).ok())
                    .and_then(|c| StainSequence::from_collection(&c, s.sequence as usize))?;
                let remap = texels.stain_remap(&stain.color_table);
                Some((stain, remap))
            });
        let (bitmap, remap) = stain
            .as_ref()
            .and_then(|(st, remap)| Some((st.bitmap(s.random)?, remap)))?;
        let mirror = crate::landscape_edit::stain_mirror(&mut self.random);
        self.texels
            .apply_stain_remapped(bitmap, remap, (s.world[0], s.world[1]), mirror, is_media)
    }

    fn paint_mark(
        &mut self,
        mark: &mandrake_tfl_sim::stains::Mark,
        is_media: &dyn Fn(i32, i32) -> bool,
    ) -> Option<mandrake_view::landscape::PixelRect> {
        let random = &mut self.random;
        match mark {
            mandrake_tfl_sim::stains::Mark::Blood(b) => {
                self.texels
                    .bloody_mesh((b.world[0], b.world[1]), is_media, &mut || {
                        random.next_u16()
                    })
            }
            mandrake_tfl_sim::stains::Mark::Footprint(f) => {
                self.texels
                    .footstep_above(f.world, TFL_FOOTPRINT_SHADE, &mut || random.next_u16())
            }
        }
    }
}

const TFL_FOOTPRINT_SHADE: u8 = 0x28;

fn tag_blend_table(data: &[u8]) -> Option<Vec<u8>> {
    let word = |at: usize| {
        data.get(at..at + 4)
            .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (count, offset) = (word(0xa0)?, usize::try_from(word(0xa4)?).ok()?);
    let start = mandrake_tag::collection::TerrainTexture::HDR + offset;
    (count != 0)
        .then(|| data.get(start..start + (1 << 16)))
        .flatten()
        .map(<[u8]>::to_vec)
}

fn shared_random<'a>(
    local: &'a mut Option<TflLocal>,
    ground: &'a mut Option<TflGround>,
    bolts: &'a mut LocalRandom,
) -> &'a mut LocalRandom {
    match (local.as_mut(), ground.as_mut()) {
        (Some(l), _) => l.local.random_mut(),
        (None, Some(g)) => &mut g.stains.random,
        (None, None) => bolts,
    }
}

const WEATHER_WARM_UP_TICKS: u32 = 300;

struct TflWeather {
    field: crate::particles::ParticleField,
    wind: Option<crate::particles::LocalWind>,
    visual: crate::unit_anim::SequenceVisual,
    flags: Vec<u32>,
    scale: i16,
    sprites: Option<crate::particles::ParticleSprites>,
    tick: u32,
    fraction: i32,
}

impl TflWeather {
    fn new(
        map: &TflMap,
        replay: &TflReplay,
        sprites: &mut TflSprites,
        eye: (i32, i32),
        random: &mut LocalRandom,
    ) -> Option<Self> {
        let sim = replay.sim()?;
        sim.particles.as_ref()?;
        let tag = sim.particle_system_tag()?;
        let definition = crate::particles::ParticleDefinition::parse_tfl(&map.tag(b"part", tag)?)?;
        let index = usize::try_from(definition.sequence).ok()?;
        let (visual, flags) =
            sprites.particle_sequence(map, definition.collection_reference, index)?;
        let shape = crate::particles::SequenceShape {
            views: visual.sequence.num_views as i16,
            frames: 0,
            ticks_per_frame: visual.sequence.ticks_per_frame,
        };
        let scale = definition.scale;
        let field = crate::particles::ParticleField::for_map_on(
            definition,
            shape,
            weather_density(replay, eye),
            random,
        );
        let wind = map
            .tag(b"mesh", map.mesh.mesh_id)
            .and_then(|m| {
                Some(mandrake_common::FourCC::new(
                    m.get(0xe4..0xe8)?.try_into().ok()?,
                ))
            })
            .filter(|id| !matches!(u32::from_be_bytes(id.0), 0 | 0xffff_ffff))
            .and_then(|id| crate::particles::LocalWind::new_on(&map.tag(b"wind", id)?, random));
        let mut w = Self {
            field,
            wind,
            visual,
            flags,
            scale,
            sprites: None,
            tick: replay.tick().saturating_sub(WEATHER_WARM_UP_TICKS),
            fraction: FRACTION_ONE,
        };
        w.follow(replay, eye, random);
        info!("TFL weather: part {tag}, sequence {index}");
        Some(w)
    }

    fn follow(&mut self, replay: &TflReplay, eye: (i32, i32), random: &mut LocalRandom) {
        let now = replay.tick();
        if now <= self.tick {
            return;
        }
        let density = weather_density(replay, eye);
        for _ in self.tick..now {
            if let Some(w) = self.wind.as_mut() {
                std::mem::swap(w.random_mut(), random);
                w.update(1);
                std::mem::swap(w.random_mut(), random);
            }
            let wind = self
                .wind
                .as_ref()
                .map(crate::particles::LocalWind::velocity);
            std::mem::swap(self.field.random_mut(), random);
            self.field.update(1, density, wind);
            std::mem::swap(self.field.random_mut(), random);
        }
        self.tick = now;
    }

    fn billboards(
        &self,
        camera: &Camera,
        size: (u32, u32),
        (cols, rows): (u32, u32),
        cells: &[mandrake_tag::tfl::TflCell],
        atlas: &mandrake_render::TextureAtlas,
    ) -> Vec<BillboardInstance> {
        let Some(sprites) = self.sprites.as_ref() else {
            return Vec::new();
        };
        let frame = world_frame((cols, rows));
        let view = crate::region_view::region_view_in(camera, frame, size, (0, 0));
        let region = tfl_region(camera, size, (cols, rows), cells);
        let surfaces = self.field.surfaces(
            &view,
            &region,
            sprites,
            self.fraction - FRACTION_ONE,
            self.wind
                .as_ref()
                .map(crate::particles::LocalWind::velocity),
        );
        sprites.instances(
            &surfaces,
            atlas,
            frame,
            camera.view_axes(),
            sprites.shade(Default::default()),
        )
    }

    fn make_sprites(&mut self, atlas: &mandrake_render::TextureAtlas) {
        self.sprites = Some(crate::particles::ParticleSprites::with_pixel_scale(
            &self.visual,
            crate::sprite_manager::frame_flags(&self.flags, self.visual.frames.len().max(1)),
            self.scale,
            crate::media::CoreRef::default(),
            |index| crate::sprite_manager::atlas_anchor(atlas, index),
            crate::tfl_objects::TFL_SPRITE_SCALE,
        ));
    }
}

pub(crate) fn world_frame((cols, rows): (u32, u32)) -> mandrake_view::model_surfaces::WorldFrame {
    mandrake_view::model_surfaces::WorldFrame {
        half_width: cols as f32 * TFL_CELL_SIZE * 0.5,
        half_height: rows as f32 * TFL_CELL_SIZE * 0.5,
    }
}

pub(crate) fn tfl_region(
    camera: &Camera,
    size: (u32, u32),
    (cols, rows): (u32, u32),
    cells: &[mandrake_tag::tfl::TflCell],
) -> mandrake_view::view_region::ViewRegion {
    let view = crate::region_view::region_view_in(camera, world_frame((cols, rows)), size, (0, 0));
    let vertex = |c: i32, r: i32| {
        let (c, r) = (c.clamp(0, cols as i32 - 1), r.clamp(0, rows as i32 - 1));
        let z = cells
            .get(r as usize * cols as usize + c as usize)
            .map_or(0, |cell| {
                let z = i32::from(cell.physical_height);
                if cell.flags & 1 != 0 { z.max(0) } else { z }
            });
        [(c << 10) as f32, (r << 10) as f32, z as f32]
    };
    let trig = mandrake_tfl_sim::trig::trig();
    mandrake_view::view_region::build_tfl_view_region(
        &view,
        observer_view(camera, (cols, rows)).facing,
        (cols as i32, rows as i32),
        (&trig.cosine, &trig.sine),
        &vertex,
    )
}

fn observer_view(camera: &Camera, cells: (u32, u32)) -> TflHudView {
    let o = mandrake_view::camera::ObserverController::from_camera(camera);
    TflHudView::of_observer(o.origin, o.yaw, (cells.0 as i32, cells.1 as i32))
}

fn weather_density(replay: &TflReplay, (x, y): (i32, i32)) -> u16 {
    replay
        .sim()
        .and_then(|s| {
            let mesh = &s.data.mesh;
            s.particles
                .as_ref()?
                .density(x, y, mesh.cell_width as i16, mesh.cell_height as i16)
        })
        .unwrap_or(0)
}

const OBJECT_FLIPPED: u32 = 0x4_0000;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TflMarkers {
    pub(crate) framed: Vec<TflFrame>,
    pub(crate) band: Option<PixelRect>,
    pub(crate) gesture: Option<crate::tfl_input::Flick>,
    pub(crate) ground: Vec<crate::move_marker::LocalSprite>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TflFrame {
    pub(crate) object: usize,
    pub(crate) monster: bool,
    pub(crate) hovered_only: bool,
    pub(crate) frame: bool,
}

#[derive(Clone, Copy, Debug)]
struct Framed {
    #[allow(dead_code)]
    object: usize,
    position: [f32; 3],
    bounds: ObjectBounds,
    flipped: bool,
    monster: Option<TflMonsterOverlay>,
    hovered_only: bool,
    frame: bool,
    submerged: bool,
}

fn framed_of(
    units: &[crate::world_view::ViewUnit],
    replay: &TflReplay,
    frame: &TflFrame,
) -> Option<(ObjectBounds, Option<TflMonsterOverlay>)> {
    if !frame.monster {
        let s = replay.sim()?.objects.get(frame.object)?;
        let bounds = ObjectBounds {
            half_width: s.radius,
            bottom: s.bottom,
            top: s.top,
        };
        return Some((bounds, None));
    }
    let i = units
        .binary_search_by_key(&(frame.object as u32), |u| u.id)
        .ok()?;
    let unit = &units[i];
    let pick = unit.pick?;
    let overlay = TflMonsterOverlay {
        eval: u32::from(pick.evaluation),
        flags: unit.monster?.flags_8,
        vitality: pick.vitality?,
    };
    Some((pick.bounds, Some(overlay)))
}

const RULES: &ClientRules = &ClientRules::TFL;

fn in_water(
    sim: &mandrake_tfl_sim::TflSim,
    cell: Option<&mandrake_tag::tfl::TflCell>,
    x: i32,
    y: i32,
) -> bool {
    crate::world_assembly::in_water(RULES, cell.map(|c| c.flags), || {
        matches!(sim.media_pixel(x >> 6, y >> 6), Ok(true))
    })
}

fn reflection_tint(media: Option<&[u8]>) -> [u8; 8] {
    use mandrake_view::media::{ENGINE_SHADING_SLOTS, ShadingSlot, build_tinting_shader};
    let slot3 = media
        .and_then(ShadingSlot::media_reflection)
        .unwrap_or(ENGINE_SHADING_SLOTS[3]);
    build_tinting_shader(slot3, 0, [0; 3], slot3)
}

fn reflection_ripple(
    waves: Option<&crate::tfl_landscape::Waves>,
    time: FrameTime,
    o: &DrawnObject,
    still: bool,
) -> i32 {
    match waves {
        Some(w) if still => w.offset_at(time, o.world[0], o.world[1])[2],
        _ => 0,
    }
}

impl TflScene {
    pub(crate) fn model_pick(&self) -> &crate::tfl_models::ModelPick {
        self.models.pick()
    }

    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &RenderPipelineConfig,
        interface_format: wgpu::TextureFormat,
        map: &TflMap,
        replay: Option<&TflReplay>,
    ) -> Self {
        let reflection_tint = reflection_tint(
            map.tfl_mesh
                .media_tag
                .and_then(|id| map.tag(b"medi", id))
                .as_deref(),
        );
        info!(
            "TFL reflections' shading (k, p, q, r): {:?}",
            &reflection_tint[..4]
        );
        let mut terrain_renderer = TerrainRenderer::new(device, queue, config);
        terrain_renderer.enable_tfl_buckets(device);
        let model_renderer = ModelRenderer::new(device, queue, config);
        let billboard_renderer =
            BillboardRenderer::with_interface_format(device, queue, config, interface_format);
        let cells = replay
            .and_then(TflReplay::cells)
            .unwrap_or(&map.tfl_mesh.cells);
        let terrain_mesh = crate::tfl_landscape::tfl_terrain_mesh(device, map, cells);
        let waves = crate::tfl_landscape::TflWaves::new(map, cells);
        let mut models = TflModels::load(device, &model_renderer, map, reflection_tint);
        models.prepare(
            device,
            queue,
            &model_renderer,
            FrameTime::whole(replay.map_or(0, TflReplay::tick)),
        );
        let ground = map.texture.as_ref().map(|tex| {
            let texels = tfl_landscape_texels(tex);
            let texture = terrain_renderer.create_landscape_texture(
                device,
                queue,
                texels.width,
                texels.height,
                &texels.rgba(&[]),
            );
            TflGround {
                stains: TflStains::new(map, texels),
                texture,
                shadows: Vec::new(),
            }
        });
        let mut sprites = match replay {
            Some(r) => {
                let mut sprites = TflSprites::new(map);
                for &(coll, seq, look) in r.shapes() {
                    sprites.preload(map, coll, seq, look);
                }
                for o in crate::world_view::tfl::objects(r, FRACTION_ONE) {
                    sprites.decode(map, &o);
                }
                sprites
            }
            None => tfl_sprites(map, LOCAL_TEAM),
        };
        let mut atlas_images = 0;
        sprites.take_grown(&mut atlas_images);
        let atlas = create_atlas_from_images_with_offsets(
            device,
            queue,
            &billboard_renderer.texture_bind_group_layout,
            &sprites.out.images,
            &sprites.out.offsets,
        );
        Self {
            terrain_renderer,
            model_renderer,
            billboard_renderer,
            terrain_mesh,
            waves,
            ground,
            models,
            sprites,
            atlas,
            atlas_images,
            drawn: Vec::new(),
            cell_flags: cells.to_vec(),
            ground_tick: replay.map(TflReplay::tick),
            translucent_batch: TranslucentBatch::new(device),
            first_pass_batch: TranslucentBatch::new(device),
            prepared: None,
            cells: (map.tfl_mesh.cell_width, map.tfl_mesh.cell_height),
            model_reflections: mandrake_render::ReflectionBatch::new(device),
            reflection_tint,
            ripple: crate::tfl_landscape::Waves::of(map),
            ground_glow: true,
            glow: Vec::new(),
            lights: Vec::new(),
            fires: None,
            interface_colors: None,
            markers: TflMarkers::default(),
            framed: Vec::new(),
            weather: None,
            weather_tried: false,
            look: TflLook::tfl_1_5(),
            reflection_edge: None,
            detail_applied: false,
            water: None,
            water_followed: false,
            water_time: FrameTime::whole(0),
            local: None,
            local_tried: false,
            fixed_frames: true,
            glide: false,
            bolts: TflBolts::new(),
            bolts_random: LocalRandom::new(0),
            view_size: FIRST_VIEW_SIZE,
            shadows: None,
            sight_cache: None,
            lit_projectiles: std::collections::HashSet::new(),
        }
    }

    pub(crate) fn consumed(&self) -> (usize, usize, usize, usize) {
        let (stains, marks) = self
            .ground
            .as_ref()
            .map_or((0, 0), |g| (g.stains.painted, g.stains.marks_painted));
        let (orders, starts) = self.local.as_ref().map_or((0, 0), TflLocal::consumed);
        (stains, marks, orders, starts)
    }

    pub(crate) fn set_glide(&mut self, smoothed: bool) {
        self.glide = smoothed;
    }

    pub(crate) fn take_local_sounds(&mut self) -> Vec<mandrake_sim::combat::SequenceSound> {
        self.local
            .as_mut()
            .map(TflLocal::take_waiting_sounds)
            .unwrap_or_default()
    }

    pub(crate) fn set_frame_per_prepare(&mut self) {
        self.fixed_frames = false;
        if let Some(l) = self.local.as_mut() {
            l.fixed_frames = false;
        }
    }

    pub(crate) fn set_look(&mut self, look: TflLook) {
        self.look = look;
        self.prepared = None;
        self.detail_applied = false;
        self.water_followed = false;
    }

    fn follow_water(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, map: &TflMap) {
        if std::mem::replace(&mut self.water_followed, true) {
            return;
        }
        self.water = self.look.water.as_ref().and_then(|settings| {
            crate::tfl_water::TflWater::new(
                &crate::tfl_water::WaterGpu {
                    device,
                    queue,
                    terrain: &self.terrain_renderer,
                    billboards: &self.billboard_renderer,
                },
                map,
                &self.cell_flags,
                settings,
                self.ground.as_ref().map(|g| &g.stains.texels),
            )
        });
    }

    fn follow_soft_edge(&mut self, device: &wgpu::Device, (w, h): (u32, u32)) {
        if !self.look.view.reflection_soft_edge {
            self.reflection_edge = None;
            return;
        }
        match &mut self.reflection_edge {
            Some((size, _)) if *size == (w, h) => {}
            Some((size, edge)) => {
                edge.resize(device, w, h);
                *size = (w, h);
            }
            None => {
                let config = RenderPipelineConfig {
                    width: w,
                    height: h,
                    ..self.terrain_renderer.config.clone()
                };
                self.reflection_edge = Some((
                    (w, h),
                    mandrake_render::ReflectionEdge::new(device, &config),
                ));
            }
        }
    }

    pub(crate) fn set_markers(
        &mut self,
        colors: Option<&InterfaceColorTable>,
        markers: TflMarkers,
    ) {
        if self.interface_colors.is_none() {
            self.interface_colors = colors.cloned();
        }
        if markers.framed != self.markers.framed || markers.ground != self.markers.ground {
            self.prepared = None;
        }
        self.markers = markers;
    }

    pub(crate) fn view_outline(
        &self,
        camera: &Camera,
        size: (u32, u32),
    ) -> crate::minimap::OutlineView {
        crate::tfl_minimap::view_outline(camera, size, self.cells, &self.cell_flags)
    }

    fn eye_world(&self, camera: &Camera) -> (i32, i32) {
        let [ex, _, ez] = camera.eye_position();
        let half_w = self.cells.0 as f32 * TFL_CELL_SIZE * 0.5;
        let half_d = self.cells.1 as f32 * TFL_CELL_SIZE * 0.5;
        (
            ((half_w - ex) * WORLD_PER_RENDER_UNIT) as i32,
            ((ez + half_d) * WORLD_PER_RENDER_UNIT) as i32,
        )
    }

    pub(crate) fn local_random_mut(&mut self) -> &mut LocalRandom {
        shared_random(&mut self.local, &mut self.ground, &mut self.bolts_random)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_changed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        map: &TflMap,
        replay: Option<&TflReplay>,
        camera: &Camera,
        world: u64,
        fraction: i32,
    ) {
        if self
            .prepared
            .as_ref()
            .is_some_and(|(w, c, f)| *w == world && c == camera && *f == fraction)
        {
            return;
        }
        self.prepare(device, queue, map, replay, camera, fraction);
        self.prepared = Some((world, camera.clone(), fraction));
    }

    fn follow_ground(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        replay: Option<&TflReplay>,
    ) {
        let Some(r) = replay else {
            return;
        };
        if self.ground_tick == Some(r.tick()) {
            return;
        }
        self.ground_tick = Some(r.tick());
        let Some(cells) = r.cells().filter(|c| c.len() == self.cell_flags.len()) else {
            return;
        };
        if cells == self.cell_flags.as_slice() {
            return;
        }
        let changed = crate::tfl_landscape::changed_ground(&self.cell_flags, cells, self.cells);
        self.cell_flags.copy_from_slice(cells);
        if changed.is_empty() {
            return;
        }
        info!(
            "TFL landscape: {} vertices moved at tick {}",
            changed.len(),
            r.tick()
        );
        let moved: Vec<(u32, [f32; 3])> = changed
            .iter()
            .map(|&(i, z)| (i, crate::tfl_landscape::ground_position(i, z, self.cells)))
            .collect();
        self.terrain_mesh.move_ground_vertices(queue, &moved);
        if let Some(w) = self.waves.as_mut() {
            w.set_ground(&changed);
        }
        if let Some(w) = self.water.as_mut() {
            w.follow_ground(device, queue, &self.cell_flags);
        }
    }

    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        map: &TflMap,
        replay: Option<&TflReplay>,
        camera: &Camera,
        fraction: i32,
    ) {
        if let (Some(g), Some(shadows)) = (self.ground.as_mut(), self.shadows.take()) {
            g.shade(queue, shadows);
        }
        let eye = self.eye_world(camera);
        let time = replay.map_or(0, TflReplay::tick);
        let frame_time = replay.map_or(FrameTime::whole(0), |r| r.time_at(fraction));
        self.follow_water(device, queue, map);
        self.follow_ground(device, queue, replay);
        self.water_time = frame_time;
        let frame_objects = replay.map(|r| crate::world_view::tfl::objects(r, fraction));
        if let (Some(w), Some(r), Some(objs)) =
            (self.water.as_mut(), replay, frame_objects.as_deref())
            && w.has_ripples()
            && let Some(sim) = r.sim()
        {
            let water_view = crate::world_view::WorldView {
                tick: r.tick(),
                engine_stopped: r.stop().is_some(),
                units: crate::world_view::tfl::units_for_water(sim, objs),
                ..crate::world_view::WorldView::default()
            };
            w.gather(&water_view);
        }
        if let Some(r) = replay.filter(|_| !self.local_tried) {
            self.local_tried = true;
            self.local = TflLocal::new(map, r, &mut self.sprites, false);
        }
        if let Some(r) = replay.filter(|r| self.local.is_none() && r.has_local_group_starts()) {
            self.local = TflLocal::new(map, r, &mut self.sprites, true);
            if let Some(l) = self.local.as_mut() {
                let held = match self.ground.as_mut() {
                    Some(g) => &mut g.stains.random,
                    None => &mut self.bolts_random,
                };
                std::mem::swap(l.local.random_mut(), held);
            }
        }
        if let (Some(l), Some(r)) = (self.local.as_mut(), replay) {
            l.fixed_frames = self.fixed_frames;
            l.load_started(map, r, &mut self.sprites);
        }
        if let Some(r) = replay {
            self.paint_ground(queue, map, r);
        }
        if let Some(w) = self.waves.as_mut() {
            w.update(queue, &mut self.terrain_mesh, frame_time);
        }
        if let Some(sim) = replay.and_then(TflReplay::sim) {
            self.models
                .follow(device, &self.model_renderer, map, sim.model_instances());
        }
        self.models
            .prepare(device, queue, &self.model_renderer, frame_time);
        self.models.sort(
            device,
            queue,
            &mut self.translucent_batch,
            &mut self.first_pass_batch,
            camera,
        );
        self.models
            .reflect(device, queue, &mut self.model_reflections, camera);
        let mut objects = self.objects(map, replay, frame_objects, fraction);
        if let (Some(_), Some(sim)) = (self.ground.as_ref(), replay.and_then(TflReplay::sim)) {
            let sprites = &self.sprites;
            let detail = self.look.detail_shadows();
            self.shadows = Some(
                objects
                    .iter()
                    .filter_map(|(o, rule)| object_shadow(sprites, sim, (o, rule), detail))
                    .collect(),
            );
        }
        for l in &self.markers.ground {
            if let Some(o) = self
                .sprites
                .local_sprite(map, l.collection, l.sequence, l.point)
            {
                objects.push((o, ObjectRule::PLAIN));
            }
        }
        if let Some(r) = replay {
            self.follow_effects(map, r, camera, (eye, fraction));
        }
        self.grow_atlas(device, queue);
        let axes = camera.view_axes();
        let rows = EngineViewRows::from_camera_axes(axes.0, axes.1, axes.2);
        self.glow.clear();
        if let (true, Some(shadows), Some(sim)) = (
            self.ground_glow,
            self.shadows.as_mut(),
            replay.and_then(TflReplay::sim),
        ) {
            if let Some(l) = self.local.as_ref()
                && let Some(sprites) = l.sprites.as_ref().filter(|s| s.has_overlays())
            {
                let eye = world_frame(self.cells)
                    .to_engine(camera.eye_position())
                    .map(|c| c as i32);
                self.glow
                    .extend(sprites.overlays(&l.surfaces, eye, rows.depth));
            }
            self.glow.extend(
                self.bolts
                    .overlays(&|x, y| sim.mesh_height(x, y).map_or(0, |h| i32::from(h as i16))),
            );
            shadows.extend(self.glow.iter().cloned());
        }
        self.follow_lights(map, replay, &objects, camera);
        self.drawn.clear();
        for (o, rule) in &objects {
            let still = replay
                .and_then(TflReplay::sim)
                .and_then(|sim| sim.objects.get(usize::try_from(o.object).ok()?))
                .is_none_or(|live| live.velocity == [0; 3]);
            let reflection = (rule.reflected && !rule.surface_only).then(|| SurfaceReflection {
                surface: 0,
                clip: rule.in_water,
                tint: self.reflection_tint,
                ripple: reflection_ripple(self.ripple.as_ref(), frame_time, o, still),
            });
            let frame = ObjectFrame {
                sprites: &TflObjectSprites {
                    sprites: &self.sprites,
                    atlas: &self.atlas,
                    reflection,
                },
                camera_ground: eye,
                axes,
                tick: time as i32,
                local_time: 0,
                stopped: false,
            };
            if let Some(ObjectDraw {
                own: Some(own),
                reflection,
                ..
            }) = object_draw(o, &frame, &rows)
            {
                self.drawn.push((own, reflection, *rule));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        targets: &WorldTargets<'_>,
        size: (u32, u32),
        camera: &Camera,
    ) {
        let lists = self.lists(camera, size);
        self.follow_soft_edge(device, size);
        if let Some(w) = self.water.as_mut() {
            w.before_passes(
                device,
                queue,
                encoder,
                (size, self.water_time),
                self.fixed_frames,
            );
        }
        self.look.poll_reloads();
        if !std::mem::replace(&mut self.detail_applied, true) {
            self.terrain_renderer
                .set_detail(device, queue, self.look.detail.as_deref());
        }
        let shadows = self.shadows.take();
        let has_shadows = shadows.is_some();
        let ground = &mut self.ground;
        let models = &self.models;
        let frame = WorldFrame::prepare(
            device,
            queue,
            &self.terrain_renderer,
            &mut self.billboard_renderer,
            WorldFeed {
                rules: RULES,
                camera,
                size,
                look: self.look.world_look(&self.lights),
                lists,
                shadows: shadows.unwrap_or_default(),
            },
            &mut None,
            |shadows| {
                if let Some(g) = ground.as_mut().filter(|_| has_shadows) {
                    g.shade(queue, shadows);
                }
            },
            || {
                let draws = models.draws();
                WorldModels {
                    category2: draws.clone(),
                    translucent: draws,
                    reflections: models.reflection_draws(),
                    ..Default::default()
                }
            },
        );
        frame.encode(
            &WorldGpu {
                terrain_renderer: &self.terrain_renderer,
                terrain_mesh: &self.terrain_mesh,
                model_renderer: &self.model_renderer,
                billboard_renderer: &self.billboard_renderer,
                atlases: SpriteAtlases {
                    atlas: &self.atlas.bind_group,
                    team: None,
                },
                category2: &self.first_pass_batch,
                reflections: &self.model_reflections,
                soft_edge: self.reflection_edge.as_ref().map(|(_, edge)| edge),
                translucent: &self.translucent_batch,
            },
            encoder,
            targets,
            self.ground.as_ref().map(|g| &g.texture.bind_group),
            self.water.as_ref().map(crate::tfl_water::TflWater::shading),
            WorldPasses {
                rule: RULES.pass,
                clear: wgpu::Color::BLACK,
                skip: PassSkips::default(),
            },
            None,
        );
    }

    fn paint_ground(&mut self, queue: &wgpu::Queue, map: &TflMap, r: &TflReplay) {
        let taken = self
            .local
            .as_mut()
            .map(|l| std::mem::replace(l.local.random_mut(), LocalRandom::new(0)));
        let local = &mut self.local;
        let mut start = |before: Option<u32>, random: &mut LocalRandom| {
            if let Some(l) = local.as_mut() {
                l.start_groups(r, before, random);
            }
        };
        let back = match (self.ground.as_mut(), taken) {
            (Some(g), Some(mut random)) => {
                std::mem::swap(&mut g.stains.random, &mut random);
                g.paint(queue, map, r, &mut start);
                std::mem::swap(&mut g.stains.random, &mut random);
                Some(random)
            }
            (Some(g), None) => {
                g.paint(queue, map, r, &mut |_, _| {});
                None
            }
            (None, Some(mut random)) => {
                start(None, &mut random);
                Some(random)
            }
            (None, None) => None,
        };
        if let (Some(l), Some(random)) = (self.local.as_mut(), back) {
            *l.local.random_mut() = random;
        }
    }

    fn objects(
        &mut self,
        map: &TflMap,
        replay: Option<&TflReplay>,
        frame_objects: Option<Vec<DrawnObject>>,
        fraction: i32,
    ) -> Vec<(DrawnObject, ObjectRule)> {
        let (cols, rows) = self.cells;
        let cells = &self.cell_flags;
        let cell_at = |(x, y): (i32, i32)| {
            let (c, r) = (x >> 10, y >> 10);
            (c >= 0 && r >= 0 && (c as u32) < cols && (r as u32) < rows)
                .then(|| cells.get(r as usize * cols as usize + c as usize))
                .flatten()
        };
        let mut objects: Vec<(DrawnObject, ObjectRule)> = Vec::new();
        self.framed.clear();
        let glides = self.glide && crate::sprite_interp::enabled() && glide_allowed();
        match (replay, frame_objects) {
            (Some(r), Some(raw_objects)) => {
                let tick = r.tick();
                let observer = r.observer();
                if self
                    .sight_cache
                    .as_ref()
                    .is_none_or(|(t, o, _)| *t != tick || *o != observer)
                {
                    let computed = r
                        .sim()
                        .and_then(|s| crate::world_view::tfl::film_sight(s, observer));
                    self.sight_cache = Some((tick, observer, computed));
                }
                let sight = self.sight_cache.as_ref().and_then(|(_, _, s)| s.as_ref());
                let units = if self.markers.framed.is_empty() {
                    Vec::new()
                } else if let Some(sim) = r.sim() {
                    let framed_ids: Vec<usize> = self
                        .markers
                        .framed
                        .iter()
                        .filter(|f| f.monster)
                        .map(|f| f.object)
                        .collect();
                    crate::world_view::tfl::framed_units(sim, &raw_objects, &framed_ids)
                } else {
                    Vec::new()
                };
                objects.reserve(raw_objects.len());
                for mut drawn in raw_objects {
                    let flags = drawn.flags;
                    let [x, y, _] = drawn.world;
                    let index = drawn.object as usize;
                    let seen = match sight {
                        Some(s) => {
                            let cell = (x >> 10, y >> 10);
                            RULES.sight.rule(flags, s.bits_at((x, y)), s.explored(cell))
                        }
                        None => Some(true),
                    };
                    let cell = cell_at((x, y));
                    let rule = ObjectRule::of(RULES, flags, seen, cell.map(|c| c.flags), || {
                        r.sim().is_some_and(|sim| in_water(sim, cell, x, y))
                    })
                    .map(|rule| {
                        if rule.reflected {
                            rule.under_model(self.models.pick().height_at(x, y), drawn.world[2])
                        } else {
                            rule
                        }
                    });
                    let Some(rule) = rule else {
                        continue;
                    };
                    if self.sprites.decode(map, &drawn) {
                        let live = r.sim().and_then(|sim| sim.objects.get(index));
                        drawn.frame = self.sprites.doubled_frame(&drawn, live, fraction);
                        if glides {
                            glide(&mut drawn, r, fraction);
                        }
                        let framed = self.markers.framed.iter().find(|f| f.object == index);
                        if let (Some(frame), true) = (framed, rule.own)
                            && let Some((bounds, monster)) = framed_of(&units, r, frame)
                        {
                            self.framed.push(Framed {
                                object: index,
                                position: drawn.position,
                                bounds,
                                flipped: flags & OBJECT_FLIPPED != 0,
                                monster,
                                hovered_only: frame.hovered_only,
                                frame: frame.frame,
                                submerged: rule.in_water,
                            });
                        }
                        objects.push((drawn, rule));
                    }
                }
            }
            _ => objects.extend(self.sprites.out.sprites.iter().map(|o| {
                let cell = cell_at((o.world[0], o.world[1])).map(|c| c.flags);
                let rule = ObjectRule {
                    reflected: ObjectRule::reflects(RULES, cell),
                    ..ObjectRule::PLAIN
                };
                (o.clone(), rule)
            })),
        }
        objects
    }

    fn follow_lights(
        &mut self,
        map: &TflMap,
        replay: Option<&TflReplay>,
        objects: &[(DrawnObject, ObjectRule)],
        camera: &Camera,
    ) {
        self.lights.clear();
        let Some(lighting) = self.look.lit().copied() else {
            return;
        };
        let frame = world_frame(self.cells);
        let fires = self
            .fires
            .get_or_insert_with(|| crate::lighting::tfl::TflFires::of(&map.archive));
        self.lit_projectiles.clear();
        if let Some(sim) = replay.and_then(TflReplay::sim) {
            self.lit_projectiles.extend(
                sim.projectiles
                    .iter()
                    .filter(|(_, p)| p.definition.is_some_and(|d| fires.lights(d)))
                    .map(|(_, p)| p.object),
            );
        }
        let fire_lights: Vec<mandrake_render::PointLight> = if self.lit_projectiles.is_empty() {
            Vec::new()
        } else {
            objects
                .iter()
                .filter(|(o, rule)| rule.own && !o.hidden && o.sequence != 0xff)
                .filter(|(o, _)| {
                    usize::try_from(o.object).is_ok_and(|k| self.lit_projectiles.contains(&k))
                })
                .map(|(o, _)| crate::lighting::fire_light(frame, o.world, lighting.light_intensity))
                .collect()
        };
        let (surfaces, sprites) = match self.local.as_ref() {
            Some(l) => (l.surfaces.as_slice(), l.sprites.as_ref()),
            None => (&[][..], None),
        };
        let none = crate::projectile_local::LocalSprites::default();
        self.lights = crate::lighting::scene_lights(
            surfaces,
            sprites.unwrap_or(&none),
            frame,
            camera.eye_position(),
            &fire_lights,
            lighting.light_intensity,
        );
    }

    fn follow_effects(
        &mut self,
        map: &TflMap,
        r: &TflReplay,
        camera: &Camera,
        (eye, fraction): ((i32, i32), i32),
    ) {
        let observer = observer_view(camera, self.cells).observer;
        let random = shared_random(&mut self.local, &mut self.ground, &mut self.bolts_random);
        if !self.weather_tried {
            self.weather_tried = true;
            self.weather = TflWeather::new(map, r, &mut self.sprites, observer, random);
        }
        if let Some(w) = self.weather.as_mut() {
            w.follow(r, observer, random);
            w.fraction = fraction;
        }
        let region = tfl_region(camera, self.view_size, self.cells, &self.cell_flags);
        let cols = self.cells.0 as i32;
        let wind = self
            .weather
            .as_ref()
            .and_then(|w| w.wind.as_ref())
            .map(crate::particles::LocalWind::velocity);
        let bolts = &mut self.bolts;
        let sprites = &mut self.sprites;
        let cell_flags = &self.cell_flags;
        let mut lightning = |random: &mut LocalRandom| {
            if let Some(sim) = r.sim() {
                bolts.follow(
                    map,
                    sim,
                    r.tick(),
                    &region,
                    cell_flags,
                    cols,
                    sprites,
                    random,
                );
            }
        };
        if let Some(l) = self.local.as_mut() {
            let [ex, ey, ez] = camera.eye_position();
            let (_, _, forward) = camera.view_axes();
            let frame = world_frame(self.cells);
            let in_front = |w: [i32; 3]| {
                let p = frame.from_engine(w.map(|c| c as f32));
                let d = [p[0] - ex, p[1] - ey, p[2] - ez];
                d[0] * forward[0] + d[1] * forward[1] + d[2] * forward[2]
                    > 0x22 as f32 / WORLD_PER_RENDER_UNIT
            };
            l.follow(
                r,
                &region,
                cell_flags,
                cols,
                eye,
                &in_front,
                wind,
                &mut lightning,
            );
            l.surfaces = l.surfaces(r, cell_flags, cols, fraction);
        } else {
            match self.ground.as_mut() {
                Some(g) => lightning(&mut g.stains.random),
                None => lightning(&mut self.bolts_random),
            }
        }
    }

    fn grow_atlas(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.sprites.take_grown(&mut self.atlas_images) {
            info!(
                "TFL: {} bitmaps; the atlas is made again",
                self.atlas_images
            );
            self.atlas = grow_atlas_from_images_with_offsets(
                device,
                queue,
                &self.billboard_renderer.texture_bind_group_layout,
                &self.atlas,
                &self.sprites.out.images,
                &self.sprites.out.offsets,
            );
            if let Some(w) = self.weather.as_mut() {
                w.sprites = None;
            }
            if let Some(l) = self.local.as_mut() {
                l.sprites = None;
            }
            self.bolts.sprites = None;
        }
        if let Some(w) = self.weather.as_mut().filter(|w| w.sprites.is_none()) {
            w.make_sprites(&self.atlas);
        }
        if let Some(l) = self.local.as_mut().filter(|l| l.sprites.is_none()) {
            l.make_sprites(&self.atlas);
        }
        if self.bolts.wants_sprites() {
            self.bolts.make_sprites(&self.atlas);
        }
    }

    fn lists(&mut self, camera: &Camera, size: (u32, u32)) -> WorldLists {
        let mut lists = WorldLists {
            objects: Vec::with_capacity(self.drawn.len()),
            ..Default::default()
        };
        for &(b, reflection, rule) in &self.drawn {
            rule.billboards(RULES, b, reflection, &mut lists.objects);
        }
        self.view_size = size;
        lists.lightning = self.bolts.billboards(
            camera,
            size,
            &self.atlas,
            world_frame(self.cells),
            self.reflection_tint,
        );
        if let Some(l) = self.local.as_ref() {
            lists.local_projectiles = l.billboards(
                camera,
                &self.atlas,
                world_frame(self.cells),
                self.reflection_tint,
            );
        }
        if let Some(w) = self.weather.as_ref() {
            lists.particles = w.billboards(camera, size, self.cells, &self.cell_flags, &self.atlas);
        }
        if let Some(colors) = self.interface_colors.as_ref() {
            for f in &self.framed {
                lists.overlays.extend(tfl_selection_overlays(
                    f.position,
                    f.bounds,
                    f.flipped,
                    f.monster,
                    TflFrameStyle {
                        hovered_only: f.hovered_only,
                        frame: f.frame,
                        thick: self.look.view.tfl_selection_boxes,
                        submerged: f.submerged,
                    },
                    colors,
                    camera,
                    size,
                ));
            }
            if let Some(band) = self.markers.band {
                lists
                    .overlays
                    .extend(band_rectangle_overlays(band, colors, size));
            }
            if let Some(f) = self.markers.gesture.as_ref() {
                lists.overlays.extend(facing_arrows_at(
                    &f.gesture,
                    f.units.iter().copied(),
                    colors,
                ));
            }
        }
        lists
    }
}

fn headless_tfl_controls(args: &Args) -> crate::controls::Controls {
    crate::conf::load_table(
        crate::conf::to_read(args.conf.as_deref(), true),
        &["tfl", "controls"],
        crate::controls::tfl::tfl_defaults,
        crate::controls::Controls::tfl_with_override,
    )
    .0
}

pub(crate) fn render_tfl_frame(
    map: &TflMap,
    replay: Option<&TflReplay>,
    camera: &Camera,
    args: &Args,
    output_path: &Path,
    hud: Option<&mut TflHud>,
) -> anyhow::Result<()> {
    let camera = &crate::screenshot::frame_camera(camera);
    let (width, height) = (args.width, args.height);
    pollster::block_on(async {
        let (device, queue) = headless_device("TFL Headless Device").await?;
        let config = RenderPipelineConfig {
            width,
            height,
            target_format: wgpu::TextureFormat::Rgba8UnormSrgb,
            depth_format: Some(wgpu::TextureFormat::Depth32Float),
        };
        let interface_format = BillboardRenderer::interface_view_format(config.target_format);
        let mut scene = TflScene::new(&device, &queue, &config, interface_format, map, replay);
        scene.set_look(TflLook::new(args, map, &ViewSettings::of(args), true));
        let controls = args.caps_lock.then(|| headless_tfl_controls(args));
        let caps_lock = controls
            .as_ref()
            .map(|c| crate::interface_state::caps_lock_mode(c.options.caps_lock));
        if args.hover.is_some() || caps_lock.is_some() {
            let sim = replay.and_then(TflReplay::sim);
            let hovered = args.hover.as_deref().and_then(|h| {
                let &[x, y] = h else {
                    return None;
                };
                let world = crate::tfl_window::drawn_world(replay, FRACTION_ONE);
                let view = crate::tfl_input::View {
                    camera,
                    size: (width, height),
                    mesh: &map.mesh,
                    world: &world,
                    models: None,
                };
                let hovered = crate::tfl_input::monster_under(&view, (x as i32, y as i32));
                println!("TFL hover at ({x}, {y}): monster {hovered:?}");
                hovered
            });
            let markers = crate::tfl_window::tfl_markers(None, sim, &[], hovered, caps_lock);
            if caps_lock.is_some() {
                println!(
                    "TFL Caps Lock {caps_lock:?}: {} framed",
                    markers.framed.len()
                );
            }
            scene.set_markers(hud.as_deref().and_then(TflHud::interface_colors), markers);
        }
        scene.prepare(&device, &queue, map, replay, camera, FRACTION_ONE);
        let outline = scene.view_outline(camera, (width, height));
        let target = Offscreen::new(&device, (width, height), interface_format);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("TFL Render Encoder"),
        });
        scene.encode(
            &device,
            &queue,
            &mut encoder,
            &target.views(),
            (width, height),
            camera,
        );
        let mut pixels = target.read(&device, &queue, encoder)?;
        if let Some(hud) = hud {
            if let Some(controls) = controls.as_ref() {
                hud.set_held_keys(controls, &Default::default(), true);
            }
            let mut frame = mandrake_tag::font::Bitmap32::new(width as i32, height as i32, 0);
            for (p, c) in frame.pixels.iter_mut().zip(pixels.as_chunks::<4>().0) {
                *p = 0xff00_0000 | u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
            }
            hud.compose(&mut frame, Some(&outline), 0);
            pixels = frame.to_rgba();
        }
        image::save_buffer(
            output_path,
            &pixels,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )?;
        anyhow::Ok(())
    })
}

#[cfg(test)]
mod tests {
    use crate::tfl_objects::render_position;
    use mandrake_tag::mesh::MarkerEntry;

    #[test]
    fn objects_show_by_the_local_team_s_sight() {
        let sight_rule = |f, b, e| crate::world_view::ClientRules::TFL.sight.rule(f, b, e);
        assert_eq!(sight_rule(0, 0, true), None);
        assert_eq!(sight_rule(0, 1, true), Some(false));
        assert_eq!(sight_rule(0, 3, true), Some(true));
        assert_eq!(sight_rule(0x1000_0000, 3, true), Some(false));
        assert_eq!(sight_rule(0x100, 0, true), Some(true));
        assert_eq!(sight_rule(0x400_0100, 0, true), None);
        assert_eq!(sight_rule(0x200, 3, true), None);
        assert_eq!(sight_rule(0, 3, false), None);
    }

    #[test]
    fn the_reflection_shading_is_slot_3_from_the_media() {
        let mut header = vec![0u8; 0x80];
        assert_eq!(
            super::reflection_tint(Some(&header)),
            [255, 0, 0, 0, 0, 0, 0, 0]
        );
        header[0x58..0x5a].copy_from_slice(&0x80u16.to_be_bytes());
        header[0x5e..0x60].copy_from_slice(&0x8000u16.to_be_bytes());
        let tint = super::reflection_tint(Some(&header));
        assert_eq!(tint, super::reflection_tint(None));
        assert_eq!(tint[..4], [127, 0, 0, 63]);
    }

    #[test]
    fn a_marker_is_placed_at_1024_world_units_a_cell() {
        let mut m = MarkerEntry {
            flags: 0,
            marker_type: 3,
            palette_index: 0,
            marker_id: 0,
            difficulty_flags: 0,
            x: 1024 * 10,
            y: 1024 * 20,
            z: 512,
            yaw: 0,
            pitch: 0,
            roll: 0,
            type_data: [0; 32],
        };
        assert_eq!(render_position(&m, (80, 80)), [60.0, 1.0, -40.0]);
        m.x = 0;
        assert_eq!(render_position(&m, (80, 80))[0], 80.0);
    }
}
