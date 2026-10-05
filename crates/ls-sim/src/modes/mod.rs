//! Mode modules: one file per game mode (`modes/<id>.rs`) behind
//! [`ModeModule`], registered in [`MODULES`] (the order is
//! [`ModeKind`]'s, which is the rule set code's mode index).
//!
//! The world ([`crate::World`]) keeps the generic [`ModeState`] (phase,
//! timer, team and player scores, lives, objectives) and calls the active
//! mode's hooks, then the active mutators' ([`crate::mutators`]):
//!
//! | Hook | When | Context |
//! | --- | --- | --- |
//! | `validate` | `RuleSet::resolve` | the mode settings |
//! | `on_join` | a player joins (before the first spawn) | [`ModeMut`] |
//! | `late_join_spectates` | a player joins: spectate until the next match? | [`ModeRef`] |
//! | `on_start` | first tick of the match, before anything moves | [`ModeCtx`] (commands) |
//! | `allow_respawn` | a dead ship's respawn timer ran out | [`ModeRef`] |
//! | `on_spawn` | a ship (re)spawned with its loadout | [`ModeMut`] + the ship |
//! | `on_damage` | a ship takes damage (after friendly fire, before gadgets) | [`ModeRef`] + [`ModeHit`] |
//! | `on_hit` | right after `on_damage` (and the mutators'): the final hit, before gadgets absorb it | [`ModeMut`] + [`ModeHit`] |
//! | `on_kill` | a ship died | [`ModeMut`] + [`Kill`] |
//! | `on_pickup` | a ship touches a pickup: may it take it? | [`ModeState`] + the ship |
//! | `on_tick` | every tick, last stage (scores, timers, objectives) | [`ModeCtx`] (commands) |
//! | `end_check` | after `on_tick`: has the match ended (and who won)? | [`ModeRef`] |
//! | `view` | `World::mode_view`: mode-specific view fields | [`ModeState`] |
//! | `move_limits` | a ship moves: speed cap scale, boost off (Heist carrier) | the player's replicated mode flags |
//! | `material_at` | a hook asks for the rock strength (craters): override it in a region | [`ModeState`] + the point |
//! | `terrain_shell` | start of every tick: the solid shell the terrain adds (The Squeeze) | [`ModeState`] + tick |
//! | `allows_event` | the comedy director rolls: may this event run now? (vetoes) | [`ModeRef`] + the event |
//! | `leader` | the director's target for leader events (Tax drone, Customer survey) | [`ModeRef`] |
//!
//! A module that does not override a hook gets the generic behaviour of
//! this file ("generic fallback", like M2's weapon stubs): kills score for
//! the killer (and their team), a suicide or team kill costs one point,
//! modes with lives lose one per death and eliminate at zero, the match
//! ends when one side is left (lives modes), when a side reaches
//! `score_target`, or on the timer (best score wins, a tie is a draw).
//!
//! Hooks with [`ModeCtx`] queue world changes through [`Ctx`] (damage,
//! debris, terrain edits, cues), exactly like weapon hooks. All hooks
//! change [`ModeState`] directly. Determinism rules apply (`CLAUDE.md`).

pub mod heist;
pub mod hot_reactor;
pub mod last_color_standing;
pub mod paint_the_void;
pub mod team_brawl;
pub mod the_squeeze;

use std::collections::BTreeMap;
use std::fmt::Debug;

use ls_math::{Tick, Vec3};

use crate::behavior::Ctx;
use crate::events::EventKind;
use crate::mode::{
    player_flags, MatchPhase, ModeKind, ModeState, ModeView, PlayerMode, LIVES_UNLIMITED,
};
use crate::ruleset::{ModeRules, Mutator, PickupKind};
use crate::ship::{PlayerId, ShipState};
use crate::terrain::PaintCoverage;
use crate::world::MatchSetup;
use crate::{Buttons, InputCmd, ShipTuning, WeaponId};

/// Players joining later than this after the match start spectate until the
/// next match in modes with lives (unless the module says otherwise).
pub const LATE_JOIN_GRACE_TICKS: u64 = 10 * ls_math::TICK_HZ as u64;

/// Read-only mode context: the state, the settings and the ships.
#[derive(Clone, Copy)]
pub struct ModeRef<'a> {
    pub state: &'a ModeState,
    pub rules: &'a ModeRules,
    pub mutators: &'a [Mutator],
    pub ships: &'a BTreeMap<PlayerId, ShipState>,
    /// Current tick.
    pub now: u64,
}

impl Debug for ModeRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModeRef").field("now", &self.now).finish()
    }
}

