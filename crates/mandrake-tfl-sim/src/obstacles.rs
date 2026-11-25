use mandrake_common::game_options::tfl;

use crate::draw_site;
use crate::groups::{distance, isqrt};
use crate::trig::{angle_difference, arctangent, index, trig};
use crate::{Stop, TflSim};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Obstacle {
    pub x: i32,
    pub y: i32,
    pub radius: i16,
    pub object: i16,
}

const CORNERS: [(i32, i32); 4] = [(0xee, 0x312), (0x312, 0xee), (0xee, 0xee), (0x312, 0x312)];

fn closest_on_segment(p: (i32, i32), a: (i32, i32), b: (i32, i32)) -> (bool, (i32, i32)) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx))) >> 10;
    if len == 0 {
        return (false, b);
    }
    let t = ((p.1 - a.1)
        .wrapping_mul(dy)
        .wrapping_add((p.0 - a.0).wrapping_mul(dx)))
        / len;
    if t < 0 {
        (false, a)
    } else if t < 0x401 {
        (true, (((t * dx) >> 10) + a.0, ((t * dy) >> 10) + a.1))
    } else {
        (false, b)
    }
}

pub(crate) fn arcsine(v: i32) -> i32 {
    if !(-0x4000..=0x4000).contains(&v) {
        return 0;
    }
    let sine = &trig().sine;
    let mut i = v >> 6;
    let mut state = 0;
    let mut prev = 0;
    loop {
        let k = if i >= 0 { i } else { i + 0x400 };
        let d = i32::from(sine[(k & 0x3ff) as usize]) - v;
        if d < 0 {
            match state {
                0 => state = 1,
                1 => i += 1,
                _ => {
                    if -d > prev {
                        i += 1;
                    }
                    state = 3;
                }
            }
        } else if d == 0 {
            state = 3;
        } else {
            match state {
                0 => state = 2,
                1 => {
                    if d > -prev {
                        i -= 1;
                    }
                    state = 3;
                }
                _ => i -= 1,
            }
        }
        prev = d;
        if state == 3 {
            return i << 6;
        }
    }
}

type Obstruction = (usize, (i32, i32), (i32, i32));

struct Avoid {
    from: (i32, i32),
    radius: i32,
    points: [Option<(i32, i32)>; 2],
    objects: [i16; 2],
    met_previous: bool,
    depth: u32,
}

impl TflSim {
    fn build_obstacles(&mut self, i: usize) -> Result<(), Stop> {
        let (def, m_object, m_flags, m_facing, m_team, m_target, m_pick_up) = {
            let m = &self.monsters[i];
            (
                m.definition,
                m.object,
                m.flags,
                m.facing,
                m.team,
                m.target,
                m.pick_up,
            )
        };
        let closed = u32::from(self.data.mons_closed_terrain(def)?);
        let (me_x, me_y, me_facing) = {
            let me = &self.objects[m_object];
            (me.x, me.y, me.facing)
        };
        let target_kind = (m_flags & 8 != 0).then(|| self.monsters[m_target as usize].definition);
        let (cw, ch) = (
            self.data.mesh.cell_width as i32,
            self.data.mesh.cell_height as i32,
        );
        let (x0, y0) = crate::targets::window_ahead(m_facing, (me_x, me_y), 8, cw, ch);
        let base = self.cell_index(x0 << 10, y0 << 10) as i32;
        self.obstacles.clear();
        for row in 0..8 {
            for col in 0..8 {
                let cell = (base + cw * row + col) as usize;
                let (cx, cy) = (x0 + col, y0 + row);
                if self.obstacles.len() < 0x40 {
                    let t = u32::from(self.cells[cell].byte_5);
                    let lo = (1 << (t & 0xf)) & closed != 0;
                    let hi = (1 << ((t >> 4) & 0xf)) & closed != 0;
                    let corner = |k: i32| CORNERS[k as usize];
                    let parity = !((cy * -2) ^ (cx * 2)) & 2;
                    let ob = match (hi, lo) {
                        (false, true) => {
                            let (dx, dy) = corner(parity | 1);
                            Some((cx * 0x400 + dx, cy * 0x400 + dy, 0xee))
                        }
                        (true, false) => {
                            let (dx, dy) = corner(parity);
                            Some((cx * 0x400 + dx, cy * 0x400 + dy, 0xee))
                        }
                        (true, true) => Some((cx * 0x400 + 0x200, cy * 0x400 + 0x200, 0x200)),
                        (false, false) => None,
                    };
                    if let Some((x, y, r)) = ob {
                        self.obstacles.push(Obstacle {
                            x,
                            y,
                            radius: r,
                            object: -1,
                        });
                    }
                }
                let mut o = self.cell_heads[cell];
                while o >= 0 {
                    let ob = &self.objects[o as usize];
                    let next = ob.next_in_cell;
                    if ob.flags & 0x200 == 0 && ob.flags & 0x400 != 0 && o as usize != m_object {
                        let include = if ob.kind == 2 {
                            let om = &self.monsters[ob.owner as usize];
                            (om.action != 3
                                || om.team != m_team
                                || ob.flags & 0x80_0000 != 0
                                || (me_facing > 0x7fff
                                    && angle_difference(
                                        i32::from(me_facing),
                                        i32::from(ob.facing),
                                    ) > 0x3fff))
                                && ob.flags & 0x80_0000 == 0
                                && om.flags_44 & 8 == 0
                                && (om.action != 1 || om.flags & 0x20 == 0)
                                && (Some(om.definition) != target_kind || om.team == m_team)
                        } else {
                            !(m_flags & 0x4000 != 0
                                && m_pick_up.0 == o
                                && ob.identifier == m_pick_up.1)
                        };
                        if include {
                            if self.obstacles.len() > 0x3f {
                                break;
                            }
                            self.obstacles.push(Obstacle {
                                x: ob.x,
                                y: ob.y,
                                radius: ob.radius,
                                object: o,
                            });
                        }
                    }
                    o = next;
                }
            }
        }
        Ok(())
    }

