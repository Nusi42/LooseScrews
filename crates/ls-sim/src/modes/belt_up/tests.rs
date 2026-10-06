//! Belt Up scenario tests (one or more per rule of
//! `docs/milestone-4-plan.md` §3.8), a full 3-heat match with autopilots
//! and the replay golden hash.

use std::collections::BTreeMap;

use ls_math::level::{BeltSite, BELT_WEDGES};
use ls_math::Vec3;

use super::pilot::PilotStyle;
use super::replay::*;
use super::*;
use crate::golden::Arena;
use crate::mode::{player_flags, MatchPhase};
use crate::modes::module;
use crate::{
    DamageSource, Event, InputCmd, Mutator, PickupKind, RuleSet, ShipType, World, WorldConfig,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const DEG: f32 = core::f32::consts::PI / 180.0;

fn world_with(rules: RuleSet) -> (World, Arena) {
    let (setup, t) = belt_arena();
    let mut cfg = WorldConfig::paint_the_void(7, setup);
    cfg.rules = rules;
    (World::new(cfg).unwrap(), t)
}

fn race_world(n: usize) -> (World, Arena, Vec<PlayerId>) {
    let (mut w, t) = world_with(belt_up_rules());
    let ids = (0..n)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    (w, t, ids)
}

fn belt(w: &World) -> BeltSite {
    w.setup().belt.clone().unwrap()
}

fn race(w: &World) -> &RaceState {
    w.mode().race.as_ref().unwrap()
}

fn racer(w: &World, id: PlayerId) -> Racer {
    race(w).racers[&id]
}

fn step(w: &mut World, t: &mut Arena, n: u64) -> Vec<Event> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.extend(w.step(t, &[]));
    }
    out
}

fn race_phase(w: &World) -> u8 {
    w.mode()
        .race
        .as_ref()
        .map_or(heat_phase::LIGHTS, |r| r.phase)
}

/// Steps through the start lights (racing from the next tick).
fn lights_out(w: &mut World, t: &mut Arena) {
    while w.tick() == 0 || race_phase(w) != heat_phase::RACING {
        w.step(t, &[]);
    }
}

/// Puts a ship at `az` degrees, radius `r`, `h` above the plane (at rest),
/// then steps one tick: the movement segment is the teleport.
fn teleport(w: &mut World, t: &mut Arena, id: PlayerId, az: f32, r: f32, h: f32) {
    let b = belt(w);
    let s = w.ship_mut(id).unwrap();
    s.pos = belt_point(&b, az * DEG, r, h);
    s.vel = Vec3::ZERO;
    w.step(t, &[]);
}

/// Flies `id` through its next `n` gates by teleports.
fn pass_gates(w: &mut World, t: &mut Arena, id: PlayerId, n: u8) {
    for _ in 0..n {
        let x = racer(w, id);
        let ga = ((x.slot + 1 + x.gates) % 8) as f32 * 45.0;
        let ga = if ga == 0.0 { 360.0 } else { ga };
        teleport(w, t, id, ga - 4.0, 120.0, 8.0);
        teleport(w, t, id, ga + 1.0, 120.0, 8.0);
    }
}

fn kill(w: &mut World, t: &mut Arena, id: PlayerId) {
    w.hook_for_test(t, |ctx| {
        ctx.damage(id, 1000.0, Vec3::ZERO, DamageSource::ENVIRONMENT, true)
    });
}

/// Runs `f` with a read-only mode context of `w`.
fn with_ref<R>(w: &World, f: impl FnOnce(&crate::modes::ModeRef) -> R) -> R {
    let ships: BTreeMap<PlayerId, crate::ShipState> =
        w.ships().map(|s| (s.player, s.clone())).collect();
    let m = crate::modes::ModeRef {
        state: w.mode(),
        rules: &w.rules().mode,
        mutators: &w.rules().mutators,
        ships: &ships,
        now: w.tick(),
    };
    f(&m)
}

fn held(w: &World, id: PlayerId) -> bool {
    w.mode().player(id).unwrap().flags & player_flags::HELD != 0
}

#[test]
fn the_default_rule_set_is_valid_and_belt_up() {
    let r = belt_up_rules();
    assert_eq!(r.mode.kind(), ModeKind::BeltUp);
    assert_eq!(RuleSet::default_for(ModeKind::BeltUp), r);
    r.resolve(&crate::WeaponTable::builtin()).unwrap();
    let m = rules(&r.mode);
    assert_eq!(
        (m.heats, m.heat_limit_secs, m.finish_grace_secs),
        (3, 180.0, 30.0)
    );
    assert_eq!(
        (m.lights_secs, m.results_secs, m.hold_secs),
        (3.0, 8.0, 4.0)
    );
    assert_eq!(m.hull_floor_percent, 30);
    assert_eq!(
        (1..=9).map(|p| m.points(p)).collect::<Vec<_>>(),
        vec![10, 8, 6, 5, 4, 3, 2, 1, 0]
    );
    // Tectonic Charge banned; the path changers are in.
    assert!(r.availability("tectonic_charge").is_banned());
    for id in [
        "magnet_sneeze",
        "swap_harpoon",
        "prox_mine",
        "seeker",
        "quicksilver",
    ] {
        assert!(r.availability(id).as_pickup(), "{id}");
    }
    assert_eq!(module(ModeKind::BeltUp).team_counts(), &[0]);
    assert!(module(ModeKind::BeltUp).implemented());
}

