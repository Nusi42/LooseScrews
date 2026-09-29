//! Per-frame controls from the browser to per-tick `ls_sim::InputCmd`s, and
//! the fixed 60 Hz step accumulator.
//!
//! The browser samples input once per animation frame (60-240 Hz) while the
//! simulation runs at 60 Hz. Thrust, roll and buttons are levels and apply to
//! every tick of the frame. Mouse look is a *delta*: the frame's rotation is
//! accumulated and spread over the ticks that run, quantized to the wire
//! resolution (1/8192 rad) with the rounding residue carried to the next
//! frame, so slow mouse movement is not lost and client and server see the
//! same command.

use ls_math::Vec3;
use ls_sim::{Buttons, InputCmd};

/// Wire resolution of look input (radians per step), see `ls_net::NetInput`.
pub const LOOK_STEP: f32 = 1.0 / 8192.0;
/// Frames longer than this many ticks are clamped (tab switches, stalls):
/// the simulation does not try to catch up on lost wall-clock time.
pub const MAX_TICKS_PER_FRAME: u32 = 8;
/// Length of the flat controls array passed from JavaScript.
pub const CONTROLS_LEN: usize = 11;

/// One frame of player controls in browser terms.
///
/// Signs follow the pilot's view: `+roll_right` rolls clockwise, `+yaw_right`
/// turns right, `+pitch_up` raises the nose. [`Controls::to_cmd`] maps them to
/// the simulation's local-axis convention (forward `-Z`, right `+X`, up `+Y`,
/// rotations counter-clockwise about the local axes).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Controls {
    /// -1..1 each.
    pub forward: f32,
    pub right: f32,
    pub up: f32,
    pub roll_right: f32,
    /// Radians of rotation accumulated this frame.
    pub yaw_right: f32,
    pub pitch_up: f32,
    /// `ls_sim::Buttons` bits.
    pub buttons: u16,
    /// Weapon id + 1, 0 = keep the current selection (a direct pick; the
    /// key scheme below is resolved by `GameCore` into this).
    pub weapon_select: u8,
    /// Number key pressed this frame: 1-9, 10 for the `0` key, 0 = none.
    /// Selects the n-th carried primary in design-doc order.
    pub weapon_key: u8,
    /// Mouse wheel this frame: +1 next primary, -1 previous, 0 none.
    pub cycle_primary: i8,
    /// X pressed this frame: +1 next secondary, 0 none.
    pub cycle_secondary: i8,
}

impl Controls {
    /// Parses `[forward, right, up, rollRight, yawRight, pitchUp, buttons,
    /// weaponSelect, weaponKey, cyclePrimary, cycleSecondary]`; missing or
    /// non-finite entries count as zero.
    pub fn from_slice(v: &[f32]) -> Controls {
        let at = |i: usize| v.get(i).copied().filter(|x| x.is_finite()).unwrap_or(0.0);
        Controls {
            forward: at(0),
            right: at(1),
            up: at(2),
            roll_right: at(3),
            yaw_right: at(4),
            pitch_up: at(5),
            buttons: at(6).clamp(0.0, u16::MAX as f32) as u16,
            weapon_select: at(7).clamp(0.0, u8::MAX as f32) as u8,
            weapon_key: at(8).clamp(0.0, crate::inventory::WEAPON_KEYS as f32) as u8,
            cycle_primary: at(9).signum() as i8 * (at(9) != 0.0) as i8,
            cycle_secondary: at(10).signum() as i8 * (at(10) != 0.0) as i8,
        }
    }

    /// The key / wheel / X part of the controls.
    pub fn select_request(&self) -> crate::inventory::SelectRequest {
        crate::inventory::SelectRequest {
            key: self.weapon_key,
            cycle_primary: self.cycle_primary,
            cycle_secondary: self.cycle_secondary,
        }
    }

    /// The command for one tick with the given per-tick look (sim signs).
    fn to_cmd(self, look_yaw: f32, look_pitch: f32) -> InputCmd {
        InputCmd {
            tick: 0,
            thrust: Vec3::new(self.right, self.up, -self.forward),
            look_pitch,
            look_yaw,
            roll: -self.roll_right,
            buttons: Buttons(self.buttons),
            weapon_select: self.weapon_select,
        }
        .sanitized()
    }
}

