use crate::media;
use crate::particles;
use crate::projectile_local;
use crate::region_view::frame_view_parts;
use crate::scenery_billboards::ObjectShadow;
use crate::sprite_manager::ClientSpriteManager;
use mandrake_common::FourCC;
use mandrake_sim::Simulation;
use mandrake_tag::collection::CollectionTag;
use mandrake_tag::{MeshTag, TagArchive};
use mandrake_view::{billboard::BillboardInstance, camera::Camera};
use tracing::debug;

pub(crate) fn particle_tags(
    small_archive: &TagArchive,
    large_archive: Option<&TagArchive>,
    mesh_tag: &MeshTag,
) -> Option<(particles::ParticleDefinition, FourCC)> {
    let tag = |group: &[u8; 4], id: FourCC| {
        crate::map_load::find_tag_data(small_archive, large_archive, *group, id, false)
    };
    let mesh_data = tag(b"mesh", mesh_tag.mesh_id)?;
    let definition = particles::definition_of_mesh(&mesh_data, tag)?;
    let reference = definition.collection_reference;
    let collection = tag(b"core", reference)
        .and_then(|core| Some(FourCC::new(core.get(0..4)?.try_into().ok()?)))
        .filter(|id| !matches!(u32::from_be_bytes(id.0), 0 | 0xffff_ffff))
        .unwrap_or(reference);
    Some((definition, collection))
}

pub(crate) fn load_local_projectiles(
    small_archive: &TagArchive,
    large_archive: Option<&TagArchive>,
    sim: &Simulation,
    mesh_tag: &MeshTag,
) -> Option<projectile_local::LocalProjectiles> {
    let catalog = &sim.engine.catalog;
    let tag = local_projectile_tags(small_archive, large_archive);
    let (media_effect, splashes) = ambient_local_sources(small_archive, large_archive, mesh_tag);
    let ambient: Vec<FourCC> = media_effect
        .map(|(s, _)| s)
        .into_iter()
        .chain(splashes)
        .map(|s| s.group)
        .chain(fire_local_groups(&tag, mesh_tag))
        .chain(marker_local_groups(mesh_tag))
        .collect();
    let mut local = projectile_local::LocalProjectiles::for_map(catalog, &ambient, &tag, 0).0;
    local.start_ambient(
        media_effect,
        sim.engine.fire_timer.is_some(),
        splashes,
        [
            i32::from(mesh_tag.cell_width) << 9,
            i32::from(mesh_tag.cell_height) << 9,
        ],
    );
    local.start_markers(&mesh_tag.palettes, &mesh_tag.markers);
    Some(local)
}

pub(crate) fn marker_local_groups(mesh_tag: &MeshTag) -> impl Iterator<Item = FourCC> + '_ {
    mesh_tag
        .palettes
        .iter()
        .filter(|p| p.marker_type == mandrake_tag::mesh::MarkerPaletteEntry::TYPE_WAYPOINT)
        .map(|p| p.tag_id)
}

pub(crate) fn fire_local_groups(
    tag: &projectile_local::TagPayload<'_>,
    mesh_tag: &MeshTag,
) -> impl Iterator<Item = FourCC> {
    let fire =
        tag(b"mesh", mesh_tag.mesh_id).is_some_and(|m| m.get(0x4c).is_some_and(|b| b & 1 != 0));
    projectile_local::FIRE_GROUPS
        .into_iter()
        .filter(move |_| fire)
}

pub(crate) fn ambient_local_sources(
    small_archive: &TagArchive,
    large_archive: Option<&TagArchive>,
    mesh_tag: &MeshTag,
) -> (
    Option<(projectile_local::AmbientSource, u16)>,
    Option<projectile_local::AmbientSource>,
) {
    let archives: Vec<&TagArchive> = large_archive
        .iter()
        .copied()
        .chain(std::iter::once(small_archive))
        .collect();
    let media_effect = media::MediaLocalEffect::of_mesh(mesh_tag, &archives).map(|e| {
        (
            projectile_local::AmbientSource {
                group: e.group,
                tries: e.tries,
                period: e.period,
            },
            e.chance,
        )
    });
    let splashes = particle_tags(small_archive, large_archive, mesh_tag).and_then(|(d, _)| {
        Some(projectile_local::AmbientSource {
            group: d.splash_group?,
            tries: d.splash_tries,
            period: d.splash_period,
        })
    });
    (media_effect, splashes)
}

