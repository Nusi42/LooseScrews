//! Wrong genre (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 10 s every ship is locked to the plane through its position perpendicular to z
//! (`move_effects` plane lock), facing snaps to +x or -x, new projectiles fly in the plane,
//! ship hits ignore depth.
//!
//! Numbers: `data/events/wrong_genre.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Wrong genre.
#[derive(Debug)]
pub struct WrongGenre;

impl EventModule for WrongGenre {
    fn kind(&self) -> EventKind {
        EventKind::WrongGenre
    }
}
