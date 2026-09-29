//! Match setup from a share code and a rule set: deterministic team spawn
//! points and pickup points in open space, and a ready-to-step [`World`].
//!
//! Placement draws from its own PCG32 streams ([`RNG_STREAM_SPAWNS`],
//! [`RNG_STREAM_PICKUP_POINTS`]) seeded with the world seed, and only reads the
//! pristine generated terrain, so every peer computes the same setup from the
//! same share code (see `docs/architecture/contracts.md`).
//!
//! 1. Random candidates in the team's spawn zone (or the whole level for
//!    pickups) snapped to nearby open space with the preferred clearance
//!    ([`SPAWN_CLEARANCE`], [`PICKUP_CLEARANCE`]).
//! 2. If that finds nothing (dense levels, high Enclosure, often have no point
//!    at all with that much clearance, because their SDF is a conservative
//!    distance bound), points are drawn from a lattice scan of the level
//!    ([`SCAN_STEP`]) with less clearance, down to [`MIN_CLEARANCE`]
//!    ([`SPAWN_CLEARANCE_LEVELS`], [`PICKUP_CLEARANCE_LEVELS`]); spawn
//!    searches widen from the zone to the team's half of the level and to a
//!    finer lattice ([`FINE_SCAN_STEP`]). Step 1 is skipped when the scan
//!    shows it cannot succeed, which keeps setup fast on every level.
//!
//! Every slider combination gets a valid setup (see the sweep test).

use ls_math::rng::mix64;
use ls_math::terrain::TerrainQuery;
use ls_math::{Aabb, Pcg32, Quat, Vec3};
use ls_sim::{MatchSetup, RuleSet, ShipTuning, SpawnPoint, WeaponTable, World, WorldConfig};
use ls_worldgen::{Generator, ShareCode};

use crate::{GameError, GameTerrain};

/// PCG32 stream for spawn point placement ("GAME_SPN").
pub const RNG_STREAM_SPAWNS: u64 = 0x4741_4d45_5f53_504e;
/// PCG32 stream for pickup point placement ("GAME_PKP").
pub const RNG_STREAM_PICKUP_POINTS: u64 = 0x4741_4d45_5f50_4b50;
/// Salt mixed into the world seed to derive the simulation's match seed
/// (`WorldConfig::seed`, which seeds `ls_sim`'s pickup rolls).
pub const MATCH_SEED_SALT: u64 = 0x4d41_5443_485f_5345; // "MATCH_SE"

/// Spawn points per team.
pub const SPAWNS_PER_TEAM: usize = 6;
/// Number of pickup points.
pub const PICKUP_POINTS: usize = 12;
/// Preferred free space around a spawn point (u).
pub const SPAWN_CLEARANCE: f32 = 3.0;
/// Preferred free space around a pickup point (u).
pub const PICKUP_CLEARANCE: f32 = 2.0;
/// Least free space (terrain SDF) any spawn or pickup point gets (u); twice
/// the Tinker's collision radius.
pub const MIN_CLEARANCE: f32 = 1.0;
/// Clearances tried for spawn points, most free space first (u).
pub const SPAWN_CLEARANCE_LEVELS: [f32; 4] = [SPAWN_CLEARANCE, 2.0, 1.5, MIN_CLEARANCE];
/// Clearances tried for pickup points, most free space first (u).
pub const PICKUP_CLEARANCE_LEVELS: [f32; 3] = [PICKUP_CLEARANCE, 1.5, MIN_CLEARANCE];
/// Minimum distance between spawn points of the same team (u).
pub const SPAWN_SPACING: f32 = 6.0;
/// Minimum distance between pickup points (u).
pub const PICKUP_SPACING: f32 = 14.0;
/// Minimum distance from a pickup point to any spawn point (u).
pub const PICKUP_SPAWN_DISTANCE: f32 = 10.0;
/// Share of the level length (along x) given to each team's spawn zone.
pub const SPAWN_ZONE_FRACTION: f32 = 0.3;
/// Lattice spacing of the open-space scan (u).
pub const SCAN_STEP: f32 = 4.0;
/// Lattice spacing of the second scan, when a team finds no open space on
/// the coarse lattice (u).
pub const FINE_SCAN_STEP: f32 = 2.0;
/// Distance kept between placed points and the sealing walls (u).
const WALL_MARGIN: f32 = 4.0;
/// Candidate draws per point before giving up on step 1.
const ATTEMPTS_PER_POINT: usize = 64;
/// Search radius (lattice steps) of `find_open_point` around a candidate.
const OPEN_SEARCH_RADIUS: i32 = 6;

