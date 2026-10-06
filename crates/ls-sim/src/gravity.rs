//! Gravity wells (M4, Gravity Chaos; `docs/milestone-4-plan.md` §3.3 and
//! §3.8 "Wells").
//!
//! A level has static wells (generator sites, [`WellSite`]: a dense rock
//! core in a carved hollow, or a belt's shepherd moon) whose strength and
//! range follow the level's Gravity Chaos slider, and the world keeps
//! dynamic wells ([`DynamicWell`]: position, strength, range, start and end
//! tick, owner) that modules add at run time (the Paperweight, The
//! Architect's Gravity Chaos+ stroke).
//!
//! Force law of every well: a softened inverse-square pull toward its
//! centre, `a = G r / (r^2 + s^2)^1.5`, tapered smoothly to zero at the
//! well's range by `(1 - r^2 / R^2)^2`. `G` is chosen so the untapered law
//! peaks at the well's `peak_accel` (at `r = s / sqrt 2`). Only IEEE-exact
//! maths (`sqrt`), so the client predictor reproduces it bit for bit.
//!
//! Who is pulled (design doc: "gravity wells bend missiles and debris, but
//! energy bolts and kinetic rounds fly straight"): ships (through
//! [`crate::ship::MoveEnv`]; the total is clamped to
//! [`GravitySettings::ship_clamp`] of the ship type's per-axis thrust
//! acceleration, so stacked wells never trap a ship), debris, and
//! projectiles whose weapon data says `gravity: true` (ordnance, mines,
//! Seed Grenade, Swarm blobs); their total is clamped to
//! [`GravitySettings::body_clamp`].
//!
//! Numbers: `data/world/gravity.ron` ([`GravitySettings`]).

use std::sync::OnceLock;

use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::ship::PlayerId;

/// `max_r r / (r^2 + 1)^1.5 = 2 / (3 sqrt 3)`: the peak of the untapered
/// law for `s = 1` (scales with `1 / s^2`).
const PEAK_FACTOR: f32 = 0.384_900_18;

/// Tuning of the well field (`data/world/gravity.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GravitySettings {
    /// Peak pull (u/s^2) of a static well of strength 1 (a site's
    /// `strength` scales it; the generator sets `strength` to Gravity Chaos
    /// / 100 and the range to 20-40 u).
    pub max_peak_accel: f32,
    /// Softening length `s` (u) of every well: the pull peaks at
    /// `s / sqrt 2` from the centre and falls to zero at the centre.
    pub softening: f32,
    /// The total pull on a ship is clamped to this share of its type's
    /// per-axis thrust acceleration (`ShipTuning::thrust_accel`).
    pub ship_clamp: f32,
    /// The total pull on debris and flagged projectiles is clamped to this
    /// (u/s^2).
    pub body_clamp: f32,
    /// Resting debris wakes up when the pull on it exceeds this (u/s^2).
    pub wake_accel: f32,
    /// On a ship axis without input the dampers aim at the drift speed
    /// `pull * drift_secs` (at most the cruise cap) instead of 0, so an
    /// idle or coasting ship falls toward the well.
    pub drift_secs: f32,
}

impl GravitySettings {
    /// The embedded `data/world/gravity.ron`.
    pub fn builtin() -> &'static GravitySettings {
        static S: OnceLock<GravitySettings> = OnceLock::new();
        S.get_or_init(|| {
            ron::from_str(GRAVITY_RON).expect("data/world/gravity.ron is valid (tested)")
        })
    }

    /// Peak pull (u/s^2) of a static well site (`max_peak_accel x strength`,
    /// strength clamped to 0..=1).
    pub fn site_peak(&self, site: &WellSite) -> f32 {
        let k = if site.strength.is_nan() {
            0.0
        } else {
            site.strength.clamp(0.0, 1.0)
        };
        self.max_peak_accel * k
    }
}

/// `data/world/gravity.ron` (embedded, part of the data hash).
pub const GRAVITY_RON: &str = include_str!("../../../data/world/gravity.ron");

