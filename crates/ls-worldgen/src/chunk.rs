//! The chunk store: lazily generated SDF chunks with the terrain edit log
//! applied on top, a per-cell paint layer, and the physics queries.
//!
//! # The world SDF
//!
//! The terrain SDF at `p` is
//!
//! ```text
//! clamp(min(edits(generator.terrain(p)), room(p)), -SDF_CLAMP, SDF_CLAMP)
//! ```
//!
//! where `edits` applies the logged geometry edits in order (reference CSG
//! from [`TerrainEdit::apply_to_sdf`]) and `room` is the sealing wall, so
//! craters can never open the level box. The clamp makes edits local: an
//! edit further than `SDF_CLAMP` from `p` cannot change the clamped value
//! (min/max/clamp only *select* values), so each chunk only needs the edits
//! near it, and the result is bit-identical whichever chunk evaluates it.
//! Points outside the level box skip the edits (they are solid either way),
//! which keeps the rule purely position-based.
//!
//! Chunks store the samples at their corners `0..=32` (33³ values), so every
//! cell's surface state is known locally. Sampling, incremental edits and
//! fresh evaluation give the same bits (see the tests), which is what keeps
//! peers that replay the same edit log in sync.
//!
//! # Cost
//!
//! - Edits are baked into the stored samples: an edit is applied once to the
//!   corners within its reach when it arrives (generated chunks) or when the
//!   chunk is generated (pristine samples first, then every registered edit
//!   in log order). Remeshing reads stored samples only, so its cost does not
//!   depend on the edit history.
//! - `eval` (physics, arbitrary points) still composes edits per point, but
//!   only those registered with the point's 8 u edit bin.
//! - Pristine samples come from a [`RegionSampler`] (feature tables plus
//!   pruned per-block candidate lists, noise rows), bit-identical to
//!   [`Generator::sample`].

use std::collections::BTreeMap;

use ls_math::terrain::{TerrainEdit, TerrainQuery};
use ls_math::{det, Aabb, Vec3};
use serde::{Deserialize, Serialize};

use crate::generator::floor_i32;
use crate::grid::SampleGrid;
use crate::mesh::{dual_contouring_with_sdf, surface_nets, Mesh};
use crate::{Generator, RegionSampler, CHUNK_CELLS, NO_TEAM};

/// Magnitude at which stored SDF values are clamped (u).
pub const SDF_CLAMP: f32 = 6.0;
/// Corners stored per axis in a chunk (`0..=32`).
pub const CHUNK_CORNERS: usize = CHUNK_CELLS as usize + 1;
/// A geometry edit can change the mesh only within this distance of its
/// bounds (two cell diagonals; see `apply`).
pub const DIRTY_MARGIN: f32 = 3.5;
/// Uniform-chunk test: the padded corner range `-1..=32` is split into 3³
/// blocks of 11 u; a block whose centre is further than its half-diagonal
/// (5.5 * sqrt(3) < 9.6) beyond the clamp has all corners at the clamp value.
const UNIFORM_BLOCK_RADIUS: f32 = 9.6;
const UNIFORM_BLOCK_CENTERS: [f32; 3] = [4.5, 15.5, 26.5];

/// Integer chunk coordinates (chunk `c` covers cells `32c .. 32c + 32`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        ChunkCoord { x, y, z }
    }

    /// World position of the chunk's corner `(0, 0, 0)`.
    pub fn origin(self) -> [i32; 3] {
        [
            self.x * CHUNK_CELLS,
            self.y * CHUNK_CELLS,
            self.z * CHUNK_CELLS,
        ]
    }

    /// World box covered by the chunk's cells.
    pub fn aabb(self) -> Aabb {
        let o = self.origin();
        let min = Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32);
        Aabb::new(min, min + Vec3::splat(CHUNK_CELLS as f32))
    }
}

/// Per-team paint coverage of the level surface.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaintCoverage {
    /// Number of surface cells (cells the surface passes through).
    pub surface_cells: u64,
    /// Painted surface cells per team.
    pub team_cells: BTreeMap<u8, u64>,
}

