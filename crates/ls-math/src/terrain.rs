//! Shared terrain contracts.
//!
//! These types live in `ls-math` so that `ls-worldgen` (which implements the
//! real terrain), `ls-sim` (which queries it for physics) and `ls-net` (which
//! sends edits over the wire) can be developed in parallel without depending
//! on each other. See `docs/architecture/contracts.md`.
//!
//! **Sign convention:** the signed distance field is *negative inside solid
//! rock* and *positive in open space*. The surface is the zero level set.

use serde::{Deserialize, Serialize};

use crate::{Aabb, Vec3};

/// A parametric terrain modification, applied in tick order and logged so
/// late joiners can replay it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TerrainEdit {
    /// Carves a spherical hole (explosion crater).
    SubtractSphere { center: Vec3, radius: f32 },
    /// Adds a solid sphere of rock.
    AddSphere { center: Vec3, radius: f32 },
    /// Paints surface cells within the sphere with a team color. Does not change geometry.
    Paint { center: Vec3, radius: f32, team: u8 },
}

impl TerrainEdit {
    #[inline]
    pub fn center(&self) -> Vec3 {
        match *self {
            TerrainEdit::SubtractSphere { center, .. }
            | TerrainEdit::AddSphere { center, .. }
            | TerrainEdit::Paint { center, .. } => center,
        }
    }

    #[inline]
    pub fn radius(&self) -> f32 {
        match *self {
            TerrainEdit::SubtractSphere { radius, .. }
            | TerrainEdit::AddSphere { radius, .. }
            | TerrainEdit::Paint { radius, .. } => radius,
        }
    }

    /// True if the edit changes the SDF (not just paint).
    #[inline]
    pub fn changes_geometry(&self) -> bool {
        !matches!(self, TerrainEdit::Paint { .. })
    }

    /// Bounding box of the region the edit can affect.
    #[inline]
    pub fn bounds(&self) -> Aabb {
        Aabb::from_center_half_extents(self.center(), Vec3::splat(self.radius()))
    }

    /// Applies this edit to an SDF value `d` sampled at `p`, using the standard
    /// CSG operators (union = `min`, subtraction = `max(d, -s)`). Paint returns `d`.
    #[inline]
    pub fn apply_to_sdf(&self, d: f32, p: Vec3) -> f32 {
        match *self {
            TerrainEdit::SubtractSphere { center, radius } => d.max(radius - p.distance(center)),
            TerrainEdit::AddSphere { center, radius } => d.min(p.distance(center) - radius),
            TerrainEdit::Paint { .. } => d,
        }
    }
}

/// Result of a terrain raycast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// World-space hit point (on or very near the surface).
    pub point: Vec3,
    /// Unit surface normal at the hit point, pointing into open space.
    pub normal: Vec3,
    /// Distance from the ray origin along the (unit) ray direction.
    pub distance: f32,
}

/// Read-only terrain queries used by physics and gameplay.
///
/// Implementations must be deterministic: the same terrain state must give
/// bit-identical answers on every platform.
pub trait TerrainQuery {
    /// Signed distance at `p`: negative inside rock, positive in open space.
    /// Must be a distance *bound*: `|sdf(a) - sdf(b)| <= |a - b|` (1-Lipschitz),
    /// which the default [`TerrainQuery::raycast`] relies on.
    fn sdf(&self, p: Vec3) -> f32;

    /// Unit surface normal near `p`, pointing towards open space. The default
    /// uses central differences of [`TerrainQuery::sdf`].
    fn normal(&self, p: Vec3) -> Vec3 {
        sdf_gradient(|q| self.sdf(q), p)
    }

    /// First surface hit along `origin + dir * t` for `t` in `[0, max_dist]`.
    /// `dir` is normalized internally; a zero `dir` never hits. If `origin`
    /// is already inside rock the hit is at distance 0. The default is sphere
    /// tracing over [`TerrainQuery::sdf`].
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
        sphere_trace(self, origin, dir, max_dist)
    }

    /// World bounds of the level. Everything outside counts as solid.
    fn bounds(&self) -> Aabb;

    /// True if a sphere at `center` with `radius` touches rock.
    fn sphere_overlap(&self, center: Vec3, radius: f32) -> bool {
        self.sdf(center) < radius
    }
}

