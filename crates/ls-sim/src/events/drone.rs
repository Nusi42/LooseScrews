//! Shared pieces of the visitor events (M4 B2: Tax drone, Pizza delivery,
//! Customer survey, Mimic pickups, Gossip wall; `docs/milestone-4-plan.md`
//! §3.2):
//!
//! - [`NavGraph`]: the level's connection network (hub caverns and tunnel
//!   centre lines, from `ls_worldgen::Network`) as a graph the drones fly
//!   along. `ls_game::setup` builds it ([`NavGraph::from_network`]) and
//!   hands it to the world (`World::set_nav_graph`); levels without a
//!   network (asteroid belts, test rooms) have an empty graph and the
//!   drones fly straight at their goal.
//! - [`NavGraph::next_waypoint`]: where to fly next: the goal itself when
//!   the way is clear, else the farthest visible node of the shortest
//!   network path (Dijkstra over the nodes visible from both ends).
//! - [`fly`]: one tick of drone flight toward a waypoint with local
//!   avoidance (pushed off nearby rock and steered along a wall ahead,
//!   bounded acceleration, swept terrain collision).
//! - Helpers for event props: finding an event's entities by kind, the
//!   entry cavern, the rock face nearest a point, facing rotations.
//!
//! Everything here is deterministic (IEEE-exact operations and `sqrt`;
//! trigonometry through `ls_math::det`) and reads the world only through
//! the hook [`View`].

use std::collections::BTreeMap;

use ls_math::terrain::TerrainQuery;
use ls_math::{Quat, Vec3, DT};

use crate::behavior::{Ctx, View};
use crate::entity::{Entity, EntityKind, EntitySpawn, ReplicatedKind, NO_TEAM};
use crate::ship::PlayerId;
use crate::visual::VisualId;

/// The visitors' props ([`EntityKind::Prop`]): event drones, mimics, the
/// gossip face.
pub const DRONE: EntityKind = EntityKind::Prop(ReplicatedKind::Drone);
pub const MIMIC: EntityKind = EntityKind::Prop(ReplicatedKind::Mimic);
pub const GOSSIP_FACE: EntityKind = EntityKind::Prop(ReplicatedKind::GossipFace);

/// Drone roles (`state[0]` of a `ReplicatedKind::Drone` entity).
pub const ROLE_TAX: u8 = 0;
pub const ROLE_PIZZA: u8 = 1;
pub const ROLE_SURVEY: u8 = 2;

/// Drone phases (`state[2]`): waiting at the entry during the announce
/// phase, at work, leaving (the event's ending phase).
pub const DRONE_WAITING: u8 = 0;
pub const DRONE_WORKING: u8 = 1;
pub const DRONE_LEAVING: u8 = 2;

/// Outcome bytes of `CueKind::EventOutcome` shared by the visitor events.
pub const OUTCOME_SHOT_DOWN: u8 = 0xF0;

/// Free space kept around a drone by the avoidance (u, beyond its radius).
pub const AVOID_CLEARANCE: f32 = 1.5;
/// The look-ahead slides along a wall closer than this beyond the radius
/// (less than [`PLAN_SLACK`], so a planned line never makes it slide).
pub const AVOID_MARGIN: f32 = 0.1;
/// How far ahead the avoidance probes (u).
pub const LOOK_AHEAD: f32 = 3.0;
/// Free space a planned line keeps beyond the drone's radius (u).
pub const PLAN_SLACK: f32 = 0.3;
/// Ticks between two path plans of a drone (0.25 s).
pub const REPLAN_TICKS: u64 = 15;
/// Nodes tried for the way onto and off the network.
const VISIBLE_TRIES: usize = 6;
/// Dangling tunnel ends this close to another tunnel are joined to it.
const LINK_DISTANCE: f32 = 8.0;
/// Smallest step of the segment test (u).
const MIN_STEP: f32 = 0.3;

