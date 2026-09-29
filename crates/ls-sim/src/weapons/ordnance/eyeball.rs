//! Eyeball: ordnance secondary. You steer it through its camera, 25 u/s. Malus: your ship is
//! exposed while you steer (it drifts without thrust). Lasting effect: 4 u crater.
//!
//! Numbers: `data/weapons/eyeball.ron` (`params`: `turn_rate`).
//!
//! Status: TODO(wave-B). Fallback: dumbfire missile.
//! To implement: `fire`: set `proj_flags::CONTROLS_OWNER` so the world freezes the owner's
//! thrust and look (the ship drifts: the malus); `update`: steer with `ctx.input(owner)` look
//! deltas (at most `params.turn_rate`). The client shows its camera (wave C).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Eyeball.
#[derive(Debug)]
pub struct Eyeball;

impl WeaponBehavior for Eyeball {
    fn id(&self) -> &'static str {
        "eyeball"
    }
}
