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
//! - **Surface nets** averages the edge crossing points (smooth, fast), then
//!   refines each vertex (see [`surface_nets`]): one Newton step onto the
//!   surface, a continuous normal and baked ambient occlusion.
//! - **Dual contouring** minimizes a quadratic error function (QEF) built
//!   from the crossing points and their normals, which keeps sharp edges and
//!   corners (mine shafts, faceted rocks).
//!
//! Winding is counter-clockwise when seen from open space; normals point
//! into open space (towards increasing SDF). Positions are in world units.

use sha2::{Digest, Sha256};

use ls_math::Vec3;

use crate::grid::{SampleGrid, GRID_CELLS, GRID_CORNERS, GRID_HALO, HALO_CORNERS};
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
    /// Display blend weights of weak and hard rock per vertex (0..255,
    /// [`crate::MaterialRegion::blend_weights`]): smooth across material
    /// borders and equal on vertices two chunks share. Filled with
    /// `materials`; empty otherwise. Client visuals only (not hashed).
    pub material_weights: Vec<[u8; 2]>,
    /// Baked ambient occlusion per vertex (255 = fully open, 0 = fully
    /// occluded; see [`surface_nets`]). Empty for dual-contouring meshes.
    pub ao: Vec<u8>,
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

    /// SHA-256 over the geometry, paint, surface states and ambient
    /// occlusion (little-endian bit patterns), for golden tests. States and
    /// paint colours are hashed only when a vertex has one, and occlusion
    /// only when the mesh carries it, so meshes without them hash as they
    /// did before they existed. Materials are a pure function of the vertex
    /// positions and the seed and are not hashed.
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
        if !self.ao.is_empty() {
            h.update(b"ao");
            h.update(&self.ao);
        }
        h.finalize().into()
    }
}

/// Meshes a chunk with surface nets and refines every vertex from the grid
/// alone (see [`surface_nets_with_sdf`] for the variant the game uses):
///
/// 1. **Snap**: the mass point `v` of the edge crossings (which lies up to a
///    few tenths of a unit inside curved rock) moves by one Newton step
///    `v -= d(v) g / |g|²` (at most [`SNAP_MAX_STEP`]) towards the surface,
///    with `g` the smooth gradient below and `d` the tricubic
///    (Catmull-Rom) interpolant of the grid. Where that disagrees with the
///    trilinear interpolant by [`SNAP_KINK`] or more (a crease or a feature
///    finer than a cell, where the grid cannot be trusted), `d` is the exact
///    field ([`surface_nets_with_sdf`]) or, from the grid alone, the
///    trilinear value (the vertex barely moves).
/// 2. **Normal**: the smooth gradient `g` at the snapped vertex, the
///    trilinear interpolation of central-difference gradients at the grid
///    corners. Unlike the gradient of one cell's interpolant it is
///    continuous across cells and chunks, so smooth rock shades smoothly.
/// 3. **Ambient occlusion** (one byte per vertex, 255 = open): at the
///    [`AO_PROBES`] along the normal the trilinear field (scaled to a
///    distance by `|g|`) is compared with the distance an open half-space
///    would give (classic SDF occlusion: flat rock stays at 255, creases,
///    pockets and narrow tunnels darken).
///
/// Every step reads the grid by world position, so a vertex shared by two
/// chunks gets the same bits in both. Square roots use `f32::sqrt`
/// (correctly rounded, the same bits as `ls_math::det::sqrt`, which is a
/// software routine on wasm32).
pub fn surface_nets(grid: &SampleGrid) -> Mesh {
    build(grid, Placement::Smooth(None), None)
}

/// [`surface_nets`] with the exact field `sdf` for the snap at creases
/// (world units, negative in rock; must agree with the grid at the corners
/// and be a pure function of the position, so neighbouring chunks agree).
/// `tests/mesh_quality.rs`: in cavern worlds the vertices lie a median
/// 0.003 u (p95 0.07 u) from the collision surface, was 0.012 u (p95 0.2 u)
/// with plain surface nets; about a quarter of the vertices need `sdf`.
pub fn surface_nets_with_sdf(grid: &SampleGrid, sdf: &dyn Fn(Vec3) -> f32) -> Mesh {
    build(grid, Placement::Smooth(Some(sdf)), None)
}

/// [`surface_nets_with_sdf`] with the thin-feature pass ([`THIN_OPEN`])
/// only at grid corners where `thin_allowed` holds; elsewhere the cells
/// mesh as plain [`surface_nets_with_sdf`] without thin features. Where
/// `thin_allowed` holds for every corner the mesh is bit-identical to
/// [`surface_nets_with_sdf`]. The game's Squeeze shell turns the pass off
/// next to its moving wall: the slivers of open space between the wall
/// and the rock are thin features that are about to be crushed, and fitting
/// them made shell remeshing about 40 % slower at the 99th percentile.
pub fn surface_nets_with_sdf_thin_where(
    grid: &SampleGrid,
    sdf: &dyn Fn(Vec3) -> f32,
    thin_allowed: &dyn Fn(Vec3) -> bool,
) -> Mesh {
    build(grid, Placement::Smooth(Some(sdf)), Some(thin_allowed))
}

/// Largest distance (u) the surface-nets snap moves a vertex.
pub const SNAP_MAX_STEP: f32 = 0.5;
/// Where the tricubic and trilinear interpolants differ by this much or
/// more (as a distance, u) at a vertex, the snap reads the exact field.
pub const SNAP_KINK: f32 = 0.1;
/// Ambient-occlusion probes along the normal: `(distance (u), weight)`;
/// the weights sum to 1.
pub const AO_PROBES: [(f32, f32); 3] = [(0.6, 0.3), (1.4, 0.35), (2.5, 0.35)];
/// Occlusion strength: `ao = 1 - AO_STRENGTH * occlusion`, clamped to 0..1.
pub const AO_STRENGTH: f32 = 1.6;
/// Lower bound on `|g|` when field values are scaled to distances (as in
/// `ls_sim::terrain::surface_estimate`).
const MIN_GRADIENT: f32 = 0.2;

/// Meshes a chunk with dual contouring from the grid alone: crossings are
/// linearly interpolated and normals are trilinear gradients inside each
/// cell. Cheap, but corners sharper than a cell are only partly recovered;
/// use [`dual_contouring_with_sdf`] for crisp edges.
pub fn dual_contouring(grid: &SampleGrid) -> Mesh {
    build(grid, Placement::Qef(None), None)
}

