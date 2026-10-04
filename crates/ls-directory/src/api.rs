//! JSON wire types of the directory API (shared by the service and the
//! [`crate::client`]). Field names are the JSON keys; the browser client reads
//! the same shapes (`docs/architecture/directory.md`).
//!
//! The data hash is a 16-digit lower-case hex string, not a JSON number: a
//! `u64` does not survive JavaScript's 53-bit numbers.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Longest server name.
pub const MAX_NAME: usize = 48;
/// Longest public address (`host:port`).
pub const MAX_ADDRESS: usize = 255;
/// Longest region tag.
pub const MAX_REGION: usize = 16;
/// Longest mode id.
pub const MAX_MODE: usize = 32;
/// Longest world (`LS..`), rule set (`LR..`) or room code.
pub const MAX_CODE: usize = 512;
/// Longest server secret.
pub const MAX_SECRET: usize = 128;
/// Shortest server secret.
pub const MIN_SECRET: usize = 16;
/// Most roster entries per server.
pub const MAX_ROSTER: usize = 64;
/// Longest player id (public invite address of a player).
pub const MAX_PLAYER_ID: usize = 64;
/// Shortest player id.
pub const MIN_PLAYER_ID: usize = 8;
/// Longest invite kind.
pub const MAX_KIND: usize = 16;

/// A player as listed in a server's roster: display name plus the public
/// player id that invites are addressed to. The id is not the browser's
/// private player token (anyone who reads the list sees it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub name: String,
    pub id: String,
}

/// What a game server publishes about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Display name (`--server-name`).
    pub name: String,
    /// Public `host:port` of the game port (WebSocket, HTTP, `/cert-hash`).
    pub address: String,
    /// WebTransport certificate SHA-256 as 64 hex digits, if enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_hash: Option<String>,
    /// Region tag (`--region`, e.g. `eu-west`); may be empty.
    #[serde(default)]
    pub region: String,
    /// Current mode id (e.g. `team_brawl`, `lobby`).
    #[serde(default)]
    pub mode: String,
    /// Current world share code (`LS..`).
    #[serde(default)]
    pub map_code: String,
    /// Current rule set code (`LR..`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_code: Option<String>,
    pub players: u16,
    pub max_players: u16,
    /// Game protocol version (`ls_net::PROTOCOL_VERSION`).
    pub protocol: u16,
    /// Game data hash (`ls_sim::data_hash`) as 16 lower-case hex digits.
    pub data_hash: String,
    /// Players on the server (for challenges), at most [`MAX_ROSTER`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roster: Vec<RosterEntry>,
}

/// `POST /servers` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    /// Random per-server secret; the first heartbeat for an address claims
    /// it, later heartbeats (and `DELETE`) must repeat it.
    pub secret: String,
    pub server: ServerInfo,
}

/// `POST /servers` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatReply {
    /// Directory id of the entry (stable while the entry lives).
    pub id: String,
    /// Seconds after the last heartbeat until the entry expires.
    pub ttl_secs: u64,
    /// Suggested heartbeat interval in seconds.
    pub heartbeat_secs: u64,
}

/// `DELETE /servers` body: removes the entry before its TTL (server shutdown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unregister {
    pub address: String,
    pub secret: String,
}

/// One entry of `GET /servers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    pub id: String,
    /// Milliseconds since the last heartbeat.
    pub age_ms: u64,
    pub server: ServerInfo,
}

/// `GET /servers` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerList {
    pub servers: Vec<ServerEntry>,
}

/// `POST /invites` body: a message for one player id, relayed until it
/// expires (duel challenges, and their answers, between servers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteRequest {
    /// Recipient's public player id.
    pub to: String,
    /// What the invite is (`duel`, `accept`, `decline`, ...); free-form for
    /// the users of the relay, at most [`MAX_KIND`] characters.
    pub kind: String,
    pub from_name: String,
    /// Sender's public player id (for answers).
    pub from_id: String,
    /// Server the invite points at (`host:port`).
    pub address: String,
    /// Room code on that server, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    /// World share code (`LS..`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_code: Option<String>,
    /// Rule set code (`LR..`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_code: Option<String>,
}

