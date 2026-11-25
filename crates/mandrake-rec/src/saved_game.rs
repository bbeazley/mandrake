//! A saved game: a `save` tag whose data is a film with a saved-game header block and snapshot.

use mandrake_common::M2Error;

use crate::binary::{be_i16, be_i32, c_bytes};
use crate::header::RecordingHeader;

pub const SAVED_GAME_HEADER_SIZE: usize = 0xa30;
pub const SNAPSHOT_WIDTH: u32 = 160;
pub const SNAPSHOT_HEIGHT: u32 = 120;
/// Stored decremented by 1 on disk (2 decodes to 32-bit pixel format 3).
pub const SNAPSHOT_FORMAT_32_BIT: i16 = 2;
pub const CONSOLE_LINES: usize = 10;
const CONSOLE_LINE_TEXT: usize = 0x100;
const CONSOLE_LINE_SIZE: usize = 2 + CONSOLE_LINE_TEXT;

/// One line of the console as it was saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    pub kind: i16,
    /// Up to the first NUL.
    pub text: Vec<u8>,
}

/// The saved game header block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedGameHeader {
    pub snapshot_width: i16,
    pub snapshot_height: i16,
    pub snapshot_format: i16,
    pub observer_x: i32,
    pub observer_y: i32,
    pub observer_facing: i16,
    pub observer_zoom: i32,
    pub console: Vec<ConsoleLine>,
    pub console_count: i16,
}

impl SavedGameHeader {
    pub fn parse(block: &[u8; SAVED_GAME_HEADER_SIZE]) -> Self {
        let console = (0..CONSOLE_LINES)
            .map(|i| {
                let at = 0x18 + i * CONSOLE_LINE_SIZE;
                let raw = &block[at + 2..at + CONSOLE_LINE_SIZE];
                ConsoleLine {
                    kind: be_i16(block, at),
                    text: c_bytes(raw).to_vec(),
                }
            })
            .collect();
        Self {
            snapshot_width: be_i16(block, 0),
            snapshot_height: be_i16(block, 2),
            observer_x: be_i32(block, 4),
            observer_y: be_i32(block, 8),
            observer_facing: be_i16(block, 0x0c),
            snapshot_format: be_i16(block, 0x10),
            observer_zoom: be_i32(block, 0x14),
            console,
            console_count: be_i16(block, 0xa2c),
        }
    }

    pub fn to_bytes(&self) -> [u8; SAVED_GAME_HEADER_SIZE] {
        let mut b = [0u8; SAVED_GAME_HEADER_SIZE];
        b[0..2].copy_from_slice(&self.snapshot_width.to_be_bytes());
        b[2..4].copy_from_slice(&self.snapshot_height.to_be_bytes());
        b[4..8].copy_from_slice(&self.observer_x.to_be_bytes());
        b[8..12].copy_from_slice(&self.observer_y.to_be_bytes());
        b[0x0c..0x0e].copy_from_slice(&self.observer_facing.to_be_bytes());
        b[0x10..0x12].copy_from_slice(&self.snapshot_format.to_be_bytes());
        b[0x14..0x18].copy_from_slice(&self.observer_zoom.to_be_bytes());
        for (i, line) in self.console.iter().take(CONSOLE_LINES).enumerate() {
            let at = 0x18 + i * CONSOLE_LINE_SIZE;
            b[at..at + 2].copy_from_slice(&line.kind.to_be_bytes());
            let len = line.text.len().min(CONSOLE_LINE_TEXT - 1);
            b[at + 2..at + 2 + len].copy_from_slice(&line.text[..len]);
        }
        b[0xa2c..0xa2e].copy_from_slice(&self.console_count.to_be_bytes());
        b
    }

    pub fn pixel_format(&self) -> i16 {
        match self.snapshot_format {
            0 => 1,
            1 => 2,
            2 => 3,
            f => f,
        }
    }

    pub fn bytes_per_pixel(&self) -> usize {
        if self.pixel_format() == 3 { 4 } else { 2 }
    }
}

/// A snapshot, as RGBA rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Snapshot {
    /// Big-endian opaque `0xAARRGGBB` words, matching the saved-game pixel format.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [0xff, p[0], p[1], p[2]])
            .collect()
    }
}

/// What a saved game holds beyond its film.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedGame {
    pub recording: RecordingHeader,
    pub header: SavedGameHeader,
    pub snapshot: Option<Snapshot>,
}

