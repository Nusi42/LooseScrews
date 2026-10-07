//! The Architect (M4; `docs/milestone-4-plan.md` §3.5): one architect
//! without a ship moves the world sliders live inside regions (strokes,
//! paid from a budget) against the runners, who must reach the exit gate at
//! the far end of the level. A match is `min(players, max_rounds)` rounds;
//! the role rotates by join order.
//!
//! - **Roles.** Free-for-all (every ship its own side). Round `r`'s
//!   architect is the `r`-th player (by player id, i.e. join order, among
//!   those not spectating by choice, wrapping). The architect's ship is out
//!   of play ([`player_flags::ARCHITECT`]); the others are runners. Runners
//!   cannot damage each other (`on_damage` zeroes it; knockback stays). A
//!   match with a single player has no architect (the runner practises);
//!   if a second player arrives in round 1's first
//!   [`ARCHITECT_GRACE_TICKS`], the lowest id becomes the architect and the
//!   round clock restarts.
//! - **Runners.** Spawn at the team 0 end (`ls_game::setup` keeps only team
//!   0's spawn points for this mode), `runner_lives` lives per round
//!   (respawn after `respawn_secs`), the rule set's loadout and pickups. A
//!   runner whose ship centre enters the exit gate scores 1 and leaves play
//!   for the rest of the round ([`player_flags::FINISHED`]).
//! - **Rounds.** A round ends when no runner is left in play (all finished
//!   or out of lives) or after `round_secs`. Every runner who did not make
//!   it scores 1 for the architect; the runners win the round if at least
//!   half of them got out. After `round_results_secs` the next round starts:
//!   the next architect, every other player respawned at the start with
//!   full lives, a fresh budget. The match ends after the last round's
//!   results; the most points win (a tie is a draw).
//! - **Exit.** The Heist site's team 1 extraction zone (an open cavern at
//!   the team 1 end), else the pickup point farthest from the spawns.
//!   Objective [`EXIT_ID`] ([`objective_kind::EXIT`]): `holder` = the
//!   round's architect, `state` per [`exit_state`], `timer_end` = the
//!   round's (or the results') end.
//! - **Budget and strokes.** The architect has `budget_start` points at the
//!   round start, `+budget_per_sec` (up to `budget_max`); the server
//!   validates every `ArchitectAction` (`World::architect_action`: role,
//!   round, kind, radius `min_radius..=max_radius`, range: inside the level
//!   and, for rock-changing strokes, `keep_clear` beyond the radius from the
//!   exit and every spawn point; cooldown, strokes in flight, budget, once
//!   per round for Verticality) and refuses the rest ([`StrokeReject`]; the
//!   room counts them). An accepted stroke is telegraphed for
//!   `telegraph_secs` (`CueKind::StrokeTelegraph` and objective
//!   [`objective_kind::STROKE`], which runners see), then acts:
//!
//! | Stroke | Effect |
//! | --- | --- |
//! | Enclosure+ | grows rock where the generator with Enclosure `+reshape_delta` has rock ([`plan_reshape`]) |
//! | Enclosure- | opens rock where the generator with Enclosure `-reshape_delta` is open |
//! | Density+ | grows rock where the generator with Density `+reshape_delta` has rock (pillars, floating rocks) |
//! | Gravity Chaos+ | a dynamic well at the centre for `well_secs` (peak as a Gravity Chaos 100 site, range 1.5 x radius in 20-40 u) |
//! | Instability+ | a cave-in at the centre (`crate::instability::plan_cave_in`: hollow, weak rubble, debris, 20 damage) |
//! | Absurdity+ | a random eligible comedy event (enabled, not banned), forced at the telegraph's end |
//! | Palette | an ink blackout of the region for `palette_secs` (presentation only) |
//! | Verticality | Bored level, once per round |
//!
//! Rock strokes emit at most [`MAX_STROKE_EDITS`] ordinary `AddSphere` /
//! `SubtractSphere` edits (no new `TerrainEdit` kind), spread over
//! [`STROKE_EDIT_TICKS`] ticks. An `AddSphere` never overlaps a ship (with
//! [`STROKE_SHIP_CLEARANCE`]) and never encloses one: a ship near a batch
//! that could reach open space `escape_reach` away before must still reach
//! it with the batch (`crate::instability::escapes`), else the batch's
//! sphere nearest to it is left out until it does.
//! - **The director** rolls nothing in this mode (`allows_event` vetoes
//!   every event); only the strokes start events.
//! - **View.** `meter`: the architect's budget (points, clamped to 255);
//!   for everyone else their architect points so far (runner points =
//!   `score - meter`); once the match ended nobody is the architect, so
//!   `meter` is every player's architect points. `lives`: runner lives
//!   left this round.
//!
//! Numbers: `data/rulesets/the_architect.ron` ([`TheArchitectRules`]).

