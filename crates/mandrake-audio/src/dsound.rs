//! The Fallen Lords' own sound device: the volume and pan its DirectSound driver gives a
//! channel's buffer, and the per-speaker gain DirectSound plays them at.
//!
//! The driver's volume table is far louder for quiet sounds than OpenAL's curve, and it leaves a
//! centred sound at full on both speakers, where OpenAL's mono centre is 4.5 dB down. Its upper
//! entries are positive, which DirectSound refuses, so a loud volume leaves the buffer at the
//! last volume it accepted.

/// DirectSound's silence (`DSBVOLUME_MIN`).
pub const DSBVOLUME_MIN: i32 = -10000;
/// The last entry of the volume table (`0x180`).
pub const VOLUME_TABLE_LAST: i32 = 0x180;

/// The volume table's entry `index` (clamped to `0..=0x180`), in millibels: silence at 0, then
/// `5 * trunc(300 log2(index + 100) - 2400)`, then 0 from `0x11d`.
pub fn volume_table(index: i32) -> i32 {
    let i = index.clamp(0, VOLUME_TABLE_LAST);
    if i == 0 {
        return DSBVOLUME_MIN;
    }
    if i > 0x11c {
        return 0;
    }
    // The original rounds toward zero.
    let x = f64::from(i + 100);
    let v = 300.0 * x.log10() / 2f64.log10() - 2400.0;
    5 * v.trunc() as i32
}

/// Amplitude of `mb` millibels.
pub fn millibels_to_gain(mb: i32) -> f32 {
    10f32.powf(mb.clamp(DSBVOLUME_MIN, 0) as f32 / 2000.0)
}

/// One channel's buffer on the driver: the volume and pan it last took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DsoundChannel {
    /// Millibels, `<= 0`.
    pub volume: i32,
    /// Millibels, `-10000..=10000`.
    pub pan: i32,
}

impl DsoundChannel {
    /// Set the buffer's volume from an attenuated volume (8.8). DirectSound refuses a positive
    /// entry, so the buffer keeps its volume.
    pub fn set_volume(&mut self, attenuated: i32) {
        let mb = volume_table(attenuated);
        if mb <= 0 {
            self.volume = mb;
        }
    }

    /// Set the buffer's pan from a play's two side volumes `a` and `b` (8.8, each clamped to
    /// `0..=0x180`): the smaller over the larger, times `0x180`, through the table, negated when
    /// `a` is the larger. Two silent sides leave the pan as it was. The sign flips once the
    /// ratio's entry turns positive, so hard pans land on the other speaker from soft ones, as in
    /// the original.
    pub fn set_pan(&mut self, a: i32, b: i32) {
        let (a, b) = (a.clamp(0, 0x180), b.clamp(0, 0x180));
        if a == 0 && b == 0 {
            return;
        }
        self.pan = if a > b {
            -volume_table(b * 0x180 / a)
        } else {
            volume_table(a * 0x180 / b)
        };
    }

    /// The gains of the first and second speaker.
    pub fn gains(&self) -> (f32, f32) {
        let first = self.volume - self.pan.max(0);
        let second = self.volume + self.pan.min(0);
        (millibels_to_gain(first), millibels_to_gain(second))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_tfl_15s() {
        assert_eq!(volume_table(0), DSBVOLUME_MIN);
        // As the original hands them to SetVolume.
        assert_eq!(volume_table(0x100), 710);
        assert_eq!(volume_table(1), -2010);
        assert_eq!(volume_table(155), -5);
        assert_eq!(volume_table(156), 0);
        assert_eq!(volume_table(157), 5);
        assert_eq!(volume_table(0x11d), 0);
        assert_eq!(volume_table(9999), 0);
    }

    #[test]
    fn a_refused_volume_keeps_the_last_one() {
        let mut ch = DsoundChannel::default();
        assert_eq!(ch.gains(), (1.0, 1.0));
        ch.set_volume(0x100);
        assert_eq!(ch.volume, 0, "a new buffer is at full");
        ch.set_volume(64);
        let quiet = ch.volume;
        assert!(quiet < 0);
        ch.set_volume(0x100);
        assert_eq!(ch.volume, quiet);
        ch.set_volume(0);
        assert_eq!(ch.volume, DSBVOLUME_MIN);
    }

    #[test]
    fn the_pan_attenuates_one_speaker() {
        let mut ch = DsoundChannel::default();
        ch.set_pan(0x100, 0x100);
        assert_eq!(ch.gains(), (1.0, 1.0));
        // `a` at 1/8 of `b`: the ratio 48, -1185 mB, on the second speaker.
        ch.set_pan(0x20, 0x100);
        assert_eq!(ch.pan, -1185);
        let (first, second) = ch.gains();
        assert_eq!(first, 1.0);
        assert!((second - 10f32.powf(-1185.0 / 2000.0)).abs() < 1e-6);
        // Both silent: the pan stays.
        ch.set_pan(0, 0);
        assert_eq!(ch.pan, -1185);
        // The larger `a`: the entry negated, a positive pan on the first speaker.
        ch.set_pan(0x100, 0x20);
        assert_eq!(ch.pan, 1185);
        assert_eq!(ch.gains().1, 1.0);
    }
}
