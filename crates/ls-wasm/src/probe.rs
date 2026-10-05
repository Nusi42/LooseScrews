//! Terrain probes for the client's scale cues (visual-quality-plan.md VQ1):
//! a plain ray cast (what the crosshair points at) and a sphere sweep (how
//! far the own ship can fly straight ahead before its hull touches rock).
//! The HUD's fit ring draws the ship's cross-section at the sweep's contact
//! point, so a ring larger than the opening ahead means "won't fit".
//!
//! Cosmetic and client-only: nothing here feeds the simulation. The sweep
//! measures contact like the simulation does (`ls_sim::terrain`'s first-order
//! distance estimate near the rock, the bound field further away), so the ring
//! agrees with where the ship really stops.

use ls_math::terrain::TerrainQuery;
use ls_math::Vec3;
use ls_sim::terrain::{surface_estimate, world_sdf};

/// Smallest step of the sweep (u): near rock the bound field can be far
/// below the true distance, so steps never shrink below this.
pub const SWEEP_MIN_STEP: f32 = 0.12;
/// Step limit of the sweep; beyond it the path counts as clear.
pub const SWEEP_MAX_STEPS: u32 = 64;
/// A sweep that starts already touching (scraping a wall) only reports
/// contacts that come this much closer than the starting gap.
pub const SWEEP_START_SLACK: f32 = 0.05;

/// Where a swept sphere first touches rock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepHit {
    /// Distance the sphere's centre travelled along the (unit) direction.
    pub travel: f32,
    /// Contact point on the rock surface (estimate).
    pub point: Vec3,
    /// Surface normal at the contact, pointing into open space.
    pub normal: Vec3,
}

