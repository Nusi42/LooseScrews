//! Connectivity of a pristine level: a flood fill over a coarse lattice of
//! clearance samples.
//!
//! [`ClearanceGrid::scan`] samples the generator SDF at lattice points
//! (cell centres of spacing `step`) and joins neighbouring points (6-way)
//! that both have at least `clearance` of free space. Two such points are
//! at most `step` apart, so the segment between them keeps at least
//! `clearance - step / 2` of free space (1-Lipschitz SDF): a component is a
//! volume a ship of that radius can fly through.
//!
//! The *main component* is the one holding the level's [`crate::Network`]
//! hubs. Tests use the grid to prove that the network joins both team ends
//! on every slider combination; match setup (`ls_game::setup`) links each
//! spawn and pickup to the network's centre lines instead, which is much
//! cheaper than scanning the whole level at this resolution.

use ls_math::{Aabb, Vec3};

use crate::generator::floor_i32;
use crate::{Generator, RegionSampler};

/// Lattice spacing of [`ClearanceGrid::level`] (u).
pub const CONNECT_STEP: f32 = 2.0;
/// Free space a lattice point needs to count as open in
/// [`ClearanceGrid::level`] (u): segments between open neighbours keep at
/// least 1 u (more than any ship's radius), and the connection tunnels are
/// open along a tube of `TUNNEL_RADIUS - CONNECT_CLEARANCE = 2` u around
/// their centre line, wide enough (> sqrt(3) u) to be 6-connected on the
/// lattice.
pub const CONNECT_CLEARANCE: f32 = 2.0;

const CLOSED: u32 = u32::MAX;
/// Lattice points per top-level scan block edge.
const BLOCK: usize = 16;

/// Lattice sampling state of [`ClearanceGrid::scan`].
struct Scan<'a, 's> {
    sampler: &'s RegionSampler<'a>,
    origin: Vec3,
    step: f32,
    dims: [usize; 3],
    clearance: f32,
    /// x coordinate of every lattice column.
    xs: &'s [f32],
}

impl Scan<'_, '_> {
    /// Fills `values` for lattice points `lo..hi` (per axis). If the SDF at
    /// the block centre is further from the clearance than the block's
    /// half diagonal (times 1.05 plus 0.25, the chunk store's margin for the
    /// noise gradient), every point is on the same side of the clearance
    /// and gets that bound; otherwise the block splits into octants, and
    /// blocks of at most 2 points per axis sample every point.
    fn fill_block(&self, values: &mut [f32], lo: [usize; 3], hi: [usize; 3]) {
        let ext = [0, 1, 2].map(|a| hi[a] - lo[a]);
        if ext.iter().any(|&e| e > 2) {
            let mid = [0, 1, 2].map(|a| (lo[a] + hi[a] - 1) as f32 * 0.5);
            let centre = self.origin + Vec3::new(mid[0], mid[1], mid[2]) * self.step;
            let half = Vec3::new(
                (ext[0] - 1) as f32,
                (ext[1] - 1) as f32,
                (ext[2] - 1) as f32,
            ) * (0.5 * self.step);
            let reach = half.length() * 1.05 + 0.25;
            let v = self.sampler.sample(centre);
            let fill = if v - reach >= self.clearance {
                Some(v - reach)
            } else if v + reach < self.clearance {
                Some(v + reach)
            } else {
                None
            };
            if let Some(f) = fill {
                for k in lo[2]..hi[2] {
                    for j in lo[1]..hi[1] {
                        let base = lo[0] + self.dims[0] * (j + self.dims[1] * k);
                        values[base..base + ext[0]].fill(f);
                    }
                }
                return;
            }
            // Split every axis longer than one point in two.
            let split = [0, 1, 2].map(|a| {
                if ext[a] > 1 {
                    let m = lo[a] + ext[a] / 2;
                    [(lo[a], m), (m, hi[a])]
                } else {
                    [(lo[a], hi[a]), (hi[a], hi[a])]
                }
            });
            for (zl, zh) in split[2] {
                for (yl, yh) in split[1] {
                    for (xl, xh) in split[0] {
                        if zl < zh && yl < yh && xl < xh {
                            self.fill_block(values, [xl, yl, zl], [xh, yh, zh]);
                        }
                    }
                }
            }
            return;
        }
        let mut row = [0.0f32; 2];
        for k in lo[2]..hi[2] {
            for j in lo[1]..hi[1] {
                let base = lo[0] + self.dims[0] * (j + self.dims[1] * k);
                let y = self.origin.y + j as f32 * self.step;
                let z = self.origin.z + k as f32 * self.step;
                let r = &mut row[..ext[0]];
                self.sampler.sample_row(y, z, &self.xs[lo[0]..hi[0]], r);
                values[base..base + ext[0]].copy_from_slice(r);
            }
        }
    }
}

