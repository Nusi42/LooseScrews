//! Brain unit tests: movement plans (M1/wave A) and the arsenal use of
//! wave C (weapon choice, charge, secondaries, gadgets, Eyeball guidance,
//! Tectonic Charge fleeing, pickups).

use super::*;
use crate::aim::lead_point;
use ls_game::ls_math::terrain::BoxRoomTerrain;
use ls_game::ls_sim::{simulate_movement, CueKind, PlayerId, ShipState, WeaponTable};

fn table() -> WeaponTable {
    WeaponTable::builtin()
}

fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(80.0, 40.0, 80.0))
}

fn brain(terrain: &BoxRoomTerrain, spots: Vec<Vec3>) -> Brain {
    Brain::new(
        BrainConfig::default(),
        Arsenal::from_table(&table()),
        ShipTuning::tinker(),
        terrain.bounds(),
        spots,
        7,
    )
}

fn perception(pos: Vec3) -> Perception {
    let t = ShipTuning::tinker();
    Perception {
        tick: 1000,
        pos,
        vel: Vec3::ZERO,
        orientation: Quat::IDENTITY,
        alive: true,
        team: 0,
        hull: t.hull_max,
        energy: t.energy_max,
        boost: t.boost_max,
        scrap: t.scrap_max,
        primary: table().find("scrap_laser"),
        id: 0,
        ship_type: ShipType::Tinker,
        secondary: None,
        weapons: None,
        gadget: None,
        gadget_on: false,
        contacts: Vec::new(),
        projectiles: Vec::new(),
        pickups: Vec::new(),
        scrap_sources: Vec::new(),
    }
}

fn ship(p: &Perception) -> ShipState {
    let mut s = ShipState::new(
        PlayerId(0),
        p.team,
        ShipType::Tinker,
        &ShipTuning::tinker(),
        p.pos,
        p.orientation,
    );
    s.vel = p.vel;
    s
}

#[test]
fn arsenal_finds_the_m1_weapons() {
    let a = Arsenal::from_table(&table());
    assert_eq!(a.laser.unwrap().speed, 30.0);
    assert_eq!(a.rivet.unwrap().speed, 180.0);
    assert_eq!(a.rocket.unwrap().speed, 40.0);
    assert_eq!(a.paint.unwrap().speed, 20.0);
    assert_eq!(a.gun(a.paint.unwrap().id), a.paint);
    assert!((a.paint.unwrap().range() - 30.0).abs() < 1e-4);
}

#[test]
fn dead_bot_idles() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::ZERO);
    p.alive = false;
    let mut c = BrainCounters::default();
    let cmd = b.think(&p, &t, &mut c);
    assert_eq!(cmd.thrust, Vec3::ZERO);
    assert!(!cmd.buttons.contains(Buttons::FIRE_PRIMARY));
    assert_eq!(b.mode(), Mode::Dead);
    assert_eq!(c.dead_ticks, 1);
}

#[test]
fn aims_with_lead_and_fires_when_on_target() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::ZERO);
    let enemy = Contact {
        id: 3,
        team: 1,
        pos: Vec3::new(0.0, 0.0, -20.0),
        vel: Vec3::new(10.0, 0.0, 0.0),
    };
    p.contacts = vec![
        enemy,
        // A team mate closer by is never a target.
        Contact {
            id: 4,
            team: 0,
            pos: Vec3::new(0.0, 0.0, -5.0),
            vel: Vec3::ZERO,
        },
    ];
    let laser = b.arsenal().laser.unwrap();
    let lead = lead_point(p.pos, enemy.pos, enemy.vel, laser.speed, laser.lifetime).unwrap();
    let mut c = BrainCounters::default();

    // Nose straight at the target's current position: must turn towards
    // the lead (to +X means negative yaw), not fire yet.
    let cmd = b.think(&p, &t, &mut c);
    assert_eq!(b.mode(), Mode::Combat);
    assert_eq!(b.target(), Some(3));
    assert!(cmd.look_yaw < 0.0, "{cmd:?}");
    assert!(!cmd.buttons.contains(Buttons::FIRE_PRIMARY));

    // Nose on the lead point: fire.
    p.tick += 1;
    p.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, lead.normalize());
    let cmd = b.think(&p, &t, &mut c);
    assert!(cmd.buttons.contains(Buttons::FIRE_PRIMARY), "{cmd:?}");
    assert_eq!(c.laser_triggers, 1);
    // In rocket range with a tight aim at the rocket's lead? The rocket
    // is faster, so its lead differs and it holds fire.
    assert!(!cmd.buttons.contains(Buttons::FIRE_SECONDARY) || c.rocket_triggers == 1);
}

