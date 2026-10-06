//! The Belt Up replay golden (M4 plan §4: "a Belt Up replay: one full heat
//! on a belt world with bumps and a respawn") and the test belt it runs on.
//!
//! [`test_belt`] is a full-size belt (the §3.8 radii, 12 u half thickness)
//! in a 320 x 96 x 320 u box, with voxel-free asteroids as rock spheres off
//! the racing circle (radius 120 u) and the dive lanes, boulders (two lanes
//! cross the racing circle, so racers bump into them) and three pickup
//! points per wedge. [`belt_up_replay`] races one heat there with 8
//! autopiloted ships ([`super::pilot`]): most on the racing circle (half of
//! them boosting, two shooting the ship ahead), a slow one with a rammer
//! behind it (bumps), one in the outer boulder lane that does not dodge
//! (boulder hits) and one skimming 4 u above the belt (radiation takes its
//! hull, it dies and respawns at its gate).
//!
//! The hash (the state hash of every second folded with the final one)
//! lives in `crates/ls-sim/tests/fixtures/mood_replays.txt` as the
//! `mode/belt_up` line, checked on native and wasm32. Re-bless after a
//! deliberate change with
//! `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib belt_up::tests::replay`
//! (rewrites only that line).

use ls_math::level::{BeltSite, BeltSlot, BoulderOrbit, BELT_WEDGES};
use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
use ls_math::{det, Quat, Vec3};

use super::pilot::{steer, PilotStyle};
use crate::golden::Arena;
use crate::ruleset::BUILTIN_RULESETS;
use crate::{
    Event, MatchSetup, PaintedTerrain, PlayerId, RuleSet, ShelledTerrain, ShipType, SimTerrain,
    SpawnPoint, World, WorldConfig,
};

/// Fixture name of the replay.
pub const BELT_UP_REPLAY: &str = "mode/belt_up";
/// Seed of the replay.
pub const BELT_UP_REPLAY_SEED: u64 = 2026;
/// Longest the replay runs (the heat ends first).
pub const BELT_UP_REPLAY_MAX_TICKS: u64 = 120 * 60;

/// Point at `azimuth` (prograde from +X), `radius` from the axis, `height`
/// above the plane.
pub fn belt_point(b: &BeltSite, azimuth: f32, radius: f32, height: f32) -> Vec3 {
    let (s, c) = det::sin_cos(azimuth);
    b.center + Vec3::new(c * radius, height, -s * radius)
}

/// The full-size test belt around the origin (see the module docs).
pub fn test_belt() -> BeltSite {
    let mut b = BeltSite {
        center: Vec3::ZERO,
        inner_radius: 95.0,
        outer_radius: 145.0,
        half_thickness: 12.0,
        cocoon_radius: 30.0,
        exit_radius: 6.0,
        gate_inner_radius: 87.0,
        gate_outer_radius: 153.0,
        slots: [BeltSlot {
            pos: Vec3::ZERO,
            orientation: Quat::IDENTITY,
        }; BELT_WEDGES],
        pickup_points: Vec::new(),
        boulders: Vec::new(),
    };
    let deg = core::f32::consts::PI / 180.0;
    for k in 0..BELT_WEDGES {
        let gate = BeltSite::gate_azimuth(k);
        let a = gate + 3.0 * deg;
        b.slots[k] = BeltSlot {
            pos: belt_point(&b, a, 120.0, 0.0),
            orientation: Quat::from_axis_angle(Vec3::Y, a),
        };
        for (i, r) in [112.0, 120.0, 128.0].into_iter().enumerate() {
            b.pickup_points.push(belt_point(
                &b,
                gate + (14.0 + 10.0 * i as f32) * deg,
                r,
                0.0,
            ));
        }
        // An outer boulder lane in every wedge ...
        b.boulders.push(BoulderOrbit {
            radius: 136.0,
            height: 3.0,
            phase: gate + 20.0 * deg,
            angular_speed: 3.0 / 136.0,
            size: 3.0,
            tumble_axis: Vec3::new(0.0, 0.6, 0.8),
            tumble_speed: 0.4,
        });
        // ... and in two wedges a slow boulder on the racing circle.
        if k % 4 == 1 {
            b.boulders.push(BoulderOrbit {
                radius: 120.0,
                height: 0.0,
                phase: gate + 30.0 * deg,
                angular_speed: 2.0 / 120.0,
                size: 2.5,
                tumble_axis: Vec3::X,
                tumble_speed: 0.3,
            });
        }
    }
    b
}

/// The rock of the test belt: spheres off the racing circle and the dive
/// lanes (8 per wedge).
pub fn test_asteroids(b: &BeltSite) -> Vec<(Vec3, f32)> {
    let deg = core::f32::consts::PI / 180.0;
    let mut out = Vec::new();
    for k in 0..BELT_WEDGES {
        let gate = BeltSite::gate_azimuth(k);
        for (i, (r, h, size)) in [
            (101.0, 4.0, 4.0),
            (106.0, -5.0, 3.0),
            (138.0, -4.0, 4.5),
            (142.0, 5.0, 3.0),
        ]
        .into_iter()
        .enumerate()
        {
            for (j, off) in [12.0, 32.0].into_iter().enumerate() {
                let a = gate + (off + 3.0 * i as f32 + 2.0 * j as f32) * deg;
                out.push((belt_point(b, a, r, h), size));
            }
        }
    }
    out
}

