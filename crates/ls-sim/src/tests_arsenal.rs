//! World-level tests of the M2 framework: ship types and ship choice,
//! gadgets (activation, drain, Shield interactions, Cloak relevance, Decoy),
//! charge mechanics, rule set v2 (tuning multipliers, shield overrides,
//! upgrade and gadget pickups, per-map caps, enforcement), debris and the
//! generic entity view.

use ls_math::terrain::{BoxRoomTerrain, SurfaceStates};
use ls_math::{Quat, Vec3};

use crate::*;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

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
    }
}

fn world_with(rules: RuleSet, setup: MatchSetup) -> World {
    World::new(WorldConfig {
        rules,
        ..WorldConfig::paint_the_void(11, setup)
    })
    .unwrap()
}

fn world() -> World {
    world_with(RuleSet::paint_the_void(), setup())
}

fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0))
}

fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
    s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
}

fn run(
    w: &mut World,
    t: &mut BoxRoomTerrain,
    inputs: &[(PlayerId, InputCmd)],
    n: u32,
) -> Vec<Event> {
    let mut ev = Vec::new();
    for _ in 0..n {
        ev.extend(w.step(t, inputs));
    }
    ev
}

fn press(id: PlayerId, b: Buttons) -> [(PlayerId, InputCmd); 1] {
    [(id, InputCmd::idle(0).with_buttons(b))]
}

fn select(w: &World, id: PlayerId, weapon: &str, buttons: Buttons) -> [(PlayerId, InputCmd); 1] {
    let wid = w.weapons().find(weapon).unwrap();
    [(
        id,
        InputCmd {
            weapon_select: wid.0 + 1,
            ..InputCmd::idle(0)
        }
        .with_buttons(buttons),
    )]
}

fn damage_to(ev: &[Event], who: PlayerId) -> f32 {
    ev.iter()
        .filter_map(|e| match e {
            Event::Damaged { target, amount, .. } if *target == who => Some(*amount),
            _ => None,
        })
        .sum()
}

fn fired(ev: &[Event]) -> usize {
    ev.iter()
        .filter(|e| matches!(e, Event::Fired { .. }))
        .count()
}

/// Gives weapon `id` to `p` directly (bypassing pickups) and selects it.
fn equip(w: &mut World, p: PlayerId, id: &str) {
    let wid = w.weapons().find(id).unwrap();
    let def = w.weapons().get(wid).clone();
    let s = w.ship_mut(p).unwrap();
    s.add_weapon(WeaponState {
        weapon: wid,
        ammo: def.max_ammo,
        level: 1,
        shots: 0,
    });
    match def.slot {
        WeaponSlot::Primary => s.primary = Some(wid),
        WeaponSlot::Secondary => s.secondary = Some(wid),
    }
}

// ----- ship types ------------------------------------------------------------------

