//! Aiming: lead for projectile travel time, fire tolerances and line of
//! sight. Projectiles never inherit the shooter's velocity (design rule), so
//! the lead depends only on the target's motion and the muzzle speed.

use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::Vec3;

/// Time (s) until a projectile fired now at `speed` from the origin meets a
/// target at `rel_pos` moving with constant `target_vel`, or `None` if it
/// can never catch it.
///
/// Solves `|rel_pos + target_vel * t| = speed * t` for the smallest `t > 0`.
pub fn intercept_time(rel_pos: Vec3, target_vel: Vec3, speed: f32) -> Option<f32> {
    if speed <= 0.0 {
        return None;
    }
    let a = target_vel.dot(target_vel) - speed * speed;
    let b = 2.0 * rel_pos.dot(target_vel);
    let c = rel_pos.dot(rel_pos);
    if c < 1e-8 {
        return Some(0.0);
    }
    if a.abs() < 1e-6 {
        // Target as fast as the projectile: linear equation b t + c = 0.
        return (b < 0.0).then(|| -c / b).filter(|t| *t > 0.0);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t1 = (-b - sq) / (2.0 * a);
    let t2 = (-b + sq) / (2.0 * a);
    let (lo, hi) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
    if lo > 0.0 {
        Some(lo)
    } else if hi > 0.0 {
        Some(hi)
    } else {
        None
    }
}

/// Point to aim at so a projectile of `speed` fired from `shooter` hits a
/// target at `target` moving with `target_vel`. `None` if it cannot catch
/// the target within `max_time` seconds (the projectile's lifetime).
pub fn lead_point(
    shooter: Vec3,
    target: Vec3,
    target_vel: Vec3,
    speed: f32,
    max_time: f32,
) -> Option<Vec3> {
    let t = intercept_time(target - shooter, target_vel, speed)?;
    (t <= max_time).then(|| target + target_vel * t)
}

/// Largest angle (rad) between the nose and the aim direction at which a
/// shot at `distance` still passes within `hit_radius` of the aim point
/// (with a 10 % margin, and at least 3 mrad so far targets stay hittable).
pub fn aim_tolerance(distance: f32, hit_radius: f32) -> f32 {
    if distance <= hit_radius {
        return std::f32::consts::FRAC_PI_4;
    }
    ((hit_radius / distance).atan() * 0.9).max(0.003)
}

/// True if a projectile of radius `radius` flying from `from` to `to`
/// would not hit rock first (a small margin at the far end allows aiming at
/// a surface point).
pub fn clear_shot<T: TerrainQuery + ?Sized>(
    terrain: &T,
    from: Vec3,
    to: Vec3,
    radius: f32,
) -> bool {
    let d = to - from;
    let dist = d.length();
    if dist < 1e-3 {
        return true;
    }
    match terrain.raycast(from, d, dist) {
        None => true,
        Some(hit) => hit.distance + radius + 0.5 >= dist,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::ls_math::terrain::{BoxRoomTerrain, TerrainEdit};

    /// Where a projectile fired at the lead point is at the intercept time,
    /// compared with where the target is then.
    fn miss_distance(shooter: Vec3, target: Vec3, vel: Vec3, speed: f32) -> f32 {
        let aim = lead_point(shooter, target, vel, speed, 100.0).unwrap();
        let t = intercept_time(target - shooter, vel, speed).unwrap();
        let shot = shooter + (aim - shooter).normalize() * (speed * t);
        shot.distance(target + vel * t)
    }

    #[test]
    fn stationary_target_is_aimed_at_directly() {
        let s = Vec3::new(1.0, 2.0, 3.0);
        let t = Vec3::new(11.0, 2.0, 3.0);
        assert_eq!(lead_point(s, t, Vec3::ZERO, 30.0, 3.0), Some(t));
        let time = intercept_time(t - s, Vec3::ZERO, 30.0).unwrap();
        assert!((time - 10.0 / 30.0).abs() < 1e-6);
    }

    #[test]
    fn moving_target_is_hit_at_the_intercept() {
        let cases = [
            (Vec3::new(0.0, 0.0, -20.0), Vec3::new(12.0, 0.0, 0.0), 30.0),
            (Vec3::new(5.0, 3.0, -40.0), Vec3::new(-10.0, 4.0, 6.0), 30.0),
            // Target flying straight at the shooter.
            (Vec3::new(0.0, 0.0, -30.0), Vec3::new(0.0, 0.0, 16.0), 40.0),
            // Target fleeing, slower than the projectile.
            (Vec3::new(0.0, 0.0, -10.0), Vec3::new(0.0, 0.0, -25.0), 30.0),
            // Fast rivet against a boosting target.
            (
                Vec3::new(30.0, -5.0, 10.0),
                Vec3::new(16.5, 16.5, -16.5),
                180.0,
            ),
        ];
        for (target, vel, speed) in cases {
            let miss = miss_distance(Vec3::ZERO, target, vel, speed);
            assert!(miss < 1e-3, "{target:?} {vel:?} {speed}: miss {miss}");
            // The lead is ahead of the target along its velocity.
            let aim = lead_point(Vec3::ZERO, target, vel, speed, 100.0).unwrap();
            assert!((aim - target).dot(vel) >= 0.0);
        }
    }

    #[test]
    fn uncatchable_targets_have_no_lead() {
        // Fleeing faster than the projectile.
        assert_eq!(
            intercept_time(Vec3::new(0.0, 0.0, -10.0), Vec3::new(0.0, 0.0, -35.0), 30.0),
            None
        );
        // As fast as the projectile and fleeing.
        assert_eq!(
            intercept_time(Vec3::new(0.0, 0.0, -10.0), Vec3::new(0.0, 0.0, -30.0), 30.0),
            None
        );
        // As fast, but approaching: linear case.
        let t = intercept_time(Vec3::new(0.0, 0.0, -30.0), Vec3::new(0.0, 0.0, 30.0), 30.0);
        assert!((t.unwrap() - 0.5).abs() < 1e-5);
        // Out of range for the projectile's lifetime (Paint Thrower: 20 u/s, 1.5 s).
        assert_eq!(
            lead_point(
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, -40.0),
                Vec3::ZERO,
                20.0,
                1.5
            ),
            None
        );
        assert_eq!(intercept_time(Vec3::X, Vec3::ZERO, 0.0), None);
    }

    #[test]
    fn tolerance_shrinks_with_distance() {
        let near = aim_tolerance(5.0, 0.65);
        let far = aim_tolerance(50.0, 0.65);
        assert!(near > far && far > 0.0);
        // A shot inside the tolerance passes within the hit radius.
        let d = 30.0;
        let off = d * aim_tolerance(d, 0.65).tan();
        assert!(off < 0.65, "{off}");
        assert_eq!(aim_tolerance(0.5, 0.65), std::f32::consts::FRAC_PI_4);
    }

    #[test]
    fn clear_shot_respects_rock() {
        let mut t = BoxRoomTerrain::centered(Vec3::splat(60.0));
        let a = Vec3::new(0.0, 0.0, -20.0);
        let b = Vec3::new(0.0, 0.0, 20.0);
        assert!(clear_shot(&t, a, b, 0.2));
        // Aiming at a wall point counts as clear.
        assert!(clear_shot(&t, Vec3::ZERO, Vec3::new(30.0, 0.0, 0.0), 0.2));
        t.apply(TerrainEdit::AddSphere {
            center: Vec3::ZERO,
            radius: 4.0,
        });
        assert!(!clear_shot(&t, a, b, 0.2));
    }
}
