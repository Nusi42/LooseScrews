//! Heist scenario tests: one per core rule (vein and zones, hard vein rock,
//! mining frees the core, the collapsing mine, taking by touch, carrier
//! limits, drop and return, delivery and respawn, scoring and the end), a
//! 60 s replay with a golden hash (native and wasm32), and a full match of
//! 8 scripted bots on the default rule set.

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit, TerrainQuery, MATERIAL_HARD, MATERIAL_WEAK};
use ls_math::{Quat, Vec3};

use super::{core_state, HeistLevel, CORE_ID, SHED_INTERVAL_TICKS};
use crate::mode::{objective_kind, player_flags, MatchPhase, ModeKind, Objective, NO_SIDE};
use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

type Room = PaintedTerrain<BoxRoomTerrain>;

const VEIN: Vec3 = Vec3::new(0.0, -8.0, 0.0);
const KNOT: f32 = 2.0;
const ZONE0: Vec3 = Vec3::new(-24.0, 8.0, 0.0);
const ZONE1: Vec3 = Vec3::new(24.0, 8.0, 0.0);
const ZONE_R: f32 = 6.0;

/// A box room with the vein knot grown in and a level matching it.
fn room() -> Room {
    let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(64.0, 32.0, 64.0)), 2.0);
    t.apply_edit(&TerrainEdit::AddSphere {
        center: VEIN,
        radius: KNOT,
    });
    t
}

fn setup(level: bool) -> MatchSetup {
    let mut spawns = Vec::new();
    for team in 0..2u8 {
        let x = if team == 0 { -20.0 } else { 20.0 };
        for k in 0..4 {
            spawns.push(SpawnPoint {
                pos: Vec3::new(x, -4.0 + 3.0 * k as f32, 6.0),
                team,
                orientation: Quat::IDENTITY,
            });
        }
    }
    MatchSetup {
        wells: Vec::new(),
        belt: None,
        spawns,
        pickup_points: vec![],
        instability: 0,
        absurdity: 0,
        heist: level.then_some(HeistLevel {
            core: VEIN,
            knot_radius: KNOT,
            chamber_radius: 9.0,
            extraction: [(ZONE0, ZONE_R), (ZONE1, ZONE_R)],
        }),
        ..Default::default()
    }
}

fn rules(teams: u8) -> RuleSet {
    let mut r = RuleSet::default_for(ModeKind::Heist);
    r.mode.set_field("teams", teams as f32);
    r
}

fn world_with(rules: RuleSet, level: bool) -> World {
    let mut cfg = WorldConfig::paint_the_void(11, setup(level));
    cfg.rules = rules;
    World::new(cfg).unwrap()
}

fn world(teams: u8) -> World {
    world_with(rules(teams), true)
}

fn core(w: &World) -> Objective {
    *w.mode().objective(CORE_ID).expect("core")
}

fn step(w: &mut World, t: &mut Room, n: u64) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..n {
        ev.extend(w.step(t, &[]));
    }
    ev
}

fn teleport(w: &mut World, id: PlayerId, pos: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
}

/// Mines the knot away completely (exact craters, as any weapon could).
fn mine(w: &mut World, t: &mut Room) {
    w.hook_for_test(t, |ctx| ctx.crater_exact(VEIN, KNOT + 0.5));
}

/// A world with two players per team, the core mined free and taken by
/// team 0's first player.
fn carried(teams: u8) -> (World, Room, Vec<PlayerId>) {
    let mut w = world(teams);
    let mut t = room();
    let ids: Vec<PlayerId> = (0..4)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    step(&mut w, &mut t, 1);
    mine(&mut w, &mut t);
    step(&mut w, &mut t, 1);
    teleport(&mut w, ids[0], VEIN + Vec3::new(0.0, 0.0, 1.2));
    step(&mut w, &mut t, 1);
    assert_eq!(core(&w).holder, Some(ids[0]));
    (w, t, ids)
}

fn kill(w: &mut World, t: &mut Room, victim: PlayerId) {
    w.hook_for_test(t, |ctx| {
        ctx.push(Command::AdjustHull {
            target: victim,
            delta: -10_000.0,
            can_kill: true,
        })
    });
}

