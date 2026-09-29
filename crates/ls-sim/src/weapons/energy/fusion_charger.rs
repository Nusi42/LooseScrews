//! Fusion Charger: energy primary. Hold up to 2 s; damage and knockback scale with charge.
//! Malus: charging past 2 s burns your own hull. Lasting effect: melts a channel along its
//! path and shoves debris aside.
//!
//! Numbers: `data/weapons/fusion_charger.ron` (`params`: `overcharge_hull_per_sec`,
//! `channel_radius`).
//!
//! Status: TODO(wave-B). Fallback: the framework charge flow (hold up to `charge.max_secs`,
//! damage and knockback scale up to `charge.damage_mult` at full charge) with a straight
//! bolt.
//! To implement: `charge_hold`: past `charge.max_secs` burn own hull (`ctx.adjust_hull(ship,
//! -params.overcharge_hull_per_sec * DT, false)`); `update`/`on_terrain_hit`: melt a channel
//! along the path (`ctx.channel`) and shove debris aside (`ctx.impulse_entity`).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Fusion Charger.
#[derive(Debug)]
pub struct FusionCharger;

impl WeaponBehavior for FusionCharger {
    fn id(&self) -> &'static str {
        "fusion_charger"
    }
}
