use std::path::{Component, Path, PathBuf};

const UI_ZOOM_KEY: &str = "ui_zoom";
const TFL_SMOOTHING_KEY: &str = "smoothing";
const TFL_HOVER_FRAMES_KEY: &str = "hover_frames";
const TFL_SPRITE_INTERPOLATION_KEY: &str = "sprite_interpolation";
const TFL_SOUND_RULES_KEY: &str = "sound_rules";
const ALLOW_DOWNLOADS_KEY: &str = "allow_downloads";
const DOWNLOAD_CAP_KEY: &str = "plugin_download_max_mb";
const DIALOG_SCALE_KEY: &str = "dialog_scale";

const DISPLAY_TABLE: &str = "display";
const SOUND_TABLE: &str = "sound";
const SOUND_VOLUME_KEY: &str = "volume";
pub const SOUND_VOLUME_DEFAULT: f32 = 1.0;

pub fn user_settings_path() -> Option<PathBuf> {
    crate::conf::in_use()
}

fn display_value<'a>(
    doc: &'a toml_edit::Document<String>,
    key: &str,
) -> Option<&'a toml_edit::Item> {
    doc.as_table().get(DISPLAY_TABLE)?.as_table_like()?.get(key)
}

fn display_tfl_value<'a>(
    doc: &'a toml_edit::Document<String>,
    key: &str,
) -> Option<&'a toml_edit::Item> {
    doc.as_table()
        .get(DISPLAY_TABLE)?
        .as_table_like()?
        .get("tfl")?
        .as_table_like()?
        .get(key)
}

fn parsed(text: &str) -> Result<toml_edit::Document<String>, String> {
    toml_edit::Document::parse(text.to_string()).map_err(|e| e.message().to_string())
}

pub fn parse_ui_zoom(text: &str) -> Result<Option<u16>, String> {
    let doc = parsed(text)?;
    let Some(item) = display_value(&doc, UI_ZOOM_KEY) else {
        return Ok(None);
    };
    match item.as_integer() {
        Some(v @ 100..=400) => Ok(Some(v as u16)),
        _ => Err(format!("{UI_ZOOM_KEY} must be a percent from 100 to 400")),
    }
}

#[cfg(test)]
fn with_ui_zoom(text: &str, percent: u16) -> Result<String, String> {
    crate::conf::with_values(
        text,
        &[DISPLAY_TABLE],
        &[(UI_ZOOM_KEY, i64::from(percent).into())],
    )
}