#[test]
fn slots_spread_evenly_and_shift_per_heat() {
    for n in 1..=8 {
        let slots: Vec<u8> = (0..n).map(|i| slot_for(i, n, 0)).collect();
        let mut sorted = slots.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "{n}: distinct {slots:?}");
        // Gaps to the neighbours differ by at most one wedge.
        let gaps: Vec<i32> = (0..n)
            .map(|i| {
                let next = slots[(i + 1) % n] as i32 + if i + 1 == n { 8 } else { 0 };
                next - slots[i] as i32
            })
            .collect();
        let (lo, hi) = (gaps.iter().min().unwrap(), gaps.iter().max().unwrap());
        assert!(hi - lo <= 1, "{n}: gaps {gaps:?}");
        for heat in 1..3u8 {
            for i in 0..n {
                assert_eq!(slot_for(i, n, heat), (slot_for(i, n, 0) + heat) % 8);
            }
        }
    }
    let of = |n: usize| (0..n).map(|i| slot_for(i, n, 0)).collect::<Vec<_>>();
    assert_eq!(of(4), [0, 2, 4, 6]);
    assert_eq!(of(2), [0, 4]);
    assert_eq!(of(3), [0, 3, 5]);
}

#[test]
fn start_lights_hold_everyone_in_their_slots() {
    let (mut w, mut t, ids) = race_world(4);
    let b = belt(&w);
    w.step(&mut t, &[]);
    assert_eq!(race_phase(&w), heat_phase::LIGHTS);
    let flying: Vec<_> = ids
        .iter()
        .map(|id| {
            (
                *id,
                InputCmd::idle(0)
                    .with_thrust(Vec3::new(0.0, 0.0, -1.0))
                    .with_buttons(crate::Buttons::BOOST | crate::Buttons::FIRE_PRIMARY),
            )
        })
        .collect();
    for _ in 0..170 {
        let ev = w.step(&mut t, &flying);
        assert!(!ev.iter().any(|e| matches!(e, Event::Fired { .. })));
    }
    for (i, id) in ids.iter().enumerate() {
        let s = w.ship(*id).unwrap();
        let slot = b.slots[slot_for(i, 4, 0) as usize].pos;
        assert!(s.pos.distance(slot) < 0.05, "{i}: {:?} vs {slot:?}", s.pos);
        assert!(held(&w, *id));
    }
    assert_eq!(w.dose_rate(b.center + Vec3::new(120.0, 60.0, 0.0)), 0.0);
    lights_out(&mut w, &mut t);
    for _ in 0..60 {
        w.step(&mut t, &flying);
    }
    for id in &ids {
        assert!(!held(&w, *id));
        assert!(
            w.ship(*id).unwrap().vel.length() > 5.0,
            "flies after the lights"
        );
    }
    assert!(w.dose_rate(b.center + Vec3::new(120.0, 40.0, 0.0)) > 2.0);
}

#[test]
fn gates_count_in_order_including_teleports_and_never_the_wrong_way() {
    let (mut w, mut t, ids) = race_world(1);
    let id = ids[0];
    lights_out(&mut w, &mut t);
    assert_eq!(racer(&w, id).slot, 0);
    // Gate 1 (45 degrees) is next; a teleport across it counts.
    teleport(&mut w, &mut t, id, 40.0, 120.0, 8.0);
    assert_eq!(racer(&w, id).gates, 0);
    teleport(&mut w, &mut t, id, 50.0, 120.0, 8.0);
    assert_eq!(racer(&w, id).gates, 1);
    // Wrong way back across gate 1 and forward again: still one gate.
    teleport(&mut w, &mut t, id, 40.0, 120.0, 8.0);
    teleport(&mut w, &mut t, id, 50.0, 120.0, 8.0);
    assert_eq!(racer(&w, id).gates, 1);
    // Gate 3 before gate 2 does not count (round gate 2 through the hot
    // centre, which is outside its radial span).
    teleport(&mut w, &mut t, id, 130.0, 60.0, 8.0);
    teleport(&mut w, &mut t, id, 130.0, 120.0, 8.0);
    teleport(&mut w, &mut t, id, 140.0, 120.0, 8.0);
    assert_eq!(racer(&w, id).gates, 1, "gate 3 before gate 2");
    // Back behind gate 2; through the hot centre again: no count.
    teleport(&mut w, &mut t, id, 120.0, 60.0, 8.0);
    teleport(&mut w, &mut t, id, 85.0, 60.0, 8.0);
    teleport(&mut w, &mut t, id, 85.0, 60.0, 8.0);
    teleport(&mut w, &mut t, id, 95.0, 60.0, 8.0);
    assert_eq!(racer(&w, id).gates, 1);
    // High above the belt still counts (the open line).
    teleport(&mut w, &mut t, id, 85.0, 120.0, 40.0);
    teleport(&mut w, &mut t, id, 95.0, 120.0, 40.0);
    assert_eq!(racer(&w, id).gates, 2);
    // The exit is closed before the lap is done.
    teleport(&mut w, &mut t, id, 95.0, 0.0, 0.0);
    assert_eq!(racer(&w, id).finish_tick, 0);
    assert!(w.ship(id).unwrap().alive);
    // The rest of the lap, own gate (0) last.
    pass_gates(&mut w, &mut t, id, 6);
    assert_eq!(racer(&w, id).gates, 8);
    assert_eq!(race(&w).view().racer(id).unwrap().next_gate(), None);
    // Lap done: the exit finishes the race and the ship leaves play.
    teleport(&mut w, &mut t, id, 0.0, 10.0, 0.0);
    assert_eq!(racer(&w, id).finish_tick, 0);
    teleport(&mut w, &mut t, id, 0.0, 3.0, 0.0);
    let x = racer(&w, id);
    assert_eq!(x.finish_tick, w.tick());
    assert_eq!(x.place, 1);
    assert!(!w.ship(id).unwrap().alive, "finishing leaves play");
    assert!(w.is_spectator(id));
    // No respawn while the heat runs.
    step(&mut w, &mut t, 300);
    assert!(!w.ship(id).unwrap().alive);
}