#[test]
fn ship_types_spawn_with_their_numbers() {
    let mut w = world();
    let mut t = room();
    let sk = w.add_player(Some(0), ShipType::Skitter).unwrap();
    let ha = w.add_player(Some(1), ShipType::Hauler).unwrap();
    assert_eq!(w.ship(sk).unwrap().hull, 70.0);
    assert_eq!(w.ship(ha).unwrap().hull, 150.0);
    // Skitter carries one secondary type: a second one swaps the selected out.
    assert!(w.give_weapon(sk, "seeker").is_ok());
    let s = w.ship(sk).unwrap();
    let secondaries = s
        .weapons
        .iter()
        .filter(|x| w.weapons().get(x.weapon).slot == WeaponSlot::Secondary)
        .count();
    assert_eq!(secondaries, 1);
    // Hauler carries three.
    w.give_weapon(ha, "seeker").unwrap();
    w.give_weapon(ha, "prox_mine").unwrap();
    let s = w.ship(ha).unwrap();
    let secondaries = s
        .weapons
        .iter()
        .filter(|x| w.weapons().get(x.weapon).slot == WeaponSlot::Secondary)
        .count();
    assert_eq!(secondaries, 3);
    // Damage multiplier: the Hauler's laser hits for 1.2x, the Skitter's for 0.85x.
    place(&mut w, ha, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
    place(&mut w, sk, Vec3::ZERO, Vec3::Z);
    let mut ev = run(&mut w, &mut t, &press(ha, Buttons::FIRE_PRIMARY), 1);
    ev.extend(run(&mut w, &mut t, &[], 20));
    assert!(
        (damage_to(&ev, sk) - 6.0 * 1.2).abs() < 1e-4,
        "{}",
        damage_to(&ev, sk)
    );
    // Hitbox: a bolt passing 0.62 u from the centre still hits the Hauler
    // (0.65 u hull + 0.15 u bolt).
    place(&mut w, sk, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
    place(&mut w, ha, Vec3::new(0.62, 0.0, 0.0), Vec3::Z);
    run(&mut w, &mut t, &[], 20);
    let mut ev = run(&mut w, &mut t, &press(sk, Buttons::FIRE_PRIMARY), 1);
    ev.extend(run(&mut w, &mut t, &[], 20));
    assert!(damage_to(&ev, ha) > 0.0, "large hitbox");
}

#[test]
fn ship_choice_applies_at_the_next_respawn() {
    let mut rules = RuleSet::paint_the_void();
    rules.allowed_ship_types = vec![ShipType::Tinker, ShipType::Tug];
    let mut w = world_with(rules, setup());
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    assert!(matches!(
        w.set_ship_type(a, ShipType::Hauler),
        Err(SimError::ShipTypeNotAllowed(ShipType::Hauler))
    ));
    w.set_ship_type(a, ShipType::Tug).unwrap();
    assert_eq!(
        w.ship(a).unwrap().ship_type,
        ShipType::Tinker,
        "not mid-life"
    );
    w.ship_mut(a).unwrap().hull = 1.0;
    // Die by own overdrive... simplest: kill through a rocket at point blank.
    place(&mut w, a, Vec3::new(0.0, -13.5, 0.0), Vec3::NEG_Y);
    run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 5);
    assert!(!w.ship(a).unwrap().alive);
    run(&mut w, &mut t, &[], 200);
    let s = w.ship(a).unwrap();
    assert!(s.alive);
    assert_eq!(s.ship_type, ShipType::Tug);
    assert_eq!(s.hull, 110.0);
    assert_eq!(s.next_ship_type, None);
}

#[test]
fn tug_passes_scrap_to_a_teammate() {
    let mut w = world();
    let mut t = room();
    let tug = w.add_player(Some(0), ShipType::Tug).unwrap();
    let mate = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, tug, Vec3::ZERO, Vec3::NEG_Z);
    place(&mut w, mate, Vec3::new(2.0, 0.0, 0.0), Vec3::NEG_Z);
    w.ship_mut(tug).unwrap().scrap = 50.0;
    w.ship_mut(mate).unwrap().hull = 60.0;
    run(&mut w, &mut t, &press(tug, Buttons::SCAVENGE), 60);
    assert!((w.ship(tug).unwrap().scrap - 30.0).abs() < 1e-3);
    assert!(
        (w.ship(mate).unwrap().hull - 80.0).abs() < 1e-3,
        "hull first"
    );
    // A Tinker cannot pass scrap.
    w.ship_mut(tug).unwrap().scrap = 0.0;
    w.ship_mut(mate).unwrap().scrap = 50.0;
    run(&mut w, &mut t, &press(mate, Buttons::SCAVENGE), 60);
    assert_eq!(w.ship(mate).unwrap().scrap, 50.0);
}

// ----- gadgets ---------------------------------------------------------------------

#[test]
fn gadget_key_toggles_the_shield_and_it_drains_energy() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    w.ship_mut(a).unwrap().energy = 50.0;
    let ev = run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
    let s = w.ship(a).unwrap();
    assert!(s.gadget_active);
    assert!(ev
        .iter()
        .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Gadget)));
    // Holding G does not toggle again; the shield drains 4 energy/s (regen 10/s).
    let e0 = s.energy;
    run(&mut w, &mut t, &press(a, Buttons::GADGET), 60);
    let s = w.ship(a).unwrap();
    assert!(s.gadget_active);
    assert!(
        (s.energy - (e0 + 10.0 - 4.0)).abs() < 0.05,
        "{} {}",
        e0,
        s.energy
    );
    // Release and press again: off.
    run(&mut w, &mut t, &[], 1);
    run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
    assert!(!w.ship(a).unwrap().gadget_active);
    // Not enough energy: stays off.
    w.ship_mut(a).unwrap().energy = 1.0;
    run(&mut w, &mut t, &[], 60);
    w.ship_mut(a).unwrap().energy = 1.0;
    run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
    assert!(!w.ship(a).unwrap().gadget_active);
}

