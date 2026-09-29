//! Per-tick player input.

use ls_math::Vec3;
use serde::{Deserialize, Serialize};
use std::ops::BitOr;

/// Button bit set carried in an [`InputCmd`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub struct Buttons(pub u16);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);
    /// Shift: boost (raises the per-axis cap while boost lasts).
    pub const BOOST: Buttons = Buttons(1 << 0);
    /// Left mouse: fire the selected primary.
    pub const FIRE_PRIMARY: Buttons = Buttons(1 << 1);
    /// Right mouse: fire the selected secondary.
    pub const FIRE_SECONDARY: Buttons = Buttons(1 << 2);
    /// R: scavenge scrap from nearby debris and scrap pickups.
    pub const SCAVENGE: Buttons = Buttons(1 << 3);
    /// Allows the overdrive gamble: with energy empty, energy weapons burn hull
    /// at 2 hull per 1 energy.
    pub const OVERDRIVE: Buttons = Buttons(1 << 4);
    /// Horizon assist (slowly levels the roll). A client setting sent every tick.
    pub const HORIZON_ASSIST: Buttons = Buttons(1 << 5);

    #[inline]
    pub fn contains(self, other: Buttons) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    pub fn with(self, other: Buttons) -> Buttons {
        Buttons(self.0 | other.0)
    }

    #[inline]
    pub fn set(&mut self, other: Buttons, on: bool) {
        if on {
            self.0 |= other.0;
        } else {
            self.0 &= !other.0;
        }
    }
}

impl BitOr for Buttons {
    type Output = Buttons;
    #[inline]
    fn bitor(self, rhs: Buttons) -> Buttons {
        self.with(rhs)
    }
}

/// One tick of input for one ship.
///
/// Axes follow `docs/architecture/contracts.md`: ship-local `+X` right, `+Y` up,
/// `-Z` forward. So W is `thrust.z = -1`, D is `thrust.x = +1`, Space is `thrust.y = +1`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct InputCmd {
    /// Tick this command is meant for (carried for networking; the simulation
    /// applies whatever commands it is handed for the current step).
    pub tick: u64,
    /// Thrust per ship-local axis, each in `-1..=1`.
    pub thrust: Vec3,
    /// Pitch delta this tick in radians (rotation about local `+X`; positive = nose up).
    pub look_pitch: f32,
    /// Yaw delta this tick in radians (rotation about local `+Y`; positive = nose left).
    pub look_yaw: f32,
    /// Roll input in `-1..=1` (rotation about local `+Z`; positive = counter-clockwise
    /// as seen by the pilot).
    pub roll: f32,
    pub buttons: Buttons,
    /// `0` keeps the current selection; `n > 0` selects the weapon with
    /// [`crate::WeaponId`] `n - 1` into its slot if the ship owns it.
    pub weapon_select: u8,
}

impl InputCmd {
    /// A neutral command (no thrust, no buttons).
    pub fn idle(tick: u64) -> InputCmd {
        InputCmd {
            tick,
            ..InputCmd::default()
        }
    }

    /// Builder: ship-local thrust.
    pub fn with_thrust(mut self, thrust: Vec3) -> InputCmd {
        self.thrust = thrust;
        self
    }

    /// Builder: adds buttons.
    pub fn with_buttons(mut self, buttons: Buttons) -> InputCmd {
        self.buttons = self.buttons.with(buttons);
        self
    }

    /// Copy with every axis clamped to `-1..=1` and non-finite values zeroed.
    /// Look deltas are only made finite here; the ship clamps them to its turn rate.
    pub fn sanitized(&self) -> InputCmd {
        let axis = |v: f32| {
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        InputCmd {
            tick: self.tick,
            thrust: Vec3::new(
                axis(self.thrust.x),
                axis(self.thrust.y),
                axis(self.thrust.z),
            ),
            look_pitch: finite(self.look_pitch),
            look_yaw: finite(self.look_yaw),
            roll: axis(self.roll),
            buttons: self.buttons,
            weapon_select: self.weapon_select,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn buttons_ops() {
        let mut b = Buttons::BOOST | Buttons::SCAVENGE;
        assert!(b.contains(Buttons::BOOST));
        assert!(!b.contains(Buttons::FIRE_PRIMARY));
        b.set(Buttons::BOOST, false);
        assert!(!b.contains(Buttons::BOOST));
        assert!(b.contains(Buttons::SCAVENGE));
    }

    #[test]
    fn sanitize_clamps_and_zeroes() {
        let cmd = InputCmd {
            thrust: Vec3::new(3.0, f32::NAN, -2.0),
            look_pitch: f32::INFINITY,
            roll: -7.0,
            ..InputCmd::default()
        }
        .sanitized();
        assert_eq!(cmd.thrust, Vec3::new(1.0, 0.0, -1.0));
        assert_eq!(cmd.look_pitch, 0.0);
        assert_eq!(cmd.roll, -1.0);
    }
}
