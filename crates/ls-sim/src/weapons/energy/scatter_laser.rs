//! Scatter Laser: energy primary. Design doc: "3 bolts in an alternating fan. Malus: damage
//! spreads thin at range. Lasting effect: fan-shaped scorch pattern."
//!
//! Numbers: `data/weapons/scatter_laser.ron` (`projectiles`, `spread_deg`; `params`:
//! `fan_alternate`, `falloff_start`, `falloff_min`, `scorch_radius`, `hot_secs`).
//!
//! Behaviour:
//! - `fire`: 3 bolts `spread_deg` apart. With `fan_alternate` on, the fan alternates between a
//!   horizontal fan (even shots, `Shot::shots`) and a vertical fan (odd shots), like Descent's
//!   Spreadfire; with it off every fan is horizontal.
//! - Malus: the fan opens with range, so fewer bolts connect far away, and each bolt's damage
//!   and knockback fall linearly from full at `falloff_start` u of flight to `falloff_min` of
//!   it at the end of its lifetime.
//! - Lasting effect: every bolt scorches the rock it hits for good (`Ctx::scorch`: the
//!   permanent `SCORCH` state within `scorch_radius`, hot for `hot_secs`); a volley leaves a
//!   fan-shaped row of scorch marks.

use ls_math::terrain::RayHit;
use ls_math::{Quat, DT};

use crate::behavior::Ctx;
use crate::weapons::{projectile_for, HitResponse, ShipHit, Shot, WeaponBehavior};
use crate::world::Projectile;

/// Behaviour of the Scatter Laser.
#[derive(Debug)]
pub struct ScatterLaser;

/// Damage fraction of a bolt that has flown `dist` u (see the module docs).
fn falloff(dist: f32, start: f32, end: f32, min: f32) -> f32 {
    if dist <= start || end <= start {
        return 1.0;
    }
    let f = ((dist - start) / (end - start)).min(1.0);
    1.0 - (1.0 - min) * f
}

impl WeaponBehavior for ScatterLaser {
    fn id(&self) -> &'static str {
        "scatter_laser"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let rot = shot.shooter.orientation;
        let fwd = rot.forward();
        let vertical = shot.def.param("fan_alternate", 1.0) > 0.0 && shot.shots % 2 == 1;
        // A horizontal fan turns about the ship's up axis, a vertical one
        // about its right axis.
        let axis = if vertical { rot.right() } else { rot.up() };
        let n = shot.def.projectiles.max(1) as usize;
        let spread = shot.def.spread_deg.to_radians();
        for i in 0..n {
            let a = (i as f32 - (n - 1) as f32 * 0.5) * spread;
            let dir = Quat::from_axis_angle(axis, a).rotate(fwd);
            let p = projectile_for(ctx, shot, dir);
            ctx.spawn_projectile(p);
        }
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        let def = ctx.def(p.weapon);
        let flown = (ctx.now() + 1 - p.spawn_tick) as f32 * DT * def.speed;
        let f = falloff(
            flown,
            def.param("falloff_start", 8.0),
            def.speed * def.lifetime_secs,
            def.param("falloff_min", 0.4),
        );
        let mut thin = p.clone();
        thin.damage *= f;
        thin.knockback *= f;
        ctx.detonate(&thin, hit.point, Some(hit.target), false);
        HitResponse::Stop
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, true);
        let def = ctx.def(p.weapon);
        let radius = def.param("scorch_radius", 0.4);
        let secs = def.param("hot_secs", 10.0);
        // Permanent scorch marks (the fan pattern), hot for `hot_secs`.
        ctx.scorch(hit.point, radius, secs);
        HitResponse::Stop
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::SurfaceStates;
    use ls_math::Vec3;

    use super::super::scrap_laser::testkit::*;
    use crate::ShipType;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn fan_alternates_between_horizontal_and_vertical() {
        let (mut w, mut t, a, _b) = duel("scatter_laser", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        let wid = w.weapons().find("scatter_laser").unwrap();
        let interval = w.weapons().get(wid).fire_interval_ticks() as u32;
        let ev = run(&mut w, &mut t, &hold(a), 1);
        assert_eq!(fired(&ev), 3);
        let first: Vec<Vec3> = w.projectiles().iter().map(|p| p.vel).collect();
        // Horizontal: spread in x, none in y.
        assert!(first.iter().all(|v| v.y.abs() < 1e-4));
        assert!(first.iter().any(|v| v.x > 1.0) && first.iter().any(|v| v.x < -1.0));
        for v in &first {
            assert!((v.length() - 30.0).abs() < 1e-3, "laser speed");
        }
        run(&mut w, &mut t, &hold(a), interval - 1);
        let before: Vec<u32> = w.projectiles().iter().map(|p| p.id).collect();
        let ev = run(&mut w, &mut t, &hold(a), 1);
        assert_eq!(fired(&ev), 3);
        let second: Vec<Vec3> = w
            .projectiles()
            .iter()
            .filter(|p| !before.contains(&p.id))
            .map(|p| p.vel)
            .collect();
        assert_eq!(second.len(), 3);
        // Vertical: spread in y, none in x.
        assert!(second.iter().all(|v| v.x.abs() < 1e-4));
        assert!(second.iter().any(|v| v.y > 1.0) && second.iter().any(|v| v.y < -1.0));
    }

    /// Damage one volley deals to a Tinker `dist` u ahead.
    fn volley_damage(dist: f32) -> (f32, usize) {
        let (mut w, mut t, a, b) = duel("scatter_laser", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, 25.0 - dist), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 100));
        (damage_to(&ev, b), hits_on(&ev, b))
    }

    #[test]
    fn damage_spreads_thin_at_range() {
        let (near, near_hits) = volley_damage(4.0);
        let (mid, _) = volley_damage(20.0);
        let (far, far_hits) = volley_damage(45.0);
        assert_eq!(near_hits, 3, "point blank: the whole fan connects");
        assert!((near - 9.0).abs() < 1e-3, "3 x 3 at point blank: {near}");
        assert_eq!(far_hits, 1, "far away only the centre bolt connects");
        assert!(mid < near && far < mid, "{near} > {mid} > {far}");
        assert!(far < 3.0 * 0.8, "the far bolt itself is weaker: {far}");
    }

    #[test]
    fn volley_leaves_a_fan_of_scorch_marks() {
        let (mut w, mut t, a, _b) = duel("scatter_laser", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 0.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 90));
        let hot = surfaces(&ev, SurfaceStates::HOT);
        assert_eq!(hot.len(), 3, "one scorch per bolt");
        let mut xs: Vec<f32> = hot.iter().map(|(c, _, _)| c.x).collect();
        xs.sort_by(f32::total_cmp);
        for (c, _, _) in &hot {
            assert!((c.z + 30.0).abs() < 0.3, "on the far wall");
        }
        // Fan-shaped: left, centre, right, about tan(8 deg) * 30 u apart.
        assert!(xs[0] < -3.0 && xs[1].abs() < 0.1 && xs[2] > 3.0, "{xs:?}");
    }
}
