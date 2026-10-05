//! Last Color Standing scenario tests (one per core rule of
//! `docs/milestone-3-plan.md` §3.2), the default rule set and the 60 s
//! replay golden hash.

use ls_math::terrain::BoxRoomTerrain;
use ls_math::{Pcg32, Quat, Vec3};

use super::{STAGE_NORMAL, STAGE_STALEMATE};
use crate::mode::{player_flags, view_flags, MatchPhase, ModeKind};
use crate::modes::{module, LATE_JOIN_GRACE_TICKS};
use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

type Room = PaintedTerrain<BoxRoomTerrain>;

/// Eight spawn points around the room's centre.
fn setup() -> MatchSetup {
    let mut spawns = Vec::new();
    for team in 0..4u8 {
        for k in 0..2 {
            let a = team as f32 * 1.5 + k as f32 * 0.4;
            spawns.push(SpawnPoint {
                pos: Vec3::new(
                    20.0 * ls_math::det::cos(a),
                    k as f32 * 4.0,
                    20.0 * ls_math::det::sin(a),
                ),
                team,
                orientation: Quat::IDENTITY,
            });
        }
    }
    MatchSetup {
        spawns,
        pickup_points: (0..8)
            .map(|i| Vec3::new(-14.0 + 4.0 * i as f32, -3.0, 0.0))
            .collect(),
        instability: 0,
    }
}

fn room() -> Room {
    PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)), 1.0)
}

fn rules() -> RuleSet {
    RuleSet::default_for(ModeKind::LastColorStanding)
}

fn world_with(rules: RuleSet, seed: u64) -> World {
    let mut cfg = WorldConfig::paint_the_void(seed, setup());
    cfg.rules = rules;
    World::new(cfg).unwrap()
}

fn world() -> World {
    world_with(rules(), 7)
}

fn players(w: &mut World, n: usize) -> Vec<PlayerId> {
    (0..n)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect()
}

fn colour(w: &World, id: PlayerId) -> u8 {
    w.mode().player(id).unwrap().meter
}

/// `from` hits `to` for `amount` (direct hit).
fn hit(w: &mut World, t: &mut Room, from: PlayerId, to: PlayerId, amount: f32) -> Vec<Event> {
    let src = DamageSource {
        attacker: Some(from),
        team: w.ship(from).unwrap().team,
        weapon: None,
    };
    w.hook_for_test(t, |ctx| ctx.damage(to, amount, Vec3::ZERO, src, true))
        .1
}

/// Kills `victim` without an attacker (environment / suicide).
fn kill_env(w: &mut World, t: &mut Room, victim: PlayerId) {
    w.hook_for_test(t, |ctx| {
        ctx.push(Command::AdjustHull {
            target: victim,
            delta: -10_000.0,
            can_kill: true,
        })
    });
}

/// Steps until `id`'s ship is alive again (or `max` ticks passed).
fn until_respawned(w: &mut World, t: &mut Room, id: PlayerId, max: u64) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..max {
        ev.extend(w.step(t, &[]));
        if w.ship(id).unwrap().alive {
            break;
        }
    }
    ev
}

#[test]
fn the_module_is_implemented_and_free_for_all_only() {
    let m = module(ModeKind::LastColorStanding);
    assert!(m.implemented());
    assert_eq!(m.team_counts(), &[0]);
    let weapons = WeaponTable::builtin();
    for teams in [2u8, 3, 4] {
        let mut r = rules();
        r.mode.set_field("teams", teams as f32);
        assert!(r.resolve(&weapons).is_err(), "{teams} teams");
    }
    for (field, value) in [
        ("colour", 0.0),
        ("drain_per_kill", 0.0),
        ("stalemate_drain_secs", 0.0),
    ] {
        let mut r = rules();
        r.mode.set_field(field, value);
        assert!(r.resolve(&weapons).is_err(), "{field} = {value}");
    }
}

