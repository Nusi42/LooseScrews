//! Rule set codes: a whole [`RuleSet`] as a short, typeable string, like
//! world share codes (design doc: "Rule sets save and share as their own
//! short code"). An invite link carries a world code and a rule set code.
//!
//! The printed code is `LR` followed by Crockford base32 symbols grouped in
//! fours (`LRab-cdef-...`). Decoding ignores case, dashes and whitespace and
//! reads `O` as `0` and `I`/`L` as `1` (after the `LR` prefix). The payload
//! bits (most significant bit first) start with a 4-bit format version:
//!
//! - **Version 3, diff** (what [`RuleSet::to_code`] normally prints): only
//!   the settings that differ from the mode's default rule set
//!   (`data/rulesets/`). The default Paint the Void rules are 13 symbols
//!   (`LR` + 13 symbols + 3 dashes = 18 characters); banning a weapon adds
//!   3 symbols (its pickups leave with it).
//! - **Version 2, full** ([`RuleSet::to_full_code`], the fallback): every
//!   setting; about 210 symbols for the default rules. `to_code` prints it
//!   only when it is shorter than the diff (almost everything changed).
//!
//! Both end with a CRC-16/CCITT-FALSE over the payload bits (packed MSB first,
//! zero padded) and are zero padded to whole symbols.
//!
//! # Common header
//!
//! | Bits | Field |
//! | --- | --- |
//! | 4 | format version (2 or 3) |
//! | 6, 4 | number of weapons and gadgets the code covers (the registry lengths of the build that made it) |
//!
//! "Per weapon" and "per gadget" fields cover the counts from the header, in
//! registry order (`weapons::MODULES`, `gadgets::MODULES`; append-only). A
//! code from an older build (fewer weapons) decodes with the missing weapons
//! banned and without rules; a code from a newer build (more weapons than
//! this build knows) is rejected with a clear error instead of misparsing.
//!
//! # Version 2 (full)
//!
//! | Bits | Field |
//! | --- | --- |
//! | 5 + 6 each | name: length (0..=31), characters from `" A-Za-z0-9-"` |
//! | 2, 3, 16, 8 | mode (0 = Paint the Void), teams, match seconds (whole), respawn (tenths of a second) |
//! | 7, 7 | friendly fire percent, absurdity |
//! | 4 | allowed ship types (bit = `ShipType::index`) |
//! | 4 | mutators (bit 0 = Instagib) |
//! | 2 per weapon | availability |
//! | 2 per gadget | availability |
//! | 5 + 6 each | loadout primaries (registry index) |
//! | 4 + (6 + 8) each | loadout secondaries (registry index, count) |
//! | 3 | loadout gadget (0 = none, else registry index + 1) |
//! | per weapon: 1 [+ rule] | weapon pickups |
//! | per weapon: 1 [+ rule] | upgrade pickups |
//! | per gadget: 1 [+ rule] | gadget pickups |
//! | 1 [+ 8 + 10 + 8] | scrap pickup: weight (quarters), respawn (seconds), amount (whole) |
//! | per weapon: 1 [+ 2 + 8] | shield override: kind (Blocked, Wears, PassThrough), value (hundredths) |
//! | per weapon: 1 [+ 8 + 8 + 8] | tuning: damage, fire rate, ammo multipliers (twentieths) |
//! | 16 | CRC-16 |
//!
//! A pickup rule is weight (8 bits, quarters, up to 63.75), respawn (10
//! bits, whole seconds), then 1 bit + 6 bits ammo if non-zero and 1 bit + 4
//! bits `max_on_map` if non-zero.
//!
//! # Version 3 (diff against the mode default)
//!
//! | Bits | Field |
//! | --- | --- |
//! | 2 | mode (0 = Paint the Void; its default rule set is the base) |
//! | 16 | base fingerprint: CRC-16 of the base's version 2 payload over the covered weapons and gadgets |
//! | 1 [+ name] | name (as in version 2) |
//! | 1 [+ 3 + 16 + 8] | mode settings: teams, match seconds, respawn tenths |
//! | 1 [+ 7], 1 [+ 7] | friendly fire percent, absurdity |
//! | 1 [+ 4], 1 [+ 4] | allowed ship types, mutators |
//! | changes (2) | weapon availability |
//! | changes (2), 4-bit indices | gadget availability |
//! | | (the base now takes the new availability; pickups and loadout entries it no longer allows leave the base) |
//! | 1 [+ loadout] | the whole starting loadout (as in version 2) |
//! | changes (1 [+ rule]) | weapon pickups, then upgrade pickups |
//! | changes (1 [+ rule]), 4-bit indices | gadget pickups |
//! | 1 [+ 1 [+ 26]] | scrap pickup (present, then as in version 2) |
//! | changes (1 [+ 2 + 8]) | shield overrides |
//! | changes (1 [+ 24]) | tuning |
//! | 16 | CRC-16 |
//!
//! A "changes" list is 1 bit (0 = nothing changed) or 1 bit, the number of
//! entries minus one (6 bits for weapons, 4 for gadgets) and the entries:
//! registry index (strictly increasing) and the new value; for per-weapon
//! maps the value is 1 bit present (0 = removed) plus the value. Every entry
//! must differ from the base. The base fingerprint catches codes made against
//! a different default rule set (another build's data): they are rejected
//! with a clear message instead of decoding to different rules.
//!
//! Values that the format cannot carry exactly (a weight of 0.3, a respawn of
//! 2.5 s, an unknown weapon id, a name with other characters) make
//! [`RuleSet::to_code`] fail, so every code decodes to exactly the rule set
//! it was made from (canonical form: banned weapons and gadgets left out of
//! the availability maps, ship types in index order).

