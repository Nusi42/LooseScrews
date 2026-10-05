//! Surface-nets mesh quality on generated worlds (visual-quality plan, VQ3a):
//! how far the drawn rock lies from the collision surface (`ChunkStore::eval`,
//! the field the sim uses), how well the normals follow the field and how
//! much of the rock the baked ambient occlusion darkens.
//!
//! The surface error at a point is the first-order distance estimate
//! `|d| / |grad d|` of the exact field there, at the vertices and at the
//! triangle centroids (the flat triangles in between). Plain surface nets
//! (before VQ3a, same chunks, release build):
//!
//! | world (seed, enclosure, density) | vertices p50 / p95 | centroids p50 / p95 |
//! | --- | --- | --- |
//! | 7, 50, 50 (caverns) | 0.012 / 0.198 u | 0.023 / 0.237 u |
//! | 42, 50, 50 (caverns) | 0.014 / 0.209 u | 0.029 / 0.246 u |
//! | 99, 70, 60 (caverns/mines) | 0.015 / 0.270 u | 0.048 / 0.240 u |
//! | 7, 20, 70 (sky and rocks) | 0.031 / 0.124 u | 0.048 / 0.183 u |
//! | 7, 100, 30 (mine shafts) | 0.000 / 0.354 u | 0.000 / 0.174 u |
//!
//! ```text
//! cargo test -p ls-worldgen --release --test mesh_quality -- --nocapture
//! ```

use ls_math::Vec3;
use ls_worldgen::{ChunkCoord, ChunkStore, Generator, Sliders};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

struct Stats {
    vertices: usize,
    err_median: f32,
    err_p95: f32,
    centroid_median: f32,
    centroid_p95: f32,
    angle_median: f32,
    /// Share of vertices with occlusion below 200 (of 255).
    ao_dark: f32,
    ao_median: f32,
}

/// Exact field gradient (central differences of `ChunkStore::eval`).
fn field_gradient(store: &ChunkStore, p: Vec3) -> Vec3 {
    let e = 0.05;
    Vec3::new(
        store.eval(p + Vec3::X * e) - store.eval(p - Vec3::X * e),
        store.eval(p + Vec3::Y * e) - store.eval(p - Vec3::Y * e),
        store.eval(p + Vec3::Z * e) - store.eval(p - Vec3::Z * e),
    ) * (0.5 / e)
}

/// First-order distance from `p` to the collision surface and the field's
/// unit gradient, if the field is unclamped and has a usable slope there.
fn surface_error(store: &ChunkStore, p: Vec3) -> Option<(f32, Vec3)> {
    let d = store.eval(p);
    let g = field_gradient(store, p);
    let gl = g.length();
    (gl >= 0.05 && d.abs() < 5.0).then(|| (d.abs() / gl, g * (1.0 / gl)))
}

fn percentile(v: &mut [f32], q: f32) -> f32 {
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * q) as usize]
}

/// Non-empty chunks around the room centre (at most `limit`).
fn chunks_near_room(store: &mut ChunkStore, limit: usize) -> Vec<ChunkCoord> {
    let center = store.generator().room().center();
    let c = store.chunk_at(center).expect("room in level");
    let mut out = Vec::new();
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let n = ChunkCoord::new(c.x + dx, c.y + dy, c.z + dz);
                if out.len() < limit
                    && store.contains_chunk(n)
                    && !store.mesh_surface_nets(n).is_empty()
                {
                    out.push(n);
                }
            }
        }
    }
    out
}

