# Simulation (mandrake-sim, mandrake-math)

This part covers the deterministic game simulation of Myth II: Soulblighter, crate `mandrake-sim`
(crate name `mandrake_sim`), and the shared integer math it builds on, crate `mandrake-math` (`mandrake_math`).
It explains what the simulation owns and what it leaves to the client, how a game is set up
from tags and a film or network header, the order of work inside a 30 Hz tick, how player orders
enter a tick, the game state, the major subsystems (monsters, movement, combat, projectiles,
explosions, fire, lightning, map actions, game end and statistics, saves, veterans, network
games), the random number generator, the determinism rules, the SYNC check, and how to verify a
change.

The simulation is a port of the original 1.8.5 engine's game logic. Its goal is bit-for-bit
agreement with the original game: the same random draws in the same order, the same integer
arithmetic, the same tick at which every order runs. A film recorded by the original game replays
here to the same end, and a network game with original 1.8.5 peers stays in sync. Everything in
this document follows from that goal.

Related parts: [data-formats.md](data-formats.md) (tags, meshes, archives), [films.md](films.md)
(the `.m2rec` format and the command encodings in `mandrake-rec`), [session.md](session.md) (campaign
flow, saves and veterans from the shell's side), [networking.md](networking.md) (the wire
protocol that carries commands), [server.md](server.md) (the dedicated server, which runs this
simulation headless), [tfl.md](tfl.md) (the separate `mandrake-tfl-sim` for The Fallen Lords; `mandrake-sim`
plays 1.8.5 only).

## Purpose and boundaries

`mandrake_sim::Simulation` is a headless lockstep simulation. It:

- reads tag bytes through a caller-supplied `mandrake_sim::engine_rng::TagSource` (a closure
  `Fn([u8; 4], FourCC) -> Option<Vec<u8>>` works), never the file system;
- takes commands (orders) as `mandrake_rec::CommandPacket` values, from a film, from the network, or
  from the local player's interface;
- advances one tick per call of `Simulation::step_with_commands` (or `Simulation::step`);
- exposes its state for reading: units, projectiles, objects, events, statistics, the game end.

It does no rendering, plays no sound, opens no sockets and reads no clock. Presentation that the
original engine computes with its non-synchronised random generator (particles, sounds, blood
stains, local projectile groups, camera shake) is the client's: the simulation emits events and
requests (`combat::CombatEvent`, sequence sounds, game-sound requests, camera actions) and the
client turns them into pictures and sound.

Dependencies: `mandrake-common` (the `FourCC` tag-id type, `M2Error`, Mac Roman text), `mandrake-math`,
`mandrake-tag` (tag parsers: meshes, markers, sequences, model hit tests), `mandrake-rec` (command packets,
film headers, the film writer). Nothing in `mandrake-sim` depends on the client, the renderer or the
network crate.

Runs on x86_64, aarch64 and wasm32 with the same results (see "Floating point" below).

## mandrake-math: the shared integer math

`mandrake_math` is `#![no_std]` and platform-agnostic.

| Item | What it is |
|---|---|
| `mandrake_math::Fixed16` | Signed Q16.16 fixed point in an `i32` (`Fixed16::ONE` is `1 << 16`). `*` is `(a as i64 * b as i64) >> 16` truncated to `i32`; there are checked and saturating variants (`Fixed16::checked_mul`, `Fixed16::saturating_mul`, ...). `from_f32`/`to_f32` exist for set-up and display only. |
| `mandrake_math::Vec3Fixed` | Three `Fixed16`s with the usual vector operations (dot, cross, length via an integer square root). |
| `mandrake_math::Angle512` | A binary angle, 512 units per turn (`Angle512::DEG_90` is 128). |
| `mandrake_math::sin`, `mandrake_math::cos`, `mandrake_math::atan2` | Table lookups on `Angle512`: a 512-entry sine table and a 64-entry octant threshold table for `atan2`. No floating point. |
| `mandrake_math::BungieRng` | The original game's linear congruential generator: `state = state * 1664525 + 1013904223` (wrapping `u32`). `BungieRng::step` advances and returns the state, `BungieRng::next_u16` returns the high 16 bits of the new state (the value the original engine's random function returns), `BungieRng::next_range(min, max)` is `min + (next_u16 * (max - min)) >> 16` and makes no draw when `min >= max`. `BungieRng::DEFAULT_SEED` is `0xfded`. |

`mandrake_sim::Angle` is an alias for `Angle512`, used by the simplified unit layer
(`entities::UnitEntity::facing`). The ported engine code does not use `Angle512`: it uses the
original engine's 16-bit angles (`0x10000` per turn) and its 1024-entry trigonometry tables with
magnitude `1 << 14`, built by `mandrake_sim::engine_move::trig_tables` (see "Floating point").

## Module map

