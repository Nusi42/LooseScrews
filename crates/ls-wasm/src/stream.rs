//! Chunk streaming around the camera: which chunk to (re)mesh next and which
//! meshes to drop.
//!
//! The host calls [`ChunkStreamer::next`] repeatedly each frame until its
//! time budget is spent (see `web/src/game/chunks.ts`), so meshing never
//! stalls a frame for long. Priorities:
//!
//! 1. **Dirty** chunks that are currently shown (terrain edits changed them),
//!    nearest first: stale geometry is visible, so it goes first.
//! 2. Chunks within the view radius that have no mesh yet, nearest first.
//!
//! Chunks further than view radius + [`UNLOAD_MARGIN`] are unloaded (the
//! margin stops a chunk on the boundary from flickering in and out).

use std::collections::BTreeSet;

use ls_math::{Aabb, Vec3};
use ls_worldgen::ChunkCoord;

/// Extra distance (u) beyond the view radius before a mesh is dropped.
pub const UNLOAD_MARGIN: f32 = 32.0;
/// Default view radius (u); the whole default 256 x 128 x 256 level fits in
/// about 190 u from its centre.
pub const DEFAULT_VIEW_RADIUS: f32 = 224.0;

/// See the module docs.
#[derive(Debug, Clone)]
pub struct ChunkStreamer {
    all: Vec<ChunkCoord>,
    loaded: BTreeSet<ChunkCoord>,
    dirty: BTreeSet<ChunkCoord>,
    view_radius: f32,
}

/// Distance from `p` to the chunk's box (0 inside).
pub fn chunk_distance(c: ChunkCoord, p: Vec3) -> f32 {
    aabb_distance(c.aabb(), p)
}

fn aabb_distance(b: Aabb, p: Vec3) -> f32 {
    let d = |v: f32, lo: f32, hi: f32| (lo - v).max(0.0).max(v - hi);
    let (dx, dy, dz) = (
        d(p.x, b.min.x, b.max.x),
        d(p.y, b.min.y, b.max.y),
        d(p.z, b.min.z, b.max.z),
    );
    (dx * dx + dy * dy + dz * dz).sqrt()
}

impl ChunkStreamer {
    /// A streamer over the level's chunk coordinates (`ChunkStore::chunk_coords`).
    pub fn new(all: Vec<ChunkCoord>) -> Self {
        ChunkStreamer {
            all,
            loaded: BTreeSet::new(),
            dirty: BTreeSet::new(),
            view_radius: DEFAULT_VIEW_RADIUS,
        }
    }

    pub fn view_radius(&self) -> f32 {
        self.view_radius
    }

    pub fn set_view_radius(&mut self, r: f32) {
        if r.is_finite() && r > 0.0 {
            self.view_radius = r;
        }
    }

    /// Records chunks changed by terrain edits. Chunks without a mesh are
    /// ignored: they will be meshed from the current terrain when they load.
    pub fn mark_dirty(&mut self, chunks: impl IntoIterator<Item = ChunkCoord>) {
        for c in chunks {
            if self.loaded.contains(&c) {
                self.dirty.insert(c);
            }
        }
    }

    pub fn is_loaded(&self, c: ChunkCoord) -> bool {
        self.loaded.contains(&c)
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded.len()
    }

    pub fn dirty_count(&self) -> usize {
        self.dirty.len()
    }

    /// Chunks still to mesh for a camera at `cam` (dirty plus missing in range).
    pub fn pending(&self, cam: Vec3) -> usize {
        self.dirty.len()
            + self
                .all
                .iter()
                .filter(|c| {
                    !self.loaded.contains(c) && chunk_distance(**c, cam) <= self.view_radius
                })
                .count()
    }

