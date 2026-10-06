//! Disco inferno (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 20 s the sun and ambient light drop to 8 % (`light_level`; the
//! client dims them while the [`MoodView`](crate::director::MoodView) names
//! the event), a mirror ball descends over the living ships' centroid and
//! every ship leaves a light trail in its side colour (client only). No
//! other simulation rule: nothing here moves ships, hurts or blocks.
//!
//! The ball (`ReplicatedKind::DiscoBall` prop, radius `ball_radius`) hangs
//! `hang` u above the centroid along the level's up (closer when a ceiling
//! is nearer, at least 2 u), comes down from `drop` u higher over
//! `descend_secs` and then drifts after the moving centroid at up to
//! `follow_speed` u/s; it spins about the up axis. State `[light percent,
//! descent progress 0-255, 0, 0]`.
//!
//! Numbers: `data/events/disco_inferno.ron`.

use ls_math::{Quat, Tick, Vec3, DT};

use crate::entity::{EntityKind, EntitySpawn, ReplicatedKind};
use crate::events::{EventCtx, EventDef, EventKind, EventModule, EventPhase};
use crate::Command;

/// Disco inferno.
#[derive(Debug)]
pub struct DiscoInferno;

const BALL: EntityKind = EntityKind::Prop(ReplicatedKind::DiscoBall);

/// Smoothstep on `[0, 1]`.
fn ease(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Centroid of the living ships (`None` without any).
fn centroid(e: &EventCtx) -> Option<Vec3> {
    let (sum, n) = e
        .ctx
        .view
        .ships
        .values()
        .filter(|s| s.alive)
        .fold((Vec3::ZERO, 0u32), |(a, n), s| (a + s.pos, n + 1));
    (n > 0).then(|| sum / n as f32)
}

/// Where the ball hangs over `c`: `hang` along `up`, closer under a
/// ceiling (2 u below it, at least 2 u above `c`).
fn hang_point(e: &EventCtx, def: &EventDef, c: Vec3, up: Vec3) -> Vec3 {
    let hang = def.param("hang", 10.0).max(2.0);
    let radius = def.param("ball_radius", 1.5);
    let h = e
        .ctx
        .view
        .terrain
        .raycast(c, up, hang + radius + 2.0)
        .map_or(hang, |r| (r.distance - radius - 2.0).clamp(2.0, hang));
    c + up * h
}

/// Light level the client dims the sun and ambient light to.
pub fn light_level(def: &EventDef) -> f32 {
    def.param("light_level", 0.08).clamp(0.0, 1.0)
}

impl EventModule for DiscoInferno {
    fn kind(&self) -> EventKind {
        EventKind::DiscoInferno
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_start(&self, e: &mut EventCtx) {
        let def = e.def;
        let up = e.ctx.view.up.try_normalize().unwrap_or(Vec3::Y);
        let c = centroid(e).unwrap_or_else(|| e.ctx.view.terrain.bounds().center());
        let target = hang_point(e, def, c, up);
        let top = target + up * def.param("drop", 15.0);
        e.event.pos = target;
        let mut s = EntitySpawn::prop(
            ReplicatedKind::DiscoBall,
            top,
            Quat::IDENTITY,
            def.param("ball_radius", 1.5),
        );
        s.state = [(light_level(def) * 100.0) as u8, 0, 0, 0];
        e.ctx.spawn_entity(s);
    }

    fn on_tick(&self, e: &mut EventCtx) {
        if e.event.phase == EventPhase::Announce {
            return;
        }
        let def = e.def;
        let up = e.ctx.view.up.try_normalize().unwrap_or(Vec3::Y);
        let k = e.now().saturating_sub(e.event.active_tick);
        // The hang point drifts after the centroid.
        if let Some(c) = centroid(e) {
            let want = hang_point(e, def, c, up);
            let step = def.param("follow_speed", 3.0).max(0.0) * DT;
            let d = want - e.event.pos;
            let len = d.length();
            e.event.pos = if len <= step {
                want
            } else {
                e.event.pos + d * (step / len)
            };
        }
        let descend = Tick::ticks_for_secs(def.param("descend_secs", 3.0)).max(1);
        let u = (k as f32 / descend as f32).min(1.0);
        let drop = def.param("drop", 15.0) * (1.0 - ease(u));
        let pos = e.event.pos + up * drop;
        let spin = ls_math::det::wrap_angle(k as f32 * DT * def.param("spin", 1.2));
        let rot = Quat::from_axis_angle(up, spin);
        let state = [(light_level(def) * 100.0) as u8, (u * 255.0) as u8, 0, 0];
        let balls: Vec<(u32, Vec3)> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == BALL)
            .map(|x| (x.id, x.pos))
            .collect();
        for (id, old) in balls {
            e.ctx.push(Command::PoseEntity {
                id,
                pos,
                vel: (pos - old) * ls_math::TICK_HZ as f32,
                rot,
                state,
            });
        }
    }

    fn on_end(&self, e: &mut EventCtx) {
        let ids: Vec<u32> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == BALL)
            .map(|x| x.id)
            .collect();
        for id in ids {
            e.ctx.push(Command::RemoveEntity { id });
        }
    }
}
