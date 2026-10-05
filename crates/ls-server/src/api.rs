//! The JSON API on the game port (`/api/...`, M3 package B7): what the
//! browser's server list, party panel and duel toasts use. Every reply is
//! JSON with `Access-Control-Allow-Origin: *` (a page served by another
//! server or the Vite dev server can call it; `OPTIONS` answers
//! preflights); errors are `{"error":"..."}`.
//!
//! | Request | Body | Reply |
//! | --- | --- | --- |
//! | `GET /api/info` | - | [`InfoReply`]: name, public address, region, mode, world, players, protocol, data hash, directory, rooms |
//! | `GET /api/players` | - | `{"players":[{"id","name"}]}`: the roster (present players) |
//! | `POST /api/presence` | [`PresenceRequest`] | `{"invites":[Note],"address","directory"}`: registers the browser and returns its inbox |
//! | `POST /api/party` | [`PartyRequest`] | `{"party","room","address"}`: a party for the public room, a private room, or a new custom match |
//! | `POST /api/rooms` | [`CustomRequest`] | `{"room","address"}`: a private custom match with a world and rules |
//! | `POST /api/duel` | [`DuelRequest`] | `{"challenge","relayed"}`: challenges a player here, or relays through the directory |
//! | `POST /api/duel/answer` | [`AnswerRequest`] | `{"room","address"}`: accepts (opens the duel room) or declines |
//!
//! Status codes: 400 invalid body or field, 403 wrong token, 404 unknown
//! player, challenge, room or path, 405 wrong method, 502 directory failed,
//! 503 no private room free. There is no authentication beyond the
//! per-browser token of a player id (plan §3.1, "Directory trust").
//!
//! [`Social`] holds the state behind it (parties, presence and duels, the
//! room factory) and is shared with the [`crate::Hub`] (party routing).

use std::sync::{Arc, Mutex};
use std::time::Instant;

use ls_directory::api::{InviteRequest, ServerInfo};
use ls_directory::DirectoryClient;
use ls_worldgen::ShareCode;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::directory::{server_info, InfoInput};
use crate::duel::{self, Answer, DuelError, Duels, Note};
use crate::endpoint::CertHash;
use crate::party::{Parties, PartyCode, PartyError};
use crate::rooms::RoomCode;
use crate::rules::{load_rule_set, ServerRules};
use crate::server::{random_seed, PrivateRoomSpec, RoomFactory, ServerError};
use crate::sim::SimCommand;

/// Largest request body accepted.
pub const MAX_BODY: usize = 16 * 1024;

/// Parties, presence and duels of a server, and how to open rooms.
#[derive(Debug)]
pub struct Social {
    factory: RoomFactory,
    parties: Arc<Parties>,
    duels: Mutex<Duels>,
    address: String,
    directory: Option<DirectoryClient>,
    cert_hash: CertHash,
}

/// A JSON reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    fn json<T: Serialize>(value: &T) -> Reply {
        Reply {
            status: 200,
            body: serde_json::to_string(value).unwrap_or_else(|_| "{}".into()),
        }
    }

    fn error(status: u16, message: impl std::fmt::Display) -> Reply {
        Reply {
            status,
            body: serde_json::json!({ "error": message.to_string() }).to_string(),
        }
    }
}

impl From<DuelError> for Reply {
    fn from(e: DuelError) -> Reply {
        let status = match e {
            DuelError::WrongToken => 403,
            DuelError::NoChallenge => 404,
            DuelError::Full => 503,
            _ => 400,
        };
        Reply::error(status, e)
    }
}

impl From<ServerError> for Reply {
    fn from(e: ServerError) -> Reply {
        let status = match e {
            ServerError::Rooms(_) => 503,
            ServerError::ShareCode(_) | ServerError::RuleSet(_) => 400,
            _ => 500,
        };
        Reply::error(status, e)
    }
}

/// `GET /api/info` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoReply {
    #[serde(flatten)]
    pub server: ServerInfo,
    /// Directory URL the server registers with (`None`: not listed).
    pub directory: Option<String>,
    pub rooms: Vec<RoomJson>,
    /// Private rooms that can still be opened.
    pub private_rooms_free: usize,
}

/// One room in [`InfoReply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomJson {
    /// `public` or the room code.
    pub label: String,
    pub phase: String,
    pub mode: String,
    pub share_code: String,
    pub ruleset: String,
    pub players: usize,
    pub max_players: u8,
}

/// A player in the roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerJson {
    pub id: String,
    pub name: String,
}

/// `POST /api/presence` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceRequest {
    /// Public player id (8-64 of `A-Za-z0-9-_.:[]`).
    pub id: String,
    /// The browser's private token for that id (16-128 printable ASCII).
    pub token: String,
    pub name: String,
}

