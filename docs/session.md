# Session: menus, campaign flow, saved films and games

This part covers the crate `mandrake-session` (`crates/mandrake-session`, crate name `mandrake_session`): the
windowless state machine behind the menus and pregame screens. It decides which screen is shown
and what it holds, takes the player's actions, and asks whoever runs the game (the client, or a
test) to do things: load a level, list a folder, save a film, talk to the network. It owns the
campaign flow (New Game, briefing, load, play, postgame, next level), Saved Films and Saved Games,
the multiplayer screens from player setup to the network postgame, and the rules that come from
game data (which levels are open, what the epilogue offers).

It has no window, no GPU, no clock and no simulation. The drawing and input side is a front end
(`mandrake-dialogs` draws the original game's dialogs; see [interface.md](interface.md)), and the game
side is the client ([client.md](client.md)). That split is the point of the crate: a different
interface can replace `mandrake-dialogs` without touching the game flow, and every flow rule is testable
without game data.

Dependencies: `mandrake-common`, `mandrake-tag` (to build the catalog from tags), `mandrake-rec` (film and save
headers, veterans, naming), `mandrake-net` (only its data types: game descriptions, player records,
lobby rules, and `mandrake_net::endpoint::Endpoint` for the shared network loop), `tracing`. It must
never depend on `mandrake-sim`, `wgpu` or a windowing crate; keep it that way.

## Modules

| Module | What it holds |
|-|-|
| `mandrake_session::session` | `Session`: the state machine (`act`, `game_event`, `screen`, `take_requests`) |
| `mandrake_session::screen` | `Screen` and the plain-data struct of every screen and overlay |
| `mandrake_session::action` | `Action` (player to session), `Request` (session to game), `GameEvent` (game to session), `GameStart`, `LevelStart` |
| `mandrake_session::front_end` | the `FrontEnd` trait, `FrameInput`, `FrameOutput`, `UiEvent`, `UiKey`, `SoundCue` |
| `mandrake_session::catalog` | `Catalog`, `Campaign`, `ListEntry`, `Level`: the campaigns and levels, from tags or by hand |
| `mandrake_session::progress` | `Progress`: levels visited, New Game's last choices, hint preferences |
| `mandrake_session::rules` | game rules on data: `level_list`, `has_briefing`, `postgame` / `postgame_with`, `level_film` |
| `mandrake_session::report` | `PostgameReport` and its sections: the carnage report as finished text |
| `mandrake_session::text` | `Text`: Mac Roman bytes with the font's `|` style codes |
| `mandrake_session::films` | the recordings folder: `FilmFolder`, `FilmFile`, `FilmGame`, film file operations |
| `mandrake_session::saves` | the saved games folder: `SaveFolder`, `SaveFile`, save naming and writing |
| `mandrake_session::chat_commands` | the lobby's dot commands (`.version`, `.vets`, ...) |
| `mandrake_session::net` | multiplayer screen data and rules, `NetRequest`, `NetEvent`, `NetGameStart` |
| `session::net_flow` (private, `src/net_flow.rs`) | the multiplayer state transitions, part of `Session` |
| `mandrake_session::net_game` | `NetLoop`: one network game's loop on an endpoint, shared by client and server |
| `mandrake_session::tfl` | TFL's menus (`TflSession`), see [tfl.md](tfl.md) |

## The boundary

```
            Screen<'_> (borrowed, plain data)
  Session ------------------------------------> FrontEnd (mandrake-dialogs, or yours)
     ^  ^                                           |
     |  |  Action (from FrameOutput.actions)        |
     |  +-------------------------------------------+
     |
     |  GameEvent                 Request (take_requests)
     +--------------- client <-------------------------- Session
                   (loads maps, runs mandrake-sim, reads/writes files, runs mandrake-net)
```

The client's frame loop, in order:

1. `let screen = session.screen();`
2. `let out = front_end.frame(&screen, &FrameInput { now_ms, size, events, sound_busy, shift_held });`
3. for each `Action` in `out.actions`: `session.act(action)`; play `out.sounds` (`SoundCue`).
4. for each `Request` in `session.take_requests()`: do it, and later answer with
   `session.game_event(...)` where the request says so.
5. while a game runs, forward the game's own news (`GameEvent::Loaded`, `MapOver`,
   `MissionAdvanced`, `Restoring`, `GameEvent::Net(...)`).

