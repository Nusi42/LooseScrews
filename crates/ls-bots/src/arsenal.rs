//! What the bots know about the weapons: every weapon of the match's table
//! as a [`Gun`] (ballistics, cost, slot, charge) with a [`Role`] from its
//! data id, and the distance bands in which each primary is worth using
//! (lasers mid range, kinetic long range, shotgun and Leech Arc close; see
//! [`primary_band`]).

use ls_game::ls_sim::{WeaponCost, WeaponFamily, WeaponId, WeaponSlot, WeaponTable};

/// A weapon by what it does (from its data id). Bots pick weapons by role,
/// so a data set without a weapon simply never offers that role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    ScrapLaser,
    ScatterLaser,
    PlasmaRepeater,
    FusionCharger,
    Corkscrew,
    RicochetArc,
    LeechArc,
    RivetGun,
    Railspike,
    ScrapShotgun,
    BoltRocket,
    Seeker,
    Quicksilver,
    Eyeball,
    SwarmPod,
    HeavyRocket,
    FlashPod,
    ProxMine,
    StalkerMine,
    TectonicCharge,
    EchoCannon,
    SwapHarpoon,
    PaintThrower,
    BorrowedTime,
    MagnetSneeze,
    PoliteLaser,
    SeedGrenade,
    /// A weapon the bots do not know (custom data): fired like a laser
    /// (primary) or a rocket (secondary).
    Other,
}

impl Role {
    /// The role of the weapon with data id `id`.
    pub fn from_id(id: &str) -> Role {
        use Role::*;
        match id {
            "scrap_laser" => ScrapLaser,
            "scatter_laser" => ScatterLaser,
            "plasma_repeater" => PlasmaRepeater,
            "fusion_charger" => FusionCharger,
            "corkscrew" => Corkscrew,
            "ricochet_arc" => RicochetArc,
            "leech_arc" => LeechArc,
            "rivet_gun" => RivetGun,
            "railspike" => Railspike,
            "scrap_shotgun" => ScrapShotgun,
            "bolt_rocket" => BoltRocket,
            "seeker" => Seeker,
            "quicksilver" => Quicksilver,
            "eyeball" => Eyeball,
            "swarm_pod" => SwarmPod,
            "heavy_rocket" => HeavyRocket,
            "flash_pod" => FlashPod,
            "prox_mine" => ProxMine,
            "stalker_mine" => StalkerMine,
            "tectonic_charge" => TectonicCharge,
            "echo_cannon" => EchoCannon,
            "swap_harpoon" => SwapHarpoon,
            "paint_thrower" => PaintThrower,
            "borrowed_time" => BorrowedTime,
            "magnet_sneeze" => MagnetSneeze,
            "polite_laser" => PoliteLaser,
            "seed_grenade" => SeedGrenade,
            _ => Other,
        }
    }

    /// Homing ordnance (worth it against a fleeing target).
    pub fn is_homing(self) -> bool {
        matches!(self, Role::Seeker | Role::SwarmPod | Role::Eyeball)
    }

    /// Mines (dropped behind when chased).
    pub fn is_mine(self) -> bool {
        matches!(self, Role::ProxMine | Role::StalkerMine)
    }
}

/// What the brain needs to know about one weapon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gun {
    pub id: WeaponId,
    pub role: Role,
    pub slot: WeaponSlot,
    pub family: WeaponFamily,
    /// Projectile speed (u/s).
    pub speed: f32,
    /// Projectile lifetime (s).
    pub lifetime: f32,
    pub projectile_radius: f32,
    pub cost: WeaponCost,
    /// Seconds between shots.
    pub fire_interval: f32,
    /// Hold-to-charge: (full charge s, shortest charge s, fires by itself at
    /// full charge).
    pub charge: Option<(f32, f32, bool)>,
    /// Splash radius (0 without splash) and whether it hurts the shooter.
    pub splash_radius: f32,
    pub splash_hurts_owner: bool,
}

