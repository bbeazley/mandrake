use mandrake_common::FourCC;

use crate::draw_site;
use crate::{Stop, TflSim};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundKind {
    SequenceFirst,
    SequenceLast,
    SequenceKey,
    MonsterVoice(i8),
    ProjectileGroup,
    ProjectileRebound,
    SoundAction {
        action: usize,
        candidates: Vec<usize>,
    },
    GameSound(i16),
    AmbientVolume {
        identifier: i16,
        volume: i16,
        ticks: i16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundEvent {
    pub tick: u32,
    pub site: u32,
    pub kind: SoundKind,
    pub sound: Option<FourCC>,
    pub object: Option<usize>,
    pub world: Option<[i32; 3]>,
    pub velocity: [i16; 3],
    pub volume: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundSource {
    pub marker: usize,
    pub identifier: i16,
    pub ambient: FourCC,
    pub world: [i32; 3],
    pub volume: i16,
}

impl TflSim {
    pub fn take_sound_events(&mut self) -> Vec<SoundEvent> {
        std::mem::take(&mut self.sound_events)
    }

    pub fn sound_sources(&self) -> &[SoundSource] {
        &self.sound_sources
    }

    pub(crate) fn object_sound(
        &mut self,
        site: u32,
        kind: SoundKind,
        object: usize,
        sound: Option<FourCC>,
    ) {
        let Some(sound) = sound else { return };
        let o = &self.objects[object];
        let event = SoundEvent {
            tick: self.tick,
            site,
            kind,
            sound: Some(sound),
            object: Some(object),
            world: Some([o.x, o.y, o.z]),
            velocity: o.velocity,
            volume: 0x100,
        };
        self.sound_events.push(event);
    }

    pub(crate) fn flat_sound(&mut self, site: u32, kind: SoundKind, sound: Option<FourCC>) {
        let event = SoundEvent {
            tick: self.tick,
            site,
            kind,
            sound,
            object: None,
            world: None,
            velocity: [0; 3],
            volume: 0x100,
        };
        self.sound_events.push(event);
    }

    fn voice_of(&mut self, m: usize, kind: i8) -> Result<Option<FourCC>, Stop> {
        let mons = self.data.def(b"mons", self.monsters[m].definition)?;
        Ok((0..10)
            .find(|&k| mons.0.get(0x1a0 + k).is_some_and(|&b| b as i8 == kind))
            .and_then(|k| mons.tag(0x14c + 4 * k)))
    }

    pub(crate) fn monster_voice(&mut self, site: u32, m: usize, kind: i8) -> Result<(), Stop> {
        let sound = self.voice_of(m, kind)?;
        let object = self.monsters[m].object;
        self.object_sound(site, SoundKind::MonsterVoice(kind), object, sound);
        Ok(())
    }

    pub fn monsters_voice(
        &mut self,
        monsters: &[usize],
        kind: i8,
    ) -> Result<Vec<SoundEvent>, Stop> {
        let mut counts: Vec<(FourCC, usize)> = Vec::new();
        for &m in monsters {
            let d = self.monsters[m].definition;
            match counts.iter_mut().find(|(e, _)| *e == d) {
                Some((_, n)) => *n += 1,
                None => counts.push((d, 1)),
            }
        }
        let Some(&(most, n)) = counts
            .iter()
            .fold(None, |b: Option<&(FourCC, usize)>, c| match b {
                Some(b) if b.1 >= c.1 => Some(b),
                _ => Some(c),
            })
        else {
            return Ok(Vec::new());
        };
        let kind = match kind {
            0 if n > 2 => 1,
            3 if n > 2 => 4,
            5 if n > 2 => 6,
            k => k,
        };
        let kept = std::mem::take(&mut self.sound_events);
        for &m in monsters {
            if self.monsters[m].definition == most {
                let sound = self.voice_of(m, kind)?;
                let object = self.monsters[m].object;
                self.object_sound(
                    draw_site::MONSTER_VOICE,
                    SoundKind::MonsterVoice(kind),
                    object,
                    sound,
                );
            }
        }
        Ok(std::mem::replace(&mut self.sound_events, kept))
    }

    pub(crate) fn game_sound(&mut self, site: u32, id: i16) {
        self.flat_sound(site, SoundKind::GameSound(id), None);
    }

    pub(crate) fn add_sound_sources(&mut self) {
        let palettes = &self.data.mesh.palettes;
        self.sound_sources = self
            .data
            .mesh
            .markers
            .iter()
            .enumerate()
            .filter(|(_, m)| m.marker_type == 5)
            .filter_map(|(k, m)| {
                let p = palettes
                    .iter()
                    .filter(|p| p.marker_type == 5)
                    .nth(m.palette_index.max(0) as usize)?;
                let t = p.tag_id;
                (t.0 != [0xff; 4] && t.0 != [0; 4]).then_some(SoundSource {
                    marker: k,
                    identifier: m.marker_id,
                    ambient: t,
                    world: [m.x, m.y, m.z],
                    volume: if m.flags & 1 != 0 { 0 } else { 0x100 },
                })
            })
            .collect();
    }
}
