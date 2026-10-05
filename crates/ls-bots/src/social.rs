//! Server list and duels for bots (M3 package C2):
//!
//! - [`pick_server`]: `--directory <url>` asks an `ls-directory` for the
//!   servers of this build (protocol and data hash) and picks one to join:
//!   the fullest server that still has room, ties by name.
//! - [`spawn_duel_watcher`]: with `--accept-duels` a bot registers a
//!   presence on its game server (`POST /api/presence`, like a browser
//!   tab, so it shows up in the roster and can be challenged), polls its
//!   inbox (and the directory's relayed invites by its public id) and
//!   accepts every duel challenge (`POST /api/duel/answer` on the
//!   challenger's server). The duel room arrives as a [`DuelMove`]; the
//!   runner then reconnects into it (`Hello::room`) and, once that match is
//!   over, goes back to its home server.
//!
//! Requests are plain HTTP/1.1 on a fresh connection, like the browser's
//! `fetch` against the game port.

use std::time::Duration;

use ls_directory::api::{data_hash_hex, ServerEntry};
use ls_directory::DirectoryClient;
use ls_game::ls_math::rng::mix64;
use ls_game::ls_net::PROTOCOL_VERSION;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// How often the duel watcher polls the inbox (the browser polls every 3 s).
pub const PRESENCE_POLL: Duration = Duration::from_secs(2);
/// Time allowed for one HTTP request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest reply read.
const MAX_REPLY: usize = 1 << 20;

/// A bot's identity for invites: the public player id challenges are
/// addressed to and the private token that proves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub token: String,
    pub name: String,
}

impl Identity {
    /// A stable identity for bot `name` with behaviour seed `seed`
    /// (ids: 8-64 of `A-Za-z0-9-_.:[]`).
    pub fn for_bot(name: &str, seed: u64) -> Identity {
        let clean: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c))
            .take(32)
            .collect();
        let h = mix64(seed ^ 0x424f_545f_4944_454e);
        Identity {
            id: format!("bot-{clean}-{:08x}", h as u32),
            token: format!("bot-token-{:016x}{:016x}", mix64(h), mix64(h ^ 1)),
            name: name.to_string(),
        }
    }
}

/// A duel the bot accepted: where to play it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuelMove {
    /// `host:port` of the challenger's server.
    pub address: String,
    /// The duel's private room code.
    pub room: String,
    /// The challenger's name (logs).
    pub from: String,
}

/// `host:port` of a `ws://host:port/...` (or `wss://`, `http://`) URL.
pub fn host_port(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split('/').next()?;
    (!authority.is_empty()).then(|| authority.to_string())
}

/// The game server's WebSocket URL for a `host:port` address.
pub fn ws_url(address: &str) -> String {
    format!("ws://{address}/ws")
}

/// The server to join among the directory's entries: those with room left,
/// the fullest first (bots fill servers rather than spreading out), then by
/// name and address.
pub fn choose_server(entries: &[ServerEntry]) -> Option<&ServerEntry> {
    entries
        .iter()
        .filter(|e| e.server.players < e.server.max_players)
        .min_by(|a, b| {
            b.server
                .players
                .cmp(&a.server.players)
                .then_with(|| a.server.name.cmp(&b.server.name))
                .then_with(|| a.server.address.cmp(&b.server.address))
        })
}

/// Asks the directory at `url` for this build's servers and picks one
/// ([`choose_server`]). Returns its WebSocket URL.
pub async fn pick_server(url: &str) -> Result<String, String> {
    let client = DirectoryClient::new(url).map_err(|e| e.to_string())?;
    let hash = data_hash_hex(ls_game::ls_sim::data_hash());
    let list = client
        .list(Some(PROTOCOL_VERSION), Some(&hash))
        .await
        .map_err(|e| format!("directory {url}: {e}"))?;
    choose_server(&list.servers)
        .map(|e| ws_url(&e.server.address))
        .ok_or_else(|| format!("directory {url} lists no server with room for this build"))
}

/// One JSON request to `http://{address}{path}`; the reply's JSON on a 2xx
/// status.
pub async fn http_json(
    address: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    tokio::time::timeout(REQUEST_TIMEOUT, http_inner(address, method, path, body))
        .await
        .map_err(|_| format!("{method} {path} on {address}: timed out"))?
}

async fn http_inner(
    address: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let err = |e: std::io::Error| format!("{method} {path} on {address}: {e}");
    let mut s = TcpStream::connect(address).await.map_err(err)?;
    let body = body.map(Value::to_string).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nUser-Agent: ls-bots\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await.map_err(err)?;
    s.write_all(body.as_bytes()).await.map_err(err)?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = s.read(&mut chunk).await.map_err(err)?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..n]);
        if raw.len() > MAX_REPLY {
            return Err(format!("{method} {path} on {address}: reply too large"));
        }
    }
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("{method} {path} on {address}: malformed reply"))?;
    let status: u16 = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("{method} {path} on {address}: malformed status"))?;
    let value: Value = if body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).map_err(|e| format!("{method} {path}: {e}"))?
    };
    if (200..300).contains(&status) {
        Ok(value)
    } else {
        Err(format!("{method} {path} on {address}: {status} {value}"))
    }
}

