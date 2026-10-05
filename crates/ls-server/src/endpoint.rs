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
//! Two endpoints exist: WebSocket ([`crate::http::HttpEndpoint`], TCP) and
//! WebTransport ([`crate::wt::WebTransportEndpoint`], UDP): reliable frames
//! on a bidirectional stream, [`Channel::Unreliable`](ls_net::Channel)
//! frames as datagrams (the link's [`LinkConfig::max_unreliable_size`] is the
//! datagram size, so oversized packets fall back to the stream), and its
//! certificate hash published through [`CertHash`] for `GET /cert-hash`.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use ls_net::{Message, Reject, RejectReason, PROTOCOL_VERSION};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::api::Social;
use crate::link::{link_pair, ConnInfo, LinkConfig, PeerLink};
use crate::rooms::{requested_room, Rooms, RouteError};
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

/// What a WebTransport endpoint publishes for browsers: the SHA-256 hash of
/// its self-signed certificate (for `serverCertificateHashes`) and where to
/// connect. Served as JSON at `GET /cert-hash` (see [`CertHashInfo::to_json`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertHashInfo {
    /// SHA-256 of the DER certificate.
    pub sha256: [u8; 32],
    /// UDP port of the WebTransport endpoint.
    pub port: u16,
    /// URL path of the WebTransport session.
    pub path: &'static str,
    /// End of the certificate's validity, Unix seconds.
    pub not_after_unix: i64,
}

impl CertHashInfo {
    /// The hash as lower-case hex (64 characters).
    pub fn hex(&self) -> String {
        self.sha256.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// `{"algorithm":"sha-256","hash":"<hex>","port":7777,"path":"/wt","expires":<unix s>}`
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"algorithm":"sha-256","hash":"{}","port":{},"path":"{}","expires":{}}}"#,
            self.hex(),
            self.port,
            self.path,
            self.not_after_unix
        )
    }
}

/// The certificate hash a WebTransport endpoint publishes (served at
/// `GET /cert-hash`; 404 while `None`). Replaced when the certificate rotates.
pub type CertHash = Arc<RwLock<Option<CertHashInfo>>>;

/// Entry point for new connections of any transport; cheap to clone. Routes
/// each connection to the room its `Hello` names (the public room without
/// one, see [`crate::rooms`]).
#[derive(Debug, Clone)]
pub struct Hub {
    rooms: Arc<Rooms>,
    /// Parties, presence and duels (`None`: no `/api` and no party codes,
    /// as in endpoint tests).
    social: Option<Arc<Social>>,
    next_id: Arc<AtomicU64>,
    stats: Arc<SharedStats>,
    settings: ConnSettings,
}

impl Hub {
    /// A hub with only a public room reachable through `commands`.
    #[cfg(test)]
    pub(crate) fn new(
        commands: std::sync::mpsc::Sender<SimCommand>,
        stats: Arc<SharedStats>,
        settings: ConnSettings,
    ) -> Self {
        let rooms = Arc::new(Rooms::public_only(commands, stats.clone()));
        Hub::with_rooms(rooms, stats, settings)
    }

    /// A hub routing to `rooms`; `stats` are the server-wide connection
    /// counters (the public room's).
    pub(crate) fn with_rooms(
        rooms: Arc<Rooms>,
        stats: Arc<SharedStats>,
        settings: ConnSettings,
    ) -> Self {
        Hub {
            rooms,
            social: None,
            next_id: Arc::new(AtomicU64::new(1)),
            stats,
            settings,
        }
    }

    /// The hub with the server's parties and duels (party routing, `/api`).
    pub(crate) fn with_social(mut self, social: Arc<Social>) -> Self {
        self.social = Some(social);
        self
    }

    /// Parties, presence and duels, if the server has them.
    pub fn social(&self) -> Option<&Arc<Social>> {
        self.social.as_ref()
    }

    /// Sends a command to the public room.
    pub(crate) fn send_command(&self, cmd: SimCommand) -> Result<(), ServerGone> {
        self.rooms.send(None, cmd).map_err(|_| ServerGone)
    }

    /// The room registry.
    pub fn rooms(&self) -> &Arc<Rooms> {
        &self.rooms
    }

    pub fn settings(&self) -> &ConnSettings {
        &self.settings
    }

    pub fn stats(&self) -> &Arc<SharedStats> {
        &self.stats
    }

    /// Checks the first packet of a connection. On success the player is
    /// queued for the sim thread of the room its `Hello` asks for (its
    /// party's room for a party code, see [`crate::party`]; which
    /// sends `Welcome`, or `Reject` if the room is full) and the socket side
    /// of the link is returned. On failure returns the encoded `Reject` to
    /// send before closing (also for an unknown room code).
    ///
    /// `max_unreliable_size` is the transport's datagram limit.
    pub fn open(
        &self,
        transport: &'static str,
        remote: SocketAddr,
        first_packet: &[u8],
        max_unreliable_size: usize,
    ) -> Result<PeerLink, Vec<u8>> {
        let mut hello = ls_net::accept_hello(first_packet).map_err(|r| self.reject(r))?;
        let mut room = requested_room(&hello);
        // A party code wins over a room code: the party's room.
        if let Some(text) = hello.party.clone().filter(|p| !p.trim().is_empty()) {
            let resolved = match &self.social {
                Some(s) => s.resolve_party(&text).map_err(|e| e.to_string()),
                None => Err(format!("unknown party {}", text.trim())),
            };
            match resolved {
                Ok((code, party_room)) => {
                    hello.party = Some(code.to_string());
                    room = party_room.map(|r| r.to_string());
                }
                Err(message) => {
                    return Err(self.reject(Reject {
                        reason: RejectReason::Other,
                        server_version: PROTOCOL_VERSION,
                        message,
                    }))
                }
            }
        } else {
            hello.party = None;
        }
        self.open_in(
            room.as_deref(),
            transport,
            remote,
            hello,
            max_unreliable_size,
        )
    }

    fn reject(&self, r: Reject) -> Vec<u8> {
        inc(&self.stats.rejected);
        Message::Reject(r).encode().unwrap_or_default()
    }

    /// [`Hub::open`] with an accepted `Hello` for the room `room` (a room
    /// code; `None` is the public room).
    pub fn open_in(
        &self,
        room: Option<&str>,
        transport: &'static str,
        remote: SocketAddr,
        hello: ls_net::Hello,
        max_unreliable_size: usize,
    ) -> Result<PeerLink, Vec<u8>> {
        let reject = |r: Reject| self.reject(r);
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
        self.rooms
            .send(room, SimCommand::Join { hello, link: sim })
            .map_err(|e| {
                let message = match e {
                    RouteError::UnknownRoom(code) => format!("unknown room {code}"),
                    RouteError::Gone => "server is shutting down".into(),
                };
                reject(Reject {
                    reason: RejectReason::Other,
                    server_version: PROTOCOL_VERSION,
                    message,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_hash_json() {
        let mut sha256 = [0u8; 32];
        sha256[0] = 0xab;
        sha256[31] = 0x01;
        let info = CertHashInfo {
            sha256,
            port: 7777,
            path: "/wt",
            not_after_unix: 1_700_000_000,
        };
        let hex = info.hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.starts_with("ab00") && hex.ends_with("0001"));
        assert_eq!(
            info.to_json(),
            format!(
                r#"{{"algorithm":"sha-256","hash":"{hex}","port":7777,"path":"/wt","expires":1700000000}}"#
            )
        );
    }
}
