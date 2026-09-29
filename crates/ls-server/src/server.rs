//! Server assembly: configuration, startup and the [`ServerHandle`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::thread::JoinHandle as ThreadHandle;
use std::time::Duration;

use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;
use ls_sim::{RuleSet, WeaponTable};
use ls_worldgen::{ShareCode, Sliders};
use thiserror::Error;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::info;

use crate::endpoint::{CertHash, ConnSettings, Endpoint, EndpointContext, Hub};
use crate::http::HttpEndpoint;
use crate::sim::{self, RoomInfo, SimCommand, SimParams};
use crate::stats::{SharedStats, StatsSnapshot};

/// Errors while starting or talking to the server.
#[derive(Debug, Error)]
pub enum ServerError {
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("share code: {0}")]
    ShareCode(#[from] ls_worldgen::WorldgenError),
    #[error("rule set: {0}")]
    RuleSet(#[from] ls_sim::SimError),
    #[error("building the match failed: {0}")]
    Room(String),
    #[error("the simulation thread has stopped")]
    SimStopped,
}

/// Everything needed to start a server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub match_cfg: MatchConfig,
    pub room: RoomConfig,
    /// Directory served over HTTP (the built web client); `None` serves a
    /// placeholder page.
    pub web_root: Option<PathBuf>,
    pub conn: ConnSettings,
    /// Interval of the tick statistics log line.
    pub stats_interval: Duration,
    /// How long a finished match stays on screen before players are sent
    /// off to rejoin the next one (see [`crate::sim`]).
    pub intermission: Duration,
}

impl ServerConfig {
    /// Defaults: 8 players, 30 Hz snapshots, lag compensation on, stats
    /// every 10 s, 10 s intermission after a match, no web root.
    pub fn new(bind: SocketAddr, match_cfg: MatchConfig) -> Self {
        ServerConfig {
            bind,
            match_cfg,
            room: RoomConfig::default(),
            web_root: None,
            conn: ConnSettings::default(),
            stats_interval: Duration::from_secs(10),
            intermission: sim::DEFAULT_INTERMISSION,
        }
    }
}

/// Builds the match configuration from CLI-style inputs: a share code (a
/// fresh random world if `None`) and a rule set file (the built-in Paint the
/// Void rules if `None`). Weapons and ship tuning are the built-in tables,
/// which the clients use too. The rule set is validated against them.
pub fn match_config(
    code: Option<&str>,
    ruleset: Option<&Path>,
) -> Result<MatchConfig, ServerError> {
    let code = match code {
        Some(text) => ShareCode::decode(text)?,
        None => ShareCode::new(random_seed(), Sliders::default()),
    };
    let rules = match ruleset {
        Some(path) => RuleSet::load(path)?,
        None => RuleSet::paint_the_void(),
    };
    let weapons = WeaponTable::builtin();
    rules.resolve(&weapons)?;
    Ok(MatchConfig {
        rules,
        weapons,
        ..MatchConfig::paint_the_void(code)
    })
}

/// A seed for a random world (wall clock and process id; not deterministic
/// on purpose, and outside the deterministic crates).
fn random_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    ls_math::rng::mix64(nanos ^ ((std::process::id() as u64) << 32))
}

/// Starts servers; see [`Server::start`].
#[derive(Debug)]
pub struct Server;

impl Server {
    /// Binds the port, builds the match on the sim thread (waits until it is
    /// ready) and starts accepting HTTP and WebSocket connections.
    pub async fn start(config: ServerConfig) -> Result<ServerHandle, ServerError> {
        let endpoint = HttpEndpoint::bind(config.bind, config.web_root.clone()).await?;
        let local_addr = endpoint.local_addr()?;
        let stats = Arc::new(SharedStats::default());
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let sim_stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = oneshot::channel();
        let params = SimParams {
            match_cfg: config.match_cfg.clone(),
            room_cfg: config.room,
            stats_interval: config.stats_interval,
            intermission: config.intermission,
        };
        let sim_thread = sim::spawn(params, cmd_rx, sim_stop.clone(), stats.clone(), ready_tx)?;
        let room = match ready_rx.await {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => {
                let _ = sim_thread.join();
                return Err(ServerError::Room(e));
            }
            Err(_) => {
                let _ = sim_thread.join();
                return Err(ServerError::SimStopped);
            }
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut handle = ServerHandle {
            local_addr,
            room,
            stats: stats.clone(),
            hub: Hub::new(cmd_tx, stats, config.conn),
            shutdown_tx,
            shutdown_rx,
            sim_stop,
            sim_thread: Some(sim_thread),
            endpoints: Vec::new(),
            cert_hash: Arc::new(RwLock::new(None)),
        };
        handle.spawn_endpoint(Box::new(endpoint));
        info!(
            addr = %local_addr,
            share_code = %handle.room.share_code,
            ruleset = %handle.room.ruleset,
            max_players = config.room.max_players,
            "listening (http + websocket at /ws)"
        );
        Ok(handle)
    }
}

/// A running server. Dropping it stops the sim thread and the endpoints
/// without waiting; [`ServerHandle::shutdown`] stops them gracefully.
#[derive(Debug)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    room: RoomInfo,
    stats: Arc<SharedStats>,
    hub: Hub,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    sim_stop: Arc<AtomicBool>,
    sim_thread: Option<ThreadHandle<()>>,
    endpoints: Vec<JoinHandle<()>>,
    cert_hash: CertHash,
}

impl ServerHandle {
    /// The bound address (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The match's share code, as printed.
    pub fn share_code(&self) -> &str {
        &self.room.share_code
    }

    pub fn ruleset_name(&self) -> &str {
        &self.room.ruleset
    }

    /// Current counters.
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// The hub endpoints use to register connections.
    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// Where a WebTransport endpoint publishes its certificate hash.
    pub fn cert_hash(&self) -> &CertHash {
        &self.cert_hash
    }

    /// Starts another endpoint (the WebTransport seam for wave 4).
    pub fn spawn_endpoint(&mut self, endpoint: Box<dyn Endpoint>) {
        let ctx = EndpointContext {
            hub: self.hub.clone(),
            shutdown: self.shutdown_rx.clone(),
            cert_hash: self.cert_hash.clone(),
        };
        info!(endpoint = endpoint.name(), "endpoint started");
        self.endpoints.push(endpoint.spawn(ctx));
    }

    /// Runs `f` on the sim thread against the room at the start of the next
    /// tick and returns its result.
    pub async fn with_room<R, F>(&self, f: F) -> Result<R, ServerError>
    where
        R: Send + 'static,
        F: FnOnce(&mut Room) -> R + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.hub
            .send_command(SimCommand::Inspect(Box::new(move |room| {
                let _ = tx.send(f(room));
            })))
            .map_err(|_| ServerError::SimStopped)?;
        rx.await.map_err(|_| ServerError::SimStopped)
    }

    /// Graceful shutdown: players get `Disconnect`, sockets flush and close,
    /// then the sim thread and endpoints are joined.
    pub async fn shutdown(mut self) {
        info!("shutting down");
        self.sim_stop.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
        if let Some(t) = self.sim_thread.take() {
            let _ = tokio::task::spawn_blocking(move || t.join()).await;
        }
        for e in std::mem::take(&mut self.endpoints) {
            let _ = e.await;
        }
        info!("server stopped");
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.sim_stop.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
    }
}
