//! Racing lines for Belt Up (M4 package C5; `docs/milestone-4-plan.md`
//! §3.8 "Bots").
//!
//! The planner lays a lattice over the course of start slot 0: stations
//! every [`STATION_DEG`] degrees of azimuth from the slot (3 degrees past
//! gate 0) once around the ring to the slot again, and at each station a
//! grid of points across radius ([`R_MIN`]..[`R_MAX`]) and height
//! ([`H_MAX`] above and below the belt plane). A point is a node if it keeps
//! [`NODE_CLEARANCE`] from rock and lies in the level; at a gate's station
//! only points inside the gate count. Neighbouring stations are linked to
//! the points at most two steps away across radius and height when the
//! segment keeps [`EDGE_CLEARANCE`] (1-Lipschitz bound first, samples only
//! where it is tight).
//!
//! A line is the cheapest path through the lattice by dynamic programming,
//! for a cost per edge of
//!
//! `time + rad_weight x dose`
//!
//! where `time` is the length over a speed that falls in narrow passages
//! ([`speed_factor`]), and `dose` the hull the radiation takes on the way
//! (the dose rate at the edge's middle times the time). The last station
//! adds the dive: the straight segment from the node to the exit, which
//! must be clear. `rad_weight` (seconds per hull point) is the line's
//! character: 0 is the open line (shortest time, any dose), a large weight
//! keeps inside the belt, values between give mixed lines (the belt, with
//! dashes over the densest stretches). Every line also knows, per point,
//! the time and the dose left to the finish (the bot's hull budget).
//!
//! Lines are built for slot 0 and rotated by whole wedges for the other
//! slots (the course is 8-fold symmetric; the voxel grid differs by less
//! than a cell, which the clearances absorb). The lattice of a world is
//! built once per process and shared ([`for_world`]); building it costs
//! about a second of CPU in release builds (most edges are in open space
//! and need no sample).
//!
//! Bots are not part of the deterministic simulation: plain `f32` maths.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use ls_game::ls_math::level::{BeltSite, BELT_WEDGES, BELT_WEDGE_ANGLE};
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Aabb, Vec3};
use ls_game::ls_sim::radiation::{dose_rate, RadiationSettings};

/// Azimuth step between stations (degrees).
pub const STATION_DEG: f32 = 1.5;
/// Stations per lap.
pub const STATIONS: usize = (360.0 / STATION_DEG) as usize;
/// Radius range of the lattice (u from the belt axis); the gates reach from
/// 87 to 153 u, so the line may cut inside the ring between gates.
pub const R_MIN: f32 = 76.0;
pub const R_MAX: f32 = 156.0;
pub const R_STEP: f32 = 4.0;
/// Height range of the lattice (u above and below the belt plane).
pub const H_MAX: f32 = 30.0;
pub const H_STEP: f32 = 3.0;
/// Free space a node keeps (u; the Tinker's radius is 0.5).
pub const NODE_CLEARANCE: f32 = 1.6;
/// Free space along a link (u).
pub const EDGE_CLEARANCE: f32 = 1.2;
/// Degrees from a gate to its start slot (the slots sit 3 degrees past
/// their gate, `docs/milestone-4-plan.md` §3.8).
const SLOT_DEG: f32 = 3.0;
/// Stations at the start and the end of a lap in which the open line may
/// still be in the belt (leaving the slot, joining the dive lane).
const OPEN_FREE_STATIONS: usize = 10;
/// Degrees before and after a gate in which the line keeps inside it.
const GATE_APPROACH_DEG: f32 = 6.0;
/// Steps across radius and height a link may make.
const REACH: i32 = 2;

/// Planned speed as a fraction of cruise speed for a passage with `clear`
/// u of free space: full speed from 4 u, about a third in a 1.2 u squeeze.
pub fn speed_factor(clear: f32) -> f32 {
    (0.3 + (clear - 1.2) * 0.25).clamp(0.3, 1.0)
}

/// The radiation a line plans with: the dose law and the race's scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radiation {
    pub settings: RadiationSettings,
    pub scale: f32,
}

impl Radiation {
    /// The built-in dose law at `scale`.
    pub fn builtin(scale: f32) -> Radiation {
        Radiation {
            settings: *RadiationSettings::builtin(),
            scale,
        }
    }

