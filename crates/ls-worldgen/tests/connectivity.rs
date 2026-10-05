//! Playable connectivity on every slider combination: over a sweep of seeds
//! and an Enclosure x Density grid, a flood fill over a coarse clearance
//! lattice ([`ClearanceGrid::level`]) must join both team spawn regions, the
//! level centre, every network hub and the Heist site (core vein access and
//! extraction zones), and every tunnel keeps its clearance.
//!
//! Debug and wasm32 builds sweep one seed and a 4 x 4 grid; native release
//! builds sweep 7 seeds and the 5 x 5 grid (0, 25, 50, 75, 100):
//!
//! ```text
//! cargo test -p ls-worldgen --release --test connectivity
//! ```

use ls_worldgen::connectivity::{CONNECT_CLEARANCE, CONNECT_STEP};
use ls_worldgen::ls_math::Vec3;
use ls_worldgen::network::TUNNEL_RADIUS;
use ls_worldgen::{ClearanceGrid, Generator, Sliders};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// Share of the level length (x) each team's spawn region takes (as in
/// `ls_game::setup::SPAWN_ZONE_FRACTION`).
const SPAWN_REGION: f32 = 0.3;
/// Free space the tunnels guarantee along their centre line (u).
const TUNNEL_CLEARANCE: f32 = 3.0;

fn seeds() -> &'static [u64] {
    if cfg!(debug_assertions) || cfg!(target_arch = "wasm32") {
        &[0xDEAD_BEEF]
    } else {
        &[0xDEAD_BEEF, 1, 2, 42, 2024, 7777, u64::MAX]
    }
}

/// Enclosure and Density values swept (every combination).
fn steps() -> &'static [u8] {
    if cfg!(debug_assertions) || cfg!(target_arch = "wasm32") {
        &[0, 50, 75, 100]
    } else {
        &[0, 25, 50, 75, 100]
    }
}

fn check_world(seed: u64, e: u8, d: u8) {
    let gen = Generator::new(seed, Sliders::new(e, d));
    let tag = format!("seed {seed} enclosure {e} density {d}");
    let net = gen.network();
    assert_eq!(net.hubs().len(), 8, "{tag}");

    // Tunnels: at least TUNNEL_CLEARANCE of free space along every centre
    // line (sampled every 0.5 u), hubs even more.
    for path in net.tunnel_paths() {
        for w in path.windows(2) {
            let n = (w[0].distance(w[1]) / 0.5) as usize + 1;
            for i in 0..=n {
                let p = w[0] + (w[1] - w[0]) * (i as f32 / n as f32);
                let v = gen.sample(p);
                assert!(v >= TUNNEL_CLEARANCE, "{tag}: tunnel point {p:?} has {v}");
            }
        }
    }
    for h in net.hubs() {
        assert!(gen.sample(*h) >= TUNNEL_RADIUS - 1e-3, "{tag}: hub {h:?}");
    }

    let grid = ClearanceGrid::level(&gen);
    let main = grid
        .main_component(&gen)
        .unwrap_or_else(|| panic!("{tag}: no main component"));
    for h in net.hubs() {
        assert_eq!(
            grid.nearest_component(*h, CONNECT_STEP),
            Some(main),
            "{tag}: hub {h:?} is cut off"
        );
    }
    // The Heist core vein (its access point) and both extraction zones are
    // in the main volume.
    let site = gen
        .heist()
        .expect("every default-size level has a Heist site");
    let zones = site.extraction.map(|z| z.pos);
    for p in std::iter::once(site.access).chain(zones) {
        assert_eq!(
            grid.nearest_component(p, CONNECT_STEP),
            Some(main),
            "{tag}: Heist point {p:?} is cut off"
        );
    }
    // Both team spawn regions hold a good share of the main volume.
    let room = gen.room();
    let (lo_x, hi_x) = (
        room.min.x + SPAWN_REGION * room.size().x,
        room.max.x - SPAWN_REGION * room.size().x,
    );
    let (mut team0, mut team1) = (0usize, 0usize);
    let step = CONNECT_STEP;
    let mut z = room.min.z + 0.5 * step;
    while z < room.max.z {
        let mut y = room.min.y + 0.5 * step;
        while y < room.max.y {
            let mut x = room.min.x + 0.5 * step;
            while x < room.max.x {
                if x < lo_x || x > hi_x {
                    let p = Vec3::new(x, y, z);
                    if grid.component_at(p) == Some(main) {
                        if x < lo_x {
                            team0 += 1;
                        } else {
                            team1 += 1;
                        }
                    }
                }
                x += 4.0 * step;
            }
            y += 2.0 * step;
        }
        z += 2.0 * step;
    }
    // A spawn cavern alone has well over 20 lattice points on this sparse
    // walk (radius >= 7 u).
    assert!(team0 >= 8 && team1 >= 8, "{tag}: {team0} / {team1}");
    assert!(grid.clearance() == CONNECT_CLEARANCE);
}

#[test]
fn every_slider_combination_connects_both_team_regions() {
    for &seed in seeds() {
        for &e in steps() {
            for &d in steps() {
                check_world(seed ^ ((e as u64) << 8 | d as u64), e, d);
            }
        }
    }
}

/// The network follows the Enclosure character and never touches sliders
/// other than Enclosure: the same seed and Enclosure give the same hubs.
#[test]
fn network_depends_on_seed_and_enclosure_only() {
    let a = Generator::new(5, Sliders::new(40, 0));
    let b = Generator::new(5, Sliders::new(40, 100));
    assert_eq!(a.network(), b.network());
    let c = Generator::new(6, Sliders::new(40, 0));
    assert_ne!(a.network().hubs(), c.network().hubs());
}
