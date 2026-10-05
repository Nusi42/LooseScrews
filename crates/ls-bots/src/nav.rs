//! Global navigation: a coarse open-space grid built from the terrain SDF,
//! A* over it, and a route follower that hands the brain one waypoint at a
//! time (local wall avoidance in [`crate::steering`] still does the
//! flying).
//!
//! Why: cave levels (the default sliders, Enclosure 50) are networks of
//! chambers joined by passages barely wider than a ship: on every such world
//! checked, the two teams' halves connect only through spots where the
//! terrain SDF stays below 0.75 u (the Tinker's radius is 0.5). Local
//! steering with a 3 u wall clearance never finds or enters those passages,
//! so bots on those worlds never met (M1 issue A, `docs/milestone-2-plan.md`).
//!
//! # The grid
//!
//! Cells of [`NavConfig::cell`] u cover the level bounds. A cell is a node if
//! some point in it has at least [`NavConfig::clearance`] of free space: the
//! centre if it is open enough, else the best of eight sub-cell points,
//! refined along the SDF gradient (passages are often narrower than a cell
//! and miss every lattice point). That point is the node's position. Two
//! neighbouring nodes (26-neighbourhood) are linked if the segment between
//! them keeps [`NavConfig::edge_clearance`], or, at a narrow end, a two-leg
//! elbow does (passage mouths rarely line up with the open cell in front).
//! Segment checks bisect with the 1-Lipschitz bound of the SDF, so open space
//! costs a sample or two and only squeezes are sampled finely.
//!
//! The grid is built incrementally within a per-tick SDF evaluation budget
//! ([`NavConfig::build_budget`]; a whole 256 x 128 x 256 u level takes 2-3 s
//! of CPU in release builds) so a bot keeps its 60 Hz rhythm, and geometry
//! edits (craters now, added rock later) re-sample only the cells they touch
//! ([`NavGrid::on_edit`]).
//!
//! Bots on a share-code world don't build their own grid: the first bot of
//! a world builds it once on a worker thread from the pristine level and
//! every bot of the process copies it ([`crate::nav_cache`],
//! [`Navigator::shared`]), then re-samples the regions of the edits it has
//! seen. Edits and blocked links stay per bot.
//!
//! # Search and following
//!
//! [`NavGrid::find_path`] is a weighted A* (narrow links cost extra) with an
//! expansion cap; if the goal cannot be reached it returns the partial route
//! to the node closest to it. [`Navigator`] keeps a route, re-plans when the
//! goal moves, the grid changes or the route gets old, skips ahead to the
//! furthest waypoint in clear view, steps back to one in view when the ship
//! was pushed off the route, and reports how much free space there is so the
//! brain can slow down and tighten its wall clearance in passages. When the
//! distance left along the route stops shrinking, the link the ship could
//! not pass is removed from the grid and the route re-planned.
//!
//! Bots are not part of the deterministic simulation. Nothing here uses
//! randomness or wall-clock time; only the tick a bot adopts a shared grid
//! depends on when the worker thread finishes.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::fmt;
use std::time::Duration;

use ls_game::ls_math::terrain::{TerrainEdit, TerrainQuery};
use ls_game::ls_math::{Aabb, Pcg32, Vec3};

use crate::nav_cache::{NavStatus, SharedNav};

/// Tuning of the navigation grid and search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavConfig {
    /// Cell size (u).
    pub cell: f32,
    /// Free space (terrain SDF, u) a node position needs. Slightly above the
    /// Tinker's collision radius (0.5): the tightest passages are flyable.
    pub clearance: f32,
    /// Free space every point of a link must keep (u).
    pub edge_clearance: f32,
    /// Terrain SDF evaluations the incremental build may spend per tick.
    pub build_budget: u32,
    /// A* gives up (and returns the best partial route) after this many
    /// node expansions.
    pub max_expansions: usize,
    /// Weight of the distance heuristic (> 1 trades optimality for speed).
    pub heuristic_weight: f32,
}

impl Default for NavConfig {
    fn default() -> Self {
        NavConfig {
            cell: 2.0,
            clearance: 0.55,
            edge_clearance: 0.5,
            build_budget: 25_000,
            max_expansions: 200_000,
            heuristic_weight: 1.3,
        }
    }
}

/// Stored clearance is `sdf * CLEAR_SCALE` in a `u8` (0 = not a node).
const CLEAR_SCALE: f32 = 32.0;
/// Segment checks accept pieces this short (u) whose ends are open enough.
const SEGMENT_RESOLUTION: f32 = 0.15;
/// Links through space narrower than this (u) cost extra, up to twice the
/// distance, so routes prefer wide caves over squeezes.
const WIDE: f32 = 1.5;
/// Links with an end narrower than this (u) may bend at an elbow.
const NARROW: f32 = 1.2;
/// Expansion cap for a goal known to be in another connected component
/// (the search only looks for the node closest to it nearby).
const UNREACHABLE_EXPANSIONS: usize = 20_000;
/// Changed regions remembered for [`NavGrid::changed_near`].
const CHANGE_HISTORY: usize = 64;
/// Grid steps between component relabelings after the grid changed.
const LABEL_INTERVAL: u32 = 60;

const fn make_dirs() -> [[i32; 3]; 26] {
    let mut out = [[0; 3]; 26];
    let mut n = 0;
    let mut z = -1;
    while z <= 1 {
        let mut y = -1;
        while y <= 1 {
            let mut x = -1;
            while x <= 1 {
                if !(x == 0 && y == 0 && z == 0) {
                    out[n] = [x, y, z];
                    n += 1;
                }
                x += 1;
            }
            y += 1;
        }
        z += 1;
    }
    out
}

/// The 26 neighbour offsets in lexicographic (z, y, x) order: the opposite
/// of direction `k` is `25 - k`.
const DIRS: [[i32; 3]; 26] = make_dirs();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Build {
    /// Sampling cells, next cell index.
    Sampling(usize),
    /// Linking nodes, next cell index.
    Linking(usize),
    Ready,
}

/// Phase of an edit-region update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Sample,
    Link,
}

/// An edit region being updated, resumable cell by cell so an update never
/// blows the per-tick budget (a 35 u collapse re-samples ~20k cells).
#[derive(Debug, Clone, PartialEq)]
struct Active {
    region: Aabb,
    lo: [i32; 3],
    hi: [i32; 3],
    /// Next cell, as an index into the box `lo..=hi`.
    next: usize,
    phase: Phase,
    changed: bool,
}

impl Active {
    fn size(&self) -> [i32; 3] {
        [
            self.hi[0] - self.lo[0] + 1,
            self.hi[1] - self.lo[1] + 1,
            self.hi[2] - self.lo[2] + 1,
        ]
    }

    fn len(&self) -> usize {
        let s = self.size();
        (s[0] * s[1] * s[2]) as usize
    }

    fn cell(&self, k: usize) -> [i32; 3] {
        let s = self.size();
        let k = k as i32;
        [
            self.lo[0] + k % s[0],
            self.lo[1] + (k / s[0]) % s[1],
            self.lo[2] + k / (s[0] * s[1]),
        ]
    }

    fn contains(&self, c: [i32; 3]) -> bool {
        (0..3).all(|a| c[a] >= self.lo[a] && c[a] <= self.hi[a])
    }
}

/// Reusable A* state for [`NavGrid::find_path_with`] (one per bot: a few
/// bytes per grid cell, allocated on the first search; a clone starts empty).
#[derive(Debug, Default)]
pub struct PathScratch {
    nodes: SearchNodes,
    open: BinaryHeap<Reverse<(u32, usize)>>,
}

impl Clone for PathScratch {
    fn clone(&self) -> Self {
        PathScratch::default()
    }
}

impl PathScratch {
    /// Starts a search over `cells` cells: bumps the search number, which
    /// invalidates every entry of the previous search at once.
    fn begin(&mut self, cells: usize) -> &mut SearchNodes {
        let n = &mut self.nodes;
        if n.stamp.len() != cells || n.search >= CLOSED - 1 {
            n.stamp = vec![0; cells];
            n.g = vec![0.0; cells];
            n.parent = vec![0; cells];
            n.search = 0;
        }
        n.search += 1;
        n
    }
}

/// Per-cell search entries; an entry is valid when its stamp is the current
/// search number (closed: the stamp with the top bit set).
#[derive(Debug, Clone, Default)]
struct SearchNodes {
    search: u32,
    stamp: Vec<u32>,
    g: Vec<f32>,
    parent: Vec<u32>,
}

