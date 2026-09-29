//! Polite Laser: special primary. Very high damage. Malus: 1.5 s charge with an 'excuse me'
//! chime the target hears (PoliteWarning cue). Lasting effect: burns a clean hole through
//! thin walls.
//!
//! Numbers: `data/weapons/polite_laser.ron` (`params`: `hole_radius`, `hole_thickness`).
//!
//! Status: TODO(wave-B). Fallback: the framework charge flow (1.5 s, auto-release) with a
//! straight bolt.
//! To implement: `charge_start`: cue `PoliteWarning` to the target in the aim;
//! `on_terrain_hit`: burn a clean hole through thin walls (`ctx.thickness`, `ctx.channel`,
//! `HitResponse::Pierce`).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Polite Laser.
#[derive(Debug)]
pub struct PoliteLaser;

impl WeaponBehavior for PoliteLaser {
    fn id(&self) -> &'static str {
        "polite_laser"
    }
}
