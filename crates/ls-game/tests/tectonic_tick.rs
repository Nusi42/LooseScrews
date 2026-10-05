//! Tectonic Charge collapse tick cost on a real generated level (carried
//! over from M2 via M3, closed in M4 A2): the crack craters are decided in
//! the collapse tick and carved over the next 6 ticks (16 a tick at
//! Instability 100), so each of those ticks stays under 2 ms. The collapse
//! tick keeps the one exact 35 u collapse crater; its sample bake in
//! `ls_worldgen::ChunkStore` cost 3.5-5.3 ms while it evaluated every corner
//! of the crater's reach box, and now only evaluates the corners within the
//! reach, with a vectorized row kernel (`bake_edit`): the whole collapse tick
//! stays under 2 ms as well (measured 1.4-1.7 ms on the shared machine, best
//! of three runs; 2.3-2.6 ms while several other builds saturate it).
//!
//! Release only (`cargo test -p ls-game --release --test tectonic_tick`);
//! debug builds are far slower than the budget. Each scenario runs three
//! times on a fresh match and the best time per tick counts, so a loaded
//! machine does not fail it.

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Quat, Vec3};
use ls_game::ls_sim::{Buttons, Event, InputCmd, ShipType};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::{build_match, MatchConfig};

/// Per-tick budget for the collapse tick and the crack ticks.
const BUDGET: Duration = Duration::from_millis(2);

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test: run with --release")]
fn collapse_and_crack_ticks_stay_under_2_ms() {
    let runs: Vec<(Vec<Duration>, Vec<usize>)> = (0..3).map(|_| collapse_run()).collect();
    let edits_per_tick = runs[0].1.clone();
    assert!(
        runs.iter().all(|r| r.1 == edits_per_tick),
        "same edits every run"
    );
    let times: Vec<Duration> = (0..edits_per_tick.len())
        .map(|t| runs.iter().map(|r| r.0[t]).min().unwrap())
        .collect();
    println!("collapse tick and the 6 after (best of 3): {times:?}, edits {edits_per_tick:?}");
    // Cracks come after the collapse tick, a share per tick.
    assert!(
        edits_per_tick[1..].iter().sum::<usize>() > 0,
        "cracks at Instability 100"
    );
    assert_eq!(edits_per_tick[0], 1, "only the collapse crater in its tick");
    assert!(edits_per_tick[1..].iter().all(|&n| n > 0 && n <= 16));
    let worst = *times[1..].iter().max().unwrap();
    assert!(
        worst < BUDGET,
        "slowest crack tick {worst:?} (budget {BUDGET:?})"
    );
    assert!(
        times[0] < BUDGET,
        "collapse tick {:?} (budget {BUDGET:?})",
        times[0]
    );
}

/// Fires the charge on a fresh match; the step times and terrain edits of
/// the collapse tick and the 6 after.
fn collapse_run() -> (Vec<Duration>, Vec<usize>) {
    let mut sliders = Sliders::new(70, 40);
    sliders.instability = 100;
    let code = ShareCode::new(0x7ec7_0a1c, sliders);
    let (mut world, mut terrain) = build_match(&MatchConfig::paint_the_void(code)).unwrap();
    let me = world.add_player(Some(0), ShipType::Tinker).unwrap();
    world.give_weapon(me, "tectonic_charge").unwrap();
    let tectonic = world.weapons().find("tectonic_charge").unwrap();
    // Aim at the rock 12 u in front of the ship (straight down, else the
    // nearest wall in one of six directions).
    let pos = world.ship(me).unwrap().pos;
    let dirs = [
        Vec3::NEG_Y,
        Vec3::Y,
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    let (dir, hit) = dirs
        .iter()
        .find_map(|&d| terrain.raycast(pos, d, 200.0).map(|h| (d, h)))
        .expect("rock in some direction");
    {
        let s = world.ship_mut(me).unwrap();
        s.secondary = Some(tectonic);
        s.pos = hit.point - dir * 12.0;
        s.vel = Vec3::ZERO;
        s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    }

    let mut collapse = None;
    let mut times = Vec::new();
    let mut edits_per_tick = Vec::new();
    for tick in 0..600u64 {
        let buttons = if tick == 0 {
            Buttons::FIRE_SECONDARY
        } else {
            Buttons::default()
        };
        let input = InputCmd::idle(tick).with_buttons(buttons);
        let t0 = Instant::now();
        let ev = world.step(&mut terrain, &[(me, input)]);
        let dt = t0.elapsed();
        let blast = ev.iter().any(
            |e| matches!(e, Event::Impact { weapon, target: None, .. } if *weapon == tectonic),
        );
        if collapse.is_none() && blast {
            collapse = Some(tick);
        }
        if let Some(c) = collapse {
            let edits = ev
                .iter()
                .filter(|e| matches!(e, Event::TerrainEdit(_)))
                .count();
            times.push(dt);
            edits_per_tick.push(edits);
            if tick >= c + 6 {
                break;
            }
        }
    }
    assert!(collapse.is_some(), "the charge went off");
    (times, edits_per_tick)
}