use std::collections::{BTreeMap, BTreeSet};

use ls_math::terrain::TerrainEdit;
use ls_math::{Tick, Vec3};
use serde::{Deserialize, Serialize};

use super::{lose_life, ModeCtx, ModeModule, ModeMut, ModeRef, Outcome};
use crate::events::EventKind;
use crate::hash::StateHasher;
use crate::mode::{
    objective_kind, player_flags, view_flags, MatchPhase, ModeKind, ModeState, ModeView, Objective,
    ObjectiveView, NO_SIDE,
};
use crate::modes::{best_player_side, Kill, ModeHit};
use crate::ruleset::{ModeRules, TheArchitectRules};
use crate::ship::PlayerId;
use crate::terrain::{reshape_slider, SimTerrain};

/// Objective id of the exit gate.
pub const EXIT_ID: u8 = 0;
/// Most edits a rock stroke emits.
pub const MAX_STROKE_EDITS: usize = 32;
/// Ticks a stroke's edits are spread over.
pub const STROKE_EDIT_TICKS: u64 = 8;
/// A stroke refused after planning ([`StrokeReject::NothingToChange`]: the
/// reshape sampled the generator, the cave-in searched a ceiling, the
/// event stroke found no event) holds the next stroke back this long, so
/// a flood of duds cannot make the server plan every tick (D1 review).
pub const STROKE_DUD_TICKS: u64 = 6;
/// Spacing (u) of the grid a reshape stroke samples inside its sphere.
pub const RESHAPE_GRID: f32 = 4.0;
/// Largest edit sphere of a reshape stroke (u).
pub const EDIT_RADIUS_MAX: f32 = 6.0;
/// Smallest edit sphere of a reshape stroke (u).
pub const EDIT_RADIUS_MIN: f32 = 1.5;
/// Room a stroke's `AddSphere` keeps around a ship (beyond its radius).
pub const STROKE_SHIP_CLEARANCE: f32 = 1.0;
/// Exit radius when the level has no Heist site.
pub const EXIT_RADIUS_FALLBACK: f32 = 6.0;
/// Dynamic well keys of Gravity Chaos+ strokes: `WELL_KEY_BASE | stroke id`.
pub const WELL_KEY_BASE: u32 = 0x7f00;
/// Budget units per point (exact per-tick accrual for multiples of 0.25
/// points per second).
pub const BUDGET_UNIT: u32 = 240;
/// A player arriving this early in round 1 of a match that started without
/// an architect makes the lowest id the architect (the round restarts).
pub const ARCHITECT_GRACE_TICKS: u64 = 10 * ls_math::TICK_HZ as u64;
/// Key of the Absurdity+ stroke's event choice (`Ctx::rng` style).
pub const STROKE_EVENT_RNG_KEY: u64 = 0x4152_4348_4556; // "ARCHEV"
/// Key of the Instability+ stroke's cave-in RNG.
pub const STROKE_CAVE_IN_RNG_KEY: u64 = 0x4152_4348_4356; // "ARCHCV"

/// [`objective_kind::EXIT`] state bits.
pub mod exit_state {
    /// Between rounds (round results); `timer_end` = when the next round
    /// (or the match end) comes.
    pub const ROUND_OVER: u8 = 1 << 0;
    /// The round that just ended went to the runners.
    pub const RUNNERS_WON: u8 = 1 << 1;
    /// Rounds of the match in the high nibble (`state >> 4`); the current
    /// round is `ModeView::stage` (0-based).
    pub const ROUNDS_SHIFT: u8 = 4;
}

/// [`objective_kind::STROKE`] phases (`state >> 4`).
pub mod stroke_phase {
    /// Glowing outline; `timer_end` = when it acts.
    pub const TELEGRAPH: u8 = 0;
    /// Acting (edits landing, well pulling, blackout); `timer_end` = its end.
    pub const ACTIVE: u8 = 1;
}

/// The eight strokes (`ArchitectAction::stroke` on the wire; append only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum StrokeKind {
    EnclosurePlus = 0,
    EnclosureMinus = 1,
    DensityPlus = 2,
    GravityChaosPlus = 3,
    InstabilityPlus = 4,
    AbsurdityPlus = 5,
    Palette = 6,
    Verticality = 7,
}

