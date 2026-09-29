//! The bridge between the fixed-step simulation thread and async socket tasks.
//!
//! [`link_pair`] creates two halves of one client connection:
//!
//! - [`SimLink`] lives on the sim thread and implements [`ls_net::Transport`]
//!   (poll-based, never blocks): `send_*` queue frames, `recv` drains packets
//!   the socket task received.
//! - [`PeerLink`] lives in the socket task (WebSocket or WebTransport): it
//!   awaits outgoing frames with [`PeerLink::next_outgoing`] and forwards
//!   received packets with [`PeerLink::push_incoming`] (reliable, waits while
//!   the queue is full) or [`PeerLink::try_push_datagram`] (unreliable, dropped
//!   while the queue is full).
//!
//! Backpressure: at most [`LinkConfig::max_unreliable_queued`] unreliable
//! frames wait at once; queuing another drops the oldest waiting unreliable
//! frame (snapshots are delta-encoded against acknowledged baselines, so any
//! newer one supersedes it). Reliable frames are never dropped; if more than
//! [`LinkConfig::max_reliable_bytes`] pile up the peer is too slow and the link
//! closes. Incoming packets use a bounded channel, so a flooding client stalls
//! its own socket reads instead of the simulation.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ls_net::{Channel, Packet, Transport, TransportError};
use tokio::sync::{mpsc, Notify};

/// Server-unique connection id (not the wire player id).
pub type ConnId = u64;

/// Queue limits of one link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    /// Unreliable frames allowed to wait for the socket (older ones are dropped).
    pub max_unreliable_queued: usize,
    /// Reliable bytes allowed to wait before the link is closed as too slow.
    pub max_reliable_bytes: usize,
    /// Received packets waiting for the sim thread before socket reads stall.
    pub max_incoming_queued: usize,
    /// Largest unreliable payload (the transport's datagram limit).
    pub max_unreliable_size: usize,
}

impl Default for LinkConfig {
    fn default() -> Self {
        LinkConfig {
            // 8 frames: ~260 ms of 30 Hz snapshots (plus pongs, which share
            // the queue); anything older is useless.
            max_unreliable_queued: 8,
            // A Welcome with a long edit log is the largest reliable message.
            max_reliable_bytes: 16 << 20,
            max_incoming_queued: 256,
            max_unreliable_size: 1 << 20,
        }
    }
}

/// Who is on the other end of a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnInfo {
    pub id: ConnId,
    /// Transport name (`"websocket"` or `"webtransport"`).
    pub transport: &'static str,
    pub remote: SocketAddr,
}

/// A frame waiting to be written to the socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutFrame {
    pub channel: Channel,
    pub data: Vec<u8>,
}

/// Why a link stopped sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The server closed it (kick, disconnect, shutdown): flush, then close.
    Server,
    /// Too many reliable bytes queued: the peer cannot keep up.
    Overflow,
}

#[derive(Debug, Default)]
struct State {
    frames: VecDeque<OutFrame>,
    unreliable: usize,
    reliable_bytes: usize,
    closing: Option<CloseReason>,
    peer_gone: bool,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    notify: Notify,
    dropped_unreliable: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves plain data behind; keep going.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Counters shared by all links (for the periodic stats log).
#[derive(Debug, Default)]
pub struct LinkCounters {
    /// Unreliable frames dropped under backpressure.
    pub dropped_unreliable: AtomicU64,
}

/// Sim-thread half of a connection; see the module docs.
#[derive(Debug)]
pub struct SimLink {
    info: ConnInfo,
    shared: Arc<Shared>,
    counters: Arc<LinkCounters>,
    incoming: mpsc::Receiver<Packet>,
    cfg: LinkConfig,
    open: bool,
}

/// Socket-task half of a connection; see the module docs.
#[derive(Debug)]
pub struct PeerLink {
    info: ConnInfo,
    shared: Arc<Shared>,
    incoming: mpsc::Sender<Packet>,
}

/// Creates the two halves of one connection.
pub fn link_pair(
    info: ConnInfo,
    cfg: LinkConfig,
    counters: Arc<LinkCounters>,
) -> (SimLink, PeerLink) {
    let shared = Arc::new(Shared::default());
    let (tx, rx) = mpsc::channel(cfg.max_incoming_queued.max(1));
    (
        SimLink {
            info: info.clone(),
            shared: shared.clone(),
            counters,
            incoming: rx,
            cfg,
            open: true,
        },
        PeerLink {
            info,
            shared,
            incoming: tx,
        },
    )
}

impl SimLink {
    pub fn info(&self) -> &ConnInfo {
        &self.info
    }

