//! Belt Up racing (M4 package C5; `docs/milestone-4-plan.md` §3.8 "Bots").
//!
//! A racer follows a planned racing line ([`crate::race_line`]):
//!
//! - **Hull-aware lines.** A line costs time plus a radiation weight times
//!   the dose (seconds per hull point). The racer plans the fastest line
//!   whose dose to the finish (the dive included) fits its hull budget: its
//!   hull above a reserve ([`HULL_RESERVE`] plus 10 % of full hull), times
//!   its personality's share. So the weight falls while it has hull to
//!   spare and rises when it is short. It plans again at the start of a
//!   heat, after a respawn, and when its hull strays from the plan (hits,
//!   scavenging, pizza).
//! - **Personalities** (§3.8, spread over the field by the bot seed): the
//!   belt hugger flies the belt line (a fixed high weight), the skimmer
//!   spends 60 % of its budget (short dashes along the surface), the
//!   daredevil all of it (open dashes over the densest stretches). Time
//!   trials pin a line ([`RaceLineChoice`]): the belt line, the open line
//!   (weight 0, no scavenging) and the mixed line (the whole budget).
//! - **Flying.** The bot aims a few units ahead along the line, slows where
//!   the line squeezes through rock (the planned speed), lifts over or
//!   ducks under drifting boulders, boosts on long clear stretches and on
//!   the dive. After the lap it dives straight to the exit once the way is
//!   clear. A bot that makes no progress for a few seconds takes a
//!   navigation route to the line further ahead.
//! - **Hull.** Before the dive (or whenever the hull would not last even on
//!   the belt line) a bot scavenges a scrap source near its line: wrecks,
//!   scrap piles, debris (hold R). When a Pizza delivery is aimed at a bot
//!   in the back half of the field, it holds still for the pizza.
//! - **Combat.** The racer just ahead in the running order is the target
//!   when it is in front within range: primaries, homing ordnance, Magnet
//!   Sneeze (pulls it back), Swap Harpoon (trades places), Paperweights
//!   thrown into its line; mines when chased (the usual secondary logic).
//!   Daredevils and skimmers bump a rival alongside, preferably out of the
//!   belt. Wanted pickups close to the line are grabbed on the way.

use std::sync::Arc;

use ls_game::ls_math::level::BeltSite;
use ls_game::ls_sim::boulders;
use ls_game::ls_sim::modes::belt_up::{heat_phase, RacerView, LAP_GATES};
use ls_game::ls_sim::PlayerId;

use super::*;
use ls_game::ls_math::level::BELT_WEDGES;

use crate::race_line::{station_of, Lattice, RacingLine, Radiation, STATIONS};

/// Weight of the belt line (seconds per hull point: radiation avoided at
/// almost any cost).
pub const BELT_WEIGHT: f32 = 4.0;
/// The open line keeps this far outside the belt (u): open space, not
/// skimming.
pub const OPEN_SHELTER: f32 = 6.0;
/// Hull a racer wants left at the finish (plus 10 % of its full hull).
pub const HULL_RESERVE: f32 = 6.0;
/// The racer plans again when its hull strays this far from the plan (hull).
const REPLAN_DRIFT: f32 = 8.0;
/// Boost kept for the dive (spent on the lap only above it).
const DIVE_BOOST_RESERVE: f32 = 70.0;
/// No progress along the line for this long: take a navigation route.
const STUCK_TICKS: u64 = 4 * TICK_HZ as u64;
/// Scrap sources further than this from the racer are not worth the detour
/// (u).
const SCAVENGE_RANGE: f32 = 40.0;
/// Pickups this close in front are worth a detour (u).
const PICKUP_DETOUR: f32 = 14.0;
/// A rival this close alongside gets bumped (u).
const BUMP_RANGE: f32 = 7.0;
/// The racer ahead is engaged within this distance (u) ...
const ENGAGE_RANGE: f32 = 60.0;
/// ... and this angle off the flight direction (rad).
const ENGAGE_CONE: f32 = 0.8;

