//! The Architect's replay hash (M4 package B4) for the native-vs-WASM
//! determinism cross-check: a 40 s scripted match of 6 ships in a test
//! arena that answers reshape strokes analytically
//! ([`crate::terrain::AnalyticReshape`]), with short rounds (10 s) so all
//! three rounds and the match end happen inside it. Every architect places
//! a scripted stroke every 1.25 s, cycling through all eight kinds at
//! seeded spots (some are refused: cooldown, budget, range); half the
//! runners fly at the exit, the others play the golden inputs
//! ([`crate::golden`]).
//!
//! The hash is the line `mode/the_architect` of
//! `crates/ls-sim/tests/fixtures/mood_replays.txt`, checked on native and
//! wasm32 by the module tests. Re-bless after a deliberate change with
//! `LS_BLESS_MOOD_REPLAYS=1 cargo test -p ls-sim --lib the_architect`
//! (rewrites only that line).

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
use ls_math::{Pcg32, Quat, Vec3};

use super::StrokeKind;
use crate::modes::heist::HeistLevel;
use crate::ruleset::ModeRules;
use crate::terrain::AnalyticReshape;
use crate::{
    Buttons, Event, InputCmd, MatchSetup, PaintedTerrain, PlayerId, RuleSet, ShipType, SimTerrain,
    SpawnPoint, World, WorldConfig,
};

/// Fixture name of the replay.
pub const ARCHITECT_REPLAY: &str = "mode/the_architect";
/// Ticks of the replay (40 s).
pub const ARCHITECT_REPLAY_TICKS: u64 = 40 * 60;
/// Ships of the replay.
pub const ARCHITECT_REPLAY_SHIPS: usize = 6;
/// Seed of the replay.
pub const ARCHITECT_REPLAY_SEED: u64 = 2026;

/// The test arena of The Architect: a 160 x 48 x 64 u room (x from -80 to
/// 80) with rock knots in the middle third and a rock shelf under the
/// ceiling, answering reshape strokes analytically.
pub type ArchitectArena = AnalyticReshape<PaintedTerrain<BoxRoomTerrain>>;

/// Builds the [`ArchitectArena`].
pub fn arena() -> ArchitectArena {
    let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(160.0, 48.0, 64.0)), 2.0);
    for (c, r) in [
        (Vec3::new(-10.0, 0.0, 20.0), 9.0),
        (Vec3::new(14.0, -8.0, 14.0), 7.0),
        (Vec3::new(20.0, 10.0, -12.0), 8.0),
        (Vec3::new(0.0, 22.0, 0.0), 10.0),
    ] {
        t.apply_edit(&TerrainEdit::AddSphere {
            center: c,
            radius: r,
        });
    }
    AnalyticReshape::new(t)
}

/// The arena's setup: six team 0 spawn points at the low x end, the exit
/// (the Heist site's team 1 extraction zone) at the high x end, pickup
/// points along the way.
pub fn setup() -> MatchSetup {
    let spawns = (0..6)
        .map(|i| SpawnPoint {
            pos: Vec3::new(
                -66.0,
                -12.0 + 8.0 * (i / 2) as f32,
                -8.0 + 16.0 * (i % 2) as f32,
            ),
            team: 0,
            orientation: Quat::from_axis_angle(Vec3::Y, -std::f32::consts::FRAC_PI_2),
        })
        .collect();
    MatchSetup {
        spawns,
        pickup_points: (0..6)
            .map(|i| Vec3::new(-40.0 + 16.0 * i as f32, -14.0, 20.0))
            .collect(),
        heist: Some(HeistLevel {
            core: Vec3::new(0.0, -20.0, -24.0),
            knot_radius: 0.0,
            chamber_radius: 0.0,
            extraction: [
                (Vec3::new(-66.0, 0.0, 0.0), 6.0),
                (Vec3::new(66.0, 0.0, 0.0), 6.0),
            ],
        }),
        ..Default::default()
    }
}

/// The built-in rule set with `round_secs` rounds.
pub fn rules(round_secs: f32) -> RuleSet {
    let mut r = RuleSet::default_for(crate::ModeKind::TheArchitect);
    r.mode.set_field("round_secs", round_secs);
    r.mode.set_field("round_results_secs", 2.0);
    debug_assert!(matches!(r.mode, ModeRules::TheArchitect(_)));
    r
}

