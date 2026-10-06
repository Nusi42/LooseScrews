//! Tax drone (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! A drone enters from the nearest boundary cavern (at least
//! `min_entry_distance` from the leader; it waits there with its siren
//! during the announce phase) and chases the leader at 22 u/s along the
//! network path with local avoidance ([`crate::events::drone`]). Each touch
//! takes 10 % of the leader's current energy (2 s apart, at most 5; after
//! the fifth it leaves). It has 40 hp; shot down it bursts into a scrap
//! pile of 2 scrap per energy taken (at most 20). Anyone can shoot it, the
//! leader too. After 25 s, or the fifth touch, it flies back to its entry
//! during the 1.5 s ending phase.
//!
//! Drone entity (`ReplicatedKind::Drone`): `state` = `[ROLE_TAX, touches,
//! phase (waiting, working, leaving), hp percent]`. Outcome cues
//! (`CueKind::EventOutcome`): touch n (1-5) with the leader as target;
//! `OUTCOME_SHOT_DOWN` with the shooter as source.
//!
//! Numbers: `data/events/tax_drone.ron`.

use ls_math::Tick;

use crate::behavior::{Command, Cue, CueKind};
use crate::events::drone::{
    self, Flight, DRONE_LEAVING, DRONE_WAITING, DRONE_WORKING, OUTCOME_SHOT_DOWN, ROLE_TAX,
};
use crate::events::{EventCtx, EventKind, EventModule, EventPhase, EventRef};

/// Tax drone.
#[derive(Debug)]
pub struct TaxDrone;

/// `EventState::vars`: touches so far, tick of the next allowed touch, done
/// (shot down or out of touches).
const V_TOUCHES: usize = 0;
const V_NEXT_TOUCH: usize = 1;
const V_DONE: usize = 2;
/// `EventState::fvars`: energy taken.
const F_TAKEN: usize = 0;

fn flight(e: &EventCtx) -> Flight {
    Flight {
        speed: e.def.param("speed", 22.0),
        accel: e.def.param("accel", 60.0),
        radius: e.def.param("radius", 0.9),
    }
}

fn cue(e: &mut EventCtx, source: Option<crate::PlayerId>, pos: ls_math::Vec3, byte: u8) {
    let target = e.event.target;
    e.ctx.cue(Cue {
        kind: CueKind::EventOutcome,
        source,
        target,
        pos,
        value: drone::outcome(EventKind::TaxDrone, byte),
    });
}

impl EventModule for TaxDrone {
    fn kind(&self) -> EventKind {
        EventKind::TaxDrone
    }

    fn implemented(&self) -> bool {
        true
    }

    /// Needs a clear leader (`ModeModule::leader`).
    fn eligible(&self, e: &EventRef) -> bool {
        e.leader.is_some()
    }

    /// Names the leader; the drone waits at its entry cavern.
    fn on_announce(&self, e: &mut EventCtx) {
        e.event.target = e.leader;
        let f = flight(e);
        let toward = drone::living(&e.ctx.view, e.leader)
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
            e.def.param("hp", 40.0),
            [ROLE_TAX, 0, DRONE_WAITING, 100],
        );
        spawn.rot = drone::facing(toward - entry);
        e.ctx.spawn_entity(spawn);
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let Some(mut d) = drone::first_prop(&e.ctx.view, drone::DRONE) else {
            return;
        };
        let now = e.now();
        let f = flight(e);
        let hp_max = e.def.param("hp", 40.0);
        if d.hp <= 0.0 {
            // Shot down: a scrap pile of what it took.
            let scrap = (e.event.fvars[F_TAKEN] * e.def.param("scrap_per_energy", 2.0))
                .min(e.def.param("max_scrap", 20.0));
            e.ctx.spawn_scrap(d.pos, scrap);
            cue(e, d.last_hit.map(|(_, by)| by), d.pos, OUTCOME_SHOT_DOWN);
            e.ctx.push(Command::RemoveEntity { id: d.id });
            e.event.vars[V_DONE] = 1;
            drone::end_now(e);
            return;
        }
        let view = e.ctx.view;
        if e.event.phase == EventPhase::Ending || e.event.vars[V_DONE] != 0 {
            // Leaving: back to the entry cavern.
            let goal = drone::plan(&view, &mut d, e.event.pos, f.radius);
            let (p, v) = drone::fly(view.terrain, d.pos, d.vel, goal, &f);
            (d.pos, d.vel, d.rot) = (p, v, drone::facing(v));
            d.state = [
                ROLE_TAX,
                d.state[1],
                DRONE_LEAVING,
                drone::hp_percent(d.hp, hp_max),
            ];
            e.ctx.pose_entity(&d);
            return;
        }
        let target = drone::living(&view, e.event.target);
        let goal = match target {
            // The leader is down: wait where it is.
            None => d.pos,
            Some(t) => {
                let touch = e.ctx.tuning_of(t).radius + f.radius + e.def.param("touch_margin", 0.3);
                let dist = d.pos.distance(t.pos);
                if dist <= touch && now as i64 >= e.event.vars[V_NEXT_TOUCH] {
                    let amount = t.energy * e.def.param("energy_fraction", 0.1);
                    e.ctx.drain_energy(t.player, None, amount);
                    e.event.fvars[F_TAKEN] += amount;
                    let touches = e.event.vars[V_TOUCHES] + 1;
                    e.event.vars[V_TOUCHES] = touches;
                    let gap = Tick::ticks_for_secs(e.def.param("touch_interval_secs", 2.0));
                    e.event.vars[V_NEXT_TOUCH] = (now + gap) as i64;
                    cue(e, None, d.pos, touches.clamp(1, 255) as u8);
                    if touches >= e.def.param("max_touches", 5.0) as i64 {
                        e.event.vars[V_DONE] = 1;
                        drone::end_now(e);
                    }
                }
                if dist <= touch * 2.0 {
                    // Close: press against the hull, never inside it.
                    t.pos + (d.pos - t.pos).normalize_or_zero() * (touch * 0.8)
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
        d.state = [
            ROLE_TAX,
            e.event.vars[V_TOUCHES].clamp(0, 255) as u8,
            DRONE_WORKING,
            drone::hp_percent(d.hp, hp_max),
        ];
        e.ctx.pose_entity(&d);
    }

    fn on_end(&self, e: &mut EventCtx) {
        drone::remove_props(e.ctx, drone::DRONE);
    }
}
