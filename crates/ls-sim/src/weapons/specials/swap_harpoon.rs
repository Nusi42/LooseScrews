//! Swap Harpoon: special secondary. On hit, you swap positions with the target (velocities
//! swap too). Malus: 4 s cooldown; you inherit their momentum. Lasting effect: the harpoon
//! spike stays in the wall as a small grab point.
//!
//! Numbers: `data/weapons/swap_harpoon.ron` (`fire_interval_secs` is the 4 s cooldown;
//! `params`: `spike_radius`, `max_spikes`, `grab_reach`, `grab_hold`).
//!
//! Status: implemented.
//! - Ship hit: `ctx.swap(owner, target)` trades positions and velocities (you inherit their
//!   momentum), a `Swapped` cue goes to both, then the light direct damage. Any ship can be
//!   swapped with, teammates included (friendly fire scales only the damage).
//! - Terrain hit: the harpoon ends and leaves its spike in the wall: a `Marker` entity with
//!   the harpoon visual, pointing along the shot, that lasts until the rock around it is
//!   blasted away (`entity_tick` removes a spike left floating) or the owner has more than
//!   `max_spikes` spikes (the oldest goes).
//! - Grab point: a ship within `grab_reach` u of a spike (hull to spike) holding Scavenge is
//!   held there: `grab_hold` of its velocity is taken away every tick.

use ls_math::terrain::RayHit;
use ls_math::{Quat, Vec3};

use crate::behavior::{Ctx, Cue, CueKind};
use crate::entity::{Entity, EntityKind, EntitySpawn, Source};
use crate::weapons::{HitResponse, ShipHit, WeaponBehavior};
use crate::world::Projectile;
use crate::Buttons;

/// Behaviour of the Swap Harpoon.
#[derive(Debug)]
pub struct SwapHarpoon;

impl WeaponBehavior for SwapHarpoon {
    fn id(&self) -> &'static str {
        "swap_harpoon"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        let owner_alive = ctx.ship(p.owner).is_some_and(|s| s.alive);
        if owner_alive && hit.target != p.owner {
            ctx.swap(p.owner, hit.target);
            ctx.cue(Cue {
                kind: CueKind::Swapped,
                source: Some(p.owner),
                target: Some(hit.target),
                pos: hit.point,
                value: 0,
            });
        }
        ctx.detonate(p, hit.point, Some(hit.target), false);
        HitResponse::Stop
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, true);
        let def = ctx.def(p.weapon);
        let radius = def.param("spike_radius", 0.3);
        let max = def.param("max_spikes", 6.0).max(1.0) as usize;
        let source = Source::Weapon(p.weapon);
        let mut mine: Vec<(u64, u32)> = ctx
            .view
            .entities
            .iter()
            .filter(|e| {
                e.kind == EntityKind::Marker && e.source == source && e.owner == Some(p.owner)
            })
            .map(|e| (e.spawn_tick, e.id))
            .collect();
        mine.sort_unstable();
        let excess = (mine.len() + 1).saturating_sub(max);
        for &(_, id) in mine.iter().take(excess) {
            ctx.push(crate::behavior::Command::RemoveEntity { id });
        }
        let dir = p.vel.normalize_or_zero();
        let rot = if dir == Vec3::ZERO {
            Quat::IDENTITY
        } else {
            Quat::from_rotation_arc(Vec3::NEG_Z, dir)
        };
        let mut spike = EntitySpawn::marker(hit.point).from_source(source);
        spike.visual = ctx.view.visuals.projectile(p.weapon);
        spike.radius = radius;
        spike.rot = rot;
        spike.owner = Some(p.owner);
        spike.team = p.team;
        ctx.spawn_entity(spike);
        HitResponse::Stop
    }

    fn entity_tick(&self, ctx: &mut Ctx, e: &mut Entity) {
        if e.kind != EntityKind::Marker {
            return;
        }
        // The wall around the spike was blasted away: the spike falls out.
        if ctx.view.terrain.sdf(e.pos) > e.radius + 0.5 {
            e.hp = 0.0;
            return;
        }
        let Source::Weapon(w) = e.source else {
            return;
        };
        let def = ctx.def(w);
        let reach = def.param("grab_reach", 1.0);
        let hold = def.param("grab_hold", 0.25).clamp(0.0, 1.0);
        let grabbing: Vec<(crate::PlayerId, Vec3)> = ctx
            .view
            .ships
            .values()
            .filter(|s| s.alive && s.vel != Vec3::ZERO)
            .filter(|s| {
                let r = ctx.tuning_of(s).radius + e.radius + reach;
                s.pos.distance_squared(e.pos) <= r * r
            })
            .filter(|s| {
                ctx.input(s.player)
                    .is_some_and(|i| i.buttons.contains(Buttons::SCAVENGE))
            })
            .map(|s| (s.player, s.vel))
            .collect();
        for (player, vel) in grabbing {
            ctx.impulse_ship(player, -vel * hold);
        }
    }
}

#[cfg(test)]
mod tests {
    use ls_math::Vec3;

