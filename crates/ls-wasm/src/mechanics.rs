//! Views for the client's mechanics UI (M2 wave C): game cues for effects
//! and banners, the charge meter, the gadget state, and ship type choice
//! with its loadout preview. Plain data built by `GameCore`; `lib.rs`
//! flattens it for JavaScript.

use ls_math::Vec3;
use ls_sim::{
    Cue, CueKind, GadgetActivation, GadgetTable, ResolvedRules, ShipTable, ShipType, WeaponTable,
};

/// A game cue (`ls_sim::CueKind`) for effects: blinding, warnings, beams,
/// muzzle flashes, shockwaves, gadget switches, decoy returns, overcharge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CueView {
    /// `ls_sim::CueKind` byte.
    pub kind: u8,
    /// Ship ids (wire id online, player id in practice), 255 for none.
    pub source: u8,
    pub target: u8,
    pub pos: Vec3,
    /// Kind-specific number (ticks, radius, gadget id).
    pub value: u16,
    /// Server tick of the cue.
    pub tick: u64,
}

impl CueView {
    /// From a simulation cue (practice).
    pub fn from_sim(c: &Cue, tick: u64) -> CueView {
        let id = |p: Option<ls_sim::PlayerId>| p.map_or(u8::MAX, |p| p.0.min(254) as u8);
        CueView {
            kind: c.kind as u8,
            source: id(c.source),
            target: id(c.target),
            pos: c.pos,
            value: c.value,
            tick,
        }
    }

    pub fn kind(&self) -> CueKind {
        CueKind::from_u8(self.kind)
    }
}

/// Cues kept until the host takes them (a frame normally takes all).
pub const MAX_CUES: usize = 128;

/// Pushes a cue, dropping the oldest beyond [`MAX_CUES`].
pub fn push_cue(cues: &mut Vec<CueView>, c: CueView) {
    if cues.len() >= MAX_CUES {
        cues.remove(0);
    }
    cues.push(c);
}

/// The charge in progress of a hold-to-charge weapon (Fusion Charger,
/// Polite Laser), for the charge meter.
#[derive(Debug, Clone, PartialEq)]
pub struct ChargeView {
    pub weapon: u8,
    pub name: String,
    pub held_secs: f32,
    /// Seconds to full charge.
    pub full_secs: f32,
    /// Releasing before this cancels the shot.
    pub min_secs: f32,
    /// `held / full`, capped at 1.
    pub fraction: f32,
    /// Fires by itself at full charge (Polite Laser).
    pub auto_release: bool,
    /// Held past full charge while the weapon burns hull (Fusion Charger).
    pub overcharge: bool,
}

impl ChargeView {
    /// A charge of `weapon` held `held` ticks.
    pub fn new(weapons: &WeaponTable, weapon: ls_sim::WeaponId, held: u64) -> Option<ChargeView> {
        if (weapon.0 as usize) >= weapons.len() {
            return None;
        }
        let def = weapons.get(weapon);
        let cd = def.charge?;
        let full = ls_sim::WeaponDef::charge_ticks(cd.max_secs);
        let hz = ls_math::TICK_HZ as f32;
        let burns = def.param("overcharge_hull_per_sec", 0.0) > 0.0;
        Some(ChargeView {
            weapon: weapon.0,
            name: def.name.clone(),
            held_secs: held as f32 / hz,
            full_secs: full as f32 / hz,
            min_secs: cd.min_secs,
            fraction: (held as f32 / full as f32).min(1.0),
            auto_release: cd.auto_release,
            overcharge: burns && held > full,
        })
    }
}

/// The local ship's gadget for the HUD.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GadgetView {
    /// Gadget id (`ls_sim::GadgetId`), 255 for none.
    pub id: u8,
    /// Gadget string id (`shield`, `cloak`, `decoy`), empty for none.
    pub key: String,
    pub name: String,
    pub active: bool,
    /// Toggle gadget (Shield, Cloak) rather than instant (Decoy).
    pub toggle: bool,
    /// Seconds until it can be used again (0 = ready).
    pub cooldown_left: f32,
    pub cooldown_secs: f32,
    pub energy_cost: f32,
    pub drain_per_sec: f32,
}