const CLOSED: u32 = 1 << 31;
const NO_PARENT: u32 = u32::MAX;

impl SearchNodes {
    fn seen(&self, i: usize) -> bool {
        self.stamp[i] & !CLOSED == self.search
    }
    fn closed(&self, i: usize) -> bool {
        self.stamp[i] == self.search | CLOSED
    }
    fn close(&mut self, i: usize) {
        self.stamp[i] = self.search | CLOSED;
    }
    /// Cost so far (infinite for a node this search has not reached).
    fn g(&self, i: usize) -> f32 {
        if self.seen(i) {
            self.g[i]
        } else {
            f32::INFINITY
        }
    }
    /// Records a (better) cost and parent of an open node (`usize::MAX`:
    /// the start).
    fn set(&mut self, i: usize, g: f32, parent: usize) {
        self.stamp[i] = self.search;
        self.g[i] = g;
        self.parent[i] = if parent == usize::MAX {
            NO_PARENT
        } else {
            parent as u32
        };
    }
    fn parent(&self, i: usize) -> Option<usize> {
        (self.seen(i) && self.parent[i] != NO_PARENT).then(|| self.parent[i] as usize)
    }
}

/// A route found by [`NavGrid::find_path`].
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    /// Node positions from the start to the goal (or the closest node).
    pub points: Vec<Vec3>,
    /// Free space (u) at each point.
    pub clearance: Vec<f32>,
    /// The grid node at each point (`None` for elbows and the exact goal).
    pub nodes: Vec<Option<usize>>,
    /// False if the goal was out of reach and the route ends at the node
    /// closest to it.
    pub complete: bool,
}

/// The open-space grid. See the module docs.
#[derive(Clone)]
pub struct NavGrid {
    cfg: NavConfig,
    origin: Vec3,
    dims: [i32; 3],
    /// Quantized clearance per cell (`0` = no node).
    clear: Vec<u8>,
    /// Node position per cell, as an offset from the cell centre in units
    /// of `cell / 254`.
    rep: Vec<[i8; 3]>,
    /// Link bits per cell, one per [`DIRS`] entry.
    edges: Vec<u32>,
    /// Linear index offset per [`DIRS`] entry.
    offsets: [isize; 26],
    build: Build,
    /// Regions whose cells must be re-sampled (geometry edits).
    pending: Vec<Aabb>,
    /// The region being updated.
    active: Option<Active>,
    version: u32,
    /// Recent changes: the version they produced and the region.
    changes: VecDeque<(u32, Aabb)>,
    /// Connected component per cell (valid for `labels_version`).
    labels: Vec<u32>,
    labels_version: Option<u32>,
    /// Steps until stale labels may be recomputed.
    label_cooldown: u32,
}

impl fmt::Debug for NavGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavGrid")
            .field("dims", &self.dims)
            .field("build", &self.build)
            .field("nodes", &self.node_count())
            .field("version", &self.version)
            .finish()
    }
}

impl NavGrid {
    /// An empty grid covering `bounds`; call [`NavGrid::step`] every tick
    /// (or [`NavGrid::build_all`]) to fill it.
    pub fn new(bounds: Aabb, cfg: NavConfig) -> NavGrid {
        let size = bounds.size();
        let n = |s: f32| ((s / cfg.cell).ceil() as i32).max(1);
        let dims = [n(size.x), n(size.y), n(size.z)];
        let count = (dims[0] * dims[1] * dims[2]) as usize;
        let mut offsets = [0isize; 26];
        for (o, d) in offsets.iter_mut().zip(DIRS) {
            *o = d[0] as isize
                + dims[0] as isize * (d[1] as isize + dims[1] as isize * d[2] as isize);
        }
        NavGrid {
            cfg,
            origin: bounds.min,
            dims,
            clear: vec![0; count],
            rep: vec![[0; 3]; count],
            edges: vec![0; count],
            offsets,
            build: Build::Sampling(0),
            pending: Vec::new(),
            active: None,
            version: 0,
            changes: VecDeque::new(),
            labels: Vec::new(),
            labels_version: None,
            label_cooldown: 0,
        }
    }

    pub fn config(&self) -> &NavConfig {
        &self.cfg
    }

    /// True once every cell has been sampled and linked.
    pub fn is_ready(&self) -> bool {
        self.build == Build::Ready
    }

    /// Build phase and progress (fraction of cells done) for diagnostics.
    pub fn build_progress(&self) -> (&'static str, f32) {
        let n = self.clear.len().max(1) as f32;
        match self.build {
            Build::Sampling(i) => ("sampling", i as f32 / n),
            Build::Linking(i) => ("linking", i as f32 / n),
            Build::Ready => ("ready", 1.0),
        }
    }

    /// Bumped whenever an edit changed the grid.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Number of nodes (open cells).
    pub fn node_count(&self) -> usize {
        self.clear.iter().filter(|c| **c != 0).count()
    }

    /// Whether the grid changed near `region` since `version` (conservatively
    /// true when the change history does not reach back that far).
    pub fn changed_near(&self, version: u32, region: &Aabb) -> bool {
        if version == self.version {
            return false;
        }
        let newer = |v: u32| (v.wrapping_sub(version) as i32) > 0;
        match self.changes.front() {
            // The history must reach back to the change right after `version`.
            Some((v, _)) if !newer(*v) || *v == version.wrapping_add(1) => {}
            _ => return true,
        }
        self.changes
            .iter()
            .any(|(v, b)| newer(*v) && b.intersects(region))
    }

    fn bump_version(&mut self, region: Aabb) {
        self.version = self.version.wrapping_add(1);
        if self.changes.len() >= CHANGE_HISTORY {
            self.changes.pop_front();
        }
        self.changes.push_back((self.version, region));
    }

    /// Whether nodes `a` and `b` are known to lie in different connected
    /// components (false while the labels are stale).
    pub fn separated(&self, a: usize, b: usize) -> bool {
        self.labels_version == Some(self.version) && self.labels[a] != self.labels[b]
    }

    /// Labels connected components with a flood fill (a few ms on a whole
    /// cave level; done after the build and at most every
    /// [`LABEL_INTERVAL`] steps after changes).
    fn label_components(&mut self) {
        let n = self.clear.len();
        self.labels.clear();
        self.labels.resize(n, u32::MAX);
        let mut stack = Vec::new();
        let mut next = 0u32;
        for s in 0..n {
            if !self.is_node(s) || self.labels[s] != u32::MAX {
                continue;
            }
            self.labels[s] = next;
            stack.push(s);
            while let Some(i) = stack.pop() {
                let e = self.edges[i];
                for k in 0..26 {
                    if e & (1 << k) == 0 {
                        continue;
                    }
                    let j = (i as isize + self.offsets[k]) as usize;
                    if self.labels[j] == u32::MAX {
                        self.labels[j] = next;
                        stack.push(j);
                    }
                }
            }
            next += 1;
        }
        self.labels_version = Some(self.version);
    }

    /// Advances the build (and pending edit updates) by one tick's budget.
    pub fn step<T: TerrainQuery + ?Sized>(&mut self, terrain: &T) {
        self.label_cooldown = self.label_cooldown.saturating_sub(1);
        let mut budget = self.cfg.build_budget as i64;
        while budget > 0 {
            match self.build {
                Build::Sampling(i) => {
                    if i >= self.clear.len() {
                        self.build = Build::Linking(0);
                        continue;
                    }
                    budget -= self.sample(terrain, i) as i64;
                    self.build = Build::Sampling(i + 1);
                }
                Build::Linking(i) => {
                    if i >= self.clear.len() {
                        self.build = Build::Ready;
                        continue;
                    }
                    budget -= 1 + self.link_forward(terrain, i) as i64;
                    self.build = Build::Linking(i + 1);
                }
                Build::Ready => {
                    if self.active.is_none() {
                        let Some(region) = self.pending.pop() else {
                            break;
                        };
                        self.active = Some(self.begin_region(region));
                    }
                    budget -= self.advance_region(terrain) as i64;
                }
            }
        }
        let idle = self.build == Build::Ready && self.active.is_none() && self.pending.is_empty();
        if idle && self.labels_version != Some(self.version) && self.label_cooldown == 0 {
            self.label_components();
            self.label_cooldown = LABEL_INTERVAL;
        }
    }

