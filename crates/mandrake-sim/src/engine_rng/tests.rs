use super::*;

fn seq(frames: i16, ticks: i16, key: i16, loop_frame: i16) -> SequenceHeader {
    SequenceHeader {
        number_of_views: 1,
        frames_per_view: frames,
        ticks_per_frame: ticks,
        key_frame: key,
        loop_frame,
        ..SequenceHeader::default()
    }
}

fn engine_rng<'a>(rng: &'a mut BungieRng, sites: &'a mut SiteCounts) -> EngineRng<'a> {
    EngineRng { rng, sites }
}

#[test]
fn a_backward_object_counts_down_and_wraps_to_its_last_frame() {
    let headers = [seq(4, 1, 1, 2)];
    let mut o = EngineObject::new(0, FourCC::new(*b"test"));
    o.sequence = 0;
    o.ticks_per_frame = 1;
    o.flags = 0x8_0000;
    o.frame = 2;
    let mut got = Vec::new();
    for _ in 0..5 {
        animate_object(&mut o, &headers);
        got.push((o.frame, o.anim_flags));
    }
    assert_eq!(got, [(1, 3), (0, 1), (3, 5), (2, 1), (1, 3)]);
    let headers = [seq(3, 1, 2, 0)];
    o.frame = 0;
    animate_object(&mut o, &headers);
    assert_eq!((o.frame, o.anim_flags), (2, 7));
}

#[test]
fn the_mons_postprocess_gives_every_sequence_the_idle_collision_rectangle() {
    let mut headers: Vec<SequenceHeader> = (0..6)
        .map(|i| {
            let mut h = seq(4, 2, -1, 0);
            h.world_bounds.half_width = 10 * (i + 1);
            h.world_bounds.top = 100 + i;
            h
        })
        .collect();
    let mut mons = vec![0xffu8; 0x400];
    mons[0..4].copy_from_slice(&0u32.to_be_bytes());
    mons[0x08..0x0a].copy_from_slice(&1i16.to_be_bytes());
    mons[0x0a..0x0c].copy_from_slice(&3i16.to_be_bytes());
    mons[0x88..0x8a].copy_from_slice(&1i16.to_be_bytes());
    mons[0x9a..0x9c].copy_from_slice(&4i16.to_be_bytes());
    let pp = crate::mons_postprocess::MonsPostprocess::COMMAND_LINE;
    equalize_monster_sequences(&pp.apply(&mons), &mut headers).unwrap();
    let hw: Vec<i16> = headers.iter().map(|h| h.world_bounds.half_width).collect();
    assert_eq!(hw, [10, 20, 30, 20, 20, 60]);
    assert_eq!(headers[3].world_bounds.top, 101);
    let mut headers2: Vec<SequenceHeader> = (0..6)
        .map(|i| {
            let mut h = seq(4, 2, -1, 0);
            h.world_bounds.half_width = 10 * (i + 1);
            h
        })
        .collect();
    mons[0..4].copy_from_slice(&0x0010_0000u32.to_be_bytes());
    mons[0x4e] = 2;
    mons[0x10..0x12].copy_from_slice(&5i16.to_be_bytes());
    let mons = pp.apply(&mons);
    assert_eq!(mons[0x198], 2);
    equalize_monster_sequences(&mons, &mut headers2).unwrap();
    let hw: Vec<i16> = headers2.iter().map(|h| h.world_bounds.half_width).collect();
    assert_eq!(hw, [10, 20, 30, 20, 50, 60]);
    let mut mons = mons;
    mons[0x0c..0x0e].copy_from_slice(&9i16.to_be_bytes());
    assert_eq!(
        equalize_monster_sequences(&mons, &mut headers2)
            .unwrap_err()
            .site,
        crate::draw_site::EQUALIZE_SEQUENCE
    );
}

