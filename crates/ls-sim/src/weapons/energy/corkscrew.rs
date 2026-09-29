//! Corkscrew: energy primary. Design doc: "5 bolts in a rotating fan. Malus: wide pattern
//! wastes energy at long range. Lasting effect: etches spiral grooves into walls."
//!
//! Numbers: `data/weapons/corkscrew.ron` (`projectiles` 5, `spread_deg` the angle between
//! neighbouring bolts; `params`: `spin_hz`, `shot_rotate_deg`, `groove_radius`, `groove_step`,
//! `groove_segments`, `max_radius`).
//!
//! Behaviour:
//! - `fire`: the 5 bolts form a fan (a line across the aim axis: centre, two inner, two outer
//!   bolts) that starts rotated by `shot_rotate_deg` per shot (`Shot::shots`).
//! - `update`: the fan spins around the shot axis at `spin_hz` while it flies: bolt `k` of the
//!   line sits at radius `min(|k| * tan(spread_deg) * s, max_radius)` after `s` u of flight
//!   (the fan widens like a cone, then stays a tube so outer bolts keep near laser speed) and
//!   angle `theta0 + 2 pi spin_hz t`, so each off-centre bolt draws a conical helix.
//!   The shot axis, its right vector and the muzzle point live in `path`, the start angle and
//!   radius factor in `vars[0..2]`. The bolt's velocity is set each tick to reach its helix
//!   point, so the framework's swept hits follow the helix.
//! - Malus: the fan opens with range, so at long range only the centre bolt connects and the
//!   other four bolts' energy is wasted.
//! - Hits knock back along the shot axis (not the helix tangent).
//! - Lasting effect: every off-centre bolt etches a short groove into the wall it hits
//!   (`groove_segments` small craters of `groove_radius`, `groove_step` apart, along the bolt's
//!   spin direction projected onto the wall); a volley's grooves wind around the aim point,
//!   and successive volleys (rotated per shot) add up to a spiral. Groove craters whose centre
//!   is already open are skipped, so sustained fire at one spot adds no edits.

use ls_math::terrain::RayHit;
use ls_math::{det, Vec3, DT};

use crate::behavior::Ctx;
use crate::weapons::{projectile_for, HitResponse, ShipHit, Shot, WeaponBehavior};
use crate::world::Projectile;

/// Behaviour of the Corkscrew.
#[derive(Debug)]
pub struct Corkscrew;

/// Spin angle of `p` (start angle plus spin) after `age` seconds.
fn spin_angle(ctx: &Ctx, p: &Projectile, age: f32) -> f32 {
    let spin_hz = ctx.def(p.weapon).param("spin_hz", 1.0);
    p.vars[0] + std::f32::consts::TAU * spin_hz * age
}

/// The shot frame kept in `path`: (muzzle, axis, right, up).
fn frame(p: &Projectile) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
    match p.path.as_slice() {
        [origin, axis, right, ..] => Some((*origin, *axis, *right, axis.cross(*right))),
        _ => None,
    }
}

