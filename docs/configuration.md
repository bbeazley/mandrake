# Configuration

The client has no Preferences screen. Its settings live in one TOML file, `mandrake.toml`,
with one table per kind of setting. The shipped values
are `crates/mandrake-client/mandrake.default.toml`, compiled in with `include_str!`; each module takes its
table of it (`conf::shipped_table`). The player's file is merged over the shipped one, so it only
needs the keys it changes, and a table left out keeps the shipped values. A bad value or an unknown
key is logged as a warning and skipped. It is never fatal.

| Table | What | Loaded by | Read again |
|---|---|---|---|
| `[controls]` (+ `controls.left_handed.toml`) | Myth II's key bindings and mouse options | `controls::load` | at the start |
| `[interface]` | windows, menus, cutscenes | `interface_config::load` | at the start |
| `[fog]` | the height fog | `fog_config::load_native` | live |
| `[lighting]` | real-time lighting per map | `lighting::load_native` | live |
| `[water]` | the enhanced water | `water_config::load` | at a map's load |
| `[display]` | loose settings and the command-line settings of a setup | `settings`, `conf::parse_args` | at the start |
| `[net]` | the multiplayer player | `settings::load_net_settings` | at the start |
| `[tfl.controls]` | TFL's key bindings (built-in presets in `controls_tfl.rs`) | `controls::tfl::load` | at the start |

A table's text is cut out of the file line by line (`conf::table_text`), so a diagnostic names the
file's own line.