impl PaintCoverage {
    /// Fraction of the surface painted by `team` (0 if there is no surface).
    pub fn fraction(&self, team: u8) -> f32 {
        if self.surface_cells == 0 {
            return 0.0;
        }
        let cells = self.team_cells.get(&team).copied().unwrap_or(0);
        (cells as f64 / self.surface_cells as f64) as f32
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Samples {
    /// Every stored and padded corner has this value (`±SDF_CLAMP`).
    Uniform(f32),
    /// `CHUNK_CORNERS³` values, x fastest.
    Dense(Vec<f32>),
}

#[derive(Debug, Clone, PartialEq)]
struct Chunk {
    samples: Samples,
    /// Painted cells: local cell index -> team. Only surface cells are kept.
    paint: BTreeMap<u16, u8>,
    /// Number of surface cells among the chunk's own cells.
    surface_cells: u32,
}

impl Chunk {
    #[inline]
    fn value(&self, i: usize, j: usize, k: usize) -> f32 {
        match &self.samples {
            Samples::Uniform(v) => *v,
            Samples::Dense(d) => d[corner_index(i, j, k)],
        }
    }

    fn is_surface_cell(&self, i: usize, j: usize, k: usize) -> bool {
        let Samples::Dense(d) = &self.samples else {
            return false;
        };
        let first = d[corner_index(i, j, k)] < 0.0;
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    if (d[corner_index(i + dx, j + dy, k + dz)] < 0.0) != first {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Counts surface cells in the (inclusive) cell range.
    fn count_surface(&self, lo: [usize; 3], hi: [usize; 3]) -> u32 {
        let Samples::Dense(d) = &self.samples else {
            return 0;
        };
        let mut n = 0;
        // Per corner column x: how many of the four corners (x, j|j+1,
        // k|k+1) are rock. A cell is a surface cell unless its two columns
        // hold 0 or 8 rock corners together.
        let mut rock = [0u8; CHUNK_CORNERS];
        for k in lo[2]..=hi[2] {
            for j in lo[1]..=hi[1] {
                for (x, r) in rock.iter_mut().enumerate().take(hi[0] + 2).skip(lo[0]) {
                    *r = (d[corner_index(x, j, k)] < 0.0) as u8
                        + (d[corner_index(x, j + 1, k)] < 0.0) as u8
                        + (d[corner_index(x, j, k + 1)] < 0.0) as u8
                        + (d[corner_index(x, j + 1, k + 1)] < 0.0) as u8;
                }
                for i in lo[0]..=hi[0] {
                    let s = rock[i] + rock[i + 1];
                    n += (s != 0 && s != 8) as u32;
                }
            }
        }
        n
    }
}

#[inline]
fn corner_index(i: usize, j: usize, k: usize) -> usize {
    i + CHUNK_CORNERS * (j + CHUNK_CORNERS * k)
}

#[inline]
fn cell_index(i: usize, j: usize, k: usize) -> u16 {
    let n = CHUNK_CELLS as usize;
    (i + n * (j + n * k)) as u16
}

/// Bins per chunk axis for the per-point edit lists used by `eval`.
const EDIT_BINS: usize = 4;
/// Edge length of an edit bin (u).
const EDIT_BIN_SIZE: f32 = (CHUNK_CELLS as usize / EDIT_BINS) as f32;

/// The geometry edits that can affect one chunk, as indices into the log.
#[derive(Debug, Clone, Default)]
struct ChunkEdits {
    /// Every edit whose reach touches the chunk (plus its padded ring), in
    /// log order. Baked into the samples when the chunk is generated.
    all: Vec<u32>,
    /// Per bin (`EDIT_BINS³`, x fastest; empty until the first edit): the
    /// edits whose reach touches the bin, in log order, minus craters that a
    /// later crater in the same bin fully contains (see [`Crater`]). `eval`
    /// only replays the bin of the point, so its cost depends on the edits
    /// near the point, not on the chunk's whole history, and repeated
    /// craters on one spot do not pile up.
    bins: Vec<Vec<u32>>,
}

/// Margin for [`Crater::covers`]: far above the f32 rounding of distances
/// at level coordinates, so a covered crater's term is below the covering
/// one's bit for bit at every point.
const CRATER_COVER_EPS: f32 = 1e-3;

/// A `SubtractSphere` that can make earlier ones redundant in `eval`.
///
/// For craters `S1` then (later) `S2` with `s2(p) >= s1(p)` everywhere
/// (`s = r - |p - c|`), and any chain `F` of min/max edits between them,
/// `max(F(max(d, s1)), s2) == max(F(d), s2)`: `F` is monotone and
/// `F(s1) <= max(F(d), s1)`. So `S1` can leave the bins `S2` is registered
/// in without changing a single `eval` bit. The log, `ChunkEdits::all`
/// (baking) and late-join replay keep every edit.
#[derive(Debug, Clone, Copy)]
struct Crater {
    center: Vec3,
    radius: f32,
}

impl Crater {
    /// Whether `earlier` is a crater fully inside this one (with margin).
    fn covers(&self, earlier: &TerrainEdit) -> bool {
        match *earlier {
            TerrainEdit::SubtractSphere { center, radius } => {
                self.radius - radius - center.distance(self.center) > CRATER_COVER_EPS
            }
            _ => false,
        }
    }
}

fn containing_crater(edit: &TerrainEdit) -> Option<Crater> {
    match *edit {
        TerrainEdit::SubtractSphere { center, radius } => Some(Crater { center, radius }),
        _ => None,
    }
}

/// Bin of a coordinate along one axis (`v - origin`), clamped to the chunk.
#[inline]
fn bin_of(v: f32, origin: i32) -> usize {
    (det::floor((v - origin as f32) / EDIT_BIN_SIZE) as i32).clamp(0, EDIT_BINS as i32 - 1) as usize
}

/// Chunked, editable, paintable terrain for one level.
#[derive(Debug, Clone)]
pub struct ChunkStore {
    gen: Generator,
    dims: [i32; 3],
    chunks: Vec<Option<Chunk>>,
    /// The geometry edits that can affect each chunk.
    chunk_edits: Vec<ChunkEdits>,
    log: Vec<TerrainEdit>,
}

impl ChunkStore {
    /// An empty store; chunks are generated on first use.
    pub fn new(gen: Generator) -> Self {
        let dims = gen.size_chunks();
        let count = (dims[0] * dims[1] * dims[2]) as usize;
        ChunkStore {
            gen,
            dims,
            chunks: vec![None; count],
            chunk_edits: vec![ChunkEdits::default(); count],
            log: Vec::new(),
        }
    }

    /// A store with `edits` applied in order (late-join replay).
    pub fn replay(gen: Generator, edits: &[TerrainEdit]) -> Self {
        let mut s = ChunkStore::new(gen);
        for e in edits {
            s.apply(*e);
        }
        s
    }

    pub fn generator(&self) -> &Generator {
        &self.gen
    }

    /// Level size in chunks.
    pub fn dims(&self) -> [i32; 3] {
        self.dims
    }

    /// Every edit applied so far, in order (invalid edits are not logged).
    pub fn edit_log(&self) -> &[TerrainEdit] {
        &self.log
    }

    /// All chunk coordinates of the level, in ascending order.
    pub fn chunk_coords(&self) -> Vec<ChunkCoord> {
        let mut v = Vec::with_capacity(self.chunks.len());
        for x in 0..self.dims[0] {
            for y in 0..self.dims[1] {
                for z in 0..self.dims[2] {
                    v.push(ChunkCoord::new(x, y, z));
                }
            }
        }
        v
    }

    /// True if `c` is inside the level.
    pub fn contains_chunk(&self, c: ChunkCoord) -> bool {
        (0..self.dims[0]).contains(&c.x)
            && (0..self.dims[1]).contains(&c.y)
            && (0..self.dims[2]).contains(&c.z)
    }

    /// True if the chunk's samples have been generated.
    pub fn is_generated(&self, c: ChunkCoord) -> bool {
        self.chunk_slot(c).is_some_and(|i| self.chunks[i].is_some())
    }

    fn chunk_slot(&self, c: ChunkCoord) -> Option<usize> {
        self.contains_chunk(c)
            .then(|| (c.x + self.dims[0] * (c.y + self.dims[1] * c.z)) as usize)
    }

    /// The chunk containing world point `p` (by `floor(p / 32)`), if inside the level.
    pub fn chunk_at(&self, p: Vec3) -> Option<ChunkCoord> {
        let s = CHUNK_CELLS as f32;
        let c = ChunkCoord::new(floor_i32(p.x / s), floor_i32(p.y / s), floor_i32(p.z / s));
        self.contains_chunk(c).then_some(c)
    }

    /// The world SDF (see the module docs). Needs no generated chunks.
    ///
    /// Replays only the edits registered with the point's edit bin (an 8 u
    /// cube), so the cost depends on how many edits are near `p`, not on the
    /// length of the edit log.
    pub fn eval(&self, p: Vec3) -> f32 {
        self.eval_with(p, |q| self.gen.terrain(q))
    }

    /// [`ChunkStore::eval`] with the generator terrain supplied by `terrain`
    /// (which must return `Generator::terrain` bits, e.g. a
    /// [`RegionSampler`](crate::generator::RegionSampler)).
    #[inline]
    fn eval_with(&self, p: Vec3, terrain: impl Fn(Vec3) -> f32) -> f32 {
        let room = self.gen.room_open(p);
        let mut d = terrain(p);
        if let Some(c) = self.chunk_at(p) {
            let edits = &self.chunk_edits[self.chunk_slot(c).expect("in level")];
            if !edits.bins.is_empty() {
                let o = c.origin();
                let bin = bin_of(p.x, o[0])
                    + EDIT_BINS * (bin_of(p.y, o[1]) + EDIT_BINS * bin_of(p.z, o[2]));
                for &e in &edits.bins[bin] {
                    d = self.log[e as usize].apply_to_sdf(d, p);
                }
            }
        }
        d.min(room).clamp(-SDF_CLAMP, SDF_CLAMP) + 0.0
    }

    /// A generator sampler with tables covering chunk `c`'s padded corners
    /// (and the small offsets used for normals).
    fn chunk_sampler(&self, c: ChunkCoord) -> RegionSampler<'_> {
        let b = c.aabb();
        self.gen.region_sampler(Aabb::new(
            b.min - Vec3::splat(2.0),
            b.max + Vec3::splat(2.0),
        ))
    }

    /// Generates a chunk if needed: pristine samples from the generator,
    /// then every registered edit applied once to the corners it can reach.
    pub fn ensure_generated(&mut self, c: ChunkCoord) {
        let Some(slot) = self.chunk_slot(c) else {
            return;
        };
        if self.chunks[slot].is_some() {
            return;
        }
        let o = c.origin();
        let mut samples = pristine_samples(&self.chunk_sampler(c), o);
        let level = self.level_corners();
        for &e in &self.chunk_edits[slot].all {
            let edit = self.log[e as usize];
            let (lo, hi) = edit_corner_range(o, edit.bounds().expand(SDF_CLAMP));
            bake_edit(&self.gen, level, o, &mut samples, &edit, lo, hi);
        }
        let mut chunk = Chunk {
            samples,
            paint: BTreeMap::new(),
            surface_cells: 0,
        };
        let m = CHUNK_CELLS as usize - 1;
        chunk.surface_cells = chunk.count_surface([0; 3], [m; 3]);
        self.chunks[slot] = Some(chunk);
    }

    /// Level size in corners along each axis (corners `0..size` are inside).
    fn level_corners(&self) -> [i32; 3] {
        self.dims.map(|d| d * CHUNK_CELLS)
    }

    /// Generates every chunk of the level.
    pub fn generate_all(&mut self) {
        for c in self.chunk_coords() {
            self.ensure_generated(c);
        }
    }

    /// The padded sample grid (and paint) of a chunk, ready for a mesher.
    pub fn sample_grid(&mut self, c: ChunkCoord) -> SampleGrid {
        self.ensure_generated(c);
        let o = c.origin();
        let chunk = self.chunk_slot(c).and_then(|s| self.chunks[s].as_ref());
        let sampler = self.chunk_sampler(c);
        let n = CHUNK_CELLS;
        let mut values = Vec::with_capacity((n as usize + 2).pow(3));
        for k in -1..=n {
            for j in -1..=n {
                for i in -1..=n {
                    let v = match chunk {
                        Some(ch) if i >= 0 && j >= 0 && k >= 0 => {
                            ch.value(i as usize, j as usize, k as usize)
                        }
                        // Padding ring: read a generated neighbour (its corner
                        // 31 is our corner -1, same bits), else evaluate.
                        _ => self.corner_value([o[0] + i, o[1] + j, o[2] + k], &sampler),
                    };
                    values.push(v);
                }
            }
        }
        let mut grid = SampleGrid::from_values(o, values);
        // Paint of own cells and of the neighbouring cells at local -1.
        for k in -1..n {
            for j in -1..n {
                for i in -1..n {
                    let t = self.team_at_cell([o[0] + i, o[1] + j, o[2] + k]);
                    if t != NO_TEAM {
                        grid.set_team(i, j, k, t);
                    }
                }
            }
        }
        grid
    }

    /// World SDF at an integer corner, from stored samples when available.
    fn corner_value(&self, g: [i32; 3], sampler: &RegionSampler) -> f32 {
        let n = CHUNK_CELLS;
        let c = ChunkCoord::new(g[0].div_euclid(n), g[1].div_euclid(n), g[2].div_euclid(n));
        if let Some(ch) = self.chunk_slot(c).and_then(|s| self.chunks[s].as_ref()) {
            let l = [g[0].rem_euclid(n), g[1].rem_euclid(n), g[2].rem_euclid(n)];
            return ch.value(l[0] as usize, l[1] as usize, l[2] as usize);
        }
        let p = Vec3::new(g[0] as f32, g[1] as f32, g[2] as f32);
        self.eval_with(p, |q| sampler.terrain(q))
    }

    /// Finds an open point near `near` with at least `clearance` of free space
    /// (`sdf >= clearance`, capped at the SDF clamp), searching a 1 u lattice
    /// in growing cubic shells up to `max_radius`. Deterministic; useful for
    /// spawn points and tests.
    pub fn find_open_point(&self, near: Vec3, clearance: f32, max_radius: i32) -> Option<Vec3> {
        let need = clearance.min(SDF_CLAMP);
        let base = Vec3::new(det::floor(near.x), det::floor(near.y), det::floor(near.z))
            + Vec3::splat(0.5);
        for r in 0..=max_radius.max(0) {
            for dz in -r..=r {
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                            continue;
                        }
                        let p = base + Vec3::new(dx as f32, dy as f32, dz as f32);
                        if self.eval(p) >= need {
                            return Some(p);
                        }
                    }
                }
            }
        }
        None
    }

