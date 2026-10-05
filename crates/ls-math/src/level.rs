//! Level contracts shared by the generator, the simulation and the client:
//! level archetypes, gravity well sites and the asteroid belt.
//!
//! `ls-worldgen` produces these from a share code (generator version 4),
//! `ls-game` copies them into the match setup, and `ls-sim` reads them for
//! gravity, radiation and Belt Up. They live in `ls-math` so those crates
//! need not depend on each other (see `docs/architecture/contracts.md`,
//! "Levels").
//!
//! # Belt angles
//!
//! The belt lies in a horizontal plane (`y = center.y`) around the vertical
//! axis through [`BeltSite::center`]. The *azimuth* of a point is measured
//! about +Y with the right-hand rule: azimuth 0 points along +X, azimuth
//! 90 degrees along -Z. Increasing azimuth is *prograde*: counter-clockwise
//! seen from above (from +Y looking down), the racing direction. Rotating a
//! point by `a` about the axis is `Quat::from_axis_angle(Vec3::Y, a)` about
//! the centre.

use serde::{Deserialize, Serialize};

use crate::{det, Quat, Vec3, DT};

/// The kind of level a share code builds (generator version 4 codes carry
/// it in 3 bits). Append only: the index is part of the share code.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub enum Archetype {
    /// Caves, mine shafts and open sky with floating islands (every level
    /// before generator version 4).
    #[default]
    Classic,
    /// A flat ring of asteroids around an open centre with 8-fold symmetry
    /// (Belt Up; `docs/milestone-4-plan.md` §3.8).
    Belt,
}

impl Archetype {
    /// Every archetype, in index order.
    pub const ALL: [Archetype; 2] = [Archetype::Classic, Archetype::Belt];
    /// Largest index the share code can hold (3 bits).
    pub const MAX_INDEX: u8 = 7;

    /// Index in the share code.
    pub const fn index(self) -> u8 {
        match self {
            Archetype::Classic => 0,
            Archetype::Belt => 1,
        }
    }

    /// The archetype with share-code index `i`, if this build knows it.
    pub const fn from_index(i: u8) -> Option<Archetype> {
        match i {
            0 => Some(Archetype::Classic),
            1 => Some(Archetype::Belt),
            _ => None,
        }
    }

    /// Lower-case name (`"classic"`, `"belt"`), as tools and the web client
    /// print it.
    pub const fn name(self) -> &'static str {
        match self {
            Archetype::Classic => "classic",
            Archetype::Belt => "belt",
        }
    }
}

/// What holds a gravity well's core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WellKind {
    /// A dense rock core in a carved hollow (caverns, mine shafts).
    #[default]
    Core,
    /// A floating stone in open space (open-sky levels).
    Floating,
    /// A shepherd moon of the asteroid belt (one per wedge).
    Shepherd,
}

/// A static gravity well of a level (Gravity Chaos; `Generator::gravity_wells`).
///
/// The generator places the core as rock (hard material) and keeps the
/// hollow around it open; the simulation turns `strength` and `range` into
/// a pull (A4: the softened inverse-square law of the M4 plan §3.3, with its
/// tuning in `data/world/gravity.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WellSite {
    /// Centre of the core.
    pub pos: Vec3,
    /// Radius of the rock core (u).
    pub core_radius: f32,
    /// Radius of the open hollow around the core (u); the space between
    /// `core_radius` and this is free of generated rock.
    pub hollow_radius: f32,
    /// Strength in `0..=1`: the share of the largest pull the rules allow
    /// (Gravity Chaos / 100; 1 at slider 100).
    pub strength: f32,
    /// Distance from `pos` at which the pull has tapered to zero (u).
    pub range: f32,
    pub kind: WellKind,
}

/// Number of wedges (start slots, gates) of the asteroid belt.
pub const BELT_WEDGES: usize = 8;
/// Angle of one belt wedge (45 degrees, radians).
pub const BELT_WEDGE_ANGLE: f32 = core::f32::consts::FRAC_PI_4;

/// A start slot of the belt.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BeltSlot {
    /// Position in the belt plane.
    pub pos: Vec3,
    /// Facing prograde (along the belt), level, no roll.
    pub orientation: Quat,
}

