//! Rule set code tests, plus the generated web catalog
//! (`web/src/rules/catalog.ts`) that lets the browser rule set editor encode
//! the same codes and validate like the server.

use super::*;
use crate::{GadgetTable, WeaponCost, WeaponSlot, WeaponTable};
use ls_math::Pcg32;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// A rule set code decodes to exactly the rule set it was made from, and
/// re-encodes to the same code.
fn round_trip(r: &RuleSet) -> String {
    let code = r.to_code().unwrap();
    assert!(code.starts_with(PREFIX));
    let back = RuleSet::from_code(&code).unwrap();
    assert_eq!(back, canonical(r));
    assert_eq!(back.to_code().unwrap(), code);
    // The full fallback decodes to the same rules.
    let full = r.to_full_code().unwrap();
    assert_eq!(RuleSet::from_code(&full).unwrap(), back);
    code
}

fn version_of(code: &str) -> u64 {
    let bits = parse_symbols(code).unwrap();
    Reader {
        bits: &bits,
        pos: 0,
    }
    .read(4)
    .unwrap()
}

fn edited() -> RuleSet {
    let mut r = RuleSet::paint_the_void();
    r.name = "Rockets only-2".into();
    r.weapons.insert("scrap_laser".into(), Availability::Banned);
    r.weapons
        .insert("railspike".into(), Availability::LoadoutOnly);
    r.starting_loadout.primaries = vec!["railspike".into(), "paint_thrower".into()];
    r.starting_loadout.gadget = Some("decoy".into());
    r.gadgets.insert("decoy".into(), Availability::LoadoutOnly);
    r.gadget_pickups.remove("decoy");
    r.pickups.remove("railspike");
    r.upgrade_pickups.clear();
    r.shield_overrides
        .insert("bolt_rocket".into(), ShieldInteraction::PassThrough(0.35));
    r.shield_overrides
        .insert("seeker".into(), ShieldInteraction::Wears(1.5));
    r.shield_overrides
        .insert("rivet_gun".into(), ShieldInteraction::Blocked);
    r.tuning.insert(
        "bolt_rocket".into(),
        WeaponTuning {
            damage: 0.85,
            fire_rate: 1.25,
            ammo: 2.0,
        },
    );
    r.friendly_fire_percent = 0;
    r.absurdity = 0;
    r.allowed_ship_types = vec![ShipType::Tug, ShipType::Skitter];
    r.scrap_pickup = None;
    let crate::ModeRules::PaintTheVoid(m) = &mut r.mode;
    m.match_secs = 600.0;
    m.respawn_secs = 1.5;
    r
}

fn banned(ids: &[&str]) -> RuleSet {
    let mut r = RuleSet::paint_the_void();
    for id in ids {
        r.ban_weapon(id);
    }
    r
}

#[test]
fn default_rules_are_a_short_diff_code() {
    let r = RuleSet::paint_the_void();
    let code = round_trip(&r);
    println!("{} chars: {code}", code.len());
    assert_eq!(version_of(&code), DIFF_VERSION as u64);
    assert!(code.len() <= 20, "{code}");
    // The full encoding is the long fallback.
    let full = r.to_full_code().unwrap();
    assert_eq!(version_of(&full), FULL_VERSION as u64);
    assert!(full.len() > 200 && full.len() < 320, "{}", full.len());
    // Lenient decoding: case, dashes, whitespace, O/I/L.
    let messy: String = code
        .to_ascii_lowercase()
        .replace('-', " ")
        .replacen("lr", "L R", 1);
    assert_eq!(RuleSet::from_code(&messy).unwrap(), canonical(&r));
    RuleSet::from_code(&code)
        .unwrap()
        .resolve(&WeaponTable::builtin())
        .unwrap();
}

#[test]
fn a_ban_costs_a_few_symbols() {
    let base = RuleSet::paint_the_void().to_code().unwrap();
    let one = round_trip(&banned(&["tectonic_charge"]));
    let two = round_trip(&banned(&["tectonic_charge", "railspike"]));
    println!("{base}\n{one}\n{two}");
    assert!(one.len() <= base.len() + 4, "{one}");
    assert!(two.len() <= one.len() + 2, "{two}");
    let back = RuleSet::from_code(&two).unwrap();
    assert!(back.availability("railspike").is_banned());
    assert!(!back.pickups.contains_key("railspike"));
    back.resolve(&WeaponTable::builtin()).unwrap();
}

