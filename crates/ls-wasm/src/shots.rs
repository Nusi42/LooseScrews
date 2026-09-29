//! Client-side prediction of the local player's shots (online play).
//!
//! Without it a shot appears only once the server's snapshot carries the
//! projectile, a full round trip after the click. [`ShotPredictor`] runs the
//! same firing rules as `ls_sim` (`World::step` / `try_fire`: weapon
//! selection, rule-set bans, fire interval, energy / scrap / count cost,
//! overdrive) on the predicted local ship for every predicted tick and
//! spawns a visual projectile at once, with the simulation's muzzle offset,
//! direction and speed.
//!
//! Resources come from the newest snapshot's own-ship status minus the cost
//! of predicted shots after that snapshot (regeneration since then is
//! ignored, so prediction errs towards not firing near empty).
//!
//! **Reconciliation.** When a snapshot shows a new projectile of the local
//! player, it is matched with the predicted shot of the same weapon whose
//! extrapolated position is nearest (within about 1.5 ticks of travel). The
//! predicted shot is removed and the authoritative one takes over; the host
//! draws the player's own projectiles in the local (predicted) timeline, so
//! a correct prediction continues without a jump and the small difference
//! of a wrong one is kept as a render offset that fades out. A predicted
//! shot that no snapshot confirmed [`CONFIRM_GRACE_TICKS`] after its tick
//! (a refused shot, or one that hit something between two snapshots) is
//! removed.
//!
//! Nothing here is deterministic game state: it only affects what is drawn.

use std::collections::{BTreeMap, BTreeSet};

use ls_math::terrain::TerrainQuery;
use ls_math::Vec3;
use ls_net::NetProjectile;
use ls_sim::{
    Buttons, InputCmd, ResolvedRules, ShipState, ShipTuning, WeaponCost, WeaponId, WeaponSlot,
    WeaponTable,
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

/// A locally predicted projectile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredictedShot {
    /// Tick the shot was fired on (it moves for the first time in the same tick).
    pub tick: u64,
    pub weapon: WeaponId,
    /// Muzzle position at the moment of firing.
    pub origin: Vec3,
    pub vel: Vec3,
    /// Distance to the first terrain hit along the path (`INFINITY` if none
    /// within the lifetime): the shot is not drawn beyond it.
    pub reach: f32,
    pub lifetime_ticks: u64,
    /// Resource spent (energy, scrap or count, by the weapon's cost kind).
    pub cost: f32,
    /// Hull burned by overdrive for the missing energy (0 otherwise).
    pub hull_cost: f32,
}

impl PredictedShot {
    /// Position at (fractional) tick `t`: the simulation moves a projectile
    /// once in the tick it is fired, so at the end of tick `tick` it is one
    /// tick of travel past the muzzle.
    pub fn pos_at(&self, t: f64) -> Vec3 {
        self.origin + self.vel * (((t - self.tick as f64 + 1.0) / HZ as f64) as f32)
    }

    /// Distance travelled from the muzzle at tick `t`.
    fn travelled(&self, t: f64) -> f32 {
        self.vel.length() * (((t - self.tick as f64 + 1.0).max(0.0) / HZ as f64) as f32)
    }