    use super::super::paint_thrower::testkit::*;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn spikes(w: &World) -> Vec<Entity> {
        let wid = w.weapons().find("swap_harpoon").unwrap();
        w.entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Marker && e.source == Source::Weapon(wid))
            .cloned()
            .collect()
    }

    #[test]
    fn hit_swaps_positions_and_velocities() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "swap_harpoon");
        place(&mut w, a, Vec3::new(0.0, 0.0, 15.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, 0.0), Vec3::Z);
        w.ship_mut(b).unwrap().vel = Vec3::new(0.0, 0.0, -6.0);
        let mut ev = Vec::new();
        let mut swapped = false;
        for _ in 0..30 {
            let (pa, va) = {
                let s = w.ship(a).unwrap();
                (s.pos, s.vel)
            };
            let pb = w.ship(b).unwrap().pos;
            let step = run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
            let hit = !cues(&step, CueKind::Swapped).is_empty();
            ev.extend(step);
            if hit {
                // Positions traded (plus at most one tick of drift).
                let (na, nb) = (w.ship(a).unwrap().pos, w.ship(b).unwrap().pos);
                assert!(na.distance(pb) < 1.0, "{na:?} vs {pb:?}");
                assert!(nb.distance(pa) < 1.0, "{nb:?} vs {pa:?}");
                // You inherit their momentum; they get yours.
                assert!(
                    w.ship(a).unwrap().vel.z < -3.0,
                    "{:?}",
                    w.ship(a).unwrap().vel
                );
                assert_eq!(va, Vec3::ZERO);
                assert!(w.ship(b).unwrap().vel.length() < 1.0);
                swapped = true;
                break;
            }
        }
        assert!(swapped, "harpoon hit");
        let c = cues(&ev, CueKind::Swapped)[0];
        assert_eq!((c.source, c.target), (Some(a), Some(b)));
        let dmg = w
            .weapons()
            .get(w.weapons().find("swap_harpoon").unwrap())
            .damage;
        assert!((damage_to(&ev, b) - dmg).abs() < 1e-3);
    }

    #[test]
    fn four_second_cooldown() {
        let (mut w, a, _b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "swap_harpoon");
        place(&mut w, a, Vec3::new(-10.0, 0.0, 0.0), Vec3::NEG_Z);
        let hold = press(a, Buttons::FIRE_SECONDARY);
        let ev = run(&mut w, &mut t, &hold, 239);
        assert_eq!(fired(&ev), 1, "one shot in the first 4 s");
        let ev = run(&mut w, &mut t, &hold, 2);
        assert_eq!(fired(&ev), 1, "next shot after 4 s");
    }

    #[test]
    fn spike_stays_in_the_wall_as_a_grab_point() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "swap_harpoon");
        place(&mut w, a, Vec3::new(-10.0, 0.0, -20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(20.0, 0.0, 20.0), Vec3::Z);
        run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        run(&mut w, &mut t, &[], 60);
        assert!(w.projectiles().is_empty());
        let s = spikes(&w);
        assert_eq!(s.len(), 1);
        let spike = s[0].clone();
        assert!((spike.pos.z - -30.0).abs() < 0.3, "{:?}", spike.pos);
        assert_eq!(spike.owner, Some(a));
        // Stays for the rest of the match.
        run(&mut w, &mut t, &[], 1200);
        assert_eq!(spikes(&w).len(), 1);

        // Drifting past it: holding Scavenge grabs on and stops the ship.
        let r = w.tuning().radius;
        let near = spike.pos + Vec3::new(0.0, 0.0, r + 0.6);
        let drift = Vec3::new(4.0, 0.0, 0.0);
        let speed_after = |w: &mut World, t: &mut TestRoom, grab: bool| {
            place(w, a, near, Vec3::NEG_Z);
            w.ship_mut(a).unwrap().vel = drift;
            let b = if grab {
                Buttons::SCAVENGE
            } else {
                Buttons::NONE
            };
            run(w, t, &press(a, b), 10);
            w.ship(a).unwrap().vel.length()
        };
        let free = speed_after(&mut w, &mut t, false);
        let held = speed_after(&mut w, &mut t, true);
        assert!(free > 1.5, "drifts on without grabbing: {free}");
        assert!(held < free * 0.2, "held by the spike: {held} vs {free}");
    }

    #[test]
    fn old_spikes_go_past_the_cap_and_blasted_walls_drop_them() {
        let (mut w, a, _b) = world2();
        let mut t = TestRoom::new();
        let wid = equip(&mut w, a, "swap_harpoon");
        let max = w.weapons().get(wid).param("max_spikes", 0.0) as usize;
        assert!(max >= 2);
        for i in 0..=max {
            place(
                &mut w,
                a,
                Vec3::new(-20.0 + 3.0 * i as f32, 0.0, -20.0),
                Vec3::NEG_Z,
            );
            w.ship_mut(a).unwrap().weapon_mut(wid).unwrap().ammo = 5;
            run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
            run(&mut w, &mut t, &[], 240);
        }
        let s = spikes(&w);
        assert_eq!(s.len(), max, "capped");
        assert!(s.iter().all(|e| e.pos.x > -19.0), "the oldest went first");
        // Blasting the rock around one spike drops it.
        let target = s[0].pos;
        t.edits.push(ls_math::terrain::TerrainEdit::SubtractSphere {
            center: target,
            radius: 3.0,
        });
        run(&mut w, &mut t, &[], 1);
        assert_eq!(spikes(&w).len(), max - 1);
    }
}
