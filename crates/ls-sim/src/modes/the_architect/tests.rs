//! The Architect's scenario tests (M4 package B4): roles and rotation,
//! rounds, the exit, scoring, lives, the stroke validation (every refusal),
//! every stroke's effect, edits per stroke and their spread, the "never
//! encloses a ship" property, the view, and the replay golden.

use ls_math::terrain::{TerrainEdit, TerrainQuery, MATERIAL_WEAK};
use ls_math::{Pcg32, Vec3};

use super::replay::{architect_replay, arena, rules, setup, ArchitectArena, ARCHITECT_REPLAY};
use super::*;
use crate::instability::{escapes, InstabilitySettings};
use crate::mode::{player_flags, MatchPhase};
use crate::{Command, DamageSource, Event, InputCmd, RuleSet, ShipType, World, WorldConfig};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const ENCLOSURE_PLUS_SPOT: Vec3 = Vec3::new(20.0, 14.0, 0.0);
const ENCLOSURE_MINUS_SPOT: Vec3 = Vec3::new(-10.0, 0.0, 20.0);
const DENSITY_SPOT: Vec3 = Vec3::new(30.0, -10.0, 6.0);

fn world_with(r: RuleSet, players: usize) -> (World, ArchitectArena, Vec<PlayerId>) {
    let mut cfg = WorldConfig::paint_the_void(77, setup());
    cfg.rules = r;
    let mut w = World::new(cfg).unwrap();
    let ids = (0..players)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    (w, arena(), ids)
}

fn step(w: &mut World, t: &mut ArchitectArena, n: u64) -> Vec<Event> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.extend(w.step(t, &[]));
    }
    out
}

fn st(w: &World) -> &ArchitectState {
    w.mode().architect.as_deref().expect("an Architect match")
}

fn architect(w: &World) -> PlayerId {
    st(w).architect.expect("an architect")
}

fn flags(w: &World, id: PlayerId) -> u8 {
    w.mode().players[&id].flags
}

fn to_exit(w: &mut World, id: PlayerId) {
    let exit = w.mode().objective(EXIT_ID).unwrap().pos;
    w.ship_mut(id).unwrap().pos = exit;
}

fn kill(w: &mut World, t: &mut ArchitectArena, victim: PlayerId) {
    w.hook_for_test(t, |ctx| {
        ctx.push(Command::AdjustHull {
            target: victim,
            delta: -10_000.0,
            can_kill: true,
        })
    });
}

fn stroke(
    w: &mut World,
    t: &ArchitectArena,
    kind: StrokeKind,
    center: Vec3,
    radius: f32,
) -> Result<u8, StrokeReject> {
    let a = architect(w);
    w.architect_action(t, a, kind.index(), center, radius)
}

#[test]
fn roles_rounds_and_the_exit() {
    let (mut w, mut t, ids) = world_with(rules(240.0), 3);
    step(&mut w, &mut t, 1);
    let a = st(&w);
    assert_eq!(a.rounds, 3, "min(players, 3) rounds");
    assert_eq!(a.round, 0);
    assert_eq!(a.architect, Some(ids[0]), "join order");
    assert_eq!(a.runners, ids[1..].iter().copied().collect());
    assert!(flags(&w, ids[0]) & player_flags::ARCHITECT != 0);
    assert!(!w.ship(ids[0]).unwrap().alive, "the architect has no ship");
    assert!(ids[1..].iter().all(|id| w.ship(*id).unwrap().alive));
    assert!(w.is_spectator(ids[0]));
    // Runners spawn at the team 0 end; the exit is at the team 1 end.
    let exit = w.mode().objective(EXIT_ID).unwrap();
    assert_eq!(exit.kind, objective_kind::EXIT);
    assert_eq!(exit.pos, Vec3::new(66.0, 0.0, 0.0));
    assert_eq!(exit.holder, Some(ids[0]));
    assert_eq!(exit.state >> exit_state::ROUNDS_SHIFT, 3);
    assert!(ids[1..].iter().all(|id| w.ship(*id).unwrap().pos.x < -60.0));
    // Lives per round from the rule set.
    assert!(ids[1..].iter().all(|id| w.mode().players[id].lives == 2));
    // The match clock covers the rounds and their results.
    assert_eq!(w.mode().stage_end_tick, 1 + 240 * 60);
    assert_eq!(w.mode().end_tick, 1 + 3 * (240 * 60 + 2 * 60));
}

