# Myth: The Fallen Lords support

The workspace plays two games: Myth II: Soulblighter 1.8.5 and Myth: The Fallen Lords (TFL),
whose reference is the original TFL 1.5 engine running on the retail data. This part covers
everything TFL-specific: the separate deterministic simulation crate `mandrake-tfl-sim`, the TFL
readers in `mandrake-tag`, `mandrake-rec`, `mandrake-net` and `mandrake-session`, and the client's game switch, which
picks one game at startup and sends a TFL run down its own path. That path still uses the
shared shell, dialogs, input, settings and renderer.

Related parts: [simulation.md](simulation.md) (Myth II's sim, which this one mirrors),
[data-formats.md](data-formats.md) (Myth II's tag formats that TFL's readers reuse),
[films.md](films.md) (the command stream both games' films share),
[networking.md](networking.md) (the wire TFL games ride on) and [client.md](client.md)
(the shell, rendering and input that TFL reuses).

## Ground rules

- **Two sims, never mixed.** `mandrake-sim` is Myth II 1.8.5 only and never names TFL. `mandrake-tfl-sim`
  holds TFL's rules and does not depend on `mandrake-sim`. Its dependencies are `mandrake-common`,
  `mandrake-math`, `mandrake-tag` and `mandrake-rec`. No sim function checks "which game am I?". The game is
  chosen once, at startup, in the client.
- **Readers chosen by data format, not by flag.** TFL's tags live in `mandrake_tag::tfl`, next to
  Myth II's readers. A file's format decides which reader runs: a `.gor` archive versus a
  monolith, mesh version 7 versus 11, a 512-byte `mons` versus a 1024-byte one. A TFL file never
  goes through a Myth II reader with a "TFL mode" switch.
- **Port exactly, or stop.** `mandrake-tfl-sim` was ported function by function from the original TFL
  1.5 engine and checked draw by draw against it. Where the port reaches behaviour it does not
  model, it returns `mandrake_tfl_sim::Stop` instead of guessing (see [Stops](#stops)).
- **Big-endian everywhere.** TFL's tags and films are Mac-era big-endian. The sim keeps each
  definition as raw big-endian bytes (`mandrake_tfl_sim::data::Def`) and reads words at the offsets
  the engine reads in memory.

## The data directory

The code expects a TFL 1.5 install: the retail data (the 1.2 CD's `.gor` archives) plus the
1.5 update's loose overrides. The layout:

```text
<tfl dir>/
  Myth_TFL.exe            only checked for by the tests and tfl_film_bench (see below)
  tags/
    TAGS.GOR  ARTSOUND.GOR  CUTSCENE.GOR  PRIVATE.GOR    archives that hold their own data
    LOCAL.GOR                                            index of tags/Local (not opened)
    SCRAP.GOR                                            scratch file (skipped)
    Local/<group>/<file>                                 loose tags; the 1.5 overrides
    Local/PREF/GAME                                      TFL's preferences (optional)
    Local/RECO/*                                         TFL's saved films
```

- `mandrake_tag::tfl::Game::detect` decides the game from `tags/` alone. A Myth II monolith there
  (`mth2`/`dng2` signature) means Myth II. Otherwise any `.gor` with a valid directory means TFL.
  It reads headers only.
- `mandrake_tag::tag_files::tfl_tag_files` lists the archives in search order. It takes every `.gor`
  that holds its own tags' data (so not `LOCAL.GOR` or `SCRAP.GOR`), sorted by file name
  ignoring case: `ARTSOUND`, `CUTSCENE`, `PRIVATE`, `TAGS`. A later file overrides an earlier
  one, so `TAGS.GOR` wins over `ARTSOUND.GOR`. The two share 31 interface tags, 5 of which
  differ, and the original engine's order between those two archives is inferred, not proven.
- `mandrake_tag::tag_files::open_tfl_tags` opens that list as one `mandrake_tag::TagArchive`
  (`TagArchive::with_overrides`) and puts `tags/Local`'s loose tags (`mandrake_tag::tfl::loose_archive`)
  on top of them all.
- The install is treated as **read-only**. The client writes its own TFL films to
  `<config dir>/tfl/RECO` (`our_films_dir` in `tfl_live.rs`), and Saved Films lists those before
  the install's. Campaign progress goes to `mandrake_session::tfl::TFL_PROGRESS_FILE` in the config
  folder. On a first run it is imported from TFL's own preferences (`mandrake_session::tfl::TflPrefs`).
- Where the data is found:
  - **Client:** `--tfl-data <dir>`, falling back to a built-in default install path.
  - **Tests and the bench example:** `TFL_DATA_DIR`. They also require
    `<dir>/Myth_TFL.exe` to exist before they treat the folder as an install.

## TFL data formats (`mandrake_tag::tfl`)

Each reader checks the tag version and that every table lies where its words say it does. It
fails with an error otherwise; it never reads a layout it doesn't know. Fields whose meaning
isn't established are kept raw. Look for names like `word_2`, `byte_5`, `field_02` and `raw`.

### `.gor` archives (`mandrake_tag::tfl::gor`)

Archive header, at the start of the file (`mandrake_tag::tfl::GorHeader`, 0x40 bytes of fields; the
first tag's data starts at 0x80):

| offset | type | field |
|---|---|---|
| `0x00` | `i16` | type (1) |
| `0x02` | `u16` | version (1) |
| `0x04` | `[u8; 32]` | name (`public.gor`, `local.gor`, ...) |
| `0x24` | `u32` | checksum |
| `0x28` | `u32` | directory offset |
| `0x2c` | `u16` | tag count |
| `0x2e` | `u16` | directory entry size (64) |
| `0x30` | `u16` | flags (meaning unknown) |

The directory sits at the **end** of the file: `count` entries of 64 bytes that tile exactly to
the end. That tiling is how a `.gor` is told from a Myth II monolith
(`GorHeader::matches_length`). `LOCAL.GOR` is only an index: its entries point at offset 0x40
of the loose files, so `GorHeader::holds_its_data` is false for it.

Each directory entry, and the start of each loose tag file, is the 64-byte `myth` tag header
(`mandrake_tag::tfl::GorTagHeader`):

| offset | type | field |
|---|---|---|
| `0x00` | `[u8; 32]` | name |
| `0x20` | FourCC | group (`mesh`, `.256`, ...) |
| `0x24` | FourCC | id |
| `0x28` | `u16` | 0 |
| `0x2a` | `u16` | version (mesh 7, `.256` 4, `scen` 2, ...) |
| `0x2c` | `u16` | unknown |
| `0x2e` | `u16` | flags (unknown) |
| `0x30` | `u32` | data offset (from the file's start; 0x40 for a loose tag) |
| `0x34` | `u32` | data size |
| `0x38` | `u32` | user data |
| `0x3c` | FourCC | signature `myth` |

`mandrake_tag::TagArchive` opens a `.gor` the same way it opens a monolith
(`mandrake_tag::archive::ArchiveFormat::TflGor`; loose tags are `ArchiveFormat::TflLoose`), so every
lookup (`find_tag_by_id`, `get_tag_data`) works for either game.

### Tag definitions (`mandrake_tag::tfl::defs`)

`mandrake_tag::tfl::TFL_TAG_DEFINITIONS` is the original engine's own table of tag groups: version,
the most definitions of a group loaded at once, and the in-memory size, which for a
variable-length tag is its fixed header. Some entries that differ from Myth II's: `mons` is 512
bytes (Myth II's is 1024), `obje` 16 (Myth II's 64), `geom` 96 (Myth II's 128) and `mesh`
version 7. The groups `bina`, `meli`, `temp`, `moov`, `reco`, `pref` and `maps` have no entry.
`mandrake_tag::tfl::tfl_tag_definition` looks one up.

`mandrake_tag::tfl::TFL_REFERENCE_FIELDS` lists `(group, offset, referenced group)` for the words that
hold a tag reference (or -1) in every installed tag. Some of them: `unit +0x00` → `mons`,
`unit +0x04` → `core`, `scen +0x08` → `core`, `scen +0x18` → `obje`, `mons +0x90` → `obje`,
`proj +0x1c` → `obje`.

`mandrake_tag::tfl::TflDefinition` wraps a fixed-size definition, checked against the table, with
word accessors. `mandrake_tag::tfl::TflObjectDefinition` is the 16-byte `obje` as eight raw words.
Its layout differs from Myth II's and is unnamed.

### Mesh, version 7 (`mandrake_tag::tfl::mesh`)

`mandrake_tag::tfl::TflMesh::from_bytes` reads a 1024-byte header that keeps Myth II's words where the
reader uses them:

| header offset | field |
|---|---|
| `+0x00` | landscape `.256` |
| `+0x04` | media |
| `+0x08`, `+0x0a` | submeshes across, down |
| `+0x0c`, `+0x10` | cell block offset (from 1024) and size |
| `+0x24..+0x30` | marker palette: count, offset, size (32-byte entries) |
| `+0x34..+0x40` | markers: count, offset, size (64-byte entries, Myth II's layout) |
| `+0x4c` | level flags (`mandrake_session::tfl::LEVEL_SOLO` 0x10000, `LEVEL_HIDDEN` 0x800000) |
| `+0x7c` | ambient sound |
| `+0x80..+0x8c` | map actions: count, offset, size |
| `+0x8c` | string list (string 0 is the level's name) |
| `+0x90`, `+0x94` | post-game art, pre-game pictures |
| `+0x98` | overhead map collection |
| `+0x9c`, `+0xa0` | next level, alternative next level |
| `+0xa4`, `+0xa8`, `+0xac` | the level's movies: pregame, win, loss |
| `+0xb0`, `+0xfc`, `+0x100`, `+0x108` | narration `text`, caption `stli`, narration music, epilogue sound |

(The menu-side fields are read by `mandrake_session::tfl::TflLevelHeader::from_mesh`.)

A submesh is **16** cells square (`mandrake_tag::tfl::mesh::TFL_SUBMESH_CELLS`; Myth II's is 32), and
a cell is **8** bytes (`mandrake_tag::tfl::TflCell`):

| offset | type | field |
|---|---|---|
| `+0` | `i16` | vertex height (1/512 world units) |
| `+2` | `u16` | unnamed (varies like Myth II's packed normal) |
| `+4` | `u8` | flags: 1 water, 4 moves with the media's waves, 0x10/0x18 media-related |
| `+5` | `u8` | unnamed; the sim marks cells under models here |
| `+6` | `i16` | first object (-1 in every installed tag) |

A world cell is 0x400 units, so a position's cell is `x >> 10`. Myth II's mesh is versioned
separately. `mandrake_tag::mesh::GameMesh::from_tag` picks the reader by the tag's version (11 Myth
II, 7 TFL).

### Models (`mandrake_tag::tfl::model`)

- `geom` version 1 (`mandrake_tag::tfl::tfl_geometry`, a 96-byte header
  `mandrake_tag::tfl::model::TFL_GEOMETRY_HEADER_SIZE`). Header words:
  - flags `+0x00`, the `core` `+0x04`
  - material, vertex, surface and draw-after counts `+0x08..+0x0e`
  - the centre `+0x10`, which the model turns about
  - tables (offset, size, 0) of materials `+0x1c`, vertices `+0x28`, surfaces `+0x34` and
    draw-after words `+0x40`, each offset from the header's end
  - the data block's offset (0x60) and size `+0x4c`/`+0x50`

  The records are Myth II's (64-byte materials, 6-byte vertices, 64-byte surfaces). The reader
  returns Myth II's `mandrake_tag::model::GeometryTag`, with TFL's centre as the origin and zero
  bounds.
- `mode` version 4 (`mandrake_tag::tfl::tfl_model`): Myth II's 64-byte header and tables.

One difference matters to renderers: TFL places a vertex at **place + geometry centre + the
rotated offset from the centre**, while Myth II adds no centre (`tfl_models.rs` in the client).

### Monsters (`mandrake_tag::tfl::monster`)

`mandrake_tag::tfl::TflMonsterDefinition` is exactly 512 bytes. Any other size is an error, so a
Myth II `mons` (1024) fails. Fields named so far:

| offset | field |
|---|---|
| `+0x00` | flags (`u32`) |
| `+0x04` | `.256` collection |
| `+0x08 + 2k` | 16 sequence slots, -1 for none (slot 0 standing) |
| `+0x48..+0x58` | 16 signed terrain speeds; a negative byte closes that terrain type (the sim's `Def::closed_terrain`) |
| `+0x90` | `obje` |
| `+0x94` | attack count (0..=3) |
| `+0x9c + 0x20k` | each attack record's projectile |
| `+0x14c + 4k` | 10 `soun` slots |

The rest stays in `raw`. The sim reads further words by offset. When the sim loads a monster
definition (`TflData::def`), it gives every sequence the collision bounds of the standing
sequence, as the original engine does, so a monster's radius and height stay its standing
ones.

### Menus and interface tags (`mandrake_tag::tfl::menus`)

These readers check the versions the installed tags carry and return Myth II's types (dialogs,
fonts, interface, sounds), with TFL's differences:

- `stli` (`mandrake_tag::tfl::TflStringList`) splits on NUL, LF and CR and **skips empty lines**.
- `text` (`mandrake_tag::tfl::tfl_text`) is one string, not a list of lines.
- A `ditl` (`mandrake_tag::tfl::tfl_ditl`) keeps every line, including the empty one after the last
  CR. Myth II's `parse_ditl` drops that line.
- `inte` (`mandrake_tag::tfl::TflInterfaceDefinition`) is 256 bytes with eight font slots.
- A dialog control's kind must be one the engine registers
  (`mandrake_tag::tfl::menus::TFL_CONTROL_KINDS`, 33 kinds).

`mandrake_tag::tfl::TFL_SCREENS` and `mandrake_tag::tfl::TFL_MAIN_MENU` name the tags each screen loads.

### Colour tables (`mandrake_tag::tfl::shading`)

- `mandrake_tag::tfl::landscape_shading_table`: the landscape's colour table, untinted, at
  `mandrake_tag::tfl::TFL_SHADING_LEVELS` (256) levels from black. Channel `c` at level `l` is
  `(c * l + (levels - 1) / 2) / (levels - 1)`. TFL's mesh has no shading colours.
- `mandrake_tag::tfl::tfl_team_colors`: for each hue change, the primary colour when its flags have
  bit 0 and the primary is on, else the secondary when the flags have bit 1 and the secondary is
  on, else the base. A colour is off when its flags word has bit 2. Myth II's rule
  (`mandrake_tag::collection::CollectionTag::team_colors`) also requires the base to be on.

## TFL films (`mandrake_rec::tfl`)

A TFL film is a `reco` tag file with the following parts:

1. The 64-byte `myth` tag header (`TflFilmHeader::TAG_HEADER_SIZE`): name first, group `reco`
   at `0x20`, signature `myth` at `0x3c`, data offset at `0x30`.
2. A film header of 0x724 bytes (`mandrake_rec::tfl::TflFilmHeader::SIZE`).
3. Up to three blocks.
4. The command stream, in Myth II's command-block format (read by `mandrake_rec`'s
   `read_command_stream`; see [films.md](films.md)).

`TflFilmHeader::is_tfl_film` tests the first 64 bytes. A Myth II film fails that test, and
Myth II's `mandrake_rec::Recording` refuses a TFL film.

Film header, big-endian, offsets from the start of the tag's data:

| offset | field |
|---|---|
| `0x00` | `u16` version (must be 3), `u16` sub-version |
| `0x04` | where the commands start (0x724 + the blocks) |
| `0x08` | end tick |
| `0x0c..0x24` | three blocks, each an offset and a size: a saved game's (0), the monster initializers, a snapshot (0) |
| `0x24` | new-game parameters, 0x28 bytes (`mandrake_rec::tfl::TflGameParameters`, below) |
| `0x4c` | player count; `0x4e` the local player's index |
| `0x50` | 16 player slots of 0x6c bytes (`mandrake_rec::tfl::TflFilmPlayer`) |
| `0x71c` | tag-archive checksum; `0x720` the active external archive's checksum |

`TflGameParameters` (0x28 bytes, at header `+0x24`):

| offset | field |
|---|---|
| `+0x00` | mode: 0 replay, 1 network, 2 solo |
| `+0x02` | game type (string list index; 11 None) |
| `+0x04` | option flags (`u32`) |
| `+0x08` | time limit in ticks |
| `+0x0c` | map's `mesh` id |
| `+0x10` | difficulty (0 Timid .. 4 Legendary) |
| `+0x12` | max players |
| `+0x14` | unnamed (the seed's low half in solo films) |
| `+0x16` | largest team, -1 when teams aren't allowed |
| `+0x18` | game seed |
| `+0x1c` | planning time |
| `+0x20..+0x28` | never written into a film (0) |

A player slot (`TflFilmPlayer`, 0x6c bytes) holds the following:

| offset | field |
|---|---|
| `+0x00` | team |
| `+0x02` | kind: 0 a person, 1 the computer |
| `+0x04` | flags |
| `+0x06` | captain's net id (-1 none) |
| `+0x08` | net id |
| `+0x0c` | metaserver id |
| `+0x18`, `+0x1a` | appearance words |
| `+0x1c` | name (32 bytes, Mac Roman) |
| `+0x3c` | team name (32 bytes, Mac Roman) |
| `+0x5c`, `+0x64` | primary and secondary colours `[r, g, b, flags]` |

A solo film has three players: the person (Light) and the computer's Dark and Neutral teams.

The monster-initializer block holds 0x28-byte records (`mandrake_tfl_sim::MonsterInitializer`): the
campaign's veterans carried from the last level. The sim reads them
(`mandrake_tfl_sim::film_monster_initializers`) only when option bit 0x40 is set.

Writing (`tfl_write.rs`, re-exported from `mandrake_rec::tfl`):

- `mandrake_rec::tfl::tfl_tag_header` writes the tag header.
- `mandrake_rec::tfl::tfl_recording_header` writes the film header from a `mandrake_rec::tfl::TflNewGame`
  (parameters, local player, players).
- `TflFilmHeader::to_bytes` re-emits a parsed header byte for byte.
- `TflNewGame::network` builds a network game's new-game data from the players' 0x7c-byte
  records (`mandrake_rec::tfl::TflNetPlayer`). It sorts them by appearance word, then by net id, and
  groups each captain with the players that name it. Co-op (`mandrake_rec::tfl::TFL_OPTION_COOPERATIVE`,
  option 0x80) sets no teams and no time limit.

The client builds its live solo films in `tfl_live_film.rs` from the same parameters the
original engine uses for a solo level.

## `mandrake-tfl-sim`

### Entry points

- `mandrake_tfl_sim::TflSim::new(game_dir, film)` sets a map up from a TFL film's bytes.
  `TflSim::set_up` does the same with the film's monster initializers (a campaign film), and
  hands back the half-built sim along with the stop if set-up stops. `TflSim::set_up_as` also
  sets the game mode (`TflSim::game_mode`: 0 replay, 1 network, 2 solo); a film read from disk
  is mode 0.
- `TflSim::step()` runs one tick and returns `Result<(), Stop>`. `TflSim::tick` counts the ticks
  done.
- **There is no set-up without a film.** A live game is set up from a film that has a header
  and no commands. The player's orders are pushed as film commands into
  `sim.data.film.tick_commands` for the tick they run at, and are recorded into the same film.
  This is how the client's live solo game works (`TflReplay::queue` in `tfl_game.rs`).
- After each tick, the client drains the following event queues: `take_sound_events`,
  `take_stains`, `take_marks`, `take_local_group_starts`, `take_local_projectile_group_orders`
  and `take_interest_events`. These carry presentation only. Draining them, or not, changes
  nothing in the sim.
- State the client reads directly: `objects`, `monsters`, `projectiles`, `lightning`,
  `model_effects`, `sight()` (the local team's visibility and explored maps, refreshed each
  64-tick window), `statistics`, `winning_team`, `map_actions.game` (the end of the game).
- State the client gives each tick: `overhead_map_state` (the tutorial's sniffing reads it) and
  `camera_control_flags` (the client clears them when its camera move ends). Neither is in a
  film, and neither draws random numbers.

Capacity limits follow the original engine: `MAX_MONSTERS` 300, `MAX_OBJECTS` 0x1000,
`MAX_PROJECTILES` 0x400, `MAX_MAP_ACTIONS` 0x400, `MAX_LIGHTNING` 0x20, `MAX_MESH_EFFECTS` and
`MAX_MODEL_EFFECTS` 16, and `MAX_MODELS` 0x80 (`models.rs`). Slots are allocated by
`mandrake_tfl_sim::objects::DatumPool` with the engine's salted identifiers, so slot reuse matches the
original.

### Set-up order

`TflSim::set_up_as` follows the original map start:

1. Seed the RNG from the film header's seed (`TflRng::new`).
2. Read the difficulty (`mandrake_tfl_sim::film_difficulty`, header `+0x34`) and the options
   (`mandrake_tfl_sim::film_game_options`, header `+0x28`).
3. Load the monster initializers.
4. Load the media coverage and the markers' motion words.
5. Start the particle system (it draws).
6. Add the markers: scenery, then the monster palette entries and units (these draw).
7. Place the marker objects: heights, media bits, physics and closed sides
   (`place_marker_objects`).
8. Start the sight maps.
9. Parse the map actions.
10. Set up the statistics teams.
11. Call `update_player_statistics` once.
12. Call `update_map_actions` at tick 0. This is the last part of set-up that draws.

### The tick

`TflSim::step` runs these stages in order. This order is part of the determinism contract,
because each stage draws from the one RNG:

1. **Unfinished orders check.** If a map action gave a monster order the port cannot carry out
   (an `mandrake_tfl_sim::Order` that is not `done`), stop.
2. **Sync sample.** At every tick that is a multiple of 512, except 0, record
   `(tick, rng state)` in `map_actions.game.sync_sample`. This happens before any stage of that
   tick.
3. `execute_monster_commands`: the film's commands for this tick, in order. A command whose
   player byte is not -1 or a valid player index is skipped. Command types:
   - 1 sync
   - 2 general
   - 3 movement
   - 4 target
   - 5 attack a point
   - 6 pick up
   - 7 rename
   - 10 detach

   Any other type is a stop.
4. `update_lightning`
5. `update_projectiles`
6. `update_monsters`, in two loops:
   - **First loop** (`targets.rs`): only the monsters in slots `tick & 0x3f`, `+64`, `+128`, ...
     look for targets. Slot 0 also swaps the sight maps, and the local team's monsters mark what
     they see.
   - **Second loop:** for every live monster, animate its object, plan its movement if it has an
     order (flag 0x20), then run its action handler: idle, celebrating, moving, attacking,
     picking up, healing, knocked down, flinching or dodging. Any other action is a stop.
7. `update_mesh_effects`
8. `update_map_actions`, which does nothing unless `tick & 0x1f == 0` (every 32 ticks). On its
   first call it activates the actions flagged 1. Then it triggers every active action whose
   time has come. It rebuilds the spatial groups lazily and resets the attack AI's records.
9. `update_explosions`
10. `update_particle_clock`
11. `update_player_statistics`. After the game ends, it records the permanent rankings once.
12. `update_model_effects`
13. `tick += 1`

Compared with Myth II's tick (see [simulation.md](simulation.md)), TFL has no fire, wind, mirror
groups, connectors, animating scenery or model animations. It has no pause or game-speed
commands. Its statistics run before the model effects.

The tick rate is 30 ticks a second, as in Myth II. The client runs it at 30 ticks a second times
the game speed.

### The SYNC check

A film's sync command (type 1) carries 12 bytes: a `u16` word, then a `u32` random state at
`+4`, then a `u32` sample tick at `+8`, all big-endian. If the sample tick equals the sim's last
sample tick (`sync_sample`) and the states differ, the game is out of sync. The port then
returns a `Stop` whose reason begins `out of sync:`. A sync whose tick doesn't match the last
sample is ignored, as in the original. Before the first sample, the comparison is against
`(0, film seed)`. This is the same rule as Myth II's, and the sample tick is a tick number, not
a checksum.

The SYNC compares only the RNG state. Any change to the number or order of draws therefore
shows up as an out-of-sync stop at the next 512-tick sample in a film that carries syncs. A
change that leaves the draws alone but moves a unit does not.

### Stops

`mandrake_tfl_sim::Stop { site, reason }` is the sim's only error type, and there is no fallback
behaviour. `site` is an opaque code for the place in the original engine where the port would
have to continue, one of the named constants of `mandrake_tfl_sim::draw_site` (or 0). `reason` names the tick and what was met: an unported command type, an
action handler, a map-action proc, a data error (site 0, `data: ...`), or the out-of-sync
stop. Once a sim has stopped it is not stepped again. The client holds the frame and prints
the tick and the reason (`TflReplay::status`).

### Random numbers (`mandrake_tfl_sim::rng`)

- `mandrake_tfl_sim::TflRng` wraps Myth II's `mandrake_math::BungieRng`. It is the same LCG (multiplier
  1664525, addend 1013904223) and returns the state's **high 16 bits**. Seed 0 becomes 0xfded
  (`BungieRng::DEFAULT_SEED`), as in Myth II.
- Every draw names its call site (`TflRng::draw(site)`, `site` a constant of
  `mandrake_tfl_sim::draw_site` such as `draw_site::PROJECTILE_LIFETIME`) and is appended to `TflRng::log` as an
  `mandrake_tfl_sim::Draw { site, state }` holding the state *before* the draw. This lets a run be
  compared with the original draw for draw. The log is never cleared, so a long game's log
  grows by one 8-byte entry per draw.
- Draw forms:
  - `draw_mod(site, n)` is the engine's `random() % n`, computed as `(r & 0xffff) % n`. It
    returns `None` for `n <= 0`, where the engine would divide by zero.
  - `draw_scaled(site, n)` is `(r * n) >> 16`.

  Which form a call site uses is part of the port. Swapping one for the other changes the
  result.
- Myth II's sim counts draws per site (`mandrake_sim::engine_rng`). TFL's keeps the full sequence. The
  stream itself is the same generator.
- **Cosmetic randomness is not this RNG.** The original TFL 1.5 has a second, local generator
  for presentation: local projectiles, stains, blood and footprints, sound permutations and
  pitch. The client owns that generator and shares one instance among those painters
  (`mandrake_tag::sound_manager::LocalRandom`). It must never be drawn from inside `mandrake-tfl-sim`. If
  the sim draws presentation randomness, every film desyncs.

### Trigonometry (`mandrake_tfl_sim::trig`)

Angles are `u16`, with 0x10000 a full turn. The sine and cosine tables are 1024 `i16` entries
in Q14 format (0x4000 = 1.0). The tangent table is `i32` Q14, with `i32::MIN` where the cosine
is 0. `mandrake_tfl_sim::trig::trig` builds them once. Each entry is
`sin/cos(i * (atan(1) * 8) / 1024) * 16384 + 0.5`, truncated toward zero, with the four axis
entries written as exact constants.

How TFL's tables and functions differ from Myth II's (`mandrake_sim::engine_move::trig_tables`):

| | Myth II | TFL |
|---|---|---|
| full turn in the table build | the constant `TAU` | `atan(1) * 8` computed by the C library at start-up |
| table index | `((angle + 0x20) & 0xffff) >> 6`, on a `u16` | `((angle + 0x20) >> 6) & 0x3ff`, on the full `i32` (`mandrake_tfl_sim::trig::index`) |
| tangent at index 0x200 | written as 0 | computed (also 0) |
| arctangent | binary search (lower bound) in the quadrant's 255 tangents, then round to the nearer neighbour | linear walk from the quadrant's start until the difference changes sign, at most 255 steps, then take the nearer of the two (`mandrake_tfl_sim::trig::arctangent`) |

Other functions:

- `mandrake_tfl_sim::trig::rotate(p, centre, angle)` rotates by **minus** the angle:
  `x' = (s*dy + c*dx) >> 14`, `y' = (c*dy - s*dx) >> 14`.
- `mandrake_tfl_sim::trig::angle_difference` returns the unsigned difference in `0..=0x8000`.
- `arcsine` (crate-private, in `obstacles.rs`) and `mandrake_tfl_sim::groups::isqrt` are TFL's own
  integer arcsine and square root. `isqrt` rounds up when the remainder exceeds the root.

The two arctangents agree on the axes and diagonals (the unit tests check `(100, 100)` →
0x2000, and so on). They can differ on near-ties, so use the one that belongs to the game.
The client's selection code takes the arctangent as a parameter for this reason
(`SelectionRules::TFL` in `selection_input.rs`).

### Floating point rules

The original TFL engine is x87 code running at double precision. The sim is integer and
fixed-point except in a few places, and each of those follows the same rules:

- **IEEE `+ - * /`, `sqrt` and Rust `as` casts are bit-exact on every target.** They are used
  freely where the original used the FPU: the mesh effects' wave profile, the projectiles'
  ballistic `fsqrt`, and the formation radius.
- **Truncation is the C runtime's `_ftol`:** truncate toward zero, and in `particles.rs`
  `i32::MIN` for NaN or out of range. Don't round.
- **`float` stores are modelled.** Where the original stored an intermediate as a 32-bit
  `float`, the port does `f64::from(x as f32)`. Examples: the formations' turn
  (`8 * atan(1)` stored as `f32`, `full_turn` in `commands.rs`), the models' pitch vectors, and
  π as an `f32` in the particle transition (`mandrake_tfl_sim::particles::PI_F32`).
- **C library calls (`sin`, `cos`, `atan`) are not guaranteed bit-exact across platforms.**
  They appear only where the result is truncated to an integer far from a rounding boundary:
  - the trig tables, built from `mandrake_tfl_sim::trig::table_sin_cos`;
  - the models' angle vectors (unit test `every_angle_truncates_far_from_an_integer` checks
    every one of the 65536 angles is more than 1e-6 from an integer);
  - the particle transition blend (`mandrake_tfl_sim::particles::transition_blend`).

  The raw `f64` bits may differ between glibc and musl, but the integer tables must not.
- **Statistics ratios are `f64` but only feed the epilogue report**, never the sim state.

`mandrake_tfl_sim::trig::float_target_hashes` hashes (FNV-1a, 64-bit):

- the raw library results (`tfl.atan1.raw`, `tfl.trig.sin_cos.raw`);
- the three integer tables (`tfl.trig.tables`).

`crates/mandrake-tfl-sim/tests/float_targets.rs` pins all three. The tables hash must be the same on
every target. The raw hashes are pinned per C library family: glibc on x86_64 and aarch64
Linux, and musl's libm on wasm32. Those two families differ in some last bits without changing
a table entry.

**Why `float_targets.rs` is its own test binary.** All other integration tests of the crate are
modules of one binary (`tests/all/main.rs`, see below). `float_targets` is kept separate because
the cross-target check builds it alone, for each target, and runs it by name:

- On aarch64 it runs under qemu-user.
- On wasm32-wasip1 it runs under wasmtime.
- On wasm32-unknown-unknown a test binary has no output, so the file exports `hash_stride`,
  `hash_name` and `hash` as `extern "C"` functions, and a small node script calls them.

Folding it into `all` would drag the data-reading film tests into those builds, and the
by-name lookup of the `float_targets-*` binary would break. Myth II does the same with
`mandrake_sim::float_targets` and an example binary.

### Module map

| module | what it holds |
|---|---|
| `lib.rs` | `TflSim`, `Stop`, `film_difficulty`, `film_game_options`, `step`, `update_monsters`, marker object placement |
| `data` | `TflData`: the tag archive, the film, the mesh, definitions read once as raw `Def`s, sequence headers, sounds and key points per collection |
| `setup` (private) | map set-up, markers, monster palette and coloured unit copies (`UnitCopy`), `MonsterInitializer`, `film_monster_initializers` |
| `rng`, `trig` | the RNG with its draw log; fixed-point trig and the float target hashes |
| `objects` | 0x40-byte objects (`TflObject`), `set_object_shape`, `DatumPool` (slot and salt allocation) |
| `monsters` | 0x80-byte monsters (`TflMonster`), action changes, idle, `animate_object` |
| `targets` (private) | `update_monsters`' first loop: target search in 64-tick strides |
| `movement`, `paths`, `obstacles` (`movement` private) | movement planning; best-first path search over a 64x64 cell window (at most 14 points); leg-end obstacle avoidance and `arcsine` |
| `terrain` | ground height, closed cell sides, media coverage |
| `commands` | film commands (sync, general, movement, target, attack point, pick up, rename, detach); formations (`command_places`); the C library heapsort the engine uses |
| `combat`, `attacks` (`attacks` private), `attack` | choosing and firing attacks; the attack AI and the `atta` action |
| `damage`, `knock_down` (private) | damage, deaths, removal, explosions (`damage::Explosions`); knocked-down monsters |
| `projectiles` | projectiles and projectile groups, `update_projectiles` |
| `lightning` | lightning bolts |
| `carried_objects` (private) | giving objects to monsters |
| `models`, `model_effects` (`models` private) | model markers and the cells they close; model effects (`moef`) |
| `mesh_effects` | ring-shaped mesh deformations from projectile groups |
| `particles` | the map's particle system clock (rain, snow) as game state |
| `map_actions`, `actions` (private), `group_actions` | map action records and parameter walks; running, activating and triggering; the group procs |
| `procs`, `legion`, `suicide`, `invisible`, `knot`, `geometry_filter` (private), `sniffing` | individual map action procs: `look`, `obco`, `foll`, `obmo`, `snif`, `lpgr`, `legi`, `suic`, `invi`, world knots, `geom`, and the tutorial's input sniffing |
| `groups` | spatial groups and supergroups, `isqrt` |
| `sight` | the local team's visibility and explored maps (`SightMaps`) |
| `statistics` | scores, rankings, permanent rankings carried between levels, the epilogue report's numbers |
| `sounds`, `stains`, `local_groups`, `tracking` | presentation event queues (sounds, stains/blood/footprints, local projectile groups, camera interest events) |

## Networking (`mandrake_net::game_kind`, `mandrake_net::tfl`)

TFL games run **between this project's clients only**, on Myth II 1.8.5's wire: the same
framing, net layer, lobby, barriers and server ([networking.md](networking.md)). They are not
wire-compatible with original TFL 1.5 peers.

- **The game kind travels in the description.** A TFL game's gameplay version (parameters
  `+0x24`) is `mandrake_net::game_kind::TFL_GAMEPLAY_VERSION`, the bytes `"TFL5"`, which is outside
  the original 1.8.5 engine's accepted range. That engine therefore refuses to join it.
  `mandrake_net::game_kind::GameKind::of` reads the kind back out of a description. Myth II's bytes on
  the wire are unchanged. The marker never reaches a film, because films zero parameters
  `+0x20..+0x28`.
- **Command codes differ** (`mandrake_net::game_kind::CommandCodes`):

  | command | Myth II | TFL |
  |---|---|---|
  | NULL | 0 | 0 |
  | SYNC | 1 | 1 |
  | drop player | 9 | 8 |
  | chat | 10 | 9 |
  | unit adjustment | 12 | 11 |
  | forced end | 0x15 | none |
  | pause | 0x17 | none |
  | game speed | 0x18 | none |
  | highest type | 0x18 | 13 |

  `Joiner::send_command` refuses a type that TFL doesn't have.
- **The description** (`mandrake_net::tfl::description`) carries TFL's parameters, no plugin list, and
  the host's `mandrake_net::tfl::DataChecksums`. Description body `+0x22c` holds the inverted
  tag-archive checksum. Body `+0x230` holds the active external archive's checksum, or
  `mandrake_net::tfl::NO_PLUGIN`. These are the same two words a TFL film keeps at header
  `+0x71c`/`+0x720` (`mandrake_net::tfl::film_checksums`).
- **Data check** (`mandrake_net::tfl::check_data`): a joiner's plugin word must match the host's, then
  its archive checksum. Failures are `mandrake_net::tfl::DataMismatch::Plugin` and
  `DataMismatch::Archive`.
- **New-game data** (`mandrake_net::tfl::new_game`): every peer builds it independently from the
  description and the listed players, with `TflNewGame::network`.
- **Player bytes.** A received command's player byte is a net id. It is mapped to the player's
  index in the new-game data before the command is queued and recorded
  (`mandrake_net::tfl::player_index`, `mandrake_net::tfl::player_order`). The byte 0xff names nobody and
  stays.
- **Heartbeat.** The server sends a heartbeat after more than 3 server ticks with nothing sent
  (`mandrake_net::server`).

The `mandrake-net` crate's `tests/tfl.rs` runs a TFL host and joiners over in-memory pipes. The
client's menus do not offer TFL multiplayer yet. Only the solo campaign and film replay are
wired up.

## Menus and session (`mandrake_session::tfl`)

`mandrake_session::tfl::TflSession` is the game side of TFL's menus: the states the original engine
moves between, what each screen shows and what a player can do there. It names no dialog or
control. It sits next to Myth II's `mandrake_session::Session`, and reuses its epilogue rules
(`mandrake_session::rules::TFL_EPILOGUE`) and `mandrake_session::Progress`.

- **Screens** (`mandrake_session::tfl::TflScreen`):
  - `MainMenu`
  - `NewGame` (the levels, the difficulties from `stli indl`)
  - `Films`
  - `Prologue` (the narration)
  - `Epilogue` (Myth II's post-game page under TFL's rules)
  - `Movie` (a `moov` tag, played by the shared cutscene player)
- **Actions and launches.** `mandrake_session::tfl::TflAction` is what the player does.
  `mandrake_session::tfl::TflLaunch` is what the session hands to the window: a level, a film or quit.
- **Levels.** `mandrake_session::tfl::solo_levels` lists the solo levels from the mesh flags. A level
  must have `LEVEL_SOLO`, must not have `LEVEL_HIDDEN`, and must not be one of three meshes the
  original leaves out by id. Hold G at New Game to list every level.
- **Level carry.** `LevelCarry` is what a level's end leaves for the next: the client's veterans
  and permanent rankings (`TflCarry` in `tfl_live.rs`). `TflSession` keeps it without reading
  it.
- **Progress.** `TflProgressStore` decides where progress is saved. It is never TFL's install.
- **Films.** `mandrake_session::tfl::saved_films` lists our films folder first, then the install's
  `tags/Local/RECO`. A file counts as a film when its tag header's group is `reco`.
- **Narration text.** `mandrake_session::tfl::style_codes` rewrites the narration's `\x` style codes
  to the `|x` codes the font code reads.

The front end over the session is `mandrake_dialogs::tfl::TflDialogs`. It draws the original dialogs
from TFL's own interface tags with the shared dialog engine.

## The client's game switch

`crates/mandrake-client/src/lib.rs` `run` decides the game before anything else. If the result is
TFL, it returns into `tfl_view::run_tfl` and never touches Myth II's startup.

How the game is chosen (`chosen_game` in `tfl_game.rs`):

1. `--game soulblighter|tfl` (or the settings file's `[game] game`) decides when given. It parses
   with `args::parse_game`, over `mandrake_tag::tfl::Game`'s `FromStr`, which also accepts `myth2` and
   `fallen lords`.
2. Otherwise the game is TFL if `--replay` names a TFL film (by its header, looked up in
   `tags/Local/RECO` if it is a bare name), or if `--tfl-data` holds TFL data (`Game::detect`).
3. Otherwise it is Myth II: Soulblighter. Nothing asks (there is no startup chooser):
   one settings file serves both games, and `--game tfl` starts TFL with it.

What a TFL run reuses and what is its own:

| area | shared with Myth II | TFL's own |
|---|---|---|
| shell and window | the shared shell (`app/shell.rs`). TFL's game is a `GameScene` (`tfl_window.rs`'s `TflApp`), as Myth II's is (`app/myth2_scene.rs`) | the scene's contents |
| menus | the dialog engine, `mandrake_session::Screen::Tfl`, the cutscene player, the post-game page | `mandrake_session::tfl`, `mandrake_dialogs::tfl`, `tfl_menu.rs`, `tfl_menu_music.rs` |
| sim state to drawing | `world_view::WorldView`, read by every drawing module; the interpolation between ticks | the adapter `world_view/tfl.rs` and `TflReplay` (`tfl_game.rs`), which work out each object's colours when it appears |
| rendering | the frame's passes, the model renderer, the sprite path, lightning drawing (`lightning_draw.rs` with `LightningRules::TFL`), local projectiles (`projectile_local.rs` with `Rules::Tfl`), the fog and lighting config (keys `tfl.map` / `tfl.<mesh>`) | the landscape (`tfl_landscape.rs`: two triangles a cell, water cells drawn flat at 0), models' placement (`tfl_models.rs`), TFL's single sorted list of sprites and model surfaces with no depth buffer (`tfl_view.rs`) |
| interface | score line layout (`scores.rs`) | status bar and HUD (`tfl_hud.rs`), score table (`tfl_hud_scores.rs`), the 8-bit overhead map (`tfl_minimap.rs`) |
| input | the controls file, camera keys and observer, selection rules with TFL's parameters (`SelectionRules::TFL`, `ClientRules::TFL`) | TFL's mouse and selection (`tfl_input.rs`); TFL's default key sets as presets (`controls_tfl.rs`) |
| sound | the mixer, the sound manager with `SoundRules::TFL` (4 one-shot channels) | the sim's sound events played by `tfl_sound.rs` |
| live game and films | the film command stream | `tfl_live.rs` (the solo game, 30 ticks a second, recording `LAST RECORDING`), `tfl_live_film.rs` (the solo film header) |

New TFL client work should extend a shared module with TFL's differences passed in as data,
following the `*Rules::TFL` pattern. Don't add a parallel `tfl_*` copy of a shared module.

## Verifying changes

Self-contained tests need no game data:

- **`mandrake-tfl-sim` unit tests** (`cargo test -p mandrake-tfl-sim --lib`):
  - the RNG's first states from a known seed, and seed 0 → 0xfded (`rng.rs`);
  - the trig axes and the arctangent's eight compass points (`trig.rs`);
  - heapsort and the rename command's name cut (`commands.rs`);
  - `isqrt` rounding (`groups.rs`);
  - the models' angle vectors and the truncation-margin check over all 65536 angles
    (`models.rs`);
  - the particle clock and density (`particles.rs`);
  - the statistics' elimination, the winner and permanent rankings (`statistics.rs`);
  - the map action memory layout, legion geometry, sniffing flags, and the tracking observer.

  One unit test in `damage.rs` needs TFL data and a film, and skips or fails without them.
- **`float_targets`** (`cargo test -p mandrake-tfl-sim --test float_targets`) pins the trig tables'
  hash and the C library's raw results. Run it after touching anything in `trig.rs` or any use
  of `sin`/`cos`/`atan`. If the tables hash changes, the sim's angles changed on that target.
- **`mandrake-tag` TFL readers** (`cargo test -p mandrake-tag tfl::`) work on synthetic data:
  - a built `.gor` reads back, and a short or untiled one is refused;
  - `Game::detect` on a temporary folder;
  - a version-7 mesh, and a short cell block refused;
  - a 96-byte `geom`, and an unknown version refused;
  - the 512-byte `mons`;
  - string splitting, dialog version checks, the 256-byte `inte`;
  - the shading table and team colour rule;
  - definition sizes against the original engine's table.
- **`mandrake-rec`**: `a_myth_ii_film_is_not_a_tfl_film`. The other TFL film test reads a film
  file that doesn't ship.
- **`mandrake-session`** (`cargo test -p mandrake-session tfl::`): preferences parsing, narration style
  codes, level movies and their skip rules, level header fields.
- **`mandrake-net`**: the server's TFL drop-player and heartbeat unit tests in `server.rs`.

The integration tests in `crates/mandrake-tfl-sim/tests/all/` are a single binary. Each file is a
module listed in `tests/all/main.rs`, and a new file runs only once it is listed there. One
file's tests run with `cargo test -p mandrake-tfl-sim --test all <module>::`. Only self-contained
tests ship there; tests that replay films need TFL data and films that are not part of this
tree. Keep `float_targets.rs` out of `all`, for the reason given above.

**Checking determinism with your own data and films.** With a TFL 1.5 install, set
`TFL_DATA_DIR` to it and replay films with the bench example:

```text
TFL_DATA_DIR=<tfl dir> cargo run --release -p mandrake-tfl-sim --example tfl_film_bench -- <film>...
```

It prints one tab-separated row per film. The columns are:

| column | meaning |
|---|---|
| `row` | the literal word `row` |
| `film` | the film's path |
| `setup` | the set-up used: `new`, or `set_up` where `new` stops |
| `ticks` | ticks reached |
| `end_tick` | the film's end tick |
| `draws` | number of draws |
| `draws_hash` | FNV-1a over every draw's site and state |
| `state_hash` | FNV-1a over every monster's and object's position and flags, the tick and the RNG state |
| `stop_site` | the stop's site, `-` for none |
| `stop` | the stop's reason, `end at tick N` for none |

Without data it prints `skip`. To check a change:

1. Record the rows before the change.
2. Rerun after it.
3. A film that reaches fewer ticks is a regression. A film whose draws or state hash change at
   the same tick count is a silent divergence, unless the change was meant to alter behaviour.
4. A stop whose reason begins `out of sync:` means the RNG stream no longer matches the
   original's SYNC samples. Something added, removed or reordered a draw.

In the client, `--replay <film>` with `--tfl-data <dir>` replays a film in the window. When the
sim stops, the frame holds, and stdout and the window title give the tick and the reason.

For a change outside the sim (rendering, HUD, input, menus), the sim cannot be affected. A
focused unit test and one look at the client suffice. For a sim change, keep the films' draws
and state hashes unchanged, or explain every row that moved.
