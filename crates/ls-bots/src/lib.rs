//! Headless bot clients for Loose Screws load and soak tests
//! (`docs/implementation-plan.md` section 4.7).
//!
//! Each bot is a real client: it connects over WebSocket or WebTransport, runs
//! `ls_game`'s [`ClientSession`](ls_game::client::ClientSession) (prediction,
//! reconciliation, interpolation, terrain replication) at 60 Hz and plays
//! with simple behaviors.
//!
//! - [`steering`]: wall avoidance through `TerrainQuery` (SDF and raycasts),
//!   velocity-to-thrust conversion, look control, open-space targets.
//! - [`aim`]: projectile lead, fire tolerance, line of sight.
//! - [`arsenal`]: every weapon as a [`Gun`] with its [`Role`] and the
//!   distance bands the bots use it in.
//! - [`nav`]: global navigation: coarse open-space grid from the terrain SDF,
//!   A* and a route follower (hunting enemies across cave levels).
//! - [`nav_cache`]: one grid build per world per process, on a worker
//!   thread, shared by the bots of a match.
//! - [`brain`]: the behavior (guide / flee / scavenge / combat / collect /
//!   hunt / paint and wander) with weapon, gadget and threat handling.
//! - [`bot`]: [`BotCore`], one bot's session + brain + desync checks, no I/O.
//! - [`net`]: the runner with reconnects.
//! - [`transport`]: WebSocket (`tokio-tungstenite`) and WebTransport
//!   (`wtransport`, trusting the server's self-signed certificate by the hash
//!   from `/cert-hash`) behind one connection type.
//! - [`stats`]: per-bot counters, the exit summary and exit code.

pub mod aim;
pub mod arsenal;
pub mod bot;
pub mod brain;
pub mod nav;
pub mod nav_cache;
pub mod net;
pub mod stats;
pub mod steering;
pub mod transport;

pub use arsenal::{Arsenal, Gun, Role};
pub use bot::{bot_hello, BotCore, BotError, Flow};
pub use brain::{Brain, BrainConfig, Perception};
pub use net::{run_bot, run_bot_blocking, BotConfig, StopSignal};
pub use stats::{exit_code, summary, BotStats, DisconnectPolicy, WeaponUse};
pub use transport::TransportKind;

/// Default server URL the bots connect to (the server's WebSocket endpoint).
pub const DEFAULT_SERVER: &str = "ws://127.0.0.1:7777/ws";
