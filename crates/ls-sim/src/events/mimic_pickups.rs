//! Mimic pickups (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! 3 + players / 4 (max 6) weapon or upgrade pickups become mimics for 20 s: they hop toward
//! the nearest ship within 30 u at 12 u/s and bite (12 damage, 1 s apart); 25 hp; a killed
//! mimic drops its content. Needs at least 4 pickups in play.
//!
//! Numbers: `data/events/mimic_pickups.ron`. Wave A stub (announce only); B2 builds the
//! rules.

use crate::events::{EventKind, EventModule, EventRef};

/// Mimic pickups.
#[derive(Debug)]
pub struct MimicPickups;

impl EventModule for MimicPickups {
    fn kind(&self) -> EventKind {
        EventKind::MimicPickups
    }

    /// Needs at least `min_pickups` (4) pickups in play.
    fn eligible(&self, e: &EventRef) -> bool {
        crate::events::pickups_in_play(e.view) as f32 >= e.def.param("min_pickups", 4.0)
    }
}