    /// Dose rate (hull/s) at `p`.
    pub fn rate(&self, belt: &BeltSite, p: Vec3) -> f32 {
        dose_rate(belt, &self.settings, p) * self.scale
    }
}

/// Extra nodes per station besides the grid: the generator's clear
/// corridor where it crosses the station (a corridor 4 u wide falls between
/// grid points) and, at the first and last station, the start slot (the
/// dive lane starts there).
const EXTRAS: usize = 2;
/// Links per node: the 25 grid neighbours around its cell and the next
/// station's extra nodes.
const LINKS: usize = GRID_LINKS + EXTRAS;
const GRID_LINKS: usize = ((2 * REACH + 1) * (2 * REACH + 1)) as usize;
/// Extra nodes link to and from points at most this far away (u).
const EXTRA_REACH: f32 = 12.0;

/// The lattice of one world (see the module docs).
#[derive(Debug, Clone)]
pub struct Lattice {
    belt: BeltSite,
    nr: usize,
    nh: usize,
    /// Nodes per station (grid plus extras).
    per: usize,
    /// Node positions, station-major.
    pos: Vec<Vec3>,
    /// Free space per node (`<= 0`: no node).
    clear: Vec<f32>,
    /// Distance outside the shelter (belt or cocoon) per node (u).
    shelter: Vec<f32>,
    /// The grid cell a node links from (its own, or the nearest for extras).
    anchor: Vec<(u8, u8)>,
    /// Smallest free space along each link in 0.1 u (`0`: no link), per
    /// node and link index (grid offset, then the extras).
    links: Vec<u8>,
    /// Last-station nodes with a clear dive to the exit.
    dive: Vec<bool>,
}

impl Lattice {
    /// Builds the lattice for `belt` over `terrain` (slot 0's course).
    /// `corridor` is the generator's clear corridor (closed polyline in
    /// prograde order, world coordinates; may be empty).
    pub fn build<T: TerrainQuery + ?Sized>(
        belt: &BeltSite,
        terrain: &T,
        corridor: &[Vec3],
    ) -> Lattice {
        let nr = ((R_MAX - R_MIN) / R_STEP) as usize + 1;
        let nh = (2.0 * H_MAX / H_STEP) as usize + 1;
        let grid = nr * nh;
        let per = grid + EXTRAS;
        let n = (STATIONS + 1) * per;
        let bounds = terrain.bounds();
        let inner = Aabb::new(bounds.min + Vec3::splat(2.0), bounds.max - Vec3::splat(2.0));
        let mut lat = Lattice {
            belt: belt.clone(),
            nr,
            nh,
            per,
            pos: vec![Vec3::ZERO; n],
            clear: vec![0.0; n],
            shelter: vec![0.0; n],
            anchor: vec![(0, 0); n],
            links: vec![0; n * LINKS],
            dive: vec![false; per],
        };
        let corridor_az: Vec<f32> = corridor.iter().map(|p| belt.azimuth(*p)).collect();
        for s in 0..=STATIONS {
            let a = station_azimuth(s);
            let gate = near_gate(s);
            let in_gate = |p: Vec3| {
                let r = belt.axis_distance(p);
                !gate || (r >= belt.gate_inner_radius + 2.0 && r <= belt.gate_outer_radius - 2.0)
            };
            for ir in 0..nr {
                for ih in 0..nh {
                    let i = s * per + ir * nh + ih;
                    let p = grid_point(belt, a, ir, ih);
                    lat.pos[i] = p;
                    lat.anchor[i] = (ir as u8, ih as u8);
                    if !inner.contains(p) || !in_gate(p) {
                        continue;
                    }
                    let d = terrain.sdf(p);
                    if d >= NODE_CLEARANCE {
                        lat.clear[i] = d;
                    }
                }
            }
            let extras = [
                corridor_at(belt, corridor, &corridor_az, a),
                (s == 0 || s == STATIONS).then_some(belt.slots[0].pos),
            ];
            for (e, p) in extras.into_iter().enumerate() {
                let i = s * per + grid + e;
                let Some(p) = p else { continue };
                lat.pos[i] = p;
                let r = belt.axis_distance(p);
                let h = p.y - belt.center.y;
                let ir = ((r - R_MIN) / R_STEP).round().clamp(0.0, (nr - 1) as f32);
                let ih = ((h + H_MAX) / H_STEP).round().clamp(0.0, (nh - 1) as f32);
                lat.anchor[i] = (ir as u8, ih as u8);
                if !inner.contains(p) || !in_gate(p) {
                    continue;
                }
                // Corridor points get a little slack: the corridor is 4 u
                // wide by construction.
                let d = terrain.sdf(p);
                if d >= EDGE_CLEARANCE {
                    lat.clear[i] = d;
                }
            }
        }
        for s in 0..STATIONS {
            for local in 0..per {
                let a = s * per + local;
                let ca = lat.clear[a];
                if ca <= 0.0 {
                    continue;
                }
                let pa = lat.pos[a];
                let (ir, ih) = lat.anchor[a];
                for k in 0..LINKS {
                    let Some(b) = lat.link_target(s, ir, ih, k) else {
                        continue;
                    };
                    let cb = lat.clear[b];
                    if cb <= 0.0 {
                        continue;
                    }
                    let pb = lat.pos[b];
                    let extra = local >= grid || k >= GRID_LINKS;
                    if extra && pa.distance(pb) > EXTRA_REACH {
                        continue;
                    }
                    let c = segment_clearance(terrain, pa, ca, pb, cb);
                    if c >= EDGE_CLEARANCE {
                        lat.links[a * LINKS + k] = (c * 10.0).clamp(1.0, 255.0) as u8;
                    }
                }
            }
        }
        for (i, p) in lat.pos.iter().enumerate() {
            lat.shelter[i] = belt.shelter_distance(*p);
        }
        // The dive: from the slot's station straight to the exit.
        for local in 0..per {
            let i = STATIONS * per + local;
            if lat.clear[i] > 0.0 {
                lat.dive[local] = dive_clear(terrain, lat.pos[i], belt.center);
            }
        }
        lat
    }

