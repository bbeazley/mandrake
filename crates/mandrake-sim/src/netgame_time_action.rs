use crate::engine_rng::Unmodelled;
use crate::map_actions::{MapActionContext, MapActions, TYPE_INTEGER};

impl MapActions {
    pub fn netgame_time_action(
        &self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let game = &ctx.world.game;
        let mut ok = true;
        if self.flag(i, b"plan") {
            ok = game.state != 0;
        }
        if self.flag(i, b"deth") && game.state != 4 {
            ok = false;
        }
        let time = self.long(i, b"time", TYPE_INTEGER);
        if self.flag(i, b"over") && game.state != 5 {
            return Ok(0);
        }
        if time.is_some_and(|t| game.timer > t.wrapping_mul(30)) {
            return Ok(0);
        }
        Ok(if ok { 2 } else { 0 })
    }
}
