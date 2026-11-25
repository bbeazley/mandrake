use mandrake_common::game_options::{COOPERATIVE, tfl};

use crate::draw_site;
use crate::projectiles::GroupRequest;
use crate::{Stop, TflSim};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DamageDef {
    pub kind: i16,
    pub flags: u16,
    pub amount: i16,
    pub amount_range: i16,
    pub radius: i16,
    pub radius_range: i16,
    pub growth: i16,
    pub force: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explosion {
    pub identifier: i16,
    pub radius: i16,
    pub max_radius: i16,
    pub def: DamageDef,
    pub ctx: DamageContext,
    pub objects: Vec<(usize, i16, i16)>,
}

#[derive(Debug, Clone)]
pub struct Explosions {
    pub list: Vec<Option<Explosion>>,
    pub pool: crate::objects::DatumPool,
}

impl Default for Explosions {
    fn default() -> Self {
        Self {
            list: vec![None; 0x10],
            pool: crate::objects::DatumPool::new(0x10),
        }
    }
}

impl DamageDef {
    pub fn read(d: &crate::data::Def, o: usize) -> Self {
        Self {
            kind: d.i16(o),
            flags: d.u16(o + 2),
            amount: d.i16(o + 4),
            amount_range: d.i16(o + 6),
            radius: d.i16(o + 8),
            radius_range: d.i16(o + 0xa),
            growth: d.i16(o + 0xc),
            force: d.i16(o + 0xe),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageContext {
    pub attacker_palette: i16,
    pub attacker: i16,
    pub attacker_id: i16,
    pub group: Option<mandrake_common::FourCC>,
    pub point: [i32; 3],
    pub velocity: [i16; 3],
    pub scale: i16,
    pub force: i16,
    pub amount: i16,
    pub flags: u32,
    pub record: Option<(usize, i16)>,
}

impl TflSim {
    pub(crate) fn damage_object(
        &mut self,
        hit: Option<usize>,
        def: &DamageDef,
        ctx: &mut DamageContext,
    ) -> Result<(), Stop> {
        if def.flags & 2 != 0 {
            return self.new_explosion(def, ctx);
        }
        let Some(o) = hit else {
            return Ok(());
        };
        if self.objects[o].flags & 0x200 != 0 {
            return Ok(());
        }
        ctx.scale = 0x100;
        self.damage_one(o, def, ctx)
    }

    fn new_explosion(&mut self, def: &DamageDef, ctx: &DamageContext) -> Result<(), Stop> {
        let (slot, id) = loop {
            let list = &self.explosions.list;
            let used = |k: usize| list.get(k).is_some_and(|e| e.is_some());
            if let Some(got) = self.explosions.pool.allocate(used) {
                break got;
            }
            self.drop_weakest_explosion()?;
        };
        let extra = if def.radius_range < 1 {
            0
        } else {
            self.rng
                .draw_scaled(draw_site::EXPLOSION_RADIUS, i32::from(def.radius_range))
                as i16
        };
        let r = def.radius.wrapping_add(extra);
        let ri = i32::from(r);
        let (x, y) = (ctx.point[0], ctx.point[1]);
        let w = self.width();
        let h = self.height_cells();
        let mut x1 = ((x + 0x3ff + ri) >> 10) as i16;
        let mut x0 = ((x - ri) >> 10) as i16;
        let mut y0 = ((y - ri) >> 10) as i16;
        let mut y1 = ((y + 0x3ff + ri) >> 10) as i16;
        let max = r.clamp(0, 0x1000);
        x0 = x0.max(0);
        if w as i16 <= x1 {
            x1 = w as i16 - 1;
        }
        y0 = y0.max(0);
        if h as i16 <= y1 {
            y1 = h as i16 - 1;
        }
        let mut objects: Vec<(usize, i16, i16)> = Vec::new();
        let mut cursor = 0usize;
        let mut cy = y0;
        while cy <= y1 {
            for cx in x0..x1 {
                let cell = self.cell_at(i32::from(cx), i32::from(cy));
                let mut k = self.cell_heads[cell];
                while k >= 0 {
                    let ob = &self.objects[k as usize];
                    let next = ob.next_in_cell;
                    if ob.flags & 0x8200 == 0 && (1u32 << (ob.kind & 0x1f)) & 0x5e != 0 {
                        let d = crate::combat::distance3(
                            (ctx.point[0], ctx.point[1], ctx.point[2]),
                            (ob.x, ob.y, ob.z),
                        ) as i32;
                        if d < i32::from(max) {
                            let entry = (k as usize, ob.identifier, d as i16);
                            if objects.len() < 0x58 {
                                objects.push(entry);
                                if objects.len() == 0x58 {
                                    cursor = 0;
                                }
                            } else if cursor < 0x58 && ob.flags & 0x40_0000 != 0 {
                                match (cursor..0x58)
                                    .find(|&j| self.objects[objects[j].0].flags & 0x40_0000 == 0)
                                {
                                    Some(j) => {
                                        objects[j] = entry;
                                        cursor = j;
                                    }
                                    None => {
                                        cursor = 0x58;
                                        let tick = self.tick;
                                        crate::tc2_trace::log(|| format!("EF {tick} {k}"));
                                    }
                                }
                            }
                        }
                    }
                    k = next;
                }
            }
            cy += 1;
        }
        self.explosions.list[slot] = Some(Explosion {
            identifier: id,
            radius: 0,
            max_radius: max,
            def: *def,
            ctx: DamageContext {
                record: None,
                ..*ctx
            },
            objects,
        });
        Ok(())
    }

    fn drop_weakest_explosion(&mut self) -> Result<(), Stop> {
        let mut best: Option<(usize, i32)> = None;
        for (k, e) in self
            .explosions
            .list
            .iter()
            .enumerate()
            .take(self.explosions.pool.high)
        {
            let Some(e) = e else { continue };
            if e.max_radius == 0 {
                return Err(Stop::new(
                    draw_site::EXPLOSION_RADIUS_ZERO,
                    "an explosion of radius 0 in a full list",
                ));
            }
            let v = (i32::from(e.radius) << 16) / i32::from(e.max_radius);
            if best.is_none_or(|(_, b)| b < v) {
                best = Some((k, v));
            }
        }
        if let Some((k, _)) = best {
            self.explosions.list[k] = None;
            let list = &self.explosions.list;
            self.explosions
                .pool
                .free(k, |j| list.get(j).is_some_and(|v| v.is_some()));
        }
        Ok(())
    }

    pub(crate) fn update_explosions(&mut self) -> Result<(), Stop> {
        for e in 0..self.explosions.list.len() {
            let (id, radius, max_radius, def, count) = match self.explosions.list[e].as_mut() {
                Some(x) => {
                    x.radius = x.radius.wrapping_add(x.def.growth);
                    (x.identifier, x.radius, x.max_radius, x.def, x.objects.len())
                }
                None => continue,
            };
            let mut gone = false;
            for k in 0..count {
                let (o, oid, d) = self.explosions.list[e].as_ref().expect("live").objects[k];
                if !(d < radius) {
                    continue;
                }
                if self
                    .objects
                    .get(o)
                    .is_some_and(|ob| ob.identifier != 0 && ob.identifier == oid)
                {
                    if max_radius == 0 {
                        return Err(Stop::new(
                            draw_site::EXPLOSION_RADIUS_ZERO,
                            "an explosion of radius 0",
                        ));
                    }
                    let s = 0x100
                        - ((i32::from(d.wrapping_sub(d >> 2)) << 8) / i32::from(max_radius)) as i16;
                    let mut ctx = self.explosions.list[e].as_ref().expect("live").ctx;
                    ctx.scale = s.max(0);
                    ctx.record = Some((e, id));
                    self.damage_one(o, &def, &mut ctx)?;
                    if let Some(Some(live)) = self.explosions.list.get_mut(e)
                        && live.identifier == id
                    {
                        live.ctx = DamageContext {
                            record: None,
                            ..ctx
                        };
                    }
                }
                match self.explosions.list[e].as_mut() {
                    Some(live) if live.identifier == id => live.objects[k].2 = 0x7fff,
                    _ => {
                        gone = true;
                        break;
                    }
                }
            }
            if !gone
                && let Some(live) = self.explosions.list[e].as_ref()
                && live.identifier == id
                && live.max_radius <= live.radius
            {
                self.explosions.list[e] = None;
                let list = &self.explosions.list;
                self.explosions
                    .pool
                    .free(e, |k| list.get(k).is_some_and(|v| v.is_some()));
            }
        }
        Ok(())
    }

    fn damage_one(
        &mut self,
        o: usize,
        def: &DamageDef,
        ctx: &mut DamageContext,
    ) -> Result<(), Stop> {
        let extra = if def.amount_range < 1 {
            0
        } else {
            self.rng
                .draw_scaled(draw_site::DAMAGE_AMOUNT, i32::from(def.amount_range))
                as i16
        };
        ctx.amount = def.amount.wrapping_add(extra);
        if def.flags & 1 != 0 {
            let v = ctx.velocity;
            let n = (i32::from(v[0]) * i32::from(v[0])
                + i32::from(v[1]) * i32::from(v[1])
                + i32::from(v[2]) * i32::from(v[2])) as u32;
            let s = (crate::groups::isqrt(n) as i16 as i32) * 0x100;
            let s = ((s + ((s >> 31) & 0xff)) >> 8) as i16;
            ctx.scale = ((i32::from(ctx.scale) * i32::from(s)) as u32 >> 8) as i16;
        }
        let a = ((i32::from(ctx.scale) * i32::from(ctx.amount)) as u32 >> 8) as i16;
        ctx.amount = a.max(0);
        ctx.force = (((i32::from(def.force) * i32::from(ctx.amount)) >> 12) as i16).min(0x100);
        match self.objects[o].kind {
            2 => {
                if def.flags & 0x100 == 0 {
                    let m = self.objects[o].owner as usize;
                    self.damage_monster(def, ctx, m)?;
                }
                Ok(())
            }
            3 | 4 => self.damage_projectile(def, ctx, o),
            1 => self.damage_scenery(def, ctx, o),
            6 => self.accelerate_flag(def, ctx, o),
            _ => Ok(()),
        }
    }

    fn damage_monster(
        &mut self,
        def: &DamageDef,
        ctx: &mut DamageContext,
        i: usize,
    ) -> Result<(), Stop> {
        if def.kind == 3 {
            return self.heal_monster(ctx, i);
        }
        if def.kind == 2 {
            return self.paralyze_monster(ctx, i);
        }
        if def.kind == 6 {
            return self.stun_monster(ctx, i);
        }
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let mut give: Option<Option<mandrake_common::FourCC>> = None;
        if matches!(def.kind, 5 | 7) {
            give = Some(mons.tag(0x130));
        }
        let mut amount = i32::from(ctx.amount);
        let mut flinch = true;
        let m = &self.monsters[i];
        let share: u16 = if m.max_vitality < ctx.amount {
            0xffff
        } else if m.max_vitality == 0 {
            return Err(Stop::new(
                draw_site::TEST_UNITS_VITALITY_ZERO,
                format!("monster {i}: max_vitality is 0"),
            ));
        } else {
            ((amount << 16) / i32::from(m.max_vitality)) as u16
        };
        if m.action == 9 && m.flags & 8 != 0 && m.target == ctx.attacker {
            let full = def.flags & 0x40 == 0;
            if !full {
                amount >>= 1;
            }
            give = Some(mons.tag(0x174));
            if full {
                ctx.flags |= 1;
                ctx.group = give.flatten();
                return Ok(());
            }
        }
        if self.monsters[i].flags_44 & 8 != 0 {
            let o = self.monsters[i].object;
            self.objects[o].flags &= 0xcfff_ffff;
            self.monsters[i].flags_44 &= 0xc7;
        }
        if self.monsters[i].action == 13 {
            let tick = self.tick;
            crate::tc2_trace::log(|| format!("SD {tick} {i} {amount}"));
            amount = amount.wrapping_add(amount);
            give = Some(self.global_group(7));
        } else if mons.u32(0) & 0x100 != 0 && self.monsters[i].vitality < (mons.i16(0x128) >> 2) {
            self.stun(i, &mons, crate::tracking::site::DAMAGE_STUN)?;
        }
        let player = self.monsters[i].player;
        if self.game_options & COOPERATIVE != 0
            && player != -1
            && self.monsters[i].flags & 0x1000_0000 == 0
            && matches!(self.difficulty, 0 | 1)
            && self.player_kind(player)? == 0
        {
            let option = self.game_options & tfl::FIXES_2000 != 0;
            amount = match (option, self.difficulty) {
                (true, 0) => amount >> 2,
                (true, _) | (false, 0) => amount >> 1,
                (false, _) => (amount >> 1) + (amount >> 2),
            };
        }
        let mut absorbed = false;
        match def.kind {
            0 => amount = (i32::from(mons.i16(0x18a)) * amount) >> 8,
            1 => amount = (i32::from(mons.i16(0x18e)) * amount) >> 8,
            8 => amount = (i32::from(mons.i16(0x18c)) * amount) >> 8,
            _ => {
                let r = self.rng.draw(draw_site::DAMAGE_MONSTER_ROLL);
                absorbed = r < mons.u16(0x78);
                if amount != 0 {
                    let k = usize::from((def.flags & 8) >> 3);
                    amount = (i32::from(mons.i16(0x186 + 2 * k)) * amount) >> 8;
                    if amount == 0 {
                        absorbed = true;
                    }
                }
            }
        }
        amount = amount.min(i32::from(self.monsters[i].vitality));
        if absorbed {
            give = Some(mons.tag(0x178));
            amount = 0;
        }
        let a = self.monsters[i].action;
        if def.flags & 0x10 != 0 && mons.i16(0x190) != 0 && a != 11 && !(13..=14).contains(&a) {
            self.knock_down(i, &mons)?;
        }
        let m = &mut self.monsters[i];
        if def.flags & 0x80 != 0 {
            m.max_vitality = m.max_vitality.wrapping_sub(amount as i16);
        }
        m.vitality = m.vitality.wrapping_sub(amount as i16);
        let dead = m.vitality < 1;
        let a = m.action;
        if a == 14 || a == 13 || (a == 11 && m.flags & 0x400_0000 != 0) {
            flinch = false;
        }
        if share < mons.u16(0x12e) {
            flinch = false;
        } else if flinch && a == 8 && m.substate as i8 != -1 {
            let k = m.substate as i8 as i16;
            let at = self.attack(i, k)?;
            if at.flags & 0x100 == 0 && def.flags & 4 == 0 {
                self.handle_monster_attack(i, k, 2)?;
            } else {
                flinch = false;
            }
        }
        let attacker = ctx.attacker;
        let valid = attacker != -1
            && self
                .monsters
                .get(attacker as usize)
                .is_some_and(|m| m.identifier != 0 && m.identifier == ctx.attacker_id);
        if valid {
            let at = attacker as usize;
            if self.monsters[i].team as i8 == self.monsters[at].team as i8 {
                if self.rng.draw(draw_site::DAMAGE_VOICE_CHOICE) & 1 != 0 {
                    self.monster_voice(draw_site::DAMAGE_VOICE, at, if dead { 14 } else { 7 })?;
                } else {
                    self.monster_voice(draw_site::DAMAGE_VOICE, i, 8)?;
                }
            } else if !dead {
                let mut mode: u8 = 2;
                let answer = flinch || self.monsters[i].action != 8;
                let ranged = mons.i16(0x146);
                let mut flee = false;
                let mut skip = false;
                if def.flags & 8 == 0 {
                    mode = 6;
                    if 1 < ranged && ranged < 5 {
                        flee = true;
                    }
                } else if self.monsters[i].flags & 0x40_0000 != 0 {
                    skip = true;
                } else {
                    let (p, q) = (
                        &self.objects[self.monsters[i].object],
                        &self.objects[self.monsters[at].object],
                    );
                    let d = crate::groups::distance((p.x, p.y), (q.x, q.y)) as i32;
                    match ranged {
                        0 => {
                            if i32::from(mons.i16(0x7c)) < d {
                                skip = true;
                            }
                        }
                        1 => {
                            let m = &mut self.monsters[i];
                            let f = m.flags;
                            if f & 8 != 0 && m.flags_44 & 0x40 != 0 && f & 4 == 0 {
                                m.flags = f & 0xffff_fff7;
                                if f & 0x10 == 0 {
                                    m.flags = f & 0xffff_fff6;
                                }
                            }
                            if i32::from(mons.i16(0x84)) < d {
                                flee = true;
                            }
                        }
                        2..=4 => flee = true,
                        _ => {}
                    }
                }
                if !skip && answer {
                    if self.monsters[i].flags & 4 != 0 {
                        flee = false;
                    }
                    if self.try_and_switch_target(i, at, mode)? {
                        if ranged == 0 && def.flags & 8 == 0 {
                            self.monsters[i].flags |= 0x2000_0000;
                        }
                        if flee {
                            self.monsters[i].flags |= 0x8_0000;
                        }
                    }
                }
            } else {
                let gain = i32::from(mons.i16(0x14a).wrapping_abs());
                let m = &mut self.monsters[at];
                m.experience = (i32::from(m.experience) + gain).min(0x7f) as i8;
                m.field_13 |= 2;
                let kind = if mons.u32(0) & 0x400 != 0 { 15 } else { 13 };
                self.monster_voice(draw_site::DAMAGE_VOICE, at, kind)?;
            }
        }
        if self.monsters[i].flags & 0x1000_0000 == 0 {
            let victim = self.monsters[i].palette;
            self.adjust_monster_scoring(ctx.attacker_palette, victim, amount, dead)?;
        }
        if !dead {
            if flinch {
                self.flinch(i, &mons)?;
            }
        } else {
            let mut kind = if share > mons.u16(0x12c) { 0x26 } else { 0x25 };
            if matches!(def.kind, 0 | 1 | 8)
                || def.flags & 0x400 != 0
                || self.monsters[i].action == 13
            {
                kind = 0x26;
            }
            self.kill_monster(i, Some(def), ctx, kind)?;
            if give.is_none() {
                return Ok(());
            }
            give = Some(None);
        }
        if let Some(g) = give {
            ctx.flags |= 1;
            ctx.group = g;
        }
        Ok(())
    }

    fn flinch(&mut self, i: usize, mons: &crate::data::Def) -> Result<(), Stop> {
        let o = self.monsters[i].object;
        self.objects[o].flags &= 0xff7f_ffff;
        let seq = mons.i16(0x14);
        self.begin_action(
            i,
            mons,
            crate::monsters::ActionTransition {
                action: 15,
                substate: 0,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: crate::tracking::site::DAMAGE_6,
                interest: 100,
                reset_order: false,
                reset_target: false,
            },
        )
    }

    pub(crate) fn kill_monster(
        &mut self,
        i: usize,
        def: Option<&DamageDef>,
        ctx: &DamageContext,
        kind: i32,
    ) -> Result<(), Stop> {
        if kind == 0x27 {
            return self.fade_monster(i);
        }
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let (a, b) = (mons.tag(0x134), mons.tag(0x124));
        let mut kind = kind;
        if kind == 0x25 && a.is_none() {
            kind = 0x26;
        }
        if kind == 0x26 && b.is_none() {
            kind = 0x25;
        }
        let group = match kind {
            0x25 => a,
            0x26 => b,
            _ => None,
        };
        let o = self.monsters[i].object;
        let mut flags = 0u32;
        if let Some(d) = def {
            if d.kind == 8 {
                flags = 8;
            }
            if d.flags & 0x200 != 0 {
                flags |= 0x20;
            }
            if d.flags & 0x400 != 0 {
                flags |= 0x80;
            }
        }
        match self.monsters[i].action {
            13 => flags |= 0x14,
            11 => flags |= 0x10,
            14 => self.objects[o].render_flags &= 0xf8,
            _ => {}
        }
        let mut r = GroupRequest {
            flags,
            group,
            source_object: o as i16,
            object: o,
            position: ctx.point,
            speed: i32::from(ctx.force),
            velocity: [0; 3],
        };
        if group.is_some() {
            self.new_projectile_group(&r)?;
        }
        if self.monsters[i].action == 13 {
            let tick = self.tick;
            crate::tc2_trace::log(|| format!("SK {tick} {i}"));
            r.group = self.global_group(8);
            if r.group.is_some() {
                self.new_projectile_group(&r)?;
            }
        }
        r.speed = r.speed.max(0x14);
        if let Some(p) = mons.tag(0x17c) {
            while self.monsters[i].ammunition > 0 {
                self.monsters[i].ammunition -= 1;
                self.new_projectile_group_part(&r, 0x80, 0x80, Some(p), 0)?;
            }
            self.monsters[i].ammunition -= 1;
        }
        let c = self.monsters[i].carried;
        if c != -1 {
            let c = c as usize;
            let def = match self.objects.get(c) {
                Some(ob) if ob.kind == 4 && ob.sleeping.is_some() => ob.sleeping,
                _ => {
                    return Err(Stop::new(
                        draw_site::KILL_CARRYING_NON_PROJECTILE,
                        format!("monster {i} dies carrying object {c}, not a projectile asleep"),
                    ));
                }
            };
            r.source_object = c as i16;
            if let Some(k) = self.new_projectile_group_part(&r, 0x80, 0x80, def, 0)? {
                self.projectile_takes_object(k, c)?;
            }
        }
        let c = self.objects[o].carried;
        if c != -1 {
            let p = self.objects[c as usize].owner as usize;
            self.free_projectile(p);
        }
        self.remove_monster(i)
    }

    pub(crate) fn fade_monster(&mut self, i: usize) -> Result<(), Stop> {
        if self.monsters.get(i).is_none_or(|m| m.identifier == 0) {
            return Err(Stop::new(
                draw_site::FADE_MONSTER_UNUSED,
                format!("fade of monster {i}, not in use"),
            ));
        }
        let o = self.monsters[i].object;
        if self.monsters[i].action != 13 {
            let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
            if self.objects[o].flags & 0x80_0000 != 0 {
                self.objects[o].flags &= 0xff7f_ffff;
            }
            let seq = mons.i16(0x14);
            self.begin_action(
                i,
                &mons,
                crate::monsters::ActionTransition {
                    action: 15,
                    substate: 0,
                    seq,
                    speed: 0x100,
                    shape_flags: 0,
                    interest_site: crate::tracking::site::KILL_MONSTER,
                    interest: 100,
                    reset_order: false,
                    reset_target: false,
                },
            )?;
            if let Some(c) = self.copy_object(o) {
                let r = i32::from(self.rng.draw(draw_site::FADE_MONSTER));
                let limit = ((r * 0x1e) >> 16) + 0x1e;
                self.new_carried_record(true, c, 0, limit as u8);
            }
        }
        let c = self.objects[o].carried;
        if c != -1 {
            let p = self.objects[c as usize].owner as usize;
            self.free_projectile(p);
        }
        self.remove_monster(i)
    }

    #[doc(hidden)]
    pub fn force_fade(&mut self, i: usize) -> Result<(), Stop> {
        self.fade_monster(i)
    }

    pub(crate) fn remove_monster(&mut self, i: usize) -> Result<(), Stop> {
        self.monster_removed(i)?;
        let o = self.monsters[i].object;
        self.delete_object(o)?;
        let m = &mut self.monsters[i];
        if m.path != 0xff {
            let p = m.path;
            m.path = 0xff;
            self.paths.delete(p);
        }
        self.monsters[i].identifier = 0;
        let monsters = &self.monsters;
        self.monster_pool
            .free(i, |k| monsters.get(k).is_some_and(|m| m.identifier != 0));
        Ok(())
    }

    pub(crate) fn monster_removed(&mut self, i: usize) -> Result<(), Stop> {
        for k in 0..self.monsters.len() {
            if self.monsters[k].identifier == 0 {
                continue;
            }
            if self.monsters[k].guard == i as i16 {
                self.monsters[k].guard = -1;
            }
            let m = &self.monsters[k];
            if m.flags & 8 == 0 || m.target != i as i16 {
                continue;
            }
            let mut t = self.find_target_excluding(k, i as i16)?;
            if t == -1 {
                t = self.find_target_near_target(k)?;
            }
            let tick = self.tick;
            crate::tc2_trace::log(|| format!("MR {tick} {i} {k} {t}"));
            let m = &mut self.monsters[k];
            m.flags_44 &= 0xbf;
            let mut f = m.flags & 0xdf5e_fff1;
            if f & 0x10 == 0 {
                f &= 0xffff_fffe;
            }
            m.flags = f | 0x8000;
            m.target = -1;
            if t != -1 {
                self.try_and_switch_target(k, t as usize, 0)?;
                let m = &self.monsters[k];
                let (mt, mf) = (m.target, m.flags);
                crate::tc2_trace::log(|| format!("TW {tick} {k} {mt} {mf}"));
                continue;
            }
            let a = self.monsters[k].action;
            if a != 8 && a <= 9 {
                self.set_monster_action(k, 1, 0)?;
            }
            let m = &mut self.monsters[k];
            m.flags &= 0xffff_efff;
            m.waypoint_count = 0;
        }
        for p in self.projectiles.list.iter_mut() {
            if p.identifier == 0 {
                continue;
            }
            if p.owner == i as i16 {
                p.owner = -1;
            }
            if p.target == i as i16 {
                p.target = -1;
            }
        }
        Ok(())
    }

    pub(crate) fn damage_projectile(
        &mut self,
        _def: &DamageDef,
        ctx: &DamageContext,
        o: usize,
    ) -> Result<(), Stop> {
        if ctx.force < 1 {
            return Ok(());
        }
        let k = match self.objects[o].kind {
            3 => self.objects[o].owner as usize,
            _ => match self.wake_projectile(o)? {
                Some(k) => k,
                None => return Ok(()),
            },
        };
        let p = self.projectiles.list[k].clone();
        if p.flags & 1 != 0 {
            return Ok(());
        }
        let id = p.definition.ok_or_else(|| {
            Stop::new(
                draw_site::DAMAGE_PROJECTILE_NO_DEFINITION,
                "damage to a projectile without a definition",
            )
        })?;
        let def = self.data.def_arc(b"proj", id)?;
        let po = p.object;
        let to = [ctx.point[0], ctx.point[1], ctx.point[2] - 0x100];
        let ob = &self.objects[po];
        let v = crate::projectiles::line_segment_to_vector(
            [ob.x, ob.y, ob.z],
            to,
            i32::from(ctx.force),
        );
        let ob = &mut self.objects[po];
        ob.velocity[1] = ob.velocity[1].wrapping_add(v[1]);
        ob.velocity[2] = ob.velocity[2].wrapping_add(v[2]);
        ob.velocity[0] = ob.velocity[0].wrapping_add(v[0]);
        if ob.flags & 0x100 != 0 && ob.velocity[2] < 0 {
            ob.velocity[2] = ob.velocity[2].wrapping_neg();
        }
        ob.flags &= 0xffff_feef;
        if def.i16(0x68) < ctx.force
            && self.rng.draw(draw_site::DAMAGE_PROJECTILE_CHANCE) <= def.u16(0x6a)
        {
            self.detonate_projectile(Some((ctx.point, ctx.record)), ctx.force, k, None, false)?;
        }
        Ok(())
    }

    fn damage_scenery(
        &mut self,
        def: &DamageDef,
        ctx: &DamageContext,
        o: usize,
    ) -> Result<(), Stop> {
        if !(def.kind >= 0 && (def.kind < 2 || def.kind == 8)) {
            return Ok(());
        }
        let Some(id) = self.objects[o].scenery else {
            return Err(Stop::new(
                draw_site::DAMAGE_SCENERY_NO_DEFINITION,
                format!("object {o}: scenery without a definition"),
            ));
        };
        let scen = self.data.def_arc(b"scen", id)?;
        if def.flags & 0x800 == 0 && scen.u32(0) & 4 != 0 {
            return Ok(());
        }
        self.detonate_scenery(ctx, o, &scen)
    }

    fn detonate_scenery(
        &mut self,
        ctx: &DamageContext,
        o: usize,
        scen: &crate::data::Def,
    ) -> Result<(), Stop> {
        let centre = self.object_centre(o);
        let v = crate::projectiles::line_segment_to_vector(ctx.point, centre, i32::from(ctx.force));
        let ob = &mut self.objects[o];
        for (c, d) in ob.velocity.iter_mut().zip(v) {
            *c = c.wrapping_add(d);
        }
        let w = ob.velocity;
        let n = w.iter().map(|&c| i32::from(c) * i32::from(c)).sum::<i32>() as u32;
        let speed = crate::groups::isqrt(n) as i16;
        let (limit, chance) = (scen.i16(0x20), scen.u16(0x2a));
        if (speed <= limit || chance < self.rng.draw(draw_site::DETONATE_SCENERY_CHANCE))
            && (i32::from(speed) <= i32::from(limit) * 2 || chance == 0)
        {
            return Ok(());
        }
        if let Some(group) = scen.tag(0x30) {
            let r = crate::projectiles::GroupRequest {
                flags: 0,
                group: Some(group),
                source_object: o as i16,
                object: o,
                position: ctx.point,
                speed: i32::from(ctx.force),
                velocity: [0; 3],
            };
            self.new_projectile_group(&r)?;
        }
        if scen.u32(0) & 0x20 != 0 {
            self.scenery_model_state(o, scen)?;
        }
        let remains = scen.i16(0x3e);
        if remains == -1 {
            return self.delete_object(o);
        }
        self.set_shape(o, remains, 0x100, 0xf)?;
        let ob = &mut self.objects[o];
        ob.kind = 0;
        ob.flags &= 0xffff_fbff;
        Ok(())
    }

    fn scenery_model_state(&mut self, o: usize, scen: &crate::data::Def) -> Result<(), Stop> {
        let (x, y) = (self.objects[o].x, self.objects[o].y);
        let Some(m) = self.model_at(i32::from((x >> 10) as i16), i32::from((y >> 10) as i16))
        else {
            return Ok(());
        };
        let inst = self
            .model_instances
            .get(m as usize)
            .cloned()
            .ok_or_else(|| {
                Stop::new(
                    draw_site::SCENERY_MODEL_MISSING,
                    format!("object {o}: model {m} not made"),
                )
            })?;
        let p = inst.permutation.wrapping_add(scen.i16(0x48));
        let permutation = if p < 0 {
            0
        } else {
            let count = i32::from(self.data.def(b"mode", inst.mode)?.i16(0x0c));
            i32::from(p).min(count - 1)
        };
        let r = crate::model_effects::Request {
            flags: crate::model_effects::REVERSE,
            new_permutation: -1,
            permutation: i32::from(permutation as i16),
            ..Default::default()
        };
        self.new_model_effect(i32::from(inst.identifier), -1, &r)
    }

    pub(crate) fn global_group(&mut self, k: usize) -> Option<mandrake_common::FourCC> {
        const TABLE: [&[u8; 4]; 14] = [
            b"inhs", b"inhe", b"inps", b"inpe", b"inpd", b"inss", b"inse", b"insi", b"insd",
            b"inti", b"into", b"inte", b"inte", b"in01",
        ];
        let id = mandrake_common::FourCC(*TABLE[k]);
        self.data.def(b"prgr", id).is_ok().then_some(id)
    }

    pub(crate) fn monster_global_group(&mut self, i: usize, k: usize) -> Result<(), Stop> {
        let Some(id) = self.global_group(k) else {
            return Ok(());
        };
        let o = self.monsters[i].object;
        let (x, y, z) = self.object_point(o);
        let r = GroupRequest {
            flags: 0,
            group: Some(id),
            source_object: o as i16,
            object: o,
            position: [x, y, z],
            speed: 0,
            velocity: [0; 3],
        };
        self.new_projectile_group(&r)
    }

    pub(crate) fn paralyze_monster(
        &mut self,
        ctx: &mut DamageContext,
        i: usize,
    ) -> Result<(), Stop> {
        match self.monsters[i].action {
            12 => return Ok(()),
            8 => {
                let k = self.monsters[i].substate as i8 as i16;
                self.handle_monster_attack(i, k, 2)?;
            }
            _ => {}
        }
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let o = self.monsters[i].object;
        self.objects[o].flags &= 0xff7f_ffff;
        self.monsters[i].field_4a = 0x636;
        let seq = mons.i16(0x14);
        self.begin_action(
            i,
            &mons,
            crate::monsters::ActionTransition {
                action: 12,
                substate: 0xff,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: crate::tracking::site::DAMAGE_2,
                interest: 100,
                reset_order: false,
                reset_target: false,
            },
        )?;
        let (tick, attacker) = (self.tick, ctx.attacker);
        crate::tc2_trace::log(|| format!("DK {tick} {i} {attacker}"));
        self.monsters[i].guard = ctx.attacker;
        ctx.flags |= 1;
        ctx.group = None;
        Ok(())
    }

    pub(crate) fn stun_monster(&mut self, ctx: &mut DamageContext, i: usize) -> Result<(), Stop> {
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let (tick, action) = (self.tick, self.monsters[i].action);
        crate::tc2_trace::log(|| format!("KE {tick} {i} {action} {}", mons.i16(0x192)));
        if action == 13 || mons.i16(0x192) == 0 {
            return Ok(());
        }
        self.stun(i, &mons, crate::tracking::site::DAMAGE_KIND_6)?;
        let (m, attacker) = (&self.monsters[i], ctx.attacker);
        let o = &self.objects[m.object];
        crate::tc2_trace::log(|| {
            format!(
                "K6 {tick} {i} {attacker} {} {} {} {:x} {:x} {} {} {} {} {} {} {} {:x}",
                m.action,
                m.substate,
                m.field_4a,
                m.flags,
                m.flags_44,
                m.target,
                m.vitality,
                o.sequence as i8,
                o.frame,
                o.ticks_per_frame,
                o.render_flags & 7,
                o.carried,
                o.flags
            )
        });
        ctx.flags |= 1;
        ctx.group = None;
        Ok(())
    }

    pub(crate) fn monster_paralyzed(&mut self, i: usize) -> Result<(), Stop> {
        self.monster_global_group(i, 13)?;
        if self.monsters[i].field_4a < 0 {
            self.stop_moving(i)?;
        }
        Ok(())
    }

    pub(crate) fn heal_monster(&mut self, ctx: &mut DamageContext, i: usize) -> Result<(), Stop> {
        if self.monsters[i].action == 8 {
            let k = self.monsters[i].substate as i8 as i16;
            self.handle_monster_attack(i, k, 2)?;
        }
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let o = self.monsters[i].object;
        self.objects[o].flags &= 0xff7f_ffff;
        let seq = if self.monsters[i].action == 13 {
            self.objects[o].sequence
        } else {
            mons.i16(0x14)
        };
        let m = &mut self.monsters[i];
        m.flags &= 0xfbff_ffff;
        if mons.i16(0x7e) == 0 {
            m.flags |= 0x400_0000;
        }
        self.monster_global_group(i, 0)?;
        self.monsters[i].field_4a = 0x4b;
        self.begin_action(
            i,
            &mons,
            crate::monsters::ActionTransition {
                action: 11,
                substate: 0xff,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: crate::tracking::site::DAMAGE_1,
                interest: 100,
                reset_order: false,
                reset_target: false,
            },
        )?;
        let (tick, bits) = (self.tick, self.objects[o].render_flags & 7);
        crate::tc2_trace::log(|| format!("HL {tick} {i} {bits}"));
        if bits != 0 {
            self.monsters[i].flags |= 0x400_0000;
            self.knock_down_copy(i, 0, 0x4b)?;
            crate::tc2_trace::log(|| {
                let (m, ob) = (&self.monsters[i], &self.objects[o]);
                format!(
                    "HT {tick} {i} {:x} {:x} {} {}",
                    m.flags,
                    m.field_13,
                    ob.render_flags & 7,
                    ob.carried
                )
            });
        }
        self.monsters[i].guard = ctx.attacker;
        ctx.flags |= 1;
        ctx.group = None;
        Ok(())
    }

    pub(crate) fn monster_healing(&mut self, i: usize) -> Result<(), Stop> {
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let rate = i32::from(mons.i16(0x7e));
        let healer = self.monsters[i].guard;
        let m = &mut self.monsters[i];
        let mut over = false;
        if rate == 0 {
            if m.vitality >= 0 {
                let v = m.vitality.wrapping_sub(m.max_vitality / 0x4b);
                m.vitality = if v < 0 { -1 } else { v };
            } else {
                over = true;
            }
        } else if m.flags & 0x400_0000 == 0 {
            let most = i32::from(m.max_vitality) * rate;
            let cap = most >> 8;
            if i32::from(m.vitality) < cap {
                let (victim, gain) = (m.palette, cap / 0x4b);
                let healer_palette = self.healer_palette(healer);
                self.adjust_monster_scoring(healer_palette, victim, gain, false)?;
                let m = &mut self.monsters[i];
                m.vitality = m.vitality.wrapping_add(gain as i16);
                if cap <= i32::from(m.vitality) {
                    m.vitality = (most as u32 >> 8) as i16;
                }
            } else {
                over = true;
            }
        } else {
            over = true;
        }
        self.monster_global_group(i, 1)?;
        if !over || self.monsters[i].field_4a >= 0 {
            return Ok(());
        }
        if rate != 0 {
            return self.stop_moving(i);
        }
        let o = self.monsters[i].object;
        let (x, y, z) = self.object_location(1, o);
        let ctx = DamageContext {
            attacker_palette: 0,
            attacker: 0,
            attacker_id: 0,
            group: None,
            point: [x, y, z],
            velocity: [0; 3],
            scale: 0,
            force: 0,
            amount: 0,
            flags: 0,
            record: None,
        };
        let team = self.monsters[i].team as i8;
        let healer_palette = self.healer_palette(healer);
        self.adjust_monster_scoring(healer_palette, self.monsters[i].palette, 0, true)?;
        self.kill_monster(i, None, &ctx, 0x25)?;
        if healer != -1 {
            let h = healer as usize;
            if self.monsters[h].team as i8 != team {
                let gain = mons.i16(0x14a).wrapping_abs() as i8;
                let m = &mut self.monsters[h];
                m.experience = m.experience.wrapping_add(gain);
                m.field_13 |= 2;
            }
        }
        Ok(())
    }
}
