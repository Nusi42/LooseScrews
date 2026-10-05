//! Instagib: Railspike only, unlimited ammo, any hit kills; no pickups, no
//! gadgets; hull and energy hidden, boost kept; instant respawn in modes
//! with respawns. Works on every PvP mode.
//!
//! Design: `docs/milestone-3-plan.md` §3.2 (Instagib).
//!
//! - `rewrite_rules`: the effective rule set allows only the Railspike, as
//!   `LoadoutOnly` (Paint the Void also keeps its Paint Thrower, the mode's
//!   tool, which deals no damage); no secondaries, no gadgets, no pickups of
//!   any kind (weapons, upgrades, gadgets, scrap piles). Railspike tuning
//!   and shield overrides stay. The Railspike keeps its 270 u/s projectile,
//!   its piercing and its lasting effects (holes through thin rock).
//! - `unlimited_ammo`: the Railspike fires without paying its scrap.
//! - `on_damage`: a direct hit from an enemy's weapon (only the Railspike
//!   deals damage under these rules) takes the target's whole hull and
//!   ignores its gadget. Self-hits, splash, environment, crushing and ram
//!   damage stay as they are; teammates take normal Railspike damage scaled
//!   by the rule set's friendly fire.
//! - `respawn_ticks`: 0 (the world respawns the ship on the next tick) in
//!   every mode with respawns.
//! - `view_flags`: `INSTAGIB` (the HUD hides hull and energy; boost stays).

use crate::mode::view_flags;
use crate::modes::{ModeHit, ModeRef};
use crate::mutators::MutatorModule;
use crate::ruleset::{Availability, Loadout, ModeRules, Mutator, RuleSet};

/// The only weapon of an Instagib match.
pub const RAILSPIKE: &str = "railspike";
/// Paint the Void's tool, kept under Instagib in that mode.
const PAINT_THROWER: &str = "paint_thrower";

/// The Instagib mutator module.
#[derive(Debug)]
pub struct Instagib;

impl MutatorModule for Instagib {
    fn mutator(&self) -> Mutator {
        Mutator::Instagib
    }

    fn implemented(&self) -> bool {
        true
    }

    fn rewrite_rules(&self, rules: &mut RuleSet) {
        let keep_paint = matches!(rules.mode, ModeRules::PaintTheVoid(_));
        rules.weapons.clear();
        rules
            .weapons
            .insert(RAILSPIKE.to_string(), Availability::LoadoutOnly);
        let mut primaries = vec![RAILSPIKE.to_string()];
        if keep_paint {
            rules
                .weapons
                .insert(PAINT_THROWER.to_string(), Availability::LoadoutOnly);
            primaries.push(PAINT_THROWER.to_string());
        }
        rules.gadgets.clear();
        rules.starting_loadout = Loadout {
            primaries,
            secondaries: Vec::new(),
            gadget: None,
        };
        rules.pickups.clear();
        rules.upgrade_pickups.clear();
        rules.gadget_pickups.clear();
        rules.scrap_pickup = None;
        rules.tuning.retain(|id, _| id == RAILSPIKE);
        rules.shield_overrides.retain(|id, _| id == RAILSPIKE);
    }

    fn respawn_ticks(&self, _ticks: u64) -> u64 {
        0
    }

    fn unlimited_ammo(&self, weapon_id: &str) -> bool {
        weapon_id == RAILSPIKE
    }

    fn on_damage(&self, m: &ModeRef, hit: &mut ModeHit) {
        let enemy_shot = hit.direct
            && hit.weapon.is_some()
            && hit.attacker.is_some()
            && !hit.self_inflicted
            && !hit.same_side;
        if !enemy_shot {
            return;
        }
        let hull = m.ships.get(&hit.target).map_or(0.0, |s| s.hull);
        // The hull that is left (at least the shot's own damage), so the
        // damage event shows what the hit took.
        hit.amount = hit.amount.max(hull);
        hit.ignore_gadget = true;
    }

    fn view_flags(&self) -> u16 {
        view_flags::INSTAGIB
    }

