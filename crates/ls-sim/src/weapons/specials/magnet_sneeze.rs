//! Magnet Sneeze: special secondary. Cone pulls loose debris and enemies toward you; great
//! for scavenging. Malus: also pulls in mines thrown at you. Lasting effect: rips loose
//! chunks off weak walls.
//!
//! Numbers: `data/weapons/magnet_sneeze.ron` (`params`: `cone_deg`, `range`, `pull_speed`,
//! `chunk_radius`).
//!
//! Status: TODO(wave-B). Fallback: harmless short-lived blob.
//! To implement: `fire`: no projectile; pull debris (`ctx.impulse_entity`), enemies
//! (`ctx.impulse_ship`) and mines in a `params.cone_deg` cone within `params.range` towards
//! the shooter; rip chunks off weak walls.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Magnet Sneeze.
#[derive(Debug)]
pub struct MagnetSneeze;

impl WeaponBehavior for MagnetSneeze {
    fn id(&self) -> &'static str {
        "magnet_sneeze"
    }
}
