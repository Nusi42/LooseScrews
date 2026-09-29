//! Seeded SDF world generation for Loose Screws.
//!
//! - [`Sliders`] and [`ShareCode`]: the seven world sliders, and the short
//!   code (`LS…`) that rebuilds the same world everywhere.
//! - [`Generator`]: the deterministic signed distance field of a freshly
//!   generated level (Enclosure and Density active in M1), and
//!   [`RegionSampler`], which samples it much faster over a region with the
//!   same bits. Every level carries a [`Network`] of caverns and tunnels
//!   that keeps both team ends connected; [`ClearanceGrid`] checks that
//!   connectivity (flood fill over a coarse clearance lattice).
//! - [`ChunkStore`]: 32³-cell chunks, generated lazily, with the terrain edit
//!   log applied on top and per-cell surface layers (paint and timed surface
//!   states: hot, charged, glow). Implements
//!   [`ls_math::terrain::TerrainQuery`] for physics (`sdf`, `normal`,
//!   `raycast`, `sphere_overlap`).
//! - [`surface_nets`] and [`dual_contouring`]: chunk meshers, seam-free
//!   across chunk borders.
//! - [`golden`]: the golden-hash procedure used to check that native and WASM
//!   builds produce bit-identical worlds.
//!
//! This crate is deterministic: only IEEE-exact float operations are used
//! directly, transcendental functions go through [`ls_math::det`], nothing
//! iterates a hash map, and there is no time, threading or OS randomness.
//! Changing generator output requires bumping [`GENERATOR_VERSION`] and
//! re-blessing the golden hashes (`tests/fixtures/golden_hashes.txt`).

pub mod chunk;
pub mod connectivity;
pub mod generator;
pub mod golden;
pub mod grid;
pub mod material;
pub mod mesh;
pub mod network;
pub mod noise;
pub mod sharecode;
pub mod sliders;

pub use chunk::{ChunkCoord, ChunkStore, PaintCoverage};
pub use connectivity::ClearanceGrid;
pub use generator::{Generator, RegionSampler, DEFAULT_SIZE_CHUNKS, WALL_THICKNESS};
pub use grid::SampleGrid;
pub use material::{strengthen_edit, Material, MaterialField, MaterialRegion};
pub use mesh::{dual_contouring, dual_contouring_with_sdf, surface_nets, Mesh};
pub use network::Network;
pub use sharecode::ShareCode;
pub use sliders::Sliders;

pub use ls_math;
pub use ls_math::terrain::SurfaceStates;

/// Version of the generator algorithm. Stored in every share code; bump it
/// whenever [`Generator::sample`] output changes.
pub const GENERATOR_VERSION: u8 = 2;

/// Cells per chunk along each axis (1 cell = 1 u).
pub const CHUNK_CELLS: i32 = 32;

/// Paint team value meaning "unpainted" (also erases paint when used in a
/// `TerrainEdit::Paint`).
pub const NO_TEAM: u8 = u8::MAX;

/// Errors from share-code parsing and generator setup.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorldgenError {
    #[error("share code must start with \"LS\"")]
    MissingPrefix,
    #[error("share code must have {expected} symbols after the prefix, found {found}")]
    InvalidLength { expected: usize, found: usize },
    #[error("invalid character {0:?} in share code")]
    InvalidCharacter(char),
    #[error("share code checksum mismatch (typo?)")]
    ChecksumMismatch,
    #[error("slider {name} is {value}, must be 0..=100")]
    SliderOutOfRange { name: &'static str, value: u8 },
    #[error(
        "share code was made by generator version {0}, this build has version {GENERATOR_VERSION}"
    )]
    UnsupportedVersion(u8),
}
