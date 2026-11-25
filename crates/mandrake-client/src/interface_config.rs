use crate::controls::{Diagnostic, line_of};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

pub static DEFAULT_INTERFACE_TOML: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| crate::conf::shipped_table(&["interface"]));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuFilter {
    Sharp,
    Lanczos2,
}

impl From<MenuFilter> for mandrake_view::interface_picture::InterfaceFilter {
    fn from(filter: MenuFilter) -> Self {
        match filter {
            MenuFilter::Sharp => Self::Sharp,
            MenuFilter::Lanczos2 => Self::Lanczos2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum CutsceneFilter {
    Sharp,
    Nearest,
    Linear,
    Lanczos2,
    Fsr1,
}

impl CutsceneFilter {
    fn parse(value: &str) -> Option<Self> {
        <Self as clap::ValueEnum>::from_str(value, false).ok()
    }
}

impl From<CutsceneFilter> for mandrake_view::interface_picture::InterfaceFilter {
    fn from(filter: CutsceneFilter) -> Self {
        match filter {
            CutsceneFilter::Sharp => Self::Sharp,
            CutsceneFilter::Nearest => Self::Nearest,
            CutsceneFilter::Linear => Self::Linear,
            CutsceneFilter::Lanczos2 => Self::Lanczos2,
            CutsceneFilter::Fsr1 => Self::Fsr1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuArt {
    Interface800x600,
    Stock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StatusBar {
    #[default]
    Original,
    FitWidth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum SelectionChoice {
    #[default]
    Myth2,
    Tfl,
}

impl SelectionChoice {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "myth2" => Some(Self::Myth2),
            "tfl" => Some(Self::Tfl),
            _ => None,
        }
    }

    pub fn tfl_rules(self) -> crate::selection_input::SelectionRules {
        match self {
            Self::Myth2 => crate::selection_input::SelectionRules::TFL_MYTH2,
            Self::Tfl => crate::selection_input::SelectionRules::TFL,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterfaceConfig {
    pub fullscreen: bool,
    pub control_bar: bool,
    pub film: bool,
    pub menu_filter: MenuFilter,
    pub lanczos_sharpen: f32,
    pub menu_art: MenuArt,
    pub cutscene_filter: CutsceneFilter,
    pub cutscene_hd: bool,
    pub fsr_sharpness: f32,
    pub status_bar: StatusBar,
    pub selection: SelectionChoice,
}

impl Default for InterfaceConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

impl InterfaceConfig {
    pub fn for_game(self, film: bool) -> Self {
        Self {
            control_bar: self.control_bar && !film,
            film,
            ..self
        }
    }

    pub fn with_cutscene_filter(self, flag: Option<CutsceneFilter>) -> Self {
        Self {
            cutscene_filter: flag.unwrap_or(self.cutscene_filter),
            ..self
        }
    }

    pub fn with_selection(self, flag: Option<SelectionChoice>) -> Self {
        Self {
            selection: flag.unwrap_or(self.selection),
            ..self
        }
    }

    pub fn with_cutscene_hd(self, flag: Option<bool>) -> Self {
        Self {
            cutscene_hd: flag.unwrap_or(self.cutscene_hd),
            ..self
        }
    }

    pub fn defaults() -> Self {
        let mut config = Self {
            fullscreen: false,
            control_bar: true,
            film: false,
            menu_filter: MenuFilter::Sharp,
            lanczos_sharpen: 0.0,
            menu_art: MenuArt::Stock,
            cutscene_filter: CutsceneFilter::Sharp,
            cutscene_hd: false,
            fsr_sharpness: 0.0,
            status_bar: StatusBar::FitWidth,
            selection: SelectionChoice::Tfl,
        };
        let diags = parse_over(&mut config, &DEFAULT_INTERFACE_TOML, "<defaults>");
        debug_assert!(
            diags.is_empty(),
            "shipped interface file is invalid: {diags:?}"
        );
        config
    }

    pub fn with_override(text: &str, source: &str) -> (Self, Vec<Diagnostic>) {
        let mut config = Self::defaults();
        let diags = parse_over(&mut config, text, source);
        (config, diags)
    }
}

fn parse_over(config: &mut InterfaceConfig, text: &str, source: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut report = |line: Option<usize>, message: String| {
        diags.push(Diagnostic {
            source: source.to_string(),
            line,
            message,
        })
    };
    let doc = match toml_edit::Document::parse(text.to_string()) {
        Ok(doc) => doc,
        Err(e) => {
            report(
                e.span().map(|r| line_of(text, r.start)),
                format!("TOML syntax error: {}", e.message().trim()),
            );
            return diags;
        }
    };
    let root = doc.as_table();
    for (key, item) in root.iter() {
        let at = root
            .get_key_value(key)
            .and_then(|(k, _)| k.span())
            .map(|r| line_of(text, r.start));
        match key {
            "fullscreen" => match item.as_bool() {
                Some(value) => config.fullscreen = value,
                None => report(at, "`fullscreen` must be true or false".to_owned()),
            },
            "control_bar" => match item.as_bool() {
                Some(value) => config.control_bar = value,
                None => report(at, "`control_bar` must be true or false".to_owned()),
            },
            "menu_filter" => match item.as_str() {
                Some("sharp") => config.menu_filter = MenuFilter::Sharp,
                Some("lanczos2") => config.menu_filter = MenuFilter::Lanczos2,
                _ => report(
                    at,
                    "`menu_filter` must be \"sharp\" or \"lanczos2\"".to_owned(),
                ),
            },
            "lanczos_sharpen" => match item
                .as_float()
                .or_else(|| item.as_integer().map(|v| v as f64))
            {
                Some(v) if v.is_finite() && (0.0..=1.0).contains(&v) => {
                    config.lanczos_sharpen = v as f32;
                }
                _ => report(
                    at,
                    "`lanczos_sharpen` must be a number from 0 to 1".to_owned(),
                ),
            },
            "cutscene_filter" => match item.as_str().and_then(CutsceneFilter::parse) {
                Some(filter) => config.cutscene_filter = filter,
                None => report(
                    at,
                    "`cutscene_filter` must be \"fsr1\", \"sharp\", \"lanczos2\", \"linear\" or \"nearest\""
                        .to_owned(),
                ),
            },
            "cutscene_hd" => match item.as_bool() {
                Some(value) => config.cutscene_hd = value,
                None => report(at, "`cutscene_hd` must be true or false".to_owned()),
            },
            "fsr_sharpness" => match item
                .as_float()
                .or_else(|| item.as_integer().map(|v| v as f64))
            {
                Some(v) if v.is_finite() && (0.0..=2.0).contains(&v) => {
                    config.fsr_sharpness = v as f32;
                }
                _ => report(
                    at,
                    "`fsr_sharpness` must be a number from 0 to 2 (stops; 0 the sharpest)"
                        .to_owned(),
                ),
            },
            "menu_art" => match item.as_str() {
                Some("800x600") => config.menu_art = MenuArt::Interface800x600,
                Some("stock") => config.menu_art = MenuArt::Stock,
                _ => report(at, "`menu_art` must be \"800x600\" or \"stock\"".to_owned()),
            },
            "status_bar" => match item.as_str() {
                Some("original") => config.status_bar = StatusBar::Original,
                Some("fit_width") => config.status_bar = StatusBar::FitWidth,
                _ => report(
                    at,
                    "`status_bar` must be \"original\" or \"fit_width\"".to_owned(),
                ),
            },
            "selection" => match item.as_str().and_then(SelectionChoice::parse) {
                Some(choice) => config.selection = choice,
                None => report(at, "`selection` must be \"myth2\" or \"tfl\"".to_owned()),
            },
            _ => report(
                at,
                format!(
                    "unknown key `{key}` (expected fullscreen, control_bar, menu_filter, lanczos_sharpen, menu_art, cutscene_filter, cutscene_hd, fsr_sharpness, status_bar or selection)"
                ),
            ),
        }
    }
    diags
}

fn load_native(path: Option<PathBuf>) -> (InterfaceConfig, Option<PathBuf>, Vec<Diagnostic>) {
    crate::conf::load_table(
        path,
        &["interface"],
        InterfaceConfig::defaults,
        InterfaceConfig::with_override,
    )
}

static TOGGLED_CONTROL_BAR: AtomicU8 = AtomicU8::new(0);

pub fn remember_control_bar(shown: bool) {
    TOGGLED_CONTROL_BAR.store(if shown { 2 } else { 1 }, Ordering::Relaxed);
}

fn with_toggled_control_bar(config: InterfaceConfig, toggled: u8) -> InterfaceConfig {
    match toggled {
        1 | 2 => InterfaceConfig {
            control_bar: toggled == 2,
            ..config
        },
        _ => config,
    }
}

fn write_control_bar(path: &Path, shown: bool) -> Result<(), String> {
    crate::conf::set_values(path, &["interface"], &[("control_bar", shown.into())])
}

pub fn save_at_exit(headless: bool) {
    let toggled = TOGGLED_CONTROL_BAR.load(Ordering::Relaxed);
    if toggled == 0 || headless {
        return;
    }
    let Some(path) = crate::conf::in_use() else {
        eprintln!("interface: no user config directory; the control bar's state is not saved");
        return;
    };
    match write_control_bar(&path, toggled == 2) {
        Ok(()) => tracing::info!(
            "Interface: control_bar = {} saved to {}",
            toggled == 2,
            path.display()
        ),
        Err(e) => eprintln!(
            "interface: {}: cannot save control_bar: {e}",
            path.display()
        ),
    }
}

pub fn load(conf: Option<&Path>, headless: bool) -> InterfaceConfig {
    let (config, path, diagnostics) = load_native(crate::conf::to_read(conf, headless));
    let config = with_toggled_control_bar(config, TOGGLED_CONTROL_BAR.load(Ordering::Relaxed));
    for d in &diagnostics {
        eprintln!("interface: {d}");
    }
    if let Some(path) = &path {
        tracing::info!(
            "Interface: {} merged over the shipped settings",
            path.display()
        );
    }
    mandrake_render::set_lanczos_sharpen(config.lanczos_sharpen);
    mandrake_render::interface_scale::set_fsr_sharpness(config.fsr_sharpness);
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_film_has_no_control_bar() {
        let shown = InterfaceConfig::defaults();
        assert!(shown.control_bar);
        assert!(!shown.for_game(true).control_bar);
        assert!(shown.for_game(false).control_bar);
        let hidden = InterfaceConfig {
            control_bar: false,
            ..shown
        };
        assert!(!hidden.for_game(true).control_bar);
        assert!(!hidden.for_game(false).control_bar);
        assert_eq!(shown.for_game(true).menu_art, shown.menu_art);
        assert!(shown.for_game(true).film);
        assert!(!shown.for_game(false).film);
    }

    #[test]
    fn the_toggled_control_bar_outlasts_the_game() {
        let shown = InterfaceConfig::defaults();
        assert_eq!(with_toggled_control_bar(shown, 0), shown);
        assert!(!with_toggled_control_bar(shown, 1).control_bar);
        let hidden = InterfaceConfig {
            control_bar: false,
            ..shown
        };
        assert!(with_toggled_control_bar(hidden, 2).control_bar);
        assert!(
            !with_toggled_control_bar(hidden, 2)
                .for_game(true)
                .control_bar
        );
    }

    #[test]
    fn the_control_bar_is_saved_to_the_settings_file() {
        let dir = std::env::temp_dir().join(format!("m2-interface-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("myth2-sourceport").join("mandrake.toml");

        write_control_bar(&path, false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Myth II source port settings"), "{text}");
        assert!(
            text.contains("[interface]\ncontrol_bar = false\n"),
            "{text}"
        );
        let (config, used, diags) = load_native(Some(path.clone()));
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(used.as_deref(), Some(path.as_path()));
        assert_eq!(
            config,
            InterfaceConfig {
                control_bar: false,
                ..InterfaceConfig::defaults()
            }
        );

        let user = "# mine\n[display]\nui_zoom = 150\n\n[interface]\nfullscreen = false # a window\ncontrol_bar   = false  # off\n\n# menus\nmenu_art = \"stock\"\n";
        std::fs::write(&path, user).unwrap();
        write_control_bar(&path, true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, user.replace("= false  # off", "= true  # off"));
        let (config, _, diags) = load_native(Some(path.clone()));
        assert!(diags.is_empty(), "{diags:?}");
        assert!(config.control_bar && !config.fullscreen);
        assert_eq!(config.menu_art, MenuArt::Stock);
        write_control_bar(&path, false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), user);

        std::fs::write(&path, "control_bar = = \n").unwrap();
        assert!(write_control_bar(&path, true).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "control_bar = = \n"
        );
        assert!(!path.with_extension("toml.new").exists());

        std::fs::write(
            &path,
            "[display]\nui_zoom = 150\n\n[interface]\nmenu_art = 3\n",
        )
        .unwrap();
        let (_, _, diags) = load_native(Some(path.clone()));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].line, Some(5), "{diags:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_shipped_file_shows_the_bar() {
        let mut config = InterfaceConfig {
            fullscreen: false,
            control_bar: false,
            film: false,
            menu_filter: MenuFilter::Lanczos2,
            lanczos_sharpen: 0.0,
            menu_art: MenuArt::Stock,
            cutscene_filter: CutsceneFilter::Sharp,
            cutscene_hd: false,
            fsr_sharpness: 0.0,
            status_bar: StatusBar::FitWidth,
            selection: SelectionChoice::Tfl,
        };
        assert!(parse_over(&mut config, &DEFAULT_INTERFACE_TOML, "shipped").is_empty());
        assert_eq!(config.status_bar, StatusBar::Original);
        assert_eq!(config.selection, SelectionChoice::Myth2);
        assert!(config.fullscreen);
        assert!(config.control_bar);
        assert_eq!(config.menu_filter, MenuFilter::Sharp);
        assert_eq!(config.lanczos_sharpen, 0.4);
        assert_eq!(config.menu_art, MenuArt::Interface800x600);
        assert_eq!(config.cutscene_filter, CutsceneFilter::Fsr1);
        assert!(config.cutscene_hd);
        assert_eq!(
            config.fsr_sharpness,
            mandrake_render::interface_scale::FSR_DEFAULT_SHARPNESS
        );
        assert_eq!(InterfaceConfig::defaults(), config);
    }

    #[test]
    fn selection_myth2_or_tfl() {
        assert_eq!(
            InterfaceConfig::defaults().selection,
            SelectionChoice::Myth2
        );
        let (config, diags) = InterfaceConfig::with_override("selection = \"tfl\"\n", "u");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config.selection, SelectionChoice::Tfl);
        assert_eq!(
            config.selection.tfl_rules(),
            crate::selection_input::SelectionRules::TFL
        );
        let (config, diags) = InterfaceConfig::with_override("selection = \"both\"\n", "u");
        assert_eq!(config.selection, SelectionChoice::Myth2);
        assert!(diags[0].message.contains("`selection`"), "{diags:?}");
    }

    #[test]
    fn status_bar_original_or_fit_width() {
        assert_eq!(InterfaceConfig::defaults().status_bar, StatusBar::Original);
        let (config, diags) = InterfaceConfig::with_override("status_bar = \"fit_width\"\n", "u");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config.status_bar, StatusBar::FitWidth);
        let (config, diags) = InterfaceConfig::with_override("status_bar = \"wide\"\n", "u");
        assert_eq!(config.status_bar, StatusBar::Original);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("`status_bar`"), "{diags:?}");
    }

    #[test]
    fn the_cutscene_filter_parses_and_defaults_to_fsr1() {
        let defaults = InterfaceConfig::defaults();
        assert_eq!(defaults.cutscene_filter, CutsceneFilter::Fsr1);
        assert_eq!(
            mandrake_view::interface_picture::InterfaceFilter::from(defaults.cutscene_filter),
            mandrake_view::interface_picture::InterfaceFilter::Fsr1
        );
        assert_eq!(defaults.menu_filter, MenuFilter::Sharp);
        for (value, filter) in [
            ("sharp", CutsceneFilter::Sharp),
            ("nearest", CutsceneFilter::Nearest),
            ("linear", CutsceneFilter::Linear),
            ("lanczos2", CutsceneFilter::Lanczos2),
            ("fsr1", CutsceneFilter::Fsr1),
        ] {
            let (config, diags) =
                InterfaceConfig::with_override(&format!("cutscene_filter = \"{value}\"\n"), "user");
            assert!(diags.is_empty(), "{diags:?}");
            assert_eq!(config.cutscene_filter, filter);
            assert_eq!(config.menu_filter, MenuFilter::Sharp);
        }
        let (config, diags) = InterfaceConfig::with_override("fsr_sharpness = 1\n", "user");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config.fsr_sharpness, 1.0);
        let (config, diags) = InterfaceConfig::with_override(
            "cutscene_filter = \"fsr2\"\nfsr_sharpness = 2.5\n",
            "user.toml",
        );
        assert_eq!(config, defaults);
        assert_eq!(diags.len(), 2, "{diags:?}");
        assert!(diags[0].message.contains("cutscene_filter"), "{diags:?}");
        assert_eq!(diags[1].line, Some(2));
        assert!(diags[1].message.contains("fsr_sharpness"), "{diags:?}");

        assert_eq!(defaults.with_cutscene_filter(None), defaults);
        let flagged = defaults.with_cutscene_filter(Some(CutsceneFilter::Lanczos2));
        assert_eq!(flagged.cutscene_filter, CutsceneFilter::Lanczos2);
        assert!(defaults.cutscene_hd);
        let (off, diags) = InterfaceConfig::with_override("cutscene_hd = false\n", "user");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(!off.cutscene_hd);
        assert!(off.with_cutscene_hd(Some(true)).cutscene_hd);
        assert!(!defaults.with_cutscene_hd(Some(false)).cutscene_hd);
        assert!(defaults.with_cutscene_hd(None).cutscene_hd);
        let (bad, diags) = InterfaceConfig::with_override("cutscene_hd = \"yes\"\n", "user");
        assert!(bad.cutscene_hd);
        assert!(diags[0].message.contains("cutscene_hd"), "{diags:?}");
        assert_eq!(flagged.menu_filter, defaults.menu_filter);
    }

    #[test]
    fn the_menu_settings_parse_and_a_bad_value_is_named() {
        let (config, diags) = InterfaceConfig::with_override(
            "menu_filter = \"lanczos2\"\nlanczos_sharpen = 0.6\nmenu_art = \"stock\"\n",
            "user",
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config.menu_filter, MenuFilter::Lanczos2);
        assert_eq!(config.lanczos_sharpen, 0.6);
        assert_eq!(config.menu_art, MenuArt::Stock);
        assert_eq!(
            mandrake_view::interface_picture::InterfaceFilter::from(config.menu_filter),
            mandrake_view::interface_picture::InterfaceFilter::Lanczos2
        );
        let (config, diags) = InterfaceConfig::with_override(
            "menu_filter = \"sharp\"\nlanczos_sharpen = 0\n",
            "user",
        );
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config.lanczos_sharpen, 0.0);
        assert_eq!(
            mandrake_view::interface_picture::InterfaceFilter::from(config.menu_filter),
            mandrake_view::interface_picture::InterfaceFilter::Sharp
        );
        let (config, diags) = InterfaceConfig::with_override("lanczos_sharpen = 1\n", "user");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config.lanczos_sharpen, 1.0);

        let (config, diags) = InterfaceConfig::with_override(
            "menu_filter = \"nearest\"\nlanczos_sharpen = 1.5\nmenu_art = 640\n",
            "user.toml",
        );
        assert_eq!(config, InterfaceConfig::defaults());
        assert_eq!(diags.len(), 3, "{diags:?}");
        assert_eq!(diags[0].line, Some(1));
        assert!(diags[0].message.contains("menu_filter"), "{diags:?}");
        assert_eq!(diags[1].line, Some(2));
        assert!(diags[1].message.contains("lanczos_sharpen"), "{diags:?}");
        assert_eq!(diags[2].line, Some(3));
        assert!(diags[2].message.contains("menu_art"), "{diags:?}");

        for bad in ["lanczos_sharpen = -0.1\n", "lanczos_sharpen = \"high\"\n"] {
            let (config, diags) = InterfaceConfig::with_override(bad, "user.toml");
            assert_eq!(config, InterfaceConfig::defaults());
            assert_eq!(diags.len(), 1, "{diags:?}");
            assert!(diags[0].message.contains("lanczos_sharpen"), "{diags:?}");
        }
    }

    #[test]
    fn fullscreen_is_the_default_and_a_user_file_turns_it_off() {
        assert!(InterfaceConfig::defaults().fullscreen);
        assert!(InterfaceConfig::defaults().for_game(true).fullscreen);
        let (config, diags) = InterfaceConfig::with_override("fullscreen = false\n", "user");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(!config.fullscreen);
        assert_eq!(
            config,
            InterfaceConfig {
                fullscreen: false,
                ..InterfaceConfig::defaults()
            }
        );
        for bad in ["fullscreen = \"no\"\n", "fullscreen = 0\n"] {
            let (config, diags) = InterfaceConfig::with_override(bad, "user.toml");
            assert!(config.fullscreen);
            assert_eq!(diags.len(), 1, "{diags:?}");
            assert_eq!(diags[0].line, Some(1));
            assert!(
                diags[0]
                    .message
                    .contains("`fullscreen` must be true or false"),
                "{diags:?}"
            );
        }
    }

    #[test]
    fn a_user_file_merges_over_the_shipped_one() {
        let (config, diags) = InterfaceConfig::with_override("control_bar = false\n", "user");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(!config.control_bar);
        let (config, diags) = InterfaceConfig::with_override("# nothing\n", "user");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(config, InterfaceConfig::defaults());
    }

    #[test]
    fn bad_entries_are_reported_and_keep_the_value() {
        let (config, diags) =
            InterfaceConfig::with_override("control_bar = \"off\"\nstatus = 1\n", "user.toml");
        assert!(config.control_bar);
        assert_eq!(diags.len(), 2, "{diags:?}");
        assert_eq!(diags[0].line, Some(1));
        assert!(diags[0].message.contains("true or false"), "{diags:?}");
        assert_eq!(diags[1].line, Some(2));
        assert!(
            diags[1].message.contains("unknown key `status`"),
            "{diags:?}"
        );

        let (config, diags) =
            InterfaceConfig::with_override("wrong = 1\ncontrol_bar = false\n", "user.toml");
        assert!(!config.control_bar);
        assert_eq!(diags.len(), 1, "{diags:?}");

        let (config, diags) = InterfaceConfig::with_override("control_bar = = \n", "user.toml");
        assert_eq!(config, InterfaceConfig::defaults());
        assert!(diags[0].message.contains("TOML syntax error"), "{diags:?}");
    }

    #[test]
    fn an_unreadable_file_is_reported() {
        let path = std::env::temp_dir().join(format!("m2-interface-none-{}", std::process::id()));
        let (config, used, diags) = load_native(Some(path));
        assert_eq!(config, InterfaceConfig::defaults());
        assert_eq!(used, None);
        assert!(diags[0].message.contains("cannot read"), "{diags:?}");
    }
}
