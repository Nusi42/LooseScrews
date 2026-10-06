//! A simple, deterministic racing autopilot for Belt Up: in-process
//! bots-in-sim for tests, replays and the netplay test (the real racing
//! bots are `ls-bots`, package C5).
//!
//! It flies prograde along a racing line (a closed polyline in prograde
//! order, e.g. the generator's clear corridor, or a circle in the belt
//! plane), through the gates in order, and after the lap back to its start
//! slot and straight down the dive lane to the exit. Inputs depend only on
//! the world state, so a replay with the autopilot is deterministic.

use ls_math::level::{BeltSite, BELT_WEDGES};
use ls_math::{det, Vec3, DT};

use crate::modes::belt_up::LAP_GATES;
use crate::ship::PlayerId;
use crate::{Buttons, InputCmd, ShipState, ShipTuning, World};

/// How an autopilot flies.
#[derive(Debug, Clone, PartialEq)]
pub struct PilotStyle {
    /// The racing line (closed, prograde order). Empty: a circle of
    /// `circle_radius` about the belt axis.
    pub line: Vec<Vec3>,
    /// Radius of the circle line (u).
    pub circle_radius: f32,
    /// Height above the belt plane (u; the open line flies above the belt).
    pub height: f32,
    /// How far ahead along the line it aims (u).
    pub lookahead: f32,
    /// Boost while lined up.
    pub boost: bool,
    /// Fire the primary at a ship in front of it.
    pub shoot: bool,
    /// Forward thrust (0-1).
    pub throttle: f32,
    /// Fly over or under drifting boulders in the way.
    pub avoid_boulders: bool,
    /// Ram the nearest ship ahead within this distance (0 = never).
    pub ram_range: f32,
}

impl Default for PilotStyle {
    fn default() -> Self {
        PilotStyle {
            line: Vec::new(),
            circle_radius: 120.0,
            height: 0.0,
            lookahead: 10.0,
            boost: true,
            shoot: false,
            throttle: 1.0,
            avoid_boulders: true,
            ram_range: 0.0,
        }
    }
}

/// Point of the circle line at azimuth `a` (prograde from +X).
fn circle_point(b: &BeltSite, radius: f32, height: f32, a: f32) -> Vec3 {
    let (s, c) = det::sin_cos(a);
    b.center + Vec3::new(c * radius, height, -s * radius)
}

/// The point the pilot aims at while lapping.
fn line_target(b: &BeltSite, style: &PilotStyle, pos: Vec3) -> Vec3 {
    if style.line.len() < 2 {
        let r = style.circle_radius.max(1.0);
        let a = b.azimuth(pos) + style.lookahead / r;
        return circle_point(b, r, style.height, a);
    }
    let n = style.line.len();
    let nearest = (0..n)
        .min_by(|&i, &j| {
            style.line[i]
                .distance_squared(pos)
                .total_cmp(&style.line[j].distance_squared(pos))
        })
        .unwrap_or(0);
    // Walk `lookahead` along the line.
    let mut left = style.lookahead;
    let mut i = nearest;
    for _ in 0..n {
        let j = (i + 1) % n;
        let seg = style.line[i].distance(style.line[j]);
        if seg >= left {
            let t = if seg > 0.0 { left / seg } else { 0.0 };
            let p = style.line[i] + (style.line[j] - style.line[i]) * t;
            return p + Vec3::new(0.0, style.height, 0.0);
        }
        left -= seg;
        i = j;
    }
    style.line[nearest]
}

/// What the autopilot needs to know about its ship and the race (a world
/// or a client session can fill it).
#[derive(Debug, Clone, Copy)]
pub struct PilotView<'a> {
    pub ship: &'a ShipState,
    pub tuning: &'a ShipTuning,
    pub belt: &'a BeltSite,
    /// Gates passed and start slot this heat.
    pub gates: u8,
    pub slot: u8,
    /// Positions of the other living ships.
    pub others: &'a [Vec3],
}

/// The autopilot's command for `id` at `tick` (idle without a ship, a race
/// or a belt).
pub fn steer(world: &World, id: PlayerId, style: &PilotStyle, tick: u64) -> InputCmd {
    let idle = InputCmd::idle(tick);
    let Some(ship) = world.ship(id).filter(|s| s.alive) else {
        return idle;
    };
    let Some(belt) = world.setup().belt.as_ref() else {
        return idle;
    };
    let Some(racer) = world.mode().race.as_ref().and_then(|r| r.racers.get(&id)) else {
        return idle;
    };
    let others: Vec<Vec3> = world
        .ships()
        .filter(|o| o.alive && o.player != id)
        .map(|o| o.pos)
        .collect();
    steer_ship(
        &PilotView {
            ship,
            tuning: world.tuning_for(ship.ship_type),
            belt,
            gates: racer.gates,
            slot: racer.slot,
            others: &others,
        },
        style,
        tick,
    )
}

