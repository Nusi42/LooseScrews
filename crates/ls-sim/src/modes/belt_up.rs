//! Belt Up: a race of 2-8 ships around an asteroid belt (M4 package B6;
//! `docs/milestone-4-plan.md` §3.8).
//!
//! Settings: `ruleset::BeltUpRules` (`data/rulesets/belt_up.ron`). The
//! level is an asteroid belt (`MatchSetup::belt`, generator archetype
//! `Belt`): 8 start slots, 8 gates, the exit at the centre.
//!
//! Rules as built:
//! - **Heats.** A match is `heats` (3) heats on the same level. A heat:
//!   `lights_secs` (3) of start lights with every racer held in its slot
//!   (the `HELD` mode flag: [`BeltUp::move_limits`] stops the ship, weapons
//!   are offline, nothing takes damage, no radiation), then the race until
//!   every racer finished, `finish_grace_secs` (30) after the first
//!   finisher, or `heat_limit_secs` (180) after the lights; then
//!   `results_secs` (8) of heat results (everyone held), then the next heat
//!   ([`crate::Command::ResetRound`]: every ship back in its shifted slot
//!   with full hull and the loadout, pickups rolled again, projectiles and
//!   dynamic wells removed; terrain scars stay). After the last heat's
//!   results the match ends. `match_secs` is only a safety cap.
//! - **Slots.** `n` racers in join order take slots `round(i x 8 / n)`
//!   (evenly spread), shifted by one wedge per heat. At most 8 racers; more
//!   players spectate. A player joining during start lights races at once
//!   (everyone is placed again); later joiners spectate until the next heat
//!   and race from then with 0 points.
//! - **Course.** A racer flies prograde through the 8 gates in order, its
//!   own gate last (slot `k` sits 3 degrees past gate `k`): a gate counts
//!   when the tick's movement segment (from the racer's position at the
//!   end of the last tick, so teleports count) crosses it prograde
//!   (`BeltSite::crosses_gate`); passed gates are never revoked, and
//!   crossing any other gate (or flying the wrong way) does nothing. After
//!   the lap the exit (`BeltSite::exit_radius` around the centre) is open:
//!   reaching it finishes the race; the ship leaves play (`SPECTATOR` flag,
//!   no death) and the pilot follows the race.
//! - **Places.** Finishers by finish tick; the others by progress: gates
//!   passed, then the angle into the current sector, then (lap done) the
//!   distance to the exit; ties to the lower player id. Live places are
//!   replicated; the heat's final places score `points_1..points_8`
//!   (10, 8, 6, 5, 4, 3, 2, 1). Kills score nothing. Match: most points,
//!   then more heat wins, then the better place in the last heat, then the
//!   lower total time (unfinished heats count their full length).
//! - **Death.** No elimination: the wreck shows for `respawn_secs`, then
//!   the ship respawns at the last gate it passed (in that gate's slot
//!   pose; before the first gate its own slot), held until `hold_secs` (4)
//!   after the death, with the hull it had when it passed that gate (at
//!   least `hull_floor_percent` (30 %) of full), full energy and the weapons
//!   and ammo it had. Instagib's instant respawn still holds 4 s.
//! - **Radiation** (`crate::radiation`) is scaled by `radiation_scale`
//!   while racing and off during lights and results.
//! - **Bumps** ([`BeltUp::ship_contact`]): touching ships get equal and
//!   opposite impulses along the line between them, split by `mass`,
//!   separating at `bump_restitution` (1.2) times their approach speed (at
//!   least 1 u/s), at most `bump_max_speed` (10 u/s) per ship, once per
//!   pair per `bump_cooldown_secs` (0.3); no damage; held ships never bump.
//!   Server-authoritative: the client predictor corrects them like
//!   knockback.
//! - **Pickups** ([`BeltUp::pickup_group`]): the belt's pickup points are
//!   wedge-major; point `i` of every wedge rolls the same content (seeded
//!   by the heat and `i`), so every wedge holds identical pickups.
//! - **Director** ([`BeltUp::allows_event`], [`BeltUp::leader`]): no event
//!   starts outside racing or in a heat's first [`EVENT_QUIET_SECS`];
//!   Bored level, Wrong genre and Moving day are vetoed; leader events aim
//!   at the running-order leader.
//!
//! Without a belt (test rooms) the slots are the spawn points and nobody
//! can finish: heats end on their limit.
//!
//! Replicated: [`RaceView`] in the mode view (protocol 6 race section), the
//! racers' points as the mode score, `HELD` in the player flags.

use std::collections::BTreeMap;

