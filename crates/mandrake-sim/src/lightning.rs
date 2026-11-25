use mandrake_common::FourCC;
use mandrake_tag::projectile::DamageDefinition;

use crate::draw_site;
use crate::engine_rng::{EngineRng, EngineWorld};
use crate::explosions::distance3d;

pub const LIGHTNING_RECORDS: i16 = 0x20;

thread_local! {
    pub static LIGHTNING_LOG: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

fn log(line: impl FnOnce() -> String) {
    LIGHTNING_LOG.with(|l| {
        if let Some(v) = l.borrow_mut().as_mut() {
            v.push(line());
        }
    });
}

fn log_records(tag: &str, w: &EngineWorld) {
    if w.lightning.records.is_empty() {
        return;
    }
    log(|| {
        let mut out = Vec::new();
        for r in &w.lightning.records {
            let (speed, flags) = w
                .catalog
                .lightnings
                .get(&r.system)
                .map_or((0, 0), |d| (d.speed, d.flags));
            let mut v = vec![
                r.index.to_string(),
                r.identifier.to_string(),
                speed.to_string(),
                format!("{flags:#x}"),
            ];
            v.extend(
                [r.source.0, r.source.1, r.target.0, r.target.1]
                    .iter()
                    .map(|x| x.to_string()),
            );
            for p in [r.origin, r.end, r.mid1, r.mid2, r.last] {
                v.extend(p.iter().map(|x| x.to_string()));
            }
            v.extend(
                [r.ticks, r.state_ticks, r.travel_ticks]
                    .iter()
                    .map(|x| x.to_string()),
            );
            v.push(r.segments.to_string());
            v.push(r.state.to_string());
            v.extend(r.head.iter().map(|x| x.to_string()));
            out.push(format!("{tag} {}", v.join(" ")));
        }
        out.join("\n")
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightningDefinition {
    pub tag: FourCC,
    pub flags: u32,
    pub collection: Option<FourCC>,
    pub state_1_ticks: i32,
    pub state_2_ticks: i32,
    pub overrun: i32,
    pub sequences: [i16; 3],
    pub speed: i16,
    pub damage: DamageDefinition,
}

impl LightningDefinition {
    pub fn from_bytes(tag: FourCC, data: &[u8]) -> Option<Self> {
        if data.len() < 0x38 {
            return None;
        }
        let w = |o: usize| i16::from_be_bytes([data[o], data[o + 1]]);
        let d = |o: usize| i32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
        let collection = match d(4) as u32 {
            0 | u32::MAX => None,
            _ => Some(FourCC::new([data[4], data[5], data[6], data[7]])),
        };
        Some(Self {
            tag,
            flags: d(0) as u32,
            collection,
            state_1_ticks: d(8),
            state_2_ticks: d(0xc),
            overrun: d(0x10),
            sequences: [w(0x14), w(0x16), w(0x18)],
            speed: w(0x1a),
            damage: DamageDefinition::from_bytes(&data[0x28..0x38]).ok()?,
        })
    }

    fn segments(&self, d: i32, new: bool) -> i16 {
        let s = self.state_2_ticks;
        if s < 1 {
            (d / 128) as i16
        } else if new && s < 0x10 {
            (d / 0xa0) as i16
        } else {
            (d / 0x155) as i16
        }
    }

    fn travel_ticks(&self, d: i32) -> i32 {
        if self.speed == 0 {
            0
        } else {
            d / i32::from(self.speed)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightningRecord {
    pub index: i16,
    pub identifier: i16,
    pub system: FourCC,
    pub source: (i16, i16),
    pub target: (i16, i16),
    pub origin: [i32; 3],
    pub end: [i32; 3],
    pub mid1: [i32; 3],
    pub mid2: [i32; 3],
    pub last: [i32; 3],
    pub ticks: i32,
    pub state_ticks: i32,
    pub travel_ticks: i32,
    pub segments: i16,
    pub state: i16,
    pub head: [i32; 3],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightningData {
    pub records: Vec<LightningRecord>,
    pub next_identifier: u16,
}

impl Default for LightningData {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            next_identifier: 0x8000,
        }
    }
}

impl LightningData {
    pub fn get(&self, index: i16, identifier: i16) -> Option<&LightningRecord> {
        self.records
            .iter()
            .find(|r| r.index == index && r.identifier == identifier)
    }

    fn position(&self, index: i16) -> Option<usize> {
        self.records.iter().position(|r| r.index == index)
    }

    fn new_datum(&mut self) -> Option<(usize, i16, i16)> {
        let index = (0..LIGHTNING_RECORDS).find(|i| self.records.iter().all(|r| r.index != *i))?;
        let identifier = self.next_identifier as i16;
        self.next_identifier = match self.next_identifier.wrapping_add(1) {
            0 => 0x8000,
            n => n,
        };
        let at = self
            .records
            .iter()
            .position(|r| r.index > index)
            .unwrap_or(self.records.len());
        Some((at, index, identifier))
    }

    pub fn state(&self, index: i16) -> Option<i16> {
        self.records
            .iter()
            .find(|r| r.index == index)
            .map(|r| r.state)
    }
}

pub fn lightning_midpoints(
    pt1: [i32; 3],
    pt2: [i32; 3],
    mut draw: impl FnMut(u32) -> u16,
) -> ([i32; 3], [i32; 3]) {
    let dist = distance3d(pt1, pt2);
    let half = dist >> 1;
    let quarter = dist >> 2;
    let qx = (pt2[0] - pt1[0]) / 4;
    let qy = (pt2[1] - pt1[1]) / 4;
    let qz = (pt2[2] - pt1[2]) / 4;
    let jitter = |r: u16, m: i32| (i32::from(r) * m) >> 16;
    let (x1, y1) = if half > 0 {
        let jx = jitter(draw(draw_site::LIGHTNING_MID1_X), half);
        let jy = jitter(draw(draw_site::LIGHTNING_MID1_Y), half);
        (pt1[0] + qx - quarter + jx, pt1[1] + qy - quarter + jy)
    } else {
        (pt1[0] + qx - quarter, pt1[1] + qy - quarter)
    };
    let z1 = pt1[2]
        + qz
        + if quarter > 0 {
            jitter(draw(draw_site::LIGHTNING_MID1_Z), quarter)
        } else {
            0
        };
    let (x2, y2) = if half > 0 {
        let jx = jitter(draw(draw_site::LIGHTNING_MID2_X), half);
        let jy = jitter(draw(draw_site::LIGHTNING_MID2_Y), half);
        (pt2[0] - qx - quarter + jx, pt2[1] - qy - quarter + jy)
    } else {
        (pt2[0] - qx - quarter, pt2[1] - qy - quarter)
    };
    let z2 = pt2[2] - qz
        + if quarter > 0 {
            jitter(draw(draw_site::LIGHTNING_MID2_Z), quarter)
        } else {
            0
        };
    ([x1, y1, z1], [x2, y2, z2])
}

pub fn bezier(t: i16, p0: [i32; 3], p1: [i32; 3], p2: [i32; 3], p3: [i32; 3]) -> [i32; 3] {
    let t = i32::from(t);
    std::array::from_fn(|i| {
        let (a, b, c, d) = (p0[i], p1[i], p2[i], p3[i]);
        let x = ((b.wrapping_mul(3) - a + d + c.wrapping_mul(-3)).wrapping_mul(t)) >> 8;
        let x = ((x + (c + b.wrapping_mul(-2) + a).wrapping_mul(3)).wrapping_mul(t)) >> 8;
        let x = ((x + (b - a).wrapping_mul(3)).wrapping_mul(t)) >> 8;
        x + a
    })
}

pub fn beam_steps(last: [i32; 3], head: [i32; 3]) -> Vec<[i32; 3]> {
    let d = distance3d(last, head);
    if d <= 0x7f {
        return Vec::new();
    }
    let n = d >> 7;
    let step: [i32; 3] = std::array::from_fn(|i| (head[i] - last[i]).wrapping_shl(16) / n);
    let mut acc = [0i32; 3];
    (0..n)
        .map(|_| {
            let p = std::array::from_fn(|i| (acc[i] >> 16) + last[i]);
            for i in 0..3 {
                acc[i] = acc[i].wrapping_add(step[i]);
            }
            p
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightningStep {
    pub definition: LightningDefinition,
    pub points: Vec<[i32; 3]>,
}

impl EngineWorld {
    fn lightning_object(&self, object: i16) -> Option<(usize, i16)> {
        let idx = i32::from(object);
        if let Some(i) = self.monster_index_by_object(idx)
            && self.monsters[i].identifier != 0
        {
            return Some((i, self.monsters[i].object.identifier));
        }
        self.monsters
            .iter()
            .position(|m| m.identifier != 0 && m.object.index == idx)
            .map(|i| (i, self.monsters[i].object.identifier))
    }

    fn lightning_object_identifier(&self, object: i16) -> i16 {
        self.lightning_object(object)
            .or_else(|| {
                self.monster_index_by_object(i32::from(object))
                    .map(|i| (i, self.monsters[i].object.identifier))
            })
            .map_or(0, |(_, id)| id)
    }

    pub(crate) fn lightning_object_alive(&self, object: i16, identifier: i16) -> bool {
        self.lightning_object(object)
            .is_some_and(|(_, id)| id == identifier)
    }

    pub(crate) fn new_lightning(
        &mut self,
        rng: &mut EngineRng<'_>,
        system: Option<FourCC>,
        source: i16,
        target: i16,
        origin: [i32; 3],
        end: [i32; 3],
    ) -> Option<(i16, i16)> {
        let system = system?;
        let def = *self.catalog.lightnings.get(&system)?;
        let (at, index, identifier) = self.lightning.new_datum()?;
        let id = |o: i16| {
            if o == -1 {
                -1
            } else {
                self.lightning_object_identifier(o)
            }
        };
        let record = LightningRecord {
            index,
            identifier,
            system,
            source: (source, id(source)),
            target: (target, id(target)),
            origin,
            end,
            mid1: [0; 3],
            mid2: [0; 3],
            last: [0; 3],
            ticks: 0,
            state_ticks: 0,
            travel_ticks: 0,
            segments: 0,
            state: 0,
            head: origin,
        };
        self.lightning.records.insert(at, record);
        self.refresh_lightning(at);
        let r = &mut self.lightning.records[at];
        r.last = r.origin;
        let (mid1, mid2) = lightning_midpoints(r.origin, r.end, |site| rng.draw(site));
        r.mid1 = mid1;
        r.mid2 = mid2;
        let d = distance3d(r.origin, r.end);
        r.travel_ticks = def.travel_ticks(d);
        r.segments = def.segments(d, true);
        r.ticks = 0;
        if r.state != 0 {
            r.state_ticks = 0;
            r.state = 0;
        }
        log(|| format!("N {index}"));
        Some((index, identifier))
    }

    fn refresh_lightning(&mut self, at: usize) {
        let r = self.lightning.records[at];
        let mut state = r.state;
        let mut end = None;
        if r.source.0 != -1 && !self.lightning_object_alive(r.source.0, r.source.1) {
            state = 2;
        }
        if r.target.0 != -1 {
            match self.lightning_object(r.target.0) {
                Some((m, id)) if id == r.target.1 => {
                    end = self.monster_middle(m as i16);
                }
                _ => state = 2,
            }
        }
        let def = self.catalog.lightnings.get(&r.system).copied();
        let r = &mut self.lightning.records[at];
        r.state = state;
        if let (Some(end), Some(def)) = (end, def) {
            r.end = end;
            let d = distance3d(r.origin, r.end);
            r.travel_ticks = def.travel_ticks(d);
            r.segments = def.segments(d, false);
        }
    }

    pub(crate) fn change_lightning_endpoints(
        &mut self,
        index: i16,
        origin: Option<[i32; 3]>,
        end: Option<[i32; 3]>,
    ) {
        let Some(at) = self.lightning.position(index) else {
            return;
        };
        let def = self
            .catalog
            .lightnings
            .get(&self.lightning.records[at].system)
            .copied();
        let r = &mut self.lightning.records[at];
        if let Some(o) = origin {
            r.origin = o;
        }
        if let Some(e) = end {
            r.end = e;
        }
        if let Some(def) = def {
            let d = distance3d(r.origin, r.end);
            r.travel_ticks = def.travel_ticks(d);
            r.segments = def.segments(d, false);
        }
    }

    pub(crate) fn lightning_ground_hit(&mut self, index: i16) {
        let Some(at) = self.lightning.position(index) else {
            return;
        };
        let r = &mut self.lightning.records[at];
        r.target = (-1, -1);
        let head = r.head;
        self.change_lightning_endpoints(index, None, Some(head));
    }

    pub(crate) fn lightning_object_hit(&mut self, index: i16, object: i16) {
        let id = self.lightning_object_identifier(object);
        if let Some(at) = self.lightning.position(index) {
            self.lightning.records[at].target = (object, id);
        }
    }

    fn update_lightning_record(&mut self, at: usize) -> Option<LightningStep> {
        let r = self.lightning.records[at];
        let def = *self.catalog.lightnings.get(&r.system)?;
        let travel = i32::from(def.speed).wrapping_mul(r.ticks);
        let d = distance3d(r.origin, r.end);
        self.refresh_lightning(at);
        let r = &mut self.lightning.records[at];
        r.ticks = r.ticks.wrapping_add(1);
        r.state_ticks = r.state_ticks.wrapping_add(1);
        let delete = if def.flags & 1 != 0 && d.wrapping_add(def.overrun) < travel {
            true
        } else if r.state == 2 {
            def.state_2_ticks < r.state_ticks
        } else {
            false
        };
        if delete {
            self.lightning.records.remove(at);
            return None;
        }
        let r = &mut self.lightning.records[at];
        if r.state == 0 {
            if r.state_ticks > r.travel_ticks {
                r.state_ticks = 0;
                r.state = 1;
            }
        } else if r.state == 1 && r.state_ticks > def.state_1_ticks {
            r.state_ticks = 0;
            r.state = 2;
        }
        r.head = if d == 0 {
            r.end
        } else {
            let t = (travel.wrapping_shl(8) / d) as i16;
            if t < 0 {
                return None;
            }
            if t > 0xff {
                r.end
            } else {
                bezier(t, r.origin, r.mid1, r.mid2, r.end)
            }
        };
        if r.last == r.head {
            return None;
        }
        let points = beam_steps(r.last, r.head);
        r.last = r.head;
        Some(LightningStep {
            definition: def,
            points,
        })
    }
}

impl crate::Simulation {
    pub(crate) fn update_lightning(&mut self, tick: u32) {
        if self.engine_stop.is_some() {
            return;
        }
        {
            let w = &self.engine;
            log_records("A", w);
        }
        let mut last = -1;
        loop {
            let w = &mut self.engine;
            let Some(at) = w.lightning.records.iter().position(|r| r.index > last) else {
                log_records("U", w);
                return;
            };
            last = w.lightning.records[at].index;
            let Some(step) = w.update_lightning_record(at) else {
                continue;
            };
            let def = step.definition;
            for point in step.points {
                if def.flags & 4 != 0 {
                    log(|| format!("L {last} {} {} {}", point[0], point[1], point[2]));
                    self.combat.flight.detonating = Some(crate::explosions::ExplosionInfo {
                        unit: crate::statistics::UnitRef::NONE,
                        attacker: -1,
                        attacker_id: -1,
                        velocity: [0; 3],
                        flags: 0,
                        mana: 0,
                        factor_bits: 0,
                        source: None,
                    });
                    self.cause_damage(tick, def.tag, &def.damage, 0, None, Some(point));
                    self.combat.flight.detonating = None;
                }
                if def.flags & 2 != 0 && def.sequences[2] != -1 {
                    log(|| format!("S {last} {} {} {}", point[0], point[1], point[2]));
                    let collection = def.collection.unwrap_or(FourCC::new([0xff; 4]));
                    self.stain_mesh(tick, collection, def.sequences[2], point);
                }
            }
        }
    }
}
