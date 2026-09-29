//! [`GameTerrain`]: the level's chunk store as seen by the simulation, the
//! network layer and the renderer.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

use ls_math::terrain::{SurfaceStates, TerrainEdit, TerrainQuery};
use ls_math::{Aabb, Vec3};
use ls_net::TerrainEditBits;
use ls_sim::{SimTerrain, StateHasher};
use ls_worldgen::{ChunkCoord, ChunkStore, Generator, Material, Mesh, ShareCode};

use crate::GameError;

/// A level: [`ChunkStore`] plus the match's terrain edit log.
///
/// - Implements [`TerrainQuery`] (physics) and [`SimTerrain`] (edits from the
///   simulation, Paint the Void coverage).
/// - Material strength: the simulation scales craters by the rock material
///   at their centre itself (`ls_sim::Ctx::crater`, reading
///   [`SimTerrain::material_at`]), so every edit, from the simulation
///   ([`SimTerrain::apply_edit`]) or from a log ([`GameTerrain::apply`],
///   [`GameTerrain::apply_sequence`], [`GameTerrain::replay`]), is applied
///   and logged verbatim.
/// - Surface states (hot, charged, glow) are stored as expiry ticks by the
///   edit log; [`GameTerrain::advance_to`] moves the display tick of meshes
///   and marks chunks whose states expired dirty.
/// - Every edit handed to it is logged, including ones the chunk store rejects
///   as invalid, so the log index is the edit's **sequence number** on the wire
///   (`Snapshot::edit_base`, `Welcome::edit_log`) on every peer.
/// - Tracks chunks whose meshes changed since the last
///   [`GameTerrain::take_dirty_chunks`].
///
/// The store sits in a `RefCell` because `SimTerrain::paint_coverage` takes
/// `&self` while the chunk store generates missing chunks on demand. The type
/// is `Send` but not `Sync`; keep it on the thread that steps the world.
#[derive(Debug, Clone)]
pub struct GameTerrain {
    store: RefCell<ChunkStore>,
    bounds: Aabb,
    log: Vec<TerrainEdit>,
    dirty: BTreeSet<ChunkCoord>,
    coverage: Cell<Option<ls_sim::PaintCoverage>>,
}

impl GameTerrain {
    /// A pristine level for `gen`.
    pub fn new(gen: Generator) -> Self {
        let bounds = gen.bounds();
        GameTerrain {
            store: RefCell::new(ChunkStore::new(gen)),
            bounds,
            log: Vec::new(),
            dirty: BTreeSet::new(),
            coverage: Cell::new(None),
        }
    }

    /// A pristine level for a decoded share code.
    pub fn from_code(code: &ShareCode) -> Result<Self, GameError> {
        Ok(Self::new(Generator::from_code(code)?))
    }

    /// A pristine level for a printed share code (`LSxx-...`).
    pub fn from_share_code(text: &str) -> Result<Self, GameError> {
        Self::from_code(&ShareCode::decode(text)?)
    }

    /// A level with `edits` applied in order (late join from `Welcome::edit_log`).
    pub fn replay(gen: Generator, edits: &[TerrainEdit]) -> Self {
        let mut t = Self::new(gen);
        for e in edits {
            t.apply(*e);
        }
        t
    }

    pub fn generator(&self) -> Generator {
        self.store.borrow().generator().clone()
    }

    pub fn share_code(&self) -> ShareCode {
        self.store.borrow().generator().code()
    }

    /// Number of edits applied so far = sequence number of the next edit.
    pub fn edit_count(&self) -> u32 {
        self.log.len() as u32
    }

    /// Every edit applied so far, in order.
    pub fn edit_log(&self) -> &[TerrainEdit] {
        &self.log
    }

