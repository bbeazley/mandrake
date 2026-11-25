use crate::conf::layered::{Field, Layer, Layered};
use crate::controls::Diagnostic;
pub use mandrake_tag::tfl::Game;

pub static DEFAULT_FOG_TOML: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| crate::conf::shipped_table(&["fog"]));

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FogLayer {
    pub opacity: Option<f32>,
    pub falloff: Option<f32>,
    pub base: Option<f32>,
    pub patchiness: Option<f32>,
    pub patch_size: Option<f32>,
    pub speed: Option<f32>,
    pub coverage: Option<f32>,
    pub softness: Option<f32>,
    pub color: Option<[f32; 3]>,
}

impl FogLayer {
    pub fn has_overrides(&self) -> bool {
        self.is_set()
    }
}

macro_rules! number {
    ($key:ident, $allowed:expr, $range:literal) => {
        Field::Number {
            key: stringify!($key),
            slot: |l: &mut FogLayer| &mut l.$key,
            allowed: $allowed,
            range: $range,
        }
    };
}

impl Layer for FogLayer {
    const FIELDS: &'static [Field<Self>] = &[
        number!(opacity, |v| (0.0..1.0).contains(&v), "from 0 to below 1"),
        number!(falloff, |v| v > 0.0, "above 0"),
        number!(base, |_| true, "any number"),
        number!(patchiness, |v| (0.0..=1.0).contains(&v), "from 0 to 1"),
        number!(patch_size, |v| v > 0.0, "above 0"),
        number!(speed, |_| true, "any number"),
        number!(coverage, |v| (0.0..=1.0).contains(&v), "from 0 to 1"),
        number!(softness, |v| v > 0.0 && v <= 1.0, "above 0, up to 1"),
        Field::Color {
            key: "color",
            slot: |l| &mut l.color,
            example: "[0.28, 0.38, 0.32]",
            maps_only: true,
        },
    ];
    const EXPECTED: [&'static str; 2] = [
        "opacity, falloff, base, patchiness, patch_size, speed, coverage, softness",
        "opacity, falloff, base, patchiness, patch_size, speed, coverage, softness or color",
    ];
    const LOWERCASE_TAGS: bool = false;
}

#[cfg(test)]
type FogFile = crate::conf::layered::LayeredFile<FogLayer>;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FogSettings {
    pub opacity: f32,
    pub falloff: f32,
    pub base: f32,
    pub patchiness: f32,
    pub patch_size: f32,
    pub speed: f32,
    pub coverage: f32,
    pub softness: f32,
    pub color: Option<[f32; 3]>,
}

impl From<FogLayer> for FogSettings {
    fn from(layer: FogLayer) -> Self {
        Self {
            opacity: layer.opacity.unwrap_or_default(),
            falloff: layer.falloff.unwrap_or_default(),
            base: layer.base.unwrap_or_default(),
            patchiness: layer.patchiness.unwrap_or_default(),
            patch_size: layer.patch_size.unwrap_or_default(),
            speed: layer.speed.unwrap_or_default(),
            coverage: layer.coverage.unwrap_or_default(),
            softness: layer.softness.unwrap_or_default(),
            color: layer.color,
        }
    }
}

pub type FogConfig = Layered<FogLayer>;

impl Default for FogConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl FogConfig {
    pub fn defaults() -> Self {
        Self::of_shipped(&DEFAULT_FOG_TOML, "fog")
    }

