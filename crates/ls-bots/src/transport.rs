//! The two ways a bot talks to the server, behind one [`Link`] type:
//!
//! - **WebSocket** (`tokio-tungstenite`): every binary message carries one
//!   encoded `ls_net::Message` in each direction; unreliable messages travel
//!   reliably too. Text frames are ignored.
//! - **WebTransport** (`wtransport`): the certificate hash and port come from
//!   `GET /cert-hash` on the server's HTTP port (the host and port of the
//!   `--server` URL), and the self-signed certificate is trusted by that hash
//!   only, as browsers do with `serverCertificateHashes`. Reliable messages
//!   are length-prefixed frames (4-byte big-endian length) on one
//!   bidirectional stream the bot opens (the `Hello` first); unreliable ones
//!   are datagrams, or stream frames when larger than the datagram limit.
//!   Datagrams that arrive before the first stream message (the `Welcome`)
//!   are dropped: they cannot be decoded yet.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use ls_game::ls_net::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};
use wtransport::error::SendDatagramError;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint, SendStream, VarInt};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Which transport the bots use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum TransportKind {
    /// WebSocket at the `--server` URL.
    #[default]
    #[value(name = "ws")]
    WebSocket,
    /// WebTransport, with the certificate hash from the server's
    /// `/cert-hash`.
    #[value(name = "wt")]
    WebTransport,
}

impl TransportKind {
    pub fn name(self) -> &'static str {
        match self {
            TransportKind::WebSocket => "websocket",
            TransportKind::WebTransport => "webtransport",
        }
    }
}

/// Length prefix size of a WebTransport stream frame.
const FRAME_HEADER: usize = 4;
/// Largest stream frame accepted from the server (a `Welcome` with a long
/// edit log is the largest message; the server caps queued reliable data at
/// 16 MiB).
const MAX_SERVER_FRAME: usize = 32 << 20;

/// What [`Link::recv`] returns.
#[derive(Debug)]
pub(crate) enum Recv {
    /// One encoded `ls_net::Message`.
    Packet(Vec<u8>),
    /// Something that is not a message (control frame, early datagram).
    Other,
    /// The connection ended.
    Closed(String),
}

/// A connection to the server; see the module docs.
pub(crate) enum Link {
    Ws(Box<Ws>),
    Wt(WtLink),
}

impl Link {
    /// Connects (without the `Hello`). `server` is the WebSocket URL, e.g.
    /// `ws://127.0.0.1:7777/ws`; WebTransport uses its host and port for
    /// `/cert-hash`.
    pub async fn connect(kind: TransportKind, server: &str) -> Result<Link, String> {
        match kind {
            TransportKind::WebSocket => {
                let (ws, _response) = connect_async_with_config(server, None, true)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(Link::Ws(Box::new(ws)))
            }
            TransportKind::WebTransport => Ok(Link::Wt(WtLink::connect(server).await?)),
        }
    }

    /// Queues a message; call [`Link::flush`] to push buffered data out.
    pub async fn send(&mut self, bytes: Vec<u8>, channel: Channel) -> Result<(), String> {
        match self {
            Link::Ws(ws) => ws
                .feed(WsMessage::Binary(bytes.into()))
                .await
                .map_err(|e| e.to_string()),
            Link::Wt(wt) => wt.send(&bytes, channel),
        }
    }

    pub async fn flush(&mut self) -> Result<(), String> {
        match self {
            Link::Ws(ws) => ws.flush().await.map_err(|e| e.to_string()),
            Link::Wt(wt) => wt.flush().await,
        }
    }

    /// The next thing from the server. Cancel safe (used in `select!`).
    pub async fn recv(&mut self) -> Recv {
        match self {
            Link::Ws(ws) => match ws.next().await {
                Some(Ok(WsMessage::Binary(b))) => Recv::Packet(b.to_vec()),
                Some(Ok(WsMessage::Close(frame))) => {
                    let why = frame.map_or_else(String::new, |f| format!(" ({})", f.reason));
                    Recv::Closed(format!("closed by server{why}"))
                }
                // Text frames are not part of the protocol; ping/pong frames
                // are answered by tungstenite.
                Some(Ok(_)) => Recv::Other,
                Some(Err(e)) => Recv::Closed(e.to_string()),
                None => Recv::Closed("connection closed".into()),
            },
            Link::Wt(wt) => wt.recv().await,
        }
    }

    /// Waits for the first message (the answer to `Hello`).
    pub async fn first_packet(&mut self, limit: Duration) -> Result<Vec<u8>, String> {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            match tokio::time::timeout_at(deadline, self.recv()).await {
                Err(_) => return Err("no answer to Hello".into()),
                Ok(Recv::Packet(b)) => return Ok(b),
                Ok(Recv::Other) => {}
                Ok(Recv::Closed(why)) => {
                    return Err(format!("connection closed during handshake: {why}"))
                }
            }
        }
    }

    /// Closes the connection (flushing what was sent before, within reason).
    pub async fn close(&mut self) {
        match self {
            Link::Ws(ws) => {
                let _ = ws.close(None).await;
            }
            Link::Wt(wt) => wt.close().await,
        }
    }
}

