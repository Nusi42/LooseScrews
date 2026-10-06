//! Gossip wall (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! When the event is announced a face starts growing on the rock surface
//! nearest the centroid of the living ships ([`drone::rock_face`]: from
//! open space down the distance field, from inside rock out along the
//! normal; the level centre when nobody is alive). For the 15 s active
//! phase every ship's position is revealed to everyone, cloaked ships too
//! (`reveal`, read by snapshot relevance: `World::ship_visible_to`). The
//! face sinks back during the 1 s ending phase.
//!
//! Face entity (`ReplicatedKind::GossipFace`): position on the rock, `rot`
//! forward = the rock's outward normal (the face looks out), `radius` =
//! face size, `state` = `[talking (1 while the reveal runs), 0, 0, 0]`.
//!
//! Numbers: `data/events/gossip_wall.ron`.

use crate::events::drone;
use crate::events::{EffectCtx, EventCtx, EventKind, EventModule, EventPhase};
use crate::ShipState;

/// Gossip wall.
#[derive(Debug)]
pub struct GossipWall;

/// Where the face grows for living ships at `positions` (the level centre
/// `fallback` without any): the rock face nearest their centroid.
pub fn face_site<T: ls_math::terrain::TerrainQuery + ?Sized>(
    terrain: &T,
    positions: &[ls_math::Vec3],
    fallback: ls_math::Vec3,
) -> (ls_math::Vec3, ls_math::Vec3) {
    let centroid = if positions.is_empty() {
        fallback
    } else {
        positions.iter().fold(ls_math::Vec3::ZERO, |a, p| a + *p) / positions.len() as f32
    };
    drone::rock_face(terrain, centroid)
}

impl EventModule for GossipWall {
    fn kind(&self) -> EventKind {
        EventKind::GossipWall
    }

    fn implemented(&self) -> bool {
        true
    }

    /// The face starts growing.
    fn on_announce(&self, e: &mut EventCtx) {
        let view = e.ctx.view;
        let living: Vec<ls_math::Vec3> = view
            .ships
            .values()
            .filter(|s| s.alive)
            .map(|s| s.pos)
            .collect();
        let (pos, normal) = face_site(view.terrain, &living, view.terrain.bounds().center());
        e.event.pos = pos;
        let mut spawn = drone::prop(
            drone::GOSSIP_FACE,
            pos,
            e.def.param("face_radius", 3.0),
            1.0,
            [0; 4],
        );
        spawn.rot = drone::facing(normal);
        e.ctx.spawn_entity(spawn);
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let Some(mut face) = drone::first_prop(&e.ctx.view, drone::GOSSIP_FACE) else {
            return;
        };
        let talking = (e.event.phase == EventPhase::Active) as u8;
        if face.state[0] != talking {
            face.state[0] = talking;
            e.ctx.pose_entity(&face);
        }
    }

    fn on_end(&self, e: &mut EventCtx) {
        drone::remove_props(e.ctx, drone::GOSSIP_FACE);
    }

    /// Every ship is shown to everyone while the face talks.
    fn reveal(&self, fx: &EffectCtx, _ship: &ShipState) -> bool {
        fx.view.phase == EventPhase::Active
    }
}
