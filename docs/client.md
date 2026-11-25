# The client (mandrake-client)

`crates/mandrake-client` is the game: window, input, camera, menus, sound, drawing and network play,
around one deterministic simulation (`mandrake_sim::Simulation`, or `mandrake_tfl_sim::TflSim` for The Fallen
Lords). It is a **library**, so the browser runs the same code (see [web.md](web.md)).
`src/main.rs` is the native start: it sets up a `tracing` logger on stderr, then calls
`mandrake_client::run` with `mandrake_client::parse_command_line`'s arguments: the command line with the
settings file's `[display]` keys for the flags it doesn't give (`conf::parse_args`).

## Startup (`src/lib.rs`, `run`)

`run(args)` decides what kind of run this is, in this order:

1. **Which game.** `tfl_game::chosen_game(&args)` returns `mandrake_tag::tfl::Game`. It is `Myth2` or
   `Tfl`, from `--game`, from a `--replay` that is a TFL film, or from a `--tfl-data` folder that
   holds TFL data; with nothing deciding it, Soulblighter (nothing asks). A TFL
   run continues in `tfl_view::run_tfl` and its window, `tfl_window::TflApp`. TFL shares the shell,
   dialogs, input, settings and renderers with Myth II, but has its own sim and world adapter.
2. **Tool exits.** `--write-default-conf` writes the shipped settings file and exits. (Before
   it, `conf::migrate` builds `mandrake.toml` from the old per-kind files once, and
   `conf::set_in_use` records the `--conf` file for what the client writes.)
3. **The menus.** `args::wants_menu`: no map, film, network address or tool named, or `--session`.
   This calls `app::shell_menu::run_menu`, the full game: main menu, campaign, saved games, films
   and multiplayer, driven by `mandrake_session::Session` (see [interface.md](interface.md)).
4. **A direct network game.** `--net-host PORT` or `--net-join HOST:PORT`. `net_game::host_and_wait`
   or `join_and_wait` runs the pregame before any window opens. Then
   `startup::start_network_level` and `app::window::run_network_window`. With `--headless`,
   `net_game::run_headless_host`/`run_headless_join` play with no window.
5. **A direct map or film.** `--mesh`, `--play MESH` or `--replay FILM`. `startup::start_game` loads
   it (`Startup`), then:
   - `headless_run::run_headless` if `--headless` or `--screenshot` was given;
   - otherwise `app::window::run_window`.

At exit, `interface_config::save_at_exit` writes back the control-bar state (see
[configuration.md](configuration.md)).

**Finding the game.** The command line names the tag archive with `--archive` (for example
`<Myth II>/tags/small install`). The game folder is the archive's grandparent. Around the archive
the client finds:

- the other `tags/` archives;
- `plugins/`, ordered as the original game orders them (`mandrake_tag::tag_files`);
- `recordings/` (films);
- `saved games/`;
- `cutscenes/`;
- `preferences/`.

`--plugin NAME` loads an installed plugin. A film names its own plugins. All game-file reads go
through `game_files.rs` to the global `mandrake_common::game_vfs()`, which is the disk natively and an
in-memory file system in the browser.

The client writes into the game folder in only two places, as the original game does: films go
into `recordings/` and saves into `saved games/`. It never writes the original game's preference
files. Its own settings live in the user's config folder.

**Loading a map** (`startup.rs`, `map_load.rs`):

- `map_load::load_mesh_and_texture` resolves the `mesh` tag and its landscape (`.256` collection).
  It is given a `MeshQuery` built from a film, from `--mesh` or from the session.
- The sim is built from the film header or from a solo or network game description
  (`Simulation::from_film`, `solo_game`, ...).
- `start_camera` places the observer where the map starts it.

## The window: shell and scenes

- `app::shell::Shell` is the winit `ApplicationHandler`, and it lives as long as the process.
  `ShellHost` owns the window, the wgpu device and surface (`ShellGpu`), the audio device,
  fullscreen, the frame cap (`app::fps_cap`), vsync, the screenshot key and the window title.
