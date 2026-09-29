//! Rule set codes: a whole [`RuleSet`] as a short, typeable string, like
//! world share codes (design doc: "Rule sets save and share as their own
//! short code"). An invite link carries a world code and a rule set code.
//!
//! # Format (version 2)
//!
//! The printed code is `LR` followed by Crockford base32 symbols grouped in
//! fours (`LRab-cdef-...`). Decoding ignores case, dashes and whitespace and
//! reads `O` as `0` and `I`/`L` as `1` (after the `LR` prefix). The payload
//! bits, most significant bit first:
//!
//! | Bits | Field |
//! | --- | --- |
//! | 4 | format version (2) |
//! | 6, 4 | number of weapons and gadgets the code covers (the registry lengths of the build that made it) |
//! | 5 + 6 each | name: length (0..=31), characters from `" A-Za-z0-9-"` |
//! | 2, 3, 16, 8 | mode (0 = Paint the Void), teams, match seconds (whole), respawn (tenths of a second) |
//! | 7, 7 | friendly fire percent, absurdity |
//! | 4 | allowed ship types (bit = `ShipType::index`) |
//! | 4 | mutators (bit 0 = Instagib) |
//! | 2 per weapon | availability, weapons in registry order (`weapons::MODULES`) |
//! | 2 per gadget | availability, gadgets in registry order (`gadgets::MODULES`) |
//!
//! "Per weapon" and "per gadget" fields cover the counts from the header.
//! A code from an older build (fewer weapons: the registry is append-only)
//! decodes with the missing weapons banned and without rules; a code from a
//! newer build (more weapons than this build knows) is rejected with a clear
//! error instead of misparsing.
//!
//! | Bits | Field |
//! | --- | --- |
//! | 5 + 6 each | loadout primaries (registry index) |
//! | 4 + (6 + 8) each | loadout secondaries (registry index, count) |
//! | 3 | loadout gadget (0 = none, else registry index + 1) |
//! | per weapon: 1 [+ rule] | weapon pickups |
//! | per weapon: 1 [+ rule] | upgrade pickups |
//! | per gadget: 1 [+ rule] | gadget pickups |
//! | 1 [+ 8 + 10 + 8] | scrap pickup: weight (quarters), respawn (seconds), amount (whole) |
//! | per weapon: 1 [+ 2 + 8] | shield override: kind (Blocked, Wears, PassThrough), value (hundredths) |
//! | per weapon: 1 [+ 8 + 8 + 8] | tuning: damage, fire rate, ammo multipliers (twentieths) |
//! | 16 | CRC-16/CCITT-FALSE over the payload bits above (packed MSB first, zero padded) |
//!
//! A pickup rule is weight (8 bits, quarters, up to 63.75), respawn (10
//! bits, whole seconds), then 1 bit + 6 bits ammo if non-zero and 1 bit + 4
//! bits `max_on_map` if non-zero. The total is zero padded to whole symbols.
//!
//! Values that the format cannot carry exactly (a weight of 0.3, a respawn of
//! 2.5 s, an unknown weapon id, a name with other characters) make
//! [`RuleSet::to_code`] fail, so every code decodes to exactly the rule set
//! it was made from. The default Paint the Void rules take about 210 symbols
//! (270 characters with the prefix and dashes).

use crate::gadgets::MODULES as GADGETS;
use crate::ruleset::{
    Availability, Loadout, ModeRules, Mutator, PaintTheVoidRules, PickupRule, RuleSet,
    ScrapPickupRule, WeaponTuning,
};
use crate::weapons::MODULES as WEAPONS;
use crate::{ShieldInteraction, ShipType, SimError};

/// Prefix of every printed rule set code.
pub const PREFIX: &str = "LR";
/// Current format version.
pub const CODE_VERSION: u8 = 2;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const NAME_CHARS: &[u8; 64] = b" ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-";
const MAX_NAME: usize = 31;

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

    fn bytes(&self, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len.div_ceil(8)];
        for (i, &b) in self.bits[..len].iter().enumerate() {
            if b {
                out[i / 8] |= 0x80 >> (i % 8);
            }
        }
        out
    }
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

