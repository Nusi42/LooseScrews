//! Quantization of positions, velocities and orientations.
//!
//! | Quantity | Encoding | Resolution (default config) |
//! | --- | --- | --- |
//! | Position | 3 x 16 bit, linear inside the level bounds | 260 u / 65535 = 0.0040 u |
//! | Velocity | 3 x 16 bit, signed linear in `±max_speed` (zero exact) | 128 u/s / 65534 = 0.002 u/s |
//! | Orientation | smallest-three, 2 bit index + 3 x 10 bit = 32 bit | about 0.25 degrees worst case |
//!
//! Position and velocity quantizers are idempotent (`pack(unpack(q)) == q`).
//! The quaternion quantizer is idempotent except when the two largest
//! components nearly tie; then re-packing may pick the other one as the
//! dropped component, which encodes the same rotation with different bits.
//! The snapshot codec therefore keeps packed integers for its baselines and
//! never re-quantizes received values.
//!
//! This module is not part of the deterministic simulation, but it uses only
//! IEEE-exact operations plus `ls_math::det`, so native and WASM produce the
//! same bits anyway.

use ls_math::{det, Aabb, Quat, Vec3};

use crate::error::EncodeError;

/// Positions packed as three 16-bit fractions of the level bounds.
pub type PackedVec3 = [u16; 3];

const POS_MAX: f32 = 65535.0;
/// Entity positions are this many bits coarser than ship positions
/// ([`Quantizer::pack_entity_position`]).
pub const ENTITY_POS_SHIFT: u32 = 3;
/// Largest packed entity position component.
pub const ENTITY_POS_MAX: u16 = (65535u32 >> ENTITY_POS_SHIFT) as u16;
const VEL_MAX: f32 = 32767.0;
/// Largest value of a 10-bit smallest-three component; 511 is zero.
const ROT_MAX: f32 = 511.0;
const SQRT_2: f32 = core::f32::consts::SQRT_2;
const INV_SQRT_2: f32 = core::f32::consts::FRAC_1_SQRT_2;

/// Quantization ranges shared by server and clients. The server sends its
/// config in the `Welcome` message so both sides always agree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuantConfig {
    /// Positions are clamped to this box. Use the level bounds (plus a small
    /// margin if ships can leave them).
    pub bounds: Aabb,
    /// Ship velocity components are clamped to `±max_ship_speed` u/s.
    pub max_ship_speed: f32,
    /// Projectile velocity components are clamped to `±max_projectile_speed` u/s.
    pub max_projectile_speed: f32,
}

impl Default for QuantConfig {
    /// The ls-worldgen level box `[0,256] x [0,128] x [0,256]` u (min corner at
    /// the origin, see `Generator::bounds`) plus a 2 u margin on every side,
    /// ships up to 64 u/s per axis (cruise 12, boost 16.5, plus knockback
    /// headroom), projectiles up to 320 u/s (Railspike 270 u/s). The server
    /// should still fill `Welcome.quant.bounds` from the real level bounds.
    fn default() -> Self {
        QuantConfig {
            bounds: Aabb::new(Vec3::splat(-2.0), Vec3::new(258.0, 130.0, 258.0)),
            max_ship_speed: 64.0,
            max_projectile_speed: 320.0,
        }
    }
}

impl QuantConfig {
    /// Checks that the ranges are finite, non-empty and representable.
    pub fn validate(&self) -> Result<(), &'static str> {
        let b = self.bounds;
        if !(b.min.is_finite() && b.max.is_finite()) {
            return Err("bounds must be finite");
        }
        if !(b.max.x > b.min.x && b.max.y > b.min.y && b.max.z > b.min.z) {
            return Err("bounds must have positive size on every axis");
        }
        if !(b.max - b.min).is_finite() {
            return Err("bounds too large");
        }
        for s in [self.max_ship_speed, self.max_projectile_speed] {
            if !(s.is_finite() && s > 0.0) {
                return Err("speed limits must be positive and finite");
            }
        }
        Ok(())
    }
}

/// A validated [`QuantConfig`] with precomputed scale factors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantizer {
    config: QuantConfig,
    pos_scale: Vec3,
    pos_step: Vec3,
}

impl Default for Quantizer {
    fn default() -> Self {
        Quantizer::new(QuantConfig::default()).expect("default config is valid")
    }
}

impl Quantizer {
    pub fn new(config: QuantConfig) -> Result<Self, EncodeError> {
        config.validate().map_err(EncodeError::OutOfRange)?;
        let size = config.bounds.max - config.bounds.min;
        Ok(Quantizer {
            config,
            pos_scale: Vec3::new(POS_MAX / size.x, POS_MAX / size.y, POS_MAX / size.z),
            pos_step: Vec3::new(size.x / POS_MAX, size.y / POS_MAX, size.z / POS_MAX),
        })
    }

