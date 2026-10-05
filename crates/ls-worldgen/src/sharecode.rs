//! Share codes: a seed, the seven sliders, the level archetype and the
//! generator version in one short, typeable string.
//!
//! # Format (generator version 4)
//!
//! 130 payload bits, most significant bit first:
//!
//! | Bits | Field |
//! | --- | --- |
//! | 8 | generator version |
//! | 3 | level archetype ([`Archetype::index`]) |
//! | 64 | seed |
//! | 47 | the seven sliders as one base-101 number, [`Sliders::to_array`] order, first slider most significant |
//! | 8 | CRC-8 (poly `0x07`, init 0) over the 122 bits above, packed MSB first |
//!
//! Versions 1 to 3 stored the sliders as 7 x 7 bits followed by the CRC and
//! one padding bit, and had no archetype. Such codes still decode (with
//! [`Archetype::Classic`]) so that [`crate::Generator::from_code`] can
//! report the version mismatch; the version field (the first 8 bits in
//! every layout) picks the layout.
//!
//! The 130 bits become exactly 26 Crockford base32 symbols. The printed form
//! puts `LS` in front and groups the result in fours:
//! `LSab-cdef-ghij-klmn-opqr-stuv-wxyz` (26 code symbols, 34 characters with
//! the prefix and dashes).
//!
//! Decoding is lenient in the Crockford way: case-insensitive, dashes and
//! whitespace ignored, `O` reads as `0` and `I`/`L` read as `1`. The CRC-8
//! detects every single-symbol typo (a symbol spans 5 bits, a burst the CRC
//! always catches) and most transpositions.

use std::fmt;
use std::str::FromStr;

use ls_math::level::Archetype;
use serde::{Deserialize, Serialize};

use crate::{Sliders, WorldgenError, GENERATOR_VERSION};

/// Crockford base32 alphabet (no I, L, O, U).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Number of base32 symbols in a code (excluding prefix and dashes).
pub const CODE_SYMBOLS: usize = 26;
/// Prefix of every printed share code.
pub const PREFIX: &str = "LS";
/// First generator version whose codes carry the archetype field.
pub const ARCHETYPE_VERSION: u8 = 4;

/// Data bits of a version 4 code: version, archetype, seed, sliders.
const DATA_BITS: usize = 8 + 3 + 64 + 47; // 122
/// Data bits of a version 1-3 code: version, seed, 7 x 7 slider bits.
const LEGACY_DATA_BITS: usize = 8 + 64 + 7 * 7; // 121
const TOTAL_BITS: usize = CODE_SYMBOLS * 5; // 130
const BUF_BYTES: usize = TOTAL_BITS.div_ceil(8); // 17
/// Number of distinct slider values (0..=100).
const SLIDER_BASE: u64 = 101;
/// `101^7`: the packed sliders are below this (and below `2^47`).
const SLIDER_STATES: u64 = 101u64.pow(7);

/// A decoded share code.
///
/// Deserializing (serde) validates the sliders but not `version`: callers that
/// build a world from deserialized input must go through
/// [`crate::Generator::from_code`], which rejects unsupported versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ShareCode {
    pub seed: u64,
    pub sliders: Sliders,
    /// Generator version the code was made with.
    pub version: u8,
    /// Kind of level (classic or asteroid belt); classic when absent from
    /// serialized input.
    #[serde(default)]
    pub archetype: Archetype,
}

impl ShareCode {
    /// A classic-level code for the current [`GENERATOR_VERSION`].
    pub fn new(seed: u64, sliders: Sliders) -> Self {
        Self::with_archetype(seed, sliders, Archetype::Classic)
    }

    /// A code for the current [`GENERATOR_VERSION`] with the given archetype.
    pub fn with_archetype(seed: u64, sliders: Sliders, archetype: Archetype) -> Self {
        ShareCode {
            seed,
            sliders,
            version: GENERATOR_VERSION,
            archetype,
        }
    }

    /// The same seed, sliders and version with another archetype (the lobby
    /// plays a Belt Up entry paired with a classic code as a belt).
    pub fn as_archetype(self, archetype: Archetype) -> Self {
        ShareCode { archetype, ..self }
    }

