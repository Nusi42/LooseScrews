//! Share-code helpers for the start screen: full decode and checksum
//! validation through `ls_worldgen::ShareCode` (the web client's own check
//! only looks at the format).

use ls_math::rng::SplitMix64;
use ls_worldgen::{Archetype, ShareCode, Sliders};

/// Result of [`check_share_code`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeCheck {
    /// The code decodes, has a valid checksum and this build can generate it.
    Valid {
        /// Printed form (`LSab-cdef-…`).
        printed: String,
        seed: u64,
        /// Sliders in share-code order (enclosure, density, verticality,
        /// gravity chaos, palette, absurdity, instability).
        sliders: [u8; 7],
        version: u8,
        /// Level archetype index (0 classic, 1 asteroid belt).
        archetype: u8,
    },
    /// Human-readable reason.
    Invalid(String),
}

/// Decodes and validates a share code typed by a player (case, dashes and
/// spaces ignored, Crockford look-alikes accepted).
pub fn check_share_code(text: &str) -> CodeCheck {
    let code = match ShareCode::decode(text) {
        Ok(c) => c,
        Err(e) => return CodeCheck::Invalid(capitalize(&e.to_string())),
    };
    // Decoding accepts any version so old codes can be recognised; generating
    // one needs a matching generator.
    if let Err(e) = ls_worldgen::Generator::from_code(&code) {
        return CodeCheck::Invalid(capitalize(&e.to_string()));
    }
    match code.encode() {
        Ok(printed) => CodeCheck::Valid {
            printed,
            seed: code.seed,
            sliders: code.sliders.to_array(),
            version: code.version,
            archetype: code.archetype.index(),
        },
        Err(e) => CodeCheck::Invalid(capitalize(&e.to_string())),
    }
}

/// A fresh share code for practice: the seed is derived from `entropy` (the
/// caller supplies randomness; this crate never touches OS RNG), the two
/// active sliders are given, the other five stay at their defaults.
pub fn random_share_code(entropy: u64, enclosure: u8, density: u8) -> Result<String, String> {
    let seed = SplitMix64::new(entropy).next_u64();
    let sliders = Sliders {
        enclosure: enclosure.min(100),
        density: density.min(100),
        ..Sliders::default()
    };
    ShareCode::new(seed, sliders)
        .encode()
        .map_err(|e| e.to_string())
}

/// [`random_share_code`]'s seed for `entropy` with all seven sliders
/// (clamped to 0..=100) and an archetype index.
pub fn world_share_code(entropy: u64, sliders: &[u8], archetype: u8) -> Result<String, String> {
    let seed = SplitMix64::new(entropy).next_u64();
    make_share_code(&seed.to_string(), sliders, archetype)
}

/// The share code for a seed (decimal), seven sliders (clamped to 0..=100)
/// and an archetype index, at the current generator version.
pub fn make_share_code(seed: &str, sliders: &[u8], archetype: u8) -> Result<String, String> {
    let seed: u64 = seed
        .trim()
        .parse()
        .map_err(|_| format!("seed {seed:?} is not a whole number"))?;
    if sliders.len() != 7 {
        return Err(format!("need 7 sliders, got {}", sliders.len()));
    }
    let mut a = [0u8; 7];
    for (v, s) in a.iter_mut().zip(sliders) {
        *v = (*s).min(100);
    }
    let archetype =
        Archetype::from_index(archetype).ok_or(format!("unknown level archetype {archetype}"))?;
    let sliders = Sliders::from_array(a).map_err(|e| e.to_string())?;
    ShareCode::with_archetype(seed, sliders, archetype)
        .encode()
        .map_err(|e| e.to_string())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// From `crates/ls-worldgen/tests/fixtures/golden_hashes.txt`.
    const REAL: &str = "LS0G-0000-0000-0001-AC58-T2XM-57ND";

    #[test]
    fn valid_codes_decode_in_any_spelling() {
        let CodeCheck::Valid {
            printed,
            seed,
            sliders,
            version,
            archetype,
        } = check_share_code(" ls0g oooo 0000-OOOl ac58 t2xm 57nd ")
        else {
            panic!("expected a valid code");
        };
        assert_eq!(printed, REAL);
        assert_eq!(seed, 42);
        assert_eq!(sliders[0], 50);
        assert_eq!(sliders[1], 40);
        assert_eq!(version, ls_worldgen::GENERATOR_VERSION);
        assert_eq!(archetype, 0);
    }

    #[test]
    fn corrupted_codes_are_rejected_with_a_reason() {
        // One symbol changed: format fine, checksum wrong.
        let typo = REAL.replace("AC58", "AC59");
        match check_share_code(&typo) {
            CodeCheck::Invalid(msg) => assert!(msg.contains("checksum"), "{msg}"),
            other => panic!("typo accepted: {other:?}"),
        }
        for bad in [
            "",
            "XX04-0000",
            "LS04-0000",
            "LS0G-0000-0000-0001-AC58-T2XM-57NU",
            // A version 3 code (M3): decodes, but this build refuses it.
            "LS0C-0000-0000-000A-K4M6-8001-8F1Y",
        ] {
            match check_share_code(bad) {
                CodeCheck::Invalid(msg) => {
                    assert!(!msg.is_empty());
                    assert!(msg.chars().next().unwrap().is_uppercase(), "{msg}");
                }
                other => panic!("{bad:?} accepted: {other:?}"),
            }
        }
    }

    #[test]
    fn random_codes_are_valid_and_depend_on_entropy() {
        let a = random_share_code(1, 40, 30).unwrap();
        let b = random_share_code(2, 40, 30).unwrap();
        assert_ne!(a, b);
        assert_eq!(random_share_code(1, 40, 30).unwrap(), a);
        let CodeCheck::Valid { sliders, .. } = check_share_code(&a) else {
            panic!("random code invalid: {a}");
        };
        assert_eq!(&sliders[..2], &[40, 30]);
        // Out-of-range sliders are clamped rather than rejected.
        assert!(random_share_code(3, 250, 255).is_ok());
    }

    #[test]
    fn made_codes_carry_seed_sliders_and_archetype() {
        let code = make_share_code("18446744073709551615", &[1, 2, 3, 4, 5, 6, 7], 1).unwrap();
        let CodeCheck::Valid {
            seed,
            sliders,
            archetype,
            ..
        } = check_share_code(&code)
        else {
            panic!("made code invalid: {code}");
        };
        assert_eq!(seed, u64::MAX);
        assert_eq!(sliders, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(archetype, 1);
        assert_eq!(
            make_share_code("42", &[50, 40, 50, 0, 0, 20, 30], 0).unwrap(),
            REAL
        );
        assert!(make_share_code("x", &[0; 7], 0).is_err());
        assert!(make_share_code("1", &[0; 6], 0).is_err());
        assert!(make_share_code("1", &[0; 7], 5).is_err());
        assert!(make_share_code("1", &[200; 7], 1).is_ok());
        // The gallery's worlds: random_share_code's seed with any sliders.
        let a = random_share_code(5, 40, 30).unwrap();
        let b = world_share_code(5, &[40, 30, 50, 0, 0, 20, 30], 0).unwrap();
        assert_eq!(a, b);
    }
}
