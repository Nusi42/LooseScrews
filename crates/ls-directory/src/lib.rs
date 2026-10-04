//! `ls-directory`: the optional server directory behind the browser's server
//! list (milestone 3, `docs/milestone-3-plan.md` §3.1).
//!
//! Game servers that run with `--directory <url>` send a heartbeat every
//! 10 s ([`api::Heartbeat`]); entries expire 30 s after the last one. The
//! browser reads `GET /servers` (CORS open), filtered to its own protocol
//! version and data hash, and pings the listed servers itself. The service
//! also relays short-lived invites (duel challenges across servers) by public
//! player id. Everything is in memory; there is no database and no account.
//! A game server works without a directory (direct link or address).
//!
//! The crate is a binary (`ls-directory`, see `src/main.rs`) plus a library:
//! [`api`] (JSON types), [`registry`] (state, TTL, limits), [`http`] (the
//! service) and [`client`] (what `ls-server` uses). It does not depend on the
//! game crates: one directory can list servers of several builds.

pub mod api;
pub mod client;
pub mod http;
pub mod registry;

pub use client::{spawn_heartbeats, DirectoryClient};
pub use http::DirectoryServer;
pub use registry::{Config, Registry};

/// Default listen address of the binary (hosting: every interface).
pub const DEFAULT_BIND: &str = "0.0.0.0:7780";

/// `2 * bytes` lower-case hex digits from the OS random source (falls back to
/// a randomly keyed hash if the OS source fails). Not used by any
/// deterministic code.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if getrandom::fill(&mut buf).is_err() {
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let state = std::collections::hash_map::RandomState::new();
        for (i, chunk) in buf.chunks_mut(8).enumerate() {
            let mut h = state.build_hasher();
            h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
            h.write_usize(i);
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            let v = h.finish().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// A new per-server secret for heartbeats (32 hex digits).
pub fn new_secret() -> String {
    random_hex(16)
}

#[cfg(test)]
mod tests {
    #[test]
    fn secrets_are_hex_and_differ() {
        let a = super::new_secret();
        let b = super::new_secret();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
