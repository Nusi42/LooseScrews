//! Ricochet Arc: energy primary. Bolts bounce off walls up to 3 times. Malus: bounces can hit
//! you. Lasting effect: chips terrain at every bounce.
//!
//! Numbers: `data/weapons/ricochet_arc.ron` (`params`: `bounces`, `chip_radius`).
//!
//! Status: TODO(wave-B). Fallback: straight bolt that stops at walls.
//! To implement: `on_terrain_hit`: `HitResponse::Bounce` up to `params.bounces` times
//! (`p.bounces`), chip terrain each bounce (`params.chip_radius`), set
//! `proj_flags::HITS_OWNER` after the first bounce (bounces can hit you).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Ricochet Arc.
#[derive(Debug)]
pub struct RicochetArc;

impl WeaponBehavior for RicochetArc {
    fn id(&self) -> &'static str {
        "ricochet_arc"
    }
}
