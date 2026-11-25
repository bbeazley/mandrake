pub mod ally;
mod artifact_order;
pub mod assassin_action;
pub mod attack_ai;
pub mod camera_actions;
pub mod carried_objects;
pub mod collide;
pub mod combat;
pub mod connectors;
pub mod delay_action;
pub mod draw_site;
pub mod drop_player;
pub mod engine_attack;
pub mod engine_damage;
pub mod engine_move;
pub mod engine_rng;
pub mod entities;
pub mod explosions;
pub mod fire;
pub mod flags;
pub mod flight;
pub mod float_targets;
pub mod follow_action;
pub mod formations;
pub mod game_speed;
pub mod game_state;
pub mod gameplay;
pub mod geometry_filter;
pub mod harass_action;
pub mod legion_action;
pub mod lightning;
pub mod lightning_action;
pub mod look_action;
pub mod map_actions;
pub mod map_attacks;
pub mod mesh_effects;
pub mod mesh_grid;
pub mod model_animations;
pub mod model_effects;
pub mod models;
pub mod mons_postprocess;
pub mod monster_matching;
pub mod move_marker_action;
pub mod munger_action;
pub mod net_game;
pub mod netgame_time_action;
pub mod object_control_action;
pub mod particles;
pub mod passability;
pub mod pathfinding;
pub mod player_count_action;
pub mod players;
pub mod projectile_groups;
pub mod projectiles;
pub mod record_list;
pub mod rout_action;
pub mod save_restore;
pub mod setup;
pub mod shaman_action;
pub mod spatial_groups;
pub mod statistics;
pub mod suicide_action;
pub mod surround_action;
#[cfg(test)]
#[path = "../../mandrake-common/tests/support/require_data.rs"]
pub(crate) mod test_data;
mod tutorial_heal;
pub mod veterans;
pub mod views;
mod simulation {
    mod commands;
    mod construct;
    mod local_commands;
    mod movement;
    mod tick;
}

use mandrake_common::FourCC;
use mandrake_common::game_options;
use mandrake_math::{BungieRng, Fixed16, Vec3Fixed};
use mandrake_rec::{CommandPacket, CommandPayload, Recording};
use mandrake_tag::MeshTag;

pub use combat::{AttackTarget, CombatDefinitions, CombatEvent, CombatState, MonsterCombat};
pub use entities::{EngineUnitObject, UnitEntity, UnitState};
pub use flight::{FlightState, ObjectHistory, ProjectileView};
pub use mesh_grid::{MeshGrid, MeshGridCell};
pub use passability::{MeshCells, MonsterPassability};
pub use projectiles::{FlightDefinition, FlightProjectile, FlightStep, FlightTerrain};
pub use setup::{MapPopulation, MapSetup, MapSource, SetupRole};

pub type Angle = mandrake_math::trig::Angle512;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TickStateHash {
    pub tick: u32,
    pub rng_state: u32,
    pub active_commands_count: usize,
    pub state_checksum: u64,
}

pub struct Simulation {
    pub current_tick: u32,
    pub rng: BungieRng,
    pub commands_by_tick: std::collections::BTreeMap<u32, Vec<CommandPacket>>,
    pub total_dispatched_commands: usize,
    pub state_checksum: u64,
    pub mesh_grid: Option<MeshGrid>,
    pub units: entities::UnitMap,
    pub next_unit_id: u32,
    pub next_projectile_id: u32,
    pub combat: CombatState,
    pub engine: engine_rng::EngineWorld,
    pub engine_stop: Option<engine_rng::Unmodelled>,
    pub engine_sites: engine_rng::SiteCounts,
    pub engine_terrain: Option<passability::MeshCells>,
    pub engine_attack_launch: bool,
    engine_hit: Option<(i32, Option<map_actions::MapActions>)>,
    pub game_options_high: u16,
    pub carried_initializers: Vec<mandrake_rec::header::MonsterInitializer>,
    pub carried_rankings: Vec<statistics::PermanentRanking>,
    pub game_type: Option<i16>,
    pub game_setup: Option<players::GameSetup>,
    pub command_log: Option<Vec<String>>,
    logging_command: bool,
    pub replay: Option<engine_rng::ReplayGame>,
    pub local_commands: Vec<QueuedCommand>,
    pub command_lead: u32,
    pub pre_game: bool,
    time_limit: i32,
    planning_time: i32,
    pub late_commands: Vec<(u32, u32)>,
    pub local_waypoints: Vec<(i32, i32)>,
    pub attack_click: attack_ai::AttackClick,
    pub scripted_input: Option<LiveFilmInput>,
    pub halt_on_stop: bool,
    pub recorded_commands: Option<Vec<CommandPacket>>,
    pub recording_ended: bool,
    pub save_loaded_observer: Option<save_restore::SaveLoaded>,
    pub sync_snapshot: (u32, u32),
    sync_timer: i32,
    pub net: Option<net_game::NetGame>,
    pub out_of_sync: Vec<net_game::OutOfSync>,
    first_body: bool,
    collision_cache: flight::CollisionList,
    sequence_headers_cache: entities::TagIdMap<std::sync::Arc<[mandrake_tag::SequenceHeader]>>,
}

