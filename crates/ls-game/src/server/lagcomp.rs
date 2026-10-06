//! Lag compensation: a short history of ship positions and the per-tick
//! "rewind" pass that lets projectiles hit targets where their shooter saw them.
//!
//! A client renders remote ships about `RTT + interpolation delay` in the
//! past. The room estimates that delay per input (`input tick - (acked
//! snapshot tick - interpolation delay)`, capped at 250 ms) and remembers it
//! for every projectile the input fires. Before each simulation step,
//! [`plan_compensation`] tests each projectile's segment for the coming tick
//! against the targets' positions rewound by the shooter's delay. When the
//! rewound target is hit but the simulation would not register it, the target
//! is displaced for that one step so that after its own movement it sits
//! exactly at the rewound position; the simulation then resolves the hit with
//! its normal rules (damage, knockback, kill credit, events) and the room
//! moves the target back afterwards.
//!
//! `ls-sim` has no hook for per-projectile target positions, hence the
//! displacement. Its side effects are kept small by refusing it when the
//! target fires or scavenges this tick, when a pickup is within reach of the
//! rewound position, or when the target's movement would differ there
//! (terrain contact). Splash from other explosions in the same tick sees the
//! displaced position.

use std::collections::{BTreeMap, VecDeque};

use ls_math::terrain::TerrainQuery;
use ls_math::{Vec3, DT};
use ls_sim::terrain::segment_sphere;
use ls_sim::world::PICKUP_RADIUS;
use ls_sim::{simulate_movement, Buttons, InputCmd, PlayerId, World};

use crate::GameTerrain;

/// Positions of every ship after one tick.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryFrame {
    pub tick: u64,
    /// `(player, position, alive)`, sorted by player.
    pub ships: Vec<(PlayerId, Vec3, bool)>,
    /// The comedy event's hit sphere scale and hit plane during that tick
    /// (Big head day, Wrong genre): a rewound target is tested as it was
    /// then.
    pub hit_scale: f32,
    pub hit_plane: Option<Vec3>,
}

/// Ring buffer of [`HistoryFrame`]s covering the lag compensation window.
#[derive(Debug, Clone)]
pub struct ShipHistory {
    capacity: usize,
    frames: VecDeque<HistoryFrame>,
}

impl ShipHistory {
    /// Keeps the last `ticks` ticks (plus one).
    pub fn new(ticks: u64) -> Self {
        ShipHistory {
            capacity: ticks as usize + 1,
            frames: VecDeque::new(),
        }
    }

    /// Records the ships of `world` after tick `tick`.
    pub fn record(&mut self, tick: u64, world: &World) {
        let ships = world.ships().map(|s| (s.player, s.pos, s.alive)).collect();
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        let fx = world.event_effects();
        self.frames.push_back(HistoryFrame {
            tick,
            ships,
            hit_scale: fx.hit_radius_scale,
            hit_plane: fx.hit_plane,
        });
    }

    pub fn oldest_tick(&self) -> Option<u64> {
        self.frames.front().map(|f| f.tick)
    }

    pub fn newest_tick(&self) -> Option<u64> {
        self.frames.back().map(|f| f.tick)
    }

    /// The frame for `tick`, or the oldest one if `tick` is older than the
    /// window; `None` if `tick` is in the future or nothing was recorded.
    pub fn frame(&self, tick: u64) -> Option<&HistoryFrame> {
        let first = self.frames.front()?;
        if tick <= first.tick {
            return Some(first);
        }
        self.frames.get((tick - first.tick) as usize)
    }

    /// The event effects of ships' hits during `tick` (as
    /// [`ShipHistory::frame`]): `(hit sphere scale, hit plane)`.
    pub fn hit_rules(&self, tick: u64) -> (f32, Option<Vec3>) {
        self.frame(tick)
            .map_or((1.0, None), |f| (f.hit_scale, f.hit_plane))
    }

    /// Position of `player` after `tick` if it was alive then.
    pub fn position(&self, player: PlayerId, tick: u64) -> Option<Vec3> {
        let f = self.frame(tick)?;
        let i = f.ships.binary_search_by_key(&player, |s| s.0).ok()?;
        let (_, pos, alive) = f.ships[i];
        alive.then_some(pos)
    }
}

/// A target moved by `offset` for one simulation step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Displacement {
    pub player: PlayerId,
    pub offset: Vec3,
    /// Projectile that will hit the rewound target.
    pub projectile: u32,
}

