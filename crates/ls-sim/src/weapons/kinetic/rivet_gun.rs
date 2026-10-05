//! Rivet Gun: kinetic primary. Fires scrap, 6x laser speed, high rate of fire, light damage.
//! Design doc: "180 u/s, high rate of fire, light damage. Malus: burns through scrap fast.
//! Lasting effect: rivets embed in walls; knocks loose debris from weak terrain."
//!
//! Numbers: `data/weapons/rivet_gun.ron` (`params`: `weak_debris_chance`, `chip_radius`,
//! `debris_radius`, `debris_speed`, `debris_scrap`, `debris_hp`, `debris_crowd`).
//!
//! Behaviour: the generic fast straight rivet that costs scrap per shot (the M1
//! behaviour), plus the lasting effect on terrain: every rivet that hits a wall reports an
//! `Impact` at the wall (clients draw the embedded rivet as a decal, design doc "scorch marks
//! and decals": not networked) and digs no crater in ordinary or hard rock. In weak rock
//! (`MATERIAL_WEAK`) a rivet knocks a chunk loose with probability `weak_debris_chance`
//! (per-rivet RNG keyed by the projectile id): a small chip crater and a debris piece that
//! flies off the wall and can be scavenged. No chunk comes loose while `debris_crowd` pieces
//! already float within 2 u (`weapons::debris_crowded`), so a burst does not fill the debris
//! cap.

use ls_math::terrain::{RayHit, MATERIAL_WEAK};
use ls_math::Vec3;

use crate::behavior::Ctx;
use crate::entity::EntitySpawn;
use crate::weapons::{debris_crowded, HitResponse, WeaponBehavior};
use crate::world::Projectile;

/// Behaviour of the Rivet Gun.
#[derive(Debug)]
pub struct RivetGun;

impl WeaponBehavior for RivetGun {
    fn id(&self) -> &'static str {
        "rivet_gun"
    }

    fn implemented(&self) -> bool {
        true
    }

    fn on_terrain_hit(&self, ctx: &mut Ctx, p: &mut Projectile, hit: &RayHit) -> HitResponse {
        // The rivet embeds: `Impact` at the wall (the data has no crater).
        ctx.detonate(p, hit.point, None, true);
        if ctx.material_at(hit.point) == MATERIAL_WEAK {
            let def = ctx.def(p.weapon);
            let mut rng = ctx.rng(u64::from(p.id));
            let crowd = def.param("debris_crowd", 2.0) as usize;
            if rng.chance(def.param("weak_debris_chance", 0.2))
                && !debris_crowded(ctx, hit.point, crowd)
            {
                let radius = def.param("debris_radius", 0.3);
                let speed = def.param("debris_speed", 4.0);
                let jitter = Vec3::new(
                    rng.range_f32(-0.5, 0.5),
                    rng.range_f32(-0.5, 0.5),
                    rng.range_f32(-0.5, 0.5),
                );
                let dir = (hit.normal + jitter).try_normalize().unwrap_or(hit.normal);
                ctx.crater(hit.point, def.param("chip_radius", 0.35));
                let mut chunk = EntitySpawn::debris(
                    hit.point + hit.normal * (radius + 0.1),
                    dir * speed,
                    radius,
                    def.param("debris_scrap", 0.5),
                );
                // Tough enough to survive the next few rivets of the burst.
                chunk.hp = chunk.hp.max(def.param("debris_hp", 10.0));
                ctx.spawn_entity(chunk);
            }
        }
        HitResponse::Stop
    }
}

/// Shared scenario-test helpers for the kinetic and ordnance I modules.
#[cfg(test)]
pub(crate) mod testkit {
    use ls_math::terrain::{BoxRoomTerrain, TerrainEdit, TerrainQuery};
    use ls_math::{Aabb, Quat, Vec3};

    use crate::*;

    /// Two teams, spawns at x = -20 / +20, Paint the Void rules.
    pub fn world() -> World {
        let setup = MatchSetup {
            wells: Vec::new(),
            belt: None,
            spawns: vec![
                SpawnPoint {
                    pos: Vec3::new(-20.0, 0.0, 0.0),
                    team: 0,
                    orientation: Quat::IDENTITY,
                },
                SpawnPoint {
                    pos: Vec3::new(20.0, 0.0, 0.0),
                    team: 1,
                    orientation: Quat::IDENTITY,
                },
            ],
            pickup_points: vec![],
            instability: 0,
            heist: None,
            ..Default::default()
        };
        World::new(WorldConfig::paint_the_void(7, setup)).unwrap()
    }

    /// A box room with a fixed rock material everywhere.
    pub struct Room {
        pub room: BoxRoomTerrain,
        pub material: u8,
    }

