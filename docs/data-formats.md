# Data formats: tag archives, tags and the game folder

This part covers how the port reads the original game's data: the shared types of `mandrake-common`
(four-character codes, the error type, the virtual file system the native client and the browser
share, Mac Roman text), the tag archive format and its writer in `mandrake-tag`, the layout of every
tag group `mandrake-tag` parses, how patches and plugins stack over the base installs, and the folder
layout the code expects from a Myth II 1.8.5 install. Everything here is read-only data handling:
no simulation state, no rendering. The simulation and client parts ([simulation.md](simulation.md),
[rendering.md](rendering.md), [audio.md](audio.md), [interface.md](interface.md)) build on the
types described here.

All multi-byte numbers in Myth II tag files are **big-endian** (the game was born on the Mac),
unless a section says otherwise. "World units" are 1/512 of a map cell; angles are 16-bit binary
angles (`0x10000` = full circle); "8.8 fixed" means `0x100` = 1.0 and "16.16 fixed" `0x10000` = 1.0.

## 1. `mandrake-common`

### 1.1 `mandrake_common::FourCC`

`FourCC(pub [u8; 4])`: a Bungie four-character code, the bytes as they are stored (so
`as_u32()` is `u32::from_be_bytes`). Tag groups (`mesh`, `.256`), tag ids (`00tm`, `wam1`) and
archive signatures (`mth2`) are all `FourCC`s. `Display`/`Debug` print printable ASCII as is and
any other byte as `\xNN`; `as_str()` gives `"????"` for non-UTF-8 bytes. Parse from text with
`"mesh".parse::<FourCC>()` or `FourCC::try_from("mesh")` (exactly 4 bytes, else an error).
Tags refer to each other by `FourCC`; in tag payloads a reference of `0xffffffff` (and, in
most parsers, `0`) means "none", and parsers return `Option<FourCC>` for those fields.

### 1.2 `mandrake_common::M2Error`

One error enum for every data crate: `Io`, `InvalidMagic { expected, found }`, `TagParse(String)`,
`ReplayParse(String)`, `Simulation(String)`. Tag parsers return `Result<_, M2Error>` and report a
short or inconsistent payload as `TagParse` with the offsets involved. They never panic on
malformed input, which matters because plugins can come from strangers (see 4.5).

### 1.3 The virtual file system

`mandrake_common::VirtualFileSystem` (`Send + Sync`) abstracts the game folder so that the same code
runs natively and in the browser:

| method | meaning |
|---|---|
| `read_file(path)` | the whole file |
| `open_reader(path)` | a boxed `SeekableRead` (any `Read + Seek`) |
| `exists(path)` | file or folder |
| `is_file(path)` | a file, not a folder |
| `list_dir(dir)` | entries as `dir` joined with each name, no particular order; empty if absent |

Implementations:

- `mandrake_common::NativeVfs`: rooted at a base path (`NativeVfs::new(base)`, or `default_cwd()`
  for `.`); absolute paths are used as given. It is plain `std::fs`.
