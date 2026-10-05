//! Everything is cake (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 20 s every crater has 1.44x the radius (`crater_scale`); crater debris is cake:
//! scavenging it repairs twice its scrap in hull (`scavenge_scale`).
//!
//! Numbers: `data/events/everything_is_cake.ron`. Wave A stub (announce only); B1 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Everything is cake.
#[derive(Debug)]
pub struct EverythingIsCake;

impl EventModule for EverythingIsCake {
    fn kind(&self) -> EventKind {
        EventKind::EverythingIsCake
    }
}