use std::collections::BTreeMap;

use crate::gadgets::MODULES as GADGETS;
use crate::ruleset::{
    Availability, Loadout, ModeRules, Mutator, PaintTheVoidRules, PickupRule, RuleSet,
    ScrapPickupRule, WeaponTuning,
};
use crate::weapons::MODULES as WEAPONS;
use crate::{ShieldInteraction, ShipType, SimError};

/// Prefix of every printed rule set code.
pub const PREFIX: &str = "LR";
/// Version of the full format ([`RuleSet::to_full_code`]).
pub const FULL_VERSION: u8 = 2;
/// Version of the diff format (what [`RuleSet::to_code`] prints).
pub const DIFF_VERSION: u8 = 3;
/// Newest format version this build writes.
pub const CODE_VERSION: u8 = DIFF_VERSION;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const NAME_CHARS: &[u8; 64] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-";
/// Longest rule set name a code carries.
pub const MAX_NAME: usize = 31;
const WEAPON_BITS: u32 = 6;
const GADGET_BITS: u32 = 4;

fn err(m: impl Into<String>) -> SimError {
    SimError::RuleSetCode(m.into())
}

#[derive(Default)]
struct Bits {
    bits: Vec<bool>,
}

impl Bits {
    fn push(&mut self, v: u64, n: u32) {
        for i in (0..n).rev() {
            self.bits.push((v >> i) & 1 == 1);
        }
    }

    fn flag(&mut self, v: bool) {
        self.bits.push(v);
    }

    fn bytes(&self, len: usize) -> Vec<u8> {
        pack(&self.bits[..len])
    }
}

fn pack(bits: &[bool]) -> Vec<u8> {
    let mut out = vec![0u8; bits.len().div_ceil(8)];
    for (i, &b) in bits.iter().enumerate() {
        if b {
            out[i / 8] |= 0x80 >> (i % 8);
        }
    }
    out
}

struct Reader<'a> {
    bits: &'a [bool],
    pos: usize,
}

impl Reader<'_> {
    fn read(&mut self, n: u32) -> Result<u64, SimError> {
        if self.pos + n as usize > self.bits.len() {
            return Err(err("code is too short"));
        }
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | self.bits[self.pos] as u64;
            self.pos += 1;
        }
        Ok(v)
    }

    fn flag(&mut self) -> Result<bool, SimError> {
        Ok(self.read(1)? == 1)
    }
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &b in bytes {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// `v` as an exact multiple of `1/steps` in `0..=max` steps, or an error.
fn fixed(v: f32, steps: f32, max: u64, what: &str) -> Result<u64, SimError> {
    if !v.is_finite() || v < 0.0 {
        return Err(err(format!("{what} {v} must be >= 0")));
    }
    let q = ls_math::det::round(v * steps);
    if q > max as f32 || q / steps != v {
        return Err(err(format!(
            "{what} {v} is not a multiple of {} up to {}",
            1.0 / steps,
            max as f32 / steps
        )));
    }
    Ok(q as u64)
}

fn index_of(ids: &[&str], id: &str, what: &str) -> Result<u64, SimError> {
    ids.iter()
        .position(|w| *w == id)
        .map(|i| i as u64)
        .ok_or_else(|| err(format!("{what} `{id}` has no rule set code index")))
}

/// Weapon ids in registry order (rule set code indices).
pub fn weapon_ids() -> Vec<&'static str> {
    WEAPONS.iter().map(|m| m.id).collect()
}

/// Gadget ids in registry order.
pub fn gadget_ids() -> Vec<&'static str> {
    GADGETS.iter().map(|m| m.id).collect()
}

fn write_rule(b: &mut Bits, r: &PickupRule, what: &str) -> Result<(), SimError> {
    b.push(fixed(r.weight, 4.0, 255, what)?, 8);
    b.push(fixed(r.respawn_secs, 1.0, 1023, what)?, 10);
    if r.ammo > 63 || r.max_on_map > 15 {
        return Err(err(format!("{what}: ammo <= 63 and max_on_map <= 15")));
    }
    b.flag(r.ammo != 0);
    if r.ammo != 0 {
        b.push(r.ammo as u64, 6);
    }
    b.flag(r.max_on_map != 0);
    if r.max_on_map != 0 {
        b.push(r.max_on_map as u64, 4);
    }
    Ok(())
}

fn read_rule(r: &mut Reader) -> Result<PickupRule, SimError> {
    let weight = r.read(8)? as f32 / 4.0;
    let respawn_secs = r.read(10)? as f32;
    let ammo = if r.flag()? { r.read(6)? as u32 } else { 0 };
    let max_on_map = if r.flag()? { r.read(4)? as u32 } else { 0 };
    Ok(PickupRule {
        weight,
        respawn_secs,
        ammo,
        max_on_map,
    })
}

