//! Bot behavior: turns what the bot sees ([`Perception`]) into one
//! [`InputCmd`] per tick.
//!
//! Priorities, evaluated every tick:
//! 0. **Guide** an Eyeball the bot launched: its look input steers the
//!    missile towards the nearest enemy (the ship drifts meanwhile).
//! 1. **Flee** from Tectonic Charge zones: a charge in flight or stuck
//!    (seen as a projectile), or the rumble / crack cues
//!    ([`Brain::on_cue`]), makes bots inside the collapse zone boost away
//!    along a navigation route to a point outside it.
//! 2. **Scavenge** when hull is low: fly to the nearest known scrap source
//!    (pickup points and recent wrecks), hold still and hold scavenge until
//!    the hull is repaired or the source runs dry.
//! 3. **Combat** when an enemy is in range with line of sight: orbit at the
//!    distance the chosen primary likes, aim with lead for the equipped
//!    weapon's projectile speed and fire when the nose is on the lead
//!    point. Weapons are chosen by range and situation (see
//!    [`combat`](self::combat)): lasers mid range, kinetic long range,
//!    shotgun and Leech Arc close, homing ordnance at fleeing targets, mines
//!    when chased, the Tectonic Charge on clusters, Borrowed Time at low
//!    hull, and gadgets (shield against splash, cloak to approach, decoy
//!    when chased).
//! 4. **Collect** weapon, ammo and gadget pickups the bot wants that are
//!    close enough (it has to pick weapons up to use them).
//! 5. **Hunt** when an enemy is alive but out of sight or range: follow a
//!    [`Navigator`] route to it across the level (to the enemy spawn area
//!    when no enemy is alive), painting walls on the way. After
//!    [`BrainConfig::hunt_secs`] without a fight, or at the end of a route
//!    that cannot reach the enemy, the bot takes a break of
//!    [`BrainConfig::hunt_break_secs`] to paint and wander.
//! 6. **Paint and wander** otherwise: fly between random open-space points
//!    (sometimes pickup points) and paint nearby walls with the Paint
//!    Thrower, now and then cycling its colour
//!    ([`BrainConfig::paint_colour_button`]); Magnet Sneeze pulls scrap in.
//!
//! Long trips (hunting, wandering, travel to scrap) follow navigation routes
//! once the bot's grid is built (see [`crate::nav`]); before that, or when no
//! route exists, bots fly straight at the goal. Every movement request goes
//! through [`avoid_walls`], so bots stay clear of rock whatever the plan
//! says; in narrow passages the route lowers the wall clearance and speed.

mod combat;

use ls_game::client::{ClientSession, GUIDED_STATE_BIT};
use ls_game::ls_math::terrain::{TerrainEdit, TerrainQuery};
use ls_game::ls_math::{Aabb, Pcg32, Quat, Vec3, DT, TICK_HZ};
use ls_game::ls_net::ship_flags;
use ls_game::ls_sim::{
    proj_flags, Buttons, GadgetId, InputCmd, ReplicatedKind, ShipTuning, ShipType, WeaponCost,
    WeaponId,
};

use crate::aim::clear_shot;
pub use crate::arsenal::{Arsenal, Gun, Role};
use crate::nav::{NavConfig, NavSteer, Navigator};
use crate::steering::{
    angle_between, arrive, avoid_walls, look_toward, random_open_point, random_unit,
    thrust_for_velocity, SteerConfig,
};

use self::combat::{Danger, EyeballGuide, SecondaryIntent};

/// Another ship the bot can see (alive only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    /// Wire id.
    pub id: u8,
    pub team: u8,
    pub pos: Vec3,
    pub vel: Vec3,
}

/// The gadget in the bot's slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GadgetKind {
    Shield,
    Cloak,
    Decoy,
    /// A gadget the bots do not know (never used).
    Other,
}

impl GadgetKind {
    pub fn from_id(id: &str) -> GadgetKind {
        match id {
            "shield" => GadgetKind::Shield,
            "cloak" => GadgetKind::Cloak,
            "decoy" => GadgetKind::Decoy,
            _ => GadgetKind::Other,
        }
    }
}

/// A projectile or mine in flight (from the newest snapshot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileSeen {
    /// Wire id.
    pub id: u16,
    pub weapon: WeaponId,
    /// Ship id of the shooter.
    pub owner: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Its owner steers it (Eyeball).
    pub guided: bool,
    /// Stuck to a wall or ship (mines, Tectonic Charge).
    pub stuck: bool,
}

/// What a pickup holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickupItem {
    Weapon(WeaponId),
    Upgrade(WeaponId),
    Gadget(GadgetKind),
}

/// A pickup lying in the level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickupSeen {
    /// Wire entity id.
    pub id: u16,
    pub pos: Vec3,
    pub item: PickupItem,
}

/// Everything the brain looks at for one decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Perception {
    /// Tick the command will be stamped with (monotonic).
    pub tick: u64,
    /// This bot's ship (wire) id.
    pub id: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    pub orientation: Quat,
    pub alive: bool,
    pub team: u8,
    pub ship_type: ShipType,
    /// Resources in absolute units (hull points, energy, boost, scrap).
    pub hull: f32,
    pub energy: f32,
    pub boost: f32,
    pub scrap: f32,
    /// Selected primary weapon according to the newest snapshot.
    pub primary: Option<WeaponId>,
    /// Selected secondary weapon (from the inventory).
    pub secondary: Option<WeaponId>,
    /// Carried weapons with their ammo (0 for non-count weapons), once the
    /// server sent an inventory.
    pub weapons: Option<Vec<(WeaponId, u32)>>,
    pub gadget: Option<GadgetKind>,
    /// The gadget is switched on (Shield, Cloak).
    pub gadget_on: bool,
    /// Living ships other than this one.
    pub contacts: Vec<Contact>,
    pub projectiles: Vec<ProjectileSeen>,
    pub pickups: Vec<PickupSeen>,
    /// Scrap piles, wrecks and debris holding scrap (position, scrap).
    pub scrap_sources: Vec<(Vec3, f32)>,
}

