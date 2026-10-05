//! Level facts of a share code without a game (generator version 4): the
//! gravity well sites and the asteroid belt, for the world workshop's
//! preview and the visual gallery's camera. The in-match HUD reads them
//! from the match instead (`MatchSetup`).

use ls_worldgen::{BeltSite, Generator, ShareCode, WellKind};

/// Floats per well in [`level_wells`].
pub const WELL_FLOATS: usize = 8;

/// The gravity well sites of `code`: per well `x, y, z, core radius,
/// hollow radius, strength, range, kind` (kind 0 core, 1 floating stone,
/// 2 shepherd moon).
pub fn level_wells(code: &str) -> Result<Vec<f32>, String> {
    let gen = generator(code)?;
    let mut out = Vec::with_capacity(gen.gravity_wells().len() * WELL_FLOATS);
    for w in gen.gravity_wells() {
        out.extend(w.pos.to_array());
        out.extend([w.core_radius, w.hollow_radius, w.strength, w.range]);
        out.push(match w.kind {
            WellKind::Core => 0.0,
            WellKind::Floating => 1.0,
            WellKind::Shepherd => 2.0,
        });
    }
    Ok(out)
}

/// The asteroid belt of `code` (empty for classic levels): centre `x, y,
/// z`, inner and outer radius, half thickness, cocoon and exit radius,
/// gate inner and outer radius, the 8 start slots as `x, y, z` each, then
/// the clear corridor's centre line (a closed loop, prograde from gate 0)
/// as `x, y, z` per point.
pub fn level_belt(code: &str) -> Result<Vec<f32>, String> {
    let gen = generator(code)?;
    let (Some(b), Some(field)) = (gen.belt(), gen.belt_field()) else {
        return Ok(Vec::new());
    };
    let mut out = belt_floats(b);
    for p in field.corridor() {
        out.extend(p.to_array());
    }
    Ok(out)
}

fn belt_floats(b: &BeltSite) -> Vec<f32> {
    let mut out = b.center.to_array().to_vec();
    out.extend([
        b.inner_radius,
        b.outer_radius,
        b.half_thickness,
        b.cocoon_radius,
        b.exit_radius,
        b.gate_inner_radius,
        b.gate_outer_radius,
    ]);
    for s in &b.slots {
        out.extend(s.pos.to_array());
    }
    out
}

fn generator(code: &str) -> Result<Generator, String> {
    let code = ShareCode::decode(code).map_err(|e| e.to_string())?;
    Generator::from_code(&code).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_worldgen::{Archetype, Sliders};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn wells_and_belts_of_a_code() {
        let wells = ShareCode::new(
            42,
            Sliders {
                gravity_chaos: 100,
                ..Sliders::default()
            },
        );
        let w = level_wells(&wells.encode().unwrap()).unwrap();
        assert_eq!(w.len(), 7 * WELL_FLOATS);
        assert!(level_belt(&wells.encode().unwrap()).unwrap().is_empty());
        let belt = ShareCode::with_archetype(9, Sliders::default(), Archetype::Belt);
        let b = level_belt(&belt.encode().unwrap()).unwrap();
        assert_eq!(b.len(), 10 + 8 * 3 + 8 * 24 * 3);
        assert_eq!(&b[..5], &[160.0, 48.0, 160.0, 95.0, 145.0]);
        assert!(level_wells("LS00").is_err());
    }
}
