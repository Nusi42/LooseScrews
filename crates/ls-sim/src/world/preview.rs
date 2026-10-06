//! Preview of one ship's movement in the coming step (the server's lag
//! compensation, `ls_game::server::lagcomp`): exactly what stage 2 of
//! [`World::step`] does to a ship's pose and velocity, including the mode's
//! movement limits, the running event's movement effects, the gravity
//! wells (static and dynamic) and the level's up.

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
        let pos_before = s.pos;
        if !fx.is_none() {
            crate::director::apply_move_effects_before(&mut s, &fx);
        }
        let plain = self.wells.is_empty() && self.up == Vec3::Y;
        if limits.is_none() {
            if plain {
                simulate_movement(&mut s, &move_cmd, tuning, terrain);
            } else {
                let env = self.wells.move_env(s.pos, now, tuning, self.up);
                simulate_movement_env(&mut s, &move_cmd, tuning, terrain, &env);
            }
        } else {
            let (limited, _) = limits.apply(tuning, &move_cmd);
            if plain {
                simulate_movement(&mut s, &move_cmd, &limited, terrain);
            } else {
                let env = self.wells.move_env(s.pos, now, &limited, self.up);
                simulate_movement_env(&mut s, &move_cmd, &limited, terrain, &env);
            }
        }
        if !fx.is_none() {
            crate::director::apply_move_effects_after(&mut s, &fx, pos_before);
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
}
