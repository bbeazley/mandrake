//! A reverb for the engine's EFX presets, fitted to OpenAL Soft's stereo response.
//!
//! With 3D sound on and a mesh with a reverb preset, the original engine loads an EAX reverb from
//! its preset table ([`crate::reverb_presets::REVERB_PRESETS`]) and sends every located source to
//! it at the source's gain; flat plays get none.
//!
//! - input shelves: `gain_hf` at `hf_reference`, `gain_lf` at `lf_reference`;
//! - early reflections from `reflections_delay`: a short four-line network, about 0.21 s at full
//!   density, at `gain * reflections_gain`;
//! - late reverb from `reflections_delay + late_reverb_delay`: two allpass diffusers set by
//!   `diffusion`, then a four-line feedback delay network whose lines `density` scales, losing
//!   60 dB in `decay_time`, with shelves for `decay_hf_ratio` (capped by `decay_hf_limit` and
//!   `air_absorption_gain_hf`) and `decay_lf_ratio`, and the small high-frequency loss per pass,
//!   at `gain * late_reverb_gain`.
//!
//! Not rendered because OpenAL Soft produces identical output or presets leave them at zero: echo,
//! `air_absorption_gain_hf` without the limit, source distance, modulation, pans, and
//! `room_rolloff_factor`.

/// `EFXEAXREVERBPROPERTIES`, in `AL_EAXREVERB_*` order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbPreset {
    pub density: f32,
    pub diffusion: f32,
    pub gain: f32,
    pub gain_hf: f32,
    pub gain_lf: f32,
    pub decay_time: f32,
    pub decay_hf_ratio: f32,
    pub decay_lf_ratio: f32,
    pub reflections_gain: f32,
    pub reflections_delay: f32,
    pub reflections_pan: [f32; 3],
    pub late_reverb_gain: f32,
    pub late_reverb_delay: f32,
    pub late_reverb_pan: [f32; 3],
    pub echo_time: f32,
    pub echo_depth: f32,
    pub modulation_time: f32,
    pub modulation_depth: f32,
    pub air_absorption_gain_hf: f32,
    pub hf_reference: f32,
    pub lf_reference: f32,
    pub room_rolloff_factor: f32,
    pub decay_hf_limit: bool,
}

/// Late lines at full size, in seconds: mutually prime at 44.1 and 48 kHz, about 30 to 45 ms.
const LINES: [f32; 4] = [0.0297, 0.0371, 0.0411, 0.0437];
/// Early lines at full size, seconds: OpenAL's reflections repeat about every 5 ms.
const EARLY_LINES: [f32; 4] = [0.00485, 0.00535, 0.00593, 0.00661];
/// The input diffusers, seconds.
const DIFFUSERS: [f32; 2] = [0.0047, 0.0113];
/// The longest pre-delay the EFX range allows: reflections 0.3 s, late 0.1 s.
const MAX_PREDELAY: f32 = 0.3 + 0.1 + 0.001;
/// Speed of sound in m/s used for air-absorption decay limiting.
const SPEED_OF_SOUND: f32 = 343.3;
/// Threshold below which decaying filter/delay states are flushed to zero to avoid subnormals.
const SUBNORMAL_FLOOR: f32 = 1e-25;

#[inline]
fn flush_subnormal(x: f32) -> f32 {
    if x.abs() < SUBNORMAL_FLOOR { 0.0 } else { x }
}