**Which file.** `--conf FILE` picks it: a bare name (`--conf mandrake.modernretro.toml`) is looked
up in the config folder, a path with a folder is used as it is, and a named file that doesn't exist
is an error. Without `--conf` it is `mandrake.toml` in the config folder, which may be absent (the
shipped values). The config folder is `<base>/myth2-sourceport/` (`user_dirs::config_dir`), `<base>`
the first of these that is set: `$XDG_CONFIG_HOME`, then `$HOME/.config`, then (Windows only)
`%APPDATA%`. Headless runs read only a file given with `--conf`, so their pictures don't depend on
the machine. Two setups ship as examples: `mandrake.original.toml` and `mandrake.modernretro.toml`
(the web page's presets).

**What the client writes** goes to the file in use (`conf::set_values`), each value replaced where
it stands, so the player's comments and order survive: `[interface]` `control_bar` at quit,
`[display]` `ui_zoom` (Ctrl +/-) and the `[net]` table (the network dialogs).

**Writing a starting file.** `--write-default-conf PATH` writes the shipped file, every table with
its comments, and exits.

**The old files.** Before the single settings file each table was a file of its own (`controls.toml`,
`interface.toml`, `fog.toml`, `lighting.toml`, `water.toml`, `settings.toml`, `tfl/controls.toml`).
When `mandrake.toml` doesn't exist and any of them does, the client builds `mandrake.toml` from them
once (`conf::migrate`), says so on stdout, and leaves them in place; they are no longer read.

**Order of precedence, lowest first:** the shipped file, the player's file, the command-line flags.

---

## Controls (`[controls]`)

These are the original game's "Configure Controls" dialog with its built-in "Right Handed" set.
Module: `crates/mandrake-client/src/controls.rs` (`Controls`, `ControlAction`, `Binding`, `MythKey`,
`ControlOptions`).

### Top level

- `preset`: only in a user file. `"right_handed"` (the default) or `"left_handed"`. With
  `"left_handed"`, `controls.left_handed.toml` (the original "Left Handed" set) is applied first
  and the user's bindings go over it. The left-handed set changes only the camera keys, which move
  to the keypad: `Keypad 8/5/1/3` move, `Keypad 4/6` turn, `Keypad 7/9` orbit, `Keypad Plus/Minus`
  zoom. `controls::PRESETS` lists the names.

### `[controls.bindings]`

The forms are `action = "Key"` and `action = ["Key", "Key"]`. `[]` unbinds the action. Key names
are the original dialog's: `"W"`, `"Backquote"`, `"Return"`, `"Up Arrow"`, `"Keypad 8"`, `"F1"`.
`MythKey::from_name` parses them, and its table is `KEY_NAMES` in `controls.rs`. A binding can
require modifiers: `"Ctrl+Q"`, `"Shift+X"`, `"Alt+X"`. The modifier keys themselves are named
`"Control"`, `"Shift"`, `"Option"` and `"Alt/Command"`. `Controls::duplicate_warnings` reports a
key bound twice.

| Key | Default | Meaning |
|---|---|---|
| `move_forward` / `move_backward` | `W` / `S` | camera moves forward/back |
| `move_left` / `move_right` | `Z` / `X` | camera moves sideways |
| `turn_left` / `turn_right` | `A` / `D` | camera turns |
| `orbit_left` / `orbit_right` | `Q` / `E` | camera orbits the view centre |
| `zoom_in` / `zoom_out` | `C` / `V` | zoom; with Shift, jump to the next zoom preset |
| `clear_selection` | `Backquote` | deselect all |
| `rotate_preset` | `F` | cycle the selection's formation |
| `stop_units` | `Space` | stop |
| `select_all` | `Return` | select every unit of the same type on screen |
| `clear_preset` | `Delete` | clear a numbered group |
| `recall_set_preset` | `Alt/Command` | modifier: hold it with a number key to store a group |
| `center_selection` | `H` | centre the camera on the selection |
| `scatter_units` | `B` | scatter |
| `retreat_units` | `R` | retreat |
| `attack_ground` | `Control` | modifier: hold it while clicking the ground |
| `special_ability` | `T` | special ability |
| `rename_units` | `N` | rename |
| `chat_message` | `Y` | open the chat line |
| `detach_units` | `Backslash` | give the selection to a teammate (network games) |
| `guard_units` | `G` | guard |
| `overhead_map` | `Tab` | overhead map |
| `pause_game` | `Escape` | pause dialog |
| `display_chat` | `Up Arrow` | show the chat backlog |
| `taunt` | `U` | taunt |
| `rotate_units_right` / `rotate_units_left` | `Right Arrow` / `Left Arrow` | turn the selection's facing |
| `inventory` | `I` | use an inventory item |
| `preset_selection_1` ... `preset_selection_0` | `1` ... `0` | recall a numbered group |
| `form_break_alliance` | `Plus` | form or break an alliance (bound in the original game, hidden from its dialog) |
| `orbit_zoom_modifier` | `Keypad 0` | modifier for orbit/zoom (bound in the original game, hidden from its dialog) |
| `quit` | `Ctrl+Q` | quit the client (this port only: Escape pauses, as in the original game) |
| `game_speed_down` / `game_speed_up` | `F1` / `F2` | game speed; with Shift, normal speed or stopped |
| `screenshot` | `F13`, `Alt+F12` | save a PNG and a text file with the camera to the pictures folder's `Screenshots` |
| `toggle_fps_display` | `Forward Delete`, `Shift+Delete` | frame-rate display at the top right |
| `toggle_fullscreen` | `Alt+F11` | switch between borderless fullscreen and a window for this run |
| `quicksave` | `F5`, `F12` | write "Quicksave 1".."Quicksave 5" in turn, in a game that can be saved |
| `toggle_control_bar` | `Shift+F6` | show or hide the control bar; the 3D view takes its space |
| `show_scores` | `F7` (held) | the score table while held |
| `toggle_scores` | `Shift+F7` | keep the score table on or off |

The fixed keys at the end of the table are hard-wired in the original game. Here they can be
rebound like any other action.

### `[options]`

These are the dialog's pop-ups and check boxes (`ControlOptions`).

| Key | Default | Values / meaning |
|---|---|---|
| `right_click` | `"gesture_move"` | `ignored`, `gesture_move` (drag to set facing), `attack_ground`, `deselect_units` |
| `mouse_wheel` | `"zoom"` | `ignored`, `move`, `zoom`, `cycle_presets`, `cycle_inventory`. The original game defaults to `ignored`; this port ships `zoom` |
| `caps_lock` | `"selection_boxes"` | `ignored`, `selection_boxes` (every unit's frame and bar), `health_bars`, `player_stats` (score table), `chat_backlog` |
| `horiz_mouse_movement` | `"move"` | what the left and right screen edges do: `ignored`, `move`, `rotate`, `orbit` |
| `enable_mouse_movement` | `true` | edge scrolling |
| `top_corners_rotate_camera` | `false` | the top corners rotate the camera |
| `bottom_corners_orbit_camera` | `false` | the bottom corners orbit the camera |
| `left_click_doesnt_gesture` | `false` | a left drag never becomes a facing gesture |
| `tfl_mouse_actions` | `false` | The Fallen Lords' mouse behaviour |
| `alternate_preset_recall` | `false` | the original game's alternate group-recall behaviour |
| `reverse_axis`, `use_control_bar_inventory`, `right_click_through_ui`, `tilde_ignores_inventory`, `system_keyboard_layout` (`true`), `alternate_unit_selection` | `false` unless noted | read and kept, but **no client code acts on them yet** |

### TFL controls

The Fallen Lords uses the same `Controls` type. It starts from TFL's own built-in sets:
`controls::TFL_PRESETS` = `tfl_right_handed`, `tfl_left_handed`, in `controls_tfl.rs`. The player's
table is `[tfl.controls]`, and in it `preset = "left_handed"` selects TFL's left-handed set.

---

## Interface (`[interface]`)

Module `interface_config.rs`, type `InterfaceConfig`.

| Key | Default | Meaning |
|---|---|---|
| `fullscreen` | `true` | Open borderless fullscreen on the current monitor. The display mode doesn't change. `false` opens a `--width` x `--height` window. `Alt+F11` toggles for one run. Headless runs, `--screenshot` and `--frame-budget` always use a window. |
| `control_bar` | `true` | Show the control bar at the bottom. With `false` the 3D view takes the strip and the bar's keys still work. `Shift+F6` toggles it. At quit the client writes this key back to the settings file in use (`interface_config::save_at_exit`) and keeps the file's other lines. A headless run writes nothing. |
| `menu_filter` | `"sharp"` | How the menus and pregame screens are scaled up to the window: `"sharp"` or `"lanczos2"`. `--ui-filter` wins. |
| `lanczos_sharpen` | `0.4` | Sharpening of the `lanczos2` filter, 0 (none) to 1. |
| `cutscene_filter` | `"fsr1"` | How cutscenes are scaled up: `"fsr1"` (FidelityFX Super Resolution 1: an edge-adaptive upscale, then sharpening), `"sharp"`, `"lanczos2"`, `"linear"` or `"nearest"`. `--cutscene-filter` wins. |
| `cutscene_hd` | `true` | Play a player-supplied HD version of a cutscene when one exists (see [audio.md](audio.md#cutscenes)). `--cutscene-hd` wins. |
| `fsr_sharpness` | `0.5` | FSR1 sharpening in stops, 0 (sharpest) to 2. Each stop halves it. |
| `menu_art` | `"800x600"` | Menu art: `"800x600"` (the original game's "Interface (800x600)" plugin) or `"stock"` (the 640x480 art). Only the menus read it. |
| `status_bar` | `"original"` | The Fallen Lords' status bar: `"original"` (640 pixels, centred) or `"fit_width"` (both ends at the screen edges, the middle tiled between them). |

---

## Height fog (`[fog]`)

Module `fog_config.rs` (`FogConfig`, `FogLayer`, `FogSettings`), drawn by `crate::fog`. This applies
with `--fog on`, which is the default unless `--sampling linear` is used. With `--fog engine` the
original game's depth fog is drawn instead and this table is ignored.

The values are resolved lowest first: the shipped `[fog.default]`, the shipped `[fog.map.<mesh>]`,
the player's `[fog.default]`, the player's `[fog.map.<mesh>]`, then the `--fog-*` flags. `FogConfig::resolve_in`
does this. `<mesh>` is the map's four-character mesh tag id, for example `cr01`. A map whose mesh
tag has a fog density gets height fog. An explicit `[map.<mesh>]` table, or any `--fog-*` flag,
turns fog on for a map without one. The Fallen Lords' maps use `[tfl.map.<mesh>]`, because the two
games' tag ids overlap. TFL maps have fog only when such a table names them.

Keys of `[default]`, `[map.*]` and `[tfl.map.*]`:

| Key | Default | Meaning | Flag |
|---|---|---|---|
| `opacity` | `0.5` | How much the fog hides, looking straight down at its base height, 0 to below 1 | `--fog-opacity` |
| `falloff` | `0.25` | The height over which the fog thins to about a third, as a share of the map's lowest-to-high ground (> 0) | `--fog-falloff` |
| `base` | `0.0` | The fog's base, in cells above the map's lowest ground | `--fog-base` |
| `color` | map tag's colour | `[r, g, b]`, each 0 to 1. A black tag colour falls back to a mist colour | `--fog-color R,G,B` |
| `patchiness` | `0.7` | 0 is even fog, 1 leaves clear gaps | `--fog-patchiness` |
| `patch_size` | `6.0` | Patch size in cells (> 0) | `--fog-patch-size` |
| `speed` | `0.35` | Patch drift, cells per second | `--fog-speed` |

Shipped per-map tables: `cr01`, `84fd`, `12mp`, `13td`, `89sn`. `--fog-time SECONDS` sets the
patch clock, so a headless frame is reproducible.

---

## Lighting (`[lighting]`)

Module `lighting.rs` (`LightingConfig`, `LightingLayer`, `MapLighting`). This is a prototype of
real-time lighting, and it is off unless `--lighting on` is given. The original maps are lit in
their textures, so a map without a `[map.<mesh>]` table stays unlit even with `--lighting on`.
Giving `--lighting-darkness` or `--lighting-ambient-color` turns lighting on for any map. TFL maps
use `[tfl.map.<mesh>]`, and TFL gets only the ambient light.

| Key | Meaning | Flag |
|---|---|---|
| `darkness` | Ambient darkness factor, 0 to 1. Lower is darker | `--lighting-darkness` |
| `ambient_color` | `[r, g, b]` ambient tint, each 0 to 1 | `--lighting-ambient-color R,G,B` |
| `light_intensity` | Point-light intensity of projectiles and fire (>= 0) | `--lighting-light-intensity` |

Shipped: `[lighting.map.cr01]` (0.95, `[0.25, 0.32, 0.45]`, 0.65) and `[lighting.tfl.map.03di]` (0.85,
`[0.55, 0.65, 0.85]`).

---

## Enhanced water (`[water]`)

Module `water_config.rs` merges the table into `mandrake_render::water::WaterSettings`. It applies only
with `--water enhanced` (or `[display]` `water = "enhanced"`), and `--water-maps 85tl,...` limits
that to the listed meshes; `[water.map.<mesh>]` tables set keys for one map. See [rendering.md](rendering.md#enhanced-water) for
what each stage draws. All lengths are in cells (512 world units) unless they say metres.

**Colour**
| Key | Default | Meaning |
|---|---|---|
| `color_mode` | `"map"` | `"map"`: each cell's colour is the depth²-weighted average of the map's painted water within `color_radius`. `"color"`: `water_color` everywhere |
| `color_scale` | `1.0` | Scale of the map's colour (below 1 is darker) |
| `color_radius` | `4.0` | Averaging radius, cells |
| `water_color` | `[0.06, 0.12, 0.14]` | The colour for `color_mode = "color"` |

**Physical model** (`physical = true`)
| Key | Default | Meaning |
|---|---|---|
| `physical` | `true` | Colour from absorption and in-scattering. `false` uses the hand-tuned curve below |
| `meters_per_cell` | `1.589` | Scale: a human unit is about 1.8 m |
| `pure_absorption` | `[0.45, 0.07, 0.02]` | Pure water's absorption per metre, RGB |
| `turbidity` | `1.0` | Particle scattering per metre (0 pure, 0.1 clear sea, 1 coastal, 5 murky) |
| `turbidity_absorption` | `0.05` | Particle absorption per metre per unit of turbidity |
| `backscatter_ratio` | `0.019` | Share of particle scattering that goes back toward the eye |
| `scatter_tint` | `true` | Tint the backscatter with the map's water hue |
| `bed_blur` | `0.25` | Blur of the bed seen through the water, cells per cell of path |
| `shore_swash` | `1.0` | How much waves move the waterline (0 is a still line) |
| `min_depth` | `0.03` | Water thinner than this (metres) counts as dry |

**Refraction and ripples**
| Key | Default | Meaning |
|---|---|---|
| `refraction` | `true` | Refract the eye ray at the surface (index 1.33) |
| `refraction_strength` | `1.0` | Scale of the refracted displacement |
| `ripples` | `true` | Interactive wave-equation ripples from units and splashes (client-side only) |
| `ripple_strength` | `1.0` | Disturbance height scale |
| `ripple_damping` | `0.8` | Share of a ripple's height lost per second |

**Wind waves, gusts, shelter** (generated; one of the three named exceptions to the art rule)
| Key | Default | Meaning |
|---|---|---|
| `wind_ripples` | `true` | FFT wind-wave cascades in the surface normal |
| `ripple_wind_scale` | `1.5` | Ripple wind, m/s per m/s of the game's wind |
| `ripple_calm` | `1.5` | Ripple wind with no game wind, m/s |
| `ripple_min_wavelength` / `ripple_max_wavelength` | `0.03` / `6.0` | Spectrum's soft cuts, metres |
| `gusts` | `true` | Drifting gust patches that scale the ripples |
| `gust_scale` | `12.0` | Patch size, metres |
| `gust_strength` | `0.7` | 0 is none, 1 goes from glassy to double |
| `gust_speed` | `0.5` | Drift, m/s per m/s of ripple wind |
| `shelter` | `true` | Fetch-limited waves: small water has short waves |

**Bed and shore**
| Key | Default | Meaning |
|---|---|---|
| `river_bed` | `true` | Repaint the painted water in the colour map as a bed of the map's own dry ground (cached on disk, see below) |
| `wet_sand` | `true` | Darken the terrain just above the waterline and give it some sheen |
| `wet_height` | `0.1` | Height of the wet band, cells |
| `wet_darken` | `0.35` | Darkening at the waterline (0 none, 1 black) |
| `wet_specular` | `0.5` | Wet sheen as a share of the water's |
| `wet_distance` | `1.5` | Sideways fade of the wet band, cells (at most 2) |

**Hand-tuned curve** (only with `physical = false`, except where noted)
| Key | Default | Meaning |
|---|---|---|
| `absorption` | `[1.2, 0.9, 0.7]` | Absorption per cell of eye path, RGB |
| `shallow_tint` | `0.3` | Water-colour share in the shallows |
| `deep_max` | `0.85` | Largest water-colour share |
| `surface_opacity` | `0.15` | Opacity of the surface's own colour |
| `shore_fade` | `0.15` | Depth over which reflections fade in |
| `surface_fade` | `0.3` | Depth over which surface colour and sheen fade in (all modes) |
| `wave_depth` | `0.5` | Depth at which the original wobble reaches full height (all modes) |
| `swell` | `0.1` | Share of the original z wobble kept (all modes) |
| `normal_strength` | `1.0` | Wobble slope in per-pixel normals (all modes) |

**Reflections and glint**
| Key | Default | Meaning |
|---|---|---|
| `reflection_strength` | `1.0` | Scale of reflections over the Fresnel term |
| `reflection_distortion` | `1.0` | How far waves shift a reflection |
| `reflection_blur` | `0.05` | Blur of reflections on rough water |
| `reflection_min` | `0.55` | (`physical = false`) Reflection share looking straight down |
| `fresnel_power` | `2.0` | (`physical = false`) Fresnel exponent |
| `specular` | `0.25` | Sun glint strength (0 is off) |
| `glint_sparkle` | `0.25` | Sparkle points at the highlight's fringe (0 is off) |
| `glint_facet` | `0.03` | Wave facet size, metres |
| `glint_lifetime` | `1.2` | Sparkle lifetime, seconds |
| `glint_size` | `1.5` | Sparkle radius, pixels |
| `shininess` | `64.0` | Specular exponent |
| `sheen_tint` | `0.0` | Tint of the glint by the map's light colour (0 white, 1 the light colour) |
| `light_fit_min_r2` | `0.5` | The light direction is fitted from the map's baked shading. Below this R², `light_fallback` is used |
| `light_fallback` | `[26.1, -134.3]` | `[elevation, azimuth]` in degrees, or `"off"` |
| `light_direction` | unset | An explicit `[x, y, z]` toward the light, in render space (y up). Overrides the fit |

The river-bed repaint is cached at `$XDG_CACHE_HOME/myth2/water-bed/` (else `~/.cache/...`),
keyed by a hash of its inputs (`water_bed.rs`).

---

## Client settings (`[display]` and `[net]`)

Module `settings.rs`. The client writes these values itself, and the player may edit them. Values
are spliced into the existing text, so hand edits survive. The client never writes the original game's
preference files, and `settings::save_progress` refuses to write inside the game directory.

| Key | Meaning | Flag that wins |
|---|---|---|
| `ui_zoom` | Interface zoom in percent: 100, 125, 150, 175, 200, 250, 300 or 400. Written by Ctrl +/- | `--ui-scale` |
| `detail_textures` | `true` draws a plugin's landscape detail textures (default off) | `--detail-textures` |
| `vsync` | `true` waits for the display refresh (default off) | `--vsync` |
| `fps_cap` | `"auto"` (default: monitor refresh minus 3 %, or 240 if unknown), a number, or `"off"` | `--fps-cap` |
| `fog`, `lighting` | Both games' look switches, as `--fog` (`on`, `off`, `engine`; `true`/`false`) and `--lighting`. Left out, each game keeps its default: Myth II fog by `--sampling` (on, `engine` with `linear`) and lighting off; TFL both on. `engine` in TFL is TFL 1.5's fog: none | `--fog`, `--lighting` |
| `detail_shadows` | TFL only: `false` draws TFL 1.5's shadow bitmaps instead of Myth II 1.8.5's detail shadows (default on). Myth II always draws its detail shadows, as 1.8.5 does | `--detail-shadows` |
| `[display.tfl]` `smoothing` | `false` draws TFL tick by tick, as TFL 1.5 does. The default interpolates as Myth II does | none |
| `[display.tfl]` `hover_frames` | `false` frames only the selection in TFL. The default also frames the unit under the pointer | none |
| `allow_downloads` | `false` never downloads a game's missing plugins from a dedicated server (default `true`) | none |
| `plugin_download_max_mb` | Largest plugin download, MB (default 512) | none |
| `ui_scale`, `ui_filter`, `sprite_filter`, `sampling`, `lighting`, `fog`, `detail_shadows`, `water`, `water_maps` | The command-line settings of a whole setup (`conf::PRESET_KEYS`), each as its flag takes it (`true`/`false` for on/off; a list for `water_maps`) | the flag of that name |
| `[net]` `name`, `team`, `address`, `port`, `icon`, `colors` | The player's network identity: name and team (Latin-1, up to 31 bytes), last address and port, icon number, eight colour numbers. Written by the network dialogs (`settings::save_net_settings`) | `--net-name` |

In the config folder, `progress.txt` holds the campaign progress (`mandrake_session::Progress::to_text`). On a
first run with no such file, progress is imported once from the original game's
`preferences/game 2`, read-only. Plugin downloads go to `<config>/myth2-sourceport/downloads/`.