use ls_math::level::{BeltSite, BELT_WEDGES, BELT_WEDGE_ANGLE};
use ls_math::{det, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::events::EventKind;
use crate::hash::StateHasher;
use crate::mode::{player_flags, ModeKind, ModeState, ModeView};
use crate::modes::{
    ticks, ContactShip, Kill, ModeCtx, ModeHit, ModeModule, ModeMut, ModeRef, MoveLimits, Outcome,
};
use crate::ruleset::{BeltUpRules, ModeRules};
use crate::ship::{PlayerId, ShipState};
use crate::world::MatchSetup;

pub mod pilot;
pub mod replay;

/// [`RaceState::phase`] / [`RaceView::phase`] values.
pub mod heat_phase {
    /// Start lights: racers held in their slots.
    pub const LIGHTS: u8 = 0;
    pub const RACING: u8 = 1;
    /// Heat results: everyone held.
    pub const RESULTS: u8 = 2;
    /// The last heat's results are over: the match ends.
    pub const DONE: u8 = 3;
}

/// Gates in a lap.
pub const LAP_GATES: u8 = BELT_WEDGES as u8;
/// Most racers in a heat (one per slot).
pub const MAX_RACERS: usize = BELT_WEDGES;
/// No comedy event starts in a heat's first seconds (§3.8).
pub const EVENT_QUIET_SECS: f32 = 15.0;
/// Smallest approach speed a bump separates with (overlapping ships that
/// are not closing in still part).
pub const MIN_BUMP_SPEED: f32 = 1.0;

/// One racer's state (kept for every player of the match).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Racer {
    /// Join order (slot assignment).
    pub order: u32,
    /// Races this heat (false: waiting for the next one, or no slot free).
    pub in_heat: bool,
    /// Start slot this heat.
    pub slot: u8,
    /// Gates passed this heat (8 = lap done).
    pub gates: u8,
    /// Tick the racer reached the exit (0 = not finished).
    pub finish_tick: u64,
    /// Live place this heat (final once the heat is over; 0 = none).
    pub place: u8,
    /// Held at its gate until this tick after a death (0 = not held).
    pub hold_end_tick: u64,
    /// Hull when the racer passed its last gate (at the start: full).
    pub gate_hull: f32,
    /// Position at the end of the last tick (gate crossing segments).
    pub last_pos: Vec3,
    /// The next spawn is a heat start (slot, full hull, loadout).
    pub fresh: bool,
    /// Heats won.
    pub heat_wins: u8,
    /// Place in the last finished heat (0 = none).
    pub last_place: u8,
    /// Sum of the heats' times (ticks; unfinished heats count in full).
    pub total_ticks: u64,
}

impl Racer {
    fn new(order: u32) -> Racer {
        Racer {
            order,
            in_heat: false,
            slot: 0,
            gates: 0,
            finish_tick: 0,
            place: 0,
            hold_end_tick: 0,
            gate_hull: 0.0,
            last_pos: Vec3::ZERO,
            fresh: true,
            heat_wins: 0,
            last_place: 0,
            total_ticks: 0,
        }
    }

    /// Racing now: in this heat and not finished.
    pub fn racing(&self) -> bool {
        self.in_heat && self.finish_tick == 0
    }
}

/// The race of a Belt Up match (`ModeState::race`; part of the state hash).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaceState {
    /// Current heat (0-based) and the number of heats.
    pub heat: u8,
    pub heats: u8,
    /// [`heat_phase`].
    pub phase: u8,
    /// Tick the current phase ends (start lights, heat limit, results).
    pub phase_end_tick: u64,
    /// Tick the current heat's lights went out (0 before).
    pub start_tick: u64,
    /// Tick the first racer finished this heat (0 = nobody yet).
    pub first_finish_tick: u64,
    /// The 8 slot poses (from the belt; without one, the spawn points).
    pub slots: Vec<(Vec3, Quat)>,
    pub racers: BTreeMap<PlayerId, Racer>,
    /// Bump cooldowns: `(a, b, tick)` pairs may bump again from `tick`.
    pub bump_ready: Vec<(PlayerId, PlayerId, u64)>,
    /// Next join order.
    pub next_order: u32,
    /// The racers of a heat in its start lights changed: place them again.
    pub dirty: bool,
}

