//! Everything besides ships and projectiles that the client draws, in one
//! generic, data-driven form (`docs/architecture/arsenal-framework.md`,
//! "Entities" and "Visuals"):
//!
//! - [`EntityDraw`]: a replicated entity (pickups, scrap piles, wrecks,
//!   debris, decoys, ghosts, dust clouds, markers) with its visual id;
//! - [`shape_name`] / [`visual_rows`]: the match's `VisualTable` flattened
//!   for the renderer, which maps a shape name to a mesh and tints it, so a
//!   new weapon or entity needs no client code for its basic look;
//! - [`PlayerInfo`]: names, teams and ship types for name tags and the
//!   scoreboard.

use ls_math::{Quat, Vec3};
use ls_net::NetEntity;
use ls_sim::{EntityView, ReplicatedKind, Shape, Visual, VisualTable};

/// A replicated entity, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityDraw {
    /// Wire id (online) or wire id of the sim id (practice).
    pub id: u16,
    /// `ls_sim::ReplicatedKind` byte.
    pub kind: u8,
    /// Index into the match's visual table.
    pub visual: u16,
    /// Owner ship id, or 255.
    pub owner: u8,
    /// Team, or 255.
    pub team: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    /// Kind-specific bytes (see `ls_sim::EntityView::state`).
    pub state: [u8; 4],
    /// Collision / volume radius in u, 0 when unknown (the renderer then uses
    /// its default size for the shape). Online it is decoded from the state
    /// bytes for debris and dust clouds ([`net_radius`]).
    pub radius: f32,
}

/// Radius (u) an entity's state bytes carry (`ls_sim::EntityView::state`):
/// debris in tenths of a unit in byte 2, dust clouds in whole units in byte
/// 0; 0 (unknown) for other kinds.
pub fn net_radius(kind: u8, state: [u8; 4]) -> f32 {
    match ReplicatedKind::from_u8(kind) {
        Some(ReplicatedKind::Debris) => state[2] as f32 / 10.0,
        Some(ReplicatedKind::DustCloud) => state[0] as f32,
        _ => 0.0,
    }
}

impl EntityDraw {
    pub fn from_net(e: &NetEntity) -> EntityDraw {
        EntityDraw {
            id: e.id,
            kind: e.kind,
            visual: e.visual,
            owner: e.owner,
            team: e.team,
            pos: e.pos,
            vel: e.vel,
            rot: e.rot,
            state: e.state,
            radius: net_radius(e.kind, e.state),
        }
    }

    pub fn from_view(v: &EntityView, radius: f32) -> EntityDraw {
        EntityDraw {
            id: v.wire_id(),
            kind: v.kind as u8,
            visual: v.visual.0,
            owner: v.owner.map_or(u8::MAX, |p| p.0.min(254) as u8),
            team: v.team,
            pos: v.pos,
            vel: v.vel,
            rot: v.rot,
            state: v.state,
            radius,
        }
    }
}

/// Every [`Shape`] in declaration order (the index sent to JavaScript).
pub const SHAPES: &[Shape] = Shape::ALL;

/// Lowercase name of a shape (the key of the web client's shape styles).
pub fn shape_name(s: Shape) -> &'static str {
    match s {
        Shape::Bolt => "bolt",
        Shape::Pellet => "pellet",
        Shape::Spike => "spike",
        Shape::Rocket => "rocket",
        Shape::Missile => "missile",
        Shape::Mine => "mine",
        Shape::Orb => "orb",
        Shape::Blob => "blob",
        Shape::Arc => "arc",
        Shape::Grenade => "grenade",
        Shape::Harpoon => "harpoon",
        Shape::Paint => "paint",
        Shape::Charge => "charge",
        Shape::Debris => "debris",
        Shape::Scrap => "scrap",
        Shape::Wreck => "wreck",
        Shape::Crate => "crate",
        Shape::Upgrade => "upgrade",
        Shape::Gadget => "gadget",
        Shape::Decoy => "decoy",
        Shape::Ghost => "ghost",
        Shape::Dust => "dust",
        Shape::Duck => "duck",
    }
}

/// Index of a shape in [`SHAPES`].
pub fn shape_index(s: Shape) -> usize {
    s.index()
}

/// Floats per visual in [`visual_rows`].
pub const VISUAL_STRIDE: usize = 5;

/// `[shapeIndex, r, g, b, scale]` per visual in id order; colours 0..1.
pub fn visual_rows(table: &VisualTable) -> Vec<f32> {
    let mut v = Vec::with_capacity(table.entries().len() * VISUAL_STRIDE);
    for Visual {
        shape,
        color,
        scale,
    } in table.entries()
    {
        v.extend_from_slice(&[
            shape_index(*shape) as f32,
            color.0 as f32 / 255.0,
            color.1 as f32 / 255.0,
            color.2 as f32 / 255.0,
            *scale,
        ]);
    }
    v
}

/// A player as name tags and the scoreboard show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    /// Wire id (online) or player id (practice).
    pub id: u8,
    pub team: u8,
    /// `ls_sim::ShipType::index`.
    pub ship_type: u8,
    /// Gadget id, 255 for none.
    pub gadget: u8,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_sim::{BuiltinVisual, GadgetTable, WeaponTable};

    #[test]
    fn every_shape_has_a_distinct_name_and_index() {
        let mut names: Vec<&str> = SHAPES.iter().map(|&s| shape_name(s)).collect();
        for (i, &s) in SHAPES.iter().enumerate() {
            assert_eq!(shape_index(s), i);
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), SHAPES.len());
    }

    #[test]
    fn debris_and_dust_radii_come_from_the_state_bytes() {
        let debris = ReplicatedKind::Debris as u8;
        let dust = ReplicatedKind::DustCloud as u8;
        assert_eq!(net_radius(debris, [4, 1, 12, 0]), 1.2);
        assert_eq!(net_radius(dust, [9, 0, 0, 0]), 9.0);
        assert_eq!(net_radius(ReplicatedKind::Wreck as u8, [9, 9, 9, 9]), 0.0);
    }

    #[test]
    fn visual_rows_follow_the_table() {
        let w = WeaponTable::builtin();
        let g = GadgetTable::builtin();
        let t = VisualTable::new(&w, &g);
        let rows = visual_rows(&t);
        assert_eq!(rows.len(), t.entries().len() * VISUAL_STRIDE);
        let wreck = BuiltinVisual::Wreck.id().0 as usize * VISUAL_STRIDE;
        assert_eq!(SHAPES[rows[wreck] as usize], Shape::Wreck);
        let rocket = t.projectile(w.find("bolt_rocket").unwrap()).0 as usize * VISUAL_STRIDE;
        assert_eq!(shape_name(SHAPES[rows[rocket] as usize]), "rocket");
        assert!(rows[rocket + 1..rocket + 4]
            .iter()
            .all(|c| (0.0..=1.0).contains(c)));
        assert!(rows[rocket + 4] > 0.0);
    }
}
