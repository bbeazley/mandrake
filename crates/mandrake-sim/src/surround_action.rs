use crate::engine_move::{arctangent, distance2d, signed_angular_difference, trig_tables};
use crate::engine_rng::{MovementHeader, Unmodelled};
use crate::map_actions::{MapActionContext, MapActions, TYPE_ACTIONS, TYPE_DISTANCE, TYPE_INTEGER};
use crate::map_attacks::{AttackConfig, attack_with_monster_indexes};
use crate::projectiles::isqrt;

const MAX_TURN: i32 = 0x2aaa;

const MAX_LISTED: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurroundState {
    pub state: i16,
    pub initial: i16,
    pub loss: i32,
    pub time: i32,
}

impl MapActions {
    pub fn surround_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let mut st = self.surround_cst.get(&i).copied().unwrap_or(SurroundState {
            state: 0,
            initial: -1,
            loss: self.long(i, b"loss", TYPE_INTEGER).unwrap_or(-1),
            time: self.long(i, b"time", TYPE_INTEGER).unwrap_or(-1),
        });
        let max = if ctx.world.formation_globals.gameplay_version >= 0x515 {
            0x7e
        } else {
            0x1e
        };
        let subjects = self.subjects(i, ctx, max);
        let count = subjects.len() as i16;
        let centre = ctx.world.centerpoint(&subjects);
        if st.initial == -1 {
            st.initial = count;
        }
        let time = st.time;
        st.time = time.wrapping_sub(1);
        let ended =
            time == 0 || (st.loss != -1 && st.loss <= i32::from(st.initial) - i32::from(count));
        if ended && st.state != 1 {
            if let Some(acat) = self.find(i, 0, b"acat", Some(TYPE_ACTIONS)) {
                let listed: Vec<i32> = acat.values.iter().take(MAX_LISTED).copied().collect();
                for id in listed {
                    let j = self.index_of(id);
                    self.activate(j, ctx)?;
                }
            }
            st.state = 1;
        }
        let result = if subjects.is_empty() {
            4
        } else {
            if ended || st.state == 1 {
                let mut config = self.attack_ai_configuration(i);
                config.point = centre;
                attack_with_monster_indexes(ctx.world, &subjects, config);
            } else if st.state == 0 {
                surround_walk(i, &subjects, ctx)?;
            }
            0
        };
        self.surround_cst.insert(i, st);
        Ok(result)
    }

    fn attack_ai_configuration(&self, i: usize) -> AttackConfig {
        let mut config = AttackConfig::new();
        if let Some(radi) = self.find(i, 0, b"radi", Some(TYPE_DISTANCE))
            && radi.count == 2
        {
            config.flags &= !1;
            config.radius = radi.values[1];
        }
        config
    }
}

fn surround_walk(
    i: usize,
    subjects: &[usize],
    ctx: &mut MapActionContext,
) -> Result<(), Unmodelled> {
    let w = &mut *ctx.world;
    let team = w.monsters[subjects[0]].team;
    let positions: Vec<(i32, i32)> = subjects.iter().map(|&m| w.monsters[m].position).collect();
    for (&m, &(x, y)) in subjects.iter().zip(&positions) {
        let mut nearest: Option<(i32, (i32, i32), i32)> = None;
        for g in &w.spatial_groups {
            if g.team == team || w.teams_are_allied(team, g.team) {
                continue;
            }
            let reach = i32::from(g.radius) + i32::from(g.range);
            let d = distance2d(x.wrapping_sub(g.center.0), y.wrapping_sub(g.center.1)) - reach;
            if nearest.is_none_or(|(best, _, _)| d < best) {
                nearest = Some((d, g.center, reach));
            }
        }
        let Some((_, centre, reach)) = nearest else {
            continue;
        };
        let (mut sum_x, mut sum_y, mut others) = (0i32, 0i32, 0);
        for &(ox, oy) in &positions {
            let (dx, dy) = (x.wrapping_sub(ox), y.wrapping_sub(oy));
            let len = isqrt(dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) as u32);
            if len != 0 {
                others += 1;
                sum_x = sum_x.wrapping_add(dx.wrapping_shl(14) / len);
                sum_y = sum_y.wrapping_add(dy.wrapping_shl(14) / len);
            }
        }
        if others == 0 {
            continue;
        }
        let away = arctangent(sum_x, sum_y);
        let out = arctangent(x.wrapping_sub(centre.0), y.wrapping_sub(centre.1));
        let turn = signed_angular_difference(out, away).clamp(-MAX_TURN, MAX_TURN);
        let angle = (turn + i32::from(out)) & 0xffff;
        let e = ((angle + 0x20) >> 6) as usize & 0x3ff;
        let t = trig_tables();
        let point = (
            ((i32::from(t.cosine[e]).wrapping_mul(reach)) >> 14).wrapping_add(centre.0),
            ((i32::from(t.sine[e]).wrapping_mul(reach)) >> 14).wrapping_add(centre.1),
        );
        w.movement_command(&[m], &[point], MovementHeader::default(), i as i32)?;
    }
    Ok(())
}