impl Gun {
    /// Longest distance the projectile flies.
    pub fn range(&self) -> f32 {
        self.speed * self.lifetime
    }

    pub fn is_primary(&self) -> bool {
        self.slot == WeaponSlot::Primary
    }
}

/// Every weapon of the match's table, indexed by id, plus the M1 weapons by
/// role (any may be missing from a data set).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Arsenal {
    guns: Vec<Gun>,
    pub laser: Option<Gun>,
    pub rivet: Option<Gun>,
    pub rocket: Option<Gun>,
    pub paint: Option<Gun>,
}

impl Arsenal {
    pub fn from_table(table: &WeaponTable) -> Arsenal {
        let guns: Vec<Gun> = table
            .iter()
            .map(|(id, d)| Gun {
                id,
                role: Role::from_id(&d.id),
                slot: d.slot,
                family: d.family,
                speed: d.speed,
                lifetime: d.lifetime_secs,
                projectile_radius: d.projectile_radius,
                cost: d.cost,
                fire_interval: d.fire_interval_secs,
                charge: d.charge.map(|c| (c.max_secs, c.min_secs, c.auto_release)),
                splash_radius: d.splash.map_or(0.0, |s| s.radius),
                splash_hurts_owner: d.splash.is_some_and(|s| s.hits_owner),
            })
            .collect();
        let find = |r: Role| guns.iter().find(|g| g.role == r).copied();
        Arsenal {
            laser: find(Role::ScrapLaser),
            rivet: find(Role::RivetGun),
            rocket: find(Role::BoltRocket),
            paint: find(Role::PaintThrower),
            guns,
        }
    }

    /// The gun with this id.
    pub fn gun(&self, id: WeaponId) -> Option<Gun> {
        self.guns.iter().find(|g| g.id == id).copied()
    }

    /// The gun with this role.
    pub fn by_role(&self, role: Role) -> Option<Gun> {
        self.guns.iter().find(|g| g.role == role).copied()
    }

    /// Every weapon, in id order.
    pub fn guns(&self) -> &[Gun] {
        &self.guns
    }

    /// Weapon ids are dense indices into the table.
    pub fn len(&self) -> usize {
        self.guns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.guns.is_empty()
    }

    /// A primary worth firing at ships (not the Paint Thrower).
    pub fn is_combat_primary(&self, id: WeaponId) -> bool {
        self.gun(id)
            .is_some_and(|g| g.is_primary() && g.role != Role::PaintThrower)
    }
}

/// Distance bands (u) of a combat primary: `usable` is where it can hit at
/// all, `ideal` where the bots prefer it. `None` for weapons that are not
/// combat primaries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub usable: (f32, f32),
    pub ideal: (f32, f32),
    /// Distance (u) to keep from the target with this weapon (slow bolts
    /// need it close: a target further away dodges them).
    pub keep: f32,
}

impl Band {
    const fn new(usable: (f32, f32), ideal: (f32, f32), keep: f32) -> Band {
        Band {
            usable,
            ideal,
            keep,
        }
    }

    /// 0 outside the usable band, 1 inside, 3 in the ideal band.
    pub fn score(&self, d: f32) -> f32 {
        if d >= self.ideal.0 && d <= self.ideal.1 {
            3.0
        } else if d >= self.usable.0 && d <= self.usable.1 {
            1.0
        } else {
            0.0
        }
    }
}