    /// The next chunk to mesh, marked as loaded and clean. The caller must
    /// mesh it and replace whatever it shows for that chunk.
    pub fn next(&mut self, cam: Vec3) -> Option<ChunkCoord> {
        let nearest = |it: &mut dyn Iterator<Item = ChunkCoord>| {
            it.map(|c| (chunk_distance(c, cam), c))
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, c)| c)
        };
        if let Some(c) = nearest(&mut self.dirty.iter().copied()) {
            self.dirty.remove(&c);
            return Some(c);
        }
        let r = self.view_radius;
        let c = nearest(
            &mut self
                .all
                .iter()
                .copied()
                .filter(|c| !self.loaded.contains(c) && chunk_distance(*c, cam) <= r),
        )?;
        self.loaded.insert(c);
        Some(c)
    }

    /// Unloads chunks beyond the view radius plus margin and returns them
    /// (the host drops their meshes).
    pub fn take_unloads(&mut self, cam: Vec3) -> Vec<ChunkCoord> {
        let limit = self.view_radius + UNLOAD_MARGIN;
        let gone: Vec<ChunkCoord> = self
            .loaded
            .iter()
            .copied()
            .filter(|c| chunk_distance(*c, cam) > limit)
            .collect();
        for c in &gone {
            self.loaded.remove(c);
            self.dirty.remove(c);
        }
        gone
    }

    /// Chunks the host shows meshes of.
    pub fn loaded(&self) -> Vec<ChunkCoord> {
        self.loaded.iter().copied().collect()
    }

    /// Forgets every mesh (e.g. after the host lost its GPU buffers).
    pub fn reset(&mut self) {
        self.loaded.clear();
        self.dirty.clear();
    }
}

/// Key the web client uses for a chunk (`"x,y,z"`).
pub fn chunk_key(c: ChunkCoord) -> String {
    format!("{},{},{}", c.x, c.y, c.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: [i32; 3]) -> Vec<ChunkCoord> {
        let mut v = Vec::new();
        for z in 0..n[2] {
            for y in 0..n[1] {
                for x in 0..n[0] {
                    v.push(ChunkCoord::new(x, y, z));
                }
            }
        }
        v
    }

    #[test]
    fn streams_nearest_first_within_the_radius() {
        let mut s = ChunkStreamer::new(grid([8, 4, 8]));
        s.set_view_radius(40.0);
        let cam = Vec3::new(16.0, 16.0, 16.0); // inside chunk (0,0,0)
        assert_eq!(s.next(cam), Some(ChunkCoord::new(0, 0, 0)));
        let mut last = 0.0;
        let mut n = 1;
        while let Some(c) = s.next(cam) {
            let d = chunk_distance(c, cam);
            assert!(d >= last && d <= 40.0, "{c:?} at {d}");
            last = d;
            n += 1;
        }
        // Chunks whose box is within 40 u of a point 16 u inside the corner chunk.
        assert_eq!(n, 8);
        assert_eq!(s.pending(cam), 0);
    }

    #[test]
    fn dirty_loaded_chunks_come_first_and_unloaded_ones_are_ignored() {
        let mut s = ChunkStreamer::new(grid([4, 1, 1]));
        s.set_view_radius(1.0);
        let cam = Vec3::new(1.0, 1.0, 1.0);
        assert_eq!(s.next(cam), Some(ChunkCoord::new(0, 0, 0)));
        assert_eq!(s.next(cam), None);
        s.mark_dirty([ChunkCoord::new(0, 0, 0), ChunkCoord::new(3, 0, 0)]);
        assert_eq!(s.dirty_count(), 1, "chunk (3,0,0) has no mesh yet");
        assert_eq!(s.pending(cam), 1);
        assert_eq!(s.next(cam), Some(ChunkCoord::new(0, 0, 0)));
        assert_eq!(s.next(cam), None);
    }

    #[test]
    fn far_chunks_unload_with_hysteresis() {
        let mut s = ChunkStreamer::new(grid([8, 1, 1]));
        s.set_view_radius(10.0);
        let start = Vec3::new(1.0, 1.0, 1.0);
        while s.next(start).is_some() {}
        assert_eq!(s.loaded_count(), 1);
        // Moving just past the radius keeps the mesh (margin).
        let near = Vec3::new(32.0 + 20.0, 1.0, 1.0);
        assert!(s.take_unloads(near).is_empty());
        let far = Vec3::new(200.0, 1.0, 1.0);
        assert_eq!(s.take_unloads(far), vec![ChunkCoord::new(0, 0, 0)]);
        assert!(!s.is_loaded(ChunkCoord::new(0, 0, 0)));
        assert_eq!(chunk_key(ChunkCoord::new(1, -2, 3)), "1,-2,3");
    }
}
