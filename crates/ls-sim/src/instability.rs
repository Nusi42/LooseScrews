//! Instability (M4; `docs/milestone-4-plan.md` §3.3 "Instability").
//!
//! - **Weaker rock.** Every crater's radius ([`crate::Ctx::crater`]) is
//!   multiplied by `1 + crater_scale_at_100 * instability / 100` on top of
//!   the rock strength (exactly 1 at Instability 0, so existing replays keep
//!   their hashes).
//! - **Strain.** Every `SubtractSphere` edit the world applies adds strain
//!   to the 16 u cell of its centre: its sphere volume times the material
//!   factor at the centre (weak 1.5, rock 1, hard 0.5). Strain decays 3 %
//!   per second (applied once per second).
//! - **Cave-ins.** When a cell's strain passes the threshold
//!   ([`InstabilitySettings::threshold`], falling with Instability, infinite
//!   at 0) the tunnel at the cell's strain-weighted crater centre caves in
//!   after a 1.5 s warning (`CueKind::CaveInWarning` with the position and
//!   the warning ticks): a hollow in the ceiling (the first rock along the
//!   level's `up`), 5-9 rubble spheres (1.5-3 u) hanging from it into the
//!   tunnel, 4-8 debris pieces with scrap, and 20 damage (no attacker) to
//!   ships within 3 u of the rubble. Rubble is weak rock: the world keeps
//!   [`RubbleSphere`] regions that override the material (hashed). Spheres
//!   that would overlap a ship are left out (Seed Grenade rule), and a
//!   cave-in never encloses a ship: for every ship near it, a flood fill on
//!   a 1 u grid must still reach open space 14 u away with the rubble
//!   (if it did without), else the rubble nearest that ship is left out
//!   until it does. At most one cave-in per 8 s per level; none within
//!   `spawn_clearance` of a spawn point, in the Heist chamber, or within
//!   6 u of an extraction zone; none where no ceiling is in reach.
//!
//! Randomness: the cave-in's shape comes from a private RNG keyed by the
//! match seed, the tick and [`CAVE_IN_RNG_KEY`] (like `Ctx::rng`).
//! Numbers: `data/world/instability.ron` ([`InstabilitySettings`]).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::sync::OnceLock;

use ls_math::terrain::{TerrainQuery, MATERIAL_HARD, MATERIAL_WEAK};
use ls_math::{det, Pcg32, Vec3};
use serde::{Deserialize, Serialize};

use crate::hash::StateHasher;

/// `data/world/instability.ron` (embedded, part of the data hash).
pub const INSTABILITY_RON: &str = include_str!("../../../data/world/instability.ron");

/// Key of the cave-in RNG (`Ctx::rng` style: mixed with seed and tick).
pub const CAVE_IN_RNG_KEY: u64 = 0x4341_5645_494e; // "CAVEIN"

/// Tuning of Instability (`data/world/instability.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InstabilitySettings {
    /// Extra crater radius at Instability 100 (0.4: 1.4x).
    pub crater_scale_at_100: f32,
    /// Edge of a strain cell (u).
    pub cell_size: f32,
    /// Share of the strain lost per second.
    pub decay_per_sec: f32,
    /// Strain (u^3 of crater volume) that caves a cell in at Instability 100.
    pub threshold_at_100: f32,
    /// The threshold at Instability `i` is `threshold_at_100 * (100 / i)^exponent`.
    pub threshold_exponent: f32,
    /// Strain factor per material (weak, rock, hard).
    pub material_factor: (f32, f32, f32),
    /// Warning before the rock comes down (s).
    pub warning_secs: f32,
    /// At most one cave-in per this many seconds per level.
    pub min_interval_secs: f32,
    /// How far along `up` the ceiling is searched (u).
    pub ceiling_search: f32,
    /// Rubble spheres per cave-in (inclusive range).
    pub rubble_count: (u8, u8),
    /// Rubble sphere radius (u).
    pub rubble_radius: (f32, f32),
    /// Rubble centres scatter this far sideways (u).
    pub rubble_spread: f32,
    /// Hollow radius per cube root of the rubble volume (r1^3 + r2^3 ...).
    pub hollow_scale: f32,
    /// Debris pieces per cave-in (inclusive range).
    pub debris_count: (u8, u8),
    /// Scrap in each debris piece.
    pub debris_scrap: (f32, f32),
    /// Damage to ships within `damage_radius` of a rubble sphere.
    pub damage: f32,
    pub damage_radius: f32,
    /// Rubble keeps this much room around a ship (beyond its radius).
    pub ship_clearance: f32,
    /// No cave-in within this distance of a spawn point.
    pub spawn_clearance: f32,
    /// No cave-in within this distance of an extraction zone (beyond its radius).
    pub extraction_clearance: f32,
    /// Escape check: a ship must reach open space this far away (u).
    pub escape_reach: f32,
    /// Rubble regions kept (oldest dropped first).
    pub max_rubble: u16,
}