    #[inline]
    pub fn config(&self) -> &QuantConfig {
        &self.config
    }

    /// Size of one position step per axis; the worst-case error is half of it.
    #[inline]
    pub fn position_step(&self) -> Vec3 {
        self.pos_step
    }

    /// Size of one ship velocity step; the worst-case error is half of it.
    #[inline]
    pub fn ship_velocity_step(&self) -> f32 {
        self.config.max_ship_speed / VEL_MAX
    }

    /// Size of one projectile velocity step.
    #[inline]
    pub fn projectile_velocity_step(&self) -> f32 {
        self.config.max_projectile_speed / VEL_MAX
    }

    /// Packs a position, clamping it into the bounds. NaN maps to the minimum corner.
    pub fn pack_position(&self, p: Vec3) -> PackedVec3 {
        let min = self.config.bounds.min;
        [
            pack_unit(p.x - min.x, self.pos_scale.x),
            pack_unit(p.y - min.y, self.pos_scale.y),
            pack_unit(p.z - min.z, self.pos_scale.z),
        ]
    }

    pub fn unpack_position(&self, q: PackedVec3) -> Vec3 {
        let min = self.config.bounds.min;
        Vec3::new(
            min.x + q[0] as f32 * self.pos_step.x,
            min.y + q[1] as f32 * self.pos_step.y,
            min.z + q[2] as f32 * self.pos_step.z,
        )
    }

    /// Packs an entity position: [`ENTITY_POS_SHIFT`] bits coarser than a
    /// ship position (steps of 8 position steps, about 0.03 u across a 256 u
    /// level), so a drifting debris piece moves only a few steps between
    /// snapshots and its diffs stay small.
    pub fn pack_entity_position(&self, p: Vec3) -> PackedVec3 {
        let min = self.config.bounds.min;
        let f = (1u32 << ENTITY_POS_SHIFT) as f32;
        let max = ENTITY_POS_MAX as f32;
        let c = |d: f32, scale: f32| {
            let v = det::round(d * (scale / f));
            if v.is_nan() {
                0
            } else {
                v.clamp(0.0, max) as u16
            }
        };
        [
            c(p.x - min.x, self.pos_scale.x),
            c(p.y - min.y, self.pos_scale.y),
            c(p.z - min.z, self.pos_scale.z),
        ]
    }

    pub fn unpack_entity_position(&self, q: PackedVec3) -> Vec3 {
        let min = self.config.bounds.min;
        let f = (1u32 << ENTITY_POS_SHIFT) as f32;
        Vec3::new(
            min.x + q[0].min(ENTITY_POS_MAX) as f32 * (self.pos_step.x * f),
            min.y + q[1].min(ENTITY_POS_MAX) as f32 * (self.pos_step.y * f),
            min.z + q[2].min(ENTITY_POS_MAX) as f32 * (self.pos_step.z * f),
        )
    }

    /// Size of one entity position step per axis.
    pub fn entity_position_step(&self) -> Vec3 {
        self.pos_step * (1u32 << ENTITY_POS_SHIFT) as f32
    }

    pub fn pack_ship_velocity(&self, v: Vec3) -> PackedVec3 {
        pack_signed_vec(v, self.config.max_ship_speed)
    }

    pub fn unpack_ship_velocity(&self, q: PackedVec3) -> Vec3 {
        unpack_signed_vec(q, self.config.max_ship_speed)
    }

    pub fn pack_projectile_velocity(&self, v: Vec3) -> PackedVec3 {
        pack_signed_vec(v, self.config.max_projectile_speed)
    }

    pub fn unpack_projectile_velocity(&self, q: PackedVec3) -> Vec3 {
        unpack_signed_vec(q, self.config.max_projectile_speed)
    }
}

/// `round(d * scale)` clamped to `0..=65535`.
#[inline]
fn pack_unit(d: f32, scale: f32) -> u16 {
    let v = det::round(d * scale);
    if v.is_nan() {
        0
    } else {
        v.clamp(0.0, POS_MAX) as u16
    }
}

/// Signed component in `±max` as a two's-complement `i16` in `-32767..=32767`
/// stored in a `u16` (so zero is exact and the range is symmetric).
#[inline]
fn pack_signed(v: f32, max: f32) -> u16 {
    let q = det::round(v / max * VEL_MAX);
    let q = if q.is_nan() {
        0.0
    } else {
        q.clamp(-VEL_MAX, VEL_MAX)
    };
    (q as i16) as u16
}

#[inline]
fn unpack_signed(q: u16, max: f32) -> f32 {
    // -32768 is never produced; clamp it for hostile input.
    let i = (q as i16).max(-32767);
    i as f32 * (max / VEL_MAX)
}

