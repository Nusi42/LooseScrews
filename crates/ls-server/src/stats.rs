//! Server-wide counters shared between the sim thread, the endpoints and the
//! [`crate::ServerHandle`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::link::LinkCounters;

/// Live counters (atomics, updated by the sim thread and endpoints).
#[derive(Debug, Default)]
pub struct SharedStats {
    /// Tick of the current match.
    pub tick: AtomicU64,
    /// Ticks simulated over all matches.
    pub total_ticks: AtomicU64,
    pub players: AtomicU64,
    /// Terrain edits of the current match.
    pub edit_count: AtomicU64,
    /// Terrain edits over all matches.
    pub total_edits: AtomicU64,
    /// Connections that completed the transport handshake.
    pub connections: AtomicU64,
    /// Handshakes rejected (bad version, full room, ...).
    pub rejected: AtomicU64,
    /// Clients removed for protocol errors or slow sockets.
    pub kicked: AtomicU64,
    /// Clients that timed out (handshake or idle).
    pub timeouts: AtomicU64,
    pub skipped_ticks: AtomicU64,
    pub last_p99_us: AtomicU64,
    pub last_max_us: AtomicU64,
    pub late_inputs: AtomicU64,
    pub missing_inputs: AtomicU64,
    pub snapshots_sent: AtomicU64,
    pub snapshot_bytes: AtomicU64,
    /// Reporting windows in which clients starved for input (the server
    /// repeated their commands, so their prediction diverges).
    pub desync_warnings: AtomicU64,
    /// Matches started (1 + rebuilds after a finished match).
    pub matches: AtomicU64,
    pub links: Arc<LinkCounters>,
}

/// A plain copy of [`SharedStats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatsSnapshot {
    pub tick: u64,
    pub total_ticks: u64,
    pub players: u64,
    pub edit_count: u64,
    pub total_edits: u64,
    pub connections: u64,
    pub rejected: u64,
    pub kicked: u64,
    pub timeouts: u64,
    pub skipped_ticks: u64,
    pub last_p99_us: u64,
    pub last_max_us: u64,
    pub late_inputs: u64,
    pub missing_inputs: u64,
    pub snapshots_sent: u64,
    pub snapshot_bytes: u64,
    pub desync_warnings: u64,
    pub matches: u64,
    pub dropped_unreliable: u64,
}

impl SharedStats {
    pub fn snapshot(&self) -> StatsSnapshot {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        StatsSnapshot {
            tick: g(&self.tick),
            total_ticks: g(&self.total_ticks),
            players: g(&self.players),
            edit_count: g(&self.edit_count),
            total_edits: g(&self.total_edits),
            connections: g(&self.connections),
            rejected: g(&self.rejected),
            kicked: g(&self.kicked),
            timeouts: g(&self.timeouts),
            skipped_ticks: g(&self.skipped_ticks),
            last_p99_us: g(&self.last_p99_us),
            last_max_us: g(&self.last_max_us),
            late_inputs: g(&self.late_inputs),
            missing_inputs: g(&self.missing_inputs),
            snapshots_sent: g(&self.snapshots_sent),
            snapshot_bytes: g(&self.snapshot_bytes),
            desync_warnings: g(&self.desync_warnings),
            matches: g(&self.matches),
            dropped_unreliable: g(&self.links.dropped_unreliable),
        }
    }
}

pub(crate) fn set(a: &AtomicU64, v: u64) {
    a.store(v, Ordering::Relaxed);
}

pub(crate) fn inc(a: &AtomicU64) {
    a.fetch_add(1, Ordering::Relaxed);
}
