//! Myth II's audio mixer: the engine's eight hardware channels, each a gapless queue of buffers
//! played with the gains the game's sound manager gives it (`mandrake_tag::sound_manager`), or panned,
//! pitched and sent to the reverb on the 3D device (`crate::device3d`).

use std::collections::VecDeque;
use std::sync::Arc;

use crate::device3d::{FLAT_MONO_GAIN, Listener3d, Place3d, doppler, pan};
use crate::reverb::Reverb;

/// Number of engine hardware channels (Myth II's sound manager drives eight: five one-shot
/// channels, two ambient loops and the stereo background bed).
pub const ENGINE_CHANNEL_COUNT: usize = 8;

/// One engine hardware channel: a queue of buffers played back to back with explicit per-side
/// gains, which the game-side sound manager computes (`mandrake_tag::sound_manager`), or a source on the
/// 3D device.
#[derive(Debug, Clone)]
struct QueuedBuffer {
    buffer: Arc<SoundBuffer>,
    pitch: f32,
}

#[derive(Debug, Clone, Default)]
struct EngineChannel {
    queue: VecDeque<QueuedBuffer>,
    cursor_frame: usize,
    gains: (f32, f32),
    target: (f32, f32),
    cursor_fraction: f32,
    space: Option<ChannelSpace>,
}

/// A channel's source on the 3D device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelSpace {
    /// `AL_GAIN`.
    pub gain: f32,
    pub place: Place3d,
    /// Whether the source feeds the reverb auxiliary send (located sources only).
    pub send: bool,
    /// The pan scaled so that a centred mono source is at `gain` on both sides and no side above
    /// it: The Fallen Lords' levels on the 3D device, at the level of its own unpanned DirectSound
    /// buffers. OpenAL's own centre ([`FLAT_MONO_GAIN`]) is 4.5 dB lower.
    pub unity_centre: bool,
}

impl EngineChannel {
    /// The gains and playback ratio `space` gives the front buffer for `listener`.
    fn spatial(space: &ChannelSpace, stereo: bool, listener: &Listener3d) -> ((f32, f32), f32) {
        let g = if space.gain.is_finite() {
            space.gain
        } else {
            0.0
        };
        let side = |p: f32| {
            if space.unity_centre {
                g * (p / FLAT_MONO_GAIN).min(1.0)
            } else {
                g * p
            }
        };
        match space.place {
            // OpenAL plays a multichannel buffer unpanned.
            _ if stereo => ((g, g), 1.0),
            Place3d::Flat => ((side(FLAT_MONO_GAIN), side(FLAT_MONO_GAIN)), 1.0),
            Place3d::At { position, velocity } => {
                let (l, r) = pan(listener, position);
                ((side(l), side(r)), doppler(listener, position, velocity))
            }
        }
    }

    fn frame_ahead(&self, ahead: usize) -> (f32, f32) {
        let mut index = self.cursor_frame + ahead;
        for queued in &self.queue {
            if index < queued.buffer.frame_count() {
                return queued.buffer.get_frame(index);
            }
            index -= queued.buffer.frame_count();
        }
        (0.0, 0.0)
    }

    fn advance(&mut self, frames: usize) {
        self.cursor_frame += frames;
        while let Some(front) = self.queue.front() {
            if self.cursor_frame < front.buffer.frame_count() {
                break;
            }
            self.cursor_frame -= front.buffer.frame_count();
            self.queue.pop_front();
        }
        if self.queue.is_empty() {
            self.cursor_frame = 0;
            self.cursor_fraction = 0.0;
        }
    }
}

/// Playback state of an engine channel, matching the engine backend's status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineChannelStatus {
    /// Nothing playing.
    Idle = 0,
    /// One buffer playing, nothing queued behind it.
    Playing = 1,
    /// One buffer playing and at least one queued.
    Queued = 2,
}

/// Audio channel configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channels {
    Mono = 1,
    Stereo = 2,
}

/// Sound buffer containing audio samples in PCM 16-bit or 32-bit float format.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundBuffer {
    /// 16-bit signed integer linear PCM.
    Pcm16 {
        channels: Channels,
        sample_rate: u32,
        samples: Vec<i16>,
    },
    /// 32-bit IEEE float linear PCM normalized in [-1.0, 1.0].
    F32 {
        channels: Channels,
        sample_rate: u32,
        samples: Vec<f32>,
    },
}

