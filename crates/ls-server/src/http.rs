//! HTTP/1.1 on the game port: static files from the web root (the built web
//! client, for one-command local play), `GET /cert-hash` for WebTransport
//! (JSON, see [`crate::endpoint::CertHashInfo`]),
//! and the WebSocket upgrade at `/ws` (also accepted at `/`, so
//! `ws://host:7777` works too).
//!
//! Deliberately small: GET/HEAD only, keep-alive, no ranges or compression.

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

use crate::endpoint::{CertHash, Endpoint, EndpointContext, Hub};
use crate::ws;

/// Largest request head accepted.
const MAX_HEAD: usize = 16 * 1024;
/// Keep-alive connections idle this long are closed.
const KEEP_ALIVE_IDLE: Duration = Duration::from_secs(15);
/// Time allowed to send a complete request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// A parsed request head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// Path without query or fragment, still percent-encoded.
    pub path: String,
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

    fn header_has_token(&self, name: &str, token: &str) -> bool {
        self.header(name)
            .is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
    }

    /// True for a WebSocket upgrade request.
    pub fn is_websocket_upgrade(&self) -> bool {
        self.header_has_token("upgrade", "websocket")
    }

    fn keep_alive(&self) -> bool {
        if self.header_has_token("connection", "close") {
            return false;
        }
        self.version == "HTTP/1.1" || self.header_has_token("connection", "keep-alive")
    }
}

/// Parses a request head (everything up to and including the blank line).
pub fn parse_request(head: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let version = first.next()?.to_string();
    if first.next().is_some() || !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return None;
    }
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Some(Request {
        method,
        path,
        version,
        headers,
    })
}

/// Decodes `%XX` escapes; `None` for malformed escapes or non-UTF-8 results.
pub fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Maps a URL path to a relative file path inside the web root, refusing
/// anything that could escape it (`..`, absolute or drive-prefixed parts,
/// backslashes, NUL). `/` and directories map to `index.html` later.
pub fn safe_relative_path(url_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(url_path)?;
    let mut rel = PathBuf::new();
    for seg in decoded.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains(['\\', ':', '\0']) {
            return None;
        }
        let p = Path::new(seg);
        if p.components().count() != 1
            || !matches!(p.components().next(), Some(Component::Normal(_)))
        {
            return None;
        }
        rel.push(seg);
    }
    Some(rel)
}

/// Content type by file extension.
pub fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "wgsl" => "text/plain; charset=utf-8",
        "ron" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

/// HTTP + WebSocket endpoint on one TCP port.
#[derive(Debug)]
pub struct HttpEndpoint {
    listener: TcpListener,
    web_root: Option<PathBuf>,
}

impl HttpEndpoint {
    /// Binds the TCP listener. `web_root` is served statically if given.
    pub async fn bind(addr: SocketAddr, web_root: Option<PathBuf>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(HttpEndpoint { listener, web_root })
    }
}

#[derive(Debug)]
struct Shared {
    hub: Hub,
    web_root: Option<PathBuf>,
    cert_hash: CertHash,
    shutdown: watch::Receiver<bool>,
}

impl Endpoint for HttpEndpoint {
    fn name(&self) -> &'static str {
        ws::TRANSPORT
    }

    fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    fn spawn(self: Box<Self>, ctx: EndpointContext) -> JoinHandle<()> {
        let HttpEndpoint { listener, web_root } = *self;
        let web_root = web_root.and_then(|r| match r.canonicalize() {
            Ok(c) if c.is_dir() => Some(c),
            _ => {
                warn!(web_root = %r.display(), "web root not found; serving a placeholder page");
                None
            }
        });
        let mut shutdown = ctx.shutdown.clone();
        let grace = ctx.hub.settings().shutdown_grace;
        let shared = Arc::new(Shared {
            hub: ctx.hub,
            web_root,
            cert_hash: ctx.cert_hash,
            shutdown: ctx.shutdown,
        });
        tokio::spawn(async move {
            let mut conns = JoinSet::new();
            loop {
                tokio::select! {
                    r = listener.accept() => match r {
                        Ok((stream, remote)) => {
                            let _ = stream.set_nodelay(true);
                            conns.spawn(handle_connection(stream, remote, shared.clone()));
                        }
                        Err(e) => {
                            warn!(error = %e, "accept failed");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    },
                    Some(_) = conns.join_next(), if !conns.is_empty() => {}
                    _ = async { let _ = shutdown.wait_for(|s| *s).await; } => break,
                }
            }
            drop(listener);
            // Connections close themselves after the grace period.
            let drained = tokio::time::timeout(grace * 2, async {
                while conns.join_next().await.is_some() {}
            })
            .await;
            if drained.is_err() {
                conns.abort_all();
            }
            debug!("http endpoint stopped");
        })
    }
}

