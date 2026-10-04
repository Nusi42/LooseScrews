//! Last Color Standing: a monochrome level; kills drain the victim's
//! colour; the last player with any colour wins (free-for-all).
//!
//! Settings: `ruleset::LastColorStandingRules`. Design: `docs/milestone-3-plan.md` §3.2.
//!
//! Status: stub. The generic fallback of `modes/mod.rs` runs the match (kills score,
//! lives where the mode has them, timer); the mode's own rules arrive in M3 wave B.

use crate::mode::ModeKind;
use crate::modes::ModeModule;

/// The LastColorStanding mode module.
#[derive(Debug)]
pub struct LastColorStanding;

impl ModeModule for LastColorStanding {
    fn kind(&self) -> ModeKind {
        ModeKind::LastColorStanding
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }
}