    impl TerrainQuery for Room {
        fn sdf(&self, p: Vec3) -> f32 {
            self.room.sdf(p)
        }
        fn bounds(&self) -> Aabb {
            self.room.bounds()
        }
    }

    impl SimTerrain for Room {
        fn apply_edit(&mut self, edit: &TerrainEdit) {
            self.room.apply(*edit);
        }
        fn material_at(&self, _p: Vec3) -> u8 {
            self.material
        }
    }

    /// 60 x 30 x 60 u room of ordinary rock (walls at x, z = +-30, y = +-15).
    pub fn room() -> Room {
        room_sized(Vec3::new(60.0, 30.0, 60.0))
    }

    pub fn room_sized(size: Vec3) -> Room {
        Room {
            room: BoxRoomTerrain::centered(size),
            material: ls_math::terrain::MATERIAL_ROCK,
        }
    }

    /// Adds a slab of rock across the room: spheres of `radius` every 1 u
    /// in x and y (|x|, |y| <= 4) at depth `z` (about `2 * radius` thick).
    pub fn slab(t: &mut Room, z: f32, radius: f32) {
        for i in -4..=4 {
            for j in -4..=4 {
                t.apply_edit(&TerrainEdit::AddSphere {
                    center: Vec3::new(i as f32, j as f32, z),
                    radius,
                });
            }
        }
    }

