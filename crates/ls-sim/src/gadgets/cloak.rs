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
