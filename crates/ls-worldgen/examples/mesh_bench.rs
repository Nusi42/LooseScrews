//! Prints meshing timings for one share code.
//!
//! ```text
//! cargo run -p ls-worldgen --release --example mesh_bench [SHARE-CODE]
//! ```

use std::time::Instant;

use ls_worldgen::{surface_nets, ChunkStore, Generator, ShareCode, Sliders};

fn main() {
    let code = match std::env::args().nth(1) {
        Some(text) => ShareCode::decode(&text).expect("invalid share code"),
        None => ShareCode::new(2024, Sliders::new(50, 60)),
    };
    println!("share code {code}");
    let gen = Generator::from_code(&code).expect("unsupported share code");
    let mut store = ChunkStore::new(gen);

    let t = Instant::now();
    store.generate_all();
    println!(
        "generate all chunks: {:.1} ms",
        t.elapsed().as_secs_f64() * 1e3
    );

    let mut best = None;
    let mut total_tris = 0;
    let t = Instant::now();
    for c in store.chunk_coords() {
        let tris = store.mesh_surface_nets(c).triangle_count();
        total_tris += tris;
        if best.is_none_or(|(_, b)| tris > b) {
            best = Some((c, tris));
        }
    }
    println!(
        "surface nets, whole level: {:.1} ms, {total_tris} triangles",
        t.elapsed().as_secs_f64() * 1e3
    );
    let (chunk, tris) = best.expect("level has chunks");
    let grid = store.sample_grid(chunk);
    let mut times: Vec<f64> = (0..50)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(surface_nets(&grid));
            t.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    times.sort_by(f64::total_cmp);
    println!(
        "surface nets, busiest chunk {chunk:?} ({tris} triangles): best {:.3} ms, median {:.3} ms",
        times[0],
        times[times.len() / 2]
    );
    let t = Instant::now();
    let dc = store.mesh_dual_contouring(chunk);
    println!(
        "dual contouring (exact Hermite), same chunk: {:.1} ms, {} triangles",
        t.elapsed().as_secs_f64() * 1e3,
        dc.triangle_count()
    );
    let cov = store.paint_coverage();
    println!("surface cells: {}", cov.surface_cells);
}
