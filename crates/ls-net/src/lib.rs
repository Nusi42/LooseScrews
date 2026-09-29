//! Wire protocol for Loose Screws (implementation plan section 4.4).
//!
//! - [`bits`]: hand-written bit writer and reader (no generic serializer on
//!   the hot path).
//! - [`quant`]: quantization of positions (3 x 16 bit in level bounds),
//!   velocities (3 x 16 bit) and orientations (smallest-three, 32 bit).
//! - [`wire`]: wire-level structs ([`NetShip`], [`NetProjectile`],
//!   [`NetInput`], [`NetEvent`], [`Snapshot`]) and their packed forms. They
//!   do not depend on `ls-sim`; the server and client convert.
//! - [`snapshot`]: full and delta snapshot encoding, [`SnapshotHistory`].
//! - [`message`]: every protocol [`Message`], the handshake and version checks.
//! - [`transport`]: the poll-based [`Transport`] trait (native and wasm32)
//!   and an in-memory implementation for tests and bots.
//!
//! A ship's core state is exactly 18 bytes, so 32 ships at 30 Hz need
//! 32 x 18 x 30 = 17,280 B/s before delta compression.

pub mod bits;
pub mod error;
pub mod message;
pub mod quant;
pub mod snapshot;
pub mod transport;
pub mod wire;

pub use ls_math;

pub use error::{DecodeError, EncodeError, HandshakeError, TransportError};
pub use message::{
    accept_hello, check_hello, check_welcome, peek_hello_version, Channel, Hello, InputMsg,
    Message, Reject, RejectReason, SnapshotMsg, Welcome, MAGIC, PROTOCOL_VERSION,
};
pub use quant::{pack_quat, unpack_quat, PackedVec3, QuantConfig, Quantizer};
pub use snapshot::{Baselines, NoBaselines, SnapshotHistory};
pub use transport::{
    recv_message, send_message, LinkConditions, MemoryTransport, NetError, Packet, Transport,
};
pub use wire::{
    buttons, ship_flags, NetEvent, NetInput, NetProjectile, NetShip, PackedProjectile, PackedShip,
    PackedSnapshot, ShipStatus, Snapshot, TerrainEditBits, MAX_SHIPS, SHIP_CORE_BITS,
};
