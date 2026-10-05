//! The own Leech Arc beam, drawn locally (online).
//!
//! The Leech Arc is the only instant weapon: it fires no projectile, and the
//! server reports its arcs only as `CueKind::Beam` cues, which reach the
//! shooter a full round trip after the click. The shot predictor
//! ([`crate::shots::ShotPredictor::last_beam`]) already knows when the own
//! arcs fire; [`aim_beam`] mirrors the weapon module's targeting
//! (`ls_sim::weapons::energy::leech_arc`: lock the nearest enemy within
//! `range` inside a cone of `lock_half_angle_deg` with line of sight, keep
//! the lock inside the wider `lock_keep_half_angle_deg` cone; without a
//! target the arc grounds on the rock ahead or ends at full range) on the
//! ships as the client draws them, so the beam appears with the click and
//! follows the aim. The server's own beam cues then only correct the lock.
//!
//! Nothing here is game state: it only affects what is drawn.

use ls_math::{det, Vec3};
use ls_sim::WeaponDef;

use crate::game::ShipView;

/// Targeting parameters of a beam weapon (from its data).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamParams {
    pub range: f32,
    /// Cosine of the lock cone's half angle.
    pub lock_cos: f32,
    /// Cosine of the half angle within which a lock is kept.
    pub keep_cos: f32,
}

impl BeamParams {
    pub fn of(def: &WeaponDef) -> Self {
        let cos = |deg: f32| det::cos(deg.to_radians());
        BeamParams {
            range: def.param("range", 12.0),
            lock_cos: cos(def.param("lock_half_angle_deg", 30.0)),
            keep_cos: cos(def.param("lock_keep_half_angle_deg", 60.0)),
        }
    }
}

/// A beam to draw: from the shooter to the locked ship or the arc's end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalBeam {
    pub from: Vec3,
    pub to: Vec3,
    /// Locked ship id, if any.
    pub target: Option<u8>,
}

/// Aims the own beam from `from` along the unit `forward`: keeps `lock`
/// while it holds, else locks the nearest enemy (team other than `team`)
/// in the cone. `clear(a, b)` says whether rock leaves the line of sight
/// from `a` to `b` free; `ground(origin, dir, max)` is the distance to the
/// rock along a ray, if any.
#[allow(clippy::too_many_arguments)]
pub fn aim_beam(
    from: Vec3,
    forward: Vec3,
    team: u8,
    ships: &[ShipView],
    lock: Option<u8>,
    p: BeamParams,
    clear: impl Fn(Vec3, Vec3) -> bool,
    ground: impl Fn(Vec3, Vec3, f32) -> Option<f32>,
) -> LocalBeam {
    let in_cone = |s: &ShipView, cos: f32| {
        let to = s.pos - from;
        let d = to.length();
        s.alive && s.team != team && d <= p.range && d > 1e-4 && to.dot(forward) >= cos * d
    };
    let kept = lock
        .and_then(|id| ships.iter().find(|s| s.id == id))
        .filter(|s| in_cone(s, p.keep_cos) && clear(from, s.pos));
    let target = kept.or_else(|| {
        ships
            .iter()
            .filter(|s| in_cone(s, p.lock_cos) && clear(from, s.pos))
            .min_by(|a, b| {
                (a.pos - from)
                    .length_squared()
                    .total_cmp(&(b.pos - from).length_squared())
            })
    });
    match target {
        Some(s) => LocalBeam {
            from,
            to: s.pos,
            target: Some(s.id),
        },
        None => {
            let reach = ground(from, forward, p.range).unwrap_or(p.range);
            LocalBeam {
                from,
                to: from + forward * reach,
                target: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::Quat;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn ship(id: u8, team: u8, pos: Vec3) -> ShipView {
        ShipView {
            id,
            team,
            alive: true,
            boosting: false,
            pos,
            rot: Quat::IDENTITY,
            ship_type: 1,
            radius: 0.5,
            gadget_active: false,
        }
    }

    fn params() -> BeamParams {
        BeamParams {
            range: 12.0,
            lock_cos: det::cos(30f32.to_radians()),
            keep_cos: det::cos(60f32.to_radians()),
        }
    }

    const FWD: Vec3 = Vec3::new(0.0, 0.0, -1.0);

    #[test]
    fn locks_the_nearest_enemy_in_the_cone() {
        let ships = [
            ship(1, 1, Vec3::new(0.0, 0.0, -9.0)),
            ship(2, 1, Vec3::new(1.0, 0.0, -5.0)),
            // A teammate closer still, an enemy outside the cone and one out of range.
            ship(3, 0, Vec3::new(0.0, 0.0, -2.0)),
            ship(4, 1, Vec3::new(4.0, 0.0, -3.0)),
            ship(5, 1, Vec3::new(0.0, 0.0, -13.0)),
        ];
        let b = aim_beam(
            Vec3::ZERO,
            FWD,
            0,
            &ships,
            None,
            params(),
            |_, _| true,
            |_, _, _| None,
        );
        assert_eq!(b.target, Some(2));
        assert_eq!(b.to, ships[1].pos);
    }

    #[test]
    fn keeps_a_lock_in_the_wider_cone_and_drops_it_behind_rock() {
        // 45° off the aim: outside the lock cone, inside the keep cone.
        let ships = [
            ship(7, 1, Vec3::new(4.0, 0.0, -4.0)),
            ship(8, 1, Vec3::new(0.0, 0.0, -8.0)),
        ];
        let b = aim_beam(
            Vec3::ZERO,
            FWD,
            0,
            &ships,
            Some(7),
            params(),
            |_, _| true,
            |_, _, _| None,
        );
        assert_eq!(b.target, Some(7));
        let fresh = aim_beam(
            Vec3::ZERO,
            FWD,
            0,
            &ships,
            None,
            params(),
            |_, _| true,
            |_, _, _| None,
        );
        assert_eq!(fresh.target, Some(8));
        let blocked = aim_beam(
            Vec3::ZERO,
            FWD,
            0,
            &ships,
            Some(7),
            params(),
            |_, to| to != ships[0].pos,
            |_, _, _| None,
        );
        assert_eq!(blocked.target, Some(8));
    }

    #[test]
    fn without_a_target_the_arc_grounds_or_ends_at_full_range() {
        let b = aim_beam(
            Vec3::ZERO,
            FWD,
            0,
            &[],
            None,
            params(),
            |_, _| true,
            |_, _, _| Some(4.5),
        );
        assert_eq!((b.target, b.to), (None, Vec3::new(0.0, 0.0, -4.5)));
        let b = aim_beam(
            Vec3::ZERO,
            FWD,
            0,
            &[],
            None,
            params(),
            |_, _| true,
            |_, _, _| None,
        );
        assert_eq!(b.to, Vec3::new(0.0, 0.0, -12.0));
    }
}
