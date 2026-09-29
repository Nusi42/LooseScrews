//! Movement helpers: wall avoidance through [`TerrainQuery`] (SDF probes and
//! raycasts), conversion of a desired world velocity into ship-local thrust,
//! look deltas that turn the nose towards a direction, and open-space
//! target picking.
//!
//! Bots are not part of the deterministic simulation, so these helpers may
//! use `std` float functions; everything they output goes through the
//! normal, quantized input path.

use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Aabb, Pcg32, Quat, Vec3};

/// Tuning of [`avoid_walls`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SteerConfig {
    /// Gap (u) the bot tries to keep between its centre and rock.
    pub clearance: f32,
    /// Shortest look-ahead distance (u) for the desired-direction raycast.
    pub lookahead_min: f32,
    /// Look-ahead grows with speed by this many seconds of travel.
    pub lookahead_secs: f32,
    /// Longest look-ahead distance (u).
    pub lookahead_max: f32,
    /// Deceleration (u/s^2) the ship can brake with (Tinker: 40); sets how
    /// far ahead of the actual motion rock matters.
    pub brake_decel: f32,
    /// Extra distance (u) on top of the stopping distance.
    pub brake_margin: f32,
}

impl Default for SteerConfig {
    fn default() -> Self {
        SteerConfig {
            clearance: 3.0,
            lookahead_min: 2.0,
            lookahead_secs: 0.6,
            lookahead_max: 16.0,
            brake_decel: 40.0,
            brake_margin: 2.5,
        }
    }
}

impl SteerConfig {
    /// Look-ahead distance at `speed` u/s.
    pub fn lookahead(&self, speed: f32) -> f32 {
        (self.lookahead_min + speed * self.lookahead_secs).min(self.lookahead_max)
    }

    /// Distance needed to stop from `speed` u/s, plus the margin.
    pub fn stopping_distance(&self, speed: f32) -> f32 {
        speed * speed / (2.0 * self.brake_decel.max(1.0)) + self.brake_margin
    }
}

/// Result of [`avoid_walls`]: the direction to fly and how much speed is
/// safe (fraction `0..=1` of the requested speed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steer {
    /// Unit direction, or zero if the bot should just brake.
    pub dir: Vec3,
    /// Fraction of the requested speed that is safe right now.
    pub speed_scale: f32,
}

/// Bends the desired flight direction away from terrain.
///
/// 1. Raycast along the desired direction (look-ahead grows with speed): if
///    rock is close, slide along its surface and push off its normal the
///    closer it is. Skipped when the goal (`goal_dist` away along `desired`,
///    `f32::INFINITY` if there is none) lies at least `clearance` before the
///    rock, so bots can still reach points near walls.
/// 2. Raycast along the current velocity over the stopping distance: rock
///    there pushes along its normal and asks for less speed, so the bot
///    brakes before a wall it is drifting into.
/// 3. SDF probe at the current position: closer than `clearance` to any
///    rock, the SDF gradient pushes straight out.
///
/// A zero `desired` still gets the repulsion terms, so a hovering bot backs
/// away from walls.
pub fn avoid_walls<T: TerrainQuery + ?Sized>(
    terrain: &T,
    pos: Vec3,
    vel: Vec3,
    desired: Vec3,
    goal_dist: f32,
    cfg: &SteerConfig,
) -> Steer {
    let speed = vel.length();
    let c = cfg.clearance;
    let mut dir = desired.normalize_or_zero();
    let mut speed_scale: f32 = 1.0;

    // 1. Obstacle along the desired direction: slide along it.
    if dir != Vec3::ZERO {
        let look = cfg.lookahead(speed.max(desired.length()));
        if let Some(hit) = terrain.raycast(pos, dir, look + c) {
            let goal_is_safe = hit.distance >= goal_dist + 0.8 * c;
            if !goal_is_safe {
                let k = (1.0 - (hit.distance - c).max(0.0) / look).clamp(0.0, 1.0);
                let mut tangent = dir.reject_from_normal(hit.normal).normalize_or_zero();
                if tangent == Vec3::ZERO {
                    // Head-on: pick any direction along the wall.
                    tangent = hit.normal.any_orthonormal();
                }
                dir = (tangent * (1.0 - 0.5 * k) + hit.normal * (1.5 * k)).normalize_or_zero();
            }
        }
    }

    // 2. Obstacle along the current motion: push off and slow down.
    if speed > 0.5 {
        let stop = cfg.stopping_distance(speed);
        if let Some(hit) = terrain.raycast(pos, vel, stop + c) {
            let k = (1.0 - (hit.distance - c).max(0.0) / stop).clamp(0.0, 1.0);
            dir += hit.normal * (2.0 * k);
            speed_scale = speed_scale.min(1.0 - 0.7 * k);
        }
    }

    // 3. Too close to rock in any direction: push straight out.
    let d = terrain.sdf(pos);
    if d < c {
        let n = terrain.normal(pos);
        let w = ((c - d) / c).clamp(0.0, 2.0);
        dir += n * (2.0 * w);
    }

    Steer {
        dir: dir.normalize_or_zero(),
        speed_scale: speed_scale.clamp(0.2, 1.0),
    }
}

