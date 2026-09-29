//! Prox Mine: ordnance secondary. Dropped behind you, arms after 1 s. Malus: hurts you and
//! teammates. Lasting effect: sticks to walls; 3 u crater.
//!
//! Numbers: `data/weapons/prox_mine.ron` (`params`: `arm_secs`, `trigger_radius`,
//! `drop_behind`, `drift_drag`, `full_friendly_fire`).
//!
//! Status: implemented.
//! - `fire`: the mine is dropped `drop_behind` u behind the ship and drifts backwards at the
//!   data's `speed`, slowing down by `drift_drag` per second until it floats still.
//! - It sticks to any wall it drifts into (`HitResponse::Stick`).
//! - Armed `arm_secs` after the drop (`arm_tick`, replicated `ARMED` flag). Before that ships
//!   pass through it harmlessly.
//! - Armed, it goes off when any living ship comes within `trigger_radius` (centre distance)
//!   or touches it: enemies, teammates and its owner alike. The blast is the data's splash
//!   (`hits_owner`) plus the direct damage to a ship that touched it. With
//!   `full_friendly_fire` > 0 the mine ignores the rule set's friendly-fire scaling, so
//!   teammates take full damage (the malus); kill credit still never goes to a teammate. A
//!   rule set with friendly fire off (0%) still protects teammates.
//! - Near a wall (stuck, or within the crater radius of rock) it leaves the data's crater
//!   (radius 1.5 u = the design doc's 3 u diameter, scaled by rock strength).
//! - An unexploded mine fizzles at the end of its lifetime.

use ls_math::terrain::RayHit;
use ls_math::{Tick, Vec3, DT};

use crate::behavior::{Command, Ctx, DamageSource};
use crate::ship::PlayerId;
use crate::weapons::{projectile_for, EntityHit, HitResponse, ShipHit, Shot, WeaponBehavior};
use crate::world::{proj_flags, Projectile};
use crate::Event;

/// Behaviour of the Prox Mine.
#[derive(Debug)]
pub struct ProxMine;

fn armed(ctx: &Ctx, p: &Projectile) -> bool {
    ctx.now() >= p.arm_tick
}

/// The mine's blast at its position; `target` touched it (direct damage).
fn explode(ctx: &mut Ctx, p: &mut Projectile, target: Option<PlayerId>) {
    let def = ctx.def(p.weapon);
    let point = p.pos;
    let dir = p.vel.normalize_or_zero();
    let mut source = DamageSource::projectile(p);
    if def.param("full_friendly_fire", 1.0) > 0.0 && ctx.view.rules.friendly_fire_percent > 0 {
        // No team: the friendly-fire scale never applies (but a host that
        // switched friendly fire off keeps teammates safe).
        source.team = u8::MAX;
    }
    ctx.emit(Event::Impact {
        projectile: p.id,
        weapon: p.weapon,
        point,
        target,
    });
    if let Some(t) = target {
        let push = ctx
            .ship(t)
            .and_then(|s| (s.pos - point).try_normalize())
            .unwrap_or(dir);
        ctx.damage(t, p.damage, push * p.knockback, source, true);
    }
    if let Some(splash) = def.splash {
        ctx.push(Command::Splash {
            center: point,
            radius: splash.radius,
            damage: p.splash_damage,
            knockback: p.knockback,
            source,
            hits_owner: splash.hits_owner,
            exclude: target,
            dir,
        });
    }
    let crater = def.terrain.crater_radius;
    if crater > 0.0 && (p.is_stuck() || ctx.view.terrain.sdf(point) < crater) {
        ctx.crater(point, crater);
    }
    p.remove();
}

