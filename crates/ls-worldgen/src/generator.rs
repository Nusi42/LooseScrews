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
//! pillars with knobs and faceted floating rocks (union via `min`). Then the
//! connection network ([`crate::network`]) carves caverns and tunnels that
//! link both team ends and the level centre (`max(terrain, carve)`), plus
//! the Heist site (the core vein chamber with its rock knot at the bottom
//! and the extraction caverns at the top; generator version 3), and the
//! level is sealed by walls [`WALL_THICKNESS`] thick inside the level box.
//!
//! Feature lookups use jittered grids; each feature stays inside its grid
//! cell, so only the neighbouring cells need to be visited, and fields are
//! capped at a value that features further away can never beat. That keeps
//! the fields continuous (and 1-Lipschitz) while each sample stays cheap.

use std::cell::OnceCell;

use ls_math::level::{Archetype, BeltSite, WellSite};
use ls_math::rng::{mix64, SplitMix64};
use ls_math::{det, Aabb, Vec3};

use crate::network::Network;
use crate::noise::{
    cell_gradients, gradient_noise_row, gradient_noise_with, hash2, hash3, unit_f32, unit_f32_b,
    CellGradients, NOISE_LIPSCHITZ,
};
use crate::{ShareCode, Sliders, WorldgenError, CHUNK_CELLS, GENERATOR_VERSION};

/// Thickness of the sealing wall inside the level box (u).
pub const WALL_THICKNESS: f32 = 2.0;
/// Default level size in chunks (256 x 128 x 256 u): the classic box at
/// Verticality 40-59.
pub const DEFAULT_SIZE_CHUNKS: [i32; 3] = [8, 4, 8];
/// The five classic level boxes in chunks, by Verticality: 0-19, 20-39,
/// 40-59, 60-79, 80-100 (flat and wide to tall and narrow, about the same
/// chunk count; M4 plan §3.3). The team axis is x in every shape.
pub const BOX_SHAPES: [[i32; 3]; 5] = [[10, 2, 10], [9, 3, 9], [8, 4, 8], [7, 5, 7], [6, 7, 6]];
/// The asteroid belt's level box in chunks (320 x 96 x 320 u).
pub const BELT_SIZE_CHUNKS: [i32; 3] = [10, 3, 10];

/// The classic level box for a Verticality value (see [`BOX_SHAPES`]).
pub fn box_shape(verticality: u8) -> [i32; 3] {
    BOX_SHAPES[(verticality.min(100) / 20).min(4) as usize]
}

/// The inner structure of a classic level for one Verticality value (M4
/// plan §3.3): chamber noise stretched vertically (flat: wide, low
/// chambers; tall: chimneys), the mine lattice's vertical tunnel chance,
/// and islands and rocks stacked in tiers in tall boxes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Structure {
    chamber_freq: [f32; 3],
    chamber_scale: f32,
    mine_edge_v: f32,
    island_cell: Vec3,
    island_jitter: Vec3,
    rock_jitter: Vec3,
}

impl Structure {
    fn for_verticality(verticality: u8) -> Self {
        let v = verticality.min(100) as f32 / 100.0;
        // Vertical stretch of the chambers: 0.5 at 0, 1 at 50, 2.2 at 100;
        // the horizontal size changes inversely, by its square root.
        let stretch = if v <= 0.5 {
            0.5 + v
        } else {
            1.0 + (v - 0.5) * 2.4
        };
        let horizontal = stretch.sqrt();
        let chamber_freq = [
            CHAMBER_FREQ[0] * horizontal,
            CHAMBER_FREQ[1] / stretch,
            CHAMBER_FREQ[2] * horizontal,
        ];
        let max_freq = chamber_freq[0].max(chamber_freq[1]).max(chamber_freq[2]);
        let (island_cell_y, island_jitter_y) = if verticality < 20 {
            (ISLAND_CELL_Y_FLAT, ISLAND_JITTER_Y_FLAT)
        } else if verticality < TIER_VERTICALITY {
            (ISLAND_CELL, ISLAND_JITTER)
        } else {
            (ISLAND_CELL_Y_TIERS, ISLAND_JITTER_Y_TIERS)
        };
        let rock_jitter_y = if verticality < TIER_VERTICALITY {
            ROCK_JITTER
        } else {
            ROCK_JITTER_Y_TIERS
        };
        Structure {
            chamber_freq,
            chamber_scale: 1.0 / (NOISE_LIPSCHITZ * max_freq),
            mine_edge_v: MINE_EDGE_CHANCE_V[0]
                + (MINE_EDGE_CHANCE_V[1] - MINE_EDGE_CHANCE_V[0]) * v,
            island_cell: Vec3::new(ISLAND_CELL, island_cell_y, ISLAND_CELL),
            island_jitter: Vec3::new(ISLAND_JITTER, island_jitter_y, ISLAND_JITTER),
            rock_jitter: Vec3::new(ROCK_JITTER, rock_jitter_y, ROCK_JITTER),
        }
    }
}

/// The level box in chunks of a level with this archetype and sliders.
pub fn level_size_chunks(archetype: Archetype, sliders: &Sliders) -> [i32; 3] {
    match archetype {
        Archetype::Classic => box_shape(sliders.verticality),
        Archetype::Belt => BELT_SIZE_CHUNKS,
    }
}

// --- Open sky: floating islands -------------------------------------------
const ISLAND_CELL: f32 = 48.0;
/// Island cell heights by level shape: flat boxes (Verticality below 20)
/// keep one tier, tall boxes (60 and above) stack islands in tiers.
const ISLAND_CELL_Y_FLAT: f32 = 64.0;
const ISLAND_CELL_Y_TIERS: f32 = 36.0;
/// Vertical island jitter for those cell heights (jitter plus the island
/// reach of at most 12 u stays inside the cell).
const ISLAND_JITTER_Y_FLAT: f32 = 8.0;
const ISLAND_JITTER_Y_TIERS: f32 = 3.0;
const ISLAND_CHANCE: f32 = 0.35;
/// Islands never reach further than this from their cell centre (< cell/2).
const ISLAND_JITTER: f32 = 9.6;
const SKY_CAP: f32 = 20.0;

// --- Caverns ----------------------------------------------------------------
/// Chamber noise frequency (per axis) and scale; scale * max_freq * NOISE_LIPSCHITZ <= 1.
const CHAMBER_FREQ: [f32; 3] = [1.0 / 40.0, 1.0 / 28.0, 1.0 / 40.0];
const CHAMBER_THRESHOLD: f32 = 0.28;
const TUNNEL_FREQ: f32 = 1.0 / 24.0;
const TUNNEL_SCALE: f32 = 24.0 / NOISE_LIPSCHITZ;
const TUNNEL_WIDTH: f32 = 0.11;

// --- Mine shafts --------------------------------------------------------------
const MINE_SPACING: f32 = 16.0;
const MINE_HALF_WIDTH: f32 = 2.5;
const MINE_EDGE_CHANCE_H: f32 = 0.55;
/// Vertical tunnel chance of the mine lattice at Verticality 0 and 100.
const MINE_EDGE_CHANCE_V: [f32; 2] = [0.05, 0.6];
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
/// Vertical rock jitter in tall boxes (Verticality 60 and above): rocks
/// line up in tiers one rock cell apart.
const ROCK_JITTER_Y_TIERS: f32 = 1.5;
/// From this Verticality on, islands and rocks stack in tiers.
pub const TIER_VERTICALITY: u8 = 60;
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
    connect: u64,
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
            connect: sm.next_u64(),
        }
    }
}

/// Deterministic level generator for one share code.
#[derive(Debug, Clone, PartialEq)]
pub struct Generator {
    code: ShareCode,
    archetype: Archetype,
    /// Gravity well sites (Gravity Chaos), in placement order.
    wells: Vec<WellSite>,
    /// The asteroid belt (belt levels only).
    belt_site: Option<BeltSite>,
    size_chunks: [i32; 3],
    bounds: Aabb,
    room: Aabb,
    seeds: Seeds,
    w_sky: f32,
    w_cave: f32,
    w_mine: f32,
    density: f32,
    mine_offset: Vec3,
    /// Chamber noise frequency per axis (stretched by Verticality) and the
    /// scale that keeps the caverns field 1-Lipschitz.
    chamber_freq: [f32; 3],
    chamber_scale: f32,
    /// Vertical tunnel chance of the mine lattice (Verticality).
    mine_edge_v: f32,
    /// Island cell size and jitter per axis (tiers in tall boxes).
    island_cell: Vec3,
    island_jitter: Vec3,
    /// Rock jitter per axis (tiers in tall boxes).
    rock_jitter: Vec3,
    network: Network,
    /// The asteroid belt's field (belt levels only).
    belt: Option<Box<crate::belt::Belt>>,
}

impl Generator {
    /// A classic level generator; the level box follows Verticality
    /// ([`box_shape`]).
    pub fn new(seed: u64, sliders: Sliders) -> Self {
        Self::with_archetype(seed, sliders, Archetype::Classic)
    }