/// `POST /api/presence` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceReply {
    pub invites: Vec<Note>,
    /// This server's public `host:port`.
    pub address: String,
    /// Directory URL, so the browser can also poll it for invites relayed
    /// from other servers.
    pub directory: Option<String>,
}

/// `POST /api/party` body: every field optional.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartyRequest {
    /// The leader's private room (`None`: the public room).
    #[serde(default)]
    pub room: Option<String>,
    /// The leader's player id in that room (`Welcome::player_id`): the
    /// leader's connection becomes the party's first member.
    #[serde(default)]
    pub player: Option<u8>,
    /// With a world and/or rule set code the party gets a new private room
    /// playing them (a custom match).
    #[serde(default)]
    pub world_code: Option<String>,
    #[serde(default)]
    pub rules_code: Option<String>,
}

/// `POST /api/party` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyReply {
    pub party: String,
    /// The party's private room (`None`: the public room).
    pub room: Option<String>,
    pub address: String,
}

/// `POST /api/rooms` body.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CustomRequest {
    #[serde(default)]
    pub world_code: Option<String>,
    #[serde(default)]
    pub rules_code: Option<String>,
}

/// `POST /api/rooms` and `POST /api/duel/answer` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomReply {
    pub room: Option<String>,
    pub address: String,
}

/// `POST /api/duel` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuelRequest {
    /// The challenger (registered with `/api/presence`).
    pub id: String,
    pub token: String,
    pub name: String,
    /// The challenged player's public id.
    pub to: String,
    /// Their name as the challenger saw it (for a player on another server).
    #[serde(default)]
    pub to_name: Option<String>,
    /// The challenger's world (seed) and rule set code.
    #[serde(default)]
    pub world_code: Option<String>,
    #[serde(default)]
    pub rules_code: Option<String>,
}

/// `POST /api/duel` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuelReply {
    pub challenge: String,
    /// True if the challenge went through the directory (the player is on
    /// another server).
    pub relayed: bool,
}

/// `POST /api/duel/answer` body (sent to the challenger's server).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerRequest {
    /// The challenger's public id.
    pub from: String,
    /// The answering player's public id.
    pub id: String,
    pub accept: bool,
}

/// Parses a JSON body.
fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Reply> {
    serde_json::from_slice(body).map_err(|e| Reply::error(400, format!("invalid JSON: {e}")))
}

/// A share code from a request (`None` for none or empty).
fn world(code: &Option<String>) -> Result<Option<ShareCode>, Reply> {
    match code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => ShareCode::decode(c)
            .map(Some)
            .map_err(|e| Reply::error(400, format!("world code: {e}"))),
        None => Ok(None),
    }
}

/// A rule set code from a request, normalised (`None` for none or empty).
fn rules_code(code: &Option<String>) -> Option<String> {
    code.as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from)
}

impl Social {
    pub(crate) fn new(
        factory: RoomFactory,
        parties: Arc<Parties>,
        address: String,
        directory: Option<DirectoryClient>,
        cert_hash: CertHash,
    ) -> Social {
        Social {
            factory,
            parties,
            duels: Mutex::new(Duels::default()),
            address,
            directory,
            cert_hash,
        }
    }

    /// Opens private rooms.
    pub fn factory(&self) -> &RoomFactory {
        &self.factory
    }

    pub fn parties(&self) -> &Arc<Parties> {
        &self.parties
    }

    /// The public `host:port` the server announces.
    pub fn address(&self) -> &str {
        &self.address
    }

    fn directory_url(&self) -> Option<String> {
        self.factory.config().listing.directory.clone()
    }

    fn duels(&self) -> std::sync::MutexGuard<'_, Duels> {
        self.duels.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The canonical code and the room of a party; a party whose private
    /// room has closed is gone.
    pub fn resolve_party(&self, text: &str) -> Result<(PartyCode, Option<RoomCode>), PartyError> {
        let (code, room) = self.parties.resolve(text, Instant::now())?;
        if let Some(r) = &room {
            if !self.factory.rooms().is_open(r) {
                self.parties.room_closed(r);
                return Err(PartyError::Unknown(code.to_string()));
            }
        }
        Ok((code, room))
    }

    /// This server's directory entry now.
    pub fn server_info(&self) -> ServerInfo {
        let rooms = self.factory.rooms();
        let public = rooms.public_status();
        let roster = self.duels().roster(Instant::now());
        let cert_hash = self
            .cert_hash
            .read()
            .ok()
            .and_then(|h| h.as_ref().map(|h| h.hex()));
        server_info(&InfoInput {
            listing: &self.factory.config().listing,
            address: &self.address,
            cert_hash,
            public: &public,
            players: rooms.total_players(),
            data_hash: self.factory.config().match_cfg.data_hash(),
            roster: &roster,
        })
    }