#[test]
fn does_not_fire_without_energy_or_line_of_sight() {
    let mut t = room();
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::ZERO);
    p.contacts = vec![Contact {
        id: 1,
        team: 1,
        pos: Vec3::new(0.0, 0.0, -20.0),
        vel: Vec3::ZERO,
    }];
    p.energy = 1.0;
    let mut c = BrainCounters::default();
    let cmd = b.think(&p, &t, &mut c);
    assert!(!cmd.buttons.contains(Buttons::FIRE_PRIMARY));

    // Rock between the two ships: no target at all.
    t.apply(ls_game::ls_math::TerrainEdit::AddSphere {
        center: Vec3::new(0.0, 0.0, -10.0),
        radius: 3.0,
    });
    let mut b = brain(&t, vec![]);
    p.energy = 100.0;
    b.think(&p, &t, &mut c);
    assert_eq!(b.target(), None);
    assert_ne!(b.mode(), Mode::Combat);
}

#[test]
fn fires_the_rivet_gun_when_equipped() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let rivet = b.arsenal().rivet.unwrap().id;
    let mut p = perception(Vec3::ZERO);
    p.weapons = Some(vec![
        (id("scrap_laser"), 0),
        (rivet, 0),
        (id("bolt_rocket"), 0),
    ]);
    p.contacts = vec![Contact {
        id: 1,
        team: 1,
        pos: Vec3::new(0.0, 0.0, -15.0),
        vel: Vec3::ZERO,
    }];
    let mut c = BrainCounters::default();
    // The snapshot shows the Rivet Gun equipped: it fires it.
    p.primary = Some(rivet);
    let cmd = b.think(&p, &t, &mut c);
    assert!(cmd.buttons.contains(Buttons::FIRE_PRIMARY), "{cmd:?}");
    assert_eq!(c.rivet_triggers, 1);
    assert_eq!(c.weapon_triggers[rivet.0 as usize], 1);
    // No scrap: no rivets.
    p.scrap = 0.0;
    p.tick += 1;
    let cmd = b.think(&p, &t, &mut c);
    assert!(!cmd.buttons.contains(Buttons::FIRE_PRIMARY), "{cmd:?}");
}

#[test]
fn paints_a_nearby_wall() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let paint = b.arsenal().paint.unwrap().id;
    // 10 u in front of the -Z wall, looking at it.
    let mut p = perception(Vec3::new(0.0, 0.0, -30.0));
    let mut c = BrainCounters::default();
    let mut s = ship(&p);
    let mut selected = false;
    for _ in 0..240 {
        let cmd = b.think(&p, &t, &mut c);
        if cmd.weapon_select == paint.0 + 1 {
            selected = true;
            // The server would switch the weapon.
            p.primary = Some(paint);
        }
        simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), &t);
        p.pos = s.pos;
        p.vel = s.vel;
        p.orientation = s.orientation;
        p.tick += 1;
    }
    assert!(selected, "never selected the Paint Thrower");
    assert!(c.paint_triggers > 20, "{c:?}");
    assert!(c.paint_ticks > 0);
}