/// A drifting boulder: it orbits the belt axis prograde on a circle, its
/// position a pure function of the tick ([`BoulderOrbit::pos_at`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoulderOrbit {
    /// Orbit radius about the axis (u).
    pub radius: f32,
    /// Height above the belt plane (u; negative below).
    pub height: f32,
    /// Azimuth at tick 0 (rad).
    pub phase: f32,
    /// Prograde angular speed (rad/s, positive).
    pub angular_speed: f32,
    /// Radius of the boulder (u).
    pub size: f32,
    /// Unit tumble axis (presentation and contact normals).
    pub tumble_axis: Vec3,
    /// Tumble rate about `tumble_axis` (rad/s).
    pub tumble_speed: f32,
}

impl BoulderOrbit {
    /// Azimuth at `tick` (rad, wrapped to `-pi..pi`).
    pub fn azimuth_at(&self, tick: u64) -> f32 {
        let t = tick as f32 * DT;
        det::wrap_angle(self.phase + self.angular_speed * t)
    }

    /// Centre of the boulder at `tick`, for a belt centred at `center`.
    pub fn pos_at(&self, center: Vec3, tick: u64) -> Vec3 {
        let (s, c) = det::sin_cos(self.azimuth_at(tick));
        center + Vec3::new(self.radius * c, self.height, -self.radius * s)
    }

    /// Velocity at any tick (u/s): tangential, prograde.
    pub fn velocity_at(&self, tick: u64) -> Vec3 {
        let (s, c) = det::sin_cos(self.azimuth_at(tick));
        let v = self.radius * self.angular_speed;
        Vec3::new(-v * s, 0.0, -v * c)
    }

    /// Tumble orientation at `tick`.
    pub fn tumble_at(&self, tick: u64) -> Quat {
        let t = tick as f32 * DT;
        Quat::from_axis_angle(
            self.tumble_axis,
            det::wrap_angle(self.tumble_speed * t + self.phase),
        )
    }
}

/// The asteroid belt of a [`Archetype::Belt`] level (`Generator::belt`):
/// the ring, its 8 start slots and gates, the exit, pickup points and
/// drifting boulders. Shepherd moons are in `Generator::gravity_wells`
/// ([`WellKind::Shepherd`]).
///
/// Course: slot `k` sits 3 degrees past gate `k`; a racer flies prograde
/// through gates `k + 1, ..., k + 7` and then its own gate `k` (one lap),
/// then dives to the exit at the centre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeltSite {
    /// The belt's centre: on the vertical axis, in the belt plane. Also the
    /// centre of the cocoon and of the exit.
    pub center: Vec3,
    /// Inner radius of the belt annulus (u).
    pub inner_radius: f32,
    /// Outer radius of the belt annulus (u).
    pub outer_radius: f32,
    /// Half thickness of the belt slab about its plane (u).
    pub half_thickness: f32,
    /// Radius of the dust cocoon around the exit (u; no rock inside).
    pub cocoon_radius: f32,
    /// Radius of the exit sphere (u).
    pub exit_radius: f32,
    /// Gates reach from this radius ... (u)
    pub gate_inner_radius: f32,
    /// ... to this one, over the full height of the level (u).
    pub gate_outer_radius: f32,
    /// Start slot `k` (3 degrees past gate `k`).
    pub slots: [BeltSlot; BELT_WEDGES],
    /// Pickup points, wedge-major (`wedge * per_wedge + i`); every wedge
    /// holds the same points rotated.
    pub pickup_points: Vec<Vec3>,
    /// Drifting boulders, wedge-major like the pickup points.
    pub boulders: Vec<BoulderOrbit>,
}

impl BeltSite {
    /// Azimuth of gate `k` (rad): `k * 45` degrees.
    pub fn gate_azimuth(k: usize) -> f32 {
        (k % BELT_WEDGES) as f32 * BELT_WEDGE_ANGLE
    }

    /// Azimuth of `p` about the belt axis (rad, `0..2 pi`, prograde positive).
    pub fn azimuth(&self, p: Vec3) -> f32 {
        let d = p - self.center;
        let a = det::atan2(-d.z, d.x);
        if a < 0.0 {
            a + core::f32::consts::TAU
        } else {
            a
        }
    }

