//! Backend-agnostic audio output plumbing.
//!
//! [`SharedAudio`] wraps a [`SpatialAudioMixer`] behind a mutex so a platform audio callback
//! (native `cpal`, or a Web Audio worklet) can pull mixed stereo frames at the device sample rate
//! while the game thread drives the engine channels. Tag sample rates are resampled to the device
//! rate when a sound is queued.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::device3d::Listener3d;
use crate::mixer::{
    ChannelSpace, ENGINE_CHANNEL_COUNT, EngineChannelStatus, SoundBuffer, SpatialAudioMixer,
};
use crate::reverb::{Reverb, ReverbPreset};

fn resample_from_rate(buffer: &SoundBuffer, from_rate: u32, to_rate: u32) -> SoundBuffer {
    let channels = buffer.channels();
    let n = channels as usize;
    let frames = buffer.frame_count();
    let samples = if frames == 0 || from_rate == 0 || to_rate == 0 || from_rate == to_rate {
        match buffer {
            SoundBuffer::Pcm16 { samples, .. } => {
                samples.iter().map(|&s| s as f32 / 32768.0).collect()
            }
            SoundBuffer::F32 { samples, .. } => samples.clone(),
        }
    } else {
        let out_frames = ((frames as u64 * to_rate as u64) / from_rate as u64).max(1) as usize;
        let step = from_rate as f64 / to_rate as f64;
        let mut out = Vec::with_capacity(out_frames * n);
        match buffer {
            SoundBuffer::Pcm16 { samples, .. } => {
                for i in 0..out_frames {
                    let pos = i as f64 * step;
                    let i0 = (pos as usize).min(frames - 1);
                    let i1 = (i0 + 1).min(frames - 1);
                    let t = (pos - i0 as f64) as f32;
                    for c in 0..n {
                        let a = samples[i0 * n + c] as f32 / 32768.0;
                        let b = samples[i1 * n + c] as f32 / 32768.0;
                        out.push(a + (b - a) * t);
                    }
                }
            }
            SoundBuffer::F32 { samples, .. } => {
                for i in 0..out_frames {
                    let pos = i as f64 * step;
                    let i0 = (pos as usize).min(frames - 1);
                    let i1 = (i0 + 1).min(frames - 1);
                    let t = (pos - i0 as f64) as f32;
                    for c in 0..n {
                        let a = samples[i0 * n + c];
                        let b = samples[i1 * n + c];
                        out.push(a + (b - a) * t);
                    }
                }
            }
        }
        out
    };
    SoundBuffer::F32 {
        channels,
        sample_rate: to_rate,
        samples,
    }
}

/// Resample `buffer` to `target_rate`, returning an `F32` buffer at that rate.
pub fn resample_buffer(buffer: &SoundBuffer, target_rate: u32) -> SoundBuffer {
    resample_from_rate(buffer, buffer.sample_rate(), target_rate)
}

/// Resample `buffer` to `target_rate` while playing it at `pitch` times its native rate.
pub fn resample_buffer_with_pitch(
    buffer: &SoundBuffer,
    target_rate: u32,
    pitch: f32,
) -> SoundBuffer {
    if !pitch.is_finite() || pitch <= 0.0 || (pitch - 1.0).abs() < 1e-6 {
        return resample_buffer(buffer, target_rate);
    }
    let rate = ((buffer.sample_rate() as f64 * pitch as f64).round() as u32).max(1);
    resample_from_rate(buffer, rate, target_rate)
}

/// Thread-shared mixer running at a fixed device sample rate.
#[derive(Clone)]
pub struct SharedAudio {
    engine: Arc<Mutex<SpatialAudioMixer>>,
    device_rate: u32,
}

impl SharedAudio {
    /// Create a shared mixer whose output is consumed at `device_rate` Hz.
    /// `volume` is the master volume; pass `0.0` for muted.
    pub fn new(device_rate: u32, volume: f32) -> Self {
        let mut engine = SpatialAudioMixer::new();
        engine.master_volume = volume.max(0.0);
        Self {
            engine: Arc::new(Mutex::new(engine)),
            device_rate,
        }
    }

    fn lock(&self) -> MutexGuard<'_, SpatialAudioMixer> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Device sample rate in Hz.
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    /// Set master volume (clamped to be non-negative).
    pub fn set_master_volume(&self, volume: f32) {
        self.lock().master_volume = volume.max(0.0);
    }

    /// Current master volume.
    pub fn master_volume(&self) -> f32 {
        self.lock().master_volume
    }

    /// Status of engine hardware channel `channel` (see [`EngineChannelStatus`]).
    pub fn channel_status(&self, channel: usize) -> EngineChannelStatus {
        self.lock().engine_channel_status(channel)
    }

    /// Queue a buffer that is already at [`Self::device_rate`] on engine channel `channel`.
    pub fn channel_queue_resampled(&self, channel: usize, buffer: Arc<SoundBuffer>) {
        self.lock().engine_channel_queue(channel, buffer);
    }

