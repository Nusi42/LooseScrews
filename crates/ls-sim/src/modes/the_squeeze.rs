//! The Squeeze: free-for-all, one life; walls grow inward from the level
//! boundary in phases (warning, then closing), crushing ships caught in
//! them; last ship flying wins.
//!
//! Settings: `ruleset::TheSqueezeRules`. Design: `docs/milestone-3-plan.md`
//! §3.2 and `docs/design/game-design-doc.md` (The Squeeze).
//!
//! # The walls
//!
//! The walls are a solid shell around a safe sphere ([`ShellSphere`]): the
//! terrain answers `min(terrain, shell)` (`SimTerrain::set_shell`, set by
//! the world at the start of every tick from [`ModeModule::terrain_shell`]),
//! so ships, projectiles and debris collide with it like with rock, and
//! clients mesh it (`ls_game::GameTerrain`). It is not a generator change:
//! worldgen golden hashes and share codes stay.
//!
//! # Timeline (ticks from the match start)
//!
//! - Stage 0: no walls yet (the zone is the level's circumscribed sphere).
//! - Phase `i` (stage `i`, 1..=`phases`) starts at
//!   `first_phase_secs + (i - 1) * (warn_secs + close_secs)`: the next zone
//!   is announced (a `NEXT_ZONE` marker); `warn_secs` later the shell closes
//!   from the current zone to it over `close_secs`, in steps of at most
//!   [`SHELL_STEP`] u of surface travel (bounded remeshing).
//! - Stage `phases + 1`: right after the last phase the shell keeps closing
//!   slowly, to radius 0 around the last centre over
//!   [`FINAL_CLOSE_FACTOR`] x `close_secs`, so the match always ends.
//! - Stage `phases + 2`: sealed.
//!
//! Safe radii are [`RADIUS_FRACTIONS`] (60 %, 45 %, 30 %, 18 %, 8 %) of the
//! level radius (half the bounds diagonal), stretched for other phase
//! counts. Each zone's centre is drawn from the match seed through the
//! [`RNG_STREAM_SQUEEZE`] stream ([`next_zone`]): it shifts every phase,
//! preferably to one of the level's spawn and pickup points (open space on
//! the pristine level), else to a random open point, always keeping the new
//! sphere inside the previous one; it is snapped to whole units, so a
//! client recovers it exactly from the quantized marker position.
//!
//! # Replication
//!
//! The schedule lives in two objectives the `ModeView` replicates:
//! `SAFE_ZONE` (id 0: current centre and radius, `timer_end` = tick the
//! close starts, `state` [`ZONE_WALLS`] once walls exist) and `NEXT_ZONE`
//! (id 1, from a phase's warning on: target centre and radius, `timer_end`
//! = tick the close ends). The shell at any tick is a pure function of the
//! two ([`ShellSchedule::shell_at`]), so a client predicts the walls at its
//! own tick from the last mode state it got (`ls_game::mode::squeeze_shell`).
//! Hand-overs between phases are continuous (a close ends exactly on the
//! next zone), so a client a few ticks behind on the mode state computes
//! the same shell.
//!
//! # Rules
//!
//! - One life (`ModeRules::lives`): the framework eliminates at zero and
//!   late joiners spectate; the last ship flying wins, ships dying in the
//!   same tick as the last rival are a draw; `match_secs` is a backstop.
//! - Crushing: `crush_per_sec` hull per second while a ship touches a
//!   moving wall (within [`CONTACT_MARGIN`]); ignores shields. A ship the
//!   shell swallowed whole, or whose zone is smaller than its hull, is
//!   crushed at once, so no ship survives inside the shell.
//! - Scrap: while a phase closes, every [`SCRAP_BATCH_TICKS`] the wall
//!   samples [`SCRAP_SAMPLES`] points on its surface; the open share of the
//!   sphere is the opening it closes, and `scrap_per_100u2` pieces per
//!   100 u² of it per phase are shed inward as debris (at most
//!   [`MAX_DEBRIS`] per phase; the world's debris cap applies too).
//! - Kills score 1 for the killer; crushing and suicides score nothing.

