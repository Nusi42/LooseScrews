//! Data-driven weapon definitions loaded from `data/weapons/*.ron`.

use std::path::Path;

use ls_math::Tick;
use serde::{Deserialize, Serialize};

use crate::SimError;

/// Weapon family from the design doc's arsenal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeaponFamily {
    /// Slow, dodgeable bolts powered by energy.
    Energy,
    /// Very fast rounds that fire scrap.
    Kinetic,
    /// Missiles and mines with limited counts.
    Ordnance,
    /// Comedy specials (Paint Thrower, ...).
    Special,
}

/// Which trigger fires the weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeaponSlot {
    Primary,
    Secondary,
}

/// What one shot costs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WeaponCost {
    /// Energy per shot (overdrive can pay the missing part with hull).
    Energy(f32),
    /// Scrap per shot (kinetic weapons fire scrap).
    Scrap(f32),
    /// Ammo count per shot (ordnance).
    Count(u32),
}

/// How the weapon interacts with a Shield gadget (present for M2; unused in M1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ShieldInteraction {
    Blocked,
    WearsDown,
    /// The given fraction (0..=1) passes through to the hull.
    PassThrough(f32),
}

/// Area damage around the impact point, falling off linearly to zero at `radius`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Splash {
    pub radius: f32,
    pub damage: f32,
    /// Whether the splash can damage the shooter.
    #[serde(default)]
    pub hits_owner: bool,
}

/// Lasting terrain effect on impact. Radii in u (the design doc lists crater diameters).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TerrainEffect {
    #[serde(default)]
    pub crater_radius: f32,
    #[serde(default)]
    pub paint_radius: f32,
}

fn one() -> u8 {
    1
}

/// One weapon's stats. Speeds in u/s, durations in seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeaponDef {
    /// Stable identifier used by rule sets (`snake_case`).
    pub id: String,
    pub name: String,
    pub family: WeaponFamily,
    pub slot: WeaponSlot,
    /// Projectile speed. Projectiles never inherit the shooter's velocity.
    pub speed: f32,
    pub lifetime_secs: f32,
    /// Direct-hit damage at level 1 (before the ship's damage multiplier).
    pub damage: f32,
    pub fire_interval_secs: f32,
    pub cost: WeaponCost,
    #[serde(default)]
    pub projectile_radius: f32,
    #[serde(default)]
    pub splash: Option<Splash>,
    /// Velocity change (u/s) applied to a ship on a direct hit at level 1, along
    /// the projectile direction. Like damage it is scaled by the level and ship
    /// damage multipliers; splash knockback also scales with the splash falloff.
    #[serde(default)]
    pub knockback: f32,
    #[serde(default)]
    pub terrain: TerrainEffect,
    pub shield: ShieldInteraction,
    /// Number of upgrade levels (1 = no upgrades).
    #[serde(default = "one")]
    pub max_level: u8,
    /// Extra damage per level above 1, as a fraction of the base damage.
    #[serde(default)]
    pub damage_per_level: f32,
    /// Ammo cap for [`WeaponCost::Count`] weapons.
    #[serde(default)]
    pub max_ammo: u32,
}

