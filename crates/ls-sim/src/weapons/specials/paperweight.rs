//! Paperweight: special secondary (M4, `docs/milestone-4-plan.md` §3.8 "Paperweight"). A
//! lobbed stone that stops where it lands and is a small gravity well for 6 s. Malus: it does
//! not care who threw it. Lasting effect: when it expires, the debris it gathered fuses into a
//! floating lump of weak rock.
//!
//! Numbers: `data/weapons/paperweight.ron` (`speed`, `lifetime_secs` = the flight time,
//! `params`: `well_secs`, `well_range`, `well_pull`, `gather_radius`, `lump_min_radius`,
//! `lump_max_radius`).
//!
//! Status: implemented.
//! - Flight: the generic projectile at `speed` (bent by other wells, `gravity: true`), flagged
//!   [`proj_flags::NO_SHIP_HITS`]; debris and decoys do not stop it either. No damage, no
//!   knockback, no crater.
//! - Landing ("paperweights stay where you put them"): on rock ([`HitResponse::Stick`], the
//!   stone rests on the surface) or after `lifetime_secs` of flight wherever it is (the fuse).
//!   It becomes a dynamic gravity well ([`crate::gravity::DynamicWell`], key = the projectile
//!   id) centred on the stone from the next tick for `well_secs`: range `well_range`, peak pull
//!   `well_pull` x the Tinker's per-axis thrust acceleration, both on the wire grid of
//!   `ls_net::well_state` (0.25 u, 0.125 u/s^2) so the client predictor rebuilds it exactly.
//!   The world's well field pulls ships (the thrower too; the total on a ship stays within the
//!   40 % clamp however many wells stack), debris, unstuck mines and `gravity: true` ordnance;
//!   bolts and kinetic rounds fly straight.
//! - The landed stone is the projectile itself: stuck, with [`LANDED`] set (replicated, so
//!   clients draw the well's stone), until the well ends. Something that pulls it loose
//!   (Magnet Sneeze) does not move it: it is put back each tick.
//! - Expiry: debris within `gather_radius` of the stone fuses with it into one `AddSphere` of
//!   weak rock (a rubble region, [`crate::Command::WeakRock`]) of radius
//!   `cbrt(lump_min_radius^3 + sum of the debris radii^3)`, at most `lump_max_radius`. If the
//!   lump would overlap a living ship it is left out and the debris stays.

use ls_math::terrain::{RayHit, TerrainEdit};
use ls_math::{det, Tick, Vec3};

use crate::behavior::{Command, Ctx};
use crate::entity::EntityKind;
use crate::gravity::DynamicWell;
use crate::weapons::{projectile_for, EntityHit, HitResponse, Shot, WeaponBehavior};
use crate::world::{proj_flags, Projectile};
use crate::{ShipType, WeaponDef};

/// Behaviour of the Paperweight.
#[derive(Debug)]
pub struct Paperweight;

/// Replicated projectile bit: the stone has landed and is a well.
pub const LANDED: u16 = proj_flags::CUSTOM_A;

/// Range step of the well wire format (`ls_net::well_state::RANGE_UNIT`).
const RANGE_UNIT: f32 = 0.25;
/// Peak pull step of the well wire format (`ls_net::well_state::PEAK_UNIT`).
const PEAK_UNIT: f32 = 0.125;

/// Gap kept between the lump and a ship's hull (as the Seed Grenade).
const SHIP_CLEARANCE: f32 = 0.3;

/// `v` rounded to multiples of `unit`.
fn on_grid(v: f32, unit: f32) -> f32 {
    det::round(v / unit) * unit
}

/// Range (u) and peak pull (u/s^2) of a Paperweight's well with `def` in a
/// world whose Tinker accelerates at `tinker_thrust` per axis, on the wire
/// grid.
pub fn well_numbers(def: &WeaponDef, tinker_thrust: f32) -> (f32, f32) {
    let range = on_grid(def.param("well_range", 16.0).clamp(0.0, 63.75), RANGE_UNIT);
    let peak = on_grid(
        (def.param("well_pull", 0.3) * tinker_thrust).clamp(0.0, 31.875),
        PEAK_UNIT,
    );
    (range, peak)
}

