//! Scrap Shotgun: kinetic primary. 8 pellets at 200 u/s. Malus: useless at range. Lasting
//! effect: pockmarks walls and sprays fresh debris.
//!
//! Numbers: `data/weapons/scrap_shotgun.ron` (`params`: `pockmark_radius`,
//! `debris_per_shot`).
//!
//! Status: TODO(wave-B). Fallback: the data's 8-pellet pattern.
//! To implement: Random pellet spread per shot (`ctx.rng(key)`), pockmark walls
//! (`params.pockmark_radius`) and spray fresh debris (`ctx.spawn_debris`).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Scrap Shotgun.
#[derive(Debug)]
pub struct ScrapShotgun;

impl WeaponBehavior for ScrapShotgun {
    fn id(&self) -> &'static str {
        "scrap_shotgun"
    }
}