#[test]
fn default_rule_set_is_the_mode_file_and_round_trips_as_a_code() {
    let r = RuleSet::default_for(ModeKind::Heist);
    assert_eq!(r.name, "Heist");
    assert!(crate::ruleset::BUILTIN_RULESETS
        .iter()
        .any(|(f, _)| *f == ModeKind::Heist.ruleset_file()));
    let ModeRules::Heist(h) = r.mode else {
        panic!("not Heist: {:?}", r.mode)
    };
    assert_eq!(h, HeistRules::default());
    assert_eq!(
        (
            h.teams,
            h.match_secs,
            h.deliveries_to_win,
            h.carrier_speed_percent
        ),
        (2, 720.0, 3, 75)
    );
    assert_eq!(
        (h.drop_return_secs, h.core_respawn_secs, h.collapse_secs),
        (20.0, 15.0, 30.0)
    );
    r.resolve(&WeaponTable::builtin()).unwrap();
    let code = r.to_code().unwrap();
    assert!(code.starts_with("LR"), "{code}");
    assert_eq!(
        RuleSet::from_code(&code).unwrap(),
        crate::rulecode::canonical(&r)
    );
    assert!(crate::modes::ModeModule::implemented(&super::Heist));
}

#[test]
fn the_level_places_the_core_in_its_vein_and_one_zone_per_team() {
    let mut w = world(2);
    let mut t = room();
    w.add_player(None, ShipType::Tinker).unwrap();
    step(&mut w, &mut t, 1);
    let c = core(&w);
    assert_eq!(
        (c.kind, c.pos, c.state),
        (objective_kind::CORE, VEIN, core_state::EMBEDDED)
    );
    assert_eq!(w.mode().score_target, 3);
    let zones: Vec<(Vec3, f32, u8)> = w
        .mode()
        .objectives
        .iter()
        .filter(|o| o.kind == objective_kind::EXTRACTION)
        .map(|o| (o.pos, o.radius, o.team))
        .collect();
    assert_eq!(zones, vec![(ZONE0, ZONE_R, 0), (ZONE1, ZONE_R, 1)]);
    // Replicated: the view carries the core and both zones.
    assert_eq!(w.mode_view().objectives.len(), 3);
    // Free-for-all: the zones belong to nobody.
    let mut f = world(0);
    f.add_player(None, ShipType::Tinker).unwrap();
    step(&mut f, &mut t, 1);
    assert!(f
        .mode()
        .objectives
        .iter()
        .filter(|o| o.kind == objective_kind::EXTRACTION)
        .all(|o| o.team == NO_SIDE));
}

fn material(w: &mut World, t: &mut Room, p: Vec3) -> u8 {
    w.hook_for_test(t, |ctx| ctx.material_at(p)).0
}

#[test]
fn the_vein_is_hard_rock_and_mining_it_frees_the_core_and_collapses_the_mine() {
    let mut w = world(2);
    let mut t = room();
    w.add_player(None, ShipType::Tinker).unwrap();
    step(&mut w, &mut t, 1);
    let knot_surface = VEIN + Vec3::new(KNOT, 0.0, 0.0);
    let chamber = VEIN + Vec3::new(6.0, 0.0, 0.0);
    assert_eq!(material(&mut w, &mut t, knot_surface), MATERIAL_HARD);
    assert_eq!(material(&mut w, &mut t, chamber), 1, "ordinary rock");
    // A weapon crater on the knot is scaled down (hard rock 0.6x).
    let n0 = t.inner.edits().len();
    w.hook_for_test(&mut t, |ctx| ctx.crater(knot_surface, 1.0));
    match t.inner.edits()[n0] {
        TerrainEdit::SubtractSphere { radius, .. } => assert!((radius - 0.6).abs() < 1e-6),
        e => panic!("{e:?}"),
    }
    step(&mut w, &mut t, 5);
    assert_eq!(
        core(&w).state,
        core_state::EMBEDDED,
        "a dent does not free it"
    );
    // Mined out: free, and the mine collapses.
    mine(&mut w, &mut t);
    step(&mut w, &mut t, 1);
    let freed = w.tick();
    assert_eq!(core(&w).state, core_state::FREE);
    assert_eq!(w.mode().vars[0] as u64, freed + 30 * 60);
    assert_eq!(material(&mut w, &mut t, chamber), MATERIAL_WEAK);
    // The ceiling sheds: a crater and a debris chunk every half second.
    let ev = step(&mut w, &mut t, 4 * SHED_INTERVAL_TICKS);
    let sheds = ev
        .iter()
        .filter(|e| matches!(e, Event::TerrainEdit(TerrainEdit::SubtractSphere { .. })))
        .count();
    assert_eq!(sheds, 4, "one shed per interval");
    assert!(
        w.entities().iter().any(|e| e.kind == EntityKind::Debris),
        "debris falls"
    );
    // After `collapse_secs` the mine settles: no more weakness or shedding.
    let left = freed + 30 * 60 - w.tick();
    step(&mut w, &mut t, left);
    assert_eq!(material(&mut w, &mut t, chamber), 1);
    let ev = step(&mut w, &mut t, 2 * SHED_INTERVAL_TICKS);
    assert!(!ev
        .iter()
        .any(|e| matches!(e, Event::TerrainEdit(TerrainEdit::SubtractSphere { .. }))));
}

