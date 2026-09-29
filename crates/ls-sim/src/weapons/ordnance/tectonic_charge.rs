//! Tectonic Charge: ordnance secondary. Earthshaker-class, 15 u/s (the slowest projectile).
//! Sticks where it hits, 1.5 s fuse, main blast then 6 sub-charges that scatter up to 20 u
//! and explode 0.5 s later. Zones: core 4 u instant kill, damage 25 u (falloff), collapse 35
//! u (terrain caves in, Instability sets how far cracks spread), shockwave 60 u (knockback,
//! no damage). Dust cloud 20 s, large debris field. One per map, 90 s respawn, a ship carries
//! one; damages its owner.
//!
//! Numbers: `data/weapons/tectonic_charge.ron` (`params`: `core_radius`, `core_damage`,
//! `damage_radius`, `collapse_radius`, `crack_count`, `crack_max`, `crack_radius`,
//! `crack_spacing`, `shockwave_radius`, `shockwave_strength`, `fuse_secs`, `sub_charges`,
//! `sub_scatter`, `sub_min_scatter`, `sub_delay_secs`, `sub_damage`, `sub_radius`,
//! `sub_crater`, `dust_secs`, `dust_radius`, `debris_count`, `debris_scrap`, `jostle_speed`).
//! The limits are data too: `max_ammo: 1` here, `max_on_map: 1` and `respawn_secs: 90` in the
//! rule sets' pickup table.
//!
//! Status: implemented, following the design doc step by step:
//! 1. **Launch** (`fire`): a straight 15 u/s charge and a `TectonicLaunch` cue (the
//!    level-wide rumble; cues go to every client).
//! 2. **Impact**: it sticks to whatever it hits: rock (`HitResponse::Stick`), or a ship,
//!    debris piece or decoy (it then rides along with it until it is gone). The fuse starts (`fuse_secs`)
//!    with a `TectonicArmed` cue (the ground cracks; `value` = fuse ticks). A charge that
//!    hits nothing arms in place when its lifetime runs out.
//! 3. **Detonation** (`on_fuse`): the main blast: every ship within `core_radius` takes
//!    `core_damage` (an instant kill through any shield), the damage zone is a splash of the
//!    data's splash damage falling off to 0 at `damage_radius`, the shockwave pushes ships
//!    and debris out to `shockwave_radius` without damage (`Shockwave` cue: screen shake).
//!    Then `sub_charges` sub-charges scatter `sub_min_scatter..=sub_scatter` u (they fly
//!    through ships and debris, stick to rock) and explode `sub_delay_secs` later with a
//!    `sub_radius` splash and a `sub_crater` crater.
//! 4. **Aftermath**: the collapse zone caves in whatever the Instability slider says: one
//!    exact (not rock-strength scaled) `collapse_radius` crater. Instability sets how far
//!    `crack_count` cracks run into the rock beyond it: `crack_max x instability / 100` u of
//!    strength-scaled craters every `crack_spacing` u, narrowing towards the tip. A dust
//!    cloud of `dust_radius` lasts `dust_secs`; `debris_count` scrap-filled debris pieces
//!    spread through the collapse zone (the debris cap removes the oldest); every loose
//!    debris piece in the level is jostled by `jostle_speed`.
//!
//! Friendly fire follows the rule set (a teammate in the core survives only with friendly
//! fire off); the owner is hurt like anyone else.

use ls_math::terrain::RayHit;
use ls_math::{Pcg32, Tick, Vec3, DT};

use crate::behavior::{Command, Ctx, Cue, CueKind, DamageSource};
use crate::entity::{EntityKind, EntitySpawn, Source};
use crate::weapons::{
    projectile_for, sub_projectile, EntityHit, HitResponse, ShipHit, Shot, WeaponBehavior,
};
use crate::world::{proj_flags, Projectile};
use crate::Event;

/// Behaviour of the Tectonic Charge.
#[derive(Debug)]
pub struct TectonicCharge;

