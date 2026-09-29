//! Golden hashes: a fixed procedure that condenses a world into a few SHA-256
//! digests. Native tests compare them against `tests/fixtures/golden_hashes.txt`;
//! the WASM build runs the same procedure under Node so any float drift
//! between targets shows up as a mismatch.

use ls_math::terrain::{TerrainEdit, TerrainQuery};
use ls_math::Vec3;
use sha2::{Digest, Sha256};

use crate::{ChunkStore, Generator, ShareCode, WorldgenError};

/// Lattice step (u) for the SDF hash.
pub const SDF_STEP: f32 = 8.0;

/// The golden digests of one share code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenHashes {
    /// Generator SDF on a lattice over the level.
    pub sdf: [u8; 32],
    /// Surface-nets mesh of the probe chunk.
    pub surface_nets: [u8; 32],
    /// Dual-contouring mesh of the probe chunk.
    pub dual_contouring: [u8; 32],
    /// Surface-nets meshes of all dirty chunks after the fixed edits.
    pub edited: [u8; 32],
}

impl GoldenHashes {
    /// Hex digests in fixture order: `sdf surface_nets dual_contouring edited`.
    pub fn to_hex(&self) -> [String; 4] {
        [
            hex(&self.sdf),
            hex(&self.surface_nets),
            hex(&self.dual_contouring),
            hex(&self.edited),
        ]
    }
}

/// Lowercase hex encoding.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

/// SHA-256 of `Generator::sample` bits over a lattice with spacing `step`
/// (offset 0.37 u so samples avoid exact grid planes), covering the level
/// box plus one step outside it.
pub fn sdf_hash(gen: &Generator, step: f32) -> [u8; 32] {
    let b = gen.bounds();
    let mut h = Sha256::new();
    let mut z = b.min.z - step + 0.37;
    while z <= b.max.z + step {
        let mut y = b.min.y - step + 0.37;
        while y <= b.max.y + step {
            let mut x = b.min.x - step + 0.37;
            while x <= b.max.x + step {
                h.update(gen.sample(Vec3::new(x, y, z)).to_bits().to_le_bytes());
                x += step;
            }
            y += step;
        }
        z += step;
    }
    h.finalize().into()
}

/// Runs the full golden procedure for a share code:
///
/// 1. SDF lattice hash ([`sdf_hash`] with [`SDF_STEP`]).
/// 2. Probe point: from the first open point near the room centre
///    ([`ChunkStore::find_open_point`], clearance 1.5 u), raycast along `(0.3, -1, 0.2)`, then
///    `+X`, `-Z`, `(-1, 0.4, 0.1)` until one hits; the probe chunk contains
///    the hit. Hash its surface-nets and dual-contouring meshes.
/// 3. Apply a 3 u crater at the hit, a 1.5 u rock 4 u out along the normal
///    and a 4 u team-1 paint splat at the hit; hash the surface-nets meshes
///    of every chunk reported dirty (in order).
pub fn golden_hashes(code: &ShareCode) -> Result<GoldenHashes, WorldgenError> {
    let gen = Generator::from_code(code)?;
    let sdf = sdf_hash(&gen, SDF_STEP);
    let mut store = ChunkStore::new(gen);
    let room = store.generator().room();
    let origin = store
        .find_open_point(room.center(), 1.5, 48)
        .unwrap_or(room.center());
    let (hit, normal) = [
        Vec3::new(0.3, -1.0, 0.2),
        Vec3::X,
        Vec3::NEG_Z,
        Vec3::new(-1.0, 0.4, 0.1),
    ]
    .iter()
    .find_map(|&d| {
        store
            .raycast(origin, d, 1000.0)
            .filter(|h| h.distance > 0.0)
    })
    .map(|h| (h.point, h.normal))
    .unwrap_or((origin, Vec3::Y));
    let probe = store.chunk_at(hit).unwrap_or_default();
    let surface_nets = store.mesh_surface_nets(probe).hash();
    let dual_contouring = store.mesh_dual_contouring(probe).hash();

    let mut dirty = store.apply(TerrainEdit::SubtractSphere {
        center: hit,
        radius: 3.0,
    });
    dirty.extend(store.apply(TerrainEdit::AddSphere {
        center: hit + normal * 4.0,
        radius: 1.5,
    }));
    dirty.extend(store.apply(TerrainEdit::Paint {
        center: hit,
        radius: 4.0,
        team: 1,
    }));
    dirty.sort();
    dirty.dedup();
    let mut h = Sha256::new();
    for c in dirty {
        h.update(c.x.to_le_bytes());
        h.update(c.y.to_le_bytes());
        h.update(c.z.to_le_bytes());
        h.update(store.mesh_surface_nets(c).hash());
    }
    Ok(GoldenHashes {
        sdf,
        surface_nets,
        dual_contouring,
        edited: h.finalize().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn hex_encoding() {
        assert_eq!(hex(&[0x00, 0xab, 0x0f, 0xf0]), "00ab0ff0");
    }
}