    /// Encodes to the printed form, e.g. `LS1A-2B3C-...`. Slider values above
    /// 100 are rejected. Codes of versions before [`ARCHETYPE_VERSION`] use
    /// the old layout (classic only).
    pub fn encode(&self) -> Result<String, WorldgenError> {
        self.sliders.validate()?;
        let mut bits = BitBuf::default();
        if self.version < ARCHETYPE_VERSION {
            if self.archetype != Archetype::Classic {
                return Err(WorldgenError::UnknownArchetype(self.archetype.index()));
            }
            bits.push(self.version as u64, 8);
            bits.push(self.seed, 64);
            for v in self.sliders.to_array() {
                bits.push(v as u64, 7);
            }
            debug_assert_eq!(bits.len, LEGACY_DATA_BITS);
            let crc = crc8(&bits.bytes[..LEGACY_DATA_BITS.div_ceil(8)]);
            bits.push(crc as u64, 8);
            bits.push(0, 1);
        } else {
            bits.push(self.version as u64, 8);
            bits.push(self.archetype.index() as u64, 3);
            bits.push(self.seed, 64);
            bits.push(pack_sliders(&self.sliders), 47);
            debug_assert_eq!(bits.len, DATA_BITS);
            let crc = crc8(&bits.bytes[..DATA_BITS.div_ceil(8)]);
            bits.push(crc as u64, 8);
        }
        debug_assert_eq!(bits.len, TOTAL_BITS);

        let mut symbols = [0u8; CODE_SYMBOLS];
        for (i, s) in symbols.iter_mut().enumerate() {
            *s = ALPHABET[bits.read(i * 5, 5) as usize];
        }
        // "LS" + 2 symbols form the first group of four, then groups of four.
        let mut out = String::with_capacity(PREFIX.len() + CODE_SYMBOLS + 6);
        out.push_str(PREFIX);
        for (i, &s) in symbols.iter().enumerate() {
            if i >= 2 && (i - 2) % 4 == 0 {
                out.push('-');
            }
            out.push(s as char);
        }
        Ok(out)
    }

    /// Parses a printed share code. See the module docs for the accepted syntax.
    pub fn decode(text: &str) -> Result<Self, WorldgenError> {
        let cleaned: Vec<u8> = text
            .bytes()
            .filter(|b| *b != b'-' && !b.is_ascii_whitespace())
            .map(|b| b.to_ascii_uppercase())
            .collect();
        if !cleaned.starts_with(PREFIX.as_bytes()) {
            return Err(WorldgenError::MissingPrefix);
        }
        let body = &cleaned[PREFIX.len()..];
        if body.len() != CODE_SYMBOLS {
            return Err(WorldgenError::InvalidLength {
                expected: CODE_SYMBOLS,
                found: body.len(),
            });
        }
        let mut bits = BitBuf::default();
        for &b in body {
            bits.push(symbol_value(b)? as u64, 5);
        }
        let version = bits.read(0, 8) as u8;
        if version < ARCHETYPE_VERSION {
            return decode_legacy(&bits);
        }
        // Recompute the CRC over the data bits only (clear the rest first).
        let stored_crc = bits.read(DATA_BITS, 8) as u8;
        let mut data = BitBuf::default();
        data.push(bits.read(0, 11), 11);
        data.push(bits.read(11, 64), 64);
        data.push(bits.read(75, 47), 47);
        if crc8(&data.bytes[..DATA_BITS.div_ceil(8)]) != stored_crc {
            return Err(WorldgenError::ChecksumMismatch);
        }
        let index = bits.read(8, 3) as u8;
        let archetype =
            Archetype::from_index(index).ok_or(WorldgenError::UnknownArchetype(index))?;
        let seed = bits.read(11, 64);
        let sliders = unpack_sliders(bits.read(75, 47))?;
        Ok(ShareCode {
            seed,
            sliders,
            version,
            archetype,
        })
    }
}

/// Decodes the payload of a version 1-3 code (no archetype, 7 x 7 slider
/// bits, CRC, padding bit).
fn decode_legacy(bits: &BitBuf) -> Result<ShareCode, WorldgenError> {
    if bits.read(TOTAL_BITS - 1, 1) != 0 {
        return Err(WorldgenError::ChecksumMismatch);
    }
    let stored_crc = bits.read(LEGACY_DATA_BITS, 8) as u8;
    let mut data = BitBuf::default();
    data.push(bits.read(0, 8), 8);
    data.push(bits.read(8, 64), 64);
    data.push(bits.read(72, 49), 49);
    if crc8(&data.bytes[..LEGACY_DATA_BITS.div_ceil(8)]) != stored_crc {
        return Err(WorldgenError::ChecksumMismatch);
    }
    let mut sl = [0u8; 7];
    for (i, v) in sl.iter_mut().enumerate() {
        *v = bits.read(72 + i * 7, 7) as u8;
    }
    Ok(ShareCode {
        seed: bits.read(8, 64),
        sliders: Sliders::from_array(sl)?,
        version: bits.read(0, 8) as u8,
        archetype: Archetype::Classic,
    })
}