    /// Builds the whole grid now (tests, tools).
    pub fn build_all<T: TerrainQuery + ?Sized>(&mut self, terrain: &T) {
        while !self.is_ready() || !self.pending.is_empty() || self.active.is_some() {
            self.step(terrain);
        }
        if self.labels_version != Some(self.version) {
            self.label_components();
        }
    }

    /// Queues the cells a geometry edit can have changed for re-sampling
    /// (paint is ignored). Cells not sampled yet pick the edit up anyway.
    pub fn on_edit(&mut self, edit: &TerrainEdit) {
        let reach = match edit {
            TerrainEdit::Paint { .. } | TerrainEdit::Surface { .. } => return,
            // Carving only changes the SDF inside the sphere.
            TerrainEdit::SubtractSphere { radius, .. } => *radius,
            // Added rock lowers the SDF up to the clamp distance around it.
            TerrainEdit::AddSphere { radius, .. } => *radius + 6.0,
        };
        let region = Aabb::from_center_half_extents(edit.center(), Vec3::splat(reach));
        if self.pending.len() >= 256 {
            // Too many updates queued (a barrage): merge into one box. The
            // update is resumable, so even a whole-level box only spreads
            // over more ticks (the old data stays in use meanwhile).
            let all = self.pending.drain(..).fold(region, |a, b| a.union(&b));
            self.pending.push(all);
        } else {
            self.pending.push(region);
        }
    }

    // ----- cells ---------------------------------------------------------------

    fn coords(&self, i: usize) -> [i32; 3] {
        let i = i as i32;
        let (nx, ny) = (self.dims[0], self.dims[1]);
        [i % nx, (i / nx) % ny, i / (nx * ny)]
    }

    fn index(&self, c: [i32; 3]) -> Option<usize> {
        let d = self.dims;
        (c[0] >= 0 && c[1] >= 0 && c[2] >= 0 && c[0] < d[0] && c[1] < d[1] && c[2] < d[2])
            .then(|| (c[0] + d[0] * (c[1] + d[1] * c[2])) as usize)
    }

    fn cell_coords(&self, p: Vec3) -> [i32; 3] {
        let q = (p - self.origin) / self.cfg.cell;
        [q.x.floor() as i32, q.y.floor() as i32, q.z.floor() as i32]
    }

    fn centre(&self, c: [i32; 3]) -> Vec3 {
        self.origin
            + Vec3::new(
                (c[0] as f32 + 0.5) * self.cfg.cell,
                (c[1] as f32 + 0.5) * self.cfg.cell,
                (c[2] as f32 + 0.5) * self.cfg.cell,
            )
    }

    /// Whether cell `i` is a node.
    pub fn is_node(&self, i: usize) -> bool {
        self.clear.get(i).is_some_and(|c| *c != 0)
    }

    /// Position of node `i`.
    pub fn node_pos(&self, i: usize) -> Vec3 {
        self.node_pos_in(i, self.coords(i))
    }

    /// [`NavGrid::node_pos`] of node `i` at cell coordinates `c`.
    fn node_pos_in(&self, i: usize, c: [i32; 3]) -> Vec3 {
        let r = self.rep[i];
        let unit = self.cfg.cell / 254.0;
        self.centre(c) + Vec3::new(r[0] as f32, r[1] as f32, r[2] as f32) * unit
    }

    /// Free space (u) at node `i` (0 for a non-node).
    pub fn node_clearance(&self, i: usize) -> f32 {
        self.clear[i] as f32 / CLEAR_SCALE
    }