impl RaceState {
    /// Hashes the race (`World::state_hash`).
    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u8(0xb6);
        h.write_u8(self.heat);
        h.write_u8(self.heats);
        h.write_u8(self.phase);
        h.write_u64(self.phase_end_tick);
        h.write_u64(self.start_tick);
        h.write_u64(self.first_finish_tick);
        h.write_u32(self.slots.len() as u32);
        for (p, q) in &self.slots {
            h.write_vec3(*p);
            h.write_quat(*q);
        }
        h.write_u32(self.racers.len() as u32);
        for (id, r) in &self.racers {
            h.write_u16(id.0);
            h.write_u32(r.order);
            h.write_bool(r.in_heat);
            h.write_u8(r.slot);
            h.write_u8(r.gates);
            h.write_u64(r.finish_tick);
            h.write_u8(r.place);
            h.write_u64(r.hold_end_tick);
            h.write_f32(r.gate_hull);
            h.write_vec3(r.last_pos);
            h.write_bool(r.fresh);
            h.write_u8(r.heat_wins);
            h.write_u8(r.last_place);
            h.write_u64(r.total_ticks);
        }
        h.write_u32(self.bump_ready.len() as u32);
        for (a, b, t) in &self.bump_ready {
            h.write_u16(a.0);
            h.write_u16(b.0);
            h.write_u64(*t);
        }
        h.write_u32(self.next_order);
        h.write_bool(self.dirty);
    }

    /// Racers in this heat.
    pub fn in_heat(&self) -> usize {
        self.racers.values().filter(|r| r.in_heat).count()
    }

    /// The view clients get.
    pub fn view(&self) -> RaceView {
        RaceView {
            heat: self.heat,
            heats: self.heats,
            phase: self.phase,
            phase_end_tick: self.phase_end_tick,
            racers: self
                .racers
                .iter()
                .filter(|(_, r)| r.in_heat)
                .map(|(id, r)| RacerView {
                    player: *id,
                    gates: r.gates,
                    finish_tick: r.finish_tick,
                    place: r.place,
                    hold_end_tick: r.hold_end_tick,
                    slot: r.slot,
                })
                .collect(),
        }
    }
}

/// One racer of a [`RaceView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RacerView {
    pub player: PlayerId,
    /// Gates passed (8 = lap done: the exit is open).
    pub gates: u8,
    /// Finish tick (0 = not finished).
    pub finish_tick: u64,
    /// Live place (final after the heat; 0 = none).
    pub place: u8,
    /// Held at a gate until this tick (0 = not held).
    pub hold_end_tick: u64,
    /// Start slot this heat (its next gate is `(slot + 1 + gates) % 8`).
    pub slot: u8,
}

impl RacerView {
    /// The next gate to pass (`None` once the lap is done).
    pub fn next_gate(&self) -> Option<usize> {
        (self.gates < LAP_GATES)
            .then(|| (self.slot as usize + 1 + self.gates as usize) % BELT_WEDGES)
    }
}

/// Belt Up's replicated race (`ModeView::race`; protocol 6 race section):
/// heat and phase, and per racer of this heat its gates, finish, place,
/// hold and slot. Sent whenever it changes (gates, places, holds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaceView {
    pub heat: u8,
    pub heats: u8,
    /// [`heat_phase`].
    pub phase: u8,
    pub phase_end_tick: u64,
    /// Racers of this heat, by player id.
    pub racers: Vec<RacerView>,
}

impl RaceView {
    pub fn racer(&self, player: PlayerId) -> Option<&RacerView> {
        self.racers.iter().find(|r| r.player == player)
    }
}

/// The Belt Up mode module.
#[derive(Debug)]
pub struct BeltUp;

/// The mode's settings (defaults for another mode's rules).
pub fn rules(r: &ModeRules) -> BeltUpRules {
    match r {
        ModeRules::BeltUp(b) => *b,
        _ => BeltUpRules::default(),
    }
}

/// The radiation scale of a race in heat phase `phase` (what
/// [`BeltUp::radiation_scale`] gives; clients scale their dose meter by it).
pub fn radiation_scale_in(rules: &BeltUpRules, phase: u8) -> f32 {
    if phase == heat_phase::RACING {
        rules.radiation_scale.max(0.0)
    } else {
        0.0
    }
}

/// Slot of the `i`-th of `n` racers in heat `heat`: evenly spread
/// (`round(i x 8 / n)`), shifted by one wedge per heat.
pub fn slot_for(i: usize, n: usize, heat: u8) -> u8 {
    let n = n.clamp(1, MAX_RACERS);
    let base = (2 * i * BELT_WEDGES + n) / (2 * n);
    ((base + heat as usize) % BELT_WEDGES) as u8
}

/// Progress of a racer at `pos` for placing it: gates passed, then a
/// position key inside the current sector (the angle past the last gate in
/// wedges, or once the lap is done, minus the distance to the exit).
pub fn progress(belt: Option<&BeltSite>, r: &Racer, pos: Vec3) -> (u8, f32) {
    let Some(b) = belt else {
        return (r.gates, 0.0);
    };
    if r.gates >= LAP_GATES {
        return (r.gates, -pos.distance(b.center));
    }
    let last_gate = (r.slot as usize + r.gates as usize) % BELT_WEDGES;
    let a = b.azimuth(pos) - BeltSite::gate_azimuth(last_gate);
    let a = det::wrap_angle(a);
    (r.gates, (a / BELT_WEDGE_ANGLE).clamp(-1.0, 2.0))
}

