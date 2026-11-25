# The interface: menus, dialogs and the in-game bars

The menus and pregame screens are the original game's own dialogs. They are built from its tags and
behave the way the original 1.8.5 engine's dialogs do. They sit behind a **boundary**, so that a
different interface can replace them without touching the game flow. Three crates are involved:

| Crate | Role |
|---|---|
| `mandrake-session` | The screen flow as a state machine (no window, GPU or sim): `Session`, `Screen`, `Action`, `Request`, `GameEvent`, and the boundary trait `FrontEnd`. The engine half of the spec covers it ([session.md](session.md)). |
| `mandrake-dialogs` | The original dialogs: `EngineDialogs` implements `FrontEnd`. It does software drawing only, with no wgpu. |
| `mandrake-client` | Runs a `Session`, feeds it input, carries out its `Request`s, and draws the front end's picture with `mandrake-render`. |

## The boundary

Defined in `crates/mandrake-session/src/front_end.rs` and re-exported from `mandrake_session`:

```rust
pub trait FrontEnd {
    fn frame(&mut self, screen: &Screen<'_>, input: &FrameInput<'_>) -> FrameOutput;
    fn modeless_rect(&self) -> Option<[i32; 4]> { None }
}
```

- `FrameInput` carries `now_ms`, `size`, `events: &[UiEvent]`, `sound_busy` and `shift_held`. The
  session has no clock, so a front end counts its own 30 Hz ticks from `now_ms`.
- `UiEvent` is one of `PointerMoved { x, y }`, `Button { left, pressed }`,
  `Key { key: UiKey, pressed, repeat, shift, alt }`, `Text(char)` or `Wheel(i32)`.
- `Screen` (`mandrake_session::Screen`) is plain data borrowed from the session. Its variants are
  `MainMenu`, `NewGame`, `Loading`, `Briefing`, `Playing { overlay }`, `CampaignEnd`, `Postgame`,
  `Films`, `FilmReport`, `SavedGames`, `Restoring`, `Cutscene`, `Net(NetScreen)`, `Tfl(..)` and
  `Quit`.
- `Overlay` lists the dialogs that open over a running game: `Hints`, `Paused`, `SaveGame`,
  `SavedGames`, `NewGame` and `UnitTrading`.
- `FrameOutput` carries `actions: Vec<Action>`, which are the player's choices (for example
  `StartLevel`, `ViewFilm`, `OpenSave`, `HostGame`, `SetReady`, `Resume`), and
  `sounds: Vec<SoundCue>` (music, narration, ambience, fades).

The client applies each action with `Session::act`. It then takes the session's work with
`Session::take_requests()`, which returns `Request`s such as `LoadGame(GameStart)`, `BeginPlay`,
`SetPaused`, `DropGame`, `ListFilms`, `SaveFilm`, `SaveGame`, `Quicksave`, `PlayCutscene`,
`Net(NetRequest)` and `Exit`. When the work is done it reports back with `Session::game_event`,
passing a `GameEvent` such as `Loaded`, `LoadFailed`, `MapOver { report, veterans }`, `FilmsListed`,
`SavesListed`, `GameSaved` or `Net(NetEvent)`.

The flow (menu, briefing, loading, playing, postgame, next level) lives entirely in `Session`. The
client never decides which screen comes next.

### Drawing

`mandrake-dialogs` draws each dialog in software into an `mandrake_tag::font::Bitmap32` and describes the
result as data: `EngineDialogs::picture(size)` returns an `InterfacePicture`, defined in
`mandrake_view::interface_picture` and re-exported by `mandrake_dialogs`:

- At the menus it returns `InterfacePicture::Menu(MenuCanvas { .. })`: one canvas at the dialogs'
  own size (640x480, or 800x600 with that interface plugin), with the briefing's scrolling text as
  a separate layer.
- Over a game it returns `InterfacePicture::Layers { layers, .. }`: one bitmap per open dialog, each
  with its rectangle.

The GPU side is `mandrake_render::DrawInterface` (`fn draw(&mut self, frame: &mut InterfaceFrame<'_>)`)
and `mandrake_render::InterfacePresenter`. These upload the picture and scale it to the window with the
menu filter (`sharp` or `lanczos2`). The client glues the two halves together in `DialogsWindow`
(`crates/mandrake-client/src/app/shell_menu.rs`), which implements both traits. The client holds it as
`Box<dyn WindowFrontEnd>`, where `WindowFrontEnd` is `FrontEnd + DrawInterface`.

### Coordinates

- At the menus, `FrameInput::size` and the pointer coordinates are in **window** pixels.
  `EngineDialogs` maps them to its canvas with `mandrake_view::canvas_fit::CanvasFit`, which centres the
  canvas on black.
