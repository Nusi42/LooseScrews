//! The Squeeze scenario tests: timeline and radii, nested zones from the
//! seed, the shell as solid terrain, crushing (ignores shields), swallowed
//! ships, wall scrap, one life and the last ship flying, the default rule
//! set and a golden-hash replay.

use ls_math::terrain::{BoxRoomTerrain, TerrainQuery};
use ls_math::{Pcg32, Quat, Vec3};

use super::*;
use crate::mode::{player_flags, MatchPhase};
use crate::modes::module;
use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

type Arena = ShelledTerrain<BoxRoomTerrain>;

/// A 60 x 30 x 60 open box around the origin with a shell layer.
fn arena() -> Arena {
    ShelledTerrain::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)))
}

/// Eight spawn points on a ring and a few pickup points.
fn setup() -> MatchSetup {
    let spawns = (0..8)
        .map(|i| {
            let a = i as f32 * 0.785;
            SpawnPoint {
                pos: Vec3::new(
                    12.0 * ls_math::det::cos(a),
                    if i % 2 == 0 { -4.0 } else { 4.0 },
                    12.0 * ls_math::det::sin(a),
                ),
                team: (i % 4) as u8,
                orientation: Quat::IDENTITY,
            }
        })
        .collect();
    MatchSetup {
        wells: Vec::new(),
        belt: None,
        spawns,
        pickup_points: vec![
            Vec3::new(-6.0, 0.0, 3.0),
            Vec3::new(5.0, 2.0, -7.0),
            Vec3::new(1.0, -3.0, 9.0),
        ],
        instability: 0,
        absurdity: 0,
        heist: None,
        ..Default::default()
    }
}

/// The default rule set with short phases (2 s, then 1 s warning and 2 s
/// close per phase; 17 s plus an 8 s final close).
fn fast_rules() -> RuleSet {
    let mut r = RuleSet::default_for(ModeKind::TheSqueeze);
    r.mode.set_field("first_phase_secs", 2.0);
    r.mode.set_field("warn_secs", 1.0);
    r.mode.set_field("close_secs", 2.0);
    r
}

/// A direction from the shell's centre whose point at `dist` lies inside
/// the room (3 u from its walls).
fn wall_dir(t: &Arena, shell: ShellSphere, dist: f32) -> Vec3 {
    let room = t.bounds().expand(-3.0);
    [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Z,
        Vec3::NEG_Z,
        Vec3::Y,
        Vec3::NEG_Y,
    ]
    .into_iter()
    .find(|d| room.contains(shell.center + *d * dist))
    .expect("a point inside the room")
}

fn squeeze(r: &RuleSet) -> TheSqueezeRules {
    match r.mode {
        ModeRules::TheSqueeze(m) => m,
        _ => panic!("not a Squeeze rule set"),
    }
}

fn world(seed: u64, rules: RuleSet) -> World {
    let mut cfg = WorldConfig::paint_the_void(seed, setup());
    cfg.rules = rules;
    World::new(cfg).unwrap()
}

/// The tick the final close ends (everything sealed).
fn sealed_tick(r: &TheSqueezeRules) -> u64 {
    close_window(r, r.phases + 1).1
}