impl GadgetView {
    /// View of gadget `id` (`None` = no gadget).
    pub fn new(gadgets: &GadgetTable, id: Option<u8>, active: bool, cooldown_left: f32) -> Self {
        let Some(g) = id.filter(|&g| (g as usize) < gadgets.len()) else {
            return GadgetView {
                id: u8::MAX,
                ..GadgetView::default()
            };
        };
        let def = gadgets.get(ls_sim::GadgetId(g));
        GadgetView {
            id: g,
            key: def.id.clone(),
            name: def.name.clone(),
            active,
            toggle: def.activation == GadgetActivation::Toggle,
            cooldown_left: cooldown_left.max(0.0),
            cooldown_secs: def.cooldown_secs,
            energy_cost: def.energy_cost,
            drain_per_sec: def.drain_per_sec,
        }
    }
}

/// Display name of a ship type (`"Skitter"`, ...).
pub fn ship_type_title(t: ShipType) -> &'static str {
    match t {
        ShipType::Skitter => "Skitter",
        ShipType::Tinker => "Tinker",
        ShipType::Hauler => "Hauler",
        ShipType::Tug => "Tug",
    }
}

/// Laser speed the design doc compares ship speeds with (u/s).
pub const LASER_SPEED: f32 = 30.0;

/// One ship type in the ship selection panel, with its loadout preview.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipChoice {
    /// `ShipType::index`.
    pub ship_type: u8,
    pub name: String,
    /// The rule set allows it.
    pub allowed: bool,
    /// The ship flown this life.
    pub current: bool,
    /// Chosen for the next respawn.
    pub next: bool,
    pub hull: f32,
    /// Top speed (boost, on the diagonal) in percent of the laser's speed.
    pub top_speed_pct: f32,
    pub damage_mult: f32,
    pub secondary_slots: u8,
    pub radius: f32,
    /// Turn rate (rad/s).
    pub turn_rate: f32,
    pub can_pass_scrap: bool,
    /// Loadout the ship spawns with under the rule set.
    pub primaries: Vec<String>,
    /// Secondaries with their starting counts (as many as the ship has slots).
    pub secondaries: Vec<(String, u32)>,
    pub gadget: String,
}

/// Ship choices for every ship type, in index order.
pub fn ship_choices(
    ships: &ShipTable,
    weapons: &WeaponTable,
    gadgets: &GadgetTable,
    rules: &ResolvedRules,
    current: ShipType,
    next: Option<ShipType>,
) -> Vec<ShipChoice> {
    ShipType::ALL
        .iter()
        .map(|&t| {
            let tu = ships.get(t);
            let name = |w: ls_sim::WeaponId| {
                if (w.0 as usize) < weapons.len() {
                    weapons.get(w).name.clone()
                } else {
                    String::from("?")
                }
            };
            ShipChoice {
                ship_type: t.index(),
                name: ship_type_title(t).to_string(),
                allowed: rules.allowed_ship_types.contains(&t),
                current: t == current,
                next: next == Some(t) && t != current,
                hull: tu.hull_max,
                top_speed_pct: tu.boost_speed * 3f32.sqrt() / LASER_SPEED * 100.0,
                damage_mult: tu.damage_mult,
                secondary_slots: tu.secondary_slots,
                radius: tu.radius,
                turn_rate: tu.max_turn_rate,
                can_pass_scrap: tu.can_pass_scrap,
                primaries: rules.loadout_primaries.iter().map(|&w| name(w)).collect(),
                secondaries: rules
                    .loadout_secondaries
                    .iter()
                    .take(tu.secondary_slots as usize)
                    .map(|&(w, n)| (name(w), n.min(rules.max_ammo(w))))
                    .collect(),
                gadget: rules
                    .loadout_gadget
                    .filter(|g| (g.0 as usize) < gadgets.len())
                    .map_or_else(String::new, |g| gadgets.get(g).name.clone()),
            }
        })
        .collect()
}

