//! Client clock: how far the client's input ticks run ahead of the server,
//! and the estimated server time used for interpolation.
//!
//! The client stamps inputs with ticks ahead of the server so each command
//! arrives before the server simulates its tick. Two signals steer the lead:
//!
//! - **Starvation** (snapshots): `ack_input` is the newest client input the
//!   server had applied at the snapshot tick. If it is older than the snapshot
//!   tick, the server ran out of input and repeated the last command; the
//!   client jumps ahead by the missing ticks.
//! - **Slack** (ping/pong): a ping sent right after the input for tick `c` is
//!   answered with the server tick `s` it arrived at, so `c - s` is how many
//!   ticks early that input was. When the smallest slack over the last few
//!   pongs exceeds [`MAX_SLACK_TICKS`], the client skips a tick to cut
//!   latency; below [`MIN_SLACK_TICKS`] it runs ahead. A negative slack
//!   before the clock is synced (the client is behind the server altogether,
//!   e.g. the first pong after a slow level generation) catches up to the
//!   same lead as a starvation report from behind (`MIN_SLACK_TICKS +
//!   INITIAL_LEAD_TICKS + BEHIND_MARGIN_TICKS` = 12 ticks of slack). Once
//!   synced, a negative slack (one late input, e.g. a Wi-Fi hiccup) is an
//!   ordinary slack sample and raises the lead only to `MIN_SLACK_TICKS + 1`.
//!
//! Look input is sent as per-tick deltas, so a command the server had to
//! replace by a repeat can leave a small permanent orientation difference.
//! The session therefore sends idle commands until the clock is
//! [`ClientClock::is_synced`] (first snapshot that was not starved).

use std::collections::VecDeque;

use super::session::INITIAL_LEAD_TICKS;

/// Slack (ticks) below which the client runs further ahead.
pub const MIN_SLACK_TICKS: i64 = 1;
/// Slack (ticks) above which the client skips a tick.
pub const MAX_SLACK_TICKS: i64 = 4;
/// Pongs considered for the slack minimum.
pub const SLACK_WINDOW: usize = 4;
/// Extra lead (ticks) when catching up from behind the server, for the
/// round trip the snapshot and the next input take (~130 ms).
pub const BEHIND_MARGIN_TICKS: u64 = 8;

/// See the module docs.
#[derive(Debug, Clone, Default)]
pub struct ClientClock {
    /// Estimated current server tick (fractional).
    server_time: f64,
    newest_snapshot: Option<u32>,
    /// Extra ticks to run (positive) or skip (negative) on upcoming frames.
    pending: i64,
    slack: VecDeque<i64>,
    synced: bool,
    /// Starvation reports for snapshot ticks before this input tick describe
    /// inputs sent before the last catch-up and are ignored.
    ignore_starved_before: u64,
    ignore_slack: usize,
    /// Starvation reports acted on (a backlog of snapshots handled at once
    /// counts once).
    pub starved: u64,
}

impl ClientClock {
    /// A clock synchronized to `server_tick` (from `Welcome`).
    pub fn new(server_tick: u32) -> Self {
        ClientClock {
            server_time: server_tick as f64,
            ..Default::default()
        }
    }

    /// Estimated current server tick.
    pub fn server_time(&self) -> f64 {
        self.server_time
    }

    /// True once a snapshot showed that the server had our input in time.
    pub fn is_synced(&self) -> bool {
        self.synced
    }

    /// Advances the server time estimate by `ticks` of local time.
    pub fn advance(&mut self, ticks: f64) {
        self.server_time += ticks;
        if let Some(n) = self.newest_snapshot {
            // Without snapshots the estimate must not run away (at most 8 ticks
            // past the newest snapshot).
            self.server_time = self.server_time.min(n as f64 + 8.0);
        }
    }

