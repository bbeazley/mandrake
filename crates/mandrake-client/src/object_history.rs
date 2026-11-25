use mandrake_math::engine::arctangent;
use mandrake_sim::engine_move::signed_angular_difference;
use mandrake_sim::views::ObjectKind;
use mandrake_sim::{ProjectileView, Simulation};

use crate::object_sprites::DrawnObject;

pub const FRACTION_ONE: i32 = 0x10000;

pub const TFL_STRIDE_JUMP: i32 = 1024;
pub const MYTH2_STRIDE_JUMP: i32 = mandrake_rec::WORLD_UNITS_PER_CELL;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stride {
    pub identifier: i64,
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub start: u32,
    pub duration: u32,
    pub step: u32,
}

impl Stride {
    pub fn start(identifier: i64, point: [i32; 3], tick: u32) -> Self {
        Self {
            identifier,
            from: point,
            to: point,
            start: tick,
            duration: 1,
            step: tick,
        }
    }

    pub fn at(&self, time: i64) -> [i32; 3] {
        let one = i64::from(FRACTION_ONE);
        let span = i64::from(self.duration.max(1)) * one;
        let into = (time - i64::from(self.start) * one).clamp(0, span);
        let mut p = self.to;
        for (k, v) in p.iter_mut().enumerate() {
            let (a, b) = (i64::from(self.from[k]), i64::from(self.to[k]));
            *v = (a + (b - a) * into / span) as i32;
        }
        p
    }

    pub fn at_tick(&self, tick: u32, fraction: i32) -> Option<[i32; 3]> {
        if self.duration <= 1 {
            return None;
        }
        let fraction = fraction.clamp(0, FRACTION_ONE);
        let time = (i64::from(tick) - 1) * i64::from(FRACTION_ONE) + i64::from(fraction);
        Some(self.at(time))
    }

    pub fn next(
        self,
        identifier: i64,
        point: [i32; 3],
        ticks_per_frame: i8,
        tick: u32,
        jump_limit: i32,
    ) -> Self {
        if identifier != self.identifier {
            return Self::start(identifier, point, tick);
        }
        if point == self.to {
            return self;
        }
        let before = tick.saturating_sub(1);
        let jump = (0..2).any(|k| (point[k] - self.to[k]).abs() > jump_limit);
        let tpf = u32::try_from(ticks_per_frame).unwrap_or(1).max(1);
        Self {
            identifier,
            from: if jump {
                self.to
            } else {
                self.at(i64::from(before) * i64::from(FRACTION_ONE))
            },
            to: point,
            start: before,
            duration: if jump {
                1
            } else {
                tick.saturating_sub(self.step).clamp(1, tpf)
            },
            step: tick,
        }
    }
}

pub fn frame_fraction(ratio: f32) -> i32 {
    if ratio <= 0.0 {
        0
    } else if ratio > 1.0 {
        FRACTION_ONE
    } else {
        (ratio * 65536.0) as i32
    }
}

