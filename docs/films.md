# Films and saved games

This part covers the crate `mandrake-rec` (`crates/mandrake-rec`, crate name `mandrake_rec`): the Myth II film file
(`.m2rec`, also called a recording or replay), how a film is written while a game is played, how a
film is replayed through `mandrake-sim`, how the SYNC command in a film is used to detect divergence
(a "SYNC miss"), and the saved game file, which is a film with one more block. The inspection
tool `tools/m2-rec-dump` is described at the end. TFL films share the command stream but have
their own header; they live in `mandrake_rec::tfl` and are described in [tfl.md](tfl.md).

Everything in these files is **big-endian**. Strings are Mac Roman bytes
(`mandrake_rec::mac_roman`, re-exported from `mandrake_common`), NUL-terminated inside fixed fields. Decoding
a string with `mac_roman::decode` and encoding it back with `mac_roman::encode` gives the same
bytes, which the writer relies on to rewrite films byte for byte.

Related parts: [simulation.md](simulation.md) (the tick loop, the random number generator, the
determinism rules), [session.md](session.md) (Saved Films and Saved Games screens, Save Film),
[networking.md](networking.md) (the wire commands a network game records),
[server.md](server.md) (the server's films), [client.md](client.md) (the client's recording and
replay options).

## Crate layout

| Module | What it holds |
|-|-|
| `mandrake_rec` (lib.rs) | `Recording`: a parsed film, header plus commands grouped by tick |
| `mandrake_rec::header` | `RecordingHeader`, `PluginInfo`, `PlayerInfo`, `MonsterInitializer` (reading) |
| `mandrake_rec::command` | `CommandType`, `CommandPacket`, `CommandPayload` and one struct per payload; `read_command_stream` |
| `mandrake_rec::writer` | `Recorder`, `NewGameData`, `SoloNewGame`, `recording_header`, the tag header, file naming, payload encoders |
| `mandrake_rec::saved_game` | `SavedGame`, `SavedGameHeader`, `Snapshot`, `ConsoleLine` |
| `mandrake_rec::tfl` | TFL films (`TflFilmHeader`, `TflNewGame`, `tfl_recording_header`), see [tfl.md](tfl.md) |

`mandrake-rec` depends only on `mandrake-common`, `mandrake-math`, `byteorder`, `thiserror`, `tracing` and `serde`.
It never reads game data; every function works on bytes. Parse errors are
`mandrake_common::M2Error::ReplayParse` (or `InvalidMagic` / `Io`).

## File layout overview

A film file is a Bungie tag file: a 64-byte tag header, then the tag's data. The data is:

```
data + 0x000  recording header           0xa28 bytes (HEADER_SIZE)
data + ....   [saved game header block]  only in a saved game: 0xa30 bytes + snapshot pixels
data + ....   monster initializer block  0 or more 40-byte records
data + ....   [snapshot block]           an offset/size pair exists, always size 0 in practice
data + ....   command blocks             until a block with the "last" flag
```

Every block position is given by an offset word in the recording header, measured from the start
of the tag's data (not from the start of the file). Readers must use those words rather than
assume the blocks are contiguous; `RecordingHeader::read` does.

### Tag header (64 bytes)

Written by `mandrake_rec::writer::tag_header` (`TAG_HEADER_SIZE` = `0x40`).

| File offset | Size | Field |
|-|-|-|
| `0x00` | 4 | zero in files written here (byte 2 is cleared on rename) |
| `0x04` | 32 | tag name, at most 31 bytes and a NUL |
| `0x24` | 4 | group: `reco` for a film (`RECORDING_GROUP`), `save` for a saved game (`SAVE_GROUP`) |
| `0x28` | 4 | subgroup: a four-character id unique within the folder (see "Naming") |
| `0x2c` | u32 | offset of the tag's data from the file start, `0x40` |
| `0x30` | u32 | data size; 0 in files the original engine writes, set by Rename Film |
| `0x38` | u16 | tag version, 3 (`RECORDING_VERSION`) |
| `0x3b` | 1 | `0xff` |
| `0x3c` | 4 | `mth2` |