    /// Queue a buffer that is already at [`Self::device_rate`] on engine channel `channel`,
    /// played at `pitch` times its native rate by the mixer.
    pub fn channel_queue_resampled_pitched(
        &self,
        channel: usize,
        buffer: Arc<SoundBuffer>,
        pitch: f32,
    ) {
        self.lock()
            .engine_channel_queue_pitched(channel, buffer, pitch);
    }

    /// Resample `buffer` (at `pitch` times its native rate) to the device rate and queue it on
    /// engine channel `channel`.
    pub fn channel_queue(&self, channel: usize, buffer: &SoundBuffer, pitch: f32) {
        let resampled = Arc::new(resample_buffer_with_pitch(buffer, self.device_rate, pitch));
        self.channel_queue_resampled(channel, resampled);
    }

    /// Set engine channel `channel`'s left/right gains (1.0 = unity).
    pub fn channel_set_gains(&self, channel: usize, left: f32, right: f32) {
        self.lock().engine_channel_set_gains(channel, left, right);
    }

    /// Put engine channel `channel` on the 3D device (see [`crate::device3d`]): the mixer pans,
    /// pitches and sends it from the 3D listener. [`Self::channel_set_gains`] takes it off.
    pub fn channel_set_space(&self, channel: usize, space: ChannelSpace) {
        self.lock().engine_channel_set_space(channel, space);
    }

    /// Engine channel `channel`'s source on the 3D device, if it is on it.
    pub fn channel_space(&self, channel: usize) -> Option<ChannelSpace> {
        self.lock().engine_channel_space(channel)
    }

    /// The preset of the reverb the device has, if any.
    pub fn reverb_preset(&self) -> Option<ReverbPreset> {
        self.lock().reverb().map(|r| *r.preset())
    }

    /// The 3D device's listener.
    pub fn set_listener_3d(&self, listener: Listener3d) {
        self.lock().set_listener_3d(listener);
    }

    /// The reverb the located channels on the 3D device feed, at the device rate; `None` for none.
    pub fn set_reverb(&self, preset: Option<&ReverbPreset>) {
        let reverb = preset.map(|p| Reverb::new(*p, self.device_rate));
        self.lock().set_reverb(reverb);
    }

    /// Stop engine channel `channel`.
    pub fn channel_stop(&self, channel: usize) {
        self.lock().engine_channel_stop(channel);
    }

    /// Stops every engine channel with its queue. A map's sounds end with the map, or a looping
    /// ambient outlives it.
    pub fn stop_all(&self) {
        self.lock().stop_all();
    }

    /// The engine channels that aren't idle.
    pub fn sounding_channel_count(&self) -> usize {
        let engine = self.lock();
        (0..ENGINE_CHANNEL_COUNT)
            .filter(|&c| engine.engine_channel_status(c) != EngineChannelStatus::Idle)
            .count()
    }

    /// Fill an interleaved stereo f32 buffer. Called from the audio callback.
    pub fn fill_stereo(&self, out: &mut [f32]) {
        self.lock().mix(out);
    }
}

