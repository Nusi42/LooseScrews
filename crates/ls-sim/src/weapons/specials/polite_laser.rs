//! Polite Laser: special primary. Very high damage. Malus: 1.5 s charge with an 'excuse me'
//! chime the target hears (PoliteWarning cue). Lasting effect: burns a clean hole through
//! thin walls.
//!
//! Numbers: `data/weapons/polite_laser.ron` (`charge`: 1.5 s, auto-release, no early shot;
//! `params`: `warn_cone_deg`, `rewarn_ticks`, `hole_radius`, `hole_thickness`, `max_walls`).
//!
//! Status: implemented.
//! - Charge (framework): nothing fires or is paid for 1.5 s; the shot releases by itself at
//!   full charge, letting go earlier cancels it.
//! - `charge_start` politely warns the ship in the aim (nearest enemy within the bolt's reach
//!   in a `warn_cone_deg` cone, line of sight): a `PoliteWarning` cue with `target` = that
//!   ship and `value` = ticks until the shot. `charge_hold` repeats the warning every
//!   `rewarn_ticks` for whoever is in the aim then (so a pilot the laser swings onto hears
//!   it too).
//! - Hits deal the very high direct damage (the generic detonate).
//! - Walls up to `hole_thickness` u thick get a clean hole burnt through them (exact,
//!   strength-independent craters of `hole_radius` every half radius) and the bolt flies on
//!   out of the far side, through at most `max_walls` walls; a thicker wall only gets a
//!   `hole_radius` dent and stops the bolt.

use ls_math::terrain::RayHit;

use crate::behavior::{Ctx, Cue, CueKind};
use crate::ship::ShipState;
use crate::weapons::{default_charge_hold, ChargeAction, HitResponse, WeaponBehavior};
use crate::world::Projectile;
use crate::{WeaponDef, WeaponId};

/// Behaviour of the Polite Laser.
#[derive(Debug)]
pub struct PoliteLaser;

/// Warns the ship in `ship`'s aim that the shot comes in `ticks_left`.
fn warn(ctx: &mut Ctx, ship: &ShipState, def: &WeaponDef, ticks_left: u64) {
    let reach = def.speed * def.lifetime_secs;
    let cone = def.param("warn_cone_deg", 8.0);
    let fwd = ship.orientation.forward();
    let Some(target) = ctx
        .target_in_cone(ship.pos, fwd, reach, cone, ship.team)
        .map(|t| t.player)
    else {
        return;
    };
    ctx.cue(Cue {
        kind: CueKind::PoliteWarning,
        source: Some(ship.player),
        target: Some(target),
        pos: ship.pos,
        value: ticks_left.min(u16::MAX as u64) as u16,
    });
}

fn charge_ticks(def: &WeaponDef) -> u64 {
    def.charge
        .map_or(0, |c| WeaponDef::charge_ticks(c.max_secs))
}

impl WeaponBehavior for PoliteLaser {
    fn id(&self) -> &'static str {
        "polite_laser"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn charge_start(&self, ctx: &mut Ctx, ship: &ShipState, weapon: WeaponId) {
        let def = ctx.def(weapon);
        warn(ctx, ship, def, charge_ticks(def));
    }

    fn charge_hold(
        &self,
        ctx: &mut Ctx,
        ship: &ShipState,
        weapon: WeaponId,
        held_ticks: u64,
    ) -> ChargeAction {
        let def = ctx.def(weapon);
        let every = (def.param("rewarn_ticks", 30.0) as u64).max(1);
        let full = charge_ticks(def);
        if held_ticks > 0 && held_ticks % every == 0 && held_ticks < full {
            warn(ctx, ship, def, full - held_ticks);
        }
        default_charge_hold(def, held_ticks)
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        let def = ctx.def(p.weapon);
        let radius = def.param("hole_radius", 0.6);
        let max_thick = def.param("hole_thickness", 2.0);
        let max_walls = def.param("max_walls", 2.0);
        let dir = p.vel.normalize_or_zero();
        ctx.detonate(p, hit.point, None, false);
        let through = if p.vars[0] < max_walls {
            ctx.thickness(hit.point, dir, max_thick)
        } else {
            None
        };
        let Some(t) = through else {
            ctx.crater_exact(hit.point, radius);
            return HitResponse::Stop;
        };
        // A clean hole: exact craters every half radius through the wall.
        let spacing = (radius * 0.5).max(0.05);
        let n = (ls_math::det::ceil(t / spacing) as usize).clamp(1, 64);
        for i in 0..=n {
            ctx.crater_exact(hit.point + dir * (t * i as f32 / n as f32), radius);
        }
        p.vars[0] += 1.0;
        HitResponse::Pierce {
            exit: hit.point + dir * (t + p.radius + 0.05),
        }
    }
}