/// Serves requests on one TCP connection until it closes, upgrading to
/// WebSocket when asked.
async fn handle_connection(mut stream: TcpStream, remote: SocketAddr, shared: Arc<Shared>) {
    let mut buf: Vec<u8> = Vec::new();
    let mut first = true;
    loop {
        let limit = if first { HEAD_TIMEOUT } else { KEEP_ALIVE_IDLE };
        first = false;
        let head_len = match tokio::time::timeout(limit, read_head(&mut stream, &mut buf)).await {
            Ok(Ok(Some(n))) => n,
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => return,
        };
        let Some(req) = parse_request(&buf[..head_len]) else {
            let _ = respond(&mut stream, 400, "text/plain", b"bad request", false, true).await;
            return;
        };
        let rest = buf.split_off(head_len);
        buf = rest;

        if req.is_websocket_upgrade() {
            if req.path == "/ws" || req.path == "/" {
                upgrade(stream, remote, req, buf, shared).await;
            } else {
                let _ = respond(
                    &mut stream,
                    404,
                    "text/plain",
                    b"websocket is at /ws",
                    false,
                    true,
                )
                .await;
            }
            return;
        }
        debug!(%remote, method = %req.method, path = %req.path, "http request");
        let keep_alive = req.keep_alive() && !*shared.shutdown.borrow();
        if handle_request(&mut stream, &req, &shared, keep_alive)
            .await
            .is_err()
            || !keep_alive
        {
            return;
        }
    }
}

/// Reads until a complete request head is in `buf`; returns its length
/// (`None` on EOF or an oversized head).
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

async fn upgrade(
    mut stream: TcpStream,
    remote: SocketAddr,
    req: Request,
    leftover: Vec<u8>,
    shared: Arc<Shared>,
) {
    let key = req.header("sec-websocket-key");
    let valid = req.method == "GET"
        && req.header_has_token("connection", "upgrade")
        && req.header("sec-websocket-version") == Some("13");
    let (true, Some(key)) = (valid, key) else {
        let _ = respond(
            &mut stream,
            400,
            "text/plain",
            b"invalid websocket upgrade",
            false,
            true,
        )
        .await;
        return;
    };
    let accept = derive_accept_key(key.as_bytes());
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if stream.write_all(response.as_bytes()).await.is_err() {
        return;
    }
    let ws = WebSocketStream::from_partially_read(
        stream,
        leftover,
        Role::Server,
        Some(ws::server_config()),
    )
    .await;
    ws::serve(ws, remote, shared.hub.clone(), shared.shutdown.clone()).await;
}

async fn handle_request(
    stream: &mut TcpStream,
    req: &Request,
    shared: &Shared,
    keep_alive: bool,
) -> std::io::Result<()> {
    let head = req.method == "HEAD";
    if req.method != "GET" && !head {
        return respond(
            stream,
            405,
            "text/plain",
            b"method not allowed",
            head,
            keep_alive,
        )
        .await;
    }
    if req.path == "/cert-hash" {
        let hash = shared
            .cert_hash
            .read()
            .map(|h| h.clone())
            .unwrap_or_default();
        // Readable cross-origin, so a client served from elsewhere (the Vite
        // dev server with `?server=host:port`) can use WebTransport too.
        let cors = "Access-Control-Allow-Origin: *\r\n";
        return match hash {
            Some(h) => {
                let body = h.to_json();
                respond_with(
                    stream,
                    200,
                    "application/json",
                    cors,
                    body.as_bytes(),
                    head,
                    keep_alive,
                )
                .await
            }
            None => {
                respond_with(
                    stream,
                    404,
                    "text/plain",
                    cors,
                    b"WebTransport is not enabled",
                    head,
                    keep_alive,
                )
                .await
            }
        };
    }
    let Some(root) = &shared.web_root else {
        let body = b"<!doctype html><title>Loose Screws server</title><p>ls-server is running. \
The game socket is at <code>/ws</code>. Build the web client (<code>cd web &amp;&amp; npm run build</code>) \
and start the server with <code>--web-root web/dist</code> to play here.</p>";
        let status = if req.path == "/" { 200 } else { 404 };
        return respond(
            stream,
            status,
            "text/html; charset=utf-8",
            body,
            head,
            keep_alive,
        )
        .await;
    };
    let Some(rel) = safe_relative_path(&req.path) else {
        return respond(stream, 403, "text/plain", b"forbidden", head, keep_alive).await;
    };
    match resolve_file(root, &rel).await {
        Some(path) => match tokio::fs::read(&path).await {
            Ok(body) => respond(stream, 200, content_type(&path), &body, head, keep_alive).await,
            Err(_) => respond(stream, 404, "text/plain", b"not found", head, keep_alive).await,
        },
        None => respond(stream, 404, "text/plain", b"not found", head, keep_alive).await,
    }
}