#[test]
fn touch_takes_the_core_and_the_carrier_is_slow_cannot_boost_and_is_shown() {
    let (mut w, mut t, ids) = carried(2);
    let c = core(&w);
    assert_eq!(c.state, core_state::CARRIED);
    assert_eq!(c.team, 0);
    assert!(w.mode().player(ids[0]).unwrap().flags & player_flags::CARRIER != 0);
    let view = w.mode_view();
    let row = view.players.iter().find(|p| p.player == ids[0]).unwrap();
    assert!(row.flags & player_flags::CARRIER != 0, "shown to everyone");
    assert_eq!(view.objectives[0].holder, Some(ids[0]));
    // Full thrust with boost held: the carrier keeps to 75 % of cruise
    // speed and never boosts; a free ship goes faster.
    teleport(&mut w, ids[0], Vec3::new(-20.0, 10.0, -20.0));
    teleport(&mut w, ids[1], Vec3::new(20.0, 10.0, -20.0));
    let cruise = w.tuning_for(ShipType::Tinker).cruise_speed;
    let boost_cmd = |tick| InputCmd {
        tick,
        thrust: Vec3::new(0.0, 0.0, 1.0),
        buttons: Buttons::BOOST,
        ..InputCmd::default()
    };
    let (mut carrier_max, mut free_max) = (0.0f32, 0.0f32);
    for _ in 0..90 {
        let now = w.tick() + 1;
        w.step(
            &mut t,
            &[(ids[0], boost_cmd(now)), (ids[1], boost_cmd(now))],
        );
        let (a, b) = (w.ship(ids[0]).unwrap(), w.ship(ids[1]).unwrap());
        assert!(!a.boosting, "the carrier cannot boost");
        carrier_max = carrier_max.max(a.vel.length());
        free_max = free_max.max(b.vel.length());
    }
    assert!(
        carrier_max <= 0.75 * cruise + 1e-3,
        "{carrier_max} vs {cruise}"
    );
    assert!(carrier_max > 0.7 * cruise, "{carrier_max}");
    assert!(free_max > cruise, "{free_max}");
    // The core follows its carrier.
    assert_eq!(core(&w).pos, w.ship(ids[0]).unwrap().pos);
}

#[test]
fn a_dead_carrier_drops_the_core_which_returns_to_the_vein() {
    let (mut w, mut t, ids) = carried(2);
    let at = Vec3::new(5.0, 2.0, 5.0);
    teleport(&mut w, ids[0], at);
    step(&mut w, &mut t, 1);
    kill(&mut w, &mut t, ids[0]);
    let c = core(&w);
    assert_eq!((c.state, c.holder, c.pos), (core_state::DROPPED, None, at));
    assert_eq!(c.timer_end, w.tick() + 20 * 60);
    assert_eq!(
        w.mode().player(ids[0]).unwrap().flags & player_flags::CARRIER,
        0
    );
    // Loose: anyone may take it (an enemy here).
    let mut w2 = w.clone();
    let mut t2 = t.clone();
    teleport(&mut w2, ids[1], at + Vec3::new(0.0, 0.5, 0.0));
    step(&mut w2, &mut t2, 1);
    assert_eq!(core(&w2).holder, Some(ids[1]));
    assert_eq!(core(&w2).team, 1);
    // Untouched for 20 s: back in the (mined) vein, free. Everyone waits
    // far away (the dead carrier respawns at its spawn).
    let drop_tick = w.tick();
    while w.tick() < drop_tick + 20 * 60 - 1 {
        for id in &ids {
            if let Some(s) = w.ship_mut(*id) {
                s.pos = Vec3::new(0.0, 12.0, 20.0 + id.0 as f32);
            }
        }
        step(&mut w, &mut t, 1);
    }
    assert_eq!(core(&w).state, core_state::DROPPED);
    step(&mut w, &mut t, 1);
    assert_eq!((core(&w).state, core(&w).pos), (core_state::FREE, VEIN));
}

