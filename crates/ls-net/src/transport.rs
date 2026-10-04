//! The transport abstraction shared by the server, the web client and bots.
//!
//! [`Transport`] is deliberately poll-based and synchronous: `send_*` queue
//! bytes without blocking and [`Transport::recv`] returns `Ok(None)` when
//! nothing is waiting. That shape works everywhere:
//!
//! - **Native server (tokio):** socket tasks push incoming packets into a
//!   channel; the fixed-step sim thread drains it through `recv` each tick,
//!   and `send_*` push into an outgoing channel the socket task writes from.
//! - **Browser (wasm32):** JS `onmessage` / datagram reader callbacks push
//!   into a queue owned by the transport; the frame loop polls `recv`.
//!   No threads or async runtime are required.
//!
//! Implementations live in `ls-server` (WebSocket, WebTransport) and the web
//! client. This module provides [`MemoryTransport`], an in-process pair for
//! tests and bots, and helpers that send and receive [`Message`]s.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use ls_math::Pcg32;
use thiserror::Error;

use crate::error::{DecodeError, EncodeError, TransportError};
use crate::message::{Channel, Message};
use crate::snapshot::Baselines;

/// Largest unreliable payload that is safe for QUIC datagrams and UDP
/// without fragmentation.
pub const DEFAULT_MAX_DATAGRAM: usize = 1200;

/// A received packet and the channel it arrived on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub channel: Channel,
    pub data: Vec<u8>,
}

/// A bidirectional, message-oriented connection to one peer.
pub trait Transport {
    /// Sends a packet that may be lost, duplicated or reordered. Must not block.
    fn send_unreliable(&mut self, data: &[u8]) -> Result<(), TransportError>;

    /// Sends a packet that arrives exactly once and in order relative to
    /// other reliable packets. Must not block.
    fn send_reliable(&mut self, data: &[u8]) -> Result<(), TransportError>;

    /// Returns the next received packet, `Ok(None)` if none is waiting, or
    /// `Err(TransportError::Closed)` once the connection is closed and drained.
    fn recv(&mut self) -> Result<Option<Packet>, TransportError>;

    /// Largest payload `send_unreliable` accepts. A transport without a real
    /// unreliable channel (plain WebSocket) may send unreliable packets
    /// reliably and report a large limit.
    fn max_unreliable_size(&self) -> usize {
        DEFAULT_MAX_DATAGRAM
    }

    /// Whether the connection is still usable for sending.
    fn is_open(&self) -> bool;

    /// Closes the connection; later sends fail with `Closed`.
    fn close(&mut self);

    /// Sends on the given channel.
    fn send(&mut self, channel: Channel, data: &[u8]) -> Result<(), TransportError> {
        match channel {
            Channel::Unreliable => self.send_unreliable(data),
            Channel::Reliable => self.send_reliable(data),
        }
    }
}

/// Error of the message-level helpers.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum NetError {
    #[error(transparent)]
    Encode(#[from] EncodeError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Encodes `msg` (looking up snapshot baselines in `baselines`) and sends it
/// on its preferred channel. Unreliable messages too large for a datagram
/// fall back to the reliable channel.
pub fn send_message<T: Transport + ?Sized>(
    t: &mut T,
    msg: &Message,
    baselines: &dyn Baselines,
) -> Result<(), NetError> {
    let bytes = msg.encode_with(baselines)?;
    let channel = match msg.channel() {
        Channel::Unreliable if bytes.len() > t.max_unreliable_size() => Channel::Reliable,
        c => c,
    };
    t.send(channel, &bytes)?;
    Ok(())
}

/// Receives and decodes the next message, if any.
pub fn recv_message<T: Transport + ?Sized>(
    t: &mut T,
    baselines: &dyn Baselines,
) -> Result<Option<(Channel, Message)>, NetError> {
    match t.recv()? {
        Some(p) => Ok(Some((p.channel, Message::decode_with(&p.data, baselines)?))),
        None => Ok(None),
    }
}

/// Settings for the unreliable channel of a [`MemoryTransport`] pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkConditions {
    /// Probability in `0..=1` that an unreliable packet is dropped.
    pub loss: f32,
    /// Probability that an unreliable packet is delivered twice.
    pub duplicate: f32,
    /// Probability that an unreliable packet swaps places with the previous one.
    pub reorder: f32,
    /// Seed of the deterministic RNG that makes these decisions.
    pub seed: u64,
    pub max_datagram: usize,
}

