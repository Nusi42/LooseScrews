//! The directory's HTTP/1.1 endpoint (same small style as `ls-server`'s
//! `http.rs`: hand-written, keep-alive, no chunked bodies or compression).
//!
//! | Request | Body / query | Reply |
//! | --- | --- | --- |
//! | `POST /servers` | [`Heartbeat`] | 200 [`HeartbeatReply`] |
//! | `DELETE /servers` | [`Unregister`] | 204 |
//! | `GET /servers` | `?protocol=5&data_hash=<16 hex>` (both optional filters) | 200 [`ServerList`] |
//! | `POST /invites` | [`InviteRequest`] | 200 [`InviteReply`] |
//! | `GET /invites` | `?player=<id>` | 200 [`InviteList`] |
//! | `OPTIONS *` | CORS preflight | 204 |
//!
//! Errors are JSON [`ErrorReply`]s: 400 invalid, 403 wrong secret, 404,
//! 405, 411/413 body problems, 429 rate limited, 503 full. Every reply
//! carries `Access-Control-Allow-Origin: *` so the browser client can read
//! the list from any origin.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tracing::{debug, info, warn};

use crate::api::{ErrorReply, Heartbeat, InviteList, InviteRequest, ServerList, Unregister};
use crate::registry::{Config, Refused, Registry};

/// Largest request head.
pub const MAX_HEAD: usize = 8 * 1024;
/// Largest request body.
pub const MAX_BODY: usize = 16 * 1024;
const KEEP_ALIVE_IDLE: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How often expired entries are swept (they are also dropped on access).
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

const CORS: &str = "Access-Control-Allow-Origin: *\r\n";
const PREFLIGHT: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nAccess-Control-Max-Age: 600\r\n";

/// A parsed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// Path without the query.
    pub path: String,
    /// Decoded query pairs in order.
    pub query: Vec<(String, String)>,
    pub version: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn query(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn keep_alive(&self) -> bool {
        let has = |t: &str| {
            self.header("connection")
                .is_some_and(|v| v.split(',').any(|x| x.trim().eq_ignore_ascii_case(t)))
        };
        !has("close") && (self.version == "HTTP/1.1" || has("keep-alive"))
    }
}

/// Decodes `%XX` escapes and `+` (query strings); `None` when malformed.
pub fn query_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                out.push(u8::from_str_radix(s.get(i + 1..i + 3)?, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Percent-encodes a query value (everything but unreserved characters).
pub fn query_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Parses a request head (up to and including the blank line).
pub fn parse_request(head: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let version = first.next()?.to_string();
    if first.next().is_some() || !version.starts_with("HTTP/1.") {
        return None;
    }
    let target = target.split('#').next().unwrap_or("");
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    if !(path.starts_with('/') || (method == "OPTIONS" && path == "*")) {
        return None;
    }
    let mut query = Vec::new();
    for pair in q.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        query.push((query_decode(k)?, query_decode(v)?));
    }
    let mut headers = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Some(Request {
        method,
        path: path.to_string(),
        query,
        version,
        headers,
    })
}

/// A reply before it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub extra_headers: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    fn json<T: Serialize>(status: u16, value: &T) -> Reply {
        Reply {
            status,
            content_type: "application/json",
            extra_headers: CORS,
            body: serde_json::to_vec(value).unwrap_or_default(),
        }
    }

    fn error(status: u16, msg: impl Into<String>) -> Reply {
        Reply::json(status, &ErrorReply { error: msg.into() })
    }

    fn empty(status: u16, extra_headers: &'static str) -> Reply {
        Reply {
            status,
            content_type: "text/plain",
            extra_headers,
            body: Vec::new(),
        }
    }

    fn refused(r: Refused) -> Reply {
        let status = match r {
            Refused::Invalid(_) => 400,
            Refused::Forbidden => 403,
            Refused::NotFound => 404,
            Refused::Full => 503,
        };
        Reply::error(status, r.to_string())
    }
}

fn parse_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, Reply> {
    serde_json::from_slice(body).map_err(|e| Reply::error(400, format!("invalid JSON: {e}")))
}

