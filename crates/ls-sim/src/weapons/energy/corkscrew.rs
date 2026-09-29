//! Corkscrew: energy primary. 5 bolts in a rotating fan. Malus: wide pattern wastes energy at
//! long range. Lasting effect: etches spiral grooves into walls.
//!
//! Numbers: `data/weapons/corkscrew.ron` (`params`: `spiral_radius`, `spin_hz`,
//! `groove_radius`).
//!
//! Status: TODO(wave-B). Fallback: the data's 5-projectile pattern (centre plus ring), flying
//! straight.
//! To implement: `update`: rotate each bolt around the shot axis (`params.spiral_radius`,
//! `params.spin_hz`; keep the axis in `vars`); `on_terrain_hit`: etch spiral grooves (small
//! craters).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Corkscrew.
#[derive(Debug)]
pub struct Corkscrew;

impl WeaponBehavior for Corkscrew {
    fn id(&self) -> &'static str {
        "corkscrew"
    }
}
