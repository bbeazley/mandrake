# Dedicated server (mandrake-server, docker/)

The `mandrake-server` crate is a headless dedicated server for Myth II 1.8.5 network games, plus a
WebSocket proxy and small web site that let browsers play on it, and the `docker/` folder that
packages all of it as one container. This file covers the server's game loop, its config file,
who starts a game (the owner, or the autohost rule), the proxy and the site, and how to build and
run the container. The network protocol itself is in [networking.md](networking.md); the web
client the site serves is in [web.md](web.md); the client's own settings files are in
[configuration.md](configuration.md).

## What the server is

One process hosts one network game at a time, lobby to postgame, then the next, on TCP port 3453
by default. To every copy of the original game that joins, it looks like an ordinary original
host with **"Server is Observer"** on:

- the server is net id 0, a player of its own game whose record has flag 0x40
  (`mandrake_net::lobby::FLAG_NO_TEAM_CHANGE`) and whose game options always have 0x10000
  (`mandrake_net::lobby::OPTION_HOST_NO_TEAM`); the sim gives that player no units;
- it runs the game server: it stamps and relays every order, sends the heartbeat and the pings,
  runs both barriers, and builds the DROP_PLAYERs and the forced end as the original host does;
- it runs **its own sim** of every game (network mode, role host,
  `mandrake_sim::net_game::NetRole::Host`) from the stamped commands, so the game ends when the sim says
  so (time limit, victory), the SYNC checksums come from it, and it writes **a film per game**;
- it logs everything to stdout (`tracing`, filtered by `RUST_LOG`, `info` by default).

The binaries: `mandrake-server` (the server), `mandrake-ws-proxy` (the WebSocket proxy and web site),
`mandrake-ws-tunnel` (a test aid). Module map:

| Module | What |
|-|-|
| `mandrake_server::config` | the server's TOML file (`Config`, `GameConfig`) |
| `mandrake_server::game` | one lobby and its games: `Server::host_one`, the tick loop, films |
| `mandrake_server::owner` | the game's owner (our first client) and its requests (`Owner`, `Policy`) |
| `mandrake_server::autohost` | the original autohost start rule, for a lobby with no owner |
| `mandrake_server::proxy` | the WebSocket proxy (`ProxyConfig`, `proxy::run`, `proxy::tunnel`) |
| `mandrake_server::ws` | RFC 6455 framing and handshake, std only |
| `mandrake_server::site` | the web site on the proxy's port (`SiteConfig`, `site::answer`) |

The crate uses `mandrake-net` for the protocol, `mandrake-session` for the shared network-game loop and the
default description, `mandrake-sim` for the sim, `mandrake-rec` for films and `mandrake-tag` for the game data.

## `mandrake-server` command line

```
mandrake-server CONFIG.toml [--games N] [--lobby-timeout SECONDS]
```

