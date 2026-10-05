//! Client-side prediction of the local player's shots (online play).
//!
//! Without it a shot appears only once the server's snapshot carries the
//! projectile, a full round trip after the click. [`ShotPredictor`] runs the
//! same firing rules as `ls_sim` (`World::step` / `try_fire`: weapon
//! selection, rule-set bans, fire interval, energy / scrap / count cost,
//! overdrive, hold-to-charge) on the predicted local ship for every
//! predicted tick and spawns visual projectiles at once, with the
//! simulation's muzzle offset, directions and speed.
//!
//! **Shot models.** Each weapon is predicted the way its module fires
//! ([`ShotModel`]): the generic pattern (one bolt, a fan of up to 3, or a
//! centre bolt plus a ring), the Scatter Laser's fan that alternates between
//! horizontal and vertical per shot, the Corkscrew's line of bolts spinning
//! around the aim axis (a conical helix per bolt, rotated per shot), and no
//! projectile at all for the instant Leech Arc (its beam cue is the shot)
//! and for shots a straight bolt would draw wrongly (Borrowed Time's
//! stationary pulse, the Prox Mine dropped behind). Charge weapons (Fusion
//! Charger, Polite Laser) fire on release, or by themselves at full charge
//! when the data says `auto_release`, like `World::release`; the charge in
//! progress is exposed for the HUD's charge meter ([`ShotPredictor::charge`]).
//!
//! **Shot phase.** Alternating and rotating patterns depend on how many
//! shots the weapon fired before (`ls_sim::WeaponState::shots`). The server
//! sends that count with the inventory (`ls_net::NetWeapon::shots`, protocol
//! 5) whenever it changes; the predictor counts its own shots per weapon on
//! top of it and, when an inventory shows another count, rebuilds the
//! volleys it predicted after the inventory's tick with the server's phase
//! ([`ShotPredictor::on_inventory`]).
//!
//! Resources come from the newest snapshot's own-ship status minus the cost
//! of predicted shots after that snapshot (regeneration since then is
//! ignored, so prediction errs towards not firing near empty).
//!
//! **Reconciliation.** When a snapshot shows a new projectile of the local
//! player, it is matched with the predicted shot of the same weapon whose
//! position is nearest (within about 1.5 ticks of travel). The predicted
//! shot is removed and the authoritative one takes over; the host draws the
//! player's own projectiles in the local (predicted) timeline, so a correct
//! prediction continues without a jump and the small difference of a wrong
//! one is kept as a render offset that fades out. A predicted shot that no
//! snapshot confirmed [`CONFIRM_GRACE_TICKS`] after its tick (a refused
//! shot, or one that hit something between two snapshots) is removed.
//!
//! Nothing here is deterministic game state: it only affects what is drawn.

use std::collections::{BTreeMap, BTreeSet};

use ls_math::terrain::TerrainQuery;
use ls_math::{det, Quat, Tick, Vec3};
use ls_net::NetProjectile;
use ls_sim::{
    Buttons, InputCmd, ResolvedRules, ShipState, ShipTuning, WeaponCost, WeaponDef, WeaponId,
    WeaponSlot, WeaponTable,
};

/// Ticks per second as `f32`.
const HZ: f32 = ls_math::TICK_HZ as f32;
/// A predicted shot not matched by a snapshot this many ticks newer than
/// the shot is dropped (snapshots come every 2 ticks; a shot fired at tick
/// `t` is in the snapshot for `t` or `t + 1` unless it already hit).
pub const CONFIRM_GRACE_TICKS: u64 = 4;
/// Share of a reconciliation offset kept per tick (fades within ~0.3 s).
pub const OFFSET_KEEP_PER_TICK: f32 = 0.85;
/// Offsets below this length (u) are dropped.
const OFFSET_EPSILON: f32 = 1e-3;
/// Extra match tolerance on top of 1.5 ticks of travel (quantization, u).
const MATCH_SLACK: f32 = 0.75;
/// Minimum cosine between predicted and authoritative directions to match.
const MATCH_MIN_COS: f32 = 0.98;
/// At most this many unconfirmed shots are kept (a stuck trigger with no
/// snapshots arriving must not grow without bound).
const MAX_PENDING: usize = 256;
/// Predicted volleys remembered for phase sync ([`ShotPredictor::on_inventory`]).
const MAX_FIRED_LOG: usize = 256;