    /// Distance of `p` from the belt axis (u).
    pub fn axis_distance(&self, p: Vec3) -> f32 {
        let d = p - self.center;
        Vec3::new(d.x, 0.0, d.z).length()
    }

    /// The wedge `p` lies in: wedge `k` spans azimuths from gate `k` to
    /// gate `k + 1`.
    pub fn wedge_of(&self, p: Vec3) -> usize {
        let w = (self.azimuth(p) / BELT_WEDGE_ANGLE) as usize;
        w.min(BELT_WEDGES - 1)
    }

    /// Signed distance from `p` to the belt volume (the annulus slab
    /// `inner_radius..outer_radius`, `|y - plane| <= half_thickness`):
    /// negative inside, positive outside.
    pub fn belt_distance(&self, p: Vec3) -> f32 {
        let r = self.axis_distance(p);
        let mid = 0.5 * (self.inner_radius + self.outer_radius);
        let half_w = 0.5 * (self.outer_radius - self.inner_radius);
        let qx = (r - mid).abs() - half_w;
        let qy = (p.y - self.center.y).abs() - self.half_thickness;
        let outside = Vec3::new(qx.max(0.0), qy.max(0.0), 0.0).length();
        outside + qx.max(qy).min(0.0)
    }

    /// Signed distance from `p` to the sheltered volume: the belt or the
    /// cocoon, whichever is nearer (negative inside either). Radiation
    /// (A4) reads its dose from this.
    pub fn shelter_distance(&self, p: Vec3) -> f32 {
        let cocoon = p.distance(self.center) - self.cocoon_radius;
        self.belt_distance(p).min(cocoon)
    }

    /// Whether `p` is inside the exit sphere.
    pub fn in_exit(&self, p: Vec3) -> bool {
        p.distance(self.center) <= self.exit_radius
    }

    /// Whether the movement segment `a -> b` crosses gate `k` prograde: it
    /// passes the gate's half-plane from behind to in front, between
    /// [`BeltSite::gate_inner_radius`] and
    /// [`BeltSite::gate_outer_radius`] from the axis (any height).
    pub fn crosses_gate(&self, k: usize, a: Vec3, b: Vec3) -> bool {
        let (s, c) = det::sin_cos(Self::gate_azimuth(k));
        // Radial direction of the gate and the prograde normal.
        let radial = Vec3::new(c, 0.0, -s);
        let normal = Vec3::new(-s, 0.0, -c);
        let sa = (a - self.center).dot(normal);
        let sb = (b - self.center).dot(normal);
        if !(sa < 0.0 && sb >= 0.0) {
            return false;
        }
        let t = sa / (sa - sb);
        let q = a + (b - a) * t;
        let r = (q - self.center).dot(radial);
        r >= self.gate_inner_radius && r <= self.gate_outer_radius
    }

    /// `p` rotated by `wedges` wedges (45 degrees each, prograde) about the
    /// belt axis.
    pub fn rotate(&self, p: Vec3, wedges: i32) -> Vec3 {
        let a = wedges.rem_euclid(BELT_WEDGES as i32) as f32 * BELT_WEDGE_ANGLE;
        let (s, c) = det::sin_cos(a);
        let d = p - self.center;
        self.center + Vec3::new(c * d.x + s * d.z, d.y, -s * d.x + c * d.z)
    }

