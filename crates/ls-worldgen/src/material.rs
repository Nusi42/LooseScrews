//! Terrain materials: a deterministic strength field over the level.
//!
//! Rock comes in three strengths, arranged in irregular regions (a Voronoi
//! partition of space with one hashed site per [`MATERIAL_CELL`] cube; each
//! site picks a material from its hash). Weak rock breaks first: craters
//! carved by the simulation are scaled by the strength of the material at
//! the crater centre ([`strengthen_edit`]), so repeated fire digs through
//! weak rock quickly and hard rock slowly.
//!
//! The field is a pure function of the world seed and the position (integer
//! hashing and IEEE-exact `+ - *` only), so every peer and target agrees. It
//! does not change the generator SDF, so it is not part of the golden
//! hashes; `tests::material_field_hash_is_pinned` pins it instead (native
//! and wasm32).

use ls_math::rng::mix64;
use ls_math::terrain::TerrainEdit;
use ls_math::{Aabb, Vec3};
use serde::{Deserialize, Serialize};

use crate::generator::floor_i32;
use crate::noise::{hash3, unit_f32, unit_f32_b};

/// Edge length (u) of the cubes that each hold one material site.
pub const MATERIAL_CELL: f32 = 24.0;
/// Share of sites that are weak rock.
pub const WEAK_SHARE: f32 = 0.3;
/// Share of sites that are hard rock (the rest is ordinary rock).
pub const HARD_SHARE: f32 = 0.25;
/// Salt mixed into the world seed for the material sites.
const MATERIAL_SALT: u64 = 0x4d41_5445_5249_414c; // "MATERIAL"

/// Rock strength class. The `u8` value is the id used in mesh vertex
/// `materials` and on the client.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum Material {
    /// Crumbly rock: craters are larger.
    Weak = 0,
    /// Ordinary rock: craters have the size the weapon asks for.
    #[default]
    Rock = 1,
    /// Hard rock: craters are smaller.
    Hard = 2,
}

impl Material {
    /// Every material, by id.
    pub const ALL: [Material; 3] = [Material::Weak, Material::Rock, Material::Hard];

    /// Crater radius multiplier (1.0 for ordinary rock).
    pub const fn crater_scale(self) -> f32 {
        ls_math::terrain::material_crater_scale(self as u8)
    }

    /// Material for an id (`None` if unknown).
    pub const fn from_id(id: u8) -> Option<Material> {
        match id {
            0 => Some(Material::Weak),
            1 => Some(Material::Rock),
            2 => Some(Material::Hard),
            _ => None,
        }
    }

    /// The id used in meshes and on the client.
    pub const fn id(self) -> u8 {
        self as u8
    }

    pub const fn name(self) -> &'static str {
        match self {
            Material::Weak => "weak rock",
            Material::Rock => "rock",
            Material::Hard => "hard rock",
        }
    }
}

/// The strength field of one world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialField {
    seed: u64,
}

impl MaterialField {
    /// The field for a world seed.
    pub fn new(world_seed: u64) -> Self {
        MaterialField {
            seed: mix64(world_seed ^ MATERIAL_SALT),
        }
    }

    /// Material at `p` (defined everywhere, including open space, so a
    /// crater centre on the surface has a material).
    pub fn at(&self, p: Vec3) -> Material {
        nearest_site(p, |cc| self.site(cc))
    }

    /// The site of material cube `cc`: position and material.
    fn site(&self, cc: [i32; 3]) -> (Vec3, Material) {
        let h = hash3(self.seed, cc[0], cc[1], cc[2]);
        let pos = Vec3::new(
            (cc[0] as f32 + unit_f32(h)) * MATERIAL_CELL,
            (cc[1] as f32 + unit_f32_b(h)) * MATERIAL_CELL,
            (cc[2] as f32 + unit_f32(mix64(h))) * MATERIAL_CELL,
        );
        let r = (h & 0xffff) as f32 * (1.0 / 65_536.0);
        let m = if r < WEAK_SHARE {
            Material::Weak
        } else if r >= 1.0 - HARD_SHARE {
            Material::Hard
        } else {
            Material::Rock
        };
        (pos, m)
    }