pub const SYNC_TIMER_RESET: i32 = 0xaa;
pub const SYNC_SNAPSHOT_TICKS: u32 = 0x200;
const INITIAL_SYNC_TIMER: i32 = SYNC_TIMER_RESET - 1;

fn outgoing_packet(c: QueuedCommand) -> (map_actions::SniffedCommand, CommandPacket) {
    let ids = |v: &[u32]| -> Vec<u16> { v.iter().map(|&u| u as u16).collect() };
    let mut raw_payload = Vec::new();
    let (sniffed, command_type, payload) = match c {
        QueuedCommand::Packet(c) => {
            let sniffed = match &c.payload {
                CommandPayload::Movement(m) => map_actions::SniffedCommand::Movement {
                    flags: m.flags,
                    formation: m.formation,
                },
                CommandPayload::General(g) => map_actions::SniffedCommand::General {
                    command: g.action as i16,
                },
                CommandPayload::Target(t) => map_actions::SniffedCommand::Target {
                    flags: t.flags,
                    units: t.unit_ids.clone(),
                    targets: t.target_ids.clone(),
                },
                _ => map_actions::SniffedCommand::Other(c.command_type.to_u8()),
            };
            raw_payload = c.raw_payload;
            (sniffed, c.command_type, c.payload)
        }
        QueuedCommand::Movement(c) => movement_packet(&c),
        QueuedCommand::MatchedMovement(c, centre) => {
            raw_payload = matched_movement_payload(&c, centre);
            movement_packet(&c)
        }
        QueuedCommand::General(c) => (
            map_actions::SniffedCommand::General {
                command: c.kind as i16,
            },
            mandrake_rec::CommandType::General,
            CommandPayload::General(mandrake_rec::GeneralCommand {
                action: c.kind,
                unit_ids: ids(&c.unit_ids),
            }),
        ),
        QueuedCommand::Target(c) => {
            let units = ids(&c.unit_ids);
            let targets = ids(&c.target_ids);
            (
                map_actions::SniffedCommand::Target {
                    flags: c.flags,
                    units: units.clone(),
                    targets: targets.clone(),
                },
                mandrake_rec::CommandType::Target,
                CommandPayload::Target(mandrake_rec::TargetCommand {
                    flags: c.flags,
                    unit_ids: units,
                    target_ids: targets,
                }),
            )
        }
        QueuedCommand::PickUp(c) => {
            let p = mandrake_rec::PickUpCommand {
                flags: 0,
                object_index: c.object_index,
                object_identifier: c.object_identifier,
                unit_ids: ids(&c.unit_ids),
            };
            raw_payload = pick_up_payload(&p);
            (
                map_actions::SniffedCommand::Other(6),
                mandrake_rec::CommandType::PickUp,
                CommandPayload::PickUp(p),
            )
        }
        QueuedCommand::AttackLocation(c) => (
            map_actions::SniffedCommand::Other(5),
            mandrake_rec::CommandType::AttackLocation,
            CommandPayload::AttackLocation(mandrake_rec::AttackLocationCommand {
                flags: c.flags,
                target_x: c.point.0,
                target_y: c.point.1,
                target_z: c.point.2,
                unit_ids: ids(&c.unit_ids),
            }),
        ),
    };
    (
        sniffed,
        CommandPacket {
            tick: 0,
            player_index: 0xff,
            command_type,
            payload,
            raw_payload,
        },
    )
}