impl StrokeKind {
    pub const ALL: [StrokeKind; 8] = [
        StrokeKind::EnclosurePlus,
        StrokeKind::EnclosureMinus,
        StrokeKind::DensityPlus,
        StrokeKind::GravityChaosPlus,
        StrokeKind::InstabilityPlus,
        StrokeKind::AbsurdityPlus,
        StrokeKind::Palette,
        StrokeKind::Verticality,
    ];

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Option<StrokeKind> {
        StrokeKind::ALL.get(i as usize).copied()
    }

    /// Stable id (UI, logs).
    pub fn id(self) -> &'static str {
        match self {
            StrokeKind::EnclosurePlus => "enclosure_plus",
            StrokeKind::EnclosureMinus => "enclosure_minus",
            StrokeKind::DensityPlus => "density_plus",
            StrokeKind::GravityChaosPlus => "gravity_chaos_plus",
            StrokeKind::InstabilityPlus => "instability_plus",
            StrokeKind::AbsurdityPlus => "absurdity_plus",
            StrokeKind::Palette => "palette",
            StrokeKind::Verticality => "verticality",
        }
    }

    /// Budget points the stroke costs.
    pub fn cost(self, r: &TheArchitectRules) -> u32 {
        (match self {
            StrokeKind::EnclosurePlus => r.cost_enclosure_plus,
            StrokeKind::EnclosureMinus => r.cost_enclosure_minus,
            StrokeKind::DensityPlus => r.cost_density_plus,
            StrokeKind::GravityChaosPlus => r.cost_gravity_plus,
            StrokeKind::InstabilityPlus => r.cost_instability_plus,
            StrokeKind::AbsurdityPlus => r.cost_absurdity_plus,
            StrokeKind::Palette => r.cost_palette,
            StrokeKind::Verticality => r.cost_verticality,
        }) as u32
    }

    /// Strokes that change rock (keep clear of the exit and the spawns).
    pub fn changes_rock(self) -> bool {
        matches!(
            self,
            StrokeKind::EnclosurePlus
                | StrokeKind::EnclosureMinus
                | StrokeKind::DensityPlus
                | StrokeKind::InstabilityPlus
        )
    }

    /// Strokes that act on the whole level (an event; the centre only
    /// places the telegraph).
    pub fn level_wide(self) -> bool {
        matches!(self, StrokeKind::AbsurdityPlus | StrokeKind::Verticality)
    }

    /// The slider and direction a reshape stroke samples the generator
    /// with (`None` for the other strokes).
    pub fn reshape(self) -> Option<(u8, i8)> {
        match self {
            StrokeKind::EnclosurePlus => Some((reshape_slider::ENCLOSURE, 1)),
            StrokeKind::EnclosureMinus => Some((reshape_slider::ENCLOSURE, -1)),
            StrokeKind::DensityPlus => Some((reshape_slider::DENSITY, 1)),
            _ => None,
        }
    }
}

/// Why the server refused an `ArchitectAction` (the room counts them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StrokeReject {
    #[error("not an Architect match, or the match is over")]
    NotArchitectMode,
    #[error("only the round's architect places strokes")]
    NotArchitect,
    #[error("the round is over")]
    RoundOver,
    #[error("unknown stroke kind")]
    UnknownStroke,
    #[error("stroke radius out of range")]
    Radius,
    #[error("stroke out of range (outside the level, or too close to the exit or a spawn)")]
    OutOfRange,
    #[error("stroke cooldown")]
    Cooldown,
    #[error("too many strokes in flight")]
    TooManyStrokes,
    #[error("not enough budget")]
    Budget,
    #[error("Verticality once per round")]
    OncePerRound,
    #[error("the stroke would change nothing there")]
    NothingToChange,
    #[error("an event is already running or coming")]
    EventBusy,
    #[error("comedy events are off in this rule set")]
    EventsOff,
}

/// Round phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoundPhase {
    Playing,
    /// Round results until `ModeState::stage_end_tick`.
    Over,
}