use ls_math::rng::mix64;
use ls_math::{det, Aabb, Pcg32, Vec3};

use crate::entity::{EntitySpawn, MAX_DEBRIS};
use crate::mode::{objective_kind, ModeKind, ModeState, Objective, ObjectiveView};
use crate::modes::{add_score, lose_life, ticks, Kill, ModeCtx, ModeModule, ModeMut};
use crate::ruleset::{ModeRules, TheSqueezeRules};
use crate::ship::PlayerId;
use crate::terrain::ShellSphere;
use crate::Command;

/// RNG stream of the safe-zone centres and the wall scrap ("SIM_SQZE"),
/// seeded per phase / per tick from the match seed.
pub const RNG_STREAM_SQUEEZE: u64 = 0x5349_4d5f_5351_5a45;
/// Most surface travel (centre shift plus radius change, u) of one shell
/// step while closing.
pub const SHELL_STEP: f32 = 0.5;
/// Safe radius as a share of the level radius after each of the 5 default
/// phases (index 0: the level itself).
pub const RADIUS_FRACTIONS: [f32; 6] = [1.0, 0.60, 0.45, 0.30, 0.18, 0.08];
/// The final close (to radius 0) lasts this many times `close_secs`.
pub const FINAL_CLOSE_FACTOR: u64 = 4;
/// A ship whose hull is within this distance of a moving wall is crushed.
pub const CONTACT_MARGIN: f32 = 0.1;
/// Ticks between two scrap batches of a closing wall.
pub const SCRAP_BATCH_TICKS: u64 = 30;
/// Wall surface samples per scrap batch.
pub const SCRAP_SAMPLES: u32 = 16;
/// Scrap carried by one piece of wall debris.
pub const SCRAP_PER_PIECE: f32 = 5.0;
/// Speed (u/s) at which wall debris is shed inward.
pub const SCRAP_SPEED: f32 = 2.5;
/// Objective id of the current safe zone.
pub const SAFE_ZONE_ID: u8 = 0;
/// Objective id of the next safe zone (present from a phase's warning on).
pub const NEXT_ZONE_ID: u8 = 1;
/// `SAFE_ZONE` state bit: walls exist (0 before the first close).
pub const ZONE_WALLS: u8 = 1;

/// A safe sphere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    pub center: Vec3,
    pub radius: f32,
}

impl Zone {
    pub fn shell(self) -> ShellSphere {
        ShellSphere {
            center: self.center,
            radius: self.radius,
        }
    }

    /// Whether `inner` lies inside this sphere.
    pub fn contains(self, inner: Zone) -> bool {
        self.center.distance(inner.center) + inner.radius <= self.radius
    }
}

/// The replicated wall schedule: the current zone and, while a phase runs,
/// the next one with its close window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellSchedule {
    pub current: Zone,
    /// Walls exist at the current zone (false before the first close).
    pub walls: bool,
    /// The next zone, the tick its close starts and the tick it ends.
    pub next: Option<(Zone, u64, u64)>,
}