    #[cfg(test)]
    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let e = self.edges[i];
        (0..26)
            .filter(move |k| e & (1 << k) != 0)
            .map(move |k| (i as isize + self.offsets[k]) as usize)
    }

    /// Samples cell `i`; returns the SDF evaluations spent.
    fn sample<T: TerrainQuery + ?Sized>(&mut self, terrain: &T, i: usize) -> u32 {
        let c = self.centre(self.coords(i));
        let (found, evals) = self.find_open(terrain, c);
        match found {
            Some((p, s)) => {
                let unit = self.cfg.cell / 254.0;
                let off = (p - c) / unit;
                let q = |v: f32| v.round().clamp(-127.0, 127.0) as i8;
                self.rep[i] = [q(off.x), q(off.y), q(off.z)];
                self.clear[i] = (s * CLEAR_SCALE).round().clamp(1.0, 255.0) as u8;
            }
            None => {
                self.rep[i] = [0; 3];
                self.clear[i] = 0;
            }
        }
        evals
    }

    /// The most open point of the cell centred at `c` if it has the node
    /// clearance, with its SDF, and the evaluations spent.
    fn find_open<T: TerrainQuery + ?Sized>(
        &self,
        terrain: &T,
        c: Vec3,
    ) -> (Option<(Vec3, f32)>, u32) {
        let need = self.cfg.clearance;
        let h = self.cfg.cell * 0.5;
        // Any point of the cell is within `h * sqrt(3)` of the centre.
        let half_diag = h * 1.733;
        let s = terrain.sdf(c);
        if s >= need {
            return (Some((c, s)), 1);
        }
        if s < need - half_diag {
            return (None, 1);
        }
        let mut evals = 1;
        let q = h * 0.5;
        let mut best = (c, s);
        // The eight sub-cell centres; their differences also give the SDF
        // gradient over the cell for free.
        let mut grad = Vec3::ZERO;
        for k in 0..8 {
            let sign = Vec3::new(
                if k & 1 == 0 { -1.0 } else { 1.0 },
                if k & 2 == 0 { -1.0 } else { 1.0 },
                if k & 4 == 0 { -1.0 } else { 1.0 },
            );
            let p = c + sign * q;
            let d = terrain.sdf(p);
            evals += 1;
            grad += sign * d;
            if d > best.1 {
                best = (p, d);
            }
        }
        if best.1 >= need {
            return (Some(best), evals);
        }
        if best.1 < need - half_diag * 0.5 {
            return (None, evals);
        }
        // Climb towards the passage axis, staying inside the cell: a short
        // line search along the cell gradient, then SDF normals.
        let lo = c - Vec3::splat(h * 0.99);
        let hi = c + Vec3::splat(h * 0.99);
        let (mut p, mut d) = best;
        if let Some(dir) = grad.try_normalize() {
            for t in [0.25, 0.5, 0.8] {
                let x = (best.0 + dir * (t * h)).clamp(lo, hi);
                let dx = terrain.sdf(x);
                evals += 1;
                if dx > d {
                    (p, d) = (x, dx);
                }
            }
            if d >= need {
                return (Some((p, d)), evals);
            }
        }
        if d < need - 0.3 {
            return (None, evals);
        }
        for _ in 0..2 {
            let n = terrain.normal(p);
            let x = (p + n * (need - d + 0.1)).clamp(lo, hi);
            let dx = terrain.sdf(x);
            evals += 7;
            if dx <= d {
                break;
            }
            (p, d) = (x, dx);
            if d >= need {
                return (Some((p, d)), evals);
            }
        }
        (None, evals)
    }

    /// How to get from node `a` to node `b`: `None` if blocked,
    /// `Some(None)` in a straight line, `Some(Some(m))` via the elbow `m`;
    /// plus the evaluations spent.
    ///
    /// Elbows matter at passage mouths: a passage narrower than a cell
    /// rarely lines up with the centre of the open cell in front of it, and
    /// the straight line clips the rim. An elbow takes some coordinates from
    /// `a` and the rest from `b` (first slide across, then fly in). Only
    /// tried when one end is narrow, which keeps the build cheap.
    fn link_way<T: TerrainQuery + ?Sized>(
        &self,
        terrain: &T,
        a: usize,
        b: usize,
    ) -> (Option<Option<Vec3>>, u32) {
        let (pa, pb) = (self.node_pos(a), self.node_pos(b));
        let (sa, sb) = (self.node_clearance(a), self.node_clearance(b));
        let lo = sa.min(sb);
        let need = self.cfg.edge_clearance;
        let (free, mut evals) = segment_clear_known(terrain, pa, sa, pb, sb, need);
        if free {
            return (Some(None), evals);
        }
        if lo >= NARROW {
            return (None, evals);
        }
        for mask in 1..7u32 {
            let pick = |bit: u32, x: f32, y: f32| if mask & bit != 0 { y } else { x };
            let m = Vec3::new(
                pick(1, pa.x, pb.x),
                pick(2, pa.y, pb.y),
                pick(4, pa.z, pb.z),
            );
            if m == pa || m == pb {
                continue;
            }
            let sm = terrain.sdf(m);
            evals += 1;
            let (f1, e1) = segment_clear_known(terrain, pa, sa, m, sm, need);
            evals += e1;
            if !f1 {
                continue;
            }
            let (f2, e2) = segment_clear_known(terrain, m, sm, pb, sb, need);
            evals += e2;
            if f2 {
                return (Some(Some(m)), evals);
            }
        }
        (None, evals)
    }

    /// Whether nodes `a` and `b` can be linked; evaluations spent.
    fn link_free<T: TerrainQuery + ?Sized>(&self, terrain: &T, a: usize, b: usize) -> (bool, u32) {
        let (way, evals) = self.link_way(terrain, a, b);
        (way.is_some(), evals)
    }

    fn set_link(&mut self, a: usize, k: usize, b: usize, free: bool) {
        if free {
            self.edges[a] |= 1 << k;
            self.edges[b] |= 1 << (25 - k);
        } else {
            self.edges[a] &= !(1 << k);
            self.edges[b] &= !(1 << (25 - k));
        }
    }

    /// Links node `i` with its neighbours in the 13 forward directions.
    fn link_forward<T: TerrainQuery + ?Sized>(&mut self, terrain: &T, i: usize) -> u32 {
        if !self.is_node(i) {
            return 0;
        }
        let c = self.coords(i);
        let mut evals = 0;
        for (k, d) in DIRS.iter().enumerate().skip(13) {
            let Some(j) = self.index([c[0] + d[0], c[1] + d[1], c[2] + d[2]]) else {
                continue;
            };
            if !self.is_node(j) {
                continue;
            }
            let (free, e) = self.link_free(terrain, i, j);
            evals += e;
            self.set_link(i, k, j, free);
        }
        evals
    }

    /// Starts updating the cells in `region`.
    fn begin_region(&self, region: Aabb) -> Active {
        let clamp = |c: [i32; 3], d: [i32; 3]| {
            [
                c[0].clamp(0, d[0] - 1),
                c[1].clamp(0, d[1] - 1),
                c[2].clamp(0, d[2] - 1),
            ]
        };
        Active {
            region,
            lo: clamp(self.cell_coords(region.min), self.dims),
            hi: clamp(self.cell_coords(region.max), self.dims),
            next: 0,
            phase: Phase::Sample,
            changed: false,
        }
    }

    /// Processes one cell of the active region: re-samples it (first pass)
    /// or re-links it (second pass: every pair with at least one end in the
    /// region, pairs with both ends in it once). Returns the evaluations
    /// spent, so a region of any size spreads over as many ticks as it needs.
    fn advance_region<T: TerrainQuery + ?Sized>(&mut self, terrain: &T) -> u32 {
        let Some(mut a) = self.active.take() else {
            return 0;
        };
        if a.next >= a.len() {
            match a.phase {
                Phase::Sample => {
                    a.phase = Phase::Link;
                    a.next = 0;
                    self.active = Some(a);
                }
                Phase::Link => {
                    if a.changed {
                        let grow = Vec3::splat(self.cfg.cell);
                        self.bump_version(Aabb::new(a.region.min - grow, a.region.max + grow));
                    }
                }
            }
            return 1;
        }
        let c = a.cell(a.next);
        a.next += 1;
        let i = self.index(c).expect("clamped");
        let mut evals = 1;
        match a.phase {
            Phase::Sample => {
                let before = (self.clear[i], self.rep[i]);
                evals += self.sample(terrain, i);
                if before != (self.clear[i], self.rep[i]) {
                    a.changed = true;
                    if !self.is_node(i) {
                        // Gone: drop its links now, the link pass is ticks away.
                        for (k, d) in DIRS.iter().enumerate() {
                            if let Some(j) = self.index([c[0] + d[0], c[1] + d[1], c[2] + d[2]]) {
                                self.set_link(i, k, j, false);
                            }
                        }
                    }
                }
            }
            Phase::Link => {
                let old = self.edges[i];
                for (k, d) in DIRS.iter().enumerate() {
                    let cj = [c[0] + d[0], c[1] + d[1], c[2] + d[2]];
                    let Some(j) = self.index(cj) else {
                        continue;
                    };
                    // Pairs inside the region are linked from their first end
                    // (the 13 forward directions).
                    if k < 13 && a.contains(cj) {
                        continue;
                    }
                    let free = if self.is_node(i) && self.is_node(j) {
                        let (f, e) = self.link_free(terrain, i, j);
                        evals += e;
                        f
                    } else {
                        false
                    };
                    self.set_link(i, k, j, free);
                }
                a.changed |= old != self.edges[i];
            }
        }
        self.active = Some(a);
        evals
    }

    /// Re-tests the link between neighbouring nodes `a` and `b` (a blocked
    /// link's time is up).
    pub fn relink<T: TerrainQuery + ?Sized>(&mut self, terrain: &T, a: usize, b: usize) {
        let (ca, cb) = (self.coords(a), self.coords(b));
        let d = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
        let Some(k) = DIRS.iter().position(|x| *x == d) else {
            return;
        };
        let old = self.edges[a];
        let free = self.is_node(a) && self.is_node(b) && self.link_free(terrain, a, b).0;
        self.set_link(a, k, b, free);
        if old != self.edges[a] {
            let (pa, pb) = (self.node_pos(a), self.node_pos(b));
            self.bump_version(Aabb::new(pa.min(pb), pa.max(pb)));
        }
    }

    // ----- queries -------------------------------------------------------------

    /// The node nearest to `p` within two cells; with `visible`, only nodes
    /// with a free straight line from `p` count (falling back to the
    /// nearest node if none has one).
    pub fn nearest_node<T: TerrainQuery + ?Sized>(
        &self,
        terrain: &T,
        p: Vec3,
        visible: bool,
    ) -> Option<usize> {
        let c = self.cell_coords(p);
        let mut found: Vec<(f32, usize)> = Vec::new();
        for r in 0..=2i32 {
            for z in -r..=r {
                for y in -r..=r {
                    for x in -r..=r {
                        if x.abs().max(y.abs()).max(z.abs()) != r {
                            continue;
                        }
                        if let Some(i) = self.index([c[0] + x, c[1] + y, c[2] + z]) {
                            if self.is_node(i) {
                                found.push((self.node_pos(i).distance_squared(p), i));
                            }
                        }
                    }
                }
            }
            if r >= 1 && !found.is_empty() {
                break;
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if visible {
            let clear = self.cfg.edge_clearance * 0.9;
            if let Some((_, i)) = found
                .iter()
                .take(8)
                .find(|(_, i)| sight_clear(terrain, p, self.node_pos(*i), clear))
            {
                return Some(*i);
            }
        }
        found.first().map(|(_, i)| *i)
    }

    /// A* from `from` to `to` over the grid. `None` if either end has no
    /// node nearby; a partial route (to the node closest to `to`) if the
    /// goal is out of reach or the expansion cap was hit. Allocates its
    /// search state; [`Navigator`] reuses one ([`NavGrid::find_path_with`]).
    pub fn find_path<T: TerrainQuery + ?Sized>(
        &self,
        terrain: &T,
        from: Vec3,
        to: Vec3,
    ) -> Option<Path> {
        self.find_path_with(terrain, from, to, &mut PathScratch::default())
    }

    /// [`NavGrid::find_path`] with reusable search state: per-cell arrays
    /// stamped with a search number, so a search touches only the nodes it
    /// expands (a hash map of the expanded nodes made long searches take
    /// 60-125 ms on cave levels, and every bot of a match re-plans towards
    /// the same objective at about the same time: the bots stalled together
    /// and the server reported input starvation, M3 soak).
    pub fn find_path_with<T: TerrainQuery + ?Sized>(
        &self,
        terrain: &T,
        from: Vec3,
        to: Vec3,
        scratch: &mut PathScratch,
    ) -> Option<Path> {
        let start = self.nearest_node(terrain, from, true)?;
        let goal = self.nearest_node(terrain, to, false)?;
        // A goal in another component cannot be reached: only look for the
        // node closest to it near the start instead of walking the whole
        // component (~100 ms on a cave level).
        let max_expansions = if self.separated(start, goal) {
            UNREACHABLE_EXPANSIONS.min(self.cfg.max_expansions)
        } else {
            self.cfg.max_expansions
        };
        let goal_pos = self.node_pos(goal);
        let w = self.cfg.heuristic_weight;
        let h = |i: usize| self.node_pos(i).distance(goal_pos);

        let mut open = std::mem::take(&mut scratch.open);
        open.clear();
        let info = scratch.begin(self.clear.len());
        info.set(start, 0.0, usize::MAX);
        open.push(Reverse(((h(start) * w).to_bits(), start)));
        let mut best = (h(start), start);
        let mut expansions = 0;
        let mut reached = false;
        while let Some(Reverse((_, cur))) = open.pop() {
            if info.closed(cur) {
                continue;
            }
            info.close(cur);
            let g = info.g(cur);
            if cur == goal {
                reached = true;
                break;
            }
            let hc = h(cur);
            if hc < best.0 {
                best = (hc, cur);
            }
            expansions += 1;
            if expansions > max_expansions {
                break;
            }
            // Neighbour positions from the current cell's coordinates (no
            // index division per neighbour); same values as `node_pos`.
            let cc = self.coords(cur);
            let pc = self.node_pos_in(cur, cc);
            let edges = self.edges[cur];
            for (k, d) in DIRS.iter().enumerate() {
                if edges & (1 << k) == 0 {
                    continue;
                }
                let nb = (cur as isize + self.offsets[k]) as usize;
                if info.closed(nb) {
                    continue;
                }
                let pn = self.node_pos_in(nb, [cc[0] + d[0], cc[1] + d[1], cc[2] + d[2]]);
                let narrow = (WIDE - self.node_clearance(nb)).clamp(0.0, 1.0);
                let ng = g + pc.distance(pn) * (1.0 + narrow);
                if ng >= info.g(nb) {
                    continue;
                }
                info.set(nb, ng, cur);
                open.push(Reverse(((ng + pn.distance(goal_pos) * w).to_bits(), nb)));
            }
        }
        scratch.open = open;
        let info = &scratch.nodes;
        let end = if reached { goal } else { best.1 };
        let mut nodes = vec![end];
        let mut cur = end;
        while let Some(parent) = info.parent(cur) {
            nodes.push(parent);
            cur = parent;
        }
        nodes.reverse();
        let mut points = vec![self.node_pos(nodes[0])];
        let mut clearance = vec![self.node_clearance(nodes[0])];
        let mut ids = vec![Some(nodes[0])];
        for w in nodes.windows(2) {
            if let (Some(Some(m)), _) = self.link_way(terrain, w[0], w[1]) {
                points.push(m);
                clearance.push(self.node_clearance(w[0]).min(self.node_clearance(w[1])));
                ids.push(None);
            }
            points.push(self.node_pos(w[1]));
            clearance.push(self.node_clearance(w[1]));
            ids.push(Some(w[1]));
        }
        if reached {
            // Finish at the exact goal if it is in view of the last node.
            let s = terrain.sdf(to);
            if s >= self.cfg.edge_clearance
                && segment_clear(terrain, goal_pos, to, self.cfg.edge_clearance).0
            {
                points.push(to);
                clearance.push(s);
                ids.push(None);
            }
        }
        Some(Path {
            points,
            clearance,
            nodes: ids,
            complete: reached,
        })
    }

    /// Removes the link between neighbouring nodes `a` and `b` (a ship got
    /// stuck on it: the geometry is tighter than the samples showed). A
    /// later edit nearby or [`NavGrid::relink`] re-tests it.
    pub fn block_link(&mut self, a: usize, b: usize) {
        let (ca, cb) = (self.coords(a), self.coords(b));
        let d = [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]];
        if let Some(k) = DIRS.iter().position(|x| *x == d) {
            self.set_link(a, k, b, false);
            let (pa, pb) = (self.node_pos(a), self.node_pos(b));
            self.bump_version(Aabb::new(pa.min(pb), pa.max(pb)));
        }
    }

    /// A random node position (a few tries), for wandering.
    pub fn random_node(&self, rng: &mut Pcg32) -> Option<Vec3> {
        (0..64).find_map(|_| {
            let i = rng.below(self.clear.len() as u32) as usize;
            (self.is_node(i) && self.edges[i] != 0).then(|| self.node_pos(i))
        })
    }
}

