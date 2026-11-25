//! The engine's 3D sound device.
//!
//! With 3D sound enabled, the original engine hands each channel to OpenAL as a source at
//! `(-x, z, y) / 512` with its velocity, or relative at the origin for a flat play; the listener
//! sits at the observer facing its heading with `+z` up and with its velocity, using
//! `AL_NONE` distance attenuation, Doppler factor 1, and speed of sound 343.3.
//!
//! - [`pan`] interpolates a measured stereo pan law (pure gain, in phase, distance-independent).
//! - [`doppler`] applies the OpenAL 1.1 Doppler formula with the ratio capped at [`MAX_DOPPLER`].
//!
//! Positions are in engine axes (`x` east, `y` south, `z` up) in cells (world units / 512);
//! velocities are in cells per second.

use crate::pan_table::PAN;

/// OpenAL's default speed of sound (`AL_SPEED_OF_SOUND`), which the original engine leaves alone.
pub const SPEED_OF_SOUND: f32 = 343.3;
/// OpenAL Soft's ceiling on a source's pitch, where the Doppler ratio stops.
pub const MAX_DOPPLER: f32 = 10.0;
/// Each ear's gain for a mono source at the listener (straight ahead).
pub const FLAT_MONO_GAIN: f32 = PAN[0][0].0;

/// The 3D device's listener.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Listener3d {
    /// Cells, engine axes.
    pub position: [f32; 3],
    /// Radians counterclockwise from `+x`.
    pub facing: f32,
    pub velocity: [f32; 3],
}

/// Where a channel's source is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place3d {
    /// Relative to the listener, at the origin (flat play).
    Flat,
    /// At `position`, moving at `velocity` (cells, engine axes).
    At {
        position: [f32; 3],
        velocity: [f32; 3],
    },
}

/// Each ear's gain for a mono source of gain 1 at `azimuth` radians counterclockwise from straight
/// ahead (positive is to the left) and `elevation` radians above or below the horizon.
pub fn pan_angles(azimuth: f32, elevation: f32) -> (f32, f32) {
    if !azimuth.is_finite() || !elevation.is_finite() {
        return PAN[0][0];
    }
    let a = azimuth.to_degrees().rem_euclid(360.0);
    let (a, mirrored) = if a > 180.0 {
        (360.0 - a, true)
    } else {
        (a, false)
    };
    let e = elevation.to_degrees().abs().min(90.0);
    let (ai, at) = split(a / 5.0, PAN[0].len() - 1);
    let (ei, et) = split(e / 10.0, PAN.len() - 1);
    let lerp =
        |p: (f32, f32), q: (f32, f32), t: f32| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
    let row = |e: usize| lerp(PAN[e][ai], PAN[e][(ai + 1).min(PAN[0].len() - 1)], at);
    let (l, r) = lerp(row(ei), row((ei + 1).min(PAN.len() - 1)), et);
    if mirrored { (r, l) } else { (l, r) }
}

fn split(x: f32, last: usize) -> (usize, f32) {
    let i = (x.floor().max(0.0) as usize).min(last);
    (i, (x - i as f32).clamp(0.0, 1.0))
}

/// [`pan_angles`] for a source at `position` heard by `listener`. A source at the listener is
/// straight ahead.
pub fn pan(listener: &Listener3d, position: [f32; 3]) -> (f32, f32) {
    let d = sub(position, listener.position);
    let horizontal = d[0].hypot(d[1]);
    if horizontal == 0.0 && d[2] == 0.0 {
        return pan_angles(0.0, 0.0);
    }
    let azimuth = d[1].atan2(d[0]) - listener.facing;
    let elevation = d[2].atan2(horizontal);
    pan_angles(azimuth, elevation)
}

/// The Doppler pitch ratio for a source at `position` moving at `velocity`, clamped to
/// `[0.0, MAX_DOPPLER]`.
pub fn doppler(listener: &Listener3d, position: [f32; 3], velocity: [f32; 3]) -> f32 {
    let u = sub(listener.position, position);
    let length = dot(u, u).sqrt();
    if !length.is_finite() || length == 0.0 {
        return 1.0;
    }
    let vl = dot(listener.velocity, u) / length;
    let vs = dot(velocity, u) / length;
    if !vl.is_finite() || !vs.is_finite() {
        return 1.0;
    }
    let below = SPEED_OF_SOUND - vs;
    if below <= 0.0 {
        return MAX_DOPPLER;
    }
    let ratio = ((SPEED_OF_SOUND - vl).max(0.0) / below).min(MAX_DOPPLER);
    if ratio.is_finite() { ratio } else { 1.0 }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pan_is_left_for_a_source_to_the_left() {
        let listener = Listener3d {
            position: [10.0, 10.0, 4.0],
            facing: std::f32::consts::FRAC_PI_2, // facing +y
            velocity: [0.0; 3],
        };
        // Facing +y, -x is to the left.
        let (l, r) = pan(&listener, [0.0, 10.0, 4.0]);
        assert!((l - 1.0).abs() < 1e-4 && r < 1e-4, "{l} {r}");
        let (l, r) = pan(&listener, [20.0, 10.0, 4.0]);
        assert!((r - 1.0).abs() < 1e-4 && l < 1e-4, "{l} {r}");
        assert_eq!(
            pan(&listener, listener.position),
            (FLAT_MONO_GAIN, FLAT_MONO_GAIN)
        );
        // Straight below is the table's 90 degrees of elevation, the same in both ears.
        let (l, r) = pan(&listener, [10.0, 10.0, 0.0]);
        assert!((l - r).abs() < 1e-6 && (l - 0.5).abs() < 0.01, "{l} {r}");
    }

    #[test]
    fn non_finite_spatial_inputs_return_safe_defaults() {
        assert_eq!(pan_angles(f32::NAN, 0.0), PAN[0][0]);
        assert_eq!(pan_angles(0.0, f32::NAN), PAN[0][0]);
        assert_eq!(pan_angles(f32::INFINITY, 0.0), PAN[0][0]);
        let nan_listener = Listener3d {
            position: [f32::NAN, 0.0, 0.0],
            facing: f32::NAN,
            velocity: [f32::NAN, 0.0, 0.0],
        };
        assert_eq!(pan(&nan_listener, [1.0, 0.0, 0.0]), PAN[0][0]);
        assert_eq!(doppler(&nan_listener, [1.0, 0.0, 0.0], [0.0; 3]), 1.0);
        let ok_listener = Listener3d::default();
        assert_eq!(
            doppler(&ok_listener, [1.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0]),
            1.0
        );
        assert_eq!(
            doppler(&ok_listener, [f32::INFINITY, 0.0, 0.0], [10.0, 0.0, 0.0]),
            1.0
        );
    }
}
