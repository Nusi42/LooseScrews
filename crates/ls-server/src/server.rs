//! Server assembly: configuration, startup and the [`ServerHandle`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::time::Duration;

use ls_game::server::{Room, RoomConfig};
use ls_game::MatchConfig;
use ls_worldgen::{ShareCode, Sliders};
use thiserror::Error;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::info;

use crate::api::Social;
use crate::endpoint::{CertHash, ConnSettings, Endpoint, EndpointContext, Hub};
use crate::http::HttpEndpoint;
use crate::lobby::LobbyTiming;
use crate::party::Parties;
use crate::playlist::{Playlist, WorldChoice};
use crate::rooms::{RoomCode, RoomEntry, RoomStatus, Rooms, SharedStatus, PUBLIC_ROOM};
use crate::rules::ServerRules;
use crate::sim::{self, RoomInfo, SimCommand, SimParams};
use crate::stats::{SharedStats, StatsSnapshot};
use crate::wt::WebTransportEndpoint;

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
    #[error("playlist: {0}")]
    Playlist(String),
    #[error("rooms: {0}")]
    Rooms(String),
    #[error("directory: {0}")]
    Directory(String),
}

/// Everything needed to start a server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// TCP address of HTTP and the WebSocket endpoint.
    pub bind: SocketAddr,
    /// UDP address of the WebTransport endpoint (`None`: WebSocket only).
    pub webtransport: Option<SocketAddr>,
    /// The first match without a `playlist`; the weapon, ship and gadget
    /// tables of every match.
    pub match_cfg: MatchConfig,
    pub room: RoomConfig,
    /// Directory served over HTTP (the built web client); `None` serves a
    /// placeholder page.
    pub web_root: Option<PathBuf>,
    pub conn: ConnSettings,
    /// Interval of the tick statistics log line.
    pub stats_interval: Duration,
    /// Warm-up, countdown and results (`--intermission`) times of every
    /// room (see [`crate::lobby`]).
    pub lobby: LobbyTiming,
    /// Where the rules come from when there is no `playlist`. When set, the
    /// effective rules (rule set, balance list, bans) are rebuilt before
    /// every new match, re-reading the balance list; `None` keeps
    /// `match_cfg.rules` for every match.
    pub rules: Option<ServerRules>,
    /// The public room's rotation. `None`: one entry, `match_cfg`'s world
    /// with `rules` (or `match_cfg.rules`), any player count.
    pub playlist: Option<Playlist>,
    /// Most private rooms at once (capped by the machine's cores, see
    /// [`ServerConfig::private_room_cap`]).
    pub private_rooms: usize,
    /// Extra time every next-room build takes (tests of slow builds).
    pub build_delay: Duration,
    /// Server list and invites: the directory to register with
    /// (`--directory`, `None`: no server list entry), the name and region
    /// shown there and the public `host:port` players reach this server at
    /// (`--public-address`; default: the bound address, `localhost` for an
    /// unspecified one). See [`crate::directory`].
    pub listing: Listing,
}

/// How the server presents itself in the server list and in invites.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    /// Directory URL (`http://host:port[/path]`).
    pub directory: Option<String>,
    /// Server name (`--server-name`, default [`crate::directory::DEFAULT_SERVER_NAME`]).
    pub server_name: Option<String>,
    /// Region tag (`--region`).
    pub region: String,
    /// Public `host:port` (`--public-address`).
    pub public_address: Option<String>,
}

impl ServerConfig {
    /// Defaults: 8 players, 30 Hz snapshots, lag compensation on, stats
    /// every 10 s, no lobby (a match starts when its first player joins;
    /// 10 s of results; the `ls-server` binary turns the lobby on with
    /// [`LobbyTiming::default`]), up to 4 private rooms, no web root, no
    /// WebTransport.
    pub fn new(bind: SocketAddr, match_cfg: MatchConfig) -> Self {
        ServerConfig {
            bind,
            webtransport: None,
            match_cfg,
            room: RoomConfig::default(),
            web_root: None,
            conn: ConnSettings::default(),
            stats_interval: Duration::from_secs(10),
            lobby: LobbyTiming::off(sim::DEFAULT_INTERMISSION),
            rules: None,
            playlist: None,
            private_rooms: crate::rooms::DEFAULT_PRIVATE_ROOMS,
            build_delay: Duration::ZERO,
            listing: Listing::default(),
        }
    }