/// Meshes a chunk with dual contouring using exact Hermite data from `sdf`
/// (world units, negative in rock; must agree in sign with the grid at the
/// corners): each edge crossing is refined with [`CROSSING_ITERATIONS`]
/// bracketing steps and its normal is the central-difference gradient
/// ([`ls_math::terrain::sdf_gradient`]). `sdf` must be a pure function of the
/// position so neighbouring chunks agree on shared vertices.
pub fn dual_contouring_with_sdf(grid: &SampleGrid, sdf: &dyn Fn(Vec3) -> f32) -> Mesh {
    build(grid, Placement::Qef(Some(sdf)), None)
}

/// Root-refinement steps per edge crossing in [`dual_contouring_with_sdf`].
pub const CROSSING_ITERATIONS: u32 = 8;

#[derive(Clone, Copy)]
enum Placement<'a> {
    /// Surface nets; the snap distance comes from the grid (`None`) or `sdf`.
    Smooth(Option<&'a dyn Fn(Vec3) -> f32>),
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

fn build(
    grid: &SampleGrid,
    placement: Placement<'_>,
    thin_allowed: Option<&dyn Fn(Vec3) -> bool>,
) -> Mesh {
    let mut mesh = Mesh::default();
    // With the exact field, rock corners next to open space finer than a
    // cell count as open (`thin_air`), so thin bores get vertices
    // all around (placed on the true surface by `fit_thin`), even in a
    // chunk whose corners are all rock.
    let thin = match placement {
        Placement::Smooth(Some(sdf)) => thin_air(grid, sdf, thin_allowed),
        _ => None,
    };
    let promoted = thin.as_ref().and_then(|t| t.values.as_deref());
    if promoted.is_none() && grid.is_uniform_sign() {
        return mesh;
    }
    let n = GRID_CELLS;
    let field = Field::new(grid);
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

    // Corner row `(j, k)` for x in `-1..=32` (contiguous in the halo array).
    let values = promoted.unwrap_or(grid.values());
    let row = |j: i32, k: i32| {
        let start = corner_index(-1, j, k);
        &values[start..start + GRID_CORNERS]
    };
    // Pass 1: one vertex per sign-changing cell in -1..32.
    for k in -1..CHUNK_CELLS {
        for j in -1..CHUNK_CELLS {
            let rows = [row(j, k), row(j + 1, k), row(j, k + 1), row(j + 1, k + 1)];
            // Rock bits of the four corners of each x column.
            let mut column = [0u8; GRID_CORNERS];
            for (x, col) in column.iter_mut().enumerate() {
                *col = (rows[0][x] < 0.0) as u8
                    | ((rows[1][x] < 0.0) as u8) << 1
                    | ((rows[2][x] < 0.0) as u8) << 2
                    | ((rows[3][x] < 0.0) as u8) << 3;
            }
            for i in -1..CHUNK_CELLS {
                let x = (i + 1) as usize;
                let cols = column[x] | column[x + 1] << 4;
                if cols == 0 || cols == 0xff {
                    continue;
                }
                let c = [
                    rows[0][x],
                    rows[0][x + 1],
                    rows[1][x],
                    rows[1][x + 1],
                    rows[2][x],
                    rows[2][x + 1],
                    rows[3][x],
                    rows[3][x + 1],
                ];
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
                    // Refined below; the cell's gradient is only a fallback.
                    (Placement::Smooth(_), _) => (mass_point(&c), [0.0; 3]),
                };
                vert_index[cell_idx(i, j, k)] = mesh.positions.len() as u32;
                let world = [
                    (origin[0] + i) as f32 + local[0],
                    (origin[1] + j) as f32 + local[1],
                    (origin[2] + k) as f32 + local[2],
                ];
                if let Placement::Smooth(sdf) = placement {
                    let forced = thin.as_ref().map_or(0, |t| {
                        let b = corner_index(i, j, k);
                        [0, 1, SY, SY + 1, SZ, SZ + 1, SZ + SY, SZ + SY + 1]
                            .iter()
                            .fold(0, |m, &o| m.max(t.corners[b + o]))
                    });
                    let v =
                        refine_vertex(&field, world, sdf, forced, || trilinear_gradient(&c, local));
                    mesh.positions.push(v.position);
                    mesh.normals.push(v.normal);
                    mesh.ao.push(v.ao);
                } else {
                    mesh.positions.push(world);
                    mesh.normals.push(normalize_or_up(normal));
                }
                mesh.teams.push(grid.team(i, j, k));
                mesh.colors.push(grid.color(i, j, k));
                mesh.states.push(grid.states(i, j, k));
            }
        }
    }

    // Pass 2: one quad per sign-changing edge whose lower corner is owned.
    for k in 0..CHUNK_CELLS {
        for j in 0..CHUNK_CELLS {
            let (r0, ry, rz) = (row(j, k), row(j + 1, k), row(j, k + 1));
            for i in 0..CHUNK_CELLS {
                let x = (i + 1) as usize;
                let solid0 = r0[x] < 0.0;
                let ends = [r0[x + 1] < 0.0, ry[x] < 0.0, rz[x] < 0.0];
                if ends == [solid0; 3] {
                    continue;
                }
                for (axis, &solid1) in ends.iter().enumerate() {
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
                    // Counter-clockwise seen from open space (on the +axis
                    // side when the lower corner is rock).
                    let quad = if solid0 { q } else { [q[0], q[3], q[2], q[1]] };
                    mesh.indices.extend_from_slice(&split_quad(&mesh, quad));
                }
            }
        }
    }
    mesh
}

