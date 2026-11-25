use crate::corpse;
use crate::effects::{
    ambient_local_sources, fire_local_groups, local_projectile_tags, marker_local_groups,
    particle_tags,
};
use crate::interface_state;
use crate::lightning_draw;
use crate::media;
use crate::move_marker;
use crate::object_bounds;
use crate::particles;
use crate::projectile_local;
use crate::projfx_log;
use crate::scenery;
use crate::shadowgen;
use crate::team_colours;
use crate::unit_anim::{FrameVisual, SequenceVisual, ShadowSprite, UnitSpriteVisual, ViewSprite};
use mandrake_common::FourCC;
use mandrake_render::{TextureAtlas, create_atlas_from_images_with_offsets};
use mandrake_sim::Simulation;
use mandrake_tag::collection::{CollectionTag, CorePermutations};
use mandrake_tag::{
    ConnectorTag, InterfaceColorTable, MeshConnectorList, MeshTag, MonsterSequenceSlot, TagArchive,
};
use mandrake_view::{billboard::BillboardInstance, selection};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Clone, Debug)]
pub(crate) struct ConnectorVisual {
    pub(crate) definition: ConnectorTag,
    pub(crate) rect: (i16, i16),
    pub(crate) frame_index: usize,
}

pub(crate) struct ClientSpriteManager {
    pub(crate) atlas: TextureAtlas,
    pub(crate) scenery_visuals: BTreeMap<FourCC, scenery::SceneryVisual>,
    pub(crate) scenery_objects: scenery::SceneryObjects,
    pub(crate) dynamic_scenery: std::cell::RefCell<Option<(u64, Vec<usize>)>>,
    pub(crate) unit_visuals: BTreeMap<FourCC, UnitSpriteVisual>,
    pub(crate) connector_visual: Option<ConnectorVisual>,
    pub(crate) interface_colors: Option<InterfaceColorTable>,
    pub(crate) scenery_cores: BTreeMap<FourCC, media::CoreRef>,
    pub(crate) unit_cores: BTreeMap<FourCC, media::CoreRef>,
    pub(crate) unit_media: std::cell::RefCell<BTreeMap<u32, media::MonsterMediaFlags>>,
    pub(crate) placed_monster_flags: BTreeMap<u32, (i16, u32)>,
    permuted_atlas: BTreeMap<(FourCC, usize, usize), usize>,
    team_atlas: BTreeMap<(FourCC, i16, usize), usize>,
    team_rows: std::collections::BTreeSet<(FourCC, i16)>,
    flag_atlas: BTreeMap<(usize, i16), usize>,
    flag_core: Option<FourCC>,
    flag_scenery: std::collections::BTreeSet<FourCC>,
    pub(crate) team_cache: TeamSpriteCache,
    pub(crate) scenery_core_ids: BTreeMap<FourCC, FourCC>,
    pub(crate) unit_core_ids: BTreeMap<FourCC, FourCC>,
    pub(crate) collection_cores: BTreeMap<FourCC, FourCC>,
    pub(crate) collection_core_tints: BTreeMap<FourCC, media::CoreRef>,
    pub(crate) core_tints: BTreeMap<FourCC, media::CoreRef>,
    pub(crate) corpse_sprites: corpse::CorpseSprites,
    pub(crate) particle_sprites: Option<particles::ParticleSprites>,
    pub(crate) local_projectile_sprites: projectile_local::LocalSprites,
    pub(crate) lightning: lightning_draw::LightningDraw,
    pub(crate) mesh_effect_sprites: crate::mesh_effect_sprites::MeshEffectSprites,
    pub(crate) projfx_log: Option<projfx_log::ProjfxLog>,
    marker_sprites: BTreeMap<i16, move_marker::MarkerSprite>,
}

impl ClientSpriteManager {
    pub(crate) fn permuted(&self, core: Option<&FourCC>, base: usize, permutation: i16) -> usize {
        core.zip(usize::try_from(permutation).ok())
            .and_then(|(c, p)| self.permuted_atlas.get(&(*c, base, p)))
            .copied()
            .unwrap_or(base)
    }

    pub(crate) fn flag_sprite(&self, scenery: FourCC, base: usize, team: i16) -> Option<usize> {
        if !self.flag_scenery.contains(&scenery) {
            return None;
        }
        Some(
            self.flag_atlas
                .get(&(base, team))
                .copied()
                .unwrap_or_else(|| self.permuted(self.flag_core.as_ref(), base, 0)),
        )
    }

    pub(crate) fn team_sprite(
        &self,
        row: Option<(FourCC, i16)>,
        base: usize,
    ) -> Option<TeamSprite> {
        let (unit, player) = self.scenery_objects.team_colours.known_row(row?)?;
        if self.team_rows.contains(&(unit, player)) {
            return Some(TeamSprite::Atlas(
                self.team_atlas
                    .get(&(unit, player, base))
                    .copied()
                    .unwrap_or(base),
            ));
        }
        if !self.team_cache.rows.contains(&(unit, player)) {
            return None;
        }
        Some(match self.team_cache.index.get(&(unit, player, base)) {
            Some(&i) => TeamSprite::Cache(i),
            None => TeamSprite::Atlas(base),
        })
    }

