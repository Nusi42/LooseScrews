//! Duels and player presence (plan §3.1, M3 package B7).
//!
//! - **Presence**: browsers on this server's page poll `POST
//!   /api/presence` (see [`crate::api`]) with their public player id, their
//!   private token and their name. The first token for an id claims it;
//!   the poll returns the player's inbox (challenges, answers). Players seen
//!   within [`PRESENCE_TTL`] form the server's roster (the server list's
//!   player list, sent to the directory).
//! - **Challenge**: a player challenges another by public id, with an
//!   optional world (the challenger's seed) and rule set code. When the
//!   challenged player is present here the challenge waits in their inbox;
//!   otherwise the server relays it through the directory (`--directory`),
//!   addressed to the public id, pointing at this server. Challenges expire
//!   after [`CHALLENGE_TTL`] (the challenged player's toast shows that long).
//! - **Answer**: the challenged player answers on the challenger's server
//!   (by the challenger's id). Accepting opens a private two-player room
//!   with the challenger's world and rules ([`duel_spec`]); both players get
//!   its code (the answer's reply, the challenger's inbox) and join it.
//!
//! [`Duels`] is pure state with explicit time; the HTTP side creates rooms.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ls_sim::{ModeKind, RuleSet};
use ls_worldgen::{ShareCode, Sliders};
use serde::{Deserialize, Serialize};

use crate::lobby::LobbyTiming;
use crate::playlist::{builtin_rule_set, Playlist, WorldChoice};
use crate::rooms::RoomCode;
use crate::rules::{load_rule_set, ServerRules};
use crate::server::{PrivateRoomSpec, ServerError};

/// A player not seen for this long leaves the roster.
pub const PRESENCE_TTL: Duration = Duration::from_secs(15);
/// A challenge (and its answer) is kept this long.
pub const CHALLENGE_TTL: Duration = Duration::from_secs(30);
/// Most players tracked at once (oldest first out).
pub const MAX_PRESENT: usize = 256;
/// Most pending challenges at once.
pub const MAX_CHALLENGES: usize = 512;
/// Longest player name kept.
pub const MAX_NAME: usize = 48;
/// Players in a duel room.
pub const DUEL_PLAYERS: u8 = 2;
/// Kills to win the fallback duel rules (without `data/rulesets/duel.ron`).
const DUEL_KILLS: f32 = 10.0;
/// Time limit of the fallback duel rules.
const DUEL_SECS: f32 = 300.0;

/// What [`Duels`] refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DuelError {
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("this player id belongs to another browser")]
    WrongToken,
    #[error("register with /api/presence first")]
    NotRegistered,
    #[error("you cannot challenge yourself")]
    SelfChallenge,
    #[error("no pending challenge from that player")]
    NoChallenge,
    #[error("too many players or challenges")]
    Full,
}

/// A message in a player's inbox (`POST /api/presence` reply).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// Unique per challenge and kind (clients de-duplicate by it).
    pub id: String,
    /// `duel` (a challenge for you), `accepted` or `declined` (the answer
    /// to your challenge).
    pub kind: String,
    /// The other player.
    pub from_id: String,
    pub from_name: String,
    /// Room code of the duel (`accepted`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    /// The challenger's world and rules (`duel`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_code: Option<String>,
    pub expires_in_ms: u64,
}

#[derive(Debug, Clone)]
struct Presence {
    token: String,
    name: String,
    last_seen: Instant,
}

/// How a challenge was answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Accepted { room: RoomCode },
    Declined,
}

/// A challenge from `from` to `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub id: String,
    pub from_id: String,
    pub from_name: String,
    pub to_id: String,
    pub to_name: String,
    pub world_code: Option<String>,
    pub rules_code: Option<String>,
    pub created: Instant,
    pub answer: Option<Answer>,
}

/// Presence and challenges of one server.
#[derive(Debug, Default)]
pub struct Duels {
    present: BTreeMap<String, Presence>,
    challenges: BTreeMap<String, Challenge>,
    next_id: u64,
}

/// Checks a public player id (8-64 of `A-Za-z0-9-_.:[]`, as the directory).
pub fn valid_player_id(id: &str) -> bool {
    ls_directory::api::validate_player_id("id", id).is_ok()
}

/// A display name: trimmed, control characters removed, at most
/// [`MAX_NAME`] characters; `None` if nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let s: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

