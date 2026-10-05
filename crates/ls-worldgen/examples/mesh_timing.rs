//! Per-chunk meshing time of the game's mesher (`ChunkStore::mesh_surface_nets`)
//! over whole levels, best of several passes per chunk (release):
//!
//! ```text
//! cargo run -p ls-worldgen --release --example mesh_timing [PASSES]
//! ```
//!
//! For each world: the busiest chunk (most triangles), p50 / p95 / max over
//! the non-empty chunks and the sum. Used for the M4 A2 before/after
//! comparison (dual-contouring placement, thin-rock pass).

use std::time::Instant;

use ls_worldgen::{ChunkStore, Generator, Sliders};

fn main() {
    let passes: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5);
    for (name, seed, e, d) in [
        ("caverns", 2024u64, 50u8, 60u8),
        ("mines", 2024, 100, 40),
        ("mine shafts", 7, 100, 30),
        ("caves/mines", 99, 70, 60),
        ("sunlit caves", 7, 47, 60),
    ] {
        let mut store = ChunkStore::new(Generator::new(seed, Sliders::new(e, d)));
        store.generate_all();
        let coords = store.chunk_coords();
        let mut best = vec![f64::INFINITY; coords.len()];
        let mut tris = vec![0usize; coords.len()];
        for _ in 0..passes {
            for (i, &c) in coords.iter().enumerate() {
                let t = Instant::now();
                let m = store.mesh_surface_nets(c);
                best[i] = best[i].min(t.elapsed().as_secs_f64() * 1e3);
                tris[i] = m.triangle_count();
            }
        }
        let mut times: Vec<f64> = (0..coords.len())
            .filter(|&i| tris[i] > 0)
            .map(|i| best[i])
            .collect();
        times.sort_by(f64::total_cmp);
        let q = |p: f64| times[((times.len() - 1) as f64 * p) as usize];
        let busiest = (0..coords.len()).max_by_key(|&i| tris[i]).unwrap();
        println!(
            "{name:12}: busiest chunk {:?} ({} tris) {:.3} ms; {} chunks p50 {:.3} p95 {:.3} max {:.3} ms, sum {:.1} ms",
            coords[busiest],
            tris[busiest],
            best[busiest],
            times.len(),
            q(0.5),
            q(0.95),
            q(1.0),
            times.iter().sum::<f64>()
        );
    }
}
