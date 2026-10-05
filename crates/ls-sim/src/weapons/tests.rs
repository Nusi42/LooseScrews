//! Framework tests: every hook kind and hit response, driven by small test
//! behaviours registered under `test_*` ids (only in test builds, see
//! `registry::behavior`), plus registry and fallback checks for the real
//! weapons.

use ls_math::terrain::{BoxRoomTerrain, RayHit, TerrainEdit};
use ls_math::{Quat, Vec3};

use super::*;
use crate::behavior::{Cue, CueKind, DamageSource};
use crate::entity::{EntityKind, EntitySpawn, Source};
use crate::world::proj_flags;
use crate::{
    Availability, Buttons, Event, InputCmd, MatchSetup, PlayerId, RuleSet, ShipType, SpawnPoint,
    WeaponTable, World, WorldConfig,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

// ----- test behaviours ---------------------------------------------------------

#[derive(Debug)]
struct Bouncer;
impl WeaponBehavior for Bouncer {
    fn id(&self) -> &'static str {
        "test_bounce"
    }
    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        if p.bounces < 2 {
            ctx.crater(hit.point, 0.3);
            p.flags |= proj_flags::HITS_OWNER;
            HitResponse::Bounce
        } else {
            ctx.detonate(p, hit.point, None, true);
            HitResponse::Stop
        }
    }
}

#[derive(Debug)]
struct Piercer;
impl WeaponBehavior for Piercer {
    fn id(&self) -> &'static str {
        "test_pierce"
    }
    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        let src = DamageSource::projectile(p);
        ctx.damage_proportional(hit.target, p.damage, p.vel, 0.1, src, true);
        HitResponse::Continue
    }
    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        let dir = p.vel.normalize_or_zero();
        match ctx.thickness(hit.point, dir, 2.0) {
            Some(t) => {
                ctx.channel(hit.point, hit.point + dir * t, 0.3, 0.5);
                HitResponse::Pierce {
                    exit: hit.point + dir * (t + 0.05),
                }
            }
            None => HitResponse::Stop,
        }
    }
}

#[derive(Debug)]
struct StickyFuse;
impl WeaponBehavior for StickyFuse {
    fn id(&self) -> &'static str {
        "test_stick"
    }
    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, _hit: &RayHit) -> HitResponse {
        p.fuse_tick = ctx.now() + 30;
        ctx.cue(Cue {
            kind: CueKind::TectonicArmed,
            source: Some(p.owner),
            target: None,
            pos: p.pos,
            value: 30,
        });
        HitResponse::Stick
    }
    fn on_fuse(&self, ctx: &mut Ctx, p: &mut Projectile) {
        ctx.detonate(p, p.pos, None, true);
        ctx.shockwave(p.pos, 20.0, 10.0);
        ctx.spawn_entity(EntitySpawn::dust(p.pos, 6.0, 120));
        ctx.spawn_debris(p.pos + Vec3::Y, Vec3::new(3.0, 2.0, 0.0), 0.5, 4.0);
    }
}

#[derive(Debug)]
struct Splitter;
impl WeaponBehavior for Splitter {
    fn id(&self) -> &'static str {
        "test_split"
    }
    fn on_expire(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if p.generation > 0 {
            return;
        }
        for i in 0..3 {
            let dir = Quat::from_axis_angle(Vec3::Y, i as f32).rotate(Vec3::NEG_Z);
            let mut sub = sub_projectile(ctx, p, p.pos, dir * 10.0, 20);
            sub.damage = 1.0;
            ctx.spawn_projectile(sub);
        }
    }
}

#[derive(Debug)]
struct Beam;
impl WeaponBehavior for Beam {
    fn id(&self) -> &'static str {
        "test_beam"
    }
    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let s = shot.shooter;
        if let Some(t) = ctx.target_in_cone(s.pos, s.orientation.forward(), 12.0, 30.0, s.team) {
            let target = t.player;
            ctx.drain_energy(target, Some(s.player), 10.0);
            let src = DamageSource {
                attacker: Some(s.player),
                team: s.team,
                weapon: Some(shot.weapon),
            };
            ctx.damage_proportional(target, shot.damage, t.pos - s.pos, 0.2, src, true);
            ctx.cue(Cue {
                kind: CueKind::Beam,
                source: Some(s.player),
                target: Some(target),
                pos: t.pos,
                value: 0,
            });
        }
    }
}

