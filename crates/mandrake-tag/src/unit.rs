use byteorder::{BigEndian, ReadBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::io::{Cursor, Read};

use crate::archive::TagArchive;
use crate::archive::TagEntry;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnitTag {
    pub tag_id: FourCC,
    pub monster_tag_id: FourCC,
    pub collection_ref_tag: FourCC,
    pub vitality: u32,
    pub movement_speed: u32,
    pub size: u32,
    pub animation_collection: FourCC,
    pub standing_sequence: i16,
    pub moving_sequence: i16,
    pub sequences: [i16; MONSTER_SEQUENCE_SLOTS],
    pub object_tag: Option<FourCC>,
    pub projectile_tag: Option<FourCC>,
    pub attack_projectiles: Vec<Option<FourCC>>,
    pub impact_group: Option<FourCC>,
    pub sounds: [Option<FourCC>; MONSTER_SOUND_SLOTS],
    pub sound_types: [i8; MONSTER_SOUND_SLOTS],
    #[serde(default)]
    pub death_groups: [Option<FourCC>; 3],
    #[serde(default)]
    pub kill_ratio_threshold: u16,
    #[serde(default)]
    pub reaction_ratio_threshold: u16,
    #[serde(default)]
    pub attack_sequence_pairs: Vec<[(u16, i16); 4]>,
}

pub const TERRAIN_TYPE_COUNT: usize = 16;
pub const MONSTER_TERRAIN_COSTS_OFFSET: usize = 0x48;
pub const MONSTER_IMPASSABLE_TERRAIN_OFFSET: usize = 0x58;
pub const MONSTER_TERRAIN_SPEEDS_OFFSET: usize = 0x5c;
pub const MONSTER_FLAG_SPEED_DECIDES_IMPASSABLE: u32 = 0x4000_0000;
pub const MONSTER_FLAG_FLYING: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MonsterTerrain {
    pub flags: u32,
    pub costs: [i8; TERRAIN_TYPE_COUNT],
    pub speeds: [u8; TERRAIN_TYPE_COUNT],
}

impl MonsterTerrain {
    pub fn from_mons_bytes(mons: &[u8]) -> Result<Self, M2Error> {
        if mons.len() < MONSTER_TERRAIN_SPEEDS_OFFSET + TERRAIN_TYPE_COUNT {
            return Err(M2Error::TagParse(format!(
                "mons too short for the terrain fields: {} bytes",
                mons.len()
            )));
        }
        let mut costs = [0i8; TERRAIN_TYPE_COUNT];
        let mut speeds = [0u8; TERRAIN_TYPE_COUNT];
        for t in 0..TERRAIN_TYPE_COUNT {
            costs[t] = mons[MONSTER_TERRAIN_COSTS_OFFSET + t] as i8;
            speeds[t] = mons[MONSTER_TERRAIN_SPEEDS_OFFSET + t];
        }
        Ok(Self {
            flags: u32::from_be_bytes([mons[0], mons[1], mons[2], mons[3]]),
            costs,
            speeds,
        })
    }

    pub fn impassable_mask(&self, gameplay_version: i32) -> u16 {
        let mut mask = 0u16;
        for t in 0..TERRAIN_TYPE_COUNT {
            let negative = self.costs[t] < 0;
            let zero = self.speeds[t] == 0;
            let blocked = if gameplay_version > 0x6a4 {
                if self.flags & MONSTER_FLAG_SPEED_DECIDES_IMPASSABLE != 0 {
                    zero
                } else {
                    negative || zero
                }
            } else if gameplay_version > 0x514 {
                negative || zero
            } else {
                negative
            };
            if blocked {
                mask |= 1 << t;
            }
        }
        mask
    }

    pub fn may_enter(&self, t: usize, gameplay_version: i32) -> bool {
        self.impassable_mask(gameplay_version) >> (t & 0xf) & 1 == 0
    }

    pub fn flies(&self) -> bool {
        self.flags & MONSTER_FLAG_FLYING != 0
    }
}

pub const MONSTER_SEQUENCE_SLOTS: usize = 19;
pub const MONSTER_SEQUENCES_OFFSET: usize = 0x08;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonsterSequenceSlot(pub usize);

impl MonsterSequenceSlot {
    pub const STANDING: Self = Self(0);
    pub const MOVING: Self = Self(1);
    pub const FLINCH: Self = Self((0x14 - 0x08) / 2);
    pub const CELEBRATE: Self = Self((0x1a - 0x08) / 2);
    pub const MODE_16: Self = Self((0x2c - 0x08) / 2);
}

pub fn attack_elevation_mask(elevation: u16) -> u16 {
    match elevation {
        0x1000..=0x7fff => 4,
        0x8000..=0xfbff => 2,
        _ => 1,
    }
}

pub const MONSTER_ATTACK_SLOTS: usize = 4;
pub const MONSTER_ATTACK_COUNT_OFFSET: usize = 0x88;
pub const MONSTER_ATTACKS_OFFSET: usize = 0x8c;
pub const MONSTER_ATTACK_SIZE: usize = 0x40;
pub const MONSTER_IMPACT_GROUP_OFFSET: usize = 0x1a0;

pub const MONSTER_SOUND_SLOTS: usize = 10;
pub const MONSTER_SOUNDS_OFFSET: usize = 0x1bc;
pub const MONSTER_SOUND_TYPES_OFFSET: usize = 0x1f8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MonsterSoundType(pub i8);

impl MonsterSoundType {
    pub const ATTACK_ORDER: Self = Self(0);
    pub const MULTIPLE_ATTACK_ORDER: Self = Self(1);
    pub const MOVE_ORDER: Self = Self(3);
    pub const MULTIPLE_MOVE_ORDER: Self = Self(4);
    pub const SELECTION: Self = Self(5);
    pub const MULTIPLE_SELECTION: Self = Self(6);
    pub const HIT_FRIENDLY_UNIT: Self = Self(7);
    pub const HIT_BY_FRIENDLY_UNIT: Self = Self(8);
    pub const ATTACK_OBSTACLE: Self = Self(9);
    pub const ATTACK_ENEMY_UNIT: Self = Self(10);
    pub const ATTACK_ENEMY_WITH_FRIENDS_NEARBY: Self = Self(11);
    pub const CAUSED_ENEMY_DEATH: Self = Self(13);
    pub const CELEBRATE: Self = Self(16);

    pub fn multiple_variant(self) -> Self {
        match self {
            Self::ATTACK_ORDER => Self::MULTIPLE_ATTACK_ORDER,
            Self::MOVE_ORDER => Self::MULTIPLE_MOVE_ORDER,
            Self::SELECTION => Self::MULTIPLE_SELECTION,
            other => other,
        }
    }
}

pub fn monster_group_sound<K: PartialEq + Copy>(
    definitions: &[K],
    sound: MonsterSoundType,
) -> (MonsterSoundType, Vec<usize>) {
    let mut counts: Vec<(K, usize)> = Vec::new();
    let mut dominant: Option<(K, usize)> = None;
    for &d in definitions {
        let count = match counts.iter_mut().find(|(k, _)| *k == d) {
            Some((_, c)) => {
                *c += 1;
                *c
            }
            None => {
                counts.push((d, 1));
                1
            }
        };
        if dominant.is_none_or(|(_, max)| count > max) {
            dominant = Some((d, count));
        }
    }
    let Some((dominant, count)) = dominant else {
        return (sound, Vec::new());
    };
    let sound = if count > 2 {
        sound.multiple_variant()
    } else {
        sound
    };
    let speakers = definitions
        .iter()
        .enumerate()
        .filter(|(_, d)| **d == dominant)
        .map(|(i, _)| i)
        .collect();
    (sound, speakers)
}

impl UnitTag {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 8 {
            return Err(M2Error::TagParse(format!(
                "Unit data too short: expected >= 8 bytes, got {}",
                data.len()
            )));
        }

        if data.len() >= 1024 {
            Self::from_mons_bytes(data)
        } else {
            Self::from_unit_bytes(data)
        }
    }

    pub fn from_unit_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 8 {
            return Err(M2Error::TagParse(format!(
                "Unit record too short: expected >= 8 bytes, got {}",
                data.len()
            )));
        }

        let mut mons_id_bytes = [0u8; 4];
        mons_id_bytes.copy_from_slice(&data[0..4]);
        let monster_tag_id = FourCC::new(mons_id_bytes);

        let mut core_id_bytes = [0u8; 4];
        core_id_bytes.copy_from_slice(&data[4..8]);
        let collection_ref_tag = FourCC::new(core_id_bytes);

        Ok(Self {
            tag_id: monster_tag_id,
            monster_tag_id,
            collection_ref_tag,
            vitality: 100,
            movement_speed: 100,
            size: 1,
            animation_collection: collection_ref_tag,
            standing_sequence: 0,
            moving_sequence: 0,
            sequences: [-1; MONSTER_SEQUENCE_SLOTS],
            object_tag: None,
            projectile_tag: None,
            attack_projectiles: Vec::new(),
            impact_group: None,
            sounds: [None; MONSTER_SOUND_SLOTS],
            sound_types: [-1; MONSTER_SOUND_SLOTS],
            death_groups: [None; 3],
            kill_ratio_threshold: 0,
            reaction_ratio_threshold: 0,
            attack_sequence_pairs: Vec::new(),
        })
    }

    pub fn from_mons_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 1024 {
            return Err(M2Error::TagParse(format!(
                "Monster record too short: expected >= 1024 bytes, got {}",
                data.len()
            )));
        }

        let mut cursor = Cursor::new(data);

        cursor.set_position(4);
        let mut anim_bytes = [0u8; 4];
        cursor.read_exact(&mut anim_bytes)?;
        let animation_collection = FourCC::new(anim_bytes);

        let standing_sequence = cursor.read_i16::<BigEndian>()?;
        let moving_sequence = cursor.read_i16::<BigEndian>()?;

        let mut sequences = [-1i16; MONSTER_SEQUENCE_SLOTS];
        for (i, s) in sequences.iter_mut().enumerate() {
            let at = MONSTER_SEQUENCES_OFFSET + 2 * i;
            *s = i16::from_be_bytes([data[at], data[at + 1]]);
        }

        cursor.set_position(0x6C);
        let vitality = cursor.read_u32::<BigEndian>()?;

        let movement_speed = cursor.read_u32::<BigEndian>()?;

        cursor.set_position(0x78);
        let size = cursor.read_u32::<BigEndian>()?;

        cursor.set_position(0x84);
        let mut object_bytes = [0u8; 4];
        let object_tag = if cursor.read_exact(&mut object_bytes).is_ok()
            && Self::is_valid_fourcc(&object_bytes)
        {
            Some(FourCC::new(object_bytes))
        } else {
            None
        };

        cursor.set_position(0x90);
        let mut proj_bytes = [0u8; 4];
        let projectile_tag =
            if cursor.read_exact(&mut proj_bytes).is_ok() && Self::is_valid_fourcc(&proj_bytes) {
                Some(FourCC::new(proj_bytes))
            } else {
                None
            };

        let attack_count = i16::from_be_bytes([
            data[MONSTER_ATTACK_COUNT_OFFSET],
            data[MONSTER_ATTACK_COUNT_OFFSET + 1],
        ]);
        let attack_projectiles = (0..usize::try_from(attack_count)
            .unwrap_or(0)
            .min(MONSTER_ATTACK_SLOTS))
            .map(|i| {
                let at = MONSTER_ATTACKS_OFFSET + MONSTER_ATTACK_SIZE * i + 4;
                let bytes: [u8; 4] = data[at..at + 4].try_into().expect("4-byte slice");
                (u32::from_be_bytes(bytes) != u32::MAX && Self::is_valid_fourcc(&bytes))
                    .then(|| FourCC::new(bytes))
            })
            .collect();
        let impact_group = {
            let at = MONSTER_IMPACT_GROUP_OFFSET;
            let bytes: [u8; 4] = data[at..at + 4].try_into().expect("4-byte slice");
            (u32::from_be_bytes(bytes) != u32::MAX && Self::is_valid_fourcc(&bytes))
                .then(|| FourCC::new(bytes))
        };

        let mut sounds = [None; MONSTER_SOUND_SLOTS];
        for (i, slot) in sounds.iter_mut().enumerate() {
            let at = MONSTER_SOUNDS_OFFSET + 4 * i;
            let bytes: [u8; 4] = data[at..at + 4].try_into().expect("4-byte slice");
            if u32::from_be_bytes(bytes) != u32::MAX && Self::is_valid_fourcc(&bytes) {
                *slot = Some(FourCC::new(bytes));
            }
        }
        let mut sound_types = [-1i8; MONSTER_SOUND_SLOTS];
        for (i, t) in sound_types.iter_mut().enumerate() {
            *t = data[MONSTER_SOUND_TYPES_OFFSET + i] as i8;
        }

        let group_at = |at: usize| {
            let bytes: [u8; 4] = data[at..at + 4].try_into().expect("4-byte slice");
            (u32::from_be_bytes(bytes) != u32::MAX && Self::is_valid_fourcc(&bytes))
                .then(|| FourCC::new(bytes))
        };
        let attack_sequence_pairs = (0..usize::try_from(attack_count)
            .unwrap_or(0)
            .min(MONSTER_ATTACK_SLOTS))
            .map(|i| {
                let base = MONSTER_ATTACKS_OFFSET + MONSTER_ATTACK_SIZE * i + 0x0c;
                std::array::from_fn(|k| {
                    let at = base + 8 * k;
                    (
                        u16::from_be_bytes([data[at], data[at + 1]]),
                        i16::from_be_bytes([data[at + 2], data[at + 3]]),
                    )
                })
            })
            .collect();
        let death_groups = [group_at(0x1a4), group_at(0x194), group_at(0x3c)];
        let kill_ratio_threshold = u16::from_be_bytes([data[0x19c], data[0x19d]]);
        let reaction_ratio_threshold = u16::from_be_bytes([data[0x19e], data[0x19f]]);

        Ok(Self {
            tag_id: animation_collection,
            monster_tag_id: animation_collection,
            collection_ref_tag: animation_collection,
            vitality,
            movement_speed,
            size,
            animation_collection,
            standing_sequence,
            moving_sequence,
            sequences,
            object_tag,
            projectile_tag,
            attack_projectiles,
            impact_group,
            sounds,
            sound_types,
            death_groups,
            attack_sequence_pairs,
            kill_ratio_threshold,
            reaction_ratio_threshold,
        })
    }

    pub fn from_archive(archive: &TagArchive, unit_entry: &TagEntry) -> Result<Self, M2Error> {
        let unit_data = archive.get_tag_data(unit_entry)?;
        let mut unit = Self::from_unit_bytes(&unit_data)?;
        unit.tag_id = unit_entry.tag_id;

        if let Some(mons_entry) = archive.find_tag_by_id(FourCC::new(*b"mons"), unit.monster_tag_id)
            && let Ok(mons_data) = archive.get_tag_data(mons_entry)
            && let Ok(mons) = Self::from_mons_bytes(&mons_data)
        {
            unit.vitality = mons.vitality;
            unit.movement_speed = mons.movement_speed;
            unit.size = mons.size;
            unit.animation_collection = mons.animation_collection;
            unit.standing_sequence = mons.standing_sequence;
            unit.moving_sequence = mons.moving_sequence;
            unit.sequences = mons.sequences;
            unit.object_tag = mons.object_tag;
            unit.projectile_tag = mons.projectile_tag;
            unit.attack_projectiles = mons.attack_projectiles;
            unit.impact_group = mons.impact_group;
            unit.sounds = mons.sounds;
            unit.sound_types = mons.sound_types;
            unit.death_groups = mons.death_groups;
            unit.kill_ratio_threshold = mons.kill_ratio_threshold;
            unit.reaction_ratio_threshold = mons.reaction_ratio_threshold;
            unit.attack_sequence_pairs = mons.attack_sequence_pairs;
        } else if let Some(core_entry) =
            archive.find_tag_by_id(FourCC::new(*b"core"), unit.collection_ref_tag)
            && let Ok(core_data) = archive.get_tag_data(core_entry)
            && core_data.len() >= 4
        {
            let mut cc = [0u8; 4];
            cc.copy_from_slice(&core_data[0..4]);
            unit.animation_collection = FourCC::new(cc);
        }

        Ok(unit)
    }

    pub fn sound_for(&self, kind: MonsterSoundType) -> Option<FourCC> {
        let slot = self.sound_types.iter().position(|&t| t == kind.0)?;
        self.sounds[slot]
    }

    pub fn sequence_for(&self, slot: MonsterSequenceSlot) -> Option<usize> {
        self.sequences
            .get(slot.0)
            .and_then(|&s| usize::try_from(s).ok())
    }

    pub fn attack_sequences(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .attack_sequence_pairs
            .iter()
            .flatten()
            .filter_map(|&(_, s)| usize::try_from(s).ok())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn attack_sequence_candidates(&self, attack: usize, elevation: u16) -> Vec<usize> {
        let Some(pairs) = self.attack_sequence_pairs.get(attack) else {
            return vec![0];
        };
        let pick = |mask: u16| -> Vec<usize> {
            pairs
                .iter()
                .filter(|&&(flags, s)| s >= 0 && flags & mask != 0)
                .map(|&(_, s)| s as usize)
                .collect()
        };
        let found = pick(attack_elevation_mask(elevation));
        if !found.is_empty() {
            return found;
        }
        let level = pick(1);
        if level.is_empty() { vec![0] } else { level }
    }

    fn is_valid_fourcc(bytes: &[u8; 4]) -> bool {
        bytes.iter().all(|&b| b.is_ascii_graphic() || b == b' ')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mons_sequence_slots_are_decoded() {
        let mut data = vec![0u8; 1024];
        data[4..8].copy_from_slice(b"wam2");
        for slot in 0..MONSTER_SEQUENCE_SLOTS {
            let at = MONSTER_SEQUENCES_OFFSET + 2 * slot;
            data[at..at + 2].copy_from_slice(&(-1i16).to_be_bytes());
        }
        data[8..10].copy_from_slice(&6i16.to_be_bytes());
        data[10..12].copy_from_slice(&7i16.to_be_bytes());
        data[0x08 + 2 * 6..0x08 + 2 * 6 + 2].copy_from_slice(&3i16.to_be_bytes());
        let unit = UnitTag::from_mons_bytes(&data).unwrap();
        assert_eq!(unit.sequence_for(MonsterSequenceSlot::STANDING), Some(6));
        assert_eq!(unit.sequence_for(MonsterSequenceSlot::MOVING), Some(7));
        assert_eq!(unit.sequence_for(MonsterSequenceSlot(6)), Some(3));
        assert_eq!(unit.sequence_for(MonsterSequenceSlot(2)), None);
        assert_eq!(unit.sequence_for(MonsterSequenceSlot(40)), None);
    }

    #[test]
    fn attack_pairs_pick_the_sequences_of_the_elevation() {
        let mut data = vec![0u8; 1024];
        data[4..8].copy_from_slice(b"wam2");
        data[0x88..0x8a].copy_from_slice(&1i16.to_be_bytes());
        for (k, (flags, seq)) in [(1u16, 20i16), (1, 21), (4, 22), (0, -1)]
            .into_iter()
            .enumerate()
        {
            let at = MONSTER_ATTACKS_OFFSET + 0x0c + 8 * k;
            data[at..at + 2].copy_from_slice(&flags.to_be_bytes());
            data[at + 2..at + 4].copy_from_slice(&seq.to_be_bytes());
        }
        let unit = UnitTag::from_mons_bytes(&data).unwrap();
        assert_eq!(unit.attack_sequences(), vec![20, 21, 22]);
        assert_eq!(unit.attack_sequence_candidates(0, 0), vec![20, 21]);
        assert_eq!(unit.attack_sequence_candidates(0, 0x2000), vec![22]);
        assert_eq!(unit.attack_sequence_candidates(0, 0x9000), vec![20, 21]);
        assert_eq!(unit.attack_sequence_candidates(3, 0), vec![0]);
    }

    fn mons_with_sounds(slots: &[(&[u8; 4], i8)]) -> Vec<u8> {
        let mut data = vec![0u8; 1024];
        data[4..8].copy_from_slice(b"arch");
        data[0x84..0x88].copy_from_slice(b"arob");
        data[MONSTER_SOUNDS_OFFSET..MONSTER_SOUNDS_OFFSET + 4 * MONSTER_SOUND_SLOTS].fill(0xff);
        data[MONSTER_SOUND_TYPES_OFFSET..MONSTER_SOUND_TYPES_OFFSET + MONSTER_SOUND_SLOTS]
            .fill(0xff);
        for (i, (tag, kind)) in slots.iter().enumerate() {
            let at = MONSTER_SOUNDS_OFFSET + 4 * i;
            data[at..at + 4].copy_from_slice(*tag);
            data[MONSTER_SOUND_TYPES_OFFSET + i] = *kind as u8;
        }
        data
    }

    #[test]
    fn mons_sound_slots_and_types() {
        let data = mons_with_sounds(&[(b"voas", 5), (b"vo03", 0), (b"voam", 3), (b"voah", -1)]);
        let unit = UnitTag::from_mons_bytes(&data).unwrap();
        assert_eq!(unit.object_tag, Some(FourCC::new(*b"arob")));
        assert_eq!(unit.sounds[0], Some(FourCC::new(*b"voas")));
        assert_eq!(unit.sounds[4], None);
        assert_eq!(&unit.sound_types[..5], &[5, 0, 3, -1, -1]);
        assert_eq!(
            unit.sound_for(MonsterSoundType::SELECTION),
            Some(FourCC::new(*b"voas"))
        );
        assert_eq!(
            unit.sound_for(MonsterSoundType::MOVE_ORDER),
            Some(FourCC::new(*b"voam"))
        );
        assert_eq!(unit.sound_for(MonsterSoundType::MULTIPLE_MOVE_ORDER), None);
    }

    #[test]
    fn mons_attacks_and_impact_group() {
        let mut data = mons_with_sounds(&[]);
        data[MONSTER_ATTACK_COUNT_OFFSET..MONSTER_ATTACK_COUNT_OFFSET + 2]
            .copy_from_slice(&3i16.to_be_bytes());
        for (i, tag) in [b"arss", b"arar", b"araf"].iter().enumerate() {
            let at = MONSTER_ATTACKS_OFFSET + MONSTER_ATTACK_SIZE * i + 4;
            data[at..at + 4].copy_from_slice(*tag);
        }
        data[MONSTER_IMPACT_GROUP_OFFSET..MONSTER_IMPACT_GROUP_OFFSET + 4].copy_from_slice(b"arhi");
        let unit = UnitTag::from_mons_bytes(&data).unwrap();
        assert_eq!(
            unit.attack_projectiles,
            vec![
                Some(FourCC::new(*b"arss")),
                Some(FourCC::new(*b"arar")),
                Some(FourCC::new(*b"araf"))
            ]
        );
        assert_eq!(unit.impact_group, Some(FourCC::new(*b"arhi")));
        data[MONSTER_IMPACT_GROUP_OFFSET..MONSTER_IMPACT_GROUP_OFFSET + 4].fill(0xff);
        assert_eq!(UnitTag::from_mons_bytes(&data).unwrap().impact_group, None);
    }

    #[test]
    fn sound_for_uses_first_matching_slot_even_if_empty() {
        let mut data = mons_with_sounds(&[(b"aaaa", 3), (b"bbbb", 3)]);
        data[MONSTER_SOUNDS_OFFSET..MONSTER_SOUNDS_OFFSET + 4].fill(0xff);
        let unit = UnitTag::from_mons_bytes(&data).unwrap();
        assert_eq!(unit.sound_for(MonsterSoundType::MOVE_ORDER), None);
    }

    #[test]
    fn group_sound_picks_first_definition_to_lead() {
        let (kind, speakers) =
            monster_group_sound(&['A', 'B', 'B', 'A'], MonsterSoundType::MOVE_ORDER);
        assert_eq!(kind, MonsterSoundType::MOVE_ORDER);
        assert_eq!(speakers, vec![1, 2]);
        let (_, speakers) = monster_group_sound(&['A', 'B'], MonsterSoundType::SELECTION);
        assert_eq!(speakers, vec![0]);
        assert!(
            monster_group_sound::<char>(&[], MonsterSoundType::SELECTION)
                .1
                .is_empty()
        );
    }

    #[test]
    fn group_sound_uses_multiple_variant_above_two() {
        let (kind, speakers) = monster_group_sound(&['W', 'W', 'D'], MonsterSoundType::MOVE_ORDER);
        assert_eq!(kind, MonsterSoundType::MOVE_ORDER);
        assert_eq!(speakers, vec![0, 1]);
        let (kind, speakers) =
            monster_group_sound(&['W', 'W', 'D', 'W'], MonsterSoundType::MOVE_ORDER);
        assert_eq!(kind, MonsterSoundType::MULTIPLE_MOVE_ORDER);
        assert_eq!(speakers, vec![0, 1, 3]);
        let three = ['W'; 3];
        assert_eq!(
            monster_group_sound(&three, MonsterSoundType::ATTACK_ORDER).0,
            MonsterSoundType::MULTIPLE_ATTACK_ORDER
        );
        assert_eq!(
            monster_group_sound(&three, MonsterSoundType::SELECTION).0,
            MonsterSoundType::MULTIPLE_SELECTION
        );
        assert_eq!(
            monster_group_sound(&three, MonsterSoundType::CELEBRATE).0,
            MonsterSoundType::CELEBRATE
        );
    }
}
