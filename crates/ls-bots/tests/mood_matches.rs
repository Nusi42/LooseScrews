//! Bots in moods (M4 package C5; `docs/milestone-4-plan.md` §5 C5 and its
//! acceptance criteria):
//!
//! - `absurdity_rotation`: a rotation of three matches (Team Brawl on a
//!   monochrome level with pigment, Heist, Paint the Void) on worlds with
//!   Absurdity 100, Gravity Chaos 60 and Instability 50, 8 bots; all 18
//!   comedy events are forced (`World::schedule_event`, six per match, back
//!   to back), so every event runs at least once with bots in it. Checks:
//!   every match reaches a result, 0 desyncs, 0 protocol errors, every
//!   event ran, the bots reacted (shot drones and mimics, dodged ducks and
//!   the paw, collected hats, took pigment, countered wells), and the
//!   largest reconciliation error near wells and ducks is at most 0.1 u.
//! - `architect_match`: The Architect with 6 bots: three rounds, every
//!   round has a bot architect placing strokes, runners reach the exit, 0
//!   desyncs.
//!
//! Needs whole-level generation, so it runs only in release:
//! `cargo test -p ls-bots --release --test mood_matches -- --nocapture`.

mod support;

use std::collections::BTreeMap;

use ls_bots::BotStats;
use ls_game::ls_sim::{EventKind, EventPhase, MatchPhase, ModeKind, RuleSet};
use ls_game::ls_worldgen::{ShareCode, Sliders};
use ls_game::MatchConfig;

use support::Match;

const BOTS: usize = 8;
/// Seconds into a match the first forced event starts.
const FIRST_EVENT_SECS: u64 = 40;

fn rules(kind: ModeKind, match_secs: f32) -> RuleSet {
    let mut r = RuleSet::default_for(kind);
    r.mode.set_field("match_secs", match_secs);
    if kind == ModeKind::Heist {
        // Played on time, not on deliveries.
        r.mode.set_field("deliveries_to_win", 31.0);
    }
    RuleSet::from_code(&r.to_code().unwrap()).unwrap()
}

fn moody(seed: u64, palette: u8) -> ShareCode {
    ShareCode::new(
        seed,
        Sliders {
            absurdity: 100,
            gravity_chaos: 60,
            instability: 50,
            palette,
            ..Sliders::new(40, 30)
        },
    )
}

/// What one match showed.
struct MoodRun {
    name: &'static str,
    ended_at: u64,
    bots: Vec<(u8, BotStats)>,
    /// Events seen active (with the tick they became active).
    events: BTreeMap<EventKind, u64>,
    winner: Option<u8>,
}

fn play_moods(
    name: &'static str,
    kind: ModeKind,
    code: ShareCode,
    events: &[EventKind],
    seed_base: u64,
) -> MoodRun {
    let match_secs = FIRST_EVENT_SECS as f32 + 50.0 * events.len() as f32 + 20.0;
    let mc = MatchConfig {
        rules: rules(kind, match_secs),
        ..MatchConfig::paint_the_void(code)
    };
    let mut m = Match::new(mc, BOTS, seed_base);
    let start = m.room.world().tick();
    for (i, e) in events.iter().enumerate() {
        let at = start + (FIRST_EVENT_SECS + 2 * i as u64) * 60;
        m.room.world_mut().schedule_event(at, *e);
    }
    let mut seen = BTreeMap::new();
    let mut ended_at = None;
    while ended_at.is_none_or(|e| m.now < e + 180) {
        assert!(
            m.now < (match_secs as u64 + 120) * 60,
            "{name}: no result after {} ticks",
            m.now
        );
        m.step();
        let v = m.room.world().mood_view();
        if let Some(e) = v.event {
            if v.phase == EventPhase::Active {
                seen.entry(e).or_insert(m.now);
            }
        }
        if ended_at.is_none() && !m.room.world().mode().is_running() {
            ended_at = Some(m.now);
        }
    }
    let bots = m.finish();
    let MatchPhase::Ended { winner } = m.room.world().mode().phase else {
        panic!("{name}: no result");
    };
    for (_, s) in &bots {
        assert_eq!(s.joins, 1, "{name} {}", s.name);
        assert_eq!(s.protocol_errors, 0, "{name} {}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{name} {}: {:?}", s.name, s.errors);
        assert_eq!(s.matches_over, 1, "{name} {}: saw the end", s.name);
    }
    MoodRun {
        name,
        ended_at: ended_at.unwrap(),
        bots,
        events: seen,
        winner,
    }
}

