//! Padded sample grid: everything a mesher needs to mesh one chunk.
//!
//! A chunk owns cells `0..32` on each axis. To emit every face whose edge
//! starts inside the chunk, the mesher also needs the vertices of the cells
//! at local index `-1` (owned by the neighbouring chunks), so the grid holds
//! corners `-1..=32` (34 per axis) and paint teams and surface state bits
//! for cells `-1..=31` (33 per axis). Neighbouring chunks compute the shared vertices from
//! bit-identical samples, which is what makes the meshes seam-free.

use ls_math::Vec3;

use crate::{CHUNK_CELLS, NO_TEAM};

/// Corners per axis in a padded grid (`-1..=32`).
pub const GRID_CORNERS: usize = CHUNK_CELLS as usize + 2;
/// Cells per axis in a padded grid (`-1..=31`).
pub const GRID_CELLS: usize = CHUNK_CELLS as usize + 1;

/// SDF samples (and surface layers) around one chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleGrid {
    /// World position of local corner `(0, 0, 0)` (1 u per cell).
    pub origin: [i32; 3],
    values: Vec<f32>,
    teams: Vec<u8>,
    /// Active surface state bits per cell (`ls_math::SurfaceStates`).
    states: Vec<u8>,
}

impl SampleGrid {
    /// A grid filled by evaluating `f` at every corner position; no paint or states.
    pub fn from_fn(origin: [i32; 3], f: impl Fn(Vec3) -> f32) -> Self {
        let n = GRID_CORNERS;
        let mut values = Vec::with_capacity(n * n * n);
        for k in -1..=CHUNK_CELLS {
            for j in -1..=CHUNK_CELLS {
                for i in -1..=CHUNK_CELLS {
                    values.push(f(Self::corner_pos(origin, i, j, k)));
                }
            }
        }
        SampleGrid {
            origin,
            values,
            teams: vec![NO_TEAM; GRID_CELLS * GRID_CELLS * GRID_CELLS],
            states: vec![0; GRID_CELLS * GRID_CELLS * GRID_CELLS],
        }
    }

    /// A grid from raw corner values (`GRID_CORNERS³`, x fastest) with no
    /// paint or states.
    ///
    /// # Panics
    /// If `values` has the wrong length.
    pub fn from_values(origin: [i32; 3], values: Vec<f32>) -> Self {
        assert_eq!(values.len(), GRID_CORNERS * GRID_CORNERS * GRID_CORNERS);
        SampleGrid {
            origin,
            values,
            teams: vec![NO_TEAM; GRID_CELLS * GRID_CELLS * GRID_CELLS],
            states: vec![0; GRID_CELLS * GRID_CELLS * GRID_CELLS],
        }
    }

    /// World position of a local corner.
    #[inline]
    pub fn corner_pos(origin: [i32; 3], i: i32, j: i32, k: i32) -> Vec3 {
        Vec3::new(
            (origin[0] + i) as f32,
            (origin[1] + j) as f32,
            (origin[2] + k) as f32,
        )
    }

    #[inline]
    fn corner_index(i: i32, j: i32, k: i32) -> usize {
        let n = GRID_CORNERS;
        (i + 1) as usize + n * ((j + 1) as usize + n * (k + 1) as usize)
    }

    #[inline]
    fn cell_index(i: i32, j: i32, k: i32) -> usize {
        let n = GRID_CELLS;
        (i + 1) as usize + n * ((j + 1) as usize + n * (k + 1) as usize)
    }

    /// Sample at local corner `(i, j, k)`, each in `-1..=32`.
    #[inline]
    pub fn value(&self, i: i32, j: i32, k: i32) -> f32 {
        self.values[Self::corner_index(i, j, k)]
    }

    /// Paint team of local cell `(i, j, k)`, each in `-1..=31`.
    #[inline]
    pub fn team(&self, i: i32, j: i32, k: i32) -> u8 {
        self.teams[Self::cell_index(i, j, k)]
    }

    /// Sets the paint team of local cell `(i, j, k)`, each in `-1..=31`.
    #[inline]
    pub fn set_team(&mut self, i: i32, j: i32, k: i32, team: u8) {
        self.teams[Self::cell_index(i, j, k)] = team;
    }

    /// Active surface state bits of local cell `(i, j, k)`, each in `-1..=31`.
    #[inline]
    pub fn states(&self, i: i32, j: i32, k: i32) -> u8 {
        self.states[Self::cell_index(i, j, k)]
    }

    /// Sets the surface state bits of local cell `(i, j, k)`, each in `-1..=31`.
    #[inline]
    pub fn set_states(&mut self, i: i32, j: i32, k: i32, bits: u8) {
        self.states[Self::cell_index(i, j, k)] = bits;
    }

    /// All corner values (x fastest, then y, then z).
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// True if every corner is on the same side of the surface (no mesh).
    pub fn is_uniform_sign(&self) -> bool {
        let first = self.values[0] < 0.0;
        self.values.iter().all(|v| (*v < 0.0) == first)
    }
}