#[test]
fn players_start_with_full_colour_and_the_view_is_monochrome() {
    let mut w = world();
    let ids = players(&mut w, 3);
    assert_eq!(w.teams(), 0);
    for &id in &ids {
        assert_eq!(colour(&w, id), 100);
        assert_eq!(w.mode().player(id).unwrap().score, 100);
    }
    let v = w.mode_view();
    assert_eq!(v.kind, ModeKind::LastColorStanding);
    assert!(v.flags & view_flags::MONOCHROME != 0);
    assert!(v.flags & view_flags::METER != 0);
    assert!(v.flags & view_flags::LIVES == 0, "no lives");
    assert!(v.players.iter().all(|p| p.meter == 100));
}

#[test]
fn a_kill_drains_the_victims_colour_and_damage_alone_does_not() {
    let mut w = world();
    let mut t = room();
    let ids = players(&mut w, 3);
    let (a, b) = (ids[0], ids[1]);
    w.step(&mut t, &[]);
    // Damage that does not kill: no drain.
    hit(&mut w, &mut t, a, b, 10.0);
    assert!(w.ship(b).unwrap().alive);
    assert_eq!(colour(&w, b), 100);
    // A kill drains the victim by 34; the killer keeps its colour.
    let ev = hit(&mut w, &mut t, a, b, 10_000.0);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Killed { victim, .. } if *victim == b)));
    assert_eq!(colour(&w, b), 66);
    assert_eq!(colour(&w, a), 100);
    assert_eq!(w.ship(a).unwrap().kills, 1);
    // Mode score mirrors the colour.
    assert_eq!(w.mode().player(b).unwrap().score, 66);
    // A death without a killer drains too.
    until_respawned(&mut w, &mut t, b, 600);
    kill_env(&mut w, &mut t, b);
    assert_eq!(colour(&w, b), 32);
    // The colour is part of the state hash.
    let h = w.state_hash();
    w.mode_mut().player_mut(b).unwrap().meter = 31;
    assert_ne!(w.state_hash(), h);
}

#[test]
fn three_deaths_eliminate_and_the_ship_never_respawns() {
    let mut w = world();
    let mut t = room();
    let ids = players(&mut w, 3);
    let (a, b) = (ids[0], ids[1]);
    w.step(&mut t, &[]);
    let mut ev = Vec::new();
    for death in 1..=3 {
        assert!(w.ship(b).unwrap().alive, "death {death}");
        ev.extend(hit(&mut w, &mut t, a, b, 10_000.0));
        if death < 3 {
            ev.extend(until_respawned(&mut w, &mut t, b, 600));
            assert!(!w.is_spectator(b));
        }
    }
    assert_eq!(colour(&w, b), 0, "100 - 3 x 34 saturates at 0");
    ev.extend(w.step(&mut t, &[]));
    assert!(ev.contains(&Event::Eliminated { player: b }));
    assert!(w.is_spectator(b));
    let p = w.mode().player(b).unwrap();
    assert!(p.flags & player_flags::ELIMINATED != 0);
    // No respawn, ever; the other two keep playing.
    for _ in 0..600 {
        w.step(&mut t, &[]);
    }
    assert!(!w.ship(b).unwrap().alive);
    assert!(w.mode().is_running(), "two players still have colour");
    // Asking to play again changes nothing.
    let _ = w.set_spectator(b, false);
    for _ in 0..300 {
        w.step(&mut t, &[]);
    }
    assert!(!w.ship(b).unwrap().alive && w.is_spectator(b));
}