/// How the client predicts a weapon's shot (mirrors its module's `fire`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotModel {
    /// Not predicted: the instant Leech Arc (no projectile; its beam cue is
    /// the shot), Borrowed Time (a stationary pulse and a rewind) and the
    /// Prox Mine (dropped behind the ship and slowed by drag).
    None,
    /// The generic `ls_sim::weapons::fire_pattern`: one bolt along the aim,
    /// a fan of up to 3 about the ship's up axis, or a centre bolt plus a
    /// ring.
    Pattern,
    /// Scatter Laser: a fan of `projectiles` bolts about the up axis that
    /// turns about the right axis (vertical) on every odd shot when
    /// `alternate`.
    Fan { alternate: bool },
    /// Scrap Shotgun: the pellets fly in a random cone keyed by the world's
    /// seed, which the client does not know; only the centre pellet is
    /// drawn until the server's pellets arrive.
    Center,
    /// Corkscrew: a line of bolts across the aim axis spinning at `spin_hz`
    /// while the fan widens up to `max_radius`, rotated by `rotate_deg` per
    /// shot.
    Helix {
        spin_hz: f32,
        rotate_deg: f32,
        max_radius: f32,
    },
}

impl ShotModel {
    /// The model for a weapon (by its module id and data parameters).
    pub fn for_weapon(def: &WeaponDef) -> ShotModel {
        match def.id.as_str() {
            "leech_arc" | "borrowed_time" | "prox_mine" => ShotModel::None,
            "scrap_shotgun" => ShotModel::Center,
            "scatter_laser" => ShotModel::Fan {
                alternate: def.param("fan_alternate", 1.0) > 0.0,
            },
            "corkscrew" => ShotModel::Helix {
                spin_hz: def.param("spin_hz", 1.0),
                rotate_deg: def.param("shot_rotate_deg", 36.0),
                max_radius: def.param("max_radius", 4.0),
            },
            _ => ShotModel::Pattern,
        }
    }

    /// Shots after which the pattern repeats (1 = it does not depend on the
    /// shot count, or the period is too long to infer).
    pub fn period(self) -> u32 {
        match self {
            ShotModel::Fan { alternate: true } => 2,
            ShotModel::Helix { rotate_deg, .. } => (1..=36u32)
                .find(|k| {
                    let a = (*k as f32 * rotate_deg).rem_euclid(360.0);
                    a < 1e-3 || 360.0 - a < 1e-3
                })
                .unwrap_or(1),
            _ => 1,
        }
    }
}

/// How a predicted projectile moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// Along `PredictedShot::vel`.
    Straight,
    /// A Corkscrew bolt: `axis * s + radial(theta) * min(factor * s, max_radius)`
    /// after `s` u of flight along the axis, `theta = theta0 + 2 pi spin_hz t`.
    Helix {
        axis: Vec3,
        right: Vec3,
        theta0: f32,
        factor: f32,
        spin_hz: f32,
        max_radius: f32,
        speed: f32,
    },
}

/// A locally predicted projectile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredictedShot {
    /// Tick the shot was fired on (it moves for the first time in the same tick).
    pub tick: u64,
    pub weapon: WeaponId,
    /// Muzzle position at the moment of firing.
    pub origin: Vec3,
    /// Muzzle velocity (for a helix bolt: its first direction).
    pub vel: Vec3,
    pub motion: Motion,
    /// Path length to the first terrain hit (`INFINITY` if none within the
    /// lifetime): the shot is not drawn beyond it.
    pub reach: f32,
    /// Age in ticks (1 at the end of the firing tick) at the terrain hit.
    pub reach_age: f64,
    pub lifetime_ticks: u64,
    /// Resource spent (energy, scrap or count, by the weapon's cost kind),
    /// split evenly over the volley's projectiles.
    pub cost: f32,
    /// Hull burned by overdrive for the missing energy (0 otherwise), split
    /// like `cost`.
    pub hull_cost: f32,
    /// The weapon's shot count this volley was predicted with.
    pub phase: u32,
    /// Index of the projectile in its volley.
    pub index: u8,
    /// Ship pose and radius at firing (to re-phase the volley).
    ship_pos: Vec3,
    ship_rot: Quat,
    ship_radius: f32,
    /// Charge fraction of a charge weapon's shot (0 otherwise).
    pub charge: f32,
}

impl PredictedShot {
    /// Age in ticks at (fractional) tick `t`: the simulation moves a
    /// projectile once in the tick it is fired, so at the end of tick
    /// `tick` it has flown one tick.
    fn age_ticks(&self, t: f64) -> f64 {
        t - self.tick as f64 + 1.0
    }

    /// Position at (fractional) tick `t`.
    pub fn pos_at(&self, t: f64) -> Vec3 {
        match self.motion {
            Motion::Straight => self.origin + self.vel * ((self.age_ticks(t) / HZ as f64) as f32),
            Motion::Helix {
                axis,
                right,
                theta0,
                factor,
                spin_hz,
                max_radius,
                speed,
            } => {
                let age = self.age_ticks(t) as f32 * ls_math::DT;
                let s = speed * age;
                let (sin, cos) = det::sin_cos(theta0 + std::f32::consts::TAU * spin_hz * age);
                let up = axis.cross(right);
                let r = (factor * s).min(max_radius);
                self.origin + axis * s + (right * cos + up * sin) * r
            }
        }
    }

