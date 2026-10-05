//! World-level tests: weapons, resources, rule-set enforcement, friendly fire,
//! Paint the Void and replay determinism.

use ls_math::terrain::{BoxRoomTerrain, TerrainEdit, TerrainQuery};
use ls_math::{Pcg32, Quat, Vec3};

use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

type Room = PaintedTerrain<BoxRoomTerrain>;

fn room(size: Vec3) -> Room {
    PaintedTerrain::new(BoxRoomTerrain::centered(size), 1.0)
}

fn std_room() -> Room {
    room(Vec3::new(60.0, 30.0, 60.0))
}

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
        pickup_points: vec![],
        instability: 0,
        heist: None,
        ..Default::default()
    }
}

fn world_with(rules: RuleSet, setup: MatchSetup) -> World {
    let mut cfg = WorldConfig::paint_the_void(42, setup);
    cfg.rules = rules;
    World::new(cfg).unwrap()
}

fn world() -> World {
    world_with(RuleSet::paint_the_void(), setup())
}

/// Places a ship at `pos` looking along `dir`, at rest.
fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
    s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
}

fn run<T: SimTerrain>(
    w: &mut World,
    t: &mut T,
    inputs: &[(PlayerId, InputCmd)],
    ticks: u32,
) -> Vec<Event> {
    let mut all = Vec::new();
    for _ in 0..ticks {
        all.extend(w.step(t, inputs));
    }
    all
}

fn fire(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
    [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_PRIMARY))]
}

fn fire_secondary(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
    [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_SECONDARY))]
}

fn select(w: &World, id: PlayerId, weapon: &str) -> [(PlayerId, InputCmd); 1] {
    let wid = w.weapons().find(weapon).unwrap();
    [(
        id,
        InputCmd {
            weapon_select: wid.0 + 1,
            ..InputCmd::idle(0)
        },
    )]
}

fn damage_events(events: &[Event]) -> Vec<(PlayerId, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Damaged { target, amount, .. } => Some((*target, *amount)),
            _ => None,
        })
        .collect()
}

fn fired_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Fired { .. }))
        .count()
}

// ----- setup ---------------------------------------------------------------

#[test]
fn players_spawn_with_loadout_and_balance_teams() {
    let mut w = world();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let b = w.add_player(None, ShipType::Tinker).unwrap();
    let c = w.add_player(None, ShipType::Tinker).unwrap();
    assert_eq!(w.ship(a).unwrap().team, 0);
    assert_eq!(w.ship(b).unwrap().team, 1);
    assert_eq!(w.ship(c).unwrap().team, 0);
    let s = w.ship(b).unwrap();
    assert_eq!(s.pos, Vec3::new(20.0, 0.0, 0.0));
    assert_eq!(s.hull, 100.0);
    let laser = w.weapons().find("scrap_laser").unwrap();
    let rocket = w.weapons().find("bolt_rocket").unwrap();
    assert_eq!(s.primary, Some(laser));
    assert_eq!(s.secondary, Some(rocket));
    assert_eq!(s.weapon(rocket).unwrap().ammo, 3);
    assert!(s
        .weapon(w.weapons().find("paint_thrower").unwrap())
        .is_some());

    assert_eq!(s.gadget, w.gadgets().find("shield"), "loadout gadget");
    assert!(!s.gadget_active);
    assert!(matches!(
        w.add_player(Some(5), ShipType::Tinker),
        Err(SimError::InvalidTeam(5))
    ));
    let mut rules = RuleSet::paint_the_void();
    rules.allowed_ship_types = vec![ShipType::Tinker];
    let mut w2 = world_with(rules, setup());
    assert!(matches!(
        w2.add_player(None, ShipType::Hauler),
        Err(SimError::ShipTypeNotAllowed(_))
    ));
}

// ----- weapons -------------------------------------------------------------

#[test]
fn scrap_laser_speed_lifetime_and_no_velocity_inheritance() {
    // Large room without a paint grid (the grid scan would be 64M cells).
    let mut t = BoxRoomTerrain::centered(Vec3::splat(400.0));
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    // Strafe sideways at full cruise speed, then fire.
    run(
        &mut w,
        &mut t,
        &[(a, InputCmd::idle(0).with_thrust(Vec3::X))],
        60,
    );
    assert_eq!(w.ship(a).unwrap().vel, Vec3::new(12.0, 0.0, 0.0));
    let ev = run(&mut w, &mut t, &fire(a), 1);
    assert_eq!(fired_count(&ev), 1);
    let p = w.projectiles()[0].clone();
    // Muzzle velocity only: exactly forward at 30 u/s, no sideways component.
    assert_eq!(p.vel, Vec3::new(0.0, 0.0, -30.0));
    let start = p.pos;

    // 3 s lifetime: 180 moves in total (the first on the firing tick), i.e.
    // 90 u of flight, then gone without impact.
    let ev = run(&mut w, &mut t, &[], 179);
    assert!(ev.iter().all(|e| !matches!(e, Event::Impact { .. })));
    let p = &w.projectiles()[0];
    assert!((p.pos.z - (start.z - 89.5)).abs() < 1e-2, "{:?}", p.pos);
    assert_eq!(p.pos.x, start.x, "flies straight; no sideways drift");
    run(&mut w, &mut t, &[], 1);
    assert!(w.projectiles().is_empty());
}

