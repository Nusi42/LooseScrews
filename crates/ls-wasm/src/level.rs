//! Level facts of a share code without a game (generator version 4): the
//! gravity well sites and the asteroid belt, for the world workshop's
//! preview and the visual gallery's camera. The in-match HUD reads them
//! from the match instead (`MatchSetup`).
//!
//! In a match (`Game`): the asteroid belt (slots, gates, exit, boulder
//! orbits), gravity wells and the level's up.
//!
//! - [`belt_json`]: `{center: [x, y, z], innerRadius, outerRadius,
//!   halfThickness, cocoonRadius, exitRadius, gateInnerRadius,
//!   gateOuterRadius, gateAzimuths: [8 rad], slots: [{pos: [x, y, z], rot:
//!   [x, y, z, w]}], pickupPoints: [[x, y, z]], boulders: [{radius, height,
//!   phase, angularSpeed, size, tumbleAxis: [x, y, z], tumbleSpeed}]}`
//!   (`ls_math::level::BeltSite`; azimuths are prograde, counter-clockwise
//!   seen from above).
//! - [`boulder_poses`]: the drifting boulders at a (fractional) tick, from
//!   the same orbit functions the simulation uses.
//! - [`wells_array`]: the well field (static and dynamic wells).

use ls_math::level::{BeltSite, BELT_WEDGES};
use ls_math::{Quat, Vec3};
use ls_worldgen::{Generator, ShareCode, WellKind};

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

/// Floats per boulder in [`boulder_poses`]: position, orientation, radius.
pub const BOULDER_STRIDE: usize = 8;
/// Floats per well in [`wells_array`].
pub const WELL_STRIDE: usize = 8;

/// The well field as flat floats ([`WELL_STRIDE`] each): `[x, y, z, range,
/// peakAccel, coreRadius (0 dynamic), startTick, endTick (-1 static)]`.
pub fn wells_array(f: &ls_sim::WellField) -> Vec<f32> {
    let mut out = Vec::with_capacity((f.statics.len() + f.dynamics.len()) * WELL_STRIDE);
    for w in &f.statics {
        out.extend_from_slice(&[
            w.pos.x,
            w.pos.y,
            w.pos.z,
            w.range,
            w.peak_accel,
            w.core_radius,
            0.0,
            -1.0,
        ]);
    }
    for w in &f.dynamics {
        out.extend_from_slice(&[
            w.pos.x,
            w.pos.y,
            w.pos.z,
            w.range,
            w.peak_accel,
            0.0,
            w.start_tick as f32,
            w.end_tick as f32,
        ]);
    }
    out
}

fn num(v: f32) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        "0".into()
    }
}

fn vec3(v: Vec3) -> String {
    format!("[{},{},{}]", num(v.x), num(v.y), num(v.z))
}

fn quat(q: Quat) -> String {
    format!("[{},{},{},{}]", num(q.x), num(q.y), num(q.z), num(q.w))
}

fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
}

/// See the module docs.
pub fn belt_json(b: &BeltSite) -> String {
    let slots = b.slots.iter().map(|s| {
        format!(
            "{{\"pos\":{},\"rot\":{}}}",
            vec3(s.pos),
            quat(s.orientation)
        )
    });
    let boulders = b.boulders.iter().map(|o| {
        format!(
            "{{\"radius\":{},\"height\":{},\"phase\":{},\"angularSpeed\":{},\"size\":{},\"tumbleAxis\":{},\"tumbleSpeed\":{}}}",
            num(o.radius),
            num(o.height),
            num(o.phase),
            num(o.angular_speed),
            num(o.size),
            vec3(o.tumble_axis),
            num(o.tumble_speed)
        )
    });
    format!(
        "{{\"center\":{},\"innerRadius\":{},\"outerRadius\":{},\"halfThickness\":{},\"cocoonRadius\":{},\"exitRadius\":{},\"gateInnerRadius\":{},\"gateOuterRadius\":{},\"gateAzimuths\":{},\"slots\":{},\"pickupPoints\":{},\"boulders\":{}}}",
        vec3(b.center),
        num(b.inner_radius),
        num(b.outer_radius),
        num(b.half_thickness),
        num(b.cocoon_radius),
        num(b.exit_radius),
        num(b.gate_inner_radius),
        num(b.gate_outer_radius),
        list((0..BELT_WEDGES).map(|k| num(BeltSite::gate_azimuth(k)))),
        list(slots),
        list(b.pickup_points.iter().map(|&p| vec3(p))),
        list(boulders),
    )
}

