//! The room registry: one public room (the playlist rotation) and up to
//! `--private-rooms N` private rooms (duels, parties, custom matches), each
//! on its own sim thread ([`crate::sim`]).
//!
//! Private rooms are addressed by a [`RoomCode`] (`R-XXXXXX`, 6 Crockford
//! base32 characters). A connection's `Hello` names the room it wants
//! ([`requested_room`]); without one it joins the public room. A private
//! room closes once its last player has left (or if nobody joined within
//! [`PRIVATE_ROOM_IDLE`] of its creation); its code is then unknown again.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use ls_net::Hello;

use crate::sim::SimCommand;
use crate::stats::{SharedStats, StatsSnapshot};

/// A private room nobody joined closes after this long.
pub const PRIVATE_ROOM_IDLE: Duration = Duration::from_secs(60);
/// Default `--private-rooms`.
pub const DEFAULT_PRIVATE_ROOMS: usize = 4;
/// Label of the public room in logs and status.
pub const PUBLIC_ROOM: &str = "public";

/// Crockford base32 alphabet (no I, L, O, U).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_LEN: usize = 6;

/// The code of a private room, `R-XXXXXX`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomCode(String);

impl RoomCode {
    /// The code for 30 random bits.
    pub fn from_bits(bits: u64) -> RoomCode {
        let mut s = String::from("R-");
        for i in (0..CODE_LEN).rev() {
            s.push(CROCKFORD[((bits >> (5 * i)) & 31) as usize] as char);
        }
        RoomCode(s)
    }