pub(crate) fn build_ambient_local_projectiles(
    local: &mut projectile_local::LocalProjectiles,
    sim: &Simulation,
    mesh_tag: &MeshTag,
    map: Option<&media::MapMedia>,
    camera: &Camera,
    (size, bars): ((u32, u32), (i32, i32)),
    ticks: i32,
) {
    if !local.ambient_due(ticks) {
        return;
    }
    let Some((_, region)) = frame_view_parts(
        camera,
        mesh_tag,
        map,
        size,
        bars,
        media::local_time(sim.current_tick),
    ) else {
        return;
    };
    let Some(map) = map else {
        return;
    };
    let cells = (
        i32::from(mesh_tag.cell_width),
        i32::from(mesh_tag.cell_height),
    );
    let cell = |col: i32, row: i32| {
        let c = &map.cells[(row * cells.0 + col) as usize];
        (c.flags, c.media_height)
    };
    let system = sim.engine.particles.as_ref();
    let intensity =
        |x: i32, y: i32| system.map_or(0, |s| particles::splash_intensity(s, (x, y), cells));
    let before = local.ambient_count();
    local.build_ambient(&projectile_local::AmbientWorld {
        region: &region,
        cell: &cell,
        splash_intensity: &intensity,
    });
    debug!(
        "Ambient local projectiles at tick {}: rows {}..{}, {} new, {} live",
        sim.current_tick,
        region.first_row,
        region.last_row,
        local.ambient_count() - before,
        local.ambient_count()
    );
}

pub(crate) fn local_projectile_tags<'a>(
    small_archive: &'a TagArchive,
    large_archive: Option<&'a TagArchive>,
) -> impl Fn(&[u8; 4], FourCC) -> Option<Vec<u8>> + 'a {
    move |group, id| crate::map_load::find_tag_data(small_archive, large_archive, *group, id, false)
}

pub(crate) fn local_projectile_overlays_from_surfaces(
    surfaces: &[projectile_local::LocalSurface],
    sprite_mgr: &ClientSpriteManager,
    camera: &Camera,
    mesh_tag: &MeshTag,
) -> Vec<ObjectShadow> {
    let sprites = &sprite_mgr.local_projectile_sprites;
    if !sprites.has_overlays() || surfaces.is_empty() {
        return Vec::new();
    }
    let frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    let eye = frame.to_engine(camera.eye_position()).map(|c| c as i32);
    let (right, up, forward) = camera.view_axes();
    let depth =
        mandrake_view::projectile::EngineViewRows::from_camera_axes(right, up, forward).depth;
    sprites.overlays(surfaces, eye, depth)
}

pub(crate) fn local_surfaces_from(
    local: &projectile_local::LocalProjectiles,
    sprite_mgr: &ClientSpriteManager,
    sim: &Simulation,
    (camera, mesh_tag): (&Camera, &MeshTag),
    region: Option<&mandrake_view::view_region::ViewRegion>,
    fraction: i32,
) -> Vec<projectile_local::LocalSurface> {
    let frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    let eye = frame.to_engine(camera.eye_position()).map(|c| c as i32);
    local_surfaces(local, sprite_mgr, sim, mesh_tag, region, fraction, eye)
}

pub(crate) fn local_projectile_billboards_from_surfaces(
    surfaces: &[projectile_local::LocalSurface],
    sprite_mgr: &ClientSpriteManager,
    camera: &Camera,
    mesh_tag: &MeshTag,
) -> Vec<BillboardInstance> {
    if surfaces.is_empty() {
        return Vec::new();
    }
    let frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    let map = sprite_mgr.scenery_objects.media.as_deref();
    sprite_mgr.local_projectile_sprites.instances(
        surfaces,
        &sprite_mgr.atlas,
        frame,
        camera.view_axes(),
        &|reference, base, permutation| sprite_mgr.permuted(Some(&reference), base, permutation),
        (
            map.map_or_else(Default::default, |m| m.global_tint),
            &|core| map.map_or([0; 8], |m| m.reflection_tint(core)),
        ),
    )
}

