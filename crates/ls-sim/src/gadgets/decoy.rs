//! Decoy (design doc, Gadgets; M2 plan): a copy of your ship that draws fire.
//! Integrity equals the owner's max hull; it lasts `duration_secs` (6 s).
//! When its timer runs out it snaps back to its owner like a rubber band,
//! very fast (`return_speed`), and hits the first ship on its path for
//! `return_fraction` (99%) of its remaining integrity: the owner if nobody is
//! in the way, otherwise that ship, teammates included.
//!
//! Status: TODO(wave-B). The fallback launches the decoy entity (drifting at
//! `launch_speed` along the owner's aim, shootable: projectiles hit it and
//! wear down its `hp`) and lets it expire without the return hit. To finish:
//! move the decoy back along the path in `entity_tick` once expired (e.g.
//! extend `expires_tick` and use `vars` as a phase flag), sweep ships on the
//! way, and damage the first one hit with `ctx.damage_proportional`.

use crate::behavior::Ctx;
use crate::entity::{EntityKind, EntitySpawn, Source};
use crate::gadgets::{GadgetBehavior, GadgetUse};
use crate::visual::BuiltinVisual;

#[derive(Debug)]
pub struct Decoy;

impl GadgetBehavior for Decoy {
    fn id(&self) -> &'static str {
        "decoy"
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
            state: [ship.ship_type.index(), 0, 0, 0],
        });
    }
}