/// How a racer likes to fly (§3.8: spread over the field by the bot seed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RacePersonality {
    /// Inside the belt, along its inner edge; scavenges.
    Hugger,
    /// The belt with dashes just above or below its surface.
    Skimmer,
    /// The open line over the densest stretches.
    Daredevil,
}

impl RacePersonality {
    /// The personality of the bot with `seed`.
    pub fn for_seed(seed: u64) -> RacePersonality {
        [
            RacePersonality::Hugger,
            RacePersonality::Skimmer,
            RacePersonality::Daredevil,
        ][(seed % 3) as usize]
    }

    /// How it plans: a fixed radiation weight, or the share of its hull
    /// budget it spends and the lowest weight it accepts.
    fn policy(self) -> LinePolicy {
        match self {
            RacePersonality::Hugger => LinePolicy::Fixed(BELT_WEIGHT),
            RacePersonality::Skimmer => LinePolicy::Budget {
                share: 0.6,
                min_weight: 0.2,
            },
            RacePersonality::Daredevil => LinePolicy::Budget {
                share: 1.0,
                min_weight: 0.1,
            },
        }
    }
}

/// How a racer picks its line weight.
#[derive(Debug, Clone, Copy, PartialEq)]
enum LinePolicy {
    /// Open space (at least [`OPEN_SHELTER`] u off the belt), radiation
    /// ignored.
    Open,
    Fixed(f32),
    Budget {
        share: f32,
        min_weight: f32,
    },
}

/// Which line a racer flies (time trials pin one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RaceLineChoice {
    /// By personality and hull (the real racers).
    #[default]
    Auto,
    /// Only the belt line (no scavenging).
    Belt,
    /// Only the open line (no scavenging, no fallback).
    Open,
    /// The fastest line the hull budget allows (as a daredevil), with
    /// scavenging, no combat.
    Mixed,
}

/// Everything a racer needs about the course (built at the match start
/// from the belt and the shared lattice) and its current plan.
#[derive(Debug, Clone)]
pub struct RaceKit {
    belt: BeltSite,
    lattice: Arc<Lattice>,
    rad: Radiation,
    personality: RacePersonality,
    choice: RaceLineChoice,
    /// The line flown now (in the racer's slot frame) and the hull it was
    /// planned with.
    line: Option<RacingLine>,
    plan_hull: f32,
    plan_slot: u8,
    plan_gates: u8,
    /// The safest line from the same point (scavenging decisions).
    safe: Option<RacingLine>,
    /// Lines planned (diagnostics).
    plans: u32,
    /// Furthest line index reached and when.
    best_index: usize,
    best_tick: u64,
    heat: u8,
    /// Scrap source being scavenged.
    scrap: Option<Vec3>,
    /// The way down to the exit is clear (checked now and then).
    dive_clear: (u64, bool),
}

impl RaceKit {
    pub fn new(
        belt: BeltSite,
        lattice: Arc<Lattice>,
        radiation_scale: f32,
        personality: RacePersonality,
        choice: RaceLineChoice,
    ) -> RaceKit {
        RaceKit {
            belt,
            lattice,
            rad: Radiation::builtin(radiation_scale),
            personality,
            choice,
            line: None,
            plan_hull: 0.0,
            plan_slot: 0,
            plan_gates: 0,
            safe: None,
            plans: 0,
            best_index: 0,
            best_tick: 0,
            heat: u8::MAX,
            scrap: None,
            dive_clear: (0, false),
        }
    }

    pub fn personality(&self) -> RacePersonality {
        self.personality
    }

    pub fn belt(&self) -> &BeltSite {
        &self.belt
    }

    /// The line flown now (in the racer's slot frame), if planned.
    pub fn line(&self) -> Option<&RacingLine> {
        self.line.as_ref()
    }

    /// Lines planned so far.
    pub fn plans(&self) -> u32 {
        self.plans
    }

    fn policy(&self) -> LinePolicy {
        match self.choice {
            RaceLineChoice::Auto => self.personality.policy(),
            RaceLineChoice::Belt => LinePolicy::Fixed(BELT_WEIGHT),
            RaceLineChoice::Open => LinePolicy::Open,
            RaceLineChoice::Mixed => LinePolicy::Budget {
                share: 1.0,
                min_weight: 0.1,
            },
        }
    }