    /// `p` mirrored in the belt plane.
    pub fn mirror(&self, p: Vec3) -> Vec3 {
        Vec3::new(p.x, 2.0 * self.center.y - p.y, p.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn belt() -> BeltSite {
        let slot = BeltSlot {
            pos: Vec3::ZERO,
            orientation: Quat::IDENTITY,
        };
        BeltSite {
            center: Vec3::new(160.0, 48.0, 160.0),
            inner_radius: 95.0,
            outer_radius: 145.0,
            half_thickness: 12.0,
            cocoon_radius: 30.0,
            exit_radius: 6.0,
            gate_inner_radius: 87.0,
            gate_outer_radius: 153.0,
            slots: [slot; BELT_WEDGES],
            pickup_points: Vec::new(),
            boulders: Vec::new(),
        }
    }

    #[test]
    fn archetype_indices_round_trip() {
        for a in Archetype::ALL {
            assert_eq!(Archetype::from_index(a.index()), Some(a));
        }
        assert_eq!(Archetype::from_index(2), None);
        assert_eq!(Archetype::default(), Archetype::Classic);
    }

    #[test]
    fn azimuth_is_prograde_counter_clockwise_from_above() {
        let b = belt();
        let c = b.center;
        assert!(b.azimuth(c + Vec3::new(100.0, 0.0, 0.0)).abs() < 1e-6);
        let quarter = b.azimuth(c + Vec3::new(0.0, 0.0, -100.0));
        assert!((quarter - core::f32::consts::FRAC_PI_2).abs() < 1e-6);
        // rotate() by 2 wedges turns +X into -Z.
        let p = b.rotate(c + Vec3::new(100.0, 3.0, 0.0), 2);
        assert!(p.distance(c + Vec3::new(0.0, 3.0, -100.0)) < 1e-3, "{p:?}");
        assert_eq!(b.wedge_of(c + Vec3::new(100.0, 0.0, -1.0)), 0);
        assert_eq!(b.wedge_of(c + Vec3::new(100.0, 0.0, 1.0)), 7);
    }

    #[test]
    fn gate_crossing_is_prograde_and_bounded() {
        let b = belt();
        let c = b.center;
        // Gate 0 lies along +X; prograde crosses it from +z to -z.
        let a = c + Vec3::new(120.0, 0.0, 1.0);
        let e = c + Vec3::new(120.0, 30.0, -1.0);
        assert!(b.crosses_gate(0, a, e));
        assert!(!b.crosses_gate(0, e, a), "retrograde");
        // Too far in or out, and the opposite half-plane, do not count.
        assert!(!b.crosses_gate(
            0,
            c + Vec3::new(50.0, 0.0, 1.0),
            c + Vec3::new(50.0, 0.0, -1.0)
        ));
        assert!(!b.crosses_gate(
            0,
            c + Vec3::new(160.0, 0.0, 1.0),
            c + Vec3::new(160.0, 0.0, -1.0)
        ));
        assert!(!b.crosses_gate(4, a, e));
        // Every gate, rotated.
        for k in 0..BELT_WEDGES {
            let (a2, e2) = (b.rotate(a, k as i32), b.rotate(e, k as i32));
            assert!(b.crosses_gate(k, a2, e2), "gate {k}");
            assert!(!b.crosses_gate((k + 1) % BELT_WEDGES, a2, e2), "gate {k}");
        }
    }

    #[test]
    fn belt_and_shelter_distances() {
        let b = belt();
        let c = b.center;
        assert_eq!(b.belt_distance(c + Vec3::new(120.0, 0.0, 0.0)), -12.0);
        assert!((b.belt_distance(c + Vec3::new(120.0, 16.0, 0.0)) - 4.0).abs() < 1e-4);
        assert!((b.belt_distance(c + Vec3::new(0.0, 0.0, 150.0)) - 5.0).abs() < 1e-4);
        assert!((b.shelter_distance(c) + 30.0).abs() < 1e-4);
        assert!(b.in_exit(c + Vec3::new(5.0, 0.0, 0.0)));
        assert!(!b.in_exit(c + Vec3::new(7.0, 0.0, 0.0)));
    }

    #[test]
    fn boulders_orbit_prograde() {
        let o = BoulderOrbit {
            radius: 120.0,
            height: 2.0,
            phase: 0.0,
            angular_speed: 3.0 / 120.0,
            size: 3.0,
            tumble_axis: Vec3::Y,
            tumble_speed: 0.5,
        };
        let c = Vec3::new(160.0, 48.0, 160.0);
        let p0 = o.pos_at(c, 0);
        let p1 = o.pos_at(c, 60);
        assert!(p0.distance(c + Vec3::new(120.0, 2.0, 0.0)) < 1e-4);
        // One second later: 3 u along the orbit, towards -z (prograde).
        assert!((p1.distance(p0) - 3.0).abs() < 0.01);
        assert!(p1.z < p0.z);
        assert!((o.velocity_at(0).length() - 3.0).abs() < 1e-4);
        assert!(o.velocity_at(0).z < 0.0);
    }
}