/// Orders racers: finishers by finish tick, then by progress, ties to the
/// lower id. `pos` gives each racer's position.
fn order_racers(
    race: &RaceState,
    belt: Option<&BeltSite>,
    pos: impl Fn(PlayerId, &Racer) -> Vec3,
) -> Vec<PlayerId> {
    let mut list: Vec<(PlayerId, u64, u8, f32)> = race
        .racers
        .iter()
        .filter(|(_, r)| r.in_heat)
        .map(|(id, r)| {
            let (g, k) = progress(belt, r, pos(*id, r));
            let fin = if r.finish_tick == 0 {
                u64::MAX
            } else {
                r.finish_tick
            };
            (*id, fin, g, k)
        })
        .collect();
    list.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then(b.2.cmp(&a.2))
            .then(b.3.total_cmp(&a.3))
            .then(a.0.cmp(&b.0))
    });
    list.into_iter().map(|x| x.0).collect()
}

/// Places the heat's racers in slots (join order, evenly spread, shifted by
/// the heat) and marks their next spawn as a heat start.
fn assign_slots(race: &mut RaceState) {
    let mut ids: Vec<(u32, PlayerId)> = race
        .racers
        .iter()
        .filter(|(_, r)| r.in_heat)
        .map(|(id, r)| (r.order, *id))
        .collect();
    ids.sort_unstable();
    let n = ids.len();
    for (i, (_, id)) in ids.into_iter().enumerate() {
        if let Some(r) = race.racers.get_mut(&id) {
            r.slot = slot_for(i, n, race.heat);
        }
    }
}

/// Fresh per-heat state for every racer; the first [`MAX_RACERS`] players
/// in join order (not spectating by choice) race.
fn start_heat(state: &mut ModeState, race: &mut RaceState, now: u64, r: &BeltUpRules) {
    race.phase = heat_phase::LIGHTS;
    race.phase_end_tick = now + ticks(r.lights_secs);
    race.start_tick = 0;
    race.first_finish_tick = 0;
    race.bump_ready.clear();
    let mut by_order: Vec<(u32, PlayerId)> =
        race.racers.iter().map(|(id, x)| (x.order, *id)).collect();
    by_order.sort_unstable();
    let mut seats = 0;
    for (_, id) in by_order {
        let x = race.racers.get_mut(&id).expect("listed");
        // Spectating by choice (flag set without a racer slot) stays out.
        let chosen_out = state.player(id).is_some_and(|p| p.is_out()) && !x.in_heat;
        let waiting = x.order >= u32::MAX / 2;
        x.in_heat = seats < MAX_RACERS && (!chosen_out || waiting);
        if waiting {
            x.order -= u32::MAX / 2;
        }
        if x.in_heat {
            seats += 1;
        }
        x.gates = 0;
        x.finish_tick = 0;
        x.place = 0;
        x.hold_end_tick = 0;
        x.fresh = true;
        if let Some(p) = state.players.get_mut(&id) {
            if x.in_heat {
                p.flags &= !player_flags::SPECTATOR;
            } else {
                p.flags |= player_flags::SPECTATOR;
            }
        }
    }
    assign_slots(race);
    race.dirty = false;
}

impl BeltUp {
    fn finish_heat(&self, m: &mut ModeCtx, race: &mut RaceState, r: &BeltUpRules) {
        let now = m.now();
        let belt = m.setup.belt.as_ref();
        let ships = m.ctx.view.ships;
        let order = order_racers(race, belt, |id, x| {
            ships
                .get(&id)
                .filter(|s| s.alive)
                .map_or(x.last_pos, |s| s.pos)
        });
        let heat_len = now.saturating_sub(race.start_tick);
        for (i, id) in order.iter().enumerate() {
            let place = i + 1;
            let x = race.racers.get_mut(id).expect("ordered racers exist");
            x.place = place as u8;
            x.last_place = place as u8;
            if place == 1 {
                x.heat_wins = x.heat_wins.saturating_add(1);
            }
            x.total_ticks += if x.finish_tick > 0 {
                x.finish_tick.saturating_sub(race.start_tick)
            } else {
                heat_len
            };
            if let Some(p) = m.state.players.get_mut(id) {
                p.score += r.points(place);
            }
        }
        // Racers that were not in this heat lose their last place.
        for x in race.racers.values_mut().filter(|x| !x.in_heat) {
            x.last_place = 0;
        }
        race.phase = heat_phase::RESULTS;
        race.phase_end_tick = now + ticks(r.results_secs);
        let hold = race.phase_end_tick - now;
        for id in order {
            m.ctx.disable_weapons(id, hold);
        }
    }

