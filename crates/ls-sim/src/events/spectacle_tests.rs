//! Scenario tests of B3's spectacles (`docs/milestone-4-plan.md` §3.2): Duck
//! parade, Hat rain, Cat paw, Disco inferno, Tired announcer. Every rule of
//! the plan's event table has a test here; the replays are in
//! `crate::event_replays`.

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit, TerrainQuery};
use ls_math::{Quat, Vec3};

use super::*;
use crate::behavior::{CueKind, DamageSource};
use crate::director::{self, DirectorEvent};
use crate::entity::{EntityKind, ReplicatedKind};
use crate::events::duck_parade::{route, Convoy, Route};
use crate::events::hat_rain::{hat_baron, hat_count, phase};
use crate::events::tired_announcer::NARRATED_ONLY;
use crate::{
    Buttons, Event, InputCmd, MatchSetup, RuleSet, ShipType, SpawnPoint, World, WorldConfig,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const SEED: u64 = 0xb3;

fn spawn(pos: Vec3, team: u8) -> SpawnPoint {
    SpawnPoint {
        pos,
        team,
        orientation: Quat::IDENTITY,
    }
}

fn base_setup() -> MatchSetup {
    MatchSetup {
        spawns: vec![
            spawn(Vec3::new(-30.0, 15.0, 30.0), 0),
            spawn(Vec3::new(-30.0, 15.0, -30.0), 1),
        ],
        ..Default::default()
    }
}

fn world_with(setup: MatchSetup, ships: usize) -> (World, Vec<PlayerId>) {
    let mut cfg = WorldConfig::paint_the_void(SEED, setup);
    let mut rules = RuleSet::paint_the_void();
    rules.mutators.clear();
    cfg.rules = rules;
    let mut w = World::new(cfg).unwrap();
    let ids = (0..ships)
        .map(|i| w.add_player(Some((i % 2) as u8), ShipType::Tinker).unwrap())
        .collect();
    (w, ids)
}

/// 80 x 40 x 80 u room centred on the origin.
fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(80.0, 40.0, 80.0))
}

fn park(w: &mut World, id: PlayerId, pos: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
}

/// Steps (idle inputs) until `kind` is in `phase`; returns the events.
fn step_until(
    w: &mut World,
    t: &mut BoxRoomTerrain,
    kind: EventKind,
    phase: EventPhase,
) -> Vec<Event> {
    let mut all = Vec::new();
    for _ in 0..2000 {
        let v = w.mood_view();
        if v.event == Some(kind) && v.phase == phase {
            return all;
        }
        all.extend(w.step(t, &[]));
    }
    panic!("{kind:?} never reached {phase:?}");
}

fn props(w: &World, kind: ReplicatedKind) -> Vec<&crate::Entity> {
    w.entities()
        .iter()
        .filter(|e| e.kind == EntityKind::Prop(kind))
        .collect()
}

fn kill(w: &mut World, t: &mut BoxRoomTerrain, id: PlayerId) {
    w.hook_for_test(t, |ctx| ctx.adjust_hull(id, -10_000.0, true));
    assert!(!w.ship(id).unwrap().alive);
}

// ----- Duck parade --------------------------------------------------------------

fn straight_route() -> Vec<Vec3> {
    vec![Vec3::new(-36.0, 0.0, 0.0), Vec3::new(36.0, 0.0, 0.0)]
}

fn parade_setup(routes: Vec<Vec<Vec3>>) -> MatchSetup {
    MatchSetup {
        routes,
        ..base_setup()
    }
}

