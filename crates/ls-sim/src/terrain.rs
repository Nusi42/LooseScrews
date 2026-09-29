//! How the simulation talks to terrain.
//!
//! The simulation never depends on `ls-worldgen`. It reads terrain through
//! [`ls_math::terrain::TerrainQuery`] and writes through [`SimTerrain::apply_edit`],
//! emitting [`TerrainEdit`] values for craters and paint. Paint the Void scores
//! come from [`SimTerrain::paint_coverage`], which the real chunk store (or an
//! adapter around it) implements; [`PaintedTerrain`] provides a simple grid-based
//! implementation for tests, bots and prototyping.
//!
//! **Paint team ids:** simulation teams are `0` and `1`. Paint edits carry the
//! simulation team unchanged ([`paint_team_id`] is the identity); [`NO_PAINT`]
//! (`255`, the same value as `ls_worldgen::NO_TEAM`) means unpainted, and a paint
//! edit with that id erases. See `docs/architecture/contracts.md`.

use std::collections::{BTreeMap, BTreeSet};

use ls_math::terrain::{BoxRoomTerrain, PlaneTerrain, TerrainEdit, TerrainQuery};
use ls_math::{det, Aabb, Vec3};
use serde::{Deserialize, Serialize};

/// Paint id meaning "unpainted"; a [`TerrainEdit::Paint`] with this id erases paint.
pub const NO_PAINT: u8 = u8::MAX;

/// Paint team id written into [`TerrainEdit::Paint`] for simulation team `team`
/// (the identity: paint ids are simulation team ids).
#[inline]
pub const fn paint_team_id(team: u8) -> u8 {
    team
}

/// Inverse of [`paint_team_id`]; `None` for [`NO_PAINT`] (unpainted).
#[inline]
pub const fn team_from_paint_id(id: u8) -> Option<u8> {
    if id == NO_PAINT {
        None
    } else {
        Some(id)
    }
}

/// Painted surface statistics for Paint the Void.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PaintCoverage {
    /// Surface cells painted by simulation team 0 and 1.
    pub team_cells: [u64; 2],
    /// Total paintable surface cells in the level.
    pub surface_cells: u64,
}

impl PaintCoverage {
    /// Share of the surface painted by `team`, in percent (0 if no surface).
    pub fn percent(&self, team: u8) -> f32 {
        if self.surface_cells == 0 || team > 1 {
            return 0.0;
        }
        (self.team_cells[team as usize] as f64 * 100.0 / self.surface_cells as f64) as f32
    }
}

/// Terrain as seen by the simulation: queries plus edits plus (optionally) paint stats.
pub trait SimTerrain: TerrainQuery {
    /// Applies an edit produced by the simulation, in tick order.
    fn apply_edit(&mut self, edit: &TerrainEdit);

    /// Current paint coverage, if this terrain tracks paint. Without it,
    /// Paint the Void scores stay at zero.
    fn paint_coverage(&self) -> Option<PaintCoverage> {
        None
    }
}

impl SimTerrain for BoxRoomTerrain {
    fn apply_edit(&mut self, edit: &TerrainEdit) {
        self.apply(*edit);
    }
}

/// A plane cannot change shape; edits are ignored.
impl SimTerrain for PlaneTerrain {
    fn apply_edit(&mut self, _edit: &TerrainEdit) {}
}

type Cell = (i32, i32, i32);

/// Wraps any [`SimTerrain`] with a coarse surface paint grid so it can report
/// [`PaintCoverage`]. A cell counts as surface when `|sdf(cell center)|` is below
/// half the cell diagonal. The surface set is built once over the terrain bounds
/// and refreshed locally after geometry edits (craters wipe paint off).
///
/// Cost is proportional to `volume / cell³`; meant for test rooms and prototypes,
/// the real chunk store reports its own coverage.
#[derive(Debug, Clone)]
pub struct PaintedTerrain<T> {
    pub inner: T,
    cell: f32,
    surface: BTreeSet<Cell>,
    paint: BTreeMap<Cell, u8>,
}

impl<T: SimTerrain> PaintedTerrain<T> {
    pub fn new(inner: T, cell: f32) -> Self {
        let cell = if cell.is_finite() && cell > 0.0 {
            cell
        } else {
            1.0
        };
        let mut t = PaintedTerrain {
            inner,
            cell,
            surface: BTreeSet::new(),
            paint: BTreeMap::new(),
        };
        let bounds = t.inner.bounds();
        t.refresh_region(bounds);
        t
    }

