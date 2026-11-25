use mandrake_common::FourCC;
use mandrake_sim::engine_rng::{
    EngineMonster, EngineObject, EngineWorld, MonsterDefinition, MonsterFormation,
};
use mandrake_sim::spatial_groups::{
    SpatialGroup, SpatialSupergroup, build_second_level, build_spatial_groups,
    isqrt_distance2d_less_than, supergroups,
};

fn definition(tag: &[u8; 4], class: i16, strength: i16, range: i16) -> MonsterDefinition {
    let mut m = vec![0u8; 0x224];
    m[0x1b6..0x1b8].copy_from_slice(&class.to_be_bytes());
    m[0x1f2..0x1f4].copy_from_slice(&strength.to_be_bytes());
    m[0x1f4..0x1f6].copy_from_slice(&range.to_be_bytes());
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

struct Fixture {
    world: EngineWorld,
    groups: Vec<SpatialGroup>,
}

impl Fixture {
    fn new() -> Self {
        Fixture {
            world: EngineWorld::default(),
            groups: Vec::new(),
        }
    }

    fn add(&mut self, team: i16, def: usize, at: (i32, i32)) -> &mut SpatialGroup {
        let index = self.world.monsters.len();
        self.world.monsters.push(monster(index, def, team, at));
        self.groups.push(SpatialGroup {
            team,
            definition: def,
            center: at,
            radius: 0,
            facing: 0,
            strength: 10,
            range: 0,
            velocity: (0, 0),
            supergroup: None,
            seeded: None,
            members: vec![index],
        });
        self.groups.last_mut().unwrap()
    }

    fn build(mut self) -> Vec<SpatialGroup> {
        build_second_level(&self.world, &mut self.groups, 0);
        self.groups
    }
}

fn links(groups: &[SpatialGroup]) -> Vec<Option<usize>> {
    groups.iter().map(|g| g.supergroup).collect()
}

#[test]
fn only_the_same_team_and_another_definition_than_the_seed_join() {
    let mut f = Fixture::new();
    f.add(0, 1, (0, 0));
    f.add(0, 2, (100, 0));
    f.add(0, 1, (200, 0));
    f.add(1, 3, (100, 0));
    let g = f.build();
    assert_eq!(links(&g), [Some(0), Some(0), Some(1), Some(2)]);
    let s: Vec<_> = supergroups(&g).collect();
    assert_eq!(s.len(), 3);
    assert_eq!((s[0].team, s[0].members, s[0].strength), (0, 2, 20));
    assert_eq!((s[1].team, s[1].members, s[1].strength), (0, 1, 10));
    assert_eq!((s[2].team, s[2].members, s[2].strength), (1, 1, 10));
    assert_eq!(s[1].first_member, 2);
}

#[test]
fn a_chain_walks_breadth_first_from_each_queued_group() {
    let mut f = Fixture::new();
    f.add(0, 1, (0, 0));
    f.add(0, 2, (0x2ff, 0));
    f.add(0, 3, (0x2ff * 2, 0));
    let g = f.build();
    assert_eq!(links(&g), [Some(0), Some(0), Some(0)]);
    assert_eq!(supergroups(&g).count(), 1);
}

#[test]
fn only_later_groups_are_scanned() {
    let mut f = Fixture::new();
    f.add(0, 1, (0, 0));
    f.add(0, 2, (0x2ff * 2, 0));
    f.add(0, 3, (0x2ff, 0));
    let g = f.build();
    assert_eq!(links(&g), [Some(0), Some(1), Some(0)]);
    let s: Vec<_> = supergroups(&g).collect();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].members, 2);
    assert_eq!(s[1].members, 1);
}

#[test]
fn the_centres_must_be_within_the_radii_plus_0x800() {
    let build = |distance: i32| {
        let mut f = Fixture::new();
        f.add(0, 1, (0, 0)).radius = 0x100;
        f.add(0, 2, (0, 0)).radius = 0x80;
        f.groups[1].center = (distance, 0);
        links(&f.build())
    };
    let r = 0x100 + 0x80 + 0x800;
    assert_eq!(build(r - 1), [Some(0), Some(0)]);
    assert_eq!(build(r), [Some(0), Some(1)]);
}

#[test]
fn isqrt_distance2d_less_than_is_the_engine_compare() {
    assert!(isqrt_distance2d_less_than(99, 0, 100));
    assert!(!isqrt_distance2d_less_than(100, 0, 100));
    assert!(isqrt_distance2d_less_than(-99, 0, 100));
    assert!(isqrt_distance2d_less_than(0, 0, 1));
    assert!(!isqrt_distance2d_less_than(1, 0, 1));
    assert!(!isqrt_distance2d_less_than(0, 0, 0));
    assert!(!isqrt_distance2d_less_than(0, 0, -5));
    assert!(isqrt_distance2d_less_than(1 << 20, 0, 0x10001));
    assert!(isqrt_distance2d_less_than(0xffff, 0, 0x10000));
    assert!(!isqrt_distance2d_less_than(0xffff, 0x100, 0x10000));
    assert!(isqrt_distance2d_less_than(0x10000, 0, 0x10000));
}

