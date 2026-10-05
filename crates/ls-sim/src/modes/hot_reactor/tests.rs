//! Hot Reactor scenario tests: one per core rule of
//! `docs/milestone-3-plan.md` §3.2 (reactor appears, passing by hit and by
//! ramming, pass immunity, fuse and proximity speed-up, detonation blast and
//! lives, elimination and the winner, gap decisions), the default rule set,
//! and a 60 s replay with a golden hash (native and wasm32).

use ls_math::terrain::BoxRoomTerrain;
use ls_math::{Pcg32, Quat, Vec3};

use super::{detonations, holder, STATE_HELD, STATE_WAITING};
use crate::mode::{objective_kind, player_flags, MatchPhase, ModeKind};
use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

type Room = PaintedTerrain<BoxRoomTerrain>;

/// Eight spawn points, every pair more than 15 u (the fuse radius) apart.
fn setup() -> MatchSetup {
    let mut spawns = Vec::new();
    for i in 0..8u8 {
        let x = if i & 1 == 0 { -25.0 } else { 25.0 };
        let y = if i & 2 == 0 { -10.0 } else { 10.0 };
        let z = if i & 4 == 0 { -25.0 } else { 25.0 };
        spawns.push(SpawnPoint {
            pos: Vec3::new(x, y, z),
            team: 0,
            orientation: Quat::IDENTITY,
        });
    }
    MatchSetup {
        spawns,
        pickup_points: (0..4)
            .map(|i| Vec3::new(-6.0 + 4.0 * i as f32, 0.0, 0.0))
            .collect(),
        instability: 0,
    }
}

fn room() -> Room {
    PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(64.0, 30.0, 64.0)), 1.0)
}

fn rules() -> RuleSet {
    RuleSet::default_for(ModeKind::HotReactor)
}

fn world_with(rules: RuleSet, players: usize) -> (World, Vec<PlayerId>) {
    let mut cfg = WorldConfig::paint_the_void(11, setup());
    cfg.rules = rules;
    let mut w = World::new(cfg).unwrap();
    let ids = (0..players)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    (w, ids)
}

fn world(players: usize) -> (World, Vec<PlayerId>) {
    world_with(rules(), players)
}

fn secs(s: f32) -> u64 {
    crate::modes::ticks(s)
}

/// Steps `n` ticks with idle inputs, collecting the events.
fn run(w: &mut World, t: &mut Room, n: u64) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..n {
        ev.extend(w.step(t, &[]));
    }
    ev
}

/// Steps until the reactor is held (at most 10 s).
fn until_held(w: &mut World, t: &mut Room) -> PlayerId {
    for _ in 0..secs(10.0) {
        if let Some(h) = holder(w.mode()) {
            return h;
        }
        w.step(t, &[]);
    }
    panic!("no reactor appeared");
}

/// Puts a ship at `pos`, at rest.
fn place(w: &mut World, id: PlayerId, pos: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
}

/// `from` hits `to` for `amount`.
fn hit(w: &mut World, t: &mut Room, from: PlayerId, to: PlayerId, amount: f32) {
    let src = DamageSource {
        attacker: Some(from),
        team: w.ship(from).unwrap().team,
        weapon: None,
    };
    w.hook_for_test(t, |ctx| ctx.damage(to, amount, Vec3::ZERO, src, true));
}

fn lives(w: &World, id: PlayerId) -> u8 {
    w.mode().player(id).unwrap().lives
}

fn reactor(w: &World) -> mode::Objective {
    *w.mode().objective(super::REACTOR_ID).unwrap()
}

#[test]
fn default_rule_set_loads_resolves_and_round_trips_its_code() {
    let r = rules();
    assert_eq!(r.name, "Hot Reactor");
    assert_eq!(r.mode.kind(), ModeKind::HotReactor);
    assert_eq!(r.mode, ModeRules::default_for(ModeKind::HotReactor));
    assert_eq!(r.mode.teams(), 0);
    assert_eq!(r.mode.lives(), 3);
    assert!(r.resolve(&WeaponTable::builtin()).is_ok());
    assert_eq!(r.availability("paint_thrower"), Availability::Banned);
    let code = r.to_code().unwrap();
    assert_eq!(
        RuleSet::from_code(&code).unwrap(),
        crate::rulecode::canonical(&r)
    );
    let full = r.to_full_code().unwrap();
    assert_eq!(
        RuleSet::from_code(&full).unwrap(),
        crate::rulecode::canonical(&r)
    );
    assert!(crate::modes::module(ModeKind::HotReactor).implemented());
}

