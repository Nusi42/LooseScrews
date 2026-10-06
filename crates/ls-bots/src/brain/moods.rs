//! Bots in moods (M4 package C5): what the bots do about the world physics
//! and the comedy director (`docs/milestone-4-plan.md` §3.1-§3.4, §5 C5).
//!
//! - **Gravity wells.** The pull on the ship is part of the perception;
//!   every axis without input gets a small counter-thrust against it (an
//!   idle axis would otherwise let the dampers drift toward the well), and
//!   bots keep a few units off the stones of dynamic wells (Paperweights,
//!   the Architect's well stroke). Static well cores are rock, so the wall
//!   avoidance already keeps clear of them.
//! - **Cave-ins and rubble.** A cave-in warning cue marks a danger zone the
//!   bot leaves (like a Tectonic Charge's); rubble and craters re-sample
//!   the navigation grid, so routes re-plan around them.
//! - **Pigment** (monochrome levels): a bot takes a pigment pickup when it
//!   is strong (hull above 60 %, not recently hit), and leaves it alone when
//!   hunted (pigment shows it to everyone).
//! - **Events.** Drones and mimics are shot (the Tax drone by everyone, the
//!   pizza drone by everyone but its recipient, the survey drone by bots
//!   near the leader: the column a shot passes is the leader's rating, so
//!   where a bot shoots from picks the star); the Cat paw's sweep and the
//!   Duck parade are dodged; a bot holds still for its pizza when it needs
//!   hull; falling and scattered hats are collected.

use ls_game::ls_net::mode::NONE;
use ls_game::ls_net::{mood_phase, NetEntity, NetMoodView};
use ls_game::ls_sim::EventKind;

use super::*;

/// Drone roles (`state[0]` of a `ReplicatedKind::Drone`).
pub mod drone_role {
    pub const TAX: u8 = 0;
    pub const PIZZA: u8 = 1;
    pub const SURVEY: u8 = 2;
}

/// Event props within this range (u) are shot at.
const PROP_RANGE: f32 = 40.0;
/// Hats within this range (u) are collected.
const HAT_RANGE: f32 = 35.0;
/// Pigment pickups within this range (u) are fetched.
const PIGMENT_RANGE: f32 = 50.0;
/// Ducks closer than this (u, centre to centre) push the bot away.
const DUCK_KEEP: f32 = 9.0;
/// Dynamic well stones closer than this (u) push the bot away.
const WELL_STONE_KEEP: f32 = 4.0;
/// Danger radius around the Cat paw (u).
const PAW_ZONE: f32 = 40.0;
/// Danger radius around a cave-in warning (u) and how long it holds
/// (ticks: the 1.5 s warning plus the collapse).
const CAVE_IN_ZONE: f32 = 9.0;
const CAVE_IN_TICKS: u64 = 150;
/// Counter-thrust on an idle axis in a well (ship-local input; one wire
/// step is 1/127).
const COUNTER_THRUST: f32 = 2.0 / 127.0;

/// The comedy director's replicated state as far as the bots care.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoodSense {
    /// The running (or announced) event.
    pub event: Option<EventKind>,
    /// `ls_net::mood_phase`.
    pub phase: u8,
    /// Wire id of the player the event aims at.
    pub target: Option<u8>,
    /// Tick the current phase ends.
    pub phase_end_tick: u64,
}

impl Default for MoodSense {
    fn default() -> Self {
        MoodSense {
            event: None,
            phase: mood_phase::ANNOUNCE,
            target: None,
            phase_end_tick: 0,
        }
    }
}

impl MoodSense {
    pub fn from_net(m: &NetMoodView) -> MoodSense {
        MoodSense {
            event: (m.event != NONE)
                .then(|| EventKind::from_index(m.event))
                .flatten(),
            phase: m.phase,
            target: (m.target != NONE).then_some(m.target),
            phase_end_tick: u64::from(m.phase_end_tick),
        }
    }

    /// Whether `kind` is announced or running.
    pub fn is(&self, kind: EventKind) -> bool {
        self.event == Some(kind)
    }

