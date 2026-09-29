//! Swarm Pod: ordnance secondary. 30 u/s; releases 6 homing blobs on impact. Malus: blobs are
//! weak alone. Lasting effect: scatter of small craters.
//!
//! Numbers: `data/weapons/swarm_pod.ron` (`params`: `blobs`, `blob_speed`, `blob_damage`,
//! `blob_knockback`, `blob_radius`, `blob_lifetime_secs`, `blob_crater_radius`, `blob_range`,
//! `blob_scatter_secs`, `turn_rate`).
//!
//! Behaviour:
//! - The pod (generation 0) is a dumbfire missile with the generic impact (direct hit, small
//!   splash, small crater). On any impact (ship, debris or decoy, rock) it releases `blobs`
//!   sub-projectiles just outside the impact point, scattered in random directions over the
//!   hemisphere facing away from the surface (RNG keyed by the pod's id).
//! - Blobs (generation 1) fly straight for `blob_scatter_secs`, then home on the nearest enemy
//!   ship or enemy decoy within `blob_range` with a clear line of sight (any direction, see
//!   `seeker::acquire`), turning at most `turn_rate` rad/s. Each does only `blob_damage` (scaled
//!   like the pod's damage) and `blob_knockback`, no splash: weak alone.
//! - Every blob that hits rock digs a `blob_crater_radius` crater: together with the pod's own
//!   crater a scatter of small craters around the impact.

use ls_math::terrain::RayHit;
use ls_math::{Tick, Vec3, DT};

use crate::behavior::{Command, Ctx, DamageSource};
use crate::weapons::ordnance::seeker::{acquire, record_lock};
use crate::weapons::{
    default_entity_hit, steer_towards, sub_projectile, EntityHit, HitResponse, ShipHit,
    WeaponBehavior,
};
use crate::world::{Event, Projectile};

/// Behaviour of the Swarm Pod.
#[derive(Debug)]
pub struct SwarmPod;

/// Releases the blobs of pod `p` at `point`, scattered around `normal`.
fn release(ctx: &mut Ctx, p: &Projectile, point: Vec3, normal: Vec3) {
    let def = ctx.def(p.weapon);
    let count = def.param("blobs", 6.0).max(0.0) as u32;
    let speed = def.param("blob_speed", 15.0);
    let radius = def.param("blob_radius", 0.12);
    let lifetime = Tick::ticks_for_secs(def.param("blob_lifetime_secs", 3.0));
    // Blob damage scales like the pod's own (level, ship type, rule set tuning).
    let scale = if def.damage > 0.0 {
        p.damage / def.damage
    } else {
        1.0
    };
    let damage = def.param("blob_damage", 6.0) * scale;
    let knockback = def.param("blob_knockback", 1.5) * scale;
    let normal = normal.try_normalize().unwrap_or(Vec3::Y);
    let mut rng = ctx.rng(u64::from(p.id));
    for _ in 0..count {
        let r = Vec3::new(
            rng.range_f32(-1.0, 1.0),
            rng.range_f32(-1.0, 1.0),
            rng.range_f32(-1.0, 1.0),
        );
        let r = if r.dot(normal) < 0.0 {
            r - normal * (2.0 * r.dot(normal))
        } else {
            r
        };
        let dir = (normal * 0.35 + r).try_normalize().unwrap_or(normal);
        let pos = point + normal * (radius + 0.1);
        let mut blob = sub_projectile(ctx, p, pos, dir * speed, lifetime);
        blob.radius = radius;
        blob.damage = damage;
        blob.knockback = knockback;
        ctx.spawn_projectile(blob);
    }
}

/// A blob's impact: `Impact` event, its small damage, no splash.
fn blob_impact(ctx: &mut Ctx, p: &Projectile, point: Vec3, target: Option<crate::PlayerId>) {
    ctx.emit(Event::Impact {
        projectile: p.id,
        weapon: p.weapon,
        point,
        target,
    });
    if let Some(t) = target {
        let dir = p.vel.normalize_or_zero();
        ctx.damage(
            t,
            p.damage,
            dir * p.knockback,
            DamageSource::projectile(p),
            true,
        );
    }
}