/// One accepted stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Unique within the match (wraps at 256).
    pub id: u8,
    pub kind: StrokeKind,
    pub center: Vec3,
    pub radius: f32,
    pub architect: PlayerId,
    /// The telegraph ends and the stroke acts at this tick.
    pub apply_tick: u64,
    /// The stroke is gone after this tick.
    pub end_tick: u64,
    /// The telegraph cue went out.
    pub announced: bool,
    /// Instability+: the cave-in was planned (at `apply_tick`).
    pub planned: bool,
    /// The telegraph is over and the stroke acts (set by the world at
    /// `apply_tick`; the replicated phase).
    pub active: bool,
    /// Rock strokes: the edits, applied over [`STROKE_EDIT_TICKS`] ticks
    /// from `apply_tick`.
    pub edits: Vec<TerrainEdit>,
}

impl Stroke {
    /// Its objective marker.
    pub fn objective(&self) -> ObjectiveView {
        let (phase, timer_end) = if self.active {
            (stroke_phase::ACTIVE, self.end_tick)
        } else {
            (stroke_phase::TELEGRAPH, self.apply_tick)
        };
        ObjectiveView {
            id: 1 + self.id % 128,
            kind: objective_kind::STROKE,
            pos: self.center,
            radius: self.radius,
            holder: Some(self.architect),
            timer_end,
            team: NO_SIDE,
            state: self.kind.index() | phase << 4,
        }
    }

    /// The edits due at `now` (an even share over [`STROKE_EDIT_TICKS`]).
    pub fn batch(&self, now: u64) -> &[TerrainEdit] {
        if now < self.apply_tick || now >= self.apply_tick + STROKE_EDIT_TICKS {
            return &[];
        }
        let i = (now - self.apply_tick) as usize;
        let per = self.edits.len().div_ceil(STROKE_EDIT_TICKS as usize);
        let a = (i * per).min(self.edits.len());
        let b = ((i + 1) * per).min(self.edits.len());
        &self.edits[a..b]
    }
}

/// One finished round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundResult {
    pub architect: Option<PlayerId>,
    /// Runners of the round (left players do not count).
    pub runners: u8,
    /// Runners through the exit.
    pub finished: u8,
}

impl RoundResult {
    /// At least half the runners got out.
    pub fn runners_won(&self) -> bool {
        self.runners > 0 && 2 * self.finished as u32 >= self.runners as u32
    }
}

/// The Architect's state (`ModeState::architect`; part of the state hash).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectState {
    /// Rounds of the match (`min(players, max_rounds)`, grows when players
    /// join before a round ends).
    pub rounds: u8,
    /// Current round (0-based; also `ModeState::stage`).
    pub round: u8,
    pub phase: RoundPhase,
    pub round_start_tick: u64,
    /// The match ends here at the latest (`match_secs`).
    pub hard_end_tick: u64,
    pub architect: Option<PlayerId>,
    /// Budget in [`BUDGET_UNIT`]s of a point.
    pub budget: u32,
    /// No stroke before this tick (cooldown).
    pub next_stroke_tick: u64,
    pub verticality_used: bool,
    pub next_stroke_id: u8,
    /// Accepted strokes still telegraphed or acting.
    pub strokes: Vec<Stroke>,
    /// Runners of the round.
    pub runners: BTreeSet<PlayerId>,
    /// Runners through the exit this round.
    pub finished: BTreeSet<PlayerId>,
    /// Points each player scored as the architect.
    pub architect_points: BTreeMap<PlayerId, i32>,
    pub history: Vec<RoundResult>,
}

impl ArchitectState {
    /// Budget in whole points.
    pub fn budget_points(&self) -> u32 {
        self.budget / BUDGET_UNIT
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u8(0xa7);
        h.write_u8(self.rounds);
        h.write_u8(self.round);
        h.write_u8(self.phase as u8);
        h.write_u64(self.round_start_tick);
        h.write_u64(self.hard_end_tick);
        h.write_u16(self.architect.map_or(u16::MAX, |p| p.0));
        h.write_u32(self.budget);
        h.write_u64(self.next_stroke_tick);
        h.write_bool(self.verticality_used);
        h.write_u8(self.next_stroke_id);
        h.write_u32(self.strokes.len() as u32);
        for s in &self.strokes {
            h.write_u8(s.id);
            h.write_u8(s.kind.index());
            h.write_vec3(s.center);
            h.write_f32(s.radius);
            h.write_u16(s.architect.0);
            h.write_u64(s.apply_tick);
            h.write_u64(s.end_tick);
            h.write_bool(s.announced);
            h.write_bool(s.planned);
            h.write_bool(s.active);
            h.write_u32(s.edits.len() as u32);
            for e in &s.edits {
                match *e {
                    TerrainEdit::AddSphere { center, radius } => {
                        h.write_u8(1);
                        h.write_vec3(center);
                        h.write_f32(radius);
                    }
                    TerrainEdit::SubtractSphere { center, radius } => {
                        h.write_u8(0);
                        h.write_vec3(center);
                        h.write_f32(radius);
                    }
                    // Strokes emit only spheres.
                    _ => h.write_u8(0xff),
                }
            }
        }
        for set in [&self.runners, &self.finished] {
            h.write_u32(set.len() as u32);
            for p in set {
                h.write_u16(p.0);
            }
        }
        h.write_u32(self.architect_points.len() as u32);
        for (p, v) in &self.architect_points {
            h.write_u16(p.0);
            h.write_u32(*v as u32);
        }
        h.write_u32(self.history.len() as u32);
        for r in &self.history {
            h.write_u16(r.architect.map_or(u16::MAX, |p| p.0));
            h.write_u8(r.runners);
            h.write_u8(r.finished);
        }
    }
}