#[test]
fn shield_cuts_self_splash_and_uses_weapon_interactions() {
    // Own rocket at point blank: 75% less self-damage with the shield up.
    let self_hit = |shield: bool| {
        let mut w = world();
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        place(&mut w, a, Vec3::new(0.0, -13.5, 0.0), Vec3::NEG_Y);
        if shield {
            run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
            assert!(w.ship(a).unwrap().gadget_active);
            place(&mut w, a, Vec3::new(0.0, -13.5, 0.0), Vec3::NEG_Y);
        }
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 5);
        damage_to(&ev, a)
    };
    let (off, on) = (self_hit(false), self_hit(true));
    assert!(off > 0.0);
    assert!((on - off * 0.25).abs() < 1e-3, "{off} {on}");

    // Enemy laser against a shield: the rule set's override decides.
    let enemy_hit = |interaction: Option<ShieldInteraction>| {
        let mut rules = RuleSet::paint_the_void();
        if let Some(i) = interaction {
            rules.shield_overrides.insert("scrap_laser".into(), i);
        }
        let mut w = world_with(rules, setup());
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        run(&mut w, &mut t, &press(b, Buttons::GADGET), 1);
        place(&mut w, a, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let e0 = w.ship(b).unwrap().energy;
        let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
        ev.extend(run(&mut w, &mut t, &[], 20));
        (damage_to(&ev, b), e0 - w.ship(b).unwrap().energy)
    };
    // Default for the laser: Wears(1.0): no hull damage, energy pays.
    let (dmg, _) = enemy_hit(None);
    assert_eq!(dmg, 0.0);
    let (dmg, _) = enemy_hit(Some(ShieldInteraction::PassThrough(0.5)));
    assert!((dmg - 3.0).abs() < 1e-4, "{dmg}");
    let (dmg, _) = enemy_hit(Some(ShieldInteraction::Blocked));
    assert_eq!(dmg, 0.0);
}

#[test]
fn cloak_hides_from_distant_enemies_and_drains_three_times_while_moving() {
    let mut rules = RuleSet::paint_the_void();
    rules.starting_loadout.gadget = Some("cloak".into());
    rules
        .gadgets
        .insert("cloak".into(), Availability::LoadoutAndPickup);
    let mut w = world_with(rules, setup());
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let c = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    place(&mut w, b, Vec3::new(20.0, 0.0, 0.0), Vec3::NEG_Z);
    place(&mut w, c, Vec3::new(-20.0, 0.0, 0.0), Vec3::NEG_Z);
    assert!(w.ship_visible_to(b, a));
    run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
    assert!(!w.ship_visible_to(b, a), "hidden from the enemy");
    assert!(w.ship_visible_to(c, a), "teammates see it");
    assert!(w.ship_visible_to(a, a));
    // Firing reveals for 0.5 s.
    run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
    assert!(w.ship_visible_to(b, a));
    run(&mut w, &mut t, &[], 31);
    assert!(!w.ship_visible_to(b, a));
    // Drain: 3/s at rest, 9/s while moving (regen 10/s while not boosting).
    w.ship_mut(a).unwrap().energy = 50.0;
    run(&mut w, &mut t, &[], 60);
    let rest = w.ship(a).unwrap().energy - 50.0;
    w.ship_mut(a).unwrap().energy = 50.0;
    run(
        &mut w,
        &mut t,
        &[(a, InputCmd::idle(0).with_thrust(Vec3::X))],
        60,
    );
    let moving = w.ship(a).unwrap().energy - 50.0;
    assert!((rest - 7.0).abs() < 0.1, "{rest}");
    assert!(moving < 2.0 && moving > 0.0, "{moving}");
}