impl Default for LinkConditions {
    fn default() -> Self {
        LinkConditions {
            loss: 0.0,
            duplicate: 0.0,
            reorder: 0.0,
            seed: 0,
            max_datagram: DEFAULT_MAX_DATAGRAM,
        }
    }
}

#[derive(Debug, Default)]
struct Queue {
    packets: VecDeque<Packet>,
    closed: bool,
}

/// One end of an in-process connection (single-threaded, works on wasm32).
/// Unreliable packets pass through a deterministic loss / duplication /
/// reordering model; reliable packets are always delivered in order.
#[derive(Debug)]
pub struct MemoryTransport {
    inbox: Rc<RefCell<Queue>>,
    outbox: Rc<RefCell<Queue>>,
    link: LinkConditions,
    rng: Pcg32,
    open: bool,
}

impl MemoryTransport {
    /// A connected pair with a perfect link.
    pub fn pair() -> (MemoryTransport, MemoryTransport) {
        Self::pair_with(LinkConditions::default())
    }

    /// A connected pair; both directions use `link` (with different RNG streams).
    pub fn pair_with(link: LinkConditions) -> (MemoryTransport, MemoryTransport) {
        let a_to_b = Rc::new(RefCell::new(Queue::default()));
        let b_to_a = Rc::new(RefCell::new(Queue::default()));
        let a = MemoryTransport {
            inbox: b_to_a.clone(),
            outbox: a_to_b.clone(),
            link,
            rng: Pcg32::new(link.seed, 1),
            open: true,
        };
        let b = MemoryTransport {
            inbox: a_to_b,
            outbox: b_to_a,
            link,
            rng: Pcg32::new(link.seed, 2),
            open: true,
        };
        (a, b)
    }

    fn check_open(&self) -> Result<(), TransportError> {
        if self.open && !self.outbox.borrow().closed {
            Ok(())
        } else {
            Err(TransportError::Closed)
        }
    }
}

impl Transport for MemoryTransport {
    fn send_unreliable(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.check_open()?;
        if data.len() > self.link.max_datagram {
            return Err(TransportError::TooLarge {
                len: data.len(),
                max: self.link.max_datagram,
            });
        }
        if self.rng.chance(self.link.loss) {
            return Ok(());
        }
        let packet = Packet {
            channel: Channel::Unreliable,
            data: data.to_vec(),
        };
        let mut q = self.outbox.borrow_mut();
        if self.rng.chance(self.link.duplicate) {
            q.packets.push_back(packet.clone());
        }
        q.packets.push_back(packet);
        if self.rng.chance(self.link.reorder) && q.packets.len() >= 2 {
            let n = q.packets.len();
            q.packets.swap(n - 1, n - 2);
        }
        Ok(())
    }

    fn send_reliable(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.check_open()?;
        self.outbox.borrow_mut().packets.push_back(Packet {
            channel: Channel::Reliable,
            data: data.to_vec(),
        });
        Ok(())
    }

    fn recv(&mut self) -> Result<Option<Packet>, TransportError> {
        let mut q = self.inbox.borrow_mut();
        match q.packets.pop_front() {
            Some(p) => Ok(Some(p)),
            None if q.closed || !self.open => Err(TransportError::Closed),
            None => Ok(None),
        }
    }

    fn max_unreliable_size(&self) -> usize {
        self.link.max_datagram
    }

    fn is_open(&self) -> bool {
        self.check_open().is_ok()
    }

    fn close(&mut self) {
        self.open = false;
        self.outbox.borrow_mut().closed = true;
        self.inbox.borrow_mut().closed = true;
    }
}

impl Drop for MemoryTransport {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{check_welcome, Hello, InputMsg, Welcome, PROTOCOL_VERSION};
    use crate::quant::QuantConfig;
    use crate::snapshot::{NoBaselines, SnapshotHistory};
    use crate::wire::{NetInput, PackedSnapshot};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn perfect_pair_delivers_in_order() {
        let (mut a, mut b) = MemoryTransport::pair();
        a.send_reliable(b"one").unwrap();
        a.send_unreliable(b"two").unwrap();
        b.send_reliable(b"back").unwrap();
        assert_eq!(b.recv().unwrap().unwrap().data, b"one");
        let p = b.recv().unwrap().unwrap();
        assert_eq!(
            (p.channel, p.data.as_slice()),
            (Channel::Unreliable, &b"two"[..])
        );
        assert_eq!(b.recv().unwrap(), None);
        assert_eq!(a.recv().unwrap().unwrap().data, b"back");
        a.close();
        assert!(!b.is_open());
        assert_eq!(b.send_reliable(b"x"), Err(TransportError::Closed));
        assert_eq!(b.recv(), Err(TransportError::Closed));
        assert!(matches!(
            a.send_unreliable(&[0; 2000]),
            Err(TransportError::Closed)
        ));
    }