impl WeaponDef {
    /// Parses one weapon definition from RON and validates it.
    pub fn from_ron(source_name: &str, text: &str) -> Result<WeaponDef, SimError> {
        let def: WeaponDef = ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })?;
        def.validate()?;
        Ok(def)
    }

    /// Checks value ranges.
    pub fn validate(&self) -> Result<(), SimError> {
        let bad = |reason: &str| SimError::InvalidWeapon {
            id: self.id.clone(),
            reason: reason.to_string(),
        };
        let pos = |v: f32| v.is_finite() && v > 0.0;
        let non_neg = |v: f32| v.is_finite() && v >= 0.0;
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(bad("id must be non-empty snake_case"));
        }
        if !pos(self.speed) {
            return Err(bad("speed must be > 0"));
        }
        if !pos(self.lifetime_secs) {
            return Err(bad("lifetime_secs must be > 0"));
        }
        if !pos(self.fire_interval_secs) {
            return Err(bad("fire_interval_secs must be > 0"));
        }
        if !non_neg(self.damage) || !non_neg(self.knockback) || !non_neg(self.projectile_radius) {
            return Err(bad("damage, knockback and projectile_radius must be >= 0"));
        }
        if !non_neg(self.terrain.crater_radius) || !non_neg(self.terrain.paint_radius) {
            return Err(bad("terrain radii must be >= 0"));
        }
        if let Some(s) = self.splash {
            if !pos(s.radius) || !non_neg(s.damage) {
                return Err(bad("splash radius must be > 0 and damage >= 0"));
            }
        }
        match self.cost {
            WeaponCost::Energy(c) | WeaponCost::Scrap(c) if !non_neg(c) => {
                return Err(bad("cost must be >= 0"));
            }
            WeaponCost::Count(n) if n == 0 || self.max_ammo < n => {
                return Err(bad("count weapons need a count >= 1 and max_ammo >= count"));
            }
            _ => {}
        }
        if let ShieldInteraction::PassThrough(f) = self.shield {
            if !(0.0..=1.0).contains(&f) {
                return Err(bad("shield pass-through fraction must be in 0..=1"));
            }
        }
        if self.max_level == 0 || !non_neg(self.damage_per_level) {
            return Err(bad("max_level must be >= 1 and damage_per_level >= 0"));
        }
        Ok(())
    }

    /// Projectile lifetime in ticks (at least 1).
    pub fn lifetime_ticks(&self) -> u64 {
        Tick::ticks_for_secs(self.lifetime_secs).max(1)
    }

    /// Ticks between shots (at least 1).
    pub fn fire_interval_ticks(&self) -> u64 {
        Tick::ticks_for_secs(self.fire_interval_secs).max(1)
    }

    /// Damage multiplier for an upgrade level (level 1 = 1.0).
    pub fn level_multiplier(&self, level: u8) -> f32 {
        let extra = level.clamp(1, self.max_level) - 1;
        1.0 + self.damage_per_level * extra as f32
    }
}

/// Index of a weapon in a [`WeaponTable`] (weapons sorted by id).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct WeaponId(pub u8);

/// All loaded weapons, sorted by id so indices are stable for a given data set.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponTable {
    defs: Vec<WeaponDef>,
}

/// The M1 weapon files, embedded so the WASM client and tests need no file system.
const BUILTIN_WEAPONS: &[(&str, &str)] = &[
    (
        "scrap_laser.ron",
        include_str!("../../../data/weapons/scrap_laser.ron"),
    ),
    (
        "rivet_gun.ron",
        include_str!("../../../data/weapons/rivet_gun.ron"),
    ),
    (
        "bolt_rocket.ron",
        include_str!("../../../data/weapons/bolt_rocket.ron"),
    ),
    (
        "paint_thrower.ron",
        include_str!("../../../data/weapons/paint_thrower.ron"),
    ),
];

