//! Regression test for M1 issue A (`docs/milestone-2-plan.md`, wave A1):
//! on some random worlds bots never met, scored no hits and only painted,
//! because their steering was purely local and could not find a way to
//! enemies on the far side of a cave level.
//!
//! Three bots (two teams, like the soak that found it) play 60 s on the soak
//! world `LS08-PNEY-V86Z-VXZM-K4M6-8001-8F8P` and on nine more random worlds
//! (default sliders, like the server's random worlds) against an in-process
//! `Room`; every world must see hits. A 5-minute match on the soak world
//! must still see hits in its last minute (navigation must not wear out).
//! Deterministic (no sockets, no wall
//! clock). Release only, like `room_play.rs`: `cargo test -p ls-bots --release`.
//! `LS_NAV_TRACE=1` prints every bot's position, mode and route every 2 s.

use std::collections::VecDeque;

use ls_bots::{BotCore, BotStats, Flow};
use ls_game::ls_net::Message;
use ls_game::ls_sim::ModeRules;
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

/// The soak world where 3 bots scored 0 hits in 12 matches (M1 code
/// `LS04-PNEY-V86Z-VXZM-K4M6-8001-8F8T`), re-encoded for generator version 2:
/// same seed and sliders, but the level now also has the tunnel network.
const SOAK_WORLD: &str = "LS08-PNEY-V86Z-VXZM-K4M6-8001-8F8P";
const BOTS: usize = 3;
const SECS: u64 = 60;

struct Link {
    delay: u64,
    queue: VecDeque<(u64, Vec<u8>)>,
}

impl Link {
    fn new(delay: u64) -> Self {
        Link {
            delay,
            queue: VecDeque::new(),
        }
    }
    fn push(&mut self, now: u64, bytes: Vec<u8>) {
        self.queue.push_back((now + self.delay, bytes));
    }
    fn pop_due(&mut self, now: u64) -> Option<Vec<u8>> {
        if self.queue.front().is_some_and(|(due, _)| *due <= now) {
            self.queue.pop_front().map(|(_, b)| b)
        } else {
            None
        }
    }
}

struct Client {
    id: Option<u8>,
    core: Option<BotCore>,
    stats: BotStats,
    up: Link,
    down: Link,
}

/// Plays `secs` seconds with `BOTS` bots on `code`; returns the bot stats
/// and the hits scored in the last 60 s.
fn play(code: ShareCode, secs: u64) -> (Vec<BotStats>, u32) {
    let mut mc = MatchConfig::paint_the_void(code);
    let ModeRules::PaintTheVoid(rules) = &mut mc.rules.mode else {
        unreachable!("a Paint the Void rule set")
    };
    // Longer than the run: the match must not end (and restart) inside it.
    rules.match_secs = rules.match_secs.max(secs as f32 + 30.0);
    let mut room = Room::new(mc, RoomConfig::default()).unwrap();
    let mut clients: Vec<Client> = Vec::new();
    let trace = std::env::var_os("LS_NAV_TRACE").is_some();
    let mut hits_before_last_minute = 0;
    for now in 0..60 * secs {
        if now == 60 * secs.saturating_sub(60) {
            hits_before_last_minute = clients.iter().map(|c| c.stats.hits_dealt).sum();
        }
        if now % 30 == 0 && clients.len() < BOTS {
            let i = clients.len();
            let delay = 2 + (i as u64 % 3);
            let c = Client {
                id: None,
                core: None,
                stats: BotStats::new(format!("bot{}", i + 1)),
                up: Link::new(delay),
                down: Link::new(delay),
            };
            let hello = Message::Hello(ls_bots::bot_hello(c.stats.name.clone(), 300 + i as u64));
            let mut c = c;
            c.up.push(now, hello.encode().unwrap());
            clients.push(c);
        }
        for c in &mut clients {
            while let Some(bytes) = c.up.pop_due(now) {
                match c.id {
                    None => c.id = Some(room.join_packet(&bytes).expect("join accepted")),
                    Some(id) => room.handle_packet(id, &bytes).unwrap(),
                }
            }
        }
        room.step();
        for o in room.take_outgoing() {
            let c = clients.iter_mut().find(|c| c.id == Some(o.client)).unwrap();
            c.down.push(now, o.bytes);
        }
        for (i, c) in clients.iter_mut().enumerate() {
            while let Some(bytes) = c.down.pop_due(now) {
                match &mut c.core {
                    None => {
                        let w = ls_game::ls_net::check_welcome(Message::decode(&bytes).unwrap())
                            .expect("welcome");
                        let core = BotCore::from_welcome(&w, 300 + i as u64).unwrap();
                        // The loop runs faster than real time: wait for the
                        // shared nav grid as a real-time bot would have.
                        assert!(core.wait_for_nav(std::time::Duration::from_secs(120)));
                        c.core = Some(core);
                        c.stats.joins += 1;
                    }
                    Some(core) => match core.handle_packet(&bytes, &mut c.stats) {
                        Ok(Flow::Continue) => {}
                        other => panic!("{}: {other:?} {:?}", c.stats.name, c.stats.errors),
                    },
                }
            }
            if let Some(core) = &mut c.core {
                for m in core.tick(&mut c.stats) {
                    c.up.push(now, m.encode().unwrap());
                }
            }
        }
        if trace && now % 120 == 0 {
            trace_bots(now, &clients);
        }
    }
    let stats: Vec<BotStats> = clients
        .into_iter()
        .map(|c| {
            let mut s = c.stats;
            if let Some(core) = &c.core {
                core.finish(&mut s);
            }
            s
        })
        .collect();
    let hits: u32 = stats.iter().map(|s| s.hits_dealt).sum();
    (stats, hits - hits_before_last_minute)
}