It hosts lobbies one after another (`Server::host_one` in a loop) until `--games N` games have
been played (default: forever). `--lobby-timeout` ends a lobby that hasn't started in that time
(and the process, after that lobby). Each finished game is logged with its end tick, its
out-of-sync count and its film. The exit code is non-zero if any game went out of sync, stopped
(the server's sim hit something it can't run) or had its start refused (the map didn't load).

## Config file (`mandrake_server::config`)

`Config::load` reads a TOML file; unknown keys are errors. Relative paths are relative to the
config file's folder.

| Key | Default | Meaning |
|-|-|-|
| `data_dir` | required | the Myth II folder (its `tags/`, and `plugins/` unless `plugins_dir`) |
| `plugins_dir` | `data_dir/plugins` | the plugins folder, if elsewhere |
| `port` | 3453 (`config::DEFAULT_PORT`) | the TCP port (and its UDP twin, see networking.md) |
| `name` | `"Myth II Server"` | the server's player name and the game's name |
| `films_dir` | `films` | where each game's film goes |
| `work_dir` | `work` | where a game folder is put together when `plugins_dir` is apart from `data_dir` |
| `download_url` | `config::DEFAULT_DOWNLOAD_URL` (`http://{host}:8080/plugins/`) | base URL for our clients' plugin downloads; `""` offers none |
| `[game] description_file` | none | a type-6 message as it travels, as the base description |
| `[game] map` | | the map's mesh tag, four characters (required unless the base has one) |
| `[game] game_type` | | the game type (0 Body Count, 2 Last Man on the Hill, ...) |
| `[game] time_limit_minutes` | | the time limit (1800 ticks a minute) |
| `[game] difficulty` | | |
| `[game] max_players` | | at most this many players; also the cap on the owner's choice |
| `[game] cooperative` | | true/false: game option 0x80 |
| `[game] password` | `""` | the game's password |

`Config::description` builds the first lobby's description: the base (the description file's,
or a new host's from `mandrake_session::net::host_description`), then the `[game]` keys over it, the
server's name as the game name, option 0x10000 always set, and the password flag from the
password. A description with no map is an error.

`Config::game_dir` is the folder the engine's tag search reads (`<dir>/tags`, `<dir>/plugins`):
`data_dir` itself, or, when `plugins_dir` is elsewhere, `work_dir/game` rebuilt at each start with
symbolic links to `data_dir`'s entries and to `plugins_dir` as `plugins` (Unix only).

`download_url` must start with `http://` or `https://`, end in `/`, and be at most
`mandrake_net::ours::DOWNLOADS_URL_MAX` bytes. `{host}` in it is replaced by the client with the host
name it joined by, so one config works for any address.

`docker/server.toml` is the image's default: `data_dir = "/data/myth2"`,
`plugins_dir = "/data/plugins"`, `port = 3453`, `films_dir = "/films"`,
`work_dir = "/tmp/mandrake-server"`, `download_url = "http://{host}:8080/plugins/"`, and a first game on
`map = "85gi"` (Gimble in the Wabe), `game_type = 0`, `time_limit_minutes = 10`,
`max_players = 16`.

## Who starts the game

### The owner (`mandrake_server::owner`)

The **first of our clients to join a lobby** owns it. Our clients recognise the server by the mark
in player 0's record (`mandrake_net::ours::SERVER_MARK`) and say `Ours::Hello`; `Owner::hello` makes the
first one in a lobby that hasn't started the owner and tells it with `Ours::Owner`. Every Hello
sender also gets `Ours::Downloads` with the config's `download_url` (unless it is `""`).

The owner has the host's controls through extension messages (`Owner::request`):

- `Ours::Description` (the owner's Game Options): checked by `owner::check_description` (a 1.8.5
  game, every plugin present in the game folder by name with the description's checksum, the map
  among the tags), forced to keep option 0x10000, held to the config's `max_players` (the owner is
  told in chat, as `owner::SERVER_NAME`), its password flag set from the password in force, then
  applied as the host's Game Options OK (`mandrake_net::endpoint::Host::set_options`: new description
  to everyone, every Ready cleared). A refused one sends the current description back to the
  owner.
- `Ours::Boot`: the host's Boot Player (`Host::boot`).
- `Ours::Lock`: Close Game / Open Game (`Host::set_accepting`).
- `Ours::Password`: the password from now on; an empty one keeps the config's (the owner never
  sees the config's password).
- Configuration code 100 to 0xfe: **Start** in the lobby (refused while any other player isn't
  Ready; the owner need not be) and **Restart Game** in the postgame.
- `Ours::ForceEnd` in a game: the pause dialog's Rehost Game (2) or Restart Game (3). The server,
  being the host, runs the forced end itself, so every peer gets command 0x15 stamped from
  player 0.

`Policy` holds what the server keeps from the config over the owner's requests (the password and
the maximum of players). **When the owner leaves, the game ends for everyone**: in the lobby the
server closes it; in a game it sends the forced end (Rehost), writes the film once that has come
back stamped, runs the end barrier and leaves.

### The autohost rule (`mandrake_server::autohost`)

A lobby with no owner (only original-game players) starts by the original autohost rule
(`Autohost::poll`, `Autohost::holds`, over `Seat`s, giving `Action`s). Each lobby poll, from 5 s
after the lobby opened, the server's own player left out:

- `n` players, `r` of them ready, `k` ready "sides" (a ready player with no team is one; a team is
  one if any of its players is ready);
- when `r >= max(2, (n + 1) / 2)` and `k >= 2` (1 in a co-operative game) a 20 s countdown runs,
  announced in chat in the name `autohost::NAME` ("Autohost"): "Game will begin in 20
  seconds...", then at 10 s and 15 s, and at 20 s "Starting game...": players not ready are
  kicked and the game starts;
- if the rule stops holding during the countdown: "Game start aborted.".