#[test]
fn set_object_shape_draws_per_flag_and_skips_an_unchanged_sequence() {
    let headers = [seq(7, 4, -1, 0)];
    let mut obj = EngineObject::new(0, FourCC::new(*b"test"));
    let (mut rng, mut sites) = (BungieRng::new(1), SiteCounts::default());
    let mut expect = BungieRng::new(1);
    set_object_shape(
        &mut obj,
        &headers,
        0,
        0x100,
        0xf,
        &mut engine_rng(&mut rng, &mut sites),
    )
    .unwrap();
    assert_eq!(
        sites.counts().keys().copied().collect::<Vec<_>>(),
        vec![
            crate::draw_site::OBJECT_SHAPE_FRAME,
            crate::draw_site::OBJECT_SHAPE_LIMIT,
            crate::draw_site::OBJECT_SHAPE_TICK,
            crate::draw_site::OBJECT_SHAPE_VIEW
        ]
    );
    expect.next_u16();
    assert_eq!(i32::from(obj.frame), i32::from(expect.next_u16()) % 7);
    assert_eq!(i32::from(obj.tick), i32::from(expect.next_u16()) % 4);
    assert_eq!(obj.ticks_per_frame, 4);
    let before = sites.total();
    obj.sub_kind = 5;
    set_object_shape(
        &mut obj,
        &headers,
        0,
        0x100,
        3,
        &mut engine_rng(&mut rng, &mut sites),
    )
    .unwrap();
    assert_eq!(sites.total(), before);
}

#[test]
fn animate_object_advances_one_frame_per_period_and_wraps_to_the_loop_frame() {
    let headers = [seq(3, 2, 1, 0)];
    let mut obj = EngineObject::new(0, FourCC::new(*b"test"));
    obj.sequence = 0;
    obj.ticks_per_frame = 2;
    let mut seen = Vec::new();
    for _ in 0..6 {
        animate_object(&mut obj, &headers);
        seen.push((obj.frame, obj.tick, obj.anim_flags));
    }
    assert_eq!(
        seen,
        vec![
            (0, 1, 0),
            (1, 2, 3),
            (1, 1, 0),
            (2, 2, 1),
            (2, 1, 0),
            (0, 2, 5)
        ]
    );
}

#[test]
fn idle_handler_draws_only_on_a_frame_advance() {
    let headers = [seq(4, 1, -1, 0), seq(2, 1, -1, 0)];
    let def = MonsterDefinition {
        tag: FourCC::new(*b"test"),
        flags: 0,
        idle_sequence: 0,
        idle_sequence_1c: 1,
        idle_sequence_1e: 1,
        idle_sequence_20: 1,
        walk_sequence: -1,
        blocked_sequence: -1,
        turn_sequence: -1,
        sequence_22: -1,
        sequence_2a: -1,
        field_1b6: 0,
        field_199: 0,
        spatial_group_strength: 0,
        spatial_group_range: 0,
        object_vitality: 0,
        held_vitality_multiplier: 0,
        pathfinding_radius: 0,
        field_82: 0,
        field_1b5: 0,
        object_flags: 0,
        object_modifiers: [0; 9],
        object_threshold: 0,
        field_74: 0,
        field_76: 0,
        speed: 0,
        creation_draw_threshold: 0,
        object: FourCC::new(*b"test"),
        map_action: None,
        inventory: None,
        ai: Default::default(),
        name_counter: -1,
        name_count: 0,
    };
    let mut m = EngineMonster {
        first_draw: 0,
        index: 0,
        identifier: 1,
        definition: 0,
        object: EngineObject::new(0, FourCC::new(*b"test")),
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
    };
    m.object.sequence = 0;
    let (mut rng, mut sites) = (BungieRng::new(7), SiteCounts::default());
    idle_handler(
        &mut m,
        IdleSequences::of(&def),
        &headers,
        &mut engine_rng(&mut rng, &mut sites),
    )
    .unwrap();
    assert_eq!(sites.total(), 0, "no frame advance, no draw");
    m.object.anim_flags = 1;
    idle_handler(
        &mut m,
        IdleSequences::of(&def),
        &headers,
        &mut engine_rng(&mut rng, &mut sites),
    )
    .unwrap();
    let first = BungieRng::new(7).next_u16();
    if first <= 0x3fff {
        assert_eq!(m.anim_state, 0x1d);
        assert_eq!(sites.total(), 2);
    } else {
        assert_eq!(m.anim_state, 0x1a);
        assert_eq!(
            sites.at(&[
                crate::draw_site::IDLE_CHANCE,
                crate::draw_site::IDLE_SEQUENCE,
                crate::draw_site::OBJECT_SHAPE_FRAME
            ]),
            3
        );
    }
}

