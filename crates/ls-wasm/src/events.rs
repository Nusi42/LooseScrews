//! Comedy event effects for the browser's event visuals (M4 C1,
//! `web/src/game/events/`): what the running event does at a tick, decoded
//! with the simulation's own functions from the replicated mood view and the
//! match seed, so the client draws exactly what the server simulates.
//!
//! [`event_fx_json`]: `{event ("" = none), phase ("announce" | "active" |
//! "ending" | ""), now, activeTick, phaseEndTick, wellScale, hitScale,
//! chicken, craterScale, scavengeScale, up ([x, y, z] or null), hitPlane
//! ([x, y, z] or null), bored, hiccups, cake}`; the last three are `null`
//! unless that event runs:
//! - `bored`: `{from: [x, y, z], axis: [x, y, z], turnDegrees, turnSecs}`
//!   (Bored level: the up at the announce turns about `axis`),
//! - `hiccups`: `{ticks: [..], liftTicks, up: [x, y, z]}` (Level hiccups:
//!   the tick of every hiccup of the active phase and the lift's length),
//! - `cake`: `{origin: [x, y, z]}` (Everything is cake: where the wave
//!   starts).
//!
//! The view is advanced to `now` ([`MoodView::advanced`]) like the
//! predictor's, so phase changes after the newest snapshot show on the
//! server's tick.

use ls_math::Vec3;
use ls_sim::events::bored_level::BoredLevel;
use ls_sim::events::everything_is_cake::EverythingIsCake;
use ls_sim::events::level_hiccups::{hiccup_ticks, lift_dir, lift_ticks};
use ls_sim::{EventKind, MoodView};

use crate::modes::{array, object};

fn num(v: f32) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        "0".into()
    }
}

fn vec3(v: Vec3) -> String {
    array([num(v.x), num(v.y), num(v.z)])
}

/// Phase ids by `EventPhase` index.
const PHASE_IDS: [&str; 3] = ["announce", "active", "ending"];

/// See the module docs.
pub fn event_fx_json(view: &MoodView, seed: u64, now: u64) -> String {
    let v = view.advanced(now);
    let fx = ls_sim::director::event_effects(&v, seed, now);
    let kind = v.event;
    let id = kind.map_or("", |k| k.id());
    let phase = if kind.is_some() {
        PHASE_IDS
            .get(v.phase.index() as usize)
            .copied()
            .unwrap_or("")
    } else {
        ""
    };
    let null = || "null".to_string();
    let bored = match kind {
        Some(EventKind::BoredLevel) => {
            let def = EventKind::BoredLevel.def();
            let (from, axis) = BoredLevel::frame(&v.params);
            object(&[
                ("from", vec3(from)),
                ("axis", vec3(axis)),
                ("turnDegrees", num(def.param("turn_degrees", 90.0))),
                ("turnSecs", num(def.param("turn_secs", 5.0))),
            ])
        }
        _ => null(),
    };
    let hiccups = match kind {
        Some(EventKind::LevelHiccups) if v.phase != ls_sim::EventPhase::Announce => {
            let def = EventKind::LevelHiccups.def();
            object(&[
                (
                    "ticks",
                    array(
                        hiccup_ticks(seed, v.active_tick, def)
                            .into_iter()
                            .map(|t| t.to_string()),
                    ),
                ),
                ("liftTicks", lift_ticks(def).to_string()),
                ("up", vec3(lift_dir(&v.params))),
            ])
        }
        _ => null(),
    };
    let cake = match kind {
        Some(EventKind::EverythingIsCake) => {
            object(&[("origin", vec3(EverythingIsCake::origin(&v.params)))])
        }
        _ => null(),
    };
    object(&[
        ("event", crate::json_str(id)),
        ("phase", crate::json_str(phase)),
        ("now", now.to_string()),
        ("activeTick", v.active_tick.to_string()),
        ("phaseEndTick", v.phase_end_tick.to_string()),
        ("wellScale", num(fx.well_scale)),
        ("hitScale", num(fx.hit_radius_scale)),
        (
            "chicken",
            (fx.projectile_flags & ls_sim::proj_flags::CHICKEN != 0).to_string(),
        ),
        ("craterScale", num(fx.crater_scale)),
        ("scavengeScale", num(fx.scavenge_scale)),
        ("up", fx.up.map_or_else(null, vec3)),
        ("hitPlane", fx.hit_plane.map_or_else(null, vec3)),
        ("bored", bored),
        ("hiccups", hiccups),
        ("cake", cake),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_sim::events::params::{pack_dir, pack_pos};
    use ls_sim::EventPhase;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn view(kind: EventKind, phase: EventPhase, params: [u8; 8]) -> MoodView {
        MoodView {
            event: Some(kind),
            phase,
            active_tick: 600,
            phase_end_tick: 600 + kind.def().active_ticks(),
            params,
            ..MoodView::default()
        }
    }

    #[test]
    fn no_event_is_neutral() {
        let j = event_fx_json(&MoodView::default(), 1, 100);
        assert!(j.contains("\"event\":\"\""), "{j}");
        assert!(j.contains("\"wellScale\":1"), "{j}");
        assert!(j.contains("\"chicken\":false"), "{j}");
        assert!(
            j.contains("\"up\":null") && j.contains("\"cake\":null"),
            "{j}"
        );
    }

    #[test]
    fn hiccup_ticks_match_the_simulation() {
        let mut p = [0u8; 8];
        p[..3].copy_from_slice(&pack_dir(Vec3::Y));
        let v = view(EventKind::LevelHiccups, EventPhase::Active, p);
        let j = event_fx_json(&v, 77, 610);
        let ticks = hiccup_ticks(77, 600, EventKind::LevelHiccups.def());
        assert!(!ticks.is_empty());
        let want = format!(
            "\"ticks\":[{}]",
            ticks
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(j.contains(&want), "{j}");
        assert!(j.contains("\"phase\":\"active\""), "{j}");
    }

    #[test]
    fn bored_cake_and_scales() {
        let mut p = [0u8; 8];
        p[..3].copy_from_slice(&pack_dir(Vec3::Y));
        p[3..6].copy_from_slice(&pack_dir(Vec3::X));
        let j = event_fx_json(&view(EventKind::BoredLevel, EventPhase::Active, p), 1, 900);
        assert!(j.contains("\"axis\":[1,0,0]"), "{j}");
        assert!(!j.contains("\"up\":null"), "{j}");
        let mut c = [0u8; 8];
        c[..6].copy_from_slice(&pack_pos(Vec3::new(10.0, 20.5, -3.0)));
        let j = event_fx_json(
            &view(EventKind::EverythingIsCake, EventPhase::Active, c),
            1,
            700,
        );
        assert!(j.contains("\"origin\":[10,20.5,-3]"), "{j}");
        assert!(j.contains("\"craterScale\":1.44"), "{j}");
        let j = event_fx_json(
            &view(EventKind::RubberChickens, EventPhase::Active, [0; 8]),
            1,
            700,
        );
        assert!(j.contains("\"chicken\":true"), "{j}");
        let j = event_fx_json(
            &view(EventKind::BigHeadDay, EventPhase::Active, [0; 8]),
            1,
            700,
        );
        assert!(j.contains("\"hitScale\":3"), "{j}");
        // The announce phase changes nothing yet.
        let j = event_fx_json(
            &view(EventKind::BigHeadDay, EventPhase::Announce, [0; 8]),
            1,
            500,
        );
        assert!(j.contains("\"hitScale\":1"), "{j}");
    }
}