/// Fitted constants matching OpenAL Soft's EAX reverb response.
pub mod fit {
    /// The room's size against `density`: `SIZE_MIN + (1 - SIZE_MIN) * density^(1/3)`, from the
    /// late and early decays measured at density 0, 0.25, 0.5 and 1.
    pub const SIZE_MIN: f32 = 0.12;
    /// The early reflections' 60 dB decay at full size, seconds.
    pub const EARLY_DECAY: f32 = 0.21;
    /// The early energy against `(gain * reflections_gain)^2`: `EARLY_LEVEL * T / (T + EARLY_KNEE)`
    /// for a decay time `T`.
    pub const EARLY_LEVEL: f32 = 1.51;
    pub const EARLY_KNEE: f32 = 0.173;
    /// The late energy against `(gain * late_reverb_gain)^2`.
    pub const LATE_LEVEL: f32 = 0.152;
    /// The late network's own high-frequency loss on each pass, as a one-pole low-pass's gain at
    /// `LOSS_FREQUENCY` Hz (OpenAL's tail loses its highs with `decay_hf_ratio` 1 too).
    pub const LOSS_GAIN: f32 = 0.95;
    pub const LOSS_FREQUENCY: f32 = 8000.0;
    /// The decay damping is a shelf that reaches the damping `r` of each pass above
    /// `hf_reference` (below `lf_reference`), half way in dB at `hf_reference * damping_at(r)`:
    /// the deeper it cuts, the further below the reference it starts.
    pub fn damping_at(r: f32) -> f32 {
        (DAMPING_AT * r.clamp(1e-3, 1.0).powf(DAMPING_SPREAD)).max(DAMPING_LOWEST)
    }
    pub const DAMPING_AT: f32 = 0.92;
    pub const DAMPING_SPREAD: f32 = 1.0;
    pub const DAMPING_LOWEST: f32 = 0.3;
    /// The shelf's gain at its middle, whose square is its far side, `r^DAMPING_DEPTH`.
    pub fn damping_gain(r: f32) -> f32 {
        r.powf(DAMPING_DEPTH / 2.0)
    }
    pub const DAMPING_DEPTH: f32 = 1.3;
    /// The deepest damping of one pass (`decay_hf_ratio` 0.1 of a short decay asks for more).
    pub const DAMPING_DEEPEST: f32 = 0.05;
    /// How much of each band's lost energy the input gives back, as a power of the full amount.
    pub const COMPENSATION: f32 = 0.5;
}

/// A one-pole low-pass `y += (1 - a) (x - y)` whose gain at `frequency` is `gain`.
#[derive(Debug, Clone, Copy, Default)]
struct OnePole {
    a: f32,
    y: f32,
}

impl OnePole {
    fn new(gain: f32, frequency: f32, rate: f32) -> Self {
        let g = gain.clamp(1e-4, 1.0);
        if g >= 0.99999 {
            return Self::default();
        }
        let c = (std::f32::consts::TAU * (frequency / rate).min(0.49)).cos();
        // (1 - a)^2 = g^2 (1 - 2 a c + a^2), the root inside the unit circle.
        let (g2, p) = (g * g, 1.0 - g * g * c);
        let a = (p - (p * p - (1.0 - g2) * (1.0 - g2)).max(0.0).sqrt()) / (1.0 - g2);
        Self { a, y: 0.0 }
    }

    fn run(&mut self, x: f32) -> f32 {
        self.y = flush_subnormal(x + self.a * (self.y - x));
        self.y
    }

    fn clear(&mut self) {
        self.y = 0.0;
    }
}

/// A second-order shelf (the audio EQ cookbook's, slope 1): gain `gain` at `frequency`, `gain^2`
/// beyond it, 1 on the other side. OpenAL's input filters and decay damping measure this way: the
/// level at 8 kHz falls about twice as many dB as at 5 kHz, and 12 dB an octave between.
#[derive(Debug, Clone, Copy, Default)]
struct Shelf {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
    off: bool,
}

