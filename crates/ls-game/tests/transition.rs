//! Match transitions on one connection (protocol 5, M3 wave A): the host
//! replaces a finished match with the next one and sends every connected
//! client a new `Welcome` on the same transport; the client sessions
//! rebuild themselves without reconnecting.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::collections::VecDeque;

use ls_game::client::{ClientSession, Incoming};
use ls_game::ls_math::Vec3;
use ls_game::ls_net::{
    check_welcome, mode_phase, Channel, Hello, LinkConditions, LobbyPhase, LobbyStateMsg,
    MemoryTransport, Message, Transport,
};
use ls_game::ls_sim::{InputCmd, ShipTuning, WeaponTable};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{LobbyRequest, Room, RoomConfig};
use ls_game::MatchConfig;

/// One direction of a link with a fixed delay in ticks.
struct Delayed {
    delay: u64,
    queue: VecDeque<(u64, Channel, Vec<u8>)>,
}

impl Delayed {
    fn new(delay: u64) -> Self {
        Delayed {
            delay,
            queue: VecDeque::new(),
        }
    }

    fn push(&mut self, now: u64, channel: Channel, bytes: Vec<u8>) {
        self.queue.push_back((now + self.delay, channel, bytes));
    }

    fn flush(&mut self, now: u64, t: &mut MemoryTransport) {
        while self.queue.front().is_some_and(|(due, _, _)| *due <= now) {
            let (_, channel, bytes) = self.queue.pop_front().unwrap();
            let channel = if bytes.len() > t.max_unreliable_size() {
                Channel::Reliable
            } else {
                channel
            };
            t.send(channel, &bytes).unwrap();
        }
    }
}

struct Client {
    name: String,
    hello: Hello,
    /// Wire id in the current room.
    id: Option<u8>,
    session: Option<ClientSession>,
    server_end: MemoryTransport,
    client_end: MemoryTransport,
    up: Delayed,
    down: Delayed,
    transitions_seen: u64,
    results_seen: u64,
    lobby_seen: u64,
}

/// A host running matches back to back on the same connections.
struct Host {
    room: Room,
    epoch: u8,
    code: ShareCode,
    clients: Vec<Client>,
    now: u64,
}

fn room(code: ShareCode, epoch: u8, match_ticks: u64) -> Room {
    let mut r = Room::new(
        MatchConfig::paint_the_void(code),
        RoomConfig {
            epoch,
            ..RoomConfig::default()
        },
    )
    .unwrap();
    // Short matches: the timer runs out after `match_ticks`.
    r.world_mut().mode_mut().end_tick = match_ticks;
    r.set_room_code(format!("R-{epoch}"));
    r
}

const MATCH_TICKS: u64 = 240;

impl Host {
    fn new(code: ShareCode) -> Self {
        Host {
            room: room(code, 0, MATCH_TICKS),
            epoch: 0,
            code,
            clients: Vec::new(),
            now: 0,
        }
    }

    fn connect(&mut self, name: &str, delay: u64, link: LinkConditions) {
        let (server_end, client_end) = MemoryTransport::pair_with(link);
        let hello = ls_game::client_hello(name, None, None);
        let mut c = Client {
            name: name.to_string(),
            hello: hello.clone(),
            id: None,
            session: None,
            server_end,
            client_end,
            up: Delayed::new(delay),
            down: Delayed::new(delay),
            transitions_seen: 0,
            results_seen: 0,
            lobby_seen: 0,
        };
        c.up.push(
            self.now,
            Channel::Reliable,
            Message::Hello(hello).encode().unwrap(),
        );
        self.clients.push(c);
    }

    /// Replaces the room with the next match and welcomes every connected
    /// client into it on its open connection.
    fn next_match(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        let code = ShareCode::new(self.code.seed + self.epoch as u64, self.code.sliders);
        let old = std::mem::replace(&mut self.room, room(code, self.epoch, MATCH_TICKS));
        drop(old);
        for c in &mut self.clients {
            if c.id.is_some() {
                c.id = Some(self.room.join(&c.hello).expect("rejoin accepted"));
            }
        }
        self.flush_room();
    }

    fn flush_room(&mut self) {
        for o in self.room.take_outgoing() {
            let c = self
                .clients
                .iter_mut()
                .find(|c| c.id == Some(o.client))
                .unwrap();
            c.down.push(self.now, o.channel, o.bytes);
        }
    }