/// Step used for central-difference normals.
pub const NORMAL_EPS: f32 = 1.0 / 512.0;
/// A sphere-traced ray counts as a hit once the SDF drops below this.
pub const HIT_EPS: f32 = 1.0 / 1024.0;
/// Maximum number of sphere-tracing steps before giving up.
pub const MAX_TRACE_STEPS: u32 = 512;

/// Normalized central-difference gradient of an SDF. Returns `+Y` where the
/// gradient vanishes (e.g. exactly at a medial point).
pub fn sdf_gradient(sdf: impl Fn(Vec3) -> f32, p: Vec3) -> Vec3 {
    let e = NORMAL_EPS;
    let g = Vec3::new(
        sdf(p + Vec3::new(e, 0.0, 0.0)) - sdf(p - Vec3::new(e, 0.0, 0.0)),
        sdf(p + Vec3::new(0.0, e, 0.0)) - sdf(p - Vec3::new(0.0, e, 0.0)),
        sdf(p + Vec3::new(0.0, 0.0, e)) - sdf(p - Vec3::new(0.0, 0.0, e)),
    );
    g.try_normalize().unwrap_or(Vec3::Y)
}

/// Generic sphere tracer over any [`TerrainQuery`] (the default `raycast`).
pub fn sphere_trace<T: TerrainQuery + ?Sized>(
    terrain: &T,
    origin: Vec3,
    dir: Vec3,
    max_dist: f32,
) -> Option<RayHit> {
    let dir = dir.try_normalize()?;
    if max_dist.is_nan() || max_dist < 0.0 {
        return None;
    }
    let mut t = 0.0f32;
    for _ in 0..MAX_TRACE_STEPS {
        let p = origin + dir * t;
        let d = terrain.sdf(p);
        if d < HIT_EPS {
            return Some(RayHit {
                point: p,
                normal: terrain.normal(p),
                distance: t,
            });
        }
        t += d;
        if t > max_dist {
            return None;
        }
    }
    None
}

/// Infinite solid half-space below a plane, clipped to `bounds` only for the
/// purpose of [`TerrainQuery::bounds`]. Rock is where `dot(p, normal) < offset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneTerrain {
    /// Unit normal pointing into open space.
    pub normal: Vec3,
    /// Plane offset along `normal`: points with `dot(p, normal) == offset` lie on the surface.
    pub offset: f32,
    pub bounds: Aabb,
}

impl PlaneTerrain {
    /// A flat floor at height `y` (normal `+Y`) with the given bounds.
    pub fn floor(y: f32, bounds: Aabb) -> Self {
        PlaneTerrain {
            normal: Vec3::Y,
            offset: y,
            bounds,
        }
    }

    /// A plane with an arbitrary normal (normalized here).
    pub fn new(normal: Vec3, offset: f32, bounds: Aabb) -> Self {
        PlaneTerrain {
            normal: normal.try_normalize().unwrap_or(Vec3::Y),
            offset,
            bounds,
        }
    }
}

impl TerrainQuery for PlaneTerrain {
    fn sdf(&self, p: Vec3) -> f32 {
        p.dot(self.normal) - self.offset
    }

    fn normal(&self, _p: Vec3) -> Vec3 {
        self.normal
    }

    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
        let dir = dir.try_normalize()?;
        let d0 = self.sdf(origin);
        if d0 <= 0.0 {
            return Some(RayHit {
                point: origin,
                normal: self.normal,
                distance: 0.0,
            });
        }
        let approach = -dir.dot(self.normal);
        if approach <= 0.0 {
            return None;
        }
        let t = d0 / approach;
        if t > max_dist {
            return None;
        }
        Some(RayHit {
            point: origin + dir * t,
            normal: self.normal,
            distance: t,
        })
    }

    fn bounds(&self) -> Aabb {
        self.bounds
    }
}

/// A closed box-shaped room: open space inside `room`, solid rock outside,
/// with [`TerrainEdit`]s applied in order on top. Paint edits are recorded
/// but do not change geometry. Intended for tests in `ls-sim` and elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxRoomTerrain {
    pub room: Aabb,
    edits: Vec<TerrainEdit>,
}

