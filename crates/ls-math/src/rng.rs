//! Portable, reproducible random number generators.
//!
//! - [`SplitMix64`]: turns one `u64` seed into a stream of well-mixed `u64`s.
//!   Use it to derive seeds (for example one per subsystem or per chunk).
//! - [`Pcg32`]: PCG-XSH-RR 64/32 (O'Neill), the workhorse generator. Each
//!   subsystem gets its own *stream* so adding a random call in one place never
//!   shifts the numbers another subsystem sees.
//!
//! Both are pure integer arithmetic, so they give identical output on every
//! target. Float helpers build values from integer bits with exact operations.

use serde::{Deserialize, Serialize};

const PCG_MULT: u64 = 6_364_136_223_846_793_005;

/// SplitMix64 (Steele, Lea, Flood; public-domain reference by Vigna).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    #[inline]
    pub const fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix64(self.state)
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform `f32` in `[0, 1)` with 24 bits of precision.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        u32_to_unit_f32(self.next_u32())
    }

    /// A [`Pcg32`] seeded from this generator on the given stream.
    #[inline]
    pub fn pcg32(&mut self, stream: u64) -> Pcg32 {
        Pcg32::new(self.next_u64(), stream)
    }
}

/// The SplitMix64 output function: a strong 64-bit bit mixer. Also useful
/// on its own for hashing coordinates into seeds.
#[inline]
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Uniform `f32` in `[0, 1)` from the top 24 bits of `x` (exact conversion).
#[inline]
fn u32_to_unit_f32(x: u32) -> f32 {
    (x >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// PCG32 (PCG-XSH-RR with 64-bit state, 32-bit output).
///
/// Streams: two generators with the same seed but different `stream` values
/// produce independent sequences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    /// Seeds exactly like the reference `pcg32_srandom_r(initstate, initseq)`.
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut rng = Pcg32 {
            state: 0,
            inc: (stream << 1) | 1,
        };
        rng.step();
        rng.state = rng.state.wrapping_add(seed);
        rng.step();
        rng
    }

    /// Like [`Pcg32::new`] but first scrambles `seed` with SplitMix64, so small
    /// or similar seeds (0, 1, 2, ...) still give unrelated sequences.
    pub fn from_seed(seed: u64, stream: u64) -> Self {
        Pcg32::new(mix64(seed.wrapping_add(0x9e37_79b9_7f4a_7c15)), stream)
    }

    /// A new independent generator on `stream`, seeded from this one's output.
    /// Advances `self` by two steps.
    pub fn stream(&mut self, stream: u64) -> Pcg32 {
        let seed = self.next_u64();
        Pcg32::new(seed, stream)
    }

    #[inline]
    fn step(&mut self) {
        self.state = self.state.wrapping_mul(PCG_MULT).wrapping_add(self.inc);
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.step();
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let hi = self.next_u32() as u64;
        let lo = self.next_u32() as u64;
        (hi << 32) | lo
    }

    /// Uniform `f32` in `[0, 1)` with 24 bits of precision.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        u32_to_unit_f32(self.next_u32())
    }

    /// Uniform `f32` in `[lo, hi)`.
    #[inline]
    pub fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    /// Uniform integer in `[0, bound)` without modulo bias. Returns 0 for `bound == 0`.
    pub fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            return 0;
        }
        // Reference pcg32_boundedrand_r: reject the low partial range.
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let r = self.next_u32();
            if r >= threshold {
                return r % bound;
            }
        }
    }

    /// Uniform integer in `[lo, hi]` (inclusive). Returns `lo` if `hi <= lo`.
    pub fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        let span = (hi as i64 - lo as i64 + 1) as u64;
        if span > u32::MAX as u64 {
            // Full 32-bit range: every u32 is valid.
            return lo.wrapping_add(self.next_u32() as i32);
        }
        (lo as i64 + self.below(span as u32) as i64) as i32
    }

    #[inline]
    pub fn next_bool(&mut self) -> bool {
        self.next_u32() & 0x8000_0000 != 0
    }

    /// True with probability `p` (clamped to `[0, 1]`).
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }

    /// Index chosen with probability proportional to `weights[i]`. Negative or
    /// NaN weights count as zero; returns `None` if the total is not positive.
    pub fn weighted_index(&mut self, weights: &[f32]) -> Option<usize> {
        let clean = |w: f32| if w > 0.0 { w } else { 0.0 };
        let total: f32 = weights.iter().copied().map(clean).sum();
        if total <= 0.0 || !total.is_finite() {
            return None;
        }
        let mut r = self.next_f32() * total;
        let mut last_positive = None;
        for (i, &w) in weights.iter().enumerate() {
            let w = clean(w);
            if w > 0.0 {
                last_positive = Some(i);
                if r < w {
                    return Some(i);
                }
                r -= w;
            }
        }
        // Rounding can leave r slightly >= the last weight.
        last_positive
    }

    /// Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// Jumps ahead by `delta` steps in O(log delta) (Brown's algorithm).
    pub fn advance(&mut self, mut delta: u64) {
        let mut cur_mult = PCG_MULT;
        let mut cur_plus = self.inc;
        let mut acc_mult: u64 = 1;
        let mut acc_plus: u64 = 0;
        while delta > 0 {
            if delta & 1 == 1 {
                acc_mult = acc_mult.wrapping_mul(cur_mult);
                acc_plus = acc_plus.wrapping_mul(cur_mult).wrapping_add(cur_plus);
            }
            cur_plus = cur_mult.wrapping_add(1).wrapping_mul(cur_plus);
            cur_mult = cur_mult.wrapping_mul(cur_mult);
            delta >>= 1;
        }
        self.state = acc_mult.wrapping_mul(self.state).wrapping_add(acc_plus);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn splitmix_reference_vector() {
        // Reference outputs for seed 1234567 (Vigna's splitmix64.c).
        let mut r = SplitMix64::new(1_234_567);
        let expected = [
            6_457_827_717_110_365_317u64,
            3_203_168_211_198_807_973,
            9_817_491_932_198_370_423,
            4_593_380_528_125_082_431,
            16_408_922_859_458_223_821,
        ];
        for e in expected {
            assert_eq!(r.next_u64(), e);
        }
    }

    #[test]
    fn pcg32_reference_vector() {
        // Output of the reference pcg32-demo with pcg32_srandom_r(42, 54).
        let mut r = Pcg32::new(42, 54);
        let expected = [
            0xa15c_02b7u32,
            0x7b47_f409,
            0xba1d_3330,
            0x83d2_f293,
            0xbfa4_784b,
            0xcbed_606e,
        ];
        for e in expected {
            assert_eq!(r.next_u32(), e);
        }
    }

    #[test]
    fn streams_are_independent() {
        let mut a = Pcg32::new(7, 1);
        let mut b = Pcg32::new(7, 2);
        let va: Vec<u32> = (0..16).map(|_| a.next_u32()).collect();
        let vb: Vec<u32> = (0..16).map(|_| b.next_u32()).collect();
        assert_ne!(va, vb);
        // Same seed and stream reproduces.
        let mut c = Pcg32::new(7, 1);
        let vc: Vec<u32> = (0..16).map(|_| c.next_u32()).collect();
        assert_eq!(va, vc);
        // from_seed scrambles nearby seeds.
        assert_ne!(
            Pcg32::from_seed(0, 0).next_u32(),
            Pcg32::from_seed(1, 0).next_u32()
        );
        // Child streams differ from each other.
        let mut parent = Pcg32::from_seed(99, 0);
        let mut s1 = parent.clone().stream(1);
        let mut s2 = parent.stream(2);
        assert_ne!(s1.next_u32(), s2.next_u32());
        let mut sm = SplitMix64::new(5);
        let mut p = sm.pcg32(3);
        let _ = p.next_u32();
    }

    #[test]
    fn floats_in_range() {
        let mut r = Pcg32::from_seed(123, 0);
        let mut sm = SplitMix64::new(9);
        for _ in 0..10_000 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f));
            let g = r.range_f32(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&g));
            let h = sm.next_f32();
            assert!((0.0..1.0).contains(&h));
        }
        assert_eq!(u32_to_unit_f32(u32::MAX), 1.0 - 1.0 / 16_777_216.0);
        assert_eq!(u32_to_unit_f32(0), 0.0);
    }

    #[test]
    fn integer_ranges() {
        let mut r = Pcg32::from_seed(5, 5);
        let mut counts = [0u32; 6];
        for _ in 0..60_000 {
            let v = r.below(6);
            counts[v as usize] += 1;
        }
        for c in counts {
            // Expected 10_000 each; generous tolerance.
            assert!((9_000..11_000).contains(&c), "{counts:?}");
        }
        assert_eq!(r.below(0), 0);
        assert_eq!(r.below(1), 0);
        for _ in 0..1000 {
            let v = r.range_i32(-3, 3);
            assert!((-3..=3).contains(&v));
        }
        assert_eq!(r.range_i32(4, 4), 4);
        assert_eq!(r.range_i32(4, 1), 4);
        let _ = r.range_i32(i32::MIN, i32::MAX);
        let trues = (0..10_000).filter(|_| r.next_bool()).count();
        assert!((4_500..5_500).contains(&trues));
        assert!(!r.chance(0.0));
        assert!(r.chance(1.0));
    }

    #[test]
    fn weighted_and_shuffle() {
        let mut r = Pcg32::from_seed(77, 0);
        assert_eq!(r.weighted_index(&[]), None);
        assert_eq!(r.weighted_index(&[0.0, -1.0]), None);
        assert_eq!(r.weighted_index(&[0.0, 2.0, 0.0]), Some(1));
        let mut counts = [0u32; 2];
        for _ in 0..10_000 {
            counts[r.weighted_index(&[1.0, 3.0]).unwrap()] += 1;
        }
        assert!((2_200..2_800).contains(&counts[0]), "{counts:?}");

        let mut v: Vec<u32> = (0..50).collect();
        r.shuffle(&mut v);
        let mut sorted = v.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
        assert_ne!(v, sorted);
    }

    #[test]
    fn advance_matches_stepping() {
        let mut a = Pcg32::new(42, 54);
        let mut b = a.clone();
        for _ in 0..1000 {
            a.next_u32();
        }
        b.advance(1000);
        assert_eq!(a, b);
        b.advance(0);
        assert_eq!(a.next_u32(), b.next_u32());
    }

    #[test]
    fn serde_roundtrip_preserves_state() {
        let mut r = Pcg32::from_seed(1, 2);
        r.next_u32();
        let s = ron::to_string(&r).unwrap();
        let mut back: Pcg32 = ron::from_str(&s).unwrap();
        assert_eq!(r.next_u32(), back.next_u32());
    }
}
