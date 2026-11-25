use byteorder::{BigEndian, ReadBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::io::Cursor;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SceneryTag {
    pub flags: u32,
    #[serde(rename = "object_tag")]
    pub collection_tag: FourCC,
    #[serde(rename = "sound_tag")]
    pub object_tag: Option<FourCC>,
    pub projectile_tag: Option<FourCC>,
    pub sequence_index: i16,
    pub destroyed_sequence_index: i16,
    pub raw_data: Vec<u8>,
}

impl SceneryTag {
    pub const SIZE: usize = 128;

    pub const FLAG_DESTRUCTIBLE: u32 = 0x0080;
    pub const FLAG_MARKS_CELLS_IMPASSABLE: u32 = 0x0080;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 12 {
            return Err(M2Error::TagParse(format!(
                "Scenery tag data too short: expected >= 12 bytes, got {}",
                data.len()
            )));
        }

        let mut cursor = Cursor::new(data);

        let flags = cursor.read_u32::<BigEndian>()?;

        let mut obj_bytes = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut obj_bytes)?;
        let collection_tag = FourCC::new(obj_bytes);

        let mut snd_bytes = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut snd_bytes)?;
        let object_tag = if Self::is_valid_fourcc(&snd_bytes) {
            Some(FourCC::new(snd_bytes))
        } else {
            None
        };

        let mut projectile_tag = None;
        if data.len() >= 32 {
            cursor.set_position(28);
            let mut proj_bytes = [0u8; 4];
            if std::io::Read::read_exact(&mut cursor, &mut proj_bytes).is_ok()
                && Self::is_valid_fourcc(&proj_bytes)
            {
                projectile_tag = Some(FourCC::new(proj_bytes));
            }
        }

        let mut sequence_index = -1;
        let mut destroyed_sequence_index = -1;
        if data.len() >= 42 {
            cursor.set_position(40);
            sequence_index = cursor.read_i16::<BigEndian>()?;
        }
        if data.len() >= 44 {
            cursor.set_position(42);
            destroyed_sequence_index = cursor.read_i16::<BigEndian>()?;
        }

        Ok(Self {
            flags,
            collection_tag,
            object_tag,
            projectile_tag,
            sequence_index,
            destroyed_sequence_index,
            raw_data: data.to_vec(),
        })
    }

    pub fn resolve_collection_tag(&self, archive: &crate::TagArchive) -> FourCC {
        if let Some(entry) = archive.find_tag_by_id(FourCC::new(*b"core"), self.collection_tag)
            && let Ok(core_bytes) = archive.get_tag_data(entry)
            && core_bytes.len() >= 4
        {
            let mut cc = [0u8; 4];
            cc.copy_from_slice(&core_bytes[0..4]);
            if Self::is_valid_fourcc(&cc) {
                return FourCC::new(cc);
            }
        }
        self.collection_tag
    }

    pub fn object_definition_tag(&self) -> Option<FourCC> {
        self.object_tag
    }

    pub fn marks_cells_impassable(&self) -> bool {
        self.flags & Self::FLAG_MARKS_CELLS_IMPASSABLE != 0
    }

    pub fn is_destructible(&self) -> bool {
        (self.flags & Self::FLAG_DESTRUCTIBLE) != 0 || self.projectile_tag.is_some()
    }

    fn is_valid_fourcc(buf: &[u8; 4]) -> bool {
        if *buf == [0, 0, 0, 0] || *buf == [0xFF, 0xFF, 0xFF, 0xFF] {
            return false;
        }
        buf.iter().all(|&b| b.is_ascii_graphic() || b == b' ')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenery_deserializer_synthetic() {
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(&0x00000080u32.to_be_bytes());
        data[4..8].copy_from_slice(b"l1_s");
        data[8..12].copy_from_slice(b"l1so");
        data[28..32].copy_from_slice(b"babi");
        data[40..42].copy_from_slice(&8i16.to_be_bytes());

        let scen = SceneryTag::from_bytes(&data).expect("Should parse synthetic scenery tag");
        assert_eq!(scen.flags, 0x80);
        assert_eq!(scen.collection_tag, "l1_s");
        assert_eq!(scen.object_tag, Some(FourCC::new(*b"l1so")));
        assert_eq!(scen.projectile_tag, Some(FourCC::new(*b"babi")));
        assert_eq!(scen.sequence_index, 8);
        assert!(scen.is_destructible());
        assert!(scen.marks_cells_impassable());
    }
}