#[derive(Debug)]
struct Homing;
impl WeaponBehavior for Homing {
    fn id(&self) -> &'static str {
        "test_homing"
    }
    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let team = p.team;
        if let Some(t) = ctx.nearest_ship(p.pos, 100.0, |s| s.team != team) {
            p.target = Some(t.player);
            p.vel = steer_towards(p.vel, t.pos - p.pos, 3.0 * ls_math::DT);
        }
    }
}

#[derive(Debug)]
struct Guided;
impl WeaponBehavior for Guided {
    fn id(&self) -> &'static str {
        "test_guided"
    }
    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let mut p = projectile_for(ctx, shot, shot.shooter.orientation.forward());
        p.flags |= proj_flags::CONTROLS_OWNER;
        ctx.spawn_projectile(p);
    }
    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if let Some(input) = ctx.input(p.owner) {
            let yaw = input.look_yaw;
            p.vel = Quat::from_axis_angle(Vec3::Y, yaw).rotate(p.vel);
        }
    }
}

#[derive(Debug)]
struct Swapper;
impl WeaponBehavior for Swapper {
    fn id(&self) -> &'static str {
        "test_swap"
    }
    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        ctx.swap(p.owner, hit.target);
        HitResponse::Stop
    }
}

#[derive(Debug)]
struct Rewinder;
impl WeaponBehavior for Rewinder {
    fn id(&self) -> &'static str {
        "test_rewind"
    }
    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let s = shot.shooter;
        ctx.spawn_entity(
            EntitySpawn::ghost(s.player, s.team, s.pos, s.orientation, 60)
                .from_source(Source::Weapon(shot.weapon)),
        );
        ctx.rewind(s.player, 60);
        ctx.disable_weapons(s.player, 60);
        ctx.blind(s.player, 10);
    }
    fn entity_tick(&self, ctx: &mut Ctx, e: &mut crate::Entity) {
        // Ghosts fade: count ticks in a cue-free way (nothing to do).
        let _ = (ctx.now(), e.id);
    }
    fn entity_expired(&self, ctx: &mut Ctx, e: &mut crate::Entity) {
        ctx.cue(Cue {
            kind: CueKind::Custom,
            source: e.owner,
            target: None,
            pos: e.pos,
            value: 42,
        });
    }
}

/// Spawns a kinematic marker that flies out, turns back after 10 ticks
/// (entity hook writing its own state) and survives its first expiry.
/// Also digs one strength-scaled and one exact crater and reports the
/// shot counter and the Instability slider in a cue.
#[derive(Debug)]
struct Phaser;
impl WeaponBehavior for Phaser {
    fn id(&self) -> &'static str {
        "test_phase"
    }
    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let s = shot.shooter;
        let fwd = s.orientation.forward();
        let mut e = EntitySpawn::marker(s.pos + fwd * 2.0).from_source(Source::Weapon(shot.weapon));
        e.vel = fwd * 30.0;
        e.kinematic = true;
        e.lifetime_ticks = Some(40);
        ctx.spawn_entity(e);
        ctx.crater(s.pos + fwd * 5.0, 2.0);
        ctx.crater_exact(s.pos + fwd * 5.0, 2.0);
        ctx.cue(Cue {
            kind: CueKind::Custom,
            source: Some(s.player),
            target: None,
            pos: s.pos,
            value: (shot.shots as u16) << 8 | ctx.view.instability as u16,
        });
    }
    fn entity_tick(&self, _ctx: &mut Ctx, e: &mut crate::Entity) {
        e.vars[0] += 1.0;
        if e.vars[0] == 10.0 {
            e.vel = -e.vel;
            e.state[0] = 1;
        }
    }
    fn entity_expired(&self, _ctx: &mut Ctx, e: &mut crate::Entity) {
        if e.vars[1] == 0.0 {
            e.vars[1] = 1.0;
            e.expires_tick += 20;
        }
    }
}