#[test]
fn the_last_player_with_colour_wins() {
    let mut w = world();
    let mut t = room();
    let ids = players(&mut w, 3);
    w.step(&mut t, &[]);
    // ids[0] has some colour drained but survives.
    hit(&mut w, &mut t, ids[1], ids[0], 10_000.0);
    until_respawned(&mut w, &mut t, ids[0], 600);
    // ids[1] and ids[2] run out of colour.
    for &victim in &ids[1..] {
        w.mode_mut().player_mut(victim).unwrap().meter = 1;
    }
    hit(&mut w, &mut t, ids[0], ids[1], 10_000.0);
    let ev = w.step(&mut t, &[]);
    assert!(ev.contains(&Event::Eliminated { player: ids[1] }));
    assert!(w.mode().is_running());
    kill_env(&mut w, &mut t, ids[2]);
    let ev = w.step(&mut t, &[]);
    let side = w.ship(ids[0]).unwrap().team;
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::MatchEnded { winner: Some(s), .. } if *s == side
    )));
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(side) });
    let v = w.mode_view();
    assert_eq!(v.winner_player().unwrap().player, ids[0]);
    assert_eq!(v.players[0].player, ids[0], "most colour first");
    let res = w.results();
    assert_eq!(res.winner, Some(side));
    assert_eq!(res.rows[0].player, ids[0]);
    assert_eq!(res.rows[0].score, 66);
    // Eliminated players: 0 colour, the one with a kill ranks higher.
    assert_eq!(res.rows[1].player, ids[1]);
    assert_eq!((res.rows[1].score, res.rows[2].score), (0, 0));
}

/// Short stalemate rules for the drain tests: starts after 2 s, 1 colour
/// every 0.5 s.
fn quick_stalemate(colour: u8) -> RuleSet {
    let mut r = rules();
    r.mode.set_field("colour", colour as f32);
    r.mode.set_field("stalemate_secs", 2.0);
    r.mode.set_field("stalemate_drain_secs", 0.5);
    r
}

#[test]
fn the_stalemate_drains_everyone_one_point_per_interval() {
    // Default numbers: 8 minutes, then 1 colour per 2 s.
    let mut w = world();
    let mut t = room();
    let ids = players(&mut w, 2);
    w.step(&mut t, &[]);
    assert_eq!(w.mode().stage, STAGE_NORMAL);
    assert_eq!(w.mode().stage_end_tick, 480 * 60);
    while w.tick() < 480 * 60 {
        w.step(&mut t, &[]);
    }
    assert_eq!(w.mode().stage, STAGE_STALEMATE);
    assert!(ids.iter().all(|&id| colour(&w, id) == 100), "not yet");
    for _ in 0..119 {
        w.step(&mut t, &[]);
    }
    assert!(
        ids.iter().all(|&id| colour(&w, id) == 100),
        "after 119 ticks"
    );
    w.step(&mut t, &[]);
    assert!(ids.iter().all(|&id| colour(&w, id) == 99), "after 2 s");
    // Dead ships drain too (players still in play, not ships alive).
    kill_env(&mut w, &mut t, ids[0]);
    assert_eq!(colour(&w, ids[0]), 65);
    for _ in 0..120 {
        w.step(&mut t, &[]);
    }
    assert_eq!(colour(&w, ids[0]), 64);
    assert_eq!(colour(&w, ids[1]), 98);
}

#[test]
fn the_stalemate_always_ends_the_match_and_takes_living_ships_out() {
    let mut w = world_with(quick_stalemate(5), 7);
    let mut t = room();
    let ids = players(&mut w, 3);
    w.step(&mut t, &[]);
    // ids[2] has more colour: the others drain out first, alive.
    w.mode_mut().player_mut(ids[2]).unwrap().meter = 8;
    let mut ev = Vec::new();
    for _ in 0..60 * 60 {
        ev.extend(w.step(&mut t, &[]));
        if !w.mode().is_running() {
            break;
        }
    }
    // 2 s + 5 x 0.5 s = 4.5 s: ids[0] and ids[1] are out, ids[2] wins.
    assert_eq!(w.tick(), 270);
    for &id in &ids[..2] {
        assert!(ev.contains(&Event::Eliminated { player: id }));
        assert!(!w.ship(id).unwrap().alive, "drained ships leave play");
        assert_eq!(w.ship(id).unwrap().deaths, 0, "without a death");
    }
    let side = w.ship(ids[2]).unwrap().team;
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(side) });
    assert_eq!(colour(&w, ids[2]), 3);
}