    /// A generator for the given archetype; the level box follows it
    /// ([`level_size_chunks`]).
    pub fn with_archetype(seed: u64, sliders: Sliders, archetype: Archetype) -> Self {
        Self::build(
            seed,
            sliders,
            archetype,
            level_size_chunks(archetype, &sliders),
        )
    }

    /// A generator for a decoded share code. Fails if the code was made by a
    /// different generator version or has invalid sliders.
    pub fn from_code(code: &ShareCode) -> Result<Self, WorldgenError> {
        if code.version != GENERATOR_VERSION {
            return Err(WorldgenError::UnsupportedVersion(code.version));
        }
        code.sliders.validate()?;
        Ok(Self::with_archetype(
            code.seed,
            code.sliders,
            code.archetype,
        ))
    }

    /// A classic generator with a custom level size in chunks (each at
    /// least 1; tests and tools).
    pub fn with_size(seed: u64, sliders: Sliders, size_chunks: [i32; 3]) -> Self {
        Self::build(seed, sliders, Archetype::Classic, size_chunks)
    }

    fn build(seed: u64, sliders: Sliders, archetype: Archetype, size_chunks: [i32; 3]) -> Self {
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
        let classic = archetype == Archetype::Classic;
        let e = sliders.enclosure.min(100) as f32 / 100.0;
        let (w_sky, w_cave, w_mine) = if classic {
            let w_sky = (1.0 - 2.0 * e).max(0.0);
            let w_mine = (2.0 * e - 1.0).max(0.0);
            (w_sky, 1.0 - w_sky - w_mine, w_mine)
        } else {
            (0.0, 0.0, 0.0)
        };
        let off = mix64(seeds.mine);
        let mine_offset = Vec3::new(
            (off & 15) as f32,
            ((off >> 8) & 15) as f32,
            ((off >> 16) & 15) as f32,
        );
        let structure = Structure::for_verticality(sliders.verticality);
        let network = if classic {
            Network::build_with(
                seeds.connect,
                sliders.enclosure.min(100),
                sliders.verticality.min(100),
                room,
            )
        } else {
            Network::empty()
        };
        let mut gen = Generator {
            code: ShareCode::with_archetype(seed, sliders, archetype),
            archetype,
            wells: Vec::new(),
            belt_site: None,
            size_chunks,
            bounds,
            room,
            seeds,
            w_sky,
            w_cave,
            w_mine,
            density: if classic {
                sliders.density.min(100) as f32 / 100.0
            } else {
                0.0
            },
            mine_offset,
            chamber_freq: structure.chamber_freq,
            chamber_scale: structure.chamber_scale,
            mine_edge_v: structure.mine_edge_v,
            island_cell: structure.island_cell,
            island_jitter: structure.island_jitter,
            rock_jitter: structure.rock_jitter,
            network,
            belt: None,
        };
        match archetype {
            Archetype::Classic => {
                gen.wells = crate::wells::place(
                    &gen,
                    seeds.connect,
                    sliders.gravity_chaos,
                    sliders.enclosure,
                );
                // The hollows and cores join the network's carving (its
                // index keeps the per-sample cost low).
                gen.network.add_wells(&gen.wells);
            }
            Archetype::Belt => {
                let belt = crate::belt::Belt::build(seed, &sliders, bounds);
                gen.wells = belt.moons().to_vec();
                gen.belt_site = Some(belt.site().clone());
                gen.belt = Some(Box::new(belt));
            }
        }
        gen
    }

    /// The share code (seed, sliders, current version) this generator uses.
    pub fn code(&self) -> ShareCode {
        self.code
    }

    /// The level archetype (classic or asteroid belt).
    pub fn archetype(&self) -> Archetype {
        self.archetype
    }

    /// The level's gravity well sites (Gravity Chaos; on belt levels the
    /// shepherd moons). Empty at Gravity Chaos 0.
    pub fn gravity_wells(&self) -> &[WellSite] {
        &self.wells
    }

    /// The asteroid belt (start slots, gates, exit, pickup points, boulder
    /// orbits); `None` on classic levels.
    pub fn belt(&self) -> Option<&BeltSite> {
        self.belt_site.as_ref()
    }

    /// The asteroid belt's field (corridor, rock count; belt levels only).
    pub fn belt_field(&self) -> Option<&crate::belt::Belt> {
        self.belt.as_deref()
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
        // Same bits as `-self.room.sdf(p)`: inside the room the outside
        // term of the box SDF is sqrt(0) = +0, so it can be skipped.
        let q = (p - self.room.center()).abs() - self.room.half_extents();
        if q.x < 0.0 && q.y < 0.0 && q.z < 0.0 {
            -q.max_element()
        } else {
            -self.room.sdf(p)
        }
    }

    /// The carved connection network (hubs and tunnels linking both team
    /// ends of the level).
    pub fn network(&self) -> &Network {
        &self.network
    }

    /// The Heist site of this level (core vein and extraction zones; see
    /// [`crate::network::HeistSite`]). `None` only for levels too small for
    /// a connection network.
    pub fn heist(&self) -> Option<&crate::network::HeistSite> {
        self.network.heist()
    }

    /// Whether a ship keeping `margin` of free space can fly in a straight
    /// line from `p` to the centre lines of the connection network in the
    /// pristine level ([`Network::reaches`] with [`Generator::sample`]).
    /// The network links both team ends, so such points share one
    /// connected open volume.
    pub fn reaches_network(&self, p: Vec3, margin: f32, tries: usize) -> bool {
        self.network.reaches(|x| self.sample(x), p, margin, tries)
    }

    /// Rock material (strength class) at `p`; see [`crate::material`].
    /// Separate from the SDF: it does not change [`Generator::sample`].
    pub fn material_at(&self, p: Vec3) -> crate::Material {
        crate::MaterialField::new(self.code.seed).at(p)
    }

    /// The terrain without the sealing walls (negative in rock).
    pub fn terrain(&self, p: Vec3) -> f32 {
        self.terrain_with(self, p)
    }