/// The two triangles of a surface-nets quad `[a, b, c, d]` (counter-
/// clockwise seen from open space). Split along `a-c` unless one of those
/// triangles faces into the rock (its winding against its vertex normals:
/// a folded quad, culled as a hole on screen) and the `b-d` split folds
/// less: the one whose worse triangle agrees better with its normals wins.
fn split_quad(mesh: &Mesh, [a, b, c, d]: [u32; 4]) -> [u32; 6] {
    let p = |i: u32| Vec3::from_array(mesh.positions[i as usize]);
    let n = |i: u32| Vec3::from_array(mesh.normals[i as usize]);
    // A triangle's winding normal and its summed vertex normals.
    let tri = |i: u32, j: u32, k: u32| {
        let (pi, pj, pk) = (p(i), p(j), p(k));
        ((pj - pi).cross(pk - pi), n(i) + n(j) + n(k))
    };
    let ac = [a, b, c, a, c, d];
    let (abc, acd) = (tri(a, b, c), tri(a, c, d));
    // The common case (no fold) needs no square roots.
    if abc.0.dot(abc.1) > 0.0 && acd.0.dot(acd.1) > 0.0 {
        return ac;
    }
    // Cosine between the two (0 for a degenerate triangle).
    let facing = |(f, m): (Vec3, Vec3)| {
        let l2 = f.length_squared() * m.length_squared();
        if l2 > 1e-24 {
            f.dot(m) / l2.sqrt()
        } else {
            0.0
        }
    };
    let bd = facing(tri(a, b, d)).min(facing(tri(b, c, d)));
    if bd > facing(abc).min(facing(acd)) {
        [a, b, d, b, c, d]
    } else {
        ac
    }
}

/// A refined surface-nets vertex.
struct Refined {
    position: [f32; 3],
    normal: [f32; 3],
    ao: u8,
}

/// Snap, normal and occlusion of the surface-nets vertex at world position
/// `v` (see [`surface_nets`]); `cell_gradient` gives the gradient of the
/// cell's own trilinear interpolant, used only where the smooth gradient
/// vanishes.
fn refine_vertex(
    field: &Field<'_>,
    v: [f32; 3],
    sdf: Option<&dyn Fn(Vec3) -> f32>,
    thin: u8,
    cell_gradient: impl FnOnce() -> [f32; 3],
) -> Refined {
    let v0 = Vec3::from_array(v);
    let (cell0, t0) = field.locate(v0, 1);
    let grads = field.corner_gradients(cell0);
    let g0 = Field::interpolate(&grads, t0);
    let g2 = g0.dot(g0);
    let mut p = v0;
    if g2 > 1e-12 && g2.is_finite() {
        // The tricubic interpolant follows the field's curvature between
        // the corners; where it disagrees with the trilinear one the field
        // has a crease or a feature finer than a cell, and only the exact
        // field (or, from the grid alone, no snap) is trusted.
        let cubic = field.cubic(cell0, t0);
        let linear = trilinear(field.values, cell0, t0);
        let d = if (cubic - linear).abs() < SNAP_KINK * g2.sqrt() {
            cubic
        } else {
            match sdf {
                Some(f) => f(v0),
                None => linear,
            }
        };
        let mut step = g0 * (d / g2);
        let len2 = step.dot(step);
        if len2 > SNAP_MAX_STEP * SNAP_MAX_STEP {
            step *= SNAP_MAX_STEP / len2.sqrt();
        }
        if step.x.is_finite() && step.y.is_finite() && step.z.is_finite() {
            p = v0 - step;
        }
    }
    // Thin features (bores and pockets about a cell wide or narrower, tight
    // inside corners): the grid cannot see them, so the exact field places
    // the vertex and gives its normal, and the vertex moves into the rock by
    // the chord sag, so the drawn faces sit on the surface instead of inside
    // the open space.
    if let Some(f) = sdf {
        // Next to a promoted corner the grid's mass point means nothing, so
        // the vertex goes onto the exact surface even at a crease.
        let fitted = match thin {
            THIN_NONE => None,
            THIN_RIDGE => fit_thin(f, v0),
            _ => fit_thin(f, v0).or_else(|| exact_snap(f, v0)),
        };
        if let Some((q, n)) = fitted {
            return Refined {
                position: q.to_array(),
                normal: n.to_array(),
                ao: exact_ambient_occlusion(f, q, n),
            };
        }
    }
    let (cell, t) = field.locate(p, 1);
    let g = if cell == cell0 {
        Field::interpolate(&grads, t)
    } else {
        Field::interpolate(&field.corner_gradients(cell), t)
    };
    let glen2 = g.dot(g);
    let (n, glen) = if glen2 > 1e-24 && glen2.is_finite() {
        let l = glen2.sqrt();
        (g * (1.0 / l), l)
    } else {
        (Vec3::from_array(normalize_or_up(cell_gradient())), 0.0)
    };
    Refined {
        position: p.to_array(),
        normal: n.to_array(),
        ao: field.ambient_occlusion(p, n, glen.max(MIN_GRADIENT)),
    }
}

/// Thin open features of a chunk ([`thin_air`]).
struct ThinAir {
    /// Corner values with the promoted rock corners, if any.
    values: Option<Vec<f32>>,
    /// Per halo corner: [`THIN_RIDGE`] for an open ridge corner of a thin
    /// feature, [`THIN_PROMOTED`] for a promoted rock corner, else
    /// [`THIN_NONE`] (the cells around a marked corner place their vertices
    /// with [`fit_thin`]).
    corners: Vec<u8>,
}

const THIN_NONE: u8 = 0;
const THIN_RIDGE: u8 = 1;
const THIN_PROMOTED: u8 = 2;

/// One Newton step pair on the exact field `f` from `v0` (no slope or
/// crease checks): the point and its unit normal.
fn exact_snap(f: &dyn Fn(Vec3) -> f32, v0: Vec3) -> Option<(Vec3, Vec3)> {
    let mut q = v0;
    let mut g = Vec3::ZERO;
    for _ in 0..THIN_NEWTON_STEPS {
        let (d, grad) = value_gradient(f, q, THIN_PROBE);
        let g2 = grad.dot(grad);
        if !(g2 > 1e-6 && g2.is_finite()) {
            return None;
        }
        g = grad;
        let mut step = grad * (d / g2);
        let len2 = step.dot(step);
        if len2 > SNAP_MAX_STEP * SNAP_MAX_STEP {
            step *= SNAP_MAX_STEP / len2.sqrt();
        }
        q -= step;
    }
    let ok = q.x.is_finite() && q.y.is_finite() && q.z.is_finite();
    ok.then(|| (q, g.normalize()))
}