    fn pull_out_of_obstacles(&self, i: usize, from: (i32, i32), leg: &mut (i32, i32)) {
        let own = i32::from(self.objects[self.monsters[i].object].radius);
        let orig = *leg;
        for k in (0..=9).rev() {
            if self.monsters[i].flags & 8 != 0 || self.obstacles.is_empty() {
                return;
            }
            let inside = self.obstacles.iter().any(|ob| {
                ob.object != -1
                    && (distance((ob.x, ob.y), *leg) as i32) < i32::from(ob.radius) + 0xb4 + own
            });
            if !inside {
                return;
            }
            *leg = (
                (orig.0 - from.0) * k / 10 + from.0,
                (orig.1 - from.1) * k / 10 + from.1,
            );
        }
    }

    pub(crate) fn adjust_path_destination(&mut self, i: usize) -> Result<(), Stop> {
        let m = &self.monsters[i];
        let object = m.object;
        let pos = (self.objects[object].x, self.objects[object].y);
        if m.path == 0xff {
            self.monsters[i].leg = pos;
            return Ok(());
        }
        let (path_index, path_len, mut leg) = {
            let path = self.paths.paths[m.path as usize]
                .as_ref()
                .ok_or_else(|| Stop::new(draw_site::PATH_FREE, "a monster's path is free"))?;
            (
                path.index,
                path.points.len(),
                path.points[path.index as usize],
            )
        };
        let m = &mut self.monsters[i];
        if i32::from(path_index) == path_len as i32 - 1 {
            m.flags_44 |= 2;
        } else {
            m.flags_44 &= !2;
        }
        let (dx, dy) = (pos.0 - leg.0, pos.1 - leg.1);
        self.build_obstacles(i)?;
        self.pull_out_of_obstacles(i, pos, &mut leg);
        let m = &mut self.monsters[i];
        if dx.abs() < 0x100 && dy.abs() < 0x100 {
            m.flags &= !0x80;
            m.leg = leg;
            return Ok(());
        }
        if m.flags & 8 != 0 {
            let opt = self.game_options & tfl::FIXES_2000 != 0;
            if (dx.abs() < 0x200 && dy.abs() < 0x200 && opt) || (m.flags & 0x100 != 0 && !opt) {
                m.flags &= !0x80;
                m.leg = leg;
                return Ok(());
            }
        }
        self.adjust_path_segment(i, pos, &mut leg);
        self.monsters[i].leg = leg;
        Ok(())
    }