#[test]
fn invalid_settings_are_rejected() {
    let weapons = WeaponTable::builtin();
    for (field, value) in [
        ("lives", 0.0),
        ("fuse_secs", 0.0),
        ("blast_damage", -1.0),
        ("teams", 2.0),
    ] {
        let mut r = rules();
        r.mode.set_field(field, value);
        assert!(r.resolve(&weapons).is_err(), "{field} = {value}");
    }
}

#[test]
fn reactor_appears_on_a_living_player_after_three_seconds() {
    let (mut w, ids) = world(4);
    let mut t = room();
    run(&mut w, &mut t, secs(3.0));
    let o = reactor(&w);
    assert_eq!(o.kind, objective_kind::REACTOR);
    assert_eq!(o.state, STATE_WAITING);
    assert_eq!(o.timer_end, 1 + secs(3.0));
    run(&mut w, &mut t, 1);
    let h = holder(w.mode()).expect("held at 3 s");
    assert!(ids.contains(&h));
    let o = reactor(&w);
    assert_eq!(o.state, STATE_HELD);
    assert_eq!(o.pos, w.ship(h).unwrap().pos);
    assert_eq!(o.radius, 15.0);
    assert_eq!(o.timer_end, w.tick() + secs(20.0));
    // The holder carries the flag, nobody else; the view shows the marker.
    for id in &ids {
        let carrier = w.mode().player(*id).unwrap().flags & player_flags::CARRIER != 0;
        assert_eq!(carrier, *id == h);
    }
    let view = w.mode_view();
    assert_eq!(view.objectives.len(), 1);
    assert_eq!(view.objectives[0].holder, Some(h));
    // Different seeds pick different players (random, deterministic).
    let mut picked = std::collections::BTreeSet::new();
    for seed in 0..12 {
        let mut cfg = WorldConfig::paint_the_void(seed, setup());
        cfg.rules = rules();
        let mut w = World::new(cfg).unwrap();
        for _ in 0..4 {
            w.add_player(None, ShipType::Tinker).unwrap();
        }
        let mut t = room();
        picked.insert(until_held(&mut w, &mut t).0);
    }
    assert!(picked.len() >= 3, "{picked:?}");
}

#[test]
fn no_reactor_with_a_single_player() {
    let (mut w, ids) = world(1);
    let mut t = room();
    run(&mut w, &mut t, secs(6.0));
    assert_eq!(holder(w.mode()), None);
    // A second player arrives: the reactor appears right away.
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    run(&mut w, &mut t, 1);
    assert!([ids[0], b].contains(&holder(w.mode()).unwrap()));
}

#[test]
fn a_hit_on_the_holder_passes_it_to_the_shooter_with_immunity() {
    let (mut w, ids) = world(3);
    let mut t = room();
    let a = until_held(&mut w, &mut t);
    let others: Vec<PlayerId> = ids.iter().copied().filter(|&i| i != a).collect();
    let (b, c) = (others[0], others[1]);
    // B shoots the holder: B holds it now.
    hit(&mut w, &mut t, b, a, 5.0);
    assert_eq!(holder(w.mode()), Some(b));
    assert!(w.mode().player(b).unwrap().flags & player_flags::CARRIER != 0);
    assert!(w.mode().player(a).unwrap().flags & player_flags::CARRIER == 0);
    // A shoots back at once: B is immune for 1 s (nor can C take it).
    hit(&mut w, &mut t, a, b, 5.0);
    hit(&mut w, &mut t, c, b, 5.0);
    assert_eq!(holder(w.mode()), Some(b));
    run(&mut w, &mut t, secs(1.0) - 1);
    hit(&mut w, &mut t, a, b, 5.0);
    assert_eq!(holder(w.mode()), Some(b), "still immune");
    run(&mut w, &mut t, 1);
    hit(&mut w, &mut t, a, b, 5.0);
    assert_eq!(holder(w.mode()), Some(a), "passed back after 1 s");
    // The holder hitting itself, or the environment, passes nothing.
    run(&mut w, &mut t, secs(1.0));
    hit(&mut w, &mut t, a, a, 1.0);
    w.hook_for_test(&mut t, |ctx| {
        ctx.damage(a, 1.0, Vec3::ZERO, DamageSource::ENVIRONMENT, true)
    });
    assert_eq!(holder(w.mode()), Some(a));
    // A shield that absorbs the hit still lets it pass.
    w.give_gadget(a, "shield").unwrap();
    w.ship_mut(a).unwrap().gadget_active = true;
    let hull = w.ship(a).unwrap().hull;
    hit(&mut w, &mut t, c, a, 1.0);
    assert_eq!(holder(w.mode()), Some(c));
    assert!(w.ship(a).unwrap().hull >= hull - 1.0);
}