impl BoxRoomTerrain {
    pub fn new(room: Aabb) -> Self {
        BoxRoomTerrain {
            room,
            edits: Vec::new(),
        }
    }

    /// A room centered on the origin with the given full size.
    pub fn centered(size: Vec3) -> Self {
        BoxRoomTerrain::new(Aabb::from_center_half_extents(Vec3::ZERO, size * 0.5))
    }

    /// Records an edit. Geometry edits affect subsequent queries.
    pub fn apply(&mut self, edit: TerrainEdit) {
        self.edits.push(edit);
    }

    /// All edits applied so far, in order.
    pub fn edits(&self) -> &[TerrainEdit] {
        &self.edits
    }

    /// Paint edits applied so far, in order.
    pub fn paint_log(&self) -> impl Iterator<Item = &TerrainEdit> {
        self.edits.iter().filter(|e| !e.changes_geometry())
    }
}

impl TerrainQuery for BoxRoomTerrain {
    fn sdf(&self, p: Vec3) -> f32 {
        // Inside the room is open space, so negate the box SDF.
        let mut d = -self.room.sdf(p);
        for e in &self.edits {
            d = e.apply_to_sdf(d, p);
        }
        d
    }

    fn bounds(&self) -> Aabb {
        self.room
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn room() -> BoxRoomTerrain {
        BoxRoomTerrain::centered(Vec3::new(20.0, 10.0, 20.0))
    }

    #[test]
    fn edit_accessors_and_serde() {
        let e = TerrainEdit::SubtractSphere {
            center: Vec3::new(1.0, 2.0, 3.0),
            radius: 3.0,
        };
        assert_eq!(e.center(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(e.radius(), 3.0);
        assert!(e.changes_geometry());
        assert_eq!(e.bounds().min, Vec3::new(-2.0, -1.0, 0.0));
        let p = TerrainEdit::Paint {
            center: Vec3::ZERO,
            radius: 1.0,
            team: 1,
        };
        assert!(!p.changes_geometry());
        for edit in [
            e,
            p,
            TerrainEdit::AddSphere {
                center: Vec3::ONE,
                radius: 0.5,
            },
        ] {
            let s = ron::to_string(&edit).unwrap();
            let back: TerrainEdit = ron::from_str(&s).unwrap();
            assert_eq!(edit, back);
        }
    }

    #[test]
    fn plane_queries() {
        let bounds = Aabb::new(Vec3::splat(-50.0), Vec3::splat(50.0));
        let t = PlaneTerrain::floor(-2.0, bounds);
        assert_eq!(t.sdf(Vec3::new(3.0, 1.0, -7.0)), 3.0);
        assert_eq!(t.sdf(Vec3::new(0.0, -3.0, 0.0)), -1.0);
        assert_eq!(t.normal(Vec3::ZERO), Vec3::Y);
        assert_eq!(t.bounds(), bounds);
        let hit = t.raycast(Vec3::new(0.0, 8.0, 0.0), Vec3::new(0.0, -2.0, 0.0), 100.0);
        assert_eq!(
            hit,
            Some(RayHit {
                point: Vec3::new(0.0, -2.0, 0.0),
                normal: Vec3::Y,
                distance: 10.0
            })
        );
        assert_eq!(t.raycast(Vec3::new(0.0, 8.0, 0.0), Vec3::NEG_Y, 5.0), None);
        assert_eq!(t.raycast(Vec3::new(0.0, 8.0, 0.0), Vec3::Y, 100.0), None);
        assert_eq!(t.raycast(Vec3::new(0.0, 8.0, 0.0), Vec3::ZERO, 100.0), None);
        assert_eq!(
            t.raycast(Vec3::new(0.0, -5.0, 0.0), Vec3::Y, 1.0)
                .unwrap()
                .distance,
            0.0
        );
        assert!(t.sphere_overlap(Vec3::new(0.0, -1.5, 0.0), 1.0));
        assert!(!t.sphere_overlap(Vec3::new(0.0, 0.0, 0.0), 1.0));

        // The generic sphere tracer agrees with the analytic raycast.
        let slanted = PlaneTerrain::new(Vec3::new(0.0, 1.0, 1.0), 0.0, bounds);
        let o = Vec3::new(0.0, 5.0, 5.0);
        let dir = Vec3::new(0.3, -1.0, -0.2);
        let a = slanted.raycast(o, dir, 100.0).unwrap();
        let b = sphere_trace(&slanted, o, dir, 100.0).unwrap();
        assert!((a.distance - b.distance).abs() < 2.0 * HIT_EPS);
    }

    #[test]
    fn box_room_sdf_and_normals() {
        let t = room();
        // Center of the room: 5 u from floor and ceiling.
        assert_eq!(t.sdf(Vec3::ZERO), 5.0);
        // Outside the room is solid.
        assert!(t.sdf(Vec3::new(0.0, 20.0, 0.0)) < 0.0);
        // Normals point into the room.
        let n_floor = t.normal(Vec3::new(0.0, -4.9, 0.0));
        assert!((n_floor - Vec3::Y).length() < 1e-3, "{n_floor:?}");
        let n_wall = t.normal(Vec3::new(9.9, 0.0, 0.0));
        assert!((n_wall - Vec3::NEG_X).length() < 1e-3, "{n_wall:?}");
        assert_eq!(t.bounds(), t.room);
    }

    #[test]
    fn box_room_raycast() {
        let t = room();
        let hit = t.raycast(Vec3::ZERO, Vec3::X, 100.0).unwrap();
        assert!((hit.distance - 10.0).abs() < 2.0 * HIT_EPS, "{hit:?}");
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-3);
        assert!(t.raycast(Vec3::ZERO, Vec3::X, 5.0).is_none());
        // Diagonal towards a corner region.
        let hit = t
            .raycast(Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0), 100.0)
            .unwrap();
        assert!((hit.point.y - 5.0).abs() < 1e-2, "{hit:?}");
        // Starting inside rock hits immediately.
        let hit = t.raycast(Vec3::new(0.0, 50.0, 0.0), Vec3::Y, 10.0).unwrap();
        assert_eq!(hit.distance, 0.0);
    }

    #[test]
    fn box_room_edits() {
        let mut t = room();
        // Add a rock sphere in the middle; a ray from the wall side hits it.
        t.apply(TerrainEdit::AddSphere {
            center: Vec3::ZERO,
            radius: 2.0,
        });
        assert_eq!(t.sdf(Vec3::ZERO), -2.0);
        let hit = t
            .raycast(Vec3::new(-8.0, 0.0, 0.0), Vec3::X, 100.0)
            .unwrap();
        assert!((hit.distance - 6.0).abs() < 2.0 * HIT_EPS, "{hit:?}");
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-3);

        // Blow a crater into the floor; the floor under the crater is now open.
        let floor_point = Vec3::new(5.0, -5.5, 5.0);
        assert!(t.sdf(floor_point) < 0.0);
        t.apply(TerrainEdit::SubtractSphere {
            center: Vec3::new(5.0, -5.0, 5.0),
            radius: 3.0,
        });
        assert!(t.sdf(floor_point) > 0.0);
        let down = t
            .raycast(Vec3::new(5.0, 0.0, 5.0), Vec3::NEG_Y, 100.0)
            .unwrap();
        assert!((down.distance - 8.0).abs() < 1e-2, "{down:?}");

        // Paint does not change geometry but is logged.
        let before = t.sdf(Vec3::new(1.0, 1.0, 1.0));
        t.apply(TerrainEdit::Paint {
            center: Vec3::ZERO,
            radius: 4.0,
            team: 2,
        });
        assert_eq!(t.sdf(Vec3::new(1.0, 1.0, 1.0)), before);
        assert_eq!(t.edits().len(), 3);
        assert_eq!(t.paint_log().count(), 1);
    }

    #[test]
    fn trait_object_usable() {
        let t: Box<dyn TerrainQuery> = Box::new(room());
        assert!(t.raycast(Vec3::ZERO, Vec3::Z, 20.0).is_some());
        assert!(t.sphere_overlap(Vec3::new(0.0, 4.5, 0.0), 1.0));
    }
}
