//! Protocol messages, their encoding, and handshake checks.
//!
//! Every packet is one message: an 8-bit tag followed by the bit-packed body.
//!
//! # Handshake
//!
//! 1. Client sends [`Hello`] (reliable). Its first fields (tag, [`MAGIC`],
//!    protocol version) never change between versions, so a server can always
//!    detect a version mismatch ([`DecodeError::VersionMismatch`]) and answer
//!    with [`Reject`], whose layout is also frozen.
//!    `Hello` also carries the client's game data hash (`ls_sim::data_hash`)
//!    and preferred ship type; a server with different data answers
//!    [`RejectReason::DataMismatch`].
//! 2. Server answers [`Welcome`] (share code, rule set, player id, team,
//!    ship type, data hash, quantization config, full terrain edit log) or
//!    [`Reject`].
//! 3. Then the client streams [`InputMsg`] (unreliable, 60 Hz, each carrying
//!    the last three commands) and the server streams snapshots (unreliable,
//!    30 Hz), [`Message::Events`] (reliable) and answers pings. A client
//!    picks its ship type for the next respawn with [`Message::SelectShip`].
//!    The server sends a client its own ship's carried weapons, ammo, levels
//!    and gadget with [`Message::Inventory`] (reliable) whenever they change.
//! 4. Match transition (protocol 5): when the next match starts, the server
//!    sends a new [`Welcome`] on the open connection. Its `epoch` differs
//!    from the previous match's; the client rebuilds its world and stamps
//!    its inputs with the new epoch, and both ends drop inputs and snapshots
//!    of another epoch (unreliable packets may still be in flight).
//!    Between matches the server sends [`Message::LobbyState`] and
//!    [`Message::MatchResults`]; clients answer with [`Message::Ready`],
//!    [`Message::PickTeam`] and [`Message::Spectate`].

use ls_math::{Aabb, TerrainEdit, Vec3};

use crate::bits::{BitReader, BitWriter};
use crate::error::{DecodeError, EncodeError, HandshakeError};
use crate::mode::{
    read_lobby, read_results, write_lobby, write_results, LobbyStateMsg, MatchResultsMsg,
    MAX_CODE_LEN,
};
use crate::quant::QuantConfig;
use crate::snapshot::{check_edit, read_snapshot, write_snapshot, Baselines, NoBaselines};
use crate::wire::{
    read_edit, write_edit, NetEvent, NetInput, PackedSnapshot, TerrainEditBits, EDIT_MIN_BITS,
    EVENT_MIN_BITS, MAX_SHIPS,
};

/// Bumped whenever the wire format changes incompatibly (2: M2 entities,
/// players, cues, data hash, ship choice; 3: predictive ship deltas with
/// the snapshot's ship motion scale; 4: paint colours in paint edits,
/// `Welcome::edit_count` (pruned edit logs), predictive projectile deltas
/// with the snapshot's projectile motion scale; 5: M3 modes and lobby:
/// mode state in snapshots, `Welcome` on an open connection with a match
/// epoch on inputs and snapshots, lobby and results messages, room and
/// party codes in `Hello`, weapon shots and state in `Inventory`).
pub const PROTOCOL_VERSION: u16 = 5;
/// First bytes of a [`Hello`] body: ASCII `LSCR` when read as little-endian.
pub const MAGIC: u32 = u32::from_le_bytes(*b"LSCR");

/// Longest player name in bytes.
pub const MAX_NAME_LEN: usize = 32;
/// Longest share code string in bytes (codes are at most 26 characters).
pub const MAX_SHARE_CODE_LEN: usize = 64;
/// Longest rule set name in bytes.
pub const MAX_RULESET_NAME_LEN: usize = 64;
/// Largest rule set payload (RON text) in bytes.
pub const MAX_RULESET_BYTES: usize = 64 * 1024;
/// Most edits a `Welcome` can carry.
pub const MAX_EDIT_LOG: usize = 1 << 18;
/// Longest reject or disconnect reason text in bytes.
pub const MAX_REASON_LEN: usize = 256;
/// Most events in one `Events` message.
pub const MAX_EVENTS: usize = 256;
/// Most redundant commands in one `Input` message.
pub const MAX_INPUT_CMDS: usize = 3;

/// Which transport channel a message should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// May be lost, duplicated or reordered (WebTransport datagram, UDP).
    Unreliable,
    /// Delivered once, in order (WebSocket, WebTransport stream).
    Reliable,
}

/// Client's opening message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// Always [`PROTOCOL_VERSION`] when built with [`Hello::new`].
    pub protocol_version: u16,
    pub name: String,
    /// Team the player would like, `None` for auto-balance.
    pub preferred_team: Option<u8>,
    /// Hash of the client's game data (`ls_sim::data_hash`); the server
    /// rejects a mismatch. `ls_game::client_hello` fills it in.
    pub data_hash: u64,
    /// Ship type index the player would like (`None` = server default).
    pub ship_type: Option<u8>,
    /// Private room to join (`None` = the public room).
    pub room: Option<String>,
    /// Party code (`P-XXXXXX`) to join with.
    pub party: Option<String>,
}

impl Hello {
    /// A hello without a data hash (0) or ship preference; use
    /// [`Hello::with_data_hash`] (or `ls_game::client_hello`).
    pub fn new(name: impl Into<String>, preferred_team: Option<u8>) -> Self {
        Hello {
            protocol_version: PROTOCOL_VERSION,
            name: name.into(),
            preferred_team,
            data_hash: 0,
            ship_type: None,
            room: None,
            party: None,
        }
    }

    pub fn with_data_hash(mut self, hash: u64) -> Self {
        self.data_hash = hash;
        self
    }

    pub fn with_ship_type(mut self, ship_type: Option<u8>) -> Self {
        self.ship_type = ship_type;
        self
    }

    /// Asks for a private room by code.
    pub fn with_room(mut self, room: Option<String>) -> Self {
        self.room = room;
        self
    }

    /// Joins with a party code.
    pub fn with_party(mut self, party: Option<String>) -> Self {
        self.party = party;
        self
    }
}

/// Server's acceptance: everything a client needs to build the world and
/// join the match.
#[derive(Debug, Clone, PartialEq)]
pub struct Welcome {
    pub protocol_version: u16,
    /// Ship id of this player (`0..MAX_SHIPS`).
    pub player_id: u8,
    pub team: u8,
    /// Ship type index the player spawned with.
    pub ship_type: u8,
    /// The server's game data hash (equal to the client's, or it would have
    /// been rejected).
    pub data_hash: u64,
    /// World share code, e.g. `LS-...`; the client regenerates the terrain from it.
    pub share_code: String,
    /// Name of the rule set (e.g. `paint_the_void`).
    pub ruleset_name: String,
    /// The rule set itself (RON text), so custom rule sets need no download.
    pub ruleset: Vec<u8>,
    /// Quantization ranges used by all snapshots of this match.
    pub quant: QuantConfig,
    /// Server tick at the time of sending, for clock sync.
    pub server_tick: u32,
    /// The terrain edits so far that still matter, in order: the server
    /// leaves out surface-state edits that expired before `server_tick`
    /// without changing what the log builds (see
    /// `ls_game::GameTerrain::welcome_log`).
    pub edit_log: Vec<TerrainEdit>,
    /// Number of edits so far (`>= edit_log.len()`): the sequence number of
    /// the next edit. Travels as the number of left-out edits.
    pub edit_count: u32,
    /// Match epoch: changes with every match on a connection. Inputs and
    /// snapshots carry it, so packets of the previous match still in
    /// flight are dropped after a transition.
    pub epoch: u8,
    /// Code of the room this match runs in (empty for the public room).
    pub room_code: String,
}

