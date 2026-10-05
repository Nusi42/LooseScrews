//! Team Brawl's default rule sets played to a result by bots in-process
//! (M3 package B1): an authoritative `ls_game` `Room` and a room of
//! [`BotCore`]s exchange encoded packets with a fixed latency, stepped tick
//! by tick (no sockets), as in `room_play.rs`. Team Brawl and Instagib
//! Brawl run with 8 bots, the duel rule set with 2. Matches are shortened
//! to 60 s so each ends inside the test (at the kill target or on time).
//!
//! Needs whole-level generation, so it runs only in release:
//! `cargo test -p ls-bots --release --test team_brawl_match`.

use std::collections::VecDeque;

use ls_bots::{BotCore, BotStats, Flow};
use ls_game::ls_net::Message;
use ls_game::ls_sim::ruleset::BUILTIN_RULESETS;
use ls_game::ls_sim::{MatchPhase, ModeKind, RuleSet};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

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

fn builtin(file: &str) -> RuleSet {
    let (name, text) = BUILTIN_RULESETS.iter().find(|(f, _)| *f == file).unwrap();
    RuleSet::from_ron(name, text).unwrap()
}

/// Plays `file` with `bots` bots for at most 60 s (+3 s after the end) and
/// checks the match reached a result without desyncs or protocol errors.
fn play(file: &str, bots: usize, seed: u64) {
    const MATCH_SECS: u64 = 60;
    let code = ShareCode::new(seed, Sliders::new(40, 30));
    let mut rules = builtin(file);
    assert_eq!(rules.mode.kind(), ModeKind::TeamBrawl);
    rules.mode.set_field("match_secs", MATCH_SECS as f32);
    let mc = MatchConfig {
        rules,
        ..MatchConfig::paint_the_void(code)
    };
    let mut room = Room::new(mc, RoomConfig::default()).unwrap();
    let mut clients: Vec<Client> = Vec::new();
    let mut ended_at = None;
    let mut now = 0u64;
    while ended_at.is_none_or(|t| now < t + 180) && now < 60 * (MATCH_SECS + 10) {
        // Staggered joins: one bot every 10 ticks, latencies 2-5 ticks.
        if now.is_multiple_of(10) && clients.len() < bots {
            let i = clients.len();
            let delay = 2 + (i as u64 % 4);
            let mut c = Client {
                id: None,
                core: None,
                stats: BotStats::new(format!("bot{}", i + 1)),
                up: Link::new(delay),
                down: Link::new(delay),
            };
            let hello = Message::Hello(ls_bots::bot_hello(c.stats.name.clone(), 200 + i as u64));
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
                        let core = BotCore::from_welcome(&w, 200 + i as u64).unwrap();
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
        if ended_at.is_none() && !room.world().mode().is_running() {
            ended_at = Some(now);
        }
        now += 1;
    }
    let mode = room.world().mode();
    assert!(
        matches!(mode.phase, MatchPhase::Ended { .. }),
        "{file}: no result"
    );
    let results = room.world().results();
    let kills: u32 = results.rows.iter().map(|r| r.kills).sum();
    eprintln!(
        "{file}: ended at tick {} (target {}), winner {:?}, team scores {:?}, {kills} kills",
        room.world().tick(),
        mode.score_target,
        results.winner,
        results.team_scores
    );
    assert_eq!(results.rows.len(), bots);
    assert!(kills > 0, "{file}: the bots fought");
    assert!(room.results().is_some(), "{file}: results sent");
    for c in &mut clients {
        let core = c.core.as_ref().expect("joined");
        core.finish(&mut c.stats);
        let s = &c.stats;
        assert_eq!(s.joins, 1, "{}", s.name);
        assert_eq!(s.protocol_errors, 0, "{file} {}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{file} {}: {:?}", s.name, s.errors);
        assert_eq!(s.matches_over, 1, "{file} {}: saw the match end", s.name);
    }
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "full level generation is slow in debug builds; run with --release"
)]
fn team_brawl_rule_sets_play_to_a_result_with_bots() {
    play("rulesets/team_brawl.ron", 8, 0x00b1_7b01);
    play("rulesets/instagib_brawl.ron", 8, 0x00b1_7b02);
    play("rulesets/duel.ron", 2, 0x00b1_7b03);
}
