//! Authoritative Loose Screws server (implementation plan section 4.5).
//!
//! The game logic is `ls_game::server::Room`; this crate hosts it:
//!
//! - [`sim`]: a dedicated thread that owns the room and steps it at a fixed
//!   60 Hz ([`timing::TickScheduler`]: deadline based, bursts of up to 4
//!   ticks to catch up, at most 250 ms of backlog), 30 Hz snapshots, and
//!   logs p50/p99/max tick time plus input counters every 10 s.
//! - [`link`]: [`link::SimLink`] implements [`ls_net::Transport`] on the sim
//!   thread; [`link::PeerLink`] is the socket side. Unreliable frames are
//!   dropped under backpressure, reliable ones never.
//! - [`endpoint`]: the transport seam ([`endpoint::Endpoint`],
//!   [`endpoint::Hub`]). [`http::HttpEndpoint`] serves static files and
//!   `GET /cert-hash` and upgrades `/ws` to a WebSocket ([`ws`]);
//!   [`wt::WebTransportEndpoint`] accepts WebTransport sessions on a UDP port
//!   (datagrams for unreliable packets, a framed stream for reliable ones).
//! - [`rules`]: match rules from the command line (rule set file or code,
//!   bans, a balance list re-read before every match).
//! - [`server`]: [`ServerConfig`], [`Server::start`] and [`ServerHandle`]
//!   (graceful shutdown, stats, room inspection).
//!
//! The binary in `main.rs` is a thin CLI around this library.

pub mod endpoint;
pub mod http;
pub mod link;
pub mod rules;
pub mod server;
pub mod sim;
pub mod stats;
pub mod timing;
pub mod ws;
pub mod wt;

pub use endpoint::{CertHashInfo, ConnSettings, Endpoint, EndpointContext, Hub};
pub use rules::{load_rule_set, ServerRules};
pub use server::{
    match_config, match_config_with, Server, ServerConfig, ServerError, ServerHandle,
};
pub use stats::StatsSnapshot;

/// Default bind address for the game server.
pub const DEFAULT_BIND: &str = "0.0.0.0:7777";

/// Default web root (the built web client), relative to the repository root.
pub const DEFAULT_WEB_ROOT: &str = "web/dist";
