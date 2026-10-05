//! Gravity coffee break (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! Every gravity well switches off for 20 s (fades over 1 s; `well_scale`).
//!
//! Numbers: `data/events/gravity_coffee_break.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule, EventRef};

/// Gravity coffee break.
#[derive(Debug)]
pub struct GravityCoffeeBreak;

impl EventModule for GravityCoffeeBreak {
    fn kind(&self) -> EventKind {
        EventKind::GravityCoffeeBreak
    }

    /// Needs gravity wells. Until the level's wells reach `MatchSetup`
    /// (A1/A4) the stub is never eligible; B1 checks the wells.
    fn eligible(&self, _e: &EventRef) -> bool {
        false
    }
}