/// The level's connection network as a graph for the drones (see the
/// module docs). Static level data; not hashed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NavGraph {
    nodes: Vec<Vec3>,
    /// Neighbours and edge lengths, per node, in insertion order.
    adj: Vec<Vec<(u32, f32)>>,
    /// The boundary caverns drones enter from.
    entries: Vec<Vec3>,
}

impl NavGraph {
    /// No network: drones fly straight at their goal.
    pub fn empty() -> NavGraph {
        NavGraph::default()
    }

    /// The graph of hub centres and tunnel centre lines (`paths`: polylines
    /// whose ends sit on hub centres or on another tunnel), with `entries`
    /// as the boundary caverns. Equal points become one node; a tunnel end
    /// that is not a hub and lies within 8 u of another tunnel's centre
    /// line is joined to it (vein tunnels start on the network).
    pub fn from_network(hubs: &[Vec3], paths: &[Vec<Vec3>], entries: &[Vec3]) -> NavGraph {
        let mut g = NavGraph {
            entries: entries.to_vec(),
            ..NavGraph::default()
        };
        let mut index: BTreeMap<[u32; 3], u32> = BTreeMap::new();
        let mut node = |g: &mut NavGraph, p: Vec3| -> u32 {
            *index.entry(p.to_bits()).or_insert_with(|| {
                g.nodes.push(p);
                g.adj.push(Vec::new());
                (g.nodes.len() - 1) as u32
            })
        };
        let hub_ids: Vec<u32> = hubs.iter().map(|&h| node(&mut g, h)).collect();
        let mut ends: Vec<u32> = Vec::new();
        for path in paths {
            let ids: Vec<u32> = path.iter().map(|&p| node(&mut g, p)).collect();
            for w in ids.windows(2) {
                g.add_edge(w[0], w[1]);
            }
            if let (Some(&a), Some(&b)) = (ids.first(), ids.last()) {
                ends.extend([a, b]);
            }
        }
        for end in ends {
            if hub_ids.contains(&end) || g.adj[end as usize].len() != 1 {
                continue;
            }
            g.link_dangling(end);
        }
        g
    }

    fn add_edge(&mut self, a: u32, b: u32) {
        if a == b || self.adj[a as usize].iter().any(|(n, _)| *n == b) {
            return;
        }
        let d = self.nodes[a as usize].distance(self.nodes[b as usize]);
        self.adj[a as usize].push((b, d));
        self.adj[b as usize].push((a, d));
    }

    fn remove_edge(&mut self, a: u32, b: u32) {
        self.adj[a as usize].retain(|(n, _)| *n != b);
        self.adj[b as usize].retain(|(n, _)| *n != a);
    }

