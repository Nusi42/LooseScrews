//! Bot runner: connects one bot to a server over WebSocket or WebTransport
//! ([`crate::transport`]) and drives its [`BotCore`] at 60 Hz until told to
//! stop, reconnecting after lost connections.
//!
//! With [`BotConfig::directory`] the server is picked from an `ls-directory`
//! before every connection ([`crate::social::pick_server`]); with
//! [`BotConfig::accept_duels`] the bot accepts duel challenges, plays the
//! duel in the challenger's private room and then returns to its server
//! ([`crate::social::spawn_duel_watcher`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ls_game::ls_math::TICK_HZ;
use ls_game::ls_net::{check_welcome, Message, RejectReason};
use ls_game::MATCH_RESTART_REASON;

use crate::bot::{BotCore, Flow};
use crate::social::{self, DuelMove, Identity};
use crate::stats::BotStats;
use crate::transport::{Link, Recv, TransportKind};

/// Most ticks run in one go after a stall; the rest are skipped (the client
/// clock catches up through its starvation handling).
const MAX_CATCH_UP_TICKS: u64 = 10;
/// Seconds between progress log lines per bot.
const PROGRESS_LOG_SECS: u64 = 30;
/// Limit for the goodbye `Disconnect` and the close handshake when stopping.
const GOODBYE_TIMEOUT: Duration = Duration::from_secs(1);
/// Wait before rejoining after the server restarted the match.
const RESTART_DELAY: Duration = Duration::from_millis(500);

/// Settings of one bot.
#[derive(Debug, Clone)]
pub struct BotConfig {
    pub name: String,
    /// WebSocket URL, e.g. `ws://127.0.0.1:7777/ws` (WebTransport fetches
    /// the certificate hash from its host and port).
    pub server: String,
    pub transport: TransportKind,
    /// Varies the behavior between bots.
    pub seed: u64,
    /// Wait before the first connection (staggered joins).
    pub start_delay: Duration,
    /// Try again after a lost or failed connection.
    pub reconnect: bool,
    pub reconnect_delay: Duration,
    /// Limit for the connection handshake and for the `Welcome`.
    pub connect_timeout: Duration,
    /// A write the server does not take within this time ends the session
    /// (the server stopped reading).
    pub write_timeout: Duration,
    /// A session in which nothing arrives for this long is lost (the server
    /// hangs or the connection is half open).
    pub silence_timeout: Duration,
    /// Private room to join (`Hello::room`; `None` = the public room).
    pub room: Option<String>,
    /// Directory URL (`http://host:port`): pick the server from its list
    /// before every connection instead of using [`BotConfig::server`].
    pub directory: Option<String>,
    /// Accept duel challenges (presence on the server, inbox polling).
    pub accept_duels: bool,
}

impl BotConfig {
    pub fn new(name: impl Into<String>, server: impl Into<String>, seed: u64) -> BotConfig {
        BotConfig {
            name: name.into(),
            server: server.into(),
            transport: TransportKind::WebSocket,
            seed,
            start_delay: Duration::ZERO,
            reconnect: true,
            reconnect_delay: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(15),
            write_timeout: Duration::from_secs(5),
            silence_timeout: Duration::from_secs(5),
            room: None,
            directory: None,
            accept_duels: false,
        }
    }
}

