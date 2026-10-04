//! A small async client for the directory API, used by `ls-server`
//! (`--directory <url>`) and by tests. Plain `http://` only: put a TLS
//! reverse proxy in front of the directory for browsers on `https://` pages,
//! and let game servers use the plain URL (or the proxy's HTTP side).
//!
//! ```no_run
//! # async fn demo(info: ls_directory::api::ServerInfo) -> Result<(), ls_directory::client::ClientError> {
//! use ls_directory::client::{spawn_heartbeats, DirectoryClient};
//! let client = DirectoryClient::new("http://127.0.0.1:7780")?;
//! let (_tx, shutdown) = tokio::sync::watch::channel(false);
//! spawn_heartbeats(client, ls_directory::new_secret(), move || info.clone(), shutdown);
//! # Ok(()) }
//! ```

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::api::{
    ErrorReply, Heartbeat, HeartbeatReply, InviteList, InviteReply, InviteRequest, ServerInfo,
    ServerList, Unregister,
};
use crate::http::query_encode;

/// Time allowed for one request (connect, send, receive).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest reply accepted.
const MAX_REPLY: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("directory URL must be http://host[:port][/path]: {0}")]
    Url(String),
    #[error("directory request failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("directory request timed out")]
    Timeout,
    #[error("directory sent a malformed reply")]
    Malformed,
    #[error("directory replied {status}: {message}")]
    Status { status: u16, message: String },
    #[error("directory reply is not the expected JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Where the directory is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryClient {
    host: String,
    port: u16,
    /// Path prefix without a trailing slash (`""` or `/dir`).
    base: String,
}

impl DirectoryClient {
    /// Parses `http://host[:port][/base]` (port 80 by default).
    pub fn new(url: &str) -> Result<Self, ClientError> {
        let bad = || ClientError::Url(url.to_string());
        let rest = url.strip_prefix("http://").ok_or_else(bad)?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() || authority.contains('@') {
            return Err(bad());
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or_else(bad)?;
            let port = match after.strip_prefix(':') {
                Some(p) => p.parse().map_err(|_| bad())?,
                None if after.is_empty() => 80,
                None => return Err(bad()),
            };
            (h.to_string(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| bad())?),
                None => (authority.to_string(), 80),
            }
        };
        if host.is_empty() {
            return Err(bad());
        }
        let base = path.trim_end_matches('/');
        let base = if base.is_empty() {
            String::new()
        } else {
            format!("/{base}")
        };
        Ok(DirectoryClient { host, port, base })
    }

    /// Registers or refreshes a server entry.
    pub async fn heartbeat(&self, hb: &Heartbeat) -> Result<HeartbeatReply, ClientError> {
        self.json("POST", "/servers", Some(hb)).await
    }

    /// Removes the entry (server shutdown).
    pub async fn unregister(&self, u: &Unregister) -> Result<(), ClientError> {
        self.request("DELETE", "/servers", Some(serde_json::to_vec(u)?))
            .await
            .map(|_| ())
    }

    /// Lists servers, optionally only those matching a protocol version and
    /// data hash (16 hex digits, see [`crate::api::data_hash_hex`]).
    pub async fn list(
        &self,
        protocol: Option<u16>,
        data_hash: Option<&str>,
    ) -> Result<ServerList, ClientError> {
        let mut q = Vec::new();
        if let Some(p) = protocol {
            q.push(format!("protocol={p}"));
        }
        if let Some(h) = data_hash {
            q.push(format!("data_hash={}", query_encode(h)));
        }
        let path = if q.is_empty() {
            "/servers".to_string()
        } else {
            format!("/servers?{}", q.join("&"))
        };
        self.json::<(), _>("GET", &path, None).await
    }

    /// Relays an invite to a player id.
    pub async fn invite(&self, inv: &InviteRequest) -> Result<InviteReply, ClientError> {
        self.json("POST", "/invites", Some(inv)).await
    }

    /// Pending invites for a player id.
    pub async fn invites(&self, player: &str) -> Result<InviteList, ClientError> {
        let path = format!("/invites?player={}", query_encode(player));
        self.json::<(), _>("GET", &path, None).await
    }

    async fn json<B: Serialize, T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
    ) -> Result<T, ClientError> {
        let body = body.map(serde_json::to_vec).transpose()?;
        let reply = self.request(method, path, body).await?;
        Ok(serde_json::from_slice(&reply)?)
    }

    /// One request on a fresh connection (`Connection: close`); returns the
    /// body of a 2xx reply.
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, ClientError> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.request_inner(method, path, body))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn request_inner(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, ClientError> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port)).await?;
        let _ = stream.set_nodelay(true);
        let host = if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        };
        let body = body.unwrap_or_default();
        let content = if body.is_empty() && method == "GET" {
            String::new()
        } else {
            format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            )
        };
        let head = format!(
            "{method} {}{path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: ls-directory-client\r\nAccept: application/json\r\n{content}Connection: close\r\n\r\n",
            self.base
        );
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&body).await?;
        stream.flush().await?;
        let mut raw = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..n]);
            if raw.len() > MAX_REPLY {
                return Err(ClientError::Malformed);
            }
        }
        let (status, reply) = parse_reply(&raw).ok_or(ClientError::Malformed)?;
        if (200..300).contains(&status) {
            Ok(reply)
        } else {
            let message = serde_json::from_slice::<ErrorReply>(&reply)
                .map(|e| e.error)
                .unwrap_or_else(|_| String::from_utf8_lossy(&reply).into_owned());
            Err(ClientError::Status { status, message })
        }
    }
}

