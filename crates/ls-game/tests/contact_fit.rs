//! Ships fit where the rock looks like they fit (VQ0, `docs/visual-quality-plan.md`).
//!
//! The cave generator's SDF is only a lower bound on the distance to the
//! rock (gradient noise divided by its worst-case slope), so the sim's sphere
//! contact uses the first-order estimate `d / |grad d|` near the surface
//! (`ls_sim::terrain::sphere_contact`). These tests fly real hulls into the
//! rock of generated cavern worlds and measure the gap to the true surface
//! at contact (found by tracing the field to its zero crossing), fly hulls
//! through narrow tunnels whose walls report a bound distance, and time
//! contact-heavy world ticks (release only, printed with `--nocapture`).

#![cfg(not(target_arch = "wasm32"))]

use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Aabb, Quat, Vec3};
use ls_game::ls_sim::{simulate_movement, InputCmd, PlayerId, ShipState, ShipTable, ShipType};
use ls_game::ls_worldgen::{ChunkStore, Generator, Sliders};

/// Small deterministic generator for probe placement.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }

    fn dir(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(
                self.next() * 2.0 - 1.0,
                self.next() * 2.0 - 1.0,
                self.next() * 2.0 - 1.0,
            );
            let l = v.length();
            if l > 0.1 && l <= 1.0 {
                return v / l;
            }
        }
    }
}

/// Distance along `dir` from `o` to the zero crossing of the field. The
/// field is a lower bound on the distance, so stepping by it never crosses
/// the surface; it converges onto it.
fn trace<T: TerrainQuery + ?Sized>(t: &T, o: Vec3, dir: Vec3, max: f32) -> Option<f32> {
    let mut s = 0.0f32;
    for _ in 0..2000 {
        let d = t.sdf(o + dir * s);
        if d < 1e-4 {
            return Some(s);
        }
        s += d;
        if s > max {
            return None;
        }
    }
    None
}

/// Evenly spread unit directions (Fibonacci sphere).
fn sphere_dirs(n: usize) -> Vec<Vec3> {
    let golden = std::f32::consts::PI * (3.0 - 5.0f32.sqrt());
    (0..n)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - y * y).sqrt();
            let a = golden * i as f32;
            Vec3::new(a.cos() * r, y, a.sin() * r)
        })
        .collect()
}

/// Euclidean distance from `p` to the true surface (minimum over traced
/// directions; a slight overestimate for curved rock).
fn true_gap<T: TerrainQuery + ?Sized>(t: &T, p: Vec3, dirs: &[Vec3], max: f32) -> f32 {
    dirs.iter()
        .filter_map(|&d| trace(t, p, d, max))
        .fold(max, f32::min)
}

fn percentile(v: &mut [f32], q: f32) -> f32 {
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * q).round() as usize]
}

struct FitStats {
    /// Gap to the surface along the contact normal.
    normal: Vec<f32>,
    /// Euclidean gap to the surface.
    euclid: Vec<f32>,
}

/// Flies `hull` ships into the rock of `store` from `probes` random starts
/// and measures the gap to the true surface at the first contact.
fn probe_fit(store: &ChunkStore, ship_type: ShipType, probes: usize, seed: u64) -> FitStats {
    let tuning = ShipTable::builtin().get(ship_type).clone();
    let r = tuning.radius;
    let dirs = sphere_dirs(96);
    let b = store.bounds();
    // Probe around the level centre (the playable middle of the cave).
    let c = b.center();
    let half = (b.max - b.min) * 0.25;
    let mut rng = Lcg(seed);
    let mut stats = FitStats {
        normal: Vec::new(),
        euclid: Vec::new(),
    };
    let mut attempts = 0;
    while stats.normal.len() < probes {
        attempts += 1;
        assert!(attempts < probes * 400, "could not place probes");
        let q = c + Vec3::new(
            (rng.next() * 2.0 - 1.0) * half.x,
            (rng.next() * 2.0 - 1.0) * half.y,
            (rng.next() * 2.0 - 1.0) * half.z,
        );
        if store.sdf(q) < 0.5 {
            continue;
        }
        let u = rng.dir();
        let Some(s) = trace(store, q, u, 24.0) else {
            continue;
        };
        let hit = q + u * s;
        let n = store.normal(hit);
        let start = hit + n * (r + 2.0);
        if store.sdf(start) <= 0.0 {
            continue;
        }
        let mut ship = ShipState::new(PlayerId(0), 0, ship_type, &tuning, start, Quat::IDENTITY);
        ship.vel = -n * 6.0;
        let mut touched = None;
        for tick in 0..90 {
            let rep = simulate_movement(&mut ship, &InputCmd::idle(tick), &tuning, store);
            if rep.contact {
                touched = Some(ship.pos);
                break;
            }
        }
        let Some(p) = touched else {
            continue;
        };
        let along = store.normal(p);
        let Some(gn) = trace(store, p, -along, 8.0) else {
            continue;
        };
        stats.normal.push(gn);
        stats.euclid.push(true_gap(store, p, &dirs, 8.0));
    }
    stats
}

