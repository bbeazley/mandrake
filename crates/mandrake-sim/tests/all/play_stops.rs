use crate::support::param;
use mandrake_common::FourCC;
use mandrake_math::BungieRng;
use mandrake_sim::engine_rng::{
    EngineMonster, EngineObject, EngineRng, EngineWorld, MonsterDefinition, MonsterFormation,
    SiteCounts,
};
use mandrake_sim::map_actions::{MapAction, MapActionContext, MapActions};
use mandrake_sim::map_attacks::{
    AttackConfig, AttackOrder, AttackRequest, attack_with_monster_indexes, update_attack_ai,
};
use mandrake_sim::spatial_groups::SpatialGroup;
use mandrake_tag::MapActionParameter;

fn mons(tag: &[u8; 4], flags: u32, class: i16, strength: i16, speed: i16) -> MonsterDefinition {
    let mut m = vec![0u8; 0x224];
    m[0..4].copy_from_slice(&flags.to_be_bytes());
    m[0x7e..0x80].copy_from_slice(&speed.to_be_bytes());
    m[0x1b6..0x1b8].copy_from_slice(&class.to_be_bytes());
    m[0x1f2..0x1f4].copy_from_slice(&strength.to_be_bytes());
    MonsterDefinition::from_bytes(FourCC::new(*tag), &m).unwrap()
}

fn monster(index: usize, definition: usize, team: i16, position: (i32, i32)) -> EngineMonster {
    EngineMonster {
        index,
        identifier: 10000 + index as i16,
        first_draw: 0,
        definition,
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
        team,
        owner: 0,
        facing: 0,
        formation: MonsterFormation::default(),
        cell_link: 0,
        flying_height: 0,
    }
}

fn group(team: i16, center: (i32, i32), members: Vec<usize>) -> SpatialGroup {
    SpatialGroup {
        team,
        definition: 0,
        center,
        radius: 0,
        facing: 0,
        strength: 10,
        range: 0,
        velocity: (0, 0),
        supergroup: None,
        seeded: None,
        members,
    }
}

fn attack_world() -> EngineWorld {
    let mut w = EngineWorld::default();
    w.monster_definitions = vec![mons(b"thrl", 0, 2, 10, 0)];
    w.monsters = vec![
        monster(0, 0, 1, (100, 0)),
        monster(1, 0, 1, (200, 0)),
        monster(2, 0, 0, (0, 0)),
        monster(3, 0, 0, (3000, 0)),
    ];
    w
}

#[test]
fn update_attack_ai_balances_attackers_over_the_enemy_groups() {
    let mut w = attack_world();
    w.spatial_groups = vec![group(0, (3000, 0), vec![3]), group(0, (0, 0), vec![2])];
    attack_with_monster_indexes(&mut w, &[0, 1], AttackConfig::new());
    assert_eq!(w.attack_queue.used, 0x28);
    update_attack_ai(&mut w).unwrap();
    assert_eq!(
        w.pending_attacks,
        vec![
            AttackOrder {
                source: -2,
                flags: 4,
                monsters: vec![1],
                targets: vec![3],
            },
            AttackOrder {
                source: -2,
                flags: 4,
                monsters: vec![0],
                targets: vec![2],
            }
        ]
    );
    let mut w = attack_world();
    w.spatial_groups = vec![group(0, (3000, 0), vec![3]), group(0, (0, 0), vec![2])];
    let config = AttackConfig {
        flags: 0,
        point: (0, 0),
        radius: 1000,
    };
    attack_with_monster_indexes(&mut w, &[0, 1], config);
    update_attack_ai(&mut w).unwrap();
    assert_eq!(w.pending_attacks[0].monsters, vec![1, 0]);
    assert_eq!(w.pending_attacks[0].targets, vec![2]);
    let mut w = attack_world();
    w.spatial_groups = vec![group(1, (0, 0), vec![0, 1])];
    attack_with_monster_indexes(&mut w, &[0, 1], AttackConfig::new());
    assert_eq!(update_attack_ai(&mut w), Ok(()));
    assert!(w.pending_attacks.is_empty());
    let mut w = attack_world();
    w.attack_queue.used = 0x4000 - 0x27;
    attack_with_monster_indexes(&mut w, &[0, 1], AttackConfig::new());
    assert!(w.attack_queue.requests.is_empty());
}

#[test]
fn update_attack_ai_picks_each_attackers_member() {
    let mut w = attack_world();
    w.monsters[0].position = (2900, 0);
    w.spatial_groups = vec![group(0, (1500, 0), vec![2, 3])];
    attack_with_monster_indexes(&mut w, &[0], AttackConfig::new());
    update_attack_ai(&mut w).unwrap();
    assert_eq!(w.pending_attacks[0].targets, vec![3]);
    let mut w = attack_world();
    w.monsters[0].position = (2900, 0);
    w.monster_definitions = vec![mons(b"thrl", 0, 2, 10, 30), mons(b"peas", 0, 2, 10, 0)];
    w.monsters[2].definition = 1;
    w.spatial_groups = vec![group(0, (1500, 0), vec![2, 3])];
    attack_with_monster_indexes(&mut w, &[0], AttackConfig::new());
    update_attack_ai(&mut w).unwrap();
    assert_eq!(w.pending_attacks[0].targets, vec![2]);
}