#[test]
fn rules_validate() {
    let m = TheArchitect;
    assert!(m
        .validate(&ModeRules::TheArchitect(Default::default()))
        .is_ok());
    for (field, value) in [
        ("max_rounds", 0.0),
        ("runner_lives", 0.0),
        ("round_secs", 0.0),
        ("min_radius", 30.0),
        ("max_strokes", 0.0),
        ("reshape_delta", 0.0),
        ("telegraph_secs", -1.0),
    ] {
        let mut r = ModeRules::TheArchitect(Default::default());
        r.set_field(field, value);
        assert!(m.validate(&r).is_err(), "{field} = {value}");
    }
    // The built-in rule set resolves; teams other than 0 are refused.
    let r = RuleSet::default_for(ModeKind::TheArchitect);
    assert!(r.resolve(&crate::WeaponTable::builtin()).is_ok());
    let mut r2 = r.clone();
    r2.mode.set_field("teams", 2.0);
    assert!(r2.resolve(&crate::WeaponTable::builtin()).is_err());
    // Runners' loadout: Scrap Laser and 4 Bolt Rockets.
    assert_eq!(
        r.starting_loadout.primaries,
        vec!["scrap_laser".to_string()]
    );
    assert_eq!(
        r.starting_loadout.secondaries,
        vec![("bolt_rocket".to_string(), 4)]
    );
}

#[test]
fn runner_at_the_exit_scores_and_leaves_play() {
    let (mut w, mut t, ids) = world_with(rules(240.0), 3);
    step(&mut w, &mut t, 1);
    to_exit(&mut w, ids[1]);
    let ev = step(&mut w, &mut t, 1);
    assert!(flags(&w, ids[1]) & player_flags::FINISHED != 0);
    assert_eq!(w.mode().players[&ids[1]].score, 1);
    assert!(!w.ship(ids[1]).unwrap().alive, "it left play");
    assert!(
        !ev.iter().any(|e| matches!(e, Event::Eliminated { .. })),
        "finishing is no elimination"
    );
    assert!(st(&w).finished.contains(&ids[1]));
    // It stays out of play (no respawn) for the rest of the round.
    step(&mut w, &mut t, 300);
    assert!(!w.ship(ids[1]).unwrap().alive);
    assert_eq!(st(&w).phase, RoundPhase::Playing, "one runner still runs");
}

#[test]
fn round_ends_on_time_architect_scores_and_the_role_rotates() {
    let (mut w, mut t, ids) = world_with(rules(3.0), 3);
    step(&mut w, &mut t, 3 * 60);
    assert_eq!(st(&w).phase, RoundPhase::Playing);
    step(&mut w, &mut t, 1);
    let a = st(&w);
    assert_eq!(a.phase, RoundPhase::Over);
    assert_eq!(a.architect_points[&ids[0]], 2, "both runners missed");
    assert_eq!(w.mode().players[&ids[0]].score, 2);
    assert_eq!(
        a.history,
        vec![RoundResult {
            architect: Some(ids[0]),
            runners: 2,
            finished: 0
        }]
    );
    let exit = w.mode().objective(EXIT_ID).unwrap();
    assert!(exit.state & exit_state::ROUND_OVER != 0);
    assert!(exit.state & exit_state::RUNNERS_WON == 0);
    // Results, then round 2 with the next architect.
    let ev = step(&mut w, &mut t, 2 * 60);
    let a = st(&w);
    assert_eq!((a.round, a.phase), (1, RoundPhase::Playing));
    assert_eq!(a.architect, Some(ids[1]));
    assert_eq!(w.mode().stage, 1);
    assert!(w.ship(ids[0]).unwrap().alive, "the old architect runs");
    assert!(!w.ship(ids[1]).unwrap().alive, "the new one has no ship");
    assert!(flags(&w, ids[0]) & player_flags::ARCHITECT == 0);
    assert!(flags(&w, ids[1]) & player_flags::ARCHITECT != 0);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Respawned { player, .. } if *player == ids[0])));
    // Everyone runs from the start again with a fresh budget.
    assert!(w.ship(ids[2]).unwrap().pos.x < -60.0);
    assert_eq!(a.budget_points(), 100);
}

#[test]
fn round_ends_early_when_no_runner_is_left_and_runners_can_win_it() {
    let (mut w, mut t, ids) = world_with(rules(240.0), 2);
    step(&mut w, &mut t, 1);
    to_exit(&mut w, ids[1]);
    step(&mut w, &mut t, 1);
    let a = st(&w);
    assert_eq!(a.phase, RoundPhase::Over);
    assert!(a.history[0].runners_won());
    assert!(w.mode().objective(EXIT_ID).unwrap().state & exit_state::RUNNERS_WON != 0);
    assert_eq!(a.architect_points.get(&ids[0]).copied().unwrap_or(0), 0);
    // A runner out of lives ends the round the same way (round 2).
    step(&mut w, &mut t, 2 * 60);
    assert_eq!(st(&w).architect, Some(ids[1]));
    for _ in 0..2 {
        kill(&mut w, &mut t, ids[0]);
        step(&mut w, &mut t, 3 * 60 + 1);
    }
    assert!(flags(&w, ids[0]) & player_flags::ELIMINATED != 0);
    let a = st(&w);
    assert_eq!(a.history.len(), 2);
    assert_eq!(a.architect_points[&ids[1]], 1);
}