/// The mode's settings (defaults outside the mode).
pub fn rules(r: &ModeRules) -> TheArchitectRules {
    match r {
        ModeRules::TheArchitect(a) => *a,
        _ => TheArchitectRules::default(),
    }
}

/// Ticks of a round.
pub fn round_ticks(r: &TheArchitectRules) -> u64 {
    Tick::ticks_for_secs(r.round_secs).max(1)
}

/// Ticks of the results between rounds.
pub fn results_ticks(r: &TheArchitectRules) -> u64 {
    Tick::ticks_for_secs(r.round_results_secs)
}

/// Budget units gained per tick (`budget_per_sec` is a multiple of 0.25).
pub fn budget_per_tick(r: &TheArchitectRules) -> u32 {
    let per_sec = if r.budget_per_sec.is_finite() {
        r.budget_per_sec.max(0.0)
    } else {
        0.0
    };
    ls_math::det::round(per_sec * (BUDGET_UNIT as f32 / ls_math::TICK_HZ as f32)) as u32
}

/// The players eligible for the architect role and as runners (everyone
/// not spectating by choice), in join (id) order.
pub fn eligible(state: &ModeState) -> Vec<PlayerId> {
    state
        .players
        .iter()
        .filter(|(_, p)| p.flags & player_flags::SPECTATOR == 0)
        .map(|(id, _)| *id)
        .collect()
}

/// Round `round`'s architect among `players` (join order, wrapping); none
/// with fewer than two players.
pub fn architect_for(players: &[PlayerId], round: u8) -> Option<PlayerId> {
    if players.len() < 2 {
        return None;
    }
    Some(players[round as usize % players.len()])
}

/// Rounds of a match with `players` players.
pub fn rounds_for(players: usize, r: &TheArchitectRules) -> u8 {
    (players.min(r.max_rounds.max(1) as usize).max(1)) as u8
}

/// The exit gate of a level: the Heist site's team 1 extraction zone (an
/// open cavern at the team 1 end), else the pickup point (or spawn point)
/// farthest from the spawn points' centre, with [`EXIT_RADIUS_FALLBACK`].
pub fn exit_of(setup: &crate::MatchSetup) -> (Vec3, f32) {
    if let Some(h) = &setup.heist {
        return h.extraction[1];
    }
    let n = setup.spawns.len().max(1) as f32;
    let centre = setup.spawns.iter().fold(Vec3::ZERO, |a, s| a + s.pos) / n;
    let far = setup
        .pickup_points
        .iter()
        .copied()
        .chain(setup.spawns.iter().map(|s| s.pos))
        .fold((centre, -1.0f32), |best, p| {
            let d = p.distance_squared(centre);
            if d > best.1 {
                (p, d)
            } else {
                best
            }
        })
        .0;
    (far, EXIT_RADIUS_FALLBACK)
}