    /// A sampler for many points in `region` (see [`RegionSampler`]).
    /// Bit-identical to [`Generator::sample`] everywhere.
    pub fn region_sampler(&self, region: Aabb) -> RegionSampler<'_> {
        RegionSampler::new(self, region, true)
    }

    /// A [`RegionSampler`] with feature tables only, no per-block candidate
    /// lists: cheaper for sparse samples (a few per 8 u cube), such as a
    /// coarse scan of the whole level.
    pub fn sparse_region_sampler(&self, region: Aabb) -> RegionSampler<'_> {
        RegionSampler::new(self, region, false)
    }

    /// [`Generator::terrain`] with feature data from `f` (hashed by the
    /// generator itself, or read from a [`RegionSampler`]'s tables). The
    /// arithmetic and the order of every operation are the same either way,
    /// so the result is too.
    #[inline(always)]
    fn terrain_with<F: Features>(&self, f: &F, p: Vec3) -> f32 {
        self.terrain_parts(f, p, || self.caverns(f, p))
    }

    /// [`Generator::terrain_with`] with the caverns field supplied by
    /// `caverns` (only called when the field is active).
    #[inline(always)]
    fn terrain_parts<F: Features>(&self, f: &F, p: Vec3, caverns: impl FnOnce() -> f32) -> f32 {
        if let Some(belt) = &self.belt {
            return belt.terrain(p);
        }
        let mut open = 0.0f32;
        if self.w_sky > 0.0 {
            open += self.w_sky * self.sky(f, p);
        }
        if self.w_cave > 0.0 {
            open += self.w_cave * caverns();
        }
        if self.w_mine > 0.0 {
            open += self.w_mine * self.mines(f, p);
        }
        if self.density > 0.0 {
            open = open.min(self.pillars(f, p)).min(self.rocks(f, p));
        }
        self.network.carve(p, open)
    }

    /// Open sky: distance to the nearest floating island, capped at [`SKY_CAP`].
    #[inline(always)]
    fn sky<F: Features>(&self, f: &F, p: Vec3) -> f32 {
        let mut best = SKY_CAP;
        f.visit_islands(f.island_cell(p), |isl, sats| {
            // Main radius 5..10; satellites reach 1.2 r (<= jitter margin).
            let dc = p.distance(isl.center);
            if dc - isl.reach >= best {
                return;
            }
            let sats = match sats {
                Some(s) => *s,
                None => isl.satellites(),
            };
            best = best.min(isl.sdf(p, dc, &sats));
        });
        best
    }

    /// Caverns: chambers and noise tunnels, positive where open.
    #[inline(always)]
    fn caverns<F: Features>(&self, f: &F, p: Vec3) -> f32 {
        let f3 = self.chamber_freq;
        let q = Vec3::new(p.x * f3[0], p.y * f3[1], p.z * f3[2]);
        let t = p * TUNNEL_FREQ;
        caverns_from_noise(
            self.chamber_scale,
            gradient_noise_with(q, |x, y, z| f.gradients(NOISE_CHAMBER, [x, y, z])),
            gradient_noise_with(t, |x, y, z| f.gradients(NOISE_TUNNEL_A, [x, y, z])),
            gradient_noise_with(t, |x, y, z| f.gradients(NOISE_TUNNEL_B, [x, y, z])),
        )
    }

    /// Mine shafts: union of lattice tunnels and rooms, floored at `-MINE_CAP`.
    #[inline(always)]
    fn mines<F: Features>(&self, f: &F, p: Vec3) -> f32 {
        let mut best = -MINE_CAP;
        f.visit_mine_boxes(f.mine_cell(p, self.mine_offset), |bx| {
            best = bx.max_neg_sdf(p, best);
        });
        best
    }

    /// Density: floor-to-ceiling pillars with knobs (rock SDF, capped).
    #[inline(always)]
    fn pillars<F: Features>(&self, f: &F, p: Vec3) -> f32 {
        let [cx, _, cz] = f.pillar_column(p);
        let mut best = PILLAR_CAP;
        f.visit_pillars(cx, cz, |pil, knobs| {
            let dxz = Vec3::new(p.x - pil.px, 0.0, p.z - pil.pz).length();
            if dxz - pil.reach >= best {
                return;
            }
            let mut d = dxz - pil.r;
            // Two knobs at random heights.
            let knobs = match knobs {
                Some(k) => *k,
                None => self.pillar_knobs_of(pil),
            };
            for (knob, kr) in knobs {
                d = d.min(p.distance(knob) - kr);
            }
            best = best.min(d);
        });
        best
    }

    /// Density: faceted floating rocks (rock SDF, capped).
    #[inline(always)]
    fn rocks<F: Features>(&self, f: &F, p: Vec3) -> f32 {
        let mut best = ROCK_CAP;
        f.visit_rocks(f.rock_cell(p), |rock| {
            let dist = p.distance(rock.center);
            if dist - rock.r >= best {
                return;
            }
            let q = (p - rock.center).abs();
            // Octahedron bound: (|x|+|y|+|z| - s) / sqrt(3) is 1-Lipschitz.
            let octa = (q.x + q.y + q.z - rock.octa) * 0.577_350_26;
            best = best.min((dist - rock.r).max(octa));
        });
        best
    }

    // --- Feature data, hashed per lookup ---------------------------------------

    #[inline(always)]
    fn island_at(&self, c: [i32; 3]) -> Option<Island> {
        let h = hash3(self.seeds.island, c[0], c[1], c[2]);
        if unit_f32(h) >= ISLAND_CHANCE {
            return None;
        }
        let center = jittered_center(c, self.island_cell, self.island_jitter, h);
        let r = 5.0 + 5.0 * unit_f32_b(h);
        Some(Island {
            center,
            r,
            reach: 1.2 * r,
            sat_r: 0.6 * r,
            top: 0.3 * r,
            h: mix64(h),
        })
    }

    #[inline(always)]
    fn rock_at(&self, c: [i32; 3]) -> Option<Rock> {
        let h = hash3(self.seeds.rock, c[0], c[1], c[2]);
        if unit_f32(h) >= ROCK_CHANCE * self.density {
            return None;
        }
        let center = jittered_center(c, Vec3::splat(ROCK_CELL), self.rock_jitter, h);
        let r = 2.0 + 2.5 * unit_f32_b(h);
        Some(Rock {
            center,
            r,
            octa: 1.35 * r,
        })
    }

    #[inline(always)]
    fn pillar_at(&self, x: i32, z: i32) -> Option<Pillar> {
        let h = hash2(self.seeds.pillar, x, z);
        if unit_f32(h) >= PILLAR_CHANCE * self.density {
            return None;
        }
        let h2 = mix64(h);
        let px = (x as f32 + 0.5) * PILLAR_CELL + (unit_f32_b(h) - 0.5) * 2.0 * PILLAR_JITTER;
        let pz = (z as f32 + 0.5) * PILLAR_CELL + (unit_f32(h2) - 0.5) * 2.0 * PILLAR_JITTER;
        let r = 1.5 + 2.5 * unit_f32_b(h2);
        Some(Pillar {
            px,
            pz,
            r,
            reach: r + 2.0,
            h: mix64(h2),
        })
    }

    fn pillar_knobs_of(&self, pil: &Pillar) -> [(Vec3, f32); 2] {
        [0u32, 1].map(|k| {
            let hk = mix64(pil.h ^ k as u64);
            let ky = unit_f32(hk) * self.bounds.max.y;
            let kr = pil.r + 1.0 + unit_f32_b(hk);
            (Vec3::new(pil.px, ky, pil.pz), kr)
        })
    }

    /// World position of mine lattice node `i`.
    #[inline(always)]
    fn mine_node(&self, i: [i32; 3]) -> Vec3 {
        let s = MINE_SPACING;
        Vec3::new(i[0] as f32 * s, i[1] as f32 * s, i[2] as f32 * s) + self.mine_offset
    }

    /// The tunnel from lattice node `l` along +`axis`, if there is one.
    #[inline(always)]
    fn mine_edge_at(&self, axis: usize, l: [i32; 3]) -> Option<Aabb> {
        let chance = if axis == 1 {
            self.mine_edge_v
        } else {
            MINE_EDGE_CHANCE_H
        };
        let h = hash3(self.seeds.mine ^ (axis as u64 + 1), l[0], l[1], l[2]);
        if unit_f32(h) >= chance {
            return None;
        }
        let mut hi = l;
        hi[axis] += 1;
        Some(Aabb::new(
            self.mine_node(l) - Vec3::splat(MINE_HALF_WIDTH),
            self.mine_node(hi) + Vec3::splat(MINE_HALF_WIDTH),
        ))
    }

    /// The room at lattice node `n`, if there is one.
    #[inline(always)]
    fn mine_room_at(&self, n: [i32; 3]) -> Option<Aabb> {
        let h = hash3(self.seeds.mine ^ 0x100, n[0], n[1], n[2]);
        if unit_f32(h) >= MINE_ROOM_CHANCE {
            return None;
        }
        let h2 = mix64(h);
        let half = Vec3::new(
            4.0 + 2.0 * unit_f32(h2),
            3.0 + 1.0 * unit_f32_b(h2),
            4.0 + 2.0 * unit_f32_b(h),
        );
        Some(Aabb::from_center_half_extents(self.mine_node(n), half))
    }

    #[inline(always)]
    fn noise_seed(&self, field: usize) -> u64 {
        match field {
            NOISE_CHAMBER => self.seeds.chamber,
            NOISE_TUNNEL_A => self.seeds.tunnel_a,
            _ => self.seeds.tunnel_b,
        }
    }
}

/// The caverns field from its three noise values (chamber, tunnel a, tunnel
/// b). The result does not depend on the sign of a zero noise value.
#[inline(always)]
fn caverns_from_noise(scale: f32, chamber: f32, a: f32, b: f32) -> f32 {
    let chamber = scale * (chamber - CHAMBER_THRESHOLD);
    let tunnel = TUNNEL_SCALE * (TUNNEL_WIDTH - a.abs().max(b.abs()));
    chamber.max(tunnel)
}

const NOISE_CHAMBER: usize = 0;
const NOISE_TUNNEL_A: usize = 1;
const NOISE_TUNNEL_B: usize = 2;

/// A floating island: main sphere, two satellites, flat top.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Island {
    center: Vec3,
    r: f32,
    /// Satellites never reach further than this from the centre (1.2 r).
    reach: f32,
    sat_r: f32,
    top: f32,
    /// Hash the satellite directions are drawn from.
    h: u64,
}

impl Island {
    /// Satellite centres: offset direction mostly horizontal, length 0.6 r.
    fn satellites(&self) -> [Vec3; 2] {
        [0u64, 1].map(|k| {
            let hk = mix64(self.h ^ (k + 1));
            let dir = Vec3::new(
                unit_f32(hk) - 0.5,
                (unit_f32_b(hk) - 0.5) * 0.5,
                unit_f32(mix64(hk)) - 0.5,
            )
            .try_normalize()
            .unwrap_or(Vec3::X);
            self.center + dir * self.sat_r
        })
    }

    /// Island SDF at `p` (top cut flat), given `dc = |p - center|`.
    #[inline(always)]
    fn sdf(&self, p: Vec3, dc: f32, sats: &[Vec3; 2]) -> f32 {
        let mut d = dc - self.r;
        for sc in sats {
            d = d.min(p.distance(*sc) - self.sat_r);
        }
        d.max(p.y - self.center.y - self.top)
    }
}

/// A faceted floating rock (sphere intersected with an octahedron).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rock {
    center: Vec3,
    r: f32,
    /// Octahedron size (1.35 r).
    octa: f32,
}

/// A floor-to-ceiling pillar.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pillar {
    px: f32,
    pz: f32,
    r: f32,
    /// Knobs never reach further than this from the axis (r + 2).
    reach: f32,
    /// Hash the knobs are drawn from.
    h: u64,
}

type Knobs = [(Vec3, f32); 2];

/// An island with its satellite centres.
type IslandShape = (Island, [Vec3; 2]);

