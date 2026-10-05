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
use ls_worldgen::{Archetype, ShareCode, Sliders, GENERATOR_VERSION};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

const FIXTURE: &str = include_str!("fixtures/golden_hashes.txt");

/// The fixed world list: open sky, sky/cavern mix, caverns, cavern/mine mix,
/// dense mines, a code with every slider at 100 (the tall box with gravity
/// wells), and the Heist world of the M3 WASM cross-check (since generator
/// version 3 the SDF hash also covers every level's Heist site: core vein
/// and extraction zones). Generator version 4 adds one world per other
/// level shape (Verticality), a default box with seven gravity wells, and
/// two asteroid belts (one with shepherd moons); the SDF hash covers the
/// well sites and the belt site too.
fn golden_codes() -> Vec<ShareCode> {
    let with = |e: u8, d: u8, v: u8, g: u8| Sliders {
        verticality: v,
        gravity_chaos: g,
        ..Sliders::new(e, d)
    };
    vec![
        ShareCode::new(1, Sliders::new(0, 0)),
        ShareCode::new(0x00C0_FFEE, Sliders::new(25, 70)),
        ShareCode::new(42, Sliders::new(50, 40)),
        ShareCode::new(0xDEAD_BEEF, Sliders::new(75, 20)),
        ShareCode::new(7777, Sliders::new(100, 100)),
        ShareCode::new(u64::MAX, Sliders::from_array([100; 7]).unwrap()),
        ShareCode::new(0x0048_4549_5354, Sliders::new(60, 50)),
        ShareCode::new(0x464c_4154, with(45, 50, 5, 0)),
        ShareCode::new(0x4c4f_5733, with(30, 40, 30, 20)),
        ShareCode::new(0x5441_4c4c, with(55, 30, 70, 60)),
        ShareCode::new(0x5745_4c4c, with(50, 40, 50, 100)),
        ShareCode::with_archetype(0x4245_4c54, with(50, 60, 50, 0), Archetype::Belt),
        ShareCode::with_archetype(0x4d4f_4f4e, with(70, 80, 70, 60), Archetype::Belt),
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

/// The version 4 worlds of the list cover what the WASM cross-check needs:
/// every level shape, a level with seven gravity wells, and two belts, one
/// with shepherd moons.
#[test]
fn golden_list_covers_shapes_wells_and_belts() {
    use ls_worldgen::{Generator, BOX_SHAPES};
    let gens: Vec<Generator> = golden_codes()
        .iter()
        .map(|c| Generator::from_code(c).unwrap())
        .collect();
    for shape in BOX_SHAPES {
        assert!(
            gens.iter()
                .any(|g| g.archetype() == Archetype::Classic && g.size_chunks() == shape),
            "{shape:?}"
        );
    }
    assert!(gens.iter().any(|g| g.gravity_wells().len() == 7));
    let belts: Vec<&Generator> = gens.iter().filter(|g| g.belt().is_some()).collect();
    assert_eq!(belts.len(), 2);
    assert!(belts.iter().any(|g| g.gravity_wells().len() == 8));
}