#[test]
fn decoy_launches_draws_fire_and_expires() {
    let mut rules = RuleSet::paint_the_void();
    rules.starting_loadout.gadget = Some("decoy".into());
    rules
        .gadgets
        .insert("decoy".into(), Availability::LoadoutAndPickup);
    let mut w = world_with(rules, setup());
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::new(0.0, 0.0, -10.0), Vec3::Z);
    let e0 = w.ship(a).unwrap().energy;
    run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
    assert!(w.ship(a).unwrap().energy < e0 - 29.0, "costs 30 energy");
    let decoy = w
        .entities()
        .iter()
        .find(|e| e.kind == EntityKind::Decoy)
        .expect("decoy launched")
        .clone();
    assert_eq!(decoy.hp, 100.0, "integrity = owner's max hull");
    assert_eq!(decoy.owner, Some(a));
    // Cooldown: pressing again right away does nothing.
    run(&mut w, &mut t, &[], 1);
    run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
    assert_eq!(
        w.entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Decoy)
            .count(),
        1
    );
    // b shoots it: projectiles hit the decoy and wear down its integrity.
    let d = w
        .entities()
        .iter()
        .find(|e| e.kind == EntityKind::Decoy)
        .unwrap()
        .pos;
    let dir = d - w.ship(b).unwrap().pos;
    let pos_b = w.ship(b).unwrap().pos;
    place(&mut w, b, pos_b, dir);
    run(&mut w, &mut t, &press(b, Buttons::FIRE_PRIMARY), 1);
    run(&mut w, &mut t, &[], 30);
    let hp = w
        .entities()
        .iter()
        .find(|e| e.kind == EntityKind::Decoy)
        .map(|e| e.hp);
    assert!(hp.is_some_and(|h| h < 100.0), "{hp:?}");
    // After six seconds it rubber-bands back and hits the first ship on its
    // path, then it is gone.
    run(&mut w, &mut t, &[], 450);
    assert!(!w.entities().iter().any(|e| e.kind == EntityKind::Decoy));
}

// ----- charge ----------------------------------------------------------------------

#[test]
fn fusion_charge_scales_damage_and_pays_on_release() {
    let hit = |hold: u32| {
        let mut w = world();
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Hauler).unwrap();
        equip(&mut w, a, "fusion_charger");
        place(&mut w, a, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let e0 = w.ship(a).unwrap().energy;
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), hold);
        assert_eq!(fired(&ev), 0, "nothing fires while charging");
        assert!(ev
            .iter()
            .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::ChargeStart)));
        assert!(w.ship(a).unwrap().energy >= e0, "not paid yet");
        let mut ev = run(&mut w, &mut t, &[], 1);
        assert_eq!(fired(&ev), 1, "fires on release");
        ev.extend(run(&mut w, &mut t, &[], 30));
        damage_to(&ev, b)
    };
    let quick = hit(1);
    let half = hit(60);
    let full = hit(120);
    let over = hit(200);
    assert!((quick - 12.0 * (1.0 + 3.0 / 120.0)).abs() < 0.01, "{quick}");
    assert!((half - 12.0 * 2.5).abs() < 0.01, "{half}");
    assert!((full - 12.0 * 4.0).abs() < 0.01, "{full}");
    assert_eq!(over, full, "capped at full charge");
}

#[test]
fn polite_laser_auto_releases_after_its_charge_and_cancels_early() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    equip(&mut w, a, "polite_laser");
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    // Released after 1 s (< 1.5 s minimum): cancelled, nothing paid.
    let e0 = w.ship(a).unwrap().energy;
    let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 60);
    ev.extend(run(&mut w, &mut t, &[], 1));
    assert_eq!(fired(&ev), 0);
    assert!(w.ship(a).unwrap().energy >= e0);
    // Held: fires by itself at 1.5 s.
    let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 89);
    assert_eq!(fired(&ev), 0);
    let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 2);
    assert_eq!(fired(&ev), 1);
    // Switching weapons drops a charge.
    run(&mut w, &mut t, &[], 150);
    run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 30);
    assert!(w.ship(a).unwrap().charge.is_some());
    let sel = select(&w, a, "scrap_laser", Buttons::NONE);
    run(&mut w, &mut t, &sel, 1);
    assert!(w.ship(a).unwrap().charge.is_none());
}

// ----- rule set v2 -----------------------------------------------------------------