    /// Velocity at tick `t` (for a helix bolt: over the tick before `t`, like
    /// the simulation's per-tick velocity).
    pub fn vel_at(&self, t: f64) -> Vec3 {
        match self.motion {
            Motion::Straight => self.vel,
            Motion::Helix { .. } => (self.pos_at(t) - self.pos_at(t - 1.0)) * HZ,
        }
    }

    /// Whether the shot is still in flight (and drawn) at tick `t`.
    pub fn visible_at(&self, t: f64) -> bool {
        t >= self.tick as f64 - 1.0
            && t < (self.tick + self.lifetime_ticks) as f64
            && self.age_ticks(t) < self.reach_age
    }
}

/// Own-ship resources as of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Resources {
    pub hull: f32,
    pub energy: f32,
    pub scrap: f32,
    /// Estimated count of the secondary weapon.
    pub count: u32,
}

/// Counters for the debug overlay and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShotStats {
    /// Shots spawned locally.
    pub predicted: u64,
    /// Predicted shots matched by a server projectile.
    pub confirmed: u64,
    /// Predicted shots the server never showed.
    pub dropped: u64,
    /// Own server projectiles that matched no prediction.
    pub unpredicted: u64,
    /// Volleys rebuilt after an inventory showed another shot count.
    pub rephased: u64,
}

/// Data tables the firing rules need.
#[derive(Debug, Clone, Copy)]
pub struct FireRules<'a> {
    pub weapons: &'a WeaponTable,
    pub rules: &'a ResolvedRules,
    pub tuning: &'a ShipTuning,
}

/// A charge in progress (hold-to-charge weapons), mirrored from the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeMirror {
    pub slot: WeaponSlot,
    pub weapon: WeaponId,
    /// Tick the trigger went down.
    pub start_tick: u64,
    /// Ticks to full charge.
    pub full_ticks: u64,
    /// Releasing before this many ticks cancels.
    pub min_ticks: u64,
}

impl ChargeMirror {
    /// Ticks held at `tick`.
    pub fn held(&self, tick: u64) -> u64 {
        tick.saturating_sub(self.start_tick)
    }
}

/// What the predictor needs to rebuild a weapon's volley.
#[derive(Debug, Clone, Copy, PartialEq)]
struct WeaponSpec {
    model: ShotModel,
    n: usize,
    spread_deg: f32,
    speed: f32,
    proj_radius: f32,
    lifetime_ticks: u64,
    /// Ticks the own weapons go offline after firing (Borrowed Time).
    offline_ticks: u64,
    /// An instant beam weapon (the Leech Arc): its paid shots are recorded
    /// for the locally drawn beam ([`ShotPredictor::last_beam`]).
    beam: bool,
}

impl WeaponSpec {
    fn new(def: &WeaponDef) -> Self {
        WeaponSpec {
            offline_ticks: if def.id == "borrowed_time" {
                ls_math::Tick::ticks_for_secs(def.param("offline_secs", 3.0))
            } else {
                0
            },
            beam: def.id == "leech_arc",
            model: ShotModel::for_weapon(def),
            n: def.projectiles.max(1) as usize,
            spread_deg: def.spread_deg,
            speed: def.speed,
            proj_radius: def.projectile_radius,
            lifetime_ticks: def.lifetime_ticks(),
        }
    }
}

/// See the module docs.
#[derive(Debug, Clone, Default)]
pub struct ShotPredictor {
    pending: Vec<PredictedShot>,
    /// Own server projectiles seen so far (by id) and their render offsets.
    own: BTreeMap<u16, Vec3>,
    /// Weapons the ship is known to carry (loadout plus any seen in snapshots).
    carried: BTreeSet<WeaponId>,
    /// Selection made by the local player (sticky, like on the server).
    selected_primary: Option<WeaponId>,
    selected_secondary: Option<WeaponId>,
    /// Primary weapon the newest snapshot reports.
    server_primary: Option<WeaponId>,
    /// Loadout secondary (the server does not send the secondary selection).
    default_secondary: Option<WeaponId>,
    primary_ready: u64,
    secondary_ready: u64,
    base: Resources,
    base_tick: u64,
    stats: ShotStats,
    /// Shots fired per weapon (the next volley's phase).
    shot_count: BTreeMap<WeaponId, u32>,
    /// Paid shots predicted so far (tick, weapon), oldest first: the shots
    /// an inventory of an earlier tick does not count yet.
    fired: std::collections::VecDeque<(u64, WeaponId)>,
    specs: BTreeMap<WeaponId, WeaponSpec>,
    charge: Option<ChargeMirror>,
    /// No shots before this tick (Borrowed Time takes the weapons offline).
    offline_until: u64,
    /// The newest predicted arc of an instant beam weapon (tick, weapon).
    last_beam: Option<(u64, WeaponId)>,
    /// The own Eyeball is guided: the world does not fire the secondary.
    guiding: bool,
}

