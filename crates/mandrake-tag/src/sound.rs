use byteorder::{BigEndian, LittleEndian, ReadBytesExt, WriteBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::io::{Cursor, Read, Write};

use crate::header::decode_c_string;

pub const ENGINE_SAMPLE_RATE: u32 = 22050;

pub const IMA4_PACKET_SIZE: usize = 34;

pub const IMA4_SAMPLES_PER_PACKET: usize = 64;

const IMA_STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

const IMA_INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

pub fn decode_ima4_packet(packet: &[u8; IMA4_PACKET_SIZE], out: &mut [i16]) {
    let header = u16::from_be_bytes([packet[0], packet[1]]);
    let mut predictor = (header & 0xff80) as i16 as i32;
    let mut index = ((header & 0x7f) as i32).min(88);
    let mut step = IMA_STEP_TABLE[index as usize];

    for (i, sample) in out.iter_mut().take(IMA4_SAMPLES_PER_PACKET).enumerate() {
        let byte = packet[2 + i / 2];
        let nibble = if i % 2 == 0 { byte & 0x0f } else { byte >> 4 } as i32;

        let mut diff = step >> 3;
        if nibble & 4 != 0 {
            diff += step;
        }
        if nibble & 2 != 0 {
            diff += step >> 1;
        }
        if nibble & 1 != 0 {
            diff += step >> 2;
        }
        if nibble & 8 != 0 {
            diff = -diff;
        }
        predictor = (predictor + diff).clamp(-32768, 32767);
        *sample = predictor as i16;

        index = (index + IMA_INDEX_TABLE[nibble as usize]).clamp(0, 88);
        step = IMA_STEP_TABLE[index as usize];
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SampledSoundHeader {
    pub flags: u32,
    pub bits_per_sample: u16,
    pub format: u16,
    pub channels: u16,
    pub sample_rate_fixed: u32,
    pub packet_count: u32,
}

impl SampledSoundHeader {
    pub const SIZE: usize = 32;

    fn read(data: &[u8]) -> Result<Self, M2Error> {
        let mut c = Cursor::new(data);
        let flags = c.read_u32::<BigEndian>()?;
        let bits_per_sample = c.read_u16::<BigEndian>()?;
        let format = c.read_u16::<BigEndian>()?;
        let channels = c.read_u16::<BigEndian>()?;
        let _pad = c.read_u16::<BigEndian>()?;
        let sample_rate_fixed = c.read_u32::<BigEndian>()?;
        let packet_count = c.read_u32::<BigEndian>()?;
        Ok(Self {
            flags,
            bits_per_sample,
            format,
            channels,
            sample_rate_fixed,
            packet_count,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate_fixed >> 16
    }

    pub fn is_stereo(&self) -> bool {
        self.channels > 1
    }

    pub fn data_size(&self) -> usize {
        (self.packet_count as usize).saturating_mul(IMA4_PACKET_SIZE)
    }

    fn checked_data_size(&self) -> Option<usize> {
        (self.packet_count as usize).checked_mul(IMA4_PACKET_SIZE)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SoundPermutation {
    pub flags: u16,
    pub skip_fraction: u16,
    pub name: String,
    pub header: SampledSoundHeader,
    pub adpcm_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSound {
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

impl DecodedSound {
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frame_count() as f32 / self.sample_rate as f32
        }
    }

    pub fn to_wav_bytes(&self) -> Result<Vec<u8>, M2Error> {
        let channels = self.channels.max(1);
        let block_align = channels * 2;
        let data_len = (self.samples.len() * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_len as usize);

        out.write_all(b"RIFF")?;
        out.write_u32::<LittleEndian>(36 + data_len)?;
        out.write_all(b"WAVE")?;

        out.write_all(b"fmt ")?;
        out.write_u32::<LittleEndian>(16)?;
        out.write_u16::<LittleEndian>(1)?;
        out.write_u16::<LittleEndian>(channels)?;
        out.write_u32::<LittleEndian>(self.sample_rate)?;
        out.write_u32::<LittleEndian>(self.sample_rate * block_align as u32)?;
        out.write_u16::<LittleEndian>(block_align)?;
        out.write_u16::<LittleEndian>(16)?;

        out.write_all(b"data")?;
        out.write_u32::<LittleEndian>(data_len)?;
        for &s in &self.samples {
            out.write_i16::<LittleEndian>(s)?;
        }
        Ok(out)
    }
}

impl SoundPermutation {
    pub const SIZE: usize = 32;

    pub fn frame_count(&self) -> usize {
        let packets = self.header.packet_count as usize;
        if self.header.is_stereo() {
            (packets / 2) * IMA4_SAMPLES_PER_PACKET
        } else {
            packets * IMA4_SAMPLES_PER_PACKET
        }
    }

    pub fn decode(&self) -> DecodedSound {
        let (packets, _) = self.adpcm_data.as_chunks::<IMA4_PACKET_SIZE>();
        let stereo = self.header.is_stereo();
        let mut samples = Vec::with_capacity(packets.len() * IMA4_SAMPLES_PER_PACKET);
        let mut left = [0i16; IMA4_SAMPLES_PER_PACKET];
        let mut right = [0i16; IMA4_SAMPLES_PER_PACKET];

        if stereo {
            for [l, r] in packets.as_chunks::<2>().0 {
                decode_ima4_packet(l, &mut left);
                decode_ima4_packet(r, &mut right);
                for (l, r) in left.iter().zip(&right) {
                    samples.push(*l);
                    samples.push(*r);
                }
            }
        } else {
            for packet in packets {
                decode_ima4_packet(packet, &mut left);
                samples.extend_from_slice(&left);
            }
        }

        DecodedSound {
            channels: if stereo { 2 } else { 1 },
            sample_rate: ENGINE_SAMPLE_RATE,
            samples,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SoundTag {
    pub flags: u32,
    pub loudness: i16,
    pub play_fraction: u16,
    pub external_frequency_modifier: i16,
    pub pitch_lower_bound: i16,
    pub pitch_delta: i16,
    pub volume_lower_bound: i16,
    pub volume_delta: i16,
    pub first_subtitle_index: i16,
    pub subtitle_string_list: Option<FourCC>,
    pub permutations: Vec<SoundPermutation>,
}

fn optional_tag(bytes: [u8; 4]) -> Option<FourCC> {
    match u32::from_be_bytes(bytes) {
        0 | 0xffff_ffff => None,
        _ => Some(FourCC::new(bytes)),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PermutationHistory {
    pub played_mask: u32,
    pub last_played: Option<usize>,
}

impl SoundTag {
    pub const HEADER_SIZE: usize = 64;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < Self::HEADER_SIZE {
            return Err(M2Error::TagParse(format!(
                "Sound data too short: expected >= 64 bytes, got {}",
                data.len()
            )));
        }

        let mut c = Cursor::new(data);
        let flags = c.read_u32::<BigEndian>()?;
        let loudness = c.read_i16::<BigEndian>()?;
        let play_fraction = c.read_u16::<BigEndian>()?;
        let external_frequency_modifier = c.read_i16::<BigEndian>()?;
        let pitch_lower_bound = c.read_i16::<BigEndian>()?;
        let pitch_delta = c.read_i16::<BigEndian>()?;
        let volume_lower_bound = c.read_i16::<BigEndian>()?;
        let volume_delta = c.read_i16::<BigEndian>()?;
        let first_subtitle_index = c.read_i16::<BigEndian>()?;
        let sample_data_offset = c.read_u32::<BigEndian>()? as usize;
        let sample_data_size = c.read_u32::<BigEndian>()? as usize;
        let mut stli = [0u8; 4];
        c.read_exact(&mut stli)?;
        c.set_position(0x24);
        let permutation_count = c.read_u32::<BigEndian>()? as usize;
        let permutation_offset = c.read_u32::<BigEndian>()? as usize;
        let _permutation_size = c.read_u32::<BigEndian>()?;

        let slice = |offset: usize, len: usize, what: &str| -> Result<&[u8], M2Error> {
            offset
                .checked_add(len)
                .and_then(|end| data.get(offset..end))
                .ok_or_else(|| {
                    M2Error::TagParse(format!(
                        "soun {what} [{offset}..+{len}] exceeds tag size {}",
                        data.len()
                    ))
                })
        };

        let perm_bytes = permutation_count
            .checked_mul(SoundPermutation::SIZE)
            .ok_or_else(|| M2Error::TagParse("soun permutation table size overflow".into()))?;
        let perm_table = slice(permutation_offset, perm_bytes, "permutation table")?;
        let sample_data = slice(sample_data_offset, sample_data_size, "sample data")?;

        let header_bytes_len = permutation_count
            .checked_mul(SampledSoundHeader::SIZE)
            .ok_or_else(|| M2Error::TagParse("soun sample headers size overflow".into()))?;
        let headers = sample_data.get(..header_bytes_len).ok_or_else(|| {
            M2Error::TagParse("soun sample data too short for its headers".to_string())
        })?;
        let mut packet_cursor = headers.len();

        let mut permutations = Vec::with_capacity(permutation_count);
        let entries = perm_table.as_chunks::<{ SoundPermutation::SIZE }>().0;
        let sample_headers = headers.as_chunks::<{ SampledSoundHeader::SIZE }>().0;
        for (entry, header_bytes) in entries.iter().zip(sample_headers) {
            let perm_flags = u16::from_be_bytes([entry[0], entry[1]]);
            let skip_fraction = u16::from_be_bytes([entry[2], entry[3]]);
            let name = decode_c_string(&entry[6..32]);
            let header = SampledSoundHeader::read(header_bytes)?;

            let len = header.checked_data_size().ok_or_else(|| {
                M2Error::TagParse(format!("soun permutation '{name}' packet size overflow"))
            })?;
            let packet_end = packet_cursor.checked_add(len).ok_or_else(|| {
                M2Error::TagParse(format!("soun permutation '{name}' packet offset overflow"))
            })?;
            let adpcm_data = sample_data
                .get(packet_cursor..packet_end)
                .ok_or_else(|| {
                    M2Error::TagParse(format!(
                        "soun permutation '{name}' packets [{packet_cursor}..+{len}] exceed sample data size {}",
                        sample_data.len()
                    ))
                })?
                .to_vec();
            packet_cursor = packet_end;

            permutations.push(SoundPermutation {
                flags: perm_flags,
                skip_fraction,
                name,
                header,
                adpcm_data,
            });
        }

        Ok(Self {
            flags,
            loudness,
            play_fraction,
            external_frequency_modifier,
            pitch_lower_bound,
            pitch_delta,
            volume_lower_bound,
            volume_delta,
            first_subtitle_index,
            subtitle_string_list: optional_tag(stli),
            permutations,
        })
    }

    pub fn decode_permutation(&self, index: usize) -> Option<DecodedSound> {
        self.permutations.get(index).map(SoundPermutation::decode)
    }

    pub fn choose_permutation(
        &self,
        history: &mut PermutationHistory,
        mut rng: impl FnMut() -> u16,
    ) -> Option<usize> {
        if self.play_fraction < rng() {
            return None;
        }
        let count = self.permutations.len().min(31);
        if count == 0 {
            return None;
        }
        let full = (1u32 << count) - 1;
        let mut mask = history.played_mask & full;
        let mut index = rng() as usize % count;
        if mask == full {
            mask = 0;
        }

        loop {
            if mask & (1 << index) == 0 {
                mask |= 1 << index;
                if self.permutations[index].skip_fraction <= rng() {
                    break;
                }
            }
            index = if index == 0 { count - 1 } else { index - 1 };
            if mask == full {
                break;
            }
        }

        history.played_mask = mask;
        history.last_played = Some(index);
        Some(index)
    }

    pub fn random_pitch(&self, rng: impl FnOnce() -> u16) -> i32 {
        random_in_range(self.pitch_lower_bound, self.pitch_delta, rng)
    }

    pub fn random_volume(&self, rng: impl FnOnce() -> u16) -> i32 {
        random_in_range(self.volume_lower_bound, self.volume_delta, rng)
    }
}

fn random_in_range(lower: i16, delta: i16, rng: impl FnOnce() -> u16) -> i32 {
    if delta > 0 {
        lower as i32 + ((rng() as i32 * delta as i32) >> 16)
    } else {
        lower as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AmbientSoundTag {
    pub flags: u32,
    pub inner_radius: i16,
    pub outer_radius: i16,
    pub period_lower_bound: i16,
    pub period_delta: i16,
    pub sound_tags: [Option<FourCC>; 6],
    pub random_sound_radius: i16,
}

impl AmbientSoundTag {
    pub const SIZE: usize = 64;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < 0x26 {
            return Err(M2Error::TagParse(format!(
                "Ambient sound data too short: expected >= 38 bytes, got {}",
                data.len()
            )));
        }

        let mut c = Cursor::new(data);
        let flags = c.read_u32::<BigEndian>()?;
        let inner_radius = c.read_i16::<BigEndian>()?;
        let outer_radius = c.read_i16::<BigEndian>()?;
        let period_lower_bound = c.read_i16::<BigEndian>()?;
        let period_delta = c.read_i16::<BigEndian>()?;
        let mut sound_tags = [None; 6];
        for slot in &mut sound_tags {
            let mut b = [0u8; 4];
            c.read_exact(&mut b)?;
            *slot = optional_tag(b);
        }
        let random_sound_radius = c.read_i16::<BigEndian>()?;

        Ok(Self {
            flags,
            inner_radius,
            outer_radius,
            period_lower_bound,
            period_delta,
            sound_tags,
            random_sound_radius,
        })
    }

    pub fn continuous_sound(&self) -> Option<FourCC> {
        self.sound_tags[0]
    }

    pub fn periodic_sound(&self) -> Option<FourCC> {
        self.sound_tags[1]
    }

    pub fn random_period(&self, rng: impl FnOnce() -> u16) -> i32 {
        random_in_range(self.period_lower_bound, self.period_delta, rng)
    }

    pub fn inner_radius_world(&self) -> f32 {
        self.inner_radius as f32 / 512.0
    }

    pub fn outer_radius_world(&self) -> f32 {
        self.outer_radius as f32 / 512.0
    }

    pub fn random_sound_radius_world(&self) -> f32 {
        self.random_sound_radius as f32 / 512.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_ima4_packet(samples: &[i16; 64], predictor: i16, index: u8) -> [u8; 34] {
        let mut out = [0u8; 34];
        let header = (predictor as u16 & 0xff80) | index as u16;
        out[..2].copy_from_slice(&header.to_be_bytes());
        let mut pred = (header & 0xff80) as i16 as i32;
        let mut idx = index as i32;
        for (i, &s) in samples.iter().enumerate() {
            let step = IMA_STEP_TABLE[idx as usize];
            let mut diff = s as i32 - pred;
            let mut nibble = 0;
            if diff < 0 {
                nibble = 8;
                diff = -diff;
            }
            let mut vpdiff = step >> 3;
            if diff >= step {
                nibble |= 4;
                diff -= step;
                vpdiff += step;
            }
            if diff >= step >> 1 {
                nibble |= 2;
                diff -= step >> 1;
                vpdiff += step >> 1;
            }
            if diff >= step >> 2 {
                nibble |= 1;
                vpdiff += step >> 2;
            }
            pred = if nibble & 8 != 0 {
                pred - vpdiff
            } else {
                pred + vpdiff
            }
            .clamp(-32768, 32767);
            idx = (idx + IMA_INDEX_TABLE[nibble as usize]).clamp(0, 88);
            out[2 + i / 2] |= if i % 2 == 0 { nibble } else { nibble << 4 } as u8;
        }
        out
    }

    fn sine_packet(phase: f32, amp: f32) -> [i16; 64] {
        let mut s = [0i16; 64];
        for (i, v) in s.iter_mut().enumerate() {
            *v = ((phase + i as f32 * 0.2).sin() * amp) as i16;
        }
        s
    }

    #[test]
    fn test_ima4_packet_roundtrip_tracks_signal() {
        let input = sine_packet(0.0, 8000.0);
        let packet = encode_ima4_packet(&input, 0, 40);
        let mut out = [0i16; 64];
        decode_ima4_packet(&packet, &mut out);
        for i in 16..64 {
            assert!(
                (out[i] as i32 - input[i] as i32).abs() < 800,
                "sample {i}: {} vs {}",
                out[i],
                input[i]
            );
        }
    }

    #[test]
    fn test_ima4_header_predictor_and_index() {
        let mut packet = [0u8; 34];
        packet[..2].copy_from_slice(&((0x1000u16) | 10).to_be_bytes());
        let mut out = [0i16; 64];
        decode_ima4_packet(&packet, &mut out);
        assert_eq!(out[0], 0x1000 + (IMA_STEP_TABLE[10] >> 3) as i16);
        assert_eq!(out[1], out[0] + (IMA_STEP_TABLE[9] >> 3) as i16);
    }

    #[test]
    fn test_ambient_sound_tag_synthetic() {
        let mut data = vec![0u8; 64];
        data[4..6].copy_from_slice(&6144i16.to_be_bytes());
        data[6..8].copy_from_slice(&12288i16.to_be_bytes());
        data[8..10].copy_from_slice(&300i16.to_be_bytes());
        data[10..12].copy_from_slice(&301i16.to_be_bytes());
        data[12..16].copy_from_slice(&[0xff; 4]);
        data[16..20].copy_from_slice(b"rabi");
        data[20..36].copy_from_slice(&[0xff; 16]);
        data[36..38].copy_from_slice(&0x2800i16.to_be_bytes());

        let amso = AmbientSoundTag::from_bytes(&data).expect("Should parse ambient sound");
        assert_eq!(amso.continuous_sound(), None);
        assert_eq!(amso.periodic_sound(), Some(FourCC::new(*b"rabi")));
        assert_eq!(amso.sound_tags[2..], [None; 4]);
        assert_eq!(amso.inner_radius_world(), 12.0);
        assert_eq!(amso.outer_radius_world(), 24.0);
        assert_eq!(amso.random_sound_radius_world(), 20.0);
        assert_eq!(amso.random_period(|| 0), 300);
        assert_eq!(amso.random_period(|| 0x8000), 300 + 150);
    }

    fn build_soun(perms: &[(&str, u16, u16, Vec<[u8; 34]>)]) -> Vec<u8> {
        let n = perms.len();
        let perm_off = 0x40;
        let data_off = perm_off + 32 * n;
        let packets: usize = perms.iter().map(|p| p.3.len()).sum();
        let data_size = 32 * n + packets * 34;
        let mut d = vec![0u8; data_off + data_size];
        d[0..4].copy_from_slice(&1u32.to_be_bytes());
        d[4..6].copy_from_slice(&2i16.to_be_bytes());
        d[6..8].copy_from_slice(&0xffffu16.to_be_bytes());
        d[8..10].copy_from_slice(&0x100i16.to_be_bytes());
        d[10..12].copy_from_slice(&0xe5i16.to_be_bytes());
        d[12..14].copy_from_slice(&0x3ci16.to_be_bytes());
        d[14..16].copy_from_slice(&0xafi16.to_be_bytes());
        d[16..18].copy_from_slice(&0x1ei16.to_be_bytes());
        d[18..20].copy_from_slice(&(-2i16).to_be_bytes());
        d[20..24].copy_from_slice(&(data_off as u32).to_be_bytes());
        d[24..28].copy_from_slice(&(data_size as u32).to_be_bytes());
        d[28..32].copy_from_slice(&[0xff; 4]);
        d[36..40].copy_from_slice(&(n as u32).to_be_bytes());
        d[40..44].copy_from_slice(&(perm_off as u32).to_be_bytes());
        d[44..48].copy_from_slice(&(32 * n as u32).to_be_bytes());
        let mut pk = data_off + 32 * n;
        for (i, (name, skip, channels, pkts)) in perms.iter().enumerate() {
            let e = perm_off + 32 * i;
            d[e + 2..e + 4].copy_from_slice(&skip.to_be_bytes());
            d[e + 6..e + 6 + name.len()].copy_from_slice(name.as_bytes());
            let h = data_off + 32 * i;
            d[h..h + 4].copy_from_slice(&1u32.to_be_bytes());
            d[h + 4..h + 6].copy_from_slice(&16u16.to_be_bytes());
            d[h + 6..h + 8].copy_from_slice(&1u16.to_be_bytes());
            d[h + 8..h + 10].copy_from_slice(&channels.to_be_bytes());
            d[h + 12..h + 16].copy_from_slice(&0x5622_0000u32.to_be_bytes());
            d[h + 16..h + 20].copy_from_slice(&(pkts.len() as u32).to_be_bytes());
            for p in pkts {
                d[pk..pk + 34].copy_from_slice(p);
                pk += 34;
            }
        }
        d
    }

    #[test]
    fn test_sound_tag_synthetic_layout_and_decode() {
        let a = encode_ima4_packet(&sine_packet(0.0, 4000.0), 0, 30);
        let b = encode_ima4_packet(&sine_packet(1.0, 4000.0), 0, 30);
        let data = build_soun(&[
            ("Sword Swing 1", 0x3333, 1, vec![a, b]),
            ("stereo bed", 0, 2, vec![a, b, b]),
        ]);

        let soun = SoundTag::from_bytes(&data).expect("Should parse synthetic sound");
        assert_eq!(soun.loudness, 2);
        assert_eq!(soun.pitch_lower_bound, 0xe5);
        assert_eq!(soun.pitch_delta, 0x3c);
        assert_eq!(soun.volume_lower_bound, 0xaf);
        assert_eq!(soun.volume_delta, 0x1e);
        assert_eq!(soun.first_subtitle_index, -2);
        assert_eq!(soun.subtitle_string_list, None);
        assert_eq!(soun.permutations.len(), 2);
        assert_eq!(soun.permutations[0].name, "Sword Swing 1");
        assert_eq!(soun.permutations[0].skip_fraction, 0x3333);
        assert_eq!(soun.permutations[0].adpcm_data[..34], a);
        assert_eq!(soun.permutations[1].adpcm_data[34..68], b);

        let mono = soun.decode_permutation(0).unwrap();
        assert_eq!((mono.channels, mono.sample_rate), (1, 22050));
        assert_eq!(mono.samples.len(), 128);
        let mut first = [0i16; 64];
        decode_ima4_packet(&a, &mut first);
        assert_eq!(mono.samples[..64], first);

        let st = soun.decode_permutation(1).unwrap();
        assert_eq!(st.channels, 2);
        assert_eq!(st.frame_count(), 64);
        assert_eq!(soun.permutations[1].frame_count(), 64);
        let mut right = [0i16; 64];
        decode_ima4_packet(&b, &mut right);
        assert_eq!((st.samples[0], st.samples[1]), (first[0], right[0]));
        assert_eq!((st.samples[126], st.samples[127]), (first[63], right[63]));

        let wav = mono.to_wav_bytes().expect("Should export WAV");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 128 * 2);

        assert_eq!(soun.random_pitch(|| 0), 0xe5);
        assert_eq!(soun.random_pitch(|| 0xffff), 0xe5 + 0x3b);
        assert_eq!(soun.random_volume(|| 0x8000), 0xaf + 0x0f);
    }

    #[test]
    fn test_sound_tag_rejects_truncated_packets() {
        let a = encode_ima4_packet(&sine_packet(0.0, 4000.0), 0, 30);
        let mut data = build_soun(&[("x", 0, 1, vec![a, a])]);
        data.truncate(data.len() - 1);
        let size = u32::from_be_bytes(data[24..28].try_into().unwrap()) - 1;
        data[24..28].copy_from_slice(&size.to_be_bytes());
        assert!(SoundTag::from_bytes(&data).is_err());
    }

    fn three_perm_tag(skips: [u16; 3]) -> SoundTag {
        let a = encode_ima4_packet(&sine_packet(0.0, 4000.0), 0, 30);
        let data = build_soun(&[
            ("p0", skips[0], 1, vec![a]),
            ("p1", skips[1], 1, vec![a]),
            ("p2", skips[2], 1, vec![a]),
        ]);
        SoundTag::from_bytes(&data).unwrap()
    }

    #[test]
    fn test_choose_permutation_cycles_without_repeats() {
        let soun = three_perm_tag([0, 0, 0]);
        let mut history = PermutationHistory::default();
        let mut seq = [0u16, 2, 0, 0, 2, 0, 0, 2, 0].into_iter();
        let mut rng = || seq.next().unwrap_or(0);
        let picks: Vec<_> = (0..3)
            .map(|_| soun.choose_permutation(&mut history, &mut rng).unwrap())
            .collect();
        assert_eq!(picks, vec![2, 1, 0]);
        assert_eq!(history.played_mask, 0b111);
        let mut rng = || 1u16;
        assert_eq!(soun.choose_permutation(&mut history, &mut rng), Some(1));
        assert_eq!(history.played_mask, 0b010);
    }

    #[test]
    fn test_choose_permutation_skip_and_play_fraction() {
        let soun = three_perm_tag([0xffff, 0, 0]);
        let mut history = PermutationHistory::default();
        let mut seq = [0u16, 0, 0x1234, 0].into_iter();
        let pick = soun.choose_permutation(&mut history, || seq.next().unwrap_or(0));
        assert_eq!(pick, Some(2));
        assert_eq!(history.played_mask, 0b101);

        let mut quiet = soun.clone();
        quiet.play_fraction = 0x0a3d;
        assert_eq!(quiet.choose_permutation(&mut history, || 0x0a3e), None);
    }
}
