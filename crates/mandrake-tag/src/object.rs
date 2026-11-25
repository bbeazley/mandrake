use mandrake_common::{FourCC, M2Error};

pub const OBJECT_DEFINITION_SIZE: usize = 64;

pub const OBJECT_FLAG_SELECTION_FRAME: u16 = 0x0001;
pub const OBJECT_FLAG_VITALITY_BAR: u16 = 0x0002;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObjectDefinition {
    pub tag_id: FourCC,
    pub flags: u16,
    pub vitality: u16,
    pub vitality_delta: i16,
    pub scale: u16,
    pub scale_delta: i16,
}

impl ObjectDefinition {
    pub fn from_bytes(tag_id: FourCC, data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 0x10 {
            return Err(M2Error::TagParse(format!(
                "obje '{}' too short: {} bytes",
                tag_id,
                data.len()
            )));
        }
        let u16_at = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);
        Ok(Self {
            tag_id,
            flags: u16_at(0x00),
            vitality: u16_at(0x08),
            vitality_delta: u16_at(0x0a) as i16,
            scale: u16_at(0x0c),
            scale_delta: u16_at(0x0e) as i16,
        })
    }

    pub fn has_vitality_bar(&self) -> bool {
        self.flags & OBJECT_FLAG_VITALITY_BAR != 0
    }

    pub fn has_selection_frame(&self) -> bool {
        self.flags & OBJECT_FLAG_SELECTION_FRAME != 0
    }
}

pub const OBJECT_FLAG_FOLLOWS_GROUND: u16 = 0x0004;
pub const OBJECT_FLAG_NO_OBJECT_COLLISIONS: u16 = 0x0008;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ObjectPhysics {
    pub flags: u16,
    pub gravity: i16,
    pub elasticity: i16,
    pub terminal_velocity: i16,
    pub damage_modifiers: [i16; 9],
    pub damage_threshold: i16,
}

impl ObjectPhysics {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 8 {
            return Err(M2Error::TagParse(format!(
                "obje too short for physics: {} bytes",
                data.len()
            )));
        }
        let w = |o: usize| i16::from_be_bytes([data[o], data[o + 1]]);
        let optional = |o: usize| {
            data.get(o..o + 2)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))
        };
        let mut damage_modifiers = [0i16; 9];
        for (k, m) in damage_modifiers.iter_mut().enumerate() {
            *m = optional(0x10 + 2 * k);
        }
        Ok(Self {
            flags: w(0) as u16,
            gravity: w(2),
            elasticity: w(4),
            terminal_velocity: w(6),
            damage_modifiers,
            damage_threshold: optional(0x30),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_warrior_object() {
        let mut data = [0u8; 64];
        data[..16].copy_from_slice(&[
            0x00, 0x03, 0xff, 0xfe, 0x00, 0x80, 0x00, 0x00, 0x05, 0x80, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00,
        ]);
        let def = ObjectDefinition::from_bytes(FourCC::new(*b"wamo"), &data).unwrap();
        assert_eq!(def.flags, 3);
        assert!(def.has_vitality_bar());
        assert!(def.has_selection_frame());
        assert_eq!(def.vitality, 0x580);
        assert_eq!(def.vitality_delta, 0);
        assert_eq!(def.scale, 0x100);
        assert_eq!(def.scale_delta, 0);
        let p = ObjectPhysics::from_bytes(&data).unwrap();
        assert_eq!(
            (p.flags, p.gravity, p.elasticity, p.terminal_velocity),
            (3, -2, 0x80, 0)
        );
    }

    #[test]
    fn test_short_payload_rejected() {
        assert!(ObjectDefinition::from_bytes(FourCC::new(*b"xxxx"), &[0; 8]).is_err());
    }
}