impl InstabilitySettings {
    /// The embedded `data/world/instability.ron`.
    pub fn builtin() -> &'static InstabilitySettings {
        static S: OnceLock<InstabilitySettings> = OnceLock::new();
        S.get_or_init(|| {
            ron::from_str(INSTABILITY_RON).expect("data/world/instability.ron is valid (tested)")
        })
    }

    /// Crater radius multiplier at `instability` (exactly 1 at 0).
    pub fn crater_scale(&self, instability: u8) -> f32 {
        if instability == 0 {
            1.0
        } else {
            1.0 + self.crater_scale_at_100 * instability.min(100) as f32 / 100.0
        }
    }

    /// Strain that caves a cell in at `instability` (infinite at 0).
    pub fn threshold(&self, instability: u8) -> f32 {
        if instability == 0 {
            return f32::INFINITY;
        }
        let i = instability.min(100) as f32;
        self.threshold_at_100 * det::powf(100.0 / i, self.threshold_exponent)
    }

    /// Strain factor of a material id (`ls_math::terrain::MATERIAL_*`).
    pub fn factor(&self, material: u8) -> f32 {
        match material {
            MATERIAL_WEAK => self.material_factor.0,
            MATERIAL_HARD => self.material_factor.2,
            _ => self.material_factor.1,
        }
    }
}

/// A rubble region: weak rock left by a cave-in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RubbleSphere {
    pub center: Vec3,
    pub radius: f32,
}

/// Strain of one cell.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct StrainCell {
    pub strain: f32,
    /// Strain-weighted sum of crater centres (centroid = `sum / strain`
    /// before decay; decay scales both).
    pub sum: Vec3,
}

/// A cave-in that is coming (warning phase).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PendingCaveIn {
    pub pos: Vec3,
    /// The rock comes down at the start of this tick's physics stage.
    pub due_tick: u64,
}

/// The Instability state of a match (part of the state hash when not empty).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InstabilityState {
    pub cells: BTreeMap<[i32; 3], StrainCell>,
    pub pending: Option<PendingCaveIn>,
    /// Tick of the last cave-in (its warning start).
    pub last_cave_in: Option<u64>,
    pub rubble: Vec<RubbleSphere>,
}

/// What a cave-in does to the level ([`plan_cave_in`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CaveInPlan {
    /// The hollow cut into the ceiling.
    pub hollow: Option<RubbleSphere>,
    /// Rubble spheres added (weak rock).
    pub rubble: Vec<RubbleSphere>,
    /// Debris pieces: `(pos, vel, radius, scrap)`.
    pub debris: Vec<(Vec3, Vec3, f32, f32)>,
}

