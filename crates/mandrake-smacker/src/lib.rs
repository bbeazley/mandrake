//! Smacker (SMK2 and SMK4) decoder: video as palette-indexed 8-bit frames, audio as PCM.
//!
//! Usage:
//!
//! ```no_run
//! let bytes = std::fs::read("prologue.smk").unwrap();
//! let smk = mandrake_smacker::Smacker::open(&bytes).unwrap();
//! let mut dec = smk.decoder();
//! while let Some(frame) = dec.next_frame().unwrap() {
//!     let _rgba = dec.rgba();
//!     for _chunk in &frame.audio {}
//! }
//! ```

mod audio;
mod bits;
mod huffman;
mod video;

use std::fmt;

pub use audio::{AudioChunk, Samples};

use bits::BitReader;
use huffman::BigTree;

/// Decoding errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The data doesn't start with `SMK2` or `SMK4`.
    BadSignature,
    /// The file ends before a structure it announces.
    Truncated(&'static str),
    /// A structure is inconsistent.
    Corrupt(&'static str),
    /// A valid feature this decoder doesn't implement.
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadSignature => write!(f, "not a Smacker file"),
            Error::Truncated(what) => write!(f, "Smacker data truncated: {what}"),
            Error::Corrupt(what) => write!(f, "corrupt Smacker data: {what}"),
            Error::Unsupported(what) => write!(f, "unsupported Smacker feature: {what}"),
        }
    }
}

impl std::error::Error for Error {}

/// Smacker format revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Smk2,
    Smk4,
}

/// One audio track as the header describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioTrack {
    /// Track number, 0..7.
    pub index: u8,
    pub sample_rate: u32,
    pub channels: u8,
    pub bits: u8,
    /// Huffman DPCM compressed (otherwise raw PCM).
    pub compressed: bool,
    /// Bink audio compression (not supported by this decoder).
    pub bink: bool,
    /// Largest decompressed chunk, from the header.
    pub max_size: u32,
}

/// The file header.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub version: Version,
    pub width: u32,
    pub height: u32,
    /// Frames, not counting the ring frame.
    pub frame_count: u32,
    /// Frame duration in microseconds.
    pub frame_micros: u32,
    /// The file carries a ring frame (a copy of frame 0 for looping) after the last frame.
    pub ring_frame: bool,
    /// Rows are stored interlaced (display at twice the height).
    pub y_interlaced: bool,
    /// Rows are stored halved (display at twice the height).
    pub y_doubled: bool,
    pub audio_tracks: Vec<AudioTrack>,
}

impl Header {
    /// Frames per second.
    pub fn fps(&self) -> f64 {
        1_000_000.0 / self.frame_micros as f64
    }
}

const HEADER_SIZE: usize = 104;

/// Header flag bits.
const FLAG_RING: u32 = 1;
const FLAG_Y_INTERLACE: u32 = 2;
const FLAG_Y_DOUBLE: u32 = 4;

/// Audio rate word bits.
const AUDIO_COMPRESSED: u32 = 1 << 31;
const AUDIO_PRESENT: u32 = 1 << 30;
const AUDIO_16BIT: u32 = 1 << 29;
const AUDIO_STEREO: u32 = 1 << 28;
const AUDIO_BINK: u32 = 3 << 26;

/// One frame's place in the file.
#[derive(Debug, Clone, Copy)]
struct FrameEntry {
    offset: usize,
    size: usize,
    keyframe: bool,
    /// Bit 0: palette record; bits 1..7: audio for tracks 0..6.
    kind: u8,
}

/// An opened Smacker file.
#[derive(Debug, Clone)]
pub struct Smacker<'a> {
    data: &'a [u8],
    header: Header,
    frames: Vec<FrameEntry>,
    trees: [BigTree; 4],
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