    pub fn belt(&self) -> &BeltSite {
        &self.belt
    }

    /// The node link `k` of a node at station `s` anchored at cell
    /// `(ir, ih)` leads to.
    fn link_target(&self, s: usize, ir: u8, ih: u8, k: usize) -> Option<usize> {
        let base = (s + 1) * self.per;
        if k >= GRID_LINKS {
            return Some(base + self.nr * self.nh + (k - GRID_LINKS));
        }
        let w = 2 * REACH + 1;
        let (dr, dh) = (k as i32 / w - REACH, k as i32 % w - REACH);
        let (jr, jh) = (ir as i32 + dr, ih as i32 + dh);
        if jr < 0 || jh < 0 || jr >= self.nr as i32 || jh >= self.nh as i32 {
            return None;
        }
        Some(base + jr as usize * self.nh + jh as usize)
    }

    /// Number of nodes (diagnostics).
    pub fn node_count(&self) -> usize {
        self.clear.iter().filter(|c| **c > 0.0).count()
    }

    /// The cheapest line for `rad_weight` (seconds per hull point) with
    /// `rad` and a ship cruising at `cruise` u/s, from the start slot.
    /// `None` if the lattice has no way around (a broken level).
    pub fn plan(&self, rad_weight: f32, rad: &Radiation, cruise: f32) -> Option<RacingLine> {
        self.plan_from(rad_weight, rad, cruise, 0, self.belt.slots[0].pos)
    }

    /// [`Lattice::plan`] from station `start` (slot 0's course) at the node
    /// of that station nearest `pos` (slot 0's frame).
    pub fn plan_from(
        &self,
        rad_weight: f32,
        rad: &Radiation,
        cruise: f32,
        start: usize,
        pos: Vec3,
    ) -> Option<RacingLine> {
        self.plan_with(rad_weight, f32::NEG_INFINITY, rad, cruise, start, pos)
    }

    /// The open line: the fastest line (radiation all but ignored) that keeps at
    /// least `min_shelter` u outside the belt once it has left the start
    /// and until the last stations before the dive (`OPEN_FREE_STATIONS`
    /// at either end), from station `start` at `pos` (slot 0's frame).
    pub fn plan_open(
        &self,
        min_shelter: f32,
        rad: &Radiation,
        cruise: f32,
        start: usize,
        pos: Vec3,
    ) -> Option<RacingLine> {
        // A tiny weight only breaks ties between equally fast lines.
        self.plan_with(0.01, min_shelter, rad, cruise, start, pos)
    }

