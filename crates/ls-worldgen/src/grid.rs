//! Padded sample grid: everything a mesher needs to mesh one chunk.
//!
//! A chunk owns cells `0..32` on each axis. To emit every face whose edge
//! starts inside the chunk, the mesher also needs the vertices of the cells
//! at local index `-1` (owned by the neighbouring chunks), so the grid holds
//! paint teams, paint colours and surface state bits for cells `-1..=31` (33
//! per axis). Corner values cover a wider halo, `-GRID_HALO..=32 + GRID_HALO`
//! ([`GRID_HALO`] = 4, 41 per axis): the surface-nets mesher reads it for
//! continuous vertex normals (central differences one corner beyond the
//! cell) and for baked ambient occlusion (probes up to 2.5 u out along the
//! normal). Neighbouring chunks compute the shared vertices,
//! normals and occlusion from bit-identical samples, which is what makes
//! the meshes seam-free.

use ls_math::Vec3;

use crate::{CHUNK_CELLS, NO_TEAM};

/// Corners per axis that vertex placement reads (`-1..=32`).
pub const GRID_CORNERS: usize = CHUNK_CELLS as usize + 2;
/// Extra corners stored beyond the chunk on every side (corner values cover
/// `-GRID_HALO..=32 + GRID_HALO`).
pub const GRID_HALO: i32 = 4;
/// Corners per axis stored in a padded grid (`-GRID_HALO..=32 + GRID_HALO`).
pub const HALO_CORNERS: usize = CHUNK_CELLS as usize + 1 + 2 * GRID_HALO as usize;
/// Cells per axis in a padded grid (`-1..=31`).
pub const GRID_CELLS: usize = CHUNK_CELLS as usize + 1;

/// SDF samples (and surface layers) around one chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleGrid {
    /// World position of local corner `(0, 0, 0)` (1 u per cell).
    pub origin: [i32; 3],
    values: Vec<f32>,
    teams: Vec<u8>,
    /// Paint colour id per cell (`ls_math::terrain::PAINT_*`).
    colors: Vec<u8>,
    /// Active surface state bits per cell (`ls_math::SurfaceStates`).
    states: Vec<u8>,
}

impl SampleGrid {
    /// A grid filled by evaluating `f` at every corner position (halo
    /// included); no paint or states.
    pub fn from_fn(origin: [i32; 3], f: impl Fn(Vec3) -> f32) -> Self {
        let n = HALO_CORNERS;
        let (lo, hi) = (-GRID_HALO, CHUNK_CELLS + GRID_HALO);
        let mut values = Vec::with_capacity(n * n * n);
        for k in lo..=hi {
            for j in lo..=hi {
                for i in lo..=hi {
                    values.push(f(Self::corner_pos(origin, i, j, k)));
                }
            }
        }
        Self::from_values(origin, values)
    }

    /// A grid from raw corner values (`HALO_CORNERS³` for corners
    /// `-GRID_HALO..=32 + GRID_HALO`, x fastest) with no paint or states.
    ///
    /// # Panics
    /// If `values` has the wrong length.
    pub fn from_values(origin: [i32; 3], values: Vec<f32>) -> Self {
        assert_eq!(values.len(), HALO_CORNERS * HALO_CORNERS * HALO_CORNERS);
        SampleGrid {
            origin,
            values,
            teams: vec![NO_TEAM; GRID_CELLS * GRID_CELLS * GRID_CELLS],
            colors: vec![0; GRID_CELLS * GRID_CELLS * GRID_CELLS],
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
        let n = HALO_CORNERS;
        let h = GRID_HALO;
        (i + h) as usize + n * ((j + h) as usize + n * (k + h) as usize)
    }

    #[inline]
    fn cell_index(i: i32, j: i32, k: i32) -> usize {
        let n = GRID_CELLS;
        (i + 1) as usize + n * ((j + 1) as usize + n * (k + 1) as usize)
    }

    /// Sample at local corner `(i, j, k)`, each in
    /// `-GRID_HALO..=32 + GRID_HALO`.
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

    /// Paint colour id of local cell `(i, j, k)`, each in `-1..=31`.
    #[inline]
    pub fn color(&self, i: i32, j: i32, k: i32) -> u8 {
        self.colors[Self::cell_index(i, j, k)]
    }

    /// Sets the paint colour id of local cell `(i, j, k)`, each in `-1..=31`.
    #[inline]
    pub fn set_color(&mut self, i: i32, j: i32, k: i32, color: u8) {
        self.colors[Self::cell_index(i, j, k)] = color;
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

    /// All corner values, halo included (x fastest, then y, then z).
    /// Corner values for in-crate display fixes (paint and states stay).
    pub(crate) fn values_mut(&mut self) -> &mut [f32] {
        &mut self.values
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// True if every corner the mesher places vertices from (`-1..=32`) is
    /// on the same side of the surface (no mesh).
    pub fn is_uniform_sign(&self) -> bool {
        let first = self.value(-1, -1, -1) < 0.0;
        (-1..=CHUNK_CELLS).all(|k| {
            (-1..=CHUNK_CELLS)
                .all(|j| (-1..=CHUNK_CELLS).all(|i| (self.value(i, j, k) < 0.0) == first))
        })
    }
}