#[test]
fn ramming_passes_it_on() {
    let (mut w, ids) = world(3);
    let mut t = room();
    let a = until_held(&mut w, &mut t);
    let b = *ids.iter().find(|&&i| i != a).unwrap();
    // Just apart (radius 0.5 each): nothing happens.
    place(&mut w, a, Vec3::new(0.0, 0.0, 0.0));
    place(&mut w, b, Vec3::new(1.05, 0.0, 0.0));
    run(&mut w, &mut t, 1);
    assert_eq!(holder(w.mode()), Some(a));
    // Touching: B takes it, and can't give it straight back while touching.
    place(&mut w, b, Vec3::new(0.9, 0.0, 0.0));
    run(&mut w, &mut t, 1);
    assert_eq!(holder(w.mode()), Some(b));
    for _ in 0..secs(1.0) - 1 {
        place(&mut w, a, Vec3::new(0.0, 0.0, 0.0));
        place(&mut w, b, Vec3::new(0.9, 0.0, 0.0));
        run(&mut w, &mut t, 1);
        assert_eq!(holder(w.mode()), Some(b), "immune at tick {}", w.tick());
    }
    place(&mut w, a, Vec3::new(0.0, 0.0, 0.0));
    place(&mut w, b, Vec3::new(0.9, 0.0, 0.0));
    run(&mut w, &mut t, 1);
    assert_eq!(holder(w.mode()), Some(a), "passed back after 1 s");
}

#[test]
fn fuse_runs_twenty_seconds_alone_and_faster_with_ships_near() {
    // Nobody near: 20 s.
    let (mut w, _) = world(3);
    let mut t = room();
    let h = until_held(&mut w, &mut t);
    let start = w.tick();
    let mut det = None;
    for _ in 0..secs(25.0) {
        let ev = w.step(&mut t, &[]);
        if ev
            .iter()
            .any(|e| matches!(e, Event::Killed { victim, .. } if *victim == h))
        {
            det = Some(w.tick());
            break;
        }
    }
    assert_eq!(det, Some(start + secs(20.0)), "alone: 20 s");

    // Two ships within 15 u of the holder all the time: 2x speed (10 s);
    // one: 1.5x (13.33 s).
    for (near, expect) in [(2usize, secs(10.0)), (1, 800)] {
        let (mut w, ids) = world(4);
        let mut t = room();
        let h = until_held(&mut w, &mut t);
        let start = w.tick();
        let others: Vec<PlayerId> = ids.iter().copied().filter(|&i| i != h).collect();
        let mut det = None;
        for _ in 0..secs(25.0) {
            place(&mut w, h, Vec3::ZERO);
            for (k, o) in others.iter().enumerate() {
                let pos = if k < near {
                    Vec3::new(5.0 + 4.0 * k as f32, 0.0, 0.0)
                } else {
                    Vec3::new(-25.0, 10.0, 25.0)
                };
                place(&mut w, *o, pos);
            }
            let ev = w.step(&mut t, &[]);
            if w.tick() == start + 1 {
                // The replicated timer predicts the detonation at this speed.
                assert_eq!(reactor(&w).timer_end, start + expect, "{near} near");
            }
            if ev
                .iter()
                .any(|e| matches!(e, Event::Killed { victim, .. } if *victim == h))
            {
                det = Some(w.tick());
                break;
            }
        }
        assert_eq!(det, Some(start + expect), "{near} ships near");
    }
}

