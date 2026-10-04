//! In-memory directory state: server entries with a TTL, relayed invites and
//! a per-IP rate limiter. No database; a restart forgets everything and the
//! servers re-register with their next heartbeat.
//!
//! Every method takes `now` so tests drive time explicitly.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::api::{
    Heartbeat, HeartbeatReply, Invalid, Invite, InviteReply, InviteRequest, ServerEntry, Unregister,
};
use crate::random_hex;

/// Limits and timings of a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// An entry expires this long after its last heartbeat.
    pub ttl: Duration,
    /// Heartbeat interval suggested to servers.
    pub heartbeat: Duration,
    /// Most server entries at once.
    pub max_servers: usize,
    /// Invites live this long.
    pub invite_ttl: Duration,
    /// Most pending invites per recipient (the oldest is dropped).
    pub max_invites_per_player: usize,
    /// Most pending invites at once.
    pub max_invites: usize,
    /// Requests per second per client IP (sustained).
    pub rate_per_sec: u32,
    /// Burst size of the per-IP rate limit.
    pub rate_burst: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ttl: Duration::from_secs(30),
            heartbeat: Duration::from_secs(10),
            max_servers: 1000,
            invite_ttl: Duration::from_secs(30),
            max_invites_per_player: 8,
            max_invites: 10_000,
            rate_per_sec: 10,
            rate_burst: 40,
        }
    }
}

/// Why a request was refused (maps to an HTTP status in [`crate::http`]).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("invalid request: {0}")]
    Invalid(#[from] Invalid),
    #[error("this address is registered with a different secret")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("directory is full")]
    Full,
}

#[derive(Debug)]
struct Entry {
    id: String,
    secret: String,
    last_seen: Instant,
    heartbeat: Heartbeat,
}