/// Projectile flag: the charge rides on the ship in `target` (offset in `vars[0..3]`).
const ON_SHIP: u16 = proj_flags::CUSTOM_A;
/// Projectile flag: the charge rides on the entity (debris, decoy) whose id
/// bits are in `vars[3]` (offset in `vars[0..3]`).
const ON_ENTITY: u16 = proj_flags::CUSTOM_B;

/// RNG keys (xor'ed with the projectile id) for the independent random draws.
const KEY_SUBS: u64 = 1 << 40;
const KEY_CRACKS: u64 = 2 << 40;
const KEY_DEBRIS: u64 = 3 << 40;
const KEY_JOSTLE: u64 = 4 << 40;

/// A uniformly distributed unit vector.
fn random_dir(rng: &mut Pcg32) -> Vec3 {
    let z = rng.range_f32(-1.0, 1.0);
    let phi = rng.range_f32(0.0, std::f32::consts::TAU);
    let r = (1.0 - z * z).max(0.0).sqrt();
    let (s, c) = ls_math::det::sin_cos(phi);
    Vec3::new(r * c, r * s, z)
}

/// Starts the fuse where the charge stuck. `normal` points away from the rock it hit (zero
/// when it did not hit rock); it is kept in `vars[0..3]` unless the charge rides a ship.
fn arm(ctx: &mut Ctx, p: &mut Projectile, point: Vec3, normal: Vec3) {
    let fuse = Tick::ticks_for_secs(ctx.def(p.weapon).param("fuse_secs", 1.5)).max(1);
    p.fuse_tick = ctx.now() + fuse;
    p.expires_tick = p.expires_tick.max(p.fuse_tick + 1);
    p.flags |= proj_flags::ARMED;
    if p.flags & (ON_SHIP | ON_ENTITY) == 0 {
        p.vars = [normal.x, normal.y, normal.z, 0.0];
    }
    ctx.cue(Cue {
        kind: CueKind::TectonicArmed,
        source: Some(p.owner),
        target: None,
        pos: point,
        value: fuse.min(u16::MAX as u64) as u16,
    });
}

fn armed(p: &Projectile) -> bool {
    p.flags & proj_flags::ARMED != 0
}