#[test]
fn scavenges_when_hull_is_low() {
    let t = room();
    let spot = Vec3::new(20.0, 0.0, 0.0);
    let mut b = brain(&t, vec![spot, Vec3::new(-30.0, 10.0, 30.0)]);
    let mut p = perception(Vec3::new(-10.0, 0.0, 0.0));
    p.hull = 20.0;
    let mut c = BrainCounters::default();
    let mut s = ship(&p);
    let mut held = false;
    for _ in 0..60 * 6 {
        let cmd = b.think(&p, &t, &mut c);
        assert_eq!(b.mode(), Mode::Scavenge);
        if cmd.buttons.contains(Buttons::SCAVENGE) {
            held = true;
            assert!(p.pos.distance(spot) <= ShipTuning::tinker().scavenge_radius);
            p.hull += 0.3; // the server repairs while in reach
        }
        simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), &t);
        p.pos = s.pos;
        p.vel = s.vel;
        p.orientation = s.orientation;
        p.tick += 1;
    }
    assert!(held, "never held scavenge; ended at {:?}", p.pos);
    // Healed past the threshold: back to normal.
    p.hull = 90.0;
    b.think(&p, &t, &mut c);
    assert_ne!(b.mode(), Mode::Scavenge);
}

#[test]
fn dry_scrap_spot_is_abandoned() {
    let t = room();
    let a = Vec3::new(5.0, 0.0, 0.0);
    let far = Vec3::new(-30.0, 10.0, 30.0);
    let mut b = brain(&t, vec![a, far]);
    let mut p = perception(a);
    p.hull = 20.0;
    let mut c = BrainCounters::default();
    for _ in 0..(SCAVENGE_CHECK_TICKS + 5) {
        b.think(&p, &t, &mut c);
        p.tick += 1;
    }
    // No hull gained at `a`: the bot moves on to the other spot.
    assert_eq!(b.scavenge.map(|j| j.spot), Some(far));
    // Wrecks are sources too.
    b.on_wreck(Vec3::new(4.0, 1.0, 0.0), p.tick);
    assert!(b.wrecks.len() == 1);
}

#[test]
fn wanders_without_hitting_walls() {
    let mut t = room();
    t.apply(ls_game::ls_math::TerrainEdit::AddSphere {
        center: Vec3::new(10.0, 0.0, 10.0),
        radius: 8.0,
    });
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::new(-20.0, 0.0, -20.0));
    let mut s = ship(&p);
    let mut c = BrainCounters::default();
    let mut contacts = 0;
    let start = p.pos;
    let mut max_travel: f32 = 0.0;
    for _ in 0..60 * 30 {
        let cmd = b.think(&p, &t, &mut c);
        let r = simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), &t);
        if r.contact {
            contacts += 1;
        }
        p.pos = s.pos;
        p.vel = s.vel;
        p.orientation = s.orientation;
        p.boost = s.boost;
        p.energy = s.energy;
        p.tick += 1;
        max_travel = max_travel.max(p.pos.distance(start));
    }
    assert!(contacts < 10, "{contacts} ticks in contact with rock");
    assert!(max_travel > 20.0, "bot did not move: {max_travel}");
    assert!(c.wander_ticks + c.paint_ticks > 0);
}
/// Steps the bot through the flight model for up to `ticks` (until
/// `until` holds); returns the modes seen.
fn fly(
    b: &mut Brain,
    t: &BoxRoomTerrain,
    p: &mut Perception,
    ticks: u64,
    mut until: impl FnMut(&Brain, &Perception) -> bool,
) -> Vec<Mode> {
    let mut s = ship(p);
    let mut c = BrainCounters::default();
    let mut modes = Vec::new();
    for _ in 0..ticks {
        let cmd = b.think(p, t, &mut c);
        if !modes.contains(&b.mode()) {
            modes.push(b.mode());
        }
        if until(b, p) {
            break;
        }
        simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), t);
        p.pos = s.pos;
        p.vel = s.vel;
        p.orientation = s.orientation;
        p.boost = s.boost;
        p.energy = s.energy;
        p.tick += 1;
    }
    modes
}