/// Runs a match without players to its sealed end; returns every zone the
/// timeline announced (phase order) and checks the shell on the way.
fn run_timeline(seed: u64) -> Vec<Zone> {
    let rules = fast_rules();
    let r = squeeze(&rules);
    let mut w = world(seed, rules);
    let mut t = arena();
    let mut prev: Option<ShellSphere> = None;
    let mut zones: Vec<Zone> = Vec::new();
    for _ in 0..sealed_tick(&r) + 5 {
        w.step(&mut t, &[]);
        let now = w.tick();
        let shell = w.terrain_shell();
        // The terrain got the same shell at the start of the tick (the
        // schedule is continuous across stage changes).
        assert_eq!(t.shell, shell, "tick {now}");
        assert_eq!(w.mode().stage, stage_at(&r, now), "tick {now}");
        if now < close_window(&r, 1).0 {
            assert!(shell.is_none(), "no walls before the first close ({now})");
        }
        if let (Some(a), Some(b)) = (prev, shell) {
            let shift = a.center.distance(b.center);
            assert!(shift + b.radius <= a.radius + 1e-3, "nested at {now}");
            assert!(
                shift + (a.radius - b.radius) <= SHELL_STEP + 1e-3,
                "step at {now}: {a:?} -> {b:?}"
            );
        }
        if prev.is_some() {
            assert!(shell.is_some(), "walls never vanish ({now})");
        }
        if let Some(next) = w.mode().objective(NEXT_ZONE_ID) {
            let z = Zone {
                center: next.pos,
                radius: next.radius,
            };
            if zones.last() != Some(&z) {
                zones.push(z);
            }
        }
        prev = shell;
    }
    assert_eq!(w.mode().stage, r.phases + 2);
    assert_eq!(prev.unwrap().radius, 0.0, "sealed");
    zones
}

#[test]
fn module_is_implemented_and_free_for_all_only() {
    let m = module(ModeKind::TheSqueeze);
    assert!(m.implemented());
    assert_eq!(m.team_counts(), &[0]);
    let mut r = RuleSet::default_for(ModeKind::TheSqueeze);
    assert!(r.resolve(&WeaponTable::builtin()).is_ok());
    r.mode.set_field("phases", 0.0);
    assert!(r.resolve(&WeaponTable::builtin()).is_err());
}

/// The default rule set is the embedded `data/rulesets/squeeze.ron`; it
/// loads, resolves and survives an `LR` code round trip.
#[test]
fn default_rule_set_loads_and_round_trips_through_a_code() {
    let r = RuleSet::default_for(ModeKind::TheSqueeze);
    assert!(crate::ruleset::BUILTIN_RULESETS
        .iter()
        .any(|(f, _)| *f == ModeKind::TheSqueeze.ruleset_file()));
    assert_eq!(r.name, "The Squeeze");
    assert_eq!(r.mode.kind(), ModeKind::TheSqueeze);
    assert_eq!(r.mode.teams(), 0);
    assert_eq!(r.mode.lives(), 1);
    let s = squeeze(&r);
    assert_eq!(
        (s.first_phase_secs, s.phases, s.warn_secs, s.close_secs),
        (60.0, 5, 20.0, 30.0)
    );
    assert_eq!((s.crush_per_sec, s.scrap_per_100u2), (25.0, 3.0));
    r.resolve(&WeaponTable::builtin()).unwrap();
    let code = r.to_code().unwrap();
    assert!(code.starts_with("LR"));
    let back = RuleSet::from_code(&code).unwrap();
    assert_eq!(back, crate::rulecode::canonical(&r));
    let full = RuleSet::from_code(&r.to_full_code().unwrap()).unwrap();
    assert_eq!(full, back);
}

/// §3.2: phase 1 warns 60 s after the start, each phase warns 20 s and
/// closes over 30 s, then the final close.
#[test]
fn default_timeline() {
    let r = squeeze(&RuleSet::default_for(ModeKind::TheSqueeze));
    assert_eq!(warn_start(&r, 1), 60 * 60);
    assert_eq!(close_window(&r, 1), (80 * 60, 110 * 60));
    assert_eq!(warn_start(&r, 2), 110 * 60);
    assert_eq!(close_window(&r, 5), (280 * 60, 310 * 60));
    assert_eq!(close_window(&r, 6), (310 * 60, 430 * 60));
    assert_eq!(stage_at(&r, 0), 0);
    assert_eq!(stage_at(&r, 3599), 0);
    assert_eq!(stage_at(&r, 3600), 1);
    assert_eq!(stage_at(&r, 110 * 60 - 1), 1);
    assert_eq!(stage_at(&r, 110 * 60), 2);
    assert_eq!(stage_at(&r, 310 * 60), 6);
    assert_eq!(stage_at(&r, 430 * 60), 7);
    // Radii: 60, 45, 30, 18, 8 % of the level radius.
    for (i, f) in [0.60, 0.45, 0.30, 0.18, 0.08].iter().enumerate() {
        assert_eq!(radius_fraction(i as u8 + 1, 5), *f);
    }
    // Other phase counts stretch the curve and still end at 8 %.
    assert_eq!(radius_fraction(3, 3), 0.08);
    assert!(radius_fraction(1, 3) < 0.6 && radius_fraction(1, 3) > 0.45);
    assert!(radius_fraction(1, 10) > 0.6);
}