    /// Surface-nets mesh of a chunk (empty for chunks outside the level).
    pub fn mesh_surface_nets(&mut self, c: ChunkCoord) -> Mesh {
        if !self.contains_chunk(c) || self.is_uniform(c) {
            return Mesh::default();
        }
        surface_nets(&self.sample_grid(c))
    }

    /// Dual-contouring mesh of a chunk (empty for chunks outside the level),
    /// with exact Hermite data from [`ChunkStore::eval`] (terrain read
    /// through a [`RegionSampler`]).
    ///
    /// Off the hot path: the game meshes with surface nets. Dual contouring
    /// needs about 14 SDF evaluations per crossing edge (8 root-refinement
    /// steps and a central-difference normal), roughly 110 000 for a busy
    /// chunk, so it stays around 10-20x slower than surface nets (several
    /// ms per chunk native release; see `examples/worldgen_bench.rs`).
    pub fn mesh_dual_contouring(&mut self, c: ChunkCoord) -> Mesh {
        if !self.contains_chunk(c) || self.is_uniform(c) {
            return Mesh::default();
        }
        let grid = self.sample_grid(c);
        let sampler = self.chunk_sampler(c);
        dual_contouring_with_sdf(&grid, &|p| self.eval_with(p, |q| sampler.terrain(q)))
    }

    /// True if the chunk (after generating it) is entirely rock or entirely open.
    fn is_uniform(&mut self, c: ChunkCoord) -> bool {
        self.ensure_generated(c);
        self.chunk_slot(c)
            .and_then(|s| self.chunks[s].as_ref())
            .is_some_and(|ch| matches!(ch.samples, Samples::Uniform(_)))
    }

    /// Paint team of the global cell `g` (cell `g` spans `g .. g + 1`).
    pub fn team_at_cell(&self, g: [i32; 3]) -> u8 {
        let n = CHUNK_CELLS;
        let c = ChunkCoord::new(g[0].div_euclid(n), g[1].div_euclid(n), g[2].div_euclid(n));
        let Some(ch) = self.chunk_slot(c).and_then(|s| self.chunks[s].as_ref()) else {
            return NO_TEAM;
        };
        let l = [g[0].rem_euclid(n), g[1].rem_euclid(n), g[2].rem_euclid(n)];
        let idx = cell_index(l[0] as usize, l[1] as usize, l[2] as usize);
        ch.paint.get(&idx).copied().unwrap_or(NO_TEAM)
    }

    /// Paint team of the cell containing `p`.
    pub fn team_at(&self, p: Vec3) -> u8 {
        self.team_at_cell([
            det::floor(p.x) as i32,
            det::floor(p.y) as i32,
            det::floor(p.z) as i32,
        ])
    }

    /// Paint coverage over the whole level (generates all chunks).
    pub fn paint_coverage(&mut self) -> PaintCoverage {
        self.generate_all();
        let mut cov = PaintCoverage::default();
        for ch in self.chunks.iter().flatten() {
            cov.surface_cells += ch.surface_cells as u64;
            for &t in ch.paint.values() {
                *cov.team_cells.entry(t).or_insert(0) += 1;
            }
        }
        cov
    }

    /// Applies an edit, logs it and returns the chunks whose meshes may have
    /// changed (sorted, no duplicates). Edits with a non-finite or negative
    /// radius or a non-finite centre are ignored and not logged.
    pub fn apply(&mut self, edit: TerrainEdit) -> Vec<ChunkCoord> {
        let r = edit.radius();
        if !edit.center().is_finite() || !r.is_finite() || r < 0.0 {
            return Vec::new();
        }
        let index = self.log.len() as u32;
        self.log.push(edit);
        match edit {
            TerrainEdit::Paint {
                center,
                radius,
                team,
            } => self.apply_paint(center, radius, team),
            _ => self.apply_geometry(edit, index),
        }
    }