/// Diagnostics (`LS_NAV_TRACE=1`): each bot's position, mode, hunt target
/// and route, and the distance to the nearest enemy.
fn trace_bots(now: u64, clients: &[Client]) {
    for c in clients {
        let Some(core) = &c.core else { continue };
        let s = core.session();
        let Some(me) = s.local_ship() else { continue };
        let b = core.brain();
        let nearest = s
            .remote_ships()
            .iter()
            .filter(|n| s.side_of(n.id) != Some(s.team()))
            .map(|n| n.pos.distance(me.pos))
            .fold(f32::MAX, f32::min);
        eprintln!(
            "t{:>5} {} pos ({:>5.1} {:>5.1} {:>5.1}) alive {} mode {:?} hunt {:?} enemy {:>6.1} route {:?} nav {:?}",
            now / 60,
            c.stats.name,
            me.pos.x,
            me.pos.y,
            me.pos.z,
            me.alive,
            b.mode(),
            b.hunt_target(),
            nearest,
            b.navigator().route_info(),
            b.navigator().grid(),
        );
    }
}

fn check(code: ShareCode) {
    check_for(code, SECS);
}

/// Plays `secs` seconds; hits must happen, and in the last minute too.
fn check_for(code: ShareCode, secs: u64) {
    let text = code.encode().unwrap();
    let (stats, last_minute) = play(code, secs);
    let hits: u32 = stats.iter().map(|s| s.hits_dealt).sum();
    let kills: u32 = stats.iter().map(|s| s.kills).sum();
    let combat: u64 = stats.iter().map(|s| s.behavior.combat_ticks).sum();
    let hunt: u64 = stats.iter().map(|s| s.behavior.hunt_ticks).sum();
    let paint: u64 = stats.iter().map(|s| s.behavior.paint_triggers).sum();
    eprintln!(
        "{text}: hits {hits}, kills {kills}, combat ticks {combat}, hunt ticks {hunt}, \
         paint triggers {paint}"
    );
    for s in &stats {
        assert_eq!(s.protocol_errors, 0, "{text} {}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{text} {}: {:?}", s.name, s.errors);
    }
    assert!(hits > 0, "{text}: bots never hit each other: {stats:#?}");
    assert!(
        last_minute > 0,
        "{text}: no hits in the last minute of {secs} s: {stats:#?}"
    );
    // Painting still happens.
    assert!(paint > 0, "{text}: bots never painted");
}

fn random_world(seed: u64) -> ShareCode {
    ShareCode::new(seed, Sliders::default())
}

/// A long match on the soak world: navigation must not wear out over time
/// (links blocked after stalls used to pile up and cut the level in two).
/// About 20 s in release.
#[test]
#[cfg_attr(debug_assertions, ignore = "5 min match; run with --release")]
fn soak_world_long_match() {
    check_for(ShareCode::decode(SOAK_WORLD).unwrap(), 300);
}

macro_rules! world_tests {
    ($($name:ident => $code:expr;)*) => {$(
        #[test]
        #[cfg_attr(
            debug_assertions,
            ignore = "60 s matches with whole-level generation; run with --release"
        )]
        fn $name() {
            check($code);
        }
    )*};
}

world_tests! {
    soak_world => ShareCode::decode(SOAK_WORLD).unwrap();
    random_world_1 => random_world(0x5eed_0001_d00d_0001);
    random_world_2 => random_world(0x0123_4567_89ab_cdef);
    random_world_3 => random_world(0x9e37_79b9_7f4a_7c15);
    random_world_4 => random_world(0xdead_beef_cafe_f00d);
    random_world_5 => random_world(0x0000_0000_0000_2a2a);
    random_world_6 => random_world(0x7777_1234_4321_7777);
    random_world_7 => random_world(0x00c0_1e47_0000_0007);
    random_world_8 => random_world(0x1357_9bdf_2468_ace0);
    random_world_9 => random_world(0xfeed_face_0bad_c0de);
}
