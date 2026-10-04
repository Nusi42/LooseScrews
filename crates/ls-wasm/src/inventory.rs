//! Carried weapons for the weapon HUD and weapon selection by key and cycle
//! (M2 key scheme, `docs/milestone-2-plan.md`: 1-0 select carried primaries
//! in design-doc order, the mouse wheel cycles primaries, X cycles
//! secondaries, G uses the gadget).
//!
//! - [`LoadoutView`]: what the HUD lists (both modes).
//! - [`SelectRequest`] + [`resolve_select`]: turns a key press or a cycle step
//!   into the `InputCmd::weapon_select` value the simulation understands.
//! - [`Inventory`]: online, the own ship's weapons, ammo, levels, selection
//!   and gadget as the server reports them (`ls_net::Message::Inventory`,
//!   sent whenever they change), with the player's latest selection shown
//!   until the server has processed it. Only used for display and
//!   selection, never for game state.

use std::collections::BTreeMap;

use ls_net::InventoryMsg;
use ls_sim::{GadgetId, ResolvedRules, WeaponDef, WeaponId, WeaponSlot, WeaponState, WeaponTable};

/// Number keys that select primaries (`1`..`9`, then `0` for the tenth).
pub const WEAPON_KEYS: u8 = 10;

/// Position of a weapon in the design doc (the registry order,
/// `ls_sim::weapons::MODULES`); weapons not in the registry sort after
/// those, by id.
pub fn design_rank(weapons: &WeaponTable, w: WeaponId) -> usize {
    if (w.0 as usize) >= weapons.len() {
        return usize::MAX;
    }
    let id = &weapons.get(w).id;
    ls_sim::weapons::MODULES
        .iter()
        .position(|m| m.id == id)
        .unwrap_or(ls_sim::weapons::MODULES.len() + w.0 as usize)
}

/// Sorts weapon ids into design-doc order.
pub fn sort_design_order(weapons: &WeaponTable, ids: &mut [WeaponId]) {
    ids.sort_by_key(|&w| (design_rank(weapons, w), w));
}

/// One carried weapon as the HUD shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponEntry {
    pub weapon: WeaponId,
    pub name: String,
    pub slot: WeaponSlot,
    /// Ammo text (`ammo_text`: shots left, count, or `∞`).
    pub ammo: String,
    pub level: u8,
    pub max_level: u8,
    pub selected: bool,
    /// Number key (1-10, 10 = the `0` key) for primaries; 0 for none.
    pub key: u8,
    /// Paid shots fired with it (`WeaponState::shots`, the pattern phase).
    pub shots: u32,
    /// Low byte of its module state (`ShipState::weapon_state`, e.g. the
    /// Paint Thrower colour; 0 = unset). Filled by
    /// [`LoadoutView::set_weapon_states`].
    pub state: u8,
}

/// Carried weapons and the gadget, for the weapon HUD.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoadoutView {
    /// Primaries in design-doc order (key 1 first).
    pub primaries: Vec<WeaponEntry>,
    /// Secondaries in design-doc order (X cycles through them).
    pub secondaries: Vec<WeaponEntry>,
    /// Gadget name (empty for none).
    pub gadget: String,
    pub gadget_active: bool,
}

impl LoadoutView {
    /// Builds the view from carried weapon states and the selection.
    /// `ammo_text` gives the ammo text of a weapon with its count.
    pub fn build(
        weapons: &WeaponTable,
        carried: &[WeaponState],
        primary: Option<WeaponId>,
        secondary: Option<WeaponId>,
        ammo_text: impl Fn(&WeaponDef, u32) -> String,
    ) -> LoadoutView {
        let mut ids: Vec<WeaponId> = carried
            .iter()
            .map(|s| s.weapon)
            .filter(|w| (w.0 as usize) < weapons.len())
            .collect();
        sort_design_order(weapons, &mut ids);
        ids.dedup();
        let mut view = LoadoutView::default();
        for w in ids {
            let def = weapons.get(w);
            let st = carried.iter().find(|s| s.weapon == w);
            let ammo = st.map_or(0, |s| s.ammo);
            let mut e = WeaponEntry {
                weapon: w,
                name: def.name.clone(),
                slot: def.slot,
                ammo: ammo_text(def, ammo),
                level: st.map_or(1, |s| s.level),
                max_level: def.max_level,
                selected: false,
                key: 0,
                shots: st.map_or(0, |s| s.shots),
                state: 0,
            };
            match def.slot {
                WeaponSlot::Primary => {
                    e.selected = primary == Some(w);
                    let n = view.primaries.len() as u8 + 1;
                    e.key = if n <= WEAPON_KEYS { n } else { 0 };
                    view.primaries.push(e);
                }
                WeaponSlot::Secondary => {
                    e.selected = secondary == Some(w);
                    view.secondaries.push(e);
                }
            }
        }
        view
    }

