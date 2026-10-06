//! Game modes: the shared types of the mode framework.
//!
//! Every mode is a module in [`crate::modes`] behind the
//! [`crate::modes::ModeModule`] hook trait; the world keeps the generic
//! [`ModeState`] (phase, timer, scores, lives, objectives) and replicates it
//! as a [`ModeView`] (`World::mode_view`). See
//! `docs/architecture/arsenal-framework.md` (Mode framework).

use std::collections::BTreeMap;

use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::ship::PlayerId;
use crate::terrain::PaintCoverage;

/// How often (ticks) Paint the Void scores are refreshed from the terrain's
/// paint coverage.
pub const SCORE_INTERVAL_TICKS: u64 = 30;
/// Most teams a match can have (team ids `0..MAX_TEAMS`).
pub const MAX_TEAMS: u8 = 4;
/// Most players a [`ModeView`] lists (the top scores).
pub const MAX_VIEW_PLAYERS: usize = 32;
/// Most objective markers a [`ModeView`] carries.
pub const MAX_OBJECTIVES: usize = 8;
/// [`PlayerModeView::lives`] / [`PlayerMode::lives`] value for "no lives limit".
pub const LIVES_UNLIMITED: u8 = u8::MAX;
/// Highest side id a free-for-all ship gets (side ids are `0..=MAX_FFA_SIDE`;
/// 254 and 255 stay free for "no team" markers).
pub const MAX_FFA_SIDE: u8 = 253;

/// The game modes, in rule set code order (append new modes at the end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ModeKind {
    PaintTheVoid = 0,
    TeamBrawl = 1,
    TheSqueeze = 2,
    Heist = 3,
    HotReactor = 4,
    LastColorStanding = 5,
    /// Belt Up (M4 §3.8). Index 7: 6 is The Architect's (package B4).
    BeltUp = 7,
}

impl ModeKind {
    /// Every mode in index order (indices may have gaps while a mode is
    /// being built in parallel: look modes up by [`ModeKind::index`], not
    /// by position).
    pub const ALL: [ModeKind; 7] = [
        ModeKind::PaintTheVoid,
        ModeKind::TeamBrawl,
        ModeKind::TheSqueeze,
        ModeKind::Heist,
        ModeKind::HotReactor,
        ModeKind::LastColorStanding,
        ModeKind::BeltUp,
    ];

    /// Stable index (rule set codes, wire).
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Option<ModeKind> {
        ModeKind::ALL.iter().copied().find(|k| k.index() == i)
    }

    /// Module id (`crates/ls-sim/src/modes/<id>.rs`, `data/rulesets/<id>.ron`).
    pub fn id(self) -> &'static str {
        match self {
            ModeKind::PaintTheVoid => "paint_the_void",
            ModeKind::TeamBrawl => "team_brawl",
            ModeKind::TheSqueeze => "the_squeeze",
            ModeKind::Heist => "heist",
            ModeKind::HotReactor => "hot_reactor",
            ModeKind::LastColorStanding => "last_color_standing",
            ModeKind::BeltUp => "belt_up",
        }
    }

    /// The mode's default rule set file in `data/` (`BUILTIN_RULESETS` key).
    pub fn ruleset_file(self) -> &'static str {
        match self {
            ModeKind::PaintTheVoid => "rulesets/paint_the_void.ron",
            ModeKind::TeamBrawl => "rulesets/team_brawl.ron",
            ModeKind::TheSqueeze => "rulesets/squeeze.ron",
            ModeKind::Heist => "rulesets/heist.ron",
            ModeKind::HotReactor => "rulesets/hot_reactor.ron",
            ModeKind::LastColorStanding => "rulesets/last_color_standing.ron",
            ModeKind::BeltUp => "rulesets/belt_up.ron",
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            ModeKind::PaintTheVoid => "Paint the Void",
            ModeKind::TeamBrawl => "Team Brawl",
            ModeKind::TheSqueeze => "The Squeeze",
            ModeKind::Heist => "Heist",
            ModeKind::HotReactor => "Hot Reactor",
            ModeKind::LastColorStanding => "Last Color Standing",
            ModeKind::BeltUp => "Belt Up",
        }
    }
}

/// Match phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchPhase {
    Running,
    /// The match is over. `winner` is the winning side: a team index in team
    /// modes, the winning ship's side id (`ShipState::team`) in free-for-all;
    /// `None` on a draw.
    Ended {
        winner: Option<u8>,
    },
}