#[test]
fn runners_have_two_lives_per_round_and_get_them_back() {
    let (mut w, mut t, ids) = world_with(rules(10.0), 3);
    step(&mut w, &mut t, 1);
    let r = ids[1];
    kill(&mut w, &mut t, r);
    assert_eq!(w.mode().players[&r].lives, 1);
    assert_eq!(w.mode().players[&r].score, 0, "dying costs no points");
    step(&mut w, &mut t, 3 * 60 + 1);
    assert!(w.ship(r).unwrap().alive, "respawned with its second life");
    kill(&mut w, &mut t, r);
    let ev = step(&mut w, &mut t, 3 * 60 + 1);
    assert!(flags(&w, r) & player_flags::ELIMINATED != 0);
    assert!(!w.ship(r).unwrap().alive);
    assert!(ev
        .iter()
        .all(|e| !matches!(e, Event::Respawned { player, .. } if *player == r)));
    // Next round: lives back, flying again.
    step(&mut w, &mut t, 10 * 60);
    assert_eq!(st(&w).round, 1);
    assert_eq!(w.mode().players[&r].lives, 2);
    assert!(flags(&w, r) & player_flags::ELIMINATED == 0);
}

#[test]
fn runners_never_hurt_each_other() {
    let (mut w, mut t, ids) = world_with(rules(240.0), 3);
    step(&mut w, &mut t, 1);
    let hull = w.ship(ids[2]).unwrap().hull;
    let src = DamageSource {
        attacker: Some(ids[1]),
        team: w.ship(ids[1]).unwrap().team,
        weapon: None,
    };
    w.hook_for_test(&mut t, |ctx| {
        ctx.damage(ids[2], 50.0, Vec3::ZERO, src, true);
    });
    assert_eq!(w.ship(ids[2]).unwrap().hull, hull);
    // The level still hurts.
    w.hook_for_test(&mut t, |ctx| {
        ctx.damage(ids[2], 10.0, Vec3::ZERO, DamageSource::ENVIRONMENT, false);
    });
    assert!(w.ship(ids[2]).unwrap().hull < hull);
}

#[test]
fn a_full_match_rotates_every_player_and_has_a_winner() {
    let (mut w, mut t, ids) = world_with(rules(2.0), 3);
    step(&mut w, &mut t, 1);
    to_exit(&mut w, ids[1]);
    let mut ended = None;
    for _ in 0..30 * 60 {
        for e in w.step(&mut t, &[]) {
            if let Event::MatchEnded { winner, .. } = e {
                ended = Some(winner);
            }
        }
        if ended.is_some() {
            break;
        }
    }
    let a = st(&w);
    let architects: Vec<_> = a.history.iter().map(|r| r.architect).collect();
    assert_eq!(architects, vec![Some(ids[0]), Some(ids[1]), Some(ids[2])]);
    // p0: 1 as architect; p1: 1 run + 2 as architect; p2: 2 as architect.
    let scores: Vec<i32> = ids.iter().map(|id| w.mode().players[id].score).collect();
    assert_eq!(scores, vec![1, 3, 2]);
    let side = w.ship(ids[1]).unwrap().team;
    assert_eq!(ended, Some(Some(side)));
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(side) });
    let res = w.results();
    assert_eq!(res.rows[0].player, ids[1]);
    // The final view: nobody is the architect, meter = architect points.
    let v = w.mode_view();
    let row = |id: PlayerId| *v.players.iter().find(|p| p.player == id).unwrap();
    assert_eq!(
        ids.iter().map(|id| row(*id).meter).collect::<Vec<_>>(),
        vec![1, 2, 2]
    );
    assert!(v
        .players
        .iter()
        .all(|p| p.flags & player_flags::ARCHITECT == 0));
}

