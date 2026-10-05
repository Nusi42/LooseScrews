//! Pizza delivery (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! A drone brings a pizza to a random living player weighted toward low hull (18 u/s along the
//! network); staying under 1.5 u/s within 5 u for 3 s in total restores full hull; others can
//! shoot the drone (30 hp), dropping a 50-scrap pile.
//!
//! Numbers: `data/events/pizza_delivery.ron`. Wave A stub (announce only); B2 builds the
//! rules.

use crate::events::{EventCtx, EventKind, EventModule, EventRef};

/// Pizza delivery.
#[derive(Debug)]
pub struct PizzaDelivery;

impl EventModule for PizzaDelivery {
    fn kind(&self) -> EventKind {
        EventKind::PizzaDelivery
    }

    /// Needs a damaged living player.
    fn eligible(&self, e: &EventRef) -> bool {
        !crate::events::damaged_ships(e.view).is_empty()
    }

    /// Names a random damaged player, weighted toward low hull.
    fn on_announce(&self, e: &mut EventCtx) {
        e.event.target = crate::director::pick_low_hull(e);
    }
}