/// Why the server refused or dropped a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    VersionMismatch,
    ServerFull,
    BadHandshake,
    Kicked,
    Other,
    /// The client was built with different game data (`data/` files).
    DataMismatch,
}

impl RejectReason {
    fn to_u8(self) -> u8 {
        match self {
            RejectReason::VersionMismatch => 0,
            RejectReason::ServerFull => 1,
            RejectReason::BadHandshake => 2,
            RejectReason::Kicked => 3,
            RejectReason::Other => 4,
            RejectReason::DataMismatch => 5,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => RejectReason::VersionMismatch,
            1 => RejectReason::ServerFull,
            2 => RejectReason::BadHandshake,
            3 => RejectReason::Kicked,
            5 => RejectReason::DataMismatch,
            _ => RejectReason::Other,
        }
    }
}

/// Refusal. Its layout is frozen across protocol versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reject {
    pub reason: RejectReason,
    /// The server's protocol version, so the client can tell the user.
    pub server_version: u16,
    pub message: String,
}

/// Client input: the newest command plus up to two older ones, repeated so a
/// lost datagram rarely costs a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputMsg {
    /// Match epoch of the `Welcome` the commands belong to.
    pub epoch: u8,
    /// Tick of the newest snapshot the client has received and stored; the
    /// server uses it as the delta baseline.
    pub ack_snapshot: Option<u32>,
    /// 1..=3 commands, newest first, ticks strictly decreasing and at most
    /// 255 apart from their predecessor.
    pub cmds: Vec<NetInput>,
}

/// A snapshot, optionally delta-encoded against an earlier one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMsg {
    /// Match epoch (see [`Welcome::epoch`]).
    pub epoch: u8,
    /// Tick of the baseline this snapshot is (or should be) encoded against.
    /// When encoding, the baseline must be available from the `Baselines`
    /// passed in; after decoding it tells which baseline was used.
    pub baseline: Option<u32>,
    pub snapshot: PackedSnapshot,
}

/// Every message of the protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Hello(Hello),
    Welcome(Box<Welcome>),
    Reject(Reject),
    Input(InputMsg),
    Snapshot(Box<SnapshotMsg>),
    Events(Vec<NetEvent>),
    Ping {
        nonce: u32,
        client_time_ms: u32,
    },
    Pong {
        nonce: u32,
        client_time_ms: u32,
        server_tick: u32,
    },
    Disconnect {
        reason: String,
    },
    /// Client: fly this ship type (index) from the next respawn on.
    SelectShip {
        ship_type: u8,
    },
    /// Server: the receiving client's own inventory (sent when it changes).
    Inventory(InventoryMsg),
    /// Server: lobby state (on every change and on join).
    LobbyState(Box<LobbyStateMsg>),
    /// Server: the result of the match that just ended.
    MatchResults(Box<MatchResultsMsg>),
    /// Client: ready (or no longer ready) for the next match.
    Ready {
        ready: bool,
    },
    /// Client: team for the next match (`None` = auto-balance).
    PickTeam {
        team: Option<u8>,
    },
    /// Client: spectate (or play again) from the next match on.
    Spectate {
        spectate: bool,
    },
}

/// Most weapons an [`InventoryMsg`] lists.
pub const MAX_INVENTORY_WEAPONS: usize = 64;

/// One carried weapon in an [`InventoryMsg`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetWeapon {
    /// Weapon id (index into the match's weapon table).
    pub weapon: u8,
    /// Ammo for count weapons (clamped to `u16::MAX`), else 0.
    pub ammo: u16,
    /// Upgrade level (1..).
    pub level: u8,
    /// Paid shots fired with this weapon (`ls_sim::WeaponState::shots`,
    /// wrapping): the phase of alternating and rotating patterns.
    pub shots: u32,
    /// Low byte of the weapon module's state (`ShipState::weapon_state`,
    /// e.g. the Paint Thrower colour).
    pub state: u8,
}

/// The own ship's inventory as the simulation has it at `tick` (dead ships
/// keep the last one until they respawn).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InventoryMsg {
    pub tick: u32,
    pub weapons: Vec<NetWeapon>,
    /// Selected primary and secondary weapon ids (255 = none).
    pub primary: u8,
    pub secondary: u8,
    /// Gadget id (255 = none).
    pub gadget: u8,
}

mod tag {
    pub const HELLO: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const REJECT: u8 = 3;
    pub const INPUT: u8 = 4;
    pub const SNAPSHOT: u8 = 5;
    pub const EVENTS: u8 = 6;
    pub const PING: u8 = 7;
    pub const PONG: u8 = 8;
    pub const DISCONNECT: u8 = 9;
    pub const SELECT_SHIP: u8 = 10;
    pub const INVENTORY: u8 = 11;
    pub const LOBBY_STATE: u8 = 12;
    pub const MATCH_RESULTS: u8 = 13;
    pub const READY: u8 = 14;
    pub const PICK_TEAM: u8 = 15;
    pub const SPECTATE: u8 = 16;
}

fn write_opt_str(w: &mut BitWriter, s: &Option<String>) {
    w.write_bool(s.is_some());
    if let Some(s) = s {
        w.write_str(s);
    }
}

fn read_opt_str(r: &mut BitReader, max: usize) -> Result<Option<String>, DecodeError> {
    Ok(if r.read_bool()? {
        Some(r.read_string(max)?)
    } else {
        None
    })
}

fn check_len(s: &str, max: usize, what: &'static str) -> Result<(), EncodeError> {
    if s.len() > max {
        Err(EncodeError::OutOfRange(what))
    } else {
        Ok(())
    }
}

fn write_quant(w: &mut BitWriter, q: &QuantConfig) {
    for v in [q.bounds.min, q.bounds.max] {
        w.write_f32(v.x);
        w.write_f32(v.y);
        w.write_f32(v.z);
    }
    w.write_f32(q.max_ship_speed);
    w.write_f32(q.max_projectile_speed);
}

fn read_quant(r: &mut BitReader) -> Result<QuantConfig, DecodeError> {
    let mut v = [Vec3::ZERO; 2];
    for slot in &mut v {
        *slot = Vec3::new(r.read_f32()?, r.read_f32()?, r.read_f32()?);
    }
    let q = QuantConfig {
        bounds: Aabb {
            min: v[0],
            max: v[1],
        },
        max_ship_speed: r.read_f32()?,
        max_projectile_speed: r.read_f32()?,
    };
    q.validate().map_err(DecodeError::Invalid)?;
    Ok(q)
}