/// Tick count of the charge (tests).
#[cfg(test)]
fn secs(s: f32) -> u64 {
    ls_math::Tick::ticks_for_secs(s)
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{TerrainEdit, TerrainQuery};
    use ls_math::Vec3;

    use super::super::paint_thrower::testkit::*;
    use super::secs;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn warns_the_target_then_hits_very_hard_after_the_charge() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new();
        let wid = equip(&mut w, a, "polite_laser");
        let def = w.weapons().get(wid).clone();
        assert!(def.damage >= 60.0, "very high damage");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.3, 0.0, -10.0), Vec3::Z);
        let hold = press(a, Buttons::FIRE_PRIMARY);
        // Warning at the first tick of the charge; nothing fires before 1.5 s.
        let ev = run(&mut w, &mut t, &hold, 1);
        let warn = cues(&ev, CueKind::PoliteWarning);
        assert_eq!(warn.len(), 1);
        assert_eq!((warn[0].source, warn[0].target), (Some(a), Some(b)));
        assert_eq!(warn[0].value as u64, secs(1.5));
        let ev = run(&mut w, &mut t, &hold, secs(1.5) as u32 - 1);
        assert_eq!(fired(&ev), 0, "still charging");
        assert!(
            cues(&ev, CueKind::PoliteWarning)
                .iter()
                .all(|c| c.target == Some(b)),
            "reminders go to the target"
        );
        let mut ev = run(&mut w, &mut t, &hold, 2);
        assert_eq!(fired(&ev), 1);
        ev.extend(run(&mut w, &mut t, &[], 60));
        let dmg = damage_to(&ev, b);
        assert!((dmg - def.damage).abs() < 1e-3, "{dmg}");
    }

    #[test]
    fn no_warning_without_a_target_and_a_late_target_is_warned() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new();
        equip(&mut w, a, "polite_laser");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(20.0, 0.0, 20.0), Vec3::Z);
        let hold = press(a, Buttons::FIRE_PRIMARY);
        let ev = run(&mut w, &mut t, &hold, 20);
        assert!(cues(&ev, CueKind::PoliteWarning).is_empty());
        // The target flies into the aim during the charge: it hears it.
        place(&mut w, b, Vec3::new(0.0, 0.0, -5.0), Vec3::Z);
        let ev = run(&mut w, &mut t, &hold, 40);
        let warn = cues(&ev, CueKind::PoliteWarning);
        assert!(!warn.is_empty());
        assert!(warn.iter().all(|c| c.target == Some(b)));
    }

    #[test]
    fn burns_a_clean_hole_through_a_thin_wall() {
        let (mut w, a, b) = world2();
        // A 1 u wall at z = 0 between the ships.
        let mut t = TestRoom::new().with_wall_z(0.0, 1.0);
        let wid = equip(&mut w, a, "polite_laser");
        let def = w.weapons().get(wid).clone();
        place(&mut w, a, Vec3::new(0.0, 0.0, 15.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -10.0), Vec3::Z);
        assert!(t.sdf(Vec3::ZERO) < 0.0);
        let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 92);
        ev.extend(run(&mut w, &mut t, &[], 60));
        let holes: Vec<TerrainEdit> = edits(&ev);
        assert!(!holes.is_empty());
        let r = def.param("hole_radius", 0.0);
        for e in &holes {
            match e {
                TerrainEdit::SubtractSphere { radius, .. } => assert_eq!(*radius, r, "clean"),
                other => panic!("only holes: {other:?}"),
            }
        }
        assert!(t.sdf(Vec3::ZERO) > 0.0, "the hole goes through");
        assert!(t.sdf(Vec3::new(0.0, 2.0, 0.0)) < 0.0, "and nothing else");
        assert!(damage_to(&ev, b) > 0.0, "hit through the wall");
        // The hole stays.
        run(&mut w, &mut t, &[], 600);
        assert!(t.sdf(Vec3::ZERO) > 0.0);
    }

    #[test]
    fn thick_walls_only_get_a_dent() {
        let (mut w, a, b) = world2();
        let mut t = TestRoom::new().with_wall_z(0.0, 4.0);
        equip(&mut w, a, "polite_laser");
        place(&mut w, a, Vec3::new(0.0, 0.0, 15.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -10.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 92);
        ev.extend(run(&mut w, &mut t, &[], 60));
        assert_eq!(edits(&ev).len(), 1, "one dent");
        assert_eq!(damage_to(&ev, b), 0.0);
        assert!(t.sdf(Vec3::ZERO) < 0.0);
        assert!(w.projectiles().is_empty());
    }
}
