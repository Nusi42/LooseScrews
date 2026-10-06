//! Everything is cake (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 20 s every crater has `crater_scale` (1.44x) the radius, about 3x
//! the volume, on top of rock strength and Instability (`crater_scale`),
//! and the crater debris is cake: scavenging while the event runs repairs
//! `scavenge_scale` (2x) the hull per scrap (`scavenge_scale`).
//!
//! The client turns the terrain to cake in a wave from the tell's position:
//! the centroid of the living ships at the announce, packed into the
//! parameter bytes 0-5 (`events::params::pack_pos`; read it with
//! [`EverythingIsCake::origin`]).
//!
//! Numbers: `data/events/everything_is_cake.ron`.

use ls_math::Vec3;

use crate::events::params::{pack_pos, unpack_pos};
use crate::events::{EffectCtx, EventCtx, EventKind, EventModule};

/// Everything is cake.
#[derive(Debug)]
pub struct EverythingIsCake;

impl EverythingIsCake {
    /// Where the cake wave starts (from the parameter bytes).
    pub fn origin(params: &[u8; 8]) -> Vec3 {
        let mut b = [0u8; 6];
        b.copy_from_slice(&params[..6]);
        unpack_pos(b)
    }
}

impl EventModule for EverythingIsCake {
    fn kind(&self) -> EventKind {
        EventKind::EverythingIsCake
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_announce(&self, e: &mut EventCtx) {
        let (sum, n) = e
            .ctx
            .view
            .ships
            .values()
            .filter(|s| s.alive)
            .fold((Vec3::ZERO, 0u32), |(a, n), s| (a + s.pos, n + 1));
        let origin = if n == 0 { Vec3::ZERO } else { sum / n as f32 };
        e.event.pos = origin;
        e.event.params[..6].copy_from_slice(&pack_pos(origin));
    }

    fn crater_scale(&self, fx: &EffectCtx) -> f32 {
        fx.def.param("crater_scale", 1.44).max(0.0)
    }

    fn scavenge_scale(&self, fx: &EffectCtx) -> f32 {
        fx.def.param("scavenge_scale", 2.0).max(0.0)
    }
}