impl Message {
    /// The message's name (for logs and error messages).
    pub fn name(&self) -> &'static str {
        match self {
            Message::Hello(_) => "Hello",
            Message::Welcome(_) => "Welcome",
            Message::Reject(_) => "Reject",
            Message::Input(_) => "Input",
            Message::Snapshot(_) => "Snapshot",
            Message::Events(_) => "Events",
            Message::Ping { .. } => "Ping",
            Message::Pong { .. } => "Pong",
            Message::Disconnect { .. } => "Disconnect",
            Message::SelectShip { .. } => "SelectShip",
            Message::Inventory(_) => "Inventory",
            Message::LobbyState(_) => "LobbyState",
            Message::MatchResults(_) => "MatchResults",
            Message::Ready { .. } => "Ready",
            Message::PickTeam { .. } => "PickTeam",
            Message::Spectate { .. } => "Spectate",
        }
    }

    /// The channel this message is meant for.
    pub fn channel(&self) -> Channel {
        match self {
            Message::Input(_)
            | Message::Snapshot(_)
            | Message::Ping { .. }
            | Message::Pong { .. } => Channel::Unreliable,
            _ => Channel::Reliable,
        }
    }

    /// Encodes a message; snapshots may only be full (no baseline).
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        self.encode_with(&NoBaselines)
    }

    /// Encodes a message, looking up snapshot baselines in `baselines`.
    pub fn encode_with(&self, baselines: &dyn Baselines) -> Result<Vec<u8>, EncodeError> {
        let mut w = BitWriter::with_capacity(64);
        match self {
            Message::Hello(h) => {
                check_len(&h.name, MAX_NAME_LEN, "name too long")?;
                for code in h.room.iter().chain(&h.party) {
                    check_len(code, MAX_CODE_LEN, "room or party code too long")?;
                }
                w.write_u8(tag::HELLO);
                w.write_u32(MAGIC);
                w.write_u16(h.protocol_version);
                w.write_str(&h.name);
                w.write_bool(h.preferred_team.is_some());
                if let Some(t) = h.preferred_team {
                    w.write_u8(t);
                }
                w.write_bits(h.data_hash, 64);
                w.write_bool(h.ship_type.is_some());
                if let Some(t) = h.ship_type {
                    w.write_u8(t);
                }
                write_opt_str(&mut w, &h.room);
                write_opt_str(&mut w, &h.party);
            }
            Message::Welcome(wl) => {
                check_len(&wl.share_code, MAX_SHARE_CODE_LEN, "share code too long")?;
                check_len(
                    &wl.ruleset_name,
                    MAX_RULESET_NAME_LEN,
                    "rule set name too long",
                )?;
                if wl.ruleset.len() > MAX_RULESET_BYTES {
                    return Err(EncodeError::OutOfRange("rule set too large"));
                }
                if wl.edit_log.len() > MAX_EDIT_LOG {
                    return Err(EncodeError::OutOfRange("edit log too long"));
                }
                let Some(pruned) = wl.edit_count.checked_sub(wl.edit_log.len() as u32) else {
                    return Err(EncodeError::OutOfRange("edit count below the log length"));
                };
                if wl.player_id >= MAX_SHIPS {
                    return Err(EncodeError::OutOfRange("player id"));
                }
                check_len(&wl.room_code, MAX_CODE_LEN, "room code too long")?;
                wl.quant.validate().map_err(EncodeError::OutOfRange)?;
                w.write_u8(tag::WELCOME);
                w.write_u16(wl.protocol_version);
                w.write_u8(wl.player_id);
                w.write_u8(wl.team);
                w.write_u8(wl.ship_type);
                w.write_bits(wl.data_hash, 64);
                w.write_str(&wl.share_code);
                w.write_str(&wl.ruleset_name);
                w.write_bytes(&wl.ruleset);
                write_quant(&mut w, &wl.quant);
                w.write_u32(wl.server_tick);
                w.write_varint(wl.edit_log.len() as u64);
                for &e in &wl.edit_log {
                    let bits = TerrainEditBits::from(e);
                    check_edit(&bits)?;
                    write_edit(&mut w, &bits);
                }
                w.write_varint(pruned as u64);
                w.write_u8(wl.epoch);
                w.write_str(&wl.room_code);
            }
            Message::Reject(rj) => {
                check_len(&rj.message, MAX_REASON_LEN, "reject message too long")?;
                w.write_u8(tag::REJECT);
                w.write_u16(rj.server_version);
                w.write_u8(rj.reason.to_u8());
                w.write_str(&rj.message);
            }
            Message::Input(inp) => {
                let n = inp.cmds.len();
                if n == 0 || n > MAX_INPUT_CMDS {
                    return Err(EncodeError::BadInputList);
                }
                for pair in inp.cmds.windows(2) {
                    let gap = pair[0].tick.wrapping_sub(pair[1].tick);
                    if !(1..=255).contains(&gap) {
                        return Err(EncodeError::BadInputList);
                    }
                }
                w.write_u8(tag::INPUT);
                w.write_u8(inp.epoch);
                w.write_bool(inp.ack_snapshot.is_some());
                if let Some(t) = inp.ack_snapshot {
                    w.write_u32(t);
                }
                w.write_bits(n as u64, 2);
                let newest = &inp.cmds[0];
                w.write_u32(newest.tick);
                newest.write_body(&mut w);
                for pair in inp.cmds.windows(2) {
                    let (newer, older) = (&pair[0], &pair[1]);
                    w.write_u8(newer.tick.wrapping_sub(older.tick) as u8);
                    // Held controls repeat often: 1 bit instead of 88.
                    let same = older.same_body(newer);
                    w.write_bool(same);
                    if !same {
                        older.write_body(&mut w);
                    }
                }
            }
            Message::Snapshot(sm) => {
                let baseline = match sm.baseline {
                    Some(t) => Some(
                        baselines
                            .baseline(t)
                            .ok_or(EncodeError::MissingBaseline(t))?,
                    ),
                    None => None,
                };
                w.write_u8(tag::SNAPSHOT);
                w.write_u8(sm.epoch);
                write_snapshot(&mut w, &sm.snapshot, baseline)?;
            }
            Message::Events(events) => {
                if events.len() > MAX_EVENTS {
                    return Err(EncodeError::OutOfRange("too many events"));
                }
                w.write_u8(tag::EVENTS);
                w.write_varint(events.len() as u64);
                for e in events {
                    e.write(&mut w)?;
                }
            }
            Message::Ping {
                nonce,
                client_time_ms,
            } => {
                w.write_u8(tag::PING);
                w.write_u32(*nonce);
                w.write_u32(*client_time_ms);
            }
            Message::Pong {
                nonce,
                client_time_ms,
                server_tick,
            } => {
                w.write_u8(tag::PONG);
                w.write_u32(*nonce);
                w.write_u32(*client_time_ms);
                w.write_u32(*server_tick);
            }
            Message::Disconnect { reason } => {
                check_len(reason, MAX_REASON_LEN, "disconnect reason too long")?;
                w.write_u8(tag::DISCONNECT);
                w.write_str(reason);
            }
            Message::SelectShip { ship_type } => {
                w.write_u8(tag::SELECT_SHIP);
                w.write_u8(*ship_type);
            }
            Message::Inventory(inv) => {
                if inv.weapons.len() > MAX_INVENTORY_WEAPONS {
                    return Err(EncodeError::OutOfRange("too many inventory weapons"));
                }
                w.write_u8(tag::INVENTORY);
                w.write_u32(inv.tick);
                w.write_varint(inv.weapons.len() as u64);
                for x in &inv.weapons {
                    w.write_u8(x.weapon);
                    w.write_u16(x.ammo);
                    w.write_u8(x.level);
                    w.write_varint(x.shots as u64);
                    w.write_u8(x.state);
                }
                w.write_u8(inv.primary);
                w.write_u8(inv.secondary);
                w.write_u8(inv.gadget);
            }
            Message::LobbyState(l) => {
                // Validate before the tag so an error leaves nothing behind.
                write_lobby(&mut BitWriter::new(), l)?;
                w.write_u8(tag::LOBBY_STATE);
                write_lobby(&mut w, l)?;
            }
            Message::MatchResults(m) => {
                write_results(&mut BitWriter::new(), m)?;
                w.write_u8(tag::MATCH_RESULTS);
                write_results(&mut w, m)?;
            }
            Message::Ready { ready } => {
                w.write_u8(tag::READY);
                w.write_bool(*ready);
            }
            Message::PickTeam { team } => {
                w.write_u8(tag::PICK_TEAM);
                w.write_bool(team.is_some());
                if let Some(t) = team {
                    w.write_u8(*t);
                }
            }
            Message::Spectate { spectate } => {
                w.write_u8(tag::SPECTATE);
                w.write_bool(*spectate);
            }
        }
        Ok(w.finish())
    }

    /// The match epoch of an encoded `Snapshot` (`None` for other
    /// messages), read without decoding it: a client drops snapshots of
    /// another epoch before it looks for their baseline, which belongs to
    /// the previous match.
    pub fn peek_snapshot_epoch(bytes: &[u8]) -> Option<u8> {
        let mut r = BitReader::new(bytes);
        if r.read_u8().ok()? != tag::SNAPSHOT {
            return None;
        }
        r.read_u8().ok()
    }

    /// Decodes a message; delta snapshots fail with `MissingBaseline`.
    pub fn decode(bytes: &[u8]) -> Result<Message, DecodeError> {
        Self::decode_with(bytes, &NoBaselines)
    }

    /// Decodes a message, looking up snapshot baselines in `baselines`.
    /// Rejects trailing bytes.
    pub fn decode_with(bytes: &[u8], baselines: &dyn Baselines) -> Result<Message, DecodeError> {
        let mut r = BitReader::new(bytes);
        let t = r.read_u8()?;
        let msg = match t {
            tag::HELLO => {
                if r.read_u32()? != MAGIC {
                    return Err(DecodeError::BadMagic);
                }
                let version = r.read_u16()?;
                check_version(version)?;
                let name = r.read_string(MAX_NAME_LEN)?;
                let preferred_team = if r.read_bool()? {
                    Some(r.read_u8()?)
                } else {
                    None
                };
                let data_hash = r.read_bits(64)?;
                let ship_type = if r.read_bool()? {
                    Some(r.read_u8()?)
                } else {
                    None
                };
                let room = read_opt_str(&mut r, MAX_CODE_LEN)?;
                let party = read_opt_str(&mut r, MAX_CODE_LEN)?;
                Message::Hello(Hello {
                    protocol_version: version,
                    name,
                    preferred_team,
                    data_hash,
                    ship_type,
                    room,
                    party,
                })
            }
            tag::WELCOME => {
                let version = r.read_u16()?;
                check_version(version)?;
                let player_id = r.read_u8()?;
                if player_id >= MAX_SHIPS {
                    return Err(DecodeError::Invalid("player id"));
                }
                let team = r.read_u8()?;
                let ship_type = r.read_u8()?;
                let data_hash = r.read_bits(64)?;
                let share_code = r.read_string(MAX_SHARE_CODE_LEN)?;
                let ruleset_name = r.read_string(MAX_RULESET_NAME_LEN)?;
                let ruleset = r.read_bytes(MAX_RULESET_BYTES)?;
                let quant = read_quant(&mut r)?;
                let server_tick = r.read_u32()?;
                let n = r.read_len(MAX_EDIT_LOG, EDIT_MIN_BITS)?;
                let mut edit_log = Vec::with_capacity(n);
                for _ in 0..n {
                    edit_log.push(read_edit(&mut r)?.into());
                }
                let edit_count = (n as u32)
                    .checked_add(r.read_varint_u32()?)
                    .ok_or(DecodeError::Invalid("edit count overflows"))?;
                let epoch = r.read_u8()?;
                let room_code = r.read_string(MAX_CODE_LEN)?;
                Message::Welcome(Box::new(Welcome {
                    protocol_version: version,
                    player_id,
                    team,
                    ship_type,
                    data_hash,
                    share_code,
                    ruleset_name,
                    ruleset,
                    quant,
                    server_tick,
                    edit_log,
                    edit_count,
                    epoch,
                    room_code,
                }))
            }
            tag::REJECT => Message::Reject(Reject {
                server_version: r.read_u16()?,
                reason: RejectReason::from_u8(r.read_u8()?),
                message: r.read_string(MAX_REASON_LEN)?,
            }),
            tag::INPUT => {
                let epoch = r.read_u8()?;
                let ack_snapshot = if r.read_bool()? {
                    Some(r.read_u32()?)
                } else {
                    None
                };
                let n = r.read_bits(2)? as usize;
                if n == 0 {
                    return Err(DecodeError::Invalid("empty input message"));
                }
                let tick = r.read_u32()?;
                let mut cmds = Vec::with_capacity(n);
                cmds.push(NetInput::read_body(&mut r, tick)?);
                for i in 1..n {
                    let gap = r.read_u8()?;
                    if gap == 0 {
                        return Err(DecodeError::Invalid("input tick gap 0"));
                    }
                    let tick = cmds[i - 1].tick.wrapping_sub(gap as u32);
                    let cmd = if r.read_bool()? {
                        NetInput {
                            tick,
                            ..cmds[i - 1]
                        }
                    } else {
                        NetInput::read_body(&mut r, tick)?
                    };
                    cmds.push(cmd);
                }
                Message::Input(InputMsg {
                    epoch,
                    ack_snapshot,
                    cmds,
                })
            }
            tag::SNAPSHOT => {
                let epoch = r.read_u8()?;
                let (snapshot, baseline) = read_snapshot(&mut r, baselines)?;
                Message::Snapshot(Box::new(SnapshotMsg {
                    epoch,
                    baseline,
                    snapshot,
                }))
            }
            tag::EVENTS => {
                let n = r.read_len(MAX_EVENTS, EVENT_MIN_BITS)?;
                let mut events = Vec::with_capacity(n);
                for _ in 0..n {
                    events.push(NetEvent::read(&mut r)?);
                }
                Message::Events(events)
            }
            tag::PING => Message::Ping {
                nonce: r.read_u32()?,
                client_time_ms: r.read_u32()?,
            },
            tag::PONG => Message::Pong {
                nonce: r.read_u32()?,
                client_time_ms: r.read_u32()?,
                server_tick: r.read_u32()?,
            },
            tag::DISCONNECT => Message::Disconnect {
                reason: r.read_string(MAX_REASON_LEN)?,
            },
            tag::SELECT_SHIP => Message::SelectShip {
                ship_type: r.read_u8()?,
            },
            tag::INVENTORY => {
                let tick = r.read_u32()?;
                let n = r.read_len(MAX_INVENTORY_WEAPONS, 48)?;
                let mut weapons = Vec::with_capacity(n);
                for _ in 0..n {
                    weapons.push(NetWeapon {
                        weapon: r.read_u8()?,
                        ammo: r.read_u16()?,
                        level: r.read_u8()?,
                        shots: r.read_varint_u32()?,
                        state: r.read_u8()?,
                    });
                }
                Message::Inventory(InventoryMsg {
                    tick,
                    weapons,
                    primary: r.read_u8()?,
                    secondary: r.read_u8()?,
                    gadget: r.read_u8()?,
                })
            }
            tag::LOBBY_STATE => Message::LobbyState(Box::new(read_lobby(&mut r)?)),
            tag::MATCH_RESULTS => Message::MatchResults(Box::new(read_results(&mut r)?)),
            tag::READY => Message::Ready {
                ready: r.read_bool()?,
            },
            tag::PICK_TEAM => Message::PickTeam {
                team: if r.read_bool()? {
                    Some(r.read_u8()?)
                } else {
                    None
                },
            },
            tag::SPECTATE => Message::Spectate {
                spectate: r.read_bool()?,
            },
            other => return Err(DecodeError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(msg)
    }
}

fn check_version(theirs: u16) -> Result<(), DecodeError> {
    if theirs == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(DecodeError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs,
        })
    }
}

