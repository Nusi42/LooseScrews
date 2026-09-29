//! Error types for encoding, decoding, handshakes and transports.

use thiserror::Error;

/// Failure while turning a message into bytes. Only produced by invalid input
/// (the encoder never fails on well-formed values).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EncodeError {
    #[error("value out of range: {0}")]
    OutOfRange(&'static str),
    #[error("ships must be sorted by id, unique and below {max}")]
    BadShipList { max: u8 },
    #[error("projectiles must be sorted by id and unique")]
    BadProjectileList,
    #[error("input message needs 1..=3 commands with strictly decreasing ticks at most 255 apart")]
    BadInputList,
    #[error("baseline snapshot for tick {0} is not available")]
    MissingBaseline(u32),
    #[error("baseline tick {baseline} is not 1..=255 ticks before snapshot tick {tick}")]
    BaselineTooOld { tick: u32, baseline: u32 },
    #[error("ship flag bit 15 is reserved for the wire format")]
    ReservedFlag,
}

/// Failure while parsing bytes. Every decode path bounds-checks, so hostile
/// input yields one of these instead of a panic or a huge allocation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error("unexpected end of packet")]
    UnexpectedEnd,
    #[error("unknown message tag {0}")]
    UnknownTag(u8),
    #[error("bad handshake magic")]
    BadMagic,
    #[error("protocol version mismatch: ours {ours}, theirs {theirs}")]
    VersionMismatch { ours: u16, theirs: u16 },
    #[error("invalid value: {0}")]
    Invalid(&'static str),
    #[error("length {len} exceeds limit {max}")]
    TooLong { len: u64, max: u64 },
    #[error("baseline snapshot for tick {0} is not available")]
    MissingBaseline(u32),
    #[error("{0} trailing bytes after message")]
    TrailingBytes(usize),
}

/// Failure of a handshake check (see [`crate::message::check_hello`]).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HandshakeError {
    #[error("protocol version mismatch: ours {ours}, theirs {theirs}")]
    VersionMismatch { ours: u16, theirs: u16 },
    #[error("invalid handshake field: {0}")]
    Invalid(&'static str),
    #[error("expected {expected} during handshake")]
    Unexpected { expected: &'static str },
    #[error("rejected by server: {0}")]
    Rejected(String),
}

/// Failure reported by a [`crate::transport::Transport`] implementation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("connection closed")]
    Closed,
    #[error("packet of {len} bytes exceeds the {max} byte limit for this channel")]
    TooLarge { len: usize, max: usize },
    #[error("transport error: {0}")]
    Other(String),
}
