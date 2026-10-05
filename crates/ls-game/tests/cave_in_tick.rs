//! Cost of an Instability cave-in tick on a real generated level (M4
//! package A4): Bolt Rockets hammer one spot of a tunnel floor at
//! Instability 100 until the rock comes down, with five more ships parked
//! around the spot so the "never enclose a ship" flood fill runs for each.
//! The cave-in tick (rubble edits, hollow, debris, the escape checks) must
//! stay within the 4 ms room tick budget; the warning ticks are ordinary.
//!
//! Release only (`cargo test -p ls-game --release --test cave_in_tick`).

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use ls_game::ls_math::terrain::{TerrainEdit, TerrainQuery};
use ls_game::ls_math::{Pcg32, Quat, Vec3};
use ls_game::ls_sim::{Buttons, CueKind, Event, InputCmd, ShipType};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::{build_match, MatchConfig};

/// The cave-in tick's bound (the room tick budget).
const BOUND: Duration = Duration::from_millis(4);

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test: run with --release")]
fn a_cave_in_tick_stays_within_the_tick_budget() {
    let mut sliders = Sliders::new(70, 40);
    sliders.instability = 100;
    let code = ShareCode::new(0x0cae_1e50, sliders);
    let mut cfg = MatchConfig::paint_the_void(code);
    cfg.rules.pickups.clear();
    let (mut world, mut terrain) = build_match(&cfg).unwrap();
    let me = world.add_player(Some(0), ShipType::Tinker).unwrap();
    let mut others = Vec::new();
    for i in 0..5 {
        others.push(
            world
                .add_player(Some((i % 2) as u8), ShipType::ALL[i % 4])
                .unwrap(),
        );
    }
    world.give_weapon(me, "bolt_rocket").unwrap();
    let rocket = world.weapons().find("bolt_rocket").unwrap();
    // A tunnel spot far from every spawn: open, a floor 4-9 u below and a
    // ceiling within 12 u above.
    let spawn_pos: Vec<Vec3> = world.ships().map(|s| s.pos).collect();
    let b = terrain.bounds();
    let mut rng = Pcg32::new(5, 5);
    let mut spot = None;
    for _ in 0..200_000 {
        let p = Vec3::new(
            rng.range_f32(b.min.x + 10.0, b.max.x - 10.0),
            rng.range_f32(b.min.y + 10.0, b.max.y - 10.0),
            rng.range_f32(b.min.z + 10.0, b.max.z - 10.0),
        );
        if terrain.sdf(p) < 2.5 || spawn_pos.iter().any(|s| s.distance(p) < 40.0) {
            continue;
        }
        let (Some(floor), Some(ceil)) = (
            terrain.raycast(p, Vec3::NEG_Y, 9.0),
            terrain.raycast(p, Vec3::Y, 12.0),
        ) else {
            continue;
        };
        if floor.distance >= 4.0 {
            spot = Some((p, floor.point, ceil.point));
            break;
        }
    }
    let (pos, floor, ceiling) = spot.expect("a tunnel spot");
    println!("ship at {pos:?}, floor {floor:?}, ceiling {ceiling:?}");
    {
        let s = world.ship_mut(me).unwrap();
        s.secondary = Some(rocket);
        s.pos = pos;
        s.vel = Vec3::ZERO;
        s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y);
    }
    // Five ships parked around the spot, in the open.
    for (k, id) in others.iter().enumerate() {
        let a = k as f32 * 1.25;
        let mut p = pos + Vec3::new(4.0 * a.cos(), 0.0, 4.0 * a.sin());
        for _ in 0..20 {
            if terrain.sdf(p) > 1.0 {
                break;
            }
            p = p.lerp(pos, 0.3);
        }
        let s = world.ship_mut(*id).unwrap();
        s.pos = p;
        s.vel = Vec3::ZERO;
    }
    let parked: Vec<(_, Vec3)> = world.ships().map(|s| (s.player, s.pos)).collect();
    let mut warned = None;
    let mut cave_in_time = None;
    let mut slowest_other = Duration::ZERO;
    for tick in 0..60 * 60u64 {
        if let Some(st) = world
            .ship_mut(me)
            .unwrap()
            .weapons
            .iter_mut()
            .find(|w| w.weapon == rocket)
        {
            st.ammo = 10;
        }
        let buttons = if warned.is_none() {
            Buttons::FIRE_SECONDARY
        } else {
            Buttons::default()
        };
        let input = InputCmd::idle(tick).with_buttons(buttons);
        let t0 = Instant::now();
        let ev = world.step(&mut terrain, &[(me, input)]);
        let dt = t0.elapsed();
        for (id, p) in &parked {
            if let Some(s) = world.ship_mut(*id) {
                s.pos = *p;
                s.vel = Vec3::ZERO;
            }
        }
        let rubble = ev
            .iter()
            .filter(|e| matches!(e, Event::TerrainEdit(TerrainEdit::AddSphere { .. })))
            .count();
        if warned.is_none() {
            if ev
                .iter()
                .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::CaveInWarning))
            {
                warned = Some(tick);
            }
        } else if warned.is_some_and(|w| tick == w + 90) {
            println!("cave-in tick {dt:?}, {rubble} rubble spheres");
            cave_in_time = Some((dt, rubble));
            break;
        } else {
            slowest_other = slowest_other.max(dt);
        }
    }
    let warned = warned.expect("the rock came down");
    let (t, rubble) = cave_in_time.expect("cave-in tick");
    assert!(rubble > 0, "rubble came down");
    println!("warning at tick {warned}; slowest warning tick {slowest_other:?}");
    assert!(t < BOUND, "cave-in tick {t:?} (bound {BOUND:?})");
}
