//! Gadgets: the one gadget slot of a ship (Shield, Cloak, Decoy).
//!
//! Like weapons, a gadget is data ([`GadgetDef`], `data/gadgets/<id>.ron`)
//! plus a behaviour module (`src/gadgets/<id>.rs`) implementing
//! [`GadgetBehavior`]; [`MODULES`] registers both in one place.
//!
//! The framework (in [`crate::World::step`]) handles activation with the
//! gadget key ([`crate::Buttons::GADGET`], G, on the press edge): toggle
//! gadgets switch on (paying `energy_cost`) or off; instant gadgets fire
//! [`GadgetBehavior::on_activate`] and start the cooldown. While a toggle
//! gadget is on it drains `drain_per_sec` energy (times `moving_drain_mult`
//! while the ship moves) and switches off at zero energy. Hooks:
//!
//! | Hook | When |
//! | --- | --- |
//! | `on_activate` / `on_deactivate` | switched on / off (instant: on use) |
//! | `tick` | every tick while on (toggle gadgets) |
//! | `on_damage` | the carrier takes damage while the gadget is on |
//! | `visible_to` | snapshot relevance: may `viewer` see the carrier? |
//! | `entity_tick` / `entity_expired` | entities this gadget spawned (decoys) |

pub mod cloak;
pub mod decoy;
pub mod shield;
#[cfg(test)]
mod test_util;

use std::collections::BTreeMap;
use std::fmt::Debug;

use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::behavior::Ctx;
use crate::entity::Entity;
use crate::ship::{PlayerId, ShipState};
use crate::visual::Visual;
use crate::{ShieldInteraction, SimError, WeaponId};

/// How the gadget key works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GadgetActivation {
    /// Press to switch on, press again to switch off (Shield, Cloak).
    Toggle,
    /// Press to use once, then cooldown (Decoy).
    Instant,
}

/// One gadget's numbers (`data/gadgets/<id>.ron`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GadgetDef {
    pub id: String,
    pub name: String,
    pub activation: GadgetActivation,
    /// Energy paid on activation.
    #[serde(default)]
    pub energy_cost: f32,
    /// Energy per second while switched on (toggle gadgets).
    #[serde(default)]
    pub drain_per_sec: f32,
    /// Drain multiplier while the ship moves (Cloak: 3x).
    #[serde(default = "one")]
    pub moving_drain_mult: f32,
    /// Seconds after activation (or switching off) before the next use.
    #[serde(default)]
    pub cooldown_secs: f32,
    /// Effect duration for instant gadgets (Decoy lifetime).
    #[serde(default)]
    pub duration_secs: f32,
    #[serde(default)]
    pub visual: Visual,
    /// Free numeric parameters for the module.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

fn one() -> f32 {
    1.0
}

impl GadgetDef {
    pub fn from_ron(source_name: &str, text: &str) -> Result<GadgetDef, SimError> {
        let def: GadgetDef = ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })?;
        def.validate()?;
        Ok(def)
    }

    pub fn validate(&self) -> Result<(), SimError> {
        let bad = |reason: &str| SimError::InvalidGadget {
            id: self.id.clone(),
            reason: reason.to_string(),
        };
        let non_neg = |v: f32| v.is_finite() && v >= 0.0;
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(bad("id must be non-empty snake_case"));
        }
        for v in [
            self.energy_cost,
            self.drain_per_sec,
            self.moving_drain_mult,
            self.cooldown_secs,
            self.duration_secs,
        ] {
            if !non_neg(v) {
                return Err(bad("costs, rates and durations must be >= 0"));
            }
        }
        if self.params.values().any(|v| !v.is_finite()) {
            return Err(bad("params must be finite"));
        }
        Ok(())
    }

    pub fn param(&self, key: &str, default: f32) -> f32 {
        self.params.get(key).copied().unwrap_or(default)
    }

    pub fn cooldown_ticks(&self) -> u64 {
        ls_math::Tick::ticks_for_secs(self.cooldown_secs)
    }

    pub fn duration_ticks(&self) -> u64 {
        ls_math::Tick::ticks_for_secs(self.duration_secs).max(1)
    }
}

/// Index of a gadget in a [`GadgetTable`] (sorted by id).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct GadgetId(pub u8);

/// All loaded gadgets, sorted by id.
#[derive(Debug, Clone, PartialEq)]
pub struct GadgetTable {
    defs: Vec<GadgetDef>,
}

impl GadgetTable {
    pub fn new(mut defs: Vec<GadgetDef>) -> Result<GadgetTable, SimError> {
        if defs.len() > u8::MAX as usize {
            return Err(SimError::InvalidGadget {
                id: String::new(),
                reason: "at most 255 gadgets".into(),
            });
        }
        for d in &defs {
            d.validate()?;
        }
        defs.sort_by(|a, b| a.id.cmp(&b.id));
        for pair in defs.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(SimError::InvalidGadget {
                    id: pair[0].id.clone(),
                    reason: "duplicate id".into(),
                });
            }
        }
        Ok(GadgetTable { defs })
    }

    /// The built-in gadgets from `data/gadgets` (embedded by [`MODULES`]).
    pub fn builtin() -> GadgetTable {
        let defs = MODULES
            .iter()
            .map(|m| GadgetDef::from_ron(m.file, m.data))
            .collect::<Result<Vec<_>, _>>()
            .expect("built-in gadget data is valid");
        GadgetTable::new(defs).expect("built-in gadget data is valid")
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn get(&self, id: GadgetId) -> &GadgetDef {
        &self.defs[id.0 as usize]
    }

    pub fn find(&self, id: &str) -> Option<GadgetId> {
        self.defs
            .binary_search_by(|d| d.id.as_str().cmp(id))
            .ok()
            .map(|i| GadgetId(i as u8))
    }

    pub fn require(&self, id: &str) -> Result<GadgetId, SimError> {
        self.find(id)
            .ok_or_else(|| SimError::UnknownGadget(id.to_string()))
    }

    pub fn iter(&self) -> impl Iterator<Item = (GadgetId, &GadgetDef)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, d)| (GadgetId(i as u8), d))
    }
}

