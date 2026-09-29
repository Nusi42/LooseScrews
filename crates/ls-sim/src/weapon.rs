//! Data-driven weapon definitions loaded from `data/weapons/*.ron`.
//!
//! A [`WeaponDef`] holds the numbers; the behaviour lives in a module per
//! weapon under `src/weapons/` (see [`crate::weapons`]). The built-in data
//! files are embedded by the registry in `src/weapons/registry.rs`.

use std::collections::BTreeMap;
use std::path::Path;

use ls_math::Tick;
use serde::{Deserialize, Serialize};

use crate::visual::Visual;
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

/// How the weapon interacts with an active Shield gadget (design doc, Gadgets).
/// Rule sets can override it per weapon (`RuleSet::shield_overrides`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ShieldInteraction {
    /// The shield stops the hit; it costs shield energy like `Wears(1.0)`
    /// but never reaches the hull, even when the energy runs out.
    Blocked,
    /// The shield absorbs the hit and loses `f x damage` energy (times the
    /// shield's `energy_per_damage`); what the energy cannot cover reaches
    /// the hull. `f >= 0`.
    Wears(f32),
    /// The fraction `f` (0..=1) passes through to the hull; the shield absorbs
    /// the rest for free.
    PassThrough(f32),
}

/// Charge mechanics: hold the trigger to charge, release to fire.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChargeDef {
    /// Full charge time; charge fraction is `held / max_secs`, capped at 1.
    pub max_secs: f32,
    /// Releasing earlier cancels the shot (nothing is paid).
    #[serde(default)]
    pub min_secs: f32,
    /// Fires by itself once `max_secs` is reached (Polite Laser).
    #[serde(default)]
    pub auto_release: bool,
    /// Damage and knockback multiplier at full charge (linear from 1 at no charge).
    #[serde(default = "one_f32")]
    pub damage_mult: f32,
}

fn one_f32() -> f32 {
    1.0
}

fn one_u8() -> u8 {
    1
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
    #[serde(default = "one_u8")]
    pub max_level: u8,
    /// Extra damage per level above 1, as a fraction of the base damage.
    #[serde(default)]
    pub damage_per_level: f32,
    /// Ammo cap for [`WeaponCost::Count`] weapons.
    #[serde(default)]
    pub max_ammo: u32,
    /// Projectiles per shot for the generic fire pattern (Scatter Laser 3,
    /// Scrap Shotgun 8). Damage is per projectile.
    #[serde(default = "one_u8")]
    pub projectiles: u8,
    /// Angle between neighbouring projectiles of the generic pattern (degrees).
    #[serde(default)]
    pub spread_deg: f32,
    /// Hold-to-charge mechanics (Fusion Charger, Polite Laser).
    #[serde(default)]
    pub charge: Option<ChargeDef>,
    /// How projectiles (and, tinted, pickups) of this weapon are drawn.
    #[serde(default)]
    pub visual: Visual,
    /// Free numeric parameters for the weapon module (`param`), so a module
    /// can add tuning numbers without touching this struct.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
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
        match self.shield {
            ShieldInteraction::PassThrough(f) if !(0.0..=1.0).contains(&f) => {
                return Err(bad("shield pass-through fraction must be in 0..=1"));
            }
            ShieldInteraction::Wears(f) if !non_neg(f) => {
                return Err(bad("shield wear factor must be >= 0"));
            }
            _ => {}
        }
        if self.projectiles == 0 || !non_neg(self.spread_deg) {
            return Err(bad("projectiles must be >= 1 and spread_deg >= 0"));
        }
        if let Some(c) = self.charge {
            if !pos(c.max_secs) || !non_neg(c.min_secs) || c.min_secs > c.max_secs {
                return Err(bad(
                    "charge needs max_secs > 0 and 0 <= min_secs <= max_secs",
                ));
            }
            if !non_neg(c.damage_mult) {
                return Err(bad("charge damage_mult must be >= 0"));
            }
        }
        if !(self.visual.scale.is_finite() && self.visual.scale > 0.0) {
            return Err(bad("visual scale must be > 0"));
        }
        if let Some((k, _)) = self.params.iter().find(|(_, v)| !v.is_finite()) {
            return Err(bad(&format!("param `{k}` must be finite")));
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

    /// A module parameter from `params`, or `default` if the data file has none.
    pub fn param(&self, key: &str, default: f32) -> f32 {
        self.params.get(key).copied().unwrap_or(default)
    }

    /// Ticks for a [`ChargeDef`] duration in seconds (at least 1).
    pub fn charge_ticks(secs: f32) -> u64 {
        Tick::ticks_for_secs(secs).max(1)
    }

    /// Knockback (u/s) per point of damage: the ratio of the data's
    /// `knockback` to `damage`, so every hit pushes in proportion to its
    /// damage. Weapons without damage use [`DEFAULT_KNOCKBACK_PER_DAMAGE`].
    pub fn knockback_per_damage(&self) -> f32 {
        if self.damage > 0.0 {
            self.knockback / self.damage
        } else {
            DEFAULT_KNOCKBACK_PER_DAMAGE
        }
    }
}

/// Knockback per damage point for hits without a weapon ratio (u/s per point).
pub const DEFAULT_KNOCKBACK_PER_DAMAGE: f32 = 0.15;

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

    /// Every weapon of the design doc from `data/weapons`, embedded at
    /// compile time by the registry (`crate::weapons::registry`).
    pub fn builtin() -> WeaponTable {
        WeaponTable::from_ron_sources(
            crate::weapons::registry::MODULES
                .iter()
                .map(|m| (m.file, m.data)),
        )
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
        assert_eq!(t.len(), 27);
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

    #[test]
    fn params_charge_and_knockback_ratio() {
        let t = WeaponTable::builtin();
        let fusion = t.get(t.find("fusion_charger").unwrap());
        let c = fusion.charge.unwrap();
        assert_eq!(c.max_secs, 2.0);
        assert!(c.damage_mult > 1.0);
        assert_eq!(fusion.param("channel_radius", 0.0), 0.8);
        assert_eq!(fusion.param("missing", 7.0), 7.0);
        let laser = t.get(t.find("scrap_laser").unwrap());
        assert_eq!(laser.knockback_per_damage(), 1.0 / 8.0);
        let flash = t.get(t.find("flash_pod").unwrap());
        assert_eq!(flash.knockback_per_damage(), DEFAULT_KNOCKBACK_PER_DAMAGE);
        let shotgun = t.get(t.find("scrap_shotgun").unwrap());
        assert_eq!(shotgun.projectiles, 8);

        let mut bad = laser.clone();
        bad.shield = ShieldInteraction::PassThrough(1.5);
        assert!(bad.validate().is_err());
        let mut bad = laser.clone();
        bad.projectiles = 0;
        assert!(bad.validate().is_err());
        let mut bad = fusion.clone();
        bad.charge = Some(ChargeDef {
            max_secs: 1.0,
            min_secs: 2.0,
            auto_release: false,
            damage_mult: 1.0,
        });
        assert!(bad.validate().is_err());
        let mut bad = laser.clone();
        bad.params.insert("x".into(), f32::NAN);
        assert!(bad.validate().is_err());
    }
}