fn load_with<T>(
    path: &Path,
    parse: impl FnOnce(&str) -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    match crate::config_store::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn load_ui_zoom(path: &Path) -> Result<Option<u16>, String> {
    load_with(path, parse_ui_zoom)
}

pub fn parse_dialog_scale(text: &str) -> Result<Option<u16>, String> {
    let doc = parsed(text)?;
    let Some(item) = display_value(&doc, DIALOG_SCALE_KEY) else {
        return Ok(None);
    };
    let range = crate::ui_scale::DIALOG_SCALE_RANGE;
    item.as_integer()
        .and_then(|v| u16::try_from(v).ok())
        .filter(|v| range.contains(v))
        .map(Some)
        .ok_or_else(|| {
            format!(
                "{DIALOG_SCALE_KEY} must be a percent from {} to {}",
                range.start(),
                range.end()
            )
        })
}

pub fn dialog_scale_setting() -> u16 {
    let default = crate::ui_scale::DIALOG_SCALE_DEFAULT;
    user_settings_path()
        .and_then(|path| {
            load_with(&path, parse_dialog_scale).unwrap_or_else(|e| {
                tracing::warn!("settings: {}: {e}; dialogs at {default}%", path.display());
                None
            })
        })
        .unwrap_or(default)
}

fn parse_sound_volume(text: &str) -> Result<Option<f32>, String> {
    let doc = parsed(text)?;
    let Some(item) = doc
        .as_table()
        .get(SOUND_TABLE)
        .and_then(|t| t.as_table_like())
        .and_then(|t| t.get(SOUND_VOLUME_KEY))
    else {
        return Ok(None);
    };
    item.as_float()
        .or_else(|| item.as_integer().map(|v| v as f64))
        .filter(|v| (0.0..=16.0).contains(v))
        .map(|v| Some(v as f32))
        .ok_or_else(|| format!("[{SOUND_TABLE}] {SOUND_VOLUME_KEY} must be a number from 0 to 16"))
}

pub fn sound_volume(flag: Option<f32>) -> f32 {
    if let Some(v) = flag {
        return v;
    }
    user_settings_path()
        .and_then(|path| {
            load_with(&path, parse_sound_volume).unwrap_or_else(|e| {
                tracing::warn!(
                    "settings: {}: {e}; volume {SOUND_VOLUME_DEFAULT}",
                    path.display()
                );
                None
            })
        })
        .unwrap_or(SOUND_VOLUME_DEFAULT)
}

fn parse_switch(text: &str, key: &str) -> Result<Option<bool>, String> {
    let doc = parsed(text)?;
    switch_of(display_value(&doc, key), key)
}

fn parse_tfl_switch(text: &str, key: &str) -> Result<Option<bool>, String> {
    let doc = parsed(text)?;
    switch_of(display_tfl_value(&doc, key), key)
}

fn parse_tfl_sound_rules(text: &str) -> Result<Option<crate::tfl_sound::TflSoundRules>, String> {
    let doc = parsed(text)?;
    let Some(item) = display_tfl_value(&doc, TFL_SOUND_RULES_KEY) else {
        return Ok(None);
    };
    item.as_str()
        .and_then(crate::tfl_sound::TflSoundRules::parse)
        .map(Some)
        .ok_or_else(|| format!("{TFL_SOUND_RULES_KEY} must be \"myth2\" or \"tfl\""))
}

pub(crate) fn load_tfl_sound_rules(
    path: &Path,
) -> Result<Option<crate::tfl_sound::TflSoundRules>, String> {
    load_with(path, parse_tfl_sound_rules)
}

fn switch_of(item: Option<&toml_edit::Item>, key: &str) -> Result<Option<bool>, String> {
    let Some(item) = item else {
        return Ok(None);
    };
    match (item.as_bool(), item.as_str()) {
        (Some(on), _) => Ok(Some(on)),
        (_, Some("on")) => Ok(Some(true)),
        (_, Some("off")) => Ok(Some(false)),
        _ => Err(format!("{key} must be true or false")),
    }
}

fn load_tfl_switch(path: &Path, key: &str) -> Result<Option<bool>, String> {
    load_with(path, |text| parse_tfl_switch(text, key))
}

#[cfg(test)]
fn parse_tfl_smoothing(text: &str) -> Result<Option<bool>, String> {
    parse_tfl_switch(text, TFL_SMOOTHING_KEY)
}

pub fn load_tfl_smoothing(path: &Path) -> Result<Option<bool>, String> {
    load_tfl_switch(path, TFL_SMOOTHING_KEY)
}

pub fn load_tfl_hover_frames(path: &Path) -> Result<Option<bool>, String> {
    load_tfl_switch(path, TFL_HOVER_FRAMES_KEY)
}

pub fn load_tfl_sprite_interpolation(path: &Path) -> Result<Option<bool>, String> {
    load_tfl_switch(path, TFL_SPRITE_INTERPOLATION_KEY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadSettings {
    pub allow: bool,
    pub max_bytes: u64,
}

pub const DEFAULT_DOWNLOAD_CAP_MB: u64 = 512;

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            allow: true,
            max_bytes: DEFAULT_DOWNLOAD_CAP_MB << 20,
        }
    }
}

pub fn parse_download_settings(text: &str) -> Result<DownloadSettings, String> {
    let mut out = DownloadSettings::default();
    if let Some(allow) = parse_switch(text, ALLOW_DOWNLOADS_KEY)? {
        out.allow = allow;
    }
    let doc = parsed(text)?;
    if let Some(item) = display_value(&doc, DOWNLOAD_CAP_KEY) {
        out.max_bytes = match item.as_integer() {
            Some(mb @ 1..=0x10_0000) => (mb as u64) << 20,
            _ => return Err(format!("{DOWNLOAD_CAP_KEY} must be megabytes from 1")),
        };
    }
    Ok(out)
}

