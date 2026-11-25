use mandrake_common::{FourCC, M2Error};

pub const TFL_MONSTER_DEFINITION_SIZE: usize = 512;
pub const TFL_MONSTER_SEQUENCE_SLOTS: usize = 16;
pub const TFL_MONSTER_SOUND_SLOTS: usize = 10;
pub const TFL_MONSTER_ATTACK_SLOTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TflMonsterDefinition {
    pub tag_id: FourCC,
    pub flags: u32,
    pub collection_tag: FourCC,
    pub sequences: [i16; TFL_MONSTER_SEQUENCE_SLOTS],
    pub object_tag: FourCC,
    pub attack_count: i16,
    pub attack_projectiles: [Option<FourCC>; TFL_MONSTER_ATTACK_SLOTS],
    pub sounds: [Option<FourCC>; TFL_MONSTER_SOUND_SLOTS],
    pub raw: Vec<u8>,
}

fn four(d: &[u8], o: usize) -> FourCC {
    FourCC(d[o..o + 4].try_into().expect("in the tag"))
}

fn opt(d: &[u8], o: usize) -> Option<FourCC> {
    let t = four(d, o);
    (t.0 != [0xff; 4]).then_some(t)
}

impl TflMonsterDefinition {
    pub fn from_bytes(tag_id: FourCC, data: &[u8]) -> Result<Self, M2Error> {
        if data.len() != TFL_MONSTER_DEFINITION_SIZE {
            return Err(M2Error::TagParse(format!(
                "TFL mons '{tag_id}': {} bytes, expected {TFL_MONSTER_DEFINITION_SIZE}",
                data.len()
            )));
        }
        let i16_at = |o: usize| i16::from_be_bytes([data[o], data[o + 1]]);
        Ok(Self {
            tag_id,
            flags: u32::from_be_bytes(data[0..4].try_into().expect("4 bytes")),
            collection_tag: four(data, 0x04),
            sequences: std::array::from_fn(|i| i16_at(0x08 + 2 * i)),
            object_tag: four(data, 0x90),
            attack_count: i16_at(0x94),
            attack_projectiles: std::array::from_fn(|k| opt(data, 0x9c + 0x20 * k)),
            sounds: std::array::from_fn(|i| opt(data, 0x14c + 4 * i)),
            raw: data.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_512_byte_definition_reads_and_others_fail() {
        let mut d = vec![0u8; 512];
        d[4..8].copy_from_slice(b"warr");
        d[0x08..0x0c].copy_from_slice(&[0, 0, 0, 1]);
        d[0x0c..0x0e].copy_from_slice(&[0xff, 0xff]);
        d[0x90..0x94].copy_from_slice(b"baob");
        d[0x94..0x96].copy_from_slice(&2i16.to_be_bytes());
        d[0x9c..0xa0].copy_from_slice(b"wass");
        d[0xbc..0xc0].copy_from_slice(&[0xff; 4]);
        d[0x14c..0x150].copy_from_slice(b"vowa");
        d[0x150..0x154].copy_from_slice(&[0xff; 4]);
        let m = TflMonsterDefinition::from_bytes(FourCC(*b"warr"), &d).unwrap();
        assert_eq!(m.collection_tag, FourCC(*b"warr"));
        assert_eq!(&m.sequences[..3], &[0, 1, -1]);
        assert_eq!(m.object_tag, FourCC(*b"baob"));
        assert_eq!(m.attack_count, 2);
        assert_eq!(m.attack_projectiles[..2], [Some(FourCC(*b"wass")), None]);
        assert_eq!(m.sounds[..2], [Some(FourCC(*b"vowa")), None]);
        assert!(TflMonsterDefinition::from_bytes(FourCC(*b"warr"), &[0; 1024]).is_err());
    }
}
