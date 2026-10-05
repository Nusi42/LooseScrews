//! Hot Reactor: pass the reactor around; whoever holds it at detonation
//! loses a life; the fuse runs faster the more ships are near it; the last
//! player with lives wins.
//!
//! Settings: `ruleset::HotReactorRules`. Design: `docs/milestone-3-plan.md`
//! §3.2 (Hot Reactor: passing, fuse).
//!
//! Rules as built:
//! - Free-for-all only. Every player has `lives` (3); a life is lost only
//!   when the reactor detonates in your hands. Other deaths cost no life and
//!   respawn on the normal timer; a kill scores 1 for the killer (the mode
//!   score breaks ties on time).
//! - The reactor (objective `objective_kind::REACTOR`, id [`REACTOR_ID`])
//!   appears on a random living player `next_reactor_secs` after the match
//!   start and after each detonation, once at least two players are in play.
//! - Passing: any hit the holder takes from another living ship (direct or
//!   splash, even one a shield absorbs) makes the shooter the holder;
//!   touching another ship (ramming: the hull spheres overlap) passes it to
//!   that ship. The holder can't drop it. A new holder can't lose it again
//!   for `pass_immunity_secs` (so it can't be passed straight back).
//! - Fuse: `fuse_secs` at 1x speed; each other ship in play within
//!   `fuse_radius` of the holder adds `fuse_speed_per_ship` (two ships near
//!   at 0.5 = 2x speed). The fuse keeps running across passes.
//!   `Objective::timer_end` is the detonation tick at the current speed.
//! - Detonation: the holder dies and loses a life (eliminated at zero), and
//!   the blast deals `blast_damage` (falling off to 0 at `blast_radius`) to
//!   every other ship; nobody gets kill credit for it.
//! - Gap decisions: a holder killed by another ship passes the reactor to
//!   the killer (even while immune); a holder dying any other way (own
//!   weapon, debris) detonates it at once (it loses the life, so suicide is
//!   no escape); a holder who leaves or spectates loses no life, the reactor
//!   just reappears after `next_reactor_secs`.
//! - Late joiners play until the first detonation, then spectate until the
//!   next match.
//! - End: the last player with lives wins; on the timer the most lives win,
//!   then the higher score; otherwise a draw.
//!
//! Replicated through the generic mode view: lives per player, the
//! `CARRIER` flag on the holder, and the reactor objective (`pos` = the
//! holder's, `radius` = `fuse_radius`, `holder`, `timer_end`, `state`:
//! [`STATE_WAITING`] or [`STATE_HELD`]). A detonation emits a
//! `CueKind::Shockwave` cue (`value` = blast radius).

use ls_math::Vec3;

use crate::behavior::{Cue, CueKind, DamageSource};
use crate::mode::{
    objective_kind, player_flags, ModeKind, ModeState, Objective, PlayerMode, LIVES_UNLIMITED,
};
use crate::modes::{
    lose_life, sides_in_play, ticks, Kill, ModeCtx, ModeHit, ModeModule, ModeMut, ModeRef, Outcome,
};
use crate::ruleset::{HotReactorRules, ModeRules};
use crate::ship::{PlayerId, ShipState};

/// Objective id of the reactor.
pub const REACTOR_ID: u8 = 0;
/// `Objective::state`: no reactor in play; the next appears at `timer_end`.
pub const STATE_WAITING: u8 = 0;
/// `Objective::state`: a ship holds it; it detonates at `timer_end`.
pub const STATE_HELD: u8 = 1;

/// [`ModeState::vars`] slots: fuse left in milli-ticks (integer, so the
/// speed-up is exact) ...
const VAR_FUSE: usize = 0;
/// ... tick until which the holder can't lose the reactor ...
const VAR_IMMUNE_UNTIL: usize = 1;
/// ... detonations so far ...
const VAR_DETONATIONS: usize = 2;
/// ... and 1 when the holder died holding it (detonate in `on_tick`).
const VAR_PENDING: usize = 3;

