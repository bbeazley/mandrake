use mandrake_common::FourCC;
use mandrake_math::BungieRng;
use mandrake_sim::engine_rng::{
    EngineMonster, EngineObject, EngineRng, EngineWorld, MonsterDefinition, MonsterFormation,
    SiteCounts,
};
use mandrake_sim::map_actions::{MapAction, MapActionContext, MapActions, TestUnitsState};
use mandrake_tag::MapActionParameter;

fn field(line: &str, name: &str) -> i64 {
    let p = format!("{name}=");
    let v = line
        .split(' ')
        .find_map(|f| f.strip_prefix(p.as_str()))
        .unwrap_or_else(|| panic!("{name} in {line}"));
    match v.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16).unwrap(),
        None => v.parse().unwrap(),
    }
}

fn world_of(units: &[String]) -> EngineWorld {
    let mut w = EngineWorld::default();
    w.mode = 2;
    w.formation_globals.gameplay_version = 0x889;
    let last = units
        .iter()
        .map(|u| u.split(' ').nth(1).unwrap().parse::<usize>().unwrap())
        .max();
    let count = last.map_or(0, |l| l + 1);
    let bytes = vec![0u8; 0x224];
    let mut def = MonsterDefinition::from_bytes(FourCC::new(*b"test"), &bytes).unwrap();
    def.object_vitality = 0;
    w.monster_definitions = vec![def.clone(); count.max(1)];
    w.monsters = (0..count)
        .map(|i| EngineMonster {
            index: i,
            identifier: 0,
            first_draw: 0,
            definition: i,
            object: EngineObject::new(i as i32, FourCC::new(*b"test")),
            mode: 1,
            anim_state: 0xff,
            speed: 0,
            position: (0, 0),
            flags: 0,
            flags_8: 0,
            neighbours: 0,
            impassable: 0,
            ambient: None,
            unmodelled: None,
            mirrored: false,
            team: 1,
            owner: 0,
            facing: 0,
            formation: MonsterFormation::default(),
            cell_link: 0,
            flying_height: 0,
        })
        .collect();
    for u in units {
        let i: usize = u.split(' ').nth(1).unwrap().parse().unwrap();
        w.monsters[i].identifier = field(u, "id") as u16 as i16;
        w.monsters[i].object.vitality = field(u, "hp") as i32;
        w.monster_definitions[i].object_vitality = field(u, "max") as i16;
    }
    w
}

fn devi_action(threshold: i32) -> MapActions {
    let mut a = MapActions::default();
    a.actions = vec![MapAction {
        identifier: 100,
        kind: 2,
        tag: u32::from_be_bytes(*b"tuni"),
        flags: 0x100,
        delay: 0,
        delay_range: 0,
        time: 0,
        parameters: vec![
            MapActionParameter {
                kind: 2,
                count: 1,
                name: FourCC::new(*b"subj"),
                values: vec![10000],
            },
            MapActionParameter {
                kind: 8,
                count: 1,
                name: FourCC::new(*b"devi"),
                values: vec![threshold],
            },
        ],
    }];
    a.started = true;
    a
}

fn run_devi(a: &mut MapActions, vitality: i32) -> u32 {
    let units = [format!("u 0 id=10000 hp={vitality} max=100")];
    let mut w = world_of(&units);
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
    let r = a.test_units(0, &mut ctx).unwrap();
    a.actions[0].flags &= !0x140;
    r
}

#[test]
fn devi_needs_the_mean_to_move_past_the_threshold_either_way() {
    let mut a = devi_action(0x4000);
    assert_eq!(run_devi(&mut a, 50), 0);
    let s = a.test_units_state(0).unwrap();
    assert_eq!((s.most_subjects, s.vitality_at_start), (1, 0x8000));
    assert_eq!(run_devi(&mut a, 50), 0);
    assert_eq!(run_devi(&mut a, 25), 0);
    assert_eq!(run_devi(&mut a, 24), 2);
    assert_eq!(run_devi(&mut a, 75), 0);
    assert_eq!(run_devi(&mut a, 76), 2);
    assert_eq!(a.test_units_state(0).unwrap().vitality_at_start, 0x8000);
    a.actions[0].flags |= 0x100;
    assert_eq!(run_devi(&mut a, 25), 0);
    assert_eq!(run_devi(&mut a, 25), 0);
    assert_eq!(a.test_units_state(0).unwrap().vitality_at_start, 0x4000);
}

#[test]
fn devi_with_no_subjects_stays_at_0() {
    let mut a = devi_action(0);
    let mut w = world_of(&[]);
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
    assert_eq!(a.test_units(0, &mut ctx).unwrap(), 0);
    a.actions[0].flags &= !0x140;
    assert_eq!(a.test_units(0, &mut ctx).unwrap(), 0);
    assert_eq!(a.test_units_state(0).unwrap(), TestUnitsState::default());
}