Rules of the boundary:

- **The session never blocks and never waits.** A slow or missing front end changes only what is
  shown. Anything that takes time (a load, a fade, a cutscene, a folder listing) is a `Request`
  answered later by a `GameEvent`.
- **The front end reads only `Screen`.** It holds no game state of its own beyond what it needs
  to draw (cursor, focus, scroll). `FourCC` values in screens are always game data tags (a mesh,
  its art, a sound), never dialog or control ids, so a non-original interface can use them.
- **No key is named on the game side.** `Action::NewGame { all_levels }` carries the result of
  "Shift held" as the front end sampled it; `Action::Quicksave` is sent by whoever checked the
  key binding.
- **Actions that do not apply are ignored**, logged at debug level. A front end may send anything
  at any time.
- `Session::front_end_has_input` is true everywhere except in play with no dialog open: the client
  routes input to the game only then. `FrontEnd::modeless_rect` gives the area of a dialog that
  floats over a running game (the unit trading dialog), so input outside it reaches the game.

`FrontEnd` is in `mandrake_session::front_end` and names no wgpu type. Drawing is a separate trait,
`mandrake_render::DrawInterface`; the client combines both (`crates/mandrake-client/src/app/shell_menu.rs`).
`crates/mandrake-session/tests/second_front_end.rs` is a minimal non-drawing front end that walks a
whole campaign path through the trait; use it as the template for a new interface.

## Screens

`Screen<'a>` (in `mandrake_session::screen`) is one of:

| Variant | Data | Notes |
|-|-|-|
| `MainMenu` | `MainMenu`: which buttons work, the menu music tracks | Saved Films / Saved Games / Replay Intro are enabled by folder listings the client sends unasked |
| `NewGame` | `NewGame`: campaigns, level list, difficulties, art, gameplay options | also as an overlay over a game (Revert with no save) |
| `Loading` | none | black: fade out, load, fade in |
| `Briefing` | `Briefing`: art, title, text, captions, narration | |
| `Playing { overlay }` | `Overlay`: `Hints`, `Paused`, `SaveGame`, `SavedGames`, `NewGame`, `UnitTrading` | `None` means the game has the input |
| `CampaignEnd` | `Narration` | the game epilogue after the last level |
| `Postgame` | `Postgame`: report, art, sound, exits, Save Film prompt | |
| `Films` | `Films`: Saved Films list, details, buttons, prompts | |
| `FilmReport` | `PostgameReport` | a film's end: its report over black |
| `SavedGames` | `SavedGames`: list, snapshot, date, Delete question | |
| `Restoring` | `Restoring`: name and progress (0..256) | a saved game's film replaying |
| `Cutscene` | `Cutscene { name }` | a video from the `cutscenes` folder |
| `Net` | `mandrake_session::net::NetScreen` | the multiplayer screens |
| `Tfl` | `mandrake_session::tfl::TflScreen` | only from `TflSession`, never from `Session` |
| `Quit` | none | the program should exit |

Each struct carries exactly what the original dialog shows, with booleans for enabled buttons
(for example `Paused::can_save`, `Films::can_view`, `PostgameExits::next`). Text is `Text`, which
keeps the tags' Mac Roman bytes and `|` style codes; `Text::to_plain_utf8` strips them for any
front end that does not render the original font, and `Text::from_plain_utf8` goes the other way.

## Requests and events

`Request` (session to game) and the matching answers:

| Request | Client does | Answer |
|-|-|-|
| `LoadGame(GameStart)` | fade out, load the level (or a film, or a save, or a network game), fade in | `GameEvent::Loaded` after the fade in, or `LoadFailed(reason)` |
| `BeginPlay` | start running ticks | |
| `SetPaused(bool)` | pause or resume the game clock | |
| `DropGame` | dispose of the map | |
| `SaveProgress(Progress)` | write the progress file | |
| `ListFilms` | `mandrake_session::films::list_folder` | `FilmsListed(FilmFolder)` |
| `ListSaves` | `mandrake_session::saves::list_folder` | `SavesListed(SaveFolder)` |
| `SaveFilm { name }` | `films::save_last_film` | `FilmSaved(Option<file name>)` |
| `DeleteFilm` / `DeleteSave { file_name }` | `films::delete_film` / `saves::delete_save` | none; a listing follows |
| `RenameFilm { file_name, name }` | `films::rename_film` | `FilmRenamed(file name now)` |
| `TakeSnapshot` | render the 160x120 save picture | `Snapshot(Snapshot)` |
| `SaveGame { name }`, `Autosave`, `Quicksave` | build the save (`mandrake_rec::writer::Recorder::saved_game`) and `saves::write_save` | `GameSaved(Option<SaveFile>)` |
| `PlayCutscene { name }` | play the video, or skip at once if it cannot | `CutsceneOver` |
| `Exit` | quit | |
| `Net(NetRequest)` | forward to the `mandrake_net` endpoint | `GameEvent::Net(NetEvent)` |
| `SaveNetPlayer(NetSettings)` | store the player name, colours, last address in the settings file | |
| `DescribeNetGame(FilmGame)` | look the lobby's map up in the tags | `NetEvent::GameDescribed` |
| `NetChatTo(ChatTo, Text)` | chat to one player or team | |

Events the client sends without a request: `FilmsListed`, `SavesListed` and `IntroFound` whenever
the main menu is shown (the original game rebuilds the menu each time and enables buttons from
what the folders hold then), `MissionAdvanced(n)` as the map's objectives advance (for the hints
dialog), `Restoring { ticks }` while a save replays, and `MapOver { report, veterans }` when the
map ends (after its last seconds), with the end code inside the report.

`GameStart` says how to start: `start: LevelStart` (`NewGame`, `ChangeMap` or `Restart`, which
decide the solo game options, see `mandrake_sim::players::solo_game_options`), `mesh`, `difficulty`,
`plugins`, `veterans` (opaque `mandrake_rec::header::MonsterInitializer` records, which the session
never looks inside), `reset_rankings`, `film` (a path: the game is that film's), `restore` (the
film is a saved game to replay and then play on) and `net` (a network game's
`mandrake_session::net::NetGameStart`, holding the new-game data both peers start from).

## The solo flow

```
MainMenu --NewGame--> NewGame --StartLevel--> [Cutscene] --> Briefing --BriefingDone--> [Cutscene]
   ^                                                                                     |
   |                                                                          LoadGame   v
   |                         <------- Loaded / BeginPlay -------- Loading <--------------+
   |                         Playing(None) <-> Playing(Some(overlay))
   |                              | MapOver
   |                              v
   +--- QuitToMenu --- Postgame (exits) --NextLevel/SkipLevel--> enter next level
                           |  RestartLevel --> Loading (same map, no briefing, original veterans)
                           |  LastSavedGame --> load the last save
                           +- campaign end --> CampaignEnd --NarrationDone--> MainMenu
```

Key points, in the code's terms (`Session` private methods named in parentheses):

- **New Game** (`open_new_game`, `new_game_dialog`): campaigns from the `Catalog`, the level list
  from `rules::level_list` with the player's `Progress`, the chosen campaign and level and the
  difficulty remembered on OK and on Cancel alike. OK (`start_new_game`) clears the veterans,
  resets rankings, forgets the last saved game and takes the campaign's plugins.