impl ShotPredictor {
    /// A predictor for a ship with the rule set's loadout.
    pub fn new(rules: &ResolvedRules) -> Self {
        let mut carried: BTreeSet<WeaponId> = rules.loadout_primaries.iter().copied().collect();
        carried.extend(rules.loadout_secondaries.iter().map(|s| s.0));
        ShotPredictor {
            carried,
            server_primary: rules.loadout_primaries.first().copied(),
            default_secondary: rules.loadout_secondaries.first().map(|s| s.0),
            ..Default::default()
        }
    }

    pub fn stats(&self) -> ShotStats {
        self.stats
    }

    /// Unconfirmed predicted shots.
    pub fn pending(&self) -> &[PredictedShot] {
        &self.pending
    }

    /// Render offset of an own server projectile (zero if none).
    pub fn offset(&self, id: u16) -> Vec3 {
        self.own.get(&id).copied().unwrap_or(Vec3::ZERO)
    }

    /// The charge in progress (charge weapons), if any.
    pub fn charge(&self) -> Option<ChargeMirror> {
        self.charge
    }

    /// The newest predicted arc of an instant beam weapon (the Leech Arc):
    /// its tick and weapon. The host draws the own beam from it at once
    /// instead of waiting a round trip for the server's beam cue.
    pub fn last_beam(&self) -> Option<(u64, WeaponId)> {
        self.last_beam
    }

    /// Shots of `w` predicted so far (the phase of its next volley).
    pub fn shot_count(&self, w: WeaponId) -> u32 {
        self.shot_count.get(&w).copied().unwrap_or(0)
    }

    /// The selected weapon of a slot as the predictor sees it.
    pub fn selected(&self, slot: WeaponSlot) -> Option<WeaponId> {
        match slot {
            WeaponSlot::Primary => self.selected_primary.or(self.server_primary),
            WeaponSlot::Secondary => self.selected_secondary.or(self.default_secondary),
        }
    }

    /// Weapons are offline until `tick` (a Borrowed Time rewind): nothing
    /// is predicted before it.
    pub fn set_offline_until(&mut self, tick: u64) {
        self.offline_until = self.offline_until.max(tick);
    }

    /// Tick until which the weapons are offline (0 = never set).
    pub fn offline_until(&self) -> u64 {
        self.offline_until
    }

    /// Whether an own guided projectile (Eyeball) is in flight: the world
    /// skips the secondary meanwhile.
    pub fn set_guiding(&mut self, guiding: bool) {
        self.guiding = guiding;
    }

    /// Marks weapons as carried (from the server's inventory), so selecting
    /// them is predicted.
    pub fn carry(&mut self, weapons: impl IntoIterator<Item = WeaponId>) {
        self.carried.extend(weapons);
    }

    /// Feeds the newest (non-stale) snapshot's view of the own ship:
    /// `primary` from the ship flags, resources from its status.
    pub fn on_snapshot_state(&mut self, tick: u64, primary: Option<WeaponId>, res: Resources) {
        if let Some(p) = primary {
            self.carried.insert(p);
            self.server_primary = Some(p);
        }
        self.base = res;
        self.base_tick = tick;
    }

    /// Runs the firing rules for one predicted tick. `ship` is the predicted
    /// state after tick `tick` (movement comes before firing in the
    /// simulation), `cmd` that tick's command. Returns the projectiles
    /// spawned.
    pub fn on_tick<T: TerrainQuery + ?Sized>(
        &mut self,
        tick: u64,
        ship: &ShipState,
        cmd: &InputCmd,
        rules: FireRules<'_>,
        terrain: &T,
    ) -> usize {
        self.apply_selection(cmd.weapon_select, rules.weapons);
        if !ship.alive {
            self.charge = None;
            return 0;
        }
        let overdrive = cmd.buttons.contains(Buttons::OVERDRIVE);
        let mut fired = 0;
        for (slot, button) in [
            (WeaponSlot::Primary, Buttons::FIRE_PRIMARY),
            (WeaponSlot::Secondary, Buttons::FIRE_SECONDARY),
        ] {
            let pressed = cmd.buttons.contains(button);
            fired += self.slot_tick(tick, ship, slot, pressed, overdrive, rules, terrain);
        }
        fired
    }