impl Duels {
    fn prune(&mut self, now: Instant) {
        self.present
            .retain(|_, p| now.saturating_duration_since(p.last_seen) < PRESENCE_TTL);
        self.challenges
            .retain(|_, c| now.saturating_duration_since(c.created) < CHALLENGE_TTL);
    }

    /// Registers or refreshes a player (`POST /api/presence`).
    pub fn register(
        &mut self,
        id: &str,
        token: &str,
        name: &str,
        now: Instant,
    ) -> Result<(), DuelError> {
        if !valid_player_id(id) {
            return Err(DuelError::Invalid("player id"));
        }
        if token.len() < 16 || token.len() > 128 || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(DuelError::Invalid("token"));
        }
        let name = clean_name(name).ok_or(DuelError::Invalid("name"))?;
        self.prune(now);
        match self.present.get_mut(id) {
            Some(p) if p.token != token => return Err(DuelError::WrongToken),
            Some(p) => {
                p.name = name;
                p.last_seen = now;
            }
            None => {
                if self.present.len() >= MAX_PRESENT {
                    // Drop the stalest player.
                    let stalest = self
                        .present
                        .iter()
                        .min_by_key(|(_, p)| p.last_seen)
                        .map(|(k, _)| k.clone());
                    if let Some(k) = stalest {
                        self.present.remove(&k);
                    }
                }
                self.present.insert(
                    id.to_string(),
                    Presence {
                        token: token.to_string(),
                        name,
                        last_seen: now,
                    },
                );
            }
        }
        Ok(())
    }

    /// Whether `id` is present on this server.
    pub fn is_present(&mut self, id: &str, now: Instant) -> bool {
        self.prune(now);
        self.present.contains_key(id)
    }

    /// The roster: present players (public id, name), by name.
    pub fn roster(&mut self, now: Instant) -> Vec<(String, String)> {
        self.prune(now);
        let mut out: Vec<(String, String)> = self
            .present
            .iter()
            .map(|(id, p)| (id.clone(), p.name.clone()))
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        out
    }

    /// The inbox of `id`: challenges to it still open, answers to its
    /// challenges.
    pub fn inbox(&mut self, id: &str, now: Instant) -> Vec<Note> {
        self.prune(now);
        let left = |c: &Challenge| {
            CHALLENGE_TTL
                .saturating_sub(now.saturating_duration_since(c.created))
                .as_millis() as u64
        };
        let mut out = Vec::new();
        for c in self.challenges.values() {
            if c.to_id == id && c.answer.is_none() {
                out.push(Note {
                    id: c.id.clone(),
                    kind: "duel".into(),
                    from_id: c.from_id.clone(),
                    from_name: c.from_name.clone(),
                    room: None,
                    world_code: c.world_code.clone(),
                    rules_code: c.rules_code.clone(),
                    expires_in_ms: left(c),
                });
            }
            if c.from_id == id {
                let (kind, room) = match &c.answer {
                    Some(Answer::Accepted { room }) => ("accepted", Some(room.to_string())),
                    Some(Answer::Declined) => ("declined", None),
                    None => continue,
                };
                out.push(Note {
                    id: format!("{}-{kind}", c.id),
                    kind: kind.into(),
                    from_id: c.to_id.clone(),
                    from_name: c.to_name.clone(),
                    room,
                    world_code: None,
                    rules_code: None,
                    expires_in_ms: left(c),
                });
            }
        }
        out
    }

    /// Records a challenge from the registered player `from_id` (proved by
    /// `token`) to `to_id`. Returns it; the caller relays it through the
    /// directory unless `to_id` [`Duels::is_present`]. A new challenge
    /// replaces an open one between the same two players.
    #[allow(clippy::too_many_arguments)]
    pub fn challenge(
        &mut self,
        from_id: &str,
        token: &str,
        to_id: &str,
        to_name: &str,
        world_code: Option<String>,
        rules_code: Option<String>,
        now: Instant,
    ) -> Result<Challenge, DuelError> {
        self.prune(now);
        if !valid_player_id(to_id) {
            return Err(DuelError::Invalid("player id"));
        }
        let from = self.present.get(from_id).ok_or(DuelError::NotRegistered)?;
        if from.token != token {
            return Err(DuelError::WrongToken);
        }
        if from_id == to_id {
            return Err(DuelError::SelfChallenge);
        }
        let from_name = from.name.clone();
        let to_name = self
            .present
            .get(to_id)
            .map(|p| p.name.clone())
            .or_else(|| clean_name(to_name))
            .unwrap_or_else(|| "player".into());
        self.challenges
            .retain(|_, c| !(c.from_id == from_id && c.to_id == to_id && c.answer.is_none()));
        if self.challenges.len() >= MAX_CHALLENGES {
            return Err(DuelError::Full);
        }
        self.next_id += 1;
        let c = Challenge {
            id: format!("c{}", self.next_id),
            from_id: from_id.to_string(),
            from_name,
            to_id: to_id.to_string(),
            to_name,
            world_code,
            rules_code,
            created: now,
            answer: None,
        };
        self.challenges.insert(c.id.clone(), c.clone());
        Ok(c)
    }

    /// The open challenge from `from_id` to `to_id` (the newest), which the
    /// challenged player is about to answer.
    pub fn pending(&mut self, from_id: &str, to_id: &str, now: Instant) -> Option<Challenge> {
        self.prune(now);
        self.challenges
            .values()
            .filter(|c| c.from_id == from_id && c.to_id == to_id && c.answer.is_none())
            .max_by_key(|c| c.created)
            .cloned()
    }

    /// Stores the answer to challenge `id` (shown in the challenger's
    /// inbox until the challenge expires).
    pub fn answer(&mut self, id: &str, answer: Answer) -> Result<(), DuelError> {
        let c = self.challenges.get_mut(id).ok_or(DuelError::NoChallenge)?;
        c.answer = Some(answer);
        Ok(())
    }
}

