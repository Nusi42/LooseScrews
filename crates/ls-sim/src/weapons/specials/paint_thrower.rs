//! Paint Thrower: special. Deals no damage; paints terrain in the shooter's team color (the
//! core tool of Paint the Void). Surface effects of paint colors come in a later milestone.
//!
//! Numbers: `data/weapons/paint_thrower.ron`.
//!
//! Status: implemented. The generic hooks are the M1 behaviour: no damage, paints in the team
//! colour.
//! Paint colours with surface effects (red bouncy, blue slippery, yellow sticky) are a later
//! milestone.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Paint Thrower.
#[derive(Debug)]
pub struct PaintThrower;

impl WeaponBehavior for PaintThrower {
    fn id(&self) -> &'static str {
        "paint_thrower"
    }

    fn implemented(&self) -> bool {
        true
    }
}