/// Everything that defines a match: world, rules and data tables.
#[derive(Debug, Clone)]
pub struct MatchConfig {
    pub code: ShareCode,
    pub rules: RuleSet,
    pub weapons: WeaponTable,
    pub tuning: ShipTuning,
}

impl MatchConfig {
    /// Built-in weapons, Tinker tuning and the default Paint the Void rules.
    pub fn paint_the_void(code: ShareCode) -> Self {
        MatchConfig {
            code,
            rules: RuleSet::paint_the_void(),
            weapons: WeaponTable::builtin(),
            tuning: ShipTuning::tinker(),
        }
    }

    /// The simulation's match seed (pickup rolls), derived from the world seed.
    pub fn match_seed(&self) -> u64 {
        match_seed(&self.code)
    }
}

/// `WorldConfig::seed` for a share code: `mix64(seed ^ MATCH_SEED_SALT)`.
pub fn match_seed(code: &ShareCode) -> u64 {
    mix64(code.seed ^ MATCH_SEED_SALT)
}

/// Deterministic spawn points (team 0 at low x, team 1 at high x, facing the
/// level centre) and pickup points for a pristine level.
///
/// `rules` decides the number of teams (Paint the Void: 2). Fails only if a
/// team's half of the level has no point with [`MIN_CLEARANCE`] of free space.
pub fn match_setup(
    code: &ShareCode,
    rules: &RuleSet,
    terrain: &GameTerrain,
) -> Result<MatchSetup, GameError> {
    let teams = rules.paint_the_void_rules().teams.clamp(1, 2);
    let gen = terrain.generator();
    let room = gen.room();
    // Keep points away from the sealing walls.
    let inner = Aabb::new(
        room.min + Vec3::splat(WALL_MARGIN),
        room.max - Vec3::splat(WALL_MARGIN),
    );
    let centre = inner.center();
    let open = scan_open_space(&gen, inner, SCAN_STEP);

    let mut rng = Pcg32::from_seed(code.seed, RNG_STREAM_SPAWNS);
    let mut spawns = Vec::new();
    for team in 0..teams {
        let zone = spawn_zone(inner, team, teams);
        let mut placed: Vec<Vec3> = Vec::new();
        let spaced =
            |p: Vec3, placed: &[Vec3]| placed.iter().all(|q| q.distance(p) >= SPAWN_SPACING);
        // Step 1, only if the scan saw room for all spawns in the zone.
        let roomy = open
            .iter()
            .filter(|(p, v)| *v >= SPAWN_CLEARANCE && zone.contains(*p))
            .count();
        if roomy >= SPAWNS_PER_TEAM {
            let mut attempts = 0;
            while placed.len() < SPAWNS_PER_TEAM && attempts < SPAWNS_PER_TEAM * ATTEMPTS_PER_POINT
            {
                attempts += 1;
                let Some(p) = open_candidate(&mut rng, zone, SPAWN_CLEARANCE, terrain) else {
                    continue;
                };
                if spaced(p, &placed) {
                    placed.push(p);
                }
            }
        }
        if placed.is_empty() {
            // Step 2 on a copy of the stream, so the next team's step 1
            // draws do not depend on whether this team needed it.
            let mut rng = rng.clone();
            let half = team_half(inner, team, teams);
            for area in [zone, half] {
                let cands = on_own_side(within(&open, area), centre.x, team, teams);
                pick(
                    &mut rng,
                    &cands,
                    &SPAWN_CLEARANCE_LEVELS,
                    SPAWNS_PER_TEAM,
                    &mut placed,
                    spaced,
                );
                if !placed.is_empty() {
                    break;
                }
            }
            if placed.is_empty() {
                let cands = on_own_side(
                    scan_open_space(&gen, half, FINE_SCAN_STEP),
                    centre.x,
                    team,
                    teams,
                );
                pick(
                    &mut rng,
                    &cands,
                    &SPAWN_CLEARANCE_LEVELS,
                    SPAWNS_PER_TEAM,
                    &mut placed,
                    spaced,
                );
            }
        }
        if placed.is_empty() {
            return Err(GameError::Setup(format!("no open space for team {team}")));
        }
        for pos in placed {
            spawns.push(SpawnPoint {
                pos,
                team,
                orientation: facing(centre - pos),
            });
        }
    }

    let mut rng = Pcg32::from_seed(code.seed, RNG_STREAM_PICKUP_POINTS);
    let mut pickups: Vec<Vec3> = Vec::new();
    let accept = |p: Vec3, placed: &[Vec3]| {
        spawns
            .iter()
            .all(|s| s.pos.distance(p) >= PICKUP_SPAWN_DISTANCE)
            && placed.iter().all(|q| q.distance(p) >= PICKUP_SPACING)
    };
    // Step 1, only if at least 1% of the scanned lattice has the preferred
    // clearance (with less, random draws mostly miss and step 1 is slow).
    let roomy = open.iter().filter(|(_, v)| *v >= PICKUP_CLEARANCE).count();
    if roomy * 100 >= lattice_points(inner, SCAN_STEP) {
        let mut attempts = 0;
        while pickups.len() < PICKUP_POINTS && attempts < PICKUP_POINTS * ATTEMPTS_PER_POINT {
            attempts += 1;
            let Some(p) = open_candidate(&mut rng, inner, PICKUP_CLEARANCE, terrain) else {
                continue;
            };
            if accept(p, &pickups) {
                pickups.push(p);
            }
        }
    }
    if pickups.is_empty() {
        pick(
            &mut rng,
            &open,
            &PICKUP_CLEARANCE_LEVELS,
            PICKUP_POINTS,
            &mut pickups,
            accept,
        );
    }

    Ok(MatchSetup {
        spawns,
        pickup_points: pickups,
    })
}

