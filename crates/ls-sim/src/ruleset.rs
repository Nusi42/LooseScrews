//! Match rule sets loaded from `data/rulesets/*.ron` (rule set v2: the full
//! match rules of the design doc).
//!
//! | Setting | Field |
//! | --- | --- |
//! | Weapon availability | `weapons` (unlisted = `Banned`) |
//! | Gadget availability | `gadgets` (unlisted = `Banned`) |
//! | Starting loadout | `starting_loadout` (primaries, secondaries with counts, gadget) |
//! | Pickup spawns | `pickups` (weapons), `upgrade_pickups`, `gadget_pickups`, `scrap_pickup`: weight, respawn time, ammo, per-map cap |
//! | Shield interaction | `shield_overrides` (per weapon; default from the weapon data) |
//! | Weapon tuning | `tuning` (per weapon damage, fire rate and ammo multipliers; default 1x) |
//! | Ship types | `allowed_ship_types` |
//! | Friendly fire | `friendly_fire_percent` (0 = off, otherwise 10..=100) |
//! | Absurdity, mutators | `absurdity` (0..=100), `mutators` (none supported yet) |
//!
//! [`RuleSet::resolve`] validates a rule set against the loaded weapons and
//! gadgets and precomputes index-based tables for the simulation. Rule sets
//! also travel as short codes (`crate::rulecode`).

use std::collections::BTreeMap;
use std::path::Path;

use ls_math::Tick;
use serde::{Deserialize, Serialize};

use crate::{
    GadgetId, GadgetTable, ShieldInteraction, ShipType, SimError, WeaponCost, WeaponId, WeaponSlot,
    WeaponTable,
};

/// Per-weapon (and per-gadget) availability (design doc, Match rules).
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

    /// Stable index (rule set codes).
    pub fn index(self) -> u8 {
        match self {
            Availability::LoadoutAndPickup => 0,
            Availability::PickupOnly => 1,
            Availability::LoadoutOnly => 2,
            Availability::Banned => 3,
        }
    }

    pub fn from_index(i: u8) -> Availability {
        match i & 3 {
            0 => Availability::LoadoutAndPickup,
            1 => Availability::PickupOnly,
            2 => Availability::LoadoutOnly,
            _ => Availability::Banned,
        }
    }
}

/// Weapons and gadget given at spawn.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Loadout {
    pub primaries: Vec<String>,
    /// Secondary weapon ids with their starting counts.
    pub secondaries: Vec<(String, u32)>,
    /// Gadget id for the gadget slot.
    #[serde(default)]
    pub gadget: Option<String>,
}

/// Spawn weight and respawn time of a pickup.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PickupRule {
    pub weight: f32,
    pub respawn_secs: f32,
    /// Ammo granted for count weapons (ignored otherwise).
    #[serde(default)]
    pub ammo: u32,
    /// At most this many on the map at once (0 = no limit; Tectonic Charge: 1).
    #[serde(default)]
    pub max_on_map: u32,
}

/// Scrap piles placed at pickup spawn points ("level junk" to scavenge).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScrapPickupRule {
    pub weight: f32,
    pub respawn_secs: f32,
    pub amount: f32,
}

/// Per-weapon multipliers (design doc: weapon tuning, default 1x).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WeaponTuning {
    #[serde(default = "one")]
    pub damage: f32,
    /// Shots per second multiplier (the fire interval is divided by it).
    #[serde(default = "one")]
    pub fire_rate: f32,
    /// Ammo multiplier for count weapons (loadout counts, pickups, max ammo).
    #[serde(default = "one")]
    pub ammo: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for WeaponTuning {
    fn default() -> Self {
        WeaponTuning {
            damage: 1.0,
            fire_rate: 1.0,
            ammo: 1.0,
        }
    }
}

/// Match mutators (design doc). None is implemented yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mutator {
    Instagib,
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

