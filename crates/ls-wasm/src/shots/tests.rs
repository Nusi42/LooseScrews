use super::*;
use ls_math::terrain::BoxRoomTerrain;
use ls_math::Quat;
use ls_sim::{Event, MatchSetup, PlayerId, RuleSet, ShipType, SpawnPoint, World, WorldConfig};

fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(200.0, 100.0, 200.0))
}

fn world() -> (World, PlayerId) {
    let setup = MatchSetup {
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
    };
    let mut w = World::new(WorldConfig::paint_the_void(42, setup)).unwrap();
    let me = w.add_player(Some(0), ShipType::Tinker).unwrap();
    (w, me)
}

struct Tables {
    weapons: WeaponTable,
    rules: ResolvedRules,
    tuning: ShipTuning,
}

impl Tables {
    fn new() -> Self {
        let weapons = WeaponTable::builtin();
        let rules = RuleSet::paint_the_void().resolve(&weapons).unwrap();
        Tables {
            weapons,
            rules,
            tuning: ShipTuning::tinker(),
        }
    }
    fn fire(&self) -> FireRules<'_> {
        FireRules {
            weapons: &self.weapons,
            rules: &self.rules,
            tuning: &self.tuning,
        }
    }
    fn id(&self, name: &str) -> WeaponId {
        self.weapons.find(name).unwrap()
    }
}

fn resources(s: &ShipState, secondary: WeaponId) -> Resources {
    Resources {
        hull: s.hull,
        energy: s.energy,
        scrap: s.scrap,
        count: s.weapon(secondary).map_or(0, |w| w.ammo),
    }
}

fn net(p: &ls_sim::Projectile) -> NetProjectile {
    NetProjectile {
        state: 0,
        id: p.id as u16,
        kind: p.weapon.0,
        owner: 0,
        pos: p.pos,
        vel: p.vel,
    }
}

fn cmd(buttons: u16, select: u8) -> InputCmd {
    InputCmd {
        tick: 0,
        thrust: Vec3::new(0.3, 0.2, -1.0),
        look_pitch: 0.004,
        look_yaw: -0.006,
        roll: 0.002,
        buttons: Buttons(buttons),
        weapon_select: select,
    }
}

/// The predictor, fed the exact ship state of a real `World` each tick,
/// spawns exactly the shots the simulation fires (same ticks, muzzle
/// positions and velocities) through weapon switches, cooldowns and running
/// out of rockets, and every one is confirmed by the "snapshot".
#[test]
fn predicted_shots_match_the_simulation() {
    let t = Tables::new();
    let (mut w, me) = world();
    let mut terrain = room();
    let rocket = t.id("bolt_rocket");
    let paint = t.id("paint_thrower");
    let laser = t.id("scrap_laser");
    let mut p = ShotPredictor::new(&t.rules);
    let (fire1, fire2) = (Buttons::FIRE_PRIMARY.0, Buttons::FIRE_SECONDARY.0);
    let mut fired_total = 0;
    for i in 0..400u64 {
        let select = match i {
            0..=99 => 0,
            100..=199 => paint.0 + 1,
            _ => laser.0 + 1,
        };
        let buttons = fire1 | if i % 40 < 20 { fire2 } else { 0 };
        let c = InputCmd {
            tick: w.tick() + 1,
            ..cmd(buttons, select)
        };
        // Snapshot of the state before this tick.
        let before = w.ship(me).unwrap().clone();
        p.on_snapshot_state(w.tick(), before.primary, resources(&before, rocket));
        let events = w.step(&mut terrain, &[(me, c)]);
        let tick = w.tick();
        let fired: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                Event::Fired { projectile, .. } => Some(*projectile),
                _ => None,
            })
            .collect();
        let ship = w.ship(me).unwrap().clone();
        let n = p.on_tick(tick, &ship, &c, t.fire(), &terrain);
        assert_eq!(
            n,
            fired.len(),
            "tick {tick}: predicted {n}, fired {fired:?}"
        );
        fired_total += n;
        for id in &fired {
            let real = w.projectiles().iter().find(|x| x.id == *id).unwrap();
            let pred = p
                .pending()
                .iter()
                .find(|s| s.tick == tick && s.weapon == real.weapon)
                .expect("a prediction for the shot");
            assert!(pred.pos_at(tick as f64).distance(real.pos) < 1e-3);
            assert!(pred.vel.distance(real.vel) < 1e-3);
        }
        let own: Vec<NetProjectile> = w.projectiles().iter().map(net).collect();
        p.reconcile(tick, &own);
        assert!(p.pending().is_empty(), "tick {tick}: every shot confirmed");
        for x in &own {
            assert!(p.offset(x.id).length() < 1e-3);
        }
    }
    let s = p.stats();
    assert_eq!(s.predicted as usize, fired_total);
    assert_eq!(s.confirmed, s.predicted);
    assert_eq!((s.dropped, s.unpredicted), (0, 0));
    // Both primaries fired and all three rockets went out.
    assert!(fired_total > 20, "{fired_total}");
    assert_eq!(w.ship(me).unwrap().weapon(rocket).unwrap().ammo, 0);
}

