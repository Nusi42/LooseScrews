//! The seam between network transports and the simulation.
//!
//! An [`Endpoint`] accepts connections of one transport kind and, for each,
//! 1. waits (at most [`ConnSettings::handshake_timeout`]) for the first packet,
//! 2. calls [`Hub::open`], which checks the `Hello` (magic, protocol
//!    version) and hands a [`SimLink`](crate::link::SimLink) to the sim thread,
//! 3. pumps packets between the socket and the returned [`PeerLink`] until
//!    either side closes or the peer is silent for
//!    [`ConnSettings::idle_timeout`].
//!
//! The WebSocket endpoint ([`crate::http::HttpEndpoint`]) is the only one in
//! milestone 1. WebTransport (wave 4) implements [`Endpoint`] the same way:
//! reliable frames on a stream, [`Channel::Unreliable`](ls_net::Channel)
//! frames as datagrams (limit [`LinkConfig::max_unreliable_size`] to the
//! datagram size so oversized packets fall back to the stream), and the cert
//! hash published through [`CertHash`] for `GET /cert-hash`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::time::Duration;

use ls_net::{Message, Reject, RejectReason, PROTOCOL_VERSION};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::link::{link_pair, ConnInfo, LinkConfig, PeerLink};
use crate::sim::SimCommand;
use crate::stats::{inc, SharedStats};

/// Per-connection limits and timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnSettings {
    /// Time allowed from connect (after the HTTP upgrade) to the `Hello`.
    pub handshake_timeout: Duration,
    /// A joined client that sends nothing for this long is dropped (clients
    /// send inputs at 60 Hz).
    pub idle_timeout: Duration,
    /// Time connections get to flush after shutdown starts.
    pub shutdown_grace: Duration,
    pub link: LinkConfig,
}

impl Default for ConnSettings {
    fn default() -> Self {
        ConnSettings {
            handshake_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(2),
            link: LinkConfig::default(),
        }
    }
}

/// The certificate hash a WebTransport endpoint publishes (served at
/// `GET /cert-hash`; 404 while `None`).
pub type CertHash = Arc<RwLock<Option<String>>>;

/// Entry point for new connections of any transport; cheap to clone.
#[derive(Debug, Clone)]
pub struct Hub {
    commands: mpsc::Sender<SimCommand>,
    next_id: Arc<AtomicU64>,
    stats: Arc<SharedStats>,
    settings: ConnSettings,
}

impl Hub {
    pub(crate) fn new(
        commands: mpsc::Sender<SimCommand>,
        stats: Arc<SharedStats>,
        settings: ConnSettings,
    ) -> Self {
        Hub {
            commands,
            next_id: Arc::new(AtomicU64::new(1)),
            stats,
            settings,
        }
    }

    pub(crate) fn send_command(&self, cmd: SimCommand) -> Result<(), ServerGone> {
        self.commands.send(cmd).map_err(|_| ServerGone)
    }

    pub fn settings(&self) -> &ConnSettings {
        &self.settings
    }

    pub fn stats(&self) -> &Arc<SharedStats> {
        &self.stats
    }

    /// Checks the first packet of a connection. On success the player is
    /// queued for the sim thread (which sends `Welcome`, or `Reject` if the
    /// room is full) and the socket side of the link is returned. On failure
    /// returns the encoded `Reject` to send before closing.
    ///
    /// `max_unreliable_size` is the transport's datagram limit.
    pub fn open(
        &self,
        transport: &'static str,
        remote: SocketAddr,
        first_packet: &[u8],
        max_unreliable_size: usize,
    ) -> Result<PeerLink, Vec<u8>> {
        let reject = |r: Reject| {
            inc(&self.stats.rejected);
            Message::Reject(r).encode().unwrap_or_default()
        };
        let hello = ls_net::accept_hello(first_packet).map_err(reject)?;
        let info = ConnInfo {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            transport,
            remote,
        };
        let cfg = LinkConfig {
            max_unreliable_size: max_unreliable_size.min(self.settings.link.max_unreliable_size),
            ..self.settings.link
        };
        let (sim, peer) = link_pair(info, cfg, self.stats.links.clone());
        self.commands
            .send(SimCommand::Join { hello, link: sim })
            .map_err(|_| {
                reject(Reject {
                    reason: RejectReason::Other,
                    server_version: PROTOCOL_VERSION,
                    message: "server is shutting down".into(),
                })
            })?;
        inc(&self.stats.connections);
        Ok(peer)
    }
}

/// Everything an endpoint needs to run.
#[derive(Debug, Clone)]
pub struct EndpointContext {
    pub hub: Hub,
    /// Flips to `true` when the server shuts down: stop accepting, let
    /// connections flush for [`ConnSettings::shutdown_grace`], then return.
    pub shutdown: watch::Receiver<bool>,
    pub cert_hash: CertHash,
}

/// A network transport the server accepts players on (see the module docs).
pub trait Endpoint: Send + 'static {
    /// Transport name for logs.
    fn name(&self) -> &'static str;
    /// Address the endpoint listens on.
    fn local_addr(&self) -> std::io::Result<SocketAddr>;
    /// Starts accepting connections on the tokio runtime.
    fn spawn(self: Box<Self>, ctx: EndpointContext) -> JoinHandle<()>;
}

/// The sim thread has stopped and no longer takes commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerGone;
