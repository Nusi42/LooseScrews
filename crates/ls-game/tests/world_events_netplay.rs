//! M4 wave B package B1 acceptance (`docs/milestone-4-plan.md` §5 wave B):
//! every world and movement event (Bored level, Level hiccups, Gravity
//! coffee break, Wrong genre, Big head day, Rubber chickens, Moving day,
//! Everything is cake) forced in turn in an in-process match with 4 clients
//! (a `Room` and `ClientSession`s over `MemoryTransport` with latency, one
//! link lossy) that fly, turn and shoot: 0 desyncs (no protocol error, no
//! unexplained prediction snap, every client on the server's mood view and
//! up) and a reconciliation error under 0.1 u while each event runs.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::collections::{BTreeMap, VecDeque};

use ls_game::client::ClientSession;
use ls_game::ls_math::Vec3;
use ls_game::ls_net::{
    check_welcome, Channel, LinkConditions, MemoryTransport, Message, Transport,
};
use ls_game::ls_sim::{Buttons, EventKind, InputCmd, ShipTuning, WeaponTable};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

type Script = Box<dyn Fn(u64, &ClientSession) -> InputCmd>;

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

struct SimClient {
    name: String,
    id: Option<u8>,
    session: Option<ClientSession>,
    server_end: MemoryTransport,
    client_end: MemoryTransport,
    up: Delayed,
    down: Delayed,
    script: Script,
    /// Largest reconciliation error per running event (server's view).
    errors: BTreeMap<u8, f32>,
    /// Prediction snaps after the first one (the sync).
    snaps: u64,
    /// Server ticks of own deaths and respawns (a snap there is expected).
    life_events: Vec<u64>,
    snap_ticks: Vec<u64>,
    was_alive: Option<bool>,
}

struct Harness {
    room: Room,
    clients: Vec<SimClient>,
    now: u64,
}

impl Harness {
    fn new(cfg: MatchConfig) -> Self {
        Harness {
            room: Room::new(cfg, RoomConfig::default()).unwrap(),
            clients: Vec::new(),
            now: 0,
        }
    }

    fn connect(&mut self, name: &str, delay: u64, link: LinkConditions, script: Script) {
        let (server_end, client_end) = MemoryTransport::pair_with(link);
        let mut c = SimClient {
            name: name.to_string(),
            id: None,
            session: None,
            server_end,
            client_end,
            up: Delayed::new(delay),
            down: Delayed::new(delay),
            script,
            errors: BTreeMap::new(),
            snaps: 0,
            life_events: Vec::new(),
            snap_ticks: Vec::new(),
            was_alive: None,
        };
        let hello = Message::Hello(ls_game::client_hello(name, None, None))
            .encode()
            .unwrap();
        c.up.push(self.now, Channel::Reliable, hello);
        self.clients.push(c);
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
        let event = self.room.net_mode_state().mood.event;
        for o in self.room.take_outgoing() {
            let c = self
                .clients
                .iter_mut()
                .find(|c| c.id == Some(o.client))
                .unwrap();
            c.down.push(now, o.channel, o.bytes);
        }
        for c in &mut self.clients {
            c.down.flush(now, &mut c.server_end);
            while let Some(p) = c.client_end.recv().unwrap() {
                match &mut c.session {
                    None => {
                        let w = check_welcome(Message::decode(&p.data).unwrap()).unwrap();
                        c.session = Some(
                            ClientSession::from_welcome(
                                &w,
                                WeaponTable::builtin(),
                                ShipTuning::tinker(),
                            )
                            .unwrap(),
                        );
                    }
                    Some(s) => {
                        let before = s.prediction_stats();
                        s.handle_packet(&p.data)
                            .unwrap_or_else(|e| panic!("{}: bad packet: {e}", c.name));
                        let after = s.prediction_stats();
                        let reconciled =
                            after.accepted + after.corrected > before.accepted + before.corrected;
                        if s.is_synced() && reconciled {
                            let e = c.errors.entry(event).or_insert(0.0);
                            *e = e.max(after.last_error);
                        }
                        if after.snapped > before.snapped && before.snapped > 0 {
                            c.snaps += after.snapped - before.snapped;
                            c.snap_ticks.push(now);
                        }
                        let alive = s.local_ship().map(|s| s.alive);
                        if alive.is_some() && alive != c.was_alive {
                            c.life_events.push(now);
                            c.was_alive = alive;
                        }
                    }
                }
            }
            if let Some(s) = &mut c.session {
                let controls = (c.script)(now, s);
                for m in s.frame(1, controls) {
                    c.up.push(now, m.channel(), m.encode().unwrap());
                }
            }
        }
    }
}

