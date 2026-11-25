use crate::draw_site;
use crate::projectiles::TflProjectile;
use crate::{Stop, TflSim};

impl TflSim {
    pub(crate) fn knock_down(&mut self, i: usize, mons: &crate::data::Def) -> Result<(), Stop> {
        let o = self.monsters[i].object;
        if self.objects[o].flags & 0x80_0000 != 0 {
            self.objects[o].flags &= 0xff7f_ffff;
        }
        let seq = mons.i16(0x14);
        self.monster_global_group(i, 2)?;
        self.monsters[i].field_4a = 0x1e;
        self.begin_action(
            i,
            mons,
            crate::monsters::ActionTransition {
                action: 14,
                substate: 0x16,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: crate::tracking::site::DAMAGE_5,
                interest: 100,
                reset_order: false,
                reset_target: false,
            },
        )?;
        self.knock_down_copy(i, 2, 0x1e)
    }

    pub(crate) fn stun(
        &mut self,
        i: usize,
        mons: &crate::data::Def,
        site: u32,
    ) -> Result<(), Stop> {
        let o = self.monsters[i].object;
        if self.objects[o].flags & 0x80_0000 != 0 {
            self.objects[o].flags &= 0xff7f_ffff;
        }
        self.monster_global_group(i, 5)?;
        let seq = self.objects[o].sequence as i8 as i16;
        self.monsters[i].field_4a = 0x2d;
        self.begin_action(
            i,
            mons,
            crate::monsters::ActionTransition {
                action: 13,
                substate: 0x14,
                seq,
                speed: 0x100,
                shape_flags: 0,
                interest_site: site,
                interest: 0x19,
                reset_order: false,
                reset_target: false,
            },
        )?;
        self.objects[o].ticks_per_frame = 0;
        self.knock_down_copy(i, 1, 0x2d)
    }

    pub(crate) fn monster_stunned(&mut self, i: usize) -> Result<(), Stop> {
        if self.monsters[i].substate != 0x14 {
            return Ok(());
        }
        self.monster_global_group(i, 6)?;
        let m = &mut self.monsters[i];
        if m.field_4a < 0 {
            m.substate = 0x15;
            let tick = self.tick;
            crate::tc2_trace::log(|| format!("SE {tick} {i}"));
        }
        Ok(())
    }

