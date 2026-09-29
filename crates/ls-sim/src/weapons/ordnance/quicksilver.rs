//! Quicksilver: ordnance secondary. Fastest missile, 45 u/s. Malus: small blast. Lasting
//! effect: 2 u crater.
//!
//! Numbers: `data/weapons/quicksilver.ron`.
//!
//! Status: TODO(wave-B). Fallback: fast dumbfire missile (the design behaviour is mostly
//! data).
//! To implement: Probably only tuning; mark implemented once the scenario test covers speed,
//! blast and crater.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Quicksilver.
#[derive(Debug)]
pub struct Quicksilver;

impl WeaponBehavior for Quicksilver {
    fn id(&self) -> &'static str {
        "quicksilver"
    }
}
