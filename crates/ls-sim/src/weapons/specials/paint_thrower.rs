//! Paint Thrower: special. Design doc: "Reshapes the arena: red is bouncy, blue is slippery,
//! yellow is sticky and slows ships. Malus: deals no damage. Lasting effect: paint stays until
//! painted over."
//!
//! Numbers: `data/weapons/paint_thrower.ron` (the colour effects' numbers are ship tuning,
//! `data/ships/*.ron`: `paint_bounce_restitution`, `paint_sticky_speed`).
//!
//! Behaviour (M2 plan decision "Paint Thrower colours"):
//! - Colour: the V key (`Buttons::WEAPON_MODE`, [`WeaponBehavior::on_mode`] while the Paint
//!   Thrower is the selected primary) cycles red, blue, yellow. The colour is the weapon's
//!   state (`ShipState::weapon_state`, `ls_math::terrain::PAINT_*`; unset = red); every switch
//!   sends a `CueKind::PaintColor` cue (source the ship, value the colour) for the HUD.
//! - Shots: the generic pattern, no damage, crater or knockback; each blob carries the colour
//!   in its replicated `CUSTOM_A`/`CUSTOM_B` bits ([`projectile_color`], so clients tint it).
//! - Paint: a blob that hits rock claims the surface within `terrain.paint_radius` for the
//!   shooter's team with its colour (`TerrainEdit::Paint { team, color }`); Paint the Void
//!   scoring counts the team only. Painting over replaces team and colour.
//! - Effects (ship physics, `ls_sim::ship`): red paint bounces ships off at 0.9 restitution,
//!   blue paint has no wall friction (no damping while touching it, wall hits keep the speed),
//!   yellow paint holds ships touching it to 40% of their speed caps. They apply to every ship,
//!   the painter's team included.
//!
//! This file also holds `testkit`, the shared test helpers of the specials modules.

use ls_math::terrain::{RayHit, PAINT_BLUE, PAINT_RED, PAINT_YELLOW};

use crate::behavior::{Ctx, Cue, CueKind};
use crate::ship::ShipState;
use crate::weapons::{pattern_dir, projectile_for, HitResponse, Shot, WeaponBehavior};
use crate::world::{proj_flags, Projectile};
use crate::WeaponId;

/// Behaviour of the Paint Thrower.
#[derive(Debug)]
pub struct PaintThrower;

/// The colour `ship` paints with (its Paint Thrower state; red when unset).
pub fn selected_color(ship: &ShipState, weapon: WeaponId) -> u8 {
    match u8::try_from(ship.weapon_state_of(weapon)) {
        Ok(c @ (PAINT_RED | PAINT_BLUE | PAINT_YELLOW)) => c,
        _ => PAINT_RED,
    }
}

/// The colour after `color` in the V cycle (red, blue, yellow).
pub fn next_color(color: u8) -> u8 {
    match color {
        PAINT_RED => PAINT_BLUE,
        PAINT_BLUE => PAINT_YELLOW,
        _ => PAINT_RED,
    }
}

/// Paint colour a blob carries (its replicated `CUSTOM_A` / `CUSTOM_B` bits).
pub fn projectile_color(p: &Projectile) -> u8 {
    (p.flags & proj_flags::CUSTOM_A != 0) as u8 | ((p.flags & proj_flags::CUSTOM_B != 0) as u8) << 1
}

fn color_flags(color: u8) -> u16 {
    let mut f = 0;
    if color & 1 != 0 {
        f |= proj_flags::CUSTOM_A;
    }
    if color & 2 != 0 {
        f |= proj_flags::CUSTOM_B;
    }
    f
}