#[test]
fn detonation_costs_the_holder_a_life_and_the_blast_hurts_ships_near() {
    let (mut w, ids) = world(4);
    let mut t = room();
    let h = until_held(&mut w, &mut t);
    let others: Vec<PlayerId> = ids.iter().copied().filter(|&i| i != h).collect();
    let (near, far, out) = (others[0], others[1], others[2]);
    w.mode_mut().vars[0] = 1000; // one tick of fuse left
    place(&mut w, h, Vec3::ZERO);
    place(&mut w, near, Vec3::new(5.0, 0.0, 0.0)); // half the blast: 30
    place(&mut w, far, Vec3::new(0.0, 0.0, 9.0)); // 6
    place(&mut w, out, Vec3::new(-11.0, 0.0, 0.0)); // outside 10 u
    let hull: Vec<f32> = [near, far, out]
        .iter()
        .map(|i| w.ship(*i).unwrap().hull)
        .collect();
    let ev = run(&mut w, &mut t, 1);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Killed { victim, killer: None, .. } if *victim == h
    )));
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Shockwave && c.value == 10)));
    assert!(!w.ship(h).unwrap().alive);
    assert_eq!(lives(&w, h), 2);
    for &o in &others {
        assert_eq!(lives(&w, o), 3, "only the holder loses a life");
    }
    let lost = |i: usize, id: PlayerId| hull[i] - w.ship(id).unwrap().hull;
    assert!((lost(0, near) - 30.0).abs() < 1e-3, "{}", lost(0, near));
    assert!((lost(1, far) - 6.0).abs() < 1e-3, "{}", lost(1, far));
    assert_eq!(lost(2, out), 0.0);
    // No kill credit for anyone.
    assert!(w.ships().all(|s| s.kills == 0));
    assert_eq!(detonations(w.mode()), 1);
    // The next reactor appears 3 s later, on a living player.
    let o = reactor(&w);
    assert_eq!(o.state, STATE_WAITING);
    assert_eq!(o.timer_end, w.tick() + secs(3.0));
    run(&mut w, &mut t, secs(3.0) - 1);
    assert_eq!(holder(w.mode()), None);
    run(&mut w, &mut t, 1);
    let next = holder(w.mode()).expect("next reactor");
    assert!(w.ship(next).unwrap().alive);
    // The holder respawns on the normal timer while lives remain.
    run(&mut w, &mut t, secs(3.0));
    assert!(w.ship(h).unwrap().alive);
}

#[test]
fn other_deaths_cost_no_life_and_kills_score() {
    let (mut w, ids) = world(3);
    let mut t = room();
    let h = until_held(&mut w, &mut t);
    let others: Vec<PlayerId> = ids.iter().copied().filter(|&i| i != h).collect();
    let (b, c) = (others[0], others[1]);
    // B kills C (not the holder): C loses no life, B scores.
    hit(&mut w, &mut t, b, c, 10_000.0);
    assert!(!w.ship(c).unwrap().alive);
    assert_eq!(lives(&w, c), 3);
    assert_eq!(w.mode().player(b).unwrap().score, 1);
    assert_eq!(holder(w.mode()), Some(h));
    // C respawns as usual.
    run(&mut w, &mut t, secs(3.0) + 1);
    assert!(w.ship(c).unwrap().alive);
}

#[test]
fn a_killed_holder_hands_it_to_the_killer_and_a_suicide_detonates() {
    let (mut w, ids) = world(3);
    let mut t = room();
    let a = until_held(&mut w, &mut t);
    let b = *ids.iter().find(|&&i| i != a).unwrap();
    // B kills A with one hit: B holds it, A loses no life.
    hit(&mut w, &mut t, b, a, 10_000.0);
    assert_eq!(holder(w.mode()), Some(b));
    assert_eq!(lives(&w, a), 3);
    // Killed while immune: the killer still takes it.
    run(&mut w, &mut t, secs(3.5));
    let c = *ids.iter().find(|&&i| i != a && i != b).unwrap();
    hit(&mut w, &mut t, c, b, 1.0);
    assert_eq!(holder(w.mode()), Some(c));
    hit(&mut w, &mut t, a, c, 10_000.0);
    assert_eq!(
        holder(w.mode()),
        Some(a),
        "killer takes it despite immunity"
    );
    // A dies by its own hand: the reactor detonates, A loses a life.
    let pos = w.ship(a).unwrap().pos;
    w.hook_for_test(&mut t, |ctx| ctx.adjust_hull(a, -10_000.0, true));
    assert!(!w.ship(a).unwrap().alive);
    let ev = run(&mut w, &mut t, 1);
    assert_eq!(lives(&w, a), 2);
    assert_eq!(detonations(w.mode()), 1);
    assert_eq!(holder(w.mode()), None);
    assert_eq!(reactor(&w).pos, pos);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Shockwave)));
}

