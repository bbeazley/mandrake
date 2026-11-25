use crate::attack::TargetCommand;
use crate::draw_site;
use crate::map_actions::{combined, find, index_of};
use crate::{Stop, TflSim};

const MAX_LISTED: usize = 0x12c;
const HEAL_FLAGS: u16 = 2;

fn worst_hurt(threshold: i32, allies: &[(i16, i16)]) -> Option<Option<usize>> {
    let mut best = 0x10000i32;
    let mut target = None;
    for (k, &(v, most)) in allies.iter().enumerate() {
        if most == 0 {
            return None;
        }
        let f = (i32::from(v) << 16).wrapping_div(i32::from(most));
        if f < threshold && f < best {
            best = f;
            target = Some(k);
        }
    }
    Some(target)
}

impl TflSim {
    fn shaman_list(&self, i: usize, name: &[u8; 4]) -> Vec<usize> {
        let ids = combined(&self.map_actions.actions, i, name, 2, MAX_LISTED);
        self.monster_slots(&ids)
            .into_iter()
            .filter(|&s| self.monsters[s].flags & 0x4000_0000 == 0)
            .collect()
    }

    pub(crate) fn shaman_action(&mut self, i: usize) -> Result<u32, Stop> {
        let subjects = self.shaman_list(i, b"subj");
        if subjects.is_empty() {
            return Ok(0);
        }
        let allies = self.shaman_list(i, b"ally");
        if allies.is_empty() {
            return Ok(0);
        }
        let Some(threshold) = find(&self.map_actions.actions, i, 0, b"<vit", 8)
            .filter(|p| p.count > 0)
            .map(|p| p.i32_at(0))
        else {
            return Ok(0);
        };
        let hurt: Vec<(i16, i16)> = allies
            .iter()
            .map(|&m| (self.monsters[m].vitality, self.monsters[m].max_vitality))
            .collect();
        let Some(target) = worst_hurt(threshold, &hurt) else {
            return Err(Stop::new(
                draw_site::SHAMAN_VITALITY_ZERO,
                "a sham ally's most vitality is 0: TFL 1.5's idiv faults",
            ));
        };
        let target = target.map(|k| allies[k]);
        crate::tc2_trace::log(|| {
            format!(
                "SH {} {i} {} {}",
                self.tick,
                subjects[0],
                target.map_or(-1, |t| t as i32)
            )
        });
        if let Some(t) = target {
            let command = TargetCommand {
                flags: HEAL_FLAGS,
                field_2: 0,
            };
            self.target_command(i as i16, command, &subjects[..1], &[t])?;
            return Ok(2);
        }
        let fall = find(&self.map_actions.actions, i, 0, b"fall", 3)
            .filter(|p| p.count > 0)
            .map(|p| p.i16_at(0));
        if let Some(j) = fall.and_then(|id| index_of(&self.map_actions.actions, id)) {
            self.trigger(j)?;
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::worst_hurt;

    #[test]
    fn worst_hurt_below_the_threshold_first_of_equals() {
        let a = [(10, 10), (3, 10), (3, 10), (5, 10)];
        assert_eq!(worst_hurt(0x8000, &a), Some(Some(1)));
        assert_eq!(worst_hurt(0x4000, &[(10, 10), (5, 10)]), Some(None));
        assert_eq!(
            worst_hurt(0x20000, &[(10, 10)]),
            Some(None),
            "best starts at 0x10000"
        );
        assert_eq!(
            worst_hurt(0x8000, &[(-2, 10), (1, 10)]),
            Some(Some(0)),
            "signed"
        );
        assert_eq!(worst_hurt(0x8000, &[(1, 10), (1, 0)]), None, "idiv by 0");
    }
}
