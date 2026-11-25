use std::collections::{BTreeMap, BTreeSet};

use mandrake_common::FourCC;
use mandrake_sim::flight::Stain;
use mandrake_tag::{
    CollectionTag, ProjectileDefinition, ProjectileGroupDefinition, TagArchive,
    view_for_relative_angle,
};
use mandrake_view::billboard::BillboardInstance;
use mandrake_view::landscape::StainBitmap;
use mandrake_view::projectile::{fade_ratio, transfer_transparency};

use crate::object_sprites::ObjectFrame;
use crate::sprite_manager::{ClientSpriteManager, sequence_frame_visuals};
use crate::unit_anim::SequenceVisual;
use mandrake_tag::MarkerEntry;

pub const SEQUENCE_FLAG_STORED_VIEW: u32 = 4;

const MAX_DEPTH: usize = 4;

#[derive(Default)]
pub struct CorpseSprites {
    pub sequences: BTreeMap<(FourCC, u8), SequenceVisual>,
    pub instance_flags: BTreeMap<(FourCC, u8), Vec<u32>>,
    pub stains: BTreeMap<(FourCC, i16), StainSequence>,
}

impl CorpseSprites {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_sequence(
        &mut self,
        coll_id: FourCC,
        coll: &CollectionTag,
        sequence: u8,
        object_bounds: &mut crate::object_bounds::SequenceBoundsTable<'_>,
        images: &mut Vec<image::RgbaImage>,
        offsets: &mut Vec<(i16, i16)>,
        atlas_of: &mut BTreeMap<(FourCC, usize), usize>,
    ) {
        if self.sequences.contains_key(&(coll_id, sequence)) {
            return;
        }
        let Some(seq) = coll.sequence(usize::from(sequence)).cloned() else {
            return;
        };
        let (nv, nf) = (usize::from(seq.num_views), usize::from(seq.frames_per_view));
        self.instance_flags.insert(
            (coll_id, sequence),
            (0..nv * nf)
                .map(|i| coll.instance_flags(usize::from(sequence), i / nf.max(1), i % nf.max(1)))
                .collect(),
        );
        let frames = sequence_frame_visuals(
            coll_id,
            coll,
            &seq,
            mandrake_view::selection::OBJECT_SCALE_ONE as u16,
            images,
            offsets,
            atlas_of,
        );
        self.sequences.insert(
            (coll_id, sequence),
            SequenceVisual {
                object_bounds: object_bounds.world_bounds(
                    coll_id,
                    usize::from(sequence),
                    seq.bounds.world(),
                ),
                sequence: seq,
                frames,
            },
        );
    }
}

#[derive(Clone, Debug)]
pub struct StainSequence {
    pub views: usize,
    pub frames: usize,
    pub bitmaps: Vec<Option<StainBitmap>>,
    pub color_table: Vec<[u16; 4]>,
}

impl StainSequence {
    pub fn from_collection(coll: &CollectionTag, sequence: usize) -> Option<Self> {
        let seq = coll.sequence(sequence)?;
        let (views, frames) = (usize::from(seq.num_views), usize::from(seq.frames_per_view));
        let mut bitmaps = Vec::with_capacity(views * frames);
        for view in 0..views {
            for frame in 0..frames {
                bitmaps.push(
                    seq.bitmap_index(view, frame)
                        .and_then(|b| coll.frame(b))
                        .zip(coll.shadow_frame_for_sequence(sequence, frame))
                        .filter(|(c, m)| (c.width, c.height) == (m.width, m.height))
                        .map(|(c, m)| StainBitmap {
                            width: c.width,
                            height: c.height,
                            key_x: c.offset_x,
                            key_y: c.offset_y,
                            color: c.pixels.clone(),
                            mask: m.pixels.clone(),
                        }),
                );
            }
        }
        Some(Self {
            views,
            frames,
            bitmaps,
            color_table: coll.color_table.clone(),
        })
    }

    pub fn bitmap(&self, random: u16) -> Option<&StainBitmap> {
        if self.views == 0 || self.frames == 0 {
            return None;
        }
        let (v, f) = (
            usize::from(random) % self.views,
            usize::from(random) % self.frames,
        );
        self.bitmaps.get(v * self.frames + f)?.as_ref()
    }
}

