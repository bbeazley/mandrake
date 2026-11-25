use mandrake_common::game_options::{COOPERATIVE, tfl};

use crate::combat::Attack;
use crate::draw_site;
use crate::projectiles::ProjectileRequest;
use crate::{Stop, TflSim};
use mandrake_common::FourCC;

fn cos_sin(angle: i32) -> (i32, i32) {
    let t = crate::trig::trig();
    let k = crate::trig::index(angle);
    (i32::from(t.cosine[k]), i32::from(t.sine[k]))
}

impl TflSim {
    pub(crate) fn object_key_point(
        &mut self,
        o: usize,
        sequence: i32,
        frame: i32,
        facing: u16,
    ) -> Result<(i32, i32, i32), Stop> {
        let collection = self.objects[o].collection;
        let h = *mandrake_tag::get_sequence(self.data.sequences(collection)?, sequence)
            .ok_or_else(|| {
                Stop::new(
                    draw_site::KEY_POINT_NO_SEQUENCES,
                    "a collection without sequences",
                )
            })?;
        let frame = if frame == -1 {
            i32::from(h.key_frame)
        } else {
            frame
        };
        let points = self.data.key_points(collection)?;
        let s = usize::try_from(sequence)
            .ok()
            .filter(|&s| s < points.len())
            .unwrap_or(0);
        let kp = points
            .get(s)
            .and_then(|f| f.get(usize::try_from(frame).ok()?))
            .copied()
            .ok_or_else(|| {
                Stop::new(
                    draw_site::KEY_POINT_FRAME,
                    format!("sequence {sequence} frame {frame}"),
                )
            })?;
        let ob = &self.objects[o];
        let scale = i32::from(ob.scale);
        let x = (i32::from(kp[0]) * scale) >> 8;
        let mut y = (i32::from(kp[1]) * scale) >> 8;
        if ob.flags & 0x2_0000 != 0 {
            y = -y;
        }
        let (c, s) = cos_sin(-i32::from(facing));
        Ok((
            ((c * x + s * y) >> 14) + ob.x,
            ((c * y - s * x) >> 14) + ob.y,
            ob.z + ((i32::from(kp[2]) * scale) >> 8),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn aim_projectile(
        &mut self,
        a: &Attack,
        me: (i32, i32, i32),
        origin: (i32, i32, i32),
        target: &mut (i32, i32, i32),
        speed: i16,
        high: bool,
    ) -> Result<(bool, i16, u16), Stop> {
        let proj = a.projectile.ok_or_else(|| {
            Stop::new(
                draw_site::AIM_WITHOUT_PROJECTILE,
                "aim without a projectile",
            )
        })?;
        let def = self.data.def_arc(b"proj", proj)?;
        let pf = def.u32(0);
        let gravity = match def.tag(0x1c) {
            Some(o) => self.data.def(b"obje", o)?.i16(0),
            None => 0,
        };
        let v = (i32::from(def.i16(0x20)) * i32::from(speed)) >> 8;
        let origin = if pf & 0x2000 != 0 { me } else { origin };
        let (dx, dy, dz) = (
            target.0 - origin.0,
            target.1 - origin.1,
            target.2 - origin.2,
        );
        let h = crate::groups::isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32)
            as i32;
        let mut ok = false;
        let (time, elevation);
        if pf & 0x2000 == 0 && gravity < 0 {
            let g = i32::from(gravity);
            let b = g.wrapping_mul(dz).wrapping_add(v.wrapping_mul(v));
            let nb = b.wrapping_neg();
            let disc = nb.wrapping_mul(nb).wrapping_sub(
                (h.wrapping_mul(h).wrapping_add(dz.wrapping_mul(dz)))
                    .wrapping_mul(g)
                    .wrapping_mul(g),
            );
            let mut reached = None;
            if disc >= 0 {
                let r = crate::groups::isqrt(disc as u32) as i32;
                let root = if high {
                    r.wrapping_add(b)
                } else {
                    r.wrapping_add(nb).wrapping_neg()
                };
                let q = root.wrapping_shl(1) / g.wrapping_mul(g);
                if q >= 0 {
                    let t = f64::from(q).sqrt() as i32;
                    let e = if t != 0 {
                        let n = (dz - ((g * t * t) >> 1)).wrapping_mul(0x4000);
                        crate::obstacles::arcsine(n / (t * v)) as u16
                    } else {
                        0
                    };
                    reached = Some((t as i16, e));
                }
            }
            match reached {
                Some((t, e)) => {
                    time = t;
                    elevation = e;
                    ok = true;
                }
                None => {
                    let e = (u32::from(crate::trig::arctangent(-dz, h)) >> 1) as u16;
                    elevation = e;
                    time = 0;
                    let (c, s) = cos_sin(i32::from(e));
                    let k = (((s * v) >> 14) * ((c * v) >> 14) * -2) / g;
                    let (c, s) = cos_sin(i32::from(crate::trig::arctangent(dx, dy)));
                    target.0 = ((c * k) >> 14) + origin.0;
                    target.1 = ((s * k) >> 14) + origin.1;
                }
            }
        } else {
            elevation = crate::trig::arctangent(h, dz);
            time = if pf & 0x2000 == 0 && v != 0 {
                let d = crate::groups::isqrt(
                    dz.wrapping_mul(dz)
                        .wrapping_add(dx.wrapping_mul(dx))
                        .wrapping_add(dy.wrapping_mul(dy)) as u32,
                ) as i32;
                (d / v) as i16
            } else {
                0
            };
            ok = true;
        }
        if pf & 0x2000 != 0 && a.flags & 0x40 == 0 && i32::from(a.max_range) < h {
            ok = false;
        }
        Ok((ok, time, elevation))
    }

    fn objects_around(&self, cx: i32, cy: i32) -> Vec<usize> {
        let mut out = Vec::new();
        for row in 0..3 {
            for col in 0..3 {
                let cell = self.cell_at(cx + col, cy + row);
                let mut k = self.cell_heads[cell];
                while k >= 0 {
                    out.push(k as usize);
                    k = self.objects[k as usize].next_in_cell;
                }
            }
        }
        out
    }

    fn line_of_fire(
        &self,
        me: usize,
        from: (i32, i32, i32),
        facing: u16,
        elevation: u16,
    ) -> Option<usize> {
        let (c, s) = cos_sin(i32::from(facing));
        let (ec, es) = cos_sin(i32::from(elevation));
        let ec = if ec == 0 { 1 } else { ec };
        let w = self.width();
        let h = self.height_cells();
        let cy = (from.1 >> 10).clamp(1, h - 2);
        let cx = (from.0 >> 10).clamp(1, w - 2);
        let mut best: Option<(usize, i32)> = None;
        for k in self.objects_around(cx - 1, cy - 1) {
            let o = &self.objects[k];
            if o.flags & 0x200 != 0 || o.flags & 0x400 == 0 || k == me {
                continue;
            }
            let (ox, oy) = (o.x - from.0, o.y - from.1);
            let along = (oy * s + ox * c) >> 14;
            if !(0 < along && along < 0x400) {
                continue;
            }
            let reach = i32::from(o.radius) + 0xaa;
            if oy * oy - along * along + ox * ox >= reach * reach {
                continue;
            }
            let z = ((along - i32::from(o.radius)) * es) / ec + from.2;
            if i32::from(o.bottom) - 0xaa + o.z <= z
                && z <= i32::from(o.top) + 0xaa + o.z
                && best.is_none_or(|(_, b)| along < b)
            {
                best = Some((k, along));
            }
        }
        best.map(|b| b.0)
    }

    fn clear_of_friends(&self, p: (i32, i32, i32), team: i8) -> bool {
        let w = self.width();
        let h = self.height_cells();
        let cx = ((p.0 >> 10) - 1).clamp(0, w - 3);
        let cy = ((p.1 >> 10) - 1).clamp(0, h - 3);
        let (mut friends, mut others) = (0i32, 0i32);
        for k in self.objects_around(cx, cy) {
            let o = &self.objects[k];
            if o.flags & 0x200 != 0 || o.kind != 2 {
                continue;
            }
            let m = &self.monsters[o.owner as usize];
            if m.team as i8 == team {
                friends += i32::from(m.vitality);
            } else {
                others += i32::from(m.vitality);
            }
        }
        others >= friends * 3
    }

    fn step_aside(&mut self, i: usize, obstacle: usize) -> Result<(), Stop> {
        let mo = self.monsters[i].object;
        let (b, me) = (&self.objects[obstacle], &self.objects[mo]);
        let a = i32::from(crate::trig::arctangent(b.x - me.x, b.y - me.y));
        let mut d = ((a - i32::from(self.monsters[i].facing)) & 0xffff) as u32 as i32;
        if d > 0x8000 {
            d -= 0x10000;
        }
        let f = if d < 1 { a + 0x4000 } else { a - 0x4000 };
        let reach = i32::from(b.radius) + 0xaa + i32::from(me.radius);
        let (mx, my) = (me.x, me.y);
        let m = &mut self.monsters[i];
        m.facing = f as u16;
        m.flags = (m.flags & 0xffff_ff7f) | 0x4_0000;
        m.leg_distance = reach as i16;
        let (c, s) = cos_sin(f);
        m.leg = (((c * reach) >> 14) + mx, ((s * reach) >> 14) + my);
        self.set_monster_action(i, 3, 0)
    }

    pub(crate) fn experience_level(&mut self, i: usize) -> Result<i32, Stop> {
        let m = &self.monsters[i];
        let mons = self.data.def(b"mons", m.definition)?;
        let mut v = i32::from(m.experience);
        if mons.u32(0) & 0x20 != 0 {
            v += (i32::from(mons.i16(0x128)) - i32::from(m.vitality)) >> 8;
        }
        Ok(v.min(5))
    }

    pub(crate) fn try_monster_attack(&mut self, i: usize) -> Result<bool, Stop> {
        let k = self.choose_attack(i, 1)?;
        let f = self.monsters[i].flags;
        crate::tc2_trace::log(|| {
            let m = &self.monsters[i];
            format!("TA {} {i} {k} {f:#x} {} {}", self.tick, m.target, m.action)
        });
        self.monsters[i].flags = f & 0xfffe_ffff;
        if f & 0x2_0000 == 0 {
            let begun = if k != -1 {
                self.handle_monster_attack(i, k, 0)?
            } else {
                false
            };
            let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
            let range = mons.i16(0x122);
            let extra = if range < 1 {
                0
            } else {
                self.rng
                    .draw_scaled(draw_site::MONSTER_ATTACK_RANGE, i32::from(range))
                    as i16
            };
            let mut c = mons.i16(0x120).wrapping_add(extra);
            self.monsters[i].field_4a = c;
            let player = self.monsters[i].player;
            if self.game_options & COOPERATIVE != 0
                && player != -1
                && matches!(self.difficulty, 0 | 1)
                && self.player_kind(player)? == 1
                && self.game_options & tfl::FIXES_2000 != 0
            {
                match self.difficulty {
                    0 => c = c.wrapping_add(60),
                    1 => c = c.wrapping_add(30),
                    _ => {}
                }
                self.monsters[i].field_4a = c;
            }
            if begun {
                let m = &mut self.monsters[i];
                m.flags_44 &= 0xbf;
                m.flags &= 0xffef_ffff;
                return Ok(true);
            }
            let m = &mut self.monsters[i];
            let f = m.flags;
            m.flags = f & 0xdfff_ffff;
            if m.action == 6 {
                m.flags = (f & 0xdfff_ff7f) | 0x20;
            }
        }
        self.monsters[i].flags_44 |= 0x40;
        Ok(false)
    }

    pub(crate) fn handle_monster_attack(
        &mut self,
        i: usize,
        k: i16,
        mode: i32,
    ) -> Result<bool, Stop> {
        if k == -1 {
            return Ok(false);
        }
        let a = self.attack(i, k)?;
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let mo = self.monsters[i].object;
        let mut go = false;
        let m = &self.monsters[i];
        let mut blind = !((m.flags & 8 == 0 || m.target as usize != i) && a.flags & 0x40 == 0);
        if self.monsters[i].flags_44 & 8 != 0 {
            self.objects[mo].flags &= 0xcfff_ffff;
            self.monsters[i].flags_44 &= 0xc7;
        }
        if mode == 2 && self.monsters[i].flags & 0x2000 != 0 {
            return Ok(false);
        }
        let mut a = a;
        if a.flags & 0x20 != 0 {
            a.projectile = self.throw_carried_object(i, k)?;
        }
        if a.projectile.is_none() {
            blind = true;
        }
        if blind {
            self.monsters[i].flags &= 0xffff_7fff;
            go = true;
        }
        crate::tc2_trace::log(|| {
            format!(
                "HA {} {i} {k} {mode} {:#x} {} {}",
                self.tick,
                self.monsters[i].flags,
                i32::from(blind),
                i32::from(a.projectile.is_some())
            )
        });
        if mode == 0 && self.monsters[i].flags & 0x18 == 0 && !blind {
            return Ok(go);
        }
        let mut time: i16 = 0;
        if mode == 0 {
            let extra = if a.velocity_range < 1 {
                0
            } else {
                self.rng.draw_scaled(
                    draw_site::MONSTER_ATTACK_VELOCITY,
                    i32::from(a.velocity_range),
                ) as i16
            };
            self.monsters[i].attack_speed = a.velocity.wrapping_add(extra);
        }
        let mut facing: u16;
        let mut elevation: u16;
        let kp: (i32, i32, i32);
        let mut aim: (i32, i32, i32);
        let mut point: (i32, i32, i32);
        let aimed = mode != 2 && !blind && self.monsters[i].flags & 0x18 != 0;
        if aimed {
            let high = a.flags & 1 != 0;
            let lead = if a.flags & 8 != 0 {
                i32::from(self.monsters[i].attack_ticks)
            } else {
                0
            };
            point = self.target_location(i, (!(u32::from(a.flags) >> 2) & 1) as i32, lead)?;
            aim = point;
            let me = self.object_point(mo);
            facing = crate::trig::arctangent(point.0 - me.0, point.1 - me.1);
            let seq = if mode == 0 {
                i32::from(a.sequence)
            } else {
                i32::from(self.objects[mo].sequence as i8)
            };
            kp = self.object_key_point(mo, seq, -1, facing)?;
            if mode == 1 {
                facing = crate::trig::arctangent(aim.0 - kp.0, aim.1 - kp.1);
            }
            let speed = self.monsters[i].attack_speed;
            let (ok, t, e) = self.aim_projectile(&a, me, kp, &mut aim, speed, high)?;
            time = t;
            elevation = e;
            go = ok;
            if !go {
                let f = self.monsters[i].flags;
                self.monsters[i].flags = f | 0x1_0000;
                if a.flags & 2 != 0 && f & 0x20_0000 == 0 {
                    go = true;
                }
            }
            if mode == 1 && self.monsters[i].flags & 0x8000 != 0 {
                elevation = self.monsters[i].attack_elevation;
                facing = self.monsters[i].facing;
                go = true;
            } else if !go {
                if mode == 0 {
                    return self.attack_recovery(i, &a, go);
                }
                elevation = self.monsters[i].attack_elevation;
                facing = self.monsters[i].facing;
                go = true;
            }
        } else {
            facing = self.monsters[i].facing;
            elevation = 0;
            let seq = if mode == 0 {
                i32::from(a.sequence)
            } else {
                i32::from(self.objects[mo].sequence as i8)
            };
            kp = self.object_key_point(mo, seq, -1, facing)?;
            let (c, s) = cos_sin(i32::from(facing));
            let r = i32::from(a.max_range);
            aim = (((c * r) >> 14) + kp.0, ((s * r) >> 14) + kp.1, kp.2);
            if aim.0 >= 0
                && aim.0 < self.width() * 0x400
                && aim.1 >= 0
                && aim.1 < self.height_cells() * 0x400
            {
                aim.2 = i32::from(self.mesh_height(aim.0, aim.1)? as i16);
            }
            point = aim;
            if !go {
                if mode == 0 {
                    return self.attack_recovery(i, &a, go);
                }
                elevation = self.monsters[i].attack_elevation;
                facing = self.monsters[i].facing;
                go = true;
            }
        }
        if mode != 0 && !blind && self.game_options & tfl::FIXES_8000 != 0 {
            let dx = (kp.0 - aim.0).abs();
            let dy = (kp.1 - aim.1).abs();
            let d = if dy < dx {
                (dy >> 1) + dx
            } else {
                (dx >> 1) + dy
            };
            let r = i32::from(a.max_range);
            if r + 0x400 < d {
                let (c, s) = cos_sin(i32::from(facing));
                aim.0 = ((c * r) >> 14) + kp.0;
                aim.1 = ((s * r) >> 14) + kp.1;
                let mut z = kp.2;
                if aim.0 >= 0
                    && aim.0 < self.width() * 0x400
                    && aim.1 >= 0
                    && aim.1 < self.height_cells() * 0x400
                {
                    z = i32::from(self.mesh_height(aim.0, aim.1)? as i16);
                }
                aim.2 = z;
                point = aim;
                self.monsters[i].flags |= 0x8000;
            }
        }
        if !go {
            return self.attack_recovery(i, &a, go);
        }
        if mode == 0 {
            let team = self.monsters[i].team as i8;
            if !blind {
                let proj = a.projectile.expect("aimed with a projectile");
                let pdef = self.data.def_arc(b"proj", proj)?;
                if pdef.u32(0) & 0x2000 == 0 {
                    if let Some(hit) = self.line_of_fire(mo, kp, facing, elevation) {
                        let h = &self.objects[hit];
                        let mut aside = true;
                        if h.kind == 2 {
                            let owner = h.owner;
                            if owner == self.monsters[i].target {
                                aside = false;
                            } else if self.monsters[owner as usize].action == 3 {
                                aside = false;
                                go = false;
                            }
                        }
                        if aside {
                            self.step_aside(i, hit)?;
                            go = false;
                            self.monsters[i].flags |= 0x2_0000;
                            let h = &self.objects[hit];
                            if h.kind == 2 && self.monsters[h.owner as usize].team as i8 == team {
                                self.monster_voice(draw_site::ATTACK_FRIENDLY_HIT_VOICE, i, 9)?;
                            }
                        }
                    }
                    let f = self.monsters[i].flags;
                    if pdef.u16(0x6e) & 2 == 0 && f & 0x10 == 0 && f & 8 != 0 {
                        let t = self.monsters[i].target as usize;
                        let to = self.monsters[t].object;
                        if let Some(hit) =
                            self.line_of_fire(to, aim, facing.wrapping_sub(0x8000), elevation)
                        {
                            go = false;
                            let h = &self.objects[hit];
                            if h.kind == 2 && self.monsters[h.owner as usize].team as i8 != team {
                                go = true;
                            } else {
                                self.monsters[i].flags &= 0xffff_fffb;
                            }
                        }
                    }
                }
            }
            if a.flags & 0x400 != 0
                && (!self.clear_of_friends(aim, team) || !self.clear_of_friends(point, team))
            {
                if self.monsters[i].flags & 4 == 0 {
                    return self.attack_recovery(i, &a, false);
                }
                self.monster_voice(draw_site::ATTACK_VOICE, i, 0xb)?;
            }
            if go {
                if blind {
                    facing = self.objects[mo].facing;
                }
                let m = &mut self.monsters[i];
                m.facing = facing;
                m.attack_elevation = elevation;
                m.attack_ticks = time;
                m.flags &= 0xffff_5fff;
                self.begin_attack_turn(i, k, &mons)?;
            }
            return self.attack_recovery(i, &a, go);
        }
        if self.monsters[i].flags & 0x2000 != 0
            || self.rng.draw(draw_site::ATTACK_CHANCE) < a.chance
        {
            return self.attack_recovery(i, &a, go);
        }
        let mut speed = i32::from(self.monsters[i].attack_speed);
        let f = self.monsters[i].flags | 0x2000;
        self.monsters[i].flags = f;
        let mut q = ProjectileRequest::new(a.projectile);
        q.flags = 0x10;
        q.field_1c = 0x100;
        q.owner_monster = i as i16;
        q.source_object = mo as i16;
        q.end = [aim.0, aim.1, aim.2];
        if mode != 2 && f & 0x8000 == 0 {
            if f & 0x10 == 0 && f & 8 != 0 {
                q.target = self.monsters[i].target;
            }
            if blind {
                q.target = i as i16;
            }
        }
        q.position = [kp.0, kp.1, kp.2];
        if mode == 2 {
            let half = speed >> 1;
            if half != 0 {
                speed = (i32::from(self.rng.draw(draw_site::ATTACK_SPEED)) % half) + half;
            }
            elevation = elevation.wrapping_add(
                (0x1000 - (i32::from(self.rng.draw(draw_site::ATTACK_YAW_ERROR)) & 0x1fff)) as u16,
            );
            facing = facing.wrapping_add(
                (0x1000 - (i32::from(self.rng.draw(draw_site::ATTACK_PITCH_ERROR)) & 0x1fff))
                    as u16,
            );
            if a.flags & 0x10 != 0
                && let Some(p) = mons.tag(0x17c)
            {
                q.definition = Some(p);
                crate::tc2_trace::log(|| {
                    format!("BA {} {i} {}", self.tick, self.monsters[i].ammunition)
                });
            }
        }
        let (ec, es) = cos_sin(i32::from(elevation));
        let (fc, fs) = cos_sin(i32::from(facing));
        let horizontal = (ec * speed) >> 14;
        let mut v = [
            ((fc * horizontal) >> 14) as i16,
            ((fs * horizontal) >> 14) as i16,
            ((es * speed) >> 14) as i16,
        ];
        if a.error != 0 {
            let e = (i32::from(a.error) - i32::from(self.monsters[i].experience)).max(0);
            for (c, site) in [
                draw_site::ATTACK_ERROR_X,
                draw_site::ATTACK_ERROR_Y,
                draw_site::ATTACK_ERROR_Z,
            ]
            .into_iter()
            .enumerate()
            {
                let d = if e < 1 {
                    0
                } else {
                    ((i32::from(self.rng.draw(site)) * (e * 2 + 1)) >> 16) - e
                };
                v[c] = v[c].wrapping_add(d as i16);
            }
        }
        q.velocity = v;
        q.facing = facing;
        q.field_1a = elevation;
        let shot = if q.definition.is_some() {
            Some(self.new_projectile(&q)?)
        } else {
            None
        };
        if mode == 2 && a.flags & 0x10 != 0 && mons.tag(0x17c).is_some() {
            crate::tc2_trace::log(|| {
                format!(
                    "BP {} {i} {}",
                    self.tick,
                    shot.flatten().map_or(-1, |s| s as i64)
                )
            });
        }
        self.spend_artifact(i, &a, shot)?;
        if a.flags & 0x10 != 0 {
            let m = &mut self.monsters[i];
            m.ammunition = m.ammunition.wrapping_sub(1);
            m.field_13 |= 0x80;
        }
        let m = &mut self.monsters[i];
        let f = m.flags;
        m.flags = f & 0xffff_ffef;
        if f & 8 == 0 {
            m.flags = f & 0xffff_ffee;
        }
        self.attack_recovery(i, &a, go)
    }

    fn throw_carried_object(&mut self, i: usize, k: i16) -> Result<Option<FourCC>, Stop> {
        let c = self.monsters[i].carried;
        let Some(ob) = self.objects.get(c as usize) else {
            return Err(Stop::new(
                draw_site::THROW_NO_CARRIED_OBJECT,
                format!("monster {i}: an attack throwing the carried object, with none ({c})"),
            ));
        };
        if ob.kind != 4 {
            return Err(Stop::new(
                draw_site::THROW_CARRIED_NOT_PROJECTILE,
                format!(
                    "monster {i}: the carried object {c} is of kind {}, not a projectile asleep",
                    ob.kind
                ),
            ));
        }
        let proj = ob.sleeping;
        crate::tc2_trace::log(|| format!("TH {} {i} {k} {c}", self.tick));
        let (group, id, o) = if k == crate::combat::ARTIFACT_ATTACK {
            let id = self.monsters[i].artifact.ok_or_else(|| {
                Stop::new(
                    draw_site::THROW_NO_ARTIFACT,
                    format!("monster {i}: attack -0x45, no artifact"),
                )
            })?;
            (b"arti", id, 0x4c + 4)
        } else {
            (
                b"mons",
                self.monsters[i].definition,
                0x98 + 0x20 * k as usize + 4,
            )
        };
        self.data.set_def_tag(group, id, o, proj);
        Ok(proj)
    }

    fn spend_artifact(
        &mut self,
        i: usize,
        a: &Attack,
        shot: Option<Option<usize>>,
    ) -> Result<(), Stop> {
        let mut drop = a.flags & 0x20 != 0;
        if self.monsters[i].substate as i8 as i16 == crate::combat::ARTIFACT_ATTACK {
            let id = self.monsters[i].artifact.ok_or_else(|| {
                Stop::new(
                    draw_site::SPEND_NO_ARTIFACT,
                    format!("monster {i}: attack -0x45, no artifact"),
                )
            })?;
            let flags = self.data.def(b"arti", id)?.u32(0);
            let c = self.monsters[i].carried;
            let Some(ob) = self.objects.get_mut(c as usize) else {
                return Err(Stop::new(
                    draw_site::SPEND_NO_CARRIED_OBJECT,
                    format!("monster {i}: attack -0x45, no carried object"),
                ));
            };
            if ob.charges > 0 {
                ob.charges -= 1;
                self.monsters[i].field_13 |= 8;
            }
            if self.objects[c as usize].charges == 0 && flags & 2 != 0 {
                drop = true;
            }
        }
        if !drop {
            return Ok(());
        }
        let c = self.monsters[i].carried;
        match shot {
            None => {
                return Err(Stop::new(
                    draw_site::SPEND_ARTIFACT_WITHOUT_PROJECTILE,
                    "an artifact used up by an attack without a projectile",
                ));
            }
            Some(Some(k)) => {
                let po = self.projectiles.list[k].object;
                self.objects[po].identifier = self.objects[c as usize].identifier;
            }
            Some(None) => {}
        }
        self.delete_object(c as usize)?;
        let m = &mut self.monsters[i];
        m.carried = -1;
        m.artifact = None;
        m.field_13 |= 8;
        if m.substate as i8 as i16 == crate::combat::ARTIFACT_ATTACK {
            m.substate = 0xff;
        }
        Ok(())
    }

    fn attack_recovery(&mut self, i: usize, a: &Attack, go: bool) -> Result<bool, Stop> {
        if self.monsters[i].identifier != 0 {
            let lvl = self.experience_level(i)?;
            let m = &mut self.monsters[i];
            m.field_4a = m.field_4a.wrapping_add(
                a.recovery
                    .wrapping_sub((lvl as i16).wrapping_mul(a.recovery_per_level)),
            );
        }
        Ok(go)
    }

    fn begin_attack_turn(&mut self, i: usize, k: i16, mons: &crate::data::Def) -> Result<(), Stop> {
        let mo = self.monsters[i].object;
        self.objects[mo].flags &= 0xff7f_ffff;
        let seq = mons.i16(0x0a);
        self.begin_action(
            i,
            mons,
            crate::monsters::ActionTransition {
                action: 7,
                substate: k as u8,
                seq,
                speed: 0x100,
                shape_flags: 3,
                interest_site: crate::tracking::site::ATTACK,
                interest: 0x4b,
                reset_order: false,
                reset_target: false,
            },
        )
    }

    pub(crate) fn begin_attacking(&mut self, i: usize) -> Result<(), Stop> {
        let k = self.monsters[i].substate as i8 as i16;
        let a = self.attack(i, k)?;
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let mo = self.monsters[i].object;
        self.objects[mo].flags &= 0xff7f_ffff;
        let seq = self.attack_sequence(i, k, self.monsters[i].attack_elevation)?;
        self.monsters[i].field_16 = (a.shots & 0xff) as u8;
        self.begin_action(
            i,
            &mons,
            crate::monsters::ActionTransition {
                action: 8,
                substate: k as u8,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: crate::tracking::site::ATTACKING,
                interest: 100,
                reset_order: true,
                reset_target: false,
            },
        )?;
        let m = &mut self.monsters[i];
        if m.flags & 0x80_0000 != 0 {
            m.flags = m.flags & 0xff7f_ffff | 0x100_0000;
        }
        if let Some(p) = a.projectile {
            let pf = self.data.def(b"proj", p)?.u32(0);
            let f = self.monsters[i].flags;
            if pf & 0x2000 != 0 && f & 0x10 == 0 && f & 8 != 0 && a.flags & 0x40 == 0 {
                let t = self.monsters[i].target as usize;
                let tmons = self.data.def_arc(b"mons", self.monsters[t].definition)?;
                let o = &self.objects[mo];
                let h = *mandrake_tag::get_sequence(
                    self.data.sequences(o.collection)?,
                    i32::from(o.sequence as i8),
                )
                .ok_or_else(|| {
                    Stop::new(
                        draw_site::ATTACKING_NO_SEQUENCES,
                        "a collection without sequences",
                    )
                })?;
                let n = i32::from(h.key_frame) * i32::from(h.ticks_per_frame) + 4;
                let tm = &self.monsters[t];
                if tmons.i16(0x12) != -1
                    && tm.target == i as i16
                    && tm.action == 6
                    && tm.field_4a < 10
                    && 4 < n
                    && n < 0x1e
                {
                    self.monsters[t].field_4a += 0x1e;
                    self.begin_dodge(t, n as u8, &tmons)?;
                }
            }
        }
        Ok(())
    }

    fn begin_dodge(&mut self, t: usize, ticks: u8, mons: &crate::data::Def) -> Result<(), Stop> {
        let o = self.monsters[t].object;
        self.objects[o].flags &= 0xff7f_ffff;
        let seq = mons.i16(0x12);
        self.begin_action(
            t,
            mons,
            crate::monsters::ActionTransition {
                action: 9,
                substate: ticks,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: crate::tracking::site::DODGE,
                interest: 0x19,
                reset_order: false,
                reset_target: false,
            },
        )
    }

    fn attack_sequence(&mut self, i: usize, k: i16, elevation: u16) -> Result<i16, Stop> {
        let a = self.attack(i, k)?;
        let e = i32::from(elevation);
        if a.flags & 0x800 != 0 {
            let mut list = Vec::new();
            if a.sequence != -1 {
                list.push(a.sequence);
            }
            if a.sequences[1] != -1 {
                list.push(a.sequences[1]);
            }
            if a.sequences[0] != -1 {
                list.push(a.sequences[0]);
            }
            if list.is_empty() {
                return Ok(-1);
            }
            let r = self.rng.draw(draw_site::ATTACK_SEQUENCE) as usize;
            return Ok(list[r % list.len()]);
        }
        if 0xfff < e && e < 0xfc00 {
            let s = if e < 0x8000 {
                a.sequences[0]
            } else {
                a.sequences[1]
            };
            if s != -1 {
                return Ok(s);
            }
        }
        Ok(a.sequence)
    }

    pub(crate) fn monster_attacking(&mut self, i: usize) -> Result<(), Stop> {
        let mo = self.monsters[i].object;
        if self.objects[mo].flags & 2 != 0 {
            let k = self.monsters[i].substate as i8 as i16;
            self.handle_monster_attack(i, k, 1)?;
        }
        if self.monsters[i].identifier == 0 || self.objects[mo].flags & 4 == 0 {
            return Ok(());
        }
        let k = self.monsters[i].substate as i8 as i16;
        let f = self.monsters[i].flags;
        if k != -1 {
            let needs_none = f & 1 == 0 && self.attack(i, k)?.flags & 0x40 != 0;
            if (f & 1 != 0 || needs_none) && f & 0x8000 == 0 {
                let m = &mut self.monsters[i];
                m.field_16 = m.field_16.wrapping_sub(1);
                if (m.field_16 as i8) > 0 {
                    m.flags = f & 0xffff_dfff;
                    return Ok(());
                }
            }
        }
        self.stop_moving(i)
    }

    pub(crate) fn try_and_switch_target(
        &mut self,
        i: usize,
        t: usize,
        mode: u8,
    ) -> Result<bool, Stop> {
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let (m, tm) = (&self.monsters[i], &self.monsters[t]);
        let mut take = false;
        let f = m.flags;
        let skip = m.team as i8 == tm.team as i8 || (f & 2 != 0 && mode & 1 == 0);
        let mut decided = skip;
        if !skip {
            if f & 8 == 0 {
                take = true;
            } else {
                let cur = m.target as usize;
                let ranged = mons.i16(0x146);
                let attacking = m.flags_44 & 0x40 != 0;
                let same_kind = self.monsters[cur].definition == tm.definition;
                let compare = if ranged == 1 {
                    if attacking && f & 4 == 0 {
                        take = true;
                        false
                    } else {
                        true
                    }
                } else if attacking && mode & 4 != 0 && same_kind {
                    take = true;
                    false
                } else {
                    true
                };
                if compare {
                    let cm = &self.monsters[cur];
                    if f & 0x2000_0000 != 0
                        || (f & 4 != 0 && mode & 1 == 0 && !same_kind)
                        || tm.target != i as i16
                        || cm.target == i as i16
                    {
                        decided = true;
                    } else {
                        let me = &self.objects[m.object];
                        let (p0, pc, pt) = (
                            (me.x, me.y),
                            {
                                let o = &self.objects[cm.object];
                                (o.x, o.y)
                            },
                            {
                                let o = &self.objects[tm.object];
                                (o.x, o.y)
                            },
                        );
                        if mode & 4 == 0 {
                            let dc = crate::groups::distance(p0, pc) as i32;
                            let dt = crate::groups::distance(p0, pt) as i32;
                            take = dt < dc - 0x800;
                        } else if mons.u32(0) & 0x200 == 0 {
                            take = true;
                        } else {
                            let f0 = i32::from(me.facing);
                            let ac = crate::trig::arctangent(pc.0 - p0.0, pc.1 - p0.1);
                            let at = crate::trig::arctangent(pt.0 - p0.0, pt.1 - p0.1);
                            let dc = crate::trig::angle_difference(f0, i32::from(ac)) as i32;
                            let dt = crate::trig::angle_difference(f0, i32::from(at)) as i32;
                            take = dt < dc - 0x1000;
                        }
                        if !take {
                            decided = true;
                        }
                    }
                }
            }
        }
        if decided {
            let m = &self.monsters[i];
            return Ok(take || (m.flags & 8 != 0 && m.target == t as i16));
        }
        let m = &self.monsters[i];
        let has = m.flags & 8;
        if has != 0 && m.target == t as i16 {
            return Ok(take);
        }
        let quiet = has != 0 && m.flags_44 & 0x40 == 0;
        let tkind = self
            .data
            .def(b"mons", self.monsters[t].definition)?
            .i16(0x146);
        let flee = match mons.i16(0x146) {
            0 | 3 => tkind == 2,
            1 => false,
            2 => has == 0 || m.flags & 0x8_0000 != 0,
            4 => true,
            _ => false,
        };
        let o = m.object;
        self.objects[o].flags &= 0xff7f_ffff;
        let m = &mut self.monsters[i];
        let old = m.flags;
        m.flags_44 &= 0xbf;
        m.target = t as i16;
        m.flags = if flee {
            (old & 0xde2e_fe7f) | 0x8_8029
        } else {
            (old & 0xde26_fe7f) | 0x8029
        };
        if !quiet && mode & 2 != 0 && !flee {
            self.call_friends(i, t)?;
        }
        Ok(take)
    }

    fn call_friends(&mut self, i: usize, t: usize) -> Result<(), Stop> {
        let mons = self.data.def_arc(b"mons", self.monsters[i].definition)?;
        let r = i32::from(mons.i16(0x7a));
        let n = (r >> 10) * 2 + 1;
        let o = &self.objects[self.monsters[i].object];
        let clamp = |v: i32, size: i32| if v < 0 { 0 } else { v.min(size - n) };
        let cx = clamp((o.x - r) >> 10, self.width());
        let cy = clamp((o.y - r) >> 10, self.height_cells());
        let team = self.monsters[i].team as i8;
        let kind = mons.i16(0x146);
        for row in 0..n {
            for col in 0..n {
                let cell = self.cell_at(cx + col, cy + row);
                let mut k = self.cell_heads[cell];
                while k >= 0 {
                    let ob = &self.objects[k as usize];
                    let next = ob.next_in_cell;
                    if ob.flags & 0x200 == 0 && ob.kind == 2 {
                        let f = ob.owner as usize;
                        let fm = &self.monsters[f];
                        if fm.flags & 0x40_0000 == 0
                            && fm.team as i8 == team
                            && f != i
                            && self.data.def(b"mons", fm.definition)?.i16(0x146) == kind
                        {
                            self.try_and_switch_target(f, t, 0)?;
                        }
                    }
                    k = next;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn point_of_retreat(&self, i: usize, from: (i32, i32)) -> (i32, i32) {
        self.retreat(i, Some(from)).1
    }

    pub(crate) fn retreat(&self, i: usize, from: Option<(i32, i32)>) -> (bool, (i32, i32), u16) {
        let mut found = false;
        let me = &self.objects[self.monsters[i].object];
        let p = (me.x, me.y);
        let w = self.width();
        let h = self.height_cells();
        let cx = ((p.0 >> 10) - 3).clamp(0, (w - 7).max(0));
        let cy = ((p.1 >> 10) - 3).clamp(0, (h - 7).max(0));
        let cx = if (p.0 >> 10) - 3 < 0 { 0 } else { cx };
        let cy = if (p.1 >> 10) - 3 < 0 { 0 } else { cy };
        let (mut sx, mut sy) = (0i32, 0i32);
        let (dx, dy) = from.map_or((0, 0), |f| (p.0 - f.0, p.1 - f.1));
        if from.is_some() {
            let n =
                crate::groups::isqrt(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32)
                    as i32;
            if n != 0 {
                sx = (dx * 0x400) / n;
                sy = (dy * 0x400) / n;
                found = true;
            }
        }
        let team = self.monsters[i].team as i8;
        for row in 0..7 {
            for col in 0..7 {
                let cell = self.cell_at(cx + col, cy + row);
                let mut k = self.cell_heads[cell];
                while k >= 0 {
                    let o = &self.objects[k as usize];
                    if o.flags & 0x200 == 0 && o.flags & 0x400 != 0 && o.kind == 2 {
                        let (ex, ey) = (p.0 - o.x, p.1 - o.y);
                        let n = crate::groups::isqrt(
                            ey.wrapping_mul(ey).wrapping_add(ex.wrapping_mul(ex)) as u32,
                        ) as i32;
                        let shift = if self.monsters[o.owner as usize].team as i8 != team {
                            10
                        } else {
                            8
                        };
                        if n != 0 {
                            found = true;
                            sx = sx.wrapping_add(ex.wrapping_shl(shift) / n);
                            sy = sy.wrapping_add(ey.wrapping_shl(shift) / n);
                        }
                    }
                    k = o.next_in_cell;
                }
            }
        }
        if sx == 0 && sy == 0 {
            sx = 1;
        }
        let mut a = i32::from(crate::trig::arctangent(sx, sy));
        if from.is_some() {
            let b = i32::from(crate::trig::arctangent(dx, dy));
            let mut d = a - b;
            if 0x7fff < d {
                d = 0x10000 - d;
            }
            if d < -0x7fff {
                d += 0x10000;
            }
            if d < -0x2aaa {
                a = b - 0x2aaa;
            }
            if 0x2aaa < d {
                a = b + 0x2aaa;
            }
        }
        let (c, s) = cos_sin(a);
        let x = ((c << 12) >> 14) + p.0;
        let y = ((s << 12) >> 14) + p.1;
        (
            found,
            (
                x.clamp(0, (w * 0x400 - 0x400).max(0)),
                if y < 0 { 0 } else { y.min(h * 0x400 - 0x400) },
            ),
            a as u16,
        )
    }
}
