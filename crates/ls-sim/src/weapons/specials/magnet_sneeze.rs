//! Magnet Sneeze: special secondary. Cone pulls loose debris and enemies toward you; great
//! for scavenging. Malus: also pulls in mines thrown at you. Lasting effect: rips loose
//! chunks off weak walls.
//!
//! Numbers: `data/weapons/magnet_sneeze.ron` (`params`: `cone_half_deg`, `range`,
//! `pull_speed`, `ship_pull_speed`, `projectile_pull_speed`, `pickup_pull`, `pickup_stop`,
//! `chunk_radius`, `chunk_rays`, `max_chunks`, `chunk_scrap`).
//!
//! Status: implemented. `fire` acts at once on everything in a cone of `cone_half_deg`
//! around the aim within `range` u with a clear line of sight:
//! - debris gets `pull_speed` u/s towards the shooter (scaled by mass, so every piece moves
//!   the same speed; resting debris wakes up),
//! - pickups and scrap piles (they have no velocity) jump `pickup_pull` u towards the shooter,
//!   stopping `pickup_stop` u short of it (`ctx.move_pickup`; a pickup's spawn point stays
//!   put): great for scavenging, a close weapon pickup lands in reach at once,
//! - enemy ships get `ship_pull_speed` u/s towards the shooter (teammates are left alone),
//! - every projectile that is not the shooter's own (mines included; a stuck mine is pulled
//!   loose) gets `projectile_pull_speed` u/s towards the shooter: the malus,
//! - weak rock (`MATERIAL_WEAK`) hit by one of `chunk_rays` rays (centre plus a ring at half
//!   the cone) loses a chunk: a strength-scaled crater of `chunk_radius` and a debris piece
//!   of that size with `chunk_scrap` scrap flying at the shooter, at most `max_chunks` per
//!   sneeze.
//!
//! The visible sneeze is a harmless blob projectile (no ship hits, no damage, vanishes on
//! contact) that also carries the `Fired` event.

use ls_math::terrain::{RayHit, MATERIAL_WEAK};
use ls_math::{Quat, Vec3};

use crate::behavior::Ctx;
use crate::entity::EntityKind;
use crate::weapons::{projectile_for, EntityHit, HitResponse, Shot, WeaponBehavior};
use crate::world::{proj_flags, Projectile};
use crate::PickupKind;

/// Behaviour of the Magnet Sneeze.
#[derive(Debug)]
pub struct MagnetSneeze;

/// Cone test: the distance to `pos` if it lies in the cone and is visible.
fn in_cone(ctx: &Ctx, origin: Vec3, fwd: Vec3, cos: f32, range: f32, pos: Vec3) -> Option<f32> {
    let to = pos - origin;
    let d = to.length();
    if d > range || d <= 1e-4 || to.dot(fwd) < cos * d {
        return None;
    }
    // Line of sight to the target's centre (stop just short of it: a mine
    // stuck to a wall sits on the surface).
    let clear = d - 0.3;
    if clear > 0.0 && ctx.raycast(origin, to, clear).is_some() {
        return None;
    }
    Some(d)
}

impl WeaponBehavior for MagnetSneeze {
    fn id(&self) -> &'static str {
        "magnet_sneeze"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let s = shot.shooter;
        let def = shot.def;
        let fwd = s.orientation.forward();
        let origin = s.pos;
        let half = def.param("cone_half_deg", 30.0);
        let cos = ls_math::det::cos(half.to_radians());
        let range = def.param("range", 20.0);
        let pull = def.param("pull_speed", 15.0);
        let ship_pull = def.param("ship_pull_speed", 10.0);
        let proj_pull = def.param("projectile_pull_speed", 12.0);

        // The visible sneeze.
        let mut blob = projectile_for(ctx, shot, fwd);
        blob.damage = 0.0;
        blob.splash_damage = 0.0;
        blob.knockback = 0.0;
        blob.flags |= proj_flags::NO_SHIP_HITS;
        ctx.spawn_projectile(blob);

