//! The seeded SDF generator.
//!
//! [`Generator::sample`] returns the signed distance (negative in rock) of the
//! freshly generated level, before any terrain edits. It is built only from
//! 1-Lipschitz pieces so the result is a valid distance bound:
//!
//! - **Open sky** (Enclosure 0): open space with sparse floating islands, each
//!   a union of spheres with a flat top.
//! - **Caverns** (Enclosure 50): large chambers plus winding tunnels carved out
//!   of rock by gradient noise (scaled so the noise gradient stays below 1).
//! - **Mine shafts** (Enclosure 100): a jittered lattice of straight, square
//!   tunnels and occasional rooms (exact box distances, sharp edges).
//!
//! The three "openness" fields are blended linearly by Enclosure (constant
//! weights keep the Lipschitz bound). Density then adds rock: floor-to-ceiling
//! pillars with knobs and faceted floating rocks (union via `min`). Finally the
//! level is sealed by walls [`WALL_THICKNESS`] thick inside the level box.
//!
//! Feature lookups use jittered grids; each feature stays inside its grid
//! cell, so only the neighbouring cells need to be visited, and fields are
//! capped at a value that features further away can never beat. That keeps
//! the fields continuous (and 1-Lipschitz) while each sample stays cheap.

use ls_math::rng::{mix64, SplitMix64};
use ls_math::{det, Aabb, Vec3};

use crate::noise::{gradient_noise, hash2, hash3, unit_f32, unit_f32_b, NOISE_LIPSCHITZ};
use crate::{ShareCode, Sliders, WorldgenError, CHUNK_CELLS, GENERATOR_VERSION};

/// Thickness of the sealing wall inside the level box (u).
pub const WALL_THICKNESS: f32 = 2.0;
/// Default level size in chunks (256 x 128 x 256 u).
pub const DEFAULT_SIZE_CHUNKS: [i32; 3] = [8, 4, 8];

// --- Open sky: floating islands -------------------------------------------
const ISLAND_CELL: f32 = 48.0;
const ISLAND_CHANCE: f32 = 0.35;
/// Islands never reach further than this from their cell centre (< cell/2).
const ISLAND_JITTER: f32 = 9.6;
const SKY_CAP: f32 = 20.0;

// --- Caverns ----------------------------------------------------------------
/// Chamber noise frequency (per axis) and scale; scale * max_freq * NOISE_LIPSCHITZ <= 1.
const CHAMBER_FREQ: [f32; 3] = [1.0 / 40.0, 1.0 / 28.0, 1.0 / 40.0];
const CHAMBER_SCALE: f32 = 28.0 / NOISE_LIPSCHITZ;
const CHAMBER_THRESHOLD: f32 = 0.28;
const TUNNEL_FREQ: f32 = 1.0 / 24.0;
const TUNNEL_SCALE: f32 = 24.0 / NOISE_LIPSCHITZ;
const TUNNEL_WIDTH: f32 = 0.11;

// --- Mine shafts --------------------------------------------------------------
const MINE_SPACING: f32 = 16.0;
const MINE_HALF_WIDTH: f32 = 2.5;
const MINE_EDGE_CHANCE_H: f32 = 0.55;
const MINE_EDGE_CHANCE_V: f32 = 0.22;
const MINE_ROOM_CHANCE: f32 = 0.12;
const MINE_ROOM_MAX_HALF: f32 = 6.0;
/// Any lattice feature not visited is at least this far away.
const MINE_CAP: f32 = MINE_SPACING - MINE_ROOM_MAX_HALF;

// --- Density: pillars and floating rocks --------------------------------------
const PILLAR_CELL: f32 = 32.0;
const PILLAR_CHANCE: f32 = 0.8;
const PILLAR_JITTER: f32 = 10.0;
const PILLAR_CAP: f32 = 24.0;
const ROCK_CELL: f32 = 20.0;
const ROCK_CHANCE: f32 = 0.45;
const ROCK_JITTER: f32 = 5.0;
const ROCK_CAP: f32 = 16.0;

/// Per-feature seeds, derived from the share-code seed with SplitMix64 so
/// each feature has its own independent hash stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seeds {
    island: u64,
    chamber: u64,
    tunnel_a: u64,
    tunnel_b: u64,
    mine: u64,
    pillar: u64,
    rock: u64,
}

