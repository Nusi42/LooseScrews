//! Fusion Charger: energy primary. Design doc: "Hold up to 2 s; damage and knockback scale
//! with charge. Malus: charging past 2 s burns your own hull. Lasting effect: melts a channel
//! along its path and shoves debris aside." Debris "can be shoved into enemies with the Fusion
//! Charger".
//!
//! Numbers: `data/weapons/fusion_charger.ron` (`charge`: 2 s to full, `damage_mult` at full;
//! `params`: `overcharge_hull_per_sec`, `channel_radius`, `channel_depth_min`,
//! `channel_depth_max`, `hot_secs`, `shove_radius`, `shove_accel`, `shove_hit_speed`).
//!
//! Behaviour:
//! - Charge: the framework charge flow (nothing paid while holding; on release damage and
//!   knockback times `1 + (damage_mult - 1) * charge`). `charge_hold` burns the shooter's hull
//!   at `overcharge_hull_per_sec` for every tick held past `charge.max_secs` (never below 1
//!   hull: the malus hurts but does not kill on its own). The first burning tick sends a
//!   `CueKind::Overcharge` cue (the hull burn itself emits no `Damaged` event).
//! - Channel: on terrain the bolt melts a channel (`ctx.channel`, craters `channel_radius`
//!   scaled from half at no charge to full at full charge) straight into the rock, up to a
//!   depth budget from `channel_depth_min` (no charge) to `channel_depth_max` (full charge),
//!   kept in `vars[0]`. A wall thinner than the remaining budget is melted through and the bolt
//!   flies on (`HitResponse::Pierce`); otherwise it ends at the channel's end. The entry glows
//!   hot (`SurfaceStates::HOT`).
//! - Debris: debris within `shove_radius` of the bolt is pushed sideways away from its path
//!   (`shove_accel` u/s^2 while near, times `1 + charge`, independent of the chunk's mass);
//!   debris the bolt runs into is flung aside and forward at `shove_hit_speed` (times
//!   `1 + charge`) and the bolt flies on.

use ls_math::terrain::{RayHit, SurfaceStates};
use ls_math::{Vec3, DT};

use super::scrap_laser::heat_surface;
use crate::behavior::{Ctx, Cue, CueKind};
use crate::entity::EntityKind;
use crate::ship::ShipState;
use crate::weapons::{
    default_charge_hold, default_entity_hit, projectile_for, ChargeAction, EntityHit, HitResponse,
    Shot, WeaponBehavior,
};
use crate::world::Projectile;
use crate::{WeaponDef, WeaponId};

/// Behaviour of the Fusion Charger.
#[derive(Debug)]
pub struct FusionCharger;

/// Unit vector from the bolt's path towards `target`, perpendicular to `dir`.
fn sideways(dir: Vec3, from: Vec3, target: Vec3) -> Vec3 {
    let d = target - from;
    let lateral = d - dir * d.dot(dir);
    lateral
        .try_normalize()
        .unwrap_or_else(|| dir.any_orthonormal())
}