#[test]
fn a_holder_who_leaves_or_spectates_loses_nothing_and_a_new_reactor_comes() {
    let (mut w, ids) = world(4);
    let mut t = room();
    let a = until_held(&mut w, &mut t);
    w.set_spectator(a, true).unwrap();
    run(&mut w, &mut t, 1);
    assert_eq!(holder(w.mode()), None);
    assert_eq!(detonations(w.mode()), 0);
    let b = until_held(&mut w, &mut t);
    assert_ne!(a, b);
    assert!(w.remove_player(b));
    run(&mut w, &mut t, 1);
    assert_eq!(holder(w.mode()), None);
    let c = until_held(&mut w, &mut t);
    assert!(ids.contains(&c) && c != a && c != b);
}

#[test]
fn three_detonations_eliminate_and_the_last_player_with_lives_wins() {
    let mut r = rules();
    r.mode.set_field("fuse_secs", 1.0);
    r.mode.set_field("next_reactor_secs", 0.5);
    r.mode.set_field("respawn_secs", 0.5);
    let (mut w, ids) = world_with(r, 3);
    let mut t = room();
    let mut eliminated = Vec::new();
    let mut winner = None;
    for _ in 0..secs(120.0) {
        // Ships stay far apart; the reactor never passes.
        for (k, id) in ids.iter().enumerate() {
            if w.ship(*id).is_some_and(|s| s.alive) {
                let pos = setup().spawns[k * 3].pos;
                place(&mut w, *id, pos);
            }
        }
        for e in w.step(&mut t, &[]) {
            match e {
                Event::Eliminated { player } => eliminated.push(player),
                Event::MatchEnded { winner: wn, .. } => winner = Some(wn),
                _ => {}
            }
        }
        if winner.is_some() {
            break;
        }
    }
    assert_eq!(eliminated.len(), 2, "{eliminated:?}");
    let last = *ids.iter().find(|i| !eliminated.contains(i)).unwrap();
    assert!(lives(&w, last) >= 1);
    for e in &eliminated {
        assert_eq!(lives(&w, *e), 0);
        assert!(w.mode().player(*e).unwrap().flags & player_flags::ELIMINATED != 0);
        assert!(!w.ship(*e).unwrap().alive);
    }
    let side = w.ship(last).unwrap().team;
    assert_eq!(winner, Some(Some(side)));
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(side) });
    assert_eq!(detonations(w.mode()), 6 + (3 - lives(&w, last) as u32));
    let res = w.results();
    assert_eq!(res.winner, Some(side));
}

#[test]
fn on_time_the_most_lives_win_then_the_score() {
    let mut r = rules();
    r.mode.set_field("match_secs", 10.0);
    let (mut w, ids) = world_with(r.clone(), 3);
    let mut t = room();
    run(&mut w, &mut t, 5);
    w.mode_mut().players.get_mut(&ids[0]).unwrap().lives = 2;
    w.mode_mut().players.get_mut(&ids[1]).unwrap().score = 5;
    let ev = run(&mut w, &mut t, secs(10.0));
    let side = |id: PlayerId| w.ship(id).unwrap().team;
    // ids[1] and ids[2] have 3 lives; ids[1] scored more.
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::MatchEnded { winner: Some(s), .. } if *s == side(ids[1])
    )));
    // Equal lives and score: a draw.
    let (mut w, _) = world_with(r, 3);
    let mut t = room();
    run(&mut w, &mut t, secs(10.0));
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: None });
}