    /// A lookup table of the sites around `region` for many queries there
    /// (meshing); same results as [`MaterialField::at`].
    pub fn region(&self, region: Aabb) -> MaterialRegion {
        let lo = cube_of(region.min).map(|v| v - 1);
        let hi = cube_of(region.max).map(|v| v + 1);
        let dims = [0, 1, 2].map(|a| hi[a] - lo[a] + 1);
        let mut sites = Vec::with_capacity((dims[0] * dims[1] * dims[2]).max(0) as usize);
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    sites.push(self.site([x, y, z]));
                }
            }
        }
        MaterialRegion {
            field: *self,
            lo,
            dims,
            sites,
        }
    }
}

/// Precomputed material sites around a region (see [`MaterialField::region`]).
#[derive(Debug, Clone)]
pub struct MaterialRegion {
    field: MaterialField,
    lo: [i32; 3],
    dims: [i32; 3],
    sites: Vec<(Vec3, Material)>,
}

impl MaterialRegion {
    /// Material at `p`; bit-identical to [`MaterialField::at`] (points
    /// outside the region fall back to hashing).
    pub fn at(&self, p: Vec3) -> Material {
        nearest_site(p, |cc| {
            let l = [0, 1, 2].map(|a| cc[a] - self.lo[a]);
            if (0..3).all(|a| l[a] >= 0 && l[a] < self.dims[a]) {
                self.sites[(l[0] + self.dims[0] * (l[1] + self.dims[1] * l[2])) as usize]
            } else {
                self.field.site(cc)
            }
        })
    }
}

/// Material cube containing `p`.
#[inline]
fn cube_of(p: Vec3) -> [i32; 3] {
    [
        floor_i32(p.x / MATERIAL_CELL),
        floor_i32(p.y / MATERIAL_CELL),
        floor_i32(p.z / MATERIAL_CELL),
    ]
}

/// Material of the site nearest to `p`. One site per cube, anywhere inside
/// it, so the nearest site is always in the 3x3x3 neighbourhood. Ties keep
/// the first cube in this order.
#[inline]
fn nearest_site(p: Vec3, site: impl Fn([i32; 3]) -> (Vec3, Material)) -> Material {
    let c = cube_of(p);
    let mut best = f32::INFINITY;
    let mut best_m = Material::Rock;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (pos, m) = site([c[0] + dx, c[1] + dy, c[2] + dz]);
                let d = p.distance_squared(pos);
                if d < best {
                    best = d;
                    best_m = m;
                }
            }
        }
    }
    best_m
}

