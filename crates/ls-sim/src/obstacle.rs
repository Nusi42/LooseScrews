//! Kinematic obstacles (M4): spheres whose motion is a pure function of
//! the tick and replicated state, so the server and the client predictor
//! place them at exactly the same spot every tick. Duck parade's rubber
//! ducks are the first ([`crate::events::duck_parade`]); Belt Up's
//! drifting boulders reuse the same push and absorb code
//! (`docs/milestone-4-plan.md` §3.2, §3.8).
//!
//! An obstacle ignores rock (it is moved, not simulated). It
//!
//! - **pushes ships aside** ([`push_sphere`]): a ship sphere overlapping it
//!   is moved out along the line between the centres (unless that would
//!   put the ship into rock: then only its velocity changes, and the next
//!   tick's movement carries it with normal terrain collisions), and the
//!   ship's velocity toward the obstacle becomes the obstacle's own (it is
//!   carried along, never pulled). Each contact is reported with the
//!   closing speed, so a caller can hurt fast contacts (boulders);
//! - **absorbs projectiles** ([`segment_hit`]): the world removes a
//!   projectile whose swept path enters an obstacle, without its hit hooks
//!   (no damage, no crater, no explosion).
//!
//! The world collects the tick's event obstacles once at the start of the tick
//! (`World::obstacles`; the running comedy event's
//! [`crate::events::EventModule::obstacles`]) and pushes every ship after
//! its movement; the client predictor does the same for the own ship
//! (`ls_game::client::prediction`). IEEE-exact arithmetic only (`sqrt`).

use std::sync::OnceLock;

use ls_math::level::{BeltSite, BoulderOrbit};
use ls_math::terrain::TerrainQuery;
use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::terrain::segment_sphere;

/// What an obstacle is (append only; clients may show contacts per kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ObstacleKind {
    /// Duck parade's giant rubber duck.
    Duck = 0,
}

/// One kinematic obstacle at one tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Obstacle {
    pub kind: ObstacleKind,
    /// Centre (u).
    pub pos: Vec3,
    /// Velocity (u/s) at this tick: what a pushed ship is carried with.
    pub vel: Vec3,
    /// Sphere radius (u).
    pub radius: f32,
}

/// One contact of [`push_sphere`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    /// Index of the obstacle in the slice.
    pub index: usize,
    /// Unit normal from the obstacle's centre toward the sphere.
    pub normal: Vec3,
    /// Speed (u/s) at which the sphere and the obstacle were closing along
    /// the normal before the push (0 when they were already separating).
    pub closing_speed: f32,
}

/// Pushes a sphere (a ship) at `pos` with velocity `vel` and `radius` out
/// of every obstacle it overlaps, in slice order (see the module docs).
/// The positional push is applied only where the sphere stays as clear of
/// rock as it was (`terrain.sdf`); the velocity change always applies.
/// Contacts are appended to `out`.
pub fn push_sphere<T: TerrainQuery + ?Sized>(
    terrain: &T,
    pos: &mut Vec3,
    vel: &mut Vec3,
    radius: f32,
    obstacles: &[Obstacle],
    out: &mut Vec<Contact>,
) {
    for (index, o) in obstacles.iter().enumerate() {
        let min = o.radius + radius;
        let Some(c) = resolve(*pos, *vel, o.pos, o.vel, min, 0.0, || {
            // Dead centre: out sideways from the obstacle's motion (any
            // fixed direction keeps it deterministic).
            let side = o.vel.cross(Vec3::Y);
            if side.length_squared() > 1e-12 {
                side.normalize()
            } else {
                Vec3::Y
            }
        }) else {
            continue;
        };
        *vel = c.vel;
        let clear_now = terrain.sdf(*pos);
        let clear_then = terrain.sdf(c.target);
        if clear_then >= radius || clear_then >= clear_now {
            *pos = c.target;
        }
        out.push(Contact {
            index,
            normal: c.normal,
            closing_speed: c.closing_speed,
        });
    }
}

/// One resolved overlap of a sphere with an obstacle sphere.
struct Resolved {
    /// The sphere's centre on the obstacle's surface.
    target: Vec3,
    /// The sphere's velocity after the contact.
    vel: Vec3,
    normal: Vec3,
    closing_speed: f32,
}