pub fn load_download_settings(path: &Path) -> Result<DownloadSettings, String> {
    load_with(path, |text| parse_download_settings(text).map(Some)).map(Option::unwrap_or_default)
}

pub fn download_settings() -> DownloadSettings {
    if cfg!(target_arch = "wasm32") {
        return DownloadSettings {
            allow: false,
            ..DownloadSettings::default()
        };
    }
    user_settings_path()
        .map(|path| {
            load_download_settings(&path).unwrap_or_else(|e| {
                tracing::warn!("settings: {}: {e}; downloads as by default", path.display());
                DownloadSettings::default()
            })
        })
        .unwrap_or_default()
}

pub fn downloads_dir() -> Option<PathBuf> {
    crate::user_dirs::config_file("downloads")
}

pub fn save_ui_zoom(path: &Path, percent: u16) -> Result<(), String> {
    crate::conf::set_values(
        path,
        &[DISPLAY_TABLE],
        &[(UI_ZOOM_KEY, i64::from(percent).into())],
    )
}

const NET_TABLE: &str = "net";

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn from_latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect()
}

pub fn parse_net_settings(
    text: &str,
    base: mandrake_session::net::NetSettings,
) -> Result<mandrake_session::net::NetSettings, String> {
    let doc = parsed(text)?;
    let Some(table) = doc
        .as_table()
        .get(NET_TABLE)
        .and_then(|t| t.as_table_like())
    else {
        return Ok(base);
    };
    net_table_over(table, base)
}

pub fn parse_tfl_net_settings(
    text: &str,
    base: mandrake_session::net::NetSettings,
) -> Result<mandrake_session::net::NetSettings, String> {
    let base = parse_net_settings(text, base)?;
    let doc = parsed(text)?;
    let Some(table) = doc
        .as_table()
        .get(NET_TABLE)
        .and_then(|t| t.as_table_like())
        .and_then(|t| t.get(TFL_TABLE))
        .and_then(|t| t.as_table_like())
    else {
        return Ok(base);
    };
    net_table_over(table, base)
}

pub fn load_tfl_net_settings(path: &Path) -> Result<mandrake_session::net::NetSettings, String> {
    load_with(path, |text| {
        parse_tfl_net_settings(text, net_defaults()).map(Some)
    })
    .map(|net| net.unwrap_or_else(net_defaults))
}

pub fn save_tfl_net_settings(
    path: &Path,
    net: &mandrake_session::net::NetSettings,
) -> Result<(), String> {
    crate::conf::set_values(path, &[NET_TABLE, TFL_TABLE], &net_values(net))
}

const TFL_TABLE: &str = "tfl";

fn net_table_over(
    table: &dyn toml_edit::TableLike,
    mut base: mandrake_session::net::NetSettings,
) -> Result<mandrake_session::net::NetSettings, String> {
    if let Some(name) = table.get("name").and_then(|v| v.as_str()) {
        base.name = mandrake_session::Text(from_latin1(name));
    }
    if let Some(team) = table.get("team").and_then(|v| v.as_str()) {
        base.team = mandrake_session::Text(from_latin1(team));
    }
    if let Some(address) = table.get("address").and_then(|v| v.as_str()) {
        base.address = mandrake_session::Text(from_latin1(address));
    }
    if let Some(icon) = table.get("icon") {
        base.icon = icon
            .as_integer()
            .and_then(|i| u16::try_from(i).ok())
            .ok_or("net.icon must be a number from 0 to 65535")?;
    }
    if let Some(port) = table.get("port") {
        base.port = port
            .as_integer()
            .and_then(|p| u16::try_from(p).ok())
            .ok_or("net.port must be a port number")?;
    }
    if let Some(colors) = table.get("colors") {
        let values: Option<Vec<u16>> = colors.as_array().map(|a| {
            a.iter()
                .filter_map(|v| v.as_integer().and_then(|n| u16::try_from(n).ok()))
                .collect()
        });
        base.colors = values
            .and_then(|v| <[u16; 8]>::try_from(v).ok())
            .ok_or("net.colors must be eight numbers from 0 to 65535")?;
    }
    Ok(base)
}