#[test]
fn the_members_must_be_within_the_octagonal_0x2ff() {
    let build = |at: (i32, i32)| {
        let mut f = Fixture::new();
        f.add(0, 1, (0, 0));
        f.add(0, 2, at);
        f.groups[1].center = (0, 0);
        links(&f.build()) == [Some(0), Some(0)]
    };
    assert!(build((0x2ff, 0)));
    assert!(!build((0x300, 0)));
    assert!(build((0, -0x2ff)));
    assert!(!build((0, -0x300)));
    assert!(build((0x2ff, 1)));
    assert!(!build((0x2ff, 2)));
    assert!(build((-0x2fe, 3)));
    assert!(!build((-0x2fe, 4)));
    assert!(build((0x1ff, 0x1ff)));
    assert!(!build((0x200, 0x200)));
}

#[test]
fn any_pair_of_members_is_enough() {
    let mut f = Fixture::new();
    f.add(0, 1, (0, 0));
    f.add(0, 2, (10000, 0));
    let extra = f.world.monsters.len();
    f.world.monsters.push(monster(extra, 1, 0, (9600, 0)));
    f.groups[0].members.push(extra);
    f.groups[0].center = (9000, 0);
    let g = f.build();
    assert_eq!(links(&g), [Some(0), Some(0)]);
}

#[test]
fn the_queue_holds_16_groups() {
    let mut f = Fixture::new();
    for k in 0..18 {
        f.add(0, k + 1, (k as i32, 0));
    }
    f.add(0, 100, (0x2ff + 17, 0));
    f.groups[17].range = 100;
    let g = f.build();
    assert_eq!(
        links(&g),
        [vec![Some(0); 18], vec![Some(1)]].concat(),
        "the last is out of reach of the 16 searched"
    );
    let s: Vec<_> = supergroups(&g).collect();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].members, 16);
    assert_eq!(s[0].strength, 160);
    assert_eq!(s[0].center, (7, 0));
    assert_eq!(s[0].range, 8);
}

#[test]
fn the_record_weights_by_strength() {
    let mut f = Fixture::new();
    {
        let a = f.add(3, 1, (1000, 2000));
        a.strength = 30;
        a.radius = 50;
        a.range = 600;
        a.facing = 0;
        a.velocity = (10, -20);
    }
    {
        let b = f.add(3, 2, (1400, 2000));
        b.strength = 10;
        b.radius = 90;
        b.range = 100;
        b.facing = 0x4000;
        b.velocity = (-30, 40);
    }
    f.groups[0].members.push(5);
    f.world
        .monsters
        .extend((2..=5).map(|i| monster(i, 1, 3, (1000, 2000))));
    let g = f.build();
    let want = SpatialSupergroup {
        team: 3,
        first_member: 0,
        center: (1100, 2000),
        radius: 390,
        facing: mandrake_sim::engine_move::arctangent(0x4000 * 30, 0x4000 * 10),
        strength: 40,
        members: 3,
        range: 700,
        velocity: (0, -5),
    };
    assert_eq!(g[0].seeded, Some(want));
    assert_eq!(g[1].seeded, None);
}

#[test]
fn zero_strength_leaves_the_means_zero() {
    let mut f = Fixture::new();
    {
        let a = f.add(0, 1, (5000, 6000));
        a.strength = 0x4000;
        a.radius = 7;
        a.range = 7;
        a.velocity = (5, 5);
    }
    f.add(0, 2, (5000, 6000)).strength = 0x4000;
    f.add(0, 3, (5000, 6000)).strength = -0x8000;
    let g = f.build();
    assert_eq!(links(&g), [Some(0), Some(0), Some(0)]);
    assert_eq!(
        g[0].seeded,
        Some(SpatialSupergroup {
            team: 0,
            first_member: 0,
            center: (0, 0),
            radius: 0,
            facing: 0,
            strength: 0,
            members: 3,
            range: 0,
            velocity: (0, 0),
        })
    );
}

#[test]
fn the_pool_limits_the_second_level() {
    let build = |pool: i32| {
        let mut f = Fixture::new();
        f.add(0, 1, (0, 0));
        f.add(1, 1, (0, 0));
        build_second_level(&f.world, &mut f.groups, pool);
        links(&f.groups)
    };
    assert_eq!(build(0x6000 - 0x50), [Some(0), Some(1)]);
    assert_eq!(build(0x6000 - 0x28), [Some(0), None]);
    assert_eq!(build(0x6000 - 0x27), [None, None]);
}

#[test]
fn the_first_level_means_velocity_and_stops_at_its_pool_limit() {
    let mut w = EngineWorld::default();
    w.mesh_cells = (64, 64);
    w.monster_definitions = vec![definition(b"a   ", 0, 10, 2048)];
    let mut a = monster(0, 0, 0, (1000, 1000));
    a.object.velocity = [-3, 7, 0];
    let mut b = monster(1, 0, 0, (1100, 1000));
    b.object.velocity = [-4, 8, 99];
    w.monsters = vec![a, b];
    let g = build_spatial_groups(&w);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].velocity, (-3, 7), "truncated: -7 / 2, 15 / 2");
    assert_eq!(g[0].supergroup, Some(0));

    let mut w = EngineWorld::default();
    w.mesh_cells = (4096, 4096);
    w.monster_definitions = vec![definition(b"a   ", 0, 10, 2048)];
    let count = 0x5ca0 / 0x42 + 5;
    w.monsters = (0..count)
        .map(|i| monster(i, 0, 0, (i as i32 * 4096, 0)))
        .collect();
    let g = build_spatial_groups(&w);
    assert_eq!(g.len(), 0x5ca0 / 0x42 + 1);
    assert_eq!(supergroups(&g).count(), 0, "it returns at once");
}
