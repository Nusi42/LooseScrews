//! The dedicated simulation thread: owns the [`Room`], runs it at a fixed
//! 60 Hz, polls every client's [`SimLink`] and hands outgoing packets back.
//!
//! The room is built on this thread (its terrain is `Send` but not `Sync`)
//! and never leaves it; other threads reach it through [`SimCommand`]s.
//!
//! Policies:
//!
//! - **Joins**: a full room (or a bad version) gets `Reject` and the link
//!   closes. The first player to join after a finished match triggers the
//!   rebuild of a fresh match (a stall of a second or two on this thread,
//!   while nobody plays; the scheduler restarts afterwards instead of
//!   counting the stall as skipped ticks). An idle or exiting server never
//!   rebuilds.
//! - **Match end**: once a match is over, players see the result for
//!   [`SimParams::intermission`]; then every client gets `Disconnect` with
//!   [`MATCH_RESTART_REASON`] and is removed (not counted as kicked).
//!   Clients reconnect and the first join starts the next match.
//! - **Kicks** (`Disconnect` with a reason, counted in `kicked`): more than
//!   [`MAX_PROTOCOL_ERRORS`] undecodable or unexpected packets, or a reliable
//!   send queue over `LinkConfig::max_reliable_bytes` (client too slow).
//! - **Oversized unreliable packets** (over the transport's datagram limit;
//!   WebTransport in wave 4) are sent on the reliable channel instead.
//! - **Desync warning**: a stats window in which the server had to repeat
//!   client commands for more than `10 + 2 %` of the player ticks.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ls_game::server::{Room, RoomConfig, RoomStats};
use ls_game::{MatchConfig, MATCH_RESTART_REASON};
use ls_math::terrain::TerrainEdit;
use ls_math::TICK_HZ;
use ls_net::{Channel, Hello, Message, Transport, TransportError};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

use crate::link::SimLink;
use crate::stats::{inc, set, SharedStats};
use crate::timing::{wait_until, TickScheduler, TickTimes, TICK_PERIOD};

/// Packets handled per client per tick (the rest wait for the next tick).
const MAX_PACKETS_PER_TICK: usize = 64;
/// Undecodable or unexpected packets tolerated before a client is kicked.
pub const MAX_PROTOCOL_ERRORS: u32 = 32;
/// Tick time budget from the plan (p99).
const TICK_BUDGET_US: u64 = 4_000;
/// Default time players see a finished match before the next one starts.
pub const DEFAULT_INTERMISSION: Duration = Duration::from_secs(10);

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

/// What the room was built with (reported to the starter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomInfo {
    pub share_code: String,
    pub ruleset: String,
}

/// Sim thread settings.
#[derive(Debug, Clone)]
pub struct SimParams {
    pub match_cfg: MatchConfig,
    pub room_cfg: RoomConfig,
    /// How often tick statistics are logged.
    pub stats_interval: Duration,
    /// How long a finished match stays on screen before everyone is sent
    /// off to rejoin the next one.
    pub intermission: Duration,
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
    /// The match ended and the intermission is over.
    MatchOver,
}

impl Leave {
    fn text(self) -> &'static str {
        match self {
            Leave::Disconnected => "client disconnected",
            Leave::Closed => "connection closed",
            Leave::ProtocolErrors => "too many protocol errors",
            Leave::TooSlow => "send buffer overflow (client too slow)",
            Leave::MatchOver => MATCH_RESTART_REASON,
        }
    }
}

struct ClientConn {
    link: SimLink,
    errors: u32,
}

struct Sim {
    params: SimParams,
    room: Room,
    clients: BTreeMap<u8, ClientConn>,
    commands: mpsc::Receiver<SimCommand>,
    shutdown: Arc<AtomicBool>,
    stats: Arc<SharedStats>,
    times: TickTimes,
    window_room: RoomStats,
    window_skipped: u64,
    window_dropped: u64,
    /// Counters of the rooms replaced by [`Sim::new_match`], so the shared
    /// totals keep growing across matches.
    past_matches: RoomStats,
    past_ticks: u64,
    past_edits: u64,
    /// Set by [`Sim::new_match`]: the room was rebuilt (a deliberate stall of
    /// a second or two while nobody plays); the scheduler restarts instead of
    /// counting the rebuild as skipped ticks.
    rebuilt: bool,
    /// Room tick at which the current match was first seen over (with
    /// players connected).
    match_over_since: Option<u64>,
}