/// Whether every point of the segment `a`-`b` keeps `clearance` of free
/// space, and the SDF evaluations spent. See [`segment_clear_known`].
pub fn segment_clear<T: TerrainQuery + ?Sized>(
    terrain: &T,
    a: Vec3,
    b: Vec3,
    clearance: f32,
) -> (bool, u32) {
    let (ok, evals) = segment_clear_known(terrain, a, terrain.sdf(a), b, terrain.sdf(b), clearance);
    (ok, evals + 2)
}

/// [`segment_clear`] from a ship's position: the start counts as open (the
/// ship is there, maybe grazing a wall).
pub fn sight_clear<T: TerrainQuery + ?Sized>(
    terrain: &T,
    from: Vec3,
    to: Vec3,
    clearance: f32,
) -> bool {
    let s = terrain.sdf(from).max(clearance);
    segment_clear_known(terrain, from, s, to, terrain.sdf(to), clearance).0
}

/// Segment check with the SDF at both ends known (`sa`, `sb`): bisects
/// until the 1-Lipschitz bound proves a piece open (every point of a piece
/// of length `l` has at least `(sa + sb - l) / 2`) or the piece is shorter
/// than [`SEGMENT_RESOLUTION`]. Open space costs a sample or two; only
/// squeezes are sampled finely.
pub fn segment_clear_known<T: TerrainQuery + ?Sized>(
    terrain: &T,
    a: Vec3,
    sa: f32,
    b: Vec3,
    sb: f32,
    need: f32,
) -> (bool, u32) {
    if sa < need || sb < need {
        return (false, 0);
    }
    if 0.5 * (sa + sb - a.distance(b)) >= need {
        return (true, 0);
    }
    let mut evals = 0;
    let mut stack = [(Vec3::ZERO, 0.0f32, Vec3::ZERO, 0.0f32); 24];
    stack[0] = (a, sa, b, sb);
    let mut n = 1;
    while n > 0 {
        n -= 1;
        let (p, sp, q, sq) = stack[n];
        let len = p.distance(q);
        if 0.5 * (sp + sq - len) >= need || len <= SEGMENT_RESOLUTION {
            continue;
        }
        let m = (p + q) * 0.5;
        let sm = terrain.sdf(m);
        evals += 1;
        if sm < need {
            return (false, evals);
        }
        if n + 2 > stack.len() {
            // Absurdly long segment: give up rather than overflow.
            return (false, evals);
        }
        stack[n] = (m, sm, q, sq);
        stack[n + 1] = (p, sp, m, sm);
        n += 2;
    }
    (true, evals)
}

/// One tick of route following: where to fly next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavSteer {
    /// The waypoint to fly straight at (in clear view).
    pub waypoint: Vec3,
    /// Distance to it (u).
    pub dist: f32,
    /// Distance left along the route (u).
    pub remaining: f32,
    /// Free space around the ship and, when close, at the waypoint (u).
    pub clearance: f32,
    /// The waypoint is the route's end.
    pub last: bool,
    /// The route reaches the goal (false: it ends as close as possible).
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Route {
    path: Path,
    next: usize,
    goal: Vec3,
    planned_at: u64,
    version: u32,
    /// The route's points, grown by one cell (for [`NavGrid::changed_near`]).
    bounds: Aabb,
    /// Shortest distance left along the route so far.
    best_remaining: f32,
    progress_tick: u64,
    next_skip_check: u64,
}

/// Re-plan a route at least this often (ticks).
const REPLAN_TICKS: u64 = 4 * 60;
/// A route is re-planned for a moving goal at most this often (ticks): a
/// goal that moves every tick (the Heist core and its carrier) made every
/// bot of a match search again every tick, and a search can take tens of
/// milliseconds. Bots that fell behind run several ticks in one loop turn,
/// so they fell further behind (stalls of 0.3-0.7 s and input starvation
/// on the server in Heist, M3 soak).
const MIN_REPLAN_TICKS: u64 = 15;
/// Re-plan when the distance to the next waypoint has not shrunk for this
/// many ticks.
const STUCK_TICKS: u64 = 120;
/// A waypoint counts as passed within this distance (u) in open space...
const REACH: f32 = 1.0;
/// ...and within this one in passages narrower than [`NARROW`].
const REACH_NARROW: f32 = 0.4;
/// How many waypoints ahead the follower looks for a shortcut...
const SKIP_AHEAD: usize = 6;
/// ...and how many behind for a way back after being pushed off the route.
const LOOK_BACK: usize = 3;
/// Ticks between line-of-sight checks.
const SIGHT_TICKS: u64 = 4;
/// A link blocked after a stall is re-tested after this many ticks: stalls
/// have other causes too (a ship in the way, a push).
const BLOCK_TICKS: u64 = 30 * 60;

