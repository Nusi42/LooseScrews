//! Per-mode replay hashes for the native-vs-WASM determinism cross-check
//! (M3 package C3; `tools/determinism-check.mjs`).
//!
//! [`mode_replay`] plays one scripted match per built-in rule set
//! ([`BUILTIN_RULESETS`]): 8 ships (every allowed ship type in turn) with
//! pseudo-random inputs that aim at the nearest enemy, in a closed box arena
//! with a rock knot (the Heist vein), two Heist extraction zones, a
//! [`ShelledTerrain`] (The Squeeze's walls) and eight pickup points. The
//! slow parts of a few modes are sped up through their generic settings
//! ([`ModeRules::set_field`]; unknown names are ignored), so the 40 s replay
//! reaches each mode's special rules: Squeeze phases close, the Last Color
//! Standing stalemate drains, the reactor's fuse runs out.
//!
//! The native hashes live in `crates/ls-sim/tests/fixtures/mode_replays.txt`
//! and are checked by the unit test below on native and on wasm32; `ls-wasm`
//! exports [`mode_replay_hashes`] (`modeReplayHashes`) so the determinism
//! cross-check compares the browser build with the same file under Node.
//! These replays are separate from the per-mode unit-test goldens in
//! `modes/*` (tailored scenarios) and from `REPLAY_GOLDEN_HASH`. Any change of
//! simulation behaviour, tuning or `data/` may change them: re-bless with
//! `LS_BLESS_MODE_REPLAYS=1 cargo test -p ls-sim --lib golden` (native) in the
//! same change.

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
use ls_math::{Pcg32, Quat, Vec3};

use crate::modes::heist::HeistLevel;
use crate::ruleset::BUILTIN_RULESETS;
use crate::{
    Buttons, Event, InputCmd, MatchSetup, PaintedTerrain, PlayerId, RuleSet, ShelledTerrain,
    ShipType, SimTerrain, SpawnPoint, World, WorldConfig,
};

/// Ticks per replay (40 s).
pub const MODE_REPLAY_TICKS: u64 = 40 * 60;
/// Ships per replay.
pub const MODE_REPLAY_SHIPS: usize = 8;
/// Seed of every replay.
pub const MODE_REPLAY_SEED: u64 = 2024;

const VEIN: Vec3 = Vec3::new(0.0, -8.0, 0.0);
const KNOT: f32 = 2.0;

/// Outcome of one [`mode_replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeReplay {
    /// Rule set file (`rulesets/<id>.ron`).
    pub file: &'static str,
    /// Final state hash folded with the state hash of every second.
    pub hash: u64,
    /// Ticks played (fewer than [`MODE_REPLAY_TICKS`] when the match ended).
    pub ticks: u64,
    /// Whether the match ended inside the replay.
    pub ended: bool,
    /// Events seen (kills, damage, cues, ...).
    pub events: usize,
    /// Kills seen.
    pub kills: usize,
}

pub(crate) type Arena = ShelledTerrain<PaintedTerrain<BoxRoomTerrain>>;

pub(crate) fn arena() -> Arena {
    let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(64.0, 32.0, 64.0)), 2.0);
    t.apply_edit(&TerrainEdit::AddSphere {
        center: VEIN,
        radius: KNOT,
    });
    ShelledTerrain::new(t)
}

pub(crate) fn setup() -> MatchSetup {
    // Eight spawns on a ring, teams 0-3 in turn (two per team; free-for-all
    // and two-team modes use them all or their own).
    let spawns = (0..8)
        .map(|i| {
            let a = i as f32 * 0.785;
            SpawnPoint {
                pos: Vec3::new(
                    14.0 * ls_math::det::cos(a),
                    if i % 2 == 0 { -4.0 } else { 4.0 },
                    14.0 * ls_math::det::sin(a),
                ),
                team: (i % 4) as u8,
                orientation: Quat::from_axis_angle(Vec3::Y, a),
            }
        })
        .collect();
    MatchSetup {
        spawns,
        pickup_points: (0..8)
            .map(|i| Vec3::new(-14.0 + 4.0 * i as f32, -3.0, 0.0))
            .collect(),
        instability: 0,
        heist: Some(HeistLevel {
            core: VEIN,
            knot_radius: KNOT,
            chamber_radius: 9.0,
            extraction: [
                (Vec3::new(-24.0, 8.0, 0.0), 6.0),
                (Vec3::new(24.0, 8.0, 0.0), 6.0),
            ],
        }),
        ..Default::default()
    }
}

