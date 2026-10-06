//! M4 A5 acceptance (`docs/milestone-4-plan.md` §5 wave A): clients
//! predicting through the comedy director's movement effects, a turned
//! level up, a gravity well field and dynamic wells that start and end
//! mid-flight keep the reconciliation error under 0.05 u, in an in-process
//! netplay test (a `Room` and `ClientSession`s over `MemoryTransport` with
//! latency, one link lossy).

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::collections::VecDeque;

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
    /// Largest position error at a snapshot tick (`PredictionStats::
    /// last_error`) after the session synchronized, and where.
    max_error: f32,
    max_error_tick: u64,
    lossy: bool,
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
            max_error: 0.0,
            max_error_tick: 0,
            lossy: link.loss > 0.0,
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
                        if s.is_synced() && reconciled && after.last_error > c.max_error {
                            c.max_error = after.last_error;
                            c.max_error_tick = now;
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

    fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }
}

/// Flies a pattern with horizon assist on (so a turned up changes the
/// roll), boosting in bursts and turning, so the ship crosses the level's
/// open space and its wells.
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
            };
        cmd
    })
}

fn lossy() -> LinkConditions {
    LinkConditions {
        loss: 0.05,
        reorder: 0.05,
        duplicate: 0.02,
        seed: 7,
        ..LinkConditions::default()
    }
}

/// Reconciliation bound of the acceptance criterion.
const MAX_ERROR: f32 = 0.05;

fn check_errors(h: &Harness, what: &str) {
    for c in &h.clients {
        let s = c.session.as_ref().expect("joined");
        let p = s.prediction_stats();
        eprintln!(
            "{what}: {} max error {:.5} u at tick {} ({p:?})",
            c.name, c.max_error, c.max_error_tick
        );
        assert!(p.accepted > 100, "{}: {p:?}", c.name);
        // A lost snapshot can let one correction grow by a few ticks of
        // drift; the bound holds for perfect and lossy links alike.
        assert!(
            c.max_error < MAX_ERROR,
            "{what}: {} reconciliation error {} at tick {}",
            c.name,
            c.max_error,
            c.max_error_tick
        );
        let _ = c.lossy;
    }
}

/// Events the director forces while the clients fly (their movement
/// effects are pure functions of the replicated mood view): every client
/// sees the server's mood view and keeps its prediction within the bound.
#[test]
fn prediction_holds_through_forced_events() {
    let code = ShareCode::new(0x4d4f_4f44, Sliders::new(40, 30));
    let mut h = Harness::new(MatchConfig::paint_the_void(code));
    h.connect("alpha", 3, LinkConditions::default(), flyer(0));
    h.connect("bravo", 5, LinkConditions::default(), flyer(1));
    h.connect("charlie", 4, lossy(), flyer(2));
    h.run(120);
    let t0 = h.room.tick();
    for (i, kind) in [
        EventKind::LevelHiccups,
        EventKind::WrongGenre,
        EventKind::BoredLevel,
        EventKind::GravityCoffeeBreak,
    ]
    .into_iter()
    .enumerate()
    {
        h.room
            .world_mut()
            .schedule_event(t0 + 30 + i as u64 * 600, kind);
    }
    let mut seen_events = 0;
    for _ in 0..2400 {
        h.step();
        let server = h.room.net_mode_state().mood;
        if server.event != ls_game::ls_net::mode::NONE {
            seen_events += 1;
        }
    }
    assert!(seen_events > 600, "events ran: {seen_events} ticks");
    // Every client ends on the server's mood view.
    let server = h.room.net_mode_state().mood;
    h.run(12);
    for c in &h.clients {
        let s = c.session.as_ref().unwrap();
        assert_eq!(s.mood().mood, server.mood, "{}", c.name);
    }
    check_errors(&h, "events");
}

