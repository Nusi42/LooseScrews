//! Party codes (plan §3.1, M3 package B7): a party groups players so they
//! play in the same room and, in team modes, on the same team.
//!
//! - A party is created on one server for its leader (`POST /api/party`,
//!   see [`crate::api`]) and lives on that server only. Its code is
//!   `P-XXXXXX` (6 Crockford base32 characters, parsed leniently like room
//!   codes).
//! - A party points at a room: the public room, or a private room (a custom
//!   match created with the party, or the leader's private room). A `Hello`
//!   with a party code joins that room ([`crate::endpoint::Hub::open`]).
//! - Teams: party members ask for the team of a party mate already in the
//!   room; team balance still decides (`ls_game::server::Room::join` only
//!   grants the smallest team, so the team sizes never differ by more than
//!   one), else the party is split by join order. On every match transition
//!   the sim thread re-plans the teams with [`plan_joins`]: whole parties on
//!   one team where the balance allows, and a join order the room accepts.
//! - A party lives while it has members connected, and [`PARTY_IDLE`] after
//!   its last member left (or after its creation if nobody ever joined); it
//!   ends with its private room.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::rooms::{code_from_bits, parse_code, RoomCode};

/// A party without members is forgotten after this long.
pub const PARTY_IDLE: Duration = Duration::from_secs(10 * 60);
/// Most parties on one server at once.
pub const MAX_PARTIES: usize = 512;

/// A party code, `P-XXXXXX`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartyCode(String);

impl PartyCode {
    /// The code for 30 random bits.
    pub fn from_bits(bits: u64) -> PartyCode {
        PartyCode(code_from_bits("P-", bits))
    }

    /// Parses a code leniently (case, missing `P-`, spaces, O/I/L).
    pub fn parse(text: &str) -> Option<PartyCode> {
        parse_code("P-", text).map(PartyCode)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PartyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    /// The room its members join (`None`: the public room).
    pub room: Option<RoomCode>,
    /// Members connected now (counted by the sim threads).
    pub members: usize,
    /// When the party last had no members (its creation at first).
    pub idle_since: Option<Instant>,
}

/// Why a party could not be created or resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PartyError {
    #[error("too many parties on this server")]
    Full,
    #[error("unknown party {0}")]
    Unknown(String),
}

/// The parties of a server (shared by the HTTP API, the hub's routing and
/// the sim threads).
#[derive(Debug)]
pub struct Parties {
    parties: Mutex<BTreeMap<PartyCode, Party>>,
    counter: AtomicU64,
}

impl Parties {
    /// An empty registry; `seed` varies the codes between servers.
    pub fn new(seed: u64) -> Parties {
        Parties {
            parties: Mutex::new(BTreeMap::new()),
            counter: AtomicU64::new(seed),
        }
    }

    /// Creates a party for the room `room` (`None`: the public room).
    pub fn create(&self, room: Option<RoomCode>, now: Instant) -> Result<PartyCode, PartyError> {
        let mut parties = self.parties.lock().map_err(|_| PartyError::Full)?;
        prune_idle(&mut parties, now);
        if parties.len() >= MAX_PARTIES {
            return Err(PartyError::Full);
        }
        let code = loop {
            let bits = ls_math::rng::mix64(self.counter.fetch_add(1, Ordering::Relaxed));
            let c = PartyCode::from_bits(bits);
            if !parties.contains_key(&c) {
                break c;
            }
        };
        parties.insert(
            code.clone(),
            Party {
                room,
                members: 0,
                idle_since: Some(now),
            },
        );
        Ok(code)
    }

    /// The canonical code and room of the party `text` names.
    pub fn resolve(
        &self,
        text: &str,
        now: Instant,
    ) -> Result<(PartyCode, Option<RoomCode>), PartyError> {
        let unknown = || PartyError::Unknown(text.trim().to_string());
        let code = PartyCode::parse(text).ok_or_else(unknown)?;
        let mut parties = self.parties.lock().map_err(|_| unknown())?;
        prune_idle(&mut parties, now);
        let party = parties.get(&code).ok_or_else(unknown)?;
        Ok((code, party.room.clone()))
    }

    /// The party `code` names, if it exists.
    pub fn get(&self, code: &PartyCode) -> Option<Party> {
        self.parties.lock().ok()?.get(code).cloned()
    }