/// A static well of the level (generator site, or a belt's shepherd moon;
/// A1's contract type): its `strength` (0-1) scales
/// [`GravitySettings::max_peak_accel`], its `range` ends the pull. Filled
/// by `ls_game::setup` from `ls_worldgen::Generator::gravity_wells()`.
pub use ls_math::level::WellSite;

/// A well added at run time by a module (Paperweight, Architect stroke).
/// Active from `start_tick` (inclusive) to `end_tick` (exclusive); the
/// world drops it after `end_tick`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DynamicWell {
    /// Caller-chosen key (e.g. the entity id of the Paperweight), so a
    /// module can remove its well early ([`crate::Command::RemoveWell`]).
    pub key: u32,
    pub pos: Vec3,
    /// Peak pull (u/s^2) of the untapered law.
    pub peak_accel: f32,
    /// Range (u): no pull beyond it.
    pub range: f32,
    pub start_tick: u64,
    pub end_tick: u64,
    /// Who placed it (presentation only: a well pulls everyone, its owner too).
    pub owner: Option<PlayerId>,
}

impl DynamicWell {
    /// Whether the well pulls at tick `now`.
    pub fn active(&self, now: u64) -> bool {
        self.start_tick <= now && now < self.end_tick
    }
}

/// Pull of one well (peak `peak`, range `range`, softening `s`) at a point
/// `d = centre - point` away from its centre.
pub fn well_accel(d: Vec3, peak: f32, range: f32, s: f32) -> Vec3 {
    if peak <= 0.0 || range <= 0.0 {
        return Vec3::ZERO;
    }
    let r2 = d.length_squared();
    let range2 = range * range;
    if r2 >= range2 {
        return Vec3::ZERO;
    }
    let s2 = s * s;
    let g = peak * s2 / PEAK_FACTOR;
    let q = r2 + s2;
    let taper = 1.0 - r2 / range2;
    d * (g * taper * taper / (q * q.sqrt()))
}

/// Clamps an acceleration to length `max`.
pub fn clamp_accel(a: Vec3, max: f32) -> Vec3 {
    let l2 = a.length_squared();
    if l2 <= max * max {
        a
    } else if max <= 0.0 {
        Vec3::ZERO
    } else {
        a * (max / l2.sqrt())
    }
}

/// A static well with its slider strength resolved.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StaticWell {
    pub pos: Vec3,
    pub core_radius: f32,
    pub peak_accel: f32,
    pub range: f32,
}

/// Every well of a match at one moment: what the simulation and the
/// client predictor evaluate ([`crate::World::well_field`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WellField {
    pub statics: Vec<StaticWell>,
    pub dynamics: Vec<DynamicWell>,
    pub settings: Option<GravitySettings>,
}

impl WellField {
    /// The static wells of a level: every site with its own strength and
    /// range; none at Gravity Chaos `slider` 0, and sites without strength
    /// or range are left out.
    pub fn new(sites: &[WellSite], slider: u8, settings: &GravitySettings) -> WellField {
        let statics = if slider == 0 {
            Vec::new()
        } else {
            sites
                .iter()
                .map(|s| StaticWell {
                    pos: s.pos,
                    core_radius: s.core_radius,
                    peak_accel: settings.site_peak(s),
                    range: s.range,
                })
                .filter(|w| w.peak_accel > 0.0 && w.range > 0.0)
                .collect()
        };
        WellField {
            statics,
            dynamics: Vec::new(),
            settings: Some(*settings),
        }
    }

    /// Whether no well can pull anything (the simulation then skips every
    /// gravity step, which keeps well-free replays bit-identical).
    pub fn is_empty(&self) -> bool {
        self.statics.is_empty() && self.dynamics.is_empty()
    }

    fn settings(&self) -> &GravitySettings {
        self.settings
            .as_ref()
            .unwrap_or_else(|| GravitySettings::builtin())
    }

