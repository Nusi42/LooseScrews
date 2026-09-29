//! Three-component `f32` vector.

use core::ops::{Add, AddAssign, Div, DivAssign, Index, Mul, MulAssign, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::det;

/// A 3D vector of `f32`. Coordinates are in world units (1 u = one ship length).
///
/// All operations are IEEE-exact element-wise arithmetic; lengths use
/// [`det::sqrt`]. No fused multiply-add is used anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const ONE: Vec3 = Vec3::new(1.0, 1.0, 1.0);
    pub const X: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    pub const Y: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    pub const Z: Vec3 = Vec3::new(0.0, 0.0, 1.0);
    pub const NEG_X: Vec3 = Vec3::new(-1.0, 0.0, 0.0);
    pub const NEG_Y: Vec3 = Vec3::new(0.0, -1.0, 0.0);
    pub const NEG_Z: Vec3 = Vec3::new(0.0, 0.0, -1.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Vec3 { x, y, z }
    }

    /// All three components set to `v`.
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Vec3 { x: v, y: v, z: v }
    }

    #[inline]
    pub const fn from_array(a: [f32; 3]) -> Self {
        Vec3::new(a[0], a[1], a[2])
    }

    #[inline]
    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// Raw bit patterns, handy for hashing simulation state.
    #[inline]
    pub fn to_bits(self) -> [u32; 3] {
        [self.x.to_bits(), self.y.to_bits(), self.z.to_bits()]
    }

    #[inline]
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    #[inline]
    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        det::sqrt(self.length_squared())
    }

    #[inline]
    pub fn distance(self, o: Vec3) -> f32 {
        (self - o).length()
    }

    #[inline]
    pub fn distance_squared(self, o: Vec3) -> f32 {
        (self - o).length_squared()
    }

    /// Unit vector in the same direction, or `None` if the length is zero,
    /// vanishingly small, or not finite.
    #[inline]
    pub fn try_normalize(self) -> Option<Vec3> {
        let len = self.length();
        if len > 1e-20 && len.is_finite() {
            Some(self / len)
        } else {
            None
        }
    }

    /// Unit vector in the same direction, or [`Vec3::ZERO`] if that is not possible.
    #[inline]
    pub fn normalize_or_zero(self) -> Vec3 {
        self.try_normalize().unwrap_or(Vec3::ZERO)
    }

    /// Unit vector in the same direction. Returns [`Vec3::ZERO`] for a zero
    /// vector instead of NaN (same as [`Vec3::normalize_or_zero`]).
    #[inline]
    pub fn normalize(self) -> Vec3 {
        self.normalize_or_zero()
    }

    /// Element-wise product.
    #[inline]
    pub fn mul_elem(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x * o.x, self.y * o.y, self.z * o.z)
    }

    /// Element-wise quotient.
    #[inline]
    pub fn div_elem(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x / o.x, self.y / o.y, self.z / o.z)
    }

    #[inline]
    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    #[inline]
    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    #[inline]
    pub fn clamp(self, lo: Vec3, hi: Vec3) -> Vec3 {
        self.max(lo).min(hi)
    }

    #[inline]
    pub fn abs(self) -> Vec3 {
        Vec3::new(self.x.abs(), self.y.abs(), self.z.abs())
    }

    #[inline]
    pub fn min_element(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }

    #[inline]
    pub fn max_element(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }

    /// Linear interpolation `self + (o - self) * t`.
    #[inline]
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }

    /// Removes the component along the unit vector `n` (projection onto the plane).
    #[inline]
    pub fn reject_from_normal(self, n: Vec3) -> Vec3 {
        self - n * self.dot(n)
    }

    /// Reflects around the plane with unit normal `n`.
    #[inline]
    pub fn reflect(self, n: Vec3) -> Vec3 {
        self - n * (2.0 * self.dot(n))
    }

    /// Scales the vector down so its length is at most `max`.
    #[inline]
    pub fn clamp_length(self, max: f32) -> Vec3 {
        let l2 = self.length_squared();
        if l2 > max * max {
            self * (max / det::sqrt(l2))
        } else {
            self
        }
    }

    /// True if all components are finite.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Some unit vector perpendicular to the unit vector `self`.
    pub fn any_orthonormal(self) -> Vec3 {
        let other = if self.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        self.cross(other).normalize_or_zero()
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    #[inline]
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    #[inline]
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Mul<Vec3> for f32 {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        v * self
    }
}

