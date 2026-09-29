//! Ricochet Arc: energy primary. Design doc: "Bolts bounce off walls up to 3 times. Malus:
//! bounces can hit you. Lasting effect: chips terrain at every bounce."
//!
//! Numbers: `data/weapons/ricochet_arc.ron` (`params`: `bounces`, `chip_radius`).
//!
//! Behaviour: a laser-speed bolt. On terrain it chips a small crater (`chip_radius`, scaled by
//! rock strength) and reflects off the surface (`HitResponse::Bounce`) while it has bounced
//! fewer than `bounces` times; the hit after the last bounce chips and ends it. From the first
//! bounce on it can hit its owner (`proj_flags::HITS_OWNER`); the first leg cannot. Ship hits
//! are the generic direct hit (damage and knockback).

use ls_math::terrain::RayHit;

use crate::behavior::Ctx;
use crate::weapons::{HitResponse, WeaponBehavior};
use crate::world::{proj_flags, Projectile};

/// Behaviour of the Ricochet Arc.
#[derive(Debug)]
pub struct RicochetArc;

impl WeaponBehavior for RicochetArc {
    fn id(&self) -> &'static str {
        "ricochet_arc"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        let def = ctx.def(p.weapon);
        let max = def.param("bounces", 3.0).max(0.0) as u8;
        let chip = def.param("chip_radius", 0.3);
        // `Impact` event (sparks) plus the data's crater and paint (none by default).
        ctx.detonate(p, hit.point, None, true);
        ctx.crater(hit.point, chip);
        if p.bounces < max {
            p.flags |= proj_flags::HITS_OWNER;
            HitResponse::Bounce
        } else {
            HitResponse::Stop
        }
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{BoxRoomTerrain, TerrainQuery};
    use ls_math::Vec3;

    use super::super::scrap_laser::testkit::*;
    use crate::world::proj_flags;
    use crate::{Event, ShipType};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn bounces_three_times_chipping_the_wall_each_time() {
        let (mut w, _, a, _b) = duel("ricochet_arc", ShipType::Tinker);
        // A room only 12 u deep: the bolt crosses it in 0.4 s.
        let mut t = SurfaceRoom::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 12.0)));
        // Slightly off the axis, so each bounce lands beside the last chip (a
        // bolt ending inside a chip would leave this room's bounds).
        place(
            &mut w,
            a,
            Vec3::new(-8.0, 0.0, 0.0),
            Vec3::new(0.15, 0.0, -1.0),
        );
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        let first_leg = w.projectiles()[0].flags;
        assert_eq!(
            first_leg & proj_flags::HITS_OWNER,
            0,
            "the first leg spares its owner"
        );
        // Get out of the way of the returning bolt.
        place(&mut w, a, Vec3::new(20.0, 0.0, 0.0), Vec3::NEG_Z);
        let mut max_bounces = 0;
        for _ in 0..120 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            if let Some(p) = w.projectiles().first() {
                max_bounces = max_bounces.max(p.bounces);
                if p.bounces > 0 {
                    assert_ne!(p.flags & proj_flags::HITS_OWNER, 0);
                }
            }
        }
        assert_eq!(max_bounces, 3);
        assert!(
            w.projectiles().is_empty(),
            "ends at the hit after the 3rd bounce"
        );
        let chips = craters(&ev);
        assert_eq!(chips.len(), 4, "a chip at every bounce and at the end");
        let impacts = ev
            .iter()
            .filter(|e| matches!(e, Event::Impact { .. }))
            .count();
        assert_eq!(impacts, 4);
        // Chips alternate between the two end walls and open the rock.
        assert!(chips[0].0.z < -5.0 && chips[1].0.z > 5.0 && chips[2].0.z < -5.0);
        let (c0, _) = chips[0];
        assert!(t.sdf(c0 + Vec3::new(0.0, 0.0, -0.1)) > 0.0, "chipped");
    }

    #[test]
    fn bounces_can_hit_you() {
        let (mut w, mut t, a, _b) = duel("ricochet_arc", ShipType::Tinker);
        // Straight at a wall 6 u ahead, and stay put.
        place(&mut w, a, Vec3::new(0.0, 0.0, -24.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        for _ in 0..40 {
            place(&mut w, a, Vec3::new(0.0, 0.0, -24.0), Vec3::NEG_Z);
            ev.extend(run(&mut w, &mut t, &[], 1));
        }
        assert!(damage_to(&ev, a) > 0.0, "the ricochet hit its owner");
        assert!(w.projectiles().is_empty());
    }

    #[test]
    fn bounced_bolts_still_hit_enemies() {
        let (mut w, mut t, a, b) = duel("ricochet_arc", ShipType::Tinker);
        // Bank a shot off the -Z wall at 45 degrees into an enemy around the corner.
        place(
            &mut w,
            a,
            Vec3::new(-10.0, 0.0, -20.0),
            Vec3::new(1.0, 0.0, -1.0),
        );
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        for _ in 0..90 {
            place(&mut w, b, Vec3::new(10.0, 0.0, -20.0), Vec3::Z);
            ev.extend(run(&mut w, &mut t, &[], 1));
        }
        assert!(damage_to(&ev, b) > 0.0);
        assert_eq!(craters(&ev).len(), 1, "one chip at the bank");
    }
}