- **The tutorial** (`start_tutorial`, mesh `rules::TUTORIAL`, `00tm`) has no plugins, no
  veterans and no postgame dialog, and is never recorded.
- **Entering a level** (`enter_level`): the level is marked visited at its briefing; the briefing
  shows only if the level has both art and text (`rules::has_briefing`); the level's pregame
  cutscene (`rules::level_film` with `PREGAME_FILM`) plays after the briefing, or before it when the
  mesh flag `MESH_FILM_BEFORE_BRIEFING` is set. Then `load_level` sends `Request::LoadGame` and
  shows `Loading`.
- **Play** (`begin_play`): on `GameEvent::Loaded` the session sends `BeginPlay`, shows the hints if
  the player wants them (`Progress::show_hints`, not on a level whose mesh disables the hints
  dialog), and asks for an autosave if Next Level led here.
- **Pause** (`Action::OpenPause`): `Paused` lists what works: Save (`can_save`: a solo game, not a
  film, not the tutorial), Open, Revert, Restart, and in a network game the host's Rehost and
  Restart (`NetPause`). A film shows only Quit and Resume (`Paused::replay`).
- **Map end** (`map_over`): a film goes to `FilmReport` and then the menu. A game stores the
  veterans the map left and asks `rules::postgame(level, end_code, can_revert)`:
  - `PostgameRule::ToMenu` for the tutorial or an unreadable level;
  - `PostgameRule::RestartFresh` for an end code outside 1..3 (veterans cleared, level again);
  - `PostgameRule::Dialog(PostgameChoice)` otherwise: the art, its sequence (win or loss), the
    ambient sound, which exits show, where Next Level and Skip This Level go, whether the
    campaign ends here, and which level becomes visited.

End codes: 1 win, 2 win by the alternate route (the level's `alternate` link; on a level with no
alternate it counts as a loss with Skip This Level offered), 3 loss. The game epilogue
(`CampaignEnd`) follows end code 1 or 2 on a level with `MESH_CAMPAIGN_END`, or with no next link in a file
of the campaign-end type. `rules::MYTH2_EPILOGUE` and `rules::TFL_EPILOGUE` hold the few places
where the two games' epilogues differ; `rules::postgame_with` takes either.

### The level list rule

`rules::level_list(entries, visited, current, all_levels)` reproduces the original New Game list:

- Only entries with `ENTRY_SOLO` are considered. Without `all_levels` (Shift not held) only
  `ENTRY_LISTED` entries without `ENTRY_OPEN_HIDDEN` are listed. Alternate levels
  (`ENTRY_EXTRA_ALTERNATE` in `ListEntry::extra_flags`, or the two hard-coded alternate meshes) are
  listed only once visited.
- The list is sorted by `order`. A row is selectable if Shift is held, it is visited (by mesh and
  plugin checksum together) or current, it has `ENTRY_OPEN_HIDDEN`, its order is the lowest, or
  its order ends in 01 (the first level of each hundred).
- Selected: the current level if listed, else the first of the lowest order, else the first row.

`Progress` keeps the visited list (at most `progress::MAX_VISITED` entries), New Game's last
choices, `show_hints` and `hint_index`. `Progress::to_text` / `from_text` are our own file format;
`Progress::import_game2` reads the original game's preferences once, when there is no file of our
own. The session sends `Request::SaveProgress` whenever it changes; it never opens files itself
for progress.

### Catalog

`Catalog::from_tags(&mandrake_tag::mesh_links::CampaignTags)` reads every campaign (the base game and
each plugin with solo levels, ordered as New Game shows them), every reachable `Level` (from its
mesh header: briefing art and text, captions, narration, post art, next and alternate links,
ambient sounds, flags, objectives, hints, cutscene names), the difficulties, the hint tabs and
general hints, the menu music, the campaign-end narration, and the network mapsets. A tag that
does not resolve is `None` in the catalog, so the session never has to ask. Tests build a
`Catalog` by hand (`crates/mandrake-session/tests/common/mod.rs`), which is why no session test needs
game data.