/// The sliders as one base-101 number (first slider most significant).
fn pack_sliders(s: &Sliders) -> u64 {
    s.to_array()
        .iter()
        .fold(0u64, |acc, &v| acc * SLIDER_BASE + v as u64)
}

/// Inverse of [`pack_sliders`]. A number of `101^7` or more is no slider
/// set (a corrupted code that passed the CRC).
fn unpack_sliders(mut packed: u64) -> Result<Sliders, WorldgenError> {
    if packed >= SLIDER_STATES {
        return Err(WorldgenError::InvalidSliders);
    }
    let mut a = [0u8; 7];
    for v in a.iter_mut().rev() {
        *v = (packed % SLIDER_BASE) as u8;
        packed /= SLIDER_BASE;
    }
    Sliders::from_array(a)
}

impl fmt::Display for ShareCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.encode() {
            Ok(s) => f.write_str(&s),
            Err(_) => f.write_str("LS-INVALID"),
        }
    }
}

impl FromStr for ShareCode {
    type Err = WorldgenError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ShareCode::decode(s)
    }
}

fn symbol_value(b: u8) -> Result<u8, WorldgenError> {
    let v = match b {
        b'0'..=b'9' => b - b'0',
        b'O' => 0,
        b'I' | b'L' => 1,
        _ => match ALPHABET.iter().position(|&a| a == b) {
            Some(i) => i as u8,
            None => return Err(WorldgenError::InvalidCharacter(b as char)),
        },
    };
    Ok(v)
}

/// CRC-8 with polynomial `x^8 + x^2 + x + 1` (0x07), init 0, no reflection.
fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Fixed-size MSB-first bit buffer.
#[derive(Default)]
struct BitBuf {
    bytes: [u8; BUF_BYTES],
    len: usize,
}

impl BitBuf {
    /// Appends the low `n` bits of `v`, most significant first.
    fn push(&mut self, v: u64, n: usize) {
        for i in (0..n).rev() {
            let bit = (v >> i) & 1;
            if bit != 0 {
                self.bytes[self.len / 8] |= 0x80 >> (self.len % 8);
            }
            self.len += 1;
        }
    }