impl ShellSchedule {
    /// The schedule from the `SAFE_ZONE` / `NEXT_ZONE` markers (`None`
    /// without a safe zone).
    pub fn from_objectives<'a>(
        objectives: impl IntoIterator<Item = &'a ObjectiveView>,
    ) -> Option<Self> {
        let mut safe = None;
        let mut next = None;
        for o in objectives {
            let zone = Zone {
                center: o.pos,
                radius: o.radius,
            };
            match o.kind {
                objective_kind::SAFE_ZONE => safe = Some((zone, o.timer_end, o.state)),
                objective_kind::NEXT_ZONE => next = Some((zone, o.timer_end)),
                _ => {}
            }
        }
        let (current, start, state) = safe?;
        Some(ShellSchedule {
            current,
            walls: state & ZONE_WALLS != 0,
            next: next
                .filter(|&(_, end)| end > start)
                .map(|(z, end)| (z, start, end)),
        })
    }

    /// The schedule held by a Squeeze mode state.
    pub fn from_state(state: &ModeState) -> Option<Self> {
        let views: Vec<ObjectiveView> = state.objectives.iter().map(|o| o.view()).collect();
        Self::from_objectives(&views)
    }

    /// The shell at tick `now` (`None` before the first close).
    pub fn shell_at(&self, now: u64) -> Option<ShellSphere> {
        match self.next {
            Some((to, start, end)) if now >= start => {
                Some(close_step(self.current, to, start, end, now).shell())
            }
            _ => self.walls.then(|| self.current.shell()),
        }
    }

    /// Whether the walls move at tick `now` (a close is running).
    pub fn closing(&self, now: u64) -> bool {
        self.next
            .is_some_and(|(_, start, end)| now >= start && now < end)
    }
}

/// The zone between `from` and `to` at `now` within the close window
/// `[start, end)`: linear in time, quantized so the surface moves at most
/// [`SHELL_STEP`] u per step; exactly `to` from `end` on. With `to` inside
/// `from`, every step lies inside the previous one.
pub fn close_step(from: Zone, to: Zone, start: u64, end: u64, now: u64) -> Zone {
    if now >= end || end <= start {
        return to;
    }
    if now <= start {
        return from;
    }
    let travel = from.center.distance(to.center) + (from.radius - to.radius).max(0.0);
    let steps = (det::ceil(travel / SHELL_STEP) as u64).max(1);
    let k = (now - start) * steps / (end - start);
    let f = k as f32 / steps as f32;
    Zone {
        center: from.center + (to.center - from.center) * f,
        radius: from.radius + (to.radius - from.radius) * f,
    }
}

/// The level radius: half the bounds diagonal (the circumscribed sphere).
pub fn level_radius(bounds: Aabb) -> f32 {
    bounds.size().length() * 0.5
}

/// The safe radius after phase `phase` (1..=`phases`) as a share of the
/// level radius: [`RADIUS_FRACTIONS`], stretched linearly over `phases`.
pub fn radius_fraction(phase: u8, phases: u8) -> f32 {
    let phases = phases.max(1);
    let last = (RADIUS_FRACTIONS.len() - 1) as f32;
    let x = (phase.min(phases) as f32) * last / phases as f32;
    let lo = (det::floor(x) as usize).min(RADIUS_FRACTIONS.len() - 1);
    let hi = (lo + 1).min(RADIUS_FRACTIONS.len() - 1);
    let t = x - lo as f32;
    RADIUS_FRACTIONS[lo] + (RADIUS_FRACTIONS[hi] - RADIUS_FRACTIONS[lo]) * t
}

/// `p` snapped to whole units.
pub fn snap(p: Vec3) -> Vec3 {
    Vec3::new(
        det::floor(p.x + 0.5),
        det::floor(p.y + 0.5),
        det::floor(p.z + 0.5),
    )
}

/// Random points tried per phase when no candidate fits.
pub const ZONE_TRIES: u32 = 8;

