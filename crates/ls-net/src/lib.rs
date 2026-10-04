//! Wire protocol for Loose Screws (implementation plan section 4.4).
//!
//! - [`bits`]: hand-written bit writer and reader (no generic serializer on
//!   the hot path).
//! - [`quant`]: quantization of positions (3 x 16 bit in level bounds),
//!   velocities (3 x 16 bit) and orientations (smallest-three, 32 bit).
//! - [`wire`]: wire-level structs ([`NetShip`], [`NetProjectile`],
//!   [`NetEntity`], [`NetPlayer`], [`NetInput`], [`NetEvent`], [`Snapshot`])
//!   and their packed forms, plus the per-client [`Relevance`] hook. They do
//!   not depend on `ls-sim`; the server and client convert.
//! - [`snapshot`]: full and delta snapshot encoding (ship deltas as
//!   residuals against motion predictions), [`SnapshotHistory`].
//! - [`message`]: every protocol [`Message`], the handshake and version checks.
//! - [`transport`]: the poll-based [`Transport`] trait (native and wasm32)
//!   and an in-memory implementation for tests and bots.
//!
//! A ship's core state is exactly 18 bytes, so 32 ships at 30 Hz need
//! 32 x 18 x 30 = 17,280 B/s before delta compression. Delta snapshots with
//! 32 thrusting, turning ships spend about 290 to 340 B on them (baselines 2
//! to 6 ticks old), leaving room for entities in the 17 KB/s budget.

pub mod bits;
pub mod error;
pub mod message;
pub mod mode;
pub mod quant;
pub mod snapshot;
pub mod transport;
pub mod wire;

pub use ls_math;

pub use error::{DecodeError, EncodeError, HandshakeError, TransportError};
pub use message::{
    accept_hello, check_data_hash, check_hello, check_welcome, peek_hello_version, Channel, Hello,
    InputMsg, InventoryMsg, Message, NetWeapon, Reject, RejectReason, SnapshotMsg, Welcome, MAGIC,
    PROTOCOL_VERSION,
};
pub use mode::{
    mode_phase, mode_player_flags, LobbyPhase, LobbyPlayer, LobbyStateMsg, MatchResultsMsg,
    NetModePlayer, NetModeState, NetObjective, PackedModeState, PackedObjective, ResultRow,
    MAX_MODE_TEAMS, MAX_OBJECTIVES,
};
pub use quant::{
    pack_quat, unpack_quat, PackedVec3, QuantConfig, Quantizer, ENTITY_POS_MAX, ENTITY_POS_SHIFT,
    MOTION_SCALE_SHIFT, TICKS_PER_SECOND,
};
pub use snapshot::{settle_projectiles, Baselines, NoBaselines, SnapshotHistory, SETTLE_TOLERANCE};
pub use transport::{
    recv_message, send_message, LinkConditions, MemoryTransport, NetError, Packet, Transport,
};
pub use wire::{
    buttons, ship_flags, AllRelevant, NetEntity, NetEvent, NetInput, NetPlayer, NetProjectile,
    NetShip, PackedEntity, PackedProjectile, PackedShip, PackedSnapshot, Relevance, ShipStatus,
    Snapshot, TerrainEditBits, MAX_PLAYER_NAME, MAX_SHIPS, SHIP_CORE_BITS,
};
