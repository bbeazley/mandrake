//! Bungie deterministic pseudorandom number generator (PRNG).
//!
//! 32-bit LCG with multiplier `1664525` (`0x19660D`) and addend `1013904223` (`0x3C6EF35F`).

/// Bungie deterministic pseudorandom number generator.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub struct BungieRng {
    pub state: u32,
}

impl BungieRng {
    pub const MULTIPLIER: u32 = 1664525;
    pub const ADDEND: u32 = 1013904223;
    pub const DEFAULT_SEED: u32 = 0xFDED;

    #[inline]
    pub const fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    #[inline]
    pub fn step(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND);
        self.state
    }

    /// Advances the state and returns its upper 16 bits.
    #[inline]
    pub fn next_u16(&mut self) -> u16 {
        (self.step() >> 16) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determinism_and_repeatability() {
        let mut rng1 = BungieRng::new(12345);
        let mut rng2 = BungieRng::new(12345);

        for _ in 0..1000 {
            assert_eq!(rng1.next_u16(), rng2.next_u16());
            assert_eq!(rng1.state, rng2.state);
        }
    }

    #[test]
    fn test_known_sequence() {
        let mut rng = BungieRng::new(0);
        assert_eq!(rng.step(), 0x3C6E_F35F);
        assert_eq!(rng.state >> 16, 0x3C6E);

        let mut rng = BungieRng::new(BungieRng::DEFAULT_SEED);
        let seq = [rng.step(), rng.step(), rng.step(), rng.step()];
        assert_eq!(seq, [0x6DCD_4668, 0x7A10_F6A7, 0x47E1_03DA, 0xEAAF_0171]);
    }
}