impl WeaponBehavior for SwarmPod {
    fn id(&self) -> &'static str {
        "swarm_pod"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if p.generation == 0 {
            return;
        }
        let def = ctx.def(p.weapon);
        let scatter = Tick::ticks_for_secs(def.param("blob_scatter_secs", 0.15));
        if ctx.now() < p.spawn_tick + scatter {
            return;
        }
        let found = acquire(ctx, p, def.param("blob_range", 30.0), None);
        record_lock(p, found.map(|(lock, _)| lock));
        if let Some((_, pos)) = found {
            p.vel = steer_towards(p.vel, pos - p.pos, def.param("turn_rate", 4.0) * DT);
        }
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        if p.generation > 0 {
            blob_impact(ctx, p, hit.point, Some(hit.target));
        } else {
            ctx.detonate(p, hit.point, Some(hit.target), false);
            release(ctx, p, hit.point, hit.normal);
        }
        HitResponse::Stop
    }

    fn on_entity_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
        if p.generation > 0 {
            blob_impact(ctx, p, hit.point, None);
            ctx.push(Command::DamageEntity {
                id: hit.id,
                amount: p.damage,
                source: DamageSource::projectile(p),
            });
            ctx.impulse_entity(hit.id, p.vel.normalize_or_zero() * p.knockback);
            return HitResponse::Stop;
        }
        default_entity_hit(ctx, p, hit);
        release(ctx, p, hit.point, hit.normal);
        HitResponse::Stop
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        if p.generation > 0 {
            blob_impact(ctx, p, hit.point, None);
            let r = ctx.def(p.weapon).param("blob_crater_radius", 0.5);
            ctx.crater(hit.point, r);
        } else {
            ctx.detonate(p, hit.point, None, true);
            release(ctx, p, hit.point, hit.normal);
        }
        HitResponse::Stop
    }
}

#[cfg(test)]
mod tests {
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Bonus: 30 u/s pod that bursts into 6 blobs on impact; the blobs home
    /// on a ship near the impact that the pod itself missed.
    #[test]
    fn releases_six_homing_blobs_on_impact() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "swarm_pod");
        place(&mut w, a, Vec3::new(0.0, 0.0, -10.0), Vec3::NEG_Z);
        // Off to the side of the impact on the -Z wall, out of the splash.
        let spot = Vec3::new(8.0, 3.0, -26.0);
        place(&mut w, b, spot, Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        assert_eq!(w.projectiles()[0].vel.length(), 30.0);
        let mut blobs = 0;
        for _ in 0..200 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, spot, Vec3::Z);
            blobs = blobs.max(w.projectiles().iter().filter(|p| p.generation == 1).count());
        }
        assert_eq!(blobs, 6);
        let hits = hits_on(&ev, b);
        assert!(hits.len() >= 3, "blobs homed in: {hits:?}");
    }

    /// Malus: each blob is weak alone (6 damage, no splash), far less than
    /// the pod's own hit.
    #[test]
    fn blobs_are_weak_alone() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "swarm_pod");
        place(&mut w, a, Vec3::new(0.0, 0.0, -10.0), Vec3::NEG_Z);
        let spot = Vec3::new(8.0, 3.0, -26.0);
        place(&mut w, b, spot, Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        for _ in 0..200 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, spot, Vec3::Z);
        }
        let hits = hits_on(&ev, b);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|&d| d == 6.0), "{hits:?}");
        let pod = w.weapons().get(w.weapons().find("swarm_pod").unwrap());
        assert!(6.0 < pod.damage);

        // A direct pod hit: the pod's damage, then the blobs come back.
        let mut w = world();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "swarm_pod");
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        for _ in 0..200 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, Vec3::ZERO, Vec3::Z);
        }
        let hits = hits_on(&ev, b);
        assert_eq!(hits[0], 15.0, "pod direct hit {hits:?}");
        assert!(hits[1..].iter().all(|&d| d == 6.0), "{hits:?}");
    }

    /// Lasting effect: a scatter of small craters: the pod's own plus one
    /// per blob that lands on rock.
    #[test]
    fn scatter_of_small_craters() {
        let mut w = world();
        let mut t = room();
        // Nobody to home on: the blobs hit rock.
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "swarm_pod");
        place(&mut w, a, Vec3::new(0.0, -10.0, -24.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 300));
        let c = craters(&ev);
        assert_eq!(c[0].1, 0.75, "the pod's crater {c:?}");
        let small: Vec<_> = c[1..].iter().filter(|(_, r)| *r == 0.5).collect();
        assert!(small.len() >= 3, "{c:?}");
        // Scattered: not all in one spot.
        let spread = small
            .iter()
            .map(|(p, _)| p.distance(c[0].0))
            .fold(0.0f32, f32::max);
        assert!(spread > 3.0, "{c:?}");
        assert!(c.iter().all(|(_, r)| *r <= 0.75), "all small");
    }
}