#[test]
fn a_second_player_early_in_a_lone_start_becomes_the_architect() {
    let (mut w, mut t, ids) = world_with(rules(240.0), 1);
    step(&mut w, &mut t, 2);
    assert_eq!(st(&w).architect, None, "a lone runner practises");
    assert_eq!(st(&w).rounds, 1);
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    step(&mut w, &mut t, 1);
    let a = st(&w);
    assert_eq!(a.architect, Some(ids[0]));
    assert_eq!(a.rounds, 2);
    assert_eq!(a.round_start_tick, 3, "the round starts over");
    assert!(a.runners.contains(&b));
    assert!(!w.ship(ids[0]).unwrap().alive);
    // Later joiners just run (with the round's lives).
    let c = w.add_player(None, ShipType::Hauler).unwrap();
    assert!(st(&w).runners.contains(&c));
    assert!(w.ship(c).unwrap().alive);
    assert_eq!(w.mode().players[&c].lives, 2);
}

#[test]
fn budget_accrues_four_points_a_second_up_to_the_cap() {
    let (mut w, mut t, ids) = world_with(rules(240.0), 2);
    step(&mut w, &mut t, 1);
    let b0 = st(&w).budget_points();
    assert_eq!(b0, 100);
    step(&mut w, &mut t, 60);
    assert_eq!(st(&w).budget_points(), 104);
    step(&mut w, &mut t, 60 * 60);
    assert_eq!(st(&w).budget_points(), 250, "capped");
    let v = w.mode_view();
    let me = v.players.iter().find(|p| p.player == ids[0]).unwrap();
    assert_eq!(me.meter, 250);
    assert!(me.flags & player_flags::ARCHITECT != 0);
}

#[test]
fn forged_and_invalid_strokes_are_refused() {
    let mut r = rules(240.0);
    r.mode.set_field("max_strokes", 2.0);
    let (mut w, mut t, ids) = world_with(r, 3);
    step(&mut w, &mut t, 1);
    let ok = ENCLOSURE_PLUS_SPOT;
    let try_as = |w: &mut World, p: PlayerId, k: u8, c: Vec3, rad: f32| {
        w.architect_action(&arena(), p, k, c, rad)
    };
    assert_eq!(
        try_as(&mut w, ids[1], 0, ok, 14.0),
        Err(StrokeReject::NotArchitect),
        "a runner"
    );
    assert_eq!(
        try_as(&mut w, ids[0], 8, ok, 14.0),
        Err(StrokeReject::UnknownStroke)
    );
    for rad in [11.9, 24.1, f32::NAN] {
        assert_eq!(
            try_as(&mut w, ids[0], 0, ok, rad),
            Err(StrokeReject::Radius)
        );
    }
    for c in [
        Vec3::new(500.0, 0.0, 0.0),
        Vec3::new(f32::INFINITY, 0.0, 0.0),
        // Rock strokes keep clear of the exit and the spawn points.
        Vec3::new(50.0, 0.0, 0.0),
        Vec3::new(-50.0, 0.0, 0.0),
    ] {
        assert_eq!(
            try_as(&mut w, ids[0], 0, c, 14.0),
            Err(StrokeReject::OutOfRange)
        );
    }
    // A well may sit near the exit (not a rock stroke).
    assert!(try_as(&mut w, ids[0], 3, Vec3::new(50.0, 0.0, 0.0), 14.0).is_ok());
    assert_eq!(
        try_as(&mut w, ids[0], 6, ok, 14.0),
        Err(StrokeReject::Cooldown)
    );
    step(&mut w, &mut t, 2 * 60);
    // Nothing to grow in open space far from rock.
    assert_eq!(
        try_as(&mut w, ids[0], 0, Vec3::new(40.0, 0.0, 4.0), 12.0),
        Err(StrokeReject::NothingToChange)
    );
    assert!(try_as(&mut w, ids[0], 6, ok, 14.0).is_ok());
    step(&mut w, &mut t, 2 * 60);
    let a = st(&w);
    assert_eq!(a.strokes.len(), 2, "well and blackout still acting");
    assert_eq!(
        try_as(&mut w, ids[0], 6, ok, 14.0),
        Err(StrokeReject::TooManyStrokes)
    );
    // Budget: Absurdity+ (60) fits the 100 at the start, then Verticality
    // (80) does not.
    let mut w2 = world_with(rules(240.0), 2).0;
    let mut t2 = arena();
    step(&mut w2, &mut t2, 1);
    let a2 = architect(&w2);
    assert!(w2.architect_action(&t2, a2, 5, ok, 14.0).is_ok());
    step(&mut w2, &mut t2, 2 * 60);
    assert_eq!(
        w2.architect_action(&t2, a2, 7, ok, 14.0),
        Err(StrokeReject::Budget)
    );
    // A stroke after the round ended.
    let (mut w3, mut t3, ids3) = world_with(rules(240.0), 2);
    step(&mut w3, &mut t3, 1);
    to_exit(&mut w3, ids3[1]);
    step(&mut w3, &mut t3, 1);
    assert_eq!(
        w3.architect_action(&t3, ids3[0], 6, ok, 14.0),
        Err(StrokeReject::RoundOver)
    );
    // Another mode.
    let mut cfg = WorldConfig::paint_the_void(1, setup());
    cfg.rules = RuleSet::default_for(ModeKind::TeamBrawl);
    let mut w4 = World::new(cfg).unwrap();
    let p = w4.add_player(None, ShipType::Tinker).unwrap();
    assert_eq!(
        w4.architect_action(&arena(), p, 6, ok, 14.0),
        Err(StrokeReject::NotArchitectMode)
    );
}

