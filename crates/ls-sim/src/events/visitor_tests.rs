//! Scenario tests of the visitor events (M4 B2, `docs/milestone-4-plan.md`
//! §3.2): Tax drone, Pizza delivery, Customer survey, Mimic pickups and
//! Gossip wall, each forced by `World::schedule_event` in a box room (or a
//! room split by a wall with one doorway for the network path).

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit, TerrainQuery};
use ls_math::{Aabb, Quat, Tick, Vec3};

use super::customer_survey::{panel_pose, star_at};
use super::drone::{self, NavGraph, OUTCOME_SHOT_DOWN};
use super::mimic_pickups::{FLAG_HUNTING, NO_SPAWN, OUTCOME_BITE};
use super::pizza_delivery::OUTCOME_DELIVERED;
use super::{EventKind, EventPhase};
use crate::entity::EntityKind;
use crate::mode::ModeKind;
use crate::ruleset::Availability;
use crate::{
    Buttons, Cue, CueKind, Event, InputCmd, MatchSetup, PlayerId, RuleSet, ShipType, SimTerrain,
    SpawnPoint, World, WorldConfig,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const ANNOUNCE: u64 = 180;

fn setup() -> MatchSetup {
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
        pickup_points: (0..12)
            .map(|i| Vec3::new(-44.0 + 8.0 * i as f32, -8.0, 20.0))
            .collect(),
        ..Default::default()
    }
}

fn rules() -> RuleSet {
    let mut r = RuleSet::default_for(ModeKind::TeamBrawl);
    r.mode.set_field("match_secs", 600.0);
    r.mutators.clear();
    r
}

fn world_with(rules: RuleSet, ships: usize) -> World {
    let mut cfg = WorldConfig::paint_the_void(77, setup());
    cfg.rules = rules;
    let mut w = World::new(cfg).unwrap();
    for i in 0..ships {
        w.add_player(Some((i % 2) as u8), ShipType::Tinker).unwrap();
    }
    w
}

fn world(ships: usize) -> World {
    world_with(rules(), ships)
}

fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(120.0, 40.0, 120.0))
}

fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
    s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
}

/// Makes player `p` the leader (Team Brawl: best player of the leading team).
fn lead(w: &mut World, p: PlayerId) {
    let team = w.ship(p).unwrap().team as usize;
    w.mode_mut().team_scores[team] = 5;
    w.mode_mut().players.get_mut(&p).unwrap().score = 5;
}

fn step<T: SimTerrain>(w: &mut World, t: &mut T, inputs: &[(PlayerId, InputCmd)]) -> Vec<Event> {
    w.step(t, inputs)
}

/// Forces `kind` next tick and runs through the announce phase; returns
/// the events of those ticks.
fn force<T: SimTerrain>(w: &mut World, t: &mut T, kind: EventKind) -> Vec<Event> {
    w.schedule_event(w.tick() + 1, kind);
    let mut out = Vec::new();
    for _ in 0..=ANNOUNCE {
        out.extend(step(w, t, &[]));
    }
    assert_eq!(w.mood_view().event, Some(kind));
    assert_eq!(w.mood_view().phase, EventPhase::Active);
    out
}

fn props(w: &World, kind: EntityKind) -> Vec<crate::Entity> {
    w.entities()
        .iter()
        .filter(|e| e.kind == kind)
        .cloned()
        .collect()
}

fn drone_of(w: &World) -> Option<crate::Entity> {
    props(w, EntityKind::Drone).into_iter().next()
}

fn outcomes(events: &[Event], kind: EventKind) -> Vec<Cue> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Cue(c)
                if c.kind == CueKind::EventOutcome && (c.value >> 8) as u8 == kind.index() =>
            {
                Some(*c)
            }
            _ => None,
        })
        .collect()
}

/// Input of a ship that aims at `at` and fires its primary.
fn fire_at(w: &mut World, id: PlayerId, at: Vec3) -> (PlayerId, InputCmd) {
    let s = w.ship_mut(id).unwrap();
    s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, (at - s.pos).normalize());
    s.ang_vel = Vec3::ZERO;
    s.energy = 100.0;
    (id, InputCmd::idle(0).with_buttons(Buttons::FIRE_PRIMARY))
}

/// A box room split at x = 0 by a 2 u wall with one round doorway (radius
/// 4) at z = 30, y = 0.
#[derive(Debug, Clone)]
struct WallRoom(BoxRoomTerrain);

const DOOR: Vec3 = Vec3::new(0.0, 0.0, 30.0);

impl TerrainQuery for WallRoom {
    fn sdf(&self, p: Vec3) -> f32 {
        let r = Vec3::new(0.0, p.y - DOOR.y, p.z - DOOR.z).length();
        let wall = (p.x.abs() - 1.0).max(4.0 - r);
        self.0.sdf(p).min(wall)
    }
    fn bounds(&self) -> Aabb {
        self.0.bounds()
    }
}

