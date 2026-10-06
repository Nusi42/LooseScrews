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
//! 5. Teams 0 and 1 spawn at the low and high x ends; with three or four
//!    teams, team 2 spawns at the low z end and team 3 at the high z end
//!    (slabs of [`SPAWN_ZONE_FRACTION`] of the z length between the x
//!    zones), each team falling back to its own network caverns
//!    (`Network::team_hubs`; generator version 4 carves a cavern at each z
//!    end).
//! 6. Gravity wells (`Generator::gravity_wells`) go into
//!    `MatchSetup::wells`; no spawn point lies within
//!    [`WELL_SPAWN_DISTANCE`] of a well.
//! 7. Asteroid belt levels (`Generator::belt`) take their spawn points from
//!    the belt's 8 start slots and their pickup points from the belt
//!    ([`belt_setup`]); `MatchSetup::belt` carries the site.
//!
//! Every slider combination gets a valid setup (see the sweep test).

use ls_math::rng::mix64;
use ls_math::{Aabb, Pcg32, Quat, Vec3};
use ls_sim::{
    GadgetTable, GameData, HeistLevel, MatchSetup, RuleSet, ShipTable, SpawnPoint, WeaponTable,
    World, WorldConfig,
};
use ls_worldgen::wells::WELL_SPAWN_DISTANCE;
use ls_worldgen::{BeltSite, Generator, RegionSampler, ShareCode};

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

/// Deterministic spawn points (team 0 at low x, team 1 at high x, teams 2
/// and 3 at low and high z, facing the level centre) and pickup points for
/// a pristine level; on belt levels the belt's start slots and pickup
/// points ([`belt_setup`]).
///
/// `rules` decides the number of teams (Paint the Void: 2). Fails only if a
/// team's part of the level has no point with [`MIN_CLEARANCE`] of free
/// space.
pub fn match_setup(
    code: &ShareCode,
    rules: &RuleSet,
    terrain: &GameTerrain,
) -> Result<MatchSetup, GameError> {
    let teams = rules.paint_the_void_rules().teams.clamp(1, 4);
    let gen = terrain.generator();
    if let Some(belt) = gen.belt() {
        return Ok(belt_setup(code, teams, &gen, belt));
    }
    let wells = gen.gravity_wells().to_vec();
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
            placed.iter().all(|q| q.distance(p) >= SPAWN_SPACING)
                && wells
                    .iter()
                    .all(|w| w.pos.distance(p) >= WELL_SPAWN_DISTANCE)
                && linked(p)
        };
        let half = team_half(inner, team, teams);
        for area in [zone, half] {
            let cands = on_own_side(within(&open, area), centre, team, teams);
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
                centre,
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
            let hubs = match teams {
                1 => net.hubs(),
                _ => net.team_hubs(team),
            };
            let cands: Vec<(Vec3, f32)> = hubs.iter().map(|h| (*h, gen.sample(*h))).collect();
            pick(
                &mut rng,
                &on_own_side(cands, centre, team, teams),
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
        wells,
        belt: None,
        spawns,
        pickup_points: pickups,
        instability: code.sliders.instability,
        absurdity: code.sliders.absurdity,
        heist: heist_level(&gen),
        // M4 world physics (A4); A1 fills the generator's wells and belt.
        gravity_chaos: code.sliders.gravity_chaos,
        monochrome: code.sliders.palette >= ls_sim::pigment::MONOCHROME_PALETTE,
    })
}

/// The setup of an asteroid belt level: one spawn point per start slot
/// (slot `k` is spawn point `k`, facing prograde; with teams, slots
/// alternate between them), the belt's pickup points, its shepherd moons as
/// wells and the site itself in `MatchSetup::belt`. No Heist site.
pub fn belt_setup(code: &ShareCode, teams: u8, gen: &Generator, belt: &BeltSite) -> MatchSetup {
    let teams = teams.max(1);
    MatchSetup {
        spawns: belt
            .slots
            .iter()
            .enumerate()
            .map(|(k, slot)| SpawnPoint {
                pos: slot.pos,
                team: (k % teams as usize) as u8,
                orientation: slot.orientation,
            })
            .collect(),
        pickup_points: belt.pickup_points.clone(),
        instability: code.sliders.instability,
        heist: None,
        wells: gen.gravity_wells().to_vec(),
        belt: Some(belt.clone()),
        absurdity: code.sliders.absurdity,
        gravity_chaos: code.sliders.gravity_chaos,
        monochrome: code.sliders.palette >= ls_sim::pigment::MONOCHROME_PALETTE,
    }
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
    let mut world = World::new(cfg.world_config(setup))?;
    world.set_nav_graph(nav_graph(&terrain.generator()));
    Ok((world, terrain))
}

