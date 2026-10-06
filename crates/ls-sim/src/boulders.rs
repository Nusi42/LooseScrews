//! Drifting boulders of asteroid belt levels (M4 Belt Up;
//! `docs/milestone-4-plan.md` §3.8 "Drifting boulders").
//!
//! Besides the static voxel asteroids, 3-10 boulders per wedge orbit the
//! belt axis prograde on seeded circles (`BeltSite::boulders`,
//! [`BoulderOrbit`]). Their positions are a pure function of the tick and
//! the level, so nothing is replicated: the server and the client predictor
//! call the same functions here with the same tick and get the same push.
//!
//! - Ships: a ship sphere overlapping a boulder is put back on its surface
//!   and its velocity relative to the boulder loses its inward part, bouncing
//!   off with [`BoulderSettings::restitution`] ([`push_sphere`], after the
//!   ship's movement in `World::step` and in the predictor). A contact
//!   faster than `damage_speed` deals `damage` (server only, no credit).
//! - Projectiles: boulders absorb them ([`segment_hit`]): a projectile that
//!   reaches one ends there; ordnance (splash damage) detonates without a
//!   crater. Boulders are indestructible.
//!
//! The world applies them on every level with a belt (classic levels have
//! none). Numbers: `data/world/boulders.ron`. This is the kinematic-obstacle
//! code B3's Duck parade shares in spirit (the plan's `obstacle.rs`); the
//! integrator may fold both into one helper.

use std::sync::OnceLock;

use ls_math::level::{BeltSite, BoulderOrbit};
use ls_math::Vec3;
use serde::{Deserialize, Serialize};

/// `data/world/boulders.ron` (embedded, part of the data hash).
pub const BOULDERS_RON: &str = include_str!("../../../data/world/boulders.ron");

/// Boulder contact numbers (`data/world/boulders.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoulderSettings {
    /// Damage of a fast contact (hull, no kill credit).
    pub damage: f32,
    /// Contacts faster than this along the normal (u/s, relative) hurt.
    pub damage_speed: f32,
    /// Share of the inward relative speed a ship bounces off with.
    pub restitution: f32,
}

impl BoulderSettings {
    /// The embedded `data/world/boulders.ron`.
    pub fn builtin() -> &'static BoulderSettings {
        static S: OnceLock<BoulderSettings> = OnceLock::new();
        S.get_or_init(|| {
            ron::from_str(BOULDERS_RON).expect("data/world/boulders.ron is valid (tested)")
        })
    }
}

/// Whether a sphere at `p` with radius `r` can touch boulder `o` at all:
/// its distance from the axis and its height must both be within reach of
/// the orbit (cheap test before evaluating the orbit's position).
fn may_touch(belt: &BeltSite, o: &BoulderOrbit, p: Vec3, r: f32) -> bool {
    let reach = o.size + r;
    let dy = p.y - (belt.center.y + o.height);
    if dy.abs() > reach {
        return false;
    }
    let d = p - belt.center;
    let axis = Vec3::new(d.x, 0.0, d.z).length();
    (axis - o.radius).abs() <= reach
}

/// Pushes a sphere (centre `pos`, velocity `vel`, radius `radius`) out of
/// every boulder it overlaps at tick `now` and removes the inward part of
/// its velocity relative to the boulder (bouncing off with the settings'
/// restitution). Returns the largest approach speed (u/s, along the contact
/// normal, relative to the boulder) of this tick's contacts, 0 without one.
/// Deterministic (IEEE-exact operations plus `ls_math::det`).
pub fn push_sphere(
    belt: &BeltSite,
    settings: &BoulderSettings,
    now: u64,
    pos: &mut Vec3,
    vel: &mut Vec3,
    radius: f32,
) -> f32 {
    let mut worst = 0.0f32;
    for o in &belt.boulders {
        if !may_touch(belt, o, *pos, radius) {
            continue;
        }
        let c = o.pos_at(belt.center, now);
        let reach = o.size + radius;
        let d = *pos - c;
        let dist2 = d.length_squared();
        if dist2 >= reach * reach {
            continue;
        }
        let dist = dist2.sqrt();
        let n = if dist > 1e-6 {
            d / dist
        } else {
            // Dead centre: push outward from the axis.
            let r = c - belt.center;
            Vec3::new(r.x, 0.0, r.z).try_normalize().unwrap_or(Vec3::Y)
        };
        *pos = c + n * reach;
        let vb = o.velocity_at(now);
        let vn = (*vel - vb).dot(n);
        if vn < 0.0 {
            *vel -= n * (vn * (1.0 + settings.restitution));
            worst = worst.max(-vn);
        }
    }
    worst
}

/// The first boulder a moving sphere (projectile) hits at tick `now`: the
/// segment from `origin` along the unit `dir` for `max` u, swept with
/// `radius`. Returns the distance along `dir` and the contact normal.
pub fn segment_hit(
    belt: &BeltSite,
    now: u64,
    origin: Vec3,
    dir: Vec3,
    max: f32,
    radius: f32,
) -> Option<(f32, Vec3)> {
    let mid = origin + dir * (0.5 * max);
    let mut best: Option<(f32, Vec3)> = None;
    for o in &belt.boulders {
        if !may_touch(belt, o, mid, radius + 0.5 * max) {
            continue;
        }
        let c = o.pos_at(belt.center, now);
        let Some(t) = crate::terrain::segment_sphere(origin, dir, max, c, o.size + radius) else {
            continue;
        };
        if best.is_none_or(|(bt, _)| t < bt) {
            let p = origin + dir * t;
            let n = (p - c).try_normalize().unwrap_or(-dir);
            best = Some((t, n));
        }
    }
    best
}

