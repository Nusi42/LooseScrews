//! Cloak (design doc, Gadgets; M2 plan): enemies do not receive a cloaked
//! ship in snapshots unless they are within `reveal_distance` (6 u) or the
//! ship fired in the last `reveal_after_fire_secs` (0.5 s). Teammates always
//! see it (clients draw a shimmer). Malus: the framework drains energy 3x
//! (`moving_drain_mult`) while the ship moves faster than `moving_speed`.
//!
//! The relevance decision is [`crate::World::ship_visible_to`], which the
//! server's per-client snapshot filter calls.
//!
//! Status: implemented (basic).

use crate::gadgets::{GadgetBehavior, GadgetDef};
use crate::ship::ShipState;

#[derive(Debug)]
pub struct Cloak;

impl GadgetBehavior for Cloak {
    fn id(&self) -> &'static str {
        "cloak"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn visible_to(&self, def: &GadgetDef, ship: &ShipState, viewer: &ShipState, now: u64) -> bool {
        if !ship.gadget_active || viewer.team == ship.team || viewer.player == ship.player {
            return true;
        }
        let reveal = def.param("reveal_distance", 6.0);
        if ship.pos.distance_squared(viewer.pos) <= reveal * reveal {
            return true;
        }
        let fired = ls_math::Tick::ticks_for_secs(def.param("reveal_after_fire_secs", 0.5));
        ship.last_fired_tick
            .is_some_and(|t| now < t.saturating_add(fired.max(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GadgetTable, PlayerId, ShipTuning, ShipType};
    use ls_math::{Quat, Vec3};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Scenario in a world: the reveal radius is 6 u, firing reveals for
    /// exactly 0.5 s, moving costs 3x the drain (9 energy/s while boosting,
    /// when nothing regenerates) and an empty cloak drops, revealing the ship.
    #[test]
    fn world_reveal_radius_fire_window_and_moving_drain() {
        use crate::gadgets::test_util::*;
        use crate::{Buttons, InputCmd};

        let mut w = world_with_gadget("cloak");
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(6.5, 0.0, 0.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
        assert!(w.ship(a).unwrap().gadget_active);
        assert!(!w.ship_visible_to(b, a), "6.5 u away: hidden");
        place(&mut w, b, Vec3::new(5.9, 0.0, 0.0), Vec3::NEG_Z);
        assert!(w.ship_visible_to(b, a), "inside 6 u: seen");
        place(&mut w, b, Vec3::new(20.0, 0.0, 0.0), Vec3::NEG_Z);
        assert!(!w.ship_visible_to(b, a));
        // Fire: visible for 30 ticks (0.5 s), then hidden again.
        run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
        let fired = w.ship(a).unwrap().last_fired_tick.expect("fired");
        while w.tick() < fired + 29 {
            run(&mut w, &mut t, &[], 1);
            assert!(w.ship_visible_to(b, a), "tick {}", w.tick() - fired);
        }
        run(&mut w, &mut t, &[], 1);
        assert!(!w.ship_visible_to(b, a), "0.5 s after the shot");
        // Moving (boosting: no regen) drains 3 x 3 = 9 energy/s.
        let boost = [(
            a,
            InputCmd::idle(0)
                .with_thrust(Vec3::X)
                .with_buttons(Buttons::BOOST),
        )];
        run(&mut w, &mut t, &boost, 10);
        let s = w.ship_mut(a).unwrap();
        s.energy = 50.0;
        s.boost = 100.0;
        run(&mut w, &mut t, &boost, 60);
        let e = w.ship(a).unwrap().energy;
        assert!((e - 41.0).abs() < 0.05, "{e}");
        // Out of energy: the cloak drops and the enemy sees the ship.
        w.ship_mut(a).unwrap().energy = 0.1;
        run(&mut w, &mut t, &boost, 2);
        assert!(!w.ship(a).unwrap().gadget_active);
        assert!(w.ship_visible_to(b, a));
    }

    #[test]
    fn hidden_from_distant_enemies_unless_firing() {
        let t = GadgetTable::builtin();
        let def = t.get(t.find("cloak").unwrap());
        let tuning = ShipTuning::tinker();
        let mut ship = ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            &tuning,
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        let far = ShipState::new(
            PlayerId(1),
            1,
            ShipType::Tinker,
            &tuning,
            Vec3::new(20.0, 0.0, 0.0),
            Quat::IDENTITY,
        );
        let near = ShipState::new(
            PlayerId(2),
            1,
            ShipType::Tinker,
            &tuning,
            Vec3::new(5.0, 0.0, 0.0),
            Quat::IDENTITY,
        );
        let mate = ShipState::new(
            PlayerId(3),
            0,
            ShipType::Tinker,
            &tuning,
            Vec3::new(50.0, 0.0, 0.0),
            Quat::IDENTITY,
        );
        assert!(Cloak.visible_to(def, &ship, &far, 100), "off: visible");
        ship.gadget_active = true;
        assert!(!Cloak.visible_to(def, &ship, &far, 100));
        assert!(Cloak.visible_to(def, &ship, &near, 100), "within 6 u");
        assert!(Cloak.visible_to(def, &ship, &mate, 100), "teammates see it");
        ship.last_fired_tick = Some(90);
        assert!(
            Cloak.visible_to(def, &ship, &far, 100),
            "fired 10 ticks ago"
        );
        assert!(
            !Cloak.visible_to(def, &ship, &far, 120),
            "0.5 s later hidden again"
        );
    }
}