/// Flies a pattern with horizon assist (so a turned up rolls the ship),
/// boosting in bursts, turning and pitching, firing now and then (so the
/// events' projectile rules replicate).
fn flyer(seed: u64) -> Script {
    Box::new(move |now, s| {
        let mut cmd = InputCmd::idle(0);
        if s.local_ship().is_none() || !s.is_synced() {
            return cmd;
        }
        let phase = ((now + seed * 37) / 50) % 5;
        cmd.thrust = match phase {
            0 => Vec3::new(0.0, 0.0, -1.0),
            1 => Vec3::new(0.6, 0.2, -0.6),
            2 => Vec3::ZERO,
            3 => Vec3::new(-0.5, -0.3, 0.4),
            _ => Vec3::new(0.0, 0.5, -0.8),
        };
        cmd.look_yaw = if phase == 1 { 0.012 } else { -0.004 };
        cmd.look_pitch = if phase == 3 { 0.008 } else { -0.002 };
        cmd.buttons = Buttons::HORIZON_ASSIST
            | if phase == 0 {
                Buttons::BOOST
            } else {
                Buttons::NONE
            }
            | if (now + seed * 13) % 90 < 6 {
                Buttons::FIRE_PRIMARY
            } else {
                Buttons::NONE
            };
        cmd
    })
}

fn lossy() -> LinkConditions {
    LinkConditions {
        loss: 0.05,
        reorder: 0.05,
        duplicate: 0.02,
        seed: 11,
        ..LinkConditions::default()
    }
}

/// Reconciliation bound of the acceptance criterion (wave B, B1-B3).
const MAX_ERROR: f32 = 0.1;

/// Server ticks around an own death or respawn in which a snap is expected.
const LIFE_WINDOW: u64 = 240;

#[test]
fn world_events_keep_four_clients_in_sync() {
    let mut sliders = Sliders::new(40, 30);
    // Wells for Gravity coffee break.
    sliders.gravity_chaos = 100;
    let code = ShareCode::new(0x6231_6576, sliders);
    let mut h = Harness::new(MatchConfig::paint_the_void(code));
    h.connect("alpha", 3, LinkConditions::default(), flyer(0));
    h.connect("bravo", 5, LinkConditions::default(), flyer(1));
    h.connect("charlie", 4, lossy(), flyer(2));
    h.connect("delta", 6, LinkConditions::default(), flyer(3));
    for _ in 0..150 {
        h.step();
    }
    assert!(
        !h.room.world().well_field().statics.is_empty(),
        "the level has wells"
    );
    let events = [
        EventKind::BoredLevel,
        EventKind::LevelHiccups,
        EventKind::GravityCoffeeBreak,
        EventKind::WrongGenre,
        EventKind::BigHeadDay,
        EventKind::RubberChickens,
        EventKind::MovingDay,
        EventKind::EverythingIsCake,
    ];
    // Forced events queue: each starts when the one before ended.
    let t0 = h.room.tick() + 30;
    for kind in events {
        h.room.world_mut().schedule_event(t0, kind);
    }
    let mut ran: BTreeMap<u8, u64> = BTreeMap::new();
    let mut chickens = 0;
    for _ in 0..9000 {
        h.step();
        let mood = h.room.net_mode_state().mood;
        if mood.event != ls_game::ls_net::mode::NONE {
            *ran.entry(mood.event).or_insert(0) += 1;
        }
        // Every client agrees with the server's mood a moment later
        // (checked on quiet ticks: the event index matches when the server
        // has been on it for a while).
        if mood.event == EventKind::RubberChickens.index() {
            chickens += h
                .clients
                .iter()
                .filter_map(|c| c.session.as_ref())
                .flat_map(|s| s.projectiles())
                .filter(|p| p.chicken)
                .count();
        }
        if h.room.world().director().schedule.is_empty()
            && h.room.world().mood_view().event.is_none()
        {
            break;
        }
    }
    for kind in events {
        assert!(
            ran.get(&kind.index()).copied().unwrap_or(0) >= 300,
            "{kind:?} ran: {ran:?}"
        );
    }
    assert!(chickens > 0, "chicken projectiles replicated");
    for _ in 0..30 {
        h.step();
    }
    let server = h.room.net_mode_state().mood;
    for c in &h.clients {
        let s = c.session.as_ref().unwrap();
        assert_eq!(s.mood().event, server.event, "{}", c.name);
        assert_eq!(s.mood().up, server.up, "{}: the turned up", c.name);
        let p = s.prediction_stats();
        eprintln!("{}: {p:?}", c.name);
        for (event, err) in &c.errors {
            let name = EventKind::from_index(*event).map_or("none", |k| k.id());
            eprintln!("{}: {name:<22} max error {err:.5} u", c.name);
            assert!(
                *err < MAX_ERROR,
                "{}: {name} reconciliation error {err}",
                c.name
            );
        }
        let unexplained: Vec<u64> = c
            .snap_ticks
            .iter()
            .copied()
            .filter(|t| !c.life_events.iter().any(|l| l.abs_diff(*t) <= LIFE_WINDOW))
            .collect();
        assert!(
            unexplained.is_empty(),
            "{}: desync snaps at {unexplained:?}",
            c.name
        );
        assert!(p.accepted > 1000, "{}: {p:?}", c.name);
        let _ = c.snaps;
    }
    // The level's up really turned (Bored level ran first).
    assert!(server.up.dot(Vec3::Y) < 0.1, "{:?}", server.up);
}