#[test]
fn edited_rules_round_trip() {
    let r = edited();
    let code = round_trip(&r);
    assert_eq!(version_of(&code), DIFF_VERSION as u64);
    r.resolve(&WeaponTable::builtin()).unwrap();
}

type Edit = Box<dyn Fn(&mut RuleSet)>;

#[test]
fn every_single_setting_round_trips_as_a_diff() {
    let base = RuleSet::paint_the_void();
    let rule = PickupRule {
        weight: 3.25,
        respawn_secs: 12.0,
        ammo: 5,
        max_on_map: 2,
    };
    let edits: Vec<(&str, Edit)> = vec![
        ("name", Box::new(|r| r.name = "X".into())),
        ("empty name", Box::new(|r| r.name.clear())),
        (
            "mode",
            Box::new(|r| {
                let ModeRules::PaintTheVoid(m) = &mut r.mode;
                m.respawn_secs = 0.0;
            }),
        ),
        ("ff", Box::new(|r| r.friendly_fire_percent = 100)),
        ("absurdity", Box::new(|r| r.absurdity = 0)),
        (
            "ships",
            Box::new(|r| r.allowed_ship_types = vec![ShipType::Hauler]),
        ),
        (
            "mutators",
            Box::new(|r| r.mutators = vec![Mutator::Instagib]),
        ),
        (
            "availability",
            Box::new(|r| {
                r.weapons
                    .insert("seed_grenade".into(), Availability::LoadoutAndPickup);
            }),
        ),
        (
            "gadget availability",
            Box::new(|r| {
                r.gadgets.insert("cloak".into(), Availability::Banned);
            }),
        ),
        (
            "loadout",
            Box::new(|r| r.starting_loadout.secondaries = vec![("bolt_rocket".into(), 7)]),
        ),
        ("no gadget", Box::new(|r| r.starting_loadout.gadget = None)),
        (
            "pickup",
            Box::new(move |r| {
                r.pickups.insert("railspike".into(), rule);
            }),
        ),
        (
            "pickup removed",
            Box::new(|r| {
                r.pickups.remove("rivet_gun");
            }),
        ),
        (
            "upgrade",
            Box::new(move |r| {
                r.upgrade_pickups.insert("scrap_laser".into(), rule);
            }),
        ),
        (
            "gadget pickup",
            Box::new(move |r| {
                r.gadget_pickups.insert("decoy".into(), rule);
            }),
        ),
        ("scrap off", Box::new(|r| r.scrap_pickup = None)),
        (
            "scrap",
            Box::new(|r| {
                r.scrap_pickup = Some(ScrapPickupRule {
                    weight: 1.0,
                    respawn_secs: 5.0,
                    amount: 10.0,
                })
            }),
        ),
        (
            "shield",
            Box::new(|r| {
                r.shield_overrides
                    .insert("seed_grenade".into(), ShieldInteraction::Wears(0.0));
            }),
        ),
        (
            "tuning",
            Box::new(|r| {
                r.tuning.insert("railspike".into(), WeaponTuning::default());
            }),
        ),
    ];
    let base_len = base.to_code().unwrap().len();
    for (what, edit) in edits {
        let mut r = base.clone();
        edit(&mut r);
        assert_ne!(canonical(&r), canonical(&base), "{what}");
        let code = round_trip(&r);
        assert_eq!(version_of(&code), DIFF_VERSION as u64, "{what}");
        assert!(code.len() >= base_len, "{what}");
        assert!(code.len() < 40, "{what}: {code}");
    }
}

/// Almost everything changed: the diff grows past the full encoding.
fn fallback_rules() -> RuleSet {
    let mut r = RuleSet::paint_the_void();
    for (i, w) in weapon_ids().iter().enumerate() {
        r.tuning.insert(
            w.to_string(),
            WeaponTuning {
                damage: (10 + i) as f32 / 20.0,
                fire_rate: 1.5,
                ammo: 2.0,
            },
        );
        r.shield_overrides
            .insert(w.to_string(), ShieldInteraction::Blocked);
        if r.availability(w).as_pickup() {
            r.pickups.insert(
                w.to_string(),
                PickupRule {
                    weight: 0.5,
                    respawn_secs: 7.0,
                    ammo: 1,
                    max_on_map: 0,
                },
            );
        }
    }
    r
}