- The game inside the window is a `GameScene` (`app/game_scene.rs`). The shell gives it the input it
  doesn't handle itself, and calls it once per frame. There are two implementations:
  - `app::myth2_scene::Myth2Scene`: Myth II's menus (`app::shell_menu::Menu`) and its current map
    (`app::ClientApp`). Loading between them is run by `app::shell_game::LoadFlow`.
  - `tfl_window::TflApp`: The Fallen Lords.
- `ClientApp` (`app/mod.rs`) is **one loaded map**: the sim, the film being replayed or recorded,
  the camera, the selection, the HUD, the sound and the GPU state. Its behaviour is split into one
  file per job, each with its own `impl ClientApp`:

  | File | Job |
  |---|---|
  | `app/tick.rs` | ticks |
  | `app/input.rs` | input |
  | `app/orders.rs` | commands to units |
  | `app/camera.rs` | camera |
  | `app/sound.rs` | sound |
  | `app/gpu.rs` | GPU setup |
  | `app/render.rs` | drawing |
  | `app/recording.rs` | films |
  | `app/save_game.rs` | saves |
  | `app/detach.rs` | giving units to a teammate |

## Ticks and frames

The simulation runs at **30 ticks a second** and the screen draws as fast as the frame cap allows
(240 fps is the goal). Each frame, `ClientApp::render_frame` first calls
`ClientApp::update_physics_and_simulation` (`app/tick.rs`), which runs these steps:

1. **Network poll** (`poll_network`): in a network game, the commands stamped by the host go into
   the sim.
2. **Input polls at 30 Hz.** A wall-clock accumulator (`physics_accumulator`, capped at 0.25 s per
   frame) runs one input poll per 1/30 s that has passed. Each poll does the same as the original's
   `process_action`:
   - reads the polled action and the held bindings;
   - runs the tutorial's input filter (`local_input::sniff_polled_action`);
   - steps the camera (`step_observer`) and the mouse selection state machine (`step_selection`,
     `selection_input`);
   - dispatches key presses (`dispatch_press`).
   Orders become sim commands through `app/orders.rs`, which uses the sim's `queue_local_*`
   functions. They are stamped one tick ahead, as in the original engine.
3. **Game ticks.** `tick_bodies(polls)` turns polls into ticks through `game_speed::GameSpeed`. The
   speed can be stopped, 1/16x up to 64x, or paused. In a network game the gate of the network
   endpoint decides instead. Each tick runs `game_tick`, which steps the sim, replays a film's
   commands or records the local ones, and plays the tick's sounds and effects.
4. **After the ticks**: particles, local projectile effects and the map's sound
   (`MapSound::update`, which counts polls rather than ticks).
5. **The interpolation fraction.** `CameraControl::motion_fraction(tick, now_ms, speed)` returns how
   far the frame is between the last tick and the next. It restarts at 0 with every tick and runs to
   1 over one tick's length at the current speed. It holds while the game is stopped. It is turned
   into a 16.16 value by `object_history::frame_fraction` and stored with
   `UnitAnimator::set_fraction`.
6. **The camera** is interpolated in one of two ways:
   - When a map action controls the camera, `interpolate_observer` is used.
   - Otherwise the render camera is `Camera::interpolate(prev_camera_state, alpha)`, where alpha is
     the accumulator's fraction of a poll. `prev_camera_state` is saved before each poll.
   The camera shake is applied after that.

**Object interpolation** (`object_history.rs`, `unit_anim.rs`). Before each tick the client keeps a
copy of every moving object's motion: the unit animator's `record_history` for units, and
`mandrake_sim::Simulation::drawn_projectile_views` for projectiles. Each frame draws
`ObjectMotion::interpolated(history, live, fraction)`, which moves position and facing from the
copy toward the live state. Objects created in the last tick have no copy and are drawn at their
live state. The live state is the original's rule: a frame at fraction 65536 shows the live state.
This happens only in the drawing; **the sim never sees the fraction.** Headless runs draw at
`--frame-fraction` (0 by default, the first frame after a tick).

