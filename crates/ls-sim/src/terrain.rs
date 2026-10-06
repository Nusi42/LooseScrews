//! How the simulation talks to terrain.
//!
//! The simulation never depends on `ls-worldgen`. It reads terrain through
//! [`ls_math::terrain::TerrainQuery`] and writes through [`SimTerrain::apply_edit`],
//! emitting [`TerrainEdit`] values for craters and paint. Paint the Void scores
//! come from [`SimTerrain::paint_coverage`], which the real chunk store (or an
//! adapter around it) implements; [`PaintedTerrain`] provides a simple grid-based
//! implementation for tests, bots and prototyping.
//!
//! **Paint team ids:** simulation teams are `0` to `3`. Paint edits carry the
//! simulation team unchanged ([`paint_team_id`] is the identity); [`NO_PAINT`]
//! (`255`, the same value as `ls_worldgen::NO_TEAM`) means unpainted, and a paint
//! edit with that id erases. See `docs/architecture/contracts.md`.

use std::collections::{BTreeMap, BTreeSet};

use ls_math::terrain::{
    paint_color, BoxRoomTerrain, PlaneTerrain, SurfaceStates, TerrainEdit, TerrainQuery,
    MATERIAL_ROCK,
};
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
    /// Surface cells painted by simulation teams 0 to 3.
    pub team_cells: [u64; 4],
    /// Total paintable surface cells in the level.
    pub surface_cells: u64,
}

impl PaintCoverage {
    /// Share of the surface painted by `team`, in percent (0 if no surface).
    pub fn percent(&self, team: u8) -> f32 {
        if self.surface_cells == 0 || team as usize >= self.team_cells.len() {
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

    /// Surface states (hot, charged, glow active at tick `now`, and
    /// permanent scorch) on the surface within `radius` of `center`: for a
    /// cell grid, every surface
    /// cell that overlaps the sphere counts (so a ship scraping a wall with
    /// `radius` = its hull radius plus a margin finds the wall's states).
    /// The world uses it for the charged-surface drain and hooks read it
    /// through `Ctx::surface_states_near`. Terrains without surface states
    /// report none.
    fn surface_states_near(&self, _center: Vec3, _radius: f32, _now: u64) -> SurfaceStates {
        SurfaceStates::NONE
    }

    /// Rock material id at `p` (`ls_math::terrain::MATERIAL_*`). Terrains
    /// without materials are ordinary rock everywhere.
    fn material_at(&self, _p: Vec3) -> u8 {
        MATERIAL_ROCK
    }

    /// Sets the solid shell a mode adds to the terrain (The Squeeze), or
    /// removes it (`None`). The world calls it at the start of every tick
    /// with the mode's shell for that tick
    /// (`modes::ModeModule::terrain_shell`). A terrain that supports it
    /// answers `sdf` as `min(terrain, shell.sdf)` from then on
    /// ([`ShelledTerrain`], `ls_game::GameTerrain`); the default ignores it
    /// (the mode's own crushing rules still apply).
    fn set_shell(&mut self, _shell: Option<ShellSphere>) {}

    /// The Architect's reshape strokes (M4, `docs/milestone-4-plan.md`
    /// §3.5 "Stroke edits"): the field (signed distance, negative in rock)
    /// the level's generator gives at each of `points` with world slider
    /// `slider` ([`reshape_slider`]: share-code order) moved by `delta`
    /// (clamped to 0..=100), written to `out` (cleared first). `false`
    /// (and `out` empty) when the terrain has no generator, the slider
    /// cannot be reshaped (Verticality changes the level box) or the
    /// clamped value equals the level's. The world compares it with the
    /// edited terrain and emits ordinary `AddSphere` / `SubtractSphere`
    /// edits (`modes::the_architect::plan_reshape`).
    fn reshape_sample(
        &self,
        _slider: u8,
        _delta: i8,
        _points: &[Vec3],
        out: &mut Vec<f32>,
    ) -> bool {
        out.clear();
        false
    }
}

/// World slider indices of [`SimTerrain::reshape_sample`] (share-code
/// order of `ls_worldgen::Sliders`).
pub mod reshape_slider {
    pub const ENCLOSURE: u8 = 0;
    pub const DENSITY: u8 = 1;
    pub const VERTICALITY: u8 = 2;
    pub const GRAVITY_CHAOS: u8 = 3;
    pub const PALETTE: u8 = 4;
    pub const ABSURDITY: u8 = 5;
    pub const INSTABILITY: u8 = 6;
}

/// The Squeeze's wall: solid rock everywhere outside a sphere (the safe
/// zone). Its field is `radius - |p - center|` (negative in rock, exact,
/// 1-Lipschitz); a terrain with a shell answers `min(terrain, shell)`.
/// See `docs/architecture/contracts.md` (Shell layer).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShellSphere {
    pub center: Vec3,
    pub radius: f32,
}

impl ShellSphere {
    /// Signed distance of the shell (negative in its rock).
    #[inline]
    pub fn sdf(&self, p: Vec3) -> f32 {
        self.radius - p.distance(self.center)
    }