/// The rule set of a replay: the built-in file with the slow mode timers
/// shortened.
fn replay_rules(name: &str, text: &str) -> RuleSet {
    let mut r = RuleSet::from_ron(name, text).expect("built-in rule set is valid RON");
    for (field, value) in [
        // The Squeeze: every phase closes inside the replay.
        ("first_phase_secs", 3.0),
        ("warn_secs", 2.0),
        ("close_secs", 3.0),
        // Last Color Standing: the stalemate drain starts at 10 s.
        ("stalemate_secs", 10.0),
        ("stalemate_drain_secs", 0.25),
        // Hot Reactor: a short fuse, so it detonates more than once.
        ("fuse_secs", 6.0),
    ] {
        r.mode.set_field(field, value);
    }
    r
}

/// Scripted inputs: pseudo-random buttons and thrust, aiming at the nearest
/// enemy. Depends only on the world state and the RNG.
pub(crate) fn inputs(
    rng: &mut Pcg32,
    w: &World,
    ids: &[PlayerId],
    tick: u64,
) -> Vec<(PlayerId, InputCmd)> {
    ids.iter()
        .map(|&id| {
            let (pitch, yaw) = aim(w, id);
            let mut buttons = Buttons::HORIZON_ASSIST;
            for (b, p) in [
                (Buttons::BOOST, 0.3),
                (Buttons::FIRE_PRIMARY, 0.45),
                (Buttons::FIRE_SECONDARY, 0.06),
                (Buttons::SCAVENGE, 0.2),
                (Buttons::OVERDRIVE, 0.1),
                (Buttons::GADGET, 0.03),
            ] {
                buttons.set(b, rng.chance(p));
            }
            buttons.set(Buttons::WEAPON_MODE, (tick + 17 * id.0 as u64) % 97 == 0);
            let axis = |r: &mut Pcg32| (r.below(3) as f32) - 1.0;
            let thrust = Vec3::new(axis(rng), axis(rng), axis(rng));
            (
                id,
                InputCmd {
                    tick,
                    thrust,
                    look_pitch: pitch + rng.range_f32(-0.02, 0.02),
                    look_yaw: yaw + rng.range_f32(-0.02, 0.02),
                    roll: rng.range_f32(-1.0, 1.0),
                    buttons,
                    weapon_select: if rng.chance(0.03) {
                        rng.below(w.weapons().len() as u32 + 1) as u8
                    } else {
                        0
                    },
                },
            )
        })
        .collect()
}

fn aim(w: &World, id: PlayerId) -> (f32, f32) {
    let Some(me) = w.ship(id) else {
        return (0.0, 0.0);
    };
    let target = w
        .ships()
        .filter(|s| s.alive && s.team != me.team)
        .min_by(|a, b| {
            a.pos
                .distance_squared(me.pos)
                .total_cmp(&b.pos.distance_squared(me.pos))
        });
    let Some(target) = target else {
        return (0.0, 0.0);
    };
    let local = me.orientation.conjugate().rotate(target.pos - me.pos);
    let pitch = ls_math::det::atan2(local.y, -local.z).clamp(-0.1, 0.1);
    let yaw = ls_math::det::atan2(-local.x, -local.z).clamp(-0.1, 0.1);
    (pitch, yaw)
}

