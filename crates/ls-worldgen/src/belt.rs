//! The asteroid belt level archetype ([`Archetype::Belt`], generator version
//! 4; M4 plan §3.8): a flat ring of static voxel asteroids around an open
//! centre, built for the racing mode Belt Up.
//!
//! # Layout
//!
//! The level box is [`crate::BELT_SIZE_CHUNKS`] (320 x 96 x 320 u). The belt
//! is the annulus [`BELT_INNER_RADIUS`]..[`BELT_OUTER_RADIUS`] about the
//! vertical axis through the box centre, in the horizontal plane at mid
//! height (y = 48, a cell boundary), [`BeltSite::half_thickness`] thick on
//! each side (Verticality: 8 to 18 u). The centre holds the cocoon (no rock
//! within [`COCOON_RADIUS`]) and the exit ([`EXIT_RADIUS`]).
//!
//! # Symmetry
//!
//! One 45-degree wedge is generated from its own random stream
//! ([`BELT_SALT`], "BELT_GEN"), with its asteroids on the upper side of the
//! plane. Every asteroid is then copied 16 times: rotated about the axis by
//! every multiple of 45 degrees (rotation constants through
//! [`ls_math::det`]) and mirrored in the belt plane. Everything else in a
//! wedge (start slot, gate, pickup points, boulder orbits, shepherd moon)
//! sits at the same relative place in every wedge, in the belt plane where
//! it has no mirror twin. The field is therefore symmetric to rounding
//! (well within 1e-4 u); the voxel grid samples rotated wedges at other
//! sub-cell offsets, so meshes differ by less than a cell.
//!
//! # Wedge contents
//!
//! - A **clear corridor**: a seeded line through the wedge in the belt plane,
//!   meandering in radius (`r(u) = 120 + a sin 2 pi (u - u0) + b sin 4 pi (u - u0)`
//!   over the wedge fraction `u`, periodic, through the start slot), kept free
//!   of asteroids [`CORRIDOR_CLEARANCE`] around its centre line, so a belt-only
//!   line always exists.
//! - The **start slot** 3 degrees past the wedge's gate at radius 120 u,
//!   facing prograde, with [`SLOT_CLEARANCE`] of free space, and a **dive
//!   lane** from it straight towards the exit, clear like the corridor up
//!   to the belt's inner edge (a racer crossing its own gate dives there;
//!   inside the inner edge the space is open).
//! - **Asteroids** (Density: 15 to 60 per wedge; Enclosure: radius 1.5-3 u
//!   at 0 to 4-10 u at 100, the biggest hollow with a bore from Enclosure
//!   60): twice as dense in the inner third of the ring as in the outer
//!   third, densest at the plane and half as dense at the belt's surface.
//!   Each is lumpy and faceted (three spheres cut by a box and an
//!   octahedron, in its own random orientation).
//! - **Pickup points** (3), in the plane beside the corridor.
//! - **Boulder orbits** (Density: 3 to 10), prograde circles at 2-4 u/s in
//!   lanes: a lane in the plane and a mirrored pair of lanes above and
//!   below it, every boulder of a lane on the same circle at the same speed
//!   (spread in phase), so boulders never meet. Asteroids keep clear of the
//!   lanes. Boulders are not part of the terrain field.
//! - A **shepherd moon** (Gravity Chaos 20 and above): a rock sphere in
//!   the plane at the ring edge farther from the corridor, at least 25 u
//!   from gates and slots, with a hollow free of asteroids; it is a
//!   [`WellSite`] of kind [`WellKind::Shepherd`].
//!
//! The field ([`Belt::terrain`]) is the distance to the nearest asteroid or
//! moon, capped at [`BELT_CAP`]; a grid index lists the rocks that can
//! matter per cell, and points outside it get the cap.

use ls_math::level::{
    rotate_wedges, BeltSite, BeltSlot, BoulderOrbit, WellKind, WellSite, BELT_WEDGES,
    BELT_WEDGE_ANGLE,
};
use ls_math::rng::SplitMix64;
use ls_math::{det, Aabb, Quat, Vec3};

use crate::generator::floor_i32;
use crate::wells::{well_range, well_strength};
use crate::Sliders;

