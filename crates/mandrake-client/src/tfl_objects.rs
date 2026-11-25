use crate::object_history::FRACTION_ONE;
use crate::object_sprites::{DrawnObject, ObjectSprites};
use crate::sprite_manager::sequence_frame_visuals;
use crate::tfl_game::UnitLook;
use crate::tfl_map::{TflMap, tag_data};
use crate::unit_anim::SequenceVisual;
use mandrake_common::FourCC;
use mandrake_render::TextureAtlas;
use mandrake_sim::views::ObjectKind;
use mandrake_tag::collection::{CollectionTag, CorePermutations};
use mandrake_tag::mesh::{MarkerEntry, MarkerPaletteEntry};
use std::collections::BTreeMap;
use tracing::{debug, info, warn};

pub(crate) const TFL_WORLD_PER_CELL: f32 = 1024.0;
pub(crate) const WORLD_PER_RENDER_UNIT: f32 = 512.0;
pub(crate) const TFL_CELL_SIZE: f32 = TFL_WORLD_PER_CELL / WORLD_PER_RENDER_UNIT;
pub(crate) const TFL_SPRITE_SCALE: f32 = crate::world_view::ClientRules::TFL.sprite_scale;
pub(crate) type LocalSpriteSource = (FourCC, u8, SequenceVisual, Vec<u32>);

pub(crate) fn local_sprites(
    sources: &[LocalSpriteSource],
    atlas: &TextureAtlas,
) -> crate::projectile_local::LocalSprites {
    let mut sprites = crate::projectile_local::LocalSprites::default();
    for (reference, sequence, visual, flags) in sources {
        sprites.add_scaled(
            (*reference, *sequence),
            visual,
            Default::default(),
            crate::sprite_manager::frame_flags(flags, visual.frames.len().max(1)),
            |index| crate::sprite_manager::atlas_anchor(atlas, index),
            TFL_SPRITE_SCALE,
        );
    }
    sprites
}

pub(crate) fn at_bucket_depth(out: &mut [mandrake_view::billboard::BillboardInstance]) {
    if crate::world_view::ClientRules::TFL.bucket_depth {
        for b in out {
            b.flags |= mandrake_view::billboard::BILLBOARD_FLAG_BUCKET_DEPTH;
        }
    }
}
const MARKER_NOT_PLACED: u32 = 0x1;
const SCEN_NET_GAME_ITEM: u32 = 0x8;

#[derive(Default)]
pub(crate) struct TflObjects {
    pub(crate) images: Vec<image::RgbaImage>,
    pub(crate) offsets: Vec<(i16, i16)>,
    pub(crate) sprites: Vec<DrawnObject>,
    pub(crate) scenery: usize,
    pub(crate) units: usize,
}

fn four(d: &[u8], o: usize) -> Option<FourCC> {
    let b: [u8; 4] = d.get(o..o + 4)?.try_into().ok()?;
    (b != [0xff; 4] && b != [0; 4]).then_some(FourCC::new(b))
}

fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    Some(i16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

#[cfg(test)]
pub(crate) fn render_position(marker: &MarkerEntry, cells: (u32, u32)) -> [f32; 3] {
    world_render_position([marker.x, marker.y, marker.z], cells)
}

pub(crate) fn world_render_position(p: [i32; 3], cells: (u32, u32)) -> [f32; 3] {
    let half_w = cells.0 as f32 * TFL_CELL_SIZE * 0.5;
    let half_d = cells.1 as f32 * TFL_CELL_SIZE * 0.5;
    [
        half_w - p[0] as f32 / WORLD_PER_RENDER_UNIT,
        p[2] as f32 / WORLD_PER_RENDER_UNIT,
        p[1] as f32 / WORLD_PER_RENDER_UNIT - half_d,
    ]
}

type Colours = Option<(FourCC, usize, Option<PlayerColours>)>;

type PlayerColours = ([u16; 4], [u16; 4]);

#[derive(Default)]
pub(crate) struct TflSprites {
    collections: BTreeMap<FourCC, Option<CollectionTag>>,
    sequences: BTreeMap<(FourCC, usize, Colours), Option<SequenceVisual>>,
    instance_flags: BTreeMap<(FourCC, u8), Vec<u32>>,
    atlas_of: BTreeMap<Colours, BTreeMap<(FourCC, usize), usize>>,
    players: Vec<(i16, PlayerColours)>,
    interp: BTreeMap<FourCC, Option<crate::sprite_interp::CollectionCache>>,
    doubled: BTreeMap<(FourCC, usize), (usize, i16)>,
    pub(crate) inbetweens: (usize, usize),
    pub(crate) out: TflObjects,
}

impl TflSprites {
    pub(crate) fn new(map: &TflMap) -> Self {
        let players = map.film.as_ref().map_or_else(Vec::new, |f| {
            f.players
                .iter()
                .filter_map(|p| {
                    let slot = i16::try_from(p.slot).ok()?;
                    Some((slot, player_colours(map, slot)?))
                })
                .collect()
        });
        Self {
            players,
            ..Self::default()
        }
    }

    fn look_colours(&self, look: Option<UnitLook>) -> Colours {
        let look = look?;
        let player = self
            .players
            .iter()
            .find(|(p, _)| *p == look.player)
            .map(|(_, c)| *c);
        Some((look.core, look.permutation, player))
    }

    fn colours_of(&self, o: &DrawnObject) -> Colours {
        let look = o.core.map(|core| UnitLook {
            core,
            player: o.player,
            permutation: usize::try_from(o.permutation).unwrap_or(0),
        });
        self.look_colours(look)
    }

    fn collection(&mut self, map: &TflMap, id: FourCC) -> Option<CollectionTag> {
        self.collections
            .entry(id)
            .or_insert_with(|| {
                let c = tag_data(&map.archive, b".256", id)
                    .and_then(|d| CollectionTag::from_bytes_tfl(&d).ok());
                if c.is_none() {
                    warn!("TFL: collection {id} not found or not read");
                }
                c
            })
            .clone()
    }

    fn coloured(map: &TflMap, mut coll: CollectionTag, colours: Colours) -> CollectionTag {
        let Some((core_id, p, player)) = colours else {
            return coll;
        };
        let core_bytes = map.tag(b"core", core_id);
        let Some(core) = core_bytes
            .as_ref()
            .and_then(|c| CorePermutations::from_bytes(c).ok())
            .filter(|c| !c.is_empty())
        else {
            warn!("TFL: core {core_id} not read; its objects keep colour table 0");
            return coll;
        };
        let p = p.min(core.len() - 1);
        coll.palette = match player {
            Some((primary, secondary)) => {
                let base = core.colors(p, coll.hue_changes.len());
                let colors = mandrake_tag::tfl::tfl_team_colors(&coll, &base, primary, secondary);
                coll.palette_of_colors(&colors)
            }
            None => {
                let mut palette = coll.permutation_palette(&core, p);
                if let Some(tint) = core_bytes.as_deref().and_then(core_tint) {
                    tint_palette(&mut palette, tint);
                }
                palette
            }
        };
        coll
    }

    fn sequence(
        &mut self,
        map: &TflMap,
        coll_id: FourCC,
        index: usize,
        colours: Colours,
    ) -> Option<&SequenceVisual> {
        let key = (coll_id, index, colours);
        if !self.sequences.contains_key(&key) {
            let coll = self
                .collection(map, coll_id)
                .map(|c| Self::coloured(map, c, colours));
            let built = coll.and_then(|coll| {
                let seq = coll.sequence(index)?.clone();
                let mut frames = sequence_frame_visuals(
                    coll_id,
                    &coll,
                    &seq,
                    0x100,
                    &mut self.out.images,
                    &mut self.out.offsets,
                    self.atlas_of.entry(colours).or_default(),
                );
                if crate::sprite_interp::installed() {
                    let cache = self
                        .interp
                        .entry(coll_id)
                        .or_insert_with(|| {
                            let data = tag_data(&map.archive, b".256", coll_id)?;
                            crate::sprite_interp::CollectionCache::load("tfl", coll_id, &data)
                        })
                        .as_ref();
                    if let Some(cache) = cache.filter(|c| c.has_sequence(index)) {
                        let n = frames.len();
                        let (images, bytes) = (self.out.images.len(), &mut self.inbetweens);
                        frames = doubled_visuals(
                            &frames,
                            &coll,
                            &seq,
                            cache,
                            &mut self.out.images,
                            &mut self.out.offsets,
                        );
                        let added = &self.out.images[images..];
                        bytes.0 += added.len();
                        bytes.1 += added.iter().map(|i| i.as_raw().len()).sum::<usize>();
                        self.doubled.insert((coll_id, index), (n, seq.loop_frame));
                    }
                }
                Some(SequenceVisual {
                    object_bounds: seq.bounds.world(),
                    sequence: seq,
                    frames,
                })
            });
            if built.is_none() {
                warn!("TFL: sequence {index} of {coll_id} not found");
            }
            if let (Some(v), Ok(sequence)) = (&built, u8::try_from(index)) {
                let (nv, nf) = (
                    usize::from(v.sequence.num_views),
                    usize::from(v.sequence.frames_per_view).max(1),
                );
                let coll = self.collection(map, coll_id);
                let k = if self.doubled.contains_key(&(coll_id, index)) {
                    2
                } else {
                    1
                };
                self.instance_flags
                    .entry((coll_id, sequence))
                    .or_insert_with(|| {
                        (0..nv * nf * k)
                            .map(|i| {
                                let (view, frame) = (i / (nf * k), i % (nf * k) / k);
                                coll.as_ref()
                                    .map_or(0, |c| c.instance_flags(index, view, frame))
                            })
                            .collect()
                    });
            }
            self.sequences.insert(key, built);
        }
        self.sequences.get(&key)?.as_ref()
    }

    fn object(
        &mut self,
        map: &TflMap,
        (coll, sequence): (FourCC, usize),
        look: Option<UnitLook>,
        (world, facing): ([i32; 3], u16),
        scale: u16,
        index: usize,
    ) -> Option<DrawnObject> {
        let colours = self.look_colours(look);
        self.sequence(map, coll, sequence, colours)?;
        let cells = (map.tfl_mesh.cell_width, map.tfl_mesh.cell_height);
        Some(drawn_object(
            index,
            world,
            cells,
            (facing, 0, 0),
            (coll, u8::try_from(sequence).ok()?),
            (0, false, scale),
            look,
        ))
    }

    fn place(
        &mut self,
        map: &TflMap,
        marker: &MarkerEntry,
        shape: (FourCC, usize),
        look: Option<UnitLook>,
        scale: u16,
    ) -> bool {
        let index = self.out.sprites.len();
        let world = [marker.x, marker.y, marker.z];
        let facing = marker.yaw as u16;
        let Some(o) = self.object(map, shape, look, (world, facing), scale, index) else {
            return false;
        };
        let colours = self.look_colours(look);
        let s = self.sequence(map, shape.0, shape.1, colours);
        let any_view = s.is_some_and(|s| {
            s.frames
                .first()
                .is_some_and(|f| f.views.iter().any(Option::is_some))
        });
        if any_view {
            self.out.sprites.push(o);
        }
        any_view
    }

    pub(crate) fn take_grown(&mut self, seen: &mut usize) -> bool {
        let grown = self.out.images.len() != *seen;
        *seen = self.out.images.len();
        grown
    }

    pub(crate) fn preload(
        &mut self,
        map: &TflMap,
        coll: FourCC,
        sequence: usize,
        look: Option<UnitLook>,
    ) {
        let colours = self.look_colours(look);
        self.sequence(map, coll, sequence, colours);
    }

    pub(crate) fn local_sprite(
        &mut self,
        map: &TflMap,
        collection: FourCC,
        sequence: i16,
        world: [i32; 3],
    ) -> Option<DrawnObject> {
        let sequence = usize::try_from(sequence).ok()?;
        self.object(
            map,
            (collection, sequence),
            None,
            (world, 0),
            0x100,
            usize::MAX,
        )
    }

    pub(crate) fn particle_sequence(
        &mut self,
        map: &TflMap,
        core: FourCC,
        index: usize,
    ) -> Option<(SequenceVisual, Vec<u32>)> {
        let coll_id = four(&map.tag(b"core", core)?, 0)?;
        let mut visual = self
            .sequence(map, coll_id, index, Some((core, 0, None)))?
            .clone();
        if self.doubled.contains_key(&(coll_id, index)) {
            visual.frames = visual.frames.into_iter().step_by(2).collect();
        }
        let coll = self.collection(map, coll_id)?;
        let seq = &visual.sequence;
        let flags = (0..usize::from(seq.num_views))
            .flat_map(|v| (0..usize::from(seq.frames_per_view)).map(move |f| (v, f)))
            .map(|(v, f)| coll.instance_flags(index, v, f))
            .collect();
        Some((visual, flags))
    }

    pub(crate) fn decode(&mut self, map: &TflMap, o: &DrawnObject) -> bool {
        let colours = self.colours_of(o);
        self.sequence(map, o.collection, usize::from(o.sequence), colours)
            .is_some()
    }

    pub(crate) fn doubled_frame(
        &self,
        o: &DrawnObject,
        live: Option<&mandrake_tfl_sim::objects::TflObject>,
        fraction: i32,
    ) -> i32 {
        let Some(&(n, loop_frame)) = self.doubled.get(&(o.collection, usize::from(o.sequence)))
        else {
            return o.frame;
        };
        let shown = usize::try_from(o.frame).unwrap_or(0) % n.max(1);
        let drawn = 2 * shown as i32;
        if !crate::sprite_interp::enabled() {
            return drawn;
        }
        let Some(live) = live.filter(|l| {
            l.collection == o.collection && i32::from(l.sequence) == i32::from(o.sequence)
        }) else {
            return drawn;
        };
        let phase = crate::sprite_interp::AnimPhase::of_sim(
            usize::try_from(live.frame).unwrap_or(0),
            i32::from(live.tick),
            i32::from(live.ticks_per_frame),
            live.flags & OBJECT_REVERSED != 0,
        );
        phase.doubled_index(n, loop_frame, fraction, FRACTION_ONE, shown) as i32
    }
}

const OBJECT_REVERSED: u32 = 0x80000;

fn doubled_visuals(
    frames: &[crate::unit_anim::FrameVisual],
    coll: &CollectionTag,
    seq: &mandrake_tag::CollectionSequence,
    cache: &crate::sprite_interp::CollectionCache,
    images: &mut Vec<image::RgbaImage>,
    offsets: &mut Vec<(i16, i16)>,
) -> Vec<crate::unit_anim::FrameVisual> {
    let mut made = 0usize;
    let out = crate::sprite_interp::doubled_frames(
        frames,
        seq.loop_frame,
        |f| f.views.as_slice(),
        |f, views| crate::unit_anim::FrameVisual { views, ..f.clone() },
        |view, i, j| {
            let m = cache.get(seq.index, view, i, j)?;
            let src = coll.frame(seq.bitmap_index(view, i)?)?;
            if !src.wide_pixels.is_empty() {
                return None;
            }
            let bitmap = mandrake_tag::BitmapFrame {
                width: m.width,
                height: m.height,
                offset_x: m.offset_x,
                offset_y: m.offset_y,
                pixels: m.pixels.clone(),
                ..src.clone()
            };
            let rgba =
                crate::sprite_manager::sprite_texels(&bitmap, coll.palette(), &coll.color_table);
            let img = image::RgbaImage::from_raw(
                u32::from(m.width.max(1)),
                u32::from(m.height.max(1)),
                rgba,
            )?;
            let atlas_index = images.len();
            images.push(img);
            offsets.push((m.offset_x, m.offset_y));
            made += 1;
            Some(crate::unit_anim::ViewSprite {
                atlas_index,
                world_size: crate::sprite_manager::object_bitmap_world_size(
                    &bitmap,
                    seq,
                    coll.pixels_to_world,
                    0x100,
                ),
            })
        },
    );
    debug!(
        "sprite interpolation: sequence {} doubled, {made} in-betweens",
        seq.index
    );
    out
}

pub(crate) fn drawn_object(
    index: usize,
    world: [i32; 3],
    cells: (u32, u32),
    (facing, view, frame): (u16, u8, i32),
    (collection, sequence): (FourCC, u8),
    (flags, mirrored, scale): (u32, bool, u16),
    look: Option<UnitLook>,
) -> DrawnObject {
    DrawnObject {
        kind: ObjectKind::Dormant,
        object: index as i32,
        world,
        position: world_render_position(world, cells),
        velocity: [0; 3],
        facing,
        collection,
        sequence,
        frame,
        view,
        scale: i32::from(scale),
        flags,
        hidden: false,
        mirror_x: mirrored,
        mirror_y: false,
        permutation: look.map_or(0, |l| l.permutation as i16),
        player: look.map_or(-1, |l| l.player),
        shading_slot: 0,
        core: look.map(|l| l.core),
        transfer: (0, 0, 0),
        monster: None,
    }
}

pub(crate) fn transfer(mode: u8, count: u8, period: u8) -> (u8, u8, u8) {
    match mode {
        1 | 2 => (mode, count, period),
        _ => (0, 0, 0),
    }
}

pub(crate) struct TflObjectSprites<'a> {
    pub(crate) sprites: &'a TflSprites,
    pub(crate) atlas: &'a TextureAtlas,
    pub(crate) reflection: Option<crate::object_sprites::SurfaceReflection>,
}

