//! Scenario tests of the world and movement events (M4 wave B package B1):
//! Bored level, Level hiccups, Gravity coffee break, Wrong genre, Big head
//! day, Rubber chickens, Moving day, Everything is cake. One test per rule
//! of `docs/milestone-4-plan.md` §3.2.

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
use ls_math::{det, Quat, Tick, Vec3};

use crate::director::{event_effects, move_effects, MoodView};
use crate::events::level_hiccups::{hiccup_ticks, lift_ticks};
use crate::events::moving_day::cycle;
use crate::events::wrong_genre::WrongGenre;
use crate::events::{EventKind, EventPhase};
use crate::mode::ModeKind;
use crate::world::proj_flags;
use crate::{
    Buttons, EntitySpawn, Event, InputCmd, MatchSetup, PlayerId, RuleSet, ShipType, SpawnPoint,
    World, WorldConfig,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const SEED: u64 = 0xb1b1;

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
            spawn(Vec3::new(-10.0, 0.0, 0.0), 0),
            spawn(Vec3::new(10.0, 0.0, 0.0), 1),
        ],
        pickup_points: (0..8)
            .map(|i| Vec3::new(-21.0 + 6.0 * i as f32, 8.0, 12.0))
            .collect(),
        heist: None,
        ..Default::default()
    }
}

fn brawl() -> RuleSet {
    let mut r = RuleSet::default_for(ModeKind::TeamBrawl);
    r.mode.set_field("match_secs", 600.0);
    r.mutators.clear();
    r
}

fn sandbox() -> RuleSet {
    let (f, text) = crate::ruleset::BUILTIN_RULESETS
        .iter()
        .find(|(f, _)| f.ends_with("sandbox_all_weapons.ron"))
        .unwrap();
    let mut r = RuleSet::from_ron(f, text).unwrap();
    r.mutators.clear();
    r
}

/// Whether a pickup point holds a pickup (not scrap).
fn holds_pickup(p: &crate::world::PickupSpawn) -> bool {
    p.content
        .is_some_and(|(_, k)| !matches!(k, crate::PickupKind::Scrap { .. }))
}

fn world(setup: MatchSetup, rules: RuleSet, ships: usize) -> World {
    let mut cfg = WorldConfig::paint_the_void(SEED, setup);
    cfg.rules = rules;
    let mut w = World::new(cfg).unwrap();
    for i in 0..ships {
        w.add_player(Some((i % 2) as u8), ShipType::Tinker).unwrap();
    }
    w
}

fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(120.0, 60.0, 120.0))
}

/// Forces `kind` on the next tick and steps until its active phase began
/// (the director's step at the end of the active tick ran). Returns the
/// active phase's start tick.
fn activate(w: &mut World, t: &mut BoxRoomTerrain, kind: EventKind) -> u64 {
    w.schedule_event(w.tick() + 1, kind);
    for _ in 0..400 {
        w.step(t, &[]);
        let v = w.mood_view();
        if v.event == Some(kind) && v.phase == EventPhase::Active {
            return v.active_tick;
        }
    }
    panic!("{kind:?} never started");
}

fn idle(w: &World, ids: &[PlayerId], buttons: Buttons) -> Vec<(PlayerId, InputCmd)> {
    ids.iter()
        .map(|&p| {
            let mut c = InputCmd::idle(w.tick() + 1);
            c.buttons = buttons;
            (p, c)
        })
        .collect()
}

fn angle(a: Vec3, b: Vec3) -> f32 {
    det::acos(a.dot(b).clamp(-1.0, 1.0)).to_degrees()
}

// ----- Bored level ----------------------------------------------------------

