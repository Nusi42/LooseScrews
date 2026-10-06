//! M4 B3 acceptance (`docs/milestone-4-plan.md` §5 wave B): each of the
//! spectacle events (Duck parade, Hat rain, Cat paw, Disco inferno, Tired
//! announcer) runs in an in-process netplay test with 4 clients (a `Room`
//! and `ClientSession`s over `MemoryTransport` with latency, one link
//! lossy) and 0 desyncs: no bad packet, no prediction snap that no death
//! explains, every client on the server's terrain (Cat paw's grooves) and
//! the server's hat counts. Duck parade pushes keep the predictor's
//! reconciliation error under 0.1 u.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use ls_game::client::ClientSession;
use ls_game::ls_math::{Quat, Vec3};
use ls_game::ls_net::{
    check_welcome, Channel, LinkConditions, MemoryTransport, Message, NetEvent, Transport,
};
use ls_game::ls_sim::{Buttons, CueKind, EventKind, InputCmd, ShipTuning, WeaponTable};
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
    /// Largest position error at a snapshot tick after the session
    /// synchronized (and outside the windows `excuse` marks), and where.
    max_error: f32,
    max_error_tick: u64,
    /// Snapshot ticks of prediction snaps (judged against `life_ticks`).
    snaps: Vec<u64>,
    /// Ticks the own ship died or respawned (the events' own ticks).
    life_ticks: Vec<u64>,
    /// Cues received, by kind.
    cues: BTreeMap<u8, u32>,
}

struct Harness {
    room: Room,
    clients: Vec<SimClient>,
    now: u64,
    /// Server ticks after which a knockback (Cat paw) may cost a correction
    /// above the movement bound.
    excuse_until: u64,
}

/// A death or respawn this close to a snap explains it (ticks; events and
/// snapshots arrive on different channels).
const LIFE_WINDOW: u64 = 60;

fn near_life(life: &[u64], tick: u64) -> bool {
    life.iter().any(|t| t.abs_diff(tick) <= LIFE_WINDOW)
}