#[test]
fn tuning_multipliers_change_damage_rate_and_ammo() {
    let mut rules = RuleSet::paint_the_void();
    rules.tuning.insert(
        "scrap_laser".into(),
        WeaponTuning {
            damage: 2.0,
            fire_rate: 2.0,
            ammo: 1.0,
        },
    );
    rules.tuning.insert(
        "bolt_rocket".into(),
        WeaponTuning {
            ammo: 2.0,
            ..WeaponTuning::default()
        },
    );
    let mut w = world_with(rules, setup());
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    let rocket = w.weapons().find("bolt_rocket").unwrap();
    assert_eq!(w.ship(a).unwrap().weapon(rocket).unwrap().ammo, 6, "3 x 2");
    place(&mut w, a, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::ZERO, Vec3::Z);
    let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
    ev.extend(run(&mut w, &mut t, &[], 20));
    assert!((damage_to(&ev, b) - 12.0).abs() < 1e-4);
    // Fire interval 0.25 s / 2 = 8 ticks: 8 shots in 60 ticks instead of 4.
    let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 60);
    assert!(fired(&ev) >= 7, "{}", fired(&ev));
}

#[test]
fn upgrade_and_gadget_pickups() {
    let mut rules = RuleSet::paint_the_void();
    rules.pickups.clear();
    rules.gadget_pickups.clear();
    rules.scrap_pickup = None;
    let mut s = setup();
    s.pickup_points = vec![Vec3::new(0.0, 0.0, -1.0)];
    let mut w = world_with(rules.clone(), s.clone());
    let mut t = room();
    let laser = w.weapons().find("scrap_laser").unwrap();
    assert!(matches!(
        w.pickups()[0].content,
        Some((_, PickupKind::Upgrade { weapon })) if weapon == laser
    ));
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    run(&mut w, &mut t, &[], 1);
    assert_eq!(w.ship(a).unwrap().weapon(laser).unwrap().level, 2);
    assert!(w.pickups()[0].content.is_none());

    // Gadget pickup replaces the loadout shield.
    rules.upgrade_pickups.clear();
    rules.gadget_pickups.insert(
        "cloak".into(),
        PickupRule {
            weight: 1.0,
            respawn_secs: 10.0,
            ammo: 0,
            max_on_map: 0,
        },
    );
    let mut w = world_with(rules, s);
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    let ev = run(&mut w, &mut t, &[], 1);
    assert!(ev.iter().any(|e| matches!(e, Event::PickupTaken { .. })));
    assert_eq!(w.ship(a).unwrap().gadget, w.gadgets().find("cloak"));
}

#[test]
fn per_map_cap_limits_tectonic_charges() {
    let mut rules = RuleSet::paint_the_void();
    rules
        .pickups
        .retain(|k, _| k == "tectonic_charge" || k == "rivet_gun");
    rules.upgrade_pickups.clear();
    rules.gadget_pickups.clear();
    rules.scrap_pickup = None;
    rules.pickups.get_mut("tectonic_charge").unwrap().weight = 100.0;
    let mut s = setup();
    s.pickup_points = (0..12)
        .map(|i| Vec3::new(-11.0 + 2.0 * i as f32, 10.0, 10.0))
        .collect();
    let w = world_with(rules, s);
    let tectonic = w.weapons().find("tectonic_charge").unwrap();
    let count = w
        .pickups()
        .iter()
        .filter(|p| matches!(p.content, Some((_, PickupKind::Weapon { weapon, .. })) if weapon == tectonic))
        .count();
    assert_eq!(count, 1, "one Tectonic Charge per map");
    assert!(w.pickups().iter().all(|p| p.content.is_some()));
}