/// Ship-local thrust (each axis `-1..=1`) that makes the ship approach the
/// world velocity `desired`. The flight model treats thrust as a per-axis
/// velocity target (`thrust * cap`), so the command is simply the desired
/// velocity in ship axes divided by the speed cap.
pub fn thrust_for_velocity(orientation: Quat, desired: Vec3, cap: f32) -> Vec3 {
    if cap <= 0.0 {
        return Vec3::ZERO;
    }
    let local = orientation.conjugate().rotate(desired) / cap;
    let axis = |v: f32| {
        let v = v.clamp(-1.0, 1.0);
        // Tiny commands would only fight damping; let the ship coast.
        if v.abs() < 0.02 {
            0.0
        } else {
            v
        }
    };
    Vec3::new(axis(local.x), axis(local.y), axis(local.z))
}

/// Velocity that flies towards `target` and slows down on arrival:
/// full `speed` far away, proportional to the distance within `slow_radius`.
pub fn arrive(pos: Vec3, target: Vec3, speed: f32, slow_radius: f32) -> Vec3 {
    let to = target - pos;
    let dist = to.length();
    if dist < 1e-3 {
        return Vec3::ZERO;
    }
    let s = if dist < slow_radius {
        speed * dist / slow_radius
    } else {
        speed
    };
    to * (s / dist)
}

/// Look deltas for one tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Pitch delta (rad, positive = nose up).
    pub pitch: f32,
    /// Yaw delta (rad, positive = nose left).
    pub yaw: f32,
    /// Angle (rad) between the current nose and the requested direction.
    pub error: f32,
}

/// Pitch and yaw deltas (each clamped to `max_step` radians) that turn the
/// nose (`-Z`) of `orientation` towards the world direction `dir`.
pub fn look_toward(orientation: Quat, dir: Vec3, max_step: f32) -> Look {
    let Some(dir) = dir.try_normalize() else {
        return Look {
            pitch: 0.0,
            yaw: 0.0,
            error: 0.0,
        };
    };
    let local = orientation.conjugate().rotate(dir);
    // Forward is -Z. Positive yaw (about +Y) swings the nose towards -X,
    // positive pitch (about +X) swings it towards +Y.
    let yaw = (-local.x).atan2(-local.z);
    let horizontal = (local.x * local.x + local.z * local.z).sqrt();
    let pitch = local.y.atan2(horizontal);
    let error = angle_between(orientation.forward(), dir);
    Look {
        pitch: pitch.clamp(-max_step, max_step),
        yaw: yaw.clamp(-max_step, max_step),
        error,
    }
}

/// Angle (rad) between two non-zero vectors.
pub fn angle_between(a: Vec3, b: Vec3) -> f32 {
    let (Some(a), Some(b)) = (a.try_normalize(), b.try_normalize()) else {
        return 0.0;
    };
    a.dot(b).clamp(-1.0, 1.0).acos()
}

