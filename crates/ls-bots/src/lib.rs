//! Headless bot clients for Loose Screws load and soak tests.
//!
//! Planned contents (see `docs/implementation-plan.md` section 4.7): bots that
//! reuse `ls-net` and `ls-sim` prediction with simple behaviors (wander, aim at
//! the nearest enemy, paint walls).

/// Default server URL the bots connect to.
pub const DEFAULT_SERVER: &str = "ws://127.0.0.1:7777";