/// Fuse units per tick at 1x speed.
const FUSE_UNIT: i64 = 1000;
/// RNG key of the "random living player" draw.
const RNG_KEY_HOLDER: u64 = 0x484f_545f_5245_4143;

/// The Hot Reactor mode module.
#[derive(Debug)]
pub struct HotReactor;

fn rules(r: &ModeRules) -> HotReactorRules {
    match r {
        ModeRules::HotReactor(h) => *h,
        _ => HotReactorRules::default(),
    }
}

/// In play: neither eliminated nor spectating.
fn in_play(state: &ModeState, id: PlayerId) -> bool {
    state.player(id).is_some_and(|p| !p.is_out())
}

/// The reactor objective (created on demand).
fn reactor(state: &mut ModeState) -> &mut Objective {
    if state.objective(REACTOR_ID).is_none() {
        state.objectives.push(Objective::new(
            REACTOR_ID,
            objective_kind::REACTOR,
            Vec3::ZERO,
        ));
    }
    state.objective_mut(REACTOR_ID).expect("just added")
}

/// The current holder, if the reactor is held.
pub fn holder(state: &ModeState) -> Option<PlayerId> {
    state
        .objective(REACTOR_ID)
        .filter(|o| o.state == STATE_HELD)
        .and_then(|o| o.holder)
}

/// Detonations so far.
pub fn detonations(state: &ModeState) -> u32 {
    state.vars[VAR_DETONATIONS].max(0) as u32
}

/// Hands the reactor to `to` (a pass or a new reactor) and moves the
/// `CARRIER` flag; `immune`: `to` can't lose it for `pass_immunity_secs`.
fn give(state: &mut ModeState, r: &HotReactorRules, to: &ShipState, now: u64, immune: bool) {
    let o = reactor(state);
    o.state = STATE_HELD;
    o.holder = Some(to.player);
    o.pos = to.pos;
    o.vel = to.vel;
    o.radius = r.fuse_radius;
    state.vars[VAR_IMMUNE_UNTIL] = if immune {
        (now + ticks(r.pass_immunity_secs)) as i64
    } else {
        0
    };
    set_carrier(state, Some(to.player));
}

/// No reactor in play: the next one appears at `at`.
fn wait_until(state: &mut ModeState, r: &HotReactorRules, at: u64) {
    let o = reactor(state);
    o.state = STATE_WAITING;
    o.holder = None;
    o.vel = Vec3::ZERO;
    o.radius = r.fuse_radius;
    o.timer_end = at;
    state.vars[VAR_FUSE] = 0;
    state.vars[VAR_IMMUNE_UNTIL] = 0;
    state.vars[VAR_PENDING] = 0;
    set_carrier(state, None);
}

fn set_carrier(state: &mut ModeState, holder: Option<PlayerId>) {
    for (id, p) in state.players.iter_mut() {
        if Some(*id) == holder {
            p.flags |= player_flags::CARRIER;
        } else {
            p.flags &= !player_flags::CARRIER;
        }
    }
}

/// Whether the holder can lose the reactor now.
fn passable(state: &ModeState, now: u64) -> bool {
    now >= state.vars[VAR_IMMUNE_UNTIL].max(0) as u64
}

/// Extra fuse units per tick per nearby ship.
fn per_ship(r: &HotReactorRules) -> i64 {
    (r.fuse_speed_per_ship.max(0.0) * FUSE_UNIT as f32).round() as i64
}

