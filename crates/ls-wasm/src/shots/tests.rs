use super::*;
use ls_math::terrain::BoxRoomTerrain;
use ls_math::Quat;
use ls_sim::{Event, MatchSetup, PlayerId, RuleSet, ShipType, SpawnPoint, World, WorldConfig};

fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(200.0, 100.0, 200.0))
}

fn world() -> (World, PlayerId) {
    let setup = MatchSetup {
        wells: Vec::new(),
        belt: None,
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
        chicken: false,
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
        chicken: false,
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
            chicken: false,
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

/// Fires `weapon` (given to the ship) for `ticks` ticks with the predictor
/// fed the exact world ship state; returns the predictor, the world and
/// the fired projectile ids per tick. `reconcile` confirms every tick.
fn run_weapon(
    t: &Tables,
    weapon: &str,
    ticks: u64,
    buttons: impl Fn(u64) -> u16,
    reconcile: bool,
) -> (ShotPredictor, World, Vec<(u64, Vec<u32>)>) {
    let (mut w, me) = world();
    let mut terrain = room();
    w.give_weapon(me, weapon).unwrap();
    let id = t.id(weapon);
    let mut p = ShotPredictor::new(&t.rules);
    p.carry([id]);
    let mut fired_log = Vec::new();
    for i in 0..ticks {
        let c = InputCmd {
            tick: w.tick() + 1,
            ..cmd(buttons(i), id.0 + 1)
        };
        let before = w.ship(me).unwrap().clone();
        // Plenty of energy for the predictor (the world regenerates it).
        let mut res = resources(&before, t.id("bolt_rocket"));
        res.energy = res.energy.max(1000.0);
        p.on_snapshot_state(w.tick(), before.primary, res);
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
        // The Scrap Shotgun's random cone is drawn as its centre pellet only.
        let expect = if weapon == "scrap_shotgun" {
            fired.len().min(1)
        } else {
            fired.len()
        };
        assert_eq!(
            n, expect,
            "{weapon} tick {tick}: predicted {n}, fired {fired:?}"
        );
        if reconcile {
            let own: Vec<NetProjectile> = w.projectiles().iter().map(net).collect();
            p.reconcile(tick, &own);
        }
        fired_log.push((tick, fired));
    }
    (p, w, fired_log)
}

/// The prediction nearest to `pos` at tick `t`.
fn nearest(p: &ShotPredictor, pos: Vec3, t: f64) -> PredictedShot {
    *p.pending()
        .iter()
        .filter(|s| s.index != u8::MAX)
        .min_by(|a, b| {
            a.pos_at(t)
                .distance(pos)
                .total_cmp(&b.pos_at(t).distance(pos))
        })
        .expect("a prediction")
}

/// Every projectile of the world has a prediction at the same position
/// (within `tol`) at the world's tick.
fn assert_predictions_match(p: &ShotPredictor, w: &World, tol: f32) {
    let t = w.tick() as f64;
    for real in w.projectiles() {
        let best = nearest(p, real.pos, t);
        assert_eq!(best.weapon, real.weapon);
        let d = best.pos_at(t).distance(real.pos);
        assert!(
            d < tol,
            "{:?} at {:?}: nearest prediction {d}",
            real.weapon,
            real.pos
        );
    }
}

#[test]
fn scatter_laser_fans_alternate_like_the_simulation() {
    let t = Tables::new();
    // Three volleys: horizontal, vertical, horizontal.
    let interval = t.weapons.get(t.id("scatter_laser")).fire_interval_ticks();
    let (p, w, log) = run_weapon(
        &t,
        "scatter_laser",
        2 * interval + 1,
        |_| Buttons::FIRE_PRIMARY.0,
        false,
    );
    let volleys: Vec<&(u64, Vec<u32>)> = log.iter().filter(|(_, f)| !f.is_empty()).collect();
    assert_eq!(volleys.len(), 3);
    assert!(volleys.iter().all(|(_, f)| f.len() == 3));
    assert_predictions_match(&p, &w, 1e-3);
    // Directions match too.
    let now = w.tick() as f64;
    for real in w.projectiles() {
        assert!(nearest(&p, real.pos, now).vel.distance(real.vel) < 1e-3);
    }
    // The second fan is vertical: its bolts differ along the ship's up axis.
    let second: Vec<_> = p.pending().iter().filter(|s| s.phase == 1).collect();
    assert_eq!(second.len(), 3);
    let up = w.ships().next().unwrap().orientation.up();
    let spread_up = second
        .iter()
        .map(|s| s.vel.dot(up).abs())
        .fold(0.0f32, f32::max);
    assert!(spread_up > 1.0, "vertical fan: {spread_up}");
    assert_eq!(p.shot_count(t.id("scatter_laser")), 3);
}

#[test]
fn corkscrew_bolts_follow_the_spinning_helix() {
    let t = Tables::new();
    let interval = t.weapons.get(t.id("corkscrew")).fire_interval_ticks();
    // Two volleys (the second rotated per shot), then 40 ticks of flight.
    let (p, w, log) = run_weapon(
        &t,
        "corkscrew",
        interval + 41,
        |i| {
            if i <= interval {
                Buttons::FIRE_PRIMARY.0
            } else {
                0
            }
        },
        false,
    );
    let fired: usize = log.iter().map(|(_, f)| f.len()).sum();
    assert_eq!(fired, 10);
    assert_eq!(w.projectiles().len(), 10, "all in flight in the big room");
    assert_predictions_match(&p, &w, 1e-2);
    // Velocities of helix bolts turn with the spin (the sim's velocity is
    // the last tick's chord).
    let now = w.tick() as f64;
    for real in w.projectiles() {
        let pred = nearest(&p, real.pos, now);
        assert!(
            pred.vel_at(now).distance(real.vel) < 0.05,
            "{:?} vs {:?}",
            pred.vel_at(now),
            real.vel
        );
    }
}

#[test]
fn fans_and_helices_are_confirmed_by_snapshots() {
    let t = Tables::new();
    for weapon in ["scatter_laser", "corkscrew", "scrap_shotgun"] {
        let (p, _, log) = run_weapon(&t, weapon, 90, |_| Buttons::FIRE_PRIMARY.0, true);
        let fired: usize = if weapon == "scrap_shotgun" {
            log.iter().filter(|(_, f)| !f.is_empty()).count()
        } else {
            log.iter().map(|(_, f)| f.len()).sum()
        };
        let s = p.stats();
        assert!(fired > 0, "{weapon}");
        assert_eq!(s.predicted as usize, fired, "{weapon}");
        if weapon != "scrap_shotgun" {
            assert_eq!(
                (s.confirmed, s.unpredicted, s.dropped),
                (s.predicted, 0, 0),
                "{weapon}"
            );
        } else {
            // The shotgun's cone is random (seeded on the server): only the
            // centre pellet is drawn; a pellet near it may still match.
            assert!(s.dropped <= s.predicted, "{weapon}: {s:?}");
        }
    }
}

#[test]
fn leech_arc_draws_no_bolt_but_still_spends_energy() {
    let t = Tables::new();
    let (p, w, log) = run_weapon(&t, "leech_arc", 30, |_| Buttons::FIRE_PRIMARY.0, false);
    assert!(log.iter().all(|(_, f)| f.is_empty()), "the arc is instant");
    assert!(w.projectiles().is_empty());
    assert_eq!(p.stats().predicted, 0);
    assert!(
        p.pending().iter().all(|s| !s.visible_at(w.tick() as f64)),
        "nothing drawn"
    );
    // The arcs' energy is still counted against later shots.
    let spent: f32 = p.pending().iter().map(|s| s.cost).sum();
    assert!(spent > 0.0);
    // Its arcs are recorded for the locally drawn beam.
    let (tick, w) = p.last_beam().expect("arcs recorded");
    assert_eq!(w, t.id("leech_arc"));
    let last = p.pending().iter().map(|s| s.tick).max().unwrap();
    assert_eq!(tick, last);
}

#[test]
fn charge_weapons_fire_on_release_and_polite_laser_by_itself() {
    let t = Tables::new();
    let fusion = t.weapons.get(t.id("fusion_charger"));
    let full = ls_sim::WeaponDef::charge_ticks(fusion.charge.unwrap().max_secs);
    // Hold for 60 ticks, release: exactly one shot, on the release tick.
    let (p, w, log) = run_weapon(
        &t,
        "fusion_charger",
        70,
        |i| {
            if i < 60 {
                Buttons::FIRE_PRIMARY.0
            } else {
                0
            }
        },
        false,
    );
    let shots: Vec<u64> = log
        .iter()
        .filter(|(_, f)| !f.is_empty())
        .map(|(k, _)| *k)
        .collect();
    assert_eq!(shots.len(), 1, "{log:?}");
    assert_predictions_match(&p, &w, 1e-3);
    let s = p.pending().iter().find(|s| s.index != u8::MAX).unwrap();
    assert!((s.charge - 60.0 / full as f32).abs() < 0.02, "{}", s.charge);
    assert!(p.charge().is_none(), "released");

    // While held, the charge is mirrored for the meter.
    let (p, _, _) = run_weapon(&t, "fusion_charger", 30, |_| Buttons::FIRE_PRIMARY.0, false);
    let c = p.charge().expect("charging");
    assert_eq!(c.weapon, t.id("fusion_charger"));
    assert_eq!(c.full_ticks, full);

    // The Polite Laser fires by itself at full charge (1.5 s) while held.
    let (p, w, log) = run_weapon(&t, "polite_laser", 100, |_| Buttons::FIRE_PRIMARY.0, false);
    let shots: Vec<u64> = log
        .iter()
        .filter(|(_, f)| !f.is_empty())
        .map(|(k, _)| *k)
        .collect();
    assert_eq!(shots.len(), 1, "{log:?}");
    assert_predictions_match(&p, &w, 1e-3);
    // Released early: nothing (min_secs is the full 1.5 s).
    let (p, w, _) = run_weapon(
        &t,
        "polite_laser",
        60,
        |i| {
            if i < 30 {
                Buttons::FIRE_PRIMARY.0
            } else {
                0
            }
        },
        false,
    );
    assert!(w.projectiles().is_empty());
    assert_eq!(p.stats().predicted, 0);
}

/// The server's shot count (protocol 5 inventory) sets the fan phase: no
/// phase is inferred from projectiles.
#[test]
fn fan_phase_follows_the_inventory_shot_count() {
    let t = Tables::new();
    let scatter = t.id("scatter_laser");
    let (mut w, me) = world();
    let mut terrain = room();
    w.give_weapon(me, "scatter_laser").unwrap();
    // The server's scatter laser already fired once: its next fan is vertical.
    w.ship_mut(me).unwrap().weapon_mut(scatter).unwrap().shots = 1;
    let mut p = ShotPredictor::new(&t.rules);
    p.carry([scatter]);
    let interval = t.weapons.get(scatter).fire_interval_ticks();
    let start = w.tick();
    let mut fired_ticks = Vec::new();
    for _ in 0..(interval + 1) {
        let c = InputCmd {
            tick: w.tick() + 1,
            ..cmd(Buttons::FIRE_PRIMARY.0, scatter.0 + 1)
        };
        let before = w.ship(me).unwrap().clone();
        let mut res = resources(&before, t.id("bolt_rocket"));
        res.energy = 1000.0;
        p.on_snapshot_state(w.tick(), before.primary, res);
        w.step(&mut terrain, &[(me, c)]);
        let tick = w.tick();
        let ship = w.ship(me).unwrap().clone();
        if p.on_tick(tick, &ship, &c, t.fire(), &terrain) > 0 {
            fired_ticks.push(tick);
        }
    }
    assert_eq!(fired_ticks.len(), 2);
    // Both volleys were predicted with the local count (phases 0 and 1):
    // horizontal first, which the world did not fire.
    assert_eq!(p.shot_count(scatter), 2);
    // The inventory from before the first shot arrives late (it was in
    // flight): shots = 1 at `start`. Both volleys after it are rebuilt with
    // the server's phases 1 and 2.
    p.on_inventory(start, &[(scatter, 1)], Some(&terrain));
    assert_eq!(p.stats().rephased, 2);
    assert_eq!(p.shot_count(scatter), 3);
    let now = w.tick() as f64;
    assert_eq!(w.projectiles().len(), 6);
    for real in w.projectiles() {
        let d = nearest(&p, real.pos, now).pos_at(now).distance(real.pos);
        assert!(d < 1e-3, "{d}");
    }
    // Every projectile is then confirmed by its prediction.
    let own: Vec<NetProjectile> = w.projectiles().iter().map(net).collect();
    p.reconcile(w.tick(), &own);
    assert_eq!(p.stats().confirmed, 6, "{:?}", p.stats());
    assert_eq!(p.stats().unpredicted, 0);
    // A newer inventory that agrees changes nothing.
    let last = *fired_ticks.last().unwrap();
    p.on_inventory(last, &[(scatter, 3)], Some(&terrain));
    assert_eq!(p.shot_count(scatter), 3);
    assert_eq!(p.stats().rephased, 2);
}

#[test]
fn offline_weapons_and_guided_eyeball_stop_predictions() {
    let t = Tables::new();
    let terrain = room();
    let ship = alive_ship(&t);
    let res = Resources {
        hull: 100.0,
        energy: 1000.0,
        scrap: 100.0,
        count: 3,
    };
    let both = cmd(Buttons::FIRE_PRIMARY.0 | Buttons::FIRE_SECONDARY.0, 0);

    // Borrowed Time: nothing fires before the weapons are back.
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(0, Some(t.id("scrap_laser")), res);
    p.set_offline_until(10);
    assert_eq!(p.on_tick(5, &ship, &both, t.fire(), &terrain), 0);
    assert_eq!(p.on_tick(10, &ship, &both, t.fire(), &terrain), 2);

    // Guiding an Eyeball: the primary fires, the secondary does not.
    let mut p = ShotPredictor::new(&t.rules);
    p.on_snapshot_state(0, Some(t.id("scrap_laser")), res);
    p.set_guiding(true);
    assert_eq!(p.on_tick(1, &ship, &both, t.fire(), &terrain), 1);
    assert!(p.pending().iter().all(|s| s.weapon == t.id("scrap_laser")));
    p.set_guiding(false);
    assert_eq!(
        p.on_tick(
            2,
            &ship,
            &cmd(Buttons::FIRE_SECONDARY.0, 0),
            t.fire(),
            &terrain
        ),
        1
    );
}

#[test]
fn borrowed_time_takes_the_weapons_offline_as_it_fires() {
    let t = Tables::new();
    let terrain = room();
    let ship = alive_ship(&t);
    let bt = t.id("borrowed_time");
    let laser = t.id("scrap_laser");
    let res = Resources {
        hull: 100.0,
        energy: 100.0,
        scrap: 100.0,
        count: 3,
    };
    let mut p = ShotPredictor::new(&t.rules);
    p.carry([bt]);
    p.on_snapshot_state(0, Some(laser), res);
    // The rewind is not drawn, but it is paid.
    let rewind = cmd(Buttons::FIRE_SECONDARY.0, bt.0 + 1);
    assert_eq!(p.on_tick(1, &ship, &rewind, t.fire(), &terrain), 0);
    // Without waiting a round trip for the Rewound cue, no laser shot is
    // predicted while the weapons are offline; afterwards it is.
    let offline = ls_math::Tick::ticks_for_secs(t.weapons.get(bt).param("offline_secs", 3.0));
    let fire = cmd(Buttons::FIRE_PRIMARY.0, laser.0 + 1);
    assert_eq!(p.on_tick(30, &ship, &fire, t.fire(), &terrain), 0);
    assert_eq!(p.on_tick(2 + offline, &ship, &fire, t.fire(), &terrain), 1);
}