#[test]
fn scrap_laser_hits_enemy_with_damage_and_knockback() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    let energy = w.ship(a).unwrap().energy;
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    assert!((w.ship(a).unwrap().energy - (energy - 3.0)).abs() < 1e-4);
    ev.extend(run(&mut w, &mut t, &[], 30));
    assert_eq!(damage_events(&ev), vec![(b, 6.0)]);
    let sb = w.ship(b).unwrap();
    assert_eq!(sb.hull, 94.0);
    assert_eq!(sb.last_attacker, Some(a));
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Impact {
            target: Some(t),
            ..
        } if *t == b
    )));
}

#[test]
fn laser_upgrade_levels_raise_damage() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    w.give_weapon(a, "scrap_laser").unwrap();
    assert_eq!(
        w.ship(a)
            .unwrap()
            .weapon(w.weapons().find("scrap_laser").unwrap())
            .unwrap()
            .level,
        2
    );
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    ev.extend(run(&mut w, &mut t, &[], 30));
    let dmg = damage_events(&ev);
    assert_eq!(dmg.len(), 1);
    assert!((dmg[0].1 - 6.0 * 1.26667).abs() < 1e-4, "{dmg:?}");
}

#[test]
fn rivet_gun_is_fast_consumes_scrap_and_does_not_tunnel() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    // Pickup-only weapon: give it as a pickup would, then select it.
    w.give_weapon(a, "rivet_gun").unwrap();
    let sel = select(&w, a, "rivet_gun");
    run(&mut w, &mut t, &sel, 1);
    place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);

    w.ship_mut(a).unwrap().scrap = 2.0;
    let ev = run(&mut w, &mut t, &fire(a), 1);
    assert_eq!(fired_count(&ev), 1);
    assert_eq!(w.ship(a).unwrap().scrap, 1.5, "half a scrap per rivet");
    // 180 u/s = 3 u per tick; the ship sphere is only 1.1 u across, but the
    // swept test still hits it within 7 ticks (20 u / 3 u).
    let rivet = &w.projectiles()[0];
    assert_eq!(rivet.vel.length(), 180.0);
    let ev = run(&mut w, &mut t, &[], 7);
    assert_eq!(damage_events(&ev), vec![(b, 3.0)]);

    // Kinetic weapons need scrap: empty scrap store means no shot.
    w.ship_mut(a).unwrap().scrap = 0.0;
    let ev = run(&mut w, &mut t, &fire(a), 30);
    assert_eq!(fired_count(&ev), 0);
}

#[test]
fn bolt_rocket_crater_and_splash_knockback() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::new(0.0, -7.0, 0.0), Vec3::NEG_Y);
    // B sits 1.5 u to the side of the impact point, off the rocket's path.
    place(&mut w, b, Vec3::new(1.5, -14.4, 0.0), Vec3::Z);
    let floor_below = Vec3::new(0.0, -15.5, 0.0);
    assert!(t.sdf(floor_below) < 0.0);

    let rocket = w.weapons().find("bolt_rocket").unwrap();
    let mut ev = run(&mut w, &mut t, &fire_secondary(a), 1);
    assert_eq!(w.ship(a).unwrap().weapon(rocket).unwrap().ammo, 2);
    assert!((w.projectiles()[0].vel.length() - 40.0).abs() < 1e-4);
    ev.extend(run(&mut w, &mut t, &[], 20));
    let crater = ev
        .iter()
        .find_map(|e| match e {
            Event::TerrainEdit(TerrainEdit::SubtractSphere { center, radius }) => {
                Some((*center, *radius))
            }
            _ => None,
        })
        .expect("rocket leaves a crater");
    // Design doc: 2 u crater (diameter).
    assert_eq!(crater.1, 1.0);
    assert!((crater.0.y + 15.0).abs() < 0.01, "{:?}", crater.0);
    assert!(t.sdf(floor_below) > 0.0, "floor was carved away");

    // Splash: B takes falloff damage and is pushed away from the blast.
    let hits = damage_events(&ev);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let (who, amount) = hits[0];
    assert_eq!(who, b);
    assert!(amount > 0.0 && amount < 20.0, "{amount}");
    let sb = w.ship(b).unwrap();
    assert!(sb.hull < 100.0);
    // Knockback outward (+X, +Y) — damping has bled some of it off by now,
    // so check B moved away from the impact.
    assert!(sb.pos.x > 1.5, "{:?}", sb.pos);
}