fn write_name(b: &mut Bits, name: &str) -> Result<(), SimError> {
    let name = name.as_bytes();
    if name.len() > MAX_NAME {
        return Err(err(format!("name longer than {MAX_NAME} characters")));
    }
    b.push(name.len() as u64, 5);
    for &c in name {
        let i = NAME_CHARS
            .iter()
            .position(|&x| x == c)
            .ok_or_else(|| err("name may only use letters, digits, spaces and '-'"))?;
        b.push(i as u64, 6);
    }
    Ok(())
}

fn read_name(r: &mut Reader) -> Result<String, SimError> {
    let len = r.read(5)? as usize;
    let mut name = String::with_capacity(len);
    for _ in 0..len {
        name.push(NAME_CHARS[r.read(6)? as usize] as char);
    }
    Ok(name)
}

fn write_mode(b: &mut Bits, m: &PaintTheVoidRules) -> Result<(), SimError> {
    if m.teams > 7 {
        return Err(err("teams must be <= 7"));
    }
    b.push(m.teams as u64, 3);
    b.push(fixed(m.match_secs, 1.0, 65535, "match_secs")?, 16);
    b.push(fixed(m.respawn_secs, 10.0, 255, "respawn_secs")?, 8);
    Ok(())
}

fn read_mode(r: &mut Reader) -> Result<PaintTheVoidRules, SimError> {
    Ok(PaintTheVoidRules {
        teams: r.read(3)? as u8,
        match_secs: r.read(16)? as f32,
        respawn_secs: r.read(8)? as f32 / 10.0,
    })
}

fn percent(v: u8, what: &str) -> Result<u64, SimError> {
    if v > 100 {
        return Err(err(format!("{what} must be <= 100")));
    }
    Ok(v as u64)
}

fn ship_mask(types: &[ShipType]) -> u64 {
    types.iter().fold(0, |m, t| m | 1 << t.index())
}

fn ships_from_mask(mask: u64) -> Vec<ShipType> {
    ShipType::ALL
        .iter()
        .copied()
        .filter(|t| mask & (1 << t.index()) != 0)
        .collect()
}

fn mutator_mask(mutators: &[Mutator]) -> u64 {
    mutators.iter().fold(0, |m, x| match x {
        Mutator::Instagib => m | 1,
    })
}

fn mutators_from_mask(mask: u64) -> Vec<Mutator> {
    let mut out = Vec::new();
    if mask & 1 != 0 {
        out.push(Mutator::Instagib);
    }
    out
}

fn write_loadout(
    b: &mut Bits,
    lo: &Loadout,
    weapons: &[&str],
    gadgets: &[&str],
) -> Result<(), SimError> {
    if lo.primaries.len() > 31 || lo.secondaries.len() > 15 {
        return Err(err("loadout too large"));
    }
    b.push(lo.primaries.len() as u64, 5);
    for id in &lo.primaries {
        b.push(index_of(weapons, id, "weapon")?, WEAPON_BITS);
    }
    b.push(lo.secondaries.len() as u64, 4);
    for (id, n) in &lo.secondaries {
        b.push(index_of(weapons, id, "weapon")?, WEAPON_BITS);
        if *n > 255 {
            return Err(err("loadout counts must be <= 255"));
        }
        b.push(*n as u64, 8);
    }
    match &lo.gadget {
        None => b.push(0, 3),
        Some(id) => b.push(index_of(gadgets, id, "gadget")? + 1, 3),
    }
    Ok(())
}

fn read_loadout(r: &mut Reader, weapons: &[&str], gadgets: &[&str]) -> Result<Loadout, SimError> {
    let weapon_id = |i: u64| -> Result<String, SimError> {
        weapons
            .get(i as usize)
            .map(|m| m.to_string())
            .ok_or_else(|| err("weapon index out of range"))
    };
    let mut primaries = Vec::new();
    for _ in 0..r.read(5)? {
        primaries.push(weapon_id(r.read(WEAPON_BITS)?)?);
    }
    let mut secondaries = Vec::new();
    for _ in 0..r.read(4)? {
        let id = weapon_id(r.read(WEAPON_BITS)?)?;
        secondaries.push((id, r.read(8)? as u32));
    }
    let gadget = match r.read(3)? {
        0 => None,
        i => Some(
            gadgets
                .get(i as usize - 1)
                .ok_or_else(|| err("gadget index out of range"))?
                .to_string(),
        ),
    };
    Ok(Loadout {
        primaries,
        secondaries,
        gadget,
    })
}

fn write_scrap(b: &mut Bits, s: &ScrapPickupRule) -> Result<(), SimError> {
    b.push(fixed(s.weight, 4.0, 255, "scrap weight")?, 8);
    b.push(fixed(s.respawn_secs, 1.0, 1023, "scrap respawn")?, 10);
    b.push(fixed(s.amount, 1.0, 255, "scrap amount")?, 8);
    Ok(())
}

fn read_scrap(r: &mut Reader) -> Result<ScrapPickupRule, SimError> {
    Ok(ScrapPickupRule {
        weight: r.read(8)? as f32 / 4.0,
        respawn_secs: r.read(10)? as f32,
        amount: r.read(8)? as f32,
    })
}