impl TectonicCharge {
    /// The main blast, the sub-charges and the aftermath (see the module docs).
    fn main_blast(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let def = ctx.def(p.weapon);
        let prm = |k: &str, d: f32| def.param(k, d);
        let center = p.pos;
        let source = DamageSource::projectile(p);
        let normal = if p.flags & (ON_SHIP | ON_ENTITY) == 0 {
            Vec3::new(p.vars[0], p.vars[1], p.vars[2])
        } else {
            Vec3::ZERO
        };
        ctx.emit(Event::Impact {
            projectile: p.id,
            weapon: p.weapon,
            point: center,
            target: None,
        });

        // Core: instant kill.
        let core_damage = prm("core_damage", 10_000.0);
        let core: Vec<_> = ctx
            .ships_within(center, prm("core_radius", 4.0))
            .into_iter()
            .map(|s| s.player)
            .collect();
        for player in core {
            ctx.damage(player, core_damage, Vec3::ZERO, source, true);
        }
        // Damage zone: heavy damage falling off to its edge.
        ctx.push(Command::Splash {
            center,
            radius: prm("damage_radius", 25.0),
            damage: p.splash_damage,
            knockback: p.knockback,
            source,
            hits_owner: true,
            exclude: None,
            dir: normal,
        });
        // Shockwave: knockback and screen shake, no damage.
        ctx.shockwave(
            center,
            prm("shockwave_radius", 60.0),
            prm("shockwave_strength", 40.0),
        );
        // Collapse: the zone caves in whatever the Instability says.
        let collapse = prm("collapse_radius", 35.0);
        ctx.crater_exact(center, collapse);

        // Sub-charges.
        let n = prm("sub_charges", 6.0).max(0.0) as usize;
        let delay = Tick::ticks_for_secs(prm("sub_delay_secs", 0.5)).max(2);
        let scatter = prm("sub_scatter", 20.0).max(0.0);
        let min_scatter = prm("sub_min_scatter", 8.0).clamp(0.0, scatter);
        let mult = if def.damage > 0.0 {
            p.damage / def.damage
        } else {
            1.0
        };
        let mut rng = ctx.rng(p.id as u64 ^ KEY_SUBS);
        for _ in 0..n {
            let mut dir = random_dir(&mut rng);
            if dir.dot(normal) < 0.0 {
                dir = -dir;
            }
            let dist = rng.range_f32(min_scatter, scatter);
            // They move on the ticks between this one and their fuse tick.
            let vel = dir * (dist / ((delay - 1) as f32 * DT));
            let mut sub = sub_projectile(ctx, p, center, vel, delay + 1);
            sub.fuse_tick = ctx.now() + delay;
            sub.splash_damage = prm("sub_damage", 35.0) * mult;
            sub.knockback = p.knockback * 0.5;
            sub.flags = proj_flags::NO_SHIP_HITS | proj_flags::ARMED;
            ctx.spawn_projectile(sub);
        }

        // Aftermath: dust.
        let dust_ticks = Tick::ticks_for_secs(prm("dust_secs", 20.0));
        if dust_ticks > 0 {
            ctx.spawn_entity(
                EntitySpawn::dust(center, prm("dust_radius", collapse), dust_ticks)
                    .from_source(Source::Weapon(p.weapon)),
            );
        }
        // Every loose debris piece in the level jostles (before the new field pushes the
        // oldest out).
        let jostle = prm("jostle_speed", 3.0);
        if jostle > 0.0 {
            let loose: Vec<(u32, f32)> = ctx
                .view
                .entities
                .iter()
                .filter(|e| e.kind == EntityKind::Debris)
                .map(|e| (e.id, e.mass()))
                .collect();
            for (id, mass) in loose {
                let mut r = ctx.rng(p.id as u64 ^ KEY_JOSTLE ^ ((id as u64) << 8));
                ctx.impulse_entity(id, random_dir(&mut r) * (jostle * mass));
            }
        }
        // A large debris field full of scrap.
        let pieces = prm("debris_count", 24.0).max(0.0) as usize;
        let scrap = prm("debris_scrap", 6.0).max(0.0);
        let mut rng = ctx.rng(p.id as u64 ^ KEY_DEBRIS);
        for _ in 0..pieces {
            let dir = random_dir(&mut rng);
            let pos = center + dir * (collapse * rng.range_f32(0.15, 0.85));
            let vel = dir * rng.range_f32(1.0, 4.0);
            let radius = rng.range_f32(0.5, 1.5);
            ctx.spawn_debris(pos, vel, radius, scrap * rng.range_f32(0.5, 1.5));
        }
        // Cracks beyond the collapse zone, as far as the Instability says (into the rock
        // behind the surface it stuck to).
        let reach = prm("crack_max", 30.0) * ctx.view.instability.min(100) as f32 / 100.0;
        let spacing = prm("crack_spacing", 2.5);
        let crack_radius = prm("crack_radius", 1.5);
        if reach > 0.0 && spacing > 0.0 && crack_radius > 0.0 {
            let steps = ls_math::det::ceil(reach / spacing) as usize;
            let mut rng = ctx.rng(p.id as u64 ^ KEY_CRACKS);
            for _ in 0..prm("crack_count", 8.0).max(0.0) as usize {
                let mut dir = random_dir(&mut rng);
                if dir.dot(normal) > 0.0 {
                    dir = -dir;
                }
                for k in 1..=steps {
                    let along = (k as f32 * spacing).min(reach);
                    let jitter = random_dir(&mut rng) * (spacing * 0.3);
                    let at = center + dir * (collapse + along) + jitter;
                    let r = crack_radius * (1.0 - 0.5 * k as f32 / steps as f32);
                    if ctx.view.terrain.sdf(at) < r {
                        ctx.crater(at, r);
                    }
                }
            }
        }
    }