#[test]
fn ducks_drift_along_the_route_in_convoy_and_leave_at_its_end() {
    // A bent route of 65 u: 13 s for the convoy to cross.
    let pts = vec![
        Vec3::new(-35.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 30.0),
    ];
    let setup = parade_setup(vec![pts.clone()]);
    let (mut w, _) = world_with(setup.clone(), 2);
    let mut t = room();
    w.schedule_event(10, EventKind::DuckParade);
    step_until(&mut w, &mut t, EventKind::DuckParade, EventPhase::Active);
    let view = w.mood_view();
    let r = route(&setup, &view.params).unwrap();
    let expect = if view.params[1] & 1 == 1 {
        Route::new(pts.iter().rev().copied()).unwrap()
    } else {
        Route::new(pts.clone()).unwrap()
    };
    assert_eq!(r, expect);
    assert!((r.length() - 65.0).abs() < 1e-3);
    let def = EventKind::DuckParade.def();
    let convoy = Convoy::new(def, &r);
    assert_eq!(convoy.ducks, 5);
    assert_eq!(convoy.radius, 3.5);
    assert_eq!(convoy.speed, 5.0);
    assert_eq!(convoy.spacing, 9.0);
    // Ends when the last duck reaches the end: 13 s, not 45.
    let end = view.phase_end_tick;
    assert_eq!(end, view.active_tick + 13 * 60 + 1);
    let mut obstacles = Vec::new();
    let mut ended = None;
    for _ in 0..(15 * 60) {
        let evs = w.step(&mut t, &[]);
        let now = w.tick();
        if evs
            .iter()
            .any(|e| matches!(e, Event::Director(DirectorEvent::Ended { .. })))
        {
            ended = Some(now);
        }
        let v = w.mood_view();
        if v.event != Some(EventKind::DuckParade) {
            continue;
        }
        // The prop entities are where the pure function puts the ducks.
        director::obstacles(&v, SEED, now, &setup, &mut obstacles);
        let ducks = props(&w, ReplicatedKind::Duck);
        assert!(ducks.len() <= 5);
        assert!(
            !ducks.is_empty() || now + 2 >= v.phase_end_tick,
            "tick {now}"
        );
        if now > v.active_tick + 1 {
            assert_eq!(ducks.len(), obstacles.len(), "tick {now}");
        }
        let secs = (now - v.active_tick) as f32 / 60.0;
        for d in &ducks {
            let i = d.state[0] as usize;
            let (p, dir) = r.at(convoy.arc(i, secs));
            if now > v.active_tick + 1 {
                assert!(d.pos.distance(p) < 1e-4, "duck {i} at {:?}", d.pos);
                assert!((d.vel - dir * 5.0).length() < 1e-4);
                assert!(obstacles.iter().any(|o| o.pos == d.pos && o.radius == 3.5));
            }
        }
        // Convoy: neighbours 9 u apart along the route.
        for o in &obstacles {
            assert_eq!(o.vel.length().round(), 5.0);
        }
    }
    assert_eq!(ended, Some(end));
    assert!(props(&w, ReplicatedKind::Duck).is_empty(), "ducks gone");
    assert_eq!(w.mood_view().event, None);
}