    /// One slot's trigger for one tick (`World::fire_slot`).
    #[allow(clippy::too_many_arguments)]
    fn slot_tick<T: TerrainQuery + ?Sized>(
        &mut self,
        tick: u64,
        ship: &ShipState,
        slot: WeaponSlot,
        pressed: bool,
        overdrive: bool,
        rules: FireRules<'_>,
        terrain: &T,
    ) -> usize {
        let selected = self.selected(slot);
        // A charge on this slot whose weapon is no longer selected is dropped.
        if self
            .charge
            .is_some_and(|c| c.slot == slot && Some(c.weapon) != selected)
        {
            self.charge = None;
        }
        let Some(w) = selected.filter(|w| (w.0 as usize) < rules.weapons.len()) else {
            return 0;
        };
        let def = rules.weapons.get(w);
        let Some(cd) = def.charge else {
            return if pressed {
                self.fire(tick, ship, slot, w, overdrive, 0.0, rules, terrain)
            } else {
                0
            };
        };
        let charging = self.charge.filter(|c| c.slot == slot);
        match (charging, pressed) {
            (None, true) => {
                if self.charge.is_none() && self.check(tick, slot, w, overdrive, rules).is_some() {
                    self.charge = Some(ChargeMirror {
                        slot,
                        weapon: w,
                        start_tick: tick,
                        full_ticks: WeaponDef::charge_ticks(cd.max_secs),
                        min_ticks: Tick::ticks_for_secs(cd.min_secs),
                    });
                }
                0
            }
            (Some(c), true) => {
                if cd.auto_release && c.held(tick) >= c.full_ticks {
                    self.release(tick, ship, c, overdrive, rules, terrain)
                } else {
                    0
                }
            }
            (Some(c), false) => self.release(tick, ship, c, overdrive, rules, terrain),
            (None, false) => 0,
        }
    }

    /// Releases a charge: fires with the charge fraction if held long enough.
    fn release<T: TerrainQuery + ?Sized>(
        &mut self,
        tick: u64,
        ship: &ShipState,
        c: ChargeMirror,
        overdrive: bool,
        rules: FireRules<'_>,
        terrain: &T,
    ) -> usize {
        self.charge = None;
        let held = c.held(tick);
        if held < c.min_ticks {
            return 0;
        }
        let charge = (held as f32 / c.full_ticks as f32).min(1.0);
        self.fire(
            tick, ship, c.slot, c.weapon, overdrive, charge, rules, terrain,
        )
    }

    fn apply_selection(&mut self, select: u8, weapons: &WeaponTable) {
        if select == 0 {
            return;
        }
        let w = WeaponId(select - 1);
        if (w.0 as usize) >= weapons.len() || !self.carried.contains(&w) {
            return;
        }
        match weapons.get(w).slot {
            WeaponSlot::Primary => self.selected_primary = Some(w),
            WeaponSlot::Secondary => self.selected_secondary = Some(w),
        }
    }

    /// Resources left after the unconfirmed shots fired since the snapshot.
    fn available(&self, weapons: &WeaponTable) -> Resources {
        let mut r = self.base;
        let mut count = 0.0f32;
        for s in self.pending.iter().filter(|s| s.tick > self.base_tick) {
            r.hull -= s.hull_cost;
            match weapons.get(s.weapon).cost {
                WeaponCost::Energy(_) => r.energy -= s.cost,
                WeaponCost::Scrap(_) => r.scrap -= s.cost,
                WeaponCost::Count(_) => count += s.cost,
            }
        }
        r.count = r.count.saturating_sub(count.round() as u32);
        r
    }

    /// Whether `w` in `slot` can fire at `tick` (cooldown, ban, cost), and
    /// what it costs: `(resource, overdrive hull)`.
    fn check(
        &self,
        tick: u64,
        slot: WeaponSlot,
        w: WeaponId,
        overdrive: bool,
        rules: FireRules<'_>,
    ) -> Option<(f32, f32)> {
        if (w.0 as usize) >= rules.weapons.len() || rules.rules.availability(w).is_banned() {
            return None;
        }
        let def = rules.weapons.get(w);
        if def.slot != slot {
            return None;
        }
        let ready = match slot {
            WeaponSlot::Primary => self.primary_ready,
            WeaponSlot::Secondary => self.secondary_ready,
        };
        if tick < ready || tick < self.offline_until {
            return None;
        }
        if slot == WeaponSlot::Secondary && self.guiding {
            return None;
        }
        let have = self.available(rules.weapons);
        match def.cost {
            WeaponCost::Energy(c) => {
                let deficit = c - have.energy.max(0.0);
                let mut hull_cost = 0.0;
                if deficit > 0.0 {
                    hull_cost = deficit * rules.tuning.overdrive_hull_per_energy;
                    if !overdrive || have.hull - hull_cost < 1.0 {
                        return None;
                    }
                }
                Some((c.min(have.energy.max(0.0)).max(0.0), hull_cost))
            }
            WeaponCost::Scrap(c) => (have.scrap >= c).then_some((c, 0.0)),
            WeaponCost::Count(n) => (have.count >= n).then_some((n as f32, 0.0)),
        }
    }

