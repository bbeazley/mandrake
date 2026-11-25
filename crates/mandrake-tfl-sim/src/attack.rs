use crate::draw_site;
use crate::map_actions::find;
use crate::{Stop, TflSim};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttackRecord {
    pub flags: u32,
    pub point: (i32, i32),
    pub radius: i32,
    pub monsters: Vec<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttackAi {
    pub records: Vec<AttackRecord>,
    pub used: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetCommand {
    pub flags: u16,
    pub field_2: u16,
}

impl TflSim {
    pub(crate) fn attack_with_monsters(&mut self, slots: &[usize], config: AttackRecord) {
        if slots.is_empty() {
            return;
        }
        let size = (slots.len() as i16).wrapping_mul(2).wrapping_add(0x1c) as i32;
        if self.attack_ai.used + size > 0x2000 {
            return;
        }
        self.attack_ai.used += size;
        self.attack_ai.records.insert(
            0,
            AttackRecord {
                monsters: slots.to_vec(),
                ..config
            },
        );
    }

    pub(crate) fn attack_configuration(&self, i: Option<usize>) -> AttackRecord {
        let mut c = AttackRecord {
            flags: 1,
            point: (0, 0),
            radius: 0,
            monsters: Vec::new(),
        };
        if let Some(i) = i
            && let Some(r) =
                find(&self.map_actions.actions, i, 0, b"radi", 6).filter(|p| p.count == 2)
        {
            c.radius = r.i32_at(1);
            c.flags &= !1;
        }
        c
    }

    pub(crate) fn attack_action(&mut self, i: usize) -> Result<u32, Stop> {
        crate::map_actions::not_read_by_tfl(&[b"focu"]);
        let ids = crate::map_actions::combined(&self.map_actions.actions, i, b"subj", 2, 300);
        let slots: Vec<usize> = self
            .monster_slots(&ids)
            .into_iter()
            .filter(|&s| self.monsters[s].flags & 0x800_0000 == 0)
            .collect();
        if slots.is_empty() {
            return Ok(4);
        }
        let a = &self.map_actions.actions;
        if find(a, i, 2, b"_all", 0).is_some() {
            let config = self.attack_configuration(None);
            self.attack_with_monsters(&slots, config);
            return Ok(2);
        }
        if let Some(w) = find(&self.map_actions.actions, i, 2, b"wayp", 12) {
            if w.count <= 0 {
                return Ok(0);
            }
            let r = self.rng.draw(draw_site::ATTACK_ACTION);
            let k = (i32::from(r) % i32::from(w.count)) as usize;
            let (x, y) = (w.i32_at(2 * k), w.i32_at(2 * k + 1));
            let flags = u16::from(find(&self.map_actions.actions, i, 2, b"spec", 0).is_some());
            let z = self.attack_height(x, y)?;
            let sender = self.map_actions.actions[i].identifier;
            self.attack_location_command(sender, flags, (x, y, z), &slots);
            return Ok(2);
        }
        let ids = crate::map_actions::combined(a, i, b"obje", 2, 0x1e);
        let targets = self.monster_slots(&ids);
        if targets.is_empty() {
            return Ok(0);
        }
        let mut command = TargetCommand {
            flags: 0,
            field_2: 0,
        };
        if find(&self.map_actions.actions, i, 2, b"spec", 0).is_some() {
            command.flags |= 2;
        }
        for chunk in slots.chunks(0x1e) {
            self.target_command(i as i16, command, chunk, &targets)?;
        }
        Ok(2)
    }

    pub(crate) fn update_attack_ai(&mut self) -> Result<(), Stop> {
        let groups = self.spatial_groups()?;
        crate::tc2_trace::log(|| {
            let mut out = Vec::new();
            for g in &groups {
                out.push(format!(
                    "AG {} {} {} {} {} {} : {}",
                    self.tick,
                    g.team,
                    g.centre.0,
                    g.centre.1,
                    g.radius,
                    g.cost,
                    join(&g.members)
                ));
            }
            for r in &self.attack_ai.records {
                out.push(format!(
                    "AR {} {:#x} : {}",
                    self.tick,
                    r.flags,
                    join(&r.monsters)
                ));
            }
            out.join("\n")
        });
        let mut cost = vec![0i16; groups.len()];
        let mut ratio = vec![0i32; groups.len()];
        let mut attackers: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
        let records = self.attack_ai.records.clone();
        'records: for r in &records {
            for &s in &r.monsters {
                let mons = self.data.def(b"mons", self.monsters[s].definition)?;
                let (ranged, unit_cost) = (mons.i16(0x146), mons.i16(0x182));
                let m = &self.monsters[s];
                if ranged == 0 && m.flags & 1 != 0 && m.flags & 0x100 != 0 {
                    continue;
                }
                let o = &self.objects[m.object];
                let (pos, team) = ((o.x, o.y), m.team as i8 as i16);
                let mut best: Option<(usize, i32)> = None;
                for (g, group) in groups.iter().enumerate() {
                    if group.team == team {
                        continue;
                    }
                    let mut d = 0;
                    if r.flags & 1 == 0 {
                        d = crate::groups::distance(r.point, group.centre) as i32
                            - i32::from(group.radius);
                    }
                    if d > r.radius {
                        continue;
                    }
                    let u = 0x5000
                        - (crate::groups::distance(pos, group.centre) as i32
                            - i32::from(group.radius));
                    let score = (0x18000_i32.wrapping_sub(ratio[g]) >> 4) + u.max(0);
                    if best.is_none_or(|(_, b)| b < score) {
                        best = Some((g, score));
                    }
                }
                let Some((g, _)) = best else {
                    continue;
                };
                cost[g] = cost[g].wrapping_add(unit_cost);
                ratio[g] = if groups[g].cost == 0 {
                    i32::MAX
                } else {
                    (i32::from(cost[g]) << 16) / i32::from(groups[g].cost)
                };
                if self.attack_ai.used + 8 > 0x2000 {
                    break 'records;
                }
                self.attack_ai.used += 8;
                attackers[g].insert(0, s);
            }
        }
        for (g, group) in groups.iter().enumerate() {
            if !attackers[g].is_empty() {
                self.pair_attackers(&attackers[g], &group.members)?;
            }
        }
        Ok(())
    }

    fn attack_preference(
        &mut self,
        a: mandrake_common::FourCC,
        b: mandrake_common::FourCC,
    ) -> Result<i32, Stop> {
        let ma = self.data.def(b"mons", a)?;
        let (fa, pa) = (ma.u32(0), ma.i16(0x8a));
        let mb = self.data.def(b"mons", b)?;
        let (fb, pb) = (mb.u32(0), mb.i16(0x8a));
        let bonus = if fa & 0x40 != 0 && fb & 0x80 != 0 {
            0x400
        } else {
            0
        };
        Ok((i32::from(pa) - i32::from(pb)) * 0x80 + bonus)
    }

    fn pair_attackers(&mut self, attackers: &[usize], members: &[usize]) -> Result<(), Stop> {
        crate::tc2_trace::log(|| {
            format!("AP {} {} : {}", self.tick, join(attackers), join(members))
        });
        let mut choice = vec![0usize; attackers.len()];
        for (k, &a) in attackers.iter().enumerate() {
            let (ad, ao) = (self.monsters[a].definition, self.monsters[a].object);
            let ap = (self.objects[ao].x, self.objects[ao].y);
            let mut best: Option<i32> = None;
            for &t in members {
                let to = &self.objects[self.monsters[t].object];
                let d = crate::groups::distance(ap, (to.x, to.y)) as i32;
                let td = self.monsters[t].definition;
                let v = self.attack_preference(ad, td)? + (0x5000 - d).max(0);
                if best.is_none_or(|b| b < v) {
                    choice[k] = t;
                    best = Some(v);
                }
            }
        }
        let command = TargetCommand {
            flags: 4,
            field_2: 0,
        };
        for &t in members {
            let mine: Vec<usize> = attackers
                .iter()
                .zip(&choice)
                .filter(|&(_, &c)| c == t)
                .map(|(&a, _)| a)
                .collect();
            for chunk in mine.chunks(0x1e) {
                self.target_command(-2, command, chunk, &[t])?;
            }
        }
        Ok(())
    }

    pub(crate) fn target_command(
        &mut self,
        sender: i16,
        command: TargetCommand,
        attackers: &[usize],
        targets: &[usize],
    ) -> Result<(), Stop> {
        let ids: Vec<i16> = attackers
            .iter()
            .map(|&s| self.monsters[s].identifier)
            .collect();
        let attackers = self.monster_slots(&ids);
        let ids: Vec<i16> = targets
            .iter()
            .map(|&s| self.monsters[s].identifier)
            .collect();
        for &s in &attackers {
            self.monsters[s].commander = sender;
        }
        let targets = self.monster_slots(&ids);
        let (n, m) = (attackers.len(), targets.len());
        if n == 0 || m == 0 {
            return Ok(());
        }
        if n > 32 {
            return Err(Stop::new(
                draw_site::TARGET_COMMAND,
                "a target command for more than 32 monsters",
            ));
        }
        let (centre, _) = self.monsters_centre_spread(&attackers);
        let pos = |s: &TflSim, k: usize| {
            let o = &s.objects[s.monsters[k].object];
            (o.x, o.y)
        };
        let keys: Vec<i32> = targets
            .iter()
            .map(|&t| {
                let p = pos(self, t);
                let (dx, dy) = (p.0.wrapping_sub(centre.0), p.1.wrapping_sub(centre.1));
                dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))
            })
            .collect();
        let mut order: Vec<i16> = (0..m as i16).collect();
        crate::commands::heapsort(&mut order, |a, b| {
            keys[*b as usize].wrapping_sub(keys[*a as usize])
        });
        let mut remaining = n;
        let mut used = 0u32;
        while remaining > 0 {
            let start = m.saturating_sub(remaining);
            for &e in &order[start..] {
                let t = targets[e as usize];
                let tp = pos(self, t);
                let mut best: Option<(usize, i32)> = None;
                for (k, &a) in attackers.iter().enumerate() {
                    let p = pos(self, a);
                    let (dx, dy) = (tp.0.wrapping_sub(p.0), tp.1.wrapping_sub(p.1));
                    let d = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
                    if used & (1 << k) == 0 && best.is_none_or(|(_, b)| d < b) {
                        best = Some((k, d));
                    }
                }
                let Some((k, _)) = best else {
                    return Err(Stop::new(
                        draw_site::TARGET_COMMAND_NO_ATTACKER,
                        "a target command with no attacker left",
                    ));
                };
                self.set_monster_target(attackers[k], command.flags, t);
                used |= 1 << k;
                remaining -= 1;
            }
        }
        Ok(())
    }

    pub(crate) fn set_monster_target(&mut self, s: usize, flags: u16, target: usize) {
        let m = &self.monsters[s];
        if flags & 2 != 0 && m.flags & 8 != 0 && m.flags & 0x2000_0000 != 0 {
            return;
        }
        let object = m.object;
        self.objects[object].flags &= 0xff7f_ffff;
        let m = &mut self.monsters[s];
        let kept = m.flags & 0x5f07_be40;
        m.flags = kept | 0x2d;
        if flags & 1 != 0 {
            m.flags = kept | 0x20_002d;
        }
        if flags & 2 == 0 {
            m.flags &= 0xff7f_ffff;
        } else {
            m.flags |= 0x80_0000;
        }
        if m.target != target as i16 {
            m.flags = (m.flags & 0xfffe_ffff) | 0x8000;
        }
        m.target = target as i16;
        m.waypoint_count = 0;
    }
}

fn join(slots: &[usize]) -> String {
    slots
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
