//! A room's simulation thread: owns the [`Room`], runs it at a fixed 60 Hz,
//! polls every client's [`SimLink`], hands outgoing packets back and runs
//! the room's [`Lobby`] and playlist [`Rotation`].
//!
//! The room is built on a thread of its own (its terrain is `Send` but not
//! `Sync`), moved here and never leaves; other threads reach it through
//! [`SimCommand`]s. Every room of a server (the public one and each private
//! one, see [`crate::rooms`]) has its own sim thread.
//!
//! Policies:
//!
//! - **Joins**: a full room (or a bad version) gets `Reject` and the link
//!   closes. Joins are served by the current room in every lobby phase, so a
//!   join never waits for a match build.
//! - **Match rotation**: see [`crate::lobby`]. The next room (warm-up or
//!   match) is built on a worker thread (`ls-build`) while the current one
//!   keeps running and swapped in when its phase ends. On a swap every
//!   connected player joins the new room with its original `Hello` and gets
//!   the new room's `Welcome` on the same connection (no `Disconnect`, no
//!   reconnect); the old room is dropped on another thread. A swap does not
//!   stall the tick, so the scheduler keeps its rhythm.
//! - **Private rooms** ([`SimParams::close_when_empty`]): the thread ends
//!   once the last player has left, or if nobody joined within
//!   [`crate::rooms::PRIVATE_ROOM_IDLE`].
//! - **Kicks** (`Disconnect` with a reason, counted in `kicked`): more than
//!   [`MAX_PROTOCOL_ERRORS`] undecodable or unexpected packets, or a reliable
//!   send queue over `LinkConfig::max_reliable_bytes` (client too slow).
//! - **Oversized unreliable packets** (over the transport's datagram limit;
//!   WebTransport in wave 4) are sent on the reliable channel instead.
//! - **Desync warning**: a stats window in which the server had to repeat
//!   client commands for more than `10 + 2 %` of the player ticks.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ls_game::server::{Room, RoomConfig, RoomStats};
use ls_game::MatchConfig;
use ls_math::terrain::TerrainEdit;
use ls_math::TICK_HZ;
use ls_net::{Channel, Hello, Message, Reject, RejectReason, Transport, TransportError};
use ls_sim::RuleSet;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

use crate::link::SimLink;
use crate::lobby::{self, warmup_rules, Action, Lobby, LobbyInput, LobbyTiming, Phase};
use crate::playlist::{Playlist, Rotation};
use crate::rooms::{RoomStatus, SharedStatus, PRIVATE_ROOM_IDLE};
use crate::rules::describe;
use crate::stats::{inc, set, SharedStats};
use crate::timing::{wait_until, TickScheduler, TickTimes, TICK_PERIOD};

/// Packets handled per client per tick (the rest wait for the next tick).
const MAX_PACKETS_PER_TICK: usize = 64;
/// Undecodable or unexpected packets tolerated before a client is kicked.
pub const MAX_PROTOCOL_ERRORS: u32 = 32;
/// Tick time budget from the plan (p99), per room.
const TICK_BUDGET_US: u64 = 4_000;
/// Default time players see a finished match before the next one starts.
pub const DEFAULT_INTERMISSION: Duration = Duration::from_secs(10);
/// `Disconnect` reason for players a closing private room still had queued.
pub const ROOM_CLOSED_REASON: &str = "room closed";

/// A request for the sim thread, processed at the start of the next tick.
pub enum SimCommand {
    /// A connection passed the version check; add its player.
    Join { hello: Hello, link: SimLink },
    /// Runs a closure against the room (admin, tests).
    Inspect(Box<dyn FnOnce(&mut Room) + Send>),
}

impl std::fmt::Debug for SimCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SimCommand::Join { hello, link } => f
                .debug_struct("Join")
                .field("name", &hello.name)
                .field("conn", &link.info().id)
                .finish(),
            SimCommand::Inspect(_) => f.write_str("Inspect"),
        }
    }
}

/// What the first match was built with (reported to the starter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomInfo {
    pub share_code: String,
    pub ruleset: String,
    /// Rule set code of the effective rules (empty if not encodable).
    pub rules_code: String,
}

/// Sim thread settings.
#[derive(Debug, Clone)]
pub struct SimParams {
    /// Weapon, ship and gadget tables of every match; world and rules come
    /// from the playlist entry.
    pub template: MatchConfig,
    pub room_cfg: RoomConfig,
    /// How often tick statistics are logged.
    pub stats_interval: Duration,
    /// Warm-up, countdown and results times.
    pub lobby: LobbyTiming,
    /// The rotation this room plays.
    pub playlist: Playlist,
    /// Room name in logs and status ([`crate::rooms::PUBLIC_ROOM`] or a
    /// room code).
    pub label: String,
    /// Private rooms end once empty (see the module docs).
    pub close_when_empty: bool,
    /// Extra time every worker build takes (tests of slow builds).
    pub build_delay: Duration,
}

/// Why a client is removed (for the log and the counters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leave {
    /// The client sent `Disconnect`.
    Disconnected,
    /// The socket closed or failed.
    Closed,
    /// Too many protocol errors: kick with a reason.
    ProtocolErrors,
    /// The reliable queue overflowed (link already closed): counted as kick.
    TooSlow,
    /// The next room would not take the player.
    NoRoom,
}

impl Leave {
    fn text(self) -> &'static str {
        match self {
            Leave::Disconnected => "client disconnected",
            Leave::Closed => "connection closed",
            Leave::ProtocolErrors => "too many protocol errors",
            Leave::TooSlow => "send buffer overflow (client too slow)",
            Leave::NoRoom => "the next match did not accept the player",
        }
    }
}

struct ClientConn {
    link: SimLink,
    /// The connection's `Hello`, replayed into every new room.
    hello: Hello,
    errors: u32,
}

/// A warm-up room (weapons off) or a match room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomKind {
    Warmup,
    Match,
}

/// The playlist entry a room belongs to and its match configuration (one
/// world for its warm-up and its match).
#[derive(Debug, Clone)]
struct Planned {
    index: usize,
    cfg: MatchConfig,
    /// Picked as a fallback: played whatever the player count.
    fallback: bool,
    share_code: String,
    rules_code: String,
}

impl Planned {
    fn new(index: usize, cfg: MatchConfig, fallback: bool) -> Planned {
        Planned {
            index,
            share_code: cfg.code.encode().unwrap_or_default(),
            rules_code: cfg.rules.to_code().unwrap_or_default(),
            cfg,
            fallback,
        }
    }

    /// The configuration of this entry's `kind` room.
    fn room_config(&self, kind: RoomKind) -> MatchConfig {
        let mut cfg = self.cfg.clone();
        if kind == RoomKind::Warmup {
            cfg.rules = warmup_rules(&cfg.rules);
        }
        cfg
    }
}