    /// Feeds a snapshot's tick and `ack_input`, and the tick the client's
    /// next input will get.
    pub fn on_snapshot(&mut self, tick: u32, ack_input: u32, next_input_tick: u64) {
        if self.newest_snapshot.is_none_or(|n| tick > n) {
            self.newest_snapshot = Some(tick);
            self.server_time = self.server_time.max(tick as f64);
        }
        if ack_input >= tick {
            self.synced = true;
            return;
        }
        // A client that took long to set up (level generation) can be behind
        // the server altogether: jump straight past the snapshot tick plus
        // the usual lead and a margin for the round trip, however far that
        // is. Such a report only ever raises the target, so it is used even
        // inside the window below (a backlog of snapshots handled at once).
        let behind = if next_input_tick <= tick as u64 {
            (tick as u64 + 1 + MIN_SLACK_TICKS as u64 + INITIAL_LEAD_TICKS + BEHIND_MARGIN_TICKS
                - next_input_tick) as i64
        } else {
            0
        };
        let fresh = tick as u64 >= self.ignore_starved_before;
        if behind > self.pending || fresh {
            // A backlog only raises the target of the catch-up already under
            // way: one starvation, not one per queued snapshot.
            if fresh {
                self.starved += 1;
            }
            self.slack.clear();
            let missing = (tick - ack_input).min(30) as i64;
            self.pending = self.pending.max(missing + MIN_SLACK_TICKS).max(behind);
            self.ignore_starved_before = next_input_tick + self.pending as u64;
        }
    }

    /// Feeds a pong: `sent_tick` is the newest input tick sent before the
    /// ping, `server_tick` the tick the pong reports. Pings sent before the
    /// last starvation catch-up describe the old lead and are ignored.
    pub fn on_pong(&mut self, sent_tick: u64, server_tick: u32) {
        if sent_tick < self.ignore_starved_before {
            return;
        }
        let slack = sent_tick as i64 - server_tick as i64;
        if slack < 0 && !self.synced {
            // Behind the server altogether (typically the first pong after a
            // slow level generation, which arrives before the first
            // snapshot): catch up to the same lead as the starvation path,
            // not just to the bottom of the slack window, where every jitter
            // spike would starve the server again. Like a starvation report,
            // this ignores reports about inputs sent before the catch-up.
            // Once synced, a late pong is an ordinary slack sample: jumping
            // to the full margin mid-match would add ~130 ms of input lead
            // that the slow trim takes ~16 s to remove.
            let target = MIN_SLACK_TICKS + INITIAL_LEAD_TICKS as i64 + BEHIND_MARGIN_TICKS as i64;
            self.pending = self.pending.max(target - slack);
            self.slack.clear();
            self.ignore_starved_before = sent_tick + 1 + self.pending as u64;
            return;
        }
        self.on_slack(slack);
    }

    /// Feeds the slack measured by a ping/pong: input tick sent with the ping
    /// minus the server tick the pong reports.
    pub fn on_slack(&mut self, slack: i64) {
        if self.ignore_slack > 0 {
            self.ignore_slack -= 1;
            return;
        }
        if self.slack.len() == SLACK_WINDOW {
            self.slack.pop_front();
        }
        self.slack.push_back(slack);
        let min = self.slack.iter().copied().min().unwrap_or(slack);
        if min < MIN_SLACK_TICKS {
            self.pending = self.pending.max(MIN_SLACK_TICKS + 1 - min);
            self.slack.clear();
            // Pings in flight were sent before this correction.
            self.ignore_slack = SLACK_WINDOW;
        } else if min > MAX_SLACK_TICKS && self.slack.len() == SLACK_WINDOW && self.pending == 0 {
            self.pending = -1;
            self.slack.clear();
        }
    }

