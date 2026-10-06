//! Bored level (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! The level's "up" turns `turn_degrees` (90) about a seeded axis
//! perpendicular to the current up over `turn_secs` (5 s, eased) and stays
//! turned. Terrain and everything in it keep their sim coordinates; what
//! turns is `World::up` (horizon assist levels to it, Level hiccups and Hat
//! rain follow it, the client turns sky, sun and fog).
//!
//! Replicated parameters (`MoodView::params`): bytes 0-2 the up at the
//! announce, bytes 3-5 the turn axis ([`crate::events::params::pack_dir`]).
//! The up of every tick is a pure function of them and the active phase's
//! clock ([`BoredLevel::up_at`]), so the client predictor's horizon assist
//! turns with the server's on the same tick. The active phase lasts a
//! little longer than the turn, so the final up is replicated before the
//! event ends.
//!
//! Numbers: `data/events/bored_level.ron`.

use ls_math::{det, Quat, Tick, Vec3};

use crate::events::params::{pack_dir, unpack_dir};
use crate::events::{EffectCtx, EventCtx, EventDef, EventKind, EventModule};

/// Bored level.
#[derive(Debug)]
pub struct BoredLevel;

impl BoredLevel {
    /// The turn's start up and axis from the parameter bytes.
    pub fn frame(params: &[u8; 8]) -> (Vec3, Vec3) {
        (
            unpack_dir([params[0], params[1], params[2]]),
            unpack_dir([params[3], params[4], params[5]]),
        )
    }

    /// The level's up `active_for` ticks into the active phase.
    pub fn up_at(def: &EventDef, params: &[u8; 8], active_for: u64) -> Vec3 {
        let (from, axis) = Self::frame(params);
        let turn = Tick::ticks_for_secs(def.param("turn_secs", 5.0)).max(1);
        let x = (active_for.min(turn) as f32) / turn as f32;
        // Smoothstep: no jolt at either end.
        let s = x * x * (3.0 - 2.0 * x);
        let angle = def.param("turn_degrees", 90.0).to_radians() * s;
        Quat::from_axis_angle(axis, angle)
            .rotate(from)
            .try_normalize()
            .unwrap_or(from)
    }

    /// A seeded unit axis perpendicular to `up`.
    fn axis(up: Vec3, angle: f32) -> Vec3 {
        let a = if up.x.abs() < 0.9 { Vec3::X } else { Vec3::Z };
        let e1 = up.cross(a).normalize();
        let e2 = up.cross(e1);
        let (s, c) = det::sin_cos(angle);
        (e1 * c + e2 * s).normalize()
    }
}

impl EventModule for BoredLevel {
    fn kind(&self) -> EventKind {
        EventKind::BoredLevel
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_announce(&self, e: &mut EventCtx) {
        let up = e.ctx.view.up;
        let mut rng = e.rng(1);
        let angle = rng.range_f32(0.0, std::f32::consts::TAU);
        // The axis is perpendicular to the up both ends decode.
        let from = unpack_dir(pack_dir(up));
        let axis = Self::axis(from, angle);
        let (u, a) = (pack_dir(from), pack_dir(axis));
        e.event.params[..3].copy_from_slice(&u);
        e.event.params[3..6].copy_from_slice(&a);
    }

    fn level_up(&self, fx: &EffectCtx) -> Option<Vec3> {
        Some(Self::up_at(fx.def, &fx.view.params, fx.active_for()))
    }
}
