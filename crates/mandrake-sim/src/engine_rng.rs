use mandrake_common::game_options;
use std::collections::{BTreeMap, HashMap};

use mandrake_common::FourCC;
use mandrake_math::BungieRng;
use mandrake_tag::{
    CollectionReference, MarkerEntry, MeshMapActions, MeshTag, SequenceHeader, get_sequence,
    sequence_headers,
};

use crate::draw_site;
use crate::engine_move::{EngineMover, ModeHooks, ModeState, MoveMode};
use crate::map_actions::{AmbientData, MapActionContext, MapActions, MovementOrder};
use crate::passability::MeshCells;

mod hooks;
mod map_load;

pub(crate) use hooks::*;
pub use map_load::*;

pub const GAMEPLAY_VERSION_1_8_3: i32 = crate::gameplay::GAMEPLAY_1_8_3;

pub const MONSTER_CAPACITY: usize = 400;

thread_local! {
    pub static DRAW_LOG: std::cell::RefCell<Option<Vec<(u32, u32)>>> =
        const { std::cell::RefCell::new(None) };
}

fn walking_state(mode: u8, flags_8: u32) -> ModeState {
    match mode {
        5 if flags_8 & 0x400 != 0 => ModeState::Value(0x13),
        4 => ModeState::Keep,
        _ => ModeState::Zero,
    }
}

pub(crate) fn log_draw(site: u32, state: u32) {
    DRAW_LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push((site, state));
        }
    });
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct SiteCounts {
    counts: BTreeMap<u32, u64>,
    total: u64,
}

impl std::fmt::Debug for SiteCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SiteCounts").field(&self.counts).finish()
    }
}

impl FromIterator<(u32, u64)> for SiteCounts {
    fn from_iter<I: IntoIterator<Item = (u32, u64)>>(iter: I) -> Self {
        let mut s = Self::default();
        for (site, n) in iter {
            s.add(site, n);
        }
        s
    }
}

impl SiteCounts {
    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn add(&mut self, site: u32, n: u64) {
        *self.counts.entry(site).or_insert(0) += n;
        self.total += n;
    }

    pub fn get(&self, site: u32) -> u64 {
        self.counts.get(&site).copied().unwrap_or(0)
    }

    pub fn counts(&self) -> &BTreeMap<u32, u64> {
        &self.counts
    }

    pub fn at(&self, sites: &[u32]) -> u64 {
        sites.iter().map(|&s| self.get(s)).sum()
    }
}

pub struct EngineRng<'a> {
    pub rng: &'a mut BungieRng,
    pub sites: &'a mut SiteCounts,
}

impl EngineRng<'_> {
    pub fn draw(&mut self, site: u32) -> u16 {
        self.sites.add(site, 1);
        log_draw(site, self.rng.state);
        self.rng.next_u16()
    }

    pub fn draw_mod(&mut self, site: u32, n: i32) -> Result<i32, Unmodelled> {
        if n == 0 {
            return Err(Unmodelled::new(site, "division by zero in the engine"));
        }
        Ok(i32::from(self.draw(site)) % n)
    }
}

pub type Unmodelled = mandrake_common::Stop;

pub trait TagSource {
    fn tag(&self, group: [u8; 4], id: FourCC) -> Option<Vec<u8>>;
}

impl<F: Fn([u8; 4], FourCC) -> Option<Vec<u8>>> TagSource for F {
    fn tag(&self, group: [u8; 4], id: FourCC) -> Option<Vec<u8>> {
        self(group, id)
    }
}

