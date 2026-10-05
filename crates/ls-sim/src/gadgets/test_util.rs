//! Shared helpers for the gadget scenario tests (test builds only).

use ls_math::terrain::BoxRoomTerrain;
use ls_math::{Quat, Vec3};

use crate::*;

pub fn setup() -> MatchSetup {
    MatchSetup {
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
    }
}

/// A Paint the Void world whose loadout gadget is `gadget`.
pub fn world_with_gadget(gadget: &str) -> World {
    let mut rules = RuleSet::paint_the_void();
    rules.starting_loadout.gadget = Some(gadget.into());
    rules
        .gadgets
        .insert(gadget.into(), Availability::LoadoutAndPickup);
    World::new(WorldConfig {
        rules,
        ..WorldConfig::paint_the_void(11, setup())
    })
    .unwrap()
}

pub fn room() -> BoxRoomTerrain {
    BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0))
}

pub fn place(w: &mut World, id: PlayerId, pos: Vec3, dir: Vec3) {
    let s = w.ship_mut(id).unwrap();
    s.pos = pos;
    s.vel = Vec3::ZERO;
    s.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, dir.normalize());
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

pub fn press(id: PlayerId, b: Buttons) -> [(PlayerId, InputCmd); 1] {
    [(id, InputCmd::idle(0).with_buttons(b))]
}

pub fn damage_to(ev: &[Event], who: PlayerId) -> f32 {
    ev.iter()
        .filter_map(|e| match e {
            Event::Damaged { target, amount, .. } if *target == who => Some(*amount),
            _ => None,
        })
        .sum()
}