impl Seeds {
    fn derive(seed: u64) -> Self {
        let mut sm = SplitMix64::new(seed ^ 0x4c53_574f_524c_4447); // "LSWORLDG"
        Seeds {
            island: sm.next_u64(),
            chamber: sm.next_u64(),
            tunnel_a: sm.next_u64(),
            tunnel_b: sm.next_u64(),
            mine: sm.next_u64(),
            pillar: sm.next_u64(),
            rock: sm.next_u64(),
        }
    }
}

/// Deterministic level generator for one share code.
#[derive(Debug, Clone, PartialEq)]
pub struct Generator {
    code: ShareCode,
    size_chunks: [i32; 3],
    bounds: Aabb,
    room: Aabb,
    seeds: Seeds,
    w_sky: f32,
    w_cave: f32,
    w_mine: f32,
    density: f32,
    mine_offset: Vec3,
}

impl Generator {
    /// A generator with the default level size.
    pub fn new(seed: u64, sliders: Sliders) -> Self {
        Self::with_size(seed, sliders, DEFAULT_SIZE_CHUNKS)
    }

    /// A generator for a decoded share code. Fails if the code was made by a
    /// different generator version or has invalid sliders.
    pub fn from_code(code: &ShareCode) -> Result<Self, WorldgenError> {
        if code.version != GENERATOR_VERSION {
            return Err(WorldgenError::UnsupportedVersion(code.version));
        }
        code.sliders.validate()?;
        Ok(Self::new(code.seed, code.sliders))
    }

    /// A generator with a custom level size in chunks (each at least 1).
    pub fn with_size(seed: u64, sliders: Sliders, size_chunks: [i32; 3]) -> Self {
        let size_chunks = size_chunks.map(|c| c.max(1));
        let size = Vec3::new(
            (size_chunks[0] * CHUNK_CELLS) as f32,
            (size_chunks[1] * CHUNK_CELLS) as f32,
            (size_chunks[2] * CHUNK_CELLS) as f32,
        );
        let bounds = Aabb::new(Vec3::ZERO, size);
        let room = Aabb::new(
            Vec3::splat(WALL_THICKNESS),
            size - Vec3::splat(WALL_THICKNESS),
        );
        let seeds = Seeds::derive(seed);
        let e = sliders.enclosure.min(100) as f32 / 100.0;
        let w_sky = (1.0 - 2.0 * e).max(0.0);
        let w_mine = (2.0 * e - 1.0).max(0.0);
        let w_cave = 1.0 - w_sky - w_mine;
        let off = mix64(seeds.mine);
        let mine_offset = Vec3::new(
            (off & 15) as f32,
            ((off >> 8) & 15) as f32,
            ((off >> 16) & 15) as f32,
        );
        Generator {
            code: ShareCode::new(seed, sliders),
            size_chunks,
            bounds,
            room,
            seeds,
            w_sky,
            w_cave,
            w_mine,
            density: sliders.density.min(100) as f32 / 100.0,
            mine_offset,
        }
    }

    /// The share code (seed, sliders, current version) this generator uses.
    pub fn code(&self) -> ShareCode {
        self.code
    }

    /// Level size in chunks.
    pub fn size_chunks(&self) -> [i32; 3] {
        self.size_chunks
    }

    /// The level box; everything outside counts as solid.
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// The open interior of the level box (inside the sealing walls).
    pub fn room(&self) -> Aabb {
        self.room
    }

    /// Signed distance of the generated level at `p` (negative in rock).
    /// 1-Lipschitz, defined everywhere (solid outside the level box).
    pub fn sample(&self, p: Vec3) -> f32 {
        // `+ 0.0` turns a -0.0 into +0.0 so ties never depend on evaluation order.
        self.terrain(p).min(self.room_open(p)) + 0.0
    }

    /// Distance to the sealing walls, positive inside the room.
    #[inline]
    pub fn room_open(&self, p: Vec3) -> f32 {
        -self.room.sdf(p)
    }

    /// The terrain without the sealing walls (negative in rock).
    pub fn terrain(&self, p: Vec3) -> f32 {
        let mut open = 0.0f32;
        if self.w_sky > 0.0 {
            open += self.w_sky * self.sky(p);
        }
        if self.w_cave > 0.0 {
            open += self.w_cave * self.caverns(p);
        }
        if self.w_mine > 0.0 {
            open += self.w_mine * self.mines(p);
        }
        if self.density > 0.0 {
            open = open.min(self.pillars(p)).min(self.rocks(p));
        }
        open
    }

