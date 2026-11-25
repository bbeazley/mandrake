//! Audio chunks: raw PCM, or Huffman-coded DPCM.

use crate::bits::BitReader;
use crate::huffman::ByteTree;
use crate::{AudioTrack, Error};

/// Decoded samples, interleaved (left, right) when stereo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Samples {
    /// 8-bit unsigned (128 is silence).
    U8(Vec<u8>),
    /// 16-bit signed.
    S16(Vec<i16>),
}

impl Samples {
    /// Number of samples over all channels.
    pub fn len(&self) -> usize {
        match self {
            Samples::U8(v) => v.len(),
            Samples::S16(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The samples as 16-bit signed (8-bit audio is re-centred and scaled by 256).
    pub fn to_i16(&self) -> Vec<i16> {
        match self {
            Samples::U8(v) => v.iter().map(|&s| ((s as i16) - 128) << 8).collect(),
            Samples::S16(v) => v.clone(),
        }
    }
}

/// One frame's audio for one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioChunk {
    pub track: u8,
    pub sample_rate: u32,
    pub channels: u8,
    pub samples: Samples,
}

/// Upper bound on one audio chunk's decompressed byte length.
const MAX_UNPACKED_BYTES: usize = 16 * 1024 * 1024;

/// Decodes a chunk's payload (after its 4-byte length).
pub(crate) fn decode(track: &AudioTrack, data: &[u8]) -> Result<AudioChunk, Error> {
    let chunk = |samples| AudioChunk {
        track: track.index,
        sample_rate: track.sample_rate,
        channels: track.channels,
        samples,
    };
    if !track.compressed {
        let samples = if track.bits == 16 {
            Samples::S16(
                data.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&b| i16::from_le_bytes(b))
                    .collect(),
            )
        } else {
            Samples::U8(data.to_vec())
        };
        return Ok(chunk(samples));
    }
    if track.bink {
        return Err(Error::Unsupported("Bink audio"));
    }
    if data.len() < 4 {
        return Err(Error::Truncated("audio unpacked length"));
    }
    let unpacked = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let limit = if track.max_size > 0 {
        (track.max_size as usize).min(MAX_UNPACKED_BYTES)
    } else {
        MAX_UNPACKED_BYTES
    };
    if unpacked > limit {
        return Err(Error::Corrupt("audio unpacked size exceeds limit"));
    }
    let mut br = BitReader::new(&data[4..]);
    let present = br.bit();
    if br.overrun() {
        return Err(Error::Truncated("audio chunk"));
    }
    if !present {
        let samples = if track.bits == 16 {
            Samples::S16(vec![0; unpacked / 2])
        } else {
            Samples::U8(vec![128; unpacked])
        };
        return Ok(chunk(samples));
    }
    let stereo = br.bit();
    let wide = br.bit();
    if stereo != (track.channels == 2) || wide != (track.bits == 16) {
        return Err(Error::Corrupt("audio chunk format differs from the header"));
    }
    let channels = 1 + stereo as usize;
    let mut trees = Vec::with_capacity(4);
    for _ in 0..channels * (1 + wide as usize) {
        trees.push(ByteTree::read(&mut br)?);
    }
    let samples = if wide {
        let count = unpacked / 2;
        let mut out = Vec::with_capacity(count);
        // Bases: the right channel's before the left's, each high byte first.
        let mut cur = [0i16; 2];
        for ch in (0..channels).rev() {
            let hi = br.bits(8);
            let lo = br.bits(8);
            cur[ch] = ((hi << 8) | lo) as u16 as i16;
        }
        if br.overrun() {
            return Err(Error::Truncated("audio samples"));
        }
        out.extend_from_slice(&cur[..channels.min(count)]);
        while out.len() < count {
            let ch = out.len() % channels;
            let lo = trees[ch * 2].decode(&mut br) as u16;
            let hi = trees[ch * 2 + 1].decode(&mut br) as u16;
            if br.overrun() {
                return Err(Error::Truncated("audio samples"));
            }
            cur[ch] = cur[ch].wrapping_add((hi << 8 | lo) as i16);
            out.push(cur[ch]);
        }
        Samples::S16(out)
    } else {
        let count = unpacked;
        let mut out = Vec::with_capacity(count);
        let mut cur = [0u8; 2];
        for ch in (0..channels).rev() {
            cur[ch] = br.bits(8) as u8;
        }
        if br.overrun() {
            return Err(Error::Truncated("audio samples"));
        }
        out.extend_from_slice(&cur[..channels.min(count)]);
        while out.len() < count {
            let ch = out.len() % channels;
            let delta = trees[ch].decode(&mut br);
            if br.overrun() {
                return Err(Error::Truncated("audio samples"));
            }
            cur[ch] = cur[ch].wrapping_add(delta);
            out.push(cur[ch]);
        }
        Samples::U8(out)
    };
    Ok(chunk(samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(channels: u8, bits: u8, compressed: bool) -> AudioTrack {
        AudioTrack {
            index: 0,
            sample_rate: 22050,
            channels,
            bits,
            compressed,
            bink: false,
            max_size: 0,
        }
    }

    fn pack(bits: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; bits.len().div_ceil(8)];
        for (i, &b) in bits.iter().enumerate() {
            out[i / 8] |= b << (i % 8);
        }
        out
    }

    fn byte(v: u8) -> impl Iterator<Item = u8> {
        (0..8).map(move |i| (v >> i) & 1)
    }

    #[test]
    fn dpcm_8bit_mono() {
        // present, mono, 8-bit; tree node(leaf +1, leaf -1); base 100; deltas +1 +1 -1.
        let mut b = vec![1, 0, 0, 1, 1, 0];
        b.extend(byte(1));
        b.push(0);
        b.extend(byte(0xff));
        b.push(0);
        b.extend(byte(100));
        b.extend([0, 0, 1]);
        let mut data = 4u32.to_le_bytes().to_vec();
        data.extend(pack(&b));
        let c = decode(&track(1, 8, true), &data).unwrap();
        assert_eq!(c.samples, Samples::U8(vec![100, 101, 102, 101]));
    }

    #[test]
    fn dpcm_16bit_stereo_bases_right_first() {
        // present, stereo, 16-bit; four absent trees (all deltas 0); bases R=0x0102, L=0xff00.
        let mut b = vec![1, 1, 1, 0, 0, 0, 0];
        b.extend(byte(0x01));
        b.extend(byte(0x02));
        b.extend(byte(0xff));
        b.extend(byte(0x00));
        let mut data = 8u32.to_le_bytes().to_vec();
        data.extend(pack(&b));
        let c = decode(&track(2, 16, true), &data).unwrap();
        assert_eq!(c.samples, Samples::S16(vec![-256, 0x0102, -256, 0x0102]));
    }

    #[test]
    fn raw_pcm() {
        let c = decode(&track(1, 16, false), &[0x34, 0x12, 0xff, 0xff]).unwrap();
        assert_eq!(c.samples, Samples::S16(vec![0x1234, -1]));
        assert_eq!(c.samples.to_i16(), vec![0x1234, -1]);
        let c = decode(&track(1, 8, false), &[0, 128]).unwrap();
        assert_eq!(c.samples.to_i16(), vec![-32768, 0]);
    }

    #[test]
    fn rejects_oversized_or_truncated_compressed_audio() {
        let mut t = track(1, 8, true);
        t.max_size = 64;
        let mut oversized = 65u32.to_le_bytes().to_vec();
        oversized.push(0);
        assert!(matches!(decode(&t, &oversized), Err(Error::Corrupt(_))));

        let mut huge = u32::MAX.to_le_bytes().to_vec();
        huge.push(0);
        assert!(matches!(
            decode(&track(1, 8, true), &huge),
            Err(Error::Corrupt(_))
        ));

        // 8-bit mono DPCM claiming 100 samples, but bitstream ends right after the base byte.
        let mut b = vec![1, 0, 0, 1, 1, 0];
        b.extend(byte(1));
        b.push(0);
        b.extend(byte(0xff));
        b.push(0);
        b.extend(byte(100));
        let mut truncated = 100u32.to_le_bytes().to_vec();
        truncated.extend(pack(&b));
        assert!(matches!(
            decode(&track(1, 8, true), &truncated),
            Err(Error::Truncated(_))
        ));
    }
}
