//! Palette records and the 4x4 block video.

use crate::bits::BitReader;
use crate::huffman::BigTree;
use crate::{Error, Version};

/// 6-bit colour components to 8 bits: 4 * i + i / 16.
const fn expand6(i: u8) -> u8 {
    i * 4 + i / 16
}

/// Applies one palette record (without its length byte) to the palette. Copy blocks read from
/// the palette as it was before this record.
pub(crate) fn update_palette(palette: &mut [[u8; 3]; 256], rec: &[u8]) {
    let old = *palette;
    let mut i = 0usize;
    let mut p = 0usize;
    while i < 256 && p < rec.len() {
        let b = rec[p];
        p += 1;
        if b & 0x80 != 0 {
            i += (b & 0x7f) as usize + 1;
        } else if b & 0x40 != 0 {
            let count = (b & 0x3f) as usize + 1;
            let Some(&src) = rec.get(p) else { break };
            p += 1;
            let src = src as usize;
            for k in 0..count {
                if i >= 256 {
                    break;
                }
                palette[i] = old[(src + k) & 0xff];
                i += 1;
            }
        } else {
            if p + 2 > rec.len() {
                break;
            }
            palette[i] = [
                expand6(b & 0x3f),
                expand6(rec[p] & 0x3f),
                expand6(rec[p + 1] & 0x3f),
            ];
            p += 2;
            i += 1;
        }
    }
}

/// Chain lengths by the 6-bit index in a type value.
const RUN: [usize; 64] = {
    let mut t = [0usize; 64];
    let mut i = 0;
    while i < 59 {
        t[i] = i + 1;
        i += 1;
    }
    t[59] = 128;
    t[60] = 256;
    t[61] = 512;
    t[62] = 1024;
    t[63] = 2048;
    t
};

const MMAP: usize = 0;
const MCLR: usize = 1;
const FULL: usize = 2;
const TYPE: usize = 3;

/// Decodes one frame's block data into `pic` (stride `bw * 4`), which holds the previous
/// picture on entry (void blocks keep it).
pub(crate) fn decode_blocks(
    trees: &mut [BigTree; 4],
    version: Version,
    data: &[u8],
    pic: &mut [u8],
    bw: usize,
    bh: usize,
) -> Result<(), Error> {
    for t in trees.iter_mut() {
        t.reset();
    }
    let stride = bw * 4;
    let blocks = bw * bh;
    let mut br = BitReader::new(data);
    let mut blk = 0usize;
    while blk < blocks {
        let t = trees[TYPE].decode(&mut br);
        let kind = t & 3;
        let run = RUN[((t >> 2) & 0x3f) as usize].min(blocks - blk);
        let extra = (t >> 8) as u8;
        let mode = if kind == 1 && version == Version::Smk4 {
            if br.bit() {
                1
            } else if br.bit() {
                2
            } else {
                0
            }
        } else {
            0
        };
        for b in blk..blk + run {
            let at = (b / bw) * 4 * stride + (b % bw) * 4;
            match kind {
                0 => {
                    let clr = trees[MCLR].decode(&mut br);
                    let mut map = trees[MMAP].decode(&mut br);
                    let (lo, hi) = (clr as u8, (clr >> 8) as u8);
                    for row in 0..4 {
                        let o = at + row * stride;
                        for px in &mut pic[o..o + 4] {
                            *px = if map & 1 != 0 { hi } else { lo };
                            map >>= 1;
                        }
                    }
                }
                1 => full_block(&mut trees[FULL], mode, &mut br, pic, at, stride),
                2 => {}
                _ => {
                    for row in 0..4 {
                        let o = at + row * stride;
                        pic[o..o + 4].fill(extra);
                    }
                }
            }
        }
        blk += run;
    }
    if br.overrun() {
        return Err(Error::Truncated("video blocks"));
    }
    Ok(())
}

/// Each row is coded right pair then left pair, low byte first.
#[inline]
fn plain_row(full: &mut BigTree, br: &mut BitReader, pic: &mut [u8], o: usize) {
    let v = full.decode(br);
    pic[o + 2] = v as u8;
    pic[o + 3] = (v >> 8) as u8;
    let v = full.decode(br);
    pic[o] = v as u8;
    pic[o + 1] = (v >> 8) as u8;
}

/// One full block: in SMK4, a per-chain prefix picks double (`1`), half (`01`) or plain (`00`).
fn full_block(
    full: &mut BigTree,
    mode: u8,
    br: &mut BitReader,
    pic: &mut [u8],
    at: usize,
    stride: usize,
) {
    match mode {
        0 => {
            for row in 0..4 {
                plain_row(full, br, pic, at + row * stride);
            }
        }
        1 => {
            // Double: each value colours two 2x2 quarters (left low byte, right high byte),
            // top half first.
            for half in 0..2 {
                let v = full.decode(br);
                for row in half * 2..half * 2 + 2 {
                    let o = at + row * stride;
                    pic[o..o + 2].fill(v as u8);
                    pic[o + 2..o + 4].fill((v >> 8) as u8);
                }
            }
        }
        _ => {
            // Half: rows 0 and 2 coded like a plain row, rows 1 and 3 repeat the row above.
            for row in [0, 2] {
                let o = at + row * stride;
                plain_row(full, br, pic, o);
                pic.copy_within(o..o + 4, o + stride);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_expansion_table() {
        assert_eq!(expand6(0), 0);
        assert_eq!(expand6(15), 60);
        assert_eq!(expand6(16), 65);
        assert_eq!(expand6(63), 255);
    }

    #[test]
    fn palette_record_blocks() {
        let mut pal = [[0u8; 3]; 256];
        for (i, c) in pal.iter_mut().enumerate() {
            *c = [i as u8, i as u8, i as u8];
        }
        // New colour at 0, keep 2 entries, copy 3 entries from old index 10, new colour.
        update_palette(
            &mut pal,
            &[0x3f, 0x10, 0x00, 0x81, 0x42, 10, 0x01, 0x02, 0x03],
        );
        assert_eq!(pal[0], [255, 65, 0]);
        assert_eq!(pal[1], [1, 1, 1]);
        assert_eq!(pal[2], [2, 2, 2]);
        assert_eq!(pal[3], [10, 10, 10]);
        assert_eq!(pal[5], [12, 12, 12]);
        assert_eq!(pal[6], [4, 8, 12]);
        assert_eq!(pal[7], [7, 7, 7]);
    }

    #[test]
    fn run_table() {
        assert_eq!(RUN[0], 1);
        assert_eq!(RUN[58], 59);
        assert_eq!(RUN[59], 128);
        assert_eq!(RUN[63], 2048);
    }
}
