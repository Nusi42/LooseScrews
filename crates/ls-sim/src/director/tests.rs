//! Comedy director tests: inertness at Absurdity 0, cadence, statistics over
//! simulated matches, moods, vetoes, bans, forced events, determinism.

use ls_math::terrain::BoxRoomTerrain;
use ls_math::{Quat, Tick, Vec3};

use super::*;
use crate::events::{EventKind, EventPhase};
use crate::mode::ModeKind;
use crate::{
    Event, MatchSetup, Mutator, PlayerId, RuleSet, ShipType, SpawnPoint, World, WorldConfig,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const MATCH_SECS: f32 = 300.0;

fn setup(absurdity: u8) -> MatchSetup {
    MatchSetup {
        spawns: vec![
            SpawnPoint {
                pos: Vec3::new(-20.0, 0.0, 0.0),
                team: 0,
                orientation: Quat::IDENTITY,
            },
            SpawnPoint {
                pos: Vec3::new(20.0, 0.0, 0.0),
                team: 1,
                orientation: Quat::IDENTITY,
            },
        ],
        // Enough pickup points for Moving day and Mimic pickups.
        pickup_points: (0..12)
            .map(|i| Vec3::new(-33.0 + 6.0 * i as f32, 5.0, 10.0))
            .collect(),
        instability: 0,
        absurdity,
        heist: None,
    }
}

fn rules(kind: ModeKind, cap: u8) -> RuleSet {
    let mut r = RuleSet::default_for(kind);
    r.mode.set_field("match_secs", MATCH_SECS);
    r.absurdity = cap;
    r
}

fn world(seed: u64, absurdity: u8, rules: RuleSet, ships: usize) -> World {
    let teams = rules.mode.teams();
    let mut cfg = WorldConfig::paint_the_void(seed, setup(absurdity));
    cfg.rules = rules;
    let mut w = World::new(cfg).unwrap();
    for i in 0..ships {
        let team = (teams > 0).then_some((i % 2) as u8);
        w.add_player(team, ShipType::Tinker).unwrap();
    }
    w
}

fn terrain() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(80.0, 40.0, 80.0))
}

/// Director events of a whole match with their ticks.
fn run_match(w: &mut World) -> Vec<(u64, DirectorEvent)> {
    let mut t = terrain();
    let mut out = Vec::new();
    while w.mode().is_running() {
        for e in w.step(&mut t, &[]) {
            if let Event::Director(d) = e {
                out.push((w.tick(), d));
            }
        }
    }
    out
}

fn announced(evs: &[(u64, DirectorEvent)]) -> Vec<(u64, EventKind)> {
    evs.iter()
        .filter_map(|(t, e)| match e {
            DirectorEvent::Announced { kind, .. } => Some((*t, *kind)),
            _ => None,
        })
        .collect()
}

fn secs(s: f32) -> u64 {
    Tick::ticks_for_secs(s)
}

fn candidates(w: &mut World) -> Vec<(EventKind, f32)> {
    let d = w.director().clone();
    w.director_candidates_for_test(&mut terrain(), &d, None)
}

#[test]
fn builtin_data_loads() {
    let d = DirectorDef::builtin();
    assert_eq!(d.announce_ticks(), 180);
    assert_eq!(d.chance(100), 0.6);
    assert_eq!(d.chance(50), 0.3);
    assert_eq!(d.chance(0), 0.0);
    assert_eq!(DirectorDef::builtin_files().len(), 19);
}

#[test]
fn off_at_absurdity_zero_or_cap_zero() {
    for (level, cap) in [(0u8, 100u8), (100, 0), (0, 0)] {
        let mut w = world(7, level, rules(ModeKind::PaintTheVoid, cap), 2);
        assert_eq!(w.effective_absurdity(), level.min(cap));
        let evs = run_match(&mut w);
        assert!(evs.is_empty(), "level {level} cap {cap}: {evs:?}");
        assert!(w.director().is_pristine());
        assert_eq!(w.mood_view(), MoodView::default());
    }
}

