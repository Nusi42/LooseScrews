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

use std::collections::BTreeMap;

use ls_math::terrain::{TerrainEdit, TerrainQuery};
use ls_math::{det, Aabb, Vec3};
use serde::{Deserialize, Serialize};

use crate::grid::SampleGrid;
use crate::mesh::{dual_contouring_with_sdf, surface_nets, Mesh};
use crate::{Generator, CHUNK_CELLS, NO_TEAM};

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
        let mut n = 0;
        for k in lo[2]..=hi[2] {
            for j in lo[1]..=hi[1] {
                for i in lo[0]..=hi[0] {
                    n += self.is_surface_cell(i, j, k) as u32;
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

/// Chunked, editable, paintable terrain for one level.
#[derive(Debug, Clone)]
pub struct ChunkStore {
    gen: Generator,
    dims: [i32; 3],
    chunks: Vec<Option<Chunk>>,
    /// Indices into `log` of the geometry edits that can affect each chunk.
    chunk_edits: Vec<Vec<u32>>,
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
            chunk_edits: vec![Vec::new(); count],
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
        let c = ChunkCoord::new(
            det::floor(p.x / s) as i32,
            det::floor(p.y / s) as i32,
            det::floor(p.z / s) as i32,
        );
        self.contains_chunk(c).then_some(c)
    }

    /// The world SDF (see the module docs). Needs no generated chunks.
    ///
    /// Cost grows linearly with the number of geometry edits registered with the
    /// point's chunk (the list only grows). Wave-4 perf work: prune edits fully
    /// contained in a later `SubtractSphere`, or read baked chunk samples instead
    /// of replaying the list.
    pub fn eval(&self, p: Vec3) -> f32 {
        let room = self.gen.room_open(p);
        let v = match self.chunk_at(p).and_then(|c| self.chunk_slot(c)) {
            Some(slot) => {
                let mut d = self.gen.terrain(p);
                for &e in &self.chunk_edits[slot] {
                    d = self.log[e as usize].apply_to_sdf(d, p);
                }
                d.min(room)
            }
            None => self.gen.terrain(p).min(room),
        };
        v.clamp(-SDF_CLAMP, SDF_CLAMP) + 0.0
    }

    /// Generates a chunk if needed.
    pub fn ensure_generated(&mut self, c: ChunkCoord) {
        let Some(slot) = self.chunk_slot(c) else {
            return;
        };
        if self.chunks[slot].is_some() {
            return;
        }
        let o = c.origin();
        let mut samples = None;
        if self.chunk_edits[slot].is_empty() {
            // 1-Lipschitz: if every block centre is far enough beyond the
            // clamp, every stored and padded corner has the clamp value.
            let (mut all_open, mut all_rock) = (true, true);
            'blocks: for bz in UNIFORM_BLOCK_CENTERS {
                for by in UNIFORM_BLOCK_CENTERS {
                    for bx in UNIFORM_BLOCK_CENTERS {
                        let p = Vec3::new(o[0] as f32 + bx, o[1] as f32 + by, o[2] as f32 + bz);
                        let v = self.gen.sample(p);
                        all_open &= v >= SDF_CLAMP + UNIFORM_BLOCK_RADIUS;
                        all_rock &= v <= -(SDF_CLAMP + UNIFORM_BLOCK_RADIUS);
                        if !all_open && !all_rock {
                            break 'blocks;
                        }
                    }
                }
            }
            if all_open {
                samples = Some(Samples::Uniform(SDF_CLAMP));
            } else if all_rock {
                samples = Some(Samples::Uniform(-SDF_CLAMP));
            }
        }
        let samples = samples.unwrap_or_else(|| {
            let n = CHUNK_CORNERS;
            let mut d = Vec::with_capacity(n * n * n);
            for k in 0..n as i32 {
                for j in 0..n as i32 {
                    for i in 0..n as i32 {
                        d.push(self.eval(SampleGrid::corner_pos(o, i, j, k)));
                    }
                }
            }
            Samples::Dense(d)
        });
        let mut chunk = Chunk {
            samples,
            paint: BTreeMap::new(),
            surface_cells: 0,
        };
        let m = CHUNK_CELLS as usize - 1;
        chunk.surface_cells = chunk.count_surface([0; 3], [m; 3]);
        self.chunks[slot] = Some(chunk);
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
                        _ => self.corner_value([o[0] + i, o[1] + j, o[2] + k]),
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
    fn corner_value(&self, g: [i32; 3]) -> f32 {
        let n = CHUNK_CELLS;
        let c = ChunkCoord::new(g[0].div_euclid(n), g[1].div_euclid(n), g[2].div_euclid(n));
        if let Some(ch) = self.chunk_slot(c).and_then(|s| self.chunks[s].as_ref()) {
            let l = [g[0].rem_euclid(n), g[1].rem_euclid(n), g[2].rem_euclid(n)];
            return ch.value(l[0] as usize, l[1] as usize, l[2] as usize);
        }
        self.eval(Vec3::new(g[0] as f32, g[1] as f32, g[2] as f32))
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
    /// with exact Hermite data from [`ChunkStore::eval`]. Much
    /// slower than surface nets; see the timing test.
    pub fn mesh_dual_contouring(&mut self, c: ChunkCoord) -> Mesh {
        if !self.contains_chunk(c) || self.is_uniform(c) {
            return Mesh::default();
        }
        let grid = self.sample_grid(c);
        dual_contouring_with_sdf(&grid, &|p| self.eval(p))
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
            self.chunk_edits[slot].push(index);
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
        let n = CHUNK_CORNERS as i32;
        let local = |v: f32, oo: i32| v - oo as f32;
        let lo = [
            (det::floor(local(reach.min.x, o[0])) as i32).clamp(0, n - 1),
            (det::floor(local(reach.min.y, o[1])) as i32).clamp(0, n - 1),
            (det::floor(local(reach.min.z, o[2])) as i32).clamp(0, n - 1),
        ];
        let hi = [
            (det::ceil(local(reach.max.x, o[0])) as i32).clamp(0, n - 1),
            (det::ceil(local(reach.max.y, o[1])) as i32).clamp(0, n - 1),
            (det::ceil(local(reach.max.z, o[2])) as i32).clamp(0, n - 1),
        ];
        let room_of = |p: Vec3| self.gen.room_open(p).clamp(-SDF_CLAMP, SDF_CLAMP);
        let inside = |p: Vec3| self.chunk_at(p).is_some();
        // Precompute new values first (immutable borrow), then write.
        let chunk = self.chunks[slot].as_ref().expect("generated");
        let mut updates = Vec::new();
        for k in lo[2]..=hi[2] {
            for j in lo[1]..=hi[1] {
                for i in lo[0]..=hi[0] {
                    let p = SampleGrid::corner_pos(o, i, j, k);
                    if !inside(p) {
                        continue; // outside the level: edits never apply
                    }
                    let old = chunk.value(i as usize, j as usize, k as usize);
                    let v = edit
                        .apply_to_sdf(old, p)
                        .clamp(-SDF_CLAMP, SDF_CLAMP)
                        .min(room_of(p))
                        + 0.0;
                    if v.to_bits() != old.to_bits() {
                        updates.push((corner_index(i as usize, j as usize, k as usize), v));
                    }
                }
            }
        }
        if updates.is_empty() {
            return;
        }
        let chunk = self.chunks[slot].as_mut().expect("generated");
        // Cells whose surface state may change: those touching updated corners.
        let cm = CHUNK_CELLS as usize - 1;
        let clo = lo.map(|v| (v as usize).saturating_sub(1).min(cm));
        let chi = hi.map(|v| (v as usize).min(cm));
        let before = chunk.count_surface(clo, chi);
        if let Samples::Uniform(u) = chunk.samples {
            chunk.samples = Samples::Dense(vec![u; CHUNK_CORNERS.pow(3)]);
        }
        if let Samples::Dense(d) = &mut chunk.samples {
            for (idx, v) in updates {
                d[idx] = v;
            }
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
