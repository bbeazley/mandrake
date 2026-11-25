use crate::engine_rng::Unmodelled;
use crate::map_actions::{MapActionContext, MapActions, TYPE_INTEGER};
use mandrake_common::game_options;

impl MapActions {
    pub fn player_count_action(
        &self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let world = &*ctx.world;
        let teams = i32::from(world.players.teams.len() as i16);
        if !self.counts_hold(i, [b"tc_=", b"tc_<", b"tc_>"], teams) {
            return Ok(0);
        }
        let names = [b"pc_=", b"pc_<", b"pc_>"];
        if names
            .iter()
            .all(|n| self.long(i, n, TYPE_INTEGER).is_none())
        {
            return Ok(2);
        }
        let mut team = -1;
        if world.formation_globals.gameplay_version > game_options::GAMEPLAY_1_5_1
            && let Some(t) = self.long(i, b"team", TYPE_INTEGER)
        {
            team = t;
        }
        let players = world
            .players
            .players
            .iter()
            .filter(|p| p.field_09 == 0 && (team == -1 || i32::from(p.team) == team))
            .count() as i32;
        Ok(if self.counts_hold(i, names, players) {
            2
        } else {
            0
        })
    }

    fn counts_hold(&self, i: usize, [eq, lt, gt]: [&[u8; 4]; 3], count: i32) -> bool {
        let test = |name, ok: fn(i32, i32) -> bool| {
            self.long(i, name, TYPE_INTEGER)
                .is_none_or(|v| ok(count, v))
        };
        test(eq, |c, v| c == v) && test(lt, |c, v| c < v) && test(gt, |c, v| c > v)
    }
}
