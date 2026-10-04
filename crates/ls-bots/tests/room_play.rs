//! Bots playing a real match in-process: an authoritative `ls_game` `Room`
//! and a full room of [`BotCore`]s (8, `RoomConfig::max_players`) exchange
//! encoded packets with a fixed latency, stepped tick by tick
//! (deterministic, no sockets, no wall clock). The match is shortened to
//! 60 s so it ends inside the test and the post-match phase is covered.
//!
//! Needs whole-level generation (the room pregenerates, scoring covers every
//! chunk), which takes ~25 s in debug builds, so it runs only in release:
//! `cargo test -p ls-bots --release`.

use std::collections::VecDeque;

use ls_bots::{BotCore, BotStats, Flow};
use ls_game::ls_net::Message;
use ls_game::ls_sim::ModeRules;
use ls_game::ls_worldgen::{Generator, ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::{GameTerrain, MatchConfig};

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
    joined_at: u64,
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "full level generation is slow in debug builds; run with --release"
)]
fn bots_play_a_full_match_without_desync() {
    const MATCH_SECS: u64 = 60;
    let code = ShareCode::new(0x00b0_7b07, Sliders::new(40, 30));
    let mut mc = MatchConfig::paint_the_void(code);
    let ModeRules::PaintTheVoid(rules) = &mut mc.rules.mode else {
        unreachable!("a Paint the Void rule set")
    };
    rules.match_secs = MATCH_SECS as f32;
    let mut room = Room::new(mc, RoomConfig::default()).unwrap();
    let mut clients: Vec<Client> = Vec::new();
    // The match, then 5 s of the post-match phase.
    let end_tick = 60 * MATCH_SECS;
    let total_ticks: u64 = end_tick + 60 * 5;
    let bots = RoomConfig::default().max_players as usize;
    // Reconciliation counters shortly after the match ended.
    let mut at_end: Vec<(u64, u64)> = Vec::new();

    for now in 0..total_ticks {
        // Staggered joins: one bot every half second, latencies 2-5 ticks.
        if now % 30 == 0 && clients.len() < bots {
            let i = clients.len();
            let delay = 2 + (i as u64 % 4);
            let mut c = Client {
                id: None,
                core: None,
                stats: BotStats::new(format!("bot{}", i + 1)),
                up: Link::new(delay),
                down: Link::new(delay),
                joined_at: now,
            };
            let hello = Message::Hello(ls_bots::bot_hello(c.stats.name.clone(), 100 + i as u64));
            c.up.push(now, hello.encode().unwrap());
            clients.push(c);
        }

        // Client -> server.
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

        // Server -> client, then one bot tick.
        for (i, c) in clients.iter_mut().enumerate() {
            while let Some(bytes) = c.down.pop_due(now) {
                match &mut c.core {
                    None => {
                        let w = ls_game::ls_net::check_welcome(Message::decode(&bytes).unwrap())
                            .expect("welcome");
                        c.core = Some(BotCore::from_welcome(&w, 100 + i as u64).unwrap());
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
        if now == end_tick + 60 {
            at_end = clients
                .iter()
                .map(|c| (c.stats.corrections, c.stats.snaps))
                .collect();
        }
    }
    assert!(!room.world().mode().is_running(), "the match ended");

    // Terrain replicated exactly: each bot's terrain equals the server's
    // edit log replayed up to the edits that bot has applied.
    let server_log = room.terrain().edit_log().to_vec();
    assert!(
        server_log.iter().any(|e| !e.changes_geometry()),
        "bots never painted"
    );
    let mut combat = 0;
    let mut paint = 0;
    for (i, c) in clients.iter_mut().enumerate() {
        let core = c.core.as_ref().expect("joined");
        core.finish(&mut c.stats);
        let s = &c.stats;
        eprintln!("{s:?}");
        assert_eq!(s.joins, 1);
        assert!(
            !core.brain().scrap_spots().is_empty(),
            "{}: no pickup points",
            s.name
        );
        assert_eq!(s.protocol_errors, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.matches_over, 1, "{}: saw the match end", s.name);
        // After the match the server stops moving ships and the bots stop
        // predicting movement: no corrections or snaps in the last 4 s.
        assert_eq!(
            (s.corrections, s.snaps),
            at_end[i],
            "{}: reconciliation after the match ended",
            s.name
        );
        let played = total_ticks - c.joined_at;
        let played_running = end_tick - c.joined_at;
        assert!(
            s.inputs_sent + 20 >= played,
            "{}: {} inputs in {played} ticks",
            s.name,
            s.inputs_sent
        );
        assert!(
            s.snapshots * 2 + 40 >= played,
            "{}: {} snapshots",
            s.name,
            s.snapshots
        );
        assert!(s.delta_snapshots > s.full_snapshots, "{}: {s:?}", s.name);
        assert!(
            s.max_reconcile_error < 1.0,
            "{}: max reconciliation error {}",
            s.name,
            s.max_reconcile_error
        );
        let n = core.session().edit_count() as usize;
        assert!(n <= server_log.len());
        let reference = GameTerrain::replay(Generator::from_code(&code).unwrap(), &server_log[..n]);
        assert_eq!(
            core.session().terrain().state_hash(),
            reference.state_hash(),
            "{}: terrain differs from the server after {n} edits",
            s.name
        );
        combat += s.behavior.combat_ticks;
        paint += s.behavior.paint_triggers;
        // Bots move around instead of idling.
        assert!(
            s.behavior.wander_ticks + s.behavior.paint_ticks + s.behavior.combat_ticks
                > played_running / 2
        );
    }
    let st = room.stats();
    eprintln!("room: {st:?}; combat ticks {combat}, paint triggers {paint}");
    assert!(paint > 100, "paint triggers {paint}");
    // The server rarely had to repeat an input (bots keep up at 60 Hz).
    assert!(st.missing_inputs < 60 * bots as u64, "{st:?}");
}