impl WeaponBehavior for ProxMine {
    fn id(&self) -> &'static str {
        "prox_mine"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let back = -shot.shooter.orientation.forward();
        let radius = ctx.tuning_of(shot.shooter).radius;
        let drop = shot.def.param("drop_behind", 1.0).max(0.0);
        let mut p = projectile_for(ctx, shot, back);
        p.pos = shot.shooter.pos + back * (radius + shot.def.projectile_radius + drop);
        p.arm_tick = ctx.now() + Tick::ticks_for_secs(shot.def.param("arm_secs", 1.0));
        ctx.spawn_projectile(p);
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if !p.is_stuck() {
            let drag = ctx.def(p.weapon).param("drift_drag", 1.0).max(0.0);
            p.vel *= (1.0 - drag * DT).max(0.0);
            if p.vel.length_squared() < 0.05 * 0.05 {
                p.vel = Vec3::ZERO;
            }
        }
        if !armed(ctx, p) {
            return;
        }
        p.flags |= proj_flags::ARMED | proj_flags::HITS_OWNER;
        let trigger = ctx.def(p.weapon).param("trigger_radius", 2.0);
        if ctx.nearest_ship(p.pos, trigger, |_| true).is_some() {
            explode(ctx, p, None);
        }
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        if armed(ctx, p) {
            explode(ctx, p, Some(hit.target));
            HitResponse::Stop
        } else {
            HitResponse::Continue
        }
    }

    fn on_entity_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
        if armed(ctx, p) {
            let source = DamageSource::projectile(p);
            let amount = p.damage;
            explode(ctx, p, None);
            ctx.push(Command::DamageEntity {
                id: hit.id,
                amount,
                source,
            });
            HitResponse::Stop
        } else {
            HitResponse::Continue
        }
    }

    fn on_terrain_hit(&self, _ctx: &mut Ctx, _p: &mut Projectile, _hit: &RayHit) -> HitResponse {
        HitResponse::Stick
    }

    fn on_fuse(&self, _ctx: &mut Ctx, _p: &mut Projectile) {}
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::TerrainEdit;
    use ls_math::Vec3;

    use crate::weapons::behavior;
    use crate::weapons::ordnance::heavy_rocket::testkit::*;
    use crate::world::proj_flags;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn mine(w: &World) -> Option<&Projectile> {
        let id = w.weapons().find("prox_mine").unwrap();
        w.projectiles().iter().find(|p| p.weapon == id)
    }

    #[test]
    fn dropped_behind_arms_after_1_s_and_floats_still() {
        let mut w = world();
        let mut t = room(80.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "prox_mine");
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        run(&mut w, &mut t, &fire2(a), 1);
        let fired = w.tick();
        let m = mine(&w).unwrap().clone();
        assert!(m.pos.z > 1.0, "behind the ship (it faces -Z): {:?}", m.pos);
        assert!(m.vel.z > 0.0, "drifting backwards");
        assert_eq!(m.arm_tick, fired + 60, "arms after 1 s");
        // Move the owner away; nobody near: it just floats.
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &[], 58);
        assert_eq!(mine(&w).unwrap().flags & proj_flags::ARMED, 0);
        run(&mut w, &mut t, &[], 2);
        assert_ne!(mine(&w).unwrap().flags & proj_flags::ARMED, 0);
        run(&mut w, &mut t, &[], 300);
        let m = mine(&w).unwrap();
        assert_eq!(m.vel, Vec3::ZERO, "drag stops the drift");
        assert!(m.pos.z < 8.0, "it drifts only a few units: {:?}", m.pos);
        assert!(behavior("prox_mine").implemented());
    }

    #[test]
    fn harmless_until_armed_then_triggers_on_proximity() {
        let mut w = world();
        let mut t = room(80.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, a, "prox_mine");
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &fire2(a), 1);
        let fired = w.tick();
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        // The enemy sits right on the unarmed mine: nothing happens.
        let pos = mine(&w).unwrap().pos;
        place(&mut w, b, pos + Vec3::new(0.3, 0.0, 0.0), Vec3::NEG_Z);
        let mut ev = Vec::new();
        for _ in 0..55 {
            let p = mine(&w).unwrap().pos;
            place(&mut w, b, p + Vec3::new(0.3, 0.0, 0.0), Vec3::NEG_Z);
            ev.extend(w.step(&mut t, &[]));
        }
        assert!(impacts(&ev).is_empty() && damage_to(&ev, b) == 0.0);
        // It goes off as soon as it is armed.
        ev = run_until(&mut w, &mut t, 10, |e| !impacts(e).is_empty());
        assert_eq!(w.tick(), fired + 60);
        assert!(damage_to(&ev, b) > 30.0, "{}", damage_to(&ev, b));
        assert!(mine(&w).is_none());
    }

    #[test]
    fn trigger_radius_counts_every_ship() {
        // (trigger distance, triggered?)
        for (d, expect) in [(1.8, true), (2.5, false)] {
            let mut w = world();
            let mut t = room(80.0);
            let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
            equip(&mut w, a, "prox_mine");
            place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
            run(&mut w, &mut t, &fire2(a), 1);
            place(&mut w, a, Vec3::new(0.0, 0.0, -30.0), Vec3::NEG_Z);
            run(&mut w, &mut t, &[], 300);
            let pos = mine(&w).unwrap().pos;
            // The owner flies back into its own mine.
            place(&mut w, a, pos + Vec3::new(d, 0.0, 0.0), Vec3::NEG_Z);
            let ev = run(&mut w, &mut t, &[], 2);
            assert_eq!(!impacts(&ev).is_empty(), expect, "at {d} u");
            if expect {
                assert!(damage_to(&ev, a) > 10.0, "hurts its owner");
            }
        }
    }

    #[test]
    fn hurts_teammates_at_full_damage() {
        // The same blast on an enemy and on a teammate at the same distance.
        let hurt = |team: u8, ff: Option<u8>| {
            let mut rules = RuleSet::paint_the_void();
            if let Some(ff) = ff {
                rules.friendly_fire_percent = ff;
            }
            let mut w = world_with(0, rules);
            let mut t = room(80.0);
            let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
            let v = w.add_player(Some(team), ShipType::Tinker).unwrap();
            equip(&mut w, a, "prox_mine");
            place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
            place(&mut w, v, Vec3::new(0.0, 20.0, 0.0), Vec3::NEG_Z);
            run(&mut w, &mut t, &fire2(a), 1);
            place(&mut w, a, Vec3::new(0.0, 0.0, -30.0), Vec3::NEG_Z);
            run(&mut w, &mut t, &[], 300);
            let pos = mine(&w).unwrap().pos;
            place(&mut w, v, pos + Vec3::new(1.5, 0.0, 0.0), Vec3::NEG_Z);
            let ev = run(&mut w, &mut t, &[], 2);
            damage_to(&ev, v)
        };
        assert_eq!(w_ff(), 50, "team modes scale friendly fire to 50%");
        let (enemy, mate) = (hurt(1, None), hurt(0, None));
        assert!(enemy > 10.0);
        assert!((enemy - mate).abs() < 1e-3, "{enemy} {mate}");
        // A host that switched friendly fire off keeps teammates safe.
        assert_eq!(hurt(0, Some(0)), 0.0);
        assert!((hurt(1, Some(0)) - enemy).abs() < 1e-3);
    }

    fn w_ff() -> u8 {
        world().rules().friendly_fire_percent
    }

    #[test]
    fn sticks_to_walls_and_leaves_a_3_u_crater() {
        let mut w = world();
        let mut t = room(40.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, a, "prox_mine");
        // Facing away from the +Z wall (z = 20): the mine drifts into it.
        place(&mut w, a, Vec3::new(0.0, 0.0, 16.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &fire2(a), 1);
        place(&mut w, a, Vec3::new(0.0, 0.0, -10.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &[], 120);
        let m = mine(&w).unwrap().clone();
        assert!(m.is_stuck(), "stuck to the wall");
        assert!((m.pos.z - (20.0 - m.radius)).abs() < 1e-3, "{:?}", m.pos);
        run(&mut w, &mut t, &[], 60);
        assert_eq!(mine(&w).unwrap().pos, m.pos, "stays put");
        place(&mut w, b, m.pos + Vec3::new(0.0, 0.0, -1.5), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &[], 2);
        let craters: Vec<_> = terrain_edits(&ev)
            .into_iter()
            .filter_map(|e| match e {
                TerrainEdit::SubtractSphere { center, radius } => Some((center, radius)),
                _ => None,
            })
            .collect();
        assert_eq!(craters.len(), 1);
        assert_eq!(craters[0].1, 1.5, "3 u diameter");
        assert!(damage_to(&ev, b) > 0.0);
    }
}