    /// Joins a dangling tunnel end to the nearest point of another edge
    /// within [`LINK_DISTANCE`] (splitting that edge there).
    fn link_dangling(&mut self, end: u32) {
        let p = self.nodes[end as usize];
        let mut best: Option<(f32, u32, u32, Vec3)> = None;
        for a in 0..self.nodes.len() as u32 {
            for &(b, _) in &self.adj[a as usize] {
                if b < a || a == end || b == end {
                    continue;
                }
                let q = closest_on_segment(p, self.nodes[a as usize], self.nodes[b as usize]);
                let d = p.distance(q);
                if d <= LINK_DISTANCE && best.is_none_or(|(bd, ..)| d < bd) {
                    best = Some((d, a, b, q));
                }
            }
        }
        let Some((_, a, b, q)) = best else {
            return;
        };
        let (pa, pb) = (self.nodes[a as usize], self.nodes[b as usize]);
        let mid = if q.distance(pa) < 1e-3 {
            a
        } else if q.distance(pb) < 1e-3 {
            b
        } else {
            self.nodes.push(q);
            self.adj.push(Vec::new());
            let m = (self.nodes.len() - 1) as u32;
            self.remove_edge(a, b);
            self.add_edge(a, m);
            self.add_edge(m, b);
            m
        };
        self.add_edge(end, mid);
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Node positions.
    pub fn nodes(&self) -> &[Vec3] {
        &self.nodes
    }

    /// Number of (undirected) edges.
    pub fn edge_count(&self) -> usize {
        self.adj.iter().map(Vec::len).sum::<usize>() / 2
    }

    /// The boundary caverns drones enter from.
    pub fn entries(&self) -> &[Vec3] {
        &self.entries
    }

    /// Whether every node reaches every other one.
    pub fn is_connected(&self) -> bool {
        if self.nodes.is_empty() {
            return true;
        }
        let mut seen = vec![false; self.nodes.len()];
        let mut stack = vec![0u32];
        seen[0] = true;
        while let Some(u) = stack.pop() {
            for &(v, _) in &self.adj[u as usize] {
                if !seen[v as usize] {
                    seen[v as usize] = true;
                    stack.push(v);
                }
            }
        }
        seen.iter().all(|s| *s)
    }

    /// Ways onto the network from `p`: the [`VISIBLE_TRIES`] nearest of the
    /// nodes and the closest points of the edges (pickup points and spawns
    /// reach a tunnel's centre line, not always a node) that `p` sees with
    /// `margin` of free space, as `(node, cost, via)`: an edge point gives
    /// both its ends, the cost running through the point.
    fn attach<T: TerrainQuery + ?Sized>(
        &self,
        terrain: &T,
        p: Vec3,
        margin: f32,
    ) -> Vec<(u32, f32, Vec3)> {
        // (distance, order, point, a, b): nodes have a == b.
        let mut c: Vec<(f32, usize, Vec3, u32, u32)> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            c.push((n.distance(p), c.len(), *n, i as u32, i as u32));
        }
        for a in 0..self.nodes.len() as u32 {
            for &(b, _) in &self.adj[a as usize] {
                if b < a {
                    continue;
                }
                let q = closest_on_segment(p, self.nodes[a as usize], self.nodes[b as usize]);
                c.push((q.distance(p), c.len(), q, a, b));
            }
        }
        c.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        let mut out = Vec::new();
        for (d, _, q, a, b) in c.into_iter().take(VISIBLE_TRIES * 2) {
            if !segment_clear(terrain, p, q, margin) {
                continue;
            }
            for n in [a, b] {
                let cost = d + q.distance(self.nodes[n as usize]);
                out.push((n, cost, q));
            }
        }
        out
    }

    /// Dijkstra from several start nodes (with entry costs) to the cheapest
    /// of several goal nodes (with exit costs); ties go to lower indices.
    fn shortest(&self, starts: &[(u32, f32)], goals: &[(u32, f32)]) -> Option<Vec<u32>> {
        let n = self.nodes.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![u32::MAX; n];
        let mut done = vec![false; n];
        for &(s, c) in starts {
            if c < dist[s as usize] {
                dist[s as usize] = c;
            }
        }
        loop {
            let mut u = usize::MAX;
            for i in 0..n {
                if !done[i] && dist[i].is_finite() && (u == usize::MAX || dist[i] < dist[u]) {
                    u = i;
                }
            }
            if u == usize::MAX {
                break;
            }
            done[u] = true;
            for &(v, w) in &self.adj[u] {
                let nd = dist[u] + w;
                if nd < dist[v as usize] {
                    dist[v as usize] = nd;
                    prev[v as usize] = u as u32;
                }
            }
        }
        let mut best: Option<(f32, u32)> = None;
        for &(g, c) in goals {
            let total = dist[g as usize] + c;
            if total.is_finite() && best.is_none_or(|(b, _)| total < b) {
                best = Some((total, g));
            }
        }
        let (_, mut at) = best?;
        let mut path = vec![at];
        while prev[at as usize] != u32::MAX {
            at = prev[at as usize];
            path.push(at);
        }
        path.reverse();
        Some(path)
    }