    /// Open sky: distance to the nearest floating island, capped at [`SKY_CAP`].
    fn sky(&self, p: Vec3) -> f32 {
        let c = cell_of(p, ISLAND_CELL);
        let mut best = SKY_CAP;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (cx, cy, cz) = (c[0] + dx, c[1] + dy, c[2] + dz);
                    let h = hash3(self.seeds.island, cx, cy, cz);
                    if unit_f32(h) >= ISLAND_CHANCE {
                        continue;
                    }
                    let center = jittered_center([cx, cy, cz], ISLAND_CELL, ISLAND_JITTER, h);
                    // Main radius 5..10; satellites reach 1.2 r (<= jitter margin).
                    let r = 5.0 + 5.0 * unit_f32_b(h);
                    if p.distance(center) - 1.2 * r >= best {
                        continue;
                    }
                    best = best.min(island_sdf(p, center, r, mix64(h)));
                }
            }
        }
        best
    }

    /// Caverns: chambers and noise tunnels, positive where open.
    fn caverns(&self, p: Vec3) -> f32 {
        let q = Vec3::new(
            p.x * CHAMBER_FREQ[0],
            p.y * CHAMBER_FREQ[1],
            p.z * CHAMBER_FREQ[2],
        );
        let chamber = CHAMBER_SCALE * (gradient_noise(self.seeds.chamber, q) - CHAMBER_THRESHOLD);
        let t = p * TUNNEL_FREQ;
        let a = gradient_noise(self.seeds.tunnel_a, t).abs();
        let b = gradient_noise(self.seeds.tunnel_b, t).abs();
        let tunnel = TUNNEL_SCALE * (TUNNEL_WIDTH - a.max(b));
        chamber.max(tunnel)
    }

    /// Mine shafts: union of lattice tunnels and rooms, floored at `-MINE_CAP`.
    fn mines(&self, p: Vec3) -> f32 {
        let s = MINE_SPACING;
        let local = p - self.mine_offset;
        let g = cell_of(local, s);
        let node = |i: [i32; 3]| {
            Vec3::new(i[0] as f32 * s, i[1] as f32 * s, i[2] as f32 * s) + self.mine_offset
        };
        let mut best = -MINE_CAP;
        // Tunnels: all edges whose lower node could be within reach (36 edges).
        for axis in 0..3usize {
            let chance = if axis == 1 {
                MINE_EDGE_CHANCE_V
            } else {
                MINE_EDGE_CHANCE_H
            };
            let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
            for da in -1..=1 {
                for db in 0..=1 {
                    for dc in 0..=1 {
                        let mut l = g;
                        l[axis] += da;
                        l[b] += db;
                        l[c] += dc;
                        let h = hash3(self.seeds.mine ^ (axis as u64 + 1), l[0], l[1], l[2]);
                        if unit_f32(h) >= chance {
                            continue;
                        }
                        let a0 = node(l);
                        let mut hi = l;
                        hi[axis] += 1;
                        let a1 = node(hi);
                        let bx = Aabb::new(
                            a0 - Vec3::splat(MINE_HALF_WIDTH),
                            a1 + Vec3::splat(MINE_HALF_WIDTH),
                        );
                        best = best.max(-bx.sdf(p));
                    }
                }
            }
        }
        // Rooms at the 8 surrounding nodes.
        for dz in 0..=1 {
            for dy in 0..=1 {
                for dx in 0..=1 {
                    let n = [g[0] + dx, g[1] + dy, g[2] + dz];
                    let h = hash3(self.seeds.mine ^ 0x100, n[0], n[1], n[2]);
                    if unit_f32(h) >= MINE_ROOM_CHANCE {
                        continue;
                    }
                    let h2 = mix64(h);
                    let half = Vec3::new(
                        4.0 + 2.0 * unit_f32(h2),
                        3.0 + 1.0 * unit_f32_b(h2),
                        4.0 + 2.0 * unit_f32_b(h),
                    );
                    let bx = Aabb::from_center_half_extents(node(n), half);
                    best = best.max(-bx.sdf(p));
                }
            }
        }
        best
    }

    /// Density: floor-to-ceiling pillars with knobs (rock SDF, capped).
    fn pillars(&self, p: Vec3) -> f32 {
        let cx = det::floor(p.x / PILLAR_CELL) as i32;
        let cz = det::floor(p.z / PILLAR_CELL) as i32;
        let chance = PILLAR_CHANCE * self.density;
        let mut best = PILLAR_CAP;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (x, z) = (cx + dx, cz + dz);
                let h = hash2(self.seeds.pillar, x, z);
                if unit_f32(h) >= chance {
                    continue;
                }
                let h2 = mix64(h);
                let px =
                    (x as f32 + 0.5) * PILLAR_CELL + (unit_f32_b(h) - 0.5) * 2.0 * PILLAR_JITTER;
                let pz =
                    (z as f32 + 0.5) * PILLAR_CELL + (unit_f32(h2) - 0.5) * 2.0 * PILLAR_JITTER;
                let r = 1.5 + 2.5 * unit_f32_b(h2);
                let dxz = Vec3::new(p.x - px, 0.0, p.z - pz).length();
                if dxz - (r + 2.0) >= best {
                    continue;
                }
                let mut d = dxz - r;
                // Two knobs at random heights.
                let h3 = mix64(h2);
                for k in 0..2u32 {
                    let hk = mix64(h3 ^ k as u64);
                    let ky = unit_f32(hk) * self.bounds.max.y;
                    let kr = r + 1.0 + unit_f32_b(hk);
                    d = d.min(p.distance(Vec3::new(px, ky, pz)) - kr);
                }
                best = best.min(d);
            }
        }
        best
    }

    /// Density: faceted floating rocks (rock SDF, capped).
    fn rocks(&self, p: Vec3) -> f32 {
        let c = cell_of(p, ROCK_CELL);
        let chance = ROCK_CHANCE * self.density;
        let mut best = ROCK_CAP;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (x, y, z) = (c[0] + dx, c[1] + dy, c[2] + dz);
                    let h = hash3(self.seeds.rock, x, y, z);
                    if unit_f32(h) >= chance {
                        continue;
                    }
                    let center = jittered_center([x, y, z], ROCK_CELL, ROCK_JITTER, h);
                    let r = 2.0 + 2.5 * unit_f32_b(h);
                    let dist = p.distance(center);
                    if dist - r >= best {
                        continue;
                    }
                    let q = (p - center).abs();
                    // Octahedron bound: (|x|+|y|+|z| - s) / sqrt(3) is 1-Lipschitz.
                    let octa = (q.x + q.y + q.z - 1.35 * r) * 0.577_350_26;
                    best = best.min((dist - r).max(octa));
                }
            }
        }
        best
    }
}