/// Builds the terrain for `cfg.code`, the setup and a [`World`] at tick 0,
/// ready for players and [`World::step`].
pub fn build_match(cfg: &MatchConfig) -> Result<(World, GameTerrain), GameError> {
    let terrain = GameTerrain::from_code(&cfg.code)?;
    let setup = match_setup(&cfg.code, &cfg.rules, &terrain)?;
    let world = World::new(WorldConfig {
        seed: cfg.match_seed(),
        weapons: cfg.weapons.clone(),
        rules: cfg.rules.clone(),
        tuning: cfg.tuning.clone(),
        setup,
    })?;
    Ok((world, terrain))
}

/// Team `team`'s spawn zone: a slab at one end of the level along x.
fn spawn_zone(inner: Aabb, team: u8, teams: u8) -> Aabb {
    let depth = (inner.max.x - inner.min.x) * SPAWN_ZONE_FRACTION;
    side(inner, team, teams, depth)
}

/// Team `team`'s half of the level along x.
fn team_half(inner: Aabb, team: u8, teams: u8) -> Aabb {
    let depth = (inner.max.x - inner.min.x) * 0.5;
    side(inner, team, teams, depth)
}

/// The slab of `inner` of `depth` along x at team `team`'s end (all of
/// `inner` with a single team).
fn side(inner: Aabb, team: u8, teams: u8, depth: f32) -> Aabb {
    if teams < 2 {
        return inner;
    }
    if team == 0 {
        Aabb::new(
            inner.min,
            Vec3::new(inner.min.x + depth, inner.max.y, inner.max.z),
        )
    } else {
        Aabb::new(
            Vec3::new(inner.max.x - depth, inner.min.y, inner.min.z),
            inner.max,
        )
    }
}

