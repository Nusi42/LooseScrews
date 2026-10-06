//! Packing helpers for the 8 replicated parameter bytes of a running event
//! ([`crate::director::MoodView::params`]).
//!
//! Effect hooks must be pure functions of the replicated view, so a value an
//! event decides at its announce (an axis, the level's up, a position) is
//! packed into the parameter bytes and every user (the server's world and
//! the client predictor) decodes the same bytes with IEEE-exact operations:
//! the decoded value, not the original, is what both sides use.

use ls_math::{det, Vec3};

/// Packs a direction into 3 bytes (signed, 1/127 steps).
pub fn pack_dir(v: Vec3) -> [u8; 3] {
    let n = v.try_normalize().unwrap_or(Vec3::Y);
    let q = |c: f32| (det::round(c.clamp(-1.0, 1.0) * 127.0) as i8) as u8;
    [q(n.x), q(n.y), q(n.z)]
}

/// Unpacks [`pack_dir`]: a unit vector (+Y for an all-zero triple). Exact
/// arithmetic only (division, `sqrt`), so both ends get the same bits.
pub fn unpack_dir(b: [u8; 3]) -> Vec3 {
    let f = |c: u8| (c as i8) as f32 / 127.0;
    Vec3::new(f(b[0]), f(b[1]), f(b[2]))
        .try_normalize()
        .unwrap_or(Vec3::Y)
}

/// Steps per unit of [`pack_pos`] (1/64 u; covers +-512 u).
const POS_STEPS: f32 = 64.0;

/// Packs a position into 6 bytes (three little-endian `i16` in 1/64 u).
pub fn pack_pos(p: Vec3) -> [u8; 6] {
    let q = |c: f32| {
        let c = if c.is_finite() { c } else { 0.0 };
        (det::round(c * POS_STEPS).clamp(i16::MIN as f32, i16::MAX as f32) as i16).to_le_bytes()
    };
    let (x, y, z) = (q(p.x), q(p.y), q(p.z));
    [x[0], x[1], y[0], y[1], z[0], z[1]]
}

/// Unpacks [`pack_pos`].
pub fn unpack_pos(b: [u8; 6]) -> Vec3 {
    let f = |lo: u8, hi: u8| i16::from_le_bytes([lo, hi]) as f32 / POS_STEPS;
    Vec3::new(f(b[0], b[1]), f(b[2], b[3]), f(b[4], b[5]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn directions_round_trip_closely_and_axes_exactly() {
        for v in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::Y, -Vec3::X] {
            assert_eq!(unpack_dir(pack_dir(v)), v);
        }
        let v = Vec3::new(0.3, -0.8, 0.52).normalize();
        let u = unpack_dir(pack_dir(v));
        assert!((u.length() - 1.0).abs() < 1e-6);
        assert!(u.dot(v) > 0.9999, "{u:?}");
        assert_eq!(unpack_dir([0, 0, 0]), Vec3::Y);
        assert_eq!(unpack_dir(pack_dir(Vec3::ZERO)), Vec3::Y);
    }

    #[test]
    fn positions_round_trip_to_a_64th() {
        for p in [
            Vec3::ZERO,
            Vec3::new(160.0, -63.5, 319.98),
            Vec3::new(-500.0, 12.34, 0.01),
        ] {
            let q = unpack_pos(pack_pos(p));
            assert!((q - p).length() < 0.01, "{p:?} -> {q:?}");
        }
        // Out of range clamps, NaN packs as 0.
        assert_eq!(unpack_pos(pack_pos(Vec3::new(1e6, 0.0, 0.0))).x, 511.984_38);
        assert_eq!(unpack_pos(pack_pos(Vec3::new(f32::NAN, 1.0, 0.0))).x, 0.0);
    }
}
