pub const BANDS: [f32; 7] = [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];

pub struct Ir {
    pub rate: f32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Measures {
    pub energy: f32,
    pub early: f32,
    pub late: f32,
    pub band_energy: [f32; 7],
    pub band_decay: [f32; 7],
    pub echo: f32,
    pub echo_strength: f32,
}

fn db(e: f64) -> f32 {
    (10.0 * e.max(1e-30).log10()) as f32
}

fn band(x: &[f32], f: f32, rate: f32) -> Vec<f32> {
    let once = |x: &[f32]| bandpass(x, f, rate);
    once(&once(&once(x)))
}

fn bandpass(x: &[f32], f: f32, rate: f32) -> Vec<f32> {
    let w0 = std::f64::consts::TAU * f64::from(f) / f64::from(rate);
    let alpha = w0.sin() * ((2f64.ln() / 2.0) * w0 / w0.sin()).sinh();
    let a0 = 1.0 + alpha;
    let (b0, b2) = (alpha / a0, -alpha / a0);
    let (a1, a2) = (-2.0 * w0.cos() / a0, (1.0 - alpha) / a0);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    x.iter()
        .map(|&v| {
            let v = f64::from(v);
            let y = b0 * v + b2 * x2 - a1 * y1 - a2 * y2;
            (x2, x1, y2, y1) = (x1, v, y1, y);
            y as f32
        })
        .collect()
}

fn decay(e: &[f64], rate: f32) -> f32 {
    let total: f64 = e.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut rest = total;
    let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, v) in e.iter().enumerate() {
        let level = 10.0 * (rest / total).max(1e-30).log10();
        rest -= v;
        if level > -5.0 {
            continue;
        }
        if level < -35.0 {
            break;
        }
        let t = i as f64 / f64::from(rate);
        n += 1.0;
        sx += t;
        sy += level;
        sxx += t * t;
        sxy += t * level;
    }
    if n < 3.0 {
        return 0.0;
    }
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    (-60.0 / slope) as f32
}

fn echo(e: &[f64], rate: f32, late_start: usize) -> (f32, f32) {
    let bin = (rate / 1000.0) as usize;
    let env: Vec<f64> = e[late_start.min(e.len())..]
        .chunks(bin)
        .map(|c| c.iter().sum::<f64>().max(1e-30).log10())
        .collect();
    if env.len() < 500 {
        return (0.0, 0.0);
    }
    let w = 25usize;
    let flat: Vec<f64> = (0..env.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(w), (i + w + 1).min(env.len()));
            env[i] - env[a..b].iter().sum::<f64>() / (b - a) as f64
        })
        .collect();
    let span = flat.len().min(1200);
    let flat = &flat[..span];
    let zero: f64 = flat.iter().map(|v| v * v).sum();
    let (mut best, mut strength) = (0usize, 0.0);
    for lag in 40..=400.min(span - 1) {
        let c: f64 = flat[..span - lag]
            .iter()
            .zip(&flat[lag..])
            .map(|(a, b)| a * b)
            .sum();
        let c = c / zero.max(1e-30);
        if c > strength {
            (best, strength) = (lag, c);
        }
    }
    (best as f32 / 1000.0, strength as f32)
}

