//! Last Color Standing (M3 wave B5) played to a result in-process: an
//! authoritative `ls_game` `Room` with the default Last Color Standing rule
//! set and 8 [`BotCore`]s exchanging encoded packets with a fixed latency,
//! stepped tick by tick (the `room_play.rs` harness). The bots have no mode
//! objective yet (C2): they hunt and fight, kills drain colour, and the
//! stalemate drain (moved from 8 minutes to 60 s, 1 colour per 0.3 s, so
//! the test stays short) ends the match with a winner or a draw.
//!
//! Needs whole-level generation, so it runs only in release:
//! `cargo test -p ls-bots --release --test last_color_standing_match`.

use std::collections::VecDeque;

use ls_bots::{BotCore, BotStats, Flow};
use ls_game::ls_net::Message;
use ls_game::ls_sim::mode::player_flags;
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

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "full level generation is slow in debug builds; run with --release"
)]
fn eight_bots_play_last_color_standing_to_a_result() {
    let code = ShareCode::new(0x00c0_1042, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    mc.rules = RuleSet::default_for(ModeKind::LastColorStanding);
    // The default rule set, round-tripped through its code like a host would.
    mc.rules = RuleSet::from_code(&mc.rules.to_code().unwrap()).unwrap();
    mc.rules.mode.set_field("stalemate_secs", 60.0);
    mc.rules.mode.set_field("stalemate_drain_secs", 0.3);
    let mut room = Room::new(mc, RoomConfig::default()).unwrap();
    let bots = RoomConfig::default().max_players as usize;
    assert_eq!(bots, 8);
    let mut clients: Vec<Client> = Vec::new();
    // At most 60 s + 100 x 0.3 s; then 3 s of the post-match phase.
    let limit: u64 = 60 * 100;
    let mut ended_at = None;
    let mut now = 0;
    while ended_at.is_none_or(|e| now < e + 180) {
        assert!(now < limit, "no result after {now} ticks");
        if now % 30 == 0 && clients.len() < bots {
            let i = clients.len();
            let delay = 2 + (i as u64 % 4);
            let c = Client {
                id: None,
                core: None,
                stats: BotStats::new(format!("bot{}", i + 1)),
                up: Link::new(delay),
                down: Link::new(delay),
            };
            let hello = Message::Hello(ls_bots::bot_hello(c.stats.name.clone(), 500 + i as u64));
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
        if ended_at.is_none() && !room.world().mode().is_running() {
            ended_at = Some(now);
        }
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
                        let core = BotCore::from_welcome(&w, 500 + i as u64).unwrap();
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
        now += 1;
    }

    let world = room.world();
    let mode = world.mode();
    let MatchPhase::Ended { winner } = mode.phase else {
        unreachable!("ended")
    };
    let res = room.current_results();
    let kills: u32 = world.ships().map(|s| s.kills).sum();
    eprintln!(
        "ended at tick {} ({:.1} s), winner {winner:?}, {kills} kills; results {res:?}",
        ended_at.unwrap(),
        ended_at.unwrap() as f32 / 60.0
    );
    // Every player but at most one is out of colour; the winner (if any)
    // is the one with colour left.
    let with_colour: Vec<_> = mode
        .players
        .iter()
        .filter(|(_, p)| p.meter > 0)
        .map(|(id, _)| world.ship(*id).unwrap().team)
        .collect();
    assert!(with_colour.len() <= 1, "{with_colour:?}");
    assert_eq!(winner, with_colour.first().copied());
    assert_eq!(mode.players.len(), 8);
    for (id, p) in &mode.players {
        if p.meter == 0 {
            assert!(p.flags & player_flags::ELIMINATED != 0, "{id:?}");
            assert!(!world.ship(*id).unwrap().alive, "{id:?} out of play");
        }
    }
    assert!(kills > 0, "the bots fought");
    let view = room.mode_view();
    assert_eq!(view.kind, ModeKind::LastColorStanding);

    for c in &mut clients {
        let core = c.core.as_ref().expect("joined");
        core.finish(&mut c.stats);
        let s = &c.stats;
        eprintln!("{s:?}");
        assert_eq!(s.joins, 1);
        assert_eq!(s.protocol_errors, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.matches_over, 1, "{}: saw the match end", s.name);
    }
}
