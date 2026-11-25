use crate::draw_site;
use crate::groups::{distance, isqrt};
use crate::map_actions::{combined, find, index_of};
use crate::trig::{angle_difference, arctangent, index, trig};
use crate::{Stop, TflSim};

pub const LEGION_INITIALISED: u32 = 1;
pub const LEGION_FALL_BACK: u32 = 2;
pub const LEGION_IN_RANGE: u32 = 4;
pub const LEGION_CASUALTIES: u32 = 0x10;
pub const LEGION_CALLED: u32 = 0x20;
pub const LEGION_CIRCLE: u32 = 0x40;

const MAX_UNITS: usize = 0x3c;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LegionGroup {
    pub offset: (i32, i32),
    pub facing_delta: u16,
    pub formation: i32,
    pub guard: i32,
    pub action: i16,
    pub units: i16,
    pub first: usize,
    pub centre: (i32, i32),
    pub mode: i16,
    pub target: (i32, i32),
    pub facing: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LegionState {
    pub flags: u32,
    pub vira: i32,
    pub atra: [i32; 2],
    pub pora: i32,
    pub vide: i32,
    pub action: i16,
    pub state: i16,
    pub vitality: i32,
    pub strength: i32,
    pub facing: u16,
    pub circle_radius: i16,
    pub podt: i32,
    pub point: (i32, i32),
    pub line: [(i32, i32); 2],
    pub count: i32,
    pub groups: [LegionGroup; 6],
}

struct Variables {
    team: i16,
    units: Vec<usize>,
    enemy: Option<crate::groups::Supergroup>,
}

pub fn closest_point(p: (i32, i32), a: (i32, i32), b: (i32, i32)) -> ((i32, i32), (i32, i32)) {
    let (dx, dy) = (b.0.wrapping_sub(a.0), b.1.wrapping_sub(a.1));
    let k = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx)) >> 10;
    if k == 0 {
        return (b, b);
    }
    let t =
        p.1.wrapping_sub(a.1)
            .wrapping_mul(dy)
            .wrapping_add(p.0.wrapping_sub(a.0).wrapping_mul(dx))
            .wrapping_div(k);
    let raw = (
        (t.wrapping_mul(dx) >> 10).wrapping_add(a.0),
        (t.wrapping_mul(dy) >> 10).wrapping_add(a.1),
    );
    let clamped = if t < 0 {
        a
    } else if t < 0x401 {
        raw
    } else {
        b
    };
    (raw, clamped)
}

fn rotate(off: (i32, i32), cos: i32, sin: i32) -> (i32, i32) {
    (
        off.0
            .wrapping_mul(cos)
            .wrapping_add(off.1.wrapping_mul(sin))
            >> 14,
        off.1
            .wrapping_mul(cos)
            .wrapping_sub(off.0.wrapping_mul(sin))
            >> 14,
    )
}

impl TflSim {
    fn legion_i32(&self, i: usize, name: &[u8; 4], kind: i16, k: usize) -> Option<i32> {
        find(&self.map_actions.actions, i, 0, name, kind)
            .filter(|p| (k as i16) < p.count)
            .map(|p| p.i32_at(k))
    }

    fn legion_i16(&self, i: usize, name: &[u8; 4], kind: i16, k: usize) -> Option<i16> {
        find(&self.map_actions.actions, i, 0, name, kind)
            .filter(|p| (k as i16) < p.count)
            .map(|p| p.i16_at(k))
    }

    fn legion_point(&self, i: usize, name: &[u8; 4], k: usize) -> Option<(i32, i32)> {
        find(&self.map_actions.actions, i, 0, name, 0xc)
            .filter(|p| (k as i16) < p.count)
            .map(|p| (p.i32_at(2 * k), p.i32_at(2 * k + 1)))
    }

