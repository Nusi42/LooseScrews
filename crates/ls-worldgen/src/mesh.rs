//! Chunk meshers: surface nets and dual contouring.
//!
//! Both share the same topology: one vertex per cell whose corners change
//! sign, and one quad (two triangles) per sign-changing grid edge, joining the
//! vertices of the four cells around that edge. A chunk emits the quads of
//! the edges whose lower corner lies inside it (`0..32` per axis); the
//! vertices of cells at local index `-1` are computed from the padded grid
//! exactly as the neighbouring chunk computes them, so borders match
//! bit-for-bit and there are no seams.
//!
//! They differ in where the vertex goes:
//! - **Surface nets** averages the edge crossing points (smooth, fast).
//! - **Dual contouring** minimizes a quadratic error function (QEF) built
//!   from the crossing points and their normals, which keeps sharp edges and
//!   corners (mine shafts, faceted rocks).
//!
//! Winding is counter-clockwise when seen from open space; normals point
//! into open space (towards increasing SDF). Positions are in world units.

use sha2::{Digest, Sha256};

use ls_math::Vec3;

use crate::grid::{SampleGrid, GRID_CELLS, GRID_CORNERS};
use crate::CHUNK_CELLS;

/// A triangle mesh for one chunk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    /// World-space vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals pointing into open space.
    pub normals: Vec<[f32; 3]>,
    /// Paint team per vertex ([`crate::NO_TEAM`] = unpainted).
    pub teams: Vec<u8>,
    /// Paint effect colour per vertex (`ls_math::terrain::PAINT_*`: 0 plain,
    /// 1 red bouncy, 2 blue slippery, 3 yellow sticky; 0 where unpainted).
    pub colors: Vec<u8>,
    /// Surface state bits per vertex (`ls_math::SurfaceStates`: 1 hot,
    /// 2 charged, 4 glow active at the chunk store's display tick, 8
    /// permanent scorch).
    pub states: Vec<u8>,
    /// Rock material id per vertex ([`crate::Material::id`]: 0 weak, 1 rock,
    /// 2 hard). Filled by the [`crate::ChunkStore`] meshers; empty for meshes
    /// built from a bare [`SampleGrid`].
    pub materials: Vec<u8>,
    /// Triangle list (counter-clockwise from outside).
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// SHA-256 over the geometry, paint and surface states (little-endian
    /// bit patterns), for golden tests. States and paint colours are hashed
    /// only when a vertex has one, so meshes without them hash as they did
    /// before they existed. Materials are a pure function of the vertex positions and the
    /// seed and are not hashed.
    pub fn hash(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update((self.positions.len() as u32).to_le_bytes());
        h.update((self.indices.len() as u32).to_le_bytes());
        for p in &self.positions {
            for c in p {
                h.update(c.to_bits().to_le_bytes());
            }
        }
        for n in &self.normals {
            for c in n {
                h.update(c.to_bits().to_le_bytes());
            }
        }
        h.update(&self.teams);
        for i in &self.indices {
            h.update(i.to_le_bytes());
        }
        if self.states.iter().any(|&b| b != 0) {
            h.update(b"states");
            h.update(&self.states);
        }
        if self.colors.iter().any(|&b| b != 0) {
            h.update(b"colors");
            h.update(&self.colors);
        }
        h.finalize().into()
    }
}

/// Meshes a chunk with surface nets.
pub fn surface_nets(grid: &SampleGrid) -> Mesh {
    build(grid, Placement::Average)
}

/// Meshes a chunk with dual contouring from the grid alone: crossings are
/// linearly interpolated and normals are trilinear gradients inside each
/// cell. Cheap, but corners sharper than a cell are only partly recovered;
/// use [`dual_contouring_with_sdf`] for crisp edges.
pub fn dual_contouring(grid: &SampleGrid) -> Mesh {
    build(grid, Placement::Qef(None))
}