/// Every boulder's centre and radius at tick `now` (clients draw them from
/// the same function; tests).
pub fn poses(belt: &BeltSite, now: u64) -> impl Iterator<Item = (Vec3, f32)> + '_ {
    belt.boulders
        .iter()
        .map(move |o| (o.pos_at(belt.center, now), o.size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radiation::tests::plan_belt;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn orbit(radius: f32, height: f32, phase: f32, speed: f32, size: f32) -> BoulderOrbit {
        BoulderOrbit {
            radius,
            height,
            phase,
            angular_speed: speed,
            size,
            tumble_axis: Vec3::Y,
            tumble_speed: 0.5,
        }
    }

    #[test]
    fn settings_parse() {
        let s = BoulderSettings::builtin();
        assert_eq!(s.damage, 8.0);
        assert_eq!(s.damage_speed, 8.0);
        assert!(s.restitution >= 0.0 && s.restitution < 1.0);
    }

    #[test]
    fn a_ship_is_pushed_out_and_bounces() {
        let mut b = plan_belt();
        b.boulders = vec![orbit(120.0, 0.0, 0.0, 0.025, 3.0)];
        let s = BoulderSettings::builtin();
        let c = b.boulders[0].pos_at(b.center, 10);
        // A ship flying into the boulder along -x at 12 u/s, overlapping by 0.5 u.
        let mut pos = c + Vec3::new(3.0, 0.0, 0.0);
        let mut vel = Vec3::new(-12.0, 0.0, 0.0);
        let hit = push_sphere(&b, s, 10, &mut pos, &mut vel, 0.5);
        assert!((pos.distance(c) - 3.5).abs() < 1e-4, "{pos:?}");
        let vb = b.boulders[0].velocity_at(10);
        // Approach speed along the normal, relative to the boulder.
        assert!((hit - (12.0 + vb.x)).abs() < 1e-3, "{hit}");
        assert!(vel.x > vb.x, "bounces away: {vel:?}");
        // Already separating: pushed out but not slowed.
        let mut pos2 = c + Vec3::new(3.0, 0.0, 0.0);
        let mut vel2 = vb + Vec3::new(5.0, 0.0, 0.0);
        assert_eq!(push_sphere(&b, s, 10, &mut pos2, &mut vel2, 0.5), 0.0);
        assert_eq!(vel2, vb + Vec3::new(5.0, 0.0, 0.0));
        // Far away: untouched.
        let mut pos3 = c + Vec3::new(0.0, 10.0, 0.0);
        let mut vel3 = Vec3::ZERO;
        assert_eq!(push_sphere(&b, s, 10, &mut pos3, &mut vel3, 0.5), 0.0);
        assert_eq!(pos3, c + Vec3::new(0.0, 10.0, 0.0));
    }

    #[test]
    fn a_resting_ship_is_swept_along() {
        // A boulder catching up with a ship at rest pushes it prograde.
        let mut b = plan_belt();
        b.boulders = vec![orbit(120.0, 2.0, 0.0, 0.03, 2.5)];
        let s = BoulderSettings::builtin();
        let start = b.boulders[0].pos_at(b.center, 0);
        let ahead = b.boulders[0].velocity_at(0).normalize();
        let mut pos = start + ahead * 4.0;
        let mut vel = Vec3::ZERO;
        let mut contacts = 0;
        for tick in 0..600u64 {
            pos += vel * ls_math::DT;
            if push_sphere(&b, s, tick, &mut pos, &mut vel, 0.5) > 0.0 {
                contacts += 1;
            }
            let c = b.boulders[0].pos_at(b.center, tick);
            assert!(pos.distance(c) >= 3.0 - 1e-3, "tick {tick}");
        }
        assert!(contacts > 0);
        assert!(vel.length() > 1.0, "pushed along: {vel:?}");
    }

    #[test]
    fn projectiles_hit_boulders() {
        let mut b = plan_belt();
        b.boulders = vec![
            orbit(120.0, 0.0, 0.0, 0.03, 3.0),
            orbit(130.0, 6.0, 1.0, 0.02, 2.0),
        ];
        let c = b.boulders[0].pos_at(b.center, 5);
        let origin = c + Vec3::new(0.0, 0.0, 10.0);
        let (t, n) = segment_hit(&b, 5, origin, Vec3::new(0.0, 0.0, -1.0), 20.0, 0.2).unwrap();
        assert!((t - (10.0 - 3.2)).abs() < 1e-3, "{t}");
        assert!(n.z > 0.99);
        // Too short, or passing beside it.
        assert!(segment_hit(&b, 5, origin, Vec3::new(0.0, 0.0, -1.0), 5.0, 0.2).is_none());
        let beside = c + Vec3::new(0.0, 4.0, 10.0);
        assert!(segment_hit(&b, 5, beside, Vec3::new(0.0, 0.0, -1.0), 20.0, 0.2).is_none());
        assert_eq!(poses(&b, 5).count(), 2);
    }
}
