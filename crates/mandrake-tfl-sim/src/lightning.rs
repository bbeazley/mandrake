use mandrake_common::FourCC;

use crate::combat::distance3;
use crate::draw_site;
use crate::objects::DatumPool;
use crate::{Stop, TflSim};

pub const MAX_LIGHTNING: usize = 0x20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TflLightning {
    pub identifier: i16,
    pub definition: FourCC,
    pub source: i16,
    pub source_id: i16,
    pub target: i16,
    pub target_id: i16,
    pub start: [i32; 3],
    pub end: [i32; 3],
    pub control: [[i32; 3]; 2],
    pub last: [i32; 3],
    pub age: i32,
    pub state_ticks: i32,
    pub duration: i32,
    pub length: i16,
    pub state: i16,
    pub tip: [i32; 3],
}

#[derive(Debug, Clone)]
pub struct Lightnings {
    pub list: Vec<TflLightning>,
    pub pool: DatumPool,
}

impl Default for Lightnings {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            pool: DatumPool::new(MAX_LIGHTNING),
        }
    }
}

impl Lightnings {
    fn used(list: &[TflLightning]) -> impl Fn(usize) -> bool + '_ {
        |k| list.get(k).is_some_and(|l| l.identifier != 0)
    }

    pub fn datum(&self, slot: i16, identifier: i16) -> Option<usize> {
        let s = usize::try_from(slot).ok()?;
        self.list
            .get(s)
            .is_some_and(|l| l.identifier != 0 && l.identifier == identifier)
            .then_some(s)
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &TflLightning)> {
        self.list
            .iter()
            .enumerate()
            .filter(|(_, l)| l.identifier != 0)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LightningRequest {
    pub definition: Option<FourCC>,
    pub source: i16,
    pub target: i16,
    pub start: [i32; 3],
    pub end: [i32; 3],
}

fn curve(t: i32, p0: [i32; 3], p1: [i32; 3], p2: [i32; 3], p3: [i32; 3]) -> [i32; 3] {
    let mut out = [0; 3];
    for k in 0..3 {
        let a = p1[k]
            .wrapping_sub(p2[k])
            .wrapping_mul(3)
            .wrapping_add(p3[k])
            .wrapping_sub(p0[k]);
        let c = p0[k]
            .wrapping_sub(p1[k].wrapping_mul(2))
            .wrapping_add(p2[k])
            .wrapping_mul(3);
        let b = p1[k].wrapping_sub(p0[k]).wrapping_mul(3);
        let v = (a.wrapping_mul(t) >> 8).wrapping_add(c);
        let v = (v.wrapping_mul(t) >> 8).wrapping_add(b);
        out[k] = (v.wrapping_mul(t) >> 8).wrapping_add(p0[k]);
    }
    out
}

fn d3(a: [i32; 3], b: [i32; 3]) -> i32 {
    distance3((a[0], a[1], a[2]), (b[0], b[1], b[2])) as i32
}

fn quarter(v: i32) -> i32 {
    v.wrapping_add(if v < 0 { 3 } else { 0 }) >> 2
}

impl TflSim {
    fn ligh(&mut self, id: FourCC) -> Result<std::sync::Arc<crate::data::Def>, Stop> {
        self.data.def_arc(b"ligh", id)
    }

    pub(crate) fn object_datum(&self, slot: i16, identifier: i16) -> Option<usize> {
        let s = usize::try_from(slot).ok()?;
        self.objects
            .get(s)
            .is_some_and(|o| o.identifier != 0 && o.identifier == identifier)
            .then_some(s)
    }

    pub(crate) fn new_lightning(&mut self, q: &LightningRequest) -> Result<Option<usize>, Stop> {
        let Some(def_id) = q.definition else {
            return Ok(None);
        };
        let def = self.ligh(def_id)?;
        let list = &self.lightning.list;
        let Some((slot, salt)) = self.lightning.pool.allocate(Lightnings::used(list)) else {
            return Ok(None);
        };
        let id_of = |o: i16| -> i16 {
            if o == -1 {
                -1
            } else {
                self.objects[o as usize].identifier
            }
        };
        let record = TflLightning {
            identifier: salt,
            definition: def_id,
            source: q.source,
            source_id: id_of(q.source),
            target: q.target,
            target_id: id_of(q.target),
            start: q.start,
            end: q.end,
            control: [[0; 3]; 2],
            last: [0; 3],
            age: 0,
            state_ticks: 0,
            duration: 0,
            length: 0,
            state: 0,
            tip: q.start,
        };
        if slot == self.lightning.list.len() {
            self.lightning.list.push(record);
        } else {
            self.lightning.list[slot] = record;
        }
        self.follow_lightning(slot)?;
        let l = &mut self.lightning.list[slot];
        l.last = l.start;
        let (start, end) = (l.start, l.end);
        let half = d3(start, end) >> 1;
        let q4 = half >> 1;
        let v = [
            quarter(end[0].wrapping_sub(start[0])),
            quarter(end[1].wrapping_sub(start[1])),
            quarter(end[2].wrapping_sub(start[2])),
        ];
        let mut r = |site: u32, n: i32| -> i32 {
            if n > 0 {
                (i32::from(self.rng.draw(site)).wrapping_mul(n)) >> 16
            } else {
                0
            }
        };
        let c0 = [
            start[0]
                .wrapping_add(r(draw_site::LIGHTNING_MID1_X, half))
                .wrapping_add(v[0])
                .wrapping_sub(q4),
            start[1]
                .wrapping_add(r(draw_site::LIGHTNING_MID1_Y, half))
                .wrapping_add(v[1])
                .wrapping_sub(q4),
            start[2]
                .wrapping_add(r(draw_site::LIGHTNING_MID1_Z, q4))
                .wrapping_add(v[2]),
        ];
        let c1 = [
            end[0]
                .wrapping_sub(v[0])
                .wrapping_add(r(draw_site::LIGHTNING_MID2_X, half))
                .wrapping_sub(q4),
            end[1]
                .wrapping_sub(v[1])
                .wrapping_add(r(draw_site::LIGHTNING_MID2_Y, half))
                .wrapping_sub(q4),
            end[2]
                .wrapping_sub(v[2])
                .wrapping_add(r(draw_site::LIGHTNING_MID2_Z, q4)),
        ];
        let d = d3(start, end);
        let speed = i32::from(def.i16(0x1a));
        if speed == 0 {
            return Err(Stop::new(
                draw_site::LIGHTNING_SPEED_ZERO,
                "a lightning of speed 0 (divides by zero)",
            ));
        }
        let l = &mut self.lightning.list[slot];
        l.control = [c0, c1];
        l.duration = d / speed;
        l.length = (d / 256) as i16;
        l.age = 0;
        if l.state != 0 {
            l.state_ticks = 0;
            l.state = 0;
        }
        Ok(Some(slot))
    }

    fn follow_lightning(&mut self, k: usize) -> Result<(), Stop> {
        let l = &self.lightning.list[k];
        let (source, source_id, target, target_id) = (l.source, l.source_id, l.target, l.target_id);
        if source != -1 && self.object_datum(source, source_id).is_none() {
            self.lightning.list[k].state = 2;
        }
        if target != -1 {
            match self.object_datum(target, target_id) {
                Some(o) => {
                    let (x, y, z) = self.object_location(1, o);
                    return self.set_lightning_ends(k, None, Some([x, y, z]));
                }
                None => self.lightning.list[k].state = 2,
            }
        }
        Ok(())
    }

    pub(crate) fn set_lightning_ends(
        &mut self,
        k: usize,
        start: Option<[i32; 3]>,
        end: Option<[i32; 3]>,
    ) -> Result<(), Stop> {
        let def = self.ligh(self.lightning.list[k].definition)?;
        let l = &mut self.lightning.list[k];
        if let Some(s) = start {
            l.start = s;
        }
        if let Some(e) = end {
            l.end = e;
        }
        let d = d3(l.start, l.end);
        let speed = i32::from(def.i16(0x1a));
        if speed == 0 {
            return Err(Stop::new(
                draw_site::LIGHTNING_ENDS_SPEED_ZERO,
                "a lightning of speed 0 (divides by zero)",
            ));
        }
        l.duration = d / speed;
        l.length = (d / 256) as i16;
        Ok(())
    }

    pub(crate) fn update_lightning(&mut self) -> Result<(), Stop> {
        for k in 0..self.lightning.list.len() {
            if self.lightning.list[k].identifier == 0 {
                continue;
            }
            let def = self.ligh(self.lightning.list[k].definition)?;
            let l = &self.lightning.list[k];
            let reach = i32::from(def.i16(0x1a)).wrapping_mul(l.age);
            let d = d3(l.start, l.end);
            self.follow_lightning(k)?;
            let l = &mut self.lightning.list[k];
            l.age = l.age.wrapping_add(1);
            l.state_ticks = l.state_ticks.wrapping_add(1);
            if (def.u32(0) & 1 != 0 && reach > (def.u32(0x10) as i32).wrapping_add(d))
                || (l.state == 2 && l.state_ticks > (def.u32(0x0c) as i32))
            {
                l.identifier = 0;
                let list = &self.lightning.list;
                self.lightning.pool.free(k, Lightnings::used(list));
                continue;
            }
            match l.state {
                0 if l.state_ticks > l.duration => {
                    l.state = 1;
                    l.state_ticks = 0;
                }
                1 if l.state_ticks > (def.u32(0x08) as i32) => {
                    l.state = 2;
                    l.state_ticks = 0;
                }
                _ => {}
            }
            let mut tip = l.end;
            if d != 0 {
                let t = (reach.wrapping_shl(8) / d) as i16;
                if t < 0 {
                    continue;
                }
                if t < 0x100 {
                    tip = curve(i32::from(t), l.start, l.control[0], l.control[1], l.end);
                }
            }
            l.tip = tip;
            if l.last != l.end {
                let from = l.last;
                self.sweep_lightning(from, tip, &def)?;
                self.lightning.list[k].last = tip;
            }
        }
        Ok(())
    }

    fn sweep_lightning(
        &mut self,
        from: [i32; 3],
        to: [i32; 3],
        def: &crate::data::Def,
    ) -> Result<(), Stop> {
        let n = d3(from, to) >> 8;
        if n <= 0 {
            return Ok(());
        }
        let flags = def.u32(0);
        let step: [i32; 3] =
            std::array::from_fn(|k| to[k].wrapping_sub(from[k]).wrapping_mul(0x10000) / n);
        let damage = crate::damage::DamageDef::read(def, 0x28);
        if flags & 4 != 0 && damage.flags & 3 == 3 {
            return Err(Stop::new(
                draw_site::LIGHTNING_AREA_DAMAGE,
                "a lightning bolt's area damage scaled by speed (the context's velocity is the stack's leftovers)",
            ));
        }
        let mut ctx = crate::damage::DamageContext {
            attacker_palette: -1,
            attacker: -1,
            attacker_id: -1,
            group: None,
            point: [0; 3],
            velocity: [0; 3],
            scale: 0,
            force: 0,
            amount: 0,
            flags: 0,
            record: None,
        };
        let mut acc = [0i32; 3];
        for _ in 0..n {
            let at: [i32; 3] = std::array::from_fn(|k| (acc[k] >> 16).wrapping_add(from[k]));
            if flags & 4 != 0 {
                ctx.point = at;
                self.damage_object(None, &damage, &mut ctx)?;
            }
            if flags & 2 != 0 {
                self.stain_mesh(draw_site::LIGHTNING_STAIN, def.tag(0x04), def.i16(0x18), at);
            }
            for k in 0..3 {
                acc[k] = acc[k].wrapping_add(step[k]);
            }
        }
        Ok(())
    }
}