fn walker_world() -> EngineWorld {
    let c = FourCC::new(*b"test");
    let mut w = EngineWorld::default();
    w.collections.insert(
        c,
        vec![
            seq(4, 3, -1, 0),
            seq(8, 2, -1, 0),
            seq(6, 5, -1, 0),
            seq(5, 4, -1, 0),
        ],
    );
    w.monster_definitions.push(MonsterDefinition {
        tag: c,
        flags: 0,
        idle_sequence: 0,
        idle_sequence_1c: -1,
        idle_sequence_1e: -1,
        idle_sequence_20: -1,
        walk_sequence: 1,
        blocked_sequence: -1,
        turn_sequence: 2,
        sequence_22: -1,
        sequence_2a: 3,
        field_1b6: 0,
        field_199: 0,
        spatial_group_strength: 0,
        spatial_group_range: 0,
        object_vitality: 0,
        held_vitality_multiplier: 0,
        pathfinding_radius: 0,
        field_82: 0,
        field_1b5: 0,
        object_flags: 0,
        object_modifiers: [0; 9],
        object_threshold: 0,
        field_74: 0,
        field_76: 0,
        speed: 10,
        creation_draw_threshold: 0,
        object: c,
        map_action: None,
        inventory: None,
        ai: Default::default(),
        name_counter: -1,
        name_count: 0,
    });
    let mut object = EngineObject::new(0, c);
    object.sequence = 0;
    object.ticks_per_frame = 3;
    w.monsters.push(EngineMonster {
        first_draw: 0,
        index: 0,
        identifier: 1,
        definition: 0,
        object,
        mode: 1,
        anim_state: 0x1a,
        speed: 10,
        position: (10_000, 10_000),
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
    });
    w
}

fn distance_gait_world(k: i8, def_flags: u32) -> EngineWorld {
    let mut w = walker_world();
    let ai = &mut w.monster_definitions[0].ai;
    ai.byte_198 = 4;
    ai.flags = def_flags;
    ai.raw[0x52] = k as u8;
    w.formation_globals.gameplay_version = crate::gameplay::GAMEPLAY_1_8_5;
    w
}

#[test]
fn distance_gait_runs_by_sub_50f460() {
    let w = distance_gait_world(8, 0);
    assert!(!w.runs_at_distance(0, 8 << 9));
    assert!(w.runs_at_distance(0, (8 << 9) - 1));
    assert!(w.runs_at_distance(0, 0));
    let mut w = distance_gait_world(8, 0x8000_0000);
    assert!(!w.runs_at_distance(0, 8 << 7));
    assert!(w.runs_at_distance(0, (8 << 7) - 1));
    w.monsters[0].flags |= 8;
    assert!(w.runs_at_distance(0, (8 << 9) - 1));
    assert!(!w.runs_at_distance(0, 8 << 9));
    let mut w = distance_gait_world(8, 0x8000_0000);
    w.formation_globals.gameplay_version = 0x640;
    assert!(w.runs_at_distance(0, (8 << 9) - 1));
    w.formation_globals.gameplay_version = 0x6ad;
    assert!(!w.runs_at_distance(0, 8 << 7));
    w.monster_definitions[0].ai.flags = 0x2000_0000;
    assert!(w.runs_at_distance(0, (8 << 9) - 1));
    let w = distance_gait_world(-1, 0);
    assert!(!w.runs_at_distance(0, 0));
}

