use crate::engine_rng::Unmodelled;
use crate::map_actions::{MapActionContext, MapActions, TYPE_DISTANCE, TYPE_MONSTERS};
use mandrake_common::FourCC;

const MAX_SUBJECTS: usize = 400;

const TYPE_SOUND_SOURCE: i16 = 0x11;

const TYPE_PROJECTILE: i16 = 0xc;

pub const MAXIMUM_SOUND_SOURCES: usize = 0x40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundSource {
    pub identifier: i16,
    pub location: [i32; 3],
}

pub fn sound_sources(
    markers: &[mandrake_tag::MarkerEntry],
    palettes: &[mandrake_tag::MarkerPaletteEntry],
    amso_exists: &dyn Fn(FourCC) -> bool,
) -> Vec<SoundSource> {
    let palettes: Vec<&mandrake_tag::MarkerPaletteEntry> =
        palettes.iter().filter(|p| p.marker_type == 5).collect();
    let mut out: Vec<SoundSource> = Vec::new();
    let mut next_zero = 1000i16;
    let mut next_identifier = 0x8000u16;
    for m in markers.iter().filter(|m| m.marker_type == 5) {
        let mut identifier = m.marker_id;
        if identifier == 0 {
            identifier = next_zero;
            next_zero = next_zero.wrapping_add(1);
        }
        let Some(p) = usize::try_from(m.palette_index)
            .ok()
            .and_then(|k| palettes.get(k))
        else {
            continue;
        };
        if !amso_exists(p.tag_id) || out.len() >= MAXIMUM_SOUND_SOURCES {
            continue;
        }
        next_identifier = match next_identifier.wrapping_add(1) {
            0 => 0x8000,
            n => n,
        };
        if out.iter().any(|s| s.identifier == identifier) {
            identifier = next_identifier as i16;
        }
        out.push(SoundSource {
            identifier,
            location: [m.x, m.y, m.z],
        });
    }
    out
}

impl MapActions {
    pub fn lightning_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let subjects: Vec<usize> = self
            .combined(i, b"subj", TYPE_MONSTERS, MAX_SUBJECTS)
            .into_iter()
            .filter_map(|id| ctx.world.monster_index_by_identifier(id as u16))
            .collect();
        if subjects.is_empty() {
            return Ok(0);
        }
        let Some(sound) = self.indexed(i, b"soun", TYPE_SOUND_SOURCE, 0) else {
            return Ok(0);
        };
        let Some(radius) = self.long(i, b"radi", TYPE_DISTANCE) else {
            return Ok(0);
        };
        let Some(projectile) = self.long(i, b"proj", TYPE_PROJECTILE) else {
            return Ok(0);
        };
        let sound = sound as i16;
        let origin = ctx
            .world
            .sound_sources
            .iter()
            .find(|s| s.identifier != 0 && s.identifier == sound)
            .map_or([0; 3], |s| s.location);
        let mut best = -1i32;
        let mut target = -1i16;
        for &m in &subjects {
            let (x, y) = ctx.world.monsters[m].position;
            let d = crate::engine_move::distance2d(
                origin[0].wrapping_sub(x),
                origin[1].wrapping_sub(y),
            );
            if d < radius && (best == -1 || d < best) {
                best = d;
                target = m as i16;
            }
        }
        let desc = crate::projectile_groups::ProjectileDescriptor {
            definition: FourCC::new((projectile as u32).to_be_bytes()),
            flags: 1,
            position: origin,
            velocity: [0; 3],
            facing: 0,
            elevation: 0,
            word_1c: 0x100,
            owner_monster: -1,
            parent: -1,
            owner_object: -1,
            target,
            target_point: [0; 3],
        };
        crate::projectile_groups::new_projectile(
            &mut crate::projectile_groups::EngineSpawn {
                world: &mut *ctx.world,
                rng: &mut *ctx.rng,
                source: None,
                host: None,
            },
            &desc,
        )?;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mandrake_tag::{MarkerEntry, MarkerPaletteEntry};

    fn marker(kind: i16, palette: i16, identifier: i16, x: i32) -> MarkerEntry {
        MarkerEntry {
            flags: 0,
            marker_type: kind,
            palette_index: palette,
            marker_id: identifier,
            difficulty_flags: 0,
            x,
            y: 2 * x,
            z: -x,
            yaw: 0,
            pitch: 0,
            roll: 0,
            type_data: [0; 32],
        }
    }

    fn palette(kind: i16, tag: &[u8; 4]) -> MarkerPaletteEntry {
        MarkerPaletteEntry {
            marker_type: kind,
            flags: 0,
            tag_id: FourCC::new(*tag),
            user_data: [0; 24],
        }
    }

    #[test]
    fn sound_sources_follow_new_sound_source() {
        let palettes = [
            palette(3, b"unit"),
            palette(5, b"good"),
            palette(5, b"miss"),
        ];
        let markers = [
            marker(5, 0, 0, 1),
            marker(3, 0, 0, 2),
            marker(5, 1, 0, 3),
            marker(5, 0, 0, 4),
            marker(5, 0, 1001, 5),
            marker(5, 7, 77, 6),
            marker(5, 0, 1000, 7),
        ];
        let s = sound_sources(&markers, &palettes, &|t| t.0 == *b"good");
        let ids: Vec<(i16, i32)> = s.iter().map(|s| (s.identifier, s.location[0])).collect();
        assert_eq!(
            ids,
            vec![(1000, 1), (1002, 4), (1001, 5), (0x8004u16 as i16, 7)]
        );
        assert_eq!(s[0].location, [1, 2, -1]);
    }

    #[test]
    fn at_most_64_sound_sources() {
        let palettes = [palette(5, b"good")];
        let markers: Vec<MarkerEntry> = (0..70).map(|k| marker(5, 0, 100 + k, 0)).collect();
        let s = sound_sources(&markers, &palettes, &|_| true);
        assert_eq!(s.len(), MAXIMUM_SOUND_SOURCES);
        assert_eq!(s.last().unwrap().identifier, 163);
    }
}
