use crate::world_view::WorldView;
use mandrake_common::FourCC;
use mandrake_render::water::ripples::Disturbance;
use std::collections::{HashMap, HashSet};

const UNIT_WAKE: f32 = 0.6;
const UNIT_JUMP: f32 = 1.0;
const UNIT_RADIUS: f32 = 0.25;
const SPLASH: f32 = 0.06;
const SPLASH_RADIUS: f32 = 0.35;
const WADE: f32 = 0.02;
const WADE_RADIUS: f32 = 0.25;

#[derive(Debug, Default)]
pub(crate) struct RippleSources {
    seen: HashSet<(i32, FourCC)>,
    next_seen: HashSet<(i32, FourCC)>,
    units: HashMap<u32, [f32; 3]>,
    next_units: HashMap<u32, [f32; 3]>,
}

impl RippleSources {
    pub(crate) fn gather(
        &mut self,
        world: &WorldView,
        effects: &[(usize, FourCC, i16)],
    ) -> Vec<Disturbance> {
        let mut out = Vec::new();
        self.next_seen.clear();
        for o in &world.objects {
            let Some(&(kind, _, _)) = effects
                .iter()
                .find(|(_, c, s)| *c == o.collection && i32::from(*s) == i32::from(o.sequence))
            else {
                continue;
            };
            let key = (o.object, o.collection);
            self.next_seen.insert(key);
            if self.seen.contains(&key) {
                continue;
            }
            let (amount, radius) = if kind == mandrake_tag::media::MEDIA_GROUP_MONSTER {
                (WADE, WADE_RADIUS)
            } else {
                (SPLASH, SPLASH_RADIUS)
            };
            out.push(Disturbance {
                position: o.position,
                amount,
                radius,
            });
        }
        std::mem::swap(&mut self.seen, &mut self.next_seen);
        self.next_units.clear();
        for u in &world.units {
            self.next_units.insert(u.id, u.position);
            let Some(last) = self.units.get(&u.id) else {
                continue;
            };
            let (dx, dz) = (u.position[0] - last[0], u.position[2] - last[2]);
            let moved = (dx * dx + dz * dz).sqrt();
            if moved > 0.0 && moved < UNIT_JUMP {
                out.push(Disturbance {
                    position: u.position,
                    amount: UNIT_WAKE * moved,
                    radius: UNIT_RADIUS,
                });
            }
        }
        std::mem::swap(&mut self.units, &mut self.next_units);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_view::ViewUnit;

    fn unit(id: u32, position: [f32; 3]) -> ViewUnit {
        ViewUnit {
            id,
            archetype: FourCC::new(*b"wam1"),
            team: 0,
            selected: false,
            position,
            engine_facing: 0,
            state_slot: mandrake_tag::MonsterSequenceSlot(0),
            still: true,
            health: 1.0,
            max_health: 1.0,
            object: None,
            monster: None,
            colour_row: None,
            pick: None,
        }
    }

    #[test]
    fn a_unit_disturbs_the_water_as_it_moves() {
        let mut sources = RippleSources::default();
        let mut world = WorldView {
            units: vec![unit(7, [1.0, 0.0, 1.0])],
            ..WorldView::default()
        };
        assert!(
            sources.gather(&world, &[]).is_empty(),
            "no last position yet"
        );
        world.units[0].position = [1.25, 0.0, 1.0];
        let d = sources.gather(&world, &[]);
        assert_eq!(d.len(), 1);
        assert!((d[0].amount - UNIT_WAKE * 0.25).abs() < 1e-6);
        assert!(sources.gather(&world, &[]).is_empty(), "standing still");
        world.units[0].position = [9.0, 0.0, 1.0];
        assert!(
            sources.gather(&world, &[]).is_empty(),
            "a jump is no movement"
        );
    }
}
