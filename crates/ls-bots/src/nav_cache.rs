//! Process-wide cache of built navigation grids, one per world.
//!
//! Building a [`NavGrid`] for a whole level costs 2-3 s of CPU in release
//! builds. Every bot of a match flies the same level, so the bots of one
//! process share one build: the first bot that joins a world starts the
//! build on a worker thread (off the bot loop), later bots of the same world
//! attach to it ([`SharedNav`]). Each bot copies the finished grid into its
//! own [`Navigator`](crate::nav::Navigator) and applies the terrain edits it
//! has seen since the match started (the cached grid is built from the
//! pristine level), so craters and blocked links stay per bot.
//!
//! The cache key is the world's share code, the generator version and the
//! [`NavConfig`]. Worlds with a live [`SharedNav`] handle always stay cached;
//! beyond that the most recent [`KEEP`] worlds do (a bot process that plays
//! several matches on one world builds its grid once).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_worldgen::{ShareCode, GENERATOR_VERSION};
use ls_game::GameTerrain;

use crate::nav::{NavConfig, NavGrid};

/// Worlds kept in the cache.
pub const KEEP: usize = 4;

/// Identifies one grid build: the world and the grid tuning.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavKey {
    world: String,
    generator: u8,
    cfg: [u64; 6],
}

impl NavKey {
    /// A key for any world name (tests use synthetic names).
    pub fn new(world: impl Into<String>, cfg: &NavConfig) -> NavKey {
        NavKey {
            world: world.into(),
            generator: GENERATOR_VERSION,
            cfg: [
                cfg.cell.to_bits() as u64,
                cfg.clearance.to_bits() as u64,
                cfg.edge_clearance.to_bits() as u64,
                cfg.build_budget as u64,
                cfg.max_expansions as u64,
                cfg.heuristic_weight.to_bits() as u64,
            ],
        }
    }

    /// The key of a share-code world (the code carries the generator
    /// version; the key repeats it so a code from an older generator never
    /// matches a grid built by a newer one).
    pub fn for_code(code: &ShareCode, cfg: &NavConfig) -> NavKey {
        NavKey::new(code.to_string(), cfg)
    }
}

#[derive(Debug)]
enum State {
    Building,
    Ready(Arc<NavGrid>),
    /// The build panicked; bots fall back to building their own grid.
    Failed,
}

#[derive(Debug)]
struct Slot {
    state: Mutex<State>,
    done: Condvar,
}

impl Slot {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish(&self, state: State) {
        *self.lock() = state;
        self.done.notify_all();
    }
}

/// What a [`SharedNav`] has to offer right now.
#[derive(Debug, Clone)]
pub enum NavStatus {
    /// The worker thread is still building.
    Building,
    Ready(Arc<NavGrid>),
    /// The build failed; build a grid locally.
    Failed,
}

/// A handle to a shared grid build (cheap to clone).
#[derive(Debug, Clone)]
pub struct SharedNav {
    slot: Arc<Slot>,
}

impl SharedNav {
    /// The build's state; never blocks for long (one uncontended lock).
    pub fn status(&self) -> NavStatus {
        match &*self.slot.lock() {
            State::Building => NavStatus::Building,
            State::Ready(g) => NavStatus::Ready(g.clone()),
            State::Failed => NavStatus::Failed,
        }
    }

    /// Blocks until the build is done or `timeout` passed (tests, tools).
    pub fn wait(&self, timeout: Duration) -> NavStatus {
        let deadline = Instant::now() + timeout;
        let mut state = self.slot.lock();
        while matches!(*state, State::Building) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return NavStatus::Building;
            }
            state = self
                .slot
                .done
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        drop(state);
        self.status()
    }

    /// Whether both handles refer to the same build.
    pub fn same_build(&self, other: &SharedNav) -> bool {
        Arc::ptr_eq(&self.slot, &other.slot)
    }
}

struct Cache {
    /// Most recently used last.
    entries: VecDeque<(NavKey, Arc<Slot>)>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    entries: VecDeque::new(),
});

/// Grid builds started by this process (cache misses).
static BUILDS: AtomicU64 = AtomicU64::new(0);

/// Number of grid builds this process has started (diagnostics, tests).
pub fn builds() -> u64 {
    BUILDS.load(Ordering::Relaxed)
}

