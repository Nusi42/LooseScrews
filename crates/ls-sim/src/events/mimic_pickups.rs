//! Mimic pickups (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! `3 + players / 4` (at most 6) weapon or upgrade pickups become mimics
//! for 20 s. The announce phase picks them (seeded; their spawn indices in
//! the replicated `params`, 255 = unused, so clients can make them tremble
//! and hide the pickup while its mimic is out). When the active phase
//! starts each picked pickup still holding a weapon or upgrade grows a
//! mimic: it hops toward the nearest ship within 30 u that it can see at
//! 12 u/s and bites (12 damage with knockback, 1 s apart per mimic; no
//! kill credit: the killer is "the level"). It has 25 hp; a killed mimic
//! drops its content where it died (the pickup moves there and can be
//! taken again). Mimicked pickups cannot be taken while their mimic is out
//! (`allows_pickup`). During the 2 s ending phase the survivors hop home,
//! then they turn back into their pickups.
//!
//! Mimic entity (`ReplicatedKind::Mimic`): `visual` = the pickup's visual,
//! `state` = `[weapon id, ammo (weapons), flags, hp percent]`, flags bit 0
//! upgrade (else weapon), bit 1 hunting a ship, bit 2 biting (for 0.3 s
//! after a bite), bit 3 going home. Outcome cues (`CueKind::EventOutcome`):
//! 2 a bite (target = the bitten ship), `OUTCOME_SHOT_DOWN` a mimic killed
//! (source = the shooter).
//!
//! Numbers: `data/events/mimic_pickups.ron`.

use ls_math::{Tick, Vec3};

use crate::behavior::{Command, Cue, CueKind, DamageSource};
use crate::events::drone::{self, Flight, OUTCOME_SHOT_DOWN};
use crate::events::{EffectCtx, EventCtx, EventKind, EventModule, EventPhase, EventRef};
use crate::PickupKind;

/// Mimic pickups.
#[derive(Debug)]
pub struct MimicPickups;

/// Outcome byte: a mimic bit a ship.
pub const OUTCOME_BITE: u8 = 2;
/// `params` value of an unused mimic slot.
pub const NO_SPAWN: u8 = 255;
/// Mimic `state[2]` flags.
pub const FLAG_UPGRADE: u8 = 1;
pub const FLAG_HUNTING: u8 = 2;
pub const FLAG_BITING: u8 = 4;
pub const FLAG_HOME: u8 = 8;

/// Mimic `vars`: spawn index, tick of the next allowed bite, tick the last
/// bite ends (flag), the prey (player id + 1, 0 none).
const M_SPAWN: usize = 0;
const M_NEXT_BITE: usize = 1;
const M_BITING_UNTIL: usize = 2;
const M_PREY: usize = 3;
/// Ticks between two looks for prey (each mimic on its own phase).
pub const RETARGET_TICKS: u64 = 6;

fn flight(e: &EventCtx) -> Flight {
    Flight {
        speed: e.def.param("speed", 12.0),
        accel: e.def.param("accel", 80.0),
        radius: e.def.param("radius", 0.9),
    }
}

/// The mimic look of a pickup content: `(visual state bytes)`, `None` for
/// contents that do not turn (gadgets, scrap).
fn mimic_state(kind: PickupKind) -> Option<[u8; 3]> {
    match kind {
        PickupKind::Weapon { weapon, ammo } => Some([weapon.0, ammo.min(255) as u8, 0]),
        PickupKind::Upgrade { weapon } => Some([weapon.0, 0, FLAG_UPGRADE]),
        _ => None,
    }
}

/// How many pickups turn with `players` ships in the match.
pub fn mimic_count(players: usize, max: usize) -> usize {
    (3 + players / 4).min(max)
}

impl EventModule for MimicPickups {
    fn kind(&self) -> EventKind {
        EventKind::MimicPickups
    }

    fn implemented(&self) -> bool {
        true
    }

