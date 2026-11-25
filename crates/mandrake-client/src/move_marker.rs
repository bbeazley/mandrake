use std::path::Path;

use mandrake_common::FourCC;
use mandrake_math::{Fixed16, Vec3Fixed};
use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::{MovementHeader, Unmodelled};
use mandrake_tag::{CollectionTag, MeshHeightMode, TagLookup};
use mandrake_view::billboard::BillboardInstance;

pub const MAX_LOCAL_SPRITES: usize = 0x7e;
pub const TFL_MAX_LOCAL_SPRITES: usize = 0x1e;
pub const MARKER_LIFETIME: i32 = 7;

pub const SEQUENCE_DESTINATION: i16 = 0;
pub const SEQUENCE_WAYPOINT: i16 = 1;
pub const SEQUENCE_ATTACK: i16 = 2;
pub const SEQUENCE_IMPASSABLE: i16 = 4;
pub const MARKER_SEQUENCES: [i16; 4] = [
    SEQUENCE_DESTINATION,
    SEQUENCE_WAYPOINT,
    SEQUENCE_ATTACK,
    SEQUENCE_IMPASSABLE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSprite {
    pub collection: FourCC,
    pub sequence: i16,
    pub point: [i32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSprites {
    entries: Vec<LocalSprite>,
    capacity: usize,
    timer: i32,
}

impl Default for LocalSprites {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveMode {
    StartTimer,
    ClearUnlessTimed,
}

impl LocalSprites {
    pub fn new() -> Self {
        Self::with_capacity(MAX_LOCAL_SPRITES)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity,
            timer: -1,
        }
    }

    pub fn new_local_sprite(&mut self, point: [i32; 3], collection: FourCC, sequence: i16) {
        let sprite = LocalSprite {
            collection,
            sequence,
            point,
        };
        if self.timer >= 0 {
            self.timer = -1;
            self.entries.clear();
            self.entries.push(sprite);
        } else if self.entries.len() < self.capacity {
            self.entries.push(sprite);
        }
    }

    pub fn remove_local_sprites(&mut self, mode: RemoveMode, now: i32) {
        match mode {
            RemoveMode::StartTimer => self.timer = now,
            RemoveMode::ClearUnlessTimed => {
                if self.timer < 0 {
                    self.entries.clear();
                }
            }
        }
    }

    pub fn drawn(&self, now: i32) -> &[LocalSprite] {
        if self.timer < 0 || now.wrapping_sub(self.timer) <= MARKER_LIFETIME {
            &self.entries
        } else {
            &[]
        }
    }

    pub fn entries(&self) -> &[LocalSprite] {
        &self.entries
    }

    pub fn timer(&self) -> Option<i32> {
        (self.timer >= 0).then_some(self.timer)
    }
}

pub fn marker_sequence(mask: u16, terrain: Option<usize>, j: usize, k: usize, order: bool) -> i16 {
    let closed = terrain.is_some_and(|t| t < 16 && (mask >> t) & 1 != 0);
    if closed {
        SEQUENCE_IMPASSABLE
    } else if order && j + 1 == k {
        SEQUENCE_DESTINATION
    } else {
        SEQUENCE_WAYPOINT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskRule {
    EachMonster,
    AllMonsters,
}

impl MaskRule {
    pub fn mask(self, masks: &[u16], i: usize) -> u16 {
        match self {
            Self::EachMonster => masks.get(i).copied().unwrap_or(0),
            Self::AllMonsters => masks.iter().fold(0xffff, |a, &m| a & m),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn place_markers(
    sprites: &mut LocalSprites,
    collection: FourCC,
    places: &[Vec<(i32, i32)>],
    masks: &[u16],
    rule: MaskRule,
    kind: MarkerKind,
    terrain: impl Fn(i32, i32) -> Option<usize>,
    height: impl Fn(i32, i32) -> i32,
) -> usize {
    let k = places.len();
    let mut made = 0;
    for i in 0..masks.len() {
        let mask = rule.mask(masks, i);
        for (j, slots) in places.iter().enumerate() {
            let Some(&(x, y)) = slots.get(i) else {
                continue;
            };
            let sequence = marker_sequence(mask, terrain(x, y), j, k, kind == MarkerKind::Order);
            sprites.new_local_sprite([x, y, height(x, y)], collection, sequence);
            made += 1;
        }
    }
    made
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    Order,
    Preview,
}

pub fn marker_height(sim: &Simulation, x: i32, y: i32) -> i32 {
    i32::from(sim.engine.mesh_height(x, y, MeshHeightMode::Surface) as i16)
}

pub fn make_markers(
    sprites: &mut LocalSprites,
    sim: &Simulation,
    unit_ids: &[u32],
    header: MovementHeader,
    points: &[(i32, i32)],
    kind: MarkerKind,
) -> Result<usize, Unmodelled> {
    let (destinations, _facings) = sim.movement_command_destinations(unit_ids, points, header)?;

    let k = points.len().min(destinations.len());
    let masks: Vec<u16> = unit_ids
        .iter()
        .map(|&id| sim.monster_impassable_mask(id).unwrap_or(0))
        .collect();
    Ok(place_markers(
        sprites,
        mandrake_tag::interface::STANDARD_INTERFACE_COLLECTION,
        &destinations[..k],
        &masks,
        MaskRule::EachMonster,
        kind,
        |x, y| sim.terrain_type_at(x, y),
        |x, y| marker_height(sim, x, y),
    ))
}

pub fn attack_ground_marker(sprites: &mut LocalSprites, point: [i32; 3], now: i32) {
    attack_ground_marker_of(
        sprites,
        mandrake_tag::interface::STANDARD_INTERFACE_COLLECTION,
        point,
        now,
    );
}

pub fn attack_ground_marker_of(
    sprites: &mut LocalSprites,
    collection: FourCC,
    point: [i32; 3],
    now: i32,
) {
    sprites.new_local_sprite(point, collection, SEQUENCE_ATTACK);
    sprites.remove_local_sprites(RemoveMode::StartTimer, now);
}

#[allow(clippy::too_many_arguments)]
pub fn ground_click(
    sprites: &mut LocalSprites,
    sim: &mut Simulation,
    selection: &[u32],
    point: (i32, i32),
    facing: Option<u16>,
    add_waypoint: bool,
    now: i32,
) -> Result<Option<mandrake_sim::LocalMovement>, Unmodelled> {
    if selection.is_empty() {
        return Ok(None);
    }
    let sent = crate::local_input::ground_move_click(sim, selection, point, facing, add_waypoint)?;
    match &sent {
        Some(c) => {
            if let Err(e) = make_markers(
                sprites,
                sim,
                &c.unit_ids,
                c.header,
                &c.points,
                MarkerKind::Order,
            ) {
                tracing::warn!("Move markers not made ({e})");
            }
            sprites.remove_local_sprites(RemoveMode::StartTimer, now);
            sprites.remove_local_sprites(RemoveMode::ClearUnlessTimed, now);
        }
        None => {
            sprites.remove_local_sprites(RemoveMode::ClearUnlessTimed, now);
            let header = sim.local_movement_header(selection, facing)?;
            let points = sim.local_waypoints.clone();
            if let Err(e) = make_markers(
                sprites,
                sim,
                selection,
                header,
                &points,
                MarkerKind::Preview,
            ) {
                tracing::warn!("Waypoint preview not made ({e})");
            }
        }
    }
    Ok(sent)
}

pub fn load_interface_collection(game_dir: &Path) -> Option<CollectionTag> {
    let tags = TagLookup::open(&mandrake_tag::engine_interface_tag_files(game_dir, &[]));
    let inte = tags.get(
        FourCC(*b"inte"),
        mandrake_tag::interface::STANDARD_INTERFACE_DEFINITION,
    )?;
    let id = mandrake_tag::interface::InterfaceDefinition::from_bytes(&inte)
        .ok()?
        .collection?;
    CollectionTag::from_bytes(&tags.get(FourCC(*b".256"), id)?).ok()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkerSprite {
    pub atlas_index: usize,
    pub world_size: [f32; 2],
}

pub fn render_position(sim: &Simulation, p: [i32; 3]) -> [f32; 3] {
    let w: Vec3Fixed = sim.command_point_to_world(p[0], p[1]);
    let z = Fixed16::from_raw(p[2].wrapping_mul(65536 / 512));
    [w.x.to_f32(), z.to_f32(), w.z.to_f32()]
}

pub fn marker_billboards(
    sprites: &LocalSprites,
    now: i32,
    sim: &Simulation,
    visual: impl Fn(i16) -> Option<MarkerSprite>,
    instance: impl Fn(&MarkerSprite, [f32; 3]) -> Option<BillboardInstance>,
) -> Vec<BillboardInstance> {
    sprites
        .drawn(now)
        .iter()
        .filter_map(|s| {
            let v = visual(s.sequence)?;
            instance(&v, render_position(sim, s.point))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STIN: FourCC = mandrake_tag::interface::STANDARD_INTERFACE_COLLECTION;

    #[test]
    fn markers_live_eight_frames_from_the_stamp() {
        let mut l = LocalSprites::new();
        for i in 0..4 {
            l.new_local_sprite([i, 0, 0], STIN, SEQUENCE_DESTINATION);
        }
        l.remove_local_sprites(RemoveMode::StartTimer, 59);
        l.remove_local_sprites(RemoveMode::ClearUnlessTimed, 59);
        assert_eq!(l.entries().len(), 4);
        let drawn: Vec<i32> = (59..75).filter(|&t| !l.drawn(t).is_empty()).collect();
        assert_eq!(drawn, (59..=66).collect::<Vec<_>>());
        assert_eq!(l.entries().len(), 4, "the entries stay");
        l.remove_local_sprites(RemoveMode::ClearUnlessTimed, 80);
        assert_eq!(l.entries().len(), 4);
        l.new_local_sprite([9, 9, 9], STIN, SEQUENCE_IMPASSABLE);
        assert_eq!(l.entries().len(), 1);
        assert_eq!(l.timer(), None);
        assert_eq!(l.drawn(1000).len(), 1, "no timer: drawn until removed");
    }

    #[test]
    fn preview_persists_and_the_order_appends_to_it() {
        let mut l = LocalSprites::new();
        l.remove_local_sprites(RemoveMode::ClearUnlessTimed, 56);
        for i in 0..4 {
            l.new_local_sprite([i, 0, 0], STIN, SEQUENCE_WAYPOINT);
        }
        assert_eq!(l.drawn(10_000).len(), 4, "the preview stays");
        l.remove_local_sprites(RemoveMode::ClearUnlessTimed, 60);
        assert!(l.entries().is_empty());
        for i in 0..8 {
            l.new_local_sprite([i, 1, 0], STIN, SEQUENCE_WAYPOINT);
        }
        for i in 0..12 {
            let seq = if i % 3 == 2 { 0 } else { 1 };
            l.new_local_sprite([i, 2, 0], STIN, seq);
        }
        l.remove_local_sprites(RemoveMode::StartTimer, 67);
        assert_eq!(l.entries().len(), 20);
        assert_eq!(l.drawn(74).len(), 20);
        assert!(l.drawn(75).is_empty());
        let mut l = LocalSprites::new();
        for i in 0..200 {
            l.new_local_sprite([i, 0, 0], STIN, 0);
        }
        assert_eq!(l.entries().len(), MAX_LOCAL_SPRITES);
        assert_eq!(l.entries()[125].point[0], 125);
    }

    #[test]
    fn tfl_s_list_holds_30() {
        let mut l = LocalSprites::with_capacity(TFL_MAX_LOCAL_SPRITES);
        for i in 0..40 {
            l.new_local_sprite([i, 0, 0], STIN, SEQUENCE_DESTINATION);
        }
        assert_eq!(l.entries().len(), 30);
        assert_eq!(l.entries()[29].point[0], 29);
        l.remove_local_sprites(RemoveMode::StartTimer, 5);
        l.new_local_sprite([99, 0, 0], STIN, SEQUENCE_WAYPOINT);
        assert_eq!(l.entries().len(), 1);
        assert_eq!(
            LocalSprites::new(),
            LocalSprites::with_capacity(MAX_LOCAL_SPRITES)
        );
    }

    #[test]
    fn the_marker_sequence_follows_the_mask_then_the_point() {
        let warrior = 0xf83c;
        let dwarf = 0xf83e;
        assert_eq!(marker_sequence(warrior, Some(6), 0, 1, true), 0);
        assert_eq!(marker_sequence(warrior, Some(3), 0, 1, true), 4);
        assert_eq!(marker_sequence(warrior, Some(5), 0, 1, true), 4);
        assert_eq!(marker_sequence(warrior, Some(1), 0, 1, true), 0);
        assert_eq!(marker_sequence(dwarf, Some(1), 0, 1, true), 4);
        assert_eq!(marker_sequence(warrior, Some(6), 0, 3, true), 1);
        assert_eq!(marker_sequence(warrior, Some(6), 1, 3, true), 1);
        assert_eq!(marker_sequence(warrior, Some(6), 2, 3, true), 0);
        assert_eq!(marker_sequence(warrior, Some(3), 0, 3, true), 4);
        assert_eq!(marker_sequence(warrior, Some(6), 0, 1, false), 1);
        assert_eq!(marker_sequence(warrior, Some(3), 0, 1, false), 4);
        assert_eq!(marker_sequence(warrior, None, 0, 1, true), 0);
    }

    #[test]
    fn the_attack_marker_is_sequence_2_for_eight_frames() {
        let stin = mandrake_tag::interface::STANDARD_INTERFACE_COLLECTION;
        let mut l = LocalSprites::new();
        l.new_local_sprite([1, 2, 3], stin, SEQUENCE_DESTINATION);
        l.remove_local_sprites(RemoveMode::StartTimer, 50);
        attack_ground_marker(&mut l, [150, 660, 7], 59);
        let want = LocalSprite {
            collection: stin,
            sequence: 2,
            point: [150, 660, 7],
        };
        assert_eq!(l.entries(), &[want]);
        assert_eq!(l.timer(), Some(59));
        l.remove_local_sprites(RemoveMode::ClearUnlessTimed, 59);
        for now in 59..=66 {
            assert_eq!(l.drawn(now), &[want], "local time {now}");
        }
        assert!(l.drawn(67).is_empty());
    }
}
