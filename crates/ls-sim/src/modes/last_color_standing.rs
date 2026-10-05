//! Last Color Standing: a monochrome level; every player has a colour
//! meter, each death drains it, and the last player with any colour wins
//! (free-for-all only).
//!
//! Settings: `ruleset::LastColorStandingRules` (`colour`, `drain_per_kill`,
//! `stalemate_secs`, `stalemate_drain_secs`); default rule set
//! `data/rulesets/last_color_standing.ron`. Design:
//! `docs/milestone-3-plan.md` §3.2.
//!
//! Rules as built:
//! - Every player starts with `colour` (100) in [`PlayerMode::meter`].
//! - Every death drains the victim's colour by `drain_per_kill` (34: three
//!   deaths drain it fully), whoever (or whatever) killed it; damage alone
//!   does not. A player at 0 colour is eliminated (no respawn, spectates).
//! - Stalemate: from `stalemate_secs` (8 minutes) on, every player still in
//!   play loses 1 colour every `stalemate_drain_secs` (2 s), dead or alive;
//!   a living ship drained to 0 leaves play. The match therefore always
//!   ends. [`ModeState::stage`] is 0 before the stalemate (with
//!   `stage_end_tick` = its start) and 1 during it.
//! - The mode score is the colour left, so scoreboards and results list the
//!   most colourful players first (eliminated players at 0, then by kills).
//! - The match ends when at most one player has colour left (that player
//!   wins; nobody left, e.g. the last two drained in the same tick, is a
//!   draw), or on the timer (`match_secs`, a safety net): the most colour
//!   wins, a tie is a draw.
//! - Late joiners (after `LATE_JOIN_GRACE_TICKS`) spectate until the next
//!   match, like in the other elimination modes.
//! - The view sets `view_flags::MONOCHROME` (the client renders the level
//!   without colour) and `view_flags::METER` (ships tinted by their meter).
//!   Pigment pickups that restore colour are M4.

use crate::mode::{player_flags, view_flags, ModeKind, ModeState, ModeView, PlayerMode};
use crate::modes::{
    best_player_side, sides_in_play, ticks, Kill, ModeCtx, ModeModule, ModeMut, ModeRef, Outcome,
    LATE_JOIN_GRACE_TICKS,
};
use crate::ruleset::{LastColorStandingRules, ModeRules};
use crate::ship::PlayerId;

/// The Last Color Standing mode module.
#[derive(Debug)]
pub struct LastColorStanding;

/// [`ModeState::stage`] before the stalemate drain starts.
pub const STAGE_NORMAL: u8 = 0;
/// [`ModeState::stage`] while every remaining player drains.
pub const STAGE_STALEMATE: u8 = 1;

fn settings(rules: &ModeRules) -> LastColorStandingRules {
    match rules {
        ModeRules::LastColorStanding(r) => *r,
        _ => LastColorStandingRules::default(),
    }
}

/// Tick at which the stalemate drain starts.
fn stalemate_tick(r: &LastColorStandingRules) -> u64 {
    ticks(r.stalemate_secs)
}

/// Ticks between two stalemate drains (at least 1).
fn drain_interval(r: &LastColorStandingRules) -> u64 {
    ticks(r.stalemate_drain_secs).max(1)
}

/// Sets a player's colour (and the mode score, which mirrors it);
/// eliminates at 0.
fn set_colour(p: &mut PlayerMode, colour: u8) {
    p.meter = colour;
    p.score = colour as i32;
    if colour == 0 {
        p.flags |= player_flags::ELIMINATED;
    }
}

/// Drains `amount` colour from a player still in play.
fn drain(state: &mut ModeState, player: PlayerId, amount: u8) {
    if let Some(p) = state.players.get_mut(&player) {
        if !p.is_out() {
            set_colour(p, p.meter.saturating_sub(amount));
        }
    }
}

impl ModeModule for LastColorStanding {
    fn kind(&self) -> ModeKind {
        ModeKind::LastColorStanding
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }

    fn validate(&self, rules: &ModeRules) -> Result<(), String> {
        let r = settings(rules);
        if r.colour == 0 {
            return Err("colour must be at least 1".into());
        }
        if r.drain_per_kill == 0 {
            return Err("drain_per_kill must be at least 1".into());
        }
        if r.stalemate_drain_secs <= 0.0 {
            return Err("stalemate_drain_secs must be > 0".into());
        }
        Ok(())
    }

    /// Every player starts with the full colour meter.
    fn on_join(&self, m: &mut ModeMut, player: PlayerId) {
        let colour = settings(m.rules).colour;
        if let Some(p) = m.state.players.get_mut(&player) {
            set_colour(p, colour);
        }
    }

    /// An elimination mode without lives: late joiners spectate too.
    fn late_join_spectates(&self, m: &ModeRef) -> bool {
        m.now > LATE_JOIN_GRACE_TICKS
    }

    fn on_start(&self, m: &mut ModeCtx) {
        let r = settings(m.rules);
        m.state.stage = STAGE_NORMAL;
        m.state.stage_end_tick = stalemate_tick(&r);
    }

    /// Every death drains the victim's colour (kills are counted on the
    /// ships; the mode score is the colour).
    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        let amount = settings(m.rules).drain_per_kill;
        drain(m.state, kill.victim, amount);
    }

    /// The stalemate drain.
    fn on_tick(&self, m: &mut ModeCtx) {
        let r = settings(m.rules);
        let now = m.now();
        let start = stalemate_tick(&r);
        if now < start {
            return;
        }
        if m.state.stage != STAGE_STALEMATE {
            m.state.stage = STAGE_STALEMATE;
            m.state.stage_end_tick = 0;
        }
        let since = now - start;
        if since > 0 && since % drain_interval(&r) == 0 {
            let ids: Vec<PlayerId> = m.state.players.keys().copied().collect();
            for id in ids {
                drain(m.state, id, 1);
            }
        }
    }

    fn end_check(&self, m: &ModeRef) -> Option<Outcome> {
        let (alive, played) = sides_in_play(m);
        if played >= 2 && alive.len() <= 1 {
            return Some(alive.first().copied());
        }
        (m.now >= m.state.end_tick).then(|| best_player_side(m))
    }

    fn view(&self, _state: &ModeState, _rules: &ModeRules, view: &mut ModeView) {
        view.flags |= view_flags::MONOCHROME | view_flags::METER;
    }
}

#[cfg(test)]
mod tests;
