//! Scrap Laser: energy primary, the laser-speed baseline (30 u/s, 3 s lifetime). Design doc:
//! "Straight bolts at 30 u/s; 6 upgrade levels found as pickups. Malus: weak at level 1."
//!
//! Numbers: `data/weapons/scrap_laser.ron`.
//!
//! Status: implemented. The generic hooks are the M1 behaviour: straight bolts at 30 u/s, 3 s
//! lifetime; upgrade pickups raise the level (damage_per_level).
//! Wave B may add the lasting effect (scorch marks, hot metal for 10 s via surface states) in
//! `on_terrain_hit`.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Scrap Laser.
#[derive(Debug)]
pub struct ScrapLaser;

impl WeaponBehavior for ScrapLaser {
    fn id(&self) -> &'static str {
        "scrap_laser"
    }

    fn implemented(&self) -> bool {
        true
    }
}
