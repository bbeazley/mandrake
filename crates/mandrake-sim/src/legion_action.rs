use crate::draw_site;
use crate::engine_move::{arctangent, distance2d, trig_tables};
use crate::engine_rng::{MovementHeader, Unmodelled, angular_difference};
use crate::map_actions::{
    MapActionContext, MapActions, TYPE_ACTIONS, TYPE_DISTANCE, TYPE_FIXED, TYPE_INTEGER, TYPE_POINT,
};
use crate::map_attacks::{AttackConfig, attack_with_monster_indexes};
use crate::projectiles::isqrt;
use crate::spatial_groups::{SpatialSupergroup, supergroups};

const TYPE_ANGLE: i16 = 4;
const LEGION_TAG: u32 = u32::from_be_bytes(*b"legi");
const MAX_UNITS: usize = 0x3c;
const MODE_INCAPACITATED: u8 = 0xd;
const FLAG_FIRST_TRIGGER: u32 = 0x100;

pub const LEGION_INITIALISED: u32 = 1;
pub const LEGION_FALL_BACK: u32 = 2;
pub const LEGION_IN_RANGE: u32 = 4;
pub const LEGION_CASUALTIES: u32 = 0x10;
pub const LEGION_CALLED: u32 = 0x20;
pub const LEGION_CIRCLE: u32 = 0x40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LegionGroup {
    pub offset: (i32, i32),
    pub facing_delta: u16,
    pub formation: i32,
    pub guard: i32,
    pub action: i16,
    pub units: i16,
    pub first: i32,
    pub center: (i32, i32),
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

fn sub(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    (a.0.wrapping_sub(b.0), a.1.wrapping_sub(b.1))
}

fn distance(a: (i32, i32), b: (i32, i32)) -> i32 {
    let d = sub(a, b);
    distance2d(d.0, d.1)
}

fn square(d: (i32, i32)) -> i32 {
    d.0.wrapping_mul(d.0).wrapping_add(d.1.wrapping_mul(d.1))
}

fn dot(a: (i32, i32), b: (i32, i32)) -> i32 {
    a.0.wrapping_mul(b.0).wrapping_add(a.1.wrapping_mul(b.1))
}

fn trig(a: i32) -> (i32, i32) {
    let t = trig_tables();
    let k = ((a + 0x20) >> 6) as usize & 0x3ff;
    (i32::from(t.cosine[k]), i32::from(t.sine[k]))
}

pub fn closest_point_on_line_segment_from_point(
    p: (i32, i32),
    a: (i32, i32),
    b: (i32, i32),
) -> ((i32, i32), (i32, i32)) {
    let d = sub(b, a);
    let k = square(d) >> 9;
    if k == 0 {
        return (b, b);
    }
    let t = dot(sub(p, a), d).wrapping_div(k);
    let raw = (
        (d.0.wrapping_mul(t) >> 9).wrapping_add(a.0),
        (d.1.wrapping_mul(t) >> 9).wrapping_add(a.1),
    );
    let clamped = if t < 0 {
        a
    } else if t <= 0x200 {
        raw
    } else {
        b
    };
    (raw, clamped)
}

impl MapActions {
    pub fn legion_state(&self, i: usize) -> Option<&LegionState> {
        self.legion_glo.get(&i)
    }

    fn legion_initial(&self, i: usize, gameplay: i32) -> Option<LegionState> {
        let mut s = LegionState {
            action: i as i16,
            vira: self.indexed(i, b"vira", TYPE_DISTANCE, 0).unwrap_or(0x6400),
            atra: [
                self.indexed(i, b"atra", TYPE_DISTANCE, 0).unwrap_or(0x2800),
                self.indexed(i, b"atra", TYPE_DISTANCE, 1).unwrap_or(0x3800),
            ],
            pora: self.indexed(i, b"pora", TYPE_FIXED, 0).unwrap_or(0x20000),
            vide: self.indexed(i, b"vide", TYPE_FIXED, 0).unwrap_or(0x10000),
            podt: self.indexed(i, b"podt", TYPE_DISTANCE, 0).unwrap_or(0x800),
            ..LegionState::default()
        };
        let line = self.find(i, 0, b"line", Some(TYPE_POINT))?;
        if line.count != 2 {
            return None;
        }
        s.line = [
            (line.values[0], line.values[1]),
            (line.values[2], line.values[3]),
        ];
        if self.flag(i, b"circ") {
            s.flags |= LEGION_CIRCLE;
            s.circle_radius = distance(s.line[0], s.line[1]) as i16;
        }
        let mut center = self.indexed_point(i, b"cent", 0);
        let mut facing = self.indexed(i, b"faci", TYPE_ANGLE, 0).map(|f| f as u16);
        s.count = self.indexed(i, b"coun", TYPE_INTEGER, 0)?;
        if s.count as u32 > 6 {
            return None;
        }
        for k in 0..s.count as usize {
            let g = &mut s.groups[k];
            let grfa = self.indexed(i, b"grfa", TYPE_ANGLE, k).map(|f| f as u16);
            let faci = match facing {
                Some(f) => {
                    g.facing_delta = f.wrapping_sub(grfa.unwrap_or(f));
                    f
                }
                None => grfa.unwrap_or(0),
            };
            facing = Some(faci);
            g.formation = self.indexed(i, b"grfo", TYPE_INTEGER, k).unwrap_or(1);
            match self.indexed_point(i, b"grce", k) {
                Some(p) => {
                    let c = *center.get_or_insert(p);
                    let d = sub(p, c);
                    let (cos, sin) = trig(i32::from(faci));
                    g.offset = (
                        cos.wrapping_mul(d.0).wrapping_add(sin.wrapping_mul(d.1)) >> 14,
                        cos.wrapping_mul(d.1).wrapping_sub(sin.wrapping_mul(d.0)) >> 14,
                    );
                }
                None if s.count != 1 => break,
                None => {}
            }
            g.guard = self.indexed(i, b"grgd", TYPE_INTEGER, k).unwrap_or(-1);
            if g.guard != -1 && !(0..s.count).contains(&g.guard) && gameplay > 0x6b3 {
                g.guard = -1;
            }
            let Some(id) = self.indexed(i, b"grun", TYPE_ACTIONS, k) else {
                break;
            };
            g.action = self.index_of(id).map_or(-1, |j| j as i16);
            if g.action == -1 {
                break;
            }
        }
        Some(s)
    }

    fn legion_listed(&self, i: usize, name: &[u8; 4]) -> Option<Vec<i32>> {
        let p = self.find(i, 0, name, Some(TYPE_ACTIONS))?;
        Some(p.values.iter().take(0x1e).copied().collect())
    }

    fn legion_activate(
        &mut self,
        i: usize,
        name: &[u8; 4],
        ctx: &mut MapActionContext,
    ) -> Result<bool, Unmodelled> {
        let Some(ids) = self.legion_listed(i, name) else {
            return Ok(false);
        };
        for id in ids {
            let j = self.index_of(id);
            self.activate(j, ctx)?;
        }
        Ok(true)
    }

    fn legion_deactivate(
        &mut self,
        i: usize,
        name: &[u8; 4],
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        for id in self.legion_listed(i, name).unwrap_or_default() {
            let j = self.index_of(id);
            self.deactivate(j, ctx)?;
        }
        Ok(())
    }

    fn legion_call_allies(&mut self, i: usize) -> Result<(), Unmodelled> {
        for id in self.legion_listed(i, b"alli").unwrap_or_default() {
            let Some(j) = self.index_of(id) else {
                return Err(Unmodelled::new(
                    draw_site::LEGION_ALLY_MISSING,
                    format!("map action #{i} 'legi': 'alli' names no action ({id})"),
                ));
            };
            if self.actions[j].tag == LEGION_TAG
                && let Some(s) = self.legion_glo.get_mut(&j)
            {
                s.flags |= LEGION_CALLED;
            }
        }
        Ok(())
    }

    pub fn legion_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let first = self.actions[i].flags & FLAG_FIRST_TRIGGER != 0;
        let stored = self.legion_glo.get(&i).copied().filter(|_| !first);
        let mut s = match stored {
            Some(s) => s,
            None => {
                let gameplay = ctx.world.formation_globals.gameplay_version;
                match self.legion_initial(i, gameplay) {
                    Some(s) => s,
                    None => return Ok(0),
                }
            }
        };
        let count = s.count as usize;

        let mut units: Vec<usize> = Vec::new();
        let (mut strength, mut vitality) = (0i32, 0i32);
        #[allow(clippy::needless_range_loop)]
        for k in 0..count {
            let action = s.groups[k].action;
            if action < 0 {
                return Err(Unmodelled::new(
                    draw_site::LEGION_GROUP_ACTION_MISSING,
                    format!("map action #{i} 'legi': group {k}'s 'grun' names no action"),
                ));
            }
            let room = MAX_UNITS - units.len();
            let found: Vec<usize> = self
                .subjects(action as usize, ctx, room)
                .into_iter()
                .filter(|&m| ctx.world.monsters[m].mode != MODE_INCAPACITATED)
                .collect();
            for &m in &found {
                let monster = &ctx.world.monsters[m];
                let definition = &ctx.world.monster_definitions[monster.definition];
                strength = strength.wrapping_add(i32::from(definition.spatial_group_strength));
                vitality = vitality.wrapping_add(monster.object.vitality);
            }
            let g = &mut s.groups[k];
            g.first = units.len() as i32;
            g.units = found.len() as i16;
            g.center = ctx.world.centerpoint(&found);
            units.extend(found);
        }

        if s.flags & LEGION_INITIALISED != 0 {
            if (s.vide >> 8).wrapping_add(vitality) < s.vitality {
                s.flags |= LEGION_CASUALTIES;
                s.vitality = vitality;
            } else {
                s.flags &= !LEGION_CASUALTIES;
            }
            if units.is_empty() {
                return Ok(4);
            }
        } else {
            s.vitality = vitality;
            if units.is_empty() {
                return Ok(4);
            }
            let d = sub(s.line[0], s.line[1]);
            s.facing = arctangent(d.0, d.1);
            s.point = s.line[0];
        }
        s.strength = strength;
        let team = ctx.world.monsters[units[0]].team;

        let mut nearest: Option<SpatialSupergroup> = None;
        let mut nearest_d = 0i32;
        let mut enemy: Option<SpatialSupergroup> = None;
        let mut enemy_d = i32::MAX;
        for g in supergroups(&ctx.world.spatial_groups) {
            if g.team == team || ctx.world.teams_are_allied(team, g.team) || g.strength == 0 {
                continue;
            }
            let v = sub(g.center, s.point);
            let mut d = distance2d(v.0, v.1);
            if dot((i32::from(g.velocity.0), i32::from(g.velocity.1)), v) < 0 {
                d = d.wrapping_mul(2);
            }
            if d >= s.vira {
                continue;
            }
            let by_range = d.wrapping_sub(i32::from(g.range));
            if nearest.is_none() || by_range < nearest_d {
                nearest = Some(*g);
                nearest_d = by_range;
            }
            let by_radius = d.wrapping_sub(i32::from(g.radius));
            let better = match enemy {
                None => true,
                Some(e) => {
                    by_radius < enemy_d
                        || (g.strength > e.strength && by_radius.wrapping_sub(enemy_d) < s.podt)
                }
            };
            if better {
                enemy = Some(*g);
                enemy_d = by_radius;
            }
        }

        let mut flags = s.flags & !LEGION_FALL_BACK;
        let (point, facing) = match (nearest, enemy) {
            (Some(n), Some(e)) => {
                if s.flags & LEGION_CIRCLE != 0 {
                    let d = sub(e.center, s.line[0]);
                    let a = arctangent(d.0, d.1);
                    let (cos, sin) = trig(i32::from(a));
                    let r = i32::from(s.circle_radius);
                    (
                        (
                            (cos.wrapping_mul(r) >> 14).wrapping_add(s.line[0].0),
                            (sin.wrapping_mul(r) >> 14).wrapping_add(s.line[0].1),
                        ),
                        a,
                    )
                } else {
                    let range = i32::from(n.range);
                    let (raw, clamped) =
                        closest_point_on_line_segment_from_point(n.center, s.line[0], s.line[1]);
                    let at = if distance(n.center, clamped) >= range {
                        clamped
                    } else {
                        let back = sub(s.line[0], raw);
                        let len = isqrt(square(back) as u32);
                        let out = isqrt(
                            range
                                .wrapping_mul(range)
                                .wrapping_sub(square(sub(raw, n.center)))
                                as u32,
                        );
                        if len > out && len != 0 {
                            (
                                (back.0.wrapping_mul(out) / len).wrapping_add(raw.0),
                                (back.1.wrapping_mul(out) / len).wrapping_add(raw.1),
                            )
                        } else {
                            flags |= LEGION_FALL_BACK;
                            s.line[0]
                        }
                    };
                    let d = sub(n.center, at);
                    (at, arctangent(d.0, d.1))
                }
            }
            _ => {
                let d = sub(s.line[1], s.line[0]);
                (s.line[1], arctangent(d.0, d.1))
            }
        };
        let in_range = if s.state == 1 { s.atra[1] } else { s.atra[0] };
        flags &= !LEGION_IN_RANGE;
        if enemy_d < in_range {
            flags |= LEGION_IN_RANGE;
        }
        s.flags = flags;

        if s.state == 0 || dot(sub(point, s.point), sub(s.line[1], s.point)) > 0 {
            s.point = point;
            s.facing = facing;
        }

        if s.flags & LEGION_FALL_BACK != 0 && self.legion_activate(i, b"fall", ctx)? {
            s.flags |= LEGION_INITIALISED;
            self.legion_glo.insert(i, s);
            return Ok(4);
        }

        let mut changed = false;
        if s.state == 0 {
            let stronger = enemy.is_some_and(|e| {
                s.flags & LEGION_IN_RANGE != 0
                    && (s.strength << 16).wrapping_div(i32::from(e.strength)) > s.pora
            });
            if s.flags & (LEGION_CASUALTIES | LEGION_CALLED) != 0
                || stronger
                || s.flags & LEGION_FALL_BACK != 0
            {
                self.legion_call_allies(i)?;
                self.legion_deactivate(i, b"deat", ctx)?;
                self.legion_activate(i, b"acat", ctx)?;
                s.state = 1;
                changed = true;
            }
        } else if s.state == 1 && s.flags & LEGION_IN_RANGE == 0 {
            self.legion_deactivate(i, b"demo", ctx)?;
            self.legion_activate(i, b"acmo", ctx)?;
            s.state = 0;
            changed = true;
        }

        let (cos, sin) = trig(-i32::from(s.facing));
        for k in 0..count {
            let g = s.groups[k];
            if g.units <= 0 {
                continue;
            }
            let group_facing = s.facing.wrapping_add(g.facing_delta);
            let mut target = (
                (g.offset
                    .0
                    .wrapping_mul(cos)
                    .wrapping_add(g.offset.1.wrapping_mul(sin))
                    >> 14)
                    .wrapping_add(s.point.0),
                (g.offset
                    .1
                    .wrapping_mul(cos)
                    .wrapping_sub(g.offset.0.wrapping_mul(sin))
                    >> 14)
                    .wrapping_add(s.point.1),
            );
            let mut attack = s.state == 1;
            if attack && g.guard != -1 {
                let Some(guard) = usize::try_from(g.guard).ok().filter(|&h| h < 6) else {
                    return Err(Unmodelled::new(
                        draw_site::LEGION_GROUP_OUT_OF_RANGE,
                        format!("map action #{i} 'legi': group {k}'s 'grgd' is out of range"),
                    ));
                };
                let h = s.groups[guard];
                if h.units >= 1 {
                    target = sub(
                        (
                            h.center.0.wrapping_add(target.0),
                            h.center.1.wrapping_add(target.1),
                        ),
                        h.target,
                    );
                    let near = if g.mode == 1 { 0x1800 } else { 0x1000 };
                    attack = ctx.world.spatial_groups.iter().any(|o| {
                        o.team != team
                            && !ctx.world.teams_are_allied(team, o.team)
                            && distance(o.center, target).wrapping_sub(i32::from(o.radius)) < near
                    });
                }
            }
            let keep = s.flags & LEGION_INITIALISED != 0
                && angular_difference(group_facing, g.facing) < 0x1556
                && distance(target, g.target) <= 0x400;
            let (order_target, order_facing) = if keep {
                (g.target, g.facing)
            } else {
                (target, group_facing)
            };
            let list = &units[g.first as usize..g.first as usize + g.units as usize];
            if attack {
                let config = AttackConfig {
                    flags: 0,
                    point: s.point,
                    radius: s.atra[1],
                };
                attack_with_monster_indexes(ctx.world, list, config);
            } else if !keep || g.mode == 1 || changed {
                let header = MovementHeader {
                    flags: 0x18,
                    formation: g.formation as i16,
                    facing: order_facing,
                };
                ctx.world
                    .movement_command(list, &[order_target], header, -2)?;
            }
            let g = &mut s.groups[k];
            g.target = order_target;
            g.facing = order_facing;
            g.mode = i16::from(attack);
        }
        s.flags = (s.flags & !LEGION_CALLED) | LEGION_INITIALISED;
        self.legion_glo.insert(i, s);
        Ok(0)
    }
}
