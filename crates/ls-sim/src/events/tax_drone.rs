//! Tax drone (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! A drone enters from the nearest boundary cavern and chases the leader at 22 u/s along the
//! network path; each touch takes 10 % of the leader's energy (2 s apart, at most 5); 40 hp;
//! shot down it bursts into a scrap pile of 2 scrap per energy taken (max 20).
//!
//! Numbers: `data/events/tax_drone.ron`. Wave A stub (announce only); B2 builds the
//! rules.

use crate::events::{EventCtx, EventKind, EventModule, EventRef};

/// Tax drone.
#[derive(Debug)]
pub struct TaxDrone;

impl EventModule for TaxDrone {
    fn kind(&self) -> EventKind {
        EventKind::TaxDrone
    }

    /// Needs a clear leader (`ModeModule::leader`).
    fn eligible(&self, e: &EventRef) -> bool {
        e.leader.is_some()
    }

    /// Names the leader.
    fn on_announce(&self, e: &mut EventCtx) {
        e.event.target = e.leader;
    }
}