| Module (`mandrake_sim::...`) | What it holds |
|---|---|
| `lib.rs` (crate root) | `Simulation`, the tick body, the command stage and every command handler, the local player's command queue, SYNC queueing |
| `setup` | `Simulation::set_up_map` and its two halves, `SetupRole`, `MapSource`, `MapSetup`; the start constructors `Simulation::replay_film`, `Simulation::start_network_game` |
| `engine_rng` | `EngineRng` (counted draws), `EngineWorld` (the ported engine's world state), `EngineMonster`, `EngineObject`, the map load (`map_load_with_options`), `GameParameters`, `ReplayGame`, `Unmodelled`, `update_monsters` and most per-monster logic |
| `game_state` | `GameState`: game state word, timer, flags, `end_game`, winner, `GameEnd` |
| `players` | `GameSetup`, players and teams at map load, `SoloGame`, `set_random_seed`, game option constants |
| `gameplay` | The gameplay version (`GAMEPLAY_1_8_3`, `GAMEPLAY_1_8_5`), `compute_gameplay_version`, `replay_clamp` |
| `entities` | `UnitEntity` (the simplified per-unit view the client draws), `UnitState`, deterministic hashers for maps keyed by unit id or tag id |
| `engine_move` | The monster movement model `EngineMover`, movement definitions, modes, the 1024-entry trig tables |
| `passability` | Mesh cell terrain types and flags (`MeshCells`), the impassable masks, `move_monster` |
| `pathfinding` | Path planning: `new_path`, the best-first `search`, `adjust_path_destination_for_obstacles` |
| `collide` | Object collisions during a monster's step and the walker's reactions |
| `formations` | Formation definitions and slot building for group moves |
| `spatial_groups` | Spatial grouping of monsters that map actions and the attack AI read |
| `monster_matching` | Pairing of the local player's monsters with destinations or targets before a command is sent (live games only; a film holds the result) |
| `attack_ai` | Monster attack AI: attack definitions, target choice, TARGET command execution |
| `engine_attack` | Attack choice, aim and release inside the monster loop; the `CombatHost` trait through which an attack launches a projectile mid-update |
| `engine_damage` | Damage amount, damage to objects, hit reactions, kills, dropped ammunition |
| `combat` | `CombatState`, `CombatDefinitions`, `CombatEvent` (events for the client), damage rolls, the version-dependent probability test |
| `flight` | Projectiles in flight in the `Simulation`: the projectile stage, `FlightState`, object history for interpolation, `ProjectileView` for the renderer |
| `projectiles` | The flight model: ballistic launch, `FlightTerrain`, collision with objects, terrain and media |
| `projectile_groups` | Projectile groups and projectile creation |
| `explosions` | Delayed area damage records and the explosion stage |
| `lightning` | Lightning records (beams from projectiles with a lightning definition) |
| `fire` | Burning cells, their spread, and the mirror groups (flame and smoke records) |
| `particles` | The weather particle system's intensity clock (the particles themselves are the client's) |
| `mesh_effects` | Travelling waves that move mesh vertex heights |
| `model_effects`, `model_animations`, `models` | Model effects started by map actions; flip-book model animations (gates, drawbridges, windmills); the mesh cells models block |
| `connectors` | Walls between scenery posts, taken down when a post is gone |
| `carried_objects` | Giving a monster an object, pick-ups, inventories, artifacts |
| `artifact_order` | The order of artifact definitions as the map load reaches them |
| `flags` | Flags of the multiplayer game types (capture the flag and similar) |
| `ally` | The 16x16 alliance matrix and its changes |
| `map_actions` | The map-action script interpreter (`MapActions`, `MapAction`) and most action procs |
| `map_attacks` | The map actions' attack AI and the TARGET commands it issues |
| `*_action` modules | One map-action proc each: `assassin_action` (`assa`), `delay_action` (`dela`), `follow_action` (`foll`), `harass_action` (`girl`), `legion_action` (`legi`), `lightning_action` (`ligh`), `look_action` (`look`), `move_marker_action` (`moma`), `munger_action` (`mung`), `netgame_time_action` (`plan`), `object_control_action` (`obco`), `player_count_action` (`plyc`), `rout_action` (`rout`), `shaman_action` (`sham`), `suicide_action` (`suic`), `surround_action` (`surr`) |
| `camera_actions` | The `obmo` map action's camera writes, recorded for the client |
| `geometry_filter` | The `geom` map action, which selects monsters, markers or objects into other actions' parameters |
| `mons_postprocess` | The field rewrite applied to every loaded `mons` definition |
| `statistics` | The statistics block, rankings, time series, and what the postgame carnage report shows |
| `veterans` | The monster initializer table that carries veterans between levels |
| `save_restore` | The commands a saved game's restore queues after its film has replayed |
| `drop_player` | The DROP_PLAYER command |
| `net_game` | Mode 1, the network game: horizon, system commands, pause, game speed, out-of-sync reports |
| `game_speed` | `GameSpeed`: how many ticks a frame runs at each speed; the network tick gate `NetGate` |
| `record_list` | A list of projectile-data records with an index set |
| `mesh_grid` | Mesh heights in the client's coordinate space and the conversion to world units |
| `views` | Read-only views for the client: object shapes, projectile objects, move-marker queries |
| `float_targets` | Hashes of every C-library float result the simulation uses, for cross-target pins |

## Units of measure and coordinates

- World units: 512 per mesh cell (`mandrake_rec::WORLD_UNITS_PER_CELL`); x is the column, y the row, z
  the height. Engine positions, waypoints in commands and projectile positions are world units.
- Angles in the ported engine code: 16-bit, `0x10000` per turn, forward is `(cos f, sin f)`.
- `UnitEntity::position` is a `Vec3Fixed` in the client's space (`Simulation::command_point_to_world`
  and `Simulation::world_to_command_point` convert); its `y` is height.
- A tick is 1/30 s (`Simulation::TICKS_PER_SECOND`).

## Two layers of state

The simulation holds two views of the world.

1. **The engine model**, `Simulation::engine: Option<engine_rng::EngineWorld>`. This is the port
   of the original engine's state: monsters (`EngineWorld::monsters`, up to
   `engine_rng::MONSTER_CAPACITY` = 400 records, in array order), scenery, objects, projectile
   records, mesh cells, map actions, players and teams, alliances, fire, wind, particles,
   statistics, the veterans table, the game state, the gameplay version
   (`EngineWorld::formation_globals`), the mode (`EngineWorld::mode`: 0 replay, 1 network game,
   2 solo game) and the local player. Every random draw that matters happens here.
2. **The unit layer**, `Simulation::units` (`entities::UnitMap`, keyed by the monster's 16-bit
   identifier, `engine_rng::unit_key`). One `entities::UnitEntity` per monster with position,
   facing, health, team, selection flag and the monster's movement model
   (`UnitEntity::engine: Option<engine_move::EngineMover>`). The client draws from this layer.

During the monster stage the movers are lent to the engine model and returned after it
(`lend_movers` / `return_movers_to` in `lib.rs`), and positions are published back to the units
(`publish_movement`); `sync_unit_vitality` copies each living monster's vitality into the unit's
health at the end of every tick body. Units without a movement model fall back to simple
straight-line locomotion (`UnitEntity::tick_step`), which only matters when no engine model is
loaded (a bare mesh without the map load).

The combat queue in `combat::CombatState` (`order_attack`, `release_attack`,
`impact_projectile`) is a simplified combat path for units without the engine model. With the
engine model loaded, attacks, projectiles and damage all run through the engine code.

### The engine stop