/// The state hash of a world whose director stays off is the hash without
/// a director (the M3 replays keep their goldens): the pristine director
/// adds nothing, whatever the rule set's cap.
#[test]
fn inert_director_leaves_the_hash_alone() {
    let mut a = world(3, 0, rules(ModeKind::PaintTheVoid, 100), 2);
    let mut b = world(3, 0, rules(ModeKind::PaintTheVoid, 0), 2);
    let mut t = terrain();
    for _ in 0..600 {
        a.step(&mut t, &[]);
        b.step(&mut t, &[]);
    }
    assert_eq!(a.state_hash(), b.state_hash());
    // A director that did something is hashed.
    let mut c = world(3, 0, rules(ModeKind::PaintTheVoid, 0), 2);
    c.schedule_event(10_000, EventKind::HatRain);
    let mut t = terrain();
    for _ in 0..600 {
        c.step(&mut t, &[]);
    }
    assert_ne!(a.state_hash(), c.state_hash());
}

#[test]
fn cadence_rules_hold() {
    let d = DirectorDef::builtin();
    let table = crate::events::EventTable::builtin();
    let mut total = 0;
    for seed in 0..12u64 {
        let mut w = world(seed, 100, rules(ModeKind::PaintTheVoid, 100), 2);
        let evs = run_match(&mut w);
        let end = secs(MATCH_SECS);
        let mut last_end: Option<u64> = None;
        let mut running: Option<(EventKind, u64)> = None;
        let mut seen = Vec::new();
        for (t, e) in &evs {
            match *e {
                DirectorEvent::Announced { kind, forced, .. } => {
                    assert!(!forced);
                    assert!(running.is_none(), "one event at a time");
                    assert!(*t >= secs(d.quiet_start_secs), "quiet start: {t}");
                    assert!(
                        *t + d.announce_ticks() + secs(d.quiet_end_secs) <= end,
                        "quiet end: {t}"
                    );
                    if let Some(e) = last_end {
                        assert!(*t >= e + secs(d.gap_secs), "gap: {e} -> {t}");
                    }
                    assert!(!seen.contains(&kind), "no repeats: {kind:?}");
                    seen.push(kind);
                    running = Some((kind, *t));
                    total += 1;
                }
                DirectorEvent::Started { kind } => {
                    let (k, at) = running.unwrap();
                    assert_eq!(k, kind);
                    assert_eq!(*t, at + d.announce_ticks(), "3 s telegraph");
                }
                DirectorEvent::Ended { kind } => {
                    let (k, at) = running.take().unwrap();
                    assert_eq!(k, kind);
                    let def = table.get(kind);
                    if *t < end {
                        assert_eq!(
                            *t,
                            at + d.announce_ticks() + def.active_ticks() + def.ending_ticks(),
                            "{kind:?} runs for its duration"
                        );
                    }
                    last_end = Some(*t);
                }
                DirectorEvent::MoodChanged { .. } => {}
            }
        }
        // The match ended: nothing left running.
        assert!(w.director().current.is_none());
    }
    assert!(total >= 24, "{total} events in 12 matches at Absurdity 100");
}

#[test]
fn moods_drift_every_60_to_90_seconds() {
    let mut w = world(11, 60, rules(ModeKind::PaintTheVoid, 100), 0);
    let evs = run_match(&mut w);
    let changes: Vec<(u64, Mood)> = evs
        .iter()
        .filter_map(|(t, e)| match e {
            DirectorEvent::MoodChanged { mood } => Some((*t, *mood)),
            _ => None,
        })
        .collect();
    // 300 s: the first mood at tick 1 (announced unless calm), then a
    // change every 60-90 s.
    assert!(changes.len() >= 3, "{changes:?}");
    for pair in changes.windows(2) {
        let dt = pair[1].0 - pair[0].0;
        assert!((secs(60.0)..=secs(90.0)).contains(&dt), "{changes:?}");
        assert_ne!(pair[0].1, pair[1].1);
    }
    let all: std::collections::BTreeSet<Mood> = (0..40u64)
        .map(|seed| {
            let mut w = world(seed, 50, rules(ModeKind::PaintTheVoid, 100), 0);
            w.step(&mut terrain(), &[]);
            w.mood_view().mood
        })
        .collect();
    assert_eq!(all.len(), 4, "every mood occurs: {all:?}");
}