/// Salt of the belt's random stream ("BELT_GEN").
pub const BELT_SALT: u64 = 0x4245_4c54_5f47_454e;
/// Inner radius of the belt annulus (u).
pub const BELT_INNER_RADIUS: f32 = 95.0;
/// Outer radius of the belt annulus (u).
pub const BELT_OUTER_RADIUS: f32 = 145.0;
/// Belt half thickness at Verticality 0 and 100 (u).
pub const BELT_HALF_THICKNESS: [f32; 2] = [8.0, 18.0];
/// Radius of the cocoon around the exit (u).
pub const COCOON_RADIUS: f32 = 30.0;
/// Radius of the exit sphere (u).
pub const EXIT_RADIUS: f32 = 6.0;
/// Gates reach this far beyond the belt on both sides (u).
pub const GATE_MARGIN: f32 = 8.0;
/// Radius of the start slots (u).
pub const SLOT_RADIUS: f32 = 120.0;
/// Start slots sit this far past their gate (degrees).
pub const SLOT_ANGLE_DEG: f32 = 3.0;
/// Free space kept around a start slot (u).
pub const SLOT_CLEARANCE: f32 = 7.0;
/// Free space kept around the corridor's centre line (u): a corridor at
/// least 4 u wide with room for every ship.
pub const CORRIDOR_CLEARANCE: f32 = 3.5;
/// Free space kept around a pickup point (u).
pub const PICKUP_CLEARANCE: f32 = 4.0;
/// Pickup points per wedge.
pub const PICKUPS_PER_WEDGE: usize = 3;
/// Asteroids per wedge at Density 0 and 100 (both sides of the plane).
pub const ASTEROIDS_PER_WEDGE: [f32; 2] = [15.0, 60.0];
/// Boulder orbits per wedge at Density 0 and 100.
pub const BOULDERS_PER_WEDGE: [f32; 2] = [3.0, 10.0];
/// Asteroid radius range at Enclosure 0 and at 100 (u).
pub const ASTEROID_RADIUS: [[f32; 2]; 2] = [[1.5, 3.0], [4.0, 10.0]];
/// From this Enclosure on, the biggest asteroids are hollow with a bore.
pub const HOLLOW_ENCLOSURE: u8 = 60;
/// Boulder radius range (u).
pub const BOULDER_RADIUS: [f32; 2] = [2.0, 4.0];
/// Boulder orbit speed range (u/s).
pub const BOULDER_SPEED: [f32; 2] = [2.0, 4.0];
/// Below this Gravity Chaos the belt has no shepherd moons.
pub const MOON_GRAVITY_CHAOS: u8 = 20;
/// Shepherd moon core radius range (u).
pub const MOON_RADIUS: [f32; 2] = [3.0, 4.5];
/// Free space around a shepherd moon (u).
pub const MOON_HOLLOW: f32 = 8.0;
/// Least distance from a shepherd moon to the gates and slots (u).
pub const MOON_GATE_DISTANCE: f32 = 25.0;
/// Cap of the belt field (u): rocks further away are not listed.
pub const BELT_CAP: f32 = 8.0;
/// Corridor centre-line samples per wedge.
const CORRIDOR_SAMPLES: usize = 24;
/// Edge length of an index cell (u).
const INDEX_CELL: f32 = 8.0;
/// Rounding margin of the skip test (u).
const SKIP_MARGIN: f32 = 1e-3;
/// Placement attempts per wanted asteroid.
const ATTEMPTS: usize = 60;

/// One rock of the belt (an asteroid or a moon), in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rock {
    center: Vec3,
    /// Rows of the world-to-local map (a rotation, or a rotation and a
    /// mirror): local `q = (axes[0].p, axes[1].p, axes[2].p)` for
    /// `p = point - center`.
    axes: [Vec3; 3],
    shape: Shape,
}

/// A rock's shape in its local frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Shape {
    /// Lump spheres (centre, radius); the first `count` are used.
    lumps: [(Vec3, f32); 3],
    count: usize,
    /// Half extents of the cutting box.
    half: Vec3,
    /// Size of the cutting octahedron (`|x|+|y|+|z| <= octa`).
    octa: f32,
    /// Hollow: shell thickness and bore radius (0 = solid).
    shell: f32,
    bore: f32,
    /// No point of the rock is further than this from its centre.
    bound: f32,
}

impl Shape {
    /// A plain sphere.
    fn sphere(r: f32) -> Shape {
        Shape {
            lumps: [(Vec3::ZERO, r); 3],
            count: 1,
            half: Vec3::splat(2.0 * r),
            octa: 4.0 * r,
            shell: 0.0,
            bore: 0.0,
            bound: r,
        }
    }

    /// Signed distance bound at local point `q` (negative inside).
    #[inline(always)]
    fn sdf(&self, q: Vec3) -> f32 {
        let mut d = q.distance(self.lumps[0].0) - self.lumps[0].1;
        for &(c, r) in &self.lumps[1..self.count] {
            d = d.min(q.distance(c) - r);
        }
        let a = q.abs();
        let bx = (a.x - self.half.x)
            .max(a.y - self.half.y)
            .max(a.z - self.half.z);
        // Octahedron bound: (|x|+|y|+|z| - s) / sqrt(3) is 1-Lipschitz.
        let oc = (a.x + a.y + a.z - self.octa) * 0.577_350_26;
        d = d.max(bx).max(oc);
        if self.shell > 0.0 {
            d = d.max(-(d + self.shell));
            let axis = Vec3::new(q.x, 0.0, q.z).length();
            d = d.max(self.bore - axis);
        }
        d
    }
}

impl Rock {
    #[inline(always)]
    fn sdf(&self, p: Vec3) -> f32 {
        let d = p - self.center;
        let q = Vec3::new(
            self.axes[0].dot(d),
            self.axes[1].dot(d),
            self.axes[2].dot(d),
        );
        self.shape.sdf(q)
    }
}

/// The generated belt: its site (slots, gates, pickups, boulders), the
/// shepherd moons and the rock field with its index.
#[derive(Debug, Clone, PartialEq)]
pub struct Belt {
    site: BeltSite,
    moons: Vec<WellSite>,
    rocks: Vec<Rock>,
    /// Corridor centre line, closed loop over all wedges (plane points).
    corridor: Vec<Vec3>,
    origin: Vec3,
    dims: [usize; 3],
    cells: Vec<(u32, u32)>,
    items: Vec<u16>,
}

/// Uniform in `lo..hi`.
fn range(rng: &mut SplitMix64, [lo, hi]: [f32; 2]) -> f32 {
    lo + (hi - lo) * rng.next_f32()
}

/// `v` (a direction) rotated by `wedges` x 45 degrees about +Y.
fn rotate_dir(v: Vec3, wedges: usize) -> Vec3 {
    rotate_wedges(v, wedges as i32)
}