    /// Applies and logs an edit verbatim; returns the chunks whose meshes may
    /// have changed.
    pub fn apply(&mut self, edit: TerrainEdit) -> Vec<ChunkCoord> {
        self.log.push(edit);
        let dirty = self.store.get_mut().apply(edit);
        self.dirty.extend(dirty.iter().copied());
        self.coverage.set(None);
        dirty
    }

    /// Applies the part of a sequence of edits starting at sequence number
    /// `base` that this terrain does not have yet. Returns how many were new.
    /// Fails with [`GameError::EditGap`] if `base` lies beyond
    /// [`GameTerrain::edit_count`] (edits were missed).
    pub fn apply_sequence(&mut self, base: u32, edits: &[TerrainEdit]) -> Result<usize, GameError> {
        let have = self.edit_count();
        if base > have {
            return Err(GameError::EditGap { have, base });
        }
        let skip = (have - base) as usize;
        let new = edits.get(skip..).unwrap_or(&[]);
        for e in new {
            self.apply(*e);
        }
        Ok(new.len())
    }

    /// Like [`GameTerrain::apply_sequence`] for wire edits (bit-exact).
    pub fn apply_wire_sequence(
        &mut self,
        base: u32,
        edits: &[TerrainEditBits],
    ) -> Result<usize, GameError> {
        let edits: Vec<TerrainEdit> = edits.iter().map(|e| TerrainEdit::from(*e)).collect();
        self.apply_sequence(base, &edits)
    }

    /// Moves the display tick of meshes to `tick` (surface states active at
    /// it are meshed) and marks the chunks whose states expired since the
    /// last call dirty. Does not change the terrain state or its hashes.
    pub fn advance_to(&mut self, tick: u64) {
        let now = tick.min(u32::MAX as u64) as u32;
        let dirty = self.store.get_mut().advance_to(now);
        self.dirty.extend(dirty);
    }

    /// Display tick of meshes (see [`GameTerrain::advance_to`]).
    pub fn display_tick(&self) -> u64 {
        self.store.borrow().now() as u64
    }

    /// Surface states active at `tick` on the cells within `radius` of
    /// `center` (e.g. a charged wall under a scraping ship).
    pub fn surface_states_near(&self, center: Vec3, radius: f32, tick: u64) -> SurfaceStates {
        let now = tick.min(u32::MAX as u64) as u32;
        self.store.borrow().surface_states_near(center, radius, now)
    }

    /// Surface states active at `tick` on the cell containing `p`.
    pub fn surface_states_at(&self, p: Vec3, tick: u64) -> SurfaceStates {
        let now = tick.min(u32::MAX as u64) as u32;
        self.store.borrow().surface_states_at(p, now)
    }

    /// Rock material (strength class) at `p`.
    pub fn material_at(&self, p: Vec3) -> Material {
        self.store.borrow().material_at(p)
    }

    /// Chunks whose meshes changed since the last call (sorted, unique).
    pub fn take_dirty_chunks(&mut self) -> Vec<ChunkCoord> {
        std::mem::take(&mut self.dirty).into_iter().collect()
    }

    /// Chunks currently marked dirty, without clearing them.
    pub fn dirty_chunks(&self) -> impl Iterator<Item = &ChunkCoord> {
        self.dirty.iter()
    }

    /// All chunk coordinates of the level, ascending.
    pub fn chunk_coords(&self) -> Vec<ChunkCoord> {
        self.store.borrow().chunk_coords()
    }

    /// Surface-nets mesh of one chunk.
    pub fn mesh_chunk(&mut self, c: ChunkCoord) -> Mesh {
        self.store.get_mut().mesh_surface_nets(c)
    }

    /// Generates every chunk now (the server does this before the match so the
    /// first paint coverage query does not stall a tick).
    pub fn generate_all(&mut self) {
        self.store.get_mut().generate_all();
    }

    /// Read access to the chunk store.
    pub fn with_store<R>(&self, f: impl FnOnce(&ChunkStore) -> R) -> R {
        f(&self.store.borrow())
    }

