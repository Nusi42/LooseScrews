//! Seeker: ordnance secondary. Homes on the nearest target in a 30 degree cone, 25 u/s.
//! Malus: can be outflown or fooled by the Decoy. Lasting effect: 2 u crater.
//!
//! Numbers: `data/weapons/seeker.ron` (`params`: `cone_deg` (full apex angle, so 15 degrees
//! either side of the flight direction), `turn_rate` (rad/s), `range`).
//!
//! Behaviour: every tick (`update`) the missile looks for the nearest target inside its cone
//! around its flight direction, within `range` and with a clear line of sight: enemy ships and
//! enemy Decoy entities alike, so a decoy closer to the missile than its pilot steals the lock
//! (fooled). It then turns towards it at most `turn_rate` rad/s at constant speed (pure
//! pursuit). A target that leaves the cone (or a pilot who simply flies away faster than
//! 25 u/s: boosting on the diagonal reaches 28.5 u/s) is lost; without a target it flies
//! straight. Impacts are the generic ones: direct hit, splash, 1 u radius crater.
//!
//! State: `target` is the locked ship (`None` while locked on a decoy); `vars[0]` is 1 while
//! locked on a decoy, `vars[1]` that decoy's entity id (its bits, `f32::from_bits`, so any
//! id is exact).

use ls_math::{Vec3, DT};

use crate::behavior::Ctx;
use crate::entity::EntityKind;
use crate::ship::PlayerId;
use crate::weapons::{steer_towards, WeaponBehavior};
use crate::world::Projectile;

/// Behaviour of the Seeker.
#[derive(Debug)]
pub struct Seeker;

/// What a homing projectile locked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    Ship(PlayerId),
    /// A Decoy entity (by entity id).
    Decoy(u32),
}

/// The nearest homing target for `p`: living enemy ships and enemy decoys
/// within `range` with a clear line of sight and, if `half_angle_deg` is
/// given, inside that cone around the flight direction. Ties go to ships
/// (in id order), then to entities in list order.
pub fn acquire(
    ctx: &Ctx,
    p: &Projectile,
    range: f32,
    half_angle_deg: Option<f32>,
) -> Option<(Lock, Vec3)> {
    let dir = p.vel.try_normalize();
    let cos = half_angle_deg.map(|a| ls_math::det::cos(a.to_radians()));
    let accept = |pos: Vec3| -> Option<f32> {
        let to = pos - p.pos;
        let d = to.length();
        if d > range || d <= 1e-4 {
            return None;
        }
        if let (Some(cos), Some(dir)) = (cos, dir) {
            if to.dot(dir) < cos * d {
                return None;
            }
        }
        if ctx.raycast(p.pos, to, d).is_some() {
            return None;
        }
        Some(d)
    };
    let mut best: Option<(Lock, Vec3, f32)> = None;
    let mut consider = |lock: Lock, pos: Vec3, d: f32| {
        if best.is_none_or(|(_, _, bd)| d < bd) {
            best = Some((lock, pos, d));
        }
    };
    for s in ctx.view.ships.values() {
        if !s.alive || s.team == p.team || s.player == p.owner {
            continue;
        }
        if let Some(d) = accept(s.pos) {
            consider(Lock::Ship(s.player), s.pos, d);
        }
    }
    for e in ctx.view.entities {
        if e.kind != EntityKind::Decoy || e.team == p.team || e.owner == Some(p.owner) {
            continue;
        }
        if let Some(d) = accept(e.pos) {
            consider(Lock::Decoy(e.id), e.pos, d);
        }
    }
    best.map(|(lock, pos, _)| (lock, pos))
}

/// Records `lock` in the projectile's generic state (see the module docs).
pub fn record_lock(p: &mut Projectile, lock: Option<Lock>) {
    match lock {
        Some(Lock::Ship(id)) => {
            p.target = Some(id);
            p.vars[0] = 0.0;
        }
        Some(Lock::Decoy(id)) => {
            p.target = None;
            p.vars[0] = 1.0;
            p.vars[1] = f32::from_bits(id);
        }
        None => {
            p.target = None;
            p.vars[0] = 0.0;
        }
    }
}