/// Mode context with a mutable state (hooks without world commands).
pub struct ModeMut<'a> {
    pub state: &'a mut ModeState,
    pub rules: &'a ModeRules,
    pub mutators: &'a [Mutator],
    pub ships: &'a BTreeMap<PlayerId, ShipState>,
    /// Current tick.
    pub now: u64,
}

impl ModeMut<'_> {
    /// The read-only view of this context.
    pub fn as_ref(&self) -> ModeRef<'_> {
        ModeRef {
            state: self.state,
            rules: self.rules,
            mutators: self.mutators,
            ships: self.ships,
            now: self.now,
        }
    }
}

/// Full mode context: the world view and command queue ([`Ctx`]) plus the
/// mutable mode state.
pub struct ModeCtx<'a, 'c> {
    pub ctx: &'c mut Ctx<'a>,
    pub state: &'c mut ModeState,
    pub rules: &'c ModeRules,
    pub mutators: &'c [Mutator],
    /// Spawn points, pickup points and sliders of the level.
    pub setup: &'c MatchSetup,
    pub(crate) coverage: &'c dyn Fn() -> Option<PaintCoverage>,
}

impl ModeCtx<'_, '_> {
    /// Current tick.
    pub fn now(&self) -> u64 {
        self.ctx.now()
    }

    /// The terrain's paint coverage (counts cells; call it sparingly).
    pub fn paint_coverage(&self) -> Option<PaintCoverage> {
        (self.coverage)()
    }

    /// Marks a player eliminated (no respawn; a living ship leaves play at
    /// the end of the tick with an `Event::Eliminated`).
    pub fn eliminate(&mut self, player: PlayerId) {
        if let Some(p) = self.state.players.get_mut(&player) {
            p.flags |= player_flags::ELIMINATED;
        }
    }
}

/// A ship's death as mode hooks see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kill {
    pub victim: PlayerId,
    /// The victim's side (team, or its own side in free-for-all).
    pub victim_side: u8,
    /// Who gets the credit (`None`: environment or nobody).
    pub killer: Option<PlayerId>,
    /// The killer's side, if there is a killer.
    pub killer_side: Option<u8>,
    pub weapon: Option<WeaponId>,
}

impl Kill {
    /// Killed by itself (or by nobody).
    pub fn is_suicide(&self) -> bool {
        self.killer.is_none_or(|k| k == self.victim)
    }

    /// Killed by a teammate.
    pub fn is_team_kill(&self) -> bool {
        !self.is_suicide() && self.killer_side == Some(self.victim_side)
    }
}

/// Damage to a ship as `on_damage` hooks see it; hooks may change `amount`
/// and `ignore_gadget`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModeHit {
    pub target: PlayerId,
    pub attacker: Option<PlayerId>,
    pub weapon: Option<WeaponId>,
    /// Damage after friendly-fire scaling; hooks may change it.
    pub amount: f32,
    /// Direct hit (as opposed to splash or area damage).
    pub direct: bool,
    pub self_inflicted: bool,
    /// Attacker and target are on the same side (and not the same ship).
    pub same_side: bool,
    /// Skip the target's gadget (shield) for this hit.
    pub ignore_gadget: bool,
}

/// How a match ended: the winning side, or `None` for a draw.
pub type Outcome = Option<u8>;

/// Movement limits a mode puts on one ship ([`ModeModule::move_limits`];
/// the Heist carrier). Clients predicting their own ship apply the same
/// limits ([`MoveLimits::apply`]) from the replicated player flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveLimits {
    /// Scale of the cruise and boost speed caps (1 = unchanged).
    pub speed_scale: f32,
    /// The ship cannot boost.
    pub no_boost: bool,
}

impl MoveLimits {
    /// No limits.
    pub const NONE: MoveLimits = MoveLimits {
        speed_scale: 1.0,
        no_boost: false,
    };

    pub fn is_none(&self) -> bool {
        *self == MoveLimits::NONE
    }

    /// The tuning and command a limited ship moves with: both speed caps
    /// scaled, and without boost the boost button released.
    pub fn apply(&self, tuning: &ShipTuning, cmd: &InputCmd) -> (ShipTuning, InputCmd) {
        let mut t = tuning.clone();
        t.cruise_speed *= self.speed_scale;
        t.boost_speed *= self.speed_scale;
        let mut c = *cmd;
        if self.no_boost {
            c.buttons.set(Buttons::BOOST, false);
        }
        (t, c)
    }
}

/// The hooks of one game mode (see the module docs for when each runs).
pub trait ModeModule: Send + Sync + Debug {
    fn kind(&self) -> ModeKind;