/// Splits a complete HTTP/1.1 reply (read to EOF) into status and body.
fn parse_reply(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let mut body = raw[end + 4..].to_vec();
    for line in lines {
        if let Some((n, v)) = line.split_once(':') {
            if n.trim().eq_ignore_ascii_case("content-length") {
                let len: usize = v.trim().parse().ok()?;
                if body.len() < len {
                    return None;
                }
                body.truncate(len);
            }
        }
    }
    Some((status, body))
}

/// Sends a heartbeat now and then every interval the directory asks for
/// (10 s by default) with the server's current [`ServerInfo`] from `info`,
/// until `shutdown` turns true; then removes the entry. Failures are logged
/// and retried at the next interval: the game server never depends on the
/// directory.
pub fn spawn_heartbeats<F>(
    client: DirectoryClient,
    secret: String,
    mut info: F,
    mut shutdown: watch::Receiver<bool>,
) -> JoinHandle<()>
where
    F: FnMut() -> ServerInfo + Send + 'static,
{
    tokio::spawn(async move {
        let mut interval = Duration::from_secs(10);
        let mut registered = false;
        let mut address = String::new();
        loop {
            let server = info();
            address.clone_from(&server.address);
            let hb = Heartbeat {
                secret: secret.clone(),
                server,
            };
            match client.heartbeat(&hb).await {
                Ok(r) => {
                    if !registered {
                        info!(id = %r.id, "registered with the directory");
                    }
                    registered = true;
                    interval = Duration::from_secs(r.heartbeat_secs.clamp(1, 300));
                }
                Err(e) => {
                    warn!(error = %e, "directory heartbeat failed");
                    registered = false;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = shutdown.wait_for(|s| *s) => break,
            }
        }
        if registered {
            let u = Unregister { address, secret };
            match client.unregister(&u).await {
                Ok(()) => debug!("left the directory"),
                Err(e) => debug!(error = %e, "directory unregister failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let c = DirectoryClient::new("http://dir.example.com").unwrap();
        assert_eq!(
            (c.host.as_str(), c.port, c.base.as_str()),
            ("dir.example.com", 80, "")
        );
        let c = DirectoryClient::new("http://127.0.0.1:7780/ls/").unwrap();
        assert_eq!(
            (c.host.as_str(), c.port, c.base.as_str()),
            ("127.0.0.1", 7780, "/ls")
        );
        let c = DirectoryClient::new("http://[::1]:9/").unwrap();
        assert_eq!((c.host.as_str(), c.port), ("::1", 9));
        for bad in [
            "https://x",
            "x:1",
            "http://",
            "http://:80",
            "http://a:b",
            "http://u@h",
            "http://[::1]x",
        ] {
            assert!(DirectoryClient::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_replies() {
        let (s, b) = parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}junk").unwrap();
        assert_eq!((s, b.as_slice()), (200, b"{}".as_slice()));
        let (s, b) = parse_reply(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
        assert_eq!((s, b.len()), (204, 0));
        assert!(parse_reply(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n{}").is_none());
        assert!(parse_reply(b"nonsense").is_none());
    }
}
