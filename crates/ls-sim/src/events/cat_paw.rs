//! Cat paw (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! After the tell on the boundary wall nearest the densest group of ships, a paw (10 u pad)
//! sweeps 90 u along an arc in 2 s and withdraws: 30 damage and 25 u/s knockback to ships
//! touched, debris swept along, three claw grooves carved (about 36 `SubtractSphere` r 2.5
//! spread over the sweep), never in spawn zones or the Heist vein. Needs an open boundary face
//! with ships within 60 u.
//!
//! Numbers: `data/events/cat_paw.ron`. Wave A stub (announce only); B3 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Cat paw.
#[derive(Debug)]
pub struct CatPaw;

impl EventModule for CatPaw {
    fn kind(&self) -> EventKind {
        EventKind::CatPaw
    }
}