    /// A sub-charge explodes where it is.
    fn sub_blast(&self, ctx: &mut Ctx, p: &mut Projectile) {
        let def = ctx.def(p.weapon);
        let point = p.pos;
        ctx.emit(Event::Impact {
            projectile: p.id,
            weapon: p.weapon,
            point,
            target: None,
        });
        ctx.push(Command::Splash {
            center: point,
            radius: def.param("sub_radius", 6.0),
            damage: p.splash_damage,
            knockback: p.knockback,
            source: DamageSource::projectile(p),
            hits_owner: true,
            exclude: None,
            dir: Vec3::ZERO,
        });
        let crater = def.param("sub_crater", 2.5);
        if crater > 0.0 && ctx.view.terrain.sdf(point) < crater {
            ctx.crater(point, crater);
        }
    }
}

impl WeaponBehavior for TectonicCharge {
    fn id(&self) -> &'static str {
        "tectonic_charge"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn fire(&self, ctx: &mut Ctx, shot: &Shot) {
        let p = projectile_for(ctx, shot, shot.shooter.orientation.forward());
        ctx.cue(Cue {
            kind: CueKind::TectonicLaunch,
            source: Some(shot.shooter.player),
            target: None,
            pos: p.pos,
            value: 0,
        });
        ctx.spawn_projectile(p);
    }

    fn update(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if p.generation > 0 {
            return;
        }
        if p.flags & ON_SHIP != 0 {
            let ship = p.target.and_then(|t| ctx.ship(t)).filter(|s| s.alive);
            match ship {
                Some(s) => p.pos = s.pos + Vec3::new(p.vars[0], p.vars[1], p.vars[2]),
                // The ship died: the charge stays where it was.
                None => p.flags &= !ON_SHIP,
            }
        }
        if p.flags & ON_ENTITY != 0 {
            let id = p.vars[3].to_bits();
            match ctx.view.entities.iter().find(|e| e.id == id) {
                Some(e) => p.pos = e.pos + Vec3::new(p.vars[0], p.vars[1], p.vars[2]),
                // The entity is gone: the charge stays where it was.
                None => p.flags &= !ON_ENTITY,
            }
        }
        if !armed(p) && ctx.now() + 1 >= p.expires_tick {
            // Hit nothing: it arms in place.
            p.vel = Vec3::ZERO;
            p.flags |= proj_flags::STUCK;
            let pos = p.pos;
            arm(ctx, p, pos, Vec3::ZERO);
        }
    }

