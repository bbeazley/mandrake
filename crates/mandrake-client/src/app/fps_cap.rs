use crate::clock::Instant;
use std::time::Duration;

pub(crate) const FALLBACK_FPS: u32 = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FpsCap {
    Auto,
    Fixed(u32),
    Off,
}

impl std::str::FromStr for FpsCap {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text.trim() {
            "auto" => Ok(Self::Auto),
            "off" | "0" => Ok(Self::Off),
            n => match n.parse::<u32>() {
                Ok(fps) => Ok(Self::Fixed(fps)),
                Err(_) => Err(format!("{n:?}: auto, off or frames a second")),
            },
        }
    }
}

impl std::fmt::Display for FpsCap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Fixed(fps) => write!(f, "{fps}"),
            Self::Off => f.write_str("off"),
        }
    }
}

pub(crate) fn cap_wanted(flag: Option<FpsCap>, saved: Option<FpsCap>) -> (FpsCap, &'static str) {
    match (flag, saved) {
        (Some(cap), _) => (cap, "--fps-cap"),
        (None, Some(cap)) => (cap, "the settings file"),
        (None, None) => (FpsCap::Auto, "the default"),
    }
}

pub(crate) fn vsync_paces(mode: wgpu::PresentMode) -> bool {
    !matches!(
        mode,
        wgpu::PresentMode::Immediate | wgpu::PresentMode::Mailbox
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapChoice {
    Paced(u32),
    VsyncPaces,
    Uncapped,
}

pub(crate) fn choose(
    cap: FpsCap,
    refresh_mhz: Option<u32>,
    vsync: bool,
    budget: bool,
) -> CapChoice {
    let refresh = refresh_mhz.filter(|&mhz| mhz > 0);
    if budget {
        return CapChoice::Uncapped;
    }
    let fps = match cap {
        FpsCap::Off | FpsCap::Fixed(0) => return CapChoice::Uncapped,
        FpsCap::Fixed(fps) => fps,
        FpsCap::Auto if vsync => return CapChoice::VsyncPaces,
        FpsCap::Auto => refresh.map_or(FALLBACK_FPS, |mhz| {
            ((mhz as u64 + 500) / 1000).saturating_sub(1) as u32
        }),
    };
    match refresh {
        Some(mhz) if vsync && fps as u64 * 1000 >= mhz as u64 => CapChoice::VsyncPaces,
        _ => CapChoice::Paced(fps.max(1)),
    }
}

pub(crate) fn cap_line(
    cap: FpsCap,
    source: &str,
    refresh_mhz: Option<u32>,
    choice: CapChoice,
) -> String {
    let monitor = match refresh_mhz.filter(|&mhz| mhz > 0) {
        Some(mhz) => format!("monitor {:.1} Hz", mhz as f64 / 1000.0),
        None => "monitor rate unknown".to_owned(),
    };
    match choice {
        CapChoice::Paced(fps) => format!("fps cap: {cap} -> {fps} ({monitor}), {source}"),
        CapChoice::VsyncPaces => format!("fps cap: {cap}, vsync paces ({monitor}), {source}"),
        CapChoice::Uncapped if source == "--frame-budget" => {
            "fps cap: off, --frame-budget".to_owned()
        }
        CapChoice::Uncapped => format!("fps cap: off ({monitor}), {source}"),
    }
}

const EARLY_WAKE: Duration = Duration::ZERO;

#[derive(Debug, Default)]
pub(crate) struct Pacer {
    interval: Option<Duration>,
    next: Option<Instant>,
    waited: bool,
    report: Report,
}

#[derive(Debug, Default)]
struct Report {
    since: Option<Instant>,
    frames: u64,
    overshoots: Vec<f32>,
}

impl Report {
    fn counts(&self, now: Instant) -> bool {
        self.since.is_some_and(|since| now >= since)
    }
}

impl Pacer {
    pub(crate) fn set(&mut self, choice: CapChoice) {
        let interval = match choice {
            CapChoice::Paced(fps) => Some(Duration::from_secs_f64(1.0 / fps as f64)),
            CapChoice::VsyncPaces | CapChoice::Uncapped => None,
        };
        if interval != self.interval {
            self.interval = interval;
            self.next = None;
        }
    }

    pub(crate) fn poll(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let Some(interval) = self.interval else {
            return (true, None);
        };
        let due = match self.next {
            None => {
                self.next = Some(now + interval);
                true
            }
            Some(next) if now >= next => {
                if self.waited && self.report.counts(now) {
                    self.report
                        .overshoots
                        .push((now - next).as_secs_f32() * 1000.0);
                }
                let after = next + interval;
                self.next = Some(if now > after { now + interval } else { after });
                true
            }
            Some(_) => false,
        };
        self.waited = !due;
        (due, self.next)
    }

    pub(crate) fn wake_at(deadline: Instant) -> Instant {
        deadline.checked_sub(EARLY_WAKE).unwrap_or(deadline)
    }

    pub(crate) fn finish_wait(deadline: Instant) {
        if EARLY_WAKE.is_zero() {
            return;
        }
        while Instant::now() < deadline {
            std::thread::yield_now();
        }
    }

    pub(crate) fn report_from(&mut self, since: Instant) {
        self.report.since = Some(since);
    }

    pub(crate) fn report_since(&self, now: Instant) -> Option<Instant> {
        self.report.since.filter(|_| self.report.counts(now))
    }

    pub(crate) fn frame_drawn(&mut self, now: Instant) {
        if self.report.counts(now) {
            self.report.frames += 1;
        }
    }

    pub(crate) fn report_line(&self, now: Instant) -> String {
        let r = &self.report;
        let secs = self
            .report_since(now)
            .map_or(0.0, |since| (now - since).as_secs_f64());
        let fps = if secs > 0.0 {
            r.frames as f64 / secs
        } else {
            0.0
        };
        let mut late = r.overshoots.clone();
        late.sort_by(f32::total_cmp);
        let overshoot = if late.is_empty() {
            "not paced".to_owned()
        } else {
            let mean = late.iter().sum::<f32>() / late.len() as f32;
            let p99 = late[(late.len() * 99 / 100).min(late.len() - 1)];
            format!(
                "wake overshoot mean {mean:.3} ms, p99 {p99:.3} ms, max {:.3} ms",
                late[late.len() - 1]
            )
        };
        format!(
            "fps report: {} frames in {secs:.1} s = {fps:.1} fps; {overshoot}",
            r.frames
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_the_refresh_rate_less_one() {
        let auto = |mhz| choose(FpsCap::Auto, mhz, false, false);
        assert_eq!(auto(Some(60_000)), CapChoice::Paced(59));
        assert_eq!(auto(Some(120_000)), CapChoice::Paced(119));
        assert_eq!(auto(Some(144_000)), CapChoice::Paced(143));
        assert_eq!(auto(Some(165_000)), CapChoice::Paced(164));
        assert_eq!(auto(Some(240_000)), CapChoice::Paced(239));
        assert_eq!(auto(Some(59_940)), CapChoice::Paced(59));
        assert_eq!(auto(None), CapChoice::Paced(FALLBACK_FPS));
        assert_eq!(auto(Some(0)), CapChoice::Paced(FALLBACK_FPS));
    }

    #[test]
    fn fixed_off_and_frame_budget() {
        assert_eq!(
            choose(FpsCap::Fixed(90), Some(120_000), false, false),
            CapChoice::Paced(90)
        );
        assert_eq!(
            choose(FpsCap::Fixed(300), Some(120_000), false, false),
            CapChoice::Paced(300)
        );
        assert_eq!(
            choose(FpsCap::Fixed(90), None, false, false),
            CapChoice::Paced(90)
        );
        assert_eq!(
            choose(FpsCap::Off, Some(120_000), false, false),
            CapChoice::Uncapped
        );
        assert_eq!(
            choose(FpsCap::Fixed(0), None, false, false),
            CapChoice::Uncapped
        );
        for cap in [FpsCap::Auto, FpsCap::Fixed(60), FpsCap::Off] {
            assert_eq!(choose(cap, Some(120_000), false, true), CapChoice::Uncapped);
            assert_eq!(choose(cap, Some(120_000), true, true), CapChoice::Uncapped);
        }
    }

    #[test]
    fn with_vsync_only_a_cap_below_the_refresh_counts() {
        let v = |cap, mhz| choose(cap, mhz, true, false);
        assert_eq!(v(FpsCap::Fixed(30), Some(60_000)), CapChoice::Paced(30));
        assert_eq!(v(FpsCap::Fixed(60), Some(60_000)), CapChoice::VsyncPaces);
        assert_eq!(v(FpsCap::Fixed(100), Some(60_000)), CapChoice::VsyncPaces);
        assert_eq!(v(FpsCap::Fixed(59), Some(59_940)), CapChoice::Paced(59));
        assert_eq!(v(FpsCap::Auto, Some(120_000)), CapChoice::VsyncPaces);
        assert_eq!(v(FpsCap::Auto, None), CapChoice::VsyncPaces);
        assert_eq!(v(FpsCap::Off, Some(60_000)), CapChoice::Uncapped);
    }

    #[test]
    fn the_flag_wins_over_the_setting() {
        assert_eq!(cap_wanted(None, None), (FpsCap::Auto, "the default"));
        assert_eq!(
            cap_wanted(None, Some(FpsCap::Fixed(90))),
            (FpsCap::Fixed(90), "the settings file")
        );
        assert_eq!(
            cap_wanted(Some(FpsCap::Off), Some(FpsCap::Fixed(90))),
            (FpsCap::Off, "--fps-cap")
        );
    }

    #[test]
    fn parses_and_says_the_cap() {
        assert_eq!("auto".parse(), Ok(FpsCap::Auto));
        assert_eq!("off".parse(), Ok(FpsCap::Off));
        assert_eq!("0".parse(), Ok(FpsCap::Off));
        assert_eq!("144".parse(), Ok(FpsCap::Fixed(144)));
        assert!("-1".parse::<FpsCap>().is_err());
        assert!("fast".parse::<FpsCap>().is_err());
        assert_eq!(
            cap_line(
                FpsCap::Auto,
                "the default",
                Some(120_000),
                CapChoice::Paced(119)
            ),
            "fps cap: auto -> 119 (monitor 120.0 Hz), the default"
        );
        assert_eq!(
            cap_line(
                FpsCap::Auto,
                "the default",
                Some(60_000),
                CapChoice::VsyncPaces
            ),
            "fps cap: auto, vsync paces (monitor 60.0 Hz), the default"
        );
        assert_eq!(
            cap_line(FpsCap::Auto, "the default", None, CapChoice::Paced(240)),
            "fps cap: auto -> 240 (monitor rate unknown), the default"
        );
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn deadlines_are_steady_and_reset_when_late() {
        let t0 = Instant::now();
        let mut pacer = Pacer::default();
        pacer.set(CapChoice::Paced(100));
        assert_eq!(pacer.poll(t0), (true, Some(t0 + ms(10))));
        assert_eq!(pacer.poll(t0 + ms(4)), (false, Some(t0 + ms(10))));
        assert_eq!(pacer.poll(t0 + ms(11)), (true, Some(t0 + ms(20))));
        assert_eq!(pacer.poll(t0 + ms(20)), (true, Some(t0 + ms(30))));
        assert_eq!(pacer.poll(t0 + ms(45)), (true, Some(t0 + ms(55))));
        assert_eq!(pacer.poll(t0 + ms(56)), (true, Some(t0 + ms(65))));
    }

    #[test]
    fn uncapped_draws_every_pass() {
        let t0 = Instant::now();
        let mut pacer = Pacer::default();
        for choice in [CapChoice::Uncapped, CapChoice::VsyncPaces] {
            pacer.set(choice);
            assert_eq!(pacer.poll(t0), (true, None));
            assert_eq!(pacer.poll(t0), (true, None));
        }
        pacer.set(CapChoice::Paced(50));
        assert_eq!(pacer.poll(t0), (true, Some(t0 + ms(20))));
        pacer.set(CapChoice::Paced(50));
        assert_eq!(pacer.poll(t0 + ms(1)), (false, Some(t0 + ms(20))));
    }
}
