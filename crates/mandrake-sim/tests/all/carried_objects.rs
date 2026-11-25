use mandrake_common::FourCC;

#[test]
fn hold_counts_the_subjects_carrying_it() {
    use mandrake_math::BungieRng;
    use mandrake_sim::engine_rng::{
        EngineMonster, EngineObject, EngineProjectile, EngineRng, EngineWorld, MonsterDefinition,
        MonsterFormation, SiteCounts,
    };
    use mandrake_sim::map_actions::{MapAction, MapActionContext, MapActions};
    use mandrake_tag::MapActionParameter;

    let mut w = EngineWorld::default();
    w.mode = 2;
    w.monster_definitions =
        vec![MonsterDefinition::from_bytes(FourCC::new(*b"ghol"), &[0u8; 0x224]).unwrap()];
    w.monsters = (0..3)
        .map(|i| EngineMonster {
            index: i as usize,
            identifier: 10 + i as i16,
            first_draw: 0,
            definition: 0,
            object: EngineObject::new(i, FourCC::new(*b"test")),
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
        })
        .collect();
    let mut held = EngineObject::new(9, FourCC::new(*b"test"));
    held.kind = 4;
    w.projectiles = vec![EngineProjectile {
        marker: 0,
        identifier: 77,
        definition: FourCC::new(*b"dwsc"),
        first_draw: 0,
        object: held,
        position: (0, 0),
        charge_time: 0,
        facing: 0,
        elevation: 0,
        cell_link: 0,
    }];
    w.combat_mut(1).carried = 9;

    let tag = |s: &[u8; 4]| i32::from_be_bytes(*s);
    let param = |name: &[u8; 4], kind: i16, values: Vec<i32>| MapActionParameter {
        kind,
        count: values.len() as i16,
        name: FourCC::new(*name),
        values,
    };
    let result = |w: &mut EngineWorld, extra: Vec<MapActionParameter>| {
        let mut a = MapActions::default();
        let mut parameters = vec![param(b"subj", 2, vec![10, 11, 12])];
        parameters.extend(extra);
        a.actions = vec![MapAction {
            identifier: 0x8001,
            kind: 0,
            tag: u32::from_be_bytes(*b"tuni"),
            flags: 1 | 4,
            delay: 0,
            delay_range: 0,
            time: 0,
            parameters,
        }];
        let mut rng = BungieRng::new(1);
        let mut sites = SiteCounts::default();
        let mut ctx = MapActionContext {
            world: w,
            rng: &mut EngineRng {
                rng: &mut rng,
                sites: &mut sites,
            },
            time: 0,
        };
        a.test_units(0, &mut ctx).unwrap()
    };
    assert_eq!(
        result(&mut w, vec![param(b"hold", 7, vec![tag(b"dwsc")])]),
        2
    );
    assert_eq!(
        result(
            &mut w,
            vec![
                param(b"hold", 7, vec![tag(b"dwsc")]),
                param(b"_all", 0, vec![])
            ]
        ),
        0
    );
    assert_eq!(
        result(&mut w, vec![param(b"hold", 7, vec![tag(b"wipp")])]),
        0
    );
    assert_eq!(result(&mut w, vec![param(b"hold", 7, vec![-1])]), 2);
    assert_eq!(
        result(
            &mut w,
            vec![param(b"hold", 7, vec![-1]), param(b"_all", 0, vec![])]
        ),
        0
    );
    assert_eq!(
        result(
            &mut w,
            vec![param(b"holi", 7, vec![-1]), param(b"_all", 0, vec![])]
        ),
        2
    );
    w.combat_mut(0).inventory.push(9);
    w.combat_mut(2).inventory.push(9);
    assert_eq!(
        result(
            &mut w,
            vec![
                param(b"holi", 7, vec![tag(b"dwsc")]),
                param(b"_all", 0, vec![])
            ]
        ),
        2
    );
    assert_eq!(
        result(&mut w, vec![param(b"holi", 7, vec![tag(b"wipp")])]),
        0
    );
    assert_eq!(
        result(&mut w, vec![param(b"holi", 7, vec![-1])]),
        2,
        "1, by its empty one"
    );
    assert_eq!(
        result(
            &mut w,
            vec![param(b"holi", 7, vec![-1]), param(b"_all", 0, vec![])]
        ),
        0
    );
}