/// A complete rule set. Weapons and gadgets missing from `weapons` /
/// `gadgets` count as [`Availability::Banned`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    pub name: String,
    pub mode: ModeRules,
    pub weapons: BTreeMap<String, Availability>,
    #[serde(default)]
    pub gadgets: BTreeMap<String, Availability>,
    pub starting_loadout: Loadout,
    #[serde(default)]
    pub pickups: BTreeMap<String, PickupRule>,
    /// Upgrade pickups: raise the level of a carried weapon (Scrap Laser 1..6).
    #[serde(default)]
    pub upgrade_pickups: BTreeMap<String, PickupRule>,
    #[serde(default)]
    pub gadget_pickups: BTreeMap<String, PickupRule>,
    #[serde(default)]
    pub scrap_pickup: Option<ScrapPickupRule>,
    /// Per-weapon shield interaction overrides (the main balancing lever).
    #[serde(default)]
    pub shield_overrides: BTreeMap<String, ShieldInteraction>,
    /// Per-weapon damage, fire rate and ammo multipliers.
    #[serde(default)]
    pub tuning: BTreeMap<String, WeaponTuning>,
    /// Damage dealt to teammates in percent: 0 = off, otherwise 10..=100.
    pub friendly_fire_percent: u8,
    pub allowed_ship_types: Vec<ShipType>,
    /// Comedy event frequency 0..=100 (the comedy director arrives in M4).
    #[serde(default)]
    pub absurdity: u8,
    #[serde(default)]
    pub mutators: Vec<Mutator>,
}

const PAINT_THE_VOID_RON: &str = include_str!("../../../data/rulesets/paint_the_void.ron");

/// Embedded rule set files (data hash).
pub const BUILTIN_RULESETS: [(&str, &str); 1] =
    [("rulesets/paint_the_void.ron", PAINT_THE_VOID_RON)];

