//! Heist: mine the core out of its vein deep in the level and carry it to an
//! extraction zone; the carrier cannot boost. 2 teams, or free-for-all for few players.
//!
//! Settings: `ruleset::HeistRules`. Design: `docs/milestone-3-plan.md` §3.2.
//!
//! Status: stub. The generic fallback of `modes/mod.rs` runs the match (kills score,
//! lives where the mode has them, timer); the mode's own rules arrive in M3 wave B.

use crate::mode::ModeKind;
use crate::modes::ModeModule;

/// The Heist mode module.
#[derive(Debug)]
pub struct Heist;

impl ModeModule for Heist {
    fn kind(&self) -> ModeKind {
        ModeKind::Heist
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0, 2]
    }
}