    /// Pays for and predicts one shot of `w` (cooldown and cost permitting).
    #[allow(clippy::too_many_arguments)]
    fn fire<T: TerrainQuery + ?Sized>(
        &mut self,
        tick: u64,
        ship: &ShipState,
        slot: WeaponSlot,
        w: WeaponId,
        overdrive: bool,
        charge: f32,
        rules: FireRules<'_>,
        terrain: &T,
    ) -> usize {
        let Some((cost, hull_cost)) = self.check(tick, slot, w, overdrive, rules) else {
            return 0;
        };
        let def = rules.weapons.get(w);
        let ready = match slot {
            WeaponSlot::Primary => &mut self.primary_ready,
            WeaponSlot::Secondary => &mut self.secondary_ready,
        };
        *ready = tick + def.fire_interval_ticks();
        let spec = *self.specs.entry(w).or_insert_with(|| WeaponSpec::new(def));
        if spec.offline_ticks > 0 {
            // Borrowed Time takes both weapons offline as it fires; do not
            // wait a round trip for the Rewound cue.
            self.set_offline_until(tick + spec.offline_ticks);
        }
        if spec.beam {
            self.last_beam = Some((tick, w));
        }
        let phase = self.shot_count(w);
        self.shot_count.insert(w, phase.wrapping_add(1));
        if self.fired.len() >= MAX_FIRED_LOG {
            self.fired.pop_front();
        }
        self.fired.push_back((tick, w));
        let volley = build_volley(
            &spec,
            VolleyStart {
                tick,
                weapon: w,
                ship_pos: ship.pos,
                ship_rot: ship.orientation,
                ship_radius: rules.tuning.radius,
                phase,
                charge,
            },
            Some(terrain),
        );
        let n = volley.len();
        if n == 0 {
            // Paid but not drawn (instant weapons): keep the cost for later
            // shots as a zero-lifetime entry only if it matters.
            if cost > 0.0 || hull_cost > 0.0 {
                self.push(PredictedShot {
                    tick,
                    weapon: w,
                    origin: ship.pos,
                    vel: Vec3::ZERO,
                    motion: Motion::Straight,
                    reach: 0.0,
                    reach_age: 0.0,
                    lifetime_ticks: 0,
                    cost,
                    hull_cost,
                    phase,
                    index: u8::MAX,
                    ship_pos: ship.pos,
                    ship_rot: ship.orientation,
                    ship_radius: rules.tuning.radius,
                    charge,
                });
            }
            return 0;
        }
        let share = 1.0 / n as f32;
        for mut s in volley {
            s.cost = cost * share;
            s.hull_cost = hull_cost * share;
            self.push(s);
            self.stats.predicted += 1;
        }
        n
    }

    fn push(&mut self, s: PredictedShot) {
        if self.pending.len() >= MAX_PENDING {
            let dropped = self.pending.remove(0);
            if dropped.index != u8::MAX {
                self.stats.dropped += 1;
            }
        }
        self.pending.push(s);
    }

    /// Matches the own projectiles of the newest snapshot (tick `snap_tick`)
    /// with predicted shots and drops predictions the server never showed.
    pub fn reconcile(&mut self, snap_tick: u64, own: &[NetProjectile]) {
        let t = snap_tick as f64;
        let mut next_own = BTreeMap::new();
        for p in own {
            // The server does not send the secondary selection: a kind seen
            // on an own projectile is carried (e.g. a picked-up secondary),
            // so selecting it is predicted from now on.
            self.carried.insert(WeaponId(p.kind));
            if let Some(off) = self.own.get(&p.id) {
                next_own.insert(p.id, *off);
                continue;
            }
            let tolerance = match_tolerance(p);
            let best = self
                .pending
                .iter()
                .enumerate()
                .filter(|(_, s)| s.index != u8::MAX)
                .filter(|(_, s)| s.weapon.0 == p.kind && s.tick <= snap_tick)
                .filter(|(_, s)| same_direction(s.vel_at(t), p.vel))
                .map(|(i, s)| (i, s.pos_at(t).distance(p.pos)))
                .filter(|(_, d)| *d <= tolerance)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            match best {
                Some((i, _)) => {
                    let shot = self.pending.remove(i);
                    let off = shot.pos_at(t) - p.pos;
                    let off = if off.length() > OFFSET_EPSILON {
                        off
                    } else {
                        Vec3::ZERO
                    };
                    next_own.insert(p.id, off);
                    self.stats.confirmed += 1;
                }
                None => {
                    next_own.insert(p.id, Vec3::ZERO);
                    self.stats.unpredicted += 1;
                }
            }
        }
        // Ids not in the snapshot are gone (and may be reused later).
        self.own = next_own;
        let before = self.pending.len();
        let mut dropped = 0;
        self.pending.retain(|s| {
            let keep = s.tick + CONFIRM_GRACE_TICKS > snap_tick;
            if !keep && s.index != u8::MAX {
                dropped += 1;
            }
            keep
        });
        let _ = before;
        self.stats.dropped += dropped;
    }

