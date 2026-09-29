//! Error type of the game glue.

use thiserror::Error;

/// Errors from building matches, converting network data and running sessions.
#[derive(Debug, Error)]
pub enum GameError {
    #[error("world generation: {0}")]
    Worldgen(#[from] ls_worldgen::WorldgenError),
    #[error("simulation: {0}")]
    Sim(#[from] ls_sim::SimError),
    #[error("encode: {0}")]
    Encode(#[from] ls_net::EncodeError),
    #[error("decode: {0}")]
    Decode(#[from] ls_net::DecodeError),
    #[error("match setup: {0}")]
    Setup(String),
    /// The peer sent something that breaks the protocol (unexpected message,
    /// unknown client, bad rule set text).
    #[error("protocol: {0}")]
    Protocol(String),
    /// A snapshot's terrain edits start after the edits this peer has applied,
    /// so some edits were missed. The client must rejoin (fresh `Welcome`).
    #[error("terrain edit gap: have {have} edits, snapshot starts at {base}")]
    EditGap { have: u32, base: u32 },
}