/// [`Field::ambient_occlusion`] on the exact field (thin features, which the
/// grid does not see; the field is a true distance there).
fn exact_ambient_occlusion(f: &dyn Fn(Vec3) -> f32, p: Vec3, n: Vec3) -> u8 {
    let mut occlusion = 0.0f32;
    for (h, w) in AO_PROBES {
        let d = f(p + n * h);
        occlusion += w * ((h - d) / h).clamp(0.0, 1.0);
    }
    let ao = (1.0 - AO_STRENGTH * occlusion).clamp(0.0, 1.0);
    (ao * 255.0 + 0.5) as u8
}

/// Open corners up to this value (field units, about the radius of the
/// widest bore treated as thin) can mark a thin feature.
pub const THIN_OPEN: f32 = 0.75;

/// Corner value of a rock corner promoted to open space by
/// [`thin_air`] (just above zero, so edge crossings sit at it).
const PROMOTED: f32 = 1.0e-4;
/// Rock corners at most this deep (field units) can be promoted.
pub const PROMOTE_DEPTH: f32 = 0.5;
/// Least drop (field units) to the lower neighbour for a ridge direction
/// of [`thin_air`].
pub const RIDGE_DROP: f32 = 0.45;

/// Surface nets sees open space only through corners whose value is
/// positive, so a bore narrower than a cell between two corner rows
/// vanishes, and one that touches a single row of corners is drawn as a
/// square of four vertices around it (much narrower than the bore).
///
/// Thin open features are found on the grid as ridges: open corners below
/// [`THIN_OPEN`] that peak in at least two axis directions (see `ridge`),
/// or rock corners that do (a bore between the corner rows). Every rock
/// corner within [`PROMOTE_DEPTH`] of the surface that is such a ridge or
/// next to an open one, and whose box of half a cell holds thin open space
/// on the exact field (`open_within_box`: a true distance field, the
/// surface inside the box, open space beyond and rock again
/// [`THIN_ACROSS`] further), counts as open for the topology. The cells
/// around marked corners place their vertices with [`fit_thin`] on the
/// true surface. Walls, room corners, large tunnels and the bound cavern
/// fields never qualify, and thin rock is never removed. Returns `None` if
/// no corner is marked. Decisions read corners by world position (within
/// the halo) and the exact field, so neighbouring chunks agree.
fn thin_air(
    grid: &SampleGrid,
    f: &dyn Fn(Vec3) -> f32,
    allowed: Option<&dyn Fn(Vec3) -> bool>,
) -> Option<ThinAir> {
    let values = grid.values();
    let origin = grid.origin;
    let corner_pos = |i: i32, j: i32, k: i32| {
        Vec3::new(
            (origin[0] + i) as f32,
            (origin[1] + j) as f32,
            (origin[2] + k) as f32,
        )
    };
    let allowed_at = |i: i32, j: i32, k: i32| allowed.is_none_or(|a| a(corner_pos(i, j, k)));
    // A ridge of the field in at least two axis directions: on each, the
    // corner is at least as open as both neighbours and clearly more open
    // than one (a bore's axis). An open corner must also be strictly more
    // open than both neighbours on one of them: an inside corner of a room
    // ties with its neighbours along the walls on every axis. A rock corner
    // may tie on all (the four corners around a bore through cell centres
    // have the same value); rock along a wall or a crease is never a ridge.
    let ridge = |idx: usize| {
        let d = values[idx];
        let (mut axes, mut strict) = (0, false);
        for s in [1, SY, SZ] {
            let (a, b) = (values[idx - s], values[idx + s]);
            let top = a.max(b);

            if d >= top && d - a.min(b) >= RIDGE_DROP {
                axes += 1;
                strict |= d > top;
            }
        }
        axes >= 2 && (strict || d < 0.0)
    };
    // Open ridge corners (thin open features the grid sees) for the corners
    // the cells read (-1..=32) and their neighbours.
    let mut open_ridge = vec![THIN_NONE; values.len()];
    let mut any = false;
    for k in -2..=CHUNK_CELLS + 1 {
        for j in -2..=CHUNK_CELLS + 1 {
            for i in -2..=CHUNK_CELLS + 1 {
                let idx = corner_index(i, j, k);
                let d = values[idx];
                if d > 0.0 && d < THIN_OPEN && ridge(idx) && allowed_at(i, j, k) {
                    open_ridge[idx] = THIN_RIDGE;
                    any = true;
                }
            }
        }
    }
    let mut promoted: Option<Vec<f32>> = None;
    for k in -1..=CHUNK_CELLS {
        for j in -1..=CHUNK_CELLS {
            for i in -1..=CHUNK_CELLS {
                let idx = corner_index(i, j, k);
                let d = values[idx];
                if !(d < 0.0 && d > -PROMOTE_DEPTH) {
                    continue;
                }
                let thin = [idx - 1, idx + 1, idx - SY, idx + SY, idx - SZ, idx + SZ]
                    .iter()
                    .any(|&n| open_ridge[n] == THIN_RIDGE)
                    || ridge(idx);
                if !thin || !allowed_at(i, j, k) {
                    continue;
                }
                let corner = corner_pos(i, j, k);
                if open_within_box(f, corner) {
                    promoted.get_or_insert_with(|| values.to_vec())[idx] = PROMOTED;
                    open_ridge[idx] = THIN_PROMOTED;
                    any = true;
                }
            }
        }
    }
    any.then_some(ThinAir {
        values: promoted,
        corners: open_ridge,
    })
}

/// Whether the exact field `f` has open space within the box of half a
/// cell around `corner` (rock there): Newton steps from the corner reach
/// the surface inside the box, and the field is positive just beyond.
fn open_within_box(f: &dyn Fn(Vec3) -> f32, corner: Vec3) -> bool {
    let Some((q, n)) = surface_point(f, corner) else {
        return false;
    };
    let o = q - corner;
    // Open space just beyond the surface, and rock again across it: a thin
    // feature, not the open side of an ordinary wall next to a bore's mouth.
    o.x.abs().max(o.y.abs()).max(o.z.abs()) <= 0.5
        && f(q + n * 0.05) > 0.0
        && f(q + n * THIN_ACROSS) < 0.0
}

