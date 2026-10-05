//! Tired announcer (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 30 s every kill plays a 3 s kill-cam for everyone, narrated by the announcer.
//!
//! Numbers: `data/events/tired_announcer.ron`. Wave A stub (announce only); B3 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Tired announcer.
#[derive(Debug)]
pub struct TiredAnnouncer;

impl EventModule for TiredAnnouncer {
    fn kind(&self) -> EventKind {
        EventKind::TiredAnnouncer
    }
}