    fn adjust_path_segment(&mut self, i: usize, from: (i32, i32), leg: &mut (i32, i32)) -> bool {
        let (dx, dy) = (leg.0 - from.0, leg.1 - from.1);
        let len = isqrt(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32) as i32;
        if len > 0x2000 {
            *leg = ((dx * 0x2000) / len + from.0, (dy * 0x2000) / len + from.1);
        }
        let object = self.monsters[i].object;
        let mut s = Avoid {
            from,
            radius: i32::from(self.objects[object].radius),
            points: [None, None],
            objects: [-1, -1],
            met_previous: false,
            depth: 0,
        };
        if self.obstacles.is_empty() {
            return false;
        }
        let previous = self.monsters[i].field_70;
        let b0 = self.find_avoidance_point(&mut s, *leg, 0, previous);
        s.depth = 0;
        let b1 = self.find_avoidance_point(&mut s, *leg, 1, previous);
        let both = b1 && b0;
        let (Some(p0), Some(p1)) = (s.points[0], s.points[1]) else {
            return true;
        };
        let a = i32::from(arctangent(leg.0 - from.0, leg.1 - from.1));
        let a0 = i32::from(arctangent(p0.0 - from.0, p0.1 - from.1));
        let a1 = i32::from(arctangent(p1.0 - from.0, p1.1 - from.1));
        let mut first = angle_difference(a0, a) < angle_difference(a1, a);
        let m = &mut self.monsters[i];
        if both && s.met_previous {
            first = m.flags_44 & 1 != 0;
        }
        if first {
            *leg = p0;
            m.field_70 = s.objects[0];
        } else {
            *leg = p1;
            m.field_70 = s.objects[1];
        }
        if both {
            m.flags |= 0x80;
        } else {
            m.field_70 = -1;
            m.flags &= !0x80;
        }
        if first {
            m.flags_44 |= 1;
        } else {
            m.flags_44 &= !1;
        }
        both
    }

    fn find_avoidance_point(
        &mut self,
        s: &mut Avoid,
        point: (i32, i32),
        side: usize,
        previous: i16,
    ) -> bool {
        let d = s.depth;
        s.depth += 1;
        if d > 0x14 {
            return false;
        }
        let Some((k, a, b)) = self.path_segment_is_obstructed(s, point) else {
            s.points[side] = Some(point);
            return false;
        };
        let cross = (a.0 - s.from.0).wrapping_mul(point.1 - s.from.1)
            - (a.1 - s.from.1).wrapping_mul(point.0 - s.from.0);
        let object = self.obstacles[k].object;
        if previous != -1 && object == previous {
            s.met_previous = true;
        }
        s.objects[side] = object;
        let next = if (cross < 1) == (side == 0) { a } else { b };
        self.find_avoidance_point(s, next, side, previous);
        true
    }

    fn path_segment_is_obstructed(&mut self, s: &Avoid, point: (i32, i32)) -> Option<Obstruction> {
        let mut best: Option<(usize, u32, i32, i32)> = None;
        let mut inside = false;
        for (k, ob) in self.obstacles.iter().enumerate() {
            if inside {
                break;
            }
            let r = i32::from(ob.radius) + s.radius;
            let mut far = r + 0x154;
            let d = distance(point, s.from);
            let scale = if d == 0 {
                0
            } else {
                ((d.wrapping_add(r as u32).wrapping_add(0xaa)).wrapping_mul(0x10000) as i32)
                    / d as i32
            };
            let ext = (
                ((point.0 - s.from.0).wrapping_mul(scale) >> 16) + s.from.0,
                ((point.1 - s.from.1).wrapping_mul(scale) >> 16) + s.from.1,
            );
            let (on, cp) = closest_on_segment((ob.x, ob.y), s.from, ext);
            if on && (distance((ob.x, ob.y), cp) as i32) < r + 0xaa {
                let ang = i32::from(arctangent(ob.x - s.from.0, ob.y - s.from.1));
                let dob = distance((ob.x, ob.y), s.from) as i32;
                let half = if dob < far && {
                    far = r + 0xb4;
                    dob < far
                } {
                    inside = true;
                    0x4000
                } else {
                    arcsine((far << 14) / dob)
                };
                if best.is_none_or(|(_, h, _, _)| h < (half as u32 & 0xffff)) || inside {
                    best = Some((k, half as u32 & 0xffff, ang, half));
                }
            }
        }
        let (k, _, ang, half) = best?;
        let ob = self.obstacles[k];
        let r = s.radius + i32::from(ob.radius);
        let near = r + 0xaa;
        let far = r + 0x154;
        let dob = distance((ob.x, ob.y), s.from);
        let mut len = s.radius as u32;
        if near < dob as i32 {
            let rr = if (dob as i32) < far {
                near * near
            } else {
                far * far
            };
            len = isqrt(dob.wrapping_mul(dob).wrapping_sub(rr as u32));
        }
        let l = len as i32 + 0x100;
        let t = trig();
        let k1 = index(ang + half);
        let a = (
            ((i32::from(t.cosine[k1]) * l) >> 14) + s.from.0,
            ((i32::from(t.sine[k1]) * l) >> 14) + s.from.1,
        );
        let k2 = index(ang - half);
        let b = (
            ((i32::from(t.cosine[k2]) * l) >> 14) + s.from.0,
            ((i32::from(t.sine[k2]) * l) >> 14) + s.from.1,
        );
        if inside {
            self.obstacles[k].radius = 0;
        }
        Some((k, a, b))
    }
}