/// The zone of phase `phase` inside `prev` with radius `radius`. Its centre
/// is drawn from `seed` ([`RNG_STREAM_SQUEEZE`]) so the zone shifts each
/// phase: among the `candidates` (open points of the level) that keep it
/// inside `prev` with a 1 u margin and lie at least a quarter of that
/// leeway away from the old centre; else among up to [`ZONE_TRIES`] random
/// points in the same shell, the first where `open` holds (or the last).
/// Snapped to whole units and kept inside `prev`.
pub fn next_zone(
    seed: u64,
    phase: u8,
    prev: Zone,
    radius: f32,
    candidates: &[Vec3],
    open: &dyn Fn(Vec3) -> bool,
) -> Zone {
    let radius = radius.clamp(0.0, prev.radius);
    let allowed = prev.radius - radius - 1.0;
    let mut rng = Pcg32::new(
        mix64(seed ^ mix64(phase as u64 ^ RNG_STREAM_SQUEEZE)),
        RNG_STREAM_SQUEEZE,
    );
    let mut center = prev.center;
    if allowed > 0.0 {
        let min_shift = allowed * 0.25;
        let fits: Vec<Vec3> = candidates
            .iter()
            .copied()
            .filter(|c| {
                let d = c.distance(prev.center);
                c.is_finite() && d <= allowed && d >= min_shift
            })
            .collect();
        center = if fits.is_empty() {
            let mut p = prev.center;
            for _ in 0..ZONE_TRIES {
                let dir = Vec3::new(
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                )
                .normalize_or_zero();
                p = prev.center + dir * rng.range_f32(min_shift, allowed);
                if open(snap(p)) {
                    break;
                }
            }
            p
        } else {
            fits[rng.below(fits.len() as u32) as usize]
        };
    }
    let mut zone = Zone {
        center: snap(center),
        radius,
    };
    if !prev.contains(zone) {
        zone.center = prev.center;
    }
    zone
}

/// Tick (from the match start) at which phase `phase` (1..) starts its
/// warning.
pub fn warn_start(r: &TheSqueezeRules, phase: u8) -> u64 {
    let period = ticks(r.warn_secs) + ticks(r.close_secs);
    ticks(r.first_phase_secs) + (phase.max(1) - 1) as u64 * period
}

/// The close window `[start, end)` of stage `stage` (1..=`phases` + 1).
pub fn close_window(r: &TheSqueezeRules, stage: u8) -> (u64, u64) {
    if stage > r.phases {
        let start = warn_start(r, r.phases + 1);
        (
            start,
            start + FINAL_CLOSE_FACTOR * ticks(r.close_secs).max(1),
        )
    } else {
        let start = warn_start(r, stage) + ticks(r.warn_secs);
        (start, start + ticks(r.close_secs).max(1))
    }
}

/// The stage the timeline is in at `now`.
pub fn stage_at(r: &TheSqueezeRules, now: u64) -> u8 {
    let first = ticks(r.first_phase_secs);
    if now < first {
        return 0;
    }
    let period = (ticks(r.warn_secs) + ticks(r.close_secs)).max(1);
    let phase = 1 + (now - first) / period;
    if phase <= r.phases as u64 {
        return phase as u8;
    }
    if now < close_window(r, r.phases + 1).1 {
        r.phases + 1
    } else {
        r.phases + 2
    }
}

fn rules(m: &ModeRules) -> TheSqueezeRules {
    match m {
        ModeRules::TheSqueeze(r) => *r,
        _ => TheSqueezeRules::default(),
    }
}

fn zone_objective(id: u8, kind: u8, zone: Zone, timer_end: u64, state: u8) -> Objective {
    let mut o = Objective::new(id, kind, zone.center);
    o.radius = zone.radius;
    o.timer_end = timer_end;
    o.state = state;
    o
}

fn zone_of(o: &Objective) -> Zone {
    Zone {
        center: o.pos,
        radius: o.radius,
    }
}

/// The Squeeze mode module.
#[derive(Debug)]
pub struct TheSqueeze;