pub(crate) fn local_surfaces(
    local: &projectile_local::LocalProjectiles,
    sprite_mgr: &ClientSpriteManager,
    sim: &Simulation,
    mesh_tag: &MeshTag,
    region: Option<&mandrake_view::view_region::ViewRegion>,
    fraction: i32,
    eye: [i32; 3],
) -> Vec<projectile_local::LocalSurface> {
    let height = |x: i32, y: i32| sim.mesh_height_at(x, y);
    let map = sprite_mgr.scenery_objects.media.as_deref();
    let media_height = |x: i32, y: i32| {
        map.and_then(|m| {
            let index = (y >> 9).clamp(0, i32::from(mesh_tag.cell_height) - 1)
                * i32::from(mesh_tag.cell_width)
                + (x >> 9).clamp(0, i32::from(mesh_tag.cell_width) - 1);
            m.cells.get(index as usize)
        })
        .map_or(0, |c| i32::from(c.media_height))
    };
    let reflective = |x: i32, y: i32| {
        map.is_some_and(|m| {
            let index = (y >> 9).clamp(0, i32::from(mesh_tag.cell_height) - 1)
                * i32::from(mesh_tag.cell_width)
                + (x >> 9).clamp(0, i32::from(mesh_tag.cell_width) - 1);
            !m.no_reflections
                && m.cells
                    .get(index as usize)
                    .is_some_and(|c| c.flags & media::CELL_FLAG_REFLECTIVE != 0)
        })
    };
    local.surfaces(
        fraction,
        &projectile_local::SurfaceView {
            eye,
            height: &height,
            media_height: &media_height,
            region,
            media_transparency: map.map_or(0, |m| m.reflection_transparency),
            reflective: &reflective,
            mirrors: (&sim.engine.fire.groups),
        },
    )
}

pub(crate) fn load_particle_field(
    small_archive: &TagArchive,
    large_archive: Option<&TagArchive>,
    mesh_tag: &MeshTag,
    sim: &Simulation,
    observer: (i32, i32),
) -> Option<particles::ParticleField> {
    let system = sim.engine.particles.as_ref()?;
    let (definition, collection) = particle_tags(small_archive, large_archive, mesh_tag)?;
    let coll =
        crate::map_load::find_tag_data(small_archive, large_archive, *b".256", collection, true)
            .and_then(|d| CollectionTag::from_bytes(&d).ok())?;
    let seq = coll.sequence(usize::try_from(definition.sequence).ok()?)?;
    let shape = particles::SequenceShape {
        views: seq.num_views as i16,
        frames: seq.frames_per_view as i16,
        ticks_per_frame: seq.ticks_per_frame,
    };
    let cells = (
        i32::from(mesh_tag.cell_width),
        i32::from(mesh_tag.cell_height),
    );
    let intensity = particles::observer_intensity(system, observer, cells);
    Some(particles::ParticleField::for_map(
        definition, shape, intensity, 0,
    ))
}

pub(crate) fn update_particle_field(
    field: &mut particles::ParticleField,
    sim: &Simulation,
    mesh_tag: &MeshTag,
    ticks: i32,
    observer: (i32, i32),
) {
    let world = &sim.engine;
    let Some(system) = world.particles.as_ref() else {
        return;
    };
    let cells = (
        i32::from(mesh_tag.cell_width),
        i32::from(mesh_tag.cell_height),
    );
    field.update(
        ticks,
        particles::observer_intensity(system, observer, cells),
        world.wind.as_ref().map(|w| w.velocity),
    );
}

