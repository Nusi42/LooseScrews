//! Hot Reactor: pass the reactor around; whoever holds it at detonation
//! loses a life; the fuse runs faster the more ships are near it; last player with lives
//! wins.
//!
//! Settings: `ruleset::HotReactorRules`. Design: `docs/milestone-3-plan.md` §3.2.
//!
//! Status: stub. The generic fallback of `modes/mod.rs` runs the match (kills score,
//! lives where the mode has them, timer); the mode's own rules arrive in M3 wave B.

use crate::mode::ModeKind;
use crate::modes::ModeModule;

/// The HotReactor mode module.
#[derive(Debug)]
pub struct HotReactor;

impl ModeModule for HotReactor {
    fn kind(&self) -> ModeKind {
        ModeKind::HotReactor
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }
}