fn pack_signed_vec(v: Vec3, max: f32) -> PackedVec3 {
    [
        pack_signed(v.x, max),
        pack_signed(v.y, max),
        pack_signed(v.z, max),
    ]
}

fn unpack_signed_vec(q: PackedVec3, max: f32) -> Vec3 {
    Vec3::new(
        unpack_signed(q[0], max),
        unpack_signed(q[1], max),
        unpack_signed(q[2], max),
    )
}

/// Smallest-three quaternion compression into 32 bits.
///
/// Layout: bits 30-31 index of the dropped (largest magnitude) component
/// in `x, y, z, w` order, then the other three components in that order at
/// bits 20-29, 10-19 and 0-9. Each is `round(c * sqrt(2) * 511) + 511`, since
/// the three smaller components of a unit quaternion lie in `±1/sqrt(2)`.
/// The sign is fixed by making the dropped component positive (`q` and `-q`
/// are the same rotation). Non-finite or zero input packs as the identity.
pub fn pack_quat(q: Quat) -> u32 {
    let len2 = q.length_squared();
    let q = if len2.is_finite() && len2 > 0.0 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    let c = q.to_array();
    let mut largest = 0;
    for i in 1..4 {
        if c[i].abs() > c[largest].abs() {
            largest = i;
        }
    }
    let sign = if c[largest] < 0.0 { -1.0 } else { 1.0 };
    let mut out = (largest as u32) << 30;
    let mut shift = 20;
    for (i, &comp) in c.iter().enumerate() {
        if i == largest {
            continue;
        }
        let v = det::round(comp * sign * SQRT_2 * ROT_MAX).clamp(-ROT_MAX, ROT_MAX);
        let packed = (v as i32 + 511) as u32;
        out |= packed << shift;
        shift -= 10;
    }
    out
}

