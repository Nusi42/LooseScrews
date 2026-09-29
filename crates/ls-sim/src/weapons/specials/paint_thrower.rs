//! Paint Thrower: special. Deals no damage; paints terrain in the shooter's team color (the
//! core tool of Paint the Void). Surface effects of paint colors come in a later milestone.
//!
//! Numbers: `data/weapons/paint_thrower.ron`.
//!
//! Status: implemented. The generic hooks are the M1 behaviour: no damage, no crater, no
//! knockback, paints `terrain.paint_radius` in the team colour; paint stays until painted
//! over (it is an ordinary logged terrain edit). The scenario tests below check this in the
//! M2 framework.
//! Paint colours with surface effects (red bouncy, blue slippery, yellow sticky) are a later
//! milestone.
//!
//! This file also holds `testkit`, the shared test helpers of the specials modules.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Paint Thrower.
#[derive(Debug)]
pub struct PaintThrower;

impl WeaponBehavior for PaintThrower {
    fn id(&self) -> &'static str {
        "paint_thrower"
    }

    fn implemented(&self) -> bool {
        true
    }
}

/// Test helpers shared by the specials modules' scenario tests.
#[cfg(test)]
pub(crate) mod testkit {
    use ls_math::terrain::{TerrainEdit, TerrainQuery, MATERIAL_ROCK, MATERIAL_WEAK};
    use ls_math::{Aabb, Quat, Vec3};

    use crate::*;