impl TflSprites {
    pub(crate) fn decoded(&self, o: &DrawnObject) -> Option<&SequenceVisual> {
        let colours = self.colours_of(o);
        self.sequences
            .get(&(o.collection, usize::from(o.sequence), colours))?
            .as_ref()
    }
}

impl ObjectSprites for TflObjectSprites<'_> {
    fn sequence(&self, o: &DrawnObject) -> Option<&SequenceVisual> {
        self.sprites.decoded(o)
    }

    fn instance_flags(&self, o: &DrawnObject, index: usize) -> u32 {
        self.sprites
            .instance_flags
            .get(&(o.collection, o.sequence))
            .and_then(|f| f.get(index))
            .copied()
            .unwrap_or(0)
    }

    fn atlas(&self) -> &TextureAtlas {
        self.atlas
    }

    fn atlas_index(&self, _: &DrawnObject, base: usize) -> usize {
        base
    }

    fn table_core(&self, _: &DrawnObject) -> crate::media::CoreRef {
        crate::media::CoreRef::default()
    }

    fn media(&self) -> Option<&crate::media::MapMedia> {
        None
    }

    fn surface_reflection(
        &self,
        _: &DrawnObject,
    ) -> Option<crate::object_sprites::SurfaceReflection> {
        self.reflection
    }

    fn rules(&self) -> &'static crate::world_view::ClientRules {
        &crate::world_view::ClientRules::TFL
    }
}