/// The comedy event drones' flight network for a level
/// (`World::set_nav_graph`, `ls_sim::events::drone`): the connection
/// network's hub caverns and tunnel centre lines, entered from the team
/// spawn caverns at the level's ends. Empty on levels without a network
/// (asteroid belts): the drones fly straight with local avoidance.
pub fn nav_graph(gen: &Generator) -> ls_sim::events::drone::NavGraph {
    let net = gen.network();
    let entries: Vec<Vec3> = (0..4u8)
        .flat_map(|team| net.team_hubs(team).iter().copied())
        .collect();
    ls_sim::events::drone::NavGraph::from_network(net.hubs(), net.tunnel_paths(), &entries)
}

/// Whether `p` reaches the connection network in a straight line keeping
/// [`CONNECT_MARGIN`] (levels too small for a network accept every point).
fn linked(gen: &Generator, sampler: &RegionSampler, p: Vec3) -> bool {
    let net = gen.network();
    net.hubs().is_empty() || net.reaches(|x| sampler.sample(x), p, CONNECT_MARGIN, LINK_TRIES)
}

/// Team `team`'s spawn zone: a slab of [`SPAWN_ZONE_FRACTION`] at one end
/// of the level, along x for teams 0 and 1, along z (between the x zones)
/// for teams 2 and 3.
pub fn spawn_zone(inner: Aabb, team: u8, teams: u8) -> Aabb {
    side(inner, team, teams, SPAWN_ZONE_FRACTION)
}

/// Team `team`'s half of the level (teams 2 and 3: their half along z,
/// between the x spawn zones).
fn team_half(inner: Aabb, team: u8, teams: u8) -> Aabb {
    side(inner, team, teams, 0.5)
}