impl TheSqueeze {
    /// Moves the state on by one stage: the next zone becomes the current
    /// one (once a phase has closed) and the following zone is announced.
    fn advance(m: &mut ModeCtx, r: &TheSqueezeRules) {
        let Some(safe) = m.state.objective(SAFE_ZONE_ID) else {
            return;
        };
        let (mut cur, mut walls) = (zone_of(safe), safe.state & ZONE_WALLS != 0);
        let stage = m.state.stage;
        if let Some(next) = m.state.objective(NEXT_ZONE_ID).filter(|_| stage >= 1) {
            cur = zone_of(next);
            walls = true;
        }
        let new_stage = stage + 1;
        m.state.objectives.clear();
        m.state.vars[1] = 0;
        m.state.vars[2] = 0;
        m.state.stage = new_stage;
        if new_stage > r.phases + 1 {
            m.state.objectives.push(zone_objective(
                SAFE_ZONE_ID,
                objective_kind::SAFE_ZONE,
                cur,
                0,
                ZONE_WALLS,
            ));
            m.state.stage_end_tick = 0;
            return;
        }
        let next = if new_stage > r.phases {
            Zone {
                center: cur.center,
                radius: 0.0,
            }
        } else {
            let level = f32::from_bits(m.state.vars[0] as u32);
            let radius = level * radius_fraction(new_stage, r.phases);
            let mut candidates: Vec<Vec3> = m.setup.spawns.iter().map(|s| s.pos).collect();
            candidates.extend(m.setup.pickup_points.iter().copied());
            let terrain = m.ctx.view.terrain;
            let open = |p: Vec3| terrain.bounds().contains(p) && terrain.sdf(p) > 2.0;
            next_zone(m.ctx.view.seed, new_stage, cur, radius, &candidates, &open)
        };
        let (start, end) = close_window(r, new_stage);
        m.state.objectives.push(zone_objective(
            SAFE_ZONE_ID,
            objective_kind::SAFE_ZONE,
            cur,
            start,
            if walls { ZONE_WALLS } else { 0 },
        ));
        m.state.objectives.push(zone_objective(
            NEXT_ZONE_ID,
            objective_kind::NEXT_ZONE,
            next,
            end,
            0,
        ));
        m.state.stage_end_tick = end;
    }

    /// Crushing: damage while touching a moving wall, death when swallowed.
    fn crush(m: &mut ModeCtx, r: &TheSqueezeRules, shell: ShellSphere, moving: bool) {
        let per_tick = r.crush_per_sec * ls_math::DT;
        let ships: Vec<(PlayerId, Vec3, f32)> = m
            .ctx
            .view
            .ships
            .values()
            .filter(|s| s.alive)
            .map(|s| (s.player, s.pos, m.ctx.tuning_of(s).radius))
            .collect();
        for (id, pos, radius) in ships {
            let d = pos.distance(shell.center);
            let delta = if shell.radius < radius || d - radius > shell.radius {
                f32::NEG_INFINITY
            } else if moving && per_tick > 0.0 && d + radius >= shell.radius - CONTACT_MARGIN {
                -per_tick
            } else {
                continue;
            };
            m.ctx.push(Command::AdjustHull {
                target: id,
                delta,
                can_kill: true,
            });
        }
    }

    /// Wall scrap: one batch while a phase closes (see the module docs).
    fn shed_scrap(m: &mut ModeCtx, r: &TheSqueezeRules, shell: ShellSphere, start: u64, end: u64) {
        let now = m.now();
        if m.state.stage > r.phases
            || r.scrap_per_100u2 <= 0.0
            || now < start
            || (now - start) % SCRAP_BATCH_TICKS != 0
            || shell.radius <= 2.0
        {
            return;
        }
        let batches = ((end - start) / SCRAP_BATCH_TICKS).max(1);
        let mut rng = Pcg32::new(
            mix64(m.ctx.view.seed ^ mix64(now ^ RNG_STREAM_SQUEEZE.rotate_left(17))),
            RNG_STREAM_SQUEEZE,
        );
        let bounds = m.ctx.view.terrain.bounds();
        let mut open: Vec<(Vec3, Vec3)> = Vec::new();
        for _ in 0..SCRAP_SAMPLES {
            let dir = Vec3::new(
                rng.range_f32(-1.0, 1.0),
                rng.range_f32(-1.0, 1.0),
                rng.range_f32(-1.0, 1.0),
            )
            .normalize_or_zero();
            if dir == Vec3::ZERO {
                continue;
            }
            let p = shell.center + dir * (shell.radius - 1.0);
            if bounds.contains(p) && m.ctx.view.terrain.sdf(p) > 0.8 {
                open.push((p, dir));
            }
        }
        let share = open.len() as f32 / SCRAP_SAMPLES as f32;
        let area = share * 4.0 * std::f32::consts::PI * shell.radius * shell.radius;
        let due = r.scrap_per_100u2 * area / 100.0 / batches as f32;
        m.state.vars[1] += (due * 1000.0) as i64;
        let budget = MAX_DEBRIS as i64 - m.state.vars[2];
        let pieces = (m.state.vars[1] / 1000)
            .min(budget)
            .min(open.len() as i64)
            .max(0);
        m.state.vars[1] -= pieces * 1000;
        m.state.vars[2] += pieces;
        for &(p, dir) in open.iter().take(pieces as usize) {
            let radius = rng.range_f32(0.3, 0.7);
            m.ctx.spawn_entity(EntitySpawn::debris(
                p,
                -dir * SCRAP_SPEED,
                radius,
                SCRAP_PER_PIECE,
            ));
        }
    }
}

