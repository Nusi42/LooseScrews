//! The process-wide navigation grid cache on a real level
//! (`docs/milestone-3-plan.md`, package A5):
//!
//! - the bots of one match share one grid build (counter);
//! - navigation CPU per bot for 8 bots on one world is at least 5x lower
//!   than a grid build per bot (the M2 cost).
//!
//! Needs whole-level generation and full grid builds, so it runs only in
//! release: `cargo test -p ls-bots --release --test nav_cache`.

use std::time::{Duration, Instant};

use ls_bots::nav::{NavConfig, NavGrid, Navigator};
use ls_bots::nav_cache;
use ls_bots::{bot_hello, BotCore};
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_net::{check_welcome, Message};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::{GameTerrain, MatchConfig};

const BOTS: usize = 8;

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "whole-level grid builds are slow in debug builds; run with --release"
)]
fn bots_of_one_world_share_one_grid_build() {
    // --- Bots of one match: one build. ---------------------------------
    let code = ShareCode::new(0x00a5_cace, Sliders::new(40, 30));
    let mut room = Room::new(MatchConfig::paint_the_void(code), RoomConfig::default()).unwrap();
    let hello = Message::Hello(bot_hello("bot1", 1)).encode().unwrap();
    let id = room.join_packet(&hello).expect("join accepted");
    room.step();
    let welcome = room
        .take_outgoing()
        .into_iter()
        .find(|o| o.client == id)
        .map(|o| check_welcome(Message::decode(&o.bytes).unwrap()).expect("welcome"))
        .expect("a welcome");
    let before = nav_cache::builds();
    let cores: Vec<BotCore> = (0..BOTS)
        .map(|i| BotCore::from_welcome(&welcome, 700 + i as u64).unwrap())
        .collect();
    assert_eq!(
        nav_cache::builds() - before,
        1,
        "{BOTS} bots of one match must start exactly one grid build"
    );
    // No bot builds a grid of its own: each waits for (or has adopted) the
    // shared one.
    assert!(cores
        .iter()
        .all(|c| c.brain().navigator().is_waiting() || c.brain().navigator().is_ready()));

    // --- CPU per bot: a build per bot (M2) vs. the shared build. -------
    // Wall time on a shared machine is noisy: a world counts if its ratio
    // reaches 5x; up to three worlds are tried.
    let mut seen = Vec::new();
    for (k, seed) in [0x00a5_0c90u64, 0x00a5_0c91, 0x00a5_0c92]
        .into_iter()
        .enumerate()
    {
        let world = ShareCode::new(seed, Sliders::new(50, 50 - 10 * k as u8));
        let (before, after) = measure(&world);
        let ratio = before.as_secs_f64() / after.as_secs_f64();
        println!(
            "nav CPU per bot, {BOTS} bots on one world: before {before:?}, after {after:?} ({ratio:.1}x lower)"
        );
        seen.push(ratio);
        if ratio >= 5.0 {
            return;
        }
    }
    panic!("navigation CPU per bot not 5x lower: ratios {seen:?}");
}

/// Navigation time per bot for `BOTS` bots on `world`: a grid build per
/// bot (M2) and the shared build plus each bot's copy (M3).
fn measure(world: &ShareCode) -> (Duration, Duration) {
    let cfg = NavConfig::default();

    // M2: every bot built its own grid; one build is the cost per bot.
    let terrain = GameTerrain::from_code(world).unwrap();
    let t = Instant::now();
    let mut own = NavGrid::new(terrain.bounds(), cfg);
    own.build_all(&terrain);
    let per_bot_before = t.elapsed();

    // Shared: the first bot starts the build, all of them adopt a copy.
    let pristine: Vec<GameTerrain> = (0..BOTS)
        .map(|_| GameTerrain::from_code(world).unwrap())
        .collect();
    let bot_terrain = GameTerrain::from_code(world).unwrap();
    let start = nav_cache::builds();
    let t = Instant::now();
    let mut navs: Vec<Navigator> = pristine
        .into_iter()
        .map(|p| {
            let shared = nav_cache::for_level(world, cfg, p);
            Navigator::shared(bot_terrain.bounds(), cfg, shared)
        })
        .collect();
    for nav in &mut navs {
        assert!(nav.wait_shared(Duration::from_secs(120)));
        nav.update(&bot_terrain);
        assert!(nav.is_ready());
    }
    let per_bot_after = t.elapsed() / BOTS as u32;
    assert_eq!(nav_cache::builds() - start, 1);
    assert_eq!(navs[0].grid().node_count(), own.node_count());
    (per_bot_before, per_bot_after)
}