    pub fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
        let s = w.ship_mut(id).unwrap();
        s.pos = pos;
        s.vel = Vec3::ZERO;
        s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
    }

    /// Two Tinkers, `a` (team 0) and `b` (team 1).
    pub fn duel(w: &mut World) -> (PlayerId, PlayerId) {
        let a = w.add_player(Some(0), ShipType::Tinker).unwrap();
        let b = w.add_player(Some(1), ShipType::Tinker).unwrap();
        (a, b)
    }

    /// Gives weapon `id` to `p` directly (full ammo, 100 scrap) and selects it.
    pub fn equip(w: &mut World, p: PlayerId, id: &str) -> WeaponId {
        let wid = w.weapons().find(id).unwrap();
        let def = w.weapons().get(wid).clone();
        let s = w.ship_mut(p).unwrap();
        s.add_weapon(WeaponState {
            weapon: wid,
            ammo: def.max_ammo,
            level: 1,
            shots: 0,
        });
        s.scrap = 100.0;
        match def.slot {
            WeaponSlot::Primary => s.primary = Some(wid),
            WeaponSlot::Secondary => s.secondary = Some(wid),
        }
        wid
    }

    pub fn run<T: SimTerrain>(
        w: &mut World,
        t: &mut T,
        inputs: &[(PlayerId, InputCmd)],
        n: u32,
    ) -> Vec<Event> {
        let mut ev = Vec::new();
        for _ in 0..n {
            ev.extend(w.step(t, inputs));
        }
        ev
    }

    pub fn fire1(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
        [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_PRIMARY))]
    }

    pub fn fire2(id: PlayerId) -> [(PlayerId, InputCmd); 1] {
        [(id, InputCmd::idle(0).with_buttons(Buttons::FIRE_SECONDARY))]
    }

    pub fn damage_to(ev: &[Event], who: PlayerId) -> f32 {
        hits_on(ev, who).iter().sum()
    }

    /// Every damage amount `who` took, in order.
    pub fn hits_on(ev: &[Event], who: PlayerId) -> Vec<f32> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Damaged { target, amount, .. } if *target == who => Some(*amount),
                _ => None,
            })
            .collect()
    }

    /// Centres and radii of the craters dug.
    pub fn craters(ev: &[Event]) -> Vec<(Vec3, f32)> {
        ev.iter()
            .filter_map(|e| match e {
                Event::TerrainEdit(TerrainEdit::SubtractSphere { center, radius }) => {
                    Some((*center, *radius))
                }
                _ => None,
            })
            .collect()
    }

    pub fn fired(ev: &[Event]) -> usize {
        ev.iter()
            .filter(|e| matches!(e, Event::Fired { .. }))
            .count()
    }

    /// Impact points of `weapon` without a ship target (walls, entities, air).
    pub fn impacts(ev: &[Event], weapon: WeaponId) -> Vec<Vec3> {
        ev.iter()
            .filter_map(|e| match e {
                Event::Impact {
                    weapon: w,
                    point,
                    target: None,
                    ..
                } if *w == weapon => Some(*point),
                _ => None,
            })
            .collect()
    }

    pub fn debris(w: &World) -> Vec<&Entity> {
        w.entities()
            .iter()
            .filter(|e| e.kind == EntityKind::Debris)
            .collect()
    }

    /// A stationary enemy decoy entity (team 1, owned by `owner`).
    pub fn decoy(owner: PlayerId, pos: Vec3) -> EntitySpawn {
        EntitySpawn {
            kind: EntityKind::Decoy,
            visual: BuiltinVisual::Decoy.id(),
            owner: Some(owner),
            team: 1,
            radius: 0.5,
            hp: 100.0,
            lifetime_ticks: Some(600),
            ..EntitySpawn::marker(pos)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use crate::*;
    use ls_math::terrain::MATERIAL_WEAK;
    use ls_math::Vec3;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// Bonus: 180 u/s, high rate of fire, light damage per rivet.
    #[test]
    fn fast_rapid_light_rivets() {
        let mut w = world();
        let mut t = room();
        let (a, b) = duel(&mut w);
        equip(&mut w, a, "rivet_gun");
        place(&mut w, a, Vec3::new(0.0, 0.0, 20.0), Vec3::NEG_Z);
        place(&mut w, b, Vec3::ZERO, Vec3::Z);
        let mut ev = Vec::new();
        for _ in 0..60 {
            ev.extend(run(&mut w, &mut t, &fire1(a), 1));
            place(&mut w, b, Vec3::ZERO, Vec3::Z);
        }
        assert!(fired(&ev) >= 10, "high rate: {} in 1 s", fired(&ev));
        let hits = hits_on(&ev, b);
        assert!(hits.len() >= 9, "{hits:?}");
        assert!(hits.iter().all(|&d| d == 3.0), "light damage {hits:?}");
        let p = &w.projectiles()[0];
        assert_eq!(p.vel.length(), 180.0);
    }

    /// Malus: every rivet costs half a scrap, so a second of fire eats 5 scrap.
    #[test]
    fn burns_through_scrap() {
        let mut w = world();
        let mut t = room();
        let (a, _) = duel(&mut w);
        equip(&mut w, a, "rivet_gun");
        place(&mut w, a, Vec3::ZERO, Vec3::NEG_Z);
        w.ship_mut(a).unwrap().scrap = 20.0;
        let ev = run(&mut w, &mut t, &fire1(a), 60);
        let used = 20.0 - w.ship(a).unwrap().scrap;
        assert_eq!(used, fired(&ev) as f32 * 0.5);
        assert!(used >= 5.0, "{used}");
        // Out of scrap: no more rivets.
        w.ship_mut(a).unwrap().scrap = 0.0;
        assert_eq!(fired(&run(&mut w, &mut t, &fire1(a), 30)), 0);
    }

    /// Lasting effect: rivets embed in ordinary rock (an impact at the wall,
    /// no crater, no debris) ...
    #[test]
    fn rivets_embed_in_ordinary_rock() {
        let mut w = world();
        let mut t = room();
        let (a, _) = duel(&mut w);
        let rivet = equip(&mut w, a, "rivet_gun");
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        let mut ev = run(&mut w, &mut t, &fire1(a), 60);
        ev.extend(run(&mut w, &mut t, &[], 10));
        let hits = impacts(&ev, rivet);
        assert!(hits.len() >= 10, "{}", hits.len());
        assert!(hits.iter().all(|p| (p.z + 30.0).abs() < 0.2), "{hits:?}");
        assert!(craters(&ev).is_empty());
        assert!(debris(&w).is_empty());
    }

    /// ... and knock loose debris (with a chip crater) from weak rock.
    #[test]
    fn rivets_knock_debris_off_weak_rock() {
        let mut w = world();
        let mut t = room();
        t.material = MATERIAL_WEAK;
        let (a, _) = duel(&mut w);
        let rivet = equip(&mut w, a, "rivet_gun");
        place(&mut w, a, Vec3::new(0.0, 0.0, -20.0), Vec3::NEG_Z);
        // Rivets that follow may shoot the loose chunks apart: watch every tick.
        let mut ev = Vec::new();
        let mut seen = Vec::new();
        for _ in 0..180 {
            w.ship_mut(a).unwrap().scrap = 100.0;
            ev.extend(run(&mut w, &mut t, &fire1(a), 1));
            seen.extend(debris(&w).into_iter().cloned());
        }
        let rivets = impacts(&ev, rivet).len();
        let chips = craters(&ev);
        let chance = w.weapons().get(rivet).param("weak_debris_chance", 0.0);
        assert!(
            !chips.is_empty(),
            "no chunk knocked loose in {rivets} rivets"
        );
        assert!(
            (chips.len() as f32) < rivets as f32 * chance * 2.0,
            "{} chips of {rivets}",
            chips.len()
        );
        // Chip craters scaled by weak rock (1.4x).
        assert!(chips.iter().all(|(_, r)| (*r - 0.35 * 1.4).abs() < 1e-4));
        assert!(!seen.is_empty(), "debris knocked loose");
        assert!(seen.iter().all(|e| e.scrap > 0.0 && e.pos.z > -30.0));
        assert!(seen.iter().any(|e| e.vel.z > 0.0), "flies off the wall");
    }
}