    pub(crate) fn monster_knocked_down(&mut self, i: usize) -> Result<(), Stop> {
        self.monster_global_group(i, 3)?;
        let m = &self.monsters[i];
        if m.field_4a >= 0 {
            return Ok(());
        }
        match m.substate {
            0x16 => {
                let down = self.data.def(b"mons", m.definition)?.i16(0x190);
                self.monsters[i].substate = 0x17;
                let r = u32::from(self.rng.draw(draw_site::KNOCKED_DOWN));
                let t = ((r * 0xd2) >> 16) as i16 + 0x96;
                self.monsters[i].field_4a = ((i32::from(down) * i32::from(t)) >> 8) as i16;
            }
            0x17 => {
                self.knock_down_copy(i, 0, 0x1e)?;
                self.monster_global_group(i, 2)?;
                let m = &mut self.monsters[i];
                m.substate = 0x18;
                m.field_4a = 0x1e;
            }
            0x18 => self.stop_moving(i)?,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn knock_down_copy(&mut self, i: usize, bits: u8, limit: u8) -> Result<(), Stop> {
        let o = self.monsters[i].object;
        let c = self.objects[o].carried;
        if c != -1 {
            let k = self.objects[c as usize].owner as usize;
            self.free_projectile(k);
            self.objects[o].carried = -1;
        }
        if let Some(c) = self.copy_object(o) {
            match self.new_carried_record(true, c, 0, limit) {
                Some(_) => self.carry_object(o, c),
                None => self.delete_object(c)?,
            }
        }
        self.monsters[i].field_13 |= 0x20;
        let ob = &mut self.objects[o];
        ob.render_flags = ob.render_flags & 0xf8 | bits;
        Ok(())
    }

    pub(crate) fn copy_object(&mut self, o: usize) -> Option<usize> {
        let objects = &self.objects;
        let (k, _) = self
            .object_pool
            .allocate(|k| objects.get(k).is_some_and(|ob| ob.identifier != 0))?;
        let mut copy = self.objects[o].clone();
        self.objects[o].next_in_cell = k as i16;
        copy.flags &= 0xfffe_ffff;
        if k == self.objects.len() {
            self.objects.push(copy);
        } else {
            self.objects[k] = copy;
        }
        Some(k)
    }

    pub(crate) fn new_carried_record(
        &mut self,
        first: bool,
        c: usize,
        delay: i16,
        limit: u8,
    ) -> Option<usize> {
        let list = &self.projectiles.list;
        let used = |k: usize| list.get(k).is_some_and(|p| p.identifier != 0);
        let (slot, salt) = self.projectiles.pool.allocate(used)?;
        let record = TflProjectile {
            identifier: salt,
            flags: 0,
            definition: None,
            object: c,
            target: 0,
            owner_unit: None,
            owner_palette: -1,
            owner: -1,
            field_0e: 0,
            trail_ticks: 0,
            trails: 0,
            lifetime: 0,
            chance: 0,
            intensity: None,
            delay,
            stain: (0, 0),
            lightning: 0,
            lightning_id: 0,
            delay_group: None,
        };
        if slot == self.projectiles.list.len() {
            self.projectiles.list.push(record);
        } else {
            self.projectiles.list[slot] = record;
        }
        let ob = &mut self.objects[c];
        ob.sub_kind = if first { 1 } else { 2 };
        ob.sub_count = 0;
        ob.sub_limit = limit;
        ob.kind = 5;
        ob.owner = slot as i16;
        if delay != 0 && !first {
            ob.flags |= 0x200;
        }
        Some(slot)
    }

    fn carry_object(&mut self, carrier: usize, c: usize) {
        self.let_go_object(c);
        self.objects[carrier].carried = c as i16;
        self.objects[c].flags |= 0x20_0000;
    }

    pub(crate) fn let_go_object(&mut self, c: usize) {
        let (x, y) = (self.objects[c].x, self.objects[c].y);
        let cell = self.cell_at(x >> 10, y >> 10);
        let mut k = self.cell_heads[cell];
        while k >= 0 && k as usize != c {
            if self.objects[k as usize].carried == c as i16 {
                self.objects[k as usize].carried = -1;
                return;
            }
            k = self.objects[k as usize].next_in_cell;
        }
        self.unlink_object(c, cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tfl_dir() -> Option<std::path::PathBuf> {
        let dir = mandrake_common::data_dirs::tfl_dir();
        dir.join("Myth_TFL.exe").is_file().then_some(dir)
    }

    #[test]
    fn stun_trace_film() {
        let (Some(path), Some(dir)) = (std::env::var_os("STUN_FILM"), tfl_dir()) else {
            return;
        };
        let film = std::fs::read(path).unwrap();
        let mut sim = TflSim::new(&dir, &film).unwrap();
        let end = sim.data.film.tick_commands.last().map_or(0, |g| g.tick);
        let ticks: u32 = std::env::var("STUN_TICKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(end);
        let state = |s: &TflSim| -> Vec<(i16, u8, u8)> {
            s.monsters
                .iter()
                .map(|m| (m.identifier, m.action, m.substate))
                .collect()
        };
        let mut before = state(&sim);
        while sim.tick < ticks {
            let t = sim.tick;
            if let Err(s) = sim.step() {
                println!("STOP {t} {s:?}");
                break;
            }
            let now = state(&sim);
            for (k, &(id, a, s)) in now.iter().enumerate() {
                let (id0, a0, s0) = before.get(k).copied().unwrap_or((0, 0, 0));
                let same = id0 == id && id != 0;
                if a == 13 && !(same && a0 == 13) && id != 0 {
                    let m = &sim.monsters[k];
                    let ob = &sim.objects[m.object];
                    println!(
                        "ST {t} {k} {a} {s} {} {:x} {:x} {} {} {} {} {} {} {}",
                        m.field_4a,
                        m.flags,
                        m.flags_44,
                        m.target,
                        m.vitality,
                        ob.sequence as i8,
                        ob.frame,
                        ob.ticks_per_frame,
                        ob.render_flags & 7,
                        ob.carried
                    );
                }
                if same && a0 == 13 && s0 == 0x14 && a == 13 && s == 0x15 {
                    println!("SE {t} {k}");
                }
                if id0 != 0 && a0 == 13 && !same {
                    println!("SK {t} {k}");
                }
            }
            before = now;
        }
        println!("END {}", sim.tick);
    }
}