/// Inverse of [`pack_quat`]; always returns a unit quaternion.
pub fn unpack_quat(bits: u32) -> Quat {
    let largest = (bits >> 30) as usize;
    let mut c = [0.0f32; 4];
    let mut shift = 20;
    let mut sum = 0.0f32;
    for (i, slot) in c.iter_mut().enumerate() {
        if i == largest {
            continue;
        }
        // 1023 is never produced; clamp it for hostile input.
        let raw = ((bits >> shift) & 0x3FF).min(1022) as i32 - 511;
        let v = raw as f32 / ROT_MAX * INV_SQRT_2;
        *slot = v;
        sum += v * v;
        shift -= 10;
    }
    c[largest] = det::sqrt((1.0 - sum).max(0.0));
    Quat::from_xyzw(c[0], c[1], c[2], c[3]).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::Pcg32;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn random_quat(rng: &mut Pcg32) -> Quat {
        loop {
            let q = Quat::from_xyzw(
                rng.range_f32(-1.0, 1.0),
                rng.range_f32(-1.0, 1.0),
                rng.range_f32(-1.0, 1.0),
                rng.range_f32(-1.0, 1.0),
            );
            let l = q.length_squared();
            if l > 0.01 && l <= 1.0 {
                return q.normalize();
            }
        }
    }

    #[test]
    fn position_error_is_at_most_half_a_step() {
        let quant = Quantizer::default();
        let b = quant.config().bounds;
        let half = quant.position_step() * 0.5;
        let mut rng = Pcg32::new(7, 3);
        for _ in 0..20_000 {
            let p = Vec3::new(
                rng.range_f32(b.min.x, b.max.x),
                rng.range_f32(b.min.y, b.max.y),
                rng.range_f32(b.min.z, b.max.z),
            );
            let q = quant.pack_position(p);
            let back = quant.unpack_position(q);
            let err = (back - p).abs();
            // Allow float rounding of a few ulps (at |p| up to 258 u one ulp is
            // about 3e-5) on top of half a step.
            assert!(err.x <= half.x * 1.001 + 1e-4, "{p:?} -> {back:?}");
            assert!(err.y <= half.y * 1.001 + 1e-4, "{p:?} -> {back:?}");
            assert!(err.z <= half.z * 1.001 + 1e-4, "{p:?} -> {back:?}");
            assert_eq!(quant.pack_position(back), q, "idempotent");
        }
        // Default resolution is better than 1/250 u.
        assert!(quant.position_step().max_element() < 0.004);
    }

    #[test]
    fn position_clamps_and_hits_corners() {
        let quant = Quantizer::default();
        let b = quant.config().bounds;
        assert_eq!(quant.pack_position(b.min), [0, 0, 0]);
        assert_eq!(quant.pack_position(b.max), [65535; 3]);
        assert_eq!(quant.unpack_position([0; 3]), b.min);
        assert_eq!(quant.pack_position(b.max + Vec3::splat(1e6)), [65535; 3]);
        assert_eq!(quant.pack_position(b.min - Vec3::splat(1e6)), [0; 3]);
        assert_eq!(
            quant.pack_position(Vec3::new(f32::NAN, f32::INFINITY, f32::NEG_INFINITY)),
            [0, 65535, 0]
        );
    }

    #[test]
    fn velocity_error_bounds_and_exact_zero() {
        let quant = Quantizer::default();
        let max = quant.config().max_ship_speed;
        let half = quant.ship_velocity_step() * 0.5;
        let mut rng = Pcg32::new(9, 1);
        for _ in 0..20_000 {
            let v = Vec3::new(
                rng.range_f32(-max, max),
                rng.range_f32(-max, max),
                rng.range_f32(-max, max),
            );
            let q = quant.pack_ship_velocity(v);
            let back = quant.unpack_ship_velocity(q);
            assert!((back - v).abs().max_element() <= half * 1.001 + 1e-6);
            assert_eq!(quant.pack_ship_velocity(back), q, "idempotent");
        }
        assert_eq!(
            quant.unpack_ship_velocity(quant.pack_ship_velocity(Vec3::ZERO)),
            Vec3::ZERO
        );
        let clamped =
            quant.unpack_ship_velocity(quant.pack_ship_velocity(Vec3::new(1e9, -1e9, 0.0)));
        assert_eq!(clamped, Vec3::new(max, -max, 0.0));
        // Projectile range covers the Rivet Gun at 180 u/s.
        let rivet = Vec3::new(0.0, 0.0, -180.0);
        let back = quant.unpack_projectile_velocity(quant.pack_projectile_velocity(rivet));
        assert!((back - rivet).length() <= quant.projectile_velocity_step());
    }

    #[test]
    fn quaternion_error_below_0_3_degrees() {
        let mut rng = Pcg32::new(11, 5);
        let mut worst = 0.0f32;
        let mut ties = 0;
        for _ in 0..50_000 {
            let q = random_quat(&mut rng);
            let bits = pack_quat(q);
            let back = unpack_quat(bits);
            assert!((back.length() - 1.0).abs() < 1e-5);
            // |dot| ~ 1 makes acos ill-conditioned; measure via the chord.
            let d = q.dot(back);
            let s = if d < 0.0 { -1.0 } else { 1.0 };
            let diff = Quat::from_xyzw(
                q.x - s * back.x,
                q.y - s * back.y,
                q.z - s * back.z,
                q.w - s * back.w,
            );
            let angle = 4.0 * det::asin(diff.length() * 0.5);
            worst = worst.max(angle);
            let again = pack_quat(back);
            if again != bits {
                // Only near ties re-pack differently, and to the same rotation.
                ties += 1;
                assert!(unpack_quat(again).dot(back).abs() > 0.99999, "{q:?}");
            }
        }
        println!(
            "quat: worst {} deg, {ties} re-pack mismatches",
            worst.to_degrees()
        );
        assert!(ties < 250, "{ties} re-pack mismatches");
        assert!(
            worst.to_degrees() < 0.3,
            "worst error {} deg",
            worst.to_degrees()
        );
    }

    #[test]
    fn quaternion_edge_cases() {
        assert_eq!(unpack_quat(pack_quat(Quat::IDENTITY)), Quat::IDENTITY);
        // q and -q pack identically.
        let q = Quat::from_axis_angle(Vec3::new(1.0, 2.0, 3.0), 2.0);
        assert_eq!(pack_quat(q), pack_quat(-q));
        // Degenerate input becomes the identity.
        assert_eq!(
            pack_quat(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)),
            pack_quat(Quat::IDENTITY)
        );
        assert_eq!(
            pack_quat(Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)),
            pack_quat(Quat::IDENTITY)
        );
        // Hostile bits still decode to a unit quaternion.
        for bits in [u32::MAX, 0, 0xFFFF_FFFF >> 2, 0x3FF << 20] {
            let u = unpack_quat(bits);
            assert!((u.length() - 1.0).abs() < 1e-5, "{bits:x}");
        }
    }

    #[test]
    fn config_validation() {
        assert!(QuantConfig::default().validate().is_ok());
        let mut c = QuantConfig::default();
        c.bounds.max.x = c.bounds.min.x;
        assert!(Quantizer::new(c).is_err());
        let c = QuantConfig {
            max_ship_speed: 0.0,
            ..QuantConfig::default()
        };
        assert!(c.validate().is_err());
        let c = QuantConfig {
            bounds: Aabb::new(Vec3::splat(-f32::MAX), Vec3::splat(f32::MAX)),
            ..QuantConfig::default()
        };
        assert!(c.validate().is_err());
    }
}
