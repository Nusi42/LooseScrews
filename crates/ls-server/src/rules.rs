//! Match rules from the command line: `--ruleset <file or LR code>`,
//! `--ban <weapon_id>` (repeatable) and `--balance-list <file>`.
//!
//! The effective rules are the rule set with the balance list and the bans
//! applied on top ([`ServerRules::effective`]). They are validated at
//! startup, sent to every client in the `Welcome` (the rule set RON there is
//! the effective one) and logged with their rule set code. The balance list
//! file is re-read before every new match, so a host can ban or retune a
//! weapon without restarting; a list that fails to load then keeps the
//! previous rules (with a warning).

use std::path::{Path, PathBuf};

use ls_sim::ruleset::BalanceList;
use ls_sim::{rulecode, GadgetTable, RuleSet, WeaponTable};

use crate::server::ServerError;

/// Loads a rule set from a file path, an `LR...` rule set code or a
/// built-in rule set id (`paint_the_void`, `sandbox_all_weapons`, ...: the
/// file name in `data/rulesets/` without `.ron`). An existing file wins;
/// then built-in ids; text that looks like a code is decoded; anything else
/// is reported as a missing file.
pub fn load_rule_set(arg: &str) -> Result<RuleSet, ServerError> {
    let path = Path::new(arg);
    if !path.is_file() {
        if let Some(text) = crate::playlist::builtin_rule_set(arg) {
            return Ok(RuleSet::from_ron(&format!("{arg}.ron"), text)?);
        }
        if rulecode::looks_like_code(arg) {
            return Ok(RuleSet::from_code(arg)?);
        }
    }
    Ok(RuleSet::load(path)?)
}

/// Where the server's rules come from.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerRules {
    /// The rule set (`--ruleset`, or the built-in Paint the Void rules).
    pub base: RuleSet,
    /// Weapons banned on the command line (`--ban`).
    pub bans: Vec<String>,
    /// Balance list file (`--balance-list`), re-read before every match.
    pub balance_list: Option<PathBuf>,
}

impl Default for ServerRules {
    fn default() -> Self {
        ServerRules {
            base: RuleSet::paint_the_void(),
            bans: Vec::new(),
            balance_list: None,
        }
    }
}

impl ServerRules {
    /// Loads the rule set (file or code) and checks the bans and the
    /// balance list against the built-in weapons and gadgets.
    pub fn new(
        ruleset: Option<&str>,
        bans: Vec<String>,
        balance_list: Option<PathBuf>,
    ) -> Result<ServerRules, ServerError> {
        let base = match ruleset {
            Some(arg) => load_rule_set(arg)?,
            None => RuleSet::paint_the_void(),
        };
        let rules = ServerRules {
            base,
            bans,
            balance_list,
        };
        rules.effective(&WeaponTable::builtin(), &GadgetTable::builtin())?;
        Ok(rules)
    }

    /// The rules to play: the rule set, then the balance list (read now),
    /// then the command line bans; validated against the tables.
    pub fn effective(
        &self,
        weapons: &WeaponTable,
        gadgets: &GadgetTable,
    ) -> Result<RuleSet, ServerError> {
        let mut rules = self.base.clone();
        if let Some(path) = &self.balance_list {
            BalanceList::load(path, weapons, gadgets)?.apply(&mut rules);
        }
        for id in &self.bans {
            weapons.require(id)?;
            rules.ban_weapon(id);
        }
        rules.resolve_with(weapons, gadgets)?;
        Ok(rules)
    }
}

