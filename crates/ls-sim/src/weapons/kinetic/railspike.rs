//! Railspike: kinetic primary. 270 u/s; heavy shots pierce ships and walls up to 2 u thick.
//! Malus: slow fire rate; muzzle flash reveals you. Lasting effect: drills straight holes
//! through thin terrain that let light through.
//!
//! Numbers: `data/weapons/railspike.ron` (`params`: `pierce_thickness`, `hole_radius`,
//! `reveal_secs`).
//!
//! Status: TODO(wave-B). Fallback: straight fast spike that stops at the first ship or wall.
//! To implement: `on_ship_hit`: damage and `HitResponse::Continue` (pierces ships);
//! `on_terrain_hit`: if `ctx.thickness(point, dir, params.pierce_thickness)` is `Some(t)`,
//! drill a hole (`ctx.channel`) and `HitResponse::Pierce { exit }`; `fire`: cue `MuzzleFlash`
//! (reveals the shooter).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Railspike.
#[derive(Debug)]
pub struct Railspike;

impl WeaponBehavior for Railspike {
    fn id(&self) -> &'static str {
        "railspike"
    }
}