#[test]
fn mood_doubles_the_weight_of_its_events() {
    let mut w = world(5, 100, rules(ModeKind::PaintTheVoid, 100), 2);
    let mut t = terrain();
    w.step(&mut t, &[]);
    for mood in Mood::ALL {
        let d = DirectorState {
            mood,
            ..DirectorState::default()
        };
        let c = w.director_candidates_for_test(&mut t, &d, None);
        assert!(c.len() >= 10, "{c:?}");
        for (kind, weight) in c {
            let def = kind.def();
            let want = if def.moods.contains(&mood) { 2.0 } else { 1.0 };
            assert_eq!(weight, want * def.weight, "{mood:?} {kind:?}");
        }
    }
    // Played events are left out while unplayed ones are eligible.
    let all = w.director_candidates_for_test(&mut t, &DirectorState::default(), None);
    let played = DirectorState {
        played: 1 << EventKind::HatRain.index(),
        ..DirectorState::default()
    };
    let c = w.director_candidates_for_test(&mut t, &played, None);
    assert_eq!(c.len() + 1, all.len());
    assert!(!c.iter().any(|(k, _)| *k == EventKind::HatRain));
    let everything = DirectorState {
        played: u32::MAX,
        ..DirectorState::default()
    };
    assert_eq!(
        w.director_candidates_for_test(&mut t, &everything, None)
            .len(),
        all.len()
    );
}

#[test]
fn bans_and_vetoes_are_respected() {
    // Rule set bans: never rolled.
    let mut r = rules(ModeKind::PaintTheVoid, 100);
    let keep = [EventKind::HatRain, EventKind::DiscoInferno];
    for k in EventKind::ALL {
        if !keep.contains(&k) {
            r.ban_event(k.id(), true);
        }
    }
    let mut seen = Vec::new();
    for seed in 0..6 {
        let mut w = world(seed, 100, r.clone(), 2);
        seen.extend(announced(&run_match(&mut w)).into_iter().map(|(_, k)| k));
    }
    assert!(seen.len() >= 6, "{seen:?}");
    assert!(seen.iter().all(|k| keep.contains(k)), "{seen:?}");

    // Instagib vetoes Tax drone and Pizza delivery.
    let mut r = rules(ModeKind::TeamBrawl, 100);
    r.mutators = vec![Mutator::Instagib];
    let mut w = world(1, 100, r, 2);
    let mut t = terrain();
    w.step(&mut t, &[]);
    w.ship_mut(PlayerId(0)).unwrap().hull -= 1.0;
    let c = candidates(&mut w);
    assert!(!c.is_empty());
    for (k, _) in c {
        assert!(!matches!(k, EventKind::TaxDrone | EventKind::PizzaDelivery));
    }
    let m = crate::mutators::module(Mutator::Instagib);
    assert!(!m.allows_event(EventKind::TaxDrone) && !m.allows_event(EventKind::PizzaDelivery));
    assert!(m.allows_event(EventKind::DuckParade));

    // The Squeeze vetoes Cat paw and Duck parade from phase 4 on.
    let sq = crate::modes::module(ModeKind::TheSqueeze);
    let mut state = crate::ModeState::new(ModeKind::TheSqueeze, 0, 1000);
    let rules_sq = rules(ModeKind::TheSqueeze, 100);
    let ships = std::collections::BTreeMap::new();
    for stage in 0..=6u8 {
        state.stage = stage;
        let m = crate::modes::ModeRef {
            state: &state,
            rules: &rules_sq.mode,
            mutators: &[],
            ships: &ships,
            now: 0,
        };
        for k in [EventKind::CatPaw, EventKind::DuckParade] {
            assert_eq!(sq.allows_event(&m, k), stage < 4, "stage {stage} {k:?}");
        }
        assert!(sq.allows_event(&m, EventKind::HatRain));
    }
}

