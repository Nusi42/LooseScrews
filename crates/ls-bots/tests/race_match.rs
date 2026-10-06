//! Belt Up with racing bots (M4 package C5; `docs/milestone-4-plan.md` §3.8
//! and the C5 acceptance criteria):
//!
//! - `race_full_match`: 8 racing bots (all three personalities) play a full
//!   3-heat match of the built-in Belt Up rule set on a generated belt: a
//!   result, 0 desyncs, 0 protocol errors, every bot finishes at least 2 of
//!   the 3 heats, and the bots race (combat on the racer ahead, bumps,
//!   scavenging, the dive).
//! - `time_trials`: on 8 belt seeds, one Tinker bot alone per line (the
//!   belt line, the open line without scavenging, the mixed line) flies a
//!   heat; the tuning targets (§5 C5 acceptance) must hold on at least 6 of
//!   8 seeds: the mixed line is fastest, the open line is not faster than
//!   the belt line, every line finishes inside the 180 s heat limit. The
//!   radiation tuning (`data/world/radiation.ron`) is judged by these runs
//!   and by `skim_lap_and_dive_doses` below.
//!
//! Needs whole-level generation, so it runs only in release:
//! `cargo test -p ls-bots --release --test race_match -- --nocapture`.

mod support;

use std::collections::BTreeMap;

use ls_bots::race_line::{dive_cost, Radiation};
use ls_bots::{BotStats, BrainConfig, RaceLineChoice, RacePersonality};
use ls_game::ls_math::level::{Archetype, BeltSite};
use ls_game::ls_math::Vec3;
use ls_game::ls_sim::modes::belt_up::heat_phase;
use ls_game::ls_sim::{MatchPhase, ModeKind, PlayerId, RuleSet, ShipType};
use ls_game::ls_worldgen::{Generator, ShareCode, Sliders};
use ls_game::MatchConfig;

use support::Match;

/// A belt world inside the Belt Up playlist ranges (§3.8), varied by seed,
/// without events (`absurdity`).
fn belt(seed: u64, absurdity: u8) -> ShareCode {
    let mix = |k: u64, lo: u64, hi: u64| {
        (lo + (seed.wrapping_mul(0x9e37_79b9) >> k) % (hi - lo + 1)) as u8
    };
    let sliders = Sliders {
        enclosure: mix(3, 30, 70),
        density: mix(9, 40, 80),
        verticality: mix(15, 30, 70),
        gravity_chaos: mix(21, 0, 50),
        palette: 20,
        absurdity,
        instability: mix(27, 10, 40),
    };
    ShareCode::with_archetype(seed, sliders, Archetype::Belt)
}

fn belt_up_rules() -> RuleSet {
    let mut rules = RuleSet::default_for(ModeKind::BeltUp);
    // Tuning runs: `LS_BELT_UP_<field>=<value>` overrides a mode field.
    for (k, v) in std::env::vars() {
        if let Some(field) = k.strip_prefix("LS_BELT_UP_") {
            let v: f32 = v.parse().expect("a number");
            rules.mode.set_field(&field.to_lowercase(), v);
        }
    }
    RuleSet::from_code(&rules.to_code().unwrap()).unwrap()
}

/// One heat flown alone.
#[derive(Debug, Clone, Copy)]
struct Trial {
    /// Seconds from the lights going out to the finish (`None`: did not
    /// finish inside the heat limit).
    secs: Option<f32>,
    deaths: u32,
    /// Hull left at the finish.
    hull: f32,
    scavenge_secs: f32,
    /// Planned time and dose of the line (the line the bot started with).
    planned: (f32, f32),
    open_share: f32,
}

