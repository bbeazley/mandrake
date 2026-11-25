use mandrake_common::FourCC;
use mandrake_sim::Simulation;
use mandrake_sim::engine_rng::{
    EngineMonster, EngineObject, EngineWorld, MonsterDefinition, MonsterFormation, MovementHeader,
};

fn monster(index: usize, position: (i32, i32)) -> EngineMonster {
    EngineMonster {
        index,
        identifier: 10000 + index as i16,
        first_draw: 0,
        definition: 0,
        object: EngineObject::new(index as i32, FourCC::new(*b"test")),
        mode: 1,
        anim_state: 0x1a,
        speed: 0,
        position,
        flags: 0x8000,
        flags_8: 0x20000,
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

fn player_world(n: usize) -> EngineWorld {
    let mut w = EngineWorld::default();
    w.mesh_cells = (256, 256);
    let mut m = vec![0u8; 0x224];
    m[0x1b6..0x1b8].copy_from_slice(&2i16.to_be_bytes());
    w.monster_definitions = vec![MonsterDefinition::from_bytes(FourCC::new(*b"test"), &m).unwrap()];
    w.monsters = (0..n)
        .map(|i| monster(i, (10_000 + 1000 * i as i32, 10_000)))
        .collect();
    w.players.players = vec![mandrake_sim::players::Player {
        flags: 0,
        field_09: 0,
        team: 0,
        field_0b: 0,
        id: 0,
    }];
    w.players.teams = vec![mandrake_sim::players::Team {
        flags: 0,
        players: 1,
        player: 0,
    }];
    w
}

#[test]
fn player_commands_filter_their_monsters() {
    let mut w = player_world(6);
    w.monsters[1].flags |= 0x1000_0000;
    w.monsters[2].flags |= 0x4000_0000;
    w.monsters[3].owner = 1;
    w.monsters[3].team = 1;
    w.monsters[4].flags_8 |= 0x100;
    w.monsters[5].mode = 14;
    let all: Vec<usize> = (0..6).collect();
    let kept = w.filter_command_monsters(&all, -1, 0);
    assert_eq!(kept, [0]);
    let ordered: Vec<bool> = w
        .monsters
        .iter()
        .map(|m| m.flags & 0x800_0000 != 0)
        .collect();
    assert_eq!(ordered, [true, false, false, false, true, false]);
    let mut v = player_world(1);
    v.monsters[0].owner = 1;
    v.monsters[0].team = 1;
    assert!(v.filter_command_monsters(&[0], -1, 0).is_empty());
    let mut x = player_world(3);
    x.monsters[1].flags_8 |= 0x4000;
    assert_eq!(x.filter_command_monsters(&[0, 1, 2], 7, 0), [0, 2]);
    assert!(x.monsters.iter().all(|m| m.flags == 0x8000));
}

#[test]
fn attack_location_commands_set_the_location() {
    let mut w = player_world(2);
    for m in &mut w.monsters {
        m.flags = 0x8009002;
        m.flags_8 = 0x21;
        m.mode = 4;
    }
    w.combat_mut(0).target = 1;
    w.monsters[1].mode = 8;
    w.monsters[1].flags_8 |= 0x4_0000;
    w.attack_location_command(&[0, 1], 0, (32020, 19430, 72), 0)
        .unwrap();
    let (m, c) = (&w.monsters[0], w.combat(0));
    assert_eq!((m.flags, m.flags_8), (0x8009037, 0x21));
    assert_eq!(
        (c.location, c.location_z, c.target, c.pending),
        ((32020, 19430), 72, -1, -1)
    );
    let m1 = &w.monsters[1];
    assert_eq!((m1.flags, m1.flags_8, m1.mode), (0x8009002, 0x21, 8));
    w.attack_location_command(&[0], 0, (32020, 19430, 72), 0)
        .unwrap();
    assert_eq!(w.monsters[0].flags, 0x8009037, "the same order");
    w.attack_location_command(&[0], 3, (32020, 19430, 72), 0)
        .unwrap();
    assert_eq!(w.monsters[0].flags, 0x8809037);
    assert_eq!(w.monsters[0].flags_8, 0x221);
}

#[test]
fn set_monster_waypoints_in_mode_8_ends_the_attack() {
    let mut w = player_world(1);
    let m = &mut w.monsters[0];
    m.flags = 0x8001017;
    m.flags_8 = 0x8001;
    m.mode = 8;
    m.anim_state = 0;
    w.set_monster_waypoints_flags(0, &[(85803, 12802)], 0x88)
        .unwrap();
    let m = &w.monsters[0];
    assert_eq!((m.flags, m.flags_8, m.mode), (0x8001032, 0x48001, 8));
}

#[test]
fn invisible_monsters_are_not_updated() {
    let mut w = player_world(3);
    let mut rng = mandrake_math::BungieRng::new(1);
    let mut sites = mandrake_sim::engine_rng::SiteCounts::default();
    let mut erng = mandrake_sim::engine_rng::EngineRng {
        rng: &mut rng,
        sites: &mut sites,
    };
    w.monsters[0].flags |= 0x19;
    w.make_monster_invisible(0, &mut erng, 0).unwrap();
    w.set_monster_uncontrollable(0, true).unwrap();
    let m = &w.monsters[0];
    assert_eq!((m.flags, m.flags_8), (0x5000_8000, 0x2002_0000));
    assert!(m.object.no_shape);
    w.combat_mut(2).target = 1;
    w.monsters[2].flags |= 8;
    assert!(w.make_monster_invisible(1, &mut erng, 0).is_err());
    w.monsters[2].flags &= !8;
    w.monsters[0].mode = 3;
    w.monsters[1].mode = 3;
    w.monsters[1].flags |= 0x4000_0000;
    let mut sim = Simulation::new(1);
    w.monsters.truncate(2);
    assert!(w.update_monsters(&mut erng, None, 0, &mut sim).is_ok());
}

#[test]
fn local_commands_run_at_the_stamped_tick() {
    use mandrake_sim::{LocalGeneral, UnitEntity};
    let mut sim = Simulation::new(1);
    let mut w = mandrake_sim::engine_rng::EngineWorld::default();
    w.mode = 2;
    sim.set_engine_world(w);
    sim.command_lead = 5;
    let pos = sim.command_point_to_world(10_000, 10_000);
    sim.units.insert(
        1,
        UnitEntity::spawn_archetype(1, FourCC::new(*b"unit"), 0, pos),
    );
    sim.queue_local_general(LocalGeneral {
        unit_ids: vec![1],
        kind: 0,
    });
    sim.step_with_commands();
    assert!(matches!(
        sim.local_commands.as_slice(),
        [mandrake_sim::QueuedCommand::Packet(p)] if p.command_type == mandrake_rec::CommandType::SyncFormation
    ));
    sim.local_commands.clear();
    assert_eq!(
        sim.commands_by_tick.keys().copied().collect::<Vec<_>>(),
        [5]
    );
    for _ in 1..5 {
        assert_eq!(sim.step_with_commands().active_commands_count, 0);
    }
    assert_eq!(sim.step_with_commands().active_commands_count, 1);
    sim.commands_by_tick
        .entry(2)
        .or_default()
        .push(mandrake_rec::CommandPacket {
            tick: 2,
            player_index: 0,
            command_type: mandrake_rec::CommandType::Sync,
            payload: mandrake_rec::CommandPayload::Null,
            raw_payload: Vec::new(),
        });
    assert_eq!(sim.step_with_commands().active_commands_count, 1);
    assert_eq!(sim.late_commands, [(2, 6)]);
}

const MATCHED_MOVEMENT: u16 = mandrake_rec::MOVEMENT_FLAG_MATCHED;
const MATCHED_TARGET: u16 = mandrake_rec::TARGET_FLAG_MATCHED;

#[test]
fn a_matched_movement_keeps_its_places() {
    let mut w = player_world(4);
    w.formation_globals.gameplay_version = 0x73c;
    w.monsters[1].owner = 1;
    w.monsters[1].team = 1;
    w.monsters[3].flags_8 |= 0x4000;
    let before: Vec<_> = w.monsters.iter().map(|m| m.formation).collect();
    let header = MovementHeader {
        flags: MATCHED_MOVEMENT | 0x10,
        formation: 3,
        facing: 0x1234,
    };
    let centre = (11_000, 10_500);
    let list = [Some(2), None, Some(1), Some(0), Some(3)];
    w.execute_matched_movement_command(&list, &[(20_000, 20_000)], header, centre, -1, 0)
        .unwrap();
    let orders: Vec<_> = w
        .pending_orders
        .iter()
        .map(|o| (o.monster, o.waypoints.clone(), o.flags, o.facing))
        .collect();
    let dest = |m: i32| {
        vec![(
            10_000 + 1000 * m - 11_000 + 20_000,
            10_000 - 10_500 + 20_000,
        )]
    };
    assert_eq!(
        orders,
        [
            (2, dest(2), header.flags, 0x1234),
            (0, dest(0), header.flags, 0x1234),
        ]
    );
    let after: Vec<_> = w.monsters.iter().map(|m| m.formation).collect();
    assert_eq!(before, after);
}

#[test]
fn a_matched_movement_in_formation_counts_the_empty_places() {
    let mut w = player_world(2);
    w.formation_globals.gameplay_version = 0x73c;
    let header = MovementHeader {
        flags: MATCHED_MOVEMENT | 0x10 | 8,
        formation: 0,
        facing: 0,
    };
    let (points, centre) = (vec![(30_000, 30_000)], (10_500, 10_000));
    w.execute_matched_movement_command(
        &[None, Some(0), None, Some(1)],
        &points,
        header,
        centre,
        -1,
        0,
    )
    .unwrap();
    let got: Vec<_> = w
        .pending_orders
        .iter()
        .map(|o| (o.monster, o.waypoints[0]))
        .collect();
    let slots = mandrake_sim::formations::build_formation(
        &w.formation_definition,
        w.formation_globals,
        0,
        points[0],
        0,
        4,
    )
    .unwrap();
    assert_eq!(got, [(0, slots[1].0), (1, slots[3].0)]);
}

#[test]
fn a_matched_movement_with_flag_40_adds_no_waypoint() {
    let mut w = player_world(1);
    w.formation_globals.gameplay_version = 0x73c;
    let header = MovementHeader {
        flags: MATCHED_MOVEMENT | 0x40,
        formation: 0,
        facing: 0,
    };
    let centre = (10_000, 10_000);
    w.execute_matched_movement_command(&[Some(0)], &[(12_000, 12_000)], header, centre, -1, 0)
        .unwrap();
    assert_eq!(w.pending_orders[0].waypoints, [(12_000, 12_000)]);
    assert_eq!(
        w.pending_orders[0].facing,
        mandrake_sim::engine_move::arctangent(0, 0)
    );
}

#[test]
fn a_matched_target_pairs_by_place() {
    let mut w = player_world(6);
    w.formation_globals.gameplay_version = 0x73c;
    for m in &mut w.monsters {
        m.mode = 1;
    }
    w.monsters[4].owner = 1;
    w.monsters[4].team = 1;
    w.monsters[5].owner = 1;
    w.monsters[5].team = 1;
    let attackers = [Some(0), Some(1), None, Some(2)];
    let targets = [Some(4), None, Some(5), None];
    w.execute_matched_target_command(-1, 0, MATCHED_TARGET, &attackers, &targets)
        .unwrap();
    assert_eq!(w.combat(0).target, 4);
    assert_eq!(w.combat(2).target, 5);
    assert_eq!(w.combat(1).target, 4);
}

#[test]
fn a_matched_target_with_flag_8_sends_the_rest_to_the_closest() {
    let mut w = player_world(5);
    w.formation_globals.gameplay_version = 0x73c;
    for m in &mut w.monsters {
        m.mode = 1;
    }
    for t in [3, 4] {
        w.monsters[t].owner = 1;
        w.monsters[t].team = 1;
    }
    let attackers = [Some(0), Some(1)];
    let targets = [Some(4), None];
    w.execute_matched_target_command(-1, 0, MATCHED_TARGET | 8, &attackers, &targets)
        .unwrap();
    assert_eq!(w.combat(0).target, 4);
    assert_eq!(w.combat(1).target, 4);
    let e = w
        .execute_matched_target_command(-1, 0, MATCHED_TARGET, &attackers, &targets[..1])
        .unwrap_err();
    assert_eq!(e.site, mandrake_sim::draw_site::MATCHED_TARGETS_SHORT);
}
