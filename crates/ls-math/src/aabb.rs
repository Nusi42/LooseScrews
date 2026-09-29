//! Axis-aligned bounding boxes.

use serde::{Deserialize, Serialize};

use crate::Vec3;

/// Axis-aligned box given by its minimum and maximum corners (inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// Box spanning the two corners in any order.
    #[inline]
    pub fn new(a: Vec3, b: Vec3) -> Self {
        Aabb {
            min: a.min(b),
            max: a.max(b),
        }
    }

    #[inline]
    pub fn from_center_half_extents(center: Vec3, half: Vec3) -> Self {
        let half = half.abs();
        Aabb {
            min: center - half,
            max: center + half,
        }
    }

    /// Smallest box containing all points, or `None` for an empty iterator.
    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Option<Self> {
        let mut it = points.into_iter();
        let first = it.next()?;
        let mut b = Aabb {
            min: first,
            max: first,
        };
        for p in it {
            b = b.include_point(p);
        }
        Some(b)
    }

    #[inline]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[inline]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    #[inline]
    pub fn half_extents(&self) -> Vec3 {
        self.size() * 0.5
    }

    #[inline]
    pub fn volume(&self) -> f32 {
        let s = self.size();
        s.x * s.y * s.z
    }

    #[inline]
    pub fn contains(&self, p: Vec3) -> bool {
        p.x >= self.min.x
            && p.y >= self.min.y
            && p.z >= self.min.z
            && p.x <= self.max.x
            && p.y <= self.max.y
            && p.z <= self.max.z
    }

    /// True if the boxes overlap or touch.
    #[inline]
    pub fn intersects(&self, o: &Aabb) -> bool {
        self.min.x <= o.max.x
            && self.max.x >= o.min.x
            && self.min.y <= o.max.y
            && self.max.y >= o.min.y
            && self.min.z <= o.max.z
            && self.max.z >= o.min.z
    }

    /// True if the sphere overlaps or touches the box.
    #[inline]
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.closest_point(center).distance_squared(center) <= radius * radius
    }

    #[inline]
    pub fn union(&self, o: &Aabb) -> Aabb {
        Aabb {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }

    #[inline]
    pub fn include_point(&self, p: Vec3) -> Aabb {
        Aabb {
            min: self.min.min(p),
            max: self.max.max(p),
        }
    }

    /// Box grown by `amount` on every side (shrinks for negative values).
    #[inline]
    pub fn expand(&self, amount: f32) -> Aabb {
        let d = Vec3::splat(amount);
        Aabb {
            min: self.min - d,
            max: self.max + d,
        }
    }

    /// Point inside the box closest to `p`.
    #[inline]
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        p.clamp(self.min, self.max)
    }

    /// Signed distance from `p` to the box surface: negative inside.
    pub fn sdf(&self, p: Vec3) -> f32 {
        let q = (p - self.center()).abs() - self.half_extents();
        let outside = q.max(Vec3::ZERO).length();
        let inside = q.max_element().min(0.0);
        outside + inside
    }

    /// Slab test. Returns the entry and exit distances `(t_enter, t_exit)`
    /// along `origin + dir * t`, clipped to `t >= 0`, or `None` on a miss.
    /// `dir` need not be normalized; distances are in units of `|dir|`.
    pub fn ray_intersect(&self, origin: Vec3, dir: Vec3) -> Option<(f32, f32)> {
        let mut t0 = 0.0f32;
        let mut t1 = f32::INFINITY;
        for axis in 0..3 {
            let (o, d, lo, hi) = (origin[axis], dir[axis], self.min[axis], self.max[axis]);
            if d == 0.0 {
                if o < lo || o > hi {
                    return None;
                }
                continue;
            }
            let inv = 1.0 / d;
            let (mut a, mut b) = ((lo - o) * inv, (hi - o) * inv);
            if a > b {
                core::mem::swap(&mut a, &mut b);
            }
            t0 = t0.max(a);
            t1 = t1.min(b);
            if t0 > t1 {
                return None;
            }
        }
        Some((t0, t1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn unit() -> Aabb {
        Aabb::new(Vec3::ONE, -Vec3::ONE)
    }

    #[test]
    fn construction() {
        let b = unit();
        assert_eq!(b.min, -Vec3::ONE);
        assert_eq!(b.max, Vec3::ONE);
        assert_eq!(b.center(), Vec3::ZERO);
        assert_eq!(b.size(), Vec3::splat(2.0));
        assert_eq!(b.half_extents(), Vec3::ONE);
        assert_eq!(b.volume(), 8.0);
        assert_eq!(Aabb::from_center_half_extents(Vec3::ZERO, -Vec3::ONE), b);
        let pts = [
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(0.0, -1.0, 1.0),
        ];
        assert_eq!(Aabb::from_points(pts), Some(b));
        assert_eq!(Aabb::from_points(core::iter::empty()), None);
    }

    #[test]
    fn containment_and_overlap() {
        let b = unit();
        assert!(b.contains(Vec3::ZERO));
        assert!(b.contains(Vec3::ONE));
        assert!(!b.contains(Vec3::new(1.01, 0.0, 0.0)));
        let c = Aabb::new(Vec3::splat(1.0), Vec3::splat(3.0));
        assert!(b.intersects(&c));
        assert!(!b.intersects(&Aabb::new(Vec3::splat(1.1), Vec3::splat(3.0))));
        assert_eq!(b.union(&c), Aabb::new(-Vec3::ONE, Vec3::splat(3.0)));
        assert_eq!(
            b.expand(1.0),
            Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0))
        );
        assert!(b.intersects_sphere(Vec3::new(2.0, 0.0, 0.0), 1.0));
        assert!(!b.intersects_sphere(Vec3::new(2.0, 2.0, 0.0), 1.0));
        assert_eq!(
            b.closest_point(Vec3::new(5.0, 0.5, -5.0)),
            Vec3::new(1.0, 0.5, -1.0)
        );
    }

    #[test]
    fn signed_distance() {
        let b = unit();
        assert_eq!(b.sdf(Vec3::ZERO), -1.0);
        assert_eq!(b.sdf(Vec3::new(3.0, 0.0, 0.0)), 2.0);
        assert_eq!(b.sdf(Vec3::new(4.0, 5.0, 0.0)), 5.0);
        assert_eq!(b.sdf(Vec3::new(0.5, 0.0, 0.0)), -0.5);
    }

    #[test]
    fn rays() {
        let b = unit();
        assert_eq!(
            b.ray_intersect(Vec3::new(-5.0, 0.0, 0.0), Vec3::X),
            Some((4.0, 6.0))
        );
        // Starting inside clips entry to 0.
        assert_eq!(b.ray_intersect(Vec3::ZERO, Vec3::Y), Some((0.0, 1.0)));
        // Parallel ray outside the slab misses.
        assert_eq!(b.ray_intersect(Vec3::new(-5.0, 2.0, 0.0), Vec3::X), None);
        // Pointing away misses.
        assert_eq!(
            b.ray_intersect(Vec3::new(-5.0, 0.0, 0.0), Vec3::NEG_X),
            None
        );
        // Diagonal hit.
        let hit = b.ray_intersect(Vec3::splat(-3.0), Vec3::ONE).unwrap();
        assert_eq!(hit, (2.0, 4.0));
    }
}
