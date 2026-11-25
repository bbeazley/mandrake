use crate::commands::MovementCommand;
use crate::draw_site;
use crate::{Order, Stop, TflSim};

const INVISIBLE_SUBJECTS: usize = 0x20;
const INVISIBLE_WAYPOINTS: usize = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvisibleEntry {
    pub monster: i16,
    pub identifier: i16,
    pub state: i16,
    pub timer: i8,
    pub repeats: i8,
    pub chance: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvisibleState {
    pub action: i16,
    pub monster: i16,
    pub identifier: i16,
    pub last: (i32, i32),
    pub moved: i32,
    pub media_radius: i32,
    pub radius: i32,
    pub chance: i16,
    pub chance_up: i16,
    pub chance_down: i16,
    pub spread: i32,
    pub entries: Vec<InvisibleEntry>,
    pub waypoints: Vec<(i32, i32)>,
    pub waypoint_spread: i32,
}

impl InvisibleState {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; 0x1f0];
        let put16 =
            |b: &mut Vec<u8>, o: usize, v: i16| b[o..o + 2].copy_from_slice(&v.to_le_bytes());
        put16(&mut b, 0, self.action);
        put16(&mut b, 2, self.monster);
        put16(&mut b, 4, self.identifier);
        let put32 =
            |b: &mut Vec<u8>, o: usize, v: i32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put32(&mut b, 8, self.last.0);
        put32(&mut b, 0xc, self.last.1);
        put32(&mut b, 0x10, self.moved);
        put32(&mut b, 0x14, self.media_radius);
        put32(&mut b, 0x18, self.radius);
        put16(&mut b, 0x1c, self.chance);
        put16(&mut b, 0x1e, self.chance_up);
        put16(&mut b, 0x20, self.chance_down);
        put32(&mut b, 0x24, self.spread);
        put16(&mut b, 0x28, self.entries.len() as i16);
        for (k, e) in self.entries.iter().enumerate() {
            let o = 0x2a + 10 * k;
            put16(&mut b, o, e.monster);
            put16(&mut b, o + 2, e.identifier);
            put16(&mut b, o + 4, e.state);
            b[o + 6] = e.timer as u8;
            b[o + 7] = e.repeats as u8;
            put16(&mut b, o + 8, e.chance);
        }
        put16(&mut b, 0x16a, self.waypoints.len() as i16);
        for (k, w) in self.waypoints.iter().enumerate() {
            put32(&mut b, 0x16c + 8 * k, w.0);
            put32(&mut b, 0x170 + 8 * k, w.1);
        }
        put32(&mut b, 0x1ec, self.waypoint_spread);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let i16_at = |o: usize| i16::from_le_bytes([b[o], b[o + 1]]);
        let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let n = (i16_at(0x28).max(0) as usize).min(INVISIBLE_SUBJECTS);
        let w = (i16_at(0x16a).max(0) as usize).min(INVISIBLE_WAYPOINTS);
        Self {
            action: i16_at(0),
            monster: i16_at(2),
            identifier: i16_at(4),
            last: (i32_at(8), i32_at(0xc)),
            moved: i32_at(0x10),
            media_radius: i32_at(0x14),
            radius: i32_at(0x18),
            chance: i16_at(0x1c),
            chance_up: i16_at(0x1e),
            chance_down: i16_at(0x20),
            spread: i32_at(0x24),
            entries: (0..n)
                .map(|k| {
                    let o = 0x2a + 10 * k;
                    InvisibleEntry {
                        monster: i16_at(o),
                        identifier: i16_at(o + 2),
                        state: i16_at(o + 4),
                        timer: b[o + 6] as i8,
                        repeats: b[o + 7] as i8,
                        chance: i16_at(o + 8),
                    }
                })
                .collect(),
            waypoints: (0..w)
                .map(|k| (i32_at(0x16c + 8 * k), i32_at(0x170 + 8 * k)))
                .collect(),
            waypoint_spread: i32_at(0x1ec),
        }
    }
}