impl Shelf {
    /// Cuts above `frequency` (`high`) or below it.
    fn new(high: bool, gain: f32, frequency: f32, rate: f32) -> Self {
        let g = gain.clamp(1e-4, 1e3);
        if (g - 1.0).abs() < 1e-5 {
            return Self {
                off: true,
                ..Self::default()
            };
        }
        let w0 = std::f64::consts::TAU * f64::from((frequency / rate).min(0.45));
        let (a, cos) = (f64::from(g), w0.cos());
        let alpha2 = 2.0 * a.sqrt() * w0.sin() / 2.0 * std::f64::consts::SQRT_2;
        let s = if high { 1.0 } else { -1.0 };
        let b0 = a * ((a + 1.0) + s * (a - 1.0) * cos + alpha2);
        let b1 = -2.0 * s * a * ((a - 1.0) + s * (a + 1.0) * cos);
        let b2 = a * ((a + 1.0) + s * (a - 1.0) * cos - alpha2);
        let a0 = (a + 1.0) - s * (a - 1.0) * cos + alpha2;
        let a1 = 2.0 * s * ((a - 1.0) - s * (a + 1.0) * cos);
        let a2 = (a + 1.0) - s * (a - 1.0) * cos - alpha2;
        Self {
            b: [(b0 / a0) as f32, (b1 / a0) as f32, (b2 / a0) as f32],
            a: [(a1 / a0) as f32, (a2 / a0) as f32],
            ..Self::default()
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        if self.off {
            return x;
        }
        let y = flush_subnormal(
            self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
                - self.a[0] * self.y[0]
                - self.a[1] * self.y[1],
        );
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }

    fn clear(&mut self) {
        self.x = [0.0; 2];
        self.y = [0.0; 2];
    }
}

/// A first-order shelf: gain `gain` at `frequency`, `gain^2` beyond it, 1 on the other side (a
/// boost above 1 too). OpenAL's decay damping measures this way: it spreads over more octaves than
/// its input filter's.
#[derive(Debug, Clone, Copy, Default)]
struct Shelf1 {
    b: [f32; 2],
    a1: f32,
    x: f32,
    y: f32,
    off: bool,
}

impl Shelf1 {
    /// Shapes above `frequency` (`high`) or below it.
    fn new(high: bool, gain: f32, frequency: f32, rate: f32) -> Self {
        let r = f64::from(gain.clamp(1e-3, 1e3));
        if (r - 1.0).abs() < 1e-5 {
            return Self {
                off: true,
                ..Self::default()
            };
        }
        // Prewarped bilinear transform of (r^2 s + w) / (s + w), w = W r (high), or of
        // (s + w r^2) / (s + w), w = W / r (low): |H| = r at W.
        let warped = (std::f64::consts::PI * f64::from((frequency / rate).min(0.45))).tan();
        let (n0, n1, w) = if high {
            let w = warped * r;
            (r * r + w, w - r * r, w)
        } else {
            let w = warped / r;
            (1.0 + w * r * r, w * r * r - 1.0, w)
        };
        let d0 = 1.0 + w;
        Self {
            b: [(n0 / d0) as f32, (n1 / d0) as f32],
            a1: ((w - 1.0) / d0) as f32,
            ..Self::default()
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        if self.off {
            return x;
        }
        let y = flush_subnormal(self.b[0] * x + self.b[1] * self.x - self.a1 * self.y);
        self.x = x;
        self.y = y;
        y
    }

    fn clear(&mut self) {
        self.x = 0.0;
        self.y = 0.0;
    }
}

/// A delay line of a fixed length.
#[derive(Debug, Clone)]
struct Delay {
    buffer: Vec<f32>,
    at: usize,
}

impl Delay {
    fn new(samples: usize) -> Self {
        Self {
            buffer: vec![0.0; samples.max(1)],
            at: 0,
        }
    }

    /// The sample written `delay` samples ago (`delay` < the line's length).
    fn tap(&self, delay: usize) -> f32 {
        let n = self.buffer.len();
        self.buffer[(self.at + n - delay.min(n - 1)) % n]
    }

    /// The oldest sample.
    fn oldest(&self) -> f32 {
        self.buffer[self.at]
    }