/// Registers the presence of `me` on the server at `address` and returns
/// the notes in its inbox.
pub async fn presence(address: &str, me: &Identity) -> Result<Vec<Value>, String> {
    let body = json!({"id": me.id, "token": me.token, "name": me.name});
    let reply = http_json(address, "POST", "/api/presence", Some(&body)).await?;
    Ok(reply["invites"].as_array().cloned().unwrap_or_default())
}

/// Accepts the duel `from` challenged `me` to, on the challenger's server
/// at `address`; returns the duel room.
pub async fn accept_duel(address: &str, from: &str, me: &Identity) -> Result<DuelMove, String> {
    let body = json!({"from": from, "id": me.id, "accept": true});
    let reply = http_json(address, "POST", "/api/duel/answer", Some(&body)).await?;
    let room = reply["room"]
        .as_str()
        .ok_or_else(|| format!("duel answer without a room: {reply}"))?;
    let address = reply["address"].as_str().unwrap_or(address);
    Ok(DuelMove {
        address: address.to_string(),
        room: room.to_string(),
        from: from.to_string(),
    })
}

/// Polls the inbox of `me` on `server` (and, with `directory`, the relayed
/// invites) every [`PRESENCE_POLL`] and accepts every duel challenge; each
/// accepted duel is sent on `tx`. Ends when `tx` is closed.
pub fn spawn_duel_watcher(
    me: Identity,
    server: String,
    directory: Option<String>,
    tx: mpsc::UnboundedSender<DuelMove>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let dir = directory.and_then(|u| DirectoryClient::new(&u).ok());
        let mut seen: Vec<String> = Vec::new();
        let mut fresh = |id: &str| {
            if seen.iter().any(|s| s == id) {
                return false;
            }
            seen.push(id.to_string());
            if seen.len() > 64 {
                seen.remove(0);
            }
            true
        };
        loop {
            if tx.is_closed() {
                return;
            }
            // (challenger's server, challenger id, note id)
            let mut challenges: Vec<(String, String, String)> = Vec::new();
            match presence(&server, &me).await {
                Ok(notes) => {
                    for n in notes {
                        if n["kind"] == "duel" {
                            if let (Some(from), Some(id)) =
                                (n["from_id"].as_str(), n["id"].as_str())
                            {
                                challenges.push((server.clone(), from.into(), id.into()));
                            }
                        }
                    }
                }
                Err(e) => tracing::debug!(bot = %me.name, "presence: {e}"),
            }
            if let Some(d) = &dir {
                match d.invites(&me.id).await {
                    Ok(list) => {
                        for i in list.invites.into_iter().filter(|i| i.invite.kind == "duel") {
                            challenges.push((i.invite.address, i.invite.from_id, i.id));
                        }
                    }
                    Err(e) => tracing::debug!(bot = %me.name, "directory invites: {e}"),
                }
            }
            for (address, from, note) in challenges {
                if !fresh(&note) {
                    continue;
                }
                match accept_duel(&address, &from, &me).await {
                    Ok(m) => {
                        tracing::info!(bot = %me.name, from = %m.from, room = %m.room, address = %m.address, "duel accepted");
                        if tx.send(m).is_err() {
                            return;
                        }
                    }
                    Err(e) => tracing::warn!(bot = %me.name, "cannot accept the duel: {e}"),
                }
            }
            tokio::time::sleep(PRESENCE_POLL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_directory::api::ServerInfo;

    fn entry(name: &str, players: u16, max: u16) -> ServerEntry {
        let server = ServerInfo {
            name: name.into(),
            address: format!("{name}.test:7777"),
            cert_hash: None,
            region: String::new(),
            mode: "team_brawl".into(),
            map_code: String::new(),
            rules_code: None,
            players,
            max_players: max,
            protocol: PROTOCOL_VERSION,
            data_hash: data_hash_hex(0),
            roster: Vec::new(),
        };
        ServerEntry {
            id: name.into(),
            age_ms: 0,
            server,
        }
    }

    #[test]
    fn bots_fill_the_fullest_server_with_room() {
        let list = [entry("b", 3, 8), entry("a", 8, 8), entry("c", 3, 8)];
        assert_eq!(choose_server(&list).unwrap().server.name, "b");
        assert!(choose_server(&[entry("full", 8, 8)]).is_none());
        assert!(choose_server(&[]).is_none());
    }

    #[test]
    fn identities_are_valid_and_stable() {
        let a = Identity::for_bot("bot 1!", 7);
        assert_eq!(a, Identity::for_bot("bot 1!", 7));
        assert_ne!(a.id, Identity::for_bot("bot 1!", 8).id);
        assert!(a.id.starts_with("bot-bot1-"));
        assert!((8..=64).contains(&a.id.len()));
        assert!(a
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:[]".contains(c)));
        assert!((16..=128).contains(&a.token.len()));
    }

    #[test]
    fn urls() {
        assert_eq!(
            host_port("ws://127.0.0.1:7777/ws").as_deref(),
            Some("127.0.0.1:7777")
        );
        assert_eq!(host_port("localhost:9").as_deref(), Some("localhost:9"));
        assert_eq!(ws_url("h:1"), "ws://h:1/ws");
    }
}