/// Where the generator fields get their features from. Each `visit_*`
/// calls `f` for the features around a point's cell, in the fields' fixed
/// visiting order; an implementation may leave out features that provably
/// cannot change the field (see [`Block`]). Optional arguments carry
/// precomputed shape data (computed on demand when `None`).
trait Features {
    /// The generator the features belong to.
    fn generator(&self) -> &Generator;
    /// Island cell of `p`.
    #[inline(always)]
    fn island_cell(&self, p: Vec3) -> [i32; 3] {
        cell_of3(p, self.generator().island_cell)
    }
    /// Rock cell of `p`.
    #[inline(always)]
    fn rock_cell(&self, p: Vec3) -> [i32; 3] {
        cell_of(p, ROCK_CELL)
    }
    /// Pillar column of `p` as `[x, 0, z]`.
    #[inline(always)]
    fn pillar_column(&self, p: Vec3) -> [i32; 3] {
        [
            floor_i32(p.x / PILLAR_CELL),
            0,
            floor_i32(p.z / PILLAR_CELL),
        ]
    }
    /// Mine lattice cell of `p` for the lattice offset `offset`.
    #[inline(always)]
    fn mine_cell(&self, p: Vec3, offset: Vec3) -> [i32; 3] {
        cell_of(p - offset, MINE_SPACING)
    }
    /// Islands of the 27 cells around island cell `c`.
    fn visit_islands(&self, c: [i32; 3], f: impl FnMut(&Island, Option<&[Vec3; 2]>));
    /// Rocks of the 27 cells around rock cell `c`.
    fn visit_rocks(&self, c: [i32; 3], f: impl FnMut(&Rock));
    /// Pillars of the 9 columns around column `(x, z)`.
    fn visit_pillars(&self, x: i32, z: i32, f: impl FnMut(&Pillar, Option<&Knobs>));
    /// Tunnel boxes (36 edges) then room boxes (8 nodes) around lattice cell `g`.
    fn visit_mine_boxes(&self, g: [i32; 3], f: impl FnMut(&MineBox));
    /// Gradient of noise lattice point `c` of noise field `field`.
    fn gradients(&self, field: usize, c: [i32; 3]) -> CellGradients;
}

/// Island cells visited around `c`, in order.
#[inline(always)]
fn island_cells(c: [i32; 3], mut f: impl FnMut([i32; 3])) {
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                f([c[0] + dx, c[1] + dy, c[2] + dz]);
            }
        }
    }
}

/// Mine features visited around lattice cell `g`, in order: `(Some(axis),
/// lower node)` for the 36 tunnel edges, then `(None, node)` for 8 rooms.
#[inline(always)]
fn mine_features(g: [i32; 3], mut f: impl FnMut(Option<usize>, [i32; 3])) {
    for axis in 0..3usize {
        let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
        for da in -1..=1 {
            for db in 0..=1 {
                for dc in 0..=1 {
                    let mut l = g;
                    l[axis] += da;
                    l[b] += db;
                    l[c] += dc;
                    f(Some(axis), l);
                }
            }
        }
    }
    for dz in 0..=1 {
        for dy in 0..=1 {
            for dx in 0..=1 {
                f(None, [g[0] + dx, g[1] + dy, g[2] + dz]);
            }
        }
    }
}

impl Features for Generator {
    #[inline(always)]
    fn generator(&self) -> &Generator {
        self
    }
    #[inline(always)]
    fn visit_islands(&self, c: [i32; 3], mut f: impl FnMut(&Island, Option<&[Vec3; 2]>)) {
        island_cells(c, |cell| {
            if let Some(isl) = self.island_at(cell) {
                f(&isl, None);
            }
        });
    }
    #[inline(always)]
    fn visit_rocks(&self, c: [i32; 3], mut f: impl FnMut(&Rock)) {
        island_cells(c, |cell| {
            if let Some(rock) = self.rock_at(cell) {
                f(&rock);
            }
        });
    }
    #[inline(always)]
    fn visit_pillars(&self, x: i32, z: i32, mut f: impl FnMut(&Pillar, Option<&Knobs>)) {
        for dz in -1..=1 {
            for dx in -1..=1 {
                if let Some(pil) = self.pillar_at(x + dx, z + dz) {
                    f(&pil, None);
                }
            }
        }
    }
    #[inline(always)]
    fn visit_mine_boxes(&self, g: [i32; 3], mut f: impl FnMut(&MineBox)) {
        mine_features(g, |axis, n| {
            let bx = match axis {
                Some(axis) => self.mine_edge_at(axis, n),
                None => self.mine_room_at(n),
            };
            if let Some(bx) = bx {
                f(&MineBox::new(&bx));
            }
        });
    }
    #[inline(always)]
    fn gradients(&self, field: usize, c: [i32; 3]) -> CellGradients {
        cell_gradients(self.noise_seed(field), c[0], c[1], c[2])
    }
}

/// A dense table over an integer box of cells.
#[derive(Debug, Clone)]
struct Table<T> {
    lo: [i32; 3],
    dims: [usize; 3],
    data: Vec<T>,
}

impl<T> Table<T> {
    /// Cells `lo..=hi`, filled by `f`.
    fn build(lo: [i32; 3], hi: [i32; 3], mut f: impl FnMut([i32; 3]) -> T) -> Self {
        let dims = [0, 1, 2].map(|a| (hi[a] - lo[a] + 1).max(0) as usize);
        let mut data = Vec::with_capacity(dims[0] * dims[1] * dims[2]);
        for z in 0..dims[2] as i32 {
            for y in 0..dims[1] as i32 {
                for x in 0..dims[0] as i32 {
                    data.push(f([lo[0] + x, lo[1] + y, lo[2] + z]));
                }
            }
        }
        Table { lo, dims, data }
    }

    #[inline(always)]
    fn get(&self, c: [i32; 3]) -> Option<&T> {
        let x = c[0].wrapping_sub(self.lo[0]) as u32 as usize;
        let y = c[1].wrapping_sub(self.lo[1]) as u32 as usize;
        let z = c[2].wrapping_sub(self.lo[2]) as u32 as usize;
        if x >= self.dims[0] || y >= self.dims[1] || z >= self.dims[2] {
            return None;
        }
        Some(&self.data[x + self.dims[0] * (y + self.dims[1] * z)])
    }
}

/// Mine features of one lattice node.
#[derive(Debug, Clone, Copy)]
struct MineNode {
    /// Tunnel boxes from this node along +x, +y, +z.
    edges: [Option<MineBox>; 3],
    room: Option<MineBox>,
}

/// Feature tables covering a region (with fallback to hashing outside it).
#[derive(Debug, Clone)]
struct Tables<'a> {
    gen: &'a Generator,
    islands: Option<Table<Option<IslandShape>>>,
    gradients: Option<[Table<CellGradients>; 3]>,
    mines: Option<Table<MineNode>>,
    pillars: Option<Table<Option<(Pillar, Knobs)>>>,
    rocks: Option<Table<Option<Rock>>>,
}

impl<'a> Tables<'a> {
    fn new(gen: &'a Generator, region: Aabb) -> Self {
        let (lo, hi) = (region.min, region.max);
        // Every field visits the cells around the point's own cell.
        let cells = |cell: f32| {
            let (a, b) = (cell_of(lo, cell), cell_of(hi, cell));
            (a.map(|v| v - 1), b.map(|v| v + 1))
        };
        let islands = (gen.w_sky > 0.0).then(|| {
            let (a, b) = (
                cell_of3(lo, gen.island_cell).map(|v| v - 1),
                cell_of3(hi, gen.island_cell).map(|v| v + 1),
            );
            Table::build(a, b, |c| gen.island_at(c).map(|i| (i, i.satellites())))
        });
        let gradients = (gen.w_cave > 0.0).then(|| {
            let lattice = |freq: [f32; 3]| {
                let f = |p: Vec3| {
                    [
                        det::floor(p.x * freq[0]) as i32,
                        det::floor(p.y * freq[1]) as i32,
                        det::floor(p.z * freq[2]) as i32,
                    ]
                };
                (f(lo), f(hi))
            };
            let (ca, cb) = lattice(gen.chamber_freq);
            let (ta, tb) = lattice([TUNNEL_FREQ; 3]);
            [
                (gen.seeds.chamber, ca, cb),
                (gen.seeds.tunnel_a, ta, tb),
                (gen.seeds.tunnel_b, ta, tb),
            ]
            .map(|(seed, a, b)| Table::build(a, b, |c| cell_gradients(seed, c[0], c[1], c[2])))
        });
        let mines = (gen.w_mine > 0.0).then(|| {
            let a = cell_of(lo - gen.mine_offset, MINE_SPACING).map(|v| v - 1);
            let b = cell_of(hi - gen.mine_offset, MINE_SPACING).map(|v| v + 1);
            Table::build(a, b, |n| MineNode {
                edges: [0, 1, 2].map(|axis| gen.mine_edge_at(axis, n).map(|b| MineBox::new(&b))),
                room: gen.mine_room_at(n).map(|b| MineBox::new(&b)),
            })
        });
        let (pillars, rocks) = if gen.density > 0.0 {
            let (a, b) = cells(PILLAR_CELL);
            let pillars = Table::build([a[0], 0, a[2]], [b[0], 0, b[2]], |c| {
                gen.pillar_at(c[0], c[2])
                    .map(|p| (p, gen.pillar_knobs_of(&p)))
            });
            let (a, b) = cells(ROCK_CELL);
            (Some(pillars), Some(Table::build(a, b, |c| gen.rock_at(c))))
        } else {
            (None, None)
        };
        Tables {
            gen,
            islands,
            gradients,
            mines,
            pillars,
            rocks,
        }
    }
}

