use std::time::Duration;

use crate::clock::Instant;

const WARM_UP: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(crate) enum Part {
    GpuWait,
    Tick,
    Prepare,
    Encode,
    HudLayers,
    HudUpload,
    Live,
    Cursor,
    Submit,
    Present,
}
const PARTS: usize = Part::Present as usize + 1;
const PART_NAMES: [&str; PARTS] = [
    "gpu wait (last frame)",
    "cpu ticks: sim, observer, hud",
    "cpu world prepare",
    "cpu world encode",
    "cpu hud layers (software)",
    "cpu hud upload + encode",
    "cpu live overlay",
    "cpu cursor",
    "cpu submit (shell)",
    "cpu present (shell)",
];

pub(crate) struct FrameParts {
    seconds: f32,
    wait: bool,
    started: Instant,
    last_frame: Option<Instant>,
    mark: Instant,
    parts: [f32; PARTS],
    frames: Vec<[f32; PARTS]>,
    intervals: Vec<f32>,
    measuring_since: Option<Instant>,
    done: bool,
}

impl FrameParts {
    pub(crate) fn new(seconds: f32, wait: bool) -> Self {
        let now = Instant::now();
        Self {
            seconds,
            wait,
            started: now,
            last_frame: None,
            mark: now,
            parts: [0.0; PARTS],
            frames: Vec::new(),
            intervals: Vec::new(),
            measuring_since: None,
            done: false,
        }
    }

    pub(crate) fn restart(&mut self) {
        let (seconds, wait) = (self.seconds, self.wait);
        *self = Self::new(seconds, wait);
    }

    pub(crate) fn waits(&self) -> bool {
        self.wait
    }

    pub(crate) fn begin(&mut self) {
        let now = Instant::now();
        let measuring = now.duration_since(self.started) >= WARM_UP;
        if let Some(last) = self.last_frame
            && measuring
        {
            let since = *self.measuring_since.get_or_insert(last);
            self.intervals
                .push(now.duration_since(last).as_secs_f32() * 1e3);
            self.frames.push(self.parts);
            if now.duration_since(since).as_secs_f32() >= self.seconds {
                self.done = true;
            }
        }
        self.last_frame = Some(now);
        self.parts = [0.0; PARTS];
        self.mark = now;
    }

    pub(crate) fn lap(&mut self, part: Part) {
        let now = Instant::now();
        self.parts[part as usize] += now.duration_since(self.mark).as_secs_f32() * 1e3;
        self.mark = now;
    }

    pub(crate) fn add(&mut self, part: Part, ms: f32) {
        self.parts[part as usize] += ms;
    }

    pub(crate) fn done(&self) -> bool {
        self.done
    }

    pub(crate) fn print(&self, size: (u32, u32), what: &str) {
        let n = self.intervals.len();
        let secs: f32 = self.intervals.iter().sum::<f32>() / 1e3;
        println!(
            "frame-budget (TFL): {}x{} {what}: {n} frames in {secs:.1} s, {:.1} fps",
            size.0,
            size.1,
            n as f32 / secs.max(0.001)
        );
        println!(
            "{:<34} {:>8} {:>8} {:>8} {:>8}",
            "part, ms", "median", "avg", "p99", "max"
        );
        let row = |name: &str, samples: Vec<f32>| println!("{}", row(name, samples));
        row("whole frame (interval)", self.intervals.clone());
        println!(
            "frames over {} ms: {} of {n}",
            crate::app::frame_budget::BUDGET_MS,
            self.intervals
                .iter()
                .filter(|&&ms| ms > crate::app::frame_budget::BUDGET_MS)
                .count()
        );
        for (i, name) in PART_NAMES.iter().enumerate() {
            if i == Part::GpuWait as usize && !self.wait {
                continue;
            }
            row(name, self.frames.iter().map(|f| f[i]).collect());
        }
        let (first, last) = (Part::Tick as usize, Part::Cursor as usize);
        row(
            "cpu scene total",
            self.frames
                .iter()
                .map(|f| f[first..=last].iter().sum())
                .collect(),
        );
        row(
            "rest: events, pacing",
            self.frames
                .iter()
                .zip(&self.intervals)
                .map(|(f, &ms)| ms - f.iter().sum::<f32>())
                .collect(),
        );
    }
}

fn row(name: &str, mut samples: Vec<f32>) -> String {
    samples.sort_by(f32::total_cmp);
    let n = samples.len();
    let at = |q: f32| {
        if n == 0 {
            0.0
        } else {
            samples[((n as f32 * q).ceil() as usize).clamp(1, n) - 1]
        }
    };
    format!(
        "{name:<34} {:>8.3} {:>8.3} {:>8.3} {:>8.3}",
        at(0.5),
        samples.iter().sum::<f32>() / n.max(1) as f32,
        at(0.99),
        samples.last().copied().unwrap_or(0.0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_give_median_average_p99_and_max() {
        let r = row("x", (1..=100).map(|v| v as f32).collect());
        let cols: Vec<f32> = r
            .split_whitespace()
            .skip(1)
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(cols, vec![50.0, 50.5, 99.0, 100.0]);
    }
}
