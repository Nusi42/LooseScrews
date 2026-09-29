//! Scatter Laser: energy primary. 3 bolts in an alternating fan. Malus: damage spreads thin
//! at range. Lasting effect: fan-shaped scorch pattern.
//!
//! Numbers: `data/weapons/scatter_laser.ron` (`params`: `fan_alternate`).
//!
//! Status: TODO(wave-B). Fallback: the data's 3-projectile horizontal fan every shot.
//! To implement: `fire`: alternate the fan (e.g. left/centre/right offsets flip each shot,
//! `params.fan_alternate`); damage per bolt stays low so it spreads thin at range; scorch
//! pattern on terrain hits.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Scatter Laser.
#[derive(Debug)]
pub struct ScatterLaser;

impl WeaponBehavior for ScatterLaser {
    fn id(&self) -> &'static str {
        "scatter_laser"
    }
}