Then `render_frame` draws the world (see [rendering.md](rendering.md#the-clients-frame)).

## Input and controls

- **Keys.** `controls.rs` maps keys to `controls::ControlAction` through `Controls`. The defaults are the
  original game's "Right Handed" set, and the user's `controls.toml` is merged over them (see
  [configuration.md](configuration.md#controls-controlsdefaulttoml)). `HeldKeys` tracks the keys
  that are down. `Controls::keyboard_axes`, `edge_scroll_axes` and `WheelState` turn keys, screen
  edges and the wheel into `ObserverAxes` for the camera. Most actions carry the original game's
  action id (`ControlAction::engine_id`), which is what the sim's input sees.
- **Mouse.** `selection_input.rs` is the original selection state machine:
  - a click selects, and a double-click selects all of a type;
  - Shift adds;
  - a drag becomes a band selection;
  - a right drag is a facing gesture.

  `picking.rs` and `mandrake_render::mesh_pick::pick_ground` find the unit or ground under the cursor.
  `attack_click.rs`, `move_marker.rs` and `selection_rules.rs` handle attack clicks, the move
  markers and who may be selected.
- **Window input** (`app/input.rs`): turns winit events into key presses, held keys, mouse moves and
  wheel notches. It routes the pointer to the HUD, dialogs or map. Keys that open dialogs become
  `DialogAsk` values for the session.
- **Not in the sim.** Some things are local only: chat (`console.rs`), the score table, the frame
  rate display, screenshots, fullscreen, the interface zoom (Ctrl +/-), and quitting (`quit`,
  default `Ctrl+Q`; Escape pauses, as in the original game).

## Camera (`camera_control.rs`, `mandrake_view::camera`)

- **The observer.** The camera is the original game's observer: a point on the ground with facing,
  zoom and pitch, moved by `ObserverController` with the original's physics constants
  (`myth_observer`). `CameraControl` holds the scripted camera moves of map actions, the camera
  shake, the map's limits (`set_mesh_limits`) and the motion clock (`MotionClock`).
- **Rendering it.** `Camera::from_myth_observer` turns the observer into a 3D eye, using the
  original's projection (`MYTH_HALF_FOV_DEG`).
- **Overrides.** `--zoom`, `--yaw`, `--pitch`, `--target-x` and `--target-z` replace it with an
  orbit camera. `--observer X,Y,FACING,ZOOM` places the original observer.

## Films and saves

- **Recording.** Every live game is recorded (`app/recording.rs`). The film is written to the game
  folder's `recordings/` under the last recording's name, as the original game does. It is written
  again every `CHECKPOINT_TICKS` (30 s of game time) and at the map's end. **Save Film** names a
  copy. The training map is not recorded.
- **Replay.** `--replay FILM` replays a film. A bare name is looked up in `recordings/`.
- **Saved games** (`app/save_game.rs`). A save is a film plus a snapshot. Loading replays the film
  to the save point, then restores (`ClientApp::restore_step`, `finish_restore`).
- **Quicksave** (F5 or F12) writes "Quicksave 1" through "Quicksave 5" in turn.

The film format and replay rules are in [films.md](films.md) and [session.md](session.md).

## Network play (`net_game.rs`, `app/shell_net.rs`)

- **From the menus.** The session's `NetRequest`s go to an `mandrake_net::endpoint::Endpoint`, which is a
  joiner or a host, polled once per frame.
- **Direct.** `--net-host PORT` with `--net-map`, `--net-game-type`, `--net-time-limit`,
  `--net-players`, `--net-coop` and `--net-password` hosts a game. `--net-join HOST:PORT` joins
  one.
- **Player identity.** The player's name, team and colours come from the `[net]` table of
  `settings.toml`, or `--net-name`.
- **Missing plugins.** These can be downloaded from a dedicated server (`net_game_download.rs`,
  native only), within `settings.toml`'s limits.
- **During play.** A network game pauses only by a stamped command. A dialog doesn't pause it.

The wire protocol is in [networking.md](networking.md).

## Headless runs

`--headless` runs without a window (`headless_run.rs`, `headless.rs`, `headless_render.rs`). It
ticks the sim `--ticks` times with scripted input, then renders one frame to `--screenshot`, if one
is given. The scripted input comes from these options:

- `--select-team` and `--select-unit`, which select units;
- `--band-drag`, a band selection;
- `--mouse TICK:click=...`, mouse input;
- `--hud-click`, a click on the bars;
- `--place-unit`, which moves a unit before rendering;
- `--input-film`, which plays a film's commands as the player's input.

`--film-report FILE` writes the sim's random draws and the game's end for comparison tools.
Headless runs read no user settings unless they are named on the command line, so their output is
the same on any machine.

At the end of a map, the client fades to black and prints the full carnage report (teams, players,
kills and losses) on **stdout**. Everything else goes to stderr.

## Settings and user files

| Where | What |
|---|---|
| `<config>/myth2-sourceport/` (`user_dirs::config_file`) | `controls.toml`, `interface.toml`, `fog.toml`, `lighting.toml`, `settings.toml`, `progress.txt`, `tfl/`, `downloads/` |
| `$XDG_DATA_HOME/myth2-sourceport/cutscenes-hd/` | optional HD cutscenes |
| `$XDG_CACHE_HOME/myth2/water-bed/` | enhanced-water river-bed cache |
| the pictures folder's `Screenshots` | screenshots |

`<config>` is `$XDG_CONFIG_HOME`, else `~/.config`, else `%APPDATA%` on Windows. Every key is in
[configuration.md](configuration.md).

## Module map (main files)

| Area | Files |
|---|---|
| Start | `lib.rs`, `main.rs`, `args.rs`, `startup.rs`, `map_load.rs`, `game_files.rs`, `tfl_game.rs` |
| Window, scenes, menus | `app/shell.rs`, `app/game_scene.rs`, `app/myth2_scene.rs`, `app/shell_menu.rs`, `app/shell_game.rs`, `app/shell_net.rs`, `app/window.rs` |
| One map | `app/mod.rs` (`ClientApp`), `app/tick.rs`, `app/input.rs`, `app/orders.rs`, `app/camera.rs`, `app/render.rs`, `app/gpu.rs`, `app/recording.rs`, `app/save_game.rs` |
| Input | `controls.rs`, `controls_tfl.rs`, `local_input.rs`, `selection_input.rs`, `selection_rules.rs`, `picking.rs`, `attack_click.rs`, `game_speed.rs` |
| Camera | `camera_control.rs`, `start_camera.rs` |
| What is drawn | `world_view.rs` (+ `world_view/myth2.rs`, `world_view/tfl.rs`), `world_assembly.rs`, `object_history.rs`, `unit_anim.rs`, `unit_billboards.rs`, `scenery.rs`, `scenery_billboards.rs`, `object_sprites.rs`, `projectile_local.rs`, `particles.rs`, `corpse.rs`, `effects.rs`, `model_instances.rs`, `model_effects.rs`, `shadowgen.rs`, `sprite_manager.rs`, `team_colours.rs`, `media.rs` (water drawing, not audio or video) |
| Passes and looks | `render_passes.rs`, `region_view.rs`, `fog.rs`, `lighting.rs`, `water_bed.rs`, `water_ripples_input.rs`, `ui_scale.rs`, `window_surface.rs` |
| Interface | `hud.rs`, `command_bar.rs`, `console.rs`, `minimap.rs`, `interface_setup.rs`, `interface_state.rs`, `carnage_report.rs`, `scores.rs`, `unit_info.rs` |
| Sound and cutscenes | `audio_out.rs`, `map_sound.rs`, `app/sound.rs`, `app/menu_sound.rs`, `app/cutscene.rs`, `app/cutscene_hd.rs` |
| Settings | `settings.rs`, `interface_config.rs`, `fog_config.rs`, `water_config.rs`, `config_store.rs`, `user_dirs.rs` |
| Network | `net_game.rs`, `net_game_orders.rs`, `net_game_download.rs` |
| Tools | `headless_run.rs`, `headless.rs`, `headless_render.rs`, `screenshot.rs`, `film_report.rs`, `app/frame_budget.rs` |
| The Fallen Lords | `tfl_view.rs`, `tfl_window.rs`, `tfl_live.rs`, `tfl_input.rs`, `tfl_hud.rs`, `tfl_menu.rs`, `tfl_sound.rs`, `tfl_models.rs`, `tfl_objects.rs`, and the other `tfl_*.rs` files (see [tfl.md](tfl.md)) |
| Browser | `web.rs`, `clock.rs` |