/// Reaches other projectiles: pulls every enemy projectile towards itself
/// and removes enemy sub-projectiles; counts what it sees in `vars[0]`.
#[derive(Debug)]
struct Magnet;
impl WeaponBehavior for Magnet {
    fn id(&self) -> &'static str {
        "test_magnet"
    }
    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let others: Vec<(u32, Vec3, u8)> = ctx
            .projectiles()
            .filter(|o| o.owner != p.owner)
            .map(|o| (o.id, o.pos, o.generation))
            .collect();
        p.vars[0] = others.len() as f32;
        for (id, pos, generation) in others {
            if generation > 0 {
                ctx.remove_projectile(id);
            } else {
                ctx.impulse_projectile(id, (p.pos - pos).normalize_or_zero() * 5.0);
            }
        }
    }
}

static TEST_BEHAVIORS: [&dyn WeaponBehavior; 11] = [
    &Bouncer,
    &Piercer,
    &StickyFuse,
    &Splitter,
    &Beam,
    &Homing,
    &Guided,
    &Swapper,
    &Rewinder,
    &Phaser,
    &Magnet,
];

/// Test-only behaviours (see `registry::behavior`).
pub(crate) fn test_behavior(id: &str) -> Option<&'static dyn WeaponBehavior> {
    TEST_BEHAVIORS.iter().copied().find(|b| b.id() == id)
}

// ----- helpers -----------------------------------------------------------------

/// Built-in weapons plus the test weapons (laser numbers under test ids;
/// `test_stick` and `test_guided` as slower secondaries).
fn world() -> World {
    let builtin = WeaponTable::builtin();
    let laser = builtin.get(builtin.find("scrap_laser").unwrap()).clone();
    let rocket = builtin.get(builtin.find("bolt_rocket").unwrap()).clone();
    let mut defs: Vec<WeaponDef> = builtin.iter().map(|(_, d)| d.clone()).collect();
    for b in TEST_BEHAVIORS {
        let mut d = if matches!(b.id(), "test_stick" | "test_guided") {
            rocket.clone()
        } else {
            laser.clone()
        };
        d.id = b.id().to_string();
        d.max_level = 1;
        if b.id() == "test_split" {
            d.lifetime_secs = 0.5;
        }
        defs.push(d);
    }
    let weapons = WeaponTable::new(defs).unwrap();
    let mut rules = RuleSet::paint_the_void();
    for b in TEST_BEHAVIORS {
        rules
            .weapons
            .insert(b.id().to_string(), Availability::PickupOnly);
    }
    let setup = MatchSetup {
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
        instability: 42,
        heist: None,
        ..Default::default()
    };
    World::new(WorldConfig {
        weapons,
        rules,
        ..WorldConfig::paint_the_void(3, setup)
    })
    .unwrap()
}

fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0))
}

fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
    s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
}

/// Gives and selects a test weapon.
fn arm(w: &mut World, t: &mut BoxRoomTerrain, id: PlayerId, weapon: &str) {
    w.give_weapon(id, weapon).unwrap();
    let wid = w.weapons().find(weapon).unwrap();
    let cmd = InputCmd {
        weapon_select: wid.0 + 1,
        ..InputCmd::idle(0)
    };
    w.step(t, &[(id, cmd)]);
}

fn run(
    w: &mut World,
    t: &mut BoxRoomTerrain,
    inputs: &[(PlayerId, InputCmd)],
    n: u32,
) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..n {
        ev.extend(w.step(t, inputs));
    }
    ev
}

fn fire(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
    [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_PRIMARY))]
}

fn fire2(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
    [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_SECONDARY))]
}

fn damage_to(ev: &[Event], who: PlayerId) -> f32 {
    ev.iter()
        .filter_map(|e| match e {
            Event::Damaged { target, amount, .. } if *target == who => Some(*amount),
            _ => None,
        })
        .sum()
}

fn edits(ev: &[Event]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, Event::TerrainEdit(_)))
        .count()
}

// ----- registry and fallbacks ---------------------------------------------------