#[test]
fn a_fast_dive_through_the_exit_counts() {
    // A swept segment through the exit sphere finishes, even if both ends
    // are outside it.
    let (mut w, mut t, ids) = race_world(1);
    let id = ids[0];
    lights_out(&mut w, &mut t);
    pass_gates(&mut w, &mut t, id, 8);
    teleport(&mut w, &mut t, id, 0.0, 8.0, 0.0);
    teleport(&mut w, &mut t, id, 180.0, 8.0, 0.0);
    assert!(racer(&w, id).finish_tick > 0);
}

#[test]
fn places_follow_progress_and_finishers_lead() {
    let (mut w, mut t, ids) = race_world(4);
    lights_out(&mut w, &mut t);
    // a: 2 gates; b: 1 gate, far into the sector; c: 1 gate, just past it;
    // d: no gate.
    let (a, b, c, d) = (ids[0], ids[1], ids[2], ids[3]);
    for (id, gates, into) in [(a, 2, 10.0), (b, 1, 40.0), (c, 1, 3.0), (d, 0, 20.0)] {
        pass_gates(&mut w, &mut t, id, gates);
        let x = racer(&w, id);
        let last = ((x.slot + gates) % 8) as f32 * 45.0;
        teleport(&mut w, &mut t, id, last + into, 120.0, 8.0);
        assert_eq!(racer(&w, id).gates, gates);
    }
    let v = w.mode_view().race.unwrap();
    let place = |id| v.racer(id).unwrap().place;
    assert_eq!([place(a), place(b), place(c), place(d)], [1, 2, 3, 4]);
    assert_eq!(
        with_ref(&w, |m| module(ModeKind::BeltUp).leader(m)),
        Some(a)
    );
    // A finisher leads everyone; the leader hook then picks the best racer
    // still racing.
    pass_gates(&mut w, &mut t, d, 8);
    teleport(&mut w, &mut t, d, 0.0, 0.0, 0.0);
    let v = w.mode_view().race.unwrap();
    assert_eq!(v.racer(d).unwrap().place, 1);
    assert_eq!(v.racer(a).unwrap().place, 2);
    assert_eq!(
        with_ref(&w, |m| module(ModeKind::BeltUp).leader(m)),
        Some(a)
    );
}

#[test]
fn death_holds_at_the_last_gate_with_its_hull_and_a_floor() {
    let (mut w, mut t, ids) = race_world(2);
    let id = ids[0];
    let r = rules(&w.rules().mode);
    lights_out(&mut w, &mut t);
    let b = belt(&w);
    // Pass gate 1, then gate 2 with 60 hull.
    pass_gates(&mut w, &mut t, id, 1);
    w.ship_mut(id).unwrap().hull = 60.0;
    pass_gates(&mut w, &mut t, id, 1);
    assert_eq!(racer(&w, id).gates, 2);
    assert_eq!(racer(&w, id).gate_hull, 60.0);
    w.ship_mut(id).unwrap().hull = 20.0;
    teleport(&mut w, &mut t, id, 120.0, 120.0, 8.0);
    let died = w.tick();
    kill(&mut w, &mut t, id);
    assert!(!w.ship(id).unwrap().alive);
    let hold = ticks(r.hold_secs);
    assert_eq!(racer(&w, id).hold_end_tick, died + hold);
    // The wreck shows for respawn_secs, then the ship is back at gate 2's
    // slot pose, held (frozen, weapons off, no damage) until the hold ends.
    step(&mut w, &mut t, ticks(r.respawn_secs) + 1);
    let s = w.ship(id).unwrap().clone();
    assert!(s.alive);
    assert!(s.pos.distance(b.slots[2].pos) < 0.05, "{:?}", s.pos);
    assert_eq!(s.hull, 60.0, "the hull it had at the gate");
    assert!(held(&w, id));
    assert!(s.weapons_offline_until >= died + hold);
    let thrust = [(id, InputCmd::idle(0).with_thrust(Vec3::new(0.0, 0.0, -1.0)))];
    kill(&mut w, &mut t, id);
    assert!(w.ship(id).unwrap().alive, "held ships take no damage");
    while w.tick() < died + hold - 1 {
        w.step(&mut t, &thrust);
    }
    assert!(w.ship(id).unwrap().pos.distance(b.slots[2].pos) < 0.05);
    for _ in 0..30 {
        w.step(&mut t, &thrust);
    }
    assert!(!held(&w, id));
    assert!(
        w.ship(id).unwrap().pos.distance(b.slots[2].pos) > 1.0,
        "free"
    );
    // The 30 % floor: gate hull 10 -> 30.
    w.mode_mut()
        .race
        .as_mut()
        .unwrap()
        .racers
        .get_mut(&id)
        .unwrap()
        .gate_hull = 10.0;
    kill(&mut w, &mut t, id);
    step(&mut w, &mut t, ticks(r.respawn_secs) + 1);
    assert_eq!(w.ship(id).unwrap().hull, 30.0);
    // Before the first gate: the own start slot, full hull.
    let other = ids[1];
    kill(&mut w, &mut t, other);
    step(&mut w, &mut t, ticks(r.respawn_secs) + 1);
    let s = w.ship(other).unwrap();
    assert!(s.pos.distance(b.slots[racer(&w, other).slot as usize].pos) < 0.05);
    assert_eq!(s.hull, 100.0);
}