fn time_trial(code: ShareCode, line: RaceLineChoice) -> Trial {
    let mut rules = belt_up_rules();
    rules.allowed_ship_types = vec![ShipType::Tinker];
    let mc = MatchConfig {
        rules,
        ..MatchConfig::paint_the_void(code)
    };
    let mut m = Match::new(mc, 1, 9100);
    m.cfg = Box::new(move |_| BrainConfig {
        race_line: line,
        ..BrainConfig::default()
    });
    let mut planned = None;
    let mut hull = 0.0;
    loop {
        m.step();
        let world = m.room.world();
        let Some(race) = world.mode().race.as_ref() else {
            continue;
        };
        if planned.is_none() {
            if let Some(kit) = m.clients[0]
                .core
                .as_ref()
                .and_then(|c| c.brain().race_kit())
            {
                if let Some(l) = kit.line() {
                    planned = Some(((l.total_time(), l.total_dose()), l.open_share(kit.belt())));
                }
            }
        }
        if let Some(s) = world.ships().next() {
            if s.alive {
                hull = s.hull;
            }
            if std::env::var("LS_TRIAL_TRACE").is_ok() && m.now.is_multiple_of(60) {
                let b = world.setup().belt.as_ref().unwrap();
                let r = race.racers.values().next();
                eprintln!(
                    "  t {:>4} {line:?} alive {} hull {:>5.1} speed {:>5.1} r {:>5.1} h {:>5.1} az {:>5.1} gates {:?} shelter {:>5.1}",
                    m.now / 60,
                    s.alive,
                    s.hull,
                    s.vel.length(),
                    b.axis_distance(s.pos),
                    s.pos.y - b.center.y,
                    b.azimuth(s.pos).to_degrees(),
                    r.map(|r| (r.slot, r.gates)),
                    b.shelter_distance(s.pos),
                );
            }
        }
        let racer = race.racers.values().next();
        let done = racer.is_some_and(|r| r.finish_tick > 0) || race.phase >= heat_phase::RESULTS;
        if done || m.now > 200 * 60 {
            let secs = racer
                .filter(|r| r.finish_tick > 0)
                .map(|r| (r.finish_tick - race.start_tick) as f32 / 60.0);
            let stats = m.finish();
            let s = &stats[0].1;
            assert_eq!(s.desyncs, 0, "{code} {line:?}: {:?}", s.errors);
            assert_eq!(s.protocol_errors, 0, "{code} {line:?}: {:?}", s.errors);
            let (planned, open_share) = planned.unwrap_or(((0.0, 0.0), 0.0));
            return Trial {
                secs,
                deaths: s.deaths,
                hull,
                scavenge_secs: s.behavior.race.scavenge_ticks as f32 / 60.0,
                planned,
                open_share,
            };
        }
    }
}

/// The time trials (see the module docs).
#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn time_trials() {
    // `LS_TIME_TRIAL_SEEDS=n` flies only the first n seeds (tuning runs).
    let n = std::env::var("LS_TIME_TRIAL_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8u64);
    let mut seeds: Vec<u64> = (0..n).map(|i| 0x00c5_7700 + i).collect();
    // `LS_TIME_TRIAL_ONLY=i` flies only seed i (tuning runs).
    if let Some(i) = std::env::var("LS_TIME_TRIAL_ONLY")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        seeds = vec![0x00c5_7700 + i];
    }
    let mut met = 0;
    let mut table = Vec::new();
    for &seed in &seeds {
        let code = belt(seed, 0);
        let belt_line = time_trial(code, RaceLineChoice::Belt);
        let open = time_trial(code, RaceLineChoice::Open);
        let mixed = time_trial(code, RaceLineChoice::Mixed);
        let t = |x: &Trial| x.secs.unwrap_or(f32::INFINITY);
        let inside = [&belt_line, &open, &mixed]
            .iter()
            .all(|x| x.secs.is_some_and(|s| s < 180.0));
        let mixed_fastest = t(&mixed) <= t(&belt_line) && t(&mixed) <= t(&open);
        let open_not_faster = t(&open) >= t(&belt_line);
        let ok = inside && mixed_fastest && open_not_faster;
        met += ok as usize;
        for (name, x) in [("belt", &belt_line), ("open", &open), ("mixed", &mixed)] {
            eprintln!(
                "{code} {name:>5}: {:>6} s, {} deaths, hull left {:>5.1}, scavenged {:>4.1} s; \
                 planned {:>5.1} s {:>5.1} hp, open {:>3.0} %",
                x.secs.map_or("DNF".to_string(), |s| format!("{s:.1}")),
                x.deaths,
                x.hull,
                x.scavenge_secs,
                x.planned.0,
                x.planned.1,
                x.open_share * 100.0
            );
        }
        eprintln!(
            "{code}: inside the limit {inside}, mixed fastest {mixed_fastest}, \
             open not faster than belt {open_not_faster} => {}",
            if ok { "met" } else { "MISSED" }
        );
        table.push((code, t(&belt_line), t(&open), t(&mixed)));
    }
    eprintln!("time trials: targets met on {met} of {} seeds", seeds.len());
    for (code, b, o, x) in &table {
        eprintln!("  {code}: belt {b:.1} s, open {o:.1} s, mixed {x:.1} s");
    }
    if seeds.len() == 8 {
        assert!(met >= 6, "tuning targets met on {met} of 8 seeds");
    }
}