#[test]
fn bolt_rocket_self_splash_and_ammo_runs_out() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    // Point-blank at the floor: the owner is caught in the splash.
    place(&mut w, a, Vec3::new(0.0, -13.5, 0.0), Vec3::NEG_Y);
    let ev = run(&mut w, &mut t, &fire_secondary(a), 5);
    let hits = damage_events(&ev);
    assert!(hits.iter().any(|(p, _)| *p == a), "{hits:?}");

    // Fire the remaining two, then the launcher is empty.
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &fire_secondary(a), 180);
    assert_eq!(fired_count(&ev), 2);
    let rocket = w.weapons().find("bolt_rocket").unwrap();
    assert_eq!(w.ship(a).unwrap().weapon(rocket).unwrap().ammo, 0);
}

#[test]
fn paint_thrower_paints_in_team_color_without_damage() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let sel = select(&w, a, "paint_thrower");
    run(&mut w, &mut t, &sel, 1);
    // Face the -X wall (x = -30) from 5 u away.
    place(&mut w, a, Vec3::new(-25.0, 0.0, 0.0), Vec3::NEG_X);
    let ev = run(&mut w, &mut t, &fire(a), 30);
    let paints: Vec<u8> = ev
        .iter()
        .filter_map(|e| match e {
            Event::TerrainEdit(TerrainEdit::Paint { team, radius, .. }) => {
                assert_eq!(*radius, 2.0);
                Some(*team)
            }
            _ => None,
        })
        .collect();
    assert!(!paints.is_empty());
    assert!(paints.iter().all(|&p| p == paint_team_id(1)));
    let cov = t.paint_coverage().unwrap();
    assert!(cov.team_cells[1] > 0 && cov.team_cells[0] == 0);
    assert!(w.mode().coverage.team_cells[1] > 0, "score refreshed");

    // Hitting a ship does no damage.
    place(&mut w, a, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    let ev = run(&mut w, &mut t, &fire(a), 20);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Impact { target: Some(t), .. } if *t == b)));
    assert!(damage_events(&ev).is_empty());
    assert_eq!(w.ship(b).unwrap().hull, 100.0);
}

// ----- rule-set enforcement ------------------------------------------------

fn rules_rocket_banned() -> RuleSet {
    let mut rules = RuleSet::paint_the_void();
    rules
        .weapons
        .insert("bolt_rocket".into(), Availability::Banned);
    rules.starting_loadout.secondaries.clear();
    rules.pickups.remove("bolt_rocket");
    rules
}

#[test]
fn banned_weapon_cannot_be_fired() {
    let mut t = std_room();
    let mut w = world_with(rules_rocket_banned(), setup());
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let rocket = w.weapons().find("bolt_rocket").unwrap();
    assert!(
        w.ship(a).unwrap().weapon(rocket).is_none(),
        "not in loadout"
    );
    // Even if it ends up in the inventory, it never fires.
    {
        let s = w.ship_mut(a).unwrap();
        s.add_weapon(WeaponState {
            weapon: rocket,
            ammo: 5,
            level: 1,
            shots: 0,
        });
        s.secondary = Some(rocket);
    }
    let ev = run(&mut w, &mut t, &fire_secondary(a), 60);
    assert_eq!(fired_count(&ev), 0);
    assert_eq!(w.ship(a).unwrap().weapon(rocket).unwrap().ammo, 5);
}

#[test]
fn banned_weapon_cannot_be_picked_up() {
    let mut t = std_room();
    let mut s = setup();
    s.pickup_points = (0..24)
        .map(|i| Vec3::new(-20.0 + 2.0 * i as f32, 5.0, 10.0))
        .collect();
    let mut w = world_with(rules_rocket_banned(), s);
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    assert!(matches!(
        w.give_weapon(a, "bolt_rocket"),
        Err(SimError::WeaponBanned(_))
    ));
    let rocket = w.weapons().find("bolt_rocket").unwrap();
    let check = |w: &World| {
        for sp in w.pickups() {
            if let Some((_, PickupKind::Weapon { weapon, .. })) = sp.content {
                assert_ne!(weapon, rocket);
            }
        }
    };
    check(&w);
    // Fly along the pickup row for a while, collecting and respawning pickups.
    place(&mut w, a, Vec3::new(-22.0, 5.0, 10.0), Vec3::X);
    let fwd = [(a, InputCmd::idle(0).with_thrust(Vec3::NEG_Z))];
    for _ in 0..20 {
        let ev = run(&mut w, &mut t, &fwd, 120);
        for e in &ev {
            if let Event::PickupSpawned {
                kind: PickupKind::Weapon { weapon, .. },
                ..
            } = e
            {
                assert_ne!(*weapon, rocket);
            }
        }
        check(&w);
        place(&mut w, a, Vec3::new(-22.0, 5.0, 10.0), Vec3::X);
    }
    assert!(w.ship(a).unwrap().weapon(rocket).is_none());

    // A rule set listing a banned weapon as a pickup is rejected outright.
    let mut bad = rules_rocket_banned();
    bad.pickups.insert(
        "bolt_rocket".into(),
        PickupRule {
            weight: 1.0,
            respawn_secs: 10.0,
            ammo: 1,
            max_on_map: 0,
        },
    );
    assert!(World::new(WorldConfig {
        rules: bad,
        ..WorldConfig::paint_the_void(1, setup())
    })
    .is_err());
}

