# Rendering (mandrake-view, mandrake-render, and the client's frame)

The client draws the world with wgpu: Vulkan, Metal or DX12 natively, and WebGPU in a browser.
The default look reproduces the original 1.8.5 engine's **software renderer**. `--sampling linear`
gives its **OpenGL renderer** instead. Every picture comes from the game's own tags: `.256`
collections, `mesh`, `geom`/`mode`, `scen`, `unit`. Generated art is not allowed, with three named
exceptions: the height fog's shader noise, the enhanced water's generated waves and a fixed
dither on the water's output against colour banding.

**Frame budget (a project goal):** 240 fps, which is **4 ms for a whole frame** (simulation, CPU
build, GPU, present). Give new rendering work its frame time as a share of that, and measure it with
`--frame-budget` (below).

## Crates

### mandrake-view: the GPU-free half

`crates/mandrake-view` depends only on `mandrake-common` and `mandrake-tag`, and has no wgpu. It holds the
geometry, camera and draw-order rules as plain data, so they can be tested without a GPU.
`mandrake-render` re-exports each module under the same name, so `mandrake_render::camera::Camera` is
`mandrake_view::camera::Camera`.

| Module | Contents |
|---|---|
| `camera` | `Mat4`, `Camera` (`view_proj_matrix`, `screen_ray`, `interpolate`, `engine_view`), `CameraState`, `ObserverController` and `ObserverPhysics` (the original game's observer motion: move, turn, orbit, zoom presets), `myth_observer` (its constants, `ZOOM_PRESETS`, `ObserverAxes`) |
| `landscape` | `LandscapeTexels`: the landscape colour map shaded on the CPU, with no lights or normals (texel = shading table[shade, colour index]). Also stains (blood, footprints, scorch: `apply_stain`), shadow overlays, and `rebuild_regions`/`PixelRect` for partial re-uploads |
| `billboard` | `BillboardInstance` (one sprite quad), `select_sprite_view` (choosing the sprite of the 8 or more directions), atlas packing (`atlas_layout`, `pack_shelves`), and the original draw order (`order_for_sprite_pass`, `category4_steps`, `sprite_category_steps`, `place_sprites_in_engine_view`) |
| `selection` | Selection frames, health and mana bars, and the band rectangle, in screen pixels (`selection_frame_rect`, `vitality_bar`, `selection::state`, `selection::band`) |
| `view_region` | Which cells and objects are in view this frame (`ViewRegion`, `build_view_region`, `walk_region`) |
| `media` | Water ("media") behaviour: surface wobble (`MediaWobble`), clipping sprites at the surface, reflection tints |
| `model_surfaces` | Rules for 3D-model surfaces: category-2 polygons, reflections (`reflection_triangles`), `WorldFrame` conversion |
| `model` | `ModelVertex`, `TranslucentDrawList` |
| `projectile` | Projectile fade and bobbing, and `VelocityQuad` (arrows drawn along their velocity) |
| `connector` | Fence and rope strips between scenery objects |
| `interface_bitmap` | Software 2D drawing into dialog bitmaps (`InterfaceArt`, `draw_bitmap_from_sequence`) |
| `interface_picture` | `InterfacePicture`, `MenuCanvas`, `InterfaceLayer`, `InterfaceFilter` (`Sharp`, `Nearest`, `Linear`, `Lanczos2`, `Fsr1`) |
| `canvas_fit` | `CanvasFit`: fits a menu canvas into the window |
| `rotation` | `port_model_matrix`: converts the original game's 3x3 fixed-point model matrix to a 4x4 |

"Category" numbers (2, 4, ...) are the original engine's render categories. Category 2 is
landscape-coplanar model polygons. Category 4 is reflections of sprites and models in water.

### mandrake-render: the GPU half

`crates/mandrake-render` (wgpu). The main types are re-exported at the crate root:

| Type | File | Role |
|---|---|---|
| `TerrainRenderer` | `pipeline.rs` | Landscape and occluder pipelines, scene uniforms, detail textures. `render_terrain_media`, `render_terrain_dry`, `render_terrain_occluders`, `update_uniforms` |
| `TerrainMesh` | `mesh.rs` | Landscape mesh from the `mesh` tag (`from_mesh_tag`), LOD blocks, water vertices (`update_media`). `mesh::ripple` rebuilds LOD blocks when cell heights change |
| `ModelRenderer`, `ModelMesh` | `model.rs` | 3D models from `geom`/`mode`: `render`, `render_depth_only`, `render_category2`, `render_reflection_triangles`, `render_sprite_triangles`, plus translucent batches (`TranslucentBatch`) |
| `BillboardRenderer`, `TextureAtlas` | `billboard.rs` | Instanced sprite quads from one atlas: `upload_instances`, `render_range`, `render_reflections`, `render_interface_range` |
| `OverlayRenderer` | `overlay.rs` | HUD and dialog bitmaps (`OverlayImage`, `OverlayLayer`) |
| `InterfaceScaler` | `interface_scale.rs` | Draws the interface into a logical-size layer and scales it to the window (`composite`, `composite_content`, `composite_into`). Holds the FSR1 and Lanczos-2 passes |
| `InterfacePresenter`, `MenuCanvasGpu` | `interface_present.rs` | Draws an `InterfacePicture` from the dialogs |
| `DrawInterface`, `InterfaceFrame` | `interface_frame.rs` | The trait a front end's GPU half implements (see [interface.md](interface.md)) |
| `ScreenFade` | `screen_fade.rs` | The fade to and from black |
| `ReflectionEdge` | `reflection_edge.rs` | Mask for `--reflection-soft-edge` |
| `SceneUniforms` | `uniforms.rs` | The shared uniform block, group 0 binding 0 in every shader |
| `mesh_pick::pick_ground` | `mesh_pick.rs` | The ground point under the cursor, picked as the original does over its sorted triangles |
| `detail::LandscapeDetail` | `detail.rs` | A plugin's detail textures (`dmap`/`dtex`) |
| `water::WaterShading` | `water/` | Enhanced water (below) |

`SceneUniforms` holds:

- `view_proj` and `camera_pos`;
- `render_params`: `[sampling mode, sprite filter mode, viewport w,
  viewport h]`;
- `fog`, `fog_height` and `fog_motion` (the height fog);
- `ambient_tint`, `light_params` and `lights` (prototype point lights; at most 8 are used).

It is built with `SceneUniforms::from_camera(..)` followed by the `with_*` builders.

### Shaders (`crates/mandrake-render/shaders/`)

The shaders are WGSL. Most of them are concatenated with `fog.wgsl` through
`concat!(include_str!(..))`.

| Shader | Entry points | What it draws |
|---|---|---|
| `terrain.wgsl` (+ `fog.wgsl`) | `vs_main`/`fs_main`, `vs_detail`/`fs_detail` | Landscape. `software_landscape` reproduces the software renderer's two-pixel filtered spans. Bicubic and sharp modes apply only while the texture is magnified |
| `occluder.wgsl` | `vs_main`, `vs_tfl_bucket` | Depth only: each landscape triangle at its farthest corner's depth, less a fixed bias, so sprites are hidden by terrain the way the original's are |
| `model.wgsl` (+ fog) | `vs_main`, `vs_reflection`, `fs_main`, `fs_media_display` | 3D models and their reflections |
| `billboard.wgsl` (+ fog) | `vs_main`, `vs_interface`, `fs_main`, `fs_interface`, `fs_reflection_soft` | Sprites, the interface's rectangles, reflections |
| `fog.wgsl` | `height_fog_factor`, `fog_patch` | The original's depth fog, and the height fog with drifting value-noise patches (a named exception) |
| `overlay.wgsl` | | HUD and dialog bitmaps |
| `interface_scale.wgsl` | | Sharp, nearest, linear and Lanczos-2 scaling of the interface layer |
| `fsr1.wgsl` | `fs_easu`, `fs_rcas` | AMD FidelityFX Super Resolution 1 (MIT), for cutscenes |
| `reflection_edge.wgsl` | | Dry-shore mask for soft reflection edges |
| `water.wgsl`, `water_waves.wgsl`, `water_bed.wgsl`, `water_surface.wgsl`, `water_billboard.wgsl`, `water_ripples.wgsl` | | Enhanced water |

The world is drawn into an **sRGB view** of the surface texture. The terrain shaders convert the
texels with `srgb_to_linear` first, so that the bytes stored match the original's palette values.
Sprites, reflections and the interface are blended into a second, **non-sRGB view** of the same
texture (`window_surface::SurfaceFormats`: `scene` and `interface`). In the code that second view
is named "interface", but world sprites draw into it too.

## The client's frame

`ClientApp` (`crates/mandrake-client/src/app/mod.rs`) is one loaded map. Each frame,
`ClientApp::render_frame` (`app/render.rs`) runs these steps:

1. **Simulation and input**: `update_physics_and_simulation` (`app/tick.rs`). See
   [client.md](client.md#ticks-and-frames) for the tick loop and the interpolation fraction.
2. **Acquire** the surface texture.
3. **View**: `region_view::frame_view_parts` finds the cells in view from the interpolated render
   camera. Local projectile surfaces, lightning and the scene lights are collected next
   (`lighting::collect_scene_lights`, only with `--lighting on`).
4. **Uniforms**: `render_passes::world_uniforms` builds `SceneUniforms` from the camera, sampling,
   fog (`fog::MapFog::apply`) and lighting. Then `TerrainRenderer::update_uniforms` runs.
5. **Water and landscape changes**: `TerrainMesh::update_media` (the water wobble, once per local
   tick), then `WaterShading::update` when the enhanced water is on.
6. **Sprites**: the static scenery billboards (built once at load), the dynamic scenery, units,
   projectiles and effects. `world_view::myth2::drawn_world` builds a `WorldView` from the sim
   through the interpolated object history. `world_assembly::myth2_sprites` then turns it into
   `WorldSprites { billboards, order, shadows }`, in the original's draw order.
7. **Uploads**: the changed landscape rectangles (stains, footprints) go up through one staging
   buffer. Then the billboard instances, category-2 and reflection batches, and translucent models.
8. **World passes**: `render_passes::encode_world` (below).
9. **Interface**: the HUD, the overhead map and the overlays, then the open dialogs
   (`DrawInterface::draw`), then `ScreenFade`. After that come the screenshot readback if one was
   asked for, `queue.submit` and `present`.

### World passes (`render_passes::encode_world`)

1. **Scene pass** (clears colour and depth): category-2 model polygons, then the landscape under
   the water (`render_terrain_media`, or the enhanced water's `render_bed`).
2. **Category 4**: reflections of sprites and models in water, merged in the original order
   (`category4_steps`) with no depth test. With `--reflection-soft-edge`, the `ReflectionEdge` mask
   is drawn first. With enhanced water, the reflections go into the water's reflection layer and
   the submerged copies into a second layer.
3. **Enhanced water only**: the surface seen from below (`render_under`).
4. **Opaque**: the water's dry, wet and surface-depth draws, the opaque models, and the dry
   landscape (`render_terrain_dry`).
5. **Sprite occluder pass** (clears depth): model depth (`render_depth_only`) and the landscape
   occluders.
6. **Sprite display pass**: billboard runs (split by atlas and team-colour cache), translucent
   model triangles and interface rectangles, in `SpriteStep` order.

`PassRule` selects the Myth II or The Fallen Lords variant of these passes.

### Setup per window and per map (`app/gpu.rs`)

`ClientApp::attach_gpu` creates the renderers **once per window**. Each map then shares their
pipelines through `sharing_pipelines`. At map load the client builds:

- the terrain mesh (`map_load::build_terrain_mesh`);
- the sprite atlas (`sprite_manager::ClientSpriteManager::build`, which decodes every sprite the
  map needs);
- the static scenery billboards;
- the shaded landscape texels with static shadows;
- the enhanced water, and its river bed if that is on.

`window_surface::create_window_surface` tries the backends in order. `handle_surface_error`
reconfigures the surface when it is lost or outdated.

## Sampling and filters

- `--sampling software|linear` (`ui_scale::Sampling`, default `software`). `software` uses mip
  level 0 only and draws sprites and models unfiltered, with the landscape's two-pixel spans.
  `linear` is the original OpenGL renderer: `GL_LINEAR` filtering and mip chains. Some defaults
  follow this flag (`args::fog_mode`, `args::reflection_soft_edge_enabled`): `linear` means the
  original depth fog and hard reflection edges.
  In `mandrake_render`, the type `TextureSampling` has a `#[default]` of `Linear`. The client always sets
  it explicitly.
- `--sprite-filter linear|sharp|nearest|lanczos2` changes only magnified sprites.
- `--ui-filter` (`UiFilter`) scales the interface. `--cutscene-filter` (`CutsceneFilter`) scales
  cutscenes, and FSR1 is available only there.
- **The world is always drawn at the window's full resolution.** There is no render scale and no
  off-screen world target. Only the interface and cutscenes are scaled.

### Interface scale

`ui_scale::UiScale { window, logical, factor, zoom }`. With `--ui-scale native` (the default), the
interface is drawn at the window's resolution, as in the original. A zoom level (1, 1.25, 1.5,
1.75, 2, 2.5, 3 or 4, or Ctrl +/-) draws the HUD and dialogs into a smaller logical layer that
`InterfaceScaler` scales up. The zoom is lowered while the window can't hold a 640x480 interface.
The level is saved as `ui_zoom` in `settings.toml`.

## Fog

`crates/mandrake-client/src/fog.rs` (`MapFog`) and `fog_config.rs` (settings, see
[configuration.md](configuration.md#height-fog-fogdefaulttoml)).

- `--fog engine` draws the original fog, a veil by depth: `exp(-density * depth)`.
- `--fog on` (the default with software sampling) draws a height fog. It falls off exponentially
  with height along the eye ray and is broken into drifting patches by two layers of value noise
  computed in the shader. This noise is one of the three named exceptions to the art rule. The patch
  clock runs on wall time in a window. A headless frame uses `--fog-time`, or the tick's time, so a
  given frame is reproducible.
- `--fog off` draws no fog.

## Lighting (prototype)

`lighting.rs`, enabled with `--lighting on`. The original maps are lit in their textures, so only
maps listed in the lighting file get an ambient tint and point lights from projectiles, fires and
static torches (`extract_static_torch_lights`). At most 8 lights are used per frame
(`MAX_ACTIVE_LIGHTS`).

## Enhanced water

This is `--water enhanced`, and `--water-maps` limits it to the listed meshes. It is off by default,
when the water is the original's: a textured, wobbling surface with reflections. The module is
`mandrake_render::water` (`WaterShading`), and the settings are in
[configuration.md](configuration.md#enhanced-water-waterdefaulttoml).

- **The bed.** The landscape under the water is drawn at its real height and seen through the
  surface. It is refracted (Snell's law), absorbed and in-scattered by the water's depth, and
  blurred with path length.
- **The river bed repaint** (`water::bed::repaint_water_paint`, `crates/mandrake-client/src/water_bed.rs`).
  The original maps paint their water into the colour map. At load, the painted water is replaced
  by patches of the map's own dry ground. Only the map's own colour indices are used, so no new art
  is involved. The result is cached on disk.
- **The surface.** It moves with the original wobble (scaled by `swell`), plus three things:
  - interactive ripples: a wave-equation height field driven by moving units and splashes
    (`water::ripples::RippleField`, `crates/mandrake-client/src/water_ripples_input.rs`);
  - wind waves: three FFT cascades of a Tessendorf spectrum driven by the game's wind
    (`water::fft::FftWaves`);
  - gust patches and fetch-limited shelter.

  The wind waves, gusts, shore swash and glint points are generated, and are the second named
  exception to the art rule.
- **Lighting the water.** Reflections are drawn into their own layer and distorted by the wave
  slope, with Fresnel. The light direction for the sun glint is fitted from the map's baked shading
  (`water::fit_light`), because the original data has no light direction.

## Measuring: `--frame-budget`

`app/frame_budget.rs`. Run it as `mandrake --replay FILM --frame-budget 10` (window only). It
turns vsync and the frame cap off, warms up for 2 s, measures, prints a table and quits:

- **CPU stages** (`Lap`): sim, acquire, billboards, landscape upload, prepare, encode, submit,
  present.
- **GPU spans** come from timestamp queries (`GpuTimer`), one per pass.
- **For each stage**: average, 99th percentile and maximum, plus the number of frames over
  `BUDGET_MS` (4.0).

Related options:

- `--frame-budget-tick T`: seek to tick T at `--game-speed` first.
- `--frame-budget-scan N`: print one line per N ticks, to find the heavy part of a film.
- `--frame-budget-wait`: wait for the GPU after each submit.
- `--frame-budget-overlay DIALOG`: measure with a dialog open.
- `--frame-budget-follow [UNIT]`: keep the camera on a unit, so runs can be compared.

When water is enhanced, some GPU labels hold more than their names say. For example, the
category-4 span includes `render_under`.

**Frame pacing** (`app/fps_cap.rs`): `--fps-cap auto|N|off`, else `fps_cap` in `settings.toml`, else
`auto`. Auto is the monitor's refresh rate less 3 %, or 240 when the rate is unknown. The cap paces
redraws only, and the 30 Hz simulation runs on the clock whatever the frame rate. `--vsync on`
uses `AutoVsync`. Otherwise the order is Immediate, then Mailbox, then Fifo.

## Headless rendering and screenshots

- **Headless frame.** `--headless --screenshot out.png [--ticks N] [--frame-fraction F]
  [--view-camera ...]` runs `headless_run::run_headless`. It ticks the sim with scripted input
  (`--mouse`, `--select-*`, `--hud-click`, ...) and renders one frame with
  `headless_render::render_headless_frame` into an off-screen `Rgba8UnormSrgb` target. The world
  passes are the same `encode_world` as the window's. This is the way to check a visual change
  without a display.
- **Screenshot key.** In a window, the screenshot key reads back the swapchain
  (`screenshot::Readback`). It writes a PNG plus a text file with the camera and the command line
  that reproduces the frame (`ShotInfo`) into the pictures folder's `Screenshots`.