pub trait DeathGroupTags {
    fn group(&self, id: FourCC) -> Option<ProjectileGroupDefinition>;
    fn projectile(&self, id: FourCC) -> Option<ProjectileDefinition>;
}

pub struct RawTags<'a>(pub &'a dyn Fn([u8; 4], FourCC) -> Option<Vec<u8>>);

impl DeathGroupTags for RawTags<'_> {
    fn group(&self, id: FourCC) -> Option<ProjectileGroupDefinition> {
        (self.0)(*b"prgr", id).and_then(|d| ProjectileGroupDefinition::from_bytes(id, &d).ok())
    }
    fn projectile(&self, id: FourCC) -> Option<ProjectileDefinition> {
        (self.0)(*b"proj", id).and_then(|d| ProjectileDefinition::from_bytes(id, &d).ok())
    }
}

pub struct EngineDeathGroups<'a> {
    pub roots: Vec<FourCC>,
    catalog: &'a mandrake_sim::projectile_groups::ProjectileCatalog,
}

impl<'a> EngineDeathGroups<'a> {
    pub fn of(world: &'a mandrake_sim::engine_rng::EngineWorld) -> Self {
        let mut roots = Vec::new();
        for d in &world.monster_definitions {
            for at in [0x1a4usize, 0x194, 0x3c] {
                let Some(b) = d.ai.raw.get(at..at + 4) else {
                    continue;
                };
                let b: [u8; 4] = b.try_into().expect("4 bytes");
                if b != [0xff; 4] && b != [0; 4] {
                    roots.push(FourCC::new(b));
                }
            }
        }
        roots.sort_unstable_by_key(|c| c.as_str().to_owned());
        roots.dedup();
        Self {
            roots,
            catalog: &world.catalog,
        }
    }
}

impl<'a> EngineDeathGroups<'a> {
    pub fn catalog(&self) -> &'a mandrake_sim::projectile_groups::ProjectileCatalog {
        self.catalog
    }
}

impl EngineDeathGroups<'_> {
    fn catalog_flight_sequences(&self) -> Vec<(FourCC, u8)> {
        let mut out: Vec<(FourCC, u8)> = self
            .catalog
            .projectiles
            .values()
            .filter_map(|p| Some((p.collection?, u8::try_from(p.sequences[0]).ok()?)))
            .collect();
        out.sort_unstable_by_key(|&(c, s)| (c.as_str().to_owned(), s));
        out.dedup();
        out
    }

    fn catalog_stain_sequences(&self) -> Vec<(FourCC, i16)> {
        let mut out: Vec<(FourCC, i16)> = self
            .catalog
            .projectiles
            .values()
            .filter_map(|p| Some((p.collection?, p)))
            .flat_map(|(c, p)| [(c, p.sequences[1]), (c, p.sequences[2])])
            .filter(|&(_, s)| s >= 0)
            .collect();
        out.sort_unstable_by_key(|&(c, s)| (c.as_str().to_owned(), s));
        out.dedup();
        out
    }
}

impl DeathGroupTags for EngineDeathGroups<'_> {
    fn group(&self, id: FourCC) -> Option<ProjectileGroupDefinition> {
        self.catalog.groups.get(&id).cloned()
    }
    fn projectile(&self, id: FourCC) -> Option<ProjectileDefinition> {
        self.catalog.projectiles.get(&id).cloned()
    }
}

pub fn death_sequences(tags: &dyn DeathGroupTags, roots: &[FourCC]) -> Vec<(FourCC, u8)> {
    let mut out = Vec::new();
    let mut seen_groups = BTreeSet::new();
    let mut seen_projectiles = BTreeSet::new();
    for &root in roots {
        walk_group(
            tags,
            root,
            0,
            &mut seen_groups,
            &mut seen_projectiles,
            &mut out,
        );
    }
    out.sort_unstable_by_key(|&(c, s)| (c.as_str().to_owned(), s));
    out.dedup();
    out
}

