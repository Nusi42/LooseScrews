//! Weapon, gadget and threat handling of the [`Brain`]:
//!
//! - **Primary** by range ([`crate::arsenal::primary_band`]): among the
//!   carried primaries the bot picks one that suits the distance (lasers mid
//!   range, kinetic long range, shotgun and Leech Arc close), weighted
//!   towards weapons it has not fired yet, and keeps it for a few seconds.
//!   Charge weapons hold the trigger: the Fusion Charger is released before
//!   the 2 s overcharge, the Polite Laser is held until it fires by itself.
//! - **Secondary** by situation: Swap Harpoon out of a Tectonic zone,
//!   Borrowed Time when hurt and low on hull, mines when chased, the
//!   Tectonic Charge on clusters of enemies (then fleeing its zone like any
//!   other), homing ordnance (Seeker, Swarm Pod, Eyeball) at fleeing
//!   targets, otherwise a random fitting one (rockets, Flash Pod, Swap
//!   Harpoon, Seed Grenade, Magnet Sneeze...); mines also now and then in a
//!   close fight; Magnet Sneeze also pulls
//!   scrap in when the bot is short of it.
//! - **Eyeball** guidance: the missile's look input turns it towards the
//!   nearest enemy; the missile orientation is tracked from the commanded
//!   turns and re-aligned with its replicated velocity.
//! - **Gadgets**: Shield on while splash threatens (ordnance flying in, a
//!   splash hit taken, a Tectonic zone), Cloak while approaching out of a fight
//!   (hunting, fetching), Decoy when chased.
//! - **Tectonic Charge zones**: a charge in flight or stuck, the launch
//!   rumble and the crack cue mark a danger zone the bot flees from.

use ls_game::ls_sim::CueKind;

use super::*;
use crate::aim::{aim_tolerance, lead_point};
use crate::arsenal::{primary_band, secondary_band};

/// Collapse zone (35 u) plus a margin: bots flee from a stuck charge.
const TECTONIC_ZONE: f32 = 42.0;
/// Zone around a charge still in flight (it may stick anywhere near).
const TECTONIC_FLIGHT_ZONE: f32 = 36.0;
/// After the crack cue the fuse runs `value` ticks; sub-charges follow for
/// about this long.
const TECTONIC_AFTERMATH_TICKS: u64 = 4 * TICK_HZ as u64;
/// Seconds a bot carries a Tectonic Charge before it settles for a single
/// target (or the rock in front of it) instead of a cluster.
const TECTONIC_PATIENCE_SECS: u64 = 20;
/// Eyeball turn rate (rad per tick; the data allows 3 rad/s).
const EYEBALL_TURN: f32 = 2.9 * DT;

/// An area to get out of (Tectonic Charge).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Danger {
    pub center: Vec3,
    pub radius: f32,
    pub until: u64,
}

/// The Eyeball being guided and its estimated orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EyeballGuide {
    pub id: u16,
    pub q: Quat,
}

/// What a secondary is aimed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum AimAt {
    /// Aim does not matter (mines, Borrowed Time).
    Nothing,
    Ship(u8),
    Point(Vec3),
}

/// A secondary the bot is about to fire (selecting it takes a round trip).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SecondaryIntent {
    pub weapon: WeaponId,
    pub aim: AimAt,
    pub until: u64,
}

impl Brain {
    // ----- events --------------------------------------------------------------

    /// A game cue arrived (`now` in the brain's input ticks). Tectonic
    /// Charge cues mark danger zones: the launch rumble near its shooter,
    /// the crack (armed) at the stuck charge for the fuse and the
    /// sub-charges after it.
    pub fn on_cue(&mut self, kind: CueKind, pos: Vec3, value: u16, now: u64) {
        match kind {
            CueKind::TectonicArmed => {
                let until = now + u64::from(value) + TECTONIC_AFTERMATH_TICKS;
                self.set_danger(pos, TECTONIC_ZONE, until);
            }
            CueKind::TectonicLaunch => {
                // Its projectile takes over once it shows up.
                self.set_danger(pos, 0.7 * TECTONIC_FLIGHT_ZONE, now + 45);
            }
            CueKind::CaveInWarning => self.on_cave_in_warning(pos, now),
            _ => {}
        }
    }