/// The walls close phase by phase to the announced radii, each zone inside
/// the previous one, the shell moving in steps of at most 0.5 u and
/// sealing at the end; the terrain always has the world's shell.
#[test]
fn phases_close_to_nested_zones_in_bounded_steps() {
    let zones = run_timeline(5);
    let r = squeeze(&fast_rules());
    assert_eq!(zones.len(), r.phases as usize + 1, "{zones:?}");
    let level = level_radius(arena().bounds());
    let mut prev = Zone {
        center: Vec3::ZERO,
        radius: level,
    };
    for (i, z) in zones.iter().enumerate() {
        let phase = i as u8 + 1;
        let want = if phase > r.phases {
            0.0
        } else {
            level * radius_fraction(phase, r.phases)
        };
        assert_eq!(z.radius, want, "phase {phase}");
        assert!(prev.contains(*z), "phase {phase}: {z:?} in {prev:?}");
        assert_eq!(z.center, snap(z.center), "whole units");
        prev = *z;
    }
}

/// Safe-zone placement is deterministic per seed and differs between seeds
/// (8 seeds).
#[test]
fn zone_placement_is_deterministic_across_seeds() {
    let mut all = Vec::new();
    for seed in 1..=8u64 {
        let a = run_timeline(seed);
        let b = run_timeline(seed);
        assert_eq!(a, b, "seed {seed}");
        all.push(a);
    }
    all.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    all.dedup();
    assert!(all.len() >= 4, "seeds place zones differently: {all:?}");
    // Zones prefer the level's open points (spawns, pickups) that fit and
    // shift the zone.
    let s = setup();
    let prev = Zone {
        center: Vec3::ZERO,
        radius: 40.0,
    };
    let mut points: Vec<Vec3> = s.spawns.iter().map(|p| p.pos).collect();
    points.extend(s.pickup_points.iter().copied());
    let anywhere = |_: Vec3| true;
    for seed in 0..16 {
        let z = next_zone(seed, 1, prev, 20.0, &points, &anywhere);
        assert!(points.iter().any(|p| snap(*p) == z.center), "{z:?}");
        assert!(z.center != prev.center);
    }
    // Nothing fits: a random point inside that is open, if one is found.
    let open = |p: Vec3| p.x > -4.0;
    for seed in 0..16 {
        let z = next_zone(seed, 2, prev, 30.0, &[Vec3::splat(100.0)], &open);
        assert!(prev.contains(z) && z.center != prev.center);
        assert!(z.center.x > -4.0, "{z:?}");
    }
    // No leeway: the old centre.
    let z = next_zone(3, 2, prev, 39.5, &points, &anywhere);
    assert_eq!(z.center, prev.center);
}

/// The shell is solid: a ship flying at the wall is held inside it.
#[test]
fn the_shell_is_solid_terrain() {
    let rules = fast_rules();
    let r = squeeze(&rules);
    let mut w = world(9, rules);
    let mut t = arena();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let radius = w.tuning_for(ShipType::Tinker).radius;
    // Walls up and still: the warning of phase 2.
    while w.tick() < warn_start(&r, 2) + 1 {
        w.step(&mut t, &[]);
        w.ship_mut(a).unwrap().pos = Vec3::ZERO;
        w.ship_mut(a).unwrap().vel = Vec3::ZERO;
    }
    let shell = w.terrain_shell().unwrap();
    let dir = wall_dir(&t, shell, shell.radius);
    {
        let s = w.ship_mut(a).unwrap();
        s.pos = shell.center + dir * (shell.radius - 4.0);
        s.hull = 100.0;
        // Nose towards the wall (forward is -Z).
        s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    }
    let push = InputCmd::idle(0).with_thrust(Vec3::NEG_Z);
    let mut touched = false;
    for _ in 0..ticks(r.warn_secs) - 2 {
        w.step(&mut t, &[(a, push)]);
        let s = w.ship(a).unwrap();
        let d = s.pos.distance(shell.center);
        assert!(d + radius <= shell.radius + 0.05, "inside: {d}");
        touched |= d + radius >= shell.radius - 0.2;
        // A still wall does not crush.
        assert_eq!(s.hull, 100.0);
    }
    assert!(touched, "the ship reached the wall");
    assert_eq!(t.sdf(shell.center + dir * (shell.radius + 1.0)), -1.0);
}