/// Steps `ticks` ticks, collecting the terrain edits with their tick.
fn edits_per_tick(w: &mut World, t: &mut ArchitectArena, ticks: u64) -> Vec<(u64, TerrainEdit)> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        for e in w.step(t, &[]) {
            if let Event::TerrainEdit(e) = e {
                out.push((w.tick(), e));
            }
        }
    }
    out
}

#[test]
fn reshape_strokes_are_telegraphed_then_land_over_eight_ticks() {
    for (kind, spot, add) in [
        (StrokeKind::EnclosurePlus, ENCLOSURE_PLUS_SPOT, true),
        (StrokeKind::EnclosureMinus, ENCLOSURE_MINUS_SPOT, false),
        (StrokeKind::DensityPlus, DENSITY_SPOT, true),
    ] {
        let (mut w, mut t, _) = world_with(rules(240.0), 3);
        step(&mut w, &mut t, 1);
        let now = w.tick();
        let id = stroke(&mut w, &t, kind, spot, 16.0).unwrap();
        let s = st(&w).strokes[0].clone();
        assert_eq!(s.id, id);
        assert!(
            !s.edits.is_empty() && s.edits.len() <= MAX_STROKE_EDITS,
            "{kind:?}"
        );
        assert_eq!(s.apply_tick, now + 120, "telegraphed 2 s");
        assert!(s.edits.iter().all(|e| match e {
            TerrainEdit::AddSphere { center, radius } =>
                add && *radius <= EDIT_RADIUS_MAX && center.distance(spot) <= 16.0,
            TerrainEdit::SubtractSphere { center, radius } =>
                !add && *radius <= EDIT_RADIUS_MAX && center.distance(spot) <= 16.0,
            _ => false,
        }));
        // The telegraph cue on the next tick; nothing changes before the
        // telegraph ends.
        let ev = w.step(&mut t, &[]);
        let cue = ev.iter().find_map(|e| match e {
            Event::Cue(c) if c.kind == crate::CueKind::StrokeTelegraph => Some(*c),
            _ => None,
        });
        let cue = cue.expect("telegraph cue");
        assert_eq!(cue.value >> 8, kind.index() as u16);
        assert_eq!(cue.value & 0xff, 16);
        assert_eq!(cue.pos, spot);
        let v = w.mode_view();
        let marker = v
            .objectives
            .iter()
            .find(|o| o.kind == objective_kind::STROKE)
            .expect("stroke marker");
        assert_eq!(marker.state, kind.index() | stroke_phase::TELEGRAPH << 4);
        assert_eq!(marker.timer_end, s.apply_tick);
        let edits = edits_per_tick(&mut w, &mut t, 140);
        assert!(edits
            .iter()
            .all(|(tick, _)| *tick >= s.apply_tick && *tick < s.apply_tick + STROKE_EDIT_TICKS));
        let per = s.edits.len().div_ceil(STROKE_EDIT_TICKS as usize);
        for k in 0..STROKE_EDIT_TICKS {
            let n = edits.iter().filter(|(t, _)| *t == s.apply_tick + k).count();
            assert!(n <= per, "{kind:?}: {n} edits at tick {k}");
        }
        assert!(edits.len() <= MAX_STROKE_EDITS);
        // Nobody stood in the way: every edit landed as planned.
        assert_eq!(
            edits.iter().map(|(_, e)| *e).collect::<Vec<_>>(),
            s.edits,
            "{kind:?}"
        );
        assert!(st(&w).strokes.is_empty(), "done");
        // The rock really changed.
        let rock = |p: Vec3| t.sdf(p) < 0.0;
        let changed = s.edits.iter().all(|e| match *e {
            TerrainEdit::AddSphere { center, .. } => rock(center),
            TerrainEdit::SubtractSphere { center, .. } => !rock(center),
            _ => false,
        });
        assert!(changed, "{kind:?}");
    }
}