    /// Reads `n <= 64` bits starting at bit `at`.
    fn read(&self, at: usize, n: usize) -> u64 {
        let mut v = 0u64;
        for i in at..at + n {
            let bit = (self.bytes[i / 8] >> (7 - i % 8)) & 1;
            v = (v << 1) | bit as u64;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// A code in the version 4 layout (the archetype field).
    fn v4(seed: u64, sliders: Sliders, archetype: Archetype) -> ShareCode {
        ShareCode {
            seed,
            sliders,
            version: GENERATOR_VERSION.max(ARCHETYPE_VERSION),
            archetype,
        }
    }

    fn sample_codes() -> Vec<ShareCode> {
        let mut v = vec![
            ShareCode::new(0, Sliders::default()),
            ShareCode::new(u64::MAX, Sliders::from_array([100; 7]).unwrap()),
            ShareCode::new(0x0123_4567_89ab_cdef, Sliders::from_array([0; 7]).unwrap()),
            v4(
                u64::MAX,
                Sliders::from_array([100; 7]).unwrap(),
                Archetype::Belt,
            ),
        ];
        let mut rng = ls_math::SplitMix64::new(7);
        for _ in 0..200 {
            let mut s = [0u8; 7];
            for x in &mut s {
                *x = (rng.next_u32() % 101) as u8;
            }
            let version = (rng.next_u32() & 0xff) as u8;
            let archetype = if version < ARCHETYPE_VERSION {
                Archetype::Classic
            } else {
                Archetype::ALL[(rng.next_u32() as usize) % Archetype::ALL.len()]
            };
            v.push(ShareCode {
                seed: rng.next_u64(),
                sliders: Sliders::from_array(s).unwrap(),
                version,
                archetype,
            });
        }
        v
    }

    #[test]
    fn round_trip() {
        for code in sample_codes() {
            let text = code.encode().unwrap();
            let symbols = text.chars().filter(|c| *c != '-').count() - PREFIX.len();
            assert_eq!(symbols, CODE_SYMBOLS, "{text}");
            assert!(text.starts_with("LS"));
            assert_eq!(ShareCode::decode(&text).unwrap(), code, "{text}");
            assert_eq!(text.parse::<ShareCode>().unwrap(), code);
            assert_eq!(code.to_string(), text);
        }
    }

    /// Property test (M4 plan, A1): version 4 codes stay 26 symbols and
    /// round-trip every archetype and every value of every slider.
    #[test]
    fn version_4_codes_round_trip_every_archetype_and_slider_value() {
        let mut rng = ls_math::SplitMix64::new(44);
        for archetype in Archetype::ALL {
            for slider in 0..7 {
                for value in 0..=100u8 {
                    let mut s = [0u8; 7];
                    for x in &mut s {
                        *x = (rng.next_u32() % 101) as u8;
                    }
                    s[slider] = value;
                    let code = v4(rng.next_u64(), Sliders::from_array(s).unwrap(), archetype);
                    let text = code.encode().unwrap();
                    assert_eq!(text.len(), 34, "{text}");
                    assert_eq!(ShareCode::decode(&text).unwrap(), code, "{text}");
                }
            }
        }
        // The extremes of the packed slider number.
        for s in [[0u8; 7], [100u8; 7]] {
            let code = v4(1, Sliders::from_array(s).unwrap(), Archetype::Classic);
            assert_eq!(ShareCode::decode(&code.encode().unwrap()).unwrap(), code);
        }
        const { assert!(SLIDER_STATES < 1u64 << 47) };
    }

    #[test]
    fn version_3_codes_still_decode_as_classic() {
        // A version 3 code from the M3 docs: decodes (so the generator can
        // name the version), classic, with its sliders.
        let old = ShareCode::decode("LS0C-0000-0000-0000-8006-8001-8F8").err();
        assert!(old.is_some(), "too short");
        let v3 = ShareCode {
            seed: 42,
            sliders: Sliders::new(50, 40),
            version: 3,
            archetype: Archetype::Classic,
        };
        let text = v3.encode().unwrap();
        assert_eq!(ShareCode::decode(&text).unwrap(), v3);
        // A belt cannot be written in the old layout.
        assert_eq!(
            v3.as_archetype(Archetype::Belt).encode(),
            Err(WorldgenError::UnknownArchetype(1))
        );
    }

    #[test]
    fn archetype_changes_the_code_not_its_length() {
        let classic = v4(9, Sliders::default(), Archetype::Classic);
        let belt = v4(9, Sliders::default(), Archetype::Belt);
        let (a, b) = (classic.encode().unwrap(), belt.encode().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), b.len());
        assert_eq!(ShareCode::decode(&b).unwrap().archetype, Archetype::Belt);
    }

    #[test]
    fn printed_shape() {
        let text = ShareCode::new(42, Sliders::default()).encode().unwrap();
        let groups: Vec<&str> = text.split('-').collect();
        assert_eq!(groups.len(), 7, "{text}");
        assert!(groups.iter().all(|g| g.len() == 4), "{text}");
        assert!(text.len() == 34);
    }

    #[test]
    fn lenient_parsing() {
        let code = ShareCode::new(0xdead_beef_0000_1111, Sliders::new(10, 90));
        let text = code.encode().unwrap();
        let lower = text.to_ascii_lowercase();
        assert_eq!(ShareCode::decode(&lower).unwrap(), code);
        let spaced = text.replace('-', " ");
        assert_eq!(ShareCode::decode(&spaced).unwrap(), code);
        let no_dashes = text.replace('-', "");
        assert_eq!(ShareCode::decode(&no_dashes).unwrap(), code);
        // Aliases: 0 <-> O and 1 <-> I/L inside the body.
        let body_aliased: String = text[2..]
            .chars()
            .map(|c| match c {
                '0' => 'o',
                '1' => 'I',
                c => c,
            })
            .collect();
        assert_eq!(
            ShareCode::decode(&format!("LS{body_aliased}")).unwrap(),
            code
        );
    }

    #[test]
    fn every_single_symbol_typo_is_detected() {
        let current = sample_codes()
            .into_iter()
            .filter(|c| c.version == ARCHETYPE_VERSION)
            .chain((0..12).map(|i| {
                v4(
                    0x9e37_79b9_7f4a_7c15u64.wrapping_mul(i + 1),
                    Sliders::default(),
                    Archetype::ALL[i as usize % Archetype::ALL.len()],
                )
            }));
        for code in current.take(20) {
            let text = code.encode().unwrap().replace('-', "");
            let bytes = text.as_bytes();
            for pos in PREFIX.len()..bytes.len() {
                for &alt in ALPHABET.iter() {
                    if alt == bytes[pos] {
                        continue;
                    }
                    let mut corrupted = bytes.to_vec();
                    corrupted[pos] = alt;
                    let s = String::from_utf8(corrupted).unwrap();
                    // A typo in the version symbols can switch to the
                    // old layout; such a code names an old version, which
                    // `Generator::from_code` refuses.
                    match ShareCode::decode(&s) {
                        Err(_) => {}
                        Ok(other) if other.version < ARCHETYPE_VERSION => {}
                        Ok(other) => panic!("typo not detected: {text} -> {s} ({other:?})"),
                    }
                }
            }
        }
    }

    #[test]
    fn most_transpositions_are_detected() {
        let mut detected = 0;
        let mut total = 0;
        for code in sample_codes().into_iter().take(40) {
            let text = code.encode().unwrap().replace('-', "");
            let bytes = text.as_bytes();
            for pos in PREFIX.len()..bytes.len() - 1 {
                if bytes[pos] == bytes[pos + 1] {
                    continue;
                }
                let mut c = bytes.to_vec();
                c.swap(pos, pos + 1);
                total += 1;
                if ShareCode::decode(std::str::from_utf8(&c).unwrap()).is_err() {
                    detected += 1;
                }
            }
        }
        // Adjacent swaps are a 10-bit burst; an 8-bit CRC misses ~1/256 of them.
        assert!(detected * 100 >= total * 97, "{detected}/{total}");
    }

    #[test]
    fn malformed_inputs() {
        let text = ShareCode::new(1, Sliders::default()).encode().unwrap();
        assert_eq!(
            ShareCode::decode(&text[2..]),
            Err(WorldgenError::MissingPrefix)
        );
        assert!(matches!(
            ShareCode::decode(&text[..text.len() - 1]),
            Err(WorldgenError::InvalidLength { .. })
        ));
        let mut bad = text.clone();
        bad.push('Q');
        assert!(matches!(
            ShareCode::decode(&bad),
            Err(WorldgenError::InvalidLength { .. })
        ));
        let with_u = format!("{}U", &text[..text.len() - 1]);
        assert_eq!(
            ShareCode::decode(&with_u),
            Err(WorldgenError::InvalidCharacter('U'))
        );
        assert!(ShareCode::decode("").is_err());
        // Out-of-range slider is rejected at encode time.
        let s = Sliders {
            density: 101,
            ..Sliders::default()
        };
        assert!(ShareCode::new(1, s).encode().is_err());
    }

    /// Hand-built version 4 payload with a correct CRC.
    fn v4_payload(archetype: u64, packed: u64) -> String {
        let mut bits = BitBuf::default();
        bits.push(ARCHETYPE_VERSION as u64, 8);
        bits.push(archetype, 3);
        bits.push(5, 64);
        bits.push(packed, 47);
        let crc = crc8(&bits.bytes[..DATA_BITS.div_ceil(8)]);
        bits.push(crc as u64, 8);
        let body: String = (0..CODE_SYMBOLS)
            .map(|i| ALPHABET[bits.read(i * 5, 5) as usize] as char)
            .collect();
        format!("LS{body}")
    }

    #[test]
    fn invalid_fields_with_valid_crc_are_rejected() {
        assert!(ShareCode::decode(&v4_payload(1, 0)).is_ok());
        assert_eq!(
            ShareCode::decode(&v4_payload(0, SLIDER_STATES)),
            Err(WorldgenError::InvalidSliders)
        );
        assert_eq!(
            ShareCode::decode(&v4_payload(7, 0)),
            Err(WorldgenError::UnknownArchetype(7))
        );
        // Version 3 layout with density = 127.
        let mut bits = BitBuf::default();
        bits.push(3, 8);
        bits.push(5, 64);
        for v in [50u64, 127, 0, 0, 0, 0, 0] {
            bits.push(v, 7);
        }
        let crc = crc8(&bits.bytes[..LEGACY_DATA_BITS.div_ceil(8)]);
        bits.push(crc as u64, 8);
        bits.push(0, 1);
        let body: String = (0..CODE_SYMBOLS)
            .map(|i| ALPHABET[bits.read(i * 5, 5) as usize] as char)
            .collect();
        assert!(matches!(
            ShareCode::decode(&format!("LS{body}")),
            Err(WorldgenError::SliderOutOfRange { value: 127, .. })
        ));
    }

    #[test]
    fn crc8_reference() {
        // CRC-8/SMBUS (poly 0x07, init 0) check value for "123456789".
        assert_eq!(crc8(b"123456789"), 0xF4);
    }
}