fn net_values(net: &mandrake_session::net::NetSettings) -> Vec<(&'static str, crate::conf::Value)> {
    let colors = net.colors.iter().map(|&c| i64::from(c)).collect();
    vec![
        ("name", latin1(&net.name.0).into()),
        ("team", latin1(&net.team.0).into()),
        ("icon", i64::from(net.icon).into()),
        ("colors", crate::conf::Value::Ints(colors)),
        ("port", i64::from(net.port).into()),
        ("address", latin1(&net.address.0).into()),
    ]
}

#[cfg(test)]
fn with_net_settings(
    text: &str,
    net: &mandrake_session::net::NetSettings,
) -> Result<String, String> {
    crate::conf::with_values(text, &[NET_TABLE], &net_values(net))
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn typed_net_name(name: &str) -> Vec<u8> {
    let kept: String = name.chars().filter(|c| matches!(c, ' '..='~')).collect();
    kept.trim().bytes().take(31).collect()
}

pub fn net_defaults() -> mandrake_session::net::NetSettings {
    mandrake_session::net::NetSettings {
        team: mandrake_session::Text(b"\xd5s Team".to_vec()),
        colors: [
            0x5998, 0x08f3, 0x2f4b, 0x0000, 0xc424, 0x8cb2, 0x1a27, 0x0000,
        ],
        ..mandrake_session::net::NetSettings::default()
    }
}

pub fn load_net_settings(path: &Path) -> Result<mandrake_session::net::NetSettings, String> {
    load_with(path, |text| {
        parse_net_settings(text, net_defaults()).map(Some)
    })
    .map(|net| net.unwrap_or_else(net_defaults))
}

pub fn save_net_settings(
    path: &Path,
    net: &mandrake_session::net::NetSettings,
) -> Result<(), String> {
    crate::conf::set_values(path, &[NET_TABLE], &net_values(net))
}

pub fn user_progress_path() -> Option<PathBuf> {
    crate::user_dirs::config_file("progress.txt")
}

#[derive(Debug, PartialEq, Eq)]
pub enum SavedProgress {
    Own(String),
    GamePreferences(Vec<u8>),
    None,
}

pub fn load_progress(path: &Path, game_dir: &Path) -> Result<SavedProgress, String> {
    match crate::config_store::read_to_string(path) {
        Ok(text) => return Ok(SavedProgress::Own(text)),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        Err(_) => {}
    }
    match crate::game_files::read(&game_dir.join("preferences").join("game 2")) {
        Ok(bytes) => Ok(SavedProgress::GamePreferences(bytes)),
        Err(_) => Ok(SavedProgress::None),
    }
}

fn resolved(path: &Path) -> PathBuf {
    let mut plain = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                plain.pop();
            }
            Component::CurDir => {}
            other => plain.push(other),
        }
    }
    let mut missing = Vec::new();
    let mut existing = plain.as_path();
    loop {
        if let Ok(found) = std::fs::canonicalize(existing) {
            return missing
                .iter()
                .rev()
                .fold(found, |path, part| path.join(part));
        }
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return plain;
        };
        missing.push(name);
        existing = parent;
    }
}

