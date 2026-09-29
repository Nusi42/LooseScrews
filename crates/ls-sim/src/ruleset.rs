//! Match rule sets loaded from `data/rulesets/*.ron`.

use std::collections::BTreeMap;
use std::path::Path;

use ls_math::Tick;
use serde::{Deserialize, Serialize};

use crate::{ShipType, SimError, WeaponCost, WeaponId, WeaponSlot, WeaponTable};

/// Per-weapon availability (design doc, Match rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Availability {
    /// Players spawn with it and can find ammo and upgrades in the level.
    LoadoutAndPickup,
    /// Spawns only in the level.
    PickupOnly,
    /// Given at spawn; never found in the level.
    LoadoutOnly,
    /// Not available in this match.
    Banned,
}

impl Availability {
    pub fn in_loadout(self) -> bool {
        matches!(
            self,
            Availability::LoadoutAndPickup | Availability::LoadoutOnly
        )
    }

    pub fn as_pickup(self) -> bool {
        matches!(
            self,
            Availability::LoadoutAndPickup | Availability::PickupOnly
        )
    }

    pub fn is_banned(self) -> bool {
        self == Availability::Banned
    }
}

/// Weapons (and gadget) given at spawn.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Loadout {
    pub primaries: Vec<String>,
    /// Secondary weapon ids with their starting counts.
    pub secondaries: Vec<(String, u32)>,
    /// Gadget id (gadgets arrive in M2; must be `None` in M1).
    #[serde(default)]
    pub gadget: Option<String>,
}

/// Spawn weight and respawn time of a weapon pickup.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PickupRule {
    pub weight: f32,
    pub respawn_secs: f32,
    /// Ammo granted for count weapons (ignored otherwise).
    #[serde(default)]
    pub ammo: u32,
}

/// Scrap piles placed at pickup spawn points ("level junk" to scavenge).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScrapPickupRule {
    pub weight: f32,
    pub respawn_secs: f32,
    pub amount: f32,
}

fn default_teams() -> u8 {
    2
}
fn default_match_secs() -> f32 {
    300.0
}
fn default_respawn_secs() -> f32 {
    3.0
}

/// Paint the Void settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaintTheVoidRules {
    #[serde(default = "default_teams")]
    pub teams: u8,
    /// Match length (default 5 minutes).
    #[serde(default = "default_match_secs")]
    pub match_secs: f32,
    #[serde(default = "default_respawn_secs")]
    pub respawn_secs: f32,
}

impl Default for PaintTheVoidRules {
    fn default() -> Self {
        PaintTheVoidRules {
            teams: default_teams(),
            match_secs: default_match_secs(),
            respawn_secs: default_respawn_secs(),
        }
    }
}

/// Game mode and its settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ModeRules {
    PaintTheVoid(PaintTheVoidRules),
}

/// A complete rule set. Weapons missing from `weapons` count as [`Availability::Banned`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    pub name: String,
    pub mode: ModeRules,
    pub weapons: BTreeMap<String, Availability>,
    pub starting_loadout: Loadout,
    #[serde(default)]
    pub pickups: BTreeMap<String, PickupRule>,
    #[serde(default)]
    pub scrap_pickup: Option<ScrapPickupRule>,
    /// Damage dealt to teammates in percent: 0 = off, otherwise 10..=100.
    pub friendly_fire_percent: u8,
    pub allowed_ship_types: Vec<ShipType>,
    /// Comedy event frequency 0..=100 (the comedy director arrives in M4).
    #[serde(default)]
    pub absurdity: u8,
}

const PAINT_THE_VOID_RON: &str = include_str!("../../../data/rulesets/paint_the_void.ron");