impl Perception {
    /// Reads the predicted local ship, the newest own status and inventory,
    /// the interpolated remote ships, projectiles and entities from a
    /// session. `None` before the first snapshot anchored the prediction.
    pub fn from_session(s: &ClientSession) -> Option<Perception> {
        let ship = s.local_ship()?;
        let ship_type = s.ship_type();
        let own = s
            .latest_snapshot()
            .and_then(|snap| snap.ships.iter().find(|n| n.id == s.player()));
        let (hull, energy, _boost, scrap) = own
            .and_then(|n| n.status)
            .map(|st| s.conv().status_values_for(ship_type, &st))
            .unwrap_or((ship.hull, ship.energy, ship.boost, ship.scrap));
        let contacts = s
            .remote_ships()
            .iter()
            .filter(|n| n.flags & ship_flags::ALIVE != 0)
            .map(|n| Contact {
                id: n.id,
                team: s.side_of(n.id).unwrap_or(ship_flags::team(n.flags)),
                pos: n.pos,
                vel: n.vel,
            })
            .collect();
        let inv = s.inventory();
        let n_weapons = s.weapons().len();
        let weapon_id = |w: u8| ((w as usize) < n_weapons).then_some(WeaponId(w));
        let weapons = inv.map(|i| {
            i.weapons
                .iter()
                .filter_map(|w| weapon_id(w.weapon).map(|id| (id, u32::from(w.ammo))))
                .collect()
        });
        let gadgets = s.gadgets();
        let gadget = inv
            .map(|i| i.gadget)
            .filter(|g| (*g as usize) < gadgets.len())
            .map(|g| GadgetKind::from_id(&gadgets.get(GadgetId(g)).id));
        let projectiles = s
            .projectiles()
            .iter()
            .filter_map(|q| {
                Some(ProjectileSeen {
                    id: q.id,
                    weapon: weapon_id(q.kind)?,
                    owner: q.owner,
                    pos: q.pos,
                    vel: q.vel,
                    guided: q.state & GUIDED_STATE_BIT != 0,
                    stuck: q.state & (proj_flags::STUCK as u8) != 0,
                })
            })
            .collect();
        let mut pickups = Vec::new();
        let mut scrap_sources = Vec::new();
        for e in s.entities() {
            let item = match ReplicatedKind::from_u8(e.kind) {
                Some(ReplicatedKind::WeaponPickup) => weapon_id(e.state[0]).map(PickupItem::Weapon),
                Some(ReplicatedKind::UpgradePickup) => {
                    weapon_id(e.state[0]).map(PickupItem::Upgrade)
                }
                Some(ReplicatedKind::GadgetPickup) => {
                    ((e.state[0] as usize) < gadgets.len()).then(|| {
                        PickupItem::Gadget(GadgetKind::from_id(
                            &gadgets.get(GadgetId(e.state[0])).id,
                        ))
                    })
                }
                Some(
                    ReplicatedKind::ScrapPile | ReplicatedKind::Wreck | ReplicatedKind::Debris,
                ) => {
                    if e.state[0] > 0 {
                        scrap_sources.push((e.pos, f32::from(e.state[0])));
                    }
                    None
                }
                _ => None,
            };
            if let Some(item) = item {
                pickups.push(PickupSeen {
                    id: e.id,
                    pos: e.pos,
                    item,
                });
            }
        }
        Some(Perception {
            tick: s.next_input_tick(),
            id: s.player(),
            pos: ship.pos,
            vel: ship.vel,
            orientation: ship.orientation,
            alive: ship.alive,
            team: s.team(),
            ship_type,
            hull,
            energy,
            // The prediction tracks boost exactly; the status byte lags.
            boost: ship.boost,
            scrap,
            primary: own.map(|n| WeaponId(ship_flags::weapon(n.flags))),
            secondary: inv.and_then(|i| weapon_id(i.secondary)),
            weapons,
            gadget,
            gadget_on: own.is_some_and(|n| n.flags & ship_flags::GADGET_ACTIVE != 0),
            contacts,
            projectiles,
            pickups,
            scrap_sources,
        })
    }

    /// The team of ship `id` (this bot's own or a visible contact).
    pub fn team_of(&self, id: u8) -> Option<u8> {
        if id == self.id {
            return Some(self.team);
        }
        self.contacts.iter().find(|c| c.id == id).map(|c| c.team)
    }
}

/// Behavior tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrainConfig {
    pub steer: SteerConfig,
    pub nav: NavConfig,
    /// Enemies further than this (u) are ignored (long-range weapons
    /// extend it up to [`BrainConfig::long_engage_range`]).
    pub engage_range: f32,
    /// Engage range while carrying a long-range (kinetic) weapon.
    pub long_engage_range: f32,
    /// Distance (u) the bot tries to keep from its target (the chosen
    /// primary's ideal band overrides it).
    pub preferred_range: f32,
    /// Start scavenging below this hull fraction.
    pub low_hull: f32,
    /// Stop scavenging at this hull fraction.
    pub healed_hull: f32,
    /// Walls further than this (u) are not painted.
    pub paint_range: f32,
    /// Seconds spent painting one spot.
    pub paint_secs: f32,
    /// Energy kept back when painting (for the laser).
    pub paint_energy_reserve: f32,
    /// Bolt Rockets only at distances in this range (u): the splash hits
    /// the shooter too.
    pub rocket_range: (f32, f32),
    /// Seconds between rockets (plus up to one second of jitter).
    pub rocket_interval_secs: f32,
    /// Hunt at most this long without a fight before a break.
    pub hunt_secs: f32,
    /// Length of a hunting break (paint and wander).
    pub hunt_break_secs: f32,
    /// Pickups further than this (u) are not fetched.
    pub collect_range: f32,
    /// During a fight, wanted pickups within this range (u) are grabbed on
    /// the way (the bot keeps shooting), if the hull is above `low_hull`.
    pub combat_collect_range: f32,
    /// Give up on a pickup after this many seconds.
    pub collect_secs: f32,
    /// Button that cycles the Paint Thrower's colour (see
    /// [`PAINT_COLOUR_BUTTON`]; `None` never cycles).
    pub paint_colour_button: Option<Buttons>,
}

/// The Paint Thrower colour cycle button (M2 plan: the V key cycles red
/// bouncy, blue slippery, yellow sticky): `Buttons::WEAPON_MODE` ("weapon
/// mode of the selected primary", acting on the press edge). Bots press it
/// for single ticks.
pub const PAINT_COLOUR_BUTTON: Option<Buttons> = Some(Buttons::WEAPON_MODE);

impl Default for BrainConfig {
    fn default() -> Self {
        BrainConfig {
            steer: SteerConfig::default(),
            nav: NavConfig::default(),
            engage_range: 55.0,
            long_engage_range: 85.0,
            preferred_range: 16.0,
            low_hull: 0.35,
            healed_hull: 0.85,
            paint_range: 24.0,
            paint_secs: 1.5,
            paint_energy_reserve: 20.0,
            rocket_range: (7.0, 35.0),
            rocket_interval_secs: 1.5,
            hunt_secs: 90.0,
            hunt_break_secs: 8.0,
            collect_range: 60.0,
            combat_collect_range: 30.0,
            collect_secs: 15.0,
            paint_colour_button: PAINT_COLOUR_BUTTON,
        }
    }
}

/// What the bot is doing (for logs and stats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dead,
    Wander,
    Paint,
    Combat,
    /// Travelling towards enemies out of sight (or their spawn area).
    Hunt,
    Scavenge,
    /// Fetching a pickup.
    Collect,
    /// Getting out of a Tectonic Charge zone.
    Flee,
    /// Steering an Eyeball (the ship drifts).
    Guide,
}

/// Highest weapon id the per-weapon counters track.
pub const MAX_TRACKED_WEAPONS: usize = 32;