    /// This bot took damage from `weapon` (if known) at `now`.
    pub fn on_damaged(&mut self, weapon: Option<WeaponId>, now: u64) {
        self.damaged_tick = Some(now);
        if weapon
            .and_then(|w| self.arsenal.gun(w))
            .is_some_and(|g| g.splash_radius > 0.0)
        {
            self.splash_threat_until = self.splash_threat_until.max(now + 90);
        }
    }

    /// The danger zone the bot is avoiding, if any (center, radius).
    pub fn danger(&self) -> Option<(Vec3, f32)> {
        self.danger.map(|d| (d.center, d.radius))
    }

    pub(super) fn set_danger(&mut self, center: Vec3, radius: f32, until: u64) {
        match &mut self.danger {
            Some(d) if d.center.distance(center) < 10.0 => {
                d.center = center;
                d.radius = d.radius.max(radius);
                d.until = d.until.max(until);
            }
            Some(d) if d.until > until && d.radius > radius => {}
            _ => {
                self.danger = Some(Danger {
                    center,
                    radius,
                    until,
                });
                self.flee_goal = None;
            }
        }
    }

    /// Per tick: Tectonic Charges in flight or stuck, ordnance with splash
    /// flying at the bot, how long the bot has carried a Tectonic Charge.
    pub(super) fn observe_threats(&mut self, p: &Perception) {
        let now = p.tick;
        for q in &p.projectiles {
            let Some(g) = self.arsenal.gun(q.weapon) else {
                continue;
            };
            if g.role == Role::TectonicCharge {
                if q.stuck {
                    self.set_danger(q.pos, TECTONIC_ZONE, now + 45);
                } else {
                    self.set_danger(q.pos + q.vel * 0.8, TECTONIC_FLIGHT_ZONE, now + 45);
                }
                continue;
            }
            if g.splash_radius <= 0.0 || q.owner == p.id || p.team_of(q.owner) == Some(p.team) {
                continue;
            }
            let to_me = p.pos - q.pos;
            let d = to_me.length();
            if d < 10.0 + g.splash_radius && to_me.dot(q.vel) > 0.3 * d * q.vel.length() {
                self.splash_threat_until = self.splash_threat_until.max(now + 45);
            }
        }
        let carries_tectonic = self.owned(p).iter().any(|(w, ammo)| {
            *ammo > 0
                && self
                    .arsenal
                    .gun(*w)
                    .is_some_and(|g| g.role == Role::TectonicCharge)
        });
        if carries_tectonic {
            self.tectonic_since.get_or_insert(now);
        } else {
            self.tectonic_since = None;
        }
    }

    // ----- fleeing -------------------------------------------------------------