    fn plan_with(
        &self,
        rad_weight: f32,
        min_shelter: f32,
        rad: &Radiation,
        cruise: f32,
        start: usize,
        pos: Vec3,
    ) -> Option<RacingLine> {
        let per = self.per;
        let n = (STATIONS + 1) * per;
        let start_station = start.min(STATIONS - 1);
        let mut cost = vec![f32::INFINITY; n];
        let mut from = vec![u32::MAX; n];
        let mut via = vec![0u8; n];
        let base = start_station * per;
        let start = (base..base + per)
            .filter(|i| self.clear[*i] > 0.0)
            .min_by(|a, b| {
                self.pos[*a]
                    .distance(pos)
                    .total_cmp(&self.pos[*b].distance(pos))
            })?;
        cost[start] = 0.0;
        let cruise = cruise.max(1.0);
        let w = 2 * REACH + 1;
        for s in start_station..STATIONS {
            for local in 0..per {
                let a = s * per + local;
                if !cost[a].is_finite() {
                    continue;
                }
                let pa = self.pos[a];
                let (ir, ih) = self.anchor[a];
                for k in 0..LINKS {
                    let c = self.links[a * LINKS + k];
                    if c == 0 {
                        continue;
                    }
                    let Some(b) = self.link_target(s, ir, ih, k) else {
                        continue;
                    };
                    let free = s + 1 < start_station + OPEN_FREE_STATIONS
                        || s + 1 > STATIONS - OPEN_FREE_STATIONS;
                    if !free && self.shelter[b] < min_shelter {
                        continue;
                    }
                    let c = f32::from(c) * 0.1;
                    let pb = self.pos[b];
                    let len = pa.distance(pb);
                    let t = len / (cruise * speed_factor(c));
                    let dose = rad.rate(&self.belt, (pa + pb) * 0.5) * t;
                    // A little smoothing: sideways steps cost a bit.
                    let side = if k < GRID_LINKS {
                        let (dr, dh) = (k as i32 / w - REACH, k as i32 % w - REACH);
                        (dr * dr + dh * dh) as f32
                    } else {
                        1.0
                    };
                    let total = cost[a] + t + rad_weight * dose + 0.004 * side;
                    if total < cost[b] {
                        cost[b] = total;
                        from[b] = a as u32;
                        via[b] = k as u8;
                    }
                }
            }
        }
        // The dive (boosting toward the exit).
        let dive_speed = cruise * 1.3;
        let mut best: Option<(f32, usize, f32, f32)> = None;
        for local in 0..per {
            let b = STATIONS * per + local;
            if !cost[b].is_finite() || !self.dive[local] {
                continue;
            }
            let (t, dose) = dive_cost(&self.belt, rad, self.pos[b], dive_speed);
            let total = cost[b] + t + rad_weight * dose;
            if best.is_none_or(|(c, ..)| total < c) {
                best = Some((total, b, t, dose));
            }
        }
        let (_, end, dive_t, dive_dose) = best?;
        // Walk back.
        let mut nodes = vec![end];
        let mut at = end;
        while from[at] != u32::MAX {
            at = from[at] as usize;
            nodes.push(at);
        }
        nodes.reverse();
        let mut line = RacingLine {
            points: Vec::with_capacity(nodes.len() + 1),
            speed: Vec::with_capacity(nodes.len() + 1),
            time_to_go: vec![0.0; nodes.len() + 1],
            dose_to_go: vec![0.0; nodes.len() + 1],
            rad_weight,
            first_station: start_station,
        };
        for (k, &node) in nodes.iter().enumerate() {
            line.points.push(self.pos[node]);
            let c = match nodes.get(k + 1) {
                Some(&next) => f32::from(self.links[node * LINKS + via[next] as usize]) * 0.1,
                None => self.clear[node],
            };
            line.speed.push(speed_factor(c));
        }
        line.points.push(self.belt.center);
        line.speed.push(1.0);
        // Time and dose left, from the end back.
        let last = line.points.len() - 1;
        line.time_to_go[last - 1] = dive_t;
        line.dose_to_go[last - 1] = dive_dose;
        for k in (0..last - 1).rev() {
            let (pa, pb) = (line.points[k], line.points[k + 1]);
            let t = pa.distance(pb) / (cruise * line.speed[k]);
            let dose = rad.rate(&self.belt, (pa + pb) * 0.5) * t;
            line.time_to_go[k] = line.time_to_go[k + 1] + t;
            line.dose_to_go[k] = line.dose_to_go[k + 1] + dose;
        }
        Some(line)
    }
}