    /// Nearest open point with `clearance` of free space (see
    /// [`ChunkStore::find_open_point`]).
    pub fn find_open_point(&self, near: Vec3, clearance: f32, max_radius: i32) -> Option<Vec3> {
        self.store
            .borrow()
            .find_open_point(near, clearance, max_radius)
    }

    /// Paint coverage as reported by the chunk store (all teams).
    pub fn full_paint_coverage(&self) -> ls_worldgen::PaintCoverage {
        self.store.borrow_mut().paint_coverage()
    }

    /// FNV-1a hash of the edit log (raw `f32` bits).
    pub fn edit_log_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u64(self.log.len() as u64);
        for e in &self.log {
            let b = TerrainEditBits::from(*e);
            h.write_u8(b.kind);
            for c in b.center {
                h.write_u32(c);
            }
            h.write_u32(b.radius);
            h.write_u8(b.team);
            if b.kind == 3 {
                h.write_u32(b.until);
            }
        }
        h.finish()
    }

    /// FNV-1a hash of the SDF sampled on a lattice with spacing `step` (cell
    /// centres, offset by `step / 2`) over the level bounds.
    pub fn sdf_hash(&self, step: f32) -> u64 {
        let store = self.store.borrow();
        let b = self.bounds;
        let mut h = StateHasher::new();
        let n = |lo: f32, hi: f32| ((hi - lo) / step) as i32;
        let (nx, ny, nz) = (
            n(b.min.x, b.max.x),
            n(b.min.y, b.max.y),
            n(b.min.z, b.max.z),
        );
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let p = b.min
                        + Vec3::new(
                            (i as f32 + 0.5) * step,
                            (j as f32 + 0.5) * step,
                            (k as f32 + 0.5) * step,
                        );
                    h.write_f32(store.eval(p));
                }
            }
        }
        h.finish()
    }

    /// FNV-1a hash of the surface layers around every edit in the log (the
    /// cells in each edit's bounding box): paint team, plus the stored expiry
    /// ticks of cells with surface states. Cheap: it only reads cells edits
    /// could have touched, so it needs no chunk generation. The ticks depend
    /// only on the log, not on the display tick, so peers agree at any time.
    pub fn paint_hash(&self) -> u64 {
        let store = self.store.borrow();
        let mut h = StateHasher::new();
        for e in &self.log {
            if !(e.center().is_finite() && e.radius().is_finite()) || e.radius() > 64.0 {
                continue;
            }
            let b = e.bounds();
            let lo = [b.min.x, b.min.y, b.min.z].map(|v| ls_math::det::floor(v) as i32);
            let hi = [b.max.x, b.max.y, b.max.z].map(|v| ls_math::det::floor(v) as i32);
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        h.write_u8(store.team_at_cell([x, y, z]));
                        let ticks = store.surface_state_ticks_at_cell([x, y, z]);
                        if ticks != [0; SurfaceStates::COUNT] {
                            h.write_u8(0x5e);
                            for t in ticks {
                                h.write_u32(t);
                            }
                        }
                    }
                }
            }
        }
        h.finish()
    }

    /// Hash of the terrain state for desync checks: edit log, SDF on a 4 u
    /// lattice and the surface layers (paint, surface states) around every edit.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u64(self.edit_log_hash());
        h.write_u64(self.sdf_hash(4.0));
        h.write_u64(self.paint_hash());
        h.finish()
    }
}

impl TerrainQuery for GameTerrain {
    fn sdf(&self, p: Vec3) -> f32 {
        self.store.borrow().eval(p)
    }

    fn bounds(&self) -> Aabb {
        self.bounds
    }
}

impl SimTerrain for GameTerrain {
    /// Applies and logs the edit verbatim (craters come already scaled by
    /// rock strength, see the type docs).
    fn apply_edit(&mut self, edit: &TerrainEdit) {
        self.apply(*edit);
    }