#[test]
fn gravity_stroke_adds_a_telegraphed_well() {
    let (mut w, mut t, ids) = world_with(rules(240.0), 2);
    step(&mut w, &mut t, 1);
    let now = w.tick();
    let c = Vec3::new(10.0, -14.0, 20.0);
    let id = stroke(&mut w, &t, StrokeKind::GravityChaosPlus, c, 16.0).unwrap();
    let well = *w
        .dynamic_wells()
        .iter()
        .find(|d| d.key == WELL_KEY_BASE | id as u32)
        .expect("well");
    assert_eq!(well.pos, c);
    assert_eq!(well.start_tick, now + 120);
    assert_eq!(well.end_tick, now + 120 + 30 * 60);
    assert_eq!(well.range, 24.0);
    assert_eq!(well.owner, Some(ids[0]));
    assert_eq!(
        well.peak_accel,
        crate::GravitySettings::builtin().max_peak_accel
    );
    step(&mut w, &mut t, 120 + 30 * 60 + 1);
    assert!(w.dynamic_wells().is_empty(), "ended");
    assert!(st(&w).strokes.is_empty());
}

#[test]
fn instability_stroke_brings_weak_rubble_down() {
    let (mut w, mut t, _) = world_with(rules(240.0), 2);
    step(&mut w, &mut t, 1);
    // Under the rock shelf (ceiling in reach along up).
    let c = Vec3::new(2.0, 4.0, 0.0);
    stroke(&mut w, &t, StrokeKind::InstabilityPlus, c, 12.0).unwrap();
    let ev = w.step(&mut t, &[]);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == crate::CueKind::CaveInWarning)));
    let edits = edits_per_tick(&mut w, &mut t, 130);
    let adds: Vec<Vec3> = edits
        .iter()
        .filter_map(|(_, e)| match *e {
            TerrainEdit::AddSphere { center, .. } => Some(center),
            _ => None,
        })
        .collect();
    assert!(!adds.is_empty(), "rubble came down");
    assert!(edits
        .iter()
        .any(|(_, e)| matches!(e, TerrainEdit::SubtractSphere { .. })));
    assert!(edits.len() <= MAX_STROKE_EDITS);
    // The rubble is weak rock (two or three rockets clear it).
    assert!(!w.instability().rubble.is_empty());
    let mat = w.hook_for_test(&mut t, |ctx| ctx.material_at(adds[0])).0;
    assert_eq!(mat, MATERIAL_WEAK);
    // Nothing to bring down in the open, far from any ceiling.
    step(&mut w, &mut t, 2 * 60);
    assert_eq!(
        stroke(
            &mut w,
            &t,
            StrokeKind::InstabilityPlus,
            Vec3::new(40.0, -20.0, 25.0),
            12.0
        ),
        Err(StrokeReject::NothingToChange)
    );
}

#[test]
fn event_strokes_force_an_event_and_verticality_turns_the_level_once() {
    let mut r = rules(240.0);
    r.mode.set_field("budget_start", 250.0);
    let (mut w, mut t, _) = world_with(r, 3);
    step(&mut w, &mut t, 1);
    let spot = Vec3::new(0.0, 0.0, 10.0);
    stroke(&mut w, &t, StrokeKind::AbsurdityPlus, spot, 12.0).unwrap();
    let sched = w.director().schedule.clone();
    assert_eq!(sched.len(), 1);
    assert_eq!(sched[0].0, w.tick() + 120);
    // Another event stroke waits for it.
    let early = step(&mut w, &mut t, 2 * 60);
    assert_eq!(
        stroke(&mut w, &t, StrokeKind::Verticality, spot, 12.0),
        Err(StrokeReject::EventBusy)
    );
    // The event runs (announce, active, end).
    let mut announced = early.iter().any(|e| {
        matches!(
            e,
            Event::Director(crate::DirectorEvent::Announced { forced: true, .. })
        )
    });
    for _ in 0..80 * 60 {
        for e in w.step(&mut t, &[]) {
            if matches!(
                e,
                Event::Director(crate::DirectorEvent::Announced { forced: true, .. })
            ) {
                announced = true;
            }
        }
        if announced && w.director().current.is_none() {
            break;
        }
    }
    assert!(announced);
    assert!(w.director().current.is_none());
    // Verticality: Bored level, once per round.
    let mut w = world_with(rules(240.0), 3).0;
    let mut t = arena();
    step(&mut w, &mut t, 1);
    stroke(&mut w, &t, StrokeKind::Verticality, spot, 12.0).unwrap();
    assert_eq!(w.director().schedule[0].1, crate::EventKind::BoredLevel);
    // (Bored level's turn is B1's; here: it is announced, forced.)
    let ev = step(&mut w, &mut t, 2 * 60 + 1);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Director(crate::DirectorEvent::Announced {
            kind: crate::EventKind::BoredLevel,
            forced: true,
            ..
        })
    )));
    assert!(st(&w).verticality_used);
    step(&mut w, &mut t, 60 * 60);
    assert_eq!(
        stroke(&mut w, &t, StrokeKind::Verticality, spot, 12.0),
        Err(StrokeReject::OncePerRound)
    );
    // A rule set cap of 0: no event strokes.
    let mut r = rules(240.0);
    r.absurdity = 0;
    let (mut w, mut t, _) = world_with(r, 2);
    step(&mut w, &mut t, 1);
    assert_eq!(
        stroke(&mut w, &t, StrokeKind::AbsurdityPlus, spot, 12.0),
        Err(StrokeReject::EventsOff)
    );
}