    /// Whether the module implements its mode's rules (false: generic
    /// fallback, a stub).
    fn implemented(&self) -> bool {
        false
    }

    /// Mode-specific validation of the settings (`RuleSet::resolve`); the
    /// framework already checked teams, match and respawn times.
    fn validate(&self, _rules: &ModeRules) -> Result<(), String> {
        Ok(())
    }

    /// Supported team counts (0 = free-for-all).
    fn team_counts(&self) -> &'static [u8] {
        &[0, 2, 3, 4]
    }

    /// A player joined: set up their [`PlayerMode`] (the framework inserted
    /// a default one with the mode's lives).
    fn on_join(&self, _m: &mut ModeMut, _player: PlayerId) {}

    /// Whether a player joining now spectates until the next match.
    fn late_join_spectates(&self, m: &ModeRef) -> bool {
        m.rules.lives() > 0 && m.now > LATE_JOIN_GRACE_TICKS
    }

    /// The match starts (first tick).
    fn on_start(&self, _m: &mut ModeCtx) {}

    /// Whether a dead player may respawn now (eliminated and spectating
    /// players never do; the framework checks that first).
    fn allow_respawn(&self, _m: &ModeRef, _player: PlayerId) -> bool {
        true
    }

    /// A ship (re)spawned (loadout given, at its spawn point).
    fn on_spawn(&self, _m: &mut ModeMut, _ship: &mut ShipState) {}

    /// A ship takes damage.
    fn on_damage(&self, _m: &ModeRef, _hit: &mut ModeHit) {}

    /// A ship was hit: the hit as `on_damage` hooks (mode, then mutators)
    /// left it, before the target's gadget (shield) absorbs any of it. For
    /// modes that react to hits (Hot Reactor passes the reactor to the
    /// shooter).
    fn on_hit(&self, _m: &mut ModeMut, _hit: &ModeHit) {}

    /// A ship died (deaths and kills on the ships are already counted).
    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        generic_kill(m, kill);
    }

    /// Whether `ship` may take a pickup of `kind` (no ship map here: the
    /// world iterates the ships mutably while collecting).
    fn on_pickup(
        &self,
        _state: &ModeState,
        _rules: &ModeRules,
        _ship: &ShipState,
        _kind: &PickupKind,
    ) -> bool {
        true
    }

    /// Every tick, after everything else moved.
    fn on_tick(&self, _m: &mut ModeCtx) {}

    /// `Some(outcome)` once the match is over.
    fn end_check(&self, m: &ModeRef) -> Option<Outcome> {
        generic_end(m)
    }

    /// Mode-specific fields of the replicated view (the framework filled
    /// the generic ones).
    fn view(&self, _state: &ModeState, _rules: &ModeRules, _view: &mut ModeView) {}

    /// Movement limits of a ship whose player has the mode flags `flags`
    /// ([`crate::mode::player_flags`]; none by default). Only the replicated
    /// flags, so a client predicting its own ship gets the same answer.
    fn move_limits(&self, _rules: &ModeRules, _flags: u8) -> MoveLimits {
        MoveLimits::NONE
    }

    /// Rock material id (`ls_math::terrain::MATERIAL_*`) the mode imposes at
    /// `p` at tick `now`, overriding the terrain's (`None`: the terrain's).
    /// Read wherever hooks ask for the material (`Ctx::material_at`, and so
    /// `Ctx::crater`); during the mode's own hooks the state is a
    /// placeholder, so the override does not apply there.
    fn material_at(
        &self,
        _state: &ModeState,
        _rules: &ModeRules,
        _now: u64,
        _p: Vec3,
    ) -> Option<u8> {
        None
    }

    /// Scale of the radiation dose on belt levels (M4; default 1, 0 turns
    /// it off; Belt Up's rule set setting, its start lights).
    fn radiation_scale(&self, _m: &ModeRef) -> f32 {
        1.0
    }

    /// A pigment pickup touched by `player` (M4): `true` if the mode takes
    /// it instead of the pigmented state (Last Color Standing restores
    /// colour); default `false`.
    fn on_pigment(&self, _m: &mut ModeMut, _player: PlayerId, _colour: u8) -> bool {
        false
    }

    /// The solid shell the mode adds to the terrain at tick `now` (The
    /// Squeeze's walls); the world hands it to the terrain at the start of
    /// every tick ([`crate::SimTerrain::set_shell`]). A pure function of the
    /// state and the tick, so clients derive the same shell from the
    /// replicated view.
    fn terrain_shell(&self, _state: &ModeState, _now: u64) -> Option<crate::ShellSphere> {
        None
    }

    /// Whether the comedy director may start `kind` now (vetoes; every
    /// event by default). Checked at each roll, not for forced events.
    fn allows_event(&self, _m: &ModeRef, _kind: EventKind) -> bool {
        true
    }

    /// The player leader events aim at (Tax drone, Customer survey), or
    /// `None` without a clear leader ([`default_leader`]).
    fn leader(&self, m: &ModeRef) -> Option<PlayerId> {
        default_leader(m)
    }
}

