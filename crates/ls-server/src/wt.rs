//! WebTransport connections (HTTP/3 over QUIC, `wtransport`) on a UDP port.
//!
//! Wire mapping, one WebTransport session per player at [`PATH`]:
//!
//! - **Reliable** packets travel on one bidirectional stream the client
//!   opens. Each packet is a frame: a 4-byte big-endian length, then the
//!   encoded `ls-net` message ([`append_frame`], [`FrameDecoder`]). The first
//!   client frame must be the `Hello`; the server answers on the same stream.
//! - **Unreliable** packets (snapshots, pongs; inputs and pings from the
//!   client) are datagrams, one raw `ls-net` message each. The link's
//!   datagram limit is the session's `max_datagram_size` at accept time (QUIC
//!   never shrinks it below that), so larger unreliable packets go on the
//!   stream instead; a datagram the connection still refuses (too large, or
//!   the peer disabled datagrams) falls back to the stream as well.
//!
//! Certificates: a fresh self-signed ECDSA P-256 certificate is generated at
//! startup, valid from 1 h ago for [`CERT_VALIDITY`] (13 days; browsers accept
//! at most 14 for `serverCertificateHashes`). Its SHA-256 hash is published
//! through [`crate::endpoint::CertHash`] (`GET /cert-hash` on the HTTP port).
//! Every [`DEFAULT_ROTATE_AFTER`] a new certificate replaces it for new
//! sessions; running sessions are unaffected, and clients fetch the hash
//! right before connecting.
//!
//! Timeouts follow [`ConnSettings`](crate::endpoint::ConnSettings): the
//! `Hello` must arrive within the handshake timeout, and a player from whom
//! nothing (stream or datagram) arrives for the idle timeout is dropped.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ls_net::Channel;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tracing::{debug, info, warn};
use wtransport::endpoint::endpoint_side::Server as ServerSide;
use wtransport::endpoint::IncomingSession;
use wtransport::error::SendDatagramError;
use wtransport::tls::self_signed::time::{Duration as TimeDuration, OffsetDateTime};
use wtransport::{Connection, Identity, RecvStream, SendStream, VarInt};

use crate::endpoint::{CertHashInfo, Endpoint, EndpointContext, Hub};
use crate::link::{CloseReason, OutFrame, PeerLink};
use crate::stats::inc;

/// Transport name in logs.
pub const TRANSPORT: &str = "webtransport";

/// URL path of the game session (`https://host:port/wt`); `/` is accepted too.
pub const PATH: &str = "/wt";

/// Largest frame accepted from a client (same limit as WebSocket messages).
pub const MAX_CLIENT_FRAME: usize = crate::ws::MAX_CLIENT_MESSAGE;

/// Length of the certificate validity period.
pub const CERT_VALIDITY: Duration = Duration::from_secs(13 * 24 * 3600);

/// How long a certificate is used for new sessions before a new one is made.
pub const DEFAULT_ROTATE_AFTER: Duration = Duration::from_secs(6 * 24 * 3600);

/// Length prefix size of a stream frame.
pub const FRAME_HEADER: usize = 4;

/// Subject alternative names of the self-signed certificate (browsers do not
/// check them with `serverCertificateHashes`; other clients might).
const CERT_SANS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// QUIC keep-alive and idle limits (the application idle timeout from
/// `ConnSettings` is enforced separately on received packets).
const QUIC_KEEP_ALIVE: Duration = Duration::from_secs(4);
const QUIC_IDLE: Duration = Duration::from_secs(30);

/// Application close codes (for logs on the client side).
mod code {
    pub const NORMAL: u32 = 0;
    pub const TOO_SLOW: u32 = 1;
    pub const IDLE: u32 = 2;
    pub const SHUTDOWN: u32 = 3;
    pub const PROTOCOL: u32 = 4;
    pub const REJECTED: u32 = 5;
}

/// Appends one length-prefixed frame to `buf`.
pub fn append_frame(buf: &mut Vec<u8>, data: &[u8]) {
    let len = u32::try_from(data.len()).expect("frames are far below 4 GiB");
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(data);
}

/// A malformed frame stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("frame of {len} bytes exceeds the limit of {max}")]
pub struct FrameTooLarge {
    pub len: usize,
    pub max: usize,
}

/// Reassembles length-prefixed frames from stream reads of any size.
#[derive(Debug)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    start: usize,
    max: usize,
}

