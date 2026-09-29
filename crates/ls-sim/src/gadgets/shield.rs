//! Shield (design doc, Gadgets): mainly protects you from your own weapons,
//! such as splash damage, ricochets and your returning Echo Cannon shots.
//! Against enemy fire each weapon has a shield setting
//! ([`ShieldInteraction`], in the weapon data, overridable per rule set):
//! blocked, wears the shield down, or partly passes through to the hull.
//!
//! Numbers (`data/gadgets/shield.ron`): `self_damage_reduction` (0.75 =
//! self-damage cut by 75%, M2 plan) and `energy_per_damage` (energy the
//! shield loses per absorbed damage point). Toggle with G; the framework
//! drains `drain_per_sec` while it is up and drops it at zero energy.
//!
//! Status: implemented (basic). Knockback is not reduced.

use crate::gadgets::{GadgetBehavior, GadgetDef, IncomingHit};
use crate::ship::ShipState;
use crate::ShieldInteraction;

#[derive(Debug)]
pub struct Shield;

impl GadgetBehavior for Shield {
    fn id(&self) -> &'static str {
        "shield"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_damage(&self, def: &GadgetDef, ship: &mut ShipState, hit: &mut IncomingHit) {
        absorb(def, ship, hit);
    }
}

/// Applies the shield to a hit: self-damage reduction, then the weapon's
/// shield interaction paid from the ship's energy. Drops the shield when
/// the energy runs out.
pub fn absorb(def: &GadgetDef, ship: &mut ShipState, hit: &mut IncomingHit) {
    if hit.amount <= 0.0 {
        return;
    }
    if hit.self_inflicted {
        let r = def.param("self_damage_reduction", 0.75).clamp(0.0, 1.0);
        hit.amount *= 1.0 - r;
        return;
    }
    let epd = def.param("energy_per_damage", 1.0).max(0.0);
    match hit.shield {
        ShieldInteraction::Blocked => {
            ship.energy = (ship.energy - hit.amount * epd).max(0.0);
            hit.amount = 0.0;
        }
        ShieldInteraction::Wears(f) => {
            let cost_per_point = f * epd;
            let cost = hit.amount * cost_per_point;
            if cost <= ship.energy {
                ship.energy -= cost;
                hit.amount = 0.0;
            } else {
                // The energy covers part of the hit; the rest reaches the hull.
                let covered = ship.energy / cost_per_point;
                ship.energy = 0.0;
                hit.amount -= covered;
            }
        }
        ShieldInteraction::PassThrough(f) => {
            hit.amount *= f;
        }
    }
    if ship.energy <= 0.0 {
        ship.gadget_active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GadgetTable, PlayerId, ShipTuning, ShipType};
    use ls_math::{Quat, Vec3};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn setup() -> (GadgetDef, ShipState) {
        let t = GadgetTable::builtin();
        let def = t.get(t.find("shield").unwrap()).clone();
        let mut ship = ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            &ShipTuning::tinker(),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        ship.gadget_active = true;
        (def, ship)
    }

    fn hit(amount: f32, shield: ShieldInteraction, self_inflicted: bool) -> IncomingHit {
        IncomingHit {
            amount,
            knockback: Vec3::ZERO,
            attacker: Some(PlayerId(1)),
            weapon: None,
            shield,
            self_inflicted,
        }
    }

    #[test]
    fn interactions() {
        let (def, mut ship) = setup();
        ship.energy = 50.0;
        let mut h = hit(10.0, ShieldInteraction::Blocked, false);
        absorb(&def, &mut ship, &mut h);
        assert_eq!((h.amount, ship.energy), (0.0, 40.0));

        let mut h = hit(10.0, ShieldInteraction::PassThrough(0.25), false);
        absorb(&def, &mut ship, &mut h);
        assert_eq!((h.amount, ship.energy), (2.5, 40.0));

        let mut h = hit(10.0, ShieldInteraction::Wears(2.0), false);
        absorb(&def, &mut ship, &mut h);
        assert_eq!((h.amount, ship.energy), (0.0, 20.0));

        // Energy covers only part: 20 energy at 2 per point = 10 points.
        let mut h = hit(30.0, ShieldInteraction::Wears(2.0), false);
        absorb(&def, &mut ship, &mut h);
        assert_eq!((h.amount, ship.energy), (20.0, 0.0));
        assert!(!ship.gadget_active, "shield drops at zero energy");
    }

    #[test]
    fn self_damage_is_reduced_by_three_quarters() {
        let (def, mut ship) = setup();
        let mut h = hit(20.0, ShieldInteraction::Wears(1.0), true);
        absorb(&def, &mut ship, &mut h);
        assert_eq!(h.amount, 5.0);
        assert_eq!(ship.energy, ShipTuning::tinker().spawn_energy, "free");
    }
}