/// Grid point `(ir, ih)` at azimuth `a`.
fn grid_point(belt: &BeltSite, a: f32, ir: usize, ih: usize) -> Vec3 {
    let r = R_MIN + ir as f32 * R_STEP;
    let h = -H_MAX + ih as f32 * H_STEP;
    let (sin, cos) = a.sin_cos();
    belt.center + Vec3::new(cos * r, h, -sin * r)
}

/// Where the corridor polyline crosses the half-plane at azimuth `a`
/// (interpolated between the two corridor points around it).
fn corridor_at(belt: &BeltSite, corridor: &[Vec3], az: &[f32], a: f32) -> Option<Vec3> {
    let n = corridor.len();
    if n < 2 {
        return None;
    }
    let a = a.rem_euclid(core::f32::consts::TAU);
    for i in 0..n {
        let j = (i + 1) % n;
        let (a0, mut a1) = (az[i], az[j]);
        if a1 < a0 {
            a1 += core::f32::consts::TAU;
        }
        let mut x = a;
        if x < a0 {
            x += core::f32::consts::TAU;
        }
        if a1 - a0 < 1.0 && x >= a0 && x <= a1 {
            let t = if a1 > a0 { (x - a0) / (a1 - a0) } else { 0.0 };
            let p = corridor[i] + (corridor[j] - corridor[i]) * t;
            // Back onto the station's half-plane (same radius and height).
            let r = belt.axis_distance(p);
            let (sin, cos) = a.sin_cos();
            return Some(Vec3::new(
                belt.center.x + cos * r,
                p.y,
                belt.center.z - sin * r,
            ));
        }
    }
    None
}

/// Azimuth of station `s` on slot 0's course (rad).
fn station_azimuth(s: usize) -> f32 {
    (SLOT_DEG + s as f32 * STATION_DEG).to_radians()
}

/// Whether station `s` lies on a gate (slot 0's course passes gates 1..7
/// and then gate 0 at 360 degrees).
#[cfg(test)]
fn gate_at_station(s: usize) -> bool {
    let deg = SLOT_DEG + s as f32 * STATION_DEG;
    let k = (deg / 45.0).round();
    k >= 1.0 && (deg - k * 45.0).abs() < 0.01
}

/// Whether station `s` lies within [`GATE_APPROACH_DEG`] of a gate: the
/// line keeps inside the gate's radii there, so a racer cutting toward a
/// point ahead still passes the gate inside.
fn near_gate(s: usize) -> bool {
    let deg = SLOT_DEG + s as f32 * STATION_DEG;
    let k = (deg / 45.0).round();
    k >= 1.0 && (deg - k * 45.0).abs() <= GATE_APPROACH_DEG + 0.01
}

/// Smallest free space along `a -> b` (ends with known free space `ca`,
/// `cb`): the 1-Lipschitz bound first, samples only where it is tight.
fn segment_clearance<T: TerrainQuery + ?Sized>(
    terrain: &T,
    a: Vec3,
    ca: f32,
    b: Vec3,
    cb: f32,
) -> f32 {
    let half = 0.5 * a.distance(b);
    let bound = ca.min(cb) - half;
    if bound >= 4.0 {
        return bound;
    }
    let mut c = ca.min(cb);
    for t in [0.25f32, 0.5, 0.75] {
        c = c.min(terrain.sdf(a + (b - a) * t));
        if c < EDGE_CLEARANCE {
            return c;
        }
    }
    c
}