    /// The public room's playlist (see [`ServerConfig::playlist`]).
    pub fn effective_playlist(&self) -> Playlist {
        self.playlist.clone().unwrap_or_else(|| {
            let rules = self.rules.clone().unwrap_or_else(|| ServerRules {
                base: self.match_cfg.rules.clone(),
                bans: Vec::new(),
                balance_list: None,
            });
            let source = rules.base.name.clone();
            Playlist::single(&source, rules, WorldChoice::Fixed(self.match_cfg.code))
        })
    }

    /// Private rooms allowed on this machine: `private_rooms`, but at most
    /// one per core beyond the public room's (at least one if any are asked
    /// for), so every room's sim thread has a core.
    pub fn private_room_cap(&self) -> usize {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        self.private_rooms.min(cores.saturating_sub(1).max(1))
    }
}

/// Settings of a private room ([`ServerHandle::create_private_room`]).
#[derive(Debug, Clone)]
pub struct PrivateRoomSpec {
    /// What the room plays (typically one entry: a duel or a custom match).
    pub playlist: Playlist,
    /// `None`: the server's lobby timing.
    pub lobby: Option<LobbyTiming>,
    /// `None`: the server's player limit.
    pub max_players: Option<u8>,
}

impl PrivateRoomSpec {
    /// A custom match: `rules` on `world` (a new random world per match if
    /// `None`).
    pub fn custom(rules: ServerRules, world: Option<ShareCode>) -> Self {
        let world = world.map_or_else(WorldChoice::random, WorldChoice::Fixed);
        let source = rules.base.name.clone();
        PrivateRoomSpec {
            playlist: Playlist::single(&source, rules, world),
            lobby: None,
            max_players: None,
        }
    }
}

/// Builds the match configuration from CLI-style inputs: a share code (a
/// fresh random world if `None`) and a rule set file or `LR` code (the
/// built-in Paint the Void rules if `None`). Weapons and ship tuning are the
/// built-in tables, which the clients use too. The rule set is validated
/// against them.
pub fn match_config(
    code: Option<&str>,
    ruleset: Option<&Path>,
) -> Result<MatchConfig, ServerError> {
    let ruleset = ruleset.map(|p| p.to_string_lossy().into_owned());
    let rules = ServerRules::new(ruleset.as_deref(), Vec::new(), None)?;
    match_config_with(code, &rules)
}

/// [`match_config`] with the effective rules of `rules` (rule set, balance
/// list and bans).
pub fn match_config_with(
    code: Option<&str>,
    rules: &ServerRules,
) -> Result<MatchConfig, ServerError> {
    let code = match code {
        Some(text) => ShareCode::decode(text)?,
        None => ShareCode::new(random_seed(), Sliders::default()),
    };
    let mut cfg = MatchConfig::paint_the_void(code);
    cfg.rules = rules.effective(&cfg.weapons, &cfg.gadgets)?;
    Ok(cfg)
}

/// A seed for a random world (wall clock, process id and a counter; not
/// deterministic on purpose, and outside the deterministic crates).
pub(crate) fn random_seed() -> u64 {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    ls_math::rng::mix64(nanos ^ ((std::process::id() as u64) << 32) ^ n.rotate_left(17))
}

/// Starts servers; see [`Server::start`].
#[derive(Debug)]
pub struct Server;