/// Starts round `state.architect.round` at `now`: roles, lives, budget,
/// clock (the world respawns the ships, see `World::step_architect`).
pub fn begin_round(state: &mut ModeState, r: &TheArchitectRules, now: u64) {
    let players = eligible(state);
    let Some(mut a) = state.architect.take() else {
        return;
    };
    let architect = architect_for(&players, a.round);
    a.architect = architect;
    a.phase = RoundPhase::Playing;
    a.round_start_tick = now;
    a.budget = r.budget_start.min(r.budget_max) as u32 * BUDGET_UNIT;
    a.next_stroke_tick = now;
    a.verticality_used = false;
    a.strokes.clear();
    a.finished.clear();
    a.runners = players
        .iter()
        .copied()
        .filter(|p| Some(*p) != architect)
        .collect();
    for (id, p) in state.players.iter_mut() {
        p.flags &= !(player_flags::ELIMINATED | player_flags::FINISHED | player_flags::ARCHITECT);
        p.lives = r.runner_lives.max(1);
        if Some(*id) == architect {
            p.flags |= player_flags::ARCHITECT;
            p.meter = a.budget_points().min(255) as u8;
        } else {
            p.meter = a
                .architect_points
                .get(id)
                .copied()
                .unwrap_or(0)
                .clamp(0, 255) as u8;
        }
    }
    state.stage = a.round;
    state.stage_end_tick = now + round_ticks(r);
    let rounds_left = (a.rounds - a.round) as u64;
    state.end_tick = (now + rounds_left * (round_ticks(r) + results_ticks(r))).min(a.hard_end_tick);
    let stage_end = state.stage_end_tick;
    if let Some(exit) = state.objective_mut(EXIT_ID) {
        exit.holder = architect;
        exit.timer_end = stage_end;
        exit.state = a.rounds << exit_state::ROUNDS_SHIFT;
    }
    state.architect = Some(a);
}

/// Edits of a reshape stroke ([`StrokeKind::reshape`]): samples the
/// current terrain and the generator with the slider moved by `delta` on a
/// [`RESHAPE_GRID`] lattice inside the sphere; where the reshaped level has
/// rock and the current one is open (growing strokes), or the other way
/// round (opening strokes), it places an edit sphere as large as the
/// reshaped field allows (at most [`EDIT_RADIUS_MAX`], kept inside the
/// stroke sphere plus 1 u; opening spheres also keep 2 u inside the
/// terrain's bounds), largest first, skipping points an earlier sphere
/// covers, at most [`MAX_STROKE_EDITS`]. Empty when the terrain cannot
/// reshape or nothing would change.
pub fn plan_reshape<T: SimTerrain + ?Sized>(
    terrain: &T,
    slider: u8,
    delta: i8,
    center: Vec3,
    radius: f32,
) -> Vec<TerrainEdit> {
    let grow = delta > 0;
    let n = ls_math::det::ceil(radius / RESHAPE_GRID) as i32;
    let mut points = Vec::new();
    for k in -n..=n {
        for j in -n..=n {
            for i in -n..=n {
                let p = center + Vec3::new(i as f32, j as f32, k as f32) * RESHAPE_GRID;
                if p.distance_squared(center) <= radius * radius {
                    points.push(p);
                }
            }
        }
    }
    let mut target = Vec::new();
    if !terrain.reshape_sample(slider, delta, &points, &mut target) || target.len() != points.len()
    {
        return Vec::new();
    }
    let b = terrain.bounds();
    let inner = ls_math::Aabb::new(b.min + Vec3::splat(2.0), b.max - Vec3::splat(2.0));
    let mut cands: Vec<(f32, usize, Vec3)> = Vec::new();
    for (idx, (&p, &t)) in points.iter().zip(&target).enumerate() {
        let now = terrain.sdf(p);
        let room = radius - p.distance(center) + 1.0;
        let r = if grow {
            if !(t < -1.0 && now > 0.5) {
                continue;
            }
            (-t + 1.0).min(EDIT_RADIUS_MAX).min(room)
        } else {
            if !(t > 1.0 && now < -0.5) {
                continue;
            }
            // Never cut through the level's sealing walls.
            let walls = -inner.sdf(p);
            t.min(EDIT_RADIUS_MAX).min(room).min(walls)
        };
        if r >= EDIT_RADIUS_MIN {
            cands.push((r, idx, p));
        }
    }
    // Largest first (ties: lattice order).
    cands.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<(Vec3, f32)> = Vec::new();
    for (r, _, p) in cands {
        if out.len() >= MAX_STROKE_EDITS {
            break;
        }
        let covered = out
            .iter()
            .any(|&(q, qr)| q.distance_squared(p) < (0.75 * qr) * (0.75 * qr));
        if !covered {
            out.push((p, r));
        }
    }
    out.into_iter()
        .map(|(center, radius)| {
            if grow {
                TerrainEdit::AddSphere { center, radius }
            } else {
                TerrainEdit::SubtractSphere { center, radius }
            }
        })
        .collect()
}

/// The Architect mode module.
#[derive(Debug)]
pub struct TheArchitect;

fn ext(state: &ModeState) -> Option<&ArchitectState> {
    state.architect.as_deref()
}

impl ModeModule for TheArchitect {
    fn kind(&self) -> ModeKind {
        ModeKind::TheArchitect
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }

    fn validate(&self, r: &ModeRules) -> Result<(), String> {
        let a = rules(r);
        if a.max_rounds == 0 {
            return Err("The Architect: max_rounds must be at least 1".into());
        }
        if a.runner_lives == 0 {
            return Err("The Architect: runner_lives must be at least 1".into());
        }
        if !(a.round_secs.is_finite() && a.round_secs >= 1.0) {
            return Err("The Architect: round_secs must be at least 1".into());
        }
        if !(a.min_radius.is_finite()
            && a.max_radius.is_finite()
            && 1.0 <= a.min_radius
            && a.min_radius <= a.max_radius)
        {
            return Err("The Architect: need 1 <= min_radius <= max_radius".into());
        }
        if a.max_strokes == 0 {
            return Err("The Architect: max_strokes must be at least 1".into());
        }
        for (name, v) in [
            ("round_results_secs", a.round_results_secs),
            ("budget_per_sec", a.budget_per_sec),
            ("stroke_cooldown_secs", a.stroke_cooldown_secs),
            ("telegraph_secs", a.telegraph_secs),
            ("keep_clear", a.keep_clear),
            ("well_secs", a.well_secs),
            ("palette_secs", a.palette_secs),
        ] {
            if !(v.is_finite() && v >= 0.0) {
                return Err(format!("The Architect: {name} must be 0 or more"));
            }
        }
        if a.reshape_delta == 0 || a.reshape_delta > 100 {
            return Err("The Architect: reshape_delta must be 1..=100".into());
        }
        Ok(())
    }

    /// Late joiners run in the current round (with its lives).
    fn on_join(&self, m: &mut ModeMut, player: PlayerId) {
        let lives = rules(m.rules).runner_lives.max(1);
        if let Some(p) = m.state.players.get_mut(&player) {
            p.lives = lives;
        }
        if let Some(a) = m.state.architect.as_deref_mut() {
            if a.phase == RoundPhase::Playing {
                a.runners.insert(player);
            }
        }
    }

    fn late_join_spectates(&self, _m: &ModeRef) -> bool {
        false
    }

    /// The exit, the rounds and round 1's roles.
    fn on_start(&self, m: &mut ModeCtx) {
        let r = rules(m.rules);
        let now = m.now();
        let (pos, radius) = exit_of(m.setup);
        let mut exit = Objective::new(EXIT_ID, objective_kind::EXIT, pos);
        exit.radius = radius;
        m.state.objectives.clear();
        m.state.objectives.push(exit);
        let players = eligible(m.state);
        m.state.architect = Some(Box::new(ArchitectState {
            rounds: rounds_for(players.len(), &r),
            round: 0,
            phase: RoundPhase::Playing,
            round_start_tick: now,
            hard_end_tick: m.state.end_tick,
            architect: None,
            budget: 0,
            next_stroke_tick: now,
            verticality_used: false,
            next_stroke_id: 0,
            strokes: Vec::new(),
            runners: BTreeSet::new(),
            finished: BTreeSet::new(),
            architect_points: BTreeMap::new(),
            history: Vec::new(),
        }));
        begin_round(m.state, &r, now);
    }

    /// Runners never hurt each other (the architect has no ship).
    fn on_damage(&self, _m: &ModeRef, hit: &mut ModeHit) {
        if hit.attacker.is_some_and(|a| a != hit.target) {
            hit.amount = 0.0;
        }
    }