fn measure(seed: u64, enclosure: u8, density: u8, limit: usize) -> Stats {
    let mut store = ChunkStore::new(Generator::new(seed, Sliders::new(enclosure, density)));
    let chunks = chunks_near_room(&mut store, limit);
    assert!(!chunks.is_empty());
    let (mut errs, mut centroids, mut angles, mut ao) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for c in chunks {
        let m = store.mesh_surface_nets(c);
        assert_eq!(m.ao.len(), m.vertex_count());
        let pos = |i: u32| Vec3::from_array(m.positions[i as usize]);
        for (i, n) in m.normals.iter().enumerate() {
            if let Some((e, g)) = surface_error(&store, pos(i as u32)) {
                errs.push(e);
                let cos = g.dot(Vec3::from_array(*n)).clamp(-1.0, 1.0);
                angles.push(cos.acos().to_degrees());
            }
        }
        for t in m.indices.chunks_exact(3) {
            let centroid = (pos(t[0]) + pos(t[1]) + pos(t[2])) * (1.0 / 3.0);
            if let Some((e, _)) = surface_error(&store, centroid) {
                centroids.push(e);
            }
        }
        ao.extend(m.ao.iter().map(|&a| a as f32));
    }
    let dark = ao.iter().filter(|&&a| a < 200.0).count() as f32 / ao.len() as f32;
    Stats {
        vertices: errs.len(),
        err_median: percentile(&mut errs, 0.5),
        err_p95: percentile(&mut errs, 0.95),
        centroid_median: percentile(&mut centroids, 0.5),
        centroid_p95: percentile(&mut centroids, 0.95),
        angle_median: percentile(&mut angles, 0.5),
        ao_dark: dark,
        ao_median: percentile(&mut ao, 0.5),
    }
}

/// Worlds of every kind: caverns (bound field, slope about 0.3), a
/// cavern/mine blend, open sky with rocks, and mine shafts (exact boxes).
const WORLDS: [(u64, u8, u8); 5] = [
    (7, 50, 50),
    (42, 50, 50),
    (99, 70, 60),
    (7, 20, 70),
    (7, 100, 30),
];

#[test]
fn drawn_surface_matches_the_collision_surface() {
    let limit = if cfg!(debug_assertions) { 2 } else { 8 };
    for (seed, e, d) in WORLDS {
        let s = measure(seed, e, d, limit);
        println!(
            "seed {seed:3} enclosure {e:3} density {d:3}: {} vertices; surface error at vertices \
             p50 {:.3} u p95 {:.3} u, at triangle centroids p50 {:.3} u p95 {:.3} u; normal vs \
             field gradient p50 {:.1} deg; occlusion p50 {:.0}, {:.0} % of vertices below 200",
            s.vertices,
            s.err_median,
            s.err_p95,
            s.centroid_median,
            s.centroid_p95,
            s.angle_median,
            s.ao_median,
            s.ao_dark * 100.0
        );
        assert!(s.vertices > 500, "too few vertices measured");
        assert!(s.err_median <= 0.02, "vertex error p50 {} u", s.err_median);
        assert!(s.err_p95 <= 0.1, "vertex error p95 {} u", s.err_p95);
        assert!(
            s.centroid_p95 <= 0.17,
            "centroid error p95 {} u",
            s.centroid_p95
        );
        assert!(
            s.angle_median <= 6.0,
            "normal error p50 {} deg",
            s.angle_median
        );
        // Occlusion darkens creases and narrow passages (most of a cavern
        // world: p50 about 160), never the whole level.
        assert!(s.ao_median >= 120.0, "occlusion p50 {}", s.ao_median);
        assert!(s.ao_dark > 0.01, "almost no occlusion: {}", s.ao_dark);
    }
}

