//! Deterministic scalar math.
//!
//! The standard library's `f32::sin` and friends call into the platform's C
//! math library (or LLVM intrinsics), whose results differ between x86_64,
//! aarch64 and wasm32. These wrappers use the pure-Rust `libm` crate instead,
//! which gives identical bits on every target. Deterministic crates must call
//! these instead of the `f32` methods.
//!
//! `sqrt`, `floor`, `ceil`, `abs`, `min`, `max` are exact in IEEE 754 anyway;
//! they are routed through `libm` too so there is one obvious place to look.

/// Sine (radians).
#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

/// Cosine (radians).
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

/// Sine and cosine at once, `(sin, cos)`.
#[inline]
pub fn sin_cos(x: f32) -> (f32, f32) {
    libm::sincosf(x)
}

/// Tangent (radians).
#[inline]
pub fn tan(x: f32) -> f32 {
    libm::tanf(x)
}

/// Arc sine, result in `[-pi/2, pi/2]`. Input is clamped to `[-1, 1]`
/// so tiny rounding errors never produce NaN.
#[inline]
pub fn asin(x: f32) -> f32 {
    libm::asinf(x.clamp(-1.0, 1.0))
}

/// Arc cosine, result in `[0, pi]`. Input is clamped to `[-1, 1]`.
#[inline]
pub fn acos(x: f32) -> f32 {
    libm::acosf(x.clamp(-1.0, 1.0))
}

/// Four-quadrant arc tangent of `y / x`, result in `[-pi, pi]`.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

/// Square root (correctly rounded).
#[inline]
pub fn sqrt(x: f32) -> f32 {
    libm::sqrtf(x)
}

/// `e^x`.
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

/// Natural logarithm.
#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

/// `x^y`.
#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// Largest integer value not greater than `x`.
#[inline]
pub fn floor(x: f32) -> f32 {
    libm::floorf(x)
}

/// Smallest integer value not less than `x`.
#[inline]
pub fn ceil(x: f32) -> f32 {
    libm::ceilf(x)
}

/// Round half away from zero.
#[inline]
pub fn round(x: f32) -> f32 {
    libm::roundf(x)
}

/// Absolute value.
#[inline]
pub fn abs(x: f32) -> f32 {
    libm::fabsf(x)
}

/// Floating-point remainder of `x / y` with the sign of `x`.
#[inline]
pub fn fmod(x: f32, y: f32) -> f32 {
    libm::fmodf(x, y)
}

/// Pi as `f32`.
pub const PI: f32 = core::f32::consts::PI;
/// Tau (2 pi) as `f32`.
pub const TAU: f32 = core::f32::consts::TAU;

/// Wraps an angle into `[-pi, pi)`.
#[inline]
pub fn wrap_angle(a: f32) -> f32 {
    let r = a - TAU * floor((a + PI) / TAU);
    // Guard against rounding pushing the value to exactly +pi.
    if r >= PI {
        r - TAU
    } else {
        r
    }
}

/// Linear interpolation `a + (b - a) * t` (no fused multiply-add).
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn trig_basics() {
        assert_eq!(sin(0.0), 0.0);
        assert_eq!(cos(0.0), 1.0);
        assert!(close(sin(PI / 2.0), 1.0, 1e-6));
        assert!(close(cos(PI), -1.0, 1e-6));
        assert!(close(tan(PI / 4.0), 1.0, 1e-6));
        let (s, c) = sin_cos(0.7);
        assert_eq!(s, sin(0.7));
        assert_eq!(c, cos(0.7));
    }

    #[test]
    fn inverse_trig() {
        assert!(close(atan2(1.0, 1.0), PI / 4.0, 1e-6));
        assert!(close(atan2(1.0, -1.0), 3.0 * PI / 4.0, 1e-6));
        assert!(close(atan2(-1.0, 0.0), -PI / 2.0, 1e-6));
        assert!(close(asin(1.0), PI / 2.0, 1e-6));
        assert!(close(acos(-1.0), PI, 1e-6));
        // Clamped: slightly out of range input is not NaN.
        assert!(!acos(1.000_001).is_nan());
        assert!(!asin(-1.000_001).is_nan());
    }

    #[test]
    fn exp_log_pow_sqrt() {
        assert_eq!(exp(0.0), 1.0);
        assert!(close(exp(1.0), core::f32::consts::E, 1e-6));
        assert!(close(ln(core::f32::consts::E), 1.0, 1e-6));
        assert_eq!(powf(2.0, 10.0), 1024.0);
        assert!(close(powf(9.0, 0.5), 3.0, 1e-6));
        assert_eq!(sqrt(16.0), 4.0);
        assert_eq!(sqrt(2.0), 2.0f32.sqrt()); // both correctly rounded
    }

    #[test]
    fn rounding() {
        assert_eq!(floor(1.7), 1.0);
        assert_eq!(floor(-1.2), -2.0);
        assert_eq!(ceil(1.2), 2.0);
        assert_eq!(ceil(-1.7), -1.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -3.0);
        assert_eq!(abs(-3.5), 3.5);
        assert!(close(fmod(7.5, 2.0), 1.5, 0.0));
    }

    #[test]
    fn wrap_and_lerp() {
        assert!(close(wrap_angle(3.0 * PI), -PI, 1e-5));
        assert!(close(wrap_angle(-3.0 * PI / 2.0), PI / 2.0, 1e-5));
        assert_eq!(wrap_angle(0.5), 0.5);
        let w = wrap_angle(PI);
        assert!((-PI..PI).contains(&w));
        assert_eq!(lerp(2.0, 4.0, 0.5), 3.0);
    }

    /// Bit-exact reference values. These must never change between targets or
    /// dependency upgrades; if one fails after a `libm` bump, pin the old version.
    #[test]
    fn golden_bits() {
        let v = [
            sin(1.0).to_bits(),
            cos(1.0).to_bits(),
            atan2(0.3, -0.8).to_bits(),
            exp(2.5).to_bits(),
            powf(1.7, 3.3).to_bits(),
        ];
        // Recompute via the same functions to check purity, then compare with
        // the recorded constants below.
        let again = [
            sin(1.0).to_bits(),
            cos(1.0).to_bits(),
            atan2(0.3, -0.8).to_bits(),
            exp(2.5).to_bits(),
            powf(1.7, 3.3).to_bits(),
        ];
        assert_eq!(v, again);
        assert_eq!(v, GOLDEN);
    }

    const GOLDEN: [u32; 5] = [
        0x3f57_6aa4,
        0x3f0a_5140,
        0x4032_19c1,
        0x4142_eb7f,
        0x40b8_5854,
    ];
}