    /// Chunk coordinates (clamped to the level) whose cube overlaps `b`.
    fn chunks_overlapping(&self, b: Aabb) -> Vec<ChunkCoord> {
        let s = CHUNK_CELLS as f32;
        let lo = |v: f32, d: i32| (det::floor(v / s) as i32).clamp(0, d - 1);
        let (x0, x1) = (lo(b.min.x, self.dims[0]), lo(b.max.x, self.dims[0]));
        let (y0, y1) = (lo(b.min.y, self.dims[1]), lo(b.max.y, self.dims[1]));
        let (z0, z1) = (lo(b.min.z, self.dims[2]), lo(b.max.z, self.dims[2]));
        let mut out = Vec::new();
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let c = ChunkCoord::new(x, y, z);
                    if c.aabb().intersects(&b) {
                        out.push(c);
                    }
                }
            }
        }
        out
    }

    fn apply_geometry(&mut self, edit: TerrainEdit, index: u32) -> Vec<ChunkCoord> {
        let reach = edit.bounds().expand(SDF_CLAMP);
        // Register with every chunk whose corners (plus the padded ring used
        // for uniform-chunk checks) the edit can influence.
        for c in self.chunks_overlapping(reach.expand(1.0)) {
            if !c.aabb().expand(1.0).intersects(&reach) {
                continue;
            }
            let slot = self.chunk_slot(c).expect("in range");
            let o = c.origin();
            let log = &self.log;
            let edits = &mut self.chunk_edits[slot];
            edits.all.push(index);
            if edits.bins.is_empty() {
                edits.bins = vec![Vec::new(); EDIT_BINS.pow(3)];
            }
            // Same bin function as `eval`: a point in the reach box lies in a
            // bin between those of the box corners.
            let lo = [0, 1, 2].map(|a| bin_of(reach.min[a], o[a]));
            let hi = [0, 1, 2].map(|a| bin_of(reach.max[a], o[a]));
            for bz in lo[2]..=hi[2] {
                for by in lo[1]..=hi[1] {
                    for bx in lo[0]..=hi[0] {
                        let bin = &mut edits.bins[bx + EDIT_BINS * (by + EDIT_BINS * bz)];
                        if let Some(later) = containing_crater(&edit) {
                            bin.retain(|&e| !later.covers(&log[e as usize]));
                        }
                        bin.push(index);
                    }
                }
            }
            if self.chunks[slot].is_some() {
                self.update_chunk_samples(c, slot, &edit, reach);
            }
        }
        // Mesh vertices use cells -1..32 of a chunk: world [o - 1, o + 32].
        let affected = edit.bounds().expand(DIRTY_MARGIN);
        let mut dirty: Vec<ChunkCoord> = self
            .chunks_overlapping(affected.expand(1.0))
            .into_iter()
            .filter(|c| mesh_region(*c).intersects(&affected))
            .collect();
        dirty.sort();
        dirty.dedup();
        dirty
    }

    /// Applies `edit` incrementally to the stored samples of a generated chunk.
    fn update_chunk_samples(
        &mut self,
        c: ChunkCoord,
        slot: usize,
        edit: &TerrainEdit,
        reach: Aabb,
    ) {
        let o = c.origin();
        let (lo, hi) = edit_corner_range(o, reach);
        let level = self.level_corners();
        let chunk = self.chunks[slot].as_mut().expect("generated");
        // Cells whose surface state may change: those touching updated corners.
        let cm = CHUNK_CELLS as usize - 1;
        let clo = lo.map(|v| (v as usize).saturating_sub(1).min(cm));
        let chi = hi.map(|v| (v as usize).min(cm));
        let before = chunk.count_surface(clo, chi);
        if !bake_edit(&self.gen, level, o, &mut chunk.samples, edit, lo, hi) {
            return;
        }
        let after = chunk.count_surface(clo, chi);
        chunk.surface_cells = chunk.surface_cells + after - before;
        // Paint lives on surfaces: drop paint from cells that stopped being surface.
        let mut stale = Vec::new();
        for &idx in chunk.paint.keys() {
            let n = CHUNK_CELLS as usize;
            let (i, j, k) = (
                idx as usize % n,
                (idx as usize / n) % n,
                idx as usize / (n * n),
            );
            if !chunk.is_surface_cell(i, j, k) {
                stale.push(idx);
            }
        }
        for idx in stale {
            chunk.paint.remove(&idx);
        }
    }

    fn apply_paint(&mut self, center: Vec3, radius: f32, team: u8) -> Vec<ChunkCoord> {
        let b = Aabb::from_center_half_extents(center, Vec3::splat(radius));
        let r2 = radius * radius;
        let mut changed_cells: Vec<[i32; 3]> = Vec::new();
        for c in self.chunks_overlapping(b) {
            self.ensure_generated(c);
            let slot = self.chunk_slot(c).expect("in range");
            let o = c.origin();
            let n = CHUNK_CELLS;
            // Cells whose centre can be inside the sphere.
            let range = |min: f32, max: f32, oo: i32| {
                let lo = (det::floor(min - 0.5) as i32 - oo).clamp(0, n - 1);
                let hi = (det::ceil(max - 0.5) as i32 - oo).clamp(0, n - 1);
                (lo, hi)
            };
            let (x0, x1) = range(b.min.x, b.max.x, o[0]);
            let (y0, y1) = range(b.min.y, b.max.y, o[1]);
            let (z0, z1) = range(b.min.z, b.max.z, o[2]);
            let chunk = self.chunks[slot].as_mut().expect("generated");
            for k in z0..=z1 {
                for j in y0..=y1 {
                    for i in x0..=x1 {
                        let cc = Vec3::new(
                            (o[0] + i) as f32 + 0.5,
                            (o[1] + j) as f32 + 0.5,
                            (o[2] + k) as f32 + 0.5,
                        );
                        if cc.distance_squared(center) > r2 {
                            continue;
                        }
                        let (iu, ju, ku) = (i as usize, j as usize, k as usize);
                        if !chunk.is_surface_cell(iu, ju, ku) {
                            continue;
                        }
                        let idx = cell_index(iu, ju, ku);
                        let prev = chunk.paint.get(&idx).copied().unwrap_or(NO_TEAM);
                        if prev == team {
                            continue;
                        }
                        if team == NO_TEAM {
                            chunk.paint.remove(&idx);
                        } else {
                            chunk.paint.insert(idx, team);
                        }
                        changed_cells.push([o[0] + i, o[1] + j, o[2] + k]);
                    }
                }
            }
        }
        // A cell is meshed by its own chunk and, at local index 31, by the
        // +1 neighbour (as its cell -1).
        let n = CHUNK_CELLS;
        let mut dirty = Vec::new();
        for g in changed_cells {
            let base = [g[0].div_euclid(n), g[1].div_euclid(n), g[2].div_euclid(n)];
            let edge = [
                g[0].rem_euclid(n) == n - 1,
                g[1].rem_euclid(n) == n - 1,
                g[2].rem_euclid(n) == n - 1,
            ];
            for dx in 0..=edge[0] as i32 {
                for dy in 0..=edge[1] as i32 {
                    for dz in 0..=edge[2] as i32 {
                        let c = ChunkCoord::new(base[0] + dx, base[1] + dy, base[2] + dz);
                        if self.contains_chunk(c) {
                            dirty.push(c);
                        }
                    }
                }
            }
        }
        dirty.sort();
        dirty.dedup();
        dirty
    }

    /// Stored samples of a generated chunk (`33³`, x fastest), for tests and tools.
    pub fn chunk_samples(&self, c: ChunkCoord) -> Option<Vec<f32>> {
        let ch = self.chunk_slot(c).and_then(|s| self.chunks[s].as_ref())?;
        Some(match &ch.samples {
            Samples::Uniform(v) => vec![*v; CHUNK_CORNERS.pow(3)],
            Samples::Dense(d) => d.clone(),
        })
    }
}

