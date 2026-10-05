//! Radiation on asteroid belt levels (M4 Belt Up; `docs/milestone-4-plan.md`
//! §3.8 "Radiation").
//!
//! Active on every level with a belt (`MatchSetup::belt`), never on classic
//! levels. The dose rate at a point follows the signed distance `d` to the
//! shelter (the belt volume or the cocoon around the exit,
//! [`BeltLevel::shelter_distance`]):
//!
//! | Where | Rate (hull/s) |
//! | --- | --- |
//! | deeper than `deep` (4 u) inside | 0 |
//! | `-deep < d < 0` | rising linearly to `surface_rate` (2) |
//! | outside (`d >= 0`) | `surface_rate + slope * d` (0.4 per u), at most `cap` (16, 35 u out) |
//!
//! The world applies it every tick to living ships as hull loss that
//! bypasses shields and gadgets (like The Squeeze's crushing) and credits
//! nobody (the killer is "the level"), scaled by the mode's
//! `ModeModule::radiation_scale` (default 1; Belt Up's rule set setting).
//! Numbers: `data/world/radiation.ron` ([`RadiationSettings`]).

use std::sync::OnceLock;

use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::belt::BeltLevel;

/// `data/world/radiation.ron` (embedded, part of the data hash).
pub const RADIATION_RON: &str = include_str!("../../../data/world/radiation.ron");

/// The dose law (`data/world/radiation.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RadiationSettings {
    /// Depth inside the shelter (u) below which the rate is 0.
    pub deep: f32,
    /// Rate at the shelter's surface (hull/s).
    pub surface_rate: f32,
    /// Extra rate per u outside (hull/s per u).
    pub slope: f32,
    /// Highest rate (hull/s).
    pub cap: f32,
}

impl RadiationSettings {
    /// The embedded `data/world/radiation.ron`.
    pub fn builtin() -> &'static RadiationSettings {
        static S: OnceLock<RadiationSettings> = OnceLock::new();
        S.get_or_init(|| {
            ron::from_str(RADIATION_RON).expect("data/world/radiation.ron is valid (tested)")
        })
    }

    /// Dose rate (hull/s) at signed shelter distance `d` (negative inside).
    pub fn rate_at_distance(&self, d: f32) -> f32 {
        if d.is_nan() || d <= -self.deep {
            0.0
        } else if d < 0.0 {
            self.surface_rate * (1.0 + d / self.deep)
        } else {
            (self.surface_rate + self.slope * d).min(self.cap)
        }
    }
}

/// Dose rate (hull/s, unscaled) at `p` on a belt level. What the HUD meter
/// shows (`ls-wasm` exports it).
pub fn dose_rate<B: Shelter + ?Sized>(belt: &B, settings: &RadiationSettings, p: Vec3) -> f32 {
    settings.rate_at_distance(belt.shelter_distance(p))
}

/// Signed distance to the sheltered volume (negative inside): the belt
/// geometry the dose law reads (`ls_math::level::BeltSite`, the match
/// setup's belt, and [`BeltLevel`]).
pub trait Shelter {
    fn shelter_distance(&self, p: Vec3) -> f32;
}

impl Shelter for ls_math::level::BeltSite {
    fn shelter_distance(&self, p: Vec3) -> f32 {
        ls_math::level::BeltSite::shelter_distance(self, p)
    }
}

impl Shelter for BeltLevel {
    fn shelter_distance(&self, p: Vec3) -> f32 {
        BeltLevel::shelter_distance(self, p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::belt::tests::plan_belt;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn rate_follows_the_plan_profile() {
        let s = RadiationSettings::builtin();
        assert_eq!(
            *s,
            RadiationSettings {
                deep: 4.0,
                surface_rate: 2.0,
                slope: 0.4,
                cap: 16.0
            }
        );
        assert_eq!(s.rate_at_distance(-10.0), 0.0, "deep inside");
        assert_eq!(s.rate_at_distance(-4.0), 0.0);
        assert_eq!(s.rate_at_distance(-2.0), 1.0, "half way in");
        assert_eq!(s.rate_at_distance(0.0), 2.0, "surface");
        assert!((s.rate_at_distance(10.0) - 6.0).abs() < 1e-5, "slope");
        assert!((s.rate_at_distance(34.0) - 15.6).abs() < 1e-4);
        assert_eq!(s.rate_at_distance(35.0), 16.0, "cap at 35 u");
        assert_eq!(s.rate_at_distance(200.0), 16.0);
        assert_eq!(s.rate_at_distance(f32::NAN), 0.0);
        // Monotone.
        let mut last = 0.0;
        for i in -100..500 {
            let r = s.rate_at_distance(i as f32 * 0.1);
            assert!(r >= last);
            last = r;
        }
    }

    #[test]
    fn dose_on_the_plan_belt() {
        let b = plan_belt();
        let s = RadiationSettings::builtin();
        // Deep in the belt and in the cocoon: nothing.
        assert_eq!(dose_rate(&b, s, b.point(0.7, 120.0, 0.0)), 0.0);
        assert_eq!(dose_rate(&b, s, b.center), 0.0);
        assert_eq!(dose_rate(&b, s, b.center + Vec3::new(0.0, 20.0, 0.0)), 0.0);
        // At the belt surface: 2 hp/s; 4 u above it (the skimming line): 3.6.
        assert_eq!(dose_rate(&b, s, b.point(0.7, 120.0, 12.0)), 2.0);
        assert!((dose_rate(&b, s, b.point(0.7, 120.0, 16.0)) - 3.6).abs() < 1e-4);
        // Far above: the cap.
        assert_eq!(dose_rate(&b, s, b.point(0.7, 120.0, 60.0)), 16.0);
        // The dive from the inner edge to the cocoon in the belt plane:
        // hottest half way (32.5 u from both).
        let mid = dose_rate(&b, s, b.point(0.7, 62.5, 0.0));
        assert!((mid - 15.0).abs() < 1e-3, "{mid}");
    }

    /// §3.8 tuning targets, integrated along straight lines at the given
    /// speeds: the dive at full straight boost costs a Tinker about 35 hp;
    /// skimming 4 u above the belt for a whole middle lap at cruise kills a
    /// Tinker (100 hull) about once.
    #[test]
    fn tuning_targets() {
        let b = plan_belt();
        let s = RadiationSettings::builtin();
        let dt = ls_math::DT;
        // Dive: inner edge (95 u) to the cocoon (30 u) at 16.5 u/s.
        let (mut r, mut hp) = (95.0f32, 0.0f32);
        while r > 30.0 {
            hp += dose_rate(&b, s, b.point(0.3, r, 0.0)) * dt;
            r -= 16.5 * dt;
        }
        assert!((25.0..45.0).contains(&hp), "dive costs {hp} hp");
        // Skim: 4 u above the belt's middle, one lap (754 u) at 16.5 u/s.
        let lap = std::f32::consts::TAU * 120.0;
        let secs = lap / 16.5;
        let skim = dose_rate(&b, s, b.point(0.3, 120.0, 16.0)) * secs;
        assert!(
            (100.0..200.0).contains(&skim),
            "skimming lap costs {skim} hp"
        );
    }
}
