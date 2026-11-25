use crate::draw_site;
use crate::groups::distance;
use crate::trig::arctangent;
use crate::{Stop, TflSim};

pub(crate) fn facing_window_bias(facing: u16) -> (i32, i32) {
    let a = (i32::from(facing) + 0x2000) & 0xffff;
    let v = (a & 0x3fff) * 2 + 0x4000;
    match a >> 14 {
        0 => (0x4000, 0x10000 - v),
        1 => (v, 0x4000),
        2 => (0xc000, v),
        _ => (0x10000 - v, 0xc000),
    }
}

pub(crate) fn clamp_window(
    centre: (i32, i32),
    bias: (i32, i32),
    size: i32,
    cw: i32,
    ch: i32,
) -> (i32, i32) {
    let mut x0 = (centre.0 >> 10) - ((bias.0 * size) >> 16);
    let mut y0 = (centre.1 >> 10) - ((bias.1 * size) >> 16);
    if x0 < 0 {
        x0 = 0;
    } else if cw - size < x0 {
        x0 = cw - size;
    }
    if y0 < 0 {
        y0 = 0;
    } else if ch - size < y0 {
        y0 = ch - size;
    }
    (x0, y0)
}

pub(crate) fn window_ahead(
    facing: u16,
    centre: (i32, i32),
    size: i32,
    cw: i32,
    ch: i32,
) -> (i32, i32) {
    clamp_window(centre, facing_window_bias(facing), size, cw, ch)
}

#[derive(Clone, Copy)]
enum TargetSearch {
    Ahead,
    AheadExcluding(i16),
    NearTarget(i16),
}

impl TflSim {
    pub(crate) fn initialize_sight(&mut self) -> Result<(), Stop> {
        let local = self.local_team()?;
        for _ in 0..2 {
            self.sight.swap(self.game_options);
            for k in 0..self.monsters.len() {
                let m = &self.monsters[k];
                if m.identifier == 0 || Some(m.team) != local || m.flags & 0x1000_0000 != 0 {
                    continue;
                }
                let s = self.data.def(b"mons", m.definition)?.i16(0x180);
                if s == -1 {
                    continue;
                }
                let o = &self.objects[m.object];
                let s = if o.flags & 0x80 != 0 { 2 } else { s as usize };
                let (cx, cy) = (o.x >> 10, o.y >> 10);
                self.sight.mark(s, cx, cy);
            }
        }
        Ok(())
    }

    pub(crate) fn update_monsters_first_loop(&mut self) -> Result<(), Stop> {
        let mut s = (self.tick & 0x3f) as usize;
        while s < self.monsters.len() {
            if s == 0 {
                self.sight.swap(self.game_options);
            }
            let m = &self.monsters[s];
            if m.identifier != 0 {
                let local = self.local_team()?;
                if Some(m.team) == local && m.flags_44 & 4 == 0 {
                    let k = self.data.def(b"mons", m.definition)?.i16(0x180);
                    if k != -1 {
                        let o = &self.objects[m.object];
                        let k = if o.flags & 0x80 != 0 { 2 } else { k as usize };
                        let (cx, cy) = (o.x >> 10, o.y >> 10);
                        self.sight.mark(k, cx, cy);
                    }
                }
            }
            let m = &self.monsters[s];
            if m.identifier != 0 && m.flags & 0x4000_0000 == 0 {
                let object = m.object;
                self.objects[object].flags &= !0x0200_0000;
                let m = &self.monsters[s];
                if m.team as i8 as i16 == self.winning_team
                    && m.flags & 0x0200_0000 == 0
                    && matches!(m.action, 1 | 3 | 4 | 6)
                {
                    let r = i32::from(self.rng.draw(draw_site::WINNER_CELEBRATION));
                    self.set_monster_action(s, 2, (((r * 0x14) >> 16) + 0x1e) as u8)?;
                }
                if matches!(self.monsters[s].action, 3 | 6) {
                    self.monsters[s].flags |= 0x20;
                }
                let m = &self.monsters[s];
                if m.action != 2
                    && m.action != 7
                    && m.flags & 0x01d0_0006 == 0
                    && m.flags_44 & 0x10 == 0
                {
                    let t = self.find_target(s)?;
                    if t != -1 {
                        self.try_and_switch_target(s, t as usize, 10)?;
                    }
                }
            }
            s += 64;
        }
        Ok(())
    }

    pub(crate) fn find_target(&mut self, s: usize) -> Result<i16, Stop> {
        self.find_target_in(s, TargetSearch::Ahead)
    }

    pub(crate) fn find_target_excluding(&mut self, s: usize, exclude: i16) -> Result<i16, Stop> {
        self.find_target_in(s, TargetSearch::AheadExcluding(exclude))
    }

