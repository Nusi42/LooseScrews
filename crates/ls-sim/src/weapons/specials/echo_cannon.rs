//! Echo Cannon: special primary. Projectile returns along the same path after 2 s, so it can
//! hit twice. Malus: the returning shot can hit you. Lasting effect: scorch trail along the
//! out-and-back path.
//!
//! Numbers: `data/weapons/echo_cannon.ron` (`params`: `return_after_secs`, `record_ticks`,
//! `scorch_reach`, `scorch_radius`, `scorch_hot_secs`, `scorch_every`, `max_scorch_edits`).
//!
//! Status: implemented.
//! - Out leg: `update` records a path point every `record_ticks` into `Projectile::path`.
//!   A ship hit deals the direct damage and the shot flies on (`Continue`), so the same
//!   target can be hit again on the way back. A terrain hit scorches the wall and the shot
//!   waits there (`Stick`) until it turns.
//! - After `return_after_secs` the shot sets `RETURNING | HITS_OWNER` (replicated) and flies
//!   back through the recorded points in reverse at its own speed; past the first point
//!   (the muzzle) it keeps going, so an owner who stayed put is hit by their own echo
//!   (self damage, which the Shield cuts by 75 %). The return leg stops at the first ship
//!   or wall it hits.
//! - Scorch trail: every `scorch_every`-th recorded path point within `scorch_reach` of rock
//!   scorches the rock for good (`Ctx::scorch`: the permanent `SCORCH` state, hot for
//!   `scorch_hot_secs`; the return leg flies the same path, so it is the out-and-back trail),
//!   at most `max_scorch_edits` per shot (counted in `vars[0]`), plus every wall impact (at
//!   most two: out and back; an already scorched spot adds no edit).

use ls_math::terrain::RayHit;
use ls_math::{Tick, Vec3, DT};

use crate::behavior::Ctx;
use crate::weapons::{HitResponse, ShipHit, WeaponBehavior};
use crate::world::{proj_flags, Projectile};
use crate::WeaponDef;

/// Behaviour of the Echo Cannon.
#[derive(Debug)]
pub struct EchoCannon;

fn return_ticks(def: &WeaponDef) -> u64 {
    Tick::ticks_for_secs(def.param("return_after_secs", 2.0)).max(1)
}

/// Scorches the rock near `point` if it is close enough (one scorch-trail mark).
fn scorch(ctx: &mut Ctx, p: &mut Projectile, point: Vec3, force: bool) {
    let def = ctx.def(p.weapon);
    let max = def.param("max_scorch_edits", 12.0);
    if !force && p.vars[0] >= max {
        return;
    }
    let reach = def.param("scorch_reach", 1.5);
    if !force && ctx.view.terrain.sdf(point) > reach {
        return;
    }
    let radius = def.param("scorch_radius", 1.0);
    let hot_secs = def.param("scorch_hot_secs", 10.0);
    // Permanent scorch (hot for a while); an already scorched spot adds no edit.
    ctx.scorch(point, radius, hot_secs);
    p.vars[0] += 1.0;
}

