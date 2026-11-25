use mandrake_common::{FourCC, M2Error};

pub const PROJECTILE_DEFINITION_SIZE: usize = 0x100;
pub const PROJECTILE_GROUP_DEFINITION_SIZE: usize = 0x200;
pub const PROJECTILE_DAMAGE_OFFSET: usize = 0x68;

pub const PROJECTILE_FLAG_DETONATES_AT_CREATION: u32 = 0x2000;
pub const PROJECTILE_FLAG_DETONATES_WITHOUT_TARGET: u32 = 0x8000_0000;

pub const DAMAGE_FLAG_PROJECTILE: u16 = 0x0008;
pub const DAMAGE_FLAG_DELAYED_EXPLOSION: u16 = 0x0002;
pub const DAMAGE_FLAG_AREA: u16 = 0x1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DamageDefinition {
    pub damage_type: i16,
    pub flags: u16,
    pub damage_lower_bound: i16,
    pub damage_delta: i16,
    pub radius_lower_bound: i16,
    pub radius_delta: i16,
    pub word_0c: i16,
    pub word_0e: i16,
}

impl DamageDefinition {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 16 {
            return Err(M2Error::TagParse(format!(
                "damage definition too short: {} bytes",
                data.len()
            )));
        }
        let w = |o: usize| i16::from_be_bytes([data[o], data[o + 1]]);
        Ok(Self {
            damage_type: w(0),
            flags: w(2) as u16,
            damage_lower_bound: w(4),
            damage_delta: w(6),
            radius_lower_bound: w(8),
            radius_delta: w(10),
            word_0c: w(12),
            word_0e: w(14),
        })
    }

    pub fn deals_damage(&self) -> bool {
        self.damage_type != -1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectileDefinition {
    pub tag_id: FourCC,
    pub flags: u32,
    pub collection: Option<FourCC>,
    pub sequences: [i16; 3],
    pub detonation_group: Option<FourCC>,
    pub projectile_14: Option<FourCC>,
    pub object: Option<FourCC>,
    pub impact_frequency: u16,
    pub flight_sound: Option<FourCC>,
    pub rebound_sound: Option<FourCC>,
    pub projectile_5c: Option<FourCC>,
    pub groups: [Option<FourCC>; 3],
    pub damage: DamageDefinition,
    #[serde(default)]
    pub speed: i16,
    #[serde(default)]
    pub speed_delta: i16,
    #[serde(default)]
    pub velocity_error: i16,
    #[serde(default)]
    pub collision_chance: u16,
    #[serde(default)]
    pub lifetime: i16,
    #[serde(default)]
    pub lifetime_delta: i16,
    #[serde(default)]
    pub countdown: i16,
    #[serde(default)]
    pub countdown_delta: i16,
    #[serde(default)]
    pub byte_df: u8,
    #[serde(default)]
    pub raw: Vec<u8>,
}

use crate::parse::optional_fourcc as tag_ref;

impl ProjectileDefinition {
    pub fn from_bytes(tag_id: FourCC, data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < PROJECTILE_DEFINITION_SIZE {
            return Err(M2Error::TagParse(format!(
                "proj '{}' too short: {} bytes",
                tag_id,
                data.len()
            )));
        }
        let w = |o: usize| i16::from_be_bytes([data[o], data[o + 1]]);
        Ok(Self {
            tag_id,
            flags: u32::from_be_bytes(data[0..4].try_into().expect("4 bytes")),
            collection: tag_ref(data, 0x04),
            sequences: [w(0x08), w(0x0a), w(0x0c)],
            detonation_group: tag_ref(data, 0x10),
            projectile_14: tag_ref(data, 0x14),
            object: tag_ref(data, 0x1c),
            impact_frequency: w(0x2a) as u16,
            flight_sound: tag_ref(data, 0x3c),
            rebound_sound: tag_ref(data, 0x40),
            projectile_5c: tag_ref(data, 0x5c),
            groups: [
                tag_ref(data, 0x60),
                tag_ref(data, 0x7c),
                tag_ref(data, 0x80),
            ],
            damage: DamageDefinition::from_bytes(
                &data[PROJECTILE_DAMAGE_OFFSET..PROJECTILE_DAMAGE_OFFSET + 16],
            )?,
            speed: w(0x20),
            speed_delta: w(0x22),
            velocity_error: w(0x24),
            collision_chance: w(0x2c) as u16,
            lifetime: w(0x50),
            lifetime_delta: w(0x52),
            countdown: w(0x84),
            countdown_delta: w(0x86),
            byte_df: data[0xdf],
            raw: data[..PROJECTILE_DEFINITION_SIZE].to_vec(),
        })
    }

    pub fn word(&self, at: usize) -> i16 {
        self.raw
            .get(at..at + 2)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
    }

    pub fn uword(&self, at: usize) -> u16 {
        self.word(at) as u16
    }

    pub fn media_group_type(&self) -> i16 {
        self.word(0x0e)
    }

    pub fn media_chance(&self) -> u16 {
        self.uword(0x2e)
    }

    pub fn group_scale(&self) -> i16 {
        self.word(0x30)
    }

    pub fn class(&self) -> i16 {
        self.word(0x32)
    }

    pub fn shape_speed(&self) -> (u16, i16) {
        (self.uword(0x4c), self.word(0x4e))
    }

    pub fn word_54(&self) -> i16 {
        self.word(0x54)
    }

    pub fn detonates_at_creation(&self) -> bool {
        self.flags & PROJECTILE_FLAG_DETONATES_AT_CREATION != 0
    }

    pub fn effective_damage(&self) -> DamageDefinition {
        let mut damage = self.damage;
        if !self.detonates_at_creation() {
            damage.flags |= DAMAGE_FLAG_PROJECTILE;
        }
        damage
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectileGroupPart {
    pub projectile: Option<FourCC>,
    pub projectile_18: Option<FourCC>,
    pub raw: Vec<u8>,
}

impl ProjectileGroupPart {
    fn word(&self, at: usize) -> i16 {
        self.raw
            .get(at..at + 2)
            .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
    }

    pub fn flags(&self) -> u32 {
        self.raw
            .get(4..8)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn count(&self) -> (i16, i16) {
        (self.word(0x08), self.word(0x0a))
    }

    pub fn height(&self) -> (i16, i16) {
        (self.word(0x0c), self.word(0x0e))
    }

    pub fn threshold(&self) -> u16 {
        self.word(0x14) as u16
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectileGroupDefinition {
    pub tag_id: FourCC,
    pub flags: u32,
    pub mesh_effect: Option<FourCC>,
    pub sound: Option<FourCC>,
    pub local_group: Option<FourCC>,
    pub parts: Vec<ProjectileGroupPart>,
}

pub const PROJECTILE_GROUP_MAX_PARTS: usize = (PROJECTILE_GROUP_DEFINITION_SIZE - 0x20) / 0x20;

impl ProjectileGroupDefinition {
    pub fn from_bytes(tag_id: FourCC, data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < PROJECTILE_GROUP_DEFINITION_SIZE {
            return Err(M2Error::TagParse(format!(
                "prgr '{}' too short: {} bytes",
                tag_id,
                data.len()
            )));
        }
        let count = i16::from_be_bytes([data[4], data[5]]);
        let count = usize::try_from(count)
            .unwrap_or(0)
            .min(PROJECTILE_GROUP_MAX_PARTS);
        let parts = (0..count)
            .map(|i| {
                let at = 0x20 + 0x20 * i;
                ProjectileGroupPart {
                    projectile: tag_ref(data, at),
                    projectile_18: tag_ref(data, at + 0x18),
                    raw: data[at..at + 0x20].to_vec(),
                }
            })
            .collect();
        let sound_bytes: [u8; 4] = data[0x0c..0x10].try_into().expect("4 bytes");
        let sound = (u32::from_be_bytes(sound_bytes) != u32::MAX).then(|| FourCC::new(sound_bytes));
        Ok(Self {
            tag_id,
            flags: u32::from_be_bytes(data[0..4].try_into().expect("4 bytes")),
            mesh_effect: tag_ref(data, 0x08),
            sound,
            local_group: tag_ref(data, 0x10),
            parts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bowman_arrow() -> Vec<u8> {
        let mut d = vec![0u8; 256];
        let head: [u8; 0x80] = [
            0x00, 0x00, 0x00, 0x21, b'a', b'r', b'c', b'h', 0x00, 0x16, 0xff, 0xff, 0xff, 0xff,
            0x00, 0x02, b'a', b'r', b'a', b'p', 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x7f, 0xff,
            b'a', b'r', b'r', b'o', 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb4,
            0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0x14, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff,
            0x01, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, b'b', b'r', b'a', b'r', 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0x00, 0x4d, 0xbf, 0xff, 0x00, 0x05, 0x00, 0x00, 0x00, 0x80, 0x00, 0x1a,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff,
        ];
        d[..0x80].copy_from_slice(&head);
        d[0x80..0x84].fill(0xff);
        d
    }

    #[test]
    fn parses_bowman_arrow() {
        let p = ProjectileDefinition::from_bytes(FourCC::new(*b"arar"), &bowman_arrow()).unwrap();
        assert_eq!(p.flags, 0x21);
        assert_eq!(p.collection, Some(FourCC::new(*b"arch")));
        assert_eq!(p.detonation_group, Some(FourCC::new(*b"arap")));
        assert_eq!(p.object, Some(FourCC::new(*b"arro")));
        assert_eq!(p.flight_sound, None);
        assert_eq!(p.rebound_sound, Some(FourCC::new(*b"brar")));
        assert_eq!(p.impact_frequency, 0);
        assert_eq!((p.speed, p.speed_delta, p.velocity_error), (0x100, 0, 0));
        assert_eq!(p.collision_chance, 0xffff);
        assert_eq!((p.lifetime, p.lifetime_delta, p.countdown), (0, 0, 0));
        assert!(!p.detonates_at_creation());
        assert_eq!(
            p.damage,
            DamageDefinition {
                damage_type: 5,
                flags: 0,
                damage_lower_bound: 128,
                damage_delta: 26,
                radius_lower_bound: 0,
                radius_delta: 0,
                word_0c: 0,
                word_0e: 256,
            }
        );
        assert_eq!(p.effective_damage().flags, 0x8);
    }

    #[test]
    fn melee_keeps_damage_flags() {
        let mut d = bowman_arrow();
        d[0..4].copy_from_slice(&0x2000u32.to_be_bytes());
        let p = ProjectileDefinition::from_bytes(FourCC::new(*b"wamt"), &d).unwrap();
        assert!(p.detonates_at_creation());
        assert_eq!(p.effective_damage().flags, 0);
    }

    #[test]
    fn parses_group_parts() {
        let mut d = vec![0u8; 512];
        d[4..6].copy_from_slice(&1i16.to_be_bytes());
        d[8..0x14].fill(0xff);
        d[0x20..0x24].copy_from_slice(b"arab");
        d[0x34..0x36].fill(0xff);
        let g = ProjectileGroupDefinition::from_bytes(FourCC::new(*b"arap"), &d).unwrap();
        assert_eq!(g.sound, None);
        assert_eq!(g.mesh_effect, None);
        assert_eq!(g.parts.len(), 1);
        assert_eq!(g.parts[0].projectile, Some(FourCC::new(*b"arab")));
        assert_eq!(g.parts[0].projectile_18, None);
    }

    #[test]
    fn short_payloads_rejected() {
        assert!(ProjectileDefinition::from_bytes(FourCC::new(*b"xxxx"), &[0; 0x80]).is_err());
        assert!(ProjectileGroupDefinition::from_bytes(FourCC::new(*b"xxxx"), &[0; 0x80]).is_err());
    }
}
