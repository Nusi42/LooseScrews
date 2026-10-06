//! Duck parade (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! Five giant rubber ducks (3.5 u) drift in a convoy along a network route
//! from one end of the level to the other at 5 u/s (45 s at most). They
//! follow the route's centre line and ignore rock; they absorb projectiles
//! (no damage) and push ships aside (no damage), as kinematic obstacles
//! ([`crate::obstacle`]). On belt levels the parade flies the ring's centre
//! line retrograde, against the racers (§3.8).
//!
//! Positions are a pure function of the replicated [`MoodView`] (active
//! tick and the route bytes), the tick and the [`MatchSetup`] (which every
//! client builds from the share code), so the client predictor knows every
//! duck exactly ([`DuckParade::obstacles`]).
//!
//! - **Route** (`params[0..2]`, picked at the announcement on the
//!   `SIM_MOOD` stream): classic levels take `MatchSetup::routes[params[0]]`
//!   (`ls_game::setup`: hub to hub through the tunnels from low x to high
//!   x), reversed when `params[1]` is odd; test rooms without routes cross
//!   between the two spawn points farthest apart; belt levels start the
//!   ring at azimuth `params[0] / 256` turns and go round retrograde.
//! - **Convoy**: duck `i` (0 leads) is `(ducks - 1 - i) x spacing` along
//!   the route at the start (the whole convoy is on the route from the
//!   first tick) and moves at `speed`; a duck past the route's end has
//!   left (its entity is removed). The active phase ends when the last
//!   duck reaches the end, or after `active_secs`.
//! - **Entities**: one `ReplicatedKind::Duck` prop per duck, posed every
//!   tick; state `[index, clearance, 0, 0]` with the free space around the
//!   duck in tenths of a unit (255 = 25.5 u or more; the client squashes
//!   ducks in narrow tunnels).
//!
//! Numbers: `data/events/duck_parade.ron` (`ducks`, `duck_radius`, `speed`,
//! `spacing`).

use ls_math::{det, Quat, Vec3, DT};

use crate::director::MoodView;
use crate::entity::{EntityKind, EntitySpawn, ReplicatedKind};
use crate::events::{EffectCtx, EventCtx, EventDef, EventKind, EventModule, EventRef};
use crate::obstacle::{Obstacle, ObstacleKind};
use crate::world::MatchSetup;
use crate::Command;

/// Duck parade.
#[derive(Debug)]
pub struct DuckParade;

/// Segments of the belt ring's polyline.
const RING_SEGMENTS: usize = 64;

/// A polyline with its cumulative arc length.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    points: Vec<Vec3>,
    /// `cum[i]`: arc length from the start to `points[i]`.
    cum: Vec<f32>,
}

impl Route {
    /// A route along `points` (consecutive duplicates dropped); `None` with
    /// fewer than two distinct points.
    pub fn new(points: impl IntoIterator<Item = Vec3>) -> Option<Route> {
        let mut pts: Vec<Vec3> = Vec::new();
        for p in points {
            if pts.last().is_none_or(|q| q.distance_squared(p) > 1e-8) {
                pts.push(p);
            }
        }
        if pts.len() < 2 {
            return None;
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut acc = 0.0f32;
        cum.push(0.0);
        for w in pts.windows(2) {
            acc += w[0].distance(w[1]);
            cum.push(acc);
        }
        Some(Route { points: pts, cum })
    }

    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    /// Total length (u).
    pub fn length(&self) -> f32 {
        *self.cum.last().unwrap_or(&0.0)
    }

    /// Position and unit direction at arc length `s` (clamped to the route).
    pub fn at(&self, s: f32) -> (Vec3, Vec3) {
        let s = s.clamp(0.0, self.length());
        // Last segment whose start is at or before `s`.
        let i = self
            .cum
            .partition_point(|c| *c <= s)
            .saturating_sub(1)
            .min(self.points.len() - 2);
        let (a, b) = (self.points[i], self.points[i + 1]);
        let len = self.cum[i + 1] - self.cum[i];
        let dir = (b - a) / len;
        (a + dir * (s - self.cum[i]), dir)
    }
}

/// The parade's route for the route bytes `params` on the level `setup`
/// (see the module docs); `None` when the level has none.
pub fn route(setup: &MatchSetup, params: &[u8; 8]) -> Option<Route> {
    if let Some(belt) = &setup.belt {
        let r = 0.5 * (belt.inner_radius + belt.outer_radius);
        let a0 = params[0] as f32 * (core::f32::consts::TAU / 256.0);
        return Route::new((0..=RING_SEGMENTS).map(|k| {
            // Retrograde: the azimuth falls (prograde is counter-clockwise
            // from above, `BeltSite::azimuth`).
            let a = a0 - core::f32::consts::TAU * k as f32 / RING_SEGMENTS as f32;
            let (s, c) = det::sin_cos(a);
            belt.center + Vec3::new(r * c, 0.0, -r * s)
        }));
    }
    let reverse = params[1] & 1 == 1;
    let pts: Vec<Vec3> = if !setup.routes.is_empty() {
        setup.routes[params[0] as usize % setup.routes.len()].clone()
    } else {
        // Test rooms: the two spawn points farthest apart (first pair wins).
        let sp = &setup.spawns;
        let mut best: Option<(f32, usize, usize)> = None;
        for i in 0..sp.len() {
            for j in i + 1..sp.len() {
                let d = sp[i].pos.distance_squared(sp[j].pos);
                if best.is_none_or(|(bd, _, _)| d > bd) {
                    best = Some((d, i, j));
                }
            }
        }
        let (_, i, j) = best?;
        vec![sp[i].pos, sp[j].pos]
    };
    if reverse {
        Route::new(pts.into_iter().rev())
    } else {
        Route::new(pts)
    }
}

/// The parade's numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Convoy {
    pub ducks: usize,
    pub radius: f32,
    pub speed: f32,
    /// Distance between neighbouring ducks along the route (shrunk so the
    /// whole convoy fits on a short route).
    pub spacing: f32,
}