impl InstabilityState {
    /// Whether nothing is tracked (keeps the state hash of quiet matches).
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
            && self.pending.is_none()
            && self.last_cave_in.is_none()
            && self.rubble.is_empty()
    }

    fn cell_of(settings: &InstabilitySettings, p: Vec3) -> [i32; 3] {
        let s = settings.cell_size;
        [
            det::floor(p.x / s) as i32,
            det::floor(p.y / s) as i32,
            det::floor(p.z / s) as i32,
        ]
    }

    /// A crater of `radius` at `center` in `material` adds strain.
    pub fn add_strain(
        &mut self,
        settings: &InstabilitySettings,
        center: Vec3,
        radius: f32,
        material: u8,
    ) {
        if radius.is_nan() || radius <= 0.0 || !center.is_finite() {
            return;
        }
        let volume = 4.0 / 3.0 * std::f32::consts::PI * radius * radius * radius;
        let strain = volume * settings.factor(material);
        let cell = self
            .cells
            .entry(Self::cell_of(settings, center))
            .or_default();
        cell.strain += strain;
        cell.sum += center * strain;
    }

    /// One second of decay; cells below a trace are dropped.
    pub fn decay(&mut self, settings: &InstabilitySettings) {
        let k = 1.0 - settings.decay_per_sec;
        self.cells.retain(|_, c| {
            c.strain *= k;
            c.sum *= k;
            c.strain > 0.01
        });
    }

    /// The cell over the threshold with the most strain (ties: the lowest
    /// cell key), taken out of the map; returns its strain-weighted centre.
    /// Cells whose centre `excluded` rejects are cleared and skipped.
    pub fn take_over_threshold(
        &mut self,
        threshold: f32,
        excluded: impl Fn(Vec3) -> bool,
    ) -> Option<Vec3> {
        loop {
            let mut best: Option<([i32; 3], f32)> = None;
            for (k, c) in &self.cells {
                if c.strain >= threshold && best.is_none_or(|(_, s)| c.strain > s) {
                    best = Some((*k, c.strain));
                }
            }
            let (key, _) = best?;
            let c = self.cells.remove(&key)?;
            let pos = c.sum / c.strain;
            if !excluded(pos) {
                return Some(pos);
            }
        }
    }

    /// Material override of rubble regions (weak rock), if `p` is in one.
    pub fn material_at(&self, p: Vec3) -> Option<u8> {
        self.rubble
            .iter()
            .any(|r| p.distance_squared(r.center) <= r.radius * r.radius)
            .then_some(MATERIAL_WEAK)
    }

    /// Keeps a cave-in's rubble regions (the oldest go past the cap).
    pub fn add_rubble(&mut self, settings: &InstabilitySettings, rubble: &[RubbleSphere]) {
        self.rubble.extend_from_slice(rubble);
        let max = settings.max_rubble as usize;
        if self.rubble.len() > max {
            let drop = self.rubble.len() - max;
            self.rubble.drain(..drop);
        }
    }

    /// Hashes the state (call only when not empty).
    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.cells.len() as u32);
        for (k, c) in &self.cells {
            for v in k {
                h.write_u32(*v as u32);
            }
            h.write_f32(c.strain);
            h.write_vec3(c.sum);
        }
        match self.pending {
            None => h.write_u8(0),
            Some(p) => {
                h.write_u8(1);
                h.write_vec3(p.pos);
                h.write_u64(p.due_tick);
            }
        }
        h.write_u64(self.last_cave_in.unwrap_or(u64::MAX));
        h.write_u32(self.rubble.len() as u32);
        for r in &self.rubble {
            h.write_vec3(r.center);
            h.write_f32(r.radius);
        }
    }
}

/// A unit vector perpendicular to `up` at `angle`.
fn lateral(up: Vec3, angle: f32) -> Vec3 {
    let a = up.any_orthonormal();
    let b = up.cross(a);
    let (s, c) = det::sin_cos(angle);
    a * c + b * s
}

