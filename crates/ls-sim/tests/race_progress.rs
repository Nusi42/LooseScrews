//! Fixture of Belt Up's progress function (`ls_sim::modes::belt_up::progress`)
//! for the browser's race HUD (M4 C7): `web/src/game/race/course.test.ts`
//! reads `fixtures/race_progress.txt` and checks its TypeScript mirror
//! (`raceProgress`) against these values, so the HUD's places, gaps and
//! distances follow the simulation's ordering.
//!
//! Re-bless after an intended change of the progress function:
//! `LS_BLESS_RACE_PROGRESS=1 cargo test -p ls-sim --test race_progress`.

use ls_math::level::{BeltSite, BeltSlot, BELT_WEDGES};
use ls_math::{Quat, Vec3};
use ls_sim::modes::belt_up::{progress, Racer};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/race_progress.txt"
);

fn belt() -> BeltSite {
    let center = Vec3::new(160.0, 48.0, 160.0);
    let mut b = BeltSite {
        center,
        inner_radius: 95.0,
        outer_radius: 145.0,
        half_thickness: 12.0,
        cocoon_radius: 30.0,
        exit_radius: 6.0,
        gate_inner_radius: 87.0,
        gate_outer_radius: 153.0,
        slots: [BeltSlot {
            pos: Vec3::ZERO,
            orientation: Quat::IDENTITY,
        }; BELT_WEDGES],
        pickup_points: Vec::new(),
        boulders: Vec::new(),
    };
    for k in 0..BELT_WEDGES {
        let a = BeltSite::gate_azimuth(k) + 3.0f32.to_radians();
        b.slots[k] = BeltSlot {
            pos: center + Vec3::new(120.0 * a.cos(), 0.0, -120.0 * a.sin()),
            orientation: Quat::from_axis_angle(Vec3::Y, a),
        };
    }
    b
}

fn racer(slot: u8, gates: u8) -> Racer {
    Racer {
        order: 0,
        in_heat: true,
        slot,
        gates,
        finish_tick: 0,
        place: 0,
        hold_end_tick: 0,
        gate_hull: 0.0,
        last_pos: Vec3::ZERO,
        fresh: false,
        heat_wins: 0,
        last_place: 0,
        total_ticks: 0,
    }
}

/// Deterministic sample points around and inside the ring (a small LCG).
fn samples() -> Vec<(u8, u8, Vec3)> {
    let b = belt();
    let mut s: u32 = 0x9e37_79b9;
    let mut next = || {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (s >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut out = Vec::new();
    for i in 0..96 {
        let slot = (i % 8) as u8;
        let gates = (i / 8 % 9) as u8;
        // Most samples near the racer's current sector (the interesting
        // keys), the rest anywhere (clamped keys, the dive).
        let last = (slot as usize + gates as usize) % BELT_WEDGES;
        let wedge = std::f32::consts::FRAC_PI_4;
        let a = if i % 4 == 3 {
            next() * std::f32::consts::TAU
        } else {
            BeltSite::gate_azimuth(last) + (next() * 1.6 - 0.3) * wedge
        };
        let r = 10.0 + next() * 150.0;
        let y = (next() - 0.5) * 60.0;
        out.push((
            slot,
            gates,
            b.center + Vec3::new(r * a.cos(), y, -r * a.sin()),
        ));
    }
    out
}

fn render() -> String {
    let b = belt();
    let mut text = String::from(
        "# Belt Up progress (ls_sim::modes::belt_up::progress) on a belt centred at 160 48 160\n\
         # (radii 95/145, exit 6). Columns: slot gates x y z -> gates key. Written by\n\
         # crates/ls-sim/tests/race_progress.rs (LS_BLESS_RACE_PROGRESS=1), read by\n\
         # web/src/game/race/course.test.ts.\n",
    );
    for (slot, gates, p) in samples() {
        let (g, k) = progress(Some(&b), &racer(slot, gates), p);
        text.push_str(&format!("{slot} {gates} {} {} {} {g} {k}\n", p.x, p.y, p.z));
    }
    text
}

#[test]
fn progress_fixture_is_current() {
    let text = render();
    if std::env::var("LS_BLESS_RACE_PROGRESS").is_ok() {
        std::fs::write(FIXTURE, &text).expect("write the fixture");
        return;
    }
    let old = std::fs::read_to_string(FIXTURE).expect("fixture exists (bless it)");
    assert_eq!(
        old.replace("\r\n", "\n"),
        text,
        "Belt Up's progress changed: re-bless with LS_BLESS_RACE_PROGRESS=1 and check the HUD mirror"
    );
}