#[test]
fn every_weapon_has_a_module_and_a_data_file() {
    let t = WeaponTable::builtin();
    assert_eq!(MODULES.len(), 27);
    assert_eq!(t.len(), MODULES.len());
    for m in &MODULES {
        assert_eq!(m.behavior.id(), m.id);
        let def = t.get(t.find(m.id).expect("data file"));
        assert_eq!(def.family, m.family, "{}", m.id);
        assert_eq!(behavior(m.id).id(), m.id);
    }
    // Wave B filled every module: none relies on the generic fallback.
    let fallback: Vec<&str> = MODULES
        .iter()
        .filter(|m| !m.behavior.implemented())
        .map(|m| m.id)
        .collect();
    assert!(fallback.is_empty(), "still on the fallback: {fallback:?}");
    assert_eq!(
        behavior("homebrew").id(),
        "",
        "unknown ids get the generic weapon"
    );
    assert_eq!(registry::index_of("seed_grenade"), Some(26));
}

/// Every weapon (implemented or fallback) fires, flies and ends without
/// panicking, and its projectiles carry the weapon's visual.
#[test]
fn every_weapon_fires_with_its_fallback() {
    for m in &MODULES {
        let mut w = world();
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Hauler).unwrap();
        let b = w.add_player(Some(1), ShipType::Skitter).unwrap();
        let wid = w.weapons().find(m.id).unwrap();
        let def = w.weapons().get(wid).clone();
        {
            let s = w.ship_mut(a).unwrap();
            s.add_weapon(crate::WeaponState {
                weapon: wid,
                ammo: def.max_ammo,
                level: 1,
                shots: 0,
            });
            s.scrap = 100.0;
            match def.slot {
                crate::WeaponSlot::Primary => s.primary = Some(wid),
                crate::WeaponSlot::Secondary => s.secondary = Some(wid),
            }
        }
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let button = match def.slot {
            crate::WeaponSlot::Primary => Buttons::FIRE_PRIMARY,
            crate::WeaponSlot::Secondary => Buttons::FIRE_SECONDARY,
        };
        let hold = [(a, InputCmd::idle(0).with_buttons(button))];
        let mut ev = run(&mut w, &mut t, &hold, 130);
        // Charge weapons fire on release.
        ev.extend(run(&mut w, &mut t, &[], 1));
        // Instant weapons (Leech Arc) spawn no projectile: their shot is
        // the beam cue.
        let fired = ev
            .iter()
            .filter(|e| match e {
                Event::Fired { .. } => true,
                Event::Cue(c) => c.kind == CueKind::Beam,
                _ => false,
            })
            .count();
        assert!(fired >= 1, "{} never fired", m.id);
        let visual = w.visuals().projectile(wid);
        for p in w.projectiles().iter().filter(|p| p.weapon == wid) {
            assert_eq!(p.visual, visual, "{}", m.id);
        }
        run(&mut w, &mut t, &[], 400);
        assert!(
            w.projectiles()
                .iter()
                .all(|p| p.weapon != wid || def.lifetime_secs > 6.0),
            "{} projectiles end",
            m.id
        );
    }
}

// ----- hit responses -------------------------------------------------------------

#[test]
fn bounce_reflects_chips_and_can_hit_the_owner() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_bounce");
    // Fire into the -Z wall (z = -30) from 10 u away.
    place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    let mut bounced_back = false;
    for _ in 0..60 {
        ev.extend(run(&mut w, &mut t, &[], 1));
        if let Some(p) = w.projectiles().first() {
            bounced_back |= p.vel.z > 0.0;
        }
        // Keep the owner parked in the return path.
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
    }
    assert!(bounced_back, "reflected off the wall");
    assert!(edits(&ev) >= 1, "chips terrain at the bounce");
    assert!(damage_to(&ev, a) > 0.0, "the bounce hit its owner");
}

