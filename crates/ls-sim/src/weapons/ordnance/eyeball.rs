//! Eyeball: ordnance secondary. You steer it through its camera, 25 u/s. Malus: your ship is
//! exposed while you steer (it drifts without thrust). Lasting effect: 4 u crater.
//!
//! Numbers: `data/weapons/eyeball.ron` (`params`: `turn_rate`, `roll_rate`; crater radius 2.0
//! = 4 u diameter).
//!
//! Behaviour (M2 plan decision "Eyeball"):
//! - `fire` launches the missile with `proj_flags::CONTROLS_OWNER`: while it lives the world
//!   gives the owner's ship no thrust and no look (it drifts: the malus), and the client shows
//!   the missile's camera (wave C, from the replicated guided bit).
//! - `update` steers it with the owner's look input exactly like a ship turns: pitch and yaw
//!   deltas (each clamped to `turn_rate` rad/s) and roll (`roll_rate` rad/s at full input)
//!   rotate the missile's own orientation, kept in `vars` as a quaternion, and the velocity
//!   follows its forward axis at constant speed. Without input this tick it flies straight.
//! - While it is guided the world does not fire the owner's secondary, so a pilot never
//!   guides two eyeballs at once.
//! - If the owner dies, the missile stops listening (clears the guided bit) and flies on
//!   straight, so the respawned ship is not frozen.
//! - Impacts are the generic ones: direct hit, 3 u splash, a 4 u (diameter) crater.

use ls_math::{Quat, Vec3, DT};

use crate::behavior::Ctx;
use crate::weapons::{projectile_for, Shot, WeaponBehavior};
use crate::world::{proj_flags, Projectile};

/// Behaviour of the Eyeball.
#[derive(Debug)]
pub struct Eyeball;

fn orientation(p: &Projectile) -> Quat {
    Quat::from_xyzw(p.vars[0], p.vars[1], p.vars[2], p.vars[3])
}

fn set_orientation(p: &mut Projectile, q: Quat) {
    p.vars = [q.x, q.y, q.z, q.w];
}

impl WeaponBehavior for Eyeball {
    fn id(&self) -> &'static str {
        "eyeball"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let q = shot.shooter.orientation;
        let mut p = projectile_for(ctx, shot, q.forward());
        p.flags |= proj_flags::CONTROLS_OWNER;
        set_orientation(&mut p, q);
        ctx.spawn_projectile(p);
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if p.flags & proj_flags::CONTROLS_OWNER == 0 {
            return;
        }
        if !ctx.ship(p.owner).is_some_and(|s| s.alive) {
            p.flags &= !proj_flags::CONTROLS_OWNER;
            return;
        }
        let Some(input) = ctx.input(p.owner) else {
            return;
        };
        let def = ctx.def(p.weapon);
        let max = def.param("turn_rate", 3.0) * DT;
        let pitch = input.look_pitch.clamp(-max, max);
        let yaw = input.look_yaw.clamp(-max, max);
        let roll = input.roll * def.param("roll_rate", 2.5);
        let omega = Vec3::new(pitch / DT, yaw / DT, roll);
        let q = orientation(p).integrate_local(omega, DT).normalize();
        set_orientation(p, q);
        p.vel = q.forward() * p.vel.length();
    }
}

#[cfg(test)]
mod tests {
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn look(id: PlayerId, pitch: f32, yaw: f32) -> [(PlayerId, InputCmd); 1] {
        [(
            id,
            InputCmd {
                look_pitch: pitch,
                look_yaw: yaw,
                ..InputCmd::idle(0)
            }
            .with_thrust(Vec3::NEG_Z),
        )]
    }