/// Reads only the tag, magic and version of a `Hello` packet. Works for any
/// protocol version, so a server can answer mismatches with a [`Reject`].
pub fn peek_hello_version(bytes: &[u8]) -> Result<u16, DecodeError> {
    let mut r = BitReader::new(bytes);
    let t = r.read_u8()?;
    if t != tag::HELLO {
        return Err(DecodeError::Invalid("not a hello"));
    }
    if r.read_u32()? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    r.read_u16()
}

/// Server side: the `Reject` for a client whose data hash differs from
/// `server_hash`, or `None` if they match.
pub fn check_data_hash(h: &Hello, server_hash: u64) -> Option<Reject> {
    (h.data_hash != server_hash).then(|| Reject {
        reason: RejectReason::DataMismatch,
        server_version: PROTOCOL_VERSION,
        message: format!(
            "game data mismatch: server data {server_hash:016x}, client data {:016x}; \
             the client and server were built from different data/ files (update the client)",
            h.data_hash
        ),
    })
}

/// Server side: validates a decoded `Hello`.
pub fn check_hello(h: &Hello) -> Result<(), HandshakeError> {
    if h.protocol_version != PROTOCOL_VERSION {
        return Err(HandshakeError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs: h.protocol_version,
        });
    }
    let name = h.name.trim();
    if name.is_empty() || h.name.len() > MAX_NAME_LEN {
        return Err(HandshakeError::Invalid("name must be 1..=32 bytes"));
    }
    if h.name.chars().any(char::is_control) {
        return Err(HandshakeError::Invalid("name contains control characters"));
    }
    Ok(())
}

