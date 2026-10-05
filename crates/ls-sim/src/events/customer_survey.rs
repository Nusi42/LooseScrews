//! Customer survey (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! A drone parks 2.5 u in front of the leader's view for 12 s or until shot (15 hp); the star
//! the destroying shot went through is the leader's rating.
//!
//! Numbers: `data/events/customer_survey.ron`. Wave A stub (announce only); B2 builds the
//! rules.

use crate::events::{EventCtx, EventKind, EventModule, EventRef};

/// Customer survey.
#[derive(Debug)]
pub struct CustomerSurvey;

impl EventModule for CustomerSurvey {
    fn kind(&self) -> EventKind {
        EventKind::CustomerSurvey
    }

    /// Needs a clear leader (`ModeModule::leader`).
    fn eligible(&self, e: &EventRef) -> bool {
        e.leader.is_some()
    }

    /// Names the leader.
    fn on_announce(&self, e: &mut EventCtx) {
        e.event.target = e.leader;
    }
}