    fn on_ship_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &ShipHit) -> HitResponse {
        if p.generation > 0 || armed(p) {
            return HitResponse::Continue;
        }
        let Some(ship) = ctx.ship(hit.target) else {
            return HitResponse::Continue;
        };
        // Where `Stick` leaves it, relative to the ship.
        let offset = hit.point + hit.normal * p.radius - ship.pos;
        p.flags |= ON_SHIP;
        p.target = Some(hit.target);
        p.vars = [offset.x, offset.y, offset.z, 0.0];
        arm(ctx, p, hit.point, Vec3::ZERO);
        HitResponse::Stick
    }

    fn on_entity_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &EntityHit) -> HitResponse {
        if p.generation > 0 || armed(p) {
            return HitResponse::Continue;
        }
        // Ride the entity (a decoy flying back, pushed debris).
        if let Some(e) = ctx.view.entities.iter().find(|e| e.id == hit.id) {
            let offset = hit.point + hit.normal * p.radius - e.pos;
            p.flags |= ON_ENTITY;
            p.vars = [offset.x, offset.y, offset.z, f32::from_bits(hit.id)];
        }
        arm(ctx, p, hit.point, Vec3::ZERO);
        HitResponse::Stick
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        if p.generation == 0 && !armed(p) {
            arm(ctx, p, hit.point, hit.normal);
        }
        HitResponse::Stick
    }

    fn on_fuse(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if p.generation == 0 {
            self.main_blast(ctx, p);
        } else {
            self.sub_blast(ctx, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{BoxRoomTerrain, TerrainEdit, TerrainQuery};
    use ls_math::Vec3;

    use crate::entity::{EntityKind, EntitySpawn};
    use crate::weapons::behavior;
    use crate::weapons::ordnance::heavy_rocket::testkit::*;
    use crate::world::proj_flags;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn charge(w: &World) -> Option<&Projectile> {
        let id = w.weapons().find("tectonic_charge").unwrap();
        w.projectiles()
            .iter()
            .find(|p| p.weapon == id && p.generation == 0)
    }

    fn subs(w: &World) -> Vec<Projectile> {
        let id = w.weapons().find("tectonic_charge").unwrap();
        w.projectiles()
            .iter()
            .filter(|p| p.weapon == id && p.generation > 0)
            .cloned()
            .collect()
    }

    fn armed_cue(ev: &[Event]) -> bool {
        !cues(ev, CueKind::TectonicArmed).is_empty()
    }

    fn blast(ev: &[Event]) -> bool {
        !impacts(ev).is_empty()
    }

    /// Room 200 (floor at y = -100); the owner fires a charge straight down onto the floor
    /// and flies off to a far corner. Returns the world, terrain, owner and the stuck
    /// charge's position, right after it stuck.
    fn floor_charge(instability: u8) -> (World, BoxRoomTerrain, PlayerId, Vec3) {
        let mut w = world_with(instability, RuleSet::paint_the_void());
        let mut t = room(200.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "tectonic_charge");
        place(&mut w, a, Vec3::new(0.0, -94.0, 0.0), Vec3::NEG_Y);
        run(&mut w, &mut t, &fire2(a), 1);
        place(&mut w, a, Vec3::new(90.0, 90.0, 90.0), Vec3::NEG_Y);
        run_until(&mut w, &mut t, 120, armed_cue);
        let pos = charge(&w).unwrap().pos;
        (w, t, a, pos)
    }

    #[test]
    fn numbers_and_limits_follow_the_design_doc() {
        let table = WeaponTable::builtin();
        let d = table.get(table.find("tectonic_charge").unwrap());
        for (_, other) in table.iter() {
            // Mines are dropped or creep; everything that flies is faster.
            if other.family == WeaponFamily::Ordnance && !other.id.ends_with("_mine") {
                assert!(other.speed >= d.speed, "slowest missile: {}", other.id);
            }
        }
        assert_eq!(d.speed, 15.0);
        let p = |k: &str| d.params[k];
        assert_eq!(p("core_radius"), 4.0);
        assert_eq!(p("damage_radius"), 25.0);
        assert_eq!(p("collapse_radius"), 35.0);
        assert_eq!(p("shockwave_radius"), 60.0);
        assert_eq!(p("fuse_secs"), 1.5);
        assert_eq!(p("sub_charges"), 6.0);
        assert_eq!(p("sub_scatter"), 20.0);
        assert_eq!(p("sub_delay_secs"), 0.5);
        assert_eq!(p("dust_secs"), 20.0);
        // One per map, 90 s respawn, a ship carries one.
        assert_eq!(d.max_ammo, 1);
        let rules = RuleSet::paint_the_void();
        let pick = &rules.pickups["tectonic_charge"];
        assert_eq!((pick.respawn_secs, pick.max_on_map), (90.0, 1));
        let mut w = world();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        w.give_weapon(a, "tectonic_charge").unwrap();
        assert!(
            w.give_weapon(a, "tectonic_charge").is_err(),
            "carry limit 1"
        );
        let wid = w.weapons().find("tectonic_charge").unwrap();
        assert_eq!(w.ship(a).unwrap().weapon(wid).unwrap().ammo, 1);
        assert!(behavior("tectonic_charge").implemented());
    }

    #[test]
    fn launch_rumbles_then_it_sticks_and_cracks_for_1_5_s() {
        let mut w = world();
        let mut t = room(200.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "tectonic_charge");
        place(&mut w, a, Vec3::new(0.0, -94.0, 0.0), Vec3::NEG_Y);
        let ev = run(&mut w, &mut t, &fire2(a), 1);
        let launch = cues(&ev, CueKind::TectonicLaunch);
        assert_eq!(launch.len(), 1, "level-wide rumble at launch");
        assert_eq!(launch[0].source, Some(a));
        assert!((charge(&w).unwrap().vel.length() - 15.0).abs() < 1e-4);
        place(&mut w, a, Vec3::new(90.0, 90.0, 90.0), Vec3::NEG_Y);
        let ev = run_until(&mut w, &mut t, 120, armed_cue);
        let stuck_at = w.tick();
        let armed = cues(&ev, CueKind::TectonicArmed)[0];
        assert_eq!(armed.value, 90, "1.5 s fuse");
        let c = charge(&w).unwrap().clone();
        assert!(c.is_stuck() && c.flags & proj_flags::ARMED != 0);
        assert!((c.pos.y - (-100.0 + c.radius)).abs() < 1e-3, "on the floor");
        let ev = run(&mut w, &mut t, &[], 89);
        assert!(impacts(&ev).is_empty());
        assert_eq!(charge(&w).unwrap().pos, c.pos, "stays stuck");
        let ev = run(&mut w, &mut t, &[], 1);
        assert_eq!(w.tick(), stuck_at + 90);
        assert_eq!(impacts(&ev).len(), 1, "main blast 1.5 s after impact");
        assert!(charge(&w).is_none());
    }

    #[test]
    fn zones_core_damage_and_shockwave() {
        let mut w = world();
        let mut t = room(200.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "tectonic_charge");
        // It sticks to a chunk of debris floating in the middle of the room.
        w.spawn_entity(EntitySpawn::debris(Vec3::ZERO, Vec3::ZERO, 1.0, 0.0));
        place(&mut w, a, Vec3::new(0.0, 0.0, 6.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &fire2(a), 1);
        run_until(&mut w, &mut t, 60, armed_cue);
        let c = charge(&w).unwrap().pos;
        // (distance, direction) per victim.
        let spots = [
            (3.9, Vec3::X),                   // core: killed
            (4.1, Vec3::NEG_X),               // just outside the core: survives
            (12.5, Vec3::Y),                  // half-way: half damage
            (24.5, Vec3::NEG_Y),              // edge of the damage zone
            (25.5, Vec3::Z),                  // shockwave only
            (59.0, Vec3::NEG_Z),              // shockwave edge
            (61.0, Vec3::new(0.6, 0.8, 0.0)), // outside everything
        ];
        let victims: Vec<PlayerId> = spots
            .iter()
            .map(|&(d, dir)| {
                let v = w.add_player(Some(1), ShipType::Tinker).unwrap();
                place(&mut w, v, c + dir * d, Vec3::NEG_Z);
                v
            })
            .collect();
        // The owner is hurt like anyone else.
        place(&mut w, a, c + Vec3::new(-8.0, 0.0, -8.0), Vec3::NEG_Z);
        let ev = run_until(&mut w, &mut t, 120, blast);
        let dmg: Vec<f32> = victims.iter().map(|v| damage_to(&ev, *v)).collect();
        assert!(killed(&ev, victims[0]), "core kills");
        assert!(!killed(&ev, victims[1]));
        let expect = |d: f32| 110.0 * (1.0 - d / 25.0);
        assert!((dmg[1] - expect(4.1)).abs() < 0.01, "{}", dmg[1]);
        assert!((dmg[2] - expect(12.5)).abs() < 0.01, "{}", dmg[2]);
        assert!(dmg[3] > 0.0 && dmg[3] < 3.0, "{}", dmg[3]);
        assert_eq!(dmg[4], 0.0, "no damage beyond 25 u");
        assert_eq!(dmg[5], 0.0);
        assert_eq!(dmg[6], 0.0);
        let owner = damage_to(&ev, a);
        assert!((owner - expect(8.0 * 2f32.sqrt())).abs() < 0.01, "{owner}");
        // Knockback out to 60 u, pointing away from the blast.
        for (i, &(d, dir)) in spots.iter().enumerate().skip(4) {
            let vel = w.ship(victims[i]).unwrap().vel;
            if d < 60.0 {
                let expect = 40.0 * (1.0 - d / 60.0);
                assert!((vel.dot(dir) - expect).abs() < 0.01, "{d}: {vel:?}");
            } else {
                assert_eq!(vel, Vec3::ZERO, "{d}");
            }
        }
        let shake = cues(&ev, CueKind::Shockwave);
        assert_eq!(shake.len(), 1);
        assert_eq!(shake[0].value, 60, "screen shake to 60 u");
    }

    #[test]
    fn six_sub_charges_scatter_up_to_20_u_and_explode_0_5_s_later() {
        let (mut w, mut t, _a, c) = floor_charge(0);
        let ev = run_until(&mut w, &mut t, 120, blast);
        let main_tick = w.tick();
        assert_eq!(impacts(&ev).len(), 1);
        let s = subs(&w);
        assert_eq!(s.len(), 6);
        let ids: Vec<u32> = s.iter().map(|p| p.id).collect();
        let ev = run(&mut w, &mut t, &[], 29);
        assert!(impacts(&ev).is_empty());
        let ev = run(&mut w, &mut t, &[], 1);
        assert_eq!(w.tick(), main_tick + 30, "0.5 s");
        let hits = impacts(&ev);
        assert_eq!(hits.len(), 6);
        let mut far = 0.0f32;
        for (id, at) in hits {
            assert!(ids.contains(&id));
            let d = at.distance(c);
            assert!((8.0 - 1e-3..=20.0 + 1e-3).contains(&d), "{d}");
            assert!(at.y >= c.y - 1e-3, "scattered away from the floor: {at:?}");
            far = far.max(d);
        }
        assert!(far > 14.0, "{far}");
        assert!(subs(&w).is_empty());
    }

    fn geometry(ev: &[Event]) -> Vec<(Vec3, f32)> {
        terrain_edits(ev)
            .into_iter()
            .filter_map(|e| match e {
                TerrainEdit::SubtractSphere { center, radius } => Some((center, radius)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn collapse_caves_in_35_u_and_instability_sets_the_cracks() {
        let mut reach = Vec::new();
        for instability in [0u8, 50, 100] {
            let (mut w, mut t, _a, c) = floor_charge(instability);
            let ev = run_until(&mut w, &mut t, 120, blast);
            let edits = geometry(&ev);
            assert_eq!(edits[0], (c, 35.0), "exact 35 u collapse");
            let cracks = &edits[1..];
            let extent = 30.0 * instability as f32 / 100.0;
            let max = cracks
                .iter()
                .map(|(p, _)| p.distance(c))
                .fold(0.0f32, f32::max);
            if instability == 0 {
                assert!(cracks.is_empty(), "no cracks at Instability 0");
            } else {
                assert!(!cracks.is_empty());
                for (p, r) in cracks {
                    let d = p.distance(c);
                    assert!(d > 35.0 - 1.0 && d <= 35.0 + extent + 1.0, "{d}");
                    assert!(p.y < c.y, "into the rock below: {p:?}");
                    assert!(*r <= 1.5);
                }
                assert!(max > 35.0 + extent - 3.0, "{instability}: {max}");
            }
            // The rock below really caved in.
            assert!(t.sdf(c + Vec3::new(0.0, -30.0, 0.0)) > 0.0);
            reach.push(max);
        }
        assert!(reach[2] > reach[1] + 10.0, "{reach:?}");
    }

    #[test]
    fn aftermath_dust_debris_field_and_level_wide_jostle() {
        let (mut w, mut t, _a, c) = floor_charge(0);
        // Loose debris far outside the shockwave (about 140 u away), resting.
        let far = w.spawn_entity(EntitySpawn::debris(
            Vec3::new(60.0, 0.0, 60.0),
            Vec3::ZERO,
            0.5,
            2.0,
        ));
        run(&mut w, &mut t, &[], 1);
        assert!(w.entities().iter().find(|e| e.id == far).unwrap().resting);
        let debris_before = w
            .entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Debris)
            .count();
        run_until(&mut w, &mut t, 120, blast);
        let blast_tick = w.tick();
        let e = w.entities().iter().find(|e| e.id == far).unwrap();
        assert!(!e.resting && e.vel.length() > 1.0, "jostled: {:?}", e.vel);
        let dust: Vec<_> = w
            .entities()
            .iter()
            .filter(|e| e.kind == EntityKind::DustCloud)
            .collect();
        assert_eq!(dust.len(), 1);
        assert_eq!(dust[0].pos, c);
        assert_eq!(dust[0].radius, 35.0);
        assert_eq!(dust[0].expires_tick, blast_tick + 20 * 60, "20 s");
        let field: Vec<_> = w
            .entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Debris && e.id != far)
            .collect();
        assert_eq!(field.len() + 1 - debris_before, 24);
        let scrap: f32 = field.iter().map(|e| e.scrap).sum();
        assert!(scrap > 24.0 * 3.0, "full of scrap: {scrap}");
        assert!(field.iter().all(|e| e.pos.distance(c) < 35.0));
        run(&mut w, &mut t, &[], 20 * 60);
        assert!(!w.entities().iter().any(|e| e.kind == EntityKind::DustCloud));
    }

    #[test]
    fn rides_a_ship_it_hits() {
        let mut w = world();
        let mut t = room(200.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        equip(&mut w, a, "tectonic_charge");
        place(&mut w, a, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::NEG_Z);
        run(&mut w, &mut t, &fire2(a), 1);
        place(&mut w, a, Vec3::new(90.0, 90.0, 90.0), Vec3::NEG_Z);
        let ev = run_until(&mut w, &mut t, 60, armed_cue);
        assert_eq!(damage_to(&ev, b), 0.0, "sticking does no damage");
        // The victim flees 20 u; the charge comes along.
        place(&mut w, b, Vec3::new(20.0, 0.0, 0.0), Vec3::NEG_Z);
        let ev = run_until(&mut w, &mut t, 120, blast);
        let (_, at) = impacts(&ev)[0];
        assert!(at.x > 19.0, "{at:?}");
        assert!(killed(&ev, b), "in the core");
    }

    #[test]
    fn rides_debris_it_hits() {
        let mut w = world();
        let mut t = room(200.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "tectonic_charge");
        place(&mut w, a, Vec3::new(0.0, 0.0, 8.0), Vec3::NEG_Z);
        // A chunk drifting sideways across the line of fire.
        w.spawn_entity(EntitySpawn::debris(
            Vec3::new(-0.5, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            1.0,
            1.0,
        ));
        run(&mut w, &mut t, &fire2(a), 1);
        place(&mut w, a, Vec3::new(90.0, 90.0, 90.0), Vec3::NEG_Z);
        let ev = run_until(&mut w, &mut t, 60, armed_cue);
        let armed_at = cues(&ev, CueKind::TectonicArmed)[0].pos;
        let ev = run_until(&mut w, &mut t, 120, blast);
        let (_, at) = impacts(&ev)[0];
        assert!(
            at.x > armed_at.x + 0.5,
            "carried along: {armed_at:?} -> {at:?}"
        );
    }

    #[test]
    fn arms_in_mid_air_at_the_end_of_its_life() {
        // Fired into open space it flies 10 s (150 u), then arms where it is.
        let mut w = world();
        let mut t = room(400.0);
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        equip(&mut w, a, "tectonic_charge");
        place(&mut w, a, Vec3::new(0.0, 0.0, 100.0), Vec3::NEG_Z);
        run(&mut w, &mut t, &fire2(a), 1);
        let fired = w.tick();
        let ev = run_until(&mut w, &mut t, 700, armed_cue);
        assert_eq!(w.tick() - fired, 599, "armed on its last tick of flight");
        let pos = cues(&ev, CueKind::TectonicArmed)[0].pos;
        assert!((pos.z - (100.0 - 150.0)).abs() < 2.0, "{pos:?}");
        let ev = run_until(&mut w, &mut t, 100, blast);
        assert_eq!(w.tick() - fired, 599 + 90);
        assert_eq!(impacts(&ev)[0].1, pos);
    }
}