/// The point at azimuth `a` (rad), radius `r` and height `h` above the
/// plane of a belt centred at `center`.
fn polar(center: Vec3, a: f32, r: f32, h: f32) -> Vec3 {
    let (s, c) = det::sin_cos(a);
    center + Vec3::new(r * c, h, -r * s)
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// A uniformly random rotation (Shoemake).
fn random_rotation(rng: &mut SplitMix64) -> Quat {
    let (u1, u2, u3) = (rng.next_f32(), rng.next_f32(), rng.next_f32());
    let a = (1.0 - u1).sqrt();
    let b = u1.sqrt();
    let (s2, c2) = det::sin_cos(core::f32::consts::TAU * u2);
    let (s3, c3) = det::sin_cos(core::f32::consts::TAU * u3);
    Quat::from_xyzw(a * s2, a * c2, b * s3, b * c3).normalize()
}

/// A unit vector in a random direction.
fn random_dir(rng: &mut SplitMix64) -> Vec3 {
    random_rotation(rng).rotate(Vec3::X)
}

/// The keep-out geometry asteroids must stay clear of. All of it is
/// symmetric (8-fold and in the plane), so checking the base asteroid of
/// wedge 0 against all of it keeps every copy clear too.
struct KeepOut {
    /// Corridor centre line (closed loop) and dive lanes: open polylines
    /// kept [`CORRIDOR_CLEARANCE`] free.
    corridor: Vec<Vec3>,
    lanes: Vec<(Vec3, Vec3)>,
    /// Points kept free with a radius: slots, pickups, moon hollows.
    points: Vec<(Vec3, f32)>,
    /// Boulder lanes: circles about the axis `(radius, height, clearance)`.
    tubes: Vec<(f32, f32, f32)>,
    center: Vec3,
}

/// Spacing of the samples along keep-out lines (u); the rock distance is
/// 1-Lipschitz, so between samples it is at most half of this lower.
const KEEP_STEP: f32 = 0.75;

impl KeepOut {
    /// Whether `rock` keeps clear of everything. Lines and circles are
    /// sampled every [`KEEP_STEP`] near the rock and the rock's own
    /// distance bound must leave the clearance plus half a step there;
    /// parts further than the rock's bounding sphere are skipped.
    fn clear(&self, rock: &Rock) -> bool {
        let c = rock.center;
        let bound = rock.shape.bound;
        let margin = 0.5 * KEEP_STEP + 0.05;
        let segment_clear = |a: Vec3, b: Vec3, clearance: f32| {
            if segment_distance(c, a, b) >= bound + clearance {
                return true;
            }
            let len = a.distance(b);
            let n = (len / KEEP_STEP) as usize + 1;
            (0..=n).all(|k| {
                let p = a + (b - a) * (k as f32 / n as f32);
                p.distance(c) >= bound + clearance || rock.sdf(p) >= clearance + margin
            })
        };
        let n = self.corridor.len();
        for i in 0..n {
            let (a, b) = (self.corridor[i], self.corridor[(i + 1) % n]);
            if (a - c).length() > bound + CORRIDOR_CLEARANCE + 8.0 {
                continue;
            }
            if !segment_clear(a, b, CORRIDOR_CLEARANCE) {
                return false;
            }
        }
        if !self
            .lanes
            .iter()
            .all(|(a, b)| segment_clear(*a, *b, CORRIDOR_CLEARANCE))
        {
            return false;
        }
        if !self
            .points
            .iter()
            .all(|(p, clearance)| p.distance(c) >= bound + clearance || rock.sdf(*p) >= *clearance)
        {
            return false;
        }
        let d = c - self.center;
        let r = Vec3::new(d.x, 0.0, d.z).length();
        let a = det::atan2(-d.z, d.x);
        self.tubes.iter().all(|&(tr, th, clearance)| {
            let (dr, dh) = (r - tr, d.y - th);
            if (dr * dr + dh * dh).sqrt() >= bound + clearance {
                return true;
            }
            // The arc of the lane next to the rock.
            let half = (bound + clearance) / tr;
            let n = (2.0 * half * tr / KEEP_STEP) as usize + 1;
            (0..=n).all(|k| {
                let az = a - half + 2.0 * half * (k as f32 / n as f32);
                let p = polar(self.center, az, tr, th);
                rock.sdf(p) >= clearance + margin
            })
        })
    }
}

impl Belt {
    /// Builds the belt for a seed and sliders in a level box `bounds`.
    pub fn build(seed: u64, sliders: &Sliders, bounds: Aabb) -> Belt {
        let mut rng = SplitMix64::new(seed ^ BELT_SALT);
        let e = sliders.enclosure.min(100) as f32 / 100.0;
        let d = sliders.density.min(100) as f32 / 100.0;
        let v = sliders.verticality.min(100) as f32 / 100.0;
        let half_thickness =
            BELT_HALF_THICKNESS[0] + (BELT_HALF_THICKNESS[1] - BELT_HALF_THICKNESS[0]) * v;
        let mid = bounds.center();
        // The plane at mid height on a cell boundary.
        let plane = det::floor(mid.y);
        let center = Vec3::new(mid.x, plane, mid.z);
        let tau = core::f32::consts::TAU;

        // Corridor: r(u) = 120 + a sin(2 pi (u - u0)) + b sin(4 pi (u - u0)).
        let u_slot = SLOT_ANGLE_DEG / 45.0;
        let a1 = range(&mut rng, [6.0, 13.0]) * if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
        let a2 = range(&mut rng, [0.0, 5.0]) * if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
        let corridor_r = move |u: f32| {
            let t = u - u_slot;
            SLOT_RADIUS + a1 * det::sin(tau * t) + a2 * det::sin(2.0 * tau * t)
        };
        let mut wedge_corridor = Vec::with_capacity(CORRIDOR_SAMPLES);
        for i in 0..CORRIDOR_SAMPLES {
            let u = i as f32 / CORRIDOR_SAMPLES as f32;
            wedge_corridor.push(polar(center, u * BELT_WEDGE_ANGLE, corridor_r(u), 0.0));
        }
        let rotate_point = |p: Vec3, k: usize| center + rotate_dir(p - center, k);
        let mut corridor = Vec::with_capacity(CORRIDOR_SAMPLES * BELT_WEDGES);
        for k in 0..BELT_WEDGES {
            corridor.extend(wedge_corridor.iter().map(|p| rotate_point(*p, k)));
        }

        // Start slots.
        let slot_angle = SLOT_ANGLE_DEG.to_radians();
        let slot0 = polar(center, slot_angle, SLOT_RADIUS, 0.0);
        let (s, c) = det::sin_cos(slot_angle);
        let prograde = Vec3::new(-s, 0.0, -c);
        let facing0 = Quat::from_rotation_arc(Vec3::NEG_Z, prograde);
        let mut slots = Vec::with_capacity(BELT_WEDGES);
        for k in 0..BELT_WEDGES {
            let turn = Quat::from_axis_angle(Vec3::Y, k as f32 * BELT_WEDGE_ANGLE);
            slots.push(BeltSlot {
                pos: rotate_point(slot0, k),
                orientation: turn.mul_quat(facing0).normalize(),
            });
        }

        // Shepherd moon (wedge 0), at the ring edge farther from the corridor.
        let moon_stream = rng.next_u64();
        let mut moon_rng = SplitMix64::new(moon_stream);
        let u_moon = range(&mut moon_rng, [0.4, 0.6]);
        let r_moon = if corridor_r(u_moon) > SLOT_RADIUS {
            BELT_INNER_RADIUS + 6.0
        } else {
            BELT_OUTER_RADIUS - 6.0
        };
        let moon_core = range(&mut moon_rng, MOON_RADIUS);
        let moon0 = polar(center, u_moon * BELT_WEDGE_ANGLE, r_moon, 0.0);
        let has_moons = sliders.gravity_chaos >= MOON_GRAVITY_CHAOS;
        let moons: Vec<WellSite> = if has_moons {
            (0..BELT_WEDGES)
                .map(|k| WellSite {
                    pos: rotate_point(moon0, k),
                    core_radius: moon_core,
                    hollow_radius: MOON_HOLLOW,
                    strength: well_strength(sliders.gravity_chaos),
                    range: well_range(sliders.gravity_chaos),
                    kind: WellKind::Shepherd,
                })
                .collect()
        } else {
            Vec::new()
        };

        // Pickup points (wedge 0): beside the corridor, in the plane.
        let mut pickups0 = Vec::with_capacity(PICKUPS_PER_WEDGE);
        let mut sides = [-1.0f32, 0.0, 1.0];
        for i in (1..sides.len()).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as usize;
            sides.swap(i, j);
        }
        for (i, side) in sides.into_iter().enumerate() {
            let u = 0.22 + 0.28 * i as f32 + range(&mut rng, [-0.04, 0.04]);
            let off = range(&mut rng, [5.0, 9.0]);
            let at = |r: f32| {
                let r = r.clamp(BELT_INNER_RADIUS + 4.0, BELT_OUTER_RADIUS - 4.0);
                polar(center, u * BELT_WEDGE_ANGLE, r, 0.0)
            };
            let mut p = at(corridor_r(u) + side * off);
            if p.distance(moon0) < MOON_HOLLOW + PICKUP_CLEARANCE + moon_core {
                // Keep the pickup out of the moon's hollow (also without
                // moons, so the pickups do not move with Gravity Chaos).
                p = at(corridor_r(u) - side * off);
            }
            pickups0.push(p);
        }

        // Boulder orbits (wedge 0) in lanes: every boulder of a lane shares
        // its circle (radius, height) and speed, so boulders never meet; a
        // lane in the plane and a mirrored pair of lanes above and below.
        let n_boulders =
            det::round(BOULDERS_PER_WEDGE[0] + (BOULDERS_PER_WEDGE[1] - BOULDERS_PER_WEDGE[0]) * d)
                as usize;
        let in_plane = if n_boulders % 2 == 1 {
            1
        } else if n_boulders >= 6 {
            2
        } else {
            0
        };
        let per_pair_lane = (n_boulders - in_plane) / 2;
        // Lanes keep clear of the shepherd moons' ring edge.
        let moon_gap = MOON_RADIUS[1] + BOULDER_RADIUS[1] + 1.5;
        let (lo, hi) = (
            BELT_INNER_RADIUS + BOULDER_RADIUS[1] + 2.0,
            BELT_OUTER_RADIUS - BOULDER_RADIUS[1] - 2.0,
        );
        let lane_range = if r_moon < SLOT_RADIUS {
            [lo.max(r_moon + moon_gap), hi]
        } else {
            [lo, hi.min(r_moon - moon_gap)]
        };
        let lane_r = |rng: &mut SplitMix64| range(rng, lane_range);
        let plane_lane = (lane_r(&mut rng), range(&mut rng, BOULDER_SPEED));
        let pair_lane = (
            lane_r(&mut rng),
            range(&mut rng, BOULDER_SPEED),
            range(&mut rng, [3.0, (half_thickness - 2.0).max(3.5)]),
        );
        let mut orbits0: Vec<BoulderOrbit> = Vec::with_capacity(n_boulders);
        let mut lane =
            |rng: &mut SplitMix64, count: usize, radius: f32, speed: f32, height: f32| {
                for j in 0..count {
                    let size = range(rng, BOULDER_RADIUS);
                    let slot = (j as f32 + 0.5 + range(rng, [-0.3, 0.3])) / count as f32;
                    let tumble_axis = random_dir(rng);
                    let orbit = BoulderOrbit {
                        radius,
                        height,
                        phase: slot * BELT_WEDGE_ANGLE,
                        angular_speed: speed / radius,
                        size,
                        tumble_axis,
                        tumble_speed: range(rng, [0.2, 0.8]),
                    };
                    orbits0.push(orbit);
                    if height != 0.0 {
                        orbits0.push(BoulderOrbit {
                            height: -height,
                            tumble_axis: Vec3::new(tumble_axis.x, -tumble_axis.y, tumble_axis.z),
                            ..orbit
                        });
                    }
                }
            };
        lane(&mut rng, in_plane, plane_lane.0, plane_lane.1, 0.0);
        lane(
            &mut rng,
            per_pair_lane,
            pair_lane.0,
            pair_lane.1,
            pair_lane.2,
        );
        let tube_clear = BOULDER_RADIUS[1] + 0.5;
        let mut tubes = Vec::new();
        if in_plane > 0 {
            tubes.push((plane_lane.0, 0.0, tube_clear));
        }
        if per_pair_lane > 0 {
            tubes.push((pair_lane.0, pair_lane.2, tube_clear));
            tubes.push((pair_lane.0, -pair_lane.2, tube_clear));
        }

        let mut points: Vec<(Vec3, f32)> = slots.iter().map(|s| (s.pos, SLOT_CLEARANCE)).collect();
        for k in 0..BELT_WEDGES {
            points.extend(
                pickups0
                    .iter()
                    .map(|p| (rotate_point(*p, k), PICKUP_CLEARANCE)),
            );
            // The moons' hollows stay free at every Gravity Chaos, so the
            // asteroids do not move with that slider.
            points.push((rotate_point(moon0, k), MOON_HOLLOW + moon_core));
        }
        let keep = KeepOut {
            corridor: corridor.clone(),
            points,
            center,
            lanes: slots
                .iter()
                .map(|s| {
                    let inward =
                        center + (s.pos - center) * ((BELT_INNER_RADIUS - 4.0) / SLOT_RADIUS);
                    (s.pos, inward)
                })
                .collect(),
            tubes,
        };

        // Asteroids of wedge 0, upper side of the plane.
        let [lo_r, hi_r] = [
            ASTEROID_RADIUS[0][0] + (ASTEROID_RADIUS[1][0] - ASTEROID_RADIUS[0][0]) * e,
            ASTEROID_RADIUS[0][1] + (ASTEROID_RADIUS[1][1] - ASTEROID_RADIUS[0][1]) * e,
        ];
        let hollow_from = lo_r + 0.65 * (hi_r - lo_r);
        let total = ASTEROIDS_PER_WEDGE[0] + (ASTEROIDS_PER_WEDGE[1] - ASTEROIDS_PER_WEDGE[0]) * d;
        let want = det::round(total * 0.5) as usize;
        let mut base: Vec<Rock> = Vec::with_capacity(want);
        let (r2_in, r2_out) = (
            BELT_INNER_RADIUS * BELT_INNER_RADIUS,
            BELT_OUTER_RADIUS * BELT_OUTER_RADIUS,
        );
        let room_reach = bounds.size().x.min(bounds.size().z) * 0.5 - 3.0;
        for _ in 0..want * ATTEMPTS {
            if base.len() >= want {
                break;
            }
            let rad = range(&mut rng, [lo_r, hi_r]);
            // Uniform over the annulus area, then thinned: 1.5 - t radially
            // (inner third twice as dense as the outer third) and
            // 1 - 0.5 |h| / thickness vertically.
            let r = (r2_in + rng.next_f32() * (r2_out - r2_in)).sqrt();
            let u = rng.next_f32();
            let h = rng.next_f32() * half_thickness;
            let t = (r - BELT_INNER_RADIUS) / (BELT_OUTER_RADIUS - BELT_INNER_RADIUS);
            let keep_p = (1.5 - t) / 1.5 * (1.0 - 0.5 * h / half_thickness);
            let roll = rng.next_f32();
            let shape = random_shape(&mut rng, rad, sliders.enclosure, hollow_from);
            let rot = random_rotation(&mut rng);
            if roll >= keep_p || r + shape.bound > room_reach {
                continue;
            }
            let rock = Rock {
                center: polar(center, u * BELT_WEDGE_ANGLE, r, h),
                axes: [
                    rot.rotate(Vec3::X),
                    rot.rotate(Vec3::Y),
                    rot.rotate(Vec3::Z),
                ],
                shape,
            };
            if !keep.clear(&rock) {
                continue;
            }
            base.push(rock);
        }

        // Every copy: 8 rotations x (as is, mirrored in the plane).
        let mut rocks = Vec::with_capacity(base.len() * 2 * BELT_WEDGES + moons.len());
        for k in 0..BELT_WEDGES {
            for rock in &base {
                let axes = rock.axes.map(|a| rotate_dir(a, k));
                let c = rotate_point(rock.center, k);
                rocks.push(Rock {
                    center: c,
                    axes,
                    shape: rock.shape,
                });
                rocks.push(Rock {
                    center: Vec3::new(c.x, 2.0 * plane - c.y, c.z),
                    axes: axes.map(|a| Vec3::new(a.x, -a.y, a.z)),
                    shape: rock.shape,
                });
            }
        }
        for m in &moons {
            rocks.push(Rock {
                center: m.pos,
                axes: [Vec3::X, Vec3::Y, Vec3::Z],
                shape: Shape::sphere(m.core_radius),
            });
        }

        let mut pickup_points = Vec::with_capacity(PICKUPS_PER_WEDGE * BELT_WEDGES);
        let mut boulders = Vec::with_capacity(orbits0.len() * BELT_WEDGES);
        for k in 0..BELT_WEDGES {
            pickup_points.extend(pickups0.iter().map(|p| rotate_point(*p, k)));
            boulders.extend(orbits0.iter().map(|o| BoulderOrbit {
                phase: det::wrap_angle(o.phase + k as f32 * BELT_WEDGE_ANGLE),
                ..*o
            }));
        }
        let slots: [BeltSlot; BELT_WEDGES] = core::array::from_fn(|k| slots[k]);
        let site = BeltSite {
            center,
            inner_radius: BELT_INNER_RADIUS,
            outer_radius: BELT_OUTER_RADIUS,
            half_thickness,
            cocoon_radius: COCOON_RADIUS,
            exit_radius: EXIT_RADIUS,
            gate_inner_radius: BELT_INNER_RADIUS - GATE_MARGIN,
            gate_outer_radius: BELT_OUTER_RADIUS + GATE_MARGIN,
            slots,
            pickup_points,
            boulders,
        };
        Belt::index(site, moons, rocks, corridor)
    }

    fn index(site: BeltSite, moons: Vec<WellSite>, rocks: Vec<Rock>, corridor: Vec<Vec3>) -> Belt {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for r in &rocks {
            lo = lo.min(r.center - Vec3::splat(r.shape.bound));
            hi = hi.max(r.center + Vec3::splat(r.shape.bound));
        }
        if rocks.is_empty() {
            lo = site.center;
            hi = site.center;
        }
        let area = Aabb::new(lo, hi).expand(BELT_CAP + INDEX_CELL);
        let origin = area.min;
        let dims = [0, 1, 2].map(|a| (floor_i32(area.size()[a] / INDEX_CELL) + 1).max(1) as usize);
        let mut lists: Vec<Vec<u16>> = vec![Vec::new(); dims[0] * dims[1] * dims[2]];
        for (i, r) in rocks.iter().enumerate() {
            // Cells whose box comes within the cap of the rock's bound.
            let reach = r.shape.bound + BELT_CAP + SKIP_MARGIN;
            let cmin = (r.center - Vec3::splat(reach) - origin) / INDEX_CELL;
            let cmax = (r.center + Vec3::splat(reach) - origin) / INDEX_CELL;
            let a = [cmin.x, cmin.y, cmin.z].map(|v| floor_i32(v).max(0) as usize);
            let b = [cmax.x, cmax.y, cmax.z];
            for z in a[2]..=(floor_i32(b[2]).max(0) as usize).min(dims[2] - 1) {
                for y in a[1]..=(floor_i32(b[1]).max(0) as usize).min(dims[1] - 1) {
                    for x in a[0]..=(floor_i32(b[0]).max(0) as usize).min(dims[0] - 1) {
                        let min = origin + Vec3::new(x as f32, y as f32, z as f32) * INDEX_CELL;
                        let cell = Aabb::new(min, min + Vec3::splat(INDEX_CELL));
                        if cell.closest_point(r.center).distance(r.center) < reach {
                            lists[x + dims[0] * (y + dims[1] * z)].push(i as u16);
                        }
                    }
                }
            }
        }
        let mut cells = Vec::with_capacity(lists.len());
        let mut items = Vec::new();
        for l in lists {
            let start = items.len() as u32;
            items.extend(l);
            cells.push((start, items.len() as u32));
        }
        Belt {
            site,
            moons,
            rocks,
            corridor,
            origin,
            dims,
            cells,
            items,
        }
    }

    /// The belt's site (slots, gates, exit, pickup points, boulders).
    pub fn site(&self) -> &BeltSite {
        &self.site
    }

    /// The shepherd moons (empty below Gravity Chaos
    /// [`MOON_GRAVITY_CHAOS`]).
    pub fn moons(&self) -> &[WellSite] {
        &self.moons
    }

    /// The corridor's centre line: a closed loop through every wedge in
    /// the belt plane, [`CORRIDOR_SAMPLES`] points per wedge, prograde.
    pub fn corridor(&self) -> &[Vec3] {
        &self.corridor
    }

    /// Number of rocks in the field (asteroid copies and moons).
    pub fn rock_count(&self) -> usize {
        self.rocks.len()
    }

    /// The belt field at `p`: distance to the nearest rock (negative
    /// inside), capped at [`BELT_CAP`]. 1-Lipschitz.
    #[inline]
    pub fn terrain(&self, p: Vec3) -> f32 {
        let mut best = BELT_CAP;
        for &i in self.cell_items(p) {
            let rock = &self.rocks[i as usize];
            let dc = p.distance(rock.center);
            if dc - rock.shape.bound - SKIP_MARGIN >= best {
                continue;
            }
            best = best.min(rock.sdf(p));
        }
        best
    }

    /// The rocks listed for the index cell of `p` (none outside).
    #[inline(always)]
    fn cell_items(&self, p: Vec3) -> &[u16] {
        let q = p - self.origin;
        let mut idx = 0usize;
        let mut stride = 1usize;
        for a in 0..3 {
            let i = floor_i32(q[a] / INDEX_CELL);
            if i < 0 || i as usize >= self.dims[a] {
                return &[];
            }
            idx += i as usize * stride;
            stride *= self.dims[a];
        }
        let (s, e) = self.cells[idx];
        &self.items[s as usize..e as usize]
    }
}