    pub fn with_override(text: &str, source: &str) -> (Self, Vec<Diagnostic>) {
        Self::defaults().with_user(text, source)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn resolve(&self, mesh_tag: &str, flags: &FogLayer) -> FogSettings {
        self.resolve_in(Game::Myth2, mesh_tag, flags)
    }

    pub fn resolve_in(&self, game: Game, mesh_tag: &str, flags: &FogLayer) -> FogSettings {
        self.resolve_layer_in(game, mesh_tag, flags).into()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn has_map(&self, mesh_tag: &str) -> bool {
        self.has_map_in(Game::Myth2, mesh_tag)
    }

    #[cfg(test)]
    pub fn has_shipped_map(&self, mesh_tag: &str) -> bool {
        self.shipped.maps.contains_key(mesh_tag)
    }

    #[cfg(test)]
    pub fn has_user_map(&self, mesh_tag: &str) -> bool {
        self.user.maps.contains_key(mesh_tag)
    }
}

#[cfg(test)]
fn parse(text: &str, source: &str) -> (FogFile, Vec<Diagnostic>) {
    crate::conf::layered::parse(text, source)
}

mod native {
    use super::{Diagnostic, FogConfig};
    use std::path::{Path, PathBuf};

    pub fn load_native(
        conf: Option<&Path>,
        headless: bool,
    ) -> (FogConfig, Option<PathBuf>, Vec<Diagnostic>) {
        load_file(crate::conf::to_read(conf, headless))
    }

    pub(super) fn load_file(
        path: Option<PathBuf>,
    ) -> (FogConfig, Option<PathBuf>, Vec<Diagnostic>) {
        crate::conf::load_table(
            path,
            &["fog"],
            FogConfig::defaults,
            FogConfig::with_override,
        )
    }
}

pub use native::load_native;

#[cfg(test)]
mod tests {
    use super::*;

    const NO_FLAGS: FogLayer = FogLayer {
        opacity: None,
        falloff: None,
        base: None,
        patchiness: None,
        patch_size: None,
        speed: None,
        coverage: None,
        softness: None,
        color: None,
    };

    fn layered(shipped: &str, user: &str) -> FogConfig {
        let (shipped, diags) = parse(shipped, "shipped");
        assert_eq!(diags, []);
        let (user, diags) = parse(user, "user");
        assert_eq!(diags, []);
        FogConfig { shipped, user }
    }

    #[test]
    fn the_shipped_file_is_valid_and_its_default_table_is_complete() {
        let (file, diags) = parse(&DEFAULT_FOG_TOML, "<defaults>");
        assert_eq!(diags, []);
        let d = file.default;
        assert!(d.opacity.is_some() && d.falloff.is_some() && d.base.is_some());
        assert!(d.patchiness.is_some() && d.patch_size.is_some() && d.speed.is_some());
        assert_eq!(
            d.color, None,
            "a map's colour is its tag's unless overridden"
        );
    }

    #[test]
    fn precedence_lowest_first() {
        let config = layered(
            "[default]\nopacity = 0.1\nfalloff = 0.1\nbase = 0.1\npatchiness = 0.1\n\
             patch_size = 1\nspeed = 0.1\ncoverage = 0.1\nsoftness = 0.1\n\
             [map.aaaa]\nfalloff = 0.2\nbase = 0.2\npatchiness = 0.2\npatch_size = 2\n\
             coverage = 0.2\ncolor = [0.2, 0.2, 0.2]\n",
            "[default]\nbase = 0.3\npatchiness = 0.3\npatch_size = 3\nsoftness = 0.3\n\
             [map.aaaa]\npatchiness = 0.4\npatch_size = 4\nsoftness = 0.4\n",
        );
        let flags = FogLayer {
            patch_size: Some(5.0),
            ..NO_FLAGS
        };
        assert_eq!(
            config.resolve("aaaa", &flags),
            FogSettings {
                opacity: 0.1,
                falloff: 0.2,
                base: 0.3,
                patchiness: 0.4,
                patch_size: 5.0,
                speed: 0.1,
                coverage: 0.2,
                softness: 0.4,
                color: Some([0.2, 0.2, 0.2]),
            }
        );
        assert_eq!(
            config.resolve("bbbb", &NO_FLAGS),
            FogSettings {
                opacity: 0.1,
                falloff: 0.1,
                base: 0.3,
                patchiness: 0.3,
                patch_size: 3.0,
                speed: 0.1,
                coverage: 0.1,
                softness: 0.3,
                color: None,
            }
        );
    }

    #[test]
    fn a_map_row_with_any_subset_of_fields_works() {
        let base = FogConfig::defaults().resolve("zzzz", &NO_FLAGS);
        for (row, want) in [
            (
                "opacity = 0.01",
                FogSettings {
                    opacity: 0.01,
                    ..base
                },
            ),
            (
                "falloff = 0.01",
                FogSettings {
                    falloff: 0.01,
                    ..base
                },
            ),
            (
                "base = -0.01",
                FogSettings {
                    base: -0.01,
                    ..base
                },
            ),
            (
                "patchiness = 0.01",
                FogSettings {
                    patchiness: 0.01,
                    ..base
                },
            ),
            (
                "patch_size = 0.01",
                FogSettings {
                    patch_size: 0.01,
                    ..base
                },
            ),
            (
                "speed = 0.01",
                FogSettings {
                    speed: 0.01,
                    ..base
                },
            ),
            (
                "color = [0.1, 0.2, 0.3]",
                FogSettings {
                    color: Some([0.1, 0.2, 0.3]),
                    ..base
                },
            ),
            ("", base),
        ] {
            let text = format!("[map.zzzz]\n{row}\n");
            let (config, diags) = FogConfig::with_override(&text, "fog.toml");
            assert_eq!(diags, [], "{row}");
            assert_eq!(config.resolve("zzzz", &NO_FLAGS), want, "{row}");
        }
    }

    #[test]
    fn user_and_flag_colours_replace_the_shipped_one() {
        let config = layered(
            "[map.aaaa]\ncolor = [0.2, 0.2, 0.2]\n",
            "[map.aaaa]\ncolor = [0.3, 0.3, 0.3]\n",
        );
        assert_eq!(
            config.resolve("aaaa", &NO_FLAGS).color,
            Some([0.3, 0.3, 0.3])
        );
        let flags = FogLayer {
            color: Some([0.4, 0.4, 0.4]),
            ..NO_FLAGS
        };
        assert_eq!(config.resolve("aaaa", &flags).color, Some([0.4, 0.4, 0.4]));
    }

    #[test]
    fn coverage_and_softness_ranges() {
        let shipped = FogConfig::defaults().resolve("zzzz", &NO_FLAGS);
        assert_eq!((shipped.coverage, shipped.softness), (0.5, 0.5));
        for (text, ok) in [
            ("coverage = 0", true),
            ("coverage = 1", true),
            ("coverage = 0.3", true),
            ("coverage = -0.1", false),
            ("coverage = 1.1", false),
            ("softness = 0", false),
            ("softness = 0.05", true),
            ("softness = 1", true),
            ("softness = 1.5", false),
        ] {
            let (config, diags) = FogConfig::with_override(&format!("[default]\n{text}\n"), "f");
            assert_eq!(diags.is_empty(), ok, "{text}: {diags:?}");
            if !ok {
                assert_eq!(config.resolve("zzzz", &NO_FLAGS), shipped, "{text}");
            }
        }
    }

    #[test]
    fn a_bad_value_is_reported_and_changes_nothing_else() {
        let text = "[default]\nopacity = 1.5\nfalloff = 0.5\nbase = \"low\"\nsmell = 1\n\
                    color = [0, 0, 0]\n\
                    [map.stti]\nspeed = 2\ncolor = [1, 2, 3]\n\
                    [map.toolong]\nspeed = 3\n\
                    [other]\nx = 1\n";
        let (config, diags) = FogConfig::with_override(text, "fog.toml");
        let shown: Vec<String> = diags.iter().map(ToString::to_string).collect();
        for (line, part) in [
            (2, "`opacity` is 1.5, and must be from 0 to below 1"),
            (4, "`base` must be a number"),
            (5, "unknown key `smell`"),
            (6, "`color` is only allowed in a [map.<mesh tag>] table"),
            (9, "`color` must be three numbers from 0 to 1"),
            (10, "[map.toolong]: a mesh tag has four characters"),
            (12, "unknown key `other`"),
        ] {
            let start = format!("fog.toml:{line}: ");
            assert!(
                shown
                    .iter()
                    .any(|s| s.starts_with(&start) && s.contains(part)),
                "{start}{part} in {shown:#?}"
            );
        }
        assert_eq!(shown.len(), 7, "{shown:#?}");
        let shipped = FogConfig::defaults().resolve("stti", &NO_FLAGS);
        assert_eq!(
            config.resolve("stti", &NO_FLAGS),
            FogSettings {
                falloff: 0.5,
                speed: 2.0,
                ..shipped
            }
        );
    }

    #[test]
    fn a_syntax_error_leaves_the_shipped_settings() {
        let (config, diags) = FogConfig::with_override("[default\n", "fog.toml");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("TOML syntax error"), "{diags:?}");
        assert_eq!(config, FogConfig::defaults());
    }

    #[test]
    fn a_missing_explicit_file_is_a_diagnostic_and_the_shipped_settings() {
        let (config, used, diags) =
            load_native(Some(std::path::Path::new("/nonexistent/fog.toml")), true);
        assert_eq!(config, FogConfig::defaults());
        assert_eq!(used, None);
        assert!(
            diags[0].message.contains("cannot read settings file"),
            "{diags:?}"
        );
    }

    #[test]
    fn the_settings_files_fog_table_is_read() {
        let dir = std::env::temp_dir().join(format!("m2-fog-conf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mandrake.toml");
        std::fs::write(
            &path,
            "[lighting.map.cr01]\ndarkness = 0.5\n\n[fog.map.stti]\nfalloff = 0.5\nopacity = \"x\"\n",
        )
        .unwrap();
        let (config, used, diags) = native::load_file(Some(path.clone()));
        assert_eq!(used.as_deref(), Some(path.as_path()));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].line, Some(6), "{diags:?}");
        assert_eq!(config.resolve("stti", &NO_FLAGS).falloff, 0.5);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn layer_has_overrides() {
        assert!(!NO_FLAGS.has_overrides());
        assert!(
            FogLayer {
                opacity: Some(0.5),
                ..NO_FLAGS
            }
            .has_overrides()
        );
        assert!(
            FogLayer {
                falloff: Some(0.5),
                ..NO_FLAGS
            }
            .has_overrides()
        );
        assert!(
            FogLayer {
                base: Some(1.0),
                ..NO_FLAGS
            }
            .has_overrides()
        );
        assert!(
            FogLayer {
                patchiness: Some(0.5),
                ..NO_FLAGS
            }
            .has_overrides()
        );
        assert!(
            FogLayer {
                patch_size: Some(10.0),
                ..NO_FLAGS
            }
            .has_overrides()
        );
        assert!(
            FogLayer {
                speed: Some(1.0),
                ..NO_FLAGS
            }
            .has_overrides()
        );
        assert!(
            FogLayer {
                color: Some([0.1, 0.2, 0.3]),
                ..NO_FLAGS
            }
            .has_overrides()
        );
    }

    #[test]
    fn level_13_ships_the_stair_of_griefs_values() {
        let defaults = FogConfig::defaults();
        assert!(defaults.has_shipped_map("13td"));
        let r = defaults.resolve("13td", &NO_FLAGS);
        assert_eq!(r, defaults.resolve("12mp", &NO_FLAGS));
        assert_eq!(r.opacity, 0.5);
        assert_eq!(r.falloff, 0.5);
        assert_eq!(r.base, 0.0);
        assert_eq!(r.color, Some([0.84, 0.88, 0.94]));
        assert_eq!(r.patchiness, 0.5);
        assert_eq!(r.patch_size, 12.0);
        assert_eq!(r.speed, 1.0);
    }

    #[test]
    fn explicit_map_detection() {
        let defaults = FogConfig::defaults();
        assert!(defaults.has_map("cr01"));
        assert!(defaults.has_shipped_map("cr01"));
        assert!(!defaults.has_user_map("cr01"));
        assert!(!defaults.has_map("zz00"));
        assert!(!defaults.has_shipped_map("zz00"));
        assert!(!defaults.has_user_map("zz00"));
        assert!(defaults.has_map("12mp"));
        assert!(defaults.has_shipped_map("12mp"));
        assert!(!defaults.has_user_map("12mp"));

        let (user_cfg, diags) = FogConfig::with_override("[map.zz00]\nopacity = 0.4\n", "test");
        assert_eq!(diags, []);
        assert!(user_cfg.has_map("cr01"));
        assert!(user_cfg.has_shipped_map("cr01"));
        assert!(!user_cfg.has_user_map("cr01"));
        assert!(user_cfg.has_map("zz00"));
        assert!(!user_cfg.has_shipped_map("zz00"));
        assert!(user_cfg.has_user_map("zz00"));
        assert!(!user_cfg.has_map("zzzz"));
    }
}
