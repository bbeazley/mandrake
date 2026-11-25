use crate::Stop;
use crate::commands::MovementCommand;
use crate::draw_site;
use crate::map_actions::{combined, find, index_of};
use crate::{Order, TflSim};

pub const SUICIDE_INITIALISED: u32 = 1;
pub const SUICIDE_DUMB: u32 = 2;
const MAX_SUBJECTS: usize = 10;
const SENDER: i16 = -2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuicideEntry {
    pub monster: i16,
    pub identifier: i16,
    pub flank: i16,
    pub timer: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuicideState {
    pub flags: u32,
    pub action: i16,
    pub suin: i16,
    pub radius: i32,
    pub fedu: i32,
    pub deth: i32,
    pub mipt: i32,
    pub dith: i32,
    pub dera: i32,
    pub entries: Vec<SuicideEntry>,
}

const ROUND: i16 = 0x201;
const WALK: i16 = 0x202;
const BURST: i16 = 0x203;

impl TflSim {
    fn suicide_long(&self, i: usize, name: &[u8; 4], kind: i16) -> Option<i32> {
        find(&self.map_actions.actions, i, 0, name, kind)
            .filter(|p| p.count > 0)
            .map(|p| p.i32_at(0))
    }

    fn suicide_initial(&mut self, i: usize) -> SuicideState {
        let a = &self.map_actions.actions;
        let mut s = SuicideState {
            action: i as i16,
            radius: self.suicide_long(i, b"radi", 6).unwrap_or(0x6400),
            fedu: self.suicide_long(i, b"fedu", 5).unwrap_or(0x14),
            deth: self.suicide_long(i, b"deth", 5).unwrap_or(3),
            dith: self.suicide_long(i, b"dith", 6).unwrap_or(0xc00),
            dera: self.suicide_long(i, b"dera", 6).unwrap_or(0x1000),
            mipt: self.suicide_long(i, b"mipt", 8).unwrap_or(0x10000),
            ..SuicideState::default()
        };
        let suin = find(a, i, 0, b"suin", 3)
            .filter(|p| p.count > 0)
            .map_or(-1, |p| p.i16_at(0));
        s.suin = index_of(a, suin).map_or(-1, |j| j as i16);
        let dela = self.suicide_long(i, b"dela", 5).unwrap_or(0);
        if find(a, i, 2, b"dumb", 0).is_some() {
            s.flags |= SUICIDE_DUMB;
        }
        for id in combined(a, i, b"subj", 2, MAX_SUBJECTS) {
            let timer = if dela > 0 {
                (u32::from(self.rng.draw(draw_site::SUICIDE_DELAY)).wrapping_mul(dela as u32) >> 16)
                    as i16
            } else {
                0
            };
            s.entries.push(SuicideEntry {
                monster: -1,
                identifier: id,
                flank: 0,
                timer,
            });
        }
        s
    }

    pub(crate) fn suicide_action(&mut self, i: usize) -> Result<u32, Stop> {
        let mut s = match self.suicides.get(&i) {
            Some(s) => s.clone(),
            None => self.suicide_initial(i),
        };
        let team = self.suicide_variables(&mut s)?;
        if s.entries.is_empty() {
            return Ok(4);
        }
        self.suicide_state(&mut s, team)?;
        s.flags |= SUICIDE_INITIALISED;
        self.suicides.insert(i, s);
        self.set_param(i, b"#glo", 1, 0x74, vec![0; 0x74]);
        Ok(0)
    }

    fn suicide_variables(&mut self, s: &mut SuicideState) -> Result<i16, Stop> {
        let mut team: i16 = -1;
        let mut k = 0;
        while k < s.entries.len() {
            let e = s.entries[k];
            if e.timer != 0 {
                k += 1;
                continue;
            }
            let mut monster = e.monster;
            if monster == -1 {
                monster = match self.make_monster_visible_new(e.identifier)? {
                    Some((m, _)) => m as i16,
                    None => -1,
                };
                s.entries[k].monster = monster;
            }
            if monster != -1 {
                if let Some(m) = self
                    .monsters
                    .get(monster as usize)
                    .filter(|m| m.identifier != 0 && m.identifier == e.identifier)
                {
                    team = m.team as i8 as i16;
                    k += 1;
                    continue;
                }
                let ids = find(&self.map_actions.actions, s.action as usize, 0, b"casu", 3)
                    .map(|p| p.shorts())
                    .unwrap_or_default();
                for &id in ids.iter().take(30) {
                    if let Some(j) = index_of(&self.map_actions.actions, id) {
                        self.activate(j)?;
                    }
                }
            }
            s.entries.remove(k);
        }
        Ok(team)
    }

    fn suicide_enemies_near(&self, p: (i32, i32), team: i16) -> i16 {
        let (w, h) = (self.width(), self.height_cells());
        let clamp = |v: i32, size: i32| if v < 0 { 0 } else { v.min(size - 3) };
        let x0 = clamp((p.0 >> 10) - 1, w);
        let y0 = clamp((p.1 >> 10) - 1, h);
        let mut n: i16 = 0;
        for row in 0..3 {
            for col in 0..3 {
                let mut k = self.cell_heads[self.cell_at(x0 + col, y0 + row)];
                while k >= 0 {
                    let o = &self.objects[k as usize];
                    if o.flags & 0x200 == 0
                        && o.kind == 2
                        && self.monsters[o.owner as usize].team as i8 as i16 != team
                    {
                        n = n.wrapping_add(1);
                    }
                    k = o.next_in_cell;
                }
            }
        }
        n
    }

    fn suicide_state(&mut self, s: &mut SuicideState, team: i16) -> Result<(), Stop> {
        let running = s.suin == -1
            || self
                .map_actions
                .actions
                .get(s.suin as usize)
                .is_some_and(|a| a.flags & 0x20 != 0);
        let supergroups = self.spatial_supergroups()?;
        let groups = self.spatial_groups()?;
        let mut target: Option<(i32, i32)> = None;
        let mut reach: Option<i32> = None;
        for k in 0..s.entries.len() {
            let e = s.entries[k];
            if e.timer != 0 {
                if running {
                    s.entries[k].timer -= 1;
                }
                continue;
            }
            let slot = e.monster as usize;
            let object = self.monsters[slot].object;
            let p = (self.objects[object].x, self.objects[object].y);
            let mut enemy: Option<(usize, i32)> = None;
            let mut ally: Option<(usize, i32)> = None;
            for (g, sg) in supergroups.iter().enumerate() {
                let dx = p.0.wrapping_sub(sg.centre.0).wrapping_abs();
                let dy = p.1.wrapping_sub(sg.centre.1).wrapping_abs();
                let d = if dy < dx {
                    (dy >> 1).wrapping_add(dx)
                } else {
                    (dx >> 1).wrapping_add(dy)
                }
                .wrapping_sub(i32::from(sg.radius));
                if sg.team == team {
                    if (dx != 0 || dy != 0) && ally.is_none_or(|(_, ad)| d < ad) {
                        ally = Some((g, d));
                    }
                } else {
                    let keep = enemy.is_some_and(|(b, bd)| {
                        let b = &supergroups[b];
                        bd <= d
                            && (i32::from(sg.strength) <= s.mipt >> 8
                                || s.mipt <= i32::from(b.strength))
                            && (sg.strength <= b.strength || bd <= d.wrapping_sub(s.dith))
                    });
                    if !keep {
                        enemy = Some((g, d));
                    }
                }
            }
            let mut near: Option<(usize, i32)> = None;
            for (g, gr) in groups.iter().enumerate() {
                if gr.team == team || self.data_def_i16(gr.definition, 0x146) != 1 {
                    continue;
                }
                let d = (crate::groups::distance(p, gr.centre) as i32)
                    .wrapping_sub(i32::from(gr.spacing))
                    .wrapping_sub(i32::from(gr.radius));
                if d < 0x4000 && near.is_none_or(|(_, bd)| d < bd) {
                    near = Some((g, d));
                }
            }
            let mut mode: i16 = -1;
            let mut check_coming = false;
            if s.flags & SUICIDE_DUMB == 0 || enemy.is_none() {
                if let Some((a, ad)) = ally
                    && ad < s.dera + 0x400
                    && enemy.is_none_or(|(_, ed)| ad < ed)
                {
                    let a = &supergroups[a];
                    target = Some(a.centre);
                    reach = Some(i32::from(a.radius) + 0x400 + s.dera);
                    mode = ROUND;
                    check_coming = true;
                } else if let Some((eg, ed)) = enemy {
                    let en = &supergroups[eg];
                    let mut rounded = false;
                    if let Some((g, gd)) = near
                        && gd < 0
                        && s.dera < i32::from(en.radius) + ed
                    {
                        let gr = &groups[g];
                        let t = crate::trig::trig();
                        let ix = crate::trig::index(i32::from(gr.facing));
                        let dot = p
                            .1
                            .wrapping_sub(gr.centre.1)
                            .wrapping_mul(i32::from(t.sine[ix]))
                            .wrapping_add(
                                i32::from(t.cosine[ix]).wrapping_mul(p.0.wrapping_sub(gr.centre.0)),
                            );
                        if dot > 0 {
                            s.entries[k].flank = s.fedu as i16;
                        }
                        if s.entries[k].flank != 0 {
                            target = Some(gr.centre);
                            reach = Some(i32::from(gr.spacing) + i32::from(gr.radius));
                            mode = ROUND;
                            rounded = true;
                        }
                    }
                    if !rounded {
                        target = Some(en.centre);
                        mode = WALK;
                    }
                    check_coming = true;
                }
            } else if let Some((eg, _)) = enemy {
                target = Some(supergroups[eg].centre);
                mode = WALK;
                check_coming = true;
            }
            if check_coming && let Some((eg, ed)) = enemy {
                let en = &supergroups[eg];
                let dot = en
                    .centre
                    .1
                    .wrapping_sub(p.1)
                    .wrapping_mul(i32::from(en.velocity.1))
                    .wrapping_add(
                        en.centre
                            .0
                            .wrapping_sub(p.0)
                            .wrapping_mul(i32::from(en.velocity.0)),
                    );
                if dot > 0 && i32::from(en.radius) + ed < s.dera - 0x200 {
                    mode = BURST;
                }
            }
            let n = self.suicide_enemies_near(p, team);
            let few =
                i32::from(n) < s.deth && enemy.is_none_or(|(eg, _)| n < supergroups[eg].members);
            if !few {
                mode = BURST;
            }
            if mode == BURST {
                let o = &self.objects[object];
                if o.flags as u8 & 0x80 != 0
                    || (o.flags & 0x40 != 0 && (i32::from(o.top) >> 1) + o.z < 0)
                {
                    mode = WALK;
                }
            }
            match mode {
                ROUND | WALK => {
                    let Some(mut t) = target else {
                        return Err(Stop::new(
                            draw_site::SUICIDE_TARGET,
                            format!("suic: monster {slot} walks to a target no subject has set"),
                        ));
                    };
                    if mode == ROUND {
                        let r = reach.ok_or_else(|| {
                            Stop::new(
                                draw_site::SUICIDE_NO_REACH,
                                format!("suic: monster {slot} without a reach"),
                            )
                        })?;
                        if r != 0 {
                            let div = r.wrapping_mul(0x648) >> 8;
                            if div == 0 {
                                return Err(Stop::new(
                                    draw_site::SUICIDE_DIVISION_BY_ZERO,
                                    "suic: a division by zero",
                                ));
                            }
                            let mut off = (0x800_0000i64 / i64::from(div)) as i32;
                            if off > 0x1000 {
                                off = 0x1000;
                            }
                            if e.monster & 1 != 0 {
                                off += 0x8000;
                            }
                            let a = i32::from(crate::trig::arctangent(
                                p.0.wrapping_sub(t.0),
                                p.1.wrapping_sub(t.1),
                            ));
                            let tr = crate::trig::trig();
                            let ix = crate::trig::index(off.wrapping_add(a));
                            t = (
                                t.0.wrapping_add(i32::from(tr.cosine[ix]).wrapping_mul(r) >> 14),
                                t.1.wrapping_add(i32::from(tr.sine[ix]).wrapping_mul(r) >> 14),
                            );
                            target = Some(t);
                        }
                    }
                    self.record_order(|| Order {
                        site: draw_site::SUICIDE_TARGET,
                        action: s.action as usize,
                        monsters: vec![slot],
                        target: Some(t),
                        offset: None,
                        done: true,
                    });
                    let command = MovementCommand {
                        flags: 0,
                        formation: 0,
                        facing: 0,
                    };
                    self.execute_movement_command(SENDER, &command, &[slot], &[t])?;
                }
                BURST => {
                    self.monsters[slot].commander = SENDER;
                    self.general_command(3, &[slot])?;
                }
                _ => {}
            }
            if s.entries[k].flank != 0 {
                s.entries[k].flank -= 1;
            }
        }
        Ok(())
    }
}
