//! Hat rain (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 25 s, `24 + 2 per player` hats drift down along the level's `up`
//! (`World::up`: they fall against it, also when Bored level turned it) at
//! 3 u/s from the upper third of the level and rest where they touch rock.
//! A ship that touches a hat wears it (`ShipState::hats`, replicated in the
//! ship's `ext` bits); a ship wearing 3 or more is revealed to everyone
//! while the event runs ([`HatRain::reveal`]). Hats are lost on death: they
//! scatter around the wreck as hats anyone can pick up for 10 s. When the
//! event ends the results name the **Hat Baron**: most hats worn, ties to
//! the one who reached that count first, then the lower player id
//! (`DirectorState::awards`, `MatchResults::awards`). Worn hats stay on the
//! hull after the event (no reveal any more) until the ship dies.
//!
//! - **Rain**: the hats appear one after another over the first
//!   `spawn_secs`, each at a random open point (at least 1.5 u from rock)
//!   in the upper third of the level along `up` (48 tries; a hat that finds
//!   none is skipped).
//! - **Entities**: `ReplicatedKind::Hat` props, state `[style, phase, 0,
//!   0]`: style 0-7 (top hat, cowboy, party cone, chef, crown, ...; the
//!   client's choice), phase 0 falling, 1 resting, 2 scattered (flying out
//!   of a wreck for 0.5 s), 3 scattered and resting.
//!
//! Numbers: `data/events/hat_rain.ron` (`hats_base`, `hats_per_player`,
//! `fall_speed`, `reveal_hats`, `drop_secs`, `hat_radius`, `spawn_secs`,
//! `scatter_speed`).

use ls_math::{Quat, Tick, Vec3};

use crate::entity::{EntityKind, EntitySpawn, ReplicatedKind};
use crate::events::{Award, EffectCtx, EventCtx, EventKind, EventModule};
use crate::ship::{PlayerId, ShipState};
use crate::Command;

/// Hat rain.
#[derive(Debug)]
pub struct HatRain;

const HAT: EntityKind = EntityKind::Prop(ReplicatedKind::Hat);

/// Hat styles the client draws (state byte 0 is below this).
pub const HAT_STYLES: u32 = 8;

/// `state[1]` of a hat.
pub mod phase {
    pub const FALLING: u8 = 0;
    pub const RESTING: u8 = 1;
    pub const SCATTERED: u8 = 2;
    pub const SCATTERED_RESTING: u8 = 3;
}

/// Ticks a scattered hat flies before it stops.
const SCATTER_TICKS: u64 = 30;
/// Least free space around a new hat (u).
const OPEN: f32 = 1.5;

/// Hats in the rain for `players` players.
pub fn hat_count(def: &crate::events::EventDef, players: usize) -> u32 {
    let n = def.param("hats_base", 24.0) + def.param("hats_per_player", 2.0) * players as f32;
    n.clamp(0.0, 200.0) as u32
}

fn unit_up(up: Vec3) -> Vec3 {
    up.try_normalize().unwrap_or(Vec3::Y)
}

/// A random open point in the upper third of the level along `up` for hat
/// `i` (see the module docs).
fn rain_point(e: &EventCtx, i: u64) -> Option<Vec3> {
    let b = e.ctx.view.terrain.bounds();
    let up = unit_up(e.ctx.view.up);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for k in 0..8 {
        let c = Vec3::new(
            if k & 1 == 0 { b.min.x } else { b.max.x },
            if k & 2 == 0 { b.min.y } else { b.max.y },
            if k & 4 == 0 { b.min.z } else { b.max.z },
        );
        lo = lo.min(c.dot(up));
        hi = hi.max(c.dot(up));
    }
    let floor = lo + (hi - lo) * (2.0 / 3.0);
    let mut rng = e.rng(0x4841_5400 + i);
    let inset = 1.0f32;
    let mut fallback = None;
    for _ in 0..48 {
        let p = Vec3::new(
            rng.range_f32(b.min.x + inset, (b.max.x - inset).max(b.min.x + inset)),
            rng.range_f32(b.min.y + inset, (b.max.y - inset).max(b.min.y + inset)),
            rng.range_f32(b.min.z + inset, (b.max.z - inset).max(b.min.z + inset)),
        );
        if e.ctx.view.terrain.sdf(p) < OPEN {
            continue;
        }
        if p.dot(up) >= floor {
            return Some(p);
        }
        fallback.get_or_insert(p);
    }
    fallback
}

fn mark(marks: &mut Vec<(PlayerId, u64)>, p: PlayerId, tick: Option<u64>) {
    let at = marks.binary_search_by_key(&p, |(q, _)| *q);
    match (at, tick) {
        (Ok(i), Some(t)) => marks[i].1 = t,
        (Err(i), Some(t)) => marks.insert(i, (p, t)),
        (Ok(i), None) => {
            marks.remove(i);
        }
        (Err(_), None) => {}
    }
}

/// The Hat Baron among `ships`: most hats, ties to the earliest mark
/// (the tick the count was reached), then the lower id. `None` when nobody
/// wears a hat.
pub fn hat_baron<'a>(
    ships: impl IntoIterator<Item = &'a ShipState>,
    marks: &[(PlayerId, u64)],
) -> Option<(PlayerId, u8)> {
    let tick_of = |p: PlayerId| {
        marks
            .binary_search_by_key(&p, |(q, _)| *q)
            .map_or(u64::MAX, |i| marks[i].1)
    };
    ships
        .into_iter()
        .filter(|s| s.hats > 0)
        .map(|s| (s.hats, tick_of(s.player), s.player))
        .min_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)))
        .map(|(h, _, p)| (p, h))
}

