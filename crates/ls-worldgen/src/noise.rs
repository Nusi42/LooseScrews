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

/// `(det::floor(x), det::floor(x) as i32)`, bit for bit, without the libm
/// call for the usual magnitudes.
#[inline(always)]
fn floor_parts(x: f32) -> (f32, i32) {
    if x.abs() < 8_388_608.0 {
        // |x| < 2^23: the truncation and the floor are exact integers.
        let t = x as i32;
        let i = if x < t as f32 { t - 1 } else { t };
        // An integral x (including -0.0) is its own floor.
        let f = i as f32;
        (if f == x { x } else { f }, i)
    } else {
        let f = det::floor(x);
        (f, f as i32)
    }
}

#[inline]
fn fade(t: f32) -> f32 {
    // 6t^5 - 15t^4 + 10t^3, Horner form with plain multiplies and adds.
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Gradient vector of the lattice point `(ix, iy, iz)` for `seed`.
#[inline]
pub fn lattice_gradient(seed: u64, ix: i32, iy: i32, iz: i32) -> [f32; 3] {
    let h = hash3(seed, ix, iy, iz);
    GRADS[((h >> 32) as u32 % 12) as usize]
}

/// The gradients of the 8 corners of a lattice cell `(ix, iy, iz)`, split
/// into components. Lane `l < 4` is the corner
/// `(ix, iy + (l & 1), iz + (l >> 1))`, lane `l + 4` the same corner one step
/// along +x. This layout lets the compiler evaluate four corners at once;
/// every lane still performs exactly the scalar operations of the reference
/// formula.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellGradients {
    pub x: [f32; 8],
    pub y: [f32; 8],
    pub z: [f32; 8],
}

/// [`CellGradients`] of a lattice cell for `seed`.
#[inline]
pub fn cell_gradients(seed: u64, ix: i32, iy: i32, iz: i32) -> CellGradients {
    let mut g = CellGradients {
        x: [0.0; 8],
        y: [0.0; 8],
        z: [0.0; 8],
    };
    for l in 0..8 {
        let v = lattice_gradient(
            seed,
            ix + (l >> 2) as i32,
            iy + (l & 1) as i32,
            iz + ((l >> 1) & 1) as i32,
        );
        g.x[l] = v[0];
        g.y[l] = v[1];
        g.z[l] = v[2];
    }
    g
}

/// 3D gradient noise in roughly `[-1, 1]`, zero at every integer lattice point.
pub fn gradient_noise(seed: u64, p: Vec3) -> f32 {
    gradient_noise_with(p, |x, y, z| cell_gradients(seed, x, y, z))
}

/// [`gradient_noise`] with the gradients of the point's lattice cell
/// supplied by `grads` (for example from a precomputed table).
/// Bit-identical to [`gradient_noise`] when `grads` returns
/// [`cell_gradients`] values.
#[inline(always)]
pub fn gradient_noise_with(p: Vec3, grads: impl Fn(i32, i32, i32) -> CellGradients) -> f32 {
    let (fx, ix) = floor_parts(p.x);
    let (fy, iy) = floor_parts(p.y);
    let (fz, iz) = floor_parts(p.z);
    let (dx, dy, dz) = (p.x - fx, p.y - fy, p.z - fz);
    let (u, v, w) = (fade(dx), fade(dy), fade(dz));

    let g = grads(ix, iy, iz);
    // Corner terms g . (dx - i, dy - j, dz - k), lanes as in CellGradients:
    // `lo` holds the corners with i = 0, `hi` those with i = 1.
    let ey = [dy, dy - 1.0, dy, dy - 1.0];
    let ez = [dz, dz, dz - 1.0, dz - 1.0];
    let dx1 = dx - 1.0;
    let lo: [f32; 4] = std::array::from_fn(|l| g.x[l] * dx + g.y[l] * ey[l] + g.z[l] * ez[l]);
    let hi: [f32; 4] =
        std::array::from_fn(|l| g.x[l + 4] * dx1 + g.y[l + 4] * ey[l] + g.z[l + 4] * ez[l]);
    // x00, x10, x01, x11
    let x: [f32; 4] = std::array::from_fn(|l| lerp(lo[l], hi[l], u));
    let y0 = lerp(x[0], x[1], v);
    let y1 = lerp(x[2], x[3], v);
    lerp(y0, y1, w)
}

/// [`gradient_noise_with`] at the points `(xs[i], y, z)`, written to `out`.
///
/// Every lattice gradient has exactly one zero component, so each corner
/// term `g.x * dx + g.y * dy + g.z * dz` equals `g.x * dx + (g.y * dy + g.z *
/// dz)` exactly, up to the sign of a zero result; the bracket only changes
/// when the lattice cell does, so it is computed once per cell. Everything
/// else is the same arithmetic in the same order, so each output equals
/// [`gradient_noise_with`] exactly, except that a zero may come out with the
/// other sign.
pub fn gradient_noise_row(
    xs: &[f32],
    y: f32,
    z: f32,
    grads: impl Fn(i32, i32, i32) -> CellGradients,
    out: &mut [f32],
) {
    assert_eq!(xs.len(), out.len());
    let (fy, iy) = floor_parts(y);
    let (fz, iz) = floor_parts(z);
    let (dy, dz) = (y - fy, z - fz);
    let (v, w) = (fade(dy), fade(dz));
    let ey = [dy, dy - 1.0, dy, dy - 1.0];
    let ez = [dz, dz, dz - 1.0, dz - 1.0];
    // Per lane (as in CellGradients): gradient x component and the y/z bracket.
    let mut cell = None;
    let mut gx = [0.0f32; 8];
    let mut rest = [0.0f32; 8];
    for (x, o) in xs.iter().zip(out.iter_mut()) {
        let (fx, ix) = floor_parts(*x);
        if cell != Some(ix) {
            cell = Some(ix);
            let g = grads(ix, iy, iz);
            gx = g.x;
            rest = std::array::from_fn(|l| g.y[l] * ey[l & 3] + g.z[l] * ez[l & 3]);
        }
        let dx = x - fx;
        let u = fade(dx);
        let dx1 = dx - 1.0;
        let x: [f32; 4] =
            std::array::from_fn(|l| lerp(gx[l] * dx + rest[l], gx[l + 4] * dx1 + rest[l + 4], u));
        let y0 = lerp(x[0], x[1], v);
        let y1 = lerp(x[2], x[3], v);
        *o = lerp(y0, y1, w);
    }
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
    fn floor_parts_matches_libm() {
        let mut rng = ls_math::SplitMix64::new(21);
        let special = [
            0.0f32,
            -0.0,
            0.25,
            -0.25,
            1.0,
            -1.0,
            8_388_607.5,
            -8_388_607.5,
            8_388_608.0,
            -8_388_608.0,
            3e9,
            -3e9,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        let random = (0..20_000).map(|_| (rng.next_f32() - 0.5) * 400.0);
        for x in special.into_iter().chain(random) {
            let (f, i) = floor_parts(x);
            assert_eq!(f.to_bits(), det::floor(x).to_bits(), "{x}");
            assert_eq!(i, det::floor(x) as i32, "{x}");
        }
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