/// Objective marker kinds ([`Objective::kind`], [`ObjectiveView::kind`]).
/// Clients draw markers by kind; values are stable (wire).
pub mod objective_kind {
    /// The Squeeze: the current safe sphere (`pos`, `radius`).
    pub const SAFE_ZONE: u8 = 0;
    /// The Squeeze: the next safe sphere (shown during the warning).
    pub const NEXT_ZONE: u8 = 1;
    /// Heist: the core (`holder` = carrier).
    pub const CORE: u8 = 2;
    /// Heist: an extraction zone (`team` = owning team or `NO_SIDE`).
    pub const EXTRACTION: u8 = 3;
    /// Hot Reactor: the reactor (`holder`, `timer_end` = detonation tick).
    pub const REACTOR: u8 = 4;
    /// Free for modules.
    pub const CUSTOM: u8 = 255;
}

/// [`Objective::team`] / [`ObjectiveView::team`] for "no team".
pub const NO_SIDE: u8 = u8::MAX;

/// [`PlayerModeView::flags`] / [`PlayerMode::flags`] bits.
pub mod player_flags {
    /// Out of the match (no lives or colour left); spectates.
    pub const ELIMINATED: u8 = 1 << 0;
    /// Spectating without playing (late join in an elimination mode, or by
    /// choice).
    pub const SPECTATOR: u8 = 1 << 1;
    /// Carries an objective (Heist core, Hot Reactor).
    pub const CARRIER: u8 = 1 << 2;
    /// Held in place (Belt Up: start lights, the death hold at a gate, heat
    /// results): the mode's `move_limits` stop the ship (bit 3 is The
    /// Architect's on the wire).
    pub const HELD: u8 = 1 << 4;
}

/// [`ModeView::flags`] bits: how clients present the mode.
pub mod view_flags {
    /// Instagib mutator: the HUD hides hull and energy.
    pub const INSTAGIB: u16 = 1 << 0;
    /// Monochrome level (Last Color Standing).
    pub const MONOCHROME: u16 = 1 << 1;
    /// Scores are paint coverage (Paint the Void): `team_scores` are cells,
    /// `score_total` the paintable surface.
    pub const PAINT_SCORES: u16 = 1 << 2;
    /// Players have limited lives (`PlayerModeView::lives` matters).
    pub const LIVES: u16 = 1 << 3;
    /// Players have a colour meter (`PlayerModeView::meter` matters).
    pub const METER: u16 = 1 << 4;
}

/// Mode state of one player (kept by the world, replicated in the view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerMode {
    /// Mode score (kills, deliveries, ...).
    pub score: i32,
    /// Lives left ([`LIVES_UNLIMITED`] = no limit).
    pub lives: u8,
    /// Mode meter (Last Color Standing colour).
    pub meter: u8,
    /// [`player_flags`] bits.
    pub flags: u8,
}

impl Default for PlayerMode {
    fn default() -> Self {
        PlayerMode {
            score: 0,
            lives: LIVES_UNLIMITED,
            meter: 0,
            flags: 0,
        }
    }
}

impl PlayerMode {
    /// Eliminated or spectating: the ship does not respawn.
    pub fn is_out(&self) -> bool {
        self.flags & (player_flags::ELIMINATED | player_flags::SPECTATOR) != 0
    }
}

/// A mode objective (zone, core, reactor, extraction point): generic state
/// for mode modules.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    /// Module-chosen id, unique among the match's objectives.
    pub id: u8,
    /// See [`objective_kind`].
    pub kind: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    pub radius: f32,
    /// Ship carrying it.
    pub holder: Option<PlayerId>,
    /// Tick of the objective's timer (detonation, return, ...; 0 = none).
    pub timer_end: u64,
    /// Owning team ([`NO_SIDE`] = none).
    pub team: u8,
    /// Module-defined state byte (replicated).
    pub state: u8,
    /// Module-defined scratch values (not replicated).
    pub vars: [f32; 4],
}

impl Objective {
    pub fn new(id: u8, kind: u8, pos: Vec3) -> Objective {
        Objective {
            id,
            kind,
            pos,
            vel: Vec3::ZERO,
            radius: 0.0,
            holder: None,
            timer_end: 0,
            team: NO_SIDE,
            state: 0,
            vars: [0.0; 4],
        }
    }

    /// The replicated part.
    pub fn view(&self) -> ObjectiveView {
        ObjectiveView {
            id: self.id,
            kind: self.kind,
            pos: self.pos,
            radius: self.radius,
            holder: self.holder,
            timer_end: self.timer_end,
            team: self.team,
            state: self.state,
        }
    }
}