/// Plans a cave-in at `pos` (see the module docs): `None` when no ceiling
/// is in reach along `up`. `ships` are `(position, radius)` of the living
/// ships; rubble never overlaps or encloses one.
pub fn plan_cave_in<T: TerrainQuery + ?Sized>(
    settings: &InstabilitySettings,
    terrain: &T,
    up: Vec3,
    pos: Vec3,
    rng: &mut Pcg32,
    ships: &[(Vec3, f32)],
) -> Option<CaveInPlan> {
    // Start in the open next to the crater centre.
    let mut start = pos;
    let d = terrain.sdf(start);
    if d < 1.0 {
        start += terrain.normal(start) * (1.0 - d);
    }
    if terrain.sdf(start) <= 0.0 {
        return None;
    }
    let hit = terrain.raycast(start, up, settings.ceiling_search)?;
    let ceiling = hit.point;
    let (lo, hi) = settings.rubble_count;
    let n = lo as i32 + rng.below((hi.saturating_sub(lo)) as u32 + 1) as i32;
    let mut rubble = Vec::new();
    let mut volume = 0.0f32;
    for _ in 0..n {
        let r = rng.range_f32(settings.rubble_radius.0, settings.rubble_radius.1);
        let side = lateral(up, rng.range_f32(0.0, std::f32::consts::TAU))
            * rng.range_f32(0.0, settings.rubble_spread);
        let depth = r * rng.range_f32(0.2, 0.8);
        let center = ceiling - up * depth + side;
        volume += r * r * r;
        let clear = ships.iter().all(|&(p, sr)| {
            let min = r + sr + settings.ship_clearance;
            p.distance_squared(center) >= min * min
        });
        if clear {
            rubble.push(RubbleSphere { center, radius: r });
        }
    }
    let hr = settings.hollow_scale * det::powf(volume.max(1e-6), 1.0 / 3.0);
    let hollow = RubbleSphere {
        center: ceiling + up * (0.6 * hr),
        radius: hr,
    };
    let (lo, hi) = settings.debris_count;
    let k = lo as u32 + rng.below((hi.saturating_sub(lo)) as u32 + 1);
    let mut debris = Vec::new();
    for _ in 0..k {
        let side = lateral(up, rng.range_f32(0.0, std::f32::consts::TAU));
        let p = ceiling - up * rng.range_f32(1.0, 3.0) + side * rng.range_f32(0.0, 3.0);
        let vel = -up * rng.range_f32(1.0, 4.0) + side * rng.range_f32(0.0, 1.5);
        let radius = rng.range_f32(0.4, 1.0);
        let scrap = rng.range_f32(settings.debris_scrap.0, settings.debris_scrap.1);
        debris.push((p, vel, radius, scrap));
    }
    // Never enclose a ship: a ship that could reach open space before must
    // still reach it with the rubble (checked only near the rubble, and
    // without the rubble only when the check with it fails).
    let reach = settings.escape_reach;
    for &(p, sr) in ships {
        let near = rubble
            .iter()
            .any(|s| p.distance(s.center) - s.radius < reach);
        if !near || escapes(terrain, &rubble, p, sr, reach) {
            continue;
        }
        if !escapes(terrain, &[], p, sr, reach) {
            continue;
        }
        while !rubble.is_empty() && !escapes(terrain, &rubble, p, sr, reach) {
            let (i, _) = rubble
                .iter()
                .enumerate()
                .map(|(i, r)| (i, r.center.distance_squared(p)))
                .fold(
                    (0, f32::INFINITY),
                    |best, x| if x.1 < best.1 { x } else { best },
                );
            rubble.remove(i);
        }
    }
    Some(CaveInPlan {
        hollow: Some(hollow),
        rubble,
        debris,
    })
}