enum WtIn {
    Reliable(Vec<u8>),
    Unreliable(Vec<u8>),
    Closed(String),
}

/// A WebTransport session: reader tasks merge the stream and datagrams into
/// one channel, so [`WtLink::recv`] is cancel safe.
pub(crate) struct WtLink {
    conn: Connection,
    send: SendStream,
    rx: mpsc::UnboundedReceiver<WtIn>,
    readers: [JoinHandle<()>; 2],
    /// Stream frames waiting for [`WtLink::flush`].
    out: Vec<u8>,
    /// The first stream message arrived; datagrams are usable from now on.
    welcomed: bool,
}

impl Drop for WtLink {
    fn drop(&mut self) {
        for r in &self.readers {
            r.abort();
        }
    }
}

/// Certificate info from `/cert-hash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CertHash {
    pub sha256: [u8; 32],
    pub port: u16,
    pub path: String,
}

impl WtLink {
    async fn connect(server: &str) -> Result<WtLink, String> {
        let authority = authority_of(server).ok_or_else(|| format!("bad server URL {server}"))?;
        let cert = fetch_cert_hash(authority).await?;
        let host = host_of(authority);
        let url = format!("https://{host}:{}{}", cert.port, cert.path);
        let builder = ClientConfig::builder();
        // A loopback server gets a loopback socket: a wildcard UDP bind makes
        // Windows ask to allow every new (hashed) test binary in the firewall.
        let builder = match loopback_bind(host) {
            Some(addr) => builder.with_bind_address(addr),
            None => builder.with_bind_default(),
        };
        let cfg = builder
            .with_server_certificate_hashes([Sha256Digest::new(cert.sha256)])
            .keep_alive_interval(Some(Duration::from_secs(4)))
            .build();
        let endpoint = Endpoint::client(cfg).map_err(|e| format!("udp socket: {e}"))?;
        let conn = endpoint
            .connect(&url)
            .await
            .map_err(|e| format!("webtransport {url}: {e}"))?;
        let (send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::unbounded_channel();
        let stream_tx = tx.clone();
        let stream_reader = tokio::spawn(async move {
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                match recv.read(&mut chunk).await {
                    Ok(Some(n)) => buf.extend_from_slice(&chunk[..n]),
                    Ok(None) => {
                        let _ = stream_tx.send(WtIn::Closed("stream finished by server".into()));
                        return;
                    }
                    Err(e) => {
                        let _ = stream_tx.send(WtIn::Closed(e.to_string()));
                        return;
                    }
                }
                let mut start = 0;
                while let Some(header) = buf.get(start..start + FRAME_HEADER) {
                    let len =
                        u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                    if len > MAX_SERVER_FRAME {
                        let _ =
                            stream_tx.send(WtIn::Closed(format!("stream frame of {len} bytes")));
                        return;
                    }
                    let Some(frame) = buf.get(start + FRAME_HEADER..start + FRAME_HEADER + len)
                    else {
                        break;
                    };
                    let _ = stream_tx.send(WtIn::Reliable(frame.to_vec()));
                    start += FRAME_HEADER + len;
                }
                buf.drain(..start);
            }
        });
        let dgram_conn = conn.clone();
        let datagram_reader = tokio::spawn(async move {
            loop {
                match dgram_conn.receive_datagram().await {
                    Ok(d) => {
                        let _ = tx.send(WtIn::Unreliable(d.payload().to_vec()));
                    }
                    // Connection loss is reported by the stream reader, after
                    // it queued every frame it received (the final Disconnect
                    // must not lose the race against this reader).
                    Err(_) => return,
                }
            }
        });
        Ok(WtLink {
            conn,
            send,
            rx,
            readers: [stream_reader, datagram_reader],
            out: Vec::new(),
            welcomed: false,
        })
    }

    fn send(&mut self, bytes: &[u8], channel: Channel) -> Result<(), String> {
        let max = self.conn.max_datagram_size().unwrap_or(0);
        if channel == Channel::Unreliable && bytes.len() <= max {
            match self.conn.send_datagram(bytes) {
                Ok(()) => return Ok(()),
                Err(SendDatagramError::NotConnected) => return Err("not connected".into()),
                Err(SendDatagramError::TooLarge | SendDatagramError::UnsupportedByPeer) => {}
            }
        }
        let len = u32::try_from(bytes.len()).map_err(|_| "message too large".to_string())?;
        self.out.extend_from_slice(&len.to_be_bytes());
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), String> {
        if self.out.is_empty() {
            return Ok(());
        }
        let r = self.send.write_all(&self.out).await;
        self.out.clear();
        r.map_err(|e| e.to_string())
    }

    async fn recv(&mut self) -> Recv {
        match self.rx.recv().await {
            Some(WtIn::Reliable(b)) => {
                self.welcomed = true;
                Recv::Packet(b)
            }
            Some(WtIn::Unreliable(b)) if self.welcomed => Recv::Packet(b),
            Some(WtIn::Unreliable(_)) => Recv::Other,
            Some(WtIn::Closed(why)) => Recv::Closed(why),
            None => Recv::Closed("connection closed".into()),
        }
    }

