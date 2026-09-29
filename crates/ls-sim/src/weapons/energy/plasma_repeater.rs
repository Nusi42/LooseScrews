//! Plasma Repeater: energy primary. Design doc: "Rapid bolts at 33 u/s. Malus: high energy
//! drain. Lasting effect: melts shallow pits into terrain."
//!
//! Numbers: `data/weapons/plasma_repeater.ron` (`speed` 33, a short `fire_interval_secs` and
//! an energy `cost` that together drain far more than the regeneration; `params`:
//! `pit_radius`, `pit_every`, `hot_radius`, `hot_secs`).
//!
//! Behaviour: the generic straight bolt, fired rapidly. On terrain every `pit_every`-th bolt
//! of a burst (by `Shot::shots`, kept in `vars[0]`) melts a shallow pit (`ctx.crater`, scaled
//! by rock strength) so a sustained burst digs in while the edit rate stays bounded; every hit
//! leaves the rock hot (`SurfaceStates::HOT`).

use ls_math::terrain::{RayHit, SurfaceStates};

use super::scrap_laser::heat_surface;
use crate::behavior::Ctx;
use crate::weapons::{projectile_for, HitResponse, Shot, WeaponBehavior};
use crate::world::Projectile;

/// Behaviour of the Plasma Repeater.
#[derive(Debug)]
pub struct PlasmaRepeater;

impl WeaponBehavior for PlasmaRepeater {
    fn id(&self) -> &'static str {
        "plasma_repeater"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let mut p = projectile_for(ctx, shot, shot.shooter.orientation.forward());
        // The shot counter decides which bolts melt a pit (exact below 2^24).
        p.vars[0] = (shot.shots % (1 << 20)) as f32;
        ctx.spawn_projectile(p);
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, true);
        let def = ctx.def(p.weapon);
        let every = def.param("pit_every", 2.0).max(1.0) as u32;
        if (p.vars[0] as u32) % every == 0 {
            ctx.crater(hit.point, def.param("pit_radius", 0.4));
        }
        let radius = def.param("hot_radius", 0.6);
        let secs = def.param("hot_secs", 10.0);
        heat_surface(ctx, hit.point, radius, SurfaceStates::HOT, secs);
        HitResponse::Stop
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{SurfaceStates, TerrainQuery};
    use ls_math::Vec3;

    use super::super::scrap_laser::testkit::*;
    use crate::ShipType;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn rapid_bolts_at_33_units_per_second() {
        let (mut w, mut t, a, b) = duel("plasma_repeater", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 15.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let ev = run(&mut w, &mut t, &hold(a), 60);
        assert!(fired(&ev) >= 9, "about 10 bolts per second: {}", fired(&ev));
        for p in w.projectiles() {
            assert!((p.vel.length() - 33.0).abs() < 1e-3);
        }
        let ev = run(&mut w, &mut t, &[], 30);
        assert!(damage_to(&ev, b) > 0.0);
    }

    /// Energy a Tinker spends holding the trigger for 2 s (regeneration included).
    fn energy_used(weapon: &str) -> f32 {
        let (mut w, mut t, a, _b) = duel(weapon, ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 0.0), Vec3::X);
        let e0 = w.ship(a).unwrap().energy;
        run(&mut w, &mut t, &hold(a), 120);
        e0 - w.ship(a).unwrap().energy
    }

    #[test]
    fn high_energy_drain() {
        let plasma = energy_used("plasma_repeater");
        let laser = energy_used("scrap_laser");
        assert!(
            plasma > 3.0 * laser.max(1.0),
            "plasma {plasma}, laser {laser}"
        );
        assert!(plasma >= 30.0, "empties the tank within seconds: {plasma}");
    }

    #[test]
    fn melts_shallow_pits_and_leaves_hot_rock() {
        let (mut w, mut t, a, _b) = duel("plasma_repeater", ShipType::Tinker);
        // A rock face at z = -25 inside the room (the pits stay inside its bounds).
        t.inner.apply(ls_math::terrain::TerrainEdit::AddSphere {
            center: Vec3::new(0.0, 0.0, -45.0),
            radius: 20.0,
        });
        place(&mut w, a, Vec3::new(0.0, 0.0, -15.0), Vec3::NEG_Z);
        let wall = Vec3::new(0.0, 0.0, -25.0);
        assert!(t.sdf(wall + Vec3::new(0.0, 0.0, -0.1)) < 0.0);
        let mut ev = run(&mut w, &mut t, &hold(a), 30);
        ev.extend(run(&mut w, &mut t, &[], 30));
        let n = fired(&ev);
        let pits = craters(&ev);
        assert!(!pits.is_empty(), "melts pits");
        assert!(
            pits.len() * 2 >= n && pits.len() < n,
            "every 2nd bolt: {} of {n}",
            pits.len()
        );
        for (c, r) in &pits {
            assert!((c.z + 25.0).abs() < 2.0 && *r <= 0.5, "shallow: {c:?} {r}");
        }
        // The rock behind the wall surface is open now.
        assert!(t.sdf(wall + Vec3::new(0.0, 0.0, -0.1)) > 0.0);
        assert!(!surfaces(&ev, SurfaceStates::HOT).is_empty(), "hot rock");
    }
}