The port is complete for the 1.8.5 films and games it has been checked on, but some branches of
the original engine are not ported. When the model reaches one, it does not guess: it returns an
`engine_rng::Unmodelled { site, reason }` and the simulation records it in
`Simulation::engine_stop`. `site` is an opaque `u32` naming the place in the original engine,
always one of the named constants of `mandrake_sim::draw_site` (e.g. `draw_site::ASSASSIN_DIVISION_BY_ZERO`);
`Unmodelled`'s `Display` prints it in hex with the reason.

Once `engine_stop` is set, the engine model makes no more draws (any later draw would be at the
wrong position in the random sequence) and its stages are skipped:

- A film keeps running ticks without the model, so the rest can still be watched; the units then
  move without the engine's rules.
- A live game (`Simulation::halt_on_stop`, set by `SetupRole::Game { live: true }` and by
  `Simulation::start_network_game`) runs no more ticks: `Simulation::ticks_stopped` is true.
- A map load that stopped leaves no game (`engine` is `None` with `engine_stop` set).

The client shows the stop on screen (its `stop_text`), and replay tools report it with its tick.
A new stop is a bug to fix by porting the missing branch, never by approximating it.

## Setting up a game

### Constructors

| Constructor | Game |
|---|---|
| `Simulation::from_recording(&Recording)` / `Simulation::from_film(&[u8])` | A film replay (mode 0). Seeds the generator with `players::set_random_seed(header.seed)` (seed 0 becomes `0xfded`), queues all the film's commands in `commands_by_tick`, takes options, game type, time limit, planning time and the film's veterans (`header.monster_initializers`) from the header, and builds an `engine_rng::ReplayGame` from the header's game mode and options. `from_film` also reads the complete player entries (`players::GameSetup::from_film`). |
| `Simulation::solo_game(players::SoloGame)` | A solo mission (mode 2): seed, difficulty, options; no film. |
| `Simulation::command_line_game(seed)` | The game the original engine starts for a mesh named on the command line (mode 2, fixed options `players::COMMAND_LINE_GAME_OPTIONS`). |
| `Simulation::live_game_of_film(&[u8])` | A film's game played live: the header's mode is kept (no replay), and the film's commands are returned as a `LiveFilmInput` to feed as local input with `Simulation::give_film_commands`. Used to test that live input lands on the same ticks as in the film. |
| `Simulation::network_game(&[u8], net_game::NetRole)` | A network game (mode 1) from the new-game header that `mandrake-net` builds in film form; the header's mode must be 1. |

A `players::GameSetup` describes the players and teams (from a film header, a game-parameter
block, a solo game or the command-line game). `engine_rng::GameParameters` carries the mode, the
difficulty and the gameplay version; `GameParameters::from_film`, `GameParameters::solo` and
`GameParameters::COMMAND_LINE` build it.

### `Simulation::set_up_map`

All consumers set a map up through `setup::Simulation::set_up_map` (or its halves `load_map` and
`populate_map`), because the order of the set-up calls decides the generator's state. The
`setup::SetupRole` names which order runs:

- `SetupRole::Headless`: a film replayed with no client (the replay tools and tests): the engine
  map load, then, if it succeeded, the mesh, one unit per mesh unit marker, and their movement.
- `SetupRole::Game { live }`: a game with a client or server (client films, solo and network
  games, the dedicated server): `command_lead = LOCAL_COMMAND_LEAD`, the mesh, the engine map
  load, then one unit per monster the load made (with the scale, vitality and colour permutation
  the load drew), and their movement. `live: true` also sets `halt_on_stop`.