/// Per-bot behavior counters (ticks spent per mode, trigger ticks per
/// weapon; a trigger tick is a tick the button was held with that weapon
/// equipped, not necessarily a shot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrainCounters {
    pub wander_ticks: u64,
    pub paint_ticks: u64,
    pub combat_ticks: u64,
    /// Ticks spent flying towards enemies out of sight (global navigation).
    pub hunt_ticks: u64,
    pub scavenge_ticks: u64,
    pub collect_ticks: u64,
    pub flee_ticks: u64,
    pub guide_ticks: u64,
    pub dead_ticks: u64,
    pub laser_triggers: u64,
    pub rivet_triggers: u64,
    pub rocket_triggers: u64,
    pub paint_triggers: u64,
    /// Secondary trigger presses (any secondary).
    pub secondary_triggers: u64,
    /// Trigger ticks per weapon id (primaries and secondaries).
    pub weapon_triggers: [u64; MAX_TRACKED_WEAPONS],
    /// Gadget button presses.
    pub gadget_presses: u64,
    /// Paint colour cycle presses.
    pub colour_presses: u64,
    /// Ticks the scavenge button was held.
    pub scavenge_holds: u64,
}

impl Default for BrainCounters {
    fn default() -> Self {
        BrainCounters {
            wander_ticks: 0,
            paint_ticks: 0,
            combat_ticks: 0,
            hunt_ticks: 0,
            scavenge_ticks: 0,
            collect_ticks: 0,
            flee_ticks: 0,
            guide_ticks: 0,
            dead_ticks: 0,
            laser_triggers: 0,
            rivet_triggers: 0,
            rocket_triggers: 0,
            paint_triggers: 0,
            secondary_triggers: 0,
            weapon_triggers: [0; MAX_TRACKED_WEAPONS],
            gadget_presses: 0,
            colour_presses: 0,
            scavenge_holds: 0,
        }
    }
}

impl BrainCounters {
    pub fn add(&mut self, o: &BrainCounters) {
        self.wander_ticks += o.wander_ticks;
        self.paint_ticks += o.paint_ticks;
        self.combat_ticks += o.combat_ticks;
        self.hunt_ticks += o.hunt_ticks;
        self.scavenge_ticks += o.scavenge_ticks;
        self.collect_ticks += o.collect_ticks;
        self.flee_ticks += o.flee_ticks;
        self.guide_ticks += o.guide_ticks;
        self.dead_ticks += o.dead_ticks;
        self.laser_triggers += o.laser_triggers;
        self.rivet_triggers += o.rivet_triggers;
        self.rocket_triggers += o.rocket_triggers;
        self.paint_triggers += o.paint_triggers;
        self.secondary_triggers += o.secondary_triggers;
        for (a, b) in self.weapon_triggers.iter_mut().zip(o.weapon_triggers) {
            *a += b;
        }
        self.gadget_presses += o.gadget_presses;
        self.colour_presses += o.colour_presses;
        self.scavenge_holds += o.scavenge_holds;
    }

