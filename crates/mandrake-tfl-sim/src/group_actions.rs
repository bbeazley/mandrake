use mandrake_common::FourCC;

use crate::draw_site;
use crate::map_actions::{MapAction, Parameter, combined, find, index_of};
use crate::objects::DatumPool;
use crate::{Stop, TflSim};

pub const MAX_MAP_ACTIONS: usize = 0x400;

pub fn map_action_pool(actions: &[MapAction]) -> DatumPool {
    let mut pool = DatumPool::new(MAX_MAP_ACTIONS);
    let mut ids: Vec<i16> = Vec::with_capacity(actions.len());
    for a in actions {
        let (k, _) = pool
            .allocate(|k| k < ids.len())
            .expect("a mesh has fewer than 0x400 map actions");
        ids.push(0);
        let id = pool.set_identifier(k, a.identifier, &ids).unwrap_or(0);
        ids[k] = id;
    }
    pool
}

fn be_i32(v: i32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

impl TflSim {
    fn action_pool(&mut self) -> &mut DatumPool {
        let actions = &self.map_actions.actions;
        self.map_actions
            .pool
            .get_or_insert_with(|| map_action_pool(actions))
    }

    pub(crate) fn new_map_action(&mut self) -> Option<usize> {
        let used: Vec<bool> = self
            .map_actions
            .actions
            .iter()
            .map(|a| a.identifier != 0)
            .collect();
        let (k, id) = self
            .action_pool()
            .allocate(|k| used.get(k).copied().unwrap_or(false))?;
        let a = MapAction {
            identifier: id,
            kind: 0,
            tag: 0xffff_ffff,
            flags: 0,
            delay: 0,
            delay_range: 0,
            time: 0,
            parameters: Vec::new(),
        };
        let list = &mut self.map_actions.actions;
        if k < list.len() {
            list[k] = a;
        } else {
            list.push(a);
        }
        Some(k)
    }

    pub(crate) fn delete_map_action(&mut self, k: usize) {
        if let Some(a) = self.map_actions.actions.get_mut(k) {
            a.identifier = 0;
            a.parameters.clear();
        }
        self.meanders.remove(&k);
        self.legions.remove(&k);
        let used: Vec<bool> = self
            .map_actions
            .actions
            .iter()
            .map(|a| a.identifier != 0)
            .collect();
        self.action_pool()
            .free(k, |j| used.get(j).copied().unwrap_or(false));
    }

    fn first_i32(&self, action: usize, name: &[u8; 4], kind: i16) -> Option<i32> {
        find(&self.map_actions.actions, action, 0, name, kind)
            .filter(|p| p.count > 0)
            .map(|p| p.i32_at(0))
    }

    pub(crate) fn group_action(&mut self, i: usize) -> Result<u32, Stop> {
        self.set_param(i, b"subs", 5, 1, be_i32(0));
        let Some(stat) = find(&self.map_actions.actions, i, 0, b"stat", 3).map(|p| p.shorts())
        else {
            return Ok(0);
        };
        for &id in &stat {
            if self.first_i32(i, b"subs", 5) != Some(0) {
                continue;
            }
            if let Some(j) = index_of(&self.map_actions.actions, id) {
                let owner = self.map_actions.actions[i].identifier;
                self.set_param(j, b"owgr", 3, 1, owner.to_be_bytes().to_vec());
                self.trigger(j)?;
            }
        }
        Ok(0)
    }

    pub(crate) fn set_group_action_state(&mut self, group: usize, states: &[i16]) {
        if self.map_actions.actions[group].tag != u32::from_be_bytes(*b"grou") {
            return;
        }
        for &id in states {
            if let Some(j) = index_of(&self.map_actions.actions, id) {
                self.set_param(j, b"subs", 5, 1, be_i32(-1));
            }
        }
        let data = states.iter().flat_map(|s| s.to_be_bytes()).collect();
        self.set_param(group, b"stat", 3, states.len() as i16, data);
        self.set_param(group, b"subs", 5, 1, be_i32(-1));
    }

    pub(crate) fn split_up_group_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let Some(owner) = find(a, i, 0, b"owgr", 3)
            .filter(|p| p.count > 0)
            .and_then(|p| index_of(a, p.i16_at(0)))
        else {
            return Ok(0);
        };
        let ids = combined(a, owner, b"subj", 2, 30);
        if ids.is_empty() {
            return Ok(0);
        }
        let (Some(thre), Some(setu)) = (
            find(a, i, 2, b"thre", 5).cloned(),
            find(a, i, 0, b"setu", 7).cloned(),
        ) else {
            return Ok(0);
        };
        if setu.count != thre.count + 1 {
            return Ok(0);
        }
        let mut subjects = self.monster_identifiers_to_slots(&ids);
        let n = subjects.len() as i32;
        let mut k = 0;
        while k < thre.count.max(0) as usize && n > thre.i32_at(k) {
            k += 1;
        }
        let name = (setu.i32_at(k) as u32).to_be_bytes();
        let Some(templates) = find(&self.map_actions.actions, i, 0, &name, 3).map(|p| p.shorts())
        else {
            return Ok(0);
        };
        if let Some(old) = find(&self.map_actions.actions, i, 3, b"chil", 3).map(|p| p.shorts()) {
            for id in old.into_iter().take(30) {
                if let Some(j) = index_of(&self.map_actions.actions, id) {
                    self.delete_map_action(j);
                }
            }
            let ps = &mut self.map_actions.actions[i].parameters;
            if let Some(p) = ps.iter().position(|p| p.name.0 == *b"chil" && p.kind == 3) {
                ps.remove(p);
            }
        }
        self.map_actions.actions[i].parameters.push(Parameter {
            kind: 3,
            count: 0,
            name: FourCC(*b"chil"),
            data: Vec::new(),
        });
        let mut children = Vec::new();
        for id in templates {
            let Some(t) = index_of(&self.map_actions.actions, id) else {
                continue;
            };
            if let Some(g) = self.build_subgroup(t, &mut subjects)? {
                let gid = self.map_actions.actions[g].identifier;
                children.push(gid);
                let ps = &mut self.map_actions.actions[i].parameters;
                if let Some(p) = ps
                    .iter_mut()
                    .rev()
                    .find(|p| p.name.0 == *b"chil" && p.kind == 3)
                {
                    p.count += 1;
                    p.data.extend(gid.to_be_bytes());
                }
            }
        }
        self.set_group_action_state(owner, &children);
        Ok(0)
    }

    pub(crate) fn monster_identifiers_to_slots(&self, ids: &[i16]) -> Vec<i16> {
        ids.iter()
            .filter_map(|&id| {
                self.monsters
                    .iter()
                    .position(|m| m.identifier != 0 && m.identifier == id)
                    .map(|k| k as i16)
            })
            .collect()
    }

    fn build_subgroup(
        &mut self,
        template: usize,
        subjects: &mut [i16],
    ) -> Result<Option<usize>, Stop> {
        let Some(g) = self.new_map_action() else {
            return Ok(None);
        };
        let deci = find(&self.map_actions.actions, template, 0, b"deci", 12)
            .filter(|p| p.count > 0)
            .map(|p| (p.i32_at(0), p.i32_at(1)));
        let distances: Vec<u32> = subjects
            .iter()
            .map(|&s| match (s, deci) {
                (-1, Some(_)) => 0x7fff_ffff,
                (_, None) => 0,
                (s, Some(p)) => {
                    let o = &self.objects[self.monsters[s as usize].object];
                    crate::groups::distance((o.x, o.y), p)
                }
            })
            .collect();
        self.map_actions.actions[g].tag = u32::from_be_bytes(*b"grou");
        self.map_actions.actions[g].parameters.push(Parameter {
            kind: 2,
            count: 0,
            name: FourCC(*b"subj"),
            data: Vec::new(),
        });
        let left = subjects.iter().filter(|&&s| s != -1).count() as i32;
        let a = &self.map_actions.actions;
        let mut take = match find(a, template, 0, b"coun", 5).filter(|p| p.count > 0) {
            Some(p) => p.i32_at(0),
            None => match find(a, template, 0, b"coun", 8).filter(|p| p.count > 0) {
                Some(p) => (p.i32_at(0).wrapping_mul(left)) >> 16,
                None => left,
            },
        };
        take = take.min(left);
        while take != 0 {
            let mut best: Option<usize> = None;
            for (k, &s) in subjects.iter().enumerate() {
                if s != -1 && best.is_none_or(|b| distances[k] < distances[b]) {
                    best = Some(k);
                }
            }
            let Some(b) = best else {
                return Err(Stop::new(
                    draw_site::BUILD_SUBGROUP_EMPTY,
                    "build_subgroup: no subject left",
                ));
            };
            let slot = subjects[b] as usize;
            subjects[b] = -1;
            let id = self.monsters[slot].identifier;
            if id != -1 {
                let p = self.map_actions.actions[g]
                    .parameters
                    .iter_mut()
                    .rev()
                    .find(|p| p.name.0 == *b"subj" && p.kind == 2)
                    .expect("the subgroup's subj");
                p.count += 1;
                p.data.extend(id.to_be_bytes());
            }
            take -= 1;
        }
        if let Some(init) =
            find(&self.map_actions.actions, template, 0, b"init", 3).map(|p| p.shorts())
        {
            self.set_group_action_state(g, &init);
        }
        Ok(Some(g))
    }

    pub(crate) fn guard_group_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let Some(owner) = find(a, i, 0, b"owgr", 3)
            .filter(|p| p.count > 0)
            .and_then(|p| index_of(a, p.i16_at(0)))
        else {
            return Ok(0);
        };
        let Some(point) = find(a, i, 0, b"poin", 12)
            .filter(|p| p.count > 0)
            .map(|p| (p.i32_at(0), p.i32_at(1)))
        else {
            return Ok(0);
        };
        let ids = combined(a, owner, b"subj", 2, 30);
        let slots: Vec<usize> = self
            .monster_identifiers_to_slots(&ids)
            .into_iter()
            .map(|s| s as usize)
            .collect();
        let subs = self.first_i32(i, b"subs", 5).unwrap_or(-1);
        let engaged = self.test_guard_radius(i, &slots, point, subs == 1)?;
        let mut state = i32::from(engaged);
        if state != subs {
            if state != 1 && slots.len() != ids.len() {
                let a = &self.map_actions.actions;
                if let Some(r) = find(a, i, 0, b"rese", 3)
                    .filter(|p| p.count > 0)
                    .and_then(|p| index_of(a, p.i16_at(0)))
                {
                    self.trigger(r)?;
                    return Ok(0);
                }
            }
        } else if state != 1 {
            return Ok(0);
        }
        if slots.is_empty() {
            return Ok(0);
        }
        let a = &self.map_actions.actions;
        let acti = find(a, i, 0, b"acti", 3).filter(|p| p.count == 2).cloned();
        if let Some(acti) = acti {
            let mut next = index_of(a, acti.i16_at(state as usize));
            if state == 1
                && let Some(inhi) = find(a, i, 0, b"inhi", 3).map(|p| p.shorts())
                && self.succeeded_any(&inhi)
            {
                state = 0;
                next = index_of(&self.map_actions.actions, acti.i16_at(0));
            }
            if let Some(n) = next {
                if state == 1 {
                    let mut config = self.attack_configuration(Some(n));
                    config.point = point;
                    self.attack_with_monsters(&slots, config);
                } else {
                    let data = ids.iter().flat_map(|s| s.to_be_bytes()).collect();
                    self.set_param(n, b"subj", 2, ids.len() as i16, data);
                    self.trigger(n)?;
                }
            }
        }
        self.set_param(i, b"subs", 5, 1, be_i32(state));
        Ok(0)
    }

    fn test_guard_radius(
        &mut self,
        i: usize,
        slots: &[usize],
        point: (i32, i32),
        engaged: bool,
    ) -> Result<bool, Stop> {
        if slots.is_empty() {
            return Ok(false);
        }
        let team = self.monsters[slots[0]].team as i8 as i16;
        let Some(radi) = find(&self.map_actions.actions, i, 0, b"radi", 6).filter(|p| p.count == 2)
        else {
            return Ok(false);
        };
        let (near, far) = (radi.i32_at(0), radi.i32_at(1));
        let groups = self.spatial_groups()?;
        let mut best: Option<i32> = None;
        for g in groups.iter().filter(|g| g.team != team) {
            let d = crate::groups::distance(point, g.centre) as i32 - i32::from(g.radius);
            if best.is_none_or(|b| d < b) {
                best = Some(d);
            }
        }
        let best = best.unwrap_or(i32::MAX);
        if best >= far {
            return Ok(false);
        }
        let d = best - near;
        if d < 0 {
            return Ok(true);
        }
        if d >= 0x4000 {
            return Ok(false);
        }
        if engaged {
            return Ok(true);
        }
        let span = far.wrapping_sub(near);
        if span == 0 {
            return Err(Stop::new(
                draw_site::GUARD_RADIUS_CHANCE,
                "guard radius check with empty span",
            ));
        }
        let chance = 0x4000 - (d << 14) / span;
        Ok(i32::from(self.rng.draw(draw_site::GUARD_RADIUS_CHANCE)) < chance)
    }

    pub(crate) fn wander_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let ids = combined(a, i, b"subj", 2, 30);
        let slots = self.monster_identifiers_to_slots(&ids);
        if slots.is_empty() {
            return Ok(0);
        }
        let a = &self.map_actions.actions;
        let (Some(centre), Some(radius)) = (
            find(a, i, 0, b"cent", 12)
                .filter(|p| p.count > 0)
                .map(|p| (p.i32_at(0), p.i32_at(1))),
            self.first_i32(i, b"radi", 6),
        ) else {
            return Ok(0);
        };
        let id = self.map_actions.actions[i].identifier;
        for &s in &slots {
            let angle = i32::from(self.rng.draw(draw_site::WANDER_ANGLE));
            let r = i64::from(self.rng.draw(draw_site::WANDER_DISTANCE));
            if radius == 0 {
                return Err(Stop::new(
                    draw_site::WANDER_DISTANCE,
                    "wander radius 0 (a division by 0)",
                ));
            }
            let d = (r % i64::from(radius)) as i32;
            let t = crate::trig::trig();
            let k = crate::trig::index(angle);
            let dest = (
                ((i32::from(t.cosine[k]) * d) >> 14) + centre.0,
                ((i32::from(t.sine[k]) * d) >> 14) + centre.1,
            );
            let command = crate::commands::MovementCommand {
                flags: 0,
                formation: 0,
                facing: 0,
            };
            self.record_order(|| crate::Order {
                site: draw_site::WANDER_ORDER,
                action: i,
                monsters: vec![s as usize],
                target: Some(dest),
                offset: None,
                done: true,
            });
            self.execute_movement_command(id, &command, &[s as usize], &[dest])?;
        }
        Ok(2)
    }

    pub(crate) fn general_action(&mut self, i: usize) -> Result<u32, Stop> {
        let ids = combined(&self.map_actions.actions, i, b"subj", 2, 30);
        let slots: Vec<usize> = self
            .monster_identifiers_to_slots(&ids)
            .into_iter()
            .map(|s| s as usize)
            .collect();
        if slots.is_empty() {
            return Ok(0);
        }
        let Some(kind) = self.first_i32(i, b"type", 5) else {
            return Ok(0);
        };
        let id = self.map_actions.actions[i].identifier;
        for &s in &slots {
            self.monsters[s].commander = id;
        }
        self.general_command(kind as u16, &slots)?;
        Ok(2)
    }

    pub(crate) fn rout_action(&mut self, i: usize) -> Result<u32, Stop> {
        let ids = combined(&self.map_actions.actions, i, b"subj", 2, 300);
        let slots: Vec<usize> = self
            .monster_slots(&ids)
            .into_iter()
            .filter(|&s| self.monsters[s].flags & 0x800_0000 == 0)
            .collect();
        if slots.is_empty() {
            return Ok(6);
        }
        let points: Vec<(i32, i32)> = {
            let Some(dest) = find(&self.map_actions.actions, i, 0, b"dest", 12) else {
                return Ok(0);
            };
            (0..dest.count.max(0) as usize)
                .map(|k| (dest.i32_at(2 * k), dest.i32_at(2 * k + 1)))
                .collect()
        };
        if points.is_empty() {
            return Ok(0);
        }
        for s in slots {
            let o = &self.objects[self.monsters[s].object];
            let pos = (o.x, o.y);
            let mut best: Option<(usize, u32)> = None;
            for (k, &p) in points.iter().enumerate() {
                let d = crate::groups::distance(pos, p);
                if best.is_none_or(|(_, b)| d < b) {
                    best = Some((k, d));
                }
            }
            let p = points[best.expect("a point").0];
            let command = crate::commands::MovementCommand {
                flags: 0,
                formation: 0,
                facing: 0,
            };
            self.record_order(|| crate::Order {
                site: draw_site::ROUT_ORDER,
                action: i,
                monsters: vec![s],
                target: Some(p),
                offset: None,
                done: true,
            });
            self.execute_movement_command(i as i16, &command, &[s], &[p])?;
        }
        Ok(0)
    }

    fn activate_list(&mut self, i: usize, name: &[u8; 4]) -> Result<(), Stop> {
        let ids = combined(&self.map_actions.actions, i, name, 3, 5);
        for id in ids {
            if let Some(j) = index_of(&self.map_actions.actions, id) {
                self.activate(j)?;
            }
        }
        Ok(())
    }

    pub(crate) fn lead_action(&mut self, i: usize) -> Result<u32, Stop> {
        let a = &self.map_actions.actions;
        let ids = combined(a, i, b"subj", 2, 300);
        let subjects = self.monster_identifiers_to_slots(&ids);
        let a = &self.map_actions.actions;
        let i32p = |name: &[u8; 4], kind: i16| {
            find(a, i, 0, name, kind)
                .filter(|p| p.count > 0)
                .map(|p| p.i32_at(0))
        };
        let status: i16;
        let mut leader = -1i16;
        let (mut centre, mut nearest) = ((0i32, 0i32), (0i32, 0i32));
        let (mira, dera, mara) = (i32p(b"mira", 6), i32p(b"dera", 6), i32p(b"mara", 6));
        let kind = i32p(b"type", 5);
        let lead = find(a, i, 0, b"lead", 2)
            .filter(|p| p.count > 0)
            .map(|p| p.i16_at(0));
        if subjects.is_empty() {
            status = 1;
        } else if let (Some(l), Some(mi), Some(de), Some(ma), Some(_)) =
            (lead, mira, dera, mara, kind)
        {
            if de < mi || ma < de {
                status = 0;
            } else {
                match self.monster_identifiers_to_slots(&[l]).first() {
                    None => status = 2,
                    Some(&ls) => {
                        leader = ls;
                        status = -1;
                        let lo = &self.objects[self.monsters[ls as usize].object];
                        let lp = (lo.x, lo.y);
                        let n = subjects.len() as i32;
                        let mut best: Option<u32> = None;
                        for &s in &subjects {
                            let o = &self.objects[self.monsters[s as usize].object];
                            let d = crate::groups::distance((o.x, o.y), lp);
                            if best.is_none_or(|b| (d as i32) < b as i32) {
                                nearest = (o.x, o.y);
                                best = Some(d);
                            }
                            centre.0 += o.x / n;
                            centre.1 += o.y / n;
                        }
                    }
                }
            }
        } else {
            status = 0;
        }
        if status != -1 {
            return self.lead_result(i, status);
        }
        let (mira, dera, mara) = (mira.unwrap(), dera.unwrap(), mara.unwrap());
        let kind = kind.unwrap();
        let a = &self.map_actions.actions;
        let i32p = |name: &[u8; 4], kind: i16| {
            find(a, i, 0, name, kind)
                .filter(|p| p.count > 0)
                .map(|p| p.i32_at(0))
        };
        let pair = |name: &[u8; 4]| {
            find(a, i, 0, name, 5)
                .filter(|p| p.count == 2)
                .map_or((0x78, 0), |p| (p.i32_at(0), p.i32_at(1)))
        };
        let rata = i32p(b"rata", 6).unwrap_or(0x800);
        let deby = pair(b"deby");
        let debe = pair(b"debe");
        let mrbf = i32p(b"mrbf", 6).unwrap_or(0x1800);
        let flra = i32p(b"flra", 6).unwrap_or(0x800);
        let yect = i32p(b"yect", 5).unwrap_or(-1);
        let bect = i32p(b"bect", 5).unwrap_or(-1);
        if kind != 0 {
            return Err(Stop::new(
                draw_site::LEAD_TYPE,
                format!("lead of type {kind}"),
            ));
        }
        let Some(wayp) = find(a, i, 0, b"wayp", 12).cloned() else {
            return self.lead_result(i, 0);
        };
        if wayp.count >= 0x10 {
            return self.lead_result(i, 0);
        }
        let points: Vec<(i32, i32)> = (0..wayp.count.max(0) as usize)
            .map(|k| (wayp.i32_at(2 * k), wayp.i32_at(2 * k + 1)))
            .collect();
        let mut stt = i32p(b"#stt", 5).unwrap_or(1);
        let mut tye = i32p(b"#tye", 5).unwrap_or(0);
        let mut yel = i32p(b"#yel", 5).unwrap_or(yect);
        let mut tbe = i32p(b"#tbe", 5).unwrap_or(0);
        let mut bec = i32p(b"#bec", 5).unwrap_or(bect);
        let mut way = i32p(b"#way", 5).unwrap_or(0);
        let mut status = -1i16;
        let id = self.map_actions.actions[i].identifier;
        let tick = self.tick as i32;
        let lo = &self.objects[self.monsters[leader as usize].object];
        let lp = (lo.x, lo.y);
        let wp = points.get(way.max(0) as usize).copied().unwrap_or((0, 0));
        let command = crate::commands::MovementCommand {
            flags: 0,
            formation: 0,
            facing: 0,
        };
        if (crate::groups::distance(lp, wp) as i32) < rata {
            way += 1;
            if way >= points.len() as i32 {
                status = 4;
            }
        } else {
            let mut st = stt;
            let mut changed = false;
            if self.map_actions.actions[i].flags & 0x20 != 0 {
                st = 0;
            }
            let d = crate::groups::distance(nearest, lp) as i32;
            if d < mira || mara < d {
                st = 1;
                let mut best: Option<(i32, u32)> = None;
                for k in way.max(0)..points.len() as i32 {
                    let d = crate::groups::distance(nearest, points[k as usize]);
                    if best.is_none_or(|(_, b)| (d as i32) < b as i32) {
                        best = Some((k, d));
                    }
                }
                if let Some((k, _)) = best {
                    way = k;
                }
            }
            if st != stt {
                changed = true;
                stt = st;
            }
            let toward = |c: (i32, i32), p: (i32, i32), r: i32| {
                let (dx, dy) = (p.0 - c.0, p.1 - c.1);
                let n = crate::groups::isqrt(
                    dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32
                ) as i32;
                if n != 0 {
                    ((dx * r) / n + c.0, (dy * r) / n + c.1)
                } else {
                    p
                }
            };
            let target = match stt {
                0 => lp,
                1 => toward(nearest, wp, dera),
                _ => (0, 0),
            };
            let lteam = self.monsters[leader as usize].team as i8;
            let mut enemy: Option<(usize, u32)> = None;
            for (k, m) in self.monsters.iter().enumerate() {
                if m.identifier == 0
                    || m.flags & 0x4000_0000 != 0
                    || m.team as i8 == lteam
                    || m.team as i8 == -1
                {
                    continue;
                }
                let o = &self.objects[m.object];
                let d = crate::groups::distance(target, (o.x, o.y));
                if (d as i32) < enemy.map_or(i32::MAX, |e| e.1 as i32) {
                    enemy = Some((k, d));
                }
            }
            match enemy {
                Some((e, d)) if (d as i32) < mrbf => {
                    let eo = &self.objects[self.monsters[e].object];
                    let p = toward(centre, (eo.x, eo.y), -flra);
                    self.execute_movement_command(id, &command, &[leader as usize], &[p])?;
                    if tye < tick && (yel == -1 || 0 < yel) {
                        self.activate_list(i, b"flee")?;
                        let r = if deby.1 < 1 {
                            0
                        } else {
                            (i32::from(self.rng.draw(draw_site::LEAD_DELAY_BY)) * deby.1) >> 16
                        };
                        tye = tick + deby.0 + r;
                        if yel > 0 {
                            yel -= 1;
                        }
                    }
                }
                _ => {
                    if changed {
                        yel = yect;
                        bec = bect;
                    }
                    if stt == 0 {
                        if tbe < tick && (bec == -1 || 0 < bec) {
                            self.activate_list(i, b"beck")?;
                            let r = if debe.1 < 1 {
                                0
                            } else {
                                (i32::from(self.rng.draw(draw_site::LEAD_DELAY_BETWEEN)) * debe.1)
                                    >> 16
                            };
                            tbe = tick + debe.0 + r;
                            if bec > 0 {
                                bec -= 1;
                            }
                        }
                    } else if stt == 1 {
                        self.execute_movement_command(id, &command, &[leader as usize], &[target])?;
                    }
                }
            }
        }
        for (name, v) in [
            (b"#stt", stt),
            (b"#tye", tye),
            (b"#yel", yel),
            (b"#tbe", tbe),
            (b"#bec", bec),
            (b"#way", way),
        ] {
            self.set_param(i, name, 5, 1, be_i32(v));
        }
        self.lead_result(i, status)
    }

    fn lead_result(&mut self, i: usize, status: i16) -> Result<u32, Stop> {
        match status {
            -1 => Ok(0),
            2 => {
                self.activate_list(i, b"died")?;
                Ok(4)
            }
            3 => {
                self.activate_list(i, b"gone")?;
                Ok(4)
            }
            4 => {
                self.activate_list(i, b"succ")?;
                Ok(4)
            }
            _ => Ok(4),
        }
    }
}
