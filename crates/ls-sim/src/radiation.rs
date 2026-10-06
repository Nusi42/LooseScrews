//! Radiation on asteroid belt levels (M4 Belt Up; `docs/milestone-4-plan.md`
//! §3.8 "Radiation").
//!
//! Active on every level with a belt (`MatchSetup::belt`), never on classic
//! levels. The dose rate at a point follows the signed distance `d` to the
//! shelter (the belt volume or the cocoon around the exit,
//! [`BeltSite::shelter_distance`]):
//!
//! | Where | Rate (hull/s) |
//! | --- | --- |
//! | deeper than `deep` (4 u) inside | 0 |
//! | `-deep < d < 0` | rising linearly to `surface_rate` (0.8) |
//! | `0 <= d <= knee` (4 u) | `surface_rate + slope * d` (0.2 per u: skimming is affordable) |
//! | `d > knee` | `+ far_slope` per u beyond the knee (0.6), at most `cap` (7, 13 u out) |
//!
//! Tuning (package C5, by the Belt Up time trials in `ls-bots`
//! `tests/race_match.rs`): a Tinker skimming 4 u above the belt for a lap
//! at cruise speed takes about one hull (1.6 hp/s for 63 s); the open line
//! (6 u or more off the belt) costs 2.8 hp/s and more, several deaths a
//! lap, so it is slower than the belt line; a mixed line (the belt with
//! skimming dashes) is fastest. The dive from the belt to the cocoon costs
//! about 20 hp at full boost and less than the 30 % respawn hull from a
//! respawn at the own gate (so a racer who dies in the dive can still
//! finish). The knee separates skimming from open flight (one slope cannot
//! make the first affordable and the second deadly).
//!
//! The world applies it every tick to living ships as hull loss that
//! bypasses shields and gadgets (like The Squeeze's crushing) and credits
//! nobody (the killer is "the level"), scaled by the mode's
//! `ModeModule::radiation_scale` (default 1; Belt Up's rule set setting).
//! Numbers: `data/world/radiation.ron` ([`RadiationSettings`]).

use std::sync::OnceLock;

use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use ls_math::level::BeltSite;

/// `data/world/radiation.ron` (embedded, part of the data hash).
pub const RADIATION_RON: &str = include_str!("../../../data/world/radiation.ron");

/// The dose law (`data/world/radiation.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RadiationSettings {
    /// Depth inside the shelter (u) below which the rate is 0.
    pub deep: f32,
    /// Rate at the shelter's surface (hull/s).
    pub surface_rate: f32,
    /// Extra rate per u outside, up to `knee` (hull/s per u).
    pub slope: f32,
    /// Distance outside (u) from which `far_slope` applies.
    pub knee: f32,
    /// Extra rate per u beyond the knee (hull/s per u).
    pub far_slope: f32,
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
            let near = self.slope * d.min(self.knee);
            let far = self.far_slope * (d - self.knee).max(0.0);
            (self.surface_rate + near + far).min(self.cap)
        }
    }
}