enum BuildState {
    Building {
        rx: mpsc::Receiver<Result<Room, String>>,
        started: Instant,
    },
    Built(Box<Room>),
}

/// The room being built (or built) to replace the current one.
struct NextRoom {
    kind: RoomKind,
    planned: Planned,
    state: BuildState,
}

impl NextRoom {
    fn built(&self, kind: RoomKind) -> bool {
        self.kind == kind && matches!(self.state, BuildState::Built(_))
    }
}

/// Builds a room on a worker thread.
fn build_room(cfg: MatchConfig, room_cfg: RoomConfig, delay: Duration) -> BuildState {
    let (tx, rx) = mpsc::channel();
    let fail = tx.clone();
    let spawned = std::thread::Builder::new()
        .name("ls-build".into())
        .spawn(move || {
            let room = Room::new(cfg, room_cfg).map_err(|e| e.to_string());
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let _ = tx.send(room);
        });
    if let Err(e) = spawned {
        let _ = fail.send(Err(format!("cannot start a build thread: {e}")));
    }
    BuildState::Building {
        rx,
        started: Instant::now(),
    }
}

struct Sim {
    params: SimParams,
    room: Room,
    kind: RoomKind,
    current: Planned,
    next: Option<NextRoom>,
    rotation: Rotation,
    lobby: Lobby,
    /// Effective rules of each playlist entry's last match (kept when the
    /// balance list fails to load).
    last_rules: BTreeMap<usize, RuleSet>,
    /// Wire ids of players that are ready (lobby).
    ready: BTreeSet<u8>,
    /// Sim-thread ticks since the start (the lobby's clock; runs while the
    /// room is empty too).
    clock: u64,
    had_players: bool,
    closing: bool,
    status: SharedStatus,
    /// Phase and player count last published to `status`.
    published: Option<(&'static str, usize)>,
    clients: BTreeMap<u8, ClientConn>,
    commands: mpsc::Receiver<SimCommand>,
    shutdown: Arc<AtomicBool>,
    stats: Arc<SharedStats>,
    times: TickTimes,
    window_room: RoomStats,
    window_skipped: u64,
    /// Most networked debris pieces at the end of a tick in this stats window.
    window_debris_peak: usize,
    /// Longest time (us) of each tick phase in this stats window: commands
    /// and receive, `Room::step`, dispatch (send) and the rest.
    window_phase_max: [u64; 3],
    window_dropped: u64,
    /// Counters of the rooms replaced so far, so the shared totals keep
    /// growing across matches.
    past_matches: RoomStats,
    past_ticks: u64,
    past_edits: u64,
}

/// Starts a room's sim thread. `ready` receives the first match's info once
/// its room is built (or the error if it cannot be).
pub fn spawn(
    params: SimParams,
    commands: mpsc::Receiver<SimCommand>,
    shutdown: Arc<AtomicBool>,
    stats: Arc<SharedStats>,
    status: SharedStatus,
    ready: oneshot::Sender<Result<RoomInfo, String>>,
) -> std::io::Result<JoinHandle<()>> {
    let name = if params.close_when_empty {
        format!("ls-sim-{}", params.label)
    } else {
        "ls-sim".into()
    };
    std::thread::Builder::new().name(name).spawn(move || {
        let started = Instant::now();
        let mut rotation = Rotation::new(params.playlist.clone());
        let index = rotation.pick(0);
        let entry = rotation.entry(index);
        let cfg = match entry.match_config(&params.template) {
            Ok(c) => c,
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        let planned = Planned::new(index, cfg, false);
        let kind = if params.lobby.is_off() {
            RoomKind::Match
        } else {
            RoomKind::Warmup
        };
        let room = match Room::new(planned.room_config(kind), params.room_cfg) {
            Ok(r) => r,
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        let info = RoomInfo {
            share_code: planned.share_code.clone(),
            ruleset: planned.cfg.rules.name.clone(),
            rules_code: planned.rules_code.clone(),
        };
        info!(
            room = %params.label,
            entry = %entry.source,
            share_code = %info.share_code,
            ruleset = %info.ruleset,
            warmup = kind == RoomKind::Warmup,
            build_ms = started.elapsed().as_millis() as u64,
            "match ready"
        );
        info!(
            room = %params.label,
            "rules: {}",
            describe(&planned.cfg.rules, &planned.cfg.weapons)
        );
        let mut sim = Sim::new(
            params, room, kind, planned, rotation, commands, shutdown, stats, status,
        );
        if ready.send(Ok(info)).is_err() {
            return;
        }
        sim.run();
    })
}

impl Sim {
    #[allow(clippy::too_many_arguments)]
    fn new(
        params: SimParams,
        room: Room,
        kind: RoomKind,
        current: Planned,
        rotation: Rotation,
        commands: mpsc::Receiver<SimCommand>,
        shutdown: Arc<AtomicBool>,
        stats: Arc<SharedStats>,
        status: SharedStatus,
    ) -> Sim {
        if kind == RoomKind::Match {
            inc(&stats.matches);
        }
        let mut last_rules = BTreeMap::new();
        last_rules.insert(current.index, current.cfg.rules.clone());
        let mut sim = Sim {
            lobby: Lobby::new(params.lobby),
            params,
            room,
            kind,
            current,
            next: None,
            rotation,
            last_rules,
            ready: BTreeSet::new(),
            clock: 0,
            had_players: false,
            closing: false,
            status,
            published: None,
            clients: BTreeMap::new(),
            commands,
            shutdown,
            stats,
            times: TickTimes::default(),
            window_room: RoomStats::default(),
            window_skipped: 0,
            window_debris_peak: 0,
            window_phase_max: [0; 3],
            window_dropped: 0,
            past_matches: RoomStats::default(),
            past_ticks: 0,
            past_edits: 0,
        };
        if kind == RoomKind::Warmup {
            sim.build_next(RoomKind::Match, sim.current.clone());
        }
        sim.publish_status(true);
        sim
    }

    fn run(&mut self) {
        let mut sched = TickScheduler::sixty_hz(Instant::now());
        let mut last_report = Instant::now();
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                // Final (partial) window, so short runs report everything.
                self.report();
                self.disconnect_all("server shutting down");
                break;
            }
            if self.closing {
                self.report();
                self.close();
                break;
            }
            let now = Instant::now();
            let due = sched.due(now);
            if due.skipped > 0 {
                warn!(
                    room = %self.params.label,
                    skipped = due.skipped,
                    "sim fell behind; skipped ticks to catch up"
                );
                self.window_skipped += due.skipped;
                self.stats
                    .skipped_ticks
                    .fetch_add(due.skipped, Ordering::Relaxed);
            }
            for _ in 0..due.run {
                let t0 = Instant::now();
                self.tick();
                self.times.record(t0.elapsed());
            }
            if now.duration_since(last_report) >= self.params.stats_interval {
                last_report = now;
                self.report();
            }
            if due.run == 0 {
                // Never sleep past one tick, so shutdown stays responsive.
                wait_until(sched.next_deadline().min(Instant::now() + TICK_PERIOD));
            }
        }
    }

    /// One 60 Hz tick: commands, received packets, step, outgoing packets,
    /// then the lobby (which may swap in the next room).
    fn tick(&mut self) {
        let t0 = Instant::now();
        self.clock += 1;
        self.process_commands();
        self.receive();
        let t1 = Instant::now();
        // The match clock only runs while someone plays.
        if self.room.player_count() > 0 {
            self.room.step();
            let debris = self.debris_count();
            self.window_debris_peak = self.window_debris_peak.max(debris);
        }
        let t2 = Instant::now();
        self.dispatch();
        self.poll_next();
        self.update_lobby();
        self.check_close();
        self.publish_stats();
        self.publish_status(false);
        let t3 = Instant::now();
        for (m, d) in self
            .window_phase_max
            .iter_mut()
            .zip([t1 - t0, t2 - t1, t3 - t2])
        {
            *m = (*m).max(d.as_micros() as u64);
        }
    }

    /// Networked debris pieces now (capped at `ls_sim::entity::MAX_DEBRIS`).
    fn debris_count(&self) -> usize {
        self.room
            .world()
            .entities()
            .iter()
            .filter(|e| e.kind == ls_sim::EntityKind::Debris)
            .count()
    }

    fn publish_stats(&self) {
        let rs = self.room.stats();
        let past = self.past_matches;
        let s = &self.stats;
        set(&s.tick, self.room.tick());
        set(&s.total_ticks, self.past_ticks + self.room.tick());
        set(&s.players, self.clients.len() as u64);
        let edits = self.room.terrain().edit_count() as u64;
        set(&s.edit_count, edits);
        set(&s.total_edits, self.past_edits + edits);
        set(&s.late_inputs, past.late_inputs + rs.late_inputs);
        set(&s.missing_inputs, past.missing_inputs + rs.missing_inputs);
        set(&s.snapshots_sent, past.snapshots_sent + rs.snapshots_sent);
        set(&s.snapshot_bytes, past.snapshot_bytes + rs.snapshot_bytes);
    }

    /// Writes the room status when the phase or player count changed (or
    /// `force`, after a swap).
    fn publish_status(&mut self, force: bool) {
        let key = (self.lobby.phase().name(), self.clients.len());
        if !force && self.published == Some(key) {
            return;
        }
        self.published = Some(key);
        let entry = self.rotation.entry(self.current.index);
        let status = RoomStatus {
            label: self.params.label.clone(),
            phase: key.0,
            entry: entry.source.clone(),
            share_code: self.current.share_code.clone(),
            ruleset: self.current.cfg.rules.name.clone(),
            rules_code: self.current.rules_code.clone(),
            players: key.1,
            max_players: self.params.room_cfg.max_players,
            closed: self.closing,
        };
        if let Ok(mut s) = self.status.write() {
            *s = status;
        }
    }

    fn process_commands(&mut self) {
        while let Ok(cmd) = self.commands.try_recv() {
            match cmd {
                SimCommand::Join { hello, link } => self.join(hello, link),
                SimCommand::Inspect(f) => f(&mut self.room),
            }
        }
    }

    fn join(&mut self, hello: Hello, mut link: SimLink) {
        let conn = link.info().id;
        match self.room.join(&hello) {
            Ok(id) => {
                let team = self
                    .room
                    .clients()
                    .into_iter()
                    .find(|c| c.id == id)
                    .map_or(0, |c| c.team);
                info!(
                    room = %self.params.label,
                    conn,
                    player = id,
                    team,
                    name = %hello.name,
                    remote = %link.info().remote,
                    transport = link.info().transport,
                    players = self.room.player_count(),
                    phase = self.lobby.phase().name(),
                    "player joined"
                );
                self.clients.insert(
                    id,
                    ClientConn {
                        link,
                        hello,
                        errors: 0,
                    },
                );
            }
            Err(reject) => {
                info!(room = %self.params.label, conn, name = %hello.name, reason = ?reject.reason, "join rejected");
                inc(&self.stats.rejected);
                if let Ok(bytes) = Message::Reject(reject).encode() {
                    let _ = link.send_reliable(&bytes);
                }
                link.close();
            }
        }
    }

    // ----- rotation ----------------------------------------------------------

    /// Starts building the `kind` room of `planned` on a worker thread
    /// (replacing any build in progress).
    fn build_next(&mut self, kind: RoomKind, planned: Planned) {
        let cfg = planned.room_config(kind);
        let state = build_room(cfg, self.params.room_cfg, self.params.build_delay);
        self.next = Some(NextRoom {
            kind,
            planned,
            state,
        });
    }

    /// Picks the next playlist entry for the current player count and its
    /// match configuration (a broken balance list keeps the entry's previous
    /// rules).
    fn plan_next(&mut self) -> Planned {
        let players = self.clients.len();
        let index = self.rotation.pick(players);
        let entry = self.rotation.entry(index).clone();
        let fallback = players > 0 && !entry.fits(players);
        let cfg = match entry.match_config(&self.params.template) {
            Ok(cfg) => cfg,
            Err(e) => {
                warn!(room = %self.params.label, error = %e, "could not reload the rules; keeping the previous ones");
                let mut cfg = self.params.template.clone();
                cfg.code = entry.world.share_code();
                cfg.rules = match self.last_rules.get(&index) {
                    Some(r) => r.clone(),
                    None => entry.rules.base.clone(),
                };
                cfg
            }
        };
        if self.last_rules.get(&index) != Some(&cfg.rules) {
            if let Some(prev) = self.last_rules.insert(index, cfg.rules.clone()) {
                info!(
                    room = %self.params.label,
                    previous = %prev.name,
                    "rules changed (balance list): {}",
                    describe(&cfg.rules, &cfg.weapons)
                );
            }
        }
        Planned::new(index, cfg, fallback)
    }

    /// Moves a finished build into place.
    fn poll_next(&mut self) {
        let Some(next) = &mut self.next else {
            return;
        };
        let BuildState::Building { rx, started } = &next.state else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(room)) => {
                debug!(
                    room = %self.params.label,
                    build_ms = started.elapsed().as_millis() as u64,
                    warmup = next.kind == RoomKind::Warmup,
                    "next room built"
                );
                next.state = BuildState::Built(Box::new(room));
            }
            Ok(Err(e)) => {
                warn!(room = %self.params.label, error = %e, "could not build the next room; trying again");
                let kind = next.kind;
                let planned = next.planned.clone();
                self.build_next(kind, planned);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                let kind = next.kind;
                let planned = next.planned.clone();
                self.build_next(kind, planned);
            }
        }
    }

    /// Takes the built next room.
    fn take_next(&mut self) -> Option<(RoomKind, Planned, Room)> {
        let next = self.next.take()?;
        match next.state {
            BuildState::Built(room) => Some((next.kind, next.planned, *room)),
            state => {
                self.next = Some(NextRoom { state, ..next });
                None
            }
        }
    }

    fn update_lobby(&mut self) {
        // A re-picked warm-up room is swapped in as soon as it is built.
        if self.lobby.in_warmup()
            && self
                .next
                .as_ref()
                .is_some_and(|n| n.built(RoomKind::Warmup))
        {
            if let Some((kind, planned, room)) = self.take_next() {
                self.swap_room(room, kind, planned);
                self.lobby.warmup_swapped(self.clock, self.clients.len());
                self.build_next(RoomKind::Match, self.current.clone());
            }
        }
        let players = self.clients.len();
        let fits = self.current.fallback || self.rotation.entry(self.current.index).fits(players);
        let next_ready = match self.lobby.phase() {
            Phase::Countdown { .. } => self.next.as_ref().is_some_and(|n| n.built(RoomKind::Match)),
            Phase::Results { .. } => self
                .next
                .as_ref()
                .is_some_and(|n| matches!(n.state, BuildState::Built(_))),
            _ => false,
        };
        let input = LobbyInput {
            now: self.clock,
            players,
            all_ready: players > 0 && self.clients.keys().all(|id| self.ready.contains(id)),
            fits,
            match_over: self.kind == RoomKind::Match && !self.room.world().mode().is_running(),
            next_ready,
        };
        let label = self.params.label.clone();
        match self.lobby.update(input) {
            Action::None => {}
            Action::CountdownStarted => {
                info!(room = %label, players, secs = self.lobby.timing().countdown.as_secs_f64(), "countdown started");
            }
            Action::CountdownCancelled => {
                info!(room = %label, players, "countdown cancelled");
            }
            Action::Repick => {
                let planned = self.plan_next();
                if planned.index == self.current.index {
                    // Nothing else fits better: play this entry anyway.
                    self.current.fallback = true;
                    self.lobby.start_countdown(self.clock);
                    info!(room = %label, players, "no playlist entry fits; countdown started anyway");
                } else {
                    info!(
                        room = %label,
                        players,
                        entry = %self.rotation.entry(planned.index).source,
                        "player count does not fit; warm-up on another playlist entry"
                    );
                    self.build_next(RoomKind::Warmup, planned);
                }
            }
            Action::StartMatch => {
                if let Some((kind, planned, room)) = self.take_next() {
                    self.swap_room(room, kind, planned);
                }
            }
            Action::MatchOver => {
                let winner = format!("{:?}", self.room.world().mode().phase);
                let planned = self.plan_next();
                info!(
                    room = %label,
                    players,
                    result = %winner,
                    next = %self.rotation.entry(planned.index).source,
                    next_share_code = %planned.share_code,
                    "match over; building the next room"
                );
                let kind = if self.lobby.timing().is_off() {
                    RoomKind::Match
                } else {
                    RoomKind::Warmup
                };
                self.build_next(kind, planned);
            }
            Action::NextRoom => {
                if let Some((kind, planned, room)) = self.take_next() {
                    self.swap_room(room, kind, planned);
                    if kind == RoomKind::Warmup {
                        self.build_next(RoomKind::Match, self.current.clone());
                    }
                }
            }
        }
    }

    /// Replaces the room: every client joins the new one with its `Hello`
    /// and gets its `Welcome` on the same connection.
    fn swap_room(&mut self, room: Room, kind: RoomKind, planned: Planned) {
        let old = std::mem::replace(&mut self.room, room);
        let os = old.stats();
        let p = &mut self.past_matches;
        p.late_inputs += os.late_inputs;
        p.missing_inputs += os.missing_inputs;
        p.snapshots_sent += os.snapshots_sent;
        p.snapshot_bytes += os.snapshot_bytes;
        p.encode_errors += os.encode_errors;
        self.past_ticks += old.tick();
        self.past_edits += old.terrain().edit_count() as u64;
        self.window_room = RoomStats::default();
        // Dropping a level frees a lot of memory; keep it off the tick.
        let _ = std::thread::Builder::new()
            .name("ls-drop".into())
            .spawn(move || drop(old));
        self.kind = kind;
        self.current = planned;
        self.ready.clear();
        let moved = std::mem::take(&mut self.clients);
        let count = moved.len();
        for (old_id, mut c) in moved {
            match self.room.join(&c.hello) {
                Ok(id) => {
                    debug!(room = %self.params.label, old_id, player = id, "player moved to the next room");
                    self.clients.insert(id, c);
                }
                Err(reject) => {
                    warn!(room = %self.params.label, name = %c.hello.name, reason = %reject.message, "player not accepted by the next room");
                    if let Ok(bytes) = (Message::Disconnect {
                        reason: format!("{}: {}", Leave::NoRoom.text(), reject.message),
                    })
                    .encode()
                    {
                        let _ = c.link.send_reliable(&bytes);
                    }
                    c.link.close();
                }
            }
        }
        if kind == RoomKind::Match {
            inc(&self.stats.matches);
        }
        let entry = self.rotation.entry(self.current.index);
        info!(
            room = %self.params.label,
            warmup = kind == RoomKind::Warmup,
            entry = %entry.source,
            share_code = %self.current.share_code,
            ruleset = %self.current.cfg.rules.name,
            rules_code = %self.current.rules_code,
            players = self.clients.len(),
            moved = count,
            matches = self.stats.matches.load(Ordering::Relaxed),
            "next room swapped in"
        );
        self.publish_status(true);
    }

    /// Private rooms: closes once empty (see the module docs).
    fn check_close(&mut self) {
        if !self.params.close_when_empty {
            return;
        }
        let players = self.clients.len();
        self.had_players |= players > 0;
        let idle = lobby::ticks(PRIVATE_ROOM_IDLE);
        if players == 0 && (self.had_players || self.clock >= idle) {
            self.closing = true;
        }
    }

    /// Ends a private room: queued joins get a `Reject`, the status says
    /// closed.
    fn close(&mut self) {
        while let Ok(cmd) = self.commands.try_recv() {
            if let SimCommand::Join { mut link, .. } = cmd {
                let reject = Reject {
                    reason: RejectReason::Other,
                    server_version: ls_net::PROTOCOL_VERSION,
                    message: ROOM_CLOSED_REASON.into(),
                };
                if let Ok(bytes) = Message::Reject(reject).encode() {
                    let _ = link.send_reliable(&bytes);
                }
                link.close();
            }
        }
        self.disconnect_all(ROOM_CLOSED_REASON);
        self.publish_status(true);
        info!(room = %self.params.label, "private room closed");
    }

    // ----- connections -------------------------------------------------------

    fn receive(&mut self) {
        let mut gone: Vec<(u8, Leave)> = Vec::new();
        for (&id, c) in self.clients.iter_mut() {
            for _ in 0..MAX_PACKETS_PER_TICK {
                match c.link.recv() {
                    Ok(Some(p)) => {
                        if let Err(e) = self.room.handle_packet(id, &p.data) {
                            c.errors += 1;
                            debug!(player = id, error = %e, "bad packet");
                            if c.errors > MAX_PROTOCOL_ERRORS {
                                gone.push((id, Leave::ProtocolErrors));
                                break;
                            }
                        }
                        // `Disconnect` removed the client from the room; the
                        // rest of its packets are moot.
                        if self.room.player_of(id).is_none() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        gone.push((id, Leave::Closed));
                        break;
                    }
                }
            }
        }
        // Clients that sent `Disconnect` were already removed from the room.
        for &id in self.clients.keys() {
            if self.room.player_of(id).is_none() && !gone.iter().any(|(g, _)| *g == id) {
                gone.push((id, Leave::Disconnected));
            }
        }
        for (id, why) in gone {
            if why == Leave::ProtocolErrors {
                self.kick(id, why);
            } else {
                self.remove(id, why);
            }
        }
    }

    fn dispatch(&mut self) {
        let mut failed: Vec<u8> = Vec::new();
        for o in self.room.take_outgoing() {
            let Some(c) = self.clients.get_mut(&o.client) else {
                continue;
            };
            let sent = match c.link.send(o.channel, &o.bytes) {
                // Larger than the transport's datagram limit: the reliable
                // stream takes any size.
                Err(TransportError::TooLarge { .. }) if o.channel == Channel::Unreliable => {
                    c.link.send_reliable(&o.bytes)
                }
                r => r,
            };
            if sent.is_err() && !failed.contains(&o.client) {
                failed.push(o.client);
            }
        }
        for id in failed {
            let slow = self.clients.get(&id).and_then(|c| c.link.close_reason())
                == Some(crate::link::CloseReason::Overflow);
            if slow {
                inc(&self.stats.kicked);
                self.remove(id, Leave::TooSlow);
            } else {
                self.remove(id, Leave::Closed);
            }
        }
    }

    fn send_disconnect(&mut self, id: u8, reason: &str) {
        if let Some(c) = self.clients.get_mut(&id) {
            if let Ok(bytes) = (Message::Disconnect {
                reason: reason.to_string(),
            })
            .encode()
            {
                let _ = c.link.send_reliable(&bytes);
            }
        }
    }

    /// Sends `Disconnect` with a reason, then removes the client.
    fn kick(&mut self, id: u8, why: Leave) {
        inc(&self.stats.kicked);
        self.send_disconnect(id, why.text());
        self.remove(id, why);
    }

    fn remove(&mut self, id: u8, why: Leave) {
        if let Some(mut c) = self.clients.remove(&id) {
            self.room.leave(id);
            c.link.close();
            info!(
                player = id,
                conn = c.link.info().id,
                reason = why.text(),
                players = self.room.player_count(),
                "player left"
            );
        }
    }

    fn disconnect_all(&mut self, reason: &str) {
        let ids: Vec<u8> = self.clients.keys().copied().collect();
        for id in ids {
            self.send_disconnect(id, reason);
            if let Some(mut c) = self.clients.remove(&id) {
                c.link.close();
            }
            self.room.leave(id);
        }
        set(&self.stats.players, 0);
        info!("sim thread stopped");
    }

    /// Logs tick timing (p50/p99/max) and input counters for the last window.
    fn report(&mut self) {
        let t = self.times.take_summary();
        let rs = self.room.stats();
        let w = self.window_room;
        self.window_room = rs;
        let dropped_total = self.stats.links.dropped_unreliable.load(Ordering::Relaxed);
        let dropped = dropped_total.saturating_sub(self.window_dropped);
        self.window_dropped = dropped_total;
        let skipped = std::mem::take(&mut self.window_skipped);
        set(&self.stats.last_p99_us, t.p99_us as u64);
        set(&self.stats.last_max_us, t.max_us as u64);

        let secs = self.params.stats_interval.as_secs_f64().max(1e-3);
        let players = self.clients.len() as u64;
        let missing = rs.missing_inputs.saturating_sub(w.missing_inputs);
        let late = rs.late_inputs.saturating_sub(w.late_inputs);
        let craters = self
            .room
            .terrain()
            .edit_log()
            .iter()
            .filter(|e| matches!(e, TerrainEdit::SubtractSphere { .. }))
            .count();
        let world = self.room.world();
        let mode = world.mode();
        let cov = mode.coverage;
        let match_left_s = mode.remaining_ticks(world.tick()) as f64 / f64::from(TICK_HZ);
        let match_over = !mode.is_running();
        let edits = self.room.terrain().edit_count();
        let debris = self.debris_count();
        let debris_peak = std::mem::take(&mut self.window_debris_peak).max(debris);
        let [recv_max_us, step_max_us, send_max_us] = std::mem::take(&mut self.window_phase_max);
        info!(
            room = %self.params.label,
            phase = self.lobby.phase().name(),
            warmup = self.kind == RoomKind::Warmup,
            tick = self.room.tick(),
            total_ticks = self.past_ticks + self.room.tick(),
            players,
            ticks = t.ticks,
            p50_us = t.p50_us,
            p99_us = t.p99_us,
            max_us = t.max_us,
            recv_max_us,
            step_max_us,
            send_max_us,
            over_budget = t.over_budget,
            skipped,
            late_inputs = late,
            missing_inputs = missing,
            duplicate_inputs = rs.duplicate_inputs.saturating_sub(w.duplicate_inputs),
            dropped_inputs = rs.dropped_inputs.saturating_sub(w.dropped_inputs),
            compensated_hits = rs.compensated_hits.saturating_sub(w.compensated_hits),
            encode_errors = rs.encode_errors.saturating_sub(w.encode_errors),
            snapshot_kib_s =
                (rs.snapshot_bytes.saturating_sub(w.snapshot_bytes)) as f64 / 1024.0 / secs,
            unreliable_dropped = dropped,
            edits,
            total_edits = self.past_edits + edits as u64,
            craters,
            debris,
            debris_peak,
            paint_cells_0 = cov.team_cells[0],
            paint_cells_1 = cov.team_cells[1],
            surface_cells = cov.surface_cells,
            match_left_s,
            match_over,
            matches = self.stats.matches.load(Ordering::Relaxed),
            desync_warnings = self.stats.desync_warnings.load(Ordering::Relaxed),
            "tick stats"
        );
        if t.p99_us as u64 > TICK_BUDGET_US {
            warn!(
                room = %self.params.label,
                p99_us = t.p99_us,
                budget_us = TICK_BUDGET_US,
                "tick p99 over budget"
            );
        }
        if starving(missing, t.ticks, players) {
            inc(&self.stats.desync_warnings);
            warn!(
                room = %self.params.label,
                missing_inputs = missing,
                player_ticks = t.ticks * players,
                "input starvation: clients will mispredict (desync risk)"
            );
        }
    }
}