    fn legion_initial(&self, i: usize) -> Option<LegionState> {
        let mut s = LegionState {
            action: i as i16,
            vira: self.legion_i32(i, b"vira", 6, 0).unwrap_or(0x6400),
            atra: [
                self.legion_i32(i, b"atra", 6, 0).unwrap_or(0x2800),
                self.legion_i32(i, b"atra", 6, 1).unwrap_or(0x3800),
            ],
            pora: self.legion_i32(i, b"pora", 8, 0).unwrap_or(0x20000),
            vide: self.legion_i32(i, b"vide", 8, 0).unwrap_or(0x10000),
            podt: self.legion_i32(i, b"podt", 6, 0).unwrap_or(0x800),
            ..LegionState::default()
        };
        let a = &self.map_actions.actions;
        let line = find(a, i, 0, b"line", 0xc).filter(|p| p.count == 2)?;
        s.line = [
            (line.i32_at(0), line.i32_at(1)),
            (line.i32_at(2), line.i32_at(3)),
        ];
        if find(a, i, 2, b"circ", 0).is_some() {
            s.flags |= LEGION_CIRCLE;
            s.circle_radius = distance(s.line[0], s.line[1]) as i16;
        }
        let mut centre = self.legion_point(i, b"cent", 0);
        let mut facing = self.legion_i16(i, b"faci", 4, 0).map(|f| f as u16);
        s.count = self.legion_i32(i, b"coun", 5, 0)?;
        if !(0..=6).contains(&s.count) {
            return None;
        }
        for k in 0..s.count as usize {
            let g = &mut s.groups[k];
            let grfa = self
                .legion_i16(i, b"grfa", 4, k)
                .map_or(facing.unwrap_or(0), |f| f as u16);
            let faci = *facing.get_or_insert(grfa);
            g.facing_delta = faci.wrapping_sub(grfa);
            g.formation = self.legion_i32(i, b"grfo", 5, k).unwrap_or(1);
            match self.legion_point(i, b"grce", k) {
                Some(p) => {
                    let c = *centre.get_or_insert(p);
                    let t = trig();
                    let x = index(i32::from(faci));
                    let (cos, sin) = (i32::from(t.cosine[x]), i32::from(t.sine[x]));
                    g.offset = rotate((p.0.wrapping_sub(c.0), p.1.wrapping_sub(c.1)), cos, sin);
                }
                None if s.count == 1 => g.offset = (0, 0),
                None => break,
            }
            g.guard = self.legion_i32(i, b"grgd", 5, k).unwrap_or(-1);
            let Some(id) = self.legion_i16(i, b"grun", 3, k) else {
                break;
            };
            g.action = index_of(&self.map_actions.actions, id).map_or(-1, |j| j as i16);
            if g.action == -1 {
                break;
            }
        }
        Some(s)
    }

    fn legion_variables(&mut self, s: &mut LegionState) -> Result<Variables, Stop> {
        let mut units: Vec<usize> = Vec::new();
        let (mut vitality, mut strength) = (0i32, 0i32);
        for k in 0..s.count as usize {
            let action = s.groups[k].action;
            if action < 0 {
                return Err(Stop::new(
                    draw_site::LEGION_GROUP_ACTION_MISSING,
                    format!(
                        "map action {}: 'legi' group {k}'s 'grun' names no action",
                        s.action
                    ),
                ));
            }
            let ids = combined(
                &self.map_actions.actions,
                action as usize,
                b"subj",
                2,
                MAX_UNITS - units.len(),
            );
            let found: Vec<usize> = self
                .monster_slots(&ids)
                .into_iter()
                .filter(|&m| self.monsters[m].action != 0xd)
                .collect();
            for &m in &found {
                vitality = vitality.wrapping_add(i32::from(self.monsters[m].vitality));
                let def = self.monsters[m].definition;
                strength =
                    strength.wrapping_add(i32::from(self.data.def(b"mons", def)?.i16(0x182)));
            }
            let g = &mut s.groups[k];
            g.first = units.len();
            g.units = found.len() as i16;
            g.centre = self.monsters_centre_spread(&found).0;
            units.extend(found);
        }
        if s.flags & LEGION_INITIALISED != 0 {
            if s.vitality <= (s.vide >> 8).wrapping_add(vitality) {
                s.flags &= !LEGION_CASUALTIES;
            } else {
                s.flags |= LEGION_CASUALTIES;
                s.vitality = vitality;
            }
        } else {
            s.vitality = vitality;
        }
        s.strength = strength;
        let team = units
            .first()
            .map_or(-1, |&m| self.monsters[m].team as i8 as i16);
        Ok(Variables {
            team,
            units,
            enemy: None,
        })
    }