/// Whether the straight dive from `p` to the exit at `center` keeps free
/// space (sampled every 2 u).
pub fn dive_clear<T: TerrainQuery + ?Sized>(terrain: &T, p: Vec3, center: Vec3) -> bool {
    let len = p.distance(center);
    let n = (len / 2.0).ceil().max(1.0) as usize;
    (1..n).all(|k| terrain.sdf(p + (center - p) * (k as f32 / n as f32)) >= 1.0)
}

/// Time (s) and dose (hull) of the straight dive from `p` to the exit.
pub fn dive_cost(belt: &BeltSite, rad: &Radiation, p: Vec3, speed: f32) -> (f32, f32) {
    let len = (p.distance(belt.center) - belt.exit_radius).max(0.0);
    let t = len / speed.max(1.0);
    let n = 16;
    let dose = (0..n)
        .map(|k| {
            let q = p + (belt.center - p) * ((k as f32 + 0.5) / n as f32);
            rad.rate(belt, q)
        })
        .sum::<f32>()
        / n as f32
        * t;
    (t, dose)
}

/// A planned racing line: points from the start slot once around and down
/// to the exit (the last point), the planned speed per point (fraction of
/// cruise speed on the way to the next point), and the time (s) and dose
/// (hull) left to the finish from each point.
#[derive(Debug, Clone, PartialEq)]
pub struct RacingLine {
    pub points: Vec<Vec3>,
    pub speed: Vec<f32>,
    pub time_to_go: Vec<f32>,
    pub dose_to_go: Vec<f32>,
    /// The weight it was planned with (seconds per hull point).
    pub rad_weight: f32,
    /// The station of the first point (0: the start slot).
    pub first_station: usize,
}

impl RacingLine {
    /// The same line for start slot `slot` (rotated by whole wedges).
    pub fn rotated(&self, belt: &BeltSite, slot: usize) -> RacingLine {
        let k = (slot % BELT_WEDGES) as i32;
        RacingLine {
            points: self.points.iter().map(|p| belt.rotate(*p, k)).collect(),
            ..self.clone()
        }
    }

    /// Planned time of the whole course (s).
    pub fn total_time(&self) -> f32 {
        self.time_to_go.first().copied().unwrap_or(0.0)
    }

    /// Planned dose of the whole course (hull).
    pub fn total_dose(&self) -> f32 {
        self.dose_to_go.first().copied().unwrap_or(0.0)
    }

    /// Share of the lap's points outside the belt volume (diagnostics: 0
    /// for a belt line, near 1 for the open line).
    pub fn open_share(&self, belt: &BeltSite) -> f32 {
        let n = self.points.len().saturating_sub(1).max(1);
        self.points[..n]
            .iter()
            .filter(|p| belt.belt_distance(**p) > 0.0)
            .count() as f32
            / n as f32
    }

    /// Index of the point a racer at `pos` that has passed `gates` gates of
    /// its lap (starting at `slot`) is at: by its azimuth past the slot,
    /// which grows by one station per point except around the start.
    pub fn index_at(&self, belt: &BeltSite, slot: usize, gates: u8, pos: Vec3) -> usize {
        let last = self.points.len().saturating_sub(2);
        if gates as usize >= BELT_WEDGES {
            return last;
        }
        station_of(belt, slot, gates, pos)
            .saturating_sub(self.first_station)
            .min(last)
    }
}

/// The station of slot `slot`'s course a racer at `pos` that has passed
/// `gates` gates is at: by its azimuth past the slot (one station per
/// [`STATION_DEG`]), except around the start and its own gate.
pub fn station_of(belt: &BeltSite, slot: usize, gates: u8, pos: Vec3) -> usize {
    if gates as usize >= BELT_WEDGES {
        return STATIONS;
    }
    let slot_az = slot as f32 * BELT_WEDGE_ANGLE + SLOT_DEG.to_radians();
    let mut rel = belt.azimuth(pos) - slot_az;
    rel = rel.rem_euclid(core::f32::consts::TAU);
    let deg = rel.to_degrees();
    // Just behind the slot before the first gate: the start.
    if gates == 0 && deg > 300.0 {
        return 0;
    }
    // Past its own gate without the lap counted (it crossed outside the
    // gate): back before the gate, to pass it inside.
    if gates as usize == BELT_WEDGES - 1 && deg < 30.0 {
        return STATIONS - 8;
    }
    ((deg / STATION_DEG).round() as usize).min(STATIONS)
}

