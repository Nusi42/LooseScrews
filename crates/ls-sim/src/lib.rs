//! Deterministic game simulation for Loose Screws.
//!
//! See `docs/implementation-plan.md` section 4.3 and, for the M2 arsenal
//! framework, `docs/architecture/arsenal-framework.md`. Contents:
//! - [`ShipState`], [`InputCmd`] and the flight model ([`simulate_movement`]):
//!   per-axis thrust caps in ship space, damping, horizon assist, terrain collision.
//! - Resources (hull, energy, boost, scrap) coupled as in the design doc.
//! - Ship types ([`ShipTable`], `data/ships/*.ron`).
//! - Data-driven weapons ([`WeaponTable`], `data/weapons/*.ron`) with one
//!   behaviour module per weapon ([`weapons`]), gadgets ([`gadgets`],
//!   `data/gadgets/*.ron`) and rule sets ([`RuleSet`], `data/rulesets/*.ron`,
//!   rule set codes in [`rulecode`]).
//! - [`World`]: projectiles, entities (debris, decoys, ghosts, dust),
//!   splash, knockback, craters and paint, pickups, scrap, respawns and the
//!   game mode, advanced by [`World::step`].
//! - The comedy director ([`director`]) and its events ([`events`], one
//!   module per event behind [`events::EventModule`], `data/events/*.ron`);
//!   the replicated part is [`MoodView`] (`ModeView::mood`).
//! - Game modes ([`modes`], one module per mode behind [`modes::ModeModule`])
//!   and mutators ([`mutators`]); the replicated mode state is [`ModeView`]
//!   ([`World::mode_view`]), the results table [`MatchResults`].
//! - World physics (M4): gravity wells ([`gravity`]), the level's up
//!   ([`World::up`]), Instability's craters and cave-ins ([`instability`]),
//!   pigment on monochrome levels ([`pigment`]) and radiation on belt
//!   levels ([`radiation`]; the belt is `ls_math::level::BeltSite`);
//!   numbers in `data/world/*.ron`.
//! - [`World::state_hash`] for replay and desync checks, [`data::data_hash`]
//!   for the handshake.
//!
//! Terrain is accessed only through [`ls_math::terrain::TerrainQuery`] (plus the
//! [`SimTerrain`] extension for edits and paint coverage); this crate never
//! depends on `ls-worldgen`.
//!
//! This crate is deterministic: follow the rules in `CLAUDE.md`.

pub mod behavior;
pub mod data;
pub mod director;
pub mod entity;
pub mod error;
pub mod event_replays;
pub mod events;
pub mod gadgets;
pub mod golden;
pub mod gravity;
pub mod hash;
pub mod input;
pub mod instability;
pub mod mode;
pub mod modes;
pub mod mutators;
pub mod obstacle;
pub mod pigment;
pub mod radiation;
pub mod rulecode;
pub mod ruleset;
pub mod ship;
pub mod terrain;
pub mod tuning;
pub mod visual;
pub mod weapon;
pub mod weapons;
pub mod world;
pub mod world_replays;

pub use behavior::{gadget_cue, Command, Ctx, Cue, CueKind, DamageSource};
pub use data::{data_hash, with_behavior_version, GameData, SIM_BEHAVIOR_VERSION};
pub use director::{DirectorEvent, DirectorState, EventEffects, Mood, MoodView, RNG_STREAM_MOOD};
pub use entity::{Entity, EntityKind, EntitySpawn, EntityView, ReplicatedKind, Source};
pub use error::SimError;
pub use events::{EventDef, EventKind, EventModule, EventPhase, EventTable, Facing, MoveEffects};
pub use gadgets::{GadgetActivation, GadgetDef, GadgetId, GadgetTable};
pub use gravity::{DynamicWell, GravitySettings, WellField, WellSite};
pub use hash::StateHasher;
pub use input::{Buttons, InputCmd};
pub use mode::{
    MatchPhase, MatchResults, ModeKind, ModeState, ModeView, Objective, ObjectiveView, PlayerMode,
    PlayerModeView, ResultRow,
};
pub use modes::heist::HeistLevel;
pub use modes::MoveLimits;
pub use ruleset::{
    Availability, BeltUpRules, EventRule, HeistRules, HotReactorRules, LastColorStandingRules,
    Loadout, ModeField, ModeRules, Mutator, PaintTheVoidRules, PickupKind, PickupRule,
    PigmentPickupRule, ResolvedRules, RuleSet, ScrapPickupRule, TeamBrawlRules, TheSqueezeRules,
    WeaponTuning,
};
pub use ship::{
    simulate_movement, simulate_movement_env, ChargeState, MoveEnv, MovementReport, PlayerId,
    ShipState, WeaponState,
};
pub use terrain::{
    paint_team_id, team_from_paint_id, PaintCoverage, PaintedTerrain, ShellSphere, ShelledTerrain,
    SimTerrain, NO_PAINT,
};
pub use tuning::{ShipTable, ShipTuning, ShipType};
pub use visual::{BuiltinVisual, Shape, Visual, VisualId, VisualTable};
pub use weapon::{
    ChargeDef, ShieldInteraction, Splash, TerrainEffect, WeaponCost, WeaponDef, WeaponFamily,
    WeaponId, WeaponSlot, WeaponTable,
};
pub use world::{
    proj_flags, Event, MatchSetup, PickupSpawn, Projectile, ScrapPile, SpawnPoint, World,
    WorldConfig,
};

pub use ls_math;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_arsenal;
#[cfg(test)]
mod tests_world;
