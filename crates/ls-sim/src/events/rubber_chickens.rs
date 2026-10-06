//! Rubber chickens (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 12 s every new projectile is flagged `CHICKEN` (`projectile_flags`;
//! sub-projectiles too); speed, damage and craters unchanged. Projectiles
//! keep the flag for their whole flight (the client draws them as rubber
//! chickens scaled to the shot; `ls_net::NetProjectile::chicken`).
//!
//! Numbers: `data/events/rubber_chickens.ron`.

use crate::events::{EffectCtx, EventKind, EventModule};
use crate::world::proj_flags;

/// Rubber chickens.
#[derive(Debug)]
pub struct RubberChickens;

impl EventModule for RubberChickens {
    fn kind(&self) -> EventKind {
        EventKind::RubberChickens
    }

    fn implemented(&self) -> bool {
        true
    }

    fn projectile_flags(&self, _fx: &EffectCtx) -> u16 {
        proj_flags::CHICKEN
    }
}
