//! Borrowed Time: special secondary. Rewinds your position, velocity and hull by 3 s. Malus:
//! weapons offline for 3 s; a ghost stays visible at the old spot for 3 s. Lasting effect:
//! terrain damage is not rewound, so the scars stay. The fallback fires a harmless slow
//! pulse.
//!
//! Numbers: `data/weapons/borrowed_time.ron` (`params`: `rewind_secs`, `offline_secs`,
//! `ghost_secs`).
//!
//! Status: TODO(wave-B). Fallback: harmless slow pulse.
//! To implement: `fire`: no projectile; `ctx.rewind(shooter, 3 s)`,
//! `ctx.disable_weapons(shooter, 3 s)`, ghost at the old spot (`EntitySpawn::ghost`), cue
//! `Rewound`. `ctx.trail` has the last 3 s.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Borrowed Time.
#[derive(Debug)]
pub struct BorrowedTime;

impl WeaponBehavior for BorrowedTime {
    fn id(&self) -> &'static str {
        "borrowed_time"
    }
}