    fn step(&mut self) {
        self.now += 1;
        let now = self.now;
        for c in &mut self.clients {
            c.up.flush(now, &mut c.client_end);
            while let Some(p) = c.server_end.recv().unwrap() {
                match c.id {
                    None => c.id = Some(self.room.join_packet(&p.data).expect("join accepted")),
                    Some(id) => self.room.handle_packet(id, &p.data).unwrap(),
                }
            }
        }
        self.room.step();
        self.flush_room();
        for c in &mut self.clients {
            c.down.flush(now, &mut c.server_end);
            while let Some(p) = c.client_end.recv().unwrap() {
                match &mut c.session {
                    None => {
                        let w = check_welcome(Message::decode(&p.data).unwrap()).unwrap();
                        let s = ClientSession::from_welcome(
                            &w,
                            WeaponTable::builtin(),
                            ShipTuning::tinker(),
                        )
                        .unwrap();
                        c.session = Some(s);
                    }
                    Some(s) => match s.handle_packet(&p.data) {
                        Ok(Incoming::MatchTransition { .. }) => c.transitions_seen += 1,
                        Ok(Incoming::Results) => c.results_seen += 1,
                        Ok(Incoming::Lobby) => c.lobby_seen += 1,
                        Ok(_) => {}
                        Err(e) => panic!("{}: bad packet: {e}", c.name),
                    },
                }
            }
            if let Some(s) = &mut c.session {
                // Fly forward a little so prediction has work to do.
                let controls = InputCmd::idle(0).with_thrust(Vec3::new(0.0, 0.0, -0.3));
                for m in s.frame(1, controls) {
                    c.up.push(now, m.channel(), m.encode().unwrap());
                }
            }
        }
    }

    fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }

    fn session(&self, i: usize) -> &ClientSession {
        self.clients[i].session.as_ref().unwrap()
    }
}