    fn cell_range(&self, region: Aabb) -> (Cell, Cell) {
        let lo = region.min / self.cell;
        let hi = region.max / self.cell;
        (
            (
                det::floor(lo.x) as i32,
                det::floor(lo.y) as i32,
                det::floor(lo.z) as i32,
            ),
            (
                det::floor(hi.x) as i32,
                det::floor(hi.y) as i32,
                det::floor(hi.z) as i32,
            ),
        )
    }

    fn center(&self, c: Cell) -> Vec3 {
        Vec3::new(
            (c.0 as f32 + 0.5) * self.cell,
            (c.1 as f32 + 0.5) * self.cell,
            (c.2 as f32 + 0.5) * self.cell,
        )
    }

    fn is_surface(&self, c: Cell) -> bool {
        let p = self.center(c);
        self.inner.bounds().contains(p) && self.inner.sdf(p).abs() <= self.cell * 0.866
    }

    fn refresh_region(&mut self, region: Aabb) {
        let (lo, hi) = self.cell_range(region);
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    let c = (x, y, z);
                    if self.is_surface(c) {
                        self.surface.insert(c);
                    } else {
                        self.surface.remove(&c);
                        self.paint.remove(&c);
                    }
                }
            }
        }
    }

    fn paint_sphere(&mut self, center: Vec3, radius: f32, team: u8) {
        let region = Aabb::from_center_half_extents(center, Vec3::splat(radius));
        let (lo, hi) = self.cell_range(region);
        let r2 = radius * radius;
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    let c = (x, y, z);
                    if self.surface.contains(&c) && self.center(c).distance_squared(center) <= r2 {
                        if team == NO_PAINT {
                            self.paint.remove(&c);
                        } else {
                            self.paint.insert(c, team);
                        }
                    }
                }
            }
        }
    }

    /// Paint team id of the cell containing `p`, or [`NO_PAINT`] if unpainted.
    pub fn paint_at(&self, p: Vec3) -> u8 {
        let (c, _) = self.cell_range(Aabb::new(p, p));
        self.paint.get(&c).copied().unwrap_or(NO_PAINT)
    }

    pub fn surface_cells(&self) -> u64 {
        self.surface.len() as u64
    }
}

impl<T: SimTerrain> TerrainQuery for PaintedTerrain<T> {
    fn sdf(&self, p: Vec3) -> f32 {
        self.inner.sdf(p)
    }
    fn normal(&self, p: Vec3) -> Vec3 {
        self.inner.normal(p)
    }
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<ls_math::terrain::RayHit> {
        self.inner.raycast(origin, dir, max_dist)
    }
    fn bounds(&self) -> Aabb {
        self.inner.bounds()
    }
    fn sphere_overlap(&self, center: Vec3, radius: f32) -> bool {
        self.inner.sphere_overlap(center, radius)
    }
}

impl<T: SimTerrain> SimTerrain for PaintedTerrain<T> {
    fn apply_edit(&mut self, edit: &TerrainEdit) {
        self.inner.apply_edit(edit);
        match *edit {
            TerrainEdit::Paint {
                center,
                radius,
                team,
            } => self.paint_sphere(center, radius, team),
            _ => {
                let region = edit.bounds().expand(self.cell * 2.0);
                self.refresh_region(region);
            }
        }
    }

    fn paint_coverage(&self) -> Option<PaintCoverage> {
        let mut team_cells = [0u64; 2];
        for &id in self.paint.values() {
            if let Some(team) = team_from_paint_id(id) {
                if (team as usize) < 2 {
                    team_cells[team as usize] += 1;
                }
            }
        }
        Some(PaintCoverage {
            team_cells,
            surface_cells: self.surface.len() as u64,
        })
    }
}

// ---------------------------------------------------------------------------
// Physics helpers shared by ships and projectiles.

/// Signed distance that also treats everything outside the level bounds as solid.
pub fn world_sdf<T: TerrainQuery + ?Sized>(t: &T, p: Vec3) -> f32 {
    t.sdf(p).min(-t.bounds().sdf(p))
}

/// Open-space normal matching [`world_sdf`].
pub fn world_normal<T: TerrainQuery + ?Sized>(t: &T, p: Vec3) -> Vec3 {
    let terrain = t.sdf(p);
    let bounds = -t.bounds().sdf(p);
    if terrain <= bounds {
        t.normal(p)
    } else {
        ls_math::terrain::sdf_gradient(|q| -t.bounds().sdf(q), p)
    }
}

