//! Mode objectives of the [`Brain`] (M3 package C2), layered on top of the
//! hunt / fight behaviour. What the bot knows about the mode comes from the
//! replicated mode state ([`ModeSense::from_mode`]):
//!
//! - **Spectating**: an eliminated player (no lives or colour left) or a
//!   late joiner of an elimination mode has no ship; the bot idles in
//!   [`Mode::Spectate`] until the next match.
//! - **The Squeeze**: stay inside the safe zone. The zone to keep is the
//!   next zone once announced (it lies inside the current one, so it is
//!   safe through the whole close), else the current one. Outside its
//!   comfort radius the bot flies (boosting) towards the centre; wander
//!   points and pickups outside the zone are ignored.
//! - **Heist**: the carrier flies the core home to its extraction zone (any
//!   zone in free-for-all) and only shoots on the way; a free or dropped
//!   core is fetched by touch; an embedded core is dug out with crater
//!   ordnance (Bolt Rocket, Quicksilver, Heavy Rocket) from a few units
//!   away: about half the bots are miners that head for the core even
//!   under fire, the rest dig when no enemy is in sight; once at the core
//!   every bot keeps digging (the target waits). An enemy carrier is the
//!   preferred target and is hunted down; team mates escort their own
//!   carrier.
//! - **Hot Reactor**: the holder rams the nearest ship (touching passes the
//!   reactor); everyone else keeps away from the holder and never shoots
//!   it (a hit from you makes you the holder).
//! - **Last Color Standing**: targets and hunts are weighted towards the
//!   most colourful players (the leaders).

use ls_game::ls_net::mode::NONE;
use ls_game::ls_net::{mode_player_flags, NetModeState};
use ls_game::ls_sim::mode::objective_kind;
use ls_game::ls_sim::modes::{heist::core_state, hot_reactor, the_squeeze};
use ls_game::ls_sim::ModeKind;

use super::*;

/// The core's knot of hard rock is about this big (u); bots dig from
/// [`DIG_STANDOFF`] away from its centre.
const KNOT_RADIUS: f32 = 2.0;
/// Distance from the core's centre the diggers hold (u): the Bolt Rocket's
/// 2.5 u splash on the knot's surface stays clear of the ship.
const DIG_STANDOFF: f32 = 6.5;
/// A rock hit counts as hitting the knot when it lands this close to the
/// core's centre (u).
const DIG_HIT_RADIUS: f32 = KNOT_RADIUS + 2.0;
/// Escorts stay about this far from their carrier (u).
const ESCORT_DISTANCE: f32 = 8.0;
/// Non-holders keep at least this far beyond the fuse radius from the
/// reactor holder (u).
const REACTOR_FLEE_MARGIN: f32 = 12.0;
/// The holder rams with this lead on the target's velocity (s).
const RAM_LEAD_SECS: f32 = 0.3;

/// A safe sphere (The Squeeze).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneSense {
    pub centre: Vec3,
    pub radius: f32,
}

impl ZoneSense {
    /// Distance from the centre the bot keeps within (u).
    pub fn comfort(&self) -> f32 {
        let r = self.radius;
        (r - (0.25 * r).clamp(4.0, 25.0)).max(0.4 * r)
    }

    pub fn contains(&self, p: Vec3, margin: f32) -> bool {
        p.distance(self.centre) <= (self.radius - margin).max(0.0)
    }
}

/// The Heist core.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoreSense {
    pub pos: Vec3,
    /// `ls_sim::modes::heist::core_state`.
    pub state: u8,
    /// Wire id of the carrier.
    pub holder: Option<u8>,
    /// Side of the carrier.
    pub holder_side: Option<u8>,
}

/// A Heist extraction zone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtractionSense {
    pub pos: Vec3,
    pub radius: f32,
    /// Owning team (`None`: any side may deliver there).
    pub team: Option<u8>,
}

/// The Hot Reactor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReactorSense {
    /// Wire id of the holder (`None` while the next one is pending).
    pub holder: Option<u8>,
    /// Fuse radius (u): other ships this close speed the fuse up.
    pub radius: f32,
}