/// The shared contact rule: a sphere at `pos` (velocity `vel`) overlapping
/// an obstacle at `center` (velocity `ovel`) within `reach` is put on the
/// obstacle's surface along the centre line (`dead_centre` gives the normal
/// when the centres coincide), and the inward part of its velocity relative
/// to the obstacle is removed, bounced back with `restitution` (0: carried
/// along). `None` without an overlap. IEEE-exact (`sqrt` only).
fn resolve(
    pos: Vec3,
    vel: Vec3,
    center: Vec3,
    ovel: Vec3,
    reach: f32,
    restitution: f32,
    dead_centre: impl FnOnce() -> Vec3,
) -> Option<Resolved> {
    let d = pos - center;
    let dist2 = d.length_squared();
    if dist2 >= reach * reach {
        return None;
    }
    let dist = dist2.sqrt();
    let normal = if dist > 1e-6 { d / dist } else { dead_centre() };
    let rel = (vel - ovel).dot(normal);
    let mut out_vel = vel;
    let closing_speed = if rel < 0.0 {
        out_vel -= normal * (rel * (1.0 + restitution));
        -rel
    } else {
        0.0
    };
    Some(Resolved {
        target: center + normal * reach,
        vel: out_vel,
        normal,
        closing_speed,
    })
}

/// The first obstacle the swept sphere `origin + dir * t` (unit `dir`, `t`
/// in `[0, max]`, sphere `radius`) enters: `(t, index)`, or `None`.
pub fn segment_hit(
    origin: Vec3,
    dir: Vec3,
    max: f32,
    radius: f32,
    obstacles: &[Obstacle],
) -> Option<(f32, usize)> {
    let mut best: Option<(f32, usize)> = None;
    for (i, o) in obstacles.iter().enumerate() {
        if let Some(t) = segment_sphere(origin, dir, max, o.pos, o.radius + radius) {
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, i));
            }
        }
    }
    best
}

// ---- Drifting boulders (belt levels) ----

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
pub fn push_boulders(
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
        let Some(r) = resolve(
            *pos,
            *vel,
            c,
            o.velocity_at(now),
            o.size + radius,
            settings.restitution,
            || {
                // Dead centre: push outward from the axis.
                let r = c - belt.center;
                Vec3::new(r.x, 0.0, r.z).try_normalize().unwrap_or(Vec3::Y)
            },
        ) else {
            continue;
        };
        *pos = r.target;
        *vel = r.vel;
        worst = worst.max(r.closing_speed);
    }
    worst
}

