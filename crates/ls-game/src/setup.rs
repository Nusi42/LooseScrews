//! Match setup from a share code and a rule set: deterministic team spawn
//! points and pickup points in open space, and a ready-to-step [`World`].
//!
//! Placement draws from its own PCG32 streams ([`RNG_STREAM_SPAWNS`],
//! [`RNG_STREAM_PICKUP_POINTS`]) seeded with the world seed, and only reads the
//! pristine generated terrain, so every peer computes the same setup from the
//! same share code (see `docs/architecture/contracts.md`).
//!
//! 1. A lattice scan of the level ([`SCAN_STEP`]) lists open points and
//!    their clearance (pristine generator SDF).
//! 2. Spawn points are drawn from the team's spawn zone, then from its half
//!    of the level, then from a finer lattice ([`FINE_SCAN_STEP`]); pickup
//!    points from the whole scan. Draws take the most free space first
//!    ([`SPAWN_CLEARANCE_LEVELS`], [`PICKUP_CLEARANCE_LEVELS`]): dense levels
//!    and high Enclosure often have little space with the preferred
//!    clearance, because their SDF is a conservative distance bound.
//! 3. Every spawn and pickup point must reach the level's connection network
//!    ([`ls_worldgen::Network`], caverns and tunnels that link both team
//!    ends) in a straight line keeping [`CONNECT_MARGIN`] of free space, so
//!    all of them share one connected open volume: no spawn or pickup sits
//!    in an isolated pocket. If a team finds no such point at all, it
//!    spawns in the network caverns of its end of the level. The tunnels and
//!    caverns themselves keep at least 3 u of clearance; the short link from
//!    a point to them only [`CONNECT_MARGIN`] (1.5 u, less up to about 0.13 u
//!    between trace samples), so the last stretch out of a spawn can be as
//!    narrow as about 2.7 u across: enough for a Hauler (radius 0.65).
//!
//! 4. The Heist site (core vein and extraction zones) comes from the
//!    generator ([`heist_level`]); the sweep checks that the vein and both
//!    zones reach the network like spawns do.
//!
//! Every slider combination gets a valid setup (see the sweep test).

use ls_math::rng::mix64;
use ls_math::{Aabb, Pcg32, Quat, Vec3};
use ls_sim::{
    GadgetTable, GameData, HeistLevel, MatchSetup, RuleSet, ShipTable, SpawnPoint, WeaponTable,
    World, WorldConfig,
};
use ls_worldgen::{Generator, RegionSampler, ShareCode};

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
/// Free space kept along the straight line from every spawn and pickup
/// point to the connection network (u); over twice the largest ship radius.
pub const CONNECT_MARGIN: f32 = 1.5;
/// Nearest network centre-line points tried per spawn or pickup point.
const LINK_TRIES: usize = 4;
/// Distance kept between placed points and the sealing walls (u).
const WALL_MARGIN: f32 = 4.0;

/// Everything that defines a match: world, rules and data tables.
#[derive(Debug, Clone)]
pub struct MatchConfig {
    pub code: ShareCode,
    pub rules: RuleSet,
    pub weapons: WeaponTable,
    /// Tuning of every ship type.
    pub ships: ShipTable,
    pub gadgets: GadgetTable,
}

impl MatchConfig {
    /// Built-in data (`ls_sim::GameData::builtin`) and the default Paint the
    /// Void rules.
    pub fn paint_the_void(code: ShareCode) -> Self {
        MatchConfig {
            code,
            rules: RuleSet::paint_the_void(),
            weapons: WeaponTable::builtin(),
            ships: ShipTable::builtin(),
            gadgets: GadgetTable::builtin(),
        }
    }

    /// The simulation config for this match with a computed setup.
    pub fn world_config(&self, setup: MatchSetup) -> WorldConfig {
        WorldConfig {
            seed: self.match_seed(),
            weapons: self.weapons.clone(),
            ships: self.ships.clone(),
            gadgets: self.gadgets.clone(),
            rules: self.rules.clone(),
            setup,
        }
    }