/// The slab of `inner` of `fraction` of its length at team `team`'s end
/// (all of `inner` with a single team): low x, high x, then (between the x
/// spawn zones) low z, high z.
fn side(inner: Aabb, team: u8, teams: u8, fraction: f32) -> Aabb {
    if teams < 2 {
        return inner;
    }
    let size = inner.size();
    let (lo, hi) = (inner.min, inner.max);
    match team {
        0 => Aabb::new(lo, Vec3::new(lo.x + size.x * fraction, hi.y, hi.z)),
        1 => Aabb::new(Vec3::new(hi.x - size.x * fraction, lo.y, lo.z), hi),
        _ => {
            let x0 = lo.x + size.x * SPAWN_ZONE_FRACTION;
            let x1 = hi.x - size.x * SPAWN_ZONE_FRACTION;
            let depth = size.z * fraction;
            if team == 2 {
                Aabb::new(Vec3::new(x0, lo.y, lo.z), Vec3::new(x1, hi.y, lo.z + depth))
            } else {
                Aabb::new(Vec3::new(x0, lo.y, hi.z - depth), Vec3::new(x1, hi.y, hi.z))
            }
        }
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

/// Drops the candidates not strictly on team `team`'s side of the level
/// centre `mid` (with two or more teams): x below or above the centre for
/// teams 0 and 1, z for teams 2 and 3. The team halves share the
/// midplane, and a spawn there would face along it and sit on another
/// team's boundary. With one team every candidate stays.
fn on_own_side(mut cands: Vec<(Vec3, f32)>, mid: Vec3, team: u8, teams: u8) -> Vec<(Vec3, f32)> {
    if teams >= 2 {
        cands.retain(|(p, _)| match team {
            0 => p.x < mid.x,
            1 => p.x > mid.x,
            2 => p.z < mid.z,
            _ => p.z > mid.z,
        });
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
    use ls_sim::ModeRules;
    use ls_worldgen::network::segment_clear;
    use ls_worldgen::{Archetype, ClearanceGrid, Sliders, BELT_WEDGES};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn code(seed: u64) -> ShareCode {
        ShareCode::new(seed, Sliders::new(40, 30))
    }

    /// Paint the Void rules with `teams` teams.
    fn rules_with_teams(teams: u8) -> RuleSet {
        let mut rules = RuleSet::paint_the_void();
        if let ModeRules::PaintTheVoid(m) = &mut rules.mode {
            m.teams = teams;
        }
        rules
    }

    /// The slider mix number `i` of the sweep: Enclosure and Density from
    /// the 5 x 5 grid, the other five sliders cycling through their range so
    /// every level shape meets every Enclosure and Gravity Chaos value.
    fn sweep_sliders(i: usize) -> Sliders {
        const STEPS: [u8; 5] = [0, 25, 50, 75, 100];
        const VERTICALITY: [u8; 5] = [5, 30, 50, 70, 95];
        const GRAVITY: [u8; 3] = [0, 50, 100];
        Sliders {
            enclosure: STEPS[i / 5 % 5],
            density: STEPS[i % 5],
            verticality: VERTICALITY[(i + i / 5) % 5],
            gravity_chaos: GRAVITY[i % 3],
            palette: (i * 37 % 101) as u8,
            absurdity: (i * 53 % 101) as u8,
            instability: (i * 71 % 101) as u8,
        }
    }

    /// Checks everything a playable setup needs, for `teams` teams.
    fn check_setup(c: &ShareCode, t: &GameTerrain, s: &MatchSetup, teams: u8) {
        // Every ship type fits (the Hauler has the largest hitbox).
        let radius = ShipTable::builtin().max_radius();
        let mid = t.bounds().center();
        let room = t.generator().room();
        for team in 0..teams {
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
            // Team 0 low x, team 1 high x, team 2 low z, team 3 high z, all
            // facing inwards.
            let fwd = sp.orientation.forward();
            match sp.team {
                0 => assert!(sp.pos.x < mid.x && fwd.x > 0.0, "{c}: {sp:?}"),
                1 => assert!(sp.pos.x > mid.x && fwd.x < 0.0, "{c}: {sp:?}"),
                2 => assert!(sp.pos.z < mid.z && fwd.z > 0.0, "{c}: {sp:?}"),
                _ => assert!(sp.pos.z > mid.z && fwd.z < 0.0, "{c}: {sp:?}"),
            }
            for other in &s.spawns[i + 1..] {
                if other.team == sp.team {
                    assert!(other.pos.distance(sp.pos) >= SPAWN_SPACING, "{c}");
                }
            }
            // No spawn within 40 u of a gravity well.
            for w in &s.wells {
                assert!(
                    w.pos.distance(sp.pos) >= WELL_SPAWN_DISTANCE,
                    "{c}: spawn {sp:?} near well {w:?}"
                );
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
        // The team 2/3 caverns reach the network.
        let net = gen.network();
        for team in 2..4u8 {
            for h in net.team_hubs(team) {
                assert!(
                    gen.reaches_network(*h, CONNECT_MARGIN, LINK_TRIES),
                    "{c}: team {team} cavern {h:?} does not reach the network"
                );
            }
        }
        // Gravity wells: the setup carries them, each hollow reaches the
        // network, and no spawn cavern lies within 40 u.
        assert_eq!(s.wells, gen.gravity_wells(), "{c}");
        assert_eq!(s.belt, None);
        for w in &s.wells {
            let r = 0.5 * (w.core_radius + w.hollow_radius);
            let ring = [
                Vec3::X,
                Vec3::NEG_X,
                Vec3::Y,
                Vec3::NEG_Y,
                Vec3::Z,
                Vec3::NEG_Z,
            ];
            assert!(
                ring.iter().any(|d| gen.reaches_network(
                    w.pos + *d * r,
                    CONNECT_MARGIN,
                    LINK_TRIES
                )),
                "{c}: well {w:?} does not reach the network"
            );
            for team in 0..4u8 {
                for h in net.team_hubs(team) {
                    assert!(h.distance(w.pos) >= WELL_SPAWN_DISTANCE, "{c}: {w:?}");
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
        check_setup(&c, &t, &a, 2);
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
        check_setup(&c, &terrain, &setup, 2);
        assert_eq!(world.tick(), 0);
    }

    /// The setup sweep: 8 seeds x 25 slider mixes (Enclosure x Density,
    /// with every level shape, Gravity Chaos and the other sliders cycled
    /// in) give a valid, steppable match, with two teams and with four.
    #[test]
    fn every_slider_combination_hosts_a_match() {
        // Debug builds are slow; release runs (CI's wasm32 job, or
        // `cargo test -p ls-game --release --lib setup`) sweep 8 seeds.
        let seeds: &[u64] = if cfg!(debug_assertions) {
            &[0xDEAD_BEEF]
        } else {
            &[0xDEAD_BEEF, 1, 2, 3, 42, 2024, 7777, u64::MAX]
        };
        let mixes = if cfg!(debug_assertions) { 6 } else { 25 };
        for &seed in seeds {
            for i in 0..mixes {
                let s = sweep_sliders(i);
                let c = ShareCode::new(seed ^ ((s.enclosure as u64) << 8 | s.density as u64), s);
                let terrain = GameTerrain::from_code(&c).unwrap();
                let teams = if i % 4 == 3 { 4 } else { 2 };
                let mut cfg = MatchConfig::paint_the_void(c);
                cfg.rules = rules_with_teams(teams);
                let setup = match_setup(&c, &cfg.rules, &terrain)
                    .unwrap_or_else(|err| panic!("{c}: {err}"));
                check_setup(&c, &terrain, &setup, teams);
                // What build_match does with it.
                let world =
                    World::new(cfg.world_config(setup)).unwrap_or_else(|err| panic!("{c}: {err}"));
                assert_eq!(world.tick(), 0);
            }
        }
    }

    /// M3 leftover (D3): a 4-team match spawns each team in its own zone
    /// (teams 2 and 3 at the z ends), and every ship of a team respawns
    /// there.
    #[test]
    fn four_team_match_spawns_each_team_in_its_own_zone() {
        for (seed, v) in [(11u64, 50u8), (12, 5), (13, 95)] {
            let s = Sliders {
                verticality: v,
                ..Sliders::new(45, 35)
            };
            let c = ShareCode::new(seed, s);
            let t = GameTerrain::from_code(&c).unwrap();
            let rules = rules_with_teams(4);
            let setup = match_setup(&c, &rules, &t).unwrap();
            check_setup(&c, &t, &setup, 4);
            let room = t.generator().room();
            let inner = Aabb::new(
                room.min + Vec3::splat(WALL_MARGIN),
                room.max - Vec3::splat(WALL_MARGIN),
            );
            let zones: Vec<Aabb> = (0..4).map(|team| spawn_zone(inner, team, 4)).collect();
            // The four zones are disjoint.
            for a in 0..4 {
                for b in a + 1..4 {
                    let (za, zb) = (zones[a], zones[b]);
                    let overlap = za.min.x < zb.max.x
                        && zb.min.x < za.max.x
                        && za.min.z < zb.max.z
                        && zb.min.z < za.max.z;
                    assert!(!overlap, "{c}: zones {a} and {b} overlap");
                }
            }
            for sp in &setup.spawns {
                assert!(
                    zones[sp.team as usize].contains(sp.pos),
                    "{c}: team {} spawn {:?} outside its zone",
                    sp.team,
                    sp.pos
                );
            }
            // In the simulation every team respawns at its own points.
            let mut cfg = MatchConfig::paint_the_void(c);
            cfg.rules = rules;
            let mut world = World::new(cfg.world_config(setup)).unwrap();
            let mut seen = [false; 4];
            for _ in 0..8 {
                let p = world.add_player(None, ls_sim::ShipType::Tinker).unwrap();
                let ship = world.ship(p).unwrap();
                let team = ship.team as usize;
                assert!(team < 4);
                assert!(
                    zones[team].contains(ship.pos),
                    "{c}: team {team} at {:?}",
                    ship.pos
                );
                seen[team] = true;
            }
            assert_eq!(seen, [true; 4], "{c}");
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
        for team in 0..4u8 {
            for hub in net.team_hubs(team) {
                assert_eq!(grid.nearest_component(*hub, 2.0), Some(main), "{c}");
            }
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
                for i in 0..25 {
                    let s = sweep_sliders(i);
                    v.push(ShareCode::new(
                        seed ^ ((s.enclosure as u64) << 8 | s.density as u64),
                        s,
                    ));
                }
            }
            v
        };
        let rules = RuleSet::paint_the_void();
        for c in codes {
            let t = GameTerrain::from_code(&c).unwrap();
            let s = match_setup(&c, &rules, &t).unwrap();
            check_setup(&c, &t, &s, 2);
            check_flood_fill(&c, &t, &s);
        }
    }

    // ----- asteroid belt ---------------------------------------------------

    /// The racing line of start slot `k` along the belt's clear corridor:
    /// the slot, then the corridor's centre line prograde from the slot for
    /// a full lap, ending just past the slot's own gate.
    fn lap_line(gen: &Generator, k: usize) -> Vec<Vec3> {
        let b = gen.belt().unwrap();
        let corridor = gen.belt_field().unwrap().corridor();
        let n = corridor.len();
        let slot = b.slots[k].pos;
        let a0 = b.azimuth(slot);
        // First corridor point ahead of the slot.
        let start = (0..n)
            .min_by(|&i, &j| {
                let ahead = |p: Vec3| {
                    let d = b.azimuth(p) - a0;
                    if d <= 0.0 {
                        d + core::f32::consts::TAU
                    } else {
                        d
                    }
                };
                ahead(corridor[i]).total_cmp(&ahead(corridor[j]))
            })
            .unwrap();
        let mut line = vec![slot];
        for i in 0..=n {
            line.push(corridor[(start + i) % n]);
        }
        line
    }

    /// Belt-only line from every slot through its 8 gates in order (own gate
    /// last), keeping [`CONNECT_MARGIN`], then the dive from the inner edge
    /// to the exit; pickup points, moons and the cocoon as §3.8 says.
    fn check_belt(c: &ShareCode, t: &GameTerrain, s: &MatchSetup) {
        let gen = t.generator();
        let b = gen.belt().expect("belt");
        let sample = |p: Vec3| gen.sample(p);
        assert_eq!(s.belt.as_ref(), Some(b), "{c}");
        assert_eq!(s.wells, gen.gravity_wells(), "{c}");
        assert!(s.heist.is_none());
        assert_eq!(s.spawns.len(), BELT_WEDGES);
        for (k, sp) in s.spawns.iter().enumerate() {
            assert_eq!(sp.pos, b.slots[k].pos);
            assert!(t.sdf(sp.pos) >= SPAWN_CLEARANCE, "{c}: slot {k}");
        }
        assert_eq!(s.pickup_points, b.pickup_points);
        for k in 0..BELT_WEDGES {
            let line = lap_line(&gen, k);
            let mut gates = Vec::new();
            for w in line.windows(2) {
                assert!(
                    segment_clear(sample, w[0], w[1], CONNECT_MARGIN),
                    "{c}: slot {k} line blocked between {:?} and {:?}",
                    w[0],
                    w[1]
                );
                assert!(b.belt_distance(w[1]) < 0.0, "{c}: line leaves the belt");
                for g in 0..BELT_WEDGES {
                    if b.crosses_gate(g, w[0], w[1]) {
                        gates.push(g);
                    }
                }
            }
            let want: Vec<usize> = (1..=BELT_WEDGES).map(|i| (k + i) % BELT_WEDGES).collect();
            assert_eq!(gates, want, "{c}: slot {k} gates");
            // The dive: past the own gate, from the slot's position straight
            // to the exit.
            let last = *line.last().unwrap();
            let slot = b.slots[k].pos;
            assert!(segment_clear(sample, last, slot, CONNECT_MARGIN), "{c}");
            assert!(
                segment_clear(sample, slot, b.center, CONNECT_MARGIN),
                "{c}: dive {k}"
            );
        }
        // Pickups: open, in the belt, reach the corridor.
        let corridor = gen.belt_field().unwrap().corridor();
        for p in &b.pickup_points {
            assert!(t.sdf(*p) >= MIN_CLEARANCE, "{c}: pickup {p:?}");
            assert!(
                b.belt_distance(*p) < 0.0,
                "{c}: pickup {p:?} outside the belt"
            );
            let near = corridor
                .iter()
                .min_by(|a, q| a.distance(*p).total_cmp(&q.distance(*p)))
                .unwrap();
            assert!(
                segment_clear(sample, *p, *near, CONNECT_MARGIN),
                "{c}: pickup {p:?}"
            );
        }
        // Shepherd moons: one per wedge from Gravity Chaos 20, 25 u from
        // the gates and slots, not on the racing line.
        if c.sliders.gravity_chaos >= ls_worldgen::belt::MOON_GRAVITY_CHAOS {
            assert_eq!(s.wells.len(), BELT_WEDGES, "{c}");
        } else {
            assert!(s.wells.is_empty(), "{c}");
        }
        for m in &s.wells {
            for slot in &b.slots {
                assert!(m.pos.distance(slot.pos) >= 25.0, "{c}: moon {m:?}");
            }
            for p in corridor {
                assert!(
                    p.distance(m.pos) >= m.hollow_radius,
                    "{c}: moon on the line"
                );
            }
        }
        // No rock in the cocoon.
        let r = b.cocoon_radius;
        let mut z = -r;
        while z <= r {
            let mut y = -r;
            while y <= r {
                let mut x = -r;
                while x <= r {
                    let p = b.center + Vec3::new(x, y, z);
                    if p.distance(b.center) <= r {
                        assert!(t.sdf(p) > 0.0, "{c}: rock in the cocoon at {p:?}");
                    }
                    x += 3.0;
                }
                y += 3.0;
            }
            z += 3.0;
        }
    }

    /// The belt setup sweep: 8 seeds x 25 slider mixes of belt worlds.
    #[test]
    fn every_belt_world_hosts_a_race() {
        let seeds: &[u64] = if cfg!(debug_assertions) {
            &[0xDEAD_BEEF]
        } else {
            &[0xDEAD_BEEF, 1, 2, 3, 42, 2024, 7777, u64::MAX]
        };
        let mixes = if cfg!(debug_assertions) { 3 } else { 25 };
        for &seed in seeds {
            for i in 0..mixes {
                let s = sweep_sliders(i);
                let c = ShareCode::with_archetype(seed ^ (i as u64) << 16, s, Archetype::Belt);
                let t = GameTerrain::from_code(&c).unwrap();
                let rules = RuleSet::paint_the_void();
                let setup = match_setup(&c, &rules, &t).unwrap();
                check_belt(&c, &t, &setup);
                let cfg = MatchConfig::paint_the_void(c);
                let world = World::new(cfg.world_config(setup)).unwrap();
                assert_eq!(world.tick(), 0);
            }
        }
    }

    /// Fairness: the shortest belt-only lap of every start slot (through
    /// its 8 gates, own gate last, keeping [`CONNECT_MARGIN`]) on a 2 u
    /// lattice; the 8 lengths differ by less than 1 %.
    fn lap_lengths(gen: &Generator) -> [f32; BELT_WEDGES] {
        let b = gen.belt().unwrap();
        let step = 2.0f32;
        let r_max = b.outer_radius + step;
        let lo = b.center - Vec3::new(r_max, b.half_thickness + step, r_max);
        let dims = [
            (2.0 * r_max / step) as usize + 1,
            (2.0 * (b.half_thickness + step) / step) as usize + 1,
            (2.0 * r_max / step) as usize + 1,
        ];
        let at = |i: [usize; 3]| lo + Vec3::new(i[0] as f32, i[1] as f32, i[2] as f32) * step;
        let idx = |i: [usize; 3]| i[0] + dims[0] * (i[1] + dims[1] * i[2]);
        let n = dims[0] * dims[1] * dims[2];
        let mut free = vec![false; n];
        let mut azim = vec![0.0f32; n];
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let p = at([x, y, z]);
                    let i = idx([x, y, z]);
                    free[i] = b.belt_distance(p) <= 0.0 && gen.sample(p) >= CONNECT_MARGIN;
                    azim[i] = b.azimuth(p);
                }
            }
        }
        let tau = core::f32::consts::TAU;
        let mut out = [0.0f32; BELT_WEDGES];
        for (k, len) in out.iter_mut().enumerate() {
            let gate = BeltSite::gate_azimuth(k);
            // Cut between the gate and the slot (1.5 degrees past the gate):
            // the lap must go all the way round.
            let cut = gate + 1.5f32.to_radians();
            let rel = |a: f32| {
                let d = a - cut;
                if d < 0.0 {
                    d + tau
                } else {
                    d
                }
            };
            let goal = tau - 1.5f32.to_radians();
            let slot = b.slots[k].pos;
            let s = (slot - lo) / step;
            let start = [s.x, s.y, s.z].map(|v| ls_math::det::round(v) as usize);
            let mut dist = vec![f32::INFINITY; n];
            let mut heap = std::collections::BinaryHeap::new();
            let si = idx(start);
            dist[si] = slot.distance(at(start));
            heap.push((std::cmp::Reverse(dist[si].to_bits()), si));
            *len = f32::INFINITY;
            while let Some((std::cmp::Reverse(d), i)) = heap.pop() {
                let d = f32::from_bits(d);
                if d > dist[i] {
                    continue;
                }
                if rel(azim[i]) >= goal {
                    *len = d;
                    break;
                }
                let (x, y, z) = (
                    i % dims[0],
                    (i / dims[0]) % dims[1],
                    i / (dims[0] * dims[1]),
                );
                for dz in -1i32..=1 {
                    for dy in -1i32..=1 {
                        for dx in -1i32..=1 {
                            if dx == 0 && dy == 0 && dz == 0 {
                                continue;
                            }
                            let (nx, ny, nz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
                            if nx < 0
                                || ny < 0
                                || nz < 0
                                || nx as usize >= dims[0]
                                || ny as usize >= dims[1]
                                || nz as usize >= dims[2]
                            {
                                continue;
                            }
                            let j = idx([nx as usize, ny as usize, nz as usize]);
                            // No jumps across the cut.
                            if !free[j] || (rel(azim[j]) - rel(azim[i])).abs() > 1.0 {
                                continue;
                            }
                            let w = step * ((dx * dx + dy * dy + dz * dz) as f32).sqrt();
                            let nd = d + w;
                            if nd < dist[j] {
                                dist[j] = nd;
                                heap.push((std::cmp::Reverse(nd.to_bits()), j));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn every_slot_flies_a_lap_of_the_same_length() {
        let worlds: Vec<(u64, Sliders)> = if cfg!(debug_assertions) {
            vec![(5, sweep_sliders(12))]
        } else {
            (0..8u64)
                .map(|i| (i * 7919 + 3, sweep_sliders(i as usize * 3)))
                .collect()
        };
        for (seed, s) in worlds {
            let c = ShareCode::with_archetype(seed, s, Archetype::Belt);
            let gen = Generator::from_code(&c).unwrap();
            let laps = lap_lengths(&gen);
            let max = laps.iter().copied().fold(0.0f32, f32::max);
            let min = laps.iter().copied().fold(f32::INFINITY, f32::min);
            assert!(max.is_finite(), "{c}: no lap {laps:?}");
            let spread = (max - min) / min;
            assert!(spread < 0.01, "{c}: laps {laps:?} spread {spread}");
            // The lap is about as long as the ring (597-911 u): belt-only.
            assert!((550.0..1000.0).contains(&min), "{c}: lap {min}");
            println!("{c}: lap lengths {laps:?} spread {:.4} %", spread * 100.0);
        }
    }

    #[test]
    fn fallback_candidates_never_sit_on_the_midplane() {
        let mid = Vec3::new(128.0, 64.0, 128.0);
        let cands = vec![
            (Vec3::new(100.0, 10.0, 10.0), 2.0),
            (Vec3::new(mid.x, 10.0, 10.0), 3.0),
            (Vec3::new(140.0, 10.0, 200.0), 2.0),
        ];
        let xs = |team, teams| -> Vec<f32> {
            on_own_side(cands.clone(), mid, team, teams)
                .iter()
                .map(|(p, _)| p.x)
                .collect()
        };
        assert_eq!(xs(0, 2), vec![100.0]);
        assert_eq!(xs(1, 2), vec![140.0]);
        // Teams 2 and 3 split along z.
        assert_eq!(xs(2, 4), vec![100.0, mid.x]);
        assert_eq!(xs(3, 4), vec![140.0]);
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