/// Crushing: 25 hull per second touching a moving wall, through an active
/// shield; nothing away from the wall.
#[test]
fn moving_walls_crush_through_shields() {
    let rules = fast_rules();
    let r = squeeze(&rules);
    let mut w = world(4, rules);
    let mut t = arena();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    let radius = w.tuning_for(ShipType::Tinker).radius;
    let hull = w.tuning_for(ShipType::Tinker).hull_max;
    let (start, end) = close_window(&r, 3);
    let squeeze_mod = module(ModeKind::TheSqueeze);
    let mut crushed_ticks = 0;
    while w.tick() < end - 1 {
        let next = w.tick() + 1;
        let shell = squeeze_mod.terrain_shell(w.mode(), next);
        for (id, pos) in [(a, Vec3::ZERO), (b, Vec3::ZERO)] {
            let s = w.ship_mut(id).unwrap();
            s.pos = pos;
            s.vel = Vec3::ZERO;
            s.hull = hull;
        }
        if next >= start {
            let shell = shell.unwrap();
            // a sits against the wall with its shield on; b in the middle.
            let dir = wall_dir(&t, shell, shell.radius);
            let s = w.ship_mut(a).unwrap();
            s.pos = shell.center + dir * (shell.radius - radius - 0.02);
            s.gadget_active = true;
            s.energy = 100.0;
            w.ship_mut(b).unwrap().pos = shell.center;
        }
        w.step(&mut t, &[]);
        if next >= start {
            let ha = w.ship(a).unwrap().hull;
            assert!(
                (hull - ha - 25.0 * ls_math::DT).abs() < 1e-3,
                "tick {next}: {ha}"
            );
            assert_eq!(w.ship(b).unwrap().hull, hull);
            crushed_ticks += 1;
        } else {
            assert_eq!(w.ship(a).unwrap().hull, hull, "no walls yet ({next})");
        }
    }
    assert!(crushed_ticks > 60);
    // Nobody died: crushing is gradual.
    assert!(w.ships().all(|s| s.alive));
}

