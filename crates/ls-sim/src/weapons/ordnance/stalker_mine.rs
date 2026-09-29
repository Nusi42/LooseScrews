//! Stalker Mine: ordnance secondary. Creeps toward ships within 10 u. Malus: slow. Lasting
//! effect: leaves a scorched trail along its path.
//!
//! Numbers: `data/weapons/stalker_mine.ron` (`params`: `arm_secs`, `detect_radius`,
//! `creep_speed`, `trigger_radius`, `launch_drag`, `scorch_spacing`, `scorch_radius`,
//! `scorch_secs`).
//!
//! Status: implemented.
//! - Launched forward at the data's `speed`, it slows by `launch_drag` per second until it
//!   floats still, and sticks to any wall it reaches.
//! - Armed after `arm_secs`. Armed, it hunts the nearest living enemy within
//!   `detect_radius` that it can see (terrain raycast): it pulls loose from its wall and
//!   creeps straight at the target at `creep_speed` (4 u/s, a third of a ship's cruise speed:
//!   the malus, a pilot who notices it simply flies away). It ignores its owner and
//!   teammates (it passes through them) but its blast does not.
//! - It goes off when the target is within `trigger_radius` (centre distance) or it touches
//!   an enemy: direct damage to a touched ship plus the data's splash (`hits_owner`) and, near
//!   rock, the data's crater.
//! - Lasting effect: every `scorch_spacing` u it creeps within `scorch_radius` of rock, it
//!   scorches the surface there (`TerrainEdit::Surface` with the `HOT` state for
//!   `scorch_secs`: there is no permanent scorch surface state yet, so the trail is a long
//!   lasting hot mark).
//! - When it loses its target it stops where it is and waits; without one it coasts to a stop
//!   (`launch_drag`; a Magnet Sneeze pull carries it along) or stays stuck. It fizzles at the
//!   end of its lifetime.

use ls_math::terrain::{RayHit, SurfaceStates, TerrainEdit};
use ls_math::{Tick, Vec3, DT};

use crate::behavior::{Command, Ctx, DamageSource};
use crate::ship::PlayerId;
use crate::weapons::{projectile_for, EntityHit, HitResponse, ShipHit, Shot, WeaponBehavior};
use crate::world::{proj_flags, Projectile};
use crate::Event;

/// Behaviour of the Stalker Mine.
#[derive(Debug)]
pub struct StalkerMine;

/// `vars` slot: distance crept since the last scorch mark.
const VAR_CREPT: usize = 0;

