//! Weapon behaviour modules: one file per weapon under
//! `src/weapons/{energy,kinetic,ordnance,specials}/<weapon_id>.rs`, each
//! implementing [`WeaponBehavior`], registered with its data file in
//! [`registry::MODULES`].
//!
//! Every hook has a default, and the defaults together are the generic
//! weapon: a straight projectile (or the data's `projectiles`/`spread_deg`
//! pattern) with direct damage, splash, knockback and the data's crater and
//! paint. That is exactly the M1 behaviour, so the four M1 weapons use the
//! defaults unchanged, and every weapon not implemented yet stays playable
//! with its data numbers. A module overrides only the hooks it needs.
//!
//! # Hook order per tick
//!
//! Inside [`crate::World::step`] (after respawns and ship movement):
//!
//! 1. **Fire** (per ship, primary then secondary): charge weapons call
//!    [`WeaponBehavior::charge_start`] when the trigger goes down and
//!    [`WeaponBehavior::charge_hold`] every tick it stays down; on release
//!    (or `ChargeAction::Release`) the shot is paid and
//!    [`WeaponBehavior::fire`] runs with the charge fraction. Other weapons
//!    call `fire` whenever the trigger is down, the cooldown is over and the
//!    cost is paid.
//! 2. **Projectiles** (in list order, each one completely): if its fuse ran
//!    out, [`WeaponBehavior::on_fuse`]; if its lifetime ran out,
//!    [`WeaponBehavior::on_expire`] (both end the projectile unless the
//!    hook re-arms it); otherwise [`WeaponBehavior::update`] (steering,
//!    homing, guidance, arming, path recording), then the swept move
//!    against ships, blocking entities (debris, decoys) and terrain, calling
//!    [`WeaponBehavior::on_ship_hit`], [`WeaponBehavior::on_entity_hit`] or
//!    [`WeaponBehavior::on_terrain_hit`] for the nearest hit; the returned
//!    [`HitResponse`] decides whether it stops, flies on, pierces, bounces or
//!    sticks. Stuck projectiles skip the move. Projectiles spawned by hooks
//!    start moving on the next tick.
//! 3. **Entities**: [`WeaponBehavior::entity_tick`] for entities a weapon
//!    spawned, [`WeaponBehavior::entity_expired`] when they time out.
//!
//! Commands queued by a hook are applied right after it returns (see
//! [`crate::behavior`]).

pub mod energy;
pub mod kinetic;
pub mod ordnance;
pub mod registry;
pub mod specials;

use std::fmt::Debug;

use ls_math::terrain::RayHit;
use ls_math::{Quat, Vec3};

use crate::behavior::Ctx;
use crate::entity::{Entity, EntityKind};
use crate::ship::{PlayerId, ShipState};
use crate::world::{proj_flags, Projectile};
use crate::{WeaponDef, WeaponId};

pub use registry::{behavior, WeaponModule, MODULES};

/// A paid shot, handed to [`WeaponBehavior::fire`].
#[derive(Debug, Clone, Copy)]
pub struct Shot<'a> {
    pub shooter: &'a ShipState,
    pub weapon: WeaponId,
    pub def: &'a WeaponDef,
    /// Upgrade level of the weapon (1..=max_level).
    pub level: u8,
    /// Paid shots of this weapon before this one (`WeaponState::shots`):
    /// the phase of alternating or rotating patterns.
    pub shots: u32,
    /// Direct-hit damage per projectile, all multipliers applied (level,
    /// ship type, rule set tuning, charge).
    pub damage: f32,
    pub splash_damage: f32,
    /// Knockback (u/s) of a direct hit, scaled like the damage.
    pub knockback: f32,
    /// Charge fraction `0..=1` (0 for weapons without charge).
    pub charge: f32,
    /// Ticks the trigger was held (charge weapons).
    pub held_ticks: u64,
}

impl Shot<'_> {
    /// Muzzle point for a projectile flying along `dir` (just outside the hull).
    pub fn muzzle(&self, ship_radius: f32, dir: Vec3) -> Vec3 {
        self.shooter.pos + dir * (ship_radius + self.def.projectile_radius + 0.05)
    }
}

/// What a projectile does after a hit hook.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitResponse {
    /// The projectile ends here (the hook already detonated it, or not).
    Stop,
    /// Flies on through the ship or entity; it is not hit again this tick.
    /// On terrain, `Continue` counts as `Stop` (use `Pierce`).
    Continue,
    /// Flies on from `exit` (the far side of a wall or ship).
    Pierce { exit: Vec3 },
    /// Reflects off the surface; the rest of this tick's move is lost.
    Bounce,
    /// Stops at the hit point and stays there ([`proj_flags::STUCK`]).
    Stick,
}

