//! Team Brawl: team deathmatch with respawns; 2 to 4 teams, or free-for-all
//! deathmatch with `teams: 0` (the duel mode). First side to the kill target wins;
//! on time the most kills win (draw if equal).
//!
//! Settings: `ruleset::TeamBrawlRules`. Design: `docs/milestone-3-plan.md` §3.2.
//!
//! Status: stub. The generic fallback of `modes/mod.rs` runs the match (kills score,
//! lives where the mode has them, timer); the mode's own rules arrive in M3 wave B.

use crate::mode::ModeKind;
use crate::modes::ModeModule;

/// The TeamBrawl mode module.
#[derive(Debug)]
pub struct TeamBrawl;

impl ModeModule for TeamBrawl {
    fn kind(&self) -> ModeKind {
        ModeKind::TeamBrawl
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0, 2, 3, 4]
    }
}