/// Wells: the level's static wells (Gravity Chaos 100; present once A1's
/// generator places them), dynamic wells that start and end mid-flight next
/// to each ship (one telegraphed 2 s ahead like the Architect's stroke, one
/// starting at once like a landing Paperweight), and the level's up turned
/// at once under the pilots (harsher than Bored level's 5 s turn: the
/// clients learn it a round trip late): the reconciliation error stays
/// under 0.05 u.
#[test]
fn prediction_holds_through_wells_and_a_turned_up() {
    let mut sliders = Sliders::new(40, 30);
    sliders.gravity_chaos = 100;
    let code = ShareCode::new(0x6772_6176, sliders);
    let mut h = Harness::new(MatchConfig::paint_the_void(code));
    h.connect("alpha", 3, LinkConditions::default(), flyer(0));
    h.connect("bravo", 5, LinkConditions::default(), flyer(1));
    h.connect("charlie", 4, lossy(), flyer(2));
    h.run(150);
    let statics = h.room.world().well_field().statics.len();
    eprintln!("static wells on the level: {statics}");
    let mut placed = 0;
    for round in 0..6u64 {
        let now = h.room.tick();
        let players: Vec<_> = h.room.clients().iter().map(|c| c.player).collect();
        for (i, p) in players.into_iter().enumerate() {
            let Some(ship) = h.room.world().ship(p) else {
                continue;
            };
            let (pos, ahead) = (ship.pos, ship.orientation.forward() * 6.0 + ship.vel * 0.5);
            let (start, len) = if (round + i as u64) % 2 == 0 {
                (now + 120, 240) // telegraphed (Architect stroke)
            } else {
                (now, 180) // at once (Paperweight landing)
            };
            h.room.world_mut().add_well(ls_game::ls_sim::DynamicWell {
                key: (round * 8 + i as u64) as u32,
                pos: pos + ahead,
                peak_accel: ls_game::ls_net::well_state::quantize_peak(9.0),
                range: ls_game::ls_net::well_state::quantize_range(16.0),
                start_tick: start,
                end_tick: start + len,
                owner: Some(p),
            });
            placed += 1;
        }
        if round == 3 {
            // Bored level: the level's up turns under the pilots.
            h.room.world_mut().set_up(Vec3::new(0.0, 0.6, 0.8));
        }
        h.run(200);
    }
    assert_eq!(placed, 18);
    h.run(200);
    for c in &h.clients {
        let s = c.session.as_ref().unwrap();
        assert!((s.mood().up - Vec3::new(0.0, 0.6, 0.8)).length() < 1e-6);
    }
    check_errors(&h, "wells");
}

/// Paperweights (M4 B7): every pilot throws one every 100 ticks while
/// flying the pattern, so stones land mid-air and on rock around the ships
/// and their wells pull the thrower and everyone near (replicated as well
/// entities on the wire grid): the reconciliation error stays under the
/// bound and the clients see the server's wells.
#[test]
fn prediction_holds_near_thrown_paperweights() {
    use ls_game::ls_sim::Availability;
    let code = ShareCode::new(0x7061_7065, Sliders::new(40, 30));
    let mut cfg = MatchConfig::paint_the_void(code);
    cfg.rules
        .weapons
        .insert("paperweight".into(), Availability::LoadoutAndPickup);
    cfg.rules.starting_loadout.secondaries = vec![("paperweight".into(), 3)];
    let mut h = Harness::new(cfg);
    let thrower = |seed: u64| -> Script {
        let fly = flyer(seed);
        Box::new(move |now, s| {
            let mut cmd = fly(now, s);
            if (now + seed * 33) % 100 == 0 {
                cmd.buttons.set(Buttons::FIRE_SECONDARY, true);
            }
            cmd
        })
    };
    h.connect("alpha", 3, LinkConditions::default(), thrower(0));
    h.connect("bravo", 5, LinkConditions::default(), thrower(1));
    h.connect("charlie", 4, lossy(), thrower(2));
    let mut most = 0;
    let mut seen_on_clients = 0;
    for _ in 0..12 {
        h.run(100);
        let wells = h.room.world().dynamic_wells().len();
        most = most.max(wells);
        for c in &h.clients {
            let s = c.session.as_ref().unwrap();
            seen_on_clients = seen_on_clients.max(s.wells().dynamics.len());
        }
    }
    eprintln!("paperweight wells at once: server {most}, clients {seen_on_clients}");
    assert!(most >= 2, "paperweights landed: {most}");
    assert!(seen_on_clients >= 2, "clients see the wells");
    check_errors(&h, "paperweights");
}
