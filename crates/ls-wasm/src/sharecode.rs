//! Share-code helpers for the start screen: full decode and checksum
//! validation through `ls_worldgen::ShareCode` (the web client's own check
//! only looks at the format).

use ls_math::rng::SplitMix64;
use ls_worldgen::{ShareCode, Sliders};

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

/// Level archetypes this build generates, by share-code archetype id (the
/// workshop's archetype switch, M4 A8). Generator version 4 (M4 A1) adds
/// `Belt` = 1; until it is merged only classic levels exist.
pub const ARCHETYPES: &[&str] = &["classic"];

/// The share code of a world made in the workshop: `seed` as a decimal u64,
/// the seven sliders in share-code order (each 0-100) and the archetype id
/// (an index into [`ARCHETYPES`]).
pub fn make_share_code(seed: &str, sliders: &[u8], archetype: u8) -> Result<String, String> {
    let seed: u64 = seed.trim().parse().map_err(|_| {
        format!("Seed {seed:?} is not a whole number from 0 to 18446744073709551615")
    })?;
    let array: [u8; 7] = sliders
        .try_into()
        .map_err(|_| format!("Expected 7 sliders, got {}", sliders.len()))?;
    let sliders = Sliders::from_array(array).map_err(|e| capitalize(&e.to_string()))?;
    if usize::from(archetype) >= ARCHETYPES.len() {
        return Err(format!(
            "Level archetype {archetype} needs a newer generator (this build has version {})",
            ls_worldgen::GENERATOR_VERSION
        ));
    }
    ShareCode::new(seed, sliders)
        .encode()
        .map_err(|e| capitalize(&e.to_string()))
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
    const REAL: &str = "LS0C-0000-0000-000A-K4M6-8001-8F1Y";

    #[test]
    fn valid_codes_decode_in_any_spelling() {
        let CodeCheck::Valid {
            printed,
            seed,
            sliders,
            version,
        } = check_share_code(" ls0c oooo 0000-OOoa k4m6 8OO1 8f1y ")
        else {
            panic!("expected a valid code");
        };
        assert_eq!(printed, REAL);
        assert_eq!(seed, 42);
        assert_eq!(sliders[0], 50);
        assert_eq!(sliders[1], 40);
        assert_eq!(version, ls_worldgen::GENERATOR_VERSION);
    }

    #[test]
    fn corrupted_codes_are_rejected_with_a_reason() {
        // One symbol changed: format fine, checksum wrong.
        let typo = REAL.replace("K4M6", "K4M7");
        match check_share_code(&typo) {
            CodeCheck::Invalid(msg) => assert!(msg.contains("checksum"), "{msg}"),
            other => panic!("typo accepted: {other:?}"),
        }
        for bad in [
            "",
            "XX04-0000",
            "LS04-0000",
            "LS0C-0000-0000-000A-K4M6-8001-8F1U",
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
    fn workshop_codes_round_trip_every_slider() {
        let sliders = [0, 100, 37, 63, 1, 99, 50];
        for seed in ["0", "42", "18446744073709551615"] {
            let code = make_share_code(seed, &sliders, 0).unwrap();
            let CodeCheck::Valid {
                seed: s,
                sliders: back,
                ..
            } = check_share_code(&code)
            else {
                panic!("workshop code invalid: {code}");
            };
            assert_eq!(s.to_string(), seed);
            assert_eq!(back, sliders);
        }
        let CodeCheck::Valid {
            seed,
            sliders: real,
            ..
        } = check_share_code(REAL)
        else {
            panic!("fixture code invalid");
        };
        assert_eq!(make_share_code(&seed.to_string(), &real, 0).unwrap(), REAL);
        assert!(make_share_code("-1", &sliders, 0).is_err());
        assert!(make_share_code("seed", &sliders, 0).is_err());
        assert!(make_share_code("1", &sliders[..6], 0).is_err());
        assert!(make_share_code("1", &[101, 0, 0, 0, 0, 0, 0], 0).is_err());
        assert!(make_share_code("1", &sliders, ARCHETYPES.len() as u8).is_err());
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
}