#[test]
fn hunts_an_enemy_out_of_sight_until_it_can_fight() {
    // A rock ball between the bot and an enemy hovering behind it.
    let mut t = room();
    t.apply(ls_game::ls_math::TerrainEdit::AddSphere {
        center: Vec3::ZERO,
        radius: 12.0,
    });
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::new(0.0, 0.0, -30.0));
    let enemy = Contact {
        id: 5,
        team: 1,
        pos: Vec3::new(0.0, 0.0, 30.0),
        vel: Vec3::ZERO,
    };
    p.contacts = vec![enemy];
    let modes = fly(&mut b, &t, &mut p, 60 * 30, |b, _| b.mode() == Mode::Combat);
    assert!(modes.contains(&Mode::Hunt), "{modes:?}");
    assert_eq!(
        b.mode(),
        Mode::Combat,
        "never got the enemy in sight: {:?}",
        p.pos
    );
    assert_eq!(b.target(), Some(5));
    assert!(b.navigator().is_ready());
}

#[test]
fn hunts_near_enemy_spawns_when_no_enemy_is_alive() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let theirs = Vec3::new(30.0, 0.0, 30.0);
    b.set_spawn_points(vec![(0, Vec3::new(-30.0, 0.0, -30.0)), (1, theirs)]);
    let mut p = perception(Vec3::new(-30.0, 0.0, -30.0));
    let mut closest = f32::MAX;
    fly(&mut b, &t, &mut p, 60 * 15, |_, p| {
        closest = closest.min(p.pos.distance(theirs));
        closest < 5.0
    });
    assert!(closest < 5.0, "never reached the enemy spawn: {closest}");
    // After a break the bot goes back to painting and wandering.
    b.take_hunt_break(p.tick);
    let mut c = BrainCounters::default();
    b.think(&p, &t, &mut c);
    assert!(
        matches!(b.mode(), Mode::Wander | Mode::Paint),
        "{:?}",
        b.mode()
    );
}

#[test]
fn craters_reach_the_navigation_grid() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let p = perception(Vec3::ZERO);
    let mut c = BrainCounters::default();
    while !b.navigator().is_ready() {
        b.think(&p, &t, &mut c);
    }
    let mut t2 = room();
    // Rock appears where the grid thought space was open.
    let edit = ls_game::ls_math::TerrainEdit::AddSphere {
        center: Vec3::new(10.0, 0.0, 0.0),
        radius: 4.0,
    };
    t2.apply(edit);
    let v = b.navigator().grid().version();
    b.on_terrain_edit(&edit);
    b.think(&p, &t2, &mut c);
    assert_ne!(b.navigator().grid().version(), v);
}

// ----- wave C: the whole arsenal ------------------------------------------------

fn id(name: &str) -> WeaponId {
    table().find(name).unwrap()
}

fn enemy(id: u8, pos: Vec3, vel: Vec3) -> Contact {
    Contact {
        id,
        team: 1,
        pos,
        vel,
    }
}

/// A bot at the origin looking down -Z carrying `weapons` (with ammo),
/// with `secondary` selected.
fn armed(weapons: &[(&str, u32)], secondary: Option<&str>) -> Perception {
    let mut p = perception(Vec3::ZERO);
    p.weapons = Some(weapons.iter().map(|(n, a)| (id(n), *a)).collect());
    p.secondary = secondary.map(id);
    p
}

/// Runs `ticks` thinks without moving (the server switches weapons as
/// asked); returns the commands.
fn hold(b: &mut Brain, t: &BoxRoomTerrain, p: &mut Perception, ticks: u32) -> Vec<InputCmd> {
    let mut c = BrainCounters::default();
    let mut out = Vec::new();
    for _ in 0..ticks {
        let cmd = b.think(p, t, &mut c);
        if cmd.weapon_select > 0 {
            let w = WeaponId(cmd.weapon_select - 1);
            if b.arsenal().gun(w).unwrap().is_primary() {
                p.primary = Some(w);
            } else {
                p.secondary = Some(w);
            }
        }
        out.push(cmd);
        p.tick += 1;
    }
    out
}