    /// Unreliable frames this link dropped under backpressure.
    pub fn dropped_unreliable(&self) -> u64 {
        self.shared.dropped_unreliable.load(Ordering::Relaxed)
    }

    /// Why the link is closing, if it is.
    pub fn close_reason(&self) -> Option<CloseReason> {
        self.shared.lock().closing
    }

    fn check_open(&self, st: &State) -> Result<(), TransportError> {
        if self.open && st.closing.is_none() && !st.peer_gone {
            Ok(())
        } else {
            Err(TransportError::Closed)
        }
    }
}

impl Transport for SimLink {
    fn send_unreliable(&mut self, data: &[u8]) -> Result<(), TransportError> {
        if data.len() > self.cfg.max_unreliable_size {
            return Err(TransportError::TooLarge {
                len: data.len(),
                max: self.cfg.max_unreliable_size,
            });
        }
        let mut st = self.shared.lock();
        self.check_open(&st)?;
        if st.unreliable >= self.cfg.max_unreliable_queued.max(1) {
            if let Some(i) = st
                .frames
                .iter()
                .position(|f| f.channel == Channel::Unreliable)
            {
                st.frames.remove(i);
                st.unreliable -= 1;
                self.shared
                    .dropped_unreliable
                    .fetch_add(1, Ordering::Relaxed);
                self.counters
                    .dropped_unreliable
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        st.frames.push_back(OutFrame {
            channel: Channel::Unreliable,
            data: data.to_vec(),
        });
        st.unreliable += 1;
        drop(st);
        self.shared.notify.notify_one();
        Ok(())
    }

    fn send_reliable(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let mut st = self.shared.lock();
        self.check_open(&st)?;
        if st.reliable_bytes + data.len() > self.cfg.max_reliable_bytes {
            st.closing = Some(CloseReason::Overflow);
            drop(st);
            self.shared.notify.notify_one();
            return Err(TransportError::Other(
                "reliable send buffer overflow".into(),
            ));
        }
        st.frames.push_back(OutFrame {
            channel: Channel::Reliable,
            data: data.to_vec(),
        });
        st.reliable_bytes += data.len();
        drop(st);
        self.shared.notify.notify_one();
        Ok(())
    }

    fn recv(&mut self) -> Result<Option<Packet>, TransportError> {
        if !self.open {
            return Err(TransportError::Closed);
        }
        match self.incoming.try_recv() {
            Ok(packet) => Ok(Some(packet)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(TransportError::Closed),
        }
    }

    fn max_unreliable_size(&self) -> usize {
        self.cfg.max_unreliable_size
    }

    fn is_open(&self) -> bool {
        self.check_open(&self.shared.lock()).is_ok()
    }

    /// Stops accepting sends; the socket task writes what is queued, then
    /// closes the connection.
    fn close(&mut self) {
        self.open = false;
        let mut st = self.shared.lock();
        st.closing.get_or_insert(CloseReason::Server);
        drop(st);
        self.incoming.close();
        self.shared.notify.notify_one();
    }
}

impl Drop for SimLink {
    fn drop(&mut self) {
        self.close();
    }
}

impl PeerLink {
    pub fn info(&self) -> &ConnInfo {
        &self.info
    }

    /// The next frame to write, or `None` once the sim side closed the link
    /// and everything queued was taken (then close the socket). On
    /// [`CloseReason::Overflow`] the queue is abandoned immediately.
    pub async fn next_outgoing(&self) -> Option<OutFrame> {
        loop {
            {
                let mut st = self.shared.lock();
                if st.closing == Some(CloseReason::Overflow) {
                    return None;
                }
                if let Some(f) = st.frames.pop_front() {
                    match f.channel {
                        Channel::Unreliable => st.unreliable -= 1,
                        Channel::Reliable => st.reliable_bytes -= f.data.len(),
                    }
                    return Some(f);
                }
                if st.closing.is_some() {
                    return None;
                }
            }
            // `notify_one` stores a permit when nobody waits, so a frame
            // queued between the check above and this await is not missed.
            self.shared.notify.notified().await;
        }
    }

    /// Takes a frame if one is queued right now (for batching writes).
    pub fn try_next_outgoing(&self) -> Option<OutFrame> {
        let mut st = self.shared.lock();
        if st.closing == Some(CloseReason::Overflow) {
            return None;
        }
        let f = st.frames.pop_front()?;
        match f.channel {
            Channel::Unreliable => st.unreliable -= 1,
            Channel::Reliable => st.reliable_bytes -= f.data.len(),
        }
        Some(f)
    }

    /// Hands a received packet to the sim thread. Waits while the incoming
    /// queue is full; fails once the sim side closed the link.
    pub async fn push_incoming(&self, data: Vec<u8>) -> Result<(), TransportError> {
        self.incoming
            .send(Packet {
                channel: Channel::Reliable,
                data,
            })
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Hands a received datagram to the sim thread without waiting: returns
    /// `Ok(false)` if the incoming queue is full and the datagram was
    /// dropped (inputs repeat the newest commands, so the next one covers
    /// it); fails once the sim side closed the link.
    pub fn try_push_datagram(&self, data: Vec<u8>) -> Result<bool, TransportError> {
        match self.incoming.try_send(Packet {
            channel: Channel::Unreliable,
            data,
        }) {
            Ok(()) => Ok(true),
            Err(mpsc::error::TrySendError::Full(_)) => Ok(false),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(TransportError::Closed),
        }
    }

    /// Why the sim side closed the link, if it did.
    pub fn close_reason(&self) -> Option<CloseReason> {
        self.shared.lock().closing
    }
}

impl Drop for PeerLink {
    fn drop(&mut self) {
        self.shared.lock().peer_gone = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(cfg: LinkConfig) -> (SimLink, PeerLink) {
        let info = ConnInfo {
            id: 1,
            transport: "test",
            remote: "127.0.0.1:1".parse().unwrap(),
        };
        link_pair(info, cfg, Arc::new(LinkCounters::default()))
    }

    #[tokio::test]
    async fn frames_flow_both_ways_in_order() {
        let (mut sim, peer) = pair(LinkConfig::default());
        sim.send_reliable(b"welcome").unwrap();
        sim.send_unreliable(b"snap").unwrap();
        let a = peer.next_outgoing().await.unwrap();
        let b = peer.next_outgoing().await.unwrap();
        assert_eq!(
            (a.channel, a.data.as_slice()),
            (Channel::Reliable, &b"welcome"[..])
        );
        assert_eq!(
            (b.channel, b.data.as_slice()),
            (Channel::Unreliable, &b"snap"[..])
        );
        assert_eq!(sim.recv().unwrap(), None);
        peer.push_incoming(b"input".to_vec()).await.unwrap();
        assert_eq!(
            sim.recv().unwrap().unwrap(),
            Packet {
                channel: Channel::Reliable,
                data: b"input".to_vec()
            }
        );
        assert_eq!(peer.try_push_datagram(b"dgram".to_vec()), Ok(true));
        assert_eq!(
            sim.recv().unwrap().unwrap(),
            Packet {
                channel: Channel::Unreliable,
                data: b"dgram".to_vec()
            }
        );
        drop(peer);
        assert_eq!(sim.recv(), Err(TransportError::Closed));
        assert!(!sim.is_open());
        assert_eq!(sim.send_reliable(b"x"), Err(TransportError::Closed));
    }

    #[tokio::test]
    async fn backpressure_drops_oldest_unreliable_but_keeps_reliable() {
        let cfg = LinkConfig {
            max_unreliable_queued: 2,
            ..LinkConfig::default()
        };
        let (mut sim, peer) = pair(cfg);
        sim.send_unreliable(b"u1").unwrap();
        sim.send_reliable(b"r1").unwrap();
        sim.send_unreliable(b"u2").unwrap();
        sim.send_unreliable(b"u3").unwrap(); // drops u1
        sim.send_reliable(b"r2").unwrap();
        sim.send_unreliable(b"u4").unwrap(); // drops u2
        assert_eq!(sim.dropped_unreliable(), 2);
        let mut got = Vec::new();
        while let Some(f) = peer.try_next_outgoing() {
            got.push(String::from_utf8(f.data).unwrap());
        }
        assert_eq!(got, ["r1", "u3", "r2", "u4"]);
    }

    #[tokio::test]
    async fn reliable_overflow_closes_the_link() {
        let cfg = LinkConfig {
            max_reliable_bytes: 10,
            ..LinkConfig::default()
        };
        let (mut sim, peer) = pair(cfg);
        sim.send_reliable(&[0; 8]).unwrap();
        assert!(sim.send_reliable(&[0; 8]).is_err());
        assert_eq!(sim.close_reason(), Some(CloseReason::Overflow));
        assert!(!sim.is_open());
        assert_eq!(peer.next_outgoing().await, None);
    }

    #[tokio::test]
    async fn close_flushes_queued_frames_first() {
        let (mut sim, peer) = pair(LinkConfig::default());
        sim.send_reliable(b"bye").unwrap();
        sim.close();
        assert_eq!(peer.next_outgoing().await.unwrap().data, b"bye");
        assert_eq!(peer.next_outgoing().await, None);
        assert!(peer.push_incoming(vec![1]).await.is_err());
        assert!(matches!(
            sim.send_unreliable(&[0; 4]),
            Err(TransportError::Closed)
        ));
    }

    #[tokio::test]
    async fn waiting_reader_wakes_up_on_send() {
        let (mut sim, peer) = pair(LinkConfig::default());
        let reader = tokio::spawn(async move { peer.next_outgoing().await });
        tokio::task::yield_now().await;
        sim.send_unreliable(b"late").unwrap();
        let f = tokio::time::timeout(std::time::Duration::from_secs(5), reader)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(f.unwrap().data, b"late");
        let _ = sim;
    }

    #[tokio::test]
    async fn datagrams_are_dropped_when_the_incoming_queue_is_full() {
        let cfg = LinkConfig {
            max_incoming_queued: 2,
            ..LinkConfig::default()
        };
        let (mut sim, peer) = pair(cfg);
        assert_eq!(peer.try_push_datagram(vec![1]), Ok(true));
        assert_eq!(peer.try_push_datagram(vec![2]), Ok(true));
        assert_eq!(peer.try_push_datagram(vec![3]), Ok(false));
        assert_eq!(sim.recv().unwrap().unwrap().data, [1]);
        assert_eq!(sim.recv().unwrap().unwrap().data, [2]);
        assert_eq!(sim.recv().unwrap(), None);
        sim.close();
        assert_eq!(peer.try_push_datagram(vec![4]), Err(TransportError::Closed));
    }

    #[test]
    fn oversized_unreliable_is_rejected() {
        let cfg = LinkConfig {
            max_unreliable_size: 4,
            ..LinkConfig::default()
        };
        let (mut sim, _peer) = pair(cfg);
        assert!(matches!(
            sim.send_unreliable(&[0; 5]),
            Err(TransportError::TooLarge { len: 5, max: 4 })
        ));
    }
}