/// Shared stop request plus an optional deadline.
#[derive(Debug, Clone)]
pub struct StopSignal {
    flag: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl StopSignal {
    pub fn new(deadline: Option<Instant>) -> StopSignal {
        StopSignal {
            flag: Arc::new(AtomicBool::new(false)),
            deadline,
        }
    }

    /// Asks every bot sharing this signal to stop.
    pub fn stop(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn should_stop(&self) -> bool {
        self.flag.load(Ordering::SeqCst) || self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

enum SessionEnd {
    Stopped,
    ConnectFailed(String),
    Lost(String),
    /// The server finished the match and asked everyone to rejoin
    /// ([`MATCH_RESTART_REASON`]).
    MatchRestart,
    /// The bot accepted a duel: reconnect into its room.
    Duel(DuelMove),
    /// The duel match is over: back to the home server.
    DuelOver,
}

/// Where a session connects to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    url: String,
    room: Option<String>,
    /// A duel room (left after its match).
    duel: bool,
}

/// Runs a bot on the current thread with its own single-threaded tokio
/// runtime (bots do CPU work for prediction and terrain queries, so each
/// gets a thread). Returns its statistics when stopped.
pub fn run_bot_blocking(cfg: BotConfig, stop: StopSignal) -> BotStats {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(run_bot(cfg, stop)),
        Err(e) => {
            let mut stats = BotStats::new(cfg.name);
            stats.note_error(format!("cannot start runtime: {e}"));
            stats.connect_failures += 1;
            stats
        }
    }
}

/// Connects, plays and reconnects until `stop` fires.
pub async fn run_bot(cfg: BotConfig, stop: StopSignal) -> BotStats {
    let mut stats = BotStats::new(cfg.name.clone());
    if !sleep_unless_stopped(cfg.start_delay, &stop).await {
        return stats;
    }
    let mut attempt: u64 = 0;
    let mut duel: Option<DuelMove> = None;
    while !stop.should_stop() {
        let seed = cfg
            .seed
            .wrapping_add(attempt.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let target = match &duel {
            Some(d) => Target {
                url: social::ws_url(&d.address),
                room: Some(d.room.clone()),
                duel: true,
            },
            None => match &cfg.directory {
                Some(dir) => match social::pick_server(dir).await {
                    Ok(url) => Target {
                        url,
                        room: cfg.room.clone(),
                        duel: false,
                    },
                    Err(e) => {
                        stats.connect_failures += 1;
                        tracing::warn!(bot = %cfg.name, "no server from the directory: {e}");
                        if stats.connect_failures <= 3 {
                            stats.note_error(format!("no server from the directory: {e}"));
                        }
                        attempt += 1;
                        if !cfg.reconnect || !sleep_unless_stopped(cfg.reconnect_delay, &stop).await
                        {
                            break;
                        }
                        continue;
                    }
                },
                None => Target {
                    url: cfg.server.clone(),
                    room: cfg.room.clone(),
                    duel: false,
                },
            },
        };
        match play_session(&cfg, &target, seed, &stop, &mut stats).await {
            SessionEnd::Stopped => break,
            SessionEnd::Duel(m) => {
                stats.duels += 1;
                tracing::info!(bot = %cfg.name, room = %m.room, from = %m.from, "joining a duel");
                duel = Some(m);
                attempt += 1;
                continue;
            }
            SessionEnd::DuelOver => {
                tracing::info!(bot = %cfg.name, "duel over; back to the server");
                duel = None;
                attempt += 1;
                continue;
            }
            SessionEnd::ConnectFailed(e) => {
                // A duel room that is gone (closed, full) is given up.
                duel = None;
                stats.connect_failures += 1;
                tracing::warn!(bot = %cfg.name, "connection failed: {e}");
                if stats.connect_failures <= 3 {
                    stats.note_error(format!("connection failed: {e}"));
                }
            }
            SessionEnd::Lost(e) => {
                duel = None;
                stats.disconnects += 1;
                tracing::warn!(bot = %cfg.name, "disconnected: {e}");
                stats.note_error(format!("disconnected: {e}"));
            }
            SessionEnd::MatchRestart => {
                stats.match_restarts += 1;
                tracing::info!(bot = %cfg.name, "match over; rejoining for the next match");
                attempt += 1;
                if !cfg.reconnect || !sleep_unless_stopped(RESTART_DELAY, &stop).await {
                    break;
                }
                continue;
            }
        }
        attempt += 1;
        if !cfg.reconnect || !sleep_unless_stopped(cfg.reconnect_delay, &stop).await {
            break;
        }
    }
    stats
}

/// Sleeps in small steps; false if `stop` fired meanwhile.
async fn sleep_unless_stopped(d: Duration, stop: &StopSignal) -> bool {
    let end = Instant::now() + d;
    loop {
        if stop.should_stop() {
            return false;
        }
        let now = Instant::now();
        if now >= end {
            return true;
        }
        tokio::time::sleep((end - now).min(Duration::from_millis(50))).await;
    }
}

async fn play_session(
    cfg: &BotConfig,
    target: &Target,
    seed: u64,
    stop: &StopSignal,
    stats: &mut BotStats,
) -> SessionEnd {
    // --- connect and handshake ---------------------------------------------
    let connect = Link::connect(cfg.transport, target.url.as_str());
    let mut ws = match tokio::time::timeout(cfg.connect_timeout, connect).await {
        Ok(Ok(link)) => link,
        Ok(Err(e)) => return SessionEnd::ConnectFailed(e),
        Err(_) => return SessionEnd::ConnectFailed("connect timed out".into()),
    };
    let hello = Message::Hello(
        crate::bot::bot_hello(cfg.name.as_str(), seed).with_room(target.room.clone()),
    );
    let bytes = match hello.encode() {
        Ok(b) => b,
        Err(e) => return protocol_error(stats, format!("cannot encode Hello: {e}")),
    };
    let sent = async {
        ws.send(bytes, hello.channel()).await?;
        ws.flush().await
    };
    if let Err(e) = write_within(cfg.write_timeout, sent).await {
        return SessionEnd::ConnectFailed(e);
    }
    let first = match ws.first_packet(cfg.connect_timeout).await {
        Ok(b) => b,
        Err(e) => return SessionEnd::ConnectFailed(e),
    };
    let welcome = match Message::decode(&first) {
        Ok(Message::Reject(r)) => {
            return match r.reason {
                RejectReason::VersionMismatch | RejectReason::BadHandshake => {
                    protocol_error(stats, format!("rejected ({:?}): {}", r.reason, r.message))
                }
                _ => SessionEnd::ConnectFailed(format!("rejected ({:?}): {}", r.reason, r.message)),
            };
        }
        Ok(msg) => match check_welcome(msg) {
            Ok(w) => w,
            Err(e) => return protocol_error(stats, format!("bad handshake: {e}")),
        },
        Err(e) => return protocol_error(stats, format!("undecodable Welcome: {e}")),
    };
    let mut core = match BotCore::from_welcome(&welcome, seed) {
        Ok(c) => c,
        Err(e) => {
            let _ = tokio::time::timeout(GOODBYE_TIMEOUT, ws.close()).await;
            return protocol_error(stats, format!("unusable Welcome: {e}"));
        }
    };
    stats.joins += 1;
    tracing::info!(
        bot = %cfg.name,
        player = welcome.player_id,
        team = welcome.team,
        code = %welcome.share_code,
        room = %welcome.room_code,
        transport = cfg.transport.name(),
        edits = welcome.edit_log.len(),
        "joined"
    );

    // Duels: a watcher on the home server (not inside a duel), and the end
    // of the duel's match.
    let (duel_tx, mut duel_rx) = tokio::sync::mpsc::unbounded_channel::<DuelMove>();
    let watcher = (cfg.accept_duels && !target.duel)
        .then(|| social::host_port(&target.url))
        .flatten()
        .map(|address| {
            social::spawn_duel_watcher(
                Identity::for_bot(&cfg.name, cfg.seed),
                address,
                cfg.directory.clone(),
                duel_tx.clone(),
            )
        });
    drop(duel_tx);
    let matches_at_join = stats.matches_over;

    // --- 60 Hz loop -------------------------------------------------------------
    let tick_period = Duration::from_secs_f64(1.0 / TICK_HZ as f64);
    let mut interval = tokio::time::interval(tick_period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let start = Instant::now();
    let mut ticks_done: u64 = 0;
    let mut next_log = start + Duration::from_secs(PROGRESS_LOG_SECS);
    let mut last_rx = Instant::now();

    let end = loop {
        tokio::select! {
            Some(m) = duel_rx.recv() => {
                goodbye(&mut ws, "off to a duel").await;
                break SessionEnd::Duel(m);
            }
            incoming = ws.recv() => match incoming {
                Recv::Packet(bytes) => {
                    last_rx = Instant::now();
                    match core.handle_packet(&bytes, stats) {
                        Ok(Flow::Continue) => {}
                        Ok(Flow::Disconnected(reason)) if reason == MATCH_RESTART_REASON => {
                            break SessionEnd::MatchRestart;
                        }
                        Ok(Flow::Disconnected(reason)) => {
                            break SessionEnd::Lost(format!("server ended the session: {reason}"));
                        }
                        Err(e) => {
                            let _ = tokio::time::timeout(GOODBYE_TIMEOUT, ws.close()).await;
                            break SessionEnd::Lost(e.to_string());
                        }
                    }
                }
                Recv::Other => last_rx = Instant::now(),
                Recv::Closed(why) => break SessionEnd::Lost(why),
            },
            _ = interval.tick() => {
                if stop.should_stop() {
                    goodbye(&mut ws, "bot stopping").await;
                    break SessionEnd::Stopped;
                }
                if target.duel && stats.matches_over > matches_at_join {
                    goodbye(&mut ws, "duel over").await;
                    break SessionEnd::DuelOver;
                }
                if last_rx.elapsed() > cfg.silence_timeout {
                    break SessionEnd::Lost(format!(
                        "no packets from the server for {:.0} s",
                        cfg.silence_timeout.as_secs_f64()
                    ));
                }
                let target = (start.elapsed().as_secs_f64() * TICK_HZ as f64) as u64;
                let due = target.saturating_sub(ticks_done).min(MAX_CATCH_UP_TICKS);
                ticks_done = target;
                let mut sent = false;
                for _ in 0..due {
                    for m in core.tick(stats) {
                        let bytes = match m.encode() {
                            Ok(b) => b,
                            Err(e) => {
                                stats.protocol_errors += 1;
                                stats.note_error(format!("cannot encode own message: {e}"));
                                continue;
                            }
                        };
                        let feed = ws.send(bytes, m.channel());
                        if let Err(e) = write_within(cfg.write_timeout, feed).await {
                            core.finish(stats);
                            return SessionEnd::Lost(e);
                        }
                        sent = true;
                    }
                }
                if sent {
                    if let Err(e) = write_within(cfg.write_timeout, ws.flush()).await {
                        core.finish(stats);
                        return SessionEnd::Lost(e);
                    }
                }
                if Instant::now() >= next_log {
                    next_log += Duration::from_secs(PROGRESS_LOG_SECS);
                    let p = core.session().prediction_stats();
                    tracing::info!(
                        bot = %cfg.name,
                        mode = ?core.brain().mode(),
                        inputs = stats.inputs_sent,
                        snapshots = stats.snapshots,
                        corrections = p.corrected,
                        snaps = p.snapped,
                        kills = stats.kills,
                        deaths = stats.deaths,
                        "progress"
                    );
                }
            }
        }
    };
    if let Some(w) = watcher {
        w.abort();
    }
    core.finish(stats);
    end
}

/// Sends a `Disconnect` and closes the link (both within
/// [`GOODBYE_TIMEOUT`]).
async fn goodbye(ws: &mut Link, reason: &str) {
    let bye = Message::Disconnect {
        reason: reason.into(),
    };
    if let Ok(bytes) = bye.encode() {
        let send = async {
            ws.send(bytes, bye.channel()).await?;
            ws.flush().await
        };
        let _ = tokio::time::timeout(GOODBYE_TIMEOUT, send).await;
    }
    let _ = tokio::time::timeout(GOODBYE_TIMEOUT, ws.close()).await;
}

/// Runs a socket write with a time limit: a server that stops reading must
/// not block the bot (and with it `--duration`) forever.
async fn write_within<F, E>(limit: Duration, write: F) -> Result<(), String>
where
    F: std::future::Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    match tokio::time::timeout(limit, write).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!(
            "send timed out after {:.0} s (server not reading)",
            limit.as_secs_f64()
        )),
    }
}

/// Counts a protocol error during the handshake (the session never started).
fn protocol_error(stats: &mut BotStats, msg: String) -> SessionEnd {
    stats.protocol_errors += 1;
    stats.note_error(msg.clone());
    SessionEnd::ConnectFailed(msg)
}