    /// A member of `code` connected (a sim thread added it).
    pub fn member_joined(&self, code: &str) {
        let Some(code) = PartyCode::parse(code) else {
            return;
        };
        if let Ok(mut parties) = self.parties.lock() {
            if let Some(p) = parties.get_mut(&code) {
                p.members += 1;
                p.idle_since = None;
            }
        }
    }

    /// A member of `code` left (a sim thread removed it).
    pub fn member_left(&self, code: &str, now: Instant) {
        let Some(code) = PartyCode::parse(code) else {
            return;
        };
        if let Ok(mut parties) = self.parties.lock() {
            if let Some(p) = parties.get_mut(&code) {
                p.members = p.members.saturating_sub(1);
                if p.members == 0 {
                    p.idle_since = Some(now);
                }
            }
        }
    }

    /// Ends the parties of a private room that closed.
    pub fn room_closed(&self, room: &RoomCode) {
        if let Ok(mut parties) = self.parties.lock() {
            parties.retain(|_, p| p.room.as_ref() != Some(room));
        }
    }

    /// Parties now (after dropping idle ones).
    pub fn len(&self, now: Instant) -> usize {
        self.parties.lock().map_or(0, |mut p| {
            prune_idle(&mut p, now);
            p.len()
        })
    }

    pub fn is_empty(&self, now: Instant) -> bool {
        self.len(now) == 0
    }
}

fn prune_idle(parties: &mut BTreeMap<PartyCode, Party>, now: Instant) {
    parties.retain(|_, p| {
        p.idle_since
            .is_none_or(|t| now.saturating_duration_since(t) < PARTY_IDLE)
    });
}

/// A player about to join a room, for [`plan_joins`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joiner {
    /// Any id that identifies the player to the caller.
    pub id: u8,
    /// Canonical party code, if the player is in a party.
    pub party: Option<String>,
    /// Team the player picked (`PickTeam`), if any.
    pub preferred: Option<u8>,
}