/// Finds the file for a request: the file itself, `index.html` of a
/// directory, or (for extension-less paths, client-side routes) the root
/// `index.html`. The result must stay inside `root` after resolving links.
async fn resolve_file(root: &Path, rel: &Path) -> Option<PathBuf> {
    let mut candidates = vec![root.join(rel), root.join(rel).join("index.html")];
    if rel.extension().is_none() {
        candidates.push(root.join("index.html"));
    }
    for c in candidates {
        let Ok(canon) = tokio::fs::canonicalize(&c).await else {
            continue;
        };
        if !canon.starts_with(root) {
            continue;
        }
        if tokio::fs::metadata(&canon).await.is_ok_and(|m| m.is_file()) {
            return Some(canon);
        }
    }
    None
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    }
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    keep_alive: bool,
) -> std::io::Result<()> {
    respond_with(
        stream,
        status,
        content_type,
        "",
        body,
        head_only,
        keep_alive,
    )
    .await
}

/// [`respond`] with extra header lines (each ending in `\r\n`).
async fn respond_with(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    extra_headers: &str,
    body: &[u8],
    head_only: bool,
    keep_alive: bool,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\n{extra_headers}Connection: {}\r\n\r\n",
        reason_phrase(status),
        body.len(),
        if keep_alive { "keep-alive" } else { "close" },
    );
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(body).await?;
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
    fn parses_request_heads() {
        let r = parse_request(
            b"GET /ws?x=1 HTTP/1.1\r\nHost: a\r\nUpgrade: WebSocket\r\nConnection: keep-alive, Upgrade\r\n\r\n",
        )
        .unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/ws");
        assert_eq!(r.header("host"), Some("a"));
        assert!(r.is_websocket_upgrade());
        assert!(r.header_has_token("connection", "upgrade"));
        assert!(r.keep_alive());
        let r = parse_request(b"GET / HTTP/1.0\r\n\r\n").unwrap();
        assert!(!r.keep_alive() && !r.is_websocket_upgrade());
        assert!(parse_request(b"GET index HTTP/1.1\r\n\r\n").is_none());
        assert!(parse_request(b"GARBAGE\r\n\r\n").is_none());
        assert!(parse_request(b"GET / HTTP/1.1\r\nno-colon\r\n\r\n").is_none());
    }

    #[test]
    fn paths_cannot_escape_the_web_root() {
        assert_eq!(safe_relative_path("/").unwrap(), PathBuf::new());
        assert_eq!(
            safe_relative_path("/assets/app%20x.js").unwrap(),
            Path::new("assets").join("app x.js")
        );
        assert_eq!(
            safe_relative_path("//a/./b").unwrap(),
            Path::new("a").join("b")
        );
        for bad in [
            "/../secret",
            "/a/../../b",
            "/%2e%2e/x",
            "/a%5c..%5cb",
            "/C:/Windows",
            "/a%00b",
            "/%zz",
        ] {
            assert!(safe_relative_path(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type(Path::new("a.wasm")), "application/wasm");
        assert!(content_type(Path::new("index.HTML")).starts_with("text/html"));
        assert!(content_type(Path::new("x.js")).starts_with("text/javascript"));
        assert_eq!(content_type(Path::new("x")), "application/octet-stream");
    }
}
