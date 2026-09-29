//! Seed Grenade: special secondary. Grows a small rock structure on impact: block tunnels or
//! build cover (a deterministic cluster of AddSphere edits seeded by the projectile id).
//! Malus: small chance (2%) to grow something absurd, like a giant rubber duck. Lasting
//! effect: the structure stays until destroyed.
//!
//! Numbers: `data/weapons/seed_grenade.ron` (`params`: `grow_spheres`, `grow_radius`,
//! `duck_chance`, `duck_scale`, `grow_on_expire`).
//!
//! Status: implemented.
//! - Terrain hit: light impact (`detonate`, no crater), then a cluster of `grow_spheres`
//!   `AddSphere` edits grows out of the wall: a core sphere on the surface plus spheres
//!   scattered over the half space in front of it, all within about `grow_radius` u. The
//!   layout comes from a private RNG seeded only by the match seed and the projectile id
//!   ([`growth`]), so every peer grows the same rock.
//! - With probability `duck_chance` (same RNG) it grows a giant rubber duck instead: seven
//!   spheres (body, chest, tail, head, beak) `duck_scale` times the unit duck, sitting on
//!   the surface, facing along the throw.
//! - The rock is ordinary terrain: it stays until something blasts it away.
//! - A grenade that runs out of time mid-air grows a floating cluster there if
//!   `grow_on_expire` is non-zero (cover in open sky). A direct ship hit only does the light
//!   damage (no growth inside ships).
//! - Spheres that would overlap a living ship are left out, so nobody gets trapped in rock.

use ls_math::rng::mix64;
use ls_math::terrain::RayHit;
use ls_math::{Pcg32, Vec3};

use crate::behavior::{Ctx, RNG_STREAM_MODULES};
use crate::weapons::{HitResponse, WeaponBehavior};
use crate::world::Projectile;
use crate::WeaponDef;

/// Behaviour of the Seed Grenade.
#[derive(Debug)]
pub struct SeedGrenade;

/// Salt of the growth RNG ("SEEDGROW").
const GROWTH_SALT: u64 = 0x5345_4544_4752_4f57;

/// Rubber duck from spheres (x right, y up from the surface, z forward;
/// unit scale): body, chest, back, tail, head, beak, beak tip.
const DUCK: [([f32; 3], f32); 7] = [
    ([0.0, 0.9, 0.0], 1.0),
    ([0.0, 0.9, 0.55], 0.85),
    ([0.0, 1.15, -0.75], 0.6),
    ([0.0, 1.6, -1.2], 0.35),
    ([0.0, 2.2, 0.6], 0.6),
    ([0.0, 2.1, 1.25], 0.3),
    ([0.0, 2.0, 1.5], 0.2),
];

/// The growth RNG of projectile `id` in a match with `seed`.
fn growth_rng(seed: u64, id: u32) -> Pcg32 {
    Pcg32::new(
        mix64(seed ^ mix64(id as u64 ^ GROWTH_SALT)),
        RNG_STREAM_MODULES,
    )
}

/// Uniform random unit vector.
fn unit(rng: &mut Pcg32) -> Vec3 {
    let z = rng.range_f32(-1.0, 1.0);
    let phi = rng.range_f32(0.0, std::f32::consts::TAU);
    let r = (1.0 - z * z).max(0.0).sqrt();
    let (s, c) = ls_math::det::sin_cos(phi);
    Vec3::new(r * c, r * s, z)
}

/// The spheres (centre, radius) a seed grows at `point` on a surface with
/// `normal`, thrown along `dir`, and whether it is the duck. Deterministic
/// in (`seed`, `id`) and the data.
pub fn growth(
    def: &WeaponDef,
    seed: u64,
    id: u32,
    point: Vec3,
    normal: Vec3,
    dir: Vec3,
) -> (Vec<(Vec3, f32)>, bool) {
    let mut rng = growth_rng(seed, id);
    let up = normal.try_normalize().unwrap_or(Vec3::Y);
    if rng.chance(def.param("duck_chance", 0.02)) {
        let s = def.param("duck_scale", 3.0);
        let flat = dir - up * dir.dot(up);
        let fwd = flat.try_normalize().unwrap_or_else(|| up.any_orthonormal());
        let right = fwd.cross(up);
        let duck = DUCK
            .iter()
            .map(|&([x, y, z], r)| (point + (right * x + up * y + fwd * z) * s, r * s))
            .collect();
        return (duck, true);
    }
    let n = def.param("grow_spheres", 6.0).clamp(1.0, 32.0) as usize;
    let size = def.param("grow_radius", 2.0);
    let mut out = Vec::with_capacity(n);
    out.push((point + up * (0.3 * size), 0.6 * size));
    for _ in 1..n {
        let mut d = unit(&mut rng);
        let along = d.dot(up);
        if along < 0.0 {
            d -= up * (2.0 * along);
        }
        let dist = rng.range_f32(0.3, 1.0) * size;
        let r = rng.range_f32(0.35, 0.65) * size;
        out.push((point + up * (0.3 * size) + d * dist, r));
    }
    (out, false)
}

