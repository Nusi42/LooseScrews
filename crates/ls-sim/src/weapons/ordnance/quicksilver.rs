//! Quicksilver: ordnance secondary. Fastest missile, 45 u/s. Malus: small blast. Lasting
//! effect: 2 u crater.
//!
//! Numbers: `data/weapons/quicksilver.ron`.
//!
//! Status: implemented. The design behaviour is data on the generic hooks: a dumbfire
//! missile at 45 u/s (the fastest ordnance, faster than any ship, so it cannot be outflown),
//! a lighter direct hit than the Bolt Rocket and an even smaller splash (1.5 u), and a 2 u
//! (diameter) crater scaled by rock strength.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Quicksilver.
#[derive(Debug)]
pub struct Quicksilver;

impl WeaponBehavior for Quicksilver {
    fn id(&self) -> &'static str {
        "quicksilver"
    }

    fn implemented(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::{Vec3, DT};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Bonus: the fastest missile (45 u/s, faster than every other ordnance
    /// and than a boosting pilot on the diagonal), so a fleeing ship is
    /// caught.
    #[test]
    fn fastest_missile_catches_a_boosting_pilot() {
        let w = world();
        let qs = w.weapons().find("quicksilver").unwrap();
        let speed = w.weapons().get(qs).speed;
        assert_eq!(speed, 45.0);
        for (id, def) in w.weapons().iter() {
            if def.family == WeaponFamily::Ordnance && id != qs {
                assert!(def.speed < speed, "{} is faster", def.id);
            }
        }

        let mut w = world();
        let mut t = room_sized(Vec3::new(60.0, 60.0, 300.0));
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "quicksilver");
        place(&mut w, a, Vec3::new(0.0, 0.0, 120.0), Vec3::NEG_Z);
        let mut pos = Vec3::new(0.0, 0.0, 100.0);
        place(&mut w, b, pos, Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        for _ in 0..150 {
            pos.z -= 28.5 * DT;
            place(&mut w, b, pos, Vec3::NEG_Z);
            ev.extend(run(&mut w, &mut t, &[], 1));
        }
        assert_eq!(hits_on(&ev, b), vec![25.0]);
    }

    /// Malus: small blast (1.5 u): 1 u from the impact hurts, 2 u does not.
    #[test]
    fn small_blast() {
        for (side, hurt) in [(1.0f32, true), (2.0, false)] {
            let mut w = world();
            let mut t = room();
            let (a, b) = duel(&mut w);
            equip(&mut w, a, "quicksilver");
            place(&mut w, a, Vec3::new(0.0, 0.0, -15.0), Vec3::NEG_Z);
            // Behind the wall's surface plane only by the hull radius.
            place(&mut w, b, Vec3::new(side, 0.0, -29.4), Vec3::Z);
            let ev = run(&mut w, &mut t, &fire2(a), 30);
            let d = damage_to(&ev, b);
            assert_eq!(d > 0.0, hurt, "{side} u from the blast: {d}");
            assert!(d < 25.0, "splash only");
        }
    }

    /// Lasting effect: a 2 u (diameter) crater.
    #[test]
    fn two_unit_crater() {
        let mut w = world();
        let mut t = room();
        let (a, _) = duel(&mut w);
        equip(&mut w, a, "quicksilver");
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &fire2(a), 30);
        let c = craters(&ev);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].1 * 2.0, 2.0);
    }
}