#[test]
fn bored_level_turns_up_ninety_degrees_over_five_seconds_and_keeps_it() {
    let mut w = world(base_setup(), brawl(), 2);
    let mut t = room();
    let ids = [PlayerId(0), PlayerId(1)];
    let start: Vec<Vec3> = ids.iter().map(|p| w.ship(*p).unwrap().pos).collect();
    assert_eq!(w.up(), Vec3::Y);
    let a = activate(&mut w, &mut t, EventKind::BoredLevel);
    let (from, axis) = super::bored_level::BoredLevel::frame(&w.mood_view().params);
    assert_eq!(from, Vec3::Y);
    // A horizontal axis (perpendicular to the old up).
    assert!(axis.dot(Vec3::Y).abs() < 0.02, "{axis:?}");
    let turn = Tick::ticks_for_secs(5.0);
    let mut seen = Vec::new();
    while w.mood_view().event.is_some() {
        let inp = idle(&w, &ids, Buttons::HORIZON_ASSIST);
        w.step(&mut t, &inp);
        seen.push((w.tick() - a, w.up()));
    }
    let at = |k: u64| seen.iter().find(|(d, _)| *d == k).unwrap().1;
    // Eased: half way at 2.5 s, done at 5 s, unit length all the way.
    assert!((angle(Vec3::Y, at(turn / 2)) - 45.0).abs() < 1.0);
    assert!((angle(Vec3::Y, at(turn)) - 90.0).abs() < 1.0);
    assert!(angle(Vec3::Y, at(10)) < 0.5, "slow start");
    for (_, u) in &seen {
        assert!((u.length() - 1.0).abs() < 1e-5);
    }
    // Stays turned after the event; terrain and idle ships keep their
    // coordinates (only the roll turns).
    let end_up = w.up();
    assert!((angle(Vec3::Y, end_up) - 90.0).abs() < 1.0);
    for _ in 0..120 {
        let inp = idle(&w, &ids, Buttons::HORIZON_ASSIST);
        w.step(&mut t, &inp);
    }
    assert_eq!(w.up(), end_up);
    for (p, s) in ids.iter().zip(&start) {
        let ship = w.ship(*p).unwrap();
        assert!(ship.pos.distance(*s) < 1e-3, "{:?}", ship.pos);
        // Horizon assist levelled the roll to the new up (where the nose
        // does not point along it).
        let f = ship.orientation.forward();
        if f.dot(end_up).abs() < 0.9 {
            assert!(ship.orientation.right().dot(end_up).abs() < 0.05);
        }
    }
}

#[test]
fn bored_level_axis_is_seeded_and_the_up_is_a_pure_function_of_the_view() {
    let axis_for = |seed: u64| {
        let mut cfg = WorldConfig::paint_the_void(seed, base_setup());
        cfg.rules = brawl();
        let mut w = World::new(cfg).unwrap();
        let mut t = room();
        activate(&mut w, &mut t, EventKind::BoredLevel);
        super::bored_level::BoredLevel::frame(&w.mood_view().params).1
    };
    let axes: Vec<Vec3> = (0..6).map(|s| axis_for(s * 977)).collect();
    assert_eq!(axis_for(0), axes[0]);
    let distinct = axes
        .iter()
        .enumerate()
        .filter(|(i, a)| axes[..*i].iter().all(|b| b.distance(**a) > 1e-3))
        .count();
    assert!(distinct >= 5, "{axes:?}");
    // The view alone gives the world's up on every tick (the predictor).
    let mut w = world(base_setup(), brawl(), 1);
    let mut t = room();
    activate(&mut w, &mut t, EventKind::BoredLevel);
    for _ in 0..200 {
        let view = w.mood_view();
        let fx = event_effects(&view, SEED, w.tick() + 1);
        w.step(&mut t, &[]);
        assert_eq!(fx.up, Some(w.up()));
    }
}

// ----- Level hiccups ----------------------------------------------------------

#[test]
fn hiccups_come_every_three_seconds_give_or_take_one() {
    let def = EventKind::LevelHiccups.def();
    for seed in 0..20u64 {
        let ticks = hiccup_ticks(seed, 1000, def);
        assert_eq!(ticks, hiccup_ticks(seed, 1000, def));
        assert!((3..=6).contains(&ticks.len()), "{ticks:?}");
        let mut prev = 1000;
        for h in &ticks {
            let gap = h - prev;
            assert!((120..=240).contains(&gap), "gap {gap}");
            prev = *h;
        }
        assert!(prev + lift_ticks(def) <= 1000 + def.active_ticks());
    }
    assert_ne!(hiccup_ticks(1, 1000, def), hiccup_ticks(2, 1000, def));
}

