//! World physics replay hashes (M4 package A4): one 40 s scripted 8-ship
//! replay each for gravity wells, cave-ins, pigment and radiation, for the
//! native-vs-WASM determinism cross-check.
//!
//! Each replay uses the [`crate::golden`] scripted inputs (pseudo-random
//! buttons and thrust, aiming at the nearest enemy):
//!
//! | Name | Level | Rule set |
//! | --- | --- | --- |
//! | `world/gravity` | the golden arena, Gravity Chaos 100 with three wells, a dynamic well from 10 s to 20 s | Sandbox (every weapon as a pickup: ordnance bends) |
//! | `world/cave_in` | a low arena (160 x 12 x 160 u, so every crater has a ceiling in reach), Instability 100, spawns on a 42 u ring | Team Brawl with 10 Bolt Rockets each |
//! | `world/pigment` | the golden arena as a monochrome level, pickup points at the spawn points; odd players fly at the nearest pigment | Team Brawl |
//! | `world/radiation` | a small belt (20-40 u, 6 u thick) with half the ships above it | Team Brawl |
//!
//! The hashes live in `crates/ls-sim/tests/fixtures/mood_replays.txt`
//! (lines starting with `world/`; the file also holds the director's event
//! replays) and are checked by the unit test below on native and wasm32.
//! Re-bless after a deliberate change with
//! `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib world_replays`
//! (rewrites only the `world/` lines).

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
use ls_math::{Pcg32, Quat, Vec3};

use crate::belt::BeltLevel;
use crate::golden::{arena, inputs, setup, Arena};
use crate::gravity::{DynamicWell, WellSite};
use crate::ruleset::BUILTIN_RULESETS;
use crate::{
    CueKind, Event, MatchSetup, PaintedTerrain, PlayerId, RuleSet, ShelledTerrain, ShipType,
    SimTerrain, SpawnPoint, World, WorldConfig,
};

/// Ticks per replay (40 s).
pub const WORLD_REPLAY_TICKS: u64 = 40 * 60;
/// Ships per replay.
pub const WORLD_REPLAY_SHIPS: usize = 8;
/// Seed of every replay.
pub const WORLD_REPLAY_SEED: u64 = 2025;
/// The replays, in fixture order.
pub const WORLD_REPLAYS: [&str; 4] = [
    "world/gravity",
    "world/cave_in",
    "world/pigment",
    "world/radiation",
];

/// Outcome of one [`world_replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldReplay {
    pub name: &'static str,
    /// Final state hash folded with the state hash of every second.
    pub hash: u64,
    pub ticks: u64,
    pub events: usize,
    pub kills: usize,
    /// Deaths without a killer (radiation, cave-ins, the level).
    pub level_kills: usize,
    /// Cave-in warnings.
    pub cave_ins: usize,
    /// Rubble spheres that came down.
    pub rubble: usize,
    /// Pigment pickups taken.
    pub pigments: usize,
}

fn rules(file: &str) -> RuleSet {
    let (f, text) = BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| f.ends_with(file))
        .expect("built-in rule set");
    let mut r = RuleSet::from_ron(f, text).expect("valid");
    r.mutators.clear();
    r
}

/// A low arena: 160 x 12 x 160 u with the golden vein knot.
fn low_arena() -> Arena {
    let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(160.0, 12.0, 160.0)), 2.0);
    t.apply_edit(&TerrainEdit::AddSphere {
        center: Vec3::new(0.0, -6.0, 0.0),
        radius: 2.0,
    });
    ShelledTerrain::new(t)
}

/// A small belt around the origin: 20-40 u, 6 u half-thickness, a 8 u
/// cocoon; slots on the ring at 30 u, half of them 14 u above the belt.
fn small_belt() -> BeltLevel {
    let mut b = BeltLevel {
        center: Vec3::ZERO,
        inner_radius: 20.0,
        outer_radius: 40.0,
        half_thickness: 6.0,
        cocoon_radius: 8.0,
        exit_radius: 3.0,
        wedges: 8,
        gate_inner: 16.0,
        gate_outer: 44.0,
        slots: Vec::new(),
        boulders: Vec::new(),
    };
    b.slots = (0..8u8)
        .map(|k| {
            let a = b.gate_angle(k) + 0.05;
            SpawnPoint {
                pos: b.point(a, 30.0, if k % 2 == 0 { 0.0 } else { 20.0 }),
                team: k % 2,
                orientation: Quat::from_axis_angle(Vec3::Y, a),
            }
        })
        .collect();
    b
}