    fn legion_centre_and_facing(
        &mut self,
        s: &mut LegionState,
        v: &mut Variables,
    ) -> Result<(), Stop> {
        if s.flags & LEGION_INITIALISED == 0 {
            s.facing = arctangent(
                s.line[0].0.wrapping_sub(s.line[1].0),
                s.line[0].1.wrapping_sub(s.line[1].1),
            );
            s.point = s.line[0];
        }
        let (mut nearest, mut nearest_d) = (None::<crate::groups::Supergroup>, 0i32);
        let (mut enemy, mut enemy_d) = (None::<crate::groups::Supergroup>, i32::MAX);
        for g in self.spatial_supergroups()? {
            if g.team == v.team || g.strength == 0 {
                continue;
            }
            let mut d = distance(g.centre, s.point) as i32;
            let (dx, dy) = (
                g.centre.0.wrapping_sub(s.point.0),
                g.centre.1.wrapping_sub(s.point.1),
            );
            if i32::from(g.velocity.1)
                .wrapping_mul(dy)
                .wrapping_add(i32::from(g.velocity.0).wrapping_mul(dx))
                < 0
            {
                d = d.wrapping_mul(2);
            }
            if d >= s.vira {
                continue;
            }
            let by_range = d.wrapping_sub(i32::from(g.range));
            if nearest.is_none() || by_range < nearest_d {
                nearest = Some(g);
                nearest_d = by_range;
            }
            let by_radius = d.wrapping_sub(i32::from(g.radius));
            if enemy.is_none_or(|e| {
                by_radius < enemy_d
                    || (e.strength < g.strength && by_radius.wrapping_sub(enemy_d) < s.podt)
            }) {
                enemy = Some(g);
                enemy_d = by_radius;
            }
        }
        v.enemy = enemy;
        let mut fall_back = false;
        let (point, facing) = match (nearest, enemy) {
            (None, _) | (_, None) => (
                s.line[1],
                arctangent(
                    s.line[1].0.wrapping_sub(s.line[0].0),
                    s.line[1].1.wrapping_sub(s.line[0].1),
                ),
            ),
            (Some(_), Some(e)) if s.flags & LEGION_CIRCLE != 0 => {
                let a = arctangent(
                    e.centre.0.wrapping_sub(s.line[0].0),
                    e.centre.1.wrapping_sub(s.line[0].1),
                );
                let t = trig();
                let x = index(i32::from(a));
                let r = i32::from(s.circle_radius);
                (
                    (
                        (i32::from(t.cosine[x]).wrapping_mul(r) >> 14).wrapping_add(s.line[0].0),
                        (i32::from(t.sine[x]).wrapping_mul(r) >> 14).wrapping_add(s.line[0].1),
                    ),
                    a,
                )
            }
            (Some(n), Some(_)) => {
                let (raw, mut at) = closest_point(n.centre, s.line[0], s.line[1]);
                let range = i32::from(n.range);
                if (distance(n.centre, at) as i32) < range {
                    let (bx, by) = (
                        s.line[0].0.wrapping_sub(raw.0),
                        s.line[0].1.wrapping_sub(raw.1),
                    );
                    let len =
                        isqrt(by.wrapping_mul(by).wrapping_add(bx.wrapping_mul(bx)) as u32) as i32;
                    let (ex, ey) = (
                        raw.0.wrapping_sub(n.centre.0),
                        raw.1.wrapping_sub(n.centre.1),
                    );
                    let out = isqrt(
                        range
                            .wrapping_mul(range)
                            .wrapping_sub(ey.wrapping_mul(ey))
                            .wrapping_sub(ex.wrapping_mul(ex)) as u32,
                    ) as i32;
                    if out < len && len != 0 {
                        at = (
                            out.wrapping_mul(bx).wrapping_div(len).wrapping_add(raw.0),
                            out.wrapping_mul(by).wrapping_div(len).wrapping_add(raw.1),
                        );
                    } else {
                        at = s.line[0];
                        fall_back = true;
                    }
                }
                let f = arctangent(n.centre.0.wrapping_sub(at.0), n.centre.1.wrapping_sub(at.1));
                (at, f)
            }
        };
        if fall_back {
            s.flags |= LEGION_FALL_BACK;
        } else {
            s.flags &= !LEGION_FALL_BACK;
        }
        let range = if s.state == 1 { s.atra[1] } else { s.atra[0] };
        if enemy_d < range {
            s.flags |= LEGION_IN_RANGE;
        } else {
            s.flags &= !LEGION_IN_RANGE;
        }
        let forward = s.line[1]
            .1
            .wrapping_sub(s.point.1)
            .wrapping_mul(point.1.wrapping_sub(s.point.1))
            .wrapping_add(
                s.line[1]
                    .0
                    .wrapping_sub(s.point.0)
                    .wrapping_mul(point.0.wrapping_sub(s.point.0)),
            );
        if s.state == 0 || forward > 0 {
            s.point = point;
            s.facing = facing;
        }
        Ok(())
    }