#[test]
fn a_hiccup_lifts_ships_debris_and_projectiles_about_two_units() {
    let mut w = world(base_setup(), brawl(), 2);
    let mut t = room();
    let debris = w.spawn_entity(EntitySpawn::debris(
        Vec3::new(0.0, -5.0, -10.0),
        Vec3::ZERO,
        0.6,
        2.0,
    ));
    let a = activate(&mut w, &mut t, EventKind::LevelHiccups);
    let h = hiccup_ticks(SEED, a, EventKind::LevelHiccups.def())[0];
    let lift = lift_ticks(EventKind::LevelHiccups.def());
    // Ship 0 fires a bolt along +x just before the hiccup.
    let p0 = PlayerId(0);
    w.ship_mut(p0).unwrap().orientation = WrongGenre::side_facing(1.0);
    while w.tick() + 3 < h {
        w.step(&mut t, &[]);
    }
    let mut fire = idle(&w, &[p0], Buttons::FIRE_PRIMARY);
    fire[0].1.tick = w.tick() + 1;
    w.step(&mut t, &fire);
    let bolt = w.projectiles().last().expect("a bolt").id;
    let bolt_y = w.projectiles().iter().find(|p| p.id == bolt).unwrap().pos.y;
    while w.tick() + 1 < h {
        w.step(&mut t, &[]);
    }
    let ship_y = w.ship(PlayerId(1)).unwrap().pos.y;
    let debris_y = w.entities().iter().find(|e| e.id == debris).unwrap().pos.y;
    w.step(&mut t, &[]); // the hiccup tick
    assert_eq!(w.tick(), h);
    let y = w
        .projectiles()
        .iter()
        .find(|p| p.id == bolt)
        .map(|p| p.pos.y);
    assert!(
        (y.unwrap() - bolt_y - 2.0).abs() < 1e-4,
        "bolt nudged by 2 u"
    );
    for _ in 0..lift + 30 {
        w.step(&mut t, &[]);
    }
    let ship = w.ship(PlayerId(1)).unwrap();
    let rise = ship.pos.y - ship_y;
    assert!((1.7..=2.3).contains(&rise), "ship rose {rise}");
    assert!(ship.vel.length() < 0.6, "the lift stops: {:?}", ship.vel);
    let e = w.entities().iter().find(|e| e.id == debris).unwrap();
    let rise = e.pos.y - debris_y;
    assert!((1.5..=2.6).contains(&rise), "debris rose {rise}");
}

#[test]
fn a_hiccup_under_a_ceiling_collides_normally() {
    let mut setup = base_setup();
    // Ship 1 sits 1 u below the room's ceiling (y = 6).
    setup.spawns[1].pos = Vec3::new(10.0, 4.5, 0.0);
    let mut w = world(setup, brawl(), 2);
    let mut t = BoxRoomTerrain::centered(Vec3::new(120.0, 12.0, 120.0));
    let a = activate(&mut w, &mut t, EventKind::LevelHiccups);
    let h = hiccup_ticks(SEED, a, EventKind::LevelHiccups.def())[0];
    let mut top = f32::MIN;
    while w.tick() < h + 60 {
        w.step(&mut t, &[]);
        top = top.max(w.ship(PlayerId(1)).unwrap().pos.y);
    }
    assert!(top > 4.6, "lifted: {top}");
    assert!(top <= 6.0 - 0.5 + 1e-3, "stays inside: {top}");
}

#[test]
fn hiccups_move_ships_only_on_their_ticks() {
    let def = EventKind::LevelHiccups.def();
    let mut w = world(base_setup(), brawl(), 1);
    let mut t = room();
    let a = activate(&mut w, &mut t, EventKind::LevelHiccups);
    let view = w.mood_view();
    let ship = w.ship(PlayerId(0)).unwrap().clone();
    let ticks = hiccup_ticks(SEED, a, def);
    let mut kicks = 0;
    for now in a..a + def.active_ticks() + 1 {
        let fx = move_effects(&view, SEED, now, &ship);
        if fx.impulse != Vec3::ZERO {
            kicks += 1;
            assert!(ticks.contains(&now), "{now}");
            assert!((fx.impulse.normalize() - Vec3::Y).length() < 1e-6);
        }
    }
    // Lift only (the idle test ship never moves up, so no stops).
    assert_eq!(kicks, ticks.len());
}