/// The rules of a duel: the challenger's rule set code, else the built-in
/// duel rule set (`data/rulesets/duel.ron`), else Team Brawl free-for-all to
/// 10 kills in 5 minutes.
pub fn duel_rules(rules_code: Option<&str>) -> Result<ServerRules, ServerError> {
    let base = match rules_code {
        Some(code) => load_rule_set(code)?,
        None => match builtin_rule_set("duel") {
            Some(text) => RuleSet::from_ron("duel.ron", text)?,
            None => {
                let mut r = RuleSet::default_for(ModeKind::TeamBrawl);
                r.name = "Duel".into();
                r.mode.set_field("teams", 0.0);
                r.mode.set_field("kill_target", DUEL_KILLS);
                r.mode.set_field("match_secs", DUEL_SECS);
                r
            }
        },
    };
    let rules = ServerRules {
        base,
        bans: Vec::new(),
        balance_list: None,
    };
    rules.effective(
        &ls_sim::WeaponTable::builtin(),
        &ls_sim::GadgetTable::builtin(),
    )?;
    Ok(rules)
}

/// A private duel room: two players, the challenger's world (a fixed
/// random one if none was given, so rematches stay on it) and rules,
/// every match on the same world.
pub fn duel_spec(rules: ServerRules, world: Option<ShareCode>, seed: u64) -> PrivateRoomSpec {
    let world = world.unwrap_or_else(|| ShareCode::new(seed, Sliders::default()));
    let source = rules.base.name.clone();
    PrivateRoomSpec {
        playlist: Playlist::single(&source, rules, WorldChoice::Fixed(world)),
        lobby: None::<LobbyTiming>,
        max_players: Some(DUEL_PLAYERS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN_A: &str = "token-aaaaaaaaaaaaaaaa";
    const TOKEN_B: &str = "token-bbbbbbbbbbbbbbbb";

    #[test]
    fn presence_claims_ids_and_expires() {
        let t0 = Instant::now();
        let mut d = Duels::default();
        d.register("player-a01", TOKEN_A, " Ace ", t0).unwrap();
        assert_eq!(
            d.register("player-a01", TOKEN_B, "Mallory", t0),
            Err(DuelError::WrongToken)
        );
        assert_eq!(
            d.register("bad id", TOKEN_A, "x", t0),
            Err(DuelError::Invalid("player id"))
        );
        assert_eq!(
            d.register("player-a02", "short", "x", t0),
            Err(DuelError::Invalid("token"))
        );
        assert_eq!(
            d.register("player-a02", TOKEN_B, "\n ", t0),
            Err(DuelError::Invalid("name"))
        );
        d.register("player-b01", TOKEN_B, "Bea", t0).unwrap();
        assert_eq!(
            d.roster(t0),
            vec![
                ("player-a01".to_string(), "Ace".to_string()),
                ("player-b01".to_string(), "Bea".to_string())
            ]
        );
        let later = t0 + PRESENCE_TTL - Duration::from_secs(1);
        d.register("player-a01", TOKEN_A, "Ace", later).unwrap();
        let gone = t0 + PRESENCE_TTL + Duration::from_secs(1);
        assert_eq!(d.roster(gone).len(), 1, "Bea timed out");
        assert!(!d.is_present("player-b01", gone));
        // An expired id can be claimed again.
        d.register("player-b01", TOKEN_A, "Other", gone).unwrap();
    }

    #[test]
    fn challenges_reach_the_inbox_and_carry_the_answer_back() {
        let t0 = Instant::now();
        let mut d = Duels::default();
        d.register("player-a01", TOKEN_A, "Ace", t0).unwrap();
        d.register("player-b01", TOKEN_B, "Bea", t0).unwrap();
        assert_eq!(
            d.challenge("player-a01", TOKEN_B, "player-b01", "", None, None, t0),
            Err(DuelError::WrongToken)
        );
        assert_eq!(
            d.challenge("player-a01", TOKEN_A, "player-a01", "", None, None, t0),
            Err(DuelError::SelfChallenge)
        );
        assert_eq!(
            d.challenge("player-x01", TOKEN_A, "player-b01", "", None, None, t0),
            Err(DuelError::NotRegistered)
        );
        let world = Some("LS2-ABC".to_string());
        let c = d
            .challenge(
                "player-a01",
                TOKEN_A,
                "player-b01",
                "",
                world.clone(),
                None,
                t0,
            )
            .unwrap();
        assert_eq!(c.to_name, "Bea");
        let inbox = d.inbox("player-b01", t0);
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].kind, "duel");
        assert_eq!(inbox[0].from_name, "Ace");
        assert_eq!(inbox[0].world_code, world);
        assert!(d.inbox("player-a01", t0).is_empty());
        // A second challenge replaces the open one.
        let c2 = d
            .challenge("player-a01", TOKEN_A, "player-b01", "", None, None, t0)
            .unwrap();
        assert_eq!(d.inbox("player-b01", t0).len(), 1);
        assert_eq!(d.pending("player-a01", "player-b01", t0).unwrap().id, c2.id);
        assert!(d.pending("player-b01", "player-a01", t0).is_none());
        let room = RoomCode::from_bits(77);
        d.answer(&c2.id, Answer::Accepted { room: room.clone() })
            .unwrap();
        assert!(d.inbox("player-b01", t0).is_empty(), "answered");
        let back = d.inbox("player-a01", t0);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].kind, "accepted");
        assert_eq!(back[0].room.as_deref(), Some(room.as_str()));
        assert_eq!(back[0].from_name, "Bea");
        // Everything expires.
        assert!(d.inbox("player-a01", t0 + CHALLENGE_TTL).is_empty());
        assert_eq!(
            d.answer(&c2.id, Answer::Declined),
            Err(DuelError::NoChallenge)
        );
    }

    #[test]
    fn a_challenge_to_an_absent_player_is_kept_for_its_answer() {
        let t0 = Instant::now();
        let mut d = Duels::default();
        d.register("player-a01", TOKEN_A, "Ace", t0).unwrap();
        let c = d
            .challenge(
                "player-a01",
                TOKEN_A,
                "remote-b01",
                "Far Bea",
                None,
                None,
                t0,
            )
            .unwrap();
        assert!(!d.is_present("remote-b01", t0));
        assert_eq!(c.to_name, "Far Bea");
        assert!(d.pending("player-a01", "remote-b01", t0).is_some());
    }

    #[test]
    fn duel_rules_default_to_a_free_for_all_brawl() {
        let r = duel_rules(None).unwrap();
        assert_eq!(r.base.mode.teams(), 0);
        if builtin_rule_set("duel").is_none() {
            assert_eq!(r.base.mode.kind(), ModeKind::TeamBrawl);
            assert_eq!(r.base.mode.match_secs(), DUEL_SECS);
            assert!(r
                .base
                .mode
                .fields()
                .iter()
                .any(|f| f.name == "kill_target" && f.value == DUEL_KILLS));
        }
        let code = RuleSet::sandbox_all_weapons().to_code().unwrap();
        let r = duel_rules(Some(&code)).unwrap();
        assert_eq!(r.base.name, RuleSet::sandbox_all_weapons().name);
        assert!(duel_rules(Some("LR00-0000")).is_err());
        let world = ShareCode::new(0xd0e1, Sliders::new(30, 50));
        let spec = duel_spec(duel_rules(None).unwrap(), Some(world), 1);
        assert_eq!(spec.max_players, Some(DUEL_PLAYERS));
        assert!(matches!(spec.playlist.entries()[0].world, WorldChoice::Fixed(c) if c == world));
    }
}