/// Detonates at `pos`: the holder (if still alive) dies and loses a life,
/// the blast hurts everyone else around, and the next reactor is scheduled.
fn detonate(m: &mut ModeCtx, r: &HotReactorRules, pos: Vec3, holder: PlayerId) {
    let now = m.now();
    if m.ctx.ship(holder).is_some_and(|s| s.alive) {
        m.ctx.adjust_hull(holder, -1.0e9, true);
    }
    lose_life(m.state, holder);
    if r.blast_radius > 0.0 && r.blast_damage > 0.0 {
        m.ctx.splash(
            pos,
            r.blast_radius,
            r.blast_damage,
            0.0,
            DamageSource::ENVIRONMENT,
            true,
            Some(holder),
        );
    }
    m.ctx.cue(Cue {
        kind: CueKind::Shockwave,
        source: Some(holder),
        target: None,
        pos,
        value: r.blast_radius.clamp(0.0, u16::MAX as f32) as u16,
    });
    m.state.vars[VAR_DETONATIONS] += 1;
    wait_until(m.state, r, now + ticks(r.next_reactor_secs));
    reactor(m.state).pos = pos;
}

impl ModeModule for HotReactor {
    fn kind(&self) -> ModeKind {
        ModeKind::HotReactor
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }

    fn validate(&self, mode: &ModeRules) -> Result<(), String> {
        let r = rules(mode);
        if r.lives == 0 {
            return Err("Hot Reactor: lives must be at least 1".into());
        }
        if !(r.fuse_secs > 0.0 && r.fuse_secs.is_finite()) {
            return Err("Hot Reactor: fuse_secs must be positive".into());
        }
        for (name, v) in [
            ("fuse_radius", r.fuse_radius),
            ("fuse_speed_per_ship", r.fuse_speed_per_ship),
            ("blast_radius", r.blast_radius),
            ("blast_damage", r.blast_damage),
            ("pass_immunity_secs", r.pass_immunity_secs),
            ("next_reactor_secs", r.next_reactor_secs),
        ] {
            if !(v >= 0.0 && v.is_finite()) {
                return Err(format!("Hot Reactor: {name} must be 0 or more"));
            }
        }
        Ok(())
    }

    /// Late joiners play until the first detonation.
    fn late_join_spectates(&self, m: &ModeRef) -> bool {
        detonations(m.state) > 0
    }

    fn on_start(&self, m: &mut ModeCtx) {
        let r = rules(m.rules);
        let now = m.now();
        wait_until(m.state, &r, now + ticks(r.next_reactor_secs));
    }

    /// The shooter takes the reactor from the holder it hit.
    fn on_hit(&self, m: &mut ModeMut, hit: &ModeHit) {
        let Some(attacker) = hit.attacker.filter(|a| *a != hit.target) else {
            return;
        };
        if holder(m.state) != Some(hit.target)
            || !passable(m.state, m.now)
            || !in_play(m.state, attacker)
        {
            return;
        }
        let Some(to) = m.ships.get(&attacker).filter(|s| s.alive) else {
            return;
        };
        let r = rules(m.rules);
        give(m.state, &r, to, m.now, true);
    }