    /// Bonus: the owner's look input steers it (25 u/s, turn rate capped)
    /// around a corner onto a target a dumbfire could never reach.
    #[test]
    fn steered_by_the_owners_look() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "eyeball");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        let target = Vec3::new(-12.0, 0.0, 0.0);
        place(&mut w, b, target, Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        let p = &w.projectiles()[0];
        assert_eq!(p.vel.length(), 25.0);
        assert_ne!(p.flags & proj_flags::CONTROLS_OWNER, 0);
        // A huge yaw is clamped to 3 rad/s: 0.05 rad per tick.
        let v0 = w.projectiles()[0].vel;
        run(&mut w, &mut t, &look(a, 0.0, 1.0), 1);
        let v1 = w.projectiles()[0].vel;
        let turned = v0.normalize().dot(v1.normalize()).clamp(-1.0, 1.0).acos();
        assert!((turned - 0.05).abs() < 1e-3, "{turned}");
        assert!(v1.x < 0.0, "positive yaw turns left");
        // Fly on straight, then steer towards the target like a pilot would.
        for _ in 0..200 {
            let Some(p) = w.projectiles().first() else {
                break;
            };
            let to = (target - p.pos).normalize();
            let fwd = p.vel.normalize();
            let yaw = if fwd.cross(to).y > 0.0 { 0.05 } else { -0.05 };
            let yaw = if fwd.dot(to) > 0.999 { 0.0 } else { yaw };
            ev.extend(run(&mut w, &mut t, &look(a, 0.0, yaw), 1));
            place(&mut w, b, target, Vec3::Z);
        }
        assert!(hits_on(&ev, b).contains(&40.0), "{:?}", hits_on(&ev, b));
        assert!(w.projectiles().is_empty());
    }

    /// Malus: while it flies the pilot's ship gets no thrust or look: it
    /// only drifts, exposed. Control comes back after the impact.
    #[test]
    fn owner_is_frozen_while_guiding() {
        let mut w = world();
        let mut t = room();
        let (a, _) = duel(&mut w);
        equip(&mut w, a, "eyeball");
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        w.ship_mut(a).unwrap().vel = Vec3::new(3.0, 0.0, 0.0);
        run(&mut w, &mut t, &fire2(a), 1);
        let rot = w.ship(a).unwrap().orientation;
        let z = w.ship(a).unwrap().pos.z;
        run(&mut w, &mut t, &look(a, 0.02, 0.02), 30);
        let s = w.ship(a).unwrap();
        assert!(s.orientation.angle_between(rot) < 1e-5, "no look");
        assert!((s.pos.z - z).abs() < 1e-3, "no thrust: {}", s.pos.z - z);
        assert!(s.pos.x > 0.0, "drifts on");
        // The eyeball hits the far wall (40 u at 25 u/s), then control is back.
        run(&mut w, &mut t, &[], 90);
        assert!(w.projectiles().is_empty());
        run(&mut w, &mut t, &look(a, 0.0, 0.0), 20);
        assert!(w.ship(a).unwrap().pos.z < z - 0.5, "thrust works again");
    }

    /// Holding the secondary while guiding launches no second eyeball: a
    /// pilot guides one at a time.
    #[test]
    fn one_guided_eyeball_at_a_time() {
        let mut w = world();
        let mut t = room_sized(Vec3::new(60.0, 30.0, 400.0));
        let (a, _) = duel(&mut w);
        equip(&mut w, a, "eyeball");
        place(&mut w, a, Vec3::new(0.0, 0.0, 150.0), Vec3::NEG_Z);
        // 1 s cooldown; hold for 70 ticks while the first one is still flying.
        let ev = run(&mut w, &mut t, &fire2(a), 70);
        assert_eq!(fired(&ev), 1);
        let guided = w
            .projectiles()
            .iter()
            .filter(|p| p.owner == a && p.flags & proj_flags::CONTROLS_OWNER != 0)
            .count();
        assert_eq!(guided, 1);
    }

    /// If the pilot dies, the eyeball flies on straight and the respawned
    /// ship is free.
    #[test]
    fn owner_death_releases_it() {
        let mut w = world();
        let mut t = room_sized(Vec3::new(60.0, 30.0, 400.0));
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "eyeball");
        place(&mut w, a, Vec3::new(0.0, 0.0, 150.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(20.0, 0.0, 150.0), Vec3::Z);
        run(&mut w, &mut t, &fire2(a), 1);
        w.ship_mut(a).unwrap().hull = 1.0;
        equip(&mut w, b, "railspike");
        place(&mut w, b, Vec3::new(20.0, 0.0, 150.0), Vec3::NEG_X);
        run(&mut w, &mut t, &fire1(b), 8);
        assert!(!w.ship(a).unwrap().alive);
        let p = w.projectiles().iter().find(|p| p.owner == a).unwrap();
        assert_eq!(p.flags & proj_flags::CONTROLS_OWNER, 0);
        let v = p.vel;
        run(&mut w, &mut t, &look(a, 0.0, 0.05), 10);
        let p = w.projectiles().iter().find(|p| p.owner == a).unwrap();
        assert_eq!(p.vel, v, "no longer steered");
    }

    /// Lasting effect: a 4 u (diameter) crater.
    #[test]
    fn four_unit_crater() {
        let mut w = world();
        let mut t = room();
        let (a, _) = duel(&mut w);
        equip(&mut w, a, "eyeball");
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &fire2(a), 40);
        let c = craters(&ev);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].1 * 2.0, 4.0);
    }
}
