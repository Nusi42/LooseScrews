//! Plasma Repeater: energy primary. Rapid bolts at 33 u/s. Malus: high energy drain. Lasting
//! effect: melts shallow pits into terrain.
//!
//! Numbers: `data/weapons/plasma_repeater.ron` (`params`: `pit_radius`).
//!
//! Status: TODO(wave-B). Fallback: straight rapid bolts at 33 u/s.
//! To implement: `on_terrain_hit`: melt a shallow pit (`ctx.crater(point,
//! params.pit_radius)`), maybe only every n-th hit to keep the edit rate sane.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Plasma Repeater.
#[derive(Debug)]
pub struct PlasmaRepeater;

impl WeaponBehavior for PlasmaRepeater {
    fn id(&self) -> &'static str {
        "plasma_repeater"
    }
}