#[test]
fn the_director_rolls_nothing_in_an_architect_match() {
    let mut cfg = WorldConfig::paint_the_void(5, setup());
    cfg.setup.absurdity = 100;
    cfg.rules = rules(240.0);
    let mut w = World::new(cfg).unwrap();
    let mut t = arena();
    for _ in 0..4 {
        w.add_player(None, ShipType::Tinker).unwrap();
    }
    for _ in 0..200 * 60 {
        w.step(&mut t, &[]);
    }
    assert!(w.effective_absurdity() > 0);
    assert_eq!(w.director().count, 0);
}

#[test]
fn palette_stroke_blacks_a_region_out_for_its_time() {
    let (mut w, mut t, _) = world_with(rules(240.0), 2);
    step(&mut w, &mut t, 1);
    let c = Vec3::new(30.0, 0.0, 0.0);
    stroke(&mut w, &t, StrokeKind::Palette, c, 20.0).unwrap();
    step(&mut w, &mut t, 121);
    let v = w.mode_view();
    let m = v
        .objectives
        .iter()
        .find(|o| o.kind == objective_kind::STROKE)
        .unwrap();
    assert_eq!(
        m.state,
        StrokeKind::Palette.index() | stroke_phase::ACTIVE << 4
    );
    assert_eq!(m.pos, c);
    assert_eq!(m.radius, 20.0);
    assert_eq!(m.timer_end, st(&w).strokes[0].apply_tick + 20 * 60);
    step(&mut w, &mut t, 20 * 60);
    assert!(w
        .mode_view()
        .objectives
        .iter()
        .all(|o| o.kind != objective_kind::STROKE));
}

/// An open point of the arena with `clear` room near `near`.
fn open_point(t: &ArchitectArena, rng: &mut Pcg32, near: Vec3, spread: f32, clear: f32) -> Vec3 {
    for _ in 0..500 {
        let p = near
            + Vec3::new(
                rng.range_f32(-spread, spread),
                rng.range_f32(-spread, spread),
                rng.range_f32(-spread, spread),
            );
        if t.sdf(p) > clear {
            return p;
        }
    }
    near
}

/// Property: over many seeded rock strokes (Enclosure+ and Density+ at the
/// largest slider move) with ships parked around and inside them, no ship
/// that could reach open space before is sealed in, and no rock overlaps a
/// ship.
#[test]
fn no_stroke_ever_encloses_a_ship() {
    let reach = InstabilitySettings::builtin().escape_reach;
    let mut r = rules(240.0);
    r.mode.set_field("reshape_delta", 100.0);
    r.mode.set_field("stroke_cooldown_secs", 0.0);
    r.mode.set_field("budget_start", 250.0);
    let mut placed = 0;
    let mut checked = 0;
    for seed in 0..12u64 {
        let (mut w, mut t, ids) = world_with(r.clone(), 4);
        step(&mut w, &mut t, 1);
        let mut rng = Pcg32::new(seed, 99);
        let kind = [
            StrokeKind::EnclosurePlus,
            StrokeKind::DensityPlus,
            StrokeKind::EnclosurePlus,
        ][seed as usize % 3];
        let centre = open_point(&t, &mut rng, Vec3::new(10.0, 4.0, 0.0), 22.0, 0.0);
        // Park the runners around it, inside the stroke.
        for id in &ids[1..] {
            let p = open_point(&t, &mut rng, centre, 10.0, 2.5);
            let s = w.ship_mut(*id).unwrap();
            s.pos = p;
            s.vel = Vec3::ZERO;
        }
        let before: Vec<(PlayerId, bool)> = ids[1..]
            .iter()
            .map(|id| {
                let s = w.ship(*id).unwrap();
                let radius = w.tuning_for(s.ship_type).radius;
                (*id, escapes(&t, &[], s.pos, radius, reach))
            })
            .collect();
        if stroke(&mut w, &t, kind, centre, 22.0).is_err() {
            continue;
        }
        placed += 1;
        let idle: Vec<(PlayerId, InputCmd)> =
            ids.iter().map(|id| (*id, InputCmd::idle(0))).collect();
        for _ in 0..140 {
            w.step(&mut t, &idle);
        }
        for (id, could) in before {
            let s = w.ship(id).unwrap();
            let radius = w.tuning_for(s.ship_type).radius;
            assert!(
                t.sdf(s.pos) > radius - 0.05,
                "seed {seed}: rock overlaps {id:?}"
            );
            if could {
                checked += 1;
                assert!(
                    escapes(&t, &[], s.pos, radius, reach),
                    "seed {seed}: {kind:?} sealed {id:?} in at {:?}",
                    s.pos
                );
            }
        }
    }
    assert!(placed >= 8, "only {placed} strokes placed");
    assert!(checked >= 20, "only {checked} ships checked");
}