    fn push(&mut self, x: f32) -> f32 {
        let out = self.buffer[self.at];
        self.buffer[self.at] = x;
        self.at = (self.at + 1) % self.buffer.len();
        out
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.at = 0;
    }
}

/// A four-line Householder feedback network: each line `x - (2 / 4) sum x`, through its own
/// filters, back in with the input.
#[derive(Debug, Clone)]
struct Network {
    lines: [Delay; 4],
    feedback: [f32; 4],
    damping_hf: [Shelf; 4],
    damping_lf: [Shelf; 4],
    loss: [OnePole; 4],
}

impl Network {
    /// Lines of `lengths` samples losing 60 dB in `decay`, `decay_hf` above `hf` Hz and
    /// `decay_lf` below `lf` Hz, with `loss` (gain, Hz) of high frequencies on each pass.
    #[allow(clippy::too_many_arguments)]
    fn new(
        lengths: [usize; 4],
        rate: f32,
        decay: f32,
        (decay_hf, hf): (f32, f32),
        (decay_lf, lf): (f32, f32),
        loss: Option<(f32, f32)>,
    ) -> Self {
        let pass = |n: usize, t: f32| 10f32.powf(-3.0 * n as f32 / (rate * t));
        let feedback = lengths.map(|n| pass(n, decay));
        Self {
            lines: lengths.map(Delay::new),
            feedback,
            damping_hf: std::array::from_fn(|i| {
                let r = if feedback[i] > 0.0 {
                    pass(lengths[i], decay_hf) / feedback[i]
                } else {
                    1.0
                };
                let r = r.clamp(fit::DAMPING_DEEPEST, stable(feedback[i]));
                Shelf::new(true, fit::damping_gain(r), hf * fit::damping_at(r), rate)
            }),
            damping_lf: std::array::from_fn(|i| {
                let r = if feedback[i] > 0.0 {
                    pass(lengths[i], decay_lf) / feedback[i]
                } else {
                    1.0
                };
                let r = r.clamp(fit::DAMPING_DEEPEST, stable(feedback[i]));
                Shelf::new(false, fit::damping_gain(r), lf / fit::damping_at(r), rate)
            }),
            loss: [loss.map_or_else(OnePole::default, |(g, f)| OnePole::new(g, f, rate)); 4],
        }
    }

    fn run(&mut self, input: f32) -> [f32; 4] {
        let out: [f32; 4] = std::array::from_fn(|i| self.lines[i].oldest());
        let half = (out[0] + out[1] + out[2] + out[3]) * 0.5;
        for (i, o) in out.iter().enumerate() {
            let mut v = (o - half) * self.feedback[i];
            v = self.damping_hf[i].run(v);
            v = self.damping_lf[i].run(v);
            v = self.loss[i].run(v);
            self.lines[i].push(flush_subnormal(input + v));
        }
        out
    }

    fn clear(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        for f in &mut self.damping_hf {
            f.clear();
        }
        for f in &mut self.damping_lf {
            f.clear();
        }
        for f in &mut self.loss {
            f.clear();
        }
    }

