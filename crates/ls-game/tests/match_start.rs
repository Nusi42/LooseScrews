//! Regression: the match-start stall (M4 wave B finding, fixed by C6).
//!
//! Tick 30 of a fresh `build_match` world took 0.5-1.2 s in release: Paint
//! the Void's first coverage query (`SimTerrain::paint_coverage`) generated
//! every chunk of a lazily built level inside that tick. `build_match` now
//! generates the level while building (`build_match_lazy` keeps the old
//! behaviour for tests that stay below tick 30), and `Room::new` generates
//! it unless `RoomConfig::pregenerate` is off. This checks that no tick of a
//! match's first 10 s is slow, for every built-in rule set, on a world from
//! `build_match` and in a `Room` with the default config.
//!
//! Release only (debug builds generate a level in about 7 s):
//! `cargo test -p ls-game --release --test match_start`.

use std::time::{Duration, Instant};

use ls_game::ls_sim::ruleset::BUILTIN_RULESETS;
use ls_game::ls_sim::{RuleSet, ShipType};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::{build_match, build_match_lazy, client_hello, MatchConfig};

/// Generous for a shared machine (a tick is budgeted 4 ms; the stall was
/// 500 ms and more).
const SLOW_TICK: Duration = Duration::from_millis(50);
/// Ticks stepped: past the first coverage query (tick 30) and the first
/// score updates.
const TICKS: u64 = 600;

fn config(file: &str, text: &str) -> MatchConfig {
    let rules = RuleSet::from_ron(file, text).unwrap();
    let code = ShareCode::new(0x5ee_d000, Sliders::new(40, 30));
    MatchConfig {
        rules,
        ..MatchConfig::paint_the_void(code)
    }
}

/// The slowest of `ticks` steps of `step` and its tick number (1-based).
fn slowest(ticks: u64, mut step: impl FnMut()) -> (u64, Duration) {
    let mut worst = (0, Duration::ZERO);
    for t in 1..=ticks {
        let t0 = Instant::now();
        step();
        let dt = t0.elapsed();
        if dt > worst.1 {
            worst = (t, dt);
        }
    }
    worst
}

#[test]
#[cfg_attr(debug_assertions, ignore = "generates whole levels: release only")]
fn no_tick_of_a_fresh_match_stalls() {
    for (file, text) in BUILTIN_RULESETS {
        let (mut world, mut terrain) = build_match(&config(file, text)).unwrap();
        for _ in 0..4 {
            world.add_player(None, ShipType::Tinker).unwrap();
        }
        let (tick, dt) = slowest(TICKS, || {
            world.step(&mut terrain, &[]);
        });
        println!("{file}: build_match world, slowest tick {tick}: {dt:?}");
        assert!(
            dt < SLOW_TICK,
            "{file}: tick {tick} of a fresh build_match world took {dt:?}"
        );

        let mut room = Room::new(config(file, text), RoomConfig::default()).unwrap();
        for i in 0..4 {
            room.join(&client_hello(format!("p{i}"), None, None))
                .unwrap();
        }
        let (tick, dt) = slowest(TICKS, || {
            room.step();
            let _ = room.take_outgoing();
        });
        println!("{file}: room, slowest tick {tick}: {dt:?}");
        assert!(
            dt < SLOW_TICK,
            "{file}: tick {tick} of a fresh room took {dt:?}"
        );
    }
}

/// The lazy build still exists (tests below tick 30) and is what stalled:
/// its first coverage query generates the level inside tick 30. Measured
/// here so the regression test above is known to cover the right tick.
#[test]
#[cfg_attr(debug_assertions, ignore = "generates whole levels: release only")]
fn the_lazy_build_generates_the_level_at_tick_30() {
    let (file, text) = BUILTIN_RULESETS[0];
    assert_eq!(file, "rulesets/paint_the_void.ron");
    let (mut world, mut terrain) = build_match_lazy(&config(file, text)).unwrap();
    world.add_player(Some(0), ShipType::Tinker).unwrap();
    let (tick, dt) = slowest(60, || {
        world.step(&mut terrain, &[]);
    });
    println!("lazy build: slowest tick {tick}: {dt:?}");
    assert_eq!(tick, 30, "the coverage query is the slow tick ({dt:?})");
    assert!(
        dt > SLOW_TICK,
        "the lazy level generates at tick 30 ({dt:?})"
    );
}