/// A shared grid build a [`Navigator`] waits for.
#[derive(Debug, Clone)]
struct Waiting {
    shared: SharedNav,
    bounds: Aabb,
    cfg: NavConfig,
    /// Edits seen while waiting (applied after adoption).
    edits: Vec<TerrainEdit>,
}

/// A [`NavGrid`] plus the route the bot is following.
#[derive(Debug, Clone)]
pub struct Navigator {
    /// The bot's own grid (a placeholder while `waiting`).
    grid: NavGrid,
    /// A shared build to adopt when it finishes (see [`crate::nav_cache`]).
    waiting: Option<Waiting>,
    route: Option<Route>,
    failures: u32,
    retry_at: u64,
    /// Tick of the last [`Navigator::steer`] call.
    last_steer: Option<u64>,
    /// Links blocked after stalls, with the tick they are re-tested.
    blocked: Vec<(usize, usize, u64)>,
    /// A* state reused by every search.
    scratch: PathScratch,
}

impl Navigator {
    pub fn new(bounds: Aabb, cfg: NavConfig) -> Navigator {
        Navigator {
            grid: NavGrid::new(bounds, cfg),
            waiting: None,
            route: None,
            failures: 0,
            retry_at: 0,
            last_steer: None,
            blocked: Vec::new(),
            scratch: PathScratch::default(),
        }
    }

    /// A navigator that adopts the shared grid `shared` (built from the
    /// pristine level covering `bounds`) instead of building its own. Edits
    /// that arrive before the build finishes are queued and re-sampled
    /// after adoption, so pass every edit the bot's terrain already holds
    /// to [`Navigator::on_edit`] too. If the shared build fails, the
    /// navigator builds its own grid incrementally as [`Navigator::new`].
    pub fn shared(bounds: Aabb, cfg: NavConfig, shared: SharedNav) -> Navigator {
        let mut nav = Navigator::new(Aabb::new(bounds.min, bounds.min), cfg);
        nav.waiting = Some(Waiting {
            shared,
            bounds,
            cfg,
            edits: Vec::new(),
        });
        nav
    }

    /// The grid (an empty placeholder while waiting for a shared build).
    pub fn grid(&self) -> &NavGrid {
        &self.grid
    }

    pub fn is_ready(&self) -> bool {
        self.waiting.is_none() && self.grid.is_ready()
    }

    /// Whether the navigator is still waiting for a shared grid build.
    pub fn is_waiting(&self) -> bool {
        self.waiting.is_some()
    }

    /// Blocks until the shared grid build is done (or `timeout` passed);
    /// the next [`Navigator::update`] adopts it. True if nothing is left to
    /// wait for. For in-process tests that step faster than real time (a
    /// real bot never blocks: it steers locally meanwhile).
    pub fn wait_shared(&self, timeout: Duration) -> bool {
        match &self.waiting {
            None => true,
            Some(w) => !matches!(w.shared.wait(timeout), NavStatus::Building),
        }
    }

    /// Advances the grid build and edit updates (call every tick). While
    /// waiting for a shared build, checks whether it has finished and
    /// adopts a copy of it.
    pub fn update<T: TerrainQuery + ?Sized>(&mut self, terrain: &T) {
        if let Some(w) = &self.waiting {
            let base = match w.shared.status() {
                NavStatus::Building => return,
                NavStatus::Ready(g) => (*g).clone(),
                // Unsampled cells pick the edits up anyway.
                NavStatus::Failed => NavGrid::new(w.bounds, w.cfg),
            };
            let w = self.waiting.take().expect("waiting");
            self.grid = base;
            for e in &w.edits {
                self.grid.on_edit(e);
            }
        }
        self.grid.step(terrain);
    }

    /// See [`NavGrid::on_edit`].
    pub fn on_edit(&mut self, edit: &TerrainEdit) {
        match &mut self.waiting {
            Some(w) => w.edits.push(*edit),
            None => self.grid.on_edit(edit),
        }
    }

    /// The current route's goal, whether it reaches it, and the index of
    /// the next waypoint out of the waypoint count (diagnostics).
    pub fn route_info(&self) -> Option<(Vec3, bool, usize, usize)> {
        self.route
            .as_ref()
            .map(|r| (r.goal, r.path.complete, r.next, r.path.points.len()))
    }

    /// Forgets the route (respawn, goal abandoned).
    pub fn clear_route(&mut self) {
        self.route = None;
    }

    /// Follows (planning or re-planning as needed) a route from `pos` to
    /// `goal`. `None` while the grid is being built or no route can be
    /// found (the caller then steers locally).
    pub fn steer<T: TerrainQuery + ?Sized>(
        &mut self,
        terrain: &T,
        pos: Vec3,
        goal: Vec3,
        now: u64,
    ) -> Option<NavSteer> {
        // Progress only counts while the route is followed every tick: after
        // a pause (combat orbit, holding still to scavenge) the stall clock
        // restarts instead of blaming the link ahead.
        if self.last_steer.is_some_and(|t| now > t + 1) {
            if let Some(r) = self.route.as_mut() {
                r.progress_tick = now;
                r.best_remaining = f32::MAX;
            }
        }
        self.last_steer = Some(now);
        if !self.is_ready() {
            return None;
        }
        self.restore_blocked(terrain, now);
        let version = self.grid.version();
        let mut replan = match &mut self.route {
            None => true,
            Some(r) => {
                let moved = r.goal.distance(goal) > (0.25 * pos.distance(goal)).max(6.0)
                    && now >= r.planned_at + MIN_REPLAN_TICKS;
                let mut changed = false;
                if r.version != version && now >= r.planned_at + 30 {
                    // Only changes near the route matter.
                    changed = self.grid.changed_near(r.version, &r.bounds);
                    if !changed {
                        r.version = version;
                    }
                }
                let old = now >= r.planned_at + REPLAN_TICKS;
                moved || changed || old
            }
        };
        if self
            .route
            .as_ref()
            .is_some_and(|r| now >= r.progress_tick + STUCK_TICKS)
        {
            self.block_stuck_link();
            replan = true;
        }
        if !replan {
            replan = !self.follow(terrain, pos, now);
        }
        if replan {
            if now < self.retry_at {
                self.route = None;
                return None;
            }
            match self
                .grid
                .find_path_with(terrain, pos, goal, &mut self.scratch)
            {
                Some(path) if !path.points.is_empty() => {
                    self.failures = 0;
                    let grow = Vec3::splat(self.grid.config().cell);
                    let first = path.points[0];
                    let (lo, hi) = path
                        .points
                        .iter()
                        .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
                    self.route = Some(Route {
                        path,
                        next: 0,
                        goal,
                        planned_at: now,
                        version: self.grid.version(),
                        bounds: Aabb::new(lo - grow, hi + grow),
                        best_remaining: f32::MAX,
                        progress_tick: now,
                        next_skip_check: now,
                    });
                    self.follow(terrain, pos, now);
                }
                _ => {
                    self.failures += 1;
                    self.retry_at = now + 60 * self.failures.min(5) as u64;
                    self.route = None;
                    return None;
                }
            }
        }
        let route = self.route.as_ref().expect("planned above");
        let pts = &route.path.points;
        let last = pts.len() - 1;
        let wp = pts[route.next];
        let dist = pos.distance(wp);
        let remaining = dist
            + pts[route.next..]
                .windows(2)
                .map(|w| w[0].distance(w[1]))
                .sum::<f32>();
        // Free space here, and at the waypoint once it is close (the line
        // to it is clear, so nothing narrower lies in between).
        let mut clearance = terrain.sdf(pos);
        if dist < 4.0 {
            clearance = clearance.min(route.path.clearance[route.next]);
        }
        Some(NavSteer {
            waypoint: wp,
            dist,
            remaining,
            clearance,
            last: route.next == last,
            complete: route.path.complete,
        })
    }

