//! Headless end-to-end tests: a `Room` and several `ClientSession`s talking
//! through `ls_net::MemoryTransport` pairs with added latency (and loss for
//! some clients), all in one thread with a shared 60 Hz tick counter.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::collections::{BTreeMap, VecDeque};

use ls_game::client::ClientSession;
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Quat, Vec3};
use ls_game::ls_net::{
    check_welcome, Channel, Hello, LinkConditions, MemoryTransport, Message, Transport,
};
use ls_game::ls_sim::{Buttons, Event, InputCmd, PlayerId, ShipTuning, WeaponTable};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;

type Script = Box<dyn Fn(u64, u64, &ClientSession) -> InputCmd>;

/// One direction of a link: packets wait `delay` ticks, then go through the
/// memory transport (which applies loss / duplication / reordering).
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
    /// `(global tick, session start tick, session) -> controls`
    script: Script,
    start: u64,
    /// Ticks between receiving the `Welcome` and having the session (a slow
    /// level generation); packets arriving meanwhile wait in `backlog`.
    setup_delay: u64,
    pending_welcome: Option<(u64, ls_game::ls_net::Welcome)>,
    backlog: Vec<Vec<u8>>,
    /// Global tick at which the session was built.
    built_at: u64,
}

struct Harness {
    room: Room,
    clients: Vec<SimClient>,
    now: u64,
    /// Server truth: `(client id, tick) -> position`.
    truth: BTreeMap<(u8, u64), Vec3>,
    events: Vec<(u64, Event)>,
}

impl Harness {
    fn new(code: ShareCode, cfg: RoomConfig) -> Self {
        Harness {
            room: Room::new(MatchConfig::paint_the_void(code), cfg).unwrap(),
            clients: Vec::new(),
            now: 0,
            truth: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    fn connect(&mut self, name: &str, delay: u64, link: LinkConditions, script: Script) -> usize {
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
            start: 0,
            setup_delay: 0,
            pending_welcome: None,
            backlog: Vec::new(),
            built_at: 0,
        };
        let hello = Message::Hello(Hello::new(name, None)).encode().unwrap();
        c.up.push(self.now, Channel::Reliable, hello);
        self.clients.push(c);
        self.clients.len() - 1
    }

    fn step(&mut self) {
        self.now += 1;
        let now = self.now;
        // 1. Client -> server.
        for c in &mut self.clients {
            c.up.flush(now, &mut c.client_end);
            while let Some(p) = c.server_end.recv().unwrap() {
                match c.id {
                    None => c.id = Some(self.room.join_packet(&p.data).expect("join accepted")),
                    Some(id) => self.room.handle_packet(id, &p.data).unwrap(),
                }
            }
        }
        // 2. Simulate.
        let report = self.room.step();
        for e in report.events {
            self.events.push((now, e));
        }
        for info in self.room.clients() {
            let s = self.room.world().ship(info.player).unwrap();
            self.truth.insert((info.id, report.tick), s.pos);
        }
        // 3. Server -> client.
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
                    None if c.pending_welcome.is_none() => {
                        let w = check_welcome(Message::decode(&p.data).unwrap()).unwrap();
                        c.pending_welcome = Some((now + c.setup_delay, w));
                    }
                    None => c.backlog.push(p.data),
                    Some(s) => {
                        s.handle_packet(&p.data)
                            .unwrap_or_else(|e| panic!("{}: bad packet: {e}", c.name));
                    }
                }
            }
            if c.pending_welcome
                .as_ref()
                .is_some_and(|(ready, _)| *ready <= now)
            {
                let (_, w) = c.pending_welcome.take().unwrap();
                let mut s =
                    ClientSession::from_welcome(&w, WeaponTable::builtin(), ShipTuning::tinker())
                        .unwrap();
                for p in std::mem::take(&mut c.backlog) {
                    s.handle_packet(&p)
                        .unwrap_or_else(|e| panic!("{}: bad packet: {e}", c.name));
                }
                c.start = s.next_input_tick();
                c.built_at = now;
                c.session = Some(s);
            }
            // 4. Client frame: inputs for the ticks due now.
            if let Some(s) = &mut c.session {
                let controls = (c.script)(now, c.start, s);
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
        self.clients[i].session.as_ref().expect("joined")
    }

    fn player(&self, i: usize) -> PlayerId {
        self.room.player_of(self.clients[i].id.unwrap()).unwrap()
    }
}

fn perfect() -> LinkConditions {
    LinkConditions::default()
}

