//! Level hiccups (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 15 s, every `interval_secs` (3 s) plus or minus `jitter_secs` (1 s,
//! seeded) the level hiccups: ships, debris and projectiles in flight
//! (unstuck mines included) are lifted about `lift` (2 u) along the level's
//! up over `lift_secs` (0.25 s), with normal collisions.
//!
//! - Ships ([`EventModule::move_effects`], also applied by the client
//!   predictor): an impulse along up at the hiccup tick, sized so a ship
//!   without input on that axis rises `lift` against its dampers
//!   (`damping_decel`, 8 u/s^2 for every ship type), and at the end of the
//!   lift the rise is taken back (at most the impulse), so the ship stops
//!   where it was lifted to.
//! - Debris: the same impulse and stop through `Command::ImpulseEntity`.
//! - Projectiles in flight: nudged by `lift` at once, swept against rock
//!   (`Command::NudgeProjectile`); stuck mines stay.
//!
//! The hiccup ticks are a pure function of the match seed and the active
//! phase's start tick ([`hiccup_ticks`]); the up is packed into the
//! parameter bytes at the announce (bytes 0-2), so the predictor knows both.
//!
//! Numbers: `data/events/level_hiccups.ron`.

use ls_math::{Tick, Vec3};

use crate::entity::EntityKind;
use crate::events::params::{pack_dir, unpack_dir};
use crate::events::{EffectCtx, EventCtx, EventDef, EventKind, EventModule, MoveEffects};
use crate::ship::ShipState;

/// RNG key of the hiccup schedule.
const KEY_SCHEDULE: u64 = 0x4849_4343; // "HICC"

/// Level hiccups.
#[derive(Debug)]
pub struct LevelHiccups;

/// The ticks of every hiccup of an event whose active phase started at
/// `active_tick`: the first `interval +- jitter` seconds in, then the same
/// spacing again, as long as the lift ends inside the active phase.
pub fn hiccup_ticks(seed: u64, active_tick: u64, def: &EventDef) -> Vec<u64> {
    let mut rng = crate::director::mood_rng(
        seed,
        active_tick,
        ((EventKind::LevelHiccups.index() as u64) << 32) ^ KEY_SCHEDULE,
    );
    let interval = def.param("interval_secs", 3.0).max(0.5);
    let jitter = def.param("jitter_secs", 1.0).clamp(0.0, interval - 0.25);
    let lift = lift_ticks(def);
    let end = active_tick + def.active_ticks();
    let mut out = Vec::new();
    let mut t = active_tick;
    loop {
        let secs = interval + rng.range_f32(-jitter, jitter);
        t += Tick::ticks_for_secs(secs).max(1);
        if t + lift > end {
            return out;
        }
        out.push(t);
    }
}

/// Ticks a lift lasts.
pub fn lift_ticks(def: &EventDef) -> u64 {
    Tick::ticks_for_secs(def.param("lift_secs", 0.25)).max(1)
}

/// Speed of the lift impulse (u/s): `lift` over `lift_secs` plus what the
/// dampers take back meanwhile (half of `damping * lift_secs`, on average).
pub fn lift_speed(def: &EventDef) -> f32 {
    let secs = lift_ticks(def) as f32 * ls_math::DT;
    def.param("lift", 2.0) / secs + 0.5 * def.param("damping", 8.0) * secs
}

/// What happens to something at `now`: a hiccup starts, a lift ends, or
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Lift,
    Stop,
}

/// The hiccup phase at `now` for an event whose active phase started at
/// `active_tick`.
pub fn phase_at(seed: u64, active_tick: u64, def: &EventDef, now: u64) -> Option<Phase> {
    let lift = lift_ticks(def);
    hiccup_ticks(seed, active_tick, def)
        .into_iter()
        .find_map(|h| {
            if now == h {
                Some(Phase::Lift)
            } else if now == h + lift {
                Some(Phase::Stop)
            } else {
                None
            }
        })
}

/// The up the hiccups lift along (packed at the announce).
pub fn lift_dir(params: &[u8; 8]) -> Vec3 {
    unpack_dir([params[0], params[1], params[2]])
}

/// The velocity change of something moving at `vel` at `phase`.
fn kick(phase: Phase, up: Vec3, speed: f32, vel: Vec3) -> Vec3 {
    match phase {
        Phase::Lift => up * speed,
        Phase::Stop => -up * vel.dot(up).clamp(0.0, speed),
    }
}

impl EventModule for LevelHiccups {
    fn kind(&self) -> EventKind {
        EventKind::LevelHiccups
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_announce(&self, e: &mut EventCtx) {
        let up = pack_dir(e.ctx.view.up);
        e.event.params[..3].copy_from_slice(&up);
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let now = e.now();
        let seed = e.ctx.view.seed;
        let Some(phase) = phase_at(seed, e.event.active_tick, e.def, now) else {
            return;
        };
        let up = lift_dir(&e.event.params);
        let speed = lift_speed(e.def);
        let debris: Vec<(u32, Vec3)> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|en| en.kind == EntityKind::Debris)
            .map(|en| (en.id, kick(phase, up, speed, en.vel) * en.mass()))
            .collect();
        for (id, dv) in debris {
            if dv != Vec3::ZERO {
                e.ctx.impulse_entity(id, dv);
            }
        }
        if phase == Phase::Lift {
            let lift = e.def.param("lift", 2.0);
            let ids: Vec<u32> = e
                .ctx
                .projectiles()
                .filter(|p| !p.is_stuck())
                .map(|p| p.id)
                .collect();
            for id in ids {
                e.ctx.push(crate::Command::NudgeProjectile {
                    id,
                    delta: up * lift,
                });
            }
        }
    }

    fn move_effects(&self, fx: &EffectCtx, ship: &ShipState) -> MoveEffects {
        let Some(phase) = phase_at(fx.seed, fx.view.active_tick, fx.def, fx.now) else {
            return MoveEffects::NONE;
        };
        let dv = kick(
            phase,
            lift_dir(&fx.view.params),
            lift_speed(fx.def),
            ship.vel,
        );
        MoveEffects {
            impulse: dv,
            ..MoveEffects::NONE
        }
    }
}