    /// The hull the line expects at its point `idx`.
    fn expected_hull(&self, idx: usize) -> f32 {
        self.line.as_ref().map_or(self.plan_hull, |l| {
            self.plan_hull - (l.dose_to_go[0] - l.dose_to_go[idx.min(l.dose_to_go.len() - 1)])
        })
    }

    /// Plans the line from where the racer is (slot `slot`, `gates`
    /// passed, at `pos` with `hull`; `reserve` hull kept at the finish).
    fn replan(&mut self, slot: u8, gates: u8, pos: Vec3, hull: f32, reserve: f32, cruise: f32) {
        let k = slot as usize % BELT_WEDGES;
        let station = station_of(&self.belt, k, gates, pos).min(STATIONS - 1);
        let local = self.belt.rotate(pos, -(k as i32));
        let lat = &self.lattice;
        let line = match self.policy() {
            LinePolicy::Open => lat
                .plan_open(OPEN_SHELTER, &self.rad, cruise, station, local)
                .or_else(|| lat.plan_from(0.0, &self.rad, cruise, station, local)),
            LinePolicy::Fixed(w) => lat.plan_from(w, &self.rad, cruise, station, local),
            LinePolicy::Budget { share, min_weight } => lat.plan_budget(
                (hull - reserve).max(0.0) * share,
                min_weight,
                &self.rad,
                cruise,
                station,
                local,
            ),
        };
        let safe = if matches!(self.choice, RaceLineChoice::Auto | RaceLineChoice::Mixed) {
            lat.plan_from(BELT_WEIGHT, &self.rad, cruise, station, local)
        } else {
            None
        };
        self.line = line.map(|l| l.rotated(&self.belt, k));
        self.safe = safe.map(|l| l.rotated(&self.belt, k));
        self.plan_hull = hull;
        self.plan_slot = slot;
        self.plan_gates = gates;
        self.best_index = 0;
        self.plans += 1;
    }
}

impl Brain {
    /// Sets up racing for a Belt Up match on a belt level.
    pub fn set_race(&mut self, kit: RaceKit) {
        self.race = Some(kit);
    }

    /// The racing kit (Belt Up on a belt level).
    pub fn race_kit(&self) -> Option<&RaceKit> {
        self.race.as_ref()
    }