#[test]
fn a_respawn_keeps_weapons_and_ammo() {
    let (mut w, mut t, ids) = race_world(1);
    let id = ids[0];
    lights_out(&mut w, &mut t);
    w.give_weapon(id, "seeker").unwrap();
    let before = w.ship(id).unwrap().weapons.clone();
    assert!(before.len() >= 3);
    kill(&mut w, &mut t, id);
    step(&mut w, &mut t, 70);
    assert!(w.ship(id).unwrap().alive);
    assert_eq!(w.ship(id).unwrap().weapons, before);
}

#[test]
fn points_ties_and_standings() {
    let (mut w, mut t, ids) = race_world(3);
    let r = rules(&w.rules().mode);
    lights_out(&mut w, &mut t);
    // Two finish (b first), one does not.
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    for &id in &[b, a] {
        pass_gates(&mut w, &mut t, id, 8);
        teleport(&mut w, &mut t, id, 0.0, 0.0, 0.0);
        assert!(racer(&w, id).finish_tick > 0);
    }
    // The grace runs out 30 s after the first finisher.
    let first = race(&w).first_finish_tick;
    while race_phase(&w) == heat_phase::RACING {
        w.step(&mut t, &[]);
    }
    assert_eq!(w.tick(), first + ticks(r.finish_grace_secs));
    let score = |w: &World, id| w.mode().player(id).unwrap().score;
    assert_eq!([score(&w, b), score(&w, a), score(&w, c)], [10, 8, 6]);
    assert_eq!(racer(&w, b).heat_wins, 1);
    assert_eq!(racer(&w, c).last_place, 3);
    assert_eq!(BeltUp::standings(w.mode()), vec![b, a, c]);
    // Ties: equal points -> more heat wins -> better last place -> time.
    let mut s = w.mode().clone();
    for id in [a, b, c] {
        s.players.get_mut(&id).unwrap().score = 20;
    }
    let race_ = s.race.as_mut().unwrap();
    race_.racers.get_mut(&a).unwrap().heat_wins = 2;
    assert_eq!(BeltUp::standings(&s)[0], a);
    let race_ = s.race.as_mut().unwrap();
    for (id, wins, last, time) in [(a, 1, 2, 500), (b, 1, 1, 900), (c, 1, 2, 400)] {
        let x = race_.racers.get_mut(&id).unwrap();
        x.heat_wins = wins;
        x.last_place = last;
        x.total_ticks = time;
    }
    assert_eq!(BeltUp::standings(&s), [b, c, a]);
}

#[test]
fn kills_score_nothing() {
    let (mut w, mut t, ids) = race_world(2);
    lights_out(&mut w, &mut t);
    let src = DamageSource {
        attacker: Some(ids[0]),
        team: w.ship(ids[0]).unwrap().team,
        weapon: None,
    };
    w.hook_for_test(&mut t, |ctx| {
        ctx.damage(ids[1], 500.0, Vec3::ZERO, src, true)
    });
    assert!(!w.ship(ids[1]).unwrap().alive);
    assert_eq!(w.mode().player(ids[0]).unwrap().score, 0);
    assert_eq!(w.mode().player(ids[1]).unwrap().score, 0);
}