#[test]
fn full_encoding_is_the_fallback_when_it_is_shorter() {
    let r = fallback_rules();
    r.resolve(&WeaponTable::builtin()).unwrap();
    let code = r.to_code().unwrap();
    assert_eq!(version_of(&code), FULL_VERSION as u64);
    assert_eq!(code, r.to_full_code().unwrap());
    round_trip(&r);
}

#[test]
fn codes_from_older_builds_decode_and_newer_ones_are_rejected() {
    let weapons = weapon_ids();
    let gadgets = gadget_ids();
    // An older build did not have the last weapon yet (the registry is
    // append-only): its codes say nothing about it.
    let last = *weapons.last().unwrap();
    for r in [RuleSet::paint_the_void(), edited()] {
        let mut old = r.clone();
        old.weapons.remove(last);
        old.pickups.remove(last);
        old.upgrade_pickups.remove(last);
        old.shield_overrides.remove(last);
        old.tuning.remove(last);
        old.starting_loadout.primaries.retain(|w| w != last);
        old.starting_loadout.secondaries.retain(|(w, _)| w != last);
        let code = old
            .to_code_with(&weapons[..weapons.len() - 1], &gadgets)
            .unwrap();
        let back = RuleSet::from_code(&code).unwrap();
        assert_eq!(back.availability(last), Availability::Banned);
        assert_eq!(back, canonical(&old));
        // The full form as well.
        let full = finish(full_payload(&old, &weapons[..weapons.len() - 1], &gadgets).unwrap());
        assert_eq!(RuleSet::from_code(&full).unwrap(), back);
    }
    // A newer build knows one weapon more: a clear error, not a checksum
    // mismatch.
    let mut newer = weapons.clone();
    newer.push("future_gun");
    let r = RuleSet::paint_the_void();
    let code = r.to_code_with(&newer, &gadgets).unwrap();
    let e = RuleSet::from_code(&code).unwrap_err().to_string();
    assert!(e.contains("newer build"), "{e}");
    let full = finish(full_payload(&r, &newer, &gadgets).unwrap());
    let e = RuleSet::from_code(&full).unwrap_err().to_string();
    assert!(e.contains("newer build"), "{e}");
}

/// Builds a diff code against a different base (another build's default
/// rules) by patching the fingerprint and re-checksumming.
#[test]
fn diff_codes_against_other_default_rules_are_rejected() {
    let code = banned(&["railspike"]).to_code().unwrap();
    let mut bits = parse_symbols(&code).unwrap();
    // Flip one fingerprint bit (after version 4, counts 10, mode 2).
    bits[16] = !bits[16];
    // Recompute the checksum over the patched payload.
    let payload_len = {
        let mut r = Reader {
            bits: &bits,
            pos: 4,
        };
        let (nw, ng) = (r.read(6).unwrap() as usize, r.read(4).unwrap() as usize);
        let w = weapon_ids();
        let g = gadget_ids();
        read_diff(&mut r, &w[..nw], &g[..ng]).unwrap().unwrap_err();
        r.pos
    };
    let mut b = Bits {
        bits: bits[..payload_len].to_vec(),
    };
    b.bits.truncate(payload_len);
    let forged = finish(b);
    let e = RuleSet::from_code(&forged).unwrap_err().to_string();
    assert!(e.contains("different default rules"), "{e}");
}

#[test]
fn unrepresentable_values_are_rejected() {
    let base = RuleSet::paint_the_void();
    let mut r = base.clone();
    r.pickups.get_mut("rivet_gun").unwrap().weight = 0.3;
    assert!(r.to_code().is_err());
    assert!(r.to_full_code().is_err());
    let mut r = base.clone();
    r.pickups.get_mut("rivet_gun").unwrap().respawn_secs = 2.5;
    assert!(r.to_code().is_err());
    let mut r = base.clone();
    r.name = "Ümlaut".into();
    assert!(r.to_code().is_err());
    let mut r = base.clone();
    r.name = "x".repeat(MAX_NAME + 1);
    assert!(r.to_code().is_err());
    let mut r = base.clone();
    r.weapons
        .insert("homebrew".into(), Availability::PickupOnly);
    assert!(r.to_code().is_err());
    let mut r = base.clone();
    r.weapons.insert("homebrew".into(), Availability::Banned);
    assert!(r.to_code().is_err(), "unknown ids fail even when banned");
    let mut r = base;
    r.tuning.insert(
        "rivet_gun".into(),
        WeaponTuning {
            damage: 0.333,
            ..WeaponTuning::default()
        },
    );
    assert!(r.to_code().is_err());
}