/// Whether a sphere of radius `r` at `from` can reach a point `reach` u
/// away (Chebyshev distance on a 1 u grid, 6-connected) through open space
/// that is outside every sphere of `rubble`. A best-first search that
/// always grows the cell farthest out (ties: the oldest), so in the open it
/// walks straight to the boundary and only a sealed pocket is explored
/// whole.
pub fn escapes<T: TerrainQuery + ?Sized>(
    terrain: &T,
    rubble: &[RubbleSphere],
    from: Vec3,
    r: f32,
    reach: f32,
) -> bool {
    let n = det::ceil(reach) as i32;
    let free = |c: [i32; 3]| {
        let p = from + Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32);
        terrain.sdf(p) > r
            && rubble.iter().all(|s| {
                let min = s.radius + r;
                p.distance_squared(s.center) > min * min
            })
    };
    let cheb = |c: [i32; 3]| c[0].abs().max(c[1].abs()).max(c[2].abs());
    let mut seen: BTreeSet<[i32; 3]> = BTreeSet::new();
    // Max-heap on (distance out, -insertion order).
    let mut open: BinaryHeap<(i32, Reverse<u32>, [i32; 3])> = BinaryHeap::new();
    let mut seq = 0u32;
    seen.insert([0, 0, 0]);
    open.push((0, Reverse(seq), [0, 0, 0]));
    while let Some((d, _, c)) = open.pop() {
        if d >= n {
            return true;
        }
        for (axis, step) in [(0, 1), (0, -1), (1, 1), (1, -1), (2, 1), (2, -1)] {
            let mut nb = c;
            nb[axis] += step;
            if !seen.insert(nb) {
                continue;
            }
            if free(nb) {
                seq += 1;
                open.push((cheb(nb), Reverse(seq), nb));
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::{BoxRoomTerrain, MATERIAL_ROCK};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn settings_parse_and_scale() {
        let s = InstabilitySettings::builtin();
        assert_eq!(s.crater_scale(0), 1.0);
        assert_eq!(s.crater_scale(100), 1.4);
        assert!((s.crater_scale(50) - 1.2).abs() < 1e-6);
        assert_eq!(s.threshold(0), f32::INFINITY);
        assert!(s.threshold(30) > s.threshold(100));
        assert!(s.threshold(1) > s.threshold(30));
        assert_eq!(s.factor(MATERIAL_WEAK), 1.5);
        assert_eq!(s.factor(MATERIAL_ROCK), 1.0);
        assert_eq!(s.factor(MATERIAL_HARD), 0.5);
    }

    #[test]
    fn strain_accumulates_decays_and_is_taken_by_cell() {
        let s = InstabilitySettings::builtin();
        let mut st = InstabilityState::default();
        st.add_strain(s, Vec3::new(1.0, 1.0, 1.0), 1.0, MATERIAL_ROCK);
        st.add_strain(s, Vec3::new(3.0, 1.0, 1.0), 1.0, MATERIAL_WEAK);
        st.add_strain(s, Vec3::new(40.0, 1.0, 1.0), 1.0, MATERIAL_ROCK);
        assert_eq!(st.cells.len(), 2);
        let v = 4.0 / 3.0 * std::f32::consts::PI;
        let c = st.cells[&[0, 0, 0]];
        assert!((c.strain - 2.5 * v).abs() < 1e-4);
        st.decay(s);
        assert!((st.cells[&[0, 0, 0]].strain - 2.5 * v * 0.97).abs() < 1e-4);
        // Weighted centre: (1 * 1 + 1.5 * 3) / 2.5 = 2.2.
        let pos = st.take_over_threshold(2.0 * v, |_| false).unwrap();
        assert!((pos.x - 2.2).abs() < 1e-4, "{pos:?}");
        assert_eq!(st.cells.len(), 1);
        assert!(st.take_over_threshold(2.0 * v, |_| false).is_none());
        // Excluded cells are cleared.
        assert!(st.take_over_threshold(0.5 * v, |_| true).is_none());
        assert!(st.cells.is_empty());
    }

    #[test]
    fn escape_check_sees_a_sealed_pocket() {
        // A 6 u wide box room: open in every direction up to its walls.
        let room = BoxRoomTerrain::centered(Vec3::splat(60.0));
        assert!(escapes(&room, &[], Vec3::ZERO, 0.5, 10.0));
        // Wrapped in a shell of overlapping spheres: no way out.
        let mut shell = Vec::new();
        for i in -4..=4 {
            for j in -4..=4 {
                for k in -4..=4 {
                    let p = Vec3::new(i as f32, j as f32, k as f32) * 1.5;
                    let l = p.length();
                    if (5.0..6.5).contains(&l) {
                        shell.push(RubbleSphere {
                            center: p,
                            radius: 1.2,
                        });
                    }
                }
            }
        }
        assert!(!escapes(&room, &shell, Vec3::ZERO, 0.5, 10.0));
    }
}
