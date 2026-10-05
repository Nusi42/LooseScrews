//! Generic simulation entities and the replication view of everything that is
//! not a ship or a projectile.
//!
//! - [`Entity`]: debris, decoys, ghosts, dust clouds and markers, stored in one
//!   list in the [`crate::World`]. Each has a [`Source`] (the weapon or gadget
//!   module that spawned it) whose entity hooks run every tick.
//! - Pickups ([`crate::PickupSpawn`]) and scrap piles / wrecks
//!   ([`crate::ScrapPile`]) keep their own typed lists (rules and scavenging
//!   read them), but share the entity id space.
//! - [`EntityView`]: one flat, generic description of any of them (kind,
//!   visual, pose, owner, team, 4 state bytes) that `ls-game` turns into
//!   `ls_net::NetEntity` for replication. See `docs/architecture/arsenal-framework.md`.
//!
//! # Ids
//!
//! Entity ids come from one counter per world. The allocator skips ids whose
//! low 15 bits ([`WIRE_ID_MASK`]) belong to a live entity, so the wire id
//! (`id & WIRE_ID_MASK`) is unique among live entities even for long-lived
//! ones (wrecks, pickups) after the counter wraps.

use ls_math::terrain::TerrainQuery;
use ls_math::{Quat, Vec3, DT};
use serde::{Deserialize, Serialize};

use crate::ship::{move_sphere, PlayerId};
use crate::visual::VisualId;
use crate::{GadgetId, WeaponId};

/// Wire ids of entities are `id & WIRE_ID_MASK` (15 bits).
pub const WIRE_ID_MASK: u32 = 0x7fff;
/// At most this many debris pieces exist; spawning more removes the oldest.
pub const MAX_DEBRIS: usize = 64;
/// Smallest and largest debris radius (u).
pub const DEBRIS_MIN_RADIUS: f32 = 0.3;
pub const DEBRIS_MAX_RADIUS: f32 = 1.5;
/// Debris velocity damping (fraction of speed lost per second).
pub const DEBRIS_DRAG: f32 = 0.6;
/// Coefficient of restitution of debris against terrain.
pub const DEBRIS_RESTITUTION: f32 = 0.45;
/// Debris slower than this goes to rest (u/s).
pub const DEBRIS_REST_SPEED: f32 = 0.2;
/// Share of a ship's closing speed transferred to debris it rams.
pub const DEBRIS_PUSH: f32 = 1.2;
/// Team value for entities without a team.
pub const NO_TEAM: u8 = u8::MAX;

/// Kinds of generic [`Entity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EntityKind {
    /// Physical chunk: bounces, blocks shots, pushable, scavengeable.
    Debris,
    /// Decoy gadget copy of a ship; has integrity (`hp`) and draws fire.
    Decoy,
    /// Borrowed Time ghost left at the old spot.
    Ghost,
    /// Dust or smoke volume (`radius`) that cuts visibility.
    DustCloud,
    /// Anything else a module wants to show (grab points, warnings).
    Marker,
}

/// Every kind of replicated non-ship, non-projectile thing (wire `kind` byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ReplicatedKind {
    Debris = 0,
    WeaponPickup = 1,
    UpgradePickup = 2,
    GadgetPickup = 3,
    ScrapPile = 4,
    Wreck = 5,
    Decoy = 6,
    Ghost = 7,
    DustCloud = 8,
    Marker = 9,
    /// A pigment pickup on a monochrome level (M4; state `[splash, side, 0, 0]`).
    PigmentPickup = 10,
}

impl ReplicatedKind {
    pub fn from_u8(v: u8) -> Option<ReplicatedKind> {
        use ReplicatedKind::*;
        [
            Debris,
            WeaponPickup,
            UpgradePickup,
            GadgetPickup,
            ScrapPile,
            Wreck,
            Decoy,
            Ghost,
            DustCloud,
            Marker,
            PigmentPickup,
        ]
        .get(v as usize)
        .copied()
    }
}

impl From<EntityKind> for ReplicatedKind {
    fn from(k: EntityKind) -> Self {
        match k {
            EntityKind::Debris => ReplicatedKind::Debris,
            EntityKind::Decoy => ReplicatedKind::Decoy,
            EntityKind::Ghost => ReplicatedKind::Ghost,
            EntityKind::DustCloud => ReplicatedKind::DustCloud,
            EntityKind::Marker => ReplicatedKind::Marker,
        }
    }
}

/// Which module spawned an entity (its entity hooks run each tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Source {
    #[default]
    None,
    Weapon(WeaponId),
    Gadget(GadgetId),
}