/// Every single-symbol change of `code` is rejected.
fn assert_typos_rejected(code: &str, seed: u64) -> usize {
    let body: Vec<char> = code.chars().collect();
    let mut rng = Pcg32::new(seed, 5);
    let mut tested = 0;
    for i in 2..body.len() {
        if body[i] == '-' {
            continue;
        }
        for _ in 0..3 {
            let mut c = body.clone();
            let orig = c[i];
            let mut repl = orig;
            while repl == orig {
                repl = ALPHABET[rng.below(32) as usize] as char;
            }
            c[i] = repl;
            let s: String = c.into_iter().collect();
            if let Ok(r) = RuleSet::from_code(&s) {
                panic!("typo at {i} in {code} accepted: {s} -> {}", r.name);
            }
            tested += 1;
        }
    }
    tested
}

#[test]
fn typos_and_garbage_are_rejected() {
    let code = RuleSet::paint_the_void().to_code().unwrap();
    assert!(RuleSet::from_code("LS12-3456").is_err());
    assert!(RuleSet::from_code("").is_err());
    assert!(RuleSet::from_code("LR").is_err());
    assert!(RuleSet::from_code(&code[..code.len() - 3]).is_err());
    assert!(RuleSet::from_code(&format!("{code}U")).is_err());
    assert!(RuleSet::from_code(&format!("{code}0")).is_err());
    assert!(RuleSet::from_code(&code.replace('-', "*")).is_err());
    let mut tested = assert_typos_rejected(&code, 1);
    tested += assert_typos_rejected(&edited().to_code().unwrap(), 2);
    tested += assert_typos_rejected(&banned(&["railspike"]).to_code().unwrap(), 3);
    tested += assert_typos_rejected(&RuleSet::paint_the_void().to_full_code().unwrap(), 4);
    assert!(tested > 700, "{tested}");
    // Swapping two neighbouring symbols is caught too.
    let c: Vec<char> = edited().to_code().unwrap().chars().collect();
    for i in 2..c.len() - 1 {
        if c[i] == '-' || c[i + 1] == '-' || c[i] == c[i + 1] {
            continue;
        }
        let mut s = c.clone();
        s.swap(i, i + 1);
        let s: String = s.into_iter().collect();
        assert!(RuleSet::from_code(&s).is_err(), "swap at {i}: {s}");
    }
}

#[test]
fn random_garbage_never_panics() {
    let mut rng = Pcg32::new(9, 9);
    for n in 0..2000 {
        let len = 1 + rng.below(60) as usize;
        let mut s = String::from("LR");
        for _ in 0..len {
            s.push(ALPHABET[rng.below(32) as usize] as char);
        }
        // Version 2 or 3 in the first symbol half the time.
        if n % 2 == 0 {
            s.replace_range(2..3, if n % 4 == 0 { "4" } else { "6" });
        }
        let _ = RuleSet::from_code(&s);
    }
}

#[test]
fn looks_like_code_tells_codes_from_paths() {
    let code = RuleSet::paint_the_void().to_code().unwrap();
    assert!(looks_like_code(&code));
    assert!(looks_like_code(&code.to_ascii_lowercase()));
    assert!(!looks_like_code("data/rulesets/paint_the_void.ron"));
    assert!(!looks_like_code("rules.ron"));
    assert!(!looks_like_code("LR"));
    assert!(!looks_like_code("Ü"));
}

// ---------------------------------------------------------------------------
// Web catalog: `web/src/rules/catalog.ts`

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_f32(v: f32) -> String {
    assert!(v.is_finite());
    format!("{v}")
}

fn json_obj(entries: impl IntoIterator<Item = (String, String)>) -> String {
    let body: Vec<String> = entries
        .into_iter()
        .map(|(k, v)| format!("{}: {v}", json_str(&k)))
        .collect();
    format!("{{{}}}", body.join(", "))
}

