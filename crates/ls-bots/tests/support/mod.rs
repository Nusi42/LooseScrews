//! In-process match harness shared by `race_match.rs` and
//! `mood_matches.rs` (M4 package C5): an authoritative `ls_game` `Room` and
//! [`BotCore`]s exchanging encoded packets with a fixed latency (2-5 ticks),
//! stepped tick by tick as in `mode_matches.rs`.

#![allow(dead_code)]

use std::collections::VecDeque;

use ls_bots::{BotCore, BotStats, BrainConfig, Flow};
use ls_game::ls_net::Message;
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

pub struct Link {
    delay: u64,
    queue: VecDeque<(u64, Vec<u8>)>,
}

impl Link {
    pub fn new(delay: u64) -> Self {
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

pub struct Client {
    pub id: Option<u8>,
    pub core: Option<BotCore>,
    pub stats: BotStats,
    seed: u64,
    cfg: BrainConfig,
    up: Link,
    down: Link,
}

/// A room with bots, stepped one tick at a time.
pub struct Match {
    pub room: Room,
    pub clients: Vec<Client>,
    pub now: u64,
    /// Brain configuration of the bots that join next.
    pub cfg: Box<dyn Fn(usize) -> BrainConfig>,
    bots: usize,
    seed_base: u64,
}

impl Match {
    /// A room for `mc` that `bots` bots join one every 10 ticks (seeds
    /// `seed_base + i`).
    pub fn new(mc: MatchConfig, bots: usize, seed_base: u64) -> Match {
        let room = Room::new(mc, RoomConfig::default()).unwrap();
        Match {
            room,
            clients: Vec::new(),
            now: 0,
            cfg: Box::new(|_| BrainConfig::default()),
            bots,
            seed_base,
        }
    }

    /// Joins every bot at once, before the room's first step (The
    /// Architect counts its rounds from the players at the start).
    pub fn join_all(&mut self) {
        while self.clients.len() < self.bots {
            self.add_client(true);
        }
    }

    fn add_client(&mut self, direct: bool) {
        let now = self.now;
        let i = self.clients.len();
        let delay = 2 + (i as u64 % 4);
        let seed = self.seed_base + i as u64;
        let mut c = Client {
            id: None,
            core: None,
            stats: BotStats::new(format!("bot{}", i + 1)),
            seed,
            cfg: (self.cfg)(i),
            up: Link::new(delay),
            down: Link::new(delay),
        };
        let hello = Message::Hello(ls_bots::bot_hello(c.stats.name.clone(), seed));
        let bytes = hello.encode().unwrap();
        if direct {
            c.id = Some(self.room.join_packet(&bytes).expect("join accepted"));
        } else {
            c.up.push(now, bytes);
        }
        self.clients.push(c);
    }

    /// One tick: joins, client to server, the room's step, server to
    /// clients, every bot's tick.
    pub fn step(&mut self) {
        let now = self.now;
        if now.is_multiple_of(10) && self.clients.len() < self.bots {
            self.add_client(false);
        }
        for c in &mut self.clients {
            while let Some(bytes) = c.up.pop_due(now) {
                match c.id {
                    None => c.id = Some(self.room.join_packet(&bytes).expect("join accepted")),
                    Some(id) => self.room.handle_packet(id, &bytes).unwrap(),
                }
            }
        }
        self.room.step();
        for o in self.room.take_outgoing() {
            let c = self
                .clients
                .iter_mut()
                .find(|c| c.id == Some(o.client))
                .unwrap();
            c.down.push(now, o.bytes);
        }
        for c in &mut self.clients {
            while let Some(bytes) = c.down.pop_due(now) {
                match &mut c.core {
                    None => {
                        let w = ls_game::ls_net::check_welcome(Message::decode(&bytes).unwrap())
                            .expect("welcome");
                        let core = BotCore::from_welcome_with(&w, c.seed, c.cfg).unwrap();
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
        self.now += 1;
    }

    /// Ends every session (adds the snapshot counters) and returns each
    /// bot's wire id and statistics.
    pub fn finish(&mut self) -> Vec<(u8, BotStats)> {
        self.clients
            .iter_mut()
            .map(|c| {
                c.core.as_ref().expect("joined").finish(&mut c.stats);
                (c.id.expect("joined"), std::mem::take(&mut c.stats))
            })
            .collect()
    }
}
