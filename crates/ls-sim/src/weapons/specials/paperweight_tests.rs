//! Scenario tests of the Paperweight (`docs/milestone-4-plan.md` §5 wave B,
//! B7 acceptance).

use ls_math::terrain::{TerrainEdit, TerrainQuery, MATERIAL_WEAK};
use ls_math::{Aabb, Vec3};

use super::super::paint_thrower::testkit::*;
use super::{lump_radius, well_numbers, LANDED};
use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// Ticks of the well (6 s) and of the flight (1.2 s).
const WELL_TICKS: u64 = 360;
const FLIGHT_TICKS: u64 = 72;

/// Two ships (team 0, team 1) under Paint the Void rules with the
/// Paperweight allowed (it is a pickup in Belt Up and the sandbox only).
fn world2() -> (World, PlayerId, PlayerId) {
    let mut cfg = WorldConfig::paint_the_void(7, setup());
    cfg.rules
        .weapons
        .insert("paperweight".into(), Availability::PickupOnly);
    let mut w = World::new(cfg).unwrap();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    (w, a, b)
}

fn def() -> WeaponDef {
    let t = WeaponTable::builtin();
    t.get(t.find("paperweight").unwrap()).clone()
}

fn stone(w: &World) -> Option<Projectile> {
    let wid = w.weapons().find("paperweight").unwrap();
    w.projectiles().iter().find(|p| p.weapon == wid).cloned()
}

/// A huge open room: nothing to land on within 1.2 s.
fn open_room() -> TestRoom {
    let mut t = TestRoom::new();
    t.room = Aabb::from_center_half_extents(Vec3::ZERO, Vec3::splat(200.0));
    t
}

/// `a` throws one Paperweight from `from` along `dir`; runs `ticks` more.
fn throw<T: SimTerrain + ?Sized>(
    w: &mut World,
    t: &mut T,
    a: PlayerId,
    from: Vec3,
    dir: Vec3,
    ticks: u32,
) -> Vec<Event> {
    place(w, a, from, dir);
    let mut ev = run(w, t, &press(a, Buttons::FIRE_SECONDARY), 1);
    ev.extend(run(w, t, &[], ticks));
    ev
}

fn grown(ev: &[Event]) -> Vec<(Vec3, f32)> {
    edits(ev)
        .iter()
        .filter_map(|e| match e {
            TerrainEdit::AddSphere { center, radius } => Some((*center, *radius)),
            _ => None,
        })
        .collect()
}

#[test]
fn data_matches_the_plan() {
    let d = def();
    assert!(crate::weapons::behavior("paperweight").implemented());
    assert_eq!(d.family, WeaponFamily::Special);
    assert_eq!(d.slot, WeaponSlot::Secondary);
    assert_eq!(d.max_ammo, 3);
    assert_eq!(d.fire_interval_secs, 2.0);
    assert_eq!(d.speed, 25.0);
    assert_eq!(d.lifetime_ticks(), FLIGHT_TICKS);
    assert_eq!(d.damage, 0.0);
    assert!(d.gravity, "other wells bend it in flight");
    // §3.8: range 16 u, peak pull 30 % of the Tinker's per-axis thrust,
    // on the wire grid (0.25 u, 0.125 u/s^2).
    let tinker = ShipTuning::tinker().thrust_accel;
    let (range, peak) = well_numbers(&d, tinker);
    assert_eq!(range, 16.0);
    assert_eq!(peak, 0.3 * tinker);
    assert_eq!((peak / 0.125).fract(), 0.0);
    // The lump: 1 u alone, growing with debris, at most 2 u.
    assert_eq!(lump_radius(&d, []), 1.0);
    let r = lump_radius(&d, [1.0, 1.0]);
    assert!((r - 3.0f32.cbrt()).abs() < 1e-4, "{r}");
    assert_eq!(lump_radius(&d, [1.5; 8]), 2.0);
}