fn weapon_index(weapons: &[&str], id: &str) -> Result<u64, SimError> {
    weapons
        .iter()
        .position(|w| *w == id)
        .map(|i| i as u64)
        .ok_or_else(|| err(format!("weapon `{id}` has no rule set code index")))
}

fn gadget_index(gadgets: &[&str], id: &str) -> Result<u64, SimError> {
    gadgets
        .iter()
        .position(|g| *g == id)
        .map(|i| i as u64)
        .ok_or_else(|| err(format!("gadget `{id}` has no rule set code index")))
}

/// Weapon ids in registry order (rule set code indices).
fn weapon_ids() -> Vec<&'static str> {
    WEAPONS.iter().map(|m| m.id).collect()
}

/// Gadget ids in registry order.
fn gadget_ids() -> Vec<&'static str> {
    GADGETS.iter().map(|m| m.id).collect()
}

fn write_rule(b: &mut Bits, r: &PickupRule, what: &str) -> Result<(), SimError> {
    b.push(fixed(r.weight, 4.0, 255, what)?, 8);
    b.push(fixed(r.respawn_secs, 1.0, 1023, what)?, 10);
    if r.ammo > 63 || r.max_on_map > 15 {
        return Err(err(format!("{what}: ammo <= 63 and max_on_map <= 15")));
    }
    b.push((r.ammo != 0) as u64, 1);
    if r.ammo != 0 {
        b.push(r.ammo as u64, 6);
    }
    b.push((r.max_on_map != 0) as u64, 1);
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

impl RuleSet {
    /// Encodes the rule set as a code (see the module docs). Fails for
    /// values the format cannot carry exactly.
    pub fn to_code(&self) -> Result<String, SimError> {
        self.to_code_with(&weapon_ids(), &gadget_ids())
    }

    /// [`RuleSet::to_code`] over explicit module lists (tests of codes from
    /// older builds).
    fn to_code_with(&self, weapons: &[&str], gadgets: &[&str]) -> Result<String, SimError> {
        let mut b = Bits::default();
        b.push(CODE_VERSION as u64, 4);
        if weapons.len() > 63 || gadgets.len() > 15 {
            return Err(err("too many weapons or gadgets for the code format"));
        }
        b.push(weapons.len() as u64, 6);
        b.push(gadgets.len() as u64, 4);

        let name = self.name.as_bytes();
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

        let ModeRules::PaintTheVoid(m) = self.mode;
        b.push(0, 2);
        if m.teams > 7 {
            return Err(err("teams must be <= 7"));
        }
        b.push(m.teams as u64, 3);
        b.push(fixed(m.match_secs, 1.0, 65535, "match_secs")?, 16);
        b.push(fixed(m.respawn_secs, 10.0, 255, "respawn_secs")?, 8);
        if self.friendly_fire_percent > 100 || self.absurdity > 100 {
            return Err(err("friendly fire and absurdity must be <= 100"));
        }
        b.push(self.friendly_fire_percent as u64, 7);
        b.push(self.absurdity as u64, 7);
        let mut ships = 0u64;
        for t in &self.allowed_ship_types {
            ships |= 1 << t.index();
        }
        b.push(ships, 4);
        let mut mutators = 0u64;
        for m in &self.mutators {
            match m {
                Mutator::Instagib => mutators |= 1,
            }
        }
        b.push(mutators, 4);

        for id in self.weapons.keys() {
            weapon_index(weapons, id)?;
        }
        for id in self.gadgets.keys() {
            gadget_index(gadgets, id)?;
        }
        for &w in weapons {
            b.push(self.availability(w).index() as u64, 2);
        }
        for &g in gadgets {
            b.push(self.gadget_availability(g).index() as u64, 2);
        }

        let lo = &self.starting_loadout;
        if lo.primaries.len() > 31 || lo.secondaries.len() > 15 {
            return Err(err("loadout too large"));
        }
        b.push(lo.primaries.len() as u64, 5);
        for id in &lo.primaries {
            b.push(weapon_index(weapons, id)?, 6);
        }
        b.push(lo.secondaries.len() as u64, 4);
        for (id, n) in &lo.secondaries {
            b.push(weapon_index(weapons, id)?, 6);
            if *n > 255 {
                return Err(err("loadout counts must be <= 255"));
            }
            b.push(*n as u64, 8);
        }
        match &lo.gadget {
            None => b.push(0, 3),
            Some(id) => b.push(gadget_index(gadgets, id)? + 1, 3),
        }

        for id in self
            .pickups
            .keys()
            .chain(self.upgrade_pickups.keys())
            .chain(self.shield_overrides.keys())
            .chain(self.tuning.keys())
        {
            weapon_index(weapons, id)?;
        }
        for id in self.gadget_pickups.keys() {
            gadget_index(gadgets, id)?;
        }
        for &w in weapons {
            match self.pickups.get(w) {
                Some(r) => {
                    b.push(1, 1);
                    write_rule(&mut b, r, w)?;
                }
                None => b.push(0, 1),
            }
        }
        for &w in weapons {
            match self.upgrade_pickups.get(w) {
                Some(r) => {
                    b.push(1, 1);
                    write_rule(&mut b, r, w)?;
                }
                None => b.push(0, 1),
            }
        }
        for &g in gadgets {
            match self.gadget_pickups.get(g) {
                Some(r) => {
                    b.push(1, 1);
                    write_rule(&mut b, r, g)?;
                }
                None => b.push(0, 1),
            }
        }
        match self.scrap_pickup {
            None => b.push(0, 1),
            Some(s) => {
                b.push(1, 1);
                b.push(fixed(s.weight, 4.0, 255, "scrap weight")?, 8);
                b.push(fixed(s.respawn_secs, 1.0, 1023, "scrap respawn")?, 10);
                b.push(fixed(s.amount, 1.0, 255, "scrap amount")?, 8);
            }
        }
        for &w in weapons {
            match self.shield_overrides.get(w) {
                None => b.push(0, 1),
                Some(s) => {
                    b.push(1, 1);
                    let (kind, v) = match *s {
                        ShieldInteraction::Blocked => (0, 0),
                        ShieldInteraction::Wears(f) => (1, fixed(f, 100.0, 255, "shield wear")?),
                        ShieldInteraction::PassThrough(f) => {
                            (2, fixed(f, 100.0, 100, "shield pass-through")?)
                        }
                    };
                    b.push(kind, 2);
                    b.push(v, 8);
                }
            }
        }
        for &w in weapons {
            match self.tuning.get(w) {
                None => b.push(0, 1),
                Some(t) => {
                    b.push(1, 1);
                    b.push(fixed(t.damage, 20.0, 255, "damage multiplier")?, 8);
                    b.push(fixed(t.fire_rate, 20.0, 255, "fire rate multiplier")?, 8);
                    b.push(fixed(t.ammo, 20.0, 255, "ammo multiplier")?, 8);
                }
            }
        }

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
        Ok(out)
    }

    /// Decodes a rule set code (see the module docs). The result still has
    /// to be validated with [`RuleSet::resolve`].
    pub fn from_code(code: &str) -> Result<RuleSet, SimError> {
        Self::from_code_with(code, &weapon_ids(), &gadget_ids())
    }

    /// [`RuleSet::from_code`] over explicit module lists.
    fn from_code_with(
        code: &str,
        all_weapons: &[&str],
        all_gadgets: &[&str],
    ) -> Result<RuleSet, SimError> {
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
        let mut r = Reader {
            bits: &bits,
            pos: 0,
        };
        let version = r.read(4)?;
        if version != CODE_VERSION as u64 {
            return Err(err(format!(
                "code version {version}, this build reads version {CODE_VERSION}"
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
        let (weapons_in, gadgets_in) = (&all_weapons[..nw], &all_gadgets[..ng]);
        let len = r.read(5)? as usize;
        let mut name = String::with_capacity(len);
        for _ in 0..len {
            name.push(NAME_CHARS[r.read(6)? as usize] as char);
        }
        if r.read(2)? != 0 {
            return Err(err("unknown mode"));
        }
        let teams = r.read(3)? as u8;
        let match_secs = r.read(16)? as f32;
        let respawn_secs = r.read(8)? as f32 / 10.0;
        let friendly_fire_percent = r.read(7)? as u8;
        let absurdity = r.read(7)? as u8;
        let ships = r.read(4)?;
        let allowed_ship_types = ShipType::ALL
            .iter()
            .copied()
            .filter(|t| ships & (1 << t.index()) != 0)
            .collect();
        let mut mutators = Vec::new();
        if r.read(4)? & 1 != 0 {
            mutators.push(Mutator::Instagib);
        }
        let mut weapons = std::collections::BTreeMap::new();
        for &w in weapons_in {
            let a = Availability::from_index(r.read(2)? as u8);
            if a != Availability::Banned {
                weapons.insert(w.to_string(), a);
            }
        }
        let mut gadgets = std::collections::BTreeMap::new();
        for &g in gadgets_in {
            let a = Availability::from_index(r.read(2)? as u8);
            if a != Availability::Banned {
                gadgets.insert(g.to_string(), a);
            }
        }
        let weapon_id = |i: u64| -> Result<String, SimError> {
            weapons_in
                .get(i as usize)
                .map(|m| m.to_string())
                .ok_or_else(|| err("weapon index out of range"))
        };
        let mut primaries = Vec::new();
        for _ in 0..r.read(5)? {
            primaries.push(weapon_id(r.read(6)?)?);
        }
        let mut secondaries = Vec::new();
        for _ in 0..r.read(4)? {
            let id = weapon_id(r.read(6)?)?;
            secondaries.push((id, r.read(8)? as u32));
        }
        let gadget = match r.read(3)? {
            0 => None,
            i => Some(
                gadgets_in
                    .get(i as usize - 1)
                    .ok_or_else(|| err("gadget index out of range"))?
                    .to_string(),
            ),
        };
        let mut pickups = std::collections::BTreeMap::new();
        for &w in weapons_in {
            if r.flag()? {
                pickups.insert(w.to_string(), read_rule(&mut r)?);
            }
        }
        let mut upgrade_pickups = std::collections::BTreeMap::new();
        for &w in weapons_in {
            if r.flag()? {
                upgrade_pickups.insert(w.to_string(), read_rule(&mut r)?);
            }
        }
        let mut gadget_pickups = std::collections::BTreeMap::new();
        for &g in gadgets_in {
            if r.flag()? {
                gadget_pickups.insert(g.to_string(), read_rule(&mut r)?);
            }
        }
        let scrap_pickup = if r.flag()? {
            Some(ScrapPickupRule {
                weight: r.read(8)? as f32 / 4.0,
                respawn_secs: r.read(10)? as f32,
                amount: r.read(8)? as f32,
            })
        } else {
            None
        };
        let mut shield_overrides = std::collections::BTreeMap::new();
        for &w in weapons_in {
            if r.flag()? {
                let kind = r.read(2)?;
                let v = r.read(8)? as f32 / 100.0;
                let s = match kind {
                    0 => ShieldInteraction::Blocked,
                    1 => ShieldInteraction::Wears(v),
                    2 => ShieldInteraction::PassThrough(v),
                    _ => return Err(err("unknown shield interaction")),
                };
                shield_overrides.insert(w.to_string(), s);
            }
        }
        let mut tuning = std::collections::BTreeMap::new();
        for &w in weapons_in {
            if r.flag()? {
                tuning.insert(
                    w.to_string(),
                    WeaponTuning {
                        damage: r.read(8)? as f32 / 20.0,
                        fire_rate: r.read(8)? as f32 / 20.0,
                        ammo: r.read(8)? as f32 / 20.0,
                    },
                );
            }
        }
        let payload = r.pos;
        let crc = r.read(16)? as u16;
        let rest = &bits[r.pos..];
        if rest.len() >= 5 || rest.iter().any(|&b| b) {
            return Err(err("trailing data after the checksum"));
        }
        let packed = Bits {
            bits: bits[..payload].to_vec(),
        }
        .bytes(payload);
        if crc16(&packed) != crc {
            return Err(err("checksum mismatch (typo?)"));
        }
        Ok(RuleSet {
            name,
            mode: ModeRules::PaintTheVoid(PaintTheVoidRules {
                teams,
                match_secs,
                respawn_secs,
            }),
            weapons,
            gadgets,
            starting_loadout: Loadout {
                primaries,
                secondaries,
                gadget,
            },
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WeaponTable;
    use ls_math::Pcg32;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn codes_from_older_builds_decode_and_newer_ones_are_rejected() {
        let weapons = weapon_ids();
        let gadgets = gadget_ids();
        let r = RuleSet::paint_the_void();
        // An older build did not have the last weapon yet (the registry is
        // append-only): its codes say nothing about it.
        let last = *weapons.last().unwrap();
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
        let mut expect = old.clone();
        expect.weapons.retain(|_, a| *a != Availability::Banned);
        expect.gadgets.retain(|_, a| *a != Availability::Banned);
        assert_eq!(back, expect);
        // A newer build knows one weapon more: a clear error, not a checksum
        // mismatch.
        let mut newer = weapons.clone();
        newer.push("future_gun");
        let code = r.to_code_with(&newer, &gadgets).unwrap();
        let e = RuleSet::from_code(&code).unwrap_err().to_string();
        assert!(e.contains("newer build"), "{e}");
    }

    /// A rule set code decodes to exactly the rule set it was made from, and
    /// re-encodes to the same code.
    fn round_trip(r: &RuleSet) -> String {
        let code = r.to_code().unwrap();
        assert!(code.starts_with(PREFIX));
        let back = RuleSet::from_code(&code).unwrap();
        // Banned weapons and gadgets may be listed or left out; compare the rest.
        let mut expect = r.clone();
        expect.weapons.retain(|_, a| *a != Availability::Banned);
        expect.gadgets.retain(|_, a| *a != Availability::Banned);
        expect.allowed_ship_types.sort();
        expect.allowed_ship_types.dedup();
        assert_eq!(back, expect);
        assert_eq!(back.to_code().unwrap(), code);
        code
    }

    #[test]
    fn default_rules_round_trip() {
        let r = RuleSet::paint_the_void();
        let code = round_trip(&r);
        println!("{} chars: {code}", code.len());
        assert!(code.len() < 320, "{}", code.len());
        // Lenient decoding: case, dashes, whitespace, O/I/L.
        let messy: String = code
            .to_ascii_lowercase()
            .replace('-', " ")
            .replacen("lr", "L R", 1);
        assert_eq!(
            RuleSet::from_code(&messy).unwrap(),
            RuleSet::from_code(&code).unwrap()
        );
        RuleSet::from_code(&code)
            .unwrap()
            .resolve(&WeaponTable::builtin())
            .unwrap();
    }

    #[test]
    fn edited_rules_round_trip() {
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
        round_trip(&r);
        r.resolve(&WeaponTable::builtin()).unwrap();
    }

    #[test]
    fn unrepresentable_values_are_rejected() {
        let base = RuleSet::paint_the_void();
        let mut r = base.clone();
        r.pickups.get_mut("rivet_gun").unwrap().weight = 0.3;
        assert!(r.to_code().is_err());
        let mut r = base.clone();
        r.pickups.get_mut("rivet_gun").unwrap().respawn_secs = 2.5;
        assert!(r.to_code().is_err());
        let mut r = base.clone();
        r.name = "Ümlaut".into();
        assert!(r.to_code().is_err());
        let mut r = base.clone();
        r.weapons
            .insert("homebrew".into(), Availability::PickupOnly);
        assert!(r.to_code().is_err());
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

    #[test]
    fn typos_and_garbage_are_rejected() {
        let code = RuleSet::paint_the_void().to_code().unwrap();
        assert!(RuleSet::from_code("LS12-3456").is_err());
        assert!(RuleSet::from_code("").is_err());
        assert!(RuleSet::from_code(&code[..code.len() - 6]).is_err());
        assert!(RuleSet::from_code(&format!("{code}U")).is_err());
        // Every single-symbol change is caught (CRC-16 or a field check).
        let body: Vec<char> = code.chars().collect();
        let mut rng = Pcg32::new(5, 5);
        let mut tested = 0;
        for i in 2..body.len() {
            if body[i] == '-' {
                continue;
            }
            let mut c = body.clone();
            let orig = c[i];
            let mut repl = orig;
            while repl == orig || repl == 'O' && orig == '0' {
                repl = ALPHABET[rng.below(32) as usize] as char;
            }
            c[i] = repl;
            let s: String = c.into_iter().collect();
            if let Ok(r) = RuleSet::from_code(&s) {
                panic!("typo at {i} accepted: {}", r.name);
            }
            tested += 1;
        }
        assert!(tested > 100);
    }
}