The input is a `setup::MapSource { mesh_data, mesh, tags, params }`: the mesh tag's bytes and its
parse (`mandrake_tag::MeshTag`), the tag source (later archives override earlier ones, in the original
engine's archive order, see [data-formats.md](data-formats.md)), and the `GameParameters`. The
result is a `setup::MapSetup` with the load's draws or its `Unmodelled` stop, the monsters, and
the `MapPopulation`. After set-up, `Simulation::load_combat_definitions` loads the combat
definitions for the units' archetypes.

The start helpers on top: `Simulation::replay_film(recording, setup, map)` (headless replay) and
`Simulation::start_network_game(film, role, carried_veterans, map, archives)` (the server's game).

### The map load

`Simulation::load_engine_map` runs `engine_rng::map_load_with_options` from the current generator
state (the film's or game's seed). It makes the original engine's map-load draws in its order:

1. The mode (0 for a replay) and the gameplay version from the parameters; the mesh's media
   definition; the mesh's map actions are imported.
2. Every `mons` definition the load reads is the postprocessed one (`mons_postprocess`), which
   depends on the mode, the game options and the gameplay version.
3. Players and teams (`players::initialize_players`), with teams from the mesh's unit palette.
4. Fire and wind initialisation (draws), then the particle system (draws).
5. The veterans table and the permanent rankings are reset or kept (game option
   `players::VETERANS_ALLOWED`, `0x40`).
6. The markers, by marker type 0..11, for each type the palette entries then the markers in array
   order. Scenery (type 1), monsters (type 3) and model animations (type 11) draw (permutations,
   scale, vitality, speed, start frames); sound sources (5), models (6) and local projectile groups
   (10) make no synchronised draws; type 9 creates projectiles from markers. Models change the mesh
   cells' passability.
7. Connectors (no draws), the player statistics, then the first run of the map actions at tick
   counter 0, then the formation definitions.

A typical map load makes thousands of draws before tick 0. Missing one shifts every later draw.

`Simulation::attach_engine_movement` then gives each unit its `EngineMover` (movement definition
from the `mons` and its collection's walking sequences, the speed the load drew) and prepares the
projectile terrain (`projectiles::FlightTerrain` with media, models, cell flags, heights) and the
scenery collision cylinders. Orders the first map-action run issued are applied.

## The tick

`Simulation::current_tick` is the original engine's tick counter: `step_with_commands` runs the
tick body for tick `current_tick` and then increments it. The returned `TickStateHash` describes
the state at the top of the next body.

A body is split in two public halves so that callers can act between the command stage and the
rest: `Simulation::run_command_stage` and `Simulation::finish_tick_body`. `step_with_commands` is
both, and returns early (no body, the counter unchanged) when `ticks_stopped()` is true or, in a
network game, when `net_body_may_run()` says the game is paused or stopped.

Order of work in one body (this order is the original engine's and must not change; every stage
can draw, so moving one reorders the random sequence):

1. Network game only: the system commands stamped up to this tick run (pause, game speed, forced
   end), and a body at or past the horizon reports out of sync once.
2. `record_object_history`: copies of every object in use, which the client interpolates from
   (`flight::ObjectHistory`).
3. While a replay runs, the gameplay version is clamped (`gameplay::replay_clamp`, at most
   `GAMEPLAY_1_8_5`).
4. Scripted input (`Simulation::scripted_input`), then `stamp_local_commands`: the local player's
   queued commands get their tick and player (see "Command input").
5. The SYNC snapshot: at a nonzero multiple of `SYNC_SNAPSHOT_TICKS` (512),
   `sync_snapshot = (rng.state, tick)`, before any stage of the tick draws.
6. `execute_monster_commands(tick)`: every queued command with a tick up to this one.
7. The first body of a live game (mode 1 or 2) queues the local player's formation definition
   (a SYNC_FORMATION command, stamped with the next batch).
8. `update_lightning`: lightning heads run along their beams, with damage and stains.
9. `update_projectile_flight`: projectiles in flight move, collide and detonate.
10. `update_unit_movement`: movers the engine model does not drive this tick.
11. The simplified combat queue (`update_combat_monsters`, `update_combat_projectiles`).
12. `update_engine_monsters`: `EngineWorld::update_monsters` (every monster in array order:
    re-planning, animation, mode handlers, movement, attacks, damage).
13. `update_engine_mesh_effects`.
14. `update_engine_map_actions`: the map actions run in bodies whose tick is a multiple of 32.
15. `update_explosions`.
16. `update_engine_fire_and_wind`: the particle clock, model effects, fire and mirror groups,
    connectors (when one is changing), wind.
17. `update_engine_model_animations`.
18. Flag positions, then `update_engine_player_statistics` (statistics, rankings, and the game
    actions the statistics take: eliminating a team, ending the game, the winner).
19. `sync_unit_vitality`.
20. `current_tick += 1`; `end_of_tick_body`: the game timer and state dispatch
    (`game_state::GameState::end_of_tick_body`), the veterans table at game over;
    `count_down_sync` (solo game SYNCs).
21. The port's rolling `state_checksum` (see below).

How many bodies run per frame is the client's decision (`game_speed::GameSpeed::advance` at
normal speed runs one tick per 30 Hz poll and at most 4 per frame, scaled above normal speed); it
changes nothing in the game's state. A network game's ticks are gated by the horizon
(`GameSpeed::advance_network`, `game_speed::NetGate`).

## Command input

All orders are `mandrake_rec::CommandPacket { tick, player_index, command_type, payload, raw_payload }`
values (encodings in [films.md](films.md)). They enter `Simulation::commands_by_tick`, keyed by the
tick they are stamped with, and nothing else changes the game.

### From a film

`from_recording` queues every command of the film at its recorded tick before tick 0. In a
replay, commands the local interface queues are discarded (`stamp_local_commands` clears them),
so the film's commands are all that run. A film's PAUSE and GAME_SPEED commands are skipped by the
command stage (outside a network game they do nothing).

### From the local player (solo game)

The client calls the queue functions with monster identifiers and world-unit points:
`Simulation::queue_local_movement` (`LocalMovement`, built by `Simulation::local_move_click` and
`Simulation::local_movement_header`), `queue_local_general` (`LocalGeneral`: Stop, Hold, Scatter,
Retreat), `queue_local_target` (`LocalTarget`), `queue_local_attack_location`
(`LocalAttackLocation`), `queue_local_pick_up` (`LocalPickUp`), `queue_local_ally`, and the
network helpers `queue_unit_adjustment` and `queue_detach`. They go on `Simulation::local_commands`
(`QueuedCommand`), unstamped.

At the start of the next body, `stamp_local_commands` gives each one tick
`current_tick + command_lead` and the local player's index, passes it through the tutorial's
command sniffer (`sniff_local_command`: the training map's scripts can refuse an order), records it
when a film is being recorded (`Simulation::recorded_commands`), and queues it. With
`command_lead = LOCAL_COMMAND_LEAD` (1), a command given while the counter is `T` runs in body
`T + 1`, as in the original game. The constructors start at lead 0; `SetupRole::Game` sets 1.

Paired MOVEMENT commands (`QueuedCommand::MatchedMovement`, flag `mandrake_rec::MOVEMENT_FLAG_MATCHED`)
carry the formation centre after the points; the pairing is computed by `monster_matching` before
the command is queued.

### From the network (mode 1)

The local queue is not stamped locally. The client takes it with
`Simulation::take_outgoing_commands` (player `0xff`, tick 0) and sends it; every stamped command
the game server sends back, the peer's own included, goes in through
`Simulation::queue_incoming_command`. NULL (a heartbeat), the forced end, PAUSE and GAME_SPEED go to
the system queue (`net_game::is_system_command`), the others to `commands_by_tick`. The highest
stamped tick is the horizon (`Simulation::net_horizon`); the client's tick gate runs no body at or
past it. See [networking.md](networking.md).

### The command stage

`execute_monster_commands(t)` takes every queued tick up to `t` in ascending order and, within a
tick, in queue order:

1. A command whose tick is earlier than `t` still runs, but is a late command: it is recorded in
   `Simulation::late_commands` and reported out of sync (in a film this means the replay's
   ticking has drifted from the recording).
2. The player byte is mapped from a network id to a player index (above gameplay `0x731`); `0xff`
   passes; a byte past the player count drops the command.
3. In the pre-game state (game state 0, the unit-trading period of a timed game) orders are
   dropped.
4. `execute_monster_command` dispatches on the type:

| Type | Effect |
|---|---|
| 1 SYNC | The seed check (below). |
| 2 GENERAL | Stop, hold, scatter, retreat and similar (`execute_general`). |
| 3 MOVEMENT | Waypoints and formations for the listed monsters (`execute_movement`). |
| 4 TARGET | Attack (or, with header flag 2, a friendly target such as a heal) through `EngineWorld::execute_target_command`. |
| 5 ATTACK_LOCATION | Attack a point (`EngineWorld::attack_location_command`). |
| 6 PICK_UP | `EngineWorld::execute_pick_up_command`. |
| 7 RENAME, 11 DETACH, 12 UNIT_ADJUSTMENT | Name filter flags, giving units to a teammate, the pre-game unit trade. |
| 9 DROP_PLAYER | `EngineWorld::drop_player` (`drop_player` module): units go to a teammate, or the team is eliminated. |
| 10 CHAT, 13 SKETCH, 15 PRESET_SELECTION | Nothing in the game: the client's. |
| 16 SYNC_FORMATION | The player's formation definition. |
| ALLY | Toggle an alliance (game option `0x2000` only). |
| 22 ADD_PLAYER, 25 SAVE_LOADED, 26 REPLACE_PLAYER, 27 SET_TEAM_CAPTAIN | Players joining or replaced, and the end of a saved game's restore (`save_restore`). |

Types 17..20 (inventory commands and two others) are not ported and stop the engine model;
types 21, 23, 24 and above 27 are skipped here (they are system commands or unused). Attack orders
are also emitted as `CombatEvent::AttackOrdered` for the client.

## Game state and the end of the game

`game_state::GameState` (in `EngineWorld::game`) holds:

- `state`: 0 pre-game (unit trading), 1 untimed, 2 timed, 3 and 4 an end asked for, 5 over;
- `timer`, decremented by every tick body; at 0 the state dispatches (`end_of_tick_body`): state 0
  becomes 2 (the game time starts), 2 or 3 calls `end_game`, 5 marks the game over;
- `flags`: `FLAG_END_REQUESTED`, `FLAG_GAME_OVER`, `FLAG_END_SUSPENDED`, `FLAG_OUT_OF_SYNC`,
  `FLAG_PAUSED`, `FLAG_SAVE_LOADED`;
- `winner` (0 Light, 1 Dark, -1 none), `end_code` (1 victory, 2 a lightning-condition failure,
  3 otherwise), `end_tick`;
- `game_sound_requests`: announcements the client plays (time warnings, game over, casualties).

`GameState::new(time_limit, planning_time)` starts untimed without a time limit, in the pre-game
with both, and timed otherwise.

The game ends through `GameState::end_game`, called by a map action, by the statistics (a team about to win, or all but one team
eliminated), by an END_GAME system
command, or by the timer. `end_game` honours a suspension (`suspend_end_game`), picks the winner
(`finalize_winning_team`: an existing winner, else the standings' leader), sets flag 2, the end
tick, and state 5 with a 300-tick timer. When that timer runs out the game is over
(`GameState::game_over`, shell state `SHELL_STATE_POSTGAME`), the veterans table is built, and no
more bodies run.

In a replay the end comes sooner: the first statistics pass after flag 2 makes the final
statistics and opens the postgame carnage report, which pauses the game (`FLAG_PAUSED`). That
pause is what stops a film. `Simulation::close_carnage_report` unpauses it so the replay runs to
the end of the 300-tick timer.

Readers: `Simulation::game_end` (`GameEnd { tick, winner, code }` once flag 2 is set),
`Simulation::game_over`, `Simulation::ticks_stopped`.

## Monsters and their actions

Each `engine_rng::EngineMonster` has an identifier (the key the commands use), a definition index,
an object (`engine_rng::EngineObject`: position, height, velocity, scale, vitality, animation
state, flags), a mode (idle, walking, engaging, attacking, hit reaction, held, dying, ...), its
owner and team, its target, and the per-monster combat state (`attack_ai::MonsterCombat`).

`EngineWorld::update_monsters` runs once per tick for every monster in array order. Before the
loop the walkers' periodic re-plan runs (a monster re-plans when `index & 63 == tick & 63`). For
each monster: animation (`animate_object`: the sequence frame clock, key frames, sounds), the
leg start when it needs a new leg (path planning), then the handler of its mode. The mode changes
draw (new sequence start frames, speeds). Movement is `engine_move::EngineMover`; collisions with
other objects during a step are `collide`; the terrain test is `passability`.

### Movement, paths and passability

- `passability::MeshCells` holds every cell's terrain type (16 types, two of them walking- and
  flying-impassable) and flags (media, killed cells, scenery and model marks, connectors). A
  monster's impassable mask comes from its definition. `passability::move_monster` moves a
  monster one step against the cells and its obstruction box.
- `pathfinding::new_path` plans at every leg start: a best-first `search` over a 128x128 cell
  window around the start, then `first_point` keeps the first corner a straight walk cannot skip.
  `pathfinding::adjust_path_destination_for_obstacles` then pulls the target out of obstacles,
  clips the leg to 8192 units and steers around objects on the way by their tangents.
- `formations` builds formation slots for group moves (a heap sort orders the slots, as in the
  original engine; ties are broken exactly as it does).
- All of this is 32-bit wrapping integer arithmetic with the 1024-entry trig tables.

### Attacks and damage

- `attack_ai`: which target a monster engages (target search on its re-plan tick, engagement,
  attack choice from its definition's attacks), and TARGET commands.
- `engine_attack`: aim and release, including ballistic solving for thrown attacks, the crowd and
  model-obstruction checks, out-of-range handling and experience. A release creates projectiles in
  the middle of the monster's update: the world hands control to the `engine_attack::CombatHost`
  (implemented by `Simulation`), which runs projectile creation and, for melee, the immediate
  detonation, then the monster's update continues. The draw order across that hand-off is the
  original engine's.
- `engine_damage`: the damage amount (`DamageInfo`), modifiers by monster effects, damage to
  objects, the hit reaction and acceleration of the victim, kills, the dropped ammunition and
  carried objects.
- `combat`: shared helpers (`combat::roll_damage`, `combat::frequency_corrected`: one draw tested
  against a threshold with a gameplay-version-dependent comparison) and `CombatEvent`s for the
  client (attacks ordered, projectiles launched and detonated, units damaged, hit reactions,
  sounds, footsteps).

### Map actions

`map_actions::MapActions` interprets the mesh's map-action scripts (see
[data-formats.md](data-formats.md) for the tag) plus one action per ambient monster. The actions
run in tick bodies whose tick is a multiple of 32, and once during the map load. Each active action
whose time has come is triggered and rescheduled or deactivated by its kind; activation can draw a
random delay. Procs are implemented in `map_actions` itself and in the `*_action` modules listed
in the module map; `map_attacks` is their attack AI. Their orders run at once through the same
executors as player commands (with a negative source instead of a player). The `obmo` action's
camera moves are recorded in `MapActions::camera_actions` for the client.

### Projectiles, explosions, lightning, fire, particles, effects

- `flight` / `projectiles` / `projectile_groups`: projectile records are created by attacks,
  projectile groups and markers; each tick `update_projectile_flight` moves them (gravity, drag,
  wind), tests collisions with objects, terrain, media and models, and detonates them (damage,
  groups, stains, fire, lightning). Objects that come to rest become dormant objects
  (`flight::DormantObject`) that can be picked up or woken by explosions.
- `explosions`: area damage with a delay grows an explosion record's radius each tick and damages
  each victim the radius passes.
- `lightning`: a projectile with a lightning definition makes a lightning record whose head runs
  along a jittered beam; it damages and stains as it goes, and the projectile follows it.
- `fire`: meshes that allow fire burn cells hit by fire projectiles; each burning cell does area
  damage every 8 ticks and tries its neighbours when its timer runs out.
- `particles`: only the weather intensity clock (it draws); the particles are the client's.
- `mesh_effects`, `model_effects`, `model_animations`, `connectors`: terrain waves, model effects
  started by map actions, flip-book animated models (which change passable cells), connector walls.

## Statistics and the carnage report

`statistics::Statistics` (in `EngineWorld::statistics`) keeps the original engine's statistics
block: totals per player, team and unit type (kills, losses, damage given and taken, costs), the
rankings, 20 time-series samples (`statistics::SAMPLES`) for the graphs, and the permanent
rankings that carry across a campaign. It is updated in the statistics stage of every tick and
frozen when the game ends. The totals change nothing in the game (no draws), except the game
actions the statistics pass takes (eliminating a team, ending the game, alliances broken when two
teams remain).

The client reads it through `Simulation::statistics` (a `statistics::StatisticsView` with the
players, game type, mode and end code) and `Simulation::statistics_samples`; the view gives the
report's pages (`ranking_lines`, `permanent_lines`, `graph_points`, `epilogue`). The client prints
the report to stdout at a map's end and draws the postgame dialog; see
[interface.md](interface.md).

## Saves, veterans, dropped players

- **Saved games** are films. Restoring one replays its film (mode 0); when the film is over the
  client calls `Simulation::end_saved_game_replay` with the save's observer, which switches the game
  to mode 2 and queues, stamped for the next tick, the restore's player commands
  (`save_restore::solo_restore_entries`, REPLACE_PLAYER, SET_TEAM_CAPTAIN) and SAVE_LOADED. The
  client then runs ticks until `Simulation::save_loaded` is true and reads
  `Simulation::save_loaded_observer` for the camera. None of these commands is recorded. See
  [session.md](session.md).
- **Veterans**: `veterans::MonsterInitializers` is the table of surviving units (names, experience)
  built when a game ends. `Simulation::monster_initializers` and `Simulation::permanent_rankings`
  read it after the game; the next level's simulation gets them in
  `Simulation::carried_initializers` and `Simulation::carried_rankings` before its map load, and
  game option `players::VETERANS_ALLOWED` decides whether the load keeps them. A film carries its
  own block of initializers in its header.
- **Dropped players**: DROP_PLAYER hands the dropped player's units to the next teammate or
  eliminates the team (`drop_player::DroppedPlayer`).

## Network games (mode 1)

`Simulation::net: Option<net_game::NetGame>` holds the role (`NetRole::Host`, the network
originator, or `NetRole::Joiner`), the horizon, the system queue, the forced end (rehost/restart,
`net_game::FORCED_ENDGAME`), and the speed index set by GAME_SPEED commands. Differences from the
solo game:

- local commands are sent, not stamped (`take_outgoing_commands`), and come back through
  `queue_incoming_command`;
- only the host queues SYNCs: at the end of each frame `Simulation::net_end_of_batch(requested)`
  takes the frame's requested ticks off the SYNC timer, and below zero the host queues a SYNC and
  resets it to `SYNC_TIMER_RESET`;
- pause and game speed are stamped system commands: every peer pauses before the body of the
  pause's tick (`net_body_may_run`, `net_head`, `net_paused`, `net_game_speed`);
  `take_system_events` gives the client what to show;
- an out-of-sync report logs and sets `FLAG_OUT_OF_SYNC`, and the game goes on.

The dedicated server runs the same simulation as an invisible peer; see [server.md](server.md).

## Random numbers

### The two streams

The original engine has two generators with the same LCG formula and separate states:

- **The synchronised generator**: `Simulation::rng` (`mandrake_math::BungieRng`). Every draw that can
  change the game comes from it, on every peer and in every replay in the same order. The film's
  or game's seed starts it (`players::set_random_seed`, 0 becomes `0xfded`).
- **The local generator**: not part of the simulation. Sounds, particles, blood and terrain
  stains, local projectile groups and other presentation draw from it in the client
  (`mandrake_tag::sound_manager::LocalRandom` is the same formula). The simulation never draws for these;
  where the original engine draws locally the port's comments say so and make no draw.

There is no reseeding and no second synchronised stream: the seeds in a film's successive SYNCs
are linked by the LCG alone.

### Counted draws

Engine code draws through `engine_rng::EngineRng { rng, sites }`: `EngineRng::draw(site)` returns
`next_u16` and counts the draw under `site` in `engine_rng::SiteCounts`;
`EngineRng::draw_mod(site, n)` is the original's `random() % n` (and reports `Unmodelled` for
`n == 0`, which the original would divide by). `site` is an opaque constant per call site, used by
tools and tests to attribute draws. Every site is a named constant in `mandrake_sim::draw_site`, named
after what the draw is for (`draw_site::FIRE_LIFETIME`); a new call site gets a new constant
there, with a value no other constant has; `Simulation::engine_sites` totals them. When the thread-local
`engine_rng::DRAW_LOG` is `Some`, every draw is appended as `(site, state before the draw)`; the
client's `--film-report` and the replay tools write it out.

### Rules every draw follows

- A draw happens only where the original draws. The idiom `base + (random * range) >> 16` draws
  only when `range > 0`; `BungieRng::next_range` has the same "no draw when the range is empty"
  property. Whether a draw happens often depends on definition data and the gameplay version, so
  the number of draws per tick is data-dependent.
- Draws happen in stage order (the tick list above), and inside a stage in the original's order:
  monsters in array order, actions in array order, cells in row-then-column order, victims in the
  order the cell walk finds them.
- `random() % n` uses the original's unsigned 16-bit sample and signed division.
- Probability tests (`combat::frequency_corrected`) take one draw and compare it with a threshold
  using the comparison the gameplay version selects.

## Determinism

### What must hold

- Integer arithmetic only on game state, with the original's widths and wrapping (32-bit wrapping
  for positions and products, 16-bit fields where the original has them, arithmetic shifts for
  signed scaling).
- No iteration over `HashMap`/`HashSet` order that can reach game state. The unit and tag maps use
  `entities::UnitIdHasher` and `entities::TagIdHasher` for speed; nothing may depend on their order
  (the command stage sorts ticks; the engine model keeps its own arrays in the original's order).
- Commands run in the body whose tick they carry; within a tick, in arrival order.
- No clock, thread, address or allocation order influences the game.
- The gameplay version gates behaviour exactly where the original does (`gameplay` module; tests
  are `> 0x726`, `> 0x731` and similar on `formation_globals.gameplay_version`). 1.8.5 games run
  at `gameplay::GAMEPLAY_1_8_5` (`0x73c`).
- Interface state reaches the game only through commands, with one exception the original has
  too: the local selection is passed to the map actions (`EngineWorld::local_selection`, from
  `UnitEntity::selected`) for the training map's selection test.

### What desyncs a film or network game

- A missing, extra or reordered synchronised draw anywhere (the most common cause).
- A stage moved in the tick body, or an action, monster or victim visited in another order.
- A command executed on a different tick (wrong `command_lead`, local commands stamped in a replay),
  or a command handled differently for the film's gameplay version.
- A floating-point result that differs between targets reaching game state.
- A definition read before or without the `mons` postprocess, or tags from another archive order.
- Changing the map-load order (`set_up_map`), or calling the set-up steps by hand.

### Floating point

Floats are allowed only where every target gives the same bits. IEEE add, subtract, multiply,
divide, `sqrt`, `floor`/`trunc` and Rust `as` casts are identical on x86_64, aarch64 and wasm32.
C-library functions are not guaranteed to be, and the simulation uses them in three places only:

| Where | Call | What the simulation keeps |
|---|---|---|
| `engine_move::table_sin_cos` (for `engine_move::trig_tables`) | `sin`, `cos` of `i * TAU / 1024` | `trunc(x * 16384 + 0.5)` table entries |
| `particles::transition_cos` | `cos` of an `f32` in `0..=pi` | the particle intensity steps |
| `mandrake_tag::model_hit::build_rotation_matrix` | `sin_cos` of a 16-bit angle | `trunc(x * 16384)` matrix words |

The last bits of the raw results do differ between glibc and the wasm `libm`, but the nearest
result is thousands of ulps away from changing a kept integer. `float_targets::hashes` hashes
each call's raw results and the kept integers over the whole input range; a test pins them (see
"Verifying changes"). Other float uses are plain IEEE arithmetic: integer square-root seeds,
damage and mana factors, formation radii, placement conversions at set-up, and the carnage report's
shares and ratios (output only). Any new float on game state needs the same treatment: plain IEEE
operations only, or a pinned table.

### The port's state checksum

`Simulation::state_checksum` (returned in `TickStateHash`) is the port's own rolling hash:
`checksum.rotate_left(5) ^ hash(tick, rng.state, commands run, units' ids and positions,
projectiles' ids and positions)`. It is not part of any file or wire format and is not compared
with the original game. Tools print it to compare two builds of the port.

## The SYNC check

The original engine has no state checksum on the wire. Its SYNC command (type 1) carries a
snapshot of the synchronised generator:

`mandrake_rec::command::SyncCommand`, 12 bytes big-endian: `u16 flags` (0) at +0, two unused bytes,
`u32 random_seed` at +4, `u32 snapshot_tick` at +8 (a
tick number, not a hash). `mandrake_rec::writer::sync_command(seed, tick)` builds one.

- **Snapshot**: at the top of every tick body whose tick is a nonzero multiple of 512, before any
  stage draws, `Simulation::sync_snapshot = (rng.state, tick)`. Before the first such body the
  snapshot is the game's seed with tick 0 (the state before the map load's draws).
