//! The browser side of [`crate::film_replay`]: `scripts/web/film-bench.html`
//! feeds it the film and the archives and reads back the summary.

use wasm_bindgen::prelude::*;

use crate::film_replay;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance)]
    fn now() -> f64;
}

/// The film's plugin names.
#[wasm_bindgen]
pub fn bench_film_plugins(film: &[u8]) -> Result<Vec<String>, JsValue> {
    film_replay::film_plugins(film).map_err(|e| JsValue::from_str(&e))
}

/// The archives to load, in search order (`tags/NAME`, `plugins/NAME`). `plugin_heads` holds each
/// plugin's first 128 bytes, back to back, in the order of `plugin_names`.
#[wasm_bindgen]
pub fn bench_order_tag_files(
    tags: Vec<String>,
    plugin_names: Vec<String>,
    plugin_heads: &[u8],
    film_plugins: Vec<String>,
) -> Result<Vec<String>, JsValue> {
    let plugins = film_replay::split_plugin_heads(plugin_names, plugin_heads)
        .map_err(|e| JsValue::from_str(&e))?;
    // The default metaserver: the bench's server serves no preferences.
    let metaserver = mandrake_tag::tagset::metaserver_named(None);
    film_replay::order_tag_files(&tags, &plugins, &film_plugins, &metaserver)
        .map_err(|e| JsValue::from_str(&e))
}

/// Archives handed in one at a time, so JS can drop each copy as it goes.
#[wasm_bindgen]
#[derive(Default)]
pub struct FilmBench {
    archives: Vec<Vec<u8>>,
    tick_ms: Vec<f64>,
}

#[wasm_bindgen]
impl FilmBench {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_archive(&mut self, bytes: Vec<u8>) {
        self.archives.push(bytes);
    }

    /// Replays the film; the summary line of [`film_replay::ReplayReport::summary`].
    pub fn run(&mut self, film: &[u8]) -> Result<String, JsValue> {
        let archives = std::mem::take(&mut self.archives);
        let r = film_replay::replay(film, archives, &now).map_err(|e| JsValue::from_str(&e))?;
        let summary = r.summary();
        self.tick_ms = r.tick_ms;
        Ok(summary)
    }

    /// The last run's tick times in milliseconds.
    pub fn tick_ms(&self) -> Vec<f64> {
        self.tick_ms.clone()
    }
}

/// The wasm memory's size in bytes. It only grows, so after a run it is the peak.
#[wasm_bindgen]
pub fn bench_memory_bytes() -> f64 {
    (core::arch::wasm32::memory_size(0) * 65536) as f64
}
