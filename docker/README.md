# A demo host: the web client and its server in one container

The steps for a machine that hosts the browser demo: one
container, `myth2-server`, runs the dedicated server and serves the web client with the **Myth II
demo's** data. Players open the site over **https** (a TLS front: a reverse proxy or a tunnel), pick a display
preset, a name and colours, Play, and the page downloads the demo once and keeps it in the browser.
Everything else (ports, mounts, config keys, TLS, licences) is in the dedicated server's
documentation.

**Only ever mount the demo.** With `M2_SITE_DATA=1`, everything under the two data mounts is public.
Never point them at a full install.

## 1. The demo's data

Extract the Myth II demo so that its folder has `tags/` (with `small install`) and `plugins/`, for
example `~/myth2-demo/Myth II Demo`.

Plugins: copy a plugin into the plugins mount's folder, e.g.
`cp MyPlugin "$HOME/myth2-demo/Myth II Demo/plugins/"`. The site lists it with the rest, so it is
public too; the page loads it at Play, and its campaigns and maps show in New Game. No rebuild or
restart is needed for a plugin alone: `/game/list` reads the folder at each request. A plugin
built for the full game may use tags the demo lacks; it draws on the demo's data only if it
carries those tags itself.

## 2. The settings

`docker/.env` (git-ignored; `KEY=value` lines):

```bash
MYTH2_DATA=/path/to/myth2-demo/Myth II Demo
MYTH2_PLUGINS=/path/to/myth2-demo/Myth II Demo/plugins
M2_SITE_DATA=1                # serve the demo's tags and plugins to browsers
M2_WEB_BIND=127.0.0.1         # the TLS front runs on this host; 0.0.0.0 to reach it from the LAN
M2_WEB_PORT=8080              # the page's port on the host
```

Then the volume folders, owned by uid 1000 (`M2_UID` to change it):

```bash
cd docker && mkdir -p config films logs
```

## 3. Build and start

On the host itself (the image is built for the machine that builds it, e.g. arm64 on an arm64 host), from the
repo root, with the commit named so that the landing page shows it:

```bash
git pull --ff-only
export M2_REVISION=$(git rev-parse --short HEAD) M2_REVISION_DATE=$(git log -1 --format=%cs)
docker compose -f docker/compose.yaml build      # or docker/build.sh (x86_64, a smaller context)
docker compose -f docker/compose.yaml up -d
```

The first build takes a while (the Rust toolchain's wasm target and wasm-bindgen-cli are cached after).

## 4. Check it

```bash
curl -s http://127.0.0.1:8080/healthz                         # ok
curl -s http://127.0.0.1:8080/game/list | head -c 300; echo   # the demo's tags and plugins only
docker compose -f docker/compose.yaml logs --tail 20          # M2_SITE_DATA=1: ... served to browsers
```

On the host, `http://127.0.0.1:8080/` (or `http://localhost:8080/`) shows the landing page with
the version at its foot, e.g. `Version 0.1.0 (03bc8ee3, 2026-11-15)`.

## 5. https in front

The page needs a **secure context**: browsers give it its storage (OPFS) and WebGPU only over
https, or on `localhost`/`127.0.0.1` of the same machine. Opened over plain http from another
machine (`http://192.168.x.y:8080/`) it says "Open this page over https" and doesn't start.

Put a TLS front (a reverse proxy or a tunnel) before `http://localhost:8080` (the `M2_WEB_PORT`),
with no path, so the page, `/game/` and the WebSocket `/myth2-ws` all reach the container; it must
pass WebSockets through. Then
`https://<hostname>/` plays, and the main menu's Multiplayer joins this container's game over
`wss://<hostname>/myth2-ws`.

Native clients (ours, and the original engine's) join on 3453/tcp, the only port to forward on the router; they must
run on the demo's data too.

## 6. Update or stop

```bash
git pull --ff-only && export M2_REVISION=... M2_REVISION_DATE=...   # as in 3
docker compose -f docker/compose.yaml up -d --build
docker compose -f docker/compose.yaml down
```

Browsers fetch only the files whose version changed. The page itself is revalidated on each
load, and it names the client's script by the code's hash (`client.js?v=...`, stamped at the image
build by `scripts/web/stamp.sh`), so a new image's client loads at once even through a front that
keeps scripts in the browser (Cloudflare gives `.js` files `max-age=14400` whatever the server
says).

## What the browser needs

A recent Chrome, Edge or Chromium on a computer (WebGPU). A browser without WebGPU gets "This
browser has no WebGPU" on the landing page instead of a broken start.

## A TFL instance beside it

The container can also host **Myth: The Fallen Lords** games for our native TFL clients
(`mandrake --game tfl`): a second `mandrake-server`, the TFL instance, runs beside the Myth II one
when TFL's folder (the one holding `tags/*.gor` and `tags/Local`, TFL 1.5's) is mounted:

```bash
TFL_DATA=/path/to/Myth_TFL      # in docker/.env; mounted read-only at /data/tfl
M2_TFL_PORT=3454                # the host's port for it (forward it too)
```

Its config is the image's `/etc/myth2/tfl-server.toml` (`docker/tfl-server.toml`: port 3454, For
Carnage Apply Within, Body Count, 10 minutes), or `config/tfl-server.toml` in the config volume.
Its films go to `films/tfl/`, its log to `logs/tfl-server.log`. Without `TFL_DATA` (an empty
`./tfl`) the container runs as before. Native players join with `mandrake --game tfl --net-join
HOST:3454`; a Myth II client that joins it is refused ("This is a TFL game: start the client with
--game tfl."), as a TFL client is refused the Myth II instance.

**TFL in the browser.** With the TFL instance running, the proxy's WebSocket also
reaches it at `/myth2-ws/tfl`. To let browsers play TFL, opt in to serving TFL's folder:

```bash
M2_SITE_TFL=1                   # in docker/.env: /data/tfl/tags is served under /game/tfl/
```

Then `https://<hostname>/?game=tfl` is TFL's landing page: Play downloads TFL's tags once
(about 460 MB with `CUTSCENE.GOR`; `tags/Local/RECO`, the install's films, is never served) and
opens TFL's main menu, whose Multiplayer Game joins this container's TFL instance at once.
As with `M2_SITE_DATA`, whatever is mounted there becomes public. A `/config/proxy.toml` of your
own replaces all of this: add `tfl_dir = "/data/tfl"` and `[targets] tfl = "127.0.0.1:3454"` to it.
