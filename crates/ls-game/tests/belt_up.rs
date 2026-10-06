//! M4 B6 acceptance (`docs/milestone-4-plan.md` §5 wave B) on generated
//! asteroid belt levels:
//!
//! - fairness: one recorded racing line flown alone from each of the 8
//!   start slots (the same ship-local inputs, so the line is rotated with
//!   the slot) finishes within 1 % of the same time;
//! - an in-process netplay test: 8 clients race a heat of Belt Up through a
//!   `Room` over `MemoryTransport` links with latency (one lossy), without a
//!   decode error, with every client's reconciliation error under 0.1 u
//!   (drifting boulders are predicted exactly) except right after a bump,
//!   and back under 0.1 u within 0.2 s of every bump.
//!
//! Generated levels and long runs: release only (`cargo test -p ls-game
//! --release --test belt_up`; part of CI).

#![cfg(all(feature = "server", not(target_arch = "wasm32")))]

use std::collections::{BTreeMap, VecDeque};

use ls_game::client::ClientSession;
use ls_game::ls_math::level::{Archetype, BELT_WEDGES};
use ls_game::ls_math::Vec3;
use ls_game::ls_net::{
    check_welcome, Channel, LinkConditions, MemoryTransport, Message, Transport,
};
use ls_game::ls_sim::modes::belt_up::pilot::{steer, steer_ship, PilotStyle, PilotView};
use ls_game::ls_sim::modes::belt_up::{heat_phase, LAP_GATES};
use ls_game::ls_sim::{InputCmd, ModeKind, PlayerId, RuleSet, ShipTuning, ShipType, WeaponTable};
use ls_game::ls_worldgen::{Generator, ShareCode, Sliders};
use ls_game::server::{Room, RoomConfig};
use ls_game::{build_match, MatchConfig};

fn belt_code(seed: u64) -> ShareCode {
    let sliders = Sliders {
        absurdity: 0,
        instability: 0,
        gravity_chaos: 30,
        ..Sliders::default()
    };
    ShareCode::with_archetype(seed, sliders, Archetype::Belt)
}

fn belt_up_config(code: ShareCode) -> MatchConfig {
    let mut cfg = MatchConfig::paint_the_void(code);
    cfg.rules = RuleSet::default_for(ModeKind::BeltUp);
    cfg
}

/// The generator's clear corridor: the racing line of the autopilots.
fn corridor(code: &ShareCode) -> Vec<Vec3> {
    Generator::from_code(code)
        .unwrap()
        .belt_field()
        .expect("a belt level")
        .corridor()
        .to_vec()
}

/// Flies one heat alone from slot `slot` with `inputs` (recorded from slot
/// 0) or, without them, with the autopilot (recording its inputs). Returns
/// the ticks from the lights going out to the finish and the inputs.
fn fly_alone(
    code: ShareCode,
    slot: usize,
    inputs: Option<&[InputCmd]>,
    line: &[Vec3],
) -> (Option<u64>, Vec<InputCmd>) {
    let (mut w, mut t) = build_match(&belt_up_config(code)).unwrap();
    let id = w.add_player(None, ShipType::Tinker).unwrap();
    let style = PilotStyle {
        line: line.to_vec(),
        boost: false,
        ..PilotStyle::default()
    };
    let mut rec = Vec::new();
    for n in 0..(200 * 60u64) {
        let tick = w.tick() + 1;
        let cmd = match inputs {
            Some(list) => list
                .get(n as usize)
                .copied()
                .unwrap_or(InputCmd::idle(tick)),
            None => steer(&w, id, &style, tick),
        };
        rec.push(cmd);
        w.step(&mut t, &[(id, InputCmd { tick, ..cmd })]);
        if w.tick() == 1 {
            // Move the racer to `slot` while the lights hold it.
            let b = w.setup().belt.clone().unwrap();
            let pose = b.slots[slot];
            let race = w.mode_mut().race.as_mut().unwrap();
            let x = race.racers.get_mut(&id).unwrap();
            x.slot = slot as u8;
            x.last_pos = pose.pos;
            let s = w.ship_mut(id).unwrap();
            s.pos = pose.pos;
            s.orientation = pose.orientation;
            s.vel = Vec3::ZERO;
        }
        let race = w.mode().race.as_ref().unwrap();
        let x = race.racers[&id];
        if x.finish_tick > 0 {
            return (Some(x.finish_tick - race.start_tick), rec);
        }
        if race.phase != heat_phase::LIGHTS && race.phase != heat_phase::RACING {
            break;
        }
    }
    (None, rec)
}

