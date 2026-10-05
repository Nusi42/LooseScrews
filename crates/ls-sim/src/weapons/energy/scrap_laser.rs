//! Scrap Laser: energy primary, the laser-speed baseline (30 u/s, 3 s lifetime). Design doc:
//! "Straight bolts at 30 u/s; 6 upgrade levels found as pickups. Malus: weak at level 1.
//! Lasting effect: scorch marks; hit metal glows hot for 10 s."
//!
//! Numbers: `data/weapons/scrap_laser.ron` (`damage` is the level 1 bolt, `damage_per_level`
//! the extra share of it per upgrade level: 6 at level 1, 14 at level 6 per the M2 plan;
//! `params`: `hot_radius`, `hot_secs`). Upgrade pickups (`RuleSet::upgrade_pickups`) raise the
//! level by one up to `max_level` (the framework's `collect_upgrade`).
//!
//! Behaviour: the generic straight bolt (direct damage and knockback scaled by the level);
//! on terrain the bolt heats the rock it hits (`SurfaceStates::HOT`). Scorch decals are drawn by
//! each client from the `Impact` event (not networked, see the design doc's effect table).
//!
//! This file also holds the small helpers shared by the energy modules ([`heat_surface`]) and
//! their scenario-test kit (`testkit`, test builds only).

use ls_math::terrain::{RayHit, SurfaceStates, TerrainEdit};
use ls_math::{Tick, Vec3};

use crate::behavior::Ctx;
use crate::weapons::{HitResponse, WeaponBehavior};
use crate::world::Projectile;

/// Behaviour of the Scrap Laser.
#[derive(Debug)]
pub struct ScrapLaser;

impl WeaponBehavior for ScrapLaser {
    fn id(&self) -> &'static str {
        "scrap_laser"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, true);
        let def = ctx.def(p.weapon);
        let radius = def.param("hot_radius", 0.5);
        let secs = def.param("hot_secs", 10.0);
        heat_surface(ctx, hit.point, radius, SurfaceStates::HOT, secs);
        HitResponse::Stop
    }
}

/// Sets surface `state` on the rock within `radius` of `center` for `secs`
/// (an explicit expiry tick, so the data file decides the duration). Skips
/// the edit when the spot already has that state (sustained fire on one spot
/// does not flood the edit log; the first mark's timer stands).
pub(crate) fn heat_surface(
    ctx: &mut Ctx,
    center: Vec3,
    radius: f32,
    state: SurfaceStates,
    secs: f32,
) {
    if radius <= 0.0 || secs <= 0.0 {
        return;
    }
    if ctx
        .surface_states_near(center, radius * 0.5)
        .contains(state)
    {
        return;
    }
    let until = ctx
        .now()
        .saturating_add(Tick::ticks_for_secs(secs))
        .min(u32::MAX as u64) as u32;
    ctx.edit(TerrainEdit::Surface {
        center,
        radius,
        states: state.bits(),
        until,
    });
}

/// Scenario-test kit shared by the energy modules' tests.
#[cfg(test)]
pub(crate) mod testkit {
    use ls_math::terrain::{BoxRoomTerrain, RayHit, SurfaceStates, TerrainEdit, TerrainQuery};
    use ls_math::{Aabb, Quat, Vec3};

    use crate::{
        Buttons, Event, InputCmd, MatchSetup, PlayerId, RuleSet, ShipType, SimTerrain, SpawnPoint,
        WeaponSlot, WeaponState, World, WorldConfig,
    };

    pub fn setup() -> MatchSetup {
        MatchSetup {
            spawns: vec![
                SpawnPoint {
                    pos: Vec3::new(-20.0, 0.0, 0.0),
                    team: 0,
                    orientation: Quat::IDENTITY,
                },
                SpawnPoint {
                    pos: Vec3::new(20.0, 0.0, 0.0),
                    team: 1,
                    orientation: Quat::IDENTITY,
                },
            ],
            pickup_points: vec![],
            instability: 0,
            heist: None,
            ..Default::default()
        }
    }

    /// Paint the Void rules without pickups (nothing spawns in the way).
    pub fn quiet_rules() -> RuleSet {
        let mut rules = RuleSet::paint_the_void();
        rules.pickups.clear();
        rules.upgrade_pickups.clear();
        rules.gadget_pickups.clear();
        rules.scrap_pickup = None;
        rules
    }

    pub fn world_with(rules: RuleSet, setup: MatchSetup) -> World {
        World::new(WorldConfig {
            rules,
            ..WorldConfig::paint_the_void(7, setup)
        })
        .unwrap()
    }