/// `POST /invites` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteReply {
    pub id: String,
    pub expires_in_secs: u64,
}

/// One relayed invite in `GET /invites?player=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    /// Unique id: clients poll and see an invite until it expires, so they
    /// de-duplicate by it.
    pub id: String,
    /// Milliseconds until the invite expires.
    pub expires_in_ms: u64,
    pub invite: InviteRequest,
}

/// `GET /invites?player=` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteList {
    pub invites: Vec<Invite>,
}

/// Error body of every non-2xx JSON reply: `{"error":"..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReply {
    pub error: String,
}

/// Why a request body was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Invalid {
    #[error("{0} is empty")]
    Empty(&'static str),
    #[error("{0} is too long (max {1})")]
    TooLong(&'static str, usize),
    #[error("{0} is too short (min {1})")]
    TooShort(&'static str, usize),
    #[error("{0} has invalid characters")]
    Chars(&'static str),
    #[error("players > max_players")]
    Players,
    #[error("data_hash must be 16 lower-case hex digits")]
    DataHash,
    #[error("cert_hash must be 64 lower-case hex digits")]
    CertHash,
    #[error("roster has more than {0} entries")]
    Roster(usize),
}

fn text(field: &'static str, s: &str, max: usize, required: bool) -> Result<(), Invalid> {
    if required && s.trim().is_empty() {
        return Err(Invalid::Empty(field));
    }
    if s.chars().count() > max {
        return Err(Invalid::TooLong(field, max));
    }
    if s.chars().any(char::is_control) {
        return Err(Invalid::Chars(field));
    }
    Ok(())
}

/// Token-like strings: ASCII letters, digits and `-_.:[]` (addresses, codes,
/// ids, secrets); no whitespace.
fn token(field: &'static str, s: &str, min: usize, max: usize) -> Result<(), Invalid> {
    if s.is_empty() {
        return Err(Invalid::Empty(field));
    }
    if s.len() > max {
        return Err(Invalid::TooLong(field, max));
    }
    if s.len() < min {
        return Err(Invalid::TooShort(field, min));
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_.:[]".contains(&b))
    {
        return Err(Invalid::Chars(field));
    }
    Ok(())
}

fn opt_token(field: &'static str, s: &Option<String>, max: usize) -> Result<(), Invalid> {
    s.as_deref().map_or(Ok(()), |s| token(field, s, 1, max))
}

fn hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Formats a data hash the way the directory expects it.
pub fn data_hash_hex(hash: u64) -> String {
    format!("{hash:016x}")
}

/// Checks a public `host:port` address.
pub fn validate_address(address: &str) -> Result<(), Invalid> {
    token("address", address, 1, MAX_ADDRESS)
}

/// Checks a player id.
pub fn validate_player_id(field: &'static str, id: &str) -> Result<(), Invalid> {
    token(field, id, MIN_PLAYER_ID, MAX_PLAYER_ID)
}

impl ServerInfo {
    pub fn validate(&self) -> Result<(), Invalid> {
        text("name", &self.name, MAX_NAME, true)?;
        validate_address(&self.address)?;
        if let Some(h) = &self.cert_hash {
            if !hex(h, 64) {
                return Err(Invalid::CertHash);
            }
        }
        text("region", &self.region, MAX_REGION, false)?;
        text("mode", &self.mode, MAX_MODE, false)?;
        if !self.map_code.is_empty() {
            token("map_code", &self.map_code, 1, MAX_CODE)?;
        }
        opt_token("rules_code", &self.rules_code, MAX_CODE)?;
        if self.players > self.max_players {
            return Err(Invalid::Players);
        }
        if !hex(&self.data_hash, 16) {
            return Err(Invalid::DataHash);
        }
        if self.roster.len() > MAX_ROSTER {
            return Err(Invalid::Roster(MAX_ROSTER));
        }
        for p in &self.roster {
            text("roster name", &p.name, MAX_NAME, true)?;
            validate_player_id("roster id", &p.id)?;
        }
        Ok(())
    }
}

impl Heartbeat {
    pub fn validate(&self) -> Result<(), Invalid> {
        token("secret", &self.secret, MIN_SECRET, MAX_SECRET)?;
        self.server.validate()
    }
}

impl InviteRequest {
    pub fn validate(&self) -> Result<(), Invalid> {
        validate_player_id("to", &self.to)?;
        token("kind", &self.kind, 1, MAX_KIND)?;
        text("from_name", &self.from_name, MAX_NAME, true)?;
        validate_player_id("from_id", &self.from_id)?;
        validate_address(&self.address)?;
        opt_token("room", &self.room, MAX_CODE)?;
        opt_token("world_code", &self.world_code, MAX_CODE)?;
        opt_token("rules_code", &self.rules_code, MAX_CODE)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn info(address: &str) -> ServerInfo {
        ServerInfo {
            name: "Test server".into(),
            address: address.into(),
            cert_hash: Some("ab".repeat(32)),
            region: "eu".into(),
            mode: "team_brawl".into(),
            map_code: "LS2-ABCDEF".into(),
            rules_code: None,
            players: 3,
            max_players: 16,
            protocol: 5,
            data_hash: data_hash_hex(0x0123_4567_89ab_cdef),
            roster: vec![RosterEntry {
                name: "Ace".into(),
                id: "player-0001".into(),
            }],
        }
    }

    #[test]
    fn valid_info_passes_and_round_trips() {
        let i = info("game.example.com:7777");
        i.validate().unwrap();
        let json = serde_json::to_string(&i).unwrap();
        assert!(json.contains(r#""data_hash":"0123456789abcdef""#), "{json}");
        assert!(
            !json.contains("rules_code"),
            "None fields are omitted: {json}"
        );
        assert_eq!(serde_json::from_str::<ServerInfo>(&json).unwrap(), i);
    }

    #[test]
    fn invalid_fields_are_refused() {
        let base = info("127.0.0.1:7777");
        type Edit = fn(&mut ServerInfo);
        let cases: Vec<(Edit, Invalid)> = vec![
            (|i| i.name = " ".into(), Invalid::Empty("name")),
            (
                |i| i.name = "x".repeat(49),
                Invalid::TooLong("name", MAX_NAME),
            ),
            (|i| i.name = "a\nb".into(), Invalid::Chars("name")),
            (|i| i.address = "a b:1".into(), Invalid::Chars("address")),
            (|i| i.address = String::new(), Invalid::Empty("address")),
            (|i| i.players = 17, Invalid::Players),
            (|i| i.data_hash = "123".into(), Invalid::DataHash),
            (
                |i| i.data_hash = "0123456789ABCDEF".into(),
                Invalid::DataHash,
            ),
            (|i| i.cert_hash = Some("zz".into()), Invalid::CertHash),
            (
                |i| {
                    i.roster = vec![
                        RosterEntry {
                            name: "a".into(),
                            id: "abcdefgh".into()
                        };
                        65
                    ]
                },
                Invalid::Roster(MAX_ROSTER),
            ),
            (
                |i| i.roster[0].id = "short".into(),
                Invalid::TooShort("roster id", MIN_PLAYER_ID),
            ),
        ];
        for (f, want) in cases {
            let mut i = base.clone();
            f(&mut i);
            assert_eq!(i.validate(), Err(want));
        }
        let hb = Heartbeat {
            secret: "short".into(),
            server: base,
        };
        assert_eq!(hb.validate(), Err(Invalid::TooShort("secret", MIN_SECRET)));
    }

    #[test]
    fn invites_validate() {
        let mut inv = InviteRequest {
            to: "player-0002".into(),
            kind: "duel".into(),
            from_name: "Ace".into(),
            from_id: "player-0001".into(),
            address: "127.0.0.1:7777".into(),
            room: Some("R-ABC123".into()),
            world_code: Some("LS2-ABCDEF".into()),
            rules_code: None,
        };
        inv.validate().unwrap();
        inv.kind = "a b".into();
        assert_eq!(inv.validate(), Err(Invalid::Chars("kind")));
    }
}