impl<'a> Smacker<'a> {
    /// Parses the header, frame tables and Huffman trees. The bytes are borrowed.
    pub fn open(data: &'a [u8]) -> Result<Smacker<'a>, Error> {
        if data.len() < HEADER_SIZE {
            return Err(Error::Truncated("header"));
        }
        let version = match &data[0..4] {
            b"SMK2" => Version::Smk2,
            b"SMK4" => Version::Smk4,
            _ => return Err(Error::BadSignature),
        };
        let width = u32_at(data, 4);
        let height = u32_at(data, 8);
        let frame_count = u32_at(data, 12);
        let rate = u32_at(data, 16) as i32;
        let flags = u32_at(data, 20);
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(Error::Corrupt("frame dimensions"));
        }
        let frame_micros = match rate {
            r if r > 0 => (r as u32).saturating_mul(1000),
            r if r < 0 => r.unsigned_abs().saturating_mul(10),
            _ => 100_000,
        };
        let mut audio_tracks = Vec::new();
        for i in 0..7 {
            let size = u32_at(data, 24 + 4 * i);
            let word = u32_at(data, 72 + 4 * i);
            if word & AUDIO_PRESENT == 0 {
                continue;
            }
            audio_tracks.push(AudioTrack {
                index: i as u8,
                sample_rate: word & 0x00ff_ffff,
                channels: if word & AUDIO_STEREO != 0 { 2 } else { 1 },
                bits: if word & AUDIO_16BIT != 0 { 16 } else { 8 },
                compressed: word & AUDIO_COMPRESSED != 0,
                bink: word & AUDIO_BINK != 0,
                max_size: size,
            });
        }
        let trees_size = u32_at(data, 52) as usize;
        let ring_frame = flags & FLAG_RING != 0;
        let n = (frame_count as usize)
            .checked_add(ring_frame as usize)
            .ok_or(Error::Corrupt("frame count overflow"))?;
        let sizes_at = HEADER_SIZE;
        let kinds_at = n
            .checked_mul(4)
            .and_then(|s| sizes_at.checked_add(s))
            .ok_or(Error::Corrupt("frame table overflow"))?;
        let trees_at = kinds_at
            .checked_add(n)
            .ok_or(Error::Corrupt("frame table overflow"))?;
        let frames_at = trees_at
            .checked_add(trees_size)
            .ok_or(Error::Corrupt("frame table overflow"))?;
        if data.len() < frames_at {
            return Err(Error::Truncated("frame tables or trees"));
        }
        let mut frames = Vec::with_capacity(n);
        let mut offset = frames_at;
        for i in 0..n {
            let raw = u32_at(data, sizes_at + 4 * i);
            let size = (raw & !3) as usize;
            let next_offset = offset
                .checked_add(size)
                .ok_or(Error::Corrupt("frame offset overflow"))?;
            if next_offset > data.len() {
                return Err(Error::Truncated("frame data"));
            }
            frames.push(FrameEntry {
                offset,
                size,
                keyframe: raw & 1 != 0,
                kind: data[kinds_at + i],
            });
            offset = next_offset;
        }
        let mut br = BitReader::new(&data[trees_at..frames_at]);
        let trees = [
            BigTree::read(&mut br)?,
            BigTree::read(&mut br)?,
            BigTree::read(&mut br)?,
            BigTree::read(&mut br)?,
        ];
        if br.overrun() {
            return Err(Error::Truncated("Huffman trees"));
        }
        Ok(Smacker {
            data,
            header: Header {
                version,
                width,
                height,
                frame_count,
                frame_micros,
                ring_frame,
                y_interlaced: flags & FLAG_Y_INTERLACE != 0,
                y_doubled: flags & FLAG_Y_DOUBLE != 0,
                audio_tracks,
            },
            frames,
            trees,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    /// A decoder positioned at frame 0.
    pub fn decoder(&self) -> Decoder<'_, 'a> {
        let h = &self.header;
        let bw = h.width.div_ceil(4) as usize;
        let bh = h.height.div_ceil(4) as usize;
        let pixels = if bw * 4 == h.width as usize {
            Vec::new()
        } else {
            vec![0; (h.width * h.height) as usize]
        };
        Decoder {
            smk: self,
            next: 0,
            trees: self.trees.clone(),
            palette: [[0; 3]; 256],
            palette_rgba: [[0, 0, 0, 255]; 256],
            bw,
            bh,
            work: vec![0; bw * 4 * bh * 4],
            pixels,
        }
    }
}

/// What a decoded frame carried besides its picture.
#[derive(Debug, Clone)]
pub struct Frame {
    pub index: u32,
    pub keyframe: bool,
    /// The frame changed the palette.
    pub palette_changed: bool,
    /// Audio for each track present in this frame, in track order.
    pub audio: Vec<AudioChunk>,
}

/// Decodes frames in order. The picture and palette of the last decoded frame are kept in the
/// decoder ([`Decoder::pixels`], [`Decoder::palette`]).
pub struct Decoder<'s, 'a> {
    smk: &'s Smacker<'a>,
    next: usize,
    trees: [BigTree; 4],
    palette: [[u8; 3]; 256],
    palette_rgba: [[u8; 4]; 256],
    bw: usize,
    bh: usize,
    /// The picture with its width and height padded to whole 4x4 blocks.
    work: Vec<u8>,
    /// Populated only when `width` is not a multiple of 4; otherwise `pixels()` slices `work`.
    pixels: Vec<u8>,
}

impl Decoder<'_, '_> {
    /// Decodes the next frame (the ring frame is not returned). `None` after the last frame.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, Error> {
        let h = &self.smk.header;
        if self.next >= h.frame_count as usize {
            return Ok(None);
        }
        let entry = self.smk.frames[self.next];
        let index = self.next as u32;
        self.next += 1;
        let mut data = &self.smk.data[entry.offset..entry.offset + entry.size];

        let palette_changed = entry.kind & 1 != 0;
        if palette_changed {
            let len = *data.first().ok_or(Error::Truncated("palette"))? as usize * 4;
            if len == 0 || len > data.len() {
                return Err(Error::Corrupt("palette record size"));
            }
            video::update_palette(&mut self.palette, &data[1..len]);
            for (dst, &[r, g, b]) in self.palette_rgba.iter_mut().zip(&self.palette) {
                *dst = [r, g, b, 255];
            }
            data = &data[len..];
        }

        let mut audio = Vec::new();
        for track in 0..7u8 {
            if entry.kind & (2 << track) == 0 {
                continue;
            }
            if data.len() < 4 {
                return Err(Error::Truncated("audio chunk"));
            }
            let len = u32_at(data, 0) as usize;
            if len < 4 || len > data.len() {
                return Err(Error::Corrupt("audio chunk size"));
            }
            if let Some(info) = h.audio_tracks.iter().find(|t| t.index == track) {
                audio.push(audio::decode(info, &data[4..len])?);
            }
            data = &data[len..];
        }

        video::decode_blocks(
            &mut self.trees,
            h.version,
            data,
            &mut self.work,
            self.bw,
            self.bh,
        )?;
        let (w, ph) = (h.width as usize, h.height as usize);
        let stride = self.bw * 4;
        if stride != w {
            for y in 0..ph {
                self.pixels[y * w..(y + 1) * w]
                    .copy_from_slice(&self.work[y * stride..y * stride + w]);
            }
        }
        Ok(Some(Frame {
            index,
            keyframe: entry.keyframe,
            palette_changed,
            audio,
        }))
    }

    /// The last decoded picture: `width * height` palette indices, row by row.
    ///
    /// Rows are stored as coded; [`Header::y_interlaced`] and [`Header::y_doubled`] scaling is
    /// left to the caller.
    pub fn pixels(&self) -> &[u8] {
        let h = &self.smk.header;
        if self.bw * 4 == h.width as usize {
            &self.work[..(h.width * h.height) as usize]
        } else {
            &self.pixels
        }
    }

    /// The current palette, 8-bit RGB.
    pub fn palette(&self) -> &[[u8; 3]; 256] {
        &self.palette
    }

    /// Writes the last decoded picture as RGBA (alpha 255) into `out`, resizing it to
    /// `width * height * 4` bytes.
    pub fn rgba_into(&self, out: &mut Vec<u8>) {
        let pixels = self.pixels();
        out.clear();
        out.resize(pixels.len() * 4, 0);
        for (dst, &i) in out.as_chunks_mut::<4>().0.iter_mut().zip(pixels) {
            *dst = self.palette_rgba[i as usize];
        }
    }

    /// The last decoded picture as RGBA (alpha 255), `width * height * 4` bytes.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.rgba_into(&mut out);
        out
    }

    /// Index of the next frame `next_frame` will decode.
    pub fn position(&self) -> u32 {
        self.next as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            Smacker::open(b"SMK2").unwrap_err(),
            Error::Truncated("header")
        );
        let mut h = vec![0u8; HEADER_SIZE];
        h[0..4].copy_from_slice(b"RIFF");
        assert_eq!(Smacker::open(&h).unwrap_err(), Error::BadSignature);
    }

