//! Decoy (design doc, Gadgets; M2 plan): a copy of your ship that draws fire.
//! Integrity equals the owner's max hull; it lasts `duration_secs` (6 s).
//! When its timer runs out it snaps back to its owner like a rubber band,
//! very fast (`return_speed`), and hits the first ship on its path for
//! `return_fraction` (99%) of its remaining integrity: the owner if nobody is
//! in the way, otherwise that ship, teammates included.
//!
//! Numbers (`data/gadgets/decoy.ron`): `launch_speed` (drift speed along the
//! owner's aim), `return_speed`, `return_fraction`, `return_max_secs` (a
//! return that has not reached anyone by then fizzles) and
//! `return_knockback_per_damage`.
//!
//! Phases (`Entity::vars[0]`): 0 = outbound (a physical sphere drifting with
//! terrain collision, shootable: projectiles wear down its `hp`), 1 =
//! returning (kinematic, homing on the owner through rock, still shootable).
//! Replicated state (`EntityView::state[1]`, the module byte): the owner's
//! ship type index in the low nibble, [`RETURNING_BIT`] while returning.
//!
//! The return hit ignores friendly-fire scaling (the design doc: "even a
//! teammate"); its attacker is the owner (kill credit, a self-kill when it
//! hits the owner), without a weapon.
//!
//! Drawing fire: projectiles of other players hit the decoy (the framework
//! treats it as a blocking entity with `hp`), and the Seeker and Swarm Pod
//! modules home on enemy decoys like on ships.
//!
//! Status: implemented.

use ls_math::{Vec3, DT};

use crate::behavior::{Ctx, Cue, CueKind, DamageSource};
use crate::entity::{Entity, EntityKind, EntitySpawn, Source, NO_TEAM};
use crate::gadgets::{GadgetBehavior, GadgetDef, GadgetUse};
use crate::ship::PlayerId;
use crate::terrain::segment_sphere;
use crate::visual::BuiltinVisual;

/// Bit of the decoy's module state byte set while it rubber-bands back.
pub const RETURNING_BIT: u8 = 0x80;

/// `Entity::vars[0]` value of a returning decoy.
const PHASE_RETURNING: f32 = 1.0;

#[derive(Debug)]
pub struct Decoy;

impl GadgetBehavior for Decoy {
    fn id(&self) -> &'static str {
        "decoy"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_activate(&self, ctx: &mut Ctx, g: &GadgetUse) {
        let ship = g.ship;
        let tuning = ctx.tuning_of(ship);
        let fwd = ship.orientation.forward();
        let speed = g.def.param("launch_speed", 12.0);
        ctx.spawn_entity(EntitySpawn {
            kind: EntityKind::Decoy,
            source: Source::Gadget(g.gadget),
            visual: BuiltinVisual::Decoy.id(),
            pos: ship.pos + fwd * (tuning.radius * 2.5),
            vel: fwd * speed,
            rot: ship.orientation,
            radius: tuning.radius,
            owner: Some(ship.player),
            team: ship.team,
            lifetime_ticks: Some(g.def.duration_ticks()),
            hp: tuning.hull_max,
            scrap: 0.0,
            kinematic: false,
            vars: [0.0; 4],
            state: [ship.ship_type.index() & 0x0f, 0, 0, 0],
        });
    }

    fn entity_tick(&self, ctx: &mut Ctx, def: &GadgetDef, e: &mut Entity) {
        if is_returning(e) {
            return_step(ctx, def, e);
        }
    }

    fn entity_expired(&self, ctx: &mut Ctx, def: &GadgetDef, e: &mut Entity) {
        if is_returning(e) {
            // The return ran out of time without reaching anyone: it fizzles.
            return;
        }
        // Timer over: snap back to the owner.
        e.vars[0] = PHASE_RETURNING;
        e.kinematic = true;
        e.state[0] |= RETURNING_BIT;
        let max = ls_math::Tick::ticks_for_secs(def.param("return_max_secs", 3.0)).max(1);
        e.expires_tick = ctx.now() + max;
        return_step(ctx, def, e);
    }
}