    fn legion_each(
        &mut self,
        i: usize,
        name: &[u8; 4],
        f: fn(&mut Self, usize) -> Result<(), Stop>,
    ) -> Result<bool, Stop> {
        let Some(ids) = find(&self.map_actions.actions, i, 0, name, 3).map(|p| p.shorts()) else {
            return Ok(false);
        };
        for &id in ids.iter().take(30) {
            if let Some(j) = index_of(&self.map_actions.actions, id) {
                f(self, j)?;
            }
        }
        Ok(true)
    }

    fn legion_call(&mut self, j: usize) -> Result<(), Stop> {
        if self.map_actions.actions[j].tag == u32::from_be_bytes(*b"legi")
            && self.legion_glo(j)
            && let Some(mut s) = self.legions.get(&j).copied()
        {
            s.flags |= LEGION_CALLED;
            self.legion_store(j, s);
        }
        Ok(())
    }

    fn legion_glo(&self, i: usize) -> bool {
        find(&self.map_actions.actions, i, 1, b"#glo", 1).is_some_and(|p| p.count == 0x180)
            && self.legions.contains_key(&i)
    }

    fn legion_store(&mut self, i: usize, s: LegionState) {
        self.set_param(i, b"#glo", 1, 0x180, vec![0; 0x180]);
        self.legions.insert(i, s);
    }