impl WeaponBehavior for FusionCharger {
    fn id(&self) -> &'static str {
        "fusion_charger"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn charge_hold(
        &self,
        ctx: &mut Ctx,
        ship: &ShipState,
        weapon: WeaponId,
        held_ticks: u64,
    ) -> ChargeAction {
        let def = ctx.def(weapon);
        if let Some(c) = def.charge {
            let full = WeaponDef::charge_ticks(c.max_secs);
            if held_ticks > full {
                let per_sec = def.param("overcharge_hull_per_sec", 10.0);
                if held_ticks == full + 1 {
                    // The burn is silent in the hull pipeline (no `Damaged`
                    // event): tell the client it started.
                    ctx.cue(Cue {
                        kind: CueKind::Overcharge,
                        source: Some(ship.player),
                        target: None,
                        pos: ship.pos,
                        value: per_sec.clamp(0.0, f32::from(u16::MAX)) as u16,
                    });
                }
                ctx.adjust_hull(ship.player, -per_sec * DT, false);
            }
        }
        default_charge_hold(def, held_ticks)
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let mut p = projectile_for(ctx, shot, shot.shooter.orientation.forward());
        let min = shot.def.param("channel_depth_min", 1.0);
        let max = shot.def.param("channel_depth_max", 5.0);
        p.vars[0] = min + (max - min) * shot.charge;
        ctx.spawn_projectile(p);
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let def = ctx.def(p.weapon);
        let reach = def.param("shove_radius", 1.5);
        let accel = def.param("shove_accel", 120.0) * (1.0 + p.charge);
        if reach <= 0.0 || accel <= 0.0 {
            return;
        }
        let dir = p.vel.normalize_or_zero();
        let pushes: Vec<(u32, Vec3)> = ctx
            .view
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Debris)
            .filter(|e| e.pos.distance(p.pos) <= reach + e.radius)
            .map(|e| (e.id, sideways(dir, p.pos, e.pos) * (accel * DT * e.mass())))
            .collect();
        for (id, dv) in pushes {
            ctx.impulse_entity(id, dv);
        }
    }

    fn on_entity_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
        if hit.kind != EntityKind::Debris {
            return default_entity_hit(ctx, p, hit);
        }
        let Some(e) = ctx.view.entities.iter().find(|e| e.id == hit.id) else {
            return HitResponse::Continue;
        };
        let dir = p.vel.normalize_or_zero();
        let speed = ctx.def(p.weapon).param("shove_hit_speed", 12.0) * (1.0 + p.charge);
        let push = (sideways(dir, hit.point, e.pos) + dir).normalize_or_zero();
        let dv = push * (speed * e.mass());
        ctx.impulse_entity(hit.id, dv);
        HitResponse::Continue
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        let def = ctx.def(p.weapon);
        let dir = p.vel.normalize_or_zero();
        let radius = def.param("channel_radius", 0.8) * (0.5 + 0.5 * p.charge);
        heat_surface(
            ctx,
            hit.point,
            radius * 1.5,
            SurfaceStates::HOT,
            def.param("hot_secs", 10.0),
        );
        let budget = p.vars[0];
        if budget > 0.0 && radius > 0.0 {
            match ctx.thickness(hit.point, dir, budget) {
                Some(t) => {
                    ctx.channel(hit.point, hit.point + dir * t, radius, radius);
                    p.vars[0] = budget - t;
                    return HitResponse::Pierce {
                        exit: hit.point + dir * (t + 0.05),
                    };
                }
                None => {
                    let end = hit.point + dir * budget;
                    ctx.channel(hit.point, end, radius, radius);
                    p.vars[0] = 0.0;
                    ctx.detonate(p, end, None, false);
                    return HitResponse::Stop;
                }
            }
        }
        ctx.detonate(p, hit.point, None, false);
        HitResponse::Stop
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{SurfaceStates, TerrainEdit, TerrainQuery};
    use ls_math::Vec3;

    use super::super::scrap_laser::testkit::*;
    use crate::{EntityKind, EntitySpawn, ShipType};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Holds the trigger `ticks` long, releases, and returns (damage to b,
    /// b's speed right after the hit, a's hull lost).
    fn charged_shot(ticks: u32) -> (f32, f32, f32) {
        let (mut w, mut t, a, b) = duel("fusion_charger", ShipType::Hauler);
        place(&mut w, a, Vec3::new(0.0, 0.0, 12.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let hull0 = w.ship(a).unwrap().hull;
        let mut ev = run(&mut w, &mut t, &hold(a), ticks);
        let hull_lost = hull0 - w.ship(a).unwrap().hull;
        let mut speed = 0.0f32;
        for _ in 0..40 {
            let e = run(&mut w, &mut t, &[], 1);
            if damage_to(&e, b) > 0.0 {
                speed = w.ship(b).unwrap().vel.length();
            }
            ev.extend(e);
        }
        assert_eq!(fired(&ev), 1);
        (damage_to(&ev, b), speed, hull_lost)
    }

    #[test]
    fn damage_and_knockback_scale_with_charge() {
        let (d_tap, kb_tap, _) = charged_shot(6);
        let (d_half, kb_half, _) = charged_shot(60);
        let (d_full, kb_full, burn_full) = charged_shot(120);
        assert!(
            d_tap < d_half && d_half < d_full,
            "{d_tap} {d_half} {d_full}"
        );
        assert!(
            kb_tap < kb_half && kb_half < kb_full,
            "{kb_tap} {kb_half} {kb_full}"
        );
        // Full charge: 4x the base damage (Hauler takes 1x).
        assert!((d_full - 48.0).abs() < 0.5, "{d_full}");
        assert_eq!(burn_full, 0.0, "a full 2 s charge is free");
        // Holding longer adds nothing but the burn.
        let (d_over, _, _) = charged_shot(180);
        assert_eq!(d_over, d_full);
    }

    #[test]
    fn charging_past_two_seconds_burns_own_hull() {
        let (_, _, burn) = charged_shot(120);
        assert_eq!(burn, 0.0);
        // One second past full at 10 hull/s.
        let (_, _, burn) = charged_shot(180);
        assert!((burn - 10.0).abs() < 0.5, "{burn}");
        // The burn alone never kills.
        let (mut w, mut t, a, _b) = duel("fusion_charger", ShipType::Tinker);
        place(&mut w, a, Vec3::ZERO, Vec3::X);
        w.ship_mut(a).unwrap().hull = 5.0;
        let ev = run(&mut w, &mut t, &hold(a), 600);
        let cues: Vec<u16> = ev
            .iter()
            .filter_map(|e| match e {
                crate::Event::Cue(c) if c.kind == crate::CueKind::Overcharge => {
                    assert_eq!(c.source, Some(a));
                    Some(c.value)
                }
                _ => None,
            })
            .collect();
        assert_eq!(cues, vec![10], "one overcharge cue when the burn starts");
        let s = w.ship(a).unwrap();
        assert!(s.alive && s.hull >= 1.0 && s.hull < 5.0, "{}", s.hull);
    }

    #[test]
    fn melts_a_channel_deeper_with_charge() {
        let depth = |ticks: u32| {
            let (mut w, mut t, a, _b) = duel("fusion_charger", ShipType::Tinker);
            place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
            let mut ev = run(&mut w, &mut t, &hold(a), ticks);
            ev.extend(run(&mut w, &mut t, &[], 60));
            let c = craters(&ev);
            assert!(c.len() >= 2, "a channel of craters");
            assert!(!surfaces(&ev, SurfaceStates::HOT).is_empty(), "hot entry");
            // Deepest open point along the path.
            let mut z = -30.0f32;
            while t.sdf(Vec3::new(0.0, 0.0, z - 0.1)) > 0.0 {
                z -= 0.1;
            }
            -30.0 - z
        };
        let low = depth(6);
        let full = depth(120);
        assert!(low > 0.5, "{low}");
        assert!(full > low + 2.0, "{low} vs {full}");
    }

    #[test]
    fn melts_through_thin_walls_and_flies_on() {
        let (mut w, mut t, a, b) = duel("fusion_charger", ShipType::Hauler);
        // A 1.2 u wall across the room at z = -5.
        for i in -6..=6 {
            for j in -3..=3 {
                t.inner.apply(TerrainEdit::AddSphere {
                    center: Vec3::new(i as f32 * 1.0, j as f32 * 1.0, -5.0),
                    radius: 0.6,
                });
            }
        }
        place(&mut w, a, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -12.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &hold(a), 120);
        for _ in 0..60 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, Vec3::new(0.0, 0.0, -12.0), Vec3::Z);
        }
        assert!(damage_to(&ev, b) > 40.0, "hit behind the wall");
        assert!(t.sdf(Vec3::new(0.0, 0.0, -5.0)) > 0.0, "a hole through it");
    }

    #[test]
    fn shoves_debris_aside() {
        let (mut w, mut t, a, _b) = duel("fusion_charger", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        // One chunk right in the path, one just beside it.
        let hit = w.spawn_entity(EntitySpawn::debris(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::ZERO,
            0.5,
            2.0,
        ));
        let near = w.spawn_entity(EntitySpawn::debris(
            Vec3::new(1.2, 0.0, -4.0),
            Vec3::ZERO,
            0.8,
            2.0,
        ));
        run(&mut w, &mut t, &hold(a), 60);
        run(&mut w, &mut t, &[], 45);
        let get = |id: u32| {
            w.entities()
                .iter()
                .find(|e| e.id == id && e.kind == EntityKind::Debris)
                .cloned()
                .expect("debris survives")
        };
        let (h, n) = (get(hit), get(near));
        assert!(
            h.pos.x.abs() > 0.3 || h.pos.y.abs() > 0.3,
            "pushed off the path: {:?}",
            h.pos
        );
        assert!(h.pos.z < -0.3, "and along it: {:?}", h.pos);
        assert!(n.pos.x > 1.4, "the side chunk is pushed away: {:?}", n.pos);
    }
}