/// Enforcement over a long scripted match: banned weapons never spawn as
/// pickups (weapon or upgrade), are never given and never fire.
#[test]
fn banned_weapons_never_spawn_or_fire() {
    let mut rules = RuleSet::paint_the_void();
    for id in ["rivet_gun", "seeker", "tectonic_charge", "scrap_laser"] {
        rules.weapons.insert(id.into(), Availability::Banned);
        rules.pickups.remove(id);
        rules.upgrade_pickups.remove(id);
    }
    rules
        .starting_loadout
        .primaries
        .retain(|p| p != "scrap_laser");
    rules.resolve(&WeaponTable::builtin()).unwrap();
    let mut s = setup();
    s.pickup_points = (0..16)
        .map(|i| Vec3::new(-15.0 + 2.0 * i as f32, (i % 3) as f32 * 3.0 - 3.0, 5.0))
        .collect();
    let mut w = world_with(rules, s);
    let mut t = room();
    let banned: Vec<WeaponId> = ["rivet_gun", "seeker", "tectonic_charge", "scrap_laser"]
        .iter()
        .map(|id| w.weapons().find(id).unwrap())
        .collect();
    let ids: Vec<PlayerId> = (0..4)
        .map(|i| {
            w.add_player(Some(i % 2), ShipType::ALL[i as usize])
                .unwrap()
        })
        .collect();
    for id in ["rivet_gun", "seeker"] {
        assert!(matches!(
            w.give_weapon(ids[0], id),
            Err(SimError::WeaponBanned(_))
        ));
    }
    // A banned weapon smuggled into the inventory still never fires.
    {
        let s = w.ship_mut(ids[1]).unwrap();
        s.add_weapon(WeaponState {
            weapon: banned[0],
            ammo: 0,
            level: 1,
            shots: 0,
        });
        s.primary = Some(banned[0]);
        s.scrap = 100.0;
    }
    let mut rng = ls_math::Pcg32::new(3, 3);
    for tick in 0..3000u64 {
        let inputs: Vec<(PlayerId, InputCmd)> = ids
            .iter()
            .map(|&id| {
                let mut b = Buttons::NONE;
                b.set(Buttons::FIRE_PRIMARY, rng.chance(0.5));
                b.set(Buttons::FIRE_SECONDARY, rng.chance(0.2));
                let axis = |r: &mut ls_math::Pcg32| r.below(3) as f32 - 1.0;
                let thrust = Vec3::new(axis(&mut rng), axis(&mut rng), axis(&mut rng));
                (
                    id,
                    InputCmd {
                        tick,
                        thrust,
                        look_yaw: rng.range_f32(-0.05, 0.05),
                        buttons: b,
                        weapon_select: if rng.chance(0.05) {
                            rng.below(28) as u8
                        } else {
                            0
                        },
                        ..InputCmd::default()
                    },
                )
            })
            .collect();
        for e in w.step(&mut t, &inputs) {
            match e {
                Event::Fired { weapon, .. } => {
                    assert!(!banned.contains(&weapon), "{weapon:?} fired")
                }
                Event::PickupSpawned {
                    kind: PickupKind::Weapon { weapon, .. } | PickupKind::Upgrade { weapon },
                    ..
                } => assert!(!banned.contains(&weapon), "{weapon:?} spawned"),
                _ => {}
            }
        }
        for s in w.ships() {
            for x in &s.weapons {
                if banned.contains(&x.weapon) {
                    assert_eq!(s.player, ids[1], "only the smuggled one");
                }
            }
        }
    }
}

// ----- debris and entities -----------------------------------------------------------