fn write_shield(b: &mut Bits, s: &ShieldInteraction, _: &str) -> Result<(), SimError> {
    let (kind, v) = match *s {
        ShieldInteraction::Blocked => (0, 0),
        ShieldInteraction::Wears(f) => (1, fixed(f, 100.0, 255, "shield wear")?),
        ShieldInteraction::PassThrough(f) => (2, fixed(f, 100.0, 100, "shield pass-through")?),
    };
    b.push(kind, 2);
    b.push(v, 8);
    Ok(())
}

fn read_shield(r: &mut Reader) -> Result<ShieldInteraction, SimError> {
    let kind = r.read(2)?;
    let v = r.read(8)? as f32 / 100.0;
    match kind {
        0 => Ok(ShieldInteraction::Blocked),
        1 => Ok(ShieldInteraction::Wears(v)),
        2 => Ok(ShieldInteraction::PassThrough(v)),
        _ => Err(err("unknown shield interaction")),
    }
}

fn write_tuning(b: &mut Bits, t: &WeaponTuning, _: &str) -> Result<(), SimError> {
    b.push(fixed(t.damage, 20.0, 255, "damage multiplier")?, 8);
    b.push(fixed(t.fire_rate, 20.0, 255, "fire rate multiplier")?, 8);
    b.push(fixed(t.ammo, 20.0, 255, "ammo multiplier")?, 8);
    Ok(())
}

fn read_tuning(r: &mut Reader) -> Result<WeaponTuning, SimError> {
    Ok(WeaponTuning {
        damage: r.read(8)? as f32 / 20.0,
        fire_rate: r.read(8)? as f32 / 20.0,
        ammo: r.read(8)? as f32 / 20.0,
    })
}

/// Every id the rule set mentions must have a code index.
fn check_ids(r: &RuleSet, weapons: &[&str], gadgets: &[&str]) -> Result<(), SimError> {
    for id in r
        .weapons
        .keys()
        .chain(r.pickups.keys())
        .chain(r.upgrade_pickups.keys())
        .chain(r.shield_overrides.keys())
        .chain(r.tuning.keys())
        .chain(r.starting_loadout.primaries.iter())
        .chain(r.starting_loadout.secondaries.iter().map(|(w, _)| w))
    {
        index_of(weapons, id, "weapon")?;
    }
    for id in r
        .gadgets
        .keys()
        .chain(r.gadget_pickups.keys())
        .chain(r.starting_loadout.gadget.iter())
    {
        index_of(gadgets, id, "gadget")?;
    }
    Ok(())
}

/// The form every decoded rule set has: banned weapons and gadgets left out
/// of the availability maps, ship types in index order without duplicates.
pub fn canonical(r: &RuleSet) -> RuleSet {
    let mut c = r.clone();
    c.weapons.retain(|_, a| *a != Availability::Banned);
    c.gadgets.retain(|_, a| *a != Availability::Banned);
    c.allowed_ship_types = ships_from_mask(ship_mask(&r.allowed_ship_types));
    c.mutators = mutators_from_mask(mutator_mask(&r.mutators));
    c
}

/// The mode's default rule set (the base of diff codes), restricted to the
/// weapons and gadgets a code covers, in canonical form.
fn base_for(mode: u64, weapons: &[&str], gadgets: &[&str]) -> Result<RuleSet, SimError> {
    let mut base = match mode {
        0 => RuleSet::paint_the_void(),
        _ => return Err(err("unknown mode")),
    };
    let w = |id: &String| weapons.contains(&id.as_str());
    let g = |id: &String| gadgets.contains(&id.as_str());
    base.weapons.retain(|k, _| w(k));
    base.pickups.retain(|k, _| w(k));
    base.upgrade_pickups.retain(|k, _| w(k));
    base.shield_overrides.retain(|k, _| w(k));
    base.tuning.retain(|k, _| w(k));
    base.gadgets.retain(|k, _| g(k));
    base.gadget_pickups.retain(|k, _| g(k));
    let lo = &mut base.starting_loadout;
    lo.primaries.retain(w);
    lo.secondaries.retain(|(id, _)| w(id));
    if lo.gadget.as_ref().is_some_and(|id| !g(id)) {
        lo.gadget = None;
    }
    Ok(canonical(&base))
}

fn mode_index(m: &ModeRules) -> u64 {
    m.kind().index() as u64
}

/// Version 2 payload (header included, checksum excluded).
fn full_payload(r: &RuleSet, weapons: &[&str], gadgets: &[&str]) -> Result<Bits, SimError> {
    let mut b = Bits::default();
    b.push(FULL_VERSION as u64, 4);
    header_counts(&mut b, weapons, gadgets)?;
    write_name(&mut b, &r.name)?;
    let ModeRules::PaintTheVoid(m) = &r.mode else {
        return Err(err("only Paint the Void has a rule set code yet"));
    };
    b.push(mode_index(&r.mode), 2);
    write_mode(&mut b, m)?;
    b.push(percent(r.friendly_fire_percent, "friendly fire")?, 7);
    b.push(percent(r.absurdity, "absurdity")?, 7);
    b.push(ship_mask(&r.allowed_ship_types), 4);
    b.push(mutator_mask(&r.mutators), 4);
    check_ids(r, weapons, gadgets)?;
    for &w in weapons {
        b.push(r.availability(w).index() as u64, 2);
    }
    for &g in gadgets {
        b.push(r.gadget_availability(g).index() as u64, 2);
    }
    write_loadout(&mut b, &r.starting_loadout, weapons, gadgets)?;
    write_all(&mut b, weapons, &r.pickups, write_rule)?;
    write_all(&mut b, weapons, &r.upgrade_pickups, write_rule)?;
    write_all(&mut b, gadgets, &r.gadget_pickups, write_rule)?;
    match &r.scrap_pickup {
        None => b.flag(false),
        Some(s) => {
            b.flag(true);
            write_scrap(&mut b, s)?;
        }
    }
    write_all(&mut b, weapons, &r.shield_overrides, write_shield)?;
    write_all(&mut b, weapons, &r.tuning, write_tuning)?;
    Ok(b)
}

