use crate::audio_out::AudioLink;
use crate::console::SoundSubtitles;
use crate::map_sound::{GameSounds, MapSound};
use crate::tfl_map::TflMap;
use mandrake_common::FourCC;
use mandrake_sim::combat::SequenceSound;
use mandrake_tag::TagArchive;
use mandrake_tag::sound_manager::{
    AmbientSoundSource, ChannelObject, ObjectPlace, SoundListener, SoundManager, SoundRules,
    SoundVolumes, WorldPoint,
};
use mandrake_tfl_sim::{SoundEvent, SoundKind, SoundSource, TflSim};
use tracing::{info, warn};

pub(crate) const TFL_LISTENER_AHEAD: i32 = 0x1400;
const MYTH2_WHISPER_ENTRY: usize = 0x16;
const SOUND_ACTION_RANGE: u32 = 0x4000;

fn tfl_distance(a: WorldPoint, b: WorldPoint) -> i32 {
    mandrake_tfl_sim::groups::distance((a.x, a.y), (b.x, b.y)) as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TflSoundRules {
    #[default]
    Myth2,
    Tfl,
}

impl TflSoundRules {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "myth2" => Some(Self::Myth2),
            "tfl" => Some(Self::Tfl),
            _ => None,
        }
    }

    pub(crate) fn rules(self) -> SoundRules {
        match self {
            Self::Myth2 => SoundRules::MYTH2,
            Self::Tfl => tfl_rules(),
        }
    }

    pub(crate) fn listener(self, observer: [i32; 3], facing: u16) -> SoundListener {
        match self {
            Self::Myth2 => SoundListener::from_observer(
                WorldPoint::new(observer[0], observer[1], observer[2]),
                facing,
            ),
            Self::Tfl => tfl_listener(observer, facing),
        }
    }
}

pub(crate) fn tfl_rules() -> SoundRules {
    SoundRules {
        arctangent: mandrake_tfl_sim::trig::arctangent,
        distance: tfl_distance,
        ..SoundRules::TFL
    }
}

pub(crate) fn tfl_listener(observer: [i32; 3], facing: u16) -> SoundListener {
    let trig = mandrake_tfl_sim::trig::trig();
    let k = mandrake_tfl_sim::trig::index(i32::from(facing));
    let (c, s) = (i32::from(trig.cosine[k]), i32::from(trig.sine[k]));
    SoundListener {
        location: WorldPoint::new(
            observer[0] + ((c * TFL_LISTENER_AHEAD) >> 14),
            observer[1] + ((s * TFL_LISTENER_AHEAD) >> 14),
            observer[2],
        ),
        facing,
    }
}

#[derive(Debug, Clone, Copy)]
struct Ramp {
    identifier: i16,
    volume: i16,
    target: i16,
    start: Option<u32>,
    ticks: i16,
}

impl Ramp {
    fn volume_at(&mut self, now: u32) -> i16 {
        let Some(start) = self.start else {
            return self.volume;
        };
        let elapsed = now.wrapping_sub(start) as i32;
        if elapsed < i32::from(self.ticks) && elapsed > 0 {
            let (v, t) = (i32::from(self.volume), i32::from(self.target));
            (v + (t - v) * elapsed / i32::from(self.ticks)) as i16
        } else {
            self.start = None;
            self.volume = self.target;
            self.target
        }
    }