impl WeaponBehavior for Seeker {
    fn id(&self) -> &'static str {
        "seeker"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let def = ctx.def(p.weapon);
        let half = def.param("cone_deg", 30.0) * 0.5;
        let found = acquire(ctx, p, def.param("range", 60.0), Some(half));
        record_lock(p, found.map(|(lock, _)| lock));
        if let Some((_, pos)) = found {
            p.vel = steer_towards(p.vel, pos - p.pos, def.param("turn_rate", 2.5) * DT);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::{Vec3, DT};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn seeker_duel(target: Vec3) -> (World, Room, PlayerId, PlayerId) {
        let mut w = world();
        let t = room();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "seeker");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, target, Vec3::Z);
        (w, t, a, b)
    }

    /// Bonus: homes on a target inside the cone that a dumbfire would miss.
    #[test]
    fn homes_on_the_nearest_target_in_its_cone() {
        let target = Vec3::new(4.0, 2.0, -10.0);
        let (mut w, mut t, a, b) = seeker_duel(target);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        assert_eq!(w.projectiles()[0].vel.length(), 25.0);
        for _ in 0..150 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, target, Vec3::Z);
        }
        assert!(
            hits_on(&ev, b).contains(&30.0),
            "direct hit {:?}",
            hits_on(&ev, b)
        );
    }

    /// Only inside the 30 degree cone: a target 45 degrees off is ignored.
    #[test]
    fn ignores_targets_outside_the_cone() {
        let target = Vec3::new(20.0, 0.0, 0.0);
        let (mut w, mut t, a, b) = seeker_duel(target);
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        for _ in 0..90 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, target, Vec3::Z);
        }
        assert_eq!(damage_to(&ev, b), 0.0);
    }

    /// Malus: a boosting pilot outflies it (28.5 u/s on the diagonal), a
    /// cruising one does not.
    #[test]
    fn can_be_outflown() {
        for (speed, caught) in [(28.5f32, false), (12.0, true)] {
            let mut w = world();
            let mut t = room_sized(Vec3::new(60.0, 60.0, 500.0));
            let (a, b) = duel(&mut w);
            equip(&mut w, a, "seeker");
            place(&mut w, a, Vec3::new(0.0, 0.0, 200.0), Vec3::NEG_Z);
            let mut pos = Vec3::new(0.0, 0.0, 180.0);
            place(&mut w, b, pos, Vec3::NEG_Z);
            let mut ev = run(&mut w, &mut t, &fire2(a), 1);
            for _ in 0..400 {
                pos.z -= speed * DT;
                place(&mut w, b, pos, Vec3::NEG_Z);
                ev.extend(run(&mut w, &mut t, &[], 1));
            }
            assert_eq!(damage_to(&ev, b) > 0.0, caught, "fleeing at {speed} u/s");
        }
    }

    /// Malus: fooled by the Decoy: a decoy nearer than its pilot takes the
    /// missile.
    #[test]
    fn fooled_by_a_decoy() {
        let target = Vec3::new(3.0, 0.0, -20.0);
        let (mut w, mut t, a, b) = seeker_duel(target);
        let id = w.spawn_entity(decoy(b, Vec3::new(-3.0, 0.0, -5.0)));
        let mut ev = run(&mut w, &mut t, &fire2(a), 1);
        let mut locked_decoy = false;
        for _ in 0..150 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, target, Vec3::Z);
            if let Some(p) = w.projectiles().first() {
                locked_decoy |= p.vars[0] == 1.0 && p.vars[1].to_bits() == id;
            }
        }
        assert!(locked_decoy, "locked on the decoy");
        assert_eq!(damage_to(&ev, b), 0.0, "the pilot was spared");
        let hp = w.entities().iter().find(|e| e.id == id).map(|e| e.hp);
        assert!(hp.is_none_or(|hp| hp < 100.0), "the decoy took the hit");
    }

    /// Lasting effect: a 2 u (diameter) crater where it hits rock.
    #[test]
    fn leaves_a_2u_crater() {
        let mut w = world();
        let mut t = room();
        let (a, _) = duel(&mut w);
        equip(&mut w, a, "seeker");
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &fire2(a), 40);
        let c = craters(&ev);
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0].1 * 2.0, 2.0);
        assert!((c[0].0.z + 30.0).abs() < 0.3);
    }
}