fn explode(ctx: &mut Ctx, p: &mut Projectile, target: Option<PlayerId>) {
    let def = ctx.def(p.weapon);
    let point = p.pos;
    let dir = p.vel.normalize_or_zero();
    let source = DamageSource::projectile(p);
    ctx.emit(Event::Impact {
        projectile: p.id,
        weapon: p.weapon,
        point,
        target,
    });
    if let Some(t) = target {
        ctx.damage(t, p.damage, dir * p.knockback, source, true);
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

impl WeaponBehavior for StalkerMine {
    fn id(&self) -> &'static str {
        "stalker_mine"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let mut p = projectile_for(ctx, shot, shot.shooter.orientation.forward());
        p.arm_tick = ctx.now() + Tick::ticks_for_secs(shot.def.param("arm_secs", 1.0));
        ctx.spawn_projectile(p);
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let def = ctx.def(p.weapon);
        let drift = |p: &mut Projectile| {
            if !p.is_stuck() {
                let drag = def.param("launch_drag", 1.5).max(0.0);
                p.vel *= (1.0 - drag * DT).max(0.0);
                if p.vel.length_squared() < 0.05 * 0.05 {
                    p.vel = Vec3::ZERO;
                }
            }
        };
        if ctx.now() < p.arm_tick {
            drift(p);
            return;
        }
        p.flags |= proj_flags::ARMED;
        let detect = def.param("detect_radius", 10.0);
        let team = p.team;
        let pos = p.pos;
        let view = ctx.view;
        let target = ctx
            .nearest_ship(pos, detect, |s| {
                s.team != team && {
                    let to = s.pos - pos;
                    let d = to.length();
                    d <= 1e-3 || view.terrain.raycast(pos, to, d).is_none()
                }
            })
            .map(|s| (s.player, s.pos));
        let Some((player, target_pos)) = target else {
            // A mine that just lost its target stops and waits; one that
            // had none coasts to a stop (a Magnet Sneeze pull carries it).
            if p.target.take().is_some() && !p.is_stuck() {
                p.vel = Vec3::ZERO;
            } else {
                drift(p);
            }
            return;
        };
        p.target = Some(player);
        let to = target_pos - pos;
        if to.length() <= def.param("trigger_radius", 1.5) {
            explode(ctx, p, None);
            return;
        }
        // Creep at the target (pulls loose from a wall).
        let speed = def.param("creep_speed", 4.0);
        p.flags &= !proj_flags::STUCK;
        p.vel = to.normalize_or_zero() * speed;
        // Scorched trail.
        let spacing = def.param("scorch_spacing", 1.5);
        let radius = def.param("scorch_radius", 1.5);
        let ticks = Tick::ticks_for_secs(def.param("scorch_secs", 60.0));
        p.vars[VAR_CREPT] += speed * DT;
        if spacing > 0.0 && p.vars[VAR_CREPT] >= spacing {
            p.vars[VAR_CREPT] -= spacing;
            if radius > 0.0 && ticks > 0 && ctx.view.terrain.sdf(pos) < radius {
                let until = (ctx.now() + ticks).min(u32::MAX as u64) as u32;
                ctx.edit(TerrainEdit::Surface {
                    center: pos,
                    radius,
                    states: SurfaceStates::HOT.0,
                    until,
                });
            }
        }
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        let enemy = ctx.ship(hit.target).is_some_and(|s| s.team != p.team);
        if enemy && ctx.now() >= p.arm_tick {
            explode(ctx, p, Some(hit.target));
            HitResponse::Stop
        } else {
            HitResponse::Continue
        }
    }

    fn on_entity_hit(&self, _ctx: &mut Ctx, _p: &mut Projectile, _hit: &EntityHit) -> HitResponse {
        HitResponse::Continue
    }

    fn on_terrain_hit(&self, _ctx: &mut Ctx, _p: &mut Projectile, _hit: &RayHit) -> HitResponse {
        HitResponse::Stick
    }

    fn on_fuse(&self, _ctx: &mut Ctx, _p: &mut Projectile) {}
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{SurfaceStates, TerrainEdit};
    use ls_math::Vec3;

    use crate::weapons::behavior;
    use crate::weapons::ordnance::heavy_rocket::testkit::*;
    use crate::world::proj_flags;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn mine(w: &World) -> Option<&Projectile> {
        let id = w.weapons().find("stalker_mine").unwrap();
        w.projectiles().iter().find(|p| p.weapon == id)
    }

    /// Room 40 (floor at y = -20); the owner drops a stalker onto the floor below it and
    /// leaves. Returns the world, the terrain, the owner and the mine's resting position.
    fn floor_mine() -> (World, ls_math::terrain::BoxRoomTerrain, PlayerId, Vec3) {
        let mut w = world();
        let mut t = room(40.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "stalker_mine");
        place(&mut w, a, Vec3::new(0.0, -16.5, 0.0), Vec3::NEG_Y);
        run(&mut w, &mut t, &fire2(a), 1);
        place(&mut w, a, Vec3::new(0.0, 15.0, -15.0), Vec3::NEG_Y);
        run(&mut w, &mut t, &[], 90);
        let m = mine(&w).unwrap();
        assert!(m.is_stuck(), "stuck to the floor: {:?}", m.pos);
        assert_ne!(m.flags & proj_flags::ARMED, 0);
        let pos = m.pos;
        (w, t, a, pos)
    }

    #[test]
    fn creeps_toward_an_enemy_within_10_u_and_blows_up() {
        let (mut w, mut t, _a, pos) = floor_mine();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        place(&mut w, b, pos + Vec3::new(8.0, 1.0, 0.0), Vec3::NEG_Z);
        let start = w.tick();
        let ev = run_until(&mut w, &mut t, 180, |e| !impacts(e).is_empty());
        let secs = (w.tick() - start) as f32 / 60.0;
        // (8.06 - 1.5) u at 4 u/s.
        assert!((secs - 1.64).abs() < 0.1, "{secs}");
        assert!(damage_to(&ev, b) > 15.0, "{}", damage_to(&ev, b));
        assert!(behavior("stalker_mine").implemented());
    }

    #[test]
    fn ignores_ships_beyond_10_u_its_owner_and_teammates() {
        let (mut w, mut t, a, pos) = floor_mine();
        let mate = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let far = w.add_player(Some(1), ShipType::Tinker).unwrap();
        place(&mut w, a, pos + Vec3::new(-5.0, 1.0, 0.0), Vec3::NEG_Z);
        place(&mut w, mate, pos + Vec3::new(0.0, 1.0, 5.0), Vec3::NEG_Z);
        place(&mut w, far, pos + Vec3::new(10.5, 1.0, 0.0), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &[], 120);
        assert!(impacts(&ev).is_empty());
        let m = mine(&w).unwrap();
        assert_eq!(m.pos, pos, "did not move");
        assert!(m.target.is_none());
    }

    /// A Magnet Sneeze from an enemy out of detection range pulls the armed
    /// mine loose and it coasts (it is not pinned in place without a target).
    #[test]
    fn a_magnet_sneeze_pulls_it_loose() {
        let (mut w, mut t, _a, pos) = floor_mine();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, b, "magnet_sneeze");
        place(&mut w, b, pos + Vec3::new(0.0, 15.0, 0.0), Vec3::NEG_Y);
        run(&mut w, &mut t, &fire2(b), 1);
        run(&mut w, &mut t, &[], 10);
        let m = mine(&w).expect("still there");
        assert!(!m.is_stuck());
        assert!(m.pos.distance(pos) >= 1.0, "{:?} vs {pos:?}", m.pos);
    }

    #[test]
    fn slow_enough_to_outfly() {
        let (mut w, mut t, _a, pos) = floor_mine();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        place(&mut w, b, pos + Vec3::new(6.0, 1.0, 0.0), Vec3::NEG_Z);
        let mut ev = Vec::new();
        let mut closest = f32::INFINITY;
        // The pilot backs off at 8 u/s, two thirds of cruise speed.
        for _ in 0..90 {
            let s = w.ship_mut(b).unwrap();
            s.vel = Vec3::new(8.0, 0.0, 0.0);
            ev.extend(w.step(&mut t, &[]));
            if let Some(m) = mine(&w) {
                closest = closest.min(m.pos.distance(w.ship(b).unwrap().pos));
            }
        }
        assert!(impacts(&ev).is_empty(), "never caught up");
        assert!(closest > 5.0, "{closest}");
        let m = mine(&w).unwrap();
        assert!(m.pos.x > pos.x + 2.0, "it gave chase: {:?}", m.pos);
        assert!(m.target.is_none(), "lost the target beyond 10 u");
        assert_eq!(m.vel, Vec3::ZERO, "and waits");
    }

    #[test]
    fn leaves_a_scorched_trail() {
        let (mut w, mut t, _a, pos) = floor_mine();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        place(&mut w, b, pos + Vec3::new(9.0, 1.0, 0.0), Vec3::NEG_Z);
        let ev = run_until(&mut w, &mut t, 180, |e| !impacts(e).is_empty());
        let now = w.tick();
        let marks: Vec<(Vec3, u32)> = terrain_edits(&ev)
            .into_iter()
            .filter_map(|e| match e {
                TerrainEdit::Surface {
                    center,
                    states,
                    until,
                    ..
                } if states == SurfaceStates::HOT.0 => Some((center, until)),
                _ => None,
            })
            .collect();
        // About 7.5 u of creeping, a mark every 1.5 u.
        assert!(marks.len() >= 4, "{}", marks.len());
        for pair in marks.windows(2) {
            let gap = pair[0].0.distance(pair[1].0);
            assert!((gap - 1.5).abs() < 0.1, "{gap}");
        }
        for (c, until) in &marks {
            assert!(c.x > pos.x && c.x < pos.x + 9.0, "along the path: {c:?}");
            let left = *until as u64 - now;
            assert!(left > 55 * 60 && left <= 60 * 60, "lasts 60 s: {left}");
        }
    }
}
