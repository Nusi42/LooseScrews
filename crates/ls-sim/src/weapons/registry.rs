//! The one place where weapons are registered: behaviour module plus
//! embedded data file, in design-doc order. Adding a weapon means adding its
//! module file, its `data/weapons/<id>.ron` and one entry here.
//!
//! The order is part of the rule set code format (`crate::rulecode`): append
//! new weapons at the end, never reorder.

use super::WeaponBehavior;
use crate::WeaponFamily;

use super::{energy, kinetic, ordnance, specials};

/// One registered weapon.
#[derive(Debug, Clone, Copy)]
pub struct WeaponModule {
    pub id: &'static str,
    pub family: WeaponFamily,
    pub behavior: &'static dyn WeaponBehavior,
    /// Data file name (for error messages and the data hash).
    pub file: &'static str,
    /// Embedded RON text of `data/weapons/<id>.ron`.
    pub data: &'static str,
}

/// Every weapon of the design doc, in design-doc order.
pub static MODULES: [WeaponModule; 27] = [
    WeaponModule {
        id: "scrap_laser",
        family: WeaponFamily::Energy,
        behavior: &energy::scrap_laser::ScrapLaser,
        file: "weapons/scrap_laser.ron",
        data: include_str!("../../../../data/weapons/scrap_laser.ron"),
    },
    WeaponModule {
        id: "scatter_laser",
        family: WeaponFamily::Energy,
        behavior: &energy::scatter_laser::ScatterLaser,
        file: "weapons/scatter_laser.ron",
        data: include_str!("../../../../data/weapons/scatter_laser.ron"),
    },
    WeaponModule {
        id: "plasma_repeater",
        family: WeaponFamily::Energy,
        behavior: &energy::plasma_repeater::PlasmaRepeater,
        file: "weapons/plasma_repeater.ron",
        data: include_str!("../../../../data/weapons/plasma_repeater.ron"),
    },
    WeaponModule {
        id: "fusion_charger",
        family: WeaponFamily::Energy,
        behavior: &energy::fusion_charger::FusionCharger,
        file: "weapons/fusion_charger.ron",
        data: include_str!("../../../../data/weapons/fusion_charger.ron"),
    },
    WeaponModule {
        id: "corkscrew",
        family: WeaponFamily::Energy,
        behavior: &energy::corkscrew::Corkscrew,
        file: "weapons/corkscrew.ron",
        data: include_str!("../../../../data/weapons/corkscrew.ron"),
    },
    WeaponModule {
        id: "ricochet_arc",
        family: WeaponFamily::Energy,
        behavior: &energy::ricochet_arc::RicochetArc,
        file: "weapons/ricochet_arc.ron",
        data: include_str!("../../../../data/weapons/ricochet_arc.ron"),
    },
    WeaponModule {
        id: "leech_arc",
        family: WeaponFamily::Energy,
        behavior: &energy::leech_arc::LeechArc,
        file: "weapons/leech_arc.ron",
        data: include_str!("../../../../data/weapons/leech_arc.ron"),
    },
    WeaponModule {
        id: "rivet_gun",
        family: WeaponFamily::Kinetic,
        behavior: &kinetic::rivet_gun::RivetGun,
        file: "weapons/rivet_gun.ron",
        data: include_str!("../../../../data/weapons/rivet_gun.ron"),
    },
    WeaponModule {
        id: "railspike",
        family: WeaponFamily::Kinetic,
        behavior: &kinetic::railspike::Railspike,
        file: "weapons/railspike.ron",
        data: include_str!("../../../../data/weapons/railspike.ron"),
    },
    WeaponModule {
        id: "scrap_shotgun",
        family: WeaponFamily::Kinetic,
        behavior: &kinetic::scrap_shotgun::ScrapShotgun,
        file: "weapons/scrap_shotgun.ron",
        data: include_str!("../../../../data/weapons/scrap_shotgun.ron"),
    },
    WeaponModule {
        id: "bolt_rocket",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::bolt_rocket::BoltRocket,
        file: "weapons/bolt_rocket.ron",
        data: include_str!("../../../../data/weapons/bolt_rocket.ron"),
    },
    WeaponModule {
        id: "seeker",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::seeker::Seeker,
        file: "weapons/seeker.ron",
        data: include_str!("../../../../data/weapons/seeker.ron"),
    },
    WeaponModule {
        id: "quicksilver",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::quicksilver::Quicksilver,
        file: "weapons/quicksilver.ron",
        data: include_str!("../../../../data/weapons/quicksilver.ron"),
    },
    WeaponModule {
        id: "eyeball",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::eyeball::Eyeball,
        file: "weapons/eyeball.ron",
        data: include_str!("../../../../data/weapons/eyeball.ron"),
    },
    WeaponModule {
        id: "swarm_pod",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::swarm_pod::SwarmPod,
        file: "weapons/swarm_pod.ron",
        data: include_str!("../../../../data/weapons/swarm_pod.ron"),
    },
    WeaponModule {
        id: "heavy_rocket",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::heavy_rocket::HeavyRocket,
        file: "weapons/heavy_rocket.ron",
        data: include_str!("../../../../data/weapons/heavy_rocket.ron"),
    },
    WeaponModule {
        id: "flash_pod",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::flash_pod::FlashPod,
        file: "weapons/flash_pod.ron",
        data: include_str!("../../../../data/weapons/flash_pod.ron"),
    },
    WeaponModule {
        id: "prox_mine",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::prox_mine::ProxMine,
        file: "weapons/prox_mine.ron",
        data: include_str!("../../../../data/weapons/prox_mine.ron"),
    },
    WeaponModule {
        id: "stalker_mine",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::stalker_mine::StalkerMine,
        file: "weapons/stalker_mine.ron",
        data: include_str!("../../../../data/weapons/stalker_mine.ron"),
    },
    WeaponModule {
        id: "tectonic_charge",
        family: WeaponFamily::Ordnance,
        behavior: &ordnance::tectonic_charge::TectonicCharge,
        file: "weapons/tectonic_charge.ron",
        data: include_str!("../../../../data/weapons/tectonic_charge.ron"),
    },
    WeaponModule {
        id: "echo_cannon",
        family: WeaponFamily::Special,
        behavior: &specials::echo_cannon::EchoCannon,
        file: "weapons/echo_cannon.ron",
        data: include_str!("../../../../data/weapons/echo_cannon.ron"),
    },
    WeaponModule {
        id: "swap_harpoon",
        family: WeaponFamily::Special,
        behavior: &specials::swap_harpoon::SwapHarpoon,
        file: "weapons/swap_harpoon.ron",
        data: include_str!("../../../../data/weapons/swap_harpoon.ron"),
    },
    WeaponModule {
        id: "paint_thrower",
        family: WeaponFamily::Special,
        behavior: &specials::paint_thrower::PaintThrower,
        file: "weapons/paint_thrower.ron",
        data: include_str!("../../../../data/weapons/paint_thrower.ron"),
    },
    WeaponModule {
        id: "borrowed_time",
        family: WeaponFamily::Special,
        behavior: &specials::borrowed_time::BorrowedTime,
        file: "weapons/borrowed_time.ron",
        data: include_str!("../../../../data/weapons/borrowed_time.ron"),
    },
    WeaponModule {
        id: "magnet_sneeze",
        family: WeaponFamily::Special,
        behavior: &specials::magnet_sneeze::MagnetSneeze,
        file: "weapons/magnet_sneeze.ron",
        data: include_str!("../../../../data/weapons/magnet_sneeze.ron"),
    },
    WeaponModule {
        id: "polite_laser",
        family: WeaponFamily::Special,
        behavior: &specials::polite_laser::PoliteLaser,
        file: "weapons/polite_laser.ron",
        data: include_str!("../../../../data/weapons/polite_laser.ron"),
    },
    WeaponModule {
        id: "seed_grenade",
        family: WeaponFamily::Special,
        behavior: &specials::seed_grenade::SeedGrenade,
        file: "weapons/seed_grenade.ron",
        data: include_str!("../../../../data/weapons/seed_grenade.ron"),
    },
];

/// The generic weapon (all default hooks), used for weapons without a module
/// (data loaded from a directory).
#[derive(Debug)]
pub struct Generic;

impl WeaponBehavior for Generic {
    fn id(&self) -> &'static str {
        ""
    }
}

/// Behaviour for a weapon id ([`Generic`] if none is registered).
pub fn behavior(id: &str) -> &'static dyn WeaponBehavior {
    #[cfg(test)]
    if let Some(b) = super::tests::test_behavior(id) {
        return b;
    }
    MODULES
        .iter()
        .find(|m| m.id == id)
        .map_or(&Generic as &'static dyn WeaponBehavior, |m| m.behavior)
}

/// Position of a weapon id in [`MODULES`] (rule set codes).
pub fn index_of(id: &str) -> Option<usize> {
    MODULES.iter().position(|m| m.id == id)
}