/// What the bot knows about the mode (from the newest mode state).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModeSense {
    /// `None` before the first mode state (or for an unknown mode).
    pub kind: Option<ModeKind>,
    pub running: bool,
    /// Number of teams (0 = free-for-all).
    pub teams: u8,
    /// This player is out of the match (eliminated, or a spectator).
    pub out: bool,
    /// This player carries the objective (Heist core, Hot Reactor).
    pub carrier: bool,
    pub zone: Option<ZoneSense>,
    pub core: Option<CoreSense>,
    pub extraction: Vec<ExtractionSense>,
    pub reactor: Option<ReactorSense>,
    /// Colour meters (wire id, meter) in Last Color Standing.
    pub meters: Vec<(u8, u8)>,
}

impl ModeSense {
    /// Reads the replicated mode state for player `me`.
    pub fn from_mode(m: &NetModeState, me: u8) -> ModeSense {
        let mut s = ModeSense {
            kind: ModeKind::from_index(m.kind),
            running: m.is_running(),
            teams: m.teams,
            ..ModeSense::default()
        };
        if m == &NetModeState::default() {
            s.kind = None;
            return s;
        }
        if let Some(own) = m.player(me) {
            s.out = own.flags & (mode_player_flags::ELIMINATED | mode_player_flags::SPECTATOR) != 0;
            s.carrier = own.flags & mode_player_flags::CARRIER != 0;
        }
        let id = |v: u8| (v != NONE).then_some(v);
        match s.kind {
            Some(ModeKind::TheSqueeze) => {
                let zone = |kind: u8| {
                    m.objectives
                        .iter()
                        .find(|o| o.kind == kind && o.radius > 0.0)
                        .map(|o| ZoneSense {
                            centre: the_squeeze::snap(o.pos),
                            radius: o.radius,
                        })
                };
                s.zone = zone(objective_kind::NEXT_ZONE).or(zone(objective_kind::SAFE_ZONE));
            }
            Some(ModeKind::Heist) => {
                for o in &m.objectives {
                    match o.kind {
                        objective_kind::CORE => {
                            s.core = Some(CoreSense {
                                pos: o.pos,
                                state: o.state,
                                holder: id(o.holder),
                                holder_side: id(o.holder).and_then(|_| id(o.team)),
                            })
                        }
                        objective_kind::EXTRACTION => s.extraction.push(ExtractionSense {
                            pos: o.pos,
                            radius: o.radius,
                            team: id(o.team),
                        }),
                        _ => {}
                    }
                }
            }
            Some(ModeKind::HotReactor) => {
                s.reactor = m
                    .objectives
                    .iter()
                    .find(|o| o.kind == objective_kind::REACTOR)
                    .map(|o| ReactorSense {
                        holder: (o.state == hot_reactor::STATE_HELD)
                            .then(|| id(o.holder))
                            .flatten(),
                        radius: o.radius,
                    });
            }
            Some(ModeKind::LastColorStanding) => {
                s.meters = m.players.iter().map(|q| (q.id, q.meter)).collect();
            }
            _ => {}
        }
        s
    }

    /// The colour meter of player `id` (Last Color Standing).
    pub fn meter(&self, id: u8) -> Option<u8> {
        self.meters.iter().find(|(q, _)| *q == id).map(|(_, m)| *m)
    }

    /// Wire id of the reactor holder.
    pub fn reactor_holder(&self) -> Option<u8> {
        self.reactor.and_then(|r| r.holder)
    }
}

impl Perception {
    /// Whether contact `c` is fair game: on another side, and not the Hot
    /// Reactor's holder (shooting it would hand the reactor over), unless
    /// this bot holds the reactor itself.
    pub fn is_hostile(&self, c: &Contact) -> bool {
        if c.team == self.team {
            return false;
        }
        match self.mode.reactor_holder() {
            Some(h) => h != c.id || h == self.id,
            None => true,
        }
    }

    /// Distance to contact `c` as the bot weighs targets: the Heist's enemy
    /// carrier counts as much closer, and in Last Color Standing colourful
    /// players count as closer than drained ones.
    pub fn target_weight(&self, c: &Contact) -> f32 {
        let d = c.pos.distance(self.pos);
        if let Some(core) = self.mode.core {
            if core.holder == Some(c.id) {
                return d * 0.35;
            }
        }
        match self.mode.meter(c.id) {
            Some(m) => d * (2.0 - f32::from(m.min(100)) / 100.0),
            None => d,
        }
    }
}