    /// Unclamped total pull at `p` at tick `now` (statics in order, then
    /// the active dynamic wells in order).
    pub fn accel_at(&self, p: Vec3, now: u64) -> Vec3 {
        let s = self.settings().softening;
        let mut a = Vec3::ZERO;
        for w in &self.statics {
            a += well_accel(w.pos - p, w.peak_accel, w.range, s);
        }
        for w in &self.dynamics {
            if w.active(now) {
                a += well_accel(w.pos - p, w.peak_accel, w.range, s);
            }
        }
        a
    }

    /// Pull on a ship at `p` whose type accelerates at `thrust_accel` per
    /// axis: the total clamped to [`GravitySettings::ship_clamp`] of it.
    pub fn ship_accel(&self, p: Vec3, now: u64, thrust_accel: f32) -> Vec3 {
        self.ship_accel_scaled(p, now, thrust_accel, 1.0)
    }

    /// [`WellField::ship_accel`] with every well's pull scaled by `scale`
    /// before the clamp (Gravity coffee break dims the wells:
    /// `EventEffects::well_scale`). A scale of 1 is the unscaled pull
    /// exactly.
    pub fn ship_accel_scaled(&self, p: Vec3, now: u64, thrust_accel: f32, scale: f32) -> Vec3 {
        if self.is_empty() {
            return Vec3::ZERO;
        }
        let max = self.settings().ship_clamp * thrust_accel;
        clamp_accel(scaled(self.accel_at(p, now), scale), max)
    }

    /// The movement environment of a ship at `p` ([`crate::ship::MoveEnv`]):
    /// its clamped pull, the level's `up` and the drift time. What the
    /// world and the client predictor hand `simulate_movement_env`.
    pub fn move_env(
        &self,
        p: Vec3,
        now: u64,
        tuning: &crate::ShipTuning,
        up: Vec3,
    ) -> crate::ship::MoveEnv {
        self.move_env_scaled(p, now, tuning, up, 1.0)
    }

    /// [`WellField::move_env`] with the wells' pull scaled by `scale`
    /// (Gravity coffee break).
    pub fn move_env_scaled(
        &self,
        p: Vec3,
        now: u64,
        tuning: &crate::ShipTuning,
        up: Vec3,
        scale: f32,
    ) -> crate::ship::MoveEnv {
        crate::ship::MoveEnv {
            gravity: self.ship_accel_scaled(p, now, tuning.thrust_accel, scale),
            up,
            drift_secs: self.settings().drift_secs,
        }
    }

    /// Pull on debris or a flagged projectile at `p` (clamped to
    /// [`GravitySettings::body_clamp`]).
    pub fn body_accel(&self, p: Vec3, now: u64) -> Vec3 {
        self.body_accel_scaled(p, now, 1.0)
    }

    /// [`WellField::body_accel`] with the wells' pull scaled by `scale`.
    pub fn body_accel_scaled(&self, p: Vec3, now: u64, scale: f32) -> Vec3 {
        if self.is_empty() {
            return Vec3::ZERO;
        }
        clamp_accel(
            scaled(self.accel_at(p, now), scale),
            self.settings().body_clamp,
        )
    }
}