/// Exact Hermite data of the grid edge from corner `a` along `axis` (unit
/// step), if the field changes sign on it: the crossing (bisection on the
/// exact field) and the unit normal there.
fn hermite(store: &ChunkStore, a: Vec3, axis: Vec3) -> Option<(Vec3, Vec3, f32)> {
    let (va, vb) = (store.eval(a), store.eval(a + axis));
    if (va < 0.0) == (vb < 0.0) {
        return None;
    }
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let t = (t0 + t1) * 0.5;
        if (store.eval(a + axis * t) < 0.0) == (va < 0.0) {
            t0 = t;
        } else {
            t1 = t;
        }
    }
    let p = a + axis * ((t0 + t1) * 0.5);
    let e = 0.01;
    let g = Vec3::new(
        store.eval(p + Vec3::X * e) - store.eval(p - Vec3::X * e),
        store.eval(p + Vec3::Y * e) - store.eval(p - Vec3::Y * e),
        store.eval(p + Vec3::Z * e) - store.eval(p - Vec3::Z * e),
    ) * (0.5 / e);
    let len = g.length();
    (len > 0.0).then(|| (p, g * (1.0 / len), len))
}

/// The sharp feature point of a cell (low corner `c`) from exact Hermite
/// data: the least-squares intersection of the tangent planes at the edge
/// crossings, if the cell holds a sharp feature of a true distance field
/// (normals at least 30 degrees apart, every gradient of length about 1,
/// the planes meeting in one point to 0.01 u, and that point inside the
/// cell).
fn feature_point(store: &ChunkStore, c: Vec3) -> Option<(Vec3, Option<Vec3>)> {
    let corner = |i: usize| c + Vec3::new((i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2) as f32);
    let mut planes = Vec::new();
    for i in 0..8 {
        for (bit, axis) in [(1, Vec3::X), (2, Vec3::Y), (4, Vec3::Z)] {
            if i & bit == 0 {
                if let Some(h) = hermite(store, corner(i), axis) {
                    planes.push(h);
                }
            }
        }
    }
    if planes.len() < 3
        || planes
            .iter()
            .any(|&(_, _, len)| !(0.97..=1.03).contains(&len))
    {
        return None;
    }
    let min_cos = planes
        .iter()
        .flat_map(|a| planes.iter().map(move |b| a.1.dot(b.1)))
        .fold(1.0f32, f32::min);
    if min_cos > 0.866 {
        return None;
    }
    // Normal equations around the mass point, with a small pull towards it
    // along directions no plane constrains (an edge's direction).
    let m = planes.iter().fold(Vec3::ZERO, |s, p| s + p.0) * (1.0 / planes.len() as f32);
    let mut ata = [[0.0f64; 3]; 3];
    let mut atb = [0.0f64; 3];
    for &(p, n, _) in &planes {
        let n = [n.x as f64, n.y as f64, n.z as f64];
        let d =
            (n[0] * (p.x - m.x) as f64) + (n[1] * (p.y - m.y) as f64) + (n[2] * (p.z - m.z) as f64);
        for r in 0..3 {
            for k in 0..3 {
                ata[r][k] += n[r] * n[k];
            }
            atb[r] += n[r] * d;
        }
    }
    for (r, row) in ata.iter_mut().enumerate() {
        row[r] += 1e-3;
    }
    let det = |a: &[[f64; 3]; 3]| {
        a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
            - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
    };
    let d = det(&ata);
    let mut y = [0.0f64; 3];
    for (col, out) in y.iter_mut().enumerate() {
        let mut a = ata;
        for r in 0..3 {
            a[r][col] = atb[r];
        }
        *out = det(&a) / d;
    }
    let x = m + Vec3::new(y[0] as f32, y[1] as f32, y[2] as f32);
    let residual = planes
        .iter()
        .map(|&(p, n, _)| n.dot(x - p).abs())
        .fold(0.0f32, f32::max);
    let local = x - c;
    let inside = [local.x, local.y, local.z]
        .iter()
        .all(|&v| (-1e-3..=1.0 + 1e-3).contains(&v));
    // An edge (all normals perpendicular to one direction) or a corner.
    let (a, b) = planes
        .iter()
        .flat_map(|a| planes.iter().map(move |b| (a.1, b.1)))
        .min_by(|p, q| p.0.dot(p.1).total_cmp(&q.0.dot(q.1)))
        .unwrap();
    let dir = a.cross(b).normalize();
    let edge = planes.iter().all(|p| p.1.dot(dir).abs() < 0.05);
    (residual < 0.01 && inside).then_some((x, edge.then_some(dir)))
}