impl Features for Tables<'_> {
    #[inline(always)]
    fn generator(&self) -> &Generator {
        self.gen
    }
    #[inline(always)]
    fn visit_islands(&self, c: [i32; 3], mut f: impl FnMut(&Island, Option<&[Vec3; 2]>)) {
        island_cells(c, |cell| {
            match self.islands.as_ref().and_then(|t| t.get(cell)) {
                Some(Some((isl, sats))) => f(isl, Some(sats)),
                Some(None) => {}
                None => {
                    if let Some(isl) = self.gen.island_at(cell) {
                        f(&isl, None);
                    }
                }
            }
        });
    }
    #[inline(always)]
    fn visit_rocks(&self, c: [i32; 3], mut f: impl FnMut(&Rock)) {
        island_cells(c, |cell| {
            match self.rocks.as_ref().and_then(|t| t.get(cell)) {
                Some(Some(rock)) => f(rock),
                Some(None) => {}
                None => {
                    if let Some(rock) = self.gen.rock_at(cell) {
                        f(&rock);
                    }
                }
            }
        });
    }
    #[inline(always)]
    fn visit_pillars(&self, x: i32, z: i32, mut f: impl FnMut(&Pillar, Option<&Knobs>)) {
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (cx, cz) = (x + dx, z + dz);
                match self.pillars.as_ref().and_then(|t| t.get([cx, 0, cz])) {
                    Some(Some((pil, knobs))) => f(pil, Some(knobs)),
                    Some(None) => {}
                    None => {
                        if let Some(pil) = self.gen.pillar_at(cx, cz) {
                            f(&pil, None);
                        }
                    }
                }
            }
        }
    }
    #[inline(always)]
    fn visit_mine_boxes(&self, g: [i32; 3], mut f: impl FnMut(&MineBox)) {
        mine_features(g, |axis, n| {
            let bx = match self.mines.as_ref().and_then(|t| t.get(n)) {
                Some(node) => match axis {
                    Some(axis) => node.edges[axis],
                    None => node.room,
                },
                None => match axis {
                    Some(axis) => self.gen.mine_edge_at(axis, n).map(|b| MineBox::new(&b)),
                    None => self.gen.mine_room_at(n).map(|b| MineBox::new(&b)),
                },
            };
            if let Some(bx) = bx {
                f(&bx);
            }
        });
    }
    #[inline(always)]
    fn gradients(&self, field: usize, c: [i32; 3]) -> CellGradients {
        match self.gradients.as_ref().and_then(|t| t[field].get(c)) {
            Some(g) => *g,
            None => cell_gradients(self.gen.noise_seed(field), c[0], c[1], c[2]),
        }
    }
}

/// Edge length of a [`Block`] (u).
const BLOCK_SIZE: f32 = 8.0;
/// Safety margin (u) of the "feature cannot matter" bounds used by [`Block`].
/// Much larger than any float rounding in the fields.
const PRUNE_MARGIN: f32 = 0.01;

/// Per-cell candidate lists of one field inside a block: the features the
/// field would visit from that cell, in visiting order, minus those that
/// provably cannot change the field anywhere in the block.
type CellLists<T> = Vec<([i32; 3], Vec<T>)>;

/// The features that matter inside one [`BLOCK_SIZE`] cube.
///
/// A field starts at its cap and only visits features closer than its
/// current best (sky, pillars, rocks), or takes the max over boxes starting
/// at `-MINE_CAP` (mines). A feature whose distance bound over the whole
/// block is at least the cap (plus [`PRUNE_MARGIN`]) is therefore skipped by
/// the field at every point of the block, whatever else it visited, so
/// leaving it out keeps every sample bit-identical.
#[derive(Debug, Clone, Default)]
struct Block {
    /// Lower bound of `Generator::room_open` over the block (minus the
    /// margin; `-inf` unless the block is inside the room): a terrain value
    /// below it is the sample, without evaluating the walls.
    room_floor: f32,
    /// Per noise field: the lattice cell and its 8 corner gradients (corner
    /// `x + 2 y + 4 z`), when the whole block lies in one lattice cell.
    noise_cells: [Option<([i32; 3], CellGradients)>; 3],
    islands: CellLists<IslandShape>,
    rocks: CellLists<Rock>,
    pillars: CellLists<(Pillar, Knobs)>,
    mines: CellLists<MineBox>,
}

/// Distance from `p` to the box `b` (0 inside).
#[inline]
fn box_distance(b: &Aabb, p: Vec3) -> f32 {
    b.closest_point(p).distance(p)
}

/// Distance between two boxes (0 if they overlap).
#[inline]
fn box_gap(a: &Aabb, b: &Aabb) -> f32 {
    let gap = (a.min - b.max).max(b.min - a.max).max(Vec3::ZERO);
    gap.length()
}

/// All cells (of side `cell`, offset by `offset`) the box `b` touches.
fn cells_touching(b: &Aabb, cell: f32, offset: Vec3) -> Vec<[i32; 3]> {
    cells_touching3(b, Vec3::splat(cell), offset)
}

/// All cells (of size `cell` per axis, offset by `offset`) the box `b` touches.
fn cells_touching3(b: &Aabb, cell: Vec3, offset: Vec3) -> Vec<[i32; 3]> {
    let lo = cell_of3(b.min - offset, cell);
    let hi = cell_of3(b.max - offset, cell);
    let mut out = Vec::new();
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                out.push([x, y, z]);
            }
        }
    }
    out
}

/// Largest distance from a point of box `b` to `p` (at a corner of `b`).
#[inline]
fn box_far(b: &Aabb, p: Vec3) -> f32 {
    let d = (p - b.min).abs().max((p - b.max).abs());
    d.length()
}

/// Largest distance from a point of box `b` to the box `o` (distance to a
/// convex set is convex, so the maximum is at a corner of `b`).
fn box_far_to_box(b: &Aabb, o: &Aabb) -> f32 {
    let mut far = 0.0f32;
    for i in 0..8 {
        let c = Vec3::new(
            if i & 1 == 0 { b.min.x } else { b.max.x },
            if i & 2 == 0 { b.min.y } else { b.max.y },
            if i & 4 == 0 { b.min.z } else { b.max.z },
        );
        far = far.max(box_distance(o, c));
    }
    far
}

/// Keeps the candidates `(item, lower, upper)` of a min-field (bounds of the
/// feature's value over the block; `lower` must also bound the field's skip
/// test) that can be the minimum somewhere in the block: those whose lower
/// bound is below both the cap and every other candidate's upper bound.
fn keep_min_candidates<T>(cands: Vec<(T, f32, f32)>, cap: f32) -> Vec<T> {
    let limit = cands.iter().fold(cap, |m, c| m.min(c.2)) + PRUNE_MARGIN;
    cands
        .into_iter()
        .filter(|c| c.1 < limit)
        .map(|c| c.0)
        .collect()
}