    /// Advances the route's next waypoint: passes reached ones, skips ahead
    /// to the furthest one in clear view, or steps back to one in view after
    /// the ship was pushed off the route. False if no waypoint nearby is in
    /// view (re-plan from here).
    fn follow<T: TerrainQuery + ?Sized>(&mut self, terrain: &T, pos: Vec3, now: u64) -> bool {
        // As strict as the links: anything less walks the ship into rock.
        // Links the ship still cannot pass stall the route and get blocked
        // (see `block_stuck_link`).
        let clear = self.grid.config().edge_clearance;
        let Some(route) = self.route.as_mut() else {
            return false;
        };
        let pts = &route.path.points;
        let widths = &route.path.clearance;
        let last = pts.len() - 1;
        let reach = |i: usize| {
            if widths[i] < NARROW {
                REACH_NARROW
            } else {
                REACH
            }
        };
        while route.next < last && pos.distance(pts[route.next]) < reach(route.next) {
            route.next += 1;
        }
        let mut ok = true;
        if now >= route.next_skip_check {
            route.next_skip_check = now + SIGHT_TICKS;
            let sees = |j: usize| sight_clear(terrain, pos, pts[j], clear);
            let far = (route.next + SKIP_AHEAD).min(last);
            let ahead = (route.next + 1..=far).rev().find(|j| sees(*j));
            if let Some(j) = ahead {
                route.next = j;
            } else if !sees(route.next) {
                let back = route.next.saturating_sub(LOOK_BACK);
                match (back..route.next).rev().find(|j| sees(*j)) {
                    Some(j) => route.next = j,
                    None => ok = false,
                }
            }
        }
        // Progress is the distance left along the route, which does not
        // reward flip-flopping between two waypoints.
        let remaining = pos.distance(pts[route.next])
            + pts[route.next..]
                .windows(2)
                .map(|w| w[0].distance(w[1]))
                .sum::<f32>();
        if remaining < route.best_remaining - 0.5 {
            route.best_remaining = remaining;
            route.progress_tick = now;
        }
        ok
    }

    /// The route stalled: drop the link into the waypoint the ship could not
    /// reach, so the next plan goes another way.
    fn block_stuck_link(&mut self) {
        let Some(route) = self.route.take() else {
            return;
        };
        // The link from the last node before the waypoint to the first
        // node from it on (the waypoint may be an elbow).
        let ids = &route.path.nodes;
        let to = ids[route.next..].iter().find_map(|i| *i);
        let from = ids[..route.next].iter().rev().find_map(|i| *i);
        if let (Some(from), Some(to)) = (from, to) {
            self.grid.block_link(from, to);
            let now = route.progress_tick + STUCK_TICKS;
            self.blocked.push((from, to, now + BLOCK_TICKS));
        }
    }