    /// The pizza's recipient while a Pizza delivery runs.
    pub fn pizza_target(&self) -> Option<u8> {
        self.target.filter(|_| self.is(EventKind::PizzaDelivery))
    }
}

/// An event prop, dynamic well or pigment pickup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropSeen {
    pub id: u16,
    pub kind: ReplicatedKind,
    pub pos: Vec3,
    pub vel: Vec3,
    pub owner: u8,
    pub state: [u8; 4],
}

impl PropSeen {
    /// The props among a snapshot's entities.
    pub fn from_entities(entities: &[NetEntity]) -> Vec<PropSeen> {
        entities
            .iter()
            .filter_map(|e| {
                let kind = ReplicatedKind::from_u8(e.kind)?;
                matches!(
                    kind,
                    ReplicatedKind::PigmentPickup
                        | ReplicatedKind::Drone
                        | ReplicatedKind::Duck
                        | ReplicatedKind::Hat
                        | ReplicatedKind::CatPaw
                        | ReplicatedKind::GossipFace
                        | ReplicatedKind::DiscoBall
                        | ReplicatedKind::Mimic
                        | ReplicatedKind::Well
                )
                .then_some(PropSeen {
                    id: e.id,
                    kind,
                    pos: e.pos,
                    vel: e.vel,
                    owner: e.owner,
                    state: e.state,
                })
            })
            .collect()
    }
}

impl Brain {
    /// Per tick, before planning: dangers from events (the Cat paw).
    pub(super) fn observe_moods(&mut self, p: &Perception) {
        if p.mood.is(EventKind::CatPaw) && p.mood.phase != mood_phase::ENDING {
            if let Some(paw) = p.props.iter().find(|q| q.kind == ReplicatedKind::CatPaw) {
                let until = p.mood.phase_end_tick.max(p.tick + 30);
                self.set_danger(paw.pos, PAW_ZONE, until);
            }
        }
    }

    /// A cave-in warning cue at `pos` (`now` in input ticks).
    pub(super) fn on_cave_in_warning(&mut self, pos: Vec3, now: u64) {
        self.set_danger(pos, CAVE_IN_ZONE, now + CAVE_IN_TICKS);
    }

    /// An event prop worth shooting, as a contact (id 255), or `None`.
    pub(super) fn prop_target<T: TerrainQuery + ?Sized>(
        &self,
        p: &Perception,
        terrain: &T,
    ) -> Option<Contact> {
        let shootable = |q: &PropSeen| match q.kind {
            ReplicatedKind::Drone => match q.state[0] {
                drone_role::TAX => true,
                // Not the own pizza (its shots pass through anyway).
                drone_role::PIZZA => p.mood.pizza_target() != Some(p.id),
                // The survey drone sits in front of the leader: rate it
                // from where the bot is (the leader's own shots hit the
                // middle column).
                drone_role::SURVEY => self.rates_surveys || p.mood.target == Some(p.id),
                _ => false,
            },
            ReplicatedKind::Mimic => true,
            _ => false,
        };
        p.props
            .iter()
            .filter(|q| shootable(q))
            .filter(|q| q.pos.distance(p.pos) < PROP_RANGE)
            .filter(|q| clear_shot(terrain, p.pos, q.pos, 0.0))
            .min_by(|a, b| {
                a.pos
                    .distance_squared(p.pos)
                    .total_cmp(&b.pos.distance_squared(p.pos))
            })
            .map(|q| Contact {
                id: u8::MAX,
                team: u8::MAX,
                pos: q.pos,
                vel: q.vel,
            })
    }