impl Block {
    fn new(t: &Tables, b: Aabb) -> Self {
        let gen = t.gen;
        let mut block = Block {
            room_floor: f32::NEG_INFINITY,
            ..Block::default()
        };
        let room = gen.room;
        if room.contains(b.min) && room.contains(b.max) {
            let lo = b.min - room.min;
            let hi = room.max - b.max;
            block.room_floor = lo.min(hi).min_element() - PRUNE_MARGIN;
        }
        if gen.w_cave > 0.0 {
            let freqs = [gen.chamber_freq, [TUNNEL_FREQ; 3], [TUNNEL_FREQ; 3]];
            for (field, freq) in freqs.into_iter().enumerate() {
                // The noise functions scale p per axis, then floor.
                let cell = |p: Vec3| {
                    [
                        det::floor(p.x * freq[0]) as i32,
                        det::floor(p.y * freq[1]) as i32,
                        det::floor(p.z * freq[2]) as i32,
                    ]
                };
                let c = cell(b.min);
                if c == cell(b.max) {
                    block.noise_cells[field] = Some((c, t.gradients(field, c)));
                }
            }
        }
        if gen.w_sky > 0.0 {
            // Islands: only the cap bound (the skip test is not a strict
            // lower bound of the island SDF, so visiting order matters).
            for c in cells_touching3(&b, gen.island_cell, Vec3::ZERO) {
                let mut v = Vec::new();
                t.visit_islands(c, |isl, sats| {
                    if box_distance(&b, isl.center) - isl.reach < SKY_CAP + PRUNE_MARGIN {
                        v.push((*isl, sats.copied().unwrap_or_else(|| isl.satellites())));
                    }
                });
                block.islands.push((c, v));
            }
        }
        if gen.w_mine > 0.0 {
            // max(-MINE_CAP, -sdf of every box): a box that is further from
            // every point of the block than the floor, or than the far side
            // of another box, can never be the maximum.
            for g in cells_touching(&b, MINE_SPACING, gen.mine_offset) {
                let mut cands = Vec::new();
                t.visit_mine_boxes(g, |bx| cands.push((*bx, box_gap(&b, &bx.aabb))));
                let limit = cands
                    .iter()
                    .fold(MINE_CAP, |m, (bx, _)| m.min(box_far_to_box(&b, &bx.aabb)))
                    + PRUNE_MARGIN;
                let v = cands
                    .into_iter()
                    .filter(|(_, gap)| *gap < limit)
                    .map(|(bx, _)| bx)
                    .collect();
                block.mines.push((g, v));
            }
        }
        if gen.density > 0.0 {
            let flat = Aabb::new(
                Vec3::new(b.min.x, 0.0, b.min.z),
                Vec3::new(b.max.x, 0.0, b.max.z),
            );
            let cx = (
                floor_i32(b.min.x / PILLAR_CELL),
                floor_i32(b.max.x / PILLAR_CELL),
            );
            let cz = (
                floor_i32(b.min.z / PILLAR_CELL),
                floor_i32(b.max.z / PILLAR_CELL),
            );
            for z in cz.0..=cz.1 {
                for x in cx.0..=cx.1 {
                    // Pillar SDF >= dxz - reach (the skip test), <= dxz - r.
                    let mut cands = Vec::new();
                    t.visit_pillars(x, z, |pil, knobs| {
                        let axis = Vec3::new(pil.px, 0.0, pil.pz);
                        let knobs = knobs.copied().unwrap_or_else(|| gen.pillar_knobs_of(pil));
                        let lower = box_distance(&flat, axis) - pil.reach;
                        let upper = box_far(&flat, axis) - pil.r;
                        cands.push(((*pil, knobs), lower, upper));
                    });
                    block
                        .pillars
                        .push(([x, 0, z], keep_min_candidates(cands, PILLAR_CAP)));
                }
            }
            for c in cells_touching(&b, ROCK_CELL, Vec3::ZERO) {
                // Rock SDF = max(dist - r, octa) >= dist - r (the skip test)
                // and <= dist - 0.779 r (octa <= dist - 1.35 r / sqrt 3).
                let mut cands = Vec::new();
                t.visit_rocks(c, |rock| {
                    let lower = box_distance(&b, rock.center) - rock.r;
                    let upper = box_far(&b, rock.center) - 0.77 * rock.r;
                    cands.push((*rock, lower, upper));
                });
                block.rocks.push((c, keep_min_candidates(cands, ROCK_CAP)));
            }
        }
        block
    }
}

/// A mine box with its centre and half extents precomputed exactly as
/// [`Aabb::sdf`] computes them, so [`MineBox::sdf`] returns the same bits.
#[derive(Debug, Clone, Copy)]
struct MineBox {
    center: Vec3,
    half: Vec3,
    aabb: Aabb,
}

impl MineBox {
    fn new(b: &Aabb) -> Self {
        MineBox {
            center: b.center(),
            half: b.half_extents(),
            aabb: *b,
        }
    }

    /// `best.max(-self.aabb.sdf(p))`, bit for bit (the same operations as
    /// [`Aabb::sdf`]), skipping the square root when the box is provably
    /// no closer than `-best`: outside the box its SDF is at least the
    /// largest per-axis gap `m` (`sqrt(fl(m * m)) = m` and every rounding
    /// is monotone), so `-sdf <= -m <= best` and the max is `best`.
    #[inline(always)]
    fn max_neg_sdf(&self, p: Vec3, best: f32) -> f32 {
        let q = (p - self.center).abs() - self.half;
        let m = q.max_element();
        if m > 0.0 && -m <= best {
            return best;
        }
        let outside = q.max(Vec3::ZERO).length();
        let inside = m.min(0.0);
        best.max(-(outside + inside))
    }
}

#[inline(always)]
fn find<T>(lists: &CellLists<T>, c: [i32; 3]) -> Option<&[T]> {
    lists
        .iter()
        .find(|(k, _)| *k == c)
        .map(|(_, v)| v.as_slice())
}

/// A [`Block`] with the region tables as fallback (for cells it does not
/// cover, which only happens for points outside the block).
struct BlockView<'b, 'a> {
    block: &'b Block,
    tables: &'b Tables<'a>,
}

/// `terrain.min(room_open(p)) + 0.0` for a point of `block`, skipping the
/// walls when the terrain value is below the block's room floor (then the
/// minimum is the terrain value either way).
#[inline(always)]
fn with_room(gen: &Generator, block: &Block, terrain: f32, p: Vec3) -> f32 {
    if terrain < block.room_floor {
        terrain + 0.0
    } else {
        terrain.min(gen.room_open(p)) + 0.0
    }
}

/// The cell of a point in the block: known without computing it when the
/// block lies in a single cell of the field (every point of the block is
/// then in that cell).
#[inline(always)]
fn single_cell<T>(lists: &CellLists<T>, compute: impl FnOnce() -> [i32; 3]) -> [i32; 3] {
    match lists.as_slice() {
        [(c, _)] => *c,
        _ => compute(),
    }
}

impl Features for BlockView<'_, '_> {
    #[inline(always)]
    fn generator(&self) -> &Generator {
        self.tables.gen
    }
    #[inline(always)]
    fn island_cell(&self, p: Vec3) -> [i32; 3] {
        single_cell(&self.block.islands, || {
            cell_of3(p, self.tables.gen.island_cell)
        })
    }
    #[inline(always)]
    fn rock_cell(&self, p: Vec3) -> [i32; 3] {
        single_cell(&self.block.rocks, || cell_of(p, ROCK_CELL))
    }
    #[inline(always)]
    fn pillar_column(&self, p: Vec3) -> [i32; 3] {
        single_cell(&self.block.pillars, || {
            [
                floor_i32(p.x / PILLAR_CELL),
                0,
                floor_i32(p.z / PILLAR_CELL),
            ]
        })
    }
    #[inline(always)]
    fn mine_cell(&self, p: Vec3, offset: Vec3) -> [i32; 3] {
        single_cell(&self.block.mines, || cell_of(p - offset, MINE_SPACING))
    }
    #[inline(always)]
    fn visit_islands(&self, c: [i32; 3], mut f: impl FnMut(&Island, Option<&[Vec3; 2]>)) {
        match find(&self.block.islands, c) {
            Some(list) => list.iter().for_each(|(isl, sats)| f(isl, Some(sats))),
            None => self.tables.visit_islands(c, f),
        }
    }
    #[inline(always)]
    fn visit_rocks(&self, c: [i32; 3], mut f: impl FnMut(&Rock)) {
        match find(&self.block.rocks, c) {
            Some(list) => list.iter().for_each(&mut f),
            None => self.tables.visit_rocks(c, f),
        }
    }
    #[inline(always)]
    fn visit_pillars(&self, x: i32, z: i32, mut f: impl FnMut(&Pillar, Option<&Knobs>)) {
        match find(&self.block.pillars, [x, 0, z]) {
            Some(list) => list.iter().for_each(|(p, k)| f(p, Some(k))),
            None => self.tables.visit_pillars(x, z, f),
        }
    }
    #[inline(always)]
    fn visit_mine_boxes(&self, g: [i32; 3], mut f: impl FnMut(&MineBox)) {
        match find(&self.block.mines, g) {
            Some(list) => list.iter().for_each(&mut f),
            None => self.tables.visit_mine_boxes(g, f),
        }
    }
    #[inline(always)]
    fn gradients(&self, field: usize, c: [i32; 3]) -> CellGradients {
        match &self.block.noise_cells[field] {
            Some((cell, grads)) if *cell == c => *grads,
            _ => self.tables.gradients(field, c),
        }
    }
}

/// [`Generator::sample`] for many points in one region, bit for bit, but
/// much cheaper per point:
///
/// - the jittered features and noise lattice gradients around the region
///   are hashed once into tables;
/// - the region is split into [`BLOCK_SIZE`] cubes (built lazily), each with
///   short per-cell candidate lists that leave out features too far away to
///   change any sample in the cube (see [`Block`]).
///
/// Points outside the region are still correct (they fall back to the
/// tables, then to hashing), just slower.
#[derive(Debug)]
pub struct RegionSampler<'a> {
    tables: Tables<'a>,
    /// Minimum corner of block `(0, 0, 0)`.
    base: Vec3,
    blocks_per_axis: [usize; 3],
    blocks: Vec<OnceCell<Block>>,
}

impl<'a> RegionSampler<'a> {
    fn new(gen: &'a Generator, region: Aabb, blocks: bool) -> Self {
        let base = Vec3::new(
            det::floor(region.min.x),
            det::floor(region.min.y),
            det::floor(region.min.z),
        );
        let count = |a: usize| {
            let span = region.max[a] - base[a];
            (floor_i32(span / BLOCK_SIZE) + 1).clamp(1, 64) as usize
        };
        let blocks_per_axis = if blocks {
            [count(0), count(1), count(2)]
        } else {
            [0; 3]
        };
        let n = blocks_per_axis[0] * blocks_per_axis[1] * blocks_per_axis[2];
        RegionSampler {
            tables: Tables::new(gen, region),
            base,
            blocks_per_axis,
            blocks: (0..n).map(|_| OnceCell::new()).collect(),
        }
    }

