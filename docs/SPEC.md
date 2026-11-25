# Myth II source port: specification

This is the map of the code. It is written for someone, human or agent, who has the source and
nothing else. Start here, then read the part you need.

The project is a Rust re-implementation of *Myth II: Soulblighter*. Its reference is **the original
1.8.5 engine**. Two rules drive it:

- **Gameplay is exact.** The simulation is deterministic and makes the original's decisions in the
  original's order, down to each random draw. Films recorded by the original game replay tick for
  tick, and network games interoperate with original 1.8.5 peers.
- **Pictures and sound come from the game's own data.** Visual modernisation is allowed: filters,
  fog, water, frame rate, interface scale. Generated art is not, apart from three named exceptions.
  See [AGENTS.md](../AGENTS.md).

## Parts

Engine:
- [data-formats.md](data-formats.md): tag archives, `.256` collections, `mesh`/`geom`/`scen`/
  `unit` and the other tags, plugins, the data folder layout.
- [simulation.md](simulation.md): `mandrake-sim`. The tick loop, determinism, random numbers, units, AI,
  projectiles, game end, the SYNC checksum.
- [films.md](films.md): `mandrake-rec`. The film format, recording, replay.
- [session.md](session.md): `mandrake-session`. Game setup, saves, campaign flow, the menu state machine.
- [networking.md](networking.md): `mandrake-net`. The wire protocol compatible with 1.8.5, lobby, orders,
  drops.
- [server.md](server.md): `mandrake-server`. The dedicated server, the WebSocket proxy, Docker.
- [tfl.md](tfl.md): `mandrake-tfl-sim`. *Myth: The Fallen Lords* and what it shares with Myth II.

Client:
- [client.md](client.md): `mandrake-client`. Startup, the game switch, the main loop, input and controls,
  camera, interpolation between ticks, settings files.
- [rendering.md](rendering.md): `mandrake-view` and `mandrake-render`. The wgpu passes, shaders, terrain,
  sprites, models, fog, lighting, water, the frame budget.
- [audio.md](audio.md): `mandrake-audio` and `mandrake-smacker`. The mixer, 3D sound, reverb, cutscenes.
- [interface.md](interface.md): `mandrake-dialogs`. The dialog boundary, the screens, how a replacement
  interface plugs in, the in-game bars.
- [web.md](web.md): `mandrake-web`. The WASM build, JS glue, browser audio and networking.
- [configuration.md](configuration.md): every key of every shipped settings file.

## Crates

Sixteen crates in `crates/`, two tools in `tools/`. All are in one Cargo workspace, edition 2024,
licensed MIT OR Apache-2.0.

### Dependency graph

Only dependencies inside the workspace are shown. An arrow means "depends on".

```
mandrake-common     (none)
mandrake-math       -> mandrake-common
mandrake-tag        -> mandrake-common
mandrake-rec        -> mandrake-common, mandrake-math
mandrake-smacker    (none)
mandrake-audio      -> mandrake-common
mandrake-view       -> mandrake-common, mandrake-tag
mandrake-net        -> mandrake-rec
mandrake-sim        -> mandrake-common, mandrake-math, mandrake-tag, mandrake-rec
mandrake-tfl-sim    -> mandrake-common, mandrake-math, mandrake-tag, mandrake-rec
mandrake-session    -> mandrake-common, mandrake-tag, mandrake-rec, mandrake-net
mandrake-render     -> mandrake-common, mandrake-math, mandrake-tag, mandrake-view
mandrake-dialogs    -> mandrake-common, mandrake-tag, mandrake-view, mandrake-session, mandrake-net
mandrake-server     -> mandrake-common, mandrake-tag, mandrake-rec, mandrake-sim, mandrake-net, mandrake-session
mandrake-client     -> mandrake-common, mandrake-math, mandrake-tag, mandrake-rec, mandrake-session, mandrake-net, mandrake-sim, mandrake-tfl-sim,
                 mandrake-render, mandrake-audio, mandrake-dialogs, mandrake-smacker
mandrake-web        -> mandrake-client, mandrake-common, mandrake-math, mandrake-tag, mandrake-rec, mandrake-sim, mandrake-render, mandrake-audio
m2-tag-dump   -> mandrake-common, mandrake-tag, mandrake-sim, mandrake-rec
m2-rec-dump   -> mandrake-common, mandrake-rec
```

The graph keeps some boundaries, so keep them when you change code:

- The simulations (`mandrake-sim`, `mandrake-tfl-sim`) have no window, GPU, audio or network code.
- `mandrake-session` knows only the data types of `mandrake-net`, never its sockets.
- `mandrake-view` and `mandrake-dialogs` have no wgpu. They describe pictures as data, and `mandrake-render` draws
  them.
- `mandrake-audio` has no device. The client opens one.

### One paragraph per crate

**mandrake-common**: shared error type `M2Error`, four-character tag ids `FourCC`, and the game-folder
file system `VirtualFileSystem` with `NativeVfs` (disk) and `MemoryVfs` (the browser). The process
reads game data through `game_vfs()` and installs a file system with `set_game_vfs()`. Also Mac
Roman text conversion (`mac_roman`).

