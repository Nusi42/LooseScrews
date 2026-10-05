//! Gossip wall (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! A face grows on the rock nearest the centroid of the living ships; for 15 s every ship's
//! position is revealed to everyone (`reveal`), cloaked ships too.
//!
//! Numbers: `data/events/gossip_wall.ron`. Wave A stub (announce only); B2 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Gossip wall.
#[derive(Debug)]
pub struct GossipWall;

impl EventModule for GossipWall {
    fn kind(&self) -> EventKind {
        EventKind::GossipWall
    }
}