/// Spreads accumulated look rotation over simulation ticks (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LookAccumulator {
    /// Pending rotation in simulation signs (yaw about +Y, pitch about +X).
    yaw: f32,
    pitch: f32,
}

impl LookAccumulator {
    /// Adds this frame's look and returns the command to use for each of the
    /// `ticks` ticks that run now. With `ticks == 0` the look stays pending.
    /// `max_per_tick` is the ship's turn limit per tick (larger inputs are
    /// clamped by the simulation anyway, so the excess is dropped here rather
    /// than replayed later as a spin).
    pub fn command(&mut self, controls: Controls, ticks: u32, max_per_tick: f32) -> InputCmd {
        self.yaw += -controls.yaw_right;
        self.pitch += controls.pitch_up;
        if ticks == 0 {
            return controls.to_cmd(0.0, 0.0);
        }
        let n = ticks as f32;
        let split = |pending: &mut f32| {
            let per_tick = quantize_look(*pending / n).clamp(-max_per_tick, max_per_tick);
            let per_tick = quantize_look(per_tick);
            let rest = *pending - per_tick * n;
            // Residue below one step carries; anything larger was clamped away.
            *pending = rest.clamp(-LOOK_STEP * n, LOOK_STEP * n);
            per_tick
        };
        let yaw = split(&mut self.yaw);
        let pitch = split(&mut self.pitch);
        controls.to_cmd(yaw, pitch)
    }

    /// Drops pending look (e.g. after pointer lock is lost or on respawn).
    pub fn reset(&mut self) {
        *self = LookAccumulator::default();
    }
}

/// Rounds a look angle to the wire resolution.
pub fn quantize_look(rad: f32) -> f32 {
    (rad / LOOK_STEP).round() * LOOK_STEP
}

/// Fixed 60 Hz step from variable frame times.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FixedStep {
    /// Fractional ticks not yet run.
    acc: f64,
}

impl FixedStep {
    /// Adds `dt` seconds and returns the number of whole ticks due (at most
    /// [`MAX_TICKS_PER_FRAME`]).
    pub fn advance(&mut self, dt: f64) -> u32 {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.acc += dt * ls_math::TICK_HZ as f64;
        let due = self.acc.floor();
        self.acc -= due;
        (due as u32).min(MAX_TICKS_PER_FRAME)
    }