impl Server {
    /// Binds the ports, builds the public room's first room on its sim
    /// thread (waits until it is ready) and starts accepting HTTP, WebSocket
    /// and (if configured) WebTransport connections. With
    /// [`Listing::directory`] it also starts the directory heartbeats.
    pub async fn start(config: ServerConfig) -> Result<ServerHandle, ServerError> {
        let directory = config
            .listing
            .directory
            .as_deref()
            .map(ls_directory::DirectoryClient::new)
            .transpose()
            .map_err(|e| ServerError::Directory(e.to_string()))?;
        let endpoint = HttpEndpoint::bind(config.bind, config.web_root.clone()).await?;
        let local_addr = endpoint.local_addr()?;
        let wt_endpoint = config
            .webtransport
            .map(WebTransportEndpoint::bind)
            .transpose()?;
        let webtransport_addr = wt_endpoint.as_ref().map(|e| e.local_addr()).transpose()?;
        let stats = Arc::new(SharedStats::default());
        let parties = Arc::new(Parties::new(random_seed()));
        let playlist = config.effective_playlist();
        let params = SimParams {
            template: config.match_cfg.clone(),
            room_cfg: config.room,
            stats_interval: config.stats_interval,
            lobby: config.lobby,
            playlist: playlist.clone(),
            label: PUBLIC_ROOM.into(),
            close_when_empty: false,
            build_delay: config.build_delay,
            parties: Some(parties.clone()),
        };
        let (entry, ready_rx) = start_room(params, stats.clone())?;
        let room = match ready_rx.await {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => {
                join_room(&entry).await;
                return Err(ServerError::Room(e));
            }
            Err(_) => {
                join_room(&entry).await;
                return Err(ServerError::SimStopped);
            }
        };
        let rooms = Arc::new(Rooms::new(entry, config.private_room_cap()));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let config = Arc::new(config);
        let factory = RoomFactory {
            config: config.clone(),
            rooms: rooms.clone(),
            stats: stats.clone(),
            parties: parties.clone(),
            code_counter: AtomicU64::new(random_seed()),
        };
        let cert_hash: CertHash = Arc::new(RwLock::new(None));
        let social = Arc::new(Social::new(
            factory,
            parties,
            crate::directory::public_address(&config.listing, local_addr),
            directory.clone(),
            cert_hash.clone(),
        ));
        let mut handle = ServerHandle {
            local_addr,
            webtransport_addr,
            room,
            stats: stats.clone(),
            hub: Hub::with_rooms(rooms.clone(), stats, config.conn).with_social(social.clone()),
            rooms,
            social: social.clone(),
            config: config.clone(),
            shutdown_tx,
            shutdown_rx,
            endpoints: Vec::new(),
            cert_hash,
        };
        handle.spawn_endpoint(Box::new(endpoint));
        if let Some(wt) = wt_endpoint {
            handle.spawn_endpoint(Box::new(wt));
        }
        if let Some(client) = directory {
            let info = social.clone();
            let task = ls_directory::spawn_heartbeats(
                client,
                ls_directory::new_secret(),
                move || info.server_info(),
                handle.shutdown_rx.clone(),
            );
            handle.endpoints.push(task);
        }
        info!(
            webtransport = ?webtransport_addr,
            addr = %local_addr,
            share_code = %handle.room.share_code,
            ruleset = %handle.room.ruleset,
            rules_code = %handle.room.rules_code,
            max_players = config.room.max_players,
            playlist_entries = playlist.len(),
            private_rooms = handle.rooms.max_private(),
            lobby = !config.lobby.is_off(),
            public_address = %social.address(),
            directory = config.listing.directory.as_deref().unwrap_or("none"),
            "listening (http + websocket at /ws)"
        );
        Ok(handle)
    }
}

/// Spawns a room's sim thread; the receiver reports when its first room is
/// built.
fn start_room(
    params: SimParams,
    stats: Arc<SharedStats>,
) -> std::io::Result<(RoomEntry, oneshot::Receiver<Result<RoomInfo, String>>)> {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let (ready_tx, ready_rx) = oneshot::channel();
    let status = SharedStatus::default();
    let thread = sim::spawn(
        params,
        cmd_rx,
        stop.clone(),
        stats.clone(),
        status.clone(),
        ready_tx,
    )?;
    let entry = RoomEntry {
        commands: cmd_tx,
        stats,
        status,
        stop,
        thread: Mutex::new(Some(thread)),
    };
    Ok((entry, ready_rx))
}

/// Stops a room's sim thread and waits for it.
async fn join_room(entry: &RoomEntry) {
    if let Some(t) = entry.stop() {
        let _ = tokio::task::spawn_blocking(move || t.join()).await;
    }
}

/// Opens private rooms (for the [`ServerHandle`] and the HTTP API).
#[derive(Debug)]
pub struct RoomFactory {
    config: Arc<ServerConfig>,
    rooms: Arc<Rooms>,
    stats: Arc<SharedStats>,
    parties: Arc<Parties>,
    /// Source of private room codes.
    code_counter: AtomicU64,
}

impl RoomFactory {
    /// The server's configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// The room registry.
    pub fn rooms(&self) -> &Arc<Rooms> {
        &self.rooms
    }

