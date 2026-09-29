//! Leech Arc: energy primary. Design doc: "Short arc (12 u) that locks the nearest target and
//! drains its energy into yours. Malus: useless beyond 12 u. Lasting effect: surfaces stay
//! charged for 15 s and drain energy from ships that scrape them." The only instant weapon
//! (M2 plan): no projectile.
//!
//! Numbers: `data/weapons/leech_arc.ron` (`damage`, `knockback` per arc; `params`: `range`,
//! `lock_half_angle_deg`, `drain_per_arc`, `beam_every`, `ground_reach`, `charged_radius`,
//! `charged_secs`,
//! and `charged_drain_per_sec`, which the framework's charged-rock step reads).
//!
//! Behaviour (`fire`, every paid shot):
//! - Lock: the nearest living enemy within `range` of the shooter inside a cone of
//!   `lock_half_angle_deg` around the aim with a clear line of sight (`ctx.target_in_cone`).
//!   The lock is re-acquired every arc (no per-weapon state holds the previous target).
//! - Hit: drains `drain_per_arc` energy from the target into the shooter (`ctx.drain_energy`,
//!   capped by what the target has and the shooter's tank; fixed per arc, so a rule set's fire
//!   rate multiplier scales drain and damage alike), deals the shot's damage with knockback
//!   proportional to it (`damage_proportional`).
//! - Beam cue: every `beam_every`-th arc sends `CueKind::Beam` (source shooter, target the
//!   locked ship or none, `pos` the arc's end, `value` the ticks it covers), so clients draw a
//!   continuous beam without a cue per arc.
//! - The arc grounds: past a locked target it reaches `ground_reach` u further; without a
//!   target it runs along the aim up to `range`. Rock it touches is charged
//!   (`SurfaceStates::CHARGED`, `charged_radius`, `charged_secs`); the framework drains ships
//!   scraping charged rock. Already charged spots are not re-edited.
//! - Malus: nothing beyond `range`; an arc that finds nothing still costs its energy (the beam
//!   cue then ends at the rock it hit or at full range).

use ls_math::terrain::SurfaceStates;
use ls_math::Vec3;

use super::scrap_laser::heat_surface;
use crate::behavior::{Ctx, Cue, CueKind, DamageSource};
use crate::weapons::{Shot, WeaponBehavior};

/// Behaviour of the Leech Arc.
#[derive(Debug)]
pub struct LeechArc;

/// Charges the rock hit by a ray from `from` along `dir` within `reach`;
/// returns the hit point.
fn ground(ctx: &mut Ctx, shot: &Shot, from: Vec3, dir: Vec3, reach: f32) -> Option<Vec3> {
    let hit = ctx.raycast(from, dir, reach)?;
    let radius = shot.def.param("charged_radius", 1.0);
    let secs = shot.def.param("charged_secs", 15.0);
    heat_surface(ctx, hit.point, radius, SurfaceStates::CHARGED, secs);
    Some(hit.point)
}