    /// Kills score; lives are only lost at detonation. A dying holder hands
    /// the reactor to its killer, or detonates it.
    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        if let (false, Some(k)) = (kill.is_suicide(), kill.killer) {
            if let Some(p) = m.state.players.get_mut(&k) {
                p.score += 1;
            }
        }
        if holder(m.state) != Some(kill.victim) {
            return;
        }
        let r = rules(m.rules);
        let killer = kill
            .killer
            .filter(|k| *k != kill.victim && in_play(m.state, *k))
            .and_then(|k| m.ships.get(&k))
            .filter(|s| s.alive);
        match killer {
            Some(to) => give(m.state, &r, to, m.now, true),
            None => m.state.vars[VAR_PENDING] = 1,
        }
    }

    fn on_tick(&self, m: &mut ModeCtx) {
        let r = rules(m.rules);
        let now = m.now();
        let o = *reactor(m.state);

        if o.state == STATE_HELD {
            let Some(h) = o.holder else {
                wait_until(m.state, &r, now + ticks(r.next_reactor_secs));
                return;
            };
            // Left the match or went spectating: no life lost, a new
            // reactor later.
            let Some(ship) = m.ctx.ship(h).filter(|_| in_play(m.state, h)) else {
                wait_until(m.state, &r, now + ticks(r.next_reactor_secs));
                return;
            };
            // Died holding it (not killed by another ship): detonates now.
            if m.state.vars[VAR_PENDING] != 0 || !ship.alive {
                detonate(m, &r, ship.pos, h);
                return;
            }
            // Ramming: touching another ship passes it on.
            let mut carrier = ship;
            if passable(m.state, now) {
                let rh = m.ctx.tuning_of(ship).radius;
                let mut best: Option<(&ShipState, f32)> = None;
                for s in m.ctx.view.ships.values() {
                    if s.player == h || !s.alive || !in_play(m.state, s.player) {
                        continue;
                    }
                    let reach = rh + m.ctx.tuning_of(s).radius;
                    let d = s.pos.distance_squared(ship.pos);
                    if d < reach * reach && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((s, d));
                    }
                }
                if let Some((to, _)) = best {
                    give(m.state, &r, to, now, true);
                    carrier = to;
                }
            }
            // The fuse: faster with every other ship near the holder.
            let near = m
                .ctx
                .ships_within(carrier.pos, r.fuse_radius)
                .into_iter()
                .filter(|s| s.player != carrier.player && in_play(m.state, s.player))
                .count() as i64;
            let rate = FUSE_UNIT + per_ship(&r) * near;
            let left = m.state.vars[VAR_FUSE] - rate;
            m.state.vars[VAR_FUSE] = left;
            if left <= 0 {
                detonate(m, &r, carrier.pos, carrier.player);
                return;
            }
            let o = reactor(m.state);
            o.pos = carrier.pos;
            o.vel = carrier.vel;
            o.timer_end = now + ((left + rate - 1) / rate) as u64;
            return;
        }

        // Waiting: once due, a random living player gets the next reactor
        // (with at least two players in play).
        if now < o.timer_end {
            return;
        }
        let in_play_count = m
            .state
            .players
            .iter()
            .filter(|(id, p)| !p.is_out() && m.ctx.ship(**id).is_some())
            .count();
        let candidates: Vec<&ShipState> = m
            .ctx
            .view
            .ships
            .values()
            .filter(|s| s.alive && in_play(m.state, s.player))
            .collect();
        if in_play_count < 2 || candidates.is_empty() {
            return;
        }
        let mut rng = m.ctx.rng(RNG_KEY_HOLDER);
        let to = candidates[rng.below(candidates.len() as u32) as usize];
        let fuse = ticks(r.fuse_secs);
        give(m.state, &r, to, now, false);
        m.state.vars[VAR_FUSE] = fuse as i64 * FUSE_UNIT;
        reactor(m.state).timer_end = now + fuse;
    }

    /// The last player with lives wins; on time the most lives, then the
    /// best score; otherwise a draw.
    fn end_check(&self, m: &ModeRef) -> Option<Outcome> {
        let (alive, played) = sides_in_play(m);
        if played >= 2 && alive.len() <= 1 {
            return Some(alive.first().copied());
        }
        if m.now < m.state.end_tick {
            return None;
        }
        let key = |p: &PlayerMode| {
            let lives = if p.lives == LIVES_UNLIMITED || p.is_out() {
                0
            } else {
                p.lives
            };
            (lives, p.score)
        };
        let mut best: Option<((u8, i32), u8)> = None;
        let mut tied = false;
        for (id, p) in &m.state.players {
            let Some(ship) = m.ships.get(id) else {
                continue;
            };
            if p.flags & player_flags::SPECTATOR != 0 {
                continue;
            }
            let k = key(p);
            match best {
                Some((bk, _)) if k < bk => {}
                Some((bk, _)) if k == bk => tied = true,
                _ => {
                    best = Some((k, ship.team));
                    tied = false;
                }
            }
        }
        Some(if tied { None } else { best.map(|(_, s)| s) })
    }
}

#[cfg(test)]
mod tests;