- **Sending**: a solo game (mode 2) counts down `sync_timer` once per body (`count_down_sync`);
  when it goes below zero `queue_sync_command` puts a SYNC of the current snapshot on the local
  queue and the timer restarts at `SYNC_TIMER_RESET` (170). The first SYNC is stamped for tick 171,
  then one every 171 ticks; films recorded by the original game have the same pattern. In a network
  game only the host sends them (`net_end_of_batch`). The SYNCs of a film given as live input are
  dropped, because the live game queues its own (`give_film_commands`).
- **Checking**: when a SYNC runs in `execute_monster_command`, it is compared only if its snapshot
  tick equals the local snapshot's tick (`sync_differs` in `net_game`); a SYNC carrying an older
  snapshot is skipped silently. A different seed calls `Simulation::game_is_out_of_sync`, once per
  game (`Simulation::is_out_of_sync`).
- **Reporting**: `game_is_out_of_sync` logs one `tracing::error!` line,
  `OUT OF SYNC: OOS occurred at line L in FILE - game time: T random seed: S`, pushes a
  `net_game::OutOfSync { file, line, tick, seed }` onto `Simulation::out_of_sync`, and sets
  `FLAG_OUT_OF_SYNC`. A late command reports the same way (without the once-only guard). The game
  goes on.