/// Server side: turns the raw first packet of a connection into a validated
/// `Hello`, or into the `Reject` that should be sent back.
pub fn accept_hello(bytes: &[u8]) -> Result<Hello, Reject> {
    let reject = |reason, message: String| Reject {
        reason,
        server_version: PROTOCOL_VERSION,
        message,
    };
    match Message::decode(bytes) {
        Ok(Message::Hello(h)) => match check_hello(&h) {
            Ok(()) => Ok(h),
            Err(e) => Err(reject(RejectReason::BadHandshake, e.to_string())),
        },
        Ok(_) => Err(reject(
            RejectReason::BadHandshake,
            "expected Hello".to_string(),
        )),
        Err(DecodeError::VersionMismatch { ours, theirs }) => Err(reject(
            RejectReason::VersionMismatch,
            format!("server speaks protocol {ours}, client {theirs}"),
        )),
        Err(e) => Err(reject(RejectReason::BadHandshake, e.to_string())),
    }
}

/// Client side: checks the server's answer to `Hello`.
pub fn check_welcome(msg: Message) -> Result<Welcome, HandshakeError> {
    match msg {
        Message::Welcome(w) => {
            if w.protocol_version != PROTOCOL_VERSION {
                return Err(HandshakeError::VersionMismatch {
                    ours: PROTOCOL_VERSION,
                    theirs: w.protocol_version,
                });
            }
            if w.share_code.is_empty() {
                return Err(HandshakeError::Invalid("empty share code"));
            }
            Ok(*w)
        }
        Message::Reject(r) => Err(HandshakeError::Rejected(format!(
            "{:?} (server protocol {}): {}",
            r.reason, r.server_version, r.message
        ))),
        _ => Err(HandshakeError::Unexpected {
            expected: "Welcome or Reject",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::Quantizer;
    use crate::snapshot::SnapshotHistory;
    use crate::wire::{
        ship_flags, NetEntity, NetPlayer, NetProjectile, NetShip, ShipStatus, Snapshot,
    };
    use ls_math::{Pcg32, Quat};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn round_trip(msg: &Message) -> Vec<u8> {
        let bytes = msg.encode().unwrap();
        assert_eq!(&Message::decode(&bytes).unwrap(), msg);
        bytes
    }

    fn welcome() -> Welcome {
        Welcome {
            epoch: 0,
            room_code: String::new(),
            protocol_version: PROTOCOL_VERSION,
            player_id: 3,
            team: 1,
            ship_type: 2,
            data_hash: 0x0123_4567_89ab_cdef,
            share_code: "LS-7K2M-9QXA-04RT-ZZ1C".into(),
            ruleset_name: "paint_the_void".into(),
            ruleset: b"(name: \"paint_the_void\", friendly_fire: 0)".to_vec(),
            quant: QuantConfig::default(),
            server_tick: 12_345,
            edit_log: vec![
                TerrainEdit::SubtractSphere {
                    center: Vec3::new(1.0, 2.0, 3.0),
                    radius: 1.5,
                },
                TerrainEdit::Paint {
                    center: Vec3::new(-4.0, 0.25, 9.0),
                    radius: 2.0,
                    team: 1,
                    color: ls_math::terrain::PAINT_RED,
                },
                TerrainEdit::AddSphere {
                    center: Vec3::new(0.0, -60.0, 100.0),
                    radius: 4.0,
                },
            ],
            edit_count: 3,
        }
    }

    fn input(tick: u32, seed: u64) -> NetInput {
        let mut rng = Pcg32::new(seed, 9);
        NetInput {
            tick,
            thrust: [
                rng.next_u32() as i8,
                rng.next_u32() as i8,
                rng.next_u32() as i8,
            ],
            look: [rng.next_u32() as i16, rng.next_u32() as i16],
            roll: rng.next_u32() as i8,
            buttons: rng.next_u32() as u16,
            weapon: rng.next_u32() as u8,
        }
    }

    #[test]
    fn hello_reject_ping_pong_disconnect_round_trip() {
        round_trip(&Message::Hello(Hello::new("Rivet Rita", Some(1))));
        round_trip(&Message::Hello(Hello::new("", None)));
        round_trip(&Message::Hello(
            Hello::new("Hauler Hank", None)
                .with_data_hash(u64::MAX - 7)
                .with_ship_type(Some(2)),
        ));
        round_trip(&Message::SelectShip { ship_type: 3 });
        round_trip(&Message::Inventory(InventoryMsg {
            tick: 1234,
            weapons: vec![
                NetWeapon {
                    shots: 0,
                    state: 0,
                    weapon: 0,
                    ammo: 0,
                    level: 3,
                },
                NetWeapon {
                    shots: 70_001,
                    state: 2,
                    weapon: 14,
                    ammo: 7,
                    level: 1,
                },
            ],
            primary: 0,
            secondary: 14,
            gadget: 255,
        }));
        round_trip(&Message::Reject(Reject {
            reason: RejectReason::DataMismatch,
            server_version: PROTOCOL_VERSION,
            message: "different data".into(),
        }));
        round_trip(&Message::Reject(Reject {
            reason: RejectReason::ServerFull,
            server_version: PROTOCOL_VERSION,
            message: "8/8 players".into(),
        }));
        round_trip(&Message::Ping {
            nonce: 7,
            client_time_ms: u32::MAX,
        });
        round_trip(&Message::Pong {
            nonce: 7,
            client_time_ms: 99,
            server_tick: 1 << 31,
        });
        round_trip(&Message::Disconnect {
            reason: "bye".into(),
        });
    }

    #[test]
    fn protocol_5_messages_round_trip() {
        use crate::mode::{LobbyPhase, LobbyPlayer, LobbyStateMsg, MatchResultsMsg, ResultRow};
        // Hello with room and party codes.
        let h = Hello::new("Rivet Rita", None)
            .with_room(Some("R-7K2M".into()))
            .with_party(Some("P-ABC123".into()));
        round_trip(&Message::Hello(h.clone()));
        round_trip(&Message::Hello(h.with_room(None)));
        let long = Hello::new("x", None).with_party(Some("P".repeat(MAX_CODE_LEN + 1)));
        assert!(Message::Hello(long).encode().is_err());
        // Welcome with epoch and room code.
        let mut w = welcome();
        w.epoch = 250;
        w.room_code = "R-7K2M".into();
        round_trip(&Message::Welcome(Box::new(w)));
        // Inventory with shots and state.
        round_trip(&Message::Inventory(InventoryMsg {
            tick: 9,
            weapons: vec![NetWeapon {
                weapon: 20,
                ammo: 0,
                level: 1,
                shots: u32::MAX,
                state: 3,
            }],
            primary: 20,
            secondary: 255,
            gadget: 1,
        }));
        // Lobby messages.
        for phase in [
            LobbyPhase::Lobby,
            LobbyPhase::Countdown,
            LobbyPhase::Match,
            LobbyPhase::Results,
        ] {
            round_trip(&Message::LobbyState(Box::new(LobbyStateMsg {
                phase,
                phase_ticks_left: 600,
                room_code: String::new(),
                next_mode: 1,
                next_ruleset: "team_brawl".into(),
                next_share_code: "LS-7K2M-9QXA-04RT-ZZ1C".into(),
                min_players: 4,
                max_players: 32,
                teams: 2,
                players: (0..32u8)
                    .map(|id| LobbyPlayer {
                        id,
                        team: if id % 5 == 0 { 255 } else { id % 2 },
                        ready: id % 3 == 0,
                        spectator: id == 7,
                    })
                    .collect(),
            })));
        }
        round_trip(&Message::LobbyState(Box::default()));
        let bad = LobbyStateMsg {
            next_ruleset: "m".repeat(65),
            ..Default::default()
        };
        assert!(Message::LobbyState(Box::new(bad)).encode().is_err());
        round_trip(&Message::MatchResults(Box::new(MatchResultsMsg {
            tick: 36_000,
            kind: 1,
            teams: 2,
            winner: 1,
            team_scores: vec![14, 15],
            rows: (0..8u8)
                .map(|id| ResultRow {
                    id,
                    side: id % 2,
                    kills: id as u32 * 3,
                    deaths: 2,
                    score: -(id as i32),
                    name: format!("pilot {id}"),
                })
                .collect(),
        })));
        round_trip(&Message::MatchResults(Box::default()));
        for ready in [false, true] {
            round_trip(&Message::Ready { ready });
            round_trip(&Message::Spectate { spectate: ready });
        }
        round_trip(&Message::PickTeam { team: None });
        round_trip(&Message::PickTeam { team: Some(3) });
        // Epochs on inputs and snapshots.
        round_trip(&Message::Input(InputMsg {
            epoch: 7,
            ack_snapshot: None,
            cmds: vec![input(5, 1)],
        }));
        let quant = Quantizer::default();
        let snap = round_trip(&Message::Snapshot(Box::new(SnapshotMsg {
            epoch: 9,
            baseline: None,
            snapshot: quant.pack_snapshot(&game_snapshot(100, 0.0)),
        })));
        assert_eq!(Message::peek_snapshot_epoch(&snap), Some(9));
        let input = Message::Input(InputMsg {
            epoch: 9,
            ack_snapshot: None,
            cmds: vec![input(5, 1)],
        });
        assert_eq!(Message::peek_snapshot_epoch(&input.encode().unwrap()), None);
        assert_eq!(Message::peek_snapshot_epoch(&[]), None);
        // Every new message is reliable.
        for m in [
            Message::LobbyState(Box::default()),
            Message::MatchResults(Box::default()),
            Message::Ready { ready: true },
            Message::PickTeam { team: None },
            Message::Spectate { spectate: true },
        ] {
            assert_eq!(m.channel(), Channel::Reliable);
        }
    }

    #[test]
    fn welcome_round_trip_with_edit_log() {
        let w = welcome();
        let bytes = round_trip(&Message::Welcome(Box::new(w.clone())));
        // Each edit costs about 17 bytes.
        assert!(bytes.len() < 200, "{}", bytes.len());
        let back = check_welcome(Message::decode(&bytes).unwrap()).unwrap();
        assert_eq!(back.edit_log, w.edit_log);
        assert_eq!(back.edit_count, 3);
        // A big log still round-trips.
        let mut big = welcome();
        big.edit_log = (0..5000)
            .map(|i| TerrainEdit::SubtractSphere {
                center: Vec3::new(i as f32 * 0.1, 0.0, -(i as f32)),
                radius: 3.0,
            })
            .collect();
        big.edit_count = 5000;
        round_trip(&Message::Welcome(Box::new(big)));
        // A pruned log carries the full edit count.
        let mut pruned = welcome();
        pruned.edit_count = 1_000_003;
        let bytes = round_trip(&Message::Welcome(Box::new(pruned)));
        assert!(bytes.len() < 200, "{}", bytes.len());
        // The count cannot be below the log length.
        let mut bad = welcome();
        bad.edit_count = 2;
        assert!(Message::Welcome(Box::new(bad)).encode().is_err());
    }

    #[test]
    fn welcome_rejects_bad_quant_config() {
        let mut w = welcome();
        w.quant.max_ship_speed = f32::NAN;
        assert!(Message::Welcome(Box::new(w)).encode().is_err());
    }

    #[test]
    fn input_round_trip_with_redundancy() {
        for n in 1..=3u32 {
            // Distinct bodies.
            let cmds: Vec<NetInput> = (0..n).map(|i| input(1000 - i, i as u64)).collect();
            let msg = Message::Input(InputMsg {
                epoch: 0,
                ack_snapshot: Some(998),
                cmds,
            });
            let bytes = round_trip(&msg);
            assert_eq!(
                bytes.len(),
                (8 + 8 + 1 + 32 + 2 + 32 + 88 + (n as usize - 1) * (8 + 1 + 88)).div_ceil(8)
            );
        }
        // Held controls: older commands cost 9 bits each.
        let held = input(50, 1);
        let msg = Message::Input(InputMsg {
            epoch: 0,
            ack_snapshot: None,
            cmds: vec![
                held,
                NetInput { tick: 49, ..held },
                NetInput { tick: 47, ..held },
            ],
        });
        let bytes = round_trip(&msg);
        assert_eq!(
            bytes.len(),
            (8 + 8 + 1 + 2 + 32 + 88 + 2 * 9_usize).div_ceil(8)
        );
        // Ticks may wrap around.
        round_trip(&Message::Input(InputMsg {
            epoch: 0,
            ack_snapshot: Some(0),
            cmds: vec![input(1, 3), input(0, 4), input(u32::MAX, 5)],
        }));
    }

    #[test]
    fn invalid_input_lists_are_rejected() {
        let bad = [
            vec![],
            vec![input(5, 1); 4],
            vec![input(5, 1), input(5, 2)],
            vec![input(5, 1), input(6, 2)],
            vec![input(500, 1), input(100, 2)],
        ];
        for cmds in bad {
            let msg = Message::Input(InputMsg {
                epoch: 0,
                ack_snapshot: None,
                cmds,
            });
            assert_eq!(msg.encode(), Err(EncodeError::BadInputList));
        }
    }

    #[test]
    fn events_round_trip() {
        let events = vec![
            NetEvent::Hit {
                tick: 10,
                attacker: 1,
                victim: 2,
                weapon: 3,
                damage: 250,
                pos: Vec3::new(1.5, -2.0, 3.25),
            },
            NetEvent::Kill {
                tick: 11,
                killer: 1,
                victim: 2,
                weapon: 3,
            },
            NetEvent::Respawn {
                tick: 200,
                ship: 2,
                pos: Vec3::new(0.0, 10.0, 0.0),
            },
            NetEvent::MatchOver {
                tick: 18_000,
                winner_team: Some(1),
            },
            NetEvent::MatchOver {
                tick: 18_000,
                winner_team: None,
            },
            NetEvent::Cue {
                tick: 5,
                kind: 1,
                source: 255,
                target: 4,
                pos: Vec3::new(1.0, 2.0, 3.0),
                value: 120,
            },
        ];
        round_trip(&Message::Events(events));
        round_trip(&Message::Events(vec![]));
    }

    fn game_snapshot(tick: u32, t: f32) -> Snapshot {
        let ships = (0..8u8)
            .map(|id| NetShip {
                id,
                pos: Vec3::new(id as f32 * 10.0 + 20.0 + t, 5.0, 100.0 - t * 2.0),
                vel: Vec3::new(12.0, 0.0, -24.0),
                rot: Quat::from_axis_angle(Vec3::Y, t * 0.1 + id as f32),
                flags: ship_flags::with_team(ship_flags::ALIVE, id % 2),
                status: (id == 3).then_some(ShipStatus {
                    hull: 100,
                    energy: 80,
                    boost: 40,
                    scrap: 12,
                }),
            })
            .collect();
        Snapshot {
            // Scores change every 5th snapshot, the carried objective moves.
            mode: crate::mode::NetModeState {
                kind: 1,
                teams: 2,
                phase: crate::mode::mode_phase::RUNNING,
                winner: 255,
                end_tick: 9000,
                score_target: 50,
                team_scores: vec![(tick / 10) as i64, 2],
                players: (0..8u8)
                    .map(|id| crate::mode::NetModePlayer {
                        id,
                        side: id % 2,
                        score: (tick / 10) as i32 * (id as i32 % 3),
                        lives: 3,
                        meter: 100,
                        flags: 0,
                    })
                    .collect(),
                objectives: vec![crate::mode::NetObjective {
                    id: 0,
                    kind: 1,
                    team: 255,
                    holder: 3,
                    state: 0,
                    timer_end: 0,
                    radius: 1.0,
                    pos: Vec3::new(20.0 + t, 5.0, 100.0 - t * 2.0),
                }],
                ..Default::default()
            },
            tick,
            ack_input: tick + 4,
            edit_base: 3,
            edits: vec![TerrainEdit::Paint {
                center: Vec3::new(1.0, 2.0, 3.0),
                radius: 2.0,
                team: 0,
                color: 0,
            }],
            ships,
            projectiles: vec![NetProjectile {
                id: 77,
                kind: 1,
                owner: 3,
                pos: Vec3::new(t + 10.0, 10.0, 10.0),
                vel: Vec3::new(0.0, 0.0, -180.0),
                state: 1,
            }],
            entities: vec![
                NetEntity {
                    id: 5,
                    kind: 4,
                    visual: 1,
                    owner: 255,
                    team: 255,
                    pos: Vec3::new(30.0, 20.0, 10.0),
                    vel: Vec3::ZERO,
                    rot: Quat::IDENTITY,
                    state: [30, 0, 0, 0],
                },
                NetEntity {
                    id: 9,
                    kind: 0,
                    visual: 0,
                    owner: 255,
                    team: 255,
                    pos: Vec3::new(50.0 + t, 20.0, 10.0),
                    vel: Vec3::new(3.0, 0.0, 0.0),
                    rot: Quat::from_axis_angle(Vec3::Y, t),
                    state: [2, 0, 0, 0],
                },
            ],
            players: (0..8u8)
                .map(|id| NetPlayer {
                    id,
                    team: id % 2,
                    ship_type: 1,
                    gadget: 255,
                    name: format!("pilot {id}"),
                })
                .collect(),
            scores: vec![1200, 900],
            match_ticks_left: 9000 - tick,
        }
    }

    #[test]
    fn snapshot_message_full_and_delta_through_history() {
        let quant = Quantizer::default();
        let mut server = SnapshotHistory::default();
        let mut client = SnapshotHistory::default();
        let mut acked = None;
        for step in 0..20u32 {
            let tick = 100 + step * 2;
            let packed = quant.pack_snapshot(&game_snapshot(tick, step as f32 * 0.4));
            server.push(packed.clone());
            let msg = Message::Snapshot(Box::new(SnapshotMsg {
                epoch: 0,
                baseline: server.choose_baseline(tick, acked),
                snapshot: packed.clone(),
            }));
            let bytes = msg.encode_with(&server).unwrap();
            let decoded = Message::decode_with(&bytes, &client).unwrap();
            assert_eq!(decoded, msg);
            let Message::Snapshot(sm) = decoded else {
                unreachable!()
            };
            if step > 0 {
                assert!(sm.baseline.is_some(), "later snapshots are deltas");
            }
            // Game-facing view is within quantization error of the source.
            let view = quant.unpack_snapshot(&sm.snapshot);
            let src = game_snapshot(tick, step as f32 * 0.4);
            for (a, b) in view.ships.iter().zip(&src.ships) {
                assert!((a.pos - b.pos).length() < 0.01);
                assert!((a.vel - b.vel).length() < 0.01);
                assert!(a.rot.dot(b.rot).abs() > 0.99999);
                assert_eq!((a.flags, a.status), (b.flags, b.status));
            }
            assert_eq!(view.edits, src.edits);
            assert_eq!(view.players, src.players);
            assert_eq!(view.projectiles[0].state, 1);
            // Entity positions are 8 position steps coarse (about 0.03 u).
            let half = quant.entity_position_step() * 0.5;
            for (a, b) in view.entities.iter().zip(&src.entities) {
                let d = a.pos - b.pos;
                assert!(d.x.abs() <= half.x && d.y.abs() <= half.y && d.z.abs() <= half.z);
                assert!((a.vel - b.vel).length() < 0.01);
                assert!(a.rot.dot(b.rot).abs() > 0.99999);
                assert_eq!(
                    (a.id, a.kind, a.visual, a.state),
                    (b.id, b.kind, b.visual, b.state)
                );
            }
            client.push(sm.snapshot);
            // Simulate losing every third ack.
            if step % 3 != 2 {
                acked = Some(tick);
            }
        }
    }

    #[test]
    fn channels() {
        assert_eq!(
            Message::Ping {
                nonce: 0,
                client_time_ms: 0
            }
            .channel(),
            Channel::Unreliable
        );
        assert_eq!(
            Message::Hello(Hello::new("a", None)).channel(),
            Channel::Reliable
        );
    }

    #[test]
    fn handshake_version_and_magic_checks() {
        let mut old = Hello::new("pilot", None);
        old.protocol_version = PROTOCOL_VERSION + 1;
        let bytes = Message::Hello(old.clone()).encode().unwrap();
        assert_eq!(peek_hello_version(&bytes), Ok(PROTOCOL_VERSION + 1));
        assert_eq!(
            Message::decode(&bytes),
            Err(DecodeError::VersionMismatch {
                ours: PROTOCOL_VERSION,
                theirs: PROTOCOL_VERSION + 1
            })
        );
        let rj = accept_hello(&bytes).unwrap_err();
        assert_eq!(rj.reason, RejectReason::VersionMismatch);
        assert_eq!(rj.server_version, PROTOCOL_VERSION);
        assert!(matches!(
            check_hello(&old),
            Err(HandshakeError::VersionMismatch { .. })
        ));

        let mut bad_magic = Message::Hello(Hello::new("pilot", None)).encode().unwrap();
        bad_magic[1] ^= 0xFF;
        assert_eq!(Message::decode(&bad_magic), Err(DecodeError::BadMagic));
        assert_eq!(
            accept_hello(&bad_magic).unwrap_err().reason,
            RejectReason::BadHandshake
        );

        let good = Message::Hello(Hello::new("pilot", Some(0)))
            .encode()
            .unwrap();
        assert_eq!(accept_hello(&good).unwrap().name, "pilot");
        // Data hash: equal passes, different is a clear DataMismatch reject.
        let h = Hello::new("pilot", None).with_data_hash(42);
        assert!(check_data_hash(&h, 42).is_none());
        let rj = check_data_hash(&h, 43).unwrap();
        assert_eq!(rj.reason, RejectReason::DataMismatch);
        assert!(rj.message.contains("data"), "{}", rj.message);
        assert!(rj.message.len() <= MAX_REASON_LEN);
        let empty = Message::Hello(Hello::new("  ", None)).encode().unwrap();
        assert!(accept_hello(&empty).is_err());
        let ping = Message::Ping {
            nonce: 1,
            client_time_ms: 2,
        }
        .encode()
        .unwrap();
        assert!(accept_hello(&ping).is_err());

        // Client side.
        let rej = check_welcome(Message::Reject(rj)).unwrap_err();
        assert!(matches!(rej, HandshakeError::Rejected(_)));
        assert!(check_welcome(Message::Disconnect {
            reason: String::new()
        })
        .is_err());
        let mut w = welcome();
        w.protocol_version = 0;
        assert!(matches!(
            check_welcome(Message::Welcome(Box::new(w))),
            Err(HandshakeError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn garbage_never_panics() {
        let mut rng = Pcg32::new(99, 0);
        let valid = [
            Message::Hello(Hello::new("x", None).with_room(Some("R-1".into())))
                .encode()
                .unwrap(),
            Message::LobbyState(Box::new(crate::mode::LobbyStateMsg {
                next_mode: 3,
                players: vec![crate::mode::LobbyPlayer::default(); 3],
                ..Default::default()
            }))
            .encode()
            .unwrap(),
            Message::MatchResults(Box::new(crate::mode::MatchResultsMsg {
                team_scores: vec![1, 2, 3],
                rows: vec![crate::mode::ResultRow::default(); 2],
                ..Default::default()
            }))
            .encode()
            .unwrap(),
            Message::Welcome(Box::new(welcome())).encode().unwrap(),
            Message::Input(InputMsg {
                epoch: 0,
                ack_snapshot: Some(1),
                cmds: vec![input(9, 1), input(8, 2)],
            })
            .encode()
            .unwrap(),
            Message::Events(vec![NetEvent::Kill {
                tick: 1,
                killer: 2,
                victim: 3,
                weapon: 4,
            }])
            .encode()
            .unwrap(),
        ];
        for bytes in &valid {
            for cut in 0..bytes.len() {
                assert!(Message::decode(&bytes[..cut]).is_err());
            }
            let mut extra = bytes.clone();
            extra.push(0);
            assert!(Message::decode(&extra).is_err(), "trailing byte");
            for _ in 0..300 {
                let mut b = bytes.clone();
                let i = rng.below(b.len() as u32) as usize;
                b[i] = rng.next_u32() as u8;
                let _ = Message::decode(&b);
            }
        }
        for _ in 0..2000 {
            let len = rng.below(64) as usize;
            let b: Vec<u8> = (0..len).map(|_| rng.next_u32() as u8).collect();
            let _ = Message::decode(&b);
        }
        assert_eq!(Message::decode(&[200]), Err(DecodeError::UnknownTag(200)));
    }
}