    /// Takes the server's shot counts (`ls_net::NetWeapon::shots` of an
    /// inventory for server tick `tick`): each weapon's counter becomes the
    /// server's count plus the shots predicted after `tick`, and volleys
    /// predicted after `tick` with another phase are rebuilt with the
    /// server's (not clipped at terrain without one).
    pub fn on_inventory<T: TerrainQuery + ?Sized>(
        &mut self,
        tick: u64,
        shots: &[(WeaponId, u32)],
        terrain: Option<&T>,
    ) {
        for &(w, server) in shots {
            let later: Vec<u64> = self
                .fired
                .iter()
                .filter(|(t, x)| *x == w && *t > tick)
                .map(|(t, _)| *t)
                .collect();
            for (j, &vt) in later.iter().enumerate() {
                let phase = server.wrapping_add(j as u32);
                let shots: Vec<PredictedShot> = self
                    .pending
                    .iter()
                    .filter(|s| s.weapon == w && s.tick == vt && s.index != u8::MAX)
                    .copied()
                    .collect();
                let Some(first) = shots.first().copied() else {
                    continue;
                };
                let Some(spec) = self.specs.get(&w).copied() else {
                    continue;
                };
                if first.phase == phase || spec.model.period() <= 1 {
                    // Same pattern: only the counter needs the new phase.
                    for s in self
                        .pending
                        .iter_mut()
                        .filter(|s| s.weapon == w && s.tick == vt)
                    {
                        s.phase = phase;
                    }
                    continue;
                }
                let start = VolleyStart::of(&first, phase);
                let rebuilt = match terrain {
                    Some(t) => build_volley(&spec, start, Some(&TerrainRef(t))),
                    None => build_volley::<dyn TerrainQuery>(&spec, start, None),
                };
                self.pending
                    .retain(|s| !(s.weapon == w && s.tick == vt && s.index != u8::MAX));
                for (mut s, old) in rebuilt.into_iter().zip(shots.iter()) {
                    s.cost = old.cost;
                    s.hull_cost = old.hull_cost;
                    self.pending.push(s);
                }
                self.stats.rephased += 1;
            }
            self.shot_count
                .insert(w, server.wrapping_add(later.len() as u32));
        }
        // Inventories come in order: older shots are counted from now on.
        while self.fired.front().is_some_and(|(t, _)| *t <= tick) {
            self.fired.pop_front();
        }
    }

    /// Fades reconciliation offsets by `ticks` simulation ticks.
    pub fn decay(&mut self, ticks: u32) {
        if ticks == 0 {
            return;
        }
        let keep = OFFSET_KEEP_PER_TICK.powi(ticks.min(64) as i32);
        for off in self.own.values_mut() {
            *off = if off.length() * keep > OFFSET_EPSILON {
                *off * keep
            } else {
                Vec3::ZERO
            };
        }
    }

    /// Removes every prediction (e.g. after the match ended).
    pub fn clear(&mut self) {
        self.stats.dropped += self.pending.iter().filter(|s| s.index != u8::MAX).count() as u64;
        self.pending.clear();
        self.charge = None;
        self.last_beam = None;
    }
}

/// A sized wrapper, so any terrain (sized or not) can be passed as
/// `&dyn TerrainQuery`.
struct TerrainRef<'a, T: TerrainQuery + ?Sized>(&'a T);

impl<T: TerrainQuery + ?Sized> TerrainQuery for TerrainRef<'_, T> {
    fn sdf(&self, p: Vec3) -> f32 {
        self.0.sdf(p)
    }
    fn normal(&self, p: Vec3) -> Vec3 {
        self.0.normal(p)
    }
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<ls_math::terrain::RayHit> {
        self.0.raycast(origin, dir, max_dist)
    }
    fn bounds(&self) -> ls_math::Aabb {
        self.0.bounds()
    }
    fn sphere_overlap(&self, center: Vec3, radius: f32) -> bool {
        self.0.sphere_overlap(center, radius)
    }
}

/// Match tolerance for a server projectile: 1.5 ticks of travel plus slack.
fn match_tolerance(p: &NetProjectile) -> f32 {
    1.5 * p.vel.length() / HZ + MATCH_SLACK
}

/// Whether two velocities point the same way (zero velocities always do).
fn same_direction(a: Vec3, b: Vec3) -> bool {
    match (a.try_normalize(), b.try_normalize()) {
        (Some(a), Some(b)) => a.dot(b) >= MATCH_MIN_COS,
        _ => true,
    }
}

/// Where and how a volley starts.
#[derive(Debug, Clone, Copy)]
struct VolleyStart {
    tick: u64,
    weapon: WeaponId,
    ship_pos: Vec3,
    ship_rot: Quat,
    ship_radius: f32,
    phase: u32,
    charge: f32,
}

impl VolleyStart {
    fn of(s: &PredictedShot, phase: u32) -> Self {
        VolleyStart {
            tick: s.tick,
            weapon: s.weapon,
            ship_pos: s.ship_pos,
            ship_rot: s.ship_rot,
            ship_radius: s.ship_radius,
            phase,
            charge: s.charge,
        }
    }
}