    pub fn world() -> World {
        world_with(quiet_rules(), setup())
    }

    /// The 60 x 30 x 60 test room: walls at x, z = +-30 and y = +-15.
    pub fn room() -> SurfaceRoom {
        SurfaceRoom::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)))
    }

    /// A test terrain that keeps surface states (so charged rock drains
    /// ships in the framework step) on top of a box room.
    #[derive(Debug, Clone)]
    pub struct SurfaceRoom {
        pub inner: BoxRoomTerrain,
        /// `(center, radius, states, until)` of every surface edit applied.
        pub surfaces: Vec<(Vec3, f32, u8, u32)>,
    }

    impl SurfaceRoom {
        pub fn new(inner: BoxRoomTerrain) -> SurfaceRoom {
            SurfaceRoom {
                inner,
                surfaces: Vec::new(),
            }
        }
    }

    impl TerrainQuery for SurfaceRoom {
        fn sdf(&self, p: Vec3) -> f32 {
            self.inner.sdf(p)
        }
        fn normal(&self, p: Vec3) -> Vec3 {
            self.inner.normal(p)
        }
        fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
            self.inner.raycast(origin, dir, max_dist)
        }
        fn bounds(&self) -> Aabb {
            self.inner.bounds()
        }
    }

    impl SimTerrain for SurfaceRoom {
        fn apply_edit(&mut self, edit: &TerrainEdit) {
            if let TerrainEdit::Surface {
                center,
                radius,
                states,
                until,
            } = *edit
            {
                self.surfaces.push((center, radius, states, until));
            }
            self.inner.apply(*edit);
        }
        fn surface_states_near(&self, center: Vec3, radius: f32, now: u64) -> SurfaceStates {
            let mut s = 0u8;
            for &(c, r, states, until) in &self.surfaces {
                if (until as u64) > now && c.distance(center) <= r + radius {
                    s |= states;
                }
            }
            SurfaceStates(s)
        }
    }

    pub fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
        let s = w.ship_mut(id).unwrap();
        s.pos = pos;
        s.vel = Vec3::ZERO;
        s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
    }

    /// Gives weapon `id` to `p` directly (level 1, bypassing pickups) and
    /// selects it.
    pub fn equip(w: &mut World, p: PlayerId, id: &str) {
        let wid = w.weapons().find(id).unwrap();
        let def = w.weapons().get(wid).clone();
        let s = w.ship_mut(p).unwrap();
        s.add_weapon(WeaponState {
            weapon: wid,
            ammo: def.max_ammo,
            level: 1,
            shots: 0,
        });
        match def.slot {
            WeaponSlot::Primary => s.primary = Some(wid),
            WeaponSlot::Secondary => s.secondary = Some(wid),
        }
    }

    /// Two ships, `a` (team 0, Tinker) armed with `weapon`, `b` (team 1).
    pub fn duel(weapon: &str, b_type: ShipType) -> (World, SurfaceRoom, PlayerId, PlayerId) {
        let mut w = world();
        let t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), b_type).unwrap();
        equip(&mut w, a, weapon);
        (w, t, a, b)
    }

    pub fn run(
        w: &mut World,
        t: &mut SurfaceRoom,
        inputs: &[(PlayerId, InputCmd)],
        n: u32,
    ) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..n {
            ev.extend(w.step(t, inputs));
        }
        ev
    }

    pub fn hold(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
        [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_PRIMARY))]
    }

    pub fn damage_to(ev: &[Event], who: PlayerId) -> f32 {
        ev.iter()
            .filter_map(|e| match e {
                Event::Damaged { target, amount, .. } if *target == who => Some(*amount),
                _ => None,
            })
            .sum()
    }

    pub fn hits_on(ev: &[Event], who: PlayerId) -> usize {
        ev.iter()
            .filter(|e| matches!(e, Event::Damaged { target, .. } if *target == who))
            .count()
    }

    pub fn fired(ev: &[Event]) -> usize {
        ev.iter()
            .filter(|e| matches!(e, Event::Fired { .. }))
            .count()
    }

    /// Crater edits (`SubtractSphere`) as applied.
    pub fn craters(ev: &[Event]) -> Vec<(Vec3, f32)> {
        ev.iter()
            .filter_map(|e| match e {
                Event::TerrainEdit(TerrainEdit::SubtractSphere { center, radius }) => {
                    Some((*center, *radius))
                }
                _ => None,
            })
            .collect()
    }

    /// Surface edits setting `state`: `(center, radius, until)`.
    pub fn surfaces(ev: &[Event], state: SurfaceStates) -> Vec<(Vec3, f32, u32)> {
        ev.iter()
            .filter_map(|e| match e {
                Event::TerrainEdit(TerrainEdit::Surface {
                    center,
                    radius,
                    states,
                    until,
                }) if SurfaceStates(*states).contains(state) => Some((*center, *radius, *until)),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::SurfaceStates;
    use ls_math::Vec3;

    use super::testkit::*;
    use crate::{Event, MatchSetup, PickupKind, ShipType};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Damage of one level-`level` bolt against a Tinker (no ship multiplier).
    fn bolt_damage(level: u8) -> f32 {
        let (mut w, mut t, a, b) = duel("scrap_laser", ShipType::Tinker);
        let wid = w.weapons().find("scrap_laser").unwrap();
        w.ship_mut(a).unwrap().weapon_mut(wid).unwrap().level = level;
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 40));
        assert_eq!(hits_on(&ev, b), 1);
        damage_to(&ev, b)
    }

    #[test]
    fn weak_at_level_one_strong_at_level_six() {
        // M2 plan: level 1 bolt 6, level 6 bolt 14 (Tinker hull 100).
        let l1 = bolt_damage(1);
        let l6 = bolt_damage(6);
        assert!((l1 - 6.0).abs() < 1e-3, "level 1: {l1}");
        assert!((l6 - 14.0).abs() < 1e-2, "level 6: {l6}");
        let mut prev = l1;
        for level in 2..=6 {
            let d = bolt_damage(level);
            assert!(d > prev, "level {level} stronger");
            prev = d;
        }
        assert_eq!(bolt_damage(9), l6, "capped at level 6");
    }

    #[test]
    fn upgrade_pickups_raise_the_level_to_six() {
        let mut rules = quiet_rules();
        rules.upgrade_pickups = crate::RuleSet::paint_the_void().upgrade_pickups;
        let points: Vec<Vec3> = (0..6)
            .map(|i| Vec3::new(-10.0 + 4.0 * i as f32, 0.0, 0.0))
            .collect();
        let setup = MatchSetup {
            pickup_points: points.clone(),
            ..setup()
        };
        let mut w = world_with(rules, setup);
        let mut t = room();
        let laser = w.weapons().find("scrap_laser").unwrap();
        for p in w.pickups() {
            assert!(
                matches!(p.content, Some((_, PickupKind::Upgrade { weapon })) if weapon == laser)
            );
        }
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        assert_eq!(w.ship(a).unwrap().weapon(laser).unwrap().level, 1);
        for (i, p) in points.iter().enumerate() {
            place(&mut w, a, *p, Vec3::NEG_Z);
            run(&mut w, &mut t, &[], 1);
            let level = w.ship(a).unwrap().weapon(laser).unwrap().level;
            assert_eq!(level as usize, (i + 2).min(6), "pickup {i}");
        }
        // The sixth pickup stays: nothing to upgrade past level 6.
        assert!(w.pickups()[5].content.is_some());
    }

    #[test]
    fn laser_speed_bolts_heat_the_metal_they_hit() {
        let (mut w, mut t, a, _b) = duel("scrap_laser", ShipType::Tinker);
        // Fire at the -Z wall (z = -30) from 15 u away.
        place(&mut w, a, Vec3::new(0.0, 5.0, -15.0), Vec3::NEG_Z);
        let t0 = w.tick();
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        let p = w.projectiles()[0].clone();
        assert!((p.vel.length() - 30.0).abs() < 1e-3, "laser speed");
        ev.extend(run(&mut w, &mut t, &[], 40));
        assert!(ev.iter().any(|e| matches!(e, Event::Impact { .. })));
        let hot = surfaces(&ev, SurfaceStates::HOT);
        assert_eq!(hot.len(), 1);
        let (c, r, until) = hot[0];
        assert!((c.z + 30.0).abs() < 0.3, "on the wall: {c:?}");
        assert!(r > 0.0);
        // The bolt flies about 14 u (under 30 ticks); hot for 10 s from then.
        let until = until as u64;
        assert!(until >= t0 + 600 && until <= t0 + 630, "hot for 10 s");
        // A second bolt on the same spot while it is still hot adds no edit.
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 40));
        assert!(surfaces(&ev, SurfaceStates::HOT).is_empty());
        // After 10 s the spot can be heated again.
        run(&mut w, &mut t, &[], 600);
        let mut ev = run(&mut w, &mut t, &hold(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 40));
        assert_eq!(surfaces(&ev, SurfaceStates::HOT).len(), 1);
    }
}
