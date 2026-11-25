//! Native OS folder picker for first-run game data setup (zero extra crate dependencies).
//!
//! When an interactive run starts and the expected game data folder cannot be found, prompts the
//! user to locate their Myth II (or TFL) folder, validates it, saves it into `mandrake.toml`, and
//! updates `args` so the launch continues seamlessly.

use crate::args::Args;
use mandrake_tag::tfl::Game;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Ensures game data exists for an interactive run. Returns `Ok(true)` to continue launching, or
/// `Ok(false)` if the user cancelled the folder picker.
pub(crate) fn ensure_game_data(args: &mut Args) -> anyhow::Result<bool> {
    if cfg!(test) || args.headless || args.screenshot.is_some() {
        return Ok(true);
    }

    let (game, tfl_dir) = crate::tfl_game::chosen_game(args);
    match game {
        Game::Myth2 => {
            if args.archive.is_file() {
                return Ok(true);
            }
            let prompt = "Myth II game data not found. Select your Myth II folder (the folder containing tags/):";
            loop {
                let Some(picked) = pick_folder(prompt) else {
                    return Ok(false);
                };
                let dir = normalize_myth2_dir(&picked);
                let archive = dir.join("tags").join("small install");
                if archive.is_file() {
                    save_game_dir("myth2_data", &dir);
                    mandrake_common::data_dirs::set_from_settings(Some(dir), None);
                    args.archive = archive;
                    return Ok(true);
                }
                show_notice(
                    "Mandrake — Game Data Not Found",
                    "That folder does not contain tags/small install.\nPlease select your Myth II: Soulblighter folder.",
                );
            }
        }
        Game::Tfl => {
            let dir = tfl_dir.unwrap_or_else(mandrake_common::data_dirs::tfl_dir);
            if Game::detect(&dir) == Some(Game::Tfl) {
                return Ok(true);
            }
            let prompt = "Myth: The Fallen Lords game data not found. Select your Myth_TFL folder (containing tags/):";
            loop {
                let Some(picked) = pick_folder(prompt) else {
                    return Ok(false);
                };
                let dir = normalize_tfl_dir(&picked);
                if Game::detect(&dir) == Some(Game::Tfl) {
                    save_game_dir("tfl_data", &dir);
                    mandrake_common::data_dirs::set_from_settings(None, Some(dir.clone()));
                    args.tfl_data = Some(dir);
                    return Ok(true);
                }
                show_notice(
                    "Mandrake — TFL Data Not Found",
                    "That folder does not contain Myth: The Fallen Lords tag archives (tags/*.gor).\nPlease select your Myth_TFL folder.",
                );
            }
        }
    }
}

/// If the user selected `Myth II/tags` directly instead of `Myth II`, walk up one level.
fn normalize_myth2_dir(picked: &Path) -> PathBuf {
    if picked.join("tags").join("small install").is_file() {
        return picked.to_path_buf();
    }
    if picked.join("small install").is_file()
        && let Some(parent) = picked.parent()
    {
        return parent.to_path_buf();
    }
    picked.to_path_buf()
}

/// If the user selected `Myth_TFL/tags` directly instead of `Myth_TFL`, walk up one level.
fn normalize_tfl_dir(picked: &Path) -> PathBuf {
    if Game::detect(picked) == Some(Game::Tfl) {
        return picked.to_path_buf();
    }
    if let Some(parent) = picked.parent()
        && Game::detect(parent) == Some(Game::Tfl)
    {
        return parent.to_path_buf();
    }
    picked.to_path_buf()
}

/// Persists `key = "<dir>"` into the active `mandrake.toml` under `[game]`.
fn save_game_dir(key: &str, dir: &Path) {
    let Some(conf_path) = crate::conf::in_use() else {
        return;
    };
    let value = crate::conf::Value::Str(dir.to_string_lossy().into_owned());
    if let Err(e) = crate::conf::set_values(&conf_path, &["game"], &[(key, value)]) {
        eprintln!(
            "settings: cannot save [game] {key} to {}: {e}",
            conf_path.display()
        );
    }
}

/// Opens the OS native folder selection dialog and returns the selected directory, or `None` if
/// cancelled or unavailable.
fn pick_folder(prompt: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let escaped = prompt.replace('\'', "''");
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             $d = New-Object System.Windows.Forms.FolderBrowserDialog; \
             $d.Description = '{escaped}'; \
             $d.UseDescriptionForTitle = $true; \
             if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ \
                 [Console]::Out.Write($d.SelectedPath) \
             }}"
        );
        let out = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-STA",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then(|| PathBuf::from(s))
    }

    #[cfg(target_os = "macos")]
    {
        let escaped = prompt.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("POSIX path of (choose folder with prompt \"{escaped}\")");
        let out = Command::new("osascript")
            .args(["-e", &script])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then(|| PathBuf::from(s))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(out) = Command::new("zenity")
            .args(["--file-selection", "--directory", "--title", prompt])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(PathBuf::from(s));
                }
            }
            return None;
        }
        if let Ok(out) = Command::new("kdialog")
            .args(["--getexistingdirectory", ".", "--title", prompt])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(PathBuf::from(s));
                }
            }
            return None;
        }
        eprintln!("{prompt}");
        None
    }
}

/// Shows a short native message box (best-effort; falls back to stderr).
fn show_notice(title: &str, message: &str) {
    #[cfg(target_os = "windows")]
    {
        let t = title.replace('\'', "''");
        let m = message.replace('\'', "''").replace('\n', "`n");
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; \
             [System.Windows.Forms.MessageBox]::Show('{m}', '{t}') | Out-Null"
        );
        let _ = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-STA",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .status();
    }

    #[cfg(target_os = "macos")]
    {
        let t = title.replace('\\', "\\\\").replace('"', "\\\"");
        let m = message.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display dialog \"{m}\" with title \"{t}\" buttons {{\"OK\"}} default button \"OK\""
        );
        let _ = Command::new("osascript").args(["-e", &script]).status();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if Command::new("zenity")
            .args(["--warning", "--title", title, "--text", message])
            .status()
            .is_err()
        {
            let _ = Command::new("kdialog")
                .args(["--sorry", message, "--title", title])
                .status();
        }
    }

    eprintln!("{title}: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_myth2_dir_accepts_parent_or_tags_subfolder() {
        let base = std::env::temp_dir().join(format!("mandrake-picker-{}", std::process::id()));
        let tags = base.join("tags");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(tags.join("small install"), b"").unwrap();

        assert_eq!(normalize_myth2_dir(&base), base);
        assert_eq!(normalize_myth2_dir(&tags), base);

        let _ = std::fs::remove_dir_all(&base);
    }
}