Because a SYNC holds the generator state, a SYNC miss at snapshot tick `T` means the port's draw
count or order differed somewhere in ticks `T - 512 .. T` (or in the map load, for the first
one). Matching SYNCs do not prove positions match, but in practice every divergence changes the
draws within a few ticks.

## What the client may read and must not write

The client (and the server, and the web client) drives the simulation through these calls only:

- set-up: the constructors, `set_up_map`, `load_combat_definitions`, `carried_initializers` /
  `carried_rankings` before the load, `recorded_commands = Some(Vec::new())` to record a film,
  `end_recording`;
- input: the local queue functions (`queue_local_movement` and its siblings), `select_units` / `deselect_all` (selection flags),
  `give_film_commands`, `queue_incoming_command`, `take_outgoing_commands`;
- time: `step_with_commands` (or `run_command_stage` + `finish_tick_body`), `net_head`,
  `net_end_of_batch`, `close_carnage_report`, `end_saved_game_replay`, `force_network_endgame`.

It reads freely: `units` (positions, facing, health, team, selection), `views` (object shapes,
projectile objects, move markers), `projectile_views`, `object_location` and the object history,
`model_animation_frames`, `drain_combat_events`, `drain_sequence_sounds`,
`take_model_animation_sounds`, `engine` (read-only: monsters, objects, mesh cells, game state,
statistics, camera actions in the map actions, game sound requests), `statistics`, `game_end`,
`game_over`, `ticks_stopped`, `engine_stop`, `out_of_sync`, `current_tick`.