/// Meshes a chunk with dual contouring using exact Hermite data from `sdf`
/// (world units, negative in rock; must agree in sign with the grid at the
/// corners): each edge crossing is refined with [`CROSSING_ITERATIONS`]
/// bracketing steps and its normal is the central-difference gradient
/// ([`ls_math::terrain::sdf_gradient`]). `sdf` must be a pure function of the
/// position so neighbouring chunks agree on shared vertices.
pub fn dual_contouring_with_sdf(grid: &SampleGrid, sdf: &dyn Fn(Vec3) -> f32) -> Mesh {
    build(grid, Placement::Qef(Some(sdf)))
}

/// Root-refinement steps per edge crossing in [`dual_contouring_with_sdf`].
pub const CROSSING_ITERATIONS: u32 = 8;

#[derive(Clone, Copy)]
enum Placement<'a> {
    Average,
    Qef(Option<&'a dyn Fn(Vec3) -> f32>),
}

/// Corner offsets; corner index bit 0 = x, bit 1 = y, bit 2 = z.
const CORNERS: [[i32; 3]; 8] = [
    [0, 0, 0],
    [1, 0, 0],
    [0, 1, 0],
    [1, 1, 0],
    [0, 0, 1],
    [1, 0, 1],
    [0, 1, 1],
    [1, 1, 1],
];

/// The 12 cube edges as (lower corner, upper corner) index pairs.
const EDGES: [(usize, usize); 12] = [
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7),
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

/// Regularization weight pulling the QEF solution towards the mass point.
const QEF_WEIGHT: f32 = 0.02;

/// Exact Hermite data (crossing parameter and normal) per grid edge, computed
/// once per edge. Everything is derived from the edge's world position, so a
/// neighbouring chunk computes the same bits for a shared edge.
struct HermiteCache<'a> {
    sdf: &'a dyn Fn(Vec3) -> f32,
    origin: [i32; 3],
    /// Per edge: crossing parameter along +axis from the lower corner, normal.
    data: Vec<(f32, [f32; 3])>,
    known: Vec<bool>,
}

impl HermiteCache<'_> {
    /// Refined crossing (local cell coordinates) and normal on edge `(a, b)`
    /// of `cell`, whose corner values are `va` and `vb`.
    fn get(
        &mut self,
        cell: [i32; 3],
        a: usize,
        b: usize,
        va: f32,
        vb: f32,
    ) -> ([f32; 3], [f32; 3]) {
        let axis = match b - a {
            1 => 0,
            2 => 1,
            _ => 2,
        };
        let lo = CORNERS[a];
        let n = GRID_CORNERS;
        let g = [cell[0] + lo[0], cell[1] + lo[1], cell[2] + lo[2]];
        let idx =
            3 * ((g[0] + 1) as usize + n * ((g[1] + 1) as usize + n * (g[2] + 1) as usize)) + axis;
        if !self.known[idx] {
            let start = Vec3::new(
                (self.origin[0] + g[0]) as f32,
                (self.origin[1] + g[1]) as f32,
                (self.origin[2] + g[2]) as f32,
            );
            let mut dir = [0.0f32; 3];
            dir[axis] = 1.0;
            let dir = Vec3::from_array(dir);
            let t = refine_crossing(|t| (self.sdf)(start + dir * t), va, vb);
            let normal = ls_math::terrain::sdf_gradient(self.sdf, start + dir * t);
            self.data[idx] = (t, normal.to_array());
            self.known[idx] = true;
        }
        let (t, normal) = self.data[idx];
        let mut p = [lo[0] as f32, lo[1] as f32, lo[2] as f32];
        p[axis] += t;
        (p, normal)
    }
}

/// Finds the sign change of `f` on `[0, 1]` (with `f(0) = va`, `f(1) = vb` of
/// opposite signs) by alternating false-position and bisection steps.
fn refine_crossing(f: impl Fn(f32) -> f32, va: f32, vb: f32) -> f32 {
    let (mut t0, mut t1, mut v0, mut v1) = (0.0f32, 1.0f32, va, vb);
    let solid0 = v0 < 0.0;
    for it in 0..CROSSING_ITERATIONS {
        let t = if it % 2 == 0 {
            t0 + (t1 - t0) * (v0 / (v0 - v1))
        } else {
            (t0 + t1) * 0.5
        };
        let v = f(t);
        if v == 0.0 {
            return t;
        }
        if (v < 0.0) == solid0 {
            t0 = t;
            v0 = v;
        } else {
            t1 = t;
            v1 = v;
        }
    }
    t0 + (t1 - t0) * (v0 / (v0 - v1))
}