        // Debris.
        let debris: Vec<(u32, Vec3, f32)> = ctx
            .view
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Debris)
            .map(|e| (e.id, e.pos, e.mass()))
            .collect();
        for (id, pos, mass) in debris {
            if in_cone(ctx, origin, fwd, cos, range, pos).is_some() {
                let dir = (origin - pos).normalize_or_zero();
                ctx.impulse_entity(id, dir * (pull * mass));
            }
        }

        // Pickups and scrap piles.
        let jump = def.param("pickup_pull", 10.0);
        let stop = def.param("pickup_stop", 1.5);
        let loose: Vec<(u32, Vec3)> = ctx
            .view
            .pickups
            .iter()
            .filter(|sp| {
                sp.content
                    .is_some_and(|(_, k)| !matches!(k, PickupKind::Scrap { .. }))
            })
            .map(|sp| (sp.id, sp.content_pos()))
            .chain(ctx.view.scrap_piles.iter().map(|s| (s.id, s.pos)))
            .collect();
        for (id, pos) in loose {
            if let Some(d) = in_cone(ctx, origin, fwd, cos, range, pos) {
                let step = jump.min(d - stop);
                if step > 0.0 {
                    ctx.move_pickup(id, (origin - pos) * (step / d));
                }
            }
        }

        // Enemy ships.
        let enemies: Vec<(crate::PlayerId, Vec3)> = ctx
            .view
            .ships
            .values()
            .filter(|o| o.alive && o.team != s.team && o.player != s.player)
            .map(|o| (o.player, o.pos))
            .collect();
        for (id, pos) in enemies {
            if in_cone(ctx, origin, fwd, cos, range, pos).is_some() {
                ctx.impulse_ship(id, (origin - pos).normalize_or_zero() * ship_pull);
            }
        }

        // Everyone else's projectiles and mines (the malus).
        let shots: Vec<(u32, Vec3)> = ctx
            .projectiles()
            .filter(|p| p.owner != s.player)
            .map(|p| (p.id, p.pos))
            .collect();
        for (id, pos) in shots {
            if in_cone(ctx, origin, fwd, cos, range, pos).is_some() {
                ctx.impulse_projectile(id, (origin - pos).normalize_or_zero() * proj_pull);
            }
        }

        // Chunks off weak walls.
        let chunk = def.param("chunk_radius", 0.6);
        let rays = def.param("chunk_rays", 5.0).max(1.0) as usize;
        let max_chunks = def.param("max_chunks", 3.0).max(0.0) as usize;
        let scrap = def.param("chunk_scrap", 2.0);
        let (right, up) = (s.orientation.right(), s.orientation.up());
        let mut chunks = 0;
        for i in 0..rays {
            if chunks >= max_chunks {
                break;
            }
            let dir = if i == 0 {
                fwd
            } else {
                let phi = std::f32::consts::TAU * (i - 1) as f32 / (rays - 1).max(1) as f32;
                let (sn, cs) = ls_math::det::sin_cos(phi);
                Quat::from_axis_angle(right * cs + up * sn, (half * 0.5).to_radians()).rotate(fwd)
            };
            let Some(hit) = ctx.raycast(origin, dir, range) else {
                continue;
            };
            if ctx.material_at(hit.point - hit.normal * 0.1) != MATERIAL_WEAK {
                continue;
            }
            ctx.crater(hit.point, chunk);
            let at = hit.point + hit.normal * (chunk * 1.5 + 0.2);
            let vel = (origin - at).normalize_or_zero() * pull;
            ctx.spawn_debris(at, vel, chunk, scrap);
            chunks += 1;
        }
    }

    fn on_terrain_hit(&self, _ctx: &mut Ctx, _p: &mut Projectile, _hit: &RayHit) -> HitResponse {
        HitResponse::Stop
    }

    fn on_entity_hit(&self, _ctx: &mut Ctx, _p: &mut Projectile, _hit: &EntityHit) -> HitResponse {
        HitResponse::Stop
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::TerrainEdit;
    use ls_math::Vec3;

    use super::super::paint_thrower::testkit::*;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn entity(w: &World, id: u32) -> Entity {
        w.entities().iter().find(|e| e.id == id).unwrap().clone()
    }

    #[test]
    fn pulls_debris_and_enemies_in_the_cone_towards_you() {
        let mut w = world();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let mate = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let mut t = TestRoom::new();
        equip(&mut w, a, "magnet_sneeze");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(2.0, 0.0, 8.0), Vec3::Z);
        place(&mut w, mate, Vec3::new(-2.0, 0.0, 8.0), Vec3::Z);
        // Debris in the cone, behind the shooter, and beyond range.
        let (inside, behind, far) = (
            Vec3::new(0.0, 3.0, 10.0),
            Vec3::new(0.0, 0.0, 26.0),
            Vec3::new(0.0, 0.0, -25.0),
        );
        let [id_in, id_behind, id_far] = [inside, behind, far]
            .map(|pos| w.spawn_entity(EntitySpawn::debris(pos, Vec3::ZERO, 0.8, 3.0)));
        run(&mut w, &mut t, &[], 30);
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        assert_eq!(fired(&ev), 1);
        let apos = w.ship(a).unwrap().pos;
        let d_in = entity(&w, id_in);
        assert!(
            d_in.vel.dot((apos - d_in.pos).normalize()) > 10.0,
            "debris pulled: {:?}",
            d_in.vel
        );
        assert_eq!(entity(&w, id_behind).vel, Vec3::ZERO, "outside the cone");
        assert_eq!(entity(&w, id_far).vel, Vec3::ZERO, "out of range");
        let vb = w.ship(b).unwrap().vel;
        assert!(vb.z > 5.0, "enemy pulled towards the shooter: {vb:?}");
        assert!(
            w.ship(mate).unwrap().vel.length() < 0.5,
            "teammates are left alone"
        );
        assert_eq!(damage_to(&ev, b), 0.0);
        // Pulled debris arrives where the shooter can scavenge it.
        run(&mut w, &mut t, &[], 30);
        let d = entity(&w, id_in);
        assert!(
            d.pos.distance(apos) < inside.distance(apos) - 4.0,
            "{:?}",
            d.pos
        );
    }

    #[test]
    fn also_pulls_in_enemy_shots_but_not_your_own() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "magnet_sneeze");
        let rocket = equip(&mut w, b, "bolt_rocket");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        // b fires a rocket across a's cone (moving along -x).
        place(&mut w, b, Vec3::new(8.0, 0.0, 8.0), Vec3::NEG_X);
        run(&mut w, &mut t, &press(b, Buttons::FIRE_SECONDARY), 1);
        place(&mut w, b, Vec3::new(20.0, 0.0, -20.0), Vec3::NEG_X);
        // a's own laser bolt in the cone too.
        run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
        let before: Vec<(u32, PlayerId, Vec3)> = w
            .projectiles()
            .iter()
            .map(|p| (p.id, p.owner, p.vel))
            .collect();
        assert_eq!(before.len(), 2);
        run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        for (id, owner, vel) in before {
            let p = w.projectiles().iter().find(|p| p.id == id).unwrap();
            if owner == a {
                assert_eq!(p.vel, vel, "own shot untouched");
            } else {
                assert_eq!(p.weapon, rocket);
                assert!(
                    p.vel.z - vel.z > 5.0,
                    "rocket pulled: {vel:?} -> {:?}",
                    p.vel
                );
            }
        }
    }

    #[test]
    fn pulls_scrap_piles_towards_you() {
        let (mut w, a, _b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "magnet_sneeze");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        let near = w.add_scrap_pile(Vec3::new(0.0, 0.0, 16.0), 5.0);
        let far = w.add_scrap_pile(Vec3::new(0.0, 0.0, 4.0), 5.0);
        let behind = w.add_scrap_pile(Vec3::new(0.0, 0.0, 25.0), 5.0);
        run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        let pos = |id: u32| w.scrap_piles().iter().find(|s| s.id == id).unwrap().pos;
        let apos = w.ship(a).unwrap().pos;
        // 4 u away: stops `pickup_stop` (1.5 u) short of the shooter.
        assert!(
            (pos(near).distance(apos) - 1.5).abs() < 0.1,
            "{:?}",
            pos(near)
        );
        // 16 u away: jumps `pickup_pull` (10 u).
        assert!((pos(far).z - 14.0).abs() < 0.1, "{:?}", pos(far));
        assert_eq!(pos(behind), Vec3::new(0.0, 0.0, 25.0), "outside the cone");
    }

    /// The malus with a real mine: a Prox Mine stuck to a wall is pulled
    /// loose and comes at you.
    #[test]
    fn pulls_a_stuck_prox_mine_loose() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "magnet_sneeze");
        equip(&mut w, b, "prox_mine");
        // b drops a mine behind it; it drifts into the -z wall and sticks.
        place(&mut w, b, Vec3::new(0.0, 0.0, -27.0), Vec3::Z);
        run(&mut w, &mut t, &press(b, Buttons::FIRE_SECONDARY), 1);
        place(&mut w, b, Vec3::new(20.0, 10.0, 20.0), Vec3::Z);
        run(&mut w, &mut t, &[], 120);
        let mine = w
            .projectiles()
            .iter()
            .find(|p| p.owner == b)
            .unwrap()
            .clone();
        assert!(mine.is_stuck(), "{:?}", mine.pos);
        place(&mut w, a, Vec3::new(0.0, 0.0, -14.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        let p = w.projectiles().iter().find(|p| p.id == mine.id).unwrap();
        assert!(!p.is_stuck(), "pulled loose");
        assert!(p.vel.z > 5.0, "towards the shooter: {:?}", p.vel);
        run(&mut w, &mut t, &[], 10);
        let p = w.projectiles().iter().find(|p| p.id == mine.id);
        assert!(
            p.is_none_or(|p| p.pos.z > mine.pos.z + 1.0),
            "moved towards the shooter (or already went off)"
        );
    }

    #[test]
    fn rips_chunks_off_weak_walls_only() {
        let (mut w, a, b) = world2();
        // Weak rock on the +x side of the room.
        let mut t = TestRoom::new().with_weak_x(0.0);
        let wid = equip(&mut w, a, "magnet_sneeze");
        let def = w.weapons().get(wid).clone();
        place(&mut w, b, Vec3::new(0.0, 10.0, -20.0), Vec3::Z);
        // Ordinary rock (-x wall): nothing breaks.
        place(&mut w, a, Vec3::new(-15.0, 0.0, 0.0), Vec3::NEG_X);
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        assert!(edits(&ev).is_empty());
        assert!(w.entities().iter().all(|e| e.kind != EntityKind::Debris));
        run(&mut w, &mut t, &[], 100);
        // Weak rock (+x wall): chunks come off and fly at the shooter.
        place(&mut w, a, Vec3::new(15.0, 0.0, 0.0), Vec3::X);
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        let craters: Vec<TerrainEdit> = edits(&ev)
            .into_iter()
            .filter(|e| matches!(e, TerrainEdit::SubtractSphere { .. }))
            .collect();
        let max = def.param("max_chunks", 0.0) as usize;
        assert!(!craters.is_empty() && craters.len() <= max, "{craters:?}");
        for c in &craters {
            assert!((c.center().x - 30.0).abs() < 0.5);
            // Weak rock breaks more (strength-scaled crater).
            assert!(c.radius() > def.param("chunk_radius", 0.0));
        }
        let chunks: Vec<&Entity> = w
            .entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Debris)
            .collect();
        assert_eq!(chunks.len(), craters.len());
        for c in chunks {
            assert!(c.vel.x < -5.0, "flies at the shooter: {:?}", c.vel);
            assert!(c.scrap > 0.0, "scavengeable");
        }
    }
}