#[test]
fn stops_where_it_hits_rock_and_becomes_a_well_for_six_seconds() {
    let (mut w, a, b) = world2();
    let mut t = TestRoom::new();
    equip(&mut w, a, "paperweight");
    place(&mut w, b, Vec3::new(20.0, 10.0, 20.0), Vec3::Z);
    // The far wall is at z = -30, 15 u ahead: it lands after ~0.6 s.
    let ev = throw(
        &mut w,
        &mut t,
        a,
        Vec3::new(0.0, 0.0, -15.0),
        Vec3::NEG_Z,
        60,
    );
    assert!(edits(&ev).is_empty(), "no crater");
    let p = stone(&w).expect("the stone stays");
    assert!(p.is_stuck() && p.flags & LANDED != 0);
    assert_eq!(p.vel, Vec3::ZERO);
    assert!((p.pos.z - (-30.0 + p.radius)).abs() < 1e-3, "{:?}", p.pos);
    let wells = w.dynamic_wells().to_vec();
    assert_eq!(wells.len(), 1);
    let well = wells[0];
    assert_eq!(well.pos, p.pos);
    assert_eq!(well.range, 16.0);
    assert_eq!(well.peak_accel, 0.3 * w.tuning().thrust_accel);
    assert_eq!(well.owner, Some(a));
    assert_eq!(well.key, p.id);
    assert_eq!(well.end_tick - well.start_tick, WELL_TICKS);
    let landed = well.start_tick - 1;
    assert!(landed > 1 && landed < 60, "{landed}");
    // It stays put for the 6 s ...
    let n = (well.end_tick - w.tick() - 2) as u32;
    run(&mut w, &mut t, &[], n);
    assert_eq!(stone(&w).unwrap().pos, p.pos);
    assert_eq!(w.dynamic_wells().len(), 1);
    // ... and then it is gone (fused into a lump).
    let ev = run(&mut w, &mut t, &[], 3);
    assert!(stone(&w).is_none());
    assert!(w.dynamic_wells().is_empty());
    assert_eq!(grown(&ev).len(), 1);
}

#[test]
fn stops_after_one_point_two_seconds_of_flight() {
    let (mut w, a, b) = world2();
    let mut t = open_room();
    equip(&mut w, a, "paperweight");
    place(&mut w, b, Vec3::new(50.0, 0.0, 0.0), Vec3::Z);
    throw(
        &mut w,
        &mut t,
        a,
        Vec3::ZERO,
        Vec3::NEG_Z,
        FLIGHT_TICKS as u32 - 2,
    );
    let p = stone(&w).unwrap();
    assert!(p.flags & LANDED == 0 && p.vel.z < 0.0, "still flying");
    assert!(w.dynamic_wells().is_empty());
    run(&mut w, &mut t, &[], 3);
    let p = stone(&w).unwrap();
    assert!(p.flags & LANDED != 0 && p.vel == Vec3::ZERO);
    // 1.2 s at 25 u/s from the muzzle.
    let flown = -p.pos.z;
    assert!((flown - 30.0).abs() < 1.5, "{flown}");
    assert_eq!(w.dynamic_wells().len(), 1);
    assert_eq!(w.dynamic_wells()[0].pos, p.pos);
}

#[test]
fn passes_through_ships_and_does_no_damage() {
    let (mut w, a, b) = world2();
    let mut t = TestRoom::new();
    equip(&mut w, a, "paperweight");
    // b sits in the flight path 6 u ahead.
    place(&mut w, b, Vec3::new(0.0, 0.0, -6.0), Vec3::Z);
    let ev = throw(&mut w, &mut t, a, Vec3::ZERO, Vec3::NEG_Z, 90);
    assert_eq!(damage_to(&ev, b), 0.0);
    assert!(!ev.iter().any(|e| matches!(e, Event::Damaged { .. })));
    assert!(!ev.iter().any(|e| matches!(e, Event::Impact { .. })));
    // It flew on past b and landed at the wall.
    let p = stone(&w).unwrap();
    assert!(p.flags & LANDED != 0 && p.pos.z < -20.0, "{:?}", p.pos);
    // A whole well's life right next to it: still no damage.
    place(&mut w, b, p.pos + Vec3::new(0.0, 0.0, 3.0), Vec3::Z);
    let ev = run(&mut w, &mut t, &[], WELL_TICKS as u32);
    assert_eq!(damage_to(&ev, b), 0.0);
    assert_eq!(damage_to(&ev, a), 0.0);
    assert_eq!(w.ship(b).unwrap().hull, w.tuning().hull_max);
}