/// The first boulder a moving sphere (projectile) hits at tick `now`: the
/// segment from `origin` along the unit `dir` for `max` u, swept with
/// `radius`. Returns the distance along `dir` and the contact normal.
pub fn boulder_hit(
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
        let Some(t) = segment_sphere(origin, dir, max, c, o.size + radius) else {
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
pub fn boulder_poses(belt: &BeltSite, now: u64) -> impl Iterator<Item = (Vec3, f32)> + '_ {
    belt.boulders
        .iter()
        .map(move |o| (o.pos_at(belt.center, now), o.size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radiation::tests::plan_belt;
    use ls_math::terrain::{BoxRoomTerrain, PlaneTerrain};
    use ls_math::Aabb;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn duck(pos: Vec3, vel: Vec3) -> Obstacle {
        Obstacle {
            kind: ObstacleKind::Duck,
            pos,
            vel,
            radius: 3.5,
        }
    }

    fn open() -> BoxRoomTerrain {
        BoxRoomTerrain::new(Aabb::new(Vec3::splat(-100.0), Vec3::splat(100.0)))
    }

    #[test]
    fn an_overlapping_sphere_is_pushed_out_and_carried() {
        let t = open();
        let o = [duck(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0))];
        let mut pos = Vec3::new(3.0, 1.0, 0.0);
        let mut vel = Vec3::new(-2.0, 0.0, 1.0);
        let mut out = Vec::new();
        push_sphere(&t, &mut pos, &mut vel, 1.0, &o, &mut out);
        assert_eq!(out.len(), 1);
        assert!((pos.length() - 4.5).abs() < 1e-4, "{pos:?}");
        let n = out[0].normal;
        // Not closing any more: the normal velocity is the duck's.
        assert!(((vel - o[0].vel).dot(n)).abs() < 1e-5);
        assert!(out[0].closing_speed > 0.0);
        // The tangential part is kept.
        assert!((vel.z - 1.0).abs() < 1e-5);
        // A separating sphere keeps its velocity.
        let mut pos2 = Vec3::new(4.0, 0.0, 0.0);
        let mut vel2 = Vec3::new(9.0, 0.0, 0.0);
        out.clear();
        push_sphere(&t, &mut pos2, &mut vel2, 1.0, &o, &mut out);
        assert_eq!(vel2, Vec3::new(9.0, 0.0, 0.0));
        assert_eq!(out[0].closing_speed, 0.0);
        // Out of reach: nothing.
        let mut pos3 = Vec3::new(4.6, 0.0, 0.0);
        let mut vel3 = Vec3::ZERO;
        out.clear();
        push_sphere(&t, &mut pos3, &mut vel3, 1.0, &o, &mut out);
        assert!(out.is_empty());
        assert_eq!(pos3, Vec3::new(4.6, 0.0, 0.0));
    }

    #[test]
    fn the_push_never_drives_a_sphere_into_rock() {
        // Floor at y = 0; the duck comes down on a ship resting on it.
        let floor = PlaneTerrain::floor(0.0, Aabb::new(Vec3::splat(-100.0), Vec3::splat(100.0)));
        let o = [duck(Vec3::new(0.0, 4.0, 0.0), Vec3::new(0.0, -5.0, 0.0))];
        let mut pos = Vec3::new(0.0, 1.0, 0.0);
        let mut vel = Vec3::ZERO;
        let mut out = Vec::new();
        push_sphere(&floor, &mut pos, &mut vel, 1.0, &o, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(pos, Vec3::new(0.0, 1.0, 0.0), "stays out of the floor");
        assert_eq!(vel, Vec3::new(0.0, -5.0, 0.0), "pressed toward it");
    }

    #[test]
    fn segments_hit_the_nearest_obstacle() {
        let o = [
            duck(Vec3::new(20.0, 0.0, 0.0), Vec3::ZERO),
            duck(Vec3::new(10.0, 0.5, 0.0), Vec3::ZERO),
        ];
        let (t, i) = segment_hit(Vec3::ZERO, Vec3::X, 50.0, 0.2, &o).unwrap();
        assert_eq!(i, 1);
        assert!(
            (t - (10.0 - (3.7f32 * 3.7 - 0.25).sqrt())).abs() < 1e-3,
            "{t}"
        );
        assert!(segment_hit(Vec3::ZERO, Vec3::X, 5.0, 0.2, &o).is_none());
        assert!(segment_hit(Vec3::ZERO, Vec3::Y, 50.0, 0.2, &o).is_none());
    }

    // ---- Boulders ----

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
        let hit = push_boulders(&b, s, 10, &mut pos, &mut vel, 0.5);
        assert!((pos.distance(c) - 3.5).abs() < 1e-4, "{pos:?}");
        let vb = b.boulders[0].velocity_at(10);
        // Approach speed along the normal, relative to the boulder.
        assert!((hit - (12.0 + vb.x)).abs() < 1e-3, "{hit}");
        assert!(vel.x > vb.x, "bounces away: {vel:?}");
        // Already separating: pushed out but not slowed.
        let mut pos2 = c + Vec3::new(3.0, 0.0, 0.0);
        let mut vel2 = vb + Vec3::new(5.0, 0.0, 0.0);
        assert_eq!(push_boulders(&b, s, 10, &mut pos2, &mut vel2, 0.5), 0.0);
        assert_eq!(vel2, vb + Vec3::new(5.0, 0.0, 0.0));
        // Far away: untouched.
        let mut pos3 = c + Vec3::new(0.0, 10.0, 0.0);
        let mut vel3 = Vec3::ZERO;
        assert_eq!(push_boulders(&b, s, 10, &mut pos3, &mut vel3, 0.5), 0.0);
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
            if push_boulders(&b, s, tick, &mut pos, &mut vel, 0.5) > 0.0 {
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
        let (t, n) = boulder_hit(&b, 5, origin, Vec3::new(0.0, 0.0, -1.0), 20.0, 0.2).unwrap();
        assert!((t - (10.0 - 3.2)).abs() < 1e-3, "{t}");
        assert!(n.z > 0.99);
        // Too short, or passing beside it.
        assert!(boulder_hit(&b, 5, origin, Vec3::new(0.0, 0.0, -1.0), 5.0, 0.2).is_none());
        let beside = c + Vec3::new(0.0, 4.0, 10.0);
        assert!(boulder_hit(&b, 5, beside, Vec3::new(0.0, 0.0, -1.0), 20.0, 0.2).is_none());
        assert_eq!(boulder_poses(&b, 5).count(), 2);
    }
}
