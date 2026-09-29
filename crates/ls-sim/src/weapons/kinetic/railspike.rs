//! Railspike: kinetic primary. 270 u/s; heavy shots pierce ships and walls up to 2 u thick.
//! Malus: slow fire rate; muzzle flash reveals you. Lasting effect: drills straight holes
//! through thin terrain that let light through.
//!
//! Numbers: `data/weapons/railspike.ron` (`params`: `pierce_thickness`, `hole_radius`,
//! `hole_spacing`, `reveal_secs`; `terrain.crater_radius` is the dent a spike leaves in a wall
//! too thick to pierce).
//!
//! Behaviour:
//! - `fire`: one fast spike (the generic pattern) plus a `MuzzleFlash` cue at the muzzle with
//!   the shooter as source and `reveal_secs` in ticks as value, sent to every client. Firing
//!   also sets the shooter's `last_fired_tick`, which uncloaks a cloaked shooter (the Cloak
//!   reveals anyone who fired in its `reveal_after_fire_secs`).
//! - Ships and blocking entities (debris, decoys): full damage and knockback to each one in
//!   the line, and the spike flies on (`HitResponse::Continue`, each hit once).
//! - Walls: if the rock along the flight line is at most `pierce_thickness` thick, the spike
//!   drills a straight hole (a channel of `hole_radius` craters from entry to exit) and flies
//!   on from the far side (`HitResponse::Pierce`); thicker rock stops it with a small dent.

use ls_math::terrain::RayHit;
use ls_math::Tick;

use crate::behavior::{Command, Ctx, Cue, CueKind, DamageSource};
use crate::weapons::{fire_pattern, EntityHit, HitResponse, ShipHit, Shot, WeaponBehavior};
use crate::world::Projectile;

/// Sample step of [`Ctx::thickness`] (u).
const THICKNESS_STEP: f32 = 0.1;

/// Behaviour of the Railspike.
#[derive(Debug)]
pub struct Railspike;