It must not write game state: `rng`, `engine` and anything inside it, `commands_by_tick` (outside
the input calls), units' positions, health or teams, `current_tick`, `sync_snapshot`. Any such
write either desyncs a film or network game at once or is overwritten by the next body. The drain
and take calls consume presentation output only. Interpolation between ticks, camera and
sound are entirely client-side (see [client.md](client.md)).

## mandrake-common

`mandrake-common` gives the simulation `mandrake_common::FourCC` (four-character tag ids, also the key of most
definition maps), `mandrake_common::M2Error` (parse errors from the constructors), Mac Roman text
conversion for names, and the virtual file system the client and tools use to find data (the
simulation itself takes bytes, not paths).

## Verifying changes

### Tests that ship

The simulation's tests that need no game data:

- Unit tests in `crates/mandrake-sim/src` (`#[cfg(test)]` modules) build synthetic definitions, monsters
  and worlds and check individual rules: damage rolls and probability tests (`combat`), map-action
  procs and their draws (`map_actions`, `particles`, `delay_action`), collisions (`collide`),
  explosions, the alliance matrix, drop player and detach (`net_game`), formations, and similar.
- `mandrake-math`'s unit tests: the generator's known sequence and range bounds (`rng`), fixed point,
  trig and vectors.
- The integration tests in `crates/mandrake-sim/tests/all/` that use synthetic data only:
  `combat_sequencing` (attack release and impact order), `projectile_flight` (a ranged attack flies
  and detonates), `general_commands` and `local_commands` (command headers and the tick they run
  in), `part_action` and `model_effects` (map-action procs), `mac_roman_names` (a film's non-ASCII
  names survive the reader, the simulation and the writer), and `float_targets` (the pinned
  library-float hashes: what the simulation keeps must hash the same on every target, and the raw
  bits must match glibc on Linux or the wasm `libm` on wasm32).

