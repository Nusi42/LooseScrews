//! M4 B2 acceptance (`docs/milestone-4-plan.md` §5 wave B, B1-B3): each
//! visitor event (Tax drone, Pizza delivery, Customer survey, Mimic
//! pickups, Gossip wall) forced in a `Room` on a generated level with 4
//! `ClientSession`s over `MemoryTransport` with latency (one link lossy):
//! 0 desyncs (no packet errors, no edit gaps, no prediction snaps), every
//! client sees the event's props and ends on the server's mood view, and
//! the drones fly the level's network without entering rock.

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ls_game::client::ClientSession;
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::Vec3;
use ls_game::ls_net::{
    check_welcome, Channel, LinkConditions, MemoryTransport, Message, Transport,
};
use ls_game::ls_sim::{
    Buttons, EntityKind, EventKind, InputCmd, ReplicatedKind, ShipTuning, WeaponTable,
};
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
    max_error: f32,
    /// Ticks the prediction snapped.
    snaps: Vec<u64>,
    /// Entity kinds seen in the client's snapshots.
    kinds_seen: Vec<u8>,
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
            snaps: Vec::new(),
            kinds_seen: Vec::new(),
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
                        if s.is_synced() && reconciled {
                            c.max_error = c.max_error.max(after.last_error);
                        }
                        if after.snapped > before.snapped {
                            c.snaps.push(now);
                        }
                        for e in s.entities() {
                            if !c.kinds_seen.contains(&e.kind) {
                                c.kinds_seen.push(e.kind);
                            }
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

/// Cruises around slowly with horizon assist and fires now and then (the
/// drones and mimics get shot at).
fn flyer(seed: u64, quiet: Arc<AtomicBool>) -> Script {
    Box::new(move |now, s| {
        let mut cmd = InputCmd::idle(0);
        if s.local_ship().is_none() || !s.is_synced() {
            return cmd;
        }
        let phase = ((now + seed * 41) / 70) % 4;
        cmd.thrust = match phase {
            0 => Vec3::new(0.0, 0.0, -0.6),
            1 => Vec3::new(0.4, 0.1, -0.3),
            2 => Vec3::ZERO,
            _ => Vec3::new(-0.3, -0.2, 0.3),
        };
        cmd.look_yaw = if phase == 1 { 0.01 } else { -0.003 };
        cmd.buttons = Buttons::HORIZON_ASSIST
            | if (now + seed * 13) % 90 < 20 && !quiet.load(Ordering::Relaxed) {
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

/// Free space between a prop's sphere and the rock, from the contact
/// estimate ships collide with (the terrain's `sdf` is only a lower bound
/// in cavern fields; `contracts.md`, "Terrain distance").
fn contact_gap<T: TerrainQuery>(t: &T, p: Vec3, radius: f32) -> f32 {
    ls_game::ls_sim::terrain::sphere_contact_distance(t, p, radius + 1.0) - radius
}

fn wire_kind(kind: EventKind) -> u8 {
    match kind {
        EventKind::GossipWall => ReplicatedKind::GossipFace as u8,
        EventKind::MimicPickups => ReplicatedKind::Mimic as u8,
        _ => ReplicatedKind::Drone as u8,
    }
}

#[test]
fn visitor_events_replicate_without_desyncs() {
    let code = ShareCode::new(0x7669_7369, Sliders::new(45, 35));
    let mut h = Harness::new(MatchConfig::paint_the_void(code));
    let quiet = Arc::new(AtomicBool::new(false));
    h.connect(
        "alpha",
        3,
        LinkConditions::default(),
        flyer(0, quiet.clone()),
    );
    h.connect(
        "bravo",
        5,
        LinkConditions::default(),
        flyer(1, quiet.clone()),
    );
    h.connect("charlie", 4, lossy(), flyer(2, quiet.clone()));
    h.connect(
        "delta",
        6,
        LinkConditions::default(),
        flyer(3, quiet.clone()),
    );
    h.run_until_synced();
    let synced_at = h.now;
    let nav = h.room.world().nav_graph().clone();
    eprintln!(
        "nav graph: {} nodes, {} edges, {} entries, connected {}",
        nav.nodes().len(),
        nav.edge_count(),
        nav.entries().len(),
        nav.is_connected()
    );
    assert!(!nav.is_empty() && nav.is_connected());
    let players: Vec<_> = h.room.clients().iter().map(|c| c.player).collect();
    assert_eq!(players.len(), 4);
    for kind in [
        EventKind::TaxDrone,
        EventKind::PizzaDelivery,
        EventKind::CustomerSurvey,
        EventKind::MimicPickups,
        EventKind::GossipWall,
    ] {
        for c in &mut h.clients {
            c.kinds_seen.clear();
        }
        // A leader and a damaged ship for the targeted events.
        {
            let w = h.room.world_mut();
            let lead = players[0];
            let team = w.ship(lead).unwrap().team as usize;
            w.mode_mut().team_scores[team] += 3;
            w.mode_mut().players.get_mut(&lead).unwrap().score += 3;
            w.ship_mut(players[1]).unwrap().hull = 40.0;
        }
        let at = h.room.tick() + 10;
        h.room.world_mut().schedule_event(at, kind);
        let mut ran = 0;
        let mut worst_clearance = f32::INFINITY;
        for _ in 0..(40 * 60) {
            // Nobody dies (respawns would be explained snaps).
            for p in &players {
                let w = h.room.world_mut();
                if let Some(s) = w.ship_mut(*p) {
                    if s.alive && s.hull < 30.0 {
                        s.hull = 30.0;
                    }
                }
            }
            h.step();
            let w = h.room.world();
            if w.mood_view().event == Some(kind) {
                ran += 1;
            } else if ran > 0 {
                break;
            }
            for e in w.entities() {
                if matches!(
                    e.kind,
                    EntityKind::Prop(ReplicatedKind::Drone | ReplicatedKind::Mimic)
                ) && e.hp > 0.0
                {
                    let gap = contact_gap(h.room.terrain(), e.pos, e.radius);
                    worst_clearance = worst_clearance.min(gap);
                }
            }
        }
        eprintln!("{kind:?}: ran {ran} ticks, worst prop clearance {worst_clearance:.3} u");
        assert!(ran > 180, "{kind:?} ran");
        assert!(worst_clearance > -0.1, "{kind:?}: a prop in the rock");
        h.run(30);
        let server = h.room.net_mode_state().mood;
        for c in &h.clients {
            let s = c.session.as_ref().unwrap();
            assert_eq!(s.mood().event, server.event, "{}", c.name);
            assert!(
                c.kinds_seen.contains(&wire_kind(kind)),
                "{}: never saw the {kind:?} prop ({:?})",
                c.name,
                c.kinds_seen
            );
        }
    }
    // Every client's edit log ends equal to the server's once the shooting
    // stops (an edit gap would have failed `handle_packet` already).
    quiet.store(true, Ordering::Relaxed);
    h.run(180);
    for c in &h.clients {
        let s = c.session.as_ref().unwrap();
        assert_eq!(
            s.edit_count(),
            h.room.terrain().edit_count(),
            "{}: edit log",
            c.name
        );
    }
    for c in &h.clients {
        let s = c.session.as_ref().unwrap();
        let p = s.prediction_stats();
        eprintln!(
            "{}: max reconciliation error {:.4} u, {p:?}",
            c.name, c.max_error
        );
        eprintln!(
            "{}: snaps at {:?} (synced from {synced_at})",
            c.name, c.snaps
        );
        assert!(
            c.snaps.iter().all(|t| *t <= synced_at),
            "{}: desync (snap after sync)",
            c.name
        );
        assert!(p.accepted > 1000, "{}: {p:?}", c.name);
    }
}

impl Harness {
    fn run(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.step();
        }
    }

    fn run_until_synced(&mut self) {
        for _ in 0..600 {
            self.step();
            if self
                .clients
                .iter()
                .all(|c| c.session.as_ref().is_some_and(|s| s.is_synced()))
            {
                self.run(60);
                return;
            }
        }
        panic!("clients never synced");
    }
}

/// The drones on generated levels: a Tax drone entering from a boundary
/// cavern at least 40 u away reaches a leader parked at a pickup point
/// (pickup points reach the network) and touches it, and a Pizza drone
/// delivers to a damaged pilot holding still at another one, on every
/// shape and enclosure; no drone ever enters the rock. Release builds only
/// (32 levels).
#[cfg(not(debug_assertions))]
#[test]
fn drones_find_their_targets_on_generated_levels() {
    use ls_game::ls_sim::{CueKind, Event, PlayerId, ShipType};
    let mut report = Vec::new();
    let mut worst_secs = [0.0f32; 2];
    // Wall time of every step while a drone flies (shared machine: a
    // report, not an assertion).
    let mut step_us: Vec<u128> = Vec::new();
    let mut slow: Vec<String> = Vec::new();
    for seed in 0..8u64 {
        for (enclosure, density, verticality) in
            [(20u8, 30u8, 50u8), (50, 40, 50), (85, 50, 20), (50, 45, 90)]
        {
            let mut sliders = Sliders::new(enclosure, density);
            sliders.verticality = verticality;
            let code = ShareCode::new(0x6472_6f6e_0000 + seed * 7919, sliders);
            let cfg = MatchConfig::paint_the_void(code);
            let (mut w, mut t) = ls_game::build_match(&cfg).unwrap();
            let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
            let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
            let points = w.setup().pickup_points.clone();
            for (kind, slot) in [(EventKind::TaxDrone, 0usize), (EventKind::PizzaDelivery, 1)] {
                // The target parks at a pickup point; the other ship far away.
                let (target, other) = if kind == EventKind::TaxDrone {
                    (a, b)
                } else {
                    (b, a)
                };
                let spot = points[(seed as usize * 3 + slot * 5) % points.len()];
                let park = |w: &mut ls_game::ls_sim::World, p: PlayerId, pos: Vec3| {
                    let s = w.ship_mut(p).unwrap();
                    s.pos = pos;
                    s.vel = Vec3::ZERO;
                };
                park(&mut w, target, spot);
                if kind == EventKind::TaxDrone {
                    let team = w.ship(target).unwrap().team as usize;
                    w.mode_mut().team_scores[team] = 9;
                    w.mode_mut().players.get_mut(&target).unwrap().score = 9;
                } else {
                    w.ship_mut(target).unwrap().hull = 40.0;
                }
                w.schedule_event(w.tick() + 1, kind);
                let mut done = None;
                let mut entry_gap = 0.0f32;
                let mut worst = f32::INFINITY;
                for i in 0..(30 * 60) {
                    park(&mut w, target, spot);
                    let other_pos = w.ship(other).unwrap().pos;
                    park(&mut w, other, other_pos);
                    let t0 = std::time::Instant::now();
                    let stepped = w.step(&mut t, &[]);
                    if i >= 180 {
                        let us = t0.elapsed().as_micros();
                        if us > 2000 {
                            slow.push(format!("seed {seed} E{enclosure} V{verticality} {kind:?} tick {i}: {us} us"));
                        }
                        step_us.push(us);
                    }
                    for e in stepped {
                        if let Event::Cue(c) = e {
                            if c.kind == CueKind::EventOutcome
                                && (c.value >> 8) as u8 == kind.index()
                                && done.is_none()
                            {
                                done = Some(i);
                            }
                        }
                    }
                    for e in w.entities() {
                        if e.kind == EntityKind::Prop(ReplicatedKind::Drone) {
                            if i == 2 {
                                entry_gap = e.pos.distance(spot);
                            }
                            worst = worst.min(contact_gap(&t, e.pos, e.radius));
                        }
                    }
                    if done.is_some() {
                        break;
                    }
                }
                // Let the event finish before the next one.
                while w.mood_view().event.is_some() {
                    w.step(&mut t, &[]);
                }
                let secs = done.map(|i| (i as f32 - 180.0) / 60.0);
                report.push(format!(
                    "seed {seed} E{enclosure} D{density} V{verticality} {kind:?}: entry {entry_gap:.0} u, \
                     done after {secs:?} s, clearance {worst:.2}"
                ));
                let secs = secs.unwrap_or_else(|| panic!("{}", report.join("\n")));
                let k = (kind == EventKind::PizzaDelivery) as usize;
                worst_secs[k] = worst_secs[k].max(secs);
                assert!(worst > -0.1, "{}", report.join("\n"));
            }
        }
    }
    println!("{}", report.join("\n"));
    println!(
        "slowest tax touch {:.1} s, slowest pizza delivery {:.1} s",
        worst_secs[0], worst_secs[1]
    );
    println!("steps over 2 ms: {slow:?}");
    step_us.sort_unstable();
    let pct = |p: f64| step_us[((step_us.len() - 1) as f64 * p) as usize];
    println!(
        "step time with a drone flying: p50 {} us, p99 {} us, max {} us ({} steps)",
        pct(0.5),
        pct(0.99),
        step_us.last().unwrap(),
        step_us.len()
    );
}

/// Cost of the visitor events on a generated level with 8 ships flying
/// around the pickups: the step time while each event runs against the
/// same match without an event (a report; the shared machine makes wall
/// times noisy). Release builds only.
#[cfg(not(debug_assertions))]
#[test]
fn visitor_event_step_cost() {
    use ls_game::ls_sim::{PlayerId, ShipType};
    let code = ShareCode::new(0x636f_7374, Sliders::new(50, 45));
    let cfg = MatchConfig::paint_the_void(code);
    let (mut w, mut t) = ls_game::build_match(&cfg).unwrap();
    let ships: Vec<PlayerId> = (0..8)
        .map(|i| w.add_player(Some(i % 2), ShipType::Tinker).unwrap())
        .collect();
    let points = w.setup().pickup_points.clone();
    let lead = ships[0];
    w.mode_mut().team_scores[0] = 9;
    w.mode_mut().players.get_mut(&lead).unwrap().score = 9;
    // Every ship hovers near a pickup point (the mimics find prey).
    let mut run = |w: &mut ls_game::ls_sim::World, kind: Option<EventKind>| -> (u128, u128) {
        if let Some(k) = kind {
            w.schedule_event(w.tick() + 1, k);
        }
        let mut us = Vec::new();
        for i in 0..(20 * 60) {
            for (n, p) in ships.iter().enumerate() {
                let s = w.ship_mut(*p).unwrap();
                s.pos = points[n % points.len()] + Vec3::new(0.0, 3.0, 0.0);
                s.vel = Vec3::ZERO;
                s.hull = s.hull.max(40.0);
            }
            w.ship_mut(ships[3]).unwrap().hull = 40.0;
            let t0 = std::time::Instant::now();
            w.step(&mut t, &[]);
            if i >= 181 {
                us.push(t0.elapsed().as_micros());
            }
        }
        while w.mood_view().event.is_some() {
            w.step(&mut t, &[]);
        }
        us.sort_unstable();
        (us[us.len() / 2], us[us.len() * 99 / 100])
    };
    let base = run(&mut w, None);
    println!("no event: p50 {} us, p99 {} us", base.0, base.1);
    for kind in [
        EventKind::TaxDrone,
        EventKind::PizzaDelivery,
        EventKind::CustomerSurvey,
        EventKind::MimicPickups,
        EventKind::GossipWall,
    ] {
        let (p50, p99) = run(&mut w, Some(kind));
        println!("{kind:?}: p50 {p50} us, p99 {p99} us");
    }
}