impl WeaponBehavior for Corkscrew {
    fn id(&self) -> &'static str {
        "corkscrew"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let rot = shot.shooter.orientation;
        let axis = rot.forward();
        let right = rot.right();
        let up = axis.cross(right);
        let per_shot = shot.def.param("shot_rotate_deg", 36.0);
        let phase = ((shot.shots % 3600) as f32 * per_shot).to_radians();
        let tan = det::tan(shot.def.spread_deg.to_radians());
        let n = shot.def.projectiles.max(1) as usize;
        for i in 0..n {
            let k = i as f32 - (n - 1) as f32 * 0.5;
            let theta0 = if k < 0.0 {
                phase + std::f32::consts::PI
            } else {
                phase
            };
            let factor = k.abs() * tan;
            let mut p = projectile_for(ctx, shot, axis);
            let (s, c) = det::sin_cos(theta0);
            let radial = right * c + up * s;
            p.vel = (axis + radial * factor) * shot.def.speed;
            p.path = vec![p.pos, axis, right];
            p.vars[0] = theta0;
            p.vars[1] = factor;
            ctx.spawn_projectile(p);
        }
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let Some((origin, axis, right, up)) = frame(p) else {
            return;
        };
        // Where the bolt is after this tick's move.
        let age = (ctx.now() + 1).saturating_sub(p.spawn_tick) as f32 * DT;
        let s = ctx.def(p.weapon).speed * age;
        let (sin, cos) = det::sin_cos(spin_angle(ctx, p, age));
        let radial = right * cos + up * sin;
        // The fan stops widening at `max_radius`, so outer bolts keep about
        // laser speed (spin speed 2 pi spin_hz r) instead of racing away.
        let max_radius = ctx.def(p.weapon).param("max_radius", 4.0);
        let r = (p.vars[1] * s).min(max_radius);
        let target = origin + axis * s + radial * r;
        p.vel = (target - p.pos) * (1.0 / DT);
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        // Knock back along the shot axis, not along the helix tangent (an
        // outer bolt's velocity is mostly sideways).
        let mut along = p.clone();
        if let Some((_, axis, _, _)) = frame(p) {
            along.vel = axis * p.vel.length();
        }
        ctx.detonate(&along, hit.point, Some(hit.target), false);
        HitResponse::Stop
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, true);
        let def = ctx.def(p.weapon);
        let radius = def.param("groove_radius", 0.25);
        let step = def.param("groove_step", 0.3);
        let segments = def.param("groove_segments", 2.0).clamp(0.0, 8.0) as u32;
        // The centre bolt draws no groove (it hits the aim point every
        // volley): the spiral comes from the four spinning bolts.
        if radius <= 0.0 || segments == 0 || p.vars[1] == 0.0 {
            return HitResponse::Stop;
        }
        // Spin direction (d radial / d angle = axis x radial), laid onto the wall.
        let age = ctx.now().saturating_sub(p.spawn_tick) as f32 * DT;
        let tangent = frame(p).map_or(Vec3::ZERO, |(_, axis, right, up)| {
            let (sin, cos) = det::sin_cos(spin_angle(ctx, p, age));
            axis.cross(right * cos + up * sin)
        });
        let n = hit.normal;
        let along = (tangent - n * tangent.dot(n))
            .try_normalize()
            .unwrap_or_else(|| n.any_orthonormal());
        for j in 0..segments {
            let c = hit.point + along * (step * j as f32);
            // Skip craters whose centre is already carved open: sustained
            // fire at one spot adds no edits (every edit is networked).
            if ctx.view.terrain.sdf(c - n * (radius * 0.5)) > 0.0 {
                continue;
            }
            ctx.crater(c, radius);
        }
        HitResponse::Stop
    }
}

#[cfg(test)]
mod tests {
    use ls_math::{det, Vec3};

    use super::super::scrap_laser::testkit::*;
    use crate::ShipType;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Angle of `p` around the -Z aim axis through `(0, 0, z0)`.
    fn angle(p: Vec3) -> f32 {
        det::atan2(p.y, p.x)
    }