/// Applies material strength to an edit the simulation produced: a crater's
/// radius is scaled by [`Material::crater_scale`] of the material at its
/// centre. Every other edit (and an invalid crater) is returned unchanged.
///
/// Apply this once, where edits enter the log (the server's simulation);
/// peers that replay the log apply the scaled edit verbatim.
pub fn strengthen_edit(field: &MaterialField, edit: TerrainEdit) -> TerrainEdit {
    match edit {
        TerrainEdit::SubtractSphere { center, radius }
            if center.is_finite() && radius.is_finite() && radius > 0.0 =>
        {
            TerrainEdit::SubtractSphere {
                center,
                radius: radius * field.at(center).crater_scale(),
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn sample_lattice(f: &MaterialField) -> Vec<Material> {
        let mut out = Vec::new();
        let mut z = 0.37f32;
        while z < 256.0 {
            let mut y = 0.37f32;
            while y < 128.0 {
                let mut x = 0.37f32;
                while x < 256.0 {
                    out.push(f.at(Vec3::new(x, y, z)));
                    x += 4.0;
                }
                y += 4.0;
            }
            z += 4.0;
        }
        out
    }

    #[test]
    fn every_material_occurs_in_regions() {
        let f = MaterialField::new(42);
        let m = sample_lattice(&f);
        let count = |k: Material| m.iter().filter(|&&v| v == k).count() as f32 / m.len() as f32;
        for k in Material::ALL {
            let share = count(k);
            assert!(share > 0.1 && share < 0.7, "{k:?} {share}");
        }
        // Regions, not noise: most lattice neighbours along x agree.
        let same = m.windows(2).filter(|w| w[0] == w[1]).count() as f32 / m.len() as f32;
        assert!(same > 0.75, "{same}");
        // Other seeds give other fields.
        assert_ne!(sample_lattice(&MaterialField::new(43)), m);
        assert_eq!(sample_lattice(&MaterialField::new(42)), m);
    }

    /// The field is pinned: this hash must be identical on native and wasm32.
    /// Changing the field is a gameplay change (craters differ): update the
    /// constant deliberately.
    #[test]
    fn material_field_hash_is_pinned() {
        let mut h = Sha256::new();
        for seed in [1u64, 42, 0xDEAD_BEEF, u64::MAX] {
            for m in sample_lattice(&MaterialField::new(seed)) {
                h.update([m.id()]);
            }
        }
        let got = crate::golden::hex(&h.finalize());
        assert_eq!(got, MATERIAL_FIELD_HASH, "material field changed");
    }

    const MATERIAL_FIELD_HASH: &str =
        "1f25871dad4e008b23051063fd1a211e41824abe1965473b3eca64f26a21e5f9";

    #[test]
    fn region_lookup_matches_field() {
        let f = MaterialField::new(0xC0FFEE);
        let region = Aabb::new(Vec3::new(31.0, -1.0, 63.0), Vec3::new(64.0, 32.0, 96.0));
        let r = f.region(region);
        for i in 0..3000 {
            // Inside, on cube planes, and outside the region (fallback).
            let p = Vec3::new(
                20.0 + (i * 7 % 60) as f32 * 0.9,
                -10.0 + (i * 13 % 60) as f32 * 0.8,
                50.0 + (i * 29 % 70) as f32 * 0.75,
            );
            assert_eq!(r.at(p), f.at(p), "{p:?}");
        }
        assert_eq!(r.at(Vec3::splat(24.0)), f.at(Vec3::splat(24.0)));
    }

    #[test]
    fn strengthen_scales_craters_only() {
        let f = MaterialField::new(7);
        let mut seen = [false; 3];
        for i in 0..400 {
            let c = Vec3::new(
                i as f32 * 3.1,
                40.0 + (i % 7) as f32,
                200.0 - i as f32 * 0.4,
            );
            let m = f.at(c);
            seen[m.id() as usize] = true;
            let e = strengthen_edit(
                &f,
                TerrainEdit::SubtractSphere {
                    center: c,
                    radius: 2.0,
                },
            );
            assert_eq!(e.radius(), 2.0 * m.crater_scale());
            assert_eq!(e.center(), c);
            let add = TerrainEdit::AddSphere {
                center: c,
                radius: 2.0,
            };
            assert_eq!(strengthen_edit(&f, add), add);
        }
        assert_eq!(seen, [true; 3]);
        let bad = TerrainEdit::SubtractSphere {
            center: Vec3::new(f32::NAN, 0.0, 0.0),
            radius: 2.0,
        };
        assert!(
            matches!(strengthen_edit(&f, bad), TerrainEdit::SubtractSphere { radius, .. } if radius == 2.0)
        );
        assert!(Material::Weak.crater_scale() > 1.0 && Material::Hard.crater_scale() < 1.0);
        for k in Material::ALL {
            assert_eq!(Material::from_id(k.id()), Some(k));
        }
        assert_eq!(Material::from_id(3), None);
        // The ids are the contract constants the simulation uses.
        use ls_math::terrain::{MATERIAL_HARD, MATERIAL_ROCK, MATERIAL_WEAK};
        assert_eq!(
            [
                Material::Weak.id(),
                Material::Rock.id(),
                Material::Hard.id()
            ],
            [MATERIAL_WEAK, MATERIAL_ROCK, MATERIAL_HARD]
        );
    }
}
