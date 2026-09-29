//! 3x3 matrix, mainly used as a rotation matrix.

use core::ops::Mul;

use serde::{Deserialize, Serialize};

use crate::Vec3;

/// Column-major 3x3 matrix. `x_axis`, `y_axis`, `z_axis` are the columns, so
/// for a rotation matrix they are the images of the basis vectors.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mat3 {
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub z_axis: Vec3,
}

impl Default for Mat3 {
    fn default() -> Self {
        Mat3::IDENTITY
    }
}

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3::from_cols(Vec3::X, Vec3::Y, Vec3::Z);
    pub const ZERO: Mat3 = Mat3::from_cols(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);

    #[inline]
    pub const fn from_cols(x_axis: Vec3, y_axis: Vec3, z_axis: Vec3) -> Self {
        Mat3 {
            x_axis,
            y_axis,
            z_axis,
        }
    }

    /// Row `i` as a vector.
    #[inline]
    pub fn row(&self, i: usize) -> Vec3 {
        Vec3::new(self.x_axis[i], self.y_axis[i], self.z_axis[i])
    }

    #[inline]
    pub fn transpose(&self) -> Mat3 {
        Mat3::from_cols(self.row(0), self.row(1), self.row(2))
    }

    #[inline]
    pub fn mul_vec3(&self, v: Vec3) -> Vec3 {
        self.x_axis * v.x + self.y_axis * v.y + self.z_axis * v.z
    }

    #[inline]
    pub fn mul_mat3(&self, o: &Mat3) -> Mat3 {
        Mat3::from_cols(
            self.mul_vec3(o.x_axis),
            self.mul_vec3(o.y_axis),
            self.mul_vec3(o.z_axis),
        )
    }

    #[inline]
    pub fn determinant(&self) -> f32 {
        self.x_axis.dot(self.y_axis.cross(self.z_axis))
    }

    /// Row-major array of 9 floats.
    pub fn to_rows_array(&self) -> [f32; 9] {
        let (r0, r1, r2) = (self.row(0), self.row(1), self.row(2));
        [r0.x, r0.y, r0.z, r1.x, r1.y, r1.z, r2.x, r2.y, r2.z]
    }

    /// Column-major array of 9 floats (the layout WGSL `mat3x3` expects, before padding).
    pub fn to_cols_array(&self) -> [f32; 9] {
        let (c0, c1, c2) = (self.x_axis, self.y_axis, self.z_axis);
        [c0.x, c0.y, c0.z, c1.x, c1.y, c1.z, c2.x, c2.y, c2.z]
    }
}

impl Mul<Vec3> for Mat3 {
    type Output = Vec3;
    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        self.mul_vec3(v)
    }
}

impl Mul for Mat3 {
    type Output = Mat3;
    #[inline]
    fn mul(self, o: Mat3) -> Mat3 {
        self.mul_mat3(&o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn identity_and_transpose() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(Mat3::IDENTITY * v, v);
        assert_eq!(Mat3::default(), Mat3::IDENTITY);
        let m = Mat3::from_cols(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
            Vec3::new(7.0, 8.0, 10.0),
        );
        assert_eq!(m.transpose().transpose(), m);
        assert_eq!(m.row(0), Vec3::new(1.0, 4.0, 7.0));
        assert_eq!(m * Mat3::IDENTITY, m);
        assert_eq!(m.determinant(), -3.0);
        assert_eq!(m.to_rows_array()[1], 4.0);
        assert_eq!(m.to_cols_array()[1], 2.0);
        assert_eq!(Mat3::ZERO * v, Vec3::ZERO);
    }

    #[test]
    fn multiply() {
        // 90 degree rotation about Z: X -> Y, Y -> -X.
        let rz = Mat3::from_cols(Vec3::Y, Vec3::NEG_X, Vec3::Z);
        assert_eq!(rz * Vec3::X, Vec3::Y);
        assert_eq!((rz * rz) * Vec3::X, Vec3::NEG_X);
        assert_eq!(rz.transpose() * rz, Mat3::IDENTITY);
    }
}