/// Plans the displacements for the step that produces tick `world.tick() + 1`
/// (see the module docs). `inputs` are the commands the step will use and
/// `rewinds` the rewind (ticks) of each projectile in flight.
pub fn plan_compensation(
    world: &World,
    terrain: &GameTerrain,
    inputs: &BTreeMap<PlayerId, InputCmd>,
    history: &ShipHistory,
    rewinds: &BTreeMap<u32, u64>,
) -> Vec<Displacement> {
    let k = world.tick() + 1;
    let mut out: Vec<Displacement> = Vec::new();
    // Post-movement positions of this step, computed on first need.
    let mut moved: Option<BTreeMap<PlayerId, Vec3>> = None;
    let cmd_of = |p: PlayerId| inputs.get(&p).copied().unwrap_or_else(|| InputCmd::idle(k));

    for p in world.projectiles() {
        let Some(&rewind) = rewinds.get(&p.id) else {
            continue;
        };
        if rewind == 0 || k >= p.expires_tick {
            continue;
        }
        let speed = p.vel.length();
        let Some(dir) = p.vel.try_normalize() else {
            continue;
        };
        let len = speed * DT;
        // The coming step's hit rules (Big head day scale, Wrong genre's
        // depth-free hits) and the rewound tick's for the rewound target.
        let fx = world.next_event_effects();
        let radius_of = |s: &ls_sim::ShipState| {
            world.tuning_for(s.ship_type).radius * fx.hit_radius_scale + p.radius
        };
        let center =
            |fx: &ls_sim::EventEffects, c: Vec3| ls_sim::director::ship_hit_center(fx, p.pos, c);
        let wall = terrain
            .raycast(p.pos, dir, len)
            .map_or(f32::INFINITY, |h| h.distance);
        let then = k.saturating_sub(rewind);
        let (then_scale, then_plane) = history.hit_rules(then);
        let then_fx = ls_sim::EventEffects {
            hit_plane: then_plane,
            ..ls_sim::EventEffects::NONE
        };

        // Nearest rewound target on the segment, in front of any wall.
        let mut best: Option<(f32, PlayerId, Vec3)> = None;
        for s in world.ships() {
            if !s.alive || s.player == p.owner || out.iter().any(|d| d.player == s.player) {
                continue;
            }
            let Some(r) = history.position(s.player, then) else {
                continue;
            };
            let then_radius = world.tuning_for(s.ship_type).radius * then_scale + p.radius;
            if let Some(t) = segment_sphere(p.pos, dir, len, center(&then_fx, r), then_radius) {
                if t <= wall && best.is_none_or(|(bt, _, _)| t < bt) {
                    best = Some((t, s.player, r));
                }
            }
        }
        let Some((t_best, target, rewound)) = best else {
            continue;
        };

        let moved = moved.get_or_insert_with(|| {
            world
                .ships()
                .filter(|s| s.alive)
                .map(|s| {
                    let mut c = s.clone();
                    let t = world.tuning_for(s.ship_type);
                    simulate_movement(&mut c, &cmd_of(s.player), t, terrain);
                    (s.player, c.pos)
                })
                .collect()
        });
        // Would the simulation hit something at least as early anyway?
        let sim_hits = moved.iter().any(|(id, pos)| {
            let r = world.ship(*id).map_or(0.5, radius_of);
            *id != p.owner
                && segment_sphere(p.pos, dir, len, center(&fx, *pos), r)
                    .is_some_and(|t| t <= wall && t <= t_best + 1e-4)
        });
        if sim_hits {
            continue;
        }

        let cmd = cmd_of(target);
        if cmd.buttons.contains(Buttons::FIRE_PRIMARY)
            || cmd.buttons.contains(Buttons::FIRE_SECONDARY)
            || cmd.buttons.contains(Buttons::SCAVENGE)
            || cmd.buttons.contains(Buttons::GADGET)
        {
            continue;
        }
        let reach = PICKUP_RADIUS
            + world
                .ship(target)
                .map_or(0.5, |s| world.tuning_for(s.ship_type).radius);
        if world
            .pickups()
            .iter()
            .any(|sp| sp.content.is_some() && sp.content_pos().distance(rewound) <= reach)
        {
            continue;
        }
        let (Some(ship), Some(&after)) = (world.ship(target), moved.get(&target)) else {
            continue;
        };
        let offset = rewound - after;
        let mut probe = ship.clone();
        probe.pos += offset;
        simulate_movement(&mut probe, &cmd, world.tuning_for(ship.ship_type), terrain);
        if probe.pos.distance(rewound) > 1e-3
            || segment_sphere(p.pos, dir, len, center(&fx, probe.pos), radius_of(ship)).is_none()
        {
            continue;
        }
        moved.insert(target, probe.pos);
        out.push(Displacement {
            player: target,
            offset,
            projectile: p.id,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::BoxRoomTerrain;
    use ls_sim::{EventKind, MatchSetup, ShipType, SpawnPoint, WorldConfig};

    /// Big head day and Wrong genre: the history keeps each tick's hit rules,
    /// so a rewound target is tested with the scale and plane of its tick.
    #[test]
    fn history_keeps_the_hit_rules_of_each_tick() {
        let setup = MatchSetup {
            spawns: vec![SpawnPoint {
                pos: Vec3::ZERO,
                team: 0,
                orientation: ls_math::Quat::IDENTITY,
            }],
            ..Default::default()
        };
        let mut w = World::new(WorldConfig::paint_the_void(3, setup)).unwrap();
        w.add_player(None, ShipType::Tinker).unwrap();
        let mut t = BoxRoomTerrain::centered(Vec3::splat(60.0));
        let mut h = ShipHistory::new(600);
        w.schedule_event(2, EventKind::BigHeadDay);
        let mut seen = Vec::new();
        for _ in 0..400 {
            let next = w.next_event_effects();
            w.step(&mut t, &[]);
            assert_eq!(next, w.event_effects(), "planned = applied");
            h.record(w.tick(), &w);
            seen.push((w.tick(), h.hit_rules(w.tick()).0));
        }
        let active = w.mood_view().active_tick;
        assert_eq!(h.hit_rules(active), (1.0, None), "announce and start tick");
        assert_eq!(h.hit_rules(active + 1), (3.0, None));
        assert_eq!(h.hit_rules(1), (1.0, None));
        assert!(seen.iter().any(|(_, s)| *s == 3.0));
    }
}