#[test]
fn plan_reshape_is_bounded_and_deterministic() {
    use crate::terrain::reshape_slider::ENCLOSURE;
    let t = arena();
    let a = plan_reshape(&t, ENCLOSURE, 100, ENCLOSURE_PLUS_SPOT, 24.0);
    let b = plan_reshape(&t, ENCLOSURE, 100, ENCLOSURE_PLUS_SPOT, 24.0);
    assert_eq!(a, b);
    assert_eq!(a.len(), MAX_STROKE_EDITS, "a big move fills the cap");
    // Opening never cuts the level's walls.
    let open = plan_reshape(&t, ENCLOSURE, -100, Vec3::new(0.0, 12.0, 0.0), 24.0);
    let bounds = t.bounds();
    assert!(!open.is_empty());
    for e in open {
        let TerrainEdit::SubtractSphere { center, radius } = e else {
            panic!("opening only subtracts");
        };
        assert!(center.y + radius <= bounds.max.y - 2.0 + 1e-4);
    }
    // No generator: nothing.
    let plain = ls_math::terrain::BoxRoomTerrain::centered(Vec3::splat(64.0));
    assert!(plan_reshape(&plain, ENCLOSURE, 40, Vec3::ZERO, 20.0).is_empty());
}

#[test]
fn state_hash_covers_the_architect_state() {
    let (mut w, mut t, _) = world_with(rules(240.0), 3);
    step(&mut w, &mut t, 1);
    let h = w.state_hash();
    stroke(
        &mut w,
        &t,
        StrokeKind::Palette,
        Vec3::new(30.0, 0.0, 0.0),
        14.0,
    )
    .unwrap();
    assert_ne!(w.state_hash(), h);
}

// ----- replay golden ------------------------------------------------------

const FIXTURE: &str = include_str!("../../../tests/fixtures/mood_replays.txt");

fn fixture_line() -> Option<String> {
    FIXTURE
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(ARCHITECT_REPLAY))
        .map(str::to_string)
}

/// The replay matches its fixture line (native and wasm32);
/// `LS_BLESS_MOOD_REPLAYS=1` rewrites that line (native).
#[test]
fn architect_replay_matches_the_fixture() {
    let r = architect_replay();
    let line = format!("{ARCHITECT_REPLAY} {:#018x}", r.hash);
    println!("{line}: {r:?}");
    // The replay plays the mode: three rounds, strokes and their edits, a
    // runner through the exit, the end.
    assert!(r.ended && r.rounds == 3, "{r:?}");
    assert!(r.strokes >= 8 && r.refused >= 1 && r.edits >= 8, "{r:?}");
    assert!(r.finished >= 1, "{r:?}");
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var_os("LS_BLESS_MOOD_REPLAYS").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mood_replays.txt"
        );
        let text = std::fs::read_to_string(path).expect("read the fixture");
        let mut lines: Vec<String> = text
            .lines()
            .filter(|l| !l.trim().starts_with(ARCHITECT_REPLAY))
            .map(str::to_string)
            .collect();
        lines.push(line);
        std::fs::write(path, lines.join("\n") + "\n").expect("write the fixture");
        return;
    }
    assert_eq!(
        fixture_line(),
        Some(line),
        "the Architect replay changed: re-bless with LS_BLESS_MOOD_REPLAYS=1 if deliberate"
    );
}

#[test]
fn architect_replay_is_deterministic() {
    assert_eq!(architect_replay(), architect_replay());
}