- Over a game, they are in the map's **logical interface** pixels (see `ui_scale::UiScale` in
  [rendering.md](rendering.md#interface-scale)).

## How a replacement interface plugs in

Implement `mandrake_session::FrontEnd` and `mandrake_render::DrawInterface` on one type, then return it from
`engine_front_end` in `crates/mandrake-client/src/app/shell_menu.rs` instead of
`DialogsWindow::new(EngineDialogs::new(..))`. That function is the only place the client names
`EngineDialogs`, and both `Menu::new` and `OwnPauseDialog::new` take what it returns.

A replacement must:

- read `Screen`, including the `Overlay` and `NetScreen` data;
- return `Action`s and `SoundCue`s;
- count its own ticks from `now_ms`;
- map pointer coordinates itself, as described above;
- leave the game flow to `Session`.

`crates/mandrake-session/tests/second_front_end.rs` is a minimal second front end that drives the whole
flow. Read it first.

A replacement does **not** cover these, which stay in the client:

- the in-game bars, overhead map, console and chat line (below);
- the key that opens the pause dialog (`app/input.rs` turns it into a `DialogAsk`);
- **The Fallen Lords' menus.** They use their own session type and front end:
  `mandrake_session::tfl::TflSession`, plus `mandrake_dialogs::tfl::TflDialogs`, whose
  `frame(&TflSession, &MenuInput) -> MenuOutput` returns `TflAction`s rather than going through
  `FrontEnd`. Replacing them means changing `crates/mandrake-dialogs/src/tfl.rs` and
  `crates/mandrake-client/src/tfl_window.rs` too.

## mandrake-dialogs internals

**One frame of `EngineDialogs`** (`impl FrontEnd for EngineDialogs` in `src/lib.rs`):

1. Compute `ScreenKind::of(screen)`.
2. If the kind changed, clear the `DialogStack` and call `screens::open(screen, ..)`, which returns
   a `Box<dyn OpenScreen>`.
3. Map the pointer to canvas or dialog pixels.
4. Pass the events to `DialogStack::event`. This collects `Hit { dialog, item }` values.
5. Run `DialogStack::tick`.
6. Call `OpenScreen::frame(screen, &mut ScreenFrame { .. })`. Here each screen turns hits into
   `Action`s and `SoundCue`s.
7. Redraw the dialogs that changed and compose the canvas.

**Resources** (`src/resources.rs`, `Resources`). The crate opens no files. `Resources::new` takes an
`mandrake_tag::TagLookup` that the client builds with `mandrake_tag::tag_files::dialog_tag_files(game_dir,
interface)`. Each dialog comes from these tags:

| Tag | Content |
|---|---|
| `bina` | Layout, parsed into `mandrake_tag::DialogDefinition` |
| `ditl` | Item names and captions |
| `.256` | Art |
| `inte` | Interface fonts |
| `font` | Fonts |
| `stli` | String lists |
| `text` | Text bodies |
| `baie` | Dialog collection: button shapes, backgrounds, colours |

`Resources::dialog(id)` loads one dialog.

**Core** (`src/core.rs`).
- `Dialog` is built with `Dialog::new(def, id, flags, res)`. Its methods include `set_enabled`,
  `set_visible`, `set_focus`, `press_item`, `set_background` and `redraw`.
- `DialogStack` provides `push`, `pop`, `top`, `find_mut`, `event`, `tick`, `place` and
  `redraw_with_static`.
- The flags are in `dialog_flags`.

**Controls** (`src/controls/`).
- Every control implements `ControlProc`, which has `draw`, `click`, `key`, `text`, `focus`, `idle`
  and `wheel`.
- `Control::new` picks a `Behaviour` from the item's four-character type.
- The control types are: buttons, check boxes and radio buttons (`button.rs`); static text; bitmaps
  with crossfade; lists (`list.rs`); edit and password fields; the scrolling briefing text
  (`prol.rs`); tabs; the postgame report (`repo.rs`); the carnage graph and tables (`carnage.rs`);
  snapshots; progress bars; the colour picker; the icon pop-up; number fields (`numb.rs`); lobby
  rows; and missing-plugin rows.

**Screens** (`src/screens/`). Each file has an `open` function:

| File | Screen |
|---|---|
| `main_menu` | Main menu |
| `new_game` | New Game, including over the game |
| `briefing` | Narrated pre-level briefing (`BriefingStyle` for TFL) |
| `loading` | Sound only, while the shell fades |
| `hints` | Mission objectives over the game |
| `paused` | Pause dialog (Resume, Restart, Quit; Rehost in network games) |
| `save_game` | Save Game and its replace question |
| `saved_games` | Continue Saved Game |
| `restoring` | Progress bar while a save's film replays |
| `campaign_end` | Campaign epilogue |
| `postgame` | Level epilogue |
| `films` | Saved Films |
| `film_manage` | Delete and rename films |
| `save_film` | Name prompt for saving a film |
| `film_report` | Carnage report |
| `unit_trading` | Modeless network unit trading (the only `modeless_rect` user) |
| `net_player` | Network player setup |
| `net_join` | Host, join by address, LAN list |
| `join_failed`, `lobby_failed`, `plugins_missing`, `plugins_download` | Join and plugin notices |
| `lobby` | Lobby |
| `game_options` | Host's game options |
| `net_postgame` | Network postgame |
| `end_barrier` | Wait-for-everyone at a network game's end |

Some kinds have no dialog: `Playing` without an overlay, `Cutscene`, `Tfl` and `Quit`. For these
`screens::open` returns `None`. `answer_to_unbuilt_screen` in the client answers
`Screen::CampaignEnd` with `Action::NarrationDone` when no dialog takes it.

**Other modules.**
- `slices.rs` draws three-slice backgrounds that stretch one painting to a wider dialog
  (`ThreeSlice`, `draw_three_slice`).
- `fade.rs` holds the screen-fade timings `fade::MYTH_II`: a 2 s fade to black and a 0.5 s fade in.

## The shell in mandrake-client

The window side has three layers:

- **The shell** (`app/shell.rs`): `Shell` and `ShellHost` own the window, the GPU, the audio
  device, fullscreen, the frame cap and screenshots.
- **The scene** (`app/game_scene.rs`): the game inside the window is a `GameScene`. That is either
  `Myth2Scene` (`app/myth2_scene.rs`) or `TflApp` (`tfl_window.rs`).
- **Each frame**, `Myth2Scene::session_frame` (`app/shell_game.rs`) runs:
  1. `Menu::step` (`app/shell_menu.rs`) builds the `FrameInput` from the queued `UiEvent`s. It
     calls `front_end.frame(&session.screen(), &input)`, applies the actions with `Session::act`,
     sends the sound cues to `MenuSound`, and returns `session.take_requests()`.
  2. `Myth2Scene::perform` carries out each `Request`:
     - `LoadGame` starts a `LoadFlow`;
     - `BeginPlay` and `SetPaused` go to the map (`ClientApp`);
     - `DropGame` drops the map;
     - the film and save requests read and write the folders;
     - `PlayCutscene` plays a cutscene;
     - `Net(..)` goes to `app/shell_net.rs`, which drives an `mandrake_net::endpoint::Endpoint`;
     - `Exit` quits.
  3. The scene steps the load, routes input, and draws.

**Loading and fades.** `LoadFlow` (`app/shell_game.rs`) runs through `Idle`, `FadingOut`, `Start`,
`ForMap`, `AttachGpu`, `Restoring` and `FadingIn`, and `LoadFlow::alpha()` gives the opacity of the
black. A saved game loads by replaying its film to the save point under `Screen::Restoring`.

**End of a map.** `Myth2Scene::map_over` sends `GameEvent::MapOver` and drops the map. The session
then shows the postgame, the campaign end or the film report.

**Over the game.**
- The open dialogs are drawn inside `ClientApp::render_frame` (`app/render.rs`) as its `front_end`
  argument.
- Map keys and bar clicks that open dialogs become `app::input::DialogAsk` values (`Pause`, `Hints`,
  `Quicksave`), which are passed on as session actions.
- `app::input::modeless_route` splits the pointer between the unit-trading dialog and the map.

**Network windows without a session.** The direct `--net-host` and `--net-join` windows have no
session. They use `OwnPauseDialog` (`app/shell_menu.rs`), which feeds a `Screen::Playing` with a
pause overlay itself.

**`Action` and `ControlAction`.** `mandrake_session::Action` is a front-end choice.
`crate::controls::ControlAction` is a key-binding action ([configuration.md](configuration.md#controls-controlsdefaulttoml)).

## In-game bars, overhead map and console (outside the boundary)

These are drawn by the client into overlay layers, using the original game's interface tags:

- **`hud.rs`**: `Hud`, the status bar at the top and the control bar at the bottom. `BarId`,
  `Button` and `HudEvent` (`Pause`, `Help`, `OverheadMap`, `Command`) describe them. The module also
  has the frame-rate display (`FpsDisplay`) and the score table.
- **`command_bar.rs`**: the control bar's commands (`BarAction`: whisper, ally, the general orders,
  special, formation), the console prompts (`ConsolePrompts`) and chat packing.
- **`console.rs`**: `Console`, the message lines and chat history, and `ConsoleInput`, the chat
  line. It also handles narration subtitles. There is no command interpreter in a game. The dot
  commands (`.version`, `.count`, `.plugins`, `.ping`, ...) exist only in the lobby and network
  postgame chat (`mandrake_session::chat_commands::DotCommand`).
- **`minimap.rs`**: the overhead map.
- **`interface_setup.rs`**: loads the bars and the overhead map and lays them out (`load_hud`,
  `load_minimap`, `layout_interface`). `interface_state.rs` holds the selection-box and health-bar
  inputs.

The Fallen Lords' status bar (`tfl_hud.rs`) reuses `mandrake_dialogs::core::Dialog`, `Resources` and
`ThreeSlice`.