fn movement_packet(
    c: &LocalMovement,
) -> (
    map_actions::SniffedCommand,
    mandrake_rec::CommandType,
    CommandPayload,
) {
    (
        map_actions::SniffedCommand::Movement {
            flags: c.header.flags,
            formation: c.header.formation,
        },
        mandrake_rec::CommandType::Movement,
        CommandPayload::Movement(mandrake_rec::MovementCommand {
            flags: c.header.flags,
            formation: c.header.formation,
            facing: c.header.facing as i16,
            unit_ids: c.unit_ids.iter().map(|&u| u as u16).collect(),
            waypoints: c
                .points
                .iter()
                .map(|&(x, y)| mandrake_rec::Waypoint {
                    x,
                    y,
                    facing_x: 0,
                    facing_y: 0,
                })
                .collect(),
        }),
    )
}

fn pick_up_payload(p: &mandrake_rec::PickUpCommand) -> Vec<u8> {
    let mut out = vec![0u8; 4];
    out[..2].copy_from_slice(&p.flags.to_be_bytes());
    out.extend_from_slice(&p.object_index.to_be_bytes());
    out.extend_from_slice(&p.object_identifier.to_be_bytes());
    out.extend_from_slice(&(p.unit_ids.len() as u16).to_be_bytes());
    for &u in &p.unit_ids {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

fn matched_movement_payload(c: &LocalMovement, centre: (i32, i32)) -> Vec<u8> {
    let mut out = vec![0u8; 8];
    out[..2].copy_from_slice(&c.header.flags.to_be_bytes());
    out[2..4].copy_from_slice(&c.header.formation.to_be_bytes());
    out[4..6].copy_from_slice(&c.header.facing.to_be_bytes());
    out.extend_from_slice(&(c.unit_ids.len() as u16).to_be_bytes());
    for &u in &c.unit_ids {
        out.extend_from_slice(&(u as u16).to_be_bytes());
    }
    out.extend_from_slice(&(c.points.len() as u16).to_be_bytes());
    for &(x, y) in &c.points {
        out.extend_from_slice(&x.to_be_bytes());
        out.extend_from_slice(&y.to_be_bytes());
    }
    out.extend_from_slice(&centre.0.to_be_bytes());
    out.extend_from_slice(&centre.1.to_be_bytes());
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMovement {
    pub unit_ids: Vec<u32>,
    pub points: Vec<(i32, i32)>,
    pub header: engine_rng::MovementHeader,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalGeneral {
    pub unit_ids: Vec<u32>,
    pub kind: u16,
}

fn engine_max_health(object: &engine_rng::EngineObject) -> Fixed16 {
    Fixed16::from_i32(i32::from(object.max_vitality as i16).max(0))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTarget {
    pub flags: u16,
    pub unit_ids: Vec<u32>,
    pub target_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAttackLocation {
    pub flags: u16,
    pub point: (i32, i32, i32),
    pub unit_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPickUp {
    pub object_index: i16,
    pub object_identifier: i16,
    pub unit_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedCommand {
    Movement(LocalMovement),
    General(LocalGeneral),
    Target(LocalTarget),
    AttackLocation(LocalAttackLocation),
    PickUp(LocalPickUp),
    Packet(CommandPacket),
    MatchedMovement(LocalMovement, (i32, i32)),
}

pub const LOCAL_COMMAND_LEAD: u32 = 1;

#[derive(Debug, Clone)]
pub struct LiveFilmInput {
    by_tick: Vec<(u32, Vec<CommandPacket>)>,
    next: usize,
    pub too_early: usize,
}

impl LiveFilmInput {
    pub fn commands_at(&self, tick: u32) -> &[CommandPacket] {
        self.by_tick[self.next.min(self.by_tick.len())..]
            .iter()
            .find(|(t, _)| *t == tick)
            .map_or(&[], |(_, c)| c.as_slice())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovementLoad {
    pub attached: usize,
    pub missing: Vec<(u32, String)>,
    pub cells_from_map_load: bool,
}

impl Simulation {
    pub const TICKS_PER_SECOND: u32 = 30;
}

impl Simulation {
    #[allow(clippy::too_many_arguments)]
    fn in_monster_stage<R>(
        &mut self,
        world: &mut engine_rng::EngineWorld,
        rng: &mut engine_rng::EngineRng,
        actions: &mut Option<map_actions::MapActions>,
        time: i32,
        (i, mv): (usize, &mut engine_move::EngineMover),
        f: impl FnOnce(&mut Simulation) -> R,
    ) -> R {
        if world.movers.len() <= i {
            world.movers.resize(i + 1, None);
        }
        world.movers[i] = Some(mv.clone());
        world.move_monster(i, (mv.x, mv.y));
        std::mem::swap(&mut self.rng, rng.rng);
        std::mem::swap(&mut self.engine_sites, rng.sites);
        std::mem::swap(
            &mut self.combat.flight.flights,
            &mut world.projectile_records,
        );
        std::mem::swap(&mut self.combat.flight.dormant, &mut world.dormant_objects);
        self.engine = std::mem::take(world);
        self.sync_solid_objects();
        self.return_movers();
        let was = std::mem::replace(&mut self.engine_attack_launch, true);
        let hit = self.engine_hit.replace((time, std::mem::take(actions)));
        let r = f(self);
        self.engine_attack_launch = was;
        if let Some((_, a)) = std::mem::replace(&mut self.engine_hit, hit) {
            *actions = a;
        }
        self.lend_movers();
        *world = std::mem::take(&mut self.engine);

        std::mem::swap(
            &mut self.combat.flight.flights,
            &mut world.projectile_records,
        );
        std::mem::swap(&mut self.combat.flight.dormant, &mut world.dormant_objects);
        world.solid_elsewhere.clear();
        std::mem::swap(&mut self.rng, rng.rng);
        std::mem::swap(&mut self.engine_sites, rng.sites);
        if let Some(m) = world.movers.get_mut(i).and_then(Option::take) {
            *mv = m;
        }
        r
    }
}

impl engine_attack::CombatHost for Simulation {
    fn new_projectile(
        &mut self,
        world: &mut engine_rng::EngineWorld,
        rng: &mut engine_rng::EngineRng,
        actions: &mut Option<map_actions::MapActions>,
        time: i32,
        launcher: (usize, &mut engine_move::EngineMover),
        desc: &projectile_groups::ProjectileDescriptor,
    ) -> Result<Option<i32>, engine_rng::Unmodelled> {
        self.in_monster_stage(world, rng, actions, time, launcher, |sim| {
            let tick = sim.current_tick;
            let mut w = projectile_groups::SimSpawn {
                sim,
                tick,
                source: None,
                reserved: None,
                source_record: None,
                engine: true,
                updating_object: None,
            };
            projectile_groups::new_projectile(&mut w, desc)
        })
    }

    fn detonate_at_creation(
        &mut self,
        world: &mut engine_rng::EngineWorld,
        rng: &mut engine_rng::EngineRng,
        actions: &mut Option<map_actions::MapActions>,
        time: i32,
        launcher: (usize, &mut engine_move::EngineMover),
        index: i32,
    ) -> Result<(), engine_rng::Unmodelled> {
        use projectile_groups::ProjectileWorld;
        self.in_monster_stage(world, rng, actions, time, launcher, |sim| {
            let tick = sim.current_tick;
            let mut w = projectile_groups::SimSpawn {
                sim,
                tick,
                source: None,
                reserved: None,
                source_record: None,
                engine: true,
                updating_object: None,
            };
            w.detonate_at_creation(index)
        })
    }

    fn monster_bumped_flag(
        &mut self,
        world: &mut engine_rng::EngineWorld,
        rng: &mut engine_rng::EngineRng,
        actions: &mut Option<map_actions::MapActions>,
        time: i32,
        launcher: (usize, &mut engine_move::EngineMover),
        object: i16,
        magnitude: i16,
    ) -> Result<(), engine_rng::Unmodelled> {
        let i = launcher.0;
        self.in_monster_stage(world, rng, actions, time, launcher, |sim| {
            sim.monster_bumped_ball(i, object, magnitude)
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn game_over_queues_a_coop_hosts_pause_0() {
        use crate::net_game::{NetGame, NetRole, PAUSE};
        let queued = |role: NetRole, options: u32, version: i32, team: i16| {
            let mut sim = Simulation::new(1);
            let mut w = engine_rng::EngineWorld::default();
            w.mode = 1;
            sim.set_engine_world(w);
            sim.net = Some(NetGame::new(role));
            sim.engine.game_options = options;
            sim.myth_game_is_over_pause(version, team);
            std::mem::take(&mut sim.local_commands)
                .into_iter()
                .map(|c| crate::outgoing_packet(c).1)
                .collect::<Vec<_>>()
        };
        let sent = queued(NetRole::Host, 0x80, 0x73c, 0);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].command_type.to_u8(), PAUSE);
        assert_eq!(&sent[0].raw_payload[..], &[0, 0, 0, 0]);
        assert!(queued(NetRole::Joiner, 0x80, 0x73c, 0).is_empty());
        assert!(queued(NetRole::Host, 0, 0x73c, 0).is_empty());
        assert!(queued(NetRole::Host, 0x80, 0x731, 0).is_empty());
        assert!(queued(NetRole::Host, 0x80, 0x73c, -1).is_empty());
        let mut solo = Simulation::new(1);
        solo.myth_game_is_over_pause(0x73c, 0);
        assert!(solo.local_commands.is_empty(), "not a network game");
    }
    use super::*;

    fn engine_with_players(version: i32, ids: &[i32]) -> Simulation {
        let mut sim = Simulation::new(1);
        let mut w = engine_rng::EngineWorld::default();
        w.mode = 2;
        w.formation_globals.gameplay_version = version;
        for &id in ids {
            w.players.players.push(players::Player {
                flags: 0,
                field_09: 0,
                team: 0,
                field_0b: 0,
                id,
            });
        }
        sim.set_engine_world(w);
        sim
    }

    #[test]
    fn the_player_byte_maps_above_1_8_4() {
        let sim = engine_with_players(0x73c, &[7, 0]);
        assert_eq!(sim.map_command_player_index(7), 0);
        assert_eq!(sim.map_command_player_index(0), 1);
        assert_eq!(sim.map_command_player_index(3), 3);
        assert_eq!(sim.map_command_player_index(0xff), 0xff);
        let sim = engine_with_players(0x726, &[7, 0]);
        assert_eq!(sim.map_command_player_index(7), 7);
    }

    #[test]
    fn a_stop_halts_a_live_game() {
        let mut sim = engine_with_players(0x73c, &[0]);
        sim.engine_stop = Some(engine_rng::Unmodelled::new(1, "test"));
        assert!(!sim.ticks_stopped());
        sim.halt_on_stop = true;
        assert!(sim.ticks_stopped());
        sim.halt_on_stop = false;
        sim.engine_stop = None;
        assert!(!sim.ticks_stopped());
        let mut bare = Simulation::new(1);
        assert_eq!(
            bare.engine_stop.as_ref().map(|s| s.site),
            Some(draw_site::NO_ENGINE_MAP)
        );
        assert!(bare.ticks_stopped());
        assert_eq!(bare.step_with_commands().tick, 0, "no tick body");
        bare.set_engine_world(engine_rng::EngineWorld::default());
        assert!(bare.engine_stop.is_none());
        assert!(!bare.ticks_stopped());
    }

    #[test]
    fn a_replay_drops_the_outgoing_queue() {
        let order = LocalGeneral {
            unit_ids: vec![10000],
            kind: 0,
        };
        let mut sim = engine_with_players(0x73c, &[0]);
        sim.queue_local_general(order.clone());
        sim.stamp_local_commands();
        assert_eq!(sim.commands_by_tick.len(), 1);
        let mut sim = engine_with_players(0x73c, &[0]);
        sim.engine.mode = 0;
        sim.queue_local_general(order);
        sim.stamp_local_commands();
        assert!(sim.commands_by_tick.is_empty());
        assert!(sim.local_commands.is_empty());
    }

    #[test]
    fn a_local_pick_up_is_stamped_as_the_film_has_it() {
        let mut sim = engine_with_players(0x73c, &[0]);
        sim.queue_local_pick_up(LocalPickUp {
            object_index: 932,
            object_identifier: 21966,
            unit_ids: vec![10012],
        });
        sim.stamp_local_commands();
        let c = &sim.commands_by_tick.values().next().unwrap()[0];
        assert_eq!(c.command_type, mandrake_rec::CommandType::PickUp);
        assert_eq!(
            c.raw_payload,
            [0, 0, 0, 0, 0x03, 0xa4, 0x55, 0xce, 0, 1, 0x27, 0x1c]
        );
        let parsed = CommandPacket::parse(c.tick, 0, 6, &c.raw_payload).payload;
        assert_eq!(parsed, c.payload);
        let mut sim = engine_with_players(0x514, &[0]);
        sim.queue_local_pick_up(LocalPickUp {
            object_index: 1,
            object_identifier: 2,
            unit_ids: (0..0x40).collect(),
        });
        let Some(QueuedCommand::PickUp(p)) = sim.local_commands.last() else {
            panic!("queued");
        };
        assert_eq!(p.unit_ids.len(), 0x1e);
    }

    fn stale_mover_sim() -> (Simulation, u32) {
        let mut w = collide::tests::world((10_000, 10_000), (20_000, 20_000));
        w.mode = 2;
        w.monsters[0].flags = 0x80a2;
        w.monsters[0].flags_8 = 0x10_0000;
        let id = w.monsters[0].identifier as u32;
        let mut u = entities::UnitEntity::new(
            id,
            FourCC::new(*b"test"),
            0,
            Vec3Fixed::ZERO,
            Fixed16::ONE,
            Fixed16::ONE,
        );
        u.engine = Some(collide::tests::mover((10_000, 10_000), (0, 0)));
        let mut sim = Simulation::new(1);
        sim.units.insert(id, u);
        sim.set_engine_world(w);
        (sim, id)
    }

    #[test]
    fn a_changed_cell_keeps_what_the_record_got_since_the_movers_update() {
        let (mut sim, _) = stale_mover_sim();
        sim.engine.impassability_updates.push(0);
        sim.apply_engine_orders();
        let w = &sim.engine;
        assert_eq!(
            (w.monsters[0].flags, w.monsters[0].flags_8),
            (0x80a2, 0x10_0000)
        );
    }

    #[test]
    fn a_map_action_order_keeps_what_the_record_got_since_the_movers_update() {
        let (mut sim, _) = stale_mover_sim();
        sim.engine.pending_orders.push(map_actions::MovementOrder {
            monster: 0,
            waypoints: vec![(10_000, 10_000)],
            flags: 0,
            facing: 0,
            action: -1,
        });
        sim.apply_engine_orders();
        let w = &sim.engine;
        assert_eq!(
            (w.monsters[0].flags, w.monsters[0].flags_8),
            (0x80a2, 0x10_0000)
        );
    }

    #[test]
    fn a_move_click_keeps_what_the_record_got_since_the_movers_update() {
        let (mut sim, id) = stale_mover_sim();
        let far = sim.command_point_to_world(12_000, 12_000);
        sim.order_move(&[id], far);
        let w = &sim.engine;
        assert_ne!(w.monsters[0].flags_8 & 0x10_0000, 0);
        assert_eq!(w.monsters[0].flags & 0xa0, 0x20, "a new leg, no detour");
    }

    #[test]
    fn rotation_command_wraps_extreme_centre_coordinates() {
        let (mut sim, id) = stale_mover_sim();
        let mut payload = Vec::new();
        payload.extend_from_slice(&0i16.to_be_bytes());
        payload.extend_from_slice(&0x4000u16.to_be_bytes());
        payload.extend_from_slice(&i32::MAX.to_be_bytes());
        payload.extend_from_slice(&i32::MIN.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&(id as u16).to_be_bytes());
        let packet = CommandPacket::parse(0, 0, 8, &payload);
        sim.commands_by_tick.entry(0).or_default().push(packet);
        sim.execute_monster_commands(0);
    }
}