/// `a * scale`, and `a` itself at scale 1 (bit-identical to the unscaled
/// field).
fn scaled(a: Vec3, scale: f32) -> Vec3 {
    if scale == 1.0 {
        a
    } else {
        a * scale.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    pub(crate) fn site(pos: Vec3, strength: f32, range: f32) -> WellSite {
        WellSite {
            pos,
            core_radius: 2.0,
            hollow_radius: 8.0,
            strength,
            range,
            kind: ls_math::level::WellKind::Core,
        }
    }

    #[test]
    fn data_file_parses_and_sites_scale_the_peak() {
        let s = GravitySettings::builtin();
        // §3.3: the peak pull at most 40 % of the Tinker's per-axis thrust
        // acceleration at slider 100 (strength 1).
        let tinker = crate::ShipTuning::tinker();
        assert_eq!(s.max_peak_accel, 0.4 * tinker.thrust_accel);
        assert_eq!(s.ship_clamp, 0.4);
        assert_eq!(s.site_peak(&site(Vec3::ZERO, 1.0, 40.0)), 12.0);
        assert_eq!(s.site_peak(&site(Vec3::ZERO, 0.5, 40.0)), 6.0);
        assert_eq!(s.site_peak(&site(Vec3::ZERO, 7.0, 40.0)), 12.0);
        assert_eq!(s.site_peak(&site(Vec3::ZERO, f32::NAN, 40.0)), 0.0);
        let f = WellField::new(
            &[site(Vec3::ZERO, 1.0, 30.0), site(Vec3::X, 0.0, 30.0)],
            100,
            s,
        );
        assert_eq!(f.statics.len(), 1, "a site without strength is left out");
        assert_eq!(f.statics[0].range, 30.0);
        assert!(WellField::new(&[site(Vec3::ZERO, 1.0, 30.0)], 0, s).is_empty());
    }

    #[test]
    fn force_law_peaks_tapers_and_points_inward() {
        let s = 3.0;
        let peak = 10.0;
        let range = 30.0;
        // Peak of the untapered law at s / sqrt 2.
        let at = |r: f32| well_accel(Vec3::new(r, 0.0, 0.0), peak, range, s).x;
        let rp = s / 2.0f32.sqrt();
        let taper = (1.0 - rp * rp / (range * range)).powi(2);
        assert!((at(rp) - peak * taper).abs() < 1e-3, "{}", at(rp));
        let mut best = 0.0f32;
        for i in 1..3000 {
            best = best.max(at(i as f32 * 0.01));
        }
        assert!(best <= peak + 1e-3, "{best}");
        assert_eq!(at(0.0), 0.0, "no pull at the centre");
        assert_eq!(at(range), 0.0, "none at the range");
        assert_eq!(at(range + 5.0), 0.0);
        assert!(
            at(range - 0.01) > 0.0 && at(range - 0.01) < 1e-4,
            "smooth taper"
        );
        // Pulls toward the centre: d = centre - point.
        let a = well_accel(Vec3::new(-5.0, 2.0, 1.0), peak, range, s);
        assert!(a.dot(Vec3::new(-5.0, 2.0, 1.0)) > 0.0);
        // Monotone falloff past the peak.
        let mut last = at(rp);
        for i in 0..250 {
            let v = at(rp + 0.1 * (i + 1) as f32);
            assert!(v <= last + 1e-6);
            last = v;
        }
    }

    #[test]
    fn dynamic_wells_start_and_end_on_their_ticks() {
        let settings = GravitySettings::builtin();
        let mut f = WellField::new(&[], 0, settings);
        assert!(f.is_empty());
        f.dynamics.push(DynamicWell {
            key: 1,
            pos: Vec3::ZERO,
            peak_accel: 9.0,
            range: 16.0,
            start_tick: 100,
            end_tick: 460,
            owner: None,
        });
        let p = Vec3::new(3.0, 0.0, 0.0);
        assert_eq!(f.accel_at(p, 99), Vec3::ZERO);
        assert!(f.accel_at(p, 100).x < 0.0);
        assert!(f.accel_at(p, 459).x < 0.0);
        assert_eq!(f.accel_at(p, 460), Vec3::ZERO);
    }

    #[test]
    fn stacked_wells_are_clamped_for_ships() {
        let settings = GravitySettings::builtin();
        let mut f = WellField::new(&[], 0, settings);
        for i in 0..3 {
            f.dynamics.push(DynamicWell {
                key: i,
                pos: Vec3::new(2.0, 0.0, 0.0),
                peak_accel: 9.0,
                range: 16.0,
                start_tick: 0,
                end_tick: u64::MAX,
                owner: None,
            });
        }
        let raw = f.accel_at(Vec3::ZERO, 5).length();
        assert!(raw > 20.0, "{raw}");
        let tinker = crate::ShipTuning::tinker();
        let a = f.ship_accel(Vec3::ZERO, 5, tinker.thrust_accel).length();
        assert!((a - 0.4 * tinker.thrust_accel).abs() < 1e-4, "{a}");
        assert!(f.body_accel(Vec3::ZERO, 5).length() <= settings.body_clamp + 1e-4);
    }
}
