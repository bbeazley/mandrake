use mandrake_common::FourCC;
use mandrake_math::BungieRng;
use mandrake_sim::engine_rng::{
    EngineMonster, EngineObject, EngineRng, EngineWorld, MonsterDefinition, MonsterFormation,
    SiteCounts,
};
use mandrake_sim::map_actions::{MapAction, MapActionContext, MapActions};
use mandrake_tag::MapActionParameter;

fn monster(i: usize, id: i16, team: i16, position: (i32, i32)) -> EngineMonster {
    EngineMonster {
        index: i,
        identifier: id,
        first_draw: 0,
        definition: 0,
        object: EngineObject::new(i as i32, FourCC::new(*b"test")),
        mode: 1,
        anim_state: 0xff,
        speed: 0,
        position,
        flags: 0,
        flags_8: 0,
        neighbours: 0,
        impassable: 0,
        ambient: None,
        unmodelled: None,
        mirrored: false,
        team,
        owner: 0,
        facing: 0,
        formation: MonsterFormation::default(),
        cell_link: 0,
        flying_height: 0,
    }
}

fn world_of(monsters: Vec<EngineMonster>) -> EngineWorld {
    let mut w = EngineWorld::default();
    w.mode = 2;
    w.formation_globals.gameplay_version = 0x889;
    let def = MonsterDefinition::from_bytes(FourCC::new(*b"test"), &[0u8; 0x224]).unwrap();
    w.monster_definitions = vec![def];
    w.monsters = monsters;
    w
}

fn team_action(extra: Vec<MapActionParameter>) -> MapActions {
    let mut a = MapActions::default();
    let mut parameters = vec![MapActionParameter {
        kind: 5,
        count: 1,
        name: FourCC::new(*b"team"),
        values: vec![0],
    }];
    parameters.extend(extra);
    a.actions = vec![MapAction {
        identifier: 100,
        kind: 2,
        tag: u32::from_be_bytes(*b"tuni"),
        flags: 0,
        delay: 0,
        delay_range: 0,
        time: 0,
        parameters,
    }];
    a.started = true;
    a
}

fn run_team(a: &mut MapActions, monsters: Vec<EngineMonster>) -> i16 {
    let mut w = world_of(monsters);
    let mut random = BungieRng::new(1);
    let mut sites = SiteCounts::default();
    let mut rng = EngineRng {
        rng: &mut random,
        sites: &mut sites,
    };
    let mut ctx = MapActionContext {
        world: &mut w,
        rng: &mut rng,
        time: 0,
    };
    a.test_units(0, &mut ctx).unwrap();
    a.test_units_state(0).unwrap().most_subjects
}

#[test]
fn team_takes_that_teams_live_monsters_up_to_400() {
    let mut a = team_action(Vec::new());
    let few = vec![
        monster(0, 1, 0, (0, 0)),
        monster(1, 0, 0, (0, 0)),
        monster(2, 3, 1, (0, 0)),
        monster(3, 4, 0x100, (0, 0)),
    ];
    assert_eq!(run_team(&mut a, few), 2);
    let mut a = team_action(Vec::new());
    let many = (0..450)
        .map(|m| monster(m, m as i16 + 1, 0, (0, 0)))
        .collect();
    assert_eq!(run_team(&mut a, many), 400);
}