/// The generic mode state of a match (in `World`, part of the state hash).
/// Mode modules read and change it through `modes::ModeCtx`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeState {
    pub kind: ModeKind,
    /// Number of teams (0 = free-for-all).
    pub teams: u8,
    pub phase: MatchPhase,
    /// Tick at which the match timer runs out.
    pub end_tick: u64,
    /// Mode-specific stage (Squeeze phase, ...; 0 = the first).
    pub stage: u8,
    /// Tick at which the current stage ends (0 = none).
    pub stage_end_tick: u64,
    /// Latest paint coverage (Paint the Void).
    pub coverage: PaintCoverage,
    /// Team scores (team modes; index = team).
    pub team_scores: [i64; MAX_TEAMS as usize],
    /// Score that wins (kill target, deliveries; 0 = none).
    pub score_target: u32,
    /// Per-player state (every player in the match).
    pub players: BTreeMap<PlayerId, PlayerMode>,
    /// Objectives; the first [`MAX_OBJECTIVES`] are replicated.
    pub objectives: Vec<Objective>,
    /// Module-defined scratch values (not replicated).
    pub vars: [i64; 4],
    /// [`view_flags`] bits.
    pub flags: u16,
    /// Belt Up's race (heats, racers, gates; `None` in other modes).
    #[serde(default)]
    pub race: Option<crate::modes::belt_up::RaceState>,
}

impl ModeState {
    pub fn new(kind: ModeKind, teams: u8, match_ticks: u64) -> ModeState {
        ModeState {
            kind,
            teams,
            phase: MatchPhase::Running,
            end_tick: match_ticks,
            stage: 0,
            stage_end_tick: 0,
            coverage: PaintCoverage::default(),
            team_scores: [0; MAX_TEAMS as usize],
            score_target: 0,
            players: BTreeMap::new(),
            objectives: Vec::new(),
            vars: [0; 4],
            flags: 0,
            race: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.phase == MatchPhase::Running
    }

    /// Whether every ship is its own side.
    pub fn free_for_all(&self) -> bool {
        self.teams == 0
    }

    /// Ticks left on the match timer at `tick`.
    pub fn remaining_ticks(&self, tick: u64) -> u64 {
        self.end_tick.saturating_sub(tick)
    }

    /// Paint the Void: team score in percent of the painted surface.
    pub fn score_percent(&self, team: u8) -> f32 {
        self.coverage.percent(team)
    }

    /// Paint the Void: the team with the most painted cells (integer
    /// comparison, so it is exact); `None` on a tie for the lead.
    pub fn leader(&self) -> Option<u8> {
        let n = self.teams.clamp(2, MAX_TEAMS) as usize;
        best_index(&self.coverage.team_cells[..n])
    }

    /// The team with the highest [`ModeState::team_scores`]; `None` on a
    /// tie for the lead (or in free-for-all).
    pub fn team_score_leader(&self) -> Option<u8> {
        let n = self.teams.min(MAX_TEAMS) as usize;
        best_index(&self.team_scores[..n])
    }

    pub fn player(&self, id: PlayerId) -> Option<&PlayerMode> {
        self.players.get(&id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut PlayerMode> {
        self.players.get_mut(&id)
    }

    pub fn objective(&self, id: u8) -> Option<&Objective> {
        self.objectives.iter().find(|o| o.id == id)
    }

    pub fn objective_mut(&mut self, id: u8) -> Option<&mut Objective> {
        self.objectives.iter_mut().find(|o| o.id == id)
    }
}

/// Index of the unique maximum; `None` if empty or tied.
fn best_index<T: Ord + Copy>(v: &[T]) -> Option<u8> {
    let max = *v.iter().max()?;
    let mut it = v.iter().enumerate().filter(|(_, x)| **x == max);
    let (i, _) = it.next()?;
    if it.next().is_some() {
        None
    } else {
        Some(i as u8)
    }
}

/// One player row of a [`ModeView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerModeView {
    pub player: PlayerId,
    /// The player's side: team index, or its own side id in free-for-all.
    pub side: u8,
    pub score: i32,
    /// Lives left ([`LIVES_UNLIMITED`] = no limit).
    pub lives: u8,
    /// Mode meter (Last Color Standing colour).
    pub meter: u8,
    /// [`player_flags`] bits.
    pub flags: u8,
}

/// One objective marker of a [`ModeView`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ObjectiveView {
    pub id: u8,
    /// See [`objective_kind`].
    pub kind: u8,
    pub pos: Vec3,
    pub radius: f32,
    pub holder: Option<PlayerId>,
    /// Timer tick (0 = none).
    pub timer_end: u64,
    /// Owning team ([`NO_SIDE`] = none).
    pub team: u8,
    pub state: u8,
}

/// The replicated mode state (protocol 5 `ModeState` item): everything a
/// client needs for the scoreboard, the mode HUD, objective markers and the
/// results screen. Built by `World::mode_view`; equal views mean nothing
/// changed (send on change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeView {
    pub kind: ModeKind,
    /// 0 = free-for-all, else 2-4.
    pub teams: u8,
    pub phase: MatchPhase,
    /// Tick at which the match timer runs out.
    pub end_tick: u64,
    /// Mode-specific stage (Squeeze phase, ...).
    pub stage: u8,
    /// Tick the stage ends (0 = none).
    pub stage_end_tick: u64,
    /// One score per team (`teams` entries; empty in free-for-all). Paint
    /// the Void: painted cells.
    pub team_scores: Vec<i64>,
    /// Paint the Void: paintable surface cells (0 otherwise).
    pub score_total: u64,
    /// Score that wins (kill target, deliveries; 0 = none).
    pub score_target: u32,
    /// Players by score (highest first, then by id), at most
    /// [`MAX_VIEW_PLAYERS`].
    pub players: Vec<PlayerModeView>,
    /// At most [`MAX_OBJECTIVES`] markers.
    pub objectives: Vec<ObjectiveView>,
    /// [`view_flags`] bits.
    pub flags: u16,
    /// The comedy director: mood and running event (protocol 6).
    #[serde(default)]
    pub mood: crate::director::MoodView,
    /// Belt Up's race (protocol 6 race section; `None` in other modes).
    #[serde(default)]
    pub race: Option<crate::modes::belt_up::RaceView>,
}

