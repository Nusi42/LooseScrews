//! Heavy Rocket: ordnance secondary. 20 u/s, 8 u blast. Malus: very slow; hurts you at close
//! range. Lasting effect: 6 u crater that can open a passage between tunnels.
//!
//! Numbers: `data/weapons/heavy_rocket.ron`.
//!
//! Status: TODO(wave-B). Fallback: slow dumbfire rocket with the data's 8 u blast and 6 u
//! crater.
//! To implement: Mostly data; mark implemented once scenario-tested (self damage at close
//! range, crater opens passages).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Heavy Rocket.
#[derive(Debug)]
pub struct HeavyRocket;

impl WeaponBehavior for HeavyRocket {
    fn id(&self) -> &'static str {
        "heavy_rocket"
    }
}
