use crate::draw_site;
use crate::engine_rng::Unmodelled;
use crate::map_actions::{MapActionContext, MapActions, TYPE_FIXED, TYPE_INTEGER, TYPE_MONSTERS};

const MAX_SUBJECTS: usize = 400;

const TYPE_TAG_NAME: i16 = 7;

pub const SITE_DELAY: u32 = draw_site::DELAY_ACTION_DELAY;

const fn tag(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

impl MapActions {
    pub fn delay_action(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
    ) -> Result<u32, Unmodelled> {
        if self.indexed(i, b"subj", TYPE_MONSTERS, 0).is_none() {
            return Ok(0);
        }
        let subjects: Vec<i32> = self
            .combined(i, b"subj", TYPE_MONSTERS, MAX_SUBJECTS)
            .into_iter()
            .filter_map(|id| ctx.world.monster_index_by_identifier(id as u16))
            .filter(|&m| ctx.world.monsters[m].flags & 0x4000_0000 == 0)
            .map(|m| i32::from(ctx.world.monsters[m].identifier))
            .collect();
        let n = subjects.len();
        if n == 0 {
            return Ok(0);
        }
        let Some(time) = self.long(i, b"time", TYPE_FIXED) else {
            return Ok(0);
        };
        let now = ctx.time;
        let Some(del) = self.find_ref(i, 3, b"#del", Some(TYPE_INTEGER)) else {
            self.start_delays(i, ctx, time, now, n);
            return Ok(0);
        };
        let Some(resu) = self.long(i, b"resu", crate::map_actions::TYPE_ACTIONS) else {
            return Ok(0);
        };
        let Some(target) = self.index_of(i32::from(resu as i16)) else {
            return Ok(0);
        };
        let param = &self.actions[del.0].parameters[del.1];
        if n > param.count.max(0) as usize {
            return Err(Unmodelled::new(
                draw_site::DELAY_ACTION_SUBJECT_COUNT,
                format!(
                    "delay_action_proc: {n} subjects, '#del' of action {i} has {}",
                    param.count
                ),
            ));
        }
        let mut entries: Vec<i32> = param.values[..n].to_vec();
        let mut all_done = true;
        let mut released = Vec::new();
        for (e, &id) in entries.iter_mut().zip(&subjects) {
            if *e != -1 {
                all_done = false;
                if *e < now {
                    *e = -1;
                    released.push(id);
                }
            }
        }
        let p = &mut self.actions[del.0].parameters[del.1];
        p.count = n as i16;
        p.values = entries;
        if !released.is_empty() {
            let name = self
                .long(i, b"type", TYPE_TAG_NAME)
                .map_or(tag(b"subj"), |v| v as u32);
            if self.replace_parameter(target, name, TYPE_MONSTERS) {
                for &id in &released {
                    self.append_parameter(target, &[i32::from(id as i16)]);
                }
            }
            self.activate(Some(target), ctx)?;
        }
        Ok(if all_done { 2 } else { 0 })
    }

    fn start_delays(
        &mut self,
        i: usize,
        ctx: &mut MapActionContext,
        time: i32,
        now: i32,
        n: usize,
    ) {
        if !self.replace_parameter(i, tag(b"#del"), TYPE_INTEGER) {
            return;
        }
        let t = time.wrapping_mul(0x1e);
        let ticks = if t < 0 { t.wrapping_add(0xffff) } else { t } >> 16;
        if self.flag(i, b"dist") {
            let mut acc = ticks;
            for _ in 0..n {
                let v = (acc / n as i32).wrapping_add(now);
                self.append_parameter(i, &[v]);
                acc = acc.wrapping_add(ticks);
            }
        } else if t > 0xffff {
            for _ in 0..n {
                let r = i32::from(ctx.rng.draw(SITE_DELAY));
                self.append_parameter(i, &[now.wrapping_add(r.wrapping_mul(ticks) >> 16)]);
            }
        } else {
            for _ in 0..n {
                self.append_parameter(i, &[now]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_rng::{
        EngineMonster, EngineObject, EngineRng, EngineWorld, MonsterFormation,
    };
    use crate::map_actions::{MapAction, TYPE_ACTIONS};
    use mandrake_common::FourCC;
    use mandrake_math::rng::BungieRng;
    use mandrake_tag::map_action::MapActionParameter;

    fn monster(index: usize, identifier: i16) -> EngineMonster {
        EngineMonster {
            index,
            identifier,
            first_draw: 0,
            definition: 0,
            object: EngineObject::new(index as i32, FourCC::new(*b"test")),
            mode: 1,
            anim_state: 0x1a,
            speed: 0,
            position: (0, 0),
            flags: 0,
            flags_8: 0,
            neighbours: 0,
            impassable: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team: 0,
            owner: 0,
            facing: 0,
            formation: MonsterFormation::default(),
            cell_link: 0,
            flying_height: 0,
        }
    }

    fn param(name: &[u8; 4], kind: i16, values: Vec<i32>) -> MapActionParameter {
        MapActionParameter {
            kind,
            count: values.len() as i16,
            name: FourCC::new(*name),
            values,
        }
    }

    fn action(identifier: u16, tag: &[u8; 4], parameters: Vec<MapActionParameter>) -> MapAction {
        MapAction {
            identifier,
            kind: 2,
            tag: u32::from_be_bytes(*tag),
            flags: 0,
            delay: 0,
            delay_range: 0,
            time: 0,
            parameters,
        }
    }

    fn setup(time: i32) -> (EngineWorld, MapActions) {
        let mut w = EngineWorld::default();
        w.monsters = (0..3).map(|i| monster(i, 100 + i as i16)).collect();
        let mut actions = MapActions::default();
        actions.actions = vec![
            action(
                1,
                b"dela",
                vec![
                    param(b"subj", TYPE_MONSTERS, vec![100, 101, 102]),
                    param(b"time", TYPE_FIXED, vec![time]),
                    param(b"resu", TYPE_ACTIONS, vec![2]),
                ],
            ),
            action(2, b"atta", vec![param(b"subj", TYPE_MONSTERS, vec![7])]),
        ];
        (w, actions)
    }

    fn call(w: &mut EngineWorld, a: &mut MapActions, time: i32) -> (Result<u32, Unmodelled>, u64) {
        let mut rng = BungieRng { state: 1 };
        let mut sites = crate::engine_rng::SiteCounts::default();
        let r = a.delay_action(
            0,
            &mut MapActionContext {
                world: w,
                rng: &mut EngineRng {
                    rng: &mut rng,
                    sites: &mut sites,
                },
                time,
            },
        );
        (r, sites.total())
    }

    fn del(a: &MapActions) -> Vec<i32> {
        a.find(0, 3, b"#del", Some(TYPE_INTEGER))
            .unwrap()
            .values
            .clone()
    }

    #[test]
    fn a_short_time_releases_everyone_at_the_next_call() {
        let (mut w, mut a) = setup(2000);
        assert_eq!(call(&mut w, &mut a, 64), (Ok(0), 0));
        assert_eq!(del(&a), vec![64, 64, 64]);
        assert_eq!(call(&mut w, &mut a, 64), (Ok(0), 0));
        assert_eq!(del(&a), vec![64, 64, 64], "not before the tick itself");
        assert_eq!(call(&mut w, &mut a, 96), (Ok(0), 0));
        assert_eq!(del(&a), vec![-1, -1, -1]);
        assert_eq!(
            a.combined(1, b"subj", TYPE_MONSTERS, 400),
            vec![100, 101, 102]
        );
        assert_ne!(a.actions[1].flags & crate::map_actions::FLAG_ACTIVE, 0);
        assert_eq!(call(&mut w, &mut a, 128), (Ok(2), 0));
    }

    #[test]
    fn entries_go_by_position_against_the_calls_subjects() {
        let (mut w, mut a) = setup(2000);
        a.actions[0].parameters.push(param(b"dist", 0, vec![]));
        a.actions[0].parameters[1].values[0] = 0x10000 * 3;
        assert_eq!(call(&mut w, &mut a, 0), (Ok(0), 0));
        assert_eq!(del(&a), vec![30, 60, 90]);
        w.monsters[0].flags |= 0x4000_0000;
        assert_eq!(call(&mut w, &mut a, 32), (Ok(0), 0));
        assert_eq!(del(&a), vec![-1, 60]);
        assert_eq!(a.combined(1, b"subj", TYPE_MONSTERS, 400), vec![101]);
        w.monsters[0].flags = 0;
        assert!(call(&mut w, &mut a, 64).0.is_err());
    }

    #[test]
    fn missing_inputs_end_with_zero() {
        let (mut w, mut a) = setup(2000);
        a.actions[0].parameters.retain(|p| p.name.0 != *b"time");
        assert_eq!(call(&mut w, &mut a, 0), (Ok(0), 0));
        assert!(a.find(0, 3, b"#del", None).is_none());
        let (mut w, mut a) = setup(2000);
        a.actions[0].parameters.retain(|p| p.name.0 != *b"resu");
        assert_eq!(call(&mut w, &mut a, 0), (Ok(0), 0));
        assert_eq!(call(&mut w, &mut a, 32), (Ok(0), 0));
        assert_eq!(del(&a), vec![0, 0, 0]);
        let (mut w, mut a) = setup(2000);
        for m in &mut w.monsters {
            m.flags |= 0x4000_0000;
        }
        assert_eq!(call(&mut w, &mut a, 0), (Ok(0), 0));
        assert!(a.find(0, 3, b"#del", None).is_none());
    }
}