#[test]
fn the_heat_limit_ends_a_heat_without_finishers_and_heats_shift_slots() {
    let (mut w, mut t, ids) = race_world(2);
    let r = rules(&w.rules().mode);
    lights_out(&mut w, &mut t);
    let start = race(&w).start_tick;
    let slots0: Vec<u8> = ids.iter().map(|id| racer(&w, *id).slot).collect();
    while race_phase(&w) == heat_phase::RACING {
        w.step(&mut t, &[]);
    }
    assert_eq!(w.tick(), start + ticks(r.heat_limit_secs));
    assert_eq!(race_phase(&w), heat_phase::RESULTS);
    // Everyone held through the results, then heat 2 in shifted slots.
    for id in &ids {
        assert!(held(&w, *id));
    }
    while race_phase(&w) == heat_phase::RESULTS {
        w.step(&mut t, &[]);
    }
    assert_eq!(race(&w).heat, 1);
    w.step(&mut t, &[]);
    let b = belt(&w);
    for (i, id) in ids.iter().enumerate() {
        let x = racer(&w, *id);
        assert_eq!(x.slot, (slots0[i] + 1) % 8);
        assert_eq!(x.gates, 0);
        let s = w.ship(*id).unwrap();
        assert!(s.pos.distance(b.slots[x.slot as usize].pos) < 0.05);
        assert_eq!(s.hull, 100.0);
    }
}

#[test]
fn a_new_heat_resets_pickups_mines_and_wells() {
    let (mut w, mut t, ids) = race_world(2);
    lights_out(&mut w, &mut t);
    w.add_well(crate::DynamicWell {
        key: 9,
        pos: Vec3::new(0.0, 30.0, 0.0),
        peak_accel: 3.0,
        range: 10.0,
        start_tick: 0,
        end_tick: u64::MAX,
        owner: None,
    });
    w.give_weapon(ids[0], "prox_mine").unwrap();
    let mine = w.weapons().require("prox_mine").unwrap();
    let select = [(
        ids[0],
        InputCmd {
            weapon_select: mine.0 + 1,
            ..InputCmd::idle(0)
        },
    )];
    w.step(&mut t, &select);
    let fire = [(
        ids[0],
        InputCmd::idle(0).with_buttons(crate::Buttons::FIRE_SECONDARY),
    )];
    for _ in 0..5 {
        w.step(&mut t, &fire);
    }
    assert!(!w.projectiles().is_empty(), "a mine is out");
    // Take a pickup's content.
    let p0 = w.pickups()[0].content_pos();
    w.ship_mut(ids[1]).unwrap().pos = p0;
    w.step(&mut t, &[]);
    assert!(w.pickups()[0].content.is_none());
    w.mode_mut().race.as_mut().unwrap().phase_end_tick = w.tick() + 1;
    while race(&w).heat == 0 {
        w.step(&mut t, &[]);
    }
    assert!(w.projectiles().is_empty());
    assert!(w.dynamic_wells().is_empty());
    assert!(w.pickups().iter().all(|p| p.content.is_some()));
}

#[test]
fn every_wedge_holds_the_same_pickups() {
    let (mut w, mut t, _) = race_world(2);
    w.step(&mut t, &[]);
    let b = belt(&w);
    let per = b.pickup_points.len() / BELT_WEDGES;
    assert_eq!(per, 3);
    let kinds: Vec<Option<PickupKind>> = w
        .pickups()
        .iter()
        .map(|p| p.content.map(|(_, k)| k))
        .collect();
    for i in 0..kinds.len() {
        assert_eq!(kinds[i], kinds[i % per], "point {i}");
        assert!(kinds[i].is_some());
    }
    // Other heats roll other (but again identical) pickups.
    let mut heats = vec![kinds[..per].to_vec()];
    for _ in 0..2 {
        let heat = race(&w).heat;
        w.mode_mut().race.as_mut().unwrap().phase_end_tick = w.tick() + 1;
        while race(&w).heat == heat {
            w.step(&mut t, &[]);
        }
        let k: Vec<_> = w
            .pickups()
            .iter()
            .map(|p| p.content.map(|(_, k)| k))
            .collect();
        for i in 0..k.len() {
            assert_eq!(k[i], k[i % per]);
        }
        heats.push(k[..per].to_vec());
    }
    assert!(heats[0] != heats[1] || heats[1] != heats[2]);
}

#[test]
fn late_joiners_spectate_until_the_next_heat_and_race_from_zero() {
    let (mut w, mut t, ids) = race_world(2);
    w.step(&mut t, &[]);
    // During the start lights a joiner races at once (everyone re-placed).
    let early = w.add_player(None, ShipType::Tinker).unwrap();
    w.step(&mut t, &[]);
    assert!(racer(&w, early).in_heat);
    assert!(w.ship(early).unwrap().alive);
    let slots: Vec<u8> = [ids[0], ids[1], early]
        .iter()
        .map(|id| racer(&w, *id).slot)
        .collect();
    assert_eq!(slots, [0, 3, 5]);
    let b = belt(&w);
    w.step(&mut t, &[]);
    for (id, slot) in [ids[0], ids[1], early].iter().zip(slots) {
        assert!(
            w.ship(*id)
                .unwrap()
                .pos
                .distance(b.slots[slot as usize].pos)
                < 0.05
        );
    }
    lights_out(&mut w, &mut t);
    let late = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(w.is_spectator(late));
    assert!(!racer(&w, late).in_heat);
    assert!(w.set_spectator(late, false).is_err());
    w.mode_mut().race.as_mut().unwrap().phase_end_tick = w.tick() + 1;
    while race(&w).heat == 0 {
        w.step(&mut t, &[]);
    }
    w.step(&mut t, &[]);
    assert!(racer(&w, late).in_heat);
    assert!(!w.is_spectator(late));
    assert!(w.ship(late).unwrap().alive);
    assert_eq!(w.mode().player(late).unwrap().score, 0);
    assert_eq!(race(&w).in_heat(), 4);
}

