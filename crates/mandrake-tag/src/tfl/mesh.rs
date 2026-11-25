use mandrake_common::{FourCC, M2Error};

use crate::mesh::{MarkerEntry, MarkerPaletteEntry, MeshTag, read_marker_palettes, read_markers};

pub const TFL_MESH_VERSION: u16 = 7;
pub const TFL_SUBMESH_CELLS: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TflCell {
    pub physical_height: i16,
    pub word_2: u16,
    pub flags: u8,
    pub byte_5: u8,
    pub first_object_index: i16,
}

impl TflCell {
    pub const SIZE: usize = 8;

    fn parse(b: &[u8]) -> Self {
        Self {
            physical_height: i16::from_be_bytes([b[0], b[1]]),
            word_2: u16::from_be_bytes([b[2], b[3]]),
            flags: b[4],
            byte_5: b[5],
            first_object_index: i16::from_be_bytes([b[6], b[7]]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TflMesh {
    pub landscape_tag: FourCC,
    pub media_tag: Option<FourCC>,
    pub submesh_w: u16,
    pub submesh_h: u16,
    pub cell_width: u32,
    pub cell_height: u32,
    pub cells: Vec<TflCell>,
    pub palettes: Vec<MarkerPaletteEntry>,
    pub markers: Vec<MarkerEntry>,
    pub ambient_sound_tag: Option<FourCC>,
    pub map_action_count: u32,
    pub string_list_tag: Option<FourCC>,
    pub postgame_tag: Option<FourCC>,
    pub pregame_tag: Option<FourCC>,
    pub overhead_tag: Option<FourCC>,
    pub next_mesh_tag: Option<FourCC>,
}

use crate::parse::optional_fourcc as tag_at;

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(d[o..o + 4].try_into().expect("in the header"))
}

impl TflMesh {
    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        let err = |m: String| Err(M2Error::TagParse(format!("TFL mesh: {m}")));
        if data.len() < MeshTag::HEADER_SIZE {
            return err(format!("{} bytes, shorter than the header", data.len()));
        }
        let submesh_w = u16::from_be_bytes([data[8], data[9]]);
        let submesh_h = u16::from_be_bytes([data[10], data[11]]);
        let cell_width = submesh_w as u32 * TFL_SUBMESH_CELLS;
        let cell_height = submesh_h as u32 * TFL_SUBMESH_CELLS;
        let cells_offset = u32_at(data, 0x0c) as usize;
        let cells_size = u32_at(data, 0x10) as usize;
        let count = (cell_width as usize)
            .checked_mul(cell_height as usize)
            .ok_or_else(|| {
                M2Error::TagParse(format!(
                    "TFL mesh: cell count overflow for {cell_width}x{cell_height}"
                ))
            })?;
        let expected_bytes = count.checked_mul(TflCell::SIZE);
        let range = crate::parse::checked_table_range(
            MeshTag::HEADER_SIZE,
            cells_offset,
            count,
            TflCell::SIZE,
            data.len(),
        );
        if expected_bytes != Some(cells_size) || range.is_none() {
            return err(format!(
                "cells: {cells_size} bytes at {}+{cells_offset} in a {}-byte tag for {cell_width}x{cell_height} 8-byte cells",
                MeshTag::HEADER_SIZE,
                data.len()
            ));
        }
        let range = range.unwrap();
        let cells = data[range]
            .as_chunks::<{ TflCell::SIZE }>()
            .0
            .iter()
            .map(|c| TflCell::parse(c))
            .collect();
        let block = |at: usize, entry: usize, what: &str| -> Result<(u32, u32), M2Error> {
            let (n, off, size) = (u32_at(data, at), u32_at(data, at + 4), u32_at(data, at + 8));
            let expected = (n as usize).checked_mul(entry);
            let fits = crate::parse::checked_table_range(
                MeshTag::HEADER_SIZE,
                off as usize,
                n as usize,
                entry,
                data.len(),
            )
            .is_some();
            if expected != Some(size as usize) || !fits {
                return Err(M2Error::TagParse(format!(
                    "TFL mesh: {what}: {n} entries, {size} bytes at {off} in a {}-byte tag",
                    data.len()
                )));
            }
            Ok((n, off))
        };
        let (palette_count, palette_offset) =
            block(0x24, MarkerPaletteEntry::SIZE, "marker palette")?;
        let (marker_count, marker_offset) = block(0x34, MarkerEntry::SIZE, "markers")?;
        let palettes = read_marker_palettes(data, palette_count, palette_offset)?;
        let markers = read_markers(data, marker_count, marker_offset)?;
        Ok(Self {
            landscape_tag: FourCC(data[0..4].try_into().expect("4 bytes")),
            media_tag: tag_at(data, 0x04),
            submesh_w,
            submesh_h,
            cell_width,
            cell_height,
            cells,
            palettes,
            markers,
            ambient_sound_tag: tag_at(data, 0x7c),
            map_action_count: u32_at(data, 0x80),
            string_list_tag: tag_at(data, 0x8c),
            postgame_tag: tag_at(data, 0x90),
            pregame_tag: tag_at(data, 0x94),
            overhead_tag: tag_at(data, 0x98),
            next_mesh_tag: tag_at(data, 0x9c),
        })
    }

    pub fn cell_at(&self, x: u32, y: u32) -> Option<&TflCell> {
        (x < self.cell_width && y < self.cell_height)
            .then(|| &self.cells[(y * self.cell_width + x) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Vec<u8> {
        let cells = 16 * 16 * 8;
        let mut d = vec![0u8; 1024 + cells + 32];
        d[0..4].copy_from_slice(b"00tm");
        d[8..12].copy_from_slice(&[0, 1, 0, 1]);
        d[0x10..0x14].copy_from_slice(&(cells as u32).to_be_bytes());
        d[0x24..0x28].copy_from_slice(&1u32.to_be_bytes());
        d[0x28..0x2c].copy_from_slice(&(cells as u32).to_be_bytes());
        d[0x2c..0x30].copy_from_slice(&32u32.to_be_bytes());
        d[0x98..0x9c].copy_from_slice(b"00om");
        d[1024..1032].copy_from_slice(&[0x08, 0, 0, 0x88, 0, 0xff, 0xff, 0xff]);
        d
    }

    #[test]
    fn a_version_7_mesh_reads() {
        let m = TflMesh::from_bytes(&tiny()).unwrap();
        assert_eq!((m.cell_width, m.cell_height, m.cells.len()), (16, 16, 256));
        let c = m.cell_at(0, 0).unwrap();
        assert_eq!((c.physical_height, c.word_2, c.byte_5), (0x800, 0x88, 0xff));
        assert_eq!(c.first_object_index, -1);
        assert_eq!(m.palettes.len(), 1);
        assert_eq!(m.overhead_tag, Some(FourCC(*b"00om")));
        assert_eq!(m.string_list_tag, None);
    }

    #[test]
    fn a_short_cell_block_is_an_error() {
        let mut d = tiny();
        d.truncate(1024 + 100);
        assert!(TflMesh::from_bytes(&d).is_err());
        assert!(MeshTag::from_bytes(&tiny()).is_err());
    }
}
