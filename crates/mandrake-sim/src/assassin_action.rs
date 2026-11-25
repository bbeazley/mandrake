use crate::draw_site;
use crate::engine_move::{arctangent, distance2d, trig_tables};
use crate::engine_rng::{MovementHeader, Unmodelled};
use crate::map_actions::{
    MapActionContext, MapActions, TYPE_DISTANCE, TYPE_INTEGER, TYPE_MONSTERS,
};
use crate::spatial_groups::{SpatialSupergroup, supergroups};

pub const ASSASSIN_INITIALISED: u32 = 1;
const MAX_SUBJECTS: usize = 0xa;
const SOURCE: i32 = -2;
const GROUP_REACH: i32 = 0xc00;
pub const DELAY_DRAW: u32 = draw_site::ASSASSIN_DELAY;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssassinEntry {
    pub monster: i16,
    pub identifier: i16,
    pub delay: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssassinState {
    pub flags: u32,
    pub radius: i32,
    pub action: i16,
    pub entries: Vec<AssassinEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    Idle,
    Target(usize),
    Move((i32, i32)),
}

fn trig(a: i32) -> (i32, i32) {
    let t = trig_tables();
    let k = (a.wrapping_add(0x20) >> 6) as usize & 0x3ff;
    (i32::from(t.cosine[k]), i32::from(t.sine[k]))
}

fn decide(
    groups: &[crate::spatial_groups::SpatialGroup],
    alliances: &crate::ally::AllianceView,
    team: i16,
    m: usize,
    p: (i32, i32),
) -> Result<Order, Unmodelled> {
    let mut single: Option<(&SpatialSupergroup, i32)> = None;
    let mut group: Option<(&SpatialSupergroup, i32)> = None;
    for g in supergroups(groups) {
        if g.team == team || alliances.teams_are_allied(team, g.team) {
            continue;
        }
        let d = distance2d(p.0.wrapping_sub(g.center.0), p.1.wrapping_sub(g.center.1))
            .wrapping_sub(i32::from(g.radius));
        let best = if g.members <= 1 {
            &mut single
        } else {
            &mut group
        };
        if best.is_none_or(|(_, bd)| d < bd) {
            *best = Some((g, d));
        }
    }
    let walk_round = match (group, single) {
        (Some((_, gd)), _) if gd >= GROUP_REACH => false,
        (Some(_), None) => true,
        (Some((g, gd)), Some((s, sd))) => {
            gd < sd
                && g.center
                    .0
                    .wrapping_sub(p.0)
                    .wrapping_mul(s.center.0.wrapping_sub(p.0))
                    .wrapping_add(
                        g.center
                            .1
                            .wrapping_sub(p.1)
                            .wrapping_mul(s.center.1.wrapping_sub(p.1)),
                    )
                    > 0
        }
        (None, _) => false,
    };
    if walk_round {
        let (g, _) = group.expect("a group");
        let c = g.center;
        let r = i32::from(g.radius).wrapping_add(GROUP_REACH);
        if r == 0 {
            return Ok(Order::Move(c));
        }
        let div = r.wrapping_mul(0x648) >> 8;
        if div == 0 {
            return Err(Unmodelled::new(
                draw_site::ASSASSIN_DIVISION_BY_ZERO,
                "assa: a division by zero",
            ));
        }
        let mut off = 0x400_0000i32.wrapping_div(div);
        if m & 1 != 0 {
            off = off.wrapping_neg();
        }
        let a = i32::from(arctangent(p.0.wrapping_sub(c.0), p.1.wrapping_sub(c.1)));
        let (cos, sin) = trig(off.wrapping_add(a));
        return Ok(Order::Move((
            c.0.wrapping_add(cos.wrapping_mul(r) >> 14),
            c.1.wrapping_add(sin.wrapping_mul(r) >> 14),
        )));
    }
    Ok(match (single, group) {
        (Some((s, _)), _) => Order::Target(s.first_member),
        (None, Some((g, _))) => Order::Move(g.center),
        (None, None) => Order::Idle,
    })
}

impl MapActions {
    pub fn assassin_state(&self, i: usize) -> Option<&AssassinState> {
        self.assassin_glo.get(&i)
    }

    pub fn set_assassin_state(&mut self, i: usize, state: Option<AssassinState>) {
        match state {
            Some(s) => self.assassin_glo.insert(i, s),
            None => self.assassin_glo.remove(&i),
        };
    }

    fn assassin_initial(&self, i: usize, ctx: &mut MapActionContext) -> AssassinState {
        let mut s = AssassinState {
            action: i as i16,
            radius: self.long(i, b"radi", TYPE_DISTANCE).unwrap_or(0x3200),
            ..AssassinState::default()
        };
        let dela = self.long(i, b"dela", TYPE_INTEGER).unwrap_or(0);
        for id in self.combined(i, b"subj", TYPE_MONSTERS, MAX_SUBJECTS) {
            let delay = if dela != 0 {
                let r = i32::from(ctx.rng.draw(DELAY_DRAW));
                r.wrapping_rem(dela) as i16
            } else {
                0
            };
            s.entries.push(AssassinEntry {
                monster: -1,
                identifier: id as i16,
                delay,
            });
        }
        s
    }

    pub fn assassin_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let mut s = match self.assassin_glo.get(&i) {
            Some(s) => s.clone(),
            None => self.assassin_initial(i, ctx),
        };
        let mut team: i16 = -1;
        let mut k = 0;
        while k < s.entries.len() {
            let e = s.entries[k];
            if e.delay != 0 {
                k += 1;
                continue;
            }
            let mut monster = e.monster;
            if monster == -1 {
                monster = ctx
                    .world
                    .make_monster_visible(e.identifier, -1, ctx.rng, ctx.time)?
                    .map_or(-1, |m| m as i16);
                s.entries[k].monster = monster;
            }
            if monster != -1 {
                let mon = &ctx.world.monsters[monster as usize];
                if mon.identifier != 0 && mon.identifier == e.identifier {
                    team = i16::from(mon.team as i8);
                    k += 1;
                    continue;
                }
            }
            s.entries.remove(k);
        }
        if s.entries.is_empty() {
            return Ok(4);
        }
        for k in 0..s.entries.len() {
            let e = s.entries[k];
            if e.delay != 0 {
                s.entries[k].delay -= 1;
                continue;
            }
            let m = e.monster as usize;
            let p = ctx.world.monsters[m].position;
            match decide(
                &ctx.world.spatial_groups,
                &ctx.world.alliance_view(),
                team,
                m,
                p,
            )? {
                Order::Idle => {}
                Order::Target(t) => crate::map_attacks::handle_target_command(
                    ctx.world,
                    i32::from(s.action),
                    0,
                    &[m],
                    &[t],
                )?,
                Order::Move(point) => ctx.world.movement_command(
                    &[m],
                    &[point],
                    MovementHeader {
                        flags: 0,
                        formation: 0,
                        facing: 0,
                    },
                    SOURCE,
                )?,
            }
        }
        s.flags |= ASSASSIN_INITIALISED;
        self.assassin_glo.insert(i, s);
        Ok(0)
    }
}
