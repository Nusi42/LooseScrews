//! Match setup from a share code and a rule set: deterministic team spawn
//! points and pickup points in open space, and a ready-to-step [`World`].
//!
//! Placement draws from its own PCG32 streams ([`RNG_STREAM_SPAWNS`],
//! [`RNG_STREAM_PICKUP_POINTS`]) seeded with the world seed, and only reads the
//! pristine generated terrain, so every peer computes the same setup from the
//! same share code (see `docs/architecture/contracts.md`).

use ls_math::rng::mix64;
use ls_math::terrain::TerrainQuery;
use ls_math::{Aabb, Pcg32, Quat, Vec3};
use ls_sim::{MatchSetup, RuleSet, ShipTuning, SpawnPoint, WeaponTable, World, WorldConfig};
use ls_worldgen::ShareCode;

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
/// Free space required around a spawn point (u).
pub const SPAWN_CLEARANCE: f32 = 3.0;
/// Free space required around a pickup point (u).
pub const PICKUP_CLEARANCE: f32 = 2.0;
/// Minimum distance between spawn points of the same team (u).
pub const SPAWN_SPACING: f32 = 6.0;
/// Minimum distance between pickup points (u).
pub const PICKUP_SPACING: f32 = 14.0;
/// Minimum distance from a pickup point to any spawn point (u).
pub const PICKUP_SPAWN_DISTANCE: f32 = 10.0;
/// Share of the level length (along x) given to each team's spawn zone.
pub const SPAWN_ZONE_FRACTION: f32 = 0.3;

/// Candidate draws per point before giving up on a zone.
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
/// `rules` decides the number of teams (Paint the Void: 2).
pub fn match_setup(
    code: &ShareCode,
    rules: &RuleSet,
    terrain: &GameTerrain,
) -> Result<MatchSetup, GameError> {
    let teams = rules.paint_the_void_rules().teams.clamp(1, 2);
    let room = terrain.with_store(|s| s.generator().room());
    // Keep candidates away from the sealing walls.
    let inner = Aabb::new(room.min + Vec3::splat(4.0), room.max - Vec3::splat(4.0));
    let centre = inner.center();

    let mut rng = Pcg32::from_seed(code.seed, RNG_STREAM_SPAWNS);
    let mut spawns = Vec::new();
    for team in 0..teams {
        let zone = spawn_zone(inner, team, teams);
        let mut placed: Vec<Vec3> = Vec::new();
        let mut attempts = 0;
        while placed.len() < SPAWNS_PER_TEAM && attempts < SPAWNS_PER_TEAM * ATTEMPTS_PER_POINT {
            attempts += 1;
            let Some(p) = open_candidate(&mut rng, zone, SPAWN_CLEARANCE, terrain) else {
                continue;
            };
            if placed.iter().all(|q| q.distance(p) >= SPAWN_SPACING) {
                placed.push(p);
            }
        }
        if placed.is_empty() {
            // Fall back to the open point nearest the zone centre.
            let p = terrain
                .find_open_point(zone.center(), SPAWN_CLEARANCE, 64)
                .ok_or_else(|| GameError::Setup(format!("no open space for team {team}")))?;
            placed.push(p);
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
    let mut attempts = 0;
    while pickups.len() < PICKUP_POINTS && attempts < PICKUP_POINTS * ATTEMPTS_PER_POINT {
        attempts += 1;
        let Some(p) = open_candidate(&mut rng, inner, PICKUP_CLEARANCE, terrain) else {
            continue;
        };
        let far_from_spawns = spawns
            .iter()
            .all(|s| s.pos.distance(p) >= PICKUP_SPAWN_DISTANCE);
        if far_from_spawns && pickups.iter().all(|q| q.distance(p) >= PICKUP_SPACING) {
            pickups.push(p);
        }
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
    if teams < 2 {
        return inner;
    }
    let depth = (inner.max.x - inner.min.x) * SPAWN_ZONE_FRACTION;
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

    #[test]
    fn setup_is_deterministic_and_in_open_space() {
        let c = code(2024);
        let rules = RuleSet::paint_the_void();
        let t = GameTerrain::from_code(&c).unwrap();
        let a = match_setup(&c, &rules, &t).unwrap();
        let b = match_setup(&c, &rules, &GameTerrain::from_code(&c).unwrap()).unwrap();
        assert_eq!(a, b);
        let tuning = ShipTuning::tinker();
        for team in 0..2u8 {
            let n = a.spawns.iter().filter(|s| s.team == team).count();
            assert!(n >= 1, "team {team} has no spawns");
        }
        let mid = t.bounds().center().x;
        for s in &a.spawns {
            assert!(t.sdf(s.pos) >= SPAWN_CLEARANCE - 1e-3);
            assert!(!t.sphere_overlap(s.pos, tuning.radius));
            // Team 0 low x, team 1 high x, both facing inwards.
            assert_eq!(s.pos.x < mid, s.team == 0);
            let fwd = s.orientation.forward();
            assert!((fwd.x > 0.0) == (s.team == 0), "{fwd:?}");
        }
        assert!(!a.pickup_points.is_empty());
        for p in &a.pickup_points {
            assert!(t.sdf(*p) >= PICKUP_CLEARANCE - 1e-3);
        }
        let other = match_setup(
            &code(99),
            &rules,
            &GameTerrain::from_code(&code(99)).unwrap(),
        );
        assert_ne!(other.unwrap(), a);
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
