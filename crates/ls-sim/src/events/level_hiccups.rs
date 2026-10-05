//! Level hiccups (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 15 s, every 3 s (plus or minus 1, seeded) ships, debris and unstuck mines get an impulse
//! along `up` that moves them about 2 u over 0.25 s; projectiles in flight are nudged by 2 u.
//!
//! Numbers: `data/events/level_hiccups.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Level hiccups.
#[derive(Debug)]
pub struct LevelHiccups;

impl EventModule for LevelHiccups {
    fn kind(&self) -> EventKind {
        EventKind::LevelHiccups
    }
}