    fn adjust(&mut self, volume: i16, ticks: i16, now: u32) {
        if (self.start.is_none() && self.volume != volume)
            || (self.start.is_some() && self.target != volume)
        {
            self.target = volume;
            self.start = Some(now);
            self.ticks = ticks;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Played {
    pub(crate) tick: u32,
    pub(crate) sound: FourCC,
    pub(crate) channel: usize,
    pub(crate) location: Option<WorldPoint>,
    pub(crate) volumes: SoundVolumes,
}

pub(crate) struct TflLevelSound {
    link: Option<AudioLink>,
    ambient: Option<FourCC>,
    sound: MapSound,
    ramps: Vec<Ramp>,
    game_sound_tags: Vec<Option<FourCC>>,
    game_sounds: GameSounds,
    time: u32,
    played: Vec<Played>,
    particles_loaded: bool,
    subtitles: SoundSubtitles,
    captions: Vec<Vec<u8>>,
    rules: TflSoundRules,
    myth2_whisper: Option<FourCC>,
    observer: bool,
}

const PART_AMBIENT: usize = 0x54;

pub(crate) fn mons_list(sim: &TflSim) -> Vec<FourCC> {
    const UNIT: i16 = 3;
    let mut list = Vec::new();
    for p in &sim.data.mesh.palettes {
        if p.marker_type != UNIT || p.team_index() == -1 {
            continue;
        }
        if let Some(mons) = sim.data.peek_def(b"unit", p.tag_id).and_then(|u| u.tag(0))
            && !list.contains(&mons)
        {
            list.push(mons);
        }
    }
    list
}

pub(crate) fn monsters_voice(
    sim: &mut TflSim,
    list: &[FourCC],
    monsters: &[usize],
    kind: i8,
) -> Vec<SoundEvent> {
    let rank = |m: usize| {
        let d = sim.monsters[m].definition;
        list.iter().position(|&e| e == d).unwrap_or(usize::MAX)
    };
    let mut ordered: Vec<usize> = monsters
        .iter()
        .copied()
        .filter(|&m| m < sim.monsters.len())
        .collect();
    ordered.sort_by_key(|&m| rank(m));
    match sim.monsters_voice(&ordered, kind) {
        Ok(events) => events,
        Err(e) => {
            warn!("TFL voice {kind} of {monsters:?}: {}", e.reason);
            Vec::new()
        }
    }
}

const LOCAL_SEED: u32 = 0x1234_5678;

pub(crate) fn level_ambient(map: &TflMap) -> Option<(FourCC, mandrake_tag::AmbientSoundTag)> {
    let id = map.tfl_mesh.ambient_sound_tag?;
    let amso = map.tag(b"amso", id)?;
    mandrake_tag::AmbientSoundTag::from_bytes(&amso)
        .map_err(|e| warn!("TFL amso {id}: {e}"))
        .ok()
        .map(|a| (id, a))
}

impl TflLevelSound {
    pub(crate) fn open(
        map: &TflMap,
        sources: &[SoundSource],
        link: Option<AudioLink>,
        flags: u16,
        rules: TflSoundRules,
    ) -> Self {
        let s = Self::open_with_preferences(map, sources, link, flags, rules);
        info!(
            "TFL level sound: {rules:?}'s rules, sound preference flags {flags}, 3D sound {}, ambient sound {}",
            if s.sound.manager.sound_3d {
                "on"
            } else {
                "off"
            },
            if s.sound.manager.ambient_sound {
                "on"
            } else {
                "off"
            }
        );
        s
    }

    pub(crate) fn open_with_preferences(
        map: &TflMap,
        sources: &[SoundSource],
        link: Option<AudioLink>,
        flags: u16,
        rules: TflSoundRules,
    ) -> Self {
        let mut manager = SoundManager::new(LOCAL_SEED);
        manager.rules = rules.rules();
        manager.map_size = (
            map.tfl_mesh.cell_width as i32,
            map.tfl_mesh.cell_height as i32,
        );
        let mut s = Self {
            link,
            ambient: None,
            sound: MapSound::with_manager(manager),
            ramps: Vec::new(),
            game_sound_tags: Vec::new(),
            game_sounds: GameSounds::default(),
            time: 0,
            played: Vec::new(),
            particles_loaded: false,
            subtitles: SoundSubtitles::default(),
            captions: Vec::new(),
            rules,
            myth2_whisper: None,
            observer: false,
        };
        s.sound.apply_sound_preferences(flags);
        s.sound.use_tfl_device();
        let archives = [&map.archive];
        let mut resolver = mandrake_tag::sound_manager::archive_resolver(&archives);
        match level_ambient(map) {
            Some((id, _)) => {
                s.ambient = Some(id);
                s.sound.manager.global_ambient = s.sound.load_ambient_with(id, &mut resolver);
            }
            None => info!("TFL level sound: the mesh has no ambient sound"),
        }
        for src in sources {
            let Some(definition) = s.sound.load_ambient_with(src.ambient, &mut resolver) else {
                warn!("TFL sound source {}: no amso {}", src.marker, src.ambient);
                continue;
            };
            s.sound.manager.sources.push(AmbientSoundSource {
                definition,
                location: WorldPoint::new(src.world[0], src.world[1], src.world[2]),
                volume: src.volume,
            });
            s.ramps.push(Ramp {
                identifier: src.identifier,
                volume: src.volume,
                target: src.volume,
                start: None,
                ticks: 0,
            });
        }
        if let Some(list) = map.tag(b"soli", FourCC::new(*b"inns")) {
            s.game_sound_tags = list
                .get(0x10..)
                .unwrap_or_default()
                .as_chunks::<8>()
                .0
                .iter()
                .map(|e| {
                    let tag = [e[0], e[1], e[2], e[3]];
                    (tag != [0xff; 4] && tag != [0; 4]).then_some(FourCC(tag))
                })
                .collect();
            for id in s.game_sound_tags.clone().into_iter().flatten() {
                s.sound.load_sound_with(id, &mut resolver);
            }
        }
        s
    }

    pub(crate) fn listener(&self, observer: [i32; 3], facing: u16) -> SoundListener {
        self.rules.listener(observer, facing)
    }

    pub(crate) fn load_myth2_whisper(&mut self, archive_path: &std::path::Path) -> Option<FourCC> {
        let archives = crate::app::sound::open_game_sound_archives(archive_path);
        let archives: Vec<&TagArchive> = archives.iter().collect();
        let id = crate::app::sound::game_sound_tag(&archives, MYTH2_WHISPER_ENTRY);
        let loaded = id.filter(|&id| {
            self.sound
                .load_sound_with(
                    id,
                    &mut mandrake_tag::sound_manager::archive_resolver(&archives),
                )
                .is_some()
        });
        info!(
            "TFL level sound: team chat sound {}",
            loaded.map_or("TFL's (no Myth II data)".to_owned(), |id| format!(
                "Myth II's {id}"
            ))
        );
        self.myth2_whisper = loaded;
        loaded
    }

    pub(crate) fn summary(&self) -> String {
        let m = &self.sound.manager;
        format!(
            "ambient {:?}, {} sound sources, {} amso / {} soun, {} announcer sounds{}",
            self.ambient,
            m.sources.len(),
            m.ambient_definitions.len(),
            m.sounds.len(),
            self.game_sound_tags.iter().flatten().count(),
            if self.link.is_none() { " (muted)" } else { "" }
        )
    }

    pub(crate) fn take_played(&mut self) -> Vec<Played> {
        std::mem::take(&mut self.played)
    }

    pub(crate) fn take_captions(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.captions)
    }

    fn caption(
        &mut self,
        events: &[SoundEvent],
        sim: Option<&TflSim>,
        archive: &TagArchive,
        listener: &SoundListener,
    ) {
        for e in events {
            let Some(id) = e.sound else { continue };
            if let SoundKind::SoundAction { candidates, .. } = &e.kind
                && !candidates.is_empty()
                && sim.is_none_or(|sim| {
                    nearest_seen(sim, candidates, listener, self.observer).is_none()
                })
            {
                continue;
            }
            let get = |group: FourCC, tag: FourCC| crate::tfl_map::tag_data(archive, &group.0, tag);
            if let Some(text) = self.subtitles.play_from(get, id)
                && !text.is_empty()
            {
                self.captions.push(text);
            }
        }
    }

    pub(crate) fn set_observer(&mut self, observer: bool) {
        self.observer = observer;
    }

    pub(crate) fn set_restored_ambient_volumes(&mut self, volumes: &[(i16, i16)]) {
        for &(identifier, volume) in volumes {
            if let Some(i) = self.ramps.iter().position(|r| r.identifier == identifier) {
                let ramp = &mut self.ramps[i];
                ramp.volume = volume;
                ramp.target = volume;
                ramp.start = None;
                ramp.ticks = 0;
                self.sound.manager.sources[i].volume = volume;
            }
        }
    }

    pub(crate) fn tick_with_local(
        &mut self,
        events: &[SoundEvent],
        local: &[SequenceSound],
        sim: Option<&TflSim>,
        archive: &TagArchive,
        listener: &SoundListener,
        ticks: u32,
    ) {
        self.caption(events, sim, archive, listener);
        let Some(audio) = self.link.as_ref().map(|l| l.audio.clone()) else {
            return;
        };
        self.time = self.time.wrapping_add(ticks);
        for e in events {
            self.event(e, sim, archive, listener, &audio);
        }
        self.play_local_sounds(local, archive, listener);
        if let Some(sim) = sim {
            self.sound.follow_with(|o| object_place(sim, o), &audio);
        }
        if ticks == 0 {
            return;
        }
        if let Some(sim) = sim {
            self.particles(sim, archive, listener);
        }
        for (i, ramp) in self.ramps.iter_mut().enumerate() {
            self.sound.manager.sources[i].volume = ramp.volume_at(self.time);
        }
        self.sound.update(ticks as i32, listener, &audio);
        let playing = |s: usize| self.sound.sound_is_playing(s);
        if let Some(id) = self.game_sounds.update(ticks as i32, playing)
            && let Some(tag) = self.game_sound_tag(id)
            && let Some(index) = self.sound.manager.sound_index(tag)
        {
            let channel = self
                .sound
                .play_object(index, None, None, [0; 3], listener, &audio);
            if let Some(channel) = channel {
                self.log(self.time, tag, channel);
            }
            self.game_sounds.started(index);
        }
    }

    fn game_sound_tag(&self, id: u16) -> Option<FourCC> {
        if id as i16 == crate::tfl_net::TEAM_CHAT_SOUND {
            return self.myth2_whisper;
        }
        self.game_sound_tags.get(usize::from(id)).copied().flatten()
    }

    pub(crate) fn play_local_sounds(
        &mut self,
        sounds: &[SequenceSound],
        archive: &TagArchive,
        listener: &SoundListener,
    ) {
        let Some(audio) = self.link.as_ref().map(|l| l.audio.clone()) else {
            return;
        };
        if sounds.is_empty() {
            return;
        }
        let started = self
            .sound
            .play_sequence_sounds(sounds, &[archive], listener, &audio);
        for (tick, sound, channel) in started {
            self.log(tick, sound, channel);
        }
    }

    fn particles(&mut self, sim: &TflSim, archive: &TagArchive, listener: &SoundListener) {
        if !self.particles_loaded {
            self.particles_loaded = true;
            let amso = sim
                .particle_system_tag()
                .and_then(|t| sim.data.peek_def(b"part", t))
                .and_then(|d| d.tag(PART_AMBIENT));
            if let Some(id) = amso {
                let archives = [archive];
                let mut resolver = mandrake_tag::sound_manager::archive_resolver(&archives);
                self.sound.manager.particle_definition =
                    self.sound.load_ambient_with(id, &mut resolver);
                info!(
                    "TFL particle system ambient: amso {id} ({:?})",
                    self.sound.manager.particle_definition
                );
            }
        }
        if self.sound.manager.particle_definition.is_none() {
            return;
        }
        let l = listener.location;
        let density = sim.particles.as_ref().and_then(|p| {
            p.density(
                l.x,
                l.y,
                sim.data.mesh.cell_width as i16,
                sim.data.mesh.cell_height as i16,
            )
        });
        self.sound.manager.particle_volume = density.map_or(0, |d| i16::from((d >> 8) as u8));
    }

    fn log(&mut self, tick: u32, sound: FourCC, channel: usize) {
        let m = &self.sound.manager;
        if let Some((_, volumes)) = m.channel_volumes(channel) {
            self.played.push(Played {
                tick,
                sound,
                channel,
                location: m.one_shot(channel).and_then(|c| c.location),
                volumes,
            });
        }
    }

    fn event(
        &mut self,
        e: &SoundEvent,
        sim: Option<&TflSim>,
        archive: &TagArchive,
        listener: &SoundListener,
        audio: &mandrake_audio::SharedAudio,
    ) {
        let (object, world) = match &e.kind {
            SoundKind::GameSound(id) => {
                let options = sim.map_or(0, |s| s.game_options);
                let id = match *id {
                    crate::tfl_net::TEAM_CHAT_SOUND if self.myth2_whisper.is_none() => {
                        crate::tfl_net::CHAT_SOUND
                    }
                    id => id,
                };
                self.game_sounds.queue(id as u32, options, false);
                return;
            }
            SoundKind::AmbientVolume {
                identifier,
                volume,
                ticks,
            } => {
                let now = self.time;
                if let Some(r) = self.ramps.iter_mut().find(|r| r.identifier == *identifier) {
                    r.adjust(*volume, *ticks, now);
                }
                return;
            }
            SoundKind::SoundAction { candidates, .. } if !candidates.is_empty() => {
                let Some(sim) = sim else { return };
                match nearest_seen(sim, candidates, listener, self.observer) {
                    Some(o) => (Some(o), Some(object_world(sim, o))),
                    None => return,
                }
            }
            _ => (e.object, e.world),
        };
        let Some(id) = e.sound else { return };
        let index = match self.sound.manager.sound_index(id) {
            Some(i) => Some(i),
            None => {
                let archives = [archive];
                self.sound.load_sound_with(
                    id,
                    &mut mandrake_tag::sound_manager::archive_resolver(&archives),
                )
            }
        };
        let Some(index) = index else {
            warn!("TFL sound {id} (tick {}, {:#x}): no soun", e.tick, e.site);
            return;
        };
        let channel_object = object.and_then(|o| {
            let identifier = sim?.objects.get(o)?.identifier;
            Some(ChannelObject {
                index: o as i32,
                identifier,
            })
        });
        let velocity = match (sim, object) {
            (Some(sim), Some(o)) if object != e.object => sim.objects[o].velocity,
            _ => e.velocity,
        };
        let at = world.map(|w| WorldPoint::new(w[0], w[1], w[2]));
        if let Some(channel) =
            self.sound
                .play_object(index, at, channel_object, velocity, listener, audio)
        {
            self.log(e.tick, id, channel);
        }
    }

    pub(crate) fn stop(&mut self) {
        if let Some(link) = &self.link {
            for c in level_channels(&self.sound.manager.rules) {
                link.audio.channel_stop(c);
            }
        }
        self.link = None;
    }
}

pub(crate) fn level_channels(rules: &SoundRules) -> impl Iterator<Item = usize> {
    (0..rules.one_shot_channels)
        .chain(rules.ambient_channels)
        .chain(rules.stereo_channel)
}

impl Drop for TflLevelSound {
    fn drop(&mut self) {
        self.stop();
    }
}

fn object_world(sim: &TflSim, o: usize) -> [i32; 3] {
    let ob = &sim.objects[o];
    [ob.x, ob.y, ob.z]
}

fn object_place(sim: &TflSim, o: ChannelObject) -> ObjectPlace {
    match sim.objects.get(o.index as usize) {
        Some(ob) if ob.identifier == o.identifier && ob.identifier != 0 => {
            ObjectPlace::At(WorldPoint::new(ob.x, ob.y, ob.z), ob.velocity)
        }
        _ => ObjectPlace::Deleted,
    }
}

fn nearest_seen(
    sim: &TflSim,
    candidates: &[usize],
    listener: &SoundListener,
    observer: bool,
) -> Option<usize> {
    let l = listener.location;
    let mut best: Option<(usize, u32)> = None;
    for &m in candidates {
        let Some(o) = sim.monsters.get(m).map(|m| m.object) else {
            continue;
        };
        let [x, y, z] = object_world(sim, o);
        if !observer && sim.sight().visible_bits(x >> 10, y >> 10) & 2 == 0 {
            continue;
        }
        let d = mandrake_tfl_sim::combat::distance3((x, y, z), (l.x, l.y, l.z));
        if d < SOUND_ACTION_RANGE && best.is_none_or(|(_, b)| d < b) {
            best = Some((o, d));
        }
    }
    best.map(|(o, _)| o)
}

#[cfg(test)]
mod tests {

    use mandrake_tag::sound_manager::DEFAULT_SOUND_PREFERENCE_FLAGS;

    #[test]
    fn the_default_preferences_are_the_engines() {
        let archive = std::path::Path::new("/nonexistent/myth/tags/tags.gor");
        assert_eq!(
            crate::app::sound::sound_preference_flags(archive),
            DEFAULT_SOUND_PREFERENCE_FLAGS
        );
    }
}
