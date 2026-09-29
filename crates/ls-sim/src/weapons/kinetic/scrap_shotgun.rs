//! Scrap Shotgun: kinetic primary. 8 pellets at 200 u/s. Malus: useless at range. Lasting
//! effect: pockmarks walls and sprays fresh debris.
//!
//! Numbers: `data/weapons/scrap_shotgun.ron` (`projectiles`, `spread_deg`; `params`:
//! `pockmark_radius`, `debris_per_shot`, `debris_radius`, `debris_speed`, `debris_scrap`,
//! `debris_hp`, `falloff_start`, `falloff_end`, `falloff_min`).
//!
//! Behaviour:
//! - `fire`: `projectiles` pellets, each in a random direction inside a cone of `spread_deg`
//!   (half angle, uniform over the cone's cross-section) around the aim; a fresh random
//!   pattern per shot (RNG keyed by the shooter and its shot counter).
//! - Malus: the short lifetime limits the range, the spread thins the pattern with distance,
//!   and pellet damage and knockback fall off linearly from full at `falloff_start` u to
//!   `falloff_min` of it at `falloff_end` u from the muzzle.
//! - Lasting effect: every pellet that hits rock digs a `pockmark_radius` pit; pellets with
//!   index `>= projectiles - debris_per_shot` that hit rock also spray a fresh debris piece
//!   (scavengeable scrap) off the wall along the pellet's reflection, unless
//!   `debris_crowd` pieces already float within 2 u (`weapons::debris_crowded`), so sustained
//!   fire does not fill the debris cap.
//!
//! Per-pellet state: `vars[0..3]` muzzle position, `vars[3]` pellet index in its shot.

use ls_math::terrain::RayHit;
use ls_math::{det, Quat, Vec3};

use crate::behavior::Ctx;
use crate::entity::EntitySpawn;
use crate::weapons::{debris_crowded, projectile_for, HitResponse, ShipHit, Shot, WeaponBehavior};
use crate::world::Projectile;
use crate::WeaponDef;

/// Behaviour of the Scrap Shotgun.
#[derive(Debug)]
pub struct ScrapShotgun;

/// Damage fraction of a pellet that hits `dist` u from the muzzle.
pub fn falloff(def: &WeaponDef, dist: f32) -> f32 {
    let start = def.param("falloff_start", 8.0);
    let end = def.param("falloff_end", 24.0);
    let min = def.param("falloff_min", 0.3);
    if dist <= start || end <= start {
        return 1.0;
    }
    let t = ((dist - start) / (end - start)).min(1.0);
    1.0 + (min - 1.0) * t
}

fn muzzle_of(p: &Projectile) -> Vec3 {
    Vec3::new(p.vars[0], p.vars[1], p.vars[2])
}

impl WeaponBehavior for ScrapShotgun {
    fn id(&self) -> &'static str {
        "scrap_shotgun"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let rot = shot.shooter.orientation;
        let fwd = rot.forward();
        let spread = shot.def.spread_deg.to_radians();
        let key = (u64::from(shot.shooter.player.0) << 32) | u64::from(shot.shots);
        let mut rng = ctx.rng(key);
        for i in 0..shot.def.projectiles.max(1) {
            // Uniform over the cone's cross-section: angle ~ sqrt(u).
            let angle = spread * det::sqrt(rng.next_f32());
            let (s, c) = det::sin_cos(std::f32::consts::TAU * rng.next_f32());
            let axis = rot.right() * c + rot.up() * s;
            let dir = Quat::from_axis_angle(axis, angle).rotate(fwd);
            let mut p = projectile_for(ctx, shot, dir);
            p.vars = [p.pos.x, p.pos.y, p.pos.z, f32::from(i)];
            ctx.spawn_projectile(p);
        }
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        let f = falloff(ctx.def(p.weapon), hit.point.distance(muzzle_of(p)));
        p.damage *= f;
        p.knockback *= f;
        ctx.detonate(p, hit.point, Some(hit.target), false);
        HitResponse::Stop
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        let def = ctx.def(p.weapon);
        ctx.detonate(p, hit.point, None, false);
        ctx.crater(hit.point, def.param("pockmark_radius", 0.25));
        // The last pellets of a shot spray the debris, so the rest of the
        // same shot (stepped earlier) does not shoot it apart at once.
        let pellets = f32::from(def.projectiles.max(1));
        let crowd = def.param("debris_crowd", 2.0) as usize;
        if p.vars[3] >= pellets - def.param("debris_per_shot", 2.0)
            && !debris_crowded(ctx, hit.point, crowd)
        {
            let radius = def.param("debris_radius", 0.3);
            let dir = p.vel.normalize_or_zero().reflect(hit.normal);
            let dir = (dir + hit.normal).try_normalize().unwrap_or(hit.normal);
            let mut chunk = EntitySpawn::debris(
                hit.point + hit.normal * (radius + 0.1),
                dir * def.param("debris_speed", 5.0),
                radius,
                def.param("debris_scrap", 0.5),
            );
            chunk.hp = chunk.hp.max(def.param("debris_hp", 10.0));
            ctx.spawn_entity(chunk);
        }
        HitResponse::Stop
    }
}

