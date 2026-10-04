//! The Squeeze: free-for-all, one life; walls grow inward from the level
//! boundary in phases (warning, then closing), crushing ships caught in them; last ship
//! flying wins.
//!
//! Settings: `ruleset::TheSqueezeRules`. Design: `docs/milestone-3-plan.md` §3.2.
//!
//! Status: stub. The generic fallback of `modes/mod.rs` runs the match (kills score,
//! lives where the mode has them, timer); the mode's own rules arrive in M3 wave B.

use crate::mode::ModeKind;
use crate::modes::ModeModule;

/// The Squeeze mode module.
#[derive(Debug)]
pub struct TheSqueeze;

impl ModeModule for TheSqueeze {
    fn kind(&self) -> ModeKind {
        ModeKind::TheSqueeze
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }
}