    /// Whether the shell can change `min(terrain, shell)` anywhere in
    /// `region` for a terrain whose field never exceeds `clamp` (false: the
    /// whole region lies deeper than `clamp` inside the safe sphere).
    pub fn touches(&self, region: Aabb, clamp: f32) -> bool {
        farthest_distance(region, self.center) > self.radius - clamp
    }
}

/// Distance from `p` to the farthest corner of `b`.
pub fn farthest_distance(b: Aabb, p: Vec3) -> f32 {
    let far = |v: f32, lo: f32, hi: f32| (v - lo).abs().max((hi - v).abs());
    Vec3::new(
        far(p.x, b.min.x, b.max.x),
        far(p.y, b.min.y, b.max.y),
        far(p.z, b.min.z, b.max.z),
    )
    .length()
}

/// Any [`SimTerrain`] plus a [`ShellSphere`] layer (`sdf = min(inner,
/// shell)`): the reference implementation of [`SimTerrain::set_shell`] for
/// tests and tools (the game's terrain, `ls_game::GameTerrain`, has its
/// own). Everything else is forwarded to `inner`.
#[derive(Debug, Clone)]
pub struct ShelledTerrain<T> {
    pub inner: T,
    pub shell: Option<ShellSphere>,
}

impl<T> ShelledTerrain<T> {
    pub fn new(inner: T) -> Self {
        ShelledTerrain { inner, shell: None }
    }
}

impl<T: SimTerrain> TerrainQuery for ShelledTerrain<T> {
    fn sdf(&self, p: Vec3) -> f32 {
        let d = self.inner.sdf(p);
        match self.shell {
            Some(s) => d.min(s.sdf(p)),
            None => d,
        }
    }
    fn bounds(&self) -> Aabb {
        self.inner.bounds()
    }
    fn paint_colors_near(&self, center: Vec3, radius: f32) -> u8 {
        self.inner.paint_colors_near(center, radius)
    }
}

impl<T: SimTerrain> SimTerrain for ShelledTerrain<T> {
    fn apply_edit(&mut self, edit: &TerrainEdit) {
        self.inner.apply_edit(edit);
    }
    fn paint_coverage(&self) -> Option<PaintCoverage> {
        self.inner.paint_coverage()
    }
    fn surface_states_near(&self, center: Vec3, radius: f32, now: u64) -> SurfaceStates {
        self.inner.surface_states_near(center, radius, now)
    }
    fn material_at(&self, p: Vec3) -> u8 {
        self.inner.material_at(p)
    }
    fn set_shell(&mut self, shell: Option<ShellSphere>) {
        self.shell = shell;
    }
    fn reshape_sample(&self, slider: u8, delta: i8, points: &[Vec3], out: &mut Vec<f32>) -> bool {
        self.inner.reshape_sample(slider, delta, points, out)
    }
}

/// Any [`SimTerrain`] that answers [`SimTerrain::reshape_sample`]
/// analytically (tests, replays and tools of The Architect without a
/// generator): Enclosure moves the surface `0.15 x delta` u into the open
/// (a positive delta grows rock everywhere, a negative one opens it),
/// Density adds vertical pillars (radius `0.05 x delta` u, at most 3 u) on
/// a 12 u grid, the other sliders give nothing. The level box (the
/// terrain's bounds shrunk by 2 u) stays solid either way. Everything else
/// is forwarded to `inner`.
#[derive(Debug, Clone)]
pub struct AnalyticReshape<T> {
    pub inner: T,
}

impl<T> AnalyticReshape<T> {
    pub fn new(inner: T) -> Self {
        AnalyticReshape { inner }
    }
}

impl<T: SimTerrain> TerrainQuery for AnalyticReshape<T> {
    fn sdf(&self, p: Vec3) -> f32 {
        self.inner.sdf(p)
    }
    fn bounds(&self) -> Aabb {
        self.inner.bounds()
    }
    fn paint_colors_near(&self, center: Vec3, radius: f32) -> u8 {
        self.inner.paint_colors_near(center, radius)
    }
}

impl<T: SimTerrain> SimTerrain for AnalyticReshape<T> {
    fn apply_edit(&mut self, edit: &TerrainEdit) {
        self.inner.apply_edit(edit);
    }
    fn paint_coverage(&self) -> Option<PaintCoverage> {
        self.inner.paint_coverage()
    }
    fn surface_states_near(&self, center: Vec3, radius: f32, now: u64) -> SurfaceStates {
        self.inner.surface_states_near(center, radius, now)
    }
    fn material_at(&self, p: Vec3) -> u8 {
        self.inner.material_at(p)
    }
    fn set_shell(&mut self, shell: Option<ShellSphere>) {
        self.inner.set_shell(shell);
    }
    fn reshape_sample(&self, slider: u8, delta: i8, points: &[Vec3], out: &mut Vec<f32>) -> bool {
        out.clear();
        if delta == 0 || !matches!(slider, reshape_slider::ENCLOSURE | reshape_slider::DENSITY) {
            return false;
        }
        let b = self.inner.bounds();
        let inner_box = Aabb::new(b.min + Vec3::splat(2.0), b.max - Vec3::splat(2.0));
        let d = delta as f32;
        out.extend(points.iter().map(|&p| {
            let base = self.inner.sdf(p);
            let shaped = if slider == reshape_slider::ENCLOSURE {
                base - 0.15 * d
            } else if d > 0.0 {
                let cell = 12.0;
                let cx = det::floor(p.x / cell) * cell + 0.5 * cell;
                let cz = det::floor(p.z / cell) * cell + 0.5 * cell;
                let dx = p.x - cx;
                let dz = p.z - cz;
                let pillar = (dx * dx + dz * dz).sqrt() - (0.05 * d).min(3.0);
                base.min(pillar)
            } else {
                base
            };
            // Solid outside the level box (`-inner_box.sdf`: negative outside).
            shaped.min(-inner_box.sdf(p))
        }));
        true
    }
}

/// The read-only terrain layers hook code sees through `View::layers`
/// (implemented by the world for any [`SimTerrain`]).
pub trait TerrainLayers {
    /// See [`SimTerrain::surface_states_near`].
    fn surface_states_near(&self, center: Vec3, radius: f32, now: u64) -> SurfaceStates;
    /// See [`SimTerrain::material_at`].
    fn material_at(&self, p: Vec3) -> u8;
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
    /// Painted cells: (team, colour).
    paint: BTreeMap<Cell, (u8, u8)>,
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