/// The test belt as a match setup and its terrain (a 320 x 96 x 320 u box
/// with the asteroids).
pub fn belt_arena() -> (MatchSetup, Arena) {
    let belt = test_belt();
    let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(320.0, 96.0, 320.0)), 2.0);
    for (center, radius) in test_asteroids(&belt) {
        t.apply_edit(&TerrainEdit::AddSphere { center, radius });
    }
    let setup = MatchSetup {
        spawns: belt
            .slots
            .iter()
            .map(|s| SpawnPoint {
                pos: s.pos,
                team: 0,
                orientation: s.orientation,
            })
            .collect(),
        pickup_points: belt.pickup_points.clone(),
        belt: Some(belt),
        ..Default::default()
    };
    (setup, ShelledTerrain::new(t))
}

/// The built-in Belt Up rule set.
pub fn belt_up_rules() -> RuleSet {
    let (f, text) = BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| f.ends_with("belt_up.ron"))
        .expect("built-in Belt Up rule set");
    RuleSet::from_ron(f, text).expect("valid")
}

/// The autopilot style of replay ship `i`.
pub fn replay_style(i: usize, b: &BeltSite) -> PilotStyle {
    let base = PilotStyle::default();
    match i {
        6 => PilotStyle {
            height: b.half_thickness + 4.0,
            ..base
        },
        // A slow racer and a rammer behind it (bumps) ...
        0 => PilotStyle {
            throttle: 0.5,
            boost: false,
            ..base
        },
        7 => PilotStyle {
            ram_range: 25.0,
            ..base
        },
        // ... and one that does not dodge boulders.
        3 => PilotStyle {
            avoid_boulders: false,
            circle_radius: 136.0,
            height: 3.0,
            boost: false,
            ..base
        },
        _ => PilotStyle {
            boost: i % 2 == 0,
            shoot: i == 1 || i == 5,
            ..base
        },
    }
}

/// Outcome of [`belt_up_replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeltUpReplay {
    pub hash: u64,
    pub ticks: u64,
    pub events: usize,
    /// Racers that reached the exit.
    pub finishers: usize,
    /// Bumps between ships.
    pub bumps: usize,
    /// Deaths and the respawns after them (at gates).
    pub deaths: usize,
    pub respawns: usize,
    /// Ships that touched a boulder hard enough to be hurt.
    pub boulder_hits: usize,
    /// The heat ended (results reached).
    pub heat_over: bool,
}

/// Plays the Belt Up replay (see the module docs).
pub fn belt_up_replay() -> BeltUpReplay {
    let (setup, mut t) = belt_arena();
    let belt = setup.belt.clone().expect("belt");
    let mut cfg = WorldConfig::paint_the_void(BELT_UP_REPLAY_SEED, setup);
    let mut rules = belt_up_rules();
    for (f, v) in [
        ("heats", 1.0),
        ("results_secs", 2.0),
        ("finish_grace_secs", 15.0),
        ("heat_limit_secs", 110.0),
    ] {
        rules.mode.set_field(f, v);
    }
    cfg.rules = rules;
    let mut w = World::new(cfg).expect("replay world");
    let ids: Vec<PlayerId> = ShipType::ALL
        .iter()
        .cycle()
        .take(8)
        .map(|ty| w.add_player(None, *ty).expect("player"))
        .collect();
    let styles: Vec<PilotStyle> = (0..ids.len()).map(|i| replay_style(i, &belt)).collect();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let mut out = BeltUpReplay {
        hash: 0,
        ticks: 0,
        events: 0,
        finishers: 0,
        bumps: 0,
        deaths: 0,
        respawns: 0,
        boulder_hits: 0,
        heat_over: false,
    };
    for tick in 1..=BELT_UP_REPLAY_MAX_TICKS {
        let input: Vec<_> = ids
            .iter()
            .zip(&styles)
            .map(|(id, s)| (*id, steer(&w, *id, s, tick)))
            .collect();
        let before = w.mode().race.as_ref().map_or(0, |r| r.bump_ready.len());
        for e in w.step(&mut t, &input) {
            out.events += 1;
            match e {
                Event::Killed { .. } => out.deaths += 1,
                Event::Respawned { .. } if tick > 1 && out.deaths > 0 => out.respawns += 1,
                Event::Damaged {
                    attacker: None,
                    amount,
                    ..
                } if amount == crate::boulders::BoulderSettings::builtin().damage => {
                    out.boulder_hits += 1
                }
                _ => {}
            }
        }
        if let Some(r) = &w.mode().race {
            out.bumps += r.bump_ready.len().saturating_sub(before);
            if r.phase != super::heat_phase::RACING && r.start_tick > 0 {
                out.heat_over = true;
            }
        }
        out.ticks = tick;
        if tick % 60 == 0 {
            hash = fold(hash, w.state_hash());
        }
        if !w.mode().is_running() {
            break;
        }
    }
    out.finishers = w.mode().race.as_ref().map_or(0, |r| {
        r.racers.values().filter(|x| x.finish_tick > 0).count()
    });
    out.hash = fold(hash, w.state_hash());
    out
}

/// The fixture line of the replay (`mode/belt_up 0x...`; what `ls-wasm`
/// can export for the determinism cross-check).
pub fn belt_up_replay_line() -> String {
    format!("{} {:#018x}", BELT_UP_REPLAY, belt_up_replay().hash)
}

/// Runs `world` with the autopilot until the match ends or `max_ticks`
/// (tests): returns the ticks played.
pub fn race_to_end<T: SimTerrain + ?Sized>(
    w: &mut World,
    t: &mut T,
    styles: &[(PlayerId, PilotStyle)],
    max_ticks: u64,
) -> u64 {
    for n in 0..max_ticks {
        if !w.mode().is_running() {
            return n;
        }
        let tick = w.tick() + 1;
        let input: Vec<_> = styles
            .iter()
            .map(|(id, s)| (*id, steer(w, *id, s, tick)))
            .collect();
        w.step(t, &input);
    }
    max_ticks
}