pub(crate) fn be_i16(d: &[u8], o: usize) -> i16 {
    i16::from_be_bytes([d[o], d[o + 1]])
}
fn be_u16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}
pub(crate) fn be_i32(d: &[u8], o: usize) -> i32 {
    i32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn fourcc(d: &[u8], o: usize) -> FourCC {
    FourCC::new([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
pub(crate) fn tag_ref(d: &[u8], o: usize) -> Option<FourCC> {
    let v = be_i32(d, o);
    (v != 0 && v != -1).then(|| fourcc(d, o))
}

pub(crate) fn resolved_mons_group(
    mons: &[u8],
    catalog: &crate::projectile_groups::ProjectileCatalog,
) -> Option<FourCC> {
    tag_ref(mons, 0x204).filter(|g| catalog.groups.contains_key(g))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineObject {
    pub index: i32,
    pub identifier: i16,
    pub collection: FourCC,
    pub sequence: u8,
    pub no_shape: bool,
    pub frame: i8,
    pub tick: i8,
    pub ticks_per_frame: i8,
    pub view: u8,
    pub sub_kind: u8,
    pub sub_count: u8,
    pub sub_limit: u8,
    pub anim_flags: u8,
    pub anim_sounds: Option<AnimSounds>,
    pub scale: i32,
    pub vitality: i32,
    pub max_vitality: i32,
    pub permutation: i16,
    pub shading_slot: u8,
    pub core: Option<FourCC>,
    pub velocity: [i16; 3],
    pub flags: u32,
    pub kind: u8,
    pub owner: i16,
    pub z: i32,
    pub charges: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimSounds {
    pub sequence: u8,
    pub first: bool,
    pub key: bool,
    pub last: bool,
}

impl EngineObject {
    pub fn new(index: i32, collection: FourCC) -> Self {
        Self {
            index,
            identifier: 0,
            collection,
            sequence: 0xff,
            no_shape: false,
            frame: 0,
            tick: 0,
            ticks_per_frame: 0,
            view: 0,
            sub_kind: 0,
            sub_count: 0,
            sub_limit: 0,
            anim_flags: 0,
            anim_sounds: None,
            scale: 0,
            vitality: 0,
            max_vitality: 0,
            permutation: 0,
            shading_slot: 0,
            core: None,
            velocity: [0; 3],
            flags: 0,
            kind: 0,
            owner: -1,
            z: 0,
            charges: -1,
        }
    }

    pub fn bounds(&self, headers: &[SequenceHeader]) -> Option<(i16, i16, i16)> {
        if self.sequence == 0xff {
            return None;
        }
        self.bounds_of_header(get_sequence(headers, i32::from(self.sequence)).copied())
    }

    pub(crate) fn bounds_of_header(
        &self,
        header: Option<SequenceHeader>,
    ) -> Option<(i16, i16, i16)> {
        if self.sequence == 0xff {
            return None;
        }
        let b = header?.world_bounds;
        let f = |v: i16| ((i32::from(v) * self.scale) >> 8) as i16;
        Some((f(b.half_width), f(b.bottom), f(b.top)))
    }
}

pub fn set_object_shape(
    obj: &mut EngineObject,
    headers: &[SequenceHeader],
    sequence: i32,
    speed: i32,
    flags: u32,
    rng: &mut EngineRng,
) -> Result<(), Unmodelled> {
    if sequence == -1 {
        obj.no_shape = true;
        return Ok(());
    }
    let header = get_sequence(headers, sequence).copied();
    set_object_shape_header(obj, header, sequence, speed, flags, rng)
}

pub(crate) fn set_object_shape_header(
    obj: &mut EngineObject,
    header: Option<SequenceHeader>,
    sequence: i32,
    speed: i32,
    flags: u32,
    rng: &mut EngineRng,
) -> Result<(), Unmodelled> {
    if sequence == -1 {
        obj.no_shape = true;
        return Ok(());
    }
    let h = header.ok_or_else(|| {
        Unmodelled::new(
            draw_site::OBJECT_SHAPE_NO_SEQUENCES,
            "collection without sequences",
        )
    })?;
    if sequence != i32::from(obj.sequence) || flags & 0x20 != 0 {
        obj.sequence = sequence as u8;
        obj.view = if flags & 8 != 0 {
            rng.draw_mod(draw_site::OBJECT_SHAPE_VIEW, i32::from(h.number_of_views))? as u8
        } else {
            0
        };
        let tpf = ((i32::from(h.ticks_per_frame) * speed) >> 8) as i8;
        obj.ticks_per_frame = if tpf <= 0 { 1 } else { tpf };
        obj.frame = if flags & 0x10 != 0 {
            h.effective_loop_frame() as u8 as i8
        } else if flags & 1 != 0 {
            rng.draw_mod(draw_site::OBJECT_SHAPE_FRAME, i32::from(h.frames_per_view))? as i8
        } else {
            0
        };
        obj.tick = if flags & 2 != 0 && h.ticks_per_frame != 0 {
            rng.draw_mod(draw_site::OBJECT_SHAPE_TICK, i32::from(h.ticks_per_frame))? as i8
        } else {
            0
        };
        obj.anim_flags = 0;
    }
    let kind = h.field_24 as u8;
    let limit = ((speed * i32::from(h.field_26)) >> 8).max(1);
    if obj.sub_kind.wrapping_sub(5) > 2 {
        obj.sub_kind = kind;
        obj.sub_limit = limit as u8;
        obj.sub_count = if flags & 4 != 0 {
            rng.draw_mod(draw_site::OBJECT_SHAPE_LIMIT, limit)? as u8
        } else {
            0
        };
    }
    obj.no_shape = false;
    Ok(())
}

pub fn equalize_sequence_collision_rectangles(headers: &mut [SequenceHeader], from: i16, to: i16) {
    if from == -1 || to == -1 || headers.is_empty() {
        return;
    }
    let at = |i: i16| {
        let i = i as u16 as usize;
        if i < headers.len() { i } else { 0 }
    };
    let b = headers[at(from)].world_bounds;
    headers[at(to)].world_bounds = b;
}

pub fn equalize_monster_sequences(
    mons: &[u8],
    headers: &mut [SequenceHeader],
) -> Result<(), Unmodelled> {
    if mons.len() < 0x224 {
        return Err(Unmodelled::new(
            draw_site::EQUALIZE_MONS_SHORT,
            "mons too short",
        ));
    }
    let bit2 = mons[0x198] & 2 != 0;
    let count = headers.len() as i32;
    let verify = |s: i16, site: u32| {
        if s == -1 || (s >= 0 && i32::from(s) < count) {
            Ok(())
        } else {
            Err(Unmodelled::new(
                site,
                format!("sequence {s} of {count} (verify_sequence_index fails the postprocess)"),
            ))
        }
    };
    let idle = be_i16(mons, 0x08);
    let attacks = be_i16(mons, 0x88);
    let attack_sequence = |k: i32, j: usize| be_i16(mons, 0x8c + 0x40 * k as usize + 0x0e + 8 * j);
    for k in 0..i32::from(attacks.max(0)) {
        for j in 0..4 {
            let seq = attack_sequence(k, j);
            verify(seq, draw_site::EQUALIZE_ATTACK_SEQUENCE)?;
            if !bit2 {
                equalize_sequence_collision_rectangles(headers, idle, seq);
            }
        }
    }
    for i in 0..=18i32 {
        let seq = be_i16(mons, 0x08 + 2 * i as usize);
        verify(seq, draw_site::EQUALIZE_SEQUENCE)?;
        if i == 0 || seq == -1 {
            continue;
        }
        if bit2 {
            if i == 0x10 {
                continue;
            }
            let n = i32::from(attacks);
            if n > 0 && (0..n).any(|k| (0..4).any(|j| i32::from(attack_sequence(k, j)) == i)) {
                continue;
            }
        }
        equalize_sequence_collision_rectangles(headers, idle, seq);
    }
    Ok(())
}

pub fn equalize_collection_for_palette(
    mesh: &MeshTag,
    tags: &dyn TagSource,
    collection: FourCC,
    headers: &mut [SequenceHeader],
) -> Result<Option<Unmodelled>, Unmodelled> {
    let mut defs: Vec<(FourCC, Vec<u8>)> = Vec::new();
    for p in mesh.palettes.iter().filter(|p| p.marker_type == 3) {
        let Some(unit) = tags.tag(*b"unit", p.tag_id).filter(|u| u.len() >= 8) else {
            continue;
        };
        let mons = fourcc(&unit, 0);
        if defs.iter().any(|(m, _)| *m == mons) {
            continue;
        }
        let Some(data) = tags.tag(*b"mons", mons).filter(|d| d.len() >= 0x224) else {
            continue;
        };
        if fourcc(&data, 4) == collection {
            defs.push((mons, data));
        }
    }
    let apply = |order: &[usize], h: &mut [SequenceHeader]| -> Result<(), Unmodelled> {
        for &k in order {
            let (mons, data) = &defs[k];
            equalize_monster_sequences(data, h)
                .map_err(|e| Unmodelled::new(e.site, format!("mons {mons}: {}", e.reason)))?;
        }
        Ok(())
    };
    let palette_order: Vec<usize> = (0..defs.len()).collect();
    let mut result = headers.to_vec();
    apply(&palette_order, &mut result)?;
    let mut dependent = defs.len() > 7;
    if defs.len() > 1 && !dependent {
        let mut order = palette_order.clone();
        let mut c = vec![0usize; order.len()];
        let mut i = 0;
        while i < order.len() && !dependent {
            if c[i] < i {
                order.swap(if i % 2 == 0 { 0 } else { c[i] }, i);
                let mut h = headers.to_vec();
                apply(&order, &mut h)?;
                dependent = h
                    .iter()
                    .zip(&result)
                    .any(|(a, b)| a.world_bounds != b.world_bounds);
                c[i] += 1;
                i = 0;
            } else {
                c[i] = 0;
                i += 1;
            }
        }
    }
    headers.copy_from_slice(&result);
    Ok(dependent.then(|| {
        let names: Vec<String> = defs.iter().map(|(m, _)| m.to_string()).collect();
        Unmodelled::new(
            draw_site::EQUALIZE_COLLECTION_RECTANGLES,
            format!(
                "collection {collection}: the mons postprocess of {} gives different collision rectangles in different orders; their palette order is assumed",
                names.join(", ")
            ),
        )
    }))
}

pub fn animate_object(obj: &mut EngineObject, headers: &[SequenceHeader]) {
    if obj.no_shape {
        return;
    }
    let Some(h) = get_sequence(headers, i32::from(obj.sequence)) else {
        return;
    };
    let r9 = i32::from(obj.ticks_per_frame);
    let mut frame = i32::from(obj.frame);
    let mut tick = i32::from(obj.tick);
    let mut flags = 0u8;
    let first = r9 > 0 && frame == 0 && tick == 0;
    if r9 > 0 {
        if tick >= r9 {
            tick = 1;
        } else {
            tick += 1;
        }
        if r9 <= tick {
            if obj.flags & 0x8_0000 != 0 {
                frame -= 1;
                if frame >= 0 {
                    flags = if i32::from(h.key_frame) == frame {
                        3
                    } else {
                        1
                    };
                } else {
                    frame = i32::from(h.frames_per_view) - 1;
                    flags = if i32::from(h.key_frame) == frame {
                        7
                    } else {
                        5
                    };
                }
            } else {
                frame += 1;
                if frame < i32::from(h.frames_per_view) {
                    flags = if i32::from(h.key_frame) == frame {
                        3
                    } else {
                        1
                    };
                } else {
                    frame = i32::from(h.effective_loop_frame());
                    flags = if i32::from(h.key_frame) == frame {
                        7
                    } else {
                        5
                    };
                }
            }
        }
    }
    obj.frame = frame as i8;
    obj.tick = tick as i8;
    if obj.sub_kind != 0 {
        obj.sub_count = obj.sub_count.wrapping_add(1);
        if (obj.sub_count as i8) > (obj.sub_limit as i8) {
            match obj.sub_kind {
                6 => obj.sub_count = obj.sub_limit,
                5 => {
                    obj.sub_kind = 7;
                    obj.sub_limit = (((obj.index * 81) & 0x3f) + 0x3c) as u8;
                    flags |= 8;
                    obj.sub_count = 0;
                }
                _ => {
                    flags |= 8;
                    obj.sub_count = 0;
                }
            }
        }
    }
    obj.anim_flags = flags;
    if first || flags & 6 != 0 {
        obj.anim_sounds = Some(AnimSounds {
            sequence: obj.sequence,
            first,
            key: flags & 2 != 0,
            last: flags & 4 != 0,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterDefinition {
    pub tag: FourCC,
    pub flags: u32,
    pub idle_sequence: i16,
    pub idle_sequence_1c: i16,
    pub idle_sequence_1e: i16,
    pub idle_sequence_20: i16,
    pub walk_sequence: i16,
    pub blocked_sequence: i16,
    pub turn_sequence: i16,
    pub sequence_22: i16,
    pub sequence_2a: i16,
    pub field_1b6: i16,
    pub field_199: u8,
    pub spatial_group_strength: i16,
    pub spatial_group_range: i16,
    pub object_vitality: i16,
    pub held_vitality_multiplier: i16,
    pub field_74: i16,
    pub field_76: i16,
    pub speed: i16,
    pub creation_draw_threshold: u16,
    pub object: FourCC,
    pub map_action: Option<FourCC>,
    pub inventory: Option<FourCC>,
    pub pathfinding_radius: i16,
    pub field_82: i16,
    pub field_1b5: u8,
    pub object_flags: u32,
    pub object_modifiers: [i16; 9],
    pub object_threshold: i16,
    pub ai: crate::attack_ai::AiDefinition,
    pub name_counter: i16,
    pub name_count: i16,
}

impl MonsterDefinition {
    pub fn from_bytes(tag: FourCC, m: &[u8]) -> Result<Self, String> {
        if m.len() < 0x224 {
            return Err(format!("mons {tag} too short ({} bytes)", m.len()));
        }
        Ok(Self {
            tag,
            flags: be_i32(m, 0) as u32,
            idle_sequence: be_i16(m, 0x08),
            idle_sequence_1c: be_i16(m, 0x1c),
            idle_sequence_1e: be_i16(m, 0x1e),
            idle_sequence_20: be_i16(m, 0x20),
            walk_sequence: be_i16(m, 0x0a),
            blocked_sequence: be_i16(m, 0x0e),
            turn_sequence: be_i16(m, 0x10),
            sequence_22: be_i16(m, 0x22),
            sequence_2a: be_i16(m, 0x2a),
            field_1b6: be_i16(m, 0x1b6),
            field_199: m[0x199],
            spatial_group_strength: be_i16(m, 0x1f2),
            spatial_group_range: be_i16(m, 0x1f4),
            object_vitality: 0,
            held_vitality_multiplier: be_i16(m, 0x72),
            field_74: be_i16(m, 0x74),
            field_76: be_i16(m, 0x76),
            speed: be_i16(m, 0x7e),
            creation_draw_threshold: be_u16(m, 0x80),
            object: fourcc(m, 0x84),
            map_action: tag_ref(m, 0x18c),
            inventory: tag_ref(m, 0x220),
            pathfinding_radius: be_i16(m, 0x5a),
            field_82: be_i16(m, 0x82),
            field_1b5: m[0x1b5],
            object_flags: 0,
            object_modifiers: [0; 9],
            object_threshold: 0,
            ai: crate::attack_ai::AiDefinition::from_tags(tag, m, Vec::new())?,
            name_counter: -1,
            name_count: 0,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineMonster {
    pub index: usize,
    pub identifier: i16,
    pub first_draw: u64,
    pub definition: usize,
    pub object: EngineObject,
    pub mode: u8,
    pub anim_state: u8,
    pub speed: i16,
    pub position: (i32, i32),
    pub flags: u32,
    pub flags_8: u32,
    pub neighbours: u8,
    pub impassable: u16,
    pub ambient: Option<AmbientData>,
    pub unmodelled: Option<String>,
    pub mirrored: bool,
    pub team: i16,
    pub owner: i16,
    pub facing: u16,
    pub formation: MonsterFormation,
    pub cell_link: u64,
    pub flying_height: i16,
}

pub fn unit_key(identifier: i16) -> Option<u32> {
    (identifier != 0).then(|| u32::from(identifier as u16))
}

impl EngineMonster {
    pub fn unit_key(&self) -> Option<u32> {
        unit_key(self.identifier)
    }

    pub fn has_velocity(&self) -> bool {
        self.object.velocity != [0; 3]
    }

    pub fn record_flags(&mut self) -> crate::engine_move::RecordFlags<'_> {
        crate::engine_move::RecordFlags {
            flags: &mut self.flags,
            flags_8: &mut self.flags_8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MonsterFormation {
    pub identifier: i16,
    pub slot: i16,
    pub formation: i16,
    pub facing: u16,
    pub preferred: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineScenery {
    pub marker: usize,
    pub identifier: i16,
    pub first_draw: u64,
    pub object: EngineObject,
    pub position: (i32, i32),
    pub facing: u16,
    pub scenery_flags: u32,
    pub projectile: Option<FourCC>,
    pub cell_link: u64,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineProjectile {
    pub marker: usize,
    pub identifier: i16,
    pub definition: FourCC,
    pub first_draw: u64,
    pub object: EngineObject,
    pub position: (i32, i32),
    pub charge_time: i16,
    pub facing: u16,
    pub elevation: u16,
    pub cell_link: u64,
}

pub fn celebration_runs(r: u16) -> u8 {
    (30 + ((u32::from(r) * 5) >> 14)) as u8
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FadeRecord {
    pub index: i32,
    pub host: usize,
    pub object: EngineObject,
    pub position: (i32, i32),
    pub facing: u16,
    pub cell_link: u64,
    pub countdown: u16,
    pub group: Option<FourCC>,
    pub monster: Option<usize>,
}

impl FadeRecord {
    pub const NO_HOST: usize = usize::MAX;
}

impl EngineWorld {
    pub fn collection_core(&self, collection: FourCC) -> Option<FourCC> {
        self.collection_table0_cores.get(&collection).copied()
    }

    pub(crate) fn solid_copies(&self) -> impl Iterator<Item = &FadeRecord> {
        self.fade_records.iter().filter(|r| {
            r.host == FadeRecord::NO_HOST && !r.object.no_shape && r.object.flags & 0x400 != 0
        })
    }

    pub(crate) fn solid_projectile_objects(
        &self,
    ) -> impl Iterator<Item = (u64, crate::pathfinding::ObjectView)> + '_ {
        (self.solid_views(
            self.projectile_records.iter().map(|p| &**p),
            &self.dormant_objects,
        ))
        .chain(self.solid_elsewhere.iter().copied())
    }

    pub(crate) fn solid_views<'a>(
        &'a self,
        records: impl Iterator<Item = &'a crate::projectiles::FlightProjectile> + 'a,
        dormant: &'a [crate::flight::DormantObject],
    ) -> impl Iterator<Item = (u64, crate::pathfinding::ObjectView)> + 'a {
        let view = |o: &EngineObject, position: [i32; 3], facing: u16, owner: i16, kind: u8| {
            let (radius, low, high) = o.bounds(self.headers(o.collection)).unwrap_or((0, 0, 0));
            crate::pathfinding::ObjectView {
                index: o.index as i16,
                identifier: o.identifier as u16,
                flags: o.flags,
                position: (position[0], position[1]),
                facing,
                radius,
                bounds_low: low,
                bounds_high: high,
                owner,
                kind,
                z: position[2],
            }
        };
        let solid = |o: &EngineObject| !o.no_shape && o.flags & 0x600 == 0x400;
        let records = records.filter_map(move |p| {
            let o = p.shape.as_ref().filter(|o| solid(o))?;
            let at = [p.object.x, p.object.y, p.object.z];
            Some((p.cell_link, view(o, at, p.facing, p.index as i16, 3)))
        });
        let resting = (dormant.iter())
            .filter(move |d| solid(&d.object))
            .map(move |d| {
                let o = &d.object;
                (d.cell_link, view(o, d.position, d.facing, o.owner, o.kind))
            });
        records.chain(resting)
    }
}

#[derive(Default)]
pub struct SceneryIndex {
    fast: std::sync::OnceLock<std::sync::Arc<SceneryCells>>,
    fallback: std::sync::Mutex<Option<std::sync::Arc<SceneryCells>>>,
}

impl Clone for SceneryIndex {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for SceneryIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SceneryIndex")
    }
}

pub(crate) enum SceneryCellsRef<'a> {
    Borrowed(&'a SceneryCells),
    Owned(std::sync::Arc<SceneryCells>),
}

impl std::ops::Deref for SceneryCellsRef<'_> {
    type Target = SceneryCells;
    fn deref(&self) -> &SceneryCells {
        match self {
            Self::Borrowed(c) => c,
            Self::Owned(c) => c,
        }
    }
}

impl SceneryIndex {
    pub(crate) fn of(&self, scenery: &[EngineScenery]) -> SceneryCellsRef<'_> {
        let fast = self
            .fast
            .get_or_init(|| std::sync::Arc::new(SceneryCells::new(scenery)));
        if fast.len == scenery.len() {
            #[cfg(feature = "identity-checks")]
            assert!(
                fast.made_from
                    .iter()
                    .copied()
                    .eq(scenery.iter().map(SceneryCells::key))
            );
            return SceneryCellsRef::Borrowed(fast);
        }
        SceneryCellsRef::Owned(self.arc_fallback(scenery))
    }

    pub(crate) fn arc(&self, scenery: &[EngineScenery]) -> std::sync::Arc<SceneryCells> {
        let fast = self
            .fast
            .get_or_init(|| std::sync::Arc::new(SceneryCells::new(scenery)));
        if fast.len == scenery.len() {
            #[cfg(feature = "identity-checks")]
            assert!(
                fast.made_from
                    .iter()
                    .copied()
                    .eq(scenery.iter().map(SceneryCells::key))
            );
            return fast.clone();
        }
        self.arc_fallback(scenery)
    }

    fn arc_fallback(&self, scenery: &[EngineScenery]) -> std::sync::Arc<SceneryCells> {
        let mut guard = self.fallback.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(c) if c.len == scenery.len() => {
                #[cfg(feature = "identity-checks")]
                assert!(
                    c.made_from
                        .iter()
                        .copied()
                        .eq(scenery.iter().map(SceneryCells::key))
                );
                c.clone()
            }
            _ => {
                let c = std::sync::Arc::new(SceneryCells::new(scenery));
                *guard = Some(c.clone());
                c
            }
        }
    }
}

const MONSTER_INDEX_EMPTY: u16 = u16::MAX;
const MONSTER_INDEX_NONE_CELL: u32 = u32::MAX;
const MONSTER_INDEX_NONE_IMP: u32 = u32::MAX;

#[derive(Default)]
pub(crate) struct MonsterIndex {
    pub(crate) active: bool,
    width: i32,
    height: i32,
    cell_head: Vec<u16>,
    next: Vec<u16>,
    prev: Vec<u16>,
    cell_of: Vec<u32>,
    overflow: Vec<u16>,
    by_object: Vec<u16>,
    def_impassable: Vec<u32>,
    sort_scratch: Vec<(u64, std::cmp::Reverse<usize>)>,
}

impl Clone for MonsterIndex {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for MonsterIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MonsterIndex")
    }
}

impl MonsterIndex {
    #[inline]
    fn cell_idx(&self, cx: i32, cy: i32) -> Option<u32> {
        if cx >= 0 && cy >= 0 && cx < self.width && cy < self.height {
            Some((cy * self.width + cx) as u32)
        } else {
            None
        }
    }

    fn push_head(&mut self, cell: u32, m: u16) {
        let head = self.cell_head[cell as usize];
        self.prev[m as usize] = MONSTER_INDEX_EMPTY;
        self.next[m as usize] = head;
        if head != MONSTER_INDEX_EMPTY {
            self.prev[head as usize] = m;
        }
        self.cell_head[cell as usize] = m;
        self.cell_of[m as usize] = cell;
    }

    fn unlink(&mut self, i: usize) {
        if i >= self.cell_of.len() {
            return;
        }
        let cell = self.cell_of[i];
        if cell != MONSTER_INDEX_NONE_CELL {
            let p = self.prev[i];
            let n = self.next[i];
            if p != MONSTER_INDEX_EMPTY {
                self.next[p as usize] = n;
            } else {
                self.cell_head[cell as usize] = n;
            }
            if n != MONSTER_INDEX_EMPTY {
                self.prev[n as usize] = p;
            }
            self.prev[i] = MONSTER_INDEX_EMPTY;
            self.next[i] = MONSTER_INDEX_EMPTY;
            self.cell_of[i] = MONSTER_INDEX_NONE_CELL;
        } else if !self.overflow.is_empty() {
            let m = i as u16;
            if let Some(pos) = self.overflow.iter().position(|&x| x == m) {
                self.overflow.remove(pos);
            }
        }
    }

    fn move_monster(&mut self, i: usize, cx: i32, cy: i32) {
        if !self.active || i >= self.cell_of.len() {
            return;
        }
        self.unlink(i);
        if let Some(cell) = self.cell_idx(cx, cy) {
            self.push_head(cell, i as u16);
        } else {
            self.overflow.insert(0, i as u16);
        }
    }

    fn build(&mut self, width: i32, height: i32, monsters: &[EngineMonster], def_count: usize) {
        if monsters.len() >= MONSTER_INDEX_EMPTY as usize || width <= 0 || height <= 0 {
            self.active = false;
            return;
        }
        let grid_len = (width as usize).saturating_mul(height as usize);
        if self.cell_head.len() != grid_len {
            self.cell_head.clear();
            self.cell_head.resize(grid_len, MONSTER_INDEX_EMPTY);
        } else {
            for &c in &self.cell_of {
                if c != MONSTER_INDEX_NONE_CELL && (c as usize) < self.cell_head.len() {
                    self.cell_head[c as usize] = MONSTER_INDEX_EMPTY;
                }
            }
        }
        self.width = width;
        self.height = height;
        self.overflow.clear();
        self.next.clear();
        self.next.resize(monsters.len(), MONSTER_INDEX_EMPTY);
        self.prev.clear();
        self.prev.resize(monsters.len(), MONSTER_INDEX_EMPTY);
        self.cell_of.clear();
        self.cell_of.resize(monsters.len(), MONSTER_INDEX_NONE_CELL);

        self.sort_scratch.clear();
        let mut max_obj = 0usize;
        for (i, m) in monsters.iter().enumerate() {
            if m.identifier != 0 {
                self.sort_scratch.push((m.cell_link, std::cmp::Reverse(i)));
            }
            if let Ok(o) = usize::try_from(m.object.index)
                && o > max_obj
            {
                max_obj = o;
            }
        }
        self.sort_scratch.sort_unstable();
        for i in 0..self.sort_scratch.len() {
            let m = self.sort_scratch[i].1.0;
            let p = monsters[m].position;
            let (cx, cy) = (p.0 >> 9, p.1 >> 9);
            if let Some(cell) = self.cell_idx(cx, cy) {
                self.push_head(cell, m as u16);
            } else {
                self.overflow.insert(0, m as u16);
            }
        }

        let needed_obj = max_obj + 1;
        if self.by_object.len() < needed_obj {
            self.by_object.resize(needed_obj, MONSTER_INDEX_EMPTY);
        }
        self.by_object.fill(MONSTER_INDEX_EMPTY);
        for (i, m) in monsters.iter().enumerate() {
            if let Ok(o) = usize::try_from(m.object.index)
                && self.by_object[o] == MONSTER_INDEX_EMPTY
            {
                self.by_object[o] = i as u16;
            }
        }

        self.refresh_def_impassable(monsters, def_count);
        self.active = true;
    }

    fn refresh_def_impassable(&mut self, monsters: &[EngineMonster], def_count: usize) {
        self.def_impassable.clear();
        self.def_impassable
            .resize(def_count, MONSTER_INDEX_NONE_IMP);
        for m in monsters {
            if let Some(slot) = self.def_impassable.get_mut(m.definition)
                && *slot == MONSTER_INDEX_NONE_IMP
            {
                *slot = u32::from(m.impassable);
            }
        }
    }

    #[inline]
    fn cell_iter<'a>(
        &'a self,
        cx: i32,
        cy: i32,
        monsters: &'a [EngineMonster],
    ) -> MonsterCellIter<'a> {
        if let Some(cell) = self.cell_idx(cx, cy) {
            MonsterCellIter::Grid {
                next: &self.next,
                cur: self.cell_head[cell as usize],
            }
        } else {
            MonsterCellIter::Overflow {
                overflow: &self.overflow,
                monsters,
                cx,
                cy,
                pos: 0,
            }
        }
    }

    #[inline]
    fn monster_by_object(&self, index: i32) -> Option<usize> {
        let o = usize::try_from(index).ok()?;
        let &slot = self.by_object.get(o)?;
        (slot != MONSTER_INDEX_EMPTY).then_some(slot as usize)
    }

    #[inline]
    fn impassable_of(&self, d: usize) -> u16 {
        self.def_impassable
            .get(d)
            .copied()
            .filter(|&v| v != MONSTER_INDEX_NONE_IMP)
            .map_or(0, |v| v as u16)
    }
}

enum MonsterCellIter<'a> {
    Grid {
        next: &'a [u16],
        cur: u16,
    },
    Overflow {
        overflow: &'a [u16],
        monsters: &'a [EngineMonster],
        cx: i32,
        cy: i32,
        pos: usize,
    },
}

impl Iterator for MonsterCellIter<'_> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        match self {
            Self::Grid { next, cur } => {
                if *cur == MONSTER_INDEX_EMPTY {
                    return None;
                }
                let m = *cur as usize;
                *cur = next[m];
                Some(m)
            }
            Self::Overflow {
                overflow,
                monsters,
                cx,
                cy,
                pos,
            } => {
                while *pos < overflow.len() {
                    let m = overflow[*pos] as usize;
                    *pos += 1;
                    let p = monsters[m].position;
                    if (p.0 >> 9, p.1 >> 9) == (*cx, *cy) {
                        return Some(m);
                    }
                }
                None
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SceneryCells {
    len: usize,
    x0: i32,
    y0: i32,
    cols: i32,
    rows: i32,
    starts: Vec<u32>,
    items: Vec<u32>,
    by_object: Vec<(i32, u32)>,
    #[cfg(feature = "identity-checks")]
    made_from: Vec<(i32, i32, u64, i32)>,
}

impl SceneryCells {
    #[cfg(feature = "identity-checks")]
    fn key(sc: &EngineScenery) -> (i32, i32, u64, i32) {
        (
            sc.position.0 >> 9,
            sc.position.1 >> 9,
            sc.cell_link,
            sc.object.index,
        )
    }

    fn new(scenery: &[EngineScenery]) -> Self {
        let cell = |sc: &EngineScenery| (sc.position.0 >> 9, sc.position.1 >> 9);
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for sc in scenery {
            let (x, y) = cell(sc);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let (cols, rows) = if scenery.is_empty() {
            (0, 0)
        } else {
            (x1 - x0 + 1, y1 - y0 + 1)
        };
        let mut keyed: Vec<(usize, std::cmp::Reverse<u64>, u32)> = scenery
            .iter()
            .enumerate()
            .map(|(i, sc)| {
                let (x, y) = cell(sc);
                (
                    ((y - y0) * cols + (x - x0)) as usize,
                    std::cmp::Reverse(sc.cell_link),
                    i as u32,
                )
            })
            .collect();
        keyed.sort_unstable();
        let n = (cols as usize) * (rows as usize);
        let mut starts = vec![0u32; n + 1];
        for &(k, _, _) in &keyed {
            starts[k + 1] += 1;
        }
        for k in 0..n {
            starts[k + 1] += starts[k];
        }
        let mut by_object: Vec<(i32, u32)> = scenery
            .iter()
            .enumerate()
            .map(|(i, sc)| (sc.object.index, i as u32))
            .collect();
        by_object.sort_unstable();
        Self {
            len: scenery.len(),
            x0,
            y0,
            cols,
            rows,
            starts,
            items: keyed.into_iter().map(|e| e.2).collect(),
            by_object,
            #[cfg(feature = "identity-checks")]
            made_from: scenery.iter().map(Self::key).collect(),
        }
    }

    pub(crate) fn cell(&self, x: i32, y: i32) -> &[u32] {
        let (col, row) = (x.wrapping_sub(self.x0), y.wrapping_sub(self.y0));
        if !(0..self.cols).contains(&col) || !(0..self.rows).contains(&row) {
            return &[];
        }
        let k = (row * self.cols + col) as usize;
        &self.items[self.starts[k] as usize..self.starts[k + 1] as usize]
    }

    pub(crate) fn of_object(&self, index: i32) -> impl Iterator<Item = usize> + '_ {
        let from = self.by_object.partition_point(|e| e.0 < index);
        self.by_object[from..]
            .iter()
            .take_while(move |e| e.0 == index)
            .map(|e| e.1 as usize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerPlace {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub yaw: i16,
    pub pitch: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterLocation {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub facing: u16,
}

#[derive(Debug, Clone, Default)]
pub struct EngineWorld {
    pub path_log: Option<Vec<PathCall>>,
    pub game_options: u32,
    pub game: crate::game_state::GameState,
    pub alliances: crate::ally::Alliances,
    pub alliance_changes: Vec<crate::ally::AllianceChange>,
    pub statistics: crate::statistics::Statistics,
    pub monster_initializers: crate::veterans::MonsterInitializers,
    pub statistics_stop: Option<Unmodelled>,
    pub setup_options: u32,
    pub flag_data: crate::flags::FlagData,
    pub collections: crate::entities::TagIdMap<Vec<SequenceHeader>>,
    pub key_points: crate::entities::TagIdMap<Vec<Vec<[i16; 3]>>>,
    pub equalized: crate::entities::TagIdMap<(FourCC, i16)>,
    pub collection_table0_cores: crate::entities::TagIdMap<FourCC>,
    pub monster_definitions: Vec<MonsterDefinition>,
    pub monsters: Vec<EngineMonster>,
    pub scenery: Vec<EngineScenery>,
    pub scenery_index: SceneryIndex,
    pub(crate) monster_index: MonsterIndex,
    pub projectiles: Vec<EngineProjectile>,
    pub difficulty: i16,
    pub game_type: i16,
    pub mesh_cells: (i32, i32),
    pub cells: MeshCells,
    pub models: Vec<crate::models::WorldModel>,
    pub model_datums: u16,
    pub model_effects: crate::model_effects::ModelEffects,
    pub geometry_surfaces: crate::entities::TagIdMap<Vec<crate::models::SurfaceMaterial>>,
    pub model_animations: crate::model_animations::ModelAnimations,
    pub cells_changed: bool,
    pub connectors: Vec<crate::connectors::Connector>,
    pub impassability_updates: Vec<usize>,
    pub map_actions: Option<MapActions>,
    pub monster_map_actions: BTreeMap<usize, usize>,
    pub pending_map_actions: Vec<crate::map_actions::PendingMonsterAction>,
    pub pending_orders: Vec<MovementOrder>,
    pub attack_queue: crate::map_attacks::AttackQueue,
    pub pending_attacks: Vec<crate::map_attacks::AttackOrder>,
    pub target_log: Option<Vec<(usize, u32, i16)>>,
    pub fire_timer: Option<i32>,
    pub fire: crate::fire::FireState,
    pub wind: Option<EngineWind>,
    pub particles: Option<crate::particles::ParticleSystem>,
    pub monster_draws: Vec<(usize, u64)>,
    next_object: i32,
    monster_datums: u16,
    object_datums: u16,
    pub cell_links: u64,
    copy_links: u64,
    pub formation_counter: FormationCounter,
    pub formation_definition: crate::formations::FormationDefinition,
    pub player_formations: Vec<(i16, crate::formations::FormationDefinition)>,
    pub formation_globals: crate::formations::FormationGlobals,
    pub spatial_groups: Vec<crate::spatial_groups::SpatialGroup>,
    pub players: crate::players::Players,
    pub unit_rows: BTreeMap<(u32, i16), UnitRow>,
    pub local_player: i16,
    pub mode: i16,
    pub local_selection: Vec<i16>,
    pub replay: Option<ReplayGame>,
    pub mons_postprocess: crate::mons_postprocess::MonsPostprocess,
    pub heights: Vec<(i16, i16)>,
    pub media: mandrake_tag::mesh::MediaCoverage,
    pub mesh_flags: u32,
    pub mesh_ceiling: i16,
    pub media_splash: Option<Option<FourCC>>,
    pub catalog: crate::projectile_groups::ProjectileCatalog,
    pub mesh_effects: Vec<Option<crate::mesh_effects::MeshEffectDatum>>,
    pub projectile_records:
        crate::record_list::RecordList<Box<crate::projectiles::FlightProjectile>>,
    pub dormant_objects: Vec<crate::flight::DormantObject>,
    pub(crate) solid_elsewhere: Vec<(u64, crate::pathfinding::ObjectView)>,
    pub fade_records: crate::record_list::RecordList<FadeRecord>,
    pub lightning: crate::lightning::LightningData,
    pub reserved_records: crate::record_list::RecordList<i32>,
    pub unit_markers: Vec<crate::geometry_filter::UnitMarker>,
    pub sound_sources: Vec<crate::lightning_action::SoundSource>,
    pub launched_groups: Vec<(crate::projectile_groups::GroupDescriptor, i16)>,
    pub scenery_taken: Vec<i32>,
    pub group_events: Vec<crate::CombatEvent>,
    pub free_objects: std::collections::BTreeSet<i32>,
    pub object_history: crate::flight::ObjectHistoryMap,
    pub combat: Vec<crate::attack_ai::MonsterCombat>,
    pub movers: Vec<Option<EngineMover>>,
    pub marker_creation: Option<std::sync::Arc<MarkerCreation>>,
    pub monster_def_index: crate::entities::TagIdMap<usize>,
    pub shown_monsters: Vec<usize>,
    pub marker_unplaced: BTreeMap<i16, bool>,
    pub marker_owners: BTreeMap<i16, i16>,
    pub marker_places: BTreeMap<i16, MarkerPlace>,
    pub renamed_monsters: Vec<(i16, i16)>,
}

#[derive(Debug, Clone)]
pub struct MarkerCreation {
    pub mesh: MeshTag,
    tags: HashMap<([u8; 4], FourCC), Vec<u8>>,
    params: GameParameters,
    game_type: i16,
    game_options: u32,
    mode: i16,
    replay: Option<ReplayGame>,
    unit_palettes: Vec<UnitPalette>,
}

impl TagSource for MarkerCreation {
    fn tag(&self, group: [u8; 4], id: FourCC) -> Option<Vec<u8>> {
        self.tags.get(&(group, id)).cloned()
    }
}

impl MarkerCreation {
    pub fn set_tag(&mut self, group: [u8; 4], id: FourCC, data: Vec<u8>) {
        self.tags.insert((group, id), data);
    }

    fn capture(ctx: &MarkerContext<'_>) -> Self {
        let mut tags = HashMap::new();
        let mut keep = |group: [u8; 4], id: FourCC| -> Option<Vec<u8>> {
            let d = ctx.tags.tag(group, id)?;
            tags.insert((group, id), d.clone());
            Some(d)
        };
        for p in ctx.mesh.palettes.iter().filter(|p| p.marker_type == 3) {
            let Some(unit) = keep(*b"unit", p.tag_id).filter(|u| u.len() >= 8) else {
                continue;
            };
            if let Some(core) = keep(*b"core", fourcc(&unit, 4))
                && let Ok(c) = CollectionReference::from_bytes(&core)
            {
                keep(*b".256", c.collection_tag);
            }
            let Some(mons) = keep(*b"mons", fourcc(&unit, 0)) else {
                continue;
            };
            if mons.len() >= 8 {
                keep(*b".256", fourcc(&mons, 4));
            }
            if mons.len() >= 0x88 {
                keep(*b"obje", fourcc(&mons, 0x84));
            }
            if let Some(n) = tag_ref(&mons, 0x1ac) {
                keep(*b"stli", n);
            }
        }
        Self {
            mesh: ctx.mesh.clone(),
            tags,
            params: ctx.params,
            game_type: ctx.game_type,
            game_options: ctx.game_options,
            mode: ctx.mode,
            replay: ctx.replay,
            unit_palettes: ctx.unit_palettes.clone(),
        }
    }
}

impl EngineWorld {
    pub(crate) fn initialize_monster_placements(&mut self) {
        if self.game_options & mandrake_common::game_options::UNIT_TRADING == 0
            || self.game.state != 0
        {
            return;
        }
        let Some(creation) = self.marker_creation.clone() else {
            return;
        };
        for m in creation.mesh.markers.iter().filter(|m| m.marker_type == 3) {
            if engine_palette(&creation.mesh, m).is_some_and(|p| p.flags as u16 & 8 == 0)
                && let Some(i) = self.monster_index_by_identifier(m.marker_id as u16)
            {
                self.monsters[i].flags |= 0x40_0040;
                self.monsters[i].flags_8 |= 0x80_0000;
            }
        }
    }

    pub(crate) fn new_monster_from_marker(
        &mut self,
        creation: &MarkerCreation,
        marker: &MarkerEntry,
        r: &mut EngineRng,
    ) -> Result<Option<usize>, Unmodelled> {
        self.marker_unplaced
            .insert(marker.marker_id, marker.flags & 1 != 0);
        let mut marker = marker.clone();
        if let Some(p) = self.marker_places.get(&marker.marker_id) {
            (marker.x, marker.y, marker.z, marker.yaw, marker.pitch) =
                (p.x, p.y, p.z, p.yaw, p.pitch);
        }
        let marker = &marker;
        let mut ctx = self.marker_context(creation);
        let result = ctx.new_monster(self, marker, r);
        self.monster_def_index = ctx.monster_def_index;
        result?;
        if let Some(stop) = ctx.stops.into_iter().next() {
            return Err(stop);
        }
        Ok(self.monster_index_by_identifier(marker.marker_id as u16))
    }

    pub(crate) fn new_monster_at_location(
        &mut self,
        unit: FourCC,
        player: i16,
        flags: (u32, u32),
        at: MonsterLocation,
        r: &mut EngineRng,
    ) -> Result<Option<usize>, Unmodelled> {
        let Some(row) = self
            .unit_rows
            .get(&(u32::from_be_bytes(unit.0), player))
            .copied()
        else {
            return Ok(None);
        };
        let Some(creation) = self.marker_creation.clone() else {
            return Err(Unmodelled::new(
                draw_site::NEW_MONSTER_AT_LOCATION_NO_TAGS,
                "new_monster_at_location without the load's tags",
            ));
        };
        let mut ctx = self.marker_context(&creation);
        let result = ctx.new_monster_at_location(self, unit, row, player, flags, at, r);
        self.monster_def_index = ctx.monster_def_index;
        let made = result?;
        if let Some(stop) = ctx.stops.into_iter().next() {
            return Err(stop);
        }
        if let Some(i) = made {
            self.shown_monsters.push(i);
        }
        Ok(made)
    }

    pub(crate) fn replacement_monster(
        &mut self,
        unit: [u8; 4],
        p: &crate::projectiles::FlightProjectile,
        r: &mut EngineRng,
    ) -> Result<Option<usize>, Unmodelled> {
        let gameplay = self.formation_globals.gameplay_version;
        let player = p.record_36;
        if gameplay <= game_options::GAMEPLAY_1_6
            || matches!(u32::from_be_bytes(unit), 0 | u32::MAX)
            || player == -1
        {
            return Ok(None);
        }
        let rf = p.record_flags;
        let flags_4 = u32::from((rf >> 5) & 1) << 28;
        let flags_8 = (u32::from(rf & 0x40) << 6) | if rf & 0x80 != 0 { 2 } else { 0 };
        let at = MonsterLocation {
            x: p.object.x,
            y: p.object.y,
            z: p.object.z,
            facing: p.facing,
        };
        let Some(n) =
            self.new_monster_at_location(FourCC::new(unit), player, (flags_4, flags_8), at, r)?
        else {
            return Ok(None);
        };
        if gameplay < 0x6a0 {
            self.monsters[n].flags |= flags_4;
            self.monsters[n].flags_8 |= flags_8;
        }
        let own_colours = self
            .players
            .players
            .get(player as usize)
            .is_some_and(|pl| pl.flags & 1 != 0);
        if !own_colours
            && let Some(o) = p.shape.as_ref()
            && self.monsters[n].object.collection == o.collection
        {
            self.monsters[n].object.permutation = o.permutation;
        }
        Ok(Some(n))
    }

    fn marker_context<'a>(&mut self, creation: &'a MarkerCreation) -> MarkerContext<'a> {
        MarkerContext {
            stops: Vec::new(),
            mesh: &creation.mesh,
            tags: creation,
            params: creation.params,
            monster_def_index: std::mem::take(&mut self.monster_def_index),
            game_type: creation.game_type,
            game_options: creation.game_options,
            mode: creation.mode,
            replay: creation.replay,
            players: self.players.clone(),
            unit_palettes: creation.unit_palettes.clone(),
            after_load: true,
            row_entry_player: None,
        }
    }

    pub(crate) fn marker_is_unplaced(&self, m: &MarkerEntry) -> bool {
        self.marker_unplaced
            .get(&m.marker_id)
            .copied()
            .unwrap_or(m.flags & 1 != 0)
    }

    fn monster_placement_data(
        &mut self,
        ctx: &mut MarkerContext<'_>,
        team: i16,
    ) -> Result<(Vec<MonsterPlacement>, i16), Unmodelled> {
        let version = self.formation_globals.gameplay_version;
        let difficulty = ctx.params.difficulty;
        let palettes: Vec<_> = ctx
            .mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == 3)
            .collect();
        let mut list = [MonsterPlacement::default(); 16];
        let mut n = 0usize;
        for (index, p) in palettes.iter().enumerate() {
            let entry = ctx.unit_palettes[index];
            if entry.team != team || p.flags as u16 & 8 != 0 {
                continue;
            }
            let row = ctx.placement_row(self, p.tag_id, entry)?;
            let at = match list[..n].iter().position(|e| e.row == row.0) {
                Some(at) => at,
                None if n > 0xf => continue,
                None => {
                    n += 1;
                    list[n - 1] = MonsterPlacement {
                        palette: index as i16,
                        row: row.0,
                        cost: row.1.max(0),
                        ..MonsterPlacement::default()
                    };
                    n - 1
                }
            };
            let compatible = ctx.palette_compatible(p, draw_site::PLACEMENT_PALETTE)?;
            for m in ctx
                .mesh
                .markers
                .iter()
                .filter(|m| m.marker_type == 3 && m.palette_index == index as i16)
            {
                let e = &mut list[at];
                if p.flags as u16 & 2 != 0 {
                    e.adjustable = true;
                }
                if (version < 0x515 || m.difficulty_flags <= difficulty) && compatible {
                    e.maximum += 1;
                    if !self.marker_is_unplaced(m) {
                        e.placed += 1;
                        if p.flags as u16 & 0x12 != 2 {
                            e.minimum += 1;
                        }
                    }
                }
                if e.maximum == e.minimum {
                    e.adjustable = false;
                }
            }
            if version > 0x6ad
                && (list[at].maximum == 0
                    || (version != game_options::GAMEPLAY_1_7_1 && !list[at].adjustable))
            {
                n -= 1;
            }
        }
        let list = &mut list[..n];
        list.sort_by(|a, b| {
            b.adjustable
                .cmp(&a.adjustable)
                .then(b.cost.cmp(&a.cost))
                .then(b.maximum.cmp(&a.maximum))
                .then(if version > game_options::GAMEPLAY_1_6 {
                    b.palette.cmp(&a.palette)
                } else {
                    std::cmp::Ordering::Equal
                })
        });
        Ok((list.to_vec(), n.min(8) as i16))
    }

    pub(crate) fn placement_rows(&mut self, team: i16) -> Option<Vec<MonsterPlacement>> {
        let creation = self.marker_creation.clone()?;
        let (definitions, index) = (
            self.monster_definitions.len(),
            self.monster_def_index.clone(),
        );
        let mut ctx = self.marker_context(&creation);
        let result = self.monster_placement_data(&mut ctx, team);
        drop(ctx);
        self.monster_definitions.truncate(definitions);
        self.monster_def_index = index;
        let (mut list, count) = result.ok()?;
        list.truncate(count.max(0) as usize);
        Some(list)
    }

    pub(crate) fn adjust_unit_placement(
        &mut self,
        team: i16,
        counts: &[i16],
        r: &mut EngineRng,
        time: i32,
    ) -> Result<Vec<i16>, Unmodelled> {
        let Some(creation) = self.marker_creation.clone() else {
            return Err(Unmodelled::new(
                draw_site::ADJUST_PLACEMENT_NO_MARKERS,
                "adjust_unit_placement without the load's markers",
            ));
        };
        let mut ctx = self.marker_context(&creation);
        let result = self.adjust_unit_placement_in(&mut ctx, team, counts, r, time);
        self.monster_def_index = std::mem::take(&mut ctx.monster_def_index);
        let deleted = result?;
        if let Some(stop) = ctx.stops.into_iter().next() {
            return Err(stop);
        }
        Ok(deleted)
    }

    fn adjust_unit_placement_in(
        &mut self,
        ctx: &mut MarkerContext<'_>,
        team: i16,
        counts: &[i16],
        r: &mut EngineRng,
        time: i32,
    ) -> Result<Vec<i16>, Unmodelled> {
        let version = self.formation_globals.gameplay_version;
        let difficulty = ctx.params.difficulty;
        let (list, count) = self.monster_placement_data(ctx, team)?;
        let state = self.game.state;
        let early = self.game.game_time.wrapping_sub(self.game.timer) < 0x1c2;
        let n = counts.len();
        if self.game_options & mandrake_common::game_options::UNIT_TRADING == 0
            || n >= 0x10
            || !(state == 0 || (state == 2 && early))
            || count as usize != n
        {
            return Ok(Vec::new());
        }
        let mut left = [0i16; 16];
        left[..n].copy_from_slice(counts);
        let list = &list[..n];
        let mut valid = true;
        let (mut spent, mut points) = (0i32, 0i32);
        for (e, &c) in list.iter().zip(counts) {
            valid &= e.minimum <= c && c <= e.maximum;
            spent += i32::from(c) * i32::from(e.cost);
            points += i32::from(e.placed) * i32::from(e.cost);
        }
        if n > 0 {
            for (e, c) in list.iter().zip(&mut left) {
                let cost = i32::from(e.cost);
                if (cost != 0 || version < 0x6a5) && cost <= points - spent {
                    let mut rest = points - spent - cost;
                    loop {
                        if e.maximum <= *c {
                            break;
                        }
                        spent += cost;
                        *c += 1;
                        let more = cost <= rest;
                        rest -= cost;
                        if !more {
                            break;
                        }
                    }
                }
            }
            if points < spent || !valid {
                return Ok(Vec::new());
            }
        }
        let palettes: Vec<_> = ctx
            .mesh
            .palettes
            .iter()
            .filter(|p| p.marker_type == 3)
            .collect();
        let mut deleted = Vec::new();
        for (index, p) in palettes.iter().enumerate() {
            let entry = ctx.unit_palettes[index];
            let compatible = ctx.palette_compatible(p, draw_site::ADJUST_PLACEMENT_PALETTE)?;
            if entry.team != team || p.flags as u16 & 8 != 0 {
                continue;
            }
            let row = ctx.placement_row(self, p.tag_id, entry)?.0;
            let at = if compatible {
                list.iter().position(|e| e.row == row)
            } else {
                None
            };
            let markers: Vec<&MarkerEntry> = ctx
                .mesh
                .markers
                .iter()
                .filter(|m| m.marker_type == 3 && m.palette_index == index as i16)
                .collect();
            let Some(at) = at else {
                if version > game_options::GAMEPLAY_1_7 {
                    for m in &markers {
                        if self.marker_is_created(m)
                            && let Some(i) = self.monster_index_by_identifier(m.marker_id as u16)
                        {
                            self.monsters[i].flags_8 &= !0x80_0000;
                        }
                    }
                }
                continue;
            };
            let in_difficulty =
                |m: &MarkerEntry| version < 0x515 || m.difficulty_flags <= difficulty;
            for m in &markers {
                if self.marker_is_created(m)
                    && let Some(i) = self.monster_index_by_identifier(m.marker_id as u16)
                {
                    self.monsters[i].flags_8 &= !0x80_0000;
                    let unit = self.statistics.monster_unit(i);
                    let hit_points = i32::from(self.monsters[i].object.vitality as i16);
                    self.statistics
                        .adjust_unit_statistics(unit, -1, -hit_points);
                    let mut actions = self.map_actions.take();
                    let done = WorldHooks {
                        world: &mut *self,
                        actions: &mut actions,
                        rng: &mut *r,
                        monster: i,
                        time,
                        host: &mut crate::engine_attack::NoHost,
                    }
                    .delete_monster();
                    self.map_actions = actions;
                    done?;
                    self.set_marker_created(m, false);
                    deleted.push(m.marker_id);
                }
                if !self.marker_is_unplaced(m) {
                    if in_difficulty(m) && left[at] != 0 {
                        left[at] -= 1;
                    } else {
                        self.marker_unplaced.insert(m.marker_id, true);
                    }
                }
            }
            for m in &markers {
                if self.marker_is_unplaced(m) && in_difficulty(m) && left[at] != 0 {
                    self.marker_unplaced.insert(m.marker_id, false);
                    left[at] -= 1;
                }
            }
            self.monster_initializers.mark_unused();
            for m in &markers {
                let saved = ctx.unit_palettes[index];
                if version > game_options::GAMEPLAY_1_6 {
                    let captain = self
                        .players
                        .teams
                        .get(team as usize)
                        .map_or(-1, |t| t.player);
                    let tag = u32::from_be_bytes(p.tag_id.0);
                    if saved.row.owner != captain
                        && let Some(own) = self.unit_rows.get(&(tag, captain))
                    {
                        ctx.unit_palettes[index].row = *own;
                        ctx.unit_palettes[index].player = captain;
                        ctx.row_entry_player = Some(saved.player);
                    }
                }
                let mut marker = (*m).clone();
                marker.flags = (marker.flags & !1) | u32::from(self.marker_is_unplaced(m));
                self.set_marker_created(m, false);
                let result = ctx.new_monster(self, &marker, r);
                ctx.unit_palettes[index] = saved;
                ctx.row_entry_player = None;
                result?;
                if let Some(i) = self
                    .marker_is_created(m)
                    .then(|| self.monster_index_by_identifier(m.marker_id as u16))
                    .flatten()
                {
                    let mon = &self.monsters[i];
                    if mon.flags & 0x4000_0000 == 0 && !mon.object.no_shape && m.flags & 4 == 0 {
                        self.shown_monsters.push(i);
                    }
                }
            }
        }
        Ok(deleted)
    }

    pub(crate) fn detach_command(
        &mut self,
        ids: &[u16],
        target: i16,
        player: i16,
    ) -> Result<Vec<usize>, Unmodelled> {
        let max = if self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
            0x7e
        } else {
            0x1e
        };
        let monsters: Vec<usize> = ids
            .iter()
            .take(max)
            .filter_map(|&id| self.monster_index_by_identifier(id))
            .collect();
        let monsters = self.filter_command_monsters(&monsters, -1, player);
        for &m in &monsters {
            self.detach_monster(m, target)?;
        }
        Ok(monsters)
    }

    pub(crate) fn detach_monster(&mut self, i: usize, player: i16) -> Result<(), Unmodelled> {
        self.detach_monster_with(i, player, false)
    }

    pub(crate) fn detach_monster_with(
        &mut self,
        i: usize,
        player: i16,
        quiet: bool,
    ) -> Result<(), Unmodelled> {
        if self.monsters[i].owner == player || player == -1 {
            return Ok(());
        }
        let Some(record) = usize::try_from(player)
            .ok()
            .and_then(|p| self.players.players.get(p))
            .copied()
        else {
            return Err(Unmodelled::new(
                draw_site::DETACH_NOT_A_PLAYER,
                format!("detach_monster: player {player} isn't a player"),
            ));
        };
        if record.flags & 0x44 != 0 {
            return Ok(());
        }
        let unit = self.statistics.monster_unit(i);
        let Some(old) = usize::try_from(unit)
            .ok()
            .and_then(|u| self.statistics.units.get(u))
            .copied()
        else {
            return Err(Unmodelled::new(
                draw_site::DETACH_NO_UNIT_ROW,
                format!("detach_monster: monster {i} has no unit row"),
            ));
        };
        let row = if self.monsters[i].flags & 0x1000_0000 != 0 {
            Some(self.statistics.tag_row(old.tag, None) as usize)
        } else {
            let mons = self.monster_definitions[self.own_definition(i)].tag;
            let mons = u32::from_be_bytes(mons.0);
            self.statistics.find_unit(player, old.tag).or_else(|| {
                self.statistics
                    .units
                    .iter()
                    .position(|u| u.owner == player && u.mons_tag == mons)
            })
        };
        let Some(row) = row else {
            return Ok(());
        };
        if row as i16 == unit {
            return Ok(());
        }
        let new = self.statistics.units[row];
        let hit_points = i32::from(self.monsters[i].object.vitality as i16);
        self.statistics.monster_detached(i, row as i16, hit_points);
        let own = self.unit_rows.get(&(new.tag, new.owner)).copied();
        if record.flags & 1 != 0 {
            let custom = own.is_some_and(|r| r.custom_core);
            let count = if custom {
                1
            } else {
                self.marker_creation
                    .as_ref()
                    .and_then(|c| {
                        let unit = c.tag(*b"unit", FourCC(new.tag.to_be_bytes()))?;
                        let core = c.tag(*b"core", fourcc(unit.get(..8)?, 4))?;
                        CollectionReference::from_bytes(&core).ok()
                    })
                    .map_or(1, |c| c.permutation_count)
            };
            if self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_8_4 {
                let last = count - 1;
                let p = &mut self.monsters[i].object.permutation;
                *p = (*p).min(last);
            }
        }
        self.combat_mut(i).unit = (new.tag, new.owner);
        let local = self.local_player;
        let m = &mut self.monsters[i];
        let lost = m.owner == local;
        let gained = player == local;
        m.owner = player;
        m.team = i16::from(record.team);
        if quiet {
            return Ok(());
        }
        if lost {
            self.game
                .game_sound_requests
                .push(crate::game_state::GAME_SOUND_UNIT_LOST);
        } else if gained {
            self.game
                .game_sound_requests
                .push(crate::game_state::GAME_SOUND_UNIT_GAINED);
        }
        Ok(())
    }

    fn marker_is_created(&self, m: &MarkerEntry) -> bool {
        self.unit_markers
            .iter()
            .any(|u| u.identifier == m.marker_id && u.created)
    }

    pub(crate) fn set_marker_created(&mut self, m: &MarkerEntry, created: bool) {
        if let Some(u) = self
            .unit_markers
            .iter_mut()
            .find(|u| u.identifier == m.marker_id)
        {
            u.created = created;
        }
    }

    fn alloc_monster_index(&mut self) -> Option<(usize, i16)> {
        self.invalidate_monster_index();
        let index = self
            .monsters
            .iter()
            .position(|m| m.identifier == 0)
            .or((self.monsters.len() < MONSTER_CAPACITY).then_some(self.monsters.len()))?;
        let identifier = self.monster_datums | 0x8000;
        self.monster_datums = self.monster_datums.wrapping_add(1) & 0x7fff;
        Some((index, identifier as i16))
    }

    pub fn new_object_datum(&mut self) -> (i32, i16) {
        let index = if let Some(i) = self.free_objects.pop_first() {
            i
        } else {
            let i = self.next_object;
            self.next_object += 1;
            i
        };
        self.object_history.remove(&index);
        let identifier = self.object_datums | 0x8000;
        self.object_datums = self.object_datums.wrapping_add(1) & 0x7fff;
        (index, identifier as i16)
    }

    pub fn monster_counter(&self) -> u16 {
        self.monster_datums | 0x8000
    }

    pub fn object_counter(&self) -> u16 {
        self.object_datums | 0x8000
    }

    pub fn free_object_identifier(&self, index: i32, wanted: i16) -> i16 {
        let mut id = wanted;
        for o in self.filter_objects() {
            if o.index != index && o.identifier != 0 && o.identifier == id {
                id = self.object_counter() as i16;
            }
        }
        id
    }

    pub fn new_object_with(
        &mut self,
        obje: (u16, u16, i16),
        collection: FourCC,
        scale: i32,
        position: (i32, i32, i32),
        r: &mut EngineRng,
    ) -> EngineObject {
        let world = self;
        let (index, identifier) = world.new_object_datum();
        let mut o = EngineObject::new(index, collection);
        o.identifier = identifier.wrapping_neg();
        o.z = position.2;
        let c = world.cells.cell_index(position.0 >> 9, position.1 >> 9);
        let media = world.heights.get(c).map_or(0, |h| i32::from(h.1));
        if media > position.2
            && world.cells.flags[c] & 0x1800 != 0
            && world.media.pixel_is_media(position.0 >> 6, position.1 >> 6)
        {
            o.flags |= 0xc0;
        }
        let delta = i32::from(obje.2);
        let extra = if delta > 0 {
            (i32::from(r.draw(draw_site::NEW_OBJECT_SCALE)) * delta) >> 16
        } else {
            0
        };
        o.vitality = i32::from(obje.1) + extra;
        o.max_vitality = o.vitality;
        o.scale = scale;
        if obje.0 & 0x10 != 0 {
            o.flags |= 0x400;
        }
        o
    }
}

fn marker_words(mesh_data: &[u8], mesh: &MeshTag, i: usize) -> [i16; 4] {
    let at = MeshTag::HEADER_SIZE + mesh.marker_offset as usize + i * MarkerEntry::SIZE + 0x18;
    let word = |k: usize| {
        mesh_data
            .get(at + 2 * k..at + 2 * k + 2)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
    };
    [word(0), word(1), word(2), word(3)]
}

fn load_projectile_catalog(
    world: &mut EngineWorld,
    mesh: &MeshTag,
    actions: Option<&MeshMapActions>,
    tags: &dyn TagSource,
) {
    let mut roots = CatalogRoots {
        groups: INTERNAL_PROJECTILE_GROUPS
            .iter()
            .map(|t| FourCC::new(*t))
            .collect(),
        projectiles: INTERNAL_PROJECTILES
            .iter()
            .map(|t| FourCC::new(*t))
            .collect(),
        ..CatalogRoots::default()
    };
    for p in actions
        .iter()
        .flat_map(|a| &a.actions)
        .filter(|r| r.tag == u32::from_be_bytes(*b"moef"))
        .flat_map(|r| &r.parameters)
        .filter(|p| p.kind == 7 && p.name == FourCC::new(*b"proj"))
    {
        roots.projectiles.extend(
            p.values
                .iter()
                .map(|&v| v as u32)
                .filter(|&v| v != u32::MAX)
                .map(|v| FourCC::new(v.to_be_bytes())),
        );
    }
    for p in actions
        .iter()
        .flat_map(|a| &a.actions)
        .flat_map(|r| &r.parameters)
    {
        let refs = p
            .values
            .iter()
            .map(|&v| v as u32)
            .filter(|&v| v != u32::MAX);
        let refs = refs.map(|v| FourCC::new(v.to_be_bytes()));
        match p.kind {
            9 => roots.groups.extend(refs),
            12 => roots.projectiles.extend(refs),
            _ => {}
        }
    }
    if let Some(m) = tags
        .tag(*b"medi", mesh.media_tag)
        .and_then(|d| mandrake_tag::media::MediaDefinition::from_bytes(mesh.media_tag, &d).ok())
    {
        roots.groups.extend(
            (0..mandrake_tag::media::MEDIA_PROJECTILE_GROUP_COUNT)
                .filter_map(|t| m.projectile_group(t)),
        );
    }
    for p in &mesh.palettes {
        match p.marker_type {
            1 => {
                let Some(scen) = tags.tag(*b"scen", p.tag_id).filter(|s| s.len() >= 0x20) else {
                    continue;
                };
                roots.projectiles.extend(tag_ref(&scen, 0x0c));
                roots.groups.extend(tag_ref(&scen, 0x18));
                roots.groups.extend(tag_ref(&scen, 0x1c));
            }
            3 => {
                if let Some(unit) = tags.tag(*b"unit", p.tag_id).filter(|u| u.len() >= 8) {
                    catalog_monster(&mut roots, fourcc(&unit, 0), tags);
                }
            }
            9 => roots.projectiles.push(p.tag_id),
            _ => {}
        }
    }
    let gameplay = world.formation_globals.gameplay_version;
    let mut catalog = std::mem::take(&mut world.catalog);
    let mut headers = |c: FourCC| {
        world.load_collection(tags, c).ok()?;
        world.collections.get(&c).cloned()
    };
    let mut read = std::collections::HashSet::new();
    loop {
        for g in std::mem::take(&mut roots.groups) {
            catalog.add_group(tags, &mut headers, g);
        }
        for p in std::mem::take(&mut roots.projectiles) {
            catalog.add_projectile(tags, &mut headers, p);
        }
        let mut new: Vec<FourCC> = catalog
            .projectiles
            .keys()
            .copied()
            .filter(|p| read.insert(*p))
            .collect();
        if new.is_empty() {
            break;
        }
        new.sort_unstable();
        for p in new {
            if let Some(a) = tags
                .tag(*b"proj", p)
                .filter(|d| d.len() >= 0x7c)
                .and_then(|d| tag_ref(&d, 0x78))
            {
                catalog_artifact(&mut roots, a, gameplay, tags);
            }
        }
    }
    let teams: Vec<i16> = world
        .players
        .players
        .iter()
        .map(|p| i16::from(p.team))
        .collect();
    catalog.artifacts = crate::artifact_order::artifact_load_order(
        mesh,
        actions,
        tags,
        gameplay,
        &teams,
        &INTERNAL_PROJECTILES,
        &INTERNAL_PROJECTILE_GROUPS,
    );
    world.catalog = catalog;
}

const INTERNAL_PROJECTILE_GROUPS: [[u8; 4]; 14] = [
    *b"inhs", *b"inhe", *b"inps", *b"inpe", *b"inpd", *b"inss", *b"inse", *b"insi", *b"insd",
    *b"inti", *b"into", *b"inte", *b"inte", *b"in01",
];

const INTERNAL_PROJECTILES: [[u8; 4]; 2] = [*b"indc", *b"inst"];

#[derive(Default)]
struct CatalogRoots {
    groups: Vec<FourCC>,
    projectiles: Vec<FourCC>,
    monsters: std::collections::HashSet<FourCC>,
    artifacts: Vec<(FourCC, Vec<u8>)>,
}

fn catalog_monster(roots: &mut CatalogRoots, id: FourCC, tags: &dyn TagSource) {
    if !roots.monsters.insert(id) {
        return;
    }
    let Some(mons) = tags.tag(*b"mons", id).filter(|d| d.len() >= 0x224) else {
        return;
    };
    for at in [0x194, 0x1a4, 0x3c, 0x1a0, 0x1e8, 0x1e4, 0x204, 0x210, 0x220] {
        roots.groups.extend(tag_ref(&mons, at));
    }
    let attacks = be_i16(&mons, 0x88).clamp(0, 4) as usize;
    for k in 0..attacks {
        roots
            .projectiles
            .extend(tag_ref(&mons, 0x8c + 0x40 * k + 4));
    }
    roots.projectiles.extend(tag_ref(&mons, 0x1ec));
}

fn catalog_artifact(roots: &mut CatalogRoots, id: FourCC, gameplay: i32, tags: &dyn TagSource) {
    if roots.artifacts.iter().any(|(t, _)| *t == id) {
        return;
    }
    let Some(arti) = tags.tag(*b"arti", id).filter(|d| d.len() >= 0xa8) else {
        return;
    };
    roots.artifacts.push((id, arti.to_vec()));
    let flags = be_i32(&arti, 0) as u32;
    if gameplay > game_options::GAMEPLAY_1_7_2 && flags & 0x2_0000 != 0 {
        for at in [0x1c, 0x20, 0x24, 0x28] {
            roots.projectiles.extend(tag_ref(&arti, at));
        }
    }
    if flags & 4 != 0
        && let Some(m) = tag_ref(&arti, 0x98)
    {
        let m = if flags & 0x8_0000 != 0 {
            tags.tag(*b"unit", m)
                .filter(|u| u.len() >= 4)
                .map_or(m, |u| fourcc(&u, 0))
        } else {
            m
        };
        catalog_monster(roots, m, tags);
    }
    if flags & 0x100 != 0
        && gameplay > game_options::GAMEPLAY_1_5_1
        && let Some(m) = tag_ref(&arti, 0x04)
    {
        catalog_monster(roots, m, tags);
    }
    roots.projectiles.extend(tag_ref(&arti, 0x58));
}

pub fn monsters_are_allied(
    alliances: &crate::ally::AllianceView,
    a: (i16, u32, i16),
    b: (i16, u32, i16),
) -> bool {
    if a.0 == 5 || b.0 == 5 {
        return false;
    }
    if a.1 & 0x4000 != 0 || b.1 & 0x4000 != 0 {
        return false;
    }
    if a.2 == b.2 {
        return true;
    }
    if a.2 < 0 || b.2 < 0 {
        return false;
    }
    alliances.teams_are_allied(a.2, b.2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetreatSource {
    pub position: (i32, i32),
    pub allied: bool,
}

pub fn point_of_retreat(
    me: (i32, i32),
    from: Option<(i32, i32)>,
    sources: &[RetreatSource],
    mesh_cells: (i32, i32),
) -> Result<(bool, (i32, i32), u16), Unmodelled> {
    let (x, y) = me;
    let (w, h) = mesh_cells;
    if w < 21 || h < 21 {
        return Err(Unmodelled::new(
            draw_site::RETREAT_SMALL_MAP,
            "generate_point_of_retreat on a map under 21 cells",
        ));
    }
    let corner = |d: i32, size: i32| if d < 0 { 0 } else { (size - 0x15).min(d) };
    let cx = corner((x >> 9) - 10, w);
    let cy = corner((y >> 9) - 10, h);
    let length = |dx: i32, dy: i32| {
        crate::projectiles::isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32)
    };
    let (mut found, mut sx, mut sy) = (false, 0i32, 0i32);
    if let Some(p) = from {
        let (dx, dy) = (x.wrapping_sub(p.0), y.wrapping_sub(p.1));
        let len = length(dx, dy);
        if len != 0 {
            found = true;
            sx = dx.wrapping_shl(10) / len;
            sy = dy.wrapping_shl(10) / len;
        }
    }
    for s in sources {
        let (ox, oy) = s.position;
        if !(cx..cx + 21).contains(&(ox >> 9)) || !(cy..cy + 21).contains(&(oy >> 9)) {
            continue;
        }
        let (dx, dy) = (x.wrapping_sub(ox), y.wrapping_sub(oy));
        let d = length(dx, dy);
        if d == 0 {
            continue;
        }
        let shift = if s.allied { 8 } else { 10 };
        found = true;
        sx = sx.wrapping_add(dx.wrapping_shl(shift) / d);
        sy = sy.wrapping_add(dy.wrapping_shl(shift) / d);
    }
    let mut facing = crate::engine_move::arctangent(if sx | sy != 0 { sx } else { 1 }, sy);
    if let Some(p) = from {
        let away = crate::engine_move::arctangent(x.wrapping_sub(p.0), y.wrapping_sub(p.1));
        let mut d = i32::from(facing) - i32::from(away);
        if d > 0x7fff {
            d = 0x10000 - d;
        }
        if d <= -0x8000 {
            d += 0x10000;
        }
        if d < -0x2aaa {
            facing = away.wrapping_sub(0x2aaa);
        } else if d > 0x2aaa {
            facing = away.wrapping_add(0x2aaa);
        }
    }
    let t = crate::engine_move::trig_tables();
    let i = (facing.wrapping_add(0x20) >> 6) as usize;
    let px = (i32::from(t.cosine[i]) >> 2) + x;
    let py = (i32::from(t.sine[i]) >> 2) + y;
    let clamp = |v: i32, size: i32| if v < 0 { 0 } else { ((size - 1) << 9).min(v) };
    Ok((found, (clamp(px, w), clamp(py, h)), facing))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveClick {
    Ground(i32, i32),
    LocalSprite,
}

pub type LocalCommand = (MovementHeader, Vec<(i32, i32)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MovementHeader {
    pub flags: u16,
    pub formation: i16,
    pub facing: u16,
}

pub fn angular_difference(a: u16, b: u16) -> i32 {
    let d = i32::from(a.wrapping_sub(b));
    if d > 0x7fff { 0x10000 - d } else { d }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormationCounter(pub i16);

impl Default for FormationCounter {
    fn default() -> Self {
        Self(-1)
    }
}

impl Default for crate::formations::FormationDefinition {
    fn default() -> Self {
        Self::STATIC_DEFAULT
    }
}

impl Default for crate::formations::FormationGlobals {
    fn default() -> Self {
        Self {
            game_options_high: 0,
            gameplay_version: crate::gameplay::GAMEPLAY_1_8_5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineWind {
    pub countdown: i32,
    pub period: i32,
    pub period_range: i32,
    pub rate: i32,
    pub base: [i32; 3],
    pub range: [i32; 3],
    pub velocity: [i32; 3],
    pub target: [i32; 3],
}

impl EngineWind {
    fn change(&mut self, rng: &mut EngineRng, site: u32) {
        let r = if self.period_range > 0 {
            (i32::from(rng.draw(site)) * self.period_range) >> 16
        } else {
            0
        };
        self.countdown = self.period + r;
        for (k, site) in [
            draw_site::WIND_TARGET_X,
            draw_site::WIND_TARGET_Y,
            draw_site::WIND_TARGET_Z,
        ]
        .into_iter()
        .enumerate()
        {
            let range = self.range[k] >> 8;
            let extra = if range > 0 {
                (range.wrapping_mul(i32::from(rng.draw(site)))) >> 16
            } else {
                0
            };
            self.target[k] = ((self.base[k] >> 8) + extra) << 8;
        }
    }

    fn approach(&mut self) {
        for k in 0..3 {
            let (t, v) = (self.target[k], self.velocity[k]);
            self.velocity[k] = if t < v {
                t.max(v - self.rate)
            } else if t > v {
                t.min(v + self.rate)
            } else {
                v
            };
        }
    }

    pub fn velocity_words(&self) -> [i16; 3] {
        self.velocity.map(|v| (v >> 16) as i16)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathCall {
    pub time: i32,
    pub monster: usize,
    pub site: u32,
    pub position: (i32, i32),
    pub path: Option<crate::pathfinding::Path>,
    pub target: (i32, i32),
    pub detour: bool,
    pub detour_side: bool,
    pub avoid: i16,
    pub obstacles: Vec<crate::pathfinding::Obstacle>,
}

impl crate::pathfinding::ObstacleWorld for EngineWorld {
    fn cells(&self) -> &MeshCells {
        &self.cells
    }

    fn fire_of(&self, monster: i16) -> crate::pathfinding::WorldFireCells<'_> {
        match usize::try_from(monster) {
            Ok(monster) => crate::pathfinding::WorldFireCells::World(crate::fire::WorldFire {
                world: self,
                monster,
            }),
            Err(_) => crate::pathfinding::WorldFireCells::None,
        }
    }

    fn cell_objects(
        &self,
        x: i32,
        y: i32,
    ) -> Result<Vec<crate::pathfinding::ObjectView>, Unmodelled> {
        let mut v: Vec<(u64, crate::pathfinding::ObjectView)> = Vec::new();
        let sc_index = self.scenery_index.of(&self.scenery);
        for &i in sc_index.cell(x, y) {
            let sc = &self.scenery[i as usize];
            if !sc.deleted {
                v.push((sc.cell_link, self.scenery_view(i as usize)));
            }
        }
        for i in self.monsters_in_cell(x, y) {
            v.push((self.monsters[i].cell_link, self.monster_object_view(i)));
        }
        for r in self
            .solid_copies()
            .filter(|r| (r.position.0 >> 9, r.position.1 >> 9) == (x, y))
        {
            v.push((r.cell_link, self.copy_view(r)));
        }
        v.extend(
            self.solid_projectile_objects()
                .filter(|(_, o)| (o.position.0 >> 9, o.position.1 >> 9) == (x, y)),
        );
        v.sort_by_key(|&(link, _)| std::cmp::Reverse(link));
        Ok(v.into_iter().map(|(_, o)| o).collect())
    }

    fn window_objects(&self, x: i32, y: i32) -> Option<Vec<Vec<crate::pathfinding::ObjectView>>> {
        Some(self.objects_in_box(x, y, 16, 16))
    }

    fn window_cells(&self, x: i32, y: i32) -> [u16; 16] {
        let mut cells = [0u16; 16];
        let index = self.scenery_index.of(&self.scenery);
        for (j, row) in cells.iter_mut().enumerate() {
            for i in 0..16 {
                let here = index.cell(x + i, y + j as i32);
                if here.iter().any(|&k| !self.scenery[k as usize].deleted) {
                    *row |= 1 << i;
                }
            }
        }
        if self.monster_index.active {
            for (j, row) in cells.iter_mut().enumerate() {
                for i in 0..16 {
                    if *row & (1 << i) == 0
                        && self
                            .monster_index
                            .cell_iter(x + i, y + j as i32, &self.monsters)
                            .next()
                            .is_some()
                    {
                        *row |= 1 << i;
                    }
                }
            }
        }
        let mut mark = |p: (i32, i32)| {
            let (i, j) = ((p.0 >> 9) - x, (p.1 >> 9) - y);
            if (0..16).contains(&i) && (0..16).contains(&j) {
                cells[j as usize] |= 1 << i;
            }
        };
        if !self.monster_index.active {
            for m in self.monsters.iter().filter(|m| m.identifier != 0) {
                mark(m.position);
            }
        }
        for r in self.solid_copies() {
            mark(r.position);
        }
        for p in &self.projectile_records {
            if let Some(o) = p.shape.as_ref()
                && !o.no_shape
                && o.flags & 0x600 == 0x400
            {
                mark((p.object.x, p.object.y));
            }
        }
        for d in &self.dormant_objects {
            if !d.object.no_shape && d.object.flags & 0x600 == 0x400 {
                mark((d.position[0], d.position[1]));
            }
        }
        for (_, o) in &self.solid_elsewhere {
            mark(o.position);
        }
        cells
    }

    fn object(&self, index: i16) -> Result<crate::pathfinding::ObjectView, Unmodelled> {
        let idx = i32::from(index);
        if let Some(i) = self.monster_index_by_object(idx) {
            return Ok(self.monster_object_view(i));
        }
        let sc_index = self.scenery_index.of(&self.scenery);
        if let Some(i) = sc_index.of_object(idx).find(|&i| !self.scenery[i].deleted) {
            return Ok(self.scenery_view(i));
        }
        if let Some(r) = self.solid_copies().find(|r| r.object.index == idx) {
            return Ok(self.copy_view(r));
        }
        if let Some((_, o)) = self.solid_projectile_object_by_index(index) {
            return Ok(o);
        }
        if let Some(pr) = self
            .projectiles
            .iter()
            .find(|p| p.object.index == idx && !p.object.no_shape)
        {
            return Ok(self.resting_projectile_view(pr));
        }
        Err(Unmodelled::new(
            draw_site::OBJECT_NO_ENGINE_MODEL,
            format!("object {index} is not modelled"),
        ))
    }

    fn monster(&self, index: i16) -> Result<crate::pathfinding::MonsterView, Unmodelled> {
        let m = self.monsters.get(index as usize).ok_or_else(|| {
            Unmodelled::new(
                draw_site::OBJECT_NO_ENGINE_MODEL,
                format!("monster {index} is not modelled"),
            )
        })?;
        Ok(crate::pathfinding::MonsterView {
            index,
            definition: m.definition as i16,
            flags: m.flags,
            flags_8: m.flags_8,
            object: m.object.index as i16,
            team: m.team as i8,
            target: self.combat(index as usize).target,
            pick_up: self.combat(index as usize).pick_up.0,
            pick_up_identifier: self.combat(index as usize).pick_up.1 as u16,
            flying_height: m.flying_height,
            speed: m.speed,
            mode: m.mode,
        })
    }

    fn definition(&self, d: i16) -> Result<crate::pathfinding::DefinitionView, Unmodelled> {
        let def = self.monster_definitions.get(d as usize).ok_or_else(|| {
            Unmodelled::new(
                draw_site::OBJECT_NO_ENGINE_MODEL,
                format!("monster definition {d} is not modelled"),
            )
        })?;
        let impassable = if self.monster_index.active {
            let imp = self.monster_index.impassable_of(d as usize);
            #[cfg(feature = "identity-checks")]
            assert_eq!(
                imp,
                self.monsters
                    .iter()
                    .find(|m| m.definition == d as usize)
                    .map_or(0, |m| m.impassable)
            );
            imp
        } else {
            self.monsters
                .iter()
                .find(|m| m.definition == d as usize)
                .map_or(0, |m| m.impassable)
        };
        Ok(crate::pathfinding::DefinitionView {
            flags: def.flags,
            impassable,
            radius: def.pathfinding_radius,
            field_82: def.field_82,
            field_1b5: def.field_1b5,
            class: def.field_1b6,
        })
    }

    fn scenery_flags(&self, _definition: i16) -> Result<u32, Unmodelled> {
        Err(Unmodelled::new(
            draw_site::SCENERY_FLAGS_NO_ENGINE_MODEL,
            "scenery definition flags by index (the model keeps them per object)",
        ))
    }

    fn scenery_flags_of_object(&self, index: i16) -> Result<u32, Unmodelled> {
        let by_index = self.scenery_index.of(&self.scenery);
        by_index
            .of_object(i32::from(index))
            .map(|i| &self.scenery[i])
            .find(|s| !s.deleted)
            .map(|s| s.scenery_flags)
            .ok_or_else(|| {
                Unmodelled::new(
                    draw_site::SCENERY_FLAGS_NO_ENGINE_MODEL,
                    format!("scenery object {index}"),
                )
            })
    }

    fn game_options(&self) -> u32 {
        self.game_options
    }
    fn alliances(&self) -> crate::ally::AllianceView {
        self.alliance_view()
    }
}

impl EngineWorld {
    pub fn path_views(
        &self,
        i: usize,
        mover: &EngineMover,
    ) -> Result<
        (
            crate::pathfinding::MonsterView,
            crate::pathfinding::ObjectView,
        ),
        Unmodelled,
    > {
        if self.monsters.len() > crate::pathfinding::PATH_CAPACITY {
            return Err(Unmodelled::new(
                draw_site::PATH_CAPACITY,
                "more monsters than path records (new_datum can fail)",
            ));
        }
        let em = &self.monsters[i];
        let mut m = mover.monster_view(
            i as i16,
            em.definition as i16,
            em.object.index as i16,
            em.flags,
            em.flags_8,
            em.team as i8,
        );
        m.flying_height = em.flying_height;
        m.target = self.combat(i).target;
        m.pick_up = self.combat(i).pick_up.0;
        m.pick_up_identifier = self.combat(i).pick_up.1 as u16;
        let mut me = self.monster_object_view(i);
        me.position = (mover.x, mover.y);
        me.facing = mover.facing;
        Ok((m, me))
    }

    pub fn replan_walkers(
        &mut self,
        time: i32,
        rng: &mut EngineRng,
        host: &mut dyn crate::engine_attack::CombatHost,
    ) -> Result<(), (usize, Unmodelled)> {
        let mut i = (time & 0x3f) as usize;
        while i < self.monsters.len() {
            let m = &mut self.monsters[i];
            if m.identifier != 0 && m.flags & 0x4000_0000 == 0 {
                m.object.flags &= !0x200_0000;
            }
            let mut mover = self.movers.get_mut(i).and_then(Option::take);
            let r = self
                .celebrate_winner(i, mover.as_mut(), time, rng, host)
                .and_then(|()| self.replan_walker(i, mover.as_mut(), time));
            if let Some(mv) = mover {
                self.movers[i] = Some(mv);
            }
            if let Err(e) = r {
                return Err((i, e));
            }
            i += 0x40;
        }
        Ok(())
    }

    fn celebrate_winner(
        &mut self,
        i: usize,
        mover: Option<&mut EngineMover>,
        time: i32,
        rng: &mut EngineRng,
        host: &mut dyn crate::engine_attack::CombatHost,
    ) -> Result<(), Unmodelled> {
        let m = &self.monsters[i];
        if m.identifier == 0 || m.flags & 0x4000_0000 != 0 || m.team != self.game.winner {
            return Ok(());
        }
        if self.monster_definitions[m.definition].ai.class == 5
            || m.flags & 0x200_0000 != 0
            || m.mode > 6
            || (1u32 << m.mode) & 0x72 == 0
        {
            return Ok(());
        }
        let Some(mv) = mover else {
            return Err(Unmodelled::new(
                draw_site::CELEBRATE_WINNER_NO_MOVER,
                format!(
                    "monster {} celebrates without a movement model",
                    m.identifier
                ),
            ));
        };
        let r = rng.draw(draw_site::CELEBRATE_WINNER);
        let state = celebration_runs(r);
        let mut actions = self.map_actions.take();
        let mut hooks = WorldHooks {
            world: self,
            actions: &mut actions,
            rng,
            monster: i,
            time,
            host,
        };
        let done = mv.set_mode(
            MoveMode::Celebrating,
            ModeState::Value(state as _),
            &mut hooks,
        );
        self.map_actions = actions;
        done
    }

    fn replan_walker(
        &mut self,
        i: usize,
        mover: Option<&mut EngineMover>,
        time: i32,
    ) -> Result<(), Unmodelled> {
        let m = &self.monsters[i];
        if m.identifier == 0 {
            return Ok(());
        }
        if m.flags & 0x4000_0000 != 0 {
            return Ok(());
        }
        if matches!(m.mode, 4 | 6) {
            let mut mover = mover;
            self.replan_walker_leg(i, mover.as_deref_mut(), time)?;
            return self.search_target(i, mover.as_deref());
        }
        self.search_target(i, mover.as_deref())
    }

    fn search_target(&mut self, i: usize, mover: Option<&EngineMover>) -> Result<(), Unmodelled> {
        let m = &self.monsters[i];
        if matches!(m.mode, 2 | 3 | 7) || m.flags & 0x1d0_0006 != 0 || m.flags_8 & 8 != 0 {
            return Ok(());
        }
        let own = match mover {
            Some(mv) => crate::attack_ai::Own::of(mv),
            None => crate::attack_ai::Own {
                position: m.position,
                timer: 0,
                speed: m.speed,
                facing: m.facing,
                desired_facing: 0,
            },
        };
        let t = self.find_target(i, 0, &own)?;
        if t != -1 {
            self.engage_monster(i, t as usize, 0xa)?;
        }
        Ok(())
    }

    fn replan_walker_leg(
        &mut self,
        i: usize,
        mover: Option<&mut EngineMover>,
        time: i32,
    ) -> Result<(), Unmodelled> {
        let m = &self.monsters[i];
        if m.flags & 1 != 0 {
            let Some(mv) = mover else {
                return Err(Unmodelled::new(
                    draw_site::ENGAGED_LEG_NO_MOVER,
                    format!("monster {} engaged without a movement model", m.identifier),
                ));
            };
            if self.walk_for_attack(i, mv)? != 0 {
                self.monsters[i].flags |= 0x20;
            }
            return Ok(());
        }
        let Some(mv) = mover else {
            return Err(Unmodelled::new(
                draw_site::WALKER_LEG_NO_MOVER,
                format!("monster {} walks without a movement model", m.identifier),
            ));
        };
        if m.flags & 0x4000 != 0 {
            self.monsters[i].flags |= 0x20;
            return Ok(());
        }
        let (mview, _) = self.path_views(i, mv)?;
        let (trace, detour) = mv.adjust(&*self, mview)?;
        detour.write(self.monsters[i].record_flags());
        let t = mv.target;
        mv.leg_budget = crate::engine_move::distance2d(mv.x - t.0, mv.y - t.1) as i16;
        let (dx, dy) = (t.0 - mv.x, t.1 - mv.y);
        let m = &self.monsters[i];
        let facing = (m.flags & 8 != 0 && usize::try_from(self.combat(i).target) != Ok(i))
            || m.flags & crate::engine_move::FINAL_FACING == 0;
        if facing && dx | dy != 0 {
            mv.desired_facing = crate::engine_move::arctangent(dx, dy);
        }
        if let Some(log) = self.path_log.as_mut() {
            log.push(PathCall {
                time,
                monster: i,
                site: if facing {
                    draw_site::WALKER_LEG_FACING
                } else {
                    draw_site::WALKER_LEG
                },
                position: (mv.x, mv.y),
                path: None,
                target: mv.target,
                detour: detour.detour,
                detour_side: detour.side,
                avoid: mv.avoid,
                obstacles: trace.obstacles,
            });
        }
        Ok(())
    }

    fn scenery_view(&self, i: usize) -> crate::pathfinding::ObjectView {
        let sc = &self.scenery[i];
        let o = &sc.object;
        let (radius, low, high) = o.bounds(self.headers(o.collection)).unwrap_or((0, 0, 0));
        let mut flags = o.flags;
        if o.no_shape {
            flags |= 0x200;
        }
        crate::pathfinding::ObjectView {
            index: o.index as i16,
            identifier: 0,
            flags,
            position: sc.position,
            facing: sc.facing,
            radius,
            bounds_low: low,
            bounds_high: high,
            owner: o.owner,
            kind: o.kind,
            z: o.z,
        }
    }

    pub(crate) fn monster_object_view(&self, i: usize) -> crate::pathfinding::ObjectView {
        let m = &self.monsters[i];
        let def = &self.monster_definitions[m.definition];
        let o = &m.object;
        let (radius, low, high) = o.bounds(self.headers(o.collection)).unwrap_or((0, 0, 0));
        let mut flags = o.flags;
        if def.flags & 0x4000 != 0 {
            if matches!(m.mode, 4 | 5) {
                flags &= !0x400;
            } else {
                flags |= 0x400;
            }
        }
        if o.no_shape {
            flags |= 0x200;
        }
        if m.mirrored {
            flags |= 0x20000;
        }
        crate::pathfinding::ObjectView {
            index: o.index as i16,
            identifier: 0,
            flags,
            position: m.position,
            facing: m.facing,
            radius,
            bounds_low: low,
            bounds_high: high,
            owner: i as i16,
            kind: 2,
            z: o.z,
        }
    }

    fn copy_view(&self, r: &FadeRecord) -> crate::pathfinding::ObjectView {
        let o = &r.object;
        let (radius, low, high) = o.bounds(self.headers(o.collection)).unwrap_or((0, 0, 0));
        crate::pathfinding::ObjectView {
            index: o.index as i16,
            identifier: o.identifier as u16,
            flags: o.flags,
            position: r.position,
            facing: r.facing,
            radius,
            bounds_low: low,
            bounds_high: high,
            owner: o.owner,
            kind: o.kind,
            z: o.z,
        }
    }

    const CELL_LINK_STEP: u64 = 1 << 16;

    pub fn next_cell_link(&mut self) -> u64 {
        self.cell_links += Self::CELL_LINK_STEP;
        self.cell_links
    }

    pub(crate) fn copy_cell_link(&mut self, source: u64) -> u64 {
        self.copy_links += 1;
        source.saturating_sub(Self::CELL_LINK_STEP) + self.copy_links % Self::CELL_LINK_STEP
    }

    pub(crate) fn activate_monster_index(&mut self) {
        let (w, h) = if self.cells.width > 0 && self.cells.height > 0 {
            (self.cells.width, self.cells.height)
        } else {
            self.mesh_cells
        };
        self.monster_index
            .build(w, h, &self.monsters, self.monster_definitions.len());
    }

    pub(crate) fn deactivate_monster_index(&mut self) {
        self.monster_index.active = false;
    }

    pub(crate) fn invalidate_monster_index(&mut self) {
        self.monster_index.active = false;
    }

    pub(crate) fn unlink_monster_from_index(&mut self, i: usize) {
        if self.monster_index.active {
            self.monster_index.unlink(i);
        }
    }

    pub(crate) fn refresh_def_impassable(&mut self) {
        if self.monster_index.active {
            self.monster_index
                .refresh_def_impassable(&self.monsters, self.monster_definitions.len());
        }
    }

    #[inline]
    pub(crate) fn monster_index_by_object(&self, index: i32) -> Option<usize> {
        if self.monster_index.active {
            let hit = self.monster_index.monster_by_object(index);
            #[cfg(feature = "identity-checks")]
            assert_eq!(
                hit,
                self.monsters.iter().position(|m| m.object.index == index)
            );
            hit
        } else {
            self.monsters.iter().position(|m| m.object.index == index)
        }
    }

    fn solid_projectile_object_by_index(
        &self,
        index: i16,
    ) -> Option<(u64, crate::pathfinding::ObjectView)> {
        let idx = i32::from(index);
        let view = |o: &EngineObject, position: [i32; 3], facing: u16, owner: i16, kind: u8| {
            let (radius, low, high) = o.bounds(self.headers(o.collection)).unwrap_or((0, 0, 0));
            crate::pathfinding::ObjectView {
                index: o.index as i16,
                identifier: o.identifier as u16,
                flags: o.flags,
                position: (position[0], position[1]),
                facing,
                radius,
                bounds_low: low,
                bounds_high: high,
                owner,
                kind,
                z: position[2],
            }
        };
        let solid = |o: &EngineObject| !o.no_shape && o.flags & 0x600 == 0x400;
        for p in &self.projectile_records {
            if let Some(o) = p.shape.as_ref()
                && solid(o)
                && o.index == idx
            {
                let at = [p.object.x, p.object.y, p.object.z];
                return Some((p.cell_link, view(o, at, p.facing, p.index as i16, 3)));
            }
        }
        for d in &self.dormant_objects {
            let o = &d.object;
            if solid(o) && o.index == idx {
                return Some((d.cell_link, view(o, d.position, d.facing, o.owner, o.kind)));
            }
        }
        self.solid_elsewhere
            .iter()
            .copied()
            .find(|(_, o)| o.index == index)
    }

    pub fn move_monster(&mut self, i: usize, position: (i32, i32)) {
        let old = self.monsters[i].position;
        let new_cell = (position.0 >> 9, position.1 >> 9);
        if (old.0 >> 9, old.1 >> 9) != new_cell {
            self.monsters[i].cell_link = self.next_cell_link();
            if self.monster_index.active && self.monsters[i].identifier != 0 {
                self.monster_index.move_monster(i, new_cell.0, new_cell.1);
            }
        }
        self.monsters[i].position = position;
    }

    pub fn monsters_in_cell(&self, cx: i32, cy: i32) -> Vec<usize> {
        if self.monster_index.active {
            let v: Vec<usize> = self
                .monster_index
                .cell_iter(cx, cy, &self.monsters)
                .collect();
            #[cfg(feature = "identity-checks")]
            {
                let mut expected: Vec<usize> = (0..self.monsters.len())
                    .filter(|&i| {
                        let p = self.monsters[i].position;
                        self.monsters[i].identifier != 0 && (p.0 >> 9, p.1 >> 9) == (cx, cy)
                    })
                    .collect();
                expected.sort_by_key(|&i| std::cmp::Reverse(self.monsters[i].cell_link));
                assert_eq!(v, expected);
            }
            return v;
        }
        let mut v: Vec<usize> = (0..self.monsters.len())
            .filter(|&i| {
                let p = self.monsters[i].position;
                self.monsters[i].identifier != 0 && (p.0 >> 9, p.1 >> 9) == (cx, cy)
            })
            .collect();
        v.sort_by_key(|&i| std::cmp::Reverse(self.monsters[i].cell_link));
        v
    }

    pub fn monsters_in_cell_box(&self, x0: i32, y0: i32, cols: i32, rows: i32) -> Vec<usize> {
        if self.monster_index.active {
            let mut out = Vec::new();
            for row in 0..rows.max(0) {
                for col in 0..cols.max(0) {
                    out.extend(
                        self.monster_index
                            .cell_iter(x0 + col, y0 + row, &self.monsters),
                    );
                }
            }
            #[cfg(feature = "identity-checks")]
            {
                let mut v: Vec<(i32, i32, std::cmp::Reverse<u64>, usize)> = self
                    .monsters
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.identifier != 0)
                    .filter_map(|(i, m)| {
                        let (row, col) = ((m.position.1 >> 9) - y0, (m.position.0 >> 9) - x0);
                        ((0..rows).contains(&row) && (0..cols).contains(&col)).then_some((
                            row,
                            col,
                            std::cmp::Reverse(m.cell_link),
                            i,
                        ))
                    })
                    .collect();
                v.sort_unstable();
                let expected: Vec<usize> = v.into_iter().map(|e| e.3).collect();
                assert_eq!(out, expected);
            }
            return out;
        }
        let mut v: Vec<(i32, i32, std::cmp::Reverse<u64>, usize)> = self
            .monsters
            .iter()
            .enumerate()
            .filter(|(_, m)| m.identifier != 0)
            .filter_map(|(i, m)| {
                let (row, col) = ((m.position.1 >> 9) - y0, (m.position.0 >> 9) - x0);
                ((0..rows).contains(&row) && (0..cols).contains(&col)).then_some((
                    row,
                    col,
                    std::cmp::Reverse(m.cell_link),
                    i,
                ))
            })
            .collect();
        v.sort_unstable();
        v.into_iter().map(|e| e.3).collect()
    }

    pub(crate) fn with_objects_in_2x2<R>(
        &self,
        x0: i32,
        y0: i32,
        f: impl FnOnce(&[Vec<crate::pathfinding::ObjectView>; 4]) -> R,
    ) -> R {
        #[derive(Default)]
        struct Box2x2Scratch {
            keyed: Vec<(usize, std::cmp::Reverse<u64>, u8, usize)>,
            moving: Vec<Option<crate::pathfinding::ObjectView>>,
            cells: [Vec<crate::pathfinding::ObjectView>; 4],
        }
        thread_local! {
            static SCRATCH: std::cell::RefCell<Box2x2Scratch> =
                std::cell::RefCell::new(Box2x2Scratch::default());
        }
        SCRATCH.with(|cell| {
            let mut s = cell.borrow_mut();
            let Box2x2Scratch {
                keyed,
                moving,
                cells,
            } = &mut *s;
            keyed.clear();
            moving.clear();
            for c in cells.iter_mut() {
                c.clear();
            }
            self.fill_objects_in_box(x0, y0, 2, 2, keyed, moving, cells);
            #[cfg(feature = "identity-checks")]
            assert!((0..2).all(|row| (0..2).all(|col| {
                self.cell_objects_scan(x0 + col, y0 + row) == cells[(row * 2 + col) as usize]
            })));
            f(cells)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_objects_in_box(
        &self,
        x0: i32,
        y0: i32,
        cols: i32,
        rows: i32,
        keyed: &mut Vec<(usize, std::cmp::Reverse<u64>, u8, usize)>,
        moving: &mut Vec<Option<crate::pathfinding::ObjectView>>,
        cells: &mut [Vec<crate::pathfinding::ObjectView>],
    ) {
        let place = |p: (i32, i32)| {
            let (col, row) = ((p.0 >> 9) - x0, (p.1 >> 9) - y0);
            ((0..cols).contains(&col) && (0..rows).contains(&row))
                .then_some((row * cols + col) as usize)
        };
        let index = self.scenery_index.of(&self.scenery);
        for row in 0..rows.max(0) {
            for col in 0..cols.max(0) {
                let k = (row * cols + col) as usize;
                for &i in index.cell(x0 + col, y0 + row) {
                    let sc = &self.scenery[i as usize];
                    if !sc.deleted {
                        keyed.push((k, std::cmp::Reverse(sc.cell_link), 0, i as usize));
                    }
                }
            }
        }
        if self.monster_index.active {
            for row in 0..rows.max(0) {
                for col in 0..cols.max(0) {
                    let k = (row * cols + col) as usize;
                    for i in self
                        .monster_index
                        .cell_iter(x0 + col, y0 + row, &self.monsters)
                    {
                        keyed.push((k, std::cmp::Reverse(self.monsters[i].cell_link), 1, i));
                    }
                }
            }
        } else {
            for (i, m) in self.monsters.iter().enumerate() {
                if let Some(k) = place(m.position).filter(|_| m.identifier != 0) {
                    keyed.push((k, std::cmp::Reverse(m.cell_link), 1, i));
                }
            }
        }
        for (i, r) in self.fade_records.iter().enumerate() {
            if r.host == FadeRecord::NO_HOST
                && !r.object.no_shape
                && r.object.flags & 0x400 != 0
                && let Some(k) = place(r.position)
            {
                keyed.push((k, std::cmp::Reverse(r.cell_link), 2, i));
            }
        }
        let view = |o: &EngineObject, position: [i32; 3], facing: u16, owner: i16, kind: u8| {
            let (radius, low, high) = o.bounds(self.headers(o.collection)).unwrap_or((0, 0, 0));
            crate::pathfinding::ObjectView {
                index: o.index as i16,
                identifier: o.identifier as u16,
                flags: o.flags,
                position: (position[0], position[1]),
                facing,
                radius,
                bounds_low: low,
                bounds_high: high,
                owner,
                kind,
                z: position[2],
            }
        };
        let solid = |o: &EngineObject| !o.no_shape && o.flags & 0x600 == 0x400;
        for p in &self.projectile_records {
            if let Some(o) = p.shape.as_ref()
                && solid(o)
                && let Some(k) = place((p.object.x, p.object.y))
            {
                let at = [p.object.x, p.object.y, p.object.z];
                keyed.push((k, std::cmp::Reverse(p.cell_link), 3, moving.len()));
                moving.push(Some(view(o, at, p.facing, p.index as i16, 3)));
            }
        }
        for d in &self.dormant_objects {
            let o = &d.object;
            if solid(o)
                && let Some(k) = place((d.position[0], d.position[1]))
            {
                keyed.push((k, std::cmp::Reverse(d.cell_link), 3, moving.len()));
                moving.push(Some(view(o, d.position, d.facing, o.owner, o.kind)));
            }
        }
        for &(link, o) in &self.solid_elsewhere {
            if let Some(k) = place(o.position) {
                keyed.push((k, std::cmp::Reverse(link), 3, moving.len()));
                moving.push(Some(o));
            }
        }
        keyed.sort_unstable();
        for &(k, _, kind, i) in keyed.iter() {
            cells[k].push(match kind {
                0 => self.scenery_view(i),
                1 => self.monster_object_view(i),
                2 => self.copy_view(&self.fade_records[i]),
                _ => moving[i].take().expect("each once"),
            });
        }
    }

    pub(crate) fn objects_in_box(
        &self,
        x0: i32,
        y0: i32,
        cols: i32,
        rows: i32,
    ) -> Vec<Vec<crate::pathfinding::ObjectView>> {
        let mut keyed = Vec::new();
        let mut moving = Vec::new();
        let mut cells = vec![Vec::new(); (cols.max(0) * rows.max(0)) as usize];
        self.fill_objects_in_box(x0, y0, cols, rows, &mut keyed, &mut moving, &mut cells);
        #[cfg(feature = "identity-checks")]
        assert!((0..rows).all(|row| (0..cols).all(|col| {
            self.cell_objects_scan(x0 + col, y0 + row) == cells[(row * cols + col) as usize]
        })));
        cells
    }

    #[cfg(feature = "identity-checks")]
    fn cell_objects_scan(&self, x: i32, y: i32) -> Vec<crate::pathfinding::ObjectView> {
        let mut v: Vec<(u64, crate::pathfinding::ObjectView)> = Vec::new();
        for (i, sc) in self.scenery.iter().enumerate() {
            if sc.deleted || (sc.position.0 >> 9, sc.position.1 >> 9) != (x, y) {
                continue;
            }
            v.push((sc.cell_link, self.scenery_view(i)));
        }
        let mut ms: Vec<usize> = (0..self.monsters.len())
            .filter(|&i| {
                let p = self.monsters[i].position;
                self.monsters[i].identifier != 0 && (p.0 >> 9, p.1 >> 9) == (x, y)
            })
            .collect();
        ms.sort_by_key(|&i| std::cmp::Reverse(self.monsters[i].cell_link));
        for i in ms {
            v.push((self.monsters[i].cell_link, self.monster_object_view(i)));
        }
        for r in self
            .solid_copies()
            .filter(|r| (r.position.0 >> 9, r.position.1 >> 9) == (x, y))
        {
            v.push((r.cell_link, self.copy_view(r)));
        }
        v.extend(
            self.solid_projectile_objects()
                .filter(|(_, o)| (o.position.0 >> 9, o.position.1 >> 9) == (x, y)),
        );
        v.sort_by_key(|&(link, _)| std::cmp::Reverse(link));
        v.into_iter().map(|(_, o)| o).collect()
    }

    pub fn next_formation_identifier(&mut self) -> i16 {
        let c = &mut self.formation_counter.0;
        *c = if *c == 0x7fff { 0 } else { *c + 1 };
        *c
    }

    pub fn renamed_unit(&self, unit: u32) -> u32 {
        self.renamed_monsters.iter().fold(unit, |u, &(old, new)| {
            if unit_key(old) == Some(u) {
                unit_key(new).unwrap_or(u)
            } else {
                u
            }
        })
    }

    pub fn monster_index_by_identifier(&self, id: u16) -> Option<usize> {
        if id == 0 {
            return None;
        }
        self.monsters.iter().position(|m| m.identifier as u16 == id)
    }

    pub fn monster_flags_by_identifier(&self, id: u16) -> u32 {
        self.monster_index_by_identifier(id)
            .map_or(0, |i| self.monsters[i].flags)
    }

    fn monsters_in_cells(&self, me: usize, xs: (i32, i32), ys: (i32, i32)) -> Vec<usize> {
        let mut v: Vec<(i32, i32, std::cmp::Reverse<u64>, usize)> = self
            .monsters
            .iter()
            .enumerate()
            .filter(|(i, m)| *i != me && m.identifier != 0 && !m.object.no_shape)
            .map(|(i, m)| {
                let r = std::cmp::Reverse(m.cell_link);
                (m.position.1 >> 9, m.position.0 >> 9, r, i)
            })
            .filter(|&(cy, cx, _, _)| (ys.0..=ys.1).contains(&cy) && (xs.0..=xs.1).contains(&cx))
            .collect();
        v.sort();
        v.into_iter().map(|(_, _, _, i)| i).collect()
    }

    pub fn nearest_monster(
        &self,
        me: usize,
        xs: (i32, i32),
        ys: (i32, i32),
        keep: impl Fn(&EngineMonster) -> bool,
    ) -> Option<usize> {
        let (x, y) = self.monsters[me].position;
        let mut best: Option<(i32, usize)> = None;
        for i in self.monsters_in_cells(me, xs, ys) {
            let o = &self.monsters[i];
            if !keep(o) {
                continue;
            }
            let d = crate::engine_move::distance2d(x - o.position.0, y - o.position.1);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    pub fn first_monster(
        &self,
        me: usize,
        xs: (i32, i32),
        ys: (i32, i32),
        keep: impl Fn(&EngineMonster) -> bool,
    ) -> Option<usize> {
        self.monsters_in_cells(me, xs, ys)
            .into_iter()
            .find(|&i| keep(&self.monsters[i]))
    }

    pub fn order_movement(
        &mut self,
        monsters: &[usize],
        point: (i32, i32),
        flags: u16,
        action: i32,
    ) -> Result<(), Unmodelled> {
        self.order_waypoints(monsters, &[point], flags, action)
    }

    pub fn order_waypoints(
        &mut self,
        monsters: &[usize],
        points: &[(i32, i32)],
        flags: u16,
        action: i32,
    ) -> Result<(), Unmodelled> {
        self.movement_command(
            monsters,
            points,
            MovementHeader {
                flags,
                formation: 0,
                facing: 0,
            },
            action,
        )
    }

    pub fn local_team(&self) -> i16 {
        self.players
            .players
            .get(usize::try_from(self.local_player).unwrap_or(usize::MAX))
            .map_or(-i16::from(self.mode != 0), |p| i16::from(p.team))
    }

    pub fn evaluate_local_team_monster_for_selection(&self, i: usize) -> Result<u32, Unmodelled> {
        let m = &self.monsters[i];
        let def = &self.monster_definitions[m.definition];
        let local = u32::from(m.owner == self.local_player);
        let mut e = if self.players.local_without_team(self.local_player) {
            4 * local + 1
        } else {
            5 * local
        };
        if m.flags & 0x1000_0000 != 0 || self.mode == 0 {
            e &= !1;
        }
        e |= 2;
        if m.flags_8 & 0x1000 != 0 || def.field_1b6 == 5 {
            e &= if def.flags & 0x0200_0000 != 0 { !3 } else { !1 };
        }
        Ok(e)
    }

    pub fn local_movement_header(
        &self,
        selection: &[usize],
        facing: Option<u16>,
    ) -> Result<MovementHeader, Unmodelled> {
        let mut header = MovementHeader::default();
        if let Some(f) = facing {
            header.flags = 0x10;
            header.facing = f;
        }
        self.selection_formation(selection, &mut header)?;
        Ok(header)
    }

    pub fn assign_formation(
        &mut self,
        selection: &[usize],
        formation: i16,
    ) -> Result<i16, Unmodelled> {
        let team = self.local_team();
        let (mut best, mut most, mut counts) = (formation, 0u32, [0u32; 10]);
        for &i in selection {
            if self.monsters[i].team != team
                || self.evaluate_local_team_monster_for_selection(i)? & 1 == 0
            {
                continue;
            }
            let old = self.monsters[i].formation.preferred;
            if let Some(c) = usize::try_from(old).ok().and_then(|k| counts.get_mut(k)) {
                *c += 1;
                if *c > most {
                    (best, most) = (old, *c);
                }
            }
            self.monsters[i].formation.preferred = formation;
        }
        Ok(best)
    }

    pub fn selection_formation(
        &self,
        selection: &[usize],
        header: &mut MovementHeader,
    ) -> Result<(), Unmodelled> {
        let team = self.local_team();
        let (mut n, mut counts) = (0i32, [0i32; 10]);
        for &i in selection {
            if self.evaluate_local_team_monster_for_selection(i)? & 1 == 0
                || self.monsters[i].team != team
            {
                continue;
            }
            n += 1;
            if let Some(c) = usize::try_from(self.monsters[i].formation.preferred)
                .ok()
                .and_then(|k| counts.get_mut(k))
            {
                *c += 1;
            }
        }
        if let Some(k) = (0..10).rev().find(|&k| counts[k] == n) {
            header.flags |= 8;
            header.formation = k as i16;
            return Ok(());
        }
        let Some(&first) = selection.first() else {
            return Ok(());
        };
        let max = if self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3 {
            0x7e
        } else {
            0x1e
        };
        let mut taken = vec![false; selection.len()];
        let mut list = vec![first];
        let mut r = 0;
        while r < list.len() {
            let p = self.monsters[list[r]].position;
            for (c, &m) in selection.iter().enumerate() {
                if taken[c] {
                    continue;
                }
                let q = self.monsters[m].position;
                let (dx, dy) = (p.0.wrapping_sub(q.0), p.1.wrapping_sub(q.1));
                let d2 = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                if d2 <= 0x18f_ffff && list.len() < max {
                    taken[c] = true;
                    list.push(m);
                }
            }
            r += 1;
        }
        if list.len() < selection.len() {
            header.flags |= 8;
            header.formation = 0;
        }
        Ok(())
    }

    pub fn local_move_click(
        &self,
        waypoints: &mut Vec<(i32, i32)>,
        selection: &[usize],
        click: MoveClick,
        facing: Option<u16>,
        add_waypoint: bool,
    ) -> Result<Option<LocalCommand>, Unmodelled> {
        if selection.is_empty() {
            return Ok(None);
        }
        let mut header = MovementHeader::default();
        if let Some(f) = facing {
            header.flags = 0x10;
            header.facing = f;
        }
        let point = match click {
            MoveClick::LocalSprite => {
                if facing.is_some() {
                    waypoints.clear();
                    return Ok(Some((header, Vec::new())));
                }
                if waypoints.is_empty() {
                    return Ok(None);
                }
                header.flags = 0x41;
                return Ok(Some((header, std::mem::take(waypoints))));
            }
            MoveClick::Ground(x, y) => (x, y),
        };
        let near = waypoints.iter().position(|w| {
            crate::engine_move::distance2d(w.0.wrapping_sub(point.0), w.1.wrapping_sub(point.1))
                <= 0x1ff
        });
        let send = match near {
            Some(k) => {
                header.flags |= if k != 0 { 3 } else { 1 };
                true
            }
            None => {
                if waypoints.len() <= 3 {
                    waypoints.push(point);
                }
                !add_waypoint
            }
        };
        self.selection_formation(selection, &mut header)?;
        if !send {
            return Ok(None);
        }
        Ok(Some((header, std::mem::take(waypoints))))
    }

    #[allow(clippy::type_complexity)]
    pub fn pair_local_movement(
        &self,
        selection: &[usize],
        points: &[(i32, i32)],
        header: MovementHeader,
    ) -> Result<Option<(Vec<usize>, (i32, i32))>, Unmodelled> {
        let list: Vec<usize> = selection.iter().take(0x7e).copied().collect();
        let centre = self.centerpoint(&list);
        let places: Vec<Option<usize>> = list.iter().map(|&m| Some(m)).collect();
        let (dest, _) =
            self.destinations_about(&places, points, header, centre, self.local_player)?;
        let from: Vec<(i32, i32)> = list.iter().map(|&m| self.monsters[m].position).collect();
        Ok(
            crate::monster_matching::match_with_destinations(&from, &dest[0])
                .map(|order| (order.iter().map(|&k| list[k]).collect(), centre)),
        )
    }

    pub fn pair_local_targets(&self, attackers: &[usize], targets: &[usize]) -> Option<Vec<usize>> {
        let (a, t): (Vec<usize>, Vec<usize>) = (
            attackers.iter().take(0x7e).copied().collect(),
            targets.iter().take(0x7e).copied().collect(),
        );
        let from: Vec<(i32, i32)> = a.iter().map(|&m| self.monsters[m].position).collect();
        let to: Vec<(i32, i32)> = t.iter().map(|&m| self.monsters[m].position).collect();
        crate::monster_matching::match_with_targets(&from, &to)
            .map(|pairs| pairs.iter().map(|&k| t[k]).collect())
    }

    pub fn monsters_are_allied(&self, a: usize, b: usize) -> bool {
        let (ma, mb) = (&self.monsters[a], &self.monsters[b]);
        let defs = &self.monster_definitions;
        monsters_are_allied(
            &self.alliance_view(),
            (defs[ma.definition].field_1b6, ma.flags_8, ma.team),
            (defs[mb.definition].field_1b6, mb.flags_8, mb.team),
        )
    }

    pub fn teams_are_allied(&self, a: i16, b: i16) -> bool {
        self.alliances
            .teams_are_allied(a, b, self.players.teams.len())
    }

    pub fn alliance_view(&self) -> crate::ally::AllianceView {
        crate::ally::AllianceView {
            alliances: self.alliances,
            teams: self.players.teams.len(),
        }
    }

    pub fn ally_with_team(&mut self, a: i16, b: i16, allied: bool) {
        let teams = self.players.teams.len();
        let Some(c) = self
            .alliances
            .ally_with_team(a, b, allied, teams, self.game_options)
        else {
            return;
        };
        if a == self.local_team() {
            self.game.game_sound_requests.push(if allied {
                crate::ally::GAME_SOUND_ALLIED
            } else {
                crate::ally::GAME_SOUND_ALLIANCE_BROKEN
            });
        }
        self.alliance_changes.push(c);
    }

    pub fn break_all_alliances(&mut self) {
        if self.alliances.break_all(self.players.teams.len())
            && self.game_options & mandrake_common::game_options::ALLIANCES != 0
        {
            self.game
                .game_sound_requests
                .push(crate::ally::GAME_SOUND_ALLIANCE_BROKEN);
        }
    }

    pub fn generate_point_of_retreat(
        &self,
        m: usize,
        from: Option<(i32, i32)>,
    ) -> Result<(bool, (i32, i32), u16), Unmodelled> {
        let sources: Vec<RetreatSource> = self
            .monsters
            .iter()
            .enumerate()
            .filter(|(_, o)| !o.object.no_shape && o.object.flags & 0x400 != 0)
            .map(|(o, other)| RetreatSource {
                position: other.position,
                allied: self.monsters_are_allied(m, o),
            })
            .collect();
        point_of_retreat(self.monsters[m].position, from, &sources, self.mesh_cells)
    }

    pub fn retreat_command(&mut self, monsters: &[usize], player: i16) -> Result<(), Unmodelled> {
        for &m in monsters {
            if self.monsters[m].flags_8 & 0x4100 != 0 {
                continue;
            }
            let (found, point, facing) = self.generate_point_of_retreat(m, None)?;
            if found {
                self.pending_orders.push(MovementOrder {
                    monster: m,
                    waypoints: vec![point],
                    flags: 0,
                    facing,
                    action: player,
                });
            }
        }
        Ok(())
    }

    pub fn general_command(
        &mut self,
        monsters: &[usize],
        kind: u16,
        player: i16,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        self.apply_pending_orders()?;
        let local = self.local_player;
        self.execute_general_command(monsters, kind, i32::from(player), local, rng, time)
    }

    pub fn execute_general_command(
        &mut self,
        monsters: &[usize],
        kind: u16,
        source: i32,
        player: i16,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        let monsters: Vec<usize> = monsters.iter().take(0x7e).copied().collect();
        let kept = self.filter_command_monsters(&monsters, source, player);
        let source = source as i16;
        match kind {
            0 => self.command_flags(&kept, true, false, false),
            1 => self.scatter_command(&kept, source),
            2 => self.retreat_command(&kept, source),
            3 => self.command_flags(&kept, false, false, true),
            4 => self.command_flags(&kept, false, true, false),
            5 => kept
                .iter()
                .try_for_each(|&m| self.make_monster_taunt(m, rng, time)),
            6 => kept
                .iter()
                .try_for_each(|&m| self.make_monster_celebrate(m, rng, time)),
            _ => Ok(()),
        }
    }

    pub fn set_monster_magical_invisibility(&mut self, i: usize, on: bool) {
        let local_team = self.local_team();
        let m = &mut self.monsters[i];
        if on {
            m.object.flags = (m.object.flags & 0xcfff_ffff) | 0x20_0000;
            m.flags_8 |= 0x1c;
            m.object.flags |= if m.team == local_team {
                0x1000_0000
            } else {
                0x2000_0000
            };
        } else {
            m.object.flags &= 0xcfdf_ffff;
            m.flags_8 &= 0xffff_ffe3;
        }
    }

    pub fn make_monster_taunt(
        &mut self,
        i: usize,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        let m = &self.monsters[i];
        if self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5
            && (m.identifier == 0 || m.flags & 0x4000_0000 != 0)
        {
            return Ok(());
        }
        let id = m.identifier;
        self.combat_mut(i).pending = -1;
        if !(11..=14).contains(&self.monsters[i].mode) {
            let r = u32::from(rng.draw(draw_site::PANIC));
            let state = 3 + ((r * 3) >> 16) as i8;
            let Some(mut mv) = self.movers.get_mut(i).and_then(Option::take) else {
                return Err(Unmodelled::new(
                    draw_site::PANIC_NO_MOVER,
                    format!("monster {id} taunts without a movement model"),
                ));
            };
            let mut actions = self.map_actions.take();
            let mut host = crate::engine_attack::NoHost;
            let mut hooks = WorldHooks {
                world: self,
                actions: &mut actions,
                rng,
                monster: i,
                time,
                host: &mut host,
            };
            let done = mv.set_mode(MoveMode::Taunting, ModeState::Value(state), &mut hooks);
            self.map_actions = actions;
            self.movers[i] = Some(mv);
            done?;
        }
        let m = &mut self.monsters[i];
        if m.flags_8 & 0x4_0000 != 0 && self.game.winner == -1 {
            m.flags_8 &= !0x4_0000;
        }
        Ok(())
    }

    pub fn make_monster_celebrate(
        &mut self,
        i: usize,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        if (11..=14).contains(&self.monsters[i].mode) {
            return Ok(());
        }
        let id = self.monsters[i].identifier;
        let r = rng.draw(draw_site::CELEBRATE);
        let state = celebration_runs(r);
        let Some(mut mv) = self.movers.get_mut(i).and_then(Option::take) else {
            return Err(Unmodelled::new(
                draw_site::CELEBRATE_NO_MOVER,
                format!("monster {id} celebrates without a movement model"),
            ));
        };
        let mut actions = self.map_actions.take();
        let mut host = crate::engine_attack::NoHost;
        let mut hooks = WorldHooks {
            world: self,
            actions: &mut actions,
            rng,
            monster: i,
            time,
            host: &mut host,
        };
        let done = mv.set_mode(
            MoveMode::Celebrating,
            ModeState::Value(state as _),
            &mut hooks,
        );
        self.map_actions = actions;
        self.movers[i] = Some(mv);
        done
    }

    pub fn command_flags(
        &mut self,
        monsters: &[usize],
        a: bool,
        b: bool,
        c: bool,
    ) -> Result<(), Unmodelled> {
        let version = self.formation_globals.gameplay_version;
        for &m in monsters {
            let (mode, state) = (self.monsters[m].mode, self.monsters[m].anim_state as i8);
            if mode == 8 && state != -1 {
                let def = &self.monster_definitions[self.monsters[m].definition].ai;
                if def.attack(state).is_some_and(|a| a.flags_3c & 4 != 0) {
                    self.monsters[m].mode = 6;
                    if let Some(Some(mv)) = self.movers.get_mut(m) {
                        mv.mode = MoveMode::Holding;
                    }
                }
            }
            if version > game_options::GAMEPLAY_1_3 {
                self.monsters[m].flags_8 &= !0x40000;
                self.combat_mut(m).pending = -1;
            }
            let class = self.monster_definitions[self.monsters[m].definition].field_1b6;
            let mon = &mut self.monsters[m];
            let mut f = if a {
                mon.flags | 0x0040_0040
            } else {
                (mon.flags & !0x0040_0000) | 0x40
            };
            if class == 0 || version > game_options::GAMEPLAY_1_7_2 {
                f = if b { f | 0x8000_0000 } else { f & !0x8000_0000 };
            }
            mon.flags = if c { f | 0x0080_0000 } else { f & !0x0080_0000 };
        }
        Ok(())
    }

    pub fn scatter_command(&mut self, monsters: &[usize], player: i16) -> Result<(), Unmodelled> {
        let c = self.centerpoint(monsters);
        let t = crate::engine_move::trig_tables();
        let (cw, ch) = self.mesh_cells;
        for &m in monsters {
            let (x, y) = self.monsters[m].position;
            let angle = crate::engine_move::arctangent(x.wrapping_sub(c.0), y.wrapping_sub(c.1));
            let k = (angle.wrapping_add(0x20) >> 6) as usize;
            let px = x + (i32::from(t.cosine[k]) >> 3);
            let py = y + (i32::from(t.sine[k]) >> 3);
            let clamp = |v: i32, cells: i32| if v < 0 { 0 } else { v.min((cells - 1) << 9) };
            self.pending_orders.push(MovementOrder {
                monster: m,
                waypoints: vec![(clamp(px, cw), clamp(py, ch))],
                flags: 0,
                facing: angle,
                action: player,
            });
        }
        Ok(())
    }

    pub fn centerpoint(&self, monsters: &[usize]) -> (i32, i32) {
        if monsters.is_empty() {
            return (-1, -1);
        }
        let n = monsters.len() as i32;
        let (sx, sy) = monsters.iter().fold((0i32, 0i32), |(x, y), &m| {
            let p = self.monsters[m].position;
            (x.wrapping_add(p.0), y.wrapping_add(p.1))
        });
        (sx / n, sy / n)
    }

    pub fn movement_command(
        &mut self,
        monsters: &[usize],
        points: &[(i32, i32)],
        header: MovementHeader,
        action: i32,
    ) -> Result<(), Unmodelled> {
        let player = self.local_player;
        self.execute_movement_command(monsters, points, header, action, player)
    }

    pub fn execute_movement_command(
        &mut self,
        monsters: &[usize],
        points: &[(i32, i32)],
        header: MovementHeader,
        source: i32,
        player: i16,
    ) -> Result<(), Unmodelled> {
        let monsters: Vec<usize> = monsters.iter().take(0x7e).copied().collect();
        let monsters = self.filter_command_monsters(&monsters, source, player);
        let mut points: Vec<(i32, i32)> = points.iter().take(4).copied().collect();
        if header.flags & 0x40 != 0 {
            let c = self.centerpoint(&monsters);
            if points.len() == 4 {
                points[3] = c;
            } else {
                points.push(c);
            }
        }
        let c = self.centerpoint(&monsters);
        let places: Vec<Option<usize>> = monsters.iter().map(|&m| Some(m)).collect();
        let (dest, facings) = self.destinations_about(&places, &points, header, c, player)?;
        let n = monsters.len();
        let mut by_slot: Vec<Option<usize>> = monsters.iter().map(|&m| Some(m)).collect();
        if header.flags & 8 != 0 && n > 0 {
            let first = self.monsters[monsters[0]].formation.identifier;
            let mut turn = 0i32;
            let mut keep = true;
            for (k, &m) in monsters.iter().enumerate() {
                let f = self.monsters[m].formation;
                turn += angular_difference(f.facing, facings[k]) / n as i32;
                if f.identifier != first
                    || f.formation != header.formation
                    || f.slot < 0
                    || n as i32 <= i32::from(f.slot)
                    || turn > 0x4000
                {
                    keep = false;
                    break;
                }
            }
            by_slot = vec![None; n];
            if keep {
                for &m in &monsters {
                    by_slot[self.monsters[m].formation.slot as usize] = Some(m);
                }
            } else {
                let def = self.player_formation(player);
                let line = crate::formations::formation_is_line(&def, header.formation, n as i32);
                let identifier = self.next_formation_identifier();
                let c = self.centerpoint(&monsters);
                let keys: Vec<i32> = (0..n)
                    .map(|k| {
                        let (dx, dy) = (
                            dest[0][k].0.wrapping_sub(c.0),
                            dest[0][k].1.wrapping_sub(c.1),
                        );
                        let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                        if line { d.wrapping_neg() } else { d }
                    })
                    .collect();
                let mut order: Vec<usize> = (0..n).collect();
                crate::formations::heap_sort(&mut order, |&a, &b| keys[b].wrapping_sub(keys[a]));
                let mut taken = vec![false; n];
                for &slot in &order {
                    let (sx, sy) = dest[0][slot];
                    let mut best: Option<(i32, usize)> = None;
                    for (j, &m) in monsters.iter().enumerate() {
                        if taken[j] {
                            continue;
                        }
                        let (mx, my) = self.monsters[m].position;
                        let (dx, dy) = (sx.wrapping_sub(mx), sy.wrapping_sub(my));
                        let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                        if best.is_none_or(|(b, _)| b > d) {
                            best = Some((d, j));
                        }
                    }
                    if let Some((_, j)) = best {
                        taken[j] = true;
                        by_slot[slot] = Some(monsters[j]);
                        self.monsters[monsters[j]].formation.slot = slot as i16;
                    }
                }
                for (k, &m) in monsters.iter().enumerate() {
                    let f = &mut self.monsters[m].formation;
                    f.identifier = identifier;
                    f.facing = facings[k];
                    f.formation = header.formation;
                }
            }
        }
        for (k, m) in by_slot.into_iter().enumerate() {
            let Some(m) = m else { continue };
            self.pending_orders.push(MovementOrder {
                monster: m,
                waypoints: dest.iter().take(points.len()).map(|d| d[k]).collect(),
                flags: header.flags,
                facing: facings[k],
                action: source as i16,
            });
        }
        Ok(())
    }

    pub fn execute_matched_movement_command(
        &mut self,
        list: &[Option<usize>],
        points: &[(i32, i32)],
        header: MovementHeader,
        centre: (i32, i32),
        source: i32,
        player: i16,
    ) -> Result<(), Unmodelled> {
        let list: Vec<Option<usize>> = list.iter().take(0x7e).copied().collect();
        let list = self.filter_command_list(&list, source, player);
        let points: Vec<(i32, i32)> = points.iter().take(4).copied().collect();
        let (dest, facings) = self.destinations_about(&list, &points, header, centre, player)?;
        for (k, m) in list.iter().enumerate() {
            let Some(m) = *m else { continue };
            self.pending_orders.push(MovementOrder {
                monster: m,
                waypoints: dest.iter().take(points.len()).map(|d| d[k]).collect(),
                flags: header.flags,
                facing: facings[k],
                action: source as i16,
            });
        }
        Ok(())
    }

    pub fn set_formation_definition_for_player(
        &mut self,
        player: i16,
        definition: crate::formations::FormationDefinition,
    ) {
        match self
            .player_formations
            .iter_mut()
            .find(|(p, _)| *p == player)
        {
            Some(r) => r.1 = definition,
            None => self.player_formations.push((player, definition)),
        }
    }

    fn player_formation(&self, player: i16) -> crate::formations::FormationDefinition {
        self.player_formations
            .iter()
            .find(|(p, _)| *p == player && player != -1)
            .map_or(self.formation_definition, |(_, d)| *d)
    }

    pub fn attack_location_command(
        &mut self,
        monsters: &[usize],
        flags: u16,
        point: (i32, i32, i32),
        player: i16,
    ) -> Result<(), Unmodelled> {
        self.execute_attack_location(monsters, flags, point, -1, player)
    }

    pub fn handle_attack_location_command(
        &mut self,
        source: i32,
        monsters: &[usize],
        flags: u16,
        point: (i32, i32, i32),
    ) -> Result<(), Unmodelled> {
        if source == -1 {
            return Err(Unmodelled::new(
                draw_site::ATTACK_LOCATION_COMMAND_QUEUE,
                "handle_attack_location_command: queue_outgoing_command",
            ));
        }
        let player = self.local_player;
        self.execute_attack_location(monsters, flags, point, source, player)
    }

    fn execute_attack_location(
        &mut self,
        monsters: &[usize],
        flags: u16,
        point: (i32, i32, i32),
        source: i32,
        player: i16,
    ) -> Result<(), Unmodelled> {
        let version = self.formation_globals.gameplay_version;
        let max = if version > game_options::GAMEPLAY_1_3 {
            0x7e
        } else {
            0x1e
        };
        let monsters: Vec<usize> = monsters.iter().take(max).copied().collect();
        let monsters = self.filter_command_monsters(&monsters, source, player);
        for m in monsters {
            self.monsters[m].flags_8 &= !0x4_0000;
            self.drop_waypoints(m)?;
            self.combat_mut(m).pending = -1;
            if matches!(self.monsters[m].mode, 7 | 8) {
                continue;
            }
            self.set_target_location(m, flags, point);
        }
        Ok(())
    }

    pub(crate) fn set_target_location(&mut self, m: usize, flags: u16, point: (i32, i32, i32)) {
        let version = self.formation_globals.gameplay_version;
        let f = self.monsters[m].flags;
        if f & 0x11 == 0x11
            && self.combat(m).location == (point.0, point.1)
            && (f & 0x200 != 0) == (flags & 2 != 0)
            && (f & 0x80_0000 != 0) == (flags & 1 != 0)
        {
            return;
        }
        let c = self.combat_mut(m);
        c.location = (point.0, point.1);
        c.location_z = point.2 as i16;
        c.target = -1;
        let mon = &mut self.monsters[m];
        mon.object.flags &= !crate::collide::OBJECT_POSED;
        mon.flags = (f & 0x7f06_bf77) | if flags & 1 != 0 { 0x80_8037 } else { 0x8037 };
        let mut f8 = mon.flags_8;
        if flags & 2 != 0 {
            f8 |= 0x200;
        } else {
            f8 &= !0x200;
        }
        if version > game_options::GAMEPLAY_1_7_1 && flags & 4 != 0 {
            f8 |= 0x800_0000;
        } else {
            f8 &= !0x800_0000;
        }
        mon.flags_8 = f8 & 0xffdf_fbff;
    }

    pub fn make_monster_invisible(
        &mut self,
        m: usize,
        rng: &mut EngineRng,
        time: i32,
    ) -> Result<(), Unmodelled> {
        self.turn_off_monster_local_group(m);
        let mon = &mut self.monsters[m];
        mon.flags = (mon.flags & !0x19) | 0x4000_0000;
        mon.object.no_shape = true;
        let c = self.combat_mut(m);
        c.target = -1;
        c.attacker = -1;
        c.pending = -1;
        c.held_unit = -1;
        let mut hooks = WorldHooks {
            world: self,
            actions: &mut None,
            rng,
            monster: m,
            time,
            host: &mut crate::engine_attack::NoHost,
        };
        hooks.call_monster_died_procs(0)
    }

    pub(crate) fn turn_off_monster_local_group(&mut self, m: usize) {
        if self.combat_mut(m).local_group.take().is_some() {
            self.group_events
                .push(crate::CombatEvent::MonsterLocalGroupOff {
                    tick: 0,
                    monster: m as u32,
                });
        }
    }

    pub fn set_monster_uncontrollable(&mut self, m: usize, on: bool) -> Result<(), Unmodelled> {
        let mon = &mut self.monsters[m];
        let was = mon.flags & 0x1000_0000 != 0;
        if was == on {
            return Ok(());
        }
        if !on {
            mon.flags &= !0x1000_0000;
            self.control_regained_statistics(m);
            return Ok(());
        }
        mon.flags |= 0x1000_0000;
        mon.flags_8 |= 0x2000_0000;
        self.control_lost_statistics(m);
        Ok(())
    }

    pub(crate) fn monster_player_control_changed(&mut self, m: usize) {
        if self.monsters[m].flags & 0x1000_0000 == 0 {
            self.control_regained_statistics(m);
            return;
        }
        if self.formation_globals.gameplay_version > game_options::GAMEPLAY_1_7_2 {
            self.monsters[m].flags_8 |= 0x2000_0000;
        }
        self.control_lost_statistics(m);
    }

    #[allow(clippy::type_complexity)]
    pub fn movement_command_destinations(
        &self,
        monsters: &[usize],
        points: &[(i32, i32)],
        header: MovementHeader,
    ) -> Result<(Vec<Vec<(i32, i32)>>, Vec<u16>), Unmodelled> {
        let c = self.centerpoint(monsters);
        let list: Vec<Option<usize>> = monsters.iter().map(|&m| Some(m)).collect();
        self.destinations_about(&list, points, header, c, self.local_player)
    }

    #[allow(clippy::type_complexity)]
    pub fn formation_destinations(
        &self,
        places: usize,
        points: &[(i32, i32)],
        header: MovementHeader,
        centre: (i32, i32),
    ) -> Result<(Vec<Vec<(i32, i32)>>, Vec<u16>), Unmodelled> {
        self.destinations_about(
            &vec![None; places],
            points,
            header,
            centre,
            self.local_player,
        )
    }

    #[allow(clippy::type_complexity)]
    fn destinations_about(
        &self,
        monsters: &[Option<usize>],
        points: &[(i32, i32)],
        header: MovementHeader,
        c: (i32, i32),
        player: i16,
    ) -> Result<(Vec<Vec<(i32, i32)>>, Vec<u16>), Unmodelled> {
        let n = monsters.len();
        let mut points = points.to_vec();
        if points.is_empty() {
            points.push(c);
        } else if header.flags & 0x40 != 0 {
            if points.len() == 4 {
                points[3] = c;
            } else {
                points.push(c);
            }
        }
        let last = *points.last().expect("a point");
        let facing = if header.flags & 0x10 != 0 {
            header.facing
        } else {
            crate::engine_move::arctangent(last.0.wrapping_sub(c.0), last.1.wrapping_sub(c.1))
        };
        let mut facings = vec![facing; n];
        let mut dest: Vec<Vec<(i32, i32)>> = Vec::with_capacity(points.len());
        if header.flags & 8 == 0 {
            for p in &points {
                dest.push(
                    monsters
                        .iter()
                        .map(|&m| {
                            let (x, y) = m.map_or(c, |m| self.monsters[m].position);
                            (
                                x.wrapping_sub(c.0).wrapping_add(p.0),
                                y.wrapping_sub(c.1).wrapping_add(p.1),
                            )
                        })
                        .collect(),
                );
            }
        } else {
            let k = points.len();
            for w in 0..k {
                let f = if k == 1 {
                    facing
                } else if header.flags & 1 != 0 {
                    let (a, b) = if w == 0 { (k - 1, 0) } else { (w, w - 1) };
                    crate::engine_move::arctangent(
                        points[a].0.wrapping_sub(points[b].0),
                        points[a].1.wrapping_sub(points[b].1),
                    )
                } else if w == k - 1 {
                    facing
                } else {
                    crate::engine_move::arctangent(
                        points[w + 1].0.wrapping_sub(points[w].0),
                        points[w + 1].1.wrapping_sub(points[w].1),
                    )
                };
                let slots = crate::formations::build_formation(
                    &self.player_formation(player),
                    self.formation_globals,
                    header.formation,
                    points[w],
                    f,
                    n as i32,
                )?;
                for (j, s) in slots.iter().enumerate() {
                    facings[j] = s.1;
                }
                dest.push(slots.into_iter().map(|s| s.0).collect());
            }
        }
        let (cw, ch) = self.mesh_cells;
        for d in &mut dest {
            for p in d.iter_mut() {
                *p = (p.0.clamp(0, (cw << 9) - 1), p.1.clamp(0, (ch << 9) - 1));
            }
        }
        Ok((dest, facings))
    }

    pub fn update_map_actions(&mut self, time: i32, rng: &mut EngineRng) -> Result<(), Unmodelled> {
        let Some(mut actions) = self.map_actions.take() else {
            return Ok(());
        };
        if time & 0x1f == 0 {
            self.activate_monster_index();
        }
        let r = actions.update(&mut MapActionContext {
            world: self,
            rng,
            time,
        });
        self.deactivate_monster_index();
        self.map_actions = Some(actions);
        r
    }

    pub(crate) fn headers(&self, c: FourCC) -> &[SequenceHeader] {
        self.collections.get(&c).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn model_index_of_cell(&self, px: i32, py: i32) -> Option<i16> {
        let (x, y) = (i32::from((px >> 9) as i16), i32::from((py >> 9) as i16));
        if x < 0 || y < 0 || x >= self.cells.width || y >= self.cells.height {
            return None;
        }
        let model = self.cells.models[(y * self.cells.width + x) as usize];
        (model != -1).then_some(model)
    }

    fn load_collection(&mut self, tags: &dyn TagSource, c: FourCC) -> Result<(), Unmodelled> {
        if let std::collections::hash_map::Entry::Vacant(e) = self.collections.entry(c) {
            let data = tags.tag(*b".256", c).ok_or_else(|| {
                Unmodelled::new(
                    draw_site::COLLECTION_MISSING,
                    format!("collection {c} missing"),
                )
            })?;
            let h = sequence_headers(&data).map_err(|e| {
                Unmodelled::new(draw_site::COLLECTION_LOAD, format!("collection {c}: {e}"))
            })?;
            e.insert(h);
            if let Ok(k) = mandrake_tag::sequence_header::sequence_key_points(&data) {
                self.key_points.insert(c, k);
            }
        }
        Ok(())
    }

    pub fn update_monsters(
        &mut self,
        rng: &mut EngineRng,
        terrain: Option<&MeshCells>,
        time: i32,
        host: &mut dyn crate::engine_attack::CombatHost,
    ) -> Result<(), (usize, Unmodelled)> {
        self.monster_draws.clear();
        if self.movers.len() < self.monsters.len() {
            self.movers.resize(self.monsters.len(), None);
        }
        self.activate_monster_index();
        if let Err(e) = self.replan_walkers(time, rng, host) {
            self.deactivate_monster_index();
            return Err(e);
        }
        let mut actions = self.map_actions.take();
        let mut r = Ok(());
        for i in 0..self.monsters.len() {
            if self.monsters[i].identifier == 0 {
                continue;
            }
            if self.monsters[i].flags & 0x4000_0000 != 0 {
                continue;
            }
            let before = rng.sites.total();
            if self.monsters[i].flags_8 & 0x40_0000 != 0
                && self.game.state != 0
                && let Err(e) = self.visible_group(i, rng)
            {
                r = Err((i, e));
                break;
            }
            let mut mover = self.movers[i].take();
            let u = self.update_monster(i, mover.as_mut(), rng, terrain, time, &mut actions, host);
            self.movers[i] = mover;
            let n = rng.sites.total() - before;
            if n > 0 {
                self.monster_draws.push((i, n));
            }
            if let Err(e) = u {
                r = Err((i, e));
                break;
            }
        }
        self.deactivate_monster_index();
        self.map_actions = actions;
        r
    }

    fn visible_group(&mut self, i: usize, rng: &mut EngineRng) -> Result<(), Unmodelled> {
        let m = &self.monsters[i];
        let def = &self.monster_definitions[m.definition];
        let group = self
            .marker_creation
            .as_ref()
            .and_then(|c| c.tag(*b"mons", def.tag))
            .and_then(|d| resolved_mons_group(&d, &self.catalog));
        if let Some(group) = group {
            let object = m.object.index;
            let desc = crate::projectile_groups::GroupDescriptor {
                flags: 0,
                group,
                shading_object: object,
                object,
                owner: i as i16,
                point: [m.position.0, m.position.1, m.object.z],
                magnitude: 0,
                velocity: [0; 3],
            };
            let mut w = crate::projectile_groups::EngineSpawn {
                world: self,
                rng,
                source: None,
                host: None,
            };
            crate::projectile_groups::new_projectile_group(&mut w, &desc)?;
        }
        self.monsters[i].flags_8 &= !0x40_0000;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn update_monster(
        &mut self,
        i: usize,
        mover: Option<&mut EngineMover>,
        rng: &mut EngineRng,
        terrain: Option<&MeshCells>,
        time: i32,
        actions: &mut Option<MapActions>,
        host: &mut dyn crate::engine_attack::CombatHost,
    ) -> Result<(), Unmodelled> {
        let m = &self.monsters[i];
        if let Some(r) = &m.unmodelled {
            return Err(Unmodelled::new(
                draw_site::MONSTER_UNMODELLED,
                format!("monster {} ({}): {r}", m.identifier, m.index),
            ));
        }
        let (def_flags, idle) = {
            let def = &self.monster_definitions[m.definition];
            (def.flags, IdleSequences::of(def))
        };
        let collection = m.object.collection;
        let Some(mv) = mover else {
            if self.pending_orders.iter().any(|o| o.monster == i) {
                return Err(Unmodelled::new(
                    draw_site::MONSTER_ORDERED_NO_MOVER,
                    format!(
                        "monster {} ({}): ordered by a map action without a movement model",
                        m.identifier, m.index
                    ),
                ));
            }
            if m.mode != 1 {
                return Err(Unmodelled::new(
                    draw_site::MONSTER_MODE_NO_MOVER,
                    format!(
                        "monster {} in mode {} without a movement model",
                        m.identifier, m.mode
                    ),
                ));
            }
            if m.flags & 0x40 != 0 {
                return Err(Unmodelled::new(
                    draw_site::MONSTER_RESET_NO_MOVER,
                    format!(
                        "monster {} ({}): mode-1 reset without a movement model",
                        m.identifier, m.index
                    ),
                ));
            }
            if m.flags_8 & 0x100 != 0 {
                return Err(Unmodelled::new(
                    draw_site::MONSTER_CONFUSED_NO_MOVER,
                    format!(
                        "monster {} ({}): confused without a movement model",
                        m.identifier, m.index
                    ),
                ));
            }
            let Self {
                monsters,
                collections,
                ..
            } = self;
            let headers = collections
                .get(&collection)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let m = &mut monsters[i];
            animate_object(&mut m.object, headers);
            idle_handler(m, idle, headers, rng)?;
            if self.monsters[i].identifier != 0 {
                self.recharge_mana(i);
            }
            return Ok(());
        };
        let Some(mode) = MoveMode::from_byte(m.mode) else {
            return Err(Unmodelled::new(
                draw_site::MONSTER_MODE_NO_MOVER,
                format!("monster {} in mode {}", m.identifier, m.mode),
            ));
        };
        if m.mode != 1 && def_flags & 0x8000 != 0 {
            return Err(Unmodelled::new(
                draw_site::MONSTER_DUPLICATE_OBJECT,
                format!(
                    "monster {}: mons flag 0x8000 (duplicate_object)",
                    m.identifier
                ),
            ));
        }
        if let Some(b) = &mv.block
            && (mv.is_moving(m.flags) || m.mode != 1)
        {
            return Err(Unmodelled::new(
                b.site,
                format!("monster {}: {}", m.identifier, b.reason),
            ));
        }
        let Self {
            monsters,
            collections,
            ..
        } = self;
        let headers = collections
            .get(&collection)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let m = &mut monsters[i];
        animate_object(&mut m.object, headers);
        mv.velocity = m.object.velocity;
        mv.mode = mode;
        mv.frame_tick = m.object.tick;
        mv.ticks_per_frame = m.object.ticks_per_frame;
        mv.frame_advanced = m.object.anim_flags & 1 != 0;
        let mut hooks = WorldHooks {
            world: self,
            actions,
            rng,
            monster: i,
            time,
            host,
        };
        if !(8..=16).contains(&mode.byte()) {
            let needs_leg = hooks.world.monsters[i].flags & crate::engine_move::NEW_LEG != 0;
            if needs_leg && hooks.world.monsters[i].flags & 0x4000 != 0 {
                hooks.pick_up_leg_start(mv)?;
            } else if needs_leg && hooks.world.monsters[i].flags & 1 != 0 {
                hooks.engaged_leg_start(mv)?;
            } else {
                mv.start_leg_if_needed(&mut hooks)?;
            }
            if hooks.world.monsters[i].flags & 0x40 != 0 {
                let old = hooks.world.monsters[i].flags;
                mv.reset_to_idle(&mut hooks)?;
                if old & 0x80_0000 != 0 {
                    hooks.general_attack(mv)?;
                }
                hooks.world.monsters[i].flags &= !0x1040;
            }
        }
        let mode = hooks.world.monsters[i].mode;
        if mode <= 16 && (1u32 << mode) & 0x1fa7e != 0 && mv.pose_timer >= 0 {
            mv.pose_timer -= 1;
        }
        if hooks.world.monsters[i].flags_8 & 0x100 != 0 {
            hooks.update_confusion(mv)?;
        }
        match mv.mode {
            MoveMode::Idle => {
                let EngineWorld {
                    monsters,
                    collections,
                    ..
                } = &mut *hooks.world;
                let headers = collections
                    .get(&collection)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                idle_handler(&mut monsters[i], idle, headers, hooks.rng)?;
            }
            MoveMode::Attacking => hooks.attack_handler(mv)?,
            MoveMode::Flinching => hooks.flinch_handler(mv)?,
            MoveMode::Blocking => hooks.block_handler(mv)?,
            MoveMode::PickingUp => hooks.pick_up_handler(mv)?,
            MoveMode::Paralyzed => hooks.paralysis_handler(mv)?,
            MoveMode::LowVitality => hooks.low_vitality_handler(mv)?,
            MoveMode::Celebrating => hooks.celebrate_handler(mv)?,
            MoveMode::Taunting => hooks.taunt_handler(mv)?,
            MoveMode::Held => hooks.held_handler(mv)?,
            MoveMode::Stunned => hooks.stunned_handler(mv)?,
            _ => {
                mv.animation_state = hooks.world.monsters[i].anim_state;
                mv.handler(terrain, &mut hooks)?;
            }
        }
        if self.monsters[i].identifier == 0 {
            return Ok(());
        }
        self.move_monster(i, (mv.x, mv.y));
        let m = &mut self.monsters[i];
        m.facing = mv.facing;
        m.object.velocity = mv.velocity;
        self.recharge_mana(i);
        Ok(())
    }

    fn recharge_mana(&mut self, i: usize) {
        let def = &self.monster_definitions[self.monsters[i].definition].ai;
        let (max, rate) = (i32::from(def.word(0x214)), i32::from(def.word(0x216)));
        if max == 0 {
            return;
        }
        let gameplay = self.formation_globals.gameplay_version;
        let flags_8 = self.monsters[i].flags_8;
        let c = self.combat_mut(i);
        let mana = i32::from(c.mana);
        let new = if gameplay <= game_options::GAMEPLAY_1_3 {
            if max <= mana {
                return;
            }
            (mana + rate).min(max)
        } else if flags_8 & 0x200_0000 != 0 {
            return;
        } else if max > mana && rate > 0 {
            (mana + rate).min(max)
        } else if mana > 0 && rate < 0 {
            (mana + rate).max(0)
        } else {
            return;
        };
        c.mana = new as i16;
    }

    pub fn update_particle_clock(&mut self, rng: &mut EngineRng) {
        if let Some(p) = self.particles.as_mut() {
            p.update_clock(rng);
        }
    }

    pub fn particle_system_intensity(&self, point: [i32; 3]) -> Result<u16, Unmodelled> {
        crate::particles::particle_system_intensity(self.particles.as_ref(), point, self.mesh_cells)
    }

    pub fn update_wind(&mut self, rng: &mut EngineRng) -> Result<(), Unmodelled> {
        let Some(w) = self.wind.as_mut() else {
            return Ok(());
        };
        w.countdown -= 1;
        if w.countdown <= 0 {
            w.change(rng, draw_site::WIND_PERIOD);
        }
        w.approach();
        Ok(())
    }
}

impl EngineWorld {
    fn lead_point(&self, t: usize, own: &crate::attack_ai::Own) -> Option<(i32, i32)> {
        let tm = &self.monsters[t];
        let v = tm.object.velocity;
        if v[0] == 0 && v[1] == 0 {
            return None;
        }
        let (tx, ty) = tm.position;
        let (dx, dy) = (
            tx.wrapping_sub(own.position.0),
            ty.wrapping_sub(own.position.1),
        );
        if dx.abs() > 0x27ff || dy.abs() > 0x27ff {
            return None;
        }
        let (w, h) = (self.cells.width, self.cells.height);
        let range = |c: i32, size: i32| -> (i32, i32) {
            let lo = if c <= 1 {
                0
            } else if c - 1 > size {
                size - 1
            } else {
                c - 2
            };
            let hi = if c < -2 {
                0
            } else if c + 1 >= size - 1 {
                size - 1
            } else {
                c + 2
            };
            (lo, hi)
        };
        let (x0, x1) = range(tx >> 9, w);
        let (y0, y1) = range(ty >> 9, h);
        let mask = u32::from(tm.impassable);
        let mut blocked = 0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let c = (y * w + x) as usize;
                if self.cells.models.get(c).copied().unwrap_or(-1) != -1 {
                    return None;
                }
                let f = u32::from(self.cells.flags.get(c).copied().unwrap_or(0));
                if ((mask >> ((f >> 4) & 0xf)) | (mask >> (f & 0xf))) & 1 != 0 {
                    blocked += 1;
                    if blocked > 3 {
                        return None;
                    }
                }
            }
        }
        let half = |c: i16| c.wrapping_sub(c / 2);
        let (vx, vy) = (i32::from(half(v[0])), i32::from(half(v[1])));
        if vx == 0 && vy == 0 {
            return None;
        }
        let (ex, ey) = (dx >> 3, dy >> 3);
        let s = i32::from(own.speed);
        let a = (vx * vx + vy * vy).wrapping_sub(s * s);
        let b = (2 * vx)
            .wrapping_mul(ex)
            .wrapping_add((2 * vy).wrapping_mul(ey));
        let disc = b.wrapping_mul(b).wrapping_sub(
            (ey.wrapping_mul(ey).wrapping_add(ex.wrapping_mul(ex))).wrapping_mul(4 * a),
        );
        if disc < 0 || a == 0 {
            return None;
        }
        let r = crate::projectiles::isqrt(disc as u32);
        let t1 = r.wrapping_sub(b) / (2 * a);
        let t2 = b.wrapping_neg().wrapping_sub(r) / (2 * a);
        let time = if t1 < 0 || (t2 >= 0 && t1 > t2) {
            t2
        } else {
            t1
        };
        if time <= 0 {
            return None;
        }
        let time = (time << 3).min(0x12c);
        let mut px = vx * time + tx;
        let mut py = vy * time + ty;
        let (mw, mh) = (self.mesh_cells.0 << 9, self.mesh_cells.1 << 9);
        px = if px < 0 {
            0
        } else if px >= mw {
            mw - 1
        } else {
            px
        };
        py = if py < 0 {
            0
        } else if py >= mh {
            mh - 1
        } else {
            py
        };
        Some((px, py))
    }
}

#[derive(Clone, Copy)]
struct IdleSequences {
    idle: i16,
    seq_1c: i16,
    seq_1e: i16,
    seq_20: i16,
}

impl IdleSequences {
    fn of(def: &MonsterDefinition) -> Self {
        Self {
            idle: def.idle_sequence,
            seq_1c: def.idle_sequence_1c,
            seq_1e: def.idle_sequence_1e,
            seq_20: def.idle_sequence_20,
        }
    }
}

fn idle_handler(
    m: &mut EngineMonster,
    idle: IdleSequences,
    headers: &[SequenceHeader],
    rng: &mut EngineRng,
) -> Result<(), Unmodelled> {
    let flags = m.object.anim_flags;
    let (sequence, speed, state) = match m.anim_state {
        0x1b if flags & 4 != 0 => (idle.idle, 0x100, 0x1a),
        0x1d if flags & 4 != 0 => (idle.seq_1e, 0x100, 0x1c),
        0x1a | 0x1c if flags & 1 != 0 => {
            let current = m.object.sequence;
            let r = rng.draw(draw_site::IDLE_CHANCE);
            if r <= 0x3fff && idle.seq_1e != -1 {
                if m.anim_state == 0x1a {
                    (idle.seq_1c, 0x100, 0x1d)
                } else {
                    (idle.seq_20, 0x100, 0x1b)
                }
            } else {
                let r2 = rng.draw(draw_site::IDLE_SEQUENCE);
                (
                    i16::from(current),
                    (i32::from(r2) >> 9) + 0x300,
                    m.anim_state,
                )
            }
        }
        _ => return Ok(()),
    };
    m.anim_state = state;
    set_object_shape(
        &mut m.object,
        headers,
        i32::from(sequence),
        speed,
        0x21,
        rng,
    )
}

impl crate::record_list::RecordIndex for FadeRecord {
    fn record_index(&self) -> i32 {
        self.index
    }
}

#[cfg(test)]
mod tests;