/// A ship deep in the wall is pushed back into the zone by the solid shell
/// (terrains with a shell layer) or, where nothing pushes it (a terrain
/// without one), crushed at once and eliminated; the last ship flying wins.
#[test]
fn swallowed_ships_die_and_the_last_ship_flying_wins() {
    let rules = fast_rules();
    let r = squeeze(&rules);
    // With the shell layer: pushed back inside, alive.
    let mut w = world(2, rules.clone());
    let mut t = arena();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    while w.tick() < warn_start(&r, 3) + 1 {
        w.step(&mut t, &[]);
        w.ship_mut(a).unwrap().pos = Vec3::ZERO;
    }
    let shell = w.terrain_shell().unwrap();
    let dir = wall_dir(&t, shell, shell.radius + 8.0);
    w.ship_mut(a).unwrap().pos = shell.center + dir * (shell.radius + 8.0);
    w.step(&mut t, &[]);
    let s = w.ship(a).unwrap();
    assert!(s.alive && s.pos.distance(shell.center) < shell.radius);

    // Without it (the mode alone): crushed at once.
    let mut w = world(2, rules);
    let mut t = BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0));
    let ids: Vec<PlayerId> = (0..3)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    while w.tick() < warn_start(&r, 3) + 1 {
        w.step(&mut t, &[]);
        for &id in &ids {
            w.ship_mut(id).unwrap().pos = Vec3::ZERO;
        }
    }
    let shell = w.terrain_shell().unwrap();
    w.ship_mut(ids[0]).unwrap().pos = shell.center + dir * (shell.radius + 8.0);
    for &id in &ids[1..] {
        w.ship_mut(id).unwrap().pos = shell.center;
    }
    let ev = w.step(&mut t, &[]);
    assert!(!w.ship(ids[0]).unwrap().alive);
    assert!(ev.contains(&Event::Eliminated { player: ids[0] }));
    assert!(w.mode().is_running());
    // A kill scores for the killer and ends the match.
    let src = DamageSource {
        attacker: Some(ids[1]),
        team: w.ship(ids[1]).unwrap().team,
        weapon: None,
    };
    w.hook_for_test(&mut t, |ctx| {
        ctx.damage(ids[2], 10_000.0, Vec3::ZERO, src, true)
    });
    let ev = w.step(&mut t, &[]);
    let side = w.ship(ids[1]).unwrap().team;
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::MatchEnded {
            winner: Some(s),
            ..
        } if *s == side
    )));
    assert_eq!(w.mode().player(ids[1]).unwrap().score, 1);
    assert_eq!(
        w.mode().player(ids[0]).unwrap().score,
        0,
        "crushing scores nothing"
    );
    let view = w.mode_view();
    assert_eq!(view.winner_player().unwrap().player, ids[1]);
    assert!(view
        .players
        .iter()
        .filter(|p| p.player != ids[1])
        .all(|p| p.flags & player_flags::ELIMINATED != 0));
}

/// The last two ships dying in the same tick is a draw; nobody respawns.
#[test]
fn last_two_dying_together_is_a_draw() {
    let mut w = world(3, fast_rules());
    let mut t = arena();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Hauler).unwrap();
    w.step(&mut t, &[]);
    w.hook_for_test(&mut t, |ctx| {
        for id in [a, b] {
            ctx.adjust_hull(id, -10_000.0, true);
        }
    });
    let ev = w.step(&mut t, &[]);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::MatchEnded { winner: None, .. })));
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: None });
}

/// Idle ships are pushed in and crushed until one (or none) is left: the
/// match ends by the walls before the timer, and at no tick is a living
/// ship inside the shell.
#[test]
fn walls_end_the_match_and_no_ship_survives_inside_the_shell() {
    let rules = fast_rules();
    let r = squeeze(&rules);
    let mut w = world(11, rules);
    let mut t = arena();
    let ids: Vec<PlayerId> = (0..4)
        .map(|_| w.add_player(None, ShipType::ALL[0]).unwrap())
        .collect();
    let mut ended = None;
    while ended.is_none() && w.tick() < sealed_tick(&r) + 10 {
        for e in w.step(&mut t, &[]) {
            if let Event::MatchEnded { winner, .. } = e {
                ended = Some((w.tick(), winner));
            }
        }
        if let Some(shell) = w.terrain_shell() {
            for s in w.ships().filter(|s| s.alive) {
                let radius = w.tuning_for(s.ship_type).radius;
                let d = s.pos.distance(shell.center);
                assert!(
                    d - radius <= shell.radius && radius <= shell.radius,
                    "tick {}: ship {:?} inside the shell",
                    w.tick(),
                    s.player
                );
            }
        }
    }
    let (tick, winner) = ended.expect("the walls end the match");
    assert!(tick <= sealed_tick(&r));
    let alive: Vec<&ShipState> = w.ships().filter(|s| s.alive).collect();
    assert!(alive.len() <= 1);
    match winner {
        Some(side) => assert_eq!(alive[0].team, side),
        None => assert!(alive.is_empty()),
    }
    assert!(ids.iter().all(|id| w.ship(*id).unwrap().deaths <= 1));
}

