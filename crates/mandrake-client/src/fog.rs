use crate::args::{self, Args, Fog};
use crate::clock::Instant;
use crate::fog_config::{self, FogConfig, FogLayer, FogSettings};
use mandrake_render::SceneUniforms;
use mandrake_tag::{MeshFog, MeshTag};
use tracing::info;

pub const FALLBACK_FOG_COLOR: [f32; 3] = [0.6, 0.65, 0.7];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Clock {
    Running { at: f32, since: Instant },
    Stopped(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct HeightFog {
    color: [f32; 3],
    shape: [f32; 3],
    motion: [f32; 4],
    edges: [f32; 4],
    clock: Clock,
}

#[derive(Clone, Debug, PartialEq)]
struct MapInfo {
    id: String,
    fog: MeshFog,
    low: f32,
    high: f32,
}

impl MapInfo {
    fn of(mesh: &MeshTag) -> Self {
        let (low, high) = ground_range(mesh);
        Self {
            id: mesh.mesh_id.as_str().to_owned(),
            fog: mesh.landscape_shading.fog,
            low,
            high,
        }
    }
}

fn resolve_height(
    game: fog_config::Game,
    mode: Fog,
    config: &FogConfig,
    flags: &FogLayer,
    map: &MapInfo,
    clock: Clock,
) -> Option<(HeightFog, FogSettings)> {
    let explicit = config.has_map_in(game, &map.id) || flags.has_overrides();
    if mode == Fog::On && (map.fog.density() != 0.0 || explicit) {
        let settings = config.resolve_in(game, &map.id, flags);
        Some((height_fog(&settings, map, clock), settings))
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MapFog {
    mode: Fog,
    mesh_fog: MeshFog,
    height: Option<HeightFog>,
}

impl Default for MapFog {
    fn default() -> Self {
        Self {
            mode: Fog::Off,
            mesh_fog: MeshFog::default(),
            height: None,
        }
    }
}

impl MapFog {
    pub(crate) fn at_map_load(args: &Args, mesh: &MeshTag) -> Self {
        let clock = Clock::Running {
            at: args.fog_time.unwrap_or(0.0),
            since: Instant::now(),
        };
        Self::new(args, mesh, clock, false)
    }

    pub(crate) fn at_map_load_headless(args: &Args, mesh: &MeshTag) -> Self {
        let seconds = args.fog_time.unwrap_or(args.ticks as f32 / 30.0);
        Self::new(args, mesh, Clock::Stopped(seconds), true)
    }

    fn new(args: &Args, mesh: &MeshTag, clock: Clock, headless: bool) -> Self {
        let config = if args::fog_mode(args.fog, args.sampling) == Fog::On {
            load_config(args, headless)
        } else {
            FogConfig::defaults()
        };
        let game = crate::tfl_game::chosen_game(args).0;
        Self::with_config_in(game, args, mesh, clock, &config)
    }

    #[cfg(test)]
    fn with_config(args: &Args, mesh: &MeshTag, clock: Clock, config: &FogConfig) -> Self {
        Self::with_config_in(fog_config::Game::Myth2, args, mesh, clock, config)
    }

    fn with_config_in(
        game: fog_config::Game,
        args: &Args,
        mesh: &MeshTag,
        clock: Clock,
        config: &FogConfig,
    ) -> Self {
        let mode = args::fog_mode(args.fog, args.sampling);
        let mesh_fog = drawn_mesh_fog(mode, mesh.landscape_shading.fog);
        let height = resolve_height(
            game,
            mode,
            config,
            &flag_layer(args),
            &MapInfo::of(mesh),
            clock,
        )
        .map(|(height, _)| height);
        Self {
            mode,
            mesh_fog,
            height,
        }
    }

    pub(crate) fn mode(&self) -> Fog {
        self.mode
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn has_height_fog(&self) -> bool {
        self.height.is_some()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn height_color(&self) -> Option<[f32; 3]> {
        self.height.map(|h| h.color)
    }

    pub(crate) fn seconds_now(&self) -> f32 {
        match self.height.map(|h| h.clock) {
            Some(Clock::Running { at, since }) => at + since.elapsed().as_secs_f32(),
            Some(Clock::Stopped(seconds)) => seconds,
            None => 0.0,
        }
    }

    pub(crate) fn apply(&self, uniforms: SceneUniforms, seconds: f32) -> SceneUniforms {
        let uniforms = uniforms.with_fog(&self.mesh_fog);
        let Some(fog) = self.height else {
            return uniforms;
        };
        let [density, base, inverse] = fog.shape;
        uniforms.with_height_fog(
            fog.color,
            [density, base, inverse, seconds],
            fog.motion,
            fog.edges,
        )
    }
}

#[derive(Debug)]
pub(crate) struct FogReload {
    watch: crate::config_watch::FileWatch,
    game: fog_config::Game,
    flags: FogLayer,
    map: MapInfo,
    clock: Clock,
    settings: Option<FogSettings>,
}

impl FogReload {
    pub(crate) fn at_map_load(args: &Args, mesh: &MeshTag, fog: &MapFog) -> Option<Self> {
        if fog.mode != Fog::On {
            return None;
        }
        let path = args.conf.clone().or_else(crate::conf::default_path)?;
        let game = crate::tfl_game::chosen_game(args).0;
        let flags = flag_layer(args);
        let map = MapInfo::of(mesh);
        let clock = fog.height.map_or(
            Clock::Running {
                at: args.fog_time.unwrap_or(0.0),
                since: Instant::now(),
            },
            |h| h.clock,
        );
        let (config, _, _) = fog_config::load_native(args.conf.as_deref(), false);
        let settings = resolve_height(game, Fog::On, &config, &flags, &map, clock).map(|(_, s)| s);
        Some(Self {
            watch: crate::config_watch::FileWatch::new(path)?,
            game,
            flags,
            map,
            clock,
            settings,
        })
    }

    pub(crate) fn poll(&mut self, fog: &mut MapFog) {
        let Some(read) = self.watch.poll() else {
            return;
        };
        let source = self.watch.path().display().to_string();
        let text = match read {
            Ok(text) => text,
            Err(e) => {
                eprintln!("fog: {source}: {e}; keeping the last values");
                return;
            }
        };
        let text = crate::conf::table_text(&text, &["fog"]);
        let (config, diagnostics) = FogConfig::with_override(&text, &source);
        if !diagnostics.is_empty() {
            for d in &diagnostics {
                eprintln!("fog: {d}");
            }
            eprintln!("fog: {source} not applied; keeping the last good values");
            return;
        }
        let clock = fog.height.map_or(self.clock, |h| h.clock);
        let resolved = resolve_height(self.game, fog.mode, &config, &self.flags, &self.map, clock);
        let settings = resolved.map(|(_, s)| s);
        eprintln!(
            "fog: {source} reloaded for {}: {}",
            self.map.id,
            describe_change(self.settings.as_ref(), settings.as_ref())
        );
        fog.height = resolved.map(|(h, _)| h);
        self.settings = settings;
    }
}

fn describe_change(old: Option<&FogSettings>, new: Option<&FogSettings>) -> String {
    let (old, new) = match (old, new) {
        (_, None) => return "no height fog".to_owned(),
        (None, Some(new)) => return format!("height fog on, {}", describe(new)),
        (Some(old), Some(new)) => (old, new),
    };
    let values = |s: &FogSettings| {
        [
            ("opacity", s.opacity),
            ("falloff", s.falloff),
            ("base", s.base),
            ("patchiness", s.patchiness),
            ("patch_size", s.patch_size),
            ("speed", s.speed),
            ("coverage", s.coverage),
            ("softness", s.softness),
        ]
    };
    let mut changes: Vec<String> = values(old)
        .iter()
        .zip(values(new))
        .filter(|((_, a), (_, b))| a != b)
        .map(|((key, a), (_, b))| format!("{key} {a} -> {b}"))
        .collect();
    if old.color != new.color {
        changes.push(format!("color {:?} -> {:?}", old.color, new.color));
    }
    if changes.is_empty() {
        "no change".to_owned()
    } else {
        changes.join(", ")
    }
}

fn describe(s: &FogSettings) -> String {
    format!(
        "opacity {}, falloff {}, base {}, patchiness {}, patch_size {}, speed {}, coverage {}, \
         softness {}, color {:?}",
        s.opacity,
        s.falloff,
        s.base,
        s.patchiness,
        s.patch_size,
        s.speed,
        s.coverage,
        s.softness,
        s.color
    )
}

fn drawn_mesh_fog(mode: Fog, map: MeshFog) -> MeshFog {
    if mode == Fog::Off {
        MeshFog::default()
    } else {
        map
    }
}

fn load_config(args: &Args, headless: bool) -> FogConfig {
    let (config, path, diagnostics) = fog_config::load_native(args.conf.as_deref(), headless);
    for d in &diagnostics {
        eprintln!("fog: {d}");
    }
    if let Some(path) = &path {
        info!("Fog: {} merged over the shipped settings", path.display());
    }
    config
}

fn flag_layer(args: &Args) -> FogLayer {
    FogLayer {
        opacity: args.fog_opacity,
        falloff: args.fog_falloff,
        base: args.fog_base,
        patchiness: args.fog_patchiness,
        patch_size: args.fog_patch_size,
        speed: args.fog_speed,
        coverage: args.fog_coverage,
        softness: args.fog_softness,
        color: args.fog_color,
    }
}

fn height_fog(settings: &FogSettings, info: &MapInfo, clock: Clock) -> HeightFog {
    let map = info.fog;
    let (low, high) = (info.low, info.high);
    let (shape, falloff) = fog_shape(settings, low, high);
    let color = settings.color.unwrap_or_else(|| {
        let tc = tag_color(&map);
        if map.density() == 0.0 && tc == [0.0, 0.0, 0.0] {
            FALLBACK_FOG_COLOR
        } else {
            tc
        }
    });
    info!(
        "[fog] {}: ground {low:.2}..{high:.2} cells, base {:.2}, falloff {falloff:.2}, \
         opacity {:.2}, coverage {:.2}, softness {:.2}, colour {color:?}, map density {}",
        info.id,
        shape[1],
        settings.opacity,
        settings.coverage,
        settings.softness,
        map.density(),
    );
    HeightFog {
        color,
        shape,
        motion: motion(settings),
        edges: patch_edges(settings.coverage, settings.softness),
        clock,
    }
}

const NOISE_QUANTILES: [f32; 21] = [
    -0.5, 0.239, 0.29, 0.327, 0.358, 0.385, 0.41, 0.433, 0.456, 0.478, 0.5, 0.522, 0.544, 0.567,
    0.59, 0.615, 0.642, 0.673, 0.71, 0.761, 1.5,
];

fn patch_edges(coverage: f32, softness: f32) -> [f32; 4] {
    let at = (1.0 - coverage.clamp(0.0, 1.0)) * 20.0;
    let i = (at.floor() as usize).min(19);
    let t = at - i as f32;
    let middle = NOISE_QUANTILES[i] + (NOISE_QUANTILES[i + 1] - NOISE_QUANTILES[i]) * t;
    let half = softness.clamp(0.01, 1.0) * 0.5;
    [middle - half, 1.0 / (2.0 * half), 0.0, 0.0]
}

fn motion(settings: &FogSettings) -> [f32; 4] {
    [
        settings.patch_size.max(0.1),
        settings.patchiness.clamp(0.0, 1.0),
        settings.speed * 0.93,
        settings.speed * 0.37,
    ]
}

fn tag_color(fog: &MeshFog) -> [f32; 3] {
    fog.color.map(|v| f32::from(v) / 65535.0)
}

fn ground_range(mesh: &MeshTag) -> (f32, f32) {
    let mut heights: Vec<f32> = mesh
        .cells
        .iter()
        .map(|c| {
            let h = if c.flags & 0x1800 != 0 {
                c.media_height
            } else {
                c.physical_height
            };
            f32::from(h) / 512.0
        })
        .collect();
    heights.sort_by(f32::total_cmp);
    let low = heights.first().copied().unwrap_or(0.0);
    let high = heights
        .get(heights.len() * 95 / 100)
        .copied()
        .unwrap_or(low);
    (low, high)
}

fn fog_shape(settings: &FogSettings, low: f32, high: f32) -> ([f32; 3], f32) {
    let falloff = ((high - low) * settings.falloff).max(0.01);
    let tau = -(1.0 - settings.opacity.clamp(0.0, 0.999)).ln();
    ([tau / falloff, low + settings.base, 1.0 / falloff], falloff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fog_config::{DEFAULT_FOG_TOML, FogConfig};
    use clap::Parser;
    use mandrake_common::FourCC;
    use mandrake_tag::LandscapeShading;

    fn flags(extra: &[&str]) -> FogLayer {
        let mut argv = vec!["mandrake"];
        argv.extend_from_slice(extra);
        flag_layer(&Args::try_parse_from(argv).unwrap())
    }

    fn shipped(tag: &str) -> FogSettings {
        FogConfig::defaults().resolve(tag, &FogLayer::default())
    }

    #[test]
    fn fog_off_is_the_mesh_without_fog() {
        let cr01 = MeshFog {
            color: [52428; 3],
            density_bits: 3e-5_f32.to_bits(),
        };
        assert_eq!(drawn_mesh_fog(Fog::On, cr01), cr01);
        assert_eq!(drawn_mesh_fog(Fog::Engine, cr01), cr01);
        assert_eq!(drawn_mesh_fog(Fog::Off, cr01), MeshFog::default());
    }

    #[test]
    fn a_tags_colour_is_used_unless_a_table_sets_one() {
        let fog = MeshFog {
            color: [65535, 0, 32768],
            density_bits: 1f32.to_bits(),
        };
        assert_eq!(tag_color(&fog), [1.0, 0.0, 32768.0 / 65535.0]);
        assert!(shipped("cr01").color.is_some(), "{}", *DEFAULT_FOG_TOML);
        assert_eq!(shipped("zzzz").color, None, "{}", *DEFAULT_FOG_TOML);
    }

    #[test]
    fn the_shape_from_the_ground_and_the_settings() {
        let settings = shipped("zzzz");
        let ([density, base, inverse], falloff) = fog_shape(&settings, 2.0, 12.0);
        let height = 10.0 * settings.falloff;
        assert!((falloff - height).abs() < 1e-5);
        assert!((density - (1.0 - settings.opacity).ln().abs() / height).abs() < 1e-5);
        assert_eq!(base, 2.0 + settings.base);
        assert!((inverse - 1.0 / height).abs() < 1e-5);
        let moved = FogSettings {
            base: -1.5,
            ..settings
        };
        assert_eq!(fog_shape(&moved, 2.0, 12.0).0[1], 0.5);
        assert_eq!(fog_shape(&settings, 3.0, 3.0).1, 0.01);
    }

    #[test]
    fn patch_edges_at_the_defaults_are_the_old_ones() {
        let settings = shipped("zzzz");
        assert_eq!(
            patch_edges(settings.coverage, settings.softness),
            [0.25, 2.0, 0.0, 0.0]
        );
        for tag in ["cr01", "84fd", "12mp", "13td", "89sn"] {
            let s = shipped(tag);
            assert_eq!((s.coverage, s.softness), (0.5, 0.5), "{tag}");
        }
        for softness in [0.01, 0.5, 1.0] {
            assert!(patch_edges(0.0, softness)[0] >= 1.0);
            let [start, inverse, _, _] = patch_edges(1.0, softness);
            assert!(start + 1.0 / inverse <= 0.0);
        }
        let [a, inverse, _, _] = patch_edges(0.3, 0.2);
        let b = a + 1.0 / inverse;
        assert!((b - a - 0.2).abs() < 1e-6);
        assert!(((a + b) / 2.0 - 0.59).abs() < 0.01, "{a} {b}");
        let mut last = f32::NEG_INFINITY;
        for i in (0..=100).rev() {
            let [lo, _, _, _] = patch_edges(i as f32 / 100.0, 0.5);
            assert!(lo >= last, "{i}");
            last = lo;
        }
    }

    #[test]
    fn a_reload_keeps_the_last_good_values() {
        let dir = std::env::temp_dir().join(format!("m2-fog-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mandrake.toml");
        std::fs::write(&path, "[lighting]\nx = 1\n[fog.map.aaaa]\ncoverage = 0.3\n").unwrap();
        let path_arg = path.display().to_string();
        let args = Args::try_parse_from(["mandrake", "--fog", "on", "--conf", &path_arg]).unwrap();
        let mesh = mock_mesh(b"aaaa", 0.0, [0; 3]);
        let mut fog = MapFog::at_map_load(&args, &mesh);
        let first = fog.height.unwrap().edges;
        assert_eq!(first, patch_edges(0.3, 0.5));
        let mut reload = FogReload::at_map_load(&args, &mesh, &fog).unwrap();
        assert_eq!(reload.settings.unwrap().coverage, 0.3);

        let settle = |reload: &mut FogReload, fog: &mut MapFog| {
            for _ in 0..3 {
                std::thread::sleep(crate::config_watch::INTERVAL);
                reload.poll(fog);
            }
        };
        std::fs::write(&path, "[lighting]\nx = 1\n[fog.map.aaaa]\ncoverage = 0.").unwrap();
        settle(&mut reload, &mut fog);
        assert_eq!(fog.height.unwrap().edges, first);
        std::fs::write(
            &path,
            "[lighting]\nx = 1\n[fog.map.aaaa]\ncoverage = 0.2\nsoftness = 7\n",
        )
        .unwrap();
        settle(&mut reload, &mut fog);
        assert_eq!(fog.height.unwrap().edges, first);
        let clock = fog.height.unwrap().clock;
        std::fs::write(
            &path,
            "[lighting]\nx = 1\n[fog.map.aaaa]\ncoverage = 0.2\nsoftness = 0.1\n",
        )
        .unwrap();
        settle(&mut reload, &mut fog);
        assert_eq!(fog.height.unwrap().edges, patch_edges(0.2, 0.1));
        assert_eq!(fog.height.unwrap().clock, clock);
        assert_eq!(
            describe_change(Some(&shipped("aaaa")), reload.settings.as_ref()),
            "coverage 0.5 -> 0.2, softness 0.5 -> 0.1"
        );
        std::fs::write(&path, "[fog.default]\nopacity = 0.2\n").unwrap();
        settle(&mut reload, &mut fog);
        assert!(fog.height.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_flags_beat_the_files() {
        let layer = flags(&[
            "--fog-opacity",
            "0.11",
            "--fog-falloff",
            "0.12",
            "--fog-base",
            "-0.13",
            "--fog-patchiness",
            "0.14",
            "--fog-patch-size",
            "15",
            "--fog-speed",
            "0.16",
            "--fog-coverage",
            "0.17",
            "--fog-softness",
            "0.18",
            "--fog-color",
            "0.1,0.2,0.3",
        ]);
        assert_eq!(
            FogConfig::defaults().resolve("cr01", &layer),
            FogSettings {
                opacity: 0.11,
                falloff: 0.12,
                base: -0.13,
                patchiness: 0.14,
                patch_size: 15.0,
                speed: 0.16,
                coverage: 0.17,
                softness: 0.18,
                color: Some([0.1, 0.2, 0.3]),
            }
        );
        assert_eq!(flags(&[]), FogLayer::default());
    }

    #[test]
    fn with_no_height_fog_the_shaders_get_what_main_gave_them() {
        let u = MapFog::default().apply(SceneUniforms::default(), 5.0);
        assert_eq!(u.fog, [0.0; 4]);
        assert_eq!(u.fog_height, [0.0; 4]);
        assert_eq!(u.fog_motion, [0.0; 4]);
        let cr01 = MeshFog {
            color: [52428; 3],
            density_bits: 3e-5f32.to_bits(),
        };
        let engine = MapFog {
            mode: Fog::Engine,
            mesh_fog: cr01,
            height: None,
        };
        let u = engine.apply(SceneUniforms::default(), 5.0);
        assert_eq!(u.fog, SceneUniforms::default().with_fog(&cr01).fog);
        assert_eq!(u.fog_height, [0.0; 4]);
    }

    #[test]
    fn a_stopped_clock_is_the_frame_and_a_running_one_starts_at_its_reading() {
        let fog = |clock| MapFog {
            mode: Fog::On,
            mesh_fog: MeshFog::default(),
            height: Some(HeightFog {
                color: [0.0; 3],
                shape: [1.0, 2.0, 3.0],
                motion: [6.0, 0.7, 0.1, 0.2],
                edges: [0.25, 0.75, 0.0, 0.0],
                clock,
            }),
        };
        let stopped = fog(Clock::Stopped(7.5));
        assert_eq!(stopped.seconds_now(), 7.5);
        let u = stopped.apply(SceneUniforms::default(), stopped.seconds_now());
        assert_eq!(u.fog_height, [1.0, 2.0, 3.0, 7.5]);
        let running = fog(Clock::Running {
            at: 100.0,
            since: Instant::now(),
        });
        assert!(running.seconds_now() >= 100.0);
        assert_eq!(MapFog::default().seconds_now(), 0.0);
    }

    fn mock_mesh(id: &[u8; 4], density: f32, color: [u16; 3]) -> MeshTag {
        MeshTag {
            mesh_id: FourCC::new(*id),
            landscape_tag: FourCC::new(*id),
            media_tag: FourCC::new(*b"test"),
            submesh_w: 1,
            submesh_h: 1,
            cell_width: 32,
            cell_height: 32,
            total_cells: 0,
            mesh_offset: 0,
            mesh_size: 0,
            cells: Vec::new(),
            palette_count: 0,
            palette_offset: 0,
            marker_count: 0,
            marker_offset: 0,
            palettes: Vec::new(),
            markers: Vec::new(),
            string_list_tag: None,
            overhead_tag: None,
            pregame_story_tag: None,
            postgame_tag: None,
            next_mesh_id: None,
            landscape_shading: LandscapeShading {
                dark_weight: 0,
                light_weight: 0,
                dark_color: [0; 3],
                light_color: [0; 3],
                transition: 0,
                fog: MeshFog {
                    color,
                    density_bits: density.to_bits(),
                },
            },
            reverb: None,
        }
    }

    #[test]
    fn density_zero_without_config_or_flags_has_no_height_fog() {
        let mesh = mock_mesh(b"zz00", 0.0, [0, 0, 0]);
        let args = Args::try_parse_from(["mandrake"]).unwrap();
        let config = FogConfig::defaults();
        let map_fog = MapFog::with_config(&args, &mesh, Clock::Stopped(0.0), &config);
        assert!(!map_fog.has_height_fog());
    }

    #[test]
    fn density_zero_with_explicit_map_entry_enables_height_fog_and_falls_back_to_mist_color() {
        let mesh = mock_mesh(b"zz00", 0.0, [0, 0, 0]);
        let args = Args::try_parse_from(["mandrake"]).unwrap();
        let (config, diags) = FogConfig::with_override("[map.zz00]\nopacity = 0.4\n", "test");
        assert_eq!(diags, []);
        let map_fog = MapFog::with_config(&args, &mesh, Clock::Stopped(0.0), &config);
        assert!(map_fog.has_height_fog());
        assert_eq!(map_fog.height_color(), Some(FALLBACK_FOG_COLOR));
    }

    #[test]
    fn density_zero_with_explicit_color_in_config_uses_specified_color() {
        let mesh = mock_mesh(b"zz00", 0.0, [0, 0, 0]);
        let args = Args::try_parse_from(["mandrake"]).unwrap();
        let (config, diags) = FogConfig::with_override(
            "[map.zz00]\nopacity = 0.4\ncolor = [0.1, 0.2, 0.3]\n",
            "test",
        );
        assert_eq!(diags, []);
        let map_fog = MapFog::with_config(&args, &mesh, Clock::Stopped(0.0), &config);
        assert!(map_fog.has_height_fog());
        assert_eq!(map_fog.height_color(), Some([0.1, 0.2, 0.3]));
    }

    #[test]
    fn stair_of_grief_shipped_entry_enables_height_fog_with_its_color() {
        let mesh = mock_mesh(b"12mp", 0.0, [0, 0, 0]);
        let args = Args::try_parse_from(["mandrake"]).unwrap();
        let config = FogConfig::defaults();
        let map_fog = MapFog::with_config(&args, &mesh, Clock::Stopped(0.0), &config);
        assert!(map_fog.has_height_fog());
        assert_eq!(map_fog.height_color(), Some([0.84, 0.88, 0.94]));
    }

    #[test]
    fn density_zero_with_cli_override_enables_height_fog() {
        let mesh = mock_mesh(b"zz00", 0.0, [0, 0, 0]);
        let args = Args::try_parse_from(["mandrake", "--fog-opacity", "0.35"]).unwrap();
        let config = FogConfig::defaults();
        let map_fog = MapFog::with_config(&args, &mesh, Clock::Stopped(0.0), &config);
        assert!(map_fog.has_height_fog());
        assert_eq!(map_fog.height_color(), Some(FALLBACK_FOG_COLOR));
    }

    #[test]
    fn stock_map_with_density_enables_height_fog_without_explicit_entry() {
        let mesh = mock_mesh(b"xxxx", 3e-5, [52428; 3]);
        let args = Args::try_parse_from(["mandrake"]).unwrap();
        let config = FogConfig::defaults();
        let map_fog = MapFog::with_config(&args, &mesh, Clock::Stopped(0.0), &config);
        assert!(map_fog.has_height_fog());
        assert_eq!(map_fog.height_color(), Some([52428.0 / 65535.0; 3]));
    }

    #[test]
    fn fog_off_or_engine_disables_height_fog_even_with_explicit_entry() {
        let mesh = mock_mesh(b"zz00", 0.0, [0, 0, 0]);
        let (config, _) = FogConfig::with_override("[map.zz00]\nopacity = 0.4\n", "test");
        let off_args = Args::try_parse_from(["mandrake", "--fog", "off"]).unwrap();
        assert!(
            !MapFog::with_config(&off_args, &mesh, Clock::Stopped(0.0), &config).has_height_fog()
        );
        let engine_args = Args::try_parse_from(["mandrake", "--fog", "engine"]).unwrap();
        assert!(
            !MapFog::with_config(&engine_args, &mesh, Clock::Stopped(0.0), &config)
                .has_height_fog()
        );
    }
}