**mandrake-math**: deterministic math that gives the same bits on x86, ARM and wasm.
- `Fixed16` is 16.16 fixed point.
- `Angle512` is 512 units per turn, with table-driven `sin`, `cos` and `atan2`.
- `Vec3Fixed` is a fixed-point vector.
- `BungieRng` is the original game's 32-bit linear congruential generator: `next_u16`,
  `next_range`.

**mandrake-tag**: reads the original's monolithic tag archives (`TagArchive`) and parses the tag types:
- `MeshTag` (maps), `CollectionTag` (`.256` sprites and textures), `UnitTag`, `ObjectDefinition`,
  `ProjectileDefinition`, `SceneryTag`;
- dialogs (`DialogDefinition`), fonts, string lists;
- `SoundManager` (`sound_manager`: the original's sound channel logic).

`tag_files` reproduces the original search order over `tags/` and `plugins/`
(`engine_tag_files`, `TagLookup`). `mandrake_tag::tfl` reads The Fallen Lords' data.

**mandrake-rec**: films (`.m2rec`/recordings) and saved games.
- `Recording` holds a `RecordingHeader` (map, seed, plugins, players, teams, game options) and the
  commands by tick (`CommandPacket`, `CommandType`, `CommandPayload`).
- `writer::Recorder` writes films, and `writer::sync_command` builds SYNC commands.
- `saved_game::SavedGame` reads and writes saves.
- `tfl::TflRecording` handles TFL films.

**mandrake-sim**: the deterministic Myth II simulation, a port of the original engine's rules: movement
and pathfinding, combat, projectiles, AI, formations, map actions, statistics, the game's end, saves.
- `Simulation` is the whole game state. It is built with `from_film`, `from_recording`,
  `solo_game` or `command_line_game`.
- `Simulation::step_with_commands` runs one tick and returns a `TickStateHash`.
- Local input enters through the `queue_local_*` functions.

There is no window or GPU. See [simulation.md](simulation.md).

**mandrake-tfl-sim**: the rules of *Myth: The Fallen Lords* 1.5, beside `mandrake-sim` (`TflSim`, `TflRng`).
It shares only pure types with Myth II. Where it doesn't model something, it returns a `Stop`
instead of guessing. See [tfl.md](tfl.md).

**mandrake-session**: the menus and pregame screens as a state machine with no window, GPU or sim.
- `Session::act(Action)`, `game_event(GameEvent)`, `take_requests()` and `screen()`.
- The front-end boundary trait `FrontEnd`.
- `Catalog` (campaigns and levels), `Progress`, the film and save folders, network flow
  (`NetRequest`/`NetEvent`), chat dot commands.
- `tfl::TflSession` for The Fallen Lords.

**mandrake-net**: TCP/IP multiplayer compatible with the original 1.8.5 wire protocol. The protocol is
written as socket-free state machines:
- joiner: `topology::Topology` + `client::Client` = `endpoint::Joiner`;
- host: `server::Server` + `topology::HostTopology` = `endpoint::Host`.

Sockets live only in `transport` and `endpoint::Endpoint`. `wire::Message` is the framing,
`game_description::GameDescription` the game, and `lan` the LAN list. See
[networking.md](networking.md).

**mandrake-view**: the GPU-free half of drawing. It holds the camera and the original observer motion
(`camera`, `ObserverController`), the CPU-shaded landscape texels (`landscape`), sprite quads, view
selection and the original draw order (`billboard`), selection frames and bars (`selection`), view
regions, water behaviour (`media`), model surface rules, and the `InterfacePicture` data that
dialogs produce. See [rendering.md](rendering.md).

**mandrake-render**: the wgpu renderer.
- Renderers: `TerrainRenderer`, `ModelRenderer`, `BillboardRenderer`, `OverlayRenderer`.
- Interface: `InterfaceScaler`, `InterfacePresenter`, `ScreenFade`.
- Shared data: the uniform block `SceneUniforms`, the WGSL shaders in `shaders/`, and the enhanced
  water (`water::WaterShading`).

It re-exports `mandrake-view`'s modules. See [rendering.md](rendering.md).

**mandrake-audio**: the software mixer.
- `SharedAudio` is the handle.
- Eight engine channels match the original's channel model.
- 3D panning and Doppler as the original's 3D device does them, and a reverb with the original's
  26 presets (`REVERB_PRESETS`).

There is no device and no I/O. See [audio.md](audio.md).

**mandrake-smacker**: a Smacker video decoder with no dependencies, for the cutscenes: `Smacker`,
`Decoder`, `Frame`, audio chunks. See [audio.md](audio.md#cutscenes).

**mandrake-dialogs**: the original game's dialogs.
- `EngineDialogs` implements `mandrake_session::FrontEnd`. It is built from the game's dialog tags and
  drawn in software.
- Dialogs, controls and screens: `core::Dialog`, `DialogStack`, `controls`, `screens`.
- `tfl::TflDialogs` for The Fallen Lords.

See [interface.md](interface.md).

**mandrake-client**: the game: window, input, camera, the per-frame build of what is drawn, HUD, sound,
films, saves and network play, around one simulation. It is a library (`mandrake_client::run`) with a
native binary, `mandrake` (the mandrake-client crate). See [client.md](client.md).

**mandrake-web**: the browser build, a `cdylib` with `wasm-bindgen` exports. `add_game_file` loads files,
`start_game` starts the game, and `set_audio_rate`/`mix_audio` run audio. JS glue lives in `js/`.
It runs `mandrake-client` unchanged. See [web.md](web.md).

**mandrake-server**: the dedicated server.
- `mandrake-server` binary: a headless host whose own player only observes. It runs its own sim from the
  stamped commands and writes one film per game.
- `mandrake-ws-proxy` binary: carries browsers' WebSockets to the server over TCP, and serves the web
  client and, optionally, game data.
- `mandrake-ws-tunnel` binary: a test tunnel.

Docker setup in `docker/`. See [server.md](server.md).

**tools/m2-tag-dump**: a command-line inspector for tag archives. It lists tags (`--info`, `-t`,
`-n`) and extracts tags (`-e`, `dump-raw`). Its subcommands inspect a tag type or write a report:
`inspect-mesh`, `inspect-unit`, `inspect-coll`, `export-obj`, `media-report`, `lighting-report`,
`map-actions`, and others. `--help` lists them.

**tools/m2-rec-dump**: prints a film as JSON, `m2-rec-dump FILM > film.json`. The output holds the
header, a command summary, and the commands by tick.

## The tick and frame model

```
wall clock ──► 30 Hz input polls ──► game ticks (speed x polls) ──► Simulation::step_with_commands
                     │                                                   │
                     │ orders stamped one tick ahead                     │ object history kept before each tick
                     ▼                                                   ▼
               camera, selection                       frame: fraction f in [0,1] since the last tick
                                                        draw = lerp(history, live, f)  at up to 240 fps
```

- **The simulation runs in ticks of 1/30 s.** Its whole state advances only in
  `Simulation::step_with_commands`. Inputs are the commands stamped for that tick, from the local
  player, a film or the network. The sim uses fixed point, integer world units (512 per cell) and
  `BungieRng`, and never reads the clock or the frame.
- **Input is polled at 30 Hz, from the wall clock.** At normal speed one poll is one tick. The game
  speed (F1/F2, from stopped up to 64x) changes how many ticks run per poll. A network game's ticks
  are gated by the host's stamps.
- **Frames are independent of ticks.** Each frame computes the fraction of a tick that has passed.
  Units and projectiles are drawn between their state before the last tick and their live state,
  and the camera is drawn between its last two polls. The interpolation lives entirely in the
  client, so the sim never sees it. The project's frame budget is 4 ms (240 fps).
- **SYNC.** Periodically the host stamps a SYNC command that carries its random seed and tick. Every
  peer, and every film replay, compares that against its own state. A mismatch means the
  simulation diverged from the original engine's, and the sim records it (`Simulation::out_of_sync`).
  Replaying the original game's films to their end with no SYNC mismatch is the main proof of
  correctness. See [simulation.md](simulation.md).

## Glossary

| Term | Meaning |
|---|---|
| **tag** | A typed resource in the original's data, identified by a four-character type and id (`FourCC`), for example `mesh` `cr01` |
| **tag archive** | A file of many tags: `tags/small install`, `large install`, a plugin |
| **plugin** | An add-on archive in `plugins/` (maps, patches, interface art). A film or game names the plugins it needs |
| **`.256` / collection** | A sprite or texture collection (8-bit art with shading tables) |
| **mesh** | A map: landscape, objects, map actions. Its id names the map (`cr01` is Salvation) |
| **media** | Water or other liquid on a map. `crates/mandrake-client/src/media.rs` and `mandrake_view::media` are about water, not multimedia |
| **film / recording** | A recorded game: the header plus every command by tick. Replaying it re-simulates the game |
| **tick** | One 1/30 s step of the simulation |
| **poll** | One 30 Hz read of the input. With game speed, polls and ticks differ |
| **command / order** | A player's input as the sim sees it (move, attack, special), stamped for a tick |
| **SYNC** | A periodic command that carries the host's random state, used to detect divergence |
| **observer** | The original game's camera: a ground point with facing, zoom and pitch |
| **category 2 / 4** | The original engine's render categories: landscape-coplanar model polygons, and water reflections |
| **billboard** | A camera-facing sprite quad (units, scenery, effects) |
| **session** | The menu-and-campaign state machine (`mandrake_session::Session`) |
| **front end** | Whatever implements `FrontEnd` and draws the session's screens. The original dialogs are one |
| **dialog** | One of the original game's dialog resources (`bina` + `ditl` tags) |
| **TFL** | *Myth: The Fallen Lords*, the first game, supported in the same client |
| **veterans** | Units carried from one level or network game to the next |
| **mode 1 / mode 2** | Network game vs solo game, in the original engine's terms |