impl Brain {
    /// Squeeze: flies back towards the zone's centre while outside its
    /// comfort radius (until well inside again). False if the bot is fine
    /// where it is.
    pub(super) fn plan_zone<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) -> bool {
        let Some(zone) = p.mode.zone.filter(|_| p.mode.running) else {
            self.zone_return = false;
            return false;
        };
        let comfort = zone.comfort();
        let d = p.pos.distance(zone.centre);
        // Hysteresis: once heading back, go to half the comfort radius.
        let limit = if self.zone_return {
            0.5 * comfort
        } else {
            comfort
        };
        self.zone_return = d > limit;
        if !self.zone_return {
            return false;
        }
        let t = &self.tuning;
        let (cruise, boost) = (t.cruise_speed, t.boost_speed);
        let routed = self
            .plan_travel(p, terrain, zone.centre, cruise, true, plan)
            .is_some();
        if !routed {
            plan.desired_vel = arrive(p.pos, zone.centre, boost, 6.0);
            plan.goal_dist = d;
            plan.boost = d > 20.0 && p.boost > 0.0;
        }
        true
    }

    /// Hot Reactor movement: the holder rams the nearest ship, the others
    /// keep away from the holder. False when neither applies.
    pub(super) fn plan_reactor<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        plan: &mut Plan,
    ) -> bool {
        let Some(r) = p.mode.reactor.filter(|_| p.mode.running) else {
            self.reactor_flee_goal = None;
            return false;
        };
        let Some(holder) = r.holder else {
            self.reactor_flee_goal = None;
            return false;
        };
        let t = &self.tuning;
        let (cruise, boost) = (t.cruise_speed, t.boost_speed);
        if holder == p.id {
            self.reactor_flee_goal = None;
            let dist = |c: &Contact| c.pos.distance(p.pos);
            let Some(victim) = p
                .contacts
                .iter()
                .min_by(|a, b| dist(a).total_cmp(&dist(b)))
                .copied()
            else {
                return false;
            };
            let d = dist(&victim);
            let aim = victim.pos + victim.vel * RAM_LEAD_SECS;
            if d > 15.0
                && self
                    .plan_travel(p, terrain, victim.pos, cruise, true, plan)
                    .is_some()
            {
                plan.aim = Some(victim.pos);
                return true;
            }
            // Close (or no route): straight at it, as fast as possible; the
            // wall clearance is lowered so the ship gets into contact.
            plan.desired_vel = (aim - p.pos).normalize_or_zero() * boost;
            plan.goal_dist = f32::INFINITY;
            plan.boost = p.boost > 0.0;
            plan.clearance = Some(0.6);
            plan.aim = Some(aim);
            return true;
        }
        let Some(h) = p.contacts.iter().find(|c| c.id == holder).copied() else {
            self.reactor_flee_goal = None;
            return false;
        };
        let keep = r.radius + REACTOR_FLEE_MARGIN;
        let d = h.pos.distance(p.pos);
        let fleeing = self.reactor_flee_goal.is_some();
        if d > keep + if fleeing { 8.0 } else { 0.0 } {
            self.reactor_flee_goal = None;
            return false;
        }
        let goal = match self.reactor_flee_goal {
            Some(g) if g.distance(h.pos) > keep + 5.0 && g.distance(p.pos) > 4.0 => g,
            _ => {
                let d = Danger {
                    center: h.pos,
                    radius: keep + 10.0,
                    until: p.tick,
                };
                self.pick_flee_goal(p, &d)
            }
        };
        self.reactor_flee_goal = Some(goal);
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

    /// Heist movement (see the module docs). `target` is the enemy in
    /// sight, if any: digging and escorting give way to a fight, carrying
    /// and fetching a loose core do not (the bot shoots on the way). False
    /// when the bot has nothing to do for the core right now.
    pub(super) fn plan_heist<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        target: Option<&Contact>,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) -> bool {
        let Some(core) = p.mode.core.filter(|_| p.mode.running) else {
            return false;
        };
        let t = &self.tuning;
        let cruise = t.cruise_speed;
        match core.state {
            core_state::CARRIED if core.holder == Some(p.id) || p.mode.carrier => {
                let home = p
                    .mode
                    .extraction
                    .iter()
                    .filter(|z| p.mode.teams == 0 || z.team.is_none_or(|t| t == p.team))
                    .min_by(|a, b| {
                        a.pos
                            .distance_squared(p.pos)
                            .total_cmp(&b.pos.distance_squared(p.pos))
                    })
                    .copied();
                let Some(home) = home else {
                    return false;
                };
                // The carrier cannot boost.
                if self
                    .plan_travel(p, terrain, home.pos, cruise, false, plan)
                    .is_none()
                {
                    plan.desired_vel = arrive(p.pos, home.pos, cruise, 4.0);
                    plan.goal_dist = home.pos.distance(p.pos);
                }
                plan.boost = false;
                true
            }
            core_state::CARRIED => {
                let Some(carrier) = core.holder else {
                    return false;
                };
                let ours = p.mode.teams > 0 && core.holder_side == Some(p.team);
                if target.is_some() {
                    // Fight first (an enemy carrier in sight is the
                    // preferred target anyway).
                    return false;
                }
                let pos = p
                    .contacts
                    .iter()
                    .find(|c| c.id == carrier)
                    .map_or(core.pos, |c| c.pos);
                let d = pos.distance(p.pos);
                if ours && d < ESCORT_DISTANCE {
                    return false;
                }
                let boost = !ours;
                if self
                    .plan_travel(p, terrain, pos, cruise, boost, plan)
                    .is_none()
                {
                    plan.desired_vel = arrive(p.pos, pos, cruise, ESCORT_DISTANCE);
                    plan.goal_dist = d;
                }
                true
            }
            core_state::FREE | core_state::DROPPED => {
                let d = core.pos.distance(p.pos);
                if d < 6.0
                    || self
                        .plan_travel(p, terrain, core.pos, cruise, true, plan)
                        .is_none()
                {
                    plan.desired_vel = arrive(p.pos, core.pos, cruise, 4.0);
                    plan.goal_dist = d;
                    plan.clearance = Some(0.5);
                }
                true
            }
            core_state::EMBEDDED => {
                // Miners go for the core whatever happens on the way and
                // keep digging under fire; the others dig when no enemy is
                // in sight (and fight otherwise).
                let close = core.pos.distance(p.pos) < DIG_STANDOFF + 8.0;
                if target.is_some() && !(self.miner || close) {
                    return false;
                }
                self.plan_dig(p, terrain, core.pos, plan, counters)
            }
            _ => false,
        }
    }

    /// Digs the embedded core out: flies to the vein, holds
    /// [`DIG_STANDOFF`] from the core and fires crater ordnance at it. False
    /// without such ordnance (the bot then fetches pickups, fights, hunts).
    fn plan_dig<T: TerrainQuery + ?Sized>(
        &mut self,
        p: &Perception,
        terrain: &T,
        core: Vec3,
        plan: &mut Plan,
        counters: &mut BrainCounters,
    ) -> bool {
        let owned = self.owned(p);
        let digger = owned
            .iter()
            .filter(|(_, ammo)| *ammo > 0)
            .filter_map(|(w, _)| self.arsenal.gun(*w))
            .filter(|g| {
                matches!(
                    g.role,
                    Role::BoltRocket | Role::Quicksilver | Role::HeavyRocket
                )
            })
            .min_by_key(|g| match g.role {
                Role::BoltRocket => 0,
                Role::Quicksilver => 1,
                _ => 2,
            });
        let Some(gun) = digger else {
            return false;
        };
        let cruise = self.tuning.cruise_speed;
        let d = core.distance(p.pos);
        let standoff = DIG_STANDOFF.max(gun.splash_radius + KNOT_RADIUS + 1.5);
        if d > standoff + 8.0 {
            if self
                .plan_travel(p, terrain, core, cruise, true, plan)
                .is_none()
            {
                plan.desired_vel = arrive(p.pos, core, cruise, 8.0);
                plan.goal_dist = d;
            }
            return true;
        }
        // In the chamber: hold the standoff on the bot's side of the core.
        let side = (p.pos - core).try_normalize().unwrap_or(Vec3::Y);
        let spot = core + side * standoff;
        plan.desired_vel = (spot - p.pos) * 1.5;
        plan.goal_dist = spot.distance(p.pos);
        plan.clearance = Some(0.8);
        plan.aim = Some(core);
        plan.select_secondary = Some(gun.id);
        self.digging = true;
        if p.secondary != Some(gun.id) || p.tick < self.next_dig {
            return true;
        }
        let dir = core - p.pos;
        let err = angle_between(p.orientation.forward(), dir);
        let Some(hit) = terrain.raycast(p.pos, dir, d + 1.0) else {
            return true;
        };
        let safe = hit.distance >= gun.splash_radius + 1.0;
        if err < 0.05 && safe && hit.point.distance(core) <= DIG_HIT_RADIUS {
            plan.fire_secondary = true;
            counters.dig_shots += 1;
            self.next_dig = p.tick + (gun.fire_interval * TICKS_PER_SEC) as u64 + 6;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_game::ls_net::{NetModePlayer, NetObjective};

    fn state(kind: ModeKind) -> NetModeState {
        NetModeState {
            kind: kind.index(),
            ..NetModeState::default()
        }
    }

    #[test]
    fn squeeze_keeps_the_next_zone_once_announced() {
        let mut m = state(ModeKind::TheSqueeze);
        m.objectives.push(NetObjective {
            id: 0,
            kind: objective_kind::SAFE_ZONE,
            radius: 100.0,
            pos: Vec3::new(10.0, 0.0, 0.0),
            ..NetObjective::default()
        });
        let s = ModeSense::from_mode(&m, 0);
        assert_eq!(s.zone.unwrap().radius, 100.0);
        m.objectives.push(NetObjective {
            id: 1,
            kind: objective_kind::NEXT_ZONE,
            radius: 60.0,
            pos: Vec3::new(20.0, 5.0, 0.0),
            ..NetObjective::default()
        });
        let z = ModeSense::from_mode(&m, 0).zone.unwrap();
        assert_eq!((z.centre, z.radius), (Vec3::new(20.0, 5.0, 0.0), 60.0));
        assert!(z.comfort() < 60.0 && z.comfort() >= 24.0);
    }

    #[test]
    fn eliminated_and_spectating_players_are_out() {
        let mut m = state(ModeKind::LastColorStanding);
        m.players.push(NetModePlayer {
            id: 3,
            flags: mode_player_flags::ELIMINATED,
            ..NetModePlayer::default()
        });
        m.players.push(NetModePlayer {
            id: 4,
            meter: 66,
            ..NetModePlayer::default()
        });
        assert!(ModeSense::from_mode(&m, 3).out);
        let s = ModeSense::from_mode(&m, 4);
        assert!(!s.out);
        assert_eq!(s.meter(4), Some(66));
        assert!(!ModeSense::from_mode(&NetModeState::default(), 0).out);
        assert_eq!(ModeSense::from_mode(&NetModeState::default(), 0).kind, None);
    }

    #[test]
    fn heist_core_and_zones_are_read() {
        let mut m = state(ModeKind::Heist);
        m.teams = 2;
        m.objectives.push(NetObjective {
            id: 0,
            kind: objective_kind::CORE,
            holder: 5,
            team: 1,
            state: core_state::CARRIED,
            pos: Vec3::new(1.0, 2.0, 3.0),
            ..NetObjective::default()
        });
        for (i, team) in [0u8, 1].into_iter().enumerate() {
            m.objectives.push(NetObjective {
                id: 1 + i as u8,
                kind: objective_kind::EXTRACTION,
                team,
                radius: 7.0,
                ..NetObjective::default()
            });
        }
        let s = ModeSense::from_mode(&m, 0);
        let core = s.core.unwrap();
        assert_eq!((core.holder, core.holder_side), (Some(5), Some(1)));
        assert_eq!(s.extraction.len(), 2);
        assert_eq!(s.extraction[1].team, Some(1));
    }
}