impl Harness {
    fn new(cfg: MatchConfig) -> Self {
        Harness {
            room: Room::new(cfg, RoomConfig::default()).unwrap(),
            clients: Vec::new(),
            now: 0,
            excuse_until: 0,
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
            snaps: Vec::new(),
            life_ticks: Vec::new(),
            cues: BTreeMap::new(),
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
        let server_tick = self.room.tick();
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
                        let me = s.player();
                        for e in s.take_events() {
                            match e {
                                NetEvent::Kill { victim, tick, .. } if victim == me => {
                                    c.life_ticks.push(tick as u64)
                                }
                                NetEvent::Respawn { ship, tick, .. } if ship == me => {
                                    c.life_ticks.push(tick as u64)
                                }
                                NetEvent::Cue { kind, .. } => {
                                    *c.cues.entry(kind).or_default() += 1;
                                }
                                _ => {}
                            }
                        }
                        let after = s.prediction_stats();
                        let synced = s.is_synced();
                        let snap_tick = s.latest_snapshot().map_or(server_tick, |x| x.tick as u64);
                        if synced && after.snapped > before.snapped {
                            c.snaps.push(snap_tick);
                        }
                        let reconciled =
                            after.accepted + after.corrected > before.accepted + before.corrected;
                        let excused =
                            server_tick <= self.excuse_until || near_life(&c.life_ticks, snap_tick);
                        if synced && reconciled && !excused && after.last_error > c.max_error {
                            c.max_error = after.last_error;
                            c.max_error_tick = server_tick;
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

    /// Steps until the server's mood view shows `kind` in `phase` (index).
    fn run_until(&mut self, kind: EventKind, phase: u8) {
        for _ in 0..3000 {
            let m = self.room.net_mode_state().mood;
            if m.event == kind.index() && m.phase == phase {
                return;
            }
            self.step();
        }
        panic!("{kind:?} never reached phase {phase}");
    }

    /// Steps until no event runs any more.
    fn run_until_over(&mut self) {
        for _ in 0..4000 {
            if self.room.world().mood_view().event.is_none() {
                return;
            }
            self.step();
        }
        panic!("the event never ended");
    }
}

/// Reconciliation bound of the acceptance criterion (movement events).
const MAX_ERROR: f32 = 0.1;

fn lossy() -> LinkConditions {
    LinkConditions {
        loss: 0.05,
        reorder: 0.05,
        duplicate: 0.02,
        seed: 7,
        ..LinkConditions::default()
    }
}

/// Flies toward the target in `slot` (if set), braking near it.
fn seeker(targets: Rc<RefCell<Vec<Option<Vec3>>>>, slot: usize, fire: Buttons) -> Script {
    Box::new(move |_now, s| {
        let mut cmd = InputCmd::idle(0);
        let Some(me) = s.local_ship().filter(|_| s.is_synced()) else {
            return cmd;
        };
        let Some(target) = targets.borrow().get(slot).copied().flatten() else {
            return cmd;
        };
        let local = me.orientation.conjugate().rotate(target - me.pos);
        let vel = me.orientation.conjugate().rotate(me.vel);
        let want = local * 1.5 - vel * 0.4;
        cmd.thrust = Vec3::new(
            want.x.clamp(-1.0, 1.0),
            want.y.clamp(-1.0, 1.0),
            want.z.clamp(-1.0, 1.0),
        );
        cmd.buttons = Buttons::HORIZON_ASSIST | fire;
        cmd
    })
}

/// Hovers, turning to aim at the nearest other ship and firing its
/// primary when `armed` is set.
fn gunner(armed: Rc<RefCell<bool>>) -> Script {
    Box::new(move |_now, s| {
        let mut cmd = InputCmd::idle(0);
        let Some(me) = s.local_ship().filter(|_| s.is_synced()) else {
            return cmd;
        };
        let target = s.remote_ships().into_iter().map(|r| r.pos).min_by(|a, b| {
            a.distance_squared(me.pos)
                .total_cmp(&b.distance_squared(me.pos))
        });
        let vel = me.orientation.conjugate().rotate(me.vel);
        cmd.thrust = Vec3::new(
            (-vel.x * 0.6).clamp(-1.0, 1.0),
            (-vel.y * 0.6).clamp(-1.0, 1.0),
            (-vel.z * 0.6).clamp(-1.0, 1.0),
        );
        if let Some(t) = target {
            let local = me.orientation.conjugate().rotate(t - me.pos);
            cmd.look_pitch = local.y.atan2(-local.z).clamp(-0.1, 0.1);
            cmd.look_yaw = (-local.x).atan2(-local.z).clamp(-0.1, 0.1);
            if *armed.borrow() && local.z < 0.0 && local.x.abs() + local.y.abs() < 1.0 {
                cmd.buttons = Buttons::FIRE_PRIMARY;
            }
        }
        cmd
    })
}

fn code() -> ShareCode {
    ShareCode::new(0x5bec_7ac1, Sliders::new(40, 30))
}

/// Four clients (one lossy link) with the given scripts, synchronized.
fn harness(scripts: Vec<Script>) -> Harness {
    let mut h = Harness::new(MatchConfig::paint_the_void(code()));
    for (i, script) in scripts.into_iter().enumerate() {
        let link = if i == 3 {
            lossy()
        } else {
            LinkConditions::default()
        };
        h.connect(&format!("pilot{i}"), 3 + i as u64, link, script);
    }
    h.run(150);
    for c in &h.clients {
        assert!(
            c.session.as_ref().is_some_and(|s| s.is_synced()),
            "{}",
            c.name
        );
    }
    h
}

/// Every client ended where the server is: same terrain (edits and SDF),
/// the server's hat counts on every ship, no unexplained snap.
fn check_in_sync(h: &mut Harness, what: &str) {
    // Let the last edits and snapshots arrive: every client on the
    // server's edit count for 60 ticks in a row.
    let mut calm = 0;
    for _ in 0..1500 {
        h.step();
        let n = h.room.terrain().edit_count();
        let level = h
            .clients
            .iter()
            .all(|c| c.session.as_ref().unwrap().terrain().edit_count() == n);
        calm = if level { calm + 1 } else { 0 };
        if calm >= 60 {
            break;
        }
    }
    assert!(
        calm >= 60,
        "{what}: the clients never caught up with the edits"
    );
    let server_terrain = h.room.terrain().state_hash();
    for c in &h.clients {
        let s = c.session.as_ref().unwrap();
        assert_eq!(
            s.terrain().state_hash(),
            server_terrain,
            "{what}: {} terrain differs (edits {} vs {}, sdf {} vs {}, paint {} vs {})",
            c.name,
            s.terrain().edit_count(),
            h.room.terrain().edit_count(),
            s.terrain().sdf_hash(4.0),
            h.room.terrain().sdf_hash(4.0),
            s.terrain().paint_hash(),
            h.room.terrain().paint_hash(),
        );
        let unexplained: Vec<u64> = c
            .snaps
            .iter()
            .copied()
            .filter(|t| !near_life(&c.life_ticks, *t))
            .collect();
        assert!(
            unexplained.is_empty(),
            "{what}: {} snapped at {unexplained:?} (deaths/respawns {:?})",
            c.name,
            c.life_ticks
        );
        let p = s.prediction_stats();
        assert!(p.accepted > 100, "{what}: {} {p:?}", c.name);
        // Hats on the wire (protocol 6 ext bits).
        let snap = s.latest_snapshot().expect("snapshot");
        for ship in &snap.ships {
            let player = h
                .room
                .clients()
                .iter()
                .find(|ci| ci.id == ship.id)
                .map(|ci| ci.player)
                .unwrap();
            let hats = h.room.world().ship(player).map_or(0, |x| x.hats);
            assert_eq!(
                ls_game::ls_net::ship_ext::hats(ship.ext),
                hats.min(31),
                "{what}: {} sees ship {} with the server's hats",
                c.name,
                ship.id
            );
        }
        eprintln!(
            "{what}: {} max error {:.5} u at {} ({p:?}), cues {:?}",
            c.name, c.max_error, c.max_error_tick, c.cues
        );
    }
}

/// Duck parade: the pilots park on the parade's route ahead of the
/// convoy and hold their spot; the ducks shove them aside. The predictor
/// knows every duck exactly, so the reconciliation error stays under
/// 0.1 u through the pushes.
#[test]
fn duck_parade_pushes_stay_predicted() {
    let targets = Rc::new(RefCell::new(vec![None; 4]));
    let scripts = (0..4)
        .map(|i| seeker(Rc::clone(&targets), i, Buttons::NONE))
        .collect();
    let mut h = harness(scripts);
    let setup = h.room.world().setup().clone();
    assert!(!setup.routes.is_empty(), "the level has parade routes");
    let start = h.room.tick() + 10;
    h.room
        .world_mut()
        .schedule_event(start, EventKind::DuckParade);
    h.run_until(EventKind::DuckParade, 0);
    let view = h.room.world().mood_view();
    let route = ls_game::ls_sim::events::duck_parade::route(&setup, &view.params).unwrap();
    let convoy =
        ls_game::ls_sim::events::duck_parade::Convoy::new(EventKind::DuckParade.def(), &route);
    let lead = convoy.arc(0, 0.0);
    // Parking spots on the route ahead of the convoy (reached during the
    // 3 s announcement and the first seconds), one per pilot, a little
    // off the centre line so the ducks push them sideways.
    let spots: Vec<Vec3> = (0..4)
        .map(|i| {
            let (p, d) = route.at(lead + 18.0 + 12.0 * i as f32);
            let side = d.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
            p + side * (if i % 2 == 0 { 1.0 } else { -1.0 })
        })
        .collect();
    // The pilots fly there first (a server-side teleport would snap them).
    for (i, p) in spots.iter().enumerate() {
        targets.borrow_mut()[i] = Some(*p);
    }
    let seed = ls_game::setup::match_seed(&code());
    let mut contacts = 0;
    let mut obstacles = Vec::new();
    let players: Vec<_> = h.room.clients().iter().map(|c| c.player).collect();
    while h.room.world().mood_view().event.is_some() {
        h.step();
        let now = h.room.tick();
        let v = h.room.world().mood_view();
        ls_game::ls_sim::director::obstacles(&v, seed, now, &setup, &mut obstacles);
        for p in &players {
            let Some(s) = h.room.world().ship(*p) else {
                continue;
            };
            if obstacles
                .iter()
                .any(|o| s.pos.distance(o.pos) < o.radius + 0.5 + 0.3)
            {
                contacts += 1;
            }
        }
    }
    eprintln!("duck contacts (ship-ticks): {contacts}");
    assert!(contacts > 20, "the ducks pushed the pilots: {contacts}");
    check_in_sync(&mut h, "duck parade");
    for c in &h.clients {
        assert!(
            c.max_error < MAX_ERROR,
            "{}: reconciliation error {} at tick {}",
            c.name,
            c.max_error,
            c.max_error_tick
        );
    }
}

/// Hat rain: two pilots hunt hats (the nearest hat entity they see), the
/// others hover; the server's hat counts reach every client in the ships'
/// ext bits, and the Hat Baron lands in the results.
#[test]
fn hat_rain_hats_reach_every_client() {
    let targets = Rc::new(RefCell::new(vec![None; 4]));
    let hunt = |slot: usize, t: Rc<RefCell<Vec<Option<Vec3>>>>| -> Script {
        let inner = seeker(Rc::clone(&t), slot, Buttons::NONE);
        Box::new(move |now, s| {
            let hat = ls_game::ls_sim::ReplicatedKind::Hat as u8;
            let me = s.local_ship().map(|m| m.pos);
            let nearest = s
                .entities()
                .into_iter()
                .filter(|e| e.kind == hat)
                .map(|e| e.pos)
                .min_by(|a, b| {
                    let m = me.unwrap_or_default();
                    a.distance_squared(m).total_cmp(&b.distance_squared(m))
                });
            t.borrow_mut()[slot] = nearest;
            inner(now, s)
        })
    };
    let scripts: Vec<Script> = vec![
        hunt(0, Rc::clone(&targets)),
        hunt(1, Rc::clone(&targets)),
        seeker(Rc::clone(&targets), 2, Buttons::NONE),
        seeker(Rc::clone(&targets), 3, Buttons::NONE),
    ];
    let mut h = harness(scripts);
    let start = h.room.tick() + 10;
    h.room.world_mut().schedule_event(start, EventKind::HatRain);
    h.run_until(EventKind::HatRain, 1);
    let mut most = 0;
    while h.room.world().mood_view().event.is_some() {
        h.step();
        most = most.max(h.room.world().ships().map(|s| s.hats).max().unwrap_or(0));
    }
    eprintln!("most hats on one ship: {most}");
    assert!(most >= 1, "the hunters wore hats");
    check_in_sync(&mut h, "hat rain");
    let awards = h.room.current_results().awards;
    assert_eq!(awards.len(), 1, "a Hat Baron: {awards:?}");
    assert_eq!(awards[0].event, EventKind::HatRain.index());
}

/// Cat paw: the pilots gather near a wall; the paw hits them (30 damage,
/// 25 u/s knockback: a server-side correction like any knockback) and
/// carves its grooves, which every client applies.
#[test]
fn cat_paw_grooves_reach_every_client() {
    let targets = Rc::new(RefCell::new(vec![None; 4]));
    let scripts = (0..4)
        .map(|i| seeker(Rc::clone(&targets), i, Buttons::NONE))
        .collect();
    let mut h = harness(scripts);
    // Gather by the nearest spawn of pilot 0 (spawns are near the level's
    // x ends, so a wall is within the paw's reach).
    let p0 = h.room.clients()[0].player;
    let base = h.room.world().ship(p0).unwrap().pos;
    for i in 0..4 {
        targets.borrow_mut()[i] = Some(base + Vec3::new(0.0, 0.0, 3.0 * i as f32 - 4.5));
    }
    h.run(240);
    let edits_before = h.room.terrain().edit_count();
    let players: Vec<_> = h.room.clients().iter().map(|c| c.player).collect();
    let hull = |h: &Harness| -> Vec<f32> {
        players
            .iter()
            .map(|p| h.room.world().ship(*p).unwrap().hull)
            .collect()
    };
    let before = hull(&h);
    let start = h.room.tick() + 10;
    h.room.world_mut().schedule_event(start, EventKind::CatPaw);
    h.run_until(EventKind::CatPaw, 1);
    h.excuse_until = h.room.tick() + 3 * 60 + 60;
    h.run_until_over();
    let grooves = h.room.terrain().edit_count() - edits_before;
    let after = hull(&h);
    let hit = before
        .iter()
        .zip(&after)
        .filter(|(b, a)| (*b - *a - 30.0).abs() < 0.5)
        .count();
    eprintln!("cat paw grooves: {grooves}, pilots hit: {hit} ({before:?} -> {after:?})");
    assert!(hit >= 2, "the paw swiped the group");
    check_in_sync(&mut h, "cat paw");
}

/// Disco inferno: nothing but a mirror ball; the clients stay in sync.
#[test]
fn disco_inferno_stays_in_sync() {
    let targets = Rc::new(RefCell::new(vec![None; 4]));
    let scripts = (0..4)
        .map(|i| seeker(Rc::clone(&targets), i, Buttons::NONE))
        .collect();
    let mut h = harness(scripts);
    let start = h.room.tick() + 10;
    h.room
        .world_mut()
        .schedule_event(start, EventKind::DiscoInferno);
    h.run_until(EventKind::DiscoInferno, 1);
    let mut ball_seen = 0;
    while h.room.world().mood_view().event.is_some() {
        h.step();
        let ball = ls_game::ls_sim::ReplicatedKind::DiscoBall as u8;
        if h.clients.iter().all(|c| {
            c.session
                .as_ref()
                .unwrap()
                .entities()
                .iter()
                .any(|e| e.kind == ball)
        }) {
            ball_seen += 1;
        }
    }
    assert!(ball_seen > 600, "every client sees the ball: {ball_seen}");
    check_in_sync(&mut h, "disco inferno");
    for c in &h.clients {
        assert!(c.max_error < MAX_ERROR, "{}: {}", c.name, c.max_error);
    }
}

/// Tired announcer: a gunner shoots the others (set to 1 hull by the
/// server) during the event; every client gets the same kill-cam cues.
#[test]
fn tired_announcer_kill_cams_reach_every_client() {
    let armed = Rc::new(RefCell::new(false));
    let targets = Rc::new(RefCell::new(vec![None; 4]));
    let scripts: Vec<Script> = vec![
        gunner(Rc::clone(&armed)),
        seeker(Rc::clone(&targets), 1, Buttons::NONE),
        seeker(Rc::clone(&targets), 2, Buttons::NONE),
        seeker(Rc::clone(&targets), 3, Buttons::NONE),
    ];
    let mut h = harness(scripts);
    // The others hover a few units in front of the gunner.
    let gun = h.room.clients()[0].player;
    let g = h.room.world().ship(gun).unwrap();
    let ahead = g.orientation.rotate(Vec3::new(0.0, 0.0, -7.0));
    let (gpos, grot) = (g.pos, g.orientation);
    for i in 1..4 {
        let side = grot.rotate(Vec3::new(2.0 * (i as f32 - 2.0), 0.0, 0.0));
        targets.borrow_mut()[i] = Some(gpos + ahead + side);
    }
    h.run(300);
    let start = h.room.tick() + 10;
    h.room
        .world_mut()
        .schedule_event(start, EventKind::TiredAnnouncer);
    h.run_until(EventKind::TiredAnnouncer, 1);
    *armed.borrow_mut() = true;
    let others: Vec<_> = h.room.clients()[1..].iter().map(|c| c.player).collect();
    let mut kills = 0;
    while h.room.world().mood_view().event.is_some() {
        for p in &others {
            if let Some(s) = h.room.world_mut().ship_mut(*p) {
                if s.alive && s.hull > 1.0 {
                    s.hull = 1.0;
                }
            }
        }
        let deaths: u32 = others
            .iter()
            .map(|p| h.room.world().ship(*p).map_or(0, |s| s.deaths))
            .sum();
        h.step();
        let after: u32 = others
            .iter()
            .map(|p| h.room.world().ship(*p).map_or(0, |s| s.deaths))
            .sum();
        kills += after - deaths;
    }
    *armed.borrow_mut() = false;
    eprintln!("kills during the event: {kills}");
    assert!(kills >= 1, "the gunner scored");
    check_in_sync(&mut h, "tired announcer");
    let cams: Vec<u32> = h
        .clients
        .iter()
        .map(|c| *c.cues.get(&(CueKind::KillCam as u8)).unwrap_or(&0))
        .collect();
    assert!(cams[0] >= 1, "kill-cams: {cams:?}");
    assert!(
        cams.iter().all(|n| *n == cams[0]),
        "same on every client: {cams:?}"
    );
    let _ = Quat::IDENTITY;
}