fn level(name: &str) -> (RuleSet, MatchSetup, Arena) {
    let mut s = setup();
    match name {
        "world/gravity" => {
            s.gravity_chaos = 100;
            s.wells = vec![
                WellSite {
                    pos: Vec3::new(16.0, 6.0, 10.0),
                    core_radius: 2.0,
                },
                WellSite {
                    pos: Vec3::new(-16.0, -6.0, -10.0),
                    core_radius: 2.0,
                },
                WellSite {
                    pos: Vec3::new(0.0, 10.0, -20.0),
                    core_radius: 1.5,
                },
            ];
            (rules("sandbox_all_weapons.ron"), s, arena())
        }
        "world/cave_in" => {
            // Spawns on a wide ring (cave-ins keep 24 u from them), a full
            // rack of Bolt Rockets each.
            s.instability = 100;
            for sp in &mut s.spawns {
                sp.pos = Vec3::new(sp.pos.x * 3.0, sp.pos.y.clamp(-3.0, 3.0), sp.pos.z * 3.0);
            }
            s.heist = None;
            let mut r = rules("team_brawl.ron");
            r.starting_loadout.secondaries = vec![("bolt_rocket".to_string(), 10)];
            (r, s, low_arena())
        }
        "world/pigment" => {
            s.monochrome = true;
            // Pickup points at the spawn points: respawning ships take them.
            s.pickup_points = s.spawns.iter().map(|sp| sp.pos).collect();
            (rules("team_brawl.ron"), s, arena())
        }
        _ => {
            let belt = small_belt();
            s.spawns = belt.slots.clone();
            s.belt = Some(belt);
            s.heist = None;
            let t = ShelledTerrain::new(PaintedTerrain::new(
                BoxRoomTerrain::centered(Vec3::new(100.0, 60.0, 100.0)),
                2.0,
            ));
            (rules("team_brawl.ron"), s, t)
        }
    }
}

/// Plays one world replay (see the module docs); `None` for an unknown name.
pub fn world_replay(name: &str) -> Option<WorldReplay> {
    let &name = WORLD_REPLAYS.iter().find(|n| **n == name)?;
    let (rules, setup, mut t) = level(name);
    let mut cfg = WorldConfig::paint_the_void(WORLD_REPLAY_SEED, setup);
    cfg.rules = rules;
    let mut w = World::new(cfg).expect("replay world");
    let mut ids: Vec<PlayerId> = Vec::new();
    for ty in ShipType::ALL.iter().cycle().take(4 * ShipType::ALL.len()) {
        if ids.len() == WORLD_REPLAY_SHIPS {
            break;
        }
        if let Ok(id) = w.add_player(None, *ty) {
            ids.push(id);
        }
    }
    let mut rng = Pcg32::new(WORLD_REPLAY_SEED, 17);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let mut r = WorldReplay {
        name,
        hash: 0,
        ticks: 0,
        events: 0,
        kills: 0,
        level_kills: 0,
        cave_ins: 0,
        rubble: 0,
        pigments: 0,
    };
    for tick in 1..=WORLD_REPLAY_TICKS {
        if name == "world/gravity" && tick == 600 {
            w.add_well(DynamicWell {
                key: 1,
                pos: Vec3::new(0.0, 0.0, 12.0),
                peak_accel: 9.0,
                range: 16.0,
                start_tick: 600,
                end_tick: 1200,
                owner: ids.first().copied(),
            });
        }
        let mut input = inputs(&mut rng, &w, &ids, tick);
        if name == "world/pigment" {
            hunt_pigment(&w, &mut input);
        }
        for e in w.step(&mut t, &input) {
            r.events += 1;
            match e {
                Event::Killed { killer, .. } => {
                    r.kills += 1;
                    r.level_kills += killer.is_none() as usize;
                }
                Event::Cue(c) if c.kind == CueKind::CaveInWarning => r.cave_ins += 1,
                Event::Cue(c) if c.kind == CueKind::Pigment => r.pigments += 1,
                Event::TerrainEdit(TerrainEdit::AddSphere { .. }) => r.rubble += 1,
                _ => {}
            }
        }
        r.ticks = tick;
        if tick % 60 == 0 {
            hash = fold(hash, w.state_hash());
        }
        if !w.mode().is_running() {
            break;
        }
    }
    r.hash = fold(hash, w.state_hash());
    Some(r)
}