// ----- Gravity coffee break ---------------------------------------------------

fn well_setup() -> MatchSetup {
    let mut s = base_setup();
    s.gravity_chaos = 100;
    s.wells = vec![crate::gravity::tests::site(
        Vec3::new(0.0, 0.0, -20.0),
        1.0,
        30.0,
    )];
    s.spawns[0].pos = Vec3::new(0.0, 0.0, -8.0);
    s
}

#[test]
fn coffee_break_needs_wells() {
    let mut t = room();
    for (wells, chaos, eligible) in [(true, 100, true), (true, 0, false), (false, 100, false)] {
        let mut s = well_setup();
        s.gravity_chaos = chaos;
        if !wells {
            s.wells.clear();
        }
        s.absurdity = 100;
        let mut r = brawl();
        r.absurdity = 100;
        let mut w = world(s, r, 2);
        w.step(&mut t, &[]);
        let d = w.director().clone();
        let c = w.director_candidates_for_test(&mut t, &d, None);
        assert_eq!(
            c.iter().any(|(k, _)| *k == EventKind::GravityCoffeeBreak),
            eligible,
            "wells {wells} chaos {chaos}"
        );
    }
}

#[test]
fn coffee_break_fades_the_wells_out_and_back_in() {
    let mut w = world(well_setup(), brawl(), 2);
    let mut t = room();
    let p = PlayerId(0);
    // The well pulls an idle ship before the break.
    let z0 = w.ship(p).unwrap().pos.z;
    for _ in 0..60 {
        w.step(&mut t, &[]);
    }
    assert!(w.ship(p).unwrap().pos.z < z0 - 0.05, "pulled before");
    let a = activate(&mut w, &mut t, EventKind::GravityCoffeeBreak);
    let mut scales = Vec::new();
    while w.mood_view().event.is_some() {
        w.step(&mut t, &[]);
        scales.push((w.tick() - a, w.event_effects().well_scale));
    }
    let at = |k: u64| scales.iter().find(|(d, _)| *d == k).unwrap().1;
    assert_eq!(at(1), 1.0 - 1.0 / 60.0);
    assert!((at(30) - 0.5).abs() < 1e-6);
    assert_eq!(at(60), 0.0);
    assert_eq!(at(1200), 0.0);
    // The 1 s ending phase brings the pull back.
    assert!((at(1230) - 0.5).abs() < 0.02, "{}", at(1230));
    assert!(scales.windows(2).skip(1200).all(|p| p[1].1 >= p[0].1));
    // During the break a resting ship near the well stays put.
    let mut w = world(well_setup(), brawl(), 2);
    let a = activate(&mut w, &mut t, EventKind::GravityCoffeeBreak);
    while w.tick() < a + 300 {
        w.step(&mut t, &[]);
    }
    let pos = w.ship(p).unwrap().pos;
    for _ in 0..300 {
        w.step(&mut t, &[]);
    }
    assert!(w.ship(p).unwrap().pos.distance(pos) < 1e-3);
    // After it the well pulls again.
    while w.mood_view().event.is_some() {
        w.step(&mut t, &[]);
    }
    let pos = w.ship(p).unwrap().pos;
    for _ in 0..60 {
        w.step(&mut t, &[]);
    }
    assert!(w.ship(p).unwrap().pos.z < pos.z - 0.05, "pulled after");
}

#[test]
fn coffee_break_frees_debris_and_ordnance_too() {
    let mut w = world(well_setup(), brawl(), 2);
    let mut t = room();
    let a = activate(&mut w, &mut t, EventKind::GravityCoffeeBreak);
    while w.tick() < a + 70 {
        w.step(&mut t, &[]);
    }
    let field = w.well_field().clone();
    let p = Vec3::new(4.0, 0.0, -14.0);
    assert!(field.body_accel(p, w.tick()).length() > 0.5);
    let scale = w.event_effects().well_scale;
    assert_eq!(scale, 0.0);
    assert_eq!(field.body_accel_scaled(p, w.tick(), scale), Vec3::ZERO);
    let d = w.spawn_entity(EntitySpawn::debris(p, Vec3::new(0.0, 0.0, 0.3), 0.5, 1.0));
    for _ in 0..30 {
        w.step(&mut t, &[]);
    }
    let e = w.entities().iter().find(|e| e.id == d).unwrap();
    assert!(
        e.vel.x.abs() < 1e-6 && e.vel.z <= 0.3,
        "not pulled: {:?}",
        e.vel
    );
}

