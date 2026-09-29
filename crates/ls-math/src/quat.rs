//! Unit quaternions for 6DOF orientation.

use core::ops::{Mul, MulAssign, Neg};

use serde::{Deserialize, Serialize};

use crate::{det, Mat3, Vec3};

/// A rotation quaternion `w + xi + yj + zk`.
///
/// Conventions (shared by the whole project, see `docs/architecture/contracts.md`):
/// right-handed coordinates, `+Y` up, ship/camera forward is `-Z`, right is `+X`.
/// `a * b` means "rotate by `b`, then by `a`".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat::from_xyzw(0.0, 0.0, 0.0, 1.0);

    #[inline]
    pub const fn from_xyzw(x: f32, y: f32, z: f32, w: f32) -> Self {
        Quat { x, y, z, w }
    }

    /// Raw bit patterns `[x, y, z, w]`, handy for hashing simulation state.
    #[inline]
    pub fn to_bits(self) -> [u32; 4] {
        [
            self.x.to_bits(),
            self.y.to_bits(),
            self.z.to_bits(),
            self.w.to_bits(),
        ]
    }

    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    /// Vector part `(x, y, z)`.
    #[inline]
    pub fn xyz(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// Rotation of `angle` radians about `axis` (normalized internally).
    /// A zero axis yields the identity.
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
        let Some(axis) = axis.try_normalize() else {
            return Quat::IDENTITY;
        };
        let (s, c) = det::sin_cos(angle * 0.5);
        Quat::from_xyzw(axis.x * s, axis.y * s, axis.z * s, c)
    }

    /// Rotation given as a rotation vector: direction is the axis, length the
    /// angle in radians. Used to integrate angular velocity.
    pub fn from_scaled_axis(v: Vec3) -> Quat {
        let angle = v.length();
        if angle < 1e-8 {
            // Small-angle approximation, renormalized.
            Quat::from_xyzw(v.x * 0.5, v.y * 0.5, v.z * 0.5, 1.0).normalize()
        } else {
            Quat::from_axis_angle(v / angle, angle)
        }
    }

    /// Axis and angle (radians, in `[0, 2pi]`) of this unit quaternion. The
    /// axis is `+X` for (near-)identity rotations.
    pub fn to_axis_angle(self) -> (Vec3, f32) {
        let q = self.normalize();
        let angle = 2.0 * det::acos(q.w);
        let axis = q.xyz().try_normalize().unwrap_or(Vec3::X);
        (axis, angle)
    }

    /// Shortest rotation taking unit vector `from` onto unit vector `to`.
    pub fn from_rotation_arc(from: Vec3, to: Vec3) -> Quat {
        let d = from.dot(to);
        if d < -0.999_999 {
            // Opposite: rotate 180 degrees about any perpendicular axis.
            return Quat::from_axis_angle(from.any_orthonormal(), det::PI);
        }
        let c = from.cross(to);
        Quat::from_xyzw(c.x, c.y, c.z, 1.0 + d).normalize()
    }

    #[inline]
    pub fn dot(self, o: Quat) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w
    }

    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f32 {
        det::sqrt(self.length_squared())
    }

    /// Unit-length copy. A zero or non-finite quaternion becomes the identity.
    pub fn normalize(self) -> Quat {
        let len = self.length();
        if len > 1e-20 && len.is_finite() {
            let inv = 1.0 / len;
            Quat::from_xyzw(self.x * inv, self.y * inv, self.z * inv, self.w * inv)
        } else {
            Quat::IDENTITY
        }
    }

    /// Conjugate; equals the inverse for unit quaternions.
    #[inline]
    pub fn conjugate(self) -> Quat {
        Quat::from_xyzw(-self.x, -self.y, -self.z, self.w)
    }

    /// Inverse of a (possibly non-unit) quaternion.
    #[inline]
    pub fn inverse(self) -> Quat {
        let l2 = self.length_squared();
        let c = self.conjugate();
        Quat::from_xyzw(c.x / l2, c.y / l2, c.z / l2, c.w / l2)
    }

    /// Hamilton product `self * o` (apply `o` first, then `self`).
    #[inline]
    pub fn mul_quat(self, o: Quat) -> Quat {
        let (a, b) = (self, o);
        Quat::from_xyzw(
            a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
            a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        )
    }

    /// Rotates a vector by this unit quaternion.
    #[inline]
    pub fn rotate(self, v: Vec3) -> Vec3 {
        // v' = v + w * t + u x t, with t = 2 (u x v).
        let u = self.xyz();
        let t = u.cross(v) * 2.0;
        v + t * self.w + u.cross(t)
    }

    /// Local forward direction (`-Z`) in world space.
    #[inline]
    pub fn forward(self) -> Vec3 {
        self.rotate(Vec3::NEG_Z)
    }

    /// Local right direction (`+X`) in world space.
    #[inline]
    pub fn right(self) -> Vec3 {
        self.rotate(Vec3::X)
    }

    /// Local up direction (`+Y`) in world space.
    #[inline]
    pub fn up(self) -> Vec3 {
        self.rotate(Vec3::Y)
    }

    /// Integrates a body-frame angular velocity `omega` (rad/s per local axis)
    /// over `dt` seconds and renormalizes.
    #[inline]
    pub fn integrate_local(self, omega: Vec3, dt: f32) -> Quat {
        (self * Quat::from_scaled_axis(omega * dt)).normalize()
    }

    /// Integrates a world-frame angular velocity over `dt` seconds and renormalizes.
    #[inline]
    pub fn integrate_world(self, omega: Vec3, dt: f32) -> Quat {
        (Quat::from_scaled_axis(omega * dt) * self).normalize()
    }

    /// Normalized linear interpolation along the shortest path.
    pub fn nlerp(self, o: Quat, t: f32) -> Quat {
        let o = if self.dot(o) < 0.0 { -o } else { o };
        Quat::from_xyzw(
            det::lerp(self.x, o.x, t),
            det::lerp(self.y, o.y, t),
            det::lerp(self.z, o.z, t),
            det::lerp(self.w, o.w, t),
        )
        .normalize()
    }

    /// Spherical linear interpolation along the shortest path. Falls back to
    /// [`Quat::nlerp`] when the rotations are nearly identical.
    pub fn slerp(self, o: Quat, t: f32) -> Quat {
        let mut d = self.dot(o);
        let mut o = o;
        if d < 0.0 {
            d = -d;
            o = -o;
        }
        if d > 0.9995 {
            return self.nlerp(o, t);
        }
        let theta0 = det::acos(d);
        let theta = theta0 * t;
        let sin0 = det::sin(theta0);
        let s1 = det::sin(theta) / sin0;
        let s0 = det::cos(theta) - d * s1;
        Quat::from_xyzw(
            self.x * s0 + o.x * s1,
            self.y * s0 + o.y * s1,
            self.z * s0 + o.z * s1,
            self.w * s0 + o.w * s1,
        )
        .normalize()
    }

    /// Angle in radians between two rotations, in `[0, pi]`.
    pub fn angle_between(self, o: Quat) -> f32 {
        2.0 * det::acos(self.dot(o).abs())
    }

    /// Rotation matrix of this unit quaternion.
    pub fn to_mat3(self) -> Mat3 {
        let Quat { x, y, z, w } = self;
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);
        Mat3::from_cols(
            Vec3::new(1.0 - (yy + zz), xy + wz, xz - wy),
            Vec3::new(xy - wz, 1.0 - (xx + zz), yz + wx),
            Vec3::new(xz + wy, yz - wx, 1.0 - (xx + yy)),
        )
    }

    /// Unit quaternion from a rotation matrix (Shepperd's method).
    pub fn from_mat3(m: &Mat3) -> Quat {
        let (m00, m01, m02) = (m.x_axis.x, m.y_axis.x, m.z_axis.x);
        let (m10, m11, m12) = (m.x_axis.y, m.y_axis.y, m.z_axis.y);
        let (m20, m21, m22) = (m.x_axis.z, m.y_axis.z, m.z_axis.z);
        let trace = m00 + m11 + m22;
        let q = if trace > 0.0 {
            let s = det::sqrt(trace + 1.0) * 2.0;
            Quat::from_xyzw((m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s)
        } else if m00 > m11 && m00 > m22 {
            let s = det::sqrt(1.0 + m00 - m11 - m22) * 2.0;
            Quat::from_xyzw(0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s)
        } else if m11 > m22 {
            let s = det::sqrt(1.0 + m11 - m00 - m22) * 2.0;
            Quat::from_xyzw((m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s)
        } else {
            let s = det::sqrt(1.0 + m22 - m00 - m11) * 2.0;
            Quat::from_xyzw((m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s)
        };
        q.normalize()
    }
}

impl Mul for Quat {
    type Output = Quat;
    #[inline]
    fn mul(self, o: Quat) -> Quat {
        self.mul_quat(o)
    }
}

impl MulAssign for Quat {
    #[inline]
    fn mul_assign(&mut self, o: Quat) {
        *self = self.mul_quat(o);
    }
}

impl Mul<Vec3> for Quat {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        self.rotate(v)
    }
}

impl Neg for Quat {
    type Output = Quat;
    #[inline]
    fn neg(self) -> Quat {
        Quat::from_xyzw(-self.x, -self.y, -self.z, -self.w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::det::PI;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn vclose(a: Vec3, b: Vec3, eps: f32) -> bool {
        (a - b).abs().max_element() <= eps
    }

    fn qclose(a: Quat, b: Quat, eps: f32) -> bool {
        // q and -q are the same rotation.
        1.0 - a.dot(b).abs() <= eps
    }

    #[test]
    fn identity() {
        let v = Vec3::new(1.0, -2.0, 3.0);
        assert_eq!(Quat::IDENTITY.rotate(v), v);
        assert_eq!(Quat::default(), Quat::IDENTITY);
        assert_eq!(Quat::IDENTITY.forward(), Vec3::NEG_Z);
        assert_eq!(Quat::IDENTITY.right(), Vec3::X);
        assert_eq!(Quat::IDENTITY.up(), Vec3::Y);
        let q = Quat::from_axis_angle(Vec3::new(1.0, 2.0, 3.0), 0.7);
        assert!(qclose(q * Quat::IDENTITY, q, 1e-7));
        assert_eq!(Quat::from_axis_angle(Vec3::ZERO, 1.0), Quat::IDENTITY);
    }

    #[test]
    fn axis_angle_rotation() {
        let q = Quat::from_axis_angle(Vec3::Z, PI / 2.0);
        assert!(vclose(q.rotate(Vec3::X), Vec3::Y, 1e-6));
        assert!(vclose(q * Vec3::Y, Vec3::NEG_X, 1e-6));
        let yaw = Quat::from_axis_angle(Vec3::Y, PI / 2.0);
        // Yawing left by 90 degrees turns forward (-Z) into -X.
        assert!(vclose(yaw.forward(), Vec3::NEG_X, 1e-6));
        let (axis, angle) = Quat::from_axis_angle(Vec3::new(0.0, 3.0, 0.0), 1.25).to_axis_angle();
        assert!(vclose(axis, Vec3::Y, 1e-6));
        assert!((angle - 1.25).abs() < 1e-5);
    }

    #[test]
    fn multiplication_composes() {
        let a = Quat::from_axis_angle(Vec3::X, 0.4);
        let b = Quat::from_axis_angle(Vec3::new(0.3, 1.0, -0.2), 1.1);
        let v = Vec3::new(0.5, -1.5, 2.0);
        assert!(vclose((a * b).rotate(v), a.rotate(b.rotate(v)), 1e-5));
        let mut c = a;
        c *= b;
        assert_eq!(c, a * b);
        // Inverse undoes rotation.
        assert!(vclose(a.conjugate().rotate(a.rotate(v)), v, 1e-5));
        assert!(qclose(b * b.inverse(), Quat::IDENTITY, 1e-6));
        assert!((b.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize() {
        let q = Quat::from_xyzw(1.0, 2.0, 3.0, 4.0).normalize();
        assert!((q.length() - 1.0).abs() < 1e-6);
        assert_eq!(
            Quat::from_xyzw(0.0, 0.0, 0.0, 0.0).normalize(),
            Quat::IDENTITY
        );
        assert_eq!(
            Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0).normalize(),
            Quat::IDENTITY
        );
    }

    #[test]
    fn slerp_endpoints_and_midpoint() {
        let a = Quat::from_axis_angle(Vec3::Y, 0.0);
        let b = Quat::from_axis_angle(Vec3::Y, PI / 2.0);
        assert!(qclose(a.slerp(b, 0.0), a, 1e-6));
        assert!(qclose(a.slerp(b, 1.0), b, 1e-6));
        let mid = a.slerp(b, 0.5);
        assert!(qclose(mid, Quat::from_axis_angle(Vec3::Y, PI / 4.0), 1e-6));
        // Shortest path: slerp towards -b gives the same result.
        assert!(qclose(a.slerp(-b, 0.5), mid, 1e-6));
        // Near-identical rotations use nlerp without NaN.
        let c = Quat::from_axis_angle(Vec3::Y, 1e-4);
        let r = a.slerp(c, 0.5);
        assert!(r.length().is_finite());
        assert!((a.angle_between(b) - PI / 2.0).abs() < 1e-5);
    }

    #[test]
    fn matrix_roundtrip() {
        let qs = [
            Quat::IDENTITY,
            Quat::from_axis_angle(Vec3::X, 2.9),
            Quat::from_axis_angle(Vec3::Y, -3.0),
            Quat::from_axis_angle(Vec3::Z, 3.1),
            Quat::from_axis_angle(Vec3::new(1.0, -2.0, 0.5), 1.7),
        ];
        let v = Vec3::new(0.3, -0.7, 1.9);
        for q in qs {
            let m = q.to_mat3();
            assert!(vclose(m * v, q.rotate(v), 1e-5));
            assert!((m.determinant() - 1.0).abs() < 1e-5);
            assert!(qclose(Quat::from_mat3(&m), q, 1e-6));
        }
    }

    #[test]
    fn rotation_arc_and_integration() {
        let from = Vec3::X;
        let to = Vec3::new(0.0, 1.0, 1.0).normalize();
        assert!(vclose(
            Quat::from_rotation_arc(from, to).rotate(from),
            to,
            1e-6
        ));
        assert!(vclose(
            Quat::from_rotation_arc(from, -from).rotate(from),
            -from,
            1e-6
        ));

        // Spinning about local Y at pi/2 rad/s for 60 ticks of 1/60 s = 90 degrees.
        let mut q = Quat::IDENTITY;
        for _ in 0..60 {
            q = q.integrate_local(Vec3::new(0.0, PI / 2.0, 0.0), crate::DT);
        }
        assert!(vclose(q.forward(), Vec3::NEG_X, 1e-4));
        let w = Quat::IDENTITY.integrate_world(Vec3::new(0.0, 0.0, PI), 0.5);
        assert!(vclose(w.rotate(Vec3::X), Vec3::Y, 1e-6));
        // Tiny rotation vectors stay finite.
        assert!(Quat::from_scaled_axis(Vec3::splat(1e-12))
            .length()
            .is_finite());
    }
}