    pub(crate) fn update_team_cache(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sim: &Simulation,
    ) {
        let world = &sim.engine;
        let team = &self.scenery_objects.team_colours;
        let cache = &mut self.team_cache;
        let missing = team.any_drawn_row(world, |r| {
            !self.team_rows.contains(&r)
                && cache.sources.contains_key(&r.0)
                && !cache.rows.contains(&r)
        });
        if !missing {
            return;
        }
        let drawn: std::collections::BTreeSet<(FourCC, i16)> = team
            .drawn_rows(world)
            .into_iter()
            .filter(|r| !self.team_rows.contains(r) && cache.sources.contains_key(&r.0))
            .collect();
        cache.rows = drawn;
        let (mut images, mut offsets) = (Vec::new(), Vec::new());
        cache.index = decode_team_rows(
            team,
            &cache.sources,
            cache.rows.iter().copied(),
            &mut images,
            &mut offsets,
        );
        let atlas = create_atlas_from_images_with_offsets(device, queue, layout, &images, &offsets);
        info!(
            "Team colours: {} rows taken after the map load drawn, {} sprite images in a {}x{} px texture",
            cache.rows.len(),
            images.len(),
            atlas.width,
            atlas.height
        );
        cache.atlas = Some(atlas);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        small_archive: &TagArchive,
        large_archive: Option<&TagArchive>,
        mesh_tag: &MeshTag,
        connectors: &MeshConnectorList,
        interface_collection: Option<&CollectionTag>,
        engine_groups: Option<&corpse::EngineDeathGroups<'_>>,
        mons_postprocess: mandrake_sim::mons_postprocess::MonsPostprocess,
        team_colours: &team_colours::TeamColours,
    ) -> Self {
        let bounds_tags = |group: [u8; 4], id: FourCC| {
            let from = |a: &TagArchive| {
                a.find_tag_by_id(FourCC::new(group), id)
                    .and_then(|e| a.get_tag_data(e).ok())
            };
            large_archive
                .filter(|_| &group == b".256")
                .and_then(from)
                .or_else(|| from(small_archive))
        };
        let mut object_bounds =
            object_bounds::SequenceBoundsTable::new(mesh_tag, &bounds_tags, mons_postprocess);
        let mut images: Vec<image::RgbaImage> = Vec::new();
        let mut offsets: Vec<(i16, i16)> = Vec::new();
        let mut unit_visuals: BTreeMap<FourCC, UnitSpriteVisual> = BTreeMap::new();
        let mut coll_cache: BTreeMap<FourCC, Option<std::sync::Arc<CollectionTag>>> =
            BTreeMap::new();

        let mut get_collection = |coll_tag: FourCC| -> Option<std::sync::Arc<CollectionTag>> {
            if let Some(cached) = coll_cache.get(&coll_tag) {
                return cached.clone();
            }
            let coll = crate::map_load::find_tag_data(
                small_archive,
                large_archive,
                *b".256",
                coll_tag,
                true,
            )
            .and_then(|d| CollectionTag::from_bytes(&d).ok())
            .map(std::sync::Arc::new);
            coll_cache.insert(coll_tag, coll.clone());
            coll
        };

        let mut atlas_of: BTreeMap<(FourCC, usize), usize> = BTreeMap::new();

        let mut scenery_visuals: BTreeMap<FourCC, scenery::SceneryVisual> = BTreeMap::new();
        let mut scenery_cores: BTreeMap<FourCC, media::CoreRef> = BTreeMap::new();
        let mut unit_cores: BTreeMap<FourCC, media::CoreRef> = BTreeMap::new();
        let mut scenery_core_ids: BTreeMap<FourCC, FourCC> = BTreeMap::new();
        let mut scenery_collections: BTreeMap<FourCC, FourCC> = BTreeMap::new();
        let mut unit_core_ids: BTreeMap<FourCC, FourCC> = BTreeMap::new();
        let mut unit_collections: BTreeMap<FourCC, (FourCC, FourCC)> = BTreeMap::new();
        let mut permutation_sources: Vec<(FourCC, FourCC)> = Vec::new();
        let mut collection_cores: BTreeMap<FourCC, FourCC> = BTreeMap::new();
        let mut collection_core_tints: BTreeMap<FourCC, media::CoreRef> = BTreeMap::new();
        let mut death_group_roots: Vec<FourCC> = Vec::new();
        let core_archives: Vec<&TagArchive> = large_archive
            .into_iter()
            .chain(std::iter::once(small_archive))
            .collect();
        let mut core_tints: BTreeMap<FourCC, media::CoreRef> = BTreeMap::new();
        for arch in &core_archives {
            for entry in arch.list_tags() {
                if entry.tag_type == FourCC::new(*b"core")
                    && let Some(core) = media::CoreRef::of_core(&core_archives, entry.tag_id)
                {
                    core_tints.entry(entry.tag_id).or_insert(core);
                }
            }
        }
        for marker in mesh_tag.scenery_markers() {
            let Some(tag_id) = mesh_tag.palette_tag_for_marker(marker) else {
                continue;
            };
            if scenery_visuals.contains_key(&tag_id) {
                continue;
            }
            let Some(scen) = small_archive
                .find_tag_by_id(FourCC::new(*b"scen"), tag_id)
                .and_then(|e| small_archive.get_tag_data(e).ok())
                .and_then(|d| mandrake_tag::SceneryTag::from_bytes(&d).ok())
            else {
                continue;
            };
            let coll_id = scen.resolve_collection_tag(small_archive);
            let Some(coll) = get_collection(coll_id) else {
                continue;
            };
            if let Some(core) = media::CoreRef::of_core(&core_archives, scen.collection_tag) {
                scenery_cores.insert(tag_id, core);
                core_tints.entry(scen.collection_tag).or_insert(core);
                collection_core_tints.entry(coll_id).or_insert(core);
            }
            collection_cores
                .entry(coll_id)
                .or_insert(scen.collection_tag);
            scenery_core_ids.insert(tag_id, scen.collection_tag);
            scenery_collections.insert(tag_id, coll_id);
            permutation_sources.push((scen.collection_tag, coll_id));
            let seq_idx = scen.sequence_index.max(0) as usize;
            let Some(seq) = coll.sequence(seq_idx).cloned() else {
                warn!("Scenery '{tag_id}': sequence {seq_idx} not in collection '{coll_id}'");
                continue;
            };
            let object = scen.object_definition_tag().and_then(|id| {
                let entry = small_archive.find_tag_by_id(FourCC::new(*b"obje"), id)?;
                let data = small_archive.get_tag_data(entry).ok()?;
                mandrake_tag::ObjectDefinition::from_bytes(id, &data).ok()
            });
            if object.is_none() {
                warn!(
                    "Scenery '{}': obje '{:?}' not found; using object scale 1.0",
                    tag_id,
                    scen.object_definition_tag()
                );
            }
            let frames = sequence_frame_visuals(
                coll_id,
                &coll,
                &seq,
                selection::OBJECT_SCALE_ONE as u16,
                &mut images,
                &mut offsets,
                &mut atlas_of,
            );
            if let Some(f) = seq.bitmap_index(0, 0).and_then(|b| coll.frame(b)) {
                let [w, h] = object_bitmap_world_size(
                    f,
                    &seq,
                    coll.pixels_to_world,
                    selection::OBJECT_SCALE_ONE as u16,
                );
                debug!(
                    "Scenery '{}' ({}, seq {}: {} frames x {} views, flags {:#x}): obje {:?} scale \
                     {:#x}+{:#x}, frame 0 {}x{} px -> {:.3}x{:.3} cells at scale 1.0",
                    tag_id,
                    coll_id,
                    seq_idx,
                    seq.frames_per_view,
                    seq.num_views,
                    seq.flags,
                    object.map(|d| d.tag_id),
                    object.map_or(0, |d| d.scale),
                    object.map_or(0, |d| d.scale_delta),
                    f.width,
                    f.height,
                    w,
                    h,
                );
            }
            let base_scale = object.map_or(selection::OBJECT_SCALE_ONE as u16, |d| d.scale);
            let remains = usize::try_from(scen.destroyed_sequence_index)
                .ok()
                .and_then(|index| {
                    let Some(seq) = coll.sequence(index).cloned() else {
                        warn!("Scenery '{tag_id}': remains {index} not in collection '{coll_id}'");
                        return None;
                    };
                    let frames = sequence_frame_visuals(
                        coll_id,
                        &coll,
                        &seq,
                        selection::OBJECT_SCALE_ONE as u16,
                        &mut images,
                        &mut offsets,
                        &mut atlas_of,
                    );
                    Some(Box::new(scenery::SceneryVisual {
                        object_bounds: object_bounds.world_bounds(
                            coll_id,
                            index,
                            seq.bounds.world(),
                        ),
                        sequence: seq,
                        frames,
                        base_scale,
                        scen_flags: scen.flags,
                        sequence_index: index as i16,
                        remains: None,
                    }))
                });
            scenery_visuals.insert(
                tag_id,
                scenery::SceneryVisual {
                    object_bounds: object_bounds.world_bounds(coll_id, seq_idx, seq.bounds.world()),
                    sequence: seq,
                    frames,
                    base_scale,
                    scen_flags: scen.flags,
                    sequence_index: seq_idx as i16,
                    remains,
                },
            );
        }

        for placement in mesh_tag.unit_placements(1.0) {
            let arch = placement.unit_tag;
            if unit_visuals.contains_key(&arch) {
                continue;
            }

            let unit = small_archive
                .find_tag_by_id(FourCC::new(*b"unit"), arch)
                .or_else(|| small_archive.find_tag(FourCC::new(*b"unit"), arch.as_str()))
                .and_then(|e| mandrake_tag::UnitTag::from_archive(small_archive, e).ok());

            let Some(unit) = unit else {
                continue;
            };
            if let Some(core) = media::CoreRef::of_core(&core_archives, unit.collection_ref_tag) {
                unit_cores.insert(arch, core);
            }
            let Some(coll) = get_collection(unit.animation_collection) else {
                continue;
            };
            unit_core_ids.insert(arch, unit.collection_ref_tag);
            unit_collections.insert(arch, (unit.collection_ref_tag, unit.animation_collection));
            permutation_sources.push((unit.collection_ref_tag, unit.animation_collection));
            let mons = small_archive
                .find_tag_by_id(FourCC::new(*b"mons"), unit.monster_tag_id)
                .and_then(|e| small_archive.get_tag_data(e).ok());
            let mut visual = UnitSpriteVisual {
                object: unit.object_tag.and_then(|id| {
                    let entry = small_archive.find_tag_by_id(FourCC::new(*b"obje"), id)?;
                    let data = small_archive.get_tag_data(entry).ok()?;
                    mandrake_tag::ObjectDefinition::from_bytes(id, &data).ok()
                }),
                definition: mons.as_deref().map_or((0, 0), mons_flags_and_class),
                ..Default::default()
            };
            let object_scale = visual
                .object
                .map_or(selection::OBJECT_SCALE_ONE as u16, |d| d.scale);
            let slot_sequences = (0..mandrake_tag::MONSTER_SEQUENCE_SLOTS)
                .map(MonsterSequenceSlot)
                .map(|slot| (Some(slot), unit.sequence_for(slot)))
                .chain(unit.attack_sequences().into_iter().map(|i| (None, Some(i))));
            for (slot, seq_idx) in slot_sequences {
                let Some(seq_idx) = seq_idx.filter(|&i| coll.sequence(i).is_some()) else {
                    continue;
                };
                if let Some(slot) = slot {
                    visual.slots[slot.0] = Some(seq_idx);
                }
                if visual.sequences.contains_key(&seq_idx) {
                    continue;
                }
                let seq = coll.sequence(seq_idx).expect("checked above").clone();
                let frames = sequence_frame_visuals(
                    unit.animation_collection,
                    &coll,
                    &seq,
                    object_scale,
                    &mut images,
                    &mut offsets,
                    &mut atlas_of,
                );
                visual.sequences.insert(
                    seq_idx,
                    SequenceVisual {
                        object_bounds: object_bounds.world_bounds(
                            unit.animation_collection,
                            seq_idx,
                            seq.bounds.world(),
                        ),
                        sequence: seq,
                        frames,
                    },
                );
            }
            collection_cores
                .entry(unit.animation_collection)
                .or_insert(unit.collection_ref_tag);
            if let Some(core) = media::CoreRef::of_core(&core_archives, unit.collection_ref_tag) {
                core_tints.entry(unit.collection_ref_tag).or_insert(core);
                collection_core_tints
                    .entry(unit.animation_collection)
                    .or_insert(core);
            }
            death_group_roots.extend(unit.death_groups.iter().flatten().copied());
            if visual.is_empty() {
                warn!(
                    "Unit '{}' has no loadable standing/moving sequence in collection '{}'",
                    arch, unit.animation_collection
                );
            } else {
                unit_visuals.insert(arch, visual);
            }
        }

        let mut corpse_sprites = corpse::load_corpse_sprites(
            small_archive,
            &death_group_roots,
            engine_groups,
            &mut object_bounds,
            &mut get_collection,
            &mut images,
            &mut offsets,
            &mut atlas_of,
        );
        for (arch, visual) in &unit_visuals {
            let Some(&(_, coll_id)) = unit_collections.get(arch) else {
                continue;
            };
            let Some(coll) = get_collection(coll_id) else {
                continue;
            };
            for &index in visual.sequences.keys() {
                let Ok(sequence) = u8::try_from(index) else {
                    continue;
                };
                corpse_sprites.insert_sequence(
                    coll_id,
                    &coll,
                    sequence,
                    &mut object_bounds,
                    &mut images,
                    &mut offsets,
                    &mut atlas_of,
                );
            }
        }

        let mut connector_visual = None;
        if let Some(conn_id) = connectors.connector_tag
            && !connectors.connectors.is_empty()
        {
            let definition = small_archive
                .find_tag_by_id(FourCC::new(*b"conn"), conn_id)
                .and_then(|e| small_archive.get_tag_data(e).ok())
                .and_then(|d| ConnectorTag::from_bytes(&d).ok());
            match definition {
                Some(definition) => {
                    let coll_id = definition.resolve_collection_tag(small_archive);
                    let seq_idx = definition.sequence_index.max(0) as usize;
                    let frame = get_collection(coll_id).and_then(|coll| {
                        let f = coll.sequence_frame(seq_idx, 0, 0)?;
                        let img = image::RgbaImage::from_raw(
                            f.width.max(1) as u32,
                            f.height.max(1) as u32,
                            sprite_texels(f, coll.palette(), &coll.color_table),
                        )?;
                        let scale = coll.sequence(seq_idx)?.bounds.world_units_per_pixel_fixed();
                        let rect = mandrake_view::connector::instance_rect_vertical(
                            f.offset_y, f.height, scale,
                        );
                        Some((img, (f.offset_x, f.offset_y), rect))
                    });
                    if let Some((img, offset, rect)) = frame {
                        let frame_index = images.len();
                        images.push(img);
                        offsets.push(offset);
                        connector_visual = Some(ConnectorVisual {
                            definition,
                            rect,
                            frame_index,
                        });
                    } else {
                        warn!(
                            "Connector '{}' collection '{}' sequence {} not found; wires not rendered",
                            conn_id, coll_id, seq_idx
                        );
                    }
                }
                None => warn!("Connector tag '{}' not found in archive", conn_id),
            }
        }

        let mut marker_sprites = BTreeMap::new();
        if let Some(coll) = interface_collection {
            for seq_idx in move_marker::MARKER_SEQUENCES {
                let Some(seq) = coll.sequence(seq_idx as usize) else {
                    warn!("Interface collection: no marker sequence {seq_idx}");
                    continue;
                };
                let Some(f) = seq.bitmap_index(0, 0).and_then(|b| coll.frame(b)) else {
                    continue;
                };
                let rgba = sprite_texels(f, coll.palette(), &coll.color_table);
                let Some(img) =
                    image::RgbaImage::from_raw(f.width.max(1) as u32, f.height.max(1) as u32, rgba)
                else {
                    continue;
                };
                let atlas_index = images.len();
                images.push(img);
                offsets.push((f.offset_x, f.offset_y));
                marker_sprites.insert(
                    seq_idx,
                    move_marker::MarkerSprite {
                        atlas_index,
                        world_size: object_bitmap_world_size(
                            f,
                            seq,
                            coll.pixels_to_world,
                            selection::OBJECT_SCALE_ONE as u16,
                        ),
                    },
                );
            }
        } else {
            warn!("Interface collection not loaded; no move markers");
        }

        let particle_visual = particle_tags(small_archive, large_archive, mesh_tag).and_then(
            |(definition, collection)| {
                let coll = get_collection(collection)?;
                let index = usize::try_from(definition.sequence).ok()?;
                let seq = coll.sequence(index)?.clone();
                let frames = sequence_frame_visuals(
                    collection,
                    &coll,
                    &seq,
                    mandrake_view::selection::OBJECT_SCALE_ONE as u16,
                    &mut images,
                    &mut offsets,
                    &mut atlas_of,
                );
                let flags: Vec<u32> = (0..usize::from(seq.num_views))
                    .flat_map(|v| (0..usize::from(seq.frames_per_view)).map(move |f| (v, f)))
                    .map(|(v, f)| coll.instance_flags(index, v, f))
                    .collect();
                let bounds = seq.bounds.world();
                Some((
                    SequenceVisual {
                        sequence: seq,
                        frames,
                        object_bounds: bounds,
                    },
                    flags,
                    definition.scale,
                    media::CoreRef::of_core(&core_archives, definition.collection_reference)
                        .unwrap_or_default(),
                ))
            },
        );

        let mut local_visuals = Vec::new();
        let mut lightning = lightning_draw::LightningDraw::default();
        let mut mesh_effect_sprites = crate::mesh_effect_sprites::MeshEffectSprites::default();
        if let Some(groups) = engine_groups {
            let tag = local_projectile_tags(small_archive, large_archive);
            let (media_effect, splashes) =
                ambient_local_sources(small_archive, large_archive, mesh_tag);
            let ambient: Vec<FourCC> = media_effect
                .map(|(s, _)| s)
                .into_iter()
                .chain(splashes)
                .map(|s| s.group)
                .chain(fire_local_groups(&tag, mesh_tag))
                .chain(marker_local_groups(mesh_tag))
                .collect();
            let (_, sources) =
                projectile_local::LocalProjectiles::for_map(groups.catalog(), &ambient, &tag, 0);
            let (draw, lightning_sources) =
                lightning_draw::LightningDraw::for_catalog(groups.catalog(), &tag);
            lightning = draw;
            let (rings, ring_sources) =
                crate::mesh_effect_sprites::MeshEffectSprites::for_catalog(groups.catalog(), &tag);
            mesh_effect_sprites = rings;
            for source in sources
                .into_iter()
                .chain(lightning_sources)
                .chain(ring_sources)
            {
                let Some(coll) = get_collection(source.collection) else {
                    continue;
                };
                permutation_sources.push((source.reference, source.collection));
                if let Some(stain) =
                    corpse::StainSequence::from_collection(&coll, source.stain_sequence as usize)
                        .filter(|_| source.stain_sequence >= 0)
                {
                    corpse_sprites
                        .stains
                        .insert((source.collection, source.stain_sequence), stain);
                }
                let index = usize::from(source.sequence);
                let Some(seq) = coll.sequence(index).cloned() else {
                    continue;
                };
                mesh_effect_sprites.set_timing(
                    (source.reference, source.sequence),
                    seq.frames_per_view,
                    seq.ticks_per_frame,
                );
                let frames = sequence_frame_visuals(
                    source.collection,
                    &coll,
                    &seq,
                    mandrake_view::selection::OBJECT_SCALE_ONE as u16,
                    &mut images,
                    &mut offsets,
                    &mut atlas_of,
                );
                let flags: Vec<u32> = (0..usize::from(seq.num_views))
                    .flat_map(|v| (0..usize::from(seq.frames_per_view)).map(move |f| (v, f)))
                    .map(|(v, f)| coll.instance_flags(index, v, f))
                    .collect();
                let bounds = seq.bounds.world();
                local_visuals.push((
                    source,
                    SequenceVisual {
                        sequence: seq,
                        frames,
                        object_bounds: bounds,
                    },
                    flags,
                ));
            }
        }

        let drawn_permutations: BTreeMap<FourCC, &std::collections::BTreeSet<usize>> = team_colours
            .base_permutations
            .iter()
            .filter_map(|(unit, set)| Some((*unit_core_ids.get(unit)?, set)))
            .collect();
        let permuted_atlas = decode_permuted_sprites(
            &permutation_sources,
            &|core, p| drawn_permutations.get(&core).is_none_or(|s| s.contains(&p)),
            &core_archives,
            &mut get_collection,
            &atlas_of,
            &mut images,
            &mut offsets,
        );

        let flag_source = team_colours.flags.as_ref().and_then(|f| {
            let core = *scenery_core_ids.get(&f.scenery)?;
            let coll = *scenery_collections.get(&f.scenery)?;
            Some((f, core, coll))
        });
        let flag_scenery: std::collections::BTreeSet<FourCC> = flag_source
            .map(|(_, _, coll)| {
                scenery_collections
                    .iter()
                    .filter(|&(_, &c)| c == coll)
                    .map(|(&scen, _)| scen)
                    .collect()
            })
            .unwrap_or_default();
        let flag_atlas = flag_source
            .map(|(tables, core, coll)| {
                decode_flag_tables(
                    tables,
                    core,
                    coll,
                    &core_archives,
                    &mut get_collection,
                    &atlas_of,
                    &mut images,
                    &mut offsets,
                )
            })
            .unwrap_or_default();

        let unit_sprites: BTreeMap<FourCC, std::collections::BTreeSet<usize>> = unit_visuals
            .iter()
            .map(|(&arch, v)| {
                let sprites = v
                    .sequences
                    .values()
                    .flat_map(|s| &s.frames)
                    .flat_map(|f| f.views.iter().flatten())
                    .map(|view| view.atlas_index)
                    .collect();
                (arch, sprites)
            })
            .collect();
        let team_sources = team_unit_sprites(
            team_colours,
            &unit_collections,
            &unit_sprites,
            &core_archives,
            &mut get_collection,
            &atlas_of,
        );
        let team_rows: std::collections::BTreeSet<(FourCC, i16)> =
            team_colours.monsters.values().copied().collect();
        let images_before = images.len();
        let team_atlas = decode_team_rows(
            team_colours,
            &team_sources,
            team_rows.iter().copied(),
            &mut images,
            &mut offsets,
        );
        info!(
            "Team colours: {} sprite images for {} (unit, player, base) draws of {} rows",
            images.len() - images_before,
            team_atlas.len(),
            team_colours.rows.len()
        );

        let atlas = create_atlas_from_images_with_offsets(device, queue, layout, &images, &offsets);
        let mut local_projectile_sprites = projectile_local::LocalSprites::default();
        for (source, visual, flags) in &local_visuals {
            let frames = visual.frames.len().max(1);
            local_projectile_sprites.add(
                (source.reference, source.sequence),
                visual,
                source.core,
                frame_flags(flags, frames),
                |index| atlas_anchor(&atlas, index),
            );
        }
        let particle_sprites = particle_visual.map(|(visual, flags, scale, core)| {
            let frames = visual.frames.len().max(1);
            particles::ParticleSprites::new(
                &visual,
                frame_flags(&flags, frames),
                scale,
                core,
                |index| atlas_anchor(&atlas, index),
            )
        });
        info!(
            "Loaded real game sprite atlas: {}x{} px, {} frames ({} scenery archetypes, {} unit archetypes)",
            atlas.width,
            atlas.height,
            atlas.frame_count(),
            scenery_visuals.len(),
            unit_visuals.len()
        );

        Self {
            atlas,
            scenery_visuals,
            scenery_objects: scenery::SceneryObjects::default(),
            dynamic_scenery: std::cell::RefCell::new(None),
            unit_visuals,
            connector_visual,
            interface_colors: None,
            scenery_cores,
            unit_cores,
            unit_media: Default::default(),
            placed_monster_flags: placed_monster_flags(mesh_tag),
            permuted_atlas,
            team_atlas,
            team_rows,
            flag_atlas,
            flag_core: flag_source.map(|(_, core, _)| core),
            flag_scenery,
            team_cache: TeamSpriteCache {
                sources: team_sources,
                ..Default::default()
            },
            scenery_core_ids,
            unit_core_ids,
            collection_cores,
            collection_core_tints,
            core_tints,
            corpse_sprites,
            particle_sprites,
            local_projectile_sprites,
            lightning,
            mesh_effect_sprites,
            projfx_log: None,
            marker_sprites,
        }
    }

