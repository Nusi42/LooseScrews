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
/// box plus one step outside it, followed by the Heist site's bits (core,
/// radii, access point, extraction zones; generator version 3), the
/// archetype, the gravity well sites and the belt site (slots, pickup
/// points, boulder orbits; generator version 4).
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
    if let Some(s) = gen.heist() {
        let mut floats = vec![s.knot_radius, s.chamber_radius];
        for v in [s.core, s.access] {
            floats.extend(v.to_array());
        }
        for z in &s.extraction {
            floats.extend(z.pos.to_array());
            floats.push(z.radius);
        }
        for f in floats {
            h.update(f.to_bits().to_le_bytes());
        }
        h.update([s.surface as u8, s.extraction[0].team, s.extraction[1].team]);
    }
    // Generator version 4: the level archetype, the gravity well sites and
    // the belt site.
    h.update([gen.archetype().index()]);
    for w in gen.gravity_wells() {
        let mut floats = w.pos.to_array().to_vec();
        floats.extend([w.core_radius, w.hollow_radius, w.strength, w.range]);
        for f in floats {
            h.update(f.to_bits().to_le_bytes());
        }
        h.update([w.kind as u8]);
    }
    if let Some(b) = gen.belt() {
        let mut floats = b.center.to_array().to_vec();
        floats.extend([
            b.inner_radius,
            b.outer_radius,
            b.half_thickness,
            b.cocoon_radius,
            b.exit_radius,
            b.gate_inner_radius,
            b.gate_outer_radius,
        ]);
        for slot in &b.slots {
            floats.extend(slot.pos.to_array());
            floats.extend(slot.orientation.to_array());
        }
        for p in &b.pickup_points {
            floats.extend(p.to_array());
        }
        for o in &b.boulders {
            floats.extend([o.radius, o.height, o.phase, o.angular_speed, o.size]);
            floats.extend(o.tumble_axis.to_array());
            floats.push(o.tumble_speed);
        }
        for f in floats {
            h.update(f.to_bits().to_le_bytes());
        }
    }
    h.finalize().into()
}

/// Runs the full golden procedure for a share code:
///
/// 1. SDF lattice hash ([`sdf_hash`] with [`SDF_STEP`]).
/// 2. Probe point: from the first open point near the room centre (belt
///    levels: near start slot 0)
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
    // Belt levels probe from start slot 0 (the room centre is the empty
    // cocoon).
    let start = match store.generator().belt() {
        Some(b) => b.slots[0].pos,
        None => store.generator().room().center(),
    };
    let origin = store.find_open_point(start, 1.5, 48).unwrap_or(start);
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
        color: ls_math::terrain::PAINT_PLAIN,
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