/// What a charge weapon does while its trigger is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeAction {
    Hold,
    /// Fire now (as if released).
    Release,
    /// Drop the charge without firing or paying.
    Cancel,
}

/// A projectile hit a ship.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShipHit {
    pub target: PlayerId,
    pub point: Vec3,
    /// Surface normal of the hit sphere at `point`.
    pub normal: Vec3,
}

/// A projectile hit a blocking entity (debris, decoy).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityHit {
    pub id: u32,
    pub kind: EntityKind,
    pub point: Vec3,
    pub normal: Vec3,
}

/// Behaviour of one weapon. See the module docs for when each hook runs.
/// All defaults together are the generic (M1) weapon.
pub trait WeaponBehavior: Send + Sync + Debug {
    /// The weapon id (`data/weapons/<id>.ron`).
    fn id(&self) -> &'static str;

    /// True once the module implements the design doc (not the fallback).
    fn implemented(&self) -> bool {
        false
    }

    /// Fires a paid shot. Default: [`fire_pattern`]. Instant weapons (Leech
    /// Arc) queue their effect here instead of spawning projectiles.
    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        fire_pattern(ctx, shot);
    }

    /// The trigger of a charge weapon went down (nothing is paid yet).
    fn charge_start(&self, _ctx: &mut Ctx, _ship: &ShipState, _weapon: WeaponId) {}

    /// Every tick the trigger of a charge weapon stays down. Default:
    /// release at full charge if the data says `auto_release`, else hold.
    fn charge_hold(
        &self,
        ctx: &mut Ctx,
        _ship: &ShipState,
        weapon: WeaponId,
        held_ticks: u64,
    ) -> ChargeAction {
        default_charge_hold(ctx.def(weapon), held_ticks)
    }

    /// Per-tick update before the move: steering (homing, guidance from
    /// `ctx.input(owner)`, spirals), arming, creeping, path recording.
    fn update(&self, _ctx: &mut Ctx, _p: &mut Projectile) {}

    /// Hit a ship. Default: detonate (direct damage, splash, knockback), stop.
    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        ctx.detonate(p, hit.point, Some(hit.target), false);
        HitResponse::Stop
    }

    /// Hit a blocking entity. Default: [`default_entity_hit`].
    fn on_entity_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
        default_entity_hit(ctx, p, hit)
    }

    /// Hit terrain. Default: detonate with the data's crater and paint, stop.
    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, true);
        HitResponse::Stop
    }

    /// The projectile's fuse (`fuse_tick`) ran out. Default: detonate where
    /// it is. The projectile is removed afterwards unless the hook sets a
    /// later `fuse_tick`.
    fn on_fuse(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let terrain = p.flags & proj_flags::STUCK != 0;
        ctx.detonate(p, p.pos, None, terrain);
    }

    /// The lifetime ran out (the projectile is removed afterwards). Default:
    /// nothing (M1 projectiles just vanish). Split into sub-projectiles here.
    fn on_expire(&self, _ctx: &mut Ctx, _p: &mut Projectile) {}

    /// Every tick for each live entity this weapon spawned. Changes to `e`
    /// (position, velocity, `vars`, `state`, `expires_tick`, `hp <= 0` to
    /// remove it) are written back before the queued commands run.
    fn entity_tick(&self, _ctx: &mut Ctx, _e: &mut Entity) {}

    /// An entity this weapon spawned expires (before it is removed). Setting
    /// a later `e.expires_tick` keeps it.
    fn entity_expired(&self, _ctx: &mut Ctx, _e: &mut Entity) {}
}

/// Default charge rule: auto-release at `max_secs` if the data says so.
pub fn default_charge_hold(def: &WeaponDef, held_ticks: u64) -> ChargeAction {
    match def.charge {
        Some(c) if c.auto_release && held_ticks >= WeaponDef::charge_ticks(c.max_secs) => {
            ChargeAction::Release
        }
        _ => ChargeAction::Hold,
    }
}

/// Default hit on debris or a decoy: `Impact` event, the direct damage goes
/// to the entity's integrity, knockback pushes it, splash as usual.
pub fn default_entity_hit(ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
    ctx.detonate(p, hit.point, None, false);
    let source = crate::behavior::DamageSource::projectile(p);
    if p.damage > 0.0 {
        ctx.push(crate::behavior::Command::DamageEntity {
            id: hit.id,
            amount: p.damage,
            source,
        });
    }
    ctx.impulse_entity(hit.id, p.vel.normalize_or_zero() * p.knockback);
    HitResponse::Stop
}