    /// Flies out of a danger zone the bot is in. False if it is not in one.
    pub(super) fn plan_flee<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) -> bool {
        let Some(d) = self.danger else {
            return false;
        };
        if p.pos.distance(d.center) > d.radius + 3.0 {
            self.flee_goal = None;
            return false;
        }
        let goal = match self.flee_goal {
            Some(g) if g.distance(d.center) > d.radius + 5.0 => g,
            _ => self.pick_flee_goal(p, &d),
        };
        self.flee_goal = Some(goal);
        let t = &self.tuning;
        let (cruise, boost) = (t.cruise_speed, t.boost_speed);
        let routed = self
            .plan_travel(p, terrain, goal, cruise, true, plan)
            .is_some();
        if !routed {
            plan.desired_vel = (goal - p.pos).normalize_or_zero() * boost;
            plan.goal_dist = goal.distance(p.pos);
        } else if plan.clearance.is_some_and(|c| c >= 2.0) {
            plan.desired_vel = plan.desired_vel.normalize_or_zero() * boost;
        }
        plan.boost = p.boost > 0.0;
        true
    }

    /// A point outside the zone, preferably away from its centre and not
    /// far from the bot.
    pub(super) fn pick_flee_goal(&mut self, p: &Perception, d: &Danger) -> Vec3 {
        let away = (p.pos - d.center)
            .try_normalize()
            .unwrap_or_else(|| random_unit(&mut self.rng));
        let fallback = d.center + away * (d.radius + 12.0);
        if !self.nav.is_ready() {
            return fallback;
        }
        let mut best: Option<(f32, Vec3)> = None;
        for _ in 0..24 {
            let Some(q) = self.nav.grid().random_node(&mut self.rng) else {
                break;
            };
            if q.distance(d.center) < d.radius + 8.0 {
                continue;
            }
            // Close to the bot and on its side of the zone.
            let score = q.distance(p.pos) - 0.5 * (q - p.pos).dot(away);
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, q));
            }
        }
        best.map_or(fallback, |(_, q)| q)
    }

    // ----- Eyeball -------------------------------------------------------------

    /// Steers an Eyeball this bot launched (look input only; the ship
    /// drifts). False if none is being guided.
    pub(super) fn guide_eyeball(&mut self, p: &Perception, cmd: &mut InputCmd) -> bool {
        let Some(m) = p
            .projectiles
            .iter()
            .find(|q| q.owner == p.id && q.guided)
            .copied()
        else {
            self.eyeball = None;
            return false;
        };
        // The missile starts with the ship's orientation (frozen while it
        // flies); after that the bot integrates its own turns and re-aligns
        // the estimate with the replicated velocity.
        let mut q = match self.eyeball {
            Some(g) if g.id == m.id => g.q,
            _ => p.orientation,
        };
        if let Some(v) = m.vel.try_normalize() {
            q = (Quat::from_rotation_arc(q.forward(), v) * q).normalize();
        }
        let speed = m.vel.length().max(1.0);
        let target = p
            .contacts
            .iter()
            .filter(|c| p.is_hostile(c))
            .min_by(|a, b| {
                a.pos
                    .distance_squared(m.pos)
                    .total_cmp(&b.pos.distance_squared(m.pos))
            });
        let dir = match target {
            Some(c) => {
                let t = (c.pos.distance(m.pos) / speed).min(2.0);
                c.pos + c.vel * t - m.pos
            }
            None => m.vel,
        };
        let look = look_toward(q, dir, EYEBALL_TURN);
        cmd.look_pitch = look.pitch;
        cmd.look_yaw = look.yaw;
        q = q
            .integrate_local(Vec3::new(look.pitch / DT, look.yaw / DT, 0.0), DT)
            .normalize();
        self.eyeball = Some(EyeballGuide { id: m.id, q });
        true
    }

    // ----- primaries -------------------------------------------------------------

    pub(super) fn plan_fire<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        t: &Contact,
        terrain: &T,
        plan: &mut Plan,
    ) {
        let dist = t.pos.distance(p.pos);
        let owned = self.owned(p);
        let choice = self.choose_primary(p, &owned, dist);
        plan.select = choice.map(|g| g.id);
        self.preferred_range = choice
            .and_then(|g| primary_band(&g))
            .map_or(self.cfg.preferred_range, |b| b.keep);

        // Aim for the weapon actually equipped if it is a combat primary
        // (switching takes a round trip), otherwise for the one wanted.
        let equipped = p
            .primary
            .filter(|w| self.arsenal.is_combat_primary(*w))
            .and_then(|w| self.arsenal.gun(w));
        if equipped.is_none_or(|g| g.charge.is_none()) {
            self.charge_started = None;
        }
        let Some(gun) = equipped.or(choice) else {
            return;
        };
        let aim = if gun.role == Role::LeechArc {
            t.pos
        } else {
            lead_point(p.pos, t.pos, t.vel, gun.speed, gun.lifetime).unwrap_or(t.pos)
        };
        plan.aim = Some(aim);
        let Some(eq) = equipped else {
            return;
        };
        let err = angle_between(p.orientation.forward(), aim - p.pos);
        let tol =
            aim_tolerance(dist, self.tuning.radius + eq.projectile_radius) + aim_slack(eq.role);
        let band = primary_band(&eq).map_or((0.0, eq.range()), |b| b.usable);
        let in_range = dist >= 0.5 * band.0 && dist <= band.1.min(eq.range().max(12.0));
        let ready = in_range
            && self.can_pay(p, &eq, 0, energy_reserve(eq.role))
            && clear_shot(terrain, p.pos, aim, eq.projectile_radius);
        plan.fire_primary = if eq.charge.is_some() {
            self.charge_trigger(p, &eq, ready, err, tol)
        } else {
            ready && err <= tol
        };
    }

    /// The primary for a fight at `dist`: keeps the current choice for a
    /// few seconds while it fits, else picks among the carried primaries
    /// that fit, weighted by fit and novelty.
    pub(super) fn choose_primary(
        &mut self,
        p: &Perception,
        owned: &[(WeaponId, u32)],
        dist: f32,
    ) -> Option<Gun> {
        let now = p.tick;
        let fits = |b: &Brain, g: &Gun| {
            primary_band(g).map_or(0.0, |band| band.score(dist))
                * f32::from(u8::from(b.can_pay(p, g, 0, energy_reserve(g.role))))
        };
        let current = self
            .primary_choice
            .filter(|w| owned.iter().any(|(x, _)| x == w))
            .and_then(|w| self.arsenal.gun(w))
            .filter(|g| fits(self, g) > 0.0);
        if let Some(c) = current {
            if now < self.next_primary_pick || self.charge_started.is_some() {
                return Some(c);
            }
        }
        let mut candidates: Vec<(Gun, f32)> = Vec::new();
        for (w, _) in owned {
            let Some(g) = self.arsenal.gun(*w) else {
                continue;
            };
            if !self.arsenal.is_combat_primary(g.id) {
                continue;
            }
            let mut weight = fits(self, &g);
            if weight <= 0.0 {
                continue;
            }
            if self.used.get(g.id.0 as usize).is_some_and(|u| *u == 0) {
                weight *= 3.0;
            }
            if current.is_some_and(|c| c.id == g.id) {
                weight *= 1.5;
            }
            candidates.push((g, weight));
        }
        let pick = weighted_pick(&mut self.rng, &candidates).or_else(|| {
            // Nothing fits: the laser, else any carried combat primary.
            self.arsenal
                .laser
                .filter(|l| owned.iter().any(|(w, _)| *w == l.id))
                .or_else(|| {
                    owned
                        .iter()
                        .filter(|(w, _)| self.arsenal.is_combat_primary(*w))
                        .find_map(|(w, _)| self.arsenal.gun(*w))
                })
        });
        self.primary_choice = pick.map(|g| g.id);
        self.next_primary_pick = now + 180 + u64::from(self.rng.below(180));
        pick
    }

    /// Trigger for a charge weapon: holds it down while charging, lets go
    /// to fire (Fusion Charger, before the overcharge) or after it fired by
    /// itself (Polite Laser).
    pub(super) fn charge_trigger(
        &mut self,
        p: &Perception,
        g: &Gun,
        ready: bool,
        err: f32,
        tol: f32,
    ) -> bool {
        let Some((max, _min, auto)) = g.charge else {
            return false;
        };
        let now = p.tick;
        let full = (max * TICKS_PER_SEC) as u64;
        match self.charge_started {
            None => {
                if ready && now >= self.next_charge && err <= 4.0 * tol + 0.1 {
                    self.charge_started = Some(now);
                    self.charge_release = if auto {
                        full + 20
                    } else {
                        (full as f32 * self.rng.range_f32(0.5, 0.9)) as u64
                    };
                    true
                } else {
                    false
                }
            }
            Some(t0) => {
                let held = now.saturating_sub(t0);
                let keep = if auto {
                    held < self.charge_release
                } else {
                    // Never into the overcharge; past the planned release
                    // only while the aim is still off.
                    held + 8 < full && (held < self.charge_release || err > tol)
                };
                if !keep {
                    self.charge_started = None;
                    self.next_charge = now + (g.fire_interval * TICKS_PER_SEC) as u64 + 2;
                }
                keep
            }
        }
    }

    // ----- secondaries -----------------------------------------------------------

    pub(super) fn plan_secondary<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        target: Option<&Contact>,
        terrain: &T,
        plan: &mut Plan,
    ) {
        let now = p.tick;
        let owned = self.owned(p);
        let loaded = |w: WeaponId| owned.iter().any(|(x, ammo)| *x == w && *ammo > 0);
        if let Some(i) = self.secondary_intent {
            let lost = matches!(i.aim, AimAt::Ship(id) if !p.contacts.iter().any(|c| c.id == id));
            if now > i.until || !loaded(i.weapon) || lost {
                self.secondary_intent = None;
            }
        }
        if self.secondary_intent.is_none() && now >= self.next_secondary {
            self.secondary_intent = self.decide_secondary(p, target, terrain, &owned);
            if self.secondary_intent.is_none() {
                self.next_secondary = now + 15;
            }
        }
        let Some(i) = self.secondary_intent else {
            return;
        };
        let Some(gun) = self.arsenal.gun(i.weapon) else {
            self.secondary_intent = None;
            return;
        };
        plan.select_secondary = Some(i.weapon);
        // Without an inventory the bot assumes the loadout's rocket.
        let selected = p.secondary.or_else(|| {
            p.weapons
                .is_none()
                .then(|| self.arsenal.rocket.map(|g| g.id))
                .flatten()
        });
        if selected != Some(i.weapon) {
            // Keep the primary's aim while the selection travels.
            return;
        }
        let aim =
            match i.aim {
                AimAt::Nothing => None,
                AimAt::Point(q) => Some(q),
                AimAt::Ship(id) => p.contacts.iter().find(|c| c.id == id).map(|c| {
                    lead_point(p.pos, c.pos, c.vel, gun.speed, gun.lifetime).unwrap_or(c.pos)
                }),
            };
        if let Some(a) = aim {
            plan.aim = Some(a);
        }
        let on_target = aim.is_none_or(|a| {
            let d = a.distance(p.pos);
            let err = angle_between(p.orientation.forward(), a - p.pos);
            let hit = self.tuning.radius + 1.0 + 0.3 * gun.splash_radius;
            err <= aim_tolerance(d, hit).max(0.03)
                && clear_shot(terrain, p.pos, a, gun.projectile_radius)
        });
        if on_target {
            plan.fire_secondary = true;
            self.secondary_intent = None;
            self.next_secondary = now
                + (gun.fire_interval * TICKS_PER_SEC) as u64
                + 30
                + u64::from(self.rng.below(60));
            if gun.role == Role::BoltRocket {
                self.next_rocket = now
                    + (self.cfg.rocket_interval_secs * TICKS_PER_SEC) as u64
                    + u64::from(self.rng.below(TICK_HZ));
            }
        }
    }

    /// Which secondary to fire now, at what (see the module docs).
    pub(super) fn decide_secondary<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        target: Option<&Contact>,
        terrain: &T,
        owned: &[(WeaponId, u32)],
    ) -> Option<SecondaryIntent> {
        let now = p.tick;
        let loaded: Vec<Gun> = owned
            .iter()
            .filter(|(_, ammo)| *ammo > 0)
            .filter_map(|(w, _)| self.arsenal.gun(*w))
            .filter(|g| !g.is_primary())
            .collect();
        if loaded.is_empty() {
            return None;
        }
        let have = |role: Role| loaded.iter().find(|g| g.role == role).map(|g| g.id);
        let intent = |weapon: WeaponId, aim: AimAt| {
            Some(SecondaryIntent {
                weapon,
                aim,
                until: now + 60,
            })
        };
        let hull_frac = p.hull / self.tuning.hull_max.max(1.0);
        let recently_hit = self.damaged_tick.is_some_and(|t| now < t + 60);
        let enemies: Vec<Contact> = p
            .contacts
            .iter()
            .filter(|c| p.is_hostile(c))
            .copied()
            .collect();

        // Out of a Tectonic zone by swapping places with an enemy outside it.
        if let (Some(d), Some(w)) = (self.danger, have(Role::SwapHarpoon)) {
            let mine = p.pos.distance(d.center);
            if mine < d.radius {
                let swap = enemies
                    .iter()
                    .filter(|c| c.pos.distance(d.center) > mine + 10.0)
                    .filter(|c| (8.0..80.0).contains(&c.pos.distance(p.pos)))
                    .filter(|c| clear_shot(terrain, p.pos, c.pos, 0.0))
                    .min_by(|a, b| {
                        a.pos
                            .distance_squared(p.pos)
                            .total_cmp(&b.pos.distance_squared(p.pos))
                    });
                if let Some(c) = swap {
                    return intent(w, AimAt::Ship(c.id));
                }
            }
        }
        // Low and still being hit: rewind 3 s.
        if hull_frac < 0.45 && recently_hit {
            if let Some(w) = have(Role::BorrowedTime) {
                return intent(w, AimAt::Nothing);
            }
        }
        // Chased, or an enemy close behind: drop a mine.
        let close_behind = enemies.iter().any(|c| {
            c.pos.distance(p.pos) < 14.0 && (c.pos - p.pos).dot(p.orientation.forward()) < 0.0
        });
        // Now and then also in a close fight: a mine left in the fight's
        // path.
        let close_fight = target.is_some_and(|t| t.pos.distance(p.pos) < 25.0);
        if self.chased(p).is_some() || close_behind || (close_fight && self.rng.chance(0.15)) {
            let mut mines = [Role::StalkerMine, Role::ProxMine];
            if self.rng.next_bool() {
                mines.swap(0, 1);
            }
            if let Some(w) = mines.into_iter().find_map(have) {
                return intent(w, AimAt::Nothing);
            }
        }
        let Some(t) = target else {
            // Nothing to shoot at: Magnet Sneeze pulls scrap in when short.
            return self.sneeze_for_scrap(p, terrain, have(Role::MagnetSneeze));
        };
        let dist = t.pos.distance(p.pos);
        // Tectonic Charge on a cluster (the bot then flees its zone).
        if let Some(w) = have(Role::TectonicCharge) {
            if let Some(point) = self.tectonic_point(p, &enemies, t, terrain) {
                return intent(w, AimAt::Point(point));
            }
        }
        // Homing ordnance at a fleeing target.
        let fleeing = t.vel.dot((t.pos - p.pos) / dist.max(1e-3)) > 3.0;
        let crowded = enemies
            .iter()
            .any(|c| c.id != t.id && c.pos.distance(p.pos) < 15.0);
        if fleeing {
            for role in [Role::Seeker, Role::SwarmPod, Role::Eyeball] {
                let Some(w) = have(role) else { continue };
                let fits = self
                    .arsenal
                    .gun(w)
                    .and_then(|g| secondary_band(&g))
                    .is_some_and(|(lo, hi)| (lo..=hi).contains(&dist));
                if fits && !(role == Role::Eyeball && crowded) {
                    return intent(w, AimAt::Ship(t.id));
                }
            }
        }
        // Otherwise one that fits the distance, now and then.
        if !self.rng.chance(0.5) {
            return None;
        }
        let close_primary = p
            .primary
            .and_then(|w| self.arsenal.gun(w))
            .is_some_and(|g| matches!(g.role, Role::ScrapShotgun | Role::LeechArc));
        let mut candidates: Vec<(Gun, f32)> = Vec::new();
        for g in &loaded {
            let Some((lo, hi)) = secondary_band(g) else {
                continue;
            };
            if !(lo..=hi).contains(&dist) {
                continue;
            }
            let mut weight = match g.role {
                Role::TectonicCharge => continue,
                Role::BoltRocket if now < self.next_rocket => continue,
                Role::Eyeball if crowded => continue,
                Role::SwapHarpoon if hull_frac < 0.4 => continue,
                Role::Seeker | Role::SwarmPod | Role::Eyeball => 0.5,
                Role::SeedGrenade if recently_hit => 3.0,
                Role::SeedGrenade => 0.5,
                Role::Paperweight => 0.5,
                Role::MagnetSneeze if close_primary => 2.0,
                Role::MagnetSneeze => 0.4,
                Role::SwapHarpoon => 0.7,
                _ => 1.0,
            };
            if self.used.get(g.id.0 as usize).is_some_and(|u| *u == 0) {
                weight *= 3.0;
            }
            candidates.push((*g, weight));
        }
        let g = weighted_pick(&mut self.rng, &candidates)?;
        intent(g.id, AimAt::Ship(t.id))
    }

    /// Magnet Sneeze at a scrap pile, wreck or debris 6-25 u away when the
    /// bot is short of scrap.
    pub(super) fn sneeze_for_scrap<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        sneeze: Option<WeaponId>,
    ) -> Option<SecondaryIntent> {
        let w = sneeze?;
        let short = p.scrap < 0.5 * self.tuning.scrap_max || self.scavenge.is_some();
        if !short || !self.rng.chance(0.3) {
            return None;
        }
        let pos = p
            .scrap_sources
            .iter()
            .filter(|(_, scrap)| *scrap >= 5.0)
            .map(|(q, _)| *q)
            .filter(|q| (6.0..25.0).contains(&q.distance(p.pos)))
            .filter(|q| clear_shot(terrain, p.pos, *q, 0.0))
            .min_by(|a, b| {
                a.distance_squared(p.pos)
                    .total_cmp(&b.distance_squared(p.pos))
            })?;
        Some(SecondaryIntent {
            weapon: w,
            aim: AimAt::Point(pos),
            until: p.tick + 90,
        })
    }

    /// Where to send a Tectonic Charge: the centre of two or more enemies
    /// close together, 35-80 u away, with no teammate near it; after
    /// [`TECTONIC_PATIENCE_SECS`] of carrying it also a lone target 30-90 u
    /// away, or the rock on the line to the target if that is at least 30 u
    /// off.
    pub(super) fn tectonic_point<T: TerrainQuery + ?Sized>(
        &self,
        p: &Perception,
        enemies: &[Contact],
        target: &Contact,
        terrain: &T,
    ) -> Option<Vec3> {
        let gun = self.arsenal.by_role(Role::TectonicCharge)?;
        let (lo, hi) = secondary_band(&gun).unwrap_or((35.0, 75.0));
        let friends_clear = |c: Vec3| {
            p.contacts
                .iter()
                .filter(|f| f.team == p.team)
                .all(|f| f.pos.distance(c) > 30.0)
        };
        let lead = |c: Vec3, v: Vec3| {
            let t = (c.distance(p.pos) / gun.speed.max(1.0)).min(3.0);
            c + v * t
        };
        let mut best: Option<(usize, Vec3)> = None;
        for e in enemies {
            let group: Vec<&Contact> = enemies
                .iter()
                .filter(|o| o.pos.distance(e.pos) < 12.0)
                .collect();
            if group.len() < 2 || best.is_some_and(|(n, _)| n >= group.len()) {
                continue;
            }
            let n = group.len() as f32;
            let centre = group.iter().fold(Vec3::ZERO, |a, c| a + c.pos) / n;
            let vel = group.iter().fold(Vec3::ZERO, |a, c| a + c.vel) / n;
            let aim = lead(centre, vel);
            let d = aim.distance(p.pos);
            if (lo..=hi).contains(&d) && friends_clear(aim) && clear_shot(terrain, p.pos, aim, 0.3)
            {
                best = Some((group.len(), aim));
            }
        }
        if let Some((_, aim)) = best {
            return Some(aim);
        }
        let patient = self
            .tectonic_since
            .is_some_and(|s| p.tick > s + TECTONIC_PATIENCE_SECS * TICK_HZ as u64);
        if !patient {
            return None;
        }
        let aim = lead(target.pos, target.vel);
        let d = aim.distance(p.pos);
        if (lo - 5.0..=hi + 15.0).contains(&d)
            && friends_clear(aim)
            && clear_shot(terrain, p.pos, aim, 0.3)
        {
            return Some(aim);
        }
        // Otherwise the rock along the line to the target (in front of or
        // behind it; fights are mostly closer than a safe distance) when it
        // is at least 30 u away: the charge sticks there and its zones
        // cover the target. The bot flees the zone afterwards.
        let dir = aim - p.pos;
        let hit = terrain.raycast(p.pos, dir, hi + 15.0)?;
        (hit.distance >= lo - 5.0 && friends_clear(hit.point))
            .then(|| p.pos + dir.normalize_or_zero() * (hit.distance - 0.6))
    }

    /// An enemy chasing the bot: close behind it along its flight
    /// direction and moving the same way.
    pub(super) fn chased(&self, p: &Perception) -> Option<Contact> {
        let speed = p.vel.length();
        if speed < 3.0 {
            return None;
        }
        let dir = p.vel / speed;
        p.contacts
            .iter()
            .filter(|c| p.is_hostile(c))
            .find(|c| {
                let rel = p.pos - c.pos;
                let d = rel.length();
                d < 25.0 && rel.dot(dir) > 0.5 * d && c.vel.dot(dir) > 0.0
            })
            .copied()
    }

    // ----- gadgets -----------------------------------------------------------------

    /// Presses the gadget button (on its press edge) to switch the Shield
    /// or Cloak to what the situation asks for, or to launch a Decoy.
    pub(super) fn plan_gadget(
        &mut self,
        p: &Perception,
        target: Option<&Contact>,
        plan: &mut Plan,
    ) {
        let now = p.tick;
        let Some(kind) = p.gadget else {
            return;
        };
        if now < self.next_gadget_press {
            return;
        }
        let in_danger = self
            .danger
            .is_some_and(|d| p.pos.distance(d.center) < d.radius + 10.0);
        let hull_frac = p.hull / self.tuning.hull_max.max(1.0);
        let recently_hit = self.damaged_tick.is_some_and(|t| now < t + 60);
        let press = match kind {
            GadgetKind::Shield => {
                let threat = now < self.splash_threat_until || in_danger;
                let want = threat && p.energy > 12.0;
                // Off again a second after the threat (or when drained).
                want != p.gadget_on
                    && (want || now >= self.splash_threat_until + 60 || p.energy < 8.0)
            }
            GadgetKind::Cloak => {
                // Not in a fight: the drain (3x while moving) would starve
                // the energy weapons.
                let approaching =
                    matches!(self.mode, Mode::Hunt | Mode::Collect) && target.is_none();
                let want = approaching && p.energy > 35.0;
                want != p.gadget_on
            }
            GadgetKind::Decoy => {
                let pressed = self.chased(p).is_some()
                    || (recently_hit && hull_frac < 0.5 && target.is_some());
                pressed && now >= self.decoy_ready && p.energy >= 32.0
            }
            GadgetKind::Other => false,
        };
        if press {
            plan.gadget = true;
            self.next_gadget_press = now + 60;
            if kind == GadgetKind::Decoy {
                self.decoy_ready = now + 11 * TICK_HZ as u64;
            }
        }
    }

    /// Counts a trigger press of weapon `w` (novelty weighting).
    pub(super) fn note_used(&mut self, w: WeaponId) {
        if let Some(u) = self.used.get_mut(w.0 as usize) {
            *u = u.saturating_add(1);
        }
    }
}

/// Extra aim tolerance (rad) for weapons that spread or reach wide.
fn aim_slack(role: Role) -> f32 {
    match role {
        // A 30 degree half-cone.
        Role::LeechArc => 0.4,
        Role::ScrapShotgun => 0.05,
        Role::ScatterLaser => 0.06,
        Role::Corkscrew => 0.03,
        _ => 0.0,
    }
}

/// Energy kept back when firing a primary (the Plasma Repeater drains 30/s).
fn energy_reserve(role: Role) -> f32 {
    match role {
        Role::PlasmaRepeater => 15.0,
        _ => 0.0,
    }
}

/// A random entry, chosen with probability proportional to its weight.
fn weighted_pick(rng: &mut Pcg32, items: &[(Gun, f32)]) -> Option<Gun> {
    let total: f32 = items.iter().map(|(_, w)| *w).sum();
    if items.is_empty() || total <= 0.0 {
        return None;
    }
    let mut x = rng.next_f32() * total;
    for (g, w) in items {
        if x < *w {
            return Some(*g);
        }
        x -= w;
    }
    items.last().map(|(g, _)| *g)
}