    #[test]
    fn five_bolt_fan_rotates_in_flight_and_per_shot() {
        let (mut w, mut t, a, _b) = duel("corkscrew", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &hold(a), 1);
        assert_eq!(fired(&ev), 5);
        run(&mut w, &mut t, &[], 9);
        let early: Vec<Vec3> = w.projectiles().iter().map(|p| p.pos).collect();
        run(&mut w, &mut t, &[], 15);
        let late: Vec<Vec3> = w.projectiles().iter().map(|p| p.pos).collect();
        assert_eq!(early.len(), 5);
        // Bolt 0 is an outer bolt: it turned by spin_hz * 15 ticks (90 deg at 1 Hz).
        let turned = det::wrap_angle(angle(late[0]) - angle(early[0]));
        assert!(
            (turned.abs() - std::f32::consts::FRAC_PI_2).abs() < 0.05,
            "{turned}"
        );
        // It flies away from the axis (the fan widens) and keeps laser speed along it.
        let r = |p: Vec3| (p.x * p.x + p.y * p.y).sqrt();
        assert!(r(late[0]) > r(early[0]) * 1.5);
        assert!(r(late[2]) < 1e-3, "centre bolt stays on the axis");
        assert!((late[2].z - early[2].z + 15.0 * 30.0 / 60.0).abs() < 1e-3);
        // The fan is a line: the two outer bolts are opposite each other.
        let opposite = det::wrap_angle(angle(late[0]) - angle(late[4]));
        assert!(
            (opposite.abs() - std::f32::consts::PI).abs() < 1e-2,
            "{opposite}"
        );

        // The next shot starts rotated by shot_rotate_deg (36 deg).
        let (mut w, mut t, a, _b) = duel("corkscrew", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        let wid = w.weapons().find("corkscrew").unwrap();
        w.ship_mut(a).unwrap().weapon_mut(wid).unwrap().shots = 1;
        run(&mut w, &mut t, &hold(a), 1);
        run(&mut w, &mut t, &[], 9);
        let rotated = w.projectiles()[0].pos;
        let d = det::wrap_angle(angle(rotated) - angle(early[0]));
        assert!((d.abs() - 36f32.to_radians()).abs() < 0.02, "{d}");
    }

    /// Bolts of one volley that hit a Tinker `dist` u ahead.
    fn hits_at(dist: f32) -> (usize, f32) {
        let (mut w, mut t, a, b) = duel("corkscrew", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        for _ in 0..120 {
            place(&mut w, b, Vec3::new(0.0, 0.0, 25.0 - dist), Vec3::Z);
            ev.extend(run(&mut w, &mut t, &[], 1));
        }
        (hits_on(&ev, b), damage_to(&ev, b))
    }

    #[test]
    fn wide_pattern_wastes_energy_at_long_range() {
        let (near, dn) = hits_at(3.0);
        let (far, df) = hits_at(40.0);
        assert!(near >= 3, "point blank: most of the fan connects ({near})");
        assert_eq!(far, 1, "far away only the centre bolt connects");
        assert!(dn > 2.5 * df, "{dn} vs {df}");
    }

    #[test]
    fn etches_spiral_grooves_into_walls() {
        let (mut w, mut t, a, _b) = duel("corkscrew", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 0.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 90));
        let c = craters(&ev);
        assert_eq!(c.len(), 8, "4 spinning bolts x 2-crater grooves");
        for (p, r) in &c {
            assert!((p.z + 30.0).abs() < 0.5, "on the wall: {p:?}");
            assert!(*r > 0.0 && *r < 0.5);
        }
        // Each groove runs around the aim point (tangential), not outward.
        for pair in c.chunks(2) {
            assert_eq!(pair.len(), 2);
            let (a0, a1) = (pair[0].0, pair[1].0);
            let along = (a1 - a0).normalize();
            let radial = Vec3::new(a0.x, a0.y, 0.0);
            if radial.length() > 1.0 {
                assert!(
                    along.dot(radial.normalize()).abs() < 0.3,
                    "tangential groove"
                );
            }
        }
        // Outer and inner grooves sit at different radii and angles: a spiral, not a line.
        let radii: Vec<f32> = c
            .iter()
            .step_by(2)
            .map(|(p, _)| Vec3::new(p.x, p.y, 0.0).length())
            .collect();
        // Outer bolts sit at `max_radius` (4 u), inner ones at about 3.15 u.
        assert!(radii.iter().any(|r| *r > 3.8) && radii.iter().any(|r| *r > 1.0 && *r < 3.5));
        assert!(radii.iter().all(|r| *r < 4.3), "{radii:?}");
    }
}
