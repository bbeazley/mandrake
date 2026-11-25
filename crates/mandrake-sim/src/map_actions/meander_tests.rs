use mandrake_common::FourCC;
use mandrake_math::BungieRng;

use super::*;
use crate::engine_rng::{
    DRAW_LOG, EngineMonster, EngineObject, MonsterDefinition, MonsterFormation, SiteCounts,
};

pub(super) fn monster(index: usize, identifier: i16, position: (i32, i32)) -> EngineMonster {
    EngineMonster {
        index,
        identifier,
        first_draw: 0,
        definition: 0,
        object: EngineObject::new(index as i32, FourCC::new(*b"test")),
        mode: 1,
        anim_state: 0x1a,
        speed: 0,
        position,
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

pub(super) fn world(monsters: Vec<EngineMonster>) -> EngineWorld {
    let mut w = EngineWorld::default();
    w.mode = 2;
    w.mesh_cells = (256, 256);
    w.monster_definitions =
        vec![MonsterDefinition::from_bytes(FourCC::new(*b"wam1"), &vec![0u8; 0x224]).unwrap()];
    w.monsters = monsters;
    w
}

pub(super) fn parameter(name: &[u8; 4], kind: i16, values: &[i32]) -> MapActionParameter {
    let per = MapActionParameter::scalars_per_element(kind).max(1);
    MapActionParameter {
        kind,
        count: (values.len() / per) as i16,
        name: FourCC::new(*name),
        values: values.to_vec(),
    }
}

pub(super) fn action(
    identifier: u16,
    tag: &[u8; 4],
    parameters: Vec<MapActionParameter>,
) -> MapAction {
    MapAction {
        identifier,
        kind: 3,
        tag: u32::from_be_bytes(*tag),
        flags: 4,
        delay: 0,
        delay_range: 0,
        time: 0,
        parameters,
    }
}

#[allow(dead_code)]
struct Run {
    result: Result<u32, Unmodelled>,
    rng: u32,
    draws: Vec<u32>,
}

fn run(actions: &mut MapActions, w: &mut EngineWorld, i: usize, state: u32, time: i32) -> Run {
    let mut rng = BungieRng::new(0);
    rng.state = state;
    let mut sites = SiteCounts::default();
    DRAW_LOG.with(|l| *l.borrow_mut() = Some(Vec::new()));
    let result = {
        let mut ctx = MapActionContext {
            world: w,
            rng: &mut EngineRng {
                rng: &mut rng,
                sites: &mut sites,
            },
            time,
        };
        actions.meander(i, &mut ctx)
    };
    let draws = DRAW_LOG
        .with(|l| l.borrow_mut().take())
        .unwrap()
        .iter()
        .map(|d| d.0)
        .collect();
    Run {
        result,
        rng: rng.state,
        draws,
    }
}

#[test]
fn a_first_run_without_waypoints() {
    let mut actions = MapActions {
        actions: vec![action(100, b"mean", vec![parameter(b"subj", 2, &[7])])],
        ..MapActions::default()
    };
    let mut w = world(vec![monster(0, 7, (5000, 6000))]);
    let state = (0..u32::MAX)
        .find(|&s| {
            let mut b = BungieRng::new(0);
            b.state = s;
            b.next_u16().is_multiple_of(12)
        })
        .unwrap();
    let r = run(&mut actions, &mut w, 0, state, 64);
    assert_eq!(r.result, Ok(0));
    assert_eq!(
        r.draws,
        [
            crate::draw_site::MEANDER_TIMER,
            crate::draw_site::MEANDER_NO_WAYPOINTS,
            crate::draw_site::MEANDER_SPREAD_X,
            crate::draw_site::MEANDER_SPREAD_Y,
            crate::draw_site::MEANDER_WAIT
        ]
    );
    assert_eq!(w.pending_orders[0].waypoints, vec![(0, 0)]);
    assert_eq!(i32::from(w.pending_orders[0].action), 100);
    assert_eq!(actions.meander_glo[&0].subjects[0].timer, 1);
    assert_eq!(actions.meander_glo[&0].threat_cells, 3);
}

#[test]
fn too_many_waypoints() {
    let mut actions = MapActions {
        actions: vec![action(
            100,
            b"mean",
            vec![
                parameter(b"subj", 2, &[7]),
                parameter(b"wayp", 13, &[1; 2 * 0x21]),
            ],
        )],
        ..MapActions::default()
    };
    let mut w = world(vec![monster(0, 7, (5000, 6000))]);
    let r = run(&mut actions, &mut w, 0, 1, 64);
    assert_eq!(
        (r.result, r.draws),
        (Ok(0), vec![crate::draw_site::MEANDER_TIMER])
    );
    assert!(actions.meander_glo.is_empty());
}

#[test]
fn a_threat_makes_the_subject_run() {
    let mut actions = MapActions {
        actions: vec![action(
            100,
            b"mean",
            vec![
                parameter(b"subj", 2, &[7]),
                parameter(b"tscw", 5, &[2]),
                parameter(b"thre", 0, &[]),
            ],
        )],
        ..MapActions::default()
    };
    let at = (51200, 51200);
    let mut w = world(vec![
        monster(0, 7, at),
        monster(1, 8, (at.0 + 100, at.1)),
        monster(2, 9, (at.0 + 200, at.1)),
        monster(3, 10, (at.0 + 300, at.1)),
        monster(4, 11, (at.0, at.1 + 700)),
        monster(5, 12, (at.0, at.1 + 400)),
        monster(6, 13, (at.0 + 3 * 512, at.1)),
    ]);
    w.monsters[2].flags_8 = 4;
    w.monsters[3].object.no_shape = true;
    for m in &mut w.monsters[2..7] {
        m.team = 1;
    }
    actions.meander_glo.insert(
        0,
        MeanderState {
            subjects: vec![MeanderSubject {
                monster: 0,
                identifier: 7,
                timer: 5,
            }],
            threat_cells: 2,
            ..MeanderState::default()
        },
    );
    let r = run(&mut actions, &mut w, 0, 1, 64);
    assert_eq!((r.result, r.draws), (Ok(0), vec![]));
    let o = &w.pending_orders[0];
    assert_eq!(i32::from(o.action), -2);
    assert_eq!(o.waypoints, vec![(at.0, at.1 - 0x1000)]);
    assert_eq!(actions.meander_glo[&0].subjects[0].timer, 0);
}

#[test]
fn casualties() {
    let mut actions = MapActions {
        actions: vec![
            action(
                100,
                b"mean",
                vec![
                    parameter(b"subj", 2, &[7, 8]),
                    parameter(b"casu", 3, &[200]),
                ],
            ),
            action(200, b"soun", vec![]),
        ],
        ..MapActions::default()
    };
    let subject = |monster, identifier| MeanderSubject {
        monster,
        identifier,
        timer: 5,
    };
    let stored = MeanderState {
        subjects: vec![subject(0, 7), subject(1, 8)],
        threat_cells: 3,
        ..MeanderState::default()
    };
    actions.meander_glo.insert(0, stored.clone());
    let mut w = world(vec![monster(0, 7, (0, 0)), monster(1, 0, (0, 0))]);
    let r = run(&mut actions, &mut w, 0, 1, 64);
    assert_eq!(r.result, Ok(0));
    assert_eq!(actions.actions[1].flags & FLAG_ACTIVE, FLAG_ACTIVE);
    assert_eq!(
        actions.meander_glo[&0].subjects,
        vec![MeanderSubject {
            timer: 4,
            ..subject(0, 7)
        }]
    );
    let kept = actions.meander_glo[&0].clone();
    w.monsters[0].identifier = 99;
    let r = run(&mut actions, &mut w, 0, 1, 96);
    assert_eq!(r.result, Ok(4));
    assert_eq!(actions.meander_glo[&0], kept);
}