/// Bisection steps of [`Lattice::plan_budget`] (on a log scale of the
/// weight between its lowest weight and [`MAX_BUDGET_WEIGHT`]).
const BUDGET_STEPS: usize = 6;
/// The safest weight [`Lattice::plan_budget`] goes to (seconds per hull
/// point: the belt line).
pub const MAX_BUDGET_WEIGHT: f32 = 4.0;

impl Lattice {
    /// The fastest line (from station `start` at `pos`, slot 0's frame)
    /// whose dose to the finish stays within `budget` hull: the lowest
    /// radiation weight from `min_weight` up that fits (bisected), else the
    /// safest line. The weight falls while the racer has hull to spare and
    /// rises when it is short (`docs/milestone-4-plan.md` §3.8 "Bots").
    pub fn plan_budget(
        &self,
        budget: f32,
        min_weight: f32,
        rad: &Radiation,
        cruise: f32,
        start: usize,
        pos: Vec3,
    ) -> Option<RacingLine> {
        let lo_line = self.plan_from(min_weight, rad, cruise, start, pos)?;
        if lo_line.total_dose() <= budget {
            return Some(lo_line);
        }
        let hi_line = self.plan_from(MAX_BUDGET_WEIGHT, rad, cruise, start, pos)?;
        if hi_line.total_dose() > budget {
            return Some(hi_line);
        }
        let (mut lo, mut hi) = (min_weight.max(0.01).ln(), MAX_BUDGET_WEIGHT.ln());
        let mut best = hi_line;
        for _ in 0..BUDGET_STEPS {
            let mid = 0.5 * (lo + hi);
            let line = self.plan_from(mid.exp(), rad, cruise, start, pos)?;
            if line.total_dose() <= budget {
                hi = mid;
                best = line;
            } else {
                lo = mid;
            }
        }
        Some(best)
    }
}