#[test]
fn a_joiner_to_an_empty_race_starts_the_heat_over() {
    // The room runs without players (a server room waiting for its first
    // pilot): the first joiner races at once, after fresh start lights.
    let (mut w, mut t) = world_with(belt_up_rules());
    step(&mut w, &mut t, 400);
    assert_eq!(race_phase(&w), heat_phase::RACING);
    let id = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(!w.is_spectator(id));
    w.step(&mut t, &[]);
    assert_eq!(race_phase(&w), heat_phase::LIGHTS);
    assert!(racer(&w, id).in_heat);
    assert!(held(&w, id));
    lights_out(&mut w, &mut t);
    assert!(!held(&w, id));
    // A second joiner now waits for the next heat.
    let late = w.add_player(None, ShipType::Tinker).unwrap();
    assert!(w.is_spectator(late));
}

#[test]
fn at_most_eight_racers() {
    let (mut w, mut t, ids) = race_world(10);
    w.step(&mut t, &[]);
    assert_eq!(race(&w).in_heat(), 8);
    assert!(w.is_spectator(ids[8]) && w.is_spectator(ids[9]));
    let slots: Vec<u8> = ids[..8].iter().map(|id| racer(&w, *id).slot).collect();
    assert_eq!(slots, [0, 1, 2, 3, 4, 5, 6, 7]);
}

#[test]
fn bumps_split_by_mass_with_their_caps() {
    let r = BeltUpRules::default();
    let ship = |pos: Vec3, vel: Vec3, mass: f32| ContactShip {
        player: PlayerId(0),
        pos,
        vel,
        radius: 0.5,
        mass,
    };
    // Equal masses, head-on at 4 u/s each: the approach (8) reverses at 1.2x.
    let a = ship(Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0), 1.0);
    let b = ship(Vec3::new(0.9, 0.0, 0.0), Vec3::new(-4.0, 0.0, 0.0), 1.0);
    let (da, db) = bump(&a, &b, &r).unwrap();
    assert!((da.x + 8.8).abs() < 1e-4 && (db.x - 8.8).abs() < 1e-4);
    let rel_after = (a.vel + da - b.vel - db).x;
    assert!(
        (rel_after + 9.6).abs() < 1e-3,
        "separates at 1.2x: {rel_after}"
    );
    // Hauler (1.5) vs Skitter (0.7): equal and opposite impulses.
    let a = ship(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0), 1.5);
    let b = ship(Vec3::new(0.9, 0.0, 0.0), Vec3::ZERO, 0.7);
    let (da, db) = bump(&a, &b, &r).unwrap();
    assert!((da.x * 1.5 + db.x * 0.7).abs() < 1e-4, "momentum kept");
    assert!(db.x.abs() > da.x.abs() * 2.0, "the light ship flies");
    // Capped at 10 u/s per ship.
    let a = ship(Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0), 1.0);
    let b = ship(Vec3::new(0.9, 0.0, 0.0), Vec3::new(-20.0, 0.0, 0.0), 1.0);
    let (da, db) = bump(&a, &b, &r).unwrap();
    assert!((da.length() - 10.0).abs() < 1e-4 && (db.length() - 10.0).abs() < 1e-4);
    // Overlapping without closing in: they still part at 1 u/s approach.
    let a = ship(Vec3::ZERO, Vec3::ZERO, 1.0);
    let b = ship(Vec3::new(0.5, 0.0, 0.0), Vec3::ZERO, 1.0);
    let (da, db) = bump(&a, &b, &r).unwrap();
    assert!(da.x < 0.0 && db.x > 0.0);
}

#[test]
fn ships_bump_once_per_cooldown_and_never_while_held() {
    let (mut w, mut t, ids) = race_world(2);
    let (a, b) = (ids[0], ids[1]);
    let bl = belt(&w);
    let place = |w: &mut World, va: f32, vb: f32| {
        let p = belt_point(&bl, 20.0 * DEG, 120.0, 8.0);
        let sa = w.ship_mut(a).unwrap();
        sa.pos = p;
        sa.vel = Vec3::new(va, 0.0, 0.0);
        let sb = w.ship_mut(b).unwrap();
        sb.pos = p + Vec3::new(0.9, 0.0, 0.0);
        sb.vel = Vec3::new(vb, 0.0, 0.0);
    };
    // Held during the lights: no bump.
    w.step(&mut t, &[]);
    w.step(&mut t, &[]);
    place(&mut w, 3.0, -3.0);
    w.step(&mut t, &[]);
    assert!(race(&w).bump_ready.is_empty());
    lights_out(&mut w, &mut t);
    place(&mut w, 3.0, -3.0);
    w.step(&mut t, &[]);
    assert_eq!(race(&w).bump_ready.len(), 1);
    let (va, vb) = (w.ship(a).unwrap().vel.x, w.ship(b).unwrap().vel.x);
    assert!(va < -2.0 && vb > 2.0, "{va} {vb}");
    // Overlapping again right away: no second bump within 0.3 s.
    place(&mut w, 3.0, -3.0);
    w.step(&mut t, &[]);
    assert!(w.ship(a).unwrap().vel.x > 0.0, "no bump in the cooldown");
    w.ship_mut(b).unwrap().pos += Vec3::new(0.0, 0.0, 10.0);
    step(&mut w, &mut t, 20);
    place(&mut w, 3.0, -3.0);
    w.step(&mut t, &[]);
    assert!(
        w.ship(a).unwrap().vel.x < 0.0,
        "bumps again: {:?} {:?}",
        w.ship(a).unwrap(),
        race(&w).bump_ready
    );
    // No damage from bumps.
    assert_eq!(w.ship(a).unwrap().hull, 100.0);
}

