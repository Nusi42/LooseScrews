//! Pigment on monochrome levels (M4; `docs/milestone-4-plan.md` §3.4).
//!
//! A level is monochrome when `MatchSetup::monochrome` is set (Palette 80
//! or more) or the mode is Last Color Standing. Then the world keeps pigment
//! pickups at the level's pickup points (beside the weighted weapon pickups,
//! which they do not replace):
//!
//! - **Count.** The rule set's `pigment_pickup` ([`crate::RuleSet`], rule
//!   set code format 5; `None`: no pigment pickups): one per
//!   `players_per_pickup` players in play, at least `min_on_map`, at
//!   most one per pickup point. A taken pickup respawns `respawn_secs` later
//!   at a pickup point no other pigment occupies, chosen by a private RNG
//!   keyed by the match seed and the tick ([`PIGMENT_RNG_KEY`]).
//! - **Pigmented.** Touching one colours the ship for `duration_secs`
//!   (`ShipState::pigment_until`; another pickup refreshes it): +25 % damage
//!   with every weapon (applied where the world deals damage), drawn in its
//!   side's colour and shown to everyone (presentation: the client).
//! - **Splash.** A pigmented ship that dies drops a splash pickup in its
//!   colour that lasts `splash_secs`.
//! - **Last Color Standing.** The mode's `on_pigment` hook takes the pickup
//!   instead: it restores `restore_colour` colour (capped at the starting
//!   colour); no pigmented state, no damage bonus (colour already makes you
//!   visible).
//!
//! Replication: [`crate::ReplicatedKind::PigmentPickup`] views with state
//! `[splash (0/1), side of a splash, 0, 0]`; taking one emits
//! `CueKind::Pigment`. Numbers: `data/world/pigment.ron`.

use std::sync::OnceLock;

use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::hash::StateHasher;

/// `data/world/pigment.ron` (embedded, part of the data hash).
pub const PIGMENT_RON: &str = include_str!("../../../data/world/pigment.ron");

/// Palette slider value from which a level is monochrome
/// (`MatchSetup::monochrome`, §3.4).
pub const MONOCHROME_PALETTE: u8 = 80;

/// Key of the respawn point RNG (`Ctx::rng` style).
pub const PIGMENT_RNG_KEY: u64 = 0x0050_4947_4d45_4e54; // "PIGMENT"

/// How many pigment pickups a level gets and how fast they come back: the
/// rule set's [`crate::PigmentPickupRule`].
pub use crate::ruleset::PigmentPickupRule;

impl PigmentPickupRule {
    /// Pickups wanted for `players` players in play (before the point cap).
    pub fn count(&self, players: usize) -> usize {
        let per = self.players_per_pickup.max(1) as usize;
        players.div_ceil(per).max(self.min_on_map as usize)
    }
}

/// Pigment tuning (`data/world/pigment.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PigmentSettings {
    pub duration_secs: f32,
    /// Extra damage share of a pigmented ship's weapons (0.25 = +25 %).
    pub damage_bonus: f32,
    pub splash_secs: f32,
    /// Colour a Last Color Standing pickup restores.
    pub restore_colour: u8,
}

impl PigmentSettings {
    /// The embedded `data/world/pigment.ron`.
    pub fn builtin() -> &'static PigmentSettings {
        static S: OnceLock<PigmentSettings> = OnceLock::new();
        S.get_or_init(|| {
            ron::from_str(PIGMENT_RON).expect("data/world/pigment.ron is valid (tested)")
        })
    }
}

/// A pigment pickup in the level.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PigmentPickup {
    /// Entity id (shares the id space of entities and pickups).
    pub id: u32,
    pub pos: Vec3,
    /// Pickup point index, or `None` for a splash dropped by a dying ship.
    pub point: Option<u16>,
    /// Side whose colour a splash carries (0 for regular pickups).
    pub side: u8,
    /// Removed at this tick (splashes; `u64::MAX` = stays until taken).
    pub expires_tick: u64,
}

/// Pigment state of a match (hashed when not empty).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PigmentState {
    pub pickups: Vec<PigmentPickup>,
    /// Ticks at which taken regular pickups come back.
    pub respawns: Vec<u64>,
}

impl PigmentState {
    pub fn is_empty(&self) -> bool {
        self.pickups.is_empty() && self.respawns.is_empty()
    }

    /// Regular (non-splash) pickups present or coming back.
    pub fn regular(&self) -> usize {
        self.pickups.iter().filter(|p| p.point.is_some()).count() + self.respawns.len()
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.pickups.len() as u32);
        for p in &self.pickups {
            h.write_u32(p.id);
            h.write_vec3(p.pos);
            h.write_u32(p.point.map_or(u32::MAX, |x| x as u32));
            h.write_u8(p.side);
            h.write_u64(p.expires_tick);
        }
        h.write_u32(self.respawns.len() as u32);
        for t in &self.respawns {
            h.write_u64(*t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn settings_parse_and_count() {
        let s = PigmentSettings::builtin();
        assert_eq!(s.duration_secs, 40.0);
        assert_eq!(s.damage_bonus, 0.25);
        assert_eq!(s.splash_secs, 15.0);
        assert_eq!(s.restore_colour, 17);
        let m = PigmentPickupRule {
            respawn_secs: 30,
            players_per_pickup: 4,
            min_on_map: 2,
        };
        assert_eq!(
            (m.count(0), m.count(4), m.count(5), m.count(9)),
            (2, 2, 2, 3)
        );
        let l = PigmentPickupRule { min_on_map: 1, ..m };
        assert_eq!(
            (l.count(1), l.count(4), l.count(8), l.count(12)),
            (1, 1, 2, 3)
        );
    }
}