impl SavedGame {
    /// Reads a saved game file; a missing or unreadable snapshot is `None`, not an error.
    pub fn from_bytes(file: &[u8]) -> Result<Self, M2Error> {
        let (recording, _) = RecordingHeader::read(&mut std::io::Cursor::new(file))?;
        if !recording.is_saved_game() {
            return Err(M2Error::ReplayParse(format!(
                "not a saved game: tag group {}",
                recording.tag_type
            )));
        }
        let data = crate::header::data_offset(file);
        let word = |o: usize| -> Option<u32> {
            let start = data.checked_add(o)?;
            let end = start.checked_add(4)?;
            let w = file.get(start..end)?;
            Some(u32::from_be_bytes([w[0], w[1], w[2], w[3]]))
        };
        let at = word(0x0c).ok_or_else(|| M2Error::ReplayParse("short header".into()))? as usize;
        let start = data
            .checked_add(at)
            .ok_or_else(|| M2Error::ReplayParse("couldn't read save game header".into()))?;
        let end = start
            .checked_add(SAVED_GAME_HEADER_SIZE)
            .ok_or_else(|| M2Error::ReplayParse("couldn't read save game header".into()))?;
        let block: &[u8; SAVED_GAME_HEADER_SIZE] = file
            .get(start..end)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| M2Error::ReplayParse("couldn't read save game header".into()))?;
        let header = SavedGameHeader::parse(block);
        let snapshot = read_snapshot(&header, file.get(end..));
        Ok(Self {
            recording,
            header,
            snapshot,
        })
    }
}

fn read_snapshot(header: &SavedGameHeader, pixels: Option<&[u8]>) -> Option<Snapshot> {
    let (w, h) = (header.snapshot_width, header.snapshot_height);
    if w <= 0 || h <= 0 {
        return None;
    }
    let (w, h) = (w as usize, h as usize);
    let bpp = header.bytes_per_pixel();
    let pixel_bytes = w.checked_mul(h)?.checked_mul(bpp)?;
    let rgba_len = w.checked_mul(h)?.checked_mul(4)?;
    let pixels = pixels?.get(..pixel_bytes)?;
    let mut rgba = Vec::with_capacity(rgba_len);
    for p in pixels.chunks_exact(bpp) {
        if bpp == 4 {
            rgba.extend_from_slice(&[p[1], p[2], p[3], 0xff]);
        } else {
            let v = u16::from_be_bytes([p[0], p[1]]);
            let c = |s: u16| {
                let x = (v >> s & 0x1f) as u8;
                x << 3 | x >> 2
            };
            rgba.extend_from_slice(&[c(10), c(5), c(0), 0xff]);
        }
    }
    Some(Snapshot {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_block_reads_its_fields() {
        let mut b = [0u8; SAVED_GAME_HEADER_SIZE];
        b[0..4].copy_from_slice(&[0, 0xa0, 0, 0x78]);
        b[4..8].copy_from_slice(&0x6d4fi32.to_be_bytes());
        b[8..12].copy_from_slice(&0x8cccu32.to_be_bytes());
        b[12..14].copy_from_slice(&[0xbd, 0x6f]);
        b[0x10..0x12].copy_from_slice(&2i16.to_be_bytes());
        b[0x14..0x18].copy_from_slice(&[0xff, 0xff, 0, 1]);
        b[0x18..0x1a].copy_from_slice(&(-1i16).to_be_bytes());
        b[0x1a..0x24].copy_from_slice(b"Casualty.\r");
        b[0xa2c..0xa2e].copy_from_slice(&5i16.to_be_bytes());
        let h = SavedGameHeader::parse(&b);
        assert_eq!((h.snapshot_width, h.snapshot_height), (160, 120));
        assert_eq!((h.observer_x, h.observer_y), (0x6d4f, 0x8ccc));
        assert_eq!(h.observer_facing, 0xbd6fu16 as i16);
        assert_eq!(h.pixel_format(), 3);
        assert_eq!(h.bytes_per_pixel(), 4);
        assert_eq!(h.observer_zoom, -65535);
        assert_eq!(h.console.len(), CONSOLE_LINES);
        assert_eq!(h.console[0].kind, -1);
        assert_eq!(h.console[0].text, b"Casualty.\r");
        assert_eq!(h.console_count, 5);
        assert_eq!(h.to_bytes(), b, "written as read");
    }

    #[test]
    fn overflowing_saved_game_header_offset_is_an_error() {
        let mut f = vec![0u8; 64 + 2600];
        f[36..40].copy_from_slice(b"save");
        f[44..48].copy_from_slice(&64u32.to_be_bytes());
        f[64..66].copy_from_slice(&3u16.to_be_bytes());
        f[64 + 0x04..64 + 0x08].copy_from_slice(&2600u32.to_be_bytes());
        f[64 + 0x0c..64 + 0x10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            SavedGame::from_bytes(&f),
            Err(M2Error::ReplayParse(_))
        ));

        let mut huge = SavedGameHeader::parse(&[0u8; SAVED_GAME_HEADER_SIZE]);
        huge.snapshot_width = i16::MAX;
        huge.snapshot_height = i16::MAX;
        huge.snapshot_format = SNAPSHOT_FORMAT_32_BIT;
        assert!(read_snapshot(&huge, Some(&[0u8; 64])).is_none());
    }
}