/// Newton steps on the exact field `f` from `start` to the surface: the
/// point and the unit normal there, or `None` where the field is not a
/// true distance ([`EXACT_SLOPE`]) or the point lies on a crease (the
/// gradient there is shorter than on the way: two surfaces meet, e.g. at
/// the mouth of a bore, where the grid's own snap is kept).
fn surface_point(f: &dyn Fn(Vec3) -> f32, start: Vec3) -> Option<(Vec3, Vec3)> {
    let mut q = start;
    let mut slope = 0.0f32;
    for _ in 0..THIN_NEWTON_STEPS {
        let (d, g) = value_gradient(f, q, THIN_PROBE);
        let g2 = g.dot(g);
        if !(g2 >= EXACT_SLOPE * EXACT_SLOPE && g2.is_finite()) {
            return None;
        }
        slope = slope.max(g2.sqrt());
        let mut step = g * (d / g2);
        let len2 = step.dot(step);
        if len2 > SNAP_MAX_STEP * SNAP_MAX_STEP {
            step *= SNAP_MAX_STEP / len2.sqrt();
        }
        q -= step;
    }
    let (d, g) = value_gradient(f, q, THIN_PROBE);
    let glen = g.length();
    let ok = q.x.is_finite()
        && q.y.is_finite()
        && q.z.is_finite()
        && d.abs() < 0.02
        && glen >= CREASE_SLOPE * slope;
    ok.then(|| (q, g * (1.0 / glen)))
}

/// Value and central-difference gradient of `f` at `p` (step `e`).
fn value_gradient(f: &dyn Fn(Vec3) -> f32, p: Vec3, e: f32) -> (f32, Vec3) {
    let g = Vec3::new(
        f(p + Vec3::X * e) - f(p - Vec3::X * e),
        f(p + Vec3::Y * e) - f(p - Vec3::Y * e),
        f(p + Vec3::Z * e) - f(p - Vec3::Z * e),
    ) * (0.5 / e);
    (f(p), g)
}

/// Places a thin-feature vertex from its mass point `v0` with the exact
/// field `f`: Newton steps onto the surface, then the principal curvatures
/// of the field there (central-difference Hessian projected onto the
/// tangent plane). Where the surface bends around open space (a bore, a
/// pocket, an inside corner) the chords between neighbouring vertices
/// would cut into the open space by their sag, so the vertex moves into
/// the rock by `sqrt(R² + L²/4) - R` (radius `R` = 1 / curvature, chord
/// `L` = [`THIN_CHORD`]): the drawn faces then sit on the surface.
/// Returns the position and the unit normal, or `None` if the field gives
/// no usable gradient or the vertex would move too far.
fn fit_thin(f: &dyn Fn(Vec3) -> f32, v0: Vec3) -> Option<(Vec3, Vec3)> {
    let finite = |p: Vec3| p.x.is_finite() && p.y.is_finite() && p.z.is_finite();
    // Only true distance fields (carved bores); the bound cavern fields and
    // creases keep the grid snap.
    let (q, n) = surface_point(f, v0)?;
    if q.distance_squared(v0) > THIN_MAX_MOVE * THIN_MAX_MOVE {
        return None;
    }
    // Second derivatives in the tangent plane (5 evaluations): the shape
    // operator's 2x2 matrix (for a distance field |gradient| = 1).
    let helper = if n.x.abs() < 0.6 { Vec3::X } else { Vec3::Y };
    let t1 = n.cross(helper).normalize();
    let t2 = n.cross(t1);
    let e = THIN_PROBE;
    let c = f(q);
    let second = |t: Vec3| f(q + t * e) - 2.0 * c + f(q - t * e);
    let inv = 1.0 / (e * e);
    let a = second(t1) * inv;
    let cc = second(t2) * inv;
    // d²/dt1dt2 from the diagonal: (s(t1 + t2) - s(t1) - s(t2)) / 2, with
    // s(t1 + t2) the second difference along (t1 + t2) at the same step.
    let b = (second(t1 + t2) * inv - a - cc) * 0.5;
    if a.abs().max(cc.abs()) > CREASE_CURVATURE {
        // A crease within the probe: keep the exact position, no sag.
        return Some((q, n));
    }
    let mid = (a + cc) * 0.5;
    let r = (((a - cc) * 0.5) * ((a - cc) * 0.5) + b * b).sqrt();
    // Negative curvature: the surface bends around open space.
    let sag = |k: f32| {
        if k >= 0.0 || k.is_nan() {
            return 0.0;
        }
        let k = -k;
        let w = ((k - THIN_SAG_FROM) / (THIN_SAG_FULL - THIN_SAG_FROM)).clamp(0.0, 1.0);
        let radius = 1.0 / k;
        w * w
            * (3.0 - 2.0 * w)
            * ((radius * radius + 0.25 * THIN_CHORD * THIN_CHORD).sqrt() - radius)
    };
    // The sharper bend counts fully, the other one a third (the triangles
    // of a sphere sag by about 4/3 of one chord's sag).
    let s = (sag(mid - r) + sag(mid + r) * (1.0 / 3.0)).min(THIN_MAX_SAG);
    let q = q - n * s;
    finite(q).then_some((q, n))
}

/// Field slope (|gradient|) from which the field counts as a true distance
/// (carved and filled edits, mine shafts): only there are sub-cell bores
/// promoted and chord sag corrected. The generated cavern fields are bounds
/// with a slope of about 0.3 whose small dents need neither.
pub const EXACT_SLOPE: f32 = 0.75;
/// A surface point whose gradient is shorter than this share of the
/// steepest one on the way lies on a crease ([`surface_point`]).
const CREASE_SLOPE: f32 = 0.97;
/// Second derivatives (1/u) beyond which [`fit_thin`] sees a crease, not a
/// bend (a bore of radius 0.25 u bends by 4).
const CREASE_CURVATURE: f32 = 4.5;
/// A promoted corner's open space must end within this distance (u)
/// across ([`open_within_box`]): bores up to about 1.5 u wide.
const THIN_ACROSS: f32 = 1.6;
/// Newton steps of [`fit_thin`].
const THIN_NEWTON_STEPS: usize = 2;
/// Finite-difference step (u) of [`fit_thin`].
const THIN_PROBE: f32 = 0.1;
/// Largest distance (u) [`fit_thin`] may move a vertex from its mass point.
const THIN_MAX_MOVE: f32 = 0.9;
/// Chord length (u) between neighbouring vertices assumed for the sag.
pub const THIN_CHORD: f32 = 0.75;
/// Curvature (1/u) where the sag correction starts and where it is full.
const THIN_SAG_FROM: f32 = 0.35;
const THIN_SAG_FULL: f32 = 0.7;
/// Largest sag correction (u).
pub const THIN_MAX_SAG: f32 = 0.35;

