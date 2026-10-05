//! Moving day (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! Every pickup not held moves to a new permutation of the pickup points over 2 s (untouchable
//! meanwhile: `allows_pickup`); spawn timers kept. Needs at least 4 pickups in play.
//!
//! Numbers: `data/events/moving_day.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule, EventRef};

/// Moving day.
#[derive(Debug)]
pub struct MovingDay;

impl EventModule for MovingDay {
    fn kind(&self) -> EventKind {
        EventKind::MovingDay
    }

    /// Needs at least `min_pickups` (4) pickups in play.
    fn eligible(&self, e: &EventRef) -> bool {
        crate::events::pickups_in_play(e.view) as f32 >= e.def.param("min_pickups", 4.0)
    }
}