    /// The racer standings for the match result: points, heat wins, last
    /// place, total time.
    pub fn standings(state: &ModeState) -> Vec<PlayerId> {
        let Some(race) = &state.race else {
            return Vec::new();
        };
        let mut list: Vec<(PlayerId, i32, u8, u8, u64)> = race
            .racers
            .iter()
            .map(|(id, x)| {
                let score = state.player(*id).map_or(0, |p| p.score);
                let last = if x.last_place == 0 {
                    u8::MAX
                } else {
                    x.last_place
                };
                (*id, score, x.heat_wins, last, x.total_ticks)
            })
            .collect();
        list.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then(b.2.cmp(&a.2))
                .then(a.3.cmp(&b.3))
                .then(a.4.cmp(&b.4))
                .then(a.0.cmp(&b.0))
        });
        list.into_iter().map(|x| x.0).collect()
    }

    fn winner(m: &ModeRef) -> Outcome {
        let race = m.state.race.as_ref()?;
        let order = Self::standings(m.state);
        let key = |id: &PlayerId| {
            let x = &race.racers[id];
            (
                m.state.player(*id).map_or(0, |p| p.score),
                x.heat_wins,
                x.last_place,
                x.total_ticks,
            )
        };
        let first = order.first()?;
        if order.get(1).is_some_and(|second| key(second) == key(first)) {
            return None;
        }
        m.ships.get(first).map(|s| s.team)
    }
}

impl ModeModule for BeltUp {
    fn kind(&self) -> ModeKind {
        ModeKind::BeltUp
    }

    fn implemented(&self) -> bool {
        true
    }

