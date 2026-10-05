//! Gravity well sites of classic levels (Gravity Chaos, generator version 4).
//!
//! Each well is a rock core ([`WELL_CORE_RADIUS`]) in a carved hollow
//! ([`WELL_HOLLOW_RADIUS`]); in open-sky levels (Enclosure below
//! [`FLOATING_ENCLOSURE`]) the core is a floating stone
//! ([`WellKind::Floating`]). The generator carves the hollow and adds the
//! core after the connection network (see [`crate::Generator::sample`]).
//!
//! Placement ([`place`]) draws candidates from its own `SplitMix64` stream
//! ([`WELL_SALT`], "GRAVWELL"): points beside the non-spawn caverns and
//! beside the tunnels of the connection network, plus open points of the
//! pristine level that reach the network. A candidate is taken, in seeded
//! order, when
//!
//! - it is at least [`WELL_SPAWN_DISTANCE`] plus the largest spawn cavern
//!   radius from every spawn cavern (the network's end hubs and the team 2/3
//!   caverns), so no cavern a team spawns in lies within 40 u of a well,
//! - its core keeps [`ls_math`]-exact `core + TUNNEL_RADIUS` from every network
//!   centre line (the tunnels keep their clearance),
//! - it is outside the Heist vein and its extraction caverns,
//! - its hollow keeps [`crate::network::WALL_GAP`] from the sealing walls,
//! - it is at least [`WELL_SPACING`] from the wells taken before.
//!
//! The hollow joins the open space next to it with a smooth maximum
//! ([`crate::network::WELL_BLEND`]), so it leaves no paper-thin sheets of
//! rock.
//!
//! The count is `1 + Gravity Chaos / 15` (at most 7, none at 0) where they
//! fit; small or tall levels may get fewer.

use ls_math::level::{WellKind, WellSite};
use ls_math::rng::SplitMix64;
use ls_math::Vec3;

use crate::network::{Network, SPAWN_HUB_RADIUS, TUNNEL_RADIUS, WALL_GAP};
use crate::Generator;

/// Salt of the well placement stream ("GRAVWELL").
pub const WELL_SALT: u64 = 0x4752_4156_5745_4c4c;
/// Radius of the open hollow around a well's core (u).
pub const WELL_HOLLOW_RADIUS: f32 = 8.0;
/// Radius range of a well's rock core (u).
pub const WELL_CORE_RADIUS: [f32; 2] = [1.5, 2.5];
/// Least distance between two wells (u).
pub const WELL_SPACING: f32 = 30.0;
/// Least distance from a well to a spawn cavern's open space (u).
pub const WELL_SPAWN_DISTANCE: f32 = 40.0;
/// Pull range at Gravity Chaos 0 and 100 (u).
pub const WELL_RANGE: [f32; 2] = [20.0, 40.0];
/// Most wells per level.
pub const MAX_WELLS: usize = 7;
/// Below this Enclosure the cores are floating stones in open sky.
pub const FLOATING_ENCLOSURE: u8 = 25;
/// Extra free space kept around the Heist vein and extraction caverns (u).
const HEIST_GAP: f32 = 4.0;
/// Distance of the cavern and tunnel candidates from the centre lines (u).
const SIDE_OFFSET: f32 = 7.5;
/// Random open-space candidates tried.
const OPEN_CANDIDATES: usize = 256;

/// Number of wells for a Gravity Chaos value: none at 0, else
/// `1 + slider / 15`, at most [`MAX_WELLS`].
pub fn well_count(gravity_chaos: u8) -> usize {
    if gravity_chaos == 0 {
        0
    } else {
        (1 + gravity_chaos.min(100) as usize / 15).min(MAX_WELLS)
    }
}

/// Strength (0..=1) of a well for a Gravity Chaos value.
pub fn well_strength(gravity_chaos: u8) -> f32 {
    gravity_chaos.min(100) as f32 / 100.0
}