    /// Where a drone at `from` flies next to reach `to` (see the module
    /// docs); `margin` is the free space it needs.
    pub fn next_waypoint<T: TerrainQuery + ?Sized>(
        &self,
        terrain: &T,
        from: Vec3,
        to: Vec3,
        margin: f32,
    ) -> Vec3 {
        if self.nodes.is_empty() || segment_clear(terrain, from, to, margin) {
            return to;
        }
        let starts = self.attach(terrain, from, margin);
        if starts.is_empty() {
            // Off the network out of sight: head for the nearest node.
            let near = self.nearest(from, 1);
            return near.first().map_or(to, |(i, _)| self.nodes[*i as usize]);
        }
        let mut goals = self.attach(terrain, to, margin);
        if goals.is_empty() {
            goals = self
                .nearest(to, 3)
                .into_iter()
                .map(|(i, d)| (i, d, self.nodes[i as usize]))
                .collect();
        }
        let cost = |list: &[(u32, f32, Vec3)]| -> Vec<(u32, f32)> {
            list.iter().map(|(n, c, _)| (*n, *c)).collect()
        };
        let Some(path) = self.shortest(&cost(&starts), &cost(&goals)) else {
            return to;
        };
        // The cheapest way onto the first node and off the last one.
        let via = |list: &[(u32, f32, Vec3)], node: u32| -> Option<Vec3> {
            list.iter()
                .filter(|(n, _, _)| *n == node)
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .map(|(_, _, v)| *v)
        };
        let mut line: Vec<Vec3> = Vec::with_capacity(path.len() + 3);
        line.extend(via(&starts, path[0]));
        line.extend(path.iter().map(|n| self.nodes[*n as usize]));
        line.extend(via(&goals, *path.last().expect("non-empty path")));
        line.push(to);
        // Skip points the drone already sits on, then pull the string: the
        // farthest of the next few points in sight.
        let mut i = 0;
        while i + 1 < line.len() && line[i].distance(from) < 1.5 {
            i += 1;
        }
        let mut wp = line[i];
        for &p in line.iter().skip(i + 1).take(3) {
            if !segment_clear(terrain, from, p, margin) {
                break;
            }
            wp = p;
        }
        wp
    }

    /// The `k` nodes nearest `p` with their distances.
    fn nearest(&self, p: Vec3, k: usize) -> Vec<(u32, f32)> {
        let mut by: Vec<(f32, u32)> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.distance(p), i as u32))
            .collect();
        by.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        by.into_iter().take(k).map(|(d, i)| (i, d)).collect()
    }
}

/// The point of segment `a`-`b` nearest `p`.
pub fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    if len2 <= 0.0 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

/// Whether a sphere of radius `margin` moves from `a` to `b` without
/// touching rock (sphere tracing on the terrain's distance bound, steps of
/// at least 0.3 u).
pub fn segment_clear<T: TerrainQuery + ?Sized>(terrain: &T, a: Vec3, b: Vec3, margin: f32) -> bool {
    let d = b - a;
    let len = d.length();
    if len < 1e-4 {
        return terrain.sdf(a) >= margin;
    }
    let dir = d / len;
    let mut t = 0.0f32;
    loop {
        let s = terrain.sdf(a + dir * t);
        if s < margin {
            return false;
        }
        if t >= len {
            return true;
        }
        t = (t + (s - margin).max(MIN_STEP)).min(len);
    }
}

/// How a drone flies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flight {
    /// Cruise speed (u/s).
    pub speed: f32,
    /// Largest change of velocity (u/s^2).
    pub accel: f32,
    /// Collision sphere (u).
    pub radius: f32,
}