pub(crate) fn core_tint(core: &[u8]) -> Option<(i32, [u16; 3])> {
    let w = |o: usize| Some(u16::from_be_bytes(core.get(o..o + 2)?.try_into().ok()?));
    let amount = w(0x06)? as i16;
    if amount == 0 || w(0x14e)? & 2 != 0 {
        return None;
    }
    Some((i32::from(amount), [w(0x148)?, w(0x14a)?, w(0x14c)?]))
}

pub(crate) fn tint_palette(palette: &mut [[u8; 4]], (a, tint): (i32, [u16; 3])) {
    for e in palette.iter_mut() {
        for k in 0..3 {
            let c = i32::from(e[k]) * 0x101;
            let t = i32::from(tint[k] >> 8);
            let v = ((t * (c >> 8) * a + c * (0x100 - a)) >> 8).clamp(0, 0xffff);
            e[k] = (v >> 8) as u8;
        }
    }
}

fn player_colours(map: &TflMap, player: i16) -> Option<PlayerColours> {
    let player = map
        .film
        .as_ref()?
        .players
        .iter()
        .find(|p| i16::try_from(p.slot).ok() == Some(player))
        .filter(|p| p.flags & 1 != 0)?;
    let on = |c: &[u16; 4]| c[3] & 4 == 0;
    (on(&player.primary) || on(&player.secondary)).then_some((player.primary, player.secondary))
}

