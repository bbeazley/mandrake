use mandrake_common::FourCC;
use mandrake_math::{BungieRng, Fixed16, Vec3Fixed};
use mandrake_tag::sound_manager::{ChannelObject, ObjectPlace, WorldPoint};
use mandrake_tag::{
    DamageDefinition, ObjectDefinition, ProjectileDefinition, ProjectileGroupDefinition,
    TagArchive, UnitTag,
};

use crate::Simulation;
use crate::attack_ai::{AiDefinition, AttackDefinition};
use crate::draw_site;
use crate::entities::UnitState;
use crate::projectiles::FlightDefinition;
use mandrake_tag::SequenceWorldBounds;
use mandrake_tag::object::ObjectPhysics;

pub const MINIMUM_GAMEPLAY_VERSION: u32 = mandrake_common::game_options::GAMEPLAY_1_8_3 as u32;

pub const KILL_CODE_DEFAULT: u8 = 0x26;
pub const KILL_CODE_EXPLOSION: u8 = 0x27;

pub fn roll_damage(def: &DamageDefinition, rng: &mut BungieRng) -> i32 {
    let base = i32::from(def.damage_lower_bound);
    if def.damage_delta > 0 {
        base + ((i32::from(rng.next_u16()) * i32::from(def.damage_delta)) >> 16)
    } else {
        base
    }
}

pub fn scale_amount(amount: i32, scale: i32) -> i32 {
    (amount * scale) >> 8
}

pub fn damage_object_draws(damage_type: i16) -> bool {
    matches!(damage_type, 2..=7) || !(0..=11).contains(&damage_type)
}