#[test]
fn contact_gap_matches_the_hull_radius_in_caverns() {
    let worlds = [(7u64, 50u8, 50u8), (42, 50, 50), (99, 70, 60)];
    let probes = if cfg!(debug_assertions) { 12 } else { 40 };
    let ships = ShipTable::builtin();
    let mut failures = Vec::new();
    for &(seed, e, d) in &worlds {
        let store = ChunkStore::new(Generator::new(seed, Sliders::new(e, d)));
        for ship_type in ShipType::ALL {
            let r = ships.get(ship_type).radius;
            let mut s = probe_fit(&store, ship_type, probes, seed ^ ship_type.index() as u64);
            let n50 = percentile(&mut s.normal, 0.5);
            let n10 = percentile(&mut s.normal, 0.1);
            let n90 = percentile(&mut s.normal, 0.9);
            let e10 = percentile(&mut s.euclid, 0.1);
            let e50 = percentile(&mut s.euclid, 0.5);
            eprintln!(
                "seed {seed} ({e}/{d}) {ship_type:?} r={r}: gap along normal p10/p50/p90 \
                 {n10:.3}/{n50:.3}/{n90:.3}, euclidean p10/p50 {e10:.3}/{e50:.3}"
            );
            if (n50 - r).abs() > 0.1 {
                failures.push(format!(
                    "seed {seed} {ship_type:?}: median gap {n50:.3} vs radius {r}"
                ));
            }
            if e10 < r - 0.15 {
                failures.push(format!(
                    "seed {seed} {ship_type:?}: p10 euclidean gap {e10:.3} clips into rock (r {r})"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A straight tunnel of `radius` from `a` to `b` carved into the rock of a
/// cavern world. Its wall field is scaled by `scale` (< 1): like the cave
/// generator's noise it reports only a lower bound on the distance.
struct BoundTunnel<'a> {
    store: &'a ChunkStore,
    a: Vec3,
    b: Vec3,
    radius: f32,
    scale: f32,
}

impl BoundTunnel<'_> {
    fn axis_distance(&self, p: Vec3) -> f32 {
        let ab = self.b - self.a;
        let t = ((p - self.a).dot(ab) / ab.dot(ab)).clamp(0.0, 1.0);
        (p - (self.a + ab * t)).length()
    }
}

impl TerrainQuery for BoundTunnel<'_> {
    fn sdf(&self, p: Vec3) -> f32 {
        let tunnel = self.scale * (self.radius - self.axis_distance(p));
        self.store.sdf(p).max(tunnel)
    }

    fn bounds(&self) -> Aabb {
        self.store.bounds()
    }
}

/// A 16 u stretch of solid rock (every sample along it and 1.5 u around it
/// at least 1 u deep) in `store`, along `dir`.
fn solid_line(store: &ChunkStore, dir: Vec3) -> (Vec3, Vec3) {
    let b = store.bounds();
    let side = dir.any_orthonormal();
    let up = dir.cross(side);
    let mut rng = Lcg(0x5eed);
    for _ in 0..200_000 {
        let size = b.max - b.min;
        let a = b.min
            + Vec3::new(
                size.x * (0.15 + 0.7 * rng.next()),
                size.y * (0.15 + 0.7 * rng.next()),
                size.z * (0.15 + 0.7 * rng.next()),
            );
        let solid = (0..=32).all(|k| {
            let p = a + dir * (k as f32 * 0.5);
            [Vec3::ZERO, side, -side, up, -up]
                .iter()
                .all(|&o| store.sdf(p + o * 1.5) < -1.0)
        });
        if solid {
            return (a, a + dir * 16.0);
        }
    }
    panic!("no solid rock found");
}

/// Flies `ship_type` through a tunnel of `diameter`; returns how far along
/// it the ship got and the worst overlap of its hull with the tunnel wall.
fn fly_tunnel(ship_type: ShipType, diameter: f32) -> (f32, f32) {
    let store = ChunkStore::new(Generator::new(7, Sliders::new(50, 50)));
    let dir = Vec3::new(1.0, 0.2, -0.3).normalize();
    let (a, b) = solid_line(&store, dir);
    let tunnel = BoundTunnel {
        store: &store,
        a,
        b,
        radius: diameter * 0.5,
        scale: 0.28,
    };
    let tuning = ShipTable::builtin().get(ship_type).clone();
    let r = tuning.radius;
    let mut ship = ShipState::new(
        PlayerId(0),
        0,
        ship_type,
        &tuning,
        a + dir * 1.0,
        Quat::from_rotation_arc(Vec3::NEG_Z, dir),
    );
    let mut worst = 0.0f32;
    let mut progress = 0.0f32;
    for tick in 0..900 {
        let input = InputCmd::idle(tick).with_thrust(Vec3::NEG_Z);
        simulate_movement(&mut ship, &input, &tuning, &tunnel);
        // Overlap of the hull with the exact tunnel wall.
        worst = worst.max(tunnel.axis_distance(ship.pos) + r - tunnel.radius);
        progress = progress.max((ship.pos - a).dot(dir));
    }
    (progress, worst)
}

#[test]
fn a_tinker_flies_through_a_1_2_u_tunnel() {
    let (progress, overlap) = fly_tunnel(ShipType::Tinker, 1.2);
    assert!(progress > 14.0, "stuck after {progress} u of 16");
    assert!(overlap < 0.05, "hull {overlap} u inside the wall");
}

#[test]
fn a_hauler_flies_through_a_1_5_u_tunnel() {
    let (progress, overlap) = fly_tunnel(ShipType::Hauler, 1.5);
    assert!(progress > 14.0, "stuck after {progress} u of 16");
    assert!(overlap < 0.05, "hull {overlap} u inside the wall");
}

/// Tick cost of a cave match where every ship keeps flying into the rock.
/// Release only; prints mean, p50, p99 and max per tick.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing: run with --release")]
fn contact_heavy_tick_cost() {
    use ls_game::ls_worldgen::ShareCode;
    use ls_game::{build_match, MatchConfig};
    use std::time::Instant;

    let code = ShareCode::new(7, Sliders::new(50, 50));
    let (mut world, mut terrain) = build_match(&MatchConfig::paint_the_void(code)).unwrap();
    let mut ids = Vec::new();
    for i in 0..8u8 {
        ids.push(
            world
                .add_player(Some(i % 2), ShipType::ALL[i as usize % 4])
                .unwrap(),
        );
    }
    let mut rng = Lcg(99);
    let mut times = Vec::new();
    let mut contact_ticks = 0u64;
    for tick in 0..1800u64 {
        let inputs: Vec<_> = ids
            .iter()
            .map(|&id| {
                let mut c = InputCmd::idle(tick).with_thrust(Vec3::new(
                    rng.next() - 0.5,
                    rng.next() - 0.5,
                    -1.0,
                ));
                c.look_yaw = (rng.next() - 0.5) * 0.08;
                c.look_pitch = (rng.next() - 0.5) * 0.08;
                (id, c)
            })
            .collect();
        let t0 = Instant::now();
        world.step(&mut terrain, &inputs);
        times.push(t0.elapsed().as_secs_f32() * 1000.0);
        contact_ticks += world.ships().filter(|s| s.alive && s.in_contact).count() as u64;
    }
    // Skip the first second (respawn, chunk generation).
    let mut t = times[60..].to_vec();
    let mean = t.iter().sum::<f32>() / t.len() as f32;
    let p50 = percentile(&mut t, 0.5);
    let p99 = percentile(&mut t, 0.99);
    let max = percentile(&mut t, 1.0);
    eprintln!(
        "contact-heavy tick (8 ships, seed 7 caverns): mean {mean:.3} ms, p50 {p50:.3} ms, \
         p99 {p99:.3} ms, max {max:.3} ms; ship-ticks in contact {contact_ticks} of {}",
        1800 * 8
    );
    assert!(p99 < 4.0, "tick p99 {p99} ms");

    // One Tinker thrusting into a cave wall: cost of a movement step in
    // contact and in open flight.
    let store = ChunkStore::new(Generator::new(7, Sliders::new(50, 50)));
    let tuning = ShipTable::builtin().get(ShipType::Tinker).clone();
    let c = store.bounds().center();
    let mut rng = Lcg(7);
    let (start, n) = loop {
        let q = c + rng.dir() * (rng.next() * 40.0);
        if store.sdf(q) < 0.5 {
            continue;
        }
        let u = rng.dir();
        let Some(s) = trace(&store, q, u, 24.0) else {
            continue;
        };
        let hit = q + u * s;
        let n = store.normal(hit);
        // Open space 8 u out from the wall for the open-flight run.
        if store.sdf(hit + n * 8.0) > 2.0 {
            break (hit, n);
        }
    };
    let run = |pos: Vec3, facing: Vec3| {
        let mut ship = ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            &tuning,
            pos,
            Quat::from_rotation_arc(Vec3::NEG_Z, facing),
        );
        let input = InputCmd::idle(0).with_thrust(Vec3::NEG_Z);
        let mut contact = 0u32;
        let t0 = Instant::now();
        for _ in 0..20_000 {
            ship.vel = facing * 8.0;
            ship.pos = pos;
            if simulate_movement(&mut ship, &input, &tuning, &store).contact {
                contact += 1;
            }
        }
        (t0.elapsed().as_secs_f32() * 1e6 / 20_000.0, contact)
    };
    let (wall_us, wall_contacts) = run(start + n * 0.5, -n);
    let (open_us, _) = run(start + n * 8.0, n);
    eprintln!(
        "Tinker movement step: {wall_us:.2} us against the wall ({wall_contacts} of 20000 in contact), {open_us:.2} us in open flight"
    );
}
