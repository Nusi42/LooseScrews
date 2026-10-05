//! Deterministic math for Loose Screws.
//!
//! Everything in this crate produces bit-identical results on x86_64 and
//! wasm32. Only IEEE-exact float operations (`+ - * /`, `sqrt`, comparisons)
//! are used directly; transcendental functions go through [`det`], which is
//! backed by the pure-Rust `libm` crate. `mul_add` is never used.
//!
//! Contents:
//! - [`Vec3`], [`Quat`], [`Mat3`], [`Aabb`]: geometry primitives.
//! - [`Tick`], [`DT`], [`TICK_HZ`]: fixed-step simulation time.
//! - [`det`]: deterministic transcendental functions.
//! - [`rng`]: [`SplitMix64`] and [`Pcg32`] random number generators.
//! - [`level`]: level contracts (archetypes, gravity well sites, the
//!   asteroid belt).
//! - [`terrain`]: the shared terrain contracts (`TerrainEdit`, `RayHit`,
//!   `TerrainQuery`) plus simple test terrains.

pub mod aabb;
pub mod det;
pub mod level;
pub mod mat3;
pub mod quat;
pub mod rng;
pub mod terrain;
pub mod tick;
pub mod vec3;

pub use aabb::Aabb;
pub use mat3::Mat3;
pub use quat::Quat;
pub use rng::{Pcg32, SplitMix64};
pub use terrain::{RayHit, SurfaceStates, TerrainEdit, TerrainQuery};
pub use tick::{Tick, DT, DT_F64, TICK_HZ};
pub use vec3::Vec3;