    /// Two spawns, team 0 at x = -20 and team 1 at x = 20.
    pub fn setup() -> MatchSetup {
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
        }
    }

    /// Built-in data, Paint the Void rules.
    pub fn world() -> World {
        World::new(WorldConfig::paint_the_void(7, setup())).unwrap()
    }

    /// Two ships (team 0, team 1).
    pub fn world2() -> (World, PlayerId, PlayerId) {
        let mut w = world();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        (w, a, b)
    }

    pub fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
        let s = w.ship_mut(id).unwrap();
        s.pos = pos;
        s.vel = Vec3::ZERO;
        s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
    }

    /// Gives weapon `id` to `p` directly (full ammo) and selects it.
    pub fn equip(w: &mut World, p: PlayerId, id: &str) -> WeaponId {
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
        wid
    }

    pub fn run<T: SimTerrain + ?Sized>(
        w: &mut World,
        t: &mut T,
        inputs: &[(PlayerId, InputCmd)],
        n: u32,
    ) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..n {
            ev.extend(w.step(t, inputs));
        }
        ev
    }

    pub fn press(id: PlayerId, b: Buttons) -> [(PlayerId, InputCmd); 1] {
        [(id, InputCmd::idle(0).with_buttons(b))]
    }

    pub fn damage_to(ev: &[Event], who: PlayerId) -> f32 {
        ev.iter()
            .filter_map(|e| match e {
                Event::Damaged { target, amount, .. } if *target == who => Some(*amount),
                _ => None,
            })
            .sum()
    }

    pub fn cues(ev: &[Event], kind: CueKind) -> Vec<Cue> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Cue(c) if c.kind == kind => Some(*c),
                _ => None,
            })
            .collect()
    }

    pub fn edits(ev: &[Event]) -> Vec<TerrainEdit> {
        ev.iter()
            .filter_map(|e| match e {
                Event::TerrainEdit(t) => Some(*t),
                _ => None,
            })
            .collect()
    }

    pub fn fired(ev: &[Event]) -> usize {
        ev.iter()
            .filter(|e| matches!(e, Event::Fired { .. }))
            .count()
    }

    /// A 60 x 30 x 60 room (open inside) with optional solid slabs (walls)
    /// and weak rock for `x > weak_x`; geometry edits apply on top.
    #[derive(Debug, Clone)]
    pub struct TestRoom {
        pub room: Aabb,
        pub walls: Vec<Aabb>,
        pub weak_x: f32,
        pub edits: Vec<TerrainEdit>,
    }

    impl TestRoom {
        pub fn new() -> TestRoom {
            TestRoom {
                room: Aabb::from_center_half_extents(Vec3::ZERO, Vec3::new(30.0, 15.0, 30.0)),
                walls: Vec::new(),
                weak_x: f32::INFINITY,
                edits: Vec::new(),
            }
        }

        /// Adds a wall slab `thickness` thick across the room at `z`.
        pub fn with_wall_z(mut self, z: f32, thickness: f32) -> TestRoom {
            self.walls.push(Aabb::from_center_half_extents(
                Vec3::new(0.0, 0.0, z),
                Vec3::new(30.0, 15.0, thickness * 0.5),
            ));
            self
        }

        pub fn with_weak_x(mut self, x: f32) -> TestRoom {
            self.weak_x = x;
            self
        }
    }

    impl TerrainQuery for TestRoom {
        fn sdf(&self, p: Vec3) -> f32 {
            let mut d = -self.room.sdf(p);
            for w in &self.walls {
                d = d.min(w.sdf(p));
            }
            for e in &self.edits {
                d = e.apply_to_sdf(d, p);
            }
            d
        }
        fn bounds(&self) -> Aabb {
            self.room
        }
    }

    impl SimTerrain for TestRoom {
        fn apply_edit(&mut self, edit: &TerrainEdit) {
            self.edits.push(*edit);
        }
        fn material_at(&self, p: Vec3) -> u8 {
            if p.x > self.weak_x {
                MATERIAL_WEAK
            } else {
                MATERIAL_ROCK
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
    use ls_math::Vec3;

    use super::testkit::*;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn paints_team_colour_without_damage_crater_or_knockback() {
        let (mut w, a, b) = world2();
        let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)), 1.0);
        let wid = equip(&mut w, a, "paint_thrower");
        assert!(crate::weapons::behavior("paint_thrower").implemented());
        let def = w.weapons().get(wid).clone();
        assert_eq!(def.damage, 0.0);
        assert_eq!(def.terrain.crater_radius, 0.0);

        // Bonus: paints the wall in the shooter's team colour.
        place(&mut w, a, Vec3::new(-25.0, 0.0, 0.0), Vec3::NEG_X);
        place(&mut w, b, Vec3::new(0.0, 0.0, 20.0), Vec3::Z);
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 30);
        let ed = edits(&ev);
        assert!(!ed.is_empty());
        for e in &ed {
            match e {
                TerrainEdit::Paint { team, radius, .. } => {
                    assert_eq!(*team, paint_team_id(0));
                    assert_eq!(*radius, def.terrain.paint_radius);
                }
                other => panic!("only paint, got {other:?}"),
            }
        }
        let cov = t.paint_coverage().unwrap();
        assert!(cov.team_cells[0] > 0 && cov.team_cells[1] == 0);

        // Malus: a direct hit deals no damage and pushes nothing.
        place(&mut w, a, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 20);
        assert!(ev
            .iter()
            .any(|e| matches!(e, Event::Impact { target: Some(t), .. } if *t == b)));
        assert_eq!(damage_to(&ev, b), 0.0);
        assert_eq!(w.ship(b).unwrap().hull, 100.0);
        assert_eq!(w.ship(b).unwrap().vel, Vec3::ZERO);

        // Lasting effect: the paint stays until painted over.
        run(&mut w, &mut t, &[], 600);
        let later = t.paint_coverage().unwrap();
        assert!(later.team_cells[0] >= cov.team_cells[0]);
    }

    #[test]
    fn enemy_paint_paints_over() {
        let (mut w, a, b) = world2();
        let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)), 1.0);
        equip(&mut w, a, "paint_thrower");
        equip(&mut w, b, "paint_thrower");
        place(&mut w, a, Vec3::new(-25.0, 0.0, 0.0), Vec3::NEG_X);
        place(&mut w, b, Vec3::new(0.0, 0.0, 25.0), Vec3::Z);
        run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 20);
        run(&mut w, &mut t, &[], 60);
        let before = t.paint_coverage().unwrap();
        assert!(before.team_cells[0] > 0);
        // Team 1 paints the same spot.
        place(&mut w, a, Vec3::new(0.0, 0.0, -25.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(-25.0, 0.0, 0.0), Vec3::NEG_X);
        run(&mut w, &mut t, &press(b, Buttons::FIRE_PRIMARY), 20);
        run(&mut w, &mut t, &[], 60);
        let after = t.paint_coverage().unwrap();
        assert!(after.team_cells[1] > 0);
        assert!(after.team_cells[0] < before.team_cells[0], "painted over");
    }
}