/// Radius of the lump a stone fuses into with gathered debris of radii
/// `debris` (`cbrt(min^3 + sum r^3)`, clamped to `min..=max`).
pub fn lump_radius(def: &WeaponDef, debris: impl IntoIterator<Item = f32>) -> f32 {
    let min = def.param("lump_min_radius", 1.0).max(0.0);
    let max = def.param("lump_max_radius", 2.0).max(min);
    let volume = debris
        .into_iter()
        .fold(min * min * min, |v, r| v + r * r * r);
    det::powf(volume, 1.0 / 3.0).clamp(min, max)
}

/// The stone lands at `pos`: it stops, and its well starts next tick.
fn land(ctx: &mut Ctx, p: &mut Projectile, pos: Vec3) {
    let def = ctx.def(p.weapon);
    let now = ctx.now();
    let tinker = ctx.view.ship_types.get(ShipType::Tinker).thrust_accel;
    let (range, peak) = well_numbers(def, tinker);
    let ticks = Tick::ticks_for_secs(def.param("well_secs", 6.0)).max(1);
    p.pos = pos;
    p.vel = Vec3::ZERO;
    p.flags |= proj_flags::STUCK | LANDED;
    p.fuse_tick = now + ticks;
    ctx.add_well(DynamicWell {
        key: p.id,
        pos,
        peak_accel: peak,
        range,
        // The ships moved already this tick: it pulls from the next one.
        start_tick: now + 1,
        end_tick: now + 1 + ticks,
        owner: Some(p.owner),
    });
}

/// The well ended: fuse the stone and the debris around it into a lump of
/// weak rock (unless that would trap a ship).
fn fuse(ctx: &mut Ctx, p: &Projectile) {
    let def = ctx.def(p.weapon);
    let reach = def.param("gather_radius", 4.0);
    let gathered: Vec<(u32, f32)> = ctx
        .view
        .entities
        .iter()
        .filter(|e| e.kind == EntityKind::Debris && e.pos.distance(p.pos) <= reach)
        .map(|e| (e.id, e.radius))
        .collect();
    let r = lump_radius(def, gathered.iter().map(|&(_, r)| r));
    let clear = ctx
        .view
        .ships
        .values()
        .filter(|s| s.alive)
        .all(|s| s.pos.distance(p.pos) > r + ctx.tuning_of(s).radius + SHIP_CLEARANCE);
    if !clear || !p.pos.is_finite() {
        return;
    }
    for (id, _) in gathered {
        ctx.push(Command::RemoveEntity { id });
    }
    ctx.edit(TerrainEdit::AddSphere {
        center: p.pos,
        radius: r,
    });
    ctx.weak_rock(p.pos, r);
}

impl WeaponBehavior for Paperweight {
    fn id(&self) -> &'static str {
        "paperweight"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let mut p = projectile_for(ctx, shot, shot.shooter.orientation.forward());
        p.flags |= proj_flags::NO_SHIP_HITS;
        p.damage = 0.0;
        p.splash_damage = 0.0;
        p.knockback = 0.0;
        // The flight ends on the fuse (it lands mid-air); the stone then
        // lives until its well ends (the second fuse).
        p.fuse_tick = p.expires_tick;
        p.expires_tick = u64::MAX;
        ctx.spawn_projectile(p);
    }

    fn update(&self, _ctx: &mut Ctx, p: &mut Projectile) {
        if p.flags & LANDED != 0 {
            // Paperweights stay where you put them (a sneeze pulls it loose).
            p.vel = Vec3::ZERO;
            p.flags |= proj_flags::STUCK;
        }
    }

    fn on_entity_hit(&self, _ctx: &mut Ctx, _p: &mut Projectile, _hit: &EntityHit) -> HitResponse {
        HitResponse::Continue
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        let pos = hit.point + hit.normal * p.radius;
        land(ctx, p, pos);
        HitResponse::Stick
    }

    fn on_fuse(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if p.flags & LANDED == 0 {
            let pos = p.pos;
            land(ctx, p, pos);
        } else {
            fuse(ctx, p);
            p.remove();
        }
    }
}

#[cfg(test)]
#[path = "paperweight_tests.rs"]
mod tests;