    fn surface_states_near(&self, center: Vec3, radius: f32, now: u64) -> SurfaceStates {
        GameTerrain::surface_states_near(self, center, radius, now)
    }

    fn material_at(&self, p: Vec3) -> u8 {
        GameTerrain::material_at(self, p).id()
    }

    /// Coverage for simulation teams 0 and 1, cached until the next edit.
    fn paint_coverage(&self) -> Option<ls_sim::PaintCoverage> {
        if let Some(c) = self.coverage.get() {
            return Some(c);
        }
        let full = self.full_paint_coverage();
        let cells = |t: u8| full.team_cells.get(&t).copied().unwrap_or(0);
        let c = ls_sim::PaintCoverage {
            team_cells: [cells(0), cells(1)],
            surface_cells: full.surface_cells,
        };
        self.coverage.set(Some(c));
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_worldgen::Sliders;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// A small level (2 x 2 x 2 chunks) so tests stay fast in debug builds.
    pub(crate) fn small_terrain(seed: u64) -> GameTerrain {
        GameTerrain::new(Generator::with_size(seed, Sliders::new(40, 30), [2, 2, 2]))
    }

    fn open_point(t: &GameTerrain) -> Vec3 {
        let c = t.bounds().center();
        t.find_open_point(c, 3.0, 20).expect("open point")
    }

    /// A surface point on generated rock (not on the sealing walls).
    fn rock_point(t: &GameTerrain) -> Vec3 {
        let o = open_point(t);
        let mut dirs = Vec::new();
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if (x, y, z) != (0, 0, 0) {
                        dirs.push(Vec3::new(x as f32, y as f32, z as f32));
                    }
                }
            }
        }
        dirs.iter()
            .filter_map(|d| t.raycast(o, *d, 200.0))
            .map(|h| h.point)
            .find(|p| t.with_store(|s| s.generator().room_open(*p)) > 2.0)
            .expect("rock surface near the centre")
    }

    #[test]
    fn crater_changes_sdf_marks_dirty_and_logs() {
        let mut t = small_terrain(7);
        let p = rock_point(&t);
        let before = t.sdf(p);
        let dirty = t.apply(TerrainEdit::SubtractSphere {
            center: p,
            radius: 2.0,
        });
        assert!(!dirty.is_empty());
        assert!(t.sdf(p) > before + 1.0);
        assert_eq!(t.edit_count(), 1);
        assert_eq!(t.take_dirty_chunks(), dirty);
        assert!(t.take_dirty_chunks().is_empty());
    }

    #[test]
    fn invalid_edits_still_take_a_sequence_number() {
        let mut t = small_terrain(7);
        t.apply(TerrainEdit::SubtractSphere {
            center: Vec3::splat(10.0),
            radius: -1.0,
        });
        assert_eq!(t.edit_count(), 1);
        assert_eq!(t.with_store(|s| s.edit_log().len()), 0);
    }

    #[test]
    fn apply_sequence_skips_known_and_detects_gaps() {
        let mut t = small_terrain(3);
        let e = |x: f32| TerrainEdit::SubtractSphere {
            center: Vec3::new(x, 20.0, 20.0),
            radius: 1.0,
        };
        assert_eq!(t.apply_sequence(0, &[e(10.0), e(11.0)]).unwrap(), 2);
        assert_eq!(t.apply_sequence(1, &[e(11.0), e(12.0)]).unwrap(), 1);
        assert_eq!(t.edit_count(), 3);
        assert!(matches!(
            t.apply_sequence(5, &[e(1.0)]),
            Err(GameError::EditGap { have: 3, base: 5 })
        ));
        let replayed = GameTerrain::replay(t.generator(), t.edit_log());
        assert_eq!(replayed.edit_log_hash(), t.edit_log_hash());
        assert_eq!(replayed.sdf_hash(4.0), t.sdf_hash(4.0));
    }

    #[test]
    fn sim_edits_apply_verbatim_and_materials_reach_the_sim() {
        let mut t = small_terrain(11);
        let p = rock_point(&t);
        let m = t.material_at(p);
        assert_eq!(SimTerrain::material_at(&t, p), m.id());
        assert_eq!(
            ls_math::terrain::material_crater_scale(m.id()),
            m.crater_scale()
        );
        // The simulation scales craters itself (`Ctx::crater`); the terrain
        // applies and logs what it is given.
        t.apply_edit(&TerrainEdit::SubtractSphere {
            center: p,
            radius: 2.0,
        });
        let logged = t.edit_log()[0];
        assert_eq!(logged.radius(), 2.0);
        assert_eq!(logged.center(), p);
        // Other edits pass through unchanged.
        let paint = TerrainEdit::Paint {
            center: p,
            radius: 3.0,
            team: 0,
        };
        t.apply_edit(&paint);
        assert_eq!(t.edit_log()[1], paint);
        // Replaying the log (late join, wire) gives the same terrain.
        let r = GameTerrain::replay(t.generator(), t.edit_log());
        assert_eq!(r.edit_log(), t.edit_log());
        assert_eq!(r.state_hash(), t.state_hash());
        // Every material scales differently somewhere in the level.
        let b = t.bounds();
        let mut seen = [false; 3];
        for i in 0..512 {
            let q = Vec3::new(
                b.min.x + (i * 37 % 64) as f32,
                b.min.y + (i * 11 % 64) as f32,
                b.min.z + (i * 23 % 64) as f32,
            );
            seen[t.material_at(q).id() as usize] = true;
        }
        assert_eq!(seen, [true; 3]);
    }

    #[test]
    fn surface_states_expire_mark_dirty_and_enter_the_state_hash() {
        let mut t = small_terrain(7);
        let p = rock_point(&t);
        let base = t.state_hash();
        let paint_only = t.paint_hash();
        t.apply_edit(&TerrainEdit::surface_for_default(
            p,
            2.5,
            SurfaceStates::CHARGED,
            60,
        ));
        assert_eq!(t.edit_count(), 1);
        let dirty = t.take_dirty_chunks();
        assert!(!dirty.is_empty());
        assert_ne!(t.paint_hash(), paint_only);
        assert_ne!(t.state_hash(), base);
        assert_eq!(t.surface_states_near(p, 1.0, 100), SurfaceStates::CHARGED);
        assert_eq!(t.surface_states_near(p, 1.0, 960), SurfaceStates::NONE);
        // Expiry: nothing before tick 960, then the same chunks again.
        t.advance_to(959);
        assert!(t.take_dirty_chunks().is_empty());
        let hash = t.state_hash();
        t.advance_to(960);
        assert_eq!(t.take_dirty_chunks(), dirty);
        assert_eq!(t.display_tick(), 960);
        // The display tick is not terrain state.
        assert_eq!(t.state_hash(), hash);
        // Wire round trip keeps the expiry tick.
        let wire: Vec<TerrainEditBits> = t.edit_log().iter().map(|e| (*e).into()).collect();
        let mut r = small_terrain(7);
        r.apply_wire_sequence(0, &wire).unwrap();
        assert_eq!(r.state_hash(), t.state_hash());
        assert_eq!(r.edit_log_hash(), t.edit_log_hash());
    }

    #[test]
    fn paint_coverage_through_sim_terrain() {
        let mut t = small_terrain(7);
        let p = rock_point(&t);
        let before = SimTerrain::paint_coverage(&t).unwrap();
        assert!(before.surface_cells > 0);
        assert_eq!(before.team_cells, [0, 0]);
        t.apply_edit(&TerrainEdit::Paint {
            center: p,
            radius: 2.0,
            team: 1,
        });
        let after = SimTerrain::paint_coverage(&t).unwrap();
        assert_eq!(after.team_cells[0], 0);
        assert!(after.team_cells[1] > 0);
    }
}
