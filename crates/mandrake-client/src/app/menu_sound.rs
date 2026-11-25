use crate::audio_out::AudioLink;
use mandrake_audio::SoundBuffer;
use mandrake_common::FourCC;
use mandrake_session::SoundCue;
use mandrake_tag::sound::PermutationHistory;
use mandrake_tag::sound_manager::{FULL_VOLUME, LocalRandom, device_gain};
use mandrake_tag::tag_files::engine_tag_files;
use mandrake_tag::{AmbientSoundTag, SoundTag, TagArchive, TagLookup};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, warn};

const STEREO_CHANNEL: usize = 7;
const MS_PER_TICK: u64 = 1000 / 30;
pub(crate) const NO_MENU_MUSIC: u16 = 8;

trait SoundSource {
    fn sound(&mut self, plugins: &[String], id: FourCC, rate: u32) -> Reading;
    fn ambient(&mut self, plugins: &[String], id: FourCC) -> Option<FourCC>;
}

enum Reading {
    Sound(Loaded),
    NotThisTime,
    Failed,
}

struct Loaded {
    buffer: Arc<SoundBuffer>,
    gain: f32,
}

#[derive(Clone, Copy, Debug)]
struct Fade {
    from_ms: u64,
    length_ms: u64,
}

pub(crate) struct MenuSound {
    link: Option<AudioLink>,
    source: Box<dyn SoundSource>,
    plugins: Vec<String>,
    failed: HashSet<(Vec<String>, FourCC)>,
    playing: Option<FourCC>,
    gain: f32,
    looping: Option<Arc<SoundBuffer>>,
    fade: Option<Fade>,
}

impl MenuSound {
    pub(super) fn new(link: Option<AudioLink>, game_dir: PathBuf, seed: u32) -> Self {
        Self::with_source(
            link,
            Box::new(GameSounds {
                game_dir,
                lookup: None,
                draw: Draw::new(seed),
            }),
        )
    }

    pub(crate) fn tfl(link: Option<AudioLink>, archive: TagArchive, seed: u32) -> Self {
        Self::with_source(
            link,
            Box::new(TflSounds {
                archive,
                draw: Draw::new(seed),
            }),
        )
    }

    fn with_source(link: Option<AudioLink>, source: Box<dyn SoundSource>) -> Self {
        Self {
            link,
            source,
            plugins: Vec::new(),
            failed: HashSet::new(),
            playing: None,
            gain: 0.0,
            looping: None,
            fade: None,
        }
    }

