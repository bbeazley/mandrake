use mandrake_common::FourCC;
use mandrake_math::BungieRng;

use super::meander_tests::{action, monster, parameter, world};
use super::*;
use crate::engine_rng::{DRAW_LOG, EngineMonster, MonsterDefinition, SiteCounts};

const SCAN: u32 = crate::draw_site::TEST_UNITS_CLOSE_BAND;
const ENEM: u32 = crate::draw_site::TEST_UNITS_ENEMY;
const ENEM_IGIN: u32 = crate::draw_site::TEST_UNITS_ENEMY_IGNORING_INVISIBLE;
const INNER: i32 = 0x100;
const OUTER: i32 = 0x300;
const BAND_D: i32 = 0x200;
fn states() -> (u32, u32) {
    let high = |state: u32| {
        let mut rng = BungieRng::new(0);
        rng.state = state;
        rng.next_u16() >> 8
    };
    let below = (1..).find(|&s| high(s) < 0x80).unwrap();
    let above = (1..).find(|&s| high(s) >= 0x80).unwrap();
    (below, above)
}

fn other(index: usize, identifier: i16, x: i32) -> EngineMonster {
    let mut m = monster(index, identifier, (x, 0));
    m.team = 1;
    m
}

fn subject(index: usize, identifier: i16, x: i32) -> EngineMonster {
    monster(index, identifier, (x, 0))
}

fn game(monsters: Vec<EngineMonster>) -> EngineWorld {
    let mut w = world(monsters);
    let mut bytes = vec![0u8; 0x224];
    bytes[0x1b7] = 5;
    let ambient = MonsterDefinition::from_bytes(FourCC::new(*b"wam2"), &bytes).unwrap();
    assert_eq!(ambient.ai.class, 5);
    w.monster_definitions.push(ambient);
    w
}

fn actions(extra: Vec<MapActionParameter>) -> MapActions {
    let mut parameters = vec![parameter(b"subj", TYPE_MONSTERS, &[1])];
    parameters.push(parameter(b"clos", TYPE_DISTANCE, &[INNER, OUTER]));
    parameters.extend(extra);
    MapActions {
        actions: vec![action(100, b"tuni", parameters)],
        ..MapActions::default()
    }
}

fn run(a: &mut MapActions, w: &mut EngineWorld, state: u32) -> (u32, Vec<u32>) {
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
            time: 0,
        };
        a.test_units(0, &mut ctx)
    };
    let draws = DRAW_LOG.with(|l| l.borrow_mut().take()).unwrap();
    (result.unwrap(), draws.iter().map(|d| d.0).collect())
}

fn inner_result(m: EngineMonster, extra: Vec<MapActionParameter>) -> (u32, Vec<u32>) {
    let mut w = game(vec![subject(0, 1, 0), m]);
    run(&mut actions(extra), &mut w, 1)
}

#[test]
fn a_monster_of_another_team_inside_the_inner_radius_counts() {
    assert_eq!(inner_result(other(1, 2, INNER), vec![]), (2, vec![]));
}

#[test]
fn the_subjects_own_team_is_skipped() {
    let mut m = other(1, 2, 0);
    m.team = 0;
    assert_eq!(inner_result(m, vec![]), (0, vec![]));
}

#[test]
fn invisible_monsters_are_skipped() {
    let mut m = other(1, 2, 0);
    m.flags = 0x4000_0000;
    assert_eq!(inner_result(m, vec![]), (0, vec![]));
}

#[test]
fn deleted_monsters_are_skipped() {
    let mut m = other(1, 2, 0);
    m.identifier = 0;
    assert_eq!(inner_result(m, vec![]), (0, vec![]));
}

#[test]
fn igin_skips_flag_4_only_with_the_parameter() {
    let mut m = other(1, 2, 0);
    m.flags_8 = 4;
    assert_eq!(inner_result(m.clone(), vec![]), (2, vec![]));
    let igin = parameter(b"igin", 0, &[]);
    assert_eq!(inner_result(m, vec![igin]), (0, vec![]));
}

#[test]
fn a_close_class_5_monster_does_not_count_and_the_scan_goes_on() {
    let mut ambient = other(1, 2, 0);
    ambient.definition = 1;
    let mut w = game(vec![subject(0, 1, 0), ambient.clone()]);
    assert_eq!(run(&mut actions(vec![]), &mut w, 1), (0, vec![]));
    let mut w = game(vec![subject(0, 1, 0), ambient, other(2, 3, 0)]);
    assert_eq!(run(&mut actions(vec![]), &mut w, 1), (2, vec![]));
}

