use crate::header::decode_c_string;
use crate::pix0r1::{self, Pix0r1Image};
use mandrake_common::{FourCC, M2Error};

pub const DETAIL_MAP_ENTRY_COUNT: usize = 128;
pub const DETAIL_MAP_ENTRY_SIZE: usize = 64;
pub const DETAIL_MAP_HEADER_SIZE: usize = 0x100;

fn be_i32(data: &[u8], at: usize) -> Result<i32, M2Error> {
    data.get(at..at + 4)
        .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| M2Error::TagParse(format!("dmap: no long at {at:#x}")))
}

pub fn rle_expand(data: &[u8], offset: usize, len: usize) -> Result<Vec<u8>, M2Error> {
    let max_output = (data.len().saturating_sub(offset) / 2).saturating_mul(256);
    if len > max_output {
        return Err(M2Error::TagParse(format!(
            "dmap: RLE stream at {offset:#x} cannot produce {len} bytes"
        )));
    }
    let mut out = Vec::with_capacity(len);
    let mut at = offset;
    while out.len() < len {
        let end = at.checked_add(2).ok_or_else(|| {
            M2Error::TagParse(format!("dmap: RLE stream at {offset:#x} overflow"))
        })?;
        let pair = data.get(at..end).ok_or_else(|| {
            M2Error::TagParse(format!("dmap: RLE stream at {offset:#x} ends early"))
        })?;
        at = end;
        let n = (pair[1] as usize + 1).min(len - out.len());
        out.extend(std::iter::repeat_n(pair[0], n));
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailMapEntry {
    pub texture: Option<FourCC>,
    pub repeat: u8,
    pub color: [u8; 3],
    pub name: String,
    pub raw: [u8; DETAIL_MAP_ENTRY_SIZE],
}

impl DetailMapEntry {
    pub fn from_bytes(raw: &[u8; DETAIL_MAP_ENTRY_SIZE]) -> Self {
        let id = [raw[0], raw[1], raw[2], raw[3]];
        Self {
            texture: (id != [0xff; 4]).then_some(FourCC::new(id)),
            repeat: raw[4],
            color: [raw[5], raw[6], raw[7]],
            name: decode_c_string(&raw[0x20..0x40]),
            raw: *raw,
        }
    }

    pub fn texture_scale(&self, width: u32, height: u32) -> [f32; 2] {
        let n = (self.repeat as u32 + 1) as f32;
        [n / width as f32, n / height as f32]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailMap {
    pub width: u32,
    pub height: u32,
    pub header: [i32; 8],
    pub cell_entries: Vec<u8>,
    pub triangle_entries: Vec<u8>,
    pub entries: Vec<DetailMapEntry>,
}

impl DetailMap {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < DETAIL_MAP_HEADER_SIZE {
            return Err(M2Error::TagParse("dmap: shorter than its header".into()));
        }
        let mut header = [0i32; 8];
        for (i, v) in header.iter_mut().enumerate() {
            *v = be_i32(data, i * 4)?;
        }
        let (w, h) = (header[0], header[1]);
        if w < 0 || h < 0 {
            return Err(M2Error::TagParse(format!("dmap: size {w} x {h}")));
        }
        let offset =
            |v: i32| usize::try_from(v).map_err(|_| M2Error::TagParse(format!("dmap: offset {v}")));
        let table_at = offset(header[6])?;
        let table_end = table_at
            .checked_add(DETAIL_MAP_ENTRY_COUNT * DETAIL_MAP_ENTRY_SIZE)
            .ok_or_else(|| M2Error::TagParse("dmap: entry table offset overflow".into()))?;
        let table = data
            .get(table_at..table_end)
            .ok_or_else(|| M2Error::TagParse("dmap: entry table past the end".into()))?;
        let entries = table
            .as_chunks::<DETAIL_MAP_ENTRY_SIZE>()
            .0
            .iter()
            .map(DetailMapEntry::from_bytes)
            .collect();
        let cells = (w as usize)
            .checked_mul(h as usize)
            .ok_or_else(|| M2Error::TagParse(format!("dmap: cell count overflow for {w} x {h}")))?;
        let triangle_len = cells
            .checked_mul(2)
            .ok_or_else(|| M2Error::TagParse("dmap: triangle table size overflow".into()))?;
        let cell_entries = rle_expand(data, offset(header[2])?, cells)?;
        let triangle_entries = rle_expand(data, offset(header[4])?, triangle_len)?;
        Ok(Self {
            width: w as u32,
            height: h as u32,
            header,
            cell_entries,
            triangle_entries,
            entries,
        })
    }

    pub fn matches_mesh(&self, submesh_w: u16, submesh_h: u16) -> bool {
        self.width as i32 == (submesh_w as i16 as i32) << 5
            && self.height as i32 == (submesh_h as i16 as i32) << 5
    }

    pub fn textures(&self) -> Vec<FourCC> {
        let mut out: Vec<FourCC> = Vec::new();
        for t in self.entries.iter().filter_map(|e| e.texture) {
            if !out.contains(&t) {
                out.push(t);
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailTexture {
    pub header: [u8; 16],
    pub stream: Vec<u8>,
}

impl DetailTexture {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        let size = data
            .get(0x10..0x14)
            .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| M2Error::TagParse("dtex: shorter than its header".into()))?;
        let stream = usize::try_from(size)
            .ok()
            .and_then(|n| data.get(0x14..0x14 + n))
            .ok_or_else(|| M2Error::TagParse(format!("dtex: {size} bytes past the end")))?;
        let mut header = [0u8; 16];
        header.copy_from_slice(&data[..16]);
        Ok(Self {
            header,
            stream: stream.to_vec(),
        })
    }

    pub fn decode(&self) -> Result<Pix0r1Image, M2Error> {
        pix0r1::decode(&self.stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_runs_and_overshoot() {
        let data = [9, 9, 5, 2, 7, 0, 1, 200];
        assert_eq!(rle_expand(&data, 2, 4).unwrap(), [5, 5, 5, 7]);
        assert_eq!(
            rle_expand(&data, 6, 3).unwrap(),
            [1, 1, 1],
            "a long run is cut"
        );
        assert!(rle_expand(&data, 2, 300).is_err(), "ends early");
    }

    #[test]
    fn small_dmap() {
        let mut d = vec![0u8; 0x100 + 0x2000];
        let longs: [i32; 8] = [2, 1, 0x2100, 0, 0x2104, 0, 0x100, 0];
        for (i, v) in longs.iter().enumerate() {
            d[i * 4..i * 4 + 4].copy_from_slice(&(*v).to_be_bytes());
        }
        for e in 0..128 {
            d[0x100 + e * 64..0x104 + e * 64].copy_from_slice(&[0xff; 4]);
        }
        d[0x140..0x148].copy_from_slice(b"dirt\x7f\x01\x02\x03");
        d[0x160..0x164].copy_from_slice(b"path");
        d.extend([1, 0, 0, 0, 3, 3]);
        let m = DetailMap::from_bytes(&d).unwrap();
        assert_eq!((m.width, m.height), (2, 1));
        assert_eq!(m.cell_entries, [1, 0]);
        assert_eq!(m.triangle_entries, [3, 3, 3, 3]);
        assert_eq!(m.entries[0].texture, None);
        let e = &m.entries[1];
        assert_eq!(e.texture, Some(FourCC::new(*b"dirt")));
        assert_eq!(
            (e.repeat, e.color, e.name.as_str()),
            (0x7f, [1, 2, 3], "path")
        );
        assert_eq!(e.texture_scale(512, 256), [0.25, 0.5]);
        assert_eq!(m.textures(), [FourCC::new(*b"dirt")]);
        assert!(!m.matches_mesh(1, 1));
    }
}