After a game with no owner the postgame waits `game::AUTOHOST_RESTART` (30 s, saying "Game will
restart in 30 seconds.") and restarts with the same players.

## One lobby and its games (`mandrake_server::game`)

`Server::new(config)` prepares the game folder, the description and `films_dir`.
`Server::host_one(addr, lobby_timeout)` runs one lobby to its end and returns `Hosted` (its
`GameResult`s, whether it timed out, the refused starts):

1. **Lobby.** `mandrake_net::endpoint::Endpoint::host_at` with the server's own player (its record
   carrying `SERVER_MARK`, which also turns on the host's hardening, see networking.md). The loop
   polls the endpoint, handles `HostEvent`s (Hello, the owner's requests, its Start, its leaving),
   runs the autohost rule when there is no owner, logs latencies every 10 s, and waits for
   arrivals (`Endpoint::wait`, 5 ms at most).
2. **Start with the map loaded first.** Before the Start goes out, the server takes the new-game
   data the Start will give (`Host::start_preview`), loads the map and the sim from it
   (`game::start_sim_with`), and only then sends the Start (`Host::start`) with the same seeds. A
   map that doesn't load is a refused start: nothing is sent, the players stay in the lobby, the
   owner (or everyone, in the autohost's name) is told "Cannot start: the map doesn't load on the
   server." When the server's own code 100 comes back, the new-game data is compared with the one
   loaded; if it differs the map is loaded again.
3. **Play** (`Server::play`). The tick loop, the same as the client's headless network game:
   - poll (`mandrake_session::net_game::NetLoop` inside `Net`): stamped commands into the sim in arrival
     order, the horizon, barrier ends;
   - once the pre-game barrier is through, the 30 Hz poll clock
     (`mandrake_session::net_game::PollClock`), the head of the tick (`Simulation::net_head`, which also
     takes a pending game-speed change), the tick gate
     (`mandrake_sim::game_speed::GameSpeed::advance_network`), and the tick bodies;
   - after the batch, the SYNC timer and the sim's outgoing commands (the SYNCs, DROP_PLAYERs,
     the forced end) up the host's own loopback, unstamped, like any player's;
   - a frame is about 4.17 ms (`FRAME`); the loop waits for arrivals for the rest of it, so a
     command is stamped and relayed as it arrives, not at the next frame.
4. **End.** At the sim's game over (or when everyone has left) the film is written and the end
   barrier begins. Then the postgame: with an owner it waits up to 30 minutes for its Restart
   Game; with none, the autohost's 30 s restart; a Restart (forced end kind 3) has no postgame and
   the next game starts at once. A restart reopens the lobby on the same endpoint with the players
   kept and the last game's veterans (the sim's `Simulation::monster_initializers`, sent as the
   start's VETS in a co-operative game). After `--games N` games, or with no one left, the server
   waits 5 s (so every peer sees the end barrier through) and leaves.
5. **A stop.** If the server's sim stops (it reached something it can't run), the server logs it
   once and ends the game for everyone as the host's Rehost Game would: the forced end (kind 2)
   from player 0, the film written once it has come back stamped (or after 5 s), the end barrier,
   and the postgame. Before the game began, the players are dropped instead.

The server never takes part in the game as a player: it gives no orders, and its observer record
is placed after the players in the sim's player order.

### Films

`<films_dir>/<NNN>-<unix seconds>.m2rec`, where NNN (three digits) is the game's number since the server
started, as each peer records it: the new-game data with the server's player local, the last
game's veteran table, every stamped command received in order (NULLs excluded), and the end tick
(the forced end's tick when the game was ended by one). The film's tag name is
`Server Game <n>`. Any copy of the original game can replay it; it should hold the same commands
at the same ticks as every player's own film ([films.md](films.md)).

### Logging

Every line goes to stdout. Lines are prefixed by phase (`lobby:`, `game:`, `mandrake-server:`); they
cover joins and leaves, the owner's requests, the autohost's lines, refused records and messages
of the hardening, every DROP_PLAYER and CHAT as its tick runs, the lead, the lobby's round trips,
every out-of-sync report of the server's sim, and each game's summary.

## The WebSocket proxy (`mandrake_server::proxy`, `mandrake_server::ws`)

Browsers can't open raw TCP, so `mandrake-ws-proxy` carries a game's TCP stream over a WebSocket:

```
mandrake-ws-proxy CONFIG.toml [--listen ADDR]
```

- **Plain HTTP and WebSocket only**; TLS ends in front of it (a reverse proxy or tunnel). One
  port serves the WebSocket, the web site and `GET /healthz` (JSON:
  `{"status":"ok","sessions":N,"max_sessions":M}`). Methods other than GET and HEAD get a 405
  (and are logged with their path). A request head must arrive whole within
  `proxy::HANDSHAKE_TIMEOUT` (10 s).
- **Targets come from the config only**: the WebSocket path `path` (default `/myth2-ws`) goes to
  `target`; `<path>/<name>` or `?game=<name>` goes to `[targets].<name>`; anything else is a 404.
  The proxy is never an open relay. `max_sessions` sessions at once, then a 503.
- **Each session** is one TCP connection (`TCP_NODELAY`, connect timeout
  `proxy::CONNECT_TIMEOUT`, 10 s) to its target. Bytes pass unchanged both ways: each TCP read is
  one binary message to the browser; each binary frame's payload goes into the TCP stream. Text
  frames are dropped.
- **The greeting is rewritten**: the browser's 2-byte greeting (port 0) is replaced by the port of
  a UDP socket the proxy binds for the session, because the host connects its UDP socket to the
  joiner's port (nothing is sent on it).
- **The host's pings are answered by the proxy** once the page has sent one pong of its own (which
  gives its player id): each ping is answered at once with the same bytes the page would send, and
  the page's own pong for it is dropped, so the host still sees one pong per ping and a background
  tab doesn't show huge latency. From then on the browser's stream is cut at message boundaries;
  anything that isn't a Myth message stream stops the answering and passes as it is.
- **Buffers**: each way at most `proxy::BUFFER_CAP` (1 MiB); the TCP side is always read, so a slow
  browser never stalls the host; a browser that falls that far behind is closed.
- **Closing**: the upstream's EOF sends a close frame (1000) after what's buffered; the browser's
  close frame or EOF closes the upstream after what's buffered; a protocol error closes with 1002,
  a frame over `ws::MAX_PAYLOAD` (65536) with 1009.

Threads, not an event loop: per session a reader and a writer each way (`proxy::run_session`).

`mandrake_server::ws` is RFC 6455 with no extensions: `ws::accept_key` (SHA-1 and base64 of the key
and the GUID), `ws::encode`, and `ws::Decoder` (client frames must be masked, server frames must
not; fragmented messages are passed on frame by frame; a reserved bit is an error).

`mandrake-ws-tunnel LISTEN ws://HOST:PORT/PATH [--once]` is the same session the other way round
(`proxy::tunnel`, `proxy::connect_ws`): each TCP connection accepted on `LISTEN` is carried
through a WebSocket to the proxy, so a native client (`mandrake --net-join LISTEN`) plays as a
browser would. With `--once` it exits when the first connection ends.

### Proxy config (`ProxyConfig`)

| Key | Default | Meaning |
|-|-|-|
| `listen` | `0.0.0.0:8080` | the HTTP port (`--listen` overrides) |
| `path` | `/myth2-ws` | the WebSocket path (starts with `/`, no trailing `/`, or `/`) |
| `target` | `127.0.0.1:3453` | the default target: the local `mandrake-server` |
| `max_sessions` | 64 | |
| `[targets]` | none | `name = "host:port"`, reached at `<path>/<name>` or `?game=<name>` |
| `web_root` | none | the web client's files |
| `game_dir` | none | its `tags/` served under `/game/tags/` |
| `plugins_dir` | none | served under `/game/plugins/` |
| `downloads_dir` | none | our clients' plugin downloads, `/plugins/<checksum>` |
| `offered_plugins` | all | only these file names of `downloads_dir` |
| `max_downloads` | 4 (`proxy::DEFAULT_MAX_DOWNLOADS`) | plugin downloads at once, then a 503 |

## The web site (`mandrake_server::site`)

With `web_root`, `game_dir` or `plugins_dir` set, a GET or HEAD that isn't a WebSocket upgrade is
answered by the site (`site::answer_range`) on the same port:

| Path | Serves | `Cache-Control` |
|-|-|-|
| `/`, `/index.html` | `web_root/index.html` (with an ETag; a revisit is a 304) | `no-cache` |
| `/<file>`, `/pkg/<file>`, `/js/<file>` | `web_root`'s files | `no-cache` |
| `/game/list` | JSON list of every served game file: `path`, `size`, `version` (`site::game_list`) | `no-store` |
| `/game/tags/<name>` | `game_dir/tags/<name>` | `no-store` |
| `/game/plugins/<name>` | `plugins_dir/<name>` | `no-store` |
| `/plugins/<checksum>` | the plugin of `downloads_dir` whose file header checksum is that (8 hex digits) | `no-store` |

The web client downloads the game data through `/game/` once and keeps it in the browser's
storage ([web.md](web.md)). A file's version and ETag are its modification time and size. A file
with a newer pre-compressed copy beside it (`<name>.br`, `<name>.gz`) is sent as that copy with
its `Content-Encoding` to a browser that accepts it (brotli first). `/plugins/<checksum>` serves
only plugin files with a valid header (`site::plugin_checksum`, `site::offered_plugin`) and takes
`Range: bytes=from-[to]` (206, or 416 past the end) so a cut download resumes. Names are one path
segment each, percent-decoded, with no `..`, no hidden files and no folders: nothing outside the
configured folders is reachable. **Everything under the served folders is public**; only ever
serve data you are allowed to share (for example the freely distributed Myth II demo).

## The container (`docker/`)

One image, `myth2-server`, runs `mandrake-server` and `mandrake-ws-proxy` together and carries the web client.
No game data and no plugins are ever in the image: they are mounted read-only at run time.

| File | What |
|-|-|
| `docker/Dockerfile` | two stages: build (Rust, the wasm target, `wasm-bindgen-cli`, `binaryen`, `brotli`) and run (Debian slim) |
| `docker/Dockerfile.dockerignore` | the build context: only the files the build copies |
| `docker/build.sh` | builds `myth2-server:latest` (or `$IMAGE`) from a small tar of the tracked files |
| `docker/compose.yaml` | the service, its ports, mounts and hardening |
| `docker/entrypoint.sh` | the supervisor (installed as `/usr/local/bin/myth2-entrypoint`) |
| `docker/server.toml` | the image's default server config (`/etc/myth2/server.toml`) |
| `docker/proxy.toml` | the image's default proxy config: WebSocket, web client, plugin downloads |
| `docker/proxy-site.toml` | `proxy.toml` plus the mounted game data served under `/game/` |
| `docker/third-party-licenses.sh` | writes `THIRD-PARTY-LICENSES` for the image from `Cargo.lock` |

**Build stage**: `cargo build --release --locked -p mandrake-server --bin mandrake-server --bin mandrake-ws-proxy`;
then the web client: `cargo build --release --locked --target wasm32-unknown-unknown -p mandrake-web
--lib`, `wasm-bindgen --target web` (its version must equal the `wasm-bindgen` crate in
`Cargo.lock`, the `WASM_BINDGEN` build argument), `wasm-opt -O2`, and a gzip and a brotli copy of
the wasm and its glue; the page and loader script go in as `index.html` and `client.js`, the
crate's `js/` files under `js/`. Build arguments `REVISION` and `REVISION_DATE` give the landing
page its version (the context has no git).

**Run stage**: user and group 1000 (`myth2`); binaries in `/usr/local/bin`; the web client in
`/usr/share/myth2/web`; default configs in `/etc/myth2`; licences in
`/usr/share/doc/myth2-server`. Exposes 3453/tcp and 8080/tcp.

**Entrypoint** (`entrypoint.sh`): uses `/config/server.toml` and `/config/proxy.toml` if present,
else the defaults in `/etc/myth2`; with `M2_SITE_DATA=1` and no `/config/proxy.toml` it takes
`proxy-site.toml` (the site serves the data mounts). It exits with code 2 if `/data/myth2/tags`
is missing or `/films` isn't writable. It starts both processes (with `M2_SERVER_ARGS` appended to
`mandrake-server`'s arguments), copies each one's output to stdout and, when `/logs` is writable, to
`/logs/server.log` and `/logs/proxy.log`; when either exits it stops the other and exits with its
code, so the restart policy brings the whole container back. SIGTERM/SIGINT stop both.

**Compose** (`compose.yaml`; settings from the environment or `docker/.env`):

| Setting | Default | Meaning |
|-|-|-|
| `MYTH2_DATA` | `./data` | the Myth II folder, mounted at `/data/myth2:ro` (needs `tags/`) |
| `MYTH2_PLUGINS` | `./plugins` | mounted at `/data/plugins:ro` (may be empty) |
| `M2_CONFIG` | `./config` | mounted at `/config:ro` (optional `server.toml`, `proxy.toml`) |
| `M2_FILMS` | `./films` | mounted at `/films` (writable by `M2_UID`) |
| `M2_LOGS` | `./logs` | mounted at `/logs` |
| `M2_BIND`, `M2_PORT` | `0.0.0.0`, 3453 | the game port for native clients |
| `M2_WEB_BIND`, `M2_WEB_PORT` | `127.0.0.1`, 8080 | the HTTP/WebSocket port, for a TLS front on the same machine |
| `M2_UID`, `M2_GID` | 1000 | the container's user |
| `M2_SITE_DATA` | 0 | 1: serve the data and plugins mounts to browsers |
| `M2_SERVER_ARGS` | empty | extra `mandrake-server` arguments, e.g. `--games 1 --lobby-timeout 600` |
| `RUST_LOG` | `info` | log filter |
| `M2_REVISION`, `M2_REVISION_DATE` | `unknown`, empty | the build arguments for the landing page's version |

The service runs with a read-only root, a tmpfs `/tmp` (where `work_dir` lives), all capabilities
dropped, `no-new-privileges`, and `restart: unless-stopped`.

### Build and run

```sh
# Build (from the repository root)
docker/build.sh                                   # small context; or:
docker compose -f docker/compose.yaml build

# Volumes, owned by uid 1000
cd docker && mkdir -p config films logs

# Run with your own Myth II 1.8.5 folder
MYTH2_DATA="/path/to/Myth II" MYTH2_PLUGINS="/path/to/Myth II/plugins" \
  docker compose -f docker/compose.yaml up -d

# Check
curl -s http://127.0.0.1:8080/healthz             # {"status":"ok",...}
docker compose -f docker/compose.yaml logs --tail 20
```

Native clients (ours and the original game) join on 3453/tcp, the only port to forward. Browsers
need a secure context (https, or localhost on the same machine), so put a TLS front (a reverse
proxy or tunnel) before port 8080 that passes `/`, `/game/` and the WebSocket path
(`/myth2-ws`) through; the web client's Multiplayer then joins over `wss://<host>/myth2-ws`. With
`M2_SITE_DATA=1` everything under the two data mounts is public.

### Local run without Docker

```sh
cargo build --release -p mandrake-server -p mandrake-client
mkdir -p /tmp/m2s
cat > /tmp/m2s/server.toml <<'EOF'
data_dir = "/path/to/Myth II"
films_dir = "films"
[game]
map = "85gi"
time_limit_minutes = 2
EOF
target/release/mandrake-server /tmp/m2s/server.toml --games 1      # hosts on 0.0.0.0:3453
target/release/mandrake --net-join 127.0.0.1:3453            # our client joins and owns it
```

As the first of our clients the joiner owns the game; in its lobby it can press Start, and a
`--net-join` run from the command line sends the Start itself once its `--net-players` are in and
ready (see [client.md](client.md) for the exact flags).

For the browser path, also run `target/release/mandrake-ws-proxy proxy.toml` with `target` pointing at
the server, or `target/release/mandrake-ws-tunnel 127.0.0.1:4000 ws://127.0.0.1:8080/myth2-ws` and join
the tunnel with `mandrake --net-join 127.0.0.1:4000` to exercise the proxy without a browser.
At the end the server logs `game 1 ended at tick ..., 0 out of sync, film ...`; a non-zero count
or exit code means the server's sim disagreed with the host's SYNCs.

## Verifying changes

Self-contained tests (no game data, loopback sockets and temporary folders only):

```sh
cargo test -p mandrake-server --lib     # config parsing and errors, autohost rule and countdown,
                                  # owner (first Hello owns, empty password, max players,
                                  # plugin check by checksum, Boot), ws framing and handshake,
                                  # site (what is served, pre-compressed copies, plugin ranges),
                                  # proxy (head timeout, config-only targets, greeting rewrite,
                                  # ping answering, buffer cap, site and WebSocket on one port)
cargo test -p mandrake-server --test ws_join   # an mandrake_net Joiner through the proxy over a WebSocket to
                                         # a loopback host: join, Ready, start barrier, a command
                                         # stamped back, and the host's leaving closes it
cargo clippy -p mandrake-server --all-targets -- -D warnings
```

A whole game through the server needs a map, so it is checked by hand with your own game data:
the local run above, with one or more of our clients (and, if you have it, a copy of the original
game) joined; the game must end with `0 out of sync` and the server's film must replay to the same
end as each player's. For the container, `docker compose up` and the `healthz` and `/game/list`
checks above. Protocol changes are verified in `mandrake-net` ([networking.md](networking.md)).