pub fn frequency_corrected(rng: &mut BungieRng, threshold: u16, version_cut: u32) -> bool {
    let r = rng.next_u16();
    if MINIMUM_GAMEPLAY_VERSION <= version_cut {
        r <= threshold
    } else {
        r < threshold || (MINIMUM_GAMEPLAY_VERSION > 0x70d && threshold > 0xfffe)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HitResult {
    pub hp_before: i32,
    pub hp_after: i32,
    pub lethal: bool,
}

pub fn damage_object(hp: i32, max_hp: i32, amount: i32) -> HitResult {
    let new = (hp - amount).min(0x7fff);
    let lethal = new <= 0;
    let mut hp_after = new.max(0);
    if MINIMUM_GAMEPLAY_VERSION > 0x514 && max_hp < hp_after {
        hp_after = max_hp;
    }
    HitResult {
        hp_before: hp,
        hp_after,
        lethal,
    }
}

pub fn hit_ratio(amount: i32, max_hp: i32) -> u16 {
    if amount > max_hp {
        0xffff
    } else if max_hp == 0 {
        0
    } else {
        ((amount << 16) / max_hp) as u16
    }
}

pub fn kill_code(
    damage_type: i16,
    amount: i32,
    max_hp: i32,
    threshold: u16,
    monster_0b_2: bool,
    has_group_29: bool,
) -> u8 {
    if damage_type == 9 && has_group_29 {
        return 0x29;
    }
    if matches!(damage_type, 0 | 1 | 8) && !monster_0b_2 {
        return KILL_CODE_EXPLOSION;
    }
    if hit_ratio(amount, max_hp) > threshold {
        KILL_CODE_EXPLOSION
    } else {
        KILL_CODE_DEFAULT
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AttackTarget {
    Units(Vec<u32>),
    Location(Vec3Fixed),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CombatEvent {
    AttackOrdered {
        tick: u32,
        attackers: Vec<u32>,
        target: AttackTarget,
    },
    ProjectileDetonated {
        tick: u32,
        projectile_id: u32,
        definition: FourCC,
        owner: u32,
        hit_unit: Option<u32>,
        position: Vec3Fixed,
        group: Option<FourCC>,
    },
    UnitDamaged {
        tick: u32,
        unit_id: u32,
        attacker: u32,
        definition: FourCC,
        damage_type: i16,
        amount: i32,
        hp_before: i32,
        hp_after: i32,
        lethal: bool,
    },
    HitReaction {
        tick: u32,
        unit_id: u32,
    },
    ProjectileRebounded {
        tick: u32,
        projectile_id: u32,
        definition: FourCC,
        sound: Option<FourCC>,
        position: Vec3Fixed,
        world: [i32; 3],
        request_flags: u16,
        object: i32,
        identifier: i16,
        velocity: [i16; 3],
    },
    ProjectileGroupLaunched {
        tick: u32,
        group: FourCC,
        flags: u32,
        object: i32,
        point: [i32; 3],
        media_height: i16,
    },
    StainMesh {
        tick: u32,
        stain: crate::flight::Stain,
    },
    Footstep {
        tick: u32,
        point: (i32, i32),
    },
    MonsterVoice {
        tick: u32,
        sound: FourCC,
        world: [i32; 3],
        request_flags: u16,
        object: i32,
        identifier: i16,
        velocity: [i16; 3],
    },
    ProjectileGroupSound {
        tick: u32,
        sound: FourCC,
        world: [i32; 3],
        request_flags: u16,
        object: i32,
        identifier: i16,
        velocity: [i16; 3],
    },
    ProjectileGroupLocal {
        tick: u32,
        local_group: FourCC,
        point: [i32; 3],
        facing: u16,
        object_point: [i32; 3],
        cylinder: [i16; 3],
    },
    ProjectileCreated {
        tick: u32,
        definition: FourCC,
        object: i32,
        identifier: i16,
        point: [i32; 3],
        facing: u16,
        velocity: [i16; 3],
    },
    MonsterLocalGroup {
        tick: u32,
        monster: u32,
        local_group: FourCC,
        object: i32,
        identifier: i16,
    },
    MonsterLocalGroupOff {
        tick: u32,
        monster: u32,
    },
    UnitKilled {
        tick: u32,
        unit_id: u32,
        killer: u32,
        animation_code: u8,
    },
}

pub const CONFUSION_LOCAL_GROUP: FourCC = FourCC::new(*b"coef");

impl crate::engine_rng::EngineWorld {
    pub(crate) fn monster_voice_in_hit(
        &mut self,
        speaker: usize,
        victim: usize,
        victim_mover: Option<(i32, i32)>,
        kind: i8,
    ) {
        let at = victim_mover
            .filter(|_| speaker == victim)
            .or_else(|| {
                self.movers
                    .get(speaker)
                    .and_then(Option::as_ref)
                    .map(|m| (m.x, m.y))
            })
            .unwrap_or(self.monsters[speaker].position);
        self.monster_voice(speaker, kind, at);
    }

    pub(crate) fn monster_voice(&mut self, i: usize, kind: i8, at: (i32, i32)) {
        let m = &self.monsters[i];
        let raw = &self.monster_definitions[m.definition].ai.raw;
        let Some(slot) = raw
            .get(mandrake_tag::unit::MONSTER_SOUND_TYPES_OFFSET..)
            .and_then(|t| t.get(..mandrake_tag::unit::MONSTER_SOUND_SLOTS))
            .and_then(|t| t.iter().position(|&t| t as i8 == kind))
        else {
            return;
        };
        let at4 = mandrake_tag::unit::MONSTER_SOUNDS_OFFSET + 4 * slot;
        let Some(b) = raw.get(at4..at4 + 4) else {
            return;
        };
        let b: [u8; 4] = b.try_into().expect("4 bytes");
        if b == [0xff; 4] {
            return;
        }
        let event = CombatEvent::MonsterVoice {
            tick: 0,
            sound: FourCC::new(b),
            world: [at.0, at.1, m.object.z],
            request_flags: u16::from(m.object.flags & 0x80 == 0),
            object: m.object.index,
            identifier: m.object.identifier,
            velocity: m.object.velocity,
        };
        self.group_events.push(event);
    }
}

impl CombatEvent {
    pub fn tick(&self) -> u32 {
        match self {
            Self::AttackOrdered { tick, .. }
            | Self::ProjectileDetonated { tick, .. }
            | Self::UnitDamaged { tick, .. }
            | Self::HitReaction { tick, .. }
            | Self::ProjectileRebounded { tick, .. }
            | Self::ProjectileGroupLaunched { tick, .. }
            | Self::StainMesh { tick, .. }
            | Self::Footstep { tick, .. }
            | Self::ProjectileCreated { tick, .. }
            | Self::ProjectileGroupSound { tick, .. }
            | Self::ProjectileGroupLocal { tick, .. }
            | Self::MonsterVoice { tick, .. }
            | Self::MonsterLocalGroup { tick, .. }
            | Self::MonsterLocalGroupOff { tick, .. }
            | Self::UnitKilled { tick, .. } => *tick,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceSound {
    pub tick: u32,
    pub object: i32,
    pub identifier: i16,
    pub collection: FourCC,
    pub sounds: crate::engine_rng::AnimSounds,
    pub world: [i32; 3],
    pub request_flags: u16,
    pub velocity: [i16; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterSoundPlace {
    object: i32,
    identifier: i16,
    world: [i32; 3],
    velocity: [i16; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterCombat {
    pub monster: FourCC,
    pub attack_projectiles: Vec<Option<FourCC>>,
    pub impact_group: Option<FourCC>,
    pub object: Option<ObjectDefinition>,
    pub unit: UnitTag,
}

#[derive(Debug, Clone, Default)]
pub struct CombatDefinitions {
    pub monsters: crate::entities::TagIdMap<MonsterCombat>,
    pub projectiles: crate::entities::TagIdMap<ProjectileDefinition>,
    pub groups: crate::entities::TagIdMap<ProjectileGroupDefinition>,
    pub flights: crate::entities::TagIdMap<std::sync::Arc<FlightDefinition>>,
    pub attacks: crate::entities::TagIdMap<Vec<AttackDefinition>>,
    pub monster_bounds: crate::entities::TagIdMap<SequenceWorldBounds>,
    pub key_points: crate::entities::TagIdMap<Vec<Vec<[i16; 3]>>>,
    pub key_frames: crate::entities::TagIdMap<Vec<i16>>,
}

fn find_tag(archives: &[&TagArchive], group: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
    archives.iter().rev().find_map(|a| {
        a.find_tag_by_id(FourCC::new(*group), id)
            .and_then(|e| a.get_tag_data(e).ok())
    })
}

impl CombatDefinitions {
    pub fn load(archives: &[&TagArchive], archetypes: impl IntoIterator<Item = FourCC>) -> Self {
        let mut defs = Self::default();
        let archetypes: Vec<FourCC> = archetypes.into_iter().collect();
        let mut equalizers: Vec<Vec<u8>> = Vec::new();
        for &archetype in &archetypes {
            let monster = find_tag(archives, b"unit", archetype)
                .filter(|d| d.len() >= 4)
                .map(|d| FourCC::new([d[0], d[1], d[2], d[3]]))
                .unwrap_or(archetype);
            if let Some(m) = find_tag(archives, b"mons", monster)
                .filter(|m| m.len() >= 0x224)
                .map(|m| crate::mons_postprocess::MonsPostprocess::COMMAND_LINE.apply(&m))
                && !equalizers.contains(&m)
            {
                equalizers.push(m);
            }
        }
        for archetype in archetypes {
            if defs.monsters.contains_key(&archetype) {
                continue;
            }
            let monster = find_tag(archives, b"unit", archetype)
                .filter(|d| d.len() >= 4)
                .map(|d| FourCC::new([d[0], d[1], d[2], d[3]]))
                .unwrap_or(archetype);
            let Some(mons) = find_tag(archives, b"mons", monster) else {
                continue;
            };
            let Ok(unit) = UnitTag::from_mons_bytes(&mons) else {
                continue;
            };
            let collection = find_tag(archives, b"unit", archetype)
                .filter(|d| d.len() >= 8)
                .and_then(|d| find_tag(archives, b"core", FourCC::new([d[4], d[5], d[6], d[7]])))
                .and_then(|c| mandrake_tag::CollectionReference::from_bytes(&c).ok())
                .and_then(|c| find_tag(archives, b".256", c.collection_tag));
            if let Some(k) = collection
                .as_ref()
                .and_then(|c| mandrake_tag::sequence_header::sequence_key_points(c).ok())
            {
                defs.key_points.insert(archetype, k);
            }
            let headers = collection.and_then(|c| mandrake_tag::sequence_headers(&c).ok());
            if let Some(h) = headers.as_ref() {
                defs.key_frames
                    .insert(archetype, h.iter().map(|s| s.key_frame).collect());
            }
            if let Some(ai) = headers
                .clone()
                .and_then(|h| AiDefinition::from_tags(monster, &mons, h).ok())
            {
                if let Some(h) = headers
                    .as_ref()
                    .and_then(|h| mandrake_tag::get_sequence(h, i32::from(ai.idle_sequence)))
                {
                    defs.monster_bounds.insert(archetype, h.world_bounds);
                }
                defs.attacks.insert(archetype, ai.attacks);
            }
            let object = unit.object_tag.and_then(|id| {
                find_tag(archives, b"obje", id)
                    .and_then(|d| ObjectDefinition::from_bytes(id, &d).ok())
            });
            for id in unit.attack_projectiles.iter().flatten() {
                defs.load_projectile(archives, *id, &equalizers);
            }
            if let Some(g) = unit.impact_group {
                defs.load_group(archives, g);
            }
            defs.monsters.insert(
                archetype,
                MonsterCombat {
                    monster,
                    attack_projectiles: unit.attack_projectiles.clone(),
                    impact_group: unit.impact_group,
                    object,
                    unit,
                },
            );
        }
        defs
    }

    fn load_projectile(&mut self, archives: &[&TagArchive], id: FourCC, equalizers: &[Vec<u8>]) {
        if self.projectiles.contains_key(&id) {
            return;
        }
        let Some(p) = find_tag(archives, b"proj", id)
            .and_then(|d| ProjectileDefinition::from_bytes(id, &d).ok())
        else {
            return;
        };
        if let Some(g) = p.detonation_group {
            self.load_group(archives, g);
        }
        let physics = p
            .object
            .and_then(|o| find_tag(archives, b"obje", o))
            .and_then(|d| ObjectPhysics::from_bytes(&d).ok());
        if let Some(physics) = physics {
            let bounds = p
                .collection
                .and_then(|c| {
                    let mut h =
                        mandrake_tag::sequence_headers(&find_tag(archives, b".256", c)?).ok()?;
                    for m in equalizers.iter().filter(|m| m[4..8] == c.0[..]) {
                        let _ = crate::engine_rng::equalize_monster_sequences(m, &mut h);
                    }
                    mandrake_tag::get_sequence(&h, i32::from(p.sequences[0])).copied()
                })
                .map(|h| h.world_bounds)
                .unwrap_or_default();
            self.flights.insert(
                id,
                std::sync::Arc::new(FlightDefinition::new(&p, physics, bounds)),
            );
        }
        self.projectiles.insert(id, p);
    }

    fn load_group(&mut self, archives: &[&TagArchive], id: FourCC) {
        if self.groups.contains_key(&id) {
            return;
        }
        if let Some(g) = find_tag(archives, b"prgr", id)
            .and_then(|d| ProjectileGroupDefinition::from_bytes(id, &d).ok())
        {
            self.groups.insert(id, g);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CombatState {
    pub definitions: CombatDefinitions,
    pub events: Vec<CombatEvent>,
    pub sequence_sounds: Vec<SequenceSound>,
    pub monster_sound_places: Vec<MonsterSoundPlace>,
    pub flight: crate::flight::FlightState,
    pub explosions: Vec<crate::explosions::Explosion>,
    pub(crate) next_projectile_id: u32,
}

impl Simulation {
    pub fn set_combat_definitions(&mut self, definitions: CombatDefinitions) {
        for unit in self.units.values_mut() {
            if let Some(o) = unit.engine_object {
                unit.health = Fixed16::from_i32(i32::from(o.vitality));
            } else if let Some(o) = definitions
                .monsters
                .get(&unit.archetype)
                .and_then(|m| m.object)
            {
                let hp = Fixed16::from_i32(i32::from(o.vitality));
                unit.health = hp;
                unit.max_health = hp;
            }
        }
        self.combat.definitions = definitions;
    }

    pub fn order_attack(&mut self, attackers: &[u32], target: AttackTarget) {
        match target {
            AttackTarget::Units(ids) => {
                let Some(&first) = ids.first() else {
                    return;
                };
                let click = self.attack_click;
                let w = &self.engine;
                let selection: Vec<usize> = attackers
                    .iter()
                    .filter_map(|&id| w.monster_index_by_identifier(id as u16))
                    .collect();
                let r = match w.monster_index_by_identifier(first as u16) {
                    Some(t) if !selection.is_empty() => Some(
                        w.local_attack_click(&selection, t, click)
                            .map(|(flags, targets)| {
                                let ids = targets
                                    .iter()
                                    .map(|&j| u32::from(w.monsters[j].identifier as u16))
                                    .collect();
                                (flags, ids)
                            }),
                    ),
                    _ => None,
                };
                let (flags, target_ids) = match r {
                    Some(Ok(v)) => v,
                    Some(Err(e)) => {
                        self.engine_stop.get_or_insert(e);
                        return;
                    }
                    None => (0, ids),
                };
                self.queue_local_target(crate::LocalTarget {
                    flags,
                    unit_ids: attackers.to_vec(),
                    target_ids,
                });
            }
            AttackTarget::Location(p) => {
                let (x, y) = self.world_to_command_point(p);
                let state_3 = self.attack_click.state_3;
                let (flags, z) = self.engine.local_attack_location(x, y, state_3);
                self.queue_local_attack_location(crate::LocalAttackLocation {
                    flags,
                    point: (x, y, z),
                    unit_ids: attackers.to_vec(),
                });
            }
        }
    }

    pub(crate) fn record_attack_order(&mut self, attackers: &[u32], target: AttackTarget) {
        let target_unit = match &target {
            AttackTarget::Units(ids) => ids.first().copied(),
            AttackTarget::Location(_) => None,
        };
        let mut ordered = Vec::new();
        for id in attackers {
            if let Some(unit) = self.units.get_mut(id)
                && unit.is_alive()
            {
                unit.target_unit_id = target_unit;
                ordered.push(*id);
            }
        }
        self.combat.events.push(CombatEvent::AttackOrdered {
            tick: self.current_tick,
            attackers: ordered,
            target,
        });
    }

    pub fn drain_combat_events(&mut self) -> Vec<CombatEvent> {
        self.take_created_records();
        let tick = self.current_tick.wrapping_sub(1);
        {
            let w = &mut self.engine;
            for mut e in std::mem::take(&mut w.group_events) {
                if let CombatEvent::ProjectileGroupSound { tick: t, .. }
                | CombatEvent::ProjectileGroupLocal { tick: t, .. }
                | CombatEvent::MonsterVoice { tick: t, .. }
                | CombatEvent::MonsterLocalGroup { tick: t, .. }
                | CombatEvent::MonsterLocalGroupOff { tick: t, .. }
                | CombatEvent::Footstep { tick: t, .. } = &mut e
                {
                    *t = tick;
                }
                self.combat.events.push(e);
            }
        }
        std::mem::take(&mut self.combat.events)
    }

    fn take_created_records(&mut self) {
        for p in &mut self.combat.flight.flights {
            let Some((point, velocity)) = p.created_at.take() else {
                continue;
            };
            let Some(shape) = p.shape.as_ref() else {
                continue;
            };
            self.combat.events.push(CombatEvent::ProjectileCreated {
                tick: p.launched_tick,
                definition: p.definition,
                object: shape.index,
                identifier: shape.identifier,
                point,
                facing: p.facing,
                velocity,
            });
        }
    }

    pub fn drain_sequence_sounds(&mut self) -> Vec<SequenceSound> {
        let tick = self.current_tick.wrapping_sub(1);
        let mut out = std::mem::take(&mut self.combat.sequence_sounds);
        let mut take = |o: &mut crate::engine_rng::EngineObject, world: [i32; 3], velocity| {
            if let Some(sounds) = o.anim_sounds.take() {
                out.push(SequenceSound {
                    tick,
                    object: o.index,
                    identifier: o.identifier,
                    collection: o.collection,
                    sounds,
                    world,
                    request_flags: u16::from(o.flags & 0x80 == 0),
                    velocity,
                });
            }
        };
        {
            let (half_w, half_d, cell_size) = self.client_frame();
            let shift = (65536 / mandrake_rec::WORLD_UNITS_PER_CELL).trailing_zeros();
            let Simulation {
                engine,
                units,
                combat,
                ..
            } = self;
            let movers = &engine.movers;
            let places = &mut combat.monster_sound_places;
            for (i, m) in engine.monsters.iter_mut().enumerate() {
                let (x, y) = match movers.get(i).and_then(Option::as_ref) {
                    Some(v) => (v.x, v.y),
                    None => m
                        .unit_key()
                        .and_then(|id| {
                            let u = units.get(&id)?;
                            Some(match u.engine.as_ref() {
                                Some(mv) => (mv.x, mv.y),
                                None => {
                                    let col = (half_w - u.position.x) / cell_size;
                                    let row = (u.position.z + half_d) / cell_size;
                                    (col.to_raw() >> shift, row.to_raw() >> shift)
                                }
                            })
                        })
                        .unwrap_or(m.position),
                };
                let now = [x, y, m.object.z];
                let (world, velocity) = places
                    .get(i)
                    .filter(|p| p.object == m.object.index && p.identifier == m.object.identifier)
                    .map_or((now, m.object.velocity), |p| (p.world, p.velocity));
                take(&mut m.object, world, velocity);
                let place = MonsterSoundPlace {
                    object: m.object.index,
                    identifier: m.object.identifier,
                    world: now,
                    velocity: m.object.velocity,
                };
                if i < places.len() {
                    places[i] = place;
                } else {
                    places.push(place);
                }
            }
            places.truncate(engine.monsters.len());
        }

        out
    }

    pub fn sound_object_place(&self, object: ChannelObject) -> ObjectPlace {
        let w = &self.engine;
        if w.free_objects.contains(&object.index) {
            return ObjectPlace::Deleted;
        }
        let at = |p: [i32; 3], v: [i16; 3]| ObjectPlace::At(WorldPoint::new(p[0], p[1], p[2]), v);
        let mut reused = false;
        let mut same = |o: &crate::engine_rng::EngineObject| {
            let index = o.index == object.index;
            reused |= index && o.identifier != object.identifier;
            index && o.identifier == object.identifier
        };
        for p in &self.combat.flight.flights {
            if p.shape.as_ref().is_some_and(&mut same) {
                let o = &p.object;
                return at([o.x, o.y, o.z], [o.vx, o.vy, o.vz]);
            }
        }
        for d in &self.combat.flight.dormant {
            if same(&d.object) {
                return at(d.position, d.object.velocity);
            }
        }
        for m in &w.monsters {
            if same(&m.object) {
                if m.identifier == 0 {
                    return ObjectPlace::Deleted;
                }
                let (x, y) = m
                    .unit_key()
                    .and_then(|id| self.unit_world_xy(id))
                    .unwrap_or(m.position);
                return at([x, y, m.object.z], m.object.velocity);
            }
        }
        for f in &w.fade_records {
            if same(&f.object)
                && let Some(p) = self.fade_object_place(f.index)
            {
                return at(p, f.object.velocity);
            }
        }
        if reused {
            ObjectPlace::Deleted
        } else {
            ObjectPlace::Unknown
        }
    }

    pub(crate) fn cause_damage(
        &mut self,
        tick: u32,
        definition: FourCC,
        damage: &DamageDefinition,
        owner: u32,
        hit: Option<u32>,
        point: Option<[i32; 3]>,
    ) -> Option<Option<FourCC>> {
        let hit = hit.filter(|u| self.units.get(u).is_some_and(|u| u.is_alive()));
        let damage = *damage;
        let mut monster_hit = false;
        let mut damage_events = Vec::new();
        if damage.flags
            & (mandrake_tag::projectile::DAMAGE_FLAG_AREA
                | mandrake_tag::projectile::DAMAGE_FLAG_DELAYED_EXPLOSION)
            == 0
            && let Some(victim) = hit
        {
            monster_hit = true;
            let amount = scale_amount(roll_damage(&damage, &mut self.rng), 0x100);
            if damage_object_draws(damage.damage_type) {
                self.rng.next_u16();
            }
            let (threshold, has_29) = self
                .units
                .get(&victim)
                .and_then(|u| self.combat.definitions.monsters.get(&u.archetype))
                .map_or((0xffff, false), |m| {
                    (
                        m.unit.kill_ratio_threshold,
                        m.unit.death_groups[2].is_some(),
                    )
                });
            let unit = self.units.get_mut(&victim).expect("victim checked above");
            let max_hp = unit.max_health.to_i32();
            let result = damage_object(unit.health.to_i32(), max_hp, amount);
            unit.health = Fixed16::from_i32(result.hp_after);
            damage_events.push(CombatEvent::UnitDamaged {
                tick,
                unit_id: victim,
                attacker: owner,
                definition,
                damage_type: damage.damage_type,
                amount,
                hp_before: result.hp_before,
                hp_after: result.hp_after,
                lethal: result.lethal,
            });
            if result.lethal {
                unit.health = Fixed16::ZERO;
                unit.state = UnitState::Dead;
                unit.velocity = Vec3Fixed::ZERO;
                unit.waypoints.clear();
                damage_events.push(CombatEvent::UnitKilled {
                    tick,
                    unit_id: victim,
                    killer: owner,
                    animation_code: kill_code(
                        damage.damage_type,
                        amount,
                        max_hp,
                        threshold,
                        false,
                        has_29,
                    ),
                });
            } else {
                damage_events.push(CombatEvent::HitReaction {
                    tick,
                    unit_id: victim,
                });
            }
        }
        let mut area_group = None;
        if damage.flags & mandrake_tag::projectile::DAMAGE_FLAG_AREA != 0 {
            match point {
                Some(point) => {
                    match self.cause_area_damage(tick, definition, damage, owner, point) {
                        Ok(g) => area_group = g,
                        Err(e) => {
                            self.engine_stop.get_or_insert(e);
                        }
                    }
                }
                None => self.combat.flight.reports.push((
                    tick,
                    crate::engine_rng::Unmodelled::new(
                        draw_site::DAMAGE_AREA_POINT,
                        "area point not modelled",
                    ),
                )),
            }
        } else if damage.flags & mandrake_tag::projectile::DAMAGE_FLAG_DELAYED_EXPLOSION != 0 {
            match point {
                Some(point) => {
                    if let Err(e) =
                        self.cause_delayed_explosion(tick, definition, damage, owner, point)
                    {
                        self.engine_stop.get_or_insert(e);
                    }
                }
                None => self.combat.flight.reports.push((
                    tick,
                    crate::engine_rng::Unmodelled::new(
                        draw_site::DAMAGE_EXPLOSION_POINT,
                        "explosion point not modelled",
                    ),
                )),
            }
        }
        let lethal = damage_events
            .iter()
            .any(|e| matches!(e, CombatEvent::UnitKilled { .. }));
        let impact = (monster_hit && !lethal).then(|| {
            hit.and_then(|u| self.units.get(&u))
                .and_then(|u| self.combat.definitions.monsters.get(&u.archetype))
                .and_then(|m| m.impact_group)
        });
        let killed = damage_events.iter().find_map(|e| match e {
            CombatEvent::UnitKilled {
                unit_id,
                animation_code,
                ..
            } if Some(*unit_id) == hit => Some((*unit_id, *animation_code)),
            _ => None,
        });
        if let Some((victim, code)) = killed {
            let amount = damage_events
                .iter()
                .find_map(|e| match e {
                    CombatEvent::UnitDamaged { amount, .. } => Some(*amount),
                    _ => None,
                })
                .unwrap_or(0);
            self.kill_monster_group(tick, victim, code, &damage, amount, point);
            self.units.remove(&victim);
        }
        self.combat.events.extend(damage_events);
        area_group.map(Some).or(impact)
    }
}

pub fn death_group(groups: [Option<FourCC>; 3], code: u8) -> Option<FourCC> {
    match code {
        0x26 => groups[0].or(groups[1]),
        0x27 => groups[1].or(groups[0]),
        0x29 => groups[2],
        _ => None,
    }
}

pub(crate) fn kill_descriptor(
    group: FourCC,
    object: i32,
    code: u8,
    damage: &DamageDefinition,
    amount: i32,
    point: [i32; 3],
) -> crate::projectile_groups::GroupDescriptor {
    let t = damage.damage_type;
    let alt = damage.flags & 0x200 != 0;
    let flags = if t == 8 || (t == 9 && code != 0x29) {
        if alt { 0x28 } else { 8 }
    } else if t == 10 {
        if alt { 0x120 } else { 0x100 }
    } else if alt {
        0x20
    } else {
        0
    };
    let magnitude = if damage.flags & 0x1000 != 0 {
        0
    } else {
        i32::from(((amount * i32::from(damage.word_0e)) >> 12).min(0x100) as i16)
    };
    crate::projectile_groups::GroupDescriptor {
        flags,
        group,
        shading_object: object,
        object,
        owner: -1,
        point,
        magnitude,
        velocity: [0; 3],
    }
}

impl Simulation {
    pub fn kill_monster_group(
        &mut self,
        tick: u32,
        victim: u32,
        code: u8,
        damage: &DamageDefinition,
        amount: i32,
        point: Option<[i32; 3]>,
    ) {
        if let Err(e) =
            self.launch_death_group(tick, victim, code, damage, amount, point, false, None)
        {
            self.combat.flight.reports.push((tick, e));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_death_group(
        &mut self,
        tick: u32,
        victim: u32,
        code: u8,
        damage: &DamageDefinition,
        amount: i32,
        point: Option<[i32; 3]>,
        engine: bool,
        record: Option<i32>,
    ) -> Result<(), crate::engine_rng::Unmodelled> {
        let engine_groups = engine
            .then(|| {
                let w = &self.engine;
                let i = w.monster_index_by_identifier(victim as u16)?;
                let raw = &w.monster_definitions[w.kill_definition(i)].ai.raw;
                let at = |o: usize| -> Option<FourCC> {
                    let b: [u8; 4] = raw.get(o..o + 4)?.try_into().ok()?;
                    (b != [0xff; 4]).then_some(FourCC::new(b))
                };
                Some([at(0x1a4), at(0x194), at(0x3c)])
            })
            .flatten();
        let Some(groups) = engine_groups.or_else(|| {
            self.units
                .get(&victim)
                .and_then(|u| self.combat.definitions.monsters.get(&u.archetype))
                .map(|m| m.unit.death_groups)
        }) else {
            return Ok(());
        };
        let Some(group) = death_group(groups, code) else {
            return Ok(());
        };
        let unmodelled =
            |reason: String| crate::engine_rng::Unmodelled::new(draw_site::DEATH_GROUP, reason);
        let Some(src) = crate::projectile_groups::SimSpawn::unit_object(self, victim) else {
            return Err(unmodelled(format!(
                "death group {group}: unit {victim} has no engine object"
            )));
        };
        let Some(point) = point else {
            return Err(unmodelled(format!(
                "death group {group}: the damage point is not modelled"
            )));
        };
        let mut desc = kill_descriptor(group, src.index, code, damage, amount, point);
        if engine && let Some(i) = self.engine.monster_index_by_identifier(victim as u16) {
            let m = &self.engine.monsters[i];
            desc.flags |= crate::engine_damage::kill_descriptor_mode_flags(m.flags_8, m.mode);
        }
        let mut w = crate::projectile_groups::SimSpawn {
            sim: self,
            tick,
            source: None,
            reserved: record,
            source_record: None,
            engine,
            updating_object: None,
        };
        crate::projectile_groups::new_projectile_group(&mut w, &desc).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(damage_type: i16, base: i16, delta: i16) -> DamageDefinition {
        DamageDefinition {
            damage_type,
            flags: 0,
            damage_lower_bound: base,
            damage_delta: delta,
            radius_lower_bound: 0,
            radius_delta: 0,
            word_0c: 0,
            word_0e: 0x100,
        }
    }

    #[test]
    fn roll_is_base_plus_scaled_draw() {
        let mut rng = BungieRng::new(12345);
        let mut copy = rng;
        let r = i32::from(copy.next_u16());
        assert_eq!(
            roll_damage(&def(5, 128, 26), &mut rng),
            128 + ((r * 26) >> 16)
        );
        assert_eq!(rng.state, copy.state, "one draw when delta > 0");
        let before = rng.state;
        assert_eq!(roll_damage(&def(5, 384, 0), &mut rng), 384);
        assert_eq!(rng.state, before);
    }

    #[test]
    fn roll_stays_in_range() {
        let mut rng = BungieRng::new(1);
        for _ in 0..2000 {
            let a = roll_damage(&def(5, 128, 26), &mut rng);
            assert!((128..=153).contains(&a), "{a}");
        }
    }

    #[test]
    fn hp_rule() {
        assert_eq!(
            damage_object(1408, 1408, 138),
            HitResult {
                hp_before: 1408,
                hp_after: 1270,
                lethal: false
            }
        );
        assert!(damage_object(100, 1408, 100).lethal);
        assert_eq!(damage_object(100, 1408, 100).hp_after, 0);
        assert_eq!(damage_object(50, 1408, 140).hp_after, 0);
        assert!(damage_object(50, 1408, 140).lethal);
        assert!(!damage_object(141, 1408, 140).lethal);
        assert_eq!(damage_object(1400, 1408, -50).hp_after, 1408);
    }

    #[test]
    fn kill_codes_and_draw_rules() {
        let hp = 0x10000;
        assert_eq!(
            kill_code(5, 0x1745, hp, 0xbfff, false, false),
            KILL_CODE_DEFAULT
        );
        assert_eq!(
            kill_code(0, 0xe042, hp, 0xbfff, false, false),
            KILL_CODE_EXPLOSION
        );
        assert_eq!(
            kill_code(0, 0xaba4, hp, 0xbfff, false, false),
            KILL_CODE_EXPLOSION
        );
        assert_eq!(
            kill_code(5, 0xc000, hp, 0xbfff, false, false),
            KILL_CODE_EXPLOSION
        );
        assert_eq!(
            kill_code(5, 40, 40, 0xbfff, false, false),
            KILL_CODE_DEFAULT
        );
        assert_eq!(kill_code(0, 1, 40, 0xbfff, true, false), KILL_CODE_DEFAULT);
        assert_eq!(kill_code(9, 1, 40, 0xbfff, false, true), 0x29);
        assert!(damage_object_draws(5));
        assert!(!damage_object_draws(0));
        assert!(!damage_object_draws(1));
        assert!(!damage_object_draws(8));
        assert!(damage_object_draws(12));
    }

    #[test]
    fn frequency_corrected_threshold() {
        let mut rng = BungieRng::new(7);
        let before = rng.state;
        assert!(!frequency_corrected(&mut rng, 0, 0x6b8));
        assert_ne!(rng.state, before, "always draws");
        assert!(frequency_corrected(&mut rng, 0xffff, 0x6b8));
    }
}