Run them inside the dev shell:

```
cargo test -p mandrake-math
cargo test -p mandrake-sim
cargo test -p mandrake-sim --test all float_targets::
```

### One test binary

`mandrake-sim` sets `autotests = false` and declares one integration test target, `all`, at
`crates/mandrake-sim/tests/all/main.rs`. Each test file is a module listed there (`mod combat_sequencing;`
and so on); a new file in `tests/all/` runs only once it has a `mod` line in `main.rs`. Run one file's
tests with `cargo test -p mandrake-sim --test all <module>::`. The draw log and the other diagnostic logs
are thread-local, so tests running in parallel threads do not see each other's draws; keep new
logs thread-local too.

The cargo feature `identity-checks` turns on assertions that compare the simulation's indexed and
scoped lookups with full scans; enable it with `--features identity-checks` when changing those
indexes.

### Replaying a film and spotting a SYNC miss

Films need your own Myth II 1.8.5 install (the data directory with `tags/` and `plugins/`, see
[data-formats.md](data-formats.md)) and films recorded by the original 1.8.5 game. Any change to
`mandrake-sim` that can touch game state should be checked by replaying films to their end:

1. **The film bench** (headless, the cleanest report):

   ```
   cargo run --release -p mandrake-web --example film_bench -- FILM.m2rec [GAME_DIR]
   ```

   `GAME_DIR` defaults to `MYTH2_DATA_DIR`. It runs `mandrake_web::film_replay::replay`, which sets the
   film up with `Simulation::replay_film` and steps it to its end, a stop, or 60000 ticks, and
   prints one summary line with `first_sync_miss=` (the first snapshot tick whose seed in the film
   differs from the port's generator state at the top of that tick, or `none`), `end=` (`end_game
   in tick T: winner W, code C`, `stop in tick T: ...`, `stop in load: ...`, or `no end_game by
   tick T`), the draw count and a hash of every draw (`draws=`, `draws_sha256=`), the port's
   `state_checksum`, the final generator state, and tick timings. A correct port of a film from the
   original game shows `first_sync_miss=none` and ends with `end_game`. The same function runs in
   the browser build, so the native and wasm lines can be compared directly.

2. **The client**: `mandrake --replay FILM.m2rec` plays a film in a window (`--game-speed 64`
   speeds it up); at its end the carnage report is printed to stdout. A SYNC miss or late command
   prints the `OUT OF SYNC: ...` error line on stderr (errors are shown with `RUST_LOG` unset; set
   `RUST_LOG=mandrake_sim=error` to see only these). `--film-report FILE` additionally writes every draw
   as `tick site seed` (tick -1 is the map load) and, in `FILE.end`, the end tick, winner, code,
   game state, generator state, checksum, draw count and any engine stop. A stop is also shown on
   screen with its tick.

3. **Before and after a refactor**: a change that should not alter behaviour must leave the bench
   line's `draws_sha256`, `first_sync_miss`, `end` and `state_checksum` unchanged for every film you
   have. For a tick-by-tick comparison, `cargo run --release -p mandrake-sim --example tick_cost -- FILM
   OUT.tsv --state` (the film is a name in `$MYTH2_DATA_DIR/recordings`) writes each tick's
   generator state, checksum, draws and hashes of monsters, scenery, object history, projectiles,
   dormant objects and units; compare two builds' outputs with `cmp` to find the first tick that
   differs.

4. **Floating point on another target**: `cargo run -p mandrake-sim --example float_targets -- hashes`
   prints the hashes `float_targets::hashes` computes (with the particle cosine over every `f32`
   when the stride is 1); compare the kept-integer hashes across targets.

When a film's first SYNC miss moves earlier or its end changes, find the first tick whose draws
differ (draw logs or the `tick_cost` example with `--state` against the previous build) and fix the port there.
