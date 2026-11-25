use super::meander_tests::{action, parameter, world};
use super::*;
use crate::engine_rng::SiteCounts;
use mandrake_math::BungieRng;

fn run(extra: Vec<MapActionParameter>) -> (Result<Vec<usize>, Unmodelled>, u32) {
    let mut parameters = vec![parameter(b"acti", TYPE_ACTIONS, &[201, 200])];
    parameters.extend(extra);
    let mut a = MapActions {
        actions: vec![
            action(200, b"acli", vec![]),
            action(201, b"acli", vec![]),
            action(100, b"acli", parameters),
        ],
        ..MapActions::default()
    };
    let mut w = world(vec![]);
    let (mut rng, mut sites) = (BungieRng::new(7), SiteCounts::default());
    let mut draws = BungieRng::new(7);
    let r = a.trigger(
        2,
        &mut MapActionContext {
            world: &mut w,
            rng: &mut EngineRng {
                rng: &mut rng,
                sites: &mut sites,
            },
            time: 640,
        },
    );
    let mut n = 0;
    while draws.state != rng.state {
        draws.next_u16();
        n += 1;
        assert!(n < 16);
    }
    let active = (0..2)
        .filter(|&s| a.actions[s].flags & FLAG_ACTIVE != 0)
        .collect();
    (r.map(|_| active), n)
}

#[test]
fn only_reads_acch_by_the_activated_slot() {
    let (r, draws) = run(vec![
        parameter(b"only", TYPE_FLAG, &[]),
        parameter(b"acch", TYPE_FIXED, &[0, 0x1_0000]),
    ]);
    assert_eq!(r.unwrap(), vec![1]);
    assert_eq!(draws, 1);
}

#[test]
fn plain_acch_reads_by_position() {
    let (r, draws) = run(vec![parameter(b"acch", TYPE_FIXED, &[0, 0x1_0000])]);
    assert_eq!(r.unwrap(), vec![0]);
    assert_eq!(draws, 2);
}

#[test]
fn only_with_a_slot_past_acch_is_unmodelled() {
    let mut parameters = vec![parameter(b"acti", TYPE_ACTIONS, &[300, 301])];
    parameters.push(parameter(b"only", TYPE_FLAG, &[]));
    parameters.push(parameter(b"acch", TYPE_FIXED, &[0x1_0000, 0x1_0000]));
    let mut a = MapActions {
        actions: vec![
            action(200, b"acli", vec![]),
            action(201, b"acli", vec![]),
            action(100, b"acli", parameters),
            action(300, b"acli", vec![]),
            action(301, b"acli", vec![]),
        ],
        ..MapActions::default()
    };
    let mut w = world(vec![]);
    let (mut rng, mut sites) = (BungieRng::new(7), SiteCounts::default());
    let r = a.trigger(
        2,
        &mut MapActionContext {
            world: &mut w,
            rng: &mut EngineRng {
                rng: &mut rng,
                sites: &mut sites,
            },
            time: 640,
        },
    );
    assert_eq!(
        r.unwrap_err().site,
        crate::draw_site::ACTION_LIST_CHANCE_PAST_END
    );
}