    fn legion_handle(&mut self, s: &mut LegionState, v: &Variables) -> Result<(), Stop> {
        let i = s.action as usize;
        let mut changed = false;
        let next = match s.state {
            0 => {
                let stronger = v.enemy.is_some_and(|e| {
                    s.flags & LEGION_IN_RANGE != 0
                        && s.strength
                            .wrapping_shl(16)
                            .wrapping_div(i32::from(e.strength))
                            > s.pora
                });
                (s.flags & (LEGION_CASUALTIES | LEGION_CALLED) != 0
                    || stronger
                    || s.flags & LEGION_FALL_BACK != 0)
                    .then_some(1)
            }
            1 if s.flags & LEGION_IN_RANGE == 0 => Some(0),
            _ => None,
        };
        if let Some(n) = next
            && n != s.state
        {
            if n == 1 {
                self.legion_each(i, b"alli", Self::legion_call)?;
                self.legion_each(i, b"deat", Self::deactivate)?;
                self.legion_each(i, b"acat", Self::activate)?;
            } else {
                self.legion_each(i, b"demo", Self::deactivate)?;
                self.legion_each(i, b"acmo", Self::activate)?;
            }
            s.state = n;
            changed = true;
        }
        let t = trig();
        let x = index(-i32::from(s.facing));
        let (cos, sin) = (i32::from(t.cosine[x]), i32::from(t.sine[x]));
        for k in 0..s.count as usize {
            let g = s.groups[k];
            if g.units <= 0 {
                continue;
            }
            let off = rotate(g.offset, cos, sin);
            let mut target = (off.0.wrapping_add(s.point.0), off.1.wrapping_add(s.point.1));
            let facing = g.facing_delta.wrapping_add(s.facing);
            let mut attack = s.state;
            if s.state == 1 && g.guard != -1 {
                let Some(h) = usize::try_from(g.guard).ok().and_then(|h| s.groups.get(h)) else {
                    return Err(Stop::new(
                        draw_site::LEGION_GROUP_OUT_OF_RANGE,
                        format!("map action {i}: 'legi' group {k}'s 'grgd' is out of range"),
                    ));
                };
                if h.units > 0 {
                    target = (
                        target.0.wrapping_add(h.centre.0.wrapping_sub(h.target.0)),
                        target.1.wrapping_add(h.centre.1.wrapping_sub(h.target.1)),
                    );
                    let near = if g.mode == 1 { 0x1800 } else { 0x1000 };
                    let groups = self.spatial_groups()?;
                    attack = i16::from(groups.iter().any(|o| {
                        o.team != v.team
                            && (distance(o.centre, target) as i32).wrapping_sub(i32::from(o.radius))
                                < near
                    }));
                }
            }
            let g = &mut s.groups[k];
            let mut moved = changed;
            if s.flags & LEGION_INITIALISED == 0
                || angle_difference(i32::from(facing), i32::from(g.facing)) > 0x1555
                || distance(target, g.target) > 0x400
            {
                g.facing = facing;
                g.target = target;
                moved = true;
            }
            if attack == 0 && g.mode == 1 {
                moved = true;
            }
            let list = v.units[g.first..g.first + g.units as usize].to_vec();
            if attack == 0 {
                if moved {
                    let command = crate::commands::MovementCommand {
                        flags: 0x18,
                        formation: g.formation as u16,
                        facing: g.facing,
                    };
                    let dest = g.target;
                    self.record_order(|| crate::Order {
                        site: draw_site::MOVEMENT_ORDER,
                        action: i,
                        monsters: list.clone(),
                        target: Some(dest),
                        offset: None,
                        done: true,
                    });
                    self.execute_movement_command(-2, &command, &list, &[dest])?;
                }
            } else if attack == 1 {
                let mut config = self.attack_configuration(None);
                config.point = s.point;
                config.flags &= !1;
                config.radius = s.atra[1];
                self.attack_with_monsters(&list, config);
            }
            s.groups[k].mode = attack;
        }
        s.flags &= !LEGION_CALLED;
        Ok(())
    }

    pub(crate) fn legion_action(&mut self, i: usize) -> Result<u32, Stop> {
        crate::map_actions::not_read_by_tfl(&[b"acde", b"crra"]);
        let stored = (self.map_actions.actions[i].flags & 0x80 == 0 && self.legion_glo(i))
            .then(|| self.legions.get(&i).copied())
            .flatten();
        let Some(mut s) = stored.or_else(|| self.legion_initial(i)) else {
            return Ok(0);
        };
        let mut v = self.legion_variables(&mut s)?;
        if v.units.is_empty() {
            return Ok(4);
        }
        self.legion_centre_and_facing(&mut s, &mut v)?;
        let mut r = 0;
        if s.flags & LEGION_FALL_BACK != 0 && self.legion_each(i, b"fall", Self::activate)? {
            r = 4;
        } else {
            self.legion_handle(&mut s, &v)?;
        }
        s.flags |= LEGION_INITIALISED;
        self.legion_store(i, s);
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_closest_point_has_a_10_bit_parameter() {
        assert_eq!(closest_point((5, 5), (0, 0), (0x400, 0)), ((5, 0), (5, 0)));
        assert_eq!(closest_point((-5, 5), (0, 0), (0x400, 0)).1, (0, 0));
        assert_eq!(closest_point((0x500, 5), (0, 0), (0x400, 0)).1, (0x400, 0));
        assert_eq!(closest_point((3, 3), (1, 1), (2, 1)), ((2, 1), (2, 1)));
    }
}
