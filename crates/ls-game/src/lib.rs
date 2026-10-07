//! Shared game glue for Loose Screws, used by `ls-server`, `ls-wasm` and
//! `ls-bots`. Pure logic, no I/O: callers own sockets, clocks and threads.
//!
//! - [`GameTerrain`]: the level's [`ls_worldgen::ChunkStore`] as
//!   [`ls_math::terrain::TerrainQuery`] + [`ls_sim::SimTerrain`], with an edit
//!   log (sequence numbers for the wire) and dirty-chunk tracking for remeshing.
//! - [`setup`]: deterministic spawn and pickup points from a share code and a
//!   rule set, and [`setup::build_match`], which returns a [`ls_sim::World`] and
//!   its terrain ready to step.
//! - [`convert`]: `ls-sim` <-> `ls-net` conversions ([`convert::NetConv`]) with
//!   the quantization derived from the level bounds.
//! - [`client`]: input sequencing, prediction with reconciliation and error
//!   smoothing, interpolation of remote ships, clock and a complete
//!   [`client::ClientSession`].
//! - [`server`] (feature `server`, on by default): the authoritative
//!   [`server::Room`] with input queues, snapshots, deltas and lag compensation.
//!
//! See `docs/architecture/game-glue.md`.

pub mod client;
pub mod convert;
pub mod error;
pub mod mode;
#[cfg(feature = "server")]
pub mod server;
pub mod setup;
pub mod terrain;

pub use convert::NetConv;
pub use error::GameError;
pub use setup::{build_match, build_match_lazy, match_setup, MatchConfig};
pub use terrain::GameTerrain;

/// The `Hello` a client sends: name, preferred team and ship type, and the
/// built-in game data hash (`ls_sim::data_hash`), which the server checks.
pub fn client_hello(
    name: impl Into<String>,
    preferred_team: Option<u8>,
    ship_type: Option<ls_sim::ShipType>,
) -> ls_net::Hello {
    ls_net::Hello::new(name, preferred_team)
        .with_data_hash(ls_sim::data_hash())
        .with_ship_type(ship_type.map(|t| t.index()))
}

/// `Disconnect` reason a host sends every client when it replaces a finished
/// match with a new one. Clients treat it as "rejoin", not as an error: a new
/// connection gets the new match's `Welcome`.
pub const MATCH_RESTART_REASON: &str = "match over; a new match is starting";

pub use ls_math;
pub use ls_net;
pub use ls_sim;
pub use ls_worldgen;
