use mandrake_common::FourCC;

use crate::map_actions::{combined, find};
use crate::projectiles::ProjectileRequest;
use crate::{Stop, TflSim};

const MAX_SUBJECTS: usize = 0x12c;
const TYPE_SOUND_SOURCE: i16 = 0x10;
const TYPE_DISTANCE: i16 = 6;
const BOLT: FourCC = FourCC(*b"toli");
const REQUEST_FLAGS: u16 = 1;
const REQUEST_1C: i16 = 0x100;

fn nearest_in_range(radius: i32, subjects: &[(usize, i32)]) -> i16 {
    let mut best = -1i32;
    let mut target = -1i16;
    for &(m, d) in subjects {
        if d < radius && (best == -1 || d < best) {
            best = d;
            target = m as i16;
        }
    }
    target
}

impl TflSim {
    pub(crate) fn lightning_action(&mut self, i: usize) -> Result<u32, Stop> {
        let ids = combined(&self.map_actions.actions, i, b"subj", 2, MAX_SUBJECTS);
        let subjects = self.monster_slots(&ids);
        if subjects.is_empty() {
            return Ok(0);
        }
        let first = |name: &[u8; 4], kind: i16| {
            find(&self.map_actions.actions, i, 0, name, kind).filter(|p| p.count > 0)
        };
        let Some(sound) = first(b"soun", TYPE_SOUND_SOURCE).map(|p| p.i16_at(0)) else {
            return Ok(0);
        };
        let Some(radius) = first(b"radi", TYPE_DISTANCE).map(|p| p.i32_at(0)) else {
            return Ok(0);
        };
        let origin = self
            .sound_sources
            .iter()
            .find(|s| s.identifier != 0 && s.identifier == sound)
            .map_or([0; 3], |s| s.world);
        let distances: Vec<(usize, i32)> = subjects
            .iter()
            .map(|&m| {
                let o = &self.objects[self.monsters[m].object];
                let d = crate::groups::distance((origin[0], origin[1]), (o.x, o.y));
                (m, d as i32)
            })
            .collect();
        let target = nearest_in_range(radius, &distances);
        crate::tc2_trace::log(|| {
            format!(
                "LI {} {i} {target} {} {} {}",
                self.tick, origin[0], origin[1], origin[2]
            )
        });
        let mut q = ProjectileRequest::new(Some(BOLT));
        q.flags = REQUEST_FLAGS;
        q.position = origin;
        q.field_1c = REQUEST_1C;
        q.target = target;
        self.new_projectile(&q)?;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::nearest_in_range;

    #[test]
    fn nearest_in_range_first_of_equals() {
        assert_eq!(
            nearest_in_range(100, &[(5, 150), (7, 60), (9, 60), (2, 80)]),
            7
        );
        assert_eq!(nearest_in_range(60, &[(7, 60)]), -1, "strictly inside");
        assert_eq!(nearest_in_range(0, &[(7, 0)]), -1);
        assert_eq!(nearest_in_range(100, &[]), -1);
    }
}
