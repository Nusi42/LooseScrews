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

/// Format 5 and the full (`true`) or diff (`false`) bit.
fn is_full(code: &str) -> bool {
    assert_eq!(version_of(code), CODE_VERSION as u64, "{code}");
    parse_symbols(code).unwrap()[HEADER_BITS]
}

/// Bits before the full/diff bit in format 5: version 4, counts 6 + 4 + 5.
const HEADER_BITS: usize = 19;

/// The registry ids of this build.
fn all_ids() -> (Vec<&'static str>, Vec<&'static str>, Vec<&'static str>) {
    (weapon_ids(), gadget_ids(), event_ids())
}

fn ids<'a>(w: &'a [&'a str], g: &'a [&'a str], e: &'a [&'a str]) -> Ids<'a> {
    Ids {
        weapons: w,
        gadgets: g,
        events: e,
    }
}

/// Weapons in the registry of M3 builds.
const M3_WEAPONS: usize = 27;

/// What a rule set looked like in an M3 build (format 2-4 codes): the
/// Absurdity of the M3 defaults where it is today's default, and decoded
/// with today's pigment default and no event bans.
fn m3(r: &RuleSet) -> RuleSet {
    let mut r = r.clone();
    // Weapons appended in M4 (the Paperweight) were not in M3 codes: they
    // decode banned.
    for id in &weapon_ids()[M3_WEAPONS..] {
        r.weapons.remove(*id);
        r.pickups.remove(*id);
        r.upgrade_pickups.remove(*id);
        r.shield_overrides.remove(*id);
        r.tuning.remove(*id);
    }
    if r.absurdity == RuleSet::default_for(r.mode.kind()).absurdity {
        r.absurdity = LEGACY_ABSURDITY;
    }
    canonical(&with_m4_fields(r))
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
    let crate::ModeRules::PaintTheVoid(m) = &mut r.mode else {
        unreachable!()
    };
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
    assert!(!is_full(&code));
    assert!(code.len() <= 21, "{code}");
    // The full encoding is the long fallback.
    let full = r.to_full_code().unwrap();
    assert!(is_full(&full));
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
    assert!(!is_full(&code));
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
                let ModeRules::PaintTheVoid(m) = &mut r.mode else {
                    unreachable!()
                };
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
        assert!(!is_full(&code), "{what}");
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
    assert!(is_full(&code));
    assert_eq!(code, r.to_full_code().unwrap());
    round_trip(&r);
}

#[test]
fn codes_from_older_builds_decode_and_newer_ones_are_rejected() {
    let (weapons, gadgets, events) = all_ids();
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
        let older = ids(&weapons[..weapons.len() - 1], &gadgets, &events);
        let code = old.to_code_with(older).unwrap();
        let back = RuleSet::from_code(&code).unwrap();
        assert_eq!(back.availability(last), Availability::Banned);
        assert_eq!(back, canonical(&old));
        // The full form as well.
        let full = finish(full_payload(&old, older, CODE_VERSION).unwrap());
        assert_eq!(RuleSet::from_code(&full).unwrap(), back);
    }
    // A newer build knows one weapon more: a clear error, not a checksum
    // mismatch.
    let mut newer = weapons.clone();
    newer.push("future_gun");
    let r = RuleSet::paint_the_void();
    let code = r.to_code_with(ids(&newer, &gadgets, &events)).unwrap();
    let e = RuleSet::from_code(&code).unwrap_err().to_string();
    assert!(e.contains("newer build"), "{e}");
    let full = finish(full_payload(&r, ids(&newer, &gadgets, &events), CODE_VERSION).unwrap());
    let e = RuleSet::from_code(&full).unwrap_err().to_string();
    assert!(e.contains("newer build"), "{e}");

    // An older build without the last comedy event: it stays on.
    let last_event = *events.last().unwrap();
    let mut r = RuleSet::paint_the_void();
    r.ban_event(events[0], true);
    let older = ids(&weapons, &gadgets, &events[..events.len() - 1]);
    for code in [
        r.to_code_with(older).unwrap(),
        finish(full_payload(&r, older, CODE_VERSION).unwrap()),
    ] {
        let back = RuleSet::from_code(&code).unwrap();
        assert!(back.event_banned(events[0]));
        assert!(!back.event_banned(last_event));
        assert_eq!(back, canonical(&r));
    }
    // A newer build with one event more: rejected.
    let mut newer = events.clone();
    newer.push("future_event");
    let code = r.to_code_with(ids(&weapons, &gadgets, &newer)).unwrap();
    let e = RuleSet::from_code(&code).unwrap_err().to_string();
    assert!(e.contains("newer build"), "{e}");
}

/// Builds a diff code against a different base (another build's default
/// rules) by patching the fingerprint and re-checksumming.
#[test]
fn diff_codes_against_other_default_rules_are_rejected() {
    let code = banned(&["railspike"]).to_code().unwrap();
    let mut bits = parse_symbols(&code).unwrap();
    // Flip one fingerprint bit (after version 4, counts 15, full/diff 1,
    // mode 4).
    bits[HEADER_BITS + 5] = !bits[HEADER_BITS + 5];
    // Recompute the checksum over the patched payload.
    let payload_len = {
        let mut r = Reader {
            bits: &bits,
            pos: 4,
        };
        let (nw, ng) = (r.read(6).unwrap() as usize, r.read(4).unwrap() as usize);
        let ne = r.read(EVENT_BITS).unwrap() as usize;
        assert!(!r.flag().unwrap());
        let kind = ModeKind::from_index(r.read(MODE_BITS).unwrap() as u8).unwrap();
        let (w, g, e) = all_ids();
        read_diff_v45(
            &mut r,
            kind,
            ids(&w[..nw], &g[..ng], &e[..ne]),
            CODE_VERSION,
        )
        .unwrap()
        .unwrap_err();
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

/// A mode's settings as `{"kind": ..., <field>: <value>, ...}`.
fn json_mode(m: &ModeRules) -> String {
    let mut out = format!("{{\"kind\": {}", json_str(&format!("{:?}", m.kind())));
    for f in m.fields() {
        out.push_str(&format!(", {}: {}", json_str(f.name), json_f32(f.value)));
    }
    out.push('}');
    out
}

fn json_rules(r: &RuleSet) -> String {
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
        ("mode", json_mode(&r.mode)),
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
        (
            "events",
            json_obj(
                r.events
                    .iter()
                    .map(|(k, v)| (k.clone(), json_str(&format!("{v:?}")))),
            ),
        ),
        (
            "pigment_pickup",
            r.pigment_pickup.map_or("null".into(), |p| {
                format!(
                    "{{\"respawn_secs\": {}, \"players_per_pickup\": {}, \"min_on_map\": {}}}",
                    p.respawn_secs, p.players_per_pickup, p.min_on_map
                )
            }),
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
                let ModeRules::PaintTheVoid(m) = &mut r.mode else {
                    unreachable!()
                };
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
        ("instagib", with(&|r| r.mutators = vec![Mutator::Instagib])),
        ("4 teams", with(&|r| r.mode.set_field("teams", 4.0))),
        (
            "invalid: 5 teams",
            with(&|r| r.mode.set_field("teams", 5.0)),
        ),
        ("invalid: 1 team", with(&|r| r.mode.set_field("teams", 1.0))),
        (
            "invalid: match 0 s",
            with(&|r| {
                let ModeRules::PaintTheVoid(m) = &mut r.mode else {
                    unreachable!()
                };
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
        (
            "ban duck parade and cat paw",
            with(&|r| {
                r.ban_event("duck_parade", true);
                r.ban_event("cat_paw", true);
            }),
        ),
        (
            "every event banned, absurdity 0",
            with(&|r| {
                for id in event_ids() {
                    r.ban_event(id, true);
                }
                r.absurdity = 0;
            }),
        ),
        (
            "pigment edited",
            with(&|r| {
                r.pigment_pickup = Some(crate::PigmentPickupRule {
                    respawn_secs: 1023,
                    players_per_pickup: 15,
                    min_on_map: 0,
                })
            }),
        ),
        ("no pigment", with(&|r| r.pigment_pickup = None)),
        (
            "invalid: pigment per 0 players",
            with(&|r| {
                r.pigment_pickup = Some(crate::PigmentPickupRule {
                    respawn_secs: 30,
                    players_per_pickup: 0,
                    min_on_map: 2,
                })
            }),
        ),
        (
            "unencodable: pigment respawn 1024",
            with(&|r| {
                r.pigment_pickup = Some(crate::PigmentPickupRule {
                    respawn_secs: 1024,
                    players_per_pickup: 4,
                    min_on_map: 2,
                })
            }),
        ),
        (
            "unencodable: unknown event",
            with(&|r| {
                r.events
                    .insert("tea_party".into(), crate::EventRule::Banned);
            }),
        ),
        (
            "unencodable: match 0.5 s",
            with(&|r| r.mode.set_field("match_secs", 0.5)),
        ),
    ]
    .into_iter()
    .chain(mode_vectors())
    .collect()
}

/// Every mode's default, edited settings, mutators and invalid team counts.
fn mode_vectors() -> Vec<(&'static str, RuleSet)> {
    let mode = |kind: ModeKind, f: &dyn Fn(&mut RuleSet)| {
        let mut r = RuleSet::default_for(kind);
        f(&mut r);
        r
    };
    vec![
        ("mode: Team Brawl", mode(ModeKind::TeamBrawl, &|_| {})),
        ("mode: The Squeeze", mode(ModeKind::TheSqueeze, &|_| {})),
        ("mode: Heist", mode(ModeKind::Heist, &|_| {})),
        ("mode: Hot Reactor", mode(ModeKind::HotReactor, &|_| {})),
        (
            "mode: Last Color Standing",
            mode(ModeKind::LastColorStanding, &|_| {}),
        ),
        ("mode: The Architect", mode(ModeKind::TheArchitect, &|_| {})),
        (
            "Team Brawl duel with instagib",
            mode(ModeKind::TeamBrawl, &|r| {
                r.name = "Duel".into();
                r.mode.set_field("teams", 0.0);
                r.mode.set_field("kill_target", 10.0);
                r.mode.set_field("match_secs", 300.0);
                r.mutators = vec![Mutator::Instagib];
            }),
        ),
        (
            "Team Brawl 3 teams, banned railspike",
            mode(ModeKind::TeamBrawl, &|r| {
                r.mode.set_field("teams", 3.0);
                r.ban_weapon("railspike");
            }),
        ),
        (
            "Squeeze edited",
            mode(ModeKind::TheSqueeze, &|r| {
                r.mode.set_field("phases", 3.0);
                r.mode.set_field("crush_per_sec", 40.0);
                r.mode.set_field("scrap_per_100u2", 1.25);
            }),
        ),
        (
            "Hot Reactor edited",
            mode(ModeKind::HotReactor, &|r| {
                r.mode.set_field("lives", 5.0);
                r.mode.set_field("fuse_speed_per_ship", 0.75);
                r.mode.set_field("pass_immunity_secs", 1.5);
            }),
        ),
        (
            "Heist free-for-all",
            mode(ModeKind::Heist, &|r| r.mode.set_field("teams", 0.0)),
        ),
        (
            "Last Color Standing edited",
            mode(ModeKind::LastColorStanding, &|r| {
                r.mode.set_field("drain_per_kill", 50.0);
                r.mode.set_field("stalemate_secs", 300.0);
            }),
        ),
        (
            "invalid: Squeeze with 2 teams",
            mode(ModeKind::TheSqueeze, &|r| r.mode.set_field("teams", 2.0)),
        ),
        (
            "invalid: Last Color Standing with 4 teams",
            mode(ModeKind::LastColorStanding, &|r| {
                r.mode.set_field("teams", 4.0)
            }),
        ),
        (
            "invalid: Heist with 3 teams",
            mode(ModeKind::Heist, &|r| r.mode.set_field("teams", 3.0)),
        ),
        (
            "unencodable: Hot Reactor fuse speed 0.33",
            mode(ModeKind::HotReactor, &|r| {
                r.mode.set_field("fuse_speed_per_ship", 0.33)
            }),
        ),
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
    out.push_str("  ],\n  \"events\": [\n");
    for (kind, d) in crate::events::EventTable::builtin().iter() {
        out.push_str(&format!(
            "    {{\"id\": {}, \"name\": {}}},\n",
            json_str(kind.id()),
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
    out.push_str("  \"modes\": [\n");
    for kind in ModeKind::ALL {
        let fields: Vec<String> = ModeRules::default_for(kind)
            .fields()
            .iter()
            .map(|f| {
                format!(
                    "{{\"name\": {}, \"steps\": {}, \"bits\": {}}}",
                    json_str(f.name),
                    json_f32(f.steps),
                    f.bits
                )
            })
            .collect();
        let module = crate::modes::module(kind);
        out.push_str(&format!(
            "    {{\"kind\": {}, \"index\": {}, \"name\": {}, \"teams\": [{}], \"lives\": {}, \"fields\": [{}]}},\n",
            json_str(&format!("{kind:?}")),
            kind.index(),
            json_str(kind.name()),
            module
                .team_counts()
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            ModeRules::default_for(kind).lives(),
            fields.join(", ")
        ));
    }
    out.push_str("  ],\n");
    out.push_str("  \"mutators\": [");
    out.push_str(
        &Mutator::ALL
            .iter()
            .map(|m| json_str(&format!("{m:?}")))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str("],\n");
    out.push_str("  \"defaults\": {\n");
    for kind in ModeKind::ALL {
        out.push_str(&format!(
            "    {}: {},\n",
            json_str(&format!("{kind:?}")),
            json_rules(&canonical(&RuleSet::default_for(kind)))
        ));
    }
    out.push_str("  },\n");
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
    out.push_str("  ],\n");
    out.push_str("  \"legacyVectors\": [\n");
    for (code, r) in legacy_vectors() {
        let back = RuleSet::from_code(code).unwrap();
        out.push_str(&format!(
            "    {{\"code\": {}, \"rules\": {}}},\n",
            json_str(code),
            json_rules(&back)
        ));
        assert_eq!(back, r, "{code}");
    }
    out.push_str("  ],\n};\n");
    out
}

/// Codes of older formats (2-4) the web decoder must read like the server,
/// with what they decode to.
fn legacy_vectors() -> Vec<(&'static str, RuleSet)> {
    let m3_builtin = |kind: ModeKind| {
        let mut r = RuleSet::default_for(kind);
        r.absurdity = LEGACY_ABSURDITY;
        m3(&r)
    };
    vec![
        ("LR6V-69CM-G006-58R", m3(&RuleSet::paint_the_void())),
        (
            "LR4V-6WGD-RX2W-0Q8K-W0PM-RVR8-0JR7-K4AF-05AN-A5AN-APAM-A40B-1506-A107-H0G3-RG61-W840-Y420-F20R-7H10-2GG8-1W86-0S44-0A8D-0G3T-2840-YGT0-GA42-G61X-1410-PR50-C3T2-840Y-GT0R-7M4G-25N0-RR21-D410-PR8G-42V0-M10P-R908-5MG4-2V14-30F0-0000-1085-MG42-T821-D4R0-7HW0-0000-0000-02B5-4",
            m3(&RuleSet::paint_the_void()),
        ),
        ("LR6V-69CM-G214-D7G1-P6B", m3(&banned(&["railspike", "tectonic_charge"]))),
        ("LR8V-6097-G000-TMBG", m3_builtin(ModeKind::PaintTheVoid)),
        ("LR8V-63T6-X000-NNJ0", m3_builtin(ModeKind::TeamBrawl)),
        ("LR8V-6B79-S000-RPJG", m3_builtin(ModeKind::LastColorStanding)),
        (
            "LR8V-6097-GQ4N-5V5F-VNM1-9M9P-FZQM-0JR1-Y081-JG83-4A0N-245G-JG3C-A23R-01B2-6BPB-80N2-4CJG-6BS",
            m3(&edited()),
        ),
        ("LR8V-6097-G084-HMY0-2J90", m3(&banned(&["railspike", "tectonic_charge"]))),
    ]
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

/// Every built-in rule set and every mode's default round-trips (format 4,
/// diff and full) and decodes to a valid rule set.
#[test]
fn every_builtin_rule_set_and_mode_default_round_trips() {
    let weapons = WeaponTable::builtin();
    let mut all: Vec<RuleSet> = crate::ruleset::BUILTIN_RULESETS
        .iter()
        .map(|(name, text)| RuleSet::from_ron(name, text).unwrap())
        .collect();
    all.extend(ModeKind::ALL.iter().map(|k| RuleSet::default_for(*k)));
    for r in all {
        let code = round_trip(&r);
        let back = RuleSet::from_code(&code).unwrap();
        assert_eq!(back.mode, r.mode);
        back.resolve(&weapons).unwrap();
    }
    // A mode's default is its shortest code.
    for kind in ModeKind::ALL {
        let code = RuleSet::default_for(kind).to_code().unwrap();
        assert!(!is_full(&code) && code.len() <= 21, "{kind:?}: {code}");
    }
}

/// Every mode setting of every mode round-trips on its own.
#[test]
fn every_mode_setting_round_trips() {
    for kind in ModeKind::ALL {
        let base = RuleSet::default_for(kind);
        for f in base.mode.fields() {
            let mut r = base.clone();
            // The largest value the field carries, and one step.
            for v in [((1u64 << f.bits) - 1) as f32 / f.steps, 1.0 / f.steps] {
                r.mode.set_field(f.name, v);
                if r.mode == base.mode {
                    continue;
                }
                let code = round_trip(&r);
                let back = RuleSet::from_code(&code).unwrap();
                let got = back.mode.fields().into_iter().find(|g| g.name == f.name);
                assert_eq!(got.unwrap().value, v, "{kind:?}.{}", f.name);
            }
        }
    }
}

/// Codes printed by M2 builds (formats 3 and 2, Paint the Void) still decode.
#[test]
fn format_3_and_2_codes_still_decode() {
    for (code, full, rules) in [
        (
            "LR6V-69CM-G006-58R",
            "LR4V-6WGD-RX2W-0Q8K-W0PM-RVR8-0JR7-K4AF-05AN-A5AN-APAM-A40B-1506-A107-H0G3-RG61-W840-Y420-F20R-7H10-2GG8-1W86-0S44-0A8D-0G3T-2840-YGT0-GA42-G61X-1410-PR50-C3T2-840Y-GT0R-7M4G-25N0-RR21-D410-PR8G-42V0-M10P-R908-5MG4-2V14-30F0-0000-1085-MG42-T821-D4R0-7HW0-0000-0000-02B5-4",
            RuleSet::paint_the_void(),
        ),
        (
            "LR6V-69CM-NS59-ESBY-XD0A-D2DK-ZXX0-4P0F-G20C-M20S-2G58-H1C4-M0V2-GGY0-0ARH-JXJT-058H-34MB-XPR",
            "LR4V-6WJM-QCNZ-EPG5-6H6S-ZYW8-15G3-R009-1NAN-J5AN-APAM-C48B-1506-T107-H0G3-RG61-W840-Y420-F20R-7H10-2G86-0S44-0A8D-0G3T-2840-YGT0-GA42-G61X-1410-PR50-C3T2-840Y-GT0R-7M4G-25N0-RR21-D410-PR8G-42V0-M10P-R908-5MG4-2V10-0000-1085-MG42-T00G-0327-CP00-000H-2698-0009-7XG",
            edited(),
        ),
        (
            "LR6V-69CM-G214-D7G1-P6B",
            "LR4V-6WGD-RX2W-0Q8K-W0PM-RVR8-0JR7-K4AF-05AN-T5AN-EPAM-A40B-1506-A107-H0G3-RG61-W840-Y420-F20R-7H10-2G86-0S44-0A8D-0G3T-2840-YGT0-GA42-G61X-1410-PR50-C3T2-840Y-GT0R-7M48-21D4-10PR-8G42-V0M1-0PR9-085M-G42V-1430-F000-0010-85MG-42T8-21D4-R07H-W000-0000-0002-1DR",
            banned(&["railspike", "tectonic_charge"]),
        ),
    ] {
        assert_eq!(version_of(code), LEGACY_DIFF_VERSION as u64);
        assert_eq!(version_of(full), LEGACY_FULL_VERSION as u64);
        assert_eq!(RuleSet::from_code(code).unwrap(), m3(&rules), "{code}");
        assert_eq!(RuleSet::from_code(full).unwrap(), m3(&rules), "{full}");
        // Re-encoding prints format 5.
        assert!(!is_full(&rules.to_code().unwrap()));
    }
    // A typo in an old code is still a typo.
    let e = RuleSet::from_code("LR6V-69CM-G006-58S")
        .unwrap_err()
        .to_string();
    assert!(e.contains("checksum"), "{e}");
}

/// A format 4 code with a mode or mutator this build does not know is
/// rejected with a clear message.
#[test]
fn unknown_modes_and_mutators_are_rejected() {
    let r = RuleSet::default_for(ModeKind::TeamBrawl);
    let (w, g, e) = all_ids();
    let mut payload = diff_payload(&r, ids(&w, &g, &e)).unwrap();
    // The mode index (the 4 bits after the full/diff bit) to 15.
    for b in &mut payload.bits[HEADER_BITS + 1..HEADER_BITS + 5] {
        *b = true;
    }
    let e = RuleSet::from_code(&finish(payload))
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown mode"), "{e}");

    // Mutator bit 3 in a full code: rejected after the checksum check.
    let mut r = RuleSet::paint_the_void();
    r.mutators = vec![Mutator::Instagib];
    let full = full_payload(&r, ids(&w, &g, &event_ids()), CODE_VERSION).unwrap();
    let mut bits = full.bits.clone();
    // After version 4, counts 15, full 1, mode 4, name, mode fields,
    // friendly fire 7, absurdity 7, ships 4: the 4 mutator bits.
    let name_bits = 5 + 6 * r.name.len();
    let mode_bits: u32 = r.mode.fields().iter().map(|f| f.bits).sum();
    let at = HEADER_BITS + 5 + name_bits + mode_bits as usize + 18;
    assert!(bits[at + 3], "Instagib is bit 0 (the last of the four)");
    bits[at] = true;
    let e = RuleSet::from_code(&finish(Bits { bits }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown mutator"), "{e}");
}

/// Codes printed by M3 builds (format 4) still decode: against the M3
/// default rule sets (Absurdity 20), with today's pigment pickups and no
/// event bans.
#[test]
fn format_4_codes_still_decode() {
    let builtin = |file: &str| {
        let (name, text) = crate::ruleset::BUILTIN_RULESETS
            .iter()
            .find(|(f, _)| *f == file)
            .unwrap();
        // Every M3 built-in rule set had Absurdity 20.
        let mut r = RuleSet::from_ron(name, text).unwrap();
        r.absurdity = LEGACY_ABSURDITY;
        r
    };
    let mode = |kind: ModeKind, f: &dyn Fn(&mut RuleSet)| {
        let mut r = RuleSet::default_for(kind);
        f(&mut r);
        r
    };
    let cases: Vec<(&str, RuleSet)> = vec![
        // The M3 built-in rule sets.
        ("LR8V-6097-G000-TMBG", builtin("rulesets/paint_the_void.ron")),
        ("LR8V-71AK-DT3S-S9S0-7W0V-K9G3-2ZDT-N6HD-80JR-7K4A-F05A-NA5A-NAMA-MA40-B150-6C20-7J10-3S0G-1WG8-0Y84-0F42-07J1-03S0-G1WG-80Y8-40F4-207R-D0G1-Y284-0FGT-103W-2G80-Z142-07R5-0G1Y-2840-FGT1-03W4-G83S-0RR4-0F42-07R9-0G1W-G80Z-0M20-7R90-G1WG-80Z1-4207-G000-010G-1WG8-0Y84-0F4C-07HW-0000-0000-000R-A2", builtin("rulesets/sandbox_all_weapons.ron")),
        ("LR8V-63T6-X000-NNJ0", builtin("rulesets/team_brawl.ron")),
        ("LR8V-72W9-MBDS-Q1HQ-005C-DWCS-04P0-Y00B-4AF3-ZZZQ-ZZZZ-ZZZY-2800-0000-0000-0000-0000-0000-00DG-7", builtin("rulesets/instagib_brawl.ron")),
        ("LR8V-63T6-XJ15-XZ6G-09C3-R510-0078-0", builtin("rulesets/duel.ron")),
        ("LR8V-69BZ-2000-Y3DG", builtin("rulesets/hot_reactor.ron")),
        ("LR8V-6B79-S000-RPJG", builtin("rulesets/last_color_standing.ron")),
        ("LR8V-67TK-E000-EQ2G", builtin("rulesets/heist.ron")),
        ("LR8V-65PE-T000-XS10", builtin("rulesets/squeeze.ron")),
        // Edited M3 rule sets (vectors of the M3 web catalog).
        ("LR8V-6097-GQ4N-5V5F-VNM1-9M9P-FZQM-0JR1-Y081-JG83-4A0N-245G-JG3C-A23R-01B2-6BPB-80N2-4CJG-6BS", edited()),
        ("LR8V-6097-G084-HMY0-2J90", banned(&["railspike", "tectonic_charge"])),
        ("LR8V-6097-G0H0-0HS6", {
            let mut r = RuleSet::paint_the_void();
            r.mutators = vec![Mutator::Instagib];
            r
        }),
        ("LR8V-63T6-XJ15-XZ6G-09C3-R512-406K-YW", mode(ModeKind::TeamBrawl, &|r| {
            r.name = "Duel".into();
            r.mode.set_field("teams", 0.0);
            r.mode.set_field("kill_target", 10.0);
            r.mode.set_field("match_secs", 300.0);
            r.mutators = vec![Mutator::Instagib];
        })),
        ("LR8V-65PE-T80E-2003-RCA1-WA05-003V-8C", mode(ModeKind::TheSqueeze, &|r| {
            r.mode.set_field("phases", 3.0);
            r.mode.set_field("crush_per_sec", 40.0);
            r.mode.set_field("scrap_per_100u2", 1.25);
        })),
        ("LR8V-6B79-S80E-21XJ-689C-2G00-287R", mode(ModeKind::LastColorStanding, &|r| {
            r.mode.set_field("drain_per_kill", 50.0);
            r.mode.set_field("stalemate_secs", 300.0);
        })),
    ];
    let weapons = WeaponTable::builtin();
    for (code, rules) in cases {
        assert_eq!(version_of(code), LEGACY_V4_VERSION as u64, "{code}");
        let back = RuleSet::from_code(code).unwrap_or_else(|e| panic!("{code}: {e}"));
        assert_eq!(back, m3(&rules), "{code}");
        back.resolve(&weapons).unwrap();
        // Re-encoding prints format 5 and round-trips.
        round_trip(&back);
    }
    // A typo in an M3 code is still a typo.
    let e = RuleSet::from_code("LR8V-6097-G000-TMBH")
        .unwrap_err()
        .to_string();
    assert!(e.contains("checksum"), "{e}");
}

/// Event bans and pigment pickups round-trip, in the diff and the full
/// form, and cost little.
#[test]
fn event_bans_and_pigment_round_trip() {
    let base = RuleSet::paint_the_void();
    let base_code = base.to_code().unwrap();
    let mut r = base.clone();
    r.ban_event("duck_parade", true);
    let one = round_trip(&r);
    assert!(one.len() <= base_code.len() + 3, "{base_code} -> {one}");
    assert!(RuleSet::from_code(&one)
        .unwrap()
        .event_banned("duck_parade"));
    for id in event_ids() {
        r.ban_event(id, true);
    }
    let all = round_trip(&r);
    assert_eq!(
        RuleSet::from_code(&all).unwrap().banned_events(),
        (1 << event_ids().len()) - 1
    );
    r.resolve(&WeaponTable::builtin()).unwrap();
    // A base that bans an event: allowing it again is a change too.
    let mut base_ban = base.clone();
    base_ban.ban_event("cat_paw", true);
    round_trip(&base_ban);

    let mut p = base.clone();
    p.pigment_pickup = Some(crate::PigmentPickupRule {
        respawn_secs: 1023,
        players_per_pickup: 15,
        min_on_map: 15,
    });
    round_trip(&p);
    p.pigment_pickup = None;
    round_trip(&p);
    p.pigment_pickup = Some(crate::PigmentPickupRule {
        respawn_secs: 1024,
        players_per_pickup: 4,
        min_on_map: 2,
    });
    assert!(p.to_code().is_err());
    // An unknown event id cannot be encoded (and does not resolve).
    let mut u = base.clone();
    u.events
        .insert("tea_party".into(), crate::EventRule::Banned);
    assert!(u.to_code().is_err());
    assert!(u.resolve(&WeaponTable::builtin()).is_err());
}