fn walk_group(
    tags: &dyn DeathGroupTags,
    id: FourCC,
    depth: usize,
    seen_groups: &mut BTreeSet<FourCC>,
    seen_projectiles: &mut BTreeSet<FourCC>,
    out: &mut Vec<(FourCC, u8)>,
) {
    if depth > MAX_DEPTH || !seen_groups.insert(id) {
        return;
    }
    let Some(group) = tags.group(id) else {
        return;
    };
    for part in &group.parts {
        for p in [part.projectile, part.projectile_18].into_iter().flatten() {
            walk_projectile(tags, p, depth, seen_groups, seen_projectiles, out);
        }
    }
}

fn walk_projectile(
    tags: &dyn DeathGroupTags,
    id: FourCC,
    depth: usize,
    seen_groups: &mut BTreeSet<FourCC>,
    seen_projectiles: &mut BTreeSet<FourCC>,
    out: &mut Vec<(FourCC, u8)>,
) {
    if !seen_projectiles.insert(id) {
        return;
    }
    let Some(p) = tags.projectile(id) else {
        return;
    };
    if let (Some(c), Ok(s)) = (p.collection, u8::try_from(p.sequences[0])) {
        out.push((c, s));
    }
    if let Some(r) = p.projectile_5c {
        walk_projectile(tags, r, depth + 1, seen_groups, seen_projectiles, out);
    }
    for g in [p.detonation_group, p.groups[0]].into_iter().flatten() {
        walk_group(tags, g, depth + 1, seen_groups, seen_projectiles, out);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn load_corpse_sprites(
    archive: &TagArchive,
    roots: &[FourCC],
    engine: Option<&EngineDeathGroups<'_>>,
    object_bounds: &mut crate::object_bounds::SequenceBoundsTable<'_>,
    get_collection: &mut dyn FnMut(FourCC) -> Option<std::sync::Arc<CollectionTag>>,
    images: &mut Vec<image::RgbaImage>,
    offsets: &mut Vec<(i16, i16)>,
    atlas_of: &mut BTreeMap<(FourCC, usize), usize>,
) -> CorpseSprites {
    let lookup = |kind: [u8; 4], id: FourCC| {
        let e = archive.find_tag_by_id(FourCC::new(kind), id)?;
        archive.get_tag_data(e).ok()
    };
    let raw = RawTags(&lookup);
    let (tags, roots): (&dyn DeathGroupTags, &[FourCC]) = match engine {
        Some(e) => (e, &e.roots),
        None => (&raw, roots),
    };
    let mut sprites = CorpseSprites::default();
    let mut sequences = death_sequences(tags, roots);
    if let Some(e) = engine {
        sequences.extend(e.catalog_flight_sequences());
        sequences.sort_unstable_by_key(|&(c, s)| (c.as_str().to_owned(), s));
        sequences.dedup();
        for (coll_id, sequence) in e.catalog_stain_sequences() {
            if let Some(stain) = get_collection(coll_id)
                .and_then(|c| StainSequence::from_collection(&c, sequence as usize))
            {
                sprites.stains.insert((coll_id, sequence), stain);
            }
        }
    }
    for (coll_id, sequence) in sequences {
        let Some(coll) = get_collection(coll_id) else {
            continue;
        };
        sprites.insert_sequence(
            coll_id,
            &coll,
            sequence,
            object_bounds,
            images,
            offsets,
            atlas_of,
        );
    }
    tracing::info!(
        "Projectile sprites: {} sequences and {} stain sequences loaded ({} death groups)",
        sprites.sequences.len(),
        sprites.stains.len(),
        roots.len()
    );
    sprites
}

pub fn object_view(
    sequence: &SequenceVisual,
    stored_view: u8,
    relative: u16,
    mirrored: bool,
) -> (usize, bool) {
    let seq = &sequence.sequence;
    let nv = usize::from(seq.num_views).max(1);
    let view = if seq.flags & SEQUENCE_FLAG_STORED_VIEW != 0 {
        usize::from(stored_view) % nv
    } else {
        view_for_relative_angle(relative, nv, mirrored)
    };
    seq.resolve_view(view)
}

pub struct ProjectileFrame<'a> {
    pub sprite_mgr: &'a ClientSpriteManager,
    pub camera_ground: (i32, i32),
    pub axes: ([f32; 3], [f32; 3], [f32; 3]),
    pub tick: i32,
    pub local_time: i32,
    pub stopped: bool,
}

pub fn object_transparency(transfer: (u8, u8, u8)) -> u16 {
    let (mode, count, period) = transfer;
    if mode == 0 {
        return 0;
    }
    fade_ratio(count, period)
        .and_then(|r| transfer_transparency(mode, r, &mandrake_math::engine::trig_tables().sine))
        .unwrap_or(0)
}

impl<'a> ProjectileFrame<'a> {
    pub fn objects(&self) -> ObjectFrame<'a, ClientSpriteManager> {
        ObjectFrame {
            sprites: self.sprite_mgr,
            camera_ground: self.camera_ground,
            axes: self.axes,
            tick: self.tick,
            local_time: self.local_time,
            stopped: self.stopped,
        }
    }
}