/// One tick of flight from `pos` (velocity `vel`) toward `goal` with local
/// avoidance (see the module docs). Returns the new position and velocity.
pub fn fly<T: TerrainQuery + ?Sized>(
    terrain: &T,
    pos: Vec3,
    vel: Vec3,
    goal: Vec3,
    f: &Flight,
) -> (Vec3, Vec3) {
    let to = goal - pos;
    let dist = to.length();
    // Arrive: slow down over the last few units.
    let want = f.speed.min(dist * 4.0);
    let mut desired = to.normalize_or_zero() * want;
    // Lean off rock that is close (the planned line keeps the drone's
    // radius free, so this only acts on chases of a moving goal).
    let clear = f.radius + AVOID_CLEARANCE;
    let d0 = terrain.sdf(pos);
    if d0 < clear {
        let n = terrain.normal(pos);
        desired += n * (want * 0.5 * ((clear - d0) / clear).min(1.0));
    }
    if let Some(dir) = desired.try_normalize() {
        // A wall in the way: slide along it instead of into it; head-on,
        // go around it (a fixed side, so it never dithers).
        let probe = pos + dir * LOOK_AHEAD.min(dist);
        let d1 = terrain.sdf(probe);
        if d1 < f.radius + AVOID_MARGIN {
            let n = terrain.normal(probe);
            let into = desired.dot(n);
            if into < 0.0 {
                desired -= n * into;
            }
            if desired.length() < want * 0.25 {
                let side = n
                    .cross(Vec3::Y)
                    .try_normalize()
                    .unwrap_or(n.any_orthonormal());
                desired += side * want;
            }
        }
    }
    let desired = desired.clamp_length(f.speed);
    let mut v = vel + (desired - vel).clamp_length(f.accel * DT);
    let mut p = pos;
    crate::ship::move_sphere(terrain, &mut p, &mut v, f.radius, 0.0);
    (p, v)
}

/// A rotation whose forward (-z) points along `dir`, upright (no roll;
/// identity for a zero direction).
pub fn facing(dir: Vec3) -> Quat {
    let Some(d) = dir.try_normalize() else {
        return Quat::IDENTITY;
    };
    let yaw = ls_math::det::atan2(-d.x, -d.z);
    let pitch = ls_math::det::asin(d.y.clamp(-1.0, 1.0));
    Quat::from_axis_angle(Vec3::Y, yaw)
        .mul_quat(Quat::from_axis_angle(Vec3::X, pitch))
        .normalize()
}

/// The event's props of `kind` (comedy events run one at a time, so the
/// kind identifies them), in list order.
pub fn props<'v>(view: &View<'v>, kind: EntityKind) -> Vec<&'v Entity> {
    view.entities.iter().filter(|e| e.kind == kind).collect()
}

/// The event's first prop of `kind` (a copy to change and write back with
/// `Ctx::set_entity`).
pub fn first_prop(view: &View, kind: EntityKind) -> Option<Entity> {
    view.entities.iter().find(|e| e.kind == kind).cloned()
}

/// Removes every prop of `kind`.
pub fn remove_props(ctx: &mut Ctx, kind: EntityKind) {
    let ids: Vec<u32> = props(&ctx.view, kind).iter().map(|e| e.id).collect();
    for id in ids {
        ctx.push(crate::behavior::Command::RemoveEntity { id });
    }
}

/// A prop of `kind` (no lifetime; its event removes it).
pub fn prop(kind: EntityKind, pos: Vec3, radius: f32, hp: f32, state: [u8; 4]) -> EntitySpawn {
    EntitySpawn {
        kind,
        visual: VisualId(0),
        radius,
        hp,
        state,
        team: NO_TEAM,
        ..EntitySpawn::marker(pos)
    }
}

/// `hp` as a percentage of `max` (0-100) for the state bytes.
pub fn hp_percent(hp: f32, max: f32) -> u8 {
    if max <= 0.0 || hp <= 0.0 {
        return 0;
    }
    crate::entity::state_byte(hp * 100.0 / max).min(100)
}