#[test]
fn loadout_only_weapon_is_not_a_pickup() {
    let mut rules = RuleSet::paint_the_void();
    rules.pickups.insert(
        "paint_thrower".into(),
        PickupRule {
            weight: 1.0,
            respawn_secs: 1.0,
            ammo: 0,
            max_on_map: 0,
        },
    );
    assert!(rules.resolve(&WeaponTable::builtin()).is_err());
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    assert!(matches!(
        w.give_weapon(a, "paint_thrower"),
        Err(SimError::WeaponNotCollectable(_))
    ));
}

#[test]
fn weapon_pickup_collect_and_respawn() {
    let mut t = std_room();
    let mut rules = RuleSet::paint_the_void();
    rules.pickups.retain(|k, _| k == "bolt_rocket");
    rules.upgrade_pickups.clear();
    rules.gadget_pickups.clear();
    rules.scrap_pickup = None;
    let mut s = setup();
    s.pickup_points = vec![Vec3::new(0.0, 0.0, -1.0)];
    let mut w = world_with(rules, s);
    let rocket = w.weapons().find("bolt_rocket").unwrap();
    assert!(matches!(
        w.pickups()[0].content,
        Some((_, PickupKind::Weapon { weapon, ammo: 3 })) if weapon == rocket
    ));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &[], 1);
    assert!(ev.iter().any(|e| matches!(e, Event::PickupTaken { .. })));
    assert_eq!(w.ship(a).unwrap().weapon(rocket).unwrap().ammo, 6);
    assert!(w.pickups()[0].content.is_none());
    // Respawns after 20 s.
    place(&mut w, a, Vec3::new(20.0, 0.0, 0.0), Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &[], 1199);
    assert!(!ev.iter().any(|e| matches!(e, Event::PickupSpawned { .. })));
    let ev = run(&mut w, &mut t, &[], 1);
    assert!(ev.iter().any(|e| matches!(e, Event::PickupSpawned { .. })));
}

// ----- friendly fire ---------------------------------------------------------

fn laser_damage_with_ff(ff: u8, same_team: bool) -> f32 {
    let mut rules = RuleSet::paint_the_void();
    rules.friendly_fire_percent = ff;
    let mut t = std_room();
    let mut w = world_with(rules, setup());
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w
        .add_player(Some(if same_team { 0 } else { 1 }), ShipType::Tinker)
        .unwrap();
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    ev.extend(run(&mut w, &mut t, &[], 30));
    let hull_lost = 100.0 - w.ship(b).unwrap().hull;
    let reported: f32 = damage_events(&ev).iter().map(|(_, d)| d).sum();
    assert!((hull_lost - reported).abs() < 1e-4);
    reported
}

#[test]
fn friendly_fire_scales_teammate_damage() {
    assert_eq!(laser_damage_with_ff(0, true), 0.0);
    assert_eq!(laser_damage_with_ff(50, true), 3.0);
    assert_eq!(laser_damage_with_ff(100, true), 6.0);
    assert!((laser_damage_with_ff(10, true) - 0.6).abs() < 1e-5);
    // Enemies always take full damage.
    assert_eq!(laser_damage_with_ff(0, false), 6.0);
    assert_eq!(laser_damage_with_ff(50, false), 6.0);
}

// ----- resources -------------------------------------------------------------

#[test]
fn overdrive_burns_two_hull_per_energy_only_when_empty() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    let over = [(
        a,
        InputCmd::idle(0).with_buttons(Buttons::FIRE_PRIMARY | Buttons::OVERDRIVE),
    )];

    // Energy available: overdrive changes nothing.
    w.ship_mut(a).unwrap().energy = 50.0;
    run(&mut w, &mut t, &over, 1);
    let s = w.ship(a).unwrap();
    assert!(
        (s.energy - (47.0 + 10.0 / 60.0)).abs() < 1e-4,
        "{}",
        s.energy
    );
    assert_eq!(s.hull, 100.0);

    // Empty energy without overdrive: no shot.
    w.ship_mut(a).unwrap().energy = 0.0;
    w.ship_mut(a).unwrap().primary_ready_tick = 0;
    let ev = run(&mut w, &mut t, &fire(a), 1);
    assert_eq!(fired_count(&ev), 0);
    assert_eq!(w.ship(a).unwrap().hull, 100.0);

    // Empty energy with overdrive: the missing energy costs 2 hull each. One
    // tick of regen (1/6 energy) happens before firing, so 3 - 1/6 is missing.
    let s = w.ship_mut(a).unwrap();
    s.energy = 0.0;
    s.primary_ready_tick = 0;
    let ev = run(&mut w, &mut t, &over, 1);
    assert_eq!(fired_count(&ev), 1);
    let s = w.ship(a).unwrap();
    let regen = 10.0f32 / 60.0;
    assert!(
        (s.hull - (100.0 - 2.0 * (3.0 - regen))).abs() < 1e-4,
        "{}",
        s.hull
    );
    assert_eq!(s.energy, 0.0);

    // Overdrive never kills the pilot.
    let s = w.ship_mut(a).unwrap();
    s.energy = 0.0;
    s.hull = 5.0;
    s.primary_ready_tick = 0;
    let ev = run(&mut w, &mut t, &over, 1);
    assert_eq!(fired_count(&ev), 0);
    assert_eq!(w.ship(a).unwrap().hull, 5.0);
}