#[test]
fn pulls_ships_its_owner_too_within_its_range_while_it_lasts() {
    let (mut w, a, b) = world2();
    let mut t = open_room();
    equip(&mut w, a, "paperweight");
    place(&mut w, b, Vec3::new(80.0, 0.0, 0.0), Vec3::Z);
    throw(
        &mut w,
        &mut t,
        a,
        Vec3::ZERO,
        Vec3::NEG_Z,
        FLIGHT_TICKS as u32 + 2,
    );
    let well = w.dynamic_wells()[0];
    let c = well.pos;
    // The thrower 8 u from it (inside), b 18 u from it (outside).
    place(&mut w, a, c + Vec3::new(8.0, 0.0, 0.0), Vec3::NEG_Z);
    place(&mut w, b, c + Vec3::new(0.0, 18.0, 0.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 60);
    let (pa, pb) = (w.ship(a).unwrap().pos, w.ship(b).unwrap().pos);
    assert!(pa.x < c.x + 7.5, "the owner falls toward it: {pa:?}");
    assert!(
        pb.distance(c + Vec3::new(0.0, 18.0, 0.0)) < 1e-3,
        "out of range: {pb:?}"
    );
    // The pull follows the force law: peak 30 % of the thrust at s / sqrt 2,
    // nothing at the range.
    let f = w.well_field();
    let now = w.tick();
    let s = GravitySettings::builtin().softening;
    let at = |r: f32| f.accel_at(c + Vec3::new(r, 0.0, 0.0), now).length();
    let taper = |r: f32| (1.0 - r * r / 256.0).powi(2);
    let rp = s / 2.0f32.sqrt();
    assert!((at(rp) - 9.0 * taper(rp)).abs() < 1e-3, "{}", at(rp));
    assert_eq!(at(16.0), 0.0);
    assert!(at(15.9) > 0.0);
    // After the 6 s nothing pulls: a ship there keeps still.
    run(&mut w, &mut t, &[], WELL_TICKS as u32);
    assert!(w.dynamic_wells().is_empty());
    place(&mut w, a, c + Vec3::new(8.0, 0.0, 0.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 30);
    assert!(
        w.ship(a)
            .unwrap()
            .pos
            .distance(c + Vec3::new(8.0, 0.0, 0.0))
            < 1e-3
    );
}

/// Angle (rad) a projectile of `weapon` fired by `b` past the landed well
/// (5 u off its centre) turns in `ticks`.
fn bend_past_a_paperweight(weapon: &str, ticks: u32) -> f32 {
    let (mut w, a, b) = world2();
    let mut t = open_room();
    equip(&mut w, a, "paperweight");
    throw(
        &mut w,
        &mut t,
        a,
        Vec3::ZERO,
        Vec3::NEG_Z,
        FLIGHT_TICKS as u32 + 2,
    );
    let c = w.dynamic_wells()[0].pos;
    place(&mut w, a, Vec3::new(150.0, 0.0, 0.0), Vec3::Z);
    let wid = equip(&mut w, b, weapon);
    let def = w.weapons().get(wid).clone();
    let button = match def.slot {
        WeaponSlot::Primary => Buttons::FIRE_PRIMARY,
        WeaponSlot::Secondary => Buttons::FIRE_SECONDARY,
    };
    // b fires along +x, passing 5 u from the well's centre.
    place(&mut w, b, c + Vec3::new(-12.0, 5.0, 0.0), Vec3::X);
    run(&mut w, &mut t, &press(b, button), 1);
    let p0 = w
        .projectiles()
        .iter()
        .find(|p| p.weapon == wid)
        .expect("fired")
        .clone();
    run(&mut w, &mut t, &[], ticks);
    let p1 = w
        .projectiles()
        .iter()
        .find(|p| p.id == p0.id)
        .expect("still flying");
    let (d0, d1) = (p0.vel.normalize(), p1.vel.normalize());
    ls_math::det::acos(d0.dot(d1).clamp(-1.0, 1.0))
}

#[test]
fn bends_flagged_ordnance_but_not_bolts_or_kinetic_rounds() {
    assert_eq!(
        bend_past_a_paperweight("scrap_laser", 40),
        0.0,
        "energy bolt"
    );
    assert_eq!(
        bend_past_a_paperweight("rivet_gun", 20),
        0.0,
        "kinetic round"
    );
    let rocket = bend_past_a_paperweight("bolt_rocket", 40);
    assert!(rocket > 0.02, "bolt rocket bends: {rocket}");
    let seed = bend_past_a_paperweight("seed_grenade", 40);
    assert!(seed > 0.02, "seed grenade bends: {seed}");
}

#[test]
fn pulls_debris_and_fuses_it_into_a_lump_of_weak_rock() {
    let (mut w, a, b) = world2();
    let mut t = open_room();
    equip(&mut w, a, "paperweight");
    place(&mut w, b, Vec3::new(80.0, 0.0, 0.0), Vec3::Z);
    throw(
        &mut w,
        &mut t,
        a,
        Vec3::ZERO,
        Vec3::NEG_Z,
        FLIGHT_TICKS as u32 + 2,
    );
    let c = w.dynamic_wells()[0].pos;
    place(&mut w, a, Vec3::new(80.0, 20.0, 0.0), Vec3::Z);
    // Three resting debris pieces 6-10 u away, one far outside the range.
    let near: Vec<u32> = [(Vec3::X, 6.0), (Vec3::Y, 8.0), (Vec3::NEG_X, 10.0)]
        .iter()
        .map(|&(dir, r)| w.spawn_entity(EntitySpawn::debris(c + dir * r, Vec3::ZERO, 0.8, 3.0)))
        .collect();
    let far = w.spawn_entity(EntitySpawn::debris(
        c + Vec3::new(0.0, 0.0, 30.0),
        Vec3::ZERO,
        0.8,
        3.0,
    ));
    let pos_of = |w: &World, id: u32| w.entities().iter().find(|e| e.id == id).map(|e| e.pos);
    // 10 u out, it takes the well's 6 s to haul a piece in.
    run(&mut w, &mut t, &[], 340);
    for &id in &near {
        let p = pos_of(&w, id).unwrap();
        assert!(p.distance(c) < 4.0, "debris gathers: {}", p.distance(c));
    }
    let ev = run(&mut w, &mut t, &[], 60);
    // The gathered debris fused; the far piece stayed.
    for &id in &near {
        assert!(pos_of(&w, id).is_none(), "fused");
    }
    assert!(pos_of(&w, far).is_some());
    let g = grown(&ev);
    assert_eq!(g.len(), 1);
    let (center, r) = g[0];
    assert_eq!(center, c);
    assert_eq!(r, lump_radius(&def(), [0.8; 3]));
    assert!(r > 1.0 && r <= 2.0, "{r}");
    // A floating lump of weak rock.
    assert!(t.sdf(c) < 0.0);
    assert!(t.sdf(c + Vec3::new(0.0, r + 0.5, 0.0)) > 0.0);
    assert_eq!(w.instability().material_at(c), Some(MATERIAL_WEAK));
}

#[test]
fn the_lump_never_overlaps_a_ship() {
    let d = def();
    let max = d.param("lump_max_radius", 2.0);
    // A ship held at 0.6-4 u from the stone when the well ends.
    for i in 0..12 {
        let (mut w, a, b) = world2();
        let mut t = open_room();
        equip(&mut w, a, "paperweight");
        place(&mut w, b, Vec3::new(80.0, 0.0, 0.0), Vec3::Z);
        throw(
            &mut w,
            &mut t,
            a,
            Vec3::ZERO,
            Vec3::NEG_Z,
            FLIGHT_TICKS as u32 + 2,
        );
        let well = w.dynamic_wells()[0];
        let c = well.pos;
        for k in 0..4 {
            w.spawn_entity(EntitySpawn::debris(
                c + Vec3::new(0.5 * k as f32, 0.3, 0.0),
                Vec3::ZERO,
                1.2,
                3.0,
            ));
        }
        let n = (well.end_tick - w.tick() - 3) as u32;
        run(&mut w, &mut t, &[], n);
        let dist = 0.6 + 0.3 * i as f32;
        let dir = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::NEG_Y][i % 4];
        let mut ev = Vec::new();
        for _ in 0..6 {
            place(&mut w, b, c + dir * dist, Vec3::Z);
            place(&mut w, a, Vec3::new(80.0, 20.0, 0.0), Vec3::Z);
            ev.extend(run(&mut w, &mut t, &[], 1));
        }
        assert!(w.dynamic_wells().is_empty());
        let rb = w.tuning().radius;
        let bp = w.ship(b).unwrap().pos;
        let g = grown(&ev);
        for &(center, radius) in &g {
            assert!(
                center.distance(bp) > radius + rb,
                "lump r {radius} swallows b at {dist} u"
            );
        }
        if dist > max + rb + 0.3 {
            assert_eq!(g.len(), 1, "grows with b {dist} u away");
        } else {
            assert!(g.is_empty(), "left out with b {dist} u away");
            // The debris stays where it was.
            assert!(w
                .entities()
                .iter()
                .any(|e| e.kind == EntityKind::Debris && e.pos.distance(c) < 5.0));
        }
    }
}

#[test]
fn three_stacked_paperweights_stay_within_the_clamp() {
    let (mut w, a, b) = world2();
    let mut t = TestRoom::new();
    equip(&mut w, a, "paperweight");
    place(&mut w, b, Vec3::new(20.0, 10.0, 20.0), Vec3::Z);
    // Three throws at the same spot of the far wall, 2 s apart.
    for _ in 0..3 {
        throw(
            &mut w,
            &mut t,
            a,
            Vec3::new(0.0, 0.0, -15.0),
            Vec3::NEG_Z,
            120,
        );
    }
    let wells = w.dynamic_wells().to_vec();
    assert_eq!(wells.len(), 3, "all three pull at once");
    let now = w.tick();
    assert!(wells.iter().all(|wl| wl.active(now)));
    let tinker = w.tuning().thrust_accel;
    let c = wells[0].pos;
    let mut worst = 0.0f32;
    for i in 0..200 {
        let r = 0.5 + 0.075 * i as f32;
        let p = c + Vec3::new(0.3 * r, 0.2 * r, r);
        let raw = w.well_field().accel_at(p, now).length();
        let ship = w.well_field().ship_accel(p, now, tinker).length();
        assert!(ship <= 0.4 * tinker + 1e-4, "{ship} at {r}");
        worst = worst.max(raw);
    }
    assert!(worst > 0.4 * tinker, "the stack alone is stronger: {worst}");
    // A Tinker 4 u in front of the stack escapes at full thrust.
    place(&mut w, b, c + Vec3::new(0.0, 0.0, 4.0), Vec3::Z);
    let start = w.ship(b).unwrap().pos.distance(c);
    let mut cmd = InputCmd::idle(0);
    cmd.thrust = Vec3::new(0.0, 0.0, -1.0);
    run(&mut w, &mut t, &[(b, cmd)], 60);
    let end = w.ship(b).unwrap().pos.distance(c);
    assert!(end > start + 5.0, "escapes: {start} -> {end}");
}

#[test]
fn a_sneeze_does_not_move_a_landed_paperweight() {
    let (mut w, a, b) = world2();
    let mut t = open_room();
    equip(&mut w, a, "paperweight");
    throw(
        &mut w,
        &mut t,
        a,
        Vec3::ZERO,
        Vec3::NEG_Z,
        FLIGHT_TICKS as u32 + 2,
    );
    let p = stone(&w).unwrap();
    equip(&mut w, b, "magnet_sneeze");
    place(&mut w, b, p.pos + Vec3::new(0.0, 0.0, -8.0), Vec3::Z);
    place(&mut w, a, Vec3::new(80.0, 0.0, 0.0), Vec3::Z);
    run(&mut w, &mut t, &press(b, Buttons::FIRE_SECONDARY), 1);
    run(&mut w, &mut t, &[], 10);
    let q = stone(&w).unwrap();
    assert_eq!(q.pos, p.pos);
    assert_eq!(w.dynamic_wells()[0].pos, p.pos);
}