    /// Belt Up: plans the racer's movement and weapons. False when the bot
    /// is not racing (no race, no belt, not in this heat).
    pub(super) fn plan_race<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) -> bool {
        let Some(race) = p.race.as_ref() else {
            return false;
        };
        let Some(me) = race.racer(PlayerId(u16::from(p.id))).copied() else {
            return false;
        };
        let Some(mut kit) = self.race.take() else {
            return false;
        };
        let handled = self.plan_race_with(p, terrain, plan, counters, race, &me, &mut kit);
        self.race = Some(kit);
        handled
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_race_with<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
        counters: &mut BrainCounters,
        race: &RaceView,
        me: &RacerView,
        kit: &mut RaceKit,
    ) -> bool {
        if me.finish_tick > 0 {
            return false;
        }
        let cruise = self.tuning.cruise_speed;
        if race.heat != kit.heat {
            kit.heat = race.heat;
            kit.best_tick = p.tick;
            kit.scrap = None;
            kit.line = None;
        }
        let held = race.phase != heat_phase::RACING || me.hold_end_tick > p.tick;
        self.choose_line(p, me, kit);
        let Some(line) = kit.line().cloned() else {
            return false;
        };
        let belt = kit.belt.clone();
        let idx = line.index_at(&belt, me.slot as usize, me.gates, p.pos);
        if held {
            // Start lights or the hold after a death: face along the line.
            let ahead = line.points[(idx + 3).min(line.points.len() - 1)];
            plan.aim = Some(ahead);
            kit.best_tick = p.tick;
            counters.race.held_ticks += 1;
            return true;
        }
        counters.race.race_ticks += 1;
        if idx > kit.best_index {
            kit.best_index = idx;
            kit.best_tick = p.tick;
        }
        let lap_done = me.gates >= LAP_GATES;

        // Pizza for a racer in the back half: hold still for it.
        if self.wants_pizza(p, race, me) {
            plan.desired_vel = Vec3::ZERO;
            plan.goal_dist = 0.0;
            counters.moods.pizza_hold_ticks += 1;
            return true;
        }

        // Hull short of the finish: scavenge near the line.
        if self.race_scavenge(p, terrain, plan, kit, &line, idx, me) {
            counters.race.scavenge_ticks += 1;
            return true;
        }

        // Movement along the line (or down to the exit).
        let target = if lap_done {
            counters.race.dive_ticks += 1;
            if p.tick >= kit.dive_clear.0 + 10 {
                let clear = crate::race_line::dive_clear(terrain, p.pos, belt.center);
                kit.dive_clear = (p.tick, clear);
            }
            if kit.dive_clear.1 {
                belt.center
            } else {
                line.points[line.points.len() - 2]
            }
        } else {
            lookahead(&line, idx, p.pos, (p.vel.length() * 0.5).clamp(5.0, 10.0))
        };
        let target = dodge_boulders(&belt, p.pos, target, p.tick);
        let to = target - p.pos;
        let dist = to.length();
        let planned = (idx..(idx + 6).min(line.speed.len()))
            .map(|k| line.speed[k])
            .fold(1.0f32, f32::min);
        let stuck = !lap_done && p.tick > kit.best_tick + STUCK_TICKS;
        if stuck {
            // No progress: a route to the line further ahead.
            let goal = line.points[(kit.best_index + 8).min(line.points.len() - 2)];
            if self
                .plan_travel(p, terrain, goal, cruise, false, plan)
                .is_some()
            {
                counters.race.stuck_ticks += 1;
                if p.tick > kit.best_tick + 3 * STUCK_TICKS {
                    kit.best_tick = p.tick;
                }
                return true;
            }
        }
        let speed = if lap_done { cruise } else { cruise * planned };
        plan.desired_vel = to / dist.max(1e-3) * speed;
        plan.goal_dist = if lap_done { dist } else { f32::INFINITY };
        plan.clearance = Some(if planned >= 0.99 { 1.6 } else { 0.9 });
        let straight = p.vel.normalize_or_zero().dot(to / dist.max(1e-3)) > 0.97;
        // Boost on the dive; on the lap only above a reserve kept for it.
        plan.boost = p.boost > 5.0
            && straight
            && (lap_done || (planned >= 0.99 && p.boost > DIVE_BOOST_RESERVE));
        if plan.boost {
            plan.desired_vel = plan.desired_vel.normalize_or_zero() * self.tuning.boost_speed;
        }

        if kit.choice != RaceLineChoice::Auto {
            return true;
        }
        // Grab a wanted pickup right by the line.
        self.race_pickup(p, plan);
        // Bump a rival alongside (not the hugger).
        if kit.personality != RacePersonality::Hugger {
            self.race_bump(p, &belt, plan, counters);
        }
        // The racer ahead.
        let ahead = racer_ahead(p, race, me);
        if let Some(t) = ahead {
            self.plan_fire(p, &t, terrain, plan);
            counters.race.engage_ticks += 1;
        } else {
            self.charge_started = None;
        }
        self.plan_secondary(p, ahead.as_ref(), terrain, plan);
        self.plan_gadget(p, ahead.as_ref(), plan);
        true
    }

    /// Plans the line when there is none (heat start), the racer changed
    /// slot, respawned, or its hull strayed from the plan.
    fn choose_line(&mut self, p: &Perception, me: &RacerView, kit: &mut RaceKit) {
        if me.gates >= LAP_GATES {
            return; // the dive needs no line
        }
        let reserve = HULL_RESERVE + 0.1 * self.tuning.hull_max;
        let stale = match kit.line.as_ref() {
            None => true,
            Some(l) => {
                let idx = l.index_at(&kit.belt, me.slot as usize, me.gates, p.pos);
                let drift = p.hull - kit.expected_hull(idx);
                kit.plan_slot != me.slot
                    || me.gates < kit.plan_gates
                    || drift > REPLAN_DRIFT
                    || (drift < -REPLAN_DRIFT && me.gates != kit.plan_gates)
            }
        };
        if stale {
            let cruise = self.tuning.cruise_speed;
            kit.replan(me.slot, me.gates, p.pos, p.hull, reserve, cruise);
        }
    }

    /// Whether to wait for a pizza: it is aimed at this bot, the bot is in
    /// the back half of the field and missing hull.
    fn wants_pizza(&self, p: &Perception, race: &RaceView, me: &RacerView) -> bool {
        let Some(target) = p.mood.pizza_target() else {
            return false;
        };
        let back = me.place as usize * 2 > race.racers.len();
        target == p.id && back && p.hull < 0.8 * self.tuning.hull_max
    }

    /// Scavenges a scrap source near the racer when its hull would not
    /// last to the finish even on the safest line it may fly. False when it
    /// keeps racing.
    #[allow(clippy::too_many_arguments)]
    fn race_scavenge<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
        kit: &mut RaceKit,
        line: &RacingLine,
        idx: usize,
        me: &RacerView,
    ) -> bool {
        if matches!(kit.choice, RaceLineChoice::Belt | RaceLineChoice::Open) {
            return false;
        }
        let need = kit.safe.as_ref().map_or(line.dose_to_go[idx], |l| {
            l.dose_to_go[l.index_at(&kit.belt, me.slot as usize, me.gates, p.pos)]
        }) + HULL_RESERVE
            + 0.1 * self.tuning.hull_max;
        let full = p.hull >= self.tuning.hull_max - 1.0;
        if p.hull >= need + if kit.scrap.is_some() { 10.0 } else { 0.0 } || full {
            kit.scrap = None;
            return false;
        }
        // Only when it matters: before the dive, or the hull is low.
        let late = me.gates + 2 >= LAP_GATES;
        if !late && p.hull > 0.35 * self.tuning.hull_max && kit.scrap.is_none() {
            return false;
        }
        let source = p
            .scrap_sources
            .iter()
            .filter(|(q, scrap)| *scrap >= 2.0 && q.distance(p.pos) < SCAVENGE_RANGE)
            .filter(|(q, _)| {
                self.exhausted
                    .iter()
                    .all(|(e, _)| e.distance(*q) > self.tuning.scavenge_radius)
            })
            .min_by(|a, b| {
                a.0.distance_squared(p.pos)
                    .total_cmp(&b.0.distance_squared(p.pos))
            })
            .map(|(q, _)| *q);
        let Some(spot) = source else {
            kit.scrap = None;
            return false;
        };
        if kit.scrap.is_none_or(|s| s.distance(spot) > 3.0) {
            kit.scrap = Some(spot);
            self.scavenge = Some(ScavengeJob {
                spot,
                started: p.tick,
                check_tick: None,
                check_hull: p.hull,
            });
        }
        if self.scavenge.is_none() {
            self.scavenge = Some(ScavengeJob {
                spot,
                started: p.tick,
                check_tick: None,
                check_hull: p.hull,
            });
        }
        self.plan_scavenge(p, terrain, plan);
        if self.scavenge.is_none() {
            // Gave up on it (exhausted): race on.
            kit.scrap = None;
        }
        true
    }

    /// Steers through a wanted pickup within [`PICKUP_DETOUR`] in front.
    fn race_pickup(&mut self, p: &Perception, plan: &mut Plan) {
        let owned = self.owned(p);
        let fwd = plan.desired_vel.normalize_or_zero();
        let near = p
            .pickups
            .iter()
            .filter(|q| {
                let d = q.pos - p.pos;
                d.length() < PICKUP_DETOUR && d.dot(fwd) > 0.2 * d.length()
            })
            .filter(|q| !self.skipped_pickups.iter().any(|(id, _)| *id == q.id))
            .find(|q| self.wants_pickup(p, &owned, q.item));
        if let Some(q) = near {
            let speed = plan.desired_vel.length().max(4.0);
            plan.desired_vel = (q.pos - p.pos).normalize_or_zero() * speed;
            plan.goal_dist = q.pos.distance(p.pos);
            plan.clearance = Some(0.8);
        }
    }

    /// Leans into a rival flying alongside, pushing it away from the belt.
    fn race_bump(
        &mut self,
        p: &Perception,
        belt: &BeltSite,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) {
        let fwd = plan.desired_vel.normalize_or_zero();
        let rival = p
            .contacts
            .iter()
            .filter(|c| {
                let d = c.pos - p.pos;
                d.length() < BUMP_RANGE && d.dot(fwd).abs() < 3.0
            })
            .min_by(|a, b| {
                a.pos
                    .distance_squared(p.pos)
                    .total_cmp(&b.pos.distance_squared(p.pos))
            });
        let Some(c) = rival else {
            return;
        };
        // Worth it when the rival is the more exposed one (the push sends
        // it further out of the shelter).
        if belt.shelter_distance(c.pos) + 0.5 < belt.shelter_distance(p.pos) {
            return;
        }
        let side = (c.pos - p.pos).reject_from_normal(fwd).normalize_or_zero();
        plan.desired_vel += side * (0.8 * self.tuning.cruise_speed);
        counters.race.bump_ticks += 1;
    }
}