    /// Handles one `/api/...` request (`OPTIONS` is answered by the caller).
    pub async fn handle(&self, method: &str, path: &str, body: &[u8]) -> Reply {
        let now = Instant::now();
        match (method, path) {
            ("GET" | "HEAD", "/api/info") => self.info(),
            ("GET" | "HEAD", "/api/players") => {
                let players: Vec<PlayerJson> = self
                    .duels()
                    .roster(now)
                    .into_iter()
                    .map(|(id, name)| PlayerJson { id, name })
                    .collect();
                Reply::json(&serde_json::json!({ "players": players }))
            }
            ("POST", "/api/presence") => match parse::<PresenceRequest>(body) {
                Ok(r) => self.presence(&r, now),
                Err(e) => e,
            },
            ("POST", "/api/party") => match parse::<PartyRequest>(body) {
                Ok(r) => self.party(&r).await,
                Err(e) => e,
            },
            ("POST", "/api/rooms") => match parse::<CustomRequest>(body) {
                Ok(r) => self.custom_room(&r).await,
                Err(e) => e,
            },
            ("POST", "/api/duel") => match parse::<DuelRequest>(body) {
                Ok(r) => self.duel(&r).await,
                Err(e) => e,
            },
            ("POST", "/api/duel/answer") => match parse::<AnswerRequest>(body) {
                Ok(r) => self.answer(&r).await,
                Err(e) => e,
            },
            (
                _,
                "/api/info" | "/api/players" | "/api/presence" | "/api/party" | "/api/rooms"
                | "/api/duel" | "/api/duel/answer",
            ) => Reply::error(405, "method not allowed"),
            _ => Reply::error(404, "no such API"),
        }
    }

    fn info(&self) -> Reply {
        let rooms = self.factory.rooms();
        let list: Vec<RoomJson> = rooms
            .statuses()
            .into_iter()
            .map(|(s, _)| RoomJson {
                label: s.label,
                phase: s.phase.to_string(),
                mode: s.mode.to_string(),
                share_code: s.share_code,
                ruleset: s.ruleset,
                players: s.players,
                max_players: s.max_players,
            })
            .collect();
        let used = list.len().saturating_sub(1);
        Reply::json(&InfoReply {
            server: self.server_info(),
            directory: self.directory_url(),
            rooms: list,
            private_rooms_free: rooms.max_private().saturating_sub(used),
        })
    }

    fn presence(&self, r: &PresenceRequest, now: Instant) -> Reply {
        let mut duels = self.duels();
        if let Err(e) = duels.register(&r.id, &r.token, &r.name, now) {
            return e.into();
        }
        Reply::json(&PresenceReply {
            invites: duels.inbox(&r.id, now),
            address: self.address.clone(),
            directory: self.directory_url(),
        })
    }

    /// The rules of a custom match: the code, else the public room's
    /// current rules.
    fn custom_rules(&self, code: Option<&str>) -> Result<ServerRules, Reply> {
        let code = match code {
            Some(c) => c.to_string(),
            None => self.factory.rooms().public_status().rules_code,
        };
        let base = if code.is_empty() {
            self.factory.config().match_cfg.rules.clone()
        } else {
            load_rule_set(&code).map_err(|e| Reply::error(400, format!("rule set code: {e}")))?
        };
        let rules = ServerRules {
            base,
            bans: Vec::new(),
            balance_list: None,
        };
        let cfg = &self.factory.config().match_cfg;
        rules
            .effective(&cfg.weapons, &cfg.gadgets)
            .map_err(|e| Reply::error(400, format!("rule set: {e}")))?;
        Ok(rules)
    }

    async fn open_custom(
        &self,
        world_code: &Option<String>,
        rules: &Option<String>,
    ) -> Result<RoomCode, Reply> {
        let world = world(world_code)?;
        let rules = self.custom_rules(rules_code(rules).as_deref())?;
        // A fixed world for every match of the room.
        let world =
            world.unwrap_or_else(|| ShareCode::new(random_seed(), ls_worldgen::Sliders::default()));
        let code = self
            .factory
            .create(PrivateRoomSpec::custom(rules, Some(world)))
            .await?;
        Ok(code)
    }

    async fn custom_room(&self, r: &CustomRequest) -> Reply {
        match self.open_custom(&r.world_code, &r.rules_code).await {
            Ok(code) => Reply::json(&RoomReply {
                room: Some(code.to_string()),
                address: self.address.clone(),
            }),
            Err(e) => e,
        }
    }