#[cfg(test)]
mod tests {
    use super::falloff;
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Bonus: 8 pellets at 200 u/s in a random cone; point blank most of
    /// them land at full damage.
    #[test]
    fn eight_pellet_blast_at_close_range() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        let gun = equip(&mut w, a, "scrap_shotgun");
        place(&mut w, a, Vec3::new(0.0, 0.0, 4.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire1(a), 1);
        assert_eq!(fired(&ev), 8);
        let fwd = Vec3::NEG_Z;
        let spread = w.weapons().get(gun).spread_deg;
        for p in w.projectiles() {
            assert!((p.vel.length() - 200.0).abs() < 1e-3);
            let angle = p
                .vel
                .normalize()
                .dot(fwd)
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            assert!(angle <= spread + 1e-3, "{angle}");
        }
        ev.extend(run(&mut w, &mut t, &[], 5));
        let hits = hits_on(&ev, b);
        assert!(hits.len() >= 5, "{hits:?}");
        assert!(
            hits.iter().all(|&d| d == 3.0),
            "full damage up close {hits:?}"
        );
    }

    /// Every shot sprays a fresh random pattern (deterministic per shot).
    #[test]
    fn new_pattern_every_shot() {
        let mut w = world();
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "scrap_shotgun");
        place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        let pattern = |w: &World| -> Vec<Vec3> { w.projectiles().iter().map(|p| p.vel).collect() };
        run(&mut w, &mut t, &fire1(a), 1);
        let first = pattern(&w);
        assert_eq!(first.len(), 8);
        run(&mut w, &mut t, &[], 60);
        run(&mut w, &mut t, &fire1(a), 1);
        let second = pattern(&w);
        assert_eq!(second.len(), 8);
        assert_ne!(first, second);
    }

    /// Malus: useless at range: pellets that connect at 20 u do a fraction
    /// of their damage, and nothing reaches 40 u.
    #[test]
    fn useless_at_range() {
        let w = world();
        let def = w.weapons().get(w.weapons().find("scrap_shotgun").unwrap());
        assert_eq!(falloff(def, 5.0), 1.0);
        assert!(falloff(def, 20.0) < 0.5);
        assert_eq!(falloff(def, 100.0), def.param("falloff_min", 0.0));

        for dist in [20.0f32, 40.0] {
            let mut w = world();
            let mut t = room();
            let (a, b) = duel(&mut w);
            equip(&mut w, a, "scrap_shotgun");
            let mut ev = Vec::new();
            for _ in 0..5 {
                place(&mut w, a, Vec3::new(0.0, 0.0, dist - 25.0), Vec3::NEG_Z);
                place(&mut w, b, Vec3::new(0.0, 0.0, -25.0), Vec3::Z);
                ev.extend(run(&mut w, &mut t, &fire1(a), 1));
                for _ in 0..60 {
                    ev.extend(run(&mut w, &mut t, &[], 1));
                    place(&mut w, b, Vec3::new(0.0, 0.0, -25.0), Vec3::Z);
                }
            }
            assert_eq!(fired(&ev), 40);
            let hits = hits_on(&ev, b);
            if dist >= 40.0 {
                assert!(hits.is_empty(), "out of range: {hits:?}");
            } else {
                assert!(!hits.is_empty(), "some pellets still connect");
                assert!(hits.iter().all(|&d| d < 3.0 * 0.6), "{hits:?}");
                assert!(hits.len() < 20, "the pattern thinned out: {}", hits.len());
            }
        }
    }

    /// Lasting effect: every pellet pockmarks the wall, and each shot sprays
    /// fresh, scavengeable debris.
    #[test]
    fn pockmarks_walls_and_sprays_debris() {
        let mut w = world();
        let mut t = room();
        let (a, _) = duel(&mut w);
        let gun = equip(&mut w, a, "scrap_shotgun");
        place(&mut w, a, Vec3::new(0.0, 0.0, -25.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &fire1(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 5));
        let pits = craters(&ev);
        // Every pellet lands: a pit, or (rarely) the fresh debris of another pellet.
        assert_eq!(impacts(&ev, gun).len(), 8);
        assert!(pits.len() >= 7, "{pits:?}");
        assert!(
            pits.iter()
                .all(|(c, r)| *r == 0.25 && (c.z + 30.0).abs() < 0.6),
            "{pits:?}"
        );
        let d = debris(&w);
        assert!((1..=2).contains(&d.len()), "{d:?}");
        assert!(d.iter().all(|e| e.scrap > 0.0 && e.pos.z > -30.0));
        assert!(d.iter().all(|e| e.vel.z > 0.0 || e.resting), "{d:?}");
    }
}