fn alive_ship(t: &Tables) -> ShipState {
    ShipState::new(
        PlayerId(0),
        0,
        ShipType::Tinker,
        &t.tuning,
        Vec3::ZERO,
        Quat::IDENTITY,
    )
}

#[test]
fn cooldown_and_costs_gate_prediction_without_server_updates() {
    let t = Tables::new();
    let terrain = room();
    let ship = alive_ship(&t);
    let laser = t.weapons.get(t.id("scrap_laser"));
    let interval = laser.fire_interval_ticks();
    let WeaponCost::Energy(cost) = laser.cost else {
        panic!("laser costs energy")
    };

    // Plenty of energy: one shot per fire interval.
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(
        0,
        Some(t.id("scrap_laser")),
        Resources {
            hull: 100.0,
            energy: 1000.0,
            scrap: 0.0,
            count: 3,
        },
    );
    let hold = cmd(Buttons::FIRE_PRIMARY.0, 0);
    let ticks: Vec<u64> = (1..=4 * interval)
        .filter(|&k| p.on_tick(k, &ship, &hold, t.fire(), &terrain) > 0)
        .collect();
    assert_eq!(
        ticks,
        vec![1, 1 + interval, 1 + 2 * interval, 1 + 3 * interval]
    );

    // Energy for one and a half shots: one shot, unless overdrive pays with hull.
    let low = Resources {
        hull: 100.0,
        energy: cost * 1.5,
        scrap: 0.0,
        count: 0,
    };
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(0, Some(t.id("scrap_laser")), low);
    let n: usize = (1..=10 * interval)
        .map(|k| p.on_tick(k, &ship, &hold, t.fire(), &terrain))
        .sum();
    assert_eq!(n, 1);
    let over = cmd(Buttons::FIRE_PRIMARY.0 | Buttons::OVERDRIVE.0, 0);
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(0, Some(t.id("scrap_laser")), low);
    let n: usize = (1..=3 * interval)
        .map(|k| p.on_tick(k, &ship, &over, t.fire(), &terrain))
        .sum();
    assert_eq!(n, 3, "overdrive keeps firing while the hull lasts");
    // Hull too low for the overdrive price.
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(
        0,
        Some(t.id("scrap_laser")),
        Resources {
            hull: 1.5,
            energy: 0.0,
            ..low
        },
    );
    assert_eq!(p.on_tick(1, &ship, &over, t.fire(), &terrain), 0);

    // Rockets: the count runs out.
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(0, None, Resources { count: 2, ..low });
    let rockets = cmd(Buttons::FIRE_SECONDARY.0, 0);
    let n: usize = (1..=600)
        .map(|k| p.on_tick(k, &ship, &rockets, t.fire(), &terrain))
        .sum();
    assert_eq!(n, 2);
}

