//! Flash Pod: ordnance secondary. Blinds pilots within 15 u for 2 s. Malus: no damage.
//! Lasting effect: surfaces glow for 30 s, lighting dark caverns and anyone inside.
//!
//! Numbers: `data/weapons/flash_pod.ron` (`params`: `blind_radius`, `blind_secs`,
//! `glow_secs`, `glow_radius`, `blind_needs_sight`).
//!
//! Status: implemented. The pod bursts where it hits a wall, a ship or debris, or in mid-air
//! when its lifetime runs out. The burst blinds every living pilot within `blind_radius`
//! that can see it (a terrain raycast; `blind_needs_sight: 0` drops the check), the shooter
//! and teammates included, for `blind_secs` (`ctx.blind`: `ShipState::blind_until` plus a
//! `Blinded` cue). It deals no damage and pushes nothing. It lights the surfaces within
//! `glow_radius` with the `GLOW` surface state for `glow_secs` (a `TerrainEdit::Surface`).

use ls_math::terrain::{RayHit, SurfaceStates, TerrainEdit};
use ls_math::{Tick, Vec3};

use crate::behavior::Ctx;
use crate::weapons::{EntityHit, HitResponse, ShipHit, WeaponBehavior};
use crate::world::Projectile;
use crate::Event;

/// Behaviour of the Flash Pod.
#[derive(Debug)]
pub struct FlashPod;

impl FlashPod {
    /// The flash at `point`; `normal` lifts the sight origin off a wall.
    fn burst(&self, ctx: &mut Ctx, p: &mut Projectile, point: Vec3, normal: Vec3) {
        let def = ctx.def(p.weapon);
        let blind_radius = def.param("blind_radius", 15.0);
        let blind_ticks = Tick::ticks_for_secs(def.param("blind_secs", 2.0));
        let glow_radius = def.param("glow_radius", 8.0);
        let glow_ticks = Tick::ticks_for_secs(def.param("glow_secs", 30.0));
        let needs_sight = def.param("blind_needs_sight", 1.0) > 0.0;
        ctx.emit(Event::Impact {
            projectile: p.id,
            weapon: p.weapon,
            point,
            target: None,
        });
        let eye = point + normal * 0.3;
        let victims: Vec<_> = ctx
            .ships_within(point, blind_radius)
            .into_iter()
            .map(|s| (s.player, s.pos))
            .collect();
        for (player, pos) in victims {
            let to = pos - eye;
            let dist = to.length();
            let seen = !needs_sight || dist <= 1e-3 || ctx.raycast(eye, to, dist).is_none();
            if seen && blind_ticks > 0 {
                ctx.blind(player, blind_ticks);
            }
        }
        if glow_radius > 0.0 && glow_ticks > 0 {
            let until = (ctx.now() + glow_ticks).min(u32::MAX as u64) as u32;
            ctx.edit(TerrainEdit::Surface {
                center: point,
                radius: glow_radius,
                states: SurfaceStates::GLOW.0,
                until,
            });
        }
        p.remove();
    }
}