/// Earliest `t` in `[0, max]` where the segment `origin + dir * t` (unit `dir`)
/// enters the sphere, or `None`. A segment starting inside the sphere hits at 0.
pub fn segment_sphere(origin: Vec3, dir: Vec3, max: f32, center: Vec3, radius: f32) -> Option<f32> {
    let m = origin - center;
    let c = m.dot(m) - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let b = m.dot(dir);
    if b > 0.0 {
        return None;
    }
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    if t <= max {
        Some(t.max(0.0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn paint_ids_roundtrip() {
        assert_eq!(paint_team_id(0), 0);
        assert_eq!(team_from_paint_id(paint_team_id(0)), Some(0));
        assert_eq!(team_from_paint_id(paint_team_id(1)), Some(1));
        assert_eq!(team_from_paint_id(NO_PAINT), None);
    }

    #[test]
    fn painted_terrain_counts_coverage() {
        let room = BoxRoomTerrain::centered(Vec3::new(20.0, 10.0, 20.0));
        let mut t = PaintedTerrain::new(room, 1.0);
        let surface = t.surface_cells();
        assert!(surface > 1000, "{surface}");
        let floor = Vec3::new(0.0, -5.0, 0.0);
        t.apply_edit(&TerrainEdit::Paint {
            center: floor,
            radius: 2.0,
            team: paint_team_id(1),
        });
        let cov = t.paint_coverage().unwrap();
        assert_eq!(cov.team_cells[0], 0);
        assert!(cov.team_cells[1] > 4);
        assert_eq!(
            t.paint_at(floor + Vec3::new(0.2, 0.1, 0.2)),
            paint_team_id(1)
        );
        assert!(cov.percent(1) > 0.0 && cov.percent(1) < 5.0);
        // Painting over switches ownership.
        t.apply_edit(&TerrainEdit::Paint {
            center: floor,
            radius: 3.0,
            team: paint_team_id(0),
        });
        let cov2 = t.paint_coverage().unwrap();
        assert_eq!(cov2.team_cells[1], 0);
        assert!(cov2.team_cells[0] > cov.team_cells[1]);
        // A crater changes the surface and wipes paint inside it.
        t.apply_edit(&TerrainEdit::SubtractSphere {
            center: floor,
            radius: 2.0,
        });
        let cov3 = t.paint_coverage().unwrap();
        assert!(cov3.team_cells[0] < cov2.team_cells[0]);
        assert_ne!(cov3.surface_cells, surface);
        // Painting with NO_PAINT erases.
        t.apply_edit(&TerrainEdit::Paint {
            center: floor,
            radius: 6.0,
            team: NO_PAINT,
        });
        assert_eq!(t.paint_coverage().unwrap().team_cells, [0, 0]);
        assert_eq!(t.paint_at(floor + Vec3::new(3.2, 0.1, 0.2)), NO_PAINT);
    }

    #[test]
    fn world_sdf_includes_bounds() {
        let bounds = Aabb::new(Vec3::splat(-10.0), Vec3::splat(10.0));
        let plane = PlaneTerrain::floor(-5.0, bounds);
        // Near the +X bound the bound wall is closer than the floor.
        let p = Vec3::new(9.0, 0.0, 0.0);
        assert!((world_sdf(&plane, p) - 1.0).abs() < 1e-6);
        let n = world_normal(&plane, p);
        assert!((n - Vec3::NEG_X).length() < 1e-3, "{n:?}");
        assert_eq!(world_normal(&plane, Vec3::new(0.0, -4.0, 0.0)), Vec3::Y);
    }

    #[test]
    fn segment_sphere_cases() {
        let hit = segment_sphere(Vec3::ZERO, Vec3::X, 10.0, Vec3::new(5.0, 0.0, 0.0), 1.0);
        assert_eq!(hit, Some(4.0));
        assert_eq!(
            segment_sphere(Vec3::ZERO, Vec3::X, 3.0, Vec3::new(5.0, 0.0, 0.0), 1.0),
            None
        );
        assert_eq!(
            segment_sphere(Vec3::ZERO, Vec3::NEG_X, 10.0, Vec3::new(5.0, 0.0, 0.0), 1.0),
            None
        );
        assert_eq!(
            segment_sphere(Vec3::ZERO, Vec3::X, 10.0, Vec3::new(0.5, 0.0, 0.0), 1.0),
            Some(0.0)
        );
        assert_eq!(
            segment_sphere(Vec3::ZERO, Vec3::X, 10.0, Vec3::new(5.0, 3.0, 0.0), 1.0),
            None
        );
    }
}