    /// Re-tests blocked links whose time is up.
    fn restore_blocked<T: TerrainQuery + ?Sized>(&mut self, terrain: &T, now: u64) {
        if self.blocked.iter().all(|b| b.2 > now) {
            return;
        }
        let (due, keep): (Vec<_>, Vec<_>) = self.blocked.drain(..).partition(|b| b.2 <= now);
        self.blocked = keep;
        for (a, b, _) in due {
            self.grid.relink(terrain, a, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steering::{arrive, avoid_walls, thrust_for_velocity, SteerConfig};
    use ls_game::ls_math::terrain::BoxRoomTerrain;
    use ls_game::ls_math::Quat;
    use ls_game::ls_sim::{simulate_movement, InputCmd, PlayerId, ShipState, ShipTuning, ShipType};

    /// A 60 x 30 x 60 room split at x = 0 by a 4 u thick wall with one
    /// round hole of `hole` u radius at (0, 8, 20), plus craters.
    struct WallRoom {
        hole: f32,
        craters: Vec<(Vec3, f32)>,
    }

    impl TerrainQuery for WallRoom {
        fn sdf(&self, p: Vec3) -> f32 {
            let room = -self.bounds().sdf(p);
            let slab = p.x.abs() - 2.0;
            let dy = p.y - 8.0;
            let dz = p.z - 20.0;
            let cyl = self.hole - (dy * dy + dz * dz).sqrt();
            let mut wall = slab.max(cyl);
            for (c, r) in &self.craters {
                wall = wall.max(r - p.distance(*c));
            }
            room.min(wall)
        }
        fn bounds(&self) -> Aabb {
            Aabb::new(Vec3::new(-30.0, -15.0, -30.0), Vec3::new(30.0, 15.0, 30.0))
        }
    }

    fn built(t: &WallRoom) -> NavGrid {
        let mut g = NavGrid::new(t.bounds(), NavConfig::default());
        g.build_all(t);
        g
    }

    #[test]
    fn dirs_are_symmetric() {
        for (k, d) in DIRS.iter().enumerate() {
            let o = DIRS[25 - k];
            assert_eq!([-d[0], -d[1], -d[2]], o);
        }
    }

    #[test]
    fn open_room_is_all_nodes_and_linked() {
        let t = BoxRoomTerrain::centered(Vec3::new(20.0, 10.0, 20.0));
        let mut g = NavGrid::new(t.bounds(), NavConfig::default());
        g.build_all(&t);
        // 10 x 5 x 10 cells, every one open (centres are 1 u from walls).
        assert_eq!(g.node_count(), 500);
        let p = g
            .find_path(&t, Vec3::new(-8.0, 0.0, -8.0), Vec3::new(8.0, 0.0, 8.0))
            .unwrap();
        assert!(p.complete);
        // A diagonal line: 8 cells plus the exact goal.
        assert!(p.points.len() <= 10, "{:?}", p.points);
        assert_eq!(*p.points.last().unwrap(), Vec3::new(8.0, 0.0, 8.0));
    }

    /// One reused search state gives every search the same route as a
    /// fresh one (stale entries of earlier searches never leak in).
    #[test]
    fn reused_search_state_matches_fresh_searches() {
        let t = WallRoom {
            hole: 0.8,
            craters: vec![],
        };
        let g = built(&t);
        let mut scratch = PathScratch::default();
        let ends = [
            (Vec3::new(-20.0, 0.0, -20.0), Vec3::new(20.0, 0.0, -20.0)),
            (Vec3::new(20.0, 5.0, 10.0), Vec3::new(-20.0, -5.0, 10.0)),
            (Vec3::new(-10.0, 0.0, 0.0), Vec3::new(-12.0, 3.0, 4.0)),
            (Vec3::new(-20.0, 0.0, -20.0), Vec3::new(20.0, 0.0, -20.0)),
        ];
        for (from, to) in ends {
            let fresh = g.find_path(&t, from, to).unwrap();
            let reused = g.find_path_with(&t, from, to, &mut scratch).unwrap();
            assert_eq!(fresh.points, reused.points);
            assert_eq!(fresh.nodes, reused.nodes);
            assert_eq!(fresh.complete, reused.complete);
        }
    }

    #[test]
    fn route_goes_through_a_narrow_hole() {
        // Hole radius 0.8: no cell centre has 0.55 u of clearance in it (the
        // wall is 4 u thick), only refined node positions do.
        let t = WallRoom {
            hole: 0.8,
            craters: vec![],
        };
        let g = built(&t);
        let from = Vec3::new(-20.0, 0.0, -20.0);
        let to = Vec3::new(20.0, 0.0, -20.0);
        let p = g.find_path(&t, from, to).unwrap();
        assert!(p.complete, "no route through the hole");
        let through = p
            .points
            .iter()
            .filter(|q| q.x.abs() < 2.0)
            .all(|q| q.distance(Vec3::new(q.x, 8.0, 20.0)) < 0.8);
        assert!(through, "route must use the hole: {:?}", p.points);
        for w in p.points.windows(2) {
            assert!(segment_clear(&t, w[0], w[1], 0.45).0, "{w:?}");
        }
    }

    #[test]
    fn unreachable_goal_gives_a_partial_route_and_craters_open_the_way() {
        let mut t = WallRoom {
            hole: 0.3, // too narrow for a ship
            craters: vec![],
        };
        let mut g = built(&t);
        let from = Vec3::new(-20.0, 0.0, 0.0);
        let to = Vec3::new(20.0, 0.0, 0.0);
        let p = g.find_path(&t, from, to).unwrap();
        assert!(!p.complete);
        let end = *p.points.last().unwrap();
        assert!(end.x < 0.0 && end.distance(to) < 25.0, "{end:?}");

        // A crater through the wall (radius 3 at the centre) opens a way.
        let crater = TerrainEdit::SubtractSphere {
            center: Vec3::new(0.0, 0.0, 0.0),
            radius: 3.0,
        };
        t.craters.push((crater.center(), crater.radius()));
        let v = g.version();
        g.on_edit(&crater);
        g.step(&t);
        assert_ne!(g.version(), v);
        let p = g.find_path(&t, from, to).unwrap();
        assert!(p.complete);
        assert!(p.points.len() < 25, "{:?}", p.points);
    }

    #[test]
    fn shared_grid_is_adopted_with_the_edits_seen_while_waiting() {
        use crate::nav_cache::{acquire, NavKey, NavStatus};
        use std::time::Duration;
        let cfg = NavConfig::default();
        // The cached grid comes from the pristine level (hole too narrow).
        let shared = acquire(NavKey::new("nav-test-adopt", &cfg), move || {
            built(&WallRoom {
                hole: 0.3,
                craters: vec![],
            })
        });
        // The bot's level already has a crater through the wall.
        let crater = TerrainEdit::SubtractSphere {
            center: Vec3::new(0.0, 0.0, 0.0),
            radius: 3.0,
        };
        let t = WallRoom {
            hole: 0.3,
            craters: vec![(crater.center(), crater.radius())],
        };
        let mut nav = Navigator::shared(t.bounds(), cfg, shared.clone());
        nav.on_edit(&crater);
        assert!(nav.is_waiting() && !nav.is_ready());
        let NavStatus::Ready(base) = shared.wait(Duration::from_secs(30)) else {
            panic!("shared build did not finish");
        };
        nav.update(&t);
        assert!(!nav.is_waiting());
        while !nav.is_ready() {
            nav.update(&t);
        }
        let from = Vec3::new(-20.0, 0.0, 0.0);
        let to = Vec3::new(20.0, 0.0, 0.0);
        assert!(nav.grid().find_path(&t, from, to).unwrap().complete);
        // The bot's copy changed, the shared grid did not.
        assert_ne!(nav.grid().version(), base.version());
        let pristine = &*base;
        assert!(!pristine.find_path(&t, from, to).unwrap().complete);
    }

    #[test]
    fn failed_shared_build_falls_back_to_a_local_build() {
        use crate::nav_cache::{acquire, NavKey};
        use std::time::Duration;
        let cfg = NavConfig::default();
        let shared = acquire(NavKey::new("nav-test-fail", &cfg), || {
            panic!("expected test panic")
        });
        let _ = shared.wait(Duration::from_secs(30));
        let t = WallRoom {
            hole: 2.0,
            craters: vec![],
        };
        let mut nav = Navigator::shared(t.bounds(), cfg, shared);
        for _ in 0..10_000 {
            if nav.is_ready() {
                break;
            }
            nav.update(&t);
        }
        assert!(nav.is_ready());
        assert_eq!(nav.grid().node_count(), built(&t).node_count());
    }

    #[test]
    fn a_pause_in_following_does_not_block_links() {
        let t = WallRoom {
            hole: 0.9,
            craters: vec![],
        };
        let mut nav = Navigator::new(t.bounds(), NavConfig::default());
        while !nav.is_ready() {
            nav.update(&t);
        }
        let from = Vec3::new(-20.0, 0.0, -20.0);
        let goal = Vec3::new(20.0, 0.0, -20.0);
        for tick in 0..10 {
            nav.steer(&t, from, goal, tick).expect("route");
        }
        let v = nav.grid().version();
        // The bot stopped following (a fight) for 200 ticks and resumes from
        // the same spot: no progress, but it was not stuck either.
        nav.steer(&t, from, goal, 210).expect("route");
        assert_eq!(nav.grid().version(), v, "a link was blocked after a pause");
        // Truly stuck while following: the link is blocked...
        let mut tick = 211;
        while nav.grid().version() == v {
            nav.steer(&t, from, goal, tick);
            tick += 1;
            assert!(tick < 211 + 2 * STUCK_TICKS, "never blocked");
        }
        assert_eq!(nav.blocked.len(), 1);
        // ...and comes back once its time is up.
        let (a, b, until) = nav.blocked[0];
        assert!(!nav.grid().neighbours(a).any(|n| n == b));
        nav.steer(&t, from, goal, until);
        assert!(nav.blocked.is_empty());
        assert!(nav.grid().neighbours(a).any(|n| n == b));
    }

    #[test]
    fn big_edits_update_within_the_budget() {
        let mut t = WallRoom {
            hole: 0.3,
            craters: vec![],
        };
        let cfg = NavConfig {
            build_budget: 2_000,
            ..NavConfig::default()
        };
        let mut g = NavGrid::new(t.bounds(), cfg);
        g.build_all(&t);
        let from = Vec3::new(-20.0, 0.0, 0.0);
        let to = Vec3::new(20.0, 0.0, 0.0);
        let (s, e) = (
            g.nearest_node(&t, from, false).unwrap(),
            g.nearest_node(&t, to, false).unwrap(),
        );
        assert!(g.separated(s, e), "the halves are separate components");
        // A huge crater: the update spreads over many steps.
        let crater = TerrainEdit::SubtractSphere {
            center: Vec3::ZERO,
            radius: 12.0,
        };
        t.craters.push((crater.center(), crater.radius()));
        let v = g.version();
        g.on_edit(&crater);
        let mut steps = 0;
        while g.active.is_some() || !g.pending.is_empty() {
            g.step(&t);
            steps += 1;
            assert!(steps < 10_000);
        }
        assert!(steps > 5, "{steps}");
        assert_ne!(g.version(), v);
        // A region far from the crater did not change; one through it did.
        let far = Aabb::new(Vec3::new(-29.0, -14.0, 25.0), Vec3::new(-25.0, -10.0, 29.0));
        assert!(!g.changed_near(v, &far));
        assert!(g.changed_near(v, &Aabb::from_center_half_extents(Vec3::ZERO, Vec3::ONE)));
        let p = g.find_path(&t, from, to).unwrap();
        assert!(p.complete);
        // Stale labels are not trusted; fresh ones join the halves.
        assert!(!g.separated(s, e));
        for _ in 0..=LABEL_INTERVAL {
            g.step(&t);
        }
        assert_eq!(g.labels_version, Some(g.version()));
        assert!(!g.separated(s, e));
    }

    #[test]
    fn build_is_incremental_within_the_budget() {
        let t = BoxRoomTerrain::centered(Vec3::new(40.0, 20.0, 40.0));
        let cfg = NavConfig {
            build_budget: 500,
            ..NavConfig::default()
        };
        let mut g = NavGrid::new(t.bounds(), cfg);
        let mut ticks = 0;
        while !g.is_ready() {
            g.step(&t);
            ticks += 1;
            assert!(ticks < 1000);
        }
        // 4000 cells at one evaluation each, then linking.
        assert!(ticks >= 8, "{ticks}");
    }

    #[test]
    fn follower_flies_through_the_hole() {
        let t = WallRoom {
            hole: 0.9,
            craters: vec![],
        };
        let mut nav = Navigator::new(t.bounds(), NavConfig::default());
        nav.update(&t);
        assert!(nav.steer(&t, Vec3::ZERO, Vec3::ZERO, 0).is_none() || nav.is_ready());
        while !nav.is_ready() {
            nav.update(&t);
        }
        let tuning = ShipTuning::tinker();
        let goal = Vec3::new(20.0, 0.0, -20.0);
        let mut ship = ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            &tuning,
            Vec3::new(-20.0, 0.0, -20.0),
            Quat::IDENTITY,
        );
        let base = SteerConfig::default();
        let mut reached = false;
        for tick in 0..60 * 60 {
            let s = nav.steer(&t, ship.pos, goal, tick).expect("route");
            let speed = tuning.cruise_speed * ((s.clearance - 0.4) / 2.0).clamp(0.3, 1.0);
            let desired = if s.last {
                arrive(ship.pos, s.waypoint, speed, 4.0)
            } else {
                (s.waypoint - ship.pos).normalize_or_zero() * speed
            };
            let cfg = SteerConfig {
                clearance: (s.clearance * 0.8).clamp(0.6, base.clearance),
                ..base
            };
            let st = avoid_walls(&t, ship.pos, ship.vel, desired, s.dist, &cfg);
            let vel = st.dir * (desired.length() * st.speed_scale);
            let cmd = InputCmd::idle(tick).with_thrust(thrust_for_velocity(
                ship.orientation,
                vel,
                tuning.cruise_speed,
            ));
            simulate_movement(&mut ship, &cmd, &tuning, &t);
            if ship.pos.distance(goal) < 2.0 {
                reached = true;
                break;
            }
        }
        assert!(reached, "ended at {:?}", ship.pos);
    }
}
