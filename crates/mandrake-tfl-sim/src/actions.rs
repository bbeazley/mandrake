use crate::commands::MovementCommand;
use crate::draw_site;
use crate::map_actions::{
    FLAG_ACTIVE, FLAG_NO_ACTIVATION_DELAY, FLAG_NO_RESCHEDULE_DELAY, FLAG_SUCCEEDED, Found,
    MapAction, Parameter, TYPE_ACTIONS, Walk, combined, find, index_of, not_read_by_tfl,
};
use crate::{Stop, TflSim};

fn element<'a>(a: &'a [MapAction], i: usize, name: &[u8; 4], kind: i16) -> Option<&'a Parameter> {
    find(a, i, 0, name, kind).filter(|p| p.count > 0)
}

type Leg = ((i32, i32), (i32, i32));

const fn tag(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    pub site: u32,
    pub action: usize,
    pub monsters: Vec<usize>,
    pub target: Option<(i32, i32)>,
    pub offset: Option<(u32, i32)>,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meander {
    pub subjects: Vec<(usize, i16, i16, i16)>,
    pub waypoints: Vec<(i32, i32)>,
}

fn walk_names(a: &mut [MapAction], i: usize) -> Vec<(u32, Vec<i16>)> {
    let mut w = Walk::new(i, 1, None, TYPE_ACTIONS);
    let mut out = Vec::new();
    while let Some(Found { action, param }) = w.next(a) {
        let p = &a[action].parameters[param];
        out.push((u32::from_be_bytes(p.name.0), p.shorts()));
    }
    out
}

impl TflSim {
    fn act(&self) -> &[MapAction] {
        &self.map_actions.actions
    }

    pub(crate) fn set_param(
        &mut self,
        action: usize,
        name: &[u8; 4],
        kind: i16,
        count: i16,
        data: Vec<u8>,
    ) {
        let ps = &mut self.map_actions.actions[action].parameters;
        ps.retain(|p| !(p.name.0 == *name && p.kind == kind));
        ps.push(Parameter {
            kind,
            count,
            name: mandrake_common::FourCC(*name),
            data,
        });
    }

    fn delete_param(&mut self, action: usize, name: &[u8; 4], kind: i16) {
        let ps = &mut self.map_actions.actions[action].parameters;
        if let Some(k) = ps.iter().position(|p| p.name.0 == *name && p.kind == kind) {
            ps.remove(k);
        }
    }

    fn id_of(&self, i: usize) -> i16 {
        self.act()[i].identifier
    }

    fn shorts_param(&self, i: usize, flags: u32, name: &[u8; 4], kind: i16) -> Option<Vec<i16>> {
        find(self.act(), i, flags, name, kind).map(|p| p.shorts())
    }

    pub(crate) fn update_map_actions(&mut self) -> Result<(), Stop> {
        if self.tick & 0x1f != 0 {
            return Ok(());
        }
        if !self.map_actions.initialized {
            for i in 0..self.act().len() {
                let a = &self.act()[i];
                if a.identifier != 0 && a.flags & 1 != 0 {
                    self.activate(i)?;
                }
            }
            self.map_actions.initialized = true;
        }
        self.spatial_groups = None;
        self.spatial_supergroups = None;
        self.spatial_groups()?;
        self.attack_ai = Default::default();
        let mut i = 0;
        while i < self.act().len() {
            let k = i;
            i += 1;
            let a = &self.act()[k];
            if a.identifier == 0 || a.flags & FLAG_ACTIVE == 0 || (self.tick as i32) < a.time {
                continue;
            }
            let r = self.trigger(k)?;
            let ok = r & 1 != 0;
            if ok {
                if r & 2 != 0 {
                    self.succeed(k)?;
                } else {
                    self.fail(k)?;
                }
            }
            let reschedule = if r & 4 != 0 {
                false
            } else {
                match self.act()[k].kind {
                    1 => !ok,
                    2 => !ok || r & 2 == 0,
                    3 => true,
                    4 => !ok || r & 2 != 0,
                    _ => false,
                }
            };
            if reschedule {
                self.reschedule(k, false);
            } else {
                self.deactivate(k)?;
            }
        }
        self.update_attack_ai()
    }

    fn reschedule(&mut self, i: usize, activation: bool) {
        let t = self.tick as i32;
        let a = &self.map_actions.actions[i];
        let no = if activation {
            FLAG_NO_ACTIVATION_DELAY
        } else {
            FLAG_NO_RESCHEDULE_DELAY
        };
        let (flags, delay, range) = (a.flags, a.delay, a.delay_range);
        let mut time = t;
        if flags & no == 0 {
            time += delay;
            if range > 0 {
                let site = if activation {
                    draw_site::ACTION_ACTIVATION_DELAY
                } else {
                    draw_site::ACTION_RESCHEDULE_DELAY
                };
                time += self.rng.draw_scaled(site, range);
            }
        }
        self.map_actions.actions[i].time = time;
    }

    pub(crate) fn activate(&mut self, i: usize) -> Result<(), Stop> {
        let f = self.act()[i].flags;
        if f & FLAG_ACTIVE != 0 || (f & 2 != 0 && f & 0x40 != 0) {
            return Ok(());
        }
        for (name, ids) in walk_names(&mut self.map_actions.actions, i) {
            match name {
                n if n == tag(b"acoa") => self.each(&ids, Self::activate)?,
                n if n == tag(b"deoa") => self.each(&ids, Self::deactivate)?,
                _ => {}
            }
        }
        self.map_actions.actions[i].flags |= 0xd0;
        self.reschedule(i, true);
        Ok(())
    }

    pub(crate) fn deactivate(&mut self, i: usize) -> Result<(), Stop> {
        if self.act()[i].flags & FLAG_ACTIVE == 0 {
            return Ok(());
        }
        for (name, ids) in walk_names(&mut self.map_actions.actions, i) {
            match name {
                n if n == tag(b"acod") => self.each(&ids, Self::activate)?,
                n if n == tag(b"deod") => self.each(&ids, Self::deactivate)?,
                _ => {}
            }
        }
        self.map_actions.actions[i].flags &= !FLAG_ACTIVE;
        Ok(())
    }

    fn each(
        &mut self,
        ids: &[i16],
        f: fn(&mut Self, usize) -> Result<(), Stop>,
    ) -> Result<(), Stop> {
        for &id in ids.iter().take(30) {
            if let Some(j) = index_of(self.act(), id) {
                f(self, j)?;
            }
        }
        Ok(())
    }

    fn each_trigger(&mut self, ids: &[i16]) -> Result<(), Stop> {
        for &id in ids.iter().take(30) {
            if let Some(j) = index_of(self.act(), id) {
                self.trigger(j)?;
            }
        }
        Ok(())
    }

    pub(crate) fn succeed(&mut self, i: usize) -> Result<(), Stop> {
        self.map_actions.actions[i].flags |= FLAG_SUCCEEDED;
        for (name, ids) in walk_names(&mut self.map_actions.actions, i) {
            match name {
                n if n == tag(b"acos") => self.each(&ids, Self::activate)?,
                n if n == tag(b"deos") => self.each(&ids, Self::deactivate)?,
                _ => {}
            }
        }
        if self.act()[i].kind == 2 {
            self.deactivate(i)?;
        }
        Ok(())
    }

    pub(crate) fn set_error(&mut self, i: usize) -> Result<(), Stop> {
        for (name, ids) in walk_names(&mut self.map_actions.actions, i) {
            match name {
                n if n == tag(b"acer") => self.each(&ids, Self::activate)?,
                n if n == tag(b"deer") => self.each(&ids, Self::deactivate)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn fail(&mut self, i: usize) -> Result<(), Stop> {
        self.map_actions.actions[i].flags &= !FLAG_SUCCEEDED;
        for (name, ids) in walk_names(&mut self.map_actions.actions, i) {
            if name == tag(b"acof") {
                self.each(&ids, Self::activate)?;
            }
        }
        Ok(())
    }

    pub(crate) fn succeeded_any(&self, ids: &[i16]) -> bool {
        self.succeeded(ids, false)
    }

    fn succeeded(&self, ids: &[i16], all: bool) -> bool {
        for &id in ids {
            if let Some(j) = index_of(self.act(), id) {
                let s = self.act()[j].flags & FLAG_SUCCEEDED != 0;
                if !s && all {
                    return false;
                }
                if s && !all {
                    return true;
                }
            }
        }
        all
    }

    #[doc(hidden)]
    pub fn force_trigger(&mut self, i: usize) -> Result<u32, Stop> {
        self.trigger(i)
    }

    pub(crate) fn trigger(&mut self, i: usize) -> Result<u32, Stop> {
        let stack = &self.map_actions.triggering;
        if stack.len() > 3 || stack.contains(&i) {
            return Ok(0);
        }
        self.map_actions.triggering.push(i);
        let r = self.trigger_inner(i);
        self.map_actions.triggering.pop();
        r
    }

    fn trigger_inner(&mut self, i: usize) -> Result<u32, Stop> {
        let names = walk_names(&mut self.map_actions.actions, i);
        for (name, ids) in &names {
            let go = match *name {
                n if n == tag(b"inhi") => !self.succeeded(ids, false),
                n if n == tag(b"prer") => self.succeeded(ids, false),
                _ => true,
            };
            if !go {
                return Ok(0);
            }
        }
        for (name, ids) in names {
            match name {
                n if n == tag(b"acoe") || n == tag(b"acot") => self.each(&ids, Self::activate)?,
                n if n == tag(b"trsi") => self.each_trigger(&ids)?,
                _ => {}
            }
        }
        let r = self.run_proc(i)?;
        Ok(r | 1)
    }

    fn run_proc(&mut self, i: usize) -> Result<u32, Stop> {
        let t = self.act()[i].tag;
        if t == 0xffff_ffff {
            return Ok(0);
        }
        let r = match &t.to_be_bytes() {
            b"tuni" => self.test_units(i)?,
            b"move" => self.movement_command(i)?,
            b"plat" => self.platoon(i)?,
            b"squa" => self.squad(i)?,
            b"plmo" | b"plsc" => self.platoon_state(i)?,
            b"mean" => self.meander(i)?,
            b"mung" => self.munger(i)?,
            b"ctrl" => self.control(i)?,
            b"acli" => self.action_list(i)?,
            b"atta" => self.attack_action(i)?,
            b"grou" => self.group_action(i)?,
            b"spup" => self.split_up_group_action(i)?,
            b"guar" => self.guard_group_action(i)?,
            b"wand" => self.wander_action(i)?,
            b"gene" => self.general_action(i)?,
            b"rout" => self.rout_action(i)?,
            b"lead" => self.lead_action(i)?,
            b"legi" => self.legion_action(i)?,
            b"endg" => self.endgame_action(i)?,
            b"suic" => self.suicide_action(i)?,
            b"geom" => self.geometry_filter(i)?,
            b"look" => self.look_action(i)?,
            b"lock" => self.lock_action(i)?,
            b"mece" => self.mesh_cell_action(i)?,
            b"lpgr" => self.local_projectile_group_action(i)?,
            b"dela" => self.delay_action(i)?,
            b"snif" => self.sniffer_action(i)?,
            b"surr" => self.surround_action(i)?,
            b"obco" => self.object_control_action(i)?,
            b"foll" => self.follow_action(i)?,
            b"obmo" => self.observer_movement_action(i)?,
            b"mele" => self.melee_action(i)?,
            b"girl" => self.harass_action(i)?,
            b"part" => self.particle_system_action(i)?,
            b"ambi" => self.ambient_sound_action(i)?,
            b"pick" => self.pick_up_object_action(i)?,
            b"moef" => self.model_effect_action(i)?,
            b"invi" => self.invisible_action(i)?,
            b"knot" => self.world_knot_action(i)?,
            b"sham" => self.shaman_action(i)?,
            b"ligh" => self.lightning_action(i)?,
            b"moma" => self.move_marker_action(i)?,
            b"soun" => {
                self.sound_action(i);
                2
            }
            _ => {
                return Err(Stop::new(
                    draw_site::MAP_ACTION_PROC_NOT_PORTED,
                    format!(
                        "tick {}: map action {i} ({}): proc '{}' not ported",
                        self.tick,
                        self.act()[i].identifier,
                        self.act()[i].tag_name()
                    ),
                ));
            }
        };
        self.map_actions.actions[i].flags &= !0xa0;
        Ok(r)
    }

    fn test_units(&mut self, i: usize) -> Result<u32, Stop> {
        not_read_by_tfl(&[b"debg"]);
        let a = self.act();
        let reverse = find(a, i, 2, b"repo", 0).is_some();
        let glo = find(a, i, 1, b"#glo", 1).filter(|p| p.count == 8).cloned();
        let (mut most, baseline) = glo.map_or((0i16, 0u32), |p| (p.i16_at(0), p.i32_at(1) as u32));
        let mut subjects: Vec<usize> = match find(a, i, 0, b"team", 5) {
            Some(t) => {
                let team = t.i32_at(0);
                (0..self.monsters.len())
                    .filter(|&k| {
                        self.monsters[k].identifier != 0 && i32::from(self.monsters[k].team) == team
                    })
                    .take(300)
                    .collect()
            }
            None => self.monster_slots(&combined(a, i, b"subj", 2, 300)),
        };
        if i32::from(most) < subjects.len() as i32 {
            most = subjects.len() as i16;
        }
        if let Some(poly) = find(self.act(), i, 0, b"poly", 12).cloned() {
            let closed = find(self.act(), i, 2, b"clpo", 0).is_some();
            let pts: Vec<(i32, i32)> = (0..poly.count.max(0) as usize)
                .map(|k| (poly.i32_at(2 * k), poly.i32_at(2 * k + 1)))
                .collect();
            subjects.retain(|&s| {
                let o = &self.objects[self.monsters[s].object];
                point_in_polygon((o.x, o.y), &pts, closed) != reverse
            });
        }
        if let Some(radi) = find(self.act(), i, 0, b"radi", 12).cloned() {
            let c = (radi.i32_at(0), radi.i32_at(1));
            let r = match radi.count {
                1 => find(self.act(), i, 0, b"crad", 6).map_or(0, |p| p.i32_at(0)),
                2 => crate::groups::distance(c, (radi.i32_at(2), radi.i32_at(3))) as i32,
                _ => 0,
            };
            subjects.retain(|&s| {
                let o = &self.objects[self.monsters[s].object];
                ((crate::groups::distance((o.x, o.y), c) as i32) < r) != reverse
            });
        }
        let subjects = self.without(subjects, 0x4000_0000);
        let n = subjects.len() as i32;
        let mut ok = true;
        let mut cost = [0i32; 2];
        for &s in &subjects {
            let m = &self.monsters[s];
            let c = i32::from(self.data.def(b"mons", m.definition)?.i16(0x182));
            if (0..2).contains(&m.team) {
                cost[m.team as usize] += c;
            }
        }
        let ratio = if n < 1 || cost[1] == 0 {
            i32::MAX
        } else {
            (cost[0] << 16) / cost[1]
        };
        let int = |s: &Self, name: &[u8; 4], kind: i16| {
            find(s.act(), i, 0, name, kind).map(|p| p.i32_at(0))
        };
        if int(self, b"por<", 8).is_some_and(|v| v < ratio) {
            ok = false;
        }
        if int(self, b"por>", 8).is_some_and(|v| ratio < v) {
            ok = false;
        }
        if int(self, b"___>", 5).is_some_and(|v| v >= n) {
            ok = false;
        }
        if int(self, b"___<", 5).is_some_and(|v| n >= v) {
            ok = false;
        }
        if int(self, b"___=", 5).is_some_and(|v| n != v) {
            ok = false;
        }
        let (mut vit, mut pow) = (0u32, 0i32);
        for &s in &subjects {
            let m = &self.monsters[s];
            let mons = self.data.def(b"mons", m.definition)?;
            let most_vit = i32::from(mons.i16(0x128));
            pow = pow.wrapping_add(i32::from(mons.i16(0x182)));
            if most_vit == 0 {
                return Err(Stop::new(
                    draw_site::TEST_UNITS_VITALITY_ZERO,
                    format!("map action {i}: tuni: monster {s}'s vitality"),
                ));
            }
            vit = vit.wrapping_add(((i32::from(m.vitality) << 16) / most_vit) as u32);
        }
        let (vit, pow) = if most == 0 {
            (0u32, 0u32)
        } else {
            let d = i32::from(most) as u32;
            (vit / d, (pow << 16) as u32 / d)
        };
        let (vit, pow) = (vit as i32, pow as i32);
        if int(self, b">vit", 8).is_some_and(|v| v >= vit) {
            ok = false;
        }
        if int(self, b"<vit", 8).is_some_and(|v| vit >= v) {
            ok = false;
        }
        let mut baseline = baseline;
        if let Some(v) = int(self, b"devi", 8) {
            if self.act()[i].flags & 0x80 != 0 {
                ok = false;
                baseline = vit as u32;
            } else if v >= (baseline as i32).wrapping_sub(vit).wrapping_abs() {
                ok = false;
            }
        }
        if int(self, b">pow", 8).is_some_and(|v| v >= pow) {
            ok = false;
        }
        if int(self, b"<pow", 8).is_some_and(|v| pow >= v) {
            ok = false;
        }
        if let Some(hold) = find(self.act(), i, 0, b"hold", 7)
            .filter(|p| p.count > 0)
            .map(|p| mandrake_common::FourCC((p.i32_at(0) as u32).to_be_bytes()))
        {
            let holding = subjects
                .iter()
                .filter(|&&s| {
                    let c = self.monsters[s].carried;
                    c != -1 && {
                        let o = &self.objects[c as usize];
                        o.kind == 4 && o.sleeping == Some(hold)
                    }
                })
                .count() as i16;
            if holding == 0 || (find(self.act(), i, 2, b"_all", 0).is_some() && holding != n as i16)
            {
                ok = false;
            }
        }
        if find(self.act(), i, 2, b"user", 0).is_some() {
            ok = subjects
                .iter()
                .any(|&s| self.monsters[s].flags & 0x0800_0000 != 0);
        }
        let d = i32::from(self.difficulty);
        if int(self, b">dif", 5).is_some_and(|v| d < v) {
            ok = false;
        }
        if int(self, b"<dif", 5).is_some_and(|v| v < d) {
            ok = false;
        }
        if int(self, b"diff", 5).is_some_and(|v| d < v) {
            ok = false;
        }
        if let Some(clos) = find(self.act(), i, 0, b"clos", 6)
            .filter(|p| p.count >= 1)
            .cloned()
        {
            let inner = clos.i32_at(0);
            let outer = if clos.count > 1 {
                clos.i32_at(1)
            } else {
                inner.wrapping_add(1)
            };
            let enemies = self.monster_slots(&combined(self.act(), i, b"enem", 2, 300));
            let listed = !combined(self.act(), i, b"enem", 2, 300).is_empty();
            ok = false;
            'subjects: for &s in &subjects {
                let so = self.monsters[s].object;
                let team = self.monsters[s].team;
                let others: Vec<usize> = if listed {
                    enemies.clone()
                } else {
                    (0..self.monsters.len())
                        .filter(|&k| {
                            self.monsters[k].identifier != 0 && self.monsters[k].team != team
                        })
                        .collect()
                };
                for k in others {
                    let o = self.monsters[k].object;
                    if self.close(so, o, outer, inner) {
                        ok = true;
                        break 'subjects;
                    }
                }
            }
        }
        let mut glo = most.to_be_bytes().to_vec();
        glo.extend_from_slice(&[0, 0]);
        glo.extend_from_slice(&baseline.to_be_bytes());
        self.set_param(i, b"#glo", 1, 8, glo);
        Ok(if ok { 2 } else { 0 })
    }

    fn close(&mut self, a: usize, b: usize, outer: i32, inner: i32) -> bool {
        let pa = (self.objects[a].x, self.objects[a].y);
        let pb = (self.objects[b].x, self.objects[b].y);
        let d = crate::groups::distance(pa, pb) as i32;
        if d >= outer {
            return false;
        }
        if d <= inner {
            return true;
        }
        let r = (i32::from(self.rng.draw(draw_site::CLOSE_DISTANCE)) << 8) >> 16;
        let q = ((i32::from(outer.wrapping_sub(d) as i16)) << 8) / outer.wrapping_sub(inner);
        r < i32::from(q as i16)
    }

    pub(crate) fn monster_slots(&self, ids: &[i16]) -> Vec<usize> {
        ids.iter()
            .filter_map(|&id| {
                self.monsters
                    .iter()
                    .position(|m| m.identifier != 0 && m.identifier == id)
            })
            .collect()
    }

    fn sound_action(&mut self, i: usize) {
        let a = self.act();
        let ids = combined(a, i, b"subj", 2, 300);
        let over = find(a, i, 2, b"over", 0).is_some();
        let Some(sound) = element(a, i, b"soun", 0xb)
            .map(|p| mandrake_common::FourCC((p.i32_at(0) as u32).to_be_bytes()))
        else {
            return;
        };
        if sound.0 == [0xff; 4] {
            return;
        }
        let sound = Some(sound);
        let mut slots = self.monster_slots(&ids);
        if !over {
            slots = self.without(slots, 0x0800_0000);
        }
        let kind = crate::sounds::SoundKind::SoundAction {
            action: i,
            candidates: slots.clone(),
        };
        if ids.is_empty() {
            self.flat_sound(draw_site::SOUND_ACTION_NO_SUBJECTS, kind, sound);
        } else if !slots.is_empty() {
            self.flat_sound(draw_site::SOUND_ACTION, kind, sound);
        }
    }

    fn without(&self, slots: Vec<usize>, mask: u32) -> Vec<usize> {
        slots
            .into_iter()
            .filter(|&s| self.monsters[s].flags & mask == 0)
            .collect()
    }

    fn movement_command(&mut self, i: usize) -> Result<u32, Stop> {
        let ids = combined(self.act(), i, b"subj", 2, 30);
        if ids.is_empty() {
            return Ok(0);
        }
        let slots = self.without(self.monster_slots(&ids), 0x0800_0000);
        let Some(wayp) = find(self.act(), i, 2, b"wayp", 12).cloned() else {
            return Ok(0);
        };
        if wayp.count >= 5 {
            return Ok(0);
        }
        let a = self.act();
        let mut command = MovementCommand {
            flags: 0,
            formation: 0,
            facing: 0,
        };
        if find(a, i, 2, b"cycl", 0).is_some() {
            command.flags |= 1;
        } else if find(a, i, 2, b"back", 0).is_some() {
            command.flags |= 3;
        }
        if find(a, i, 2, b"prta", 0).is_some() {
            command.flags |= 4;
        }
        if let Some(form) = element(a, i, b"form", 5) {
            command.flags |= 8;
            command.formation = form.i32_at(0) as u16;
        }
        if let Some(fifa) = element(a, i, b"fifa", 4) {
            command.flags |= 0x10;
            command.facing = fifa.i16_at(0) as u16;
        }
        if !slots.is_empty() {
            let points: Vec<(i32, i32)> = (0..wayp.count.max(0) as usize)
                .map(|k| (wayp.i32_at(2 * k), wayp.i32_at(2 * k + 1)))
                .collect();
            self.record_order(|| Order {
                site: draw_site::MOVEMENT_ORDER,
                action: i,
                monsters: slots.clone(),
                target: points.first().copied(),
                offset: None,
                done: true,
            });
            let id = self.id_of(i);
            self.execute_movement_command(id, &command, &slots, &points)?;
        }
        Ok(0)
    }

    fn platoon(&mut self, i: usize) -> Result<u32, Stop> {
        if let Some(stat) = self.shorts_param(i, 0, b"stat", TYPE_ACTIONS) {
            let r = self.rng.draw(draw_site::PLATOON_STATE);
            let state = index_of(self.act(), stat[usize::from(r) % stat.len()]);
            self.delete_param(i, b"stat", TYPE_ACTIONS);
            self.set_platoon_state(i, state)?;
        }
        if let Some(cst) = self.shorts_param(i, 0, b"#cst", TYPE_ACTIONS)
            && let Some(state) = index_of(self.act(), cst[0])
        {
            let own = self.id_of(i).to_be_bytes().to_vec();
            self.set_param(state, b"#own", TYPE_ACTIONS, 1, own);
            self.trigger(state)?;
            return Ok(0);
        }
        Ok(4)
    }

    fn set_platoon_state(&mut self, plat: usize, state: Option<usize>) -> Result<(), Stop> {
        if self.act()[plat].tag != tag(b"plat") {
            return Ok(());
        }
        match state {
            Some(s) => {
                self.set_param(s, b"#sub", 5, 1, (-1i32).to_be_bytes().to_vec());
                let id = self.id_of(s).to_be_bytes().to_vec();
                self.set_param(plat, b"#cst", TYPE_ACTIONS, 1, id);
                self.reset_platoon(plat)
            }
            None => {
                self.set_param(plat, b"#cst", TYPE_ACTIONS, 0, Vec::new());
                Ok(())
            }
        }
    }

    fn reset_platoon(&mut self, plat: usize) -> Result<(), Stop> {
        if self.act()[plat].tag != tag(b"plat") {
            return Ok(());
        }
        self.delete_param(plat, b"#squ", TYPE_ACTIONS);
        self.map_actions.actions[plat].parameters.push(Parameter {
            kind: TYPE_ACTIONS,
            count: 0,
            name: mandrake_common::FourCC(*b"#squ"),
            data: Vec::new(),
        });
        let Some(init) = self.shorts_param(plat, 0, b"init", TYPE_ACTIONS) else {
            return Ok(());
        };
        let id = self.id_of(plat).to_be_bytes().to_vec();
        for s in init {
            if let Some(j) = index_of(self.act(), s) {
                self.set_param(j, b"#owp", TYPE_ACTIONS, 1, id.clone());
                self.trigger(j)?;
            }
        }
        Ok(())
    }

    fn squad(&mut self, i: usize) -> Result<u32, Stop> {
        let Some(owner) = self
            .shorts_param(i, 0, b"#owp", TYPE_ACTIONS)
            .and_then(|o| index_of(self.act(), o[0]))
        else {
            return Ok(0);
        };
        let id = self.id_of(i);
        let ps = &mut self.map_actions.actions[owner].parameters;
        if let Some(p) = ps
            .iter_mut()
            .find(|p| p.name.0 == *b"#squ" && p.kind == TYPE_ACTIONS)
        {
            p.count += 1;
            p.data.extend_from_slice(&id.to_be_bytes());
        }
        Ok(0)
    }

    fn platoon_state(&mut self, i: usize) -> Result<u32, Stop> {
        let Some(plat) = self
            .shorts_param(i, 0, b"#own", TYPE_ACTIONS)
            .and_then(|o| index_of(self.act(), o[0]))
        else {
            return Ok(0);
        };
        let Some(squads) = self.shorts_param(plat, 3, b"#squ", TYPE_ACTIONS) else {
            return Ok(0);
        };
        let mut ids = Vec::new();
        for s in squads {
            if let Some(j) = index_of(self.act(), s) {
                let room = 300 - ids.len();
                ids.extend(combined(self.act(), j, b"subj", 2, room));
            }
        }
        let slots = self.monster_slots(&ids);
        let centre = self.monsters_centre(&slots);
        let old = find(self.act(), i, 0, b"#sub", 5).map_or(-1, |p| p.i32_at(0));
        let mut sub = old;
        if find(self.act(), plat, 2, b"#att", 0).is_some() {
            sub = 3;
            self.delete_param(plat, b"#att", 0);
        }
        let mut target = None;
        sub = if self.act()[i].tag == tag(b"plmo") {
            self.platoon_check(i, &slots, centre, sub)?
        } else {
            let (s, g) = self.platoon_engage(i, &slots, centre, sub)?;
            target = g;
            s
        };
        match sub {
            0 => self.handle_moving_substate(i, plat, old, centre)?,
            3 => self.platoon_attack(plat, i, &slots, centre)?,
            1 | 2 => {
                let g = target.ok_or_else(|| {
                    Stop::new(
                        draw_site::PLATOON_CLOSING_WITHOUT_GROUP,
                        "a platoon closing without a group",
                    )
                })?;
                self.platoon_close(sub, plat, i, &slots, centre, &g)?;
            }
            _ => {}
        }
        if sub != old {
            self.set_param(i, b"#sub", 5, 1, sub.to_be_bytes().to_vec());
        }
        Ok(0)
    }

    pub(crate) fn monsters_centre(&self, slots: &[usize]) -> (i32, i32) {
        if slots.is_empty() {
            return (-1, -1);
        }
        let n = slots.len() as i32;
        let (sx, sy) = slots.iter().fold((0i32, 0i32), |(x, y), &s| {
            let o = &self.objects[self.monsters[s].object];
            (x.wrapping_add(o.x), y.wrapping_add(o.y))
        });
        (sx / n, sy / n)
    }

    fn platoon_check(
        &mut self,
        i: usize,
        slots: &[usize],
        centre: (i32, i32),
        sub: i32,
    ) -> Result<i32, Stop> {
        if slots.is_empty() {
            return Ok(0);
        }
        let team = self.monsters[slots[0]].team as i8 as i16;
        let Some(radi) = find(self.act(), i, 0, b"radi", 6).filter(|p| p.count == 2) else {
            return Ok(0);
        };
        let (near, far) = (radi.i32_at(0), radi.i32_at(1));
        let groups = self.spatial_groups()?;
        let mut best: Option<i32> = None;
        for g in groups.iter().filter(|g| g.team != team) {
            let d = crate::groups::distance(centre, g.centre) as i32 - i32::from(g.radius);
            if best.is_none_or(|b| d < b) {
                best = Some(d);
            }
        }
        let best = best.unwrap_or(i32::MAX);
        if best >= far {
            return Ok(0);
        }
        let v = best - near;
        if v < 0 || (v < 0x4000 && sub == 3) {
            return Ok(3);
        }
        if v >= 0x4000 {
            return Ok(0);
        }
        let span = far.wrapping_sub(near);
        if span == 0 {
            return Err(Stop::new(
                draw_site::PLATOON_CHECK,
                "platoon check with empty radius span",
            ));
        }
        let chance = 0x4000 - (v << 14) / span;
        let r = i32::from(self.rng.draw(draw_site::PLATOON_CHECK));
        Ok(if r >= chance { 0 } else { 3 })
    }

    fn platoon_engage(
        &mut self,
        i: usize,
        slots: &[usize],
        centre: (i32, i32),
        sub: i32,
    ) -> Result<(i32, Option<crate::groups::Group>), Stop> {
        if slots.is_empty() {
            return Ok((0, None));
        }
        let team = self.monsters[slots[0]].team as i8 as i16;
        let Some(radi) = find(self.act(), i, 0, b"radi", 6).filter(|p| p.count == 2) else {
            return Ok((0, None));
        };
        let reach = radi.i32_at(1);
        let charge = sub == 3 && find(self.act(), i, 2, b"chrg", 0).is_some();
        let groups = self.spatial_groups()?;
        let mut best: Option<(i32, i32, bool, usize)> = None;
        for (k, g) in groups.iter().enumerate().filter(|(_, g)| g.team != team) {
            let d = crate::groups::distance(centre, g.centre) as i32;
            let edge = d - i32::from(g.radius);
            let gap = edge - i32::from(g.spacing);
            let engage_flag = charge || g.definition_flags & 0x80 != 0;
            if !engage_flag && gap >= 0 {
                continue;
            }
            if best.is_none_or(|(b, ..)| gap < b) {
                best = Some((gap, edge, engage_flag, k));
            }
        }
        let Some((_, edge, flag, k)) = best else {
            return Ok((0, None));
        };
        if edge >= reach {
            return Ok((0, None));
        }
        let g = groups[k].clone();
        let mut strength = 0i32;
        for &s in slots {
            strength += i32::from(
                self.data
                    .def(b"mons", self.monsters[s].definition)?
                    .i16(0x182),
            );
        }
        let mut gcost = i32::from(g.cost);
        let a = crate::trig::arctangent(centre.0 - g.centre.0, centre.1 - g.centre.1);
        if crate::trig::angle_difference(i32::from(g.facing), i32::from(a)) > 0x4000 {
            gcost >>= 1;
        }
        let ratio = if gcost == 0 {
            i32::MAX
        } else {
            (strength << 16) / gcost
        };
        let result = if flag { 1 } else { 2 };
        let d = crate::groups::distance(g.centre, centre) as i32;
        let near = d < (i32::from(g.spacing) >> 1) + i32::from(g.radius) * 2;
        let open = near || charge || self.group_in_the_way(centre, &g, &groups).is_none();
        if open
            && (sub == 3
                || (i32::from(self.rng.draw(draw_site::PLATOON_ENGAGE))
                    < ratio.wrapping_sub(0xc000)))
            && (flag || 0xc000 < ratio)
        {
            return Ok((3, Some(g)));
        }
        Ok((result, Some(g)))
    }

    fn group_in_the_way(
        &self,
        p: (i32, i32),
        g: &crate::groups::Group,
        groups: &[crate::groups::Group],
    ) -> Option<usize> {
        let (dx, dy) = (p.0 - g.centre.0, p.1 - g.centre.1);
        let a = crate::trig::arctangent(dx, dy);
        for (k, o) in groups.iter().enumerate() {
            let kind = self.data_def_i16(o.definition, 0x146);
            if kind == 0 && o != g && o.team == g.team {
                let (ex, ey) = (p.0 - o.centre.0, p.1 - o.centre.1);
                let near = ey
                    .wrapping_mul(ey)
                    .wrapping_sub(i32::from(o.radius))
                    .wrapping_add(ex.wrapping_mul(ex));
                if near < dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) {
                    let b = crate::trig::arctangent(ex, ey);
                    if (crate::trig::angle_difference(i32::from(a), i32::from(b)) as i32) < 0x2000 {
                        return Some(k);
                    }
                }
            }
        }
        None
    }

    pub(crate) fn data_def_i16(&self, id: mandrake_common::FourCC, o: usize) -> i16 {
        self.data.peek_def(b"mons", id).map_or(0, |d| d.i16(o))
    }

    fn platoon_close(
        &mut self,
        sub: i32,
        plat: usize,
        i: usize,
        slots: &[usize],
        centre: (i32, i32),
        g: &crate::groups::Group,
    ) -> Result<(), Stop> {
        let mut angle: i32 = 0;
        if sub == 2 {
            if let Some(w) = find(self.act(), i, 0, b"wayp", 12).cloned()
                && let Some(k) = find(self.act(), i, 0, b"#wpi", 5)
                    .filter(|p| p.count > 0)
                    .map(|p| p.i32_at(0))
                && 0 <= k
                && k < i32::from(w.count)
            {
                let k = k as usize;
                angle = i32::from(crate::trig::arctangent(
                    centre.0 - w.i32_at(2 * k),
                    centre.1 - w.i32_at(2 * k + 1),
                ));
            }
        } else if !slots.is_empty() {
            let team = self.monsters[slots[0]].team as i8 as i16;
            let (mut sx, mut sy) = (0i32, 0i32);
            for o in self.spatial_groups()?.iter().filter(|o| o.team == team) {
                let (dx, dy) = (o.centre.0 - centre.0, o.centre.1 - centre.1);
                let n = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
                if n != 0 && n < 0x2710_0000 {
                    let r = crate::groups::isqrt(n as u32) as i32;
                    sx = sx.wrapping_add(dx.wrapping_mul(0x10000) / r);
                    sy = sy.wrapping_add(dy.wrapping_mul(0x10000) / r);
                }
            }
            angle = i32::from(crate::trig::arctangent(sx, sy));
        }
        let reach = i32::from(g.spacing) + i32::from(g.radius);
        let a = i32::from(crate::trig::arctangent(
            g.centre.0 - centre.0,
            g.centre.1 - centre.1,
        ));
        let mut d = a - angle;
        if -0x8001 < d && d < 0x8001 {
            d = -d;
        } else if -0x8000 < d {
            d -= 0x8000;
        } else {
            d += 0x8000;
        }
        let d = d.clamp(-0x1000, 0x1000);
        let f = (a + d) & 0xffff;
        let k = crate::trig::index(f);
        let t = crate::trig::trig();
        let point = (
            g.centre.0 - ((i32::from(t.cosine[k]) * reach) >> 14),
            g.centre.1 - ((i32::from(t.sine[k]) * reach) >> 14),
        );
        let last = find(self.act(), i, 0, b"#lde", 12)
            .filter(|p| p.count > 0)
            .map(|p| (p.i32_at(0), p.i32_at(1)));
        if last.is_none_or(|l| crate::groups::distance(l, point) as i32 > 0x200) {
            self.platoon_movement(plat, point, f as u16)?;
            let mut data = point.0.to_be_bytes().to_vec();
            data.extend(point.1.to_be_bytes());
            self.set_param(i, b"#lde", 12, 1, data);
        }
        Ok(())
    }

    fn platoon_attack(
        &mut self,
        plat: usize,
        i: usize,
        slots: &[usize],
        centre: (i32, i32),
    ) -> Result<(), Stop> {
        if let Some(alli) = find(self.act(), plat, 0, b"alli", 3).map(|p| p.shorts()) {
            for id in alli {
                if let Some(j) = index_of(self.act(), id) {
                    self.set_param(j, b"#att", 0, 0, Vec::new());
                }
            }
        }
        let mut config = self.attack_configuration(Some(i));
        config.point = centre;
        self.attack_with_monsters(slots, config);
        Ok(())
    }

    fn platoon_movement(
        &mut self,
        plat: usize,
        target: (i32, i32),
        facing: u16,
    ) -> Result<(), Stop> {
        if self.act()[plat].tag != tag(b"plat") {
            return Ok(());
        }
        let a = self.act();
        let (Some(cent), Some(faci)) =
            (element(a, plat, b"cent", 12), element(a, plat, b"faci", 4))
        else {
            return Ok(());
        };
        let centre = (cent.i32_at(0), cent.i32_at(1));
        let faci = faci.i16_at(0) as u16;
        let Some(squads) = self.shorts_param(plat, 3, b"#squ", TYPE_ACTIONS) else {
            return Ok(());
        };
        for s in squads {
            let Some(j) = index_of(self.act(), s) else {
                return Err(Stop::new(
                    draw_site::PLATOON_SQUAD_MISSING,
                    format!("platoon {plat}: squad {s} not found"),
                ));
            };
            self.squad_movement(j, centre, target, faci, facing)?;
        }
        Ok(())
    }

    fn squad_movement(
        &mut self,
        j: usize,
        plat_centre: (i32, i32),
        target: (i32, i32),
        plat_facing: u16,
        facing: u16,
    ) -> Result<(), Stop> {
        if self.act()[j].tag != tag(b"squa") {
            return self.succeed(j);
        }
        let ids = combined(self.act(), j, b"subj", 2, 0x1e);
        let slots = self.monster_slots(&ids);
        if slots.is_empty() {
            return self.succeed(j);
        }
        let a = self.act();
        let own = element(a, j, b"faci", 4).map_or(0, |p| p.i16_at(0));
        let (Some(cent), Some(form)) = (element(a, j, b"cent", 12), element(a, j, b"form", 5))
        else {
            return self.succeed(j);
        };
        let centre = crate::trig::rotate(
            (cent.i32_at(0), cent.i32_at(1)),
            plat_centre,
            i32::from(plat_facing) - i32::from(facing),
        );
        let command = MovementCommand {
            flags: 0x18,
            formation: form.i32_at(0) as u16,
            facing: facing.wrapping_add(own as u16),
        };
        let point = (
            target.0 - plat_centre.0 + centre.0,
            target.1 - plat_centre.1 + centre.1,
        );
        let id = self.id_of(j);
        self.execute_movement_command(id, &command, &slots, &[point])
    }

    fn handle_moving_substate(
        &mut self,
        i: usize,
        plat: usize,
        old: i32,
        centre: (i32, i32),
    ) -> Result<(), Stop> {
        let mut ntrp = find(self.act(), i, 2, b"ntrp", 0).is_some();
        if find(self.act(), i, 2, b"cong", 0).is_some() {
            self.congregation(i, centre);
        }
        let Some(wayp) = find(self.act(), i, 0, b"wayp", 12).cloned() else {
            return Ok(());
        };
        let n = i32::from(wayp.count);
        if n == 1 {
            ntrp = false;
        }
        let point = |k: i32| (wayp.i32_at(2 * k as usize), wayp.i32_at(2 * k as usize + 1));
        let long = |s: &Self, name: &[u8; 4]| find(s.act(), i, 0, name, 5).map(|p| p.i32_at(0));
        let mut k = long(self, b"#wpi").unwrap_or(i32::from(ntrp));
        let (mut iwi, mut iwc) = if ntrp {
            (
                long(self, b"#iwi").unwrap_or(0),
                long(self, b"#iwc").unwrap_or(0),
            )
        } else {
            (0, 0)
        };
        if old == 0 {
            let Some(squads) = self.shorts_param(plat, 0, b"#squ", TYPE_ACTIONS) else {
                return Ok(());
            };
            let waiting = squads
                .iter()
                .any(|&s| index_of(self.act(), s).is_some_and(|j| self.act()[j].flags & 0x20 == 0));
            if waiting {
                return Ok(());
            }
            self.reset_platoon(plat)?;
            if !ntrp {
                k += 1;
            } else {
                iwi += 1;
                if iwi >= iwc {
                    k += 1;
                    iwi = 0;
                    iwc = self.interpolation_steps(i, k, iwc)?;
                }
            }
        } else if !ntrp {
            let mut best: Option<u32> = None;
            k = -1;
            for w in 0..n {
                let d = crate::groups::distance(centre, point(w));
                if best.is_none_or(|b| d < b) {
                    best = Some(d);
                    k = w;
                }
            }
        } else {
            iwi = 0;
            iwc = self.interpolation_steps(i, k, iwc)?;
        }
        let looped = find(self.act(), i, 2, b"loop", 0).is_some();
        if looped {
            if ntrp {
                if k > n {
                    k = 1;
                }
            } else if k == n {
                k = 0;
            }
        }
        let past = if ntrp {
            k < 1 || k > n
        } else {
            k < 0 || k >= n
        };
        if past {
            self.succeed(i)?;
            self.set_platoon_state(plat, None)?;
        } else {
            let (mut target, facing) = if ntrp {
                let p = self.interpolated_point(i, k, iwi, iwc)?;
                let facing = if k == 1 && iwi == 0 {
                    let (mut j, mut kk) = (1, 1);
                    if j >= iwc {
                        j = 0;
                        kk = 2;
                        if kk > n {
                            kk = if looped { 1 } else { n };
                        }
                    }
                    let q = self.interpolated_point(i, kk, j, iwc)?;
                    crate::trig::arctangent(q.0.wrapping_sub(p.0), q.1.wrapping_sub(p.1))
                } else {
                    let (j, kk) = if iwi - 1 < 0 {
                        (iwc - 1, k - 1)
                    } else {
                        (iwi - 1, k)
                    };
                    let q = self.interpolated_point(i, kk, j, iwc)?;
                    crate::trig::arctangent(p.0.wrapping_sub(q.0), p.1.wrapping_sub(q.1))
                };
                (p, facing)
            } else {
                let facing = if n > 1 {
                    let (from, to) = if k == 0 {
                        (point(0), point(1))
                    } else {
                        (point(k - 1), point(k))
                    };
                    crate::trig::arctangent(to.0 - from.0, to.1 - from.1)
                } else {
                    0
                };
                (point(k), facing)
            };
            let mut offset = None;
            if let Some(wayr) = find(self.act(), i, 0, b"wayr", 6).cloned()
                && k >= 0
                && k < i32::from(wayr.count)
            {
                let r = wayr.i32_at(k as usize);
                if r != 0 {
                    let angle = u32::from(self.rng.draw(draw_site::PLATOON_WALK_ANGLE));
                    let d =
                        i32::from(self.rng.draw(draw_site::PLATOON_WALK_DISTANCE)).wrapping_rem(r);
                    let t = (((angle + 0x20) >> 6) & 0x3ff) as usize;
                    offset = Some((t as u32, d));
                    let trig = crate::trig::trig();
                    target.0 += (i32::from(trig.cosine[t]) * d) >> 14;
                    target.1 += (i32::from(trig.sine[t]) * d) >> 14;
                }
            }
            let mut monsters = Vec::new();
            if let Some(squads) = self.shorts_param(plat, 3, b"#squ", TYPE_ACTIONS) {
                for s in squads {
                    if let Some(j) = index_of(self.act(), s) {
                        monsters.extend(self.monster_slots(&combined(
                            self.act(),
                            j,
                            b"subj",
                            2,
                            300,
                        )));
                    }
                }
            }
            self.record_order(|| Order {
                site: draw_site::PLATOON_WALK_ORDER,
                action: plat,
                monsters,
                target: Some(target),
                offset,
                done: true,
            });
            self.platoon_movement(plat, target, facing)?;
        }
        self.set_param(i, b"#wpi", 5, 1, k.to_be_bytes().to_vec());
        if ntrp {
            self.set_param(i, b"#iwi", 5, 1, iwi.to_be_bytes().to_vec());
            self.set_param(i, b"#iwc", 5, 1, iwc.to_be_bytes().to_vec());
        }
        Ok(())
    }

    fn congregation(&mut self, i: usize, centre: (i32, i32)) {
        let mut data = centre.0.to_be_bytes().to_vec();
        data.extend(centre.1.to_be_bytes());
        match find(self.act(), i, 0, b"wayp", 12).cloned() {
            None => self.set_param(i, b"wayp", 12, 1, data),
            Some(w) if i32::from(w.count) + 1 < 0x20 => {
                data.extend(&w.data[..8 * w.count.max(0) as usize]);
                self.set_param(i, b"wayp", 12, w.count + 1, data);
            }
            Some(_) => {}
        }
        self.map_actions.actions[i]
            .parameters
            .retain(|p| p.name.0 != *b"cong");
    }

    fn interpolation_leg(&self, i: usize, k: i32, site: u32) -> Result<Option<Leg>, Stop> {
        let Some(f) = Walk::new(i, 0, Some(b"wayp"), 12).next(self.act()) else {
            return Ok(None);
        };
        let n = i32::from(self.act()[f.action].parameters[f.param].count);
        let a = self.waypoint_read(f, k - 1, site)?;
        let b = if k == n {
            if find(self.act(), i, 2, b"loop", 0).is_some() {
                self.waypoint_read(f, 0, site)?
            } else {
                a
            }
        } else {
            self.waypoint_read(f, k, site)?
        };
        Ok(Some((a, b)))
    }

    fn waypoint_read(&self, f: Found, j: i32, site: u32) -> Result<(i32, i32), Stop> {
        let p = &self.act()[f.action].parameters[f.param];
        if (0..i32::from(p.count)).contains(&j) {
            return Ok((p.i32_at(2 * j as usize), p.i32_at(2 * j as usize + 1)));
        }
        let (bytes, offsets) = self.act()[f.action].memory();
        let at = offsets[f.param] as i64 + 8 * i64::from(j);
        let long = |o: i64| {
            let b = bytes.get(usize::try_from(o).ok()?..usize::try_from(o).ok()? + 4)?;
            Some(i32::from_le_bytes([b[0]?, b[1]?, b[2]?, b[3]?]))
        };
        match (long(at), long(at + 4)) {
            (Some(x), Some(y)) => Ok((x, y)),
            _ => Err(Stop::new(
                site,
                format!(
                    "map action {}: waypoint {j} of {} reads padding or past the action's parameters",
                    f.action, p.count
                ),
            )),
        }
    }

    fn interpolation_steps(&self, i: usize, k: i32, count: i32) -> Result<i32, Stop> {
        let n = find(self.act(), i, 0, b"wayp", 12).map_or(0, |w| i32::from(w.count));
        if k > n && find(self.act(), i, 2, b"loop", 0).is_none() {
            return Ok(count);
        }
        let Some((a, b)) = self.interpolation_leg(i, k, draw_site::INTERPOLATION_STEPS_LEG)? else {
            return Ok(count);
        };
        let d = crate::groups::distance(a, b) as i32;
        if d == 0 {
            return Ok(1);
        }
        let spac = element(self.act(), i, b"spac", 6).map_or(0x400, |p| p.i32_at(0));
        if spac == 0 {
            return Err(Stop::new(
                draw_site::INTERPOLATION_SPACING_ZERO,
                format!("map action {i}: 'spac' 0 (a division by 0)"),
            ));
        }
        Ok(d / spac)
    }

    fn interpolated_point(&self, i: usize, k: i32, j: i32, count: i32) -> Result<(i32, i32), Stop> {
        if count == 0 {
            return Err(Stop::new(
                draw_site::INTERPOLATION_STOPS_ZERO,
                format!("map action {i}: an interpolation of 0 stops (a division by 0)"),
            ));
        }
        let t = i32::from(j.wrapping_shl(8).wrapping_div(count) as i16);
        let Some((a, b)) = self.interpolation_leg(i, k, draw_site::INTERPOLATED_POINT_LEG)? else {
            return Err(Stop::new(
                draw_site::INTERPOLATION_WITHOUT_WAYPOINTS,
                format!("map action {i}: no 'wayp' (out unset)"),
            ));
        };
        Ok((
            (b.0.wrapping_sub(a.0).wrapping_mul(t) >> 8).wrapping_add(a.0),
            (b.1.wrapping_sub(a.1).wrapping_mul(t) >> 8).wrapping_add(a.1),
        ))
    }

    fn meander(&mut self, i: usize) -> Result<u32, Stop> {
        let had = self.meanders.contains_key(&i);
        if !had {
            let ids = combined(self.act(), i, b"subj", 2, 0x20);
            let slots = self.monster_slots(&ids);
            let mut subjects = Vec::new();
            for s in slots {
                let r = self.rng.draw(draw_site::MEANDER_SUBJECT);
                subjects.push((s, self.monsters[s].identifier, 0, (r % 12) as i16));
            }
            let mut waypoints = Vec::new();
            if let Some(w) = find(self.act(), i, 2, b"wayp", 12) {
                if w.count > 0x10 {
                    return Err(Stop::new(
                        draw_site::MEANDER_TOO_MANY_WAYPOINTS,
                        "too many waypoints in search group!",
                    ));
                }
                waypoints = (0..w.count as usize)
                    .map(|k| (w.i32_at(2 * k), w.i32_at(2 * k + 1)))
                    .collect();
            }
            self.meanders.insert(
                i,
                Meander {
                    subjects,
                    waypoints,
                },
            );
        }
        let stored = self.meanders.remove(&i).expect("made above");
        let mut m = stored.clone();
        let mut k = 0;
        while k < m.subjects.len() {
            let (s, id, _, _) = m.subjects[k];
            if self.monsters.get(s).is_some_and(|x| x.identifier == id) {
                k += 1;
                continue;
            }
            m.subjects.remove(k);
            if let Err(e) = self.activate_listed(i, b"casu") {
                if had {
                    self.meanders.insert(i, stored);
                }
                return Err(e);
            }
        }
        if m.subjects.is_empty() {
            if had {
                self.meanders.insert(i, stored);
            }
            return Ok(4);
        }
        for k in 0..m.subjects.len() {
            m.subjects[k].3 -= 1;
            if m.subjects[k].2 == 0
                && m.subjects[k].3 < 0
                && let Err(e) = self.meander_send(i, &mut m, k)
            {
                self.meanders.insert(i, m);
                return Err(e);
            }
        }
        self.meanders.insert(i, m);
        self.set_param(i, b"#glo", 1, 0x18c, vec![0; 0x18c]);
        Ok(0)
    }
}

impl TflSim {
    fn meander_send(&mut self, i: usize, m: &mut Meander, k: usize) -> Result<(), Stop> {
        const RADIUS: i32 = 0xc00;
        m.subjects[k].2 = 0;
        let slot = m.subjects[k].0;
        let object = self.monsters[slot].object;
        let pos = (self.objects[object].x, self.objects[object].y);
        if m.waypoints.is_empty() {
            return Err(Stop::new(
                draw_site::MEANDER_WAYPOINT,
                "a meander without waypoints (a division by 0)",
            ));
        }
        let w = self.rng.draw(draw_site::MEANDER_WAYPOINT) as usize % m.waypoints.len();
        let (mut x, mut y) = m.waypoints[w];
        x += i32::from(self.rng.draw(draw_site::MEANDER_OFFSET_X)) % (2 * RADIUS) - RADIUS;
        y += i32::from(self.rng.draw(draw_site::MEANDER_OFFSET_Y)) % (2 * RADIUS) - RADIUS;
        let command = crate::commands::MovementCommand {
            flags: 0,
            formation: 0,
            facing: 0,
        };
        self.record_order(|| Order {
            site: draw_site::MEANDER_ORDER,
            action: i,
            monsters: vec![slot],
            target: Some((x, y)),
            offset: None,
            done: true,
        });
        let id = self.id_of(i);
        self.execute_movement_command(id, &command, &[slot], &[(x, y)])?;
        let d = crate::groups::distance((x, y), pos) as i32;
        let speed = i32::from(
            self.data
                .def(b"mons", self.monsters[slot].definition)?
                .i16(0x8a),
        ) << 5;
        if speed == 0 {
            return Err(Stop::new(
                draw_site::MEANDER_ORDER,
                "a meander subject with no speed",
            ));
        }
        let half = (((d / speed) as i16).wrapping_add(2)) >> 1;
        if half == 0 {
            return Err(Stop::new(
                draw_site::MEANDER_WAIT,
                "a meander walk of no time (a division by 0)",
            ));
        }
        let r = self.rng.draw(draw_site::MEANDER_WAIT);
        m.subjects[k].3 = (i32::from(r) % i32::from(half)) as i16 + half;
        Ok(())
    }
}

pub fn point_in_polygon(p: (i32, i32), poly: &[(i32, i32)], closed: bool) -> bool {
    let n = poly.len();
    let end = if closed { n + 1 } else { n };
    for k in 1..end {
        let a = poly[k - 1];
        let b = poly[if k >= n { k - n } else { k }];
        let cross = (b.1 - a.1)
            .wrapping_mul(p.0 - a.0)
            .wrapping_sub((p.1 - a.1).wrapping_mul(b.0 - a.0));
        if cross < 0 {
            return false;
        }
    }
    true
}

impl TflSim {
    fn endgame_action(&mut self, i: usize) -> Result<u32, Stop> {
        not_read_by_tfl(&[b"dark", b"team"]);
        if self.winning_team == 0 {
            return Ok(0);
        }
        let a = self.act();
        let light = find(a, i, 2, b"ligh", 0).is_some();
        let fail = find(a, i, 2, b"fail", 0).is_some();
        let noce = find(a, i, 2, b"noce", 0).is_some();
        self.winning_team = if light { 0 } else { 1 };
        self.force_player_statistics_update()?;
        self.end_game();
        let g = &mut self.map_actions.game;
        if light {
            g.end_reason = if fail { 2 } else { 1 };
        }
        if noce {
            let team = self.winning_team;
            for m in self.monsters.iter_mut() {
                if m.identifier != 0 && m.team as i8 as i16 == team {
                    m.flags |= 0x200_0000;
                }
            }
        }
        Ok(0)
    }
}