/// A generic simulation entity. See the module docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: u32,
    pub kind: EntityKind,
    pub source: Source,
    pub visual: VisualId,
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    /// Collision / blocking sphere (debris, decoys) or volume (dust) radius.
    pub radius: f32,
    pub owner: Option<PlayerId>,
    /// Team, or [`NO_TEAM`].
    pub team: u8,
    pub spawn_tick: u64,
    /// Removed at the start of this tick (`u64::MAX` = never).
    pub expires_tick: u64,
    /// Integrity (decoys) or toughness (debris); `<= 0` destroys it.
    pub hp: f32,
    /// Scavengeable scrap left in it (debris).
    pub scrap: f32,
    /// Debris at rest skips physics until something pushes it.
    pub resting: bool,
    /// Moved by its module only: the world just adds `vel * DT` each tick,
    /// with no terrain collision (a decoy's rubber-band return).
    #[serde(default)]
    pub kinematic: bool,
    /// Free per-module state.
    pub vars: [f32; 4],
    /// Module-defined state replicated to clients (e.g. a warning phase).
    pub state: [u8; 4],
}

impl Entity {
    /// Whether projectiles collide with it.
    pub fn blocks_projectiles(&self) -> bool {
        matches!(self.kind, EntityKind::Debris | EntityKind::Decoy)
    }

    /// Wire id (`id & WIRE_ID_MASK`).
    pub fn wire_id(&self) -> u16 {
        (self.id & WIRE_ID_MASK) as u16
    }

    /// Mass used for impulses: proportional to volume, 1 for a 1 u radius.
    pub fn mass(&self) -> f32 {
        (self.radius * self.radius * self.radius).max(0.01)
    }
}

/// What a module asks the world to spawn (the world assigns the id).
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySpawn {
    pub kind: EntityKind,
    pub source: Source,
    pub visual: VisualId,
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub radius: f32,
    pub owner: Option<PlayerId>,
    pub team: u8,
    /// Lifetime in ticks (`None` = until destroyed or scavenged).
    pub lifetime_ticks: Option<u64>,
    pub hp: f32,
    pub scrap: f32,
    /// See [`Entity::kinematic`].
    pub kinematic: bool,
    pub vars: [f32; 4],
    pub state: [u8; 4],
}

impl EntitySpawn {
    /// A debris chunk (radius clamped to 0.3..=1.5 u) with scrap to scavenge.
    /// Toughness scales with volume.
    pub fn debris(pos: Vec3, vel: Vec3, radius: f32, scrap: f32) -> EntitySpawn {
        let radius = radius.clamp(DEBRIS_MIN_RADIUS, DEBRIS_MAX_RADIUS);
        EntitySpawn {
            kind: EntityKind::Debris,
            source: Source::None,
            visual: crate::visual::BuiltinVisual::Debris.id(),
            pos,
            vel,
            rot: Quat::IDENTITY,
            radius,
            owner: None,
            team: NO_TEAM,
            lifetime_ticks: None,
            hp: 20.0 * radius * radius * radius,
            scrap: scrap.max(0.0),
            kinematic: false,
            vars: [0.0; 4],
            state: [0; 4],
        }
    }

    /// A dust cloud of `radius` lasting `ticks`.
    pub fn dust(center: Vec3, radius: f32, ticks: u64) -> EntitySpawn {
        EntitySpawn {
            kind: EntityKind::DustCloud,
            visual: crate::visual::BuiltinVisual::DustCloud.id(),
            radius,
            lifetime_ticks: Some(ticks),
            ..EntitySpawn::marker(center)
        }
    }

    /// A ghost of a ship at `pos` lasting `ticks`.
    pub fn ghost(owner: PlayerId, team: u8, pos: Vec3, rot: Quat, ticks: u64) -> EntitySpawn {
        EntitySpawn {
            kind: EntityKind::Ghost,
            visual: crate::visual::BuiltinVisual::Ghost.id(),
            rot,
            owner: Some(owner),
            team,
            lifetime_ticks: Some(ticks),
            radius: 0.5,
            ..EntitySpawn::marker(pos)
        }
    }

    /// A static marker without physics.
    pub fn marker(pos: Vec3) -> EntitySpawn {
        EntitySpawn {
            kind: EntityKind::Marker,
            source: Source::None,
            visual: VisualId(0),
            pos,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            radius: 0.5,
            owner: None,
            team: NO_TEAM,
            lifetime_ticks: None,
            hp: 1.0,
            scrap: 0.0,
            kinematic: false,
            vars: [0.0; 4],
            state: [0; 4],
        }
    }

    /// Builder: sets the spawning module.
    pub fn from_source(mut self, source: Source) -> EntitySpawn {
        self.source = source;
        self
    }
}

/// Generic replication view of any entity-like thing (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityView {
    pub id: u32,
    pub kind: ReplicatedKind,
    pub visual: VisualId,
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub owner: Option<PlayerId>,
    pub team: u8,
    /// Kind-specific bytes: weapon pickups `[weapon, ammo, 0, 0]`, upgrade
    /// and gadget pickups `[weapon or gadget, 0, 0, 0]`, scrap piles and
    /// wrecks `[scrap (whole units, max 255), 0, 0, 0]`, debris `[scrap,
    /// resting, radius in tenths of a unit, 0]`, decoys `[integrity percent,
    /// module state, 0, 0]`, dust clouds `[radius in whole units, module
    /// state...]`, others the module's `Entity::state`.
    pub state: [u8; 4],
}

