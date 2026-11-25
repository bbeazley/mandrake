use mandrake_common::FourCC;

use crate::draw_site;
use crate::{Stop, TflSim};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attack {
    pub chance: u16,
    pub flags: u16,
    pub projectile: Option<FourCC>,
    pub min_range: i16,
    pub max_range: i16,
    pub sequence: i16,
    pub shots: u16,
    pub velocity: i16,
    pub velocity_range: i16,
    pub error: i16,
    pub recovery: i16,
    pub recovery_per_level: i16,
    pub sequences: [i16; 2],
}

pub const ARTIFACT_ATTACK: i16 = -0x45;

impl Attack {
    pub fn read(mons: &crate::data::Def, k: usize) -> Self {
        Self::read_at(mons, 0x98 + 0x20 * k)
    }

    pub fn read_at(mons: &crate::data::Def, o: usize) -> Self {
        Self {
            chance: mons.u16(o),
            flags: mons.u16(o + 2),
            projectile: mons.tag(o + 4),
            min_range: mons.i16(o + 8),
            max_range: mons.i16(o + 0xa),
            sequence: mons.i16(o + 0xc),
            shots: mons.u16(o + 0xe),
            velocity: mons.i16(o + 0x10),
            velocity_range: mons.i16(o + 0x12),
            error: mons.i16(o + 0x14),
            recovery: mons.i16(o + 0x16),
            sequences: [mons.i16(o + 0x18), mons.i16(o + 0x1a)],
            recovery_per_level: mons.i16(o + 0x1c),
        }
    }
}

pub fn distance3(a: (i32, i32, i32), b: (i32, i32, i32)) -> u32 {
    let (dx, dy, dz) = (a.0 - b.0, a.1 - b.1, a.2 - b.2);
    let ok = |d: i32| d < 0x7fff && d > -0x8000;
    if ok(dx) && ok(dy) && ok(dz) {
        crate::groups::isqrt(
            (dz.wrapping_mul(dz))
                .wrapping_add(dy.wrapping_mul(dy))
                .wrapping_add(dx.wrapping_mul(dx)) as u32,
        )
    } else {
        0x7fff
    }
}

impl TflSim {
    pub(crate) fn object_point(&self, o: usize) -> (i32, i32, i32) {
        let o = &self.objects[o];
        (o.x, o.y, o.z)
    }

    pub(crate) fn object_location(&self, mode: i32, o: usize) -> (i32, i32, i32) {
        let ob = &self.objects[o];
        if mode == 1 {
            (
                ob.x,
                ob.y,
                ((i32::from(ob.top) + i32::from(ob.bottom)) >> 1) + ob.z,
            )
        } else {
            (ob.x, ob.y, ob.z)
        }
    }

    pub(crate) fn target_location(
        &self,
        i: usize,
        mode: i32,
        lead: i32,
    ) -> Result<(i32, i32, i32), Stop> {
        let m = &self.monsters[i];
        if m.flags & 0x10 != 0 {
            let (x, y, z) = m.attack_point;
            return Ok((x, y, i32::from(z)));
        }
        let o = self.monsters[m.target as usize].object;
        let mut p = self.object_location(mode, o);
        if lead > 0 {
            let v = self.objects[o].velocity;
            p.0 += i32::from(v[0]) * lead;
            p.1 += i32::from(v[1]) * lead;
            p.2 += i32::from(v[2]) * lead;
        }
        Ok(p)
    }

    fn artifact_attack(&mut self, i: usize) -> Result<Attack, Stop> {
        let a = self.monsters[i].artifact.ok_or_else(|| {
            Stop::new(
                draw_site::ARTIFACT_ATTACK_NO_ARTIFACT,
                format!("monster {i}: attack -0x45, no artifact"),
            )
        })?;
        Ok(Attack::read_at(self.data.def(b"arti", a)?, 0x4c))
    }

    pub(crate) fn attack(&mut self, i: usize, k: i16) -> Result<Attack, Stop> {
        if k == ARTIFACT_ATTACK {
            return self.artifact_attack(i);
        }
        let mons = self.data.def(b"mons", self.monsters[i].definition)?;
        Ok(Attack::read(mons, k as usize))
    }

