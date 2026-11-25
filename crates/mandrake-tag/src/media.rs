use crate::archive::TagArchive;
use crate::projectile::{ProjectileDefinition, ProjectileGroupDefinition};
use mandrake_common::{FourCC, M2Error};

pub const MEDIA_DEFINITION_SIZE: usize = 0x100;
pub const MEDIA_PROJECTILE_GROUP_COUNT: usize = 16;

pub const MEDIA_GROUP_PROJECTILE: usize = 4;
pub const MEDIA_GROUP_PROJECTILE_FLAG_10000: usize = 5;
pub const MEDIA_GROUP_MONSTER: usize = 6;

pub fn projectile_media_group_type(projectile_flags: u32) -> usize {
    if projectile_flags & 0x10000 != 0 {
        MEDIA_GROUP_PROJECTILE_FLAG_10000
    } else {
        MEDIA_GROUP_PROJECTILE
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDefinition {
    pub tag_id: FourCC,
    pub flags: u32,
    pub collection_reference: Option<FourCC>,
    pub projectile_groups: [Option<FourCC>; MEDIA_PROJECTILE_GROUP_COUNT],
    pub reflection_tint_value: i16,
    pub reflection_tint_data: [u8; 8],
    pub local_effect_chance: u16,
    pub local_projectile_group: Option<FourCC>,
    pub wobble_amplitude: [i16; 3],
    pub wobble_frequency: [i16; 3],
    pub local_effect_count: i16,
    pub local_effect_period: i16,
    pub reflection_transparency: u16,
}

pub const MEDIA_FLAG_NO_REFLECTIONS: u32 = 0x1;

use crate::parse::optional_fourcc as tag_ref;

impl MediaDefinition {
    pub fn from_bytes(tag_id: FourCC, d: &[u8]) -> Result<Self, M2Error> {
        if d.len() < MEDIA_DEFINITION_SIZE {
            return Err(M2Error::TagParse(format!(
                "medi '{}' too short: {} bytes",
                tag_id,
                d.len()
            )));
        }
        let be16 = |o: usize| i16::from_be_bytes([d[o], d[o + 1]]);
        let mut reflection_tint_data = [0u8; 8];
        reflection_tint_data.copy_from_slice(&d[0x5a..0x62]);
        Ok(Self {
            tag_id,
            flags: u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
            collection_reference: tag_ref(d, 0x04),
            projectile_groups: std::array::from_fn(|i| tag_ref(d, 0x18 + 4 * i)),
            reflection_tint_value: be16(0x58),
            reflection_tint_data,
            local_effect_chance: be16(0x62) as u16,
            local_projectile_group: tag_ref(d, 0x64),
            wobble_amplitude: [be16(0x68), be16(0x6a), be16(0x6c)],
            wobble_frequency: [be16(0x6e), be16(0x70), be16(0x72)],
            local_effect_count: be16(0x74),
            local_effect_period: be16(0x76),
            reflection_transparency: be16(0x78) as u16,
        })
    }

    pub fn no_reflections(&self) -> bool {
        self.flags & MEDIA_FLAG_NO_REFLECTIONS != 0
    }

    pub fn projectile_group(&self, group_type: usize) -> Option<FourCC> {
        self.projectile_groups.get(group_type).copied().flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEffectSprite {
    pub group: FourCC,
    pub projectile: FourCC,
    pub collection_reference: FourCC,
    pub collection: FourCC,
    pub sequence: i16,
    pub sequence_name: String,
    pub frames: u16,
    pub ticks_per_frame: i16,
}

impl MediaEffectSprite {
    pub fn sequence_ticks(&self) -> u32 {
        u32::from(self.frames) * u32::from(self.ticks_per_frame.max(0) as u16)
    }
}

fn find(archives: &[&TagArchive], kind: &[u8; 4], id: FourCC) -> Option<Vec<u8>> {
    archives.iter().rev().find_map(|a| {
        let e = a.find_tag_by_id(FourCC::new(*kind), id)?;
        a.get_tag_data(e).ok()
    })
}

pub fn media_effect_sprites(
    archives: &[&TagArchive],
    media: &MediaDefinition,
    group_type: usize,
) -> Vec<MediaEffectSprite> {
    let Some(group) = media.projectile_group(group_type) else {
        return Vec::new();
    };
    let Some(prgr) = find(archives, b"prgr", group)
        .and_then(|d| ProjectileGroupDefinition::from_bytes(group, &d).ok())
    else {
        return Vec::new();
    };
    let mut collections: std::collections::HashMap<FourCC, Option<Vec<u8>>> =
        std::collections::HashMap::new();
    prgr.parts
        .iter()
        .filter_map(|part| {
            let projectile = part.projectile?;
            let proj = find(archives, b"proj", projectile)
                .and_then(|d| ProjectileDefinition::from_bytes(projectile, &d).ok())?;
            let collection_reference = proj.collection?;
            let core = find(archives, b"core", collection_reference)?;
            let collection = tag_ref(&core, 0)?;
            let coll_bytes = collections
                .entry(collection)
                .or_insert_with(|| find(archives, b".256", collection))
                .as_deref()?;
            let sequence = proj.sequences[0];
            let (sequence_name, frames, ticks_per_frame) =
                crate::sequence_header::sequence_summary(
                    coll_bytes,
                    usize::try_from(sequence).ok()?,
                )?;
            Some(MediaEffectSprite {
                group,
                projectile,
                collection_reference,
                collection,
                sequence,
                sequence_name,
                frames,
                ticks_per_frame,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wate_bytes() -> Vec<u8> {
        let mut d = vec![0u8; 256];
        d[4..8].copy_from_slice(b"wate");
        d[8..12].copy_from_slice(&[0x00, 0x02, 0x00, 0x07]);
        d[12..0x18].fill(0xff);
        for (i, r) in [
            b"wass", b"wasm", b"wasl", b"wasb", b"wasf", b"wa#1", b"wadi",
        ]
        .iter()
        .enumerate()
        {
            d[0x18 + 4 * i..0x1c + 4 * i].copy_from_slice(*r);
        }
        d[0x34..0x58].fill(0xff);
        d[0x58..0x5a].copy_from_slice(&[0x00, 0xc0]);
        d[0x64..0x68].fill(0xff);
        d[0x68..0x76].copy_from_slice(&[
            0x00, 0x33, 0x00, 0x33, 0x00, 0x66, 0x01, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00,
        ]);
        d
    }

    #[test]
    fn wate_media_definition_fields() {
        let m = MediaDefinition::from_bytes(FourCC::new(*b"wate"), &wate_bytes()).unwrap();
        assert_eq!(m.collection_reference, Some(FourCC::new(*b"wate")));
        assert_eq!(m.projectile_group(0), Some(FourCC::new(*b"wass")));
        assert_eq!(
            m.projectile_group(MEDIA_GROUP_PROJECTILE),
            Some(FourCC::new(*b"wasf"))
        );
        assert_eq!(
            m.projectile_group(MEDIA_GROUP_PROJECTILE_FLAG_10000),
            Some(FourCC::new(*b"wa#1"))
        );
        assert_eq!(
            m.projectile_group(MEDIA_GROUP_MONSTER),
            Some(FourCC::new(*b"wadi"))
        );
        assert_eq!(m.projectile_group(7), None);
        assert_eq!(m.projectile_group(16), None);
        assert_eq!(m.reflection_tint_value, 0xc0);
        assert_eq!(m.local_projectile_group, None);
        assert_eq!(m.local_effect_count, 0);
        assert_eq!(m.wobble_amplitude, [0x33, 0x33, 0x66]);
        assert_eq!(m.wobble_frequency, [0x100, 0x100, 0x200]);
        assert!(MediaDefinition::from_bytes(FourCC::new(*b"wate"), &[0; 10]).is_err());
        assert!(!m.no_reflections());
        assert_eq!(m.reflection_transparency, 0);
    }

    #[test]
    fn projectile_group_type_follows_flag_0x10000() {
        assert_eq!(
            projectile_media_group_type(0x0020_4098),
            MEDIA_GROUP_PROJECTILE
        );
        assert_eq!(
            projectile_media_group_type(0x0001_0000),
            MEDIA_GROUP_PROJECTILE_FLAG_10000
        );
    }
}