#[test]
fn debris_is_scavengeable_pushable_capped_and_blocks_shots() {
    let mut w = world();
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
    place(&mut w, b, Vec3::new(0.0, 0.0, -10.0), Vec3::Z);
    // Blocks shots: a chunk between a and b takes the hit.
    let chunk = w.spawn_entity(EntitySpawn::debris(
        Vec3::new(0.0, 0.0, -5.0),
        Vec3::ZERO,
        1.0,
        10.0,
    ));
    let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
    ev.extend(run(&mut w, &mut t, &[], 30));
    assert_eq!(damage_to(&ev, b), 0.0, "the chunk blocked the bolt");
    let e = w.entities().iter().find(|e| e.id == chunk).unwrap();
    assert!(e.hp < 20.0, "took the damage");
    assert!(e.vel.z < 0.0 || e.pos.z < -5.0, "pushed by the hit");
    // Pushable: ramming it moves it.
    let chunk2 = w.spawn_entity(EntitySpawn::debris(
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::ZERO,
        0.5,
        5.0,
    ));
    place(&mut w, a, Vec3::new(8.0, 0.0, 0.0), Vec3::NEG_Z);
    w.ship_mut(a).unwrap().vel = Vec3::new(12.0, 0.0, 0.0);
    run(
        &mut w,
        &mut t,
        &[(a, InputCmd::idle(0).with_thrust(Vec3::X))],
        20,
    );
    let e = w.entities().iter().find(|e| e.id == chunk2).unwrap();
    assert!(e.pos.x > 10.5, "{:?}", e.pos);
    // Scavengeable: its scrap repairs hull; emptied debris disappears.
    let pos = e.pos;
    place(&mut w, a, pos + Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Z);
    w.ship_mut(a).unwrap().hull = 50.0;
    for _ in 0..60 {
        w.step(&mut t, &press(a, Buttons::SCAVENGE));
        let p = w.entities().iter().find(|e| e.id == chunk2).map(|e| e.pos);
        if let Some(p) = p {
            place(&mut w, a, p + Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Z);
        }
    }
    assert!((w.ship(a).unwrap().hull - 55.0).abs() < 1e-3);
    assert!(!w.entities().iter().any(|e| e.id == chunk2));
    // Cap: at most 64 pieces, the oldest go first.
    for i in 0..80 {
        w.spawn_entity(EntitySpawn::debris(
            Vec3::new(
                -20.0 + 0.5 * (i % 40) as f32,
                10.0,
                20.0 - 2.0 * (i / 40) as f32,
            ),
            Vec3::ZERO,
            0.3,
            0.0,
        ));
    }
    let debris = w
        .entities()
        .iter()
        .filter(|e| e.kind == EntityKind::Debris)
        .count();
    assert_eq!(debris, entity::MAX_DEBRIS);
    assert!(
        !w.entities().iter().any(|e| e.id == chunk),
        "oldest removed"
    );
    // Explosions push debris outward.
    let e0 = w
        .entities()
        .iter()
        .find(|e| e.kind == EntityKind::Debris)
        .unwrap()
        .clone();
    place(&mut w, a, e0.pos + Vec3::new(0.0, 0.0, 6.0), Vec3::NEG_Z);
    run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
    run(&mut w, &mut t, &[], 20);
    let moved = w
        .entities()
        .iter()
        .filter(|e| {
            e.kind == EntityKind::Debris
                && (e.vel != Vec3::ZERO || e.id == e0.id && e.pos != e0.pos)
        })
        .count();
    assert!(moved > 0);
}

#[test]
fn entity_views_are_unique_and_typed() {
    let mut s = setup();
    s.pickup_points = (0..10)
        .map(|i| Vec3::new(-10.0 + 2.0 * i as f32, 5.0, 5.0))
        .collect();
    let mut w = world_with(RuleSet::paint_the_void(), s);
    let mut t = room();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
    w.add_scrap_pile(Vec3::new(1.0, 1.0, 1.0), 12.0);
    w.spawn_entity(EntitySpawn::debris(Vec3::ZERO, Vec3::ZERO, 0.5, 3.0));
    w.spawn_entity(EntitySpawn::dust(Vec3::ZERO, 5.0, 100));
    // A kill leaves a wreck.
    w.ship_mut(b).unwrap().hull = 1.0;
    place(&mut w, a, Vec3::new(0.0, -8.0, 8.0), Vec3::NEG_Z);
    place(&mut w, b, Vec3::new(0.0, -8.0, 0.0), Vec3::Z);
    run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 20);
    let views = w.entity_views();
    let mut ids: Vec<u16> = views.iter().map(|v| v.wire_id()).collect();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "wire ids unique and sorted");
    let has = |k: ReplicatedKind| views.iter().any(|v| v.kind == k);
    assert!(has(ReplicatedKind::ScrapPile));
    assert!(has(ReplicatedKind::Wreck));
    assert!(has(ReplicatedKind::Debris));
    assert!(has(ReplicatedKind::DustCloud));
    assert!(views.iter().any(|v| matches!(
        v.kind,
        ReplicatedKind::WeaponPickup | ReplicatedKind::UpgradePickup | ReplicatedKind::GadgetPickup
    )));
    for v in &views {
        assert!(w.visuals().get(v.visual).is_some(), "{v:?}");
    }
    let pile = views
        .iter()
        .find(|v| v.kind == ReplicatedKind::ScrapPile && v.pos == Vec3::new(1.0, 1.0, 1.0))
        .unwrap();
    assert_eq!(pile.state[0], 12);
}

#[test]
fn entity_ids_never_collide_on_the_wire() {
    let mut w = world();
    // Keep a long-lived pile, then churn through more than 32768 ids.
    let keep = w.add_scrap_pile(Vec3::ZERO, 5.0);
    for i in 0..40_000u32 {
        let id = w.add_scrap_pile(Vec3::ZERO, 1.0);
        assert_ne!(
            id & entity::WIRE_ID_MASK,
            keep & entity::WIRE_ID_MASK,
            "{i}"
        );
        // Drop it again (as if scavenged).
        w.remove_last_scrap_for_test();
    }
}

