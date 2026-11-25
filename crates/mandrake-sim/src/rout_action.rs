use crate::engine_move::distance2d;
use crate::engine_rng::{MovementHeader, Unmodelled};
use crate::map_actions::{MapActionContext, MapActions, TYPE_POINT};

const MAX_SUBJECTS: usize = 400;

const UNDER_USER_CONTROL: u32 = 0x0800_0000;

impl MapActions {
    pub fn rout_action(&mut self, i: usize, ctx: &mut MapActionContext) -> Result<u32, Unmodelled> {
        let subjects: Vec<usize> = self
            .subjects(i, ctx, MAX_SUBJECTS)
            .into_iter()
            .filter(|&m| ctx.world.monsters[m].flags & UNDER_USER_CONTROL == 0)
            .collect();
        if subjects.is_empty() {
            return Ok(6);
        }
        let Some(dest) = self.find(i, 0, b"dest", Some(TYPE_POINT)) else {
            return Ok(0);
        };
        let points: Vec<(i32, i32)> = dest
            .values
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[x, y]| (x, y))
            .collect();
        for m in subjects {
            let (x, y) = ctx.world.monsters[m].position;
            let mut nearest = points[0];
            let mut best = distance2d(nearest.0.wrapping_sub(x), nearest.1.wrapping_sub(y));
            for &p in &points[1..] {
                let d = distance2d(p.0.wrapping_sub(x), p.1.wrapping_sub(y));
                if d < best {
                    best = d;
                    nearest = p;
                }
            }
            ctx.world
                .movement_command(&[m], &[nearest], MovementHeader::default(), i as i32)?;
        }
        Ok(0)
    }
}
