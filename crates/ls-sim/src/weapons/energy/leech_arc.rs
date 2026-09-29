//! Leech Arc: energy primary. Short arc (12 u) that locks the nearest target and drains its
//! energy into yours. Malus: useless beyond 12 u. Lasting effect: surfaces stay charged for
//! 15 s and drain energy from ships that scrape them. The only instant weapon (M2 plan); the
//! fallback fires a short-lived fast bolt with the same reach.
//!
//! Numbers: `data/weapons/leech_arc.ron` (`params`: `range`, `lock_cone_deg`,
//! `drain_per_sec`, `charged_secs`).
//!
//! Status: TODO(wave-B). Fallback: a fast short-lived bolt with the arc's reach.
//! To implement: The only instant weapon: `fire` locks the nearest target within
//! `params.range` (`ctx.target_in_cone`), drains its energy into yours (`ctx.drain_energy`)
//! with damage, and cues the beam (`CueKind::Beam`); no projectile. Charged surfaces (15 s)
//! need the A3 surface states.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Leech Arc.
#[derive(Debug)]
pub struct LeechArc;

impl WeaponBehavior for LeechArc {
    fn id(&self) -> &'static str {
        "leech_arc"
    }
}
