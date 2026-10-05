//! Duck parade (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! Five giant rubber ducks (3.5 u) drift in a convoy along a network route at 5 u/s (45 s max),
//! ignore rock, absorb projectiles and push ships aside (no damage). Positions are a function
//! of the start tick and the seed.
//!
//! Numbers: `data/events/duck_parade.ron`. Wave A stub (announce only); B3 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Duck parade.
#[derive(Debug)]
pub struct DuckParade;

impl EventModule for DuckParade {
    fn kind(&self) -> EventKind {
        EventKind::DuckParade
    }
}