    /// How many simulation ticks to run this frame when the host's fixed-step
    /// accumulator says `due` ticks are due: `due` plus/minus the pending
    /// adjustment (at most 4 extra ticks per call once synced, 64 before, or
    /// 1 skipped tick per call).
    pub fn ticks_to_run(&mut self, due: u32) -> u32 {
        if self.pending > 0 {
            // Before sync the commands are idle, so a long catch-up costs
            // nothing but prediction steps.
            let cap = if self.synced { 4 } else { 64 };
            let extra = self.pending.min(cap);
            self.pending -= extra;
            due + extra as u32
        } else if self.pending < 0 && due > 0 {
            self.pending += 1;
            due - 1
        } else {
            due
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn starvation_speeds_up_and_slack_slows_down() {
        let mut c = ClientClock::new(100);
        c.on_snapshot(102, 99, 104);
        assert!(!c.is_synced());
        assert_eq!(c.starved, 1);
        // A second report about inputs sent before the catch-up is ignored
        // (the client is still ahead of that snapshot).
        c.on_snapshot(103, 101, 104);
        assert_eq!(c.starved, 1);
        assert_eq!(c.ticks_to_run(1), 5);
        assert_eq!(c.ticks_to_run(1), 1);
        c.on_snapshot(110, 110, 112);
        assert!(c.is_synced());
        for _ in 0..SLACK_WINDOW {
            c.on_slack(7);
        }
        assert_eq!(c.ticks_to_run(1), 0, "too early: skip a tick");
        assert_eq!(c.ticks_to_run(1), 1);
        c.on_slack(0);
        assert_eq!(c.ticks_to_run(1), 3, "late: run ahead");
        c.on_slack(0);
        assert_eq!(
            c.ticks_to_run(1),
            1,
            "pings from before the correction are ignored"
        );
        c.advance(3.0);
        assert!(c.server_time() >= 110.0);
    }

    #[test]
    fn a_client_far_behind_the_server_catches_up_in_one_report() {
        // The session was built 90 ticks after the Welcome (tick 100): its
        // next input is for tick 104 while the server is at 190.
        let mut c = ClientClock::new(100);
        c.on_snapshot(190, 100, 104);
        let expected =
            190 + 1 + MIN_SLACK_TICKS as u64 + INITIAL_LEAD_TICKS + BEHIND_MARGIN_TICKS - 104;
        let mut ran = 0;
        for _ in 0..40 {
            ran += c.ticks_to_run(1) as u64 - 1;
        }
        assert_eq!(ran, expected);
        // Reports about inputs from before the catch-up are ignored.
        c.on_snapshot(192, 100, 104 + 40 + ran);
        assert_eq!(c.starved, 1);
    }

    #[test]
    fn a_pong_from_far_behind_catches_up_with_the_full_margin() {
        // First pong after a slow setup, before any snapshot: our input for
        // tick 104 reached the server at tick 190.
        let mut c = ClientClock::new(100);
        c.on_pong(104, 190);
        let target = MIN_SLACK_TICKS + INITIAL_LEAD_TICKS as i64 + BEHIND_MARGIN_TICKS as i64;
        let mut ran = 0;
        for _ in 0..40 {
            ran += c.ticks_to_run(1) as i64 - 1;
        }
        assert_eq!(104 + ran - 190, target, "slack after the catch-up");
        // Pongs sent before the catch-up do not add to it.
        c.on_pong(105, 191);
        assert_eq!(c.ticks_to_run(1), 1);
    }

    #[test]
    fn one_late_pong_after_sync_only_nudges_the_lead() {
        let mut c = ClientClock::new(100);
        c.on_snapshot(110, 110, 113);
        assert!(c.is_synced());
        // Steady slack 3, then one input arrives a tick late (slack -1).
        c.on_pong(400, 397);
        assert_eq!(c.ticks_to_run(1), 1);
        c.on_pong(400, 401);
        let slack = -1;
        let mut ran = 0;
        for _ in 0..10 {
            ran += c.ticks_to_run(1) as i64 - 1;
        }
        assert_eq!(
            ran,
            MIN_SLACK_TICKS + 1 - slack,
            "not the full catch-up margin"
        );
        // Starvation reports are not hidden by the late pong.
        let before = c.starved;
        c.on_snapshot(420, 418, 430);
        assert_eq!(c.starved, before + 1);
    }

    #[test]
    fn a_backlog_of_snapshots_counts_as_one_starvation() {
        // Eight snapshots queued while the session was built, handled at once:
        // each raises the catch-up target, but it is one starvation.
        let mut c = ClientClock::new(100);
        for tick in (102..=116).step_by(2) {
            c.on_snapshot(tick, 100, 104);
        }
        assert_eq!(c.starved, 1);
        let expected =
            116 + 1 + MIN_SLACK_TICKS as u64 + INITIAL_LEAD_TICKS + BEHIND_MARGIN_TICKS - 104;
        let mut ran = 0;
        for _ in 0..40 {
            ran += c.ticks_to_run(1) as u64 - 1;
        }
        assert_eq!(ran, expected, "the newest snapshot sets the target");
    }
}