#[test]
fn hull_only_repairs_by_scavenging_near_scrap() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    w.ship_mut(a).unwrap().hull = 50.0;
    let pile = w.add_scrap_pile(Vec3::new(2.0, 0.0, 0.0), 100.0);
    let pile_amount = |w: &World| {
        w.scrap_piles()
            .iter()
            .find(|p| p.id == pile)
            .unwrap()
            .amount
    };

    // Hull never regenerates on its own, even next to scrap.
    run(&mut w, &mut t, &[], 120);
    assert_eq!(w.ship(a).unwrap().hull, 50.0);

    // Holding scavenge next to scrap: 20 scrap/s repairs 20 hull/s.
    let scav = [(a, InputCmd::idle(0).with_buttons(Buttons::SCAVENGE))];
    run(&mut w, &mut t, &scav, 60);
    let s = w.ship(a).unwrap();
    assert!((s.hull - 70.0).abs() < 1e-3, "{}", s.hull);
    assert!((pile_amount(&w) - 80.0).abs() < 1e-3);
    assert_eq!(s.scrap, 20.0, "hull is repaired first");

    // Too far from the pile: nothing.
    place(&mut w, a, Vec3::new(10.0, 0.0, 0.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &scav, 60);
    assert!((w.ship(a).unwrap().hull - 70.0).abs() < 1e-3);

    // Full hull: scavenged scrap goes to the scrap store (ammo for kinetics).
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    w.ship_mut(a).unwrap().hull = 100.0;
    run(&mut w, &mut t, &scav, 30);
    let s = w.ship(a).unwrap();
    assert_eq!(s.hull, 100.0);
    assert!((s.scrap - 30.0).abs() < 1e-3, "{}", s.scrap);
}

#[test]
fn kills_leave_scavengeable_wrecks_and_ships_respawn() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    w.ship_mut(b).unwrap().hull = 5.0;
    let mut ev = run(&mut w, &mut t, &fire(a), 1);
    ev.extend(run(&mut w, &mut t, &[], 30));
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Killed { victim, killer: Some(k), .. } if *victim == b && *k == a
    )));
    let sb = w.ship(b).unwrap();
    assert!(!sb.alive);
    assert_eq!(sb.deaths, 1);
    assert_eq!(w.ship(a).unwrap().kills, 1);
    // Wreck: 30 + half of B's 20 scrap.
    let wreck = &w.scrap_piles()[0];
    assert_eq!(wreck.amount, 40.0);
    assert!(wreck.pos.distance(Vec3::ZERO) < 0.5);

    // Dead ships do not move or get hit; respawn after 3 s at the team spawn.
    let ev = run(&mut w, &mut t, &[], 200);
    let respawned = ev
        .iter()
        .find_map(|e| match e {
            Event::Respawned { player, pos } if *player == b => Some(*pos),
            _ => None,
        })
        .expect("respawn");
    assert_eq!(respawned, Vec3::new(20.0, 0.0, 0.0));
    let sb = w.ship(b).unwrap();
    assert!(sb.alive);
    assert_eq!(sb.hull, 100.0);
    assert_eq!(sb.deaths, 1);
    assert_eq!(
        sb.weapon(w.weapons().find("bolt_rocket").unwrap())
            .unwrap()
            .ammo,
        3
    );
}

#[test]
fn scrap_pickup_spawns_and_respawns_when_emptied() {
    let mut t = std_room();
    let mut rules = RuleSet::paint_the_void();
    rules.pickups.clear();
    rules.upgrade_pickups.clear();
    rules.gadget_pickups.clear();
    let mut s = setup();
    s.pickup_points = vec![Vec3::new(0.0, 0.0, -2.0)];
    let mut w = world_with(rules, s);
    assert_eq!(w.scrap_piles().len(), 1);
    assert_eq!(w.scrap_piles()[0].amount, 30.0);
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    w.ship_mut(a).unwrap().hull = 10.0;
    let scav = [(a, InputCmd::idle(0).with_buttons(Buttons::SCAVENGE))];
    run(&mut w, &mut t, &scav, 100);
    assert!(w.scrap_piles().is_empty());
    assert!((w.ship(a).unwrap().hull - 40.0).abs() < 1e-3);
    assert!(w.pickups()[0].content.is_none());
    let ev = run(&mut w, &mut t, &[], 15 * 60);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::PickupSpawned {
            kind: PickupKind::Scrap { .. },
            ..
        }
    )));
    assert_eq!(w.scrap_piles().len(), 1);
}