    /// Fraction of the next tick already elapsed (0..1), for render interpolation.
    pub fn alpha(&self) -> f32 {
        self.acc.clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::BoxRoomTerrain;
    use ls_math::Quat;
    use ls_sim::{simulate_movement, PlayerId, ShipState, ShipTuning, ShipType};

    fn ship() -> (ShipState, ShipTuning, BoxRoomTerrain) {
        let tuning = ShipTuning::tinker();
        let terrain = BoxRoomTerrain::centered(Vec3::splat(200.0));
        let s = ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            &tuning,
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        (s, tuning, terrain)
    }

    fn run(c: Controls, ticks: u32) -> ShipState {
        let (mut s, tuning, terrain) = ship();
        let mut look = LookAccumulator::default();
        let max = tuning.max_turn_rate * ls_math::DT;
        for i in 0..ticks {
            // All look arrives in the first frame, then zero.
            let frame = if i == 0 {
                c
            } else {
                Controls {
                    yaw_right: 0.0,
                    pitch_up: 0.0,
                    ..c
                }
            };
            let cmd = look.command(frame, 1, max);
            simulate_movement(&mut s, &cmd, &tuning, &terrain);
        }
        s
    }

    #[test]
    fn browser_signs_map_to_the_sim_axes() {
        let fwd = run(
            Controls {
                forward: 1.0,
                ..Default::default()
            },
            30,
        );
        assert!(fwd.vel.z < -1.0, "W flies towards -Z: {:?}", fwd.vel);
        let right = run(
            Controls {
                right: 1.0,
                ..Default::default()
            },
            30,
        );
        assert!(right.vel.x > 1.0);
        let up = run(
            Controls {
                up: 1.0,
                ..Default::default()
            },
            30,
        );
        assert!(up.vel.y > 1.0);

        let yaw = run(
            Controls {
                yaw_right: 0.1,
                ..Default::default()
            },
            1,
        );
        assert!(yaw.orientation.forward().x > 0.05, "yaw right turns right");
        let pitch = run(
            Controls {
                pitch_up: 0.1,
                ..Default::default()
            },
            1,
        );
        assert!(
            pitch.orientation.forward().y > 0.05,
            "pitch up raises the nose"
        );
        let roll = run(
            Controls {
                roll_right: 1.0,
                ..Default::default()
            },
            10,
        );
        assert!(
            roll.orientation.right().y < -0.05,
            "roll right dips the right wing"
        );
    }

    #[test]
    fn look_is_spread_over_ticks_and_residue_carries() {
        let mut look = LookAccumulator::default();
        let c = Controls {
            yaw_right: 0.03,
            ..Default::default()
        };
        // No tick this frame: nothing applied yet.
        let cmd = look.command(c, 0, 1.0);
        assert_eq!(cmd.look_yaw, 0.0);
        // Next frame runs 3 ticks: the pending 0.03 rad is spread over them
        // (yaw right is a negative rotation about +Y).
        let cmd = look.command(Controls::default(), 3, 1.0);
        assert!((cmd.look_yaw + 0.01).abs() <= LOOK_STEP, "{}", cmd.look_yaw);
        assert_eq!(cmd.look_yaw, quantize_look(cmd.look_yaw));

        // Tiny movements below one wire step add up instead of vanishing.
        let mut look = LookAccumulator::default();
        let tiny = Controls {
            pitch_up: LOOK_STEP * 0.3,
            ..Default::default()
        };
        let total: f32 = (0..10).map(|_| look.command(tiny, 1, 1.0).look_pitch).sum();
        assert!((total - LOOK_STEP * 3.0).abs() <= LOOK_STEP, "{total}");
    }

    #[test]
    fn look_beyond_the_turn_limit_is_dropped_not_replayed() {
        let mut look = LookAccumulator::default();
        let big = Controls {
            yaw_right: 5.0,
            ..Default::default()
        };
        let first = look.command(big, 1, 0.2);
        assert!((first.look_yaw + 0.2).abs() < 1e-3);
        let next = look.command(Controls::default(), 1, 0.2);
        assert!(next.look_yaw.abs() <= LOOK_STEP);
    }

    #[test]
    fn controls_parse_from_a_flat_array() {
        let c = Controls::from_slice(&[1.0, -1.0, 0.5, 1.0, 0.1, -0.2, 3.0, 4.0]);
        assert_eq!(c.buttons, 3);
        assert_eq!(c.weapon_select, 4);
        assert!(
            c.select_request().is_empty(),
            "older 8-float arrays still parse"
        );
        let k = Controls::from_slice(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 64.0, 0.0, 10.0, -3.0, 1.0]);
        assert_eq!(k.buttons, Buttons::GADGET.0);
        assert_eq!(
            (k.weapon_key, k.cycle_primary, k.cycle_secondary),
            (10, -1, 1)
        );
        let clamp = Controls::from_slice(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 99.0, 0.0, 0.0]);
        assert_eq!(clamp.weapon_key, 10);
        let cmd = c.to_cmd(0.0, 0.0);
        assert!(cmd.buttons.contains(Buttons::BOOST));
        assert!(cmd.buttons.contains(Buttons::FIRE_PRIMARY));
        assert_eq!(cmd.thrust, Vec3::new(-1.0, 0.5, -1.0));
        assert_eq!(cmd.roll, -1.0);
        let short = Controls::from_slice(&[f32::NAN]);
        assert_eq!(short, Controls::default());
    }

    #[test]
    fn fixed_step_counts_ticks_and_clamps_stalls() {
        let mut s = FixedStep::default();
        assert_eq!(s.advance(1.0 / 144.0), 0);
        assert!(s.alpha() > 0.4 && s.alpha() < 0.42);
        let total: u32 = (0..143).map(|_| s.advance(1.0 / 144.0)).sum();
        // 144 frames of 1/144 s = one second = 60 ticks (59 if float rounding
        // leaves the last tick a hair short; it then runs on the next frame).
        assert!((59..=60).contains(&total), "{total}");
        assert_eq!(s.advance(5.0), MAX_TICKS_PER_FRAME);
        assert_eq!(s.advance(f64::NAN), 0);
        assert_eq!(s.advance(-1.0), 0);
    }
}