/// Whether a decoy entity is rubber-banding back to its owner.
pub fn is_returning(e: &Entity) -> bool {
    e.vars[0] == PHASE_RETURNING
}

/// One tick of the return: aim at the owner, sweep the path the decoy will
/// fly this tick (`return_speed * DT`) against every living ship and hit the
/// first one; otherwise set the velocity the framework moves it by.
fn return_step(ctx: &mut Ctx, def: &GadgetDef, e: &mut Entity) {
    let owner = e.owner.and_then(|o| ctx.ship(o)).filter(|s| s.alive);
    let Some(owner) = owner else {
        // Nobody to return to (owner dead or gone): the decoy vanishes.
        e.hp = 0.0;
        return;
    };
    let to = owner.pos - e.pos;
    let dist = to.length();
    let dir = if dist > 1e-4 {
        to / dist
    } else {
        e.vel.normalize_or_zero()
    };
    let step = def.param("return_speed", 60.0).max(0.0) * DT;
    // The sweep reaches the owner's centre at the latest.
    let reach = step.min(dist.max(0.0));
    let mut best: Option<(f32, PlayerId)> = None;
    for s in ctx.view.ships.values() {
        if !s.alive {
            continue;
        }
        let r = ctx.tuning_of(s).radius + e.radius;
        let hit = if dir == Vec3::ZERO {
            (s.pos.distance_squared(e.pos) <= r * r).then_some(0.0)
        } else {
            segment_sphere(e.pos, dir, reach, s.pos, r)
        };
        if let Some(t) = hit {
            // Ties go to the lower player id (ships iterate in id order).
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, s.player));
            }
        }
    }
    let Some((t, target)) = best else {
        e.vel = dir * (step / DT);
        return;
    };
    let amount = e.hp.max(0.0) * def.param("return_fraction", 0.99).clamp(0.0, 1.0);
    let kb = def.param("return_knockback_per_damage", 0.2).max(0.0);
    let point = e.pos + dir * t;
    let source = DamageSource {
        attacker: e.owner,
        // No team: the return hit ignores friendly-fire scaling.
        team: NO_TEAM,
        weapon: None,
    };
    ctx.damage_proportional(target, amount, dir, kb, source, true);
    ctx.cue(Cue {
        kind: CueKind::DecoyReturn,
        source: e.owner,
        target: Some(target),
        pos: point,
        value: amount.clamp(0.0, u16::MAX as f32) as u16,
    });
    e.pos = point;
    e.vel = Vec3::ZERO;
    e.hp = 0.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gadgets::test_util::*;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn decoys(w: &World) -> Vec<Entity> {
        w.entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Decoy)
            .cloned()
            .collect()
    }

    /// A decoy of `owner` at `pos` with integrity `hp` whose timer ends
    /// after `ticks` (as if launched earlier).
    fn spawn_decoy(w: &mut World, owner: PlayerId, pos: Vec3, hp: f32, ticks: u64) -> u32 {
        let g = w.gadgets().find("decoy").unwrap();
        let s = w.ship(owner).unwrap().clone();
        w.spawn_entity(EntitySpawn {
            kind: EntityKind::Decoy,
            source: Source::Gadget(g),
            visual: BuiltinVisual::Decoy.id(),
            pos,
            vel: Vec3::ZERO,
            rot: s.orientation,
            radius: 0.5,
            owner: Some(owner),
            team: s.team,
            lifetime_ticks: Some(ticks),
            hp,
            scrap: 0.0,
            kinematic: false,
            vars: [0.0; 4],
            state: [s.ship_type.index(), 0, 0, 0],
        })
    }

    fn return_cues(ev: &[Event]) -> Vec<Cue> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Cue(c) if c.kind == CueKind::DecoyReturn => Some(*c),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn launched_decoy_snaps_back_and_hits_its_owner_for_99_percent() {
        let mut w = world_with_gadget("decoy");
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &press(a, Buttons::GADGET), 1);
        let d = decoys(&w);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].hp, 100.0, "integrity = owner's max hull");
        let expires = d[0].expires_tick;
        assert_eq!(expires - d[0].spawn_tick, 360, "6 s");
        // Outbound: it drifts away along the aim and hurts nobody.
        let n = (expires - w.tick() - 1) as u32;
        ev.extend(run(&mut w, &mut t, &[], n));
        assert_eq!(damage_to(&ev, a), 0.0);
        let far = decoys(&w)[0].pos;
        // (It bounced off the far wall and drifts back a little.)
        assert!(far.z < -10.0, "{far:?}");
        // Timer over: it turns kinematic and flies back very fast.
        ev.extend(run(&mut w, &mut t, &[], 2));
        let d = decoys(&w);
        assert_eq!(d.len(), 1);
        assert!(d[0].kinematic && is_returning(&d[0]));
        assert_eq!(d[0].state[0] & RETURNING_BIT, RETURNING_BIT);
        assert!((d[0].vel.length() - 60.0).abs() < 1e-3, "{:?}", d[0].vel);
        let view = w
            .entity_views()
            .into_iter()
            .find(|v| v.kind == ReplicatedKind::Decoy)
            .unwrap();
        assert_eq!(view.state[1] & RETURNING_BIT, RETURNING_BIT);
        let start = w.tick();
        let mut hit_tick = None;
        for _ in 0..60 {
            let e = w.step(&mut t, &[]);
            if damage_to(&e, a) > 0.0 {
                hit_tick = Some(w.tick());
            }
            ev.extend(e);
            if hit_tick.is_some() {
                break;
            }
        }
        // About 29 u at 60 u/s: half a second.
        let took = hit_tick.expect("return hit") - start;
        let limit = (far.length() / 60.0 * 60.0) as u64 + 3;
        assert!(took <= limit, "{took} ticks > {limit}");
        assert!(
            (damage_to(&ev, a) - 99.0).abs() < 1e-3,
            "{}",
            damage_to(&ev, a)
        );
        assert!(ev.iter().any(|e| matches!(
            e,
            Event::Damaged { target, attacker: Some(att), weapon: None, direct: true, .. }
                if *target == a && *att == a
        )));
        let s = w.ship(a).unwrap();
        assert!(s.alive && (s.hull - 1.0).abs() < 1e-3, "{}", s.hull);
        // 99 x 0.2 u/s along the return direction (+Z, back towards a).
        assert!(
            (s.vel.z - 19.8).abs() < 0.01,
            "pushed along the return path: {:?}",
            s.vel
        );
        let cues = return_cues(&ev);
        assert_eq!(cues.len(), 1);
        assert_eq!(
            (cues[0].source, cues[0].target, cues[0].value),
            (Some(a), Some(a), 99)
        );
        assert!(decoys(&w).is_empty(), "spent");
    }

    #[test]
    fn a_shot_up_decoy_hits_for_99_percent_of_what_is_left() {
        let mut w = world_with_gadget("decoy");
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        // b's laser hits the decoy in front of a (it draws fire) ...
        place(&mut w, a, Vec3::new(0.0, 0.0, -10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        spawn_decoy(&mut w, a, Vec3::ZERO, 100.0, 600);
        let mut ev = run(&mut w, &mut t, &press(b, Buttons::FIRE_PRIMARY), 1);
        ev.extend(run(&mut w, &mut t, &[], 30));
        assert_eq!(damage_to(&ev, a), 0.0, "the decoy blocks the shot");
        let hp = decoys(&w)[0].hp;
        assert!(hp < 100.0, "{hp}");
        // ... and a decoy at half integrity returns for half the damage.
        let mut w = world_with_gadget("decoy");
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        spawn_decoy(&mut w, a, Vec3::new(0.0, 0.0, -10.0), 50.0, 1);
        let ev = run(&mut w, &mut t, &[], 30);
        assert!(
            (damage_to(&ev, a) - 49.5).abs() < 1e-3,
            "{}",
            damage_to(&ev, a)
        );
    }

    #[test]
    fn the_first_ship_in_the_path_takes_the_hit_even_a_teammate() {
        let mut w = world_with_gadget("decoy");
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let mate = w.add_player(Some(0), ShipType::Tinker).unwrap();
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, mate, Vec3::new(0.3, 0.0, 0.0), Vec3::NEG_Z);
        spawn_decoy(&mut w, a, Vec3::new(0.0, 0.0, -10.0), 100.0, 1);
        let ev = run(&mut w, &mut t, &[], 40);
        assert_eq!(damage_to(&ev, a), 0.0, "the teammate was in the way");
        assert!(
            (damage_to(&ev, mate) - 99.0).abs() < 1e-3,
            "no friendly-fire scaling: {}",
            damage_to(&ev, mate)
        );
        assert!(decoys(&w).is_empty());
        let cues = return_cues(&ev);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].target, Some(mate));
    }

    #[test]
    fn an_enemy_in_the_path_is_hit_and_the_owner_gets_the_kill() {
        let mut w = world_with_gadget("decoy");
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Hauler).unwrap();
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::NEG_Z);
        w.ship_mut(b).unwrap().hull = 50.0;
        spawn_decoy(&mut w, a, Vec3::new(0.0, 0.0, -10.0), 100.0, 1);
        let ev = run(&mut w, &mut t, &[], 40);
        assert_eq!(damage_to(&ev, a), 0.0);
        assert!(ev.iter().any(|e| matches!(
            e,
            Event::Killed { victim, killer: Some(k), weapon: None } if *victim == b && *k == a
        )));
        assert_eq!(w.ship(a).unwrap().kills, 1);
    }

    #[test]
    fn without_its_owner_the_decoy_vanishes() {
        let mut w = world_with_gadget("decoy");
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        place(&mut w, b, Vec3::new(0.0, 0.0, -5.0), Vec3::NEG_Z);
        spawn_decoy(&mut w, a, Vec3::new(0.0, 0.0, -10.0), 100.0, 3);
        assert!(w.remove_player(a));
        let ev = run(&mut w, &mut t, &[], 30);
        assert_eq!(damage_to(&ev, b), 0.0);
        assert!(decoys(&w).is_empty());
    }

    #[test]
    fn a_return_that_reaches_nobody_fizzles_after_its_time_limit() {
        let mut gadgets: Vec<GadgetDef> = GadgetTable::builtin()
            .iter()
            .map(|(_, d)| d.clone())
            .collect();
        for d in &mut gadgets {
            if d.id == "decoy" {
                d.params.insert("return_speed".into(), 2.0);
                d.params.insert("return_max_secs".into(), 1.0);
            }
        }
        let mut rules = RuleSet::paint_the_void();
        rules.starting_loadout.gadget = Some("decoy".into());
        rules
            .gadgets
            .insert("decoy".into(), Availability::LoadoutAndPickup);
        let mut w = World::new(WorldConfig {
            rules,
            gadgets: GadgetTable::new(gadgets).unwrap(),
            ..WorldConfig::paint_the_void(11, setup())
        })
        .unwrap();
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        spawn_decoy(&mut w, a, Vec3::new(0.0, 0.0, -20.0), 100.0, 1);
        let ev = run(&mut w, &mut t, &[], 30);
        assert_eq!(decoys(&w).len(), 1, "still returning at 2 u/s");
        let ev2 = run(&mut w, &mut t, &[], 40);
        assert!(decoys(&w).is_empty(), "fizzled after 1 s");
        assert_eq!(damage_to(&ev, a) + damage_to(&ev2, a), 0.0);
    }
}