// ----- Wrong genre ------------------------------------------------------------

#[test]
fn wrong_genre_locks_ships_to_their_z_plane_and_snaps_them_sideways() {
    let mut w = world(base_setup(), brawl(), 2);
    let mut t = room();
    let ids = [PlayerId(0), PlayerId(1)];
    // Ship 1 looks mostly along -x, slightly along z.
    w.ship_mut(ids[1]).unwrap().orientation = Quat::from_axis_angle(Vec3::Y, 1.2);
    let a = activate(&mut w, &mut t, EventKind::WrongGenre);
    let z: Vec<f32> = ids.iter().map(|p| w.ship(*p).unwrap().pos.z).collect();
    let snap = Tick::ticks_for_secs(0.3);
    // Thrust and look in every direction: z stays, facing ends at +-x.
    for k in 1..=500u64 {
        let inp: Vec<_> = ids
            .iter()
            .map(|&p| {
                let mut c = InputCmd::idle(w.tick() + 1);
                c.thrust = Vec3::new(0.7, -0.3, if k % 120 < 60 { 1.0 } else { -1.0 });
                c.look_yaw = 0.02;
                c.look_pitch = if k < 200 { 0.0 } else { 0.01 };
                c.roll = 0.5;
                c.buttons = Buttons::BOOST | Buttons::HORIZON_ASSIST;
                (p, c)
            })
            .collect();
        w.step(&mut t, &inp);
        for (i, p) in ids.iter().enumerate() {
            let s = w.ship(*p).unwrap();
            assert_eq!(s.pos.z, z[i], "tick {k}");
            assert_eq!(s.vel.z, 0.0);
            if k == snap {
                let f = s.orientation.forward();
                let want = if i == 0 { 1.0 } else { -1.0 };
                // Ship 0 looked along -z (identity): it snaps to +x.
                assert!(f.dot(Vec3::X * want) > 0.9999, "ship {i}: {f:?}");
                assert!(s.orientation.up().dot(Vec3::Y) > 0.9999);
            }
            if k > snap {
                // In the plane: forward has no z, right is along z.
                let o = s.orientation;
                assert!(o.forward().z.abs() < 1e-4, "{:?}", o.forward());
                assert!(o.right().z.abs() > 0.9999);
            }
        }
    }
    let _ = a;
    // Pitch turned ship 0 inside the plane (it can turn around by looping).
    let f = w.ship(ids[0]).unwrap().orientation.forward();
    assert!(f.x < 0.9, "{f:?}");
    // After the event: free flight again.
    while w.mood_view().event.is_some() {
        w.step(&mut t, &[]);
    }
    let mut c = InputCmd::idle(w.tick() + 1);
    c.thrust = Vec3::new(0.0, 0.0, -1.0);
    c.look_yaw = 0.05;
    for _ in 0..30 {
        c.tick = w.tick() + 1;
        w.step(&mut t, &[(ids[0], c)]);
    }
    assert!(w.ship(ids[0]).unwrap().pos.z != z[0]);
}