impl EntityView {
    pub fn wire_id(&self) -> u16 {
        (self.id & WIRE_ID_MASK) as u16
    }
}

/// One physics step of a debris piece: drag, terrain collision with bounce,
/// rest detection (there is no gravity, so debris drifts until drag stops
/// it). Returns true if it touched terrain.
pub fn step_debris<T: TerrainQuery + ?Sized>(e: &mut Entity, terrain: &T) -> bool {
    if e.resting {
        return false;
    }
    e.vel *= (1.0 - DEBRIS_DRAG * DT).max(0.0);
    let (contact, _) = move_sphere(
        terrain,
        &mut e.pos,
        &mut e.vel,
        e.radius,
        DEBRIS_RESTITUTION,
    );
    // No gravity: slow debris rests wherever it is (in contact or floating).
    if e.vel.length() < DEBRIS_REST_SPEED {
        e.vel = Vec3::ZERO;
        e.resting = true;
    }
    contact
}

/// [`step_debris`] in a gravity well (M4): the pull `accel` is added
/// first, and the debris only comes to rest while it touches rock (in open
/// space a pulled piece keeps falling). Returns true if it touched terrain.
pub fn step_debris_pulled<T: TerrainQuery + ?Sized>(
    e: &mut Entity,
    terrain: &T,
    accel: Vec3,
) -> bool {
    e.resting = false;
    e.vel += accel * DT;
    e.vel *= (1.0 - DEBRIS_DRAG * DT).max(0.0);
    let (contact, _) = move_sphere(
        terrain,
        &mut e.pos,
        &mut e.vel,
        e.radius,
        DEBRIS_RESTITUTION,
    );
    if contact && e.vel.length() < DEBRIS_REST_SPEED {
        e.vel = Vec3::ZERO;
        e.resting = true;
    }
    contact
}

/// Byte helper for [`EntityView::state`]: whole units clamped to 0..=255.
pub fn state_byte(v: f32) -> u8 {
    if v.is_nan() || v <= 0.0 {
        0
    } else {
        ls_math::det::ceil(v).min(255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::PlaneTerrain;
    use ls_math::Aabb;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn debris(pos: Vec3, vel: Vec3) -> Entity {
        let s = EntitySpawn::debris(pos, vel, 0.5, 5.0);
        Entity {
            id: 1,
            kind: s.kind,
            source: s.source,
            visual: s.visual,
            pos: s.pos,
            vel: s.vel,
            rot: s.rot,
            radius: s.radius,
            owner: None,
            team: s.team,
            spawn_tick: 0,
            expires_tick: u64::MAX,
            hp: s.hp,
            scrap: s.scrap,
            resting: false,
            kinematic: false,
            vars: [0.0; 4],
            state: [0; 4],
        }
    }

    #[test]
    fn debris_bounces_damps_and_comes_to_rest() {
        let floor = PlaneTerrain::floor(0.0, Aabb::new(Vec3::splat(-100.0), Vec3::splat(100.0)));
        let mut e = debris(Vec3::new(0.0, 3.0, 0.0), Vec3::new(2.0, -20.0, 0.0));
        let mut bounced = false;
        for _ in 0..600 {
            let before = e.vel.y;
            step_debris(&mut e, &floor);
            assert!(e.pos.y >= e.radius - 1e-3, "{:?}", e.pos);
            bounced |= before < 0.0 && e.vel.y > 0.0;
        }
        assert!(bounced, "bounces off the floor");
        assert!(e.resting, "comes to rest");
        assert_eq!(e.vel, Vec3::ZERO);
        let rest = e.pos;
        step_debris(&mut e, &floor);
        assert_eq!(e.pos, rest, "resting debris skips physics");
    }

    #[test]
    fn spawn_helpers_clamp_and_kinds_round_trip() {
        let s = EntitySpawn::debris(Vec3::ZERO, Vec3::ZERO, 9.0, -1.0);
        assert_eq!(s.radius, DEBRIS_MAX_RADIUS);
        assert_eq!(s.scrap, 0.0);
        assert_eq!(
            EntitySpawn::dust(Vec3::ZERO, 10.0, 60).lifetime_ticks,
            Some(60)
        );
        for k in 0..11u8 {
            assert_eq!(ReplicatedKind::from_u8(k).unwrap() as u8, k);
        }
        assert!(ReplicatedKind::from_u8(11).is_none());
        assert_eq!(state_byte(3.2), 4);
        assert_eq!(state_byte(1e9), 255);
        assert_eq!(state_byte(f32::NAN), 0);
    }
}