impl ModeModule for TheSqueeze {
    fn kind(&self) -> ModeKind {
        ModeKind::TheSqueeze
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }

    fn validate(&self, rules: &ModeRules) -> Result<(), String> {
        let r = self::rules(rules);
        if r.phases == 0 {
            return Err("The Squeeze needs at least one phase".into());
        }
        if r.close_secs.is_nan() || r.close_secs < 1.0 {
            return Err("The Squeeze: close_secs must be at least 1".into());
        }
        for (name, v) in [
            ("first_phase_secs", r.first_phase_secs),
            ("warn_secs", r.warn_secs),
            ("crush_per_sec", r.crush_per_sec),
            ("scrap_per_100u2", r.scrap_per_100u2),
        ] {
            if !(v.is_finite() && v >= 0.0) {
                return Err(format!("The Squeeze: {name} must be 0 or more"));
            }
        }
        Ok(())
    }

    /// The first zone: the level's circumscribed sphere, no walls yet. The
    /// level radius is kept in `vars[0]` (f32 bits) for the phase radii.
    fn on_start(&self, m: &mut ModeCtx) {
        let bounds = m.ctx.view.terrain.bounds();
        let radius = level_radius(bounds);
        m.state.vars[0] = radius.to_bits() as i64;
        let zone = Zone {
            center: snap(bounds.center()),
            radius,
        };
        let r = rules(m.rules);
        m.state.objectives.clear();
        m.state.objectives.push(zone_objective(
            SAFE_ZONE_ID,
            objective_kind::SAFE_ZONE,
            zone,
            0,
            0,
        ));
        m.state.stage = 0;
        m.state.stage_end_tick = warn_start(&r, 1);
    }

    /// Kills score 1 for the killer; every death costs the one life.
    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        if let Some(k) = kill.killer.filter(|_| !kill.is_suicide()) {
            add_score(m, k, 1);
        }
        lose_life(m.state, kill.victim);
    }

    fn on_tick(&self, m: &mut ModeCtx) {
        let r = rules(m.rules);
        let now = m.now();
        let Some(sched) = ShellSchedule::from_state(m.state) else {
            return;
        };
        // The shell the physics used this tick (set before anything moved).
        if let Some(shell) = sched.shell_at(now) {
            let closing = sched.closing(now);
            let sealed = m.state.stage > r.phases + 1;
            Self::crush(m, &r, shell, closing || sealed);
            if let Some((_, start, end)) = sched.next.filter(|_| closing) {
                Self::shed_scrap(m, &r, shell, start, end);
            }
        }
        // Then the timeline moves on (the shell is continuous across a
        // stage change: the new state gives the same shell at `now`).
        let target = stage_at(&r, now);
        while m.state.stage < target {
            Self::advance(m, &r);
        }
    }

    fn terrain_shell(&self, state: &ModeState, now: u64) -> Option<ShellSphere> {
        ShellSchedule::from_state(state)?.shell_at(now)
    }
}

#[cfg(test)]
mod tests;
