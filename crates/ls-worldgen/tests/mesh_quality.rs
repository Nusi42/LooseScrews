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