impl FrameDecoder {
    /// A decoder that refuses frames larger than `max` bytes.
    pub fn new(max: usize) -> Self {
        FrameDecoder {
            buf: Vec::new(),
            start: 0,
            max,
        }
    }

    /// Adds received bytes.
    pub fn extend(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame, if one is buffered.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameTooLarge> {
        let rest = &self.buf[self.start..];
        let Some(header) = rest.get(..FRAME_HEADER) else {
            return Ok(None);
        };
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if len > self.max {
            return Err(FrameTooLarge { len, max: self.max });
        }
        let Some(frame) = rest.get(FRAME_HEADER..FRAME_HEADER + len) else {
            return Ok(None);
        };
        let frame = frame.to_vec();
        self.start += FRAME_HEADER + len;
        // Compact now and then so the buffer does not grow without bound.
        if self.start > 64 * 1024 && self.start * 2 > self.buf.len() {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        Ok(Some(frame))
    }

    /// Bytes buffered but not yet returned as frames.
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }
}

/// A generated certificate and what gets published about it.
struct Cert {
    identity: Identity,
    sha256: [u8; 32],
    not_after_unix: i64,
}

/// Generates a self-signed ECDSA P-256 certificate valid from 1 h before
/// `now` (clock skew) for [`CERT_VALIDITY`].
fn generate_cert(now: OffsetDateTime) -> std::io::Result<Cert> {
    let not_before = now - TimeDuration::hours(1);
    let not_after = not_before + TimeDuration::seconds(CERT_VALIDITY.as_secs() as i64);
    let identity = Identity::self_signed_builder()
        .subject_alt_names(CERT_SANS)
        .validity_period(not_before, not_after)
        .build()
        .map_err(|e| std::io::Error::other(format!("self-signed certificate: {e}")))?;
    let sha256 = *identity.certificate_chain().as_slice()[0].hash().as_ref();
    Ok(Cert {
        identity,
        sha256,
        not_after_unix: not_after.unix_timestamp(),
    })
}

fn server_config(
    bind: SocketAddr,
    identity: Identity,
    dual_stack: bool,
) -> std::io::Result<wtransport::ServerConfig> {
    let builder = wtransport::ServerConfig::builder();
    let builder = if dual_stack {
        builder.with_bind_default(bind.port())
    } else {
        builder.with_bind_address(bind)
    };
    Ok(builder
        .with_identity(identity)
        .keep_alive_interval(Some(QUIC_KEEP_ALIVE))
        .max_idle_timeout(Some(QUIC_IDLE))
        .map_err(|e| std::io::Error::other(format!("idle timeout: {e}")))?
        .build())
}

/// WebTransport endpoint on a UDP port; see the module docs.
pub struct WebTransportEndpoint {
    endpoint: wtransport::Endpoint<ServerSide>,
    dual_stack: bool,
    info: CertHashInfo,
    rotate_after: Duration,
}

impl std::fmt::Debug for WebTransportEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebTransportEndpoint")
            .field("local_addr", &self.endpoint.local_addr().ok())
            .field("cert", &self.info)
            .finish()
    }
}

impl WebTransportEndpoint {
    /// Generates the certificate and binds the UDP socket. `0.0.0.0` binds
    /// all IPv4 and IPv6 addresses (browsers may resolve `localhost` to
    /// `::1`), falling back to IPv4 only. Needs a tokio runtime.
    pub fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let cert = generate_cert(OffsetDateTime::now_utc())?;
        let want_dual = addr.ip() == IpAddr::V4(Ipv4Addr::UNSPECIFIED);
        let (endpoint, dual_stack) = if want_dual {
            let dual = server_config(addr, cert.identity.clone_identity(), true)
                .and_then(wtransport::Endpoint::server);
            match dual {
                Ok(e) => (e, true),
                Err(e) => {
                    debug!(error = %e, "dual-stack UDP bind failed; using IPv4 only");
                    let cfg = server_config(addr, cert.identity.clone_identity(), false)?;
                    (wtransport::Endpoint::server(cfg)?, false)
                }
            }
        } else {
            let cfg = server_config(addr, cert.identity.clone_identity(), false)?;
            (wtransport::Endpoint::server(cfg)?, false)
        };
        let port = endpoint.local_addr()?.port();
        Ok(WebTransportEndpoint {
            endpoint,
            dual_stack,
            info: CertHashInfo {
                sha256: cert.sha256,
                port,
                path: PATH,
                not_after_unix: cert.not_after_unix,
            },
            rotate_after: DEFAULT_ROTATE_AFTER,
        })
    }

    /// Changes how often the certificate is replaced (tests).
    pub fn with_rotation(mut self, rotate_after: Duration) -> Self {
        self.rotate_after = rotate_after.max(Duration::from_millis(100));
        self
    }

    /// The published certificate hash and port.
    pub fn cert_hash(&self) -> &CertHashInfo {
        &self.info
    }

    /// Replaces the certificate for new sessions.
    fn rotate(&mut self) -> std::io::Result<()> {
        let cert = generate_cert(OffsetDateTime::now_utc())?;
        let local = self.endpoint.local_addr()?;
        let cfg = server_config(local, cert.identity, self.dual_stack)?;
        self.endpoint.reload_config(cfg, false)?;
        self.info.sha256 = cert.sha256;
        self.info.not_after_unix = cert.not_after_unix;
        Ok(())
    }
}

