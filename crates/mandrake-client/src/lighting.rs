use crate::args::Lighting;
use crate::conf::layered::{Field, Layer, Layered};
use crate::controls::Diagnostic;
use crate::fog_config::Game;
use crate::scenery_billboards::ObjectShadow;
use crate::sprite_manager::ClientSpriteManager;
use mandrake_render::PointLight;
use mandrake_sim::Simulation;
use mandrake_tag::MeshTag;
use mandrake_view::landscape::OverlayMode;
use mandrake_view::model_surfaces::WorldFrame;
use tracing::info;

pub(crate) mod tfl;

pub static DEFAULT_LIGHTING_TOML: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| crate::conf::shipped_table(&["lighting"]));

pub const DEFAULT_LIGHT_INTENSITY: f32 = 1.125;

pub const MAX_ACTIVE_LIGHTS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightingLayer {
    pub darkness: Option<f32>,
    pub ambient_color: Option<[f32; 3]>,
    pub light_intensity: Option<f32>,
}

impl Layer for LightingLayer {
    const FIELDS: &'static [Field<Self>] = &[
        Field::Number {
            key: "darkness",
            slot: |l| &mut l.darkness,
            allowed: |v| (0.0..=1.0).contains(&v),
            range: "from 0 to 1",
        },
        Field::Number {
            key: "light_intensity",
            slot: |l| &mut l.light_intensity,
            allowed: |v| v >= 0.0,
            range: "above or equal to 0",
        },
        Field::Color {
            key: "ambient_color",
            slot: |l| &mut l.ambient_color,
            example: "[0.25, 0.32, 0.45]",
            maps_only: false,
        },
    ];
    const EXPECTED: [&'static str; 2] = ["darkness, ambient_color, or light_intensity"; 2];
    const LOWERCASE_TAGS: bool = true;
}

pub type LightingConfig = Layered<LightingLayer>;

impl Default for LightingConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl LightingConfig {
    pub fn defaults() -> Self {
        Self::of_shipped(&DEFAULT_LIGHTING_TOML, "lighting")
    }

    pub fn with_override(text: &str, source: &str) -> (Self, Vec<Diagnostic>) {
        Self::defaults().with_user(text, source)
    }

