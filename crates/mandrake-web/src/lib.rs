//! Myth II in the browser (WebAssembly): the game itself (`client`) and the
//! headless film replay with its browser bench.

#[cfg(target_arch = "wasm32")]
mod client;
#[cfg(target_arch = "wasm32")]
mod film_bench;
/// A film replayed headless from bytes, natively and in wasm.
pub mod film_replay;