    #[test]
    fn lossy_link_is_deterministic_and_reliable_channel_is_intact() {
        let link = LinkConditions {
            loss: 0.3,
            duplicate: 0.1,
            reorder: 0.1,
            seed: 42,
            ..Default::default()
        };
        let run = || {
            let (mut a, mut b) = MemoryTransport::pair_with(link);
            for i in 0..200u32 {
                a.send_unreliable(&i.to_le_bytes()).unwrap();
                if i % 10 == 0 {
                    a.send_reliable(&(1000 + i).to_le_bytes()).unwrap();
                }
            }
            let mut got = Vec::new();
            while let Some(p) = b.recv().unwrap() {
                got.push((p.channel, u32::from_le_bytes(p.data.try_into().unwrap())));
            }
            got
        };
        let got = run();
        assert_eq!(got, run(), "same seed, same outcome");
        let reliable: Vec<u32> = got
            .iter()
            .filter(|(c, _)| *c == Channel::Reliable)
            .map(|&(_, v)| v)
            .collect();
        assert_eq!(reliable, (0..20).map(|i| 1000 + i * 10).collect::<Vec<_>>());
        let unreliable = got
            .iter()
            .filter(|(c, _)| *c == Channel::Unreliable)
            .count();
        assert!((100..200).contains(&unreliable), "{unreliable}");
        let (mut a, _b) = MemoryTransport::pair_with(link);
        assert!(matches!(
            a.send_unreliable(&[0; 1201]),
            Err(TransportError::TooLarge { .. })
        ));
    }

    #[test]
    fn handshake_and_messages_over_a_transport() {
        let (mut client, mut server) = MemoryTransport::pair();
        send_message(
            &mut client,
            &Message::Hello(Hello::new("pilot", None)),
            &NoBaselines,
        )
        .unwrap();
        let p = server.recv().unwrap().unwrap();
        assert_eq!(p.channel, Channel::Reliable);
        let hello = crate::message::accept_hello(&p.data).unwrap();
        assert_eq!(hello.name, "pilot");
        let welcome = Welcome {
            epoch: 0,
            room_code: String::new(),
            protocol_version: PROTOCOL_VERSION,
            player_id: 0,
            team: 0,
            ship_type: 1,
            data_hash: 7,
            share_code: "LS-TEST".into(),
            ruleset_name: "paint_the_void".into(),
            ruleset: vec![],
            quant: QuantConfig::default(),
            server_tick: 1,
            edit_log: vec![],
            edit_count: 0,
        };
        send_message(
            &mut server,
            &Message::Welcome(Box::new(welcome.clone())),
            &NoBaselines,
        )
        .unwrap();
        let (ch, msg) = recv_message(&mut client, &NoBaselines).unwrap().unwrap();
        assert_eq!(ch, Channel::Reliable);
        assert_eq!(check_welcome(msg).unwrap(), welcome);

        let input = Message::Input(InputMsg {
            epoch: 0,
            ack_snapshot: None,
            cmds: vec![NetInput {
                tick: 5,
                ..Default::default()
            }],
        });
        send_message(&mut client, &input, &NoBaselines).unwrap();
        let history = SnapshotHistory::default();
        assert_eq!(
            recv_message(&mut server, &history).unwrap(),
            Some((Channel::Unreliable, input))
        );
        assert_eq!(recv_message(&mut server, &history).unwrap(), None);
    }

    #[test]
    fn oversized_unreliable_message_falls_back_to_reliable() {
        let (mut a, mut b) = MemoryTransport::pair_with(LinkConditions {
            max_datagram: 16,
            ..Default::default()
        });
        let msg = Message::Snapshot(Box::new(crate::message::SnapshotMsg {
            epoch: 0,
            baseline: None,
            snapshot: PackedSnapshot {
                scores: vec![1_000_000; 8],
                ..Default::default()
            },
        }));
        send_message(&mut a, &msg, &NoBaselines).unwrap();
        let (ch, back) = recv_message(&mut b, &NoBaselines).unwrap().unwrap();
        assert_eq!((ch, back), (Channel::Reliable, msg));
    }
}