#[test]
fn boulders_push_hurt_and_absorb() {
    let (mut w, mut t, ids) = race_world(1);
    let id = ids[0];
    lights_out(&mut w, &mut t);
    let bl = belt(&w);
    let o = bl.boulders[0];
    // A slow touch pushes without damage.
    let now = w.tick() + 1;
    let c = o.pos_at(bl.center, now);
    {
        let s = w.ship_mut(id).unwrap();
        s.pos = c + Vec3::new(0.0, o.size + 0.3, 0.0);
        s.vel = Vec3::new(0.0, -2.0, 0.0);
    }
    w.step(&mut t, &[]);
    let s = w.ship(id).unwrap();
    assert!(s.pos.distance(o.pos_at(bl.center, now)) >= o.size + 0.5 - 1e-3);
    assert_eq!(s.hull, 100.0);
    // A fast one hurts (8, no credit).
    let now = w.tick() + 1;
    let c = o.pos_at(bl.center, now);
    {
        let s = w.ship_mut(id).unwrap();
        s.pos = c + Vec3::new(0.0, o.size + 0.6, 0.0);
        s.vel = Vec3::new(0.0, -12.0, 0.0);
    }
    let ev = w.step(&mut t, &[]);
    assert_eq!(w.ship(id).unwrap().hull, 92.0);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Damaged {
            attacker: None,
            amount,
            ..
        } if *amount == 8.0
    )));
    // A rocket into a boulder: it ends there, without a crater.
    let now = w.tick() + 2;
    let c = o.pos_at(bl.center, now);
    {
        let s = w.ship_mut(id).unwrap();
        s.pos = c + Vec3::new(0.0, 0.0, 12.0);
        s.vel = Vec3::ZERO;
        s.orientation = ls_math::Quat::IDENTITY;
    }
    let fire = [(
        id,
        InputCmd::idle(0).with_buttons(crate::Buttons::FIRE_SECONDARY),
    )];
    let mut impact = None;
    let mut craters = 0;
    for _ in 0..40 {
        for e in w.step(&mut t, &fire) {
            match e {
                Event::Impact { point, target, .. } => impact = Some((point, target)),
                Event::TerrainEdit(_) => craters += 1,
                _ => {}
            }
        }
        if impact.is_some() {
            break;
        }
    }
    let (point, target) = impact.expect("the rocket hit the boulder");
    assert_eq!(target, None);
    let bc = o.pos_at(bl.center, w.tick());
    assert!(point.distance(bc) < o.size + 1.0, "{point:?} vs {bc:?}");
    assert_eq!(craters, 0, "no crater");
    assert!(w.projectiles().is_empty());
}

#[test]
fn radiation_burns_only_while_racing() {
    let (mut w, mut t, ids) = race_world(1);
    let id = ids[0];
    let b = belt(&w);
    let high = b.center + Vec3::new(120.0, 40.0, 0.0);
    w.step(&mut t, &[]);
    assert_eq!(w.dose_rate(high), 0.0, "start lights");
    lights_out(&mut w, &mut t);
    assert!(w.dose_rate(high) >= 10.0);
    let r = rules(&w.rules().mode);
    assert_eq!(radiation_scale_in(&r, heat_phase::RACING), 1.0);
    assert_eq!(radiation_scale_in(&r, heat_phase::RESULTS), 0.0);
    let s = w.ship_mut(id).unwrap();
    s.pos = high;
    s.vel = Vec3::ZERO;
    step(&mut w, &mut t, 60);
    assert!(w.ship(id).unwrap().hull < 90.0);
}

#[test]
fn events_are_vetoed_outside_racing_and_in_the_first_seconds() {
    let (mut w, mut t, _) = race_world(2);
    let m = module(ModeKind::BeltUp);
    w.step(&mut t, &[]);
    let allows = |w: &World, k| with_ref(w, |r| m.allows_event(r, k));
    assert!(!allows(&w, EventKind::TaxDrone), "lights");
    lights_out(&mut w, &mut t);
    assert!(!allows(&w, EventKind::TaxDrone), "first 15 s");
    step(&mut w, &mut t, ticks(EVENT_QUIET_SECS));
    assert!(allows(&w, EventKind::TaxDrone));
    assert!(allows(&w, EventKind::DuckParade));
    for k in [
        EventKind::BoredLevel,
        EventKind::WrongGenre,
        EventKind::MovingDay,
    ] {
        assert!(!allows(&w, k), "{k:?}");
    }
    assert!(with_ref(&w, |r| m.leader(r)).is_some());
}

