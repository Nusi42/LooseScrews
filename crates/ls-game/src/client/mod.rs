//! Client core (pure logic, shared by the WASM client and the bots).
//!
//! - [`InputSequencer`]: consecutive input ticks, quantized commands,
//!   redundant input messages.
//! - [`Predictor`]: local ship prediction, reconciliation, error smoothing.
//! - [`InterpolationBuffer`]: remote ships rendered ~100 ms in the past.
//! - [`ClientClock`]: input lead over the server and server time estimate.
//! - [`ClientSession`]: all of the above plus the local [`crate::GameTerrain`]
//!   kept in sync with snapshot terrain edits.

pub mod clock;
pub mod input;
pub mod interp;
pub mod prediction;
pub mod session;

pub use clock::ClientClock;
pub use input::InputSequencer;
pub use interp::{InterpolationBuffer, INTERP_DELAY_TICKS};
pub use prediction::{PredictionConfig, PredictionStats, Predictor, Reconcile};
pub use session::{ClientSession, Incoming, SessionStats, GUIDED_STATE_BIT};