#[test]
fn a_carrier_that_leaves_drops_the_core() {
    let (mut w, mut t, ids) = carried(2);
    assert!(w.remove_player(ids[0]));
    step(&mut w, &mut t, 1);
    assert_eq!(core(&w).state, core_state::DROPPED);
    assert_eq!(core(&w).holder, None);
}

#[test]
fn delivery_scores_for_the_team_and_the_core_respawns_embedded() {
    let (mut w, mut t, ids) = carried(2);
    // The enemy's zone does not count.
    teleport(&mut w, ids[0], ZONE1);
    step(&mut w, &mut t, 1);
    assert_eq!(core(&w).state, core_state::CARRIED);
    assert_eq!(w.mode().team_scores[..2], [0, 0]);
    // The own zone does.
    teleport(&mut w, ids[0], ZONE0 + Vec3::new(ZONE_R - 0.5, 0.0, 0.0));
    step(&mut w, &mut t, 1);
    let c = core(&w);
    assert_eq!(
        (c.state, c.holder, c.pos),
        (core_state::RESPAWNING, None, VEIN)
    );
    assert_eq!(c.timer_end, w.tick() + 15 * 60);
    assert_eq!(w.mode().team_scores[..2], [1, 0]);
    assert_eq!(w.mode().player(ids[0]).unwrap().score, 1);
    assert_eq!(
        w.mode().player(ids[0]).unwrap().flags & player_flags::CARRIER,
        0
    );
    assert_eq!(w.mode_view().team_scores, vec![1, 0]);
    // A ship in the vein delays the regrowth...
    teleport(&mut w, ids[2], VEIN);
    step(&mut w, &mut t, 15 * 60);
    assert_eq!(core(&w).state, core_state::RESPAWNING);
    assert!(t.sdf(VEIN) > 0.0);
    // ...until it leaves: the knot grows back and holds the core again.
    teleport(&mut w, ids[2], Vec3::new(0.0, 10.0, 20.0));
    step(&mut w, &mut t, 2);
    assert_eq!(core(&w).state, core_state::EMBEDDED);
    assert!(t.sdf(VEIN) <= -KNOT + 1e-3, "{}", t.sdf(VEIN));
    // Nothing to take while embedded.
    teleport(&mut w, ids[1], VEIN + Vec3::new(0.0, 0.0, KNOT + 0.6));
    step(&mut w, &mut t, 5);
    assert_eq!(core(&w).state, core_state::EMBEDDED);
}

#[test]
fn kills_do_not_score() {
    let mut w = world(2);
    let mut t = room();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    step(&mut w, &mut t, 1);
    let src = DamageSource {
        attacker: Some(a),
        team: 0,
        weapon: None,
    };
    w.hook_for_test(&mut t, |ctx| ctx.damage(b, 10_000.0, Vec3::ZERO, src, true));
    assert!(!w.ship(b).unwrap().alive);
    assert_eq!(w.ship(a).unwrap().kills, 1);
    assert_eq!(w.mode().player(a).unwrap().score, 0);
    assert_eq!(w.mode().team_scores[..2], [0, 0]);
}

/// Delivers the (free, knot-less) core with player `id` at `zone`.
fn deliver(w: &mut World, t: &mut Room, id: PlayerId, zone: Vec3) {
    if core(w).state == core_state::RESPAWNING {
        let wait = core(w).timer_end.saturating_sub(w.tick()) + 1;
        step(w, t, wait);
    }
    let c = core(w);
    teleport(w, id, c.pos);
    step(w, t, 1);
    assert_eq!(core(w).holder, Some(id));
    teleport(w, id, zone);
    step(w, t, 1);
    assert!(
        core(w).state == core_state::RESPAWNING || !w.mode().is_running(),
        "{:?}",
        core(w)
    );
}

fn zone_of(w: &World, team: u8) -> Vec3 {
    w.mode()
        .objectives
        .iter()
        .find(|o| o.kind == objective_kind::EXTRACTION && o.team == team)
        .unwrap()
        .pos
}