    /// The generator this sampler reads.
    pub fn generator(&self) -> &'a Generator {
        self.tables.gen
    }

    /// Same as [`Generator::sample`], bit for bit.
    #[inline]
    pub fn sample(&self, p: Vec3) -> f32 {
        let gen = self.tables.gen;
        match self.block(p) {
            Some(block) => {
                let view = BlockView {
                    block,
                    tables: &self.tables,
                };
                let t = gen.terrain_with(&view, p);
                with_room(gen, block, t, p)
            }
            None => gen.terrain_with(&self.tables, p).min(gen.room_open(p)) + 0.0,
        }
    }

    /// Same as [`Generator::terrain`], bit for bit.
    #[inline]
    pub fn terrain(&self, p: Vec3) -> f32 {
        let gen = self.tables.gen;
        match self.block(p) {
            Some(block) => {
                let view = BlockView {
                    block,
                    tables: &self.tables,
                };
                gen.terrain_with(&view, p)
            }
            None => gen.terrain_with(&self.tables, p),
        }
    }

    /// Samples the points `(xs[i], y, z)` into `out` (same length), with
    /// the same bits as [`RegionSampler::sample`] but cheaper: along a row
    /// the noise lattice cell rarely changes, so the caverns noise is
    /// evaluated with per-cell constants ([`gradient_noise_row`]).
    pub fn sample_row(&self, y: f32, z: f32, xs: &[f32], out: &mut [f32]) {
        assert_eq!(xs.len(), out.len());
        let gen = self.tables.gen;
        const CHUNK: usize = 64;
        let mut noise = [[0.0f32; CHUNK]; 3];
        for (xs, out) in xs.chunks(CHUNK).zip(out.chunks_mut(CHUNK)) {
            let n = xs.len();
            if gen.w_cave > 0.0 {
                let grad = |field: usize| move |x, y, z| self.tables.gradients(field, [x, y, z]);
                let mut cx = [0.0f32; CHUNK];
                let mut tx = [0.0f32; CHUNK];
                for (i, x) in xs.iter().enumerate() {
                    cx[i] = x * gen.chamber_freq[0];
                    tx[i] = x * TUNNEL_FREQ;
                }
                let (cy, cz) = (y * gen.chamber_freq[1], z * gen.chamber_freq[2]);
                let (ty, tz) = (y * TUNNEL_FREQ, z * TUNNEL_FREQ);
                gradient_noise_row(&cx[..n], cy, cz, grad(NOISE_CHAMBER), &mut noise[0][..n]);
                gradient_noise_row(&tx[..n], ty, tz, grad(NOISE_TUNNEL_A), &mut noise[1][..n]);
                gradient_noise_row(&tx[..n], ty, tz, grad(NOISE_TUNNEL_B), &mut noise[2][..n]);
            }
            for (i, (x, o)) in xs.iter().zip(out.iter_mut()).enumerate() {
                let p = Vec3::new(*x, y, z);
                let cave =
                    || caverns_from_noise(gen.chamber_scale, noise[0][i], noise[1][i], noise[2][i]);
                *o = match self.block(p) {
                    Some(block) => {
                        let view = BlockView {
                            block,
                            tables: &self.tables,
                        };
                        let t = gen.terrain_parts(&view, p, cave);
                        with_room(gen, block, t, p)
                    }
                    None => {
                        gen.terrain_parts(&self.tables, p, cave)
                            .min(gen.room_open(p))
                            + 0.0
                    }
                };
            }
        }
    }

    /// The (lazily built) block containing `p`, if `p` is in the region.
    #[inline(always)]
    fn block(&self, p: Vec3) -> Option<&Block> {
        let (idx, b) = self.block_index(p)?;
        Some(self.blocks[idx].get_or_init(|| {
            let min = self.base + Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32) * BLOCK_SIZE;
            Block::new(&self.tables, Aabb::new(min, min + Vec3::splat(BLOCK_SIZE)))
        }))
    }

    /// Block containing `p` (its index and coordinates), if any.
    #[inline(always)]
    fn block_index(&self, p: Vec3) -> Option<(usize, [usize; 3])> {
        let mut b = [0usize; 3];
        for a in 0..3 {
            let i = floor_i32((p[a] - self.base[a]) / BLOCK_SIZE);
            if i < 0 || i as usize >= self.blocks_per_axis[a] {
                return None;
            }
            b[a] = i as usize;
        }
        let [nx, ny, _] = self.blocks_per_axis;
        Some((b[0] + nx * (b[1] + ny * b[2]), b))
    }
}

/// Integer grid cell containing `p` for a cell of size `cell` per axis.
#[inline(always)]
fn cell_of3(p: Vec3, cell: Vec3) -> [i32; 3] {
    [
        floor_i32(p.x / cell.x),
        floor_i32(p.y / cell.y),
        floor_i32(p.z / cell.z),
    ]
}

/// Integer grid cell containing `p` for a cubic cell of side `cell`.
#[inline(always)]
fn cell_of(p: Vec3, cell: f32) -> [i32; 3] {
    [
        floor_i32(p.x / cell),
        floor_i32(p.y / cell),
        floor_i32(p.z / cell),
    ]
}

/// `det::floor(x) as i32` without the libm call: the same result for every
/// input (saturating at the `i32` range, 0 for NaN).
#[inline(always)]
pub(crate) fn floor_i32(x: f32) -> i32 {
    let i = x as i32;
    if x < i as f32 {
        i.saturating_sub(1)
    } else {
        i
    }
}