    fn trigger(&mut self, w: WeaponId) {
        if let Some(c) = self.weapon_triggers.get_mut(w.0 as usize) {
            *c += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PaintJob {
    point: Vec3,
    /// Direction across the wall the aim sweeps along.
    sweep: Vec3,
    started: u64,
    until: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ScavengeJob {
    spot: Vec3,
    started: u64,
    /// Tick and hull of the last progress check while in reach.
    check_tick: Option<u64>,
    check_hull: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CollectJob {
    /// Wire entity id of the pickup.
    id: u16,
    pos: Vec3,
    item: PickupItem,
    started: u64,
    /// First tick the bot was on the pickup (it stays if the server does
    /// not hand it over, e.g. an upgrade past the top level).
    reached: Option<u64>,
}

/// The per-bot decision maker. See the module docs.
#[derive(Debug, Clone)]
pub struct Brain {
    cfg: BrainConfig,
    arsenal: Arsenal,
    tuning: ShipTuning,
    ship_type: ShipType,
    bounds: Aabb,
    rng: Pcg32,
    mode: Mode,
    was_alive: bool,

    target: Option<u8>,
    next_target_scan: u64,
    strafe_sign: f32,
    next_strafe_flip: u64,
    /// Distance kept from the target (from the chosen primary).
    preferred_range: f32,

    // ----- weapons (see `combat`) -----
    /// Primary chosen for the current fight and when to choose again.
    primary_choice: Option<WeaponId>,
    next_primary_pick: u64,
    /// Tick the trigger of a charge weapon went down, and when to let go.
    charge_started: Option<u64>,
    charge_release: u64,
    next_charge: u64,
    secondary_intent: Option<SecondaryIntent>,
    next_secondary: u64,
    next_rocket: u64,
    /// Tick the bot first carried a Tectonic Charge (this life).
    tectonic_since: Option<u64>,
    /// Trigger presses per weapon id this match (novelty: bots try weapons
    /// they have not used yet).
    used: Vec<u32>,
    /// Newest tick this bot took damage, and whether from a splash weapon.
    damaged_tick: Option<u64>,
    splash_threat_until: u64,
    danger: Option<Danger>,
    flee_goal: Option<Vec3>,
    eyeball: Option<EyeballGuide>,
    next_gadget_press: u64,
    decoy_ready: u64,
    /// The gadget this bot likes to pick up.
    gadget_pref: GadgetKind,

    wander: Option<Vec3>,
    wander_best: f32,
    wander_progress_tick: u64,

    paint: Option<PaintJob>,
    next_paint_search: u64,
    painted: Vec<Vec3>,
    colour_presses: u8,
    next_colour_press: u64,

    collect: Option<CollectJob>,
    /// Pickups given up on (entity id, until).
    skipped_pickups: Vec<(u16, u64)>,

    scrap_spots: Vec<Vec3>,
    wrecks: Vec<(Vec3, u64)>,
    exhausted: Vec<(Vec3, u64)>,
    scavenge: Option<ScavengeJob>,

    nav: Navigator,
    /// Spawn points of all teams (team, position).
    spawns: Vec<(u8, Vec3)>,
    hunt_target: Option<u8>,
    hunt_spawn: Option<Vec3>,
    /// Tick the current hunt (without a fight) started.
    hunt_since: Option<u64>,
    hunt_break_until: u64,
}

const TICKS_PER_SEC: f32 = TICK_HZ as f32;
/// Stream id of the bot's own (non-simulation) RNG.
const RNG_STREAM_BRAIN: u64 = 0x424f_545f_4252_4e31; // "BOT_BRN1"
/// Wrecks stay interesting for this many ticks.
const WRECK_TICKS: u64 = 45 * TICK_HZ as u64;
/// A scrap source that yielded nothing is skipped for this many ticks.
const EXHAUSTED_TICKS: u64 = 30 * TICK_HZ as u64;
/// Scavenge progress is checked this often while in reach.
const SCAVENGE_CHECK_TICKS: u64 = 90;
/// Give up on reaching a scrap source after this many ticks.
const SCAVENGE_TRAVEL_TICKS: u64 = 20 * TICK_HZ as u64;
/// A pickup given up on is ignored this long.
const SKIP_PICKUP_TICKS: u64 = 40 * TICK_HZ as u64;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Plan {
    desired_vel: Vec3,
    /// Distance to the movement goal along `desired_vel` (infinite if none).
    goal_dist: f32,
    boost: bool,
    aim: Option<Vec3>,
    select: Option<WeaponId>,
    select_secondary: Option<WeaponId>,
    fire_primary: bool,
    fire_secondary: bool,
    scavenge: bool,
    gadget: bool,
    /// Wall clearance for [`avoid_walls`] instead of the configured one
    /// (narrow passages on a route).
    clearance: Option<f32>,
}

impl Plan {
    fn new() -> Plan {
        Plan {
            desired_vel: Vec3::ZERO,
            goal_dist: f32::INFINITY,
            boost: false,
            aim: None,
            select: None,
            select_secondary: None,
            fire_primary: false,
            fire_secondary: false,
            scavenge: false,
            gadget: false,
            clearance: None,
        }
    }
}

impl Brain {
    /// A brain for a level with the given bounds. `scrap_spots` are the
    /// match's pickup points (scrap pickups spawn there); `seed` makes each
    /// bot behave differently.
    pub fn new(
        cfg: BrainConfig,
        arsenal: Arsenal,
        tuning: ShipTuning,
        bounds: Aabb,
        scrap_spots: Vec<Vec3>,
        seed: u64,
    ) -> Brain {
        let mut rng = Pcg32::from_seed(seed, RNG_STREAM_BRAIN);
        let strafe_sign = if rng.next_bool() { 1.0 } else { -1.0 };
        let gadget_pref =
            [GadgetKind::Shield, GadgetKind::Cloak, GadgetKind::Decoy][rng.below(3) as usize];
        let used = vec![0; arsenal.len()];
        Brain {
            cfg,
            arsenal,
            tuning,
            ship_type: ShipType::Tinker,
            bounds,
            rng,
            mode: Mode::Dead,
            was_alive: false,
            target: None,
            next_target_scan: 0,
            strafe_sign,
            next_strafe_flip: 0,
            preferred_range: cfg.preferred_range,
            primary_choice: None,
            next_primary_pick: 0,
            charge_started: None,
            charge_release: 0,
            next_charge: 0,
            secondary_intent: None,
            next_secondary: 0,
            next_rocket: 0,
            tectonic_since: None,
            used,
            damaged_tick: None,
            splash_threat_until: 0,
            danger: None,
            flee_goal: None,
            eyeball: None,
            next_gadget_press: 0,
            decoy_ready: 0,
            gadget_pref,
            wander: None,
            wander_best: f32::MAX,
            wander_progress_tick: 0,
            paint: None,
            next_paint_search: 0,
            painted: Vec::new(),
            colour_presses: 0,
            next_colour_press: 0,
            collect: None,
            skipped_pickups: Vec::new(),
            scrap_spots,
            wrecks: Vec::new(),
            exhausted: Vec::new(),
            scavenge: None,
            nav: Navigator::new(bounds, cfg.nav),
            spawns: Vec::new(),
            hunt_target: None,
            hunt_spawn: None,
            hunt_since: None,
            hunt_break_until: 0,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Wire id of the current target.
    pub fn target(&self) -> Option<u8> {
        self.target
    }

    pub fn arsenal(&self) -> &Arsenal {
        &self.arsenal
    }

    /// The ship type the brain flies (its tuning is used for steering).
    pub fn ship_type(&self) -> ShipType {
        self.ship_type
    }

    /// Switches to another ship type's tuning (the session reports the
    /// type of the current life).
    pub fn set_ship(&mut self, ship_type: ShipType, tuning: &ShipTuning) {
        if self.ship_type != ship_type || self.tuning != *tuning {
            self.ship_type = ship_type;
            self.tuning = tuning.clone();
        }
    }

    /// Sets the known scrap sources besides wrecks (the match's pickup points).
    pub fn set_scrap_spots(&mut self, spots: Vec<Vec3>) {
        self.scrap_spots = spots;
    }

    /// Known pickup points.
    pub fn scrap_spots(&self) -> &[Vec3] {
        &self.scrap_spots
    }

    /// Sets the match's spawn points (team, position); bots with no enemy
    /// alive hunt near the other teams' spawns.
    pub fn set_spawn_points(&mut self, spawns: Vec<(u8, Vec3)>) {
        self.spawns = spawns;
    }

    /// The navigation grid and route.
    pub fn navigator(&self) -> &Navigator {
        &self.nav
    }

    /// Replaces the navigator (e.g. by one that adopts a shared grid, see
    /// [`Navigator::shared`]).
    pub fn set_navigator(&mut self, nav: Navigator) {
        self.nav = nav;
    }

    /// A terrain edit arrived: craters (and added rock) update the
    /// navigation grid.
    pub fn on_terrain_edit(&mut self, edit: &TerrainEdit) {
        self.nav.on_edit(edit);
    }

    /// Wire id of the enemy being hunted.
    pub fn hunt_target(&self) -> Option<u8> {
        self.hunt_target
    }

    /// A ship was destroyed at `pos` (tick `now`): its wreck holds scrap.
    pub fn on_wreck(&mut self, pos: Vec3, now: u64) {
        self.wrecks.push((pos, now + WRECK_TICKS));
        if self.wrecks.len() > 16 {
            self.wrecks.remove(0);
        }
    }

    /// Decides the command for one tick and counts what it did.
    pub fn think<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        counters: &mut BrainCounters,
    ) -> InputCmd {
        let mut cmd = InputCmd::idle(p.tick);
        cmd.buttons = Buttons::HORIZON_ASSIST;
        // The grid builds (and follows edits) in the background, dead or alive.
        self.nav.update(terrain);
        if !p.alive {
            self.on_death();
            counters.dead_ticks += 1;
            return cmd;
        }
        if !self.was_alive {
            self.on_spawn(p.tick);
        }
        self.was_alive = true;
        self.expire(p);
        self.observe_threats(p);

        // An Eyeball in flight takes the look input; the ship only drifts.
        if self.guide_eyeball(p, &mut cmd) {
            self.mode = Mode::Guide;
            counters.guide_ticks += 1;
            return cmd;
        }

        let mut plan = Plan::new();
        let target = self.update_target(p, terrain);
        let hull_frac = p.hull / self.tuning.hull_max.max(1.0);
        let wants_scrap = if self.scavenge.is_some() {
            hull_frac < self.cfg.healed_hull
        } else {
            hull_frac < self.cfg.low_hull
        };
        if !wants_scrap {
            self.scavenge = None;
        } else if self.scavenge.is_none() {
            self.scavenge = self.pick_scrap_spot(p).map(|spot| ScavengeJob {
                spot,
                started: p.tick,
                check_tick: None,
                check_hull: p.hull,
            });
        }

        if target.is_some() {
            self.hunt_since = None;
        }
        self.mode = if self.plan_flee(p, terrain, &mut plan) {
            Mode::Flee
        } else if self.scavenge.is_some() {
            self.plan_scavenge(p, terrain, &mut plan);
            Mode::Scavenge
        } else if let Some(t) = target {
            // A wanted pickup close by is grabbed mid-fight (the bot keeps
            // aiming and firing at its target); secondaries mostly come
            // from pickups.
            let near = self.cfg.combat_collect_range;
            if hull_frac > self.cfg.low_hull && self.plan_collect(p, terrain, &mut plan, near) {
                Mode::Collect
            } else {
                self.plan_orbit(p, terrain, &t, &mut plan);
                Mode::Combat
            }
        } else if self.plan_collect(p, terrain, &mut plan, self.cfg.collect_range) {
            Mode::Collect
        } else if self.plan_hunt(p, terrain, &mut plan) {
            Mode::Hunt
        } else {
            self.plan_wander(p, terrain, &mut plan);
            if self.paint.is_some() {
                Mode::Paint
            } else {
                Mode::Wander
            }
        };

        if let Some(t) = target {
            self.plan_fire(p, &t, terrain, &mut plan);
            self.paint = None;
        } else {
            self.charge_started = None;
            if !matches!(self.mode, Mode::Scavenge | Mode::Flee) {
                self.plan_paint(p, terrain, &mut plan);
                if self.paint.is_some() && self.mode == Mode::Wander {
                    self.mode = Mode::Paint;
                }
            }
        }
        self.plan_secondary(p, target.as_ref(), terrain, &mut plan);
        self.plan_gadget(p, target.as_ref(), &mut plan);

        match self.mode {
            Mode::Wander => counters.wander_ticks += 1,
            Mode::Paint => counters.paint_ticks += 1,
            Mode::Combat => counters.combat_ticks += 1,
            Mode::Hunt => counters.hunt_ticks += 1,
            Mode::Scavenge => counters.scavenge_ticks += 1,
            Mode::Collect => counters.collect_ticks += 1,
            Mode::Flee => counters.flee_ticks += 1,
            Mode::Guide => counters.guide_ticks += 1,
            Mode::Dead => counters.dead_ticks += 1,
        }

        self.write_command(p, terrain, &plan, &mut cmd, counters);
        cmd
    }

    // ----- life cycle --------------------------------------------------------

    fn on_death(&mut self) {
        self.mode = Mode::Dead;
        self.was_alive = false;
        self.target = None;
        self.paint = None;
        self.scavenge = None;
        self.wander = None;
        self.collect = None;
        self.nav.clear_route();
        self.hunt_target = None;
        self.hunt_since = None;
        // Respawning resets the loadout to the rule set's starting weapons.
        self.primary_choice = None;
        self.charge_started = None;
        self.secondary_intent = None;
        self.tectonic_since = None;
        self.eyeball = None;
        self.flee_goal = None;
        self.damaged_tick = None;
    }

    fn on_spawn(&mut self, now: u64) {
        self.wander = None;
        self.next_paint_search = now + 30;
        self.next_rocket = now + 60;
        self.next_secondary = now + 60;
        self.next_gadget_press = now + 30;
    }

    fn expire(&mut self, p: &Perception) {
        let now = p.tick;
        self.wrecks.retain(|(_, until)| *until > now);
        self.exhausted.retain(|(_, until)| *until > now);
        self.skipped_pickups.retain(|(_, until)| *until > now);
        if self.danger.is_some_and(|d| d.until <= now) {
            self.danger = None;
            self.flee_goal = None;
        }
    }

    /// Carried weapons with ammo. Without an inventory from the server the
    /// bot assumes the selected primary plus the default loadout.
    fn owned(&self, p: &Perception) -> Vec<(WeaponId, u32)> {
        if let Some(w) = &p.weapons {
            return w.clone();
        }
        let mut v: Vec<(WeaponId, u32)> = Vec::new();
        let mut add = |w: WeaponId, ammo: u32| {
            if !v.iter().any(|(x, _)| *x == w) {
                v.push((w, ammo));
            }
        };
        if let Some(w) = p.primary {
            add(w, 0);
        }
        for g in [self.arsenal.laser, self.arsenal.paint]
            .into_iter()
            .flatten()
        {
            add(g.id, 0);
        }
        if let Some(r) = self.arsenal.rocket {
            add(r.id, 1);
        }
        v
    }

    /// Whether the bot can pay one shot of `gun` (keeping `reserve` energy).
    fn can_pay(&self, p: &Perception, gun: &Gun, ammo: u32, reserve: f32) -> bool {
        match gun.cost {
            WeaponCost::Energy(c) => p.energy >= c + reserve,
            WeaponCost::Scrap(c) => p.scrap >= c,
            WeaponCost::Count(c) => ammo >= c.max(1),
        }
    }

    // ----- targets -----------------------------------------------------------

    fn update_target<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
    ) -> Option<Contact> {
        let enemy = |c: &&Contact| c.team != p.team;
        let range = self.engage_range(p);
        let current = self
            .target
            .and_then(|id| p.contacts.iter().find(|c| c.id == id))
            .filter(enemy)
            .copied();
        if p.tick < self.next_target_scan {
            // Between scans keep the target while it stays roughly in range.
            let keep = current.filter(|c| c.pos.distance(p.pos) < range * 1.2);
            self.target = keep.map(|c| c.id);
            return keep;
        }
        self.next_target_scan = p.tick + 6;
        let mut best: Option<(f32, Contact)> = None;
        for c in p.contacts.iter().filter(enemy) {
            let mut d = c.pos.distance(p.pos);
            if d > range {
                continue;
            }
            // Hysteresis: prefer the current target unless another is much closer.
            if Some(c.id) == self.target {
                d *= 0.7;
            }
            if best.as_ref().is_some_and(|(bd, _)| *bd <= d) {
                continue;
            }
            if clear_shot(terrain, p.pos, c.pos, 0.0) {
                best = Some((d, *c));
            }
        }
        self.target = best.map(|(_, c)| c.id);
        best.map(|(_, c)| c)
    }

    /// Engage range: longer while carrying a long-range kinetic weapon.
    fn engage_range(&self, p: &Perception) -> f32 {
        let long = p.weapons.as_ref().is_some_and(|w| {
            w.iter().any(|(id, _)| {
                self.arsenal
                    .gun(*id)
                    .is_some_and(|g| matches!(g.role, Role::RivetGun | Role::Railspike))
            })
        });
        if long {
            self.cfg.long_engage_range.max(self.cfg.engage_range)
        } else {
            self.cfg.engage_range
        }
    }

    // ----- movement plans ----------------------------------------------------

    fn plan_orbit<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        t: &Contact,
        plan: &mut Plan,
    ) {
        let cruise = self.tuning.cruise_speed;
        let preferred = self.preferred_range;
        let to = t.pos - p.pos;
        let dist = to.length().max(1e-3);
        // Far away (in caves the straight way in is usually blocked): close
        // in along a route, guns on the target.
        if dist > preferred + 12.0
            && self
                .plan_travel(p, terrain, t.pos, cruise, false, plan)
                .is_some()
        {
            plan.aim = Some(t.pos);
            return;
        }
        let radial = to / dist;
        if p.tick >= self.next_strafe_flip {
            self.strafe_sign = -self.strafe_sign;
            self.next_strafe_flip = p.tick + 90 + self.rng.below(150) as u64;
        }
        let mut tangent = radial.cross(Vec3::Y).normalize_or_zero();
        if tangent == Vec3::ZERO {
            tangent = radial.any_orthonormal();
        }
        let binormal = radial.cross(tangent);
        let radial_speed = ((dist - preferred) / 6.0).clamp(-1.0, 1.0) * cruise;
        let bob = (p.tick as f32 * 0.035).sin() * 0.35 * cruise;
        plan.desired_vel =
            radial * radial_speed + tangent * (self.strafe_sign * 0.6 * cruise) + binormal * bob;
        plan.desired_vel = plan.desired_vel.clamp_length(cruise);
        plan.aim = Some(t.pos);
    }

    fn plan_wander<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) {
        let reached = self.wander.is_some_and(|w| w.distance(p.pos) < 5.0);
        let stuck = p.tick > self.wander_progress_tick + 5 * TICK_HZ as u64;
        if self.wander.is_none() || reached || stuck {
            self.wander = self.pick_wander_point(p, terrain);
            self.wander_best = f32::MAX;
            self.wander_progress_tick = p.tick;
        }
        let Some(goal) = self.wander else {
            return;
        };
        let dist = goal.distance(p.pos);
        if dist < self.wander_best - 2.0 {
            self.wander_best = dist;
            self.wander_progress_tick = p.tick;
        }
        let painting = self.paint.is_some();
        let boost_ok = !painting
            && dist > 40.0
            && p.boost > 0.6 * self.tuning.boost_max
            && p.vel.length() > 0.8 * self.tuning.cruise_speed;
        let speed = if painting {
            0.4 * self.tuning.cruise_speed
        } else if boost_ok {
            self.tuning.boost_speed
        } else {
            self.tuning.cruise_speed
        };
        if self
            .plan_travel(p, terrain, goal, speed, boost_ok, plan)
            .is_none()
        {
            plan.boost = boost_ok;
            plan.desired_vel = arrive(p.pos, goal, speed, 8.0);
            plan.goal_dist = dist;
        }
    }

    /// Flies along a navigation route to `goal` at up to `speed` (capped at
    /// cruise speed and slower in narrow passages; boosting on long, wide
    /// stretches if `boost`). `None`, with `plan` untouched, while there is
    /// no route.
    fn plan_travel<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        goal: Vec3,
        speed: f32,
        boost: bool,
        plan: &mut Plan,
    ) -> Option<NavSteer> {
        let s = self.nav.steer(terrain, p.pos, goal, p.tick)?;
        let t = &self.tuning;
        let narrow = ((s.clearance - 0.3) / 0.9).clamp(0.4, 1.0);
        let boosting = boost
            && s.clearance >= 4.0
            && s.dist > 20.0
            && p.boost > 0.6 * t.boost_max
            && p.vel.length() > 0.8 * t.cruise_speed;
        let v = if boosting {
            t.boost_speed
        } else {
            speed.min(t.cruise_speed) * narrow
        };
        plan.desired_vel = if s.last {
            arrive(p.pos, s.waypoint, v, 8.0)
        } else {
            (s.waypoint - p.pos).normalize_or_zero() * v
        };
        plan.goal_dist = s.dist;
        plan.boost = boosting;
        plan.clearance = Some((s.clearance * 0.8).clamp(0.6, self.cfg.steer.clearance));
        Some(s)
    }

    /// Hunting: travel towards the chosen enemy (or an enemy spawn point).
    /// False if the bot should not or cannot hunt right now.
    fn plan_hunt<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) -> bool {
        if p.tick < self.hunt_break_until || !self.nav.is_ready() {
            return false;
        }
        let since = *self.hunt_since.get_or_insert(p.tick);
        if p.tick > since + (self.cfg.hunt_secs * TICKS_PER_SEC) as u64 {
            self.take_hunt_break(p.tick);
            return false;
        }
        let Some((goal, is_spawn)) = self.hunt_goal(p) else {
            return false;
        };
        let cruise = self.tuning.cruise_speed;
        let Some(s) = self.plan_travel(p, terrain, goal, cruise, true, plan) else {
            return false;
        };
        if s.last && s.dist < 4.0 {
            if is_spawn {
                // Nobody here: try another spawn point.
                self.hunt_spawn = None;
            } else if !s.complete {
                // As close as the level allows: give up on it for a while.
                self.take_hunt_break(p.tick);
            }
        }
        true
    }

    fn take_hunt_break(&mut self, now: u64) {
        self.hunt_break_until = now + (self.cfg.hunt_break_secs * TICKS_PER_SEC) as u64;
        self.hunt_since = None;
        self.hunt_target = None;
        self.hunt_spawn = None;
        self.nav.clear_route();
    }

    /// Where to hunt: the current or a much closer enemy, else an enemy
    /// spawn point (`true` in the second field).
    fn hunt_goal(&mut self, p: &Perception) -> Option<(Vec3, bool)> {
        let dist = |c: &Contact| c.pos.distance(p.pos);
        let enemies = || p.contacts.iter().filter(|c| c.team != p.team);
        let current = self
            .hunt_target
            .and_then(|id| enemies().find(|c| c.id == id));
        let nearest = enemies().min_by(|a, b| dist(a).total_cmp(&dist(b)));
        let pick = match (current, nearest) {
            (Some(c), Some(n)) if dist(n) < 0.6 * dist(c) => Some(n),
            (Some(c), _) => Some(c),
            (None, n) => n,
        };
        if let Some(e) = pick {
            self.hunt_target = Some(e.id);
            self.hunt_spawn = None;
            return Some((e.pos, false));
        }
        self.hunt_target = None;
        if self.hunt_spawn.is_none() {
            let theirs: Vec<Vec3> = self
                .spawns
                .iter()
                .filter(|(team, _)| *team != p.team)
                .map(|(_, q)| *q)
                .collect();
            if !theirs.is_empty() {
                self.hunt_spawn = Some(theirs[self.rng.below(theirs.len() as u32) as usize]);
            }
        }
        self.hunt_spawn.map(|q| (q, true))
    }

    fn pick_wander_point<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
    ) -> Option<Vec3> {
        // Now and then visit a pickup point (weapons, rockets, scrap).
        if !self.scrap_spots.is_empty() && self.rng.chance(0.3) {
            let i = self.rng.below(self.scrap_spots.len() as u32) as usize;
            return Some(self.scrap_spots[i]);
        }
        // Prefer points some distance away so bots cross the level.
        let mut fallback = None;
        if self.nav.is_ready() {
            for _ in 0..4 {
                let q = self.nav.grid().random_node(&mut self.rng)?;
                if q.distance(p.pos) > 25.0 {
                    return Some(q);
                }
                fallback = Some(q);
            }
            return fallback;
        }
        for _ in 0..4 {
            let q = random_open_point(terrain, &mut self.rng, self.bounds, 8.0, 4.0, 16)?;
            if q.distance(p.pos) > 25.0 {
                return Some(q);
            }
            fallback = Some(q);
        }
        fallback
    }