/// Turns the nose down, approaches the terrain below and fires rockets and
/// paint at it for a while, then flies a pattern without firing.
fn gunner(paint_thrower: u8) -> Script {
    Box::new(move |now, start, s| {
        let mut cmd = InputCmd::idle(0);
        let Some(ship) = s.local_ship().filter(|_| s.is_synced()) else {
            return cmd;
        };
        let elapsed = now.saturating_sub(start);
        if elapsed < 240 {
            let fwd = ship.orientation.forward();
            cmd.weapon_select = paint_thrower + 1;
            if fwd.y > -0.9 {
                cmd.look_pitch = -0.04;
            } else {
                let dist = s
                    .terrain()
                    .raycast(ship.pos, fwd, 100.0)
                    .map_or(100.0, |h| h.distance);
                if dist > 12.0 {
                    cmd.thrust = Vec3::new(0.0, 0.0, -1.0);
                } else {
                    cmd.buttons = Buttons::FIRE_PRIMARY | Buttons::FIRE_SECONDARY;
                }
            }
        } else {
            let phase = (elapsed / 45) % 4;
            cmd.thrust = match phase {
                0 => Vec3::new(0.0, 0.0, 1.0),
                1 => Vec3::new(0.7, 0.0, -0.4),
                2 => Vec3::new(-0.7, -0.5, 0.0),
                _ => Vec3::ZERO,
            };
            cmd.look_yaw = if phase == 1 { 0.01 } else { -0.004 };
            cmd.look_pitch = if phase == 2 { 0.006 } else { 0.0 };
            cmd.buttons = if phase == 0 {
                Buttons::BOOST
            } else {
                Buttons::NONE
            };
        }
        cmd
    })
}

#[test]
fn prediction_converges_snapshots_decode_and_terrain_replicates() {
    let code = ShareCode::new(0x5eed_1234, Sliders::new(40, 30));
    let mut h = Harness::new(code, RoomConfig::default());
    let paint = WeaponTable::builtin().find("paint_thrower").unwrap().0;
    let lossy = LinkConditions {
        loss: 0.1,
        reorder: 0.05,
        duplicate: 0.05,
        seed: 9,
        ..LinkConditions::default()
    };
    h.connect("alpha", 3, perfect(), gunner(paint));
    h.connect("bravo", 5, perfect(), gunner(paint));
    h.connect("charlie", 4, lossy, gunner(paint));
    h.run(240);
    // A late joiner replays the edit log from its Welcome.
    h.connect("delta", 2, perfect(), gunner(paint));
    h.run(300);
    let before: Vec<_> = (0..4).map(|i| h.session(i).prediction_stats()).collect();
    h.run(90);

    let server_edits = h.room.terrain().edit_count();
    assert!(
        server_edits > 0,
        "rockets and paint must have edited the terrain"
    );
    let log = h.room.terrain().edit_log();
    assert!(log.iter().any(|e| e.changes_geometry()), "no crater");
    assert!(log.iter().any(|e| !e.changes_geometry()), "no paint");
    let server_hash = h.room.terrain().state_hash();

    for (i, c) in h.clients.iter().enumerate() {
        let s = h.session(i);
        let stats = s.stats();
        // Snapshots decode (full first, then deltas) at ~30 Hz.
        assert!(stats.snapshots > 100, "{}: {stats:?}", c.name);
        assert!(
            stats.delta_snapshots > stats.full_snapshots,
            "{}: {stats:?}",
            c.name
        );
        // Terrain edits replicated in order: identical terrain.
        assert_eq!(s.edit_count(), server_edits, "{}", c.name);
        assert_eq!(s.terrain().state_hash(), server_hash, "{}", c.name);

        // Prediction for the newest snapshot tick matches the server's exact
        // state, and the render offset has decayed.
        let tick = s.latest_snapshot().unwrap().tick as u64;
        let predicted = s
            .predictor()
            .state_at(tick)
            .expect("state for snapshot tick");
        let truth = h.truth[&(c.id.unwrap(), tick)];
        let err = predicted.pos.distance(truth);
        assert!(err < 0.02, "{}: error {err} at tick {tick}", c.name);
        assert!(s.predictor().pos_offset().length() < 0.01, "{}", c.name);
        let local = s.local_ship().unwrap().pos;
        let current = h.room.world().ship(h.player(i)).unwrap().pos;
        assert!(local.is_finite() && current.is_finite());

        let p = s.prediction_stats();
        eprintln!("{}: {:?} / {:?}", c.name, stats, p);
        if c.name != "charlie" {
            // No persistent error: over the last 1.5 s of flying, every
            // server state matched the prediction.
            assert_eq!(p.corrected, before[i].corrected, "{}: {p:?}", c.name);
            assert_eq!(p.snapped, before[i].snapped, "{}: {p:?}", c.name);
            assert!(p.accepted > before[i].accepted + 30, "{}: {p:?}", c.name);
        }
    }

    // Remote ships are interpolated ~100 ms behind the server.
    let s = h.session(0);
    let rt = s.render_tick();
    let remote = s.remote_ships();
    assert_eq!(remote.len(), 3);
    for r in remote {
        let t = rt.floor() as u64;
        let truth = h.truth[&(r.id, t)];
        assert!(
            r.pos.distance(truth) < 1.0,
            "remote {} off by {}",
            r.id,
            r.pos.distance(truth)
        );
    }
    let st = h.room.stats();
    eprintln!("room: {st:?}");
    assert!(st.missing_inputs < 200, "{st:?}");
}