impl EventModule for HatRain {
    fn kind(&self) -> EventKind {
        EventKind::HatRain
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_start(&self, e: &mut EventCtx) {
        e.event.vars[0] = 0; // hats rained so far
        e.event.vars[1] = hat_count(e.def, e.ctx.view.ships.len()) as i64;
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let now = e.now();
        let def = e.def;
        let radius = def.param("hat_radius", 0.8).max(0.1);
        let fall = def.param("fall_speed", 3.0).max(0.0);
        let up = unit_up(e.ctx.view.up);
        let total = e.event.vars[1].max(0) as u64;
        let spawn_ticks = Tick::ticks_for_secs(def.param("spawn_secs", 8.0)).max(1);
        // 1. The rain: hat `i` appears at `active + i * spawn_ticks / total`.
        if e.event.phase == crate::events::EventPhase::Active {
            while (e.event.vars[0] as u64) < total {
                let i = e.event.vars[0] as u64;
                if e.event.active_tick + i * spawn_ticks / total > now {
                    break;
                }
                e.event.vars[0] += 1;
                let Some(p) = rain_point(e, i) else {
                    continue;
                };
                let style = e.rng(0x5354_0000 + i).below(HAT_STYLES) as u8;
                let mut s = EntitySpawn::prop(ReplicatedKind::Hat, p, Quat::IDENTITY, radius);
                s.vel = -up * fall;
                s.state = [style, phase::FALLING, 0, 0];
                e.ctx.spawn_entity(s);
            }
        }
        // 2. Falling hats rest where they touch rock; scattered ones stop.
        let hats: Vec<_> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == HAT)
            .map(|x| (x.id, x.pos, x.vel, x.rot, x.state, x.spawn_tick))
            .collect();
        let b = e.ctx.view.terrain.bounds();
        for &(id, pos, vel, rot, state, spawned) in &hats {
            let touching = e.ctx.view.terrain.sdf(pos) <= radius;
            let pose = |vel: Vec3, ph: u8| Command::PoseEntity {
                id,
                pos,
                vel,
                rot,
                state: [state[0], ph, state[2], state[3]],
            };
            match state[1] {
                phase::FALLING => {
                    let below = !b.contains(pos);
                    if touching || below {
                        e.ctx.push(pose(Vec3::ZERO, phase::RESTING));
                    } else if vel != -up * fall {
                        // The level's up turned (Bored level).
                        e.ctx.push(pose(-up * fall, phase::FALLING));
                    }
                }
                phase::SCATTERED if touching || now >= spawned + SCATTER_TICKS => {
                    e.ctx.push(pose(Vec3::ZERO, phase::SCATTERED_RESTING));
                }
                _ => {}
            }
        }
        // 3. Ships touching hats wear them (ships in id order, hats in id
        //    order; a hat goes to the first ship that touches it).
        let mut taken: Vec<u32> = Vec::new();
        let ships: Vec<(PlayerId, Vec3, f32, u8)> = e
            .ctx
            .view
            .ships
            .values()
            .filter(|s| s.alive)
            .map(|s| {
                let r = e.ctx.view.ship_types.get(s.ship_type).radius;
                (s.player, s.pos, r, s.hats)
            })
            .collect();
        for (player, pos, r, worn) in ships {
            let reach = r + radius;
            let mut got = 0u32;
            for &(id, hpos, ..) in &hats {
                if taken.contains(&id) || hpos.distance_squared(pos) > reach * reach {
                    continue;
                }
                taken.push(id);
                got += 1;
                e.ctx.push(Command::RemoveEntity { id });
            }
            if got > 0 {
                let hats = (worn as u32 + got).min(u8::MAX as u32) as u8;
                e.ctx.push(Command::SetHats {
                    target: player,
                    hats,
                });
                mark(&mut e.event.marks, player, Some(now));
            }
        }
        // 4. Hats scatter from the wrecks of this tick's kills.
        let drop = Tick::ticks_for_secs(def.param("drop_secs", 10.0)).max(1);
        let speed = def.param("scatter_speed", 4.0).max(0.0);
        let kills: Vec<_> = e.kills.to_vec();
        for k in kills {
            let worn = e.ctx.view.ships.get(&k.victim).map_or(0, |s| s.hats);
            if worn == 0 {
                continue;
            }
            let mut rng = e.rng(0x5343_0000 + k.victim.0 as u64);
            for j in 0..worn {
                let dir = Vec3::new(
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                )
                .try_normalize()
                .unwrap_or(up);
                let style = (k.victim.0 as u32 + j as u32) % HAT_STYLES;
                let mut s = EntitySpawn::prop(ReplicatedKind::Hat, k.pos, Quat::IDENTITY, radius);
                s.vel = dir * speed;
                s.lifetime_ticks = Some(drop);
                s.state = [style as u8, phase::SCATTERED, 0, 0];
                e.ctx.spawn_entity(s);
            }
            e.ctx.push(Command::SetHats {
                target: k.victim,
                hats: 0,
            });
            mark(&mut e.event.marks, k.victim, None);
        }
    }

    fn on_end(&self, e: &mut EventCtx) {
        if let Some((player, hats)) = hat_baron(e.ctx.view.ships.values(), &e.event.marks) {
            e.event.award = Some(Award {
                kind: EventKind::HatRain,
                player,
                value: hats as u32,
            });
        }
        let ids: Vec<u32> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == HAT)
            .map(|x| x.id)
            .collect();
        for id in ids {
            e.ctx.push(Command::RemoveEntity { id });
        }
    }

    fn reveal(&self, fx: &EffectCtx, ship: &ShipState) -> bool {
        ship.alive && ship.hats as f32 >= fx.def.param("reveal_hats", 3.0)
    }
}
