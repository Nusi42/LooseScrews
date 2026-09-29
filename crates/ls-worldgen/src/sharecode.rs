//! Share codes: a seed, the seven sliders and the generator version in one
//! short, typeable string.
//!
//! # Format
//!
//! 130 payload bits, most significant bit first:
//!
//! | Bits | Field |
//! | --- | --- |
//! | 8 | generator version |
//! | 64 | seed |
//! | 7 × 7 | sliders in [`Sliders::to_array`] order (each `0..=100`) |
//! | 8 | CRC-8 (poly `0x07`, init 0) over the 121 bits above, packed MSB first |
//! | 1 | padding, always 0 |
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

use serde::{Deserialize, Serialize};

use crate::{Sliders, WorldgenError, GENERATOR_VERSION};

/// Crockford base32 alphabet (no I, L, O, U).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Number of base32 symbols in a code (excluding prefix and dashes).
pub const CODE_SYMBOLS: usize = 26;
/// Prefix of every printed share code.
pub const PREFIX: &str = "LS";

const DATA_BITS: usize = 8 + 64 + 7 * 7; // 121
const TOTAL_BITS: usize = CODE_SYMBOLS * 5; // 130
const BUF_BYTES: usize = TOTAL_BITS.div_ceil(8); // 17

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
}

impl ShareCode {
    /// A code for the current [`GENERATOR_VERSION`].
    pub fn new(seed: u64, sliders: Sliders) -> Self {
        ShareCode {
            seed,
            sliders,
            version: GENERATOR_VERSION,
        }
    }

    /// Encodes to the printed form, e.g. `LS1A-2B3C-...`. Slider values above
    /// 100 are rejected.
    pub fn encode(&self) -> Result<String, WorldgenError> {
        self.sliders.validate()?;
        let mut bits = BitBuf::default();
        bits.push(self.version as u64, 8);
        bits.push(self.seed, 64);
        for v in self.sliders.to_array() {
            bits.push(v as u64, 7);
        }
        debug_assert_eq!(bits.len, DATA_BITS);
        let crc = crc8(&bits.bytes[..DATA_BITS.div_ceil(8)]);
        bits.push(crc as u64, 8);
        bits.push(0, 1);
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
        if bits.read(TOTAL_BITS - 1, 1) != 0 {
            return Err(WorldgenError::ChecksumMismatch);
        }
        // Recompute the CRC over the data bits only (clear the rest first).
        let stored_crc = bits.read(DATA_BITS, 8) as u8;
        let mut data = BitBuf::default();
        data.push(bits.read(0, 8), 8);
        data.push(bits.read(8, 64), 64);
        data.push(bits.read(72, 49), 49);
        if crc8(&data.bytes[..DATA_BITS.div_ceil(8)]) != stored_crc {
            return Err(WorldgenError::ChecksumMismatch);
        }
        let version = bits.read(0, 8) as u8;
        let seed = bits.read(8, 64);
        let mut sl = [0u8; 7];
        for (i, v) in sl.iter_mut().enumerate() {
            *v = bits.read(72 + i * 7, 7) as u8;
        }
        let sliders = Sliders::from_array(sl)?;
        Ok(ShareCode {
            seed,
            sliders,
            version,
        })
    }
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

    fn sample_codes() -> Vec<ShareCode> {
        let mut v = vec![
            ShareCode::new(0, Sliders::default()),
            ShareCode::new(u64::MAX, Sliders::from_array([100; 7]).unwrap()),
            ShareCode::new(0x0123_4567_89ab_cdef, Sliders::from_array([0; 7]).unwrap()),
        ];
        let mut rng = ls_math::SplitMix64::new(7);
        for _ in 0..200 {
            let mut s = [0u8; 7];
            for x in &mut s {
                *x = (rng.next_u32() % 101) as u8;
            }
            v.push(ShareCode {
                seed: rng.next_u64(),
                sliders: Sliders::from_array(s).unwrap(),
                version: (rng.next_u32() & 0xff) as u8,
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
        for code in sample_codes().into_iter().take(20) {
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
                    assert!(
                        ShareCode::decode(&s).is_err(),
                        "typo not detected: {text} -> {s}"
                    );
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

    #[test]
    fn out_of_range_slider_with_valid_crc_is_rejected() {
        // Hand-build a payload with density = 127 and a correct CRC.
        let mut bits = BitBuf::default();
        bits.push(GENERATOR_VERSION as u64, 8);
        bits.push(5, 64);
        for v in [50u64, 127, 0, 0, 0, 0, 0] {
            bits.push(v, 7);
        }
        let crc = crc8(&bits.bytes[..DATA_BITS.div_ceil(8)]);
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
