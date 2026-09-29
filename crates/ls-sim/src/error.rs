//! Error type for loading data and setting up a simulation.

use thiserror::Error;

/// Errors from loading weapon / rule set data or configuring a [`crate::World`].
#[derive(Debug, Error)]
pub enum SimError {
    #[error("RON parse error in {source_name}: {message}")]
    Parse {
        source_name: String,
        message: String,
    },
    #[error("I/O error reading {path}: {message}")]
    Io { path: String, message: String },
    #[error("invalid weapon `{id}`: {reason}")]
    InvalidWeapon { id: String, reason: String },
    #[error("duplicate weapon id `{0}`")]
    DuplicateWeapon(String),
    #[error("too many weapons ({0}); at most 255 are supported")]
    TooManyWeapons(usize),
    #[error("unknown weapon `{0}`")]
    UnknownWeapon(String),
    #[error("invalid rule set: {0}")]
    InvalidRuleSet(String),
    #[error("ship type {0:?} is not allowed by the rule set")]
    ShipTypeNotAllowed(crate::ShipType),
    #[error("ship type {0:?} is not implemented yet")]
    ShipTypeUnavailable(crate::ShipType),
    #[error("invalid team {0}")]
    InvalidTeam(u8),
    #[error("weapon `{0}` is banned by the rule set")]
    WeaponBanned(String),
    #[error("weapon `{0}` cannot be picked up (availability or slots)")]
    WeaponNotCollectable(String),
    #[error("unknown player {0:?}")]
    UnknownPlayer(crate::PlayerId),
    #[error("too many players")]
    TooManyPlayers,
    #[error("invalid gadget `{id}`: {reason}")]
    InvalidGadget { id: String, reason: String },
    #[error("unknown gadget `{0}`")]
    UnknownGadget(String),
    #[error("gadget `{0}` is banned by the rule set")]
    GadgetBanned(String),
    #[error("rule set code: {0}")]
    RuleSetCode(String),
}