## Saved Films

`mandrake_session::films` is the game side of the recordings folder (`recordings` under the game
directory, `films::FOLDER`). The file format is in [films.md](films.md).

- `films::list_folder(game_dir, tags)` reads every file whose tag header group is `reco`
  (`films::is_recording`) and whose recording header reads (`films::read_header`, version at most
  3), parses its game (`films::parse_game` into `FilmGame`: mode, game type, options, time limit,
  mesh, difficulty, planning time, version, plugins) and looks it up in the tags
  (`FilmTags::look_up`: the map's display name, whether the plugins are installed, the map's art,
  its starting locations). Files are sorted with `films::csstrcmp` (case-folding, numbers by
  value). Files being written (`.<name>.writing`, `films::WRITING_SUFFIX`) are skipped.
- `FilmFolder` also says whether the last recording exists and plays (`last`, `last_exists`), the
  last recording's name (`inrf` string 0, default "Last Recording"; `FilmTags::last_recording_name`)
  and the default name for Save Film (`inrf` string 2, default "Untitled Replay").
- `FilmGame::gameplay_version` is `films::compute_gameplay_version`, which picks the rules version
  from the options and the recorded version id; `films::gameplay_version_to_string` formats it
  for the details pane.
- Screen behaviour (`choose_film`): View Film works when the map is found or plugins are
  reported missing; the button becomes Find Plugins when something is missing; Delete works with a
  film chosen; Rename not for the last recording.
- **View Film** (`play_film`) starts a `GameStart` with `film: Some(path)` and the film's own mesh,
  difficulty and plugins. The game is entirely the film's. At its end the client sends `MapOver`
  and the session shows `FilmReport`, then the menu.
- **Save Film / Save Last Film** opens `SaveFilm`, whose OK is checked by `films::film_name_ok`
  (not the last recording's name, and the name must survive `mandrake_rec::writer::film_names`
  unchanged). The client then calls `films::save_last_film`, which copies the last recording's data
  under a new tag header (the subgroup of an existing tag of that name, or a new one).
- **Rename Film**: `films::rename_name_ok`, then `films::rename_film` renames the file to
  `<name>.m2rec` and rewrites the tag header's name; a name another film already has (any case)
  shows `RenameFilm::exists`. **Delete Film**: `films::delete_film`. File names are checked to be
  plain names inside the folder, never paths.
- `films::write_recording` and `films::last_recording_subgroup` are how the client writes the last
  recording safely (see [films.md](films.md)).

## Saved Games

`mandrake_session::saves` is the `saved games` folder (`saves::FOLDER`). A save is a film of the game so
far under a `save` tag (format in [films.md](films.md)); loading one replays its film to its end
at full speed and then plays on.