impl WeaponBehavior for Railspike {
    fn id(&self) -> &'static str {
        "railspike"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        fire_pattern(ctx, shot);
        let ship = shot.shooter;
        let radius = ctx.tuning_of(ship).radius;
        let reveal = Tick::ticks_for_secs(shot.def.param("reveal_secs", 0.5));
        ctx.cue(Cue {
            kind: CueKind::MuzzleFlash,
            source: Some(ship.player),
            target: None,
            pos: shot.muzzle(radius, ship.orientation.forward()),
            value: reveal.min(u64::from(u16::MAX)) as u16,
        });
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        ctx.detonate(p, hit.point, Some(hit.target), false);
        HitResponse::Continue
    }

    fn on_entity_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, false);
        if p.damage > 0.0 {
            ctx.push(Command::DamageEntity {
                id: hit.id,
                amount: p.damage,
                source: DamageSource::projectile(p),
            });
        }
        ctx.impulse_entity(hit.id, p.vel.normalize_or_zero() * p.knockback);
        HitResponse::Continue
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        let def = ctx.def(p.weapon);
        let dir = p.vel.normalize_or_zero();
        // `thickness` samples every 0.1 u and the first open sample behind a
        // wall of exactly `pierce_thickness` lies one step further.
        let max = def.param("pierce_thickness", 2.0) + THICKNESS_STEP;
        match ctx.thickness(hit.point, dir, max) {
            Some(t) => {
                // Entry impact, then a straight hole through to the far side.
                ctx.detonate(p, hit.point, None, false);
                let radius = def.param("hole_radius", 0.35);
                let spacing = def.param("hole_spacing", 0.5);
                ctx.channel(hit.point, hit.point + dir * t, radius, spacing);
                HitResponse::Pierce {
                    exit: hit.point + dir * (t + 0.05),
                }
            }
            None => {
                ctx.detonate(p, hit.point, None, true);
                HitResponse::Stop
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::weapons::kinetic::rivet_gun::testkit::*;
    use crate::*;
    use ls_math::terrain::TerrainQuery;
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Bonus: 270 u/s, 35 damage, and it pierces every ship in the line.
    #[test]
    fn pierces_ships_in_a_line() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        let c = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, a, "railspike");
        place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        let park = |w: &mut World| {
            place(w, b, Vec3::new(0.0, 0.0, 10.0), Vec3::Z);
            place(w, c, Vec3::new(0.0, 0.0, -10.0), Vec3::Z);
        };
        park(&mut w);
        let mut ev = run(&mut w, &mut t, &fire1(a), 1);
        assert_eq!(w.projectiles()[0].vel.length(), 270.0);
        for _ in 0..20 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            park(&mut w);
        }
        assert_eq!(hits_on(&ev, b), vec![35.0]);
        assert_eq!(hits_on(&ev, c), vec![35.0], "flew on through b");
    }

    /// Bonus and lasting effect: pierces a 1.5 u wall, drilling a straight
    /// hole that lets light through, and hits the ship behind it.
    #[test]
    fn pierces_thin_walls_and_drills_a_hole() {
        let mut w = world();
        let mut t = room();
        slab(&mut t, -5.0, 0.75);
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "railspike");
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -15.0), Vec3::Z);
        let (from, to) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -10.0));
        assert!(t.raycast(from, to - from, 10.0).is_some(), "wall blocks");
        let mut ev = run(&mut w, &mut t, &fire1(a), 1);
        for _ in 0..20 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, Vec3::new(0.0, 0.0, -15.0), Vec3::Z);
        }
        assert_eq!(hits_on(&ev, b), vec![35.0], "hit through the wall");
        // (The spike flies on and dents the back wall of the room too.)
        let hole: Vec<_> = craters(&ev)
            .into_iter()
            .filter(|(c, _)| c.z > -10.0)
            .collect();
        assert!(hole.len() >= 3, "{hole:?}");
        assert!(
            hole.iter()
                .all(|(c, r)| c.x.abs() < 1e-3 && c.y.abs() < 1e-3 && *r == 0.35),
            "{hole:?}"
        );
        assert!(
            t.raycast(from, to - from, 10.0).is_none(),
            "light gets through the hole"
        );
    }

    /// A wall of exactly 2 u is still pierced.
    #[test]
    fn pierces_a_wall_of_exactly_two_units() {
        let mut w = world();
        let mut t = room();
        slab(&mut t, -5.0, 1.0);
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "railspike");
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -15.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire1(a), 1);
        for _ in 0..20 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, Vec3::new(0.0, 0.0, -15.0), Vec3::Z);
        }
        assert_eq!(hits_on(&ev, b), vec![35.0], "hit through the 2 u wall");
    }

    /// A wall thicker than 2 u stops the spike and keeps only a dent.
    #[test]
    fn thick_walls_stop_it() {
        let mut w = world();
        let mut t = room();
        slab(&mut t, -5.0, 1.5);
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "railspike");
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -15.0), Vec3::Z);
        let mut ev = run(&mut w, &mut t, &fire1(a), 1);
        for _ in 0..20 {
            ev.extend(run(&mut w, &mut t, &[], 1));
            place(&mut w, b, Vec3::new(0.0, 0.0, -15.0), Vec3::Z);
        }
        assert_eq!(damage_to(&ev, b), 0.0);
        assert_eq!(craters(&ev).len(), 1, "one dent");
        assert!(w.projectiles().is_empty());
    }

    /// Malus: slow fire rate, and every shot flashes: a `MuzzleFlash` cue
    /// for all clients that also uncloaks a cloaked shooter.
    #[test]
    fn slow_fire_and_muzzle_flash_reveals_the_shooter() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "railspike");
        w.give_gadget(a, "cloak").unwrap();
        place(&mut w, a, Vec3::new(0.0, 0.0, 25.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(20.0, 0.0, -20.0), Vec3::Z);
        let g = [(a, InputCmd::idle(0).with_buttons(Buttons::GADGET))];
        run(&mut w, &mut t, &g, 1);
        run(&mut w, &mut t, &[], 30);
        assert!(!w.ship_visible_to(b, a), "cloaked");
        let ev = run(&mut w, &mut t, &fire1(a), 60);
        assert_eq!(fired(&ev), 1, "1.5 s between spikes");
        let flash = ev
            .iter()
            .find_map(|e| match e {
                Event::Cue(c) if c.kind == CueKind::MuzzleFlash => Some(*c),
                _ => None,
            })
            .expect("muzzle flash");
        assert_eq!(flash.source, Some(a));
        assert_eq!(flash.value, 30);
        assert!(flash.pos.distance(Vec3::new(0.0, 0.0, 25.0)) < 1.0);
        let ev = run(&mut w, &mut t, &fire1(a), 31);
        assert_eq!(fired(&ev), 1);
        assert!(w.ship_visible_to(b, a), "the flash gives the shooter away");
    }
}
