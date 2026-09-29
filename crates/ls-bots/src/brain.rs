//! Bot behavior: turns what the bot sees ([`Perception`]) into one
//! [`InputCmd`] per tick.
//!
//! Priorities, evaluated every tick:
//! 1. **Scavenge** when hull is low: fly to the nearest known scrap source
//!    (pickup points and recent wrecks), hold still and hold scavenge until
//!    the hull is repaired or the source runs dry.
//! 2. **Combat** when an enemy is in range with line of sight: orbit at a
//!    preferred distance, aim with lead for the equipped weapon's projectile
//!    speed, fire the primary (Scrap Laser, or the Rivet Gun once picked up)
//!    when the nose is on the lead point, and a Bolt Rocket now and then.
//! 3. **Paint and wander** otherwise: fly between random open-space points
//!    (sometimes pickup points) and paint nearby walls with the Paint
//!    Thrower.
//!
//! Every movement request goes through [`avoid_walls`], so bots stay clear
//! of rock whatever the plan says.

use ls_game::client::ClientSession;
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Aabb, Pcg32, Quat, Vec3, DT, TICK_HZ};
use ls_game::ls_net::ship_flags;
use ls_game::ls_sim::{Buttons, InputCmd, ShipTuning, WeaponCost, WeaponId, WeaponTable};

use crate::aim::{aim_tolerance, clear_shot, lead_point};
use crate::steering::{
    angle_between, arrive, avoid_walls, look_toward, random_open_point, random_unit,
    thrust_for_velocity, SteerConfig,
};

/// Another ship the bot can see (alive only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    /// Wire id.
    pub id: u8,
    pub team: u8,
    pub pos: Vec3,
    pub vel: Vec3,
}

/// Everything the brain looks at for one decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Perception {
    /// Tick the command will be stamped with (monotonic).
    pub tick: u64,
    pub pos: Vec3,
    pub vel: Vec3,
    pub orientation: Quat,
    pub alive: bool,
    pub team: u8,
    /// Resources in absolute units (hull points, energy, boost, scrap).
    pub hull: f32,
    pub energy: f32,
    pub boost: f32,
    pub scrap: f32,
    /// Selected primary weapon according to the newest snapshot.
    pub primary: Option<WeaponId>,
    /// Living ships other than this one.
    pub contacts: Vec<Contact>,
}

impl Perception {
    /// Reads the predicted local ship, the newest own status and the
    /// interpolated remote ships from a session. `None` before the first
    /// snapshot anchored the prediction.
    pub fn from_session(s: &ClientSession) -> Option<Perception> {
        let ship = s.local_ship()?;
        let own = s
            .latest_snapshot()
            .and_then(|snap| snap.ships.iter().find(|n| n.id == s.player()));
        let (hull, energy, _boost, scrap) = own
            .and_then(|n| n.status)
            .map(|st| s.conv().status_values(&st))
            .unwrap_or((ship.hull, ship.energy, ship.boost, ship.scrap));
        let contacts = s
            .remote_ships()
            .iter()
            .filter(|n| n.flags & ship_flags::ALIVE != 0)
            .map(|n| Contact {
                id: n.id,
                team: ship_flags::team(n.flags),
                pos: n.pos,
                vel: n.vel,
            })
            .collect();
        Some(Perception {
            tick: s.next_input_tick(),
            pos: ship.pos,
            vel: ship.vel,
            orientation: ship.orientation,
            alive: ship.alive,
            team: s.team(),
            hull,
            energy,
            // The prediction tracks boost exactly; the status byte lags.
            boost: ship.boost,
            scrap,
            primary: own.map(|n| WeaponId(ship_flags::weapon(n.flags))),
            contacts,
        })
    }
}

/// What the brain needs to know about one weapon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gun {
    pub id: WeaponId,
    /// Projectile speed (u/s).
    pub speed: f32,
    /// Projectile lifetime (s).
    pub lifetime: f32,
    pub projectile_radius: f32,
    pub cost: WeaponCost,
}

impl Gun {
    /// Longest distance the projectile flies.
    pub fn range(&self) -> f32 {
        self.speed * self.lifetime
    }
}