impl WeaponBehavior for PaintThrower {
    fn id(&self) -> &'static str {
        "paint_thrower"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let flags = color_flags(selected_color(shot.shooter, shot.weapon));
        let n = shot.def.projectiles.max(1) as usize;
        for i in 0..n {
            let dir = pattern_dir(shot.shooter.orientation, i, n, shot.def.spread_deg);
            let mut p = projectile_for(ctx, shot, dir);
            p.flags |= flags;
            ctx.spawn_projectile(p);
        }
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, false);
        let radius = ctx.def(p.weapon).terrain.paint_radius;
        ctx.paint_color(hit.point, radius, p.team, projectile_color(p));
        HitResponse::Stop
    }

    fn on_mode(&self, ctx: &mut Ctx, ship: &ShipState, weapon: WeaponId) {
        let color = next_color(selected_color(ship, weapon));
        ctx.set_weapon_state(ship.player, weapon, color as u32);
        ctx.cue(Cue {
            kind: CueKind::PaintColor,
            source: Some(ship.player),
            target: None,
            pos: ship.pos,
            value: color as u16,
        });
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
            heist: None,
            ..Default::default()
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

    /// M2 plan: V cycles the colour red, blue, yellow; blobs carry it and
    /// paint with it; each switch cues the HUD; scoring counts the team.
    #[test]
    fn v_cycles_the_paint_colour() {
        use ls_math::terrain::{PAINT_BLUE, PAINT_RED, PAINT_YELLOW};
        let (mut w, a, b) = world2();
        let mut t = PaintedTerrain::new(BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0)), 1.0);
        let wid = equip(&mut w, a, "paint_thrower");
        place(&mut w, a, Vec3::new(-25.0, 0.0, 0.0), Vec3::NEG_X);
        place(&mut w, b, Vec3::new(0.0, 0.0, 20.0), Vec3::Z);
        let wall = Vec3::new(-30.0, 0.0, 0.0);
        let paint_colors = |ev: &[Event]| -> Vec<u8> {
            edits(ev)
                .iter()
                .map(|e| match e {
                    TerrainEdit::Paint { color, team, .. } => {
                        assert_eq!(*team, 0);
                        *color
                    }
                    other => panic!("only paint, got {other:?}"),
                })
                .collect()
        };
        // Red by default.
        assert_eq!(super::selected_color(w.ship(a).unwrap(), wid), PAINT_RED);
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 30);
        let c = paint_colors(&ev);
        assert!(!c.is_empty() && c.iter().all(|&c| c == PAINT_RED), "{c:?}");
        assert_eq!(t.paint_color_at(wall + Vec3::X * 0.5), Some(PAINT_RED));
        // V (press edges only) cycles blue, yellow, red.
        let mut seen = Vec::new();
        for _ in 0..3 {
            let ev = run(&mut w, &mut t, &press(a, Buttons::WEAPON_MODE), 3);
            let cue = cues(&ev, CueKind::PaintColor);
            assert_eq!(cue.len(), 1, "one switch per press");
            assert_eq!(cue[0].source, Some(a));
            seen.push(cue[0].value as u8);
            assert_eq!(
                super::selected_color(w.ship(a).unwrap(), wid),
                cue[0].value as u8
            );
            run(&mut w, &mut t, &[], 1);
        }
        assert_eq!(seen, [PAINT_BLUE, PAINT_YELLOW, PAINT_RED]);
        // Blue: the blobs carry it (replicated bits) and paint over the red.
        run(&mut w, &mut t, &press(a, Buttons::WEAPON_MODE), 1);
        run(&mut w, &mut t, &[], 1);
        let fire = press(a, Buttons::FIRE_PRIMARY);
        run(&mut w, &mut t, &fire, 1);
        let blob = w.projectiles().last().expect("a blob in flight");
        assert_eq!(super::projectile_color(blob), PAINT_BLUE);
        assert_eq!(blob.net_state() >> 6, PAINT_BLUE);
        let ev = run(&mut w, &mut t, &fire, 30);
        assert!(paint_colors(&ev).iter().all(|&c| c == PAINT_BLUE));
        assert_eq!(t.paint_color_at(wall + Vec3::X * 0.5), Some(PAINT_BLUE));
        // V with another primary selected does nothing to the colour.
        equip(&mut w, a, "scrap_laser");
        let ev = run(&mut w, &mut t, &press(a, Buttons::WEAPON_MODE), 3);
        assert!(cues(&ev, CueKind::PaintColor).is_empty());
        assert_eq!(super::selected_color(w.ship(a).unwrap(), wid), PAINT_BLUE);
        // Scoring counts cells per team, whatever the colour.
        let cov = t.paint_coverage().unwrap();
        assert!(cov.team_cells[0] > 0 && cov.team_cells[1] == 0);
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