impl Default for GadgetTable {
    fn default() -> Self {
        GadgetTable::builtin()
    }
}

/// A hit on a ship whose gadget is on, as the gadget sees it; the gadget
/// may reduce `amount` (and pay for it with the ship's energy).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IncomingHit {
    /// Hull damage after friendly-fire scaling.
    pub amount: f32,
    pub knockback: Vec3,
    pub attacker: Option<PlayerId>,
    pub weapon: Option<WeaponId>,
    /// Shield setting of the weapon (rule set override applied);
    /// `Wears(1.0)` for damage without a weapon.
    pub shield: ShieldInteraction,
    /// The carrier hit itself (own splash, ricochet, returning echo).
    pub self_inflicted: bool,
}

/// What a gadget activation is about.
#[derive(Debug, Clone, Copy)]
pub struct GadgetUse<'a> {
    pub gadget: GadgetId,
    pub def: &'a GadgetDef,
    pub ship: &'a ShipState,
}

/// Behaviour of one gadget. Every hook has a default that does nothing, so a
/// module only overrides what it needs. See the module docs for when each runs.
pub trait GadgetBehavior: Send + Sync + Debug {
    /// The gadget id (`data/gadgets/<id>.ron`).
    fn id(&self) -> &'static str;

    /// True once the module implements the design doc fully.
    fn implemented(&self) -> bool {
        false
    }

    fn on_activate(&self, _ctx: &mut Ctx, _g: &GadgetUse) {}

    fn on_deactivate(&self, _ctx: &mut Ctx, _g: &GadgetUse) {}

    fn tick(&self, _ctx: &mut Ctx, _g: &GadgetUse) {}

    /// Called while the gadget is on and the carrier is hit. May lower
    /// `hit.amount` and drain `ship.energy`; setting `ship.gadget_active =
    /// false` switches the gadget off.
    fn on_damage(&self, _def: &GadgetDef, _ship: &mut ShipState, _hit: &mut IncomingHit) {}

    /// Whether `viewer` receives the carrier `ship` in snapshots.
    fn visible_to(
        &self,
        _def: &GadgetDef,
        _ship: &ShipState,
        _viewer: &ShipState,
        _now: u64,
    ) -> bool {
        true
    }

    /// Every tick for each live entity this gadget spawned.
    fn entity_tick(&self, _ctx: &mut Ctx, _def: &GadgetDef, _e: &mut Entity) {}

    /// When an entity this gadget spawned expires (before it is removed).
    fn entity_expired(&self, _ctx: &mut Ctx, _def: &GadgetDef, _e: &mut Entity) {}
}

/// One registered gadget: behaviour plus embedded data file.
#[derive(Debug, Clone, Copy)]
pub struct GadgetModule {
    pub id: &'static str,
    pub behavior: &'static dyn GadgetBehavior,
    pub file: &'static str,
    pub data: &'static str,
}

/// Every gadget in design-doc order. Register new gadgets here (behaviour
/// module + data file); rule set codes use this order.
pub static MODULES: [GadgetModule; 3] = [
    GadgetModule {
        id: "shield",
        behavior: &shield::Shield,
        file: "gadgets/shield.ron",
        data: include_str!("../../../../data/gadgets/shield.ron"),
    },
    GadgetModule {
        id: "cloak",
        behavior: &cloak::Cloak,
        file: "gadgets/cloak.ron",
        data: include_str!("../../../../data/gadgets/cloak.ron"),
    },
    GadgetModule {
        id: "decoy",
        behavior: &decoy::Decoy,
        file: "gadgets/decoy.ron",
        data: include_str!("../../../../data/gadgets/decoy.ron"),
    },
];

/// Gadget without behaviour (data loaded from elsewhere): toggles and drains
/// energy, nothing else.
#[derive(Debug)]
pub struct NoGadget;

impl GadgetBehavior for NoGadget {
    fn id(&self) -> &'static str {
        ""
    }
}

/// Behaviour for a gadget id.
pub fn behavior(id: &str) -> &'static dyn GadgetBehavior {
    MODULES
        .iter()
        .find(|m| m.id == id)
        .map_or(&NoGadget as &'static dyn GadgetBehavior, |m| m.behavior)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn registry_matches_data() {
        let t = GadgetTable::builtin();
        assert_eq!(t.len(), MODULES.len());
        for m in &MODULES {
            let g = t.require(m.id).unwrap();
            assert_eq!(t.get(g).id, m.id);
            assert_eq!(m.behavior.id(), m.id);
            assert_eq!(behavior(m.id).id(), m.id);
        }
        assert_eq!(behavior("nope").id(), "");
        let cloak = t.get(t.find("cloak").unwrap());
        assert_eq!(cloak.moving_drain_mult, 3.0, "design doc: 3x while moving");
        let decoy = t.get(t.find("decoy").unwrap());
        assert_eq!(decoy.activation, GadgetActivation::Instant);
        assert_eq!(decoy.param("return_fraction", 0.0), 0.99);
    }

    #[test]
    fn rejects_bad_defs() {
        let mut d = GadgetTable::builtin().get(GadgetId(0)).clone();
        d.drain_per_sec = -1.0;
        assert!(d.validate().is_err());
        let d = GadgetTable::builtin().get(GadgetId(0)).clone();
        assert!(GadgetTable::new(vec![d.clone(), d]).is_err());
    }
}