    /// The energy of the response to an impulse, both outputs as [`Reverb::process`] mixes them,
    /// over `frames` samples.
    fn impulse_energy(&self, frames: usize) -> f32 {
        let mut n = self.clone();
        let mut e = 0.0f64;
        for i in 0..frames {
            let o = n.run(if i == 0 { 1.0 } else { 0.0 });
            let (l, r) = mix(o);
            e += f64::from(l * l + r * r);
        }
        e as f32
    }
}

/// The largest shelf gain whose square keeps a line of feedback `g` below 1.
fn stable(g: f32) -> f32 {
    if g > 0.0 { (0.999 / g).sqrt() } else { 1.0 }
}

/// The input shelf that gives each band the energy of the middle (OpenAL keeps a band's level
/// when its decay changes): `sqrt((1 - (g r)^2) / (1 - g^2))` for the feedback `g` and the
/// damping `r`.
pub(crate) fn compensation(g: f32, r: f32) -> f32 {
    let r = r.min(stable(g));
    ((1.0 - (g * r).powi(2)).max(1e-6) / (1.0 - g * g).max(1e-6)).powf(0.5 * fit::COMPENSATION)
}

/// Two outputs from four lines.
fn mix(o: [f32; 4]) -> (f32, f32) {
    (o[0] + o[2] - o[1] * 0.5, o[1] + o[3] - o[2] * 0.5)
}

/// The reverb for one preset at one output rate.
#[derive(Debug, Clone)]
pub struct Reverb {
    preset: ReverbPreset,
    input_hf: Shelf,
    input_lf: Shelf,
    predelay: Delay,
    early_start: usize,
    early: Network,
    early_gain: f32,
    late_start: usize,
    diffusers: [Delay; 2],
    diffusion: f32,
    late: Network,
    late_hf: Shelf1,
    late_lf: Shelf1,
    late_gain: f32,
    idle: bool,
    silent_samples: usize,
    silent_threshold: usize,
}

/// The decay at high frequencies, with `decay_hf_limit`: no longer than air absorption at
/// `air_absorption_gain_hf` per metre allows.
pub fn decay_hf(preset: &ReverbPreset) -> f32 {
    let decay = preset.decay_time.clamp(0.1, 20.0);
    let mut ratio = preset.decay_hf_ratio.clamp(0.1, 2.0);
    let air = preset.air_absorption_gain_hf.clamp(0.892, 1.0);
    if preset.decay_hf_limit && air < 1.0 {
        // 60 dB of air at the speed of sound.
        let air_decay = 60.0 / (-20.0 * air.log10() * SPEED_OF_SOUND);
        ratio = ratio.min(air_decay / decay);
    }
    decay * ratio
}

impl Reverb {
    /// `preset` at `rate` Hz.
    pub fn new(preset: ReverbPreset, rate: u32) -> Self {
        let rate = rate.max(1) as f32;
        let samples = |s: f32| (s * rate).round().max(1.0) as usize;
        let reflections = preset.reflections_delay.clamp(0.0, 0.3);
        let late = reflections + preset.late_reverb_delay.clamp(0.0, 0.1);
        let size = fit::SIZE_MIN + (1.0 - fit::SIZE_MIN) * preset.density.clamp(0.0, 1.0).cbrt();
        let decay = preset.decay_time.clamp(0.1, 20.0);
        let hf = preset.hf_reference.clamp(1000.0, 20000.0);
        let lf = preset.lf_reference.clamp(20.0, 1000.0);
        let decay_lf = decay * preset.decay_lf_ratio.clamp(0.1, 2.0);

        let early = Network::new(
            EARLY_LINES.map(|l| samples(l * size)),
            rate,
            fit::EARLY_DECAY * size,
            (fit::EARLY_DECAY * size, hf),
            (fit::EARLY_DECAY * size, lf),
            None,
        );
        let early_energy = fit::EARLY_LEVEL * decay / (decay + fit::EARLY_KNEE);
        let energy = early.impulse_energy(samples(fit::EARLY_DECAY * 1.5));
        let early_norm = if energy > 0.0 {
            (early_energy / energy).max(0.0).sqrt()
        } else {
            0.0
        };

        let lengths = LINES.map(|l| samples(l * size));
        let late_net = Network::new(
            lengths,
            rate,
            decay,
            (decay_hf(&preset), hf),
            (decay_lf, lf),
            Some((fit::LOSS_GAIN, fit::LOSS_FREQUENCY)),
        );
        // The network's level grows with its decay: scale the input so every decay holds the
        // same energy, as OpenAL's does.
        let feedback = late_net.feedback;
        let mean = lengths.iter().sum::<usize>() as f32 / 4.0;
        let pass = |t: f32| 10f32.powf(-3.0 * mean / (rate * t));
        let g = pass(decay);
        let (c_hf, c_lf) = if g > 0.0 {
            (
                compensation(g, pass(decay_hf(&preset)) / g),
                compensation(g, pass(decay_lf) / g),
            )
        } else {
            (1.0, 1.0)
        };
        let late_hf = Shelf1::new(true, c_hf, hf, rate);
        let late_lf = Shelf1::new(false, c_lf, lf, rate);
        // OpenAL's middle rises as much when either end is damped.
        let late_norm = (1.0 - feedback.iter().map(|g| g * g).sum::<f32>() / 4.0)
            .max(1e-5)
            .sqrt()
            * c_hf
            * c_lf;
        let predelay = Delay::new(samples(MAX_PREDELAY));
        let diffusers = DIFFUSERS.map(|d| Delay::new(samples(d)));
        let silent_threshold = predelay.buffer.len()
            + diffusers.iter().map(|d| d.buffer.len()).sum::<usize>()
            + late_net
                .lines
                .iter()
                .map(|d| d.buffer.len())
                .max()
                .unwrap_or(0)
            + early
                .lines
                .iter()
                .map(|d| d.buffer.len())
                .max()
                .unwrap_or(0)
            + 16;
        Self {
            preset,
            input_hf: Shelf::new(true, preset.gain_hf, hf, rate),
            input_lf: Shelf::new(false, preset.gain_lf, lf, rate),
            predelay,
            early_start: samples(reflections),
            early,
            early_gain: preset.gain * preset.reflections_gain.clamp(0.0, 3.16) * early_norm,
            late_start: samples(late),
            diffusers,
            diffusion: 0.6 * preset.diffusion.clamp(0.0, 1.0),
            late: late_net,
            late_hf,
            late_lf,
            late_gain: preset.gain
                * preset.late_reverb_gain.clamp(0.0, 10.0)
                * late_norm
                * fit::LATE_LEVEL.sqrt(),
            idle: true,
            silent_samples: 0,
            silent_threshold,
        }
    }