    /// A 4x4, one-frame SMK4 file whose type tree is a single solid-block leaf.
    #[test]
    fn minimal_solid_frame() {
        let mut f = vec![0u8; HEADER_SIZE];
        f[0..4].copy_from_slice(b"SMK4");
        f[4] = 4;
        f[8] = 4;
        f[12] = 1;
        f[16] = 100;
        let mut bits = vec![0u8, 0, 0, 1, 1, 0];
        bits.extend((0..8).map(|i| (0x03u8 >> i) & 1));
        bits.extend([0, 1, 0]);
        bits.extend((0..8).map(|i| (0x2au8 >> i) & 1));
        bits.push(0);
        for _ in 0..3 {
            bits.extend((0..16).map(|i| ((0x0101u16 >> i) & 1) as u8));
        }
        bits.extend([0, 0]);
        let mut trees = vec![0u8; bits.len().div_ceil(8)];
        for (i, &b) in bits.iter().enumerate() {
            trees[i / 8] |= b << (i % 8);
        }
        f[52..56].copy_from_slice(&(trees.len() as u32).to_le_bytes());
        f.extend(4u32.to_le_bytes());
        f.push(0);
        f.extend(&trees);
        f.extend([0u8; 4]);
        let smk = Smacker::open(&f).unwrap();
        assert_eq!(smk.header().version, Version::Smk4);
        assert_eq!(smk.header().fps(), 10.0);
        let mut dec = smk.decoder();
        assert!(dec.next_frame().unwrap().is_some());
        assert_eq!(dec.pixels(), &[0x2a; 16]);
        assert_eq!(dec.rgba().len(), 64);
        assert!(dec.next_frame().unwrap().is_none());
    }