    pub(crate) fn marker_billboards(
        &self,
        sprites: &move_marker::LocalSprites,
        now: i32,
        sim: &Simulation,
    ) -> Vec<BillboardInstance> {
        move_marker::marker_billboards(
            sprites,
            now,
            sim,
            |seq| self.marker_sprites.get(&seq).copied(),
            |v, position| {
                self.atlas.create_instance_with_world_size(
                    v.atlas_index,
                    position,
                    v.world_size,
                    false,
                )
            },
        )
    }
}

pub(crate) fn atlas_anchor(atlas: &mandrake_render::TextureAtlas, index: usize) -> [f32; 2] {
    atlas
        .create_instance_with_world_size(index, [0.0; 3], [1.0; 2], false)
        .map_or([0.0; 2], |i| i.anchor)
}

pub(crate) fn frame_flags(flags: &[u32], frames: usize) -> impl Fn(usize, usize) -> u32 + '_ {
    move |view, frame| flags.get(view * frames + frame).copied().unwrap_or(0)
}

pub(crate) fn decode_permuted_sprites(
    sources: &[(FourCC, FourCC)],
    drawn: &dyn Fn(FourCC, usize) -> bool,
    core_archives: &[&TagArchive],
    get_collection: &mut dyn FnMut(FourCC) -> Option<std::sync::Arc<CollectionTag>>,
    atlas_of: &BTreeMap<(FourCC, usize), usize>,
    images: &mut Vec<image::RgbaImage>,
    offsets: &mut Vec<(i16, i16)>,
) -> BTreeMap<(FourCC, usize, usize), usize> {
    let mut sources = sources.to_vec();
    sources.sort_unstable();
    sources.dedup();
    let mut permuted = BTreeMap::new();
    let mut decoded: DecodedPaletteMap = BTreeMap::new();
    let mut bitmaps_of: BTreeMap<FourCC, Vec<(usize, usize)>> = BTreeMap::new();
    for (&(coll, bitmap), &index) in atlas_of {
        bitmaps_of.entry(coll).or_default().push((bitmap, index));
    }
    for list in bitmaps_of.values_mut() {
        list.sort_unstable();
    }
    for (core_id, coll_id) in sources {
        let Some(core) = core_archives.iter().rev().find_map(|a| {
            let e = a.find_tag_by_id(FourCC::new(*b"core"), core_id)?;
            CorePermutations::from_bytes(&a.get_tag_data(e).ok()?).ok()
        }) else {
            continue;
        };
        let Some(coll) = get_collection(coll_id).filter(|c| c.has_hue_changes()) else {
            continue;
        };
        let Some(bitmaps) = bitmaps_of.get(&coll_id) else {
            continue;
        };
        for p in (0..core.len()).filter(|&p| drawn(core_id, p)) {
            let palette = coll.permutation_palette(&core, p);
            let mut out = PaletteDecode {
                decoded: &mut decoded,
                images,
                offsets,
            };
            for (base, index) in out.decode(coll_id, &coll, bitmaps, &palette) {
                permuted.insert((core_id, base, p), index);
            }
        }
    }
    info!(
        "Colour permutations: {} sprite images for {} (core, base, permutation) draws",
        decoded.len(),
        permuted.len()
    );
    permuted
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_flag_tables(
    tables: &team_colours::FlagTables,
    core_id: FourCC,
    coll_id: FourCC,
    core_archives: &[&TagArchive],
    get_collection: &mut dyn FnMut(FourCC) -> Option<std::sync::Arc<CollectionTag>>,
    atlas_of: &BTreeMap<(FourCC, usize), usize>,
    images: &mut Vec<image::RgbaImage>,
    offsets: &mut Vec<(i16, i16)>,
) -> BTreeMap<(usize, i16), usize> {
    let mut out = BTreeMap::new();
    let Some(core) = core_archives.iter().rev().find_map(|a| {
        let e = a.find_tag_by_id(FourCC::new(*b"core"), core_id)?;
        CorePermutations::from_bytes(&a.get_tag_data(e).ok()?).ok()
    }) else {
        return out;
    };
    let Some(coll) = get_collection(coll_id).filter(|c| c.has_hue_changes()) else {
        return out;
    };
    let bitmaps: Vec<(usize, usize)> = atlas_of
        .range((coll_id, 0)..=(coll_id, usize::MAX))
        .map(|(&(_, bitmap), &index)| (bitmap, index))
        .collect();
    let base = core.colors(0, coll.hue_changes.len());
    let mut decoded = BTreeMap::new();
    for (team, colours) in tables.teams.iter().enumerate() {
        let Some((primary, secondary)) = *colours else {
            continue;
        };
        let palette = coll.palette_of_colors(&coll.team_colors(&base, primary, secondary));
        let mut decode = PaletteDecode {
            decoded: &mut decoded,
            images,
            offsets,
        };
        for (b, index) in decode.decode(coll_id, &coll, &bitmaps, &palette) {
            out.insert((b, team as i16), index);
        }
    }
    info!(
        "Flag tables: {} sprite images for {} teams of collection '{coll_id}'",
        decoded.len(),
        tables.teams.len()
    );
    out
}

type DecodedPaletteMap = BTreeMap<(FourCC, usize, std::sync::Arc<[[u8; 4]]>), usize>;

struct PaletteDecode<'a> {
    decoded: &'a mut DecodedPaletteMap,
    images: &'a mut Vec<image::RgbaImage>,
    offsets: &'a mut Vec<(i16, i16)>,
}