impl RuleSet {
    /// Parses a rule set from RON (structure only; use [`RuleSet::resolve`] to
    /// validate it against a weapon table).
    pub fn from_ron(source_name: &str, text: &str) -> Result<RuleSet, SimError> {
        ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })
    }

    /// Loads a rule set file (e.g. `data/rulesets/paint_the_void.ron`).
    pub fn load(path: impl AsRef<Path>) -> Result<RuleSet, SimError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| SimError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        RuleSet::from_ron(&path.display().to_string(), &text)
    }

    /// The default Paint the Void rule set, embedded from `data/rulesets/paint_the_void.ron`.
    pub fn paint_the_void() -> RuleSet {
        RuleSet::from_ron("paint_the_void.ron", PAINT_THE_VOID_RON)
            .expect("built-in rule set is valid RON")
    }

    /// Availability of a weapon by string id (unlisted = banned).
    pub fn availability(&self, weapon: &str) -> Availability {
        self.weapons
            .get(weapon)
            .copied()
            .unwrap_or(Availability::Banned)
    }

    pub fn paint_the_void_rules(&self) -> PaintTheVoidRules {
        match self.mode {
            ModeRules::PaintTheVoid(r) => r,
        }
    }

    /// Validates the rule set against the loaded weapons and precomputes
    /// index-based tables for the simulation.
    pub fn resolve(&self, weapons: &WeaponTable) -> Result<ResolvedRules, SimError> {
        let bad = |m: String| SimError::InvalidRuleSet(m);
        if self.friendly_fire_percent > 100 || (1..10).contains(&self.friendly_fire_percent) {
            return Err(bad(format!(
                "friendly_fire_percent {} must be 0 (off) or 10..=100",
                self.friendly_fire_percent
            )));
        }
        if self.absurdity > 100 {
            return Err(bad(format!("absurdity {} > 100", self.absurdity)));
        }
        if self.allowed_ship_types.is_empty() {
            return Err(bad("allowed_ship_types is empty".into()));
        }
        let mode = self.paint_the_void_rules();
        if mode.teams != 2 {
            return Err(bad(format!(
                "Paint the Void supports 2 teams, got {}",
                mode.teams
            )));
        }
        if !(mode.match_secs.is_finite() && mode.match_secs > 0.0)
            || !(mode.respawn_secs.is_finite() && mode.respawn_secs >= 0.0)
        {
            return Err(bad("match_secs must be > 0 and respawn_secs >= 0".into()));
        }
        for id in self.weapons.keys() {
            weapons.require(id)?;
        }

        let availability: Vec<Availability> = weapons
            .iter()
            .map(|(_, d)| self.availability(&d.id))
            .collect();

        let loadout_weapon = |id: &str, slot: WeaponSlot| -> Result<WeaponId, SimError> {
            let w = weapons.require(id)?;
            let avail = availability[w.0 as usize];
            if avail.is_banned() {
                return Err(SimError::WeaponBanned(id.to_string()));
            }
            if !avail.in_loadout() {
                return Err(bad(format!(
                    "`{id}` is {avail:?} and cannot be in the loadout"
                )));
            }
            if weapons.get(w).slot != slot {
                return Err(bad(format!("`{id}` is not a {slot:?} weapon")));
            }
            Ok(w)
        };
        let mut loadout_primaries = Vec::new();
        for id in &self.starting_loadout.primaries {
            let w = loadout_weapon(id, WeaponSlot::Primary)?;
            if !loadout_primaries.contains(&w) {
                loadout_primaries.push(w);
            }
        }
        let mut loadout_secondaries: Vec<(WeaponId, u32)> = Vec::new();
        for (id, count) in &self.starting_loadout.secondaries {
            let w = loadout_weapon(id, WeaponSlot::Secondary)?;
            if loadout_secondaries.iter().any(|(x, _)| *x == w) {
                return Err(bad(format!("`{id}` listed twice in secondaries")));
            }
            loadout_secondaries.push((w, *count));
        }
        if self.starting_loadout.gadget.is_some() {
            return Err(bad("gadgets are not supported yet".into()));
        }

        let mut pickup_table = Vec::new();
        for (id, rule) in &self.pickups {
            let w = weapons.require(id)?;
            let avail = availability[w.0 as usize];
            if !avail.as_pickup() {
                return Err(bad(format!("`{id}` is {avail:?} and cannot be a pickup")));
            }
            if !(rule.weight.is_finite() && rule.weight >= 0.0)
                || !(rule.respawn_secs.is_finite() && rule.respawn_secs >= 0.0)
            {
                return Err(bad(format!(
                    "pickup `{id}` has an invalid weight or respawn"
                )));
            }
            let ammo = match weapons.get(w).cost {
                WeaponCost::Count(_) => rule.ammo,
                _ => 0,
            };
            pickup_table.push(PickupEntry {
                kind: PickupKind::Weapon { weapon: w, ammo },
                weight: rule.weight,
                respawn_ticks: Tick::ticks_for_secs(rule.respawn_secs).max(1),
            });
        }
        if let Some(s) = self.scrap_pickup {
            if !(s.weight.is_finite() && s.weight >= 0.0)
                || !(s.respawn_secs.is_finite() && s.respawn_secs >= 0.0)
                || !(s.amount.is_finite() && s.amount > 0.0)
            {
                return Err(bad("scrap_pickup has invalid values".into()));
            }
            pickup_table.push(PickupEntry {
                kind: PickupKind::Scrap { amount: s.amount },
                weight: s.weight,
                respawn_ticks: Tick::ticks_for_secs(s.respawn_secs).max(1),
            });
        }

        Ok(ResolvedRules {
            availability,
            loadout_primaries,
            loadout_secondaries,
            pickup_table,
            friendly_fire_percent: self.friendly_fire_percent,
            match_ticks: Tick::ticks_for_secs(mode.match_secs).max(1),
            respawn_ticks: Tick::ticks_for_secs(mode.respawn_secs),
            allowed_ship_types: self.allowed_ship_types.clone(),
        })
    }
}