/// Lattices built in this process, by world share code.
type Cache = Mutex<HashMap<String, Arc<Lattice>>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The lattice of world `key` (its share code), built from `terrain` on
/// first use and shared by every bot of the process.
pub fn for_world<T: TerrainQuery + ?Sized>(
    key: &str,
    belt: &BeltSite,
    terrain: &T,
    corridor: &[Vec3],
) -> Arc<Lattice> {
    let mut c = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(l) = c.get(key) {
        return l.clone();
    }
    if c.len() >= 4 {
        c.clear();
    }
    let l = Arc::new(Lattice::build(belt, terrain, corridor));
    c.insert(key.to_string(), l.clone());
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::ls_math::level::BeltSlot;
    use ls_game::ls_math::Quat;

    /// A belt like the plan's around the origin.
    pub(crate) fn test_belt() -> BeltSite {
        let mut b = BeltSite {
            center: Vec3::ZERO,
            inner_radius: 95.0,
            outer_radius: 145.0,
            half_thickness: 12.0,
            cocoon_radius: 30.0,
            exit_radius: 6.0,
            gate_inner_radius: 87.0,
            gate_outer_radius: 153.0,
            slots: [BeltSlot {
                pos: Vec3::ZERO,
                orientation: Quat::IDENTITY,
            }; BELT_WEDGES],
            pickup_points: Vec::new(),
            boulders: Vec::new(),
        };
        for k in 0..BELT_WEDGES {
            let a = (k as f32 * 45.0 + 3.0).to_radians();
            let (s, c) = a.sin_cos();
            b.slots[k] = BeltSlot {
                pos: Vec3::new(c * 120.0, 0.0, -s * 120.0),
                orientation: Quat::from_axis_angle(Vec3::Y, a),
            };
        }
        b
    }

    /// Open space everywhere in a 400 x 120 x 400 box, plus (optionally) a
    /// slab of "asteroids" filling the belt except a corridor at radius 120
    /// in the plane.
    struct Toy {
        belt: BeltSite,
        rocky: bool,
    }

    impl TerrainQuery for Toy {
        fn sdf(&self, p: Vec3) -> f32 {
            if !self.rocky {
                return 50.0;
            }
            // Open space: outside the belt, a tube along the corridor and
            // the dive lanes (union of open sets: the largest openness).
            let b = &self.belt;
            let r = b.axis_distance(p);
            let tube = ((r - 120.0).powi(2) + p.y.powi(2)).sqrt();
            let lane = (0..BELT_WEDGES)
                .map(|k| {
                    let slot = b.slots[k].pos;
                    let t = (p.dot(slot) / slot.length_squared()).clamp(0.0, 1.0);
                    p.distance(slot * t)
                })
                .fold(f32::MAX, f32::min);
            b.belt_distance(p).max(3.0 - tube).max(3.0 - lane).min(50.0)
        }

        fn bounds(&self) -> Aabb {
            Aabb::new(
                Vec3::new(-200.0, -60.0, -200.0),
                Vec3::new(200.0, 60.0, 200.0),
            )
        }
    }

    #[test]
    fn gates_lie_on_stations() {
        let gates: Vec<usize> = (0..=STATIONS).filter(|s| gate_at_station(*s)).collect();
        assert_eq!(gates, vec![28, 58, 88, 118, 148, 178, 208, 238]);
        assert!(near_gate(24) && near_gate(32) && !near_gate(23) && !near_gate(33));
    }

    #[test]
    fn the_open_line_cuts_the_corners_and_the_belt_line_stays_inside() {
        let belt = test_belt();
        let open_space = Toy {
            belt: belt.clone(),
            rocky: false,
        };
        let lat = Lattice::build(&belt, &open_space, &[]);
        let rad = Radiation::builtin(1.0);
        let open = lat.plan(0.0, &rad, 12.0).unwrap();
        // Shortest way: close to the gates' inner ends (87 u), cutting
        // inside the ring between them.
        let n = open.points.len() - 1;
        let lap: f32 = open.points[..n]
            .windows(2)
            .map(|w| w[0].distance(w[1]))
            .sum();
        assert!(lap < 2.0 * std::f32::consts::PI * 100.0, "{lap}");
        assert!(open.open_share(&belt) > 0.8);
        let safe = lat.plan(20.0, &rad, 12.0).unwrap();
        assert!(safe.open_share(&belt) < 0.1, "{}", safe.open_share(&belt));
        assert!(safe.total_dose() < open.total_dose());
        assert!(safe.total_time() >= open.total_time());
        // Dose and time to go fall along the line.
        for w in safe.time_to_go.windows(2) {
            assert!(w[0] >= w[1]);
        }
        // The line passes every gate inside it.
        for k in 1..=8usize {
            let crossed = open
                .points
                .windows(2)
                .any(|w| belt.crosses_gate(k % 8, w[0], w[1]));
            assert!(crossed, "gate {k}");
        }
        // Rotated for slot 3: still crosses every gate.
        let r = open.rotated(&belt, 3);
        for k in 0..8usize {
            assert!(r
                .points
                .windows(2)
                .any(|w| belt.crosses_gate(k, w[0], w[1])));
        }
        assert_eq!(r.index_at(&belt, 3, 0, belt.slots[3].pos), 0);
    }

    #[test]
    fn a_rocky_belt_keeps_the_line_in_its_corridor() {
        let belt = test_belt();
        let rocky = Toy {
            belt: belt.clone(),
            rocky: true,
        };
        let corridor: Vec<Vec3> = (0..360)
            .map(|d| {
                let (s, c) = (d as f32).to_radians().sin_cos();
                Vec3::new(c * 120.0, 0.0, -s * 120.0)
            })
            .collect();
        let lat = Lattice::build(&belt, &rocky, &corridor);
        let line = lat.plan(50.0, &Radiation::builtin(1.0), 12.0).unwrap();
        for p in &line.points[..line.points.len() - 1] {
            assert!(rocky.sdf(*p) >= NODE_CLEARANCE, "{p:?}");
        }
        let lap_dose = line.total_dose() - line.dose_to_go[line.points.len() - 2];
        assert!(lap_dose < 3.0, "{lap_dose}");
    }

    #[test]
    fn speed_falls_in_squeezes() {
        assert_eq!(speed_factor(10.0), 1.0);
        assert!(speed_factor(1.2) < 0.35);
        assert!(speed_factor(2.0) < speed_factor(3.0));
    }
}