/// Grows the seed of `p` at `point` (leaving out spheres that would
/// swallow a ship).
fn grow_at(ctx: &mut Ctx, p: &Projectile, point: Vec3, normal: Vec3) {
    let def = ctx.def(p.weapon);
    let (spheres, _duck) = growth(def, ctx.view.seed, p.id, point, normal, p.vel);
    let ships: Vec<(Vec3, f32)> = ctx
        .view
        .ships
        .values()
        .filter(|s| s.alive)
        .map(|s| (s.pos, ctx.tuning_of(s).radius))
        .collect();
    for (c, r) in spheres {
        let clear = ships.iter().all(|&(sp, sr)| sp.distance(c) > r + sr + 0.3);
        if clear {
            ctx.grow(c, r);
        }
    }
}

impl WeaponBehavior for SeedGrenade {
    fn id(&self) -> &'static str {
        "seed_grenade"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        ctx.detonate(p, hit.point, None, false);
        grow_at(ctx, p, hit.point, hit.normal);
        HitResponse::Stop
    }

    fn on_expire(&self, ctx: &mut Ctx, p: &mut Projectile) {
        if ctx.def(p.weapon).param("grow_on_expire", 1.0) <= 0.0 {
            return;
        }
        let normal = -p.vel.normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            Vec3::Y
        } else {
            normal
        };
        // Floating: the core sits on the grenade, not above an absent wall.
        let size = ctx.def(p.weapon).param("grow_radius", 2.0);
        grow_at(ctx, p, p.pos - normal * (0.3 * size), normal);
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::{TerrainEdit, TerrainQuery};
    use ls_math::Vec3;

    use super::super::paint_thrower::testkit::*;
    use super::growth;
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn grown(ev: &[Event]) -> Vec<(Vec3, f32)> {
        edits(ev)
            .into_iter()
            .filter_map(|e| match e {
                TerrainEdit::AddSphere { center, radius } => Some((center, radius)),
                _ => None,
            })
            .collect()
    }

    /// A world whose Seed Grenade has `duck_chance` = `duck`.
    fn world_duck(duck: f32) -> (World, PlayerId, PlayerId) {
        let mut defs: Vec<WeaponDef> = WeaponTable::builtin()
            .iter()
            .map(|(_, d)| d.clone())
            .collect();
        for d in &mut defs {
            if d.id == "seed_grenade" {
                d.params.insert("duck_chance".into(), duck);
            }
        }
        let mut w = World::new(WorldConfig {
            weapons: WeaponTable::new(defs).unwrap(),
            ..WorldConfig::paint_the_void(7, setup())
        })
        .unwrap();
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        (w, a, b)
    }

    /// Throws one grenade from `from` along -z at the far wall (z = -30).
    fn throw(w: &mut World, t: &mut TestRoom, a: PlayerId, from: Vec3) -> Vec<Event> {
        place(w, a, from, Vec3::NEG_Z);
        let mut ev = run(w, t, &press(a, Buttons::FIRE_SECONDARY), 1);
        ev.extend(run(w, t, &[], 150));
        ev
    }

    #[test]
    fn grows_a_rock_cluster_that_stays_until_blasted() {
        let (mut w, a, b) = world_duck(0.0);
        let mut t = TestRoom::new();
        let wid = equip(&mut w, a, "seed_grenade");
        let def = w.weapons().get(wid).clone();
        place(&mut w, b, Vec3::new(20.0, 10.0, 20.0), Vec3::Z);
        let ev = throw(&mut w, &mut t, a, Vec3::new(0.0, 0.0, 0.0));
        let g = grown(&ev);
        let size = def.param("grow_radius", 0.0);
        assert_eq!(g.len(), def.param("grow_spheres", 0.0) as usize);
        let hit = Vec3::new(0.0, 0.0, -30.0);
        for &(c, r) in &g {
            assert!(c.distance(hit) <= 1.4 * size, "{c:?}");
            assert!(r > 0.3 * size && r <= 0.65 * size);
        }
        // New rock where there was open air in front of the wall.
        let front = Vec3::new(0.0, 0.0, -29.0);
        assert!(t.sdf(front) < 0.0, "cover grew");
        // It stays...
        run(&mut w, &mut t, &[], 1200);
        assert!(t.sdf(front) < 0.0);
        // ...and blocks shots: a laser bolt hits the new rock before the wall.
        place(&mut w, a, Vec3::new(0.0, 0.0, -10.0), Vec3::NEG_Z);
        let ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
        let ev = [ev, run(&mut w, &mut t, &[], 60)].concat();
        let impact = ev
            .iter()
            .find_map(|e| match e {
                Event::Impact { point, .. } => Some(*point),
                _ => None,
            })
            .unwrap();
        assert!(impact.z > -29.5, "stopped by the growth: {impact:?}");
        // Until destroyed: a rocket blasts it open again.
        equip(&mut w, a, "bolt_rocket");
        let _ = throw(&mut w, &mut t, a, Vec3::new(0.0, 0.0, -10.0));
        let _ = throw(&mut w, &mut t, a, Vec3::new(0.0, 0.0, -10.0));
        let _ = throw(&mut w, &mut t, a, Vec3::new(0.0, 0.0, -10.0));
        assert!(t.sdf(front) > 0.0, "blasted away");
    }

    #[test]
    fn growth_is_deterministic_per_projectile_id() {
        let clusters = |seed_shift: u32| {
            let (mut w, a, _b) = world_duck(0.0);
            let mut t = TestRoom::new();
            let wid = equip(&mut w, a, "seed_grenade");
            // Different projectile ids: fire some lasers first.
            for _ in 0..seed_shift {
                run(&mut w, &mut t, &press(a, Buttons::FIRE_PRIMARY), 1);
                run(&mut w, &mut t, &[], 20);
            }
            w.ship_mut(a).unwrap().weapon_mut(wid).unwrap().ammo = 3;
            grown(&throw(&mut w, &mut t, a, Vec3::ZERO))
        };
        let first = clusters(0);
        assert_eq!(first, clusters(0), "same id, same rock");
        assert_ne!(first, clusters(2), "another id grows another cluster");
        // The layout depends only on (seed, id), not on the tick.
        let def = WeaponTable::builtin()
            .get(WeaponTable::builtin().find("seed_grenade").unwrap())
            .clone();
        let at = |dz: f32| {
            growth(&def, 5, 17, Vec3::new(0.0, 0.0, dz), Vec3::Z, Vec3::NEG_Z)
                .0
                .iter()
                .map(|(c, r)| (*c - Vec3::new(0.0, 0.0, dz), *r))
                .collect::<Vec<_>>()
        };
        let (x, y) = (at(0.0), at(5.0));
        assert_eq!(x.len(), y.len());
        for ((cx, rx), (cy, ry)) in x.iter().zip(&y) {
            assert_eq!(rx, ry);
            assert!(cx.distance(*cy) < 1e-4, "same shape wherever it lands");
        }
    }

    #[test]
    fn about_two_percent_grow_a_giant_rubber_duck() {
        let def = WeaponTable::builtin()
            .get(WeaponTable::builtin().find("seed_grenade").unwrap())
            .clone();
        assert_eq!(def.param("duck_chance", 0.0), 0.02);
        let n = 10_000u32;
        let ducks = (0..n)
            .filter(|&id| growth(&def, 99, id, Vec3::ZERO, Vec3::Y, Vec3::Z).1)
            .count();
        let rate = ducks as f32 / n as f32;
        assert!((0.012..=0.028).contains(&rate), "duck rate {rate}");

        // A duck is bigger than any cluster and sits on the surface.
        let (mut w, a, b) = world_duck(1.0);
        let mut t = TestRoom::new();
        equip(&mut w, a, "seed_grenade");
        place(&mut w, b, Vec3::new(20.0, 10.0, 20.0), Vec3::Z);
        let ev = throw(&mut w, &mut t, a, Vec3::ZERO);
        let g = grown(&ev);
        assert_eq!(g.len(), 7, "duck spheres");
        let reach = g
            .iter()
            .map(|(c, r)| c.distance(Vec3::new(0.0, 0.0, -30.0)) + r)
            .fold(0.0f32, f32::max);
        let size = def.param("grow_radius", 0.0);
        assert!(reach > 3.0 * size, "giant: {reach}");
        // Head up out of the wall (+z is away from the far wall).
        assert!(g.iter().any(|(c, _)| c.z > -30.0 + 5.0));
    }

    #[test]
    fn never_grows_rock_into_a_ship() {
        let (mut w, a, b) = world_duck(0.0);
        let mut t = TestRoom::new();
        equip(&mut w, a, "seed_grenade");
        let rb = w.tuning().radius;
        // b hovers right next to the impact point.
        place(&mut w, b, Vec3::new(1.5, 0.0, -28.8), Vec3::Z);
        let ev = throw(&mut w, &mut t, a, Vec3::ZERO);
        let g = grown(&ev);
        let bp = w.ship(b).unwrap().pos;
        assert!(!g.is_empty(), "grows the rest");
        for (c, r) in g {
            assert!(
                c.distance(bp) > r + rb,
                "sphere {c:?} r {r} swallows b at {bp:?}"
            );
        }
    }

    #[test]
    fn a_grenade_that_expires_mid_air_grows_floating_cover() {
        let (mut w, a, b) = world_duck(0.0);
        // A huge room: the grenade runs out of time before any wall.
        let mut t = TestRoom::new();
        t.room = ls_math::Aabb::from_center_half_extents(Vec3::ZERO, Vec3::splat(200.0));
        let wid = equip(&mut w, a, "seed_grenade");
        let def = w.weapons().get(wid).clone();
        place(&mut w, b, Vec3::new(50.0, 0.0, 0.0), Vec3::Z);
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &press(a, Buttons::FIRE_SECONDARY), 1);
        ev.extend(run(&mut w, &mut t, &[], 300));
        let g = grown(&ev);
        assert_eq!(g.len(), def.param("grow_spheres", 0.0) as usize);
        let end = def.speed * def.lifetime_secs;
        assert!(g.iter().all(|(c, _)| (c.z + end).abs() < 8.0), "{g:?}");
    }
}