fn json_rule(r: &PickupRule) -> String {
    format!(
        "{{\"weight\": {}, \"respawn_secs\": {}, \"ammo\": {}, \"max_on_map\": {}}}",
        json_f32(r.weight),
        json_f32(r.respawn_secs),
        r.ammo,
        r.max_on_map
    )
}

fn json_shield(s: &ShieldInteraction) -> String {
    match *s {
        ShieldInteraction::Blocked => "{\"kind\": \"Blocked\", \"value\": 0}".into(),
        ShieldInteraction::Wears(f) => {
            format!("{{\"kind\": \"Wears\", \"value\": {}}}", json_f32(f))
        }
        ShieldInteraction::PassThrough(f) => {
            format!("{{\"kind\": \"PassThrough\", \"value\": {}}}", json_f32(f))
        }
    }
}

fn json_rules(r: &RuleSet) -> String {
    let m = r.paint_the_void_rules();
    let avail = |map: &BTreeMap<String, Availability>| {
        json_obj(
            map.iter()
                .map(|(k, v)| (k.clone(), json_str(&format!("{v:?}")))),
        )
    };
    let rules = |map: &BTreeMap<String, PickupRule>| {
        json_obj(map.iter().map(|(k, v)| (k.clone(), json_rule(v))))
    };
    let lo = &r.starting_loadout;
    let fields = [
        ("name", json_str(&r.name)),
        (
            "mode",
            format!(
                "{{\"kind\": \"PaintTheVoid\", \"teams\": {}, \"match_secs\": {}, \"respawn_secs\": {}}}",
                m.teams,
                json_f32(m.match_secs),
                json_f32(m.respawn_secs)
            ),
        ),
        ("weapons", avail(&r.weapons)),
        ("gadgets", avail(&r.gadgets)),
        (
            "starting_loadout",
            format!(
                "{{\"primaries\": [{}], \"secondaries\": [{}], \"gadget\": {}}}",
                lo.primaries
                    .iter()
                    .map(|s| json_str(s))
                    .collect::<Vec<_>>()
                    .join(", "),
                lo.secondaries
                    .iter()
                    .map(|(s, n)| format!("[{}, {n}]", json_str(s)))
                    .collect::<Vec<_>>()
                    .join(", "),
                lo.gadget.as_deref().map_or("null".into(), json_str)
            ),
        ),
        ("pickups", rules(&r.pickups)),
        ("upgrade_pickups", rules(&r.upgrade_pickups)),
        ("gadget_pickups", rules(&r.gadget_pickups)),
        (
            "scrap_pickup",
            r.scrap_pickup.map_or("null".into(), |s| {
                format!(
                    "{{\"weight\": {}, \"respawn_secs\": {}, \"amount\": {}}}",
                    json_f32(s.weight),
                    json_f32(s.respawn_secs),
                    json_f32(s.amount)
                )
            }),
        ),
        (
            "shield_overrides",
            json_obj(
                r.shield_overrides
                    .iter()
                    .map(|(k, v)| (k.clone(), json_shield(v))),
            ),
        ),
        (
            "tuning",
            json_obj(r.tuning.iter().map(|(k, t)| {
                (
                    k.clone(),
                    format!(
                        "{{\"damage\": {}, \"fire_rate\": {}, \"ammo\": {}}}",
                        json_f32(t.damage),
                        json_f32(t.fire_rate),
                        json_f32(t.ammo)
                    ),
                )
            })),
        ),
        ("friendly_fire_percent", r.friendly_fire_percent.to_string()),
        (
            "allowed_ship_types",
            format!(
                "[{}]",
                r.allowed_ship_types
                    .iter()
                    .map(|t| json_str(&format!("{t:?}")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        ("absurdity", r.absurdity.to_string()),
        (
            "mutators",
            format!(
                "[{}]",
                r.mutators
                    .iter()
                    .map(|m| json_str(&format!("{m:?}")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
    ];
    json_obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)))
}

/// Rule sets whose code and validity the web editor must reproduce.
fn vectors() -> Vec<(&'static str, RuleSet)> {
    let base = RuleSet::paint_the_void();
    let with = |f: &dyn Fn(&mut RuleSet)| {
        let mut r = base.clone();
        f(&mut r);
        r
    };
    let rule = PickupRule {
        weight: 0.25,
        respawn_secs: 1000.0,
        ammo: 63,
        max_on_map: 15,
    };
    vec![
        ("default", base.clone()),
        ("edited", edited()),
        (
            "ban railspike and tectonic",
            banned(&["railspike", "tectonic_charge"]),
        ),
        (
            "every weapon tuned (full fallback)",
            with(&|r| {
                for w in weapon_ids() {
                    r.tuning.insert(
                        w.into(),
                        WeaponTuning {
                            damage: 1.5,
                            fire_rate: 0.5,
                            ammo: 3.0,
                        },
                    );
                    r.shield_overrides
                        .insert(w.into(), ShieldInteraction::Wears(0.25));
                }
            }),
        ),
        ("full fallback", fallback_rules()),
        (
            "edge values",
            with(&|r| {
                r.pickups.insert("railspike".into(), rule);
                r.gadget_pickups.insert("cloak".into(), rule);
                r.name = "a-Z 09".into();
                r.friendly_fire_percent = 10;
                r.absurdity = 100;
                let ModeRules::PaintTheVoid(m) = &mut r.mode;
                m.match_secs = 65535.0;
                m.respawn_secs = 25.5;
            }),
        ),
        (
            "invalid: friendly fire 5",
            with(&|r| r.friendly_fire_percent = 5),
        ),
        (
            "invalid: friendly fire 101",
            with(&|r| r.friendly_fire_percent = 101),
        ),
        ("invalid: absurdity 101", with(&|r| r.absurdity = 101)),
        (
            "invalid: no ship types",
            with(&|r| r.allowed_ship_types.clear()),
        ),
        (
            "invalid: instagib",
            with(&|r| r.mutators = vec![Mutator::Instagib]),
        ),
        (
            "invalid: 3 teams",
            with(&|r| {
                let ModeRules::PaintTheVoid(m) = &mut r.mode;
                m.teams = 3;
            }),
        ),
        (
            "invalid: match 0 s",
            with(&|r| {
                let ModeRules::PaintTheVoid(m) = &mut r.mode;
                m.match_secs = 0.0;
            }),
        ),
        (
            "invalid: banned weapon still a pickup",
            with(&|r| {
                r.weapons.insert("rivet_gun".into(), Availability::Banned);
            }),
        ),
        (
            "invalid: banned loadout weapon",
            with(&|r| {
                r.weapons.insert("bolt_rocket".into(), Availability::Banned);
                r.pickups.remove("bolt_rocket");
            }),
        ),
        (
            "invalid: pickup-only weapon in the loadout",
            with(&|r| r.starting_loadout.primaries.push("rivet_gun".into())),
        ),
        (
            "invalid: secondary in the primaries",
            with(&|r| {
                r.weapons
                    .insert("seeker".into(), Availability::LoadoutAndPickup);
                r.starting_loadout.primaries.push("seeker".into());
            }),
        ),
        (
            "invalid: secondary listed twice",
            with(&|r| {
                r.starting_loadout
                    .secondaries
                    .push(("bolt_rocket".into(), 1));
            }),
        ),
        (
            "invalid: loadout-only weapon as a pickup",
            with(&|r| {
                r.weapons
                    .insert("rivet_gun".into(), Availability::LoadoutOnly);
            }),
        ),
        (
            "invalid: upgrades of a weapon without levels",
            with(&|r| {
                r.upgrade_pickups.insert("rivet_gun".into(), rule);
            }),
        ),
        (
            "invalid: banned loadout gadget",
            with(&|r| {
                r.gadgets.insert("shield".into(), Availability::Banned);
                r.gadget_pickups.remove("shield");
            }),
        ),
        (
            "invalid: pickup-only loadout gadget",
            with(&|r| {
                r.gadgets.insert("shield".into(), Availability::PickupOnly);
            }),
        ),
        (
            "invalid: banned gadget as a pickup",
            with(&|r| {
                r.gadgets.insert("cloak".into(), Availability::Banned);
            }),
        ),
        (
            "invalid: fire rate 0",
            with(&|r| {
                r.tuning.insert(
                    "rivet_gun".into(),
                    WeaponTuning {
                        fire_rate: 0.0,
                        ..WeaponTuning::default()
                    },
                );
            }),
        ),
        (
            "invalid: pass-through over 1",
            with(&|r| {
                r.shield_overrides
                    .insert("rivet_gun".into(), ShieldInteraction::PassThrough(1.5));
            }),
        ),
        (
            "unencodable: weight 0.3",
            with(&|r| r.pickups.get_mut("rivet_gun").unwrap().weight = 0.3),
        ),
        (
            "unencodable: ammo 64",
            with(&|r| r.pickups.get_mut("bolt_rocket").unwrap().ammo = 64),
        ),
        (
            "unencodable: tuning 0.33",
            with(&|r| {
                r.tuning.insert(
                    "rivet_gun".into(),
                    WeaponTuning {
                        damage: 0.33,
                        ..WeaponTuning::default()
                    },
                );
            }),
        ),
        ("unencodable: long name", with(&|r| r.name = "n".repeat(32))),
    ]
}

fn catalog_ts() -> String {
    let weapons = WeaponTable::builtin();
    let gadgets = GadgetTable::builtin();
    let mut out = String::new();
    out.push_str(
        "// GENERATED by crates/ls-sim/src/rulecode/tests.rs from data/ (weapons, gadgets,\n\
         // rule sets) and the rule set code format: do not edit. After a data or\n\
         // registry change: LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode\n\n\
         import type { Catalog } from \"./types.ts\";\n\n\
         export const CATALOG: Catalog = {\n",
    );
    out.push_str("  \"weapons\": [\n");
    for id in weapon_ids() {
        let d = weapons.get(weapons.find(id).unwrap());
        out.push_str(&format!(
            "    {{\"id\": {}, \"name\": {}, \"family\": {}, \"slot\": {}, \"count\": {}, \"maxLevel\": {}}},\n",
            json_str(id),
            json_str(&d.name),
            json_str(&format!("{:?}", d.family)),
            json_str(match d.slot {
                WeaponSlot::Primary => "primary",
                WeaponSlot::Secondary => "secondary",
            }),
            matches!(d.cost, WeaponCost::Count(_)),
            d.max_level
        ));
    }
    out.push_str("  ],\n  \"gadgets\": [\n");
    for id in gadget_ids() {
        let d = gadgets.get(gadgets.find(id).unwrap());
        out.push_str(&format!(
            "    {{\"id\": {}, \"name\": {}}},\n",
            json_str(id),
            json_str(&d.name)
        ));
    }
    out.push_str("  ],\n  \"shipTypes\": [");
    out.push_str(
        &ShipType::ALL
            .iter()
            .map(|t| json_str(&format!("{t:?}")))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str("],\n");
    out.push_str(&format!(
        "  \"defaults\": {{\"PaintTheVoid\": {}}},\n",
        json_rules(&canonical(&RuleSet::paint_the_void()))
    ));
    out.push_str("  \"vectors\": [\n");
    for (label, r) in vectors() {
        let valid = r.resolve(&weapons).is_ok();
        let code = r.to_code().ok();
        let full = r.to_full_code().ok();
        out.push_str(&format!(
            "    {{\"label\": {}, \"valid\": {valid}, \"code\": {}, \"full\": {}, \"rules\": {}}},\n",
            json_str(label),
            code.as_deref().map_or("null".into(), json_str),
            full.as_deref().map_or("null".into(), json_str),
            json_rules(&canonical(&r))
        ));
    }
    out.push_str("  ],\n};\n");
    out
}

#[test]
fn vectors_behave_as_labelled() {
    let weapons = WeaponTable::builtin();
    for (label, r) in vectors() {
        let valid = r.resolve(&weapons).is_ok();
        let encodable = r.to_code().is_ok();
        if label.starts_with("invalid") {
            assert!(!valid, "{label}");
        } else if label.starts_with("unencodable") {
            assert!(!encodable, "{label}");
        } else {
            assert!(valid && encodable, "{label}");
        }
        if encodable {
            round_trip(&r);
        }
    }
}

/// The web rule set editor encodes codes in TypeScript from this catalog;
/// its Node tests check the vectors. Bless with `LS_BLESS_RULES_CATALOG=1`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn web_catalog_is_current() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/rules/catalog.ts");
    let want = catalog_ts();
    if std::env::var_os("LS_BLESS_RULES_CATALOG").is_some() {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace('\r', "");
    assert!(
        have == want,
        "web/src/rules/catalog.ts is stale: run LS_BLESS_RULES_CATALOG=1 cargo test -p ls-sim --lib rulecode"
    );
}