#[test]
fn distance_gait_sets_and_clears_the_run_flag() {
    let mut w = distance_gait_world(8, 0);
    assert!(w.distance_gait(0, 100));
    assert_eq!(w.monsters[0].flags_8 & 0x400, 0x400);
    assert!(w.distance_gait(0, 8 << 9));
    assert_eq!(w.monsters[0].flags_8 & 0x400, 0);
    for mode in [15, 16] {
        w.monsters[0].mode = mode;
        w.monsters[0].flags_8 |= 0x400;
        w.distance_gait(0, 8 << 9);
        assert_eq!(w.monsters[0].flags_8 & 0x400, 0x400);
    }
    let mut w = distance_gait_world(8, 0);
    w.monster_definitions[0].ai.byte_198 = 0;
    w.monsters[0].flags_8 = 0x400;
    assert!(!w.distance_gait(0, 8 << 9));
    assert_eq!(w.monsters[0].flags_8, 0x400);
}

#[test]
fn common_gait_sets_the_walking_state() {
    let run = |w: &mut EngineWorld, mover: &mut EngineMover| {
        let (mut rng, mut sites) = (BungieRng::new(5), SiteCounts::default());
        let mut r = engine_rng(&mut rng, &mut sites);
        let mut actions = None;
        let mut h = WorldHooks {
            world: w,
            actions: &mut actions,
            rng: &mut r,
            monster: 0,
            time: 0,
            host: &mut crate::engine_attack::NoHost,
        };
        h.common_gait(mover).unwrap();
    };
    let def = crate::engine_move::MovementDefinition {
        tag: FourCC::new(*b"test"),
        per_tick: false,
        speed: 10,
        turn_rate: 1000,
        terrain_speed: [255; 16],
        impassable: 0,
        flags: 0,
        slide_without_nudge: false,
        walk_half_width: 0,
        walking_sequence: 1,
        turning_sequence: 2,
        walk_ticks_per_frame: 2,
        turn_ticks_per_frame: 5,
        field_198_clear: false,
        run_sequence: 3,
        run_bonus: 0,
        field_198_bit4: true,
        field_198_bit8: false,
        pathfinding_radius: 0,
        field_82: 0,
        field_1b5: 0,
        class: 0,
    };
    for (def_flags, state) in [(0x8000_0000u32, 0u8), (0, 0x13)] {
        let mut w = distance_gait_world(8, def_flags);
        w.movers = vec![None];
        let mut mover = EngineMover::new(def.clone(), 10_000, 10_000, 0);
        w.monsters[0].mode = 4;
        w.monsters[0].anim_state = 0x13;
        w.monsters[0].flags_8 = 0x400;
        run(&mut w, &mut mover);
        assert_eq!(w.monsters[0].flags_8 & 0x400, 0);
        assert_eq!(w.monsters[0].anim_state, state);
    }
    let mut w = distance_gait_world(8, 0x8000_0000);
    w.monster_definitions[0].ai.byte_198 = 0;
    w.movers = vec![None];
    let mut mover = EngineMover::new(def.clone(), 10_000, 10_000, 0);
    w.monsters[0].flags_8 = 0x400;
    w.monsters[0].mode = 4;
    w.monsters[0].anim_state = 0;
    run(&mut w, &mut mover);
    assert_eq!(
        (w.monsters[0].anim_state, mover.animation_state),
        (0x13, 0x13)
    );
    w.monsters[0].mode = 6;
    w.monsters[0].anim_state = 0;
    run(&mut w, &mut mover);
    assert_eq!(w.monsters[0].anim_state, 0);
}