#[test]
fn disabled_events_are_not_rolled() {
    // `enabled: false` in the data: the candidate list leaves it out (the
    // built-in data enables every event; a table copy disables one).
    let mut w = world(2, 100, rules(ModeKind::PaintTheVoid, 100), 2);
    let mut t = terrain();
    w.step(&mut t, &[]);
    assert!(candidates(&mut w)
        .iter()
        .any(|(k, _)| *k == EventKind::HatRain));
    let files: Vec<(String, String)> = crate::events::MODULES
        .iter()
        .map(|m| {
            let text = if m.kind == EventKind::HatRain {
                m.data.replace("enabled: true", "enabled: false")
            } else {
                m.data.to_string()
            };
            (m.file.to_string(), text)
        })
        .collect();
    let table =
        crate::events::EventTable::from_files(files.iter().map(|(a, b)| (a.as_str(), b.as_str())))
            .unwrap();
    assert!(!table.get(EventKind::HatRain).enabled);
    let d = w.director().clone();
    let c = w.director_candidates_for_test(&mut t, &d, Some(&table));
    assert!(!c.is_empty());
    assert!(!c.iter().any(|(k, _)| *k == EventKind::HatRain));
}

#[test]
fn eligibility_follows_the_game_state() {
    let mut w = world(4, 100, rules(ModeKind::TeamBrawl, 100), 2);
    let mut t = terrain();
    for _ in 0..30 {
        w.step(&mut t, &[]);
    }
    let kinds =
        |w: &mut World| -> Vec<EventKind> { candidates(w).into_iter().map(|(k, _)| k).collect() };
    let c = kinds(&mut w);
    // No leader (nobody scored), nobody damaged, no wells.
    for k in [
        EventKind::TaxDrone,
        EventKind::CustomerSurvey,
        EventKind::PizzaDelivery,
        EventKind::GravityCoffeeBreak,
    ] {
        assert!(!c.contains(&k), "{k:?}");
    }
    // Pickups in play: Moving day and Mimic pickups are eligible.
    assert!(c.contains(&EventKind::MovingDay) && c.contains(&EventKind::MimicPickups));
    // A damaged ship and a leader.
    let p0 = PlayerId(0);
    w.ship_mut(p0).unwrap().hull -= 30.0;
    w.mode_mut().team_scores[0] = 3;
    w.mode_mut().players.get_mut(&p0).unwrap().score = 3;
    let c = kinds(&mut w);
    for k in [
        EventKind::TaxDrone,
        EventKind::CustomerSurvey,
        EventKind::PizzaDelivery,
    ] {
        assert!(c.contains(&k), "{k:?}");
    }
}

#[test]
fn leader_events_name_the_leader_and_pizza_a_damaged_player() {
    let mut w = world(4, 0, rules(ModeKind::TeamBrawl, 100), 4);
    let mut t = terrain();
    w.step(&mut t, &[]);
    // Team 1 (players 1 and 3) leads; player 3 is its best.
    w.mode_mut().team_scores[1] = 5;
    w.mode_mut().players.get_mut(&PlayerId(3)).unwrap().score = 4;
    w.mode_mut().players.get_mut(&PlayerId(1)).unwrap().score = 1;
    w.mode_mut().players.get_mut(&PlayerId(0)).unwrap().score = 9;
    w.ship_mut(PlayerId(2)).unwrap().hull = 10.0;
    let now = w.tick();
    w.schedule_event(now + 1, EventKind::TaxDrone);
    w.step(&mut t, &[]);
    assert_eq!(w.mood_view().event, Some(EventKind::TaxDrone));
    assert_eq!(w.mood_view().target, Some(PlayerId(3)));
    // Pizza after the drone: the only damaged ship.
    w.schedule_event(w.tick() + 1, EventKind::PizzaDelivery);
    while w.mood_view().event != Some(EventKind::PizzaDelivery) {
        w.step(&mut t, &[]);
    }
    assert_eq!(w.mood_view().target, Some(PlayerId(2)));
}