fn build(grid: &SampleGrid, placement: Placement<'_>) -> Mesh {
    let mut mesh = Mesh::default();
    if grid.is_uniform_sign() {
        return mesh;
    }
    let n = GRID_CELLS;
    let mut vert_index = vec![u32::MAX; n * n * n];
    let cell_idx =
        |i: i32, j: i32, k: i32| (i + 1) as usize + n * ((j + 1) as usize + n * (k + 1) as usize);
    let origin = grid.origin;
    let mut cache = match placement {
        Placement::Qef(Some(sdf)) => {
            let edges = 3 * GRID_CORNERS * GRID_CORNERS * GRID_CORNERS;
            Some(HermiteCache {
                sdf,
                origin,
                data: vec![(0.0, [0.0; 3]); edges],
                known: vec![false; edges],
            })
        }
        _ => None,
    };

    // Pass 1: one vertex per sign-changing cell in -1..32.
    for k in -1..CHUNK_CELLS {
        for j in -1..CHUNK_CELLS {
            for i in -1..CHUNK_CELLS {
                let mut c = [0.0f32; 8];
                let mut mask = 0u8;
                for (ci, off) in CORNERS.iter().enumerate() {
                    let v = grid.value(i + off[0], j + off[1], k + off[2]);
                    c[ci] = v;
                    if v < 0.0 {
                        mask |= 1 << ci;
                    }
                }
                if mask == 0 || mask == 0xff {
                    continue;
                }
                let (local, normal) = match (placement, cache.as_mut()) {
                    (Placement::Qef(_), Some(cache)) => {
                        // Vertex normal: sum of the Hermite normals.
                        let mut sum = [0.0f32; 3];
                        let l = qef_point(&c, |a, b| {
                            let (p, nn) = cache.get([i, j, k], a, b, c[a], c[b]);
                            sum = [sum[0] + nn[0], sum[1] + nn[1], sum[2] + nn[2]];
                            (p, nn)
                        });
                        (l, sum)
                    }
                    (Placement::Qef(_), None) => {
                        let l = qef_point(&c, |a, b| {
                            let p = crossing(&c, a, b);
                            (p, normalize_or_up(trilinear_gradient(&c, p)))
                        });
                        (l, trilinear_gradient(&c, l))
                    }
                    (Placement::Average, _) => {
                        let l = mass_point(&c);
                        (l, trilinear_gradient(&c, l))
                    }
                };
                vert_index[cell_idx(i, j, k)] = mesh.positions.len() as u32;
                mesh.positions.push([
                    (origin[0] + i) as f32 + local[0],
                    (origin[1] + j) as f32 + local[1],
                    (origin[2] + k) as f32 + local[2],
                ]);
                mesh.normals.push(normalize_or_up(normal));
                mesh.teams.push(grid.team(i, j, k));
                mesh.colors.push(grid.color(i, j, k));
                mesh.states.push(grid.states(i, j, k));
            }
        }
    }

    // Pass 2: one quad per sign-changing edge whose lower corner is owned.
    for k in 0..CHUNK_CELLS {
        for j in 0..CHUNK_CELLS {
            for i in 0..CHUNK_CELLS {
                let solid0 = grid.value(i, j, k) < 0.0;
                for axis in 0..3usize {
                    let mut hi = [i, j, k];
                    hi[axis] += 1;
                    let solid1 = grid.value(hi[0], hi[1], hi[2]) < 0.0;
                    if solid0 == solid1 {
                        continue;
                    }
                    let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
                    let base = [i, j, k];
                    let cell = |db: i32, dc: i32| {
                        let mut p = base;
                        p[b] += db;
                        p[c] += dc;
                        vert_index[cell_idx(p[0], p[1], p[2])]
                    };
                    // Counter-clockwise when seen from +axis (b x c = axis).
                    let q = [cell(-1, -1), cell(0, -1), cell(0, 0), cell(-1, 0)];
                    debug_assert!(q.iter().all(|&v| v != u32::MAX));
                    if solid0 {
                        // Open space on the +axis side.
                        mesh.indices
                            .extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
                    } else {
                        mesh.indices
                            .extend_from_slice(&[q[0], q[2], q[1], q[0], q[3], q[2]]);
                    }
                }
            }
        }
    }
    mesh
}