/// A room that remembers scorch marks (surface edits with the scorch bit).
struct ScorchRoom {
    room: BoxRoomTerrain,
    marks: Vec<(Vec3, f32)>,
}

impl ls_math::terrain::TerrainQuery for ScorchRoom {
    fn sdf(&self, p: Vec3) -> f32 {
        self.room.sdf(p)
    }
    fn bounds(&self) -> ls_math::Aabb {
        self.room.bounds()
    }
}

impl SimTerrain for ScorchRoom {
    fn apply_edit(&mut self, edit: &ls_math::terrain::TerrainEdit) {
        if let ls_math::terrain::TerrainEdit::Surface {
            center,
            radius,
            states,
            until,
        } = *edit
        {
            if states & SurfaceStates::SCORCH.bits() != 0 && until != 0 {
                self.marks.push((center, radius));
            }
        }
    }
    fn surface_states_near(&self, center: Vec3, radius: f32, _now: u64) -> SurfaceStates {
        if self
            .marks
            .iter()
            .any(|&(c, r)| c.distance(center) <= r + radius)
        {
            SurfaceStates::SCORCH
        } else {
            SurfaceStates::NONE
        }
    }
}

/// `Ctx::scorch` marks the surface for good (hot for a while too) and skips
/// spots already scorched; `Ctx::set_weapon_state` only touches carried
/// weapons, and dropping a weapon drops its state.
#[test]
fn scorch_helper_and_weapon_state_commands() {
    use ls_math::terrain::TerrainEdit;
    let mut w = world();
    let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
    let mut t = ScorchRoom {
        room: room(),
        marks: Vec::new(),
    };
    let wall = Vec3::new(-30.0, 0.0, 0.0);
    let now = w.tick();
    let (_, ev) = w.hook_for_test(&mut t, |ctx| ctx.scorch(wall, 1.0, 10.0));
    let edits: Vec<TerrainEdit> = ev
        .iter()
        .filter_map(|e| match e {
            Event::TerrainEdit(e) => Some(*e),
            _ => None,
        })
        .collect();
    assert_eq!(edits, [TerrainEdit::scorch(wall, 1.0, now, 600)]);
    let (_, ev) = w.hook_for_test(&mut t, |ctx| {
        ctx.scorch(wall + Vec3::Y * 0.3, 1.0, 10.0);
        ctx.scorch(wall + Vec3::Y * 5.0, 0.5, 0.0);
    });
    let edits: Vec<&Event> = ev
        .iter()
        .filter(|e| matches!(e, Event::TerrainEdit(_)))
        .collect();
    assert_eq!(edits.len(), 1, "the scorched spot is skipped");
    assert!(matches!(
        edits[0],
        Event::TerrainEdit(TerrainEdit::Surface {
            states: 8,
            until: u32::MAX,
            ..
        })
    ));

    let laser = w.weapons().find("scrap_laser").unwrap();
    let rocket = w.weapons().find("heavy_rocket").unwrap();
    assert!(w.ship(a).unwrap().weapon(laser).is_some());
    assert!(w.ship(a).unwrap().weapon(rocket).is_none());
    w.hook_for_test(&mut t, |ctx| {
        ctx.set_weapon_state(a, laser, 7);
        ctx.set_weapon_state(a, rocket, 9);
    });
    let s = w.ship(a).unwrap();
    assert_eq!(s.weapon_state_of(laser), 7);
    assert_eq!(s.weapon_state_of(rocket), 0, "not carried");
    let h = w.state_hash();
    w.hook_for_test(&mut t, |ctx| ctx.set_weapon_state(a, laser, 0));
    assert!(w.ship(a).unwrap().weapon_state.is_empty());
    assert_ne!(w.state_hash(), h, "weapon state is hashed");
    w.hook_for_test(&mut t, |ctx| ctx.set_weapon_state(a, laser, 3));
    w.ship_mut(a).unwrap().remove_weapon(laser);
    assert!(w.ship(a).unwrap().weapon_state.is_empty());
}
