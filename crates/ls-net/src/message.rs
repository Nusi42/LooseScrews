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
//! 2. Server answers [`Welcome`] (share code, rule set, player id, team,
//!    quantization config, full terrain edit log) or [`Reject`].
//! 3. Then the client streams [`InputMsg`] (unreliable, 60 Hz, each carrying
//!    the last three commands) and the server streams snapshots (unreliable,
//!    30 Hz), [`Message::Events`] (reliable) and answers pings.

use ls_math::{Aabb, TerrainEdit, Vec3};

use crate::bits::{BitReader, BitWriter};
use crate::error::{DecodeError, EncodeError, HandshakeError};
use crate::quant::QuantConfig;
use crate::snapshot::{check_edit, read_snapshot, write_snapshot, Baselines, NoBaselines};
use crate::wire::{
    read_edit, write_edit, NetEvent, NetInput, PackedSnapshot, TerrainEditBits, EDIT_MIN_BITS,
    EVENT_MIN_BITS, MAX_SHIPS,
};

/// Bumped whenever the wire format changes incompatibly.
pub const PROTOCOL_VERSION: u16 = 1;
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
}

impl Hello {
    pub fn new(name: impl Into<String>, preferred_team: Option<u8>) -> Self {
        Hello {
            protocol_version: PROTOCOL_VERSION,
            name: name.into(),
            preferred_team,
        }
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
    /// Every terrain edit so far, in order; `edit_log.len()` is the sequence
    /// number of the next edit.
    pub edit_log: Vec<TerrainEdit>,
}

/// Why the server refused or dropped a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    VersionMismatch,
    ServerFull,
    BadHandshake,
    Kicked,
    Other,
}

impl RejectReason {
    fn to_u8(self) -> u8 {
        match self {
            RejectReason::VersionMismatch => 0,
            RejectReason::ServerFull => 1,
            RejectReason::BadHandshake => 2,
            RejectReason::Kicked => 3,
            RejectReason::Other => 4,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => RejectReason::VersionMismatch,
            1 => RejectReason::ServerFull,
            2 => RejectReason::BadHandshake,
            3 => RejectReason::Kicked,
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
                w.write_u8(tag::HELLO);
                w.write_u32(MAGIC);
                w.write_u16(h.protocol_version);
                w.write_str(&h.name);
                w.write_bool(h.preferred_team.is_some());
                if let Some(t) = h.preferred_team {
                    w.write_u8(t);
                }
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
                if wl.player_id >= MAX_SHIPS {
                    return Err(EncodeError::OutOfRange("player id"));
                }
                wl.quant.validate().map_err(EncodeError::OutOfRange)?;
                w.write_u8(tag::WELCOME);
                w.write_u16(wl.protocol_version);
                w.write_u8(wl.player_id);
                w.write_u8(wl.team);
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
        }
        Ok(w.finish())
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
                Message::Hello(Hello {
                    protocol_version: version,
                    name,
                    preferred_team,
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
                Message::Welcome(Box::new(Welcome {
                    protocol_version: version,
                    player_id,
                    team,
                    share_code,
                    ruleset_name,
                    ruleset,
                    quant,
                    server_tick,
                    edit_log,
                }))
            }
            tag::REJECT => Message::Reject(Reject {
                server_version: r.read_u16()?,
                reason: RejectReason::from_u8(r.read_u8()?),
                message: r.read_string(MAX_REASON_LEN)?,
            }),
            tag::INPUT => {
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
                Message::Input(InputMsg { ack_snapshot, cmds })
            }
            tag::SNAPSHOT => {
                let (snapshot, baseline) = read_snapshot(&mut r, baselines)?;
                Message::Snapshot(Box::new(SnapshotMsg { baseline, snapshot }))
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
    use crate::wire::{ship_flags, NetProjectile, NetShip, ShipStatus, Snapshot};
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
            protocol_version: PROTOCOL_VERSION,
            player_id: 3,
            team: 1,
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
                },
                TerrainEdit::AddSphere {
                    center: Vec3::new(0.0, -60.0, 100.0),
                    radius: 4.0,
                },
            ],
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
    fn welcome_round_trip_with_edit_log() {
        let w = welcome();
        let bytes = round_trip(&Message::Welcome(Box::new(w.clone())));
        // Each edit costs about 17 bytes.
        assert!(bytes.len() < 200, "{}", bytes.len());
        let back = check_welcome(Message::decode(&bytes).unwrap()).unwrap();
        assert_eq!(back.edit_log, w.edit_log);
        // A big log still round-trips.
        let mut big = welcome();
        big.edit_log = (0..5000)
            .map(|i| TerrainEdit::SubtractSphere {
                center: Vec3::new(i as f32 * 0.1, 0.0, -(i as f32)),
                radius: 3.0,
            })
            .collect();
        round_trip(&Message::Welcome(Box::new(big)));
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
                ack_snapshot: Some(998),
                cmds,
            });
            let bytes = round_trip(&msg);
            assert_eq!(
                bytes.len(),
                (8 + 1 + 32 + 2 + 32 + 88 + (n as usize - 1) * (8 + 1 + 88)).div_ceil(8)
            );
        }
        // Held controls: older commands cost 9 bits each.
        let held = input(50, 1);
        let msg = Message::Input(InputMsg {
            ack_snapshot: None,
            cmds: vec![
                held,
                NetInput { tick: 49, ..held },
                NetInput { tick: 47, ..held },
            ],
        });
        let bytes = round_trip(&msg);
        assert_eq!(bytes.len(), (8 + 1 + 2 + 32 + 88 + 2 * 9_usize).div_ceil(8));
        // Ticks may wrap around.
        round_trip(&Message::Input(InputMsg {
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
            tick,
            ack_input: tick + 4,
            edit_base: 3,
            edits: vec![TerrainEdit::Paint {
                center: Vec3::new(1.0, 2.0, 3.0),
                radius: 2.0,
                team: 0,
            }],
            ships,
            projectiles: vec![NetProjectile {
                id: 77,
                kind: 1,
                owner: 3,
                pos: Vec3::new(t + 10.0, 10.0, 10.0),
                vel: Vec3::new(0.0, 0.0, -180.0),
            }],
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
            Message::Hello(Hello::new("x", None)).encode().unwrap(),
            Message::Welcome(Box::new(welcome())).encode().unwrap(),
            Message::Input(InputMsg {
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
