//! Heist: mine the core out of its vein deep in the level and carry it to an
//! extraction zone; the carrier cannot boost. 2 teams, or free-for-all for
//! few players (`teams: 0`).
//!
//! Settings: `ruleset::HeistRules`. Design: `docs/milestone-3-plan.md` §3.2.
//!
//! Status: implemented.
//!
//! The level ([`HeistLevel`], from the generator's Heist site via
//! `MatchSetup::heist`): the core sits in a knot of rock (the vein) inside
//! a chamber at the bottom of the level; one extraction zone per team, at
//! the level's surface (top) or, on fully enclosed levels, in the team's
//! spawn cavern.
//!
//! Rules:
//! - **Mining**: the vein knot is hard rock (`MATERIAL_HARD`, smaller
//!   craters). Any weapon's craters count: the core is free once a hole
//!   reaches its centre ([`FREE_CLEARANCE`]).
//! - **Collapsing mine**: when the core comes free, the chamber region
//!   (chamber radius + [`WEAK_MARGIN`]) turns to weak rock and the chamber
//!   sheds a piece of its ceiling (a small crater and a debris chunk) every
//!   [`SHED_INTERVAL_TICKS`] for `collapse_secs`.
//! - **Carrying**: a free core is taken by touch (lowest player id first).
//!   The carrier (`player_flags::CARRIER`, the core marker's `holder`, both
//!   replicated to everyone) cannot boost and has `carrier_speed_percent` of
//!   its top speed ([`ModeModule::move_limits`]).
//! - **Drop and return**: a carrier that dies (or leaves) drops the core
//!   where it was; it stays loose (and can be taken) for `drop_return_secs`,
//!   then returns to the vein.
//! - **Delivery**: the carrier inside its own team's extraction zone (any
//!   zone in free-for-all) scores 1 for itself and its team; the core
//!   respawns in the vein after `core_respawn_secs`, embedded again (the
//!   knot grows back once no ship is in the way). First to
//!   `deliveries_to_win` or the most deliveries when the timer runs out
//!   wins (a tie is a draw). Kills do not score.
//!
//! State: objective 0 is the core ([`objective_kind::CORE`], `state` one of
//! [`core_state`], `timer_end` the return or respawn tick, `vars[0..3]` the
//! vein position); objectives 1 and 2 are the extraction zones
//! ([`objective_kind::EXTRACTION`], `team` = owner, `NO_SIDE` in
//! free-for-all). `ModeState::vars`: `[0]` collapse end tick, `[1]` knot
//! radius and `[2]` chamber radius (f32 bits).

use ls_math::terrain::{MATERIAL_HARD, MATERIAL_WEAK};
use ls_math::Vec3;
use serde::{Deserialize, Serialize};

use crate::mode::{
    objective_kind, player_flags, MatchPhase, ModeKind, ModeState, Objective, NO_SIDE,
};
use crate::modes::{ticks, Kill, ModeCtx, ModeModule, ModeMut, MoveLimits};
use crate::ruleset::{HeistRules, ModeRules};
use crate::ship::PlayerId;

/// Radius of the core (u; replicated in its marker).
pub const CORE_RADIUS: f32 = 0.8;
/// The core is free once its centre is in open space: the terrain SDF there
/// is above this (u). A hole dug to it is enough.
pub const FREE_CLEARANCE: f32 = 0.0;
/// A ship takes a free core within its radius + [`CORE_RADIUS`] + this (u):
/// enough to reach it from the mouth of a hole dug through the knot.
pub const TOUCH_MARGIN: f32 = 1.5;
/// The weak (collapsing) region reaches this far beyond the chamber (u).
pub const WEAK_MARGIN: f32 = 3.0;
/// Margin around the knot that counts as vein rock (hard) for craters (u).
pub const VEIN_MARGIN: f32 = 0.5;
/// The collapsing chamber sheds a piece of ceiling this often (ticks).
pub const SHED_INTERVAL_TICKS: u64 = 30;
/// Radius of the crater a shed piece leaves (u, exact).
pub const SHED_CRATER_RADIUS: f32 = 1.0;
/// Radius of a shed debris chunk (u).
pub const SHED_DEBRIS_RADIUS: f32 = 0.6;
/// Scrap in a shed debris chunk.
pub const SHED_DEBRIS_SCRAP: f32 = 2.0;
/// Fall speed of a shed debris chunk (u/s).
pub const SHED_DEBRIS_SPEED: f32 = 4.0;
/// The knot grows back only when no ship is within knot radius + this (u).
pub const REGROW_CLEARANCE: f32 = 2.0;
/// Radius of the fallback extraction zones (levels without a Heist site).
pub const FALLBACK_ZONE_RADIUS: f32 = 6.0;
/// Key salt of the collapse's private RNG ("HEISTCOL").
const SHED_RNG_KEY: u64 = 0x4845_4953_5443_4f4c;