#[test]
fn first_team_to_three_deliveries_wins() {
    // No generated site: the core lies free between the spawns (fallback).
    let mut w = world_with(rules(2), false);
    let mut t = room();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    step(&mut w, &mut t, 1);
    assert_eq!(core(&w).state, core_state::FREE, "fallback: no rock");
    let (za, zb) = (zone_of(&w, 0), zone_of(&w, 1));
    deliver(&mut w, &mut t, a, za);
    deliver(&mut w, &mut t, b, zb);
    deliver(&mut w, &mut t, a, za);
    assert_eq!(w.mode().phase, MatchPhase::Running);
    deliver(&mut w, &mut t, a, za);
    assert_eq!(w.mode().team_scores[..2], [3, 1]);
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(0) });
    let res = w.results();
    assert_eq!(res.winner, Some(0));
    assert_eq!(res.rows[0].player, a);
    assert_eq!(res.rows[0].score, 3);
}

#[test]
fn most_deliveries_win_on_time_and_a_tie_is_a_draw() {
    let mut r = rules(2);
    r.mode.set_field("match_secs", 60.0);
    for (deliveries, winner) in [((1, 0), Some(0u8)), ((1, 1), None)] {
        let mut w = world_with(r.clone(), false);
        let mut t = room();
        let a = w.add_player(None, ShipType::Tinker).unwrap();
        let b = w.add_player(None, ShipType::Tinker).unwrap();
        step(&mut w, &mut t, 1);
        let (za, zb) = (zone_of(&w, 0), zone_of(&w, 1));
        for _ in 0..deliveries.0 {
            deliver(&mut w, &mut t, a, za);
        }
        for _ in 0..deliveries.1 {
            deliver(&mut w, &mut t, b, zb);
        }
        step(&mut w, &mut t, 60 * 60);
        assert_eq!(
            w.mode().phase,
            MatchPhase::Ended { winner },
            "{deliveries:?}"
        );
    }
}

#[test]
fn free_for_all_any_zone_scores_for_the_player() {
    let mut w = world_with(rules(0), true);
    let mut t = room();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    step(&mut w, &mut t, 1);
    mine(&mut w, &mut t);
    step(&mut w, &mut t, 1);
    teleport(&mut w, b, VEIN + Vec3::new(0.0, 1.2, 0.0));
    step(&mut w, &mut t, 1);
    assert_eq!(core(&w).holder, Some(b));
    teleport(&mut w, b, ZONE0);
    step(&mut w, &mut t, 1);
    assert_eq!(w.mode().player(b).unwrap().score, 1);
    assert_eq!(w.mode().player(a).unwrap().score, 0);
    assert!(w.mode_view().team_scores.is_empty());
}

#[test]
fn rules_are_validated() {
    let weapons = WeaponTable::builtin();
    for (field, v) in [
        ("deliveries_to_win", 0.0),
        ("carrier_speed_percent", 0.0),
        ("carrier_speed_percent", 101.0),
    ] {
        let mut r = rules(2);
        r.mode.set_field(field, v);
        assert!(r.resolve(&weapons).is_err(), "{field} = {v}");
    }
}

// ----- scripted bots ---------------------------------------------------------

/// A minimal Heist pilot for in-process tests: flies straight at its goal
/// (ships keep their spawn orientation, facing -Z), mines the vein with
/// Bolt Rockets from its +Z side, takes the core and carries it home;
/// enemies of a carrier chase it with the laser.
fn bot_input(w: &World, id: PlayerId, slot: usize) -> InputCmd {
    let now = w.tick() + 1;
    let (Some(ship), Some(c)) = (
        w.ship(id).filter(|s| s.alive),
        w.mode().objective(CORE_ID).copied(),
    ) else {
        return InputCmd::idle(now);
    };
    let home = w
        .mode()
        .objectives
        .iter()
        .find(|o| {
            o.kind == objective_kind::EXTRACTION && (o.team == ship.team || o.team == NO_SIDE)
        })
        .map_or(Vec3::ZERO, |o| o.pos);
    let vein = Vec3::new(c.vars[0], c.vars[1], c.vars[2]);
    let lane = Vec3::new(
        (slot % 3) as f32 * 0.6 - 0.6,
        (slot / 3 % 3) as f32 * 0.6 - 0.6,
        0.0,
    );
    let mut buttons = Buttons::default();
    let goal = match c.state {
        core_state::CARRIED if c.holder == Some(id) => home,
        core_state::CARRIED => match c.holder.and_then(|h| w.ship(h)) {
            Some(s) if s.team != ship.team => {
                buttons = buttons | Buttons::FIRE_PRIMARY;
                s.pos + Vec3::new(0.0, 0.0, 4.0)
            }
            _ => home,
        },
        core_state::FREE | core_state::DROPPED => c.pos,
        core_state::EMBEDDED => {
            let spot = vein + lane + Vec3::new(0.0, 0.0, KNOT + 3.0);
            if ship.pos.distance(spot) < 1.0 {
                buttons = buttons | Buttons::FIRE_SECONDARY;
            }
            spot
        }
        _ => vein + lane * 4.0 + Vec3::new(0.0, 6.0, 8.0),
    };
    // Desired velocity towards the goal, as body-frame thrust.
    let to = goal - ship.pos;
    let want = if to.length() > 8.0 {
        to.normalize() * 8.0
    } else {
        to
    };
    let local = ship.orientation.conjugate().rotate((want - ship.vel) * 0.5);
    InputCmd {
        tick: now,
        thrust: local.clamp(Vec3::splat(-1.0), Vec3::splat(1.0)),
        buttons,
        ..InputCmd::default()
    }
}