impl ModeView {
    pub fn is_running(&self) -> bool {
        self.phase == MatchPhase::Running
    }

    /// Ticks left on the match timer at `tick`.
    pub fn remaining_ticks(&self, tick: u64) -> u64 {
        self.end_tick.saturating_sub(tick)
    }

    /// The winner's player row in free-for-all (`None` in team modes, while
    /// running, or on a draw).
    pub fn winner_player(&self) -> Option<&PlayerModeView> {
        match self.phase {
            MatchPhase::Ended { winner: Some(side) } if self.teams == 0 => {
                self.players.iter().find(|p| p.side == side)
            }
            _ => None,
        }
    }
}

/// One row of the results table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultRow {
    pub player: PlayerId,
    /// Team index, or the player's side id in free-for-all.
    pub side: u8,
    pub kills: u32,
    pub deaths: u32,
    /// Mode score.
    pub score: i32,
}

/// The result of a match (`World::results`): winner (side) or draw, and
/// one row per player, best first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchResults {
    pub kind: ModeKind,
    pub teams: u8,
    /// Whether the match has ended.
    pub ended: bool,
    /// Winning side; `None` while running or on a draw.
    pub winner: Option<u8>,
    /// One per team (empty in free-for-all).
    pub team_scores: Vec<i64>,
    pub rows: Vec<ResultRow>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn leader_and_percent() {
        let mut m = ModeState::new(ModeKind::PaintTheVoid, 2, 100);
        assert_eq!(m.leader(), None);
        m.coverage = PaintCoverage {
            team_cells: [10, 30, 0, 0],
            surface_cells: 200,
        };
        assert_eq!(m.leader(), Some(1));
        assert_eq!(m.score_percent(1), 15.0);
        assert_eq!(m.remaining_ticks(40), 60);
        assert_eq!(m.remaining_ticks(400), 0);
        // Four teams: a third team can lead; a tie for the lead is a draw.
        m.teams = 4;
        m.coverage.team_cells = [10, 30, 31, 0];
        assert_eq!(m.leader(), Some(2));
        m.coverage.team_cells = [10, 31, 31, 0];
        assert_eq!(m.leader(), None);
        m.team_scores = [3, 1, 0, 3];
        assert_eq!(m.team_score_leader(), None);
        m.team_scores[3] = 4;
        assert_eq!(m.team_score_leader(), Some(3));
    }

    #[test]
    fn kinds_round_trip() {
        for k in ModeKind::ALL {
            assert_eq!(ModeKind::from_index(k.index()), Some(k));
        }
        assert_eq!(ModeKind::from_index(6), None);
        assert_eq!(ModeKind::from_index(7), Some(ModeKind::BeltUp));
        assert_eq!(ModeKind::from_index(8), None);
    }
}