#[derive(Debug)]
struct PendingInvite {
    id: String,
    expires: Instant,
    invite: InviteRequest,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// The directory's state.
#[derive(Debug)]
pub struct Registry {
    cfg: Config,
    /// By public address (one entry per game server).
    servers: BTreeMap<String, Entry>,
    next_id: u64,
    /// By recipient player id, oldest first.
    invites: BTreeMap<String, Vec<PendingInvite>>,
    invite_count: usize,
    buckets: HashMap<IpAddr, Bucket>,
}

impl Registry {
    pub fn new(cfg: Config) -> Self {
        Registry {
            cfg,
            servers: BTreeMap::new(),
            next_id: 1,
            invites: BTreeMap::new(),
            invite_count: 0,
            buckets: HashMap::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Registers or refreshes a server.
    pub fn heartbeat(&mut self, hb: Heartbeat, now: Instant) -> Result<HeartbeatReply, Refused> {
        hb.validate()?;
        self.expire(now);
        let reply = |id: &str, cfg: &Config| HeartbeatReply {
            id: id.to_string(),
            ttl_secs: cfg.ttl.as_secs(),
            heartbeat_secs: cfg.heartbeat.as_secs(),
        };
        if let Some(e) = self.servers.get_mut(&hb.server.address) {
            if e.secret != hb.secret {
                return Err(Refused::Forbidden);
            }
            e.last_seen = now;
            e.heartbeat = hb;
            return Ok(reply(&e.id, &self.cfg));
        }
        if self.servers.len() >= self.cfg.max_servers {
            return Err(Refused::Full);
        }
        let id = format!("s{}", self.next_id);
        self.next_id += 1;
        let r = reply(&id, &self.cfg);
        self.servers.insert(
            hb.server.address.clone(),
            Entry {
                id,
                secret: hb.secret.clone(),
                last_seen: now,
                heartbeat: hb,
            },
        );
        Ok(r)
    }

    /// Removes a server's entry (it is shutting down).
    pub fn unregister(&mut self, u: &Unregister, now: Instant) -> Result<(), Refused> {
        self.expire(now);
        match self.servers.get(&u.address) {
            None => Err(Refused::NotFound),
            Some(e) if e.secret != u.secret => Err(Refused::Forbidden),
            Some(_) => {
                self.servers.remove(&u.address);
                Ok(())
            }
        }
    }

    /// Live servers, sorted by address, optionally only those with the given
    /// protocol version and data hash (a client lists only servers its build
    /// can join).
    pub fn list(
        &mut self,
        protocol: Option<u16>,
        data_hash: Option<&str>,
        now: Instant,
    ) -> Vec<ServerEntry> {
        self.expire(now);
        self.servers
            .values()
            .filter(|e| protocol.is_none_or(|p| e.heartbeat.server.protocol == p))
            .filter(|e| data_hash.is_none_or(|h| e.heartbeat.server.data_hash == h))
            .map(|e| ServerEntry {
                id: e.id.clone(),
                age_ms: now.saturating_duration_since(e.last_seen).as_millis() as u64,
                server: e.heartbeat.server.clone(),
            })
            .collect()
    }

    /// Queues an invite for its recipient.
    pub fn invite(&mut self, inv: InviteRequest, now: Instant) -> Result<InviteReply, Refused> {
        inv.validate()?;
        self.expire(now);
        let per_player = self.cfg.max_invites_per_player;
        let full = self.invite_count >= self.cfg.max_invites;
        let queue = self.invites.entry(inv.to.clone()).or_default();
        if queue.len() >= per_player {
            queue.remove(0);
            self.invite_count -= 1;
        } else if full {
            if queue.is_empty() {
                self.invites.remove(&inv.to);
            }
            return Err(Refused::Full);
        }
        let id = random_hex(8);
        queue.push(PendingInvite {
            id: id.clone(),
            expires: now + self.cfg.invite_ttl,
            invite: inv,
        });
        self.invite_count += 1;
        Ok(InviteReply {
            id,
            expires_in_secs: self.cfg.invite_ttl.as_secs(),
        })
    }

    /// Pending invites for a player id (they stay until they expire; clients
    /// de-duplicate by id).
    pub fn invites_for(&mut self, player: &str, now: Instant) -> Vec<Invite> {
        self.expire(now);
        self.invites
            .get(player)
            .map(|q| {
                q.iter()
                    .map(|p| Invite {
                        id: p.id.clone(),
                        expires_in_ms: p.expires.saturating_duration_since(now).as_millis() as u64,
                        invite: p.invite.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drops expired servers, invites and idle rate buckets.
    pub fn expire(&mut self, now: Instant) {
        let ttl = self.cfg.ttl;
        self.servers
            .retain(|_, e| now.saturating_duration_since(e.last_seen) < ttl);
        let mut count = 0;
        self.invites.retain(|_, q| {
            q.retain(|p| p.expires > now);
            count += q.len();
            !q.is_empty()
        });
        self.invite_count = count;
        let refill = self.refill_time();
        self.buckets
            .retain(|_, b| now.saturating_duration_since(b.last) < refill);
    }

    /// Time an empty bucket needs to fill up again.
    fn refill_time(&self) -> Duration {
        Duration::from_secs_f64(
            f64::from(self.cfg.rate_burst) / f64::from(self.cfg.rate_per_sec.max(1)),
        )
    }

    /// Takes one request from `ip`'s token bucket; `false` = rate limited.
    pub fn allow(&mut self, ip: IpAddr, now: Instant) -> bool {
        let burst = f64::from(self.cfg.rate_burst.max(1));
        let rate = f64::from(self.cfg.rate_per_sec);
        let b = self.buckets.entry(ip).or_insert(Bucket {
            tokens: burst,
            last: now,
        });
        let dt = now.saturating_duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + dt * rate).min(burst);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    pub fn server_count(&self) -> usize {
        self.servers.len()
    }

    pub fn invite_count(&self) -> usize {
        self.invite_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::info;

    fn hb(address: &str, secret: &str) -> Heartbeat {
        Heartbeat {
            secret: secret.into(),
            server: info(address),
        }
    }

    const SECRET: &str = "0123456789abcdef0123";

    fn invite(to: &str) -> InviteRequest {
        InviteRequest {
            to: to.into(),
            kind: "duel".into(),
            from_name: "Ace".into(),
            from_id: "player-0001".into(),
            address: "127.0.0.1:7777".into(),
            room: None,
            world_code: None,
            rules_code: None,
        }
    }

    #[test]
    fn register_refresh_and_expire() {
        let t0 = Instant::now();
        let mut r = Registry::new(Config::default());
        let a = r.heartbeat(hb("a:1", SECRET), t0).unwrap();
        assert_eq!((a.ttl_secs, a.heartbeat_secs), (30, 10));
        let b = r.heartbeat(hb("b:1", SECRET), t0).unwrap();
        assert_ne!(a.id, b.id);
        // Refresh `a` at 20 s: same id, `b` expires at 30 s, `a` at 50 s.
        let mut again = hb("a:1", SECRET);
        again.server.players = 7;
        assert_eq!(
            r.heartbeat(again, t0 + Duration::from_secs(20)).unwrap().id,
            a.id
        );
        let l = r.list(None, None, t0 + Duration::from_secs(29));
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].server.players, 7);
        assert_eq!(l[0].age_ms, 9000);
        let l = r.list(None, None, t0 + Duration::from_secs(30));
        assert_eq!(
            l.iter()
                .map(|e| e.server.address.as_str())
                .collect::<Vec<_>>(),
            ["a:1"]
        );
        assert!(r.list(None, None, t0 + Duration::from_secs(50)).is_empty());
        // After expiry the address can be claimed with a new secret.
        r.heartbeat(
            hb("a:1", "another-secret-0000"),
            t0 + Duration::from_secs(51),
        )
        .unwrap();
    }

    #[test]
    fn secret_protects_an_entry() {
        let t0 = Instant::now();
        let mut r = Registry::new(Config::default());
        r.heartbeat(hb("a:1", SECRET), t0).unwrap();
        assert_eq!(
            r.heartbeat(hb("a:1", "wrong-secret-00000"), t0),
            Err(Refused::Forbidden)
        );
        let bad = Unregister {
            address: "a:1".into(),
            secret: "wrong-secret-00000".into(),
        };
        assert_eq!(r.unregister(&bad, t0), Err(Refused::Forbidden));
        let good = Unregister {
            address: "a:1".into(),
            secret: SECRET.into(),
        };
        r.unregister(&good, t0).unwrap();
        assert_eq!(r.unregister(&good, t0), Err(Refused::NotFound));
        assert_eq!(r.server_count(), 0);
    }

    #[test]
    fn list_filters_by_protocol_and_data_hash() {
        let t0 = Instant::now();
        let mut r = Registry::new(Config::default());
        r.heartbeat(hb("a:1", SECRET), t0).unwrap();
        let mut old = hb("b:1", SECRET);
        old.server.protocol = 4;
        r.heartbeat(old, t0).unwrap();
        let mut other = hb("c:1", SECRET);
        other.server.data_hash = "ffffffffffffffff".into();
        r.heartbeat(other, t0).unwrap();
        assert_eq!(r.list(None, None, t0).len(), 3);
        let addrs =
            |l: Vec<ServerEntry>| l.into_iter().map(|e| e.server.address).collect::<Vec<_>>();
        assert_eq!(addrs(r.list(Some(5), None, t0)), ["a:1", "c:1"]);
        assert_eq!(
            addrs(r.list(Some(5), Some("0123456789abcdef"), t0)),
            ["a:1"]
        );
    }

    #[test]
    fn max_servers_is_enforced() {
        let t0 = Instant::now();
        let mut r = Registry::new(Config {
            max_servers: 2,
            ..Config::default()
        });
        r.heartbeat(hb("a:1", SECRET), t0).unwrap();
        r.heartbeat(hb("b:1", SECRET), t0).unwrap();
        assert_eq!(r.heartbeat(hb("c:1", SECRET), t0), Err(Refused::Full));
        // Existing entries still refresh.
        r.heartbeat(hb("a:1", SECRET), t0).unwrap();
    }

    #[test]
    fn invites_relay_expire_and_cap() {
        let t0 = Instant::now();
        let mut r = Registry::new(Config {
            max_invites_per_player: 2,
            max_invites: 3,
            ..Config::default()
        });
        let first = r.invite(invite("player-0002"), t0).unwrap();
        assert_eq!(first.expires_in_secs, 30);
        let l = r.invites_for("player-0002", t0 + Duration::from_secs(1));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, first.id);
        assert_eq!(l[0].expires_in_ms, 29_000);
        assert_eq!(l[0].invite.kind, "duel");
        // Reading does not consume.
        assert_eq!(r.invites_for("player-0002", t0).len(), 1);
        assert!(r.invites_for("player-0003", t0).is_empty());
        // Per-player cap drops the oldest.
        r.invite(invite("player-0002"), t0).unwrap();
        r.invite(invite("player-0002"), t0).unwrap();
        let l = r.invites_for("player-0002", t0);
        assert_eq!(l.len(), 2);
        assert!(l.iter().all(|i| i.id != first.id));
        // Global cap.
        r.invite(invite("player-0003"), t0).unwrap();
        assert_eq!(r.invite(invite("player-0004"), t0), Err(Refused::Full));
        assert_eq!(r.invite_count(), 3);
        // Expiry.
        assert!(r
            .invites_for("player-0002", t0 + Duration::from_secs(30))
            .is_empty());
        assert_eq!(r.invite_count(), 0);
        let mut bad = invite("x");
        bad.to = "x".into();
        assert!(matches!(r.invite(bad, t0), Err(Refused::Invalid(_))));
    }

    #[test]
    fn rate_limit_per_ip() {
        let t0 = Instant::now();
        let mut r = Registry::new(Config {
            rate_per_sec: 2,
            rate_burst: 3,
            ..Config::default()
        });
        let a: IpAddr = "127.0.0.1".parse().unwrap();
        let b: IpAddr = "127.0.0.2".parse().unwrap();
        assert!((0..3).all(|_| r.allow(a, t0)));
        assert!(!r.allow(a, t0));
        assert!(r.allow(b, t0), "other IPs have their own bucket");
        assert!(r.allow(a, t0 + Duration::from_millis(500)));
        assert!(!r.allow(a, t0 + Duration::from_millis(500)));
    }
}
