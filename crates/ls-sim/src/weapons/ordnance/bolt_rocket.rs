//! Bolt Rocket: dumbfire ordnance secondary, limited count. Design doc: "Dumbfire, 40 u/s.
//! Malus: small blast. Lasting effect: 2 u crater" (diameter -> radius 1.0).
//!
//! Numbers: `data/weapons/bolt_rocket.ron`.
//!
//! Status: implemented. The generic hooks are the M1 behaviour: dumbfire rocket, direct hit
//! plus splash, 2 u crater.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Bolt Rocket.
#[derive(Debug)]
pub struct BoltRocket;

impl WeaponBehavior for BoltRocket {
    fn id(&self) -> &'static str {
        "bolt_rocket"
    }

    fn implemented(&self) -> bool {
        true
    }
}
