use crate::engine_move::{arctangent, distance2d, trig_tables};
use crate::engine_rng::{MovementHeader, Unmodelled};
use crate::map_actions::{
    MapActionContext, MapActions, TYPE_ACTIONS, TYPE_DISTANCE, TYPE_MONSTERS,
};
use crate::projectiles::isqrt;
use crate::spatial_groups::supergroups;
use mandrake_common::game_options;

const MAX_SUBJECTS: usize = 0x10;
const MAX_OBJECTS: usize = 0xc;
const MODE_INCAPACITATED: u8 = 0xd;
const MAX_LISTED: usize = 0x1e;
const MONSTER_HOLD: u32 = 0x40_0000;
const SOURCE: i32 = -2;

pub const FOLLOW_INITIALISED: u32 = 1;
pub const FOLLOW_FAR: u32 = 2;
pub const FOLLOW_LOST: u32 = 4;
pub const FOLLOW_OBJECTS: u32 = 8;
pub const FOLLOW_STOP: u32 = 0x10;
pub const FOLLOW_BLIND: u32 = 0x20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FollowState {
    pub action: i16,
    pub flags: u32,
    pub radius: i32,
    pub forward: i32,
    pub centre: (i32, i32),
    pub destination: (i32, i32),
}

impl MapActions {
    pub fn follow_state(&self, i: usize) -> Option<FollowState> {
        self.follow_glo.get(&i).copied()
    }

    pub fn set_follow_state(&mut self, i: usize, state: Option<FollowState>) {
        match state {
            Some(s) => self.follow_glo.insert(i, s),
            None => self.follow_glo.remove(&i),
        };
    }

    fn follow_activate(
        &mut self,
        i: usize,
        name: &[u8; 4],
        ctx: &mut MapActionContext,
    ) -> Result<(), Unmodelled> {
        let ids: Vec<i32> = self
            .find(i, 0, name, Some(TYPE_ACTIONS))
            .map(|p| p.values.iter().take(MAX_LISTED).copied().collect())
            .unwrap_or_default();
        for id in ids {
            let j = self.index_of(id);
            self.activate(j, ctx)?;
        }
        Ok(())
    }