- `mandrake_common::MemoryVfs`: a map of path to shared bytes (`Arc<[u8]>`, so a reader of a 100 MB
  archive doesn't copy it). Paths are normalised (`\` to `/`, leading `/` dropped), so
  `/game/tags/x` and `game/tags/x` are the same file. A folder exists when some file is under it;
  `list_dir` lists the direct children (files and folder names), sorted.

The process-wide game folder is `mandrake_common::game_vfs()`. Natively it is a `NativeVfs` on the
working directory, so a path means what it means to `std::fs`. The browser build (`mandrake-web`)
calls `mandrake_common::set_game_vfs` once, before the first read, with a `MemoryVfs` filled with the
files of the folder the player picked (the page hands each file to the wasm export
`add_game_file`, under `/game/...`; see [web.md](web.md)).
Everything that reads the game's own files goes through `game_vfs()`: tag archives, films,
saved games, preferences. Files the client writes itself (its settings, screenshots, downloaded
plugins) don't.

### 1.4 `mandrake_common::mac_roman`

The original game's text is Mac Roman. Names inside tags, archive names, plugin names and film
player names are compared as bytes, so the port keeps them as bytes and decodes for display:

- `decode(bytes)`: every byte is one character (lossless; `encode(decode(b)) == b`).
- `decode_c(bytes)`: up to the first NUL.
- `encode(text)`: back to bytes; characters outside Mac Roman become `?`.
- `trim(bytes)`: trims ASCII white space on the bytes.

`mandrake_tag::header::decode_c_string` is an older, lossy decoder (curly quotes to straight ones)
still used for display-only names such as bitmap and sequence names.

## 2. The monolithic tag archive

A Myth II tag file (`tags/small install`, `plugins/Patch 1.2`, a map plugin) is one
"monolith": a header, an optional entry-point table, a directory of tag headers, and the tag
payloads. Types: `mandrake_tag::header::SBMonoHeader`, `mandrake_tag::header::SBEntryTag`,
`mandrake_tag::header::SBTagHeader`; reader `mandrake_tag::TagArchive`; writer `mandrake_tag::write_monolith`.

```
offset 0                     SBMonoHeader            128 bytes
offset 128                   SBEntryTag x entry_tag_count     112 bytes each
then                         SBTagHeader x tag_list_count     64 bytes each
then                         tag payloads (each at its header's offset)
```

### 2.1 Archive header (`SBMonoHeader`, 128 bytes)

| offset | type | field | notes |
|---|---|---|---|
| `0x00` | `i16` | `type` | 1 plugin, 2 patch, 3 foundation install, 5 interface plugin, 6 (HD texture/template packs), 7 metaserver component |
| `0x02` | `u16` | `version` | for patches the patch level (2 = Patch 1.2 ... 5 = Patch 1.5, 8 = Patch 1.8 Interface) |
| `0x04` | `[u8; 32]` | `name` | NUL-terminated, Mac Roman (`name_as_str()`) |
| `0x24` | `[u8; 64]` | `description` | NUL-terminated; for plugins usually a URL |
| `0x64` | `u16` | `entry_tag_count` | |
| `0x66` | `u16` | `tag_list_count` | |
| `0x68` | `[u8; 4]` | `checksum` | identifies the file in a game's plugin list and in the tag-set checksum (4.4) |
| `0x6c` | `u32` | `flags` | |
| `0x70` | `u32` | `size` | whole file length |
| `0x74` | `[u8; 4]` | `header_checksum` | |
| `0x78` | `[u8; 4]` | `padding` | |
| `0x7c` | FourCC | `signature` | `mth2` (`SBMonoHeader::SIG_MTH2`) or `dng2` (`SBMonoHeader::SIG_DNG2`) |

The reader accepts either signature. The port never verifies or recomputes the two checksums;
it uses `checksum` as an identity (it is copied verbatim into network game descriptions and the
tag-set checksum).

### 2.2 Entry tag (`SBEntryTag`, 112 bytes)

An entry point is a map the file offers in the game's map lists.

| offset | type | field |
|---|---|---|
| `0x00` | FourCC | `tag_id`: the map's `mesh` id |
| `0x04` | `[u8; 12]` | `reserved1` |
| `0x10` | `[u8; 32]` | `internal_name`: the entry's "file name", e.g. `00 Training Map`; its leading number orders the map list |
| `0x30` | `[u8; 32]` | `display_name` |
| `0x50` | `[u8; 32]` | `description` |

`mandrake_tag::mesh_links` builds the menus' map records from these plus fields of the mesh header
in the same file (`mandrake_tag::mesh_links::CachedEntry`, `mandrake_tag::mesh_links::CampaignTags`).

### 2.3 Tag header (`SBTagHeader`, 64 bytes)

| offset | type | field | notes |
|---|---|---|---|
| `0x00` | `i16` | `identifier` | editor data |
| `0x02` | `i8` | `flags` | |
| `0x03` | `i8` | `tag_type_idx` | |
| `0x04` | `[u8; 32]` | `name` | NUL-terminated Mac Roman, e.g. `amb boat` |
| `0x24` | FourCC | `tag_type` | the group, e.g. `mesh` |
| `0x28` | FourCC | `tag_id` | the id, unique per group, e.g. `00tm` |
| `0x2c` | `i32` | `tag_data_offset` | from the **start of the file** |
| `0x30` | `i32` | `tag_data_size` | |
| `0x34` | `u32` | `user_data` | |
| `0x38` | `i16` | `version` | the group's format version (Myth II meshes are 11, `mandrake_tag::mesh::MYTH2_MESH_VERSION`) |
| `0x3a` | `i8` | `destination` | |
| `0x3b` | `i8` | `owner_index` | |
| `0x3c` | FourCC | `signature` | always `mth2` (`SBTagHeader::SIGNATURE`), even in `dng2` files |

A tag is addressed by `(group, id)`. Names are for people and lists; lookups by name exist
(`TagArchive::find_tag`, exact then ASCII case-insensitive) but the game resolves references
by id.

### 2.4 `mandrake_tag::TagArchive`

- `TagArchive::open(vfs, path)` streams from a reader (only the directory is read up front;
  `get_tag_data` seeks and reads one payload). `TagArchive::from_bytes(Vec<u8>)` holds the whole
  file. `TagArchive::from_reader` takes any `SeekableRead`.
- The directory parser checks the archive signature and every tag header's signature. Payload
  reads check `offset + size` against the real file length before allocating, so a lying
  directory gives an error, not a huge allocation.
- `list_tags()`, `find_tag_by_id(group, id)`, `find_tags_by_type(group)`, `find_tag(group, name)`,
  `get_tag_data(entry)`, `read_tag_prefix(entry, len)` (e.g. a mesh's first 1024 bytes),
  `header()`, `entry_tags()`.
- `TagEntry` is the directory row: `index`, `name`, `tag_type`, `tag_id`, `offset`, `size`, the
  full `header`, and `layer` (which file of a merged archive holds it, see below).
- `TagArchive::with_overrides(files)` stacks other archives over this one: later files win over
  earlier ones and all win over the base, per `(group, id)`; overridden entries disappear from
  `list_tags()`. The header and entry tags stay the base file's.
- `format()` says which on-disk format the file was (`mandrake_tag::ArchiveFormat`): `Myth2Monolith`,
  or one of TFL's (`TflGor`, `TflLoose`), see section 6.

### 2.5 Writing archives (`mandrake_tag::archive_writer`)

`write_monolith(header, entries, tags) -> Vec<u8>` is the reader's inverse. It writes the given
header with `entry_tag_count`, `tag_list_count` and `size` replaced by what it writes, then the
entries, then one `SBTagHeader` per `mandrake_tag::MonolithTag { header, data }` with
`tag_data_offset`, `tag_data_size` and `signature` set (payloads are laid out back to back right
after the directory), then the payloads. Everything else (type, version, names, both checksums,
flags, signature, each tag's name, id, version and user data) is written as given. It refuses
more than 65535 tags or entries and files over `i32::MAX` bytes.

Because the original checksum algorithm is not reproduced, a tool that builds a new plugin must
choose its own `checksum` (it only has to differ from other plugins'; our plugin builders write an
FNV-1a hash of the bytes after the header at `0x68`, and one of the first `0x74` bytes at `0x74`).

## 3. Tag groups

### 3.1 Table: group -> module -> what it is

Parsers take the raw payload (`TagArchive::get_tag_data`) and return a typed struct. Most keep
only the fields the port uses; several keep the raw bytes too.

| group | parser (`mandrake_tag::...`) | what it is |
|---|---|---|
| `mesh` | `mesh::MeshTag`, `mesh::GameMesh`, `mesh::MediaCoverage`, `mesh::MeshLod`, `mesh::LandscapeShading`, `map_action::MeshMapActions`, `connector::MeshConnectorList`, `mesh_links::MeshLinks` | a map: header, cell grid, marker palette and markers, map actions, connectors, campaign links |
| `.256` | `collection::CollectionTag`, `sequence_header::SequenceHeader` | sprite collection: bitmaps, bitmap instances, sequences, color tables, hue changes, shadow maps; also a map's landscape texture (`collection::TerrainTexture`) |
| `core` | `sequence_header::CollectionReference`, `collection::CorePermutations` | collection reference: a `.256` plus colour permutations |
| `unit` | `unit::UnitTag::from_unit_bytes` | a placeable unit: its `mons` and `core` |
| `mons` | `unit::UnitTag::from_mons_bytes`, `unit::MonsterTerrain` | monster definition (1024 bytes): sequences, speeds, terrain, attacks, sounds |
| `scen` | `scenery::SceneryTag` | scenery definition |
| `obje` | `object::ObjectDefinition`, `object::ObjectPhysics` | object definition: vitality, scale, physics, damage modifiers |
| `proj` | `projectile::ProjectileDefinition` | projectile definition (256 bytes) incl. `projectile::DamageDefinition` |
| `prgr` | `projectile::ProjectileGroupDefinition` | projectile group: parts launched together (512 bytes) |
| `geom` | `model::GeometryTag` | 3D geometry: materials, vertices, surfaces |
| `mode` | `model::ModelTag` | model: a `geom` plus texture permutations and cells |
| `anim` | `model_anim::ModelAnimationTag` | model animation: a flip-book of `mode` frames |
| `soun` | `sound::SoundTag` | sound effect with IMA4 ADPCM permutations |
| `amso` | `sound::AmbientSoundTag` | ambient sound (looping + periodic) |
| `medi` | `media::MediaDefinition` | media (water, lava...): splash groups, reflections, wobble |
| `conn` | `connector::ConnectorTag` | connector strip (barbed wire, rope) between scenery |
| `dmap` | `detail_map::DetailMap` | landscape detail map (plugin only) |
| `dtex` | `detail_map::DetailTexture`, `pix0r1` | detail texture, `pix0r1`-compressed RGB (plugin only) |
| `stli`, `text` | `stli::StringListTag`, `mesh_links::string_list_lines`, `mesh_links::text_bytes` | string lists and text |
| `font` | `font::FontTag`, `font::FontFamily` | bitmap font; plus the styled-text layout and software rasteriser |
| `inte` | `interface::InterfaceDefinition`, `interface::InterfaceColorTable` | interface definition: the interface collection and six fonts |
| `bina` | `dialog::DialogDefinition` | dialog: rectangle and items (also the in-game status and control bars) |
| `ditl` | `dialog::parse_ditl` | a dialog's item captions |

Groups read outside `mandrake-tag`, by the simulation or client, straight from the payload:
`ligh` (lightning), `lpgr` (local projectile groups, client-side effects), `arti` (artifacts),
`phys` (local physics), `wind`, and the metaserver/lobby dialogs `nesp`,
`inmg`, `inmn`. See [simulation.md](simulation.md) and [interface.md](interface.md).

Modules in `mandrake-tag` that are not a tag group:
`sequence_anim` (per-object sprite frame playback, the per-tick rule the simulation and client
share), `model_hit` (integer model transforms and the segment-vs-model test used by both the
line of fire and the renderer), `sound_manager` (the original sound manager: channel allocation,
ambient sources, 3D gain, behind a `SoundChannelBackend` trait; see [audio.md](audio.md)), `hud`
(layout rules of the status bar, control bar and minimap), `tag_files` and `tagset` (section 4),
`mesh_links` (what the menus need from tag files), `pix0r1` (an image codec).

Unless stated, offsets below are from the start of the payload.

### 3.2 `mesh` (map)

`MeshTag::from_bytes` needs at least the 1024-byte header (`MeshTag::HEADER_SIZE`). Every
"offset" field in the header is relative to the **end of the header** (payload offset 1024).

Header fields the port reads:

| offset | type | field |
|---|---|---|
| `0x00` | FourCC | landscape collection (`.256`, the ground texture) |
| `0x04` | FourCC | media (`medi`) |
| `0x08`, `0x0a` | `u16` x2 | submesh width, height; the map is `32 * w` by `32 * h` cells |
| `0x0c`, `0x10` | `u32` x2 | cell grid offset, size (`cells * 12`, else an error) |
| `0x24`, `0x28` | `u32` x2 | marker palette count, offset (32-byte entries) |
| `0x34`, `0x38` | `u32` x2 | marker count, offset (64-byte entries) |
| `0x48` | FourCC | connector definition (`conn`) |
| `0x4c` | `u32` | map flags (interface bits such as "overhead map shows only explored area") |
| `0x58..0x6f` | | landscape shading: dark weight, light weight (`i16`), dark and light colours (`u16` x3 each), transition point (16.16) |
| `0x80`, `0x84`, `0x88` | `u32` x3 | map action count, offset, total size |
| `0x8c` | FourCC | string list: map name, then objectives |
| `0x90`, `0x94` | FourCC x2 | post-game and pre-game art |
| `0x98` | FourCC | overhead map collection |
| `0x9c`, `0xa0` | FourCC x2 | next map, alternate next map |
| `0xb0` | FourCC | briefing `text` |
| `0xc0`, `0xc4` | `u32` x2 | media coverage section offset, size |
| `0xcc`, `0xd0` | `u32` x2 | level-of-detail section offset, size |
| `0xfc` | FourCC | captions string list |
| `0x100` | FourCC | narration `soun` |
| `0x104`, `0x108` | FourCC x2 | ambient `amso` after a win, after a loss |
| `0x10c` | `i32` | reverb preset (used with 3D sound) |
| `0x11c`, `0x120`, `0x124` | `u32` x3 | connector record count, offset, size |
| `0x12c` | 3 x 64 bytes | cutscene file names (briefing, win, loss) in the `cutscenes` folder |
| `0x1ec` | FourCC | hints string list |
| `0x1f0..0x1f4` | `u16` x3 | fog colour |
| `0x1f8` | `f32` bits | fog density (0 = no fog) |
| `0x224` | `u32` | campaign flags |

The map's ambient background `amso` and the remaining fields are read by the simulation and the
client directly; see [simulation.md](simulation.md).

**Cells** (`mesh::Myth2Cell`, 12 bytes each, row-major, `cell_width * cell_height`):

| offset | type | field |
|---|---|---|
| `0x00` | `i16` | ground height (world units) |
| `0x02` | `u16` | packed normal |
| `0x04` | `u16` | flags: bits 0..3 and 4..7 the terrain types (0..15) of the cell's two triangles; `0x1800` media corner; `0x4000` cell killed (not drawn) |
| `0x06` | `i16` | first object index (runtime; -1) |
| `0x08` | `i16` | media surface height |
| `0x0a` | `i16` | model index (runtime; reset at load) |

Heights inside a cell come from `mesh::get_mesh_height` (integer, the original's triangle split
and media rules), which the simulation and renderer both use.

**Marker palette** (`mesh::MarkerPaletteEntry`, 32 bytes): `i16` marker type (0 observer,
1 scenery, 3 unit, 5 sound, 6 model, 9 projectile, 10 waypoint, 11 model animation), `i16` flags,
FourCC tag (`scen`, `unit`, `amso`, `mode`, `proj`, `anim`...), 24 bytes of type data (for units,
`i16` team first).

**Markers** (`mesh::MarkerEntry`, 64 bytes): `u32` flags, `i16` type, `i16` palette index,
`i16` identifier (the object's identifier in the game, e.g. monster 10006), `i16` minimum
difficulty, `i32` x, y, z (world units), then at `0x20` yaw and `0x22` pitch (`i16`), with
type-specific data after (a model marker's texture permutation is the `i16` at `0x24`). Some
old maps hold the angles at `0x18`/`0x1a` instead; the parser falls back to those when `0x20`
and `0x22` are both zero.

**Media coverage** (`mesh::MediaCoverage`): per row of coverage pixels (8 per cell), a sorted list
of `i16` columns where coverage toggles, ended by a value `>= width * 8`. **LOD** (`mesh::MeshLod`):
one `u16` per 2x2-cell block: bits 0..3 edge subdivided, 4..7 block cells drawn, 8..15 media
triangles. Both are used as stored.

**Map actions** (`map_action::MeshMapActions::from_mesh_bytes`): `count` 64-byte records
(`map_action::MapActionRecord`: `u16` identifier, `i16` trigger kind, `u32` action tag, `u32`
flags, `i32` delay and delay range, `i16` parameter count and size and offset at `0x14..0x1a`,
`i32` next time at `0x3c`), then a parameter buffer. Each parameter is an 8-byte header (`i16`
type, `i16` element count, FourCC name) followed by `count` elements of the type's width
(`map_action::PARAMETER_TYPE_WIDTHS`), padded to 4 bytes. What the actions do is in
[simulation.md](simulation.md).

**Connectors** (`connector::MeshConnectorList`): 64-byte records of up to 31 `i16` scenery
marker identifiers with the `i16` count at `0x3e`; bytes past the count are garbage.

`mesh::GameMesh::from_tag(version, data)` dispatches on the tag header's version: 11 is Myth II,
7 is TFL ([tfl.md](tfl.md)), anything else an error.

### 3.3 `.256` (collection)

`CollectionTag::from_bytes`. A 320-byte header (`CollectionTag::HEADER_SIZE`); all its offsets
are relative to the end of the header (payload offset 320).

| header offset | type | field |
|---|---|---|
| `0x06` | `i16` | collection pixels-to-world scale |
| `0x40`, `0x44` | `u32` x2 | color table count, offset |
| `0x50`, `0x54` | | hue change count, offset (128-byte records, `collection::HueChange`) |
| `0x60`, `0x64` | `i32` x2 | bitmap count, offset (128-byte bitmap references) |
| `0x70`, `0x74` | `i32` x2 | bitmap instance count, offset (64 bytes each) |
| `0x80`, `0x84` | `i32` x2 | sequence count, offset (128-byte sequence references) |
| `0x90`, `0x94` | `u32` x2 | shadow map count, offset (32 bytes each, `collection::ShadowDescriptor`) |

- **Color table**: a 32-byte header (`u32` count first), then entries of 8 bytes: `u16` R, G, B
  (16 bits per channel; `palette` keeps the high bytes) and a `u16` flags word. Index 0 is the
  transparent key. `color_table` keeps the full 16-bit entries.
- **Bitmap reference** (128 bytes): name at `0x00` (32 bytes), `i32` data offset at `0x40`, `i32`
  size at `0x44`. **Bitmap data**: `i16` width, height, bytes per row, `u16` flags (`0x04` RLE,
  `0x10` 16-bit pixels, `0x40` RLE rows padded to even, `0x02` index 0 is transparent in an
  uncompressed bitmap, `0x0800` opacity from colour, `0x2000` drawn with the raw color table),
  48 bytes of header in all, then `height * 4` bytes of row data, then pixels. An RLE row is
  `i16` run count, `i16` pixel count, `run_count` pairs of `i16` `[x0, x1)`, then the run pixels.
  16-bit pixels hold the palette index in the low byte and a transparency level in the high
  byte's low nibble (`BitmapFrame::wide_pixels`).
- **Bitmap instance** (64 bytes): `u32` flags at `0x00` (1/2 mirror x/y, `0x10` drawn along the
  velocity), registration point `i16` x, y at `0x0c`/`0x0e`, bitmap index `i16` at `0x1c`. When
  instances share a bitmap with different registration points, the parser clones the frame so
  each keeps its own point.
- **Sequence reference** (128 bytes): name at `0x00`, `i32` offset of the sequence at `0x40`.
  **Sequence**: a 64-byte header (`u32` flags at `0x00`: bit 0 later views are mirrored copies,
  bit 2 the view comes from the object rather than the camera; `i16` pixels-to-world at `0x04`;
  `i16` views at `0x08`, frames per view `0x0a`, ticks per frame `0x0c`, key frame `0x0e`, loop
  frame `0x10`; the 16.16 selection rectangle at `0x28..0x34`), then per frame a record of
  `46 + 2 * views` bytes: `i16` shadow map index first, the per-view `i16` instance indices at
  `+46`. Only `collection::displayed_views(n)` views are stored; `collection::mirror_view` gives the
  stored view a mirrored one copies.
- `sequence_header::sequence_headers` reads just the sequence headers, for the simulation, which
  needs timing (frames, ticks per frame, loop frame) but never pixels. An index past the count
  falls back to sequence 0 (`sequence_header::get_sequence`), as the original does.

A map's landscape `.256` holds, per submesh, a 256 x 256 colour index section and a shading
section (`collection::TerrainTexture`, 8 pixels per cell).

**`core`** (collection reference): FourCC `.256` at `0x00`, `i16` permutation count at `0x04`,
then per permutation `p` one colour per hue change `j` at `0x08 + 0x40 * p + 8 * j` (`u16` R, G,
B, flags). Objects draw `random % count` to pick a permutation at creation (a simulation draw:
see [simulation.md](simulation.md)).

### 3.4 `unit`, `mons`, `scen`, `obje`

**`unit`** (128 bytes): FourCC `mons` at `0x00`, FourCC `core` at `0x04`.
`UnitTag::from_bytes` picks the `mons` parser when the payload is at least 1024 bytes.

**`mons`** (1024 bytes), the fields `unit::UnitTag` decodes:

| offset | field |
|---|---|
| `0x04` | FourCC animation collection (`.256`) |
| `0x08` | 19 `i16` sequence slots (`unit::MONSTER_SEQUENCE_SLOTS`; 0 standing, 1 moving, ...; -1 none) |
| `0x3c` | FourCC death group (`prgr`) |
| `0x48` | 16 signed per-terrain bytes; negative = impassable terrain (`unit::MONSTER_TERRAIN_COSTS_OFFSET`) |
| `0x58` | impassable terrain mask (`unit::MONSTER_IMPASSABLE_TERRAIN_OFFSET`) |
| `0x5c` | per-terrain speeds (`unit::MONSTER_TERRAIN_SPEEDS_OFFSET`) |
| `0x6c`, `0x70` | `u32` vitality, movement speed |
| `0x78` | `u32` size |
| `0x84` | FourCC object definition (`obje`) |
| `0x88` | `i16` attack count (at most 4), attacks of 64 bytes from `0x8c`: projectile FourCC at `+4`, four `(u16 flags, i16 sequence)` pairs at `+0x0c` |
| `0x194`, `0x1a4` | FourCC death groups (`prgr`) |
| `0x19c`, `0x19e` | `u16` kill and hit-reaction damage ratio thresholds |
| `0x1a0` | FourCC impact group (`prgr`) launched when a projectile hits this monster |
| `0x1bc` | 10 FourCC sound slots (`unit::MONSTER_SOUND_SLOTS`) |
| `0x1f8` | 10 `i8` sound types the slots answer to (`unit::MonsterSoundType`) |

Monster flags `0x08` (flying) and `0x4000_0000` (speed decides passability) are
`unit::MONSTER_FLAG_FLYING` and `unit::MONSTER_FLAG_SPEED_DECIDES_IMPASSABLE`.

**`scen`** (128 bytes, `scenery::SceneryTag`): `u32` flags (`0x80` the cells under it become
impassable at map load), `core` at `0x04`, `obje` at `0x08`, destruction `prgr` at `0x1c`,
`i16` intact and destroyed sequence at `0x28`/`0x2a`; `raw_data` keeps all bytes.
`SceneryTag::collection_tag` holds the `core` and `SceneryTag::object_tag` the `obje`.

**`obje`** (64 bytes, `object::ObjectDefinition`, `object::ObjectPhysics`): `u16` flags at `0x00`
(1 selection frame, 2 vitality bar, 4 follows ground, 8 no object collisions), `i16` gravity
`0x02`, elasticity `0x04`, terminal velocity `0x06`, `u16` vitality and `i16` random extra at
`0x08`/`0x0a`, 8.8 scale and random extra at `0x0c`/`0x0e`, damage modifiers `0x10..0x22`,
damage threshold `0x30`.

### 3.5 `proj`, `prgr`

**`proj`** (256 bytes, `projectile::ProjectileDefinition`): `u32` flags (`0x2000` detonates at
creation, `0x80000000` detonates without a target), `.256` at `0x04`, three `i16` sequences
`0x08..0x0e`, detonation `prgr` `0x10`, `proj` `0x14`, `obje` `0x1c`, speed and random extra
`0x20`/`0x22`, velocity error `0x24`, chances at `0x2a`/`0x2c`/`0x2e`, `ligh` `0x34`, flight and
rebound `soun` `0x3c`/`0x40`, lifetime and extra `0x50`/`0x52`, homing turn rate `0x5a`, the
16-byte `projectile::DamageDefinition` at `0x68` (`projectile::PROJECTILE_DAMAGE_OFFSET`),
`arti` `0x78`, `prgr` `0x7c`/`0x80`, `lpgr` `0x88`.

**`prgr`** (512 bytes, `projectile::ProjectileGroupDefinition`): `u32` flags, `i16` part count at
`0x04`, mesh effect `0x08`, launch `soun` `0x0c`, `lpgr` `0x10`; 32-byte parts from `0x20`
(`projectile::ProjectileGroupPart`: `proj` at `+0x00`, flags at `+0x04`, threshold at `+0x14`,
alternative `proj` at `+0x18`).

### 3.6 `geom`, `mode`, `anim`

**`geom`** (`model::GeometryTag`): a 128-byte header: `u32` flags, texture collection FourCC at
`0x04`, `u16` material, vertex, surface and draw-after counts at `0x08..0x0e`, `i16` origin x, y,
z at `0x10`, table offsets (relative to the end of the header) at `0x1c` (materials, 64 bytes
each), `0x28` (vertices, 6 bytes: three `i16`), `0x34` (surfaces, 64 bytes) and `0x40`
(draw-after lists, `i16` words, each list ended by -1), bounds corners at `0x58`/`0x5e`.
Geometry units are the tag's `i16`s around its origin; model matrices are nine `i16` with
`0x200` = 1.0 (`model_hit`).

**`mode`** (`model::ModelTag`): a 64-byte header: flags, `geom` FourCC, then three counted
tables (vertex flags 4 bytes, texture permutations 64 bytes, cells 8 bytes) whose offsets at
`0x10`/`0x1c`/`0x28` are relative to the data block at `0x34`. A permutation
(`model::ModelPermutation`) is an `i16` colour table then one view byte per material (`0xff` =
untextured).

**`anim`** (`model_anim::ModelAnimationTag`): a flip-book of whole models, not a transform. A
1024-byte definition: `u32` flags, `i16` frame count `0x04`, `i16` ticks per frame `0x06`, then
16-byte frames from `0x08` (flags, `mode` FourCC at `+4`), forward and backward wrap `soun` at
`0x3c0`/`0x3c4`; per-frame shadow maps follow.

### 3.7 `soun`, `amso`

**`soun`** (`sound::SoundTag`, 64-byte header): `u32` flags (bit 3 narration), `i16` loudness,
`u16` play chance (of 65535), 8.8 frequency modifier, pitch lower bound and range, volume lower
bound and range (`0x08..0x10`), `i16` first subtitle index, `u32` sample data offset and size
(`0x14`, `0x18`, **from the start of the payload**), subtitle `stli` at `0x1c`, `u32` permutation
count, offset, size at `0x24..0x2c`. A permutation entry is 32 bytes (`u16` flags, `u16` skip
chance, name at `+6`). The sample data begins with one 32-byte `sound::SampledSoundHeader` per
permutation (channels at `0x08`, 16.16 rate at `0x0c`, packet count at `0x10`), then each
permutation's packets in order. Samples are Apple IMA4 ADPCM (34-byte packets of 64 samples,
`sound::decode_ima4_packet`) at 22050 Hz (`sound::ENGINE_SAMPLE_RATE`), mono or stereo;
`SoundTag::decode_permutation` gives interleaved 16-bit PCM.

**`amso`** (64 bytes, `sound::AmbientSoundTag`): flags, `i16` inner and outer radius (world
units), period lower bound and random extra (ticks), six `soun` slots at `0x0c` of which only
[0] (continuous loop) and [1] (periodic) are used, `i16` random sound radius at `0x24`.

### 3.8 `medi`, `conn`, `dmap`/`dtex`

**`medi`** (256 bytes, `media::MediaDefinition`): flags (bit 0 no reflections), `core` at `0x04`,
16 projectile groups at `0x18` (indexed by a media effect type: projectile, monster, ...),
reflection tint `0x58`/`0x5a`, local effect chance `0x62`, local `lpgr` `0x64`, wobble
amplitudes and frequencies `0x68..0x72`, local effect count and period `0x74`/`0x76`, reflection
transparency `0x78`.

**`conn`** (32 bytes, `connector::ConnectorTag`): flags, `core` at `0x04`, `i16` sequence
`0x08`, 8.8 attachment height fraction `0x0a`, segment length (world units) `0x0c`, damaged
sequence `0x0e`, runtime fields after.

**`dmap`** (`detail_map::DetailMap`, plugin-only): a 256-byte header of eight big-endian longs:
width and height in cells, then offsets of an RLE stream of one entry index per cell (`0x08`),
an RLE stream of two bytes per cell forcing per-triangle entries (`0x10`), and the table of 128
entries of 64 bytes (`0x18`; entry: `dtex` id, repeat byte at `+4`). RLE is `(value, count - 1)`
byte pairs (`detail_map::rle_expand`). **`dtex`**: a **little-endian** `u32` byte count at `0x10`
and a `pix0r1` stream at `0x14`. `mandrake_tag::pix0r1::decode` is a lossless predictive RGB codec read
as little-endian 32-bit words from the top bit down; the module keeps the original decoder's
quirks (fields straddling words take their high bits from the second word; byte arithmetic
wraps).

### 3.9 Text, fonts, interface and dialogs

**`stli` / `text`** (`stli::StringListTag::from_bytes`): either an offset table (`u16` or `u32`
count, offsets, NUL-terminated strings) or plain text split on CR, LF or NUL. For the engine's
exact splitting (any run of NUL/LF/CR ends a string, so empty lines are not strings), use
`mesh_links::string_list_lines`.

**`font`** (`font::FontTag`): a 64-byte definition: `u32` flags, `i16` ascent, descent, leading,
`i32` glyph count `0x0c`, glyph data offset and size `0x10`/`0x14`, four style font FourCCs at
`0x18` (style 0 italic, style 1 bold). Each glyph is a 16-byte header (eight `i16`: code, advance,
width, height, origin x, origin y, two runtime words) plus `width * height` coverage bytes padded
to even; coverage is **inverted** (`0x00` full ink, `0xff` none) and only its high nibble is
used. Styled text is Mac Roman bytes with `|` codes: `|p` plain, `|i`, `|b`, `|u`, `|l`, `|x`,
`|0`..`|9` and `|A`..`|Z` colours, `||` a literal bar. Localised layout strings come from
`stli in#1` (`font::TextLocale`). `font::wrap_string` and `font::draw_string` reproduce the
original line breaking and rasteriser.

**`inte`** (`interface::InterfaceDefinition`): interface `.256` at `0x00` and six font FourCCs at
`0x04..0x18`. Flat interface colours come from bitmap 0 of that collection (`stin`): pixel `i` is
the palette index of colour `i` (`interface::InterfaceColorTable`); missing pixels fall back to
palette entry 0.

**`bina`** (`dialog::DialogDefinition`): `u16` version, `u16` item count, `i16` rectangle `{top,
left, bottom, right}` (right/bottom exclusive), then 24-byte items (control type FourCC such as
`butt`/`stat`/`STAT`, `i16` at `+4`, `i16` item id at `+6`, four `i16` at `+8`, rectangle at
`+0x10`), then the items' NUL-terminated names in order. A `ditl` with the same id replaces the
names by item id (line `id - 1`, CR separated; `DialogDefinition::apply_ditl`). Items with a
left/right of 16384 or more are parked off-screen.

## 4. Tag files, patches and plugins

### 4.1 The search order (`mandrake_tag::tag_files`)

A game never reads one archive; it reads an ordered list of tag files where **later files
override earlier ones** by `(group, id)`. `mandrake_tag::engine_tag_files(game_dir, plugins)` builds
that list as the original 1.8.5 engine does, for a game that names `plugins` (a film's or a
network game's plugin list):

1. `tags/international small install`, `tags/international large install`
   (`tag_files::INTERNATIONAL_FILES`; they hold most interface tags and some effect
   collections the simulation draws from).
2. The foundation installs `tags/large install`, `tags/medium install`, `tags/small install`
   (`tag_files::FOUNDATION_FILES`), when present.
3. Patches: files in `plugins/` of header type 2 with version 2, 3, 4, 5 in that order
   (Patch 1.2 to 1.5).
4. The metaserver component (type 7) that `preferences/metaserver plugin` names
   (`tagset::chosen_metaserver`, default `tagset::DEFAULT_METASERVER`), if installed. Its tags are
   interface tags only.
5. `Patch 1.8 Interface` (type 2).
6. The named plugins (type 1), in the order the game lists them. A name not found is reported in
   `EngineTagFiles::missing_plugins`.

Plugins (type 1) are never searched unless named, so the same folder of plugins gives each film
exactly the tags it was recorded with. `EngineTagFiles::missing_patches` lists required patches
(`tag_files::REQUIRED_PATCHES`) that are absent; a tag set with any missing is not the original
game's (the client logs a warning at map load and goes on with what is there, so films may
then go out of sync). `Patch 1.5 Unity` (a type-1 TFL-units plugin,
`tag_files::TFL_ONLY_PATCH`) must be installed in the original but no Myth II game searches it.

A plugin is matched by its **file name** (its first 31 bytes, Mac Roman,
`tag_files::plugin_name`), compared ASCII case-insensitively, never by the header's name (they
can differ). Files in `plugins/` without an `mth2`/`dng2` signature are ignored.

Other lists:

- `tag_files::dialog_tag_files(game_dir, interface)`: the menus' list, `engine_tag_files` with no
  plugins, then the interface plugin (type 5, e.g. `tag_files::INTERFACE_800X600`). The interface
  plugin must never join the list the simulation reads.
- `mandrake_tag::engine_interface_tag_files`: `engine_tag_files` with the international files in front
  if missing.
- `tag_files::order_engine_tag_files`: the same ordering over names and 128-byte headers instead
  of a directory, used by the browser (`mandrake_web::film_replay::order_tag_files`).

`mandrake_tag::TagLookup::open(&files)` opens the list (unreadable files are skipped); `get(group, id)`
returns the payload from the last file that has it; `get_beside(group, id, beside)` prefers the
file that also holds `beside` (a metaserver collection next to its string list).
`TagArchive::with_overrides` is the equivalent as one merged archive.

### 4.2 Downloaded plugins

The client can download a network game's missing plugins into its own downloads folder
(`tag_files::set_downloads_dir`, never the game folder). `tag_files::downloaded_plugins` lists
type-1 archives there (files ending `.part` are in-progress downloads and skipped); they are
searched after the game's `plugins/`, so an installed file of the same name wins. The dedicated
server and the browser never set a downloads folder. See [networking.md](networking.md).

### 4.3 Menus' view of tag files (`mandrake_tag::mesh_links`)

The map lists need entry points and a few mesh header fields without loading a map:
`mesh_links::CampaignTags::open(game_dir)` reads the base files' and every plugin's entry
tags; each entry takes its campaign fields (`mesh_links::MeshLinks`) from the mesh **in the same
file**; duplicates in a set of files are dropped; a plugin's entries need their meshes present.
`CampaignTags::plugin_search(i)` and `TagSearch` give a lookup with that plugin over the base.
Everything is opened read-only. Loose tags under the game folder's `local/` are not read.

### 4.4 Tag-set identity (`mandrake_tag::tagset`)

Network games compare tag sets (see [networking.md](networking.md)). `tagset::plugin_entries`
lists `plugins/` as the original's plugin table does: types 1, 2, 5, 6, 7, sorted by file name
ignoring case, each with its file name (31 bytes), description, version, type and header
checksum as stored. Patches that are not one of the five required ones by checksum and name
(`tagset::REQUIRED_PATCHES`) are dropped, as are files whose checksum is `0xffffffff`; a
plugin's version bit 2 is cleared when its description is empty or looks like a URL, and the
TFL plugins' (`tagset::TFL_PLUGINS`) version is 0.

The foundation checksum (`tagset::foundation_checksum`, `tagset::foundation_checksum_of`) is

```
0x101 (tagset::FOUNDATION_SALT)
  ^ the header checksums of the five foundation files (tagset::FOUNDATION_NAMES, type 3)
  ^ every patch of version 2 ..= limit   (limit: 1, raised while a patch of the next version exists)
  ^ Patch 1.8 Interface
  ^ the chosen metaserver component (type 7)
```

and `None` when a foundation file is missing. The interface plugin is not part of it.

### 4.5 Robustness

Plugins can come from other players, so the readers treat files as untrusted: header and
directory parsing return errors on any inconsistency, every payload read is bounds-checked
against the real file length before allocating, and tag parsers check their tables against the
payload size. `crates/mandrake-tag/tests/plugin_file_fuzz.rs` mutates a small valid plugin thousands of
ways (deterministic RNG, no fuzz crate) and requires no panic and no oversized allocation.

## 5. The game folder

The code expects a Myth II 1.8.5 install. Natively its root is the "game directory"; the
client names it through its archive path (`--archive <game>/tags/small install`, the game
directory being that file's grandparent; see [configuration.md](configuration.md) for the
settings files). Tools and tests take it from the `MYTH2_DATA_DIR` environment variable; the
dedicated server from `data_dir` in its TOML (see [server.md](server.md)). In the browser it is
the folder the player picks, mounted at `/game` in the `MemoryVfs`.

```
<game>/
  tags/
    small install, medium install, large install          foundation files (type 3)
    international small install, international large install
  plugins/
    Patch 1.2 .. Patch 1.5, Patch 1.8 Interface          required patches (type 2)
    Patch 1.5 Unity                                       required by the original, not searched
    bagrada metaserver                                    metaserver component (type 7)
    Myth II Interface (800x600)                           interface plugin (type 5), optional
    <map and unit plugins>                                type 1, used only when a game names them
  preferences/
    metaserver plugin                                     the chosen metaserver component's name (bytes)
    game 2, sound, ...                                    the original's preference files
  recordings/                                             films (mandrake_session::films::FOLDER)
  saved games/                                            saves (mandrake_session::saves::FOLDER)
  veterans/                                               veteran files for network games
  cutscenes/                                              Smacker films named by mesh headers
```

Without `medium install` or `large install` the lists simply skip them, but the landscape
textures of many maps are in `large install`. Which folders the client writes into and how
films and saves are stored is in [films.md](films.md) and [session.md](session.md).

## 6. TFL formats (brief)

`mandrake_tag::tfl` reads Myth: The Fallen Lords 1.5 data beside Myth II's, chosen by the data's
format, never by a flag inside a Myth II parser: `.gor` archives (a 64-byte header at the start,
the directory of 64-byte `myth` tag headers at the end of the file; `tfl::gor`), the loose tags
of `tags/Local` (`tfl::loose`), the version-7 mesh with 8-byte cells (`tfl::mesh`), version-1
`geom` and version-4 `mode` (`tfl::model`), the 512-byte `mons` (`tfl::monster`), other
fixed-size definitions (`tfl::defs`), the menus' tags (`tfl::menus`) and the landscape shading
(`tfl::shading`). `TagArchive::open` recognises a `.gor` by its directory tiling the file's end and
presents it with the same API; `tag_files::open_tfl_tags(game_dir)` merges every data-holding
`.gor` in `tags/` (by file name) with `tags/Local` on top. `mandrake_tag::tfl::Game::detect` tells the
two games' folders apart. Details are in [tfl.md](tfl.md).

## 7. Inspecting data: `tools/m2-tag-dump`

`m2-tag-dump` is the workspace's archive inspector. Run it from the dev shell, e.g.
`cargo run -p m2-tag-dump -- "<game>/tags/small install" ...`.

With an archive and no subcommand:

| option | effect |
|---|---|
| (none) | table of every tag: index, group, id, offset, size, name |
| `-i` / `--info` | header and entry tags only |
| `-t GROUP` / `-n NAME` / `-l N` | filter by group, by name substring, limit rows |
| `-e ID [-t GROUP] -o FILE` | extract one payload raw (big-endian, as stored) |
| `--inspect-mesh`, `--inspect-coll`, `--inspect-unit`, `--inspect-strings`, `--inspect-proj`, `--inspect-prgr`, `--inspect-selection` | decode one tag with the `mandrake-tag` parser and print its fields (`--coll-archive` names the archive holding a unit's collection) |

Subcommands (most take the game directory and use `engine_tag_files`, so later files override
earlier ones as in a game): `dump-raw ARCHIVE GROUP ID OUT`, `inspect-mesh`, `inspect-coll`,
`inspect-unit`, `inspect-strings`, `export-obj` (terrain to Wavefront OBJ), `media-report`,
`media-lod`, `media-objects`, `lighting-report`, `colperm-report`, `combanim-report`,
`projfx-report`, `movemark-report`, `hud-report`, `mesh-flags`, `map-actions`,
`detail-map` and `unitanim-report`. `--help` on each lists its arguments. Writing an archive goes
through `mandrake_tag::write_monolith`.

## Verifying changes

**Self-contained tests** (no game data):

```
cargo test -p mandrake-common --lib
cargo test -p mandrake-tag --lib
cargo test -p mandrake-tag --test plugin_file_fuzz
```

- `mandrake-common`: `FourCC` formatting and parsing, `MemoryVfs` folders and shared reads,
  `NativeVfs` (reads the crate's own `Cargo.toml`), Mac Roman round trips.
- `mandrake-tag` library tests that build their input in memory: header round trips for
  `SBMonoHeader`, `SBTagHeader` and `SBEntryTag`; a synthetic two-tag archive read from bytes
  and from a `MemoryVfs` (`test_synthetic_tag_archive` in `crates/mandrake-tag/src/lib.rs`); override
  order (`overrides_win_in_file_order_and_keep_the_rest` in `archive.rs`); the writer round
  trip (`written_archive_reads_back_with_the_same_tags` in `archive_writer.rs`); synthetic
  string lists, meshes, collections, units, scenery and objects; `mesh_links` (entry fields,
  ordering, duplicates, read-only opening, built from in-memory archives); `pix0r1`,
  `interface`, `sequence_header`.
- Tests that need game data are not part of this tree: every shipped test passes with
  `MYTH2_DATA_DIR` unset. To check a parser against real data, use `m2-tag-dump` on your own
  Myth II 1.8.5 install (below), or write a local test that reads `MYTH2_DATA_DIR`.

**Round-trip a real archive with the writer.** Reading then writing must reproduce every
directory field the writer does not own and every payload byte. A short check, in a scratch
binary or test:

```rust
let a = mandrake_tag::TagArchive::open(&mandrake_common::NativeVfs::default_cwd(), path)?;
let tags: Vec<mandrake_tag::MonolithTag> = a.list_tags().iter()
    .map(|e| Ok(mandrake_tag::MonolithTag { header: e.header.clone(), data: a.get_tag_data(e)? }))
    .collect::<Result<_, mandrake_common::M2Error>>()?;
let bytes = mandrake_tag::write_monolith(a.header(), a.entry_tags(), &tags)?;
let b = mandrake_tag::TagArchive::from_bytes(bytes)?;
assert_eq!(b.tag_count(), a.tag_count());
for (x, y) in a.list_tags().iter().zip(b.list_tags()) {
    assert_eq!((x.tag_type, x.tag_id, &x.name), (y.tag_type, y.tag_id, &y.name));
    assert_eq!(a.get_tag_data(x)?, b.get_tag_data(y)?);
}
```

A shipped file is usually byte-identical after this unless its payloads are not laid out back
to back in directory order; compare with `m2-tag-dump -i` and the tag table to see the
difference. A plugin you write is checked by placing it in `plugins/`, listing it with
`m2-tag-dump`, and naming it in a game (`engine_tag_files(game, &["<file name>"])` must end with
it and report it in no missing list).

**Parser changes.** A tag parser feeds the simulation, so a change to a field it decodes can
change a film's outcome. After changing a parser the simulation reads (`mesh`, `.256` sequence
headers, `core`, `unit`, `mons`, `scen`, `obje`, `proj`, `prgr`, `geom`/`mode` bounds, `medi`,
map actions), replay films with your own data and check that they still play to their end with
no SYNC miss, as [films.md](films.md) describes. Display-only parsers (fonts, dialogs, bitmap
pixels, sounds) need a focused unit test and a look in the client.
