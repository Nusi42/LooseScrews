//! Gravity coffee break (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! Every gravity well (the level's and the dynamic ones: Paperweights,
//! Architect strokes) switches off for 20 s: the pull fades to nothing over
//! `fade_secs` (1 s) at the start of the active phase and back during the
//! 1 s ending phase (`well_scale`, applied by the world to ships, debris and
//! flagged projectiles and by the client predictor to the own ship).
//! Needs wells: the level has well sites and Gravity Chaos above 0.
//!
//! Numbers: `data/events/gravity_coffee_break.ron`.

use ls_math::Tick;

use crate::events::{EffectCtx, EventKind, EventModule, EventPhase, EventRef};

/// Gravity coffee break.
#[derive(Debug)]
pub struct GravityCoffeeBreak;

impl GravityCoffeeBreak {
    /// The wells' scale at `fx.now` (1 = full pull, 0 = off).
    pub fn scale(fx: &EffectCtx) -> f32 {
        match fx.view.phase {
            EventPhase::Active => {
                let fade = Tick::ticks_for_secs(fx.def.param("fade_secs", 1.0)).max(1);
                let x = fx.active_for().min(fade) as f32 / fade as f32;
                1.0 - x
            }
            EventPhase::Ending => {
                let len = fx.def.ending_ticks().max(1);
                let left = fx.view.phase_end_tick.saturating_sub(fx.now).min(len);
                1.0 - left as f32 / len as f32
            }
            EventPhase::Announce => 1.0,
        }
    }
}

impl EventModule for GravityCoffeeBreak {
    fn kind(&self) -> EventKind {
        EventKind::GravityCoffeeBreak
    }

    fn implemented(&self) -> bool {
        true
    }

    /// Needs wells: well sites on the level and Gravity Chaos above 0 (the
    /// static wells are off at 0).
    fn eligible(&self, e: &EventRef) -> bool {
        e.setup.gravity_chaos > 0 && !e.setup.wells.is_empty()
    }

    fn well_scale(&self, fx: &EffectCtx) -> f32 {
        Self::scale(fx)
    }
}
