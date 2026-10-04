//! Tectonic Charge collapse tick cost on a real generated level (M3
//! carried-over item): the crack craters are decided in the collapse tick
//! and carved over the next 6 ticks (16 a tick at Instability 100), so each
//! of those ticks stays under 2 ms. The collapse tick keeps the one exact
//! 35 u collapse crater, whose sample bake in `ls_worldgen::ChunkStore`
//! (`apply_geometry` / `bake_edit` over the crater's whole reach box) alone
//! costs about 3-4 ms; that tick is held to a looser bound here until the
//! bake gets cheaper (see `docs/milestone-3-plan.md`, A1 notes).
//!
//! Release only (`cargo test -p ls-game --release --test tectonic_tick`);
//! debug builds are far slower than the budget.

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Quat, Vec3};
use ls_game::ls_sim::{Buttons, Event, InputCmd, ShipType};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::{build_match, MatchConfig};

/// Per-tick budget for the crack ticks.
const BUDGET: Duration = Duration::from_millis(2);
/// Bound for the collapse tick (the 35 u crater bake; M2 measured 2-7 ms
/// with the cracks in the same tick).
const COLLAPSE_BOUND: Duration = Duration::from_millis(8);

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test: run with --release")]
fn crack_ticks_stay_under_2_ms() {
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
    println!("collapse tick and the 6 after: {times:?}, edits {edits_per_tick:?}");
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
        times[0] < COLLAPSE_BOUND,
        "collapse tick {:?} (bound {COLLAPSE_BOUND:?})",
        times[0]
    );
}