#[test]
fn paralysis_runs_its_states_and_fades() {
    let mut w = walker_world();
    w.monster_definitions[0].object_modifiers[5] = 0x180;
    w.next_object = 7;
    w.movers = vec![None];
    let mut mover = EngineMover::new(
        crate::engine_move::MovementDefinition {
            tag: FourCC::new(*b"test"),
            per_tick: false,
            speed: 10,
            turn_rate: 1000,
            terrain_speed: [255; 16],
            impassable: 0,
            flags: 0,
            slide_without_nudge: false,
            walk_half_width: 0,
            walking_sequence: 1,
            turning_sequence: 2,
            walk_ticks_per_frame: 2,
            turn_ticks_per_frame: 5,
            field_198_clear: true,
            run_sequence: -1,
            run_bonus: 0,
            field_198_bit4: false,
            field_198_bit8: false,
            pathfinding_radius: 0,
            field_82: 0,
            field_1b5: 0,
            class: 0,
        },
        10_000,
        10_000,
        0,
    );
    let (mut rng, mut sites) = (BungieRng::new(5), SiteCounts::default());
    let mut r = engine_rng(&mut rng, &mut sites);
    let mut actions = None;
    let mut h = WorldHooks {
        world: &mut w,
        actions: &mut actions,
        rng: &mut r,
        monster: 0,
        time: 0,
        host: &mut crate::engine_attack::NoHost,
    };
    mover
        .set_mode(MoveMode::Paralyzed, ModeState::Value(0x17), &mut h)
        .unwrap();
    assert_eq!(
        (h.world.monsters[0].mode, h.world.monsters[0].anim_state),
        (14, 0x17)
    );
    assert_eq!(mover.pose_timer, 0x1e);
    assert_eq!(h.world.monsters[0].object.shading_slot, 2);
    assert_eq!(h.world.combat(0).byte_8d & 0x20, 0x20);
    let fade = h.world.fade_records[0].clone();
    assert_eq!((fade.index, fade.host, fade.object.index), (0, 0, 7));
    assert_eq!((fade.object.kind, fade.object.owner), (5, 0));
    assert_eq!((fade.object.sub_kind, fade.object.sub_limit), (1, 0x1e));

    let mut expect = *h.rng.rng;
    for t in (0..=0x1e).rev() {
        assert_eq!(mover.pose_timer, t);
        h.paralysis_handler(&mut mover).unwrap();
        assert_eq!(h.world.monsters[0].anim_state, 0x17);
        mover.pose_timer -= 1;
    }
    h.paralysis_handler(&mut mover).unwrap();
    let r0 = i32::from(expect.next_u16());
    assert_eq!(h.rng.rng.state, expect.state);
    assert_eq!(h.world.monsters[0].anim_state, 0x18);
    assert_eq!(
        i32::from(mover.pose_timer),
        ((((r0 * 0xd2) >> 16) + 0x96) * 0x180) >> 8
    );

    mover.pose_timer = -1;
    h.paralysis_handler(&mut mover).unwrap();
    assert_eq!(h.world.monsters[0].anim_state, 0x19);
    assert_eq!(mover.pose_timer, 0x1e);
    assert_eq!(h.world.monsters[0].object.shading_slot, 0);
    assert_eq!(h.world.fade_records.len(), 1);
    assert_eq!(h.world.fade_records[0].index, 0);
    assert_eq!(h.world.fade_records[0].object.index, 7);

    mover.pose_timer = -1;
    h.paralysis_handler(&mut mover).unwrap();
    assert_ne!(h.world.monsters[0].mode, 14);

    for _ in 0..30 {
        h.world.update_fade_record(0);
        assert_eq!(h.world.fade_records.len(), 1);
    }
    h.world.update_fade_record(0);
    assert!(h.world.fade_records.is_empty());
    assert_eq!(h.world.new_object_datum().0, 7);
}

#[test]
fn object_identifiers_come_from_the_arrays_counter() {
    let mut w = EngineWorld::default();
    assert_eq!(w.object_counter(), 0x8000);
    assert_eq!(w.new_object_datum(), (0, i16::MIN));
    assert_eq!(w.new_object_datum(), (1, -0x7fff));
    w.free_objects.insert(0);
    assert_eq!(w.new_object_datum(), (0, -0x7ffe));
    assert_eq!(w.object_counter(), 0x8003);
    w.object_datums = 0x7fff;
    assert_eq!(w.new_object_datum().1, -1);
    assert_eq!(w.object_counter(), 0x8000);
}

#[test]
fn a_taken_identifier_becomes_the_counter() {
    let mut w = walker_world();
    w.monsters[0].identifier = 7;
    w.monsters[0].object.index = 0;
    w.monsters[0].object.identifier = 123;
    w.object_datums = 5;
    assert_eq!(w.free_object_identifier(1, 200), 200);
    assert_eq!(w.free_object_identifier(0, 123), 123);
    assert_eq!(w.free_object_identifier(1, 123), -0x7ffb);
    assert_eq!(w.object_counter(), 0x8005);
}