/// See [`Band`]: lasers and other energy weapons mid range, kinetic weapons
/// long range, the Scrap Shotgun and the Leech Arc close.
pub fn primary_band(g: &Gun) -> Option<Band> {
    use Role::*;
    if !g.is_primary() {
        return None;
    }
    let reach = g.range();
    Some(match g.role {
        ScrapLaser => Band::new((0.0, 60.0), (8.0, 40.0), 16.0),
        ScatterLaser => Band::new((0.0, 40.0), (6.0, 25.0), 12.0),
        PlasmaRepeater => Band::new((0.0, 42.0), (5.0, 28.0), 12.0),
        FusionCharger => Band::new((6.0, 60.0), (12.0, 45.0), 18.0),
        Corkscrew => Band::new((0.0, 45.0), (5.0, 22.0), 10.0),
        RicochetArc => Band::new((0.0, 55.0), (10.0, 35.0), 16.0),
        EchoCannon => Band::new((0.0, 60.0), (10.0, 38.0), 16.0),
        PoliteLaser => Band::new((8.0, 90.0), (18.0, 70.0), 22.0),
        LeechArc => Band::new((0.0, 11.5), (0.0, 10.0), 7.0),
        ScrapShotgun => Band::new((0.0, 22.0), (0.0, 14.0), 7.0),
        RivetGun => Band::new((0.0, 150.0), (22.0, 120.0), 20.0),
        Railspike => Band::new((8.0, 200.0), (25.0, 150.0), 28.0),
        PaintThrower => return None,
        _ => Band::new((0.0, reach * 0.8), (6.0, reach * 0.5), 16.0),
    })
}

/// Distance bands (u) in which a secondary may be aimed at a ship, or
/// `None` for secondaries that are not aimed at ships (mines, Borrowed
/// Time, Magnet Sneeze's scrap use, ...), which the brain handles by
/// situation.
pub fn secondary_band(g: &Gun) -> Option<(f32, f32)> {
    use Role::*;
    if g.is_primary() {
        return None;
    }
    Some(match g.role {
        BoltRocket => (7.0, 35.0),
        Quicksilver => (6.0, 45.0),
        // Its 8 u splash hurts the shooter.
        HeavyRocket => (16.0, 55.0),
        Seeker => (8.0, 60.0),
        SwarmPod => (10.0, 50.0),
        Eyeball => (15.0, 80.0),
        // Blinds everyone within 15 u of the burst, the shooter too.
        FlashPod => (18.0, 40.0),
        SwapHarpoon => (15.0, 70.0),
        SeedGrenade => (10.0, 40.0),
        MagnetSneeze => (8.0, 25.0),
        TectonicCharge => (35.0, 75.0),
        Other => (8.0, (g.range() * 0.7).max(10.0)),
        ProxMine | StalkerMine | BorrowedTime => return None,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_weapon_has_a_role() {
        let table = WeaponTable::builtin();
        let a = Arsenal::from_table(&table);
        assert_eq!(a.len(), table.len());
        for g in a.guns() {
            assert_ne!(g.role, Role::Other, "{:?}", table.get(g.id).id);
            if g.is_primary() && g.role != Role::PaintThrower {
                assert!(primary_band(g).is_some());
            }
        }
        // The charge weapons are known as such.
        let fusion = a.by_role(Role::FusionCharger).unwrap();
        assert!(fusion
            .charge
            .is_some_and(|(max, _, auto)| max == 2.0 && !auto));
        let polite = a.by_role(Role::PoliteLaser).unwrap();
        assert!(polite
            .charge
            .is_some_and(|(_, min, auto)| min == 1.5 && auto));
        assert!(a.by_role(Role::HeavyRocket).unwrap().splash_hurts_owner);
    }

    #[test]
    fn bands_follow_the_weapon_classes() {
        let a = Arsenal::from_table(&WeaponTable::builtin());
        let band = |r: Role| primary_band(&a.by_role(r).unwrap()).unwrap();
        // Close: shotgun and Leech Arc; long: kinetic; mid: lasers.
        assert_eq!(band(Role::ScrapShotgun).score(8.0), 3.0);
        assert_eq!(band(Role::ScrapShotgun).score(50.0), 0.0);
        assert_eq!(band(Role::LeechArc).score(20.0), 0.0);
        assert_eq!(band(Role::Railspike).score(80.0), 3.0);
        assert_eq!(band(Role::RivetGun).score(80.0), 3.0);
        assert_eq!(band(Role::ScrapLaser).score(20.0), 3.0);
        assert_eq!(band(Role::ScrapLaser).score(100.0), 0.0);
        assert!(primary_band(&a.paint.unwrap()).is_none());
    }
}