pub fn shows_history_shape(fraction: i32) -> bool {
    fraction <= 0x7fff
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ObjectMotion {
    pub world: [i32; 3],
    pub velocity: [i16; 3],
    pub facing: u16,
    pub elevation: u16,
    pub scale: i16,
}

pub(crate) fn interpolate_i16(history: i16, live: i16, fraction: i32) -> i16 {
    (((i32::from(live) - i32::from(history)).wrapping_mul(fraction) >> 16) + i32::from(history))
        as i16
}

fn interpolate_angle(history: u16, live: u16, fraction: i32) -> u16 {
    (signed_angular_difference(history, live).wrapping_mul(fraction) >> 16)
        .wrapping_add(i32::from(history)) as u16
}

impl ObjectMotion {
    pub fn interpolated(history: &Self, live: &Self, fraction: i32) -> Self {
        if fraction > 0xffff {
            return *live;
        }
        let world = std::array::from_fn(|i| {
            let delta = i64::from(live.world[i].wrapping_sub(history.world[i]));
            ((delta * i64::from(fraction)) >> 16) as i32 + history.world[i]
        });
        let velocity = std::array::from_fn(|i| {
            interpolate_i16(history.velocity[i], live.velocity[i], fraction)
        });
        Self {
            world,
            velocity,
            facing: interpolate_angle(history.facing, live.facing, fraction),
            elevation: interpolate_angle(history.elevation, live.elevation, fraction),
            scale: interpolate_i16(history.scale, live.scale, fraction),
        }
    }
}

fn drawn_motion(o: &DrawnObject) -> ObjectMotion {
    ObjectMotion {
        world: o.world,
        velocity: o.velocity,
        facing: o.facing,
        elevation: 0,
        scale: o.scale as i16,
    }
}

pub(crate) fn interpolated_drawn(
    history: &DrawnObject,
    live: &DrawnObject,
    fraction: i32,
) -> DrawnObject {
    let m = ObjectMotion::interpolated(&drawn_motion(history), &drawn_motion(live), fraction);
    let shape = if shows_history_shape(fraction) {
        history
    } else {
        live
    };
    DrawnObject {
        world: m.world,
        velocity: m.velocity,
        facing: m.facing,
        scale: i32::from(m.scale as u16),
        collection: shape.collection,
        sequence: shape.sequence,
        frame: shape.frame,
        view: shape.view,
        ..live.clone()
    }
}

fn view_motion(v: &ProjectileView) -> ObjectMotion {
    ObjectMotion {
        world: v.world,
        velocity: v.velocity,
        facing: v.facing,
        elevation: v.elevation,
        scale: v.scale as i16,
    }
}

fn interpolated_projectile(
    sim: &Simulation,
    history: &ProjectileView,
    live: &ProjectileView,
    fraction: i32,
) -> ProjectileView {
    let m = ObjectMotion::interpolated(&view_motion(history), &view_motion(live), fraction);
    let shape = if shows_history_shape(fraction) {
        history
    } else {
        live
    };
    let mut v = shape.clone();
    v.world = m.world;
    v.position = sim.world_units_to_sim(m.world);
    v.velocity = m.velocity;
    v.facing = m.facing;
    v.elevation = m.elevation;
    v.scale = i32::from(m.scale);
    v
}

pub fn drawn_projectile_views(sim: &Simulation, fraction: i32) -> Vec<ProjectileView> {
    let drawn = sim.drawn_projectile_views();
    let mut out: Vec<ProjectileView> = if fraction == 0 {
        drawn
    } else {
        drawn
            .iter()
            .zip(sim.projectile_views())
            .map(|(h, live)| interpolated_projectile(sim, h, &live, fraction))
            .collect()
    };
    out.extend(marker_projectile_views(sim));
    out.extend(fade_copy_views(sim, fraction));
    out
}

pub fn fade_copy_views(sim: &Simulation, fraction: i32) -> Vec<ProjectileView> {
    if shows_history_shape(fraction) {
        sim.drawn_fade_copy_views()
    } else {
        sim.fade_copy_views()
    }
}

pub fn marker_projectile_views(sim: &Simulation) -> Vec<ProjectileView> {
    let w = &sim.engine;
    w.projectiles
        .iter()
        .filter(|p| p.object.kind == 4)
        .map(|p| {
            let o = &p.object;
            let world = [p.position.0, p.position.1, o.z];
            let v = o.velocity;
            let heading = if v[0] == 0 && v[1] == 0 {
                p.facing
            } else {
                arctangent(i32::from(v[0]), i32::from(v[1]))
            };
            let ground = (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2)).sqrt() as i32;
            ProjectileView {
                kind: ObjectKind::Dormant,
                id: 0,
                record: -1,
                object: o.index,
                definition: p.definition,
                owner: 0,
                target: None,
                world,
                position: sim.world_units_to_sim(world),
                velocity: v,
                facing: p.facing,
                elevation: p.elevation,
                heading,
                pitch: arctangent(ground, i32::from(v[2])),
                launched_tick: 0,
                collection: o.collection,
                sequence: o.sequence,
                frame: o.frame,
                view: o.view,
                scale: o.scale,
                flags: o.flags,
                hidden: o.no_shape,
                mirror_x: o.flags & 0x2_0000 != 0,
                mirror_y: o.flags & 0x4_0000 != 0,
                permutation: o.permutation,
                shading_slot: o.shading_slot,
                core: o.core,
                transfer: (o.sub_kind, o.sub_count, o.sub_limit),
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParasiteView {
    pub host: u32,
    pub view: ProjectileView,
}

pub fn parasite_views(sim: &Simulation) -> Vec<ParasiteView> {
    let w = &sim.engine;
    let mut out: Vec<(i32, ParasiteView)> = w
        .fade_records
        .iter()
        .filter(|r| {
            r.host != mandrake_sim::engine_rng::FadeRecord::NO_HOST && r.object.sub_count > 0
        })
        .filter_map(|r| {
            let m = w.monsters.get(r.host)?;
            let host = m.unit_key()?;
            let o = &r.object;
            let world = [m.position.0, m.position.1, m.object.z];
            let v = m.object.velocity;
            let heading = if v[0] == 0 && v[1] == 0 {
                m.facing
            } else {
                arctangent(i32::from(v[0]), i32::from(v[1]))
            };
            let ground = (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2)).sqrt() as i32;
            let view = ProjectileView {
                kind: ObjectKind::Copy,
                id: 0,
                record: r.index,
                object: o.index,
                definition: mandrake_common::FourCC::new([0; 4]),
                owner: 0,
                target: None,
                world,
                position: sim.world_units_to_sim(world),
                velocity: v,
                facing: m.facing,
                elevation: 0,
                heading,
                pitch: arctangent(ground, i32::from(v[2])),
                launched_tick: 0,
                collection: o.collection,
                sequence: o.sequence,
                frame: o.frame,
                view: o.view,
                scale: o.scale,
                flags: o.flags,
                hidden: o.no_shape,
                mirror_x: o.flags & 0x2_0000 != 0,
                mirror_y: o.flags & 0x4_0000 != 0,
                permutation: o.permutation,
                shading_slot: o.shading_slot,
                core: o.core,
                transfer: (o.sub_kind, o.sub_count, o.sub_limit),
            };
            Some((r.index, ParasiteView { host, view }))
        })
        .collect();
    out.sort_by_key(|(record, _)| *record);
    out.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drawn_object_between_ticks_is_between_its_two_states() {
        let object = |world: [i32; 3], facing: u16, frame: i32| {
            crate::tfl_objects::drawn_object(
                3,
                world,
                (64, 64),
                (facing, 0, frame),
                (mandrake_common::FourCC::new(*b"dwar"), 2),
                (0, false, 0x100),
                None,
            )
        };
        let before = object([1000, 2000, 64], 0xf000, 4);
        let mut after = object([1040, 1960, 96], 0x1000, 5);
        after.scale = 0x180;
        after.mirror_x = true;
        let at = |f| interpolated_drawn(&before, &after, f);
        assert_eq!(at(0), {
            let mut o = before.clone();
            o.mirror_x = true;
            o.position = after.position;
            o
        });
        let quarter = at(0x4000);
        assert_eq!(quarter.world, [1010, 1990, 72]);
        assert_eq!(
            (quarter.facing, quarter.scale, quarter.frame),
            (0xf800, 0x120, 4)
        );
        let half = at(0x8000);
        assert_eq!(half.world, [1020, 1980, 80]);
        assert_eq!((half.facing, half.scale, half.frame), (0, 0x140, 5));
        assert_eq!((at(0xc000).world, at(0xc000).frame), ([1030, 1970, 88], 5));
        assert_eq!(at(FRACTION_ONE), after);
    }

    fn motion(x: i32, facing: u16) -> ObjectMotion {
        ObjectMotion {
            world: [x, -x, 100],
            velocity: [8, -8, 0],
            facing,
            elevation: 0,
            scale: 0x100,
        }
    }

    #[test]
    fn fraction_zero_is_the_history_and_one_the_live_object() {
        let (h, l) = (motion(1000, 10), motion(1100, 40));
        assert_eq!(ObjectMotion::interpolated(&h, &l, 0), h);
        assert_eq!(ObjectMotion::interpolated(&h, &l, FRACTION_ONE), l);
    }

    #[test]
    fn half_way_rounds_towards_minus_infinity_like_sar() {
        let h = motion(1000, 0);
        let l = motion(1101, 0);
        let m = ObjectMotion::interpolated(&h, &l, 0x8000);
        assert_eq!(m.world[0], 1050);
        assert_eq!(m.world[1], -1051);
    }

    #[test]
    fn positions_do_not_overflow_on_a_long_move() {
        let h = motion(0, 0);
        let l = motion(0x40000, 0);
        let m = ObjectMotion::interpolated(&h, &l, 0xc000);
        assert_eq!(m.world[0], 0x30000);
    }

    #[test]
    fn facing_takes_the_short_way_round() {
        let h = motion(0, 0xfff0);
        let l = motion(0, 0x0010);
        assert_eq!(ObjectMotion::interpolated(&h, &l, 0x8000).facing, 0);
    }

    #[test]
    fn the_shape_switches_to_the_live_object_at_one_half() {
        assert!(shows_history_shape(0));
        assert!(shows_history_shape(0x7fff));
        assert!(!shows_history_shape(0x8000));
    }

    #[test]
    fn a_new_object_in_a_reused_slot_is_not_drawn_from_the_old_occupants_motion() {
        use mandrake_common::FourCC;
        use mandrake_sim::engine_rng::{EngineObject, EngineWorld};
        use mandrake_sim::flight::DormantObject;

        let dormant = |index, position, collection| DormantObject {
            definition: collection,
            object: EngineObject::new(index, collection),
            position,
            facing: 0,
            record_36: -1,
            flags_3d: 0,
            elevation: 0,
            cell_link: 0,
        };
        let (arrow, ripple) = (FourCC::new(*b"arch"), FourCC::new(*b"wate"));
        let (from, to) = ([39287, 78649, -702], [60229, 51598, -1203]);
        let mut sim = Simulation::new(0);
        let mut world = EngineWorld::default();
        let (slot, _) = world.new_object_datum();
        sim.set_engine_world(world);
        sim.combat.flight.dormant.push(dormant(slot, from, arrow));
        sim.run_command_stage();

        sim.combat.flight.dormant.clear();
        let world = &mut sim.engine;
        world.free_objects.insert(slot);
        assert_eq!(world.new_object_datum().0, slot);
        sim.combat.flight.dormant.push(dormant(slot, to, ripple));

        for fraction in [0, 0x4000, 0x7fff, 0x8000, 0xc000, 0xffff, FRACTION_ONE] {
            let drawn = drawn_projectile_views(&sim, fraction);
            assert_eq!(drawn[0].world, to, "fraction {fraction:#x}");
            assert_eq!(drawn[0].collection, ripple, "fraction {fraction:#x}");
        }

        sim.run_command_stage();
        sim.combat.flight.dormant[0].position = [to[0] + 100, to[1], to[2]];
        let drawn = drawn_projectile_views(&sim, 0x8000);
        assert_eq!(drawn[0].world, [to[0] + 50, to[1], to[2]]);
    }

    #[test]
    fn frame_fraction_follows_begin_motion_interpolated_frame() {
        assert_eq!(frame_fraction(0.0), 0);
        assert_eq!(frame_fraction(0.5), 0x8000);
        assert_eq!(frame_fraction(1.5), FRACTION_ONE);
    }
}
