//! Ship types and per-type tuning numbers.
//!
//! The four Scrapling hulls of the design doc (Skitter, Tinker, Hauler, Tug).
//! The Tinker is the standard ship and the reference for every speed in the
//! design doc. Numbers follow the design doc's ship table where it gives them
//! (top speed relative to the laser, damage multiplier, hull, secondary slots);
//! the rest are tuning starting points. They live in `data/ships/*.ron` and are
//! embedded with `include_str!` so the WASM client and tests need no file system.

use serde::{Deserialize, Serialize};

use crate::SimError;

/// Embedded ship data, in [`ShipType::index`] order.
pub const BUILTIN_SHIPS: [(ShipType, &str, &str); 4] = [
    (
        ShipType::Skitter,
        "ships/skitter.ron",
        include_str!("../../../data/ships/skitter.ron"),
    ),
    (
        ShipType::Tinker,
        "ships/tinker.ron",
        include_str!("../../../data/ships/tinker.ron"),
    ),
    (
        ShipType::Hauler,
        "ships/hauler.ron",
        include_str!("../../../data/ships/hauler.ron"),
    ),
    (
        ShipType::Tug,
        "ships/tug.ron",
        include_str!("../../../data/ships/tug.ron"),
    ),
];

/// The Scrapling hull types from the design doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ShipType {
    Skitter,
    Tinker,
    Hauler,
    Tug,
}

impl ShipType {
    /// All types in index order.
    pub const ALL: [ShipType; 4] = [
        ShipType::Skitter,
        ShipType::Tinker,
        ShipType::Hauler,
        ShipType::Tug,
    ];

    /// Stable index (wire and rule set code): Skitter 0, Tinker 1, Hauler 2, Tug 3.
    pub fn index(self) -> u8 {
        match self {
            ShipType::Skitter => 0,
            ShipType::Tinker => 1,
            ShipType::Hauler => 2,
            ShipType::Tug => 3,
        }
    }

    /// Inverse of [`ShipType::index`].
    pub fn from_index(i: u8) -> Option<ShipType> {
        ShipType::ALL.get(i as usize).copied()
    }

    /// Lower-case name (`"skitter"`, ...), as in the data file names.
    pub fn name(self) -> &'static str {
        match self {
            ShipType::Skitter => "skitter",
            ShipType::Tinker => "tinker",
            ShipType::Hauler => "hauler",
            ShipType::Tug => "tug",
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
    /// Support: can pass scrap to a teammate within scavenge reach (Tug).
    #[serde(default)]
    pub can_pass_scrap: bool,
    /// Scrap per second passed to a teammate while scavenging next to them.
    #[serde(default)]
    pub scrap_pass_rate: f32,
    /// Restitution against red (bouncy) paint (Paint Thrower, M2 plan: 0.9).
    #[serde(default = "default_paint_bounce")]
    pub paint_bounce_restitution: f32,
    /// Speed fraction a ship keeps while touching yellow (sticky) paint: the
    /// per-axis caps and the ship's local velocity are limited to this share
    /// of the cruise or boost cap (M2 plan: 0.4).
    #[serde(default = "default_paint_sticky")]
    pub paint_sticky_speed: f32,
}

fn default_paint_bounce() -> f32 {
    0.9
}

fn default_paint_sticky() -> f32 {
    0.4
}

impl ShipTuning {
    /// The Tinker, the standard ship type, from the embedded
    /// `data/ships/tinker.ron`.
    pub fn tinker() -> ShipTuning {
        let (_, name, text) = BUILTIN_SHIPS[ShipType::Tinker.index() as usize];
        ShipTuning::from_ron(name, text).expect("embedded data/ships/tinker.ron is valid")
    }