/// Starts the sim thread. `ready` receives the room info once the room is
/// built (or the error if it cannot be).
pub fn spawn(
    params: SimParams,
    commands: mpsc::Receiver<SimCommand>,
    shutdown: Arc<AtomicBool>,
    stats: Arc<SharedStats>,
    ready: oneshot::Sender<Result<RoomInfo, String>>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("ls-sim".into())
        .spawn(move || {
            let started = Instant::now();
            let room = match Room::new(params.match_cfg.clone(), params.room_cfg) {
                Ok(r) => r,
                Err(e) => {
                    let _ = ready.send(Err(e.to_string()));
                    return;
                }
            };
            let info = RoomInfo {
                share_code: params.match_cfg.code.encode().unwrap_or_default(),
                ruleset: params.match_cfg.rules.name.clone(),
            };
            info!(
                share_code = %info.share_code,
                ruleset = %info.ruleset,
                build_ms = started.elapsed().as_millis() as u64,
                "match ready"
            );
            let mut sim = Sim::new(params, room, commands, shutdown, stats);
            if ready.send(Ok(info)).is_err() {
                return;
            }
            sim.run();
        })
}

impl Sim {
    fn new(
        params: SimParams,
        room: Room,
        commands: mpsc::Receiver<SimCommand>,
        shutdown: Arc<AtomicBool>,
        stats: Arc<SharedStats>,
    ) -> Sim {
        inc(&stats.matches);
        Sim {
            params,
            room,
            clients: BTreeMap::new(),
            commands,
            shutdown,
            stats,
            times: TickTimes::default(),
            window_room: RoomStats::default(),
            window_skipped: 0,
            window_dropped: 0,
            past_matches: RoomStats::default(),
            past_ticks: 0,
            past_edits: 0,
            rebuilt: false,
            match_over_since: None,
        }
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
            let now = Instant::now();
            let due = sched.due(now);
            if due.skipped > 0 {
                warn!(
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
                if std::mem::take(&mut self.rebuilt) {
                    // Not a tick-time sample, and not a backlog to catch up.
                    sched = TickScheduler::sixty_hz(Instant::now());
                    break;
                }
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
    /// then the match-end policy.
    fn tick(&mut self) {
        self.process_commands();
        self.receive();
        // The match clock only runs while someone plays.
        if self.room.player_count() > 0 {
            self.room.step();
        }
        self.dispatch();
        self.check_match_end();
        self.publish_stats();
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

    fn process_commands(&mut self) {
        while let Ok(cmd) = self.commands.try_recv() {
            match cmd {
                SimCommand::Join { hello, link } => self.join(hello, link),
                SimCommand::Inspect(f) => f(&mut self.room),
            }
        }
    }

    fn join(&mut self, hello: Hello, mut link: SimLink) {
        // A finished match nobody plays any more is replaced lazily, when the
        // next player arrives.
        if self.clients.is_empty()
            && self.room.player_count() == 0
            && !self.room.world().mode().is_running()
        {
            self.new_match();
        }
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
                    conn,
                    player = id,
                    team,
                    name = %hello.name,
                    remote = %link.info().remote,
                    transport = link.info().transport,
                    players = self.room.player_count(),
                    "player joined"
                );
                self.clients.insert(id, ClientConn { link, errors: 0 });
            }
            Err(reject) => {
                info!(conn, name = %hello.name, reason = ?reject.reason, "join rejected");
                inc(&self.stats.rejected);
                if let Ok(bytes) = Message::Reject(reject).encode() {
                    let _ = link.send_reliable(&bytes);
                }
                link.close();
            }
        }
    }

    /// Replaces a finished match with a fresh one (only while nobody plays).
    fn new_match(&mut self) {
        let started = Instant::now();
        match Room::new(self.params.match_cfg.clone(), self.params.room_cfg) {
            Ok(room) => {
                let old = self.room.stats();
                let p = &mut self.past_matches;
                p.late_inputs += old.late_inputs;
                p.missing_inputs += old.missing_inputs;
                p.snapshots_sent += old.snapshots_sent;
                p.snapshot_bytes += old.snapshot_bytes;
                p.encode_errors += old.encode_errors;
                self.past_ticks += self.room.tick();
                self.past_edits += self.room.terrain().edit_count() as u64;
                self.rebuilt = true;
                self.room = room;
                self.window_room = RoomStats::default();
                self.match_over_since = None;
                inc(&self.stats.matches);
                info!(
                    build_ms = started.elapsed().as_millis() as u64,
                    matches = self.stats.matches.load(Ordering::Relaxed),
                    "previous match finished; started a new one"
                );
            }
            Err(e) => warn!(error = %e, "could not start a new match"),
        }
    }

    /// After the intermission of a finished match, sends every client off to
    /// rejoin (see the module docs).
    fn check_match_end(&mut self) {
        if self.clients.is_empty() || self.room.world().mode().is_running() {
            self.match_over_since = None;
            return;
        }
        let now = self.room.tick();
        let since = *self.match_over_since.get_or_insert(now);
        let intermission = (self.params.intermission.as_secs_f64() * f64::from(TICK_HZ)) as u64;
        if now.saturating_sub(since) < intermission {
            return;
        }
        info!(
            players = self.clients.len(),
            "match over; sending players off to rejoin the next match"
        );
        let ids: Vec<u8> = self.clients.keys().copied().collect();
        for id in ids {
            self.send_disconnect(id, MATCH_RESTART_REASON);
            self.remove(id, Leave::MatchOver);
        }
        self.match_over_since = None;
    }

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
        info!(
            tick = self.room.tick(),
            total_ticks = self.past_ticks + self.room.tick(),
            players,
            ticks = t.ticks,
            p50_us = t.p50_us,
            p99_us = t.p99_us,
            max_us = t.max_us,
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
                p99_us = t.p99_us,
                budget_us = TICK_BUDGET_US,
                "tick p99 over budget"
            );
        }
        if starving(missing, t.ticks, players) {
            inc(&self.stats.desync_warnings);
            warn!(
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
    use ls_game::ls_sim::MatchPhase;
    use ls_game::ls_worldgen::{ShareCode, Sliders};

    /// A sim with a small room config (no pregeneration: tests stay below
    /// tick 30, the first paint coverage query).
    fn sim(max_players: u8) -> (Sim, mpsc::Sender<SimCommand>) {
        let code = ShareCode::new(99, Sliders::new(40, 30));
        let room_cfg = RoomConfig {
            max_players,
            pregenerate: false,
            ..RoomConfig::default()
        };
        let params = SimParams {
            match_cfg: MatchConfig::paint_the_void(code),
            room_cfg,
            stats_interval: Duration::from_secs(10),
            intermission: Duration::from_millis(50), // 3 ticks
        };
        let room = Room::new(params.match_cfg.clone(), room_cfg).unwrap();
        let (tx, rx) = mpsc::channel();
        let s = Sim::new(
            params,
            room,
            rx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(SharedStats::default()),
        );
        (s, tx)
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
    async fn finished_match_sends_players_off_and_the_next_join_rebuilds() {
        let (mut s, _tx) = sim(2);
        let a = join(&mut s, 1, LinkConfig::default());
        let b = join(&mut s, 2, LinkConfig::default());
        s.tick();
        s.room.world_mut().mode_mut().phase = MatchPhase::Ended { winner: Some(0) };
        // Intermission (3 ticks), then everyone is sent off.
        for _ in 0..3 {
            s.tick();
            assert_eq!(s.clients.len(), 2);
        }
        s.tick();
        assert!(s.clients.is_empty());
        for p in [&a, &b] {
            assert_eq!(
                disconnect_reason(&frames(p)).as_deref(),
                Some(MATCH_RESTART_REASON)
            );
            assert_eq!(p.next_outgoing().await, None);
        }
        let st = s.stats.snapshot();
        assert_eq!(st.kicked, 0);
        assert_eq!(st.matches, 1, "no rebuild while nobody plays");
        let ticks = st.total_ticks;
        assert!(ticks >= 5);

        // Idle ticks with nobody connected do not rebuild either.
        s.tick();
        assert_eq!(s.stats.snapshot().matches, 1);
        // The first join does, and totals carry over.
        let c = join(&mut s, 3, LinkConfig::default());
        assert!(s.rebuilt);
        assert!(s.room.world().mode().is_running());
        s.tick();
        assert!(matches!(frames(&c)[0], Message::Welcome(_)));
        let st = s.stats.snapshot();
        assert_eq!(st.matches, 2);
        assert_eq!(st.tick, 1);
        assert_eq!(st.total_ticks, ticks + 1);
    }

    #[test]
    fn last_player_leaving_a_finished_match_does_not_rebuild() {
        let (mut s, _tx) = sim(2);
        let (link, _peer) = conn(1, LinkConfig::default());
        s.join(ls_game::client_hello("a", None, None), link);
        s.tick();
        s.room.world_mut().mode_mut().phase = MatchPhase::Ended { winner: None };
        s.remove(0, Leave::Closed);
        assert_eq!(s.stats.snapshot().matches, 1);
        assert!(!s.rebuilt);
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
