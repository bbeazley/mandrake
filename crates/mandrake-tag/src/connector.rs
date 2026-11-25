use byteorder::{BigEndian, ReadBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::io::Cursor;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConnectorTag {
    pub flags: u32,
    pub collection_reference_tag: FourCC,
    pub sequence_index: i16,
    pub height_fraction: i16,
    pub segment_length: i16,
    pub damaged_sequence_index: i16,
}

impl ConnectorTag {
    pub const SIZE: usize = 32;
    pub const WORLD_ONE: f32 = 512.0;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 16 {
            return Err(M2Error::TagParse(format!(
                "Connector tag data too short: expected >= 16 bytes, got {}",
                data.len()
            )));
        }
        let mut cursor = Cursor::new(data);
        let flags = cursor.read_u32::<BigEndian>()?;
        let mut cc = [0u8; 4];
        std::io::Read::read_exact(&mut cursor, &mut cc)?;
        let sequence_index = cursor.read_i16::<BigEndian>()?;
        let height_fraction = cursor.read_i16::<BigEndian>()?;
        let segment_length = cursor.read_i16::<BigEndian>()?;
        let damaged_sequence_index = cursor.read_i16::<BigEndian>()?;
        Ok(Self {
            flags,
            collection_reference_tag: FourCC::new(cc),
            sequence_index,
            height_fraction,
            segment_length,
            damaged_sequence_index,
        })
    }

    pub fn height_fraction_f32(&self) -> f32 {
        self.height_fraction as f32 / 256.0
    }

    pub fn segment_length_world(&self) -> f32 {
        if self.segment_length > 0 {
            self.segment_length as f32 / Self::WORLD_ONE
        } else {
            1.0
        }
    }

    pub fn segment_count(&self, distance: f32) -> usize {
        ((distance / self.segment_length_world()).floor() as usize).max(1)
    }

    pub fn resolve_collection_tag(&self, archive: &crate::TagArchive) -> FourCC {
        if let Some(entry) =
            archive.find_tag_by_id(FourCC::new(*b"core"), self.collection_reference_tag)
            && let Ok(core_bytes) = archive.get_tag_data(entry)
            && core_bytes.len() >= 4
        {
            let mut cc = [0u8; 4];
            cc.copy_from_slice(&core_bytes[0..4]);
            if cc != [0; 4] && cc != [0xFF; 4] {
                return FourCC::new(cc);
            }
        }
        self.collection_reference_tag
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeshConnector {
    pub marker_identifiers: Vec<i16>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MeshConnectorList {
    pub connector_tag: Option<FourCC>,
    pub connectors: Vec<MeshConnector>,
}

impl MeshConnectorList {
    pub const TAG_OFFSET: usize = 0x48;
    pub const COUNT_OFFSET: usize = 0x11C;
    pub const RECORD_SIZE: usize = 64;
    pub const MAX_POINTS: usize = 31;

    pub fn from_mesh_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < crate::MeshTag::HEADER_SIZE {
            return Err(M2Error::TagParse(format!(
                "Mesh tag data too short for connector list: {} bytes",
                data.len()
            )));
        }
        let mut cc = [0u8; 4];
        cc.copy_from_slice(&data[Self::TAG_OFFSET..Self::TAG_OFFSET + 4]);
        let connector_tag = if cc != [0; 4] && cc != [0xFF; 4] {
            Some(FourCC::new(cc))
        } else {
            None
        };

        let mut cursor = Cursor::new(data);
        cursor.set_position(Self::COUNT_OFFSET as u64);
        let count = cursor.read_u32::<BigEndian>()? as usize;
        let offset = cursor.read_u32::<BigEndian>()? as usize;

        let mut connectors = Vec::new();
        if count > 0
            && count < 4096
            && let Some(range) = crate::parse::checked_table_range(
                crate::MeshTag::HEADER_SIZE,
                offset,
                count,
                Self::RECORD_SIZE,
                data.len(),
            )
        {
            for i in 0..count {
                let rec = &data[range.start + i * Self::RECORD_SIZE
                    ..range.start + (i + 1) * Self::RECORD_SIZE];
                let n = i16::from_be_bytes([rec[0x3E], rec[0x3F]]).clamp(0, Self::MAX_POINTS as i16)
                    as usize;
                let marker_identifiers = (0..n)
                    .map(|p| i16::from_be_bytes([rec[p * 2], rec[p * 2 + 1]]))
                    .collect();
                connectors.push(MeshConnector { marker_identifiers });
            }
        }

        Ok(Self {
            connector_tag,
            connectors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIFE: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, b's', b'c', b'o', b'b', 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0xff,
        0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    const SNOWY: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, b'8', b'9', b's', b's', 0x00, 0x09, 0x01, 0x00, 0x02, 0x00, 0x00,
        0x09, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn test_parse_real_wife_payload() {
        let c = ConnectorTag::from_bytes(&WIFE).unwrap();
        assert_eq!(c.flags, 0);
        assert_eq!(c.collection_reference_tag, FourCC::new(*b"scob"));
        assert_eq!(c.sequence_index, 0);
        assert_eq!(c.height_fraction, 256);
        assert_eq!(c.segment_length, 512);
        assert_eq!(c.damaged_sequence_index, -1);
        assert!((c.height_fraction_f32() - 1.0).abs() < 1e-6);
        assert!((c.segment_length_world() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_real_snowy_wire_payload() {
        let c = ConnectorTag::from_bytes(&SNOWY).unwrap();
        assert_eq!(c.collection_reference_tag, FourCC::new(*b"89ss"));
        assert_eq!(c.sequence_index, 9);
        assert_eq!(c.damaged_sequence_index, 9);
    }

    #[test]
    fn test_segment_count_matches_engine_division() {
        let c = ConnectorTag::from_bytes(&WIFE).unwrap();
        assert_eq!(c.segment_count(0.2), 1);
        assert_eq!(c.segment_count(1.0), 1);
        assert_eq!(c.segment_count(3.9), 3);
        assert_eq!(c.segment_count(4.0), 4);
    }

    #[test]
    fn test_too_short_rejected() {
        assert!(ConnectorTag::from_bytes(&WIFE[..8]).is_err());
    }

    #[test]
    fn test_mesh_connector_list_respects_count_and_ignores_garbage() {
        let mut data = vec![0u8; 1024 + 128];
        data[0x48..0x4C].copy_from_slice(b"wife");
        data[0x11C..0x120].copy_from_slice(&2u32.to_be_bytes());
        data[0x120..0x124].copy_from_slice(&0u32.to_be_bytes());
        data[0x124..0x128].copy_from_slice(&128u32.to_be_bytes());
        let r0 = 1024;
        for (i, id) in [10000i16, 10001, 10002].iter().enumerate() {
            data[r0 + i * 2..r0 + i * 2 + 2].copy_from_slice(&id.to_be_bytes());
        }
        for i in 3..31 {
            data[r0 + i * 2..r0 + i * 2 + 2].copy_from_slice(&0x6060i16.to_be_bytes());
        }
        data[r0 + 0x3E..r0 + 0x40].copy_from_slice(&3i16.to_be_bytes());
        let r1 = 1024 + 64;
        for (i, id) in [10002i16, 10003, 10002].iter().enumerate() {
            data[r1 + i * 2..r1 + i * 2 + 2].copy_from_slice(&id.to_be_bytes());
        }
        data[r1 + 0x3E..r1 + 0x40].copy_from_slice(&3i16.to_be_bytes());

        let list = MeshConnectorList::from_mesh_bytes(&data).unwrap();
        assert_eq!(list.connector_tag, Some(FourCC::new(*b"wife")));
        assert_eq!(list.connectors.len(), 2);
        assert_eq!(
            list.connectors[0].marker_identifiers,
            vec![10000, 10001, 10002]
        );
        assert_eq!(
            list.connectors[1].marker_identifiers,
            vec![10002, 10003, 10002]
        );
    }

    #[test]
    fn test_mesh_without_connectors() {
        let mut data = vec![0u8; 1024];
        data[0x48..0x4C].copy_from_slice(&[0xFF; 4]);
        let list = MeshConnectorList::from_mesh_bytes(&data).unwrap();
        assert_eq!(list.connector_tag, None);
        assert!(list.connectors.is_empty());
    }
}