#[test]
fn pierce_passes_ships_and_thin_walls() {
    let mut w = world();
    let mut t = room();
    // A thin wall (1 u) across the room at z = -5 with an opening nowhere.
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let c = w.add_player(Some(1), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_pierce");
    for i in -6..=6 {
        for j in -3..=3 {
            t.apply(TerrainEdit::AddSphere {
                center: Vec3::new(i as f32 * 2.0, j as f32 * 2.0, -8.0),
                radius: 0.6,
            });
        }
    }
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::new(0.0, 0.0, 4.0), Vec3::Z);
    place(&mut w, c, Vec3::new(0.0, 0.0, -14.0), Vec3::Z);
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    for _ in 0..60 {
        ev.extend(run(&mut w, &mut t, &[], 1));
        place(&mut w, b, Vec3::new(0.0, 0.0, 4.0), Vec3::Z);
        place(&mut w, c, Vec3::new(0.0, 0.0, -14.0), Vec3::Z);
    }
    assert!(damage_to(&ev, b) > 0.0, "first ship hit");
    assert!(
        damage_to(&ev, c) > 0.0,
        "flew on through the ship and the wall"
    );
    assert!(edits(&ev) >= 1, "drilled a hole");
    // A 30 u/s bolt spends several ticks inside a ship's sphere: it still
    // hits it only once.
    let hits = |who: PlayerId| {
        ev.iter()
            .filter(|e| matches!(e, Event::Damaged { target, .. } if *target == who))
            .count()
    };
    assert_eq!(hits(b), 1, "passed through b once");
    assert_eq!(hits(c), 1);
}

#[test]
fn entity_hooks_write_back_and_expiry_can_be_extended() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_phase");
    // Facing a wall 3 u away: the kinematic marker flies into the rock.
    place(&mut w, a, Vec3::new(0.0, 0.0, -27.0), Vec3::NEG_Z);
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    ev.extend(run(&mut w, &mut t, &[], 5));
    let e = &w.entities()[0];
    assert!(
        e.pos.z < -30.5,
        "kinematic: no terrain collision: {:?}",
        e.pos
    );
    assert_eq!(e.vars[0], 6.0, "hooked from its spawn tick on");
    ev.extend(run(&mut w, &mut t, &[], 10));
    let e = &w.entities()[0];
    assert_eq!(e.state[0], 1, "the hook's own state change stuck");
    assert!(e.vel.z > 0.0, "turned back");
    // Lifetime 40 ticks, extended once by 20.
    ev.extend(run(&mut w, &mut t, &[], 30));
    assert_eq!(w.entities().len(), 1, "survived its first expiry");
    run(&mut w, &mut t, &[], 20);
    assert!(w.entities().is_empty(), "gone after the extension");
    // Shot counter and the Instability slider reach the hook.
    let custom = |ev: &[Event]| {
        ev.iter()
            .find_map(|e| match e {
                Event::Cue(c) if c.kind == CueKind::Custom => Some(c.value),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(custom(&ev), 42, "first shot (0), instability 42");
    run(&mut w, &mut t, &[], 60);
    let ev = run(&mut w, &mut t, &fire(a), 1);
    assert_eq!(custom(&ev) >> 8, 1, "second shot");
}

#[test]
fn hooks_reach_other_projectiles() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_magnet");
    arm(&mut w, &mut t, b, "test_split");
    place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::new(20.0, 0.0, -20.0), Vec3::NEG_X);
    // The enemy bolt flies across; the magnet bolt then pulls it.
    run(&mut w, &mut t, &fire(b), 1);
    let bolt = w.projectiles()[0].clone();
    run(&mut w, &mut t, &fire(a), 1);
    run(&mut w, &mut t, &[], 1);
    let magnet = w.projectiles().iter().find(|p| p.owner == a).unwrap();
    assert_eq!(magnet.vars[0], 1.0, "sees the enemy bolt while stepping");
    let pulled = w.projectiles().iter().find(|p| p.id == bolt.id).unwrap();
    assert!(
        pulled.vel.z > 1.0,
        "pulled: {:?} -> {:?}",
        bolt.vel,
        pulled.vel
    );
    // The split bolt's children (generation 1) are removed by the magnet.
    run(&mut w, &mut t, &[], 30);
    assert!(
        w.projectiles().iter().all(|p| p.generation == 0),
        "{:?}",
        w.projectiles()
    );
}

/// A room whose rock is hard for x < 0 and weak elsewhere, charged
/// everywhere.
struct StrengthRoom(BoxRoomTerrain);

impl ls_math::terrain::TerrainQuery for StrengthRoom {
    fn sdf(&self, p: Vec3) -> f32 {
        self.0.sdf(p)
    }
    fn bounds(&self) -> ls_math::Aabb {
        self.0.bounds()
    }
}