    /// Needs at least `min_pickups` (4) pickups in play, one of them a
    /// weapon or an upgrade.
    fn eligible(&self, e: &EventRef) -> bool {
        crate::events::pickups_in_play(e.view) as f32 >= e.def.param("min_pickups", 4.0)
            && e.view
                .pickups
                .iter()
                .any(|p| p.content.is_some_and(|(_, k)| mimic_state(k).is_some()))
    }

    /// Picks the pickups that turn.
    fn on_announce(&self, e: &mut EventCtx) {
        let mut candidates: Vec<u8> = e
            .ctx
            .view
            .pickups
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                *i < NO_SPAWN as usize && p.content.is_some_and(|(_, k)| mimic_state(k).is_some())
            })
            .map(|(i, _)| i as u8)
            .collect();
        let max = (e.def.param("max_mimics", 6.0) as usize).min(8);
        let n = mimic_count(e.ctx.view.ships.len(), max).min(candidates.len());
        let mut rng = e.rng(0);
        rng.shuffle(&mut candidates);
        let mut picked: Vec<u8> = candidates.into_iter().take(n).collect();
        picked.sort_unstable();
        e.event.params = [NO_SPAWN; 8];
        for (slot, idx) in picked.into_iter().enumerate() {
            e.event.params[slot] = idx;
        }
    }

    /// The picked pickups grow teeth.
    fn on_start(&self, e: &mut EventCtx) {
        let f = flight(e);
        let hp = e.def.param("hp", 25.0);
        for slot in 0..8 {
            let idx = e.event.params[slot];
            if idx == NO_SPAWN {
                continue;
            }
            let Some(sp) = e.ctx.view.pickups.get(idx as usize) else {
                e.event.params[slot] = NO_SPAWN;
                continue;
            };
            let Some((kind, look)) = sp
                .content
                .and_then(|(_, k)| mimic_state(k).map(|look| (k, look)))
            else {
                // Taken during the announcement.
                e.event.params[slot] = NO_SPAWN;
                continue;
            };
            let visual = match kind {
                PickupKind::Weapon { weapon, .. } => e.ctx.view.visuals.weapon_pickup(weapon),
                _ => crate::visual::BuiltinVisual::UpgradePickup.id(),
            };
            let mut spawn = drone::prop(
                drone::MIMIC,
                sp.content_pos(),
                f.radius,
                hp,
                [look[0], look[1], look[2], 100],
            );
            spawn.visual = visual;
            spawn.vars = [idx as f32, 0.0, 0.0, 0.0];
            e.ctx.spawn_entity(spawn);
        }
    }

    fn on_tick(&self, e: &mut EventCtx) {
        let view = e.ctx.view;
        let f = flight(e);
        let now = e.now();
        let hp_max = e.def.param("hp", 25.0);
        let range = e.def.param("range", 30.0);
        let ending = e.event.phase == EventPhase::Ending;
        for m in drone::props(&view, drone::MIMIC) {
            let mut m = m.clone();
            let idx = m.vars[M_SPAWN] as usize;
            let home = view.pickups.get(idx).map_or(m.pos, |sp| sp.content_pos());
            if m.hp <= 0.0 {
                // Killed: the content drops where the mimic died.
                if let Some(sp) = view.pickups.get(idx).filter(|sp| sp.content.is_some()) {
                    e.ctx.move_pickup(sp.id, m.pos - sp.content_pos());
                }
                for p in e.event.params.iter_mut() {
                    if *p as usize == idx {
                        *p = NO_SPAWN;
                    }
                }
                e.ctx.cue(Cue {
                    kind: CueKind::EventOutcome,
                    source: m.last_hit.map(|(_, by)| by),
                    target: None,
                    pos: m.pos,
                    value: drone::outcome(EventKind::MimicPickups, OUTCOME_SHOT_DOWN),
                });
                e.ctx.push(Command::RemoveEntity { id: m.id });
                continue;
            }
            // The nearest ship in sight and range (looked for every 0.1 s,
            // the line-of-sight tests are the costly part), unless it is
            // time to go home.
            let prey = if ending {
                None
            } else if (now + m.id as u64) % RETARGET_TICKS == 0 {
                nearest_visible(&view, m.pos, range)
            } else {
                let id = m.vars[M_PREY] as u32;
                (id > 0)
                    .then(|| crate::PlayerId((id - 1) as u16))
                    .and_then(|p| view.ships.get(&p))
                    .filter(|s| s.alive && s.pos.distance(m.pos) <= range)
            };
            m.vars[M_PREY] = prey.map_or(0.0, |s| (s.player.0 as u32 + 1) as f32);
            let mut flags = m.state[2] & FLAG_UPGRADE;
            let goal = match prey {
                Some(s) => {
                    flags |= FLAG_HUNTING;
                    let reach =
                        e.ctx.tuning_of(s).radius + f.radius + e.def.param("bite_margin", 0.3);
                    let d = s.pos - m.pos;
                    if d.length() <= reach && now as f32 >= m.vars[M_NEXT_BITE] {
                        let dir = d.normalize_or_zero();
                        e.ctx.damage(
                            s.player,
                            e.def.param("bite_damage", 12.0),
                            dir * e.def.param("bite_knockback", 8.0),
                            DamageSource::ENVIRONMENT,
                            true,
                        );
                        let gap = Tick::ticks_for_secs(e.def.param("bite_interval_secs", 1.0));
                        m.vars[M_NEXT_BITE] = (now + gap) as f32;
                        m.vars[M_BITING_UNTIL] = (now + Tick::ticks_for_secs(0.3)) as f32;
                        e.ctx.cue(Cue {
                            kind: CueKind::EventOutcome,
                            source: None,
                            target: Some(s.player),
                            pos: m.pos,
                            value: drone::outcome(EventKind::MimicPickups, OUTCOME_BITE),
                        });
                    }
                    // Snap at the hull, not inside it.
                    s.pos - d.normalize_or_zero() * (reach * 0.8)
                }
                None => {
                    flags |= FLAG_HOME;
                    home
                }
            };
            if (now as f32) < m.vars[M_BITING_UNTIL] {
                flags |= FLAG_BITING;
            }
            let (p, v) = drone::fly(view.terrain, m.pos, m.vel, goal, &f);
            m.pos = p;
            m.vel = v;
            let flat = Vec3::new(v.x, 0.0, v.z);
            if flat.length_squared() > 0.01 {
                m.rot = drone::facing(flat);
            }
            m.state = [
                m.state[0],
                m.state[1],
                flags,
                drone::hp_percent(m.hp, hp_max),
            ];
            e.ctx.pose_entity(&m);
        }
    }

    fn on_end(&self, e: &mut EventCtx) {
        drone::remove_props(e.ctx, drone::MIMIC);
    }

    /// Mimicked pickups cannot be taken while their mimic is out.
    fn allows_pickup(&self, fx: &EffectCtx, spawn: u16) -> bool {
        spawn >= NO_SPAWN as u16 || !fx.view.params.contains(&(spawn as u8))
    }
}

/// The living ship nearest `pos` within `range` with a clear line of sight
/// (ties go to the lower player id).
fn nearest_visible<'v>(
    view: &crate::behavior::View<'v>,
    pos: Vec3,
    range: f32,
) -> Option<&'v crate::ShipState> {
    let mut best: Option<(&crate::ShipState, f32)> = None;
    for s in view.ships.values() {
        if !s.alive {
            continue;
        }
        let d = s.pos.distance(pos);
        if d > range || best.is_some_and(|(_, bd)| d >= bd) {
            continue;
        }
        let to = s.pos - pos;
        if d > 1e-3 && view.terrain.raycast(pos, to / d, d).is_some() {
            continue;
        }
        best = Some((s, d));
    }
    best.map(|(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn mimic_count_follows_the_plan() {
        assert_eq!(mimic_count(1, 6), 3);
        assert_eq!(mimic_count(4, 6), 4);
        assert_eq!(mimic_count(8, 6), 5);
        assert_eq!(mimic_count(12, 6), 6);
        assert_eq!(mimic_count(16, 6), 6);
    }
}