/// See the module docs: [`BOULDER_STRIDE`] floats per boulder, positions
/// lerped and tumbles nlerped between the two integer ticks around `tick`.
pub fn boulder_poses(b: &BeltSite, tick: f64) -> Vec<f32> {
    let tick = if tick.is_finite() { tick.max(0.0) } else { 0.0 };
    let t0 = tick.floor() as u64;
    let f = (tick - t0 as f64) as f32;
    let mut out = Vec::with_capacity(b.boulders.len() * BOULDER_STRIDE);
    for o in &b.boulders {
        let (p0, p1) = (o.pos_at(b.center, t0), o.pos_at(b.center, t0 + 1));
        let (q0, q1) = (o.tumble_at(t0), o.tumble_at(t0 + 1));
        let p = p0.lerp(p1, f);
        let q = q0.nlerp(q1, f);
        out.extend_from_slice(&[p.x, p.y, p.z, q.x, q.y, q.z, q.w, o.size]);
    }
    out
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

    use ls_math::level::{BeltSlot, BoulderOrbit};

    fn belt() -> BeltSite {
        BeltSite {
            center: Vec3::new(160.0, 48.0, 160.0),
            inner_radius: 95.0,
            outer_radius: 145.0,
            half_thickness: 12.0,
            cocoon_radius: 30.0,
            exit_radius: 6.0,
            gate_inner_radius: 87.0,
            gate_outer_radius: 153.0,
            slots: [BeltSlot {
                pos: Vec3::new(280.0, 48.0, 160.0),
                orientation: Quat::IDENTITY,
            }; BELT_WEDGES],
            pickup_points: vec![Vec3::new(1.0, 2.0, 3.0)],
            boulders: vec![BoulderOrbit {
                radius: 120.0,
                height: 2.0,
                phase: 0.5,
                angular_speed: 0.02,
                size: 3.0,
                tumble_axis: Vec3::Y,
                tumble_speed: 0.3,
            }],
        }
    }

    #[test]
    fn belt_json_lists_everything() {
        let j = belt_json(&belt());
        for key in [
            "\"center\":[160,48,160]",
            "\"innerRadius\":95",
            "\"exitRadius\":6",
            "\"gateAzimuths\":[0,",
            "\"slots\":[{\"pos\":[280,48,160],\"rot\":[0,0,0,1]}",
            "\"pickupPoints\":[[1,2,3]]",
            "\"angularSpeed\":0.02",
        ] {
            assert!(j.contains(key), "{key} in {j}");
        }
        assert_eq!(j.matches("\"pos\"").count(), BELT_WEDGES);
    }

    #[test]
    fn boulder_poses_follow_the_orbit_function() {
        let b = belt();
        let at = boulder_poses(&b, 600.0);
        assert_eq!(at.len(), BOULDER_STRIDE);
        let exact = b.boulders[0].pos_at(b.center, 600);
        assert_eq!(Vec3::new(at[0], at[1], at[2]), exact);
        assert_eq!(at[7], 3.0);
        // Between ticks: between the two positions.
        let mid = boulder_poses(&b, 600.5);
        let next = b.boulders[0].pos_at(b.center, 601);
        let m = Vec3::new(mid[0], mid[1], mid[2]);
        assert!((m - (exact + next) * 0.5).length() < 1e-4);
        assert!(boulder_poses(&b, f64::NAN).iter().all(|v| v.is_finite()));
    }
}