    /// Fills every entry's module state byte.
    pub fn set_weapon_states(&mut self, state: impl Fn(WeaponId) -> u8) {
        for e in self.primaries.iter_mut().chain(&mut self.secondaries) {
            e.state = state(e.weapon);
        }
    }

    pub fn primary_ids(&self) -> Vec<WeaponId> {
        self.primaries.iter().map(|e| e.weapon).collect()
    }

    pub fn secondary_ids(&self) -> Vec<WeaponId> {
        self.secondaries.iter().map(|e| e.weapon).collect()
    }
}

/// A selection request from one frame of controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectRequest {
    /// Number key 1-10 (10 = the `0` key), 0 = none.
    pub key: u8,
    /// Mouse wheel: +1 next primary, -1 previous, 0 none.
    pub cycle_primary: i8,
    /// X: +1 next secondary, 0 none.
    pub cycle_secondary: i8,
}

impl SelectRequest {
    pub fn is_empty(&self) -> bool {
        self.key == 0 && self.cycle_primary == 0 && self.cycle_secondary == 0
    }
}

/// The weapon a request selects, given the carried primaries and
/// secondaries in design-doc order and the current selection. A number key
/// wins over a wheel step, which wins over X. `None` if nothing changes.
pub fn resolve_select(
    req: SelectRequest,
    primaries: &[WeaponId],
    primary: Option<WeaponId>,
    secondaries: &[WeaponId],
    secondary: Option<WeaponId>,
) -> Option<WeaponId> {
    if req.key > 0 {
        return primaries.get(req.key as usize - 1).copied();
    }
    let step = |list: &[WeaponId], cur: Option<WeaponId>, dir: i8| -> Option<WeaponId> {
        if list.is_empty() || dir == 0 {
            return None;
        }
        let n = list.len() as i64;
        let next = match cur.and_then(|c| list.iter().position(|&w| w == c)) {
            Some(i) => (i as i64 + dir.signum() as i64).rem_euclid(n),
            None if dir > 0 => 0,
            None => n - 1,
        };
        let w = list[next as usize];
        (Some(w) != cur).then_some(w)
    };
    step(primaries, primary, req.cycle_primary)
        .or_else(|| step(secondaries, secondary, req.cycle_secondary))
}

/// The own ship's weapons and gadget online (see the module docs).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inventory {
    weapons: BTreeMap<WeaponId, WeaponState>,
    /// Module state bytes from the server (`NetWeapon::state`).
    states: BTreeMap<WeaponId, u8>,
    pub primary: Option<WeaponId>,
    pub secondary: Option<WeaponId>,
    pub gadget: Option<GadgetId>,
}

impl Inventory {
    /// The rule set's loadout, like `ls_sim` gives it at every spawn.
    /// `slots` is the ship type's number of secondary slots.
    pub fn loadout(rules: &ResolvedRules, slots: u8) -> Inventory {
        let mut inv = Inventory::default();
        for &w in &rules.loadout_primaries {
            inv.weapons.insert(
                w,
                WeaponState {
                    weapon: w,
                    ammo: 0,
                    level: 1,
                    shots: 0,
                },
            );
            inv.primary.get_or_insert(w);
        }
        for &(w, count) in rules.loadout_secondaries.iter().take(slots as usize) {
            inv.weapons.insert(
                w,
                WeaponState {
                    weapon: w,
                    ammo: count.min(rules.max_ammo(w)),
                    level: 1,
                    shots: 0,
                },
            );
            inv.secondary.get_or_insert(w);
        }
        inv.gadget = rules.loadout_gadget;
        inv
    }

