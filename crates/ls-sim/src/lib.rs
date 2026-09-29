//! Deterministic game simulation for Loose Screws.
//!
//! See `docs/implementation-plan.md` section 4.3. Contents:
//! - [`ShipState`], [`InputCmd`] and the flight model ([`simulate_movement`]):
//!   per-axis thrust caps in ship space, damping, horizon assist, terrain collision.
//! - Resources (hull, energy, boost, scrap) coupled as in the design doc.
//! - Data-driven weapons ([`WeaponTable`], `data/weapons/*.ron`) and rule sets
//!   ([`RuleSet`], `data/rulesets/*.ron`).
//! - [`World`]: projectiles, splash, knockback, craters and paint, pickups,
//!   scrap, respawns and the Paint the Void mode, advanced by [`World::step`].
//! - [`World::state_hash`] for replay and desync checks.
//!
//! Terrain is accessed only through [`ls_math::terrain::TerrainQuery`] (plus the
//! [`SimTerrain`] extension for edits and paint coverage); this crate never
//! depends on `ls-worldgen`.
//!
//! This crate is deterministic: follow the rules in `CLAUDE.md`.

pub mod error;
pub mod hash;
pub mod input;
pub mod mode;
pub mod ruleset;
pub mod ship;
pub mod terrain;
pub mod tuning;
pub mod weapon;
pub mod world;

pub use error::SimError;
pub use hash::StateHasher;
pub use input::{Buttons, InputCmd};
pub use mode::{MatchPhase, PaintTheVoid};
pub use ruleset::{
    Availability, Loadout, ModeRules, PaintTheVoidRules, PickupKind, PickupRule, ResolvedRules,
    RuleSet, ScrapPickupRule,
};
pub use ship::{simulate_movement, MovementReport, PlayerId, ShipState, WeaponState};
pub use terrain::{
    paint_team_id, team_from_paint_id, PaintCoverage, PaintedTerrain, SimTerrain, NO_PAINT,
};
pub use tuning::{ShipTuning, ShipType};
pub use weapon::{
    ShieldInteraction, Splash, TerrainEffect, WeaponCost, WeaponDef, WeaponFamily, WeaponId,
    WeaponSlot, WeaponTable,
};
pub use world::{
    Event, MatchSetup, PickupSpawn, Projectile, ScrapPile, SpawnPoint, World, WorldConfig,
};

pub use ls_math;

#[cfg(test)]
mod tests;