/// The radiation numbers against the tuning targets, on a generated belt: a
/// Tinker skimming 4 u above the belt at cruise speed for a lap (the middle
/// radius, 754 u) takes about one hull's worth (dies about once, §3.8); the
/// dive from the inner edge to the cocoon at full boost costs about 20 hp
/// (§3.8 started from 35; the time trials showed that a racer who dies in
/// the dive respawns at its gate with the 30 % hull floor and must still
/// make it), and from a respawn (half boost, then cruise) less than 30 hp.
#[test]
fn skim_lap_and_dive_doses() {
    let generator = Generator::from_code(&belt(0x00c5_7700, 0)).unwrap();
    let b: BeltSite = generator.belt().expect("a belt").clone();
    let rad = Radiation::builtin(1.0);
    let lap = 2.0 * std::f32::consts::PI * 120.0;
    let skim = rad.rate(&b, b.center + Vec3::new(120.0, b.half_thickness + 4.0, 0.0));
    let lap_dose = skim * lap / 12.0;
    let edge = b.center + Vec3::new(b.inner_radius, 0.0, 0.0);
    let (secs, dive) = dive_cost(&b, &rad, edge, 16.5);
    // From the inner edge to the cocoon (the rest of the way is sheltered).
    let to_cocoon = (b.inner_radius - b.cocoon_radius) / 16.5;
    eprintln!(
        "skim 4 u above: {skim:.2} hp/s, {lap_dose:.0} hp per {:.0} s lap; \
         dive {dive:.1} hp in {secs:.1} s ({to_cocoon:.1} s exposed)",
        lap / 12.0
    );
    assert!(
        (80.0..=130.0).contains(&lap_dose),
        "a skimming Tinker dies about once per lap: {lap_dose:.0} hp"
    );
    assert!(
        (15.0..=28.0).contains(&dive),
        "the dive costs about 20 hp: {dive:.1}"
    );
    // From a respawn at the own gate: 50 boost (2.5 s at 16.5 u/s, from
    // the slot 25 u inside the belt), then cruise.
    let slot = b.center + Vec3::new(120.0, 0.0, 0.0);
    let boosted = slot - Vec3::new(2.5 * 16.5, 0.0, 0.0);
    let (_, part1) = dive_cost(&b, &rad, slot, 16.5);
    let (_, rest) = dive_cost(&b, &rad, boosted, 12.0);
    let (_, rest_fast) = dive_cost(&b, &rad, boosted, 16.5);
    let respawn_dive = part1 - rest_fast + rest;
    eprintln!("dive from a respawn: {respawn_dive:.1} hp");
    assert!(
        respawn_dive < 30.0,
        "a respawned Tinker can dive: {respawn_dive:.1}"
    );
}

/// Per heat: the racers that finished it.
#[derive(Default, Debug)]
struct HeatLog {
    finishers: BTreeMap<u8, Vec<PlayerId>>,
    racers: BTreeMap<u8, Vec<PlayerId>>,
    phase: Option<(u8, u8)>,
}

