# The web client (mandrake-web)

The browser runs **the same game as the native client**. `mandrake-client` is a library (`mandrake_client::run`),
and `mandrake-web` is a thin `cdylib` that exposes it to JavaScript through `wasm-bindgen`. The page
loads the game files into an in-memory file system, then calls `start_game` with a command line, and
from there the code path is the native one: `Args` is parsed by clap, then `mandrake_client::run`, then
the winit `Shell` and `Myth2Scene`, with wgpu on WebGPU.

## Build

```bash
cargo build --release --target wasm32-unknown-unknown -p mandrake-web --lib
wasm-bindgen --target web --out-dir OUT/pkg target/wasm32-unknown-unknown/release/mandrake_web.wasm
cp crates/mandrake-web/js/*.js OUT/js/
cp scripts/web/client.html OUT/index.html && cp scripts/web/client.js OUT/
```

- `wasm-bindgen-cli` must be **exactly** the version of the `wasm-bindgen` crate in `Cargo.lock`.
  The Nix shell pins the matching one, and `docker/Dockerfile` pins it with `ARG WASM_BINDGEN`.
- `wasm-opt -O2` from binaryen is optional and shrinks the module by about a quarter. The Docker
  build runs it with an explicit feature list, and it also writes `.br` and `.gz` copies that the
  site serves when a browser accepts them.
- `crates/mandrake-web/build.rs` stamps `M2_WEB_REVISION` and `M2_WEB_REVISION_DATE` from git, or from the
  `M2_REVISION`/`M2_REVISION_DATE` environment variables, which the Docker build passes in.
  `client_version()` returns them, and the landing page shows them in its footer.
- `cargo build -p mandrake-web` (native) also builds. It runs the shared film-replay code and the
  `examples/film_bench.rs` benchmark: `cargo run --release -p mandrake-web --example film_bench --
  FILM.m2rec [GAME_DIR]`.

## Rust side

`crates/mandrake-web/src/`:

- `client.rs` (wasm32 only) is the real game's exports:
  - `add_game_file(path, bytes)` puts a file into the `mandrake_common::MemoryVfs` that is registered as
    the game's file system (`mandrake_common::set_game_vfs`). Every read of game data in the client goes
    through `crates/mandrake-client/src/game_files.rs`, which uses `mandrake_common::game_vfs()`, so native code
    and web code read files the same way.
  - `film_plugin_names(film)` and `engine_tag_set(tags, plugin_names, plugin_heads, film_plugins,
    metaserver_preference)` let the page work out which tag and plugin files a run needs before
    downloading them. This is the same search order as `mandrake_tag::tag_files`.
  - `start_game(words)` calls `mandrake_client::web::start`, which installs a panic hook and parses the
    words as the command line, then calls `crate::run`.
  - `set_audio_rate(rate)` and `mix_audio(out) -> bool` are the audio pump (see below).
  - `net_player_name()`, `net_player_colors()` and `set_net_player(name, colors)` read and write the
    `[net]` table of `settings.toml`, so the landing page and the game's player dialog share them.
  - `client_version()`.
- `film_replay.rs` and `film_bench.rs` hold a headless film replay (`film_replay::replay`,
  `film_plugins`, `order_tag_files`) shared by the native example and a wasm `FilmBench`. Use them
  to compare sim speed between native code and wasm.
- `lib.rs` has `init_web_client()` and `WebClient`. This is an older demo API (`WebEntity`
  stand-ins, its own `SharedAudio`) that **the game doesn't use**. Its method names
  `set_audio_sample_rate` and `mix_audio` are what `mandrake-web-audio.js` calls, and the page bridges
  them to the free functions with a small shim object.

`crates/mandrake-client/src/web.rs` is the client's browser module (`#[cfg(target_arch = "wasm32")]`):
`start`, `console` (the client's `println!`/`eprintln!` are redefined in `lib.rs` to log to the
console), `net_player`, `set_net_player`, `Connecting` (a WebSocket join), and the return to the
landing page when the game quits. Other `cfg(target_arch = "wasm32")` branches in the client:

- `clock.rs` uses `web-time`'s `Instant`, because `std::time` panics on wasm32.
- `config_store.rs` stores each settings file as a `localStorage` entry named by its path under
  `user_dirs::WEB_CONFIG_BASE` (`/config`). Settings therefore persist per browser.