    async fn close(&mut self) {
        let _ = self.flush().await;
        // `finish` completes once the server acknowledged everything sent.
        let _ = self.send.finish().await;
        self.conn.close(VarInt::from_u32(0), b"bot leaving");
    }
}

/// `host:port` of a `ws://`, `wss://`, `http://` or `https://` URL (or of a
/// bare `host:port`).
pub(crate) fn authority_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then_some(authority)
}

/// Host part of an authority (`[::1]:7777` -> `[::1]`, `a:1` -> `a`).
pub(crate) fn host_of(authority: &str) -> &str {
    if authority.starts_with('[') {
        return authority.find(']').map_or(authority, |i| &authority[..=i]);
    }
    authority.rsplit_once(':').map_or(authority, |(h, _)| h)
}

/// The local UDP address for a WebTransport client of `host` (from
/// [`host_of`]): port 0 on the matching loopback address when `host` is a
/// loopback IP, else `None` (bind all addresses).
pub(crate) fn loopback_bind(host: &str) -> Option<std::net::SocketAddr> {
    let ip: std::net::IpAddr = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()?;
    ip.is_loopback().then(|| std::net::SocketAddr::new(ip, 0))
}

/// `GET /cert-hash` over plain HTTP/1.1.
pub(crate) async fn fetch_cert_hash(authority: &str) -> Result<CertHash, String> {
    let fetch = async {
        let mut s = TcpStream::connect(authority)
            .await
            .map_err(|e| format!("cannot reach {authority}: {e}"))?;
        let req =
            format!("GET /cert-hash HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
        Ok::<_, String>(buf)
    };
    let buf = tokio::time::timeout(Duration::from_secs(10), fetch)
        .await
        .map_err(|_| "GET /cert-hash timed out".to_string())??;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or("malformed /cert-hash response")?;
    let status = head.split(' ').nth(1).unwrap_or("");
    if status != "200" {
        return Err(format!(
            "server has no WebTransport (GET /cert-hash: {status})"
        ));
    }
    parse_cert_hash(body).ok_or_else(|| format!("bad /cert-hash body: {body}"))
}

/// Parses `{"algorithm":"sha-256","hash":"<hex>","port":N,"path":"/wt",...}`.
pub(crate) fn parse_cert_hash(json: &str) -> Option<CertHash> {
    if json_field(json, "algorithm")? != "sha-256" {
        return None;
    }
    let hex = json_field(json, "hash")?;
    if hex.len() != 64 {
        return None;
    }
    let mut sha256 = [0u8; 32];
    for (i, b) in sha256.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    let port = json_field(json, "port")?.parse().ok()?;
    let path = json_field(json, "path").unwrap_or("/wt");
    if !path.starts_with('/') {
        return None;
    }
    Some(CertHash {
        sha256,
        port,
        path: path.to_string(),
    })
}

/// Value of `"key":` in a flat JSON object without escapes (strings are
/// returned without quotes).
fn json_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let at = json.find(&pat)? + pat.len();
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(s) = rest.strip_prefix('"') {
        return s.split('"').next();
    }
    rest.split([',', '}']).next().map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            authority_of("ws://127.0.0.1:7777/ws"),
            Some("127.0.0.1:7777")
        );
        assert_eq!(authority_of("http://host:1"), Some("host:1"));
        assert_eq!(authority_of("[::1]:7777"), Some("[::1]:7777"));
        assert_eq!(authority_of("ws:///ws"), None);
        assert_eq!(host_of("127.0.0.1:7777"), "127.0.0.1");
        assert_eq!(
            loopback_bind("127.0.0.1"),
            Some("127.0.0.1:0".parse().unwrap())
        );
        assert_eq!(loopback_bind("[::1]"), Some("[::1]:0".parse().unwrap()));
        assert_eq!(loopback_bind("example.org"), None);
        assert_eq!(loopback_bind("10.0.0.2"), None);
        assert_eq!(host_of("[::1]:7777"), "[::1]");
        assert_eq!(host_of("localhost"), "localhost");
    }

    #[test]
    fn cert_hash_json() {
        let hex = "ab".repeat(31) + "01";
        let json = format!(
            r#"{{"algorithm":"sha-256","hash":"{hex}","port":7777,"path":"/wt","expires":1700000000}}"#
        );
        let c = parse_cert_hash(&json).unwrap();
        assert_eq!(c.port, 7777);
        assert_eq!(c.path, "/wt");
        assert_eq!(c.sha256[0], 0xab);
        assert_eq!(c.sha256[31], 0x01);
        // Whitespace is fine; wrong algorithms, short hashes and missing ports are not.
        let spaced = json.replace(':', ": ").replace(',', ", ");
        assert_eq!(parse_cert_hash(&spaced), Some(c));
        assert!(parse_cert_hash(&json.replace("sha-256", "md5")).is_none());
        assert!(parse_cert_hash(&json.replace(&hex, "abcd")).is_none());
        assert!(parse_cert_hash(r#"{"algorithm":"sha-256"}"#).is_none());
    }
}
