//! Deterministic state hashing (FNV-1a, 64 bit) for replay and desync checks.

use ls_math::{Quat, Vec3};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Incremental FNV-1a hasher over little-endian encodings. Floats are hashed
/// by their exact bit patterns, so any divergence shows up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateHasher(u64);

impl Default for StateHasher {
    fn default() -> Self {
        StateHasher(FNV_OFFSET)
    }
}

impl StateHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }

    pub fn write_bool(&mut self, v: bool) {
        self.write_u8(v as u8);
    }

    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    pub fn write_f32(&mut self, v: f32) {
        self.write_u32(v.to_bits());
    }

    pub fn write_vec3(&mut self, v: Vec3) {
        for b in v.to_bits() {
            self.write_u32(b);
        }
    }

    pub fn write_quat(&mut self, q: Quat) {
        for b in q.to_bits() {
            self.write_u32(b);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn fnv_reference_vectors() {
        // Published FNV-1a 64 test vectors.
        assert_eq!(StateHasher::new().finish(), 0xcbf29ce484222325);
        let mut h = StateHasher::new();
        h.write_bytes(b"a");
        assert_eq!(h.finish(), 0xaf63dc4c8601ec8c);
        let mut h = StateHasher::new();
        h.write_bytes(b"foobar");
        assert_eq!(h.finish(), 0x85944171f73967e8);
    }

    #[test]
    fn distinguishes_signed_zero() {
        let mut a = StateHasher::new();
        a.write_f32(0.0);
        let mut b = StateHasher::new();
        b.write_f32(-0.0);
        assert_ne!(a.finish(), b.finish());
    }
}
