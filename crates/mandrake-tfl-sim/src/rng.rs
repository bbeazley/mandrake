use mandrake_math::BungieRng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    pub site: u32,
    pub state: u32,
}

#[derive(Debug, Clone)]
pub struct TflRng {
    rng: BungieRng,
    record: bool,
    pub log: Vec<Draw>,
}

impl TflRng {
    pub fn new(seed: u32) -> Self {
        Self::with_recording(seed, false)
    }

    pub fn new_recording(seed: u32) -> Self {
        Self::with_recording(seed, true)
    }

    pub fn with_recording(seed: u32, record: bool) -> Self {
        Self {
            rng: BungieRng::new(if seed == 0 {
                BungieRng::DEFAULT_SEED
            } else {
                seed
            }),
            record,
            log: Vec::new(),
        }
    }

    pub fn set_recording(&mut self, record: bool) {
        self.record = record;
    }

    pub fn state(&self) -> u32 {
        self.rng.state
    }

    pub fn draw(&mut self, site: u32) -> u16 {
        if self.record {
            self.log.push(Draw {
                site,
                state: self.rng.state,
            });
        }
        self.rng.next_u16()
    }

    pub fn draw_mod(&mut self, site: u32, n: i32) -> Option<i32> {
        let r = i32::from(self.draw(site));
        (n > 0).then(|| r % n)
    }

    pub fn draw_scaled(&mut self, site: u32, n: i32) -> i32 {
        (i32::from(self.draw(site)) * n) >> 16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_film_seed_gives_tfl_15s_first_states() {
        let mut r = TflRng::new(0x0012_789c);
        r.draw(0);
        assert_eq!(r.state(), 0x60a9_3b4b);
        assert_eq!(TflRng::new(0).state(), 0xfded);
    }
}