#[test]
fn primary_follows_the_range() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let p = armed(
        &[("scrap_laser", 0), ("scrap_shotgun", 0), ("railspike", 0)],
        None,
    );
    let owned = b.owned(&p);
    for i in 0..40 {
        let mut p = p.clone();
        p.tick = 1000 + i * 1000;
        // Long range: only the Railspike reaches.
        let far = b.choose_primary(&p, &owned, 100.0).unwrap();
        assert_eq!(far.role, Role::Railspike);
        // Point blank: never the Railspike (its minimum distance).
        p.tick += 500;
        let close = b.choose_primary(&p, &owned, 4.0).unwrap();
        assert_ne!(close.role, Role::Railspike);
    }
    // Close range prefers the shotgun over the laser.
    let mut shotgun = 0;
    for i in 0..200 {
        let mut p = p.clone();
        p.tick = 500_000 + i * 1000;
        if b.choose_primary(&p, &owned, 4.0).unwrap().role == Role::ScrapShotgun {
            shotgun += 1;
        }
    }
    assert!(shotgun > 120, "shotgun picked {shotgun} of 200 times");
}

#[test]
fn fusion_charger_releases_before_the_overcharge() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = armed(&[("fusion_charger", 0)], None);
    p.primary = Some(id("fusion_charger"));
    p.contacts = vec![enemy(1, Vec3::new(0.0, 0.0, -20.0), Vec3::ZERO)];
    let cmds = hold(&mut b, &t, &mut p, 600);
    let mut runs = Vec::new();
    let mut run = 0;
    for c in &cmds {
        if c.buttons.contains(Buttons::FIRE_PRIMARY) {
            run += 1;
        } else if run > 0 {
            runs.push(run);
            run = 0;
        }
    }
    assert!(runs.len() >= 2, "{runs:?}");
    // Held long enough to charge, released before 2 s (120 ticks).
    assert!(runs.iter().all(|r| *r >= 20 && *r < 120), "{runs:?}");
}

#[test]
fn polite_laser_is_held_until_it_fires() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = armed(&[("polite_laser", 0)], None);
    p.primary = Some(id("polite_laser"));
    p.contacts = vec![enemy(1, Vec3::new(0.0, 0.0, -30.0), Vec3::ZERO)];
    let cmds = hold(&mut b, &t, &mut p, 300);
    let held = cmds
        .iter()
        .take_while(|c| c.buttons.contains(Buttons::FIRE_PRIMARY))
        .count();
    // 1.5 s charge that fires by itself at 90 ticks.
    assert!(held > 90, "held {held} ticks");
    assert!(cmds
        .iter()
        .skip(held)
        .any(|c| !c.buttons.contains(Buttons::FIRE_PRIMARY)));
}

#[test]
fn drops_a_mine_when_chased() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = armed(
        &[("scrap_laser", 0), ("bolt_rocket", 3), ("prox_mine", 2)],
        Some("bolt_rocket"),
    );
    // Flying +X, an enemy right behind and following.
    p.vel = Vec3::new(10.0, 0.0, 0.0);
    p.contacts = vec![enemy(
        1,
        Vec3::new(-10.0, 0.0, 0.0),
        Vec3::new(10.0, 0.0, 0.0),
    )];
    assert!(b.chased(&p).is_some());
    let cmds = hold(&mut b, &t, &mut p, 120);
    let fired = cmds
        .iter()
        .position(|c| c.buttons.contains(Buttons::FIRE_SECONDARY))
        .expect("never fired a secondary");
    assert_eq!(p.secondary, Some(id("prox_mine")), "fired at tick {fired}");
}

#[test]
fn borrowed_time_when_low_and_hit() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = armed(
        &[("scrap_laser", 0), ("borrowed_time", 1)],
        Some("borrowed_time"),
    );
    p.hull = 20.0;
    p.contacts = vec![enemy(1, Vec3::new(0.0, 0.0, -20.0), Vec3::ZERO)];
    let mut fired = false;
    let mut c = BrainCounters::default();
    for _ in 0..90 {
        b.on_damaged(Some(id("scrap_laser")), p.tick);
        let cmd = b.think(&p, &t, &mut c);
        fired |= cmd.buttons.contains(Buttons::FIRE_SECONDARY);
        p.tick += 1;
    }
    assert!(fired);
    assert_eq!(c.weapon_triggers[id("borrowed_time").0 as usize], 1);
}

