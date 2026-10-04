//! Per-bot counters and the summary printed at exit.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::brain::BrainCounters;

/// Use of one weapon by one bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WeaponUse {
    /// Shots seen: new own projectiles of the weapon in a snapshot (one per
    /// weapon per snapshot, so a shotgun blast or a volley counts once and
    /// weapons firing faster than the 30 Hz snapshots are under-counted),
    /// Leech Arc arcs (each beam cue covers `value` ticks of arcs) and
    /// Railspike muzzle flashes. Hits count every damaging hit, so hits can
    /// exceed shots for pellets, fans and splash.
    pub fired: u64,
    /// Hits on other ships (`Hit` events with this bot as attacker).
    pub hits: u64,
    pub kills: u32,
}

/// Everything one bot counted over its whole run (all connections).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BotStats {
    pub name: String,
    /// Successful handshakes (1 + reconnects).
    pub joins: u32,
    /// Connection attempts that failed before a `Welcome` (refused, timed
    /// out, rejected as full).
    pub connect_failures: u32,
    /// Sessions that ended while the bot still wanted to play (socket
    /// closed, server `Disconnect`, error, write timeout, silence).
    pub disconnects: u32,
    /// Matches the bot moved on from: a new `Welcome` on the open
    /// connection (handled in place), or a `Disconnect` with
    /// `ls_game::MATCH_RESTART_REASON` from older hosts (the bot rejoins).
    /// Not a failure.
    pub match_restarts: u32,
    pub inputs_sent: u64,
    pub pings_sent: u64,
    pub pongs: u64,
    pub snapshots: u64,
    pub stale_snapshots: u64,
    pub full_snapshots: u64,
    pub delta_snapshots: u64,
    pub event_messages: u64,
    pub edits_applied: u64,
    /// Snapshots whose `ack_input` showed the server ran out of our input.
    pub starved_snapshots: u64,

    pub accepted: u64,
    pub corrections: u64,
    pub snaps: u64,
    /// Snaps explained by lag (starvation) rather than a desync.
    pub lag_snaps: u64,
    /// Corrections explained by an own teleport cue (a Swap Harpoon swap or
    /// Borrowed Time rewind shorter than the snap distance); they do not
    /// count toward `max_reconcile_error`.
    pub teleport_corrections: u64,
    /// Largest position error (u) of a correction or unexplained snap.
    pub max_reconcile_error: f32,

    pub kills: u32,
    pub deaths: u32,
    pub hits_dealt: u32,
    pub hits_taken: u32,
    pub matches_over: u32,

    /// Undecodable or unexpected packets, rejected handshakes (other than a
    /// full server), invalid `Welcome` data.
    pub protocol_errors: u32,
    /// Terrain edit gaps and prediction snaps that neither a respawn, a
    /// death nor starvation explains.
    pub desyncs: u32,
    /// Human-readable descriptions of the first few errors.
    pub errors: Vec<String>,

    pub behavior: BrainCounters,
    /// Per weapon (data id): shots and hits.
    pub weapons: BTreeMap<String, WeaponUse>,
    /// Lives per ship type (name).
    pub ship_lives: BTreeMap<String, u32>,
}

impl BotStats {
    pub fn new(name: impl Into<String>) -> BotStats {
        BotStats {
            name: name.into(),
            ..BotStats::default()
        }
    }

    /// Records an error description (keeps the first eight).
    pub fn note_error(&mut self, msg: impl Into<String>) {
        if self.errors.len() < 8 {
            self.errors.push(msg.into());
        }
    }

    /// The counters of weapon `id` (data id).
    pub fn weapon_mut(&mut self, id: &str) -> &mut WeaponUse {
        self.weapons.entry(id.to_string()).or_default()
    }

    /// Weapons this bot fired at least once.
    pub fn weapons_fired(&self) -> impl Iterator<Item = &str> {
        self.weapons
            .iter()
            .filter(|(_, u)| u.fired > 0)
            .map(|(k, _)| k.as_str())
    }

    /// True if this bot saw a protocol error or a desync.
    pub fn failed(&self) -> bool {
        self.protocol_errors > 0 || self.desyncs > 0
    }
}