    /// See [`ServerHandle::create_private_room`].
    pub async fn create(&self, spec: PrivateRoomSpec) -> Result<RoomCode, ServerError> {
        let mut room_cfg = self.config.room;
        if let Some(n) = spec.max_players {
            room_cfg.max_players = n;
        }
        let mut ready = None;
        let counter = &self.code_counter;
        let code = self.rooms.insert_private(
            || RoomCode::from_bits(ls_math::rng::mix64(counter.fetch_add(1, Ordering::Relaxed))),
            |code| {
                let params = SimParams {
                    template: self.config.match_cfg.clone(),
                    room_cfg,
                    stats_interval: self.config.stats_interval,
                    lobby: spec.lobby.unwrap_or(self.config.lobby),
                    playlist: spec.playlist.clone(),
                    label: code.to_string(),
                    close_when_empty: true,
                    build_delay: self.config.build_delay,
                    parties: Some(self.parties.clone()),
                };
                let stats = Arc::new(SharedStats {
                    links: self.stats.links.clone(),
                    ..SharedStats::default()
                });
                let (entry, rx) = start_room(params, stats)?;
                ready = Some(rx);
                Ok(entry)
            },
        )?;
        let built = match ready {
            Some(rx) => rx.await,
            None => return Err(ServerError::SimStopped),
        };
        match built {
            Ok(Ok(info)) => {
                info!(
                    room = %code,
                    share_code = %info.share_code,
                    ruleset = %info.ruleset,
                    max_players = room_cfg.max_players,
                    private_rooms = self.rooms.private_count(),
                    "private room open"
                );
                Ok(code)
            }
            other => {
                if let Some(t) = self.rooms.remove_private(&code) {
                    let _ = tokio::task::spawn_blocking(move || t.join()).await;
                }
                Err(match other {
                    Ok(Err(e)) => ServerError::Room(e),
                    _ => ServerError::SimStopped,
                })
            }
        }
    }
}

/// A running server. Dropping it stops the sim threads and the endpoints
/// without waiting; [`ServerHandle::shutdown`] stops them gracefully.
#[derive(Debug)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    webtransport_addr: Option<SocketAddr>,
    room: RoomInfo,
    stats: Arc<SharedStats>,
    hub: Hub,
    rooms: Arc<Rooms>,
    social: Arc<Social>,
    config: Arc<ServerConfig>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    endpoints: Vec<JoinHandle<()>>,
    cert_hash: CertHash,
}

impl ServerHandle {
    /// The bound address (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The bound UDP address of the WebTransport endpoint, if enabled.
    pub fn webtransport_addr(&self) -> Option<SocketAddr> {
        self.webtransport_addr
    }

    /// The first match's share code, as printed (later matches of a
    /// rotation have their own; see [`ServerHandle::rooms`]).
    pub fn share_code(&self) -> &str {
        &self.room.share_code
    }

    pub fn ruleset_name(&self) -> &str {
        &self.room.ruleset
    }

    /// Rule set code of the first match's effective rules (empty if the
    /// rules cannot be encoded as a code). It is not updated for later
    /// matches; [`ServerHandle::rooms`] and the log line at each match start
    /// show the current code.
    pub fn rules_code(&self) -> &str {
        &self.room.rules_code
    }

    /// Current counters of the public room (the connection counters are
    /// server-wide).
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// Players connected over all rooms.
    pub fn total_players(&self) -> u64 {
        self.rooms.total_players()
    }

    /// Status and counters of every room, the public one first.
    pub fn rooms(&self) -> Vec<(RoomStatus, StatsSnapshot)> {
        self.rooms.statuses()
    }

    /// The hub endpoints use to register connections.
    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// Where the WebTransport endpoint publishes its certificate hash.
    pub fn cert_hash(&self) -> &CertHash {
        &self.cert_hash
    }

    /// Starts another endpoint.
    pub fn spawn_endpoint(&mut self, endpoint: Box<dyn Endpoint>) {
        let ctx = EndpointContext {
            hub: self.hub.clone(),
            shutdown: self.shutdown_rx.clone(),
            cert_hash: self.cert_hash.clone(),
        };
        info!(endpoint = endpoint.name(), "endpoint started");
        self.endpoints.push(endpoint.spawn(ctx));
    }

    /// Runs `f` on the public room's sim thread against its current room at
    /// the start of the next tick and returns its result.
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

    /// Opens a private room on its own sim thread and returns its code once
    /// its first room is built. Players join it with the code in their
    /// `Hello` (see [`crate::rooms::requested_room`]); it closes when empty.
    /// Fails while [`ServerConfig::private_room_cap`] rooms are open.
    pub async fn create_private_room(
        &self,
        spec: PrivateRoomSpec,
    ) -> Result<RoomCode, ServerError> {
        self.social.factory().create(spec).await
    }

    /// The configuration the server started with.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Parties, duels and presence (the `/api` endpoints' state).
    pub fn social(&self) -> &Arc<Social> {
        &self.social
    }

    /// The public `host:port` this server announces (directory, invites).
    pub fn public_address(&self) -> &str {
        self.social.address()
    }

    /// Graceful shutdown: players get `Disconnect`, sockets flush and close,
    /// then the sim threads and endpoints are joined.
    pub async fn shutdown(mut self) {
        info!("shutting down");
        let threads = self.rooms.stop_all();
        let _ = self.shutdown_tx.send(true);
        for t in threads {
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
        self.rooms.signal_stop();
        let _ = self.shutdown_tx.send(true);
    }
}
