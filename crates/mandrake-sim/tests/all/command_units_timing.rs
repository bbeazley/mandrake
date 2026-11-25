use mandrake_common::FourCC;
use mandrake_math::Fixed16;
use mandrake_rec::{CommandPacket, CommandPayload, CommandType, MovementCommand, Waypoint};
use mandrake_sim::{MeshGrid, Simulation, UnitEntity};

const MDL_10002_DEST: (i32, i32) = (89_733, 14_475);

fn move_packet(tick: u32, unit: u16, x: i32, y: i32) -> CommandPacket {
    CommandPacket {
        tick,
        player_index: 0,
        command_type: CommandType::Movement,
        payload: CommandPayload::Movement(MovementCommand {
            flags: 0,
            formation: 0,
            facing: 0,
            unit_ids: vec![unit],
            waypoints: vec![Waypoint {
                x,
                y,
                facing_x: 0,
                facing_y: 0,
            }],
        }),
        raw_payload: Vec::new(),
    }
}

fn add_unit(sim: &mut Simulation, id: u32, at: (i32, i32)) {
    let pos = sim.command_point_to_world(at.0, at.1);
    let unit = UnitEntity::spawn_archetype(id, FourCC::new(*b"unit"), 0, pos);
    sim.units.insert(id, unit);
}

#[test]
fn world_units_are_512_per_cell() {
    let sim = Simulation::new(1);
    let p = sim.command_point_to_world(512, 1024);
    assert_eq!(p.x, Fixed16::from_i32(-1));
    assert_eq!(p.z, Fixed16::from_i32(2));
    assert_eq!(sim.world_to_command_point(p), (512, 1024));

    let p = sim.command_point_to_world(83_973, 12_507);
    assert_eq!(p.x.to_raw(), -83_973 * 128);
    assert_eq!(p.z.to_raw(), 12_507 * 128);

    let mut sim = Simulation::new(1);
    sim.set_mesh_grid(MeshGrid::new(256, 192));
    let p = sim.command_point_to_world(MDL_10002_DEST.0, MDL_10002_DEST.1);
    assert_eq!(sim.world_to_command_point(p), MDL_10002_DEST);
    let grid = sim.mesh_grid.as_ref().unwrap();
    assert_eq!(grid.world_to_mesh_units(p.x, p.z), MDL_10002_DEST);
    assert_eq!(grid.world_to_grid(p.x, p.z), (175, 28));
}

#[test]
fn command_applies_in_the_body_of_its_recorded_tick() {
    let mut sim = Simulation::new(7);
    sim.set_engine_world(mandrake_sim::engine_rng::EngineWorld::default());
    add_unit(&mut sim, 42, (0, 0));
    sim.commands_by_tick
        .insert(5, vec![move_packet(5, 42, 51_200, 0)]);

    for body in 0..5 {
        assert_eq!(sim.current_tick, body);
        let h = sim.step_with_commands();
        assert_eq!(h.tick, body + 1);
        assert_eq!(h.active_commands_count, 0, "body {body}");
    }
    assert_eq!(sim.current_tick, 5);
    let h = sim.step_with_commands();
    assert_eq!(h.active_commands_count, 1);
    assert_eq!(sim.total_dispatched_commands, 1);
}

#[test]
fn tick_zero_commands_run_in_the_first_body() {
    let mut sim = Simulation::new(7);
    sim.set_engine_world(mandrake_sim::engine_rng::EngineWorld::default());
    sim.commands_by_tick
        .insert(0, vec![move_packet(0, 1, 512, 512)]);
    let h = sim.step_with_commands();
    assert_eq!(h.active_commands_count, 1);
    assert_eq!(sim.current_tick, 1);
}
