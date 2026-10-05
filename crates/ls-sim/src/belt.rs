//! The asteroid belt of a Belt archetype level as the simulation sees it
//! (`MatchSetup::belt`; `docs/milestone-4-plan.md` §3.8).
//!
//! Filled by `ls_game::setup` from `ls_worldgen::Generator::belt()`
//! (package A1); read by radiation ([`crate::radiation`], package A4) and
//! by Belt Up (`modes/belt_up.rs`, package B6: gates, slots, exit, boulders).
//! `None` on classic levels: no radiation, no race.
//!
//! Geometry: the belt is an annulus around the vertical axis through
//! [`BeltLevel::center`] (world +Y), between [`BeltLevel::inner_radius`] and
//! [`BeltLevel::outer_radius`], `half_thickness` above and below the belt
//! plane (`y = center.y`). Angles are measured about +Y counter-clockwise
//! seen from above (from +X toward -Z), which is the prograde racing
//! direction: [`BeltLevel::point`] maps (angle, radius, height) to a world
//! point. Wedge `k` spans `[k, k + 1) * wedge_angle`; gate `k` is the
//! vertical half-plane at `k * wedge_angle` from `gate_inner` to
//! `gate_outer`. The cocoon (a sphere of `cocoon_radius` at the centre)
//! shields like the belt and holds the exit (`exit_radius`).

use ls_math::{det, Vec3};
use serde::{Deserialize, Serialize};

use crate::world::SpawnPoint;

/// A drifting boulder's circular orbit about the belt axis. Positions are
/// a pure function of the tick (nothing is replicated):
/// angle `phase + speed / radius * t` (prograde), height `height` above
/// the belt plane, at `radius` from the axis ([`BoulderOrbit::pos_at`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoulderOrbit {
    /// Orbit radius about the belt axis (u).
    pub radius: f32,
    /// Height above the belt plane (u, negative below).
    pub height: f32,
    /// Angle at tick 0 (radians, prograde).
    pub phase: f32,
    /// Speed along the orbit (u/s, prograde).
    pub speed: f32,
    /// Boulder radius (u).
    pub size: f32,
    /// Tumble rate (rad/s) about `tumble_axis`.
    pub tumble: f32,
    /// Unit axis of the tumble.
    pub tumble_axis: Vec3,
}

/// The belt of a Belt archetype level (see the module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeltLevel {
    /// A point on the belt axis in the belt plane (the exit's centre).
    pub center: Vec3,
    pub inner_radius: f32,
    pub outer_radius: f32,
    /// Half the belt's thickness (Verticality: 8-18 u).
    pub half_thickness: f32,
    /// The dust cocoon around the exit (shields like the belt).
    pub cocoon_radius: f32,
    /// The exit sphere at the centre.
    pub exit_radius: f32,
    /// Wedges (and gates, slots): 8.
    pub wedges: u8,
    /// Radial extent of every gate.
    pub gate_inner: f32,
    pub gate_outer: f32,
    /// Start slot of each wedge, in wedge order (3 degrees past its gate,
    /// facing prograde).
    pub slots: Vec<SpawnPoint>,
    /// Drifting boulders (every wedge's copies included).
    pub boulders: Vec<BoulderOrbit>,
}

impl BeltLevel {
    /// Angle of one wedge (2 pi / wedges).
    pub fn wedge_angle(&self) -> f32 {
        std::f32::consts::TAU / self.wedges.max(1) as f32
    }

    /// Angle of gate `k`.
    pub fn gate_angle(&self, k: u8) -> f32 {
        self.wedge_angle() * k as f32
    }

    /// World point at `angle` (prograde from +X), `radius` from the axis
    /// and `height` above the belt plane.
    pub fn point(&self, angle: f32, radius: f32, height: f32) -> Vec3 {
        let (s, c) = det::sin_cos(angle);
        self.center + Vec3::new(c * radius, height, -s * radius)
    }

    /// `(angle in [0, 2 pi), radius, height)` of a world point (inverse of
    /// [`BeltLevel::point`]).
    pub fn polar(&self, p: Vec3) -> (f32, f32, f32) {
        let d = p - self.center;
        let radius = (d.x * d.x + d.z * d.z).sqrt();
        let mut angle = det::atan2(-d.z, d.x);
        if angle < 0.0 {
            angle += std::f32::consts::TAU;
        }
        (angle, radius, d.y)
    }

