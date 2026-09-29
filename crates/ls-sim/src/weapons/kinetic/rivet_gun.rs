//! Rivet Gun: kinetic primary. Fires scrap, 6x laser speed, high rate of fire, light damage.
//! Design doc: "180 u/s, high rate of fire, light damage. Malus: burns through scrap fast."
//!
//! Numbers: `data/weapons/rivet_gun.ron`.
//!
//! Status: implemented. The generic hooks are the M1 behaviour: fast straight rivets that
//! cost scrap.
//! Wave B may add the lasting effect (rivets embed in walls, knock loose debris from weak
//! terrain via `ctx.spawn_debris`).

use crate::weapons::WeaponBehavior;

/// Behaviour of the Rivet Gun.
#[derive(Debug)]
pub struct RivetGun;

impl WeaponBehavior for RivetGun {
    fn id(&self) -> &'static str {
        "rivet_gun"
    }

    fn implemented(&self) -> bool {
        true
    }
}
