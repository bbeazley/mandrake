//! The game in the browser: the page hands in the files of the player's Myth II
//! folder, then starts the game (`mandrake`) with a command line, as the desktop is started. The
//! game's own code runs; only the platform under it is the browser's.
//!
//! `scripts/web/client.html` is the page: the folder picker, the copy kept in the browser's
//! storage (OPFS), and the canvas `#myth2` the game draws into.

use mandrake_common::MemoryVfs;
use std::sync::OnceLock;
use wasm_bindgen::prelude::*;

/// The game folder's files, as the page handed them in; the process's game VFS once the first is
/// in ([`mandrake_common::set_game_vfs`]).
fn game_files() -> &'static MemoryVfs {
    static FILES: OnceLock<MemoryVfs> = OnceLock::new();
    FILES.get_or_init(|| {
        let files = MemoryVfs::new();
        // Only fails if the game VFS was set already, and only this sets it.
        let _ = mandrake_common::set_game_vfs(Box::new(files.clone()));
        files
    })
}

/// One file of the game folder, at `path` (as the command line will name it, e.g.
/// `/game/tags/small install`).
#[wasm_bindgen]
pub fn add_game_file(path: String, bytes: Vec<u8>) {
    game_files().insert(path, bytes);
}

/// The film's plugin names, for the page to choose the files to load.
#[wasm_bindgen]
pub fn film_plugin_names(film: &[u8]) -> Result<Vec<String>, JsValue> {
    crate::film_replay::film_plugins(film).map_err(|e| JsValue::from_str(&e))
}

/// The engine's tag set for a film (`tags/NAME`, `plugins/NAME` in search order): `plugin_heads`
/// holds each plugin's first 128 bytes, back to back, in the order of `plugin_names`;
/// `metaserver_preference` the folder's `preferences/metaserver plugin`, if any.
#[wasm_bindgen]
pub fn engine_tag_set(
    tags: Vec<String>,
    plugin_names: Vec<String>,
    plugin_heads: &[u8],
    film_plugins: Vec<String>,
    metaserver_preference: Option<Vec<u8>>,
) -> Result<Vec<String>, JsValue> {
    let plugins = crate::film_replay::split_plugin_heads(plugin_names, plugin_heads)
        .map_err(|e| JsValue::from_str(&e))?;
    let metaserver = mandrake_tag::tagset::metaserver_named(metaserver_preference.as_deref());
    crate::film_replay::order_tag_files(&tags, &plugins, &film_plugins, &metaserver)
        .map_err(|e| JsValue::from_str(&e))
}

/// The page's audio output rate (`AudioContext.sampleRate`): call before [`start_game`] for sound.
#[wasm_bindgen]
pub fn set_audio_rate(rate: u32) {
    mandrake_client::web::set_audio_rate(rate);
}

/// Fills `out` with the game's next interleaved stereo frames (silence before the game has opened
/// its output), for the page's audio worklet. Returns whether the game's mixer filled them.
#[wasm_bindgen]
pub fn mix_audio(out: &mut [f32]) -> bool {
    mandrake_client::web::mix_audio(out)
}

/// Starts the game with `words`, the command line after the program's name (`--archive`,
/// `--replay`, ...). It returns once the game runs in the page's event loop; an error is one before
/// that (a bad command line, a file that won't load).
#[wasm_bindgen]
pub fn start_game(words: Vec<String>) -> Result<(), JsValue> {
    game_files();
    // TFL films kept in localStorage are exposed alongside the game files for Saved Films.
    mandrake_client::web::set_game_files(|path, bytes| match bytes {
        Some(bytes) => game_files().insert(path, bytes),
        None => {
            game_files().remove(path);
        }
    });
    mandrake_client::web::start(words).map_err(|e| JsValue::from_str(&e))
}

/// The `[net]` table's player name of the settings file `conf` (or the default file): empty if
/// none is saved.
#[wasm_bindgen]
pub fn net_player_name(conf: Option<String>) -> String {
    mandrake_client::web::net_player(conf.as_deref()).0
}

/// The `[net]` table's colours of the settings file `conf`: `[r, g, b, flags]` primary, then
/// secondary, 16 bits each.
#[wasm_bindgen]
pub fn net_player_colors(conf: Option<String>) -> Vec<u16> {
    mandrake_client::web::net_player(conf.as_deref()).1.to_vec()
}

type SaveNetPlayer = fn(&str, [u16; 8], Option<&str>) -> Result<(), String>;

fn set_player_with(
    name: &str,
    colors: Vec<u16>,
    conf: Option<&str>,
    save: SaveNetPlayer,
) -> Result<(), JsValue> {
    let colors: [u16; 8] = colors
        .try_into()
        .map_err(|_| JsValue::from_str("eight colour numbers"))?;
    save(name, colors, conf).map_err(|e| JsValue::from_str(&e))
}

/// Writes the landing page's name and colours into the `[net]` table of `conf`, before
/// [`start_game`].
#[wasm_bindgen]
pub fn set_net_player(name: String, colors: Vec<u16>, conf: Option<String>) -> Result<(), JsValue> {
    set_player_with(
        &name,
        colors,
        conf.as_deref(),
        mandrake_client::web::set_net_player,
    )
}

/// Writes the landing page's name and colours into TFL's `[net.tfl]` table of `conf`.
#[wasm_bindgen]
pub fn set_tfl_net_player(
    name: String,
    colors: Vec<u16>,
    conf: Option<String>,
) -> Result<(), JsValue> {
    set_player_with(
        &name,
        colors,
        conf.as_deref(),
        mandrake_client::web::set_tfl_net_player,
    )
}

/// TFL's `[net.tfl]` player name of `conf`, as [`net_player_name`].
#[wasm_bindgen]
pub fn tfl_net_player_name(conf: Option<String>) -> String {
    mandrake_client::web::tfl_net_player(conf.as_deref()).0
}

/// TFL's `[net.tfl]` colours of `conf`, as [`net_player_colors`].
#[wasm_bindgen]
pub fn tfl_net_player_colors(conf: Option<String>) -> Vec<u16> {
    mandrake_client::web::tfl_net_player(conf.as_deref())
        .1
        .to_vec()
}

/// Installs a preset's settings file `shipped` as `name` (used via `--conf name`), preserving the
/// saved `[net]` table.
#[wasm_bindgen]
pub fn install_conf(name: String, shipped: String) -> Result<(), JsValue> {
    mandrake_client::web::install_conf(&name, &shipped).map_err(|e| JsValue::from_str(&e))
}

/// The version the landing page shows: package version, commit and date (`build.rs`),
/// e.g. `0.1.0 (03bc8ee3, 2026-11-15)`.
#[wasm_bindgen]
pub fn client_version() -> String {
    let (version, revision, date) = (
        env!("CARGO_PKG_VERSION"),
        env!("M2_WEB_REVISION"),
        env!("M2_WEB_REVISION_DATE"),
    );
    if date.is_empty() {
        format!("{version} ({revision})")
    } else {
        format!("{version} ({revision}, {date})")
    }
}