    /// Clear all delay lines and filter states in place without reallocating.
    pub fn clear(&mut self) {
        self.input_hf.clear();
        self.input_lf.clear();
        self.predelay.clear();
        self.early.clear();
        for d in &mut self.diffusers {
            d.clear();
        }
        self.late.clear();
        self.late_hf.clear();
        self.late_lf.clear();
        self.idle = true;
        self.silent_samples = 0;
    }

    /// Whether the reverb has no active tail and can be skipped while input is silent.
    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// The same reverb with its tail cleared.
    pub fn cleared(&self) -> Self {
        let mut copy = self.clone();
        copy.clear();
        copy
    }

    /// The preset this reverb renders.
    pub fn preset(&self) -> &ReverbPreset {
        &self.preset
    }

    /// One sample of the wet signal for the mono send `x`.
    pub fn process(&mut self, x: f32) -> (f32, f32) {
        let x = if x.is_finite() {
            flush_subnormal(x)
        } else {
            0.0
        };
        if self.idle {
            if x == 0.0 {
                return (0.0, 0.0);
            }
            self.idle = false;
            self.silent_samples = 0;
        }
        let filtered = self.input_lf.run(self.input_hf.run(x));
        self.predelay.push(filtered);
        let (mut left, mut right) = mix(self.early.run(self.predelay.tap(self.early_start)));
        left *= self.early_gain;
        right *= self.early_gain;
        let mut late = self.predelay.tap(self.late_start);
        for d in &mut self.diffusers {
            // Schroeder allpass.
            let delayed = d.oldest();
            let v = flush_subnormal(late + self.diffusion * delayed);
            d.push(v);
            late = flush_subnormal(delayed - self.diffusion * v);
        }
        let late = self.late_lf.run(self.late_hf.run(late));
        let (l, r) = mix(self.late.run(late));
        left = flush_subnormal(left + l * self.late_gain);
        right = flush_subnormal(right + r * self.late_gain);
        if x == 0.0 && left == 0.0 && right == 0.0 {
            self.silent_samples += 1;
            if self.silent_samples >= self.silent_threshold {
                self.clear();
            }
        } else {
            self.silent_samples = 0;
        }
        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reverb_presets::REVERB_PRESETS;

    /// The energy of the response to an impulse in each 50 ms after `skip`.
    fn tail(preset: ReverbPreset, rate: u32, seconds: f32) -> Vec<f32> {
        let mut r = Reverb::new(preset, rate);
        let block = (rate as f32 * 0.05) as usize;
        let n = (rate as f32 * seconds) as usize;
        let mut energy = vec![0.0; n / block];
        for i in 0..n {
            let (l, rr) = r.process(if i == 0 { 1.0 } else { 0.0 });
            if i / block < energy.len() {
                energy[i / block] += l * l + rr * rr;
            }
        }
        energy
    }

    #[test]
    fn the_tail_falls_60_db_in_about_the_decay_time() {
        for (i, preset) in REVERB_PRESETS.iter().enumerate() {
            let decay = preset.decay_time.clamp(0.1, 20.0);
            if !(0.5..=5.0).contains(&decay) {
                continue;
            }
            let start = preset.reflections_delay + preset.late_reverb_delay;
            let e = tail(*preset, 48000, start + decay + 0.5);
            // From the block 0.2 s into the late reverb to the block a decay later: about 60 dB.
            let a = ((start + 0.2) / 0.05) as usize;
            let b = a + (decay / 0.05) as usize;
            let db = 10.0 * (e[a] / e[b].max(1e-30)).log10();
            assert!(
                (52.0..=70.0).contains(&db),
                "preset {i}: {db} dB in {decay} s"
            );
        }
    }

    #[test]
    fn it_is_silent_without_input_and_stable() {
        for preset in REVERB_PRESETS {
            let mut r = Reverb::new(preset, 44100);
            assert_eq!(r.process(0.0), (0.0, 0.0));
            let mut peak = 0.0f32;
            for i in 0..44100 * 2 {
                let x = if i % 4410 == 0 { 1.0 } else { 0.0 };
                let (l, rr) = r.process(x);
                peak = peak.max(l.abs()).max(rr.abs());
            }
            assert!(peak.is_finite() && peak < 4.0, "{peak}");
        }
    }

    #[test]
    fn the_table_is_the_engines_26_presets() {
        assert_eq!(REVERB_PRESETS.len(), 26);
        // 0 is EFX's generic preset.
        let g = REVERB_PRESETS[0];
        assert_eq!(
            (g.density, g.gain, g.decay_time, g.decay_hf_ratio),
            (1.0, 0.3162, 1.49, 0.83)
        );
    }

    #[test]
    fn very_low_sample_rates_produce_finite_output() {
        for rate in 0..=8 {
            for preset in REVERB_PRESETS {
                let mut r = Reverb::new(preset, rate);
                for i in 0..32 {
                    let (l, rr) = r.process(if i == 0 { 1.0 } else { 0.0 });
                    assert!(l.is_finite() && rr.is_finite(), "rate {rate}: ({l}, {rr})");
                }
            }
        }
    }

    #[test]
    fn clear_matches_fresh_reverb_bit_for_bit() {
        let preset = REVERB_PRESETS[19];
        let mut fresh = Reverb::new(preset, 48000);
        let mut reused = Reverb::new(preset, 48000);
        for i in 0..4096 {
            reused.process(if i % 100 == 0 { 0.7 } else { 0.0 });
        }
        reused.clear();
        assert!(reused.is_idle());
        for i in 0..2048 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            assert_eq!(reused.process(x), fresh.process(x), "sample {i}");
        }
    }

    #[test]
    fn tail_decays_to_idle_without_subnormals() {
        let mut preset = REVERB_PRESETS[0];
        preset.decay_time = 0.1;
        let mut r = Reverb::new(preset, 8000);
        assert!(r.is_idle());
        let _ = r.process(1.0);
        assert!(!r.is_idle());
        for _ in 0..8000 * 6 {
            let (l, rr) = r.process(0.0);
            assert!(l == 0.0 || l.abs() >= SUBNORMAL_FLOOR, "subnormal l: {l:e}");
            assert!(
                rr == 0.0 || rr.abs() >= SUBNORMAL_FLOOR,
                "subnormal r: {rr:e}"
            );
        }
        assert!(r.is_idle());
        assert_eq!(r.process(0.0), (0.0, 0.0));
    }
}
