//! Disco inferno (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 20 s the sun and ambient light drop to 8 %, a disco ball descends over the players'
//! centroid, every ship leaves a light trail. No other sim rule.
//!
//! Numbers: `data/events/disco_inferno.ron`. Wave A stub (announce only); B3 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Disco inferno.
#[derive(Debug)]
pub struct DiscoInferno;

impl EventModule for DiscoInferno {
    fn kind(&self) -> EventKind {
        EventKind::DiscoInferno
    }
}