#[test]
fn wrong_genre_projectiles_fly_in_the_plane_and_hits_ignore_depth() {
    let run = |event: bool| {
        let mut setup = base_setup();
        // The target sits 5 u deeper on the shooter's line.
        setup.spawns[0].pos = Vec3::new(-10.0, 0.0, 0.0);
        setup.spawns[1].pos = Vec3::new(2.0, 0.0, 5.0);
        let mut w = world(setup, brawl(), 2);
        let mut t = room();
        if event {
            activate(&mut w, &mut t, EventKind::WrongGenre);
            // Let the snap finish (ship 0 faces +x).
            for _ in 0..30 {
                w.step(&mut t, &[]);
            }
        }
        let p0 = PlayerId(0);
        w.ship_mut(p0).unwrap().orientation = WrongGenre::side_facing(1.0);
        let mut hits = 0;
        let mut planar = 0;
        for _ in 0..40 {
            let mut c = InputCmd::idle(w.tick() + 1);
            c.buttons = Buttons::FIRE_PRIMARY;
            for e in w.step(&mut t, &[(p0, c)]) {
                if let Event::Damaged {
                    target,
                    attacker: Some(a),
                    ..
                } = e
                {
                    if target == PlayerId(1) && a == p0 {
                        hits += 1;
                    }
                }
            }
            for p in w.projectiles() {
                if p.flags & proj_flags::PLANAR != 0 {
                    planar += 1;
                    assert_eq!(p.vel.z, 0.0);
                    assert!(p.pos.z.abs() < 1e-5, "{:?}", p.pos);
                }
            }
        }
        (hits, planar)
    };
    let (hits, planar) = run(true);
    assert!(hits > 0 && planar > 0, "hits {hits}, planar {planar}");
    assert_eq!(run(false), (0, 0), "depth counts outside the event");
}

#[test]
fn wrong_genre_old_projectiles_keep_their_depth() {
    let mut w = world(base_setup(), brawl(), 2);
    let mut t = room();
    let p0 = PlayerId(0);
    w.schedule_event(w.tick() + 1, EventKind::WrongGenre);
    w.step(&mut t, &[]);
    let active = w.mood_view().phase_end_tick;
    while w.tick() + 3 < active {
        w.step(&mut t, &[]);
    }
    // A bolt fired in the announce phase, along -z and up.
    w.ship_mut(p0).unwrap().orientation = Quat::from_axis_angle(Vec3::X, 0.3);
    let mut c = InputCmd::idle(w.tick() + 1);
    c.buttons = Buttons::FIRE_PRIMARY;
    w.step(&mut t, &[(p0, c)]);
    let id = w.projectiles()[0].id;
    for _ in 0..8 {
        w.step(&mut t, &[]);
    }
    assert_eq!(w.mood_view().phase, EventPhase::Active);
    let p = w
        .projectiles()
        .iter()
        .find(|p| p.id == id)
        .expect("in flight");
    assert_eq!(p.flags & proj_flags::PLANAR, 0);
    assert!(p.vel.z != 0.0);
}

// ----- Big head day -----------------------------------------------------------

#[test]
fn big_heads_triple_the_hit_sphere_against_projectiles_only() {
    let run = |event: bool| {
        let mut setup = base_setup();
        setup.spawns[0].pos = Vec3::new(-10.0, 0.0, 0.0);
        // 1.2 u off the bolt's line: outside 0.5 u, inside 1.5 u.
        setup.spawns[1].pos = Vec3::new(2.0, 1.2, 0.0);
        let mut w = world(setup, brawl(), 2);
        let mut t = room();
        if event {
            activate(&mut w, &mut t, EventKind::BigHeadDay);
            assert_eq!(w.next_event_effects().hit_radius_scale, 3.0);
        }
        let p0 = PlayerId(0);
        w.ship_mut(p0).unwrap().orientation = WrongGenre::side_facing(1.0);
        let mut hits = 0;
        for _ in 0..40 {
            let mut c = InputCmd::idle(w.tick() + 1);
            c.buttons = Buttons::FIRE_PRIMARY;
            for e in w.step(&mut t, &[(p0, c)]) {
                if matches!(e, Event::Damaged { target, .. } if target == PlayerId(1)) {
                    hits += 1;
                }
            }
        }
        // Terrain collision and pickups use the plain radius.
        assert_eq!(
            w.tuning_for(ShipType::Tinker).radius,
            crate::ShipTuning::tinker().radius
        );
        hits
    };
    assert!(run(true) > 0);
    assert_eq!(run(false), 0);
}

// ----- Rubber chickens --------------------------------------------------------