    /// Carried weapons sorted by id.
    pub fn states(&self) -> Vec<WeaponState> {
        self.weapons.values().copied().collect()
    }

    pub fn carries(&self, w: WeaponId) -> bool {
        self.weapons.contains_key(&w)
    }

    pub fn ammo(&self, w: WeaponId) -> u32 {
        self.weapons.get(&w).map_or(0, |s| s.ammo)
    }

    /// Module state byte of a carried weapon as the server sent it (0 =
    /// unset or unknown).
    pub fn weapon_state(&self, w: WeaponId) -> u8 {
        self.states.get(&w).copied().unwrap_or(0)
    }

    /// Server shot counts per carried weapon (the shot predictor's phases).
    pub fn shot_counts(&self) -> Vec<(WeaponId, u32)> {
        self.weapons.values().map(|s| (s.weapon, s.shots)).collect()
    }

    /// The inventory the server sent. Weapons outside the table are dropped.
    pub fn from_server(msg: &InventoryMsg, weapons: &WeaponTable) -> Inventory {
        let known = |w: u8| ((w as usize) < weapons.len()).then_some(WeaponId(w));
        let mut inv = Inventory::default();
        for x in &msg.weapons {
            if let Some(w) = known(x.weapon) {
                inv.weapons.insert(
                    w,
                    WeaponState {
                        weapon: w,
                        ammo: x.ammo as u32,
                        level: x.level.max(1),
                        shots: x.shots,
                    },
                );
                inv.states.insert(w, x.state);
            }
        }
        inv.primary = known(msg.primary).filter(|w| inv.carries(*w));
        inv.secondary = known(msg.secondary).filter(|w| inv.carries(*w));
        inv.gadget = (msg.gadget != u8::MAX).then_some(GadgetId(msg.gadget));
        inv
    }