/// Objective id of the core.
pub const CORE_ID: u8 = 0;

/// Values of the core marker's `state` byte (wire-stable).
pub mod core_state {
    /// In the vein, inside rock.
    pub const EMBEDDED: u8 = 0;
    /// In the vein, mined free: take it by touch.
    pub const FREE: u8 = 1;
    /// Carried (`holder`).
    pub const CARRIED: u8 = 2;
    /// Dropped by a dead carrier: loose until `timer_end`, then back to the vein.
    pub const DROPPED: u8 = 3;
    /// Delivered: back in the vein at `timer_end`.
    pub const RESPAWNING: u8 = 4;
}

/// The level's Heist site as the simulation sees it (`MatchSetup::heist`;
/// `ls_game::setup` fills it from the generator's `ls_worldgen::HeistSite`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeistLevel {
    /// The core's position: the centre of the vein knot.
    pub core: Vec3,
    /// Radius of the rock knot around the core (0: no rock, the core starts free).
    pub knot_radius: f32,
    /// Radius of the open chamber around the knot.
    pub chamber_radius: f32,
    /// Extraction zones (centre, radius), team 0 then team 1.
    pub extraction: [(Vec3, f32); 2],
}

/// The Heist mode module.
#[derive(Debug)]
pub struct Heist;

fn rules(r: &ModeRules) -> HeistRules {
    match r {
        ModeRules::Heist(h) => *h,
        _ => HeistRules::default(),
    }
}

fn f32_var(v: i64) -> f32 {
    f32::from_bits(v as u32)
}

fn var_f32(f: f32) -> i64 {
    f.to_bits() as i64
}

/// The vein position (stored in the core's scratch values).
fn vein(core: &Objective) -> Vec3 {
    Vec3::new(core.vars[0], core.vars[1], core.vars[2])
}

/// The level of a match without a generated Heist site (test rooms, tiny
/// levels): the core lies free at the centre of the spawn points, each
/// team's zone at the centre of its spawns.
fn fallback_level(m: &ModeCtx) -> HeistLevel {
    let spawns = &m.setup.spawns;
    let centre = |team: Option<u8>| {
        let pts: Vec<Vec3> = spawns
            .iter()
            .filter(|s| team.is_none_or(|t| s.team == t))
            .map(|s| s.pos)
            .collect();
        if pts.is_empty() {
            None
        } else {
            Some(pts.iter().fold(Vec3::ZERO, |a, p| a + *p) / pts.len() as f32)
        }
    };
    let all = centre(None).unwrap_or(Vec3::ZERO);
    let zone = |t: u8| (centre(Some(t)).unwrap_or(all), FALLBACK_ZONE_RADIUS);
    HeistLevel {
        core: all,
        knot_radius: 0.0,
        chamber_radius: 0.0,
        extraction: [zone(0), zone(1)],
    }
}

/// Sets or clears a player's carrier flag.
fn set_carrier(state: &mut ModeState, player: PlayerId, on: bool) {
    if let Some(p) = state.players.get_mut(&player) {
        if on {
            p.flags |= player_flags::CARRIER;
        } else {
            p.flags &= !player_flags::CARRIER;
        }
    }
}

/// Puts the core loose at `pos` (its carrier died or left).
fn drop_core(state: &mut ModeState, pos: Vec3, return_tick: u64) {
    let Some(core) = state.objective_mut(CORE_ID) else {
        return;
    };
    let Some(holder) = core.holder.take() else {
        return;
    };
    core.state = core_state::DROPPED;
    core.pos = pos;
    core.timer_end = return_tick;
    core.team = NO_SIDE;
    set_carrier(state, holder, false);
}

impl Heist {
    /// The core is free when the terrain is open around it.
    fn is_free(m: &ModeCtx, pos: Vec3) -> bool {
        m.ctx.view.terrain.sdf(pos) > FREE_CLEARANCE
    }

    /// A delivery by `carrier` (on side `side`): scores, then the core waits
    /// in the vein for its respawn.
    fn deliver(m: &mut ModeCtx, carrier: PlayerId, side: u8, respawn_tick: u64) {
        if let Some(p) = m.state.players.get_mut(&carrier) {
            p.score += 1;
        }
        if (side as usize) < m.state.teams as usize {
            m.state.team_scores[side as usize] += 1;
        }
        set_carrier(m.state, carrier, false);
        if let Some(core) = m.state.objective_mut(CORE_ID) {
            core.holder = None;
            core.team = NO_SIDE;
            core.state = core_state::RESPAWNING;
            core.timer_end = respawn_tick;
            core.pos = vein(core);
        }
    }