/// A random lumpy, faceted asteroid shape of radius `rad`; hollow with a
/// bore when Enclosure is at least [`HOLLOW_ENCLOSURE`] and `rad` is at
/// least `hollow_from`.
fn random_shape(rng: &mut SplitMix64, rad: f32, enclosure: u8, hollow_from: f32) -> Shape {
    let mut lumps = [(Vec3::ZERO, rad); 3];
    let mut bound = rad;
    for lump in lumps.iter_mut().skip(1) {
        let r = rad * range(rng, [0.5, 0.7]);
        let off = random_dir(rng) * (rad * range(rng, [0.45, 0.65]));
        *lump = (off, r);
        bound = bound.max(off.length() + r);
    }
    let half = Vec3::new(
        rad * range(rng, [0.75, 1.05]),
        rad * range(rng, [0.6, 0.95]),
        rad * range(rng, [0.75, 1.05]),
    );
    let octa = rad * range(rng, [1.2, 1.45]);
    let hollow = enclosure >= HOLLOW_ENCLOSURE && rad >= hollow_from;
    Shape {
        lumps,
        count: 3,
        half,
        octa,
        shell: if hollow { (0.28 * rad).max(1.2) } else { 0.0 },
        bore: if hollow { (0.32 * rad).max(1.6) } else { 0.0 },
        bound: bound + 0.01,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Generator, ShareCode};
    use ls_math::level::Archetype;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn belt_gen(seed: u64, s: Sliders) -> Generator {
        Generator::with_archetype(seed, s, Archetype::Belt)
    }

    fn sliders(e: u8, d: u8, v: u8, g: u8) -> Sliders {
        Sliders {
            enclosure: e,
            density: d,
            verticality: v,
            gravity_chaos: g,
            ..Sliders::default()
        }
    }

    /// M4 plan A1: the generator field at 10 000 random points equals the
    /// field at the point rotated by every multiple of 45 degrees and
    /// mirrored in the belt plane, within 1e-4 u.
    #[test]
    fn field_has_eightfold_rotation_and_plane_mirror_symmetry() {
        let mut rng = SplitMix64::new(4545);
        for (seed, s) in [
            (1u64, sliders(50, 60, 50, 40)),
            (2, sliders(100, 100, 100, 100)),
            (3, sliders(0, 20, 0, 0)),
        ] {
            let g = belt_gen(seed, s);
            let b = g.belt().unwrap().clone();
            let mut worst = 0.0f32;
            let mut near = 0;
            for _ in 0..10_000 {
                // Inside the cylinder the rotations keep in the level; most
                // points near the belt, where the rocks are.
                let a = rng.next_f32() * core::f32::consts::TAU;
                let (r, h) = if rng.next_f32() < 0.8 {
                    (95.0 + rng.next_f32() * 50.0, (rng.next_f32() - 0.5) * 36.0)
                } else {
                    (rng.next_f32().sqrt() * 158.0, (rng.next_f32() - 0.5) * 90.0)
                };
                let p = polar(b.center, a, r, h);
                let v = g.terrain(p);
                if v < BELT_CAP {
                    near += 1;
                }
                for k in 1..BELT_WEDGES as i32 {
                    worst = worst.max((g.terrain(b.rotate(p, k)) - v).abs());
                }
                worst = worst.max((g.terrain(b.mirror(p)) - v).abs());
                worst = worst.max((g.terrain(b.mirror(b.rotate(p, 3))) - v).abs());
            }
            println!("seed {seed}: largest asymmetry {worst:e} u ({near} points near rock)");
            assert!(worst <= 1e-4, "seed {seed}: asymmetry {worst}");
            assert!(near > 2000, "seed {seed}: only {near} points near rock");
        }
    }

    #[test]
    fn site_layout_follows_the_plan() {
        for (seed, s) in [
            (5u64, sliders(50, 40, 50, 0)),
            (6, sliders(100, 100, 0, 100)),
            (7, sliders(0, 0, 100, 25)),
        ] {
            let g = belt_gen(seed, s);
            let b = g.belt().unwrap();
            assert_eq!(g.size_chunks(), crate::BELT_SIZE_CHUNKS);
            assert_eq!(b.center, Vec3::new(160.0, 48.0, 160.0));
            let ht = 8.0 + 10.0 * s.verticality as f32 / 100.0;
            assert!((b.half_thickness - ht).abs() < 1e-4);
            assert_eq!((b.gate_inner_radius, b.gate_outer_radius), (87.0, 153.0));
            // Slots: radius 120, 3 degrees past their gate, facing prograde.
            for (k, slot) in b.slots.iter().enumerate() {
                assert!((b.axis_distance(slot.pos) - 120.0).abs() < 1e-3);
                assert!((slot.pos.y - b.center.y).abs() < 1e-4);
                let past = b.azimuth(slot.pos) - BeltSite::gate_azimuth(k);
                assert!((past.to_degrees() - 3.0).abs() < 1e-2, "slot {k}: {past}");
                let ahead = slot.pos + slot.orientation.forward() * 2.0;
                assert!(
                    b.azimuth(ahead) > b.azimuth(slot.pos),
                    "slot {k} faces prograde"
                );
                assert!(g.sample(slot.pos) >= SLOT_CLEARANCE - 0.01, "slot {k}");
            }
            // Pickups: 3 per wedge, in open space in the belt, rotated copies.
            assert_eq!(b.pickup_points.len(), 24);
            for (i, p) in b.pickup_points.iter().enumerate() {
                assert!(g.sample(*p) >= PICKUP_CLEARANCE - 0.01, "pickup {i}");
                assert!(b.belt_distance(*p) < 0.0, "pickup {i} outside the belt");
                let twin = b.rotate(b.pickup_points[i % 3], (i / 3) as i32);
                assert!(twin.distance(*p) < 1e-3);
            }
            // Boulders: 3 to 10 per wedge, prograde, mirrored pairs.
            let per = b.boulders.len() / 8;
            assert_eq!(per * 8, b.boulders.len());
            assert!((3..=10).contains(&per), "{per}");
            let heights: f32 = b.boulders[..per].iter().map(|o| o.height).sum();
            assert!(heights.abs() < 1e-4, "pairs mirror: {heights}");
            for o in &b.boulders {
                let v = o.radius * o.angular_speed;
                assert!((2.0 - 1e-4..=4.0 + 1e-4).contains(&v), "speed {v}");
                assert!((2.0..=4.0).contains(&o.size));
            }
            // Boulders never touch an asteroid: their lanes are clear.
            for o in &b.boulders {
                for tick in (0..60 * 600).step_by(97) {
                    let p = o.pos_at(b.center, tick);
                    assert!(g.terrain(p) >= o.size, "boulder {o:?} hits rock at {p:?}");
                }
            }
            // No rock in the cocoon.
            let mut rng = SplitMix64::new(seed);
            for _ in 0..2000 {
                let p = b.center + random_dir(&mut rng) * (rng.next_f32() * COCOON_RADIUS);
                assert!(g.sample(p) > 0.0, "rock in the cocoon at {p:?}");
            }
            // Shepherd moons from Gravity Chaos 20: one per wedge, 25 u from
            // gates and slots, in the plane.
            let moons = g.gravity_wells();
            if s.gravity_chaos >= MOON_GRAVITY_CHAOS {
                assert_eq!(moons.len(), 8);
                for m in moons {
                    assert_eq!(m.kind, WellKind::Shepherd);
                    assert!((m.pos.y - b.center.y).abs() < 1e-4);
                    assert!(g.sample(m.pos) < 0.0, "moon core is rock");
                    for k in 0..8 {
                        let (s, c) = det::sin_cos(BeltSite::gate_azimuth(k));
                        let normal = Vec3::new(-s, 0.0, -c);
                        let radial = Vec3::new(c, 0.0, -s);
                        let d = m.pos - b.center;
                        let gate_dist = if d.dot(radial) > 0.0 {
                            d.dot(normal).abs()
                        } else {
                            d.length()
                        };
                        assert!(gate_dist >= MOON_GATE_DISTANCE, "{gate_dist}");
                        assert!(m.pos.distance(b.slots[k].pos) >= MOON_GATE_DISTANCE);
                    }
                }
            } else {
                assert!(moons.is_empty());
            }
        }
    }

    #[test]
    fn corridor_is_clear_and_closed() {
        for (seed, s) in [
            (8u64, sliders(100, 100, 50, 60)),
            (9, sliders(30, 80, 20, 0)),
        ] {
            let g = belt_gen(seed, s);
            let belt = g.belt_field().unwrap();
            let c = belt.corridor();
            assert_eq!(c.len(), CORRIDOR_SAMPLES * 8);
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                for t in 0..8 {
                    let p = a + (b - a) * (t as f32 / 8.0);
                    assert!(
                        g.sample(p) >= CORRIDOR_CLEARANCE - 0.05,
                        "seed {seed}: corridor blocked at {p:?}: {}",
                        g.sample(p)
                    );
                    assert!(g.belt().unwrap().belt_distance(p) < -2.0);
                }
            }
        }
    }

    #[test]
    fn density_shapes_the_field() {
        let sparse = belt_gen(11, sliders(50, 0, 50, 0));
        let dense = belt_gen(11, sliders(50, 100, 50, 0));
        let n = |g: &Generator| g.belt_field().unwrap().rock_count();
        assert!(n(&dense) > 2 * n(&sparse), "{} {}", n(&dense), n(&sparse));
        // Per wedge: 15 and 60 asteroids wanted (both sides), most placed.
        assert!(n(&sparse) >= 8 * 12, "{}", n(&sparse));
        assert!(n(&dense) >= 8 * 40, "{}", n(&dense));
    }

    #[test]
    fn belt_codes_build_belts() {
        let code = ShareCode::with_archetype(77, sliders(50, 50, 50, 50), Archetype::Belt);
        let g = Generator::from_code(&code).unwrap();
        assert_eq!(g.archetype(), Archetype::Belt);
        assert_eq!(g.code(), code);
        assert!(g.belt().is_some());
        assert!(g.network().hubs().is_empty());
        assert!(g.heist().is_none());
        let classic = Generator::from_code(&code.as_archetype(Archetype::Classic)).unwrap();
        assert!(classic.belt().is_none());
    }
}