    fn team_counts(&self) -> &'static [u8] {
        &[0]
    }

    fn validate(&self, rules: &ModeRules) -> Result<(), String> {
        let r = self::rules(rules);
        if r.heats == 0 {
            return Err("Belt Up needs at least one heat".into());
        }
        if r.heat_limit_secs <= 0.0 || r.lights_secs < 0.0 || r.results_secs < 0.0 {
            return Err("Belt Up heat times must be positive".into());
        }
        if !(0.0..=100.0).contains(&(r.hull_floor_percent as f32)) {
            return Err("hull_floor_percent must be 0-100".into());
        }
        Ok(())
    }

    fn on_join(&self, m: &mut ModeMut, player: PlayerId) {
        let r = rules(m.rules);
        let now = m.now;
        let Some(race) = m.state.race.as_mut() else {
            return;
        };
        if race.in_heat() == 0 && race.phase != heat_phase::DONE {
            // Nobody races this heat (an empty room): it starts over with
            // the joiner instead of making it wait.
            race.phase = heat_phase::LIGHTS;
            race.phase_end_tick = now + ticks(r.lights_secs);
            race.start_tick = 0;
            race.first_finish_tick = 0;
        }
        let order = race.next_order;
        race.next_order += 1;
        let mut x = Racer::new(order);
        if race.phase == heat_phase::LIGHTS && race.in_heat() < MAX_RACERS {
            // Joins this heat: everyone is placed again.
            x.in_heat = true;
            race.dirty = true;
        } else {
            // Waits for the next heat (marked by a high order until then).
            x.order = order + u32::MAX / 2;
        }
        race.racers.insert(player, x);
    }

    fn late_join_spectates(&self, m: &ModeRef) -> bool {
        let Some(race) = &m.state.race else {
            return false;
        };
        // The newest racer is the joiner: it spectates unless it got a seat.
        race.racers
            .values()
            .max_by_key(|x| x.order % (u32::MAX / 2))
            .is_some_and(|x| !x.in_heat)
            || race.phase != heat_phase::LIGHTS
    }

    /// Back from spectating only into a heat's start lights with a free
    /// seat (the lights seat it).
    fn may_stop_spectating(&self, m: &ModeRef, _player: PlayerId) -> bool {
        m.state
            .race
            .as_ref()
            .is_none_or(|race| race.phase == heat_phase::LIGHTS && race.in_heat() < MAX_RACERS)
    }

    fn on_start(&self, m: &mut ModeCtx) {
        let r = rules(m.rules);
        let now = m.now();
        let slots: Vec<(Vec3, Quat)> = match &m.setup.belt {
            Some(b) => b.slots.iter().map(|s| (s.pos, s.orientation)).collect(),
            None => m
                .setup
                .spawns
                .iter()
                .map(|s| (s.pos, s.orientation))
                .cycle()
                .take(BELT_WEDGES)
                .collect(),
        };
        let mut race = RaceState {
            heat: 0,
            heats: r.heats.max(1),
            phase: heat_phase::LIGHTS,
            phase_end_tick: 0,
            start_tick: 0,
            first_finish_tick: 0,
            slots,
            racers: BTreeMap::new(),
            bump_ready: Vec::new(),
            next_order: 0,
            dirty: false,
        };
        let ids: Vec<PlayerId> = m.state.players.keys().copied().collect();
        for id in ids {
            let mut x = Racer::new(race.next_order);
            race.next_order += 1;
            // Spectating by choice stays out; everyone else races.
            x.in_heat = !m.state.player(id).is_some_and(|p| p.is_out());
            race.racers.insert(id, x);
        }
        // Players beyond the first 8 wait; mark them so start_heat seats them.
        start_heat(m.state, &mut race, now, &r);
        m.state.race = Some(race);
        m.ctx.push(crate::Command::ResetRound);
    }

    fn allow_respawn(&self, m: &ModeRef, player: PlayerId) -> bool {
        let Some(race) = &m.state.race else {
            return true;
        };
        race.phase == heat_phase::RACING && race.racers.get(&player).is_some_and(|x| x.racing())
    }

    fn on_spawn(&self, m: &mut ModeMut, ship: &mut ShipState) {
        let r = rules(m.rules);
        let now = m.now;
        let id = ship.player;
        let old = m.ships.get(&id).cloned();
        let Some(race) = m.state.race.as_mut() else {
            return;
        };
        let (phase, phase_end) = (race.phase, race.phase_end_tick);
        let Some(x) = race.racers.get_mut(&id) else {
            return;
        };
        let pose = |k: usize| race.slots.get(k % race.slots.len().max(1)).copied();
        let full = ship.hull;
        let mut held_until = 0;
        if x.fresh {
            if let Some((p, q)) = pose(x.slot as usize) {
                ship.pos = p;
                ship.orientation = q;
            }
            x.fresh = false;
            x.gate_hull = full;
            if phase == heat_phase::LIGHTS {
                held_until = phase_end;
            }
        } else {
            // Back at the last gate passed (its slot pose; before the first
            // gate the own slot), with the gate's hull and the old weapons.
            let k = x.slot as usize + x.gates as usize;
            if let Some((p, q)) = pose(k) {
                ship.pos = p;
                ship.orientation = q;
            }
            let floor = full * r.hull_floor_percent as f32 / 100.0;
            ship.hull = x.gate_hull.max(floor).min(full);
            if let Some(old) = old.filter(|o| o.ship_type == ship.ship_type) {
                ship.weapons = old.weapons;
                ship.primary = old.primary;
                ship.secondary = old.secondary;
                ship.gadget = old.gadget;
                ship.weapon_state = old.weapon_state;
            }
            held_until = x.hold_end_tick.max(now);
        }
        x.last_pos = ship.pos;
        if held_until > now {
            ship.weapons_offline_until = ship.weapons_offline_until.max(held_until);
            if let Some(p) = m.state.players.get_mut(&id) {
                p.flags |= player_flags::HELD;
            }
        }
    }

    fn on_damage(&self, m: &ModeRef, hit: &mut ModeHit) {
        let Some(race) = &m.state.race else {
            return;
        };
        let held = m
            .state
            .player(hit.target)
            .is_some_and(|p| p.flags & player_flags::HELD != 0);
        if race.phase != heat_phase::RACING || held {
            hit.amount = 0.0;
        }
    }

    fn on_kill(&self, m: &mut ModeMut, kill: &Kill) {
        let r = rules(m.rules);
        let now = m.now;
        if let Some(x) = m
            .state
            .race
            .as_mut()
            .and_then(|race| race.racers.get_mut(&kill.victim))
        {
            x.hold_end_tick = now + ticks(r.hold_secs);
        }
    }

    fn on_tick(&self, m: &mut ModeCtx) {
        let Some(mut race) = m.state.race.take() else {
            return;
        };
        let r = rules(m.rules);
        let now = m.now();
        let ships = m.ctx.view.ships;
        race.racers.retain(|id, _| ships.contains_key(id));
        race.bump_ready.retain(|(_, _, t)| *t > now);
        // Racers who switched to spectating during the heat (finishers
        // spectate too, but with their finish tick) leave it: no place, no
        // points, and they stay out of the next heats until they play again.
        let racing_or_lights = matches!(race.phase, heat_phase::LIGHTS | heat_phase::RACING);
        for (id, x) in race.racers.iter_mut() {
            let spectating = m
                .state
                .player(*id)
                .is_some_and(|p| p.flags & player_flags::SPECTATOR != 0);
            if x.in_heat && x.finish_tick == 0 && spectating {
                x.in_heat = false;
                if racing_or_lights {
                    x.place = 0;
                }
                if race.phase == heat_phase::LIGHTS {
                    race.dirty = true;
                }
            }
        }
        // Everyone racing left while others wait for the next heat: the
        // heat starts over with them instead of running empty to its limit.
        if race.phase == heat_phase::RACING && race.in_heat() == 0 {
            let seats_someone = race.racers.iter().any(|(id, x)| {
                x.order >= u32::MAX / 2 || !m.state.player(*id).is_some_and(|p| p.is_out())
            });
            if seats_someone {
                start_heat(m.state, &mut race, now, &r);
                m.ctx.push(crate::Command::ResetRound);
            }
        }
        match race.phase {
            heat_phase::LIGHTS => {
                // Players who stopped spectating during the lights race.
                let free = MAX_RACERS.saturating_sub(race.in_heat());
                let mut back: Vec<PlayerId> = race
                    .racers
                    .iter()
                    .filter(|(id, x)| {
                        !x.in_heat && !m.state.player(**id).is_some_and(|p| p.is_out())
                    })
                    .map(|(id, _)| *id)
                    .collect();
                back.truncate(free);
                for id in back {
                    let x = race.racers.get_mut(&id).expect("listed");
                    x.in_heat = true;
                    x.order %= u32::MAX / 2;
                    race.dirty = true;
                }
                if race.dirty {
                    assign_slots(&mut race);
                    for x in race.racers.values_mut().filter(|x| x.in_heat) {
                        x.fresh = true;
                    }
                    race.dirty = false;
                    m.ctx.push(crate::Command::ResetRound);
                }
                if now >= race.phase_end_tick {
                    race.phase = heat_phase::RACING;
                    race.start_tick = now;
                    race.phase_end_tick = now + ticks(r.heat_limit_secs);
                }
            }
            heat_phase::RACING => {
                let belt = m.setup.belt.as_ref();
                let mut finished = race.racers.values().filter(|x| x.finish_tick > 0).count();
                for (id, x) in race.racers.iter_mut() {
                    if !x.racing() {
                        continue;
                    }
                    let Some(s) = ships.get(id).filter(|s| s.alive) else {
                        continue;
                    };
                    if let Some(b) = belt {
                        // Gates in order (a teleport may pass several).
                        while x.gates < LAP_GATES {
                            let next = (x.slot as usize + 1 + x.gates as usize) % BELT_WEDGES;
                            if !b.crosses_gate(next, x.last_pos, s.pos) {
                                break;
                            }
                            x.gates += 1;
                            x.gate_hull = s.hull;
                        }
                        if x.gates >= LAP_GATES && reaches_exit(b, x.last_pos, s.pos) {
                            finished += 1;
                            x.finish_tick = now;
                            x.place = finished as u8;
                            if race.first_finish_tick == 0 {
                                race.first_finish_tick = now;
                            }
                            if let Some(p) = m.state.players.get_mut(id) {
                                p.flags |= player_flags::SPECTATOR;
                            }
                        }
                    }
                    x.last_pos = s.pos;
                }
                // Live places.
                let order = order_racers(&race, belt, |id, x| {
                    ships
                        .get(&id)
                        .filter(|s| s.alive)
                        .map_or(x.last_pos, |s| s.pos)
                });
                for (i, id) in order.iter().enumerate() {
                    if let Some(x) = race.racers.get_mut(id) {
                        x.place = (i + 1) as u8;
                    }
                }
                let all_done = race.in_heat() > 0
                    && race
                        .racers
                        .values()
                        .filter(|x| x.in_heat)
                        .all(|x| x.finish_tick > 0);
                let grace_over = race.first_finish_tick > 0
                    && now >= race.first_finish_tick + ticks(r.finish_grace_secs);
                if all_done || grace_over || now >= race.phase_end_tick {
                    self.finish_heat(m, &mut race, &r);
                }
            }
            heat_phase::RESULTS if now >= race.phase_end_tick => {
                if race.heat + 1 < race.heats {
                    race.heat += 1;
                    start_heat(m.state, &mut race, now, &r);
                    m.ctx.push(crate::Command::ResetRound);
                } else {
                    race.phase = heat_phase::DONE;
                }
            }
            _ => {}
        }
        // Holds: start lights and results hold every racer; a death hold
        // ends on its tick.
        let all_held = matches!(race.phase, heat_phase::LIGHTS | heat_phase::RESULTS);
        for (id, x) in race.racers.iter_mut() {
            if x.hold_end_tick != 0 && x.hold_end_tick <= now {
                x.hold_end_tick = 0;
            }
            let held = (all_held && x.in_heat) || x.hold_end_tick > now;
            if let Some(p) = m.state.players.get_mut(id) {
                if held {
                    p.flags |= player_flags::HELD;
                } else {
                    p.flags &= !player_flags::HELD;
                }
            }
        }
        m.state.race = Some(race);
    }

    fn end_check(&self, m: &ModeRef) -> Option<Outcome> {
        match &m.state.race {
            Some(race) if race.phase == heat_phase::DONE => Some(Self::winner(m)),
            Some(_) if m.now >= m.state.end_tick => Some(Self::winner(m)),
            Some(_) => None,
            None => (m.now >= m.state.end_tick).then_some(None),
        }
    }

    fn view(&self, state: &ModeState, _rules: &ModeRules, view: &mut ModeView) {
        if let Some(race) = &state.race {
            view.stage = race.heat;
            view.stage_end_tick = race.phase_end_tick;
            view.race = Some(race.view());
        }
    }

    fn move_limits(&self, _rules: &ModeRules, flags: u8) -> MoveLimits {
        if flags & player_flags::HELD != 0 {
            MoveLimits {
                speed_scale: 0.0,
                no_boost: true,
            }
        } else {
            MoveLimits::NONE
        }
    }

    fn radiation_scale(&self, m: &ModeRef) -> f32 {
        match &m.state.race {
            Some(race) => radiation_scale_in(&rules(m.rules), race.phase),
            None => 0.0,
        }
    }

    fn allows_event(&self, m: &ModeRef, kind: EventKind) -> bool {
        if matches!(
            kind,
            EventKind::BoredLevel | EventKind::WrongGenre | EventKind::MovingDay
        ) {
            return false;
        }
        m.state.race.as_ref().is_some_and(|race| {
            race.phase == heat_phase::RACING && m.now >= race.start_tick + ticks(EVENT_QUIET_SECS)
        })
    }

    fn leader(&self, m: &ModeRef) -> Option<PlayerId> {
        let race = m.state.race.as_ref()?;
        if race.phase != heat_phase::RACING {
            return None;
        }
        race.racers
            .iter()
            .filter(|(id, x)| x.racing() && x.place > 0 && m.ships.get(id).is_some_and(|s| s.alive))
            .min_by_key(|(id, x)| (x.place, **id))
            .map(|(id, _)| *id)
    }

    fn wants_ship_contact(&self) -> bool {
        true
    }

    fn ship_contact(
        &self,
        m: &mut ModeMut,
        a: &ContactShip,
        b: &ContactShip,
    ) -> Option<(Vec3, Vec3)> {
        let r = rules(m.rules);
        let now = m.now;
        let held = |id: PlayerId| {
            m.state
                .player(id)
                .is_some_and(|p| p.flags & player_flags::HELD != 0)
        };
        if held(a.player) || held(b.player) {
            return None;
        }
        let race = m.state.race.as_mut()?;
        if race.phase != heat_phase::RACING {
            return None;
        }
        if race
            .bump_ready
            .iter()
            .any(|(x, y, t)| *x == a.player && *y == b.player && *t > now)
        {
            return None;
        }
        let (da, db) = bump(a, b, &r)?;
        race.bump_ready
            .push((a.player, b.player, now + ticks(r.bump_cooldown_secs).max(1)));
        Some((da, db))
    }

    fn pickup_group(&self, state: &ModeState, setup: &MatchSetup, point: usize) -> Option<u64> {
        let belt = setup.belt.as_ref()?;
        let n = setup.pickup_points.len();
        if n == 0 || n % BELT_WEDGES != 0 || belt.pickup_points.len() != n {
            return None;
        }
        let per = n / BELT_WEDGES;
        let heat = state.race.as_ref().map_or(0, |r| r.heat) as u64;
        Some(ls_math::rng::mix64(
            0x4245_4c54_5550_0000 ^ (heat << 8) ^ (point % per) as u64,
        ))
    }
}

