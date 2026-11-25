use crate::engine_rng::{EngineWorld, Unmodelled};
use crate::map_actions::{MapActionContext, MapActions, TYPE_ACTIONS, TYPE_FIXED, TYPE_MONSTERS};

const MAX_LISTED: usize = 0x190;
const LEFT_THE_MESH: u32 = 0x4000_0000;
const HEAL_FLAGS: u16 = 2;
pub const SHAMAN_HEALING: u32 = 2;

fn present(w: &EngineWorld, ids: Vec<i32>) -> Vec<usize> {
    ids.into_iter()
        .filter_map(|id| w.monster_index_by_identifier(id as u16))
        .filter(|&m| w.monsters[m].flags & LEFT_THE_MESH == 0)
        .collect()
}

fn vitality_fraction(w: &EngineWorld, m: usize) -> i32 {
    let o = &w.monsters[m].object;
    let max = i32::from(o.max_vitality as i16);
    if max == 0 {
        return 0;
    }
    (i32::from(o.vitality as i16) << 16).wrapping_div(max)
}

impl MapActions {
    pub fn shaman_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        let subjects = present(
            &*ctx.world,
            self.combined(i, b"subj", TYPE_MONSTERS, MAX_LISTED),
        );
        if subjects.is_empty() {
            return Ok(0);
        }
        let allies = present(
            &*ctx.world,
            self.combined(i, b"ally", TYPE_MONSTERS, MAX_LISTED),
        );
        if allies.is_empty() {
            return Ok(0);
        }
        let Some(threshold) = self.long(i, b"<vit", TYPE_FIXED) else {
            return Ok(0);
        };
        let mut best = 0x10000i32;
        let mut target = None;
        for &m in &allies {
            let f = vitality_fraction(ctx.world, m);
            if f < threshold.min(best) {
                best = f;
                target = Some(m);
            }
        }
        if let Some(t) = target {
            crate::map_attacks::handle_target_command(
                ctx.world,
                i as i32,
                HEAL_FLAGS,
                &subjects[..1],
                &[t],
            )?;
            return Ok(SHAMAN_HEALING);
        }
        let fall = self
            .indexed(i, b"fall", TYPE_ACTIONS, 0)
            .and_then(|id| self.index_of(i32::from(id as i16)));
        if let Some(j) = fall {
            self.trigger(j, ctx)?;
        }
        Ok(0)
    }
}