    /// Energy and hull are hidden: no Tax drone, no Pizza delivery.
    fn allows_event(&self, kind: crate::events::EventKind) -> bool {
        !matches!(
            kind,
            crate::events::EventKind::TaxDrone | crate::events::EventKind::PizzaDelivery
        )
    }
}

#[cfg(test)]
mod tests {
    //! Instagib scenario tests: the rule rewrite (Railspike only, no
    //! pickups or gadgets), any enemy hit kills (not self-hits, splash or
    //! teammates), unlimited ammo, instant respawn, and the combinations
    //! with Team Brawl, The Squeeze and a duel.

    use ls_math::Vec3;

    use super::*;
    use crate::mode::{player_flags, ModeKind};
    use crate::modes::team_brawl::tests::{builtin, ended, frag, hit_with, room, world, Room};
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn instagib(mut r: RuleSet) -> RuleSet {
        r.mutators.push(Mutator::Instagib);
        r
    }

    fn rail(w: &World) -> WeaponId {
        w.weapons().find(RAILSPIKE).unwrap()
    }

    /// Places a ship at `pos` looking along `dir`, at rest.
    fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
        let s = w.ship_mut(id).unwrap();
        s.pos = pos;
        s.vel = Vec3::ZERO;
        s.orientation = ls_math::Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
    }

    fn fire(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
        [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_PRIMARY))]
    }

    /// A two-team Instagib Team Brawl world with players a (team 0), b
    /// (team 1) and a2 (team 0).
    fn duo() -> (World, Room, PlayerId, PlayerId, PlayerId) {
        let mut w = world(builtin("rulesets/instagib_brawl.ron"));
        let t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Hauler).unwrap();
        let a2 = w.add_player(Some(0), ShipType::Tinker).unwrap();
        (w, t, a, b, a2)
    }

    #[test]
    fn rewrite_leaves_only_the_railspike_without_pickups_or_gadgets() {
        let weapons = WeaponTable::builtin();
        let base = RuleSet::default_for(ModeKind::TeamBrawl);
        let r = instagib(base.clone());
        let e = r.effective();
        assert_eq!(e.weapons.len(), 1);
        assert_eq!(e.availability(RAILSPIKE), Availability::LoadoutOnly);
        assert!(e.gadgets.is_empty());
        assert_eq!(e.starting_loadout.primaries, vec![RAILSPIKE.to_string()]);
        assert!(e.starting_loadout.secondaries.is_empty());
        assert_eq!(e.starting_loadout.gadget, None);
        assert!(e.pickups.is_empty() && e.upgrade_pickups.is_empty());
        assert!(e.gadget_pickups.is_empty() && e.scrap_pickup.is_none());
        assert_eq!(e.mode, base.mode, "mode settings stay");
        assert_eq!(e.friendly_fire_percent, base.friendly_fire_percent);
        assert_eq!(e.effective(), e, "idempotent");
        let res = r.resolve(&weapons).unwrap();
        let rail = weapons.find(RAILSPIKE).unwrap();
        assert_eq!(res.loadout_primaries, vec![rail]);
        assert!(res.loadout_secondaries.is_empty());
        assert_eq!(res.loadout_gadget, None);
        assert!(res.pickup_table.is_empty());
        assert_eq!(res.respawn_ticks, 0, "instant respawn");
        assert!(res.free_fire(rail));
        assert!(!res.free_fire(weapons.find("scrap_laser").unwrap()));
        // Without the mutator nothing is free and respawns take 3 s.
        let plain = base.resolve(&weapons).unwrap();
        assert!(!plain.free_fire(rail));
        assert_eq!(plain.respawn_ticks, 180);
        // Paint the Void keeps its Paint Thrower (the mode's tool).
        let ptv = instagib(RuleSet::paint_the_void()).effective();
        assert_eq!(ptv.weapons.len(), 2);
        assert_eq!(
            ptv.starting_loadout.primaries,
            vec![RAILSPIKE.to_string(), PAINT_THROWER.to_string()]
        );
        // Every mode's default rule set resolves with Instagib.
        for kind in ModeKind::ALL {
            let r = instagib(RuleSet::default_for(kind));
            r.resolve(&weapons).unwrap();
            assert!(
                world(r).mode_view().flags & view_flags::INSTAGIB != 0,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn no_pickups_and_no_gadgets_in_the_match() {
        let (mut w, mut t, a, _, _) = duo();
        assert!(!w.pickups().is_empty(), "the level has pickup points");
        for _ in 0..600 {
            w.step(&mut t, &[]);
        }
        assert!(w.pickups().iter().all(|p| p.content.is_none()));
        let s = w.ship(a).unwrap();
        assert_eq!(s.gadget, None);
        assert_eq!(s.secondary, None);
        assert_eq!(s.primary, Some(rail(&w)));
        assert!(matches!(
            w.give_gadget(a, "shield"),
            Err(SimError::GadgetBanned(_))
        ));
        assert!(matches!(
            w.give_weapon(a, "scrap_laser"),
            Err(SimError::WeaponBanned(_))
        ));
    }

    #[test]
    fn a_railspike_shot_kills_at_once_through_a_shield() {
        let (mut w, mut t, a, b, _) = duo();
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        // Even an active shield does not help (gadgets are banned; forced
        // here to check the hit ignores it).
        let shield = w.gadgets().find("shield").unwrap();
        {
            let s = w.ship_mut(b).unwrap();
            s.gadget = Some(shield);
            s.gadget_active = true;
        }
        let hull = w.ship(b).unwrap().hull;
        assert!(
            hull > 35.0,
            "a normal Railspike hit would not kill a Hauler"
        );
        let mut ev = w.step(&mut t, &fire(a));
        for _ in 0..10 {
            ev.extend(w.step(&mut t, &[]));
        }
        assert!(ev.iter().any(|e| matches!(
            e,
            Event::Killed { victim, killer: Some(k), .. } if *victim == b && *k == a
        )));
        let dmg: Vec<f32> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Damaged { target, amount, .. } if *target == b => Some(*amount),
                _ => None,
            })
            .collect();
        assert_eq!(dmg, vec![hull], "one hit took the whole hull");
        assert_eq!(w.mode().team_scores[..2], [1, 0]);
    }

    #[test]
    fn self_hits_splash_and_teammates_take_normal_damage() {
        let (mut w, mut t, a, b, a2) = duo();
        let rail = Some(rail(&w));
        let hull = |w: &World, id| w.ship(id).unwrap().hull;
        let full = hull(&w, b);
        // Splash (not direct) from an enemy: normal damage.
        hit_with(&mut w, &mut t, a, b, 10.0, rail, false);
        assert_eq!(hull(&w, b), full - 10.0);
        // Damage without a weapon (ramming, environment): normal.
        hit_with(&mut w, &mut t, a, b, 10.0, None, true);
        assert_eq!(hull(&w, b), full - 20.0);
        // Self-hit: normal.
        let full_a = hull(&w, a);
        hit_with(&mut w, &mut t, a, a, 10.0, rail, true);
        assert_eq!(hull(&w, a), full_a - 10.0);
        // Teammate: the rule set's friendly fire (50 %).
        let full_a2 = hull(&w, a2);
        hit_with(&mut w, &mut t, a, a2, 10.0, rail, true);
        assert_eq!(hull(&w, a2), full_a2 - 5.0);
        assert!(w.ship(a2).unwrap().alive);
        // A direct enemy hit of any size kills.
        hit_with(&mut w, &mut t, a2, b, 0.5, rail, true);
        assert!(!w.ship(b).unwrap().alive);
    }

    #[test]
    fn the_railspike_never_runs_out() {
        let (mut w, mut t, a, _, _) = duo();
        place(&mut w, a, Vec3::new(0.0, 0.0, 10.0), Vec3::Y);
        w.ship_mut(a).unwrap().scrap = 0.0;
        let ev: Vec<Event> = (0..600).flat_map(|_| w.step(&mut t, &fire(a))).collect();
        let shots = ev
            .iter()
            .filter(|e| matches!(e, Event::Fired { .. }))
            .count();
        // 1.5 s fire interval: 7 shots in 10 s, all without scrap.
        assert!(shots >= 6, "{shots} shots");
        assert_eq!(w.ship(a).unwrap().scrap, 0.0);
        // Without Instagib the empty ship cannot fire the Railspike.
        let mut w = world(RuleSet::default_for(ModeKind::TeamBrawl));
        let p = w.add_player(None, ShipType::Tinker).unwrap();
        w.give_weapon(p, RAILSPIKE).unwrap();
        let rid = w.weapons().find(RAILSPIKE).unwrap();
        {
            let s = w.ship_mut(p).unwrap();
            s.primary = Some(rid);
            s.scrap = 0.0;
        }
        let ev: Vec<Event> = (0..120).flat_map(|_| w.step(&mut t, &fire(p))).collect();
        assert!(!ev.iter().any(|e| matches!(e, Event::Fired { .. })));
    }

    #[test]
    fn respawn_is_instant() {
        let (mut w, mut t, a, b, _) = duo();
        w.step(&mut t, &[]);
        frag(&mut w, &mut t, a, b);
        assert!(!w.ship(b).unwrap().alive);
        let ev = w.step(&mut t, &[]);
        assert!(ev
            .iter()
            .any(|e| matches!(e, Event::Respawned { player, .. } if *player == b)));
        assert!(w.ship(b).unwrap().alive);
    }

    #[test]
    fn instagib_team_brawl_ends_at_the_kill_target() {
        let mut r = builtin("rulesets/instagib_brawl.ron");
        r.mode.set_field("kill_target", 2.0);
        let mut w = world(r);
        let mut t = room();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        let rail = Some(rail(&w));
        hit_with(&mut w, &mut t, b, a, 1.0, rail, true);
        assert_eq!(ended(&w.step(&mut t, &[])), None);
        hit_with(&mut w, &mut t, b, a, 1.0, rail, true);
        assert_eq!(ended(&w.step(&mut t, &[])), Some(Some(1)));
    }

    #[test]
    fn instagib_squeeze_one_hit_eliminates() {
        let mut w = world(instagib(RuleSet::default_for(ModeKind::TheSqueeze)));
        let mut t = room();
        let ids: Vec<PlayerId> = (0..3)
            .map(|_| w.add_player(None, ShipType::Hauler).unwrap())
            .collect();
        let rail = Some(rail(&w));
        assert_eq!(w.ship(ids[0]).unwrap().primary, rail);
        w.step(&mut t, &[]);
        hit_with(&mut w, &mut t, ids[1], ids[0], 1.0, rail, true);
        let ev = w.step(&mut t, &[]);
        assert!(ev.contains(&Event::Eliminated { player: ids[0] }));
        // No respawn without lives, instant or not.
        for _ in 0..120 {
            w.step(&mut t, &[]);
        }
        assert!(!w.ship(ids[0]).unwrap().alive);
        hit_with(&mut w, &mut t, ids[1], ids[2], 1.0, rail, true);
        let ev = w.step(&mut t, &[]);
        let side = w.ship(ids[1]).unwrap().team;
        assert_eq!(ended(&ev), Some(Some(side)));
        let v = w.mode_view();
        assert!(v
            .players
            .iter()
            .filter(|p| p.player != ids[1])
            .all(|p| p.flags & player_flags::ELIMINATED != 0));
    }

    #[test]
    fn instagib_duel_first_to_ten() {
        let mut w = world(instagib(builtin("rulesets/duel.ron")));
        let mut t = room();
        let a = w.add_player(None, ShipType::Tinker).unwrap();
        let b = w.add_player(None, ShipType::Skitter).unwrap();
        assert_eq!(w.mode().score_target, 10);
        let rail = Some(rail(&w));
        let mut result = None;
        for k in 0..10 {
            assert_eq!(result, None, "ended after {k}");
            hit_with(&mut w, &mut t, a, b, 1.0, rail, true);
            // Instant respawn: b is back one tick later.
            let ev = w.step(&mut t, &[]);
            result = ended(&ev);
            if result.is_none() {
                assert!(w.ship(b).unwrap().alive);
            }
        }
        let side = w.ship(a).unwrap().team;
        assert_eq!(result, Some(Some(side)));
        let res = w.results();
        assert_eq!((res.rows[0].player, res.rows[0].kills), (a, 10));
    }
}
