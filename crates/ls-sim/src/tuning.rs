//! Ship types and per-type tuning numbers.
//!
//! Only the Tinker (the standard ship and the reference for every speed in the
//! design doc) is implemented in M1. Its numbers follow the design doc where the
//! doc gives them (speeds, hull, 2:1 overdrive, two secondary slots); the rest are
//! tuning starting points. The numbers live in `data/ships/*.ron` and are embedded
//! with `include_str!` so the WASM client and tests need no file system.

use serde::{Deserialize, Serialize};

use crate::SimError;

const TINKER_RON: &str = include_str!("../../../data/ships/tinker.ron");

/// The Scrapling hull types from the design doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ShipType {
    Skitter,
    Tinker,
    Hauler,
    Tug,
}

impl ShipType {
    pub fn index(self) -> u8 {
        match self {
            ShipType::Skitter => 0,
            ShipType::Tinker => 1,
            ShipType::Hauler => 2,
            ShipType::Tug => 3,
        }
    }
}

/// Flight model and resource numbers for one ship type. Speeds in u/s,
/// accelerations in u/s², rates per second, angles in radians.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipTuning {
    /// Collision and hit sphere radius (1 u = one ship length).
    pub radius: f32,
    /// Per-axis speed cap in ship space while cruising.
    pub cruise_speed: f32,
    /// Per-axis speed cap in ship space while boosting.
    pub boost_speed: f32,
    /// Acceleration towards the thrust target on an axis.
    pub thrust_accel: f32,
    /// Deceleration when thrusting against the current motion on an axis.
    pub brake_accel: f32,
    /// Damping (linear deceleration) on an axis with no thrust input.
    pub damping_decel: f32,
    /// Deceleration of speed above the current cap (after boost ends or knockback).
    pub overspeed_decel: f32,
    /// Coefficient of restitution for terrain collisions (0 = slide, 1 = bounce).
    pub restitution: f32,
    /// Maximum mouse-look turn rate; larger look deltas are clamped.
    pub max_turn_rate: f32,
    /// Roll rate at full roll input.
    pub roll_rate: f32,
    /// Horizon assist: roll correction gain (1/s) and maximum correction rate.
    pub horizon_gain: f32,
    pub horizon_max_rate: f32,

    pub hull_max: f32,
    pub energy_max: f32,
    pub boost_max: f32,
    pub scrap_max: f32,
    pub spawn_energy: f32,
    pub spawn_boost: f32,
    pub spawn_scrap: f32,
    /// Energy regenerated per second, only while not boosting.
    pub energy_regen: f32,
    /// Boost consumed per second while boosting.
    pub boost_drain: f32,
    /// Boost gained per u/s of speed removed by hard braking.
    pub brake_refill_per_speed: f32,
    /// Braking only counts as "hard" while the axis speed is above this.
    pub hard_brake_min_speed: f32,
    /// Near-miss wall scrape: gap below which scraping refills boost.
    pub scrape_distance: f32,
    /// Minimum speed for a scrape to count.
    pub scrape_min_speed: f32,
    /// Boost per second at zero gap (falls off linearly to the scrape distance).
    pub scrape_refill: f32,
    /// Distance from the ship center to a scrap source for scavenging.
    pub scavenge_radius: f32,
    /// Scrap drawn per second while scavenging.
    pub scavenge_rate: f32,
    /// Hull repaired per unit of scavenged scrap.
    pub hull_per_scrap: f32,
    /// Overdrive gamble: hull burned per missing energy point.
    pub overdrive_hull_per_energy: f32,
    /// Damage multiplier for this ship's weapons.
    pub damage_mult: f32,
    /// Number of secondary weapon types carried at once.
    pub secondary_slots: u8,
    /// Scrap left in the wreck when the ship is destroyed (plus half its carried scrap).
    pub wreck_scrap: f32,
}

impl ShipTuning {
    /// The Tinker, the standard ship type, from the embedded
    /// `data/ships/tinker.ron`.
    pub fn tinker() -> ShipTuning {
        ShipTuning::from_ron("data/ships/tinker.ron", TINKER_RON)
            .expect("embedded data/ships/tinker.ron is valid")
    }

    /// Parses ship tuning from RON (schema: this struct).
    pub fn from_ron(source_name: &str, text: &str) -> Result<ShipTuning, SimError> {
        ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })
    }

    /// Tuning for a ship type, or `None` if the type is not implemented yet.
    pub fn for_type(ship_type: ShipType) -> Option<ShipTuning> {
        match ship_type {
            ShipType::Tinker => Some(ShipTuning::tinker()),
            _ => None,
        }
    }

    /// Top speed: all three axes at the boost cap (`boost_speed * sqrt(3)`).
    pub fn top_speed(&self) -> f32 {
        self.boost_speed * 3.0f32.sqrt()
    }
}

impl Default for ShipTuning {
    fn default() -> Self {
        ShipTuning::tinker()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// The values that were hard-coded before the data file existed; the data
    /// file must reproduce them bit for bit (the replay golden hash depends on it).
    fn reference() -> ShipTuning {
        ShipTuning {
            radius: 0.5,
            cruise_speed: 12.0,
            boost_speed: 16.5,
            thrust_accel: 30.0,
            brake_accel: 40.0,
            damping_decel: 8.0,
            overspeed_decel: 20.0,
            restitution: 0.2,
            max_turn_rate: 12.0,
            roll_rate: 2.5,
            horizon_gain: 1.5,
            horizon_max_rate: 0.8,
            hull_max: 100.0,
            energy_max: 100.0,
            boost_max: 100.0,
            scrap_max: 100.0,
            spawn_energy: 100.0,
            spawn_boost: 50.0,
            spawn_scrap: 20.0,
            energy_regen: 10.0,
            boost_drain: 20.0,
            brake_refill_per_speed: 0.6,
            hard_brake_min_speed: 6.0,
            scrape_distance: 0.6,
            scrape_min_speed: 8.0,
            scrape_refill: 30.0,
            scavenge_radius: 3.0,
            scavenge_rate: 20.0,
            hull_per_scrap: 1.0,
            overdrive_hull_per_energy: 2.0,
            damage_mult: 1.0,
            secondary_slots: 2,
            wreck_scrap: 30.0,
        }
    }

    #[test]
    fn tinker_data_file_matches_reference() {
        assert_eq!(ShipTuning::tinker(), reference());
    }

    #[test]
    fn bad_ron_is_a_parse_error() {
        assert!(matches!(
            ShipTuning::from_ron("x", "ShipTuning(radius: 1.0)"),
            Err(SimError::Parse { .. })
        ));
    }
}
