//! Integer-hashed gradient noise (improved-Perlin style).
//!
//! Everything here is integer hashing plus IEEE-exact float arithmetic
//! (`+ - *`, `floor`), so the output is bit-identical on every target. The
//! noise is our own code on purpose: a third-party noise crate could change
//! its output between versions and silently change every world.

use ls_math::rng::mix64;
use ls_math::{det, Vec3};

/// Upper bound on the gradient magnitude of [`gradient_noise`] (per unit of
/// input distance). The measured maximum is about 3.26 (10^7 samples, central differences); the bound keeps a ~10% margin and is checked by a test; the
/// generator divides by bounds built from this constant to keep its SDF
/// 1-Lipschitz.
pub const NOISE_LIPSCHITZ: f32 = 3.6;

/// Hashes an integer lattice point and a seed into 64 well-mixed bits.
#[inline]
pub fn hash3(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    mix64(
        seed.wrapping_add((x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .wrapping_add((y as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
            .wrapping_add((z as u32 as u64).wrapping_mul(0x1656_67B1_9E37_79F9)),
    )
}

/// Hashes a 2D lattice point.
#[inline]
pub fn hash2(seed: u64, x: i32, z: i32) -> u64 {
    hash3(seed ^ 0x2D2D_2D2D_2D2D_2D2D, x, 0x5bd1_e995, z)
}

/// Uniform `f32` in `[0, 1)` from the top 24 bits of a hash (exact conversion).
#[inline]
pub fn unit_f32(h: u64) -> f32 {
    (h >> 40) as f32 * (1.0 / 16_777_216.0)
}

/// A second independent uniform value from the same hash (bits 16..40).
#[inline]
pub fn unit_f32_b(h: u64) -> f32 {
    ((h >> 16) & 0xff_ffff) as f32 * (1.0 / 16_777_216.0)
}

/// The 12 cube-edge gradients of improved Perlin noise.
const GRADS: [[f32; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

#[inline]
fn fade(t: f32) -> f32 {
    // 6t^5 - 15t^4 + 10t^3, Horner form with plain multiplies and adds.
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn grad_dot(seed: u64, ix: i32, iy: i32, iz: i32, dx: f32, dy: f32, dz: f32) -> f32 {
    let h = hash3(seed, ix, iy, iz);
    let g = GRADS[((h >> 32) as u32 % 12) as usize];
    g[0] * dx + g[1] * dy + g[2] * dz
}

/// 3D gradient noise in roughly `[-1, 1]`, zero at every integer lattice point.
pub fn gradient_noise(seed: u64, p: Vec3) -> f32 {
    let fx = det::floor(p.x);
    let fy = det::floor(p.y);
    let fz = det::floor(p.z);
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (dx, dy, dz) = (p.x - fx, p.y - fy, p.z - fz);
    let (u, v, w) = (fade(dx), fade(dy), fade(dz));

    let n000 = grad_dot(seed, ix, iy, iz, dx, dy, dz);
    let n100 = grad_dot(seed, ix + 1, iy, iz, dx - 1.0, dy, dz);
    let n010 = grad_dot(seed, ix, iy + 1, iz, dx, dy - 1.0, dz);
    let n110 = grad_dot(seed, ix + 1, iy + 1, iz, dx - 1.0, dy - 1.0, dz);
    let n001 = grad_dot(seed, ix, iy, iz + 1, dx, dy, dz - 1.0);
    let n101 = grad_dot(seed, ix + 1, iy, iz + 1, dx - 1.0, dy, dz - 1.0);
    let n011 = grad_dot(seed, ix, iy + 1, iz + 1, dx, dy - 1.0, dz - 1.0);
    let n111 = grad_dot(seed, ix + 1, iy + 1, iz + 1, dx - 1.0, dy - 1.0, dz - 1.0);

    let x00 = lerp(n000, n100, u);
    let x10 = lerp(n010, n110, u);
    let x01 = lerp(n001, n101, u);
    let x11 = lerp(n011, n111, u);
    let y0 = lerp(x00, x10, v);
    let y1 = lerp(x01, x11, v);
    lerp(y0, y1, w)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn zero_on_lattice_and_bounded() {
        let mut rng = ls_math::SplitMix64::new(3);
        for _ in 0..1000 {
            let p = Vec3::new(
                (rng.next_u32() % 200) as f32 - 100.0,
                (rng.next_u32() % 200) as f32 - 100.0,
                (rng.next_u32() % 200) as f32 - 100.0,
            );
            assert_eq!(gradient_noise(9, p), 0.0);
        }
        let mut max = 0.0f32;
        for _ in 0..20000 {
            let p = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 64.0;
            let n = gradient_noise(9, p);
            assert!(n.is_finite());
            max = max.max(n.abs());
        }
        assert!(max <= 1.1 && max > 0.4, "{max}");
    }

    #[test]
    fn lipschitz_bound_holds() {
        let mut rng = ls_math::SplitMix64::new(11);
        let h = 1.0 / 256.0;
        let mut worst = 0.0f32;
        for _ in 0..100_000 {
            let p = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 32.0;
            let dir = Vec3::new(
                rng.next_f32() - 0.5,
                rng.next_f32() - 0.5,
                rng.next_f32() - 0.5,
            )
            .normalize();
            let g = (gradient_noise(5, p + dir * h) - gradient_noise(5, p)) / h;
            worst = worst.max(g.abs());
        }
        assert!(worst < NOISE_LIPSCHITZ, "measured {worst}");
    }

    #[test]
    fn seeds_differ_and_are_stable() {
        let p = Vec3::new(1.3, 2.7, -0.4);
        assert_ne!(gradient_noise(1, p), gradient_noise(2, p));
        assert_eq!(
            gradient_noise(1, p).to_bits(),
            gradient_noise(1, p).to_bits()
        );
        // Hash is well spread over unit values.
        let mut sum = 0.0f32;
        for i in 0..1000 {
            sum += unit_f32(hash3(1, i, 0, 0));
        }
        assert!((sum / 1000.0 - 0.5).abs() < 0.05);
    }
}