// ----- Paint the Void --------------------------------------------------------

#[test]
fn paint_the_void_match_ends_after_five_minutes() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let _b = w.add_player(Some(0), ShipType::Tinker).unwrap();
    assert_eq!(w.mode().end_tick, 5 * 60 * 60);
    let sel = select(&w, a, "paint_thrower");
    run(&mut w, &mut t, &sel, 1);
    place(&mut w, a, Vec3::new(-25.0, 0.0, 0.0), Vec3::NEG_X);
    run(&mut w, &mut t, &fire(a), 60);

    let mut ended = None;
    for _ in 0..20_000 {
        for e in w.step(&mut t, &[]) {
            if let Event::MatchEnded { winner, team_cells } = e {
                ended = Some((w.tick(), winner, team_cells));
            }
        }
        if ended.is_some() {
            break;
        }
    }
    let (tick, winner, cells) = ended.expect("match ends");
    assert_eq!(tick, 18_000);
    assert_eq!(winner, Some(1));
    assert!(cells[1] > 0 && cells[0] == 0);
    assert!(w.mode().score_percent(1) > 0.0);
    assert_eq!(w.mode().phase, MatchPhase::Ended { winner: Some(1) });
    // Nothing happens after the end.
    assert!(w.step(&mut t, &fire(a)).is_empty());
    assert_eq!(w.tick(), 18_000);
}

// ----- determinism -----------------------------------------------------------

