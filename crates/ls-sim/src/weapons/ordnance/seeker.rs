//! Seeker: ordnance secondary. Homes on the nearest target in a 30 degree cone, 25 u/s.
//! Malus: can be outflown or fooled by the Decoy. Lasting effect: 2 u crater.
//!
//! Numbers: `data/weapons/seeker.ron` (`params`: `cone_deg`, `turn_rate`).
//!
//! Status: TODO(wave-B). Fallback: dumbfire missile.
//! To implement: `update`: home on the nearest target in a `params.cone_deg` cone
//! (`ctx.target_in_cone`, keep it in `p.target`), turn at most `params.turn_rate` rad/s
//! (`steer_towards`); decoys should be valid targets.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Seeker.
#[derive(Debug)]
pub struct Seeker;

impl WeaponBehavior for Seeker {
    fn id(&self) -> &'static str {
        "seeker"
    }
}