/// What a pickup spawn point holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PickupKind {
    /// A weapon (grants it, adds ammo for count weapons, or upgrades it).
    Weapon { weapon: WeaponId, ammo: u32 },
    /// A scrap pile to scavenge.
    Scrap { amount: f32 },
}

/// One row of the resolved pickup table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickupEntry {
    pub kind: PickupKind,
    pub weight: f32,
    pub respawn_ticks: u64,
}

/// A validated rule set with weapon ids resolved to table indices.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRules {
    /// Indexed by [`WeaponId`].
    pub availability: Vec<Availability>,
    pub loadout_primaries: Vec<WeaponId>,
    pub loadout_secondaries: Vec<(WeaponId, u32)>,
    /// Weapon pickups sorted by weapon id, then the scrap entry.
    pub pickup_table: Vec<PickupEntry>,
    pub friendly_fire_percent: u8,
    pub match_ticks: u64,
    pub respawn_ticks: u64,
    pub allowed_ship_types: Vec<ShipType>,
}

impl ResolvedRules {
    pub fn availability(&self, w: WeaponId) -> Availability {
        self.availability
            .get(w.0 as usize)
            .copied()
            .unwrap_or(Availability::Banned)
    }

    /// Scales damage between teammates by the friendly-fire percentage.
    pub fn friendly_fire_scale(&self) -> f32 {
        self.friendly_fire_percent as f32 / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn default_rule_set_loads_and_resolves() {
        let rules = RuleSet::paint_the_void();
        assert_eq!(rules.name, "Paint the Void");
        assert_eq!(rules.friendly_fire_percent, 50);
        let mode = rules.paint_the_void_rules();
        assert_eq!(mode.teams, 2);
        assert_eq!(mode.match_secs, 300.0);
        let weapons = WeaponTable::builtin();
        let r = rules.resolve(&weapons).unwrap();
        assert_eq!(r.match_ticks, 300 * 60);
        // Design doc default loadout: Scrap Laser and 3 Bolt Rockets.
        let laser = weapons.find("scrap_laser").unwrap();
        let rocket = weapons.find("bolt_rocket").unwrap();
        assert!(r.loadout_primaries.contains(&laser));
        assert_eq!(r.loadout_secondaries, vec![(rocket, 3)]);
        // Weapon entries + scrap.
        assert_eq!(r.pickup_table.len(), 4);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../data/rulesets/paint_the_void.ron"
            );
            assert_eq!(RuleSet::load(path).unwrap(), rules);
        }
    }

    #[test]
    fn mode_defaults_apply() {
        let text = r#"RuleSet(
            name: "t", mode: PaintTheVoid(()), weapons: {}, starting_loadout: (primaries: [], secondaries: []),
            friendly_fire_percent: 0, allowed_ship_types: [Tinker])"#;
        let rules = RuleSet::from_ron("t", text).unwrap();
        assert_eq!(rules.paint_the_void_rules(), PaintTheVoidRules::default());
        assert_eq!(rules.availability("scrap_laser"), Availability::Banned);
        rules.resolve(&WeaponTable::builtin()).unwrap();
    }

    #[test]
    fn validation_errors() {
        let weapons = WeaponTable::builtin();
        let base = RuleSet::paint_the_void();

        let mut r = base.clone();
        r.friendly_fire_percent = 101;
        assert!(r.resolve(&weapons).is_err());
        for ff in [1, 9] {
            r.friendly_fire_percent = ff;
            assert!(r.resolve(&weapons).is_err(), "ff {ff}");
        }
        for ff in [0, 10, 100] {
            r.friendly_fire_percent = ff;
            assert!(r.resolve(&weapons).is_ok(), "ff {ff}");
        }

        let mut r = base.clone();
        r.weapons.insert("bolt_rocket".into(), Availability::Banned);
        r.pickups.remove("bolt_rocket");
        assert!(matches!(
            r.resolve(&weapons),
            Err(SimError::WeaponBanned(_))
        ));

        let mut r = base.clone();
        r.weapons
            .insert("rivet_gun".into(), Availability::LoadoutOnly);
        assert!(
            r.resolve(&weapons).is_err(),
            "LoadoutOnly cannot be a pickup"
        );

        let mut r = base.clone();
        r.starting_loadout.primaries.push("rivet_gun".into());
        assert!(
            r.resolve(&weapons).is_err(),
            "PickupOnly cannot be in loadout"
        );

        let mut r = base.clone();
        r.weapons
            .insert("laser_of_doom".into(), Availability::PickupOnly);
        assert!(matches!(
            r.resolve(&weapons),
            Err(SimError::UnknownWeapon(_))
        ));

        let mut r = base.clone();
        r.starting_loadout.primaries.push("bolt_rocket".into());
        assert!(r.resolve(&weapons).is_err(), "slot mismatch");

        let mut r = base;
        r.allowed_ship_types.clear();
        assert!(r.resolve(&weapons).is_err());
    }
}
