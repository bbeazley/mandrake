# Working on this code: a guide for agents

Start with [docs/SPEC.md](docs/SPEC.md). It has the crate map, the dependency graph, the tick and
frame model, a glossary, and links to one document per area. Read the document for the area you
are changing before you change it. The code has no comments, so these documents are where the
reasons are written down.

## Build and test

Use the Nix dev shell from `flake.nix` (`nix develop`), or install what it lists. It provides
stable Rust with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` pinned to the crate
version, binaryen, nasm, and the Vulkan, Wayland, X11 and ALSA libraries. `MYTH2_DATA_DIR` names
the Myth II 1.8.5 folder you supply. The game data is not part of this repository.

```bash
cargo build --workspace
cargo build -p mandrake-web --target wasm32-unknown-unknown --lib     # the web client must keep building
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings           # zero warnings
cargo test --workspace                                          # self-contained; needs no game data
cargo test -p mandrake-sim                                            # while working: just your crate
```

Rules for builds and tests:

- The tests in this tree need no game data, so they pass with `MYTH2_DATA_DIR` unset.
- When you add a test, keep it self-contained. If it needs input, use a small synthetic fixture
  inside the crate.
- `mandrake-sim`'s integration tests are one binary, `crates/mandrake-sim/tests/all/main.rs`. A new file in
  `tests/all/` runs only if `main.rs` lists it with a `mod` line.
- Run the client with your data (see [README.md](README.md)), for example:
  `cargo run --release -p mandrake-client -- --archive "$MYTH2_DATA_DIR/tags/small install"`.

## Determinism: what must not change

The simulation must make the same decisions as the original 1.8.5 engine, in the same order, with
the same random draws. Films recorded by the original game must replay to their end, and network
games with original peers must not go out of sync. Any change to `mandrake-sim`, `mandrake-tfl-sim`, `mandrake-rec`,
`mandrake-math` or the parts of `mandrake-tag` that the sim reads (unit, projectile, object and mesh
definitions) needs a film check.

Rules for simulation code:

- **No floating point in the sim.** Use `mandrake_math::Fixed16`, `Angle512` and integer world units
  (512 per cell). Float maths is used only for drawing and sound.
- **No clock, no frame rate, no hash-map iteration order and no thread timing in the sim.** Its
  only inputs are the commands stamped for each tick.
- **Every random draw goes through the sim's `BungieRng`, and its call sites and order matter.**
  Adding, removing or reordering a draw moves every later decision.
- **Interpolation, camera, particles, sound and other visual effects are client-side.** They must
  not draw from the sim's generator or write sim state. The client has its own local random states
  for its effects.
- **The client's input path must keep stamping commands as it does now.** An order given at tick T
  is stamped for T+1. A change there changes what live games record.

How to check a sim change:

1. **Before the change**, replay one or more original 1.8.5 films headless and keep the draw log.
   Use films of your own from `recordings/`, as long as possible:
   ```bash
   cargo run --release -p mandrake-client -- --archive "$MYTH2_DATA_DIR/tags/small install" \
     --replay "FILM.m2rec" --headless --ticks 100000 --film-report before.draws
   ```
2. **After the change**, run the same command into `after.draws`. Then
   `diff before.draws after.draws` (one `tick site seed` line per random draw). Also diff the two
   `.end` files, which are written once the replay reaches the film's end. If you meant to fix a
   divergence from the original, the first difference must be where you expected it.
3. **Watch the log** (`RUST_LOG=info`). A SYNC mismatch logs `OUT OF SYNC ... game time: T` and is
   recorded in `Simulation::out_of_sync`. A film that replayed to its end without one must still do
   so, and must not stop earlier than before.
4. **Network or film-format changes**: also replay a network film recorded by the original game.

A pure client change (drawing, sound, interface) cannot move a replay. If it seems to, the change
has reached into the sim.

## The art rule

Every visual element comes from the game's own tags: `.256` collections, `mesh`, `geom`/`mode`,
`scen`, `unit`, and the interface tags. Do not add procedural sprites, generated textures or
untextured placeholders. Filters, upscaling, fog, lighting and frame-rate work are allowed, as long
as what they process is the game's art.

There are three named exceptions:

- the **height fog's noise**, computed in the shader (`fog.wgsl`, `--fog on`);
- the **enhanced water's generated waves** (`--water enhanced`): the FFT wind-wave cascades, the
  gusts, the shore swash and the glint points;
- a fixed **dither** of about half an 8-bit step on the water's output, against colour banding.

Anything else generated needs the same explicit decision.

The original look must stay reachable. New looks are options with an `off` setting, or a
`--sampling linear` path that keeps the original OpenGL picture.

## Performance

The project's goal is 240 fps, which is **4 ms for a whole frame**. Give new per-frame work a share
of that budget and measure it with `mandrake --replay FILM --frame-budget 10` (see
`docs/rendering.md`).

## Boundaries worth keeping

- **No platform code in the sims.** `mandrake-sim` and `mandrake-tfl-sim` have no window, GPU, audio or
  sockets.
- **The session sees only the network's data.** `mandrake-session` uses `mandrake-net`'s data types, never its
  sockets.
- **`mandrake-view` and `mandrake-dialogs` have no wgpu.**
- **Shared code comes before copies.** The Fallen Lords reuses Myth II's shell, dialogs, input,
  settings and renderers. Extend the shared code rather than copying it into `tfl_*` files.
- **Settings live in TOML files.** There is no Preferences screen. A new option gets a command-line
  flag and, where it should persist, a key in the matching `*.default.toml`. Document the key in
  `docs/configuration.md`.
- **The web client must keep building.** Code that needs threads, the file system or native
  libraries goes behind `cfg(not(target_arch = "wasm32"))`, with a browser path or a graceful
  absence.

## Licence

The code is MIT OR Apache-2.0. Dependencies must be permissively licensed, and `deny.toml` is
checked with `cargo deny check`. Do not add game data, or anything derived from it, to the
repository.
