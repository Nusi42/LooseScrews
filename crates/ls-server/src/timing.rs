//! Fixed-step scheduling and tick timing statistics for the sim thread.

use std::time::{Duration, Instant};

/// Length of one 60 Hz tick.
pub const TICK_PERIOD: Duration = Duration::from_nanos(1_000_000_000 / ls_math::TICK_HZ as u64);

/// What the scheduler wants done now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Due {
    /// Ticks to run back to back now.
    pub run: u32,
    /// Ticks given up because the loop fell too far behind.
    pub skipped: u64,
}

/// Deadline-based fixed-step scheduler: tick `n` is due at `start + n *
/// period`, so timing errors never accumulate. After a stall it catches up at
/// most `max_burst` ticks per call; if more than `max_backlog` ticks are
/// overdue it drops the excess (the simulation slows down instead of
/// spiralling).
#[derive(Debug, Clone)]
pub struct TickScheduler {
    period: Duration,
    next: Instant,
    max_burst: u32,
    max_backlog: u64,
}

impl TickScheduler {
    /// First tick due one period after `start`.
    pub fn new(start: Instant, period: Duration, max_burst: u32, max_backlog: u64) -> Self {
        TickScheduler {
            period,
            next: start + period,
            max_burst: max_burst.max(1),
            max_backlog: max_backlog.max(1),
        }
    }

    /// The 60 Hz scheduler used by the server: bursts of up to 4 ticks, at
    /// most 15 ticks (250 ms) of backlog.
    pub fn sixty_hz(start: Instant) -> Self {
        TickScheduler::new(start, TICK_PERIOD, 4, 15)
    }

    /// When the next tick is due.
    pub fn next_deadline(&self) -> Instant {
        self.next
    }

    /// Ticks to run at `now`; advances the schedule by what it returns.
    pub fn due(&mut self, now: Instant) -> Due {
        if now < self.next {
            return Due::default();
        }
        let behind = now - self.next;
        let mut due = (behind.as_nanos() / self.period.as_nanos().max(1)) as u64 + 1;
        let mut skipped = 0;
        if due > self.max_backlog {
            skipped = due - self.max_backlog;
            self.next += self.period * skipped as u32;
            due = self.max_backlog;
        }
        let run = due.min(self.max_burst as u64) as u32;
        self.next += self.period * run;
        Due { run, skipped }
    }
}

/// Sleeps until `deadline` with sub-millisecond accuracy: a coarse sleep
/// until shortly before, then yields.
pub fn wait_until(deadline: Instant) {
    const SPIN: Duration = Duration::from_micros(1500);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > SPIN {
            std::thread::sleep(left - SPIN);
        } else {
            std::thread::yield_now();
        }
    }
}

/// Tick durations over one reporting window.
#[derive(Debug, Clone, Default)]
pub struct TickTimes {
    micros: Vec<u32>,
    over_budget: u64,
}

/// Summary of a window of tick durations (microseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickSummary {
    pub ticks: u64,
    pub p50_us: u32,
    pub p99_us: u32,
    pub max_us: u32,
    /// Ticks that took longer than one period.
    pub over_budget: u64,
}

impl TickTimes {
    pub fn record(&mut self, d: Duration) {
        if d > TICK_PERIOD {
            self.over_budget += 1;
        }
        self.micros.push(d.as_micros().min(u32::MAX as u128) as u32);
    }

    pub fn len(&self) -> usize {
        self.micros.len()
    }

    pub fn is_empty(&self) -> bool {
        self.micros.is_empty()
    }

    /// Percentiles of the window (nearest rank), then clears it.
    pub fn take_summary(&mut self) -> TickSummary {
        let mut v = std::mem::take(&mut self.micros);
        let over_budget = std::mem::take(&mut self.over_budget);
        if v.is_empty() {
            return TickSummary::default();
        }
        v.sort_unstable();
        let rank = |p: usize| v[((v.len() * p).div_ceil(100)).clamp(1, v.len()) - 1];
        TickSummary {
            ticks: v.len() as u64,
            p50_us: rank(50),
            p99_us: rank(99),
            max_us: *v.last().unwrap_or(&0),
            over_budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Duration = Duration::from_millis(10);

    #[test]
    fn scheduler_runs_on_deadlines_without_drift() {
        let t0 = Instant::now();
        let mut s = TickScheduler::new(t0, P, 4, 15);
        assert_eq!(s.due(t0).run, 0);
        assert_eq!(s.due(t0 + Duration::from_millis(9)).run, 0);
        assert_eq!(s.due(t0 + Duration::from_millis(10)).run, 1);
        // Late wake-up by 3 ms: the next deadline stays on the grid.
        assert_eq!(s.due(t0 + Duration::from_millis(23)).run, 1);
        assert_eq!(s.next_deadline(), t0 + Duration::from_millis(30));
    }

    #[test]
    fn scheduler_catches_up_in_bursts_and_limits_backlog() {
        let t0 = Instant::now();
        let mut s = TickScheduler::new(t0, P, 4, 15);
        // 10 ticks overdue: 4 + 4 + 2.
        let now = t0 + Duration::from_millis(105);
        assert_eq!(s.due(now), Due { run: 4, skipped: 0 });
        assert_eq!(s.due(now), Due { run: 4, skipped: 0 });
        assert_eq!(s.due(now), Due { run: 2, skipped: 0 });
        assert_eq!(s.due(now).run, 0);
        // A 1 s stall: only 15 ticks are caught up, the rest skipped.
        let now = now + Duration::from_secs(1);
        let first = s.due(now);
        assert_eq!(first.run, 4);
        assert!(first.skipped > 80, "{first:?}");
        let mut total = first.run;
        loop {
            let d = s.due(now);
            assert_eq!(d.skipped, 0);
            if d.run == 0 {
                break;
            }
            total += d.run;
        }
        assert_eq!(total, 15);
    }

    #[test]
    fn summary_percentiles() {
        let mut t = TickTimes::default();
        for i in 1..=100u64 {
            t.record(Duration::from_micros(i * 100));
        }
        t.record(Duration::from_millis(20));
        let s = t.take_summary();
        assert_eq!(s.ticks, 101);
        assert_eq!(s.p50_us, 5100);
        assert_eq!(s.p99_us, 10_000);
        assert_eq!(s.max_us, 20_000);
        assert_eq!(s.over_budget, 1);
        assert!(t.is_empty());
        assert_eq!(t.take_summary(), TickSummary::default());
    }

    /// Lateness of one 5 ms `wait_until`.
    fn wait_lateness() -> Duration {
        let deadline = Instant::now() + Duration::from_millis(5);
        wait_until(deadline);
        Instant::now() - deadline
    }

    /// Loose bound for the normal (parallel, possibly loaded) test run: only
    /// gross breakage fails here.
    #[test]
    fn wait_until_is_not_grossly_late() {
        let late = wait_lateness();
        assert!(late < Duration::from_millis(50), "{late:?}");
    }

    /// The strict bound, run by tools/ci.* in its own serial step
    /// (`--ignored --test-threads=1`), where parallel builds can't delay it.
    #[test]
    #[ignore = "wall-clock sensitive: run serially by tools/ci.* (--ignored --test-threads=1)"]
    fn wait_until_is_accurate() {
        let late = wait_lateness();
        assert!(late < Duration::from_millis(10), "{late:?}");
    }
}
