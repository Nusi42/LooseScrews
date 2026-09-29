//! Seed Grenade: special secondary. Grows a small rock structure on impact: block tunnels or
//! build cover (a deterministic cluster of AddSphere edits seeded by the projectile id).
//! Malus: small chance (2%) to grow something absurd, like a giant rubber duck. Lasting
//! effect: the structure stays until destroyed.
//!
//! Numbers: `data/weapons/seed_grenade.ron` (`params`: `grow_spheres`, `grow_radius`,
//! `duck_chance`).
//!
//! Status: TODO(wave-B). Fallback: straight grenade with light damage.
//! To implement: `on_terrain_hit`: grow a deterministic `AddSphere` cluster seeded by the
//! projectile id (`ctx.rng(p.id as u64)`), with a `params.duck_chance` rubber duck built from
//! spheres.
//! Then return `true` from `implemented` and add scenario tests for the bonus, malus and
//! lasting effect.

use crate::weapons::WeaponBehavior;

/// Behaviour of the Seed Grenade.
#[derive(Debug)]
pub struct SeedGrenade;

impl WeaponBehavior for SeedGrenade {
    fn id(&self) -> &'static str {
        "seed_grenade"
    }
}
