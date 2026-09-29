//! Swap Harpoon: special secondary. On hit, you swap positions with the target (velocities
//! swap too). Malus: 4 s cooldown; you inherit their momentum. Lasting effect: the harpoon
//! spike stays in the wall as a small grab point.
//!
//! Numbers: `data/weapons/swap_harpoon.ron` (`params`: `spike_radius`).
//!
//! Status: TODO(wave-B). Fallback: straight harpoon doing light damage.
//! To implement: `on_ship_hit`: `ctx.swap(owner, target)` (positions and velocities) and cue
//! `Swapped`; `on_terrain_hit`: stick and leave a grab-point marker (`EntitySpawn::marker`).
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Swap Harpoon.
#[derive(Debug)]
pub struct SwapHarpoon;

impl WeaponBehavior for SwapHarpoon {
    fn id(&self) -> &'static str {
        "swap_harpoon"
    }
}