/// Routes one request (rate limiting happens before this).
pub fn route(reg: &Mutex<Registry>, req: &Request, body: &[u8], now: Instant) -> Reply {
    let lock = || reg.lock().unwrap_or_else(|p| p.into_inner());
    if req.method == "OPTIONS" {
        return Reply::empty(204, PREFLIGHT);
    }
    let result = match (req.method.as_str(), req.path.as_str()) {
        ("GET" | "HEAD", "/") => Ok(Reply {
            status: 200,
            content_type: "text/plain; charset=utf-8",
            extra_headers: CORS,
            body: b"ls-directory: GET /servers lists Loose Screws servers\n".to_vec(),
        }),
        ("GET" | "HEAD", "/servers") => {
            let protocol = match req.query("protocol").map(str::parse::<u16>) {
                None => None,
                Some(Ok(p)) => Some(p),
                Some(Err(_)) => return Reply::error(400, "protocol must be a number"),
            };
            let servers = lock().list(protocol, req.query("data_hash"), now);
            Ok(Reply::json(200, &ServerList { servers }))
        }
        ("POST", "/servers") => parse_body::<Heartbeat>(body).map(|hb| {
            lock()
                .heartbeat(hb, now)
                .map_or_else(Reply::refused, |r| Reply::json(200, &r))
        }),
        ("DELETE", "/servers") => parse_body::<Unregister>(body).map(|u| {
            lock()
                .unregister(&u, now)
                .map_or_else(Reply::refused, |()| Reply::empty(204, CORS))
        }),
        ("POST", "/invites") => parse_body::<InviteRequest>(body).map(|inv| {
            lock()
                .invite(inv, now)
                .map_or_else(Reply::refused, |r| Reply::json(200, &r))
        }),
        ("GET" | "HEAD", "/invites") => match req.query("player") {
            Some(p) if !p.is_empty() => {
                let invites = lock().invites_for(p, now);
                Ok(Reply::json(200, &InviteList { invites }))
            }
            _ => Err(Reply::error(400, "missing ?player=")),
        },
        (_, "/" | "/servers" | "/invites") => Err(Reply::error(405, "method not allowed")),
        _ => Err(Reply::error(404, "not found")),
    };
    result.unwrap_or_else(|e| e)
}

/// The directory service on one TCP listener.
#[derive(Debug)]
pub struct DirectoryServer {
    listener: TcpListener,
    registry: Arc<Mutex<Registry>>,
}