/// Where a drone heading for `target` enters: the nearest boundary cavern
/// at least `min_distance` away (the farthest if none is), or, on levels
/// without a network, the open point nearest the level's boundary face
/// closest to the target.
pub fn entry_point(view: &View, target: Vec3, min_distance: f32, radius: f32) -> Vec3 {
    let entries = view.nav.entries();
    if !entries.is_empty() {
        let mut by: Vec<(f32, usize)> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.distance(target), i))
            .collect();
        by.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let pick = by
            .iter()
            .find(|(d, _)| *d >= min_distance)
            .or(by.last())
            .map_or(0, |(_, i)| *i);
        return entries[pick];
    }
    let b = view.terrain.bounds();
    let inset = 2.0 + radius;
    let lo = b.min + Vec3::splat(inset);
    let hi = b.max - Vec3::splat(inset);
    let t = target.clamp(lo, hi);
    // The six face points, nearest face first (ties in axis order).
    let mut faces = [
        (t.x - lo.x, Vec3::new(lo.x, t.y, t.z)),
        (hi.x - t.x, Vec3::new(hi.x, t.y, t.z)),
        (t.y - lo.y, Vec3::new(t.x, lo.y, t.z)),
        (hi.y - t.y, Vec3::new(t.x, hi.y, t.z)),
        (t.z - lo.z, Vec3::new(t.x, t.y, lo.z)),
        (hi.z - t.z, Vec3::new(t.x, t.y, hi.z)),
    ];
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    let start = faces[0].1;
    // March toward the target until there is room for the drone.
    let to = target - start;
    let len = to.length();
    let dir = to.normalize_or_zero();
    let mut s = 0.0;
    while s < len {
        let p = start + dir * s;
        if view.terrain.sdf(p) >= radius + 1.0 {
            return p;
        }
        s += 1.0;
    }
    target
}

/// The rock surface nearest `p` and its outward normal: from open space
/// along the falling distance, from inside rock out along the normal.
pub fn rock_face<T: TerrainQuery + ?Sized>(terrain: &T, p: Vec3) -> (Vec3, Vec3) {
    let mut q = p;
    if terrain.sdf(q) < 0.0 {
        for _ in 0..64 {
            let s = terrain.sdf(q);
            if s >= 0.0 {
                break;
            }
            q += terrain.normal(q) * (-s).max(0.25);
        }
    } else {
        for _ in 0..64 {
            let s = terrain.sdf(q);
            if s < 0.05 {
                break;
            }
            q -= terrain.normal(q) * s;
        }
    }
    let n = terrain.normal(q).try_normalize().unwrap_or(Vec3::Y);
    (q, n)
}

/// Where drone `d` flies to reach `goal`: the planned waypoint, replanned
/// every [`REPLAN_TICKS`] or when reached. The plan lives in the drone's
/// `vars`: `[x, y, z, ±next plan tick]`, negative while the way to the goal
/// was clear (then it flies straight at the moving goal).
pub fn plan(view: &View, d: &mut Entity, goal: Vec3, radius: f32) -> Vec3 {
    let now = view.now as f32;
    let wp = Vec3::new(d.vars[0], d.vars[1], d.vars[2]);
    let due = now >= d.vars[3].abs() || (d.vars[3] > 0.0 && wp.distance(d.pos) < 1.0);
    if due {
        let next = (view.now + REPLAN_TICKS) as f32;
        let wp = view
            .nav
            .next_waypoint(view.terrain, d.pos, goal, radius + PLAN_SLACK);
        d.vars = if wp == goal {
            [goal.x, goal.y, goal.z, -next]
        } else {
            [wp.x, wp.y, wp.z, next]
        };
    }
    if d.vars[3] < 0.0 {
        goal
    } else {
        Vec3::new(d.vars[0], d.vars[1], d.vars[2])
    }
}

/// Ends the running event's active phase now (its ending phase follows).
pub fn end_now(e: &mut crate::events::EventCtx) {
    if e.event.phase == crate::events::EventPhase::Active {
        e.event.phase_end_tick = e.now();
    }
}