`RecordingHeader::read` also accepts two headerless forms: data that starts with the ASCII
`reco`, and data that starts directly with a version word of 1 to 3. Those exist only for
robustness; every file the game writes has the tag header.

### Recording header (0xa28 bytes)

Offsets are from the start of the tag's data. Built by `mandrake_rec::writer::recording_header`; read by
`RecordingHeader::read` (and, in full, by `mandrake_sim::players::GameSetup::from_film`,
`mandrake_sim::engine_rng::GameParameters::from_film` and `mandrake_session::films::parse_game`).

| Offset | Type | Field |
|-|-|-|
| `0x00` | u16 | version, 3. A higher version is refused |
| `0x02` | u16 | sub-version: 0 a film, 1 a saved game (`SAVED_GAME_SUB_VERSION`) |
| `0x04` | u32 | offset of the first command block |
| `0x08` | u32 | end tick: the tick counter when recording stopped. 0 while a film is being written |
| `0x0c` | u32 | saved game header block offset (= `0xa28`) |
| `0x10` | u32 | saved game header block size (0 in a film; `0xa30` + snapshot bytes in a save) |
| `0x14` | u32 | monster initializer block offset |
| `0x18` | u32 | monster initializer block size (a multiple of 40, at most 32000) |
| `0x1c` | u32 | snapshot block offset |
| `0x20` | u32 | snapshot block size (always 0; a save's pixels are counted in `0x10`) |
| `0x24` | 0x9fc | the new-game data (below) |

The block order after the header is fixed: saved game header, initializers, snapshot, commands, so
`+0x04 = 0xa28 + size(0x10) + size(0x18) + size(0x20)`.

### New-game data (0x9fc bytes at header +0x24)

This is the game's whole setup; the simulation starts from it, so it must be bit exact. Offsets
below are relative to the new-game data (add `0x24` for the header offset). `NewGameData` in
`mandrake_rec::writer` holds the raw bytes (`NEW_GAME_DATA_OFFSET`, `NEW_GAME_DATA_SIZE`).

| Offset | Type | Field |
|-|-|-|
| `0x00` | i16 | mode: 0 replay, 1 network game, 2 solo game |
| `0x02` | i16 | game type (13 is a solo mission, `mandrake_sim::engine_rng::SOLO_GAME_TYPE`) |
| `0x04` | u32 | game options bit field |
| `0x08` | u32 | time limit in ticks |
| `0x0c` | 4 | the map's `mesh` tag id |
| `0x10` | i16 | difficulty, 0 (Timid) to 4 (Legendary) |
| `0x12` | i16 | (16 in solo films) |
| `0x14` | i16 | team seed: the low half of the game seed |
| `0x16` | i16 | maximum teams / captain, -1 for none |
| `0x18` | u32 | **the game's random seed** |
| `0x1c` | u32 | planning time in ticks |
| `0x20` | u16 | flags: bit 1 set means "no saved-game commands recorded", bit 0 cleared by the writer |
| `0x22` | u16 | the build number of the program that recorded it |
| `0x24` | u32 | version id: `0x889` for 1.8.5, `0x887` for 1.8.3. Decides the gameplay version |
| `0x2a` | u16 | plugin count |
| `0x2c` | 0x200 | plugin list: u16 count again, then per plugin u32 checksum, name (at most 31 bytes, NUL), URL (at most 63 bytes, NUL). A plugin that does not fit is left out |
| `0x22c` | i16 | player count (0..16) |
| `0x22e` | i16 | local player index (-1 in a replay that is not solo) |
| `0x230` | 16 x 0x7c | player entries |
| `0x9f4` | 8 | trailing bytes, copied as they are |

A player entry (`0x7c` bytes):

| Offset | Type | Field |
|-|-|-|
| `0x00` | i16 | team |
| `0x02` | i16 | a second team word (0 for the local player in solo, 1 for the others) |
| `0x04` | u16 | flags |
| `0x08` | i32 | -1 |
| `0x0c` | i32 | player id (-1 for computer players in solo) |
| `0x1c` | u32 | gameplay version id (only the local player's is set) |
| `0x20` | u16 | build number (same) |
| `0x2c` | 64 | name (Mac Roman, NUL-terminated; the sim keeps 32 bytes) |
| `0x6c` | 2 x 4 x u16 | primary and secondary colour |

`NewGameData::solo` builds the block a solo mission writes from a `SoloNewGame` (mesh, difficulty,
seed, options, plugins): mode 2, game type 13, the three players "Light", "Dark" and "Neutral",
and the build and version id of the original 1.8.5 engine (the private constants
`BUILD_NUMBER` and `GAMEPLAY_VERSION_ID` in `mandrake_rec::writer`). A network game's block is built by
`mandrake_net` from the lobby (see [networking.md](networking.md)). `NewGameData::seed` reads `+0x18`.

When the header is written, only the first `player count` entries are copied; the rest stay zero.
`recording_header` also rewrites the flag word at `+0x20`: if any command of kind `0x17..0x19`
was recorded (`mandrake_rec::writer::is_saved_game_command`) bits 0 and 1 are cleared, otherwise bit 1
is set and bit 0 cleared.

**`RecordingHeader` fields.** `RecordingHeader::mode` is `+0x00`, `RecordingHeader::game_type`
`+0x02` and `RecordingHeader::version_id` `+0x24`; `seed` and `initial_seed` are both `+0x18`.
`PlayerInfo::team` is the entry's team word (`+0x00`) and `PlayerInfo::field_02` the second word
(`+0x02`); `RecordingHeader::teams` is just the list of distinct player names. The serde names
(`m2-rec-dump`'s JSON) are the reader's old ones: `game_type` for the mode, `difficulty` for the
game type, `build_id`, and `id`/`team` for a player's two words. Code that needs exact fields
reads the new-game data itself (`GameSetup::from_film`, `mandrake_session::films::parse_game`).

### Monster initializer block ("veterans")

Each record is 40 bytes (`MonsterInitializer::SIZE`): a named or experienced unit carried from one
game into the next (veterans in a campaign or a network game with the veterans options).

| Offset | Type | Field |
|-|-|-|
| `0x00` | u32 | run-time state, written 0 and not kept |
| `0x04` | u32 | owning player's id |
| `0x08` | 4 | unit tag (`unit_tag`) |
| `0x0c` | 4 | monster tag (`monster_tag`) |
| `0x10` | u8 | experience |
| `0x11` | u8 | `field_8f_plus_one` (a monster byte plus one; meaning not established) |
| `0x12` | u8 | collection permutation |
| `0x13` | u8 | name index |
| `0x14` | 2 | zero |
| `0x16` | 18 | custom name, NUL-terminated, usually empty |

A block whose size is not a whole number of records, is past the end of the file, or exceeds
`MonsterInitializer::MAX_BLOCK_BYTES` (32000) is a parse error. `mandrake_rec::writer::initializer_block`
writes a table back. The simulation uses the block only when the game options ask for veterans.

## The command stream

Commands are stored in blocks. Each block has an 8-byte header:

| Offset | Type | Field |
|-|-|-|
| `0x00` | u32 | flags: bit 0 set on the film's last block |
| `0x04` | i16 | size of the commands that follow, in bytes, 0 to `0x800` (`MAX_BLOCK_BYTES`) |
| `0x06` | i16 | number of commands; a negative count is read as none |

and then the commands, each with its own 8-byte header followed by its payload:

| Offset | Type | Field |
|-|-|-|
| `0x00` | i16 | size of the whole command including this header, 8 to `0x800` |
| `0x02` | u8 | command type (`CommandType`) |
| `0x03` | u8 | player index; `0xff` means "no player / system" |
| `0x04` | u32 | tick stamp: the tick body that executes it |
| `0x08` | ... | payload, `size - 8` bytes |

`mandrake_rec::command::read_command_stream` reads blocks from the header's command offset until the
flagged block. A block header that cannot be read (end of file), or a block cut short, ends the
stream quietly, as the original engine plays the commands before a truncated block. A block size
outside `0..=0x800`, or a command whose size is below 8, above `0x800`, or runs past its block,
is an error. Parsing never panics on hostile bytes (see the fuzz-style test below).

After reading, packets are sorted by tick (stable, so the order within a tick is the order in the
file, which is the order of execution) and grouped into `TickCommands { tick, commands }`. A
saved game's stream is cut at the header's end tick in `Recording::open` (`tick_commands` with a
tick greater than `end_tick` are dropped), because the original engine reads a save's film only up
to its end.

Every packet keeps its `raw_payload`. Writers use the raw bytes when present, so a film read and
written again is byte-identical even where the typed decode drops fields.

### Command types

`CommandType` covers 0..18 by name; anything else is `CommandType::Unknown(u8)` with a
`RawCommand` payload. The simulation acts on more codes than the enum names (21..27 below).
Coordinates are signed 32-bit **world units, 512 per mesh cell** (`WORLD_UNITS_PER_CELL`), x =
column, y = row, the same space as object positions. Unit lists are u16 unit identifiers.

| Code | Name | Payload (after the 8-byte header) |
|-|-|-|
| 0 | NULL | none. Never stored in a film (see recording); in a network game a NULL is a heartbeat that only advances the horizon |
| 1 | SYNC | u16 flags, u16 unused (0), u32 random seed, u32 snapshot tick. See "SYNC" |
| 2 | GENERAL | u16 unused, u16 kind (0 stop, 1 scatter, 2 retreat, 4 hold, ...), i16 count, ids. Negative count = none |
| 3 | MOVEMENT | u16 flags, i16 formation, i16 facing, u16 (a selection word, written 0 here), u16 count, ids, u16 waypoint count, waypoints (i32 x, i32 y each). With flag `0x80` (`MOVEMENT_FLAG_MATCHED`) an i32 x, i32 y formation centre follows the whole waypoint array (`movement_formation_centre`) |
| 4 | TARGET | u16 flags, u16 count, attacker ids, u16 count, target ids. Flag `0x200` (`TARGET_FLAG_MATCHED`): attacker i is paired with target i |
| 5 | ATTACK_LOCATION | u16 flags, u16 unused, i32 x, i32 y, i32 z, u16 count, ids |
| 6 | PICK_UP | u16 flags, u16 unused, i16 object index, i16 object identifier, u16 count, ids |
| 7 | RENAME | u16 unit id, u16 length, name bytes |
| 8 | ROTATION | u16 flags, i16 angle, u16 count, ids |
| 9 | DROP_PLAYER | u8 player index, u8 reason |
| 10 | CHAT | u16 recipient, u16 length, text bytes |
| 11 | DETACH | u16 flags, u16 count, ids |
| 12 | UNIT_ADJUSTMENT | u16 unit id, u16 adjustment type, i32 value (the pre-game unit trade) |
| 13 | SKETCH | u16 colour, u16 count, points (i16 x, i16 y) |
| 14 | ALLY | u16, 2 bytes, i16 team, i16 other team: toggles whether `team` treats `other_team` as an ally |
| 15 | PRESET_SELECTION | u16 preset index, u16 count, ids |
| 16 | SYNC_FORMATION | a list of i16 formation parameters (the player's formation definition) |
| 17 | INVENTORY | u16 action, u16 count, ids |
| 18 | INVENTORY_DROP | u16 unit id, u16 slot |
| 21 | forced end | i16 kind: 2 rehost, 3 restart (`mandrake_sim::net_game::FORCED_ENDGAME`); network games only |
| 22 | ADD_PLAYER | a player entry; queued by a saved game's restore |
| 23 | pause | i16 paused, i16 0 (`mandrake_sim::net_game::PAUSE`) |
| 24 | game speed | i16 index, i16 0 (`mandrake_sim::net_game::GAME_SPEED`) |
| 25 | SAVE_LOADED | the saved observer position and zoom (`mandrake_sim::save_restore::SAVE_LOADED`) |
| 26 | REPLACE_PLAYER | `mandrake_sim::save_restore::REPLACE_PLAYER` |
| 27 | SET_TEAM_CAPTAIN | `mandrake_sim::save_restore::SET_TEAM_CAPTAIN` |

Decoding rules worth knowing (all in `CommandPacket::parse`):

- A payload shorter than its kind's fixed part decodes as `CommandPayload::Unknown` with the
  bytes kept; it does not fail the film.
- Lists stop early when the bytes run out; they are not errors.
- MOVEMENT keeps at most `MAX_WAYPOINTS` (4) waypoints, as the original engine does, but the
  formation centre is found after the whole array whatever its count.
- `Waypoint::facing_x` / `facing_y` are not on disk. They are always 0.
- In 1.8.5 network films, MOVEMENT and TARGET lists made by the issuing machine's pairing keep
  `-1` (0xffff) entries in their slots; the executor reads the matched flags only for gameplay
  versions after 1.8.3.

Which commands do what in the simulation is the job of `mandrake_sim::Simulation`'s
`execute_monster_commands` stage (see [simulation.md](simulation.md)): CHAT, SKETCH and
PRESET_SELECTION do nothing in the sim (they are the client's), pause and game speed never reach
it outside a network game, and a command type the sim has no handler for stops the engine model
(`Simulation::engine_stop`).

## SYNC

A SYNC does **not** carry a state hash. Its
two words are a snapshot `{seed, tick}`:

- At the top of every tick body whose tick is a nonzero multiple of 512
  (`mandrake_sim::SYNC_SNAPSHOT_TICKS`), the sim stores `Simulation::sync_snapshot = (rng state, tick)`.
  At the start of a map it is `(game seed, 0)`.
- A live game queues a SYNC of the current snapshot every 171 ticks (`mandrake_sim::SYNC_TIMER_RESET` is
  170; the countdown fires when it goes below zero, and the first SYNC is stamped tick 171). Only a
  solo game (mode 2) or the network originator queues them.
- `random_seed` is the snapshot's random number generator state; `snapshot_tick` is the snapshot's
  tick (0, 512, 1024, ...). `mandrake_rec::writer::sync_command(seed, snapshot_tick)` builds one.

When a SYNC executes (in the tick body of its stamp), the sim compares it with its own held
snapshot (`mandrake_sim::net_game` `sync_differs`): if the SYNC names the same snapshot tick and a
different seed, the game is out of sync. A SYNC naming an older snapshot than the one held is
not compared. The first mismatch calls `Simulation::game_is_out_of_sync`, which:

- logs an error line `OUT OF SYNC: OOS occurred at line ... - game time: T random seed: S`,
- pushes an `mandrake_sim::net_game::OutOfSync { file, line, tick, seed }` onto
  `Simulation::out_of_sync`,
- sets `mandrake_sim::game_state::FLAG_OUT_OF_SYNC` in the game flags.

Only the first report is made (`Simulation::is_out_of_sync`). The same mechanism detects a
divergence between network peers and between a film and our replay of it.

A **SYNC miss** is the first snapshot tick at which a film's SYNC seed disagrees with our replay.
The film's SYNCs are the original game's random number generator state at those ticks, so they
are the ground truth a replay is checked against. Because the original engine draws random
numbers for nearly everything that happens in the world, a wrong draw count anywhere shows up at
the next snapshot. The replay can still run past a miss; everything after it is no longer the
recorded game.

Other out-of-sync reports the sim makes on the same path: a command executed in a later tick body
than its stamp (`Simulation::late_commands`), and in a network game a tick body at or past the
horizon.

## Recording a game

### The writer

`mandrake_rec::writer::Recorder` builds the tag's data exactly as the original engine does, so that the
original game can play our films and vice versa:

1. `Recorder::new(game, initializers)`: the recording header with end tick 0, then the monster
   initializer block of the table the map load found.
2. `Recorder::record(bytes)` / `Recorder::record_packet(packet)`: each command, with its 8-byte
   header (`CommandPacket::to_bytes`), is appended to a `0x800`-byte buffer. If it would not fit,
   the buffer is flushed first as a block with flag 0. A command of type 0 (NULL) is not stored
   and not counted. A command of kind `0x17..0x19` marks the film as having saved-game commands.
3. `Recorder::finish(end_tick)`: the buffer as the last block with flag 1 (even if empty), then the
   header written again with the end tick. Returns the tag's data.

`mandrake_rec::writer::film_file(name, subgroup, data)` and `save_file` prefix the tag header.
`Recorder::is_empty` tells whether anything but the header and initializers was recorded.

`CommandPayload::encode` writes the payloads the local player creates (SYNC, GENERAL, MOVEMENT,
TARGET, ATTACK_LOCATION, SYNC_FORMATION, and `Unknown` raw bytes); it returns `None` for the
others, which only come from a film or the network and are written from `raw_payload`.

### Which commands are recorded

The film holds every command the game queues as incoming, in that order, stamped with its tick:

- In a solo game, the sim stamps the local player's commands (tick = `current_tick +
  command_lead`, `mandrake_sim::LOCAL_COMMAND_LEAD` = 1 in a live game) and, when
  `Simulation::recorded_commands` is `Some`, pushes each accepted packet onto it. This includes the
  SYNCs and the first body's SYNC_FORMATION the sim generates itself.
- In a network game, every stamped command received from the host except NULL is recorded
  (`Simulation::queue_incoming_command`).
- `Simulation::end_recording` (sets `recording_ended`) stops recording when the map is disposed.
- The commands a saved game's restore queues (ADD_PLAYER, REPLACE_PLAYER, SET_TEAM_CAPTAIN,
  SAVE_LOADED) are not recorded.

The training map (`00tm`) is never recorded, and a film being replayed is not recorded.

### Who writes films

- **Client, solo game** (`crates/mandrake-client/src/app/recording.rs`): `LiveRecording` holds the
  new-game data, the initializers and, for a restored saved game, the save's commands up to its end
  (`before`), which are copied into the new recording ahead of the game's own. The film is
  written whole to `recordings/<last recording name>` (the `inrf` string list's string 0, by
  default "Last Recording", no extension) at the map's end, when the window closes during a map,
  and every `CHECKPOINT_TICKS` (900 ticks, 30 s) while playing, each time over the last. The
  original engine instead writes a block every `0x800` bytes; writing whole files means a crash
  loses at most 30 s. The subgroup is chosen randomly once per file
  (`mandrake_session::films::last_recording_subgroup`, `mandrake_rec::writer::random_subgroup`).
  `mandrake_session::films::write_recording` writes to a hidden `.<name>.writing` file and renames it
  over the old one, so a crash never leaves half a film; the Saved Films list skips such files.
- **Client, network game** (`crates/mandrake-client/src/net_game.rs`, `recording::network_film`): the
  new-game data `mandrake_net` built (with our local player), every stamped command, no initializers.
- **Dedicated server** (`crates/mandrake-server/src/game.rs`, `write_film`): the same, with the veterans
  table the load found, into the configured films folder as `<n>-<unix seconds>.m2rec`, tag name
  "Server Game n".
- **`mandrake_net::client::film`**: the film a headless network peer writes.

### Naming

- `mandrake_rec::writer::sanitize_name(name, max)`: the characters `\ / | " < > * ? :` and a leading `.`
  become `_`, the name is cut at `max`, and trailing spaces and dots go. Returns whether anything
  changed; the name prompts disable OK when it did.
- `film_names(name)` gives (tag name of at most 31 bytes, file name with `FILM_EXTENSION`
  `.m2rec` appended unless present, changed?). `save_names` is the same without an extension.
- `subgroup_for_name(tag_name, taken)`: the subgroup a named tag gets. Lower case, split into words
  at ` \t\n()[]{}?!:;-`, up to four letters of the first word, then the first letters of the next
  words from the third place on, padded with spaces; while a tag in the folder has it, the next
  value as a big-endian number. `random_subgroup` is four random printable characters.

Save Film (from the epilogue or Saved Films) copies the last recording's data under a new tag
header (`mandrake_session::films::save_last_film`); only the 64 bytes of tag header differ.

## Replaying a film

The replay driver is `mandrake_sim::Simulation`:

```
bytes --Recording::from_bytes--> Recording
      --Simulation::from_recording / from_film--> Simulation (commands_by_tick, rng seeded)
      --set_up_map(MapSource, SetupRole)--> map loaded and populated
loop: step_with_commands()  // one tick body: stamps, SYNC snapshot, execute_monster_commands(t), the rest
```

- `Simulation::from_recording(&Recording)` puts every `TickCommands` into `commands_by_tick`,
  seeds the random number generator from `header.seed` (`mandrake_sim::players::set_random_seed`), sets
  `sync_snapshot = (seed, 0)`, and builds an `mandrake_sim::engine_rng::ReplayGame` from the mode and
  options: the replay runs in mode 0; it is "solo" if the recorded mode was 2 or option bit `0x80`
  is set; a replay that is not solo clears option 4 and has no local player.
- `Simulation::from_film(&[u8])` does the same and also reads the full player entries
  (`GameSetup::from_film`), which the map load needs. Prefer it when you have the file bytes.
- `Simulation::replay_film(&Recording, GameSetup, &MapSource)` is the headless form: the sim plus
  `set_up_map` in `SetupRole::Headless` order. `mandrake_web::film_replay::replay` is a complete,
  data-agnostic example (it takes the tag archives as bytes).
- Each `step_with_commands` runs one tick body `t = current_tick`: the command stage
  (`run_command_stage`: object history, local stamps, the snapshot if `t` is a nonzero multiple of
  512, then every queued command with tick `<= t` in tick order) and `finish_tick_body` (the rest
  of the world). The film's commands are thus executed in the body of their stamp, as in the
  original engine.
- While a replay runs, the gameplay version is clamped as the original engine does.
- `Simulation::live_game_of_film` and `give_film_commands` replay a film's commands as if a player
  typed them into a live game (its SYNCs and SYNC_FORMATION are left out, because a live game makes
  its own): a test aid for the live path.

### End of a film

A Myth II film plays until the sim's game ends (`Simulation::game_end` returns the end tick,
winner and code), and the client then fades to black, shows the film's report and returns to the
menu (see [session.md](session.md)). The replay also ends early if the engine model stops
(`Simulation::engine_stop`, or `ticks_stopped`). The header's end tick is the tick counter when
the recording stopped, normally some 300 ticks after the game's end; a film replay does not use
it. A live game records SYNCs during those last ticks, which a replay never reaches, so SYNCs past
the replay's last tick are not comparable.

### Saved game restore

A saved game is replayed the same way, at full speed and muted, until `current_tick` reaches the
header's end tick. Then `Simulation::end_saved_game_replay(observer)` turns the game into a live
solo game (mode 2), queues the restore's player commands and SAVE_LOADED
(`mandrake_sim::save_restore::save_loaded_command`) for the next tick, and the client sets
`command_lead` to `LOCAL_COMMAND_LEAD` and `halt_on_stop`. Play continues once SAVE_LOADED has run
(`Simulation::save_loaded`). The client side is `crates/mandrake-client/src/app/tick.rs`
(`restore_step`, `finish_restore`).

## Saved games

A saved game (`mandrake_rec::saved_game`) is a tag of group `save` in the `saved games` folder whose
data is a film of the game so far, with sub-version 1 and one extra block between the header and
the initializers. `Recorder::saved_game(end_tick, header, snapshot)` makes it from the recording in
progress: header (sub-version 1, end tick = the current tick, block sizes), saved game header
block, snapshot pixels, the recording's initializers and flushed blocks, then what is still
buffered as a last block. The recording itself goes on.

The saved game header block (`SAVED_GAME_HEADER_SIZE` = `0xa30` bytes, at the header's word
`+0x0c`):

| Offset | Type | Field |
|-|-|-|
| `0x00` | i16, i16 | snapshot width, height (160 x 120: `SNAPSHOT_WIDTH`, `SNAPSHOT_HEIGHT`) |
| `0x04` | i32, i32 | observer x, y (world units) |
| `0x0c` | i16 | observer facing |
| `0x0e` | 2 | zero |
| `0x10` | i16 | snapshot pixel format as stored (2 for a 32-bit snapshot, `SNAPSHOT_FORMAT_32_BIT`) |
| `0x14` | i32 | observer zoom |
| `0x18` | 10 x 258 | console lines (`CONSOLE_LINES`): i16 kind (-1), 256 text bytes NUL-terminated |
| `0xa2c` | i16 | number of console lines in use |

The pixels follow: width x height, 4 bytes each (`0xAARRGGBB`) when the stored format plus one is
3 or more, else 2 bytes (5-5-5 RGB). `SavedGameHeader::pixel_format` and `bytes_per_pixel` apply
that rule; `Snapshot::to_bytes` writes 32-bit pixels, opaque. `SavedGame::from_bytes(file)` reads
the recording header, the block and the `Snapshot` (RGBA rows, top to bottom), with `snapshot`
`None` where the pixels cannot be read.

How saves are listed, named (autosave, quicksave) and loaded is in [session.md](session.md).

## TFL films

A TFL film is a `reco` tag with TFL's own 64-byte header and a `0x724`-byte film header (no
plugin block), followed by command blocks in exactly the Myth II format above
(`read_command_stream` is shared). See `mandrake_rec::tfl` and [tfl.md](tfl.md).

## Inspecting a film: m2-rec-dump

`tools/m2-rec-dump` prints a film as JSON on stdout (logging goes to stderr):

```
cargo run -p m2-rec-dump -- path/to/film.m2rec > film.json
```

The JSON has `metadata` (name, mesh, seed, plugin names, the player names as `teams`, player
count, and the header's `game_type`, `difficulty`, `game_options_flags`, `time_limit` under the old
names given above), `command_summary` (count per command name) and `events_by_tick` (every
`TickCommands`, each packet with its decoded payload and raw bytes). It uses only
`Recording::open` and serde, so it is also a quick way to check that a film you wrote parses.

## Determinism rules for this crate

- Never change the byte layout the writer produces: the original game must be able to play our
  films, and network peers must record the same film. Tests check byte-for-byte rewrites.
- Never reorder commands within a tick: execution order within a tick is file order.
- Keep `raw_payload` round-tripping; do not regenerate payloads from the typed structs when the raw
  bytes are there.
- The seed at `+0x18` and the full new-game data decide the whole game. Anything that changes
  how they are read changes every replay.

## Verifying changes

Self-contained tests (no game data) in `mandrake-rec`:

```
cargo test -p mandrake-rec
```

Among them: `the_block_before_the_commands_is_monster_initializers`,
`a_malformed_film_is_an_error` (every bad size and offset is an error), `damaged_films_never_panic`
(6000 rounds of random payloads and random damage), `block_count_and_end_flag_read_as_the_engine_does`,
`test_command_type_roundtrip`, `test_synthetic_command_parsing`, the payload layout tests in
`command.rs` (ALLY, PICK_UP, GENERAL, the formation centre),
`a_block_is_flushed_before_a_command_that_wont_fit` and `names_and_subgroups_are_the_engines` in
`writer.rs`, `the_header_block_reads_its_fields` in `saved_game.rs`, and the TFL header tests that
build their headers in code (`a_two_player_network_header_reads_back`,
`a_player_record_round_trips` and the others in `tests/tfl_header.rs` that read no film file).

To check that a change to replay keeps films in sync, with your own game data installed and a
1.8.5 film:

- `cargo run --release -p mandrake-web --example film_bench -- FILM.m2rec [GAME_DIR]` (GAME_DIR defaults
  to `MYTH2_DATA_DIR`) replays the film headless and prints one summary line; `first_sync_miss=none`
  means every comparable SYNC matched, otherwise it names the first snapshot tick that missed.
  `end=` says whether the game reached its end (`end_game in tick T`) or the model stopped. Run it
  before and after your change: the draw count and `draws_sha256` should not change either unless
  you meant them to.
- In the client, `--replay FILM` plays the film in a window; an `OUT OF SYNC` error line in the log
  means a SYNC missed (see [client.md](client.md) for the options).
- A film you record and save can be dumped with `m2-rec-dump` and replayed the same way; a live
  game's own SYNCs then check our live path against our replay path.