#[test]
#[cfg_attr(debug_assertions, ignore = "generated levels: run with --release")]
fn one_line_from_every_slot_finishes_within_one_percent() {
    for seed in [0xB017_0001u64, 0xB017_0002] {
        let code = belt_code(seed);
        let line = corridor(&code);
        let (base, inputs) = fly_alone(code, 0, None, &line);
        let base = base.unwrap_or_else(|| panic!("{code}: the autopilot finishes from slot 0"));
        let mut times = Vec::new();
        for k in 0..BELT_WEDGES {
            let (time, _) = fly_alone(code, k, Some(&inputs), &line);
            let time = time.unwrap_or_else(|| panic!("{code}: slot {k} did not finish"));
            times.push(time);
        }
        eprintln!("{code}: autopilot {base} ticks; recorded line per slot {times:?}");
        assert_eq!(times[0], base, "{code}: the recording replays exactly");
        let lo = *times.iter().min().unwrap() as f64;
        let hi = *times.iter().max().unwrap() as f64;
        assert!(
            hi <= lo * 1.01,
            "{code}: slot times differ by {:.2} %: {times:?}",
            (hi / lo - 1.0) * 100.0
        );
    }
}

// ----- netplay ---------------------------------------------------------------

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
    /// `(snapshot tick, position error)` of every reconciliation after the
    /// session synchronized.
    errors: Vec<(u64, f32)>,
}

struct Harness {
    room: Room,
    clients: Vec<SimClient>,
    now: u64,
    /// Server ticks at which a ship (by sim player) was bumped.
    bumps: BTreeMap<PlayerId, Vec<u64>>,
    /// Ticks a ship touched a boulder (pushed), by sim player.
    boulder_contacts: usize,
}