#[test]
fn the_last_two_drained_in_the_same_tick_is_a_draw() {
    let mut w = world_with(quick_stalemate(3), 7);
    let mut t = room();
    let ids = players(&mut w, 2);
    let mut ended = None;
    for _ in 0..60 * 60 {
        for e in w.step(&mut t, &[]) {
            if let Event::MatchEnded { winner, .. } = e {
                ended = Some((w.tick(), winner));
            }
        }
        if ended.is_some() {
            break;
        }
    }
    assert_eq!(ended, Some((120 + 3 * 30, None)));
    assert!(ids.iter().all(|&id| w.is_spectator(id)));
}

#[test]
fn on_the_timer_the_most_colour_wins_and_a_tie_is_a_draw() {
    let mut r = rules();
    r.mode.set_field("match_secs", 5.0);
    let mut w = world_with(r.clone(), 7);
    let mut t = room();
    let ids = players(&mut w, 3);
    w.step(&mut t, &[]);
    hit(&mut w, &mut t, ids[0], ids[1], 10_000.0);
    hit(&mut w, &mut t, ids[0], ids[2], 10_000.0);
    hit(&mut w, &mut t, ids[1], ids[0], 10_000.0);
    // Everyone died once: 66 each, a tie.
    while w.mode().is_running() {
        w.step(&mut t, &[]);
    }
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: None });
    let mut w = world_with(r, 7);
    let ids = players(&mut w, 3);
    w.step(&mut t, &[]);
    hit(&mut w, &mut t, ids[0], ids[1], 10_000.0);
    hit(&mut w, &mut t, ids[1], ids[2], 10_000.0);
    while w.mode().is_running() {
        w.step(&mut t, &[]);
    }
    let side = w.ship(ids[0]).unwrap().team;
    assert_eq!(w.tick(), 5 * 60);
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(side) });
}

#[test]
fn late_joiners_spectate_until_the_next_match() {
    let mut w = world();
    let mut t = room();
    let early = players(&mut w, 2);
    // Inside the grace period: a normal join with full colour.
    for _ in 0..LATE_JOIN_GRACE_TICKS {
        w.step(&mut t, &[]);
    }
    let grace = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(w.ship(grace).unwrap().alive && colour(&w, grace) == 100);
    w.step(&mut t, &[]);
    let late = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(!w.ship(late).unwrap().alive && w.is_spectator(late));
    assert!(w.set_spectator(late, false).is_err());
    for _ in 0..300 {
        w.step(&mut t, &[]);
    }
    assert!(!w.ship(late).unwrap().alive, "spectators never respawn");
    assert!(early.iter().all(|&id| w.ship(id).unwrap().alive));
    // A late spectator does not keep the match going on its own.
    w.mode_mut().player_mut(early[1]).unwrap().meter = 1;
    w.mode_mut().player_mut(grace).unwrap().meter = 1;
    hit(&mut w, &mut t, early[0], early[1], 10_000.0);
    hit(&mut w, &mut t, early[0], grace, 10_000.0);
    w.step(&mut t, &[]);
    let side = w.ship(early[0]).unwrap().team;
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(side) });
}

#[test]
fn the_default_rule_set_loads_resolves_and_round_trips_as_a_code() {
    let r = rules();
    assert_eq!(r.name, "Last Color Standing");
    assert_eq!(
        r.mode,
        ModeRules::LastColorStanding(LastColorStandingRules::default()),
        "the file matches the documented defaults"
    );
    let file = crate::ruleset::BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| *f == ModeKind::LastColorStanding.ruleset_file());
    assert!(file.is_some(), "embedded and hashed");
    assert!(r.availability("paint_thrower").is_banned());
    r.resolve(&WeaponTable::builtin()).unwrap();
    let code = r.to_code().unwrap();
    assert!(code.starts_with("LR"), "{code}");
    assert_eq!(RuleSet::from_code(&code).unwrap(), r);
    let full = r.to_full_code().unwrap();
    assert_eq!(RuleSet::from_code(&full).unwrap().mode, r.mode);
    // A changed setting survives the code.
    let mut changed = r.clone();
    changed.mode.set_field("drain_per_kill", 50.0);
    changed.mode.set_field("stalemate_secs", 300.0);
    let back = RuleSet::from_code(&changed.to_code().unwrap()).unwrap();
    assert_eq!(back.mode, changed.mode);
}

