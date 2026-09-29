//! Prints world generation, remesh and dual-contouring timings (release).
//!
//! ```text
//! cargo run -p ls-worldgen --release --example worldgen_bench
//! ```
//!
//! - generation: `generate_all` per slider mix, and ms per chunk;
//! - remesh after 1 and after 200 craters, with every chunk generated (the
//!   game's case) and lazily (a fresh store that replays 200 craters first);
//! - `ChunkStore::eval` cost next to 0 and 200 craters;
//! - dual contouring (exact Hermite data) of the busiest chunk.

use std::time::Instant;

use ls_worldgen::ls_math::terrain::{TerrainEdit, TerrainQuery};
use ls_worldgen::ls_math::{det, SplitMix64, Vec3};
use ls_worldgen::{ChunkCoord, ChunkStore, Generator, Sliders};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Best of `runs` timings of `f` in ms.
fn best_of(runs: usize, mut f: impl FnMut()) -> f64 {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            ms(t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// A terrain surface point away from the walls, plus its normal.
fn surface_point(s: &ChunkStore) -> (Vec3, Vec3) {
    let room = s.generator().room();
    let o = s
        .find_open_point(room.center(), 1.5, 60)
        .expect("open point");
    for i in 0..64u32 {
        let a = i as f32 * 0.7;
        let dir = Vec3::new(det::cos(a), 0.9 - (i % 7) as f32 * 0.3, det::sin(a));
        if let Some(hit) = s.raycast(o, dir, 400.0) {
            if hit.distance > 0.0 && room.expand(-8.0).contains(hit.point) {
                return (hit.point, hit.normal);
            }
        }
    }
    panic!("no surface");
}

fn craters(hit: Vec3, n: usize) -> Vec<TerrainEdit> {
    let mut rng = SplitMix64::new(5);
    (0..n)
        .map(|_| TerrainEdit::SubtractSphere {
            center: hit
                + Vec3::new(
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                ) * 12.0,
            radius: 3.0,
        })
        .collect()
}

/// Cost of one generator sample at every corner of a busy chunk, hashed
/// per lookup (`Generator::sample`) and through a `RegionSampler`.
fn per_sample_costs() {
    use ls_worldgen::ls_math::Aabb;
    println!("== generator sample cost (all 33^3 corners of chunk 3,1,3) ==");
    for (seed, e, d) in [
        (2024u64, 50u8, 60u8),
        (2024, 100, 40),
        (0xDEAD_BEEF, 75, 20),
        (1, 0, 0),
    ] {
        let gen = Generator::new(seed, Sliders::new(e, d));
        let o = Vec3::new(96.0, 32.0, 96.0);
        let pts: Vec<Vec3> = (0..33 * 33 * 33)
            .map(|i| o + Vec3::new((i % 33) as f32, (i / 33 % 33) as f32, (i / 1089) as f32))
            .collect();
        let direct = best_of(3, || {
            let mut acc = 0.0f32;
            for p in &pts {
                acc += gen.sample(*p);
            }
            std::hint::black_box(acc);
        });
        let region = best_of(3, || {
            let s = gen.region_sampler(Aabb::new(o - Vec3::splat(2.0), o + Vec3::splat(34.0)));
            let mut acc = 0.0f32;
            for p in &pts {
                acc += s.sample(*p);
            }
            std::hint::black_box(acc);
        });
        let n = pts.len() as f64;
        println!(
            "enclosure {e:3} density {d:3}: Generator::sample {:.0} ns, RegionSampler {:.0} ns",
            direct * 1e6 / n,
            region * 1e6 / n
        );
    }
}

fn main() {
    println!("== generation (generate_all, whole 8x4x8 level) ==");
    for (seed, e, d) in [
        (2024u64, 50u8, 60u8),
        (2024, 100, 40),
        (0xDEAD_BEEF, 75, 20),
        (0x00C0_FFEE, 25, 70),
        (1, 0, 0),
    ] {
        let gen = Generator::new(seed, Sliders::new(e, d));
        let t = best_of(3, || {
            let mut s = ChunkStore::new(gen.clone());
            s.generate_all();
            std::hint::black_box(&s);
        });
        println!(
            "enclosure {e:3} density {d:3}: {t:7.1} ms total, {:.2} ms/chunk",
            t / 256.0
        );
    }

    per_sample_costs();

    let gen = Generator::new(2024, Sliders::new(50, 60));
    let mut store = ChunkStore::new(gen.clone());
    store.generate_all();
    let (hit, _) = surface_point(&store);
    let all = craters(hit, 201);

    println!("== remesh, all chunks generated ==");
    let remesh = |s: &mut ChunkStore, e: TerrainEdit| {
        let t = Instant::now();
        let dirty = s.apply(e);
        let mut tris = 0;
        for c in &dirty {
            tris += s.mesh_surface_nets(*c).triangle_count();
        }
        (ms(t), dirty.len(), tris)
    };
    let mut s1 = store.clone();
    let (t, n, tris) = remesh(&mut s1, all[0]);
    println!("after 1 crater:    {t:.2} ms ({n} dirty chunks, {tris} triangles)");
    let mut s200 = store.clone();
    for e in &all[..199] {
        s200.apply(*e);
    }
    let (t, n, tris) = remesh(&mut s200, all[199]);
    println!("after 200 craters: {t:.2} ms ({n} dirty chunks, {tris} triangles)");

    println!("== remesh, lazy store (craters replayed before generation) ==");
    let chunk = store.chunk_at(hit).expect("in level");
    for count in [1usize, 200] {
        let t = best_of(3, || {
            let mut s = ChunkStore::replay(gen.clone(), &all[..count]);
            std::hint::black_box(s.mesh_surface_nets(chunk));
        });
        println!("{count:3} craters, generate + mesh chunk {chunk:?}: {t:.2} ms");
    }

    println!("== ChunkStore::eval near the craters ==");
    for (name, s) in [("0 craters", &store), ("200 craters", &s200)] {
        let mut rng = SplitMix64::new(9);
        let pts: Vec<Vec3> = (0..20_000)
            .map(|_| {
                hit + Vec3::new(
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                    rng.next_f32() - 0.5,
                ) * 16.0
            })
            .collect();
        let t = best_of(3, || {
            let mut acc = 0.0f32;
            for p in &pts {
                acc += s.sdf(*p);
            }
            std::hint::black_box(acc);
        });
        println!("{name:12}: {:.0} ns per eval", t * 1e6 / pts.len() as f64);
    }

    println!("== dual contouring (exact Hermite) ==");
    for (seed, e, d) in [(2024u64, 50u8, 60u8), (2024, 100, 40)] {
        let mut s = ChunkStore::new(Generator::new(seed, Sliders::new(e, d)));
        s.generate_all();
        let (c, tris) = s
            .chunk_coords()
            .into_iter()
            .map(|c| (c, s.mesh_surface_nets(c).triangle_count()))
            .max_by_key(|&(c, t)| (t, std::cmp::Reverse(c)))
            .unwrap();
        let t = best_of(3, || {
            std::hint::black_box(s.mesh_dual_contouring(c));
        });
        let sn = best_of(5, || {
            std::hint::black_box(s.mesh_surface_nets(c));
        });
        println!(
            "enclosure {e:3} density {d:3}, chunk {:?} ({tris} tris): DC {t:.2} ms, surface nets {sn:.2} ms",
            ChunkCoord::new(c.x, c.y, c.z)
        );
    }
}