/// A random point inside `bounds` (shrunk by `margin`) with at least
/// `clearance` of open space around it, trying `attempts` times.
pub fn random_open_point<T: TerrainQuery + ?Sized>(
    terrain: &T,
    rng: &mut Pcg32,
    bounds: Aabb,
    margin: f32,
    clearance: f32,
    attempts: u32,
) -> Option<Vec3> {
    let lo = bounds.min + Vec3::splat(margin);
    let hi = bounds.max - Vec3::splat(margin);
    if lo.x >= hi.x || lo.y >= hi.y || lo.z >= hi.z {
        return None;
    }
    (0..attempts).find_map(|_| {
        let p = Vec3::new(
            rng.range_f32(lo.x, hi.x),
            rng.range_f32(lo.y, hi.y),
            rng.range_f32(lo.z, hi.z),
        );
        (terrain.sdf(p) >= clearance).then_some(p)
    })
}

/// A random unit vector (uniform on the sphere).
pub fn random_unit(rng: &mut Pcg32) -> Vec3 {
    loop {
        let v = Vec3::new(
            rng.range_f32(-1.0, 1.0),
            rng.range_f32(-1.0, 1.0),
            rng.range_f32(-1.0, 1.0),
        );
        let l2 = v.length_squared();
        if l2 > 1e-4 && l2 <= 1.0 {
            return v / l2.sqrt();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::ls_math::terrain::{BoxRoomTerrain, TerrainEdit};
    use ls_game::ls_math::DT;
    use ls_game::ls_sim::{simulate_movement, InputCmd, PlayerId, ShipState, ShipTuning, ShipType};

    fn room() -> BoxRoomTerrain {
        BoxRoomTerrain::centered(Vec3::new(60.0, 40.0, 60.0))
    }

    fn ship_at(pos: Vec3, orientation: Quat) -> ShipState {
        ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            &ShipTuning::tinker(),
            pos,
            orientation,
        )
    }

    #[test]
    fn open_space_keeps_the_desired_direction() {
        let t = room();
        let desired = Vec3::new(1.0, 0.0, 1.0);
        let s = avoid_walls(
            &t,
            Vec3::ZERO,
            Vec3::ZERO,
            desired,
            f32::INFINITY,
            &SteerConfig::default(),
        );
        assert!(angle_between(s.dir, desired) < 1e-4, "{s:?}");
        assert_eq!(s.speed_scale, 1.0);
    }

    #[test]
    fn heading_into_a_wall_turns_away_from_it() {
        let t = room();
        // Wall at x = +30; bot 5 u from it, flying at it.
        let pos = Vec3::new(25.0, 0.0, 0.0);
        let vel = Vec3::new(12.0, 0.0, 0.0);
        let s = avoid_walls(
            &t,
            pos,
            vel,
            Vec3::X,
            f32::INFINITY,
            &SteerConfig::default(),
        );
        assert!(s.dir.x < 0.0, "must steer away from the wall: {s:?}");
        assert!(s.speed_scale < 1.0, "must slow down: {s:?}");
    }

    #[test]
    fn oblique_approach_slides_along_the_wall() {
        let t = room();
        let pos = Vec3::new(26.0, 0.0, 0.0);
        let desired = Vec3::new(1.0, 0.0, 1.0);
        let s = avoid_walls(
            &t,
            pos,
            Vec3::ZERO,
            desired,
            f32::INFINITY,
            &SteerConfig::default(),
        );
        // Keeps going along +Z, loses (most of) the component into the wall.
        assert!(s.dir.z > 0.5, "{s:?}");
        assert!(s.dir.x < desired.normalize().x, "{s:?}");
    }

    #[test]
    fn hugging_a_wall_pushes_out_even_without_a_goal() {
        let t = room();
        let pos = Vec3::new(0.0, -19.0, 0.0); // 1 u above the floor
        let s = avoid_walls(
            &t,
            pos,
            Vec3::ZERO,
            Vec3::ZERO,
            f32::INFINITY,
            &SteerConfig::default(),
        );
        assert!(s.dir.y > 0.9, "{s:?}");
    }

    #[test]
    fn obstacle_in_the_path_is_avoided_in_flight() {
        // Fly across the room towards a target hidden behind a rock sphere;
        // with steering the ship must never touch rock.
        let mut t = room();
        t.apply(TerrainEdit::AddSphere {
            center: Vec3::ZERO,
            radius: 6.0,
        });
        let tuning = ShipTuning::tinker();
        let cfg = SteerConfig::default();
        let target = Vec3::new(0.0, 0.0, 25.0);
        let mut ship = ship_at(Vec3::new(0.0, 0.0, -25.0), Quat::IDENTITY);
        let mut reached = false;
        for _ in 0..60 * 20 {
            let desired = arrive(ship.pos, target, tuning.cruise_speed, 6.0);
            let goal = ship.pos.distance(target);
            let s = avoid_walls(&t, ship.pos, ship.vel, desired, goal, &cfg);
            let vel = s.dir * (desired.length() * s.speed_scale);
            let cmd = InputCmd::idle(0).with_thrust(thrust_for_velocity(
                ship.orientation,
                vel,
                tuning.cruise_speed,
            ));
            let report = simulate_movement(&mut ship, &cmd, &tuning, &t);
            assert!(!report.contact, "touched rock at {:?}", ship.pos);
            if ship.pos.distance(target) < 2.0 {
                reached = true;
                break;
            }
        }
        assert!(reached, "never reached the target, ended at {:?}", ship.pos);
    }

    #[test]
    fn wall_approach_at_boost_speed_does_not_crash() {
        let t = room();
        let tuning = ShipTuning::tinker();
        let cfg = SteerConfig::default();
        let mut ship = ship_at(Vec3::ZERO, Quat::IDENTITY);
        ship.vel = Vec3::new(16.0, 0.0, 0.0);
        for _ in 0..60 * 5 {
            // Keep asking to fly into the +X wall.
            let desired = Vec3::X * tuning.cruise_speed;
            let s = avoid_walls(&t, ship.pos, ship.vel, desired, f32::INFINITY, &cfg);
            let vel = s.dir * (desired.length() * s.speed_scale);
            let cmd = InputCmd::idle(0).with_thrust(thrust_for_velocity(
                ship.orientation,
                vel,
                tuning.cruise_speed,
            ));
            let report = simulate_movement(&mut ship, &cmd, &tuning, &t);
            assert!(!report.contact, "touched rock at {:?}", ship.pos);
        }
        assert!(ship.pos.x < 30.0 - 1.0);
    }

    #[test]
    fn thrust_is_desired_velocity_in_ship_axes() {
        // Identity: forward is -Z, so flying to -Z at cruise speed is W.
        let t = thrust_for_velocity(Quat::IDENTITY, Vec3::new(0.0, 0.0, -12.0), 12.0);
        assert_eq!(t, Vec3::new(0.0, 0.0, -1.0));
        // Yawed 90 degrees left: forward is -X.
        let q = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2);
        let t = thrust_for_velocity(q, Vec3::new(-6.0, 0.0, 0.0), 12.0);
        assert!((t.z + 0.5).abs() < 1e-5 && t.x.abs() < 1e-5, "{t:?}");
        // Clamped per axis.
        let t = thrust_for_velocity(Quat::IDENTITY, Vec3::new(100.0, -100.0, 0.0), 12.0);
        assert_eq!(t, Vec3::new(1.0, -1.0, 0.0));
    }

    #[test]
    fn thrust_reaches_the_desired_velocity() {
        let t = room();
        let tuning = ShipTuning::tinker();
        let q = Quat::from_axis_angle(Vec3::new(0.3, 1.0, 0.2).normalize(), 1.1);
        let mut ship = ship_at(Vec3::ZERO, q);
        let desired = Vec3::new(5.0, -3.0, 4.0);
        for _ in 0..90 {
            let cmd = InputCmd::idle(0).with_thrust(thrust_for_velocity(
                ship.orientation,
                desired,
                tuning.cruise_speed,
            ));
            simulate_movement(&mut ship, &cmd, &tuning, &t);
        }
        assert!(ship.vel.distance(desired) < 0.3, "{:?}", ship.vel);
    }

    #[test]
    fn look_signs_follow_the_axis_conventions() {
        let max = 1.0;
        // Target to the left (-X): positive yaw.
        let l = look_toward(Quat::IDENTITY, Vec3::new(-1.0, 0.0, -1.0), max);
        assert!(l.yaw > 0.7 && l.pitch.abs() < 1e-5, "{l:?}");
        // Target above: positive pitch.
        let l = look_toward(Quat::IDENTITY, Vec3::new(0.0, 1.0, -1.0), max);
        assert!(l.pitch > 0.7 && l.yaw.abs() < 1e-5, "{l:?}");
        // Target behind: full-rate turn.
        let l = look_toward(Quat::IDENTITY, Vec3::new(0.1, 0.0, 1.0), 0.2);
        assert_eq!(l.yaw.abs(), 0.2);
        assert!(l.error > 3.0);
        // On target: nothing to do.
        let l = look_toward(Quat::IDENTITY, Vec3::NEG_Z, max);
        assert!(l.yaw.abs() < 1e-6 && l.pitch.abs() < 1e-6 && l.error < 1e-3);
    }

    #[test]
    fn look_converges_through_the_flight_model() {
        let t = room();
        let tuning = ShipTuning::tinker();
        let max_step = tuning.max_turn_rate * DT;
        for target in [
            Vec3::new(1.0, 0.2, 0.3),
            Vec3::new(-0.2, -0.9, 0.1),
            Vec3::new(0.0, 0.1, 1.0),   // behind
            Vec3::new(0.3, 0.95, -0.1), // nearly straight up
        ] {
            let mut ship = ship_at(Vec3::ZERO, Quat::IDENTITY);
            let mut buttons = ls_game::ls_sim::Buttons::HORIZON_ASSIST;
            let mut error = f32::MAX;
            for i in 0..120 {
                let l = look_toward(ship.orientation, target, max_step);
                error = l.error;
                if i == 60 {
                    // Horizon assist must not keep the nose off target.
                    buttons = ls_game::ls_sim::Buttons::NONE;
                }
                let cmd = InputCmd {
                    look_pitch: l.pitch,
                    look_yaw: l.yaw,
                    buttons,
                    ..InputCmd::idle(0)
                };
                simulate_movement(&mut ship, &cmd, &tuning, &t);
            }
            assert!(error < 0.02, "target {target:?}: error {error}");
            let final_error = angle_between(ship.orientation.forward(), target);
            assert!(final_error < 0.02, "target {target:?}: {final_error}");
        }
    }

    #[test]
    fn arrive_slows_down_near_the_target() {
        let v = arrive(Vec3::ZERO, Vec3::new(0.0, 0.0, 100.0), 12.0, 8.0);
        assert!((v.length() - 12.0).abs() < 1e-4);
        let v = arrive(Vec3::ZERO, Vec3::new(0.0, 4.0, 0.0), 12.0, 8.0);
        assert!((v.length() - 6.0).abs() < 1e-4);
        assert_eq!(arrive(Vec3::ONE, Vec3::ONE, 12.0, 8.0), Vec3::ZERO);
    }

    #[test]
    fn random_open_points_have_clearance() {
        let mut t = room();
        t.apply(TerrainEdit::AddSphere {
            center: Vec3::ZERO,
            radius: 10.0,
        });
        let mut rng = Pcg32::from_seed(1, 2);
        for _ in 0..50 {
            let p = random_open_point(&t, &mut rng, t.bounds(), 2.0, 3.0, 32).unwrap();
            assert!(t.sdf(p) >= 3.0);
            let u = random_unit(&mut rng);
            assert!((u.length() - 1.0).abs() < 1e-5);
        }
    }
}