/// Cell centre plus a hashed offset of at most `jitter` per axis.
#[inline]
fn jittered_center(c: [i32; 3], cell: Vec3, jitter: Vec3, h: u64) -> Vec3 {
    let h2 = mix64(h ^ 0x6a09_e667_f3bc_c909);
    let j = Vec3::new(unit_f32(h2), unit_f32_b(h2), unit_f32(mix64(h2))) - Vec3::splat(0.5);
    Vec3::new(
        (c[0] as f32 + 0.5) * cell.x + j.x * (2.0 * jitter.x),
        (c[1] as f32 + 0.5) * cell.y + j.y * (2.0 * jitter.y),
        (c[2] as f32 + 0.5) * cell.z + j.z * (2.0 * jitter.z),
    )
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
    fn render_and_sim_sliders_do_not_change_terrain() {
        let a = Generator::new(9, Sliders::new(40, 60));
        let mut s = Sliders::new(40, 60);
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

    fn with(e: u8, d: u8, v: u8, g: u8) -> Sliders {
        Sliders {
            enclosure: e,
            density: d,
            verticality: v,
            gravity_chaos: g,
            ..Sliders::default()
        }
    }

    /// Verticality picks one of five level boxes of about the same chunk
    /// count, and stretches the inner structure with it.
    #[test]
    fn verticality_shapes_the_box_and_structure() {
        let counts: Vec<i32> = BOX_SHAPES.iter().map(|s| s[0] * s[1] * s[2]).collect();
        assert!(counts.iter().all(|c| (200..=256).contains(c)), "{counts:?}");
        for (v, shape) in [
            (0u8, [10, 2, 10]),
            (19, [10, 2, 10]),
            (20, [9, 3, 9]),
            (50, [8, 4, 8]),
            (60, [7, 5, 7]),
            (79, [7, 5, 7]),
            (80, [6, 7, 6]),
            (100, [6, 7, 6]),
        ] {
            let g = Generator::new(3, with(50, 40, v, 0));
            assert_eq!(g.size_chunks(), shape, "verticality {v}");
            assert_eq!(box_shape(v), shape);
            let size = g.bounds().size();
            assert_eq!(size.x, shape[0] as f32 * 32.0);
            assert_eq!(size.y, shape[1] as f32 * 32.0);
        }
        let flat = Structure::for_verticality(0);
        let mid = Structure::for_verticality(50);
        let tall = Structure::for_verticality(100);
        // Flat: wide, low chambers; tall: chimneys.
        assert!(flat.chamber_freq[1] > mid.chamber_freq[1]);
        assert!(flat.chamber_freq[0] < mid.chamber_freq[0]);
        assert!(tall.chamber_freq[1] < mid.chamber_freq[1]);
        assert!(tall.chamber_freq[0] > mid.chamber_freq[0]);
        assert_eq!(flat.mine_edge_v, 0.05);
        assert_eq!(tall.mine_edge_v, 0.6);
        // Tiers in tall boxes.
        assert_eq!(tall.island_cell.y, ISLAND_CELL_Y_TIERS);
        assert!(tall.rock_jitter.y < mid.rock_jitter.y);
        // The caverns field stays 1-Lipschitz in every shape (checked in
        // sdf_is_one_lipschitz_in_every_shape).
    }

    #[test]
    fn sdf_is_one_lipschitz_in_every_shape() {
        let mut rng = SplitMix64::new(78);
        for (seed, e, d, v, gc) in [
            (1u64, 0u8, 60u8, 0u8, 100u8),
            (2, 50, 50, 10, 50),
            (3, 50, 30, 90, 80),
            (4, 75, 20, 100, 30),
            (5, 100, 70, 65, 100),
            (6, 25, 80, 30, 0),
        ] {
            let g = Generator::new(seed, with(e, d, v, gc));
            let size = g.bounds().size();
            let mut worst = 0.0f32;
            for _ in 0..20_000 {
                let p = Vec3::new(
                    rng.next_f32() * size.x,
                    rng.next_f32() * size.y,
                    rng.next_f32() * size.z,
                );
                let dir = Vec3::new(
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                )
                .normalize();
                let q = p + dir * (0.05 + rng.next_f32() * 3.0);
                worst = worst.max((g.sample(q) - g.sample(p)).abs() / p.distance(q));
            }
            assert!(worst <= 1.0 + 1e-3, "e {e} d {d} v {v}: {worst}");
        }
    }

    /// Gravity Chaos places 1 + slider / 15 wells (none at 0), 30 u apart,
    /// 40 u clear of the spawn caverns, each a rock core in an open hollow.
    #[test]
    fn gravity_chaos_places_wells() {
        assert!(Generator::new(1, with(50, 40, 50, 0))
            .gravity_wells()
            .is_empty());
        for (seed, e, v, gc) in [
            (1u64, 50u8, 50u8, 100u8),
            (2, 10, 50, 60),
            (3, 100, 0, 100),
            (4, 50, 100, 100),
            (5, 30, 30, 15),
        ] {
            let g = Generator::new(seed, with(e, 40, v, gc));
            let wells = g.gravity_wells();
            let want = crate::wells::well_count(gc);
            assert!(!wells.is_empty(), "seed {seed}");
            assert!(wells.len() <= want);
            let net = g.network();
            for (i, w) in wells.iter().enumerate() {
                assert!(g.sample(w.pos) < 0.0, "{seed}: core is rock");
                // Open just outside the core, around it.
                let ring = w.pos + Vec3::X * (w.core_radius + 2.0);
                assert!(g.sample(ring) > 0.5, "{seed}: hollow at {ring:?}");
                for h in (0..4u8).flat_map(|t| net.team_hubs(t)) {
                    assert!(h.distance(w.pos) >= 40.0 + 9.0, "{seed}: near spawn {h:?}");
                }
                for o in &wells[i + 1..] {
                    assert!(o.pos.distance(w.pos) >= 30.0);
                }
                assert_eq!(w.strength, gc as f32 / 100.0);
                let floating = e < crate::wells::FLOATING_ENCLOSURE;
                assert_eq!(w.kind == ls_math::level::WellKind::Floating, floating);
            }
        }
        // Full count where there is room (default box, caverns).
        let g = Generator::new(42, with(50, 40, 50, 100));
        assert_eq!(g.gravity_wells().len(), 7);
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

    #[test]
    fn floor_i32_matches_libm_floor() {
        let mut rng = SplitMix64::new(4);
        let special = [
            0.0f32,
            -0.0,
            0.5,
            -0.5,
            -1.0,
            1.0,
            -1e-30,
            16_777_215.5,
            -16_777_215.5,
            2_147_483_520.0,
            2_147_483_648.0,
            -2_147_483_648.0,
            -2_147_483_904.0,
            1e20,
            -1e20,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        let random = (0..10_000).map(|_| (rng.next_f32() - 0.5) * 2000.0);
        for x in special.into_iter().chain(random) {
            assert_eq!(floor_i32(x), det::floor(x) as i32, "{x}");
        }
    }

    /// The region sampler (tables plus pruned per-block candidate lists)
    /// returns the generator's bits, inside its region and outside.
    #[test]
    fn region_sampler_is_bit_identical() {
        let mut rng = SplitMix64::new(8);
        let mut checked = 0;
        for (i, (e, d)) in [
            (0u8, 0u8),
            (0, 100),
            (25, 50),
            (50, 60),
            (75, 20),
            (100, 0),
            (100, 100),
            (40, 30),
        ]
        .into_iter()
        .enumerate()
        {
            let g = Generator::new(1000 + i as u64, Sliders::new(e, d));
            for _ in 0..4 {
                let min = Vec3::new(
                    (rng.next_u32() % 8) as f32 * 32.0 - 2.0,
                    (rng.next_u32() % 4) as f32 * 32.0 - 2.0,
                    (rng.next_u32() % 8) as f32 * 32.0 - 2.0,
                );
                let region = Aabb::new(min, min + Vec3::splat(36.0));
                let s = g.region_sampler(region);
                for n in 0..3000 {
                    // Mostly inside, some just outside the region.
                    let p = min + Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 40.0
                        - Vec3::splat(2.0);
                    let p = if n % 3 == 0 {
                        Vec3::new(
                            ls_math::det::floor(p.x),
                            ls_math::det::floor(p.y),
                            ls_math::det::floor(p.z),
                        )
                    } else {
                        p
                    };
                    assert_eq!(
                        s.sample(p).to_bits(),
                        g.sample(p).to_bits(),
                        "enclosure {e} density {d} at {p:?}"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 8 * 4 * 3000);
    }

    /// The region, row and sparse samplers return the generator's bits on
    /// version 4 levels too: every level shape, gravity wells and belts.
    #[test]
    fn samplers_are_bit_identical_on_shapes_wells_and_belts() {
        let mut rng = SplitMix64::new(81);
        for (i, (archetype, s)) in [
            (Archetype::Classic, with(50, 60, 5, 100)),
            (Archetype::Classic, with(20, 40, 95, 100)),
            (Archetype::Classic, with(80, 30, 70, 60)),
            (Archetype::Belt, with(60, 70, 50, 50)),
            (Archetype::Belt, with(100, 100, 100, 0)),
        ]
        .into_iter()
        .enumerate()
        {
            let g = Generator::with_archetype(500 + i as u64, s, archetype);
            // Regions around every well, the belt's slot 0 and the centre.
            let mut centres: Vec<Vec3> = g.gravity_wells().iter().map(|w| w.pos).collect();
            centres.extend(g.belt().map(|b| b.slots[0].pos));
            centres.push(g.room().center());
            for c in centres {
                let min = c - Vec3::splat(18.0);
                let region = Aabb::new(min, min + Vec3::splat(36.0));
                let dense = g.region_sampler(region);
                let sparse = g.sparse_region_sampler(region);
                for _ in 0..1500 {
                    let p = min + Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 40.0
                        - Vec3::splat(2.0);
                    let want = g.sample(p).to_bits();
                    assert_eq!(dense.sample(p).to_bits(), want, "{i} at {p:?}");
                    assert_eq!(sparse.sample(p).to_bits(), want, "{i} sparse at {p:?}");
                }
                let y = c.y + 0.5;
                let xs: Vec<f32> = (0..40).map(|k| min.x + k as f32 * 0.9).collect();
                let mut out = vec![0.0; xs.len()];
                dense.sample_row(y, c.z, &xs, &mut out);
                for (x, v) in xs.iter().zip(&out) {
                    assert_eq!(v.to_bits(), g.sample(Vec3::new(*x, y, c.z)).to_bits());
                }
            }
        }
    }

    /// Row sampling (per-cell noise constants) and the sparse sampler return
    /// the generator's bits too.
    #[test]
    fn row_and_sparse_sampling_are_bit_identical() {
        let mut rng = SplitMix64::new(12);
        for (i, (e, d)) in [(50u8, 60u8), (25, 0), (75, 20), (100, 40), (0, 100)]
            .into_iter()
            .enumerate()
        {
            let g = Generator::new(77 + i as u64, Sliders::new(e, d));
            let min = Vec3::new(62.0, 30.0, 94.0);
            let region = Aabb::new(min, min + Vec3::splat(36.0));
            let dense = g.region_sampler(region);
            let sparse = g.sparse_region_sampler(region);
            for r in 0..200 {
                // Integer rows (chunk corners) and fractional ones.
                let frac = if r % 2 == 0 { 0.0 } else { rng.next_f32() };
                let y = min.y + (rng.next_u32() % 36) as f32 + frac;
                let z = min.z + (rng.next_u32() % 36) as f32 + frac;
                let x0 = min.x - 3.0 + frac;
                let xs: Vec<f32> = (0..45)
                    .map(|k| x0 + k as f32 * (1.0 - frac * 0.5))
                    .collect();
                let mut out = vec![0.0; xs.len()];
                dense.sample_row(y, z, &xs, &mut out);
                let mut out2 = vec![0.0; xs.len()];
                sparse.sample_row(y, z, &xs, &mut out2);
                for ((x, a), b) in xs.iter().zip(&out).zip(&out2) {
                    let p = Vec3::new(*x, y, z);
                    let want = g.sample(p).to_bits();
                    assert_eq!(a.to_bits(), want, "row, enclosure {e} density {d} at {p:?}");
                    assert_eq!(b.to_bits(), want, "sparse row at {p:?}");
                    assert_eq!(sparse.sample(p).to_bits(), want, "sparse at {p:?}");
                }
            }
        }
    }
}