    pub(crate) fn set_plugins(&mut self, plugins: &[String]) {
        if self.plugins != plugins {
            self.plugins = plugins.to_vec();
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.link
            .as_ref()
            .is_some_and(|link| link.audio.sounding_channel_count() > 0)
    }

    pub(crate) fn cue(&mut self, cue: SoundCue, now_ms: u64) {
        if self.link.is_none() {
            return;
        }
        debug!("menu sound: {cue:?}");
        match cue {
            SoundCue::Music(tag) | SoundCue::Narration(tag) => self.play(tag),
            SoundCue::Ambient(tag) => self.play_ambient(tag),
            SoundCue::FadeOut { ticks } => self.fade_out(ticks, now_ms),
            SoundCue::Stop => self.stop(),
        }
    }

    pub(crate) fn update(&mut self, now_ms: u64) {
        let Some(link) = &self.link else { return };
        if let Some(fade) = self.fade {
            let elapsed = now_ms.saturating_sub(fade.from_ms);
            if elapsed >= fade.length_ms {
                self.stop();
                return;
            }
            let gain = self.gain * (1.0 - elapsed as f32 / fade.length_ms as f32);
            link.audio.channel_set_gains(STEREO_CHANNEL, gain, gain);
        }
        if self.playing.is_none() {
            return;
        }
        match link.audio.channel_status(STEREO_CHANNEL) {
            mandrake_audio::EngineChannelStatus::Queued => {}
            status => match &self.looping {
                Some(buffer) => link
                    .audio
                    .channel_queue_resampled(STEREO_CHANNEL, buffer.clone()),
                None if status == mandrake_audio::EngineChannelStatus::Idle => self.playing = None,
                None => {}
            },
        }
    }

    fn fade_out(&mut self, ticks: u16, now_ms: u64) {
        if self.playing.is_none() {
            return;
        }
        let length_ms = u64::from(ticks) * MS_PER_TICK;
        let sooner = self
            .fade
            .is_none_or(|fade| fade.from_ms + fade.length_ms > now_ms + length_ms);
        if sooner {
            self.fade = Some(Fade {
                from_ms: now_ms,
                length_ms,
            });
        }
    }

    pub(crate) fn stop(&mut self) {
        self.fade = None;
        self.looping = None;
        if self.playing.take().is_some()
            && let Some(link) = &self.link
        {
            link.audio.channel_stop(STEREO_CHANNEL);
        }
    }

    pub(crate) fn end_ambient(&mut self) {
        if self.looping.is_some() {
            self.stop();
        }
    }

    fn play(&mut self, tag: FourCC) {
        if self.playing == Some(tag) && self.fade.is_none() {
            return;
        }
        let Some(sound) = self.load(tag) else {
            return;
        };
        self.start(tag, sound, false);
    }

    fn play_ambient(&mut self, tag: FourCC) {
        let continuous = self.source.ambient(&self.plugins, tag);
        let Some(sound) = continuous.and_then(|id| self.load(id)) else {
            warn!("menu sound: no ambient {}", tag.as_str());
            return;
        };
        self.start(tag, sound, true);
    }

    fn start(&mut self, tag: FourCC, sound: Loaded, looping: bool) {
        let Some(link) = &self.link else { return };
        link.audio.channel_stop(STEREO_CHANNEL);
        link.audio
            .channel_set_gains(STEREO_CHANNEL, sound.gain, sound.gain);
        link.audio
            .channel_queue_resampled(STEREO_CHANNEL, sound.buffer.clone());
        self.playing = Some(tag);
        self.gain = sound.gain;
        self.fade = None;
        self.looping = looping.then_some(sound.buffer);
    }

    fn load(&mut self, id: FourCC) -> Option<Loaded> {
        let key = (self.plugins.clone(), id);
        if self.failed.contains(&key) {
            return None;
        }
        let rate = self.link.as_ref()?.audio.device_rate();
        let started = crate::clock::Instant::now();
        let reading = self.source.sound(&self.plugins, id, rate);
        debug!(
            "menu sound: {} read in {} ms",
            id.as_str(),
            started.elapsed().as_millis()
        );
        match reading {
            Reading::Sound(sound) => Some(sound),
            Reading::NotThisTime => None,
            Reading::Failed => {
                warn!("menu sound: sound {} won't read", id.as_str());
                self.failed.insert(key);
                None
            }
        }
    }
}

struct GameSounds {
    game_dir: PathBuf,
    lookup: Option<(Vec<String>, TagLookup)>,
    draw: Draw,
}

struct Draw {
    random: LocalRandom,
    histories: HashMap<FourCC, PermutationHistory>,
}

impl Draw {
    fn new(seed: u32) -> Self {
        Self {
            random: LocalRandom::new(seed),
            histories: HashMap::new(),
        }
    }

