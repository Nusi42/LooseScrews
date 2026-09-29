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
///   and marks chunks whose states expired dirty. Scorch is permanent.
/// - Every edit handed to it is logged, including ones the chunk store rejects
///   as invalid, so the log index is the edit's **sequence number** on the wire
///   (`Snapshot::edit_base`, `Welcome::edit_count`) on every peer. A late
///   joiner's log lacks the expired surface-state edits the server left out
///   of its `Welcome` ([`GameTerrain::welcome_log`]); it counts them
///   ([`GameTerrain::edit_count`]) but cannot serve its log to others.
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
    /// Sequence numbers without an edit in `log` (left out of the Welcome).
    pruned: u32,
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
            pruned: 0,
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

    /// A level with `edits` applied in order.
    pub fn replay(gen: Generator, edits: &[TerrainEdit]) -> Self {
        let mut t = Self::new(gen);
        for e in edits {
            t.apply(*e);
        }
        t
    }

    /// A late joiner's level from `Welcome::edit_log` and
    /// `Welcome::edit_count` (the log may leave out expired surface-state
    /// edits, see [`GameTerrain::welcome_log`]; `edit_count` below the log
    /// length counts the log).
    pub fn replay_welcome(gen: Generator, edits: &[TerrainEdit], edit_count: u32) -> Self {
        let mut t = Self::replay(gen, edits);
        t.pruned = edit_count.saturating_sub(t.log.len() as u32);
        t
    }

    /// The edit log for a `Welcome` sent at tick `now`, and the number of
    /// edits so far (the sequence number of the next edit).
    ///
    /// It leaves out what expired before `now` and can no longer matter:
    /// the timed part (hot, charged, glow) of a surface-state edit whose
    /// `until <= now` (a surface edit with scorch keeps its scorch bit), as
    /// long as no earlier edit still active at `now` (`until > now`) shares
    /// one of its state bits and may share a cell (spheres closer than the
    /// sum of their radii): such an edit may have cut a running state short,
    /// and dropping it would bring the state back. Replaying the result
    /// gives the same geometry, paint and scorch, and the same timed states
    /// at every tick from `now` on, as the full log; the time-independent
    /// [`GameTerrain::state_hash`] and [`GameTerrain::state_hash_at`] any
    /// tick `>= now` agree between the server, earlier clients and the late
    /// joiner.
    pub fn welcome_log(&self, now: u64) -> (Vec<TerrainEdit>, u32) {
        let now = now.min(u32::MAX as u64) as u32;
        let timed = SurfaceStates::TIMED.bits();
        // Earlier edits with timed states still running at `now`.
        let mut active: Vec<(Vec3, f32, u8)> = Vec::new();
        let mut out = Vec::with_capacity(self.log.len());
        for e in &self.log {
            let TerrainEdit::Surface {
                center,
                radius,
                states,
                until,
            } = *e
            else {
                out.push(*e);
                continue;
            };
            let bits = states & timed;
            if bits == 0 || until > now {
                if bits != 0 {
                    active.push((center, radius, bits));
                }
                out.push(*e);
                continue;
            }
            let blocked = active.iter().any(|&(c, r, b)| {
                b & bits != 0 && c.distance(center) <= r + radius + CELL_OVERLAP_EPS
            });
            if blocked {
                out.push(*e);
            } else if let Some(kept) = e.without_expired_states(now) {
                out.push(kept);
            }
        }
        (out, self.edit_count())
    }

    pub fn generator(&self) -> Generator {
        self.store.borrow().generator().clone()
    }

    pub fn share_code(&self) -> ShareCode {
        self.store.borrow().generator().code()
    }

    /// Number of edits so far = sequence number of the next edit (on a late
    /// joiner including the ones its Welcome left out).
    pub fn edit_count(&self) -> u32 {
        self.log.len() as u32 + self.pruned
    }

    /// Every edit applied so far, in order (a late joiner's lacks the edits
    /// its Welcome left out: only a log with [`GameTerrain::pruned_count`]
    /// 0 is indexed by sequence number).
    pub fn edit_log(&self) -> &[TerrainEdit] {
        &self.log
    }

    /// Sequence numbers this terrain counts without having their edits
    /// (left out of its Welcome).
    pub fn pruned_count(&self) -> u32 {
        self.pruned
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

    /// Surface states active at `tick` on the surface cells (1 u cubes) that
    /// overlap the sphere of `radius` around `center` (e.g. a charged wall
    /// under a scraping ship). Overlap, not the cell centre, decides: a ship
    /// touching a wall is within a fraction of a unit of the surface but up
    /// to a cell diagonal away from the centre of the cell holding its state.
    pub fn surface_states_near(&self, center: Vec3, radius: f32, tick: u64) -> SurfaceStates {
        let now = tick.min(u32::MAX as u64) as u32;
        self.store
            .borrow()
            .surface_states_overlapping(center, radius, now)
    }

    /// Paint colours (`1 << PAINT_*`) of the painted surface cells that
    /// overlap the sphere (ship physics: bouncy, slippery, sticky paint).
    pub fn paint_colors_near(&self, center: Vec3, radius: f32) -> u8 {
        self.store.borrow().paint_colors_near(center, radius)
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

    /// FNV-1a hash of the edit count and the lasting part of the edit log
    /// (raw `f32` bits): every geometry and paint edit, and the scorch part
    /// of surface edits. Timed surface states are left out (the Welcome of a
    /// late joiner may leave them out; [`GameTerrain::state_hash_at`] checks
    /// the states themselves), so peers agree at any time.
    pub fn edit_log_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u64(self.edit_count() as u64);
        for e in &self.log {
            let mut b = TerrainEditBits::from(*e);
            if b.kind == 3 {
                b.team &= SurfaceStates::SCORCH.bits();
                if b.team == 0 {
                    continue;
                }
                b.until = (b.until != 0) as u32;
            }
            h.write_u8(b.kind);
            for c in b.center {
                h.write_u32(c);
            }
            h.write_u32(b.radius);
            h.write_u8(b.team);
            h.write_u8(b.color);
            h.write_u32(b.until);
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

    /// FNV-1a hash of the lasting surface layers: every painted or scorched
    /// surface cell with its paint team and colour and whether it is
    /// scorched. Reads the cells that hold layers (no chunk generation).
    pub fn paint_hash(&self) -> u64 {
        let store = self.store.borrow();
        let mut h = StateHasher::new();
        store.for_each_layer_cell(|g, paint, _, scorch| {
            if paint.is_none() && !scorch {
                return;
            }
            for c in g {
                h.write_u32(c as u32);
            }
            let (team, color) = paint.unwrap_or((ls_worldgen::NO_TEAM, 0));
            h.write_u8(team);
            h.write_u8(color);
            h.write_bool(scorch);
        });
        h.finish()
    }

    /// FNV-1a hash of the timed surface states (hot, charged, glow) running
    /// at `tick`: every cell with such a state, and the expiry tick of each
    /// running state (0 for the others).
    pub fn surface_state_hash_at(&self, tick: u64) -> u64 {
        let now = tick.min(u32::MAX as u64) as u32;
        let store = self.store.borrow();
        let mut h = StateHasher::new();
        store.for_each_layer_cell(|g, _, ticks, _| {
            let running = ticks.map(|u| if u > now { u } else { 0 });
            if running == [0; SurfaceStates::COUNT] {
                return;
            }
            for c in g {
                h.write_u32(c as u32);
            }
            for u in running {
                h.write_u32(u);
            }
        });
        h.finish()
    }

    /// Hash of the terrain state for desync checks: edit count and lasting
    /// edits, SDF on a 4 u lattice and the lasting surface layers (paint,
    /// scorch). Independent of time, so peers compare it at any moment; it
    /// leaves out the timed surface states (see
    /// [`GameTerrain::state_hash_at`]).
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u64(self.edit_log_hash());
        h.write_u64(self.sdf_hash(4.0));
        h.write_u64(self.paint_hash());
        h.finish()
    }

    /// [`GameTerrain::state_hash`] plus the timed surface states running at
    /// `tick` ([`GameTerrain::surface_state_hash_at`]). Peers agree for
    /// every tick from the one they joined at on.
    pub fn state_hash_at(&self, tick: u64) -> u64 {
        let mut h = StateHasher::new();
        h.write_u64(self.state_hash());
        h.write_u64(self.surface_state_hash_at(tick));
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

    fn paint_colors_near(&self, center: Vec3, radius: f32) -> u8 {
        GameTerrain::paint_colors_near(self, center, radius)
    }
}

/// Margin on the sphere-overlap test of [`GameTerrain::welcome_log`]
/// (conservative: a larger margin only keeps more edits).
const CELL_OVERLAP_EPS: f32 = 0.01;

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
            color: 0,
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
        let base_at = t.state_hash_at(100);
        t.apply_edit(&TerrainEdit::surface_for_default(
            p,
            2.5,
            SurfaceStates::CHARGED,
            60,
        ));
        assert_eq!(t.edit_count(), 1);
        let dirty = t.take_dirty_chunks();
        assert!(!dirty.is_empty());
        // Timed states count in the hash at a tick while they run; the
        // time-independent hash only sees the edit count.
        assert_ne!(t.state_hash_at(100), base_at);
        assert_ne!(t.state_hash(), base);
        assert_eq!(
            t.surface_state_hash_at(960),
            small_terrain(7).surface_state_hash_at(960)
        );
        assert_eq!(t.surface_states_near(p, 1.0, 100), SurfaceStates::CHARGED);
        assert_eq!(t.surface_states_near(p, 1.0, 960), SurfaceStates::NONE);
        // Expiry: nothing before tick 960, then the same chunks again.
        t.advance_to(959);
        assert!(t.take_dirty_chunks().is_empty());
        let hash = t.state_hash_at(500);
        t.advance_to(960);
        assert_eq!(t.take_dirty_chunks(), dirty);
        assert_eq!(t.display_tick(), 960);
        // The display tick is not terrain state.
        assert_eq!(t.state_hash_at(500), hash);
        // Wire round trip keeps the expiry tick.
        let wire: Vec<TerrainEditBits> = t.edit_log().iter().map(|e| (*e).into()).collect();
        let mut r = small_terrain(7);
        r.apply_wire_sequence(0, &wire).unwrap();
        assert_eq!(r.state_hash_at(500), t.state_hash_at(500));
        assert_eq!(r.edit_log_hash(), t.edit_log_hash());
    }

    /// Late joiners get a Welcome log without the surface-state edits that
    /// expired and cannot matter any more; they end up with the same
    /// terrain as clients that saw every edit (hashes, states, sequence
    /// numbers), also for later edits.
    #[test]
    fn welcome_log_prunes_expired_surface_edits_without_changing_the_terrain() {
        let mut t = small_terrain(7);
        let p = rock_point(&t);
        // Scorch on the same rock as the rest (layers share cells).
        let q = p;
        let far = p + Vec3::new(0.0, 0.0, 20.0);
        let surface = |c, states: SurfaceStates, until| TerrainEdit::Surface {
            center: c,
            radius: 2.0,
            states: states.bits(),
            until,
        };
        let hot = SurfaceStates::HOT;
        let glow = SurfaceStates::GLOW;
        let edits = [
            // 0: hot until 100 (expires): pruned.
            surface(p, hot, 100),
            // 1: crater: kept.
            TerrainEdit::SubtractSphere {
                center: p + Vec3::new(0.0, 0.0, -15.0),
                radius: 1.5,
            },
            // 2: scorch, hot until 200: its scorch stays.
            TerrainEdit::scorch(q, 2.0, 100, 100),
            // 3: glow until 5000 (still running) ...
            surface(far, glow, 5000),
            // 4: ... cut short to 300 later: kept (dropping it would bring
            // the glow back).
            surface(far, glow, 300),
            // 5: hot until 400 near the glow: pruned (other state).
            surface(far, hot, 400),
            // 6: paint: kept.
            TerrainEdit::Paint {
                center: p,
                radius: 3.0,
                team: 1,
                color: ls_math::terrain::PAINT_YELLOW,
            },
            // 7: charged until 2000 (still running at 1000): kept.
            surface(p, SurfaceStates::CHARGED, 2000),
            // 8: a clear of hot (until 0) with nothing running: pruned.
            surface(q, hot, 0),
        ];
        for e in edits {
            t.apply_edit(&e);
        }
        let now = 1000;
        let (log, count) = t.welcome_log(now);
        assert_eq!(count, 9);
        let scorch_only = TerrainEdit::Surface {
            center: q,
            radius: 2.0,
            states: SurfaceStates::SCORCH.bits(),
            until: 200,
        };
        assert_eq!(
            log,
            [
                edits[1],
                scorch_only,
                edits[3],
                edits[4],
                edits[6],
                edits[7]
            ]
        );
        let mut late = GameTerrain::replay_welcome(t.generator(), &log, count);
        assert_eq!(late.edit_count(), 9);
        assert_eq!(late.pruned_count(), 3);
        for tick in [now, now + 1, 1999, 2000, 6000] {
            assert_eq!(
                late.state_hash_at(tick),
                t.state_hash_at(tick),
                "tick {tick}"
            );
        }
        assert_eq!(late.state_hash(), t.state_hash());
        for c in [p, q, far] {
            assert_eq!(
                late.surface_states_near(c, 2.0, now),
                t.surface_states_near(c, 2.0, now)
            );
        }
        assert!(t
            .surface_states_near(q, 1.0, now)
            .contains(SurfaceStates::SCORCH));
        assert_eq!(late.surface_states_near(far, 1.0, now), SurfaceStates::NONE);
        // Later edits arrive with their sequence numbers on both.
        let next = TerrainEdit::surface_for_default(p, 1.0, glow, now);
        assert_eq!(late.apply_sequence(9, &[next]).unwrap(), 1);
        t.apply_edit(&next);
        assert_eq!(late.edit_count(), t.edit_count());
        assert_eq!(late.state_hash_at(now + 5), t.state_hash_at(now + 5));
        assert!(late.apply_sequence(12, &[next]).is_err(), "gap");
        // A second late joiner, pruned against the first one's log view,
        // agrees too; nothing expired means nothing pruned.
        let (fresh, n) = t.welcome_log(0);
        assert_eq!(fresh.len() as u32, n);
        assert_eq!(fresh, t.edit_log());
    }

    /// M2 plan: a charged surface (Leech Arc) drains 5 energy/s from a ship
    /// scraping it. The simulation reads the states through
    /// `SimTerrain::surface_states_near`, which this adapter answers from the
    /// chunk store; a ship in open space is not drained, and the drain ends
    /// with the charge.
    #[test]
    fn charged_rock_drains_ships_scraping_it_through_the_adapter() {
        use ls_math::Quat;
        use ls_sim::{InputCmd, MatchSetup, ShipTuning, ShipType, SpawnPoint, World, WorldConfig};

        let mut t = small_terrain(7);
        let p = rock_point(&t);
        let n = ls_math::terrain::sdf_gradient(|q| t.sdf(q), p);
        let open = open_point(&t);
        // The trait answers like the store, at the tick it is asked for.
        let until = 120u64;
        t.apply_edit(&TerrainEdit::Surface {
            center: p,
            radius: 3.0,
            states: SurfaceStates::CHARGED.bits(),
            until: until as u32,
        });
        assert!(SimTerrain::surface_states_near(&t, p, 1.0, 10).contains(SurfaceStates::CHARGED));
        assert_eq!(
            SimTerrain::surface_states_near(&t, p, 1.0, until),
            SurfaceStates::NONE
        );
        assert_eq!(
            SimTerrain::surface_states_near(&t, open, 1.0, 10),
            SurfaceStates::NONE
        );

        let spawn = |pos, team| SpawnPoint {
            pos,
            team,
            orientation: Quat::IDENTITY,
        };
        let setup = MatchSetup {
            spawns: vec![spawn(open, 0), spawn(open, 1)],
            pickup_points: vec![],
            instability: 0,
        };
        let mut w = World::new(WorldConfig::paint_the_void(5, setup)).unwrap();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let radius = ShipTuning::tinker().radius;
        {
            // a presses its nose into the charged wall; b floats in the open.
            let s = w.ship_mut(a).unwrap();
            s.pos = p + n * (radius + 0.05);
            s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, -n);
            s.energy = 50.0;
            let s = w.ship_mut(b).unwrap();
            s.pos = open;
            s.energy = 50.0;
        }
        let into_wall = InputCmd::idle(0).with_thrust(Vec3::NEG_Z);
        let inputs = [(a, into_wall), (b, InputCmd::idle(0))];
        for _ in 0..60 {
            w.step(&mut t, &inputs);
        }
        let (ea, eb) = (w.ship(a).unwrap().energy, w.ship(b).unwrap().energy);
        assert!(w.ship(a).unwrap().in_contact, "scraping the wall");
        assert!((eb - 60.0).abs() < 0.05, "regen only: {eb}");
        assert!((ea - 55.0).abs() < 0.2, "regen minus 5/s: {ea}");
        // The charge expires at tick 120: from then on both regenerate alike.
        for _ in 0..60 {
            w.step(&mut t, &inputs);
        }
        let (ea, eb) = (w.ship(a).unwrap().energy, w.ship(b).unwrap().energy);
        for _ in 0..30 {
            w.step(&mut t, &inputs);
        }
        let (ea2, eb2) = (w.ship(a).unwrap().energy, w.ship(b).unwrap().energy);
        assert!((ea2 - ea - 5.0).abs() < 0.05, "{ea} -> {ea2}");
        assert!((eb2 - eb - 5.0).abs() < 0.05, "{eb} -> {eb2}");
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
            color: 0,
        });
        let after = SimTerrain::paint_coverage(&t).unwrap();
        assert_eq!(after.team_cells[0], 0);
        assert!(after.team_cells[1] > 0);
    }
}