impl SimTerrain for WallRoom {
    fn apply_edit(&mut self, edit: &TerrainEdit) {
        self.0.apply_edit(edit);
    }
}

fn wall_room() -> WallRoom {
    WallRoom(room())
}

/// The doorway as a network: a node on each side and in the door, entries
/// at the far ends of both halves.
fn wall_graph() -> NavGraph {
    let a = Vec3::new(-30.0, 0.0, 30.0);
    let b = Vec3::new(30.0, 0.0, 30.0);
    let entries = [Vec3::new(-50.0, 0.0, -40.0)];
    NavGraph::from_network(
        &[a, b],
        &[
            vec![a, DOOR, b],
            vec![Vec3::new(-50.0, 0.0, -40.0), a],
            vec![Vec3::new(50.0, 0.0, 40.0), b],
        ],
        &entries,
    )
}

// ----- Tax drone ------------------------------------------------------------------

#[test]
fn tax_drone_enters_from_the_nearest_boundary_cavern_at_least_40_u_away() {
    let mut w = world(2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    place(&mut w, a, Vec3::new(-40.0, 0.0, 0.0), Vec3::X);
    place(&mut w, b, Vec3::new(40.0, 0.0, 0.0), Vec3::X);
    lead(&mut w, a);
    let near = Vec3::new(-50.0, 0.0, 0.0); // 10 u: too close
    let mid = Vec3::new(-40.0, 10.0, 45.0); // 46 u
    let far = Vec3::new(50.0, 0.0, 0.0);
    w.set_nav_graph(NavGraph::from_network(&[], &[], &[far, near, mid]));
    w.schedule_event(w.tick() + 1, EventKind::TaxDrone);
    step(&mut w, &mut t, &[]);
    step(&mut w, &mut t, &[]);
    assert_eq!(w.mood_view().target, Some(a));
    let d = drone_of(&w).expect("the drone waits at its entry while announced");
    assert_eq!(d.pos, mid);
    assert_eq!(d.state, [drone::ROLE_TAX, 0, drone::DRONE_WAITING, 100]);
    // It waits there for the whole telegraph.
    for _ in 0..ANNOUNCE - 3 {
        step(&mut w, &mut t, &[]);
    }
    assert_eq!(drone_of(&w).unwrap().pos, mid);
    // Without a network it enters from the boundary face nearest the leader.
    let mut w = world(2);
    place(&mut w, a, Vec3::new(-40.0, 0.0, 0.0), Vec3::X);
    lead(&mut w, a);
    w.schedule_event(w.tick() + 1, EventKind::TaxDrone);
    step(&mut w, &mut t, &[]);
    step(&mut w, &mut t, &[]);
    let d = drone_of(&w).unwrap();
    assert!(d.pos.x < -55.0 && t.sdf(d.pos) >= d.radius, "{:?}", d.pos);
}

#[test]
fn tax_drone_chases_at_22_and_taxes_ten_percent_per_touch_five_times() {
    let mut w = world(2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    place(&mut w, a, Vec3::new(-20.0, 0.0, 0.0), Vec3::X);
    place(&mut w, b, Vec3::new(40.0, 10.0, 0.0), Vec3::X);
    lead(&mut w, a);
    let entry = Vec3::new(45.0, 0.0, 40.0);
    w.set_nav_graph(NavGraph::from_network(&[], &[], &[entry]));
    let mut events = force(&mut w, &mut t, EventKind::TaxDrone);
    let start = w.tick();
    let mut top = 0.0f32;
    let mut touches: Vec<(u64, f32, f32)> = Vec::new();
    while w.mood_view().event == Some(EventKind::TaxDrone) && w.tick() < start + 30 * 60 {
        let before = w.ship(a).unwrap().energy;
        let ev = step(&mut w, &mut t, &[]);
        if let Some(d) = drone_of(&w) {
            top = top.max(d.vel.length());
        }
        let n = outcomes(&ev, EventKind::TaxDrone).len();
        if n > 0 {
            touches.push((w.tick(), before, w.ship(a).unwrap().energy));
        }
        events.extend(ev);
    }
    assert!((top - 22.0).abs() < 0.05, "cruise speed {top}");
    let cues = outcomes(&events, EventKind::TaxDrone);
    assert_eq!(
        cues.iter().map(|c| c.value as u8).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5],
        "five touches, numbered"
    );
    assert!(cues.iter().all(|c| c.target == Some(a)));
    for (i, &(tick, before, after)) in touches.iter().enumerate() {
        // 10 % of the current energy (plus one tick of regeneration).
        let regen = w.ship_types().get(ShipType::Tinker).energy_regen / 60.0;
        assert!(
            (after - (before * 0.9 + regen)).abs() < 0.2,
            "touch {i}: {before} -> {after}"
        );
        if i > 0 {
            assert!(tick - touches[i - 1].0 >= 120, "2 s apart");
        }
    }
    // After the fifth touch it leaves: the active phase ends early.
    let fifth = touches[4].0;
    assert!(fifth < start + 25 * 60);
    assert_eq!(w.mood_view().event, None);
    assert!(w.tick() <= fifth + 91 + 1, "ending phase of 1.5 s");
    assert!(
        props(&w, EntityKind::Drone).is_empty(),
        "removed at the end"
    );
}

#[test]
fn tax_drone_follows_the_network_through_the_doorway() {
    let mut t = wall_room();
    for graph in [true, false] {
        let mut w = world(2);
        let (a, b) = (PlayerId(0), PlayerId(1));
        place(&mut w, a, Vec3::new(30.0, 0.0, -20.0), Vec3::X);
        place(&mut w, b, Vec3::new(30.0, 0.0, 40.0), Vec3::X);
        lead(&mut w, a);
        if graph {
            w.set_nav_graph(wall_graph());
        } else {
            // Same entry, no network.
            w.set_nav_graph(NavGraph::from_network(
                &[],
                &[],
                &[Vec3::new(-50.0, 0.0, -40.0)],
            ));
        }
        let mut events = force(&mut w, &mut t, EventKind::TaxDrone);
        let entry = Vec3::new(-50.0, 0.0, -40.0);
        assert!(
            drone_of(&w).unwrap().pos.distance(entry) < 0.1,
            "enters on the far side"
        );
        let start = w.tick();
        let mut first = None;
        while w.tick() < start + 10 * 60 && first.is_none() {
            let ev = step(&mut w, &mut t, &[]);
            if !outcomes(&ev, EventKind::TaxDrone).is_empty() {
                first = Some(w.tick() - start);
            }
            if let Some(d) = drone_of(&w) {
                assert!(t.sdf(d.pos) >= d.radius - 0.05, "never in the wall");
            }
            events.extend(ev);
        }
        if graph {
            let secs = first.expect("reaches the leader") as f32 / 60.0;
            // The way through the door is about 130 u: under 8 s at 22 u/s.
            assert!(secs < 8.0, "{secs} s");
        } else {
            assert!(
                first.is_none(),
                "without the network it is stuck at the wall"
            );
        }
    }
}

#[test]
fn tax_drone_shot_down_bursts_into_two_scrap_per_energy_taken() {
    let mut w = world(2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    place(&mut w, a, Vec3::new(-20.0, 0.0, 0.0), Vec3::X);
    place(&mut w, b, Vec3::new(10.0, 0.0, -30.0), Vec3::X);
    lead(&mut w, a);
    w.ship_mut(a).unwrap().energy = 40.0;
    w.set_nav_graph(NavGraph::from_network(
        &[],
        &[],
        &[Vec3::new(-40.0, 0.0, 30.0)],
    ));
    let mut events = force(&mut w, &mut t, EventKind::TaxDrone);
    // Let it take one touch, then shoot it down.
    while outcomes(&events, EventKind::TaxDrone).is_empty() {
        events.extend(step(&mut w, &mut t, &[]));
    }
    let mut shot = None;
    for _ in 0..600 {
        let Some(d) = drone_of(&w) else { break };
        assert_eq!(d.hp > 0.0, d.state[3] > 0);
        let input = fire_at(&mut w, b, d.pos);
        let ev = step(&mut w, &mut t, &[input]);
        if let Some(c) = outcomes(&ev, EventKind::TaxDrone)
            .into_iter()
            .find(|c| c.value as u8 == OUTCOME_SHOT_DOWN)
        {
            shot = Some(c);
        }
        events.extend(ev);
        if shot.is_some() {
            break;
        }
    }
    let shot = shot.expect("shot down");
    assert_eq!(shot.source, Some(b), "credited to the shooter");
    assert!(drone_of(&w).is_none());
    // 4 energy taken (10 % of 40) or a little more with regeneration;
    // at most 20.
    let pile = w
        .scrap_piles()
        .iter()
        .find(|p| p.pos.distance(shot.pos) < 1e-3)
        .expect("a scrap pile where it burst");
    assert!(pile.amount >= 8.0 && pile.amount <= 20.0, "{}", pile.amount);
    // The event goes into its ending phase and ends.
    for _ in 0..100 {
        step(&mut w, &mut t, &[]);
    }
    assert_eq!(w.mood_view().event, None);
}

#[test]
fn tax_drone_scrap_is_capped_at_20_and_it_has_40_hp() {
    let mut w = world(2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    place(&mut w, a, Vec3::new(-20.0, 0.0, 0.0), Vec3::X);
    place(&mut w, b, Vec3::new(10.0, 0.0, -30.0), Vec3::X);
    lead(&mut w, a);
    let mut events = force(&mut w, &mut t, EventKind::TaxDrone);
    assert_eq!(drone_of(&w).unwrap().hp, 40.0);
    // Two touches at full energy (100 + 90) = 19 energy taken -> 38 scrap.
    while outcomes(&events, EventKind::TaxDrone).len() < 2 {
        events.extend(step(&mut w, &mut t, &[]));
    }
    let mut hits = 0;
    let mut hp = 40.0;
    while let Some(d) = drone_of(&w) {
        if d.hp < hp {
            assert!((hp - d.hp - 6.0).abs() < 1e-4, "a bolt does 6");
            hits += 1;
            hp = d.hp;
        }
        let input = fire_at(&mut w, b, d.pos);
        let ev = step(&mut w, &mut t, &[input]);
        assert!(w.tick() < 100_000);
        if let Some(c) = outcomes(&ev, EventKind::TaxDrone)
            .into_iter()
            .find(|c| c.value as u8 == OUTCOME_SHOT_DOWN)
        {
            hits += 1;
            let pile = w
                .scrap_piles()
                .iter()
                .find(|p| p.pos.distance(c.pos) < 1e-3)
                .unwrap();
            assert_eq!(pile.amount, 20.0);
            break;
        }
    }
    // Scrap Laser bolts do 6 damage: 7 hits for 40 hp.
    assert_eq!(hits, 7);
}

// ----- Pizza delivery --------------------------------------------------------------

#[test]
fn pizza_restores_full_hull_after_three_seconds_held_in_total() {
    let mut w = world(2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    place(&mut w, a, Vec3::new(-20.0, 0.0, 0.0), Vec3::X);
    place(&mut w, b, Vec3::new(40.0, 0.0, 0.0), Vec3::X);
    w.ship_mut(a).unwrap().hull = 20.0;
    w.set_nav_graph(NavGraph::from_network(
        &[],
        &[],
        &[Vec3::new(-50.0, 0.0, 50.0)],
    ));
    let mut events = force(&mut w, &mut t, EventKind::PizzaDelivery);
    assert_eq!(w.mood_view().target, Some(a), "the only damaged ship");
    let d = drone_of(&w).unwrap();
    assert_eq!(d.owner, Some(a));
    let start = w.tick();
    let mut top = 0.0f32;
    // Fly in.
    while drone_of(&w).unwrap().pos.distance(w.ship(a).unwrap().pos) > 4.0 {
        events.extend(step(&mut w, &mut t, &[]));
        top = top.max(drone_of(&w).unwrap().vel.length());
        assert!(w.tick() < start + 600);
    }
    assert!((top - 18.0).abs() < 0.05, "{top}");
    // Hold still 1.5 s, drift away from it for a second, hold again.
    let hold = |w: &mut World, t: &mut BoxRoomTerrain, n: u32, ev: &mut Vec<Event>| {
        for _ in 0..n {
            ev.extend(step(w, t, &[]));
        }
    };
    let progress = |w: &World| drone_of(w).map_or(255, |d| d.state[1]);
    let before = progress(&w);
    hold(&mut w, &mut t, 90, &mut events);
    let half = progress(&w);
    // 90 of 180 ticks: half of 255.
    assert!((126..=128).contains(&(half - before)), "{before} -> {half}");
    for _ in 0..60 {
        w.ship_mut(a).unwrap().vel = Vec3::new(0.0, 3.0, 0.0);
        events.extend(step(&mut w, &mut t, &[]));
    }
    assert_eq!(progress(&w), half, "no progress while moving");
    assert!(w.ship(a).unwrap().hull < 25.0);
    // Back to rest and wait for the rest.
    w.ship_mut(a).unwrap().vel = Vec3::ZERO;
    let mut delivered = None;
    for _ in 0..240 {
        let ev = step(&mut w, &mut t, &[]);
        if let Some(c) = outcomes(&ev, EventKind::PizzaDelivery).first() {
            delivered = Some((w.tick(), *c));
        }
        events.extend(ev);
        if delivered.is_some() {
            break;
        }
    }
    let (_, c) = delivered.expect("delivered");
    assert_eq!(c.value as u8, OUTCOME_DELIVERED);
    assert_eq!(c.target, Some(a));
    assert_eq!(w.ship(a).unwrap().hull, 100.0, "full hull");
    assert_eq!(w.mood_view().phase, EventPhase::Ending, "it leaves");
}

#[test]
fn pizza_drone_ignores_its_recipient_and_drops_50_scrap_when_shot() {
    let mut w = world(2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    place(&mut w, a, Vec3::new(-20.0, 0.0, 0.0), Vec3::X);
    place(&mut w, b, Vec3::new(20.0, 0.0, 0.0), Vec3::X);
    w.ship_mut(a).unwrap().hull = 20.0;
    w.set_nav_graph(NavGraph::from_network(
        &[],
        &[],
        &[Vec3::new(10.0, 0.0, -50.0)],
    ));
    force(&mut w, &mut t, EventKind::PizzaDelivery);
    assert_eq!(drone_of(&w).unwrap().hp, 30.0);
    // The recipient fires at it: the shots pass through.
    for _ in 0..90 {
        let d = drone_of(&w).unwrap();
        let input = fire_at(&mut w, a, d.pos);
        step(&mut w, &mut t, &[input]);
    }
    assert_eq!(drone_of(&w).unwrap().hp, 30.0, "its own pizza is safe");
    // Someone else shoots it down.
    let mut shot = None;
    for _ in 0..900 {
        let Some(d) = drone_of(&w) else { break };
        let input = fire_at(&mut w, b, d.pos);
        let ev = step(&mut w, &mut t, &[input]);
        shot = shot.or(outcomes(&ev, EventKind::PizzaDelivery).first().copied());
    }
    let c = shot.expect("shot down");
    assert_eq!(c.value as u8, OUTCOME_SHOT_DOWN);
    assert_eq!(c.source, Some(b));
    let pile = w
        .scrap_piles()
        .iter()
        .find(|p| p.pos.distance(c.pos) < 1e-3)
        .expect("the pizza as scrap");
    assert_eq!(pile.amount, 50.0);
    assert!(w.ship(a).unwrap().hull < 21.0, "not delivered");
}

// ----- Customer survey ----------------------------------------------------------------

#[test]
fn survey_star_columns() {
    let c = Vec3::ZERO;
    for (x, star) in [
        (-1.2, 1),
        (-0.6, 2),
        (0.0, 3),
        (0.2, 3),
        (0.6, 4),
        (1.0, 5),
        (3.0, 5),
        (-3.0, 1),
    ] {
        assert_eq!(
            star_at(Vec3::new(x, 0.3, 0.0), c, Vec3::X, 1.25),
            star,
            "{x}"
        );
    }
}

#[test]
fn survey_parks_in_front_of_the_leader_and_follows_its_view() {
    let mut w = world(2);
    let mut t = room();
    let a = PlayerId(0);
    place(&mut w, a, Vec3::new(-10.0, 0.0, 0.0), Vec3::X);
    lead(&mut w, a);
    w.schedule_event(w.tick() + 1, EventKind::CustomerSurvey);
    for _ in 0..ANNOUNCE {
        step(&mut w, &mut t, &[]);
        assert!(drone_of(&w).is_none(), "no drone while announced");
    }
    step(&mut w, &mut t, &[]);
    assert_eq!(w.mood_view().phase, EventPhase::Active);
    let check = |w: &World| {
        let s = w.ship(a).unwrap();
        let d = drone_of(w).unwrap();
        let (pos, rot) = panel_pose(s.pos, s.orientation, 2.5);
        assert!(d.pos.distance(pos) < 1e-4, "{:?} vs {pos:?}", d.pos);
        assert!(d.rot.dot(rot).abs() > 0.9999);
        assert!(
            d.rot.forward().dot(s.orientation.forward()) < -0.999,
            "faces the leader"
        );
        assert_eq!(d.hp, 15.0);
        assert_eq!(d.radius, 1.25);
    };
    check(&w);
    // The leader turns and moves: the panel stays in its view.
    let mut cmd = InputCmd::idle(0).with_thrust(Vec3::new(0.3, 0.0, -1.0));
    cmd.look_yaw = 0.03;
    cmd.look_pitch = 0.01;
    for _ in 0..120 {
        step(&mut w, &mut t, &[(a, cmd)]);
        check(&w);
    }
    // Nobody answers: rating 0 at the end of the 12 s.
    let mut events = Vec::new();
    while w.mood_view().event == Some(EventKind::CustomerSurvey) {
        events.extend(step(&mut w, &mut t, &[]));
    }
    let cues = outcomes(&events, EventKind::CustomerSurvey);
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].value as u8, 0);
    assert!(props(&w, EntityKind::Drone).is_empty());
}

#[test]
fn survey_rating_is_the_star_the_destroying_shot_went_through() {
    for (offset, star) in [(-0.9f32, 1u8), (0.0, 3), (0.9, 5)] {
        let mut w = world(2);
        let mut t = room();
        let (a, b) = (PlayerId(0), PlayerId(1));
        // The leader looks along +x; b faces it from 20 u ahead.
        place(&mut w, a, Vec3::new(-10.0, 0.0, 0.0), Vec3::X);
        place(&mut w, b, Vec3::new(12.5, 0.0, 0.0), Vec3::NEG_X);
        lead(&mut w, a);
        force(&mut w, &mut t, EventKind::CustomerSurvey);
        let leader_right = w.ship(a).unwrap().orientation.right();
        let mut rated = None;
        for _ in 0..600 {
            let Some(d) = drone_of(&w) else { break };
            // Shoot parallel to the view through the column at `offset`.
            let from = d.pos + Vec3::new(22.5, 0.0, 0.0) + leader_right * offset;
            let s = w.ship_mut(b).unwrap();
            s.pos = from;
            s.vel = Vec3::ZERO;
            let input = fire_at(&mut w, b, d.pos + leader_right * offset);
            let ev = step(&mut w, &mut t, &[input]);
            rated = rated.or(outcomes(&ev, EventKind::CustomerSurvey).first().copied());
        }
        let c = rated.expect("rated");
        assert_eq!(c.value as u8, star, "offset {offset}");
        assert_eq!(c.source, Some(b));
        assert_eq!(c.target, Some(a));
        assert_eq!(w.mood_view().phase, EventPhase::Ending);
    }
}

// ----- Mimic pickups --------------------------------------------------------------------

fn mimic_world(ships: usize) -> World {
    let mut w = world(ships);
    // Park every ship far from the pickups.
    for i in 0..ships {
        let p = PlayerId(i as u16);
        place(
            &mut w,
            p,
            Vec3::new(-50.0 + 4.0 * i as f32, 15.0, -50.0),
            Vec3::X,
        );
    }
    w
}

fn weapon_pickups(w: &World) -> usize {
    w.pickups()
        .iter()
        .filter(|p| {
            p.content.is_some_and(|(_, k)| {
                matches!(
                    k,
                    crate::PickupKind::Weapon { .. } | crate::PickupKind::Upgrade { .. }
                )
            })
        })
        .count()
}

#[test]
fn mimics_count_three_plus_a_quarter_of_the_players() {
    for (ships, want) in [(2usize, 3usize), (4, 4), (8, 5)] {
        let mut w = mimic_world(ships);
        let mut t = room();
        assert!(weapon_pickups(&w) >= 6, "{}", weapon_pickups(&w));
        force(&mut w, &mut t, EventKind::MimicPickups);
        let mimics = props(&w, EntityKind::Mimic);
        assert_eq!(mimics.len(), want, "{ships} ships");
        let listed: Vec<u8> = w
            .mood_view()
            .params
            .iter()
            .copied()
            .filter(|p| *p != NO_SPAWN)
            .collect();
        assert_eq!(listed.len(), want);
        for m in &mimics {
            let sp = &w.pickups()[m.vars[0] as usize];
            assert_eq!(m.pos, sp.content_pos(), "grows where its pickup lies");
            assert_eq!(m.hp, 25.0);
            let (_, kind) = sp.content.unwrap();
            match kind {
                crate::PickupKind::Weapon { weapon, .. } => assert_eq!(m.state[0], weapon.0),
                crate::PickupKind::Upgrade { weapon } => {
                    assert_eq!((m.state[0], m.state[2] & 1), (weapon.0, 1))
                }
                _ => panic!("only weapons and upgrades turn"),
            }
        }
    }
}

#[test]
fn mimics_hunt_within_30_u_bite_12_a_second_and_return_home() {
    let mut w = mimic_world(2);
    let mut t = room();
    force(&mut w, &mut t, EventKind::MimicPickups);
    let mimics = props(&w, EntityKind::Mimic);
    let first = mimics[0].clone();
    // Ships beyond 30 u: the mimics sit at home.
    for _ in 0..30 {
        step(&mut w, &mut t, &[]);
    }
    for m in props(&w, EntityKind::Mimic) {
        let home = w.pickups()[m.vars[0] as usize].content_pos();
        assert!(m.pos.distance(home) < 1e-3);
        assert_eq!(m.state[2] & FLAG_HUNTING, 0);
    }
    // A ship comes within 25 u of the first mimic.
    let a = PlayerId(0);
    place(&mut w, a, first.pos + Vec3::new(0.0, 0.0, -25.0), Vec3::X);
    let mut top = 0.0f32;
    let mut bites: Vec<(u64, Vec<Event>)> = Vec::new();
    for _ in 0..(5 * 60) {
        place(&mut w, a, first.pos + Vec3::new(0.0, 0.0, -25.0), Vec3::X);
        w.ship_mut(a).unwrap().hull = 100.0;
        let ev = step(&mut w, &mut t, &[]);
        let m = props(&w, EntityKind::Mimic)
            .into_iter()
            .find(|m| m.id == first.id)
            .unwrap();
        top = top.max(m.vel.length());
        // Bites of this mimic (the cue sits at the biting mimic).
        if outcomes(&ev, EventKind::MimicPickups).iter().any(|c| {
            c.value as u8 == OUTCOME_BITE && c.target == Some(a) && c.pos.distance(m.pos) < 1.0
        }) {
            bites.push((w.tick(), ev));
        }
    }
    assert!(top <= 12.0 + 1e-3 && top > 11.9, "{top}");
    assert!(bites.len() >= 3, "{}", bites.len());
    for w2 in bites.windows(2) {
        assert!(w2[1].0 - w2[0].0 >= 60, "1 s apart per mimic");
    }
    for (_, ev) in &bites {
        let dmg = ev
            .iter()
            .find_map(|e| match e {
                Event::Damaged {
                    target,
                    attacker,
                    amount,
                    ..
                } if *target == a => Some((*attacker, *amount)),
                _ => None,
            })
            .expect("damage");
        assert_eq!(dmg.0, None, "no kill credit");
        assert!((dmg.1 - 12.0).abs() < 1e-3, "{}", dmg.1);
    }
    // The ship leaves: the mimic hops home.
    place(&mut w, a, Vec3::new(50.0, 15.0, -50.0), Vec3::X);
    for _ in 0..(4 * 60) {
        step(&mut w, &mut t, &[]);
    }
    let m = props(&w, EntityKind::Mimic)
        .into_iter()
        .find(|m| m.id == first.id)
        .unwrap();
    assert!(m.pos.distance(first.pos) < 0.2, "{:?}", m.pos);
}

#[test]
fn mimicked_pickups_cannot_be_taken_and_a_killed_mimic_drops_its_content() {
    let mut w = mimic_world(2);
    let mut t = room();
    force(&mut w, &mut t, EventKind::MimicPickups);
    let (a, b) = (PlayerId(0), PlayerId(1));
    let mimics = props(&w, EntityKind::Mimic);
    let (m0, m1) = (mimics[0].clone(), mimics[1].clone());
    let (i0, i1) = (m0.vars[0] as usize, m1.vars[0] as usize);
    // A ship parked on a mimicked pickup's spot does not take it (the
    // mimic bites it instead).
    let content0 = w.pickups()[i1].content;
    for _ in 0..30 {
        let spot = w.pickups()[i1].pos;
        place(&mut w, a, spot, Vec3::X);
        w.ship_mut(a).unwrap().hull = 100.0;
        step(&mut w, &mut t, &[]);
    }
    assert_eq!(w.pickups()[i1].content, content0, "untouchable");
    place(&mut w, a, Vec3::new(-50.0, 15.0, -50.0), Vec3::X);
    // b lures mimic 0 away from home and shoots it.
    let lure = m0.pos + Vec3::new(0.0, 12.0, -14.0);
    let mut killed = None;
    for _ in 0..1200 {
        let Some(m) = props(&w, EntityKind::Mimic)
            .into_iter()
            .find(|m| m.id == m0.id)
        else {
            break;
        };
        place(&mut w, b, lure, Vec3::X);
        w.ship_mut(b).unwrap().hull = 100.0;
        let input = if m.pos.distance(m0.pos) > 5.0 {
            Some(fire_at(&mut w, b, m.pos))
        } else {
            None
        };
        let ev = step(&mut w, &mut t, input.as_slice());
        if let Some(c) = outcomes(&ev, EventKind::MimicPickups)
            .into_iter()
            .find(|c| c.value as u8 == OUTCOME_SHOT_DOWN)
        {
            killed = Some(c);
        }
    }
    let c = killed.expect("killed");
    assert_eq!(c.source, Some(b));
    let sp = &w.pickups()[i0];
    assert!(sp.content.is_some());
    assert!(
        sp.content_pos().distance(c.pos) < 1e-3,
        "dropped where it died"
    );
    assert!(sp.content_pos().distance(sp.pos) > 4.0);
    assert!(!w.mood_view().params.contains(&(i0 as u8)), "free again");
    // Now b can take it where it lies.
    let ammo_before = w.ship(b).unwrap().weapons.len();
    for _ in 0..3 {
        place(&mut w, b, c.pos, Vec3::X);
        step(&mut w, &mut t, &[]);
    }
    let sp = &w.pickups()[i0];
    assert!(
        sp.content.is_none() || w.ship(b).unwrap().weapons.len() >= ammo_before,
        "taken"
    );
    assert!(sp.content.is_none(), "taken where it dropped");
    // The event ends: survivors turn back into their pickups at home.
    while w.mood_view().event.is_some() {
        step(&mut w, &mut t, &[]);
    }
    assert!(props(&w, EntityKind::Mimic).is_empty());
    let sp1 = w.pickups()[i1].clone();
    assert_eq!(sp1.content, content0);
    assert_eq!(sp1.offset, Vec3::ZERO);
    for _ in 0..3 {
        place(&mut w, a, sp1.pos, Vec3::X);
        step(&mut w, &mut t, &[]);
    }
    assert!(w.pickups()[i1].content.is_none(), "takeable again");
}

#[test]
fn mimics_need_weapon_pickups_and_skip_ones_taken_during_the_announcement() {
    // The eligibility test reads the pickups in play.
    let w = mimic_world(2);
    assert!(weapon_pickups(&w) >= 4);
    let mut w = mimic_world(2);
    let mut t = room();
    w.schedule_event(w.tick() + 1, EventKind::MimicPickups);
    step(&mut w, &mut t, &[]);
    step(&mut w, &mut t, &[]);
    let picked: Vec<u8> = w
        .mood_view()
        .params
        .iter()
        .copied()
        .filter(|p| *p != NO_SPAWN)
        .collect();
    assert_eq!(picked.len(), 3);
    // Someone grabs a picked pickup during the telegraph.
    let a = PlayerId(0);
    let grab = w.pickups()[picked[0] as usize].pos;
    for _ in 0..3 {
        place(&mut w, a, grab, Vec3::X);
        step(&mut w, &mut t, &[]);
    }
    assert!(w.pickups()[picked[0] as usize].content.is_none());
    place(&mut w, a, Vec3::new(-50.0, 15.0, -50.0), Vec3::X);
    while w.mood_view().phase != EventPhase::Active {
        step(&mut w, &mut t, &[]);
    }
    step(&mut w, &mut t, &[]);
    assert_eq!(props(&w, EntityKind::Mimic).len(), 2);
    assert!(!w.mood_view().params.contains(&picked[0]));
}

// ----- Gossip wall ------------------------------------------------------------------------

#[test]
fn gossip_face_grows_on_the_rock_nearest_the_ships() {
    let mut w = world(2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    // Centroid at (52, 0, 10): the +x wall (x = 60) is nearest.
    place(&mut w, a, Vec3::new(50.0, 4.0, 0.0), Vec3::X);
    place(&mut w, b, Vec3::new(54.0, -4.0, 20.0), Vec3::X);
    w.schedule_event(w.tick() + 1, EventKind::GossipWall);
    step(&mut w, &mut t, &[]);
    step(&mut w, &mut t, &[]);
    let f = props(&w, EntityKind::GossipFace);
    assert_eq!(f.len(), 1, "grows during the announcement");
    let f = &f[0];
    assert!(
        (f.pos - Vec3::new(60.0, 0.0, 10.0)).length() < 0.1,
        "{:?}",
        f.pos
    );
    assert!(
        f.rot.forward().dot(Vec3::NEG_X) > 0.999,
        "looks out of the rock"
    );
    assert_eq!(f.state[0], 0);
    while w.mood_view().phase != EventPhase::Active {
        step(&mut w, &mut t, &[]);
    }
    step(&mut w, &mut t, &[]);
    assert_eq!(props(&w, EntityKind::GossipFace)[0].state[0], 1, "talking");
    while w.mood_view().event.is_some() {
        step(&mut w, &mut t, &[]);
    }
    assert!(props(&w, EntityKind::GossipFace).is_empty());
}

#[test]
fn gossip_reveals_cloaked_ships_for_15_s() {
    let mut r = rules();
    r.starting_loadout.gadget = Some("cloak".into());
    r.gadgets
        .insert("cloak".into(), Availability::LoadoutAndPickup);
    let mut w = world_with(r, 2);
    let mut t = room();
    let (a, b) = (PlayerId(0), PlayerId(1));
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    place(&mut w, b, Vec3::new(40.0, 0.0, 0.0), Vec3::NEG_Z);
    let gadget = [(a, InputCmd::idle(0).with_buttons(Buttons::GADGET))];
    step(&mut w, &mut t, &gadget);
    assert!(!w.ship_visible_to(b, a), "cloaked");
    w.schedule_event(w.tick() + 1, EventKind::GossipWall);
    let mut seen = 0u64;
    let mut hidden_in_announce = 0u64;
    let mut after = 0u64;
    let mut ended = false;
    for _ in 0..(25 * 60) {
        w.ship_mut(a).unwrap().energy = 100.0;
        step(&mut w, &mut t, &[]);
        let v = w.mood_view();
        let visible = w.ship_visible_to(b, a);
        match (v.event, v.phase) {
            (Some(EventKind::GossipWall), EventPhase::Active) => {
                assert!(visible, "revealed at tick {}", w.tick());
                seen += 1;
            }
            (Some(EventKind::GossipWall), EventPhase::Announce) => {
                hidden_in_announce += (!visible) as u64;
            }
            (None, _) => {
                ended = true;
                assert!(!visible, "hidden again");
                after += 1;
            }
            _ => {}
        }
    }
    assert!(ended && after > 0);
    assert_eq!(seen, Tick::ticks_for_secs(15.0));
    assert!(hidden_in_announce > 170, "nothing changes while announced");
    assert!(w.ship(a).unwrap().gadget_active, "still cloaked throughout");
}