/// Crossing point on the edge between corners `a` and `b` (local cell coords).
#[inline]
fn crossing(c: &[f32; 8], a: usize, b: usize) -> [f32; 3] {
    let (va, vb) = (c[a], c[b]);
    let t = va / (va - vb);
    let (pa, pb) = (CORNERS[a], CORNERS[b]);
    [
        pa[0] as f32 + (pb[0] - pa[0]) as f32 * t,
        pa[1] as f32 + (pb[1] - pa[1]) as f32 * t,
        pa[2] as f32 + (pb[2] - pa[2]) as f32 * t,
    ]
}

#[inline]
fn is_crossing(c: &[f32; 8], a: usize, b: usize) -> bool {
    (c[a] < 0.0) != (c[b] < 0.0)
}

/// Average of the edge crossing points (surface nets vertex).
fn mass_point(c: &[f32; 8]) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut count = 0.0f32;
    for &(a, b) in &EDGES {
        if is_crossing(c, a, b) {
            let p = crossing(c, a, b);
            sum[0] += p[0];
            sum[1] += p[1];
            sum[2] += p[2];
            count += 1.0;
        }
    }
    [sum[0] / count, sum[1] / count, sum[2] / count]
}

/// Dual contouring vertex: minimizes `sum (n_i . (x - p_i))^2 + w |x - m|^2`
/// over the Hermite samples `(p_i, n_i) = hermite(a, b)` of the crossing
/// edges, where `m` is their mass point, then clamps the result into the cell.
fn qef_point(
    c: &[f32; 8],
    mut hermite: impl FnMut(usize, usize) -> ([f32; 3], [f32; 3]),
) -> [f32; 3] {
    let mut samples = [([0.0f32; 3], [0.0f32; 3]); 12];
    let mut count = 0;
    let mut m = [0.0f32; 3];
    for &(a, b) in &EDGES {
        if is_crossing(c, a, b) {
            let s = hermite(a, b);
            m = [m[0] + s.0[0], m[1] + s.0[1], m[2] + s.0[2]];
            samples[count] = s;
            count += 1;
        }
    }
    let inv = 1.0 / count as f32;
    let m = [m[0] * inv, m[1] * inv, m[2] * inv];
    // Normal equations relative to the mass point: (A^T A + w I) y = A^T b.
    let mut ata = [[0.0f32; 3]; 3];
    let mut atb = [0.0f32; 3];
    for &(p, nrm) in &samples[..count] {
        let d = nrm[0] * (p[0] - m[0]) + nrm[1] * (p[1] - m[1]) + nrm[2] * (p[2] - m[2]);
        for r in 0..3 {
            for col in 0..3 {
                ata[r][col] += nrm[r] * nrm[col];
            }
            atb[r] += nrm[r] * d;
        }
    }
    for (r, row) in ata.iter_mut().enumerate() {
        row[r] += QEF_WEIGHT;
    }
    let y = solve3(&ata, &atb).unwrap_or([0.0; 3]);
    [
        (m[0] + y[0]).clamp(0.0, 1.0),
        (m[1] + y[1]).clamp(0.0, 1.0),
        (m[2] + y[2]).clamp(0.0, 1.0),
    ]
}

