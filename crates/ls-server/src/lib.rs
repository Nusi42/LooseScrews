//! Authoritative Loose Screws server library.
//!
//! Planned contents (see `docs/implementation-plan.md` section 4.5): the fixed
//! 60 Hz simulation loop, rooms, WebSocket and WebTransport endpoints, and lag
//! compensation. The binary in `main.rs` is a thin CLI around this library.

/// Default bind address for the game server.
pub const DEFAULT_BIND: &str = "0.0.0.0:7777";