/// Dose rate (hull/s, unscaled) at `p` on a belt level. What the HUD meter
/// shows (`ls-wasm` exports it).
pub fn dose_rate(belt: &BeltSite, settings: &RadiationSettings, p: Vec3) -> f32 {
    settings.rate_at_distance(belt.shelter_distance(p))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ls_math::level::{BeltSlot, BELT_WEDGES};
    use ls_math::Quat;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// World point at `azimuth` (prograde from +X), `radius` from the belt
    /// axis and `height` above the belt plane.
    pub(crate) fn belt_point(b: &BeltSite, azimuth: f32, radius: f32, height: f32) -> Vec3 {
        let (s, c) = ls_math::det::sin_cos(azimuth);
        b.center + Vec3::new(c * radius, height, -s * radius)
    }

    /// A belt with the §3.8 numbers around the origin (tests).
    pub(crate) fn plan_belt() -> BeltSite {
        let mut b = BeltSite {
            center: Vec3::ZERO,
            inner_radius: 95.0,
            outer_radius: 145.0,
            half_thickness: 12.0,
            cocoon_radius: 30.0,
            exit_radius: 6.0,
            gate_inner_radius: 87.0,
            gate_outer_radius: 153.0,
            slots: [BeltSlot {
                pos: Vec3::ZERO,
                orientation: Quat::IDENTITY,
            }; BELT_WEDGES],
            pickup_points: Vec::new(),
            boulders: Vec::new(),
        };
        for k in 0..BELT_WEDGES {
            let a = BeltSite::gate_azimuth(k) + 3.0f32.to_radians();
            b.slots[k] = BeltSlot {
                pos: belt_point(&b, a, 120.0, 0.0),
                orientation: Quat::from_axis_angle(Vec3::Y, a),
            };
        }
        b
    }

    #[test]
    fn rate_follows_the_plan_profile() {
        let s = RadiationSettings::builtin();
        assert_eq!(
            *s,
            RadiationSettings {
                deep: 4.0,
                surface_rate: 0.8,
                slope: 0.2,
                knee: 4.0,
                far_slope: 0.6,
                cap: 7.0
            }
        );
        assert_eq!(s.rate_at_distance(-10.0), 0.0, "deep inside");
        assert_eq!(s.rate_at_distance(-4.0), 0.0);
        assert!((s.rate_at_distance(-2.0) - 0.4).abs() < 1e-6, "half way in");
        assert_eq!(s.rate_at_distance(0.0), 0.8, "surface");
        assert!(
            (s.rate_at_distance(4.0) - 1.6).abs() < 1e-5,
            "skimming, the knee"
        );
        assert!((s.rate_at_distance(6.0) - 2.8).abs() < 1e-5, "open line");
        assert!((s.rate_at_distance(10.0) - 5.2).abs() < 1e-4, "far slope");
        assert!((s.rate_at_distance(12.9) - 6.94).abs() < 1e-4);
        assert_eq!(s.rate_at_distance(13.0), 7.0, "cap at 13 u");
        assert_eq!(s.rate_at_distance(200.0), 7.0);
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
        assert_eq!(dose_rate(&b, s, belt_point(&b, 0.7, 120.0, 0.0)), 0.0);
        assert_eq!(dose_rate(&b, s, b.center), 0.0);
        assert_eq!(dose_rate(&b, s, b.center + Vec3::new(0.0, 20.0, 0.0)), 0.0);
        // At the belt surface: 0.8 hp/s; 4 u above it (the skimming line): 1.6.
        assert_eq!(dose_rate(&b, s, belt_point(&b, 0.7, 120.0, 12.0)), 0.8);
        assert!((dose_rate(&b, s, belt_point(&b, 0.7, 120.0, 16.0)) - 1.6).abs() < 1e-4);
        // Far above: the cap.
        assert_eq!(dose_rate(&b, s, belt_point(&b, 0.7, 120.0, 60.0)), 7.0);
        // The dive from the inner edge to the cocoon in the belt plane:
        // hottest half way (32.5 u from both).
        let mid = dose_rate(&b, s, belt_point(&b, 0.7, 62.5, 0.0));
        assert_eq!(mid, 7.0, "{mid}");
    }

    /// Tuning targets (§3.8 as tuned by C5's time trials), integrated along
    /// straight lines at the given speeds: the dive at full straight boost
    /// costs a Tinker about 20 hp (§3.8 started from 35; C5 lowered it so a
    /// racer respawning at its own gate with the 30 % hull floor can still
    /// dive); skimming 4 u above the belt for a whole middle lap at cruise
    /// speed (12 u/s; boost does not last a lap in open space) kills a
    /// Tinker (100 hull) about once.
    #[test]
    fn tuning_targets() {
        let b = plan_belt();
        let s = RadiationSettings::builtin();
        let dt = ls_math::DT;
        // Dive: inner edge (95 u) to the cocoon (30 u) at 16.5 u/s.
        let (mut r, mut hp) = (95.0f32, 0.0f32);
        while r > 30.0 {
            hp += dose_rate(&b, s, belt_point(&b, 0.3, r, 0.0)) * dt;
            r -= 16.5 * dt;
        }
        assert!((15.0..28.0).contains(&hp), "dive costs {hp} hp");
        // Skim: 4 u above the belt's middle, one lap (754 u) at 12 u/s.
        let lap = std::f32::consts::TAU * 120.0;
        let secs = lap / 12.0;
        let skim = dose_rate(&b, s, belt_point(&b, 0.3, 120.0, 16.0)) * secs;
        assert!(
            (80.0..130.0).contains(&skim),
            "skimming lap costs {skim} hp"
        );
    }
}