/// Draws a random point in `zone` and snaps it to nearby open space with the
/// given clearance, if the snapped point is still inside the zone.
fn open_candidate(
    rng: &mut Pcg32,
    zone: Aabb,
    clearance: f32,
    terrain: &GameTerrain,
) -> Option<Vec3> {
    let p = Vec3::new(
        rng.range_f32(zone.min.x, zone.max.x),
        rng.range_f32(zone.min.y, zone.max.y),
        rng.range_f32(zone.min.z, zone.max.z),
    );
    let q = terrain.find_open_point(p, clearance, OPEN_SEARCH_RADIUS)?;
    (zone.contains(q) && terrain.sdf(q) >= clearance.min(ls_worldgen::chunk::SDF_CLAMP))
        .then_some(q)
}

/// Lattice points (cell centres of spacing `step`) inside `area` with at
/// least [`MIN_CLEARANCE`] of free space in the pristine level, with their
/// clearance, in scan order (x fastest, then y, then z).
fn scan_open_space(gen: &Generator, area: Aabb, step: f32) -> Vec<(Vec3, f32)> {
    let size = area.size();
    let count = |s: f32| ((s / step) as usize).max(1);
    let (nx, ny, nz) = (count(size.x), count(size.y), count(size.z));
    let xs: Vec<f32> = (0..nx)
        .map(|i| area.min.x + (i as f32 + 0.5) * step)
        .collect();
    let sampler = gen.sparse_region_sampler(area);
    let mut row = vec![0.0f32; nx];
    let mut out = Vec::new();
    for k in 0..nz {
        let z = area.min.z + (k as f32 + 0.5) * step;
        for j in 0..ny {
            let y = area.min.y + (j as f32 + 0.5) * step;
            sampler.sample_row(y, z, &xs, &mut row);
            for (x, v) in xs.iter().zip(&row) {
                if *v >= MIN_CLEARANCE {
                    out.push((Vec3::new(*x, y, z), *v));
                }
            }
        }
    }
    out
}

/// Number of lattice points [`scan_open_space`] samples in `area`.
fn lattice_points(area: Aabb, step: f32) -> usize {
    let size = area.size();
    let count = |s: f32| ((s / step) as usize).max(1);
    count(size.x) * count(size.y) * count(size.z)
}

/// The candidates inside `area`, in order.
fn within(cands: &[(Vec3, f32)], area: Aabb) -> Vec<(Vec3, f32)> {
    cands
        .iter()
        .copied()
        .filter(|(p, _)| area.contains(*p))
        .collect()
}

/// Drops the candidates not strictly on team `team`'s side of the midplane
/// `x = mid_x` (with two teams): the team halves share the midplane, and a
/// spawn there would face along the midplane and sit on the other team's
/// boundary. With one team every candidate stays.
fn on_own_side(mut cands: Vec<(Vec3, f32)>, mid_x: f32, team: u8, teams: u8) -> Vec<(Vec3, f32)> {
    if teams >= 2 {
        cands.retain(|(p, _)| if team == 0 { p.x < mid_x } else { p.x > mid_x });
    }
    cands
}

/// Adds up to `want` points (in total) to `chosen`: for each clearance level
/// in turn, draws candidates with at least that clearance in random order and
/// keeps those `accept` allows next to the points chosen so far.
fn pick(
    rng: &mut Pcg32,
    cands: &[(Vec3, f32)],
    levels: &[f32],
    want: usize,
    chosen: &mut Vec<Vec3>,
    accept: impl Fn(Vec3, &[Vec3]) -> bool,
) {
    for &level in levels {
        if chosen.len() >= want {
            return;
        }
        let mut pool: Vec<Vec3> = cands
            .iter()
            .filter(|(p, v)| *v >= level && !chosen.contains(p))
            .map(|(p, _)| *p)
            .collect();
        while chosen.len() < want && !pool.is_empty() {
            let p = pool.swap_remove(rng.below(pool.len() as u32) as usize);
            if accept(p, chosen) {
                chosen.push(p);
            }
        }
    }
}

