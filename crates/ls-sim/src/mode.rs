//! Paint the Void: two teams claim territory by painting surfaces; score is the
//! share of the level's surface in the team's color when the timer runs out.

use serde::{Deserialize, Serialize};

use crate::terrain::PaintCoverage;

/// How often (ticks) scores are refreshed from the terrain's paint coverage.
pub const SCORE_INTERVAL_TICKS: u64 = 30;

/// Match phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchPhase {
    Running,
    /// The match is over; `winner` is `None` on a draw.
    Ended {
        winner: Option<u8>,
    },
}

/// Paint the Void match state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaintTheVoid {
    /// Tick at which the match timer runs out.
    pub end_tick: u64,
    /// Latest coverage reported by the terrain.
    pub coverage: PaintCoverage,
    pub phase: MatchPhase,
}

impl PaintTheVoid {
    pub fn new(match_ticks: u64) -> Self {
        PaintTheVoid {
            end_tick: match_ticks,
            coverage: PaintCoverage::default(),
            phase: MatchPhase::Running,
        }
    }

    pub fn is_running(&self) -> bool {
        self.phase == MatchPhase::Running
    }

    /// Ticks left on the match timer at `tick`.
    pub fn remaining_ticks(&self, tick: u64) -> u64 {
        self.end_tick.saturating_sub(tick)
    }

    /// Team score in percent of the painted surface.
    pub fn score_percent(&self, team: u8) -> f32 {
        self.coverage.percent(team)
    }

    /// Winner by painted cells (integer comparison, so it is exact); `None` on a tie.
    pub fn leader(&self) -> Option<u8> {
        let [a, b] = self.coverage.team_cells;
        match a.cmp(&b) {
            std::cmp::Ordering::Greater => Some(0),
            std::cmp::Ordering::Less => Some(1),
            std::cmp::Ordering::Equal => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn leader_and_percent() {
        let mut m = PaintTheVoid::new(100);
        assert_eq!(m.leader(), None);
        m.coverage = PaintCoverage {
            team_cells: [10, 30],
            surface_cells: 200,
        };
        assert_eq!(m.leader(), Some(1));
        assert_eq!(m.score_percent(1), 15.0);
        assert_eq!(m.remaining_ticks(40), 60);
        assert_eq!(m.remaining_ticks(400), 0);
    }
}