    fn play(&mut self, id: FourCC, tag: Option<SoundTag>, rate: u32) -> Reading {
        let Some(tag) = tag else {
            return Reading::Failed;
        };
        let random = &mut self.random;
        let mut draw = |low: i16, delta: i16| {
            if delta > 0 {
                low as i32 + ((random.next_u16() as i32 * delta as i32) >> 16)
            } else {
                low as i32
            }
        };
        let pitch = draw(tag.pitch_lower_bound, tag.pitch_delta);
        let volume = draw(tag.volume_lower_bound, tag.volume_delta);
        let history = self.histories.entry(id).or_default();
        let Some(permutation) = tag.choose_permutation(history, || random.next_u16()) else {
            return Reading::NotThisTime;
        };
        let Some(buffer) = tag.decode_permutation(permutation).and_then(|pcm| {
            SoundBuffer::from_pcm16_interleaved(pcm.channels, pcm.sample_rate, pcm.samples)
        }) else {
            return Reading::Failed;
        };
        let pitch = pitch.max(1) as f32 / f32::from(FULL_VOLUME);
        Reading::Sound(Loaded {
            buffer: Arc::new(mandrake_audio::resample_buffer_with_pitch(
                &buffer, rate, pitch,
            )),
            gain: device_gain(volume as i16),
        })
    }
}

impl GameSounds {
    fn tag(&mut self, plugins: &[String], group: FourCC, id: FourCC) -> Option<Vec<u8>> {
        let matches = self
            .lookup
            .as_ref()
            .is_some_and(|(keys, _)| keys.as_slice() == plugins);
        if !matches {
            let names: Vec<&str> = plugins.iter().map(String::as_str).collect();
            self.lookup = Some((
                plugins.to_vec(),
                TagLookup::open(&engine_tag_files(&self.game_dir, &names)),
            ));
        }
        self.lookup.as_ref()?.1.get(group, id)
    }
}

impl SoundSource for GameSounds {
    fn sound(&mut self, plugins: &[String], id: FourCC, rate: u32) -> Reading {
        let tag = self
            .tag(plugins, FourCC(*b"soun"), id)
            .and_then(|data| SoundTag::from_bytes(&data).ok());
        self.draw.play(id, tag, rate)
    }

    fn ambient(&mut self, plugins: &[String], id: FourCC) -> Option<FourCC> {
        self.tag(plugins, FourCC(*b"amso"), id)
            .and_then(|data| AmbientSoundTag::from_bytes(&data).ok())
            .and_then(|tag| tag.continuous_sound())
    }
}

struct TflSounds {
    archive: TagArchive,
    draw: Draw,
}

impl SoundSource for TflSounds {
    fn sound(&mut self, _: &[String], id: FourCC, rate: u32) -> Reading {
        let tag = mandrake_session::tfl::tfl_tag(&self.archive, b"soun", id).and_then(
            |(version, data)| mandrake_tag::tfl::menus::tfl_menu_sound(id, version, &data).ok(),
        );
        self.draw.play(id, tag, rate)
    }

