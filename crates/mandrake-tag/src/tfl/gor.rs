use std::io::{Read, Seek, SeekFrom};

use mandrake_common::{FourCC, M2Error};

use crate::header::decode_c_string;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GorHeader {
    pub r#type: i16,
    pub version: u16,
    pub name: String,
    pub checksum: u32,
    pub directory_offset: u32,
    pub tag_count: u16,
    pub entry_size: u16,
    pub flags: u16,
}

impl GorHeader {
    pub const SIZE: usize = 0x40;
    pub const DATA_START: u32 = 0x80;

    pub fn parse(b: &[u8]) -> Result<Self, M2Error> {
        if b.len() < Self::SIZE {
            return Err(M2Error::TagParse(format!(
                ".gor header needs {} bytes, got {}",
                Self::SIZE,
                b.len()
            )));
        }
        let u16_at = |o: usize| u16::from_be_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        Ok(Self {
            r#type: u16_at(0) as i16,
            version: u16_at(2),
            name: decode_c_string(&b[0x04..0x24]),
            checksum: u32_at(0x24),
            directory_offset: u32_at(0x28),
            tag_count: u16_at(0x2c),
            entry_size: u16_at(0x2e),
            flags: u16_at(0x30),
        })
    }

    pub fn matches_length(&self, len: u64) -> bool {
        self.entry_size as usize == GorTagHeader::SIZE
            && self.directory_offset as u64 >= Self::SIZE as u64
            && self.directory_offset as u64 + self.tag_count as u64 * GorTagHeader::SIZE as u64
                == len
    }

    pub fn holds_its_data(&self, tags: &[GorTagHeader]) -> bool {
        tags.iter().all(|t| {
            t.offset >= Self::DATA_START
                && t.offset as u64 + t.size as u64 <= self.directory_offset as u64
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GorTagHeader {
    pub name: String,
    pub name_bytes: [u8; 32],
    pub group: FourCC,
    pub id: FourCC,
    pub reserved: u16,
    pub version: u16,
    pub unknown_2c: u16,
    pub flags: u16,
    pub offset: u32,
    pub size: u32,
    pub user_data: u32,
}

impl GorTagHeader {
    pub const SIZE: usize = 64;
    pub const SIGNATURE: FourCC = FourCC(*b"myth");

    pub fn parse(b: &[u8]) -> Result<Self, M2Error> {
        if b.len() < Self::SIZE {
            return Err(M2Error::TagParse(format!(
                "TFL tag header needs 64 bytes, got {}",
                b.len()
            )));
        }
        if b[0x3c..0x40] != Self::SIGNATURE.0 {
            return Err(M2Error::InvalidMagic {
                expected: "myth".to_string(),
                found: String::from_utf8_lossy(&b[0x3c..0x40]).into_owned(),
            });
        }
        let u16_at = |o: usize| u16::from_be_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let four = |o: usize| FourCC([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        Ok(Self {
            name: decode_c_string(&b[..0x20]),
            name_bytes: b[..0x20].try_into().expect("32 bytes"),
            group: four(0x20),
            id: four(0x24),
            reserved: u16_at(0x28),
            version: u16_at(0x2a),
            unknown_2c: u16_at(0x2c),
            flags: u16_at(0x2e),
            offset: u32_at(0x30),
            size: u32_at(0x34),
            user_data: u32_at(0x38),
        })
    }
}

pub fn read_gor_directory<R: Read + Seek>(
    reader: &mut R,
) -> Result<Option<(GorHeader, Vec<GorTagHeader>)>, M2Error> {
    let len = reader.seek(SeekFrom::End(0))?;
    if len < GorHeader::SIZE as u64 {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(0))?;
    let mut head = [0u8; GorHeader::SIZE];
    reader.read_exact(&mut head)?;
    let header = GorHeader::parse(&head)?;
    if !header.matches_length(len) {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(header.directory_offset as u64))?;
    let mut dir = vec![0u8; header.tag_count as usize * GorTagHeader::SIZE];
    reader.read_exact(&mut dir)?;
    let tags = dir
        .as_chunks::<{ GorTagHeader::SIZE }>()
        .0
        .iter()
        .map(|e| GorTagHeader::parse(e))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some((header, tags)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    pub(crate) fn gor(tags: &[(&[u8; 4], &[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = vec![0u8; GorHeader::DATA_START as usize];
        let mut dir = Vec::new();
        for (group, id, data) in tags {
            let mut e = [0u8; 64];
            e[..4].copy_from_slice(&id[..]);
            e[0x20..0x24].copy_from_slice(&group[..]);
            e[0x24..0x28].copy_from_slice(&id[..]);
            e[0x2a..0x2c].copy_from_slice(&7u16.to_be_bytes());
            e[0x30..0x34].copy_from_slice(&(out.len() as u32).to_be_bytes());
            e[0x34..0x38].copy_from_slice(&(data.len() as u32).to_be_bytes());
            e[0x3c..0x40].copy_from_slice(b"myth");
            out.extend_from_slice(data);
            dir.extend_from_slice(&e);
        }
        let dir_off = out.len() as u32;
        out[0..2].copy_from_slice(&1u16.to_be_bytes());
        out[2..4].copy_from_slice(&1u16.to_be_bytes());
        out[4..14].copy_from_slice(b"public.gor");
        out[0x28..0x2c].copy_from_slice(&dir_off.to_be_bytes());
        out[0x2c..0x2e].copy_from_slice(&(tags.len() as u16).to_be_bytes());
        out[0x2e..0x30].copy_from_slice(&64u16.to_be_bytes());
        out.extend_from_slice(&dir);
        out
    }

    #[test]
    fn a_synthetic_gor_reads_back() {
        let bytes = gor(&[(b"mesh", b"00tm", b"cells"), (b"soun", b"0801", b"pcm")]);
        let (h, tags) = read_gor_directory(&mut Cursor::new(&bytes))
            .unwrap()
            .unwrap();
        assert_eq!(h.name, "public.gor");
        assert_eq!(h.tag_count, 2);
        assert!(h.holds_its_data(&tags));
        assert_eq!(tags[0].group, FourCC(*b"mesh"));
        assert_eq!(tags[0].version, 7);
        assert_eq!(tags[1].offset, 0x80 + 5);
    }

    #[test]
    fn a_short_or_untiled_file_is_not_a_gor() {
        let mut bytes = gor(&[(b"mesh", b"00tm", b"cells")]);
        bytes.push(0);
        assert!(
            read_gor_directory(&mut Cursor::new(&bytes))
                .unwrap()
                .is_none()
        );
        assert!(
            read_gor_directory(&mut Cursor::new(&[0u8; 8]))
                .unwrap()
                .is_none()
        );
    }
}