/// Distance along `dir` to the first rock surface (sphere tracing over the
/// bound field, exact enough for a HUD), or `None` within `max`.
pub fn raycast<T: TerrainQuery + ?Sized>(
    terrain: &T,
    origin: Vec3,
    dir: Vec3,
    max: f32,
) -> Option<f32> {
    let dir = dir.try_normalize()?;
    let bounds = terrain.bounds();
    // The bounds are solid: clip like `world_sdf` does.
    let hit = terrain.raycast(origin, dir, max).map(|h| h.distance);
    let wall =
        ls_math::terrain::sphere_trace(&BoundsOnly(bounds), origin, dir, max).map(|h| h.distance);
    match (hit, wall) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The level box as terrain (open inside), for clipping rays to it.
struct BoundsOnly(ls_math::Aabb);

impl TerrainQuery for BoundsOnly {
    fn sdf(&self, p: Vec3) -> f32 {
        -self.0.sdf(p)
    }
    fn bounds(&self) -> ls_math::Aabb {
        self.0
    }
}

/// Sweeps a sphere of `radius` from `origin` along `dir` and returns the
/// first contact within `max` travel, or `None` when the path is clear (or
/// the step limit is reached first: a long tight tunnel counts as clear).
pub fn sweep_sphere<T: TerrainQuery + ?Sized>(
    terrain: &T,
    origin: Vec3,
    dir: Vec3,
    radius: f32,
    max: f32,
) -> Option<SweepHit> {
    let dir = dir.try_normalize()?;
    if radius.is_nan() || max.is_nan() || radius <= 0.0 || max <= 0.0 {
        return None;
    }
    let gap_at = |p: Vec3| -> (f32, f32) {
        let bound = world_sdf(terrain, p);
        if bound >= radius {
            (bound, bound)
        } else {
            (surface_estimate(terrain, p).distance, bound)
        }
    };
    // Scraping along a wall at the start must not count as a contact.
    let (start_gap, _) = gap_at(origin);
    let touch = radius.min(start_gap - SWEEP_START_SLACK);
    if touch <= 0.0 {
        return None;
    }
    let mut prev = 0.0f32;
    let mut t = 0.0f32;
    for _ in 0..SWEEP_MAX_STEPS {
        let (gap, bound) = gap_at(origin + dir * t);
        if gap < touch {
            // Bisect between the last free position and this one.
            let (mut lo, mut hi) = (prev, t);
            for _ in 0..4 {
                let mid = 0.5 * (lo + hi);
                if gap_at(origin + dir * mid).0 < touch {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let centre = origin + dir * hi;
            let est = surface_estimate(terrain, centre);
            return Some(SweepHit {
                travel: hi,
                point: centre - est.normal * est.distance.max(0.0),
                normal: est.normal,
            });
        }
        if t >= max {
            return None;
        }
        // The bound is safe to step; the estimate only to first order, so
        // take half of its margin.
        let step = (bound - touch).max(0.5 * (gap - touch)).max(SWEEP_MIN_STEP);
        prev = t;
        t = (t + step).min(max);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
    use ls_math::Aabb;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// A room with a rock ball ahead (-Z, front face at z = -5) bored by a
    /// dead-end tunnel of radius `bore` along the axis from z = -4 to -12.
    fn bored_ball(bore: f32) -> BoxRoomTerrain {
        let mut t = BoxRoomTerrain::new(Aabb::from_center_half_extents(
            Vec3::ZERO,
            Vec3::splat(40.0),
        ));
        t.apply(TerrainEdit::AddSphere {
            center: Vec3::new(0.0, 0.0, -15.0),
            radius: 10.0,
        });
        for i in 0..=160 {
            t.apply(TerrainEdit::SubtractSphere {
                center: Vec3::new(0.0, 0.0, -4.0 - i as f32 * 0.05),
                radius: bore,
            });
        }
        t
    }

    /// The same field scaled down: a distance bound that is not a true
    /// distance (like the cave noise, which reports about 0.3 of it).
    struct Scaled<T>(T, f32);

    impl<T: TerrainQuery> TerrainQuery for Scaled<T> {
        fn sdf(&self, p: Vec3) -> f32 {
            self.0.sdf(p) * self.1
        }
        fn bounds(&self) -> Aabb {
            self.0.bounds()
        }
    }

    const AHEAD: Vec3 = Vec3::new(0.0, 0.0, -1.0);

    #[test]
    fn ray_hits_the_wall_and_the_bounds() {
        let t = bored_ball(0.6);
        let side = raycast(&t, Vec3::new(0.0, 2.0, 0.0), AHEAD, 30.0).unwrap();
        // Ball surface at height 2: z = -15 + sqrt(100 - 4).
        assert!((side - (15.0 - 96f32.sqrt())).abs() < 0.01, "{side}");
        // Down the bore to its dead end (z = -12.6).
        let bore = raycast(&t, Vec3::ZERO, AHEAD, 30.0).unwrap();
        assert!((bore - 12.6).abs() < 0.02, "{bore}");
        // Upwards to the room's ceiling at y = 40.
        let up = raycast(&t, Vec3::ZERO, Vec3::Y, 60.0).unwrap();
        assert!((up - 40.0).abs() < 0.01, "{up}");
        assert_eq!(raycast(&t, Vec3::ZERO, Vec3::Y, 30.0), None);
    }

    #[test]
    fn a_ship_that_fits_flies_down_the_bore() {
        let t = bored_ball(0.6);
        // Tinker (r = 0.5) in a 1.2 u bore: from inside it, stops at the
        // dead end (z = -12.6, so the centre stops at -12.1).
        let start = Vec3::new(0.0, 0.0, -6.0);
        let hit = sweep_sphere(&t, start, AHEAD, 0.5, 20.0).unwrap();
        assert!((hit.travel - 6.1).abs() < 0.1, "{hit:?}");
        assert!(hit.point.z < -12.4, "{hit:?}");
        // From outside it enters the bore: no contact at the mouth; the step
        // limit may call the rest of the narrow bore clear.
        let outside = sweep_sphere(&t, Vec3::ZERO, AHEAD, 0.5, 20.0);
        assert!(outside.is_none_or(|h| h.travel > 8.0), "{outside:?}");
    }

    #[test]
    fn a_ship_that_does_not_fit_stops_at_the_rim() {
        let t = bored_ball(0.6);
        // Hauler (r = 0.65): the rim of the 0.6 u bore at z = -5 stops it.
        // Geometrically at travel 5 - sqrt(0.65² - 0.6²) = 4.75; the carved
        // field (max of the ball and the bore spheres) is only a bound at
        // the rim edge with slope 1, so the simulation's contact (and the
        // sweep, which measures the same way) stops at 5 - 0.65 = 4.35.
        let hit = sweep_sphere(&t, Vec3::ZERO, AHEAD, 0.65, 20.0).unwrap();
        assert!((4.3..=4.8).contains(&hit.travel), "{hit:?}");
        // The contact lies at the depth of the rim (the HUD draws the ring there).
        assert!((hit.point.z + 5.0).abs() < 0.1, "{hit:?}");
    }

    #[test]
    fn a_bound_field_gives_the_same_contact() {
        // With the field at 0.3 of the distance the estimate still finds
        // the true contact (as the simulation's collision does).
        let t = Scaled(bored_ball(0.6), 0.3);
        let fits = sweep_sphere(&t, Vec3::ZERO, AHEAD, 0.5, 20.0);
        let exact = sweep_sphere(&bored_ball(0.6), Vec3::ZERO, AHEAD, 0.65, 20.0).unwrap();
        let blocked = sweep_sphere(&t, Vec3::ZERO, AHEAD, 0.65, 20.0).unwrap();
        assert!(
            (blocked.travel - exact.travel).abs() < 0.1,
            "{blocked:?} vs {exact:?}"
        );
        assert!((blocked.point.z + 5.0).abs() < 0.15, "{blocked:?}");
        // The fitting ship gets well past the rim (the step limit may call
        // the narrow bore clear before its end).
        assert!(fits.is_none_or(|h| h.travel > 8.0), "{fits:?}");
    }

    #[test]
    fn head_on_wall_contact_is_on_the_axis() {
        let t = bored_ball(0.6);
        let hit = sweep_sphere(&t, Vec3::new(0.0, 3.0, 0.0), AHEAD, 0.5, 20.0).unwrap();
        let wall = 15.0 - 91f32.sqrt();
        assert!((hit.point.z + wall).abs() < 0.1, "{hit:?}");
        assert!(
            hit.point.x.abs() < 0.05 && (hit.point.y - 3.0).abs() < 0.2,
            "{hit:?}"
        );
    }

    #[test]
    fn clear_paths_scrapes_and_bad_input_report_nothing() {
        let t = bored_ball(0.6);
        // Away from the ball.
        assert_eq!(sweep_sphere(&t, Vec3::ZERO, Vec3::Z, 0.5, 12.0), None);
        // Sliding along the floor 0.3 u above it (closer than the radius).
        let floor = Vec3::new(0.0, -39.7, 10.0);
        assert_eq!(sweep_sphere(&t, floor, Vec3::X, 0.5, 12.0), None);
        assert_eq!(sweep_sphere(&t, Vec3::ZERO, Vec3::ZERO, 0.5, 12.0), None);
        assert_eq!(sweep_sphere(&t, Vec3::ZERO, AHEAD, 0.0, 12.0), None);
        assert_eq!(raycast(&t, Vec3::ZERO, Vec3::ZERO, 12.0), None);
    }
}
