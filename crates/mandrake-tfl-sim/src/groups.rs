use mandrake_common::FourCC;

use crate::{Stop, TflSim};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub team: i16,
    pub definition: FourCC,
    pub definition_flags: u32,
    pub centre: (i32, i32),
    pub radius: i16,
    pub cost: i16,
    pub spacing: i16,
    pub facing: u16,
    pub velocity: (i16, i16),
    pub members: Vec<usize>,
}

pub fn isqrt(v: u32) -> u32 {
    mandrake_math::engine::isqrt(v)
}

pub fn distance(a: (i32, i32), b: (i32, i32)) -> u32 {
    let (dx, dy) = (a.0.wrapping_sub(b.0), a.1.wrapping_sub(b.1));
    if dx < 0x7fff && dx > -0x8000 && dy < 0x7fff && dy > -0x8000 {
        isqrt((i64::from(dy) * i64::from(dy) + i64::from(dx) * i64::from(dx)) as u32)
    } else {
        0x7fff
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Supergroup {
    pub team: i16,
    pub first: i16,
    pub centre: (i32, i32),
    pub radius: i16,
    pub facing: u16,
    pub strength: i16,
    pub members: i16,
    pub range: i16,
    pub velocity: (i16, i16),
}

fn supergroups(
    groups: &[Group],
    objects: &[crate::TflObject],
    monsters: &[crate::TflMonster],
) -> Vec<Supergroup> {
    let pos = |slot: usize| {
        let o = &objects[monsters[slot].object];
        (o.x, o.y)
    };
    let mut taken = vec![false; groups.len()];
    let mut out = Vec::new();
    for seed in 0..groups.len() {
        if taken[seed] {
            continue;
        }
        let (team, def) = (groups[seed].team, groups[seed].definition);
        let mut s = Supergroup {
            team,
            first: groups[seed].members[0] as i16,
            centre: (0, 0),
            radius: 0,
            facing: 0,
            strength: 0,
            members: 0,
            range: 0,
            velocity: (0, 0),
        };
        taken[seed] = true;
        let mut queue = vec![seed];
        let (mut sx, mut sy, mut vx, mut vy, mut fc, mut fs) = (0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
        let mut k = 0;
        while k < queue.len() {
            let g = &groups[queue[k]];
            k += 1;
            s.members = s.members.wrapping_add(g.members.len() as i16);
            s.strength = s.strength.wrapping_add(g.cost);
            let w = i32::from(g.cost);
            sx = sx.wrapping_add(g.centre.0.wrapping_mul(w));
            sy = sy.wrapping_add(g.centre.1.wrapping_mul(w));
            vx = vx.wrapping_add(i32::from(g.velocity.0) * w);
            vy = vy.wrapping_add(i32::from(g.velocity.1) * w);
            let t = crate::trig::index(i32::from(g.facing));
            fc = fc.wrapping_add(i32::from(crate::trig::trig().cosine[t]) * w);
            fs = fs.wrapping_add(i32::from(crate::trig::trig().sine[t]) * w);
            for j in queue[k - 1] + 1..groups.len() {
                let h = &groups[j];
                if taken[j] || h.team != team || h.definition == def {
                    continue;
                }
                let (dx, dy) = (h.centre.0 - g.centre.0, h.centre.1 - g.centre.1);
                let d = isqrt(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32) as i32;
                if d - i32::from(h.radius) - i32::from(g.radius) >= 0x800 {
                    continue;
                }
                let close = g.members.iter().any(|&a| {
                    let pa = pos(a);
                    h.members.iter().any(|&b| {
                        let pb = pos(b);
                        let (x, y) = ((pa.0 - pb.0).abs(), (pa.1 - pb.1).abs());
                        (if y < x { x + (y >> 1) } else { y + (x >> 1) }) < 0x600
                    })
                });
                if close {
                    taken[j] = true;
                    if queue.len() < 0x10 {
                        queue.push(j);
                    }
                }
            }
        }
        if s.strength != 0 {
            let n = i32::from(s.strength);
            s.centre = (sx / n, sy / n);
            s.velocity = ((vx / n) as i16, (vy / n) as i16);
            s.facing = crate::trig::arctangent(fc, fs);
            for &q in &queue {
                let g = &groups[q];
                let (dx, dy) = (g.centre.0 - s.centre.0, g.centre.1 - s.centre.1);
                let d = isqrt(dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) as u32) as i32;
                let r = i32::from(g.radius) + d;
                let range = i32::from(g.spacing) + d;
                if i32::from(s.range) < range {
                    s.range = range as i16;
                }
                if r > i32::from(s.radius) {
                    s.radius = r as i16;
                }
            }
        }
        out.push(s);
    }
    out
}

impl TflSim {
    pub(crate) fn spatial_supergroups(&mut self) -> Result<Vec<Supergroup>, Stop> {
        if self.spatial_supergroups.is_none() {
            self.spatial_groups = None;
            self.spatial_groups()?;
        }
        Ok(self.spatial_supergroups.clone().unwrap_or_default())
    }

    pub(crate) fn spatial_groups(&mut self) -> Result<Vec<Group>, Stop> {
        if let Some(g) = &self.spatial_groups {
            return Ok(g.clone());
        }
        let (w, h) = (
            self.data.mesh.cell_width as i32,
            self.data.mesh.cell_height as i32,
        );
        let window = |c: i32, n: i32| (c - 1).clamp(0, (n - 3).max(0));
        let eligible = |s: &TflSim, i: usize| {
            let m = &s.monsters[i];
            m.identifier != 0 && m.flags_44 & 0x20 == 0 && s.objects[m.object].flags & 0x200 == 0
        };
        let mut taken = vec![false; self.monsters.len()];
        let mut groups = Vec::new();
        for seed in 0..self.monsters.len() {
            if taken[seed] || !eligible(self, seed) {
                continue;
            }
            let (team, def) = (self.monsters[seed].team, self.monsters[seed].definition);
            taken[seed] = true;
            let mut members = vec![seed];
            let mut k = 0;
            while k < members.len() && members.len() <= 0x1d {
                let o = &self.objects[self.monsters[members[k]].object];
                k += 1;
                let (x0, y0) = (window(o.x >> 10, w), window(o.y >> 10, h));
                for row in 0..3 {
                    for col in 0..3 {
                        let mut ob = self.cell_heads[(w * (y0 + row) + x0 + col) as usize];
                        while ob >= 0 {
                            let obj = &self.objects[ob as usize];
                            let next = obj.next_in_cell;
                            if obj.flags & 0x200 == 0 && obj.kind == 2 {
                                let j = obj.owner as usize;
                                let m = &self.monsters[j];
                                if !taken[j]
                                    && m.flags_44 & 0x20 == 0
                                    && m.team == team
                                    && m.definition == def
                                {
                                    taken[j] = true;
                                    members.push(j);
                                }
                            }
                            ob = next;
                        }
                    }
                }
            }
            let n = members.len() as i32;
            let pos: Vec<(i32, i32)> = members
                .iter()
                .map(|&m| {
                    let o = &self.objects[self.monsters[m].object];
                    (o.x, o.y)
                })
                .collect();
            let cx = pos.iter().map(|p| p.0).sum::<i32>() / n;
            let cy = pos.iter().map(|p| p.1).sum::<i32>() / n;
            let (x0, x1) = (
                pos.iter().map(|p| p.0).min().unwrap(),
                pos.iter().map(|p| p.0).max().unwrap(),
            );
            let (y0, y1) = (
                pos.iter().map(|p| p.1).min().unwrap(),
                pos.iter().map(|p| p.1).max().unwrap(),
            );
            let mut r = cx - x0;
            if r < x1 - cx {
                r = x1 - cx;
            }
            if r < cy - y0 {
                r = cy - y0;
            }
            if r < y1 - cy {
                r = y1 - cy;
            }
            let (mut vx, mut vy) = (0i32, 0i32);
            for &m in &members {
                let v = self.objects[self.monsters[m].object].velocity;
                vx += i32::from(v[0]);
                vy += i32::from(v[1]);
            }
            let (mut fc, mut fs) = (0i32, 0i32);
            for &m in &members {
                let k = crate::trig::index(i32::from(self.objects[self.monsters[m].object].facing));
                fc += i32::from(crate::trig::trig().cosine[k]);
                fs += i32::from(crate::trig::trig().sine[k]);
            }
            let facing = crate::trig::arctangent(fc, fs);
            let mons = self.data.def(b"mons", def)?;
            groups.push(Group {
                facing,
                velocity: ((vx / n) as i16, (vy / n) as i16),
                team,
                definition: def,
                definition_flags: mons.u32(0),
                centre: (cx, cy),
                radius: r as i16,
                cost: mons.i16(0x182).wrapping_mul(n as i16),
                spacing: mons.i16(0x184),
                members,
            });
        }
        self.spatial_supergroups = Some(supergroups(&groups, &self.objects, &self.monsters));
        self.spatial_groups = Some(groups.clone());
        Ok(groups)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_rounds_like_the_engine() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(20), 4);
        assert_eq!(isqrt(21), 5);
        assert_eq!(distance((0, 0), (3, 4)), 5);
        assert_eq!(distance((0, 0), (0x8000, 0)), 0x7fff);
    }
}
