//! Instagib: Railspike only, unlimited ammo, any hit kills; no pickups, no
//! gadgets; hull and energy hidden, boost kept; instant respawn in modes
//! with respawns. Works on every PvP mode.
//!
//! Design: `docs/milestone-3-plan.md` §3.2 (Instagib).
//!
//! Status: stub. It only sets the HUD flag; the rule rewrite and the
//! damage hook arrive in M3 wave B.

use crate::mode::view_flags;
use crate::mutators::MutatorModule;
use crate::ruleset::Mutator;

/// The Instagib mutator module.
#[derive(Debug)]
pub struct Instagib;

impl MutatorModule for Instagib {
    fn mutator(&self) -> Mutator {
        Mutator::Instagib
    }

    fn view_flags(&self) -> u16 {
        view_flags::INSTAGIB
    }
}
