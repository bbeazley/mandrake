# Networking (mandrake-net)

The `mandrake-net` crate is the TCP/IP multiplayer protocol of the original 1.8.5 engine, rebuilt byte
for byte so that our client and our dedicated server can join and host games with copies of the
original game. This file covers the transport, the wire format with its byte layouts, the net
layer (ids and the player list), the pregame lobby, the in-game lockstep (stamped orders, the
heartbeat, pings, barriers, drops, the forced end, pause and speed, veterans), our own extension
messages, the LAN game list, and how a TFL game differs. The crate depends only on `std` and
`mandrake-rec` (for the new-game data and the film writer); it never reaches the sim.

How a client's session uses it is in [session.md](session.md); how the sim runs the stamped
commands is in [simulation.md](simulation.md); the film format the peers record is in
[films.md](films.md); the dedicated server built on it is in [server.md](server.md); the TFL
game is in [tfl.md](tfl.md).

## Module map

| Module | What it holds | Sockets? |
|-|-|-|
| `mandrake_net::wire` | the 8-byte header, every message type, the stream splitter, the greeting | no |
| `mandrake_net::player_data` | the 0x7c-byte player record, the join request, the player sort | no |
| `mandrake_net::game_description` | the game description (type 6), the plugin list, the new-game data | no |
| `mandrake_net::topology` | the net layer: `Topology` (joiner) and `HostTopology` (host) | no |
| `mandrake_net::lobby` | the pregame: `Lobby` (joiner), `HostLobby` (host), teams, chat, barriers' host side | no |
| `mandrake_net::client` | each peer's game side: horizon, pong, barrier answers, the network film | no |
| `mandrake_net::server` | the host's game server: clock, stamping, heartbeat, history, ping | no |
| `mandrake_net::endpoint` | `Joiner`, `Host` (the above combined) and `Endpoint` (them on sockets) | `Endpoint` only |
| `mandrake_net::transport` | `Connection`, `Listener`, `ByteStream`, `Notify` | yes |
| `mandrake_net::ours` | our extension messages between our client and our server | no |
| `mandrake_net::lan` | LAN browse queries and answers, `LanList`, `Browser` | `Browser` only |
| `mandrake_net::game_kind` | `GameKind` (Myth II or TFL) and each kind's command codes | no |
| `mandrake_net::tfl` | what a TFL game changes on the same wire | no |

### The sans-IO design

Everything except `mandrake_net::transport` (and the thin `Endpoint` and `lan::Browser` wrappers over
it) is a state machine with no socket and no clock: bytes in, a caller-supplied time in
milliseconds, bytes and events out. `Joiner::receive(bytes)` / `Joiner::poll(now_ms)` /
`Joiner::take_outgoing()`, and the same per connection id on `Host` (`Host::receive`,
`Host::poll`, `Host::take_outgoing`, `Host::connection_lost`). Nothing in those types reads a
clock or starts a thread; the caller polls, as the original engine polls from its main loop.

This is what lets the same code run natively, in the browser (where the stream is a WebSocket and
`std::time::Instant` is unavailable), in the dedicated server, and in tests with a fake clock and
in-memory pipes. A host's own player is a `Joiner` too, connected to its `Host` by an in-process
loopback buffer, exactly as the original host is a player of its own game.

## Transport

`mandrake_net::transport` is the only module with sockets.

- **TCP carries everything.** The default port is `mandrake_net::wire::DEFAULT_PORT` (3453). Each
  stream has `TCP_NODELAY`.
- **A UDP socket exists but carries no game data.** The joiner binds one on an ephemeral port and
  sends its number in the 2-byte *greeting*; the host's listener binds UDP on the same port as its
  TCP listener and greets each accepted connection with that port. The original engine connects
  its UDP socket to the peer's; nothing is ever sent on it, but the greeting must be there. The
  host's UDP socket also answers LAN browse queries (`Listener::answer_queries`).
- **The greeting** is the first 2 bytes of each direction of a stream: the sender's UDP port,
  big-endian (`wire::encode_greeting`, `wire::decode_greeting`). Then messages follow.