#[test]
fn tectonic_charge_on_a_cluster_then_flee() {
    let t = BoxRoomTerrain::centered(Vec3::new(160.0, 40.0, 160.0));
    let mut b = brain(&t, vec![]);
    let mut p = armed(
        &[("scrap_laser", 0), ("tectonic_charge", 1)],
        Some("tectonic_charge"),
    );
    // Two enemies close together 45 u ahead.
    p.contacts = vec![
        enemy(1, Vec3::new(0.0, 0.0, -45.0), Vec3::ZERO),
        enemy(2, Vec3::new(4.0, 0.0, -47.0), Vec3::ZERO),
    ];
    let cmds = hold(&mut b, &t, &mut p, 200);
    assert!(
        cmds.iter()
            .any(|c| c.buttons.contains(Buttons::FIRE_SECONDARY)),
        "never launched it"
    );
    // The charge in flight towards them: the launcher is inside its zone
    // at first and gets away.
    p.projectiles = vec![ProjectileSeen {
        id: 9,
        weapon: id("tectonic_charge"),
        owner: 0,
        pos: Vec3::new(0.0, 0.0, -5.0),
        vel: Vec3::new(0.0, 0.0, -15.0),
        guided: false,
        stuck: false,
    }];
    let mut c = BrainCounters::default();
    b.think(&p, &t, &mut c);
    assert_eq!(b.mode(), Mode::Flee);
    assert!(b.danger().is_some());
}

#[test]
fn flees_the_zone_on_the_crack_cue() {
    let t = BoxRoomTerrain::centered(Vec3::new(160.0, 40.0, 160.0));
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::new(10.0, 0.0, 0.0));
    b.on_cue(CueKind::TectonicArmed, Vec3::ZERO, 90, p.tick);
    let start = p.pos.distance(Vec3::ZERO);
    let modes = fly(&mut b, &t, &mut p, 60 * 5, |_, _| false);
    assert!(modes.contains(&Mode::Flee), "{modes:?}");
    let end = p.pos.distance(Vec3::ZERO);
    assert!(end > 40.0, "only got from {start} to {end} u");
    // The rumble alone (launch far away) is no reason to run.
    let mut b = brain(&t, vec![]);
    let p = perception(Vec3::new(60.0, 0.0, 0.0));
    b.on_cue(CueKind::TectonicLaunch, Vec3::ZERO, 0, p.tick);
    let mut c = BrainCounters::default();
    b.think(&p, &t, &mut c);
    assert_ne!(b.mode(), Mode::Flee);
}

#[test]
fn guides_the_eyeball_towards_the_target() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = armed(&[("scrap_laser", 0), ("eyeball", 0)], Some("eyeball"));
    // Own guided missile flying -Z; the enemy is off to the left (-X).
    p.projectiles = vec![ProjectileSeen {
        id: 4,
        weapon: id("eyeball"),
        owner: 0,
        pos: Vec3::new(0.0, 0.0, -5.0),
        vel: Vec3::new(0.0, 0.0, -25.0),
        guided: true,
        stuck: false,
    }];
    p.contacts = vec![enemy(1, Vec3::new(-20.0, 0.0, -15.0), Vec3::ZERO)];
    let mut c = BrainCounters::default();
    let cmd = b.think(&p, &t, &mut c);
    assert_eq!(b.mode(), Mode::Guide);
    assert_eq!(cmd.thrust, Vec3::ZERO);
    assert!(cmd.look_yaw > 0.0, "positive yaw turns left: {cmd:?}");
    assert!(cmd.look_yaw <= 3.0 * DT + 1e-6);
    assert_eq!(c.guide_ticks, 1);
}

