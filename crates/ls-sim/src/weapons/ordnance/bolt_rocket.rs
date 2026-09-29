//! Bolt Rocket: dumbfire ordnance secondary, limited count. Design doc: "Dumbfire, 40 u/s.
//! Malus: small blast. Lasting effect: 2 u crater" (diameter -> radius 1.0).
//!
//! Numbers: `data/weapons/bolt_rocket.ron` (M2 plan: 30 direct plus splash).
//!
//! Status: implemented. The generic hooks are exactly the design behaviour: a straight rocket,
//! direct hit plus a small splash (2.5 u) that also hurts its owner, knockback, and a 2 u crater
//! scaled by rock strength.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Bolt Rocket.
#[derive(Debug)]
pub struct BoltRocket;

impl WeaponBehavior for BoltRocket {
    fn id(&self) -> &'static str {
        "bolt_rocket"
    }

    fn implemented(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::terrain::{MATERIAL_HARD, MATERIAL_WEAK};
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Bonus: dumbfire at 40 u/s, 30 direct damage plus knockback; it does
    /// not turn towards a target off its line.
    #[test]
    fn dumbfire_40_with_a_30_direct_hit() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "bolt_rocket");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        let v = w.projectiles()[0].vel;
        assert_eq!(v, Vec3::new(0.0, 0.0, -40.0));
        ev.extend(run(&mut w, &mut t, &[], 40));
        assert_eq!(hits_on(&ev, b), vec![30.0]);
        assert!(w.ship(b).unwrap().pos.z < 0.0, "knocked back");

        // Off the line: it flies straight past.
        let mut w = world();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "bolt_rocket");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(4.0, 0.0, 0.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 40));
        assert_eq!(damage_to(&ev, b), 0.0);
    }

    /// Malus: small blast: splash reaches a ship 2 u from the impact but not
    /// one 3 u away.
    #[test]
    fn small_blast() {
        for (side, hurt) in [(2.0f32, true), (3.0, false)] {
            let mut w = world();
            let mut t = room();
            let (a, b) = duel(&mut w);
            equip(&mut w, a, "bolt_rocket");
            place(&mut w, a, Vec3::new(0.0, 0.0, -15.0), Vec3::NEG_Z);
            place(&mut w, b, Vec3::new(side, 0.0, -29.0), Vec3::Z);
            let ev = run(&mut w, &mut t, &fire2(a), 30);
            assert_eq!(damage_to(&ev, b) > 0.0, hurt, "{side} u from the blast");
        }
    }

    /// Lasting effect: a 2 u (diameter) crater, bigger in weak rock and
    /// smaller in hard rock.
    #[test]
    fn two_unit_crater_scaled_by_rock() {
        for (material, radius) in [
            (ls_math::terrain::MATERIAL_ROCK, 1.0f32),
            (MATERIAL_WEAK, 1.4),
            (MATERIAL_HARD, 0.6),
        ] {
            let mut w = world();
            let mut t = room();
            t.material = material;
            let (a, _) = duel(&mut w);
            equip(&mut w, a, "bolt_rocket");
            place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
            let ev = run(&mut w, &mut t, &fire2(a), 30);
            let c = craters(&ev);
            assert_eq!(c.len(), 1);
            assert!((c[0].1 - radius).abs() < 1e-5, "{material}: {c:?}");
        }
    }
}