/// Outcome of [`architect_replay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectReplay {
    /// Final state hash folded with the state hash of every second.
    pub hash: u64,
    pub ticks: u64,
    pub ended: bool,
    pub events: usize,
    /// Strokes accepted and refused.
    pub strokes: usize,
    pub refused: usize,
    /// Stroke edits applied (terrain edit events while strokes acted).
    pub edits: usize,
    /// Runners through the exit.
    pub finished: usize,
    pub rounds: usize,
}

/// Steering at `target`: boosted thrust toward it while the nose turns to it.
pub fn steer(w: &World, id: PlayerId, target: Vec3, tick: u64) -> InputCmd {
    let Some(me) = w.ship(id) else {
        return InputCmd::idle(tick);
    };
    let local = me.orientation.conjugate().rotate(target - me.pos);
    let pitch = ls_math::det::atan2(local.y, -local.z).clamp(-0.08, 0.08);
    let yaw = ls_math::det::atan2(-local.x, -local.z).clamp(-0.08, 0.08);
    let dir = local.normalize_or_zero();
    InputCmd {
        tick,
        thrust: dir,
        look_pitch: pitch,
        look_yaw: yaw,
        roll: 0.0,
        buttons: Buttons::HORIZON_ASSIST | Buttons::BOOST,
        weapon_select: 0,
    }
}

/// Plays the replay (see the module docs).
pub fn architect_replay() -> ArchitectReplay {
    let mut cfg = WorldConfig::paint_the_void(ARCHITECT_REPLAY_SEED, setup());
    cfg.rules = rules(10.0);
    let mut w = World::new(cfg).expect("the Architect's rule set builds a world");
    let mut t = arena();
    let ids: Vec<PlayerId> = ShipType::ALL
        .iter()
        .cycle()
        .take(ARCHITECT_REPLAY_SHIPS)
        .map(|ty| w.add_player(None, *ty).expect("room for the ships"))
        .collect();
    let mut rng = Pcg32::new(ARCHITECT_REPLAY_SEED, 21);
    let mut spots = Pcg32::new(ARCHITECT_REPLAY_SEED, 22);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: u64, v: u64| (h ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    let mut out = ArchitectReplay {
        hash: 0,
        ticks: 0,
        ended: false,
        events: 0,
        strokes: 0,
        refused: 0,
        edits: 0,
        finished: 0,
        rounds: 0,
    };
    let exit = super::exit_of(&setup()).0;
    let mut k = 0u8;
    for tick in 1..=ARCHITECT_REPLAY_TICKS {
        // The architect's script: a stroke every 1.25 s.
        if tick % 75 == 0 {
            let architect = w.mode().architect.as_ref().and_then(|a| a.architect);
            if let Some(a) = architect {
                let kind = StrokeKind::ALL[k as usize % StrokeKind::ALL.len()];
                k = k.wrapping_add(1);
                let center = Vec3::new(
                    spots.range_f32(-30.0, 45.0),
                    spots.range_f32(-14.0, 14.0),
                    spots.range_f32(-20.0, 20.0),
                );
                let radius = spots.range_f32(12.0, 20.0);
                match w.architect_action(&t, a, kind.index(), center, radius) {
                    Ok(_) => out.strokes += 1,
                    Err(_) => out.refused += 1,
                }
            }
        }
        let golden = crate::golden::inputs(&mut rng, &w, &ids, tick);
        let input: Vec<(PlayerId, InputCmd)> = golden
            .into_iter()
            .map(|(id, cmd)| {
                if id.0 % 2 == 1 {
                    (id, steer(&w, id, exit, tick))
                } else {
                    (id, cmd)
                }
            })
            .collect();
        let finished_before = w.mode().architect.as_ref().map_or(0, |a| a.finished.len());
        let striking = w
            .mode()
            .architect
            .as_ref()
            .is_some_and(|a| a.strokes.iter().any(|s| s.active && !s.edits.is_empty()));
        for e in w.step(&mut t, &input) {
            out.events += 1;
            if striking && matches!(e, Event::TerrainEdit(_)) {
                out.edits += 1;
            }
        }
        if let Some(a) = w.mode().architect.as_ref() {
            out.finished += a.finished.len().saturating_sub(finished_before);
            out.rounds = a.history.len();
        }
        out.ticks = tick;
        if tick % 60 == 0 {
            hash = fold(hash, w.state_hash());
        }
        if !w.mode().is_running() {
            break;
        }
    }
    out.hash = fold(hash, w.state_hash());
    out.ended = !w.mode().is_running();
    out
}
