//! World physics scenario tests (M4 package A4): gravity wells (static and
//! dynamic, the 40 % clamp, who is pulled), the level's up, Instability
//! (crater scale, strain, cave-ins), pigment and radiation.
//! `docs/milestone-4-plan.md` §3.3, §3.4, §3.8 and the wave A acceptance.

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit, TerrainQuery, MATERIAL_WEAK};
use ls_math::{det, Pcg32, Quat, Vec3};

use crate::gravity::{GravitySettings, WellSite};
use crate::instability::{escapes, InstabilitySettings};
use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

type Room = BoxRoomTerrain;

fn room(size: Vec3) -> Room {
    BoxRoomTerrain::centered(size)
}

fn spawns() -> Vec<SpawnPoint> {
    vec![
        SpawnPoint {
            pos: Vec3::new(-60.0, 0.0, 0.0),
            team: 0,
            orientation: Quat::IDENTITY,
        },
        SpawnPoint {
            pos: Vec3::new(60.0, 0.0, 0.0),
            team: 1,
            orientation: Quat::IDENTITY,
        },
    ]
}

fn world(setup: MatchSetup) -> World {
    World::new(WorldConfig::paint_the_void(7, setup)).unwrap()
}

fn world_rules(rules: RuleSet, setup: MatchSetup) -> World {
    let mut cfg = WorldConfig::paint_the_void(7, setup);
    cfg.rules = rules;
    World::new(cfg).unwrap()
}

fn builtin_rules(file: &str) -> RuleSet {
    let (f, text) = crate::ruleset::BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| f.ends_with(file))
        .unwrap();
    RuleSet::from_ron(f, text).unwrap()
}

fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
    s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
}

fn run<T: SimTerrain>(
    w: &mut World,
    t: &mut T,
    inputs: &[(PlayerId, InputCmd)],
    ticks: u32,
) -> Vec<Event> {
    let mut all = Vec::new();
    for _ in 0..ticks {
        all.extend(w.step(t, inputs));
    }
    all
}

fn cues(events: &[Event], kind: CueKind) -> Vec<Cue> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Cue(c) if c.kind == kind => Some(*c),
            _ => None,
        })
        .collect()
}

// ----- gravity -----------------------------------------------------------------

