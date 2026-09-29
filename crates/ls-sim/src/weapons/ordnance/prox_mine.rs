//! Prox Mine: ordnance secondary. Dropped behind you, arms after 1 s. Malus: hurts you and
//! teammates. Lasting effect: sticks to walls; 3 u crater.
//!
//! Numbers: `data/weapons/prox_mine.ron` (`params`: `arm_secs`, `trigger_radius`,
//! `drop_behind`).
//!
//! Status: TODO(wave-B). Fallback: slow straight projectile.
//! To implement: `fire`: drop behind the ship; arm after `params.arm_secs` (`p.arm_tick`);
//! `on_terrain_hit`: `HitResponse::Stick`; `update`: detonate when any ship (teammates and
//! owner included) is within `params.trigger_radius` once armed (`p.fuse_tick = now`).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Prox Mine.
#[derive(Debug)]
pub struct ProxMine;

impl WeaponBehavior for ProxMine {
    fn id(&self) -> &'static str {
        "prox_mine"
    }
}