/// One registered mode.
#[derive(Debug, Clone, Copy)]
pub struct ModeEntry {
    pub kind: ModeKind,
    pub module: &'static dyn ModeModule,
}

/// Every mode, in [`ModeKind`] order.
pub static MODULES: [ModeEntry; 6] = [
    ModeEntry {
        kind: ModeKind::PaintTheVoid,
        module: &paint_the_void::PaintTheVoid,
    },
    ModeEntry {
        kind: ModeKind::TeamBrawl,
        module: &team_brawl::TeamBrawl,
    },
    ModeEntry {
        kind: ModeKind::TheSqueeze,
        module: &the_squeeze::TheSqueeze,
    },
    ModeEntry {
        kind: ModeKind::Heist,
        module: &heist::Heist,
    },
    ModeEntry {
        kind: ModeKind::HotReactor,
        module: &hot_reactor::HotReactor,
    },
    ModeEntry {
        kind: ModeKind::LastColorStanding,
        module: &last_color_standing::LastColorStanding,
    },
];

/// The module of a mode.
pub fn module(kind: ModeKind) -> &'static dyn ModeModule {
    MODULES[kind.index() as usize].module
}

// ----- generic fallback ------------------------------------------------------

/// The side ids that still have a player in play (not eliminated, not a
/// pure spectator), and how many sides ever played.
pub fn sides_in_play(m: &ModeRef) -> (Vec<u8>, usize) {
    let mut alive = Vec::new();
    let mut played = Vec::new();
    for (id, p) in &m.state.players {
        let Some(ship) = m.ships.get(id) else {
            continue;
        };
        if p.flags & player_flags::SPECTATOR != 0 {
            continue;
        }
        if !played.contains(&ship.team) {
            played.push(ship.team);
        }
        if p.flags & player_flags::ELIMINATED == 0 && !alive.contains(&ship.team) {
            alive.push(ship.team);
        }
    }
    alive.sort_unstable();
    (alive, played.len())
}

/// The best player's side by mode score in free-for-all; `None` on a tie
/// (or without players).
pub fn best_player_side(m: &ModeRef) -> Option<u8> {
    let mut best: Option<(i32, u8)> = None;
    let mut tied = false;
    for (id, p) in &m.state.players {
        let Some(ship) = m.ships.get(id) else {
            continue;
        };
        if p.flags & player_flags::SPECTATOR != 0 {
            continue;
        }
        match best {
            Some((s, _)) if p.score < s => {}
            Some((s, _)) if p.score == s => tied = true,
            _ => {
                best = Some((p.score, ship.team));
                tied = false;
            }
        }
    }
    if tied {
        None
    } else {
        best.map(|(_, side)| side)
    }
}

/// The default leader: in team modes the best player (most mode score,
/// ties to the lower id) of the side with the most team score (ties to the
/// lower team); in free-for-all the best player. `None` until the leading
/// score is above 0, and for players out of play or without a ship.
pub fn default_leader(m: &ModeRef) -> Option<PlayerId> {
    let playing = |id: &PlayerId, p: &PlayerMode| !p.is_out() && m.ships.contains_key(id);
    let side = if m.state.teams > 0 {
        let n = m.state.teams.min(crate::mode::MAX_TEAMS) as usize;
        let scores = &m.state.team_scores[..n];
        let best = *scores.iter().max()?;
        if best <= 0 {
            return None;
        }
        Some(scores.iter().position(|&s| s == best)? as u8)
    } else {
        None
    };
    let mut best: Option<(i32, PlayerId)> = None;
    for (id, p) in &m.state.players {
        if !playing(id, p) || side.is_some_and(|t| m.ships[id].team != t) {
            continue;
        }
        if best.is_none_or(|(s, _)| p.score > s) {
            best = Some((p.score, *id));
        }
    }
    match best {
        Some((s, id)) if s > 0 || side.is_some() => Some(id),
        _ => None,
    }
}

/// Adds `points` to a player's score and (team modes) its team's score.
pub fn add_score(m: &mut ModeMut, player: PlayerId, points: i32) {
    let side = m.ships.get(&player).map(|s| s.team);
    if let Some(p) = m.state.players.get_mut(&player) {
        p.score += points;
    }
    if let Some(t) = side.filter(|t| (*t as usize) < m.state.teams as usize) {
        m.state.team_scores[t as usize] += points as i64;
    }
}