/// Pristine samples (generator only, clamped) of the chunk at `o`.
///
/// Uses the generator's 1-Lipschitz bound twice: first the whole padded
/// chunk (27 probes), then an octree over the stored corners, where a block
/// whose centre is further than its half-diagonal (plus a safety margin)
/// beyond the clamp gets the clamp value without sampling each corner. The
/// result is bit-identical to sampling every corner.
fn pristine_samples(sampler: &RegionSampler, o: [i32; 3]) -> Samples {
    let at = |i: f32, j: f32, k: f32| {
        sampler.sample(Vec3::new(o[0] as f32 + i, o[1] as f32 + j, o[2] as f32 + k))
    };
    // If every block centre is far enough beyond the clamp, every stored and
    // padded corner has the clamp value.
    let (mut all_open, mut all_rock) = (true, true);
    'blocks: for bz in UNIFORM_BLOCK_CENTERS {
        for by in UNIFORM_BLOCK_CENTERS {
            for bx in UNIFORM_BLOCK_CENTERS {
                let v = at(bx, by, bz);
                all_open &= v >= SDF_CLAMP + UNIFORM_BLOCK_RADIUS;
                all_rock &= v <= -(SDF_CLAMP + UNIFORM_BLOCK_RADIUS);
                if !all_open && !all_rock {
                    break 'blocks;
                }
            }
        }
    }
    if all_open {
        return Samples::Uniform(SDF_CLAMP);
    }
    if all_rock {
        return Samples::Uniform(-SDF_CLAMP);
    }
    let n = CHUNK_CORNERS;
    // NaN marks corners still to sample: the octree fills the provably
    // clamped blocks, then the rest is sampled a row run at a time.
    let mut d = vec![f32::NAN; n * n * n];
    cull_blocks(sampler, o, &mut d, [0; 3], [n - 1; 3]);
    let xs: Vec<f32> = (0..n as i32).map(|i| (o[0] + i) as f32).collect();
    let mut row = vec![0.0f32; n];
    for k in 0..n {
        for j in 0..n {
            let base = corner_index(0, j, k);
            let (y, z) = ((o[1] + j as i32) as f32, (o[2] + k as i32) as f32);
            let mut i = 0;
            while i < n {
                if !d[base + i].is_nan() {
                    i += 1;
                    continue;
                }
                let start = i;
                while i < n && d[base + i].is_nan() {
                    i += 1;
                }
                sampler.sample_row(y, z, &xs[start..i], &mut row[start..i]);
                for x in start..i {
                    d[base + x] = row[x].clamp(-SDF_CLAMP, SDF_CLAMP) + 0.0;
                }
            }
        }
    }
    Samples::Dense(d)
}

/// Blocks with fewer corners than this per axis are not tested.
const MIN_CULL_BLOCK: usize = 4;

/// Fills the blocks within corners `lo..=hi` of `d` whose every corner
/// provably has the clamp value (see [`pristine_samples`]); leaves the rest.
fn cull_blocks(
    sampler: &RegionSampler,
    o: [i32; 3],
    d: &mut [f32],
    lo: [usize; 3],
    hi: [usize; 3],
) {
    let span = [0, 1, 2].map(|a| hi[a] - lo[a]);
    if span.iter().all(|&s| s + 1 >= MIN_CULL_BLOCK) {
        let c = [0, 1, 2].map(|a| (lo[a] + hi[a]) as f32 * 0.5);
        let centre = Vec3::new(o[0] as f32 + c[0], o[1] as f32 + c[1], o[2] as f32 + c[2]);
        let half = Vec3::new(span[0] as f32, span[1] as f32, span[2] as f32) * 0.5;
        // 1-Lipschitz with a margin for the measured noise gradient bound.
        let reach = SDF_CLAMP + half.length() * 1.05 + 0.25;
        let v = sampler.sample(centre);
        let fill = if v >= reach {
            Some(SDF_CLAMP)
        } else if v <= -reach {
            Some(-SDF_CLAMP)
        } else {
            None
        };
        if let Some(fill) = fill {
            for k in lo[2]..=hi[2] {
                for j in lo[1]..=hi[1] {
                    for i in lo[0]..=hi[0] {
                        d[corner_index(i, j, k)] = fill;
                    }
                }
            }
            return;
        }
        if span.iter().all(|&s| s + 1 >= 2 * MIN_CULL_BLOCK) {
            let mid = [0, 1, 2].map(|a| (lo[a] + hi[a]) / 2);
            for oct in 0..8 {
                let pick = |a: usize| {
                    if oct >> a & 1 == 0 {
                        (lo[a], mid[a])
                    } else {
                        (mid[a] + 1, hi[a])
                    }
                };
                let (x, y, z) = (pick(0), pick(1), pick(2));
                cull_blocks(sampler, o, d, [x.0, y.0, z.0], [x.1, y.1, z.1]);
            }
        }
    }
}

/// Corner index range (inclusive, within `0..=32`) of the chunk at `o` that
/// the edit reach box `reach` can touch.
fn edit_corner_range(o: [i32; 3], reach: Aabb) -> ([i32; 3], [i32; 3]) {
    let n = CHUNK_CORNERS as i32;
    let lo = [0, 1, 2].map(|a| (det::floor(reach.min[a] - o[a] as f32) as i32).clamp(0, n - 1));
    let hi = [0, 1, 2].map(|a| (det::ceil(reach.max[a] - o[a] as f32) as i32).clamp(0, n - 1));
    (lo, hi)
}

/// Applies a geometry edit to the stored corners `lo..=hi` of the chunk at
/// `o` (skipping corners outside the level, where edits never apply), exactly
/// as [`ChunkStore::eval`] composes it. Returns true if any value changed.
fn bake_edit(
    gen: &Generator,
    level: [i32; 3],
    o: [i32; 3],
    samples: &mut Samples,
    edit: &TerrainEdit,
    lo: [i32; 3],
    hi: [i32; 3],
) -> bool {
    let mut changed = false;
    for k in lo[2]..=hi[2] {
        for j in lo[1]..=hi[1] {
            for i in lo[0]..=hi[0] {
                let g = [o[0] + i, o[1] + j, o[2] + k];
                if (0..3).any(|a| g[a] < 0 || g[a] >= level[a]) {
                    continue; // outside the level: edits never apply
                }
                let p = SampleGrid::corner_pos(o, i, j, k);
                let idx = corner_index(i as usize, j as usize, k as usize);
                let old = match samples {
                    Samples::Uniform(u) => *u,
                    Samples::Dense(d) => d[idx],
                };
                let room = gen.room_open(p).clamp(-SDF_CLAMP, SDF_CLAMP);
                let v = edit
                    .apply_to_sdf(old, p)
                    .clamp(-SDF_CLAMP, SDF_CLAMP)
                    .min(room)
                    + 0.0;
                if v.to_bits() == old.to_bits() {
                    continue;
                }
                if let Samples::Uniform(u) = *samples {
                    *samples = Samples::Dense(vec![u; CHUNK_CORNERS.pow(3)]);
                }
                if let Samples::Dense(d) = samples {
                    d[idx] = v;
                }
                changed = true;
            }
        }
    }
    changed
}

/// World region whose corners feed a chunk's mesh (cells -1..32).
fn mesh_region(c: ChunkCoord) -> Aabb {
    let b = c.aabb();
    Aabb::new(b.min - Vec3::ONE, b.max)
}