pub(crate) fn particle_billboards(
    field: &particles::ParticleField,
    sprite_mgr: &ClientSpriteManager,
    sim: &Simulation,
    camera: &Camera,
    mesh_tag: &MeshTag,
    parts: (
        &mandrake_view::view_region::RegionView,
        &mandrake_view::view_region::ViewRegion,
    ),
    fraction: i32,
) -> Vec<BillboardInstance> {
    let Some(sprites) = sprite_mgr.particle_sprites.as_ref() else {
        return Vec::new();
    };
    let wind = (sim.engine.wind.as_ref()).map(|w| w.velocity);
    let surfaces = field.surfaces(parts.0, parts.1, sprites, fraction, wind);
    let global_tint = sprite_mgr
        .scenery_objects
        .media
        .as_deref()
        .map_or_else(Default::default, |m| m.global_tint);
    sprites.instances(
        &surfaces,
        &sprite_mgr.atlas,
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height),
        camera.view_axes(),
        sprites.shade(global_tint),
    )
}

fn media_cell<'a>(
    map: Option<&'a media::MapMedia>,
    mesh_tag: &MeshTag,
    (x, y): (i32, i32),
) -> Option<&'a mandrake_tag::Myth2Cell> {
    let map = map?;
    let index = (y >> 9).clamp(0, i32::from(mesh_tag.cell_height) - 1)
        * i32::from(mesh_tag.cell_width)
        + (x >> 9).clamp(0, i32::from(mesh_tag.cell_width) - 1);
    map.cells.get(index as usize)
}

pub(crate) fn lightning_surfaces(
    sprite_mgr: &ClientSpriteManager,
    sim: &Simulation,
    mesh_tag: &MeshTag,
    region: Option<&mandrake_view::view_region::ViewRegion>,
    random: &mut mandrake_tag::sound_manager::LocalRandom,
) -> crate::lightning_draw::LightningSurfaces {
    let world = &sim.engine;
    if world.lightning.records.is_empty() {
        return Default::default();
    }
    let map = sprite_mgr.scenery_objects.media.as_deref();
    let inside = |x: i32, y: i32| {
        (0..i32::from(mesh_tag.cell_width)).contains(&(x >> 9))
            && (0..i32::from(mesh_tag.cell_height)).contains(&(y >> 9))
    };
    let reflective = |x: i32, y: i32| {
        inside(x, y)
            && media_cell(map, mesh_tag, (x, y))
                .is_some_and(|c| c.flags & media::CELL_FLAG_REFLECTIVE != 0)
    };
    let media_height =
        |x: i32, y: i32| media_cell(map, mesh_tag, (x, y)).map_or(0, |c| c.media_height);
    sprite_mgr.lightning.surfaces(
        &world.lightning.records,
        &crate::lightning_draw::LightningView {
            region,
            reflective: &reflective,
            media_height: &media_height,
            media_transparency: map.map_or(0, |m| m.reflection_transparency),
        },
        random,
    )
}

pub(crate) fn lightning_billboards(
    surfaces: &crate::lightning_draw::LightningSurfaces,
    sprite_mgr: &ClientSpriteManager,
    camera: &Camera,
    mesh_tag: &MeshTag,
    aspect: f32,
) -> Vec<BillboardInstance> {
    if surfaces.segments.is_empty() {
        return Vec::new();
    }
    let frame =
        mandrake_view::model_surfaces::WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);
    let map = sprite_mgr.scenery_objects.media.as_deref();
    crate::lightning_draw::SegmentScreen::with_camera(frame, camera, aspect, |screen, axes| {
        crate::lightning_draw::instances(
            &sprite_mgr.lightning,
            &surfaces.segments,
            &sprite_mgr.local_projectile_sprites,
            &sprite_mgr.atlas,
            frame,
            axes,
            screen,
            &|reference, base| sprite_mgr.permuted(Some(&reference), base, 0),
            (
                map.map_or_else(Default::default, |m| m.global_tint),
                &|core| map.map_or([0; 8], |m| m.reflection_tint(core)),
            ),
        )
    })
}

pub(crate) fn lightning_overlays(
    surfaces: &crate::lightning_draw::LightningSurfaces,
    sprite_mgr: &ClientSpriteManager,
    sim: &Simulation,
) -> Vec<ObjectShadow> {
    if surfaces.overlays.is_empty() {
        return Vec::new();
    }
    crate::lightning_draw::overlays(
        &sprite_mgr.lightning,
        &surfaces.overlays,
        &sprite_mgr.local_projectile_sprites,
        &|x, y| sim.mesh_height_at(x, y),
    )
}
