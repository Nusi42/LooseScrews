//! Heavy Rocket: ordnance secondary. 20 u/s, 8 u blast. Malus: very slow; hurts you at close
//! range. Lasting effect: 6 u crater that can open a passage between tunnels.
//!
//! Numbers: `data/weapons/heavy_rocket.ron` (70 direct per the M2 damage scale, 8 u splash
//! that also hits the shooter, crater radius 3 u = the design doc's 6 u diameter).
//!
//! Status: implemented. The behaviour is the generic dumbfire rocket driven by the data: the
//! splash has `hits_owner`, so firing at a wall or a ship closer than the 8 u blast hurts the
//! shooter; the 3 u crater (scaled by rock strength) breaks through walls up to about 2.5 u
//! thick, leaving an opening wide enough for a ship between two tunnels.
//!
//! This file also holds `testkit`, the small world harness the Ordnance II modules share in
//! their scenario tests.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Heavy Rocket.
#[derive(Debug)]
pub struct HeavyRocket;

impl WeaponBehavior for HeavyRocket {
    fn id(&self) -> &'static str {
        "heavy_rocket"
    }

    fn implemented(&self) -> bool {
        true
    }
}

/// World harness for the Ordnance II scenario tests (heavy rocket, flash pod, mines,
/// tectonic charge).
#[cfg(test)]
pub(crate) mod testkit {
    use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
    use ls_math::{Quat, Vec3};

    use crate::*;