fn publish(ctx: &EndpointContext, info: Option<CertHashInfo>) {
    if let Ok(mut h) = ctx.cert_hash.write() {
        *h = info;
    }
}

impl Endpoint for WebTransportEndpoint {
    fn name(&self) -> &'static str {
        TRANSPORT
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    fn spawn(mut self: Box<Self>, ctx: EndpointContext) -> JoinHandle<()> {
        publish(&ctx, Some(self.info.clone()));
        info!(
            port = self.info.port,
            cert_sha256 = %self.info.hex(),
            "webtransport listening (certificate hash at /cert-hash)"
        );
        tokio::spawn(async move {
            let mut shutdown = ctx.shutdown.clone();
            let grace = ctx.hub.settings().shutdown_grace;
            let mut conns = JoinSet::new();
            let mut rotate_at = tokio::time::Instant::now() + self.rotate_after;
            loop {
                tokio::select! {
                    incoming = self.endpoint.accept() => {
                        conns.spawn(serve(incoming, ctx.hub.clone(), ctx.shutdown.clone()));
                    }
                    Some(_) = conns.join_next(), if !conns.is_empty() => {}
                    _ = tokio::time::sleep_until(rotate_at) => {
                        rotate_at = tokio::time::Instant::now() + self.rotate_after;
                        match self.rotate() {
                            Ok(()) => {
                                info!(cert_sha256 = %self.info.hex(), "webtransport certificate rotated");
                                publish(&ctx, Some(self.info.clone()));
                            }
                            Err(e) => warn!(error = %e, "webtransport certificate rotation failed"),
                        }
                    }
                    _ = async { let _ = shutdown.wait_for(|s| *s).await; } => break,
                }
            }
            publish(&ctx, None);
            // Sessions close themselves after the grace period.
            let drained = tokio::time::timeout(grace * 2, async {
                while conns.join_next().await.is_some() {}
            })
            .await;
            if drained.is_err() {
                conns.abort_all();
            }
            self.endpoint
                .close(VarInt::from_u32(code::SHUTDOWN), b"server stopped");
            let _ = tokio::time::timeout(grace, self.endpoint.wait_idle()).await;
            debug!("webtransport endpoint stopped");
        })
    }
}

/// Why a session ended (for logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    ServerClosed,
    Overflow,
    PeerClosed,
    IdleTimeout,
    Protocol,
    Error,
    Shutdown,
}