/// Open lattice points of a level and their connected components.
#[derive(Debug, Clone)]
pub struct ClearanceGrid {
    /// Position of lattice point `(0, 0, 0)`.
    origin: Vec3,
    step: f32,
    dims: [usize; 3],
    clearance: f32,
    /// SDF per lattice point (x fastest, then y, then z): sampled, or a
    /// bound on the same side of the clearance (a lower bound for open
    /// points) where a block sample decided a whole block.
    values: Vec<f32>,
    /// Component per lattice point (`CLOSED` below the clearance).
    labels: Vec<u32>,
    /// Lattice points per component.
    sizes: Vec<u32>,
}

impl ClearanceGrid {
    /// The whole open room of `gen` at [`CONNECT_STEP`] and
    /// [`CONNECT_CLEARANCE`].
    pub fn level(gen: &Generator) -> Self {
        Self::scan(gen, gen.room(), CONNECT_STEP, CONNECT_CLEARANCE)
    }

    /// Samples the lattice of spacing `step` over `area` and labels the
    /// components of points with at least `clearance` of free space.
    pub fn scan(gen: &Generator, area: Aabb, step: f32, clearance: f32) -> Self {
        let size = area.size();
        let count = |s: f32| ((s / step) as usize).max(1);
        let dims = [count(size.x), count(size.y), count(size.z)];
        let origin = area.min + Vec3::splat(0.5 * step);
        let xs: Vec<f32> = (0..dims[0]).map(|i| origin.x + i as f32 * step).collect();
        let sampler = gen.sparse_region_sampler(area);
        let n = dims[0] * dims[1] * dims[2];
        let mut values = vec![0.0f32; n];
        // Octree over blocks of up to BLOCK^3 lattice points: one sample at
        // a block's centre decides the whole block when it is far enough
        // from the clearance (see `fill_block`).
        let blocks = dims.map(|d| d.div_ceil(BLOCK));
        let scan = Scan {
            sampler: &sampler,
            origin,
            step,
            dims,
            clearance,
            xs: &xs,
        };
        for bz in 0..blocks[2] {
            for by in 0..blocks[1] {
                for bx in 0..blocks[0] {
                    let lo = [bx, by, bz].map(|b| b * BLOCK);
                    let hi = [0, 1, 2].map(|a| (lo[a] + BLOCK).min(dims[a]));
                    scan.fill_block(&mut values, lo, hi);
                }
            }
        }
        let mut labels = vec![CLOSED; n];
        let mut sizes = Vec::new();
        let mut stack = Vec::new();
        let (sx, sy) = (1usize, dims[0]);
        let sz = dims[0] * dims[1];
        for start in 0..n {
            if labels[start] != CLOSED || values[start] < clearance {
                continue;
            }
            let label = sizes.len() as u32;
            let mut size = 0u32;
            labels[start] = label;
            stack.push(start);
            while let Some(i) = stack.pop() {
                size += 1;
                let (x, y, z) = (i % dims[0], (i / sy) % dims[1], i / sz);
                let mut visit = |j: usize| {
                    if labels[j] == CLOSED && values[j] >= clearance {
                        labels[j] = label;
                        stack.push(j);
                    }
                };
                if x > 0 {
                    visit(i - sx);
                }
                if x + 1 < dims[0] {
                    visit(i + sx);
                }
                if y > 0 {
                    visit(i - sy);
                }
                if y + 1 < dims[1] {
                    visit(i + sy);
                }
                if z > 0 {
                    visit(i - sz);
                }
                if z + 1 < dims[2] {
                    visit(i + sz);
                }
            }
            sizes.push(size);
        }
        ClearanceGrid {
            origin,
            step,
            dims,
            clearance,
            values,
            labels,
            sizes,
        }
    }

    /// Lattice spacing (u).
    pub fn step(&self) -> f32 {
        self.step
    }

    /// Free space an open lattice point has at least (u).
    pub fn clearance(&self) -> f32 {
        self.clearance
    }

    /// Number of connected components.
    pub fn component_count(&self) -> usize {
        self.sizes.len()
    }

    /// Lattice points in component `c`.
    pub fn component_size(&self, c: u32) -> usize {
        self.sizes.get(c as usize).map_or(0, |&s| s as usize)
    }

    /// The component with the most lattice points, if any point is open.
    pub fn largest_component(&self) -> Option<u32> {
        (0..self.sizes.len() as u32).max_by_key(|&c| (self.sizes[c as usize], u32::MAX - c))
    }

    /// The main open volume: the component of the level's first network
    /// hub, or the largest component when the level has no network.
    pub fn main_component(&self, gen: &Generator) -> Option<u32> {
        match gen.network().hubs().first() {
            Some(&hub) => self.nearest_component(hub, self.step * 2.0),
            None => self.largest_component(),
        }
    }