fn header_counts(b: &mut Bits, weapons: &[&str], gadgets: &[&str]) -> Result<(), SimError> {
    if weapons.len() > 63 || gadgets.len() > 15 {
        return Err(err("too many weapons or gadgets for the code format"));
    }
    b.push(weapons.len() as u64, 6);
    b.push(gadgets.len() as u64, 4);
    Ok(())
}

type Writer<T> = fn(&mut Bits, &T, &str) -> Result<(), SimError>;
type ReadFn<T> = fn(&mut Reader) -> Result<T, SimError>;

/// Version 2 per-id map: 1 bit per id, then the value if present.
fn write_all<T>(
    b: &mut Bits,
    ids: &[&str],
    map: &BTreeMap<String, T>,
    write: Writer<T>,
) -> Result<(), SimError> {
    for &id in ids {
        match map.get(id) {
            Some(v) => {
                b.flag(true);
                write(b, v, id)?;
            }
            None => b.flag(false),
        }
    }
    Ok(())
}

fn read_all<T>(
    r: &mut Reader,
    ids: &[&str],
    read: ReadFn<T>,
) -> Result<BTreeMap<String, T>, SimError> {
    let mut out = BTreeMap::new();
    for &id in ids {
        if r.flag()? {
            out.insert(id.to_string(), read(r)?);
        }
    }
    Ok(out)
}

/// Version 3 "changes" list over the ids where `changed` holds.
fn write_changes(
    b: &mut Bits,
    ids: &[&str],
    idx_bits: u32,
    changed: impl Fn(&str) -> bool,
    mut write: impl FnMut(&mut Bits, &str) -> Result<(), SimError>,
) -> Result<(), SimError> {
    let list: Vec<usize> = (0..ids.len()).filter(|&i| changed(ids[i])).collect();
    if list.is_empty() {
        b.flag(false);
        return Ok(());
    }
    b.flag(true);
    b.push(list.len() as u64 - 1, idx_bits);
    for i in list {
        b.push(i as u64, idx_bits);
        write(b, ids[i])?;
    }
    Ok(())
}