/// Bots on `w`'s level: refills Bolt Rockets every 10 s (standing in for
/// pickups); returns the state hash once per second.
fn run_bots(w: &mut World, t: &mut Room, ids: &[PlayerId], ticks: u64) -> Vec<u64> {
    let rocket = w.weapons().require("bolt_rocket").unwrap();
    let mut hashes = Vec::new();
    for _ in 0..ticks {
        if !w.mode().is_running() {
            break;
        }
        if w.tick() % 600 == 0 {
            for id in ids {
                if let Some(s) = w.ship_mut(*id) {
                    for ws in &mut s.weapons {
                        if ws.weapon == rocket {
                            ws.ammo = ws.ammo.max(3);
                        }
                    }
                }
            }
        }
        let inputs: Vec<(PlayerId, InputCmd)> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, bot_input(w, *id, i)))
            .collect();
        w.step(t, &inputs);
        if w.tick() % 60 == 0 {
            hashes.push(w.state_hash());
        }
    }
    hashes
}

/// The default rule set runs a full match of 8 scripted bots to a result:
/// they mine the vein with real rockets, carry the core home and score.
#[test]
fn eight_bots_play_a_full_match_to_a_result() {
    let mut w = world_with(RuleSet::default_for(ModeKind::Heist), true);
    let mut t = room();
    let ids: Vec<PlayerId> = (0..8)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    run_bots(&mut w, &mut t, &ids, 12 * 60 * 60 + 1);
    let res = w.results();
    assert!(res.ended, "no result after 12 minutes: {:?}", w.mode_view());
    assert_eq!(
        res.team_scores.iter().max(),
        Some(&3),
        "{:?}",
        res.team_scores
    );
    assert!(res.winner.is_some());
    assert_eq!(res.rows.len(), 8);
    println!("8 bots: {:?} after {} ticks", res.team_scores, w.tick());
}

/// Golden hash of a 60 s, 8-bot Heist replay (mining, collapse, carrying,
/// deliveries; the match ends by deliveries inside the 60 s). Checked on native and wasm32; re-bless after a deliberate
/// change of Heist or the simulation (the test prints the new value).
const HEIST_REPLAY_GOLDEN_HASH: u64 = 0x4348_368f_eb4b_1f43;

#[test]
fn heist_replay_golden_hash() {
    let run = || {
        let mut w = world(2);
        let mut t = room();
        let ids: Vec<PlayerId> = (0..8)
            .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
            .collect();
        let h = run_bots(&mut w, &mut t, &ids, 60 * 60);
        (h, core(&w), w.mode().team_scores)
    };
    let (a, core_a, scores) = run();
    let (b, _, _) = run();
    assert_eq!(a, b, "deterministic");
    assert_eq!(
        scores[..2],
        [0, 3],
        "the replay covers mining and deliveries"
    );
    assert_eq!(core_a.state, core_state::RESPAWNING);
    let last = *a.last().unwrap();
    println!("heist replay hash {last:#018x}, core {core_a:?}, scores {scores:?}");
    assert_eq!(
        last, HEIST_REPLAY_GOLDEN_HASH,
        "heist replay hash {last:#018x}"
    );
}