fn report(run: &MoodRun) {
    let sum = |f: fn(&BotStats) -> u64| run.bots.iter().map(|(_, s)| f(s)).sum::<u64>();
    let max_err = |f: fn(&BotStats) -> f32| run.bots.iter().map(|(_, s)| f(s)).fold(0.0, f32::max);
    eprintln!(
        "{}: ended at {:.0} s, winner {:?}; events {:?}",
        run.name,
        run.ended_at as f32 / 60.0,
        run.winner,
        run.events.keys().map(|e| e.id()).collect::<Vec<_>>()
    );
    eprintln!(
        "  wells {} s, prop shots {}, duck dodges {} s, paw flight {} s, cave-in flight {} s, \
         pizza holds {} s, hats {} s, pigment {} s; kills {}, deaths {}",
        sum(|s| s.behavior.moods.well_ticks) / 60,
        sum(|s| s.behavior.moods.prop_shots),
        sum(|s| s.behavior.moods.duck_dodge_ticks) / 60,
        sum(|s| s.behavior.moods.paw_flee_ticks) / 60,
        sum(|s| s.behavior.moods.cave_in_flee_ticks) / 60,
        sum(|s| s.behavior.moods.pizza_hold_ticks) / 60,
        sum(|s| s.behavior.moods.hat_ticks) / 60,
        sum(|s| s.behavior.moods.pigment_ticks) / 60,
        run.bots.iter().map(|(_, s)| s.kills).sum::<u32>(),
        run.bots.iter().map(|(_, s)| s.deaths).sum::<u32>(),
    );
    eprintln!(
        "  reconciliation: largest {:.4} u, near wells and ducks {:.4} u ({} corrections there)",
        max_err(|s| s.max_reconcile_error),
        max_err(|s| s.max_reconcile_error_moods),
        sum(|s| s.mood_corrections),
    );
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn absurdity_rotation() {
    use EventKind::*;
    let rotation: [(&str, ModeKind, ShareCode, [EventKind; 6]); 3] = [
        (
            "team_brawl (monochrome)",
            ModeKind::TeamBrawl,
            moody(0x00c5_0101, 90),
            [
                TaxDrone,
                DuckParade,
                GravityCoffeeBreak,
                HatRain,
                CatPaw,
                PizzaDelivery,
            ],
        ),
        (
            "heist",
            ModeKind::Heist,
            moody(0x00c5_0102, 30),
            [
                MimicPickups,
                CustomerSurvey,
                RubberChickens,
                BigHeadDay,
                GossipWall,
                LevelHiccups,
            ],
        ),
        (
            "paint_the_void",
            ModeKind::PaintTheVoid,
            moody(0x00c5_0103, 40),
            [
                DiscoInferno,
                EverythingIsCake,
                MovingDay,
                TiredAnnouncer,
                WrongGenre,
                BoredLevel,
            ],
        ),
    ];
    let mut all = BTreeMap::new();
    let mut runs = Vec::new();
    for (i, (name, kind, code, events)) in rotation.into_iter().enumerate() {
        let run = play_moods(name, kind, code, &events, 900 + 10 * i as u64);
        report(&run);
        for e in &events {
            assert!(run.events.contains_key(e), "{name}: {} ran", e.id());
        }
        all.extend(run.events.clone());
        runs.push(run);
    }
    assert_eq!(all.len(), EventKind::ALL.len(), "every event ran");
    let sum = |f: fn(&BotStats) -> u64| {
        runs.iter()
            .flat_map(|r| r.bots.iter())
            .map(|(_, s)| f(s))
            .sum::<u64>()
    };
    assert!(sum(|s| s.behavior.moods.well_ticks) > 0, "bots met wells");
    assert!(
        sum(|s| s.behavior.moods.prop_shots) > 0,
        "bots shot drones and mimics"
    );
    assert!(
        sum(|s| s.behavior.moods.duck_dodge_ticks) > 0,
        "bots dodged ducks"
    );
    assert!(
        sum(|s| s.behavior.moods.paw_flee_ticks) > 0,
        "bots fled the paw"
    );
    assert!(
        sum(|s| s.behavior.moods.hat_ticks) > 0,
        "bots went for hats"
    );
    assert!(
        sum(|s| s.behavior.moods.pigment_ticks) > 0,
        "bots took pigment"
    );
    let near = runs
        .iter()
        .flat_map(|r| r.bots.iter())
        .map(|(_, s)| s.max_reconcile_error_moods)
        .fold(0.0f32, f32::max);
    assert!(
        near <= 0.1,
        "reconciliation error near wells and ducks: {near:.4} u"
    );
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn architect_match() {
    let code = ShareCode::new(0x00c5_0201, Sliders::new(40, 30));
    let rules = RuleSet::default_for(ModeKind::TheArchitect);
    let rules = RuleSet::from_code(&rules.to_code().unwrap()).unwrap();
    let mc = MatchConfig {
        rules,
        ..MatchConfig::paint_the_void(code)
    };
    let mut m = Match::new(mc, 6, 1300);
    m.join_all();
    let mut architects = Vec::new();
    let mut finished = 0usize;
    let mut ended_at = None;
    while ended_at.is_none_or(|e| m.now < e + 180) {
        assert!(m.now < 16 * 60 * 60, "no result after {} ticks", m.now);
        m.step();
        let world = m.room.world();
        if let Some(a) = world.mode().architect.as_ref() {
            if let Some(id) = a.architect {
                if architects.last() != Some(&(a.round, id)) {
                    architects.push((a.round, id));
                }
            }
            finished = finished.max(a.history.iter().map(|h| h.finished as usize).sum());
        }
        if ended_at.is_none() && !world.mode().is_running() {
            ended_at = Some(m.now);
        }
    }
    let bots = m.finish();
    let world = m.room.world();
    let MatchPhase::Ended { winner } = world.mode().phase else {
        panic!("no result");
    };
    let stats = m.room.stats();
    let sum = |f: fn(&BotStats) -> u64| bots.iter().map(|(_, s)| f(s)).sum::<u64>();
    let rounds = world
        .mode()
        .architect
        .as_ref()
        .map_or(0, |a| a.history.len());
    eprintln!(
        "the_architect: ended at {:.0} s, winner {winner:?}, {rounds} rounds, architects {architects:?}; \
         strokes asked {}, accepted {}, refused {}; runners out {finished}; runner ticks {} s, digs {}",
        ended_at.unwrap() as f32 / 60.0,
        sum(|s| s.behavior.moods.architect_strokes),
        stats.architect_strokes,
        stats.rejected_architect_actions,
        sum(|s| s.behavior.moods.runner_ticks) / 60,
        sum(|s| s.behavior.moods.runner_dig_shots),
    );
    for r in &world.results().rows {
        eprintln!("  {:?}: score {}", r.player, r.score);
    }
    for (_, s) in &bots {
        assert_eq!(s.protocol_errors, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.matches_over, 1, "{} saw the end", s.name);
    }
    assert_eq!(rounds, 3, "three rounds");
    let distinct: std::collections::BTreeSet<_> = architects.iter().map(|(_, id)| *id).collect();
    assert_eq!(
        distinct.len(),
        3,
        "a new architect each round: {architects:?}"
    );
    assert!(
        stats.architect_strokes >= 3,
        "the architect bots placed strokes"
    );
    assert!(finished > 0, "runners reached the exit");
}