#[test]
fn ducks_ignore_rock_and_circle_belts_retrograde() {
    // A rock blob on the route: the ducks go straight through it.
    let setup = parade_setup(vec![straight_route()]);
    let (mut w, _) = world_with(setup.clone(), 2);
    let mut t = room();
    t.apply(TerrainEdit::AddSphere {
        center: Vec3::new(0.0, 0.0, 0.0),
        radius: 6.0,
    });
    w.schedule_event(10, EventKind::DuckParade);
    step_until(&mut w, &mut t, EventKind::DuckParade, EventPhase::Active);
    let mut in_rock = 0;
    for _ in 0..(12 * 60) {
        w.step(&mut t, &[]);
        let v = w.mood_view();
        let r = route(&setup, &v.params).unwrap();
        for d in props(&w, ReplicatedKind::Duck) {
            // On the centre line, whatever the rock.
            assert!(d.pos.y.abs() < 1e-4 && d.pos.z.abs() < 1e-4);
            assert!(d.pos.x >= r.points()[0].x.min(r.points()[1].x) - 1e-3);
            if t.sdf(d.pos) < 0.0 {
                in_rock += 1;
                assert_eq!(d.state[1], 0, "clearance 0 in rock");
            }
        }
    }
    assert!(in_rock > 0, "the convoy passed through the rock");

    // Belt: the ring's centre line, azimuth falling.
    let belt = test_belt();
    let mid = 0.5 * (belt.inner_radius + belt.outer_radius);
    let setup = MatchSetup {
        belt: Some(belt.clone()),
        ..parade_setup(vec![straight_route()])
    };
    let r = route(&setup, &[64, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    assert!((r.length() - core::f32::consts::TAU * mid).abs() < 0.01 * r.length());
    let mut last = belt.azimuth(r.at(0.0).0);
    assert!((last - core::f32::consts::FRAC_PI_2).abs() < 1e-3, "{last}");
    for k in 1..20 {
        let p = r.at(k as f32 * 5.0).0;
        assert!((belt.axis_distance(p) - mid).abs() < 0.2);
        assert!((p.y - belt.center.y).abs() < 1e-4);
        let a = belt.azimuth(p);
        assert!(a < last, "retrograde: {a} after {last}");
        last = a;
    }
}

#[test]
fn ducks_absorb_projectiles_without_damage_or_craters() {
    let setup = parade_setup(vec![straight_route()]);
    let (mut w, ids) = world_with(setup.clone(), 2);
    let (a, b) = (ids[0], ids[1]);
    let mut t = room();
    w.give_weapon(a, "bolt_rocket").unwrap();
    w.schedule_event(10, EventKind::DuckParade);
    step_until(&mut w, &mut t, EventKind::DuckParade, EventPhase::Active);
    w.step(&mut t, &[]);
    w.step(&mut t, &[]);
    // The lead duck; the shooter on one side, the target behind it.
    let lead = props(&w, ReplicatedKind::Duck)
        .into_iter()
        .find(|d| d.state[0] == 0)
        .unwrap()
        .pos;
    let aim_x = lead.x + 1.0;
    park(&mut w, a, Vec3::new(aim_x, 0.0, 14.0));
    park(&mut w, b, Vec3::new(aim_x, 0.0, -14.0));
    w.ship_mut(a).unwrap().orientation = Quat::IDENTITY; // facing -z
    let rocket = w.weapons().find("bolt_rocket").unwrap();
    w.ship_mut(a).unwrap().secondary = Some(rocket);
    let hull_b = w.ship(b).unwrap().hull;
    let mut absorbed = 0;
    for tick in 0..90u64 {
        let cmd = InputCmd::idle(w.tick() + 1).with_buttons(if tick == 0 {
            Buttons::FIRE_SECONDARY
        } else {
            Buttons::NONE
        });
        for e in w.step(&mut t, &[(a, cmd)]) {
            match e {
                Event::Cue(c) if c.kind == CueKind::EventHit => {
                    assert_eq!(c.value, ReplicatedKind::Duck as u16);
                    absorbed += 1;
                }
                Event::Damaged { .. } => panic!("no damage: {e:?}"),
                Event::TerrainEdit(_) => panic!("no crater: {e:?}"),
                _ => {}
            }
        }
    }
    assert_eq!(absorbed, 1, "the rocket hit the duck");
    assert!(w.projectiles().is_empty());
    assert_eq!(w.ship(b).unwrap().hull, hull_b);
}

#[test]
fn ducks_push_ships_aside_without_damage() {
    let setup = parade_setup(vec![straight_route()]);
    let (mut w, ids) = world_with(setup.clone(), 2);
    let a = ids[0];
    let mut t = room();
    w.schedule_event(10, EventKind::DuckParade);
    step_until(&mut w, &mut t, EventKind::DuckParade, EventPhase::Active);
    w.step(&mut t, &[]);
    let v = w.mood_view();
    let r = route(&setup, &v.params).unwrap();
    // Parked on the centre line 10 u ahead of the lead duck, a little off it.
    let lead_s = Convoy::new(EventKind::DuckParade.def(), &r).arc(0, 0.0) + 10.0;
    let (p, _) = r.at(lead_s);
    park(&mut w, a, p + Vec3::new(0.0, 0.5, 0.0));
    let hull = w.ship(a).unwrap().hull;
    let start = w.ship(a).unwrap().pos;
    let radius = w.tuning_for(ShipType::Tinker).radius;
    let mut obstacles = Vec::new();
    let mut touched = false;
    for _ in 0..(6 * 60) {
        for e in w.step(&mut t, &[]) {
            assert!(!matches!(e, Event::Damaged { .. }), "{e:?}");
        }
        let now = w.tick();
        director::obstacles(&w.mood_view(), SEED, now, &setup, &mut obstacles);
        let s = w.ship(a).unwrap();
        for o in &obstacles {
            let d = s.pos.distance(o.pos);
            // Pushed out every tick (the ship moved after the duck did).
            touched |= d < o.radius + radius + 0.5;
            assert!(d >= o.radius + radius - 0.2, "inside a duck: {d}");
        }
    }
    assert!(touched, "the convoy reached the ship");
    let s = w.ship(a).unwrap();
    assert_eq!(s.hull, hull, "no damage");
    assert!(s.pos.distance(start) > 2.0, "pushed aside");
}

/// A belt of radii 60-90 u around (0, 5, 0) (slots unused here).
fn test_belt() -> ls_math::level::BeltSite {
    use ls_math::level::{BeltSite, BeltSlot, BELT_WEDGES};
    BeltSite {
        center: Vec3::new(0.0, 5.0, 0.0),
        inner_radius: 60.0,
        outer_radius: 90.0,
        half_thickness: 8.0,
        cocoon_radius: 20.0,
        exit_radius: 6.0,
        gate_inner_radius: 52.0,
        gate_outer_radius: 98.0,
        slots: [BeltSlot {
            pos: Vec3::ZERO,
            orientation: Quat::IDENTITY,
        }; BELT_WEDGES],
        pickup_points: Vec::new(),
        boulders: Vec::new(),
    }
}

// ----- Hat rain -------------------------------------------------------------------

#[test]
fn hats_rain_from_the_upper_third_and_rest_on_rock() {
    for up in [Vec3::Y, Vec3::X] {
        let (mut w, ids) = world_with(base_setup(), 4);
        let mut t = room();
        if up != Vec3::Y {
            w.set_up(up);
        }
        // Ships parked in a corner, out of the way.
        for (k, id) in ids.iter().enumerate() {
            park(&mut w, *id, Vec3::new(-38.0, -18.0, -38.0 + 2.0 * k as f32));
        }
        w.schedule_event(10, EventKind::HatRain);
        step_until(&mut w, &mut t, EventKind::HatRain, EventPhase::Active);
        let total = hat_count(EventKind::HatRain.def(), 4);
        assert_eq!(total, 24 + 2 * 4);
        let b = t.bounds();
        let (lo, hi) = (b.min.dot(up), b.max.dot(up));
        let mut seen = std::collections::BTreeSet::new();
        let mut resting = 0;
        for _ in 0..(25 * 60) {
            w.step(&mut t, &[]);
            let hats = props(&w, ReplicatedKind::Hat);
            resting = resting.max(hats.iter().filter(|h| h.state[1] == phase::RESTING).count());
            for h in props(&w, ReplicatedKind::Hat) {
                if seen.insert(h.id) {
                    // Upper third along up, falling at 3 u/s against it.
                    assert!(h.pos.dot(up) >= lo + (hi - lo) * 2.0 / 3.0 - 0.06);
                    assert!(h.state[0] < 8);
                }
                match h.state[1] {
                    phase::FALLING => assert!((h.vel - up * -3.0).length() < 1e-5),
                    phase::RESTING => {
                        assert_eq!(h.vel, Vec3::ZERO);
                        assert!(t.sdf(h.pos) <= 0.8 + 0.06, "rests on rock");
                    }
                    p => panic!("phase {p}"),
                }
            }
        }
        assert_eq!(seen.len() as u32, total, "every hat rained (up {up:?})");
        // Along y a hat falls at most 40 u (13 s): nearly all rest before
        // the end; along x (80 u) the late ones are still falling.
        if up == Vec3::Y {
            assert!(resting + 2 >= total as usize, "{resting} resting");
        } else {
            assert!(resting > 0);
        }
        // The end of the event takes the hats nobody wears.
        assert!(props(&w, ReplicatedKind::Hat).is_empty());
    }
}

/// Ship `id` grabs `n` hats one after another and goes back to its corner.
fn give(w: &mut World, t: &mut BoxRoomTerrain, id: PlayerId, n: u8) {
    for _ in 0..n {
        assert!(grab_hat(w, id));
        w.step(t, &[]);
        park(w, id, Vec3::new(-38.0, -18.0, id.0 as f32 * 3.0));
    }
}

/// Puts ship `id` onto the next falling or resting hat; returns whether
/// there was one.
fn grab_hat(w: &mut World, id: PlayerId) -> bool {
    let Some(p) = props(w, ReplicatedKind::Hat).first().map(|h| h.pos) else {
        return false;
    };
    park(w, id, p);
    true
}

#[test]
fn touching_hats_wears_them_and_three_reveal_the_wearer() {
    let (mut w, ids) = world_with(base_setup(), 2);
    let (a, b) = (ids[0], ids[1]);
    let mut t = room();
    park(&mut w, b, Vec3::new(-38.0, -18.0, 0.0));
    park(&mut w, a, Vec3::new(-38.0, -18.0, 10.0));
    w.schedule_event(10, EventKind::HatRain);
    step_until(&mut w, &mut t, EventKind::HatRain, EventPhase::Active);
    for _ in 0..120 {
        w.step(&mut t, &[]);
    }
    for want in 1..=3u8 {
        let before = props(&w, ReplicatedKind::Hat).len();
        assert!(grab_hat(&mut w, a));
        w.step(&mut t, &[]);
        assert_eq!(w.ship(a).unwrap().hats, want);
        assert!(props(&w, ReplicatedKind::Hat).len() < before + 1);
        let v = w.mood_view();
        let revealed = director::reveals(&v, SEED, w.tick(), w.ship(a).unwrap());
        assert_eq!(revealed, want >= 3, "{want} hats");
        park(&mut w, a, Vec3::new(-38.0, -18.0, 10.0));
    }
    assert!(!director::reveals(
        &w.mood_view(),
        SEED,
        w.tick(),
        w.ship(b).unwrap()
    ));
    // A cloaked wearer of 3 hats is still shown (relevance).
    assert!(w.ship_visible_to(b, a));
    // After the event: the hats stay on the hull, the reveal ends.
    while w.mood_view().event.is_some() {
        w.step(&mut t, &[]);
    }
    assert_eq!(w.ship(a).unwrap().hats, 3);
    assert!(!director::reveals(
        &w.mood_view(),
        SEED,
        w.tick(),
        w.ship(a).unwrap()
    ));
    assert!(
        props(&w, ReplicatedKind::Hat).is_empty(),
        "leftover hats pop"
    );
}

#[test]
fn hats_scatter_on_death_and_the_hat_baron_is_crowned() {
    let (mut w, ids) = world_with(base_setup(), 3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let mut t = room();
    for id in &ids {
        park(&mut w, *id, Vec3::new(-38.0, -18.0, id.0 as f32 * 3.0));
    }
    w.schedule_event(10, EventKind::HatRain);
    step_until(&mut w, &mut t, EventKind::HatRain, EventPhase::Active);
    for _ in 0..200 {
        w.step(&mut t, &[]);
    }
    give(&mut w, &mut t, a, 4);
    give(&mut w, &mut t, b, 3);
    give(&mut w, &mut t, c, 1);
    assert_eq!(w.ship(a).unwrap().hats, 4);
    // A dies: its 4 hats scatter as pickups for 10 s.
    let at = w.ship(a).unwrap().pos;
    kill(&mut w, &mut t, a);
    let before: Vec<u32> = props(&w, ReplicatedKind::Hat)
        .iter()
        .map(|h| h.id)
        .collect();
    w.step(&mut t, &[]);
    assert_eq!(w.ship(a).unwrap().hats, 0);
    let scattered: Vec<crate::Entity> = props(&w, ReplicatedKind::Hat)
        .into_iter()
        .filter(|h| !before.contains(&h.id))
        .cloned()
        .collect();
    assert_eq!(scattered.len(), 4);
    for h in &scattered {
        assert_eq!(h.state[1], phase::SCATTERED);
        assert!(h.pos.distance(at) < 1.0);
        assert_eq!(h.expires_tick, h.spawn_tick + 600);
        assert!((h.vel.length() - 4.0).abs() < 1e-4);
    }
    for _ in 0..40 {
        w.step(&mut t, &[]);
    }
    for h in props(&w, ReplicatedKind::Hat) {
        if scattered.iter().any(|s| s.id == h.id) {
            assert_eq!(h.state[1], phase::SCATTERED_RESTING);
            assert_eq!(h.vel, Vec3::ZERO);
        }
    }
    // C ties B at 3 later: B reached 3 first and stays the Hat Baron.
    give(&mut w, &mut t, c, 2);
    assert_eq!(w.ship(c).unwrap().hats, 3);
    while w.mood_view().event.is_some() {
        w.step(&mut t, &[]);
    }
    let awards = w.results().awards;
    assert_eq!(
        awards,
        vec![Award {
            kind: EventKind::HatRain,
            player: b,
            value: 3,
        }]
    );
}

#[test]
fn the_hat_baron_rule() {
    let mut ships: Vec<crate::ShipState> = (0..3)
        .map(|i| {
            crate::ShipState::new(
                PlayerId(i),
                0,
                ShipType::Tinker,
                &crate::ShipTuning::tinker(),
                Vec3::ZERO,
                Quat::IDENTITY,
            )
        })
        .collect();
    assert_eq!(hat_baron(&ships, &[]), None);
    ships[0].hats = 2;
    ships[1].hats = 2;
    ships[2].hats = 1;
    // Ties go to the one who got there first, then the lower id.
    let marks = [(PlayerId(0), 50), (PlayerId(1), 40), (PlayerId(2), 1)];
    assert_eq!(hat_baron(&ships, &marks), Some((PlayerId(1), 2)));
    let marks = [(PlayerId(0), 40), (PlayerId(1), 40)];
    assert_eq!(hat_baron(&ships, &marks), Some((PlayerId(0), 2)));
    ships[2].hats = 5;
    assert_eq!(hat_baron(&ships, &marks), Some((PlayerId(2), 5)));
}

// ----- Cat paw --------------------------------------------------------------------

/// The 80 x 40 x 80 room with a group of three ships 8 u above the floor
/// near the +x wall and one ship far from them; spawn zones far away.
fn paw_world() -> (World, Vec<PlayerId>, BoxRoomTerrain) {
    let (mut w, ids) = world_with(base_setup(), 4);
    park(&mut w, ids[0], Vec3::new(30.0, -12.0, 0.0));
    park(&mut w, ids[1], Vec3::new(28.0, -12.0, 4.0));
    park(&mut w, ids[2], Vec3::new(31.0, -11.0, -5.0));
    park(&mut w, ids[3], Vec3::new(-25.0, 15.0, 0.0));
    (w, ids, room())
}

#[test]
fn the_cat_paw_needs_a_ship_near_a_wall() {
    // A huge room: nobody within 60 u of a face.
    let (mut w, ids) = world_with(base_setup(), 2);
    let mut t = BoxRoomTerrain::centered(Vec3::new(400.0, 200.0, 400.0));
    for id in &ids {
        park(&mut w, *id, Vec3::ZERO);
    }
    let d = crate::DirectorState::default();
    let c = w.director_candidates_for_test(&mut t, &d, None);
    assert!(!c.iter().any(|(k, _)| *k == EventKind::CatPaw));
    park(&mut w, ids[0], Vec3::new(150.0, 0.0, 0.0));
    let c = w.director_candidates_for_test(&mut t, &d, None);
    assert!(c.iter().any(|(k, _)| *k == EventKind::CatPaw));
}

#[test]
fn the_cat_paw_swipes_the_densest_group() {
    let (mut w, ids, mut t) = paw_world();
    // Debris in the group.
    w.hook_for_test(&mut t, |ctx| {
        ctx.spawn_debris(Vec3::new(29.0, -12.0, 1.0), Vec3::ZERO, 1.0, 2.0)
    });
    w.schedule_event(10, EventKind::CatPaw);
    let hulls: Vec<f32> = ids.iter().map(|id| w.ship(*id).unwrap().hull).collect();
    let mut evs = step_until(&mut w, &mut t, EventKind::CatPaw, EventPhase::Announce);
    w.step(&mut t, &[]);
    // The tell: the paw waits outside the floor (face -y: code 2).
    let v = w.mood_view();
    assert_eq!(v.params[0], 2, "the floor is the nearest face");
    let paw = props(&w, ReplicatedKind::CatPaw);
    assert_eq!(paw.len(), 1);
    assert_eq!(paw[0].state[0], 0, "tell phase");
    assert!(paw[0].pos.y < -20.0, "outside the wall: {:?}", paw[0].pos);
    assert_eq!(paw[0].radius, 10.0);
    // Ships hold still through the swipe (the paw knocks them about).
    let mut phases = Vec::new();
    let mut edit_ticks = std::collections::BTreeSet::new();
    let mut edits = Vec::new();
    while w.mood_view().event.is_some() {
        let e = w.step(&mut t, &[]);
        let now = w.tick();
        for x in &e {
            if let Event::TerrainEdit(TerrainEdit::SubtractSphere { center, radius }) = x {
                edit_ticks.insert(now);
                edits.push((*center, *radius));
            }
        }
        if let Some(p) = props(&w, ReplicatedKind::CatPaw).first() {
            if phases.last() != Some(&p.state[0]) {
                phases.push(p.state[0]);
            }
        }
        evs.extend(e);
    }
    assert_eq!(phases, vec![0, 1, 2, 3], "tell, reach, sweep, withdraw");
    // The group took 30 damage once each with knockback; the far ship none.
    let hits: Vec<_> = evs
        .iter()
        .filter_map(|e| match e {
            Event::Damaged {
                target,
                attacker,
                amount,
                ..
            } => Some((*target, *attacker, *amount)),
            _ => None,
        })
        .collect();
    for (k, id) in ids.iter().enumerate() {
        let n = hits.iter().filter(|h| h.0 == *id).count();
        if k < 3 {
            assert_eq!(n, 1, "ship {k} hit once");
            assert!((w.ship(*id).unwrap().hull - (hulls[k] - 30.0)).abs() < 1e-3);
        } else {
            assert_eq!(n, 0, "the far ship is untouched");
        }
    }
    assert!(hits.iter().all(|h| h.1.is_none() && h.2 == 30.0));
    // Knocked about at about 25 u/s (the dampers slow them since).
    assert!(ids[..3].iter().any(|id| {
        let s = w.ship(*id).unwrap();
        s.pos.distance(Vec3::new(30.0, -12.0, 0.0)) > 2.0
    }));
    // Debris swept along.
    let debris = w
        .entities()
        .iter()
        .find(|e| e.kind == EntityKind::Debris)
        .unwrap();
    assert!(
        debris.pos.distance(Vec3::new(29.0, -12.0, 1.0)) > 3.0,
        "swept"
    );
    // Claw grooves: at most 36 spheres of 3 u, spread over the sweep.
    assert!(!edits.is_empty(), "the claws scraped the wall");
    assert!(edits.len() <= 36, "{} edits", edits.len());
    assert!(edits.iter().all(|(_, r)| (*r - 3.0).abs() < 1e-5));
    assert!(edit_ticks.len() * 3 >= edits.len(), "at most 3 a tick");
    assert!(edit_ticks.len() >= 4, "spread over ticks: {edit_ticks:?}");
    for (c, _) in &edits {
        // Grooves in the floor under the group (the claws rake the wall the
        // paw came through), every sphere `groove_inset` (0.6 u) inside it
        // (D5: even grooves).
        assert!((c.y + 20.6).abs() < 0.3, "{c:?}");
    }
    // Three separate grooves (D5): the sweep runs along z here and the claws
    // spread along x, in lanes `claw_gap` apart, more than a groove's width,
    // so a ridge of rock stays between them.
    let mut lanes: Vec<f32> = edits.iter().map(|(c, _)| c.x).collect();
    lanes.sort_by(|x, y| x.total_cmp(y));
    let mut centres = vec![vec![lanes[0]]];
    for x in &lanes[1..] {
        if x - centres.last().unwrap().last().unwrap() > 2.0 {
            centres.push(vec![*x]);
        } else {
            centres.last_mut().unwrap().push(*x);
        }
    }
    let mid: Vec<f32> = centres
        .iter()
        .map(|l| l.iter().sum::<f32>() / l.len() as f32)
        .collect();
    assert_eq!(mid.len(), 3, "three grooves: {lanes:?}");
    for pair in mid.windows(2) {
        assert!(pair[1] - pair[0] > 6.5, "grooves apart: {mid:?}");
    }
    assert!(props(&w, ReplicatedKind::CatPaw).is_empty(), "the paw left");
}

/// D5: rock rising above the face the paw comes through (here a 4 u slab
/// on the floor, the top at y = -16) is what the claws rake. Before, they
/// raked the face itself, inside the slab, and carved nothing; on a level
/// the face is the sealing wall, which no crater can open.
#[test]
fn the_cat_paw_rakes_the_rock_in_front_of_the_face() {
    let (mut w, _ids, mut t) = paw_world();
    // A nearly flat slab: the top of a 1000 u sphere at y = -16.
    t.apply(TerrainEdit::AddSphere {
        center: Vec3::new(30.0, -1016.0, 0.0),
        radius: 1000.0,
    });
    w.schedule_event(10, EventKind::CatPaw);
    let mut edits = Vec::new();
    for _ in 0..(10 * 60) {
        for e in w.step(&mut t, &[]) {
            if let Event::TerrainEdit(TerrainEdit::SubtractSphere { center, .. }) = e {
                edits.push(center);
            }
        }
    }
    assert!(edits.len() >= 12, "grooves in the slab: {edits:?}");
    for c in &edits {
        // Just under the slab's top there: `groove_inset` (0.6 u), a little
        // more where the groove before it already opened the rock.
        let (dx, dz) = (c.x - 30.0, c.z);
        let top = -1016.0 + (1000.0f32 * 1000.0 - dx * dx - dz * dz).sqrt();
        assert!(c.y < top - 0.3 && c.y > top - 1.5, "{c:?} under {top}");
    }
    // The grooves are open now: their bottoms lie below the slab's top.
    assert!(edits
        .iter()
        .all(|c| t.sdf(*c + Vec3::new(0.0, 1.0, 0.0)) > 0.5));
}

#[test]
fn the_cat_paw_kills_without_credit_and_spares_spawn_zones() {
    let (mut w, ids, mut t) = paw_world();
    w.ship_mut(ids[0]).unwrap().hull = 10.0;
    w.schedule_event(10, EventKind::CatPaw);
    let mut evs = Vec::new();
    for _ in 0..(10 * 60) {
        evs.extend(w.step(&mut t, &[]));
    }
    let killed = evs.iter().find_map(|e| match e {
        Event::Killed { victim, killer, .. } if *victim == ids[0] => Some(*killer),
        _ => None,
    });
    assert_eq!(killed, Some(None), "killed by the level");

    // A spawn point right at the group: no grooves near it.
    let spawn_at = Vec3::new(30.0, -19.0, 0.0);
    let mut grooves = Vec::new();
    for with_spawn in [false, true] {
        let mut setup = base_setup();
        if with_spawn {
            setup.spawns.push(spawn(spawn_at, 0));
        }
        let (mut w, ids) = world_with(setup, 3);
        for id in &ids {
            park(&mut w, *id, Vec3::new(30.0, -12.0, id.0 as f32));
        }
        let mut t = room();
        w.schedule_event(10, EventKind::CatPaw);
        let mut edits = Vec::new();
        for _ in 0..(10 * 60) {
            for e in w.step(&mut t, &[]) {
                if let Event::TerrainEdit(TerrainEdit::SubtractSphere { center, .. }) = e {
                    edits.push(center);
                }
            }
        }
        grooves.push(edits);
    }
    assert!(grooves[0].iter().any(|c| c.distance(spawn_at) < 26.5));
    assert!(!grooves[1].is_empty());
    assert!(
        grooves[1].iter().all(|c| c.distance(spawn_at) >= 26.5),
        "no grooves in a spawn zone"
    );
}

// ----- Disco inferno ----------------------------------------------------------------

fn wander(tick: u64, k: u16) -> InputCmd {
    let mut c = InputCmd::idle(tick);
    c.thrust = match (tick / 40 + k as u64) % 3 {
        0 => Vec3::new(0.0, 0.0, -1.0),
        1 => Vec3::new(0.5, 0.3, 0.0),
        _ => Vec3::new(-0.4, -0.2, 0.6),
    };
    c.look_yaw = 0.01;
    c
}

#[test]
fn disco_inferno_hangs_a_ball_over_the_ships_and_changes_nothing_else() {
    let (mut plain, ids) = world_with(base_setup(), 3);
    let (mut disco, _) = world_with(base_setup(), 3);
    let (mut t1, mut t2) = (room(), room());
    disco.schedule_event(30, EventKind::DiscoInferno);
    let mut was_active = false;
    let mut first_drop = None;
    for _ in 0..(30 * 60) {
        let tick = plain.tick() + 1;
        let input: Vec<_> = ids.iter().map(|id| (*id, wander(tick, id.0))).collect();
        plain.step(&mut t1, &input);
        disco.step(&mut t2, &input);
        // Ships fly exactly as without the event.
        for id in &ids {
            assert_eq!(plain.ship(*id), disco.ship(*id));
        }
        let v = disco.mood_view();
        let balls = props(&disco, ReplicatedKind::DiscoBall);
        if v.event == Some(EventKind::DiscoInferno) && v.phase == EventPhase::Active {
            was_active = true;
            if disco.tick() > v.active_tick + 1 {
                assert_eq!(balls.len(), 1);
                let ball = balls[0];
                assert_eq!(ball.state[0], 8, "8 % light");
                let c = ids
                    .iter()
                    .map(|id| disco.ship(*id).unwrap().pos)
                    .fold(Vec3::ZERO, |a, p| a + p)
                    / 3.0;
                let above = ball.pos.y - c.y;
                if disco.tick() == v.active_tick + 2 {
                    first_drop = Some(above);
                }
                // Above the centroid (within its follow lag).
                let flat = Vec3::new(ball.pos.x - c.x, 0.0, ball.pos.z - c.z).length();
                assert!(above > 1.9, "above the ships: {above}");
                if disco.tick() > v.active_tick + 4 * 60 {
                    assert!(above < 12.5, "came down: {above}");
                    assert!(flat < 20.0, "over the ships: {flat}");
                }
            }
        } else {
            assert!(balls.is_empty());
        }
    }
    assert!(was_active);
    assert!(first_drop.unwrap() > 14.0, "descends from above");
}

// ----- Tired announcer ----------------------------------------------------------------

fn kill_cams(evs: &[Event]) -> Vec<(Option<PlayerId>, PlayerId, u16)> {
    evs.iter()
        .filter_map(|e| match e {
            Event::Cue(c) if c.kind == CueKind::KillCam => {
                Some((c.source, c.target.unwrap(), c.value))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn tired_announcer_plays_every_kill_back_to_back() {
    let (mut w, ids) = world_with(base_setup(), 6);
    let mut t = room();
    w.schedule_event(10, EventKind::TiredAnnouncer);
    // A kill during the announcement gets no kill-cam.
    let mut evs = Vec::new();
    for _ in 0..12 {
        evs.extend(w.step(&mut t, &[]));
    }
    assert_eq!(w.mood_view().phase, EventPhase::Announce);
    kill(&mut w, &mut t, ids[5]);
    evs.extend(w.step(&mut t, &[]));
    assert!(kill_cams(&evs).is_empty());
    step_until(
        &mut w,
        &mut t,
        EventKind::TiredAnnouncer,
        EventPhase::Active,
    );
    w.step(&mut t, &[]);
    // Four kills at once, the first credited.
    w.hook_for_test(&mut t, |ctx| {
        let src = DamageSource {
            attacker: Some(ids[0]),
            team: 0,
            weapon: None,
        };
        ctx.damage(ids[1], 10_000.0, Vec3::ZERO, src, true);
    });
    for id in &ids[2..5] {
        kill(&mut w, &mut t, *id);
    }
    let pos = w.ship(ids[1]).unwrap().pos;
    let evs = w.step(&mut t, &[]);
    let cams = kill_cams(&evs);
    assert_eq!(cams.len(), 4);
    assert_eq!(cams[0], (Some(ids[0]), ids[1], 0));
    assert_eq!(cams[1].2, 180, "after the first");
    assert_eq!(cams[2].2, 360, "after the second");
    assert_eq!(cams[3].2, NARRATED_ONLY, "the queue is full");
    let c0 = evs
        .iter()
        .find_map(|e| match e {
            Event::Cue(c) if c.kind == CueKind::KillCam => Some(*c),
            _ => None,
        })
        .unwrap();
    assert_eq!(c0.pos, pos);
    // Ten seconds later the queue is free again.
    for _ in 0..600 {
        w.step(&mut t, &[]);
    }
    kill(&mut w, &mut t, ids[0]);
    let cams = kill_cams(&w.step(&mut t, &[]));
    assert_eq!(cams, vec![(None, ids[0], 0)]);
    // After the event: no kill-cams.
    while w.mood_view().event.is_some() {
        w.step(&mut t, &[]);
    }
    let alive = ids
        .iter()
        .copied()
        .find(|id| w.ship(*id).unwrap().alive)
        .unwrap();
    kill(&mut w, &mut t, alive);
    assert!(kill_cams(&w.step(&mut t, &[])).is_empty());
}