/// Pull range (u) of a well for a Gravity Chaos value.
pub fn well_range(gravity_chaos: u8) -> f32 {
    let t = gravity_chaos.min(100) as f32 / 100.0;
    WELL_RANGE[0] + (WELL_RANGE[1] - WELL_RANGE[0]) * t
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Distance from `p` to the nearest network centre line (tunnels; hubs are
/// tunnel end points).
fn centre_line_distance(net: &Network, p: Vec3) -> f32 {
    let mut best = f32::INFINITY;
    for path in net.tunnel_paths() {
        for w in path.windows(2) {
            best = best.min(segment_distance(p, w[0], w[1]));
        }
    }
    for h in net.hubs() {
        best = best.min(p.distance(*h));
    }
    best
}

/// The level's well sites (see the module docs). `gen` is the generator
/// without wells (its pristine level decides which open points qualify).
pub fn place(gen: &Generator, seed: u64, gravity_chaos: u8, enclosure: u8) -> Vec<WellSite> {
    let want = well_count(gravity_chaos);
    let net = gen.network();
    if want == 0 || net.hubs().is_empty() {
        return Vec::new();
    }
    let room = gen.room();
    let mut rng = SplitMix64::new(seed ^ WELL_SALT);
    let unit = |rng: &mut SplitMix64| rng.next_f32();
    let dir = |rng: &mut SplitMix64| {
        let a = rng.next_f32() * core::f32::consts::TAU;
        let (s, c) = ls_math::det::sin_cos(a);
        Vec3::new(c, 0.0, s)
    };

    // Candidates: beside the non-spawn caverns, beside the tunnels, then
    // open points of the pristine level that reach the network.
    let spawn_hubs: Vec<Vec3> = (0..4u8)
        .flat_map(|t| net.team_hubs(t).iter().copied())
        .collect();
    let mut cands: Vec<Vec3> = Vec::new();
    for h in net.hubs() {
        if spawn_hubs.contains(h) {
            continue;
        }
        for _ in 0..6 {
            cands.push(*h + dir(&mut rng) * SIDE_OFFSET);
        }
    }
    for path in net.tunnel_paths() {
        for w in path.windows(2) {
            let mid = (w[0] + w[1]) * 0.5;
            let d = w[1] - w[0];
            let side = Vec3::new(-d.z, 0.0, d.x)
                .try_normalize()
                .unwrap_or_else(|| dir(&mut rng));
            let sign = if unit(&mut rng) < 0.5 { -1.0 } else { 1.0 };
            cands.push(mid + side * (SIDE_OFFSET * sign));
        }
    }
    for _ in 0..OPEN_CANDIDATES {
        let t = Vec3::new(unit(&mut rng), unit(&mut rng), unit(&mut rng));
        let size = room.size();
        let p = room.min + Vec3::new(t.x * size.x, t.y * size.y, t.z * size.z);
        if gen.sample(p) >= 3.0 && gen.reaches_network(p, 1.5, 4) {
            cands.push(p);
        }
    }
    // Seeded order (Fisher-Yates).
    for i in (1..cands.len()).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        cands.swap(i, j);
    }

    let heist = gen.heist().copied();
    let spawn_keep = WELL_SPAWN_DISTANCE + SPAWN_HUB_RADIUS[1];
    let wall_keep = WELL_HOLLOW_RADIUS + WALL_GAP;
    let kind = if enclosure < FLOATING_ENCLOSURE {
        WellKind::Floating
    } else {
        WellKind::Core
    };
    let mut wells: Vec<WellSite> = Vec::new();
    for c in cands {
        if wells.len() >= want {
            break;
        }
        let inside =
            (c - room.min).min_element() >= wall_keep && (room.max - c).min_element() >= wall_keep;
        if !inside
            || spawn_hubs.iter().any(|h| h.distance(c) < spawn_keep)
            || wells.iter().any(|w| w.pos.distance(c) < WELL_SPACING)
            || centre_line_distance(net, c) < WELL_CORE_RADIUS[1] + TUNNEL_RADIUS
        {
            continue;
        }
        if let Some(h) = heist {
            if c.distance(h.core) < h.chamber_radius + WELL_HOLLOW_RADIUS + HEIST_GAP
                || h.extraction
                    .iter()
                    .any(|z| c.distance(z.pos) < z.radius + WELL_HOLLOW_RADIUS + HEIST_GAP)
            {
                continue;
            }
        }
        let core =
            WELL_CORE_RADIUS[0] + (WELL_CORE_RADIUS[1] - WELL_CORE_RADIUS[0]) * unit(&mut rng);
        wells.push(WellSite {
            pos: c,
            core_radius: core,
            hollow_radius: WELL_HOLLOW_RADIUS,
            strength: well_strength(gravity_chaos),
            range: well_range(gravity_chaos),
            kind,
        });
    }
    wells
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn count_strength_and_range_follow_the_slider() {
        assert_eq!(well_count(0), 0);
        assert_eq!(well_count(1), 1);
        assert_eq!(well_count(15), 2);
        assert_eq!(well_count(60), 5);
        assert_eq!(well_count(100), 7);
        assert_eq!(well_strength(100), 1.0);
        assert_eq!(well_range(0), 20.0);
        assert_eq!(well_range(100), 40.0);
    }
}