impl Div<f32> for Vec3 {
    type Output = Vec3;
    #[inline]
    fn div(self, s: f32) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    #[inline]
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

impl AddAssign for Vec3 {
    #[inline]
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl SubAssign for Vec3 {
    #[inline]
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}

impl MulAssign<f32> for Vec3 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}

impl DivAssign<f32> for Vec3 {
    #[inline]
    fn div_assign(&mut self, s: f32) {
        *self = *self / s;
    }
}

impl Index<usize> for Vec3 {
    type Output = f32;
    #[inline]
    fn index(&self, i: usize) -> &f32 {
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            _ => panic!("Vec3 index out of range: {i}"),
        }
    }
}

impl From<[f32; 3]> for Vec3 {
    #[inline]
    fn from(a: [f32; 3]) -> Self {
        Vec3::from_array(a)
    }
}

impl From<Vec3> for [f32; 3] {
    #[inline]
    fn from(v: Vec3) -> Self {
        v.to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn arithmetic() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, -5.0, 6.0);
        assert_eq!(a + b, Vec3::new(5.0, -3.0, 9.0));
        assert_eq!(a - b, Vec3::new(-3.0, 7.0, -3.0));
        assert_eq!(a * 2.0, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(2.0 * a, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(a / 2.0, Vec3::new(0.5, 1.0, 1.5));
        assert_eq!(-a, Vec3::new(-1.0, -2.0, -3.0));
        assert_eq!(a.mul_elem(b), Vec3::new(4.0, -10.0, 18.0));
        assert_eq!(a.div_elem(Vec3::splat(2.0)), a / 2.0);
        let mut c = a;
        c += b;
        c -= a;
        c *= 3.0;
        c /= 3.0;
        assert_eq!(c, b);
        assert_eq!(a[0] + a[1] + a[2], 6.0);
    }

    #[test]
    fn dot_cross() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
        assert_eq!(Vec3::Z.cross(Vec3::X), Vec3::Y);
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(4.0, -5.0, 6.0);
        assert_eq!(a.dot(b), 12.0);
        let c = a.cross(b);
        assert_eq!(c.dot(a), 0.0);
        assert_eq!(c.dot(b), 0.0);
    }

    #[test]
    fn length_normalize() {
        let v = Vec3::new(3.0, 4.0, 12.0);
        assert_eq!(v.length(), 13.0);
        assert_eq!(v.length_squared(), 169.0);
        assert!((v.normalize().length() - 1.0).abs() < 1e-6);
        assert_eq!(Vec3::ZERO.normalize(), Vec3::ZERO);
        assert_eq!(Vec3::ZERO.try_normalize(), None);
        assert_eq!(Vec3::splat(f32::INFINITY).try_normalize(), None);
        assert_eq!(
            Vec3::new(1.0, 0.0, 0.0).distance(Vec3::new(4.0, 4.0, 0.0)),
            5.0
        );
    }

    #[test]
    fn misc() {
        let a = Vec3::new(1.0, 5.0, -3.0);
        let b = Vec3::new(2.0, -1.0, 0.0);
        assert_eq!(a.min(b), Vec3::new(1.0, -1.0, -3.0));
        assert_eq!(a.max(b), Vec3::new(2.0, 5.0, 0.0));
        assert_eq!(a.abs(), Vec3::new(1.0, 5.0, 3.0));
        assert_eq!(a.min_element(), -3.0);
        assert_eq!(a.max_element(), 5.0);
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        assert_eq!(
            Vec3::new(1.0, -1.0, 0.0).reflect(Vec3::Y),
            Vec3::new(1.0, 1.0, 0.0)
        );
        assert_eq!(
            Vec3::new(1.0, 2.0, 3.0).reject_from_normal(Vec3::Y),
            Vec3::new(1.0, 0.0, 3.0)
        );
        assert_eq!(
            Vec3::new(0.0, 10.0, 0.0).clamp_length(2.0),
            Vec3::new(0.0, 2.0, 0.0)
        );
        assert_eq!(
            Vec3::new(0.0, 1.0, 0.0).clamp_length(2.0),
            Vec3::new(0.0, 1.0, 0.0)
        );
        for n in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 1.0, 1.0).normalize(),
        ] {
            let o = n.any_orthonormal();
            assert!(o.dot(n).abs() < 1e-6);
            assert!((o.length() - 1.0).abs() < 1e-6);
        }
        let arr: [f32; 3] = a.into();
        assert_eq!(Vec3::from(arr), a);
        assert!(a.is_finite());
        assert!(!Vec3::new(f32::NAN, 0.0, 0.0).is_finite());
    }

    #[test]
    fn serde_roundtrip() {
        let v = Vec3::new(1.5, -2.25, 1e-7);
        let s = ron::to_string(&v).unwrap();
        let back: Vec3 = ron::from_str(&s).unwrap();
        assert_eq!(v.to_bits(), back.to_bits());
    }
}