impl crate::SimTerrain for StrengthRoom {
    fn apply_edit(&mut self, edit: &TerrainEdit) {
        self.0.apply(*edit);
    }
    fn surface_states_near(
        &self,
        _center: Vec3,
        _radius: f32,
        _now: u64,
    ) -> ls_math::terrain::SurfaceStates {
        ls_math::terrain::SurfaceStates::CHARGED
    }
    fn material_at(&self, p: Vec3) -> u8 {
        if p.x < 0.0 {
            ls_math::terrain::MATERIAL_HARD
        } else {
            ls_math::terrain::MATERIAL_WEAK
        }
    }
}

fn run_on(
    w: &mut World,
    t: &mut StrengthRoom,
    inputs: &[(PlayerId, InputCmd)],
    n: u32,
) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..n {
        ev.extend(w.step(t, inputs));
    }
    ev
}

#[test]
fn craters_scale_by_rock_strength_and_charged_rock_drains() {
    let mut w = world();
    let mut t = StrengthRoom(room());
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    w.give_weapon(a, "test_phase").unwrap();
    let wid = w.weapons().find("test_phase").unwrap();
    let select = InputCmd {
        weapon_select: wid.0 + 1,
        ..InputCmd::idle(0)
    };
    w.step(&mut t, &[(a, select)]);
    let radii = |w: &mut World, t: &mut StrengthRoom, x: f32| {
        place(w, a, Vec3::new(x, 0.0, 0.0), Vec3::NEG_Z);
        run_on(w, t, &fire(a), 1)
            .iter()
            .filter_map(|e| match e {
                Event::TerrainEdit(TerrainEdit::SubtractSphere { radius, .. }) => Some(*radius),
                _ => None,
            })
            .collect::<Vec<f32>>()
    };
    // The world runs at Instability 42: craters (not exact ones) also grow
    // by 1 + 0.4 * 42 / 100 (M4).
    let k = crate::instability::InstabilitySettings::builtin().crater_scale(42);
    assert_eq!(
        radii(&mut w, &mut t, -10.0),
        vec![2.0 * 0.6 * k, 2.0],
        "hard rock, exact"
    );
    run_on(&mut w, &mut t, &[], 60);
    assert_eq!(
        radii(&mut w, &mut t, 10.0),
        vec![2.0 * 1.4 * k, 2.0],
        "weak rock, exact"
    );

    // Scraping charged rock drains 5 energy/s; floating does not.
    for (id, pos) in [
        (a, Vec3::new(0.0, 0.0, 10.0)),
        (b, Vec3::new(0.0, -14.6, 0.0)),
    ] {
        place(&mut w, id, pos, Vec3::NEG_Z);
        w.ship_mut(id).unwrap().energy = 50.0;
    }
    for _ in 0..60 {
        w.ship_mut(b).unwrap().vel = Vec3::new(0.0, -2.0, 0.0);
        run_on(&mut w, &mut t, &[], 1);
    }
    let (ea, eb) = (w.ship(a).unwrap().energy, w.ship(b).unwrap().energy);
    assert!(w.ship(b).unwrap().in_contact);
    assert!((ea - eb - 5.0).abs() < 0.5, "drain {ea} vs {eb}");
}

#[test]
fn projectile_wire_ids_skip_live_ones_after_wrapping() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_stick");
    // A sticky charge on the floor lives a while (30-tick fuse).
    place(&mut w, a, Vec3::new(0.0, -12.0, 0.0), Vec3::NEG_Y);
    run(&mut w, &mut t, &fire2(a), 10);
    let mine = w.projectiles()[0].id;
    // The counter comes round to the mine's wire id again.
    w.set_next_projectile_for_test(mine + 0x1_0000);
    arm(&mut w, &mut t, a, "test_pierce");
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &fire(a), 1);
    let ids: Vec<u32> = w.projectiles().iter().map(|p| p.id & 0xffff).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1], "wire ids stay unique");
}