/// Scripted pseudo-random inputs; each ship also steers towards the nearest
/// enemy so fights actually happen. Depends only on the world state and the RNG.
fn scripted_inputs(
    rng: &mut Pcg32,
    w: &World,
    ids: &[PlayerId],
    tick: u64,
) -> Vec<(PlayerId, InputCmd)> {
    ids.iter()
        .map(|&id| {
            let (aim_pitch, aim_yaw) = aim_at_nearest_enemy(w, id);
            let mut buttons = Buttons::HORIZON_ASSIST;
            for (b, p) in [
                (Buttons::BOOST, 0.3),
                (Buttons::FIRE_PRIMARY, 0.4),
                (Buttons::FIRE_SECONDARY, 0.05),
                (Buttons::SCAVENGE, 0.2),
                (Buttons::OVERDRIVE, 0.1),
                (Buttons::GADGET, 0.03),
            ] {
                buttons.set(b, rng.chance(p));
            }
            // V now and then (Paint Thrower colours), without an RNG draw.
            buttons.set(Buttons::WEAPON_MODE, (tick + 17 * id.0 as u64) % 97 == 0);
            let axis = |r: &mut Pcg32| (r.below(3) as f32) - 1.0;
            let thrust = Vec3::new(axis(rng), axis(rng), axis(rng));
            (
                id,
                InputCmd {
                    tick,
                    thrust,
                    look_pitch: aim_pitch + rng.range_f32(-0.02, 0.02),
                    look_yaw: aim_yaw + rng.range_f32(-0.02, 0.02),
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

fn aim_at_nearest_enemy(w: &World, id: PlayerId) -> (f32, f32) {
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

/// Runs a scripted 4-player match and returns the state hash every 60 ticks
/// plus all events.
fn replay(seed: u64, ticks: u64, tweak_at: Option<u64>) -> (Vec<u64>, Vec<Event>) {
    let mut s = setup();
    s.spawns.push(SpawnPoint {
        pos: Vec3::new(-20.0, 5.0, 10.0),
        team: 0,
        orientation: Quat::IDENTITY,
    });
    s.spawns.push(SpawnPoint {
        pos: Vec3::new(20.0, 5.0, -10.0),
        team: 1,
        orientation: Quat::from_axis_angle(Vec3::Y, 3.0),
    });
    s.pickup_points = (0..8)
        .map(|i| Vec3::new(-14.0 + 4.0 * i as f32, -3.0, 0.0))
        .collect();
    let mut w = World::new(WorldConfig::paint_the_void(seed, s)).unwrap();
    let mut t = std_room();
    // Every ship type, and some debris in the arena.
    let ids: Vec<PlayerId> = ShipType::ALL
        .iter()
        .map(|&ty| w.add_player(None, ty).unwrap())
        .collect();
    for i in 0..6 {
        w.spawn_entity(EntitySpawn::debris(
            Vec3::new(-10.0 + 4.0 * i as f32, 2.0, -4.0),
            Vec3::new(0.5, 0.0, 0.25 * i as f32),
            0.4 + 0.2 * i as f32,
            5.0,
        ));
    }
    let mut rng = Pcg32::new(seed, 7);
    let mut hashes = Vec::new();
    let mut events = Vec::new();
    for tick in 1..=ticks {
        let mut inputs = scripted_inputs(&mut rng, &w, &ids, tick);
        if tweak_at == Some(tick) {
            inputs[0].1.look_yaw += 0.01;
        }
        events.extend(w.step(&mut t, &inputs));
        if tick % 60 == 0 {
            hashes.push(w.state_hash());
        }
    }
    (hashes, events)
}

#[test]
fn replay_same_inputs_same_state_hash() {
    let (h1, e1) = replay(1234, 1800, None);
    let (h2, e2) = replay(1234, 1800, None);
    assert_eq!(h1, h2);
    assert_eq!(e1, e2);
    // The scripted match actually exercises the systems.
    assert!(e1.iter().any(|e| matches!(e, Event::Fired { .. })));
    assert!(e1.iter().any(|e| matches!(e, Event::Damaged { .. })));
    assert!(e1.iter().any(|e| matches!(e, Event::TerrainEdit(_))));
    assert!(e1.iter().any(|e| matches!(e, Event::Killed { .. })));
    assert!(e1.iter().any(|e| matches!(e, Event::Respawned { .. })));
    // M2 systems take part too: gadgets and pickups.
    assert!(e1
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Gadget)));
    assert!(e1.iter().any(|e| matches!(e, Event::PickupTaken { .. })));
    // Paint colours: V switches them.
    assert!(e1
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::PaintColor)));
    // All hashes differ over time (the state evolves).
    let unique: std::collections::BTreeSet<_> = h1.iter().collect();
    assert_eq!(unique.len(), h1.len());

    // A tiny input change diverges the hash from that point on.
    let (h3, _) = replay(1234, 1800, Some(600));
    assert_eq!(h1[..9], h3[..9]);
    assert_ne!(h1[10..], h3[10..]);
}

/// Golden hash of the scripted replay. Must be identical on native and wasm32
/// (run the crate tests under `wasm-bindgen-test-runner`, see `CLAUDE.md`).
/// Changing simulation behavior or tuning changes it: update the constant in
/// the same change and say so in the commit. (M3 wave A kept it: Paint the
/// Void runs bit-identically through the mode framework, and the replay
/// fires no Tectonic Charge, whose cracks now spread over 6 ticks. VQ0's
/// contact estimate `d / |grad d|` kept it too: the replay arena's field is
/// exact, slope 1, so the estimate equals the bound there; the bound-field
/// path has its own golden hash, `ship::tests::bound_field_contact_golden_hash`.)
const REPLAY_GOLDEN_HASH: u64 = 0x8321_c0ab_902b_8ee5;

#[test]
fn replay_golden_hash() {
    let (hashes, _) = replay(1234, 1800, None);
    let last = *hashes.last().unwrap();
    assert_eq!(last, REPLAY_GOLDEN_HASH, "replay hash {last:#018x}");
}

#[test]
fn cloned_world_continues_identically() {
    let mut t1 = std_room();
    let mut w1 = world();
    let ids: Vec<PlayerId> = (0..2)
        .map(|_| w1.add_player(None, ShipType::Tinker).unwrap())
        .collect();
    let mut rng = Pcg32::new(9, 1);
    for tick in 1..=300 {
        let inputs = scripted_inputs(&mut rng, &w1, &ids, tick);
        w1.step(&mut t1, &inputs);
    }
    let mut w2 = w1.clone();
    let mut t2 = t1.clone();
    assert_eq!(w1.state_hash(), w2.state_hash());
    let mut rng2 = rng.clone();
    for tick in 301..=600 {
        let i1 = scripted_inputs(&mut rng, &w1, &ids, tick);
        let i2 = scripted_inputs(&mut rng2, &w2, &ids, tick);
        w1.step(&mut t1, &i1);
        w2.step(&mut t2, &i2);
    }
    assert_eq!(w1.state_hash(), w2.state_hash());
}

#[test]
fn state_hash_is_sensitive_to_ship_state() {
    let mut w = world();
    let a = w.add_player(None, ShipType::Tinker).unwrap();
    let h = w.state_hash();
    w.ship_mut(a).unwrap().scrap += 1.0;
    assert_ne!(w.state_hash(), h);
}

#[test]
fn world_new_rejects_bad_setup_and_loadout() {
    // No spawn points at all.
    let mut s = setup();
    s.spawns.clear();
    let cfg = WorldConfig::paint_the_void(1, s);
    assert!(matches!(World::new(cfg), Err(SimError::InvalidRuleSet(_))));
    // A spawn point for a team the format does not have.
    let mut s = setup();
    s.spawns[0].team = 4;
    let cfg = WorldConfig::paint_the_void(1, s);
    assert!(matches!(World::new(cfg), Err(SimError::InvalidTeam(4))));
    // A team without spawn points of its own spawns at the others'.
    let mut s = setup();
    s.spawns.retain(|sp| sp.team == 0);
    let mut w = World::new(WorldConfig::paint_the_void(1, s)).unwrap();
    let a = w.add_player(Some(1), ShipType::Tinker).unwrap();
    assert_eq!(w.ship(a).unwrap().pos, Vec3::new(-20.0, 0.0, 0.0));

    // More loadout secondaries than the ship has slots.
    let mut cfg = WorldConfig::paint_the_void(1, setup());
    assert!(!cfg.rules.starting_loadout.secondaries.is_empty());
    cfg.ships.get_mut(ShipType::Tinker).secondary_slots = 0;
    assert!(matches!(World::new(cfg), Err(SimError::InvalidRuleSet(_))));

    // A starting count above the weapon's max_ammo.
    let mut cfg = WorldConfig::paint_the_void(1, setup());
    cfg.rules.starting_loadout.secondaries[0].1 = u32::MAX;
    assert!(matches!(World::new(cfg), Err(SimError::InvalidRuleSet(_))));
}

#[test]
fn knockback_scales_with_damage_multiplier() {
    // Same hit with a doubled ship damage multiplier pushes twice as hard.
    let push = |mult: f32| {
        let mut cfg = WorldConfig::paint_the_void(7, setup());
        cfg.ships.get_mut(ShipType::Tinker).damage_mult = mult;
        let mut w = World::new(cfg).unwrap();
        let mut t = std_room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        run(&mut w, &mut t, &[], 1);
        place(&mut w, a, Vec3::new(-5.0, 0.0, 0.0), Vec3::X);
        place(&mut w, b, Vec3::new(5.0, 0.0, 0.0), Vec3::X);
        let fire_a = fire(a);
        let mut hit_vel = None;
        for _ in 0..60 {
            let ev = run(&mut w, &mut t, &fire_a, 1);
            if ev
                .iter()
                .any(|e| matches!(e, Event::Impact { target: Some(p), .. } if *p == b))
            {
                hit_vel = Some(w.ship(b).unwrap().vel.x);
                break;
            }
            // Keep b parked until it is hit.
            let s = w.ship_mut(b).unwrap();
            s.pos = Vec3::new(5.0, 0.0, 0.0);
            s.vel = Vec3::ZERO;
        }
        hit_vel.expect("laser hit the target")
    };
    let v1 = push(1.0);
    let v2 = push(2.0);
    assert!(v1 > 0.0, "{v1}");
    assert!((v2 / v1 - 2.0).abs() < 0.05, "{v1} {v2}");
}

#[test]
fn shift_ship_moves_the_ship_and_its_newest_trail_sample() {
    let mut t = std_room();
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 3);
    let before = w.trail_for_test(a, 1).unwrap().0;
    let newest = w.trail_for_test(a, 0).unwrap().0;
    let pos = w.ship(a).unwrap().pos;
    let d = Vec3::new(1.0, 2.0, 3.0);
    w.shift_ship(a, d);
    assert_eq!(w.ship(a).unwrap().pos, pos + d);
    assert_eq!(w.trail_for_test(a, 0).unwrap().0, newest + d);
    assert_eq!(
        w.trail_for_test(a, 1).unwrap().0,
        before,
        "older samples stay"
    );
}

#[test]
fn magnet_sneeze_pulls_a_weapon_pickup_off_its_spawn_point() {
    let mut t = std_room();
    let mut rules = RuleSet::paint_the_void();
    rules.pickups.retain(|k, _| k == "bolt_rocket");
    rules.upgrade_pickups.clear();
    rules.gadget_pickups.clear();
    rules.scrap_pickup = None;
    let mut s = setup();
    let spawn = Vec3::new(0.0, 0.0, -15.0);
    s.pickup_points = vec![spawn];
    let mut w = world_with(rules, s);
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let sneeze = w.weapons().find("magnet_sneeze").unwrap();
    {
        let sh = w.ship_mut(a).unwrap();
        sh.add_weapon(crate::WeaponState {
            weapon: sneeze,
            ammo: 5,
            level: 1,
            shots: 0,
        });
        sh.secondary = Some(sneeze);
    }
    let sneeze_input = [(a, InputCmd::idle(0).with_buttons(Buttons::FIRE_SECONDARY))];
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    run(&mut w, &mut t, &sneeze_input, 1);
    // 15 u away: it jumps `pickup_pull` (10 u) towards the shooter.
    let sp = &w.pickups()[0];
    assert_eq!(sp.pos, spawn, "the spawn point stays");
    assert!(
        (sp.content_pos().z + 5.0).abs() < 0.2,
        "{:?}",
        sp.content_pos()
    );
    assert!(w
        .entity_views()
        .iter()
        .any(|e| e.id == sp.id && (e.pos.z + 5.0).abs() < 0.2));
    // The next sneeze lands it in reach: collected.
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &sneeze_input, 120);
    assert!(ev.iter().any(|e| matches!(e, Event::PickupTaken { .. })));
    assert_eq!(w.pickups()[0].offset, Vec3::ZERO);
}
