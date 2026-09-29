//! Golden-hash tests: a fixed list of share codes must reproduce the SDF and
//! mesh hashes stored in `tests/fixtures/golden_hashes.txt`, on native and on
//! wasm32 (run under Node with `wasm-bindgen-test-runner`).
//!
//! After an intentional generator change, bump `GENERATOR_VERSION` and
//! re-bless on native:
//!
//! ```text
//! LS_BLESS_GOLDEN=1 cargo test -p ls-worldgen --release --test golden
//! ```

use ls_worldgen::golden::golden_hashes;
use ls_worldgen::{ShareCode, Sliders, GENERATOR_VERSION};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const FIXTURE: &str = include_str!("fixtures/golden_hashes.txt");

/// The fixed world list: open sky, sky/cavern mix, caverns, cavern/mine mix,
/// dense mines, and a code with every slider at 100.
fn golden_codes() -> Vec<ShareCode> {
    vec![
        ShareCode::new(1, Sliders::new(0, 0)),
        ShareCode::new(0x00C0_FFEE, Sliders::new(25, 70)),
        ShareCode::new(42, Sliders::new(50, 40)),
        ShareCode::new(0xDEAD_BEEF, Sliders::new(75, 20)),
        ShareCode::new(7777, Sliders::new(100, 100)),
        ShareCode::new(u64::MAX, Sliders::from_array([100; 7]).unwrap()),
    ]
}

struct Entry {
    code: String,
    hashes: [String; 4],
}

fn parse_fixture() -> Vec<Entry> {
    FIXTURE
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f.len(), 5, "bad fixture line: {l}");
            Entry {
                code: f[0].to_string(),
                hashes: [
                    f[1].to_string(),
                    f[2].to_string(),
                    f[3].to_string(),
                    f[4].to_string(),
                ],
            }
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn bless() {
    let mut out = String::new();
    out.push_str(&format!(
        "# Loose Screws golden hashes, generator version {GENERATOR_VERSION}.\n"
    ));
    out.push_str(
        "# Regenerate: LS_BLESS_GOLDEN=1 cargo test -p ls-worldgen --release --test golden\n",
    );
    out.push_str("# code sdf surface_nets dual_contouring edited\n");
    for code in golden_codes() {
        let h = golden_hashes(&code).unwrap().to_hex();
        out.push_str(&format!(
            "{} {} {} {} {}\n",
            code.encode().unwrap(),
            h[0],
            h[1],
            h[2],
            h[3]
        ));
    }
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/golden_hashes.txt"
    );
    std::fs::write(path, out).expect("write fixture");
}

#[test]
fn golden_hashes_match_fixture() {
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var_os("LS_BLESS_GOLDEN").is_some() {
        bless();
        return;
    }
    let entries = parse_fixture();
    let expected_codes: Vec<String> = golden_codes().iter().map(|c| c.encode().unwrap()).collect();
    let fixture_codes: Vec<String> = entries.iter().map(|e| e.code.clone()).collect();
    assert_eq!(
        fixture_codes, expected_codes,
        "fixture code list is stale; re-bless (see module docs)"
    );
    let mut failures = Vec::new();
    for e in &entries {
        let code = ShareCode::decode(&e.code).expect("fixture code decodes");
        assert_eq!(code.version, GENERATOR_VERSION);
        let got = golden_hashes(&code).unwrap().to_hex();
        for (i, name) in ["sdf", "surface_nets", "dual_contouring", "edited"]
            .iter()
            .enumerate()
        {
            if got[i] != e.hashes[i] {
                failures.push(format!(
                    "{} {name}: expected {} got {}",
                    e.code, e.hashes[i], got[i]
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "golden hash mismatch:\n{}",
        failures.join("\n")
    );
}

#[test]
fn golden_procedure_is_repeatable() {
    let code = golden_codes()[2];
    assert_eq!(golden_hashes(&code).unwrap(), golden_hashes(&code).unwrap());
}