#[test]
fn stick_fuse_shockwave_dust_and_debris() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_stick");
    // Fire down at the floor (y = -15).
    place(&mut w, a, Vec3::new(0.0, -5.0, 0.0), Vec3::NEG_Y);
    place(&mut w, b, Vec3::new(8.0, -12.0, 0.0), Vec3::Z);
    let mut ev = run(&mut w, &mut t, &fire2(a), 1);
    ev.extend(run(&mut w, &mut t, &[], 20));
    let p = w
        .projectiles()
        .first()
        .expect("stuck, not exploded")
        .clone();
    assert!(p.is_stuck());
    assert_eq!(
        p.net_state() & proj_flags::STUCK as u8,
        proj_flags::STUCK as u8
    );
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::TectonicArmed)));
    // Still there 10 ticks later, then the fuse detonates it.
    let stuck_at = p.pos;
    ev.extend(run(&mut w, &mut t, &[], 10));
    assert_eq!(w.projectiles().first().map(|p| p.pos), Some(stuck_at));
    let ev2 = run(&mut w, &mut t, &[], 25);
    assert!(w.projectiles().is_empty(), "the fuse ended it");
    assert!(ev2.iter().any(|e| matches!(e, Event::Impact { .. })));
    assert!(ev2
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Shockwave)));
    assert!(w.ship(b).unwrap().vel.length() > 0.0, "shockwave pushed b");
    assert!(w.entities().iter().any(|e| e.kind == EntityKind::DustCloud));
    assert!(w.entities().iter().any(|e| e.kind == EntityKind::Debris));
    // Dust expires after its lifetime.
    run(&mut w, &mut t, &[], 130);
    assert!(!w.entities().iter().any(|e| e.kind == EntityKind::DustCloud));
}

#[test]
fn expiry_splits_into_sub_projectiles() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_split");
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &fire(a), 1);
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, Event::Fired { .. }))
            .count(),
        1
    );
    let ev = run(&mut w, &mut t, &[], 30);
    let subs: Vec<&Projectile> = w
        .projectiles()
        .iter()
        .filter(|p| p.generation == 1)
        .collect();
    assert_eq!(subs.len(), 3);
    assert!(subs.iter().all(|p| p.damage == 1.0 && p.owner == a));
    assert!(
        !ev.iter().any(|e| matches!(e, Event::Fired { .. })),
        "sub-projectiles are not new shots"
    );
}

// ----- fire hooks ------------------------------------------------------------------

#[test]
fn instant_beam_drains_energy_without_a_projectile() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_beam");
    place(&mut w, a, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    w.ship_mut(a).unwrap().energy = 50.0;
    w.ship_mut(b).unwrap().energy = 50.0;
    let ev = run(&mut w, &mut t, &fire(a), 1);
    assert!(w.projectiles().is_empty());
    assert!(damage_to(&ev, b) > 0.0);
    assert!(w.ship(b).unwrap().energy < 45.0, "drained");
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Beam && c.target == Some(b))));
    assert!(
        w.ship(b).unwrap().vel.z < 0.0,
        "knockback proportional to damage"
    );
    // Out of reach: nothing.
    place(&mut w, b, Vec3::new(0.0, 0.0, -20.0), Vec3::Z);
    let ev = run(&mut w, &mut t, &fire(a), 30);
    assert_eq!(damage_to(&ev, b), 0.0);
}

#[test]
fn homing_turns_towards_the_target() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_homing");
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    // Target off to the side of the aim line.
    place(&mut w, b, Vec3::new(8.0, 0.0, -10.0), Vec3::Z);
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    for _ in 0..90 {
        ev.extend(run(&mut w, &mut t, &[], 1));
        place(&mut w, b, Vec3::new(8.0, 0.0, -10.0), Vec3::Z);
    }
    assert!(damage_to(&ev, b) > 0.0, "a straight bolt would have missed");
}

#[test]
fn guided_projectile_steers_with_owner_look_and_freezes_the_ship() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_guided");
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    run(&mut w, &mut t, &fire2(a), 1);
    let orientation = w.ship(a).unwrap().orientation;
    let steer = [(
        a,
        InputCmd {
            look_yaw: 0.02,
            ..InputCmd::idle(0)
        }
        .with_thrust(Vec3::NEG_Z),
    )];
    run(&mut w, &mut t, &steer, 20);
    let p = &w.projectiles()[0];
    assert!(
        p.vel.x < -0.5,
        "turned left with the look input: {:?}",
        p.vel
    );
    let s = w.ship(a).unwrap();
    assert!(
        s.orientation.angle_between(orientation) < 1e-5,
        "look went to the missile"
    );
    assert_eq!(s.vel, Vec3::ZERO, "no thrust while guiding");
}