    pub(crate) fn choose_attack(&mut self, i: usize, mode: i32) -> Result<i16, Stop> {
        let (def, object, mflags, target_slot, artifact, carried, ammunition) = {
            let m = &self.monsters[i];
            (
                m.definition,
                m.object,
                m.flags,
                m.target,
                m.artifact,
                m.carried,
                m.ammunition,
            )
        };
        let mons = self.data.def_arc(b"mons", def)?;
        let mut dist = 0i32;
        let mut held = false;
        if mode != 2 && mode != 3 {
            let target = self.target_location(i, 0, 0)?;
            dist = distance3(self.object_point(object), target) as i32;
            if mflags & 8 != 0 {
                let td = self.monsters[target_slot as usize].definition;
                if self.data.def(b"mons", td)?.u32(0) & 0x1000 != 0 {
                    held = true;
                }
            }
        }
        let mut best: i16 = -1;
        let mut best_range = 0u32;
        let count = mons.i16(0x94).max(0);
        let artifact_attack = if let Some(id) = artifact {
            let arti = self.data.def(b"arti", id)?;
            if arti.u32(0) & 1 != 0
                && (arti.i16(0x0a) == 0
                    || self
                        .objects
                        .get(carried as usize)
                        .is_some_and(|o| o.charges > 0))
            {
                Some((ARTIFACT_ATTACK, Attack::read_at(arti, 0x4c)))
            } else {
                None
            }
        } else {
            None
        };
        let ranged = mons.i16(0x146);
        let candidates = artifact_attack
            .into_iter()
            .chain((0..count).map(|k| (k, Attack::read(&mons, k as usize))));
        for (k, a) in candidates {
            let af = a.flags;
            let mut ok;
            let mut range = 0u32;
            if mode == 2 {
                ok = af & 0x80 != 0;
            } else if mode == 3 {
                ok = af & 0x80 != 0 && af & 0x40 != 0;
            } else {
                ok = true;
                if dist < i32::from(a.min_range) {
                    range = 1;
                } else if dist > i32::from(a.max_range) {
                    range = 2;
                }
                if (af & 0x80 != 0) != (mflags & 0x80_0000 != 0) {
                    ok = false;
                }
                let mut check_range = true;
                if af & 0x200 == 0 {
                    if ranged == 0 {
                        if mode == 0 {
                            if mflags & 0x20_0000 != 0 {
                                ok = false;
                            }
                            check_range = false;
                        }
                    } else if ranged == 1 && mflags & 0x20_0000 == 0 {
                        ok = false;
                    }
                }
                if check_range && mode == 1 {
                    if range & 3 != 0 {
                        ok = false;
                    }
                } else if best != -1 && range & 1 != 0 && best_range & 2 != 0 {
                    ok = false;
                }
                if af & 0x1000 != 0 && !held {
                    ok = false;
                }
                if af & 0x2000 != 0 && held {
                    ok = false;
                }
            }
            if af & 0x20 != 0 && carried == -1 {
                ok = false;
            }
            if af & 0x10 != 0 && ammunition < 1 {
                ok = false;
            }
            let mut skip = false;
            if mflags & 1 != 0 && mflags & 0x10 != 0 {
                if af & 0x40 != 0 {
                    skip = true;
                } else if let Some(p) = a.projectile {
                    let pf = self.data.def(b"proj", p)?.u32(0);
                    if pf & 0x2000 != 0 && pf & 0x2000_0000 != 0 {
                        skip = true;
                    }
                }
            }
            if ok && !skip {
                best = k;
                best_range = range;
                if mode == 2 || mode == 3 || range & 3 == 0 {
                    return Ok(best);
                }
            }
        }
        Ok(best)
    }

    pub(crate) fn target_range(&mut self, i: usize) -> Result<i32, Stop> {
        let m = &self.monsters[i];
        let lead = i32::from(m.field_4a);
        let at = self.target_location(i, 0, lead)?;
        let me = self.object_point(self.monsters[i].object);
        let mut d = distance3(me, at) as i32;
        let dz = (at.2 as i16).wrapping_sub(me.2 as i16);
        if dz > 0 {
            d += i32::from(dz) * 3;
        }
        let m = &mut self.monsters[i];
        if d < 0x800 {
            m.flags |= 0x100;
        } else {
            m.flags &= !0x100;
        }
        if m.flags & 0x8_0000 != 0 {
            if m.flags & 8 == 0 {
                return Ok(i32::MIN);
            }
            let td = self.monsters[self.monsters[i].target as usize].definition;
            let keep = i32::from(self.data.def(b"mons", td)?.i16(0x184));
            if d < keep {
                return Ok(i32::MIN);
            }
            self.monsters[i].flags &= 0xffef_ffff;
            return Ok(0);
        }
        let k = self.choose_attack(i, 0)?;
        if k == -1 {
            return Ok(0);
        }
        let a = self.attack(i, k)?;
        let (min, max) = (i32::from(a.min_range), i32::from(a.max_range));
        if d < min {
            return Ok(d - min);
        }
        if d < max {
            return Ok(0);
        }
        let speed = i32::from(
            self.data
                .def(b"mons", self.monsters[i].definition)?
                .i16(0x8a),
        );
        if d - speed * i32::from(self.monsters[i].field_4a) < min {
            return Ok(0);
        }
        Ok(d - max)
    }

    pub(crate) fn face_toward_monster(&mut self, i: usize, t: usize) -> Result<u16, Stop> {
        let (mo, to) = (self.monsters[i].object, self.monsters[t].object);
        let (a, b) = (&self.objects[mo], &self.objects[to]);
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        if dx.abs() < 0x800 && dy.abs() < 0x800 {
            let s = i32::from(
                self.data
                    .def(b"mons", self.monsters[i].definition)?
                    .i16(0x8a),
            );
            let b = &self.objects[to];
            let (vx, vy) = (i32::from(b.velocity[0]), i32::from(b.velocity[1]));
            let dot = vx * dx + vy * dy;
            let qa = vx * vx - s * s + vy * vy;
            let b2 = dot * 2;
            let disc = (b2.wrapping_mul(b2))
                .wrapping_add((dy * dy + dx * dx).wrapping_mul(qa).wrapping_mul(-4));
            if disc >= 0 && qa != 0 {
                let r = crate::groups::isqrt(disc as u32) as i32;
                let t1 = (r + dot * -2) / (qa * 2);
                let mut t2 = (b2 + r) / (qa * -2);
                if t1 >= 0 && (t2 < 0 || t1 <= t2) {
                    t2 = t1;
                }
                if t2 > 0 {
                    let t2 = t2.min(0x78);
                    return Ok(crate::trig::arctangent(vx * t2 + dx, vy * t2 + dy));
                }
            }
        }
        Ok(crate::trig::arctangent(dx, dy))
    }
}