#[test]
fn instagib_works_and_still_holds_four_seconds() {
    let mut rules_ = belt_up_rules();
    rules_.mutators = vec![Mutator::Instagib];
    let (mut w, mut t) = world_with(rules_);
    let ids: Vec<PlayerId> = (0..2)
        .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    lights_out(&mut w, &mut t);
    let died = w.tick();
    kill(&mut w, &mut t, ids[0]);
    w.step(&mut t, &[]);
    assert!(w.ship(ids[0]).unwrap().alive, "instant respawn");
    assert!(held(&w, ids[0]));
    assert_eq!(racer(&w, ids[0]).hold_end_tick, died + ticks(4.0));
    assert!(
        w.pickups().iter().all(|p| p.content.is_none()),
        "no pickups"
    );
}

#[test]
fn without_a_belt_heats_end_on_their_limit() {
    // Test rooms (and the golden arena replays) have no belt.
    let mut rules_ = belt_up_rules();
    for (f, v) in [
        ("heat_limit_secs", 2.0),
        ("results_secs", 1.0),
        ("lights_secs", 1.0),
    ] {
        rules_.mode.set_field(f, v);
    }
    let mut cfg = WorldConfig::paint_the_void(3, crate::golden::setup());
    cfg.rules = rules_;
    let mut w = World::new(cfg).unwrap();
    let mut t = crate::golden::arena();
    for _ in 0..3 {
        w.add_player(None, ShipType::Tinker).unwrap();
    }
    let mut n = 0;
    while w.mode().is_running() && n < 60 * 20 {
        w.step(&mut t, &[]);
        n += 1;
    }
    assert!(!w.mode().is_running());
    assert!((3 * 240..3 * 240 + 4).contains(&n), "{n}");
}

/// A full 3-heat match with 8 autopiloted ships on the test belt reaches a
/// result; most racers finish every heat.
#[test]
fn a_full_three_heat_match_reaches_a_result() {
    let (mut w, mut t) = world_with(belt_up_rules());
    let b = w.setup().belt.clone().unwrap();
    let ids: Vec<PlayerId> = ShipType::ALL
        .iter()
        .cycle()
        .take(8)
        .map(|ty| w.add_player(None, *ty).unwrap())
        .collect();
    let styles: Vec<(PlayerId, PilotStyle)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, replay_style(i, &b)))
        .collect();
    let mut finishes = 0;
    let mut heats_seen = 0;
    while w.mode().is_running() && w.tick() < 60 * 900 {
        let before = race_phase(&w);
        race_to_end(&mut w, &mut t, &styles, 1);
        if let Some(r) = &w.mode().race {
            if before == heat_phase::RACING && r.phase == heat_phase::RESULTS {
                heats_seen += 1;
                finishes += r.racers.values().filter(|x| x.finish_tick > 0).count();
            }
        }
    }
    let MatchPhase::Ended { winner } = w.mode().phase else {
        panic!("the match ended");
    };
    assert_eq!(heats_seen, 3);
    assert!(finishes >= 15, "{finishes} finishes in 3 heats");
    let res = w.results();
    assert!(res.ended);
    let best = BeltUp::standings(w.mode())[0];
    assert_eq!(winner, Some(w.ship(best).unwrap().team));
    assert_eq!(res.rows[0].player, best);
    assert!(res.rows[0].score >= 20, "{:?}", res.rows);
    println!("match over at tick {}: {:?}", w.tick(), res.rows);
}

const FIXTURE: &str = include_str!("../../../tests/fixtures/mood_replays.txt");

/// The Belt Up replay matches the fixture on native and wasm32, and covers
/// what it should (finishers, bumps, a death and its respawn, boulders).
/// `LS_BLESS_MOOD_REPLAYS=1` rewrites its line (native).
#[test]
fn replay_matches_the_fixture() {
    let r = belt_up_replay();
    println!("{r:?}");
    assert!(r.heat_over, "{r:?}");
    assert!(r.finishers >= 3, "{r:?}");
    assert!(r.bumps >= 1, "{r:?}");
    assert!(r.deaths >= 1 && r.respawns >= 1, "{r:?}");
    assert!(r.boulder_hits >= 1, "{r:?}");
    assert_eq!(r, belt_up_replay(), "deterministic");
    let line = format!("{} {:#018x}", BELT_UP_REPLAY, r.hash);
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var_os("LS_BLESS_MOOD_REPLAYS").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mood_replays.txt"
        );
        let mut text: String = FIXTURE
            .lines()
            .filter(|l| !l.trim().starts_with(BELT_UP_REPLAY))
            .map(|l| format!("{l}\n"))
            .collect();
        text.push_str(&line);
        text.push('\n');
        std::fs::write(path, text).expect("write the fixture");
        return;
    }
    let want = FIXTURE
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(BELT_UP_REPLAY));
    assert_eq!(
        Some(line.as_str()),
        want,
        "Belt Up replay hash changed: re-bless with LS_BLESS_MOOD_REPLAYS=1 if deliberate"
    );
}