/// Closing walls shed scrap debris inward from the open parts of the wall
/// (at most `MAX_DEBRIS` per phase), none while the walls stand still.
#[test]
fn closing_walls_shed_scrap_inward() {
    let rules = fast_rules();
    let r = squeeze(&rules);
    let mut w = world(6, rules);
    let mut t = arena();
    let mut seen = std::collections::BTreeSet::new();
    let mut per_stage = std::collections::BTreeMap::<u8, usize>::new();
    while w.tick() < close_window(&r, r.phases).1 {
        w.step(&mut t, &[]);
        let now = w.tick();
        let shell = w.terrain_shell();
        for e in w.entities() {
            if e.kind != EntityKind::Debris || !seen.insert(e.id) {
                continue;
            }
            let shell = shell.expect("debris only once walls move");
            let sched = ShellSchedule::from_state(w.mode()).unwrap();
            assert!(sched.closing(now) || sched.closing(now - 1), "tick {now}");
            assert_eq!(e.scrap, SCRAP_PER_PIECE);
            // Shed inward, from just inside the wall.
            let out = e.pos - shell.center;
            assert!(e.vel.dot(out) < 0.0);
            assert!((out.length() - (shell.radius - 1.0)).abs() < 0.6);
            *per_stage.entry(w.mode().stage).or_default() += 1;
        }
    }
    assert!(!per_stage.is_empty(), "no scrap");
    assert!(per_stage.values().all(|&n| n <= MAX_DEBRIS));
}

// ----- replay ----------------------------------------------------------------

/// A scripted 60 s Squeeze match (4 ships, every phase closes within it):
/// the state hash every second.
fn replay(seed: u64) -> (Vec<u64>, Vec<Event>) {
    let mut rules = RuleSet::default_for(ModeKind::TheSqueeze);
    rules.mode.set_field("first_phase_secs", 5.0);
    rules.mode.set_field("warn_secs", 3.0);
    rules.mode.set_field("close_secs", 6.0);
    let mut w = world(seed, rules);
    let mut t = arena();
    let ids: Vec<PlayerId> = ShipType::ALL
        .iter()
        .map(|&ty| w.add_player(None, ty).unwrap())
        .collect();
    let mut rng = Pcg32::new(seed, 11);
    let mut hashes = Vec::new();
    let mut events = Vec::new();
    for tick in 1..=3600u64 {
        let inputs: Vec<(PlayerId, InputCmd)> = ids
            .iter()
            .map(|&id| {
                let axis = |r: &mut Pcg32| (r.below(3) as f32) - 1.0;
                let mut buttons = Buttons::HORIZON_ASSIST;
                buttons.set(Buttons::FIRE_PRIMARY, rng.chance(0.3));
                buttons.set(Buttons::FIRE_SECONDARY, rng.chance(0.03));
                buttons.set(Buttons::BOOST, rng.chance(0.2));
                buttons.set(Buttons::SCAVENGE, rng.chance(0.2));
                let thrust = Vec3::new(axis(&mut rng), axis(&mut rng), axis(&mut rng));
                (
                    id,
                    InputCmd {
                        tick,
                        thrust,
                        look_pitch: rng.range_f32(-0.05, 0.05),
                        look_yaw: rng.range_f32(-0.05, 0.05),
                        roll: 0.0,
                        buttons,
                        weapon_select: 0,
                    },
                )
            })
            .collect();
        events.extend(w.step(&mut t, &inputs));
        if tick % 60 == 0 {
            hashes.push(w.state_hash());
        }
    }
    (hashes, events)
}

/// Golden hash of the scripted Squeeze replay; identical on native and
/// wasm32. Changing The Squeeze's behaviour changes it: update it in the
/// same change.
const SQUEEZE_REPLAY_GOLDEN_HASH: u64 = 0xe621_da2d_5ace_169b;

#[test]
fn replay_is_deterministic_and_matches_its_golden_hash() {
    let (h1, e1) = replay(77);
    let (h2, e2) = replay(77);
    assert_eq!(h1, h2);
    assert_eq!(e1, e2);
    assert!(e1.iter().any(|e| matches!(e, Event::Fired { .. })));
    assert!(e1.iter().any(|e| matches!(e, Event::Eliminated { .. })));
    let last = *h1.last().unwrap();
    assert_eq!(last, SQUEEZE_REPLAY_GOLDEN_HASH, "replay hash {last:#018x}");
}
