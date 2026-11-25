use crate::engine_move::distance2d;
use crate::engine_rng::{EngineWorld, Unmodelled};
use crate::map_actions::{MapActionContext, MapActions};

const MAX_SUBJECTS: usize = 400;

fn centre_and_radius(w: &EngineWorld, subjects: &[usize]) -> ((i32, i32), i32) {
    let centre = w.centerpoint(subjects);
    let (mut min, mut max) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
    for &m in subjects {
        let p = w.monsters[m].position;
        min = (min.0.min(p.0), min.1.min(p.1));
        max = (max.0.max(p.0), max.1.max(p.1));
    }
    let radius = max
        .0
        .wrapping_sub(centre.0)
        .max(centre.0.wrapping_sub(min.0))
        .max(centre.1.wrapping_sub(min.1))
        .max(max.1.wrapping_sub(centre.1));
    (centre, radius)
}

impl MapActions {
    pub fn look_action(&self, i: usize, ctx: &MapActionContext) -> Result<u32, Unmodelled> {
        let subjects = self.subjects(i, ctx, MAX_SUBJECTS);
        if subjects.is_empty() {
            return Ok(2);
        }
        let w = &*ctx.world;
        let (centre, radius) = centre_and_radius(w, &subjects);
        let team = w.monsters[subjects[0]].team;
        let seen = w.spatial_groups.iter().any(|g| {
            g.team != team
                && !w.teams_are_allied(team, g.team)
                && distance2d(
                    centre.0.wrapping_sub(g.center.0),
                    centre.1.wrapping_sub(g.center.1),
                )
                .wrapping_sub(
                    i32::from(g.radius)
                        .wrapping_add(i32::from(g.range))
                        .wrapping_add(radius),
                ) < 0
        });
        Ok(if seen { 2 } else { 0 })
    }
}
