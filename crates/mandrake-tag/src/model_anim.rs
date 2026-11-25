use byteorder::{BigEndian, ReadBytesExt};
use mandrake_common::{FourCC, M2Error};
use std::io::{Cursor, Read};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAnimationFrame {
    pub flags: u32,
    pub model_tag: FourCC,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelAnimationTag {
    pub flags: u32,
    pub ticks_per_frame: i16,
    pub frames: Vec<ModelAnimationFrame>,
    pub sounds: [Option<FourCC>; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAnimationShadowMaps {
    pub width: i16,
    pub height: i16,
    pub row_bytes: i16,
    pub offset: [i16; 2],
    pub pixels: Vec<u8>,
}

impl ModelAnimationShadowMaps {
    pub fn from_tag_data(data: &[u8]) -> Option<Self> {
        let be16 = |o: usize| Some(i16::from_be_bytes(data.get(o..o + 2)?.try_into().ok()?));
        let be32 = |o: usize| Some(i32::from_be_bytes(data.get(o..o + 4)?.try_into().ok()?));
        let (start, size) = (
            usize::try_from(be32(0x3F4)?).ok()?,
            usize::try_from(be32(0x3F8)?).ok()?,
        );
        if size == 0 {
            return None;
        }
        Some(Self {
            width: be16(0x3B4)?,
            height: be16(0x3B6)?,
            row_bytes: be16(0x3B8)?,
            offset: [be16(0x3BC)?, be16(0x3BE)?],
            pixels: data.get(start..start.checked_add(size)?)?.to_vec(),
        })
    }

    pub fn frame(&self, frame: usize) -> Option<&[u8]> {
        let height = usize::try_from(self.height).ok()?;
        let row_bytes = usize::try_from(self.row_bytes).ok()?;
        let size = height.checked_mul(row_bytes)?;
        let start = frame.checked_mul(size)?;
        let end = start.checked_add(size)?;
        self.pixels.get(start..end)
    }
}

impl ModelAnimationTag {
    pub const TAG_TYPE: [u8; 4] = *b"anim";
    pub const DEFINITION_SIZE: usize = 0x400;
    pub const FRAMES_OFFSET: usize = 0x08;
    pub const FRAME_SIZE: usize = 0x10;
    pub const FRAME_TABLE_END: usize = 0x3B0;
    pub const SOUNDS_OFFSET: usize = 0x3C0;
    pub const MAXIMUM_FRAMES: usize =
        (Self::FRAME_TABLE_END - Self::FRAMES_OFFSET) / Self::FRAME_SIZE;

    pub const FLAG_RANDOM_START_FRAME: u32 = 0x1;
    pub const FLAG_LOOPS: u32 = 0x2;
    pub const FLAG_PING_PONG: u32 = 0x4;
    pub const FLAG_STARTS_ACTIVE: u32 = 0x8;

    pub fn from_bytes(data: &[u8]) -> Result<Self, M2Error> {
        if data.len() < Self::FRAMES_OFFSET {
            return Err(M2Error::TagParse(format!(
                "Model animation data too short: expected >= {} bytes, got {}",
                Self::FRAMES_OFFSET,
                data.len()
            )));
        }
        let mut cursor = Cursor::new(data);
        let flags = cursor.read_u32::<BigEndian>()?;
        let frame_count = cursor.read_i16::<BigEndian>()?;
        let ticks_per_frame = cursor.read_i16::<BigEndian>()?;

        if frame_count < 0 || frame_count as usize > Self::MAXIMUM_FRAMES {
            return Err(M2Error::TagParse(format!(
                "Model animation frame count {} out of range 0..={}",
                frame_count,
                Self::MAXIMUM_FRAMES
            )));
        }
        let frame_count = frame_count as usize;
        if data.len() < Self::FRAMES_OFFSET + frame_count * Self::FRAME_SIZE {
            return Err(M2Error::TagParse(format!(
                "Model animation data too short for {} frames: {} bytes",
                frame_count,
                data.len()
            )));
        }

        let mut frames = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            cursor.set_position((Self::FRAMES_OFFSET + i * Self::FRAME_SIZE) as u64);
            let frame_flags = cursor.read_u32::<BigEndian>()?;
            let mut tag = [0u8; 4];
            cursor.read_exact(&mut tag)?;
            frames.push(ModelAnimationFrame {
                flags: frame_flags,
                model_tag: FourCC::new(tag),
            });
        }

        let mut sounds = [None; 2];
        for (i, sound) in sounds.iter_mut().enumerate() {
            let o = Self::SOUNDS_OFFSET + 4 * i;
            if let Some(bytes) = data.get(o..o + 4) {
                *sound = match u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) {
                    0 | 0xffff_ffff => None,
                    _ => Some(FourCC::new([bytes[0], bytes[1], bytes[2], bytes[3]])),
                };
            }
        }

        Ok(Self {
            flags,
            ticks_per_frame,
            frames,
            sounds,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn starts_active(&self) -> bool {
        self.flags & Self::FLAG_STARTS_ACTIVE != 0
    }

    pub fn ticks_per_displayed_frame(&self) -> u32 {
        (self.ticks_per_frame.max(0) as u32) + 1
    }

    pub fn resolve_frame_geometry_tags(&self, archive: &crate::TagArchive) -> Vec<Option<FourCC>> {
        self.frames
            .iter()
            .map(|f| {
                let entry = archive.find_tag_by_id(FourCC::new(*b"mode"), f.model_tag)?;
                let data = archive.get_tag_data(entry).ok()?;
                crate::ModelTag::from_bytes(&data)
                    .ok()
                    .map(|m| m.geometry_tag)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelAnimationPlayback {
    pub frame: usize,
    pub tick_counter: i16,
    pub forward: bool,
    pub ping_pong: bool,
    pub looping: bool,
    pub active: bool,
}

impl ModelAnimationPlayback {
    pub fn new(def: &ModelAnimationTag, start_frame: usize) -> Self {
        let frame = if def.flags & ModelAnimationTag::FLAG_RANDOM_START_FRAME != 0 {
            start_frame.min(def.frame_count().saturating_sub(1))
        } else {
            0
        };
        Self {
            frame,
            tick_counter: 0,
            forward: true,
            ping_pong: def.flags & ModelAnimationTag::FLAG_PING_PONG != 0,
            looping: def.flags & ModelAnimationTag::FLAG_LOOPS != 0,
            active: def.starts_active(),
        }
    }

    pub fn creation_sound(&self, def: &ModelAnimationTag) -> Option<FourCC> {
        if self.active { def.sounds[0] } else { None }
    }

    pub fn tick(&mut self, def: &ModelAnimationTag) -> Option<FourCC> {
        if !self.active {
            return None;
        }
        self.tick_counter += 1;
        if self.tick_counter > def.ticks_per_frame {
            self.tick_counter = 0;
            if def.frame_count() > 1 {
                return self.advance(def).and_then(|slot| def.sounds[slot]);
            }
        }
        None
    }

    pub fn advance(&mut self, def: &ModelAnimationTag) -> Option<usize> {
        let count = def.frame_count() as i32;
        let next = self.frame as i32 + if self.forward { 1 } else { -1 };
        let mut sound = None;
        let target = if self.looping {
            if next >= count {
                sound = Some(0);
                0
            } else if next < 0 {
                sound = Some(1);
                count - 1
            } else {
                next
            }
        } else if self.ping_pong {
            if next >= count {
                self.forward = false;
                sound = Some(1);
                count - 1
            } else if next < 0 {
                self.forward = true;
                sound = Some(0);
                0
            } else {
                next
            }
        } else if next >= count {
            self.active = false;
            count - 1
        } else if next < 0 {
            self.active = false;
            0
        } else {
            next
        };
        self.frame = target.max(0) as usize;
        self.tick_counter = 0;
        sound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_maps_come_from_the_header_and_go_by_frame() {
        let mut data = vec![0u8; ModelAnimationTag::DEFINITION_SIZE + 12];
        assert_eq!(ModelAnimationShadowMaps::from_tag_data(&data), None);
        data[0x3B4..0x3B6].copy_from_slice(&2i16.to_be_bytes());
        data[0x3B6..0x3B8].copy_from_slice(&2i16.to_be_bytes());
        data[0x3B8..0x3BA].copy_from_slice(&3i16.to_be_bytes());
        data[0x3BC..0x3BE].copy_from_slice(&(-64i16).to_be_bytes());
        data[0x3BE..0x3C0].copy_from_slice(&128i16.to_be_bytes());
        data[0x3F4..0x3F8].copy_from_slice(&0x400i32.to_be_bytes());
        data[0x3F8..0x3FC].copy_from_slice(&12i32.to_be_bytes());
        for (i, b) in data[0x400..].iter_mut().enumerate() {
            *b = i as u8;
        }
        let maps = ModelAnimationShadowMaps::from_tag_data(&data).expect("maps");
        assert_eq!((maps.width, maps.height, maps.row_bytes), (2, 2, 3));
        assert_eq!(maps.offset, [-64, 128]);
        assert_eq!(maps.frame(1), Some(&[6, 7, 8, 9, 10, 11][..]));
        assert_eq!(maps.frame(2), None);
    }

    fn synthetic(flags: u32, frames: &[&[u8; 4]], ticks_per_frame: i16) -> Vec<u8> {
        let mut data = vec![0u8; ModelAnimationTag::DEFINITION_SIZE];
        data[0x3C0..0x3C4].copy_from_slice(b"snd0");
        data[0x3C4..0x3C8].copy_from_slice(&[0xff; 4]);
        data[0..4].copy_from_slice(&flags.to_be_bytes());
        data[4..6].copy_from_slice(&(frames.len() as i16).to_be_bytes());
        data[6..8].copy_from_slice(&ticks_per_frame.to_be_bytes());
        for (i, tag) in frames.iter().enumerate() {
            let o = ModelAnimationTag::FRAMES_OFFSET + i * ModelAnimationTag::FRAME_SIZE;
            data[o + 4..o + 8].copy_from_slice(*tag);
        }
        data
    }

    #[test]
    fn test_parse_synthetic_anim() {
        let data = synthetic(0xa, &[b"mill", b"mi01", b"mi02"], 4);
        let anim = ModelAnimationTag::from_bytes(&data).unwrap();
        assert_eq!(anim.frame_count(), 3);
        assert_eq!(anim.ticks_per_frame, 4);
        assert_eq!(anim.frames[1].model_tag, FourCC::new(*b"mi01"));
        assert!(anim.starts_active());
        assert_eq!(anim.ticks_per_displayed_frame(), 5);
        assert_eq!(anim.sounds, [Some(FourCC::new(*b"snd0")), None]);
    }

    #[test]
    fn test_parse_rejects_bad_frame_count() {
        let mut data = synthetic(0, &[b"mill"], 4);
        data[4..6].copy_from_slice(&(-1i16).to_be_bytes());
        assert!(ModelAnimationTag::from_bytes(&data).is_err());
        assert!(ModelAnimationTag::from_bytes(&data[..4]).is_err());
    }

    #[test]
    fn test_looping_playback_matches_engine_timing() {
        let anim = ModelAnimationTag::from_bytes(&synthetic(
            ModelAnimationTag::FLAG_LOOPS | ModelAnimationTag::FLAG_STARTS_ACTIVE,
            &[b"f000", b"f001", b"f002"],
            4,
        ))
        .unwrap();
        let mut p = ModelAnimationPlayback::new(&anim, 0);
        assert_eq!(p.creation_sound(&anim), Some(FourCC::new(*b"snd0")));
        let mut seen = Vec::new();
        let mut sound_ticks = Vec::new();
        for t in 1..=30 {
            if p.tick(&anim).is_some() {
                sound_ticks.push(t);
            }
            seen.push(p.frame);
        }
        let expected: Vec<usize> = (1..=30).map(|t| (t / 5) % 3).collect();
        assert_eq!(seen, expected);
        assert!(p.active);
        assert_eq!(sound_ticks, vec![15, 30]);
    }

    #[test]
    fn test_inactive_and_one_shot_playback() {
        let idle = ModelAnimationTag::from_bytes(&synthetic(0, &[b"a000", b"a001"], 0)).unwrap();
        let mut p = ModelAnimationPlayback::new(&idle, 1);
        for _ in 0..10 {
            p.tick(&idle);
        }
        assert_eq!((p.frame, p.active), (0, false));

        let once = ModelAnimationTag::from_bytes(&synthetic(
            ModelAnimationTag::FLAG_STARTS_ACTIVE,
            &[b"a000", b"a001", b"a002"],
            0,
        ))
        .unwrap();
        let mut p = ModelAnimationPlayback::new(&once, 0);
        for _ in 0..10 {
            p.tick(&once);
        }
        assert_eq!((p.frame, p.active), (2, false));
    }

    #[test]
    fn test_ping_pong_playback() {
        let anim = ModelAnimationTag::from_bytes(&synthetic(
            ModelAnimationTag::FLAG_PING_PONG | ModelAnimationTag::FLAG_STARTS_ACTIVE,
            &[b"a000", b"a001", b"a002"],
            0,
        ))
        .unwrap();
        let mut p = ModelAnimationPlayback::new(&anim, 0);
        let (seen, sounds): (Vec<usize>, Vec<bool>) = (0..8)
            .map(|_| {
                let sound = p.tick(&anim);
                (p.frame, sound.is_some())
            })
            .unzip();
        assert_eq!(seen, vec![1, 2, 2, 1, 0, 0, 1, 2]);
        assert_eq!(
            sounds,
            vec![false, false, false, false, false, true, false, false]
        );
    }
}
