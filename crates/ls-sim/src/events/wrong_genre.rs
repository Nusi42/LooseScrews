//! Wrong genre (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! For 10 s the match is a side-scroller:
//! - every ship is locked to the plane through its position perpendicular to
//!   z (`move_effects` plane lock: z velocity and z movement removed);
//! - its facing snaps to +x or -x (whichever is nearer) over
//!   `face_snap_secs` (0.3 s), roll levelled; after that the ship turns
//!   only inside the plane (pitch; yaw and roll out of it are cancelled), so
//!   it turns around by looping;
//! - new projectiles fly in the plane (`proj_flags::PLANAR`: z velocity
//!   removed at spawn and every tick, for their whole flight);
//! - hits between ships and projectiles ignore depth (`hit_plane`: 2D
//!   distance in x and y); terrain stays 3D.
//!
//! Everything is a pure function of the mood view, so the client predictor
//! applies the same lock and facing.
//!
//! Numbers: `data/events/wrong_genre.ron`.

use ls_math::{Mat3, Quat, Tick, Vec3};

use crate::events::{EffectCtx, EventKind, EventModule, Facing, MoveEffects};
use crate::ship::ShipState;
use crate::world::proj_flags;

/// The plane normal (the depth axis).
pub const DEPTH: Vec3 = Vec3::Z;

/// Wrong genre.
#[derive(Debug)]
pub struct WrongGenre;

impl WrongGenre {
    /// Level orientation facing +x (`sign` >= 0) or -x: right along +z or
    /// -z, up +y.
    pub fn side_facing(sign: f32) -> Quat {
        let s = if sign < 0.0 { -1.0 } else { 1.0 };
        let fwd = Vec3::X * s;
        let right = DEPTH * s;
        let back = -fwd;
        Quat::from_mat3(&Mat3::from_cols(right, back.cross(right), back))
    }
}

impl EventModule for WrongGenre {
    fn kind(&self) -> EventKind {
        EventKind::WrongGenre
    }

    fn implemented(&self) -> bool {
        true
    }

    fn move_effects(&self, fx: &EffectCtx, ship: &ShipState) -> MoveEffects {
        let snap = Tick::ticks_for_secs(fx.def.param("face_snap_secs", 0.3)).max(1);
        let k = fx.active_for();
        let facing = if k <= snap {
            // Tick k of the snap turns 1 / (ticks left) of the way, so the
            // last one lands exactly on the side facing.
            Facing::Toward {
                target: Self::side_facing(ship.orientation.forward().x),
                blend: 1.0 / (snap + 1 - k.max(1)) as f32,
            }
        } else {
            Facing::InPlane { normal: DEPTH }
        };
        MoveEffects {
            lock_normal: Some(DEPTH),
            facing,
            ..MoveEffects::NONE
        }
    }

    fn projectile_flags(&self, _fx: &EffectCtx) -> u16 {
        proj_flags::PLANAR
    }

    fn hit_plane(&self, _fx: &EffectCtx) -> Option<Vec3> {
        Some(DEPTH)
    }
}