/// Three matches after the first on one connection each: every client
/// rebuilds in place (no reconnect), costs at most one full snapshot per
/// transition, ends up with the server's terrain and a converged
/// prediction, and drops the old match's packets by epoch.
#[test]
fn sessions_survive_three_match_transitions_on_one_connection() {
    let code = ShareCode::new(0x7a_2a17, Sliders::new(30, 30));
    let mut h = Host::new(code);
    // A local client, one with a 3-tick link, one with a lossy, reordering
    // link (old snapshots may overtake the new Welcome there).
    h.connect("local", 1, LinkConditions::default());
    h.connect("far", 3, LinkConditions::default());
    h.connect(
        "lossy",
        2,
        LinkConditions {
            loss: 0.05,
            duplicate: 0.05,
            reorder: 0.2,
            seed: 7,
            ..LinkConditions::default()
        },
    );
    let n = h.clients.len();
    let mut full_before: Vec<u64> = vec![0; n];
    for round in 0..4u64 {
        // Play the match to its end and into the results.
        h.run(MATCH_TICKS + 60);
        assert!(!h.room.world().mode().is_running(), "round {round}: ended");
        let results = h.room.results().expect("results after the end").clone();
        assert_eq!(results.rows.len(), n);
        // The host's lobby talks to everyone on the same connection.
        h.room
            .broadcast(&Message::LobbyState(Box::new(LobbyStateMsg {
                phase: LobbyPhase::Results,
                phase_ticks_left: 600,
                room_code: h.room.room_code().to_string(),
                next_mode: 0,
                next_ruleset: "paint_the_void".into(),
                next_share_code: code.encode().unwrap(),
                min_players: 2,
                max_players: 8,
                teams: 2,
                players: Vec::new(),
            })));
        // A ready-up from the first client reaches the host's lobby queue.
        let ready = h.session(0).ready(true);
        let c0 = &mut h.clients[0];
        c0.up.push(h.now, ready.channel(), ready.encode().unwrap());
        h.run(30);
        let reqs = h.room.take_lobby_requests();
        assert_eq!(
            reqs,
            vec![(h.clients[0].id.unwrap(), LobbyRequest::Ready(true))]
        );
        for (i, c) in h.clients.iter().enumerate() {
            let s = h.session(i);
            assert_eq!(s.stats().transitions, round, "{}", c.name);
            assert_eq!(s.results(), Some(&results), "{}: results", c.name);
            assert_eq!(c.results_seen, round + 1, "{}", c.name);
            assert!(s.lobby().is_some(), "{}: lobby state", c.name);
            assert_eq!(s.mode().phase, mode_phase::ENDED, "{}", c.name);
            assert_eq!(
                s.terrain().state_hash(),
                h.room.terrain().state_hash(),
                "{}: terrain",
                c.name
            );
            // No full-snapshot storm: one full snapshot for this match's
            // join or transition on a local link; on longer links the ones
            // sent before the first acknowledgement arrives (one per snapshot
            // interval of round trip, as for any join).
            let full = s.stats().full_snapshots;
            let delay = c.up.delay;
            let max_full = 1 + (2 * delay - 1) / 2;
            assert!(
                (1..=max_full).contains(&(full - full_before[i])),
                "{} round {round}: {} full snapshots (at most {max_full})",
                c.name,
                full - full_before[i]
            );
            if delay == 1 {
                assert_eq!(full - full_before[i], 1, "{}: local link", c.name);
            }
            full_before[i] = full;
            assert!(s.stats().delta_snapshots > 100 * (round + 1));
        }
        if round == 3 {
            break;
        }
        // Next match on the same connections.
        h.next_match();
        h.run(90);
        for (i, c) in h.clients.iter().enumerate() {
            let s = h.session(i);
            assert_eq!(c.transitions_seen, round + 1, "{}", c.name);
            assert_eq!(s.epoch(), h.epoch, "{}", c.name);
            assert_eq!(s.room_code(), format!("R-{}", h.epoch));
            assert!(s.results().is_none(), "{}: results reset", c.name);
            assert!(s.lobby().is_some(), "{}: lobby state kept", c.name);
            assert!(s.is_synced(), "{}: synced again", c.name);
            assert_eq!(s.mode().phase, mode_phase::RUNNING, "{}", c.name);
        }
    }
    // The prediction converged in the last match: the newest snapshot's own
    // ship matches the prediction for that tick.
    for i in 0..n {
        let s = h.session(i);
        let snap = s.latest_snapshot().unwrap();
        let own = snap.ships.iter().find(|x| x.id == s.player()).unwrap();
        let predicted = s.predictor().state_at(snap.tick as u64).unwrap();
        assert!(
            (predicted.pos - own.pos).length() < 0.05,
            "{}: prediction {:?} vs server {:?}",
            h.clients[i].name,
            predicted.pos,
            own.pos
        );
    }
    // Old-epoch inputs that were in flight at a transition were dropped by
    // the new room, never applied.
    assert_eq!(h.room.stats().encode_errors, 0);
}

/// Packets of the previous match are dropped by epoch on both ends.
#[test]
fn packets_of_another_epoch_are_dropped() {
    let code = ShareCode::new(99, Sliders::new(20, 20));
    let mut h = Host::new(code);
    h.connect("a", 1, LinkConditions::default());
    h.run(60);
    let s = h.session(0);
    let old_input = s.clone().tick(InputCmd::idle(0));
    // Capture a (full) snapshot of epoch 0 as the server sent it.
    let old_snapshot = loop {
        h.room.step();
        let snap = h
            .room
            .take_outgoing()
            .into_iter()
            .find(|o| matches!(Message::decode(&o.bytes), Ok(Message::Snapshot(_))));
        if let Some(o) = snap {
            break o.bytes;
        }
    };
    h.next_match();
    h.run(60);
    // An input of epoch 0 reaches the epoch-1 room: counted and dropped.
    let id = h.clients[0].id.unwrap();
    let before = h.room.stats().stale_epoch_inputs;
    h.room
        .handle_packet(id, &old_input.encode().unwrap())
        .unwrap();
    assert_eq!(h.room.stats().stale_epoch_inputs, before + 1);
    // A snapshot of epoch 0 reaches the epoch-1 session: ignored.
    let s = h.clients[0].session.as_mut().unwrap();
    let tick = s.latest_snapshot().unwrap().tick;
    assert_eq!(s.handle_packet(&old_snapshot).unwrap(), Incoming::Ignored);
    assert_eq!(s.stats().stale_epoch_snapshots, 1);
    assert_eq!(s.latest_snapshot().unwrap().tick, tick);
}