/// The M1 weapons by role. Any of them may be missing from a data set.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Arsenal {
    pub laser: Option<Gun>,
    pub rivet: Option<Gun>,
    pub rocket: Option<Gun>,
    pub paint: Option<Gun>,
}

impl Arsenal {
    pub fn from_table(table: &WeaponTable) -> Arsenal {
        let gun = |name: &str| {
            table.find(name).map(|id| {
                let d = table.get(id);
                Gun {
                    id,
                    speed: d.speed,
                    lifetime: d.lifetime_secs,
                    projectile_radius: d.projectile_radius,
                    cost: d.cost,
                }
            })
        };
        Arsenal {
            laser: gun("scrap_laser"),
            rivet: gun("rivet_gun"),
            rocket: gun("bolt_rocket"),
            paint: gun("paint_thrower"),
        }
    }

    /// The gun with this id, if it is one of the four.
    pub fn gun(&self, id: WeaponId) -> Option<Gun> {
        [self.laser, self.rivet, self.rocket, self.paint]
            .into_iter()
            .flatten()
            .find(|g| g.id == id)
    }

    fn is_combat_primary(&self, id: WeaponId) -> bool {
        self.laser.is_some_and(|g| g.id == id) || self.rivet.is_some_and(|g| g.id == id)
    }
}

/// Behavior tuning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrainConfig {
    pub steer: SteerConfig,
    /// Enemies further than this (u) are ignored.
    pub engage_range: f32,
    /// Distance (u) the bot tries to keep from its target.
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
}

impl Default for BrainConfig {
    fn default() -> Self {
        BrainConfig {
            steer: SteerConfig::default(),
            engage_range: 55.0,
            preferred_range: 16.0,
            low_hull: 0.35,
            healed_hull: 0.85,
            paint_range: 24.0,
            paint_secs: 1.5,
            paint_energy_reserve: 20.0,
            rocket_range: (7.0, 35.0),
            rocket_interval_secs: 1.5,
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
    Scavenge,
}

/// Per-bot behavior counters (ticks spent per mode, trigger ticks per
/// weapon; a trigger tick is a tick the button was held with that weapon
/// equipped, not necessarily a shot).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BrainCounters {
    pub wander_ticks: u64,
    pub paint_ticks: u64,
    pub combat_ticks: u64,
    pub scavenge_ticks: u64,
    pub dead_ticks: u64,
    pub laser_triggers: u64,
    pub rivet_triggers: u64,
    pub rocket_triggers: u64,
    pub paint_triggers: u64,
    /// Ticks the scavenge button was held.
    pub scavenge_holds: u64,
}

impl BrainCounters {
    pub fn add(&mut self, o: &BrainCounters) {
        self.wander_ticks += o.wander_ticks;
        self.paint_ticks += o.paint_ticks;
        self.combat_ticks += o.combat_ticks;
        self.scavenge_ticks += o.scavenge_ticks;
        self.dead_ticks += o.dead_ticks;
        self.laser_triggers += o.laser_triggers;
        self.rivet_triggers += o.rivet_triggers;
        self.rocket_triggers += o.rocket_triggers;
        self.paint_triggers += o.paint_triggers;
        self.scavenge_holds += o.scavenge_holds;
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

/// Rivet Gun ownership is not in the snapshot; the bot finds out by trying
/// to select it and watching the primary weapon bits.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct RivetProbe {
    owned: bool,
    probing_until: u64,
    next_probe: u64,
}

/// The per-bot decision maker. See the module docs.
#[derive(Debug, Clone)]
pub struct Brain {
    cfg: BrainConfig,
    arsenal: Arsenal,
    tuning: ShipTuning,
    bounds: Aabb,
    rng: Pcg32,
    mode: Mode,
    was_alive: bool,

    target: Option<u8>,
    next_target_scan: u64,
    strafe_sign: f32,
    next_strafe_flip: u64,
    next_rocket: u64,
    rivet: RivetProbe,

    wander: Option<Vec3>,
    wander_best: f32,
    wander_progress_tick: u64,

    paint: Option<PaintJob>,
    next_paint_search: u64,
    painted: Vec<Vec3>,

