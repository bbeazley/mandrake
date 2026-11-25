use super::meander_tests::{action, monster, parameter, world};
use super::*;
use crate::engine_rng::SiteCounts;
use mandrake_math::BungieRng;

fn run(w: &mut EngineWorld, tag: &[u8; 4], parameters: Vec<MapActionParameter>) -> u32 {
    let mut a = MapActions {
        actions: vec![action(100, tag, parameters)],
        ..MapActions::default()
    };
    let (mut rng, mut sites) = (BungieRng::new(7), SiteCounts::default());
    let before = rng.state;
    let r = a
        .trigger(
            0,
            &mut MapActionContext {
                world: w,
                rng: &mut EngineRng {
                    rng: &mut rng,
                    sites: &mut sites,
                },
                time: 640,
            },
        )
        .unwrap();
    assert_eq!(rng.state, before, "the end-game procs draw nothing");
    r
}

fn two_teams() -> EngineWorld {
    let mut w = world(vec![monster(0, 1, (0, 0)), monster(1, 2, (0, 0))]);
    w.monsters[1].team = 1;
    w
}

fn flag(name: &[u8; 4]) -> MapActionParameter {
    parameter(name, 0, &[])
}

#[test]
fn eliminate_team_clears_option_4_for_the_local_team() {
    let teams = || {
        let mut w = two_teams();
        w.players.teams = vec![
            crate::players::Team {
                players: 1,
                ..Default::default()
            };
            2
        ];
        w.players.players = vec![crate::players::Player {
            flags: 0,
            field_09: 0,
            team: 0,
            field_0b: 0,
            id: 0,
        }];
        w.local_player = 0;
        w
    };
    for (team, before, after, flags) in [
        (0, 0x200c_407c, 0x200c_4078, 1),
        (0, 0x200c_40fc, 0x200c_40fc, 1),
        (1, 0x200c_407c, 0x200c_407c, 1),
    ] {
        let mut w = teams();
        w.game_options = before;
        w.eliminate_team(team, None).unwrap();
        assert_eq!(w.game_options, after);
        assert_eq!(w.players.teams[team as usize].flags, flags);
    }
    let mut w = teams();
    w.game_options = 0x7c;
    w.game.flags = crate::game_state::FLAG_GAME_OVER;
    w.eliminate_team(0, None).unwrap();
    assert_eq!((w.game_options, w.players.teams[0].flags), (0x7c, 0));
    let mut w = teams();
    w.game_options = 0x7c;
    w.players.teams[0].players = 0;
    w.eliminate_team(0, None).unwrap();
    assert_eq!((w.game_options, w.players.teams[0].flags), (0x7c, 0));
}

#[test]
fn endg_winner_and_code() {
    for (params, winner, code) in [
        (vec![], 1, 3),
        (vec![flag(b"dark")], 1, 3),
        (vec![flag(b"ligh")], 0, 1),
        (vec![flag(b"ligh"), flag(b"fail")], 0, 2),
        (vec![flag(b"fail")], 1, 3),
    ] {
        let mut w = two_teams();
        assert_eq!(run(&mut w, b"endg", params), 1);
        let g = &w.game;
        assert_eq!((g.winner, g.end_code), (winner, code));
        assert_eq!((g.state, g.timer, g.flags, g.end_tick), (5, 300, 2, 640));
    }
}

#[test]
fn a_light_win_stands_and_a_dark_one_does_not() {
    let mut w = two_teams();
    run(&mut w, b"endg", vec![flag(b"ligh")]);
    run(&mut w, b"endg", vec![]);
    assert_eq!((w.game.winner, w.game.end_code), (0, 1));

    let mut w = two_teams();
    run(&mut w, b"endg", vec![]);
    run(&mut w, b"endg", vec![flag(b"ligh")]);
    assert_eq!((w.game.winner, w.game.end_code), (0, 1));
}

#[test]
fn noce_marks_the_winners() {
    let mut w = two_teams();
    run(&mut w, b"endg", vec![flag(b"ligh"), flag(b"noce")]);
    assert_eq!(w.monsters[0].flags & 0x200_0000, 0x200_0000);
    assert_eq!(w.monsters[1].flags & 0x200_0000, 0);

    let mut w = two_teams();
    run(&mut w, b"endg", vec![flag(b"ligh")]);
    assert_eq!(w.monsters[0].flags & 0x200_0000, 0);
}

#[test]
fn end2() {
    let team = |t| parameter(b"team", TYPE_INTEGER, &[t]);
    let mut w = two_teams();
    assert_eq!(run(&mut w, b"end2", vec![team(0), flag(b"noce")]), 1);
    assert_eq!((w.game.winner, w.game.end_code, w.game.state), (0, 3, 5));
    assert_eq!(w.monsters[0].flags & 0x200_0000, 0x200_0000);

    let mut w = two_teams();
    run(&mut w, b"end2", vec![team(1), flag(b"susp")]);
    assert_eq!((w.game.winner, w.game.state, w.game.flags), (1, 1, 4));
    assert_eq!(w.game.end(), None);
    run(&mut w, b"end2", vec![]);
    assert_eq!((w.game.winner, w.game.state, w.game.flags), (1, 5, 2));

    run(&mut w, b"end2", vec![team(0)]);
    assert_eq!(w.game.winner, 1);
}
