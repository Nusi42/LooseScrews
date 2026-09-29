//! Echo Cannon: special primary. Projectile returns along the same path after 2 s, so it can
//! hit twice. Malus: the returning shot can hit you. Lasting effect: scorch trail along the
//! out-and-back path.
//!
//! Numbers: `data/weapons/echo_cannon.ron` (`params`: `return_after_secs`).
//!
//! Status: TODO(wave-B). Fallback: straight projectile.
//! To implement: `update`: record the path (`p.path`), after `params.return_after_secs` fly
//! it back and set `proj_flags::HITS_OWNER | RETURNING`; scorch trail.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Echo Cannon.
#[derive(Debug)]
pub struct EchoCannon;

impl WeaponBehavior for EchoCannon {
    fn id(&self) -> &'static str {
        "echo_cannon"
    }
}