impl RuleSet {
    /// Parses a rule set from RON (structure only; use [`RuleSet::resolve`] to
    /// validate it against the weapon and gadget tables).
    pub fn from_ron(source_name: &str, text: &str) -> Result<RuleSet, SimError> {
        ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })
    }

    /// Pretty RON text (what `data/rulesets/*.ron` files look like).
    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .expect("rule sets always serialize")
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

    /// Availability of a gadget by string id (unlisted = banned).
    pub fn gadget_availability(&self, gadget: &str) -> Availability {
        self.gadgets
            .get(gadget)
            .copied()
            .unwrap_or(Availability::Banned)
    }

    pub fn paint_the_void_rules(&self) -> PaintTheVoidRules {
        match self.mode {
            ModeRules::PaintTheVoid(r) => r,
        }
    }

    /// [`RuleSet::resolve_with`] against the built-in gadgets.
    pub fn resolve(&self, weapons: &WeaponTable) -> Result<ResolvedRules, SimError> {
        self.resolve_with(weapons, &GadgetTable::builtin())
    }

    /// Validates the rule set against the loaded weapons and gadgets and
    /// precomputes index-based tables for the simulation.
    pub fn resolve_with(
        &self,
        weapons: &WeaponTable,
        gadgets: &GadgetTable,
    ) -> Result<ResolvedRules, SimError> {
        let bad = |m: String| SimError::InvalidRuleSet(m);
        let finite_non_neg = |v: f32| v.is_finite() && v >= 0.0;
        if self.friendly_fire_percent > 100 || (1..10).contains(&self.friendly_fire_percent) {
            return Err(bad(format!(
                "friendly_fire_percent {} must be 0 (off) or 10..=100",
                self.friendly_fire_percent
            )));
        }
        if self.absurdity > 100 {
            return Err(bad(format!("absurdity {} > 100", self.absurdity)));
        }
        if let Some(m) = self.mutators.first() {
            return Err(bad(format!("mutator {m:?} is not supported yet")));
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
            || !finite_non_neg(mode.respawn_secs)
        {
            return Err(bad("match_secs must be > 0 and respawn_secs >= 0".into()));
        }
        for id in self.weapons.keys() {
            weapons.require(id)?;
        }
        for id in self.gadgets.keys() {
            gadgets.require(id)?;
        }

        let availability: Vec<Availability> = weapons
            .iter()
            .map(|(_, d)| self.availability(&d.id))
            .collect();
        let gadget_availability: Vec<Availability> = gadgets
            .iter()
            .map(|(_, d)| self.gadget_availability(&d.id))
            .collect();

        // Tuning multipliers and shield settings, indexed by weapon id.
        let mut weapon_tuning = vec![WeaponTuning::default(); weapons.len()];
        for (id, t) in &self.tuning {
            let w = weapons.require(id)?;
            if !finite_non_neg(t.damage)
                || !(t.fire_rate.is_finite() && t.fire_rate > 0.0)
                || !finite_non_neg(t.ammo)
            {
                return Err(bad(format!(
                    "tuning for `{id}` needs damage >= 0, fire_rate > 0, ammo >= 0"
                )));
            }
            weapon_tuning[w.0 as usize] = *t;
        }
        let mut shield: Vec<ShieldInteraction> = weapons.iter().map(|(_, d)| d.shield).collect();
        for (id, s) in &self.shield_overrides {
            let w = weapons.require(id)?;
            match *s {
                ShieldInteraction::PassThrough(f) if !(0.0..=1.0).contains(&f) => {
                    return Err(bad(format!(
                        "shield override for `{id}`: fraction in 0..=1"
                    )));
                }
                ShieldInteraction::Wears(f) if !finite_non_neg(f) => {
                    return Err(bad(format!("shield override for `{id}`: factor >= 0")));
                }
                _ => {}
            }
            shield[w.0 as usize] = *s;
        }
        let scale_ammo = |w: WeaponId, n: u32| -> u32 {
            let m = weapon_tuning[w.0 as usize].ammo;
            if n == 0 || m == 1.0 {
                return n;
            }
            let v = ls_math::det::round(n as f32 * m);
            (v.clamp(1.0, u32::MAX as f32)) as u32
        };
        let max_ammo: Vec<u32> = weapons
            .iter()
            .map(|(w, d)| scale_ammo(w, d.max_ammo))
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
            loadout_secondaries.push((w, scale_ammo(w, *count)));
        }
        let loadout_gadget = match &self.starting_loadout.gadget {
            None => None,
            Some(id) => {
                let g = gadgets.require(id)?;
                let avail = gadget_availability[g.0 as usize];
                if !avail.in_loadout() {
                    return Err(bad(format!(
                        "gadget `{id}` is {avail:?} and cannot be in the loadout"
                    )));
                }
                Some(g)
            }
        };

        let check_rule = |what: &str, id: &str, rule: &PickupRule| -> Result<(), SimError> {
            if !finite_non_neg(rule.weight) || !finite_non_neg(rule.respawn_secs) {
                return Err(bad(format!(
                    "{what} pickup `{id}` has an invalid weight or respawn"
                )));
            }
            Ok(())
        };
        let entry = |kind: PickupKind, rule: &PickupRule| PickupEntry {
            kind,
            weight: rule.weight,
            respawn_ticks: Tick::ticks_for_secs(rule.respawn_secs).max(1),
            max_on_map: rule.max_on_map,
        };
        let mut pickup_table = Vec::new();
        for (id, rule) in &self.pickups {
            let w = weapons.require(id)?;
            let avail = availability[w.0 as usize];
            if !avail.as_pickup() {
                return Err(bad(format!("`{id}` is {avail:?} and cannot be a pickup")));
            }
            check_rule("weapon", id, rule)?;
            let ammo = match weapons.get(w).cost {
                WeaponCost::Count(_) => scale_ammo(w, rule.ammo),
                _ => 0,
            };
            pickup_table.push(entry(PickupKind::Weapon { weapon: w, ammo }, rule));
        }
        for (id, rule) in &self.upgrade_pickups {
            let w = weapons.require(id)?;
            let avail = availability[w.0 as usize];
            if avail.is_banned() {
                return Err(SimError::WeaponBanned(id.to_string()));
            }
            if !avail.as_pickup() {
                return Err(bad(format!(
                    "`{id}` is {avail:?}; its upgrades cannot be pickups"
                )));
            }
            if weapons.get(w).max_level <= 1 {
                return Err(bad(format!("`{id}` has no upgrade levels")));
            }
            check_rule("upgrade", id, rule)?;
            pickup_table.push(entry(PickupKind::Upgrade { weapon: w }, rule));
        }
        for (id, rule) in &self.gadget_pickups {
            let g = gadgets.require(id)?;
            let avail = gadget_availability[g.0 as usize];
            if !avail.as_pickup() {
                return Err(bad(format!(
                    "gadget `{id}` is {avail:?} and cannot be a pickup"
                )));
            }
            check_rule("gadget", id, rule)?;
            pickup_table.push(entry(PickupKind::Gadget { gadget: g }, rule));
        }
        if let Some(s) = self.scrap_pickup {
            if !finite_non_neg(s.weight)
                || !finite_non_neg(s.respawn_secs)
                || !(s.amount.is_finite() && s.amount > 0.0)
            {
                return Err(bad("scrap_pickup has invalid values".into()));
            }
            pickup_table.push(PickupEntry {
                kind: PickupKind::Scrap { amount: s.amount },
                weight: s.weight,
                respawn_ticks: Tick::ticks_for_secs(s.respawn_secs).max(1),
                max_on_map: 0,
            });
        }

        Ok(ResolvedRules {
            availability,
            gadget_availability,
            loadout_primaries,
            loadout_secondaries,
            loadout_gadget,
            pickup_table,
            shield,
            weapon_tuning,
            max_ammo,
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
    /// Raises the level of a carried weapon (never grants it).
    Upgrade { weapon: WeaponId },
    /// Puts a gadget into the gadget slot (replacing the old one).
    Gadget { gadget: GadgetId },
    /// A scrap pile to scavenge.
    Scrap { amount: f32 },
}

/// One row of the resolved pickup table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickupEntry {
    pub kind: PickupKind,
    pub weight: f32,
    pub respawn_ticks: u64,
    /// At most this many on the map at once (0 = no limit).
    pub max_on_map: u32,
}

/// A validated rule set with weapon and gadget ids resolved to table indices.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRules {
    /// Indexed by [`WeaponId`].
    pub availability: Vec<Availability>,
    /// Indexed by [`GadgetId`].
    pub gadget_availability: Vec<Availability>,
    pub loadout_primaries: Vec<WeaponId>,
    /// Secondaries with their starting counts (ammo multiplier applied).
    pub loadout_secondaries: Vec<(WeaponId, u32)>,
    pub loadout_gadget: Option<GadgetId>,
    /// Weapon pickups sorted by weapon id, then upgrades, gadgets, scrap.
    pub pickup_table: Vec<PickupEntry>,
    /// Shield interaction per weapon (override applied).
    pub shield: Vec<ShieldInteraction>,
    /// Tuning multipliers per weapon.
    pub weapon_tuning: Vec<WeaponTuning>,
    /// Ammo cap per weapon (ammo multiplier applied).
    pub max_ammo: Vec<u32>,
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

    pub fn gadget_availability(&self, g: GadgetId) -> Availability {
        self.gadget_availability
            .get(g.0 as usize)
            .copied()
            .unwrap_or(Availability::Banned)
    }

    /// Scales damage between teammates by the friendly-fire percentage.
    pub fn friendly_fire_scale(&self) -> f32 {
        self.friendly_fire_percent as f32 / 100.0
    }

    /// Shield setting of a weapon (rule set override applied).
    pub fn shield_interaction(&self, w: WeaponId) -> ShieldInteraction {
        self.shield
            .get(w.0 as usize)
            .copied()
            .unwrap_or(ShieldInteraction::Wears(1.0))
    }

    pub fn tuning(&self, w: WeaponId) -> WeaponTuning {
        self.weapon_tuning
            .get(w.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    pub fn max_ammo(&self, w: WeaponId) -> u32 {
        self.max_ammo.get(w.0 as usize).copied().unwrap_or(0)
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
        let gadgets = GadgetTable::builtin();
        let r = rules.resolve(&weapons).unwrap();
        assert_eq!(r.match_ticks, 300 * 60);
        // Design doc default loadout: Scrap Laser and 3 Bolt Rockets.
        let laser = weapons.find("scrap_laser").unwrap();
        let rocket = weapons.find("bolt_rocket").unwrap();
        assert!(r.loadout_primaries.contains(&laser));
        assert_eq!(r.loadout_secondaries, vec![(rocket, 3)]);
        assert_eq!(r.loadout_gadget, gadgets.find("shield"));
        // Every weapon of the design doc is listed.
        assert_eq!(rules.weapons.len(), weapons.len());
        assert_eq!(rules.allowed_ship_types.len(), 4);
        // Tectonic Charge: 1 per map, 90 s respawn.
        let tectonic = weapons.find("tectonic_charge").unwrap();
        let e = r
            .pickup_table
            .iter()
            .find(|e| matches!(e.kind, PickupKind::Weapon { weapon, .. } if weapon == tectonic))
            .unwrap();
        assert_eq!((e.max_on_map, e.respawn_ticks), (1, 90 * 60));
        assert!(r
            .pickup_table
            .iter()
            .any(|e| matches!(e.kind, PickupKind::Upgrade { weapon } if weapon == laser)));
        assert!(r
            .pickup_table
            .iter()
            .any(|e| matches!(e.kind, PickupKind::Gadget { .. })));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../data/rulesets/paint_the_void.ron"
            );
            assert_eq!(RuleSet::load(path).unwrap(), rules);
        }
        // The pretty RON round-trips.
        assert_eq!(RuleSet::from_ron("x", &rules.to_ron()).unwrap(), rules);
    }

    #[test]
    fn mode_defaults_apply() {
        let text = r#"RuleSet(
            name: "t", mode: PaintTheVoid(()), weapons: {}, starting_loadout: (primaries: [], secondaries: []),
            friendly_fire_percent: 0, allowed_ship_types: [Tinker])"#;
        let rules = RuleSet::from_ron("t", text).unwrap();
        assert_eq!(rules.paint_the_void_rules(), PaintTheVoidRules::default());
        assert_eq!(rules.availability("scrap_laser"), Availability::Banned);
        assert_eq!(rules.gadget_availability("shield"), Availability::Banned);
        rules.resolve(&WeaponTable::builtin()).unwrap();
    }

    #[test]
    fn tuning_and_shield_overrides_resolve() {
        let weapons = WeaponTable::builtin();
        let mut r = RuleSet::paint_the_void();
        r.tuning.insert(
            "bolt_rocket".into(),
            WeaponTuning {
                damage: 1.5,
                fire_rate: 2.0,
                ammo: 2.0,
            },
        );
        r.shield_overrides
            .insert("scrap_laser".into(), ShieldInteraction::Blocked);
        let res = r.resolve(&weapons).unwrap();
        let rocket = weapons.find("bolt_rocket").unwrap();
        let laser = weapons.find("scrap_laser").unwrap();
        assert_eq!(res.tuning(rocket).damage, 1.5);
        assert_eq!(res.loadout_secondaries, vec![(rocket, 6)], "ammo x2");
        assert_eq!(res.max_ammo(rocket), 20);
        assert_eq!(res.shield_interaction(laser), ShieldInteraction::Blocked);
        let paint = weapons.find("paint_thrower").unwrap();
        assert_eq!(res.shield_interaction(paint), ShieldInteraction::Blocked);

        let mut bad = r.clone();
        bad.tuning.get_mut("bolt_rocket").unwrap().fire_rate = 0.0;
        assert!(bad.resolve(&weapons).is_err());
        let mut bad = r.clone();
        bad.shield_overrides
            .insert("rivet_gun".into(), ShieldInteraction::PassThrough(2.0));
        assert!(bad.resolve(&weapons).is_err());
        let mut bad = r;
        bad.mutators.push(Mutator::Instagib);
        assert!(bad.resolve(&weapons).is_err());
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

        let mut r = base.clone();
        r.gadgets.insert("shield".into(), Availability::Banned);
        assert!(r.resolve(&weapons).is_err(), "banned loadout gadget");

        let mut r = base.clone();
        r.gadget_pickups.insert(
            "jetpack".into(),
            PickupRule {
                weight: 1.0,
                respawn_secs: 1.0,
                ammo: 0,
                max_on_map: 0,
            },
        );
        assert!(r.resolve(&weapons).is_err(), "unknown gadget");

        let mut r = base.clone();
        r.upgrade_pickups.insert(
            "rivet_gun".into(),
            PickupRule {
                weight: 1.0,
                respawn_secs: 1.0,
                ammo: 0,
                max_on_map: 0,
            },
        );
        assert!(r.resolve(&weapons).is_err(), "no upgrade levels");

        let mut r = base;
        r.allowed_ship_types.clear();
        assert!(r.resolve(&weapons).is_err());
    }
}