    /// Signed distance from `p` to the belt volume (an annulus of
    /// rectangular cross-section): negative inside.
    pub fn belt_distance(&self, p: Vec3) -> f32 {
        let d = p - self.center;
        let rho = (d.x * d.x + d.z * d.z).sqrt();
        let mid = 0.5 * (self.inner_radius + self.outer_radius);
        let half_width = 0.5 * (self.outer_radius - self.inner_radius);
        let qx = (rho - mid).abs() - half_width;
        let qy = d.y.abs() - self.half_thickness;
        let ox = qx.max(0.0);
        let oy = qy.max(0.0);
        (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0)
    }

    /// Signed distance from `p` to the shelter (belt or cocoon): negative
    /// inside either.
    pub fn shelter_distance(&self, p: Vec3) -> f32 {
        let cocoon = p.distance(self.center) - self.cocoon_radius;
        self.belt_distance(p).min(cocoon)
    }
}

impl BoulderOrbit {
    /// Centre of the boulder at tick `tick` on `belt`.
    pub fn pos_at(&self, belt: &BeltLevel, tick: u64) -> Vec3 {
        let t = tick as f32 * ls_math::DT;
        let angle = if self.radius > 0.0 {
            self.phase + self.speed / self.radius * t
        } else {
            self.phase
        };
        belt.point(det::wrap_angle(angle), self.radius, self.height)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ls_math::Quat;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// The §3.8 belt in a box centred at the origin (tests).
    pub(crate) fn plan_belt() -> BeltLevel {
        let mut b = BeltLevel {
            center: Vec3::ZERO,
            inner_radius: 95.0,
            outer_radius: 145.0,
            half_thickness: 12.0,
            cocoon_radius: 30.0,
            exit_radius: 6.0,
            wedges: 8,
            gate_inner: 87.0,
            gate_outer: 153.0,
            slots: Vec::new(),
            boulders: Vec::new(),
        };
        b.slots = (0..8u8)
            .map(|k| {
                let a = b.gate_angle(k) + 3.0f32.to_radians();
                SpawnPoint {
                    pos: b.point(a, 120.0, 0.0),
                    team: 0,
                    orientation: Quat::from_axis_angle(Vec3::Y, a),
                }
            })
            .collect();
        b
    }

    #[test]
    fn polar_round_trips_and_runs_prograde() {
        let b = plan_belt();
        for i in 0..32 {
            let a = i as f32 * 0.19;
            let p = b.point(a, 120.0, -3.0);
            let (a2, r, h) = b.polar(p);
            assert!((a2 - a).abs() < 1e-4, "{a} {a2}");
            assert!((r - 120.0).abs() < 1e-3 && (h + 3.0).abs() < 1e-5);
        }
        // Counter-clockwise seen from above: +X, then -Z.
        assert!((b.point(0.0, 1.0, 0.0) - Vec3::X).length() < 1e-6);
        assert!((b.point(std::f32::consts::FRAC_PI_2, 1.0, 0.0) - Vec3::NEG_Z).length() < 1e-6);
        assert_eq!(b.gate_angle(2), std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn belt_distance_is_signed() {
        let b = plan_belt();
        assert_eq!(b.belt_distance(b.point(1.0, 120.0, 0.0)), -12.0);
        assert!((b.belt_distance(b.point(1.0, 120.0, 20.0)) - 8.0).abs() < 1e-4);
        assert!((b.belt_distance(b.point(1.0, 160.0, 0.0)) - 15.0).abs() < 1e-3);
        assert!((b.belt_distance(b.point(1.0, 90.0, 0.0)) - 5.0).abs() < 1e-3);
        // Diagonal outside the corner.
        let d = b.belt_distance(b.point(1.0, 148.0, 16.0));
        assert!((d - 5.0).abs() < 1e-3, "{d}");
        assert!(
            b.shelter_distance(Vec3::new(0.0, 5.0, 0.0)) < -20.0,
            "cocoon"
        );
    }

    #[test]
    fn boulders_orbit_prograde() {
        let b = plan_belt();
        let o = BoulderOrbit {
            radius: 120.0,
            height: 2.0,
            phase: 0.0,
            speed: 3.0,
            size: 3.0,
            tumble: 0.5,
            tumble_axis: Vec3::Y,
        };
        let p0 = o.pos_at(&b, 0);
        let p1 = o.pos_at(&b, 60);
        assert!((p0.distance(p1) - 3.0).abs() < 0.01);
        assert!(b.polar(p1).0 > b.polar(p0).0);
    }
}
