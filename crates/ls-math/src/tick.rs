//! Fixed-step simulation time.
//!
//! Deterministic code never reads a clock. Time is a tick counter at
//! [`TICK_HZ`] and every step advances by exactly [`DT`] seconds.

use core::fmt;
use core::ops::{Add, AddAssign, Sub};

use serde::{Deserialize, Serialize};

use crate::det;

/// Simulation rate in ticks per second.
pub const TICK_HZ: u32 = 60;

/// Seconds per tick as `f32` (1/60, rounded to nearest `f32`).
pub const DT: f32 = 1.0 / TICK_HZ as f32;

/// Seconds per tick as `f64`, for non-deterministic uses such as display.
pub const DT_F64: f64 = 1.0 / TICK_HZ as f64;

/// A simulation tick number. Tick 0 is the first simulated step of a match.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Tick(pub u64);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    #[inline]
    pub const fn new(n: u64) -> Self {
        Tick(n)
    }

    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The following tick.
    #[inline]
    pub const fn next(self) -> Tick {
        Tick(self.0 + 1)
    }

    /// The previous tick, saturating at zero.
    #[inline]
    pub const fn prev(self) -> Tick {
        Tick(self.0.saturating_sub(1))
    }

    /// Ticks elapsed from `earlier` to `self`, saturating at zero.
    #[inline]
    pub const fn since(self, earlier: Tick) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// Elapsed time since tick 0 in seconds (`f64`, for display and logging only).
    #[inline]
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 * DT_F64
    }

    /// Number of whole ticks closest to `secs` seconds (deterministic,
    /// rounds half away from zero; negative input gives 0).
    #[inline]
    pub fn ticks_for_secs(secs: f32) -> u64 {
        let t = det::round(secs * TICK_HZ as f32);
        if t <= 0.0 {
            0
        } else {
            t as u64
        }
    }

    /// Number of ticks for a duration in milliseconds, rounded to nearest.
    #[inline]
    pub const fn ticks_for_millis(ms: u64) -> u64 {
        (ms * TICK_HZ as u64 + 500) / 1000
    }
}

impl Add<u64> for Tick {
    type Output = Tick;
    #[inline]
    fn add(self, n: u64) -> Tick {
        Tick(self.0 + n)
    }
}

impl AddAssign<u64> for Tick {
    #[inline]
    fn add_assign(&mut self, n: u64) {
        self.0 += n;
    }
}

impl Sub<u64> for Tick {
    type Output = Tick;
    /// Saturating subtraction of a tick count.
    #[inline]
    fn sub(self, n: u64) -> Tick {
        Tick(self.0.saturating_sub(n))
    }
}

impl From<u64> for Tick {
    #[inline]
    fn from(n: u64) -> Self {
        Tick(n)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tick {}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn dt_value() {
        assert_eq!(TICK_HZ, 60);
        assert_eq!(DT, 1.0f32 / 60.0);
        assert_eq!(DT.to_bits(), 0x3c88_8889);
        assert!((DT_F64 - 1.0 / 60.0).abs() < 1e-15);
    }

    #[test]
    fn tick_arithmetic() {
        let t = Tick::new(10);
        assert_eq!(t.next(), Tick(11));
        assert_eq!(t.prev(), Tick(9));
        assert_eq!(Tick::ZERO.prev(), Tick::ZERO);
        assert_eq!(t + 5, Tick(15));
        assert_eq!(t - 20, Tick::ZERO);
        assert_eq!(Tick(15).since(t), 5);
        assert_eq!(t.since(Tick(15)), 0);
        let mut u = t;
        u += 50;
        assert_eq!(u.get(), 60);
        assert_eq!(u.as_secs_f64(), 1.0);
        assert!(Tick(1) < Tick(2));
        assert_eq!(Tick::from(3u64), Tick(3));
        assert_eq!(Tick(7).to_string(), "tick 7");
    }

    #[test]
    fn durations() {
        assert_eq!(Tick::ticks_for_secs(1.0), 60);
        assert_eq!(Tick::ticks_for_secs(3.0), 180);
        assert_eq!(Tick::ticks_for_secs(0.1), 6);
        assert_eq!(Tick::ticks_for_secs(-1.0), 0);
        assert_eq!(Tick::ticks_for_secs(300.0), 18_000);
        assert_eq!(Tick::ticks_for_millis(100), 6);
        assert_eq!(Tick::ticks_for_millis(250), 15);
        assert_eq!(Tick::ticks_for_millis(1000), 60);
    }
}