impl WeaponBehavior for LeechArc {
    fn id(&self) -> &'static str {
        "leech_arc"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let s = shot.shooter;
        let def = shot.def;
        let fwd = s.orientation.forward();
        let range = def.param("range", 12.0);
        let half = def.param("lock_half_angle_deg", 30.0);
        let target = ctx
            .target_in_cone(s.pos, fwd, range, half, s.team)
            .map(|t| (t.player, t.pos));
        let (end, target) = match target {
            Some((player, pos)) => {
                let drain = def.param("drain_per_arc", 2.0);
                ctx.drain_energy(player, Some(s.player), drain);
                let src = DamageSource {
                    attacker: Some(s.player),
                    team: s.team,
                    weapon: Some(shot.weapon),
                };
                let kb = def.knockback_per_damage();
                ctx.damage_proportional(player, shot.damage, pos - s.pos, kb, src, true);
                let dir = (pos - s.pos).normalize_or_zero();
                ground(ctx, shot, pos, dir, def.param("ground_reach", 3.0));
                (pos, Some(player))
            }
            None => {
                let end = ground(ctx, shot, s.pos, fwd, range).unwrap_or(s.pos + fwd * range);
                (end, None)
            }
        };
        let every = def.param("beam_every", 3.0).max(1.0) as u32;
        if shot.shots % every == 0 {
            let ticks = def.fire_interval_ticks() * u64::from(every);
            ctx.cue(Cue {
                kind: CueKind::Beam,
                source: Some(s.player),
                target,
                pos: end,
                value: ticks.min(u64::from(u16::MAX)) as u16,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::SurfaceStates;
    use ls_math::Vec3;

    use super::super::scrap_laser::testkit::*;
    use crate::{CueKind, Event, InputCmd, PlayerId, ShipTuning, ShipType, SimTerrain};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn beams(ev: &[Event]) -> Vec<(Option<PlayerId>, Vec3)> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Cue(c) if c.kind == CueKind::Beam => Some((c.target, c.pos)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn locks_the_nearest_target_and_drains_its_energy_into_yours() {
        // Shooter's energy after 30 ticks of arcing at a target `dist` u away.
        let arc = |dist: f32| {
            let (mut w, mut t, a, b) = duel("leech_arc", ShipType::Tinker);
            let c = w.add_player(Some(1), ShipType::Tinker).unwrap();
            place(&mut w, a, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z);
            // b slightly off the aim, c further away right on it.
            place(&mut w, b, Vec3::new(1.5, 0.0, 5.0 - dist), Vec3::Z);
            place(&mut w, c, Vec3::new(0.0, 0.0, 5.0 - dist - 2.5), Vec3::Z);
            w.ship_mut(a).unwrap().energy = 20.0;
            w.ship_mut(b).unwrap().energy = 50.0;
            let ev = run(&mut w, &mut t, &hold(a), 30);
            let ea = w.ship(a).unwrap().energy;
            let eb = w.ship(b).unwrap().energy;
            (ev, ea, eb, b, c)
        };
        let (ev, ea, eb, b, c) = arc(8.0);
        assert_eq!(fired(&ev), 0, "instant: no projectile");
        // 10 arcs per second (5 in 30 ticks); a beam cue every 3rd arc,
        // covering 18 ticks.
        let shots = beams(&ev);
        assert_eq!(shots.len(), 2);
        assert!(ev
            .iter()
            .any(|e| matches!(e, Event::Cue(c) if c.kind == CueKind::Beam && c.value == 18)));
        assert!(
            shots.iter().all(|(t, _)| *t == Some(b)),
            "locked on the nearest"
        );
        assert!(damage_to(&ev, b) > 0.0);
        assert_eq!(damage_to(&ev, c), 0.0);
        // 5 arcs x 2 energy drained from b into a.
        let (_, ea_miss, eb_miss, _, _) = arc(20.0);
        assert!((ea - ea_miss - 10.0).abs() < 0.05, "{ea} vs {ea_miss}");
        assert!((eb_miss - eb - 10.0).abs() < 0.05, "{eb} vs {eb_miss}");
    }

    #[test]
    fn useless_beyond_twelve_units() {
        let (mut w, mut t, a, b) = duel("leech_arc", ShipType::Tinker);
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -3.0), Vec3::Z);
        let e0 = w.ship(a).unwrap().energy;
        w.ship_mut(a).unwrap().energy = 50.0;
        let ev = run(&mut w, &mut t, &hold(a), 30);
        assert_eq!(damage_to(&ev, b), 0.0);
        assert_eq!(w.ship(b).unwrap().energy, e0);
        let shots = beams(&ev);
        assert_eq!(shots.len(), 2);
        assert!(shots.iter().all(|(t, _)| t.is_none()));
        // Still paid: 5 x 0.5 energy against 5 energy of regeneration.
        assert!((w.ship(a).unwrap().energy - 52.5).abs() < 0.05);
        // At 11.5 u the same arc connects.
        place(&mut w, b, Vec3::new(0.0, 0.0, -1.5), Vec3::Z);
        let ev = run(&mut w, &mut t, &hold(a), 30);
        assert!(damage_to(&ev, b) > 0.0);
    }

    #[test]
    fn charged_surfaces_last_fifteen_seconds_and_drain_scrapers() {
        let (mut w, mut t, a, b) = duel("leech_arc", ShipType::Tinker);
        // Arc the floor (y = -15) from 10 u above it.
        place(&mut w, a, Vec3::new(0.0, -5.0, 0.0), Vec3::NEG_Y);
        place(&mut w, b, Vec3::new(20.0, 0.0, 20.0), Vec3::Z);
        let t0 = w.tick();
        let ev = run(&mut w, &mut t, &hold(a), 12);
        let charged = surfaces(&ev, SurfaceStates::CHARGED);
        assert_eq!(charged.len(), 1, "the spot is charged once, not per arc");
        let (c, _, until) = charged[0];
        assert!((c.y + 15.0).abs() < 0.1, "{c:?}");
        // The first arc goes off in the first step (tick t0 + 1).
        let until = until as u64;
        assert_eq!(until, t0 + 1 + 900, "15 s");
        assert!(beams(&ev).iter().all(|(_, p)| (p.y + 15.0).abs() < 0.1));
        assert!(t
            .surface_states_near(c, 0.1, until - 1)
            .contains(SurfaceStates::CHARGED));
        assert!(t.surface_states_near(c, 0.1, until).is_empty());

        // b scrapes the charged floor and loses 5 energy/s against a free-floating a.
        let floor_y = -15.0 + ShipTuning::tinker().radius;
        place(&mut w, a, Vec3::new(10.0, 0.0, 0.0), Vec3::Z);
        place(&mut w, b, Vec3::new(0.0, floor_y, 0.0), Vec3::Z);
        for id in [a, b] {
            w.ship_mut(id).unwrap().energy = 50.0;
        }
        let press = InputCmd::idle(0).with_thrust(Vec3::NEG_Y);
        let inputs = [(b, press), (a, InputCmd::idle(0))];
        run(&mut w, &mut t, &inputs, 60);
        let (ea, eb) = (w.ship(a).unwrap().energy, w.ship(b).unwrap().energy);
        assert!(w.ship(b).unwrap().in_contact);
        assert!((ea - eb - 5.0).abs() < 0.1, "{ea} {eb}");
    }
}