    /// A death costs a life (no points either way).
    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        lose_life(m.state, kill.victim);
    }

    /// The level does not roll events here; strokes start them.
    fn allows_event(&self, _m: &ModeRef, _kind: EventKind) -> bool {
        false
    }

    fn on_tick(&self, m: &mut ModeCtx) {
        if m.state.phase != MatchPhase::Running {
            return;
        }
        let r = rules(m.rules);
        let now = m.now();
        let Some(mut a) = m.state.architect.take() else {
            return;
        };
        if a.phase == RoundPhase::Playing {
            // A second player early in round 1 of a lone start: an
            // architect after all (the round starts over).
            if a.architect.is_none()
                && a.round == 0
                && a.history.is_empty()
                && now < a.round_start_tick + ARCHITECT_GRACE_TICKS
                && eligible(m.state).len() >= 2
            {
                a.rounds = a.rounds.max(rounds_for(eligible(m.state).len(), &r));
                m.state.architect = Some(a);
                begin_round(m.state, &r, now);
                let Some(b) = m.state.architect.take() else {
                    return;
                };
                a = b;
            }
            // Budget.
            if let Some(arch) = a.architect {
                a.budget = a
                    .budget
                    .saturating_add(budget_per_tick(&r))
                    .min(r.budget_max as u32 * BUDGET_UNIT);
                if let Some(p) = m.state.players.get_mut(&arch) {
                    p.meter = a.budget_points().min(255) as u8;
                }
            }
            // Runners at the exit.
            let exit = m.state.objective(EXIT_ID).map(|o| (o.pos, o.radius));
            if let Some((pos, radius)) = exit {
                let arrived: Vec<PlayerId> = a
                    .runners
                    .iter()
                    .copied()
                    .filter(|id| !a.finished.contains(id))
                    .filter(|id| {
                        m.ctx.ship(*id).is_some_and(|s| {
                            s.alive && s.pos.distance_squared(pos) <= radius * radius
                        })
                    })
                    .collect();
                for id in arrived {
                    if let Some(p) = m.state.players.get_mut(&id) {
                        if p.is_out() {
                            continue;
                        }
                        p.flags |= player_flags::FINISHED;
                        p.score += 1;
                        a.finished.insert(id);
                    }
                }
            }
            // Players who left are no runners any more.
            a.runners.retain(|id| m.state.players.contains_key(id));
            let in_play = a
                .runners
                .iter()
                .any(|id| m.state.players.get(id).is_some_and(|p| !p.is_out()));
            if !in_play || now >= m.state.stage_end_tick {
                end_round(m.state, &mut a, &r, now);
            }
        }
        m.state.architect = Some(a);
    }

    fn end_check(&self, m: &ModeRef) -> Option<Outcome> {
        let a = ext(m.state)?;
        let last = a.round + 1 >= a.rounds;
        let done = a.phase == RoundPhase::Over && last && m.now >= m.state.stage_end_tick;
        if done || m.now >= m.state.end_tick {
            return Some(best_player_side(m));
        }
        None
    }

    fn view(&self, state: &ModeState, _rules: &ModeRules, view: &mut ModeView) {
        let Some(a) = ext(state) else {
            return;
        };
        let ended = state.phase != MatchPhase::Running;
        for p in &mut view.players {
            if ended || Some(p.player) != a.architect || p.flags & player_flags::ARCHITECT == 0 {
                p.meter = a
                    .architect_points
                    .get(&p.player)
                    .copied()
                    .unwrap_or(0)
                    .clamp(0, 255) as u8;
                if ended {
                    p.flags &= !player_flags::ARCHITECT;
                }
            }
        }
        view.flags |= view_flags::LIVES;
        for s in a.strokes.iter() {
            if view.objectives.len() >= crate::mode::MAX_OBJECTIVES {
                break;
            }
            view.objectives.push(s.objective());
        }
    }
}

/// Ends the running round at `now`: architect points for every runner who
/// did not make it, the round's result, and the results phase.
pub fn end_round(state: &mut ModeState, a: &mut ArchitectState, r: &TheArchitectRules, now: u64) {
    let runners = a.runners.len() as u8;
    let finished = a.finished.len() as u8;
    let missed = runners.saturating_sub(finished) as i32;
    if let Some(arch) = a.architect.filter(|id| state.players.contains_key(id)) {
        if missed > 0 {
            *a.architect_points.entry(arch).or_insert(0) += missed;
            if let Some(p) = state.players.get_mut(&arch) {
                p.score += missed;
            }
        }
    }
    let result = RoundResult {
        architect: a.architect,
        runners,
        finished,
    };
    a.history.push(result);
    a.phase = RoundPhase::Over;
    // More players than rounds so far: more rounds (up to the maximum).
    a.rounds = a.rounds.max(rounds_for(eligible(state).len(), r));
    state.stage_end_tick = now + results_ticks(r);
    let rounds_left = (a.rounds - a.round - 1) as u64;
    state.end_tick = (state.stage_end_tick + rounds_left * (round_ticks(r) + results_ticks(r)))
        .min(a.hard_end_tick);
    let stage_end = state.stage_end_tick;
    if let Some(exit) = state.objective_mut(EXIT_ID) {
        exit.timer_end = stage_end;
        exit.state = a.rounds << exit_state::ROUNDS_SHIFT
            | exit_state::ROUND_OVER
            | if result.runners_won() {
                exit_state::RUNNERS_WON
            } else {
                0
            };
    }
}

pub mod replay;

#[cfg(test)]
mod tests;