/// A client whose level generation takes 4 s (240 ticks) after the
/// `Welcome` is far behind the server when it starts; it catches up and
/// synchronizes within a few round trips instead of one 30-tick step per
/// round trip (and pongs from before the catch-up do not push it further).
#[test]
fn slow_setup_client_syncs_within_a_few_round_trips() {
    let code = ShareCode::new(0x5eed_1234, Sliders::new(40, 30));
    let mut h = Harness::new(code, RoomConfig::default());
    let delay = 3;
    let i = h.connect(
        "slow",
        delay,
        perfect(),
        Box::new(|_, _, _| InputCmd::idle(0)),
    );
    h.clients[i].setup_delay = 240;
    while h.clients[i].session.is_none() {
        h.step();
    }
    let built = h.clients[i].built_at;
    let rtt = 2 * delay;
    let mut synced_at = None;
    for _ in 0..(8 * rtt) {
        h.step();
        if h.session(i).is_synced() {
            synced_at = Some(h.now);
            break;
        }
    }
    let synced_at = synced_at.expect("synchronized");
    assert!(
        synced_at - built <= 3 * rtt,
        "synced {} ticks after the session was built",
        synced_at - built
    );
}

/// Finds a shooter/target pair of positions 12 u apart with a clear line of
/// fire and open space above the target.
fn duel_positions(h: &Harness) -> (Vec3, Vec3, Vec3) {
    let t = h.room.terrain();
    let b = t.bounds();
    for i in 0..64 {
        let f = i as f32 / 64.0;
        let guess = b.min + (b.max - b.min).mul_elem(Vec3::new(0.2 + 0.6 * f, 0.5, 0.8 - 0.6 * f));
        let Some(c) = t.find_open_point(guess, 6.0, 12) else {
            continue;
        };
        for axis in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
            let shooter = c - axis * 6.0;
            let target = c + axis * 6.0;
            let clear = t.raycast(shooter, axis, 13.0).is_none();
            let open = |p: Vec3| t.sdf(p) > 2.5 && b.expand(-3.0).contains(p);
            if clear && open(shooter) && open(target) && open(target + Vec3::Y * 3.0) {
                return (shooter, target, axis);
            }
        }
    }
    panic!("no duel positions found");
}

fn duel(lag_compensation: bool) -> (bool, u64) {
    let code = ShareCode::new(4242, Sliders::new(10, 10));
    let cfg = RoomConfig {
        lag_compensation,
        ..RoomConfig::default()
    };
    let mut h = Harness::new(code, cfg);
    const FIRE_AT: u64 = 200;
    // Shooter presses fire for one tick; the target starts climbing (boost)
    // at the same moment, so by the time the shot arrives it has moved out
    // of the line of fire on the server.
    h.connect(
        "shooter",
        5,
        perfect(),
        Box::new(|now, _, _| {
            let mut c = InputCmd::idle(0);
            if now == FIRE_AT {
                c.buttons = Buttons::FIRE_PRIMARY;
            }
            c
        }),
    );
    h.connect(
        "target",
        5,
        perfect(),
        Box::new(|now, _, _| {
            let mut c = InputCmd::idle(0);
            if now >= FIRE_AT {
                c.thrust = Vec3::new(0.0, 1.0, 0.0);
                c.buttons = Buttons::BOOST;
            }
            c
        }),
    );
    h.run(60);
    let (shooter_pos, target_pos, axis) = duel_positions(&h);
    let (s, t) = (h.player(0), h.player(1));
    {
        let w = h.room.world_mut();
        let sh = w.ship_mut(s).unwrap();
        sh.pos = shooter_pos;
        sh.vel = Vec3::ZERO;
        sh.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, axis);
        let tg = w.ship_mut(t).unwrap();
        tg.pos = target_pos;
        tg.vel = Vec3::ZERO;
        tg.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, -axis);
    }
    h.run(FIRE_AT + 60 - h.now);
    let hit = h.events.iter().any(|(_, e)| {
        matches!(e, Event::Damaged { target, attacker, direct: true, .. }
            if *target == t && *attacker == s)
    });
    (hit, h.room.stats().compensated_hits)
}

#[test]
fn lag_compensation_hits_target_that_moved_after_the_shot() {
    // Both runs in parallel (each builds a full level).
    let without = std::thread::spawn(|| duel(false));
    let (hit_with, comp_with) = duel(true);
    let (hit_without, comp_without) = without.join().unwrap();
    assert!(
        !hit_without,
        "without compensation the target dodges on the server"
    );
    assert_eq!(comp_without, 0);
    assert!(hit_with, "the rewound target is hit");
    assert!(comp_with >= 1);
}