pub fn measure(ir: &Ir, late_start: f32) -> Measures {
    let e: Vec<f64> = ir
        .left
        .iter()
        .zip(&ir.right)
        .map(|(l, r)| f64::from(l * l + r * r))
        .collect();
    let split = ((late_start * ir.rate) as usize).min(e.len());
    let mut band_energy = [0.0; 7];
    let mut band_decay = [0.0; 7];
    for (i, &f) in BANDS.iter().enumerate() {
        let (l, r) = (band(&ir.left, f, ir.rate), band(&ir.right, f, ir.rate));
        let eb: Vec<f64> = l
            .iter()
            .zip(&r)
            .map(|(l, r)| f64::from(l * l + r * r))
            .collect();
        band_energy[i] = db(eb.iter().sum());
        band_decay[i] = decay(&eb, ir.rate);
    }
    let (echo, echo_strength) = echo(&e, ir.rate, split);
    Measures {
        energy: db(e.iter().sum()),
        early: db(e[..split].iter().sum()),
        late: db(e[split..].iter().sum()),
        band_energy,
        band_decay,
        echo,
        echo_strength,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reverb::{Reverb, ReverbPreset};
    use crate::reverb_presets::REVERB_PRESETS;

    fn late_start(p: &ReverbPreset) -> f32 {
        p.reflections_delay + p.late_reverb_delay
    }

    fn ours(p: ReverbPreset, rate: u32, frames: usize) -> Ir {
        let mut r = Reverb::new(p, rate);
        let (mut left, mut right) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
        for i in 0..frames {
            let (l, rr) = r.process(if i == 0 { 1.0 } else { 0.0 });
            left.push(l);
            right.push(rr);
        }
        Ir {
            rate: rate as f32,
            left,
            right,
        }
    }

    fn frames(p: &ReverbPreset) -> usize {
        ((p.decay_time * 1.6 + 0.5).clamp(1.0, 20.0) * 44100.0) as usize
    }

    const PICKABLE: std::ops::RangeInclusive<usize> = 1..=21;

    #[test]
    fn the_reverb_is_the_same_at_the_mixers_rate() {
        for i in [19, 13, 21] {
            let p = REVERB_PRESETS[i];
            let a = measure(&ours(p, 44100, frames(&p)), late_start(&p));
            let n = (frames(&p) as f32 * 48000.0 / 44100.0) as usize;
            let b = measure(&ours(p, 48000, n), late_start(&p));
            assert!((a.energy - b.energy).abs() < 0.5, "preset {i}: {a:?} {b:?}");
            for k in 2..6 {
                let d = b.band_decay[k] / a.band_decay[k] - 1.0;
                assert!(d.abs() < 0.08, "preset {i} band {k}: {d}");
            }
        }
    }

    #[test]
    fn echo_does_nothing_as_in_openal() {
        let mut p = REVERB_PRESETS[19];
        let with = ours(p, 44100, 20000);
        p.echo_depth = 0.0;
        p.echo_time = 0.1;
        let without = ours(p, 44100, 20000);
        assert_eq!(with.left, without.left);
        assert_eq!(with.right, without.right);
    }

    #[test]
    fn the_decay_hf_limit_is_air_absorptions() {
        use crate::reverb::decay_hf;
        let mut p = REVERB_PRESETS[0];
        (p.decay_time, p.decay_hf_ratio) = (6.0, 2.0);
        assert!((decay_hf(&p) - 3.52).abs() < 0.01, "{}", decay_hf(&p));
        p.decay_hf_limit = false;
        assert_eq!(decay_hf(&p), 12.0);
        (p.decay_hf_limit, p.air_absorption_gain_hf) = (true, 1.0);
        assert_eq!(decay_hf(&p), 12.0);
        for i in PICKABLE {
            let p = REVERB_PRESETS[i];
            assert_eq!(decay_hf(&p), p.decay_time * p.decay_hf_ratio, "preset {i}");
        }
    }

    #[test]
    fn the_fitted_constants() {
        use crate::reverb::fit;
        assert_eq!(
            [
                fit::SIZE_MIN,
                fit::EARLY_DECAY,
                fit::EARLY_LEVEL,
                fit::EARLY_KNEE,
                fit::LATE_LEVEL,
                fit::LOSS_GAIN,
                fit::LOSS_FREQUENCY,
                fit::DAMPING_AT,
                fit::DAMPING_SPREAD,
                fit::DAMPING_LOWEST,
                fit::DAMPING_DEPTH,
                fit::DAMPING_DEEPEST,
                fit::COMPENSATION,
            ],
            [
                0.12, 0.21, 1.51, 0.173, 0.152, 0.95, 8000.0, 0.92, 1.0, 0.3, 1.3, 0.05, 0.5
            ]
        );
    }
}