/// Plays the replay of the built-in rule set `file` (see the module docs).
/// `None` for a file that is not built in.
pub fn mode_replay(file: &str) -> Option<ModeReplay> {
    let &(file, text) = BUILTIN_RULESETS.iter().find(|(f, _)| *f == file)?;
    let mut cfg = WorldConfig::paint_the_void(MODE_REPLAY_SEED, setup());
    cfg.rules = replay_rules(file, text);
    let mut w = World::new(cfg).expect("built-in rule sets build a world");
    let mut t = arena();
    let mut ids = Vec::new();
    for ty in ShipType::ALL.iter().cycle().take(4 * ShipType::ALL.len()) {
        if ids.len() == MODE_REPLAY_SHIPS {
            break;
        }
        if let Ok(id) = w.add_player(None, *ty) {
            ids.push(id);
        }
    }
    let mut rng = Pcg32::new(MODE_REPLAY_SEED, 13);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let (mut events, mut kills, mut ticks) = (0usize, 0usize, 0u64);
    for tick in 1..=MODE_REPLAY_TICKS {
        let input = inputs(&mut rng, &w, &ids, tick);
        for e in w.step(&mut t, &input) {
            events += 1;
            if matches!(e, Event::Killed { .. }) {
                kills += 1;
            }
        }
        ticks = tick;
        if tick % 60 == 0 {
            hash = fold(hash, w.state_hash());
        }
        if !w.mode().is_running() {
            break;
        }
    }
    let hash = fold(hash, w.state_hash());
    Some(ModeReplay {
        file,
        hash,
        ticks,
        ended: !w.mode().is_running(),
        events,
        kills,
    })
}

/// [`mode_replay`] of every built-in rule set, in [`BUILTIN_RULESETS`] order.
pub fn mode_replays() -> Vec<ModeReplay> {
    BUILTIN_RULESETS
        .iter()
        .filter_map(|(f, _)| mode_replay(f))
        .collect()
}

/// Fixture lines (`<file> <hash as 0x%016x>`) of every built-in rule set:
/// the format of `tests/fixtures/mode_replays.txt` and what `ls-wasm`'s
/// `modeReplayHashes` returns.
pub fn mode_replay_hashes() -> Vec<String> {
    mode_replays()
        .iter()
        .map(|r| format!("{} {:#018x}", r.file, r.hash))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const FIXTURE: &str = include_str!("../tests/fixtures/mode_replays.txt");

    fn fixture() -> Vec<String> {
        FIXTURE
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect()
    }

    /// Every built-in rule set's replay matches the fixture (native and
    /// wasm32). `LS_BLESS_MODE_REPLAYS=1` rewrites the fixture (native).
    #[test]
    fn mode_replays_match_the_fixture() {
        let replays = mode_replays();
        assert_eq!(replays.len(), BUILTIN_RULESETS.len());
        let got: Vec<String> = replays
            .iter()
            .map(|r| format!("{} {:#018x}", r.file, r.hash))
            .collect();
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var_os("LS_BLESS_MODE_REPLAYS").is_some() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/mode_replays.txt"
            );
            let mut text = String::from(
                "# Per-mode replay hashes (ls_sim::golden): one 40 s scripted 8-ship replay per\n\
                 # built-in rule set. Checked by `cargo test -p ls-sim --lib golden` (native and\n\
                 # wasm32) and by tools/determinism-check.mjs (the WASM build under Node).\n\
                 # Re-bless: LS_BLESS_MODE_REPLAYS=1 cargo test -p ls-sim --lib golden\n",
            );
            for l in &got {
                text.push_str(l);
                text.push('\n');
            }
            std::fs::write(path, text).expect("write the fixture");
            for r in &replays {
                println!("{r:?}");
            }
            return;
        }
        for r in &replays {
            println!(
                "{} {:#018x}: {} ticks, ended {}, {} events, {} kills",
                r.file, r.hash, r.ticks, r.ended, r.events, r.kills
            );
        }
        assert_eq!(
            got,
            fixture(),
            "mode replay hashes changed: re-bless with LS_BLESS_MODE_REPLAYS=1 if deliberate"
        );
    }

    /// The replays exercise the modes: fights in every one, and they are
    /// deterministic.
    #[test]
    fn mode_replays_are_deterministic_and_have_fights() {
        let file = "rulesets/hot_reactor.ron";
        let a = mode_replay(file).unwrap();
        let b = mode_replay(file).unwrap();
        assert_eq!(a, b);
        assert!(mode_replay("rulesets/nope.ron").is_none());
        for r in mode_replays() {
            assert!(r.events > 100, "{r:?}");
            assert!(r.ticks >= 60, "{r:?}");
        }
    }
}