    scrap_spots: Vec<Vec3>,
    wrecks: Vec<(Vec3, u64)>,
    exhausted: Vec<(Vec3, u64)>,
    scavenge: Option<ScavengeJob>,
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

#[derive(Debug, Clone, Copy, PartialEq)]
struct Plan {
    desired_vel: Vec3,
    /// Distance to the movement goal along `desired_vel` (infinite if none).
    goal_dist: f32,
    boost: bool,
    aim: Option<Vec3>,
    select: Option<WeaponId>,
    fire_primary: bool,
    fire_secondary: bool,
    scavenge: bool,
}

impl Plan {
    fn new() -> Plan {
        Plan {
            desired_vel: Vec3::ZERO,
            goal_dist: f32::INFINITY,
            boost: false,
            aim: None,
            select: None,
            fire_primary: false,
            fire_secondary: false,
            scavenge: false,
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
        Brain {
            cfg,
            arsenal,
            tuning,
            bounds,
            rng,
            mode: Mode::Dead,
            was_alive: false,
            target: None,
            next_target_scan: 0,
            strafe_sign,
            next_strafe_flip: 0,
            next_rocket: 0,
            rivet: RivetProbe::default(),
            wander: None,
            wander_best: f32::MAX,
            wander_progress_tick: 0,
            paint: None,
            next_paint_search: 0,
            painted: Vec::new(),
            scrap_spots,
            wrecks: Vec::new(),
            exhausted: Vec::new(),
            scavenge: None,
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

    /// Whether the bot knows it carries the Rivet Gun.
    pub fn has_rivet_gun(&self) -> bool {
        self.rivet.owned
    }

    /// Sets the known scrap sources besides wrecks (the match's pickup points).
    pub fn set_scrap_spots(&mut self, spots: Vec<Vec3>) {
        self.scrap_spots = spots;
    }

    /// Known pickup points.
    pub fn scrap_spots(&self) -> &[Vec3] {
        &self.scrap_spots
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
        if !p.alive {
            self.on_death();
            counters.dead_ticks += 1;
            return cmd;
        }
        if !self.was_alive {
            self.on_spawn(p.tick);
        }
        self.was_alive = true;
        self.expire(p.tick);
        if let (Some(r), Some(primary)) = (self.arsenal.rivet, p.primary) {
            if r.id == primary {
                self.rivet.owned = true;
            }
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

        self.mode = if self.scavenge.is_some() {
            self.plan_scavenge(p, &mut plan);
            Mode::Scavenge
        } else if let Some(t) = target {
            self.plan_orbit(p, &t, &mut plan);
            Mode::Combat
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
        } else if self.mode != Mode::Scavenge {
            self.plan_paint(p, terrain, &mut plan);
            if self.paint.is_some() {
                self.mode = Mode::Paint;
            }
        }

        match self.mode {
            Mode::Wander => counters.wander_ticks += 1,
            Mode::Paint => counters.paint_ticks += 1,
            Mode::Combat => counters.combat_ticks += 1,
            Mode::Scavenge => counters.scavenge_ticks += 1,
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
        // Respawning resets the loadout to the rule set's starting weapons.
        self.rivet = RivetProbe::default();
    }

    fn on_spawn(&mut self, now: u64) {
        self.wander = None;
        self.next_paint_search = now + 30;
        self.next_rocket = now + 60;
        self.rivet.next_probe = now + 120;
    }

    fn expire(&mut self, now: u64) {
        self.wrecks.retain(|(_, until)| *until > now);
        self.exhausted.retain(|(_, until)| *until > now);
    }

    // ----- targets -----------------------------------------------------------

    fn update_target<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
    ) -> Option<Contact> {
        let enemy = |c: &&Contact| c.team != p.team;
        let current = self
            .target
            .and_then(|id| p.contacts.iter().find(|c| c.id == id))
            .filter(enemy)
            .copied();
        if p.tick < self.next_target_scan {
            // Between scans keep the target while it stays roughly in range.
            let keep = current.filter(|c| c.pos.distance(p.pos) < self.cfg.engage_range * 1.2);
            self.target = keep.map(|c| c.id);
            return keep;
        }
        self.next_target_scan = p.tick + 6;
        let mut best: Option<(f32, Contact)> = None;
        for c in p.contacts.iter().filter(enemy) {
            let mut d = c.pos.distance(p.pos);
            if d > self.cfg.engage_range {
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

    // ----- movement plans ----------------------------------------------------

    fn plan_orbit(&mut self, p: &Perception, t: &Contact, plan: &mut Plan) {
        let cruise = self.tuning.cruise_speed;
        let to = t.pos - p.pos;
        let dist = to.length().max(1e-3);
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
        let radial_speed = ((dist - self.cfg.preferred_range) / 6.0).clamp(-1.0, 1.0) * cruise;
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
        plan.boost = boost_ok;
        plan.desired_vel = arrive(p.pos, goal, speed, 8.0);
        plan.goal_dist = dist;
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

    fn plan_scavenge(&mut self, p: &Perception, plan: &mut Plan) {
        let Some(mut job) = self.scavenge else {
            return;
        };
        let reach = self.tuning.scavenge_radius * 0.6;
        let dist = job.spot.distance(p.pos);
        let mut give_up = false;
        if dist > reach {
            plan.desired_vel = arrive(p.pos, job.spot, self.tuning.cruise_speed, 8.0);
            plan.goal_dist = dist;
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

    // ----- weapons -----------------------------------------------------------

    fn has_energy(&self, p: &Perception, gun: &Gun, reserve: f32) -> bool {
        match gun.cost {
            WeaponCost::Energy(c) => p.energy >= c + reserve,
            WeaponCost::Scrap(c) => p.scrap >= c,
            WeaponCost::Count(_) => true,
        }
    }

    fn plan_fire<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        t: &Contact,
        terrain: &T,
        plan: &mut Plan,
    ) {
        let dist = t.pos.distance(p.pos);
        // Rivet Gun ownership probe.
        if self.arsenal.rivet.is_some() && !self.rivet.owned && p.tick >= self.rivet.next_probe {
            self.rivet.probing_until = p.tick + 30;
            self.rivet.next_probe = p.tick + 10 * TICK_HZ as u64;
        }
        let probing = p.tick < self.rivet.probing_until && !self.rivet.owned;

        // Weapon choice: Rivet Gun up close when owned and fed, else laser.
        let wanted = match (self.arsenal.rivet, self.arsenal.laser) {
            (Some(r), _) if (self.rivet.owned && dist < 30.0 && p.scrap >= 10.0) || probing => {
                Some(r)
            }
            (_, Some(l)) => Some(l),
            (Some(r), None) => Some(r),
            (None, None) => None,
        };
        plan.select = wanted.map(|g| g.id);

        // Aim for the weapon actually equipped if it is a combat primary
        // (switching takes a round trip), otherwise for the one wanted.
        let equipped = p
            .primary
            .filter(|w| self.arsenal.is_combat_primary(*w))
            .and_then(|w| self.arsenal.gun(w));
        let gun = equipped.or(wanted);
        let hit_radius = self.tuning.radius;
        if let Some(gun) = gun {
            let aim = lead_point(p.pos, t.pos, t.vel, gun.speed, gun.lifetime).unwrap_or(t.pos);
            plan.aim = Some(aim);
            let err = angle_between(p.orientation.forward(), aim - p.pos);
            let tol = aim_tolerance(dist, hit_radius + gun.projectile_radius);
            if equipped.is_some()
                && dist <= gun.range()
                && err <= tol
                && self.has_energy(p, &gun, 0.0)
                && clear_shot(terrain, p.pos, aim, gun.projectile_radius)
            {
                plan.fire_primary = true;
            }
        }

        // Bolt Rocket: medium range, tight aim, not too often.
        if let Some(rocket) = self.arsenal.rocket {
            let (lo, hi) = self.cfg.rocket_range;
            if p.tick >= self.next_rocket && dist >= lo && dist <= hi.min(rocket.range()) {
                if let Some(aim) = lead_point(p.pos, t.pos, t.vel, rocket.speed, rocket.lifetime) {
                    let err = angle_between(p.orientation.forward(), aim - p.pos);
                    let tol = aim_tolerance(dist, hit_radius + 1.0);
                    if err <= tol && clear_shot(terrain, p.pos, aim, rocket.projectile_radius) {
                        plan.fire_secondary = true;
                        let jitter = self.rng.below(TICK_HZ) as u64;
                        self.next_rocket = p.tick
                            + (self.cfg.rocket_interval_secs * TICKS_PER_SEC) as u64
                            + jitter;
                    }
                }
            }
        }
    }

    fn plan_paint<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) {
        let Some(paint) = self.arsenal.paint else {
            return;
        };
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
            && self.has_energy(p, &paint, self.cfg.paint_energy_reserve)
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
        let steer = avoid_walls(
            terrain,
            p.pos,
            p.vel,
            plan.desired_vel,
            plan.goal_dist,
            &self.cfg.steer,
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

        if let Some(w) = plan.select {
            if p.primary != Some(w) {
                cmd.weapon_select = w.0 + 1;
            }
        }
        if plan.fire_primary {
            cmd.buttons = cmd.buttons | Buttons::FIRE_PRIMARY;
            let id = p.primary;
            let is = |g: Option<Gun>| g.is_some() && g.map(|g| g.id) == id;
            if is(self.arsenal.laser) {
                counters.laser_triggers += 1;
            } else if is(self.arsenal.rivet) {
                counters.rivet_triggers += 1;
            } else if is(self.arsenal.paint) {
                counters.paint_triggers += 1;
            }
        }
        if plan.fire_secondary {
            cmd.buttons = cmd.buttons | Buttons::FIRE_SECONDARY;
            counters.rocket_triggers += 1;
        }
        if plan.scavenge {
            cmd.buttons = cmd.buttons | Buttons::SCAVENGE;
            counters.scavenge_holds += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::ls_math::terrain::BoxRoomTerrain;
    use ls_game::ls_sim::{simulate_movement, PlayerId, ShipState, ShipType};

    fn table() -> WeaponTable {
        WeaponTable::builtin()
    }

    fn room() -> BoxRoomTerrain {
        BoxRoomTerrain::centered(Vec3::new(80.0, 40.0, 80.0))
    }

    fn brain(terrain: &BoxRoomTerrain, spots: Vec<Vec3>) -> Brain {
        Brain::new(
            BrainConfig::default(),
            Arsenal::from_table(&table()),
            ShipTuning::tinker(),
            terrain.bounds(),
            spots,
            7,
        )
    }

    fn perception(pos: Vec3) -> Perception {
        let t = ShipTuning::tinker();
        Perception {
            tick: 1000,
            pos,
            vel: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            alive: true,
            team: 0,
            hull: t.hull_max,
            energy: t.energy_max,
            boost: t.boost_max,
            scrap: t.scrap_max,
            primary: table().find("scrap_laser"),
            contacts: Vec::new(),
        }
    }

    fn ship(p: &Perception) -> ShipState {
        let mut s = ShipState::new(
            PlayerId(0),
            p.team,
            ShipType::Tinker,
            &ShipTuning::tinker(),
            p.pos,
            p.orientation,
        );
        s.vel = p.vel;
        s
    }

    #[test]
    fn arsenal_finds_the_m1_weapons() {
        let a = Arsenal::from_table(&table());
        assert_eq!(a.laser.unwrap().speed, 30.0);
        assert_eq!(a.rivet.unwrap().speed, 180.0);
        assert_eq!(a.rocket.unwrap().speed, 40.0);
        assert_eq!(a.paint.unwrap().speed, 20.0);
        assert_eq!(a.gun(a.paint.unwrap().id), a.paint);
        assert!((a.paint.unwrap().range() - 30.0).abs() < 1e-4);
    }

    #[test]
    fn dead_bot_idles() {
        let t = room();
        let mut b = brain(&t, vec![]);
        let mut p = perception(Vec3::ZERO);
        p.alive = false;
        let mut c = BrainCounters::default();
        let cmd = b.think(&p, &t, &mut c);
        assert_eq!(cmd.thrust, Vec3::ZERO);
        assert!(!cmd.buttons.contains(Buttons::FIRE_PRIMARY));
        assert_eq!(b.mode(), Mode::Dead);
        assert_eq!(c.dead_ticks, 1);
    }

    #[test]
    fn aims_with_lead_and_fires_when_on_target() {
        let t = room();
        let mut b = brain(&t, vec![]);
        let mut p = perception(Vec3::ZERO);
        let enemy = Contact {
            id: 3,
            team: 1,
            pos: Vec3::new(0.0, 0.0, -20.0),
            vel: Vec3::new(10.0, 0.0, 0.0),
        };
        p.contacts = vec![
            enemy,
            // A team mate closer by is never a target.
            Contact {
                id: 4,
                team: 0,
                pos: Vec3::new(0.0, 0.0, -5.0),
                vel: Vec3::ZERO,
            },
        ];
        let laser = b.arsenal().laser.unwrap();
        let lead = lead_point(p.pos, enemy.pos, enemy.vel, laser.speed, laser.lifetime).unwrap();
        let mut c = BrainCounters::default();

        // Nose straight at the target's current position: must turn towards
        // the lead (to +X means negative yaw), not fire yet.
        let cmd = b.think(&p, &t, &mut c);
        assert_eq!(b.mode(), Mode::Combat);
        assert_eq!(b.target(), Some(3));
        assert!(cmd.look_yaw < 0.0, "{cmd:?}");
        assert!(!cmd.buttons.contains(Buttons::FIRE_PRIMARY));

        // Nose on the lead point: fire.
        p.tick += 1;
        p.orientation = Quat::from_rotation_arc(Vec3::NEG_Z, lead.normalize());
        let cmd = b.think(&p, &t, &mut c);
        assert!(cmd.buttons.contains(Buttons::FIRE_PRIMARY), "{cmd:?}");
        assert_eq!(c.laser_triggers, 1);
        // In rocket range with a tight aim at the rocket's lead? The rocket
        // is faster, so its lead differs and it holds fire.
        assert!(!cmd.buttons.contains(Buttons::FIRE_SECONDARY) || c.rocket_triggers == 1);
    }

    #[test]
    fn does_not_fire_without_energy_or_line_of_sight() {
        let mut t = room();
        let mut b = brain(&t, vec![]);
        let mut p = perception(Vec3::ZERO);
        p.contacts = vec![Contact {
            id: 1,
            team: 1,
            pos: Vec3::new(0.0, 0.0, -20.0),
            vel: Vec3::ZERO,
        }];
        p.energy = 1.0;
        let mut c = BrainCounters::default();
        let cmd = b.think(&p, &t, &mut c);
        assert!(!cmd.buttons.contains(Buttons::FIRE_PRIMARY));

        // Rock between the two ships: no target at all.
        t.apply(ls_game::ls_math::TerrainEdit::AddSphere {
            center: Vec3::new(0.0, 0.0, -10.0),
            radius: 3.0,
        });
        let mut b = brain(&t, vec![]);
        p.energy = 100.0;
        b.think(&p, &t, &mut c);
        assert_eq!(b.target(), None);
        assert_ne!(b.mode(), Mode::Combat);
    }

    #[test]
    fn selects_the_rivet_gun_once_owned() {
        let t = room();
        let mut b = brain(&t, vec![]);
        let rivet = b.arsenal().rivet.unwrap().id;
        let mut p = perception(Vec3::ZERO);
        p.contacts = vec![Contact {
            id: 1,
            team: 1,
            pos: Vec3::new(0.0, 0.0, -15.0),
            vel: Vec3::ZERO,
        }];
        let mut c = BrainCounters::default();
        // The snapshot shows the Rivet Gun equipped: owned from now on.
        p.primary = Some(rivet);
        let cmd = b.think(&p, &t, &mut c);
        assert!(b.has_rivet_gun());
        assert!(cmd.buttons.contains(Buttons::FIRE_PRIMARY), "{cmd:?}");
        assert_eq!(c.rivet_triggers, 1);
        // Death resets the loadout.
        p.alive = false;
        b.think(&p, &t, &mut c);
        assert!(!b.has_rivet_gun());
    }

    #[test]
    fn paints_a_nearby_wall() {
        let t = room();
        let mut b = brain(&t, vec![]);
        let paint = b.arsenal().paint.unwrap().id;
        // 10 u in front of the -Z wall, looking at it.
        let mut p = perception(Vec3::new(0.0, 0.0, -30.0));
        let mut c = BrainCounters::default();
        let mut s = ship(&p);
        let mut selected = false;
        for _ in 0..240 {
            let cmd = b.think(&p, &t, &mut c);
            if cmd.weapon_select == paint.0 + 1 {
                selected = true;
                // The server would switch the weapon.
                p.primary = Some(paint);
            }
            simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), &t);
            p.pos = s.pos;
            p.vel = s.vel;
            p.orientation = s.orientation;
            p.tick += 1;
        }
        assert!(selected, "never selected the Paint Thrower");
        assert!(c.paint_triggers > 20, "{c:?}");
        assert!(c.paint_ticks > 0);
    }

    #[test]
    fn scavenges_when_hull_is_low() {
        let t = room();
        let spot = Vec3::new(20.0, 0.0, 0.0);
        let mut b = brain(&t, vec![spot, Vec3::new(-30.0, 10.0, 30.0)]);
        let mut p = perception(Vec3::new(-10.0, 0.0, 0.0));
        p.hull = 20.0;
        let mut c = BrainCounters::default();
        let mut s = ship(&p);
        let mut held = false;
        for _ in 0..60 * 6 {
            let cmd = b.think(&p, &t, &mut c);
            assert_eq!(b.mode(), Mode::Scavenge);
            if cmd.buttons.contains(Buttons::SCAVENGE) {
                held = true;
                assert!(p.pos.distance(spot) <= ShipTuning::tinker().scavenge_radius);
                p.hull += 0.3; // the server repairs while in reach
            }
            simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), &t);
            p.pos = s.pos;
            p.vel = s.vel;
            p.orientation = s.orientation;
            p.tick += 1;
        }
        assert!(held, "never held scavenge; ended at {:?}", p.pos);
        // Healed past the threshold: back to normal.
        p.hull = 90.0;
        b.think(&p, &t, &mut c);
        assert_ne!(b.mode(), Mode::Scavenge);
    }

    #[test]
    fn dry_scrap_spot_is_abandoned() {
        let t = room();
        let a = Vec3::new(5.0, 0.0, 0.0);
        let far = Vec3::new(-30.0, 10.0, 30.0);
        let mut b = brain(&t, vec![a, far]);
        let mut p = perception(a);
        p.hull = 20.0;
        let mut c = BrainCounters::default();
        for _ in 0..(SCAVENGE_CHECK_TICKS + 5) {
            b.think(&p, &t, &mut c);
            p.tick += 1;
        }
        // No hull gained at `a`: the bot moves on to the other spot.
        assert_eq!(b.scavenge.map(|j| j.spot), Some(far));
        // Wrecks are sources too.
        b.on_wreck(Vec3::new(4.0, 1.0, 0.0), p.tick);
        assert!(b.wrecks.len() == 1);
    }

    #[test]
    fn wanders_without_hitting_walls() {
        let mut t = room();
        t.apply(ls_game::ls_math::TerrainEdit::AddSphere {
            center: Vec3::new(10.0, 0.0, 10.0),
            radius: 8.0,
        });
        let mut b = brain(&t, vec![]);
        let mut p = perception(Vec3::new(-20.0, 0.0, -20.0));
        let mut s = ship(&p);
        let mut c = BrainCounters::default();
        let mut contacts = 0;
        let start = p.pos;
        let mut max_travel: f32 = 0.0;
        for _ in 0..60 * 30 {
            let cmd = b.think(&p, &t, &mut c);
            let r = simulate_movement(&mut s, &cmd, &ShipTuning::tinker(), &t);
            if r.contact {
                contacts += 1;
            }
            p.pos = s.pos;
            p.vel = s.vel;
            p.orientation = s.orientation;
            p.boost = s.boost;
            p.energy = s.energy;
            p.tick += 1;
            max_travel = max_travel.max(p.pos.distance(start));
        }
        assert!(contacts < 10, "{contacts} ticks in contact with rock");
        assert!(max_travel > 20.0, "bot did not move: {max_travel}");
        assert!(c.wander_ticks + c.paint_ticks > 0);
    }
}