impl HeatLog {
    fn observe(&mut self, m: &Match) {
        let Some(race) = m.room.world().mode().race.as_ref() else {
            return;
        };
        let now = (race.heat, race.phase);
        if self.phase != Some(now) && race.phase == heat_phase::RESULTS {
            let mut fin = Vec::new();
            let mut all = Vec::new();
            for (id, r) in &race.racers {
                if r.in_heat {
                    all.push(*id);
                    if r.finish_tick > 0 {
                        fin.push(*id);
                    }
                }
            }
            self.finishers.insert(race.heat, fin);
            self.racers.insert(race.heat, all);
        }
        self.phase = Some(now);
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "full level generation; run with --release")]
fn race_full_match() {
    let code = belt(0x00c5_0001, 30);
    let mc = MatchConfig {
        rules: belt_up_rules(),
        ..MatchConfig::paint_the_void(code)
    };
    let mut m = Match::new(mc, 8, 7300);
    m.cfg = Box::new(|i| BrainConfig {
        race_personality: Some(
            [
                RacePersonality::Hugger,
                RacePersonality::Skimmer,
                RacePersonality::Daredevil,
            ][i % 3],
        ),
        ..BrainConfig::default()
    });
    let mut log = HeatLog::default();
    let mut ended_at = None;
    while ended_at.is_none_or(|e| m.now < e + 180) {
        assert!(m.now < 16 * 60 * 60, "no result after {} ticks", m.now);
        m.step();
        log.observe(&m);
        if ended_at.is_none() && !m.room.world().mode().is_running() {
            ended_at = Some(m.now);
        }
    }
    let bots = m.finish();
    let world = m.room.world();
    let MatchPhase::Ended { winner } = world.mode().phase else {
        panic!("no result");
    };
    let results = world.results();
    eprintln!(
        "belt_up: ended at {:.1} s, winner {winner:?}",
        ended_at.unwrap() as f32 / 60.0
    );
    for r in &results.rows {
        eprintln!(
            "  {:?}: {:>2} points, {:>2} kills {:>2} deaths",
            r.player, r.score, r.kills, r.deaths
        );
    }
    for (heat, fin) in &log.finishers {
        eprintln!(
            "  heat {}: {} of {} finished",
            heat + 1,
            fin.len(),
            log.racers[heat].len()
        );
    }
    let sum = |f: fn(&BotStats) -> u64| bots.iter().map(|(_, s)| f(s)).sum::<u64>();
    eprintln!(
        "  racing {} s, held {} s, scavenging {} s, diving {} s, stuck {} s, \
         engaging {} s, bumping {} s; hits {}, deaths {}, max reconciliation error {:.3} u",
        sum(|s| s.behavior.race.race_ticks) / 60,
        sum(|s| s.behavior.race.held_ticks) / 60,
        sum(|s| s.behavior.race.scavenge_ticks) / 60,
        sum(|s| s.behavior.race.dive_ticks) / 60,
        sum(|s| s.behavior.race.stuck_ticks) / 60,
        sum(|s| s.behavior.race.engage_ticks) / 60,
        sum(|s| s.behavior.race.bump_ticks) / 60,
        bots.iter().map(|(_, s)| s.hits_dealt).sum::<u32>(),
        bots.iter().map(|(_, s)| s.deaths).sum::<u32>(),
        bots.iter()
            .map(|(_, s)| s.max_reconcile_error)
            .fold(0.0f32, f32::max),
    );
    let mut weapons: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for (_, s) in &bots {
        for (name, u) in &s.weapons {
            let e = weapons.entry(name.clone()).or_default();
            e.0 += u.fired;
            e.1 += u.hits;
        }
    }
    eprintln!("  weapons (fired, hits): {weapons:?}");
    assert_eq!(log.finishers.len(), 3, "three heats: {log:?}");
    for (id, s) in &bots {
        assert_eq!(s.joins, 1, "{}", s.name);
        assert_eq!(s.protocol_errors, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.desyncs, 0, "{}: {:?}", s.name, s.errors);
        assert_eq!(s.matches_over, 1, "{} saw the match end", s.name);
        let player = m.room.player_of(*id).expect("a player per bot");
        let finished = log
            .finishers
            .values()
            .filter(|f| f.contains(&player))
            .count();
        assert!(finished >= 2, "{} finished {finished} of 3 heats", s.name);
    }
    assert!(sum(|s| s.behavior.race.engage_ticks) > 0, "racers fought");
}
