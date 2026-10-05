//! Meshing performance check. Prints surface-nets (and dual-contouring)
//! timings for a busy chunk; in release builds it also asserts the M1 target
//! of < 2 ms per 32³ chunk for surface nets (best of several runs, so a
//! loaded machine does not cause false failures).
//!
//! ```text
//! cargo test -p ls-worldgen --release --test timing -- --nocapture
//! ```
#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use ls_worldgen::{surface_nets, ChunkCoord, ChunkStore, Generator, Sliders};

fn busiest_chunk(store: &mut ChunkStore, candidates: &[ChunkCoord]) -> (ChunkCoord, usize) {
    candidates
        .iter()
        .map(|&c| (c, store.mesh_surface_nets(c).triangle_count()))
        .max_by_key(|&(c, t)| (t, std::cmp::Reverse(c)))
        .unwrap()
}

#[test]
fn surface_nets_chunk_timing() {
    let release = !cfg!(debug_assertions);
    let candidates: Vec<ChunkCoord> = if release {
        (1..7)
            .flat_map(|x| (1..3).map(move |y| ChunkCoord::new(x, y, 3)))
            .collect()
    } else {
        vec![ChunkCoord::new(3, 1, 3), ChunkCoord::new(4, 2, 4)]
    };
    for (name, sliders) in [
        ("caverns", Sliders::new(50, 60)),
        ("mines", Sliders::new(100, 40)),
    ] {
        let mut store = ChunkStore::new(Generator::new(2024, sliders));
        let t0 = Instant::now();
        store.ensure_generated(candidates[0]);
        let gen_time = t0.elapsed();
        let (chunk, _) = busiest_chunk(&mut store, &candidates);
        let t0 = Instant::now();
        let grid = store.sample_grid(chunk);
        let grid_time = t0.elapsed();

        // The bare grid has no exact field: no thin passes, its own count.
        let tris = surface_nets(&grid).triangle_count();
        let runs = if release { 25 } else { 3 };
        let mut times: Vec<Duration> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                let m = surface_nets(&grid);
                let e = t.elapsed();
                assert_eq!(m.triangle_count(), tris);
                e
            })
            .collect();
        times.sort();
        let best = times[0];
        let median = times[times.len() / 2];

        let t0 = Instant::now();
        let dc = store.mesh_dual_contouring(chunk);
        let dc_time = t0.elapsed();

        println!(
            "[{name}] chunk {chunk:?}: {tris} triangles; surface nets best {:.3} ms, median {:.3} ms; \
             dual contouring (exact Hermite) {:.1} ms, {} triangles; chunk generation {:.1} ms; \
             padded grid {:.3} ms ({})",
            best.as_secs_f64() * 1e3,
            median.as_secs_f64() * 1e3,
            dc_time.as_secs_f64() * 1e3,
            dc.triangle_count(),
            gen_time.as_secs_f64() * 1e3,
            grid_time.as_secs_f64() * 1e3,
            if release { "release" } else { "debug, not asserted" },
        );
        assert!(tris > 1000, "chunk too empty to be a meaningful benchmark");
        if release {
            assert!(
                best < Duration::from_millis(2),
                "surface nets took {best:?} (target < 2 ms)"
            );
        }
    }
}