#[test]
fn swap_trades_positions_and_momentum() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_swap");
    place(&mut w, a, Vec3::new(0.0, 0.0, 3.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    w.ship_mut(b).unwrap().vel = Vec3::new(5.0, 0.0, 0.0);
    run(&mut w, &mut t, &fire(a), 8);
    let (sa, sb) = (w.ship(a).unwrap(), w.ship(b).unwrap());
    assert!(
        sa.pos.z < 1.5 && sb.pos.z > 1.5,
        "{:?} {:?}",
        sa.pos,
        sb.pos
    );
    assert!(sa.vel.x > 1.0, "inherited the target's momentum");
}

#[test]
fn rewind_restores_the_past_and_takes_weapons_offline() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    arm(&mut w, &mut t, a, "test_rewind");
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    w.ship_mut(a).unwrap().hull = 80.0;
    run(&mut w, &mut t, &[], 1);
    let start = w.ship(a).unwrap().pos;
    let fly = [(a, InputCmd::idle(0).with_thrust(Vec3::X))];
    run(&mut w, &mut t, &fly, 60);
    w.ship_mut(a).unwrap().hull = 30.0;
    let moved = w.ship(a).unwrap().pos;
    assert!(moved.x > start.x + 5.0);
    let ev = run(&mut w, &mut t, &fire(a), 1);
    let s = w.ship(a).unwrap();
    assert!(s.pos.x < start.x + 1.0, "rewound: {:?}", s.pos);
    assert_eq!(s.hull, 80.0, "hull from 60 ticks ago");
    assert!(w.entities().iter().any(|e| e.kind == EntityKind::Ghost));
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Blinded)));
    // Weapons offline for 60 ticks.
    let laser = w.weapons().find("scrap_laser").unwrap();
    let sel = [(
        a,
        InputCmd {
            weapon_select: laser.0 + 1,
            ..InputCmd::idle(0)
        }
        .with_buttons(Buttons::FIRE_PRIMARY),
    )];
    let ev = run(&mut w, &mut t, &sel, 50);
    assert!(!ev.iter().any(|e| matches!(e, Event::Fired { .. })));
    let ev = run(&mut w, &mut t, &sel, 20);
    assert!(ev.iter().any(|e| matches!(e, Event::Fired { .. })));
    // The ghost expired and its weapon's entity_expired hook ran.
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Custom && c.value == 42)));
    assert!(!w.entities().iter().any(|e| e.kind == EntityKind::Ghost));
}

// ----- helpers -------------------------------------------------------------------------

#[test]
fn pattern_and_steering_helpers() {
    let rot = Quat::IDENTITY;
    assert_eq!(pattern_dir(rot, 0, 1, 10.0), Vec3::NEG_Z);
    let fan: Vec<Vec3> = (0..3).map(|i| pattern_dir(rot, i, 3, 10.0)).collect();
    assert!(fan[0].x.abs() > 0.1 && fan[1].x.abs() < 1e-6, "{fan:?}");
    assert!((fan[0].x + fan[2].x).abs() < 1e-6, "symmetric fan: {fan:?}");
    let ring: Vec<Vec3> = (0..8).map(|i| pattern_dir(rot, i, 8, 6.0)).collect();
    assert_eq!(ring[0], Vec3::NEG_Z);
    for d in &ring[1..] {
        let angle = ls_math::det::acos(d.dot(Vec3::NEG_Z).min(1.0)).to_degrees();
        assert!((angle - 6.0).abs() < 0.01, "{angle}");
    }
    let v = steer_towards(Vec3::new(0.0, 0.0, -10.0), Vec3::X, 0.1);
    assert!((v.length() - 10.0).abs() < 1e-4);
    assert!(v.x > 0.0 && v.x < 1.1);
    assert_eq!(
        steer_towards(Vec3::NEG_Z, Vec3::NEG_Z * 3.0, 0.1),
        Vec3::NEG_Z
    );
}