impl PaletteDecode<'_> {
    fn decode(
        &mut self,
        coll_id: FourCC,
        coll: &CollectionTag,
        bitmaps: &[(usize, usize)],
        palette: &[[u8; 4]],
    ) -> Vec<(usize, usize)> {
        if palette == coll.palette.as_slice() {
            return Vec::new();
        }
        let palette_key: std::sync::Arc<[[u8; 4]]> = std::sync::Arc::from(palette);
        let mut out = Vec::with_capacity(bitmaps.len());
        for &(bitmap, base) in bitmaps {
            let Some(f) = coll
                .frame(bitmap)
                .filter(|f| f.flags & mandrake_tag::BITMAP_FLAG_RAW_COLOR_TABLE == 0)
            else {
                continue;
            };
            let key = (coll_id, bitmap, std::sync::Arc::clone(&palette_key));
            let index = match self.decoded.get(&key) {
                Some(&i) => i,
                None => {
                    let Some(img) = image::RgbaImage::from_raw(
                        f.width.max(1) as u32,
                        f.height.max(1) as u32,
                        sprite_texels(f, palette, &coll.color_table),
                    ) else {
                        continue;
                    };
                    let i = self.images.len();
                    self.images.push(img);
                    self.offsets.push((f.offset_x, f.offset_y));
                    self.decoded.insert(key, i);
                    i
                }
            };
            out.push((base, index));
        }
        out
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TeamUnitSprites {
    coll_id: FourCC,
    coll: std::sync::Arc<CollectionTag>,
    core: CorePermutations,
    bitmaps: Vec<(usize, usize)>,
}

pub(crate) fn team_unit_sprites(
    team: &team_colours::TeamColours,
    units: &BTreeMap<FourCC, (FourCC, FourCC)>,
    unit_sprites: &BTreeMap<FourCC, std::collections::BTreeSet<usize>>,
    core_archives: &[&TagArchive],
    get_collection: &mut dyn FnMut(FourCC) -> Option<std::sync::Arc<CollectionTag>>,
    atlas_of: &BTreeMap<(FourCC, usize), usize>,
) -> BTreeMap<FourCC, TeamUnitSprites> {
    let tags: std::collections::BTreeSet<FourCC> =
        team.rows.keys().map(|&(unit, _)| unit).collect();
    let mut out = BTreeMap::new();
    for unit in tags {
        let Some(&(core_id, coll_id)) = units.get(&unit) else {
            continue;
        };
        let Some(core) = core_archives.iter().rev().find_map(|a| {
            let e = a.find_tag_by_id(FourCC::new(*b"core"), core_id)?;
            CorePermutations::from_bytes(&a.get_tag_data(e).ok()?).ok()
        }) else {
            continue;
        };
        let Some(coll) = get_collection(coll_id).filter(|c| c.has_hue_changes()) else {
            continue;
        };
        let Some(drawn) = unit_sprites.get(&unit) else {
            continue;
        };
        let mut bitmaps: Vec<(usize, usize)> = atlas_of
            .range((coll_id, 0)..=(coll_id, usize::MAX))
            .map(|(&(_, bitmap), &index)| (bitmap, index))
            .filter(|(_, index)| drawn.contains(index))
            .collect();
        bitmaps.sort_unstable();
        out.insert(
            unit,
            TeamUnitSprites {
                coll_id,
                coll,
                core,
                bitmaps,
            },
        );
    }
    out
}

pub(crate) fn decode_team_rows(
    team: &team_colours::TeamColours,
    sources: &BTreeMap<FourCC, TeamUnitSprites>,
    rows: impl IntoIterator<Item = (FourCC, i16)>,
    images: &mut Vec<image::RgbaImage>,
    offsets: &mut Vec<(i16, i16)>,
) -> BTreeMap<(FourCC, i16, usize), usize> {
    let mut out = BTreeMap::new();
    let mut decoded = BTreeMap::new();
    for (unit, player) in rows {
        let (Some(row), Some(src)) = (team.rows.get(&(unit, player)), sources.get(&unit)) else {
            continue;
        };
        let base = src.core.colors(row.base, src.coll.hue_changes.len());
        let palette =
            src.coll
                .palette_of_colors(&src.coll.team_colors(&base, row.primary, row.secondary));
        let mut decode = PaletteDecode {
            decoded: &mut decoded,
            images,
            offsets,
        };
        for (base, index) in decode.decode(src.coll_id, &src.coll, &src.bitmaps, &palette) {
            out.insert((unit, player, base), index);
        }
    }
    out
}

#[derive(Default)]
pub(crate) struct TeamSpriteCache {
    sources: BTreeMap<FourCC, TeamUnitSprites>,
    rows: std::collections::BTreeSet<(FourCC, i16)>,
    index: BTreeMap<(FourCC, i16, usize), usize>,
    pub(crate) atlas: Option<TextureAtlas>,
}

pub(crate) const TEAM_CACHE_FLAG: u32 = 1 << 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TeamSprite {
    Atlas(usize),
    Cache(usize),
}

fn placed_monster_flags(mesh_tag: &MeshTag) -> BTreeMap<u32, (i16, u32)> {
    mesh_tag
        .unit_markers()
        .filter_map(|marker| {
            let id = u32::try_from(marker.marker_id).ok()?;
            let palette = mesh_tag.palette_for_marker(marker)?;
            Some((id, (palette.flags, marker.flags)))
        })
        .collect()
}

fn mons_flags_and_class(mons: &[u8]) -> (u32, i16) {
    let flags = mons
        .get(0..4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let at = interface_state::MONS_CLASS_OFFSET;
    let class = mons
        .get(at..at + 2)
        .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]));
    (flags, class)
}