/// The ship type to fly when `wanted` may not be allowed: `wanted`, else
/// the Tinker, else the first allowed type (like the server's `Room`).
pub fn allowed_or_fallback(rules: &ResolvedRules, wanted: ShipType) -> ShipType {
    let ok = |t: ShipType| rules.allowed_ship_types.contains(&t);
    if ok(wanted) {
        wanted
    } else if ok(ShipType::Tinker) {
        ShipType::Tinker
    } else {
        rules
            .allowed_ship_types
            .first()
            .copied()
            .unwrap_or(ShipType::Tinker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_sim::{RuleSet, WeaponId};

    fn tables() -> (ShipTable, WeaponTable, GadgetTable) {
        (
            ShipTable::builtin(),
            WeaponTable::builtin(),
            GadgetTable::builtin(),
        )
    }

    #[test]
    fn ship_choices_follow_the_rule_set_and_ship_data() {
        let (ships, weapons, gadgets) = tables();
        let mut rules = RuleSet::paint_the_void();
        rules.allowed_ship_types = vec![ShipType::Skitter, ShipType::Hauler];
        let resolved = rules.resolve_with(&weapons, &gadgets).unwrap();
        let c = ship_choices(
            &ships,
            &weapons,
            &gadgets,
            &resolved,
            ShipType::Skitter,
            Some(ShipType::Hauler),
        );
        assert_eq!(c.len(), 4);
        let allowed: Vec<bool> = c.iter().map(|s| s.allowed).collect();
        assert_eq!(allowed, [true, false, true, false]);
        assert!(c[0].current && !c[0].next);
        assert!(c[2].next && !c[2].current);
        assert_eq!(c[0].name, "Skitter");
        // Design doc: Skitter 70 hull at 98% of laser speed, one secondary slot.
        assert_eq!(c[0].hull, 70.0);
        assert!(
            (c[0].top_speed_pct - 98.0).abs() < 0.5,
            "{}",
            c[0].top_speed_pct
        );
        assert!(c[0].secondaries.len() <= c[0].secondary_slots as usize);
        assert!(c[2].secondaries.len() >= c[0].secondaries.len());
        assert!(!c[1].primaries.is_empty());
        assert_eq!(c[1].primaries, c[3].primaries, "same rule set loadout");
    }

    #[test]
    fn fallback_picks_an_allowed_type() {
        let (_, weapons, gadgets) = tables();
        let mut rules = RuleSet::paint_the_void();
        rules.allowed_ship_types = vec![ShipType::Tug];
        let r = rules.resolve_with(&weapons, &gadgets).unwrap();
        assert_eq!(allowed_or_fallback(&r, ShipType::Skitter), ShipType::Tug);
        let r = RuleSet::paint_the_void()
            .resolve_with(&weapons, &gadgets)
            .unwrap();
        assert_eq!(allowed_or_fallback(&r, ShipType::Hauler), ShipType::Hauler);
    }

    #[test]
    fn charge_view_reports_fraction_and_overcharge() {
        let weapons = WeaponTable::builtin();
        let fusion = weapons.find("fusion_charger").unwrap();
        let polite = weapons.find("polite_laser").unwrap();
        let half = ChargeView::new(&weapons, fusion, 60).unwrap();
        assert!((half.fraction - 0.5).abs() < 1e-6);
        assert!(!half.overcharge && !half.auto_release);
        let over = ChargeView::new(&weapons, fusion, 150).unwrap();
        assert_eq!(over.fraction, 1.0);
        assert!(over.overcharge);
        let p = ChargeView::new(&weapons, polite, 90).unwrap();
        assert!(p.auto_release && !p.overcharge);
        assert_eq!(p.fraction, 1.0);
        let laser = weapons.find("scrap_laser").unwrap();
        assert!(ChargeView::new(&weapons, laser, 10).is_none());
        assert!(ChargeView::new(&weapons, WeaponId(250), 10).is_none());
    }

    #[test]
    fn gadget_view_describes_the_gadget() {
        let gadgets = GadgetTable::builtin();
        let shield = gadgets.find("shield").unwrap();
        let v = GadgetView::new(&gadgets, Some(shield.0), true, -1.0);
        assert_eq!(v.key, "shield");
        assert!(v.toggle && v.active);
        assert_eq!(v.cooldown_left, 0.0);
        let decoy = gadgets.find("decoy").unwrap();
        assert!(!GadgetView::new(&gadgets, Some(decoy.0), false, 2.0).toggle);
        assert_eq!(GadgetView::new(&gadgets, None, false, 0.0).id, u8::MAX);
    }

    #[test]
    fn cues_convert_and_are_capped() {
        let c = Cue {
            kind: CueKind::Blinded,
            source: Some(ls_sim::PlayerId(3)),
            target: None,
            pos: Vec3::X,
            value: 120,
        };
        let v = CueView::from_sim(&c, 9);
        assert_eq!((v.kind, v.source, v.target, v.value), (1, 3, 255, 120));
        assert_eq!(v.kind(), CueKind::Blinded);
        let mut q = Vec::new();
        for i in 0..MAX_CUES + 5 {
            push_cue(
                &mut q,
                CueView {
                    tick: i as u64,
                    ..v
                },
            );
        }
        assert_eq!(q.len(), MAX_CUES);
        assert_eq!(q[0].tick, 5);
    }
}