    /// Parses a code leniently: case, a missing `R-` prefix, spaces and the
    /// Crockford look-alikes (O for 0, I and L for 1) are accepted.
    pub fn parse(text: &str) -> Option<RoomCode> {
        let t = text.trim().to_ascii_uppercase();
        let body = t.strip_prefix("R-").unwrap_or(&t);
        let mut s = String::from("R-");
        for c in body.chars().filter(|c| *c != '-' && *c != ' ') {
            let c = match c {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            };
            if !CROCKFORD.contains(&(c as u8)) || !c.is_ascii() {
                return None;
            }
            s.push(c);
        }
        (s.len() == 2 + CODE_LEN).then_some(RoomCode(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RoomCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The room a `Hello` asks for (`None`: the public room).
///
/// Protocol 4 has no room field yet; protocol 5 (wave A, package A2) adds
/// an optional room code to `Hello`, read here.
pub fn requested_room(_hello: &Hello) -> Option<String> {
    None
}

/// What a room is doing, for status queries (updated by its sim thread).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomStatus {
    /// [`PUBLIC_ROOM`] or the room code.
    pub label: String,
    /// Lobby phase (`warmup`, `countdown`, `match`, `results`).
    pub phase: &'static str,
    /// Playlist entry being warmed up or played (rule set id, file or code).
    pub entry: String,
    /// World of the current or upcoming match.
    pub share_code: String,
    /// Rule set name and code of the current or upcoming match.
    pub ruleset: String,
    pub rules_code: String,
    pub players: usize,
    pub max_players: u8,
    /// Set when a private room has closed.
    pub closed: bool,
}

/// Shared, updatable room status.
pub type SharedStatus = Arc<RwLock<RoomStatus>>;

/// One running room: its sim thread and how to reach it.
#[derive(Debug)]
pub(crate) struct RoomEntry {
    pub commands: mpsc::Sender<SimCommand>,
    pub stats: Arc<SharedStats>,
    pub status: SharedStatus,
    pub stop: Arc<AtomicBool>,
    pub thread: Mutex<Option<JoinHandle<()>>>,
}

impl RoomEntry {
    fn closed(&self) -> bool {
        self.status.read().map_or(true, |s| s.closed)
    }

    /// Stops the sim thread and returns its handle.
    pub fn stop(&self) -> Option<JoinHandle<()>> {
        self.stop.store(true, Ordering::Release);
        self.thread.lock().ok().and_then(|mut t| t.take())
    }
}

/// Why a join could not be routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// No open room has this code.
    UnknownRoom(String),
    /// The room's sim thread has stopped.
    Gone,
}

/// The registry; shared by the [`crate::Hub`] (routing) and the
/// [`crate::ServerHandle`] (creation, status, shutdown).
#[derive(Debug)]
pub struct Rooms {
    public: RoomEntry,
    private: Mutex<BTreeMap<RoomCode, RoomEntry>>,
    max_private: usize,
}

impl Rooms {
    pub(crate) fn new(public: RoomEntry, max_private: usize) -> Self {
        Rooms {
            public,
            private: Mutex::new(BTreeMap::new()),
            max_private,
        }
    }

    /// A registry with only a public room reachable through `commands` (for
    /// endpoint tests).
    #[cfg(test)]
    pub(crate) fn public_only(commands: mpsc::Sender<SimCommand>, stats: Arc<SharedStats>) -> Self {
        Rooms::new(
            RoomEntry {
                commands,
                stats,
                status: SharedStatus::default(),
                stop: Arc::new(AtomicBool::new(false)),
                thread: Mutex::new(None),
            },
            0,
        )
    }

    /// Most private rooms at once.
    pub fn max_private(&self) -> usize {
        self.max_private
    }

    /// Sends a command to the room `code` names (the public room for
    /// `None`).
    pub(crate) fn send(&self, code: Option<&str>, cmd: SimCommand) -> Result<(), RouteError> {
        let Some(text) = code else {
            return self.public.commands.send(cmd).map_err(|_| RouteError::Gone);
        };
        let unknown = || RouteError::UnknownRoom(text.to_string());
        let code = RoomCode::parse(text).ok_or_else(unknown)?;
        self.prune();
        let rooms = self.private.lock().map_err(|_| RouteError::Gone)?;
        let room = rooms
            .get(&code)
            .filter(|r| !r.closed())
            .ok_or_else(unknown)?;
        room.commands.send(cmd).map_err(|_| RouteError::Gone)
    }

    /// Removes closed private rooms (their threads have ended).
    pub(crate) fn prune(&self) {
        let closed: Vec<RoomEntry> = {
            let Ok(mut rooms) = self.private.lock() else {
                return;
            };
            let codes: Vec<RoomCode> = rooms
                .iter()
                .filter(|(_, r)| r.closed())
                .map(|(c, _)| c.clone())
                .collect();
            codes.iter().filter_map(|c| rooms.remove(c)).collect()
        };
        for r in closed {
            if let Some(t) = r.stop() {
                let _ = t.join();
            }
        }
    }

    /// Open private rooms now.
    pub fn private_count(&self) -> usize {
        self.prune();
        self.private.lock().map_or(0, |r| r.len())
    }

    /// Registers a private room unless the cap is reached. `make` picks a
    /// free code from candidate codes and starts the room.
    pub(crate) fn insert_private(
        &self,
        mut next_code: impl FnMut() -> RoomCode,
        start: impl FnOnce(&RoomCode) -> std::io::Result<RoomEntry>,
    ) -> Result<RoomCode, crate::ServerError> {
        self.prune();
        let mut rooms = self
            .private
            .lock()
            .map_err(|_| crate::ServerError::Rooms("registry poisoned".into()))?;
        if rooms.len() >= self.max_private {
            return Err(crate::ServerError::Rooms(format!(
                "all {} private rooms are in use",
                self.max_private
            )));
        }
        let code = loop {
            let c = next_code();
            if !rooms.contains_key(&c) {
                break c;
            }
        };
        let entry = start(&code)?;
        rooms.insert(code.clone(), entry);
        Ok(code)
    }

    /// Removes a private room (a room that failed to start).
    pub(crate) fn remove_private(&self, code: &RoomCode) -> Option<JoinHandle<()>> {
        let entry = self.private.lock().ok()?.remove(code)?;
        entry.stop()
    }

    /// Status of every room, the public one first.
    pub fn statuses(&self) -> Vec<(RoomStatus, StatsSnapshot)> {
        self.prune();
        let snap = |r: &RoomEntry| {
            (
                r.status.read().map(|s| s.clone()).unwrap_or_default(),
                r.stats.snapshot(),
            )
        };
        let mut out = vec![snap(&self.public)];
        if let Ok(rooms) = self.private.lock() {
            out.extend(rooms.values().map(snap));
        }
        out
    }

    /// Players connected over all rooms.
    pub fn total_players(&self) -> u64 {
        let private: u64 = self
            .private
            .lock()
            .map_or(0, |r| r.values().map(|e| e.stats.snapshot().players).sum());
        self.public.stats.snapshot().players + private
    }

    /// Stops every room's sim thread and returns the handles to join.
    pub(crate) fn stop_all(&self) -> Vec<JoinHandle<()>> {
        let mut out: Vec<JoinHandle<()>> = self.public.stop().into_iter().collect();
        if let Ok(mut rooms) = self.private.lock() {
            out.extend(
                std::mem::take(&mut *rooms)
                    .values()
                    .filter_map(|r| r.stop()),
            );
        }
        out
    }

    /// Signals every sim thread to stop (without waiting).
    pub(crate) fn signal_stop(&self) {
        self.public.stop.store(true, Ordering::Release);
        if let Ok(rooms) = self.private.lock() {
            for r in rooms.values() {
                r.stop.store(true, Ordering::Release);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_codes_round_trip_leniently() {
        let c = RoomCode::from_bits(0x3fff_ffff);
        assert_eq!(c.as_str(), "R-ZZZZZZ");
        let c = RoomCode::from_bits(0);
        assert_eq!(c.as_str(), "R-000000");
        let c = RoomCode::from_bits(0x1234_5678);
        assert_eq!(RoomCode::parse(c.as_str()), Some(c.clone()));
        let body = &c.as_str()[2..];
        assert_eq!(RoomCode::parse(&body.to_ascii_lowercase()), Some(c.clone()));
        assert_eq!(
            RoomCode::parse("r-oil-0ab"),
            Some(RoomCode("R-0110AB".into()))
        );
        for bad in [
            "",
            "R-",
            "R-12345",
            "R-1234567",
            "R-12345U",
            "R-1234!5",
            "R-12345é",
        ] {
            assert_eq!(RoomCode::parse(bad), None, "{bad}");
        }
    }
}