/// Pigment replay: odd players fly at the nearest pigment pickup (the rest
/// keep the scripted inputs), so pickups, the damage bonus and splashes all
/// happen.
fn hunt_pigment(w: &World, input: &mut [(PlayerId, crate::InputCmd)]) {
    for (id, cmd) in input.iter_mut() {
        if id.0 % 2 == 0 {
            continue;
        }
        let Some(me) = w.ship(*id).filter(|s| s.alive) else {
            continue;
        };
        let Some(target) = w.pigment_pickups().iter().map(|p| p.pos).min_by(|a, b| {
            a.distance_squared(me.pos)
                .total_cmp(&b.distance_squared(me.pos))
        }) else {
            continue;
        };
        let local = me.orientation.conjugate().rotate(target - me.pos);
        cmd.look_pitch = ls_math::det::atan2(local.y, -local.z).clamp(-0.1, 0.1);
        cmd.look_yaw = ls_math::det::atan2(-local.x, -local.z).clamp(-0.1, 0.1);
        cmd.thrust = Vec3::new(0.0, 0.0, -1.0);
    }
}

/// Every world replay, in [`WORLD_REPLAYS`] order.
pub fn world_replays() -> Vec<WorldReplay> {
    WORLD_REPLAYS
        .iter()
        .filter_map(|n| world_replay(n))
        .collect()
}

/// Fixture lines (`<name> <hash as 0x%016x>`) of every world replay: the
/// format of `tests/fixtures/mood_replays.txt`.
pub fn world_replay_hashes() -> Vec<String> {
    world_replays()
        .iter()
        .map(|r| format!("{} {:#018x}", r.name, r.hash))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const FIXTURE: &str = include_str!("../tests/fixtures/mood_replays.txt");

    fn fixture_world_lines() -> Vec<String> {
        FIXTURE
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("world/"))
            .map(str::to_string)
            .collect()
    }

    /// Every world replay matches the fixture (native and wasm32), and each
    /// exercises its system. `LS_BLESS_MOOD_REPLAYS=1` rewrites the
    /// `world/` lines of the fixture (native).
    #[test]
    fn world_replays_match_the_fixture() {
        let replays = world_replays();
        for r in &replays {
            println!("{r:?}");
        }
        let by = |n: &str| replays.iter().find(|r| r.name == n).unwrap();
        assert!(by("world/gravity").events > 100);
        assert!(by("world/cave_in").cave_ins >= 1 && by("world/cave_in").rubble >= 1);
        assert!(by("world/pigment").pigments >= 4);
        assert!(by("world/radiation").level_kills >= 1);
        let got: Vec<String> = replays
            .iter()
            .map(|r| format!("{} {:#018x}", r.name, r.hash))
            .collect();
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var_os("LS_BLESS_MOOD_REPLAYS").is_some() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/mood_replays.txt"
            );
            let mut text: String = FIXTURE
                .lines()
                .filter(|l| !l.trim().starts_with("world/"))
                .map(|l| format!("{l}\n"))
                .collect();
            for l in &got {
                text.push_str(l);
                text.push('\n');
            }
            std::fs::write(path, text).expect("write the fixture");
            return;
        }
        assert_eq!(
            got,
            fixture_world_lines(),
            "world replay hashes changed: re-bless with LS_BLESS_MOOD_REPLAYS=1 if deliberate"
        );
    }

    /// The physics really act: the same replay without its system differs.
    #[test]
    fn world_replays_are_deterministic() {
        let a = world_replay("world/gravity").unwrap();
        let b = world_replay("world/gravity").unwrap();
        assert_eq!(a, b);
        assert!(world_replay("world/nope").is_none());
    }
}