    pub fn has_map(&self, mesh_tag: &str) -> bool {
        self.has_map_in(Game::Myth2, mesh_tag)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn resolve_layer(&self, mesh_tag: &str, flags: &LightingLayer) -> LightingLayer {
        self.resolve_layer_in(Game::Myth2, mesh_tag, flags)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapLighting {
    pub enabled: bool,
    pub ambient_tint: [f32; 3],
    pub light_intensity: f32,
}

impl Default for MapLighting {
    fn default() -> Self {
        Self {
            enabled: false,
            ambient_tint: [1.0, 1.0, 1.0],
            light_intensity: DEFAULT_LIGHT_INTENSITY,
        }
    }
}

impl MapLighting {
    pub(crate) fn at_map_load(
        args: &crate::args::Args,
        mesh_tag: &MeshTag,
        mesh_name: &str,
    ) -> Self {
        let (config, path, diagnostics) = load_native(args.conf.as_deref());
        for d in &diagnostics {
            eprintln!("lighting: {d}");
        }
        if let Some(path) = &path {
            info!(
                "Lighting: {} merged over the shipped settings",
                path.display()
            );
        }
        let tag = map_table(&config, mesh_tag.mesh_id.as_str(), mesh_name);
        resolve_lighting(
            args.lighting.unwrap_or_default(),
            &config,
            &flag_layer(args),
            tag,
        )
    }
}

#[derive(Debug)]
pub(crate) struct LightingReload {
    watch: crate::config_watch::FileWatch,
    game: Game,
    mode: Lighting,
    flags: LightingLayer,
    mesh_id: String,
    mesh_name: String,
}

impl LightingReload {
    pub(crate) fn at_map_load(
        args: &crate::args::Args,
        mesh_tag: &MeshTag,
        mesh_name: &str,
    ) -> Option<Self> {
        if args.lighting.unwrap_or_default() == Lighting::Off {
            return None;
        }
        let path = args.conf.clone().or_else(crate::conf::default_path)?;
        Some(Self {
            watch: crate::config_watch::FileWatch::new(path)?,
            game: Game::Myth2,
            mode: args.lighting.unwrap_or_default(),
            flags: flag_layer(args),
            mesh_id: mesh_tag.mesh_id.as_str().to_owned(),
            mesh_name: mesh_name.to_owned(),
        })
    }

    pub(crate) fn for_tfl(args: &crate::args::Args, mesh_id: &str) -> Option<Self> {
        let path = args.conf.clone().or_else(crate::conf::default_path)?;
        Some(Self {
            watch: crate::config_watch::FileWatch::new(path)?,
            game: Game::Tfl,
            mode: Lighting::On,
            flags: flag_layer(args),
            mesh_id: mesh_id.to_owned(),
            mesh_name: mesh_id.to_owned(),
        })
    }

    pub(crate) fn poll(&mut self, lighting: &mut MapLighting) {
        let Some(read) = self.watch.poll() else {
            return;
        };
        let source = self.watch.path().display().to_string();
        let text = match read {
            Ok(text) => text,
            Err(e) => {
                eprintln!("lighting: {source}: {e}; keeping the last values");
                return;
            }
        };
        let text = crate::conf::table_text(&text, &["lighting"]);
        let (config, diagnostics) = LightingConfig::with_override(&text, &source);
        if !diagnostics.is_empty() {
            for d in &diagnostics {
                eprintln!("lighting: {d}");
            }
            eprintln!("lighting: {source} not applied; keeping the last good values");
            return;
        }
        let table = match self.game {
            Game::Myth2 => map_table(&config, &self.mesh_id, &self.mesh_name),
            Game::Tfl => self.mesh_id.as_str(),
        };
        let new = resolve_lighting_in(self.game, self.mode, &config, &self.flags, table);
        let describe = |l: &MapLighting| {
            if l.enabled {
                format!(
                    "ambient {:?}, light intensity {}",
                    l.ambient_tint, l.light_intensity
                )
            } else {
                "unlit".to_owned()
            }
        };
        if new == *lighting {
            eprintln!(
                "lighting: {source} reloaded for {}: no change",
                self.mesh_id
            );
        } else {
            eprintln!(
                "lighting: {source} reloaded for {}: {} -> {}",
                self.mesh_id,
                describe(lighting),
                describe(&new)
            );
        }
        *lighting = new;
    }
}

pub(crate) fn flag_layer(args: &crate::args::Args) -> LightingLayer {
    LightingLayer {
        darkness: args.lighting_darkness,
        ambient_color: args.lighting_ambient_color,
        light_intensity: args.lighting_light_intensity,
    }
}

fn map_table<'a>(config: &LightingConfig, mesh_id: &'a str, mesh_name: &'a str) -> &'a str {
    if config.has_map(mesh_id) {
        mesh_id
    } else {
        mesh_name
    }
}

pub(crate) fn resolve_lighting(
    lighting: Lighting,
    config: &LightingConfig,
    flags: &LightingLayer,
    mesh_name: &str,
) -> MapLighting {
    resolve_lighting_in(Game::Myth2, lighting, config, flags, mesh_name)
}

pub(crate) fn resolve_lighting_in(
    game: Game,
    lighting: Lighting,
    config: &LightingConfig,
    flags: &LightingLayer,
    mesh_name: &str,
) -> MapLighting {
    if lighting == Lighting::Off {
        return MapLighting::default();
    }

    let has_config = config.has_map_in(game, mesh_name);
    let has_cli_override = flags.darkness.is_some() || flags.ambient_color.is_some();

    if !has_config && !has_cli_override {
        return MapLighting::default();
    }

    let layer = config.resolve_layer_in(game, mesh_name, flags);
    let darkness = layer.darkness.unwrap_or(0.4);
    let ambient_color = layer.ambient_color.unwrap_or([1.0, 1.0, 1.0]);
    let light_intensity = layer.light_intensity.unwrap_or(DEFAULT_LIGHT_INTENSITY);

    let ambient_tint = [
        ambient_color[0] * darkness,
        ambient_color[1] * darkness,
        ambient_color[2] * darkness,
    ];

    MapLighting {
        enabled: true,
        ambient_tint,
        light_intensity,
    }
}

#[cfg(test)]
pub(crate) fn resolve_ambient(
    lighting: Lighting,
    darkness: Option<f32>,
    ambient_color: Option<[f32; 3]>,
    mesh_name: &str,
) -> ([f32; 3], bool) {
    let config = LightingConfig::defaults();
    let flags = LightingLayer {
        darkness,
        ambient_color,
        light_intensity: None,
    };
    let resolved = resolve_lighting(lighting, &config, &flags, mesh_name);
    (resolved.ambient_tint, resolved.enabled)
}

#[cfg(test)]
pub(crate) fn resolve_ambient_with_config(
    lighting: Lighting,
    config: &LightingConfig,
    darkness: Option<f32>,
    ambient_color: Option<[f32; 3]>,
    mesh_name: &str,
) -> ([f32; 3], bool) {
    let flags = LightingLayer {
        darkness,
        ambient_color,
        light_intensity: None,
    };
    let resolved = resolve_lighting(lighting, config, &flags, mesh_name);
    (resolved.ambient_tint, resolved.enabled)
}

mod native {
    use super::{Diagnostic, LightingConfig};
    use std::path::{Path, PathBuf};

    pub fn load_native(conf: Option<&Path>) -> (LightingConfig, Option<PathBuf>, Vec<Diagnostic>) {
        crate::conf::load_table(
            crate::conf::to_read(conf, false),
            &["lighting"],
            LightingConfig::defaults,
            LightingConfig::with_override,
        )
    }
}

pub use native::load_native;

pub(crate) fn extract_static_torch_lights(
    static_shadows: &[ObjectShadow],
    sim: &Simulation,
    mesh_tag: &MeshTag,
) -> Vec<PointLight> {
    let mut lights = Vec::new();
    let frame = WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height);

    for shadow in static_shadows {
        let overlay = shadow.overlay();
        if overlay.mode == OverlayMode::Lighten {
            let world_z = sim.mesh_height_at(overlay.x, overlay.y) + 512;
            lights.push(torch_light(frame, [overlay.x, overlay.y, world_z]));
        }
    }

    lights
}

pub(crate) fn fire_light(frame: WorldFrame, engine: [i32; 3], light_intensity: f32) -> PointLight {
    let world_pos = frame.from_engine(engine.map(|c| c as f32));
    PointLight {
        pos: [world_pos[0], world_pos[1], world_pos[2], 8.0],
        color: [1.0, 0.55, 0.15, light_intensity],
    }
}

fn torch_light(frame: WorldFrame, engine: [i32; 3]) -> PointLight {
    let world_pos = frame.from_engine(engine.map(|c| c as f32));
    PointLight {
        pos: [world_pos[0], world_pos[1], world_pos[2], 6.0],
        color: [1.0, 0.65, 0.2, 1.2],
    }
}

pub(crate) fn collect_scene_lights(
    projectile_surfaces: &[crate::projectile_local::LocalSurface],
    sprite_mgr: &ClientSpriteManager,
    mesh_tag: &MeshTag,
    camera_eye: [f32; 3],
    static_lights: &[PointLight],
    light_intensity: f32,
) -> Vec<PointLight> {
    scene_lights(
        projectile_surfaces,
        &sprite_mgr.local_projectile_sprites,
        WorldFrame::new(mesh_tag.cell_width, mesh_tag.cell_height),
        camera_eye,
        static_lights,
        light_intensity,
    )
}

pub(crate) fn scene_lights(
    projectile_surfaces: &[crate::projectile_local::LocalSurface],
    sprites: &crate::projectile_local::LocalSprites,
    frame: WorldFrame,
    camera_eye: [f32; 3],
    static_lights: &[PointLight],
    light_intensity: f32,
) -> Vec<PointLight> {
    let mut lights = Vec::with_capacity(projectile_surfaces.len() + static_lights.len());

    for s in projectile_surfaces {
        let has_light_overlay = usize::try_from(s.frame)
            .ok()
            .is_some_and(|f| sprites.is_lighten_overlay(&(s.collection_reference, s.sequence), f));

        let fourcc_bytes = s.collection_reference.0;
        let is_fire_tag = fourcc_bytes.starts_with(b"dw")
            || fourcc_bytes.starts_with(b"fire")
            || fourcc_bytes.starts_with(b"fb")
            || fourcc_bytes.starts_with(b"ex");

        if has_light_overlay || is_fire_tag {
            lights.push(fire_light(frame, s.world, light_intensity));
        }
    }

    lights.extend_from_slice(static_lights);

    lights.sort_by(|a, b| {
        let da = (a.pos[0] - camera_eye[0]).powi(2)
            + (a.pos[1] - camera_eye[1]).powi(2)
            + (a.pos[2] - camera_eye[2]).powi(2);
        let db = (b.pos[0] - camera_eye[0]).powi(2)
            + (b.pos[1] - camera_eye[1]).powi(2)
            + (b.pos[2] - camera_eye[2]).powi(2);
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    });

    lights.truncate(MAX_ACTIVE_LIGHTS);
    lights
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tfl_reload_reads_tfls_tables() {
        let dir = std::env::temp_dir().join(format!("m2-lighting-tfl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mandrake.toml");
        std::fs::write(&path, "[lighting.map.00tm]\ndarkness = 0.5\n").unwrap();
        let args = <crate::args::Args as clap::Parser>::parse_from([
            "mandrake",
            "--conf",
            path.to_str().unwrap(),
        ]);
        let mut reload = LightingReload::for_tfl(&args, "00tm").unwrap();
        let mut lighting = MapLighting::default();
        let settle = |reload: &mut LightingReload, lighting: &mut MapLighting| {
            for _ in 0..3 {
                std::thread::sleep(crate::config_watch::INTERVAL);
                reload.poll(lighting);
            }
        };
        std::fs::write(&path, "[lighting.map.00tm]\ndarkness = 0.6\n").unwrap();
        settle(&mut reload, &mut lighting);
        assert!(!lighting.enabled, "Myth II's 00tm lit TFL's");
        std::fs::write(
            &path,
            "[lighting.map.00tm]\ndarkness = 0.6\n[lighting.tfl.map.00tm]\ndarkness = 0.5\nambient_color = [1, 0.5, 0.25]\n",
        )
        .unwrap();
        settle(&mut reload, &mut lighting);
        assert!(lighting.enabled);
        assert_eq!(lighting.ambient_tint, [0.5, 0.25, 0.125]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shipped_lighting_toml_parses_cr01() {
        let config = LightingConfig::defaults();
        assert!(config.has_map("cr01"));
        assert!(config.has_map("CR01"));
        assert!(!config.has_map("00tm"));

        let layer = config.resolve_layer("cr01", &LightingLayer::default());
        assert_eq!(layer.darkness, Some(0.96));
        assert_eq!(layer.ambient_color, Some([0.50, 0.58, 0.74]));
        assert_eq!(layer.light_intensity, Some(0.65));

        for (tag, darkness, ambient, intensity) in [
            ("19it", 0.95, [0.28, 0.35, 0.5], 0.7),
            ("20sc", 0.95, [0.36, 0.42, 0.55], 0.65),
            ("11an", 0.95, [0.36, 0.42, 0.55], 0.65),
        ] {
            assert!(config.has_map(tag), "missing shipped map {tag}");
            let m = config.resolve_layer(tag, &LightingLayer::default());
            assert_eq!(m.darkness, Some(darkness), "{tag} darkness");
            assert_eq!(m.ambient_color, Some(ambient), "{tag} ambient_color");
            assert_eq!(m.light_intensity, Some(intensity), "{tag} light_intensity");
        }

        assert!(config.has_map_in(Game::Tfl, "2mll"));
        let gjol = config.resolve_layer_in(Game::Tfl, "2mll", &LightingLayer::default());
        assert_eq!(gjol.darkness, Some(0.8));
        assert_eq!(gjol.ambient_color, Some([0.76, 0.75, 0.77]));
        assert_eq!(gjol.light_intensity, Some(0.55));
    }

    #[test]
    fn resolve_ambient_respects_lighting_mode() {
        let (amb, enabled) = resolve_ambient(Lighting::Off, Some(0.4), None, "cr01");
        assert!(!enabled);
        assert_eq!(amb, [1.0, 1.0, 1.0]);

        let (amb, enabled) = resolve_ambient(Lighting::On, None, None, "cr01");
        assert!(enabled);
        assert_eq!(amb, [0.50 * 0.96, 0.58 * 0.96, 0.74 * 0.96]);

        let (amb, enabled) = resolve_ambient(Lighting::On, Some(0.4), None, "cr01");
        assert!(enabled);
        assert_eq!(amb, [0.50 * 0.4, 0.58 * 0.4, 0.74 * 0.4]);

        let (amb, enabled) = resolve_ambient(Lighting::On, None, None, "00tm");
        assert!(!enabled);
        assert_eq!(amb, [1.0, 1.0, 1.0]);

        let (amb, enabled) = resolve_ambient(Lighting::On, Some(0.3), None, "00tm");
        assert!(enabled);
        assert_eq!(amb, [0.3, 0.3, 0.3]);

        let (amb, enabled) =
            resolve_ambient(Lighting::On, Some(0.5), Some([0.2, 0.4, 0.6]), "other");
        assert!(enabled);
        assert_eq!(amb, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn user_config_override_enables_new_map() {
        let user_toml = r#"
[map.00tm]
darkness = 0.5
ambient_color = [0.1, 0.2, 0.3]
light_intensity = 2.0
"#;
        let (config, diags) = LightingConfig::with_override(user_toml, "test.toml");
        assert_eq!(diags, []);
        assert!(config.has_map("00tm"));

        let (amb, enabled) = resolve_ambient_with_config(Lighting::On, &config, None, None, "00tm");
        assert!(enabled);
        assert_eq!(amb, [0.05, 0.1, 0.15]);

        let resolved = resolve_lighting(Lighting::On, &config, &LightingLayer::default(), "00tm");
        assert_eq!(resolved.light_intensity, 2.0);
    }

    #[test]
    fn default_light_intensity_value() {
        assert_eq!(DEFAULT_LIGHT_INTENSITY, 1.125);
    }

    #[test]
    fn max_active_lights_cap() {
        assert_eq!(MAX_ACTIVE_LIGHTS, 8);
    }

    #[test]
    fn load_native_error() {
        let (config, used, diags) =
            load_native(Some(std::path::Path::new("/nonexistent/lighting.toml")));
        assert_eq!(config, LightingConfig::defaults());
        assert_eq!(used, None);
        assert!(
            diags[0].message.contains("cannot read settings file"),
            "{diags:?}"
        );
    }

    #[test]
    fn toml_syntax_error_reported() {
        let (config, diags) = LightingConfig::with_override("[map\n", "test.toml");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("TOML syntax error"), "{diags:?}");
        assert_eq!(config, LightingConfig::defaults());
    }
}