    /// Hash of this match's data tables, as sent in `Hello`/`Welcome`. For
    /// the built-in tables this is `ls_sim::data_hash()`; custom tables
    /// (tests, tools) hash their serialized form.
    pub fn data_hash(&self) -> u64 {
        let builtin = GameData::builtin();
        if self.weapons == builtin.weapons
            && self.ships == builtin.ships
            && self.gadgets == builtin.gadgets
        {
            return ls_sim::data_hash();
        }
        let weapons: Vec<String> = self
            .weapons
            .iter()
            .map(|(_, d)| ron::to_string(d).unwrap_or_default())
            .collect();
        let ships: Vec<String> = self
            .ships
            .iter()
            .map(|(_, t)| ron::to_string(t).unwrap_or_default())
            .collect();
        let gadgets: Vec<String> = self
            .gadgets
            .iter()
            .map(|(_, d)| ron::to_string(d).unwrap_or_default())
            .collect();
        let files: Vec<(String, &str)> = weapons
            .iter()
            .map(|t| ("weapons".to_string(), t.as_str()))
            .chain(ships.iter().map(|t| ("ships".to_string(), t.as_str())))
            .chain(gadgets.iter().map(|t| ("gadgets".to_string(), t.as_str())))
            .enumerate()
            .map(|(i, (k, t))| (format!("{k}/{i:03}"), t))
            .collect();
        ls_sim::with_behavior_version(ls_sim::data::hash_files(
            files.iter().map(|(p, t)| (p.as_str(), *t)),
        ))
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
    let sampler = gen.sparse_region_sampler(room);
    let linked = |p: Vec3| linked(&gen, &sampler, p);

    let mut rng = Pcg32::from_seed(code.seed, RNG_STREAM_SPAWNS);
    let mut spawns = Vec::new();
    for team in 0..teams {
        let zone = spawn_zone(inner, team, teams);
        let mut placed: Vec<Vec3> = Vec::new();
        let spaced = |p: Vec3, placed: &[Vec3]| {
            placed.iter().all(|q| q.distance(p) >= SPAWN_SPACING) && linked(p)
        };
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
            if placed.len() >= SPAWNS_PER_TEAM {
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
        if placed.is_empty() {
            // The network caverns at this team's end always qualify.
            let net = gen.network();
            let hubs = match (teams, team) {
                (1, _) => net.hubs(),
                (_, 0) => net.low_x_hubs(),
                _ => net.high_x_hubs(),
            };
            let cands: Vec<(Vec3, f32)> = hubs.iter().map(|h| (*h, gen.sample(*h))).collect();
            pick(
                &mut rng,
                &on_own_side(cands, centre.x, team, teams),
                &[MIN_CLEARANCE],
                SPAWNS_PER_TEAM,
                &mut placed,
                spaced,
            );
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
            && linked(p)
    };
    pick(
        &mut rng,
        &open,
        &PICKUP_CLEARANCE_LEVELS,
        PICKUP_POINTS,
        &mut pickups,
        accept,
    );

    Ok(MatchSetup {
        spawns,
        pickup_points: pickups,
        instability: code.sliders.instability,
        heist: heist_level(&gen),
        // M4 world physics (A4); A1 adds the generator's wells and belt.
        gravity_chaos: code.sliders.gravity_chaos,
        monochrome: code.sliders.palette >= ls_sim::pigment::MONOCHROME_PALETTE,
        ..Default::default()
    })
}

/// The generator's Heist site (core vein, extraction zones) for the
/// simulation (`MatchSetup::heist`; every rule set gets it, only Heist
/// reads it).
pub fn heist_level(gen: &Generator) -> Option<HeistLevel> {
    gen.heist().map(|h| HeistLevel {
        core: h.core,
        knot_radius: h.knot_radius,
        chamber_radius: h.chamber_radius,
        extraction: h.extraction.map(|z| (z.pos, z.radius)),
    })
}

/// Builds the terrain for `cfg.code`, the setup and a [`World`] at tick 0,
/// ready for players and [`World::step`].
pub fn build_match(cfg: &MatchConfig) -> Result<(World, GameTerrain), GameError> {
    let terrain = GameTerrain::from_code(&cfg.code)?;
    let setup = match_setup(&cfg.code, &cfg.rules, &terrain)?;
    let world = World::new(cfg.world_config(setup))?;
    Ok((world, terrain))
}

/// Whether `p` reaches the connection network in a straight line keeping
/// [`CONNECT_MARGIN`] (levels too small for a network accept every point).
fn linked(gen: &Generator, sampler: &RegionSampler, p: Vec3) -> bool {
    let net = gen.network();
    net.hubs().is_empty() || net.reaches(|x| sampler.sample(x), p, CONNECT_MARGIN, LINK_TRIES)
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
    use ls_math::terrain::TerrainQuery;
    use ls_worldgen::network::segment_clear;
    use ls_worldgen::{ClearanceGrid, Sliders};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn code(seed: u64) -> ShareCode {
        ShareCode::new(seed, Sliders::new(40, 30))
    }

    /// Checks everything a playable setup needs.
    fn check_setup(c: &ShareCode, t: &GameTerrain, s: &MatchSetup) {
        // Every ship type fits (the Hauler has the largest hitbox).
        let radius = ShipTable::builtin().max_radius();
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
            assert!(!t.sphere_overlap(sp.pos, radius), "{c}: {sp:?}");
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
        // Every spawn and pickup reaches the connection network.
        let gen = t.generator();
        for p in s
            .spawns
            .iter()
            .map(|sp| sp.pos)
            .chain(s.pickup_points.iter().copied())
        {
            assert!(
                gen.network().hubs().is_empty()
                    || gen.reaches_network(p, CONNECT_MARGIN, LINK_TRIES),
                "{c}: {p:?} does not reach the network"
            );
        }
        // The Heist core vein (its access point in the chamber) and both
        // extraction zones reach the network like spawns do, and the knot
        // holding the core is rock.
        let site = gen.heist().expect("every level has a Heist site");
        assert_eq!(s.heist, heist_level(&gen), "{c}");
        for p in std::iter::once(site.access).chain(site.extraction.map(|z| z.pos)) {
            assert!(
                gen.reaches_network(p, CONNECT_MARGIN, LINK_TRIES),
                "{c}: Heist point {p:?} does not reach the network"
            );
        }
        assert!(t.sdf(site.core) < 0.0, "{c}: the core is not in rock");
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

    /// Regression: under generator version 1 this code failed with "no open
    /// space for team 0" (no point of the level had 3 u of clearance). The
    /// version 2 network now carves spawn caverns into every level, so this
    /// checks that the former cramped world still hosts a match; the
    /// clearance fallbacks for cramped levels are covered by the slider
    /// sweep (`every_slider_combination_hosts_a_match`).
    #[test]
    fn cramped_share_code_hosts_a_match() {
        // The code was made by generator version 1; same seed and sliders.
        let old = ShareCode::decode("LS04-0000-00VT-PVXV-WPA6-8001-8F6R").unwrap();
        let c = ShareCode::new(old.seed, old.sliders);
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
                    let world = World::new(cfg.world_config(setup))
                        .unwrap_or_else(|err| panic!("{c}: {err}"));
                    assert_eq!(world.tick(), 0);
                }
            }
        }
    }

    /// Independent check with a flood fill over the level's clearance
    /// lattice: every spawn and pickup has a clear straight line (keeping
    /// [`CONNECT_MARGIN`]) to a network centre-line point that lies in the
    /// main open volume, the flood-fill component that also holds both
    /// teams' spawn caverns.
    fn check_flood_fill(c: &ShareCode, t: &GameTerrain, s: &MatchSetup) {
        let gen = t.generator();
        let grid = ClearanceGrid::level(&gen);
        let main = grid.main_component(&gen).expect("main component");
        let net = gen.network();
        for hub in net.low_x_hubs().iter().chain(net.high_x_hubs()) {
            assert_eq!(grid.nearest_component(*hub, 2.0), Some(main), "{c}");
        }
        let site = gen.heist().expect("Heist site");
        for p in std::iter::once(site.access).chain(site.extraction.map(|z| z.pos)) {
            assert_eq!(
                grid.nearest_component(p, 2.0),
                Some(main),
                "{c}: Heist {p:?}"
            );
        }
        for p in s
            .spawns
            .iter()
            .map(|sp| sp.pos)
            .chain(s.pickup_points.iter().copied())
        {
            let ok = net.link_targets(p).into_iter().take(LINK_TRIES).any(|q| {
                segment_clear(|x| gen.sample(x), p, q, CONNECT_MARGIN)
                    && grid.nearest_component(q, 2.0) == Some(main)
            });
            assert!(ok, "{c}: {p:?} is cut off from the main volume");
        }
    }

    #[test]
    fn spawns_and_pickups_share_one_open_volume() {
        // Debug builds are slow: one cavern world; release sweeps more.
        let codes: Vec<ShareCode> = if cfg!(debug_assertions) {
            vec![code(2024)]
        } else {
            let mut v = Vec::new();
            for seed in [1u64, 2024] {
                for e in [0u8, 25, 50, 75, 100] {
                    for d in [0u8, 50, 100] {
                        v.push(ShareCode::new(
                            seed ^ ((e as u64) << 8 | d as u64),
                            Sliders::new(e, d),
                        ));
                    }
                }
            }
            v
        };
        let rules = RuleSet::paint_the_void();
        for c in codes {
            let t = GameTerrain::from_code(&c).unwrap();
            let s = match_setup(&c, &rules, &t).unwrap();
            check_setup(&c, &t, &s);
            check_flood_fill(&c, &t, &s);
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
