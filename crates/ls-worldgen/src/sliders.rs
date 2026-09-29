//! The seven world sliders.

use serde::{Deserialize, Serialize};

use crate::WorldgenError;

/// Largest value a slider may take.
pub const SLIDER_MAX: u8 = 100;

/// World shaping sliders, each `0..=100`.
///
/// Milestone 1 uses only [`Sliders::enclosure`] and [`Sliders::density`]; the
/// other five are validated, stored in share codes and passed around, but do not
/// change the generated terrain yet.
///
/// Deserializing validates the range (`serde(try_from)`), so serde input can never
/// produce a slider above [`SLIDER_MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "SlidersRepr")]
pub struct Sliders {
    /// 0 = open sky with floating islands, 50 = caverns, 100 = dense mine shafts.
    pub enclosure: u8,
    /// 0 = few obstacles, 100 = many pillars and floating rocks.
    pub density: u8,
    /// Inert in M1.
    pub verticality: u8,
    /// Inert in M1.
    pub gravity_chaos: u8,
    /// Inert in M1 (render-side only later).
    pub palette: u8,
    /// Inert in M1 (comedy director later).
    pub absurdity: u8,
    /// Inert in M1.
    pub instability: u8,
}

/// Unvalidated serde mirror of [`Sliders`] (same field names, same format).
#[derive(Deserialize)]
struct SlidersRepr {
    enclosure: u8,
    density: u8,
    verticality: u8,
    gravity_chaos: u8,
    palette: u8,
    absurdity: u8,
    instability: u8,
}

impl TryFrom<SlidersRepr> for Sliders {
    type Error = WorldgenError;
    fn try_from(r: SlidersRepr) -> Result<Self, Self::Error> {
        Sliders::from_array([
            r.enclosure,
            r.density,
            r.verticality,
            r.gravity_chaos,
            r.palette,
            r.absurdity,
            r.instability,
        ])
    }
}

impl Default for Sliders {
    fn default() -> Self {
        Sliders {
            enclosure: 50,
            density: 40,
            verticality: 50,
            gravity_chaos: 0,
            palette: 0,
            absurdity: 20,
            instability: 30,
        }
    }
}

impl Sliders {
    /// Sliders with the two active values set and the inert ones at their defaults.
    pub fn new(enclosure: u8, density: u8) -> Self {
        Sliders {
            enclosure,
            density,
            ..Sliders::default()
        }
    }

    /// All slider values in share-code order.
    pub fn to_array(self) -> [u8; 7] {
        [
            self.enclosure,
            self.density,
            self.verticality,
            self.gravity_chaos,
            self.palette,
            self.absurdity,
            self.instability,
        ]
    }

    /// Inverse of [`Sliders::to_array`]. Fails if any value exceeds [`SLIDER_MAX`].
    pub fn from_array(a: [u8; 7]) -> Result<Self, WorldgenError> {
        let s = Sliders {
            enclosure: a[0],
            density: a[1],
            verticality: a[2],
            gravity_chaos: a[3],
            palette: a[4],
            absurdity: a[5],
            instability: a[6],
        };
        s.validate()?;
        Ok(s)
    }

    /// Checks every slider is in `0..=100`.
    pub fn validate(&self) -> Result<(), WorldgenError> {
        const NAMES: [&str; 7] = [
            "enclosure",
            "density",
            "verticality",
            "gravity_chaos",
            "palette",
            "absurdity",
            "instability",
        ];
        for (name, v) in NAMES.iter().zip(self.to_array()) {
            if v > SLIDER_MAX {
                return Err(WorldgenError::SliderOutOfRange { name, value: v });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn serde_validates_range() {
        let s = Sliders::new(70, 30);
        let text = ron::to_string(&s).unwrap();
        assert_eq!(ron::from_str::<Sliders>(&text).unwrap(), s);
        let bad = text.replace("enclosure:70", "enclosure:101");
        assert_ne!(bad, text, "{text}");
        assert!(ron::from_str::<Sliders>(&bad).is_err());
    }

    #[test]
    fn array_round_trip_and_validation() {
        let s = Sliders {
            enclosure: 1,
            density: 2,
            verticality: 3,
            gravity_chaos: 4,
            palette: 5,
            absurdity: 6,
            instability: 100,
        };
        assert_eq!(Sliders::from_array(s.to_array()).unwrap(), s);
        assert!(Sliders::default().validate().is_ok());
        let err = Sliders::from_array([0, 101, 0, 0, 0, 0, 0]).unwrap_err();
        assert_eq!(
            err,
            WorldgenError::SliderOutOfRange {
                name: "density",
                value: 101
            }
        );
    }

    #[test]
    fn serde_ron() {
        let s = Sliders::new(80, 20);
        let text = ron::to_string(&s).unwrap();
        assert_eq!(ron::from_str::<Sliders>(&text).unwrap(), s);
    }
}
