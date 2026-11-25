use crate::data::Def;
use crate::draw_site;
use crate::rng::TflRng;

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

impl ParticleSystemDefinition {
    pub fn parse(d: &Def) -> Self {
        let state = |o: usize| ParticleStateDefinition {
            duration: d.i16(o),
            duration_range: d.i16(o + 2),
            intensity: d.u16(o + 4),
            intensity_range: d.u16(o + 6),
            transition: d.i16(o + 8),
            transition_range: d.i16(o + 10),
        };
        Self {
            flags: d.u32(0),
            scale_x: [d.i16(0x34), d.i16(0x36)],
            scale_y: [d.i16(0x38), d.i16(0x3a)],
            states: [state(0x3c), state(0x48)],
        }
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

fn drawn_i16(r: &mut TflRng, site: u32, base: i16, range: i16) -> i32 {
    let v = if range > 0 {
        (i32::from(r.draw(site)) * i32::from(range)) >> 16
    } else {
        0
    };
    i32::from(base) + v
}

fn drawn_u16(r: &mut TflRng, site: u32, base: u16, range: u16) -> u16 {
    let v = if range != 0 {
        (u32::from(r.draw(site)) * u32::from(range)) >> 16
    } else {
        0
    };
    base.wrapping_add(v as u16)
}

fn ftol(x: f64) -> i32 {
    if x.is_nan() || x >= 2_147_483_648.0 || x < -2_147_483_648.0 {
        i32::MIN
    } else {
        x as i32
    }
}

pub const PI_F32: f32 = std::f32::consts::PI;

pub fn transition_blend(d: i16, timer: i32, rising: bool) -> f64 {
    let t = f64::from(i32::from(d).wrapping_sub(timer)) * f64::from(PI_F32) / f64::from(d);
    let c = t.cos();
    if rising {
        (1.0 - c) * 65536.0
    } else {
        (c + 1.0) * 65536.0
    }
}

impl ParticleSystem {
    pub fn new_for_map(definition: ParticleSystemDefinition, r: &mut TflRng) -> Self {
        let s0 = definition.states[0];
        let intensity = drawn_u16(
            r,
            draw_site::PARTICLE_INTENSITY,
            s0.intensity,
            s0.intensity_range,
        );
        let mut s = ParticleSystem {
            definition,
            state: 0,
            timer: 0,
            transitioning: false,
            transition_duration: 0,
            previous_intensity: 0,
            intensity,
            multiplier: 0x100,
        };
        s.reset_clock(r);
        s
    }

    fn reset_clock(&mut self, r: &mut TflRng) {
        let s = self.definition.states[self.state as usize & 1];
        self.timer = if self.transitioning {
            drawn_i16(
                r,
                draw_site::PARTICLE_TRANSITION,
                s.transition,
                s.transition_range,
            )
        } else {
            drawn_i16(
                r,
                draw_site::PARTICLE_DURATION,
                s.duration,
                s.duration_range,
            )
        };
    }

    fn next_state(&self) -> i16 {
        let n = self.state.wrapping_add(1);
        if n == 2 { 0 } else { n }
    }

    fn start_transition(&mut self, r: &mut TflRng, next: i16, site: u32) {
        self.transition_duration = self.timer as i16;
        self.previous_intensity = self.intensity;
        let n = self.definition.states[next as usize & 1];
        self.intensity = drawn_u16(r, site, n.intensity, n.intensity_range);
    }

    pub fn update_clock(&mut self, r: &mut TflRng) {
        if !self.transitioning && self.definition.flags & 1 != 0 {
            return;
        }
        self.timer = self.timer.wrapping_sub(1);
        if self.timer > 0 {
            return;
        }
        let next = self.next_state();
        self.transitioning = !self.transitioning;
        self.reset_clock(r);
        if self.transitioning {
            self.start_transition(r, next, draw_site::PARTICLE_CLOCK_TRANSITION);
        } else {
            self.state = next;
        }
    }

    pub fn switch_states(&mut self, r: &mut TflRng) {
        let next = self.next_state();
        self.transitioning = true;
        self.reset_clock(r);
        self.start_transition(r, next, draw_site::PARTICLE_SWITCH_TRANSITION);
    }

    pub fn current_intensity(&self) -> u16 {
        if !self.transitioning {
            return self.intensity;
        }
        let (to, from) = (self.intensity, self.previous_intensity);
        let rising = to > from;
        let f = (ftol(transition_blend(
            self.transition_duration,
            self.timer,
            rising,
        )) >> 1)
            .clamp(0, 0xffff);
        let (base, span) = if rising {
            (from, i32::from(to) - i32::from(from))
        } else {
            (to, i32::from(from) - i32::from(to))
        };
        (span.wrapping_mul(f) >> 16).wrapping_add(i32::from(base)) as u16
    }

    pub fn density(&self, x: i32, y: i32, width: i16, height: i16) -> Option<u16> {
        if width == 0 || height == 0 {
            return None;
        }
        let lerp = |p: i32, size: i16, s: [i16; 2]| -> i16 {
            let frac = ((i32::from((p >> 10) as i16) << 8) / i32::from(size)) as i16;
            let delta = i32::from(frac).wrapping_mul(i32::from(s[1]) - i32::from(s[0])) >> 8;
            delta.wrapping_add(i32::from(s[0])) as i16
        };
        let sx = lerp(x, width, self.definition.scale_x);
        let sy = lerp(y, height, self.definition.scale_y);
        let v = i32::from(self.current_intensity())
            .wrapping_mul(i32::from(sy))
            .wrapping_mul(i32::from(sx));
        let v = (i32::from((v >> 16) as u16) * i32::from(self.multiplier)) >> 8;
        Some(v.clamp(0, 0xffff) as u16)
    }

    pub fn intensity(&self, x: i32, y: i32, width: i16, height: i16) -> Option<u16> {
        if self.definition.flags & 2 != 0 {
            return Some(0);
        }
        self.density(x, y, width, height)
    }
}

impl crate::TflSim {
    pub fn particle_system_tag(&self) -> Option<mandrake_common::FourCC> {
        let t = self.data.mesh_bytes.get(0x50..0x54)?;
        let v = u32::from_be_bytes([t[0], t[1], t[2], t[3]]);
        (v != 0xffff_ffff).then(|| mandrake_common::FourCC(v.to_be_bytes()))
    }

    pub(crate) fn initialize_particle_system(&mut self) -> Result<(), crate::Stop> {
        let Some(tag) = self.particle_system_tag() else {
            return Ok(());
        };
        let def = self.data.def(b"part", tag).map_err(|e| {
            crate::Stop::new(
                draw_site::PARTICLE_SYSTEM_LOAD,
                format!("the particle system {tag} doesn't load: {}", e.reason),
            )
        })?;
        let def = ParticleSystemDefinition::parse(def);
        self.particles = Some(ParticleSystem::new_for_map(def, &mut self.rng));
        Ok(())
    }

    pub(crate) fn update_particle_clock(&mut self) {
        if let Some(p) = &mut self.particles {
            p.update_clock(&mut self.rng);
        }
    }

    pub(crate) fn particle_intensity_at(&self, x: i32, y: i32) -> Option<u16> {
        match &self.particles {
            None => Some(0),
            Some(p) => p.intensity(
                x,
                y,
                self.data.mesh.cell_width as i16,
                self.data.mesh.cell_height as i16,
            ),
        }
    }
}

impl crate::TflSim {
    pub(crate) fn particle_system_action(&mut self, i: usize) -> Result<u32, crate::Stop> {
        let switch =
            crate::map_actions::find(&self.map_actions.actions, i, 2, b"swit", 0).is_some();
        let mult = crate::map_actions::find(&self.map_actions.actions, i, 0, b"mult", 8)
            .filter(|p| p.count > 0)
            .map(|p| p.i32_at(0));
        if (switch || mult.is_some()) && self.particles.is_none() {
            return Err(crate::Stop::new(
                draw_site::PARTICLE_SYSTEM_MISSING,
                format!("map action {i}: part on a map without a particle system"),
            ));
        }
        tracing::debug!(
            "PART tick {} action {i} switch {switch} mult {mult:?}",
            self.tick
        );
        if switch {
            let p = self.particles.as_mut().unwrap();
            p.switch_states(&mut self.rng);
        }
        if let Some(m) = mult {
            self.particles.as_mut().unwrap().multiplier = (m >> 8) as i16;
        }
        Ok(if switch || mult.is_some() { 2 } else { 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> ParticleSystemDefinition {
        let s = |duration, intensity, transition| ParticleStateDefinition {
            duration,
            duration_range: 0,
            intensity,
            intensity_range: 0,
            transition,
            transition_range: 0,
        };
        ParticleSystemDefinition {
            flags: 0,
            scale_x: [0x100, 0x100],
            scale_y: [0x100, 0x100],
            states: [s(3, 0x1000, 4), s(5, 0x3000, 2)],
        }
    }

    #[test]
    fn the_clock_cycles_steady_and_transition() {
        let mut r = TflRng::new(1);
        let mut s = ParticleSystem::new_for_map(definition(), &mut r);
        assert_eq!((s.state, s.timer, s.intensity), (0, 3, 0x1000));
        for _ in 0..3 {
            s.update_clock(&mut r);
        }
        assert!(s.transitioning);
        assert_eq!((s.state, s.timer, s.transition_duration), (0, 4, 4));
        assert_eq!((s.previous_intensity, s.intensity), (0x1000, 0x3000));
        assert_eq!(s.density(0, 0, 64, 64), Some(0x1000));
        s.update_clock(&mut r);
        s.update_clock(&mut r);
        assert_eq!(s.current_intensity(), 0x2000);
        s.update_clock(&mut r);
        s.update_clock(&mut r);
        assert!(!s.transitioning);
        assert_eq!((s.state, s.timer), (1, 3));
        assert_eq!(s.density(0, 0, 64, 64), Some(0x3000));
        assert!(r.log.is_empty());
    }

    #[test]
    fn the_density_follows_the_edges_and_the_multiplier() {
        let mut r = TflRng::new(1);
        let mut d = definition();
        d.scale_x = [0, 0x200];
        let mut s = ParticleSystem::new_for_map(d, &mut r);
        assert_eq!(s.density(32 << 10, 0, 64, 64), Some(0x1000));
        s.multiplier = 0x80;
        assert_eq!(s.density(32 << 10, 0, 64, 64), Some(0x800));
        s.definition.flags = 2;
        assert_eq!(s.intensity(32 << 10, 0, 64, 64), Some(0));
    }
}