/// The point `dist` u ahead along `line` from its point `idx` (projected
/// from `pos`).
fn lookahead(line: &RacingLine, idx: usize, pos: Vec3, dist: f32) -> Vec3 {
    let pts = &line.points;
    let last = pts.len() - 2; // the exit is not part of the lap
    let mut i = idx.min(last);
    // Start from the projection of `pos` on the segment `i -> i + 1`.
    let mut from = pts[i];
    if i < last {
        let seg = pts[i + 1] - pts[i];
        let t = ((pos - pts[i]).dot(seg) / seg.length_squared().max(1e-6)).clamp(0.0, 1.0);
        from = pts[i] + seg * t;
    }
    let mut left = dist;
    while i < last {
        let to = pts[i + 1];
        let d = from.distance(to);
        if d >= left {
            return from + (to - from) * (left / d.max(1e-6));
        }
        left -= d;
        from = to;
        i += 1;
    }
    pts[last]
}

/// Moves the aim point over or under a drifting boulder in the way.
fn dodge_boulders(belt: &BeltSite, pos: Vec3, target: Vec3, tick: u64) -> Vec3 {
    let Some(dir) = (target - pos).try_normalize() else {
        return target;
    };
    let mut out = target;
    for (c, size) in boulders::poses(belt, tick + 20) {
        let d = c - pos;
        let ahead = d.dot(dir);
        if !(0.0..16.0).contains(&ahead) {
            continue;
        }
        let side = d - dir * ahead;
        if side.length() < size + 2.0 {
            let up = if c.y > pos.y { -1.0 } else { 1.0 };
            out = Vec3::new(out.x, c.y + up * (size + 2.5), out.z);
        }
    }
    out
}

/// The nearest racer ahead of the bot in the running order (or any racer
/// close by) that is in front of it within range, as a contact.
fn racer_ahead(p: &Perception, race: &RaceView, me: &RacerView) -> Option<Contact> {
    let fwd = p.vel.normalize_or_zero();
    let ahead_ids: Vec<u16> = race
        .racers
        .iter()
        .filter(|r| r.finish_tick == 0 && r.place > 0 && r.place < me.place)
        .map(|r| r.player.0)
        .collect();
    p.contacts
        .iter()
        .filter(|c| {
            let d = c.pos - p.pos;
            let dist = d.length();
            let in_front = fwd == Vec3::ZERO || d.dot(fwd) > dist * ENGAGE_CONE.cos();
            let wanted = ahead_ids.contains(&u16::from(c.id)) || dist < 25.0;
            wanted && in_front && dist < ENGAGE_RANGE
        })
        .min_by(|a, b| {
            a.pos
                .distance_squared(p.pos)
                .total_cmp(&b.pos.distance_squared(p.pos))
        })
        .copied()
}
