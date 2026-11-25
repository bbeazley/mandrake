//! Myth II deterministic fixed-point math, 3D vector arithmetic,
//! 512-step binary angle (`Angle512`), Bungie PRNG, and the engine's 1024-step tables,
//! `isqrt`, `distance2d` and `arctangent` (`engine`).
//!
//! This crate is `#![no_std]` capable and platform-agnostic, ensuring 100%
//! bit-for-bit repeatability across x86, ARM, WebAssembly, and other targets.

#![no_std]

pub mod engine;
pub mod fixed;
pub mod rng;
pub mod trig;
pub mod vector;

pub use fixed::Fixed16;
pub use rng::BungieRng;
pub use trig::Angle512;
pub use vector::Vec3Fixed;
