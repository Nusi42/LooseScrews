//! Headless bot clients for Loose Screws load and soak tests
//! (`docs/implementation-plan.md` section 4.7).
//!
//! Each bot is a real client: it connects over WebSocket, runs
//! `ls_game`'s [`ClientSession`](ls_game::client::ClientSession) (prediction,
//! reconciliation, interpolation, terrain replication) at 60 Hz and plays
//! with simple behaviors.
//!
//! - [`steering`]: wall avoidance through `TerrainQuery` (SDF and raycasts),
//!   velocity-to-thrust conversion, look control, open-space targets.
//! - [`aim`]: projectile lead, fire tolerance, line of sight.
//! - [`brain`]: the behavior (scavenge / combat / paint and wander).
//! - [`bot`]: [`BotCore`], one bot's session + brain + desync checks, no I/O.
//! - [`net`]: the tokio-tungstenite runner with reconnects.
//! - [`stats`]: per-bot counters, the exit summary and exit code.

pub mod aim;
pub mod bot;
pub mod brain;
pub mod net;
pub mod stats;
pub mod steering;

pub use bot::{BotCore, BotError, Flow};
pub use brain::{Brain, BrainConfig, Perception};
pub use net::{run_bot, run_bot_blocking, BotConfig, StopSignal};
pub use stats::{exit_code, summary, BotStats, DisconnectPolicy};

/// Default server URL the bots connect to (the server's WebSocket endpoint).
pub const DEFAULT_SERVER: &str = "ws://127.0.0.1:7777/ws";