pub fn save_progress(path: &Path, game_dir: &Path, text: &str) -> Result<(), String> {
    if resolved(path).starts_with(resolved(game_dir)) {
        return Err(format!(
            "won't write {} into the game directory",
            path.display()
        ));
    }
    crate::config_store::write_replacing(path, "txt.tmp", text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_sound_volume_is_read() {
        assert_eq!(parse_sound_volume("[sound]\nvolume = 1.5\n"), Ok(Some(1.5)));
        assert_eq!(parse_sound_volume("[sound]\nvolume = 2\n"), Ok(Some(2.0)));
        assert_eq!(parse_sound_volume("[sound]\n"), Ok(None));
        assert_eq!(parse_sound_volume(""), Ok(None));
        assert!(parse_sound_volume("[sound]\nvolume = -1.0\n").is_err());
        assert!(parse_sound_volume("[sound]\nvolume = \"loud\"\n").is_err());
        let shipped = include_str!("../mandrake.default.toml");
        assert_eq!(parse_sound_volume(shipped), Ok(None));
        assert!(shipped.contains("# volume = 1.0"));
        assert_eq!(sound_volume(Some(0.25)), 0.25);
    }

    use super::*;

    #[test]
    fn a_typed_name_keeps_printable_ascii_up_to_31_bytes() {
        assert_eq!(typed_net_name("  Ranger "), b"Ranger");
        assert_eq!(typed_net_name("Ren\u{e9}e\u{1}\u{2019}s"), b"Renes");
        assert_eq!(typed_net_name(&"x".repeat(40)).len(), 31);
        assert_eq!(typed_net_name(""), b"");
    }

    #[test]
    fn the_net_table_keeps_the_player_and_the_rest_of_the_file() {
        let net = mandrake_session::net::NetSettings {
            name: mandrake_session::Text(b"Tester".to_vec()),
            team: mandrake_session::Text(b"Blue\0am".to_vec()),
            icon: 3,
            colors: [0x5998, 0x08f3, 0x2f4b, 0, 0xc424, 0x8cb2, 0x1a27, 0],
            port: 3454,
            address: mandrake_session::Text(b"10.0.0.2:3453".to_vec()),
        };
        let mine =
            "[display]\nui_zoom = 150 # mine\n\n[net]\n# my name\nname = \"Old\"\nextra = 1\n";
        let text = with_net_settings(mine, &net).unwrap();
        assert_eq!(parse_ui_zoom(&text), Ok(Some(150)));
        assert!(text.contains("ui_zoom = 150 # mine\n"), "{text}");
        assert!(text.contains("# my name\nname = \"Tester\"\n"), "{text}");
        let back =
            parse_net_settings(&text, mandrake_session::net::NetSettings::default()).unwrap();
        assert_eq!(back, net);
        let fresh = with_net_settings("", &net).unwrap();
        assert_eq!(
            parse_net_settings(&fresh, mandrake_session::net::NetSettings::default()),
            Ok(net.clone())
        );
        assert!(parse_net_settings("[net]\ncolors = [1, 2]\n", back.clone()).is_err());
        assert_eq!(parse_net_settings("", back.clone()), Ok(back));
    }

    #[test]
    fn parses_dialog_scale() {
        assert_eq!(parse_dialog_scale(""), Ok(None));
        assert_eq!(parse_dialog_scale("[display]\nui_zoom = 200\n"), Ok(None));
        assert_eq!(
            parse_dialog_scale("[display]\ndialog_scale = 75\n"),
            Ok(Some(75))
        );
        assert_eq!(
            parse_dialog_scale("[display]\ndialog_scale = 100\n"),
            Ok(Some(100))
        );
        for bad in ["0", "10", "101", "-75", "\"75\"", "0.75"] {
            assert!(
                parse_dialog_scale(&format!("[display]\ndialog_scale = {bad}\n")).is_err(),
                "{bad}"
            );
        }
        assert!(crate::conf::DEFAULT_CONF_TOML.contains("# dialog_scale = 75"));
        assert_eq!(parse_dialog_scale(crate::conf::DEFAULT_CONF_TOML), Ok(None));
    }

    #[test]
    fn parses_ui_zoom() {
        assert_eq!(parse_ui_zoom(""), Ok(None));
        assert_eq!(parse_ui_zoom("[display]\nother = 1\n"), Ok(None));
        assert_eq!(parse_ui_zoom("[display]\nui_zoom = 150\n"), Ok(Some(150)));
        assert_eq!(parse_ui_zoom("[display]\nui_zoom = 130\n"), Ok(Some(130)));
        assert!(parse_ui_zoom("[display]\nui_zoom = 90\n").is_err());
        assert!(parse_ui_zoom("[display]\nui_zoom = 500\n").is_err());
        assert!(parse_ui_zoom("[display]\nui_zoom = \"big\"\n").is_err());
        assert!(parse_ui_zoom("[display]\nui_zoom = \n").is_err());
    }

    #[test]
    fn parses_tfl_smoothing() {
        assert_eq!(parse_tfl_smoothing(""), Ok(None));
        assert_eq!(parse_tfl_smoothing("[display]\nvsync = true\n"), Ok(None));
        assert_eq!(
            parse_tfl_smoothing("[display]\nsmoothing = false\n"),
            Ok(None)
        );
        assert_eq!(
            parse_tfl_smoothing("[display.tfl]\nsmoothing = false\n"),
            Ok(Some(false))
        );
        assert_eq!(
            parse_tfl_smoothing("[display]\nvsync = true\n[display.tfl]\nsmoothing = \"on\"\n"),
            Ok(Some(true))
        );
        assert!(parse_tfl_smoothing("[display.tfl]\nsmoothing = 30\n").is_err());
        let text = with_ui_zoom("[display.tfl]\nsmoothing = false\n", 150).unwrap();
        assert_eq!(parse_tfl_smoothing(&text), Ok(Some(false)));
        assert_eq!(parse_ui_zoom(&text), Ok(Some(150)));
    }

    #[test]
    fn parses_tfl_hover_frames() {
        assert_eq!(parse_tfl_switch("", TFL_HOVER_FRAMES_KEY), Ok(None));
        assert_eq!(
            parse_tfl_switch(
                "[display.tfl]\nhover_frames = false\n",
                TFL_HOVER_FRAMES_KEY
            ),
            Ok(Some(false))
        );
        assert!(
            parse_tfl_switch("[display.tfl]\nhover_frames = 1\n", TFL_HOVER_FRAMES_KEY).is_err()
        );
    }

    #[test]
    fn parses_tfl_sprite_interpolation() {
        let key = TFL_SPRITE_INTERPOLATION_KEY;
        assert_eq!(parse_tfl_switch("", key), Ok(None));
        assert_eq!(
            parse_tfl_switch("[display]\nsprite_interpolation = false\n", key),
            Ok(None)
        );
        assert_eq!(
            parse_tfl_switch("[display.tfl]\nsprite_interpolation = false\n", key),
            Ok(Some(false))
        );
        assert_eq!(
            parse_tfl_switch("[display.tfl]\nsprite_interpolation = \"on\"\n", key),
            Ok(Some(true))
        );
        assert!(parse_tfl_switch("[display.tfl]\nsprite_interpolation = 2\n", key).is_err());
    }

    #[test]
    fn parses_tfl_sound_rules() {
        use crate::tfl_sound::TflSoundRules;
        assert_eq!(parse_tfl_sound_rules(""), Ok(None));
        assert_eq!(
            parse_tfl_sound_rules("[display]\nsound_rules = \"tfl\"\n"),
            Ok(None)
        );
        assert_eq!(
            parse_tfl_sound_rules("[display.tfl]\nsound_rules = \"tfl\"\n"),
            Ok(Some(TflSoundRules::Tfl))
        );
        assert_eq!(
            parse_tfl_sound_rules("[display.tfl]\nsound_rules = \"myth2\"\n"),
            Ok(Some(TflSoundRules::Myth2))
        );
        assert!(parse_tfl_sound_rules("[display.tfl]\nsound_rules = \"TFL\"\n").is_err());
        assert!(parse_tfl_sound_rules("[display.tfl]\nsound_rules = true\n").is_err());
        assert_eq!(TflSoundRules::default(), TflSoundRules::Myth2);
    }

    #[test]
    fn the_shipped_file_documents_sound_rules_and_sprite_interpolation() {
        use crate::tfl_sound::TflSoundRules;
        let shipped = crate::conf::DEFAULT_CONF_TOML.replace("\r\n", "\n");
        assert_eq!(parse_tfl_sound_rules(&shipped), Ok(None));
        assert_eq!(
            parse_tfl_switch(&shipped, TFL_SPRITE_INTERPOLATION_KEY),
            Ok(None)
        );
        let uncommented = |line: &str| {
            assert!(shipped.contains(&format!("\n# {line}\n")), "{line}");
            shipped.replace(&format!("\n# {line}\n"), &format!("\n{line}\n"))
        };
        assert_eq!(
            parse_tfl_sound_rules(&uncommented("sound_rules = \"myth2\"")),
            Ok(Some(TflSoundRules::Myth2))
        );
        assert_eq!(
            parse_tfl_switch(
                &uncommented("sprite_interpolation = true"),
                TFL_SPRITE_INTERPOLATION_KEY
            ),
            Ok(Some(true))
        );
    }

    #[test]
    fn writes_only_the_value() {
        let new = with_ui_zoom("", 200).unwrap();
        assert_eq!(parse_ui_zoom(&new), Ok(Some(200)));

        let edited =
            "# mine\n[display]\nui_zoom   = 150  # note\nother = \"x\"\n\n[table]\na = 1\n";
        assert_eq!(
            with_ui_zoom(edited, 300).unwrap(),
            "# mine\n[display]\nui_zoom   = 300  # note\nother = \"x\"\n\n[table]\na = 1\n"
        );

        let added = with_ui_zoom("[table]\na = 1\n", 125).unwrap();
        assert_eq!(parse_ui_zoom(&added), Ok(Some(125)));
        assert!(added.starts_with("[table]\na = 1\n"), "{added}");

        assert!(with_ui_zoom("not toml = = 1", 125).is_err());
        assert_eq!(parse_ui_zoom("ui_zoom = 150\n"), Ok(None));
    }

    #[test]
    fn saves_and_loads_a_file() {
        let dir = std::env::temp_dir().join(format!("m2-settings-test-{}", std::process::id()));
        let path = dir.join("sub").join("mandrake.toml");
        assert_eq!(load_ui_zoom(&path), Ok(None));
        save_ui_zoom(&path, 175).unwrap();
        assert_eq!(load_ui_zoom(&path), Ok(Some(175)));
        save_ui_zoom(&path, 400).unwrap();
        assert_eq!(load_ui_zoom(&path), Ok(Some(400)));
        std::fs::write(&path, "= broken").unwrap();
        assert!(save_ui_zoom(&path, 200).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "= broken");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn downloads_are_allowed_by_default_and_capped() {
        let d = parse_download_settings("").unwrap();
        assert_eq!(d, DownloadSettings::default());
        assert!(d.allow);
        assert_eq!(d.max_bytes, 512 << 20);
        let d = parse_download_settings(
            "[display]\nallow_downloads = false\nplugin_download_max_mb = 64\n",
        )
        .unwrap();
        assert!(!d.allow);
        assert_eq!(d.max_bytes, 64 << 20);
        assert!(parse_download_settings("[display]\nplugin_download_max_mb = 0\n").is_err());
        assert!(parse_download_settings("[display]\nallow_downloads = 3\n").is_err());
        if let (Some(dir), Some(downloads)) = (crate::user_dirs::config_dir(), downloads_dir()) {
            assert_eq!(Some(dir.as_path()), downloads.parent());
        }
    }

    #[test]
    fn progress_lives_in_the_config_folder() {
        if let (Some(dir), Some(progress)) = (crate::user_dirs::config_dir(), user_progress_path())
        {
            assert_eq!(Some(dir.as_path()), progress.parent());
            assert_eq!(progress.file_name().unwrap(), "progress.txt");
        }
    }

    #[test]
    fn no_game_preferences_is_a_fresh_player() {
        let dir = std::env::temp_dir().join(format!("m2-progress-fresh-{}", std::process::id()));
        assert_eq!(
            load_progress(&dir.join("progress.txt"), &dir.join("no game")),
            Ok(SavedProgress::None)
        );
    }
}