/// The order in which `joiners` join a fresh room of `teams` teams and the
/// team each asks for, so that parties play together where team balance
/// allows: the final team sizes differ by at most one (what
/// `Room::join` enforces), whole parties (largest first) go to one team
/// when it has room, solo players keep their picked team when it has room,
/// and the order is one in which every request is for a smallest team (so
/// the room grants all of them). Without teams (free-for-all) or parties,
/// the joiners keep their order and picks.
pub fn plan_joins(teams: u8, joiners: &[Joiner]) -> Vec<(u8, Option<u8>)> {
    let n = usize::from(teams);
    if n < 2 || !joiners.iter().any(|j| j.party.is_some()) {
        return joiners.iter().map(|j| (j.id, j.preferred)).collect();
    }
    let total = joiners.len();
    let cap = total.div_ceil(n);
    // Teams that may reach `cap` (the others end at `cap - 1`).
    let full_allowed = if total.is_multiple_of(n) {
        n
    } else {
        total % n
    };
    let fits = |size: &[usize], t: usize, g: usize| -> bool {
        let new = size[t] + g;
        if new > cap {
            return false;
        }
        if new == cap && size[t] < cap {
            let full = size.iter().filter(|&&s| s == cap).count();
            return full < full_allowed;
        }
        true
    };
    // Groups: parties (largest first, then by their first member), then
    // the solo players in their order.
    let mut parties: BTreeMap<&str, Vec<&Joiner>> = BTreeMap::new();
    let mut solos: Vec<&Joiner> = Vec::new();
    for j in joiners {
        match &j.party {
            Some(p) => parties.entry(p.as_str()).or_default().push(j),
            None => solos.push(j),
        }
    }
    let mut groups: Vec<Vec<&Joiner>> = parties.into_values().collect();
    groups.sort_by_key(|g| (std::cmp::Reverse(g.len()), g[0].id));
    let mut size = vec![0usize; n];
    let mut planned: Vec<Vec<u8>> = vec![Vec::new(); n];
    let smallest_fitting = |size: &[usize], g: usize| -> Option<usize> {
        (0..n)
            .filter(|&t| fits(size, t, g))
            .min_by_key(|&t| (size[t], t))
    };
    let mut place = |size: &mut Vec<usize>, t: usize, id: u8| {
        size[t] += 1;
        planned[t].push(id);
    };
    for g in &groups {
        if let Some(t) = smallest_fitting(&size, g.len()) {
            for j in g {
                place(&mut size, t, j.id);
            }
            continue;
        }
        // Too big for any team: split, adding each member to the fitting
        // team that already has most of the party (then the smallest), so
        // the parts stay as large as possible.
        let mut mates = vec![0usize; n];
        for j in g {
            let t = (0..n)
                .filter(|&t| fits(&size, t, 1))
                .min_by_key(|&t| (std::cmp::Reverse(mates[t]), size[t], t))
                .expect("capacity left for every joiner");
            mates[t] += 1;
            place(&mut size, t, j.id);
        }
    }
    for j in &solos {
        let t = j
            .preferred
            .map(usize::from)
            .filter(|&t| t < n && fits(&size, t, 1))
            .or_else(|| smallest_fitting(&size, 1))
            .expect("capacity left for every joiner");
        place(&mut size, t, j.id);
    }
    // Join order: always the team with the fewest joined so far among those
    // with members left (lowest index on a tie).
    let mut joined = vec![0usize; n];
    let mut out = Vec::with_capacity(total);
    while out.len() < total {
        let t = (0..n)
            .filter(|&t| joined[t] < planned[t].len())
            .min_by_key(|&t| (joined[t], t))
            .expect("members left");
        out.push((planned[t][joined[t]], Some(t as u8)));
        joined[t] += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn party_codes_round_trip_leniently() {
        let c = PartyCode::from_bits(0x1234_5678);
        assert!(c.as_str().starts_with("P-") && c.as_str().len() == 8);
        assert_eq!(PartyCode::parse(c.as_str()), Some(c.clone()));
        assert_eq!(PartyCode::parse(&c.as_str()[2..].to_lowercase()), Some(c));
        assert_eq!(
            PartyCode::parse(" p-oil 0ab "),
            Some(PartyCode("P-0110AB".into()))
        );
        assert_eq!(
            PartyCode::parse("R-123456"),
            None,
            "room codes are not party codes"
        );
        assert_eq!(PartyCode::parse("P-12345"), None);
    }

    #[test]
    fn parties_live_while_used_and_end_with_their_room() {
        let t0 = Instant::now();
        let parties = Parties::new(7);
        let room = RoomCode::from_bits(42);
        let a = parties.create(None, t0).unwrap();
        let b = parties.create(Some(room.clone()), t0).unwrap();
        assert_ne!(a, b);
        assert_eq!(parties.resolve(a.as_str(), t0).unwrap(), (a.clone(), None));
        let lower = b.as_str().to_lowercase();
        assert_eq!(
            parties.resolve(&lower, t0).unwrap(),
            (b.clone(), Some(room.clone()))
        );
        assert!(matches!(
            parties.resolve("P-ZZZZZZ", t0),
            Err(PartyError::Unknown(_))
        ));
        // A member keeps `a` alive past the idle time; it ends PARTY_IDLE
        // after the last member left.
        parties.member_joined(a.as_str());
        let later = t0 + PARTY_IDLE + Duration::from_secs(1);
        assert!(parties.resolve(a.as_str(), later).is_ok());
        assert!(
            parties.resolve(b.as_str(), later).is_err(),
            "never used: idle"
        );
        parties.member_left(a.as_str(), later);
        assert!(parties.resolve(a.as_str(), later + PARTY_IDLE / 2).is_ok());
        assert!(parties
            .resolve(a.as_str(), later + PARTY_IDLE + Duration::from_secs(1))
            .is_err());
        // Ending with the room.
        let c = parties.create(Some(room.clone()), t0).unwrap();
        parties.member_joined(c.as_str());
        parties.room_closed(&room);
        assert!(parties.get(&c).is_none());
    }

    #[test]
    fn the_party_cap_holds() {
        let t0 = Instant::now();
        let parties = Parties::new(1);
        for _ in 0..MAX_PARTIES {
            parties.create(None, t0).unwrap();
        }
        assert_eq!(parties.create(None, t0), Err(PartyError::Full));
        // Idle ones make room again.
        assert!(parties.create(None, t0 + PARTY_IDLE * 2).is_ok());
    }

    fn j(id: u8, party: Option<&str>, preferred: Option<u8>) -> Joiner {
        Joiner {
            id,
            party: party.map(String::from),
            preferred,
        }
    }

    /// Replays a plan against the rule of `Room::join` (a pick is granted
    /// only for a smallest team) and returns each id's team.
    fn replay(teams: u8, plan: &[(u8, Option<u8>)]) -> BTreeMap<u8, u8> {
        let mut size = vec![0usize; usize::from(teams)];
        let mut out = BTreeMap::new();
        for &(id, pick) in plan {
            let smallest = *size.iter().min().unwrap();
            let t = match pick {
                Some(t) if size[usize::from(t)] <= smallest => usize::from(t),
                _ => (0..size.len()).min_by_key(|&t| (size[t], t)).unwrap(),
            };
            size[t] += 1;
            out.insert(id, t as u8);
        }
        let (min, max) = (size.iter().min().unwrap(), size.iter().max().unwrap());
        assert!(max - min <= 1, "unbalanced: {size:?}");
        out
    }

    #[test]
    fn parties_stay_together_where_balance_allows() {
        // Two solos and a party of two on two teams.
        let joiners = [
            j(0, None, None),
            j(1, Some("P-A"), None),
            j(2, None, None),
            j(3, Some("P-A"), None),
        ];
        let teams = replay(2, &plan_joins(2, &joiners));
        assert_eq!(teams[&1], teams[&3]);
        assert_eq!(teams[&0], teams[&2], "the solos fill the other team");
        assert_ne!(teams[&0], teams[&1]);
        // Two parties of two: one per team.
        let joiners = [
            j(0, Some("P-A"), None),
            j(1, Some("P-B"), None),
            j(2, Some("P-A"), None),
            j(3, Some("P-B"), None),
        ];
        let teams = replay(2, &plan_joins(2, &joiners));
        assert_eq!(teams[&0], teams[&2]);
        assert_eq!(teams[&1], teams[&3]);
        assert_ne!(teams[&0], teams[&1]);
        // Three teams, a party of three among six players.
        let joiners: Vec<Joiner> = (0..6)
            .map(|i| j(i, (i % 2 == 0).then_some("P-C"), None))
            .collect();
        let teams = replay(3, &plan_joins(3, &joiners));
        // 6 players on 3 teams: 2 each, so a party of 3 must split 2 + 1.
        let mut party_teams: Vec<u8> = [0, 2, 4].iter().map(|i| teams[i]).collect();
        party_teams.sort_unstable();
        party_teams.dedup();
        assert_eq!(party_teams.len(), 2, "split as little as possible");
    }

    #[test]
    fn balance_wins_over_the_party_and_picks() {
        // Only two players, both in one party, two teams: 2 vs 0 is not
        // allowed, so the party is split.
        let joiners = [j(0, Some("P-A"), None), j(1, Some("P-A"), None)];
        let teams = replay(2, &plan_joins(2, &joiners));
        assert_ne!(teams[&0], teams[&1]);
        // Solo picks are kept where they fit.
        let joiners = [
            j(0, Some("P-A"), None),
            j(1, Some("P-A"), None),
            j(2, None, Some(1)),
            j(3, None, Some(1)),
            j(4, None, Some(0)),
        ];
        let plan = plan_joins(2, &joiners);
        let teams = replay(2, &plan);
        assert_eq!(teams[&0], teams[&1]);
        assert_eq!(plan.len(), 5);
        // Free-for-all and party-less rooms keep the order and picks.
        let joiners = [j(5, None, Some(1)), j(2, Some("P-A"), None)];
        assert_eq!(plan_joins(0, &joiners), vec![(5, Some(1)), (2, None)]);
        let joiners = [j(5, None, Some(1)), j(2, None, None)];
        assert_eq!(plan_joins(2, &joiners), vec![(5, Some(1)), (2, None)]);
    }

    #[test]
    fn every_plan_is_granted_by_the_room_rule() {
        // Exhaustive-ish: 1-9 players, 2-4 teams, mixed parties and picks.
        for teams in 2..=4u8 {
            for total in 1..=9u8 {
                for pattern in 0..16u8 {
                    let joiners: Vec<Joiner> = (0..total)
                        .map(|i| {
                            let party = match (i + pattern) % 4 {
                                0 => Some("P-A"),
                                1 if pattern & 1 == 1 => Some("P-B"),
                                _ => None,
                            };
                            let pick = (pattern & 2 == 2).then_some(i % teams);
                            j(i, party, pick)
                        })
                        .collect();
                    let plan = plan_joins(teams, &joiners);
                    assert_eq!(plan.len(), joiners.len());
                    let got = replay(teams, &plan);
                    // Every pick in the plan was granted.
                    for (id, pick) in &plan {
                        if joiners.iter().any(|j| j.party.is_some()) {
                            assert_eq!(Some(got[id]), *pick, "{teams} {total} {pattern}");
                        }
                    }
                }
            }
        }
    }
}