- `app/shell.rs`: the winit window draws into the page's `<canvas id="myth2">`, and the app is
  started with `spawn_app`. The GPU arrives asynchronously, which is why `GameScene::on_gpu` takes
  `late`.
- `window_surface.rs` chooses `wgpu::Backends::BROWSER_WEBGPU`. The world is drawn into an sRGB
  view even where the canvas format is not sRGB (`SurfaceFormats`).
- `audio_out.rs` has no cpal. A thread-local mixer is fed by the page.

## JavaScript glue (`crates/mandrake-web/js/`)

| File | What it does |
|---|---|
| `mandrake-web-audio.js` | `M2WebAudio.attach(client, {workletUrl})`: creates an `AudioContext`, gives its sample rate to the game, then pumps `mix_audio` every 10 ms in 128-frame blocks and keeps about 2048 frames queued in the worklet. It falls back to a `ScriptProcessorNode`. The context is resumed on the first pointer or key event. |
| `mandrake-audio-worklet.js` | `"m2-mixer-output"` worklet: plays the posted blocks and reports what it consumed. All mixing happens in wasm on the main thread. |
| `mandrake-net-ws.js` | `installNet()` sets `globalThis.m2Net` (`open`, `state`, `take`, `send`, `close`, `url`, `why`), a WebSocket to the page's own host at `/myth2-ws` (`wss:` on https). |

The page itself is `scripts/web/client.html` and `scripts/web/client.js`. It is the landing page:
a display preset, the player's name and colours, then Play. The page then:

1. Finds the data. If the site serves data, it uses `GET /game/list` and fetches the needed files,
   caching them in the browser's OPFS by version, so a second visit downloads nothing. Otherwise
   (or with `?own`) it uses the player's own Myth II folder from a directory picker, copied into
   OPFS.
2. Calls `add_game_file('/game/' + path, bytes)` for each file.
3. Starts audio, then calls `start_game(['--archive', '/game/tags/small install', ...preset])`.

The presets are plain client options:

- **Original**: `--ui-filter nearest --sampling linear`.
- **Retro-modern**: `--ui-filter lanczos2 --sprite-filter sharp --sampling software --lighting on
  --fog on --ui-scale 200`, plus an interface file that hides the control bar.

Both add `--water enhanced --water-maps 85tl`. The query string can override the defaults:
`?play=MESH`, `?film=NAME`, `?menu`, `?args=...` (extra command-line words), `?mute`, `?own`.

## Networking in the browser

A browser can't open TCP sockets. Instead:

- `mandrake_client::web::Connecting::start` wraps `m2Net` as an `mandrake_net::transport::ByteStream`, polled
  once a frame. It then hands it to `mandrake_net::endpoint::Endpoint::join_over`, so from there the
  protocol code is the native code.
- The default address is the page's own host. Browsers join the dedicated server that serves the
  page and do not host games.
- `mandrake-ws-proxy` (in `mandrake-server`, see [server.md](server.md)) turns each WebSocket into a TCP
  connection to `mandrake-server`. It answers keep-alive pings for the browser, so a background tab is
  not dropped, and caps each direction's buffer.

## Serving

The page needs a **secure context** (https, or `localhost`) for WebGPU and OPFS. `mandrake-ws-proxy`
serves `web_root` (the built page) on port 8080. With the site data option it also serves
`/game/list`, `/game/tags/*` and `/game/plugins/*` from the mounted game folder. The Docker image
does all of this, so see the [README](../README.md#run-the-dedicated-server-docker). Terminate TLS
in front of the container.

## Limits compared to native

- **No threads.** Cutscenes are skipped, because their decoder runs on a thread. The AV1 HD
  cutscene path isn't compiled. A network join polls instead of blocking.
- **Audio is mixed on the main thread** from a 10 ms timer, so a throttled background tab can
  stutter.
- **Native-only features are off:** plugin downloads (`settings::download_settings` disables them),
  the river-bed disk cache of the enhanced water, screenshots, and the HD cutscenes.
- **Browser support:** a Chromium-family browser with WebGPU on a computer. The page checks for
  WebGPU, OPFS and a secure context first, and explains what's missing.
- **Memory:** wasm32 has a 4 GB address space.