/// Starvation (the server repeating a client's command) is what makes client
/// prediction diverge; a few misses around joins and hiccups are normal.
/// Missing inputs only count from a client's first applied input on (see
/// `ls_game::server::RoomStats::missing_inputs`).
fn starving(missing: u64, ticks: u64, players: u64) -> bool {
    players > 0 && missing > 10 + ticks * players / 50
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::{link_pair, ConnInfo, LinkConfig, LinkCounters, OutFrame, PeerLink};
    use crate::playlist::{PlaylistEntry, WorldChoice};
    use crate::rules::ServerRules;
    use ls_game::ls_sim::MatchPhase;
    use ls_game::ls_worldgen::{ShareCode, Sliders};

    fn code() -> ShareCode {
        ShareCode::new(99, Sliders::new(40, 30))
    }

    fn entry(rules: RuleSet, min: u8, max: u8) -> PlaylistEntry {
        PlaylistEntry {
            source: rules.name.clone(),
            rules: ServerRules {
                base: rules,
                bans: Vec::new(),
                balance_list: None,
            },
            world: WorldChoice::Fixed(code()),
            min_players: min,
            max_players: max,
        }
    }

    /// Sim parameters with a small room config (no pregeneration: tests stay
    /// below tick 30, the first paint coverage query), no lobby, 3 ticks of
    /// results.
    fn params(max_players: u8, playlist: Playlist) -> SimParams {
        let room_cfg = RoomConfig {
            max_players,
            pregenerate: false,
            ..RoomConfig::default()
        };
        SimParams {
            template: MatchConfig::paint_the_void(code()),
            room_cfg,
            stats_interval: Duration::from_secs(10),
            lobby: LobbyTiming::off(Duration::from_millis(50)), // 3 ticks
            playlist,
            label: "test".into(),
            close_when_empty: false,
            build_delay: Duration::ZERO,
        }
    }

    fn sim_with(params: SimParams) -> (Sim, mpsc::Sender<SimCommand>) {
        let mut rotation = Rotation::new(params.playlist.clone());
        let index = rotation.pick(0);
        let cfg = rotation
            .entry(index)
            .match_config(&params.template)
            .unwrap();
        let planned = Planned::new(index, cfg, false);
        let kind = if params.lobby.is_off() {
            RoomKind::Match
        } else {
            RoomKind::Warmup
        };
        let room = Room::new(planned.room_config(kind), params.room_cfg).unwrap();
        let (tx, rx) = mpsc::channel();
        let s = Sim::new(
            params,
            room,
            kind,
            planned,
            rotation,
            rx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(SharedStats::default()),
            SharedStatus::default(),
        );
        (s, tx)
    }

    fn sim(max_players: u8) -> (Sim, mpsc::Sender<SimCommand>) {
        let playlist = Playlist::new(vec![entry(RuleSet::paint_the_void(), 1, 32)]).unwrap();
        sim_with(params(max_players, playlist))
    }

    fn conn(id: u64, cfg: LinkConfig) -> (SimLink, PeerLink) {
        let info = ConnInfo {
            id,
            transport: "test",
            remote: "127.0.0.1:1".parse().unwrap(),
        };
        link_pair(info, cfg, Arc::new(LinkCounters::default()))
    }

    fn join(s: &mut Sim, id: u64, cfg: LinkConfig) -> PeerLink {
        let (link, peer) = conn(id, cfg);
        s.join(ls_game::client_hello(format!("p{id}"), None, None), link);
        peer
    }

    /// An encoded `Input` with one idle command for `tick`.
    fn idle_input(tick: u64) -> Vec<u8> {
        let cmd = ls_game::ls_sim::InputCmd::idle(tick);
        Message::Input(ls_net::InputMsg {
            epoch: 0,
            ack_snapshot: None,
            cmds: vec![ls_game::convert::input_to_net(&cmd)],
        })
        .encode()
        .unwrap()
    }

    /// Every frame queued for the peer right now, decoded.
    fn frames(peer: &PeerLink) -> Vec<Message> {
        let mut out = Vec::new();
        while let Some(OutFrame { data, .. }) = peer.try_next_outgoing() {
            if let Ok(m) = Message::decode(&data) {
                out.push(m);
            }
        }
        out
    }

    fn disconnect_reason(msgs: &[Message]) -> Option<String> {
        msgs.iter().find_map(|m| match m {
            Message::Disconnect { reason } => Some(reason.clone()),
            _ => None,
        })
    }

    /// The `Welcome`s in a message list.
    fn welcomes(msgs: &[Message]) -> Vec<&ls_net::Welcome> {
        msgs.iter()
            .filter_map(|m| match m {
                Message::Welcome(w) => Some(&**w),
                _ => None,
            })
            .collect()
    }

    /// The rule set of a `Welcome`.
    fn rules_of(w: &ls_net::Welcome) -> RuleSet {
        RuleSet::from_ron(&w.ruleset_name, std::str::from_utf8(&w.ruleset).unwrap()).unwrap()
    }

    /// Ticks (in real time, so worker builds can finish) until `done`.
    fn tick_until(s: &mut Sim, what: &str, mut done: impl FnMut(&Sim) -> bool) {
        let start = Instant::now();
        while !done(s) {
            assert!(
                start.elapsed() < Duration::from_secs(120),
                "timed out waiting for {what}"
            );
            s.tick();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn end_match(s: &mut Sim) {
        s.room.world_mut().mode_mut().phase = MatchPhase::Ended { winner: None };
    }

    /// Ends the match and ticks until the next match room is swapped in.
    fn next_match(s: &mut Sim) {
        let matches = s.stats.snapshot().matches;
        end_match(s);
        tick_until(s, "the next match", |s| {
            s.stats.snapshot().matches > matches && s.kind == RoomKind::Match
        });
    }

    #[tokio::test]
    async fn full_room_rejects_and_closes() {
        let (mut s, _tx) = sim(1);
        let a = join(&mut s, 1, LinkConfig::default());
        let b = join(&mut s, 2, LinkConfig::default());
        s.tick();
        assert!(matches!(frames(&a)[0], Message::Welcome(_)));
        let got = frames(&b);
        assert!(
            matches!(&got[..], [Message::Reject(r)] if r.reason == ls_net::RejectReason::ServerFull),
            "{got:?}"
        );
        assert_eq!(b.next_outgoing().await, None, "link closed after Reject");
        let st = s.stats.snapshot();
        assert_eq!((st.rejected, st.players), (1, 1));
    }

    #[tokio::test]
    async fn protocol_errors_get_a_kick() {
        let (mut s, _tx) = sim(2);
        let a = join(&mut s, 1, LinkConfig::default());
        s.tick();
        frames(&a);
        for _ in 0..=MAX_PROTOCOL_ERRORS {
            a.push_incoming(vec![0xff, 0x13, 0x37]).await.unwrap();
        }
        s.tick();
        assert!(s.clients.is_empty());
        assert_eq!(s.room.player_count(), 0);
        assert_eq!(
            disconnect_reason(&frames(&a)).as_deref(),
            Some("too many protocol errors")
        );
        assert_eq!(s.stats.snapshot().kicked, 1);
    }

    #[tokio::test]
    async fn graceful_disconnect_ignores_the_packets_after_it() {
        let (mut s, _tx) = sim(2);
        let a = join(&mut s, 1, LinkConfig::default());
        s.tick();
        frames(&a);
        let bye = Message::Disconnect {
            reason: "bye".into(),
        }
        .encode()
        .unwrap();
        a.push_incoming(bye).await.unwrap();
        for _ in 0..40 {
            a.push_incoming(vec![0xff]).await.unwrap();
        }
        s.tick();
        assert!(s.clients.is_empty());
        assert_eq!(s.stats.snapshot().kicked, 0, "left on its own");
        assert_eq!(disconnect_reason(&frames(&a)), None);
    }

    #[tokio::test]
    async fn reliable_overflow_kicks_the_slow_client() {
        let (mut s, _tx) = sim(2);
        let cfg = LinkConfig {
            max_reliable_bytes: 1,
            ..LinkConfig::default()
        };
        let _a = join(&mut s, 1, cfg);
        // The Welcome does not fit the reliable queue.
        s.tick();
        assert!(s.clients.is_empty());
        assert_eq!(s.room.player_count(), 0);
        assert_eq!(s.stats.snapshot().kicked, 1);
    }

    #[tokio::test]
    async fn oversized_snapshots_fall_back_to_the_reliable_channel() {
        let (mut s, _tx) = sim(2);
        let cfg = LinkConfig {
            max_unreliable_size: 8,
            ..LinkConfig::default()
        };
        let a = join(&mut s, 1, cfg);
        s.tick();
        // Snapshots flow once the client has sent an input (it is set up).
        a.push_incoming(idle_input(s.room.tick() + 4))
            .await
            .unwrap();
        for _ in 0..4 {
            s.tick();
        }
        assert_eq!(s.clients.len(), 1, "not dropped for a large datagram");
        let mut snapshots = 0;
        while let Some(f) = a.try_next_outgoing() {
            assert_eq!(f.channel, Channel::Reliable);
            if matches!(
                Message::decode_with(&f.data, &ls_net::SnapshotHistory::default()),
                Ok(Message::Snapshot(_))
            ) {
                snapshots += 1;
            }
        }
        assert!(snapshots >= 1);
    }

    #[tokio::test]
    async fn finished_match_moves_everyone_to_the_next_match_on_the_same_connection() {
        let (mut s, _tx) = sim(2);
        let a = join(&mut s, 1, LinkConfig::default());
        let b = join(&mut s, 2, LinkConfig::default());
        s.tick();
        for p in [&a, &b] {
            assert_eq!(welcomes(&frames(p)).len(), 1);
        }
        let ticks_before = s.stats.snapshot().total_ticks;
        end_match(&mut s);
        // Results (3 ticks): nobody is moved yet.
        for _ in 0..3 {
            s.tick();
            assert_eq!(s.lobby.phase().name(), "results");
        }
        next_match(&mut s);
        assert_eq!(s.clients.len(), 2, "both players moved");
        assert!(s.room.world().mode().is_running());
        s.tick();
        for p in [&a, &b] {
            let msgs = frames(p);
            assert_eq!(welcomes(&msgs).len(), 1, "one new Welcome: {msgs:?}");
            assert_eq!(disconnect_reason(&msgs), None);
            assert!(p.close_reason().is_none(), "connection stays open");
        }
        let st = s.stats.snapshot();
        assert_eq!(st.kicked, 0);
        assert_eq!(st.matches, 2);
        assert!(st.total_ticks > ticks_before, "totals carry over");
        assert_eq!(s.room.player_count(), 2);

        // And once more: matches keep rotating on the same connections.
        next_match(&mut s);
        s.tick();
        for p in [&a, &b] {
            assert_eq!(welcomes(&frames(p)).len(), 1);
        }
        assert_eq!(s.stats.snapshot().matches, 3);
    }

    #[tokio::test]
    async fn joins_during_the_next_match_build_are_welcomed_at_once() {
        let mut p = params(
            4,
            Playlist::new(vec![entry(RuleSet::paint_the_void(), 1, 32)]).unwrap(),
        );
        p.build_delay = Duration::from_secs(2);
        let (mut s, _tx) = sim_with(p);
        let a = join(&mut s, 1, LinkConfig::default());
        s.tick();
        frames(&a);
        end_match(&mut s);
        for _ in 0..5 {
            s.tick();
        }
        assert!(
            s.next.as_ref().is_some_and(|n| !n.built(RoomKind::Match)),
            "still building"
        );
        // A join while the next match is being built: Welcome on the next tick.
        let t0 = Instant::now();
        let b = join(&mut s, 2, LinkConfig::default());
        s.tick();
        assert_eq!(welcomes(&frames(&b)).len(), 1);
        assert!(
            t0.elapsed() < Duration::from_millis(200),
            "{:?}",
            t0.elapsed()
        );
        // The build finishes; both move over.
        next_match(&mut s);
        s.tick();
        assert_eq!(welcomes(&frames(&b)).len(), 1);
        assert_eq!(s.clients.len(), 2);
    }

    #[tokio::test]
    async fn playlist_entries_outside_the_player_range_are_skipped() {
        let mut sandbox = RuleSet::sandbox_all_weapons();
        sandbox.name = "Big".into();
        let mut third = RuleSet::paint_the_void();
        third.name = "Third".into();
        let playlist = Playlist::new(vec![
            entry(RuleSet::paint_the_void(), 1, 32),
            entry(sandbox, 3, 32),
            entry(third, 1, 32),
        ])
        .unwrap();
        let (mut s, _tx) = sim_with(params(4, playlist));
        let a = join(&mut s, 1, LinkConfig::default());
        let _b = join(&mut s, 2, LinkConfig::default());
        s.tick();
        frames(&a);
        next_match(&mut s);
        s.tick();
        let msgs = frames(&a);
        assert_eq!(
            rules_of(welcomes(&msgs)[0]).name,
            "Third",
            "Big (3+) skipped"
        );
        // A third player: the rotation reaches Big.
        let _c = join(&mut s, 3, LinkConfig::default());
        s.tick();
        frames(&a);
        next_match(&mut s); // Paint the Void (wrapped around)
        next_match(&mut s); // Big
        s.tick();
        let msgs = frames(&a);
        let names: Vec<String> = welcomes(&msgs).iter().map(|w| rules_of(w).name).collect();
        assert_eq!(names, ["Paint the Void", "Big"]);
    }

    #[tokio::test]
    async fn lobby_warms_up_counts_down_and_starts_the_match() {
        let mut p = params(
            4,
            Playlist::new(vec![entry(RuleSet::paint_the_void(), 2, 32)]).unwrap(),
        );
        p.lobby = LobbyTiming {
            warmup: Duration::from_millis(100),   // 6 ticks
            countdown: Duration::from_millis(50), // 3 ticks
            results: Duration::from_millis(50),
        };
        let (mut s, _tx) = sim_with(p);
        assert_eq!(s.kind, RoomKind::Warmup);
        assert_eq!(s.stats.snapshot().matches, 0);
        let a = join(&mut s, 1, LinkConfig::default());
        s.tick();
        let msgs = frames(&a);
        let w = rules_of(welcomes(&msgs)[0]);
        assert!(w.name.starts_with("Warm-up: "), "{}", w.name);
        // One player is below the entry's minimum (2): no countdown, and
        // after the warm-up no other entry fits better: countdown anyway.
        for _ in 0..3 {
            s.tick();
        }
        assert_eq!(s.lobby.phase().name(), "warmup");
        let _b = join(&mut s, 2, LinkConfig::default());
        tick_until(&mut s, "the countdown", |s| {
            s.lobby.phase().name() == "countdown"
        });
        tick_until(&mut s, "the match", |s| s.kind == RoomKind::Match);
        assert_eq!(s.lobby.phase(), Phase::Match);
        s.tick();
        let msgs = frames(&a);
        let ws = welcomes(&msgs);
        assert_eq!(ws.len(), 1);
        let rules = rules_of(ws[0]);
        assert_eq!(rules.name, "Paint the Void");
        assert_eq!(
            ws[0].share_code,
            code().encode().unwrap(),
            "same level as the warm-up"
        );
        assert_eq!(s.stats.snapshot().matches, 1);
        assert_eq!(s.status.read().unwrap().phase, "match");
        assert_eq!(s.status.read().unwrap().players, 2);
        // Match over: results, then the next warm-up (a warm-up room again).
        end_match(&mut s);
        tick_until(&mut s, "the next warm-up", |s| s.kind == RoomKind::Warmup);
        s.tick();
        let msgs = frames(&a);
        assert!(rules_of(welcomes(&msgs)[0]).name.starts_with("Warm-up: "));
        assert_eq!(disconnect_reason(&msgs), None);
    }

    #[tokio::test]
    async fn welcome_carries_the_effective_rules_and_the_balance_list_reloads() {
        let list =
            std::env::temp_dir().join(format!("ls-server-sim-balance-{}.ron", std::process::id()));
        std::fs::write(&list, r#"(bans: ["railspike"])"#).unwrap();
        let rules =
            ServerRules::new(None, vec!["tectonic_charge".into()], Some(list.clone())).unwrap();
        let playlist = Playlist::single("ptv", rules, WorldChoice::Fixed(code()));
        let (mut s, _tx) = sim_with(params(2, playlist));

        let a = join(&mut s, 1, LinkConfig::default());
        s.tick();
        let r = rules_of(welcomes(&frames(&a))[0]);
        assert!(r.availability("railspike").is_banned(), "balance list");
        assert!(r.availability("tectonic_charge").is_banned(), "--ban");
        assert!(!r.availability("seeker").is_banned());

        // The host edits the list: the next match plays the new rules.
        std::fs::write(&list, r#"(bans: ["seeker"])"#).unwrap();
        next_match(&mut s);
        s.tick();
        let r = rules_of(welcomes(&frames(&a))[0]);
        assert!(r.availability("seeker").is_banned());
        assert!(!r.availability("railspike").is_banned());
        assert!(r.availability("tectonic_charge").is_banned());
        let seeker = s.room.match_config().weapons.find("seeker").unwrap();
        assert!(s
            .room
            .world()
            .resolved_rules()
            .availability(seeker)
            .is_banned());

        // A broken list keeps the previous rules.
        std::fs::write(&list, "(bans: [").unwrap();
        next_match(&mut s);
        s.tick();
        assert_eq!(rules_of(welcomes(&frames(&a))[0]), r);
        std::fs::remove_file(&list).unwrap();
    }

    #[tokio::test]
    async fn private_rooms_close_once_empty() {
        let mut p = params(
            2,
            Playlist::new(vec![entry(RuleSet::paint_the_void(), 1, 32)]).unwrap(),
        );
        p.close_when_empty = true;
        let (mut s, _tx) = sim_with(p);
        for _ in 0..10 {
            s.tick();
        }
        assert!(!s.closing, "waits for its first player");
        let a = join(&mut s, 1, LinkConfig::default());
        s.tick();
        frames(&a);
        assert!(!s.closing);
        s.remove(0, Leave::Closed);
        s.tick();
        assert!(s.closing);
        s.close();
        assert!(s.status.read().unwrap().closed);
        // A room nobody joins closes after the idle time.
        let mut p = params(
            2,
            Playlist::new(vec![entry(RuleSet::paint_the_void(), 1, 32)]).unwrap(),
        );
        p.close_when_empty = true;
        let (mut s, tx) = sim_with(p);
        s.clock = lobby::ticks(PRIVATE_ROOM_IDLE) - 1;
        s.tick();
        assert!(s.closing);
        // A join that raced the close gets a Reject.
        let (link, peer) = conn(9, LinkConfig::default());
        tx.send(SimCommand::Join {
            hello: ls_game::client_hello("late", None, None),
            link,
        })
        .unwrap();
        s.close();
        let msgs = frames(&peer);
        assert!(
            matches!(&msgs[..], [Message::Reject(r)] if r.message == ROOM_CLOSED_REASON),
            "{msgs:?}"
        );
    }

    #[test]
    fn desync_warning_threshold() {
        // 1 player, 600 ticks: up to 10 + 12 misses are tolerated.
        assert!(!starving(22, 600, 1));
        assert!(starving(23, 600, 1));
        assert!(!starving(1000, 600, 0));
        // Scales with players.
        assert!(!starving(100, 600, 8));
        assert!(starving(107, 600, 8));
    }
}
