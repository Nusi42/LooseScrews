//! Bored level (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! The level's "up" turns 90 degrees about a seeded horizontal axis over 5 s and stays turned
//! (`World::up`; terrain keeps its coordinates).
//!
//! Numbers: `data/events/bored_level.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Bored level.
#[derive(Debug)]
pub struct BoredLevel;

impl EventModule for BoredLevel {
    fn kind(&self) -> EventKind {
        EventKind::BoredLevel
    }
}