impl std::fmt::Debug for SharedAudio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedAudio")
            .field("device_rate", &self.device_rate)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_doubles_length_and_interpolates() {
        let b = SoundBuffer::from_f32_mono(vec![0.0, 1.0, 0.0, -1.0], 1000);
        let r = resample_buffer(&b, 2000);
        assert_eq!(r.sample_rate(), 2000);
        assert_eq!(r.frame_count(), 8);
        assert_eq!(r.get_frame(1).0, 0.5);
    }

    #[test]
    fn resample_same_rate_is_identity_for_f32() {
        let b = SoundBuffer::from_f32_mono(vec![0.25, 0.5], 48000);
        assert_eq!(resample_buffer(&b, 48000), b);
    }

    #[test]
    fn shared_audio_plays_and_finishes() {
        let a = SharedAudio::new(48000, 1.0);
        let b = SoundBuffer::from_pcm16_mono(vec![16000; 100], 24000);
        a.channel_set_gains(0, 1.0, 1.0);
        a.channel_queue(0, &b, 1.0);
        assert_eq!(a.sounding_channel_count(), 1);
        let mut out = vec![0.0f32; 512];
        a.fill_stereo(&mut out);
        assert!(out.iter().any(|s| s.abs() > 0.01));
        assert_eq!(a.sounding_channel_count(), 0);
    }

    #[test]
    fn pitched_channel_sound_is_shorter() {
        let a = SharedAudio::new(22050, 1.0);
        let b = SoundBuffer::from_pcm16_mono(vec![1000; 2205], 22050);
        a.channel_queue(0, &b, 2.0);
        let mut out = vec![0.0f32; 2 * 1500];
        a.fill_stereo(&mut out);
        assert_eq!(a.channel_status(0), EngineChannelStatus::Idle);
    }

    #[test]
    fn pitch_shortens_resampled_buffer() {
        let b = SoundBuffer::from_pcm16_mono(vec![1000; 2205], 22050);
        assert_eq!(
            resample_buffer_with_pitch(&b, 22050, 1.0).frame_count(),
            2205
        );
        assert_eq!(
            resample_buffer_with_pitch(&b, 22050, 2.0).frame_count(),
            1102
        );
        assert_eq!(
            resample_buffer_with_pitch(&b, 44100, 0.5).frame_count(),
            8820
        );
    }

    #[test]
    fn engine_channel_queues_and_reports_status() {
        let a = SharedAudio::new(22050, 1.0);
        let b = SoundBuffer::from_pcm16_mono(vec![16384; 100], 22050);
        assert_eq!(a.channel_status(5), EngineChannelStatus::Idle);
        a.channel_set_gains(5, 1.0, 0.5);
        a.channel_queue(5, &b, 1.0);
        assert_eq!(a.channel_status(5), EngineChannelStatus::Playing);
        a.channel_queue(5, &b, 1.0);
        assert_eq!(a.channel_status(5), EngineChannelStatus::Queued);
        let mut out = vec![0.0f32; 2 * 150];
        a.fill_stereo(&mut out);
        // Gapless: the second buffer continues straight after the first.
        assert!((out[2 * 120] - 0.5).abs() < 1e-4 && (out[2 * 120 + 1] - 0.25).abs() < 1e-4);
        assert_eq!(a.channel_status(5), EngineChannelStatus::Playing);
        a.channel_stop(5);
        assert_eq!(a.channel_status(5), EngineChannelStatus::Idle);
    }

    #[test]
    fn stop_all_ends_engine_channels_with_their_queues() {
        let a = SharedAudio::new(22050, 1.0);
        let b = SoundBuffer::from_pcm16_mono(vec![16384; 100], 22050);
        a.channel_queue(3, &b, 1.0);
        a.channel_queue(0, &b, 1.0);
        a.channel_queue(0, &b, 1.0);
        a.channel_queue(7, &b, 1.0);
        assert_eq!(a.sounding_channel_count(), 3);
        a.stop_all();
        assert_eq!(a.sounding_channel_count(), 0);
        let mut out = vec![0.0f32; 64];
        a.fill_stereo(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn muted_volume_is_silent() {
        let a = SharedAudio::new(48000, 0.0);
        a.channel_set_gains(0, 1.0, 1.0);
        a.channel_queue(0, &SoundBuffer::from_f32_mono(vec![1.0; 50], 48000), 1.0);
        let mut out = vec![0.0f32; 64];
        a.fill_stereo(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn resampled_sound_reaches_callback_with_expected_peak() {
        let a = SharedAudio::new(44100, 1.0);
        let b = SoundBuffer::from_pcm16_mono(vec![20000; 2205], 22050);
        a.channel_set_gains(0, 1.0, 1.0);
        a.channel_queue(0, &b, 1.0);
        let mut peak = 0.0f32;
        let mut buf = vec![0.0f32; 1024];
        while a.sounding_channel_count() > 0 {
            a.fill_stereo(&mut buf);
            peak = buf.iter().fold(peak, |m, s| m.max(s.abs()));
        }
        assert!(peak > 0.4, "peak {peak}");
    }

    #[test]
    fn non_finite_or_non_positive_pitch_falls_back_to_unity() {
        let b = SoundBuffer::from_pcm16_mono(vec![8192; 2205], 22050);
        let unity = resample_buffer(&b, 48000);
        for pitch in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
            let res = resample_buffer_with_pitch(&b, 48000, pitch);
            assert_eq!(res, unity, "pitch {pitch:?}");
        }
    }

    #[test]
    fn mixer_steps_resampled_buffer_at_pitch_ratio() {
        let a = SharedAudio::new(22050, 1.0);
        let b = Arc::new(SoundBuffer::from_f32_mono(vec![0.5; 2000], 22050));
        a.channel_set_gains(0, 1.0, 1.0);
        a.channel_queue_resampled_pitched(0, Arc::clone(&b), 2.0);
        let mut out = vec![0.0f32; 2 * 1001];
        a.fill_stereo(&mut out);
        assert!(out[0] > 0.4);
        assert_eq!(a.channel_status(0), EngineChannelStatus::Idle);

        a.channel_queue_resampled_pitched(0, b, 0.5);
        let mut half = vec![0.0f32; 2 * 2500];
        a.fill_stereo(&mut half);
        assert_eq!(a.channel_status(0), EngineChannelStatus::Playing);
        let mut rest = vec![0.0f32; 2 * 1600];
        a.fill_stereo(&mut rest);
        assert_eq!(a.channel_status(0), EngineChannelStatus::Idle);
    }
}