#[test]
fn selection_bans_and_death_are_respected() {
    let t = Tables::new();
    let terrain = room();
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(
        0,
        None,
        Resources {
            hull: 100.0,
            energy: 100.0,
            scrap: 100.0,
            count: 3,
        },
    );
    let mut ship = alive_ship(&t);
    // The rivet gun is pickup-only: not carried, so selecting it keeps the
    // loadout laser.
    let rivet = t.id("rivet_gun");
    let c = cmd(Buttons::FIRE_PRIMARY.0, rivet.0 + 1);
    assert_eq!(p.on_tick(1, &ship, &c, t.fire(), &terrain), 1);
    assert_eq!(p.pending()[0].weapon, t.id("scrap_laser"));
    // Once a snapshot shows it as the primary (picked up), it fires.
    p.on_snapshot_state(
        1,
        Some(rivet),
        Resources {
            hull: 100.0,
            energy: 100.0,
            scrap: 100.0,
            count: 3,
        },
    );
    assert_eq!(p.on_tick(100, &ship, &c, t.fire(), &terrain), 1);
    assert_eq!(p.pending().last().unwrap().weapon, rivet);
    // Dead ships do not fire.
    ship.alive = false;
    assert_eq!(p.on_tick(200, &ship, &c, t.fire(), &terrain), 0);

    // A banned weapon is never predicted, even if a snapshot claims it.
    let mut rules = t.rules.clone();
    rules.availability[rivet.0 as usize] = ls_sim::Availability::Banned;
    let banned = FireRules {
        rules: &rules,
        ..t.fire()
    };
    let mut p = ShotPredictor::new(&rules);
    p.on_snapshot_state(
        0,
        Some(rivet),
        Resources {
            hull: 100.0,
            energy: 100.0,
            scrap: 100.0,
            count: 0,
        },
    );
    assert_eq!(
        p.on_tick(1, &alive_ship(&t), &c, banned, &terrain),
        0,
        "banned"
    );
}

#[test]
fn reconcile_offsets_fade_and_unconfirmed_shots_are_dropped() {
    let t = Tables::new();
    let terrain = room();
    let ship = alive_ship(&t);
    let laser = t.id("scrap_laser");
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(
        0,
        Some(laser),
        Resources {
            hull: 100.0,
            energy: 100.0,
            scrap: 0.0,
            count: 0,
        },
    );
    let hold = cmd(Buttons::FIRE_PRIMARY.0, 0);
    assert_eq!(p.on_tick(10, &ship, &hold, t.fire(), &terrain), 1);
    let shot = p.pending()[0];

    // The server's projectile is 0.3 u off (quantization, a slightly
    // different aim): matched, the difference becomes a fading offset.
    let err = Vec3::new(0.3, 0.0, 0.0);
    let server = NetProjectile {
        state: 0,
        id: 7,
        kind: laser.0,
        owner: 0,
        pos: shot.pos_at(11.0) + err,
        vel: shot.vel,
    };
    p.reconcile(11, &[server]);
    assert!(p.pending().is_empty());
    assert!(p.offset(7).distance(-err) < 1e-4);
    p.decay(10);
    let faded = p.offset(7).length();
    assert!(faded < 0.3 * 0.25 && faded > 0.0, "{faded}");
    p.decay(200);
    assert_eq!(p.offset(7), Vec3::ZERO);

    // A projectile nobody predicted (e.g. fired before sync) is shown too.
    let other = NetProjectile { id: 8, ..server };
    p.reconcile(12, &[server, other]);
    assert_eq!(p.stats().unpredicted, 1);
    // Gone from the snapshot: forgotten.
    p.reconcile(14, &[]);
    assert_eq!(p.offset(7), Vec3::ZERO);

    // A shot the server never shows disappears after the grace period, not
    // before; a far-away projectile of the same weapon does not claim it.
    let later = 40;
    assert_eq!(p.on_tick(later, &ship, &hold, t.fire(), &terrain), 1);
    let far = NetProjectile {
        state: 0,
        id: 9,
        pos: Vec3::new(50.0, 0.0, 0.0),
        ..server
    };
    p.reconcile(later + CONFIRM_GRACE_TICKS - 1, &[far]);
    assert_eq!(p.pending().len(), 1);
    p.reconcile(later + CONFIRM_GRACE_TICKS, &[far]);
    assert!(p.pending().is_empty());
    let s = p.stats();
    assert_eq!((s.predicted, s.confirmed, s.dropped), (2, 1, 1));
}