impl Harness {
    fn new(cfg: MatchConfig) -> Self {
        Harness {
            room: Room::new(cfg, RoomConfig::default()).unwrap(),
            clients: Vec::new(),
            now: 0,
            bumps: BTreeMap::new(),
            boulder_contacts: 0,
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
            errors: Vec::new(),
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
        let bumps_before: Vec<_> = self
            .room
            .world()
            .mode()
            .race
            .as_ref()
            .map_or(Vec::new(), |r| r.bump_ready.clone());
        let report = self.room.step();
        if let Some(r) = &self.room.world().mode().race {
            for (a, b, t) in &r.bump_ready {
                if !bumps_before.contains(&(*a, *b, *t)) {
                    self.bumps.entry(*a).or_default().push(report.tick);
                    self.bumps.entry(*b).or_default().push(report.tick);
                }
            }
        }
        if let Some(belt) = &self.room.world().setup().belt {
            for s in self.room.world().ships().filter(|s| s.alive) {
                let touching = ls_game::ls_sim::boulders::poses(belt, report.tick)
                    .any(|(c, r)| c.distance(s.pos) < r + 0.6);
                self.boulder_contacts += touching as usize;
            }
        }
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
                            let tick = s.latest_snapshot().map_or(0, |x| x.tick as u64);
                            c.errors.push((tick, after.last_error));
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

/// The autopilot on the client: the own ship as predicted, the belt and the
/// race view of the newest snapshot, the other ships as interpolated.
fn racer(line: Vec<Vec3>, throttle: f32, ram_range: f32) -> Script {
    let tuning = ShipTuning::tinker();
    Box::new(move |now, s| {
        let idle = InputCmd::idle(0);
        let (Some(ship), Some(belt)) = (s.local_ship(), s.belt()) else {
            return idle;
        };
        if !s.is_synced() {
            return idle;
        }
        let Some(race) = s.race() else {
            return idle;
        };
        let Some(me) = race.racer(PlayerId(s.player() as u16)) else {
            return idle;
        };
        let others: Vec<Vec3> = s.remote_ships().iter().map(|r| r.pos).collect();
        let style = PilotStyle {
            line: line.clone(),
            throttle,
            ram_range,
            boost: ram_range > 0.0,
            ..PilotStyle::default()
        };
        let view = PilotView {
            ship,
            tuning: &tuning,
            belt,
            gates: me.gates,
            slot: me.slot,
            others: &others,
        };
        steer_ship(&view, &style, now)
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

/// Reconciliation bound of the acceptance criterion.
const MAX_ERROR: f32 = 0.1;
/// "Back under 0.1 u within 0.2 s of a bump."
const BUMP_RECOVERY_TICKS: u64 = 12;

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "generated level, 8 clients: run with --release"
)]
fn eight_clients_race_a_heat_with_bumps_and_boulders() {
    let code = belt_code(0xB017_0003);
    let line = corridor(&code);
    let mut h = Harness::new(belt_up_config(code));
    for i in 0..8u64 {
        // Every other racer is slow; the ones behind them ram.
        let (throttle, ram) = if i % 2 == 0 { (0.45, 0.0) } else { (1.0, 30.0) };
        let link = if i == 5 {
            lossy()
        } else {
            LinkConditions::default()
        };
        h.connect(
            &format!("racer{i}"),
            2 + i % 4,
            link,
            racer(line.clone(), throttle, ram),
        );
    }
    h.run(60 * 60);
    let race = h.room.world().mode().race.clone().expect("a race");
    eprintln!(
        "heat {} phase {}: {} bumps, {} boulder contact ticks",
        race.heat,
        race.phase,
        h.bumps.values().map(Vec::len).sum::<usize>() / 2,
        h.boulder_contacts
    );
    assert_eq!(race.in_heat(), 8, "every client races");
    assert!(
        race.racers.values().any(|x| x.gates >= 2),
        "the racers pass gates: {race:?}"
    );
    assert!(!h.bumps.is_empty(), "bumps happened");
    for c in &h.clients {
        let s = c.session.as_ref().expect("joined");
        let p = s.prediction_stats();
        let player = h.room.player_of(c.id.unwrap()).unwrap();
        let bumps = h.bumps.get(&player).cloned().unwrap_or_default();
        let near_bump = |tick: u64| {
            bumps
                .iter()
                .any(|&b| tick >= b && tick < b + BUMP_RECOVERY_TICKS)
        };
        let worst =
            c.errors
                .iter()
                .filter(|(t, _)| !near_bump(*t))
                .fold(
                    (0.0f32, 0u64),
                    |m, (t, e)| if *e > m.0 { (*e, *t) } else { m },
                );
        let worst_bump = c
            .errors
            .iter()
            .filter(|(t, _)| near_bump(*t))
            .fold(0.0f32, |m, (_, e)| m.max(*e));
        eprintln!(
            "{}: {} bumps, worst error {:.4} u at tick {} (in a bump's first 0.2 s: {:.3} u), {p:?}",
            c.name,
            bumps.len(),
            worst.0,
            worst.1,
            worst_bump
        );
        assert!(p.accepted > 200, "{}: {p:?}", c.name);
        assert!(
            worst.0 < MAX_ERROR,
            "{}: reconciliation error {} u at tick {} (bumps at {bumps:?})",
            c.name,
            worst.0,
            worst.1
        );
        // The session sees the server's race.
        let mine = s.race().unwrap();
        assert_eq!(mine.heat, race.heat, "{}", c.name);
        let _ = LAP_GATES;
    }
}