    /// Parses ship tuning from RON (schema: this struct).
    pub fn from_ron(source_name: &str, text: &str) -> Result<ShipTuning, SimError> {
        ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })
    }

    /// Built-in tuning for a ship type (all four types exist since M2; the
    /// `Option` is kept for data sets that leave a type out).
    pub fn for_type(ship_type: ShipType) -> Option<ShipTuning> {
        Some(ShipTable::builtin().get(ship_type).clone())
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

/// Tuning of every ship type, indexed by [`ShipType::index`].
#[derive(Debug, Clone, PartialEq)]
pub struct ShipTable {
    types: [ShipTuning; 4],
}

impl ShipTable {
    /// The four built-in ship files.
    pub fn builtin() -> ShipTable {
        let parse = |i: usize| {
            let (_, name, text) = BUILTIN_SHIPS[i];
            ShipTuning::from_ron(name, text).expect("embedded ship data is valid")
        };
        ShipTable {
            types: [parse(0), parse(1), parse(2), parse(3)],
        }
    }

    /// A table with the same tuning for every type (tests, tools).
    pub fn uniform(tuning: ShipTuning) -> ShipTable {
        ShipTable {
            types: [tuning.clone(), tuning.clone(), tuning.clone(), tuning],
        }
    }

    /// Replaces one type's tuning (builder).
    pub fn with(mut self, ship_type: ShipType, tuning: ShipTuning) -> ShipTable {
        self.types[ship_type.index() as usize] = tuning;
        self
    }

    pub fn get(&self, ship_type: ShipType) -> &ShipTuning {
        &self.types[ship_type.index() as usize]
    }

    pub fn get_mut(&mut self, ship_type: ShipType) -> &mut ShipTuning {
        &mut self.types[ship_type.index() as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = (ShipType, &ShipTuning)> {
        ShipType::ALL.iter().map(move |&t| (t, self.get(t)))
    }

    /// Largest collision radius of any type (broad-phase margins).
    pub fn max_radius(&self) -> f32 {
        self.types.iter().map(|t| t.radius).fold(0.0, f32::max)
    }
}

impl Default for ShipTable {
    fn default() -> Self {
        ShipTable::builtin()
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
            can_pass_scrap: false,
            scrap_pass_rate: 0.0,
            paint_bounce_restitution: 0.9,
            paint_sticky_speed: 0.4,
        }
    }

    #[test]
    fn tinker_data_file_matches_reference() {
        assert_eq!(ShipTuning::tinker(), reference());
    }

    /// Design doc ship table: top speed relative to the 30 u/s laser, damage
    /// multiplier, hull and secondary slots; every type stays slower than a laser.
    #[test]
    fn ship_types_match_design_doc() {
        let t = ShipTable::builtin();
        let laser = 30.0f32;
        for (ty, speed, dmg, hull, slots) in [
            (ShipType::Skitter, 0.98, 0.85, 70.0, 1),
            (ShipType::Tinker, 0.95, 1.0, 100.0, 2),
            (ShipType::Hauler, 0.85, 1.2, 150.0, 3),
            (ShipType::Tug, 0.90, 0.8, 110.0, 2),
        ] {
            let s = t.get(ty);
            let ratio = s.top_speed() / laser;
            assert!((ratio - speed).abs() < 0.006, "{ty:?} {ratio}");
            assert!(s.top_speed() < laser, "{ty:?} slower than a laser");
            assert_eq!(s.damage_mult, dmg, "{ty:?}");
            assert_eq!(s.hull_max, hull, "{ty:?}");
            assert_eq!(s.secondary_slots, slots, "{ty:?}");
        }
        // Skitter: smallest hitbox, fastest turning; Hauler: largest hitbox.
        let r = |ty| t.get(ty).radius;
        let turn = |ty| t.get(ty).max_turn_rate;
        for ty in [ShipType::Tinker, ShipType::Hauler, ShipType::Tug] {
            assert!(r(ShipType::Skitter) < r(ty) && turn(ShipType::Skitter) > turn(ty));
            assert!(r(ShipType::Hauler) >= r(ty));
        }
        assert_eq!(t.max_radius(), r(ShipType::Hauler));
        // Tug: scavenges faster and can pass scrap.
        let tug = t.get(ShipType::Tug);
        assert!(tug.can_pass_scrap && tug.scavenge_rate > t.get(ShipType::Tinker).scavenge_rate);
        for ty in ShipType::ALL {
            assert_eq!(ShipType::from_index(ty.index()), Some(ty));
        }
        assert_eq!(ShipType::from_index(4), None);
    }

    #[test]
    fn bad_ron_is_a_parse_error() {
        assert!(matches!(
            ShipTuning::from_ron("x", "ShipTuning(radius: 1.0)"),
            Err(SimError::Parse { .. })
        ));
    }
}
