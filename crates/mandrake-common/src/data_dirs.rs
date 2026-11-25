//! Where the games' data is looked for when no flag names it.
//!
//! Lowest first, what decides a folder is: the default (below), the settings file's `[game]`
//! table ([`set_from_settings`]), then the environment variable (`MYTH2_DATA_DIR`,
//! `TFL_DATA_DIR`). The client's flags (`--archive`, `--tfl-data`) win over all of them.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// The settings file's `[game]` `myth2_data` and `tfl_data` ([`set_from_settings`]).
static FROM_SETTINGS: RwLock<(Option<PathBuf>, Option<PathBuf>)> = RwLock::new((None, None));

/// Records the settings file's data folders (its `[game]` `myth2_data`, `tfl_data`, already made
/// absolute or relative to the current folder), for [`myth2_dir`] and [`tfl_dir`] below their
/// environment variables. Set once at the start, before anything opens the data.
pub fn set_from_settings(myth2: Option<PathBuf>, tfl: Option<PathBuf>) {
    if let Ok(mut from) = FROM_SETTINGS.write() {
        *from = (myth2, tfl);
    }
}

fn from_settings() -> (Option<PathBuf>, Option<PathBuf>) {
    FROM_SETTINGS
        .read()
        .map(|f| f.clone())
        .unwrap_or((None, None))
}

/// The environment variable's folder if it is set, else the settings file's.
fn env_or(env: Option<OsString>, settings: Option<PathBuf>) -> Option<PathBuf> {
    env.map(PathBuf::from).or(settings)
}

/// Returns the portable directory beside `exe` (or beside `*.app` on macOS).
pub fn portable_dir_from_exe(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    if dir.file_name().and_then(|n| n.to_str()) == Some("MacOS")
        && let Some(contents) = dir.parent()
        && contents.file_name().and_then(|n| n.to_str()) == Some("Contents")
        && let Some(bundle) = contents.parent()
        && bundle
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
        && let Some(beside_app) = bundle.parent()
    {
        return Some(beside_app.to_path_buf());
    }
    Some(dir.to_path_buf())
}

/// The portable directory beside the running executable (or beside `Mandrake.app` on macOS).
pub fn portable_exe_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    portable_dir_from_exe(&exe)
}

/// Myth II's game folder: `MYTH2_DATA_DIR`, else the settings file's `myth2_data`, else the
/// executable's own folder (drop-in beside `mandrake` or `Mandrake.app`) when it holds `tags`,
/// else the current folder when it holds `tags`, else `data` beside the executable when that does,
/// else the current folder.
pub fn myth2_dir() -> PathBuf {
    if let Some(dir) = env_or(std::env::var_os("MYTH2_DATA_DIR"), from_settings().0) {
        return dir;
    }
    let exe_dir = portable_exe_dir();
    if let Some(dir) = exe_dir.as_ref()
        && dir.join("tags").is_dir()
    {
        return dir.clone();
    }
    beside_exe_fallback(Path::new("tags").is_dir(), exe_dir.map(|d| d.join("data")))
}

/// [`myth2_dir`]'s last two choices: the current folder (`""`) when it holds the game, else
/// `beside` (the executable's `data`) when it holds `tags`, else the current folder.
fn beside_exe_fallback(here_has_tags: bool, beside: Option<PathBuf>) -> PathBuf {
    match beside {
        Some(dir) if !here_has_tags && dir.join("tags").is_dir() => dir,
        _ => PathBuf::new(),
    }
}

/// Whether `dir` has a `tags/` folder containing at least one `.gor` file (TFL's tag archives).
fn has_tfl_tags(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir.join("tags")) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|e| {
        e.path()
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gor"))
    })
}

/// TFL's game folder: `TFL_DATA_DIR`, else the settings file's `tfl_data`, else the executable's
/// own folder when dropped into a TFL install, else `Myth_TFL` beside the executable, else TFL
/// installed in a Steam Proton prefix, else `Myth_TFL` in the current folder.
pub fn tfl_dir() -> PathBuf {
    if let Some(dir) = env_or(std::env::var_os("TFL_DATA_DIR"), from_settings().1) {
        return dir;
    }
    if let Some(exe_dir) = portable_exe_dir() {
        if has_tfl_tags(&exe_dir) {
            return exe_dir;
        }
        let beside_tfl = exe_dir.join("Myth_TFL");
        if beside_tfl.join("tags").is_dir() {
            return beside_tfl;
        }
    }
    static PROTON_TFL: OnceLock<Option<PathBuf>> = OnceLock::new();
    PROTON_TFL
        .get_or_init(|| proton_install("Myth_TFL"))
        .clone()
        .unwrap_or_else(|| PathBuf::from("Myth_TFL"))
}

/// `folder` under `Program Files (x86)` of a Steam Proton prefix that holds a `tags` folder.
fn proton_install(folder: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let compat = PathBuf::from(home).join(".local/share/Steam/steamapps/compatdata");
    let mut prefixes: Vec<PathBuf> = std::fs::read_dir(compat)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    prefixes.sort();
    prefixes
        .into_iter()
        .map(|p| p.join("pfx/drive_c/Program Files (x86)").join(folder))
        .find(|dir| dir.join("tags").is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_beats_the_settings_file() {
        let file = Some(PathBuf::from("/games/file"));
        assert_eq!(
            env_or(Some("/games/env".into()), file.clone()),
            Some(PathBuf::from("/games/env"))
        );
        assert_eq!(env_or(None, file.clone()), file);
        assert_eq!(env_or(None, None), None);
    }

    #[test]
    fn data_beside_the_executable_when_the_current_folder_has_none() {
        let dir = std::env::temp_dir().join(format!("m2-data-dirs-{}", std::process::id()));
        let data = dir.join("data");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            beside_exe_fallback(false, Some(data.clone())),
            PathBuf::new()
        );
        std::fs::create_dir_all(data.join("tags")).unwrap();
        assert_eq!(beside_exe_fallback(false, Some(data.clone())), data);
        assert_eq!(
            beside_exe_fallback(true, Some(data.clone())),
            PathBuf::new()
        );
        assert_eq!(beside_exe_fallback(false, None), PathBuf::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
