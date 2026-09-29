//! Tectonic Charge: ordnance secondary. Earthshaker-class, 15 u/s (the slowest projectile).
//! Sticks where it hits, 1.5 s fuse, main blast then 6 sub-charges that scatter up to 20 u
//! and explode 0.5 s later. Zones: core 4 u instant kill, damage 25 u (falloff), collapse 35
//! u (terrain caves in, Instability sets how far cracks spread), shockwave 60 u (knockback,
//! no damage). Dust cloud 20 s, large debris field. One per map, 90 s respawn, a ship carries
//! one; damages its owner. The fallback is a plain slow rocket with a small crater (the real
//! zones are wave B).
//!
//! Numbers: `data/weapons/tectonic_charge.ron` (`params`: `core_radius`, `damage_radius`,
//! `collapse_radius`, `shockwave_radius`, `fuse_secs`, `sub_charges`, `sub_scatter`,
//! `sub_delay_secs`, `dust_secs`).
//!
//! Status: TODO(wave-B). Fallback: slow dumbfire rocket with a small crater.
//! To implement: Launch cue (`TectonicLaunch`), `HitResponse::Stick` + fuse
//! `params.fuse_secs` (`TectonicArmed`), `on_fuse`: core kill within `params.core_radius`,
//! falloff damage to `params.damage_radius`, collapse edits to `params.collapse_radius`
//! (scaled by Instability), shockwave to `params.shockwave_radius` (`ctx.shockwave`), 6 sub-
//! charges (`sub_projectile`, fuse 0.5 s), dust cloud (`EntitySpawn::dust`) and a debris
//! field (`ctx.spawn_debris`, capped).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Tectonic Charge.
#[derive(Debug)]
pub struct TectonicCharge;

impl WeaponBehavior for TectonicCharge {
    fn id(&self) -> &'static str {
        "tectonic_charge"
    }
}