#[test]
fn forced_events_run_whatever_the_absurdity() {
    let mut w = world(9, 0, rules(ModeKind::PaintTheVoid, 0), 2);
    w.schedule_event(100, EventKind::DuckParade);
    w.schedule_event(110, EventKind::HatRain);
    let mut t = terrain();
    let mut seen = Vec::new();
    for _ in 0..secs(120.0) {
        for e in w.step(&mut t, &[]) {
            if let Event::Director(d) = e {
                seen.push((w.tick(), d));
            }
        }
    }
    let ann = announced(&seen);
    assert_eq!(ann[0], (100, EventKind::DuckParade));
    // The second waits until the first ended (no gap for forced events).
    let parade = EventKind::DuckParade.def();
    let end = 100 + 180 + parade.active_ticks();
    assert_eq!(ann[1], (end + 1, EventKind::HatRain));
    assert_eq!(ann.len(), 2);
    assert!(seen
        .iter()
        .all(|(_, e)| !matches!(e, DirectorEvent::Announced { forced: false, .. })));
    // Phases in the view.
    let mut w = world(9, 0, rules(ModeKind::PaintTheVoid, 0), 0);
    w.schedule_event(5, EventKind::BigHeadDay);
    let mut t = terrain();
    for _ in 0..5 {
        w.step(&mut t, &[]);
    }
    let v = w.mood_view();
    assert_eq!(
        (v.event, v.phase, v.phase_end_tick, v.active_tick),
        (Some(EventKind::BigHeadDay), EventPhase::Announce, 185, 185)
    );
    assert_eq!(w.mode_view().mood, v);
    for _ in 0..180 {
        w.step(&mut t, &[]);
    }
    let v = w.mood_view();
    assert_eq!((v.phase, v.active_tick), (EventPhase::Active, 185));
    assert_eq!(
        v.phase_end_tick,
        185 + EventKind::BigHeadDay.def().active_ticks()
    );
    // An event with an ending phase (Gravity coffee break: 1 s fade).
    let mut w = world(9, 0, rules(ModeKind::PaintTheVoid, 0), 0);
    w.schedule_event(1, EventKind::GravityCoffeeBreak);
    let mut t = terrain();
    let mut phases = Vec::new();
    for _ in 0..secs(30.0) {
        w.step(&mut t, &[]);
        let v = w.mood_view();
        if phases.last() != Some(&(v.event, v.phase)) {
            phases.push((v.event, v.phase));
        }
    }
    let k = Some(EventKind::GravityCoffeeBreak);
    assert_eq!(
        phases,
        vec![
            (k, EventPhase::Announce),
            (k, EventPhase::Active),
            (k, EventPhase::Ending),
            (None, EventPhase::Announce)
        ]
    );
}

#[test]
fn an_event_running_at_the_match_end_ends_with_it() {
    let mut r = rules(ModeKind::PaintTheVoid, 0);
    r.mode.set_field("match_secs", 10.0);
    let mut w = world(9, 0, r, 0);
    w.schedule_event(secs(8.0), EventKind::HatRain);
    let evs = run_match(&mut w);
    assert!(matches!(
        evs.last(),
        Some((
            _,
            DirectorEvent::Ended {
                kind: EventKind::HatRain
            }
        ))
    ));
    assert!(w.director().current.is_none());
}

#[test]
fn director_is_deterministic_and_seeded() {
    let run = |seed| {
        let mut w = world(seed, 100, rules(ModeKind::PaintTheVoid, 100), 2);
        let evs = run_match(&mut w);
        (announced(&evs), w.state_hash())
    };
    assert_eq!(run(21), run(21));
    let schedules: std::collections::BTreeSet<Vec<(u64, EventKind)>> =
        (0..8).map(|s| run(s).0).collect();
    assert!(schedules.len() >= 6, "seeds give different schedules");
}