/// Reads a "changes" list, calling `apply` with each id (strictly increasing).
fn read_changes<'a>(
    r: &mut Reader,
    ids: &[&'a str],
    idx_bits: u32,
    mut apply: impl FnMut(&mut Reader, &'a str) -> Result<(), SimError>,
) -> Result<(), SimError> {
    if !r.flag()? {
        return Ok(());
    }
    let n = r.read(idx_bits)? + 1;
    let mut last: Option<u64> = None;
    for _ in 0..n {
        let i = r.read(idx_bits)?;
        if last.is_some_and(|l| i <= l) || i as usize >= ids.len() {
            return Err(err("corrupt code (bad index list)"));
        }
        last = Some(i);
        apply(r, ids[i as usize])?;
    }
    Ok(())
}

/// Version 3 per-id map diff: changed ids, then 1 bit present + value.
fn write_map_diff<T: PartialEq>(
    b: &mut Bits,
    ids: &[&str],
    idx_bits: u32,
    map: &BTreeMap<String, T>,
    base: &BTreeMap<String, T>,
    write: Writer<T>,
) -> Result<(), SimError> {
    write_changes(
        b,
        ids,
        idx_bits,
        |id| map.get(id) != base.get(id),
        |b, id| match map.get(id) {
            Some(v) => {
                b.flag(true);
                write(b, v, id)
            }
            None => {
                b.flag(false);
                Ok(())
            }
        },
    )
}

fn read_map_diff<T: PartialEq>(
    r: &mut Reader,
    ids: &[&str],
    idx_bits: u32,
    map: &mut BTreeMap<String, T>,
    read: ReadFn<T>,
) -> Result<(), SimError> {
    read_changes(r, ids, idx_bits, |r, id| {
        let v = if r.flag()? { Some(read(r)?) } else { None };
        if v.as_ref() == map.get(id) {
            return Err(err("corrupt code (an entry equals the default)"));
        }
        match v {
            Some(v) => map.insert(id.to_string(), v),
            None => map.remove(id),
        };
        Ok(())
    })
}

/// Version 3 availability diff (2 bits per changed id).
fn write_availability_diff(
    b: &mut Bits,
    ids: &[&str],
    idx_bits: u32,
    get: impl Fn(&str) -> Availability,
    base: impl Fn(&str) -> Availability,
) -> Result<(), SimError> {
    write_changes(
        b,
        ids,
        idx_bits,
        |id| get(id) != base(id),
        |b, id| {
            b.push(get(id).index() as u64, 2);
            Ok(())
        },
    )
}

fn read_availability_diff(
    r: &mut Reader,
    ids: &[&str],
    idx_bits: u32,
    map: &mut BTreeMap<String, Availability>,
) -> Result<(), SimError> {
    read_changes(r, ids, idx_bits, |r, id| {
        let a = Availability::from_index(r.read(2)? as u8);
        let old = map.get(id).copied().unwrap_or(Availability::Banned);
        if a == old {
            return Err(err("corrupt code (an entry equals the default)"));
        }
        if a == Availability::Banned {
            map.remove(id);
        } else {
            map.insert(id.to_string(), a);
        }
        Ok(())
    })
}

/// Drops pickups and loadout entries that the availability no longer allows
/// (diff codes: banning a weapon costs one availability entry).
fn implied_by_availability(r: &mut RuleSet) {
    let (weapons, gadgets) = (r.weapons.clone(), r.gadgets.clone());
    let wa = |id: &String| weapons.get(id).copied().unwrap_or(Availability::Banned);
    let ga = |id: &String| gadgets.get(id).copied().unwrap_or(Availability::Banned);
    r.pickups.retain(|id, _| wa(id).as_pickup());
    r.upgrade_pickups.retain(|id, _| wa(id).as_pickup());
    r.gadget_pickups.retain(|id, _| ga(id).as_pickup());
    let lo = &mut r.starting_loadout;
    lo.primaries.retain(|id| wa(id).in_loadout());
    lo.secondaries.retain(|(id, _)| wa(id).in_loadout());
    if lo.gadget.as_ref().is_some_and(|id| !ga(id).in_loadout()) {
        lo.gadget = None;
    }
}

/// Fingerprint of a (restricted, canonical) base rule set.
fn fingerprint(base: &RuleSet, weapons: &[&str], gadgets: &[&str]) -> Result<u16, SimError> {
    let b = full_payload(base, weapons, gadgets)
        .map_err(|e| err(format!("the default rule set cannot be encoded: {e}")))?;
    Ok(crc16(&b.bytes(b.bits.len())))
}

/// Version 3 payload (header included, checksum excluded).
fn diff_payload(r: &RuleSet, weapons: &[&str], gadgets: &[&str]) -> Result<Bits, SimError> {
    check_ids(r, weapons, gadgets)?;
    let r = canonical(r);
    let mode = mode_index(&r.mode);
    let base = base_for(mode, weapons, gadgets)?;
    let mut b = Bits::default();
    b.push(DIFF_VERSION as u64, 4);
    header_counts(&mut b, weapons, gadgets)?;
    b.push(mode, 2);
    b.push(fingerprint(&base, weapons, gadgets)? as u64, 16);

    b.flag(r.name != base.name);
    if r.name != base.name {
        write_name(&mut b, &r.name)?;
    }
    let (m, bm) = (r.paint_the_void_rules(), base.paint_the_void_rules());
    b.flag(m != bm);
    if m != bm {
        write_mode(&mut b, &m)?;
    }
    for (v, bv, what) in [
        (
            r.friendly_fire_percent,
            base.friendly_fire_percent,
            "friendly fire",
        ),
        (r.absurdity, base.absurdity, "absurdity"),
    ] {
        b.flag(v != bv);
        if v != bv {
            b.push(percent(v, what)?, 7);
        }
    }
    for (v, bv) in [
        (
            ship_mask(&r.allowed_ship_types),
            ship_mask(&base.allowed_ship_types),
        ),
        (mutator_mask(&r.mutators), mutator_mask(&base.mutators)),
    ] {
        b.flag(v != bv);
        if v != bv {
            b.push(v, 4);
        }
    }
    write_availability_diff(
        &mut b,
        weapons,
        WEAPON_BITS,
        |id| r.availability(id),
        |id| base.availability(id),
    )?;
    write_availability_diff(
        &mut b,
        gadgets,
        GADGET_BITS,
        |id| r.gadget_availability(id),
        |id| base.gadget_availability(id),
    )?;
    // Later sections diff against the base with the new availability and
    // what it implies (a ban drops the weapon's pickups and loadout entry).
    let mut base = base;
    base.weapons = r.weapons.clone();
    base.gadgets = r.gadgets.clone();
    implied_by_availability(&mut base);
    let lo_changed = r.starting_loadout != base.starting_loadout;
    b.flag(lo_changed);
    if lo_changed {
        write_loadout(&mut b, &r.starting_loadout, weapons, gadgets)?;
    }
    write_map_diff(
        &mut b,
        weapons,
        WEAPON_BITS,
        &r.pickups,
        &base.pickups,
        write_rule,
    )?;
    write_map_diff(
        &mut b,
        weapons,
        WEAPON_BITS,
        &r.upgrade_pickups,
        &base.upgrade_pickups,
        write_rule,
    )?;
    write_map_diff(
        &mut b,
        gadgets,
        GADGET_BITS,
        &r.gadget_pickups,
        &base.gadget_pickups,
        write_rule,
    )?;
    b.flag(r.scrap_pickup != base.scrap_pickup);
    if r.scrap_pickup != base.scrap_pickup {
        b.flag(r.scrap_pickup.is_some());
        if let Some(s) = &r.scrap_pickup {
            write_scrap(&mut b, s)?;
        }
    }
    write_map_diff(
        &mut b,
        weapons,
        WEAPON_BITS,
        &r.shield_overrides,
        &base.shield_overrides,
        write_shield,
    )?;
    write_map_diff(
        &mut b,
        weapons,
        WEAPON_BITS,
        &r.tuning,
        &base.tuning,
        write_tuning,
    )?;
    Ok(b)
}

/// Appends the checksum and prints the symbols.
fn finish(mut b: Bits) -> String {
    let payload = b.bits.len();
    let crc = crc16(&b.bytes(payload));
    b.push(crc as u64, 16);
    while b.bits.len() % 5 != 0 {
        b.bits.push(false);
    }
    let mut out = String::from(PREFIX);
    for (i, chunk) in b.bits.chunks(5).enumerate() {
        if i > 0 && i % 4 == 2 {
            out.push('-');
        }
        let v = chunk.iter().fold(0usize, |v, &bit| (v << 1) | bit as usize);
        out.push(ALPHABET[v] as char);
    }
    out
}

/// The bits of a printed code (after the prefix).
fn parse_symbols(code: &str) -> Result<Vec<bool>, SimError> {
    let mut chars = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase());
    if chars.next() != Some('L') || chars.next() != Some('R') {
        return Err(err("a rule set code starts with LR"));
    }
    let mut bits = Vec::new();
    for c in chars {
        let c = match c {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        let v = ALPHABET
            .iter()
            .position(|&a| a as char == c)
            .ok_or_else(|| err(format!("invalid character `{c}`")))?;
        for i in (0..5).rev() {
            bits.push((v >> i) & 1 == 1);
        }
    }
    Ok(bits)
}

/// Whether `text` looks like a rule set code (prefix and alphabet only; the
/// checksum is not verified). Used to tell codes from file paths.
pub fn looks_like_code(text: &str) -> bool {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    t.len() > 4
        && t.get(..2).is_some_and(|p| p.eq_ignore_ascii_case(PREFIX))
        && t[2..].chars().all(|c| {
            let c = c.to_ascii_uppercase();
            ALPHABET.contains(&(c as u8)) || matches!(c, 'O' | 'I' | 'L')
        })
}

impl RuleSet {
    /// Encodes the rule set as a code (see the module docs): the diff
    /// against the mode's default rule set, or the full encoding when that
    /// is shorter. Fails for values the format cannot carry exactly.
    pub fn to_code(&self) -> Result<String, SimError> {
        self.to_code_with(&weapon_ids(), &gadget_ids())
    }

    /// The full (version 2) code: independent of the default rule set, and
    /// much longer.
    pub fn to_full_code(&self) -> Result<String, SimError> {
        Ok(finish(full_payload(self, &weapon_ids(), &gadget_ids())?))
    }

    /// [`RuleSet::to_code`] over explicit module lists (tests of codes from
    /// older builds).
    fn to_code_with(&self, weapons: &[&str], gadgets: &[&str]) -> Result<String, SimError> {
        let full = full_payload(self, weapons, gadgets)?;
        let diff = diff_payload(self, weapons, gadgets)?;
        Ok(finish(if diff.bits.len() <= full.bits.len() {
            diff
        } else {
            full
        }))
    }

    /// Decodes a rule set code of either format (see the module docs). The
    /// result is canonical and still has to be validated with
    /// [`RuleSet::resolve`].
    pub fn from_code(code: &str) -> Result<RuleSet, SimError> {
        Self::from_code_with(code, &weapon_ids(), &gadget_ids())
    }

    /// [`RuleSet::from_code`] over explicit module lists.
    fn from_code_with(
        code: &str,
        all_weapons: &[&str],
        all_gadgets: &[&str],
    ) -> Result<RuleSet, SimError> {
        let bits = parse_symbols(code)?;
        let mut r = Reader {
            bits: &bits,
            pos: 0,
        };
        let version = r.read(4)?;
        if version != FULL_VERSION as u64 && version != DIFF_VERSION as u64 {
            return Err(err(format!(
                "code version {version}, this build reads versions {FULL_VERSION} and {DIFF_VERSION}"
            )));
        }
        let (nw, ng) = (r.read(6)? as usize, r.read(4)? as usize);
        if nw > all_weapons.len() || ng > all_gadgets.len() {
            return Err(err(format!(
                "code from a newer build ({nw} weapons, {ng} gadgets; this build knows {} and {})",
                all_weapons.len(),
                all_gadgets.len()
            )));
        }
        // Weapons and gadgets the code does not cover stay banned, without rules.
        let (weapons, gadgets) = (&all_weapons[..nw], &all_gadgets[..ng]);
        let rules = if version == FULL_VERSION as u64 {
            read_full(&mut r, weapons, gadgets)?
        } else {
            read_diff(&mut r, weapons, gadgets)?
        };
        let payload = r.pos;
        let crc = r.read(16)? as u16;
        let rest = &bits[r.pos..];
        if rest.len() >= 5 || rest.iter().any(|&b| b) {
            return Err(err("trailing data after the checksum"));
        }
        if crc16(&pack(&bits[..payload])) != crc {
            return Err(err("checksum mismatch (typo?)"));
        }
        rules
    }
}

/// Reads a version 2 body. The outer result is a parse failure; the inner
/// one a value error reported only if the checksum matches (a typo is
/// reported as a typo).
fn read_full(
    r: &mut Reader,
    weapons: &[&str],
    gadgets: &[&str],
) -> Result<Result<RuleSet, SimError>, SimError> {
    let name = read_name(r)?;
    let mode = r.read(2)?;
    let m = read_mode(r)?;
    let friendly_fire_percent = r.read(7)? as u8;
    let absurdity = r.read(7)? as u8;
    let allowed_ship_types = ships_from_mask(r.read(4)?);
    let mutators = mutators_from_mask(r.read(4)?);
    let mut wmap = BTreeMap::new();
    for &w in weapons {
        let a = Availability::from_index(r.read(2)? as u8);
        if a != Availability::Banned {
            wmap.insert(w.to_string(), a);
        }
    }
    let mut gmap = BTreeMap::new();
    for &g in gadgets {
        let a = Availability::from_index(r.read(2)? as u8);
        if a != Availability::Banned {
            gmap.insert(g.to_string(), a);
        }
    }
    let starting_loadout = read_loadout(r, weapons, gadgets)?;
    let pickups = read_all(r, weapons, read_rule)?;
    let upgrade_pickups = read_all(r, weapons, read_rule)?;
    let gadget_pickups = read_all(r, gadgets, read_rule)?;
    let scrap_pickup = if r.flag()? {
        Some(read_scrap(r)?)
    } else {
        None
    };
    let shield_overrides = read_all(r, weapons, read_shield)?;
    let tuning = read_all(r, weapons, read_tuning)?;
    if mode != 0 {
        return Ok(Err(err("unknown mode")));
    }
    Ok(Ok(RuleSet {
        name,
        mode: ModeRules::PaintTheVoid(m),
        weapons: wmap,
        gadgets: gmap,
        starting_loadout,
        pickups,
        upgrade_pickups,
        gadget_pickups,
        scrap_pickup,
        shield_overrides,
        tuning,
        friendly_fire_percent,
        allowed_ship_types,
        absurdity,
        mutators,
    }))
}

/// Reads a version 3 body (see [`read_full`] for the nested result).
fn read_diff(
    r: &mut Reader,
    weapons: &[&str],
    gadgets: &[&str],
) -> Result<Result<RuleSet, SimError>, SimError> {
    let mode = r.read(2)?;
    let fp = r.read(16)? as u16;
    let mut out = match base_for(mode, weapons, gadgets) {
        Ok(b) => b,
        // Checked after the checksum, so a typo is reported as a typo.
        Err(e) => return Ok(Err(e)),
    };
    let base_fp = fingerprint(&out, weapons, gadgets);
    if r.flag()? {
        out.name = read_name(r)?;
    }
    if r.flag()? {
        let m = read_mode(r)?;
        out.mode = ModeRules::PaintTheVoid(m);
    }
    if r.flag()? {
        out.friendly_fire_percent = r.read(7)? as u8;
    }
    if r.flag()? {
        out.absurdity = r.read(7)? as u8;
    }
    if r.flag()? {
        out.allowed_ship_types = ships_from_mask(r.read(4)?);
    }
    let mut unknown_mutator = false;
    if r.flag()? {
        let mask = r.read(4)?;
        unknown_mutator = mask & !1 != 0;
        out.mutators = mutators_from_mask(mask);
    }
    read_availability_diff(r, weapons, WEAPON_BITS, &mut out.weapons)?;
    read_availability_diff(r, gadgets, GADGET_BITS, &mut out.gadgets)?;
    implied_by_availability(&mut out);
    if r.flag()? {
        let lo = read_loadout(r, weapons, gadgets)?;
        if lo == out.starting_loadout {
            return Err(err("corrupt code (an entry equals the default)"));
        }
        out.starting_loadout = lo;
    }
    read_map_diff(r, weapons, WEAPON_BITS, &mut out.pickups, read_rule)?;
    read_map_diff(r, weapons, WEAPON_BITS, &mut out.upgrade_pickups, read_rule)?;
    read_map_diff(r, gadgets, GADGET_BITS, &mut out.gadget_pickups, read_rule)?;
    if r.flag()? {
        let s = if r.flag()? {
            Some(read_scrap(r)?)
        } else {
            None
        };
        if s == out.scrap_pickup {
            return Err(err("corrupt code (an entry equals the default)"));
        }
        out.scrap_pickup = s;
    }
    read_map_diff(
        r,
        weapons,
        WEAPON_BITS,
        &mut out.shield_overrides,
        read_shield,
    )?;
    read_map_diff(r, weapons, WEAPON_BITS, &mut out.tuning, read_tuning)?;
    match base_fp {
        Ok(f) if f == fp => {}
        Ok(_) => {
            return Ok(Err(err(
                "code made against different default rules (another game version); \
                 ask for the full code",
            )))
        }
        Err(e) => return Ok(Err(e)),
    }
    if unknown_mutator {
        return Ok(Err(err("unknown mutator (code from a newer build?)")));
    }
    Ok(Ok(out))
}

#[cfg(test)]
mod tests;