    /// Two-team Paint the Void world with the given Instability and rules.
    pub fn world_with(instability: u8, rules: RuleSet) -> World {
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
            instability,
            absurdity: 0,
            heist: None,
        };
        World::new(WorldConfig {
            rules,
            ..WorldConfig::paint_the_void(7, setup)
        })
        .unwrap()
    }

    pub fn world() -> World {
        world_with(0, RuleSet::paint_the_void())
    }

    /// A closed box room of full size `size` centred on the origin.
    pub fn room(size: f32) -> BoxRoomTerrain {
        BoxRoomTerrain::centered(Vec3::splat(size))
    }

    pub fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
        let s = w.ship_mut(id).unwrap();
        s.pos = pos;
        s.vel = Vec3::ZERO;
        s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
    }

    pub fn run(
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

    /// Steps until `done` holds for the events of a tick (at most `max` ticks); returns
    /// every event.
    pub fn run_until(
        w: &mut World,
        t: &mut BoxRoomTerrain,
        max: u32,
        done: impl Fn(&[Event]) -> bool,
    ) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..max {
            let e = w.step(t, &[]);
            let stop = done(&e);
            ev.extend(e);
            if stop {
                return ev;
            }
        }
        panic!("condition not reached in {max} ticks");
    }

    pub fn fire2(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
        [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_SECONDARY))]
    }

    /// Gives `weapon` (full ammo) to `p` and selects it.
    pub fn equip(w: &mut World, p: PlayerId, weapon: &str) -> WeaponId {
        let wid = w.weapons().find(weapon).unwrap();
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
        wid
    }

    pub fn damage_to(ev: &[Event], who: PlayerId) -> f32 {
        ev.iter()
            .filter_map(|e| match e {
                Event::Damaged { target, amount, .. } if *target == who => Some(*amount),
                _ => None,
            })
            .sum()
    }

    pub fn killed(ev: &[Event], who: PlayerId) -> bool {
        ev.iter()
            .any(|e| matches!(e, Event::Killed { victim, .. } if *victim == who))
    }

    pub fn terrain_edits(ev: &[Event]) -> Vec<TerrainEdit> {
        ev.iter()
            .filter_map(|e| match e {
                Event::TerrainEdit(t) => Some(*t),
                _ => None,
            })
            .collect()
    }

    pub fn impacts(ev: &[Event]) -> Vec<(u32, Vec3)> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Impact {
                    projectile, point, ..
                } => Some((*projectile, *point)),
                _ => None,
            })
            .collect()
    }

    pub fn cues(ev: &[Event], kind: CueKind) -> Vec<Cue> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Cue(c) if c.kind == kind => Some(*c),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{TerrainEdit, TerrainQuery};
    use ls_math::Vec3;

    use super::testkit::*;
    use crate::weapons::behavior;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn numbers_follow_the_design_doc() {
        let t = WeaponTable::builtin();
        let d = t.get(t.find("heavy_rocket").unwrap());
        assert_eq!(d.speed, 20.0, "20 u/s: slower than every other missile");
        assert_eq!(d.damage, 70.0, "M2 damage scale");
        let splash = d.splash.unwrap();
        assert_eq!(splash.radius, 8.0, "8 u blast");
        assert!(splash.hits_owner, "hurts you at close range");
        assert_eq!(d.terrain.crater_radius, 3.0, "6 u crater (diameter)");
        let bolt = t.get(t.find("bolt_rocket").unwrap());
        assert!(d.speed < bolt.speed && d.damage > bolt.damage);
        assert!(behavior("heavy_rocket").implemented());
    }

    /// Fires a heavy rocket from `dist` u in front of a wall; returns the owner's damage.
    fn self_damage_at(dist: f32) -> f32 {
        let mut w = world();
        let mut t = room(60.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "heavy_rocket");
        // The -X wall is at x = -30.
        place(&mut w, a, Vec3::new(-30.0 + dist, 0.0, 0.0), Vec3::NEG_X);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        ev.extend(run(&mut w, &mut t, &[], 120));
        assert_eq!(impacts(&ev).len(), 1);
        damage_to(&ev, a)
    }

    #[test]
    fn hurts_the_shooter_at_close_range_only() {
        let close = self_damage_at(3.0);
        assert!(close > 20.0, "point-blank blast hurts: {close}");
        assert_eq!(self_damage_at(12.0), 0.0, "outside the 8 u blast");
    }

    #[test]
    fn direct_hit_and_blast_hurt_enemies() {
        let mut w = world();
        let mut t = room(60.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let c = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, a, "heavy_rocket");
        place(&mut w, a, Vec3::new(0.0, 0.0, 15.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        place(&mut w, c, Vec3::new(5.0, 0.0, 0.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        // 15 u at 20 u/s: about 0.7 s of flight.
        ev.extend(run(&mut w, &mut t, &[], 60));
        assert!(damage_to(&ev, b) >= 70.0 - 1e-3, "direct hit");
        let side = damage_to(&ev, c);
        assert!(side > 5.0 && side < 50.0, "blast falls off: {side}");
        assert_eq!(damage_to(&ev, a), 0.0);
    }

    #[test]
    fn crater_opens_a_passage_between_tunnels() {
        // A second tunnel runs parallel to the room's -X wall, 2 u of rock away.
        let open_after = |weapon: &str| {
            let mut w = world();
            let mut t = room(60.0);
            let wall = -30.0f32;
            for z in -10..=10 {
                t.apply(TerrainEdit::SubtractSphere {
                    center: Vec3::new(wall - 2.0 - 3.0, 0.0, z as f32),
                    radius: 3.0,
                });
            }
            let probe = |t: &ls_math::terrain::BoxRoomTerrain| {
                // Open for a ship (radius 0.5) all the way from the room into the tunnel?
                (0..=40).all(|i| {
                    let x = wall + 2.0 - i as f32 * 0.2;
                    t.sdf(Vec3::new(x, 0.0, 0.0)) > 0.5
                })
            };
            assert!(!probe(&t), "sealed before the shot");
            let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
            equip(&mut w, a, weapon);
            place(&mut w, a, Vec3::new(wall + 15.0, 0.0, 0.0), Vec3::NEG_X);
            run(&mut w, &mut t, &fire2(a), 1);
            run(&mut w, &mut t, &[], 90);
            probe(&t)
        };
        assert!(open_after("heavy_rocket"), "the 6 u crater breaks through");
        assert!(!open_after("bolt_rocket"), "a 2 u crater does not");
    }
}