/// Runs one WebTransport session from the QUIC handshake to the end.
async fn serve(incoming: IncomingSession, hub: Hub, mut shutdown: watch::Receiver<bool>) {
    let settings = *hub.settings();
    // 1. QUIC + HTTP/3 handshake and the session request.
    let request = match tokio::time::timeout(settings.handshake_timeout, incoming).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            debug!(error = %e, "webtransport handshake failed");
            return;
        }
        Err(_) => {
            inc(&hub.stats().timeouts);
            debug!("webtransport handshake timed out");
            return;
        }
    };
    let path = request.path().split(['?', '#']).next().unwrap_or("");
    if path != PATH && path != "/" {
        debug!(path = %request.path(), "webtransport session for an unknown path");
        request.not_found().await;
        return;
    }
    let connection = match request.accept().await {
        Ok(c) => c,
        Err(e) => {
            debug!(error = %e, "webtransport session accept failed");
            return;
        }
    };
    let remote = connection.remote_address();

    // 2. The client opens the stream and sends `Hello` as its first frame.
    let mut decoder = FrameDecoder::new(MAX_CLIENT_FRAME);
    let opened = tokio::time::timeout(settings.handshake_timeout, async {
        let (send, mut recv) = connection.accept_bi().await.ok()?;
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            match decoder.next_frame() {
                Ok(Some(hello)) => return Some((send, recv, hello)),
                Ok(None) => {}
                Err(_) => return None,
            }
            let n = recv.read(&mut chunk).await.ok()??;
            decoder.extend(&chunk[..n]);
        }
    })
    .await;
    let (mut send, recv, hello) = match opened {
        Ok(Some(x)) => x,
        Ok(None) => {
            debug!(%remote, "webtransport session ended before Hello");
            connection.close(VarInt::from_u32(code::PROTOCOL), b"expected Hello");
            return;
        }
        Err(_) => {
            inc(&hub.stats().timeouts);
            info!(%remote, "handshake timed out");
            connection.close(VarInt::from_u32(code::IDLE), b"handshake timeout");
            return;
        }
    };
    let max_datagram = connection.max_datagram_size().unwrap_or(0);
    let peer = match hub.open(TRANSPORT, remote, &hello, max_datagram) {
        Ok(p) => p,
        Err(reject) => {
            info!(%remote, "handshake rejected");
            let mut buf = Vec::new();
            append_frame(&mut buf, &reject);
            let _ = tokio::time::timeout(settings.shutdown_grace, async {
                if send.write_all(&buf).await.is_ok() {
                    let _ = send.finish().await;
                }
            })
            .await;
            connection.close(VarInt::from_u32(code::REJECTED), b"rejected");
            return;
        }
    };
    let conn = peer.info().id;
    debug!(conn, %remote, max_datagram, "webtransport handshake done");

    // 3. Pump until one side stops.
    let end = pump(
        &connection,
        &peer,
        &mut send,
        recv,
        decoder,
        max_datagram,
        settings.idle_timeout,
        async {
            let _ = shutdown.wait_for(|s| *s).await;
            // Give the sim thread time to queue `Disconnect` and close links.
            tokio::time::sleep(settings.shutdown_grace).await;
        },
    )
    .await;
    if end == End::IdleTimeout {
        inc(&hub.stats().timeouts);
    }
    info!(conn, %remote, reason = ?end, "webtransport closed");
    // Dropping the peer link tells the sim thread the player is gone.
    drop(peer);
    let (code, reason): (u32, &[u8]) = match end {
        End::ServerClosed => {
            // Deliver what was written (the final `Disconnect`) first.
            let _ = tokio::time::timeout(settings.shutdown_grace, send.finish()).await;
            (code::NORMAL, b"closed by server")
        }
        End::Overflow => (code::TOO_SLOW, b"client too slow"),
        End::IdleTimeout => (code::IDLE, b"idle timeout"),
        End::Shutdown => (code::SHUTDOWN, b"server shutting down"),
        End::Protocol => (code::PROTOCOL, b"bad frame"),
        End::PeerClosed | End::Error => (code::NORMAL, b""),
    };
    connection.close(VarInt::from_u32(code), reason);
}