impl WeaponBehavior for FlashPod {
    fn id(&self) -> &'static str {
        "flash_pod"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        self.burst(ctx, p, hit.point, hit.normal);
        HitResponse::Stop
    }

    fn on_entity_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
        self.burst(ctx, p, hit.point, hit.normal);
        HitResponse::Stop
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        self.burst(ctx, p, hit.point, hit.normal);
        HitResponse::Stop
    }

    fn on_expire(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let pos = p.pos;
        self.burst(ctx, p, pos, Vec3::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{SurfaceStates, TerrainEdit};
    use ls_math::Vec3;

    use crate::weapons::behavior;
    use crate::weapons::ordnance::heavy_rocket::testkit::*;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn blinds_pilots_within_15_u_for_2_s_without_damage() {
        let mut w = world();
        let mut t = room(80.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let near = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let edge = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let far = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let mate = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "flash_pod");
        // The pod hits the -Z wall (z = -40) at (0, 0, -40).
        place(&mut w, a, Vec3::new(0.0, 0.0, -10.0), Vec3::NEG_Z);
        // Off the pod's path; distances to the burst 5.8, 14.8 and 15.5 u.
        place(&mut w, near, Vec3::new(0.0, 5.0, -37.0), Vec3::Z);
        place(&mut w, edge, Vec3::new(0.0, 8.0, -40.0 + 12.45), Vec3::Z);
        place(&mut w, far, Vec3::new(0.0, 8.0, -40.0 + 13.28), Vec3::Z);
        place(&mut w, mate, Vec3::new(5.0, 0.0, -30.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        ev.extend(run_until(&mut w, &mut t, 120, |e| !impacts(e).is_empty()));
        let now = w.tick();
        let blinded: Vec<PlayerId> = cues(&ev, CueKind::Blinded)
            .iter()
            .filter_map(|c| c.target)
            .collect();
        assert!(blinded.contains(&near) && blinded.contains(&edge));
        assert!(blinded.contains(&mate), "teammates are blinded too");
        assert!(!blinded.contains(&far), "beyond 15 u");
        assert!(!blinded.contains(&a), "the shooter is 30 u away");
        for c in cues(&ev, CueKind::Blinded) {
            assert_eq!(c.value, 120, "2 s");
        }
        assert_eq!(w.ship(near).unwrap().blind_until, now + 120);
        assert_eq!(w.ship(far).unwrap().blind_until, 0);
        // No damage, no push.
        assert!(!ev.iter().any(|e| matches!(e, Event::Damaged { .. })));
        assert_eq!(w.ship(near).unwrap().vel, Vec3::ZERO);
        assert!(behavior("flash_pod").implemented());
    }

    #[test]
    fn walls_shade_the_flash_and_it_bursts_in_mid_air() {
        let mut w = world();
        let mut t = room(80.0);
        // A pillar of rock between the burst and one pilot.
        t.apply(TerrainEdit::AddSphere {
            center: Vec3::new(0.0, 0.0, -30.0),
            radius: 3.0,
        });
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let hidden = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let open = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, a, "flash_pod");
        // The pod bursts on the pillar's near side (z = -27).
        place(&mut w, a, Vec3::new(0.0, 0.0, -24.0), Vec3::NEG_Z);
        place(&mut w, hidden, Vec3::new(0.0, 0.0, -36.0), Vec3::Z);
        place(&mut w, open, Vec3::new(8.0, 0.0, -24.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        ev.extend(run_until(&mut w, &mut t, 60, |e| !impacts(e).is_empty()));
        let hit = impacts(&ev)[0].1;
        assert!((hit.z - -27.0).abs() < 0.5, "burst on the pillar: {hit:?}");
        let blinded: Vec<PlayerId> = cues(&ev, CueKind::Blinded)
            .iter()
            .filter_map(|c| c.target)
            .collect();
        assert!(!blinded.contains(&hidden), "behind the pillar");
        assert!(blinded.contains(&open) && blinded.contains(&a));

        // Mid-air: fired into open space it bursts when its lifetime ends.
        let mut w = world();
        let mut t = room(200.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, a, "flash_pod");
        place(&mut w, a, Vec3::new(0.0, 0.0, 60.0), Vec3::NEG_Z);
        // 3 s x 30 u/s = 90 u of flight: it ends near z = -30.
        place(&mut w, b, Vec3::new(5.0, 0.0, -30.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        ev.extend(run_until(&mut w, &mut t, 240, |e| !impacts(e).is_empty()));
        let burst = impacts(&ev)[0].1;
        assert!((burst.z - -30.0).abs() < 1.5, "{burst:?}");
        assert!(cues(&ev, CueKind::Blinded)
            .iter()
            .any(|c| c.target == Some(b)));
    }

    #[test]
    fn surfaces_glow_for_30_s() {
        let mut w = world();
        let mut t = room(60.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "flash_pod");
        place(&mut w, a, Vec3::new(0.0, 0.0, -10.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        ev.extend(run_until(&mut w, &mut t, 120, |e| !impacts(e).is_empty()));
        let now = w.tick();
        let glow: Vec<_> = terrain_edits(&ev)
            .into_iter()
            .filter_map(|e| match e {
                TerrainEdit::Surface {
                    center,
                    radius,
                    states,
                    until,
                } => Some((center, radius, states, until)),
                _ => None,
            })
            .collect();
        assert_eq!(glow.len(), 1);
        let (center, radius, states, until) = glow[0];
        assert!((center.z - -30.0).abs() < 0.5);
        assert_eq!(radius, 8.0);
        assert_eq!(states, SurfaceStates::GLOW.0);
        assert_eq!(until as u64, now + 30 * 60, "30 s");
        // The pod leaves no crater.
        assert!(terrain_edits(&ev).iter().all(|e| !e.changes_geometry()));
    }
}
