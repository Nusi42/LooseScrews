//! Rubber chickens (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 12 s every new projectile is flagged `CHICKEN` (`projectile_flags`); speed, damage and
//! craters unchanged.
//!
//! Numbers: `data/events/rubber_chickens.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Rubber chickens.
#[derive(Debug)]
pub struct RubberChickens;

impl EventModule for RubberChickens {
    fn kind(&self) -> EventKind {
        EventKind::RubberChickens
    }
}