impl SoundBuffer {
    /// Create a mono 16-bit signed PCM buffer.
    pub fn from_pcm16_mono(samples: Vec<i16>, sample_rate: u32) -> Self {
        Self::Pcm16 {
            channels: Channels::Mono,
            sample_rate,
            samples,
        }
    }

    /// Create an interleaved stereo 16-bit signed PCM buffer.
    pub fn from_pcm16_stereo(samples: Vec<i16>, sample_rate: u32) -> Self {
        Self::Pcm16 {
            channels: Channels::Stereo,
            sample_rate,
            samples,
        }
    }

    /// Create a 16-bit PCM buffer from a channel count and interleaved samples, e.g. the
    /// `channels`/`sample_rate`/`samples` of a decoded `soun` permutation
    /// (`mandrake_tag::DecodedSound`). Returns `None` for channel counts other than 1 or 2.
    pub fn from_pcm16_interleaved(
        channels: u16,
        sample_rate: u32,
        samples: Vec<i16>,
    ) -> Option<Self> {
        let channels = match channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => return None,
        };
        Some(Self::Pcm16 {
            channels,
            sample_rate,
            samples,
        })
    }

    /// Create a mono 32-bit float buffer.
    pub fn from_f32_mono(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self::F32 {
            channels: Channels::Mono,
            sample_rate,
            samples,
        }
    }

    /// Create an interleaved stereo 32-bit float buffer.
    pub fn from_f32_stereo(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self::F32 {
            channels: Channels::Stereo,
            sample_rate,
            samples,
        }
    }

    /// Channel configuration (Mono or Stereo).
    #[inline]
    pub fn channels(&self) -> Channels {
        match self {
            Self::Pcm16 { channels, .. } | Self::F32 { channels, .. } => *channels,
        }
    }

    /// Audio sample rate in Hz.
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::Pcm16 { sample_rate, .. } | Self::F32 { sample_rate, .. } => *sample_rate,
        }
    }

    /// Total number of sample frames.
    #[inline]
    pub fn frame_count(&self) -> usize {
        match self {
            Self::Pcm16 {
                channels, samples, ..
            } => match channels {
                Channels::Mono => samples.len(),
                Channels::Stereo => samples.len() / 2,
            },
            Self::F32 {
                channels, samples, ..
            } => match channels {
                Channels::Mono => samples.len(),
                Channels::Stereo => samples.len() / 2,
            },
        }
    }

    /// Read a stereo sample frame (left, right) in `[-1.0, 1.0]` at the given frame index.
    #[inline]
    pub fn get_frame(&self, frame: usize) -> (f32, f32) {
        match self {
            Self::Pcm16 {
                channels, samples, ..
            } => match channels {
                Channels::Mono => {
                    if frame < samples.len() {
                        let s = samples[frame] as f32 / 32768.0;
                        (s, s)
                    } else {
                        (0.0, 0.0)
                    }
                }
                Channels::Stereo => {
                    let idx = frame * 2;
                    if idx + 1 < samples.len() {
                        (
                            samples[idx] as f32 / 32768.0,
                            samples[idx + 1] as f32 / 32768.0,
                        )
                    } else {
                        (0.0, 0.0)
                    }
                }
            },
            Self::F32 {
                channels, samples, ..
            } => match channels {
                Channels::Mono => {
                    if frame < samples.len() {
                        let s = samples[frame];
                        (s, s)
                    } else {
                        (0.0, 0.0)
                    }
                }
                Channels::Stereo => {
                    let idx = frame * 2;
                    if idx + 1 < samples.len() {
                        (samples[idx], samples[idx + 1])
                    } else {
                        (0.0, 0.0)
                    }
                }
            },
        }
    }
}

/// The mixer of the engine channels into the device's stereo output.
pub struct SpatialAudioMixer {
    pub master_volume: f32,
    engine_channels: [EngineChannel; ENGINE_CHANNEL_COUNT],
    /// The 3D device's listener ([`Self::engine_channel_set_space`]).
    listener3d: Listener3d,
    /// The map's reverb, fed by the located channels on the 3D device.
    reverb: Option<Reverb>,
    /// The reverb's input for a block.
    send: Vec<f32>,
}