/// Whether the movement `a -> b` reaches the exit sphere (swept).
fn reaches_exit(belt: &BeltSite, a: Vec3, b: Vec3) -> bool {
    if belt.in_exit(b) {
        return true;
    }
    let d = b - a;
    let len = d.length();
    if len <= 0.0 {
        return false;
    }
    crate::terrain::segment_sphere(a, d / len, len, belt.center, belt.exit_radius).is_some()
}

/// The velocity changes of a bump between `a` and `b` (see the module
/// docs): along the line from `a` to `b`, split by mass, separating at
/// `bump_restitution` times the approach speed (at least
/// [`MIN_BUMP_SPEED`]), each capped at `bump_max_speed`.
pub fn bump(a: &ContactShip, b: &ContactShip, r: &BeltUpRules) -> Option<(Vec3, Vec3)> {
    let n = (b.pos - a.pos)
        .try_normalize()
        .or_else(|| (a.vel - b.vel).try_normalize())
        .unwrap_or(Vec3::X);
    let approach = (a.vel - b.vel).dot(n).max(MIN_BUMP_SPEED);
    let (ma, mb) = (a.mass.max(0.01), b.mass.max(0.01));
    // Relative speed change: from +approach to -restitution x approach.
    let dv_rel = approach * (1.0 + r.bump_restitution.max(0.0));
    let j = dv_rel * ma * mb / (ma + mb);
    let cap = |v: Vec3| {
        let l = v.length();
        if l > r.bump_max_speed && l > 0.0 {
            v * (r.bump_max_speed / l)
        } else {
            v
        }
    };
    Some((cap(n * (-j / ma)), cap(n * (j / mb))))
}

#[cfg(test)]
mod tests;
