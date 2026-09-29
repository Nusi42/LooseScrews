//! Stalker Mine: ordnance secondary. Creeps toward ships within 10 u. Malus: slow. Lasting
//! effect: leaves a scorched trail along its path.
//!
//! Numbers: `data/weapons/stalker_mine.ron` (`params`: `arm_secs`, `detect_radius`,
//! `creep_speed`, `trigger_radius`).
//!
//! Status: TODO(wave-B). Fallback: slow straight projectile.
//! To implement: `update`: creep towards the nearest enemy within `params.detect_radius` at
//! `params.creep_speed`, detonate within `params.trigger_radius`; leave a scorched trail.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Stalker Mine.
#[derive(Debug)]
pub struct StalkerMine;

impl WeaponBehavior for StalkerMine {
    fn id(&self) -> &'static str {
        "stalker_mine"
    }
}
