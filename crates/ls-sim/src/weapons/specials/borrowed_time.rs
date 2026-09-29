//! Borrowed Time: special secondary. Rewinds your position, velocity and hull by 3 s. Malus:
//! weapons offline for 3 s; a ghost stays visible at the old spot for 3 s. Lasting effect:
//! terrain damage is not rewound, so the scars stay.
//!
//! Numbers: `data/weapons/borrowed_time.ron` (`params`: `rewind_secs`, `offline_secs`,
//! `ghost_secs`).
//!
//! Status: implemented. `fire` spawns no real shot: it leaves a `Ghost` entity at the spot
//! the ship leaves (for `ghost_secs`), rewinds the ship's position, velocity and hull by
//! `rewind_secs` from the per-ship history the world keeps (`ctx.trail`, 3 s; a ship with
//! less history goes back to its oldest record), takes both weapon slots offline for
//! `offline_secs` and cues `Rewound` (target = shooter, `pos` = the ghost spot, `value` =
//! ticks rewound). Terrain, projectiles and everyone else stay where they are. A motionless,
//! harmless "time pulse" projectile marks the old spot for the client (and carries the
//! `Fired` event).

use ls_math::{Tick, Vec3};

use crate::behavior::{Ctx, Cue, CueKind};
use crate::entity::{EntitySpawn, Source};
use crate::weapons::{projectile_for, Shot, WeaponBehavior};
use crate::world::proj_flags;

/// Behaviour of the Borrowed Time.
#[derive(Debug)]
pub struct BorrowedTime;

impl WeaponBehavior for BorrowedTime {
    fn id(&self) -> &'static str {
        "borrowed_time"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let s = shot.shooter;
        let def = shot.def;
        let rewind = Tick::ticks_for_secs(def.param("rewind_secs", 3.0));
        let offline = Tick::ticks_for_secs(def.param("offline_secs", 3.0));
        let ghost = Tick::ticks_for_secs(def.param("ghost_secs", 3.0));
        let old = s.pos;

        let mut pulse = projectile_for(ctx, shot, s.orientation.forward());
        pulse.pos = old;
        pulse.vel = Vec3::ZERO;
        pulse.damage = 0.0;
        pulse.splash_damage = 0.0;
        pulse.knockback = 0.0;
        pulse.flags |= proj_flags::NO_SHIP_HITS;
        ctx.spawn_projectile(pulse);

        ctx.spawn_entity(
            EntitySpawn::ghost(s.player, s.team, old, s.orientation, ghost)
                .from_source(Source::Weapon(shot.weapon)),
        );
        ctx.rewind(s.player, rewind);
        ctx.disable_weapons(s.player, offline);
        ctx.cue(Cue {
            kind: CueKind::Rewound,
            source: Some(s.player),
            target: Some(s.player),
            pos: old,
            value: rewind.min(u16::MAX as u64) as u16,
        });
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{TerrainEdit, TerrainQuery};
    use ls_math::Vec3;

    use super::super::paint_thrower::testkit::*;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Flies `a` sideways for `n` ticks, recording (pos, vel, hull) after each.
    fn fly(w: &mut World, t: &mut TestRoom, a: PlayerId, n: u32, log: &mut Vec<(Vec3, Vec3, f32)>) {
        let cmd = [(a, InputCmd::idle(0).with_thrust(Vec3::new(1.0, 0.0, 0.0)))];
        for _ in 0..n {
            w.step(t, &cmd);
            let s = w.ship(a).unwrap();
            log.push((s.pos, s.vel, s.hull));
        }
    }

    #[test]
    fn rewinds_position_velocity_and_hull_by_three_seconds() {
        let (mut w, a, _b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "borrowed_time");
        place(&mut w, a, Vec3::new(-25.0, 0.0, 10.0), Vec3::NEG_Z);
        let mut log = Vec::new();
        fly(&mut w, &mut t, a, 100, &mut log);
        w.ship_mut(a).unwrap().hull = 40.0;
        fly(&mut w, &mut t, a, 150, &mut log);
        let before = w.ship(a).unwrap().pos;
        let n = log.len();
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        let s = w.ship(a).unwrap();
        let (pos, vel, hull) = log[n - 1 - 180];
        assert!(s.pos.distance(pos) < 1e-4, "{:?} vs {pos:?}", s.pos);
        assert!(s.vel.distance(vel) < 1e-4);
        assert_eq!(s.hull, hull);
        assert_eq!(hull, 100.0, "hull from before the damage");
        assert!(before.x - s.pos.x > 10.0, "went back along its path");
        let c = cues(&ev, CueKind::Rewound);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].target, Some(a));
        assert!(c[0].pos.distance(before) < 0.5, "the spot it left");
        assert_eq!(c[0].value, 180);
        assert_eq!(fired(&ev), 1);
    }

    #[test]
    fn weapons_go_offline_and_a_ghost_stays_at_the_old_spot() {
        let (mut w, a, _b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "borrowed_time");
        place(&mut w, a, Vec3::new(-25.0, 0.0, 10.0), Vec3::NEG_Z);
        let mut log = Vec::new();
        fly(&mut w, &mut t, a, 200, &mut log);
        let old = w.ship(a).unwrap().pos;
        run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        let ghosts: Vec<Entity> = w
            .entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Ghost)
            .cloned()
            .collect();
        assert_eq!(ghosts.len(), 1);
        assert!(ghosts[0].pos.distance(old) < 0.5, "at the spot it left");
        assert_eq!(ghosts[0].owner, Some(a));

        // Primary (Scrap Laser) is offline for 3 s.
        let both = press(a, Buttons::FIRE_PRIMARY | Buttons::FIRE_SECONDARY);
        let ev = run(&mut w, &mut t, &both, 178);
        assert_eq!(fired(&ev), 0, "weapons offline");
        assert_eq!(
            w.entities()
                .iter()
                .filter(|e| e.kind == EntityKind::Ghost)
                .count(),
            1,
            "ghost visible for 3 s"
        );
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 4);
        assert!(fired(&ev) >= 1, "back online after 3 s");
        assert!(w.entities().iter().all(|e| e.kind != EntityKind::Ghost));
    }

    #[test]
    fn terrain_damage_is_not_rewound() {
        let (mut w, a, _b) = world2();
        let mut t = TestRoom::new();
        let rocket = equip(&mut w, a, "bolt_rocket");
        place(&mut w, a, Vec3::new(-25.0, 0.0, 10.0), Vec3::NEG_Z);
        let mut log = Vec::new();
        fly(&mut w, &mut t, a, 60, &mut log);
        // Blast the far wall, then rewind past the shot.
        run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        fly(&mut w, &mut t, a, 90, &mut log);
        let craters: Vec<TerrainEdit> = t
            .edits
            .iter()
            .filter(|e| matches!(e, TerrainEdit::SubtractSphere { .. }))
            .copied()
            .collect();
        assert!(!craters.is_empty(), "the rocket dug a crater");
        let ammo = w.ship(a).unwrap().weapon(rocket).unwrap().ammo;
        equip(&mut w, a, "borrowed_time");
        run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        run(&mut w, &mut t, &[], 60);
        let c = craters[0];
        assert!(t.sdf(c.center()) > 0.0, "the crater is still open");
        assert_eq!(
            t.edits
                .iter()
                .filter(|e| matches!(e, TerrainEdit::SubtractSphere { .. }))
                .count(),
            craters.len()
        );
        assert_eq!(
            w.ship(a).unwrap().weapon(rocket).unwrap().ammo,
            ammo,
            "spent ammo stays spent"
        );
    }
}