#[test]
fn the_band_draws_at_0x4efa0a_and_passes_below_the_quotient() {
    let (below, above) = states();
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, BAND_D)]);
    assert_eq!(run(&mut actions(vec![]), &mut w, below), (2, vec![SCAN]));
    assert_eq!(run(&mut actions(vec![]), &mut w, above), (0, vec![SCAN]));
}

#[test]
fn the_band_quotient_is_taken_from_the_low_16_bits_of_the_difference() {
    let (below, _) = states();
    let mut a = actions(vec![]);
    a.actions[0].parameters[1] = parameter(b"clos", TYPE_DISTANCE, &[0, 0x9000]);
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, 0x100)]);
    assert_eq!(run(&mut a, &mut w, below), (0, vec![SCAN]));
}

#[test]
fn the_band_edges_make_no_draw() {
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, OUTER)]);
    assert_eq!(run(&mut actions(vec![]), &mut w, 1), (0, vec![]));
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, INNER)]);
    assert_eq!(run(&mut actions(vec![]), &mut w, 1), (2, vec![]));
}

#[test]
fn a_class_5_monster_in_the_band_still_draws() {
    let (below, _) = states();
    let mut ambient = other(1, 2, BAND_D);
    ambient.definition = 1;
    let mut w = game(vec![subject(0, 1, 0), ambient]);
    assert_eq!(run(&mut actions(vec![]), &mut w, below), (0, vec![SCAN]));
}

#[test]
fn the_scan_stops_at_the_first_monster_that_counts() {
    let (below, _) = states();
    let mut w = game(vec![
        subject(0, 1, 0),
        other(1, 2, BAND_D),
        other(2, 3, BAND_D),
    ]);
    assert_eq!(run(&mut actions(vec![]), &mut w, below), (2, vec![SCAN]));
}

#[test]
fn each_subject_scans_every_monster_in_turn() {
    let mut a = actions(vec![]);
    a.actions[0].parameters[0] = parameter(b"subj", TYPE_MONSTERS, &[1, 2]);
    let mut w = game(vec![
        subject(0, 1, -0x10000),
        subject(1, 2, 0),
        other(2, 3, INNER),
    ]);
    assert_eq!(run(&mut a, &mut w, 1), (2, vec![]));
    let mut w = game(vec![
        subject(0, 1, -0x10000),
        subject(1, 2, 0),
        other(2, 3, OUTER),
    ]);
    assert_eq!(run(&mut a, &mut w, 1), (0, vec![]));
}

#[test]
fn enem_listing_only_dead_monsters_does_not_scan() {
    let enem = parameter(b"enem", TYPE_MONSTERS, &[3]);
    let mut dead = other(2, 3, 0);
    dead.identifier = 0;
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, 0), dead]);
    assert_eq!(run(&mut actions(vec![enem]), &mut w, 1), (0, vec![]));
}

#[test]
fn enem_keeps_its_own_rules_and_draw_sites() {
    let (below, _) = states();
    let enem = parameter(b"enem", TYPE_MONSTERS, &[2]);
    let mut m = subject(1, 2, 0);
    m.definition = 1;
    let mut w = game(vec![subject(0, 1, 0), m]);
    assert_eq!(
        run(&mut actions(vec![enem.clone()]), &mut w, 1),
        (2, vec![])
    );
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, BAND_D)]);
    assert_eq!(
        run(&mut actions(vec![enem.clone()]), &mut w, below),
        (2, vec![ENEM])
    );
    let igin = parameter(b"igin", 0, &[]);
    assert_eq!(
        run(&mut actions(vec![enem, igin]), &mut w, below),
        (2, vec![ENEM_IGIN])
    );
}

#[test]
fn clos_replaces_the_earlier_result() {
    let count = parameter(b"___>", TYPE_INTEGER, &[5]);
    assert_eq!(inner_result(other(1, 2, 0), vec![count]), (2, vec![]));
}

#[test]
fn no_subjects_fail_without_a_draw() {
    let mut w = game(vec![other(0, 2, 0)]);
    assert_eq!(run(&mut actions(vec![]), &mut w, 1), (0, vec![]));
}

#[test]
fn one_clos_value_means_an_outer_radius_one_past_the_inner() {
    let mut a = actions(vec![]);
    a.actions[0].parameters[1] = parameter(b"clos", TYPE_DISTANCE, &[INNER]);
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, INNER + 1)]);
    assert_eq!(run(&mut a, &mut w, 1), (0, vec![]));
    let mut w = game(vec![subject(0, 1, 0), other(1, 2, INNER)]);
    assert_eq!(run(&mut a, &mut w, 1), (2, vec![]));
}