const BITMAP_FLAG_RLE: u16 = 0x04;

pub(crate) const MAGENTA_ENTRY_16: [u16; 4] = [0xff00, 0, 0xff00, 0];

pub(crate) fn keeps_color_alpha(f: &mandrake_tag::BitmapFrame) -> bool {
    let tiled =
        f.flags & BITMAP_FLAG_RLE == 0 && f.width.is_power_of_two() && f.height.is_power_of_two();
    f.flags & mandrake_tag::collection::BITMAP_FLAG_COLOR_ALPHA != 0 && !tiled
}

pub(crate) fn sprite_texels(
    f: &mandrake_tag::BitmapFrame,
    palette: &[[u8; 4]],
    color_table: &[[u16; 4]],
) -> Vec<u8> {
    if !keeps_color_alpha(f) {
        if !f.wide_pixels.is_empty() {
            return f.to_rgba_wide(palette);
        }
        let mut rgba = f.to_rgba(palette);
        if f.flags & BITMAP_FLAG_RLE == 0
            && color_table.get(1).is_some_and(|c| *c == MAGENTA_ENTRY_16)
        {
            for (px, &i) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(&f.pixels) {
                if i == 1 {
                    *px = [0; 4];
                }
            }
        }
        return rgba;
    }
    let mut rgba = f.to_rgba_color_alpha(palette, color_table);
    for px in rgba.as_chunks_mut::<4>().0 {
        let scale = px[3] as u32 + 1;
        for c in &mut px[..3] {
            *c = ((*c as u32 * scale) >> 8) as u8;
        }
    }
    rgba
}