    /// Fetches the nearest hat (Hat rain) or, on a monochrome level, a
    /// pigment pickup when the bot is strong. False if there is none.
    pub(super) fn plan_mood_collect<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) -> bool {
        let hull_frac = p.hull / self.tuning.hull_max.max(1.0);
        let recently_hit = self.damaged_tick.is_some_and(|t| p.tick < t + 180);
        let strong = hull_frac > 0.6 && !recently_hit && !p.pigmented;
        let near = |q: &&PropSeen| match q.kind {
            ReplicatedKind::Hat => q.pos.distance(p.pos) < HAT_RANGE,
            // `state[0]`: a splash (dropped on death) or a pickup point.
            ReplicatedKind::PigmentPickup => strong && q.pos.distance(p.pos) < PIGMENT_RANGE,
            _ => false,
        };
        let Some(q) = p.props.iter().filter(near).min_by(|a, b| {
            a.pos
                .distance_squared(p.pos)
                .total_cmp(&b.pos.distance_squared(p.pos))
        }) else {
            return false;
        };
        let cruise = self.tuning.cruise_speed;
        let dist = q.pos.distance(p.pos);
        if dist < 6.0
            || self
                .plan_travel(p, terrain, q.pos, cruise, false, plan)
                .is_none()
        {
            plan.desired_vel = arrive(p.pos, q.pos, cruise, 3.0);
            plan.goal_dist = dist;
            plan.clearance = Some(0.8);
        }
        match q.kind {
            ReplicatedKind::Hat => counters.moods.hat_ticks += 1,
            _ => counters.moods.pigment_ticks += 1,
        }
        true
    }

    /// Holds still for the own pizza when missing hull. False otherwise.
    pub(super) fn plan_pizza(
        &mut self,
        p: &Perception,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) -> bool {
        if p.mood.pizza_target() != Some(p.id)
            || p.mood.phase != mood_phase::ACTIVE
            || p.hull > 0.9 * self.tuning.hull_max
        {
            return false;
        }
        plan.desired_vel = Vec3::ZERO;
        plan.goal_dist = 0.0;
        counters.moods.pizza_hold_ticks += 1;
        true
    }

    /// Bends the plan away from ducks and dynamic well stones close by.
    pub(super) fn dodge_props(
        &mut self,
        p: &Perception,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) {
        let cruise = self.tuning.cruise_speed;
        let mut push = Vec3::ZERO;
        let mut ducks = false;
        for q in &p.props {
            let keep = match q.kind {
                ReplicatedKind::Duck => DUCK_KEEP,
                ReplicatedKind::Well => WELL_STONE_KEEP,
                _ => continue,
            };
            // Where it will be shortly.
            let d = p.pos - (q.pos + q.vel * 0.3);
            let dist = d.length();
            if dist >= keep {
                continue;
            }
            let away = d.try_normalize().unwrap_or(Vec3::Y);
            push += away * (cruise * 1.5 * (1.0 - dist / keep));
            ducks |= q.kind == ReplicatedKind::Duck;
        }
        if push != Vec3::ZERO {
            plan.desired_vel = (plan.desired_vel + push).clamp_length(self.tuning.boost_speed);
            plan.goal_dist = f32::INFINITY;
            if ducks {
                counters.moods.duck_dodge_ticks += 1;
            }
        }
    }

    /// Counter-thrust against a well's pull on axes without input.
    pub(super) fn counter_wells(
        &self,
        p: &Perception,
        cmd: &mut InputCmd,
        counters: &mut BrainCounters,
    ) {
        if p.well_pull.length() < 0.25 {
            return;
        }
        counters.moods.well_ticks += 1;
        let g = p.orientation.conjugate().rotate(p.well_pull);
        let fix = |input: &mut f32, g: f32| {
            if *input == 0.0 && g.abs() > 0.1 {
                *input = -g.signum() * COUNTER_THRUST;
            }
        };
        fix(&mut cmd.thrust.x, g.x);
        fix(&mut cmd.thrust.y, g.y);
        fix(&mut cmd.thrust.z, g.z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mood_sense_reads_the_event_and_target() {
        let m = NetMoodView {
            event: EventKind::PizzaDelivery.index(),
            phase: mood_phase::ACTIVE,
            target: 3,
            ..NetMoodView::default()
        };
        let s = MoodSense::from_net(&m);
        assert!(s.is(EventKind::PizzaDelivery));
        assert_eq!(s.pizza_target(), Some(3));
        let none = MoodSense::from_net(&NetMoodView::default());
        assert_eq!(none.event, None);
        assert_eq!(none.pizza_target(), None);
    }
}