impl DirectoryServer {
    /// Binds the listener (tests use `127.0.0.1:0`).
    pub async fn bind(addr: SocketAddr, cfg: Config) -> std::io::Result<Self> {
        Ok(DirectoryServer {
            listener: TcpListener::bind(addr).await?,
            registry: Arc::new(Mutex::new(Registry::new(cfg))),
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// The shared state (for tests and stats).
    pub fn registry(&self) -> Arc<Mutex<Registry>> {
        self.registry.clone()
    }

    /// Serves until `shutdown` turns true.
    pub fn spawn(self, mut shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
        let DirectoryServer { listener, registry } = self;
        tokio::spawn(async move {
            if let Ok(a) = listener.local_addr() {
                info!(addr = %a, "directory listening");
            }
            let mut conns = JoinSet::new();
            let mut sweep = tokio::time::interval(SWEEP_INTERVAL);
            loop {
                tokio::select! {
                    r = listener.accept() => match r {
                        Ok((stream, remote)) => {
                            let _ = stream.set_nodelay(true);
                            conns.spawn(handle_connection(stream, remote, registry.clone()));
                        }
                        Err(e) => {
                            warn!(error = %e, "accept failed");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    },
                    Some(_) = conns.join_next(), if !conns.is_empty() => {}
                    _ = sweep.tick() => {
                        registry.lock().unwrap_or_else(|p| p.into_inner()).expire(Instant::now());
                    }
                    _ = async { let _ = shutdown.wait_for(|s| *s).await; } => break,
                }
            }
            conns.abort_all();
            debug!("directory stopped");
        })
    }
}

async fn handle_connection(mut stream: TcpStream, remote: SocketAddr, reg: Arc<Mutex<Registry>>) {
    let mut buf: Vec<u8> = Vec::new();
    let mut first = true;
    loop {
        let limit = if first {
            REQUEST_TIMEOUT
        } else {
            KEEP_ALIVE_IDLE
        };
        first = false;
        let head_len = match tokio::time::timeout(limit, read_head(&mut stream, &mut buf)).await {
            Ok(Ok(Some(n))) => n,
            _ => return,
        };
        let Some(req) = parse_request(&buf[..head_len]) else {
            let _ = write_reply(&mut stream, &Reply::error(400, "bad request"), false, false).await;
            return;
        };
        buf.drain(..head_len);
        let keep_alive = req.keep_alive();
        if req.header("transfer-encoding").is_some() {
            let r = Reply::error(411, "send a Content-Length body");
            let _ = write_reply(&mut stream, &r, false, false).await;
            return;
        }
        let len = match req.header("content-length").map(str::parse::<usize>) {
            None => 0,
            Some(Ok(n)) if n <= MAX_BODY => n,
            Some(Ok(_)) => {
                let r = Reply::error(413, "body too large");
                let _ = write_reply(&mut stream, &r, false, false).await;
                return;
            }
            Some(Err(_)) => {
                let r = Reply::error(400, "bad content-length");
                let _ = write_reply(&mut stream, &r, false, false).await;
                return;
            }
        };
        match tokio::time::timeout(REQUEST_TIMEOUT, read_exact_into(&mut stream, &mut buf, len))
            .await
        {
            Ok(Ok(true)) => {}
            _ => return,
        }
        let body: Vec<u8> = buf.drain(..len).collect();
        let now = Instant::now();
        let allowed = reg
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .allow(remote.ip(), now);
        let reply = if allowed {
            route(&reg, &req, &body, now)
        } else {
            Reply::error(429, "rate limited")
        };
        debug!(%remote, method = %req.method, path = %req.path, status = reply.status, "request");
        let head_only = req.method == "HEAD";
        if write_reply(&mut stream, &reply, head_only, keep_alive)
            .await
            .is_err()
            || !keep_alive
        {
            return;
        }
    }
}

async fn read_head(stream: &mut TcpStream, buf: &mut Vec<u8>) -> std::io::Result<Option<usize>> {
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            return Ok(Some(i + 4));
        }
        if buf.len() > MAX_HEAD {
            return Ok(None);
        }
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Reads until `buf` holds at least `len` bytes; `false` on EOF.
async fn read_exact_into(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    len: usize,
) -> std::io::Result<bool> {
    while buf.len() < len {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(false);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(true)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

async fn write_reply(
    stream: &mut TcpStream,
    r: &Reply,
    head_only: bool,
    keep_alive: bool,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n{}Connection: {}\r\n\r\n",
        r.status,
        reason(r.status),
        r.content_type,
        r.body.len(),
        r.extra_headers,
        if keep_alive { "keep-alive" } else { "close" },
    );
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(&r.body).await?;
    }
    stream.flush().await?;
    if !keep_alive {
        let _ = stream.shutdown().await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_queries_and_heads() {
        let r = parse_request(
            b"GET /invites?player=abc%2D1+2&x HTTP/1.1\r\nHost: a\r\nContent-Length: 3\r\n\r\n",
        )
        .unwrap();
        assert_eq!(r.path, "/invites");
        assert_eq!(r.query("player"), Some("abc-1 2"));
        assert_eq!(r.query("x"), Some(""));
        let odd = "a b&c=d/\u{fc}";
        assert_eq!(query_decode(&query_encode(odd)).as_deref(), Some(odd));
        assert_eq!(r.header("content-length"), Some("3"));
        assert!(r.keep_alive());
        assert!(parse_request(b"OPTIONS * HTTP/1.1\r\n\r\n").is_some());
        assert!(parse_request(b"GET servers HTTP/1.1\r\n\r\n").is_none());
        assert!(parse_request(b"GET /s?a=%zz HTTP/1.1\r\n\r\n").is_none());
        assert!(!parse_request(b"GET / HTTP/1.0\r\n\r\n")
            .unwrap()
            .keep_alive());
    }

    #[test]
    fn routes_errors_as_json() {
        let reg = Mutex::new(Registry::new(Config::default()));
        let req = |m: &str, p: &str| {
            parse_request(format!("{m} {p} HTTP/1.1\r\n\r\n").as_bytes()).unwrap()
        };
        let now = Instant::now();
        let r = route(&reg, &req("GET", "/nope"), b"", now);
        assert_eq!(r.status, 404);
        assert_eq!(r.body, br#"{"error":"not found"}"#);
        assert_eq!(route(&reg, &req("PUT", "/servers"), b"", now).status, 405);
        assert_eq!(route(&reg, &req("POST", "/servers"), b"{", now).status, 400);
        assert_eq!(route(&reg, &req("GET", "/invites"), b"", now).status, 400);
        assert_eq!(
            route(&reg, &req("GET", "/servers?protocol=x"), b"", now).status,
            400
        );
        let pre = route(&reg, &req("OPTIONS", "/invites"), b"", now);
        assert_eq!(pre.status, 204);
        assert!(pre.extra_headers.contains("Access-Control-Allow-Methods"));
        assert!(route(&reg, &req("GET", "/servers"), b"", now)
            .extra_headers
            .contains("Access-Control-Allow-Origin: *"));
    }
}