    fn pick_scrap_spot(&self, p: &Perception) -> Option<Vec3> {
        let usable = |s: &Vec3| {
            self.exhausted
                .iter()
                .all(|(e, _)| e.distance(*s) > self.tuning.scavenge_radius)
        };
        self.scrap_spots
            .iter()
            .chain(self.wrecks.iter().map(|(w, _)| w))
            .filter(|s| usable(s))
            .min_by(|a, b| {
                a.distance_squared(p.pos)
                    .total_cmp(&b.distance_squared(p.pos))
            })
            .copied()
    }

    fn plan_scavenge<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) {
        let Some(mut job) = self.scavenge else {
            return;
        };
        let reach = self.tuning.scavenge_radius * 0.6;
        let dist = job.spot.distance(p.pos);
        let mut give_up = false;
        if dist > reach {
            let cruise = self.tuning.cruise_speed;
            if self
                .plan_travel(p, terrain, job.spot, cruise, false, plan)
                .is_none()
            {
                plan.desired_vel = arrive(p.pos, job.spot, cruise, 8.0);
                plan.goal_dist = dist;
            }
            plan.aim = Some(job.spot);
            job.check_tick = None;
            give_up = p.tick > job.started + SCAVENGE_TRAVEL_TICKS;
        } else {
            // In reach: hold position (gentle correction) and scavenge.
            plan.desired_vel = (job.spot - p.pos) * 1.5;
            plan.goal_dist = dist;
            plan.scavenge = true;
            match job.check_tick {
                None => {
                    job.check_tick = Some(p.tick);
                    job.check_hull = p.hull;
                }
                Some(t) if p.tick >= t + SCAVENGE_CHECK_TICKS => {
                    if p.hull <= job.check_hull + 0.5 {
                        give_up = true; // nothing (left) here
                    }
                    job.check_tick = Some(p.tick);
                    job.check_hull = p.hull;
                }
                Some(_) => {}
            }
        }
        if give_up {
            self.exhausted.push((job.spot, p.tick + EXHAUSTED_TICKS));
            self.scavenge = None;
            // Try the next source on the next tick.
        } else {
            self.scavenge = Some(job);
        }
    }

    // ----- pickups -----------------------------------------------------------

    /// Whether the bot wants what `pickup` holds.
    fn wants_pickup(&self, p: &Perception, owned: &[(WeaponId, u32)], item: PickupItem) -> bool {
        match item {
            PickupItem::Weapon(w) => {
                let Some(g) = self.arsenal.gun(w) else {
                    return false;
                };
                match owned.iter().find(|(x, _)| *x == w) {
                    // Ammo for an empty count weapon.
                    Some((_, ammo)) => matches!(g.cost, WeaponCost::Count(_)) && *ammo == 0,
                    None => g.role != Role::PaintThrower,
                }
            }
            PickupItem::Upgrade(w) => owned.iter().any(|(x, _)| *x == w),
            PickupItem::Gadget(kind) => match p.gadget {
                None => kind != GadgetKind::Other,
                // Swap only for the gadget this bot likes.
                Some(current) => current != self.gadget_pref && kind == self.gadget_pref,
            },
        }
    }

    /// Fetches a wanted pickup within `range` u. False if there is none
    /// (or no way to it); a job further away than `range` (a fetch started
    /// out of combat) is kept for later.
    fn plan_collect<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
        range: f32,
    ) -> bool {
        // The job ends when the pickup is gone (collected by anyone).
        if let Some(job) = self.collect {
            let still_there = p
                .pickups
                .iter()
                .any(|q| q.id == job.id && q.item == job.item);
            let refused = job.reached.is_some_and(|t| p.tick > t + 45);
            let late =
                refused || p.tick > job.started + (self.cfg.collect_secs * TICKS_PER_SEC) as u64;
            if late {
                self.skipped_pickups
                    .push((job.id, p.tick + SKIP_PICKUP_TICKS));
            }
            if !still_there || late {
                self.collect = None;
                self.nav.clear_route();
            }
        }
        if self
            .collect
            .is_some_and(|job| job.pos.distance(p.pos) > range)
        {
            return false;
        }
        if self.collect.is_none() {
            let owned = self.owned(p);
            // The one Tectonic Charge on the map is worth a longer trip: it
            // counts as three times closer.
            let weight = |q: &PickupSeen| {
                let tectonic = matches!(q.item, PickupItem::Weapon(w)
                    if self.arsenal.gun(w).is_some_and(|g| g.role == Role::TectonicCharge));
                q.pos.distance(p.pos) / if tectonic { 3.0 } else { 1.0 }
            };
            self.collect = p
                .pickups
                .iter()
                .filter(|q| !self.skipped_pickups.iter().any(|(id, _)| *id == q.id))
                .filter(|q| weight(q) <= range)
                .filter(|q| self.wants_pickup(p, &owned, q.item))
                .min_by(|a, b| weight(a).total_cmp(&weight(b)))
                .map(|q| CollectJob {
                    id: q.id,
                    pos: q.pos,
                    item: q.item,
                    started: p.tick,
                    reached: None,
                });
        }
        let Some(mut job) = self.collect else {
            return false;
        };
        if job.reached.is_none() && job.pos.distance(p.pos) < 1.0 {
            job.reached = Some(p.tick);
            self.collect = Some(job);
        }
        // A new secondary swaps out the selected one when the slots are
        // full: select the least useful one first (empty, else the rocket).
        if let PickupItem::Weapon(w) = job.item {
            if self.arsenal.gun(w).is_some_and(|g| !g.is_primary()) {
                plan.select_secondary = self.secondary_to_drop(p);
            }
        }
        let cruise = self.tuning.cruise_speed;
        let dist = job.pos.distance(p.pos);
        if dist < 6.0
            || self
                .plan_travel(p, terrain, job.pos, cruise, true, plan)
                .is_none()
        {
            plan.desired_vel = arrive(p.pos, job.pos, cruise, 5.0);
            plan.goal_dist = dist;
            plan.clearance = Some(0.8);
        }
        true
    }

    /// The carried secondary to give up for a new one when every slot is
    /// taken, or `None` if a slot is free.
    fn secondary_to_drop(&self, p: &Perception) -> Option<WeaponId> {
        let owned = p.weapons.as_ref()?;
        let secondaries: Vec<(WeaponId, u32)> = owned
            .iter()
            .filter(|(w, _)| self.arsenal.gun(*w).is_some_and(|g| !g.is_primary()))
            .copied()
            .collect();
        if secondaries.len() < self.tuning.secondary_slots as usize {
            return None;
        }
        secondaries
            .iter()
            .min_by_key(|(w, ammo)| {
                let keep = match self.arsenal.gun(*w).map(|g| g.role) {
                    Some(Role::TectonicCharge) => 3,
                    Some(Role::BoltRocket) => 1,
                    _ => 2,
                };
                (*ammo > 0, keep)
            })
            .map(|(w, _)| *w)
    }

    // ----- paint -------------------------------------------------------------

    fn plan_paint<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) {
        let Some(paint) = self.arsenal.paint else {
            return;
        };
        if p.weapons
            .as_ref()
            .is_some_and(|w| !w.iter().any(|(id, _)| *id == paint.id))
        {
            return;
        }
        let range = self.cfg.paint_range.min(paint.range() * 0.9);
        if let Some(job) = self.paint {
            let dist = job.point.distance(p.pos);
            if p.tick >= job.until || dist > range + 6.0 {
                self.painted.push(job.point);
                if self.painted.len() > 32 {
                    self.painted.remove(0);
                }
                self.paint = None;
                self.next_paint_search = p.tick + 15;
            }
        }
        if self.paint.is_none() && p.tick >= self.next_paint_search {
            self.paint = self.find_wall(p, terrain, range);
            if self.paint.is_none() {
                self.next_paint_search = p.tick + 20;
            } else if self.cfg.paint_colour_button.is_some() && self.rng.chance(0.4) {
                // A new spot: now and then switch the effect colour.
                self.colour_presses = 1 + self.rng.below(2) as u8;
            }
        }
        let Some(job) = self.paint else {
            return;
        };
        plan.select = Some(paint.id);
        // Sweep across the wall to cover more than one splat.
        let phase = (p.tick - job.started) as f32 * DT * 4.0;
        let aim = job.point + job.sweep * (phase.sin() * 1.5);
        plan.aim = Some(aim);
        let err = angle_between(p.orientation.forward(), aim - p.pos);
        if p.primary == Some(paint.id)
            && err < 0.08
            && self.can_pay(p, &paint, 0, self.cfg.paint_energy_reserve)
            && clear_shot(terrain, p.pos, aim, paint.projectile_radius)
        {
            plan.fire_primary = true;
        }
    }

    /// Looks for a wall point to paint: a few raycasts, mostly ahead.
    fn find_wall<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        range: f32,
    ) -> Option<PaintJob> {
        let fwd = p.orientation.forward();
        for i in 0..8 {
            let r = random_unit(&mut self.rng);
            let dir = if i < 5 {
                (fwd * 1.2 + r).normalize_or_zero()
            } else {
                r
            };
            let Some(hit) = terrain.raycast(p.pos, dir, range) else {
                continue;
            };
            if hit.distance < 4.0 || self.painted.iter().any(|q| q.distance(hit.point) < 4.0) {
                continue;
            }
            let sweep = hit.normal.any_orthonormal();
            let secs = self.cfg.paint_secs;
            return Some(PaintJob {
                point: hit.point,
                sweep,
                started: p.tick,
                until: p.tick + (secs * TICKS_PER_SEC) as u64,
            });
        }
        None
    }

    // ----- command -------------------------------------------------------------

    fn write_command<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &Plan,
        cmd: &mut InputCmd,
        counters: &mut BrainCounters,
    ) {
        let t = &self.tuning;
        let mut steer_cfg = self.cfg.steer;
        if let Some(c) = plan.clearance {
            // On a route: the way to the waypoint is known to be clear, so
            // brake for rock along the motion later than when roaming.
            steer_cfg.clearance = c;
            steer_cfg.brake_margin = steer_cfg.brake_margin.min(1.0);
        }
        let steer = avoid_walls(
            terrain,
            p.pos,
            p.vel,
            plan.desired_vel,
            plan.goal_dist,
            &steer_cfg,
        );
        let want = plan.desired_vel.length();
        let vel = if want > 1e-3 {
            steer.dir * (want * steer.speed_scale)
        } else {
            // Hovering: only back away from walls that are too close.
            steer.dir * (0.3 * t.cruise_speed)
        };
        let boosting = plan.boost && steer.speed_scale >= 0.99 && p.boost > 0.0;
        let cap = if boosting {
            t.boost_speed
        } else {
            t.cruise_speed
        };
        cmd.thrust = thrust_for_velocity(p.orientation, vel, cap);
        if boosting && cmd.thrust != Vec3::ZERO {
            cmd.buttons = cmd.buttons | Buttons::BOOST;
        }

        // Look at the aim point, or along the flight path.
        let look_dir = match plan.aim {
            Some(a) => a - p.pos,
            None if vel.length() > 1.0 => vel,
            None => p.orientation.forward(),
        };
        let look = look_toward(p.orientation, look_dir, t.max_turn_rate * DT);
        cmd.look_pitch = look.pitch;
        cmd.look_yaw = look.yaw;

        // One selection per tick: a secondary about to be fired first, then
        // the primary, then any other secondary change.
        let primary_change = plan.select.filter(|w| p.primary != Some(*w));
        let secondary_change = plan.select_secondary.filter(|w| p.secondary != Some(*w));
        let select = match (primary_change, secondary_change) {
            (_, Some(s)) if self.secondary_intent.is_some() => Some(s),
            (Some(w), Some(s)) => Some(if p.tick.is_multiple_of(2) { w } else { s }),
            (Some(w), None) => Some(w),
            (None, s) => s,
        };
        if let Some(w) = select {
            cmd.weapon_select = w.0 + 1;
        }
        if plan.fire_primary {
            cmd.buttons = cmd.buttons | Buttons::FIRE_PRIMARY;
            if let Some(id) = p.primary {
                counters.trigger(id);
                self.note_used(id);
                let is = |g: Option<Gun>| g.is_some_and(|g| g.id == id);
                if is(self.arsenal.laser) {
                    counters.laser_triggers += 1;
                } else if is(self.arsenal.rivet) {
                    counters.rivet_triggers += 1;
                } else if is(self.arsenal.paint) {
                    counters.paint_triggers += 1;
                }
            }
        }
        if plan.fire_secondary {
            cmd.buttons = cmd.buttons | Buttons::FIRE_SECONDARY;
            counters.secondary_triggers += 1;
            if let Some(id) = p.secondary.or(self.arsenal.rocket.map(|g| g.id)) {
                counters.trigger(id);
                self.note_used(id);
                if self.arsenal.rocket.is_some_and(|g| g.id == id) {
                    counters.rocket_triggers += 1;
                }
            }
        }
        if plan.scavenge {
            cmd.buttons = cmd.buttons | Buttons::SCAVENGE;
            counters.scavenge_holds += 1;
        }
        if plan.gadget {
            cmd.buttons = cmd.buttons | Buttons::GADGET;
            counters.gadget_presses += 1;
        }
        // Paint colour: one press per few ticks while presses are pending,
        // only with the Paint Thrower selected (the button is the selected
        // primary's weapon mode).
        let painting = self.arsenal.paint.is_some_and(|g| p.primary == Some(g.id));
        if let Some(b) = self.cfg.paint_colour_button {
            if painting && self.colour_presses > 0 && p.tick >= self.next_colour_press {
                cmd.buttons = cmd.buttons | b;
                self.colour_presses -= 1;
                self.next_colour_press = p.tick + 12;
                counters.colour_presses += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests;