#[test]
fn rng_is_keyed_by_seed_tick_and_key() {
    let a = mood_rng(1, 100, 0).next_u64();
    assert_eq!(a, mood_rng(1, 100, 0).next_u64());
    assert_ne!(a, mood_rng(2, 100, 0).next_u64());
    assert_ne!(a, mood_rng(1, 101, 0).next_u64());
    assert_ne!(a, mood_rng(1, 100, 1).next_u64());
}

#[test]
fn move_effects_apply_forces_and_plane_locks() {
    let t = crate::tuning::ShipTable::builtin();
    let mut s = crate::ShipState::new(
        PlayerId(0),
        0,
        ShipType::Tinker,
        t.get(ShipType::Tinker),
        Vec3::ZERO,
        Quat::IDENTITY,
    );
    s.vel = Vec3::new(1.0, 2.0, 3.0);
    let fx = crate::MoveEffects {
        accel: Vec3::new(0.0, 60.0, 0.0),
        impulse: Vec3::new(1.0, 0.0, 0.0),
        lock_normal: Some(Vec3::Z),
    };
    apply_move_effects_before(&mut s, &fx);
    assert_eq!(s.vel, Vec3::new(2.0, 3.0, 0.0));
    let before = Vec3::new(0.0, 0.0, 5.0);
    s.pos = Vec3::new(1.0, 1.0, 5.5);
    s.vel.z = 4.0;
    apply_move_effects_after(&mut s, &fx, before);
    assert_eq!(s.pos, Vec3::new(1.0, 1.0, 5.0));
    assert_eq!(s.vel.z, 0.0);
    // No event: neutral effects.
    let v = MoodView::default();
    assert_eq!(event_effects(&v, 1, 1), EventEffects::NONE);
    assert!(move_effects(&v, 1, 1, &s).is_none());
    assert!(allows_pickup(&v, 1, 1, 0));
    assert!(!reveals(&v, 1, 1, &s));
    // Stubs have no effects either, in any phase.
    for kind in EventKind::ALL {
        let v = MoodView {
            event: Some(kind),
            phase: EventPhase::Active,
            ..MoodView::default()
        };
        assert_eq!(event_effects(&v, 1, 1), EventEffects::NONE, "{kind:?}");
        assert!(move_effects(&v, 1, 1, &s).is_none());
    }
}

/// Mean events per 5-minute match by Absurdity: within 20 % of the §3.1
/// expectations (1.4 at 20, 3.5 at 50, 5 at 100), 0 at 0. 200 matches per
/// level in release builds (the acceptance criterion), 24 in debug builds
/// (with a looser bound).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn event_statistics_match_the_plan() {
    let matches: u64 = if cfg!(debug_assertions) { 24 } else { 200 };
    let tolerance = if cfg!(debug_assertions) { 0.4 } else { 0.2 };
    let mut report = Vec::new();
    for (absurdity, expected) in [(0u8, 0.0f64), (20, 1.4), (50, 3.5), (100, 5.0)] {
        let mut total = 0usize;
        for m in 0..matches {
            let seed = 0x5eed_0000 + m * 7919 + absurdity as u64;
            let mut w = world(seed, absurdity, rules(ModeKind::PaintTheVoid, 100), 2);
            total += announced(&run_match(&mut w)).len();
        }
        let mean = total as f64 / matches as f64;
        report.push(format!(
            "Absurdity {absurdity}: {mean:.2} events per match (plan {expected})"
        ));
        if expected == 0.0 {
            assert_eq!(total, 0);
        } else {
            assert!(
                (mean - expected).abs() <= expected * tolerance,
                "Absurdity {absurdity}: {mean:.2} events per match, plan {expected}"
            );
        }
    }
    println!("{}", report.join("\n"));
}