    /// Whether the shot is still in flight (and drawn) at tick `t`.
    pub fn visible_at(&self, t: f64) -> bool {
        t >= self.tick as f64 - 1.0
            && t < (self.tick + self.lifetime_ticks) as f64
            && self.travelled(t) < self.reach
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
}

/// Data tables the firing rules need.
#[derive(Debug, Clone, Copy)]
pub struct FireRules<'a> {
    pub weapons: &'a WeaponTable,
    pub rules: &'a ResolvedRules,
    pub tuning: &'a ShipTuning,
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
    /// simulation), `cmd` that tick's command. Returns the shots spawned.
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
            return 0;
        }
        let overdrive = cmd.buttons.contains(Buttons::OVERDRIVE);
        let mut fired = 0;
        for (slot, button) in [
            (WeaponSlot::Primary, Buttons::FIRE_PRIMARY),
            (WeaponSlot::Secondary, Buttons::FIRE_SECONDARY),
        ] {
            if !cmd.buttons.contains(button) {
                continue;
            }
            if let Some(shot) = self.try_fire(tick, ship, slot, overdrive, rules, terrain) {
                if self.pending.len() >= MAX_PENDING {
                    self.pending.remove(0);
                    self.stats.dropped += 1;
                }
                self.pending.push(shot);
                self.stats.predicted += 1;
                fired += 1;
            }
        }
        fired
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
        for s in self.pending.iter().filter(|s| s.tick > self.base_tick) {
            r.hull -= s.hull_cost;
            match weapons.get(s.weapon).cost {
                WeaponCost::Energy(_) => r.energy -= s.cost,
                WeaponCost::Scrap(_) => r.scrap -= s.cost,
                WeaponCost::Count(_) => r.count = r.count.saturating_sub(s.cost as u32),
            }
        }
        r
    }

    fn try_fire<T: TerrainQuery + ?Sized>(
        &mut self,
        tick: u64,
        ship: &ShipState,
        slot: WeaponSlot,
        overdrive: bool,
        rules: FireRules<'_>,
        terrain: &T,
    ) -> Option<PredictedShot> {
        let w = match slot {
            WeaponSlot::Primary => self.selected_primary.or(self.server_primary)?,
            WeaponSlot::Secondary => self.selected_secondary.or(self.default_secondary)?,
        };
        if (w.0 as usize) >= rules.weapons.len() || rules.rules.availability(w).is_banned() {
            return None;
        }
        let def = rules.weapons.get(w);
        if def.slot != slot {
            return None;
        }
        let ready = match slot {
            WeaponSlot::Primary => &mut self.primary_ready,
            WeaponSlot::Secondary => &mut self.secondary_ready,
        };
        if tick < *ready {
            return None;
        }
        let have = self.available(rules.weapons);
        let mut hull_cost = 0.0;
        let cost = match def.cost {
            WeaponCost::Energy(c) => {
                let deficit = c - have.energy.max(0.0);
                if deficit > 0.0 {
                    hull_cost = deficit * rules.tuning.overdrive_hull_per_energy;
                    if !overdrive || have.hull - hull_cost < 1.0 {
                        return None;
                    }
                }
                c.min(have.energy.max(0.0)).max(0.0)
            }
            WeaponCost::Scrap(c) => {
                if have.scrap < c {
                    return None;
                }
                c
            }
            WeaponCost::Count(n) => {
                if have.count < n {
                    return None;
                }
                n as f32
            }
        };
        let ready = match slot {
            WeaponSlot::Primary => &mut self.primary_ready,
            WeaponSlot::Secondary => &mut self.secondary_ready,
        };
        *ready = tick + def.fire_interval_ticks();
        let fwd = ship.orientation.forward();
        let origin = ship.pos + fwd * (rules.tuning.radius + def.projectile_radius + 0.05);
        let range = def.speed * def.lifetime_secs;
        let reach = terrain
            .raycast(origin, fwd, range)
            .map_or(f32::INFINITY, |h| h.distance);
        Some(PredictedShot {
            tick,
            weapon: w,
            origin,
            vel: fwd * def.speed,
            reach,
            lifetime_ticks: def.lifetime_ticks(),
            cost,
            hull_cost,
        })
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
            let speed = p.vel.length();
            let tolerance = 1.5 * speed / HZ + MATCH_SLACK;
            let dir = p.vel.try_normalize();
            let best = self
                .pending
                .iter()
                .enumerate()
                .filter(|(_, s)| s.weapon.0 == p.kind && s.tick <= snap_tick)
                .filter(|(_, s)| match (dir, s.vel.try_normalize()) {
                    (Some(a), Some(b)) => a.dot(b) >= MATCH_MIN_COS,
                    _ => true,
                })
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
        self.pending
            .retain(|s| s.tick + CONFIRM_GRACE_TICKS > snap_tick);
        self.stats.dropped += (before - self.pending.len()) as u64;
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
        self.stats.dropped += self.pending.len() as u64;
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests;