    fn ambient(&mut self, _: &[String], id: FourCC) -> Option<FourCC> {
        let (_, data) = mandrake_session::tfl::tfl_tag(&self.archive, b"amso", id)?;
        AmbientSoundTag::from_bytes(&data)
            .ok()
            .and_then(|tag| tag.continuous_sound())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_out::CallbackStats;
    use mandrake_audio::SharedAudio;

    const RATE: u32 = 48_000;

    struct Fake {
        frames: usize,
    }

    impl SoundSource for Fake {
        fn sound(&mut self, _: &[String], id: FourCC, _: u32) -> Reading {
            if id == FourCC(*b"none") {
                return Reading::Failed;
            }
            Reading::Sound(Loaded {
                buffer: Arc::new(SoundBuffer::from_f32_stereo(
                    vec![0.25; self.frames * 2],
                    RATE,
                )),
                gain: 1.0,
            })
        }

        fn ambient(&mut self, _: &[String], _: FourCC) -> Option<FourCC> {
            Some(FourCC(*b"loop"))
        }
    }

    fn rig(frames: usize) -> (MenuSound, SharedAudio) {
        let audio = SharedAudio::new(RATE, 1.0);
        let link = AudioLink {
            audio: audio.clone(),
            stats: Arc::new(CallbackStats::default()),
        };
        (
            MenuSound::with_source(Some(link), Box::new(Fake { frames })),
            audio,
        )
    }

    const TRACK: FourCC = FourCC(*b"mamm");

    fn mix(audio: &SharedAudio, frames: usize) -> f32 {
        let mut out = vec![0.0; frames * 2];
        audio.fill_stereo(&mut out);
        out.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn a_cue_plays_on_the_stereo_channel_until_it_ends() {
        let (mut sound, audio) = rig(RATE as usize / 10);
        assert!(!sound.busy());
        sound.cue(SoundCue::Music(TRACK), 0);
        assert!(sound.busy());
        assert_eq!(
            audio.channel_status(STEREO_CHANNEL),
            mandrake_audio::EngineChannelStatus::Playing
        );
        assert!(mix(&audio, 100) > 0.0, "it reaches the mixer");
        mix(&audio, RATE as usize);
        sound.update(100);
        assert!(!sound.busy(), "over, and another may be picked");
    }

    #[test]
    fn a_fade_runs_down_and_stops() {
        let (mut sound, audio) = rig(RATE as usize * 10);
        sound.cue(SoundCue::Narration(TRACK), 0);
        mix(&audio, 512);
        let full = mix(&audio, 512);
        sound.cue(SoundCue::FadeOut { ticks: 15 }, 1000);
        sound.update(1000 + 250);
        mix(&audio, 512);
        let half = mix(&audio, 512);
        assert!(half < full * 0.6 && half > full * 0.4, "{half} of {full}");
        assert!(sound.busy(), "it's still sounding");
        sound.update(1000 + 15 * MS_PER_TICK);
        assert!(!sound.busy());
    }

    #[test]
    fn the_sooner_fade_wins() {
        let (mut sound, _audio) = rig(RATE as usize * 10);
        sound.cue(SoundCue::Narration(TRACK), 0);
        sound.cue(SoundCue::FadeOut { ticks: 15 }, 0);
        sound.cue(SoundCue::FadeOut { ticks: 60 }, 16);
        sound.update(15 * MS_PER_TICK);
        assert!(!sound.busy());
    }

    #[test]
    fn a_fade_leaves_a_maps_sound_alone() {
        let (mut sound, audio) = rig(RATE as usize);
        let buffer = Arc::new(SoundBuffer::from_f32_stereo(vec![0.1; 48_000], RATE));
        audio.channel_queue_resampled(STEREO_CHANNEL, buffer);
        sound.cue(SoundCue::FadeOut { ticks: 60 }, 0);
        sound.update(3000);
        assert!(sound.busy(), "still playing");
        sound.stop();
        assert!(sound.busy(), "stop too");
    }

    #[test]
    fn a_missing_tag_is_asked_for_once() {
        let (mut sound, _audio) = rig(100);
        let none = FourCC(*b"none");
        sound.cue(SoundCue::Music(none), 0);
        assert!(!sound.busy());
        assert_eq!(sound.failed.len(), 1);
        sound.cue(SoundCue::Music(none), 0);
        assert!(!sound.busy());
    }

    #[test]
    fn no_device_is_never_busy() {
        let mut sound = MenuSound::new(None, PathBuf::new(), 0);
        sound.cue(SoundCue::Music(TRACK), 0);
        sound.update(1);
        assert!(!sound.busy());
    }

    #[test]
    fn a_new_sound_cuts_the_last() {
        let (mut sound, audio) = rig(RATE as usize * 10);
        sound.cue(SoundCue::Music(TRACK), 0);
        mix(&audio, 512);
        sound.cue(SoundCue::Narration(FourCC(*b"na01")), 0);
        assert_eq!(sound.playing, Some(FourCC(*b"na01")));
        assert_eq!(
            audio.channel_status(STEREO_CHANNEL),
            mandrake_audio::EngineChannelStatus::Playing,
            "not queued behind it"
        );
    }

    #[test]
    fn an_ambient_loops() {
        let (mut sound, audio) = rig(4800);
        sound.cue(SoundCue::Ambient(FourCC(*b"wisc")), 0);
        for at in 0..20 {
            sound.update(at);
            assert!(mix(&audio, 960) > 0.0, "still going at {at}");
        }
        sound.cue(SoundCue::Stop, 20);
        assert!(!sound.busy());
    }
}