pub(crate) fn reflected_origin(
    mut base: BillboardInstance,
    surface: i16,
    up: [f32; 3],
) -> BillboardInstance {
    let m = f32::from(surface) / MarkerEntry::WORLD_ONE;
    let z = base.position[1];
    let mirrored = 2.0 * m - z;
    if z > m {
        base.position[1] = mirrored;
    }
    let d = (mirrored - base.position[1]) * up[1];
    for (p, u) in base.position.iter_mut().zip(up) {
        *p += u * d;
    }
    base
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandscapeEdit {
    Stain(Stain),
    Bloody { tick: u32, point: (i32, i32) },
    Footstep { point: (i32, i32) },
}

#[cfg(test)]
mod tests {
    use super::*;

    use mandrake_tag::CollectionSequence;

    fn visual(flags: u32, views: u16) -> SequenceVisual {
        SequenceVisual {
            sequence: CollectionSequence {
                index: 0,
                name: "s".into(),
                flags,
                pixels_to_world: 1,
                num_views: views,
                frames_per_view: 1,
                ticks_per_frame: 1,
                key_frame: -1,
                loop_frame: 0,
                bitmap_indices: vec![0; usize::from(views)],
                shadow_indices: vec![-1],
                bounds: Default::default(),
            },
            frames: Vec::new(),
            object_bounds: Default::default(),
        }
    }

    #[test]
    fn a_flag_4_sequence_draws_the_stored_view_whatever_the_camera() {
        let seq = visual(SEQUENCE_FLAG_STORED_VIEW, 5);
        for facing in [0u16, 0x4000, 0xc000] {
            assert_eq!(object_view(&seq, 3, facing, false), (3, false));
        }
    }

    #[test]
    fn a_sequence_without_flag_4_follows_the_camera() {
        let seq = visual(0, 8);
        let a = object_view(&seq, 3, 0, false);
        let b = object_view(&seq, 3, 0x8000, false);
        assert_ne!(a, b, "the view depends on the bearing to the camera");
    }

    #[test]
    fn the_fade_of_an_object() {
        assert_eq!(object_transparency((0, 3, 21)), 0);
        assert_eq!(object_transparency((1, 1, 28)), 0x0924);
        assert_eq!(object_transparency((1, 28, 28)), 0xffff);
        assert_eq!(object_transparency((1, 1, 0)), 0);
    }

    #[test]
    fn a_stain_picks_view_and_frame_from_its_random() {
        let bitmap = |w: u16| {
            Some(StainBitmap {
                width: w,
                height: 1,
                key_x: 0,
                key_y: 0,
                color: vec![0; usize::from(w)],
                mask: vec![0; usize::from(w)],
            })
        };
        let s = StainSequence {
            views: 1,
            frames: 3,
            bitmaps: vec![bitmap(1), bitmap(2), None],
            color_table: Vec::new(),
        };
        assert_eq!(s.bitmap(46339).unwrap().width, 2);
        assert_eq!(s.bitmap(0).unwrap().width, 1);
        assert!(s.bitmap(2).is_none());
    }
}