pub(crate) fn tfl_sprites(map: &TflMap, team: i16) -> TflSprites {
    let mut b = TflSprites::new(map);
    let mesh = &map.mesh;
    let placed = |m: &&MarkerEntry| m.flags & MARKER_NOT_PLACED == 0;
    let mut skipped_items = 0;
    for marker in mesh.scenery_markers().filter(placed) {
        let Some(scen_id) = mesh.palette_tag_for_marker(marker) else {
            continue;
        };
        let Some(scen) = map.tag(b"scen", scen_id) else {
            warn!("TFL: scen {scen_id} not found");
            continue;
        };
        let Some(flags) = scen
            .get(0..4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
        else {
            warn!("TFL: scen {scen_id} too short ({} bytes)", scen.len());
            continue;
        };
        if flags & SCEN_NET_GAME_ITEM != 0 {
            debug!("TFL: scen {scen_id} (flags {flags:#x}) is a net game item; not placed");
            skipped_items += 1;
            continue;
        }
        let Some(coll) = four(&scen, 0x08)
            .and_then(|core| map.tag(b"core", core))
            .and_then(|core| four(&core, 0))
        else {
            continue;
        };
        let (Some(seq), Some(scale)) = (i16_at(&scen, 0x3c), i16_at(&scen, 0x22)) else {
            continue;
        };
        let Ok(seq) = usize::try_from(seq) else {
            debug!("TFL: scen {scen_id} has no sequence");
            continue;
        };
        if b.place(map, marker, (coll, seq), None, scale as u16) {
            b.out.scenery += 1;
        }
    }
    for marker in mesh
        .markers_by_type(MarkerPaletteEntry::TYPE_UNIT)
        .filter(placed)
    {
        let Some(entry) = mesh.palette_for_marker(marker) else {
            continue;
        };
        if entry.team_index() != team {
            continue;
        }
        let unit = map.tag(b"unit", entry.tag_id);
        let Some(mons) = unit
            .as_ref()
            .and_then(|u| four(u, 0))
            .and_then(|m| map.tag(b"mons", m))
        else {
            warn!("TFL: unit {} or its mons not found", entry.tag_id);
            continue;
        };
        let (Some(coll), Some(standing)) = (four(&mons, 0x04), i16_at(&mons, 0x08)) else {
            continue;
        };
        let Ok(standing) = usize::try_from(standing) else {
            continue;
        };
        debug!(
            "TFL unit {} (marker {}): collection {coll}, standing sequence {standing}",
            entry.tag_id, marker.marker_id
        );
        let look = unit
            .as_ref()
            .and_then(|u| four(u, 0x04))
            .filter(|_| player_colours(map, team).is_some())
            .map(|core| UnitLook {
                core,
                player: team,
                permutation: 0,
            });
        if b.place(map, marker, (coll, standing), look, 0x100) {
            b.out.units += 1;
        }
    }
    info!(
        "TFL objects: {} scenery ({skipped_items} net game items left out), {} units of team {team}, {} bitmaps",
        b.out.scenery,
        b.out.units,
        b.out.images.len()
    );
    b
}

#[cfg(test)]
mod tint_tests {
    use super::*;

    #[test]
    fn the_core_tint_mixes_towards_the_tint_by_its_amount() {
        let mut core = vec![0u8; 0x180];
        assert_eq!(core_tint(&core), None);
        core[6..8].copy_from_slice(&0x80i16.to_be_bytes());
        core[0x148..0x14a].copy_from_slice(&0xff00u16.to_be_bytes());
        let tint = core_tint(&core).unwrap();
        assert_eq!(tint, (0x80, [0xff00, 0, 0]));
        let mut pal = vec![[0x80u8, 0x80, 0x80, 255]];
        tint_palette(&mut pal, tint);
        assert_eq!(pal[0], [0x80, 0x40, 0x40, 255]);
        core[0x14e..0x150].copy_from_slice(&2u16.to_be_bytes());
        assert_eq!(core_tint(&core), None, "flag 2: the map's tint");
    }
}

#[cfg(test)]
mod transfer_tests {
    #[test]
    fn only_modes_1_and_2_fade() {
        assert_eq!(super::transfer(2, 5, 0x1e), (2, 5, 0x1e));
        assert_eq!(super::transfer(1, 3, 0x3c), (1, 3, 0x3c));
        for mode in [0, 3, 5, 6, 7] {
            assert_eq!(super::transfer(mode, 5, 0x1e), (0, 0, 0));
        }
    }
}