/// The autopilot's command for the ship of `v` at `tick`.
pub fn steer_ship(v: &PilotView, style: &PilotStyle, tick: u64) -> InputCmd {
    let (ship, b) = (v.ship, v.belt);
    let target = if v.gates < LAP_GATES {
        line_target(b, style, ship.pos)
    } else {
        // Lap done: back to the own slot, then down the dive lane.
        let slot = b.slots[v.slot as usize % BELT_WEDGES].pos;
        let lane = slot - b.center;
        let along = (ship.pos - b.center).dot(lane) / lane.length_squared().max(1.0);
        let on_lane = b.center + lane * along.clamp(0.0, 1.0);
        if ship.pos.distance(on_lane) > 2.5 && along > 0.2 {
            on_lane + (b.center - on_lane).try_normalize().unwrap_or(Vec3::ZERO) * 2.0
        } else {
            b.center
        }
    };
    let target = if style.avoid_boulders {
        avoid_boulders(b, ship.pos, target, tick)
    } else {
        target
    };
    let target = if style.ram_range > 0.0 && v.gates < LAP_GATES {
        let fwd = (target - ship.pos).try_normalize().unwrap_or(Vec3::ZERO);
        v.others
            .iter()
            .copied()
            .filter(|p| {
                let d = *p - ship.pos;
                d.length() < style.ram_range && d.dot(fwd) > 0.0
            })
            .min_by(|a, c| a.distance(ship.pos).total_cmp(&c.distance(ship.pos)))
            .unwrap_or(target)
    } else {
        target
    };
    aim_at(v, target, style, tick)
}

/// Lifts (or lowers) the aim point over a drifting boulder in the way.
fn avoid_boulders(b: &BeltSite, pos: Vec3, target: Vec3, tick: u64) -> Vec3 {
    let Some(dir) = (target - pos).try_normalize() else {
        return target;
    };
    let mut out = target;
    for (c, size) in crate::boulders::poses(b, tick) {
        let d = c - pos;
        let ahead = d.dot(dir);
        if !(0.0..20.0).contains(&ahead) {
            continue;
        }
        let side = d - dir * ahead;
        if side.length() < size + 2.5 {
            let up = if c.y > pos.y { -1.0 } else { 1.0 };
            out = Vec3::new(out.x, c.y + up * (size + 3.0), out.z);
        }
    }
    out
}

fn aim_at(v: &PilotView, target: Vec3, style: &PilotStyle, tick: u64) -> InputCmd {
    let ship = v.ship;
    let to = target - ship.pos;
    let local = ship.orientation.conjugate().rotate(to);
    let dist = local.length().max(1e-3);
    let dir = local / dist;
    let max = v.tuning.max_turn_rate * DT;
    let yaw = det::atan2(-dir.x, -dir.z).clamp(-max, max);
    let pitch = det::atan2(dir.y, -dir.z).clamp(-max, max);
    // Thrust toward the target on every axis (strafing corrects the line);
    // against the sideways drift too.
    let vloc = ship.orientation.conjugate().rotate(ship.vel);
    let axis = |want: f32, v: f32| (want * 3.0 - v * 0.15).clamp(-1.0, 1.0);
    let thrust = Vec3::new(
        axis(dir.x, vloc.x),
        axis(dir.y, vloc.y),
        -style.throttle.clamp(0.0, 1.0),
    );
    let lined_up = dir.z < -0.95;
    let mut buttons = Buttons::HORIZON_ASSIST;
    if style.boost && lined_up && ship.boost > 10.0 {
        buttons = buttons | Buttons::BOOST;
    }
    if style.shoot {
        let fwd = ship.orientation.rotate(Vec3::new(0.0, 0.0, -1.0));
        let in_front = v.others.iter().any(|p| {
            let d = *p - ship.pos;
            let l = d.length();
            l < 40.0 && l > 0.0 && d.dot(fwd) / l > 0.97
        });
        if in_front {
            buttons = buttons | Buttons::FIRE_PRIMARY;
        }
    }
    InputCmd {
        tick,
        thrust,
        look_pitch: pitch,
        look_yaw: yaw,
        roll: 0.0,
        buttons,
        weapon_select: 0,
    }
}
