use crate::draw_site;
use crate::engine_rng::{EngineRng, Unmodelled};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParticleStateDefinition {
    pub duration: i16,
    pub duration_range: i16,
    pub intensity: u16,
    pub intensity_range: u16,
    pub transition: i16,
    pub transition_range: i16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParticleSystemDefinition {
    pub flags: u32,
    pub scale_x: [i16; 2],
    pub scale_y: [i16; 2],
    pub states: [ParticleStateDefinition; 2],
}

fn be_u16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([d[o], d[o + 1]])
}

fn be_i16(d: &[u8], o: usize) -> i16 {
    be_u16(d, o) as i16
}

impl ParticleSystemDefinition {
    pub fn parse(d: &[u8]) -> Option<Self> {
        if d.len() < 0x66 {
            return None;
        }
        let state = |o: usize| ParticleStateDefinition {
            duration: be_i16(d, o),
            duration_range: be_i16(d, o + 2),
            intensity: be_u16(d, o + 4),
            intensity_range: be_u16(d, o + 6),
            transition: be_i16(d, o + 8),
            transition_range: be_i16(d, o + 10),
        };
        Some(Self {
            flags: u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
            scale_x: [be_i16(d, 0x34), be_i16(d, 0x36)],
            scale_y: [be_i16(d, 0x38), be_i16(d, 0x3a)],
            states: [state(0x3c), state(0x48)],
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticleSystem {
    pub definition: ParticleSystemDefinition,
    pub state: i16,
    pub timer: i32,
    pub transitioning: bool,
    pub transition_duration: i16,
    pub previous_intensity: u16,
    pub intensity: u16,
    pub multiplier: i16,
}

fn drawn_i16(r: &mut EngineRng, site: u32, base: i16, range: i16) -> i32 {
    let mut v = 0;
    if range > 0 {
        v = (i32::from(r.draw(site)) * i32::from(range)) >> 16;
    }
    v + i32::from(base)
}

fn drawn_u16(r: &mut EngineRng, site: u32, base: u16, range: u16) -> u16 {
    let mut v = 0;
    if range != 0 {
        v = (u32::from(r.draw(site)) * u32::from(range)) >> 16;
    }
    (v + u32::from(base)) as u16
}

fn cvttsd2si(x: f64) -> i32 {
    if x.is_nan() || x >= 2_147_483_648.0 || x < -2_147_483_648.0 {
        i32::MIN
    } else {
        x as i32
    }
}

fn cosine_step(x: f64, span: u16) -> u32 {
    let f = cvttsd2si(x * 65536.0) >> 1;
    if f < 0 {
        return 0;
    }
    (f.min(0xffff) as u32).wrapping_mul(u32::from(span)) >> 16
}

pub fn transition_cos(t: f32) -> f64 {
    f64::from(t).cos()
}

impl ParticleSystem {
    pub fn new_for_map(definition: ParticleSystemDefinition, r: &mut EngineRng) -> Self {
        let s0 = definition.states[0];
        let intensity = drawn_u16(
            r,
            draw_site::PARTICLE_INTENSITY,
            s0.intensity,
            s0.intensity_range,
        );
        let timer = drawn_i16(
            r,
            draw_site::PARTICLE_DURATION,
            s0.duration,
            s0.duration_range,
        );
        ParticleSystem {
            definition,
            state: 0,
            timer,
            transitioning: false,
            transition_duration: 0,
            previous_intensity: 0,
            intensity,
            multiplier: 0x100,
        }
    }

    pub fn update_clock(&mut self, r: &mut EngineRng) {
        if !self.transitioning && self.definition.flags & 1 != 0 {
            return;
        }
        self.timer -= 1;
        if self.timer > 0 {
            return;
        }
        let current = self.state;
        let mut next = current.wrapping_add(1);
        if next == 2 {
            next = 0;
        }
        let was_transitioning = self.transitioning;
        self.transitioning = !was_transitioning;
        let s = self.definition.states[current as usize & 1];
        if was_transitioning {
            self.timer = drawn_i16(
                r,
                draw_site::PARTICLE_CLOCK_DURATION,
                s.duration,
                s.duration_range,
            );
            self.state = next;
        } else {
            self.timer = drawn_i16(
                r,
                draw_site::PARTICLE_CLOCK_TRANSITION,
                s.transition,
                s.transition_range,
            );
            self.transition_duration = self.timer as i16;
            self.previous_intensity = self.intensity;
            let n = self.definition.states[next as usize & 1];
            self.intensity = drawn_u16(
                r,
                draw_site::PARTICLE_CLOCK_INTENSITY,
                n.intensity,
                n.intensity_range,
            );
        }
    }

    pub fn set_multiplier(&mut self, multiplier: i16) {
        self.multiplier = multiplier;
    }

    pub fn switch_states(&mut self, r: &mut EngineRng) {
        let current = self.state;
        let mut next = current.wrapping_add(1);
        if next == 2 {
            next = 0;
        }
        self.transitioning = true;
        let s = self.definition.states[current as usize & 1];
        self.timer = drawn_i16(
            r,
            draw_site::PARTICLE_SWITCH_TRANSITION,
            s.transition,
            s.transition_range,
        );
        self.transition_duration = self.timer as i16;
        self.previous_intensity = self.intensity;
        let n = self.definition.states[next as usize & 1];
        self.intensity = drawn_u16(
            r,
            draw_site::PARTICLE_SWITCH_INTENSITY,
            n.intensity,
            n.intensity_range,
        );
    }

    fn current_intensity(&self) -> u16 {
        if !self.transitioning {
            return self.intensity;
        }
        let d = i32::from(self.transition_duration);
        let t = (d.wrapping_sub(self.timer) as f32 * std::f32::consts::PI) / d as f32;
        let c = transition_cos(t);
        let to = self.intensity;
        let from = self.previous_intensity;
        if to > from {
            from.wrapping_add(cosine_step(1.0 - c, to.wrapping_sub(from)) as u16)
        } else {
            to.wrapping_add(cosine_step(c + 1.0, from.wrapping_sub(to)) as u16)
        }
    }

    pub fn intensity(&self, point: [i32; 3], mesh_cells: (i32, i32)) -> Result<u16, Unmodelled> {
        if self.definition.flags & 2 != 0 {
            return Ok(0);
        }
        let (width, height) = (mesh_cells.0 as i16, mesh_cells.1 as i16);
        if width == 0 || height == 0 {
            return Err(Unmodelled::new(
                draw_site::PARTICLE_MESH_SIZE_ZERO,
                "mesh size 0 in particle_system_intensity",
            ));
        }
        let lerp = |p: i32, size: i16, s: [i16; 2]| -> i16 {
            let cell = i32::from((p >> 9) as i16) << 8;
            let frac = i32::from((cell / i32::from(size)) as i16);
            let delta = (frac.wrapping_mul(i32::from(s[1]) - i32::from(s[0]))) >> 8;
            (i32::from(s[0] as u16) + delta) as i16
        };
        let sx = lerp(point[0], width, self.definition.scale_x);
        let sy = lerp(point[1], height, self.definition.scale_y);
        let v = (i32::from(sx)
            .wrapping_mul(i32::from(sy))
            .wrapping_mul(i32::from(self.current_intensity())) as u32)
            >> 16;
        let v = (i32::from(v as u16) * i32::from(self.multiplier)) >> 8;
        Ok(v.clamp(0, 0xffff) as u16)
    }
}

pub fn particle_system_intensity(
    system: Option<&ParticleSystem>,
    point: [i32; 3],
    mesh_cells: (i32, i32),
) -> Result<u16, Unmodelled> {
    system.map_or(Ok(0), |s| s.intensity(point, mesh_cells))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_rng::SiteCounts;
    use mandrake_math::BungieRng;

    fn definition() -> ParticleSystemDefinition {
        ParticleSystemDefinition {
            flags: 0,
            scale_x: [0x100, 0x100],
            scale_y: [0x100, 0x100],
            states: [
                ParticleStateDefinition {
                    duration: 3,
                    duration_range: 0,
                    intensity: 0x1000,
                    intensity_range: 0,
                    transition: 4,
                    transition_range: 0,
                },
                ParticleStateDefinition {
                    duration: 5,
                    duration_range: 0,
                    intensity: 0x3000,
                    intensity_range: 0,
                    transition: 2,
                    transition_range: 0,
                },
            ],
        }
    }

    #[test]
    fn the_clock_cycles_steady_and_transition_without_draws_for_zero_ranges() {
        let mut rng = BungieRng::new(1);
        let mut sites = SiteCounts::default();
        let mut r = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        let mut s = ParticleSystem::new_for_map(definition(), &mut r);
        assert_eq!((s.state, s.timer, s.intensity), (0, 3, 0x1000));
        for _ in 0..3 {
            s.update_clock(&mut r);
        }
        assert!(s.transitioning);
        assert_eq!((s.state, s.timer, s.transition_duration), (0, 4, 4));
        assert_eq!((s.previous_intensity, s.intensity), (0x1000, 0x3000));
        let i0 = s.intensity([0, 0, 0], (64, 64)).unwrap();
        assert!((0x0fff..=0x1000).contains(&i0), "{i0:#x}");
        for _ in 0..4 {
            s.update_clock(&mut r);
        }
        assert!(!s.transitioning);
        assert_eq!((s.state, s.timer), (1, 3));
        assert_eq!(s.intensity([0, 0, 0], (64, 64)).unwrap(), 0x3000);
        assert_eq!(r.sites.total(), 0);
    }

    #[test]
    fn intensity_is_scaled_by_the_edges_and_the_multiplier() {
        let mut rng = BungieRng::new(1);
        let mut sites = SiteCounts::default();
        let mut r = EngineRng {
            rng: &mut rng,
            sites: &mut sites,
        };
        let mut d = definition();
        d.scale_x = [0, 0x200];
        let mut s = ParticleSystem::new_for_map(d, &mut r);
        assert_eq!(s.intensity([32 << 9, 0, 0], (64, 64)).unwrap(), 0x1000);
        s.multiplier = 0x80;
        assert_eq!(s.intensity([32 << 9, 0, 0], (64, 64)).unwrap(), 0x800);
        s.definition.flags = 2;
        assert_eq!(s.intensity([32 << 9, 0, 0], (64, 64)).unwrap(), 0);
        assert_eq!(
            particle_system_intensity(None, [0; 3], (64, 64)).unwrap(),
            0
        );
    }
}