/// Generic kill handling: the killer scores 1 (its team too), a suicide or
/// team kill costs the victim's (or killer's) side one point; with lives,
/// the victim loses one and is eliminated at zero.
pub fn generic_kill(m: &mut ModeMut, kill: &Kill) {
    if kill.is_suicide() {
        add_score(m, kill.victim, -1);
    } else if let Some(k) = kill.killer {
        if kill.is_team_kill() {
            add_score(m, k, -1);
        } else {
            add_score(m, k, 1);
        }
    }
    lose_life(m.state, kill.victim);
}

/// Takes a life (if lives are limited); eliminates at zero.
pub fn lose_life(state: &mut ModeState, player: PlayerId) {
    if let Some(p) = state.players.get_mut(&player) {
        if p.lives != LIVES_UNLIMITED {
            p.lives = p.lives.saturating_sub(1);
            if p.lives == 0 {
                p.flags |= player_flags::ELIMINATED;
            }
        }
    }
}

/// Generic end: one side left (modes with lives, after at least two sides
/// played), a side at `score_target`, or the timer (best score wins, a tie
/// is a draw).
pub fn generic_end(m: &ModeRef) -> Option<Outcome> {
    if m.rules.lives() > 0 {
        let (alive, played) = sides_in_play(m);
        if played >= 2 && alive.len() <= 1 {
            return Some(alive.first().copied());
        }
    }
    let target = m.state.score_target as i64;
    if target > 0 {
        if m.state.teams > 0 {
            let n = m.state.teams as usize;
            if let Some(t) = (0..n).find(|&t| m.state.team_scores[t] >= target) {
                return Some(Some(t as u8));
            }
        } else {
            let winner = m.state.players.iter().find_map(|(id, p)| {
                (p.score as i64 >= target)
                    .then(|| m.ships.get(id).map(|s| s.team))
                    .flatten()
            });
            if let Some(side) = winner {
                return Some(Some(side));
            }
        }
    }
    if m.now >= m.state.end_tick {
        return Some(if m.state.teams > 0 {
            m.state.team_score_leader()
        } else {
            best_player_side(m)
        });
    }
    None
}

/// The generic part of the view (every mode).
pub(crate) fn base_view(
    state: &ModeState,
    ships: &BTreeMap<PlayerId, ShipState>,
    rules: &ModeRules,
    mutators: &[Mutator],
) -> ModeView {
    use crate::mode::{view_flags, PlayerModeView, MAX_OBJECTIVES, MAX_VIEW_PLAYERS};
    let mut players: Vec<PlayerModeView> = state
        .players
        .iter()
        .filter_map(|(id, p)| {
            let ship = ships.get(id)?;
            Some(PlayerModeView {
                player: *id,
                side: ship.team,
                score: p.score,
                lives: p.lives,
                meter: p.meter,
                flags: p.flags,
            })
        })
        .collect();
    players.sort_by(|a, b| b.score.cmp(&a.score).then(a.player.cmp(&b.player)));
    players.truncate(MAX_VIEW_PLAYERS);
    let mut flags = state.flags;
    if rules.lives() > 0 {
        flags |= view_flags::LIVES;
    }
    for m in mutators {
        flags |= crate::mutators::module(*m).view_flags();
    }
    ModeView {
        kind: state.kind,
        teams: state.teams,
        phase: state.phase,
        end_tick: state.end_tick,
        stage: state.stage,
        stage_end_tick: state.stage_end_tick,
        team_scores: state.team_scores[..state.teams.min(4) as usize].to_vec(),
        score_total: 0,
        score_target: state.score_target,
        players,
        objectives: state
            .objectives
            .iter()
            .take(MAX_OBJECTIVES)
            .map(|o| o.view())
            .collect(),
        flags,
        mood: crate::director::MoodView::default(),
    }
}

/// The default player state for a mode: its lives (or unlimited).
pub(crate) fn initial_player(rules: &ModeRules) -> PlayerMode {
    PlayerMode {
        lives: match rules.lives() {
            0 => LIVES_UNLIMITED,
            n => n,
        },
        ..PlayerMode::default()
    }
}

/// Whether the phase is still running (helper for modules).
pub fn running(state: &ModeState) -> bool {
    state.phase == MatchPhase::Running
}

/// Ticks for `secs` seconds (at least 1 for positive times).
pub fn ticks(secs: f32) -> u64 {
    Tick::ticks_for_secs(secs)
}

#[cfg(test)]
mod tests;