    fn paint_sphere(&mut self, center: Vec3, radius: f32, team: u8, color: u8) {
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
                            self.paint.insert(c, (team, paint_color(color)));
                        }
                    }
                }
            }
        }
    }

    /// Paint team id of the cell containing `p`, or [`NO_PAINT`] if unpainted.
    pub fn paint_at(&self, p: Vec3) -> u8 {
        let (c, _) = self.cell_range(Aabb::new(p, p));
        self.paint.get(&c).map_or(NO_PAINT, |&(t, _)| t)
    }

    /// Paint colour id (`ls_math::terrain::PAINT_*`) of the cell containing
    /// `p`, if painted.
    pub fn paint_color_at(&self, p: Vec3) -> Option<u8> {
        let (c, _) = self.cell_range(Aabb::new(p, p));
        self.paint.get(&c).map(|&(_, color)| color)
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
    /// Colours of the painted cells overlapping the sphere.
    fn paint_colors_near(&self, center: Vec3, radius: f32) -> u8 {
        if !center.is_finite() || !(0.0..=64.0).contains(&radius) {
            return 0;
        }
        let region = Aabb::from_center_half_extents(center, Vec3::splat(radius));
        let (lo, hi) = self.cell_range(region);
        let mut bits = 0u8;
        for (&c, &(_, color)) in self.paint.range(lo..=(hi.0, i32::MAX, i32::MAX)) {
            let inside = (lo.0..=hi.0).contains(&c.0)
                && (lo.1..=hi.1).contains(&c.1)
                && (lo.2..=hi.2).contains(&c.2);
            let min = Vec3::new(c.0 as f32, c.1 as f32, c.2 as f32) * self.cell;
            let cell = Aabb::new(min, min + Vec3::splat(self.cell));
            if inside && cell.sdf(center) <= radius {
                bits |= 1 << color;
            }
        }
        bits
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
                color,
            } => self.paint_sphere(center, radius, team, color),
            _ => {
                let region = edit.bounds().expand(self.cell * 2.0);
                self.refresh_region(region);
            }
        }
    }

    fn surface_states_near(&self, center: Vec3, radius: f32, now: u64) -> SurfaceStates {
        self.inner.surface_states_near(center, radius, now)
    }

    fn material_at(&self, p: Vec3) -> u8 {
        self.inner.material_at(p)
    }

    fn reshape_sample(&self, slider: u8, delta: i8, points: &[Vec3], out: &mut Vec<f32>) -> bool {
        self.inner.reshape_sample(slider, delta, points, out)
    }

    fn paint_coverage(&self) -> Option<PaintCoverage> {
        let mut team_cells = [0u64; 4];
        for &(id, _) in self.paint.values() {
            if let Some(team) = team_from_paint_id(id) {
                if (team as usize) < team_cells.len() {
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

/// Smallest field slope [`surface_estimate`] divides by: where the field is
/// flatter than this (degenerate gradients, kinks), the estimate is at most
/// `1 / MIN_CONTACT_SLOPE` times the bound.
pub const MIN_CONTACT_SLOPE: f32 = 0.2;

/// Estimated distance to the surface and the open-space normal near a point
/// (see [`surface_estimate`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceEstimate {
    /// First-order distance estimate to the surface (negative inside rock).
    pub distance: f32,
    /// Unit normal pointing into open space (the gradient of [`world_sdf`]).
    pub normal: Vec3,
}

/// First-order distance estimate `d / |grad d|` of [`world_sdf`] at `p`, with
/// its normal (one central-difference gradient: 6 extra field evaluations).
///
/// The terrain field is only a lower bound on the distance (the cave
/// generator's noise reports about 0.3 of it), so a sphere that treated `d`
/// as the distance would stop well short of the visible rock. Dividing by
/// the local slope (clamped to `[MIN_CONTACT_SLOPE, 1]`, so the estimate is
/// never below the bound) gives the true distance to first order; exact
/// fields (slope 1) are unchanged. Inside rock (`d <= 0`) the bound itself is
/// returned, so the push-out never overshoots on a clamped, flat field.
///
/// Deterministic: plain IEEE arithmetic and `sqrt` only. Sphere contact uses
/// it ([`sphere_contact_distance`], `ship::move_sphere_with`); sphere tracing
/// keeps the bound. See `docs/architecture/contracts.md` (terrain distance).
pub fn surface_estimate<T: TerrainQuery + ?Sized>(t: &T, p: Vec3) -> SurfaceEstimate {
    estimate_from(t, p, world_sdf(t, p))
}

/// [`surface_estimate`] with `d = world_sdf(t, p)` already evaluated.
fn estimate_from<T: TerrainQuery + ?Sized>(t: &T, p: Vec3, d: f32) -> SurfaceEstimate {
    let e = ls_math::terrain::NORMAL_EPS;
    let f = |q: Vec3| world_sdf(t, q);
    let g = Vec3::new(
        f(p + Vec3::new(e, 0.0, 0.0)) - f(p - Vec3::new(e, 0.0, 0.0)),
        f(p + Vec3::new(0.0, e, 0.0)) - f(p - Vec3::new(0.0, e, 0.0)),
        f(p + Vec3::new(0.0, 0.0, e)) - f(p - Vec3::new(0.0, 0.0, e)),
    );
    let len = g.length();
    let normal = if len > 0.0 && len.is_finite() {
        g / len
    } else {
        Vec3::Y
    };
    let slope = (len / (2.0 * e)).clamp(MIN_CONTACT_SLOPE, 1.0);
    let distance = if d > 0.0 { d / slope } else { d };
    SurfaceEstimate { distance, normal }
}

/// The surface estimate a sphere contact test needs, or `None` when the
/// bound [`world_sdf`] already proves the point at least `reach` (the sphere
/// radius plus any margin) from the rock: one field evaluation then, so open
/// flight costs nothing extra.
pub fn sphere_contact<T: TerrainQuery + ?Sized>(
    t: &T,
    p: Vec3,
    reach: f32,
) -> Option<SurfaceEstimate> {
    let d = world_sdf(t, p);
    if d >= reach {
        None
    } else {
        Some(estimate_from(t, p, d))
    }
}

/// Distance from `p` to the surface for a sphere test whose answer only
/// matters below `reach`: the bound when it is `>= reach`, else the
/// [`surface_estimate`] (see [`sphere_contact`]).
pub fn sphere_contact_distance<T: TerrainQuery + ?Sized>(t: &T, p: Vec3, reach: f32) -> f32 {
    let d = world_sdf(t, p);
    if d >= reach {
        d
    } else {
        estimate_from(t, p, d).distance
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
            color: 0,
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
            color: 0,
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
            color: 0,
        });
        assert_eq!(t.paint_coverage().unwrap().team_cells, [0; 4]);
        assert_eq!(t.paint_at(floor + Vec3::new(3.2, 0.1, 0.2)), NO_PAINT);
    }

    #[test]
    fn painted_terrain_reports_paint_colours_under_a_sphere() {
        use ls_math::terrain::{PAINT_BLUE, PAINT_YELLOW};
        let room = BoxRoomTerrain::centered(Vec3::new(20.0, 10.0, 20.0));
        let mut t = PaintedTerrain::new(room, 1.0);
        let floor = Vec3::new(0.0, -5.0, 0.0);
        let paint = |team, color, x: f32| TerrainEdit::Paint {
            center: floor + Vec3::X * x,
            radius: 2.0,
            team,
            color,
        };
        t.apply_edit(&paint(0, PAINT_YELLOW, 0.0));
        t.apply_edit(&paint(1, PAINT_BLUE, 6.0));
        let above = floor + Vec3::Y * 0.6;
        assert_eq!(t.paint_colors_near(above, 0.7), 1 << PAINT_YELLOW);
        assert_eq!(
            t.paint_color_at(floor + Vec3::new(0.2, 0.1, 0.2)),
            Some(PAINT_YELLOW)
        );
        assert_eq!(
            t.paint_colors_near(above + Vec3::X * 3.0, 1.2),
            (1 << PAINT_YELLOW) | (1 << PAINT_BLUE)
        );
        // Out of reach of the floor: nothing.
        assert_eq!(t.paint_colors_near(floor + Vec3::Y * 3.0, 0.7), 0);
        // Painting over replaces the colour; coverage counts teams only.
        t.apply_edit(&paint(1, PAINT_BLUE, 0.0));
        assert_eq!(t.paint_colors_near(above, 0.7), 1 << PAINT_BLUE);
        assert_eq!(t.paint_coverage().unwrap().team_cells[0], 0);
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
