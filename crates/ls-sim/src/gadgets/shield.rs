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

    /// Scenario: the weapon data's shield setting decides what an enemy hit
    /// does to a shielded ship (Scrap Laser `Wears`: energy pays, Rivet Gun
    /// `PassThrough`: that fraction reaches the hull); upkeep drains energy
    /// while boosting (no regen) and the shield drops at zero energy.
    #[test]
    fn enemy_weapons_follow_their_shield_setting_and_upkeep_drops_it() {
        use crate::gadgets::test_util::*;
        use crate::{Buttons, InputCmd, WeaponSlot, WeaponState};

        let hit = |weapon: &str, shield: bool| {
            let mut w = world_with_gadget("shield");
            let mut t = room();
            let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
            let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
            if shield {
                run(&mut w, &mut t, &press(b, Buttons::GADGET), 1);
                assert!(w.ship(b).unwrap().gadget_active);
            }
            let wid = w.weapons().find(weapon).unwrap();
            let def = w.weapons().get(wid).clone();
            let s = w.ship_mut(a).unwrap();
            s.add_weapon(WeaponState {
                weapon: wid,
                ammo: def.max_ammo,
                level: 1,
                shots: 0,
            });
            assert_eq!(def.slot, WeaponSlot::Primary);
            s.primary = Some(wid);
            s.scrap = 50.0;
            place(&mut w, a, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
            place(&mut w, b, Vec3::ZERO, Vec3::Z);
            w.ship_mut(b).unwrap().energy = 50.0;
            let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
            ev.extend(run(&mut w, &mut t, &[], 20));
            // 21 ticks of regen (10/s) minus upkeep (4/s).
            let regen = 21.0 / 60.0 * (10.0 - 4.0);
            let spent = 50.0 + regen - w.ship(b).unwrap().energy;
            (damage_to(&ev, b), spent, def.shield)
        };
        // Scrap Laser: Wears(1.0) -> no hull damage, the energy pays 1 per point.
        let (bare, _, _) = hit("scrap_laser", false);
        let (dmg, spent, setting) = hit("scrap_laser", true);
        assert_eq!(setting, ShieldInteraction::Wears(1.0));
        assert!(bare > 0.0);
        assert_eq!(dmg, 0.0);
        assert!((spent - bare).abs() < 0.1, "{spent} vs {bare}");
        // Rivet Gun: PassThrough(f) -> f of the damage reaches the hull, free.
        let (bare, _, _) = hit("rivet_gun", false);
        let (dmg, _, setting) = hit("rivet_gun", true);
        let ShieldInteraction::PassThrough(f) = setting else {
            panic!("rivet gun data: {setting:?}");
        };
        assert!(bare > 0.0);
        assert!((dmg - bare * f).abs() < 1e-3, "{dmg} vs {bare} x {f}");

        // Upkeep: boosting stops regen, so the shield eats 4 energy/s and drops at 0.
        let mut w = world_with_gadget("shield");
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
        let s = w.ship_mut(a).unwrap();
        s.energy = 2.0;
        s.boost = 100.0;
        let boost = [(
            a,
            InputCmd::idle(0)
                .with_thrust(Vec3::X)
                .with_buttons(Buttons::BOOST),
        )];
        run(&mut w, &mut t, &boost, 15);
        let s = w.ship(a).unwrap();
        assert!(
            s.gadget_active && (s.energy - 1.0).abs() < 0.02,
            "{}",
            s.energy
        );
        let ev = run(&mut w, &mut t, &boost, 16);
        assert!(!w.ship(a).unwrap().gadget_active, "dropped at zero energy");
        assert!(ev.iter().any(|e| matches!(
            e,
            crate::Event::Cue(c) if c.kind == crate::CueKind::Gadget && c.value == 255
        )));
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