#[test]
fn homing_ordnance_at_a_fleeing_target() {
    let t = BoxRoomTerrain::centered(Vec3::new(160.0, 40.0, 160.0));
    let mut b = brain(&t, vec![]);
    let mut p = armed(&[("scrap_laser", 0), ("seeker", 2)], Some("seeker"));
    // Flying away from the bot at 10 u/s, 30 u ahead.
    p.contacts = vec![enemy(
        1,
        Vec3::new(0.0, 0.0, -30.0),
        Vec3::new(0.0, 0.0, -10.0),
    )];
    let seeker = id("seeker");
    let mut fired = false;
    let mut c = BrainCounters::default();
    for _ in 0..150 {
        let lead = lead_point(p.pos, p.contacts[0].pos, p.contacts[0].vel, 25.0, 6.0).unwrap();
        p.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, lead.normalize());
        let cmd = b.think(&p, &t, &mut c);
        fired |= cmd.buttons.contains(Buttons::FIRE_SECONDARY);
        p.tick += 1;
    }
    assert!(fired);
    assert!(c.weapon_triggers[seeker.0 as usize] > 0);
}

#[test]
fn shield_against_incoming_splash_and_off_again() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::ZERO);
    p.gadget = Some(GadgetKind::Shield);
    // An enemy's Heavy Rocket 6 u away, flying at the bot.
    p.contacts = vec![enemy(1, Vec3::new(0.0, 0.0, -30.0), Vec3::ZERO)];
    p.projectiles = vec![ProjectileSeen {
        id: 7,
        weapon: id("heavy_rocket"),
        owner: 1,
        pos: Vec3::new(0.0, 0.0, -6.0),
        vel: Vec3::new(0.0, 0.0, 20.0),
        guided: false,
        stuck: false,
    }];
    let mut c = BrainCounters::default();
    let mut pressed = None;
    for i in 0..60 {
        let cmd = b.think(&p, &t, &mut c);
        if cmd.buttons.contains(Buttons::GADGET) {
            pressed = Some(i);
            break;
        }
        p.tick += 1;
    }
    assert!(pressed.is_some(), "shield never switched on");
    // Shield on, the rocket gone: it goes off after the threat.
    p.gadget_on = true;
    p.projectiles.clear();
    let mut off = false;
    for _ in 0..300 {
        p.tick += 1;
        off |= b.think(&p, &t, &mut c).buttons.contains(Buttons::GADGET);
    }
    assert!(off, "shield never switched off");
}

#[test]
fn decoy_when_chased_and_cloak_when_hunting() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::ZERO);
    p.gadget = Some(GadgetKind::Decoy);
    p.vel = Vec3::new(10.0, 0.0, 0.0);
    p.contacts = vec![enemy(
        1,
        Vec3::new(-12.0, 0.0, 0.0),
        Vec3::new(9.0, 0.0, 0.0),
    )];
    let cmds = hold(&mut b, &t, &mut p, 60);
    assert!(cmds.iter().any(|c| c.buttons.contains(Buttons::GADGET)));

    // Cloak: switched on while hunting an enemy out of sight.
    let mut t = room();
    t.apply(ls_game::ls_math::TerrainEdit::AddSphere {
        center: Vec3::ZERO,
        radius: 12.0,
    });
    let mut b = brain(&t, vec![]);
    let mut p = perception(Vec3::new(0.0, 0.0, -30.0));
    p.gadget = Some(GadgetKind::Cloak);
    p.contacts = vec![enemy(5, Vec3::new(0.0, 0.0, 30.0), Vec3::ZERO)];
    let mut pressed = false;
    let mut c = BrainCounters::default();
    for _ in 0..600 {
        let cmd = b.think(&p, &t, &mut c);
        if b.mode() == Mode::Hunt && cmd.buttons.contains(Buttons::GADGET) {
            pressed = true;
            break;
        }
        p.tick += 1;
    }
    assert!(pressed, "never cloaked while hunting");
}

