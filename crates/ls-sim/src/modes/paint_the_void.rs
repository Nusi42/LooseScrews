//! Paint the Void: 2 to 4 teams claim territory by painting surfaces; the
//! score is the share of the level's surface in the team's colour when the
//! timer runs out (the most painted cells win, a tie is a draw). Kills do
//! not score.
//!
//! Settings: `ruleset::PaintTheVoidRules` (`teams`, `match_secs`,
//! `respawn_secs`).
//!
//! Status: implemented (ported from the M1/M2 `World`; same behaviour and
//! state hash for two teams).

use crate::mode::{view_flags, ModeKind, ModeState, ModeView, SCORE_INTERVAL_TICKS};
use crate::modes::{Kill, ModeCtx, ModeModule, ModeMut, ModeRef, Outcome};
use crate::ruleset::ModeRules;
use crate::Event;

/// The Paint the Void mode module.
#[derive(Debug)]
pub struct PaintTheVoid;

impl ModeModule for PaintTheVoid {
    fn kind(&self) -> ModeKind {
        ModeKind::PaintTheVoid
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[2, 3, 4]
    }

    /// Kills do not score in Paint the Void.
    fn on_kill(&self, _m: &mut ModeMut, _kill: &Kill) {}

    /// Scores: the terrain's paint coverage, refreshed every
    /// `SCORE_INTERVAL_TICKS` and when the timer runs out.
    fn on_tick(&self, m: &mut ModeCtx) {
        let now = m.now();
        if now % SCORE_INTERVAL_TICKS == 0 || now >= m.state.end_tick {
            let cov = m.paint_coverage().unwrap_or_default();
            if cov != m.state.coverage {
                m.state.coverage = cov;
                m.ctx.emit(Event::ScoreChanged {
                    team_cells: cov.team_cells,
                    surface_cells: cov.surface_cells,
                });
            }
        }
    }

    fn end_check(&self, m: &ModeRef) -> Option<Outcome> {
        (m.now >= m.state.end_tick).then(|| m.state.leader())
    }

    fn view(&self, state: &ModeState, _rules: &ModeRules, view: &mut ModeView) {
        let n = state.teams.min(4) as usize;
        view.team_scores = state.coverage.team_cells[..n]
            .iter()
            .map(|&c| c as i64)
            .collect();
        view.score_total = state.coverage.surface_cells;
        view.flags |= view_flags::PAINT_SCORES;
    }
}