#[test]
fn late_joiners_play_until_the_first_detonation() {
    let (mut w, _) = world(2);
    let mut t = room();
    run(&mut w, &mut t, secs(15.0));
    let late = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(!w.is_spectator(late), "before the first detonation: plays");
    let h = until_held(&mut w, &mut t);
    w.mode_mut().vars[0] = 1000;
    run(&mut w, &mut t, 1);
    assert_eq!(detonations(w.mode()), 1);
    assert!(!w.ship(h).unwrap().alive);
    let later = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(w.is_spectator(later), "after it: spectates");
}

/// Scripted 4-ship match on the default rule set: ships chase the nearest
/// other ship (or flee the holder), fire at random, ram and get rammed.
fn replay(seed: u64, ticks: u64) -> (Vec<u64>, Vec<Event>, u32) {
    let (mut w, ids) = {
        let mut cfg = WorldConfig::paint_the_void(seed, setup());
        cfg.rules = rules();
        let mut w = World::new(cfg).unwrap();
        let ids: Vec<PlayerId> = ShipType::ALL
            .iter()
            .map(|&ty| w.add_player(None, ty).unwrap())
            .collect();
        (w, ids)
    };
    let mut t = room();
    let mut rng = Pcg32::new(seed, 17);
    let mut hashes = Vec::new();
    let mut events = Vec::new();
    let mut passes = 0;
    for tick in 1..=ticks {
        let h = holder(w.mode());
        let inputs: Vec<(PlayerId, InputCmd)> = ids
            .iter()
            .map(|&id| {
                let me = w.ship(id).unwrap();
                let target = w
                    .ships()
                    .filter(|s| s.alive && s.player != id)
                    .min_by(|a, b| {
                        a.pos
                            .distance_squared(me.pos)
                            .total_cmp(&b.pos.distance_squared(me.pos))
                    })
                    .map(|s| s.pos);
                let toward = target.map_or(Vec3::ZERO, |p| p - me.pos);
                let local = me.orientation.conjugate().rotate(toward);
                // The holder chases; the others flee it now and then.
                let flee = h.is_some_and(|h| h != id) && rng.chance(0.5);
                let thrust = if flee { -local } else { local }.normalize_or_zero();
                let pitch = ls_math::det::atan2(local.y, -local.z).clamp(-0.1, 0.1);
                let yaw = ls_math::det::atan2(-local.x, -local.z).clamp(-0.1, 0.1);
                let mut buttons = Buttons::HORIZON_ASSIST;
                buttons.set(Buttons::FIRE_PRIMARY, rng.chance(0.3));
                buttons.set(Buttons::FIRE_SECONDARY, rng.chance(0.02));
                buttons.set(Buttons::BOOST, rng.chance(0.3));
                (
                    id,
                    InputCmd {
                        tick,
                        thrust,
                        look_pitch: pitch,
                        look_yaw: yaw,
                        roll: 0.0,
                        buttons,
                        weapon_select: 0,
                    },
                )
            })
            .collect();
        events.extend(w.step(&mut t, &inputs));
        let now = holder(w.mode());
        if h.is_some() && now.is_some() && now != h {
            passes += 1;
        }
        if tick % 60 == 0 {
            hashes.push(w.state_hash());
        }
    }
    (hashes, events, passes)
}

/// Golden hash of the 60 s Hot Reactor replay; must match on native and
/// wasm32. Changes with Hot Reactor behaviour, its rule set or general sim
/// behaviour: update it in the same change and say so in the commit.
const HOT_REACTOR_REPLAY_GOLDEN_HASH: u64 = 0xa94b_6f50_7b53_d4f3;

#[test]
fn replay_is_deterministic_and_matches_the_golden_hash() {
    let (h1, e1, passes) = replay(4242, 3600);
    let (h2, e2, _) = replay(4242, 3600);
    assert_eq!(h1, h2);
    assert_eq!(e1, e2);
    let dets = e1
        .iter()
        .filter(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Shockwave))
        .count();
    eprintln!("hot reactor replay: {passes} passes, {dets} detonations");
    assert!(passes >= 1, "the reactor changes hands");
    // The replay exercises the mode: the reactor changes hands and detonates.
    assert!(e1.iter().any(|e| matches!(e, Event::Damaged { .. })));
    assert!(e1
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Shockwave)));
    let last = *h1.last().unwrap();
    assert_eq!(
        last, HOT_REACTOR_REPLAY_GOLDEN_HASH,
        "hot reactor replay hash {last:#018x}"
    );
}
