//! Big head day (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 15 s ship hit spheres against projectiles are 3x larger (`hit_radius_scale`); terrain
//! collision, ramming and pickups unchanged.
//!
//! Numbers: `data/events/big_head_day.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Big head day.
#[derive(Debug)]
pub struct BigHeadDay;

impl EventModule for BigHeadDay {
    fn kind(&self) -> EventKind {
        EventKind::BigHeadDay
    }
}