pub(crate) fn sequence_frame_visuals(
    coll_id: FourCC,
    coll: &CollectionTag,
    seq: &mandrake_tag::CollectionSequence,
    object_scale: u16,
    images: &mut Vec<image::RgbaImage>,
    offsets: &mut Vec<(i16, i16)>,
    atlas_of: &mut BTreeMap<(FourCC, usize), usize>,
) -> Vec<FrameVisual> {
    let mut upload = |b: usize, f: &mandrake_tag::BitmapFrame| -> Option<usize> {
        if let Some(&i) = atlas_of.get(&(coll_id, b)) {
            return Some(i);
        }
        let rgba = sprite_texels(f, coll.palette(), &coll.color_table);
        let img = image::RgbaImage::from_raw(f.width.max(1) as u32, f.height.max(1) as u32, rgba)?;
        let i = images.len();
        images.push(img);
        offsets.push((f.offset_x, f.offset_y));
        atlas_of.insert((coll_id, b), i);
        Some(i)
    };
    let nv = (seq.num_views as usize).max(1);
    let mut frames = Vec::new();
    let mut coverage_of: BTreeMap<usize, Arc<[u8]>> = BTreeMap::new();
    for frame in 0..seq.frames_per_view as usize {
        let mut fv = FrameVisual::default();
        for view in 0..nv {
            if seq.resolve_view(view) != (view, false) {
                fv.views.push(None);
                continue;
            }
            fv.views.push(
                seq.bitmap_index(view, frame)
                    .filter(|_| coll.has_bitmap(seq.index, view, frame))
                    .and_then(|b| coll.frame(b).map(|f| (b, f)))
                    .and_then(|(b, f)| {
                        Some(ViewSprite {
                            atlas_index: upload(b, f)?,
                            world_size: object_bitmap_world_size(
                                f,
                                seq,
                                coll.pixels_to_world,
                                object_scale,
                            ),
                        })
                    }),
            );
        }
        fv.shadow = seq
            .shadow_descriptor_index(frame)
            .and_then(|d| coll.shadow_descriptors().get(d))
            .and_then(|d| {
                let f = coll.frame(usize::try_from(d.shadow_bitmap_index).ok()?)?;
                Some(ShadowSprite {
                    flags: d.flags,
                    offset_x: d.offset_x,
                    offset_y: d.offset_y,
                    width: f.width,
                    height: f.height,
                    pixels: f.pixels.clone().into(),
                })
            });
        if fv.shadow.as_ref().is_some_and(|s| s.flags & 1 == 0) {
            fv.shadow_sources = (0..nv)
                .map(|view| {
                    let (stored, mirrored) = seq.resolve_view(view);
                    let b = seq
                        .bitmap_index(stored, frame)
                        .filter(|_| coll.has_bitmap(seq.index, stored, frame))?;
                    let f = coll.frame(b)?;
                    let alpha = coverage_of
                        .entry(b)
                        .or_insert_with(|| shadowgen::coverage(f))
                        .clone();
                    let flags = coll.instance_flags(seq.index, stored, frame);
                    shadowgen::source_for_view(seq, f, alpha, flags, mirrored).map(Arc::new)
                })
                .collect();
        }
        frames.push(fv);
    }
    frames
}

pub(crate) fn object_bitmap_world_size(
    frame: &mandrake_tag::BitmapFrame,
    seq: &mandrake_tag::CollectionSequence,
    collection_pixels_to_world: i16,
    object_scale: u16,
) -> [f32; 2] {
    let units_per_px = seq.bounds.world_units_per_pixel_fixed();
    if units_per_px <= 0 {
        let (w, h) = frame.world_size(collection_pixels_to_world);
        return [w, h];
    }
    let per_px = units_per_px as f32 / 65536.0 * (object_scale as f32 / 256.0)
        / selection::WORLD_UNITS_PER_CELL;
    [frame.width as f32 * per_px, frame.height as f32 * per_px]
}
