use crate::support::param;

use mandrake_math::BungieRng;
use mandrake_sim::engine_rng::{EngineRng, EngineWorld, SiteCounts};
use mandrake_sim::map_actions::{FLAG_ACTIVE, MapAction, MapActions};
use mandrake_tag::MapActionParameter;

fn action(identifier: u16, tag: &[u8; 4], parameters: Vec<MapActionParameter>) -> MapAction {
    MapAction {
        identifier,
        kind: 0,
        tag: u32::from_be_bytes(*tag),
        flags: 1 | 4,
        delay: 0,
        delay_range: 0,
        time: 0,
        parameters,
    }
}

fn target(identifier: u16) -> MapAction {
    MapAction {
        identifier,
        kind: 3,
        tag: u32::MAX,
        flags: 4,
        delay: 0,
        delay_range: 0,
        time: 0,
        parameters: vec![],
    }
}

fn active(world: &EngineWorld, i: usize) -> bool {
    world.map_actions.as_ref().unwrap().actions[i].flags & FLAG_ACTIVE != 0
}

const ACOS: i16 = 3;

#[test]
fn ngty_team_reads_the_team_rankings() {
    use mandrake_sim::statistics::TeamRanking;
    let rank = |team: i16, rank: i16, score: i32| TeamRanking {
        team,
        rank,
        score,
        ..Default::default()
    };
    let result_in = |game_type: i16, rankings: Vec<TeamRanking>, extra: Vec<MapActionParameter>| {
        let mut p = vec![
            param(b"type", 5, &[13]),
            param(b"team", 5, &[0]),
            param(b"scor", 5, &[6]),
            param(b"acos", ACOS, &[0x8002u16 as i16 as i32]),
            param(b"acof", ACOS, &[0x8003u16 as i16 as i32]),
        ];
        p.extend(extra);
        let mut map_actions = MapActions::default();
        map_actions.actions = vec![action(0x8001, b"ngty", p), target(0x8002), target(0x8003)];
        let mut world = EngineWorld::default();
        world.game_type = game_type;
        world.statistics.team_rankings = rankings;
        world.map_actions = Some(map_actions);
        let mut rng = BungieRng::new(1);
        let mut sites = SiteCounts::default();
        world
            .update_map_actions(
                0,
                &mut EngineRng {
                    rng: &mut rng,
                    sites: &mut sites,
                },
            )
            .unwrap();
        assert_eq!(sites.total(), 0);
        let (s, f) = (active(&world, 1), active(&world, 2));
        assert!(s != f);
        if s { 2 } else { 0 }
    };
    let result = |rankings, extra| result_in(13, rankings, extra);
    let ties = || vec![param(b"ties", 0, &[])];
    assert_eq!(result(vec![rank(1, 0, 0), rank(0, 0, 0)], vec![]), 0);
    assert_eq!(result(vec![rank(0, 0, 6), rank(1, 1, 0)], vec![]), 2);
    assert_eq!(result(vec![rank(0, 0, 5), rank(1, 1, 0)], vec![]), 0);
    assert_eq!(result(vec![rank(1, 0, 6), rank(0, 0, 6)], vec![]), 0);
    assert_eq!(result(vec![rank(1, 0, 6), rank(0, 0, 6)], ties()), 2);
    assert_eq!(result(vec![rank(1, 0, 6), rank(0, 1, 6)], ties()), 0);
    assert_eq!(result_in(1, vec![rank(0, 0, 6), rank(1, 1, 0)], vec![]), 2);
    assert_eq!(
        result(
            vec![rank(0, 0, 6), rank(1, 1, 0)],
            vec![param(b"patc", 5, &[99])]
        ),
        0
    );
}