#[test]
fn shots_are_hidden_past_the_first_wall_and_after_their_lifetime() {
    let t = Tables::new();
    // Wall 10 u ahead (-Z) of the ship.
    let terrain = BoxRoomTerrain::new(ls_math::Aabb::new(
        Vec3::new(-50.0, -50.0, -10.0),
        Vec3::new(50.0, 50.0, 50.0),
    ));
    let ship = alive_ship(&t);
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(
        0,
        Some(t.id("scrap_laser")),
        Resources {
            hull: 100.0,
            energy: 100.0,
            scrap: 0.0,
            count: 0,
        },
    );
    p.on_tick(
        1,
        &ship,
        &cmd(Buttons::FIRE_PRIMARY.0, 0),
        t.fire(),
        &terrain,
    );
    let s = p.pending()[0];
    assert!(s.reach < 10.0 && s.reach > 8.0, "{}", s.reach);
    assert!(s.visible_at(1.0));
    // 30 u/s: the wall is ~9 u away, reached after ~18 ticks.
    assert!(s.visible_at(15.0));
    assert!(!s.visible_at(25.0));

    // In open space the lifetime ends it.
    let open = room();
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(
        0,
        Some(t.id("scrap_laser")),
        Resources {
            hull: 100.0,
            energy: 100.0,
            scrap: 0.0,
            count: 0,
        },
    );
    p.on_tick(1, &ship, &cmd(Buttons::FIRE_PRIMARY.0, 0), t.fire(), &open);
    let s = p.pending()[0];
    assert!(s.reach.is_infinite());
    assert!(s.visible_at(1.0 + s.lifetime_ticks as f64 - 1.0));
    assert!(!s.visible_at(1.0 + s.lifetime_ticks as f64));
    p.clear();
    assert!(p.pending().is_empty());
}

/// Hull burned by predicted overdrive shots counts against the next ones.
#[test]
fn overdrive_hull_spent_by_predicted_shots_is_tracked() {
    let t = Tables::new();
    let terrain = room();
    let ship = alive_ship(&t);
    let laser = t.id("scrap_laser");
    let WeaponCost::Energy(cost) = t.weapons.get(laser).cost else {
        panic!("laser costs energy")
    };
    let price = cost * t.tuning.overdrive_hull_per_energy;
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(
        0,
        Some(laser),
        Resources {
            hull: 1.0 + price * 1.5,
            energy: 0.0,
            scrap: 0.0,
            count: 0,
        },
    );
    let over = cmd(Buttons::FIRE_PRIMARY.0 | Buttons::OVERDRIVE.0, 0);
    let interval = t.weapons.get(laser).fire_interval_ticks();
    let n: usize = (1..=5 * interval)
        .map(|k| p.on_tick(k, &ship, &over, t.fire(), &terrain))
        .sum();
    assert_eq!(
        n, 1,
        "the second overdrive shot would leave less than 1 hull"
    );
    assert_eq!(p.pending()[0].hull_cost, price);
}

/// A weapon seen on an own server projectile becomes selectable (the server
/// does not send the secondary selection, so pickups show up this way).
#[test]
fn weapons_seen_on_own_projectiles_become_selectable() {
    let t = Tables::new();
    let terrain = room();
    let ship = alive_ship(&t);
    let rivet = t.id("rivet_gun");
    let laser = t.id("scrap_laser");
    let res = Resources {
        hull: 100.0,
        energy: 100.0,
        scrap: 100.0,
        count: 3,
    };
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(0, Some(laser), res);
    let pick_rivet = cmd(Buttons::FIRE_PRIMARY.0, rivet.0 + 1);
    assert_eq!(p.on_tick(1, &ship, &pick_rivet, t.fire(), &terrain), 1);
    assert_eq!(p.pending()[0].weapon, laser, "rivet gun not carried yet");
    p.reconcile(
        1,
        &[NetProjectile {
            state: 0,
            id: 3,
            kind: rivet.0,
            owner: 0,
            pos: Vec3::new(50.0, 0.0, 0.0),
            vel: Vec3::new(0.0, 0.0, -30.0),
        }],
    );
    p.on_snapshot_state(1, Some(laser), res);
    let k = 1 + t
        .weapons
        .get(laser)
        .fire_interval_ticks()
        .max(t.weapons.get(rivet).fire_interval_ticks());
    assert_eq!(p.on_tick(k, &ship, &pick_rivet, t.fire(), &terrain), 1);
    assert_eq!(p.pending().last().unwrap().weapon, rivet);
}