/// The shared grid for `key`: an existing build (finished or still
/// running) or a new one, run by `build` on a worker thread.
pub fn acquire<F>(key: NavKey, build: F) -> SharedNav
where
    F: FnOnce() -> NavGrid + Send + 'static,
{
    let slot = {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(k) = cache.entries.iter().position(|(k, _)| *k == key) {
            let hit = cache.entries.remove(k).expect("index in range");
            let slot = hit.1.clone();
            cache.entries.push_back(hit);
            return SharedNav { slot };
        }
        let slot = Arc::new(Slot {
            state: Mutex::new(State::Building),
            done: Condvar::new(),
        });
        cache.entries.push_back((key, slot.clone()));
        // Drop the least recently used worlds nobody holds a handle to.
        while cache.entries.len() > KEEP {
            let Some(k) = cache
                .entries
                .iter()
                .position(|(_, s)| Arc::strong_count(s) == 1)
            else {
                break;
            };
            cache.entries.remove(k);
        }
        slot
    };
    BUILDS.fetch_add(1, Ordering::Relaxed);
    let worker = slot.clone();
    let run = move || {
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(build));
        worker.finish(match built {
            Ok(grid) => State::Ready(Arc::new(grid)),
            Err(_) => State::Failed,
        });
    };
    // The closure is moved into the thread; if spawning fails it is gone,
    // so mark the build failed (bots then build their own grids).
    if thread::Builder::new()
        .name("nav-build".into())
        .spawn(run)
        .is_err()
    {
        slot.finish(State::Failed);
    }
    SharedNav { slot }
}

/// The shared grid of a share-code world. `pristine` is the untouched level
/// (callers usually have one at hand from the match setup); it is moved to
/// the worker thread on a cache miss and dropped on a hit.
pub fn for_level(code: &ShareCode, cfg: NavConfig, pristine: GameTerrain) -> SharedNav {
    acquire(NavKey::for_code(code, &cfg), move || {
        build_grid(&pristine, cfg)
    })
}

/// Builds a whole grid for `terrain` now.
pub fn build_grid(terrain: &GameTerrain, cfg: NavConfig) -> NavGrid {
    let mut grid = NavGrid::new(terrain.bounds(), cfg);
    grid.build_all(terrain);
    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::ls_math::Aabb;
    use ls_game::ls_math::Vec3;
    use std::sync::atomic::AtomicU32;

    fn tiny() -> NavGrid {
        let b = Aabb::new(Vec3::ZERO, Vec3::splat(4.0));
        NavGrid::new(b, NavConfig::default())
    }

    #[test]
    fn second_acquire_reuses_the_build() {
        static RUNS: AtomicU32 = AtomicU32::new(0);
        let key = NavKey::new("cache-test-reuse", &NavConfig::default());
        let build = || {
            RUNS.fetch_add(1, Ordering::SeqCst);
            tiny()
        };
        let a = acquire(key.clone(), build);
        let b = acquire(key.clone(), build);
        assert!(a.same_build(&b));
        let (NavStatus::Ready(ga), NavStatus::Ready(gb)) = (
            a.wait(Duration::from_secs(10)),
            b.wait(Duration::from_secs(10)),
        ) else {
            panic!("build did not finish");
        };
        assert!(Arc::ptr_eq(&ga, &gb));
        // A third bot after the build finished attaches too.
        let c = acquire(key, build);
        assert!(c.same_build(&a));
        assert_eq!(RUNS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn other_config_or_world_builds_again() {
        let cfg = NavConfig::default();
        let other = NavConfig { cell: 3.0, ..cfg };
        let a = acquire(NavKey::new("cache-test-cfg", &cfg), tiny);
        let b = acquire(NavKey::new("cache-test-cfg", &other), tiny);
        let c = acquire(NavKey::new("cache-test-cfg-2", &cfg), tiny);
        assert!(!a.same_build(&b));
        assert!(!a.same_build(&c));
    }

    #[test]
    fn a_panicking_build_reports_failure() {
        let a = acquire(
            NavKey::new("cache-test-panic", &NavConfig::default()),
            || panic!("expected test panic"),
        );
        assert!(matches!(a.wait(Duration::from_secs(10)), NavStatus::Failed));
    }
}
