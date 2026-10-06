//! Big head day (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 15 s ship hit spheres against projectiles are `hit_scale` (3x)
//! larger (`hit_radius_scale`); terrain collision, ramming, splash and
//! pickups unchanged (tunnels still fit). The server's lag compensation
//! tests a rewound target with the scale of the rewound tick
//! (`ls_game::server::lagcomp`).
//!
//! Numbers: `data/events/big_head_day.ron`.

use crate::events::{EffectCtx, EventKind, EventModule};

/// Big head day.
#[derive(Debug)]
pub struct BigHeadDay;

impl EventModule for BigHeadDay {
    fn kind(&self) -> EventKind {
        EventKind::BigHeadDay
    }

    fn implemented(&self) -> bool {
        true
    }

    fn hit_radius_scale(&self, fx: &EffectCtx) -> f32 {
        fx.def.param("hit_scale", 3.0).max(0.0)
    }
}