    /// Component of the lattice point containing `p` (nearest point), if
    /// that point is open.
    pub fn component_at(&self, p: Vec3) -> Option<u32> {
        let i = self.index(self.node_of(p)?)?;
        let l = self.labels[i];
        (l != CLOSED).then_some(l)
    }

    /// Component of the open lattice point closest to `p` within `radius`
    /// (ties go to the lower index).
    pub fn nearest_component(&self, p: Vec3, radius: f32) -> Option<u32> {
        let mut best: Option<(f32, u32)> = None;
        self.for_nodes_near(p, radius, |q, _, label| {
            if label != CLOSED {
                let d = q.distance(p);
                if d <= radius && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, label));
                }
            }
        });
        best.map(|(_, l)| l)
    }

    /// The lattice points within `radius` of `p` with their component
    /// (`None` when closed), in index order.
    pub fn nodes_near(&self, p: Vec3, radius: f32) -> Vec<(Vec3, Option<u32>)> {
        let mut out = Vec::new();
        self.for_nodes_near(p, radius, |q, _, label| {
            if q.distance(p) <= radius {
                out.push((q, (label != CLOSED).then_some(label)));
            }
        });
        out
    }

    /// Calls `f(position, sdf, label)` for the lattice points in the cube
    /// of half size `radius` around `p`, in index order.
    fn for_nodes_near(&self, p: Vec3, radius: f32, mut f: impl FnMut(Vec3, f32, u32)) {
        let lo = (p - Vec3::splat(radius) - self.origin) / self.step;
        let hi = (p + Vec3::splat(radius) - self.origin) / self.step;
        let range = |a: usize| {
            let l = (floor_i32(lo[a]) + 1).max(0);
            let h = floor_i32(hi[a]).min(self.dims[a] as i32 - 1);
            l..=h
        };
        for k in range(2) {
            for j in range(1) {
                for i in range(0) {
                    let n = [i as usize, j as usize, k as usize];
                    if let Some(idx) = self.index(n) {
                        f(self.position(n), self.values[idx], self.labels[idx]);
                    }
                }
            }
        }
    }

    /// Lattice point nearest `p`, if inside the lattice.
    fn node_of(&self, p: Vec3) -> Option<[usize; 3]> {
        let mut n = [0usize; 3];
        for a in 0..3 {
            let i = floor_i32((p[a] - self.origin[a]) / self.step + 0.5);
            if i < 0 || i as usize >= self.dims[a] {
                return None;
            }
            n[a] = i as usize;
        }
        Some(n)
    }

    fn index(&self, n: [usize; 3]) -> Option<usize> {
        (n[0] < self.dims[0] && n[1] < self.dims[1] && n[2] < self.dims[2])
            .then(|| n[0] + self.dims[0] * (n[1] + self.dims[1] * n[2]))
    }

    fn position(&self, n: [usize; 3]) -> Vec3 {
        self.origin + Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32) * self.step
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sliders;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Block decisions give exactly the open set of sampling every point,
    /// and decided values stay on the safe side of the true SDF.
    #[test]
    fn block_scan_matches_point_samples() {
        for (seed, e, d) in [(1u64, 0u8, 50u8), (2, 50, 40), (3, 75, 20), (4, 100, 100)] {
            let gen = Generator::new(seed, Sliders::new(e, d));
            let area = Aabb::new(Vec3::new(2.0, 2.0, 2.0), Vec3::new(70.0, 50.0, 60.0));
            let grid = ClearanceGrid::scan(&gen, area, 2.0, CONNECT_CLEARANCE);
            let mut open = 0;
            for k in 0..grid.dims[2] {
                for j in 0..grid.dims[1] {
                    for i in 0..grid.dims[0] {
                        let n = [i, j, k];
                        let idx = grid.index(n).unwrap();
                        let v = gen.sample(grid.position(n));
                        let got = grid.values[idx];
                        assert_eq!(v >= CONNECT_CLEARANCE, got >= CONNECT_CLEARANCE);
                        if v >= CONNECT_CLEARANCE {
                            assert!(got <= v, "{v} {got}");
                            open += 1;
                        }
                    }
                }
            }
            let total: usize = (0..grid.component_count() as u32)
                .map(|c| grid.component_size(c))
                .sum();
            assert_eq!(total, open);
        }
    }

    #[test]
    fn components_and_reach() {
        let gen = Generator::new(8, Sliders::new(50, 30));
        let grid = ClearanceGrid::level(&gen);
        let main = grid.main_component(&gen).unwrap();
        assert_eq!(grid.largest_component(), Some(main));
        let hub = gen.network().hubs()[3];
        assert_eq!(grid.nearest_component(hub, 2.0), Some(main));
        let near = grid.nodes_near(hub, 2.0);
        assert!(!near.is_empty() && near.iter().all(|(_, c)| *c == Some(main)));
        // In the sealing wall nothing is open.
        let wall = Vec3::new(2.5, 60.0, 60.0);
        assert_eq!(grid.component_at(wall), None);
    }
}
