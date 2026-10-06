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
//! The world collects the tick's obstacles once at the start of the tick
//! (`World::obstacles`; the running comedy event's
//! [`crate::events::EventModule::obstacles`]) and pushes every ship after
//! its movement; the client predictor does the same for the own ship
//! (`ls_game::client::prediction`). IEEE-exact arithmetic only (`sqrt`).

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
        let d = *pos - o.pos;
        let min = o.radius + radius;
        let dist2 = d.length_squared();
        if dist2 >= min * min {
            continue;
        }
        let dist = dist2.sqrt();
        let normal = if dist > 1e-6 {
            d / dist
        } else {
            // Dead centre: out sideways from the obstacle's motion (any
            // fixed direction keeps it deterministic).
            let side = o.vel.cross(Vec3::Y);
            if side.length_squared() > 1e-12 {
                side.normalize()
            } else {
                Vec3::Y
            }
        };
        let rel = (*vel - o.vel).dot(normal);
        let closing_speed = if rel < 0.0 { -rel } else { 0.0 };
        if rel < 0.0 {
            *vel -= normal * rel;
        }
        let target = o.pos + normal * min;
        let clear_now = terrain.sdf(*pos);
        let clear_then = terrain.sdf(target);
        if clear_then >= radius || clear_then >= clear_now {
            *pos = target;
        }
        out.push(Contact {
            index,
            normal,
            closing_speed,
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