- **Connecting**: `Connection::connect` with `transport::CONNECT_TIMEOUT` (30 s, the engine's),
  the one blocking call. A host refuses a connection (game full or closed) by accepting it and
  closing it at once.
- **Queues**: per connection a send queue of at most `wire::MAX_QUEUE_SIZE` (0x4000) bytes. An
  overflow, a failed send or EOF is a lost connection, as the original engine drops a player.
- **Threads**: a TCP connection has a reader thread and a writer thread so that the network never
  waits on the caller's frames. The reader takes what arrives (with its arrival time) and, on a
  joiner's connection, answers the host's ping at once with the pong the caller would have sent
  (`Connection::set_pong_from`, `Connection::take_pong_times`); the caller is told which pings
  were already answered (`Joiner::pongs_sent`) so it doesn't answer twice. Each arrival sets the
  connection's `Notify` (and an optional `Waker`), which a loop without a window waits on
  (`Endpoint::wait`) instead of sleeping.
- **Other byte streams**: `Connection::over` wraps any `ByteStream` (non-blocking read, write,
  close, carrying the TCP stream's bytes in order with no message boundaries). The browser's
  WebSocket to the dedicated server's proxy is one; its greeting carries port 0, which the proxy
  replaces (see [server.md](server.md)).

## Wire format (`mandrake_net::wire`)

All multi-byte fields are **big-endian** unless stated, except the ping's tick field (below).
Text is Mac Roman bytes, never UTF-8; strings are NUL-terminated unless a fixed field.

### The header

Every message starts with 8 bytes (`wire::Header`, `wire::HEADER_SIZE`):

| Offset | Size | Field | Meaning |
|-|-|-|-|
| 0 | u16 | `kind` | message type; bit 15 set = net layer (`0x8000 + n`), else game (`0..=0xd`), or ours (`0x40..=0x47`) |
| 2 | u16 | `length` | the whole message's length, header included, 8..=0x1000 |
| 4 | u8 | `to` | destination: a player's net id, `TO_HOST` (0xfe) or `TO_ALL` (0xff) |
| 5 | u8 | `from` | sender's net id; the host rewrites it on relay; 0xfe for the host's net layer |
| 6 | u16 | `spare` | always 0; kept so every message round-trips |

`wire::MAX_MESSAGE_SIZE` is 0x1000. A length outside 8..=0x1000 breaks the stream: the original
engine drops the player, and `wire::Splitter` keeps the error (`WireError::BadLength`).

### Framing

`wire::Splitter` cuts one direction of a stream: `Splitter::with_greeting()` first yields
`Frame::Greeting(port)` after 2 bytes, then `Frame::Message(bytes)` for each whole message (it
waits for 8 bytes, reads the length, waits for the rest). `Splitter::without_greeting()` is for a
stream with no greeting. `Message::decode` takes exactly one whole message; `Message::encode`
writes the header from the body and fails above 0x1000 bytes (`WireError::TooLong`). Decoding is
strict: bytes left after a body, a missing NUL, or an unknown type is an error.

### Message types (`wire::kind`)

Net layer (bit 15):

| Type | Name | Direction | Body |
|-|-|-|-|
| 0x8000 | `JOIN_REQUEST` | joiner to 0xfe | name\0, password\0, player record (0x7c) |
| 0x8001 | `PLAYER_DATA_CHANGED` | player to 0xfe | u16 id, player record |
| 0x8002 | `GAME_DATA_CHANGED` | (unused) | raw |
| 0x8003 | `NEW_PLAYER` | host to each | u16 id, u16 flags (bit 0: "this is you"), name\0, player record |
| 0x8004 | `JOIN_DENIED` | host to joiner | reason\0 [, `error=N`\0] |
| 0x8005 | `PLAYER_DELETED` | host to each | u16 id |
| 0x8006 | `GAME_DATA` | (unused) | raw |
| 0x8007 | `CONNECTION_CLOSED` | (unused) | raw |

Game messages:

| Type | Name | Body |
|-|-|-|
| 0 | `CHAT` | lobby and postgame chat (below) |
| 1 | `COMMAND` | one command (below) |
| 2 | `CONFIGURATION` | u16 flags, u16 code, u32 value |
| 3 | `PLAYER_SYNC_DATA` | u16 player id, u16 value (a barrier entry) |
| 4 | `BEGIN_SYNC_DATA` | empty: a barrier begins |
| 5 | `SERVER_COMMAND` | u16: 0 starts the server clock, 1 stops it (host-local) |
| 6 | `GAME_DESCRIPTION` | the game description (below) |
| 7 | `MYTH_PLAYER_DATA` | raw (never built) |
| 8 | `VETS` | raw: the last game's monster initializers |
| 9 | `PING` | server to player: u32 ticks, then one s16 latency per listed player |
| 9 | (pong) | player to 0xfe: the u32 ticks echoed; told from a ping by length (4 bytes) |
| 0xa | `BEGIN_GAME_MIDGAME` | raw (mid-game join; not supported) |
| 0xb | `REQUEST_MORE_COMMANDS` | empty (mid-game join; not supported) |
| 0xc | `SAVED_GAME_HEADER` | raw (network saves; not supported) |
| 0xd | `SAVED_GAME_PROGRESS` | u32 tick |
| 0x40..0x47 | ours | see [Our extensions](#our-extensions-m2_netours) |

`wire::Body` has one variant per type; bodies a later layer types (the player record, the
description, the vets, the saved-game header) stay raw bytes in `Body`, so every message
round-trips exactly.

**The ping's tick field is not byte-swapped**: it is the sender's millisecond clock as the host's
CPU stores it, which is little-endian on every x86 copy of the original game. `mandrake-net` writes and
reads it little-endian (`Body::Ping`, `Body::Pong`); the latencies after it are big-endian s16,
-1 once a player has been silent 4 s.

### A command (type 1)

The body is one command with its own 8-byte header (`wire::Command`,
`wire::COMMAND_HEADER_SIZE`):

| Offset | Size | Field |
|-|-|-|
| 0 | s16 | length, counting these 8 bytes; must equal the message body's length |
| 2 | u8 | command type (0 NULL, 1 SYNC, 9 DROP_PLAYER, 10 CHAT, 0x15 forced end, ...) |
| 3 | u8 | player: 0xff from a client; the sender's net id once the server stamped it |
| 4 | s32 | tick: 0 from a client; the server's tick once stamped |
| 8 | ... | payload, big-endian, as the film format packs it |

These are the same bytes a film stores per command (`client::command_bytes`), so a stamped command
can be recorded as it arrives. The payload layouts per command type are the film's
([films.md](films.md), `mandrake_rec::CommandPacket`).

### Chat (type 0)

`wire::Chat`: the sender's name in a 32-byte NUL-padded field, eight u16 words, then the text,
every byte of which is sent **plus `wire::CHAT_SHIFT` (13)** modulo 256, then a plain NUL.
`lobby::chat` names the words: `words[0..3]` the sender's colour (R, G, B), `[3]` 0, `[4]` flags
(`chat::PASS_ON`, bit 0: "host's player, pass this on"), `[5]` the mode (`chat::TEAM` 0,
`chat::PLAYER` 1, `chat::ALL` 2), `[6]` the target (a leader's or player's net id, or
`chat::NO_TARGET`), `[7]` 0. Text is at most `chat::TEXT_MAX` (0xff) bytes. In-game chat is not
this message but a CHAT command (type 10).

## The player record and join request (`mandrake_net::player_data`)

`PlayerData` is the 0x7c-byte record (`PLAYER_DATA_SIZE`) a player sends and the host lists. The
same bytes, in the same order, are a player entry of the new-game data and so of a film header.

| Offset | Size | Field (`PlayerData`) | Notes |
|-|-|-|-|
| 0x00 | s16 | `field_00` | -1 in the lobby; in the new-game data -1, or 0 in a co-operative game |
| 0x02 | s16 | `field_02` | 0 (1 for the co-op computer players) |
| 0x04 | u16 | `flags` | 0x11 normally; 0x40 = observer, no team changes (`lobby::FLAG_NO_TEAM_CHANGE`) |
| 0x06 | u16 | `field_06` | |
| 0x08 | s32 | `team` | net id of the player's team leader, -1 none |
| 0x0c | s32 | `net_id` | the player's net id (0 = host) |
| 0x10 | u32 | `ready` | the lobby's Ready |
| 0x14 | u32 | `team_locked` | the team lock (1 locked) |
| 0x18 | s32 | `metaserver_id` | -1 |
| 0x1c | u8 | `metaserver` | joined through a metaserver; a LAN host refuses anything but 0 |
| 0x1d | u8 | `field_1d` | |
| 0x1e | u16 | `version` | gameplay version, `GAMEPLAY_VERSION` (0x889 for 1.8.5) |
| 0x20 | u16 | `build` | `BUILD_NUMBER` (0x1d7) |
| 0x22 | 6 bytes | `field_22` | never swapped or read by the original engine; our server's mark (`ours::SERVER_MARK`) |
| 0x28 | 0x54 | `appearance` | `Appearance` |

`Appearance` (0x54 bytes): `+0x00` u16 icon (0), `+0x02` s16 `rank` (-1; the first sort key),
`+0x04` 32-byte name, `+0x24` 32-byte team name, `+0x44` eight u16: two colours of four words
(R, G, B, flags). `PlayerData::new` builds a fresh joiner's record. `player_data::copy_over`
copies a new name over an old one the way the original game's dialog does (the bytes after the
new NUL keep the old text), which matters because those bytes travel and end up in films.

`JoinRequest` (type 0x8000): name\0, password\0, then the record. `JoinRequest::from_body`,
`JoinRequest::to_body`.

`player_data::sort_players` is the original game's player sort used for the new-game data:
by `appearance.rank` highest first, then by net id, then grouped as each leader followed by the
players whose `team` is its id; a player whose leader is missing gets team -1 and flag 0x20 and the
grouping restarts. **Every peer must sort identically**: the sorted order is the game's player
order in every peer's sim and film.

## The game description and new-game data (`mandrake_net::game_description`)

`GameDescription` is type 6's body, sent by the host to 0xff when a player arrives, after every
options change, and with the new seed at the start. Offsets are within the body (add 8 for the
message):

| Offset | Size | Field |
|-|-|-|
| 0x000 | 0x2c | `parameters`: `GameParameters`, the new-game data's head |
| 0x02c | 0x200 | `plugins`: `PluginList` |
| 0x22c | u32 | `foundation_checksum` (0 in a LAN game; only the metaserver path writes or checks it) |
| 0x230 | u32 | `field_230` |
| 0x234 | u32 | 0, not read back |
| 0x238 | u16 | `flags` (`FLAG_PASSWORD`, bit 1: the game has a password) |
| 0x23a | u8 | `player_count` |
| 0x23b | u8 | 0 |
| 0x23c | | game name\0 (at most `GAME_NAME_MAX` 0x20 with NUL), map name\0 (at most `MAP_NAME_MAX` 0x40) |

`GameParameters` (0x2c bytes):

| Offset | Size | Field |
|-|-|-|
| 0x00 | s16 | `mode` (1 in a network game) |
| 0x02 | s16 | `game_type` (0 Body Count, 2 Last Man on the Hill, ...) |
| 0x04 | u32 | `options` (bit flags, below) |
| 0x08 | s32 | `time_limit` in ticks (1800 a minute), -1 none |
| 0x0c | 4 bytes | `mesh`: the map's mesh tag |
| 0x10 | s16 | `difficulty` |
| 0x12 | s16 | `max_players` |
| 0x14 | u16 | `seed_low`: the seed's low half |
| 0x16 | s16 | `field_16` (-1) |
| 0x18 | u32 | `seed` |
| 0x1c | s32 | `planning_time` |
| 0x20 | u16 | `flags` (a film's recording sets bit 1) |
| 0x22 | u16 | `build` |
| 0x24 | u32 | `version`: gameplay version (0x889); a TFL game's marker (see below) |
| 0x28 | u16 | `field_28` |
| 0x2a | u16 | `plugin_count` |

Option bits named in the code: `OPTION_COOPERATIVE` 0x80; `lobby::OPTION_TEAMS` 2;
`lobby::OPTION_VETERANS` 0x20; `lobby::OPTION_VETS` 0x40; `lobby::OPTION_HOST_NO_TEAM` 0x10000
("Server is Observer": the host's own record takes flag 0x40 and gets no units).

`GameParameters::reset_seeds(machine_ticks)` is how the host makes the game's seed at the start:
from the wall clock's milliseconds (`mandrake_session::net_game::machine_ticks`), with its low half also
at 0x14. Every peer's sim seeds its random generator from the description, so the seed is shared
and nothing else is.

**The plugin list** (`PluginList`, 0x200 bytes): u16 count, then per plugin a u32 checksum (the
plugin file header's), name\0 (at most 0x1f) and URL\0 (at most 0x3f), zero-padded; at most
`MAX_PLUGINS` (16) decode. A plugin that doesn't fit is left out but the count stays, as the
original encoder does. In the lobby the host adds the gameplay version's *pseudo-plugin*
("Myth II 1.8.5" at `GAMEPLAY_PLUGIN_URL`, `PluginList::add_gameplay_version`,
`gameplay_version_plugin`, the table `GAMEPLAY_VERSION_PLUGINS`) and drops it again at the start
(`PluginList::remove_gameplay_versions`), so the description sent with the new seed is exactly the
new-game data's head. `PluginList::for_entry_point` gives the list a chosen map opens. A joiner
compares the list with its own plugins (`mandrake_session::net::missing_plugins`).

**The new-game data** (`game_description::new_game_data`, a `mandrake_rec::writer::NewGameData` of
`mandrake_rec::writer::NEW_GAME_DATA_SIZE` bytes) is built by **every peer alone** at the start from
the last description and the listed players: the head (parameters and plugin list, 0x22c bytes),
u16 player count at 0x22c, u16 local player index at 0x22e, the sorted player records from 0x230
(0x7c each). A co-operative game turns its time limit off, sets option 0x20, adds the "Dark" and
"Neutral" computer players, and may force a shared team; gameplay version 0x867 forces option bit
24. This is the film header's new-game data ([films.md](films.md)), and it is what the sim starts
from. If two peers build different bytes here (a record with a different net id, a different
sort, a different plugin list), their sims differ from tick 0.

## The net layer (`mandrake_net::topology`)

**Joiner** (`Topology`): `Topology::new` sends the join request (to 0xfe, sender 0) and waits
`JOIN_ANSWER_TIMEOUT_MS` (5 s). The host answers with a 0x8003 for every listed player; the one
with flag `NEW_PLAYER_IS_YOU` is ours and gives our net id (`Topology::local_id`). Or a 0x8004: the
reason text, and with `error=N` (`topology::denial_code`) the receiver shows its own line N of
the game's internal messages (`Topology::set_internal_messages`, up to `MAX_INTERNAL_MESSAGE`).
Later 0x8003s replace a record; 0x8005 removes one. Our own changes go up as 0x8001 with our record
(`Topology::change_local`, `Topology::record_message`).

`Topology::list_order` is the player table by position, as the original engine keeps it: new
players at the end, a leaver's place closed up, and the whole list re-sorted at a game's start
(`Topology::sort_for_game`). **Ping latencies are by these positions**, so the joiner must keep
the same order as the host.

**Host** (`HostTopology`): ids are given round 1..=`MAX_NET_ID` (0x7f), the host itself
`HOST_NET_ID` (0). `HostTopology::accept` refuses a connection when the game is closed (Lock, or a
game started) or full. `HostTopology::join_request` checks the request (`Denial`: `Malformed`,
`PasswordMissing`, `PasswordWrong`, `RecordSize`, `NotOnMetaserver`) and on success returns every
0x8003 pair: each listed player's record to the newcomer and the newcomer's to each, flag bit 0 on
the receiver's own. `HostTopology::player_changed` takes a 0x8001 and relists it to everyone;
`HostTopology::remove` sends 0x8005 to the rest. The host's net-layer messages go out with sender
`NET_LAYER_SENDER` (0xfe).

## The lobby (`mandrake_net::lobby`)

**Flow.** Joiner connects and sends greeting + join request; host lists everyone and sends the
description (type 6) to all. Players change their records (Ready, team, colours) with 0x8001; the
host relists them. A player's requests to the host go as configuration `{flags 1, code, value}` to
net id 0 (`lobby::configuration_request`); the host broadcasts `{flags 0, code, value}`
(`lobby::configuration_broadcast`). Codes (`lobby::code`): `CANCEL` 4, `SEAT` 0xb (team request:
the value is a leader's net id to join, or -1 to leave), `START` 100, `BEGIN_MIDGAME` 0x65 (not
supported).

**Joiner side** (`Lobby`): keeps the last description, the vets if sent, and whether code 100 has
come. `lobby::check` (and `lobby::check_as` for a `GameKind`) refuses a description whose gameplay
version is outside `VERSION_RANGE` (0x867..=0x897) or that is the other game's
(`InvalidGame::OtherGame`). At code 100, `Lobby::new_game_data` builds the new-game data with our
player local.

**Host side** (`HostLobby`): holds the description (`HostLobby::set_description`,
`HostLobby::description_message`), the start, the barriers' host side and the vets.

- `HostLobby::start(machine_ticks)`: new seeds, the pseudo-plugin dropped, then the description
  and `{0, 100}` to everyone. `Host::start` sends the vets first if any, clears the server's
  history, closes the game to new connections and stops LAN advertising.
- `HostLobby::reopen`: the lobby after a game (option 0x40 set again, the pseudo-plugin put back).
- `HostLobby::begin_barrier(kind)` / `HostLobby::sync_data` / `HostLobby::barrier_done`: see
  barriers below.

**Teams and other lobby rules**, each returning the records to send as 0x8001s, in order:
`team_request` (configuration 0xb), `team_kick`, `disband_team`, `leave_every_team` (options
without teams), `host_options_records` (option 0x10000 changed), `restart_records` (every Ready
cleared: before a Restart Game's code 100 and after new options), `boot_unready` (Boot: the
booted player's teammates' Ready lowered in the host's own table only). `list_players` builds the
lobby list with the original colour arbitration (a clash within 0x3f00 on each of R, G, B takes
one of 20 substitute colours); a chat line's colour words are the sender's row colour.

**Chat**: a player's line goes to the host's player flagged `PASS_ON` (`lobby::chat_message`);
the host's player passes it on (`lobby::pass_on_chat`): to everyone one copy to 0xff; to a player
one to it and one back to the sender; to a team one to each member and one to the sender if not
among them. Chat is taken only outside a game (`Joiner::in_game`).

## The game side: lockstep (`mandrake_net::client`, `mandrake_net::server`)

The original game is a **command lockstep with a central stamping server** run by the host:

1. A peer's order goes up as a type-1 message to 0xfe, **unstamped**: player 0xff, tick 0
   (`client::command_message`, `Joiner::send_command`).
2. The host's game server stamps it (`Server::stamp`): the player byte becomes the sender's net id
   (except a DROP_PLAYER from the host's own player, which keeps its player byte), and the tick
   becomes the server's tick `(now_ms - start) * 30 / 1000` in 32-bit wrapping arithmetic
   (`Server::tick`, `server::TICKS_PER_SECOND`). Every non-NULL stamped command is kept in
   `Server::history`.
3. The stamped command goes to **every arrived player, the sender included**, each copy addressed
   to its id from net id 0 (`Host` routing; the host's own player gets it over the loopback).
4. Every peer queues each stamped command into its sim in arrival order, records it in its film,
   and moves its *horizon* to the command's tick if higher (`Client::handle`, `Client::horizon`).
   A NULL (type 0) moves the horizon and is neither queued as an order nor recorded.
5. A peer runs a tick body only below its horizon. The sim's tick gate (in `mandrake-sim`) keeps a lead:
   no body runs until the horizon is `mandrake_sim::game_speed::NET_LEAD` (5) ticks ahead; twelve
   starvations (`NET_STARVATIONS`) in a window of `NET_WINDOW` (600 input polls, 20 s) raise the
   lead by `NET_LEAD_STEP` (2). The gate (`mandrake_sim::game_speed::NetGate`) is local to each peer and
   nothing of it is sent.

Because TCP delivers in order and the server stamps in the order it receives, every peer sees the
same commands with the same ticks in the same order, and each command runs at the tick body it
was stamped with. Nobody waits for anybody per tick; a slow peer just falls behind the horizon.

**The server clock** starts with server command 0, which the host's own client sends once the
pre-game barrier is through (`Host` does this itself when the barrier ends), and stops with server
command 1 at the end barrier. Only the host's own player may start or stop it; our host ignores a
server command from anyone else.

**The heartbeat** (`Server::idle`): while the clock runs, a stamped NULL (player 0xff) once
`server::HEARTBEAT_MS` (101 ms) pass with nothing sent, so horizons keep moving when nobody gives
orders.

**The ping** (`Server::ping`, `Server::pong`): once more than `PING_INTERVAL_MS` (1000 ms) since
the last, a ping to every player whose record version is above `PING_MIN_VERSION` (0x883),
carrying the server's ms clock and each listed player's latency in table order; a latency is
`NO_LATENCY` (shown as -1) after `PONG_SILENCE_MS` (4 s) with no pong. A pong is taken only if it
echoes a time no later than the last ping and no earlier than that player's last pong; the round
trip is smoothed `(rtt + 3 * old) / 4`. Latencies are display only: they never reach a sim or a
film. They arrive as `Event::Latencies`.

**The SYNC** is not a network-layer feature but an ordinary command: the host's sim (the network
originator, `mandrake_sim::net_game::NetRole::Host`) queues a SYNC (type 1) every
171 ticks (`mandrake_sim::SYNC_TIMER_RESET` plus one) of its requested time (`Simulation::net_end_of_batch`); it goes
through the server like any order, and every peer's sim compares it with its own state at that
tick. See [simulation.md](simulation.md).

**SYNC_FORMATION** (type 16): each peer's first tick sends its default formation definition
(`client::DEFAULT_FORMATION`, 20 big-endian s16, the sim's static default).

### Barriers

A barrier (pre-game, `lobby::BarrierKind::Start`; end of game, `BarrierKind::End`):

1. The host sends begin-sync (type 4) to everyone (`Host::begin_barrier`; the end one stops the
   server first). A peer counts barriers from the join (`Client::barriers_begun`: odd = pre-game,
   even = end) and is "in a game" from a pre-game begin-sync until its end barrier is through.
2. Each peer, once loaded (pre-game) or once its game is over (end), answers with
   player sync data `{its id, 1}` **to net id 0** (`Client::answer_barrier`, `Joiner::loaded`).
3. The host marks it arrived (`HostLobby::arrived`; only arrived players receive stamped
   commands), and once every listed player has arrived it broadcasts each player's entry with
   value 0, in list order (`Event::AllSynced` on each peer, `HostEvent::BarrierDone` on the host).
   A player who leaves during a barrier counts as arrived.

### Drops

When a joined player's connection is lost, the host sends 0x8005 to the rest. In a game, the
host's own player also sends a DROP_PLAYER command (`client::drop_player_message`: type 9, payload
empty, **player byte = the dropped id**, tick the game's), which the server stamps keeping that
player byte; every peer's sim then removes the player at the same tick. A player lost during the
pre-game barrier is held and dropped once the barrier is through; a player who never arrived in
the game (or a loss in the lobby or postgame) gets no DROP_PLAYER. If the **host** is lost, a
joiner gets `Event::HostLost` and the game is over for it (the session gives up the end barrier,
`mandrake_session::net_game::NetLoop::end_given_up`).

Our host also closes a connection that hasn't joined within `endpoint::JOIN_REQUEST_TIMEOUT_MS`
(10 s) of being seen; the original host would keep it.

### End of game, Rehost and Restart

- **Normal end**: the sim's game-over (for a time limit, `client::END_GAME_COUNTDOWN`, 300 ticks
  after the limit). Each peer then answers the end barrier; the host's own game-over begins it
  (`mandrake_session::net_game::NetLoop::map_over`). After it the postgame screen ("endi") opens.
- **Forced end** (the pause dialog's Rehost Game and Restart Game): a command
  `mandrake_sim::net_game::FORCED_ENDGAME` (0x15) with an s16 kind, 2 = Rehost (to the postgame), 3 =
  Restart (no postgame: the next game starts at once with the same players). Only the network
  originator (the host) runs it (`Simulation::force_network_endgame`), so it reaches every peer
  stamped from player 0. On our dedicated server the owning client asks for it with
  `Ours::ForceEnd`.
- **Restart Game** in the postgame: the host clears every Ready and resends the records
  (`Host::clear_ready`, `lobby::restart_records`), then code 100 to everyone
  (`Host::restart_game`), and the lobby reopens (`HostLobby::reopen`, `Host::set_after_game`).

### Pause and game speed

Only in a co-operative game, and only from the host's player: pause is command
`mandrake_sim::net_game::PAUSE` (0x17, `{s16 paused, s16 0}`), speed `mandrake_sim::net_game::GAME_SPEED`
(0x18, `{s16 index, s16 0}`). They are stamped like any command; every peer pauses before the body
of the pause's tick (`Simulation::net_body_may_run`), and while paused the head of the tick loop
still takes what arrives up to the horizon (`Simulation::net_head`).

### Veterans

After a game, the sim's surviving units' monster initializers (`Simulation::monster_initializers`,
with the co-op adjustment applied) are given to the host (`Host::set_vets`). At the next start,
`HostLobby::start_vets` builds a type-8 VETS message when the gameplay version is at least
`VETS_MIN_VERSION` (0x87a) and the options have both `OPTION_VETERANS` and `OPTION_VETS`; it is
**sent only in a co-operative game**, before the description. With none built, option 0x40 is
cleared from the description. A TFL game has no VETS message.

### The film of a network game

Each peer records its own film: the new-game data with its own player local, every stamped
command received in arrival order (NULLs excluded), no monster initializers, and the end tick
(`client::film`, `mandrake_rec::writer::Recorder`). Every peer's film should therefore hold the same
commands at the same ticks; the original game can replay any of them.

## Endpoints (`mandrake_net::endpoint`)

**`Joiner`** = `Topology` + `Lobby` + `Client`, with its splitter and outgoing buffer.
`Joiner::new(request, udp_port, now_ms)` (or `Joiner::new_as` with a `GameKind`) queues the
greeting and the join request. `Joiner::poll(now_ms)` returns `Event`s in order: `Greeting`,
`Joined`, `Denied`, `JoinTimedOut`, `Player`, `PlayerLeft`, `Description`, `Configuration`,
`Chat`, `Vets`, `Latencies`, `BeginSync`, `AllSynced`, `Commands { commands, horizon }`,
`Ignored`, `Malformed`, `Undecoded`, `StreamError`, `HostLost`, and ours `Owner`, `Downloads`.
Actions: `Joiner::set_ready`, `Joiner::team_button`, `Joiner::team_kick`, `Joiner::team_lock`,
`Joiner::chat`, `Joiner::request_configuration`, `Joiner::loaded`, `Joiner::send_command`,
`Joiner::new_game_data`, `Joiner::new_map` (horizon back to 0). Each returns false when it can't
act yet (no id, or in a game).

**`Host`** = `HostTopology` + `HostLobby` + `Server` + a peer table by net id, with its own player
a `Joiner` (net id 0) on an in-process loopback. `Host::new(options, udp_port, now_ms)` takes
`HostOptions` (the host's own `JoinRequest`, the password, the description). `Host::poll(now_ms)`
reads every peer, routes, runs the heartbeat and the ping, feeds the host's own player and returns
its `Event`s; `Host::take_events` returns the host-only `HostEvent`s (`Accepted`, `Refused`,
`Joined`, `Denied`, `PlayerChanged`, `PlayerLeft`, `ConfigurationRequest`, `Arrived`, `Ours`,
`BarrierDone`, `ServerStarted`, `ServerStopped`, `Chat`, `RecordRefused`, `NotRelayed`,
`Undecoded`, `StreamError`).

Routing is on header byte 4: 0xfe to the host's net layer or server; 0xff to every joined player,
sender included; 0 to the host's own player; any other id to that player. The host rewrites the
sender byte (byte 5) to the connection's id, so a peer can't impersonate another.

Host actions: `Host::start`, `Host::begin_barrier`, `Host::set_options` (Game Options OK: new
description to all, then every Ready cleared), `Host::configure`, `Host::set_password`,
`Host::set_accepting` (Lock), `Host::kick`, `Host::boot`, `Host::chat_as` (a line in another name,
as the autohost's), `Host::restart_game`, `Host::set_vets`, `Host::start_preview` (the new-game
data a start would give, with nothing sent; our server loads its map from it first).

**`Endpoint`** puts either on sockets: `Endpoint::join` / `Endpoint::join_as` (TCP),
`Endpoint::join_over` (any `ByteStream`), `Endpoint::host` / `Endpoint::host_at` (listens; port 0
picks one). `Endpoint::poll(now_ms)` reads every socket, accepts connections, answers LAN queries
while advertising, and polls the state machine; `Endpoint::flush` sends; `Endpoint::wait(timeout)`
sleeps until something arrives; `Endpoint::leave` closes. `Endpoint::joiner` / `joiner_mut` give
our own player (for a host, its local `Joiner`); `Endpoint::as_host` / `as_host_mut` give the
`Host`.

## How the session and the sim drive it

`mandrake_session::net_game::NetLoop` is the one network-game loop both the client and the dedicated
server run, from code 100 to the end barrier ([session.md](session.md)). Per frame:

1. `NetLoop::poll_events(sim, ..)`: `Endpoint::poll`; each stamped command goes into the sim with
   `Simulation::queue_incoming_command` (as a `mandrake_rec::CommandPacket` with the stamped tick and
   player byte), in arrival order; barrier ends and the host's loss are noted.
2. The sim's tick gate (`mandrake_sim::game_speed::GameSpeed::advance_network` over a
   `mandrake_sim::game_speed::NetGate`) decides how many tick bodies this frame may run, from the
   horizon (`Simulation::net_horizon`), the lead and the 30 Hz poll clock
   (`mandrake_session::net_game::PollClock`).
3. The bodies run; then `Simulation::net_end_of_batch` (the host queues its SYNC).
4. `NetLoop::send_outgoing(sim, ..)`: `Simulation::take_outgoing_commands` up the endpoint
   unstamped (`Joiner::send_command`), then flush.
5. `NetLoop::end_poll`: a NULL heartbeat's horizon is passed to the sim
   (`NetLoop::heartbeat`); barriers are answered once loaded or over (`NetLoop::map_loaded`,
   `NetLoop::map_over`).

The sim is reached only through the `mandrake_session::net_game::NetSim` trait (`current_tick`,
`net_horizon`, `queue_incoming_command`, `take_outgoing_commands`); `mandrake-net` and `mandrake-session` do
not depend on `mandrake-sim`.

## What breaks compatibility or causes out-of-sync

A network game is in sync only while every peer's sim gets identical inputs. The sim reports
out-of-sync (`Simulation::game_is_out_of_sync`, one log line, and the game goes on) when a SYNC
from the host differs from its own state, when a command runs at a later tick than it was stamped
with, or when a tick body would run at or past the horizon. Things that cause it or break joining:

- **The new-game data must be byte-identical on every peer.** It is built locally from the last
  description and the player list. Different records (especially `net_id`), a different sort
  (`sort_players`), or a different plugin list gives different sims. Example: a joiner's record
  is listed as it sent it (net id 0) until its own 0x8001; our owning client resends its record
  before code 100 (`Joiner::owner_configure`) because above gameplay 0x731 a command's player is
  found by the records' net ids, and a stale 0 made its orders run as someone else's.
- **Every peer must run each stamped command at its stamped tick, in arrival order**, and nothing
  else: no local stamping in a network game (the sim's mode 1), NULLs only move the horizon.
- **The same data**: the same tags and plugins (the plugin list's checksums), the same gameplay
  version (0x889 for 1.8.5). Joiners outside `lobby::VERSION_RANGE` are refused.
- **The seed** comes only from the description; the host must reset it at the start, and the
  description sent with code 100 is the one every peer builds from.
- **Byte order**: everything big-endian except the ping's tick word; chat text +13.
- **Ordering of the player table**: pings carry latencies by table position; a different order
  shows wrong latencies (display only).
- **Message sizes**: nothing above 0x1000 bytes; a send queue above 0x4000 drops the player.
- **The record's unused bytes** (`field_22`, the bytes after a name's NUL) travel and end up in
  films; keep them as received.
- **Unknown types**: the original engine's game dispatcher drops types at or above 0xe, so our
  extension types (0x40..) are harmless to it, but they are only ever sent to our own clients.

## Our extensions (`mandrake_net::ours`)

Messages between our client and our dedicated server only; a copy of the original game never
receives one. Types `0x40..=0x47` (`ours::kind`, `ours::is_ours`):

| Type | Variant | Direction | Body |
|-|-|-|-|
| 0x40 | `Ours::Hello` | our joiner to 0xfe | u16 extension `VERSION` (1) |
| 0x41 | `Ours::Owner` | server to the owner | u16 the owner's net id |
| 0x42 | `Ours::Description` | owner to 0xfe | a type-6 body (the owner's Game Options) |
| 0x43 | `Ours::Boot` | owner to 0xfe | u16 net id |
| 0x44 | `Ours::Lock` | owner to 0xfe | u16 1 close / 0 open |
| 0x45 | `Ours::Password` | owner to 0xfe | password\0 (empty: none) |
| 0x46 | `Ours::ForceEnd` | owner to 0xfe | s16 forced-end kind (2 Rehost, 3 Restart) |
| 0x47 | `Ours::Downloads` | server to each Hello | base URL\0 (at most `DOWNLOADS_URL_MAX`) |

The handshake: the server marks its own record's `field_22` with `ours::SERVER_MARK`
(`"m2sv\0\x01"`); to the original game that record is an ordinary observer's. A joiner that sees
the mark on player 0 (`Joiner::server_is_ours`) sends Hello once it has an id. The server makes the
first one in the lobby the owner (`Event::Owner`, `Joiner::is_owner`); only the owner may send the
rest (`Joiner::send_ours`) and configuration code 100 to 0xfe (`Joiner::owner_configure`: Start in
the lobby, Restart Game in the postgame). `Ours::Downloads` gives the base URL for plugin downloads:
each plugin at `<base><checksum as 8 lowercase hex digits>`, `{host}` standing for the host name
the client joined by.

**Dedicated-server hardening** (only when the host's own record carries the mark,
`Host::is_dedicated`): a player may change only its own record, or end its team for another
(`HostTopology::team_ended`; else `HostEvent::RecordRefused`); and a player's command, begin-sync,
barrier entry to anyone but net id 0, or non-request configuration is not relayed
(`HostEvent::NotRelayed`). A client that hosts keeps the original host's behaviour and relays
everything.

## LAN game list (`mandrake_net::lan`)

UDP broadcast, as the original game:

- **Query** (`lan::query`): 512 bytes (`QUERY_SIZE`), the word `fa fb fc fd` (`QUERY_WORD`) 128
  times, broadcast to `255.255.255.255:3453` (`lan::BROADCAST`) every `QUERY_INTERVAL_MS` (500 ms)
  from the browser's own UDP socket on an ephemeral port with `SO_BROADCAST`. `lan::is_query`
  checks one.
- **Answer** (`lan::Answer`, 0x38 bytes, `ANSWER_SIZE`, big-endian): `+0x00` magic
  `ANSWER_MAGIC` (`af bf cf df`), `+0x04` game id `GAME_ID` (`'myth'`), `+0x08` version
  `VERSION` (0x100), `+0x0c` the host socket's IPv4 address (0 when bound to all), `+0x10` u16
  port, `+0x12` u16 0, `+0x14` the game name (31 bytes and a NUL), `+0x34` u32 extra length (0).
- **Host**: the listener's UDP socket answers a query to its sender while the host advertises
  (`Host::advertising`: in the lobby, not in a game), with `Host::lan_answer`; `lan::answer_for`
  is the pure function.
- **Browser** (`LanList`, `lan::Browser`): every answer with the magic and our game id is a game at
  the answer's *source* address and port (not the packet's own address fields); at most
  `PENDING_MAX` (10) new answers a poll; a known address refreshes its entry; a game silent for more
  than `FORGET_MS` (2 s) goes. `LanList::update` returns `LanEvent`s and whether to send the query.

## TFL games (`mandrake_net::game_kind`, `mandrake_net::tfl`)

The same wire carries a Myth: The Fallen Lords game; see [tfl.md](tfl.md). `GameKind::of` tells
the kind from the description alone: a TFL description's parameters `version` (offset 0x24) is
`game_kind::TFL_GAMEPLAY_VERSION` (the bytes `"TFL5"`), outside the 1.8.5 range, so an original
1.8.5 joiner refuses it and our Myth II client refuses it with `InvalidGame::OtherGame` (and the
reverse). The original TFL engine writes 0 there and a TFL film never keeps those bytes, so the
marker never reaches a film. What changes for TFL:

- command codes (`game_kind::TFL_COMMANDS` versus `MYTH2_COMMANDS`): DROP_PLAYER 8, CHAT 9,
  UNIT_ADJUSTMENT 11, nothing above 13 (no forced end, pause or speed);
- the heartbeat: a NULL once the server's tick is more than `server::TFL_HEARTBEAT_TICKS` (3) past
  the last command sent; TFL had no ping, ours still goes (lobby latency only);
- the description: TFL's parameters, no plugin list, and at body 0x22c the inverted tag-archive
  checksum and at 0x230 the active plugin's checksum (`tfl::description`, `tfl::check_data`,
  `tfl::DataChecksums`, `tfl::NO_PLUGIN`);
- the new-game data in TFL's film form (`tfl::new_game`, `Joiner::tfl_new_game`);
- a received command's player byte mapped from net id to the player's index
  (`tfl::player_order`, `tfl::player_index`) before it is queued and recorded;
- no VETS message.

## Test binaries

`src/bin/` has two small headless command-line tools that host or join a game with no sim (they
send the default SYNC_FORMATION, answer both barriers and record a film of the stamped commands).
They are development aids for checking wire compatibility by hand against other copies of the game;
nothing else depends on them, and the client and server don't use them.

## Verifying changes

All of these are self-contained (no game data, a fake clock, in-memory pipes or loopback sockets):

```sh
cargo test -p mandrake-net                     # everything below
cargo test -p mandrake-net --lib               # unit tests in wire, transport, server, lobby, lan, ...
cargo test -p mandrake-net --test wire         # greeting, chat +13, ping/pong byte order, splitter limits
cargo test -p mandrake-net --test host         # Host state machines (stamping, heartbeat, ping, barriers,
                                         # drops, teams, chat, owner and hardening, LAN on loopback)
cargo test -p mandrake-net --test joiner       # Joiner rules (5 s timeout, denial, pong, horizon, barriers)
cargo test -p mandrake-net --test tfl          # TFL marker, codes, heartbeat, player-byte mapping
cargo clippy -p mandrake-net --all-targets -- -D warnings
```

What to check for a change:

- **Wire format**: add a round-trip test (`Message::encode` then `Message::decode` equal; split
  the bytes at every point through `Splitter` and get the same frames).
- **Lockstep**: a host and a joiner with a fake clock (`Host::new`, `Joiner::new`, shuttling
  `take_outgoing`/`receive`): every peer must receive the same stamped commands with the same
  ticks, including the sender's own.
- **New-game data**: build it on two peers (`Joiner::new_game_data`) and compare the bytes.
- **End to end with your own game data installed**: host with one client
  (`mandrake --net-host 3453` with a map, see [client.md](client.md)) and join with another
  (`mandrake --net-join 127.0.0.1:3453`),
  or join the dedicated server ([server.md](server.md)); play past several SYNCs and check that
  neither log reports OUT OF SYNC, and that the films both peers wrote replay to the same end
  ([films.md](films.md)).