/// Solves a 3x3 linear system with Cramer's rule (plain arithmetic only).
fn solve3(a: &[[f32; 3]; 3], b: &[f32; 3]) -> Option<[f32; 3]> {
    let det3 = |m: &[[f32; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det3(a);
    if d.abs() < 1e-12 || !d.is_finite() {
        return None;
    }
    let mut out = [0.0f32; 3];
    for (col, o) in out.iter_mut().enumerate() {
        let mut m = *a;
        for r in 0..3 {
            m[r][col] = b[r];
        }
        *o = det3(&m) / d;
    }
    Some(out)
}

/// Gradient of the trilinear interpolant of the cell at local position `p`.
fn trilinear_gradient(c: &[f32; 8], p: [f32; 3]) -> [f32; 3] {
    let (u, v, w) = (p[0], p[1], p[2]);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let gx = lerp(
        lerp(c[1] - c[0], c[3] - c[2], v),
        lerp(c[5] - c[4], c[7] - c[6], v),
        w,
    );
    let gy = lerp(
        lerp(c[2] - c[0], c[3] - c[1], u),
        lerp(c[6] - c[4], c[7] - c[5], u),
        w,
    );
    let gz = lerp(
        lerp(c[4] - c[0], c[5] - c[1], u),
        lerp(c[6] - c[2], c[7] - c[3], u),
        v,
    );
    [gx, gy, gz]
}

fn normalize_or_up(g: [f32; 3]) -> [f32; 3] {
    let len2 = g[0] * g[0] + g[1] * g[1] + g[2] * g[2];
    if len2 > 1e-24 && len2.is_finite() {
        let len = ls_math::det::sqrt(len2);
        [g[0] / len, g[1] / len, g[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn sphere_grid(origin: [i32; 3], center: Vec3, r: f32) -> SampleGrid {
        // Rock inside the sphere (negative inside).
        SampleGrid::from_fn(origin, |p| p.distance(center) - r)
    }

    fn tri_normal(m: &Mesh, t: usize) -> Vec3 {
        let p = |i: u32| Vec3::from_array(m.positions[i as usize]);
        let (a, b, c) = (
            p(m.indices[3 * t]),
            p(m.indices[3 * t + 1]),
            p(m.indices[3 * t + 2]),
        );
        (b - a).cross(c - a)
    }

    fn check_sphere(mesher: fn(&SampleGrid) -> Mesh) {
        let center = Vec3::new(16.3, 15.7, 16.1);
        let r = 9.4;
        let m = mesher(&sphere_grid([0, 0, 0], center, r));
        assert!(m.triangle_count() > 500);
        assert_eq!(m.positions.len(), m.normals.len());
        assert_eq!(m.positions.len(), m.teams.len());
        assert_eq!(m.positions.len(), m.colors.len());
        for (p, n) in m.positions.iter().zip(&m.normals) {
            let p = Vec3::from_array(*p);
            assert!((p.distance(center) - r).abs() < 0.1, "{p:?}");
            let n = Vec3::from_array(*n);
            assert!((n.length() - 1.0).abs() < 1e-4);
            // Rock inside: normals point outwards.
            assert!(n.dot((p - center).normalize()) > 0.95);
        }
        // Every triangle faces outwards (towards open space).
        for t in 0..m.triangle_count() {
            let a = Vec3::from_array(m.positions[m.indices[3 * t] as usize]);
            assert!(tri_normal(&m, t).dot(a - center) > 0.0, "triangle {t}");
        }
        // Closed surface: every edge is shared by exactly two triangles.
        let mut edges = std::collections::BTreeMap::new();
        for t in 0..m.triangle_count() {
            for e in 0..3 {
                let a = m.indices[3 * t + e];
                let b = m.indices[3 * t + (e + 1) % 3];
                *edges.entry((a.min(b), a.max(b))).or_insert(0u32) += 1;
            }
        }
        assert!(edges.values().all(|&c| c == 2));
    }

    #[test]
    fn surface_nets_sphere() {
        check_sphere(surface_nets);
    }

    #[test]
    fn dual_contouring_sphere() {
        check_sphere(dual_contouring);
    }

    #[test]
    fn uniform_grid_is_empty() {
        let g = SampleGrid::from_fn([0, 0, 0], |_| 3.0);
        assert!(surface_nets(&g).is_empty());
        assert!(dual_contouring(&g).is_empty());
        let g = SampleGrid::from_fn([0, 0, 0], |_| -3.0);
        assert!(surface_nets(&g).is_empty());
    }

    /// Two neighbouring chunks share their border vertices exactly.
    #[test]
    fn seam_free_across_chunks() {
        let center = Vec3::new(32.2, 20.4, 16.7);
        let f = |p: Vec3| p.distance(center) - 11.3;
        let exact_dc = |g: &SampleGrid| dual_contouring_with_sdf(g, &f);
        let meshers: [&dyn Fn(&SampleGrid) -> Mesh; 3] =
            [&surface_nets, &dual_contouring, &exact_dc];
        for mesher in meshers {
            let a = mesher(&SampleGrid::from_fn([0, 0, 0], f));
            let b = mesher(&SampleGrid::from_fn([32, 0, 0], f));
            // Positions each chunk actually references at the shared plane x in (31, 33).
            let border = |m: &Mesh| {
                let mut v: Vec<[u32; 3]> = m
                    .indices
                    .iter()
                    .map(|&i| m.positions[i as usize])
                    .filter(|p| p[0] > 31.0 && p[0] < 33.0)
                    .map(|p| p.map(f32::to_bits))
                    .collect();
                v.sort();
                v.dedup();
                v
            };
            let (ba, bb) = (border(&a), border(&b));
            assert!(!ba.is_empty());
            // Vertices B uses from its cell -1 are A's cell-31 vertices, bit for bit.
            let all_a: std::collections::BTreeSet<[u32; 3]> =
                a.positions.iter().map(|p| p.map(f32::to_bits)).collect();
            let shared: Vec<_> = bb.iter().filter(|p| f32::from_bits(p[0]) < 32.0).collect();
            assert!(!shared.is_empty());
            for p in shared {
                assert!(all_a.contains(p), "vertex {p:?} missing in neighbour");
            }
            // Union of both meshes is closed: every edge used exactly twice.
            let mut edges = std::collections::BTreeMap::new();
            for m in [&a, &b] {
                for t in 0..m.triangle_count() {
                    for e in 0..3 {
                        let pa = m.positions[m.indices[3 * t + e] as usize].map(f32::to_bits);
                        let pb =
                            m.positions[m.indices[3 * t + (e + 1) % 3] as usize].map(f32::to_bits);
                        let key = if pa < pb { (pa, pb) } else { (pb, pa) };
                        *edges.entry(key).or_insert(0u32) += 1;
                    }
                }
            }
            assert!(edges.values().all(|&c| c == 2), "open seam");
        }
    }

    /// Dual contouring reproduces a sharp box corner much better than surface nets.
    #[test]
    fn dual_contouring_keeps_sharp_corners() {
        let corner = Vec3::new(20.3, 18.6, 21.4);
        // Rock is the box x,y,z < corner (inside the octant); exact box SDF.
        let lo = Vec3::splat(-100.0);
        let rock = ls_math::Aabb::new(lo, corner);
        let f = |p: Vec3| rock.sdf(p);
        let grid = SampleGrid::from_fn([0, 0, 0], f);
        let nearest = |m: &Mesh| {
            m.positions
                .iter()
                .map(|p| Vec3::from_array(*p).distance(corner))
                .fold(f32::INFINITY, f32::min)
        };
        let sn = nearest(&surface_nets(&grid));
        let dc_grid = nearest(&dual_contouring(&grid));
        let m = dual_contouring_with_sdf(&grid, &f);
        let dc = nearest(&m);
        assert!(dc < 0.05, "dual contouring corner error {dc}");
        assert!(dc < sn * 0.2, "dc {dc} vs sn {sn}");
        assert!(dc_grid <= sn, "grid-only dc {dc_grid} vs sn {sn}");
        // Flat faces stay flat: vertices near the x face lie on it.
        for p in &m.positions {
            let p = Vec3::from_array(*p);
            if p.y < corner.y - 2.0 && p.z < corner.z - 2.0 && p.x > corner.x - 1.0 {
                assert!((p.x - corner.x).abs() < 1e-3, "{p:?}");
            }
        }
    }

    #[test]
    fn solve3_identity() {
        let a = [[2.0, 0.0, 0.0], [0.0, 4.0, 0.0], [0.0, 0.0, 8.0]];
        assert_eq!(solve3(&a, &[2.0, 4.0, 8.0]), Some([1.0, 1.0, 1.0]));
        assert_eq!(solve3(&[[0.0; 3]; 3], &[1.0, 1.0, 1.0]), None);
    }
}