impl TerrainQuery for ChunkStore {
    fn sdf(&self, p: Vec3) -> f32 {
        self.eval(p)
    }

    fn bounds(&self) -> Aabb {
        self.gen.bounds()
    }
}

impl TerrainQuery for Generator {
    fn sdf(&self, p: Vec3) -> f32 {
        self.sample(p)
    }

    fn bounds(&self) -> Aabb {
        Generator::bounds(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sliders;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn store(e: u8, d: u8) -> ChunkStore {
        ChunkStore::new(Generator::new(12345, Sliders::new(e, d)))
    }

    /// Finds a surface point by casting rays from an open point near the room centre.
    fn surface_point(s: &ChunkStore) -> (Vec3, Vec3) {
        let room = s.generator().room();
        let o = s
            .find_open_point(room.center(), 1.5, 40)
            .expect("open point");
        // Prefer terrain surfaces well away from the sealing walls.
        let inner = Aabb::new(room.min + Vec3::splat(5.0), room.max - Vec3::splat(5.0));
        for i in 0..64u32 {
            let a = i as f32 * 0.7;
            let dir = Vec3::new(
                ls_math::det::cos(a),
                0.9 - (i % 7) as f32 * 0.3,
                ls_math::det::sin(a),
            );
            if let Some(hit) = s.raycast(o, dir, 400.0) {
                if hit.distance > 0.0 && inner.contains(hit.point) {
                    return (hit.point, hit.normal);
                }
            }
        }
        panic!("no surface found");
    }

    #[test]
    fn eval_matches_generator_without_edits() {
        let s = store(50, 50);
        for i in 0..300 {
            let p = Vec3::new(
                (i * 7 % 256) as f32 + 0.25,
                (i * 13 % 128) as f32 + 0.5,
                (i * 29 % 256) as f32,
            );
            let g = s.generator().sample(p).clamp(-SDF_CLAMP, SDF_CLAMP) + 0.0;
            assert_eq!(s.eval(p).to_bits(), g.to_bits());
        }
    }

    #[test]
    fn stored_samples_match_eval() {
        let mut s = store(50, 60);
        let c = ChunkCoord::new(2, 1, 3);
        s.ensure_generated(c);
        let d = s.chunk_samples(c).unwrap();
        let o = c.origin();
        for (n, v) in d.iter().enumerate().step_by(97) {
            let (i, j, k) = (
                n % CHUNK_CORNERS,
                (n / CHUNK_CORNERS) % CHUNK_CORNERS,
                n / CHUNK_CORNERS.pow(2),
            );
            let p = SampleGrid::corner_pos(o, i as i32, j as i32, k as i32);
            assert_eq!(v.to_bits(), s.eval(p).to_bits());
        }
    }

    /// Incremental edits on generated chunks equal fresh generation after the
    /// edits, bit for bit, and match `eval` everywhere.
    #[test]
    fn incremental_edits_equal_fresh_generation() {
        let mut a = store(60, 40);
        let (hit, n) = surface_point(&a);
        let edits = [
            TerrainEdit::SubtractSphere {
                center: hit,
                radius: 3.0,
            },
            TerrainEdit::AddSphere {
                center: hit + n * 3.0,
                radius: 1.5,
            },
            TerrainEdit::SubtractSphere {
                center: hit + Vec3::new(2.0, 0.5, -1.0),
                radius: 4.5,
            },
            TerrainEdit::Paint {
                center: hit,
                radius: 5.0,
                team: 1,
            },
            TerrainEdit::SubtractSphere {
                center: hit + Vec3::new(-1.0, 0.0, 1.0),
                radius: 2.0,
            },
        ];
        // Generate everything around the edits first, then apply.
        let region = Aabb::from_center_half_extents(hit, Vec3::splat(20.0));
        let coords = a.chunks_overlapping(region);
        for &c in &coords {
            a.ensure_generated(c);
        }
        for e in edits {
            a.apply(e);
        }
        // Fresh store: edits first, generation later.
        let mut b = ChunkStore::replay(a.generator().clone(), &edits);
        for &c in &coords {
            b.ensure_generated(c);
            assert_eq!(a.chunk_samples(c), b.chunk_samples(c), "chunk {c:?}");
            let sa = a.chunk_samples(c).unwrap();
            let o = c.origin();
            for (idx, v) in sa.iter().enumerate().step_by(31) {
                let (i, j, k) = (
                    idx % CHUNK_CORNERS,
                    (idx / CHUNK_CORNERS) % CHUNK_CORNERS,
                    idx / CHUNK_CORNERS.pow(2),
                );
                let p = SampleGrid::corner_pos(o, i as i32, j as i32, k as i32);
                assert_eq!(v.to_bits(), a.eval(p).to_bits());
            }
            assert_eq!(
                a.mesh_surface_nets(c).hash(),
                b.mesh_surface_nets(c).hash(),
                "mesh {c:?}"
            );
        }
        // Same paint and surface bookkeeping (both stores, same chunks).
        for &c in &coords {
            let slot = a.chunk_slot(c).unwrap();
            let (ca, cb) = (
                a.chunks[slot].as_ref().unwrap(),
                b.chunks[slot].as_ref().unwrap(),
            );
            assert_eq!(ca.paint, cb.paint, "paint {c:?}");
            assert_eq!(ca.surface_cells, cb.surface_cells, "surface cells {c:?}");
            let m = CHUNK_CELLS as usize - 1;
            assert_eq!(ca.surface_cells, ca.count_surface([0; 3], [m; 3]));
        }
        assert!(coords.iter().any(|&c| !a.chunks[a.chunk_slot(c).unwrap()]
            .as_ref()
            .unwrap()
            .paint
            .is_empty()));
        // The crater really opened space.
        assert!(a.eval(hit - n * 1.0) > 0.0);
    }

    #[test]
    fn crater_dirties_at_most_eight_chunks() {
        let mut s = store(50, 50);
        let mut rng = ls_math::SplitMix64::new(99);
        let mut max_dirty = 0;
        for _ in 0..500 {
            let center = Vec3::new(
                rng.next_f32() * 256.0,
                rng.next_f32() * 128.0,
                rng.next_f32() * 256.0,
            );
            let dirty = s.apply(TerrainEdit::SubtractSphere {
                center,
                radius: 3.0,
            });
            assert!(!dirty.is_empty());
            max_dirty = max_dirty.max(dirty.len());
        }
        // Worst case: a crater right at a chunk corner.
        let dirty = s.apply(TerrainEdit::SubtractSphere {
            center: Vec3::new(64.0, 32.0, 96.0),
            radius: 3.0,
        });
        assert_eq!(dirty.len(), 8);
        assert!(max_dirty <= 8, "{max_dirty}");
    }

    /// Everything outside the dirty set keeps a bit-identical mesh.
    #[test]
    fn non_dirty_chunks_keep_their_mesh() {
        let mut s = store(50, 50);
        let (hit, _) = surface_point(&s);
        let region = Aabb::from_center_half_extents(hit, Vec3::splat(40.0));
        let coords = s.chunks_overlapping(region);
        let before: Vec<_> = coords
            .iter()
            .map(|&c| s.mesh_surface_nets(c).hash())
            .collect();
        let dirty = s.apply(TerrainEdit::SubtractSphere {
            center: hit,
            radius: 3.0,
        });
        let mut changed = 0;
        for (c, h) in coords.iter().zip(before) {
            let after = s.mesh_surface_nets(*c).hash();
            if after != h {
                changed += 1;
                assert!(
                    dirty.contains(c),
                    "{c:?} changed but was not reported dirty"
                );
            }
        }
        assert!(changed >= 1);
    }

    #[test]
    fn craters_cannot_break_the_walls() {
        let mut s = store(100, 0);
        s.apply(TerrainEdit::SubtractSphere {
            center: Vec3::new(1.0, 64.0, 128.0),
            radius: 5.0,
        });
        assert!(s.eval(Vec3::new(0.5, 64.0, 128.0)) < 0.0);
        assert!(s.eval(Vec3::new(1.9, 64.0, 128.0)) < 0.0);
        // Out-of-level points are solid.
        assert!(s.eval(Vec3::new(-5.0, 64.0, 128.0)) < 0.0);
        assert!(s.sphere_overlap(Vec3::new(-5.0, 64.0, 128.0), 0.1));
    }

    #[test]
    fn invalid_edits_are_ignored() {
        let mut s = store(50, 50);
        assert!(s
            .apply(TerrainEdit::SubtractSphere {
                center: Vec3::new(f32::NAN, 0.0, 0.0),
                radius: 3.0
            })
            .is_empty());
        assert!(s
            .apply(TerrainEdit::AddSphere {
                center: Vec3::ONE,
                radius: -1.0
            })
            .is_empty());
        assert!(s.edit_log().is_empty());
    }

    #[test]
    fn paint_and_coverage() {
        let mut s = ChunkStore::new(Generator::with_size(7, Sliders::new(50, 30), [2, 2, 2]));
        let cov0 = s.paint_coverage();
        assert!(cov0.surface_cells > 1000);
        assert!(cov0.team_cells.is_empty());
        let (hit, _) = surface_point(&s);
        let dirty = s.apply(TerrainEdit::Paint {
            center: hit,
            radius: 4.0,
            team: 0,
        });
        assert!(!dirty.is_empty() && dirty.len() <= 8);
        let near_hit = (-1..=1).any(|dz| {
            (-1..=1).any(|dy| {
                (-1..=1).any(|dx| s.team_at(hit + Vec3::new(dx as f32, dy as f32, dz as f32)) == 0)
            })
        });
        assert!(near_hit);
        let cov1 = s.paint_coverage();
        let red = cov1.team_cells[&0];
        assert!(red > 10, "{red}");
        assert!(cov1.fraction(0) > 0.0 && cov1.fraction(0) < 0.2);
        assert_eq!(cov1.fraction(1), 0.0);
        // Overpaint half with team 1.
        s.apply(TerrainEdit::Paint {
            center: hit,
            radius: 2.0,
            team: 1,
        });
        let cov2 = s.paint_coverage();
        assert!(cov2.team_cells[&1] > 0);
        assert_eq!(cov2.team_cells[&0] + cov2.team_cells[&1], red);
        // Painting again with the same team changes nothing.
        assert!(s
            .apply(TerrainEdit::Paint {
                center: hit,
                radius: 2.0,
                team: 1
            })
            .is_empty());
        // The mesh carries the paint.
        let c = s.chunk_at(hit).unwrap();
        let m = s.mesh_surface_nets(c);
        assert!(m.teams.contains(&1) && m.teams.contains(&0));
        // A crater through the painted area removes paint from vanished surfaces.
        s.apply(TerrainEdit::SubtractSphere {
            center: hit,
            radius: 3.0,
        });
        let cov3 = s.paint_coverage();
        assert!(
            cov3.team_cells.get(&1).copied().unwrap_or(0) < cov2.team_cells[&1],
            "{cov2:?} {cov3:?} {hit:?}"
        );
        // Erase with NO_TEAM.
        s.apply(TerrainEdit::Paint {
            center: hit,
            radius: 10.0,
            team: NO_TEAM,
        });
        assert!(s.paint_coverage().team_cells.is_empty());
    }

    #[test]
    fn raycast_and_overlap_queries() {
        let s = store(50, 50);
        let (hit, n) = surface_point(&s);
        assert!(s.eval(hit).abs() < 0.01);
        assert!((n.length() - 1.0).abs() < 1e-4);
        // A ray from just off the surface back towards it hits near the same point.
        let from = hit + n * 2.0;
        let back = s.raycast(from, -n, 10.0).expect("hit");
        assert!(back.distance < 2.5, "{back:?}");
        assert!(s.sphere_overlap(hit, 0.5));
        assert!(!s.sphere_overlap(hit + n * 3.0, 0.1) || s.eval(hit + n * 3.0) < 0.1);
        // Rays leaving the level always hit the sealing wall.
        let c = s.generator().room().center();
        let wall = s.raycast(c, Vec3::Y, 1000.0);
        assert!(wall.is_some());
        assert!(wall.unwrap().point.y <= 126.1);
        // Trait object and bounds.
        let q: &dyn TerrainQuery = &s;
        assert_eq!(q.bounds().max, Vec3::new(256.0, 128.0, 256.0));
    }

    #[test]
    fn sky_chunks_are_uniform_and_cheap() {
        let mut s = store(0, 0);
        // Interior chunks away from the walls; stop at the first uniform one.
        let mut found = None;
        'search: for y in 1..3 {
            for z in 1..7 {
                for x in 1..7 {
                    let c = ChunkCoord::new(x, y, z);
                    if s.is_uniform(c) {
                        found = Some(c);
                        break 'search;
                    }
                }
            }
        }
        let c = found.expect("open sky should have uniform chunks");
        // A uniform chunk's full padded grid really has no surface.
        let g = SampleGrid::from_fn(c.origin(), |p| s.eval(p));
        assert!(g.values().iter().all(|v| *v == SDF_CLAMP));
        assert_eq!(s.sample_grid(c), g);
        assert!(s.mesh_surface_nets(c).is_empty());
    }

    /// Many random edits, clustered and spread, including ones overlapping
    /// chunk and bin borders and the walls.
    fn random_edits(center: Vec3, n: usize, seed: u64) -> Vec<TerrainEdit> {
        let mut rng = ls_math::SplitMix64::new(seed);
        (0..n)
            .map(|i| {
                let spread = if i % 3 == 0 { 120.0 } else { 14.0 };
                let c = center
                    + Vec3::new(
                        rng.next_f32() - 0.5,
                        rng.next_f32() - 0.5,
                        rng.next_f32() - 0.5,
                    ) * spread;
                // Some centres exactly on chunk / bin planes.
                let c = if i % 7 == 0 {
                    Vec3::new(
                        det::floor(c.x / 8.0) * 8.0,
                        c.y,
                        det::floor(c.z / 32.0) * 32.0,
                    )
                } else {
                    c
                };
                let radius = 1.0 + rng.next_f32() * 5.0;
                if i % 5 == 4 {
                    TerrainEdit::AddSphere { center: c, radius }
                } else {
                    TerrainEdit::SubtractSphere { center: c, radius }
                }
            })
            .collect()
    }

    /// Reference world SDF: every logged edit replayed at every point.
    fn reference_eval(s: &ChunkStore, p: Vec3) -> f32 {
        let gen = s.generator();
        let mut d = gen.terrain(p);
        if s.chunk_at(p).is_some() {
            for e in s.edit_log() {
                d = e.apply_to_sdf(d, p);
            }
        }
        d.min(gen.room_open(p)).clamp(-SDF_CLAMP, SDF_CLAMP) + 0.0
    }

    /// `eval` (per-bin edit lists) equals replaying the whole log, bit for bit.
    #[test]
    fn binned_eval_matches_full_replay() {
        let mut s = store(50, 50);
        let (hit, _) = surface_point(&s);
        for e in random_edits(hit, 300, 3) {
            s.apply(e);
        }
        let mut rng = ls_math::SplitMix64::new(4);
        for i in 0..20_000 {
            let mut p = hit
                + Vec3::new(
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                ) * 40.0;
            if i % 4 == 0 {
                // On bin / chunk planes and lattice corners.
                p = Vec3::new(det::floor(p.x / 8.0) * 8.0, det::floor(p.y), p.z);
            }
            assert_eq!(
                s.eval(p).to_bits(),
                reference_eval(&s, p).to_bits(),
                "{p:?}"
            );
        }
        for p in [Vec3::new(-0.5, 64.0, 128.0), Vec3::new(256.0, 64.0, 128.0)] {
            assert_eq!(s.eval(p).to_bits(), reference_eval(&s, p).to_bits());
        }
    }

    /// Block culling in `pristine_samples` (Lipschitz bound) never changes a
    /// stored sample: every corner of every chunk equals the clamped
    /// generator sample, for several slider mixes. Release builds check all
    /// corners of all chunks; debug builds a subset.
    #[test]
    fn culled_samples_match_generator_everywhere() {
        let full = !cfg!(debug_assertions);
        let mixes: &[(u8, u8)] = if full {
            &[(0, 0), (50, 60), (75, 20), (100, 100)]
        } else {
            &[(50, 60), (100, 100)]
        };
        for &(e, d) in mixes {
            let mut s = store(e, d);
            let gen = s.generator().clone();
            let coords = s.chunk_coords();
            let (chunk_step, corner_step) = if full { (1, 1) } else { (23, 7) };
            for c in coords.into_iter().step_by(chunk_step) {
                s.ensure_generated(c);
                let samples = s.chunk_samples(c).unwrap();
                let o = c.origin();
                for (n, v) in samples.iter().enumerate().step_by(corner_step) {
                    let (i, j, k) = (
                        n % CHUNK_CORNERS,
                        (n / CHUNK_CORNERS) % CHUNK_CORNERS,
                        n / CHUNK_CORNERS.pow(2),
                    );
                    let p = SampleGrid::corner_pos(o, i as i32, j as i32, k as i32);
                    let want = gen.sample(p).clamp(-SDF_CLAMP, SDF_CLAMP) + 0.0;
                    assert_eq!(v.to_bits(), want.to_bits(), "{e}/{d} {c:?} {p:?}");
                }
            }
        }
    }

    /// Clustered, nested craters interleaved with AddSpheres: pruning covered
    /// craters from the bins keeps `eval` bit-identical to a full replay
    /// and keeps the bins short.
    #[test]
    fn pruned_clustered_craters_match_full_replay() {
        let mut s = store(50, 50);
        let (hit, _) = surface_point(&s);
        let mut rng = ls_math::SplitMix64::new(11);
        let mut jitter = |scale: f32| {
            Vec3::new(
                rng.next_f32() - 0.5,
                rng.next_f32() - 0.5,
                rng.next_f32() - 0.5,
            ) * scale
        };
        for i in 0..240 {
            let c = hit + jitter(3.0);
            let radius = 1.0 + (i % 6) as f32 * 0.8 + jitter(0.5).x;
            if i % 5 == 3 {
                s.apply(TerrainEdit::AddSphere {
                    center: c,
                    radius: radius * 0.5,
                });
            } else {
                s.apply(TerrainEdit::SubtractSphere { center: c, radius });
            }
            // Exact nesting, and a same-sized repeat (not strictly covered).
            if i % 9 == 0 {
                s.apply(TerrainEdit::SubtractSphere { center: c, radius });
                s.apply(TerrainEdit::SubtractSphere {
                    center: c,
                    radius: radius + 0.5,
                });
            }
        }
        for _ in 0..20_000 {
            let p = hit + jitter(24.0);
            assert_eq!(
                s.eval(p).to_bits(),
                reference_eval(&s, p).to_bits(),
                "{p:?}"
            );
        }
        let c = s.chunk_at(hit).unwrap();
        let edits = &s.chunk_edits[s.chunk_slot(c).unwrap()];
        let longest = edits.bins.iter().map(Vec::len).max().unwrap();
        assert!(
            longest * 2 < edits.all.len(),
            "bins not pruned: {longest} of {}",
            edits.all.len()
        );
    }

    /// Chunks generated after 200 edits (baked once each) equal chunks that
    /// were generated first and edited incrementally, and match `eval`.
    #[test]
    fn baked_generation_after_many_edits_equals_incremental() {
        let mut a = store(75, 30);
        let (hit, _) = surface_point(&a);
        let edits = random_edits(hit, 200, 9);
        let region = Aabb::from_center_half_extents(hit, Vec3::splat(24.0));
        let coords = a.chunks_overlapping(region);
        for &c in &coords {
            a.ensure_generated(c);
        }
        for e in &edits {
            a.apply(*e);
        }
        let mut b = ChunkStore::replay(a.generator().clone(), &edits);
        for &c in &coords {
            b.ensure_generated(c);
            let (sa, sb) = (a.chunk_samples(c).unwrap(), b.chunk_samples(c).unwrap());
            assert!(
                sa.iter().zip(&sb).all(|(x, y)| x.to_bits() == y.to_bits()),
                "{c:?}"
            );
            let o = c.origin();
            for (idx, v) in sb.iter().enumerate().step_by(13) {
                let (i, j, k) = (
                    idx % CHUNK_CORNERS,
                    (idx / CHUNK_CORNERS) % CHUNK_CORNERS,
                    idx / CHUNK_CORNERS.pow(2),
                );
                let p = SampleGrid::corner_pos(o, i as i32, j as i32, k as i32);
                assert_eq!(v.to_bits(), b.eval(p).to_bits(), "{p:?}");
            }
            let slot = a.chunk_slot(c).unwrap();
            let (ca, cb) = (
                a.chunks[slot].as_ref().unwrap(),
                b.chunks[slot].as_ref().unwrap(),
            );
            assert_eq!(ca.surface_cells, cb.surface_cells, "{c:?}");
            let m = CHUNK_CELLS as usize - 1;
            assert_eq!(cb.surface_cells, cb.count_surface([0; 3], [m; 3]));
            assert_eq!(a.mesh_surface_nets(c).hash(), b.mesh_surface_nets(c).hash());
        }
    }

    /// The fast surface-cell count agrees with the per-cell test.
    #[test]
    fn surface_count_matches_cell_test() {
        let mut s = store(50, 60);
        let c = ChunkCoord::new(3, 1, 3);
        s.ensure_generated(c);
        let ch = s.chunks[s.chunk_slot(c).unwrap()].as_ref().unwrap();
        for (lo, hi) in [
            ([0; 3], [31; 3]),
            ([3, 0, 7], [9, 31, 8]),
            ([31; 3], [31; 3]),
        ] {
            let mut n = 0;
            for k in lo[2]..=hi[2] {
                for j in lo[1]..=hi[1] {
                    for i in lo[0]..=hi[0] {
                        n += ch.is_surface_cell(i, j, k) as u32;
                    }
                }
            }
            assert_eq!(ch.count_surface(lo, hi), n);
        }
        assert!(ch.surface_cells > 1000);
    }

    #[test]
    fn chunk_coord_helpers() {
        let s = store(50, 50);
        assert_eq!(s.chunk_coords().len(), 8 * 4 * 8);
        assert_eq!(
            s.chunk_at(Vec3::new(33.0, 0.0, 255.9)),
            Some(ChunkCoord::new(1, 0, 7))
        );
        assert_eq!(s.chunk_at(Vec3::new(256.0, 0.0, 0.0)), None);
        assert_eq!(s.chunk_at(Vec3::new(-0.1, 0.0, 0.0)), None);
        assert!(!s.is_generated(ChunkCoord::new(0, 0, 0)));
    }
}