    /// The collapse: one piece of the chamber ceiling above the vein falls
    /// (a small crater where it was, a debris chunk falling from there).
    fn shed(m: &mut ModeCtx, vein: Vec3, chamber: f32) {
        let mut rng = m.ctx.rng(SHED_RNG_KEY);
        let dir =
            Vec3::new(rng.next_f32() * 2.0 - 1.0, 1.0, rng.next_f32() * 2.0 - 1.0).normalize();
        let Some(hit) = m.ctx.raycast(vein, dir, 4.0 * chamber + 8.0) else {
            return;
        };
        m.ctx.crater_exact(hit.point, SHED_CRATER_RADIUS);
        let pos = hit.point - dir * (SHED_CRATER_RADIUS + SHED_DEBRIS_RADIUS);
        m.ctx.spawn_debris(
            pos,
            Vec3::new(0.0, -SHED_DEBRIS_SPEED, 0.0),
            SHED_DEBRIS_RADIUS,
            SHED_DEBRIS_SCRAP,
        );
    }
}

impl ModeModule for Heist {
    fn kind(&self) -> ModeKind {
        ModeKind::Heist
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0, 2]
    }

    fn validate(&self, r: &ModeRules) -> Result<(), String> {
        let h = rules(r);
        if h.deliveries_to_win == 0 {
            return Err("Heist: deliveries_to_win must be at least 1".into());
        }
        if !(1..=100).contains(&h.carrier_speed_percent) {
            return Err("Heist: carrier_speed_percent must be 1..=100".into());
        }
        for (name, v) in [
            ("drop_return_secs", h.drop_return_secs),
            ("core_respawn_secs", h.core_respawn_secs),
            ("collapse_secs", h.collapse_secs),
        ] {
            if !(v.is_finite() && v >= 0.0) {
                return Err(format!("Heist: {name} must be 0 or more"));
            }
        }
        Ok(())
    }

    /// Places the core in its vein and the extraction zones.
    fn on_start(&self, m: &mut ModeCtx) {
        let level = m.setup.heist.unwrap_or_else(|| fallback_level(m));
        let h = rules(m.rules);
        m.state.score_target = h.deliveries_to_win as u32;
        m.state.vars[1] = var_f32(level.knot_radius);
        m.state.vars[2] = var_f32(level.chamber_radius);
        let mut core = Objective::new(CORE_ID, objective_kind::CORE, level.core);
        core.radius = CORE_RADIUS;
        core.vars = [level.core.x, level.core.y, level.core.z, 0.0];
        core.state = core_state::EMBEDDED;
        m.state.objectives.clear();
        m.state.objectives.push(core);
        let ffa = m.state.free_for_all();
        for (team, (pos, radius)) in level.extraction.iter().enumerate() {
            let mut zone = Objective::new(1 + team as u8, objective_kind::EXTRACTION, *pos);
            zone.radius = *radius;
            zone.team = if ffa { NO_SIDE } else { team as u8 };
            m.state.objectives.push(zone);
        }
    }

    /// Kills do not score; a dying carrier drops the core.
    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        let carried = m
            .state
            .objective(CORE_ID)
            .is_some_and(|c| c.holder == Some(kill.victim));
        if carried {
            let pos = m.ships.get(&kill.victim).map_or(Vec3::ZERO, |s| s.pos);
            let back = m.now + ticks(rules(m.rules).drop_return_secs);
            drop_core(m.state, pos, back);
        }
    }

    fn on_tick(&self, m: &mut ModeCtx) {
        if m.state.phase != MatchPhase::Running {
            return;
        }
        let h = rules(m.rules);
        let now = m.now();
        let Some(core) = m.state.objective(CORE_ID).copied() else {
            return;
        };
        let vein_pos = vein(&core);
        let knot = f32_var(m.state.vars[1]);
        let chamber = f32_var(m.state.vars[2]);
        let mut state = core.state;

        // Back to the vein after a drop.
        if state == core_state::DROPPED && now >= core.timer_end {
            state = if knot <= 0.0 || Self::is_free(m, vein_pos) {
                core_state::FREE
            } else {
                core_state::EMBEDDED
            };
            if let Some(c) = m.state.objective_mut(CORE_ID) {
                c.state = state;
                c.pos = vein_pos;
                c.timer_end = 0;
            }
        }

        // Respawn after a delivery: the knot grows back once nobody is in it.
        if state == core_state::RESPAWNING && now >= core.timer_end {
            let blocked = knot > 0.0
                && m.ctx
                    .view
                    .ships
                    .values()
                    .any(|s| s.alive && s.pos.distance(vein_pos) < knot + REGROW_CLEARANCE);
            if !blocked {
                if knot > 0.0 {
                    m.ctx.grow(vein_pos, knot);
                    state = core_state::EMBEDDED;
                } else {
                    state = core_state::FREE;
                }
                if let Some(c) = m.state.objective_mut(CORE_ID) {
                    c.state = state;
                    c.timer_end = 0;
                }
            }
        }

        // Mined free: the mine starts collapsing. (The knot grown back this
        // tick is not in the terrain yet; it is checked next tick.)
        if state == core_state::EMBEDDED
            && core.state == core_state::EMBEDDED
            && (knot <= 0.0 || Self::is_free(m, vein_pos))
        {
            state = core_state::FREE;
            if let Some(c) = m.state.objective_mut(CORE_ID) {
                c.state = state;
            }
            if knot > 0.0 {
                m.state.vars[0] = (now + ticks(h.collapse_secs)) as i64;
            }
        }

        // Taken by touch.
        if state == core_state::FREE || state == core_state::DROPPED {
            let pos = m.state.objective(CORE_ID).map_or(vein_pos, |c| c.pos);
            let taker = m.ctx.view.ships.values().find(|s| {
                let out = m.state.player(s.player).is_none_or(|p| p.is_out());
                let reach =
                    m.ctx.view.ship_types.get(s.ship_type).radius + CORE_RADIUS + TOUCH_MARGIN;
                s.alive && !out && s.pos.distance(pos) <= reach
            });
            if let Some(s) = taker {
                let (id, side, ship_pos) = (s.player, s.team, s.pos);
                if let Some(c) = m.state.objective_mut(CORE_ID) {
                    c.state = core_state::CARRIED;
                    c.holder = Some(id);
                    c.team = side;
                    c.timer_end = 0;
                    c.pos = ship_pos;
                }
                set_carrier(m.state, id, true);
                state = core_state::CARRIED;
            }
        }

        // Carried: follows the carrier; delivered in an extraction zone.
        if state == core_state::CARRIED {
            let holder = m.state.objective(CORE_ID).and_then(|c| c.holder);
            let ship = holder
                .and_then(|id| m.ctx.view.ships.get(&id))
                .filter(|s| s.alive);
            match ship {
                None => {
                    // The carrier left (or died outside `on_kill`).
                    let pos = m.state.objective(CORE_ID).map_or(vein_pos, |c| c.pos);
                    drop_core(m.state, pos, now + ticks(h.drop_return_secs));
                }
                Some(s) => {
                    let (id, side, pos) = (s.player, s.team, s.pos);
                    if let Some(c) = m.state.objective_mut(CORE_ID) {
                        c.pos = pos;
                    }
                    let ffa = m.state.free_for_all();
                    let home = m.state.objectives.iter().any(|z| {
                        z.kind == objective_kind::EXTRACTION
                            && (ffa || z.team == side)
                            && pos.distance(z.pos) <= z.radius
                    });
                    if home {
                        Self::deliver(m, id, side, now + ticks(h.core_respawn_secs));
                    }
                }
            }
        }

        // The collapsing mine sheds its ceiling.
        let collapse_end = m.state.vars[0] as u64;
        if now < collapse_end && now % SHED_INTERVAL_TICKS == 0 {
            Self::shed(m, vein_pos, chamber.max(knot));
        }
    }

    fn move_limits(&self, r: &ModeRules, flags: u8) -> MoveLimits {
        if flags & player_flags::CARRIER != 0 {
            MoveLimits {
                speed_scale: rules(r).carrier_speed_percent as f32 / 100.0,
                no_boost: true,
            }
        } else {
            MoveLimits::NONE
        }
    }

    /// The vein knot is hard rock; while the mine collapses, the chamber
    /// region is weak rock.
    fn material_at(&self, state: &ModeState, _r: &ModeRules, now: u64, p: Vec3) -> Option<u8> {
        let core = state.objective(CORE_ID)?;
        if core.kind != objective_kind::CORE {
            return None;
        }
        let d = p.distance(vein(core));
        let knot = f32_var(state.vars[1]);
        let chamber = f32_var(state.vars[2]);
        if now < state.vars[0] as u64 && d <= chamber.max(knot) + WEAK_MARGIN {
            Some(MATERIAL_WEAK)
        } else if knot > 0.0 && d <= knot + VEIN_MARGIN {
            Some(MATERIAL_HARD)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests;