impl Convoy {
    pub fn new(def: &EventDef, route: &Route) -> Convoy {
        let ducks = def.param("ducks", 5.0).clamp(1.0, 16.0) as usize;
        let spacing = def
            .param("spacing", 9.0)
            .min(route.length() / ducks as f32)
            .max(0.0);
        Convoy {
            ducks,
            radius: def.param("duck_radius", 3.5).max(0.1),
            speed: def.param("speed", 5.0).max(0.1),
            spacing,
        }
    }

    /// Arc length of duck `i` after `secs` of the active phase.
    pub fn arc(&self, i: usize, secs: f32) -> f32 {
        (self.ducks - 1 - i) as f32 * self.spacing + self.speed * secs
    }

    /// Seconds until the last duck reaches the end of `route`.
    pub fn crossing_secs(&self, route: &Route) -> f32 {
        route.length() / self.speed
    }
}

/// Every duck still on the route at `now` (`(index, centre, velocity)`),
/// for the parade in `view` on `setup`.
pub fn ducks_at(
    view: &MoodView,
    def: &EventDef,
    setup: &MatchSetup,
    now: u64,
) -> Vec<(usize, Vec3, Vec3)> {
    let Some(route) = route(setup, &view.params) else {
        return Vec::new();
    };
    let convoy = Convoy::new(def, &route);
    let secs = now.saturating_sub(view.active_tick) as f32 * DT;
    let len = route.length();
    (0..convoy.ducks)
        .filter_map(|i| {
            let s = convoy.arc(i, secs);
            (s <= len).then(|| {
                let (p, d) = route.at(s);
                (i, p, d * convoy.speed)
            })
        })
        .collect()
}

fn facing(dir: Vec3) -> Quat {
    if dir.length_squared() < 1e-12 {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize())
    }
}

const DUCK: EntityKind = EntityKind::Prop(ReplicatedKind::Duck);

impl EventModule for DuckParade {
    fn kind(&self) -> EventKind {
        EventKind::DuckParade
    }

    fn implemented(&self) -> bool {
        true
    }

    fn eligible(&self, e: &EventRef) -> bool {
        route(e.setup, &[0; 8]).is_some()
    }

    fn on_announce(&self, e: &mut EventCtx) {
        let mut rng = e.rng(0);
        let (a, b) = (rng.next_u32(), rng.next_u32());
        e.event.params[0] = if e.setup.belt.is_some() || e.setup.routes.is_empty() {
            (a & 0xff) as u8
        } else {
            (a % e.setup.routes.len() as u32) as u8
        };
        e.event.params[1] = (b & 1) as u8;
        if let Some(r) = route(e.setup, &e.event.params) {
            e.event.pos = r.at(0.0).0;
        }
    }

    fn on_start(&self, e: &mut EventCtx) {
        let Some(route) = route(e.setup, &e.event.params) else {
            return;
        };
        let convoy = Convoy::new(e.def, &route);
        // Over once the last duck has left the route.
        let ticks = ls_math::Tick::ticks_for_secs(convoy.crossing_secs(&route)) + 1;
        e.event.phase_end_tick = e.event.phase_end_tick.min(e.event.active_tick + ticks);
        for i in 0..convoy.ducks {
            let (p, d) = route.at(convoy.arc(i, 0.0));
            let mut spawn = EntitySpawn::prop(ReplicatedKind::Duck, p, facing(d), convoy.radius);
            spawn.vel = d * convoy.speed;
            spawn.state = [i as u8, 0, 0, 0];
            e.ctx.spawn_entity(spawn);
        }
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let view = MoodView {
            active_tick: e.event.active_tick,
            params: e.event.params,
            ..MoodView::default()
        };
        let present = ducks_at(&view, e.def, e.setup, e.now());
        let ducks: Vec<(u32, usize)> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == DUCK)
            .map(|x| (x.id, x.state[0] as usize))
            .collect();
        for (id, i) in ducks {
            match present.iter().find(|(k, _, _)| *k == i) {
                Some(&(_, pos, vel)) => {
                    let clear = e.ctx.view.terrain.sdf(pos).clamp(0.0, 25.5);
                    let clear = det::round(clear * 10.0) as u8;
                    e.ctx.push(Command::PoseEntity {
                        id,
                        pos,
                        vel,
                        rot: facing(vel),
                        state: [i as u8, clear, 0, 0],
                    });
                }
                None => e.ctx.push(Command::RemoveEntity { id }),
            }
        }
    }

    fn on_end(&self, e: &mut EventCtx) {
        let ids: Vec<u32> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == DUCK)
            .map(|x| x.id)
            .collect();
        for id in ids {
            e.ctx.push(Command::RemoveEntity { id });
        }
    }

    fn obstacles(&self, fx: &EffectCtx, setup: &MatchSetup, out: &mut Vec<Obstacle>) {
        let radius = fx.def.param("duck_radius", 3.5).max(0.1);
        for (_, pos, vel) in ducks_at(fx.view, fx.def, setup, fx.now) {
            out.push(Obstacle {
                kind: ObstacleKind::Duck,
                pos,
                vel,
                radius,
            });
        }
    }
}