impl WeaponBehavior for EchoCannon {
    fn id(&self) -> &'static str {
        "echo_cannon"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let def = ctx.def(p.weapon);
        let age = ctx.now().saturating_sub(p.spawn_tick);
        let speed = def.speed;
        if p.flags & proj_flags::RETURNING == 0 {
            let every = (def.param("record_ticks", 4.0) as u64).max(1);
            if !p.is_stuck() && age % every == 0 {
                let pos = p.pos;
                p.path.push(pos);
                // Scorch every `scorch_every`-th recorded point (edit rate).
                let scorch_every = (def.param("scorch_every", 2.0) as usize).max(1);
                if age > 0 && p.path.len() % scorch_every == 0 {
                    scorch(ctx, p, pos, false);
                }
            }
            if age < return_ticks(def) {
                return;
            }
            // Turn around.
            p.flags |= proj_flags::RETURNING | proj_flags::HITS_OWNER;
            p.passed = None;
            if p.is_stuck() {
                p.flags &= !proj_flags::STUCK;
                // Step off the wall towards the way back.
                if let Some(back) = p.path.last().and_then(|&q| (q - p.pos).try_normalize()) {
                    p.pos += back * (p.radius + 0.05);
                }
            } else {
                // The current position is the last recorded point already or
                // close to it; start from the one before if it is here.
                if p.path.last().is_some_and(|&q| q.distance(p.pos) < 1e-3) {
                    p.path.pop();
                }
            }
            if let Some(back) = p.path.last().and_then(|&q| (q - p.pos).try_normalize()) {
                p.vel = back * speed;
            } else {
                p.vel = -p.vel;
            }
        }
        // Return leg: follow the recorded points in reverse.
        let step = speed * DT;
        while let Some(&q) = p.path.last() {
            if q.distance(p.pos) > step {
                break;
            }
            p.path.pop();
        }
        if let Some(dir) = p.path.last().and_then(|&q| (q - p.pos).try_normalize()) {
            p.vel = dir * speed;
        }
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        ctx.detonate(p, hit.point, Some(hit.target), false);
        if p.flags & proj_flags::RETURNING == 0 {
            HitResponse::Continue
        } else {
            HitResponse::Stop
        }
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, true);
        scorch(ctx, p, hit.point, true);
        if p.flags & proj_flags::RETURNING == 0 {
            p.pos = hit.point;
            HitResponse::Stick
        } else {
            HitResponse::Stop
        }
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{SurfaceStates, TerrainEdit};
    use ls_math::Vec3;

    use super::super::paint_thrower::testkit::*;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn hot_edits(ev: &[Event]) -> Vec<Vec3> {
        edits(ev)
            .into_iter()
            .filter_map(|e| match e {
                TerrainEdit::Surface { center, states, .. }
                    if SurfaceStates(states).contains(SurfaceStates::SCORCH) =>
                {
                    Some(center)
                }
                _ => None,
            })
            .collect()
    }

    /// Fires one shot (primary held for one tick).
    fn shoot(w: &mut World, t: &mut TestRoom, a: PlayerId) -> Vec<Event> {
        run(w, t, &press(a, Buttons::FIRE_PRIMARY), 1)
    }

    #[test]
    fn hits_a_target_on_the_way_out_and_again_on_the_way_back() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "echo_cannon");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, 5.0), Vec3::Z);
        let mut ev = shoot(&mut w, &mut t, a);
        assert_eq!(fired(&ev), 1);
        // Move the owner out of the way of the return leg.
        place(&mut w, a, Vec3::new(10.0, 0.0, 20.0), Vec3::NEG_Z);
        ev.extend(run(&mut w, &mut t, &[], 40));
        let dmg = w
            .weapons()
            .get(w.weapons().find("echo_cannon").unwrap())
            .damage;
        let out = damage_to(&ev, b);
        assert!((out - dmg).abs() < 1e-3, "first pass {out}");
        let p = &w.projectiles()[0];
        assert_eq!(p.flags & proj_flags::RETURNING, 0);
        assert!(!p.path.is_empty(), "records its path");
        // Target stays put; the echo comes back through it.
        place(&mut w, b, Vec3::new(0.0, 0.0, 5.0), Vec3::Z);
        let mut back = Vec::new();
        for _ in 0..200 {
            place(&mut w, b, Vec3::new(0.0, 0.0, 5.0), Vec3::Z);
            back.extend(run(&mut w, &mut t, &[], 1));
        }
        let second = damage_to(&back, b);
        assert!((second - dmg).abs() < 1e-3, "second pass {second}");
        assert_eq!(damage_to(&back, a), 0.0, "owner out of the path");
    }

    #[test]
    fn returns_along_the_recorded_path_after_two_seconds() {
        let (mut w, a, _b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "echo_cannon");
        place(&mut w, a, Vec3::new(-5.0, 0.0, 25.0), Vec3::NEG_Z);
        shoot(&mut w, &mut t, a);
        let start = w.projectiles()[0].pos;
        // Out for 2 s at 30 u/s: stays unreturned until then.
        place(&mut w, a, Vec3::new(10.0, 0.0, 25.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &[], 100);
        let p = w.projectiles()[0].clone();
        assert_eq!(p.flags & proj_flags::RETURNING, 0);
        assert!(p.vel.z < 0.0);
        run(&mut w, &mut t, &[], 25);
        let p = w.projectiles()[0].clone();
        assert_ne!(p.flags & proj_flags::RETURNING, 0);
        assert_ne!(p.flags & proj_flags::HITS_OWNER, 0);
        assert!(p.vel.z > 0.0, "flies back: {:?}", p.vel);
        assert!((p.pos.x - start.x).abs() < 1e-3, "on the same line");
        // Back near the muzzle about 2 s after turning.
        run(&mut w, &mut t, &[], 100);
        let p = w.projectiles()[0].clone();
        assert!(p.pos.distance(start) < 3.0, "{:?} vs {:?}", p.pos, start);
    }

    #[test]
    fn returning_shot_hits_its_owner_and_the_shield_cuts_it() {
        let dmg_with = |shield: bool| {
            let (mut w, a, _b) = world2();
            let mut t = TestRoom::new();
            equip(&mut w, a, "echo_cannon");
            if shield {
                w.give_gadget(a, "shield").unwrap();
                let s = w.ship_mut(a).unwrap();
                s.gadget_active = true;
            }
            place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
            let mut ev = shoot(&mut w, &mut t, a);
            for _ in 0..280 {
                place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
                ev.extend(run(&mut w, &mut t, &[], 1));
            }
            (damage_to(&ev, a), 100.0 - w.ship(a).unwrap().hull)
        };
        let (plain, hull_lost) = dmg_with(false);
        assert!(plain > 0.0, "the echo hits its owner");
        assert!(hull_lost > 0.0);
        let (_, shielded_hull) = dmg_with(true);
        assert!(
            shielded_hull < hull_lost * 0.5,
            "{shielded_hull} vs {hull_lost}"
        );
    }

    #[test]
    fn wall_hit_waits_then_returns_and_leaves_a_scorch_trail() {
        let (mut w, a, b) = world2();
        // A wall 20 u ahead; the shot flies along 1 u from the floor-less
        // side wall at x = 29 (grazing it) so the trail scorches.
        let mut t = TestRoom::new().with_wall_z(-5.0, 2.0);
        equip(&mut w, a, "echo_cannon");
        place(&mut w, a, Vec3::new(28.8, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(-20.0, 0.0, 0.0), Vec3::Z);
        let mut ev = shoot(&mut w, &mut t, a);
        place(&mut w, a, Vec3::new(20.0, 0.0, 20.0), Vec3::NEG_Z);
        ev.extend(run(&mut w, &mut t, &[], 60));
        let p = w.projectiles()[0].clone();
        assert!(p.is_stuck(), "waits at the wall");
        assert!((p.pos.z - -4.0).abs() < 0.3, "{:?}", p.pos);
        ev.extend(run(&mut w, &mut t, &[], 70));
        let p = w.projectiles()[0].clone();
        assert!(!p.is_stuck());
        assert!(p.vel.z > 0.0, "returning from the wall");
        ev.extend(run(&mut w, &mut t, &[], 200));
        assert!(w.projectiles().is_empty());
        let hot = hot_edits(&ev);
        assert!(hot.len() >= 3, "scorch trail: {hot:?}");
        assert!(
            hot.iter().any(|c| (c.z - -4.0).abs() < 0.2),
            "impact scorch"
        );
        assert!(hot.iter().any(|c| c.z > 5.0), "trail along the path");
        let max = w
            .weapons()
            .get(w.weapons().find("echo_cannon").unwrap())
            .param("max_scorch_edits", 0.0);
        // Trail marks are capped; wall impacts (out and back) always scorch.
        assert!(hot.len() as f32 <= max + 2.0, "{}", hot.len());
        // No geometry is carved (scorch only).
        assert!(edits(&ev)
            .iter()
            .all(|e| matches!(e, TerrainEdit::Surface { .. })));
    }
}