/// Seven wells at Gravity Chaos 100 (the most and strongest the generator
/// makes), 50 u apart in open space.
fn seven_wells() -> MatchSetup {
    MatchSetup {
        spawns: spawns(),
        gravity_chaos: 100,
        wells: (0..7)
            .map(|i| WellSite {
                pos: Vec3::new(-150.0 + 50.0 * i as f32, 0.0, 0.0),
                core_radius: 2.0,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn a_ship_at_full_thrust_escapes_every_well_at_gravity_chaos_100() {
    let setup = seven_wells();
    let wells = setup.wells.clone();
    let (_, range) = GravitySettings::builtin().for_slider(100);
    for ty in ShipType::ALL {
        let mut w = world(setup.clone());
        let mut t = room(Vec3::new(400.0, 200.0, 400.0));
        let a = w.add_player(Some(0), ty).unwrap();
        let r = w.tuning_for(ty).radius;
        for site in &wells {
            for dir in [Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::new(1.0, 1.0, 1.0)] {
                let dir = dir.normalize();
                // At the strongest pull: next to the core stone.
                let start = site.pos + dir * (site.core_radius + r + 0.05);
                place(&mut w, a, start, dir);
                // No boost: plain cruise thrust is enough.
                let fwd = [(
                    a,
                    InputCmd::idle(0)
                        .with_thrust(Vec3::new(0.0, 0.0, -1.0))
                        .with_buttons(Buttons::HORIZON_ASSIST),
                )];
                run(&mut w, &mut t, &fwd, 6 * 60);
                let s = w.ship(a).unwrap();
                let d = s.pos.distance(site.pos);
                assert!(d > range, "{ty:?} stuck at {d} u from {:?}", site.pos);
                // Thrust keeps most of the cruise speed against the pull.
                assert!(s.vel.length() > 10.0, "{ty:?}: {}", s.vel.length());
            }
        }
    }
}

#[test]
fn an_idle_ship_falls_toward_a_well_and_a_well_free_level_is_unchanged() {
    let mut w = world(seven_wells());
    let mut t = room(Vec3::new(400.0, 200.0, 400.0));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let site = Vec3::new(-150.0, 0.0, 0.0);
    place(&mut w, a, site + Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Z);
    let idle = [(a, InputCmd::idle(0))];
    run(&mut w, &mut t, &idle, 60);
    let s = w.ship(a).unwrap();
    assert!(s.pos.y < 9.0, "falls: {:?}", s.pos);
    assert!(s.vel.y < -1.0, "{:?}", s.vel);
    assert!(s.vel.x.abs() < 1e-3 && s.vel.z.abs() < 1e-3, "{:?}", s.vel);
    // Without wells the same ship stays put (the M3 flight model).
    let mut w = world(MatchSetup {
        spawns: spawns(),
        gravity_chaos: 100,
        ..Default::default()
    });
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, site + Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &idle, 60);
    assert_eq!(w.ship(a).unwrap().pos, site + Vec3::new(0.0, 10.0, 0.0));
    // Gravity Chaos 0: sites pull nothing.
    let mut quiet = seven_wells();
    quiet.gravity_chaos = 0;
    let w = world(quiet);
    assert!(w.well_field().is_empty());
}

/// Fires `weapon` from `from` along +X past a well 6 u off the line; returns
/// the projectile's velocity direction change (radians) after `ticks`.
fn bend(weapon: &str, ticks: u32) -> f32 {
    let mut setup = MatchSetup {
        spawns: spawns(),
        gravity_chaos: 100,
        wells: vec![WellSite {
            pos: Vec3::new(0.0, 6.0, 0.0),
            core_radius: 1.5,
        }],
        ..Default::default()
    };
    setup.spawns[0].pos = Vec3::new(-30.0, 0.0, 0.0);
    let mut rules = RuleSet::paint_the_void();
    rules
        .weapons
        .insert(weapon.to_string(), Availability::LoadoutAndPickup);
    let mut w = world_rules(rules, setup);
    let mut t = room(Vec3::new(400.0, 200.0, 400.0));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    w.give_weapon(a, weapon).unwrap();
    let wid = w.weapons().find(weapon).unwrap();
    place(&mut w, a, Vec3::new(-30.0, 0.0, 0.0), Vec3::X);
    run(
        &mut w,
        &mut t,
        &[(
            a,
            InputCmd {
                weapon_select: wid.0 + 1,
                ..InputCmd::idle(0)
            },
        )],
        1,
    );
    let button = match w.weapons().get(wid).slot {
        WeaponSlot::Primary => Buttons::FIRE_PRIMARY,
        WeaponSlot::Secondary => Buttons::FIRE_SECONDARY,
    };
    run(
        &mut w,
        &mut t,
        &[(a, InputCmd::idle(0).with_buttons(button))],
        1,
    );
    let p0 = w
        .projectiles()
        .iter()
        .find(|p| p.weapon == wid)
        .expect("fired")
        .clone();
    // The shooter moves out of the way and stays idle.
    place(&mut w, a, Vec3::new(-30.0, -60.0, 0.0), Vec3::X);
    run(&mut w, &mut t, &[], ticks);
    let p1 = w
        .projectiles()
        .iter()
        .find(|p| p.id == p0.id)
        .expect("still flying");
    let (d0, d1) = (p0.vel.normalize(), p1.vel.normalize());
    det::acos(d0.dot(d1).clamp(-1.0, 1.0))
}

#[test]
fn bolts_and_kinetic_rounds_fly_straight_missiles_bend() {
    assert_eq!(bend("scrap_laser", 50), 0.0, "energy bolt");
    assert_eq!(bend("plasma_repeater", 50), 0.0, "energy bolt");
    assert_eq!(bend("rivet_gun", 10), 0.0, "kinetic round");
    let rocket = bend("bolt_rocket", 50);
    assert!(rocket > 0.02, "bolt rocket bends: {rocket}");
    let heavy = bend("heavy_rocket", 90);
    assert!(heavy > 0.02, "heavy rocket bends: {heavy}");
    // Every weapon's flag: ordnance and the Seed Grenade, nothing else.
    let table = WeaponTable::builtin();
    for (_, d) in table.iter() {
        let expect = d.family == WeaponFamily::Ordnance || d.id == "seed_grenade";
        assert_eq!(d.gravity, expect, "{}", d.id);
    }
}

#[test]
fn debris_is_pulled_and_wakes_up() {
    let mut w = world(MatchSetup {
        spawns: spawns(),
        gravity_chaos: 60,
        wells: vec![WellSite {
            pos: Vec3::ZERO,
            core_radius: 2.0,
        }],
        ..Default::default()
    });
    let mut t = room(Vec3::new(400.0, 200.0, 400.0));
    let mut e = EntitySpawn::debris(Vec3::new(8.0, 0.0, 0.0), Vec3::ZERO, 0.5, 3.0);
    e.vel = Vec3::ZERO;
    let id = w.spawn_entity(e);
    run(&mut w, &mut t, &[], 60);
    let d = w.entities().iter().find(|e| e.id == id).unwrap();
    assert!(d.pos.x < 7.5, "pulled in: {:?}", d.pos);
    assert!(!d.resting);
}

#[test]
fn dynamic_wells_start_and_end_on_their_ticks_and_stack_only_to_the_clamp() {
    let mut w = world(MatchSetup {
        spawns: spawns(),
        ..Default::default()
    });
    let mut t = room(Vec3::new(400.0, 200.0, 400.0));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::new(0.0, 3.0, 0.0), Vec3::NEG_Z);
    let idle = [(a, InputCmd::idle(0))];
    let now = w.tick();
    let well = |key: u32, pos: Vec3| DynamicWell {
        key,
        pos,
        peak_accel: 9.0,
        range: 16.0,
        start_tick: now + 30,
        end_tick: now + 90,
        owner: Some(a),
    };
    w.add_well(well(1, Vec3::ZERO));
    // Not yet.
    run(&mut w, &mut t, &idle, 29);
    assert_eq!(w.ship(a).unwrap().pos, Vec3::new(0.0, 3.0, 0.0));
    assert_eq!(w.dynamic_wells().len(), 1);
    // Pulls from its start tick (its owner too).
    run(&mut w, &mut t, &idle, 1);
    assert!(w.ship(a).unwrap().vel.y < 0.0);
    run(&mut w, &mut t, &idle, 59);
    assert_eq!(w.dynamic_wells().len(), 1);
    let v = w.ship(a).unwrap().vel;
    // Gone at its end tick: the dampers take over.
    run(&mut w, &mut t, &idle, 1);
    assert!(w.dynamic_wells().is_empty());
    let v2 = w.ship(a).unwrap().vel;
    assert!(v2.length() < v.length(), "{v:?} {v2:?}");

    // A ship between three overlapping wells still escapes at full thrust.
    let now = w.tick();
    for (k, p) in [
        Vec3::new(-2.5, 0.0, 0.0),
        Vec3::new(2.5, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 2.5),
    ]
    .into_iter()
    .enumerate()
    {
        w.add_well(DynamicWell {
            start_tick: now,
            end_tick: now + 10_000,
            ..well(10 + k as u32, p)
        });
    }
    let raw = w.well_field().accel_at(Vec3::new(0.0, 0.0, -1.0), now + 1);
    let clamped = w
        .well_field()
        .ship_accel(Vec3::new(0.0, 0.0, -1.0), now + 1, 30.0);
    assert!(raw.length() > 12.0, "stacked: {}", raw.length());
    assert!((clamped.length() - 12.0).abs() < 1e-4);
    place(&mut w, a, Vec3::new(0.0, 0.0, -1.0), Vec3::NEG_Z);
    let fwd = [(a, InputCmd::idle(0).with_thrust(Vec3::new(0.0, 0.0, -1.0)))];
    run(&mut w, &mut t, &fwd, 4 * 60);
    assert!(
        w.ship(a).unwrap().pos.z < -16.0,
        "{:?}",
        w.ship(a).unwrap().pos
    );
    // Removing by key, and bad wells are ignored.
    for k in 10..13 {
        w.remove_well(k);
    }
    assert!(w.dynamic_wells().is_empty());
    w.add_well(DynamicWell {
        start_tick: now,
        end_tick: now,
        ..well(99, Vec3::ZERO)
    });
    assert!(w.dynamic_wells().is_empty());
}

#[test]
fn horizon_assist_levels_to_the_turned_up() {
    let mut w = world(MatchSetup {
        spawns: spawns(),
        ..Default::default()
    });
    let mut t = room(Vec3::new(400.0, 200.0, 400.0));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    assert_eq!(w.up(), Vec3::Y);
    w.set_up(Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(w.up(), Vec3::X);
    run(
        &mut w,
        &mut t,
        &[(a, InputCmd::idle(0).with_buttons(Buttons::HORIZON_ASSIST))],
        6 * 60,
    );
    let q = w.ship(a).unwrap().orientation;
    // The ship's up now points along +X (rolled 90 degrees).
    assert!(q.up().dot(Vec3::X) > 0.99, "{:?}", q.up());
    w.set_up(Vec3::ZERO);
    assert_eq!(w.up(), Vec3::X, "a zero up is ignored");
}

// ----- instability ---------------------------------------------------------------

/// A low, wide tunnel: floor at y = -4, ceiling at y = 4.
fn tunnel() -> Room {
    room(Vec3::new(200.0, 8.0, 200.0))
}

fn unstable(instability: u8) -> World {
    world(MatchSetup {
        spawns: spawns(),
        instability,
        ..Default::default()
    })
}

/// Bolt Rocket craters (radius 1, scaled like the real thing) on the floor
/// at one spot every `every` ticks; returns how many it took until the
/// cave-in warning (`None` after `max`).
fn craters_until_cave_in(instability: u8, every: u32, max: u32) -> Option<u32> {
    let mut w = unstable(instability);
    let mut t = tunnel();
    let spot = Vec3::new(10.0, -4.0, 10.0);
    for n in 1..=max {
        let (_, ev) = w.hook_for_test(&mut t, |ctx| ctx.crater(spot, 1.0));
        assert!(cues(&ev, CueKind::CaveInWarning).is_empty());
        let ev = run(&mut w, &mut t, &[], every);
        if !cues(&ev, CueKind::CaveInWarning).is_empty() {
            return Some(n);
        }
    }
    None
}

#[test]
fn cave_ins_follow_the_tuning_targets_and_never_happen_at_instability_0() {
    // §3.3: about 10 Bolt Rocket craters in one spot within 20 s at 30,
    // about 4 at 100, never at 0.
    let at30 = craters_until_cave_in(30, 120, 30).unwrap();
    let at100 = craters_until_cave_in(100, 120, 30).unwrap();
    assert!((9..=11).contains(&at30), "Instability 30: {at30} craters");
    assert!((3..=5).contains(&at100), "Instability 100: {at100} craters");
    // Faster fire (every 0.75 s, the Bolt Rocket's interval) needs no more.
    let fast30 = craters_until_cave_in(30, 45, 30).unwrap();
    assert!(fast30 <= at30, "{fast30}");
    assert!(craters_until_cave_in(0, 30, 200).is_none());
    // Spread out (one per 16 u cell) they never add up.
    let mut w = unstable(100);
    let mut t = tunnel();
    for i in 0..10 {
        let p = Vec3::new(-80.0 + 17.0 * i as f32, -4.0, 30.0);
        w.hook_for_test(&mut t, |ctx| ctx.crater(p, 1.0));
        let ev = run(&mut w, &mut t, &[], 60);
        assert!(cues(&ev, CueKind::CaveInWarning).is_empty());
    }
}

#[test]
fn craters_grow_with_instability() {
    for (instability, scale) in [(0u8, 1.0f32), (50, 1.2), (100, 1.4)] {
        let mut w = unstable(instability);
        let mut t = tunnel();
        let (_, ev) = w.hook_for_test(&mut t, |ctx| ctx.crater(Vec3::new(0.0, -4.0, 0.0), 2.0));
        let r = ev
            .iter()
            .find_map(|e| match e {
                Event::TerrainEdit(TerrainEdit::SubtractSphere { radius, .. }) => Some(*radius),
                _ => None,
            })
            .unwrap();
        assert!((r - 2.0 * scale).abs() < 1e-5, "{instability}: {r}");
    }
}

/// Fills a cell to the threshold and steps through the warning; returns the
/// events of the cave-in tick and the world.
fn cave_in_at(spot: Vec3, w: &mut World, t: &mut Room) -> (Vec<Event>, Vec<Event>) {
    let threshold = InstabilitySettings::builtin().threshold(100);
    let mut warn = Vec::new();
    let mut n = 0;
    while warn.is_empty() {
        w.hook_for_test(t, |ctx| ctx.crater(spot, 1.0));
        warn = cues(&run(w, t, &[], 1), CueKind::CaveInWarning);
        n += 1;
        assert!(n < 50, "no cave-in (threshold {threshold})");
    }
    let c = warn[0];
    assert_eq!(c.value, 90, "1.5 s warning");
    let mut before = Vec::new();
    for _ in 0..89 {
        before.extend(w.step(t, &[]));
    }
    assert!(
        !before
            .iter()
            .any(|e| matches!(e, Event::TerrainEdit(TerrainEdit::AddSphere { .. }))),
        "nothing falls during the warning"
    );
    (warn.into_iter().map(Event::Cue).collect(), w.step(t, &[]))
}

#[test]
fn a_cave_in_drops_weak_rubble_from_the_ceiling_after_its_warning() {
    let mut w = unstable(100);
    let mut t = tunnel();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let spot = Vec3::new(10.0, -4.0, 10.0);
    // A ship right under the ceiling where the rock comes down.
    place(&mut w, a, Vec3::new(11.0, 1.5, 10.0), Vec3::NEG_Z);
    let (_, ev) = cave_in_at(spot, &mut w, &mut t);
    let adds: Vec<(Vec3, f32)> = ev
        .iter()
        .filter_map(|e| match e {
            Event::TerrainEdit(TerrainEdit::AddSphere { center, radius }) => {
                Some((*center, *radius))
            }
            _ => None,
        })
        .collect();
    let hollows = ev
        .iter()
        .filter(|e| matches!(e, Event::TerrainEdit(TerrainEdit::SubtractSphere { .. })))
        .count();
    assert_eq!(hollows, 1, "a hollow in the ceiling");
    assert!(!adds.is_empty() && adds.len() <= 9, "{adds:?}");
    for (c, r) in &adds {
        assert!((1.5..=3.0).contains(r));
        assert!(c.y > 0.0, "hangs from the ceiling: {c:?}");
        // Weak rock (rubble region).
        assert_eq!(w.instability().material_at(*c), Some(MATERIAL_WEAK));
    }
    let debris = w
        .entities()
        .iter()
        .filter(|e| e.kind == EntityKind::Debris)
        .count();
    assert!((4..=8).contains(&debris), "{debris} debris");
    // Nothing overlaps the ship; it took 20 damage and can still get out.
    let s = w.ship(a).unwrap();
    assert!(
        !t.sphere_overlap(s.pos, 0.5),
        "rubble left out over the ship"
    );
    let hurt: f32 = ev
        .iter()
        .filter_map(|e| match e {
            Event::Damaged {
                target,
                amount,
                attacker: None,
                ..
            } if *target == a => Some(*amount),
            _ => None,
        })
        .sum();
    assert_eq!(hurt, 20.0);
    assert!(escapes(&t, &[], s.pos, 0.5, 14.0));
    // At most one per 8 s: more strain right away waits.
    for _ in 0..30 {
        w.hook_for_test(&mut t, |ctx| ctx.crater(spot, 1.0));
    }
    let ev = run(&mut w, &mut t, &[], 6 * 60);
    assert!(cues(&ev, CueKind::CaveInWarning).is_empty());
    let ev = run(&mut w, &mut t, &[], 2 * 60 + 10);
    assert_eq!(cues(&ev, CueKind::CaveInWarning).len(), 1);
}

#[test]
fn no_cave_in_near_spawn_points() {
    let mut w = unstable(100);
    let mut t = tunnel();
    let spot = Vec3::new(-60.0, -4.0, 5.0);
    for _ in 0..40 {
        w.hook_for_test(&mut t, |ctx| ctx.crater(spot, 1.0));
        let ev = run(&mut w, &mut t, &[], 10);
        assert!(cues(&ev, CueKind::CaveInWarning).is_empty());
    }
}

/// Property: over 8 seeds with ships scattered around the spot (some in
/// dead-end pockets), a cave-in never overlaps or encloses a ship.
#[test]
fn cave_ins_never_enclose_a_ship() {
    let mut rubble_total = 0;
    for seed in 0..8u64 {
        let mut rng = Pcg32::new(seed, 99);
        let mut w = world(MatchSetup {
            spawns: spawns(),
            instability: 100,
            ..Default::default()
        });
        let mut t = tunnel();
        let spot = Vec3::new(rng.range_f32(-20.0, 20.0), -4.0, rng.range_f32(-20.0, 20.0));
        // A pocket: a short side tunnel walled off on three sides by rock
        // grown next to the spot, so a ship inside has one way out.
        let pocket = spot + Vec3::new(rng.range_f32(-2.0, 2.0), 3.0, rng.range_f32(-2.0, 2.0));
        for (dx, dz) in [(3.0, 0.0), (-3.0, 0.0), (0.0, 3.0)] {
            t.apply_edit(&TerrainEdit::AddSphere {
                center: pocket + Vec3::new(dx, 0.0, dz),
                radius: 2.0,
            });
        }
        let mut ids = Vec::new();
        for i in 0..6 {
            let ty = ShipType::ALL[i % 4];
            let id = w.add_player(None, ty).unwrap();
            let r = w.tuning_for(ty).radius;
            // Rejection-sample an open spot (the first ship in the pocket).
            let mut p = pocket;
            for _ in 0..100 {
                if i > 0 {
                    p = spot
                        + Vec3::new(
                            rng.range_f32(-6.0, 6.0),
                            rng.range_f32(0.5, 3.0),
                            rng.range_f32(-6.0, 6.0),
                        );
                }
                if t.sdf(p) > r + 0.1 {
                    break;
                }
                p += Vec3::new(0.0, 0.5, 0.0);
            }
            assert!(t.sdf(p) > r, "seed {seed}: no open spot");
            place(&mut w, id, p, Vec3::NEG_Z);
            ids.push(id);
        }
        let could: Vec<bool> = ids
            .iter()
            .map(|id| {
                let s = w.ship(*id).unwrap();
                escapes(&t, &[], s.pos, w.tuning_for(s.ship_type).radius, 14.0)
            })
            .collect();
        // Craters until the warning, then the ships hold still while the
        // rock comes down.
        let threshold = InstabilitySettings::builtin().threshold(100);
        let mut fell = false;
        let mut warned = None;
        for i in 0..400 {
            if warned.is_none() {
                w.hook_for_test(&mut t, |ctx| ctx.crater(spot, 1.0));
            }
            let pos: Vec<Vec3> = ids.iter().map(|id| w.ship(*id).unwrap().pos).collect();
            let ev = w.step(&mut t, &[]);
            for (id, p) in ids.iter().zip(&pos) {
                place(&mut w, *id, *p, Vec3::NEG_Z);
            }
            if warned.is_none() && !cues(&ev, CueKind::CaveInWarning).is_empty() {
                warned = Some(i);
            }
            if warned.is_some_and(|wt| i == wt + 90) {
                fell = true;
                rubble_total += ev
                    .iter()
                    .filter(|e| matches!(e, Event::TerrainEdit(TerrainEdit::AddSphere { .. })))
                    .count();
                break;
            }
        }
        assert!(fell, "seed {seed}: no cave-in (threshold {threshold})");
        for (id, could) in ids.iter().zip(could) {
            let Some(s) = w.ship(*id).filter(|s| s.alive) else {
                continue;
            };
            let r = w.tuning_for(s.ship_type).radius;
            assert!(
                t.sdf(s.pos) > r - 1e-3,
                "seed {seed}: rubble on {id:?} at {:?}: {}",
                s.pos,
                t.sdf(s.pos)
            );
            if could {
                assert!(
                    escapes(&t, &[], s.pos, r, 14.0),
                    "seed {seed}: {id:?} enclosed at {:?}",
                    s.pos
                );
            }
        }
    }
    assert!(
        rubble_total >= 8,
        "the rock really came down: {rubble_total}"
    );
}

#[test]
fn instability_state_is_hashed_and_cloned() {
    let mut w = unstable(100);
    let mut t = tunnel();
    let h0 = w.state_hash();
    w.hook_for_test(&mut t, |ctx| ctx.crater(Vec3::new(0.0, -4.0, 0.0), 1.0));
    assert_ne!(w.state_hash(), h0, "strain is state");
    let mut c = w.clone();
    let mut t2 = t.clone();
    run(&mut w, &mut t, &[], 30);
    run(&mut c, &mut t2, &[], 30);
    assert_eq!(w.state_hash(), c.state_hash());
}

// ----- pigment --------------------------------------------------------------------

fn mono_setup(points: usize) -> MatchSetup {
    MatchSetup {
        spawns: spawns(),
        pickup_points: (0..points)
            .map(|i| Vec3::new(-40.0 + 10.0 * i as f32, 20.0, 0.0))
            .collect(),
        monochrome: true,
        ..Default::default()
    }
}

fn no_weapon_pickups(mut r: RuleSet) -> RuleSet {
    r.pickups.clear();
    r.upgrade_pickups.clear();
    r.gadget_pickups.clear();
    r.scrap_pickup = None;
    r
}

#[test]
fn pigment_pickups_spawn_per_players_and_only_on_monochrome_levels() {
    let rules = no_weapon_pickups(RuleSet::paint_the_void());
    let mut t = room(Vec3::new(200.0, 100.0, 200.0));
    // Not monochrome: none.
    let mut plain = mono_setup(8);
    plain.monochrome = false;
    let mut w = world_rules(rules.clone(), plain);
    w.add_player(None, ShipType::Tinker).unwrap();
    run(&mut w, &mut t, &[], 2);
    assert!(w.pigment_pickups().is_empty());
    // Monochrome: at least 2, one per 4 players.
    let mut w = world_rules(rules, mono_setup(8));
    for _ in 0..4 {
        w.add_player(None, ShipType::Tinker).unwrap();
    }
    run(&mut w, &mut t, &[], 1);
    assert_eq!(w.pigment_pickups().len(), 2);
    for _ in 0..5 {
        w.add_player(None, ShipType::Tinker).unwrap();
    }
    run(&mut w, &mut t, &[], 1);
    assert_eq!(w.pigment_pickups().len(), 3);
    let views: Vec<EntityView> = w
        .entity_views()
        .into_iter()
        .filter(|v| v.kind == ReplicatedKind::PigmentPickup)
        .collect();
    assert_eq!(views.len(), 3);
    assert!(views.iter().all(|v| v.state[0] == 0));
    // Distinct points.
    let mut pts: Vec<u16> = w.pigment_pickups().iter().filter_map(|p| p.point).collect();
    pts.sort_unstable();
    pts.dedup();
    assert_eq!(pts.len(), 3);
}

#[test]
fn pigment_colours_a_ship_adds_damage_expires_and_drops_on_death() {
    let rules = no_weapon_pickups(RuleSet::paint_the_void());
    let mut w = world_rules(rules, mono_setup(4));
    let mut t = room(Vec3::new(200.0, 100.0, 200.0));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let c = w.add_player(Some(1), ShipType::Tinker).unwrap();
    place(&mut w, b, Vec3::new(0.0, -20.0, 30.0), Vec3::NEG_Z);
    place(&mut w, c, Vec3::new(10.0, -20.0, 30.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 1);
    let pk = w.pigment_pickups()[0];
    place(&mut w, a, pk.pos, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &[], 1);
    let taken = cues(&ev, CueKind::Pigment);
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].source, Some(a));
    let now = w.tick();
    let s = w.ship(a).unwrap();
    assert!(s.pigmented(now));
    assert_eq!(s.pigment_until, now + 40 * 60);
    // +25 % damage from a pigmented attacker.
    let hit = |w: &mut World, t: &mut Room, from: PlayerId| {
        let src = DamageSource {
            attacker: Some(from),
            team: w.ship(from).unwrap().team,
            weapon: None,
        };
        let (_, ev) = w.hook_for_test(t, |ctx| ctx.damage(b, 10.0, Vec3::ZERO, src, true));
        ev.iter()
            .find_map(|e| match e {
                Event::Damaged { amount, .. } => Some(*amount),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(hit(&mut w, &mut t, a), 12.5);
    w.ship_mut(b).unwrap().hull = 100.0;
    // The respawned pickup comes back 30 s later at a free point.
    assert_eq!(w.pigment_pickups().len(), 1);
    run(&mut w, &mut t, &[], 30 * 60 - 1);
    assert_eq!(w.pigment_pickups().len(), 1);
    run(&mut w, &mut t, &[], 1);
    assert_eq!(w.pigment_pickups().len(), 2);
    // Expiry after 40 s.
    place(&mut w, a, Vec3::new(0.0, -30.0, 0.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 10 * 60);
    assert!(!w.ship(a).unwrap().pigmented(w.tick()));
    assert_eq!(hit(&mut w, &mut t, a), 10.0);
    // Pigmented again, then destroyed: a splash in its colour for 15 s.
    let pk = w.pigment_pickups()[0];
    place(&mut w, a, pk.pos, Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 1);
    assert!(w.ship(a).unwrap().pigmented(w.tick()));
    let wreck = w.ship(a).unwrap().pos;
    w.hook_for_test(&mut t, |ctx| {
        ctx.adjust_hull(a, -1000.0, true);
    });
    assert!(!w.ship(a).unwrap().alive);
    assert_eq!(w.ship(a).unwrap().pigment_until, 0);
    let splash = *w
        .pigment_pickups()
        .iter()
        .find(|p| p.point.is_none())
        .expect("splash");
    assert_eq!(splash.pos, wreck);
    assert_eq!(splash.side, 0);
    let view = w
        .entity_views()
        .into_iter()
        .find(|v| v.id == splash.id)
        .unwrap();
    assert_eq!(view.state, [1, 0, 0, 0]);
    // An enemy picks the splash up and is pigmented.
    place(&mut w, c, splash.pos, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &[], 1);
    assert_eq!(cues(&ev, CueKind::Pigment)[0].value, 1);
    assert!(w.ship(c).unwrap().pigmented(w.tick()));
    assert!(w.pigment_pickups().iter().all(|p| p.point.is_some()));
}

#[test]
fn a_splash_nobody_takes_dries_up_after_15_s() {
    let rules = no_weapon_pickups(RuleSet::paint_the_void());
    let mut w = world_rules(rules, mono_setup(4));
    let mut t = room(Vec3::new(200.0, 100.0, 200.0));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    run(&mut w, &mut t, &[], 1);
    w.ship_mut(a).unwrap().pigment_until = w.tick() + 100;
    place(&mut w, a, Vec3::new(0.0, -40.0, 0.0), Vec3::NEG_Z);
    w.hook_for_test(&mut t, |ctx| ctx.adjust_hull(a, -1000.0, true));
    assert_eq!(
        w.pigment_pickups()
            .iter()
            .filter(|p| p.point.is_none())
            .count(),
        1
    );
    // Respawn elsewhere and stay away.
    run(&mut w, &mut t, &[], 15 * 60 - 1);
    assert_eq!(
        w.pigment_pickups()
            .iter()
            .filter(|p| p.point.is_none())
            .count(),
        1
    );
    run(&mut w, &mut t, &[], 1);
    assert_eq!(
        w.pigment_pickups()
            .iter()
            .filter(|p| p.point.is_none())
            .count(),
        0
    );
}

#[test]
fn last_color_standing_pigment_restores_colour_without_a_bonus() {
    let rules = no_weapon_pickups(builtin_rules("last_color_standing.ron"));
    let mut setup = mono_setup(4);
    setup.monochrome = false; // Last Color Standing is monochrome anyway.
    let mut w = world_rules(rules, setup);
    let mut t = room(Vec3::new(200.0, 100.0, 200.0));
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    place(&mut w, b, Vec3::new(0.0, -30.0, 0.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 1);
    assert_eq!(
        w.pigment_pickups().len(),
        1,
        "one per 4 players, at least 1"
    );
    w.mode_mut().players.get_mut(&a).unwrap().meter = 66;
    let pk = w.pigment_pickups()[0];
    place(&mut w, a, pk.pos, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &[], 1);
    assert_eq!(cues(&ev, CueKind::Pigment).len(), 1);
    assert_eq!(w.mode().player(a).unwrap().meter, 83);
    assert_eq!(w.mode().player(a).unwrap().score, 83);
    assert!(
        !w.ship(a).unwrap().pigmented(w.tick()),
        "no pigmented state"
    );
    // Capped at the starting colour; the next one comes after 45 s.
    run(&mut w, &mut t, &[], 45 * 60 - 1);
    assert!(w.pigment_pickups().is_empty());
    run(&mut w, &mut t, &[], 1);
    let pk = w.pigment_pickups()[0];
    w.mode_mut().players.get_mut(&a).unwrap().meter = 95;
    place(&mut w, a, pk.pos, Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 1);
    assert_eq!(w.mode().player(a).unwrap().meter, 100);
}

// ----- radiation -------------------------------------------------------------------

fn belt_world() -> (World, Room) {
    let belt = crate::belt::tests::plan_belt();
    let setup = MatchSetup {
        spawns: belt.slots.clone(),
        belt: Some(belt),
        ..Default::default()
    };
    let mut rules = no_weapon_pickups(builtin_rules("team_brawl.ron"));
    rules.mutators.clear();
    let w = world_rules(rules, setup);
    (w, room(Vec3::new(340.0, 120.0, 340.0)))
}

#[test]
fn radiation_burns_hull_outside_the_belt_only_on_belt_levels() {
    let (mut w, mut t) = belt_world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let belt = crate::belt::tests::plan_belt();
    place(&mut w, a, belt.point(0.5, 120.0, 0.0), Vec3::NEG_Z);
    place(&mut w, b, belt.point(2.0, 120.0, 22.0), Vec3::NEG_Z);
    let (ha, hb) = (w.ship(a).unwrap().hull, w.ship(b).unwrap().hull);
    run(&mut w, &mut t, &[], 60);
    assert_eq!(w.ship(a).unwrap().hull, ha, "deep in the belt");
    // 10 u above the surface: 2 + 0.4 * 10 = 6 hull/s.
    let lost = hb - w.ship(b).unwrap().hull;
    assert!((lost - 6.0).abs() < 0.01, "{lost}");
    assert!((w.dose_rate(w.ship(b).unwrap().pos) - 6.0).abs() < 1e-3);
    // A classic level has no radiation.
    let mut w = world(MatchSetup {
        spawns: spawns(),
        ..Default::default()
    });
    let c = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, c, Vec3::new(0.0, 40.0, 0.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 120);
    assert_eq!(w.ship(c).unwrap().hull, w.tuning().hull_max);
    assert_eq!(w.dose_rate(Vec3::new(0.0, 40.0, 0.0)), 0.0);
}

#[test]
fn radiation_passes_a_raised_shield_and_credits_nobody() {
    let (mut w, mut t) = belt_world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let belt = crate::belt::tests::plan_belt();
    w.give_gadget(a, "shield").unwrap();
    place(&mut w, a, belt.point(1.0, 120.0, 60.0), Vec3::NEG_Z);
    place(&mut w, b, belt.point(4.0, 120.0, 0.0), Vec3::NEG_Z);
    let press = [(a, InputCmd::idle(0).with_buttons(Buttons::GADGET))];
    run(&mut w, &mut t, &press, 1);
    assert!(w.ship(a).unwrap().gadget_active, "shield up");
    w.ship_mut(a).unwrap().energy = 100.0;
    let h0 = w.ship(a).unwrap().hull;
    run(&mut w, &mut t, &press, 60);
    let lost = h0 - w.ship(a).unwrap().hull;
    assert!(
        (lost - 16.0).abs() < 0.01,
        "the cap, through the shield: {lost}"
    );
    // b damaged a last; radiation kills a and credits nobody.
    w.ship_mut(a).unwrap().last_attacker = Some(b);
    w.ship_mut(a).unwrap().hull = 1.0;
    let ev = run(&mut w, &mut t, &press, 10);
    let killed: Vec<&Event> = ev
        .iter()
        .filter(|e| matches!(e, Event::Killed { .. }))
        .collect();
    assert_eq!(
        killed,
        vec![&Event::Killed {
            victim: a,
            killer: None,
            weapon: None
        }]
    );
    assert_eq!(w.ship(b).unwrap().kills, 0);
    assert!(!ev.iter().any(|e| matches!(e, Event::Damaged { .. })));
}