    #[test]
    fn rejects_extreme_header_values_without_panic() {
        let mut h = vec![0u8; HEADER_SIZE];
        h[0..4].copy_from_slice(b"SMK2");
        h[4..8].copy_from_slice(&4u32.to_le_bytes());
        h[8..12].copy_from_slice(&4u32.to_le_bytes());
        h[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        h[16..20].copy_from_slice(&i32::MAX.to_le_bytes());
        h[20..24].copy_from_slice(&FLAG_RING.to_le_bytes());
        assert!(Smacker::open(&h).is_err());

        h[12..16].copy_from_slice(&0u32.to_le_bytes());
        h[16..20].copy_from_slice(&i32::MIN.to_le_bytes());
        h[20..24].copy_from_slice(&0u32.to_le_bytes());
        h[52..56].copy_from_slice(&1u32.to_le_bytes());
        h.push(0);
        let smk = Smacker::open(&h).unwrap();
        assert_eq!(smk.header().frame_micros, u32::MAX);
        assert!(smk.header().fps().is_finite());
    }

    #[test]
    fn smk4_full_block_chain_modes_match_spec() {
        fn pack(bits: &[u8]) -> Vec<u8> {
            let mut out = vec![0u8; bits.len().div_ceil(8)];
            for (i, &b) in bits.iter().enumerate() {
                out[i / 8] |= (b & 1) << (i % 8);
            }
            out
        }
        fn byte(v: u8) -> impl Iterator<Item = u8> {
            (0..8).map(move |i| (v >> i) & 1)
        }
        fn u16_bits(v: u16) -> impl Iterator<Item = u8> {
            (0..16).map(move |i| ((v >> i) & 1) as u8)
        }
        fn build_smk4(frame_bits: &[u8]) -> Vec<u8> {
            // 8x4 (2 blocks), single frame; TYPE is kind=1 (full), run_idx=1 (run=2);
            // FULL has code 0 -> 0x0201 and code 1 -> 0x0403.
            let mut tb = vec![0u8, 0];
            tb.extend([1, 1, 1, 0]);
            tb.extend(byte(0x01));
            tb.push(0);
            tb.extend(byte(0x03));
            tb.extend([0, 1, 1, 0]);
            tb.extend(byte(0x02));
            tb.push(0);
            tb.extend(byte(0x04));
            tb.push(0);
            for e in [0xfff1u16, 0xfff2, 0xfff3] {
                tb.extend(u16_bits(e));
            }
            tb.extend([1, 0, 0, 0, 0, 1, 1, 0]);
            // TYPE single leaf = (1 << 2) | 1 = 5.
            tb.extend([1, 1, 0]);
            tb.extend(byte(5));
            tb.extend([0, 1, 0]);
            tb.extend(byte(0));
            tb.push(0);
            for e in [0xfff1u16, 0xfff2, 0xfff3] {
                tb.extend(u16_bits(e));
            }
            tb.extend([0, 0]);
            let trees = pack(&tb);

            let mut frame_data = pack(frame_bits);
            while !frame_data.len().is_multiple_of(4) {
                frame_data.push(0);
            }
            let mut f = vec![0u8; HEADER_SIZE];
            f[0..4].copy_from_slice(b"SMK4");
            f[4..8].copy_from_slice(&8u32.to_le_bytes());
            f[8..12].copy_from_slice(&4u32.to_le_bytes());
            f[12..16].copy_from_slice(&1u32.to_le_bytes());
            f[16..20].copy_from_slice(&100u32.to_le_bytes());
            f[52..56].copy_from_slice(&(trees.len() as u32).to_le_bytes());
            f.extend((frame_data.len() as u32).to_le_bytes());
            f.push(0);
            f.extend(&trees);
            f.extend(&frame_data);
            f
        }

        // Mode 1 (double, prefix `1`): 2 values per block, applied to both blocks in the run.
        let data = build_smk4(&[1, 0, 1, 1, 0]);
        let smk = Smacker::open(&data).unwrap();
        let mut dec = smk.decoder();
        dec.next_frame().unwrap();
        assert_eq!(
            dec.pixels(),
            &[
                1, 1, 2, 2, 3, 3, 4, 4, //
                1, 1, 2, 2, 3, 3, 4, 4, //
                3, 3, 4, 4, 1, 1, 2, 2, //
                3, 3, 4, 4, 1, 1, 2, 2, //
            ]
        );

        // Mode 2 (half, prefix `01`): 4 values per block, rows 1 and 3 repeat rows 0 and 2.
        let data = build_smk4(&[0, 1, 0, 1, 1, 0, 1, 0, 0, 1]);
        let smk = Smacker::open(&data).unwrap();
        let mut dec = smk.decoder();
        dec.next_frame().unwrap();
        assert_eq!(
            dec.pixels(),
            &[
                3, 4, 1, 2, 1, 2, 3, 4, //
                3, 4, 1, 2, 1, 2, 3, 4, //
                1, 2, 3, 4, 3, 4, 1, 2, //
                1, 2, 3, 4, 3, 4, 1, 2, //
            ]
        );

        // Mode 0 (plain, prefix `00`): 8 values per block.
        let mut plain_bits = vec![0, 0];
        plain_bits.extend([0, 1, 0, 1, 0, 1, 0, 1]);
        plain_bits.extend([1, 0, 1, 0, 1, 0, 1, 0]);
        let data = build_smk4(&plain_bits);
        let smk = Smacker::open(&data).unwrap();
        let mut dec = smk.decoder();
        dec.next_frame().unwrap();
        assert_eq!(
            dec.pixels(),
            &[
                3, 4, 1, 2, 1, 2, 3, 4, //
                3, 4, 1, 2, 1, 2, 3, 4, //
                3, 4, 1, 2, 1, 2, 3, 4, //
                3, 4, 1, 2, 1, 2, 3, 4, //
            ]
        );
    }
}