impl WeaponTable {
    /// Builds a table from definitions (validated, sorted by id, ids unique).
    pub fn new(mut defs: Vec<WeaponDef>) -> Result<WeaponTable, SimError> {
        if defs.len() > u8::MAX as usize {
            return Err(SimError::TooManyWeapons(defs.len()));
        }
        for d in &defs {
            d.validate()?;
        }
        defs.sort_by(|a, b| a.id.cmp(&b.id));
        for pair in defs.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(SimError::DuplicateWeapon(pair[0].id.clone()));
            }
        }
        Ok(WeaponTable { defs })
    }

    /// Parses `(source name, RON text)` pairs.
    pub fn from_ron_sources<'a>(
        sources: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<WeaponTable, SimError> {
        let defs = sources
            .into_iter()
            .map(|(name, text)| WeaponDef::from_ron(name, text))
            .collect::<Result<Vec<_>, _>>()?;
        WeaponTable::new(defs)
    }

    /// Loads every `*.ron` file in a directory (e.g. `data/weapons`).
    pub fn load_dir(dir: impl AsRef<Path>) -> Result<WeaponTable, SimError> {
        let dir = dir.as_ref();
        let io = |e: std::io::Error| SimError::Io {
            path: dir.display().to_string(),
            message: e.to_string(),
        };
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if path.extension().is_some_and(|e| e == "ron") {
                paths.push(path);
            }
        }
        paths.sort();
        let mut defs = Vec::with_capacity(paths.len());
        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|e| SimError::Io {
                path: path.display().to_string(),
                message: e.to_string(),
            })?;
            defs.push(WeaponDef::from_ron(&path.display().to_string(), &text)?);
        }
        WeaponTable::new(defs)
    }

    /// The four M1 weapons from `data/weapons`, embedded at compile time.
    pub fn builtin() -> WeaponTable {
        WeaponTable::from_ron_sources(BUILTIN_WEAPONS.iter().copied())
            .expect("built-in weapon data is valid")
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Definition for an id. Panics on an id from a different table.
    pub fn get(&self, id: WeaponId) -> &WeaponDef {
        &self.defs[id.0 as usize]
    }

    /// Looks up a weapon by its string id.
    pub fn find(&self, id: &str) -> Option<WeaponId> {
        self.defs
            .binary_search_by(|d| d.id.as_str().cmp(id))
            .ok()
            .map(|i| WeaponId(i as u8))
    }

    /// Looks up a weapon by string id, or errors.
    pub fn require(&self, id: &str) -> Result<WeaponId, SimError> {
        self.find(id)
            .ok_or_else(|| SimError::UnknownWeapon(id.to_string()))
    }

    pub fn iter(&self) -> impl Iterator<Item = (WeaponId, &WeaponDef)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, d)| (WeaponId(i as u8), d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn builtin_weapons_match_design_doc() {
        let t = WeaponTable::builtin();
        assert_eq!(t.len(), 4);
        let laser = t.get(t.find("scrap_laser").unwrap());
        assert_eq!(laser.speed, 30.0);
        assert_eq!(laser.lifetime_secs, 3.0);
        assert_eq!(laser.lifetime_ticks(), 180);
        assert_eq!(laser.family, WeaponFamily::Energy);
        assert!(matches!(laser.cost, WeaponCost::Energy(_)));
        assert_eq!(laser.max_level, 6);

        let rivet = t.get(t.find("rivet_gun").unwrap());
        assert_eq!(rivet.speed, 180.0);
        assert_eq!(rivet.speed / laser.speed, 6.0);
        assert_eq!(rivet.family, WeaponFamily::Kinetic);
        assert!(matches!(rivet.cost, WeaponCost::Scrap(_)));

        let rocket = t.get(t.find("bolt_rocket").unwrap());
        assert_eq!(rocket.speed, 40.0);
        assert_eq!(rocket.slot, WeaponSlot::Secondary);
        // 2 u crater (diameter) in the design doc.
        assert_eq!(rocket.terrain.crater_radius * 2.0, 2.0);
        assert!(matches!(rocket.cost, WeaponCost::Count(1)));
        assert!(rocket.splash.is_some());

        let paint = t.get(t.find("paint_thrower").unwrap());
        assert_eq!(paint.damage, 0.0);
        assert!(paint.terrain.paint_radius > 0.0);
        assert_eq!(paint.family, WeaponFamily::Special);
    }

    #[test]
    fn table_sorted_and_lookup() {
        let t = WeaponTable::builtin();
        let ids: Vec<&str> = t.iter().map(|(_, d)| d.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert!(t.find("nope").is_none());
        assert!(t.require("nope").is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn load_dir_matches_builtin() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/weapons");
        assert_eq!(WeaponTable::load_dir(dir).unwrap(), WeaponTable::builtin());
    }

    #[test]
    fn rejects_bad_defs() {
        let mut d = WeaponTable::builtin().get(WeaponId(0)).clone();
        d.speed = 0.0;
        assert!(d.validate().is_err());
        let mut d2 = WeaponTable::builtin().get(WeaponId(0)).clone();
        d2.id = "Bad Id".into();
        assert!(d2.validate().is_err());
        let a = WeaponTable::builtin().get(WeaponId(0)).clone();
        assert!(matches!(
            WeaponTable::new(vec![a.clone(), a]),
            Err(SimError::DuplicateWeapon(_))
        ));
        assert!(WeaponDef::from_ron("x", "WeaponDef(id: \"x\")").is_err());
    }

    #[test]
    fn level_multiplier() {
        let t = WeaponTable::builtin();
        let laser = t.get(t.find("scrap_laser").unwrap());
        assert_eq!(laser.level_multiplier(1), 1.0);
        assert!(laser.level_multiplier(6) > laser.level_multiplier(5));
        assert_eq!(laser.level_multiplier(9), laser.level_multiplier(6));
    }
}