/// Island shape: a main sphere plus two satellites, with the top cut flat.
fn island_sdf(p: Vec3, center: Vec3, r: f32, h: u64) -> f32 {
    let mut d = p.distance(center) - r;
    for k in 0..2u64 {
        let hk = mix64(h ^ (k + 1));
        // Offset direction mostly horizontal, length 0.6 r.
        let dir = Vec3::new(
            unit_f32(hk) - 0.5,
            (unit_f32_b(hk) - 0.5) * 0.5,
            unit_f32(mix64(hk)) - 0.5,
        )
        .try_normalize()
        .unwrap_or(Vec3::X);
        let sc = center + dir * (0.6 * r);
        d = d.min(p.distance(sc) - 0.6 * r);
    }
    d.max(p.y - center.y - 0.3 * r)
}

/// Integer grid cell containing `p` for a cubic cell of side `cell`.
#[inline]
fn cell_of(p: Vec3, cell: f32) -> [i32; 3] {
    [
        det::floor(p.x / cell) as i32,
        det::floor(p.y / cell) as i32,
        det::floor(p.z / cell) as i32,
    ]
}

/// Cell centre plus a hashed offset of at most `jitter` per axis.
#[inline]
fn jittered_center(c: [i32; 3], cell: f32, jitter: f32, h: u64) -> Vec3 {
    let h2 = mix64(h ^ 0x6a09_e667_f3bc_c909);
    let j = Vec3::new(unit_f32(h2), unit_f32_b(h2), unit_f32(mix64(h2))) - Vec3::splat(0.5);
    Vec3::new(
        (c[0] as f32 + 0.5) * cell,
        (c[1] as f32 + 0.5) * cell,
        (c[2] as f32 + 0.5) * cell,
    ) + j * (2.0 * jitter)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn open_fraction(g: &Generator, step: f32) -> f32 {
        let (mut open, mut total) = (0u32, 0u32);
        let r = g.room();
        let mut z = r.min.z + 0.5;
        while z < r.max.z {
            let mut y = r.min.y + 0.5;
            while y < r.max.y {
                let mut x = r.min.x + 0.5;
                while x < r.max.x {
                    total += 1;
                    if g.sample(Vec3::new(x, y, z)) > 0.0 {
                        open += 1;
                    }
                    x += step;
                }
                y += step;
            }
            z += step;
        }
        open as f32 / total as f32
    }

    #[test]
    fn enclosure_closes_the_level() {
        let sky = Generator::new(1, Sliders::new(0, 0));
        let caves = Generator::new(1, Sliders::new(50, 0));
        let mines = Generator::new(1, Sliders::new(100, 0));
        let (fs, fc, fm) = (
            open_fraction(&sky, 6.0),
            open_fraction(&caves, 6.0),
            open_fraction(&mines, 6.0),
        );
        assert!(fs > 0.85, "sky open fraction {fs}");
        assert!(fs > fc && fc > fm, "sky {fs} caves {fc} mines {fm}");
        assert!(fm > 0.03 && fm < 0.45, "mines open fraction {fm}");
        assert!(fc > 0.05 && fc < 0.7, "caves open fraction {fc}");
        // Sky has some islands.
        assert!(fs < 0.999, "no islands at all: {fs}");
    }

    #[test]
    fn density_adds_rock() {
        for e in [0u8, 50, 100] {
            let sparse = open_fraction(&Generator::new(3, Sliders::new(e, 0)), 6.0);
            let dense = open_fraction(&Generator::new(3, Sliders::new(e, 100)), 6.0);
            assert!(dense < sparse, "enclosure {e}: {dense} !< {sparse}");
        }
    }

    #[test]
    fn inert_sliders_do_not_change_terrain() {
        let a = Generator::new(9, Sliders::new(40, 60));
        let mut s = Sliders::new(40, 60);
        s.verticality = 3;
        s.gravity_chaos = 99;
        s.palette = 100;
        s.absurdity = 0;
        s.instability = 77;
        let b = Generator::new(9, s);
        for i in 0..500 {
            let p = Vec3::new(
                (i * 37 % 256) as f32 + 0.3,
                (i * 11 % 128) as f32 + 0.7,
                (i * 53 % 256) as f32 + 0.1,
            );
            assert_eq!(a.sample(p).to_bits(), b.sample(p).to_bits());
        }
    }

    #[test]
    fn sealed_box() {
        for e in [0u8, 50, 100] {
            let g = Generator::new(4, Sliders::new(e, 50));
            let b = g.bounds();
            assert_eq!(b.max, Vec3::new(256.0, 128.0, 256.0));
            // Level faces and everything outside are solid.
            for p in [
                Vec3::new(0.0, 64.0, 128.0),
                Vec3::new(256.0, 64.0, 128.0),
                Vec3::new(128.0, 0.0, 128.0),
                Vec3::new(128.0, 128.0, 128.0),
                Vec3::new(128.0, 64.0, -10.0),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(1000.0, -1000.0, 5.0),
            ] {
                assert!(g.sample(p) < 0.0, "{p:?} not solid at enclosure {e}");
            }
        }
    }

    #[test]
    fn sdf_is_one_lipschitz() {
        let mut rng = SplitMix64::new(77);
        for (seed, e, d) in [
            (1u64, 0u8, 0u8),
            (2, 25, 80),
            (3, 50, 50),
            (4, 75, 20),
            (5, 100, 100),
        ] {
            let g = Generator::new(seed, Sliders::new(e, d));
            let mut worst = 0.0f32;
            for _ in 0..20_000 {
                let p = Vec3::new(
                    rng.next_f32() * 256.0,
                    rng.next_f32() * 128.0,
                    rng.next_f32() * 256.0,
                );
                let dir = Vec3::new(
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                )
                .normalize();
                let step = 0.05 + rng.next_f32() * 3.0;
                let q = p + dir * step;
                let ratio = (g.sample(q) - g.sample(p)).abs() / p.distance(q);
                worst = worst.max(ratio);
            }
            assert!(worst <= 1.0 + 1e-3, "enclosure {e} density {d}: {worst}");
        }
    }

    #[test]
    fn from_code_checks_version() {
        let mut code = ShareCode::new(5, Sliders::default());
        assert!(Generator::from_code(&code).is_ok());
        code.version = GENERATOR_VERSION + 1;
        assert_eq!(
            Generator::from_code(&code),
            Err(WorldgenError::UnsupportedVersion(GENERATOR_VERSION + 1))
        );
    }

    #[test]
    fn seeds_change_the_world() {
        let a = Generator::new(1, Sliders::new(50, 50));
        let b = Generator::new(2, Sliders::new(50, 50));
        let differing = (0..200)
            .filter(|&i| {
                let p = Vec3::new(10.0 + i as f32, 40.0, 100.0);
                a.sample(p) != b.sample(p)
            })
            .count();
        assert!(differing > 150);
    }
}