impl TflSim {
    fn monster_datum(&self, slot: i16, identifier: i16) -> Option<usize> {
        let s = usize::try_from(slot).ok()?;
        self.monsters
            .get(s)
            .is_some_and(|m| m.identifier != 0 && m.identifier == identifier)
            .then_some(s)
    }

    pub(crate) fn invisible_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut s = match self.structure(i, b"#glo", 0x1f0) {
            Some(b) => InvisibleState::from_bytes(&b),
            None => match self.invisible_globals(i)? {
                Some(s) => s,
                None => return Ok(0),
            },
        };
        let target = self.monster_datum(s.monster, s.identifier);
        self.invisible_variables(&mut s)?;
        if s.entries.is_empty() {
            return Ok(4);
        }
        let Some(t) = target else {
            return Ok(4);
        };
        if self.monsters[t].flags_44 & 8 == 0 {
            self.activate_listed(i, b"visi")?;
            return Ok(4);
        }
        let r = self.invisible_states(&mut s);
        self.set_param(i, b"#glo", 1, 0x1f0, s.to_bytes());
        r.map(|()| 0)
    }

    fn invisible_globals(&mut self, i: usize) -> Result<Option<InvisibleState>, Stop> {
        let a = &self.map_actions.actions;
        let target = self.monster_slots(&crate::map_actions::combined(a, i, b"invi", 2, 1));
        if target.len() != 1 {
            return Ok(None);
        }
        let subjects = self.monster_slots(&crate::map_actions::combined(
            a,
            i,
            b"subj",
            2,
            INVISIBLE_SUBJECTS,
        ));
        let mut s = InvisibleState {
            action: i as i16,
            monster: target[0] as i16,
            identifier: self.monsters[target[0]].identifier,
            last: (0, 0),
            moved: 0,
            media_radius: 0x2000,
            radius: 0x1000,
            chance: 0,
            chance_up: 0x15,
            chance_down: 0xa,
            spread: 0xc00,
            entries: Vec::new(),
            waypoints: Vec::new(),
            waypoint_spread: 0,
        };
        for &m in &subjects {
            let r = self.rng.draw(draw_site::INVISIBLE_GLOBALS);
            s.entries.push(InvisibleEntry {
                monster: m as i16,
                identifier: self.monsters[m].identifier,
                state: 0,
                timer: (r % 12) as i8,
                repeats: 0,
                chance: s.chance,
            });
        }
        let a = &self.map_actions.actions;
        if let Some(p) = crate::map_actions::find(a, i, 2, b"wayp", 0xc) {
            if p.count > INVISIBLE_WAYPOINTS as i16 {
                return Ok(None);
            }
            s.waypoints = (0..p.count.max(0) as usize)
                .map(|k| (p.i32_at(2 * k), p.i32_at(2 * k + 1)))
                .collect();
            s.waypoint_spread = 0xc00;
        }
        Ok(Some(s))
    }

    fn invisible_variables(&mut self, s: &mut InvisibleState) -> Result<(), Stop> {
        let Some(t) = self.monster_datum(s.monster, s.identifier) else {
            return Ok(());
        };
        let to = self.monsters[t].object;
        let (tx, ty) = (self.objects[to].x, self.objects[to].y);
        let media = self.objects[to].flags & 0xe0 != 0;
        s.moved = i32::from(tx as i16 != s.last.0 as i16 || ty as i16 != s.last.1 as i16);
        let mut k = 0;
        while k < s.entries.len() {
            let e = s.entries[k];
            let Some(m) = self.monster_datum(e.monster, e.identifier) else {
                s.entries.remove(k);
                self.activate_listed(s.action as usize, b"casu")?;
                continue;
            };
            if s.moved != 0 {
                let o = self.monsters[m].object;
                let (x, y, facing) = (self.objects[o].x, self.objects[o].y, self.objects[o].facing);
                let d = crate::groups::distance((tx, ty), (x, y)) as i32;
                let radius = if media { s.media_radius } else { s.radius };
                if d < radius && (0..=1).contains(&e.state) {
                    let tr = crate::trig::trig();
                    let ix = crate::trig::index(i32::from(facing));
                    let front = ty
                        .wrapping_sub(y)
                        .wrapping_mul(i32::from(tr.sine[ix]))
                        .wrapping_add(i32::from(tr.cosine[ix]).wrapping_mul(tx.wrapping_sub(x)));
                    if front > 0
                        || i32::from(self.rng.draw(draw_site::INVISIBLE_VARIABLES_CHANCE) & 0xff)
                            < i32::from(e.chance)
                    {
                        let state = if media { 2 } else { 5 };
                        self.set_unit_state(s, state, k, Some((tx, ty)))?;
                    }
                }
            }
            k += 1;
        }
        s.last = (tx, ty);
        Ok(())
    }

    fn invisible_states(&mut self, s: &mut InvisibleState) -> Result<(), Stop> {
        let Some(t) = self.monster_datum(s.monster, s.identifier) else {
            return Ok(());
        };
        let to = self.monsters[t].object;
        let at = (self.objects[to].x, self.objects[to].y);
        for k in 0..s.entries.len() {
            let timer = s.entries[k].timer.wrapping_sub(1);
            s.entries[k].timer = timer;
            match s.entries[k].state {
                0 | 1 if timer < 0 => {
                    self.set_unit_state(s, 0, k, None)?;
                    let e = &mut s.entries[k];
                    e.chance = e.chance.wrapping_sub(s.chance_down);
                    if e.chance < 0 {
                        e.chance = 0;
                    }
                }
                2 => {
                    if timer < 2 && s.moved != 0 {
                        self.set_unit_state(s, 3, k, None)?;
                    } else if timer < 0 {
                        let r = (self.rng.draw(draw_site::INVISIBLE_STATE_CHANCE) & 0xff) as i16;
                        let e = &mut s.entries[k];
                        e.chance = e.chance.wrapping_add(s.chance_up);
                        let state = if r < e.chance { 3 } else { 1 };
                        self.set_unit_state(s, state, k, None)?;
                    }
                }
                3 if timer < 0 => self.set_unit_state(s, 6, k, None)?,
                4 if timer < 0 => self.set_unit_state(s, 7, k, Some(at))?,
                5 if timer < 0 => {
                    let r = (self.rng.draw(draw_site::INVISIBLE_STATE_NEXT) & 0xff) as i16;
                    let e = &mut s.entries[k];
                    e.chance = e.chance.wrapping_add(s.chance_up);
                    let c = e.chance;
                    if r < c {
                        if i32::from(r) < i32::from(c) / 2 {
                            self.set_unit_state(s, 4, k, None)?;
                        } else {
                            self.set_unit_state(s, 1, k, Some(at))?;
                        }
                    } else {
                        self.set_unit_state(s, 1, k, None)?;
                    }
                }
                7 => {
                    if s.moved != 0 {
                        s.entries[k].repeats = s.entries[k].repeats.wrapping_add(1);
                    }
                    if timer < 0 {
                        s.entries[k].repeats = s.entries[k].repeats.wrapping_sub(1);
                        if s.entries[k].repeats >= 0 {
                            self.set_unit_state(s, 7, k, Some(at))?;
                        } else {
                            self.set_unit_state(s, 0, k, None)?;
                            s.entries[k].repeats = 0;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn set_unit_state(
        &mut self,
        s: &mut InvisibleState,
        state: i16,
        k: usize,
        at: Option<(i32, i32)>,
    ) -> Result<(), Stop> {
        s.entries[k].state = state;
        let m = s.entries[k].monster as usize;
        let action = s.action as usize;
        let sender = self.map_actions.actions[action].identifier;
        let o = self.monsters[m].object;
        let pos = (self.objects[o].x, self.objects[o].y);
        match state {
            0 | 1 => {
                let (command, point) = match at {
                    Some(p) => (
                        MovementCommand {
                            flags: 0x10,
                            formation: 0,
                            facing: crate::trig::arctangent(
                                p.0.wrapping_sub(pos.0),
                                p.1.wrapping_sub(pos.1),
                            ),
                        },
                        p,
                    ),
                    None => {
                        let n = s.waypoints.len() as i32;
                        if n == 0 {
                            return Err(Stop::new(
                                draw_site::INVISIBLE_WALK_NO_WAYPOINTS,
                                "invi: a walk without waypoints (a division by 0)",
                            ));
                        }
                        let w = (i32::from(self.rng.draw(draw_site::INVISIBLE_WALK_WAYPOINT)) % n)
                            as usize;
                        let (mut x, mut y) = s.waypoints[w];
                        let sp = s.waypoint_spread;
                        x = x.wrapping_add(
                            i32::from(self.rng.draw(draw_site::INVISIBLE_WALK_OFFSET_X)) % (2 * sp)
                                - sp,
                        );
                        y = y.wrapping_add(
                            i32::from(self.rng.draw(draw_site::INVISIBLE_WALK_OFFSET_Y)) % (2 * sp)
                                - sp,
                        );
                        (
                            MovementCommand {
                                flags: 0,
                                formation: 0,
                                facing: 0,
                            },
                            (x, y),
                        )
                    }
                };
                self.invisible_move(action, sender, m, &command, point)?;
                let d = crate::groups::distance(point, pos) as i32;
                let speed = i32::from(
                    self.data
                        .def(b"mons", self.monsters[m].definition)?
                        .i16(0x8a),
                ) << 5;
                if speed == 0 {
                    return Err(Stop::new(
                        draw_site::INVISIBLE_NO_SPEED,
                        "invi: a subject with no speed",
                    ));
                }
                s.entries[k].timer = ((d / speed) as i8).wrapping_add(2);
            }
            2 | 5 => {
                let p = at.ok_or_else(|| {
                    Stop::new(
                        draw_site::INVISIBLE_TURN_TOWARD_NOTHING,
                        "invi: a turn toward nothing",
                    )
                })?;
                let command = MovementCommand {
                    flags: 0x10,
                    formation: 0,
                    facing: crate::trig::arctangent(
                        p.0.wrapping_sub(pos.0),
                        p.1.wrapping_sub(pos.1),
                    ),
                };
                self.invisible_move(action, sender, m, &command, pos)?;
                let r = self.rng.draw(draw_site::INVISIBLE_MOVE);
                s.entries[k].timer = ((r & 3) + 2) as i8;
            }
            3 | 4 => {
                self.monsters[m].commander = sender;
                self.general_command(3, &[m])?;
                s.entries[k].timer = 1;
            }
            6 => {
                if let Some(t) = self.monster_datum(s.monster, s.identifier) {
                    self.target_command(
                        sender,
                        crate::attack::TargetCommand {
                            flags: 0,
                            field_2: 0,
                        },
                        &[m],
                        &[t],
                    )?;
                }
            }
            7 => {
                let p = at.ok_or_else(|| {
                    Stop::new(
                        draw_site::INVISIBLE_ATTACK_NEAR_NOTHING,
                        "invi: an attack near nothing",
                    )
                })?;
                let sp = s.spread;
                if sp == 0 {
                    return Err(Stop::new(
                        draw_site::INVISIBLE_ATTACK_SPREAD_ZERO,
                        "invi: an attack spread of 0 (a division by 0)",
                    ));
                }
                let x = (i32::from(self.rng.draw(draw_site::INVISIBLE_ATTACK_X)) % sp - sp / 2)
                    .wrapping_add(p.0);
                let y = (i32::from(self.rng.draw(draw_site::INVISIBLE_ATTACK_Y)) % sp - sp / 2)
                    .wrapping_add(p.1);
                let z = self.attack_height(x, y)?;
                self.attack_location_command(sender, 0, (x, y, z), &[m]);
                let e = &mut s.entries[k];
                e.timer = 4;
                if e.repeats == 0 {
                    e.repeats = (self.rng.draw(draw_site::INVISIBLE_REPEATS) % 6) as i8;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn invisible_move(
        &mut self,
        action: usize,
        sender: i16,
        m: usize,
        command: &MovementCommand,
        point: (i32, i32),
    ) -> Result<(), Stop> {
        self.record_order(|| Order {
            site: draw_site::MOVEMENT_ORDER,
            action,
            monsters: vec![m],
            target: Some(point),
            offset: None,
            done: true,
        });
        self.execute_movement_command(sender, command, &[m], &[point])
    }
}