/// Generic fire pattern: `def.projectiles` projectiles, one along the aim
/// for a single projectile; up to 3 in a horizontal fan `spread_deg` apart;
/// more as one centre projectile plus a ring at `spread_deg`.
pub fn fire_pattern(ctx: &mut Ctx, shot: &Shot) {
    let n = shot.def.projectiles.max(1) as usize;
    for i in 0..n {
        let dir = pattern_dir(shot.shooter.orientation, i, n, shot.def.spread_deg);
        let p = projectile_for(ctx, shot, dir);
        ctx.spawn_projectile(p);
    }
}

/// Direction of projectile `i` of `n` in the generic pattern.
pub fn pattern_dir(rot: Quat, i: usize, n: usize, spread_deg: f32) -> Vec3 {
    let fwd = rot.forward();
    if n <= 1 || spread_deg <= 0.0 {
        return fwd;
    }
    let spread = spread_deg.to_radians();
    if n <= 3 {
        let a = (i as f32 - (n - 1) as f32 * 0.5) * spread;
        return Quat::from_axis_angle(rot.up(), a).rotate(fwd);
    }
    if i == 0 {
        return fwd;
    }
    let phi = std::f32::consts::TAU * (i - 1) as f32 / (n - 1) as f32;
    let (s, c) = ls_math::det::sin_cos(phi);
    let axis = rot.right() * c + rot.up() * s;
    Quat::from_axis_angle(axis, spread).rotate(fwd)
}

/// A projectile of `shot` flying along `dir` from the muzzle, with the
/// shot's damage and the weapon's speed, lifetime, radius and visual.
pub fn projectile_for(ctx: &Ctx, shot: &Shot, dir: Vec3) -> Projectile {
    let ship = shot.shooter;
    let radius = ctx.tuning_of(ship).radius;
    let now = ctx.now();
    Projectile {
        id: 0,
        owner: ship.player,
        team: ship.team,
        weapon: shot.weapon,
        pos: shot.muzzle(radius, dir),
        vel: dir * shot.def.speed,
        expires_tick: now + shot.def.lifetime_ticks(),
        damage: shot.damage,
        splash_damage: shot.splash_damage,
        knockback: shot.knockback,
        spawn_tick: now,
        radius: shot.def.projectile_radius,
        visual: ctx.view.visuals.projectile(shot.weapon),
        charge: shot.charge,
        generation: 0,
        bounces: 0,
        flags: 0,
        arm_tick: now,
        fuse_tick: u64::MAX,
        target: None,
        vars: [0.0; 4],
        path: Vec::new(),
        passed: None,
    }
}

/// A sub-projectile of `parent` (splitting, swarm blobs, sub-charges): same
/// owner, team and weapon, one generation deeper, no damage until the
/// caller sets it.
pub fn sub_projectile(
    ctx: &Ctx,
    parent: &Projectile,
    pos: Vec3,
    vel: Vec3,
    lifetime_ticks: u64,
) -> Projectile {
    let now = ctx.now();
    Projectile {
        id: 0,
        pos,
        vel,
        expires_tick: now + lifetime_ticks.max(1),
        damage: 0.0,
        splash_damage: 0.0,
        knockback: 0.0,
        spawn_tick: now,
        generation: parent.generation.saturating_add(1),
        bounces: 0,
        flags: 0,
        arm_tick: now,
        fuse_tick: u64::MAX,
        target: None,
        vars: [0.0; 4],
        path: Vec::new(),
        passed: None,
        ..parent.clone()
    }
}

/// Turns `vel` towards `desired` by at most `max_angle` radians, keeping
/// its speed (homing, guidance).
pub fn steer_towards(vel: Vec3, desired: Vec3, max_angle: f32) -> Vec3 {
    let speed = vel.length();
    let (Some(from), Some(to)) = (vel.try_normalize(), desired.try_normalize()) else {
        return vel;
    };
    let cos = from.dot(to).clamp(-1.0, 1.0);
    let angle = ls_math::det::acos(cos);
    if angle <= max_angle {
        return to * speed;
    }
    let axis = from.cross(to);
    let axis = if axis.length_squared() < 1e-12 {
        from.any_orthonormal()
    } else {
        axis
    };
    Quat::from_axis_angle(axis, max_angle).rotate(from) * speed
}

#[cfg(test)]
mod tests;
