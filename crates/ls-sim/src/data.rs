//! The game data set: every `data/` file the simulation embeds (weapons,
//! ships, gadgets, rule sets), the tables built from them, and the data hash.
//!
//! The data hash is a deterministic FNV-1a hash over all embedded files,
//! sorted by path (`weapons/scrap_laser.ron`, ...): for each file its path,
//! a 0 byte, its length and its bytes, with `\r` removed so a checkout with
//! CRLF line endings hashes like one with LF. Clients send it in `Hello`; the
//! server rejects a client built with different data, because both sides
//! must simulate with the same numbers (prediction, visual ids, weapon ids).

use std::sync::OnceLock;

use crate::hash::StateHasher;
use crate::{GadgetTable, ShipTable, WeaponTable};

/// The data tables of a match.
#[derive(Debug, Clone, PartialEq)]
pub struct GameData {
    pub weapons: WeaponTable,
    pub ships: ShipTable,
    pub gadgets: GadgetTable,
}

impl GameData {
    /// The embedded data (`data/`).
    pub fn builtin() -> GameData {
        GameData {
            weapons: WeaponTable::builtin(),
            ships: ShipTable::builtin(),
            gadgets: GadgetTable::builtin(),
        }
    }
}

impl Default for GameData {
    fn default() -> Self {
        GameData::builtin()
    }
}

/// Every embedded data file as `(path relative to data/, text)`, sorted by path.
pub fn builtin_files() -> Vec<(&'static str, &'static str)> {
    let mut files: Vec<(&'static str, &'static str)> = Vec::new();
    files.extend(crate::weapons::MODULES.iter().map(|m| (m.file, m.data)));
    files.extend(crate::gadgets::MODULES.iter().map(|m| (m.file, m.data)));
    files.extend(
        crate::tuning::BUILTIN_SHIPS
            .iter()
            .map(|(_, f, d)| (*f, *d)),
    );
    files.extend(crate::ruleset::BUILTIN_RULESETS.iter().copied());
    files.sort_by(|a, b| a.0.cmp(b.0));
    files
}

/// Hash of a set of data files (see the module docs); order-independent.
pub fn hash_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a str)>) -> u64 {
    let mut files: Vec<(&str, &str)> = files.into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(b.0));
    let mut h = StateHasher::new();
    h.write_u32(files.len() as u32);
    for (path, text) in files {
        h.write_bytes(path.as_bytes());
        h.write_u8(0);
        let bytes: Vec<u8> = text.bytes().filter(|&b| b != b'\r').collect();
        h.write_u64(bytes.len() as u64);
        h.write_bytes(&bytes);
    }
    h.finish()
}

/// Version of the simulation behaviour (weapon and gadget modules, framework
/// rules), mixed into [`data_hash`]: a client with the same data but older
/// behaviour code must not join and desync silently. Bump it together with
/// `REPLAY_GOLDEN_HASH` (`crates/ls-sim/src/tests.rs`) whenever behaviour
/// changes without a data change.
pub const SIM_BEHAVIOR_VERSION: u32 = 3;

/// Hash of the embedded data set plus [`SIM_BEHAVIOR_VERSION`] (computed once).
pub fn data_hash() -> u64 {
    static HASH: OnceLock<u64> = OnceLock::new();
    *HASH.get_or_init(|| with_behavior_version(hash_files(builtin_files())))
}

/// Mixes [`SIM_BEHAVIOR_VERSION`] into a data file hash.
pub fn with_behavior_version(files_hash: u64) -> u64 {
    let mut h = StateHasher::new();
    h.write_u64(files_hash);
    h.write_u32(SIM_BEHAVIOR_VERSION);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn hash_is_stable_and_sensitive() {
        let files = builtin_files();
        assert_eq!(files.len(), 27 + 3 + 4 + 2);
        assert_eq!(
            data_hash(),
            with_behavior_version(hash_files(files.clone()))
        );
        assert_ne!(
            data_hash(),
            hash_files(files.clone()),
            "behaviour version mixed in"
        );
        let base = hash_files(files.clone());
        // Order and line endings do not matter.
        let mut rev = files.clone();
        rev.reverse();
        assert_eq!(hash_files(rev), base);
        let crlf: Vec<(String, String)> = files
            .iter()
            .map(|(p, t)| (p.to_string(), t.replace('\n', "\r\n")))
            .collect();
        assert_eq!(
            hash_files(crlf.iter().map(|(p, t)| (p.as_str(), t.as_str()))),
            base
        );
        // Any change does.
        let mut changed = files.clone();
        let tweaked = changed[0].1.replace("speed", "speeed");
        changed[0].1 = &tweaked;
        assert_ne!(hash_files(changed), base);
        assert_ne!(hash_files(files[1..].iter().copied()), base);
    }

    /// Every embedded file on disk is embedded unchanged (native only), and
    /// every file in `data/` of a known kind is embedded.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn embedded_files_match_disk() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let embedded = builtin_files();
        for (path, text) in &embedded {
            let disk = std::fs::read_to_string(root.join(path)).unwrap();
            assert_eq!(disk.replace('\r', ""), text.replace('\r', ""), "{path}");
        }
        for dir in ["weapons", "ships", "gadgets", "rulesets"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap() {
                let name = entry.unwrap().file_name().into_string().unwrap();
                if name.ends_with(".ron") {
                    let path = format!("{dir}/{name}");
                    assert!(
                        embedded.iter().any(|(p, _)| *p == path),
                        "{path} is not embedded (register it)"
                    );
                }
            }
        }
    }
}