/// `CueKind::EventOutcome` value for `kind` and `byte`.
pub fn outcome(kind: crate::events::EventKind, byte: u8) -> u16 {
    ((kind.index() as u16) << 8) | byte as u16
}

/// Living ship `id`, if any.
pub fn living<'v>(view: &View<'v>, id: Option<PlayerId>) -> Option<&'v crate::ShipState> {
    id.and_then(|id| view.ships.get(&id)).filter(|s| s.alive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::BoxRoomTerrain;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn graph_joins_tunnels_at_hubs_and_dangling_ends() {
        let hubs = [Vec3::new(-40.0, 0.0, 0.0), Vec3::new(40.0, 0.0, 0.0)];
        let paths = vec![
            vec![hubs[0], Vec3::new(0.0, 5.0, 0.0), hubs[1]],
            // A vein tunnel starting 2 u off the hub tunnel.
            vec![Vec3::new(-20.0, 4.5, 2.0), Vec3::new(-20.0, 4.5, 30.0)],
        ];
        let g = NavGraph::from_network(&hubs, &paths, &hubs);
        // 2 hubs + the bend + 2 vein points + the split point.
        assert_eq!(g.nodes().len(), 6);
        assert_eq!(g.edge_count(), 5);
        assert!(g.is_connected());
        assert_eq!(g.entries(), &hubs);
        let empty = NavGraph::empty();
        assert!(empty.is_empty() && empty.is_connected());
    }

    #[test]
    fn open_space_flies_straight_and_a_segment_test_sees_walls() {
        let room = BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0));
        let a = Vec3::new(-10.0, 0.0, 0.0);
        let b = Vec3::new(10.0, 0.0, 0.0);
        assert!(segment_clear(&room, a, b, 1.0));
        assert!(!segment_clear(&room, a, Vec3::new(40.0, 0.0, 0.0), 1.0));
        let g = NavGraph::from_network(&[a], &[], &[]);
        assert_eq!(g.next_waypoint(&room, a, b, 1.0), b);
        // Flight reaches the goal at cruise speed and stops there.
        let f = Flight {
            speed: 20.0,
            accel: 80.0,
            radius: 1.0,
        };
        let (mut p, mut v) = (a, Vec3::ZERO);
        let mut top = 0.0f32;
        for _ in 0..120 {
            (p, v) = fly(&room, p, v, b, &f);
            top = top.max(v.length());
        }
        assert!(p.distance(b) < 0.1, "{p:?}");
        assert!((top - 20.0).abs() < 1e-3, "{top}");
        // A goal outside the room: the drone stays inside.
        for _ in 0..240 {
            (p, v) = fly(&room, p, v, Vec3::new(80.0, 0.0, 0.0), &f);
        }
        assert!(room.sdf(p) >= 0.99, "{p:?}");
    }

    #[test]
    fn facing_points_forward_upright() {
        for dir in [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.3, 0.5, -0.8),
        ] {
            let q = facing(dir);
            assert!((q.forward() - dir.normalize()).length() < 1e-4, "{dir:?}");
            assert!(q.right().y.abs() < 1e-4, "no roll");
        }
        assert_eq!(facing(Vec3::ZERO), Quat::IDENTITY);
    }

    #[test]
    fn rock_face_finds_the_nearest_wall_from_both_sides() {
        let room = BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0));
        let (p, n) = rock_face(&room, Vec3::new(25.0, 0.0, 0.0));
        assert!((p.x - 30.0).abs() < 0.1, "{p:?}");
        assert!((n - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-3, "{n:?}");
        let (p, n) = rock_face(&room, Vec3::new(0.0, 17.0, 0.0));
        assert!((p.y - 15.0).abs() < 0.3, "{p:?}");
        assert!(n.y < -0.99, "{n:?}");
    }
}
