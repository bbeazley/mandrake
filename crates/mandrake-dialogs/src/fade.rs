#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickFade {
    pub count: i32,
}

impl TickFade {
    pub const fn new(count: i32) -> Self {
        Self { count }
    }

    pub const fn steps(self) -> i32 {
        self.count + 1
    }

    pub const fn value(self, k: i32) -> u32 {
        let u = (k << 8) / self.count;
        if u > 0xff { 0xff } else { u as u32 }
    }

    pub fn from_black_alpha(self, k: i32) -> f32 {
        1.0 - self.value(k) as f32 / 255.0
    }

    pub fn to_black_alpha(self, k: i32) -> f32 {
        self.value(k) as f32 / 255.0
    }
}

pub const BRIEFING_FADE_IN: TickFade = TickFade::new(6);
pub const BRIEFING_CROSSFADE: TickFade = TickFade::new(6);
pub const BRIEFING_FADE_OUT: TickFade = TickFade::new(8);
pub const EPILOGUE_FADE_IN: TickFade = TickFade::new(8);
pub const EPILOGUE_FADE_OUT: TickFade = TickFade::new(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelEndFade {
    ToBlack { live_ticks: i32, film_ms: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameFades {
    pub briefing_fade_in: TickFade,
    pub briefing_crossfade: TickFade,
    pub load_fade_out_ms: u32,
    pub load_fade_in_ms: u32,
    pub level_end: LevelEndFade,
    pub epilogue: Option<(TickFade, TickFade)>,
}

pub const MYTH_II: GameFades = GameFades {
    briefing_fade_in: BRIEFING_FADE_IN,
    briefing_crossfade: BRIEFING_CROSSFADE,
    load_fade_out_ms: 2000,
    load_fade_in_ms: 500,
    level_end: LevelEndFade::ToBlack {
        live_ticks: 60,
        film_ms: 2000,
    },
    epilogue: None,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickFadeRun {
    pub fade: TickFade,
    pub step: i32,
    credit: i32,
}

impl TickFadeRun {
    pub const fn new(fade: TickFade) -> Self {
        Self {
            fade,
            step: 0,
            credit: 1,
        }
    }

    pub fn advance(&mut self, ticks: i32, mut on_step: impl FnMut(i32, u32)) {
        self.credit += ticks;
        while self.credit > 0 && !self.done() {
            self.credit -= 1;
            on_step(self.step, self.fade.value(self.step));
            self.step += 1;
        }
    }

    pub const fn done(&self) -> bool {
        self.step >= self.fade.steps()
    }

    pub const fn last_step(&self) -> Option<i32> {
        if self.step > 0 {
            Some(self.step - 1)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_fade_is_its_count_in_ticks() {
        for (fade, ticks) in [
            (BRIEFING_FADE_IN, 6),
            (BRIEFING_CROSSFADE, 6),
            (BRIEFING_FADE_OUT, 8),
            (EPILOGUE_FADE_IN, 8),
            (EPILOGUE_FADE_OUT, 8),
        ] {
            let mut run = TickFadeRun::new(fade);
            let mut at = Vec::new();
            for tick in 0..20 {
                run.advance(i32::from(tick != 0), |k, _| at.push((tick, k)));
            }
            assert_eq!(at.len() as i32, ticks + 1);
            assert_eq!(at.first(), Some(&(0, 0)));
            assert_eq!(at.last(), Some(&(ticks, ticks)));
            assert!(run.done());
        }
        assert_eq!(BRIEFING_FADE_IN.value(1), 42);
        assert_eq!(EPILOGUE_FADE_IN.from_black_alpha(8), 0.0);
    }

    #[test]
    fn a_long_frame_runs_the_steps_it_owes() {
        let mut run = TickFadeRun::new(EPILOGUE_FADE_OUT);
        let mut seen = Vec::new();
        run.advance(3, |k, u| seen.push((k, u)));
        assert_eq!(seen, [(0, 0), (1, 32), (2, 64), (3, 96)]);
        assert_eq!(run.last_step(), Some(3));
        run.advance(100, |_, _| {});
        assert!(run.done());
    }
}
