//! Moving day (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! Every pickup lying in the level (weapons, upgrades, gadgets; not scrap)
//! moves to a new place: the pickup points are permuted in one cycle
//! (seeded; every point maps to another one) and the content of point `i`
//! moves to point `sigma(i)` (`Command::MovePickup`). The content still
//! belongs to its own spawn point, so spawn timers, per-map caps and the
//! respawn place are kept: once taken, the point respawns where it always
//! did. During the 2 s move (the active phase) no pickup can be taken
//! (`allows_pickup`); the client packs them into boxes that arc to their new
//! places. Needs at least `min_pickups` (4) pickups in play.
//!
//! Numbers: `data/events/moving_day.ron`.

use crate::events::{EffectCtx, EventCtx, EventKind, EventModule, EventRef};
use crate::PickupKind;

/// Moving day.
#[derive(Debug)]
pub struct MovingDay;

/// A seeded cyclic permutation of `0..n` (Sattolo's algorithm: one cycle,
/// so no point maps to itself when `n >= 2`).
pub fn cycle(n: usize, rng: &mut ls_math::Pcg32) -> Vec<usize> {
    let mut p: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.below(i as u32) as usize;
        p.swap(i, j);
    }
    p
}

impl EventModule for MovingDay {
    fn kind(&self) -> EventKind {
        EventKind::MovingDay
    }

    fn implemented(&self) -> bool {
        true
    }

    /// Needs at least `min_pickups` (4) pickups in play.
    fn eligible(&self, e: &EventRef) -> bool {
        crate::events::pickups_in_play(e.view) as f32 >= e.def.param("min_pickups", 4.0)
    }

    fn on_start(&self, e: &mut EventCtx) {
        let points = e.ctx.view.pickups;
        if points.len() < 2 {
            return;
        }
        let mut rng = e.rng(1);
        let to = cycle(points.len(), &mut rng);
        let moves: Vec<(u32, ls_math::Vec3)> = points
            .iter()
            .enumerate()
            .filter_map(|(i, sp)| {
                let (_, kind) = sp.content?;
                if matches!(kind, PickupKind::Scrap { .. }) {
                    return None;
                }
                Some((sp.id, points[to[i]].pos - sp.content_pos()))
            })
            .collect();
        e.event.vars[0] = moves.len() as i64;
        for (id, delta) in moves {
            e.ctx.move_pickup(id, delta);
        }
    }

    /// Nothing can be taken while the boxes are on their way.
    fn allows_pickup(&self, _fx: &EffectCtx, _spawn: u16) -> bool {
        false
    }
}
