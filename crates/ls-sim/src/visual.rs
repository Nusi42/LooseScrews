//! Data-driven visuals: every networked entity carries a [`VisualId`], an index
//! into the match's [`VisualTable`]. The table is built from the loaded data
//! (weapons, gadgets) plus a fixed block of built-in entity visuals, so the
//! server and every client derive the same ids from the same data (the data
//! hash in the handshake guarantees they have the same data).
//!
//! Layout of the table (ids are `u16`):
//!
//! | Ids | Visual |
//! | --- | --- |
//! | `0..BUILTIN_COUNT` | [`BuiltinVisual`] in declaration order (debris, scrap pile, wreck, ...) |
//! | then 2 per weapon, in [`crate::WeaponId`] order | projectile visual, pickup visual |
//! | then 1 per gadget, in [`crate::GadgetId`] order | gadget pickup visual |
//!
//! Renderers map [`Visual::shape`] to a mesh and tint it with [`Visual::color`].

use serde::{Deserialize, Serialize};

use crate::{GadgetId, GadgetTable, WeaponId, WeaponTable};

/// Declares [`Shape`] and [`Shape::ALL`] from one list, so the list can never
/// miss a variant.
macro_rules! shapes {
    ($($v:ident),* $(,)?) => {
        /// Mesh family a renderer draws for a visual.
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub enum Shape {
            $($v),*
        }

        impl Shape {
            /// Every shape in declaration order (`Shape::ALL[s.index()] == s`;
            /// the index renderers receive).
            pub const ALL: &'static [Shape] = &[$(Shape::$v),*];
        }
    };
}

shapes! {
    // Projectiles.
    Bolt,
    Pellet,
    Spike,
    Rocket,
    Missile,
    Mine,
    Orb,
    Blob,
    Arc,
    Grenade,
    Harpoon,
    Paint,
    Charge,
    // Other entities.
    Debris,
    Scrap,
    Wreck,
    Crate,
    Upgrade,
    Gadget,
    Decoy,
    Ghost,
    Dust,
    Duck,
    // M4 (append only).
    // The Paperweight's stone (B7): in flight and as a landed well.
    Paperweight,
}

impl Shape {
    /// Position in [`Shape::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// How to draw one kind of entity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Visual {
    pub shape: Shape,
    /// sRGB tint.
    pub color: (u8, u8, u8),
    /// Size multiplier on the shape's default size.
    #[serde(default = "one")]
    pub scale: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for Visual {
    fn default() -> Self {
        Visual {
            shape: Shape::Bolt,
            color: (255, 255, 255),
            scale: 1.0,
        }
    }
}

impl Visual {
    pub const fn new(shape: Shape, color: (u8, u8, u8), scale: f32) -> Visual {
        Visual {
            shape,
            color,
            scale,
        }
    }
}

/// Index into a [`VisualTable`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct VisualId(pub u16);

/// Visuals that do not come from weapon or gadget data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum BuiltinVisual {
    Debris = 0,
    ScrapPile = 1,
    Wreck = 2,
    Decoy = 3,
    Ghost = 4,
    DustCloud = 5,
    UpgradePickup = 6,
    RubberDuck = 7,
}

/// Number of [`BuiltinVisual`] entries at the start of every table.
pub const BUILTIN_COUNT: u16 = 8;

impl BuiltinVisual {
    pub const ALL: [BuiltinVisual; BUILTIN_COUNT as usize] = [
        BuiltinVisual::Debris,
        BuiltinVisual::ScrapPile,
        BuiltinVisual::Wreck,
        BuiltinVisual::Decoy,
        BuiltinVisual::Ghost,
        BuiltinVisual::DustCloud,
        BuiltinVisual::UpgradePickup,
        BuiltinVisual::RubberDuck,
    ];

    pub fn id(self) -> VisualId {
        VisualId(self as u16)
    }

    pub fn visual(self) -> Visual {
        match self {
            BuiltinVisual::Debris => Visual::new(Shape::Debris, (120, 110, 100), 1.0),
            BuiltinVisual::ScrapPile => Visual::new(Shape::Scrap, (170, 150, 110), 1.0),
            BuiltinVisual::Wreck => Visual::new(Shape::Wreck, (90, 80, 70), 1.0),
            BuiltinVisual::Decoy => Visual::new(Shape::Decoy, (255, 255, 255), 1.0),
            BuiltinVisual::Ghost => Visual::new(Shape::Ghost, (140, 200, 255), 1.0),
            BuiltinVisual::DustCloud => Visual::new(Shape::Dust, (150, 140, 120), 1.0),
            BuiltinVisual::UpgradePickup => Visual::new(Shape::Upgrade, (255, 220, 60), 1.0),
            BuiltinVisual::RubberDuck => Visual::new(Shape::Duck, (255, 220, 0), 1.0),
        }
    }
}

/// All visuals of a match; see the module docs for the id layout.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualTable {
    entries: Vec<Visual>,
    weapons: u16,
}

impl VisualTable {
    /// Builds the table for a data set.
    pub fn new(weapons: &WeaponTable, gadgets: &GadgetTable) -> VisualTable {
        let mut entries: Vec<Visual> = BuiltinVisual::ALL.iter().map(|b| b.visual()).collect();
        for (_, def) in weapons.iter() {
            let v = def.visual;
            entries.push(v);
            entries.push(Visual::new(Shape::Crate, v.color, 1.0));
        }
        for (_, def) in gadgets.iter() {
            entries.push(Visual::new(Shape::Gadget, def.visual.color, 1.0));
        }
        VisualTable {
            entries,
            weapons: weapons.len() as u16,
        }
    }

    /// Every visual in id order (renderers upload this once per match).
    pub fn entries(&self) -> &[Visual] {
        &self.entries
    }

    pub fn get(&self, id: VisualId) -> Option<&Visual> {
        self.entries.get(id.0 as usize)
    }

    /// Visual of a weapon's projectiles.
    pub fn projectile(&self, w: WeaponId) -> VisualId {
        VisualId(BUILTIN_COUNT + 2 * w.0 as u16)
    }

    /// Visual of a weapon pickup.
    pub fn weapon_pickup(&self, w: WeaponId) -> VisualId {
        VisualId(BUILTIN_COUNT + 2 * w.0 as u16 + 1)
    }

    /// Visual of a gadget pickup.
    pub fn gadget_pickup(&self, g: GadgetId) -> VisualId {
        VisualId(BUILTIN_COUNT + 2 * self.weapons + g.0 as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn shape_all_lists_every_shape_in_order() {
        for (i, s) in Shape::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
        }
        assert_eq!(Shape::ALL.len(), Shape::Paperweight.index() + 1);
    }

    #[test]
    fn table_layout_matches_ids() {
        let w = WeaponTable::builtin();
        let g = GadgetTable::builtin();
        let t = VisualTable::new(&w, &g);
        assert_eq!(
            t.entries().len(),
            BUILTIN_COUNT as usize + 2 * w.len() + g.len()
        );
        assert_eq!(
            t.get(BuiltinVisual::Wreck.id()).unwrap().shape,
            Shape::Wreck
        );
        let rocket = w.find("bolt_rocket").unwrap();
        assert_eq!(t.get(t.projectile(rocket)).unwrap().shape, Shape::Rocket);
        assert_eq!(t.get(t.weapon_pickup(rocket)).unwrap().shape, Shape::Crate);
        let shield = g.find("shield").unwrap();
        assert_eq!(t.get(t.gadget_pickup(shield)).unwrap().shape, Shape::Gadget);
        assert!(t.get(VisualId(u16::MAX)).is_none());
    }
}