/// Moves packets between the session and the link; returns why it stopped.
#[allow(clippy::too_many_arguments)]
async fn pump(
    connection: &Connection,
    peer: &PeerLink,
    send: &mut SendStream,
    mut recv: RecvStream,
    mut decoder: FrameDecoder,
    max_datagram: usize,
    idle_timeout: Duration,
    stop: impl std::future::Future<Output = ()>,
) -> End {
    // Nanoseconds since `start` of the last received packet.
    let start = Instant::now();
    let last_rx = AtomicU64::new(0);
    let touch = || last_rx.store(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    let writer = async {
        let mut buf = Vec::new();
        while let Some(f) = peer.next_outgoing().await {
            buf.clear();
            if !queue_frame(connection, f, max_datagram, &mut buf) {
                return End::Error;
            }
            // Batch what is already queued into one stream write.
            while let Some(f) = peer.try_next_outgoing() {
                if !queue_frame(connection, f, max_datagram, &mut buf) {
                    return End::Error;
                }
            }
            if !buf.is_empty() && send.write_all(&buf).await.is_err() {
                return End::Error;
            }
        }
        match peer.close_reason() {
            Some(CloseReason::Overflow) => End::Overflow,
            _ => End::ServerClosed,
        }
    };
    let stream_reader = async {
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            loop {
                match decoder.next_frame() {
                    Ok(Some(frame)) => {
                        touch();
                        if peer.push_incoming(frame).await.is_err() {
                            // The sim closed the link; the writer finishes up.
                            std::future::pending::<()>().await;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        debug!(error = %e, "bad stream frame");
                        return End::Protocol;
                    }
                }
            }
            match recv.read(&mut chunk).await {
                Ok(Some(n)) => {
                    touch();
                    decoder.extend(&chunk[..n]);
                }
                // The client finished its stream: it is leaving.
                Ok(None) => return End::PeerClosed,
                Err(_) => return End::PeerClosed,
            }
        }
    };
    let datagram_reader = async {
        loop {
            match connection.receive_datagram().await {
                Ok(d) => {
                    touch();
                    if peer.try_push_datagram(d.payload().to_vec()).is_err() {
                        std::future::pending::<()>().await;
                    }
                }
                Err(_) => return End::PeerClosed,
            }
        }
    };
    let idle = async {
        loop {
            let last = Duration::from_nanos(last_rx.load(Ordering::Relaxed));
            let deadline = start + last + idle_timeout;
            if Instant::now() >= deadline {
                return End::IdleTimeout;
            }
            tokio::time::sleep_until(deadline.into()).await;
        }
    };
    tokio::select! {
        e = writer => e,
        e = stream_reader => e,
        e = datagram_reader => e,
        e = idle => e,
        _ = stop => End::Shutdown,
    }
}

/// Sends an unreliable frame as a datagram when possible, otherwise appends
/// it to the stream buffer. False if the connection is gone.
fn queue_frame(
    connection: &Connection,
    f: OutFrame,
    max_datagram: usize,
    buf: &mut Vec<u8>,
) -> bool {
    if f.channel == Channel::Unreliable && f.data.len() <= max_datagram {
        match connection.send_datagram(&f.data) {
            Ok(()) => return true,
            Err(SendDatagramError::NotConnected) => return false,
            Err(SendDatagramError::TooLarge | SendDatagramError::UnsupportedByPeer) => {}
        }
    }
    append_frame(buf, &f.data);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_across_arbitrary_splits() {
        let packets: Vec<Vec<u8>> = vec![vec![], vec![1], (0..=255).collect(), vec![7; 70_000]];
        let mut wire = Vec::new();
        for p in &packets {
            append_frame(&mut wire, p);
        }
        assert_eq!(&wire[..4], &[0, 0, 0, 0]);
        assert_eq!(&wire[4..9], &[0, 0, 0, 1, 1]);
        for split in [1usize, 3, 4, 5, 1000, wire.len()] {
            let mut d = FrameDecoder::new(1 << 20);
            let mut got = Vec::new();
            for chunk in wire.chunks(split) {
                d.extend(chunk);
                while let Some(f) = d.next_frame().unwrap() {
                    got.push(f);
                }
            }
            assert_eq!(got, packets, "split {split}");
            assert_eq!(d.pending(), 0);
        }
    }

    #[test]
    fn oversized_frames_are_refused() {
        let mut d = FrameDecoder::new(8);
        let mut wire = Vec::new();
        append_frame(&mut wire, &[0; 9]);
        d.extend(&wire[..4]);
        assert_eq!(d.next_frame(), Err(FrameTooLarge { len: 9, max: 8 }));
    }

    #[test]
    fn certificate_is_browser_compatible() {
        let now = OffsetDateTime::now_utc();
        let cert = generate_cert(now).unwrap();
        // Browsers: ECDSA P-256 and at most 14 days of validity.
        let not_before = now - TimeDuration::hours(1);
        let validity = cert.not_after_unix - not_before.unix_timestamp();
        assert!(validity > 0 && validity <= 14 * 24 * 3600, "{validity}");
        assert!(cert.not_after_unix > now.unix_timestamp());
        let der = cert.identity.certificate_chain().as_slice()[0]
            .der()
            .to_vec();
        // id-ecPublicKey (1.2.840.10045.2.1) and prime256v1 (1.2.840.10045.3.1.7).
        let ec_public_key = [0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
        let p256 = [0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
        let has = |needle: &[u8]| der.windows(needle.len()).any(|w| w == needle);
        assert!(has(&ec_public_key) && has(&p256));
        // A second certificate has a different hash.
        assert_ne!(generate_cert(now).unwrap().sha256, cert.sha256);
    }
}