fn atta(parameters: Vec<MapActionParameter>) -> MapActions {
    let mut a = MapActions::default();
    a.actions = vec![MapAction {
        identifier: 0x8001,
        kind: 3,
        tag: u32::from_be_bytes(*b"atta"),
        flags: 4,
        delay: 0,
        delay_range: 0,
        time: 0,
        parameters,
    }];
    a
}

fn trigger_in(w: &mut EngineWorld, a: &mut MapActions) -> Result<u32, String> {
    let mut rng = BungieRng::new(7);
    let mut sites = SiteCounts::default();
    let mut e = EngineRng {
        rng: &mut rng,
        sites: &mut sites,
    };
    let mut ctx = MapActionContext {
        world: w,
        rng: &mut e,
        time: 0,
    };
    let r = a.trigger(0, &mut ctx).map_err(|e| format!("{:#x}", e.site));
    assert_eq!(sites.total(), 0, "atta makes no draws on these paths");
    r
}

#[test]
fn atta_orders_its_subjects_to_attack_its_targets() {
    let subj = param(b"subj", 2, &[10000, 10001]);
    let mut w = attack_world();
    let mut a = atta(vec![subj.clone(), param(b"obje", 2, &[10002, 10003])]);
    assert_eq!(trigger_in(&mut w, &mut a), Ok(3));
    assert_eq!(
        w.pending_attacks,
        vec![AttackOrder {
            source: 0,
            flags: 0,
            monsters: vec![0, 1],
            targets: vec![2, 3],
        }]
    );
    let mut w = attack_world();
    let mut a = atta(vec![
        subj.clone(),
        param(b"obje", 2, &[10002]),
        param(b"dore", 0, &[]),
        param(b"clos", 0, &[]),
        param(b"spec", 0, &[]),
    ]);
    trigger_in(&mut w, &mut a).unwrap();
    assert_eq!(w.pending_attacks[0].flags, 0x13);
    let mut w = attack_world();
    let mut a = atta(vec![
        subj.clone(),
        param(b"obje", 2, &[10002]),
        param(b"team", 5, &[1]),
    ]);
    assert_eq!(trigger_in(&mut w, &mut a), Ok(1));
    let mut w = attack_world();
    w.monsters[0].flags |= 0x0800_0000;
    w.monsters[1].flags |= 0x0800_0000;
    let mut a = atta(vec![subj.clone(), param(b"obje", 2, &[10002])]);
    assert_eq!(trigger_in(&mut w, &mut a), Ok(5));
    let mut w = attack_world();
    let mut a = atta(vec![subj.clone(), param(b"_all", 0, &[])]);
    assert_eq!(trigger_in(&mut w, &mut a), Ok(3));
    assert_eq!(
        w.attack_queue.requests[0],
        AttackRequest {
            config: AttackConfig::new(),
            monsters: vec![(0, 10000), (1, 10001)],
        }
    );
    let mut w = attack_world();
    let mut a = atta(vec![subj, param(b"wayp", 13, &[])]);
    assert_eq!(trigger_in(&mut w, &mut a), Ok(1));
}

#[test]
fn atta_wayp_attacks_a_drawn_point() {
    let mut w = attack_world();
    w.mesh_cells = (4, 4);
    w.cells.width = 4;
    w.cells.height = 4;
    w.cells.flags = vec![0; 16];
    w.heights = vec![(0x40, 0); 16];
    let mut a = atta(vec![
        param(b"subj", 2, &[10000, 10001]),
        param(b"wayp", 13, &[5, 7, 700, 900]),
        param(b"spec", 0, &[]),
    ]);
    let mut rng = BungieRng::new(7);
    let k = usize::from(BungieRng::new(7).next_u16() % 2);
    let mut sites = SiteCounts::default();
    let mut e = EngineRng {
        rng: &mut rng,
        sites: &mut sites,
    };
    let mut ctx = MapActionContext {
        world: &mut w,
        rng: &mut e,
        time: 0,
    };
    assert_eq!(a.trigger(0, &mut ctx).map_err(|e| e.site), Ok(3));
    assert_eq!(sites.total(), 1);
    let point = [(5, 7), (700, 900)][k];
    for m in 0..2 {
        assert_eq!(w.combat(m).location, point);
        assert_eq!(w.combat(m).location_z, 0x40);
        assert_eq!(w.combat(m).target, -1);
        assert_eq!(w.monsters[m].flags & 0x80_8037, 0x80_8037);
        assert_eq!(w.monsters[m].flags_8 & 0x800_0000, 0x800_0000);
    }
}