// ----- replay ----------------------------------------------------------------

/// Scripted pseudo-random inputs; each ship steers towards the nearest
/// enemy so fights happen. Depends only on the world state and the RNG.
fn scripted_inputs(
    rng: &mut Pcg32,
    w: &World,
    ids: &[PlayerId],
    tick: u64,
) -> Vec<(PlayerId, InputCmd)> {
    ids.iter()
        .map(|&id| {
            let (pitch, yaw) = aim_at_nearest(w, id);
            let mut buttons = Buttons::HORIZON_ASSIST;
            for (b, p) in [
                (Buttons::BOOST, 0.3),
                (Buttons::FIRE_PRIMARY, 0.6),
                (Buttons::FIRE_SECONDARY, 0.05),
                (Buttons::SCAVENGE, 0.2),
                (Buttons::GADGET, 0.03),
            ] {
                buttons.set(b, rng.chance(p));
            }
            let axis = |r: &mut Pcg32| (r.below(3) as f32) - 1.0;
            let thrust = Vec3::new(axis(rng), axis(rng), axis(rng) - 0.5);
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

fn aim_at_nearest(w: &World, id: PlayerId) -> (f32, f32) {
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

/// A scripted 60 s match of 8 ships with the default rule set, the stalemate
/// moved to 30 s with a drain every 0.25 s so both colour rules run; returns
/// the state hash every second and all events.
fn replay(seed: u64) -> (Vec<u64>, Vec<Event>, World) {
    let mut r = rules();
    r.mode.set_field("stalemate_secs", 30.0);
    r.mode.set_field("stalemate_drain_secs", 0.25);
    let mut w = world_with(r, seed);
    let mut t = room();
    let ids: Vec<PlayerId> = (0..8)
        .map(|i| {
            w.add_player(None, ShipType::ALL[i % ShipType::ALL.len()])
                .unwrap()
        })
        .collect();
    let mut rng = Pcg32::new(seed, 7);
    let mut hashes = Vec::new();
    let mut events = Vec::new();
    for tick in 1..=60 * 60 {
        let inputs = scripted_inputs(&mut rng, &w, &ids, tick);
        events.extend(w.step(&mut t, &inputs));
        if tick % 60 == 0 {
            hashes.push(w.state_hash());
        }
    }
    (hashes, events, w)
}

/// Golden hash of the Last Color Standing replay (native and wasm32 must
/// agree). Changing the mode, the default rule set or sim behaviour changes
/// it: update it in the same change.
const LCS_REPLAY_GOLDEN_HASH: u64 = 0x7bcd_a0fc_22ef_666a;

#[test]
fn replay_is_deterministic_and_matches_the_golden_hash() {
    let (h1, e1, w) = replay(4321);
    let (h2, e2, _) = replay(4321);
    assert_eq!(h1, h2);
    assert_eq!(e1, e2);
    // The replay exercises the mode: kills drain colour, the stalemate
    // drains everyone, players are eliminated.
    let kills = e1
        .iter()
        .filter(|e| matches!(e, Event::Killed { .. }))
        .count();
    assert!(kills > 0, "no kills");
    assert!(w.mode().stage == STAGE_STALEMATE);
    assert!(
        e1.iter().any(|e| matches!(e, Event::Eliminated { .. })),
        "nobody eliminated"
    );
    let last = *h1.last().unwrap();
    assert_eq!(last, LCS_REPLAY_GOLDEN_HASH, "LCS replay hash {last:#018x}");
}