/// Corner error at sharp-feature cells (M4 A2, dual-contouring placement):
/// for every cell of mine-shaft chunks whose exact field has a sharp edge or
/// corner in it ([`feature_point`]), the distance from the feature (the
/// edge line or the corner point) to the nearest mesh vertex within half a
/// cell of the cell. Plain surface nets round every edge off by
/// a few tenths of a unit; placing feature vertices by the QEF puts them
/// on the edge.
fn corner_errors(seed: u64, enclosure: u8, density: u8, limit: usize) -> Vec<f32> {
    let mut store = ChunkStore::new(Generator::new(seed, Sliders::new(enclosure, density)));
    let chunks = chunks_near_room(&mut store, limit);
    let mut errs = Vec::new();
    for c in chunks {
        let m = store.mesh_surface_nets(c);
        let verts: Vec<Vec3> = m.positions.iter().map(|p| Vec3::from_array(*p)).collect();
        let o = c.origin();
        for k in 0..32 {
            for j in 0..32 {
                for i in 0..32 {
                    let low = Vec3::new((o[0] + i) as f32, (o[1] + j) as f32, (o[2] + k) as f32);
                    let Some((x, edge)) = feature_point(&store, low) else {
                        continue;
                    };
                    // Distance to the feature: to the edge line, or to the
                    // corner point.
                    let off = |v: Vec3| {
                        let d = v - x;
                        match edge {
                            Some(e) => (d - e * d.dot(e)).length(),
                            None => d.length(),
                        }
                    };
                    // The vertices of this cell (within half a cell of it).
                    let near = |v: &&Vec3| {
                        let l = **v - low;
                        [l.x, l.y, l.z].iter().all(|&t| (-0.5..=1.5).contains(&t))
                    };
                    let nearest = verts
                        .iter()
                        .filter(near)
                        .map(|&v| off(v))
                        .fold(f32::INFINITY, f32::min);
                    if nearest > 0.03 {
                        let vals: Vec<f32> = (0..8)
                            .map(|q| {
                                store.eval(
                                    low + Vec3::new(
                                        (q & 1) as f32,
                                        (q >> 1 & 1) as f32,
                                        (q >> 2) as f32,
                                    ),
                                )
                            })
                            .collect();
                        let nv: Vec<Vec3> = verts.iter().filter(near).map(|&v| v - low).collect();
                        eprintln!("BAD low {low:?} x-low {:?} edge {edge:?} err {nearest} verts {nv:?} vals {vals:?}", x - low);
                    }
                    errs.push(nearest);
                }
            }
        }
    }
    errs
}

#[test]
fn mine_shaft_corners_are_sharp() {
    let limit = if cfg!(debug_assertions) { 1 } else { 8 };
    let mut errs = corner_errors(7, 100, 30, limit);
    let n = errs.len();
    let p50 = percentile(&mut errs, 0.5);
    let p95 = percentile(&mut errs, 0.95);
    let max = percentile(&mut errs, 1.0);
    println!(
        "mine shafts (7, 100, 30): {n} sharp-feature cells; feature point to nearest vertex          p50 {p50:.3} u p95 {p95:.3} u max {max:.3} u"
    );
    assert!(n > 100, "too few feature cells measured: {n}");
    // M4 A2 measured p95 0.025 u (0.400 before) on the generator version 3
    // mines. Version 4 mines (Verticality 50: more vertical shafts) add
    // shaft/tunnel junction corners a cell apart, where the plane solve falls
    // back to the snapped vertex: p95 0.041 u at the wave A merge. Open item
    // (M4 plan, A2 as built); the bound keeps the 10x gain from regressing.
    assert!(p95 <= 0.045, "corner error p95 {p95} u");
}
