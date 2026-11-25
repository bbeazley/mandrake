#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn game_dir() -> PathBuf {
    mandrake_common::data_dirs::myth2_dir()
}

pub fn game_dir_string() -> String {
    game_dir().to_string_lossy().into_owned()
}

pub fn game_path(rel: &str) -> String {
    game_dir().join(rel).to_string_lossy().into_owned()
}

pub fn tfl_dir() -> PathBuf {
    mandrake_common::data_dirs::tfl_dir()
}

pub fn skip_allowed() -> bool {
    std::env::var_os("M2_GATE_NO_FILMS").is_some_and(|v| v == "1")
}

#[track_caller]
pub fn require(found: bool, what: &str) -> bool {
    if found {
        return true;
    }
    let at = std::panic::Location::caller();
    if skip_allowed() {
        eprintln!("Notice: {what} not found, skipping (M2_GATE_NO_FILMS=1; {at})");
        false
    } else {
        panic!(
            "test data missing: {what} (MYTH2_DATA_DIR={}, TFL_DATA_DIR={}). Install it, fix the \
             variable, or set M2_GATE_NO_FILMS=1 to skip the tests that need it.",
            game_dir().display(),
            tfl_dir().display()
        )
    }
}

#[track_caller]
pub fn require_data(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    require(path.exists(), &path.display().to_string())
}

#[track_caller]
pub fn require_game() -> Option<PathBuf> {
    let dir = game_dir();
    require_data(dir.join("tags/small install")).then_some(dir)
}

#[track_caller]
pub fn require_game_path(rel: &str) -> Option<PathBuf> {
    let path = game_dir().join(rel);
    require_data(&path).then_some(path)
}

#[track_caller]
pub fn require_tfl() -> Option<PathBuf> {
    let dir = tfl_dir();
    require_data(dir.join("tags")).then_some(dir)
}