    /// Applies a selection like `ls_sim::World` does (only carried weapons).
    pub fn select(&mut self, w: WeaponId, weapons: &WeaponTable) {
        if (w.0 as usize) >= weapons.len() || !self.carries(w) {
            return;
        }
        match weapons.get(w).slot {
            WeaponSlot::Primary => self.primary = Some(w),
            WeaponSlot::Secondary => self.secondary = Some(w),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> WeaponTable {
        WeaponTable::builtin()
    }

    fn id(t: &WeaponTable, s: &str) -> WeaponId {
        t.find(s).unwrap_or_else(|| panic!("weapon {s}"))
    }

    #[test]
    fn design_order_follows_the_registry() {
        let t = table();
        let mut ids: Vec<WeaponId> = t.iter().map(|(w, _)| w).collect();
        sort_design_order(&t, &mut ids);
        let order: Vec<&str> = ids.iter().map(|&w| t.get(w).id.as_str()).collect();
        let registry: Vec<&str> = ls_sim::weapons::MODULES.iter().map(|m| m.id).collect();
        assert_eq!(order, registry);
        assert_eq!(order[0], "scrap_laser");
        assert_eq!(design_rank(&t, WeaponId(250)), usize::MAX);
    }

    #[test]
    fn loadout_view_numbers_primaries_in_design_order() {
        let t = table();
        let laser = id(&t, "scrap_laser");
        let rivet = id(&t, "rivet_gun");
        let paint = id(&t, "paint_thrower");
        let rocket = id(&t, "bolt_rocket");
        let st = |weapon, ammo| WeaponState {
            weapon,
            ammo,
            level: 1,
            shots: 0,
        };
        // Deliberately unsorted input.
        let carried = [st(rocket, 3), st(paint, 0), st(rivet, 0), st(laser, 0)];
        let v = LoadoutView::build(&t, &carried, Some(rivet), Some(rocket), |_, n| {
            n.to_string()
        });
        let names: Vec<&str> = v.primaries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Scrap Laser", "Rivet Gun", "Paint Thrower"]);
        assert_eq!(
            v.primaries.iter().map(|e| e.key).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(v.primaries[1].selected && !v.primaries[0].selected);
        assert_eq!(v.secondaries.len(), 1);
        assert!(v.secondaries[0].selected);
        assert_eq!(v.secondaries[0].ammo, "3");
        assert_eq!(v.secondaries[0].key, 0);
    }

    #[test]
    fn keys_wheel_and_x_resolve_to_weapons() {
        let (a, b, c) = (WeaponId(4), WeaponId(1), WeaponId(9));
        let (s1, s2) = (WeaponId(2), WeaponId(7));
        let prim = [a, b, c];
        let sec = [s1, s2];
        let r = |req| resolve_select(req, &prim, Some(a), &sec, Some(s1));
        let key = |k| SelectRequest {
            key: k,
            ..Default::default()
        };
        assert_eq!(r(key(2)), Some(b));
        assert_eq!(r(key(4)), None, "no fourth primary carried");
        assert_eq!(r(SelectRequest::default()), None);
        let wheel = |d| SelectRequest {
            cycle_primary: d,
            ..Default::default()
        };
        assert_eq!(r(wheel(1)), Some(b));
        assert_eq!(r(wheel(-1)), Some(c), "wraps backwards");
        assert_eq!(
            resolve_select(wheel(1), &prim, Some(c), &sec, None),
            Some(a),
            "wraps forwards"
        );
        assert_eq!(
            resolve_select(wheel(1), &[a], Some(a), &sec, None),
            None,
            "only one primary: nothing changes"
        );
        let x = SelectRequest {
            cycle_secondary: 1,
            ..Default::default()
        };
        assert_eq!(r(x), Some(s2));
        assert_eq!(resolve_select(x, &prim, None, &sec, Some(s2)), Some(s1));
        assert_eq!(resolve_select(x, &prim, None, &sec, None), Some(s1));
        // A key wins over the wheel, the wheel over X.
        let all = SelectRequest {
            key: 3,
            cycle_primary: 1,
            cycle_secondary: 1,
        };
        assert_eq!(r(all), Some(c));
        assert_eq!(
            r(SelectRequest {
                cycle_primary: 1,
                cycle_secondary: 1,
                ..Default::default()
            }),
            Some(b)
        );
    }

    #[test]
    fn inventory_from_the_loadout_and_the_server() {
        let t = table();
        let rules = ls_sim::RuleSet::paint_the_void().resolve(&t).unwrap();
        let inv = Inventory::loadout(&rules, 1);
        let laser = id(&t, "scrap_laser");
        let rocket = id(&t, "bolt_rocket");
        assert!(inv.carries(laser));
        assert_eq!(inv.primary, rules.loadout_primaries.first().copied());
        assert_eq!(inv.ammo(rocket), 3);

        let rivet = id(&t, "rivet_gun");
        let seeker = id(&t, "seeker");
        let msg = InventoryMsg {
            tick: 10,
            weapons: vec![
                ls_net::NetWeapon {
                    shots: 0,
                    state: 0,
                    weapon: laser.0,
                    ammo: 0,
                    level: 2,
                },
                ls_net::NetWeapon {
                    shots: 0,
                    state: 0,
                    weapon: rivet.0,
                    ammo: 0,
                    level: 1,
                },
                ls_net::NetWeapon {
                    shots: 0,
                    state: 0,
                    weapon: seeker.0,
                    ammo: 2,
                    level: 1,
                },
                // Unknown to this client's table: dropped.
                ls_net::NetWeapon {
                    shots: 0,
                    state: 0,
                    weapon: 200,
                    ammo: 1,
                    level: 1,
                },
            ],
            primary: rivet.0,
            secondary: seeker.0,
            gadget: 255,
        };
        let mut inv = Inventory::from_server(&msg, &t);
        assert_eq!(inv.states().len(), 3);
        assert_eq!(inv.primary, Some(rivet));
        assert_eq!(inv.secondary, Some(seeker));
        assert_eq!(inv.ammo(seeker), 2);
        assert_eq!(inv.gadget, None);
        assert!(!inv.carries(rocket));
        // Selecting something not carried is ignored.
        inv.select(rocket, &t);
        assert_eq!(inv.secondary, Some(seeker));
        inv.select(laser, &t);
        assert_eq!(inv.primary, Some(laser));
    }
}