/// How [`exit_code`] treats lost connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisconnectPolicy {
    /// A session the bot did not ask to end fails the run (soak tests: a
    /// server crash or kick must not pass).
    #[default]
    Fail,
    /// Lost sessions are only counted (load tests that restart the server).
    Allow,
}

/// Exit status for a finished run: 0 all good, 1 protocol error or desync,
/// 2 a bot never managed to join, 3 a bot lost its connection (unless
/// `policy` allows that). Match restarts are not failures.
pub fn exit_code(stats: &[BotStats], policy: DisconnectPolicy) -> i32 {
    if stats.iter().any(BotStats::failed) {
        1
    } else if stats.iter().any(|s| s.joins == 0) {
        2
    } else if policy == DisconnectPolicy::Fail && stats.iter().any(|s| s.disconnects > 0) {
        3
    } else {
        0
    }
}

/// Formats the per-bot table and totals.
pub fn summary(stats: &[BotStats], elapsed_secs: f64, policy: DisconnectPolicy) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "ls-bots summary after {elapsed_secs:.1} s");
    let _ = writeln!(
        out,
        "{:<10} {:>5} {:>5} {:>8} {:>7} {:>6} {:>6} {:>7} {:>6} {:>5} {:>5} {:>6} {:>6} {:>5} {:>5}",
        "bot",
        "joins",
        "disc",
        "inputs",
        "snaps",
        "starv",
        "corr",
        "maxerr",
        "k/d",
        "hits",
        "paint",
        "shots",
        "scav",
        "proto",
        "desync"
    );
    let mut total = BotStats::new("total");
    for s in stats {
        let shots: u64 = s.weapons.values().map(|u| u.fired).sum();
        let _ = writeln!(
            out,
            "{:<10} {:>5} {:>5} {:>8} {:>7} {:>6} {:>6} {:>7.3} {:>6} {:>5} {:>5} {:>6} {:>6} {:>5} {:>5}",
            truncate(&s.name, 10),
            s.joins,
            s.disconnects,
            s.inputs_sent,
            s.snapshots,
            s.starved_snapshots,
            s.corrections,
            s.max_reconcile_error,
            format!("{}/{}", s.kills, s.deaths),
            s.hits_dealt,
            s.behavior.paint_triggers,
            shots,
            s.behavior.scavenge_holds,
            s.protocol_errors,
            s.desyncs
        );
        total.joins += s.joins;
        total.disconnects += s.disconnects;
        total.match_restarts += s.match_restarts;
        total.connect_failures += s.connect_failures;
        total.inputs_sent += s.inputs_sent;
        total.snapshots += s.snapshots;
        total.stale_snapshots += s.stale_snapshots;
        total.delta_snapshots += s.delta_snapshots;
        total.full_snapshots += s.full_snapshots;
        total.starved_snapshots += s.starved_snapshots;
        total.corrections += s.corrections;
        total.snaps += s.snaps;
        total.lag_snaps += s.lag_snaps;
        total.teleport_corrections += s.teleport_corrections;
        total.max_reconcile_error = total.max_reconcile_error.max(s.max_reconcile_error);
        total.kills += s.kills;
        total.deaths += s.deaths;
        total.hits_dealt += s.hits_dealt;
        total.protocol_errors += s.protocol_errors;
        total.desyncs += s.desyncs;
        total.behavior.add(&s.behavior);
        for (k, u) in &s.weapons {
            let t = total.weapon_mut(k);
            t.fired += u.fired;
            t.hits += u.hits;
            t.kills += u.kills;
        }
        for (k, n) in &s.ship_lives {
            *total.ship_lives.entry(k.clone()).or_default() += n;
        }
    }
    let b = &total.behavior;
    let _ = writeln!(
        out,
        "totals: joins {} (connect failures {}), disconnects {}, match restarts {}, inputs sent {}, snapshots {} ({} delta, {} full, {} stale), starved {}",
        total.joins,
        total.connect_failures,
        total.disconnects,
        total.match_restarts,
        total.inputs_sent,
        total.snapshots,
        total.delta_snapshots,
        total.full_snapshots,
        total.stale_snapshots,
        total.starved_snapshots
    );
    let _ = writeln!(
        out,
        "        reconciliation: {} corrections ({} from own teleports), {} snaps ({} from lag), max error {:.3} u",
        total.corrections,
        total.teleport_corrections,
        total.snaps,
        total.lag_snaps,
        total.max_reconcile_error
    );
    let _ = writeln!(
        out,
        "        combat: {} kills, {} hits; mode ticks combat {} hunt {}; trigger ticks laser {} rivet {} rocket {} paint {}; scavenge ticks {}",
        total.kills,
        total.hits_dealt,
        b.combat_ticks,
        b.hunt_ticks,
        b.laser_triggers,
        b.rivet_triggers,
        b.rocket_triggers,
        b.paint_triggers,
        b.scavenge_holds
    );
    let _ = writeln!(
        out,
        "        arsenal: mode ticks flee {} collect {} guide {}; secondary presses {}, gadget presses {}, paint colour presses {}",
        b.flee_ticks,
        b.collect_ticks,
        b.guide_ticks,
        b.secondary_triggers,
        b.gadget_presses,
        b.colour_presses
    );
    let fired = total.weapons_fired().count();
    let _ = writeln!(
        out,
        "        weapons fired: {fired} distinct (shots/hits/kills per weapon below; a shot is one volley per snapshot, a hit every damaging hit)"
    );
    for (k, u) in &total.weapons {
        let _ = writeln!(
            out,
            "          {k:<16} {:>6} {:>6} {:>4}",
            u.fired, u.hits, u.kills
        );
    }
    if !total.ship_lives.is_empty() {
        let lives: Vec<String> = total
            .ship_lives
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect();
        let _ = writeln!(out, "        lives per ship type: {}", lives.join(", "));
    }
    let _ = writeln!(
        out,
        "        protocol errors {}, desyncs {}",
        total.protocol_errors, total.desyncs
    );
    for s in stats {
        for e in &s.errors {
            let _ = writeln!(out, "  {}: {}", s.name, e);
        }
    }
    let verdict = match exit_code(stats, policy) {
        0 => "OK",
        1 => "FAILED (protocol error or desync)",
        2 => "FAILED (a bot never joined)",
        _ => "FAILED (a bot lost its connection)",
    };
    let _ = writeln!(out, "result: {verdict}");
    out
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes() {
        let fail = DisconnectPolicy::Fail;
        let mut ok = BotStats::new("a");
        ok.joins = 1;
        ok.match_restarts = 2;
        assert_eq!(exit_code(&[ok.clone()], fail), 0, "restarts are fine");
        let never = BotStats::new("b");
        assert_eq!(exit_code(&[ok.clone(), never.clone()], fail), 2);
        let mut lost = ok.clone();
        lost.disconnects = 1;
        assert_eq!(exit_code(&[ok.clone(), lost.clone()], fail), 3);
        assert_eq!(
            exit_code(&[ok.clone(), lost.clone()], DisconnectPolicy::Allow),
            0
        );
        assert!(summary(&[lost], 1.0, fail).contains("lost its connection"));
        let mut bad = ok.clone();
        bad.desyncs = 1;
        assert_eq!(exit_code(&[ok.clone(), bad.clone(), never], fail), 1);
        bad.desyncs = 0;
        bad.protocol_errors = 2;
        assert!(bad.failed());
        let text = summary(&[ok, bad], 12.0, fail);
        assert!(text.contains("result: FAILED"), "{text}");
        assert!(text.contains("inputs sent"), "{text}");
        assert!(text.contains("match restarts 4"), "{text}");
        let mut w = BotStats::new("w");
        w.joins = 1;
        w.weapon_mut("railspike").fired = 3;
        w.weapon_mut("railspike").hits = 1;
        w.weapon_mut("seeker").hits = 0;
        assert_eq!(w.weapons_fired().collect::<Vec<_>>(), ["railspike"]);
        let text = summary(&[w], 1.0, fail);
        assert!(text.contains("weapons fired: 1 distinct"), "{text}");
        assert!(text.contains("railspike"), "{text}");
    }
}