impl Default for SpatialAudioMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl SpatialAudioMixer {
    /// A mixer with idle channels, no reverb and master volume 1.0.
    pub fn new() -> Self {
        Self {
            master_volume: 1.0,
            engine_channels: Default::default(),
            listener3d: Listener3d::default(),
            reverb: None,
            send: Vec::new(),
        }
    }

    /// The 3D device's listener.
    pub fn set_listener_3d(&mut self, listener: Listener3d) {
        self.listener3d = listener;
    }

    /// The reverb the located 3D channels feed, or none.
    pub fn set_reverb(&mut self, reverb: Option<Reverb>) {
        self.reverb = reverb;
    }

    /// The reverb, if any.
    pub fn reverb(&self) -> Option<&Reverb> {
        self.reverb.as_ref()
    }

    /// Put engine channel `channel` on the 3D device: the mixer pans it, pitches it by Doppler and
    /// sends it to the reverb itself, from the listener of each block. [`Self::engine_channel_set_gains`]
    /// takes it off again.
    pub fn engine_channel_set_space(&mut self, channel: usize, space: ChannelSpace) {
        let listener = self.listener3d;
        if let Some(ch) = self.engine_channels.get_mut(channel) {
            ch.space = Some(space);
            if ch.queue.is_empty() {
                // Mono until a buffer says otherwise; the first block corrects a stereo one.
                ch.target = EngineChannel::spatial(&space, false, &listener).0;
                ch.gains = ch.target;
            }
        }
    }

    /// The 3D device's source of engine channel `channel`, if it is on that device.
    pub fn engine_channel_space(&self, channel: usize) -> Option<ChannelSpace> {
        self.engine_channels.get(channel).and_then(|c| c.space)
    }

    /// Stop every engine channel with its queue.
    pub fn stop_all(&mut self) {
        for ch in &mut self.engine_channels {
            *ch = EngineChannel::default();
        }
        // The map's reverb stays, without the tail of what it was fed.
        if let Some(reverb) = &mut self.reverb {
            reverb.clear();
        }
    }

    /// Status of engine channel `channel` (out-of-range channels are always idle).
    pub fn engine_channel_status(&self, channel: usize) -> EngineChannelStatus {
        match self
            .engine_channels
            .get(channel)
            .map_or(0, |c| c.queue.len())
        {
            0 => EngineChannelStatus::Idle,
            1 => EngineChannelStatus::Playing,
            _ => EngineChannelStatus::Queued,
        }
    }

    /// Queue `buffer` behind whatever engine channel `channel` is playing. The buffer must
    /// already be at the output sample rate.
    pub fn engine_channel_queue(&mut self, channel: usize, buffer: Arc<SoundBuffer>) {
        self.engine_channel_queue_pitched(channel, buffer, 1.0);
    }

    /// Queue `buffer` (already at the output sample rate) to play at `pitch` times its rate.
    pub fn engine_channel_queue_pitched(
        &mut self,
        channel: usize,
        buffer: Arc<SoundBuffer>,
        pitch: f32,
    ) {
        let pitch = if !pitch.is_finite() || pitch <= 0.0 || (pitch - 1.0).abs() < 1e-6 {
            1.0
        } else {
            pitch
        };
        if let Some(ch) = self.engine_channels.get_mut(channel)
            && buffer.frame_count() > 0
        {
            if ch.queue.is_empty() {
                ch.cursor_frame = 0;
                ch.cursor_fraction = 0.0;
                if let Some(space) = ch.space {
                    let stereo = buffer.channels() == Channels::Stereo;
                    ch.target = EngineChannel::spatial(&space, stereo, &self.listener3d).0;
                }
                ch.gains = ch.target;
            }
            ch.queue.push_back(QueuedBuffer { buffer, pitch });
        }
    }

