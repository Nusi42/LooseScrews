//! Preview of one ship's movement in the coming step (the server's lag
//! compensation, `ls_game::server::lagcomp`): exactly what stage 2 of
//! [`World::step`] does to a ship's pose and velocity, including the mode's
//! movement limits, the running event's movement effects, the gravity
//! wells (static and dynamic, dimmed by Gravity coffee break), the level's
//! up (turned by Bored level on the coming tick), the running event's
//! kinematic obstacles (Duck parade) and a belt's drifting boulders.

use ls_math::terrain::TerrainQuery;
use ls_math::Vec3;

use super::{proj_flags, World};
use crate::events::MoveEffects;
use crate::ship::{simulate_movement, simulate_movement_env, ShipState};
use crate::InputCmd;

impl World {
    /// `ship` (a copy, possibly displaced) after the movement the next
    /// [`World::step`] gives it with input `cmd`, bit for bit as that step
    /// moves it at the same position. Dead ships do not move. Nothing else
    /// in the world changes.
    pub fn preview_movement<T: TerrainQuery + ?Sized>(
        &self,
        ship: &ShipState,
        cmd: &InputCmd,
        terrain: &T,
    ) -> ShipState {
        let mut s = ship.clone();
        if !s.alive {
            return s;
        }
        let cfg = &*self.config;
        let now = self.tick + 1;
        let cmd = cmd.sanitized();
        let tuning = cfg.tuning(s.ship_type);
        let flags = self.mode.player(s.player).map_or(0, |p| p.flags);
        let limits = cfg.mode_module.move_limits(&cfg.rules.mode, flags);
        let cmd = if limits.is_none() {
            cmd
        } else {
            limits.apply(tuning, &cmd).1
        };
        let controlled = self
            .projectiles
            .iter()
            .any(|p| p.flags & proj_flags::CONTROLS_OWNER != 0 && p.owner == s.player);
        let move_cmd = if controlled {
            InputCmd {
                thrust: Vec3::ZERO,
                look_pitch: 0.0,
                look_yaw: 0.0,
                roll: 0.0,
                ..cmd
            }
        } else {
            cmd
        };
        let fx = if self.director.current.is_some() {
            crate::director::move_effects(&self.director.view(), cfg.seed, now, &s)
        } else {
            MoveEffects::NONE
        };
        // The coming step's event effects (well scale, the up it sets
        // before the ships move), as `World::step` computes them.
        let effects = self.next_event_effects();
        let up = effects.up.unwrap_or(self.up);
        let pos_before = s.pos;
        if !fx.is_none() {
            crate::director::apply_move_effects_before(&mut s, &fx);
        }
        let plain = self.wells.is_empty() && up == Vec3::Y;
        let scale = effects.well_scale;
        if limits.is_none() {
            if plain {
                simulate_movement(&mut s, &move_cmd, tuning, terrain);
            } else {
                let env = self.wells.move_env_scaled(s.pos, now, tuning, up, scale);
                simulate_movement_env(&mut s, &move_cmd, tuning, terrain, &env);
            }
        } else {
            let (limited, _) = limits.apply(tuning, &move_cmd);
            if plain {
                simulate_movement(&mut s, &move_cmd, &limited, terrain);
            } else {
                let env = self.wells.move_env_scaled(s.pos, now, &limited, up, scale);
                simulate_movement_env(&mut s, &move_cmd, &limited, terrain, &env);
            }
        }
        if !fx.is_none() {
            crate::director::apply_move_effects_after(&mut s, &fx, pos_before);
        }
        // The running event's kinematic obstacles at the coming tick.
        if self.director.current.is_some() {
            let mut obstacles = Vec::new();
            let view = self.director.view();
            crate::director::obstacles(&view, cfg.seed, now, &cfg.setup, &mut obstacles);
            if !obstacles.is_empty() {
                let mut contacts = Vec::new();
                crate::obstacle::push_sphere(
                    terrain,
                    &mut s.pos,
                    &mut s.vel,
                    tuning.radius,
                    &obstacles,
                    &mut contacts,
                );
            }
        }
        // Drifting boulders (belt levels).
        if let Some(belt) = cfg.setup.belt.as_ref().filter(|b| !b.boulders.is_empty()) {
            crate::obstacle::push_boulders(
                belt,
                crate::obstacle::BoulderSettings::builtin(),
                now,
                &mut s.pos,
                &mut s.vel,
                tuning.radius,
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use ls_math::terrain::BoxRoomTerrain;
    use ls_math::{Pcg32, Quat, Vec3};

    use crate::gravity::{DynamicWell, WellSite};
    use crate::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn spawns() -> Vec<SpawnPoint> {
        (0..4)
            .map(|i| SpawnPoint {
                pos: Vec3::new(-15.0 + 10.0 * i as f32, 0.0, 0.0),
                team: (i % 2) as u8,
                orientation: Quat::IDENTITY,
            })
            .collect()
    }

    /// The preview equals the real step for every ship, every tick, in a
    /// level with static and dynamic wells (one starting and ending inside
    /// the run) and a turned up, with random inputs.
    #[test]
    fn preview_matches_the_step_in_wells_and_under_a_turned_up() {
        let setup = MatchSetup {
            spawns: spawns(),
            gravity_chaos: 100,
            wells: vec![WellSite {
                pos: Vec3::new(0.0, 6.0, 4.0),
                core_radius: 2.0,
                hollow_radius: 8.0,
                strength: 1.0,
                range: 30.0,
                kind: ls_math::level::WellKind::Core,
            }],
            ..Default::default()
        };
        let mut w = World::new(WorldConfig::paint_the_void(11, setup)).unwrap();
        let ids: Vec<PlayerId> = ShipType::ALL
            .iter()
            .map(|&t| w.add_player(None, t).unwrap())
            .collect();
        let mut t = BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0));
        w.add_well(DynamicWell {
            key: 9,
            pos: Vec3::new(-8.0, -4.0, 0.0),
            peak_accel: 9.0,
            range: 16.0,
            start_tick: 40,
            end_tick: 160,
            owner: None,
        });
        let mut rng = Pcg32::new(5, 1);
        let mut moved = 0;
        for tick in 1..=240u64 {
            if tick == 120 {
                w.set_up(Vec3::new(0.3, 0.9, 0.1));
            }
            let inputs: Vec<(PlayerId, InputCmd)> = ids
                .iter()
                .map(|&id| {
                    let mut c = InputCmd::idle(tick);
                    if rng.chance(0.7) {
                        let axis = |r: &mut Pcg32| r.below(3) as f32 - 1.0;
                        c.thrust = Vec3::new(axis(&mut rng), axis(&mut rng), axis(&mut rng));
                    }
                    c.look_yaw = rng.range_f32(-0.05, 0.05);
                    c.buttons.set(Buttons::BOOST, rng.chance(0.2));
                    c.buttons.set(Buttons::HORIZON_ASSIST, true);
                    (id, c)
                })
                .collect();
            let preview: Vec<ShipState> = inputs
                .iter()
                .map(|(id, c)| w.preview_movement(w.ship(*id).unwrap(), c, &t))
                .collect();
            w.step(&mut t, &inputs);
            for p in &preview {
                let s = w.ship(p.player).unwrap();
                if !s.alive {
                    continue;
                }
                assert_eq!(p.pos, s.pos, "tick {tick} {:?}", p.player);
                assert_eq!(p.vel, s.vel, "tick {tick} {:?}", p.player);
                assert_eq!(p.orientation, s.orientation, "tick {tick}");
                moved += 1;
            }
        }
        assert!(moved > 800, "{moved}");
        // The wells really act: a plain flight model would differ.
        let s = w.ship(ids[0]).unwrap().clone();
        let mut plain = s.clone();
        simulate_movement(&mut plain, &InputCmd::idle(0), w.tuning(), &t);
        let preview = w.preview_movement(&s, &InputCmd::idle(0), &t);
        assert_ne!(plain.pos, preview.pos);
    }

    /// Steps `w` for `ticks` with random inputs and checks the preview of
    /// every living ship against the step; `probe` sees the world before
    /// each step. Returns the number of compared ship moves.
    fn compare<T: crate::terrain::SimTerrain>(
        w: &mut World,
        t: &mut T,
        ids: &[PlayerId],
        ticks: u64,
        mut probe: impl FnMut(&mut World),
    ) -> usize {
        let mut rng = Pcg32::new(8, 3);
        let mut moved = 0;
        for _ in 0..ticks {
            probe(w);
            let tick = w.tick() + 1;
            let inputs: Vec<(PlayerId, InputCmd)> = ids
                .iter()
                .map(|&id| {
                    let mut c = InputCmd::idle(tick);
                    let axis = |r: &mut Pcg32| r.below(3) as f32 - 1.0;
                    c.thrust = Vec3::new(axis(&mut rng), axis(&mut rng), axis(&mut rng));
                    c.look_yaw = rng.range_f32(-0.05, 0.05);
                    c.buttons.set(Buttons::HORIZON_ASSIST, true);
                    (id, c)
                })
                .collect();
            let preview: Vec<ShipState> = inputs
                .iter()
                .map(|(id, c)| w.preview_movement(w.ship(*id).unwrap(), c, &*t))
                .collect();
            w.step(t, &inputs);
            for p in &preview {
                let s = w.ship(p.player).unwrap();
                if !s.alive || !p.alive {
                    continue;
                }
                assert_eq!(p.pos, s.pos, "tick {} {:?}", w.tick(), p.player);
                assert_eq!(p.vel, s.vel, "tick {} {:?}", w.tick(), p.player);
                assert_eq!(p.orientation, s.orientation, "tick {}", w.tick());
                moved += 1;
            }
        }
        moved
    }

    /// D1 review: the preview (lag compensation) moves a ship like the step
    /// while Gravity coffee break dims the wells, Bored level turns the up
    /// and Duck parade's ducks push ships aside.
    #[test]
    fn preview_matches_the_step_during_movement_events() {
        let setup = MatchSetup {
            spawns: spawns(),
            gravity_chaos: 100,
            wells: vec![WellSite {
                pos: Vec3::new(0.0, 6.0, 4.0),
                core_radius: 2.0,
                hollow_radius: 8.0,
                strength: 1.0,
                range: 30.0,
                kind: ls_math::level::WellKind::Core,
            }],
            ..Default::default()
        };
        let mut w = World::new(WorldConfig::paint_the_void(12, setup)).unwrap();
        let ids: Vec<PlayerId> = (0..4)
            .map(|_| w.add_player(None, ShipType::Tinker).unwrap())
            .collect();
        let mut t = BoxRoomTerrain::centered(Vec3::new(60.0, 30.0, 60.0));
        w.schedule_event(1, EventKind::GravityCoffeeBreak);
        w.schedule_event(2, EventKind::BoredLevel);
        w.schedule_event(3, EventKind::DuckParade);
        let (mut dimmed, mut turning, mut ducks) = (0, 0, 0);
        let moved = compare(&mut w, &mut t, &ids, 3000, |w| {
            let fx = w.next_event_effects();
            if fx.well_scale < 1.0 {
                dimmed += 1;
            }
            if fx.up.is_some_and(|u| u != w.up()) {
                turning += 1;
            }
            let parade = w.director().current.as_ref().is_some_and(|e| {
                e.kind == EventKind::DuckParade && e.phase != crate::events::EventPhase::Announce
            });
            if parade {
                ducks += 1;
                // Fly two ships into the parade's path (the spawn line).
                for id in [ids[0], ids[1]] {
                    if let Some(s) = w.ship_mut(id).filter(|s| s.alive) {
                        s.pos = Vec3::new(s.pos.x, 0.0, 0.0);
                    }
                }
            }
        });
        assert!(
            dimmed > 600 && turning > 200 && ducks > 200,
            "{dimmed} {turning} {ducks}"
        );
        assert!(moved > 8000, "{moved}");
    }

    /// D1 review: a belt's drifting boulders push the previewed ship as the
    /// step pushes it.
    #[test]
    fn preview_matches_the_step_against_boulders() {
        let (setup, mut t) = crate::modes::belt_up::replay::belt_arena();
        let belt = setup.belt.clone().unwrap();
        assert!(!belt.boulders.is_empty());
        let mut w = World::new(WorldConfig::paint_the_void(13, setup)).unwrap();
        let id = w.add_player(None, ShipType::Tinker).unwrap();
        let radius = w.tuning_for(ShipType::Tinker).radius;
        let mut touching = 0;
        let moved = compare(&mut w, &mut t, &[id], 240, |w| {
            // Just touching the first boulder where it will be, moving in.
            let now = w.tick() + 1;
            let o = belt.boulders[0];
            let c = o.pos_at(belt.center, now);
            let out = (c - belt.center).normalize_or_zero();
            if let Some(s) = w.ship_mut(id).filter(|s| s.alive) {
                s.pos = c + out * (o.size + radius - 0.05);
                s.vel = -out * 6.0;
                touching += 1;
            }
        });
        assert!(touching > 200 && moved > 200, "{touching} {moved}");
    }
}