/// The grid's corner values as a continuous field over world positions
/// (trilinear values and smooth gradients), read by world position so a
/// point shared by two chunks gives the same bits in both.
struct Field<'a> {
    values: &'a [f32],
    origin: [i32; 3],
}

/// Index of local corner `(i, j, k)` in the halo corner array.
#[inline]
fn corner_index(i: i32, j: i32, k: i32) -> usize {
    (i + GRID_HALO) as usize + SY * (j + GRID_HALO) as usize + SZ * (k + GRID_HALO) as usize
}

/// Strides of the halo corner array.
const SY: usize = HALO_CORNERS;
const SZ: usize = HALO_CORNERS * HALO_CORNERS;
/// The `4³` corners from `(-1, -1, -1)` to `(2, 2, 2)` of a cell span this
/// many entries of the halo array, starting `WINDOW_LOW` before the cell's
/// low corner.
const WINDOW: usize = 3 * SZ + 3 * SY + 4;
const WINDOW_LOW: usize = SZ + SY + 1;

/// `floor(x)` as an integer (exact for |x| < 2^31; plain conversions only).
#[inline]
fn floor_i32(x: f32) -> i32 {
    let c = x as i32;
    if (c as f32) > x {
        c - 1
    } else {
        c
    }
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Trilinear interpolation in the halo array `x` at the cell with low
/// corner index `b`.
#[inline]
fn trilinear(x: &[f32], b: usize, [u, v, w]: [f32; 3]) -> f32 {
    let c: &[f32; SZ + SY + 2] = x[b..b + SZ + SY + 2]
        .try_into()
        .expect("window inside the halo");
    lerp(
        lerp(lerp(c[0], c[1], u), lerp(c[SY], c[SY + 1], u), v),
        lerp(
            lerp(c[SZ], c[SZ + 1], u),
            lerp(c[SZ + SY], c[SZ + SY + 1], u),
            v,
        ),
        w,
    )
}

impl<'a> Field<'a> {
    fn new(grid: &'a SampleGrid) -> Self {
        Field {
            values: grid.values(),
            origin: grid.origin,
        }
    }

    /// Index of the corner at the low end of the cell holding `p` (clamped
    /// so that the cell's corners and `margin` more on each side stay inside
    /// the halo) and the position inside that cell.
    #[inline]
    fn locate(&self, p: Vec3, margin: i32) -> (usize, [f32; 3]) {
        let lo = -GRID_HALO + margin;
        let hi = CHUNK_CELLS + GRID_HALO - 1 - margin;
        let axis = |x: f32, origin: i32| {
            let f = floor_i32(x);
            let c = f - origin;
            let frac = x - f as f32;
            let (c, frac) = if c < lo || !frac.is_finite() {
                (lo, 0.0)
            } else if c > hi {
                (hi, 1.0)
            } else {
                (c, frac)
            };
            ((c + GRID_HALO) as usize, frac)
        };
        let (i, u) = axis(p.x, self.origin[0]);
        let (j, v) = axis(p.y, self.origin[1]);
        let (k, w) = axis(p.z, self.origin[2]);
        (i + SY * j + SZ * k, [u, v, w])
    }

    /// Trilinear interpolation of the corner values at `p`.
    #[inline]
    fn value(&self, p: Vec3) -> f32 {
        let (b, t) = self.locate(p, 0);
        trilinear(self.values, b, t)
    }

    /// Central-difference gradients at the 8 corners of the cell with low
    /// corner index `b` (located with margin 1).
    fn corner_gradients(&self, b: usize) -> [Vec3; 8] {
        let x: &[f32; WINDOW] = self.values[b - WINDOW_LOW..b - WINDOW_LOW + WINDOW]
            .try_into()
            .expect("window inside the halo");
        let g = |o: usize| {
            let o = WINDOW_LOW + o;
            Vec3::new(
                (x[o + 1] - x[o - 1]) * 0.5,
                (x[o + SY] - x[o - SY]) * 0.5,
                (x[o + SZ] - x[o - SZ]) * 0.5,
            )
        };
        [
            g(0),
            g(1),
            g(SY),
            g(SY + 1),
            g(SZ),
            g(SZ + 1),
            g(SZ + SY),
            g(SZ + SY + 1),
        ]
    }

    /// Tricubic (Catmull-Rom) interpolation of the corner values in the cell
    /// with low corner index `b` (located with margin 1) at `t`.
    fn cubic(&self, b: usize, [u, v, w]: [f32; 3]) -> f32 {
        let x: &[f32; WINDOW] = self.values[b - WINDOW_LOW..b - WINDOW_LOW + WINDOW]
            .try_into()
            .expect("window inside the halo");
        let weights = |t: f32| {
            let t2 = t * t;
            let t3 = t2 * t;
            [
                (2.0 * t2 - t3 - t) * 0.5,
                (3.0 * t3 - 5.0 * t2 + 2.0) * 0.5,
                (4.0 * t2 - 3.0 * t3 + t) * 0.5,
                (t3 - t2) * 0.5,
            ]
        };
        let (wx, wy, wz) = (weights(u), weights(v), weights(w));
        let mut sum = 0.0f32;
        for (kz, fz) in wz.into_iter().enumerate() {
            let mut plane = 0.0f32;
            for (jy, fy) in wy.into_iter().enumerate() {
                let r = &x[kz * SZ + jy * SY..kz * SZ + jy * SY + 4];
                plane += fy * (wx[0] * r[0] + wx[1] * r[1] + wx[2] * r[2] + wx[3] * r[3]);
            }
            sum += fz * plane;
        }
        sum
    }

    /// Smooth gradient: trilinear interpolation of a cell's corner
    /// gradients (continuous across cells and chunks).
    #[inline]
    fn interpolate(c: &[Vec3; 8], [u, v, w]: [f32; 3]) -> Vec3 {
        let l = |a: Vec3, b: Vec3, t: f32| a + (b - a) * t;
        l(
            l(l(c[0], c[1], u), l(c[2], c[3], u), v),
            l(l(c[4], c[5], u), l(c[6], c[7], u), v),
            w,
        )
    }

    /// Baked ambient occlusion at `p` with unit normal `n`; `scale` turns
    /// field values into distances. Each probe at `p + n h` expects the open
    /// half-space distance `h` and counts the relative shortfall.
    fn ambient_occlusion(&self, p: Vec3, n: Vec3, scale: f32) -> u8 {
        let inv_scale = 1.0 / scale;
        let mut occlusion = 0.0f32;
        for (h, w) in AO_PROBES {
            let d = self.value(p + n * h) * inv_scale;
            occlusion += w * ((h - d) / h).clamp(0.0, 1.0);
        }
        let ao = (1.0 - AO_STRENGTH * occlusion).clamp(0.0, 1.0);
        (ao * 255.0 + 0.5) as u8
    }
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
        let len = len2.sqrt();
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

    /// Two neighbouring chunks share their border vertices exactly, with
    /// the same normals and occlusion.
    #[test]
    fn seam_free_across_chunks() {
        let center = Vec3::new(32.2, 20.4, 16.7);
        // A bumpy sphere (bound field, slope 0.5) so snapping, normals and
        // occlusion all have work to do at the seam.
        let f = |p: Vec3| {
            let q = p - center;
            (q.length() - 10.0 + 0.8 * (q.x * 0.7).sin() * (q.y * 0.5).cos()) * 0.5
        };
        let exact_dc = |g: &SampleGrid| dual_contouring_with_sdf(g, &f);
        let exact_sn = |g: &SampleGrid| surface_nets_with_sdf(g, &f);
        let meshers: [&dyn Fn(&SampleGrid) -> Mesh; 4] =
            [&surface_nets, &exact_sn, &dual_contouring, &exact_dc];
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
            let shared: Vec<_> = bb.iter().filter(|p| f32::from_bits(p[0]) < 31.5).collect();
            assert!(!shared.is_empty());
            for p in shared {
                assert!(all_a.contains(p), "vertex {p:?} missing in neighbour");
            }
            // Shared vertices carry bit-identical normals and occlusion.
            let attrs = |m: &Mesh, i: usize| {
                (
                    m.normals[i].map(f32::to_bits),
                    m.ao.get(i).copied().unwrap_or(0),
                )
            };
            let in_a: std::collections::BTreeMap<[u32; 3], _> = (0..a.vertex_count())
                .map(|i| (a.positions[i].map(f32::to_bits), attrs(&a, i)))
                .collect();
            let mut matched = 0;
            for i in 0..b.vertex_count() {
                if let Some(at) = in_a.get(&b.positions[i].map(f32::to_bits)) {
                    assert_eq!(*at, attrs(&b, i), "vertex {:?}", b.positions[i]);
                    matched += 1;
                }
            }
            assert!(matched > 20, "{matched} shared vertices");
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

    /// Plain surface-nets vertices (mass points of the edge crossings) of a
    /// grid, for comparison with the refined ones.
    fn mass_points(grid: &SampleGrid) -> Vec<Vec3> {
        let mut out = Vec::new();
        for k in -1..CHUNK_CELLS {
            for j in -1..CHUNK_CELLS {
                for i in -1..CHUNK_CELLS {
                    let c: [f32; 8] = std::array::from_fn(|ci| {
                        let o = CORNERS[ci];
                        grid.value(i + o[0], j + o[1], k + o[2])
                    });
                    if c.iter().all(|v| *v < 0.0) || c.iter().all(|v| *v >= 0.0) {
                        continue;
                    }
                    let l = mass_point(&c);
                    out.push(Vec3::new(i as f32 + l[0], j as f32 + l[1], k as f32 + l[2]));
                }
            }
        }
        out
    }

    /// A bumpy rock blob whose field is a bound with slope 0.5 (like the
    /// cavern fields).
    fn bumpy(p: Vec3) -> f32 {
        let q = p - Vec3::new(16.2, 15.9, 16.4);
        (q.length() - 10.0 + 1.2 * (q.x * 0.45).sin() * (q.z * 0.35).cos()) * 0.5
    }

    fn bumpy_gradient(p: Vec3) -> Vec3 {
        let e = 1e-3;
        Vec3::new(
            bumpy(p + Vec3::X * e) - bumpy(p - Vec3::X * e),
            bumpy(p + Vec3::Y * e) - bumpy(p - Vec3::Y * e),
            bumpy(p + Vec3::Z * e) - bumpy(p - Vec3::Z * e),
        ) * (0.5 / e)
    }

    /// First-order distance from `p` to the bumpy surface.
    fn surface_error(p: Vec3) -> f32 {
        bumpy(p).abs() / bumpy_gradient(p).length()
    }

    fn mean(v: impl Iterator<Item = f32>) -> f32 {
        let (s, n) = v.fold((0.0, 0), |(s, n), x| (s + x, n + 1));
        s / n as f32
    }

    /// The snap puts vertices onto the surface: from the grid alone (cubic
    /// interpolation) and with the exact field at creases.
    #[test]
    fn snapped_vertices_lie_on_the_surface() {
        let grid = SampleGrid::from_fn([0, 0, 0], bumpy);
        let plain = mean(mass_points(&grid).into_iter().map(surface_error));
        let grid_only = surface_nets(&grid);
        let exact = surface_nets_with_sdf(&grid, &bumpy);
        let err = |m: &Mesh| {
            mean(
                m.positions
                    .iter()
                    .map(|p| surface_error(Vec3::from_array(*p))),
            )
        };
        let (e_grid, e_exact) = (err(&grid_only), err(&exact));
        println!("mean surface error: plain {plain:.4} u, grid snap {e_grid:.4} u, exact snap {e_exact:.4} u");
        assert!(plain > 0.02, "plain surface nets error {plain}");
        assert!(e_grid < plain * 0.5, "grid snap {e_grid} vs plain {plain}");
        assert!(
            e_exact < plain * 0.25,
            "exact snap {e_exact} vs plain {plain}"
        );
        assert!(e_exact < 0.01, "exact snap {e_exact}");
        // A smooth sphere with an exact field: on the surface to 0.01 u.
        let center = Vec3::new(16.3, 15.7, 16.1);
        let m = surface_nets(&sphere_grid([0, 0, 0], center, 9.4));
        for p in &m.positions {
            let d = Vec3::from_array(*p).distance(center) - 9.4;
            assert!(d.abs() < 0.01, "{d}");
        }
        // Never more than SNAP_MAX_STEP from the mass point.
        let mp = mass_points(&grid);
        assert_eq!(mp.len(), exact.vertex_count());
        for (a, b) in mp.iter().zip(&exact.positions) {
            assert!(a.distance(Vec3::from_array(*b)) <= SNAP_MAX_STEP + 1e-4);
        }
    }

    /// Normals follow the field's gradient more closely than the old
    /// per-cell gradients, and turn smoothly between neighbouring vertices.
    #[test]
    fn normals_are_smooth_and_follow_the_field() {
        let grid = SampleGrid::from_fn([0, 0, 0], bumpy);
        let m = surface_nets_with_sdf(&grid, &bumpy);
        let angle = |a: Vec3, b: Vec3| a.dot(b).clamp(-1.0, 1.0).acos().to_degrees();
        // The old normal of each vertex (same order): gradient of the cell's
        // own trilinear interpolant at the mass point.
        let old: Vec<Vec3> = mass_points(&grid)
            .into_iter()
            .map(|p| {
                let cell = [p.x.floor(), p.y.floor(), p.z.floor()];
                let c: [f32; 8] = std::array::from_fn(|ci| {
                    let o = CORNERS[ci];
                    let i = |a: usize| cell[a] as i32 + o[a];
                    grid.value(i(0), i(1), i(2))
                });
                let l = [p.x - cell[0], p.y - cell[1], p.z - cell[2]];
                Vec3::from_array(normalize_or_up(trilinear_gradient(&c, l)))
            })
            .collect();
        assert_eq!(old.len(), m.vertex_count());
        let field = |i: usize| bumpy_gradient(Vec3::from_array(m.positions[i])).normalize();
        let new_n = |i: usize| Vec3::from_array(m.normals[i]);
        let new_err = mean((0..m.vertex_count()).map(|i| angle(new_n(i), field(i))));
        let old_err = mean((0..m.vertex_count()).map(|i| angle(old[i], field(i))));
        assert!(new_err < 1.0, "normal error {new_err} deg");
        assert!(
            new_err < old_err * 0.8,
            "new {new_err} deg vs old {old_err} deg"
        );
        // Facets: along a mesh edge, how much more the normals turn than the
        // field does. The continuous normals turn about as much as the
        // field; the per-cell ones jump between cells.
        let excess = |n: &dyn Fn(usize) -> Vec3| {
            let mut v = Vec::new();
            for t in m.indices.chunks_exact(3) {
                for e in 0..3 {
                    let (a, b) = (t[e] as usize, t[(e + 1) % 3] as usize);
                    v.push((angle(n(a), n(b)) - angle(field(a), field(b))).max(0.0));
                }
            }
            v.sort_by(f32::total_cmp);
            (mean(v.iter().copied()), v[v.len() * 99 / 100])
        };
        let (new_mean, new_p99) = excess(&new_n);
        let (old_mean, old_p99) = excess(&|i| old[i]);
        println!(
            "normal error {new_err:.2} deg (per-cell normals {old_err:.2}); excess turn per edge mean/p99 {new_mean:.2}/{new_p99:.2} deg (per-cell {old_mean:.2}/{old_p99:.2})"
        );
        assert!(
            new_mean < old_mean * 0.6 && new_p99 < old_p99 * 0.6,
            "excess turn new {new_mean}/{new_p99} deg vs old {old_mean}/{old_p99} deg"
        );
    }

    fn ao_at(m: &Mesh, near: impl Fn(Vec3) -> bool) -> Vec<u8> {
        m.positions
            .iter()
            .zip(&m.ao)
            .filter(|(p, _)| near(Vec3::from_array(**p)))
            .map(|(_, a)| *a)
            .collect()
    }

    /// Flat and convex rock stays fully lit; an inside corner darkens
    /// towards the crease; a narrow slot is dark.
    #[test]
    fn ambient_occlusion_darkens_creases_only() {
        // Floor (rock below y = 10.3), seen from above.
        let flat = surface_nets(&SampleGrid::from_fn([0, 0, 0], |p| p.y - 10.3));
        assert_eq!(flat.ao.len(), flat.vertex_count());
        assert!(flat.ao.iter().all(|&a| a == 255));
        // The same with a bound field (slope 0.3): scaled back to distances.
        let flat = surface_nets(&SampleGrid::from_fn([0, 0, 0], |p| (p.y - 10.3) * 0.3));
        assert!(flat.ao.iter().all(|&a| a >= 254));
        // Inside corner: floor plus a wall (rock beyond x = 20.4).
        let corner = surface_nets(&SampleGrid::from_fn([0, 0, 0], |p| {
            (p.y - 10.3).min(20.4 - p.x)
        }));
        let floor_at = |x0: f32, x1: f32| {
            let v = ao_at(&corner, |p| {
                (p.y - 10.3).abs() < 0.1 && p.x > x0 && p.x < x1
            });
            assert!(!v.is_empty());
            mean(v.iter().map(|&a| a as f32))
        };
        let (crease, near, far) = (
            floor_at(19.5, 20.0),
            floor_at(18.5, 19.5),
            floor_at(10.0, 16.0),
        );
        assert!(crease < near && near < far, "{crease} {near} {far}");

        assert!(crease < 215.0, "crease ao {crease}");
        assert!(far >= 254.0, "open floor ao {far}");
        // Outside corner (a block): fully lit.
        let block = surface_nets(&SampleGrid::from_fn([0, 0, 0], |p| {
            (10.3 - p.y).max(p.x - 20.4)
        }));
        assert!(block.ao.iter().all(|&a| a >= 254));
        // A slot 1.6 u high between a floor and a ceiling.
        let slot = surface_nets(&SampleGrid::from_fn([0, 0, 0], |p| {
            (p.y - 10.3).min(11.9 - p.y)
        }));
        assert!(slot.ao.iter().all(|&a| a < 160), "{:?}", &slot.ao[..8]);
        // Dual contouring carries none.
        assert!(
            dual_contouring(&SampleGrid::from_fn([0, 0, 0], |p| p.y - 10.3))
                .ao
                .is_empty()
        );
    }
}
