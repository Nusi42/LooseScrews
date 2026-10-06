//! Pizza delivery (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! A drone brings a pizza to a random living player weighted toward low
//! hull (`director::pick_low_hull`). It enters from the boundary cavern
//! nearest the recipient (at least `min_entry_distance` away; it waits
//! there during the announce phase) and flies at 18 u/s along the network
//! ([`crate::events::drone`]), then hovers 2.5 u from the recipient.
//! Staying under 1.5 u/s within 5 u of the drone for 3 s in total (not
//! necessarily in one go) restores full hull and the drone leaves. Others
//! can shoot the drone (30 hp; the recipient's own shots pass through it:
//! the drone's `owner` is the recipient), which drops the pizza as a
//! 50-scrap pile anyone can scavenge. 30 s, then a 1.5 s ending phase in
//! which the drone flies off.
//!
//! Drone entity (`ReplicatedKind::Drone`): `state` = `[ROLE_PIZZA, hold
//! progress 0-255, phase (waiting, working, leaving), hp percent]`, owner
//! = the recipient. Outcome cues (`CueKind::EventOutcome`): 1 delivered
//! (target = recipient), `OUTCOME_SHOT_DOWN` (source = shooter).
//!
//! Numbers: `data/events/pizza_delivery.ron`.

use ls_math::{Tick, Vec3};

use crate::behavior::{Command, Cue, CueKind};
use crate::events::drone::{
    self, Flight, DRONE_LEAVING, DRONE_WAITING, DRONE_WORKING, OUTCOME_SHOT_DOWN, ROLE_PIZZA,
};
use crate::events::{EventCtx, EventKind, EventModule, EventPhase, EventRef};

/// Pizza delivery.
#[derive(Debug)]
pub struct PizzaDelivery;

/// Outcome byte: the pizza was delivered.
pub const OUTCOME_DELIVERED: u8 = 1;

/// `EventState::vars`: ticks held so far, done (delivered or shot down).
const V_HELD: usize = 0;
const V_DONE: usize = 1;

fn flight(e: &EventCtx) -> Flight {
    Flight {
        speed: e.def.param("speed", 18.0),
        accel: e.def.param("accel", 50.0),
        radius: e.def.param("radius", 0.9),
    }
}

fn hold_ticks(e: &EventCtx) -> i64 {
    Tick::ticks_for_secs(e.def.param("hold_secs", 3.0)).max(1) as i64
}

fn cue(e: &mut EventCtx, source: Option<crate::PlayerId>, pos: Vec3, byte: u8) {
    let target = e.event.target;
    e.ctx.cue(Cue {
        kind: CueKind::EventOutcome,
        source,
        target,
        pos,
        value: drone::outcome(EventKind::PizzaDelivery, byte),
    });
}

impl EventModule for PizzaDelivery {
    fn kind(&self) -> EventKind {
        EventKind::PizzaDelivery
    }

    fn implemented(&self) -> bool {
        true
    }

    /// Needs a damaged living player.
    fn eligible(&self, e: &EventRef) -> bool {
        !crate::events::damaged_ships(e.view).is_empty()
    }

    /// Names a random damaged player, weighted toward low hull; the drone
    /// waits at its entry cavern.
    fn on_announce(&self, e: &mut EventCtx) {
        e.event.target = crate::director::pick_low_hull(e);
        let f = flight(e);
        let toward = drone::living(&e.ctx.view, e.event.target)
            .map_or_else(|| e.ctx.view.terrain.bounds().center(), |s| s.pos);
        let entry = drone::entry_point(
            &e.ctx.view,
            toward,
            e.def.param("min_entry_distance", 40.0),
            f.radius,
        );
        e.event.pos = entry;
        let mut spawn = drone::prop(
            drone::DRONE,
            entry,
            f.radius,
            e.def.param("hp", 30.0),
            [ROLE_PIZZA, 0, DRONE_WAITING, 100],
        );
        spawn.owner = e.event.target;
        spawn.rot = drone::facing(toward - entry);
        e.ctx.spawn_entity(spawn);
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let Some(mut d) = drone::first_prop(&e.ctx.view, drone::DRONE) else {
            return;
        };
        let f = flight(e);
        let hp_max = e.def.param("hp", 30.0);
        let held = e.event.vars[V_HELD];
        let progress = (held * 255 / hold_ticks(e)).clamp(0, 255) as u8;
        if d.hp <= 0.0 && e.event.vars[V_DONE] != 0 {
            // Shot on its way home: the pizza is eaten already.
            e.ctx.push(Command::RemoveEntity { id: d.id });
            return;
        }
        if d.hp <= 0.0 {
            // Shot down: the pizza becomes a scrap pile for anyone.
            e.ctx.spawn_scrap(d.pos, e.def.param("scrap", 50.0));
            cue(e, d.last_hit.map(|(_, by)| by), d.pos, OUTCOME_SHOT_DOWN);
            e.ctx.push(Command::RemoveEntity { id: d.id });
            e.event.vars[V_DONE] = 1;
            drone::end_now(e);
            return;
        }
        let view = e.ctx.view;
        if e.event.phase == EventPhase::Ending || e.event.vars[V_DONE] != 0 {
            // Off it goes, back to its entry.
            let goal = drone::plan(&view, &mut d, e.event.pos, f.radius);
            let (p, v) = drone::fly(view.terrain, d.pos, d.vel, goal, &f);
            (d.pos, d.vel, d.rot) = (p, v, drone::facing(v));
            d.state = [
                ROLE_PIZZA,
                progress,
                DRONE_LEAVING,
                drone::hp_percent(d.hp, hp_max),
            ];
            e.ctx.pose_entity(&d);
            return;
        }
        let target = drone::living(&view, e.event.target);
        let goal = match target {
            None => d.pos,
            Some(t) => {
                let dist = d.pos.distance(t.pos);
                if dist <= e.def.param("hold_radius", 5.0)
                    && t.vel.length() < e.def.param("hold_speed", 1.5)
                {
                    e.event.vars[V_HELD] = held + 1;
                    if held + 1 >= hold_ticks(e) {
                        let max = e.ctx.tuning_of(t).hull_max;
                        e.ctx.adjust_hull(t.player, (max - t.hull).max(0.0), false);
                        cue(e, None, d.pos, OUTCOME_DELIVERED);
                        e.event.vars[V_DONE] = 1;
                        drone::end_now(e);
                    }
                }
                let park = e.def.param("park_distance", 2.5);
                if dist <= park * 3.0 {
                    // Hover at the recipient's side.
                    t.pos + (d.pos - t.pos).normalize_or_zero() * park
                } else {
                    drone::plan(&view, &mut d, t.pos, f.radius)
                }
            }
        };
        let (p, v) = drone::fly(view.terrain, d.pos, d.vel, goal, &f);
        d.pos = p;
        d.vel = v;
        if let Some(t) = target {
            d.rot = drone::facing(t.pos - p);
        }
        let progress = (e.event.vars[V_HELD] * 255 / hold_ticks(e)).clamp(0, 255) as u8;
        d.state = [
            ROLE_PIZZA,
            progress,
            DRONE_WORKING,
            drone::hp_percent(d.hp, hp_max),
        ];
        e.ctx.pose_entity(&d);
    }

    fn on_end(&self, e: &mut EventCtx) {
        drone::remove_props(e.ctx, drone::DRONE);
    }
}