    async fn party(&self, r: &PartyRequest) -> Reply {
        let custom = rules_code(&r.world_code).is_some() || rules_code(&r.rules_code).is_some();
        let room = if custom {
            match self.open_custom(&r.world_code, &r.rules_code).await {
                Ok(code) => Some(code),
                Err(e) => return e,
            }
        } else {
            match r.room.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(text) => match RoomCode::parse(text) {
                    Some(code) if self.factory.rooms().is_open(&code) => Some(code),
                    _ => return Reply::error(404, format!("unknown room {text}")),
                },
                None => None,
            }
        };
        let party = match self.parties.create(room.clone(), Instant::now()) {
            Ok(p) => p,
            Err(e) => return Reply::error(503, e),
        };
        // The leader's connection joins the party (same room, not a custom
        // match: the leader moves to that room with the party code).
        if let (Some(player), false) = (r.player, custom) {
            let cmd = SimCommand::SetParty {
                player,
                party: party.to_string(),
            };
            if self.factory.rooms().send_to(room.as_ref(), cmd).is_err() {
                warn!(%party, "could not tag the party leader");
            }
        }
        info!(%party, room = room.as_ref().map_or("public", RoomCode::as_str), custom, "party created");
        Reply::json(&PartyReply {
            party: party.to_string(),
            room: room.map(|r| r.to_string()),
            address: self.address.clone(),
        })
    }

    async fn duel(&self, r: &DuelRequest) -> Reply {
        let now = Instant::now();
        // Codes are checked now, so accepting cannot fail on them.
        if let Err(e) = world(&r.world_code) {
            return e;
        }
        let rules = rules_code(&r.rules_code);
        if let Err(e) = duel::duel_rules(rules.as_deref()) {
            return Reply::error(400, format!("rule set code: {e}"));
        }
        let world_code = rules_code(&r.world_code);
        let (challenge, local) = {
            let mut duels = self.duels();
            if let Err(e) = duels.register(&r.id, &r.token, &r.name, now) {
                return e.into();
            }
            let to_name = r.to_name.clone().unwrap_or_default();
            let c = match duels.challenge(
                &r.id,
                &r.token,
                &r.to,
                &to_name,
                world_code.clone(),
                rules.clone(),
                now,
            ) {
                Ok(c) => c,
                Err(e) => return e.into(),
            };
            let local = duels.is_present(&r.to, now);
            (c, local)
        };
        if local {
            info!(from = %challenge.from_name, to = %challenge.to_name, "duel challenge");
            return Reply::json(&DuelReply {
                challenge: challenge.id,
                relayed: false,
            });
        }
        let Some(dir) = &self.directory else {
            return Reply::error(404, "that player is not on this server");
        };
        let invite = InviteRequest {
            to: r.to.clone(),
            kind: "duel".into(),
            from_name: challenge.from_name.clone(),
            from_id: challenge.from_id.clone(),
            address: self.address.clone(),
            room: None,
            world_code,
            rules_code: rules,
        };
        match dir.invite(&invite).await {
            Ok(_) => {
                info!(from = %challenge.from_name, to = %r.to, "duel challenge relayed through the directory");
                Reply::json(&DuelReply {
                    challenge: challenge.id,
                    relayed: true,
                })
            }
            Err(e) => Reply::error(502, format!("directory: {e}")),
        }
    }

    async fn answer(&self, r: &AnswerRequest) -> Reply {
        let now = Instant::now();
        let Some(c) = self.duels().pending(&r.from, &r.id, now) else {
            return DuelError::NoChallenge.into();
        };
        if !r.accept {
            let _ = self.duels().answer(&c.id, Answer::Declined);
            info!(from = %c.from_name, to = %c.to_name, "duel declined");
            return Reply::json(&RoomReply {
                room: None,
                address: self.address.clone(),
            });
        }
        let rules = match duel::duel_rules(c.rules_code.as_deref()) {
            Ok(r) => r,
            Err(e) => return e.into(),
        };
        let world = match world(&c.world_code) {
            Ok(w) => w,
            Err(e) => return e,
        };
        let spec = duel::duel_spec(rules, world, random_seed());
        let room = match self.factory.create(spec).await {
            Ok(room) => room,
            Err(e) => return e.into(),
        };
        let _ = self
            .duels()
            .answer(&c.id, Answer::Accepted { room: room.clone() });
        info!(from = %c.from_name, to = %c.to_name, %room, "duel accepted");
        Reply::json(&RoomReply {
            room: Some(room.to_string()),
            address: self.address.clone(),
        })
    }
}