#[test]
fn rubber_chickens_flag_new_projectiles_and_change_nothing_else() {
    let shoot = |event: bool| {
        let mut w = world(base_setup(), brawl(), 2);
        let mut t = room();
        if event {
            activate(&mut w, &mut t, EventKind::RubberChickens);
        } else {
            for _ in 0..181 {
                w.step(&mut t, &[]);
            }
        }
        let p0 = PlayerId(0);
        let mut c = InputCmd::idle(w.tick() + 1);
        c.buttons = Buttons::FIRE_PRIMARY | Buttons::FIRE_SECONDARY;
        w.step(&mut t, &[(p0, c)]);
        w.projectiles()
            .iter()
            .map(|p| (p.flags, p.vel, p.damage, p.splash_damage, p.radius))
            .collect::<Vec<_>>()
    };
    let chickens = shoot(true);
    let plain = shoot(false);
    assert_eq!(chickens.len(), plain.len());
    assert!(!chickens.is_empty());
    for (c, p) in chickens.iter().zip(&plain) {
        assert_eq!(c.0 & proj_flags::CHICKEN, proj_flags::CHICKEN);
        assert_eq!(p.0 & proj_flags::CHICKEN, 0);
        assert_eq!(c.0 & !proj_flags::CHICKEN, p.0);
        assert_eq!((c.1, c.2, c.3, c.4), (p.1, p.2, p.3, p.4));
    }
}

// ----- Moving day -------------------------------------------------------------

#[test]
fn cyclic_permutations_move_every_point() {
    let mut rng = ls_math::Pcg32::new(5, 6);
    for n in 2..20 {
        let p = cycle(n, &mut rng);
        let mut sorted = p.clone();
        sorted.sort();
        assert_eq!(sorted, (0..n).collect::<Vec<_>>());
        assert!(p.iter().enumerate().all(|(i, j)| i != *j));
    }
}

#[test]
fn moving_day_moves_every_pickup_to_another_point_and_keeps_timers() {
    let mut w = world(base_setup(), sandbox(), 2);
    let mut t = room();
    for _ in 0..5 {
        w.step(&mut t, &[]);
    }
    let before = w.pickups().to_vec();
    let held = before.iter().filter(|p| holds_pickup(p)).count();
    assert!(held >= 4, "{held} pickups");
    let a = activate(&mut w, &mut t, EventKind::MovingDay);
    let after = w.pickups().to_vec();
    let points: Vec<Vec3> = before.iter().map(|p| p.pos).collect();
    let mut landed = Vec::new();
    for (b, n) in before.iter().zip(&after) {
        assert_eq!(b.content, n.content);
        assert_eq!(b.respawn_tick, n.respawn_tick);
        assert_eq!(b.pos, n.pos);
        if holds_pickup(b) {
            let at = n.content_pos();
            assert!(at.distance(b.pos) > 1.0, "moved");
            let j = points.iter().position(|p| p.distance(at) < 1e-4).unwrap();
            assert!(!landed.contains(&j), "one per point");
            landed.push(j);
        }
    }
    // Untouchable during the 2 s move: a ship parked on a moved pickup.
    let target = after.iter().find(|p| holds_pickup(p)).unwrap();
    let spot = target.content_pos();
    let p1 = PlayerId(1);
    let mut taken_at = None;
    for _ in 0..240 {
        if let Some(s) = w.ship_mut(p1) {
            s.pos = spot;
            s.vel = Vec3::ZERO;
        }
        for e in w.step(&mut t, &[]) {
            if let Event::PickupTaken { player, .. } = e {
                if player == p1 && taken_at.is_none() {
                    taken_at = Some(w.tick());
                }
            }
        }
    }
    let taken_at = taken_at.expect("taken after the move");
    assert!(taken_at > a + EventKind::MovingDay.def().active_ticks());
}

