//! Flash Pod: ordnance secondary. Blinds pilots within 15 u for 2 s. Malus: no damage.
//! Lasting effect: surfaces glow for 30 s, lighting dark caverns and anyone inside.
//!
//! Numbers: `data/weapons/flash_pod.ron` (`params`: `blind_radius`, `blind_secs`,
//! `glow_secs`, `glow_radius`).
//!
//! Status: TODO(wave-B). Fallback: harmless dumbfire pod.
//! To implement: On impact blind every pilot within `params.blind_radius` for
//! `params.blind_secs` (`ctx.blind`); surfaces glow for 30 s (surface states, A3).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Flash Pod.
#[derive(Debug)]
pub struct FlashPod;

impl WeaponBehavior for FlashPod {
    fn id(&self) -> &'static str {
        "flash_pod"
    }
}