/// The projectiles of one volley as the weapon's module fires them (see
/// [`ShotModel`]), clipped at `terrain` when given. Costs are left at 0.
fn build_volley<T: TerrainQuery + ?Sized>(
    spec: &WeaponSpec,
    v: VolleyStart,
    terrain: Option<&T>,
) -> Vec<PredictedShot> {
    let rot = v.ship_rot;
    let fwd = rot.forward();
    let muzzle = |dir: Vec3| v.ship_pos + dir * (v.ship_radius + spec.proj_radius + 0.05);
    let n = spec.n;
    let spread = spec.spread_deg.to_radians();
    let shot = |i: usize, origin: Vec3, vel: Vec3, motion: Motion| PredictedShot {
        tick: v.tick,
        weapon: v.weapon,
        origin,
        vel,
        motion,
        reach: f32::INFINITY,
        reach_age: f64::INFINITY,
        lifetime_ticks: spec.lifetime_ticks,
        cost: 0.0,
        hull_cost: 0.0,
        phase: v.phase,
        index: i.min(u8::MAX as usize - 1) as u8,
        ship_pos: v.ship_pos,
        ship_rot: v.ship_rot,
        ship_radius: v.ship_radius,
        charge: v.charge,
    };
    let mut out: Vec<PredictedShot> = match spec.model {
        ShotModel::None => Vec::new(),
        ShotModel::Center => vec![shot(0, muzzle(fwd), fwd * spec.speed, Motion::Straight)],
        ShotModel::Pattern => (0..n)
            .map(|i| {
                let dir = ls_sim::weapons::pattern_dir(rot, i, n, spec.spread_deg);
                shot(i, muzzle(dir), dir * spec.speed, Motion::Straight)
            })
            .collect(),
        ShotModel::Fan { alternate } => {
            let axis = if alternate && v.phase % 2 == 1 {
                rot.right()
            } else {
                rot.up()
            };
            (0..n)
                .map(|i| {
                    let a = (i as f32 - (n - 1) as f32 * 0.5) * spread;
                    let dir = Quat::from_axis_angle(axis, a).rotate(fwd);
                    shot(i, muzzle(dir), dir * spec.speed, Motion::Straight)
                })
                .collect()
        }
        ShotModel::Helix {
            spin_hz,
            rotate_deg,
            max_radius,
        } => {
            let axis = fwd;
            let right = rot.right();
            let up = axis.cross(right);
            let phase = ((v.phase % 3600) as f32 * rotate_deg).to_radians();
            let tan = det::tan(spread);
            (0..n)
                .map(|i| {
                    let k = i as f32 - (n - 1) as f32 * 0.5;
                    let theta0 = if k < 0.0 {
                        phase + std::f32::consts::PI
                    } else {
                        phase
                    };
                    let factor = k.abs() * tan;
                    let (s, c) = det::sin_cos(theta0);
                    let radial = right * c + up * s;
                    let motion = Motion::Helix {
                        axis,
                        right,
                        theta0,
                        factor,
                        spin_hz,
                        max_radius,
                        speed: spec.speed,
                    };
                    shot(
                        i,
                        muzzle(axis),
                        (axis + radial * factor) * spec.speed,
                        motion,
                    )
                })
                .collect()
        }
    };
    if let Some(terrain) = terrain {
        for s in &mut out {
            clip_at_terrain(s, terrain);
        }
    }
    out
}

/// Sets `reach` / `reach_age` from the first terrain hit along the path.
fn clip_at_terrain<T: TerrainQuery + ?Sized>(s: &mut PredictedShot, terrain: &T) {
    match s.motion {
        Motion::Straight => {
            let speed = s.vel.length();
            let range = speed * s.lifetime_ticks as f32 / HZ;
            let Some(dir) = s.vel.try_normalize() else {
                return;
            };
            if let Some(h) = terrain.raycast(s.origin, dir, range) {
                s.reach = h.distance;
                s.reach_age = if speed > 0.0 {
                    h.distance as f64 / (speed as f64 / HZ as f64)
                } else {
                    f64::INFINITY
                };
            }
        }
        Motion::Helix { .. } => {
            // March the helix tick by tick.
            let mut from = s.origin;
            let mut travelled = 0.0;
            for k in 1..=s.lifetime_ticks {
                let to = s.pos_at(s.tick as f64 + k as f64 - 1.0);
                let seg = to - from;
                let len = seg.length();
                if len > 0.0 {
                    if let Some(h) = terrain.raycast(from, seg, len) {
                        s.reach = travelled + h.distance;
                        s.reach_age = (k - 1) as f64 + (h.distance / len) as f64;
                        return;
                    }
                }
                travelled += len;
                from = to;
            }
        }
    }
}

#[cfg(test)]
mod tests;