    pub(crate) fn find_target_near_target(&mut self, s: usize) -> Result<i16, Stop> {
        let t = self.monsters[s].target;
        self.find_target_in(s, TargetSearch::NearTarget(t))
    }

    fn find_target_in(&mut self, s: usize, search: TargetSearch) -> Result<i16, Stop> {
        let (near_target, exclude, mode) = match search {
            TargetSearch::Ahead => (false, -1, 0),
            TargetSearch::AheadExcluding(e) => (false, e, i32::from(e != -1)),
            TargetSearch::NearTarget(t) => (true, t, 2),
        };
        let (m_def, m_object, m_flags, m_flags_44, m_target, m_team, m_palette) = {
            let m = &self.monsters[s];
            (
                m.definition,
                m.object,
                m.flags,
                m.flags_44,
                m.target,
                m.team,
                m.palette,
            )
        };
        let (range_84, range_7c, ranged_146) = {
            let mons = self.data.def(b"mons", m_def)?;
            (mons.i16(0x84), mons.i16(0x7c), mons.i16(0x146))
        };
        let (me_x, me_y, me_facing, me_flags) = {
            let me = &self.objects[m_object];
            (me.x, me.y, me.facing, me.flags)
        };
        let mut current = (m_flags & 8 != 0).then_some(m_target as usize);
        let size = if near_target {
            6
        } else if current.is_none() {
            (i32::from(range_84) >> 10) + 1
        } else {
            0xb
        };
        let centre = if near_target {
            let t = current.ok_or_else(|| {
                Stop::new(draw_site::TARGET_MODE_2_NONE, "mode 2 without a target")
            })?;
            let o = &self.objects[self.monsters[t].object];
            (o.x, o.y)
        } else {
            (me_x, me_y)
        };
        if ranged_146 == 1 && m_flags & 4 == 0 && m_flags_44 & 0x40 != 0 {
            current = None;
        }
        if size <= 0 {
            return Ok(-1);
        }
        let (u, w) = if near_target {
            (0x8000, 0x8000)
        } else {
            let facing = match current {
                None => me_facing,
                Some(t) => {
                    let o = &self.objects[self.monsters[t].object];
                    arctangent(o.x - me_x, o.y - me_y)
                }
            };
            facing_window_bias(facing)
        };
        let (cw, ch) = (
            self.data.mesh.cell_width as i32,
            self.data.mesh.cell_height as i32,
        );
        let (x0, y0) = clamp_window(centre, (u, w), size, cw, ch);
        let tick = self.tick;
        crate::tc2_trace::log(|| {
            format!(
                "TS {tick} {s} {mode} {size} {u} {w} {} {} {exclude}",
                centre.0, centre.1
            )
        });
        let base = self.cell_index(x0 << 10, y0 << 10) as i32;
        let mut best: Option<(i16, u32, i32)> = None;
        for row in 0..size {
            for col in 0..size {
                let cell = (base + cw * row + col) as usize;
                let mut o = self.cell_heads.get(cell).copied().unwrap_or(-1);
                while o >= 0 {
                    let ob = &self.objects[o as usize];
                    let next = ob.next_in_cell;
                    if ob.flags & 0x200 == 0 && ob.kind == 2 && ((me_flags ^ ob.flags) & 0x80) == 0
                    {
                        let other = ob.owner as usize;
                        let om = &self.monsters[other];
                        let candidate = if om.team == m_team {
                            (om.flags & 4 == 0
                                && om.flags & 8 != 0
                                && om.flags & 0x8_0000 == 0
                                && om.palette == m_palette)
                                .then_some(om.target)
                        } else {
                            Some(other as i16)
                        };
                        if let Some(c) = candidate
                            && c != -1
                            && c != exclude
                            && c as usize != s
                        {
                            let cm = &self.monsters[c as usize];
                            let co = &self.objects[cm.object];
                            let d = distance((co.x, co.y), centre);
                            let mut score = 0;
                            if let Some(t) = current
                                && self.monsters[t].definition == cm.definition
                            {
                                score = 1;
                            }
                            if (d as i32) < i32::from(range_7c) {
                                score += 1;
                            }
                            if (score != 0 || current.is_none()) && cm.target as usize == s {
                                score += 1;
                            }
                            crate::tc2_trace::log(|| {
                                format!("TK {tick} {s} {other} {c} {d} {score}")
                            });
                            let better = match best {
                                None => true,
                                Some((_, bd, bs)) => (bs == score && d < bd) || bs < score,
                            };
                            if better && cm.flags_44 & 0x20 == 0 {
                                crate::tc2_trace::log(|| format!("TB {tick} {s} {c} {d} {score}"));
                                best = Some((c, d, score));
                            }
                        }
                    }
                    o = next;
                }
            }
        }
        Ok(best.map_or(-1, |b| b.0))
    }
}
