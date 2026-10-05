//! Hat rain (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 25 s, 24 + 2 per player hats drift down along `up` at 3 u/s and rest on rock; touch to
//! wear one; 3 or more hats reveal the wearer; hats scatter on death; the results name the Hat
//! Baron.
//!
//! Numbers: `data/events/hat_rain.ron`. Wave A stub (announce only); B3 builds the
//! rules.

use crate::events::{EventKind, EventModule};

/// Hat rain.
#[derive(Debug)]
pub struct HatRain;

impl EventModule for HatRain {
    fn kind(&self) -> EventKind {
        EventKind::HatRain
    }
}