    pub fn follow_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let mut st = match self.follow_glo.get(&i) {
            Some(&s) => s,
            None => {
                let mut flags = 0;
                if self.flag(i, b"stop") {
                    flags |= FOLLOW_STOP;
                }
                if self.flag(i, b"blnk") {
                    flags |= FOLLOW_BLIND;
                }
                FollowState {
                    action: i as i16,
                    flags,
                    radius: self.long(i, b"radi", TYPE_DISTANCE).unwrap_or(0x1e00),
                    forward: self.long(i, b"fora", TYPE_DISTANCE).unwrap_or(0x100),
                    centre: (0, 0),
                    destination: (0, 0),
                }
            }
        };
        let a = st.action as u16 as usize;
        let w = &*ctx.world;
        let subjects = self.subjects(a, ctx, MAX_SUBJECTS);
        let listed = self.combined(a, b"obje", TYPE_MONSTERS, MAX_OBJECTS);
        if listed.is_empty() {
            st.flags &= !FOLLOW_OBJECTS;
        } else {
            st.flags |= FOLLOW_OBJECTS;
        }
        let objects: Vec<usize> = listed
            .iter()
            .filter_map(|&id| w.monster_index_by_identifier(id as u16))
            .filter(|&m| w.monsters[m].mode != MODE_INCAPACITATED)
            .collect();
        if subjects.is_empty() {
            self.follow_activate(a, b"erel", ctx)?;
            return Ok(4);
        }
        st.centre = w.centerpoint(&subjects);
        if objects.is_empty() && st.flags & FOLLOW_OBJECTS != 0 {
            self.follow_activate(a, b"edel", ctx)?;
            return Ok(4);
        }
        let team = w.monsters[subjects[0]].team;
        let (cx, cy) = st.centre;
        let (mut vx, mut vy, mut strength) = (0i32, 0i32, 0i32);
        let mut best: Option<(&crate::spatial_groups::SpatialSupergroup, i32)> = None;
        for g in supergroups(&w.spatial_groups) {
            let (dx, dy) = (cx.wrapping_sub(g.center.0), cy.wrapping_sub(g.center.1));
            let d = isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32);
            let edge = d.wrapping_sub(i32::from(g.radius));
            if edge >= st.radius {
                continue;
            }
            if g.team != team {
                if d != 0 && st.flags & FOLLOW_BLIND == 0 {
                    let s = i32::from(g.strength);
                    strength = strength.wrapping_add(s);
                    vx = vx.wrapping_add(dx.wrapping_mul(s).wrapping_div(d));
                    vy = vy.wrapping_add(dy.wrapping_mul(s).wrapping_div(d));
                }
                continue;
            }
            if let Some((b, b_edge)) = best
                && edge >= b_edge
                && (g.strength <= b.strength || edge.wrapping_sub(0x400) >= b_edge)
            {
                continue;
            }
            if dx | dy != 0 {
                best = Some((g, edge));
            }
        }
        if strength != 0 {
            vx = vx.wrapping_shl(16).wrapping_div(strength);
            vy = vy.wrapping_shl(16).wrapping_div(strength);
        }
        let (target, standoff) = if st.flags & FOLLOW_OBJECTS != 0 {
            let mut nearest = i32::MAX;
            let mut target = (0, 0);
            for (k, &o) in objects.iter().enumerate() {
                let p = w.monsters[o].position;
                let (dx, dy) = (cx.wrapping_sub(p.0), cy.wrapping_sub(p.1));
                let d = distance2d(dx, dy);
                if k == 0 || d < nearest {
                    target = p;
                    nearest = d;
                }
                if dx.wrapping_mul(vx).wrapping_add(dy.wrapping_mul(vy)) < 0 {
                    target = p;
                    break;
                }
            }
            (target, 0i16)
        } else {
            match best {
                Some((g, _)) => (g.center, g.radius),
                None => return self.follow_lost(i, st, &subjects, ctx),
            }
        };
        let first = (w.formation_globals.gameplay_version > game_options::GAMEPLAY_1_3)
            .then(|| w.monsters[subjects[0]].position);
        let reach = i32::from(standoff.wrapping_add(st.forward as i16));
        let angle = if strength == 0 {
            arctangent(cx.wrapping_sub(target.0), cy.wrapping_sub(target.1))
        } else {
            arctangent(vx, vy)
        };
        let e = ((i32::from(angle) + 0x20) >> 6) as usize & 0x3ff;
        let t = trig_tables();
        let point = (
            (i32::from(t.cosine[e]).wrapping_mul(reach) >> 14).wrapping_add(target.0),
            (i32::from(t.sine[e]).wrapping_mul(reach) >> 14).wrapping_add(target.1),
        );
        let off = |p: (i32, i32)| distance2d(point.0.wrapping_sub(p.0), point.1.wrapping_sub(p.1));
        let send = st.flags & FOLLOW_INITIALISED == 0
            || off(st.destination) > 0x200
            || first.is_some_and(|p| off(p) > st.forward);
        if send {
            let header = MovementHeader {
                flags: 0x10,
                formation: 0,
                facing: angle.wrapping_add(0x8000),
            };
            ctx.world
                .movement_command(&subjects, &[point], header, SOURCE)?;
            st.destination = point;
        } else if st.flags & FOLLOW_STOP != 0 {
            for &m in &subjects {
                ctx.world.monsters[m].flags |= MONSTER_HOLD;
            }
        }
        let away = distance2d(
            cx.wrapping_sub(st.destination.0),
            cy.wrapping_sub(st.destination.1),
        );
        if st.flags & FOLLOW_FAR != 0 {
            if away <= 0x600 {
                st.flags &= !FOLLOW_FAR;
            }
        } else if away > 0xa00 {
            self.follow_activate(a, b"acfa", ctx)?;
            st.flags |= FOLLOW_FAR;
        }
        st.flags = (st.flags & !FOLLOW_LOST) | FOLLOW_INITIALISED;
        self.follow_glo.insert(i, st);
        Ok(0)
    }

    fn follow_lost(
        &mut self,
        i: usize,
        mut st: FollowState,
        subjects: &[usize],
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let a = st.action as u16 as usize;
        ctx.world
            .general_command(subjects, 2, SOURCE as i16, ctx.rng, ctx.time)?;
        if st.flags & FOLLOW_LOST == 0 {
            self.follow_activate(a, b"acab", ctx)?;
        } else if st.flags & FOLLOW_FAR == 0 {
            self.follow_activate(a, b"acfa", ctx)?;
        }
        st.flags |= FOLLOW_FAR | FOLLOW_LOST | FOLLOW_INITIALISED;
        self.follow_glo.insert(i, st);
        Ok(0)
    }
}