#[test]
fn moving_day_content_respawns_at_its_own_point() {
    let mut w = world(base_setup(), sandbox(), 2);
    let mut t = room();
    for _ in 0..5 {
        w.step(&mut t, &[]);
    }
    activate(&mut w, &mut t, EventKind::MovingDay);
    while w.mood_view().event.is_some() {
        w.step(&mut t, &[]);
    }
    let i = w.pickups().iter().position(holds_pickup).unwrap();
    let spot = w.pickups()[i].content_pos();
    let home = w.pickups()[i].pos;
    let p1 = PlayerId(1);
    let mut taken = false;
    for _ in 0..30 {
        w.ship_mut(p1).unwrap().pos = spot;
        for e in w.step(&mut t, &[]) {
            if matches!(e, Event::PickupTaken { spawn, .. } if spawn as usize == i) {
                taken = true;
            }
        }
        if taken {
            break;
        }
    }
    assert!(taken);
    assert_eq!(w.pickups()[i].offset, Vec3::ZERO);
    assert_eq!(w.pickups()[i].content_pos(), home);
}

// ----- Everything is cake -----------------------------------------------------

#[test]
fn cake_craters_are_bigger_and_cake_heals_twice() {
    let crater_radius = |w: &mut World, t: &mut BoxRoomTerrain| {
        let (_, evs) = w.hook_for_test(t, |ctx| ctx.crater(Vec3::new(0.0, -20.0, 0.0), 2.0));
        evs.iter()
            .find_map(|e| match e {
                Event::TerrainEdit(TerrainEdit::SubtractSphere { radius, .. }) => Some(*radius),
                _ => None,
            })
            .unwrap()
    };
    let mut w = world(base_setup(), brawl(), 2);
    let mut t = room();
    let plain = crater_radius(&mut w, &mut t);
    activate(&mut w, &mut t, EventKind::EverythingIsCake);
    w.step(&mut t, &[]);
    let cake = crater_radius(&mut w, &mut t);
    assert!((cake / plain - 1.44).abs() < 1e-5, "{plain} -> {cake}");
    // The wave starts at the ships' centroid.
    let origin = super::everything_is_cake::EverythingIsCake::origin(&w.mood_view().params);
    assert!(origin.distance(Vec3::ZERO) < 0.05, "{origin:?}");

    // Scavenging: twice the hull per scrap.
    let heal = |event: bool| {
        let mut w = world(base_setup(), brawl(), 2);
        let mut t = room();
        if event {
            activate(&mut w, &mut t, EventKind::EverythingIsCake);
        }
        let p = PlayerId(0);
        let pos = w.ship(p).unwrap().pos;
        w.ship_mut(p).unwrap().hull -= 60.0;
        w.spawn_entity(EntitySpawn::debris(pos + Vec3::X, Vec3::ZERO, 0.6, 4.0));
        let hull = w.ship(p).unwrap().hull;
        for _ in 0..120 {
            let mut c = InputCmd::idle(w.tick() + 1);
            c.buttons = Buttons::SCAVENGE;
            w.step(&mut t, &[(p, c)]);
        }
        w.ship(p).unwrap().hull - hull
    };
    let (cake, plain) = (heal(true), heal(false));
    assert!(plain > 0.0);
    assert!((cake / plain - 2.0).abs() < 1e-3, "{plain} -> {cake}");
}

// ----- prediction ---------------------------------------------------------------

/// A view taken after an earlier tick (a client's snapshot) advanced to
/// later ticks gives the effects the world applies on those ticks.
#[test]
fn advanced_views_match_the_world_tick_for_tick() {
    for kind in [
        EventKind::GravityCoffeeBreak,
        EventKind::BoredLevel,
        EventKind::LevelHiccups,
    ] {
        let mut w = world(well_setup(), brawl(), 1);
        let mut t = room();
        w.schedule_event(10, kind);
        let mut views: Vec<MoodView> = Vec::new();
        let total = 180 + kind.def().active_ticks() + kind.def().ending_ticks() + 30;
        for _ in 0..total {
            w.step(&mut t, &[]);
            views.push(w.mood_view());
            let now = w.tick();
            // Every snapshot up to 20 ticks old predicts this tick's effects.
            for back in 1..=20u64.min(views.len() as u64 - 1) {
                let old = views[views.len() - 1 - back as usize];
                let adv = old.advanced(now);
                let fx = if adv.event.is_some() {
                    event_effects(&adv, SEED, now)
                } else {
                    crate::EventEffects::NONE
                };
                assert_eq!(fx, w.event_effects(), "{kind:?} tick {now} back {back}");
            }
        }
    }
}