- `saves::list_folder` reads every `save` tag (`saves::is_save`, `saves::read_header`), with its
  game, end tick, veterans, snapshot (`saves::snapshot_of`), subgroup and file time, sorted newest
  first. Dates are formatted with `saves::date_string` and the local zone (`saves::LocalZone`, read
  from the system's zone file; UTC when there is none).
- **Continue Saved Game** (`continue_saved_game`): a `GameStart` with `film: Some(path)`,
  `restore: true`, the save's veterans (also kept as the restart backup), and the save as the
  "last saved game". While the client replays it, `GameEvent::Restoring { ticks }` moves
  `Screen::Restoring`'s progress in steps of `session::RESTORE_TICKS_PER_STEP` (1800 ticks); then
  `Loaded` starts play. A restored game is a live solo game from that point.
- **Save** (pause dialog): `Request::TakeSnapshot` and `ListSaves`, then the `SaveGame` prompt with
  the last save's name (unless it was an autosave) or `inrf` string 1 ("New Saved Game"). OK uses
  `saves::save_name_ok`; a name a save already has asks to replace first (`SaveGame::replace`).
- **Autosave**: on the first tick of a level reached by Next Level, named
  `saves::autosave_name` ("Autosave- YYYY-MM-DD HH.MM.SS", local time; `saves::AUTOSAVE_PREFIX`).
- **Quicksave**: `saves::quicksave_name` picks "Quicksave 1" to "Quicksave 5"
  (`saves::QUICKSAVES`): the first missing, else the oldest.
- `saves::write_save` names the file with `mandrake_rec::writer::save_names` (no extension) and picks
  the subgroup with `saves::subgroup_for_save` (the replaced file's, else one made from the name).
- **Revert / Last Saved Game** loads the last save made or loaded; if its file is gone, New Game
  opens instead, as in the original game.

## Multiplayer screens

`mandrake_session::net` holds the data and rules; the transitions are `net_flow.rs`, a child module of
`session` sharing `Session`'s state. A network game's load and play reuse `Screen::Loading` and
`Screen::Playing`. The network itself is the client's `mandrake_net` endpoint: the session asks with
`NetRequest` (through `Request::Net`) and hears `NetEvent` (through `GameEvent::Net`). The wire
protocol is in [networking.md](networking.md).

```
MainMenu --Multiplayer--> Net(Player: nesp) --ChooseOther--> Net(Protocol)
   --> Net(HostOrJoin: Inet) --HostGame--> NetRequest::Host --NetEvent::Listening--> Net(Lobby)
                             --JoinByAddress--> Net(Joining) --NetEvent::Joined + Description--> Net(Lobby)
                                                             --Denied/TimedOut--> Net(JoinFailed)
Net(Lobby): Ready, Options (host: GameOptions), team buttons, Kick, Lock, chat, dot commands
   host Start --> NetRequest::Start --> everyone: NetEvent::Configuration(CODE_START)
   --> start_net_load: new-game data from the description and players --> LoadGame(net) --> Loading
   --> GameEvent::Loaded --> NetRequest::Loaded (answer the pre-game barrier)
   --> NetEvent::AllSynced --> BeginPlay, Playing
   --> MapOver --> [co-op epilogue] --> NetRequest::MapOver --> Net(EndBarrier) --AllSynced-->
   --> Net(Postgame: endi) --host Restart / CODE_START--> Net(Lobby) ...
```

- `net::NetSettings` is the player's name, team, icon, colours and last address, kept in the
  client's settings (`Request::SaveNetPlayer`).
- In the lobby the session holds the game description and every player's record (`net::Lobby`)
  and decides who may do what: `net::may_start` (everyone else ready), `net::may_change_options`,
  `net::may_kick`, `net::description_valid`, `net::missing_plugins` (the `MissingPlugins` screen;
  with our own dedicated server it can download them instead, `NetRequest::DownloadPlugins`).
- On configuration code `net::CODE_START` (100) every peer builds the same new-game data
  (`mandrake_net::game_description::new_game_data` from the description, the sorted players with Ready
  set, and the local id) into `NetGameStart`. Both the simulation and the film start from it, so it
  must be identical on all peers.
- Co-operative games with Show Narration play the level's prologue and epilogue (options `0x880`).
- `net::is_our_server` recognises the dedicated server's invisible player record; it is never
  shown as a player ([server.md](server.md)).
- `Action::SaveReplay` in `endi` copies the last recording as `Netgame- <name>`
  (`net::NETGAME_FILM_PREFIX`).
- `chat_commands` implements the dot commands typed into lobby chat (`chat_commands::parse` into
  `DotCommand`). Their output goes only to this machine's chat list; `.savevets` / `.loadvets`
  write and read veterans tables in the `veterans` folder (`chat_commands::save_veterans`,
  `chat_commands::load_veterans`).

### The network game loop (`net_game.rs`)

`mandrake_session::net_game::NetLoop` is one network game on an `mandrake_net::endpoint::Endpoint`, from code
100 to the end barrier. Both the client's network game and the dedicated server's use it, so a
host on either side behaves identically. It reaches the sim only through the `net_game::NetSim`
trait (`current_tick`, `net_horizon`, `queue_incoming_command`, `take_outgoing_commands`; each runner wraps
`mandrake_sim::Simulation` in a small adapter that implements it) and time only through a `net_game::Clock`, because
`std::time::Instant` is not available in the browser.

Per poll: `NetLoop::poll_events(sim, on_command)` feeds the stamped commands received into the sim
in arrival order (each parsed with `mandrake_rec::CommandPacket::parse`) and notes barrier ends and a
lost host (`NetNews`); `NetLoop::end_poll` moves the horizon with the NULL heartbeat
(`NetLoop::heartbeat`) and answers barriers; `NetLoop::send_outgoing` sends the sim's outgoing
commands up the endpoint. `NetLoop::map_loaded` and `NetLoop::map_over` mark when the pre-game and
end barriers may be answered; the end barrier is given up after `net_game::END_BARRIER_WAIT`.
`net_game::PollClock` turns elapsed time into 30 Hz polls and `net_game::machine_ticks` gives the
machine tick count the game seeds use.

## TFL

`mandrake_session::tfl::TflSession` is the same idea for Myth: The Fallen Lords' simpler menus, producing
`Screen::Tfl(TflScreen)` and taking `tfl::TflAction`, with the same `rules` (through
`rules::TFL_EPILOGUE`) and `Progress`. TFL network games use the same `Session` with
`Session::set_tfl` (their own data checksums). See [tfl.md](tfl.md).

## Invariants

- The session holds no simulation state and makes no random choices that affect a game: the
  seed, options and players of a game come from `GameStart` and the film or network data. It
  cannot desync a game, but the new-game data it passes for a network game must be the same on
  every peer.
- Every rule that the original game applies to data lives in a tested function (`rules`,
  `films`, `saves`, `net`); `Session` only sequences them.
- The session decides; the client acts. A new feature that needs a file, a sound or the network
  adds a `Request` and a `GameEvent`, not file access in `Session`. (The folder functions in
  `films` and `saves` do touch the disk, but the client calls them in answer to requests.)
- Screens are rebuilt from session state; a front end must not cache them across frames beyond
  its own presentation state.

## Verifying changes

All of the following run with no game data:

```
cargo test -p mandrake-session
```

- `tests/flow.rs`: every step of the solo flow, with a stand-in client that reads the requests
  (menu buttons, New Game and the level list, briefings, loads, pause, postgame exits for each end
  code, campaign end, restart with the original veterans).
- `tests/rules.rs`: `level_list` and `postgame` tables.
- `tests/second_front_end.rs`: a whole campaign path driven only through `FrontEnd` and `Screen`.
- `tests/net.rs`: the multiplayer screens and their transitions, with fake `NetEvent`s.
- `tests/cutscenes.rs`, `tests/films.rs`, `tests/save_film.rs`, `tests/save_manage.rs`,
  `tests/film_manage.rs`, `tests/quicksave.rs`, `tests/saves.rs`, `tests/save_game.rs`: the
  folder screens and file operations, on films and saves the tests write into a temporary folder.
- Unit tests in `films.rs` (sorting, naming), `saves.rs` (dates, zones, quicksave names),
  `progress.rs` (the progress file) and `chat_commands.rs`.

A new screen or action should get a `flow.rs`-style test that drives `Session::act` and
`Session::game_event` and checks `Session::screen` and `Session::take_requests`. For changes that
affect what the client must do, also check the client by hand (see [client.md](client.md)): for
example `--session --play MESH` walks to a level through the real menus.