/// Orientation looking along the horizontal part of `dir` (level, no roll).
fn facing(dir: Vec3) -> Quat {
    let flat = Vec3::new(dir.x, 0.0, dir.z);
    match flat.try_normalize() {
        Some(d) => Quat::from_rotation_arc(Vec3::NEG_Z, d),
        None => Quat::IDENTITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_worldgen::Sliders;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn code(seed: u64) -> ShareCode {
        ShareCode::new(seed, Sliders::new(40, 30))
    }

    /// Checks everything a playable setup needs.
    fn check_setup(c: &ShareCode, t: &GameTerrain, s: &MatchSetup) {
        let tuning = ShipTuning::tinker();
        let mid = t.bounds().center().x;
        let room = t.generator().room();
        for team in 0..2u8 {
            let n = s.spawns.iter().filter(|p| p.team == team).count();
            assert!(
                (1..=SPAWNS_PER_TEAM).contains(&n),
                "{c}: team {team} has {n} spawns"
            );
        }
        for (i, sp) in s.spawns.iter().enumerate() {
            assert!(
                t.sdf(sp.pos) >= MIN_CLEARANCE,
                "{c}: spawn {sp:?} too tight"
            );
            assert!(!t.sphere_overlap(sp.pos, tuning.radius), "{c}: {sp:?}");
            assert!(room.contains(sp.pos));
            // Team 0 low x, team 1 high x, both facing inwards.
            assert_eq!(sp.pos.x < mid, sp.team == 0, "{c}: {sp:?}");
            let fwd = sp.orientation.forward();
            assert!((fwd.x > 0.0) == (sp.team == 0), "{c}: {fwd:?}");
            for other in &s.spawns[i + 1..] {
                if other.team == sp.team {
                    assert!(other.pos.distance(sp.pos) >= SPAWN_SPACING, "{c}");
                }
            }
        }
        assert!(!s.pickup_points.is_empty(), "{c}: no pickup points");
        assert!(s.pickup_points.len() <= PICKUP_POINTS);
        for (i, p) in s.pickup_points.iter().enumerate() {
            assert!(t.sdf(*p) >= MIN_CLEARANCE, "{c}: pickup {p:?} too tight");
            assert!(s
                .spawns
                .iter()
                .all(|sp| sp.pos.distance(*p) >= PICKUP_SPAWN_DISTANCE));
            assert!(s.pickup_points[i + 1..]
                .iter()
                .all(|q| q.distance(*p) >= PICKUP_SPACING));
        }
    }

    #[test]
    fn setup_is_deterministic_and_in_open_space() {
        let c = code(2024);
        let rules = RuleSet::paint_the_void();
        let t = GameTerrain::from_code(&c).unwrap();
        let a = match_setup(&c, &rules, &t).unwrap();
        let b = match_setup(&c, &rules, &GameTerrain::from_code(&c).unwrap()).unwrap();
        assert_eq!(a, b);
        check_setup(&c, &t, &a);
        // Roomy level: every point gets the preferred clearance.
        assert_eq!(a.spawns.len(), 2 * SPAWNS_PER_TEAM);
        assert!(a.spawns.iter().all(|s| t.sdf(s.pos) >= SPAWN_CLEARANCE));
        assert!(a
            .pickup_points
            .iter()
            .all(|p| t.sdf(*p) >= PICKUP_CLEARANCE));
        let other = match_setup(
            &code(99),
            &rules,
            &GameTerrain::from_code(&code(99)).unwrap(),
        );
        assert_ne!(other.unwrap(), a);
    }

    /// Regression: this code used to fail with "no open space for team 0"
    /// (no point of the level has 3 u of clearance).
    #[test]
    fn cramped_share_code_hosts_a_match() {
        let c = ShareCode::decode("LS04-0000-00VT-PVXV-WPA6-8001-8F6R").unwrap();
        assert_eq!((c.sliders.enclosure, c.sliders.density), (75, 20));
        let cfg = MatchConfig::paint_the_void(c);
        let (world, terrain) = build_match(&cfg).unwrap();
        let setup = match_setup(&c, &cfg.rules, &terrain).unwrap();
        check_setup(&c, &terrain, &setup);
        assert_eq!(world.tick(), 0);
    }

    /// Every Enclosure x Density combination (0/25/50/75/100) over several
    /// seeds gives a valid, steppable match.
    #[test]
    fn every_slider_combination_hosts_a_match() {
        // Debug builds are slow; release runs (CI's wasm32 job, or
        // `cargo test -p ls-game --release --lib setup`) sweep 8 seeds.
        let seeds: &[u64] = if cfg!(debug_assertions) {
            &[0xDEAD_BEEF]
        } else {
            &[0xDEAD_BEEF, 1, 2, 3, 42, 2024, 7777, u64::MAX]
        };
        let steps = [0u8, 25, 50, 75, 100];
        for &seed in seeds {
            for e in steps {
                for d in steps {
                    let c = ShareCode::new(seed ^ ((e as u64) << 8 | d as u64), Sliders::new(e, d));
                    let cfg = MatchConfig::paint_the_void(c);
                    let terrain = GameTerrain::from_code(&c).unwrap();
                    let setup = match_setup(&c, &cfg.rules, &terrain)
                        .unwrap_or_else(|err| panic!("{c}: {err}"));
                    check_setup(&c, &terrain, &setup);
                    // What build_match does with it.
                    let world = World::new(WorldConfig {
                        seed: cfg.match_seed(),
                        weapons: cfg.weapons.clone(),
                        rules: cfg.rules.clone(),
                        tuning: cfg.tuning.clone(),
                        setup,
                    })
                    .unwrap_or_else(|err| panic!("{c}: {err}"));
                    assert_eq!(world.tick(), 0);
                }
            }
        }
    }

    #[test]
    fn fallback_candidates_never_sit_on_the_midplane() {
        let mid = 128.0;
        let cands = vec![
            (Vec3::new(100.0, 10.0, 10.0), 2.0),
            (Vec3::new(mid, 10.0, 10.0), 3.0),
            (Vec3::new(140.0, 10.0, 10.0), 2.0),
        ];
        let xs = |team| -> Vec<f32> {
            on_own_side(cands.clone(), mid, team, 2)
                .iter()
                .map(|(p, _)| p.x)
                .collect()
        };
        assert_eq!(xs(0), vec![100.0]);
        assert_eq!(xs(1), vec![140.0]);
        // A single team keeps everything.
        assert_eq!(on_own_side(cands.clone(), mid, 0, 1).len(), 3);
    }

    #[test]
    fn build_match_gives_a_steppable_world() {
        let cfg = MatchConfig::paint_the_void(code(5));
        let (mut world, mut terrain) = build_match(&cfg).unwrap();
        let p0 = world.add_player(None, ls_sim::ShipType::Tinker).unwrap();
        let p1 = world.add_player(None, ls_sim::ShipType::Tinker).unwrap();
        assert_ne!(world.ship(p0).unwrap().team, world.ship(p1).unwrap().team);
        let cmd = ls_sim::InputCmd::idle(1).with_thrust(Vec3::new(0.0, 0.0, -1.0));
        // Stay below tick 30, the first paint coverage query (it generates
        // every chunk, which is slow in debug builds).
        for _ in 0..29 {
            world.step(&mut terrain, &[(p0, cmd)]);
        }
        assert_eq!(world.tick(), 29);
        assert!(world.ship(p0).unwrap().speed() > 1.0);
    }
}