#[test]
fn mode_changes_draw_the_start_frame_and_tick_of_a_new_sequence() {
    let mut w = walker_world();
    let (mut rng, mut sites) = (BungieRng::new(5), SiteCounts::default());
    let mut r = engine_rng(&mut rng, &mut sites);
    let mut actions = None;
    let mut mover = EngineMover::new(
        crate::engine_move::MovementDefinition {
            tag: FourCC::new(*b"test"),
            per_tick: false,
            speed: 10,
            turn_rate: 1000,
            terrain_speed: [255; 16],
            impassable: 0,
            flags: 0,
            slide_without_nudge: false,
            walk_half_width: 0,
            walking_sequence: 1,
            turning_sequence: 2,
            walk_ticks_per_frame: 2,
            turn_ticks_per_frame: 5,
            field_198_clear: true,
            run_sequence: -1,
            run_bonus: 0,
            field_198_bit4: false,
            field_198_bit8: false,
            pathfinding_radius: 0,
            field_82: 0,
            field_1b5: 0,
            class: 0,
        },
        10_000,
        10_000,
        0,
    );
    let mut h = WorldHooks {
        world: &mut w,
        actions: &mut actions,
        rng: &mut r,
        monster: 0,
        time: 0,
        host: &mut crate::engine_attack::NoHost,
    };
    let mut expect = BungieRng::new(5);
    let frame = i32::from(expect.next_u16()) % 8;
    let tick = i32::from(expect.next_u16()) % 2;
    let clock = h
        .change_mode(&mut mover, MoveMode::Walking, ModeState::Zero)
        .unwrap();
    assert_eq!(clock, Some((tick as i8, 2)));
    let m = &h.world.monsters[0];
    assert_eq!((m.mode, m.anim_state, m.object.sequence), (4, 0, 1));
    assert_eq!(i32::from(m.object.frame), frame);
    assert_eq!(
        h.change_mode(&mut mover, MoveMode::Walking, ModeState::Keep)
            .unwrap(),
        None
    );
    assert_eq!(h.rng.sites.total(), 2);
    h.world.monsters[0].anim_state = 0x14;
    assert!(
        h.change_mode(&mut mover, MoveMode::Walking, ModeState::Keep)
            .unwrap()
            .is_some()
    );
    assert_eq!(h.world.monsters[0].object.sequence, 3);
    h.change_mode(&mut mover, MoveMode::Turning, ModeState::Zero)
        .unwrap();
    let m = &h.world.monsters[0];
    assert_eq!((m.mode, m.anim_state, m.object.sequence), (5, 0, 2));
    assert_eq!(
        h.rng.sites.at(&[
            crate::draw_site::OBJECT_SHAPE_FRAME,
            crate::draw_site::OBJECT_SHAPE_TICK
        ]),
        6
    );
    let mut expect = BungieRng::new(h.rng.rng.state);
    let speed = 0x300 + (i32::from(expect.next_u16()) >> 9);
    h.change_mode(&mut mover, MoveMode::Idle, ModeState::Zero)
        .unwrap();
    let m = &h.world.monsters[0];
    assert_eq!((m.mode, m.anim_state, m.object.sequence), (1, 0x1a, 0));
    assert_eq!(i32::from(m.object.ticks_per_frame), (3 * speed) >> 8);
    assert_eq!(h.rng.sites.at(&[crate::draw_site::CHANGE_MODE_ROLL]), 1);
    assert_eq!(h.rng.sites.total(), 9);
    h.change_mode(&mut mover, MoveMode::Idle, ModeState::Zero)
        .unwrap();
    assert_eq!(h.rng.sites.total(), 10);
    assert_eq!(h.walking_becomes(), None);
    h.world.monsters[0].flags |= 0x8000_0000;
    assert_eq!(h.walking_becomes(), Some(MoveMode::Idle));
    h.world.monsters[0].flags |= 8;
    assert_eq!(h.walking_becomes(), Some(MoveMode::Holding));
}