    /// Set engine channel `channel`'s left/right gains. A playing channel ramps to them over the
    /// next mixed block; an idle one takes them immediately.
    pub fn engine_channel_set_gains(&mut self, channel: usize, left: f32, right: f32) {
        if let Some(ch) = self.engine_channels.get_mut(channel) {
            ch.space = None;
            let l = if left.is_finite() { left.max(0.0) } else { 0.0 };
            let r = if right.is_finite() {
                right.max(0.0)
            } else {
                0.0
            };
            ch.target = (l, r);
            if ch.queue.is_empty() {
                ch.gains = ch.target;
            }
        }
    }

    /// Stop engine channel `channel` and drop its queue.
    pub fn engine_channel_stop(&mut self, channel: usize) {
        if let Some(ch) = self.engine_channels.get_mut(channel) {
            ch.queue.clear();
            ch.cursor_frame = 0;
            ch.cursor_fraction = 0.0;
        }
    }

    fn mix_engine_channels(&mut self, output: &mut [f32]) {
        let num_frames = output.len() / 2;
        let master = if self.master_volume.is_finite() {
            self.master_volume
        } else {
            0.0
        };
        let listener = self.listener3d;
        let reverb = self.reverb.is_some();
        let mut send_ready = false;
        for ch in &mut self.engine_channels {
            if ch.queue.is_empty() {
                ch.gains = ch.target;
                continue;
            }
            let mut spatial_rate = 1.0f32;
            let mut send = None;
            if let Some(space) = ch.space {
                let stereo = ch
                    .queue
                    .front()
                    .is_some_and(|q| q.buffer.channels() == Channels::Stereo);
                let (target, ratio) = EngineChannel::spatial(&space, stereo, &listener);
                ch.target = target;
                spatial_rate = if ratio.is_finite() {
                    ratio.max(0.0)
                } else {
                    1.0
                };
                send = (space.send && reverb && space.gain.is_finite()).then_some(space.gain);
            }
            if send.is_some() && !send_ready {
                self.send.clear();
                self.send.resize(num_frames, 0.0);
                send_ready = true;
            }
            let (l0, r0) = ch.gains;
            let (l1, r1) = ch.target;
            let step = 1.0 / num_frames as f32;
            let all_unity = spatial_rate == 1.0
                && ch.cursor_fraction == 0.0
                && ch.queue.iter().all(|q| q.pitch == 1.0);
            if !all_unity {
                for f in 0..num_frames {
                    let Some(front) = ch.queue.front() else {
                        break;
                    };
                    let rate = spatial_rate * front.pitch;
                    let rate = if rate.is_finite() { rate.max(0.0) } else { 1.0 };
                    let t = (f + 1) as f32 * step;
                    let (a, b) = (ch.frame_ahead(0), ch.frame_ahead(1));
                    let u = ch.cursor_fraction;
                    let (s_l, s_r) = (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
                    output[f * 2] += s_l * (l0 + (l1 - l0) * t) * master;
                    output[f * 2 + 1] += s_r * (r0 + (r1 - r0) * t) * master;
                    if let Some(gain) = send {
                        self.send[f] += (s_l + s_r) * 0.5 * gain;
                    }
                    let next = ch.cursor_fraction + rate;
                    let whole = next.floor();
                    ch.cursor_fraction = next - whole;
                    ch.advance(whole as usize);
                }
                ch.gains = ch.target;
                continue;
            }
            for f in 0..num_frames {
                let Some(front) = ch.queue.front() else {
                    break;
                };
                let t = (f + 1) as f32 * step;
                let (s_l, s_r) = front.buffer.get_frame(ch.cursor_frame);
                output[f * 2] += s_l * (l0 + (l1 - l0) * t) * master;
                output[f * 2 + 1] += s_r * (r0 + (r1 - r0) * t) * master;
                if let Some(gain) = send {
                    self.send[f] += (s_l + s_r) * 0.5 * gain;
                }
                ch.cursor_frame += 1;
                if ch.cursor_frame >= front.buffer.frame_count() {
                    ch.queue.pop_front();
                    ch.cursor_frame = 0;
                }
            }
            ch.gains = ch.target;
        }
        if let Some(reverb) = &mut self.reverb {
            if send_ready {
                for (f, &x) in self.send.iter().enumerate() {
                    let (l, r) = reverb.process(x);
                    output[f * 2] += l * master;
                    output[f * 2 + 1] += r * master;
                }
            } else if !reverb.is_idle() {
                for f in 0..num_frames {
                    let (l, r) = reverb.process(0.0);
                    output[f * 2] += l * master;
                    output[f * 2 + 1] += r * master;
                }
            }
        }
    }

    /// Mix the engine channels into an interleaved stereo output buffer `[L0, R0, L1, R1, ...]`,
    /// clamped to `[-1.0, 1.0]`.
    pub fn mix(&mut self, output: &mut [f32]) {
        output.fill(0.0);
        let num_frames = output.len() / 2;
        if num_frames == 0 {
            return;
        }

        self.mix_engine_channels(output);

        for sample in output.iter_mut() {
            *sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm16_interleaved_constructor() {
        let mono = SoundBuffer::from_pcm16_interleaved(1, 22050, vec![1, 2, 3]).unwrap();
        assert_eq!(mono.channels(), Channels::Mono);
        assert_eq!(mono.frame_count(), 3);
        let stereo = SoundBuffer::from_pcm16_interleaved(2, 22050, vec![16384, -16384]).unwrap();
        assert_eq!(stereo.channels(), Channels::Stereo);
        assert_eq!(stereo.frame_count(), 1);
        assert_eq!(stereo.get_frame(0), (0.5, -0.5));
        assert!(SoundBuffer::from_pcm16_interleaved(0, 22050, vec![]).is_none());
        assert!(SoundBuffer::from_pcm16_interleaved(3, 22050, vec![]).is_none());
    }

    #[test]
    fn test_engine_channel_gain_ramp_and_gapless_queue() {
        let mut mixer = SpatialAudioMixer::new();
        let buf = Arc::new(SoundBuffer::from_f32_mono(vec![1.0; 4], 48000));
        mixer.engine_channel_set_gains(7, 1.0, 1.0);
        mixer.engine_channel_queue(7, buf.clone());
        mixer.engine_channel_queue(7, buf);
        assert_eq!(mixer.engine_channel_status(7), EngineChannelStatus::Queued);

        // Lower the gain while playing: it ramps over the block instead of jumping.
        mixer.engine_channel_set_gains(7, 0.0, 0.5);
        let mut out = vec![0.0f32; 8]; // 4 frames
        mixer.mix(&mut out);
        assert!((out[0] - 0.75).abs() < 1e-6 && (out[6] - 0.0).abs() < 1e-6);
        assert!((out[1] - 0.875).abs() < 1e-6 && (out[7] - 0.5).abs() < 1e-6);
        assert_eq!(mixer.engine_channel_status(7), EngineChannelStatus::Playing);

        // The next buffer plays at the new gain.
        mixer.mix(&mut out);
        assert_eq!(out, vec![0.0, 0.5, 0.0, 0.5, 0.0, 0.5, 0.0, 0.5]);
        assert_eq!(mixer.engine_channel_status(7), EngineChannelStatus::Idle);

        // Out-of-range channels are ignored.
        mixer.engine_channel_set_gains(99, 1.0, 1.0);
        assert_eq!(mixer.engine_channel_status(99), EngineChannelStatus::Idle);
    }

    fn mono(n: usize) -> Arc<SoundBuffer> {
        Arc::new(SoundBuffer::from_f32_mono(vec![0.5; n], 48000))
    }

    fn located(position: [f32; 3], velocity: [f32; 3], send: bool) -> ChannelSpace {
        ChannelSpace {
            gain: 0.8,
            place: Place3d::At { position, velocity },
            send,
            unity_centre: false,
        }
    }

    #[test]
    fn a_3d_channel_is_panned_by_the_measured_table_and_flat_at_the_listener() {
        let mut m = SpatialAudioMixer::new();
        m.set_listener_3d(Listener3d::default());
        // Facing +x, +y is 90 degrees to the left: the near ear at the source's gain, the far
        // one silent.
        m.engine_channel_set_space(0, located([0.0, 10.0, 0.0], [0.0; 3], false));
        m.engine_channel_queue(0, mono(4096));
        let mut out = vec![0.0; 512];
        m.mix(&mut out);
        assert!(
            (out[10] - 0.4).abs() < 1e-4 && out[11].abs() < 1e-4,
            "{:?}",
            &out[10..12]
        );
        // The listener turns to face the source: both ears at the table's straight ahead.
        m.set_listener_3d(Listener3d {
            facing: std::f32::consts::FRAC_PI_2,
            ..Listener3d::default()
        });
        m.mix(&mut out);
        m.mix(&mut out);
        let ahead = 0.4 * FLAT_MONO_GAIN;
        assert!((out[10] - ahead).abs() < 1e-4 && (out[11] - ahead).abs() < 1e-4);
        // A flat play is the same at any listener.
        m.engine_channel_stop(0);
        let flat = ChannelSpace {
            gain: 1.0,
            place: Place3d::Flat,
            send: false,
            unity_centre: false,
        };
        m.engine_channel_set_space(0, flat);
        m.engine_channel_queue(0, mono(4096));
        m.mix(&mut out);
        assert!((out[10] - 0.5 * FLAT_MONO_GAIN).abs() < 1e-4);
        assert_eq!(out[10], out[11]);
        // Explicit gains take it off the device again.
        m.engine_channel_set_gains(0, 1.0, 0.25);
        assert_eq!(m.engine_channel_space(0), None);
        m.mix(&mut out);
        m.mix(&mut out);
        assert!((out[10] - 0.5).abs() < 1e-4 && (out[11] - 0.125).abs() < 1e-4);
    }

    /// A unity-centre source is at its gain when centred or flat, and never above it to a side.
    #[test]
    fn a_unity_centre_source_is_at_its_gain_ahead_and_never_above_it() {
        let mut m = SpatialAudioMixer::new();
        let ahead = ChannelSpace {
            gain: 0.8,
            place: Place3d::At {
                position: [10.0, 0.0, 0.0],
                velocity: [0.0; 3],
            },
            send: false,
            unity_centre: true,
        };
        m.engine_channel_set_space(0, ahead);
        m.engine_channel_queue(0, mono(4096));
        let mut out = vec![0.0; 512];
        m.mix(&mut out);
        assert!((out[10] - 0.4).abs() < 1e-4 && (out[11] - 0.4).abs() < 1e-4);
        // Hard left: the near side at the gain, not 1/FLAT_MONO_GAIN above it.
        m.engine_channel_set_space(
            0,
            ChannelSpace {
                place: Place3d::At {
                    position: [0.0, 10.0, 0.0],
                    velocity: [0.0; 3],
                },
                ..ahead
            },
        );
        m.mix(&mut out);
        m.mix(&mut out);
        assert!((out[10] - 0.4).abs() < 1e-4 && out[11].abs() < 1e-4);
        // Flat: the gain on both sides.
        m.engine_channel_set_space(
            0,
            ChannelSpace {
                place: Place3d::Flat,
                ..ahead
            },
        );
        m.mix(&mut out);
        m.mix(&mut out);
        assert!((out[10] - 0.4).abs() < 1e-4 && out[10] == out[11]);
    }

    #[test]
    fn a_stereo_buffer_on_the_3d_device_is_not_panned() {
        let mut m = SpatialAudioMixer::new();
        m.engine_channel_set_space(7, located([0.0, 10.0, 0.0], [0.0; 3], false));
        m.engine_channel_queue(
            7,
            Arc::new(SoundBuffer::from_f32_stereo(vec![0.5; 2048], 48000)),
        );
        let mut out = vec![0.0; 256];
        m.mix(&mut out);
        assert!((out[10] - 0.4).abs() < 1e-4 && (out[11] - 0.4).abs() < 1e-4);
    }

    #[test]
    fn doppler_plays_an_approaching_source_faster() {
        // From 30 cells ahead at half the speed of sound: twice the rate.
        let v = crate::device3d::SPEED_OF_SOUND / 2.0;
        let mut m = SpatialAudioMixer::new();
        m.engine_channel_set_space(0, located([30.0, 0.0, 0.0], [-v, 0.0, 0.0], false));
        m.engine_channel_queue(0, mono(1000));
        m.engine_channel_set_gains(1, 1.0, 1.0);
        m.engine_channel_queue(1, mono(1000));
        let mut out = vec![0.0; 2 * 501];
        m.mix(&mut out);
        assert_eq!(m.engine_channel_status(0), EngineChannelStatus::Idle);
        assert_eq!(m.engine_channel_status(1), EngineChannelStatus::Playing);
        // A source at rest plays at its rate.
        m.engine_channel_set_space(2, located([30.0, 0.0, 0.0], [0.0; 3], false));
        m.engine_channel_queue(2, mono(1000));
        m.mix(&mut out);
        assert_eq!(m.engine_channel_status(2), EngineChannelStatus::Playing);
    }

    #[test]
    fn only_a_sent_source_leaves_a_reverb_tail() {
        let preset = crate::reverb_presets::REVERB_PRESETS[19];
        for send in [false, true] {
            let mut m = SpatialAudioMixer::new();
            m.set_reverb(Some(Reverb::new(preset, 48000)));
            m.engine_channel_set_space(0, located([5.0, 0.0, 0.0], [0.0; 3], send));
            m.engine_channel_queue(0, mono(480));
            let mut out = vec![0.0; 2 * 480];
            m.mix(&mut out);
            assert_eq!(m.engine_channel_status(0), EngineChannelStatus::Idle);
            // Preset 19 reflects from 0.18 s.
            let mut tail = vec![0.0; 2 * 48000];
            m.mix(&mut tail);
            let energy: f32 = tail.iter().map(|s| s * s).sum();
            assert_eq!(energy > 1e-6, send, "send {send}: {energy}");
        }
    }

    #[test]
    fn non_finite_spatial_state_does_not_freeze_channel_or_poison_reverb() {
        let mut m = SpatialAudioMixer::new();
        m.set_reverb(Some(Reverb::new(
            crate::reverb_presets::REVERB_PRESETS[19],
            48000,
        )));
        m.set_listener_3d(Listener3d {
            position: [f32::NAN, 0.0, 0.0],
            facing: f32::NAN,
            velocity: [f32::NAN, 0.0, 0.0],
        });
        m.engine_channel_set_space(
            0,
            ChannelSpace {
                gain: f32::NAN,
                place: Place3d::At {
                    position: [f32::NAN, 1.0, 0.0],
                    velocity: [f32::NAN, 0.0, 0.0],
                },
                send: true,
                unity_centre: false,
            },
        );
        m.engine_channel_queue(0, mono(128));
        let mut out = vec![0.0; 2 * 256];
        m.mix(&mut out);
        assert_eq!(m.engine_channel_status(0), EngineChannelStatus::Idle);
        assert!(out.iter().all(|s| s.is_finite()));

        // Reverb must still work cleanly afterwards.
        m.set_listener_3d(Listener3d::default());
        m.engine_channel_set_space(0, located([5.0, 0.0, 0.0], [0.0; 3], true));
        m.engine_channel_queue(0, mono(480));
        let mut out2 = vec![0.0; 2 * 480];
        m.mix(&mut out2);
        assert!(out2.iter().all(|s| s.is_finite()));
        let mut tail = vec![0.0; 2 * 48000];
        m.mix(&mut tail);
        assert!(tail.iter().all(|s| s.is_finite()));
        let energy: f32 = tail.iter().map(|s| s * s).sum();
        assert!(energy > 1e-6, "reverb poisoned: {energy}");
    }

    #[test]
    fn stop_all_clears_reverb_tail_in_place() {
        let preset = crate::reverb_presets::REVERB_PRESETS[19];
        let mut m = SpatialAudioMixer::new();
        m.set_reverb(Some(Reverb::new(preset, 48000)));
        m.engine_channel_set_space(0, located([5.0, 0.0, 0.0], [0.0; 3], true));
        m.engine_channel_queue(0, mono(480));
        let mut out = vec![0.0; 2 * 480];
        m.mix(&mut out);
        m.stop_all();
        assert_eq!(m.reverb().map(|r| *r.preset()), Some(preset));
        assert!(m.reverb().unwrap().is_idle());
        let mut tail = vec![0.0; 2 * 48000];
        m.mix(&mut tail);
        assert!(tail.iter().all(|&s| s == 0.0));
    }
}