/// One log line describing rules: name, code and what is banned.
pub fn describe(rules: &RuleSet, weapons: &WeaponTable) -> String {
    let code = rules
        .to_code()
        .unwrap_or_else(|e| format!("(no code: {e})"));
    let banned: Vec<&str> = weapons
        .iter()
        .map(|(_, d)| d.id.as_str())
        .filter(|id| rules.availability(id).is_banned())
        .collect();
    let banned = if banned.is_empty() {
        "none".to_string()
    } else {
        banned.join(", ")
    };
    format!("{} {code} (banned weapons: {banned})", rules.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::server::{Room, RoomConfig};
    use ls_game::MatchConfig;
    use ls_sim::{Availability, PickupKind};
    use ls_worldgen::{ShareCode, Sliders};

    /// A unique temp file with `text`.
    pub(crate) fn temp_file(name: &str, text: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("ls-server-rules-{}-{name}", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn rule_sets_load_from_files_and_codes() {
        let mut custom = RuleSet::paint_the_void();
        custom.name = "Custom".into();
        custom.friendly_fire_percent = 0;
        let file = temp_file("custom.ron", &custom.to_ron());
        let from_file = load_rule_set(file.to_str().unwrap()).unwrap();
        assert_eq!(from_file, custom);

        let code = custom.to_code().unwrap();
        let from_code = load_rule_set(&code).unwrap();
        assert_eq!(from_code, rulecode::canonical(&custom));
        // Codes are lenient (case, dashes).
        let messy = code.to_ascii_lowercase().replace('-', "");
        assert_eq!(load_rule_set(&messy).unwrap(), from_code);

        // A typo in a code is a code error, not "file not found".
        let mut typo: Vec<char> = code.chars().collect();
        let i = typo.len() - 2;
        typo[i] = if typo[i] == 'A' { 'B' } else { 'A' };
        let typo: String = typo.into_iter().collect();
        let e = load_rule_set(&typo).unwrap_err().to_string();
        assert!(e.contains("rule set code"), "{e}");
        // A missing file names the file.
        let e = load_rule_set("no/such/rules.ron").unwrap_err().to_string();
        assert!(e.contains("no/such/rules.ron"), "{e}");
        // Built-in ids.
        assert_eq!(
            load_rule_set("sandbox_all_weapons").unwrap(),
            RuleSet::sandbox_all_weapons()
        );
        assert_eq!(
            load_rule_set("paint_the_void").unwrap(),
            RuleSet::paint_the_void()
        );
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn bans_and_balance_list_apply_on_top() {
        let weapons = WeaponTable::builtin();
        let gadgets = GadgetTable::builtin();
        let list = temp_file(
            "balance.ron",
            r#"BalanceList(bans: ["tectonic_charge"], tuning: { "railspike": (damage: 0.5) })"#,
        );
        let rules = ServerRules::new(None, vec!["bolt_rocket".into()], Some(list.clone())).unwrap();
        let eff = rules.effective(&weapons, &gadgets).unwrap();
        assert!(eff.availability("tectonic_charge").is_banned());
        assert!(eff.availability("bolt_rocket").is_banned());
        assert!(
            eff.starting_loadout.secondaries.is_empty(),
            "left the loadout"
        );
        assert_eq!(eff.tuning["railspike"].damage, 0.5);
        let line = describe(&eff, &weapons);
        assert!(line.contains("bolt_rocket, tectonic_charge"), "{line}");
        assert!(line.contains(" LR"), "{line}");

        // The list is re-read: an edit shows up in the next effective rules.
        std::fs::write(&list, r#"(bans: ["seeker"])"#).unwrap();
        let eff = rules.effective(&weapons, &gadgets).unwrap();
        assert!(eff.availability("seeker").is_banned());
        assert_eq!(
            eff.availability("tectonic_charge"),
            Availability::PickupOnly
        );
        // A broken list is an error (the caller keeps the previous rules).
        std::fs::write(&list, "(bans: [").unwrap();
        assert!(rules.effective(&weapons, &gadgets).is_err());
        std::fs::remove_file(&list).unwrap();
        assert!(rules.effective(&weapons, &gadgets).is_err(), "missing file");
    }

    #[test]
    fn invalid_inputs_fail_at_startup() {
        let e = ServerRules::new(None, vec!["laser_of_doom".into()], None)
            .unwrap_err()
            .to_string();
        assert!(e.contains("laser_of_doom"), "{e}");
        let list = temp_file("bad-balance.ron", r#"(bans: ["nope"])"#);
        let e = ServerRules::new(None, vec![], Some(list.clone()))
            .unwrap_err()
            .to_string();
        assert!(e.contains("nope"), "{e}");
        std::fs::remove_file(list).unwrap();
        // A code that decodes but does not validate (friendly fire 5 %).
        let mut r = RuleSet::paint_the_void();
        r.friendly_fire_percent = 5;
        let code = r.to_code().unwrap();
        assert!(ServerRules::new(Some(&code), vec![], None).is_err());
    }

    /// Banned weapons never spawn in the level and cannot be given.
    #[test]
    fn banned_weapons_never_spawn_in_a_room() {
        let rules =
            ServerRules::new(None, vec!["rivet_gun".into(), "scrap_laser".into()], None).unwrap();
        let mut mc = MatchConfig::paint_the_void(ShareCode::new(7, Sliders::new(40, 30)));
        mc.rules = rules.effective(&mc.weapons, &mc.gadgets).unwrap();
        let banned: Vec<_> = ["rivet_gun", "scrap_laser"]
            .iter()
            .map(|id| mc.weapons.find(id).unwrap())
            .collect();
        let mut room = Room::new(
            mc,
            RoomConfig {
                pregenerate: false,
                ..RoomConfig::default()
            },
        )
        .unwrap();
        let player = room
            .world_mut()
            .add_player(Some(0), ls_sim::ShipType::Tinker)
            .unwrap();
        for tick in 0..1200 {
            room.step();
            if tick % 60 == 0 {
                for p in room.world().pickups() {
                    if let Some((
                        _,
                        PickupKind::Weapon { weapon, .. } | PickupKind::Upgrade { weapon },
                    )) = p.content
                    {
                        assert!(!banned.contains(&weapon), "tick {tick}: {weapon:?}");
                    }
                }
            }
        }
        assert!(room.world().pickups().iter().any(|p| p.content.is_some()));
        let ship = room.world().ship(player).unwrap();
        assert!(ship.weapons.iter().all(|w| !banned.contains(&w.weapon)));
        assert!(room.world_mut().give_weapon(player, "rivet_gun").is_err());
    }
}
