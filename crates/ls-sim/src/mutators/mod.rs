//! Mutators: rule changes that combine with any mode (design doc,
//! Mutators). One file per mutator (`mutators/<id>.rs`) behind
//! [`MutatorModule`], registered in [`MODULES`] (in [`Mutator::ALL`]
//! order). Their hooks run after the mode's:
//!
//! | Hook | When |
//! | --- | --- |
//! | `rewrite_rules` | `RuleSet::resolve`: change the effective rule set (weapons, pickups, gadgets) before it is validated |
//! | `respawn_ticks` | a ship died: the respawn delay (instant respawn) |
//! | `on_damage` | a ship takes damage, after the mode's `on_damage` |
//! | `view_flags` | `World::mode_view`: HUD flags (`mode::view_flags`) |

pub mod instagib;

use std::fmt::Debug;

use crate::modes::{ModeHit, ModeRef};
use crate::ruleset::{Mutator, RuleSet};

/// The hooks of one mutator.
pub trait MutatorModule: Send + Sync + Debug {
    fn mutator(&self) -> Mutator;

    /// Whether the module implements the mutator (false: a stub that
    /// changes nothing).
    fn implemented(&self) -> bool {
        false
    }

    /// Rewrites the rule set before it is validated and resolved.
    fn rewrite_rules(&self, _rules: &mut RuleSet) {}

    /// Respawn delay in ticks, given the mode's.
    fn respawn_ticks(&self, ticks: u64) -> u64 {
        ticks
    }

    /// A ship takes damage (after the mode's hook).
    fn on_damage(&self, _m: &ModeRef, _hit: &mut ModeHit) {}

    /// `mode::view_flags` bits this mutator sets.
    fn view_flags(&self) -> u16 {
        0
    }
}

/// One registered mutator.
#[derive(Debug, Clone, Copy)]
pub struct MutatorEntry {
    pub mutator: Mutator,
    pub module: &'static dyn MutatorModule,
}

/// Every mutator, in [`Mutator::ALL`] order.
pub static MODULES: [MutatorEntry; 1] = [MutatorEntry {
    mutator: Mutator::Instagib,
    module: &instagib::Instagib,
}];

/// The module of a mutator.
pub fn module(m: Mutator) -> &'static dyn MutatorModule {
    MODULES[m.index() as usize].module
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn registry_matches_the_mutator_list() {
        assert_eq!(MODULES.len(), Mutator::ALL.len());
        for (i, m) in Mutator::ALL.iter().enumerate() {
            assert_eq!(MODULES[i].mutator, *m);
            assert_eq!(module(*m).mutator(), *m);
            assert_eq!(m.index() as usize, i);
        }
    }
}