#[test]
fn fetches_a_wanted_pickup() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = armed(
        &[("scrap_laser", 0), ("paint_thrower", 0), ("bolt_rocket", 3)],
        Some("bolt_rocket"),
    );
    let spot = Vec3::new(15.0, 0.0, -10.0);
    p.pickups = vec![
        // Owned already: not interesting.
        PickupSeen {
            id: 1,
            pos: Vec3::new(-5.0, 0.0, 0.0),
            item: PickupItem::Weapon(id("scrap_laser")),
        },
        PickupSeen {
            id: 2,
            pos: spot,
            item: PickupItem::Weapon(id("railspike")),
        },
    ];
    let modes = fly(&mut b, &t, &mut p, 60 * 6, |_, p| {
        p.pos.distance(spot) < 1.5
    });
    assert!(modes.contains(&Mode::Collect), "{modes:?}");
    assert!(p.pos.distance(spot) < 1.5, "ended at {:?}", p.pos);
}

#[test]
fn magnet_sneeze_pulls_scrap_when_short() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let mut p = armed(
        &[("scrap_laser", 0), ("magnet_sneeze", 2)],
        Some("magnet_sneeze"),
    );
    p.scrap = 5.0;
    p.scrap_sources = vec![(Vec3::new(0.0, 0.0, -15.0), 30.0)];
    let mut fired = false;
    let mut c = BrainCounters::default();
    for _ in 0..600 {
        let cmd = b.think(&p, &t, &mut c);
        if cmd.buttons.contains(Buttons::FIRE_SECONDARY) {
            fired = true;
            break;
        }
        p.tick += 1;
    }
    assert!(fired);
}

#[test]
fn swap_harpoon_out_of_a_tectonic_zone() {
    let t = BoxRoomTerrain::centered(Vec3::new(160.0, 40.0, 160.0));
    let mut b = brain(&t, vec![]);
    let p = armed(
        &[("scrap_laser", 0), ("swap_harpoon", 2)],
        Some("swap_harpoon"),
    );
    b.on_cue(
        CueKind::TectonicArmed,
        Vec3::new(0.0, 0.0, 10.0),
        90,
        p.tick,
    );
    // An enemy well outside the zone, straight ahead.
    let e = enemy(1, Vec3::new(0.0, 0.0, -45.0), Vec3::ZERO);
    let mut p = p;
    p.contacts = vec![e];
    let owned = b.owned(&p);
    let i = b.decide_secondary(&p, Some(&e), &t, &owned).unwrap();
    assert_eq!(i.weapon, id("swap_harpoon"));
}

#[test]
fn paint_colour_is_cycled_when_the_game_has_the_button() {
    let t = room();
    let cfg = BrainConfig {
        paint_colour_button: Some(Buttons(1 << 7)),
        ..BrainConfig::default()
    };
    let mut b = Brain::new(
        cfg,
        Arsenal::from_table(&table()),
        ShipTuning::tinker(),
        t.bounds(),
        vec![],
        11,
    );
    let mut p = perception(Vec3::new(0.0, 0.0, -30.0));
    let mut c = BrainCounters::default();
    let mut s = ship(&p);
    for _ in 0..60 * 30 {
        let cmd = b.think(&p, &t, &mut c);
        if let Some(w) = cmd.weapon_select.checked_sub(1) {
            p.primary = Some(WeaponId(w));
        }
        simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), &t);
        p.pos = s.pos;
        p.vel = s.vel;
        p.orientation = s.orientation;
        p.tick += 1;
    }
    assert!(c.paint_triggers > 0);
    assert!(c.colour_presses > 0, "{c:?}");
}

#[test]
fn ship_type_switches_the_tuning() {
    let t = room();
    let mut b = brain(&t, vec![]);
    let hauler = ls_game::ls_sim::ShipTable::builtin()
        .get(ShipType::Hauler)
        .clone();
    b.set_ship(ShipType::Hauler, &hauler);
    assert_eq!(b.ship_type(), ShipType::Hauler);
    // 50 of 150 hull is below the scavenge threshold.
    let mut p = perception(Vec3::ZERO);
    p.hull = 50.0;
    p.ship_type = ShipType::Hauler;
    b.set_scrap_spots(vec![Vec3::new(10.0, 0.0, 0.0)]);
    let mut c = BrainCounters::default();
    b.think(&p, &t, &mut c);
    assert_eq!(b.mode(), Mode::Scavenge);
}
