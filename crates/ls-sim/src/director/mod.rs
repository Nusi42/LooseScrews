//! The comedy director (`docs/milestone-4-plan.md` §3.1): a deterministic
//! last stage of [`crate::World::step`] that rolls comedy events
//! ([`crate::events`]) weighted by the effective Absurdity and the level's
//! mood, telegraphs each one for 3 s, runs it and keeps the gaps.
//!
//! - **Effective Absurdity**: `min(MatchSetup::absurdity, RuleSet::absurdity)`
//!   (the world's temperament, capped by the host's rule set). At 0 the
//!   director is off: no mood drift, no events, nothing hashed, so the world
//!   is bit-identical to a build without a director.
//! - **Cadence** (numbers in `data/events/director.ron`, [`DirectorDef`]): no
//!   announcement in the first 45 s of a match or in its last 20 s (plus the
//!   announce phase); a roll every 20 s from the quiet start with a chance
//!   of `0.6 x absurdity / 100`; one event at a time, and at least 30 s
//!   (longer when the mood is calm) between one event's end and the next
//!   announcement: a successful roll while an event runs or during that gap
//!   waits (at most one waits) and the event is picked and announced as soon
//!   as the gap has passed; an event does not repeat within a match while
//!   others are eligible. Expected per 5-minute match: about 1.4 events at
//!   Absurdity 20, 3.5 at 50, 5 at 100 (the gaps cap it there).
//! - **Choice**: enabled events, not banned by the rule set
//!   (`RuleSet::events`), allowed by the mode (`ModeModule::allows_event`)
//!   and the mutators, and `eligible`; weight `EventDef::weight`, times
//!   `mood_bias` when the event lists the current mood.
//! - **Moods**: calm, bored, grumpy, giddy; drifts every 60-90 s to another
//!   mood. Presentation and weights only.
//! - **Determinism**: every roll draws from a fresh `Pcg32` on
//!   [`RNG_STREAM_MOOD`] keyed by the match seed and the tick
//!   ([`mood_rng`]); the director state is part of the state hash once it
//!   did anything.
//! - **Forced events** (`World::schedule_event`; practice `?event=<id>`, the
//!   server's `--event-schedule`, replays): start at their tick (or as soon
//!   as the running event ended), whatever the Absurdity, bans, vetoes,
//!   `enabled` and eligibility.
//!
//! The replicated part is the [`MoodView`] (in `ModeView::mood`).
//! Everything an event does to movement and the other effect hooks are
//! pure functions of the view, the match seed and the tick
//! ([`event_effects`], [`move_effects`]), so the client predictor applies
//! them too.

use ls_math::rng::mix64;
use ls_math::{Pcg32, Tick, Vec3, DT};
use serde::{Deserialize, Serialize};

use crate::behavior::Ctx;
use crate::events::{
    EffectCtx, EventCtx, EventKind, EventPhase, EventRef, EventState, EventTable, MoveEffects,
};
use crate::hash::StateHasher;
use crate::mode::ModeState;
use crate::modes::{ModeModule, ModeRef};
use crate::mutators::MutatorModule;
use crate::ship::{PlayerId, ShipState};
use crate::world::MatchSetup;
use crate::SimError;

/// RNG stream of the director and the events (see
/// `docs/architecture/contracts.md`, RNG streams).
pub const RNG_STREAM_MOOD: u64 = 0x5349_4d5f_4d4f_4f44; // "SIM_MOOD"

/// A private RNG on [`RNG_STREAM_MOOD`] for `key` at `tick`:
/// `Pcg32::new(mix64(match_seed ^ mix64(tick ^ key ^ stream)), stream)`.
/// Key 0 is the director's roll, key 1 its mood drift, key 2 the choice of
/// the event, keys with the event
/// index in the high 32 bits belong to the events (`EventCtx::rng`).
pub fn mood_rng(seed: u64, tick: u64, key: u64) -> Pcg32 {
    Pcg32::new(
        mix64(seed ^ mix64(tick ^ key.rotate_left(17) ^ RNG_STREAM_MOOD)),
        RNG_STREAM_MOOD,
    )
}

const KEY_ROLL: u64 = 0;
const KEY_MOOD: u64 = 1;
const KEY_CHOOSE: u64 = 2;

/// The level's mood (append only: the index is the wire id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum Mood {
    #[default]
    Calm = 0,
    Bored = 1,
    Grumpy = 2,
    Giddy = 3,
}

impl Mood {
    pub const ALL: [Mood; 4] = [Mood::Calm, Mood::Bored, Mood::Grumpy, Mood::Giddy];

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Option<Mood> {
        Mood::ALL.get(i as usize).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Mood::Calm => "calm",
            Mood::Bored => "bored",
            Mood::Grumpy => "grumpy",
            Mood::Giddy => "giddy",
        }
    }
}

/// The replicated director state (protocol 6 `MoodView` item, inside the
/// mode state; sent on change). Equal views mean nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MoodView {
    pub mood: Mood,
    /// The running event (announced, active or ending); `None` between events.
    pub event: Option<EventKind>,
    /// Its phase (meaningless without an event).
    pub phase: EventPhase,
    /// Tick the current phase ends.
    pub phase_end_tick: u64,
    /// Tick the active phase starts (started): the time base of everything
    /// the event moves (duck paths, hiccup ticks).
    pub active_tick: u64,
    /// The player the event names.
    pub target: Option<PlayerId>,
    /// Event-defined parameter bytes (axis, route, seed bits).
    pub params: [u8; 8],
}

impl MoodView {
    /// Whether the event's effects apply (active or ending phase).
    pub fn effects_on(&self) -> bool {
        self.event.is_some() && self.phase != EventPhase::Announce
    }
}

/// The director's numbers (`data/events/director.ron`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectorDef {
    /// Telegraph before every event.
    pub announce_secs: f32,
    /// Seconds between rolls.
    pub roll_secs: f32,
    /// Chance per roll at Absurdity 100 (scaled linearly).
    pub chance_at_100: f32,
    /// No announcement before this many seconds into the match...
    pub quiet_start_secs: f32,
    /// ...nor in the last this many seconds (plus the announce phase).
    pub quiet_end_secs: f32,
    /// Least time between one event's end and the next announcement.
    pub gap_secs: f32,
    /// The gap is this much longer while the mood is calm.
    pub calm_gap_scale: f32,
    /// Weight factor of events that list the current mood.
    pub mood_bias: f32,
    /// The mood drifts after a uniform time in this range.
    pub mood_min_secs: f32,
    pub mood_max_secs: f32,
}

const DIRECTOR_RON: &str = include_str!("../../../../data/events/director.ron");
/// Path of the director's data file (data hash).
pub const DIRECTOR_FILE: &str = "events/director.ron";

impl DirectorDef {
    pub fn from_ron(source_name: &str, text: &str) -> Result<DirectorDef, SimError> {
        let def: DirectorDef = ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })?;
        let ok = |v: f32| v.is_finite() && v >= 0.0;
        let all = [
            def.announce_secs,
            def.roll_secs,
            def.chance_at_100,
            def.quiet_start_secs,
            def.quiet_end_secs,
            def.gap_secs,
            def.calm_gap_scale,
            def.mood_bias,
            def.mood_min_secs,
            def.mood_max_secs,
        ];
        if !all.iter().all(|&v| ok(v))
            || def.roll_secs <= 0.0
            || def.chance_at_100 > 1.0
            || def.mood_max_secs < def.mood_min_secs
            || def.mood_min_secs <= 0.0
        {
            return Err(SimError::InvalidEvent {
                id: "director".into(),
                reason: "values must be >= 0, roll_secs and mood times > 0, chance <= 1".into(),
            });
        }
        Ok(def)
    }

    /// The embedded `data/events/director.ron`.
    pub fn builtin() -> &'static DirectorDef {
        static DEF: std::sync::OnceLock<DirectorDef> = std::sync::OnceLock::new();
        DEF.get_or_init(|| {
            DirectorDef::from_ron(DIRECTOR_FILE, DIRECTOR_RON).expect("built-in director data")
        })
    }

    /// Embedded data files of the director and the events, `(path, text)`.
    pub fn builtin_files() -> Vec<(&'static str, &'static str)> {
        let mut files = vec![(DIRECTOR_FILE, DIRECTOR_RON)];
        files.extend(crate::events::MODULES.iter().map(|m| (m.file, m.data)));
        files
    }

    pub fn announce_ticks(&self) -> u64 {
        Tick::ticks_for_secs(self.announce_secs)
    }

    /// Chance per roll at `absurdity` (0-100).
    pub fn chance(&self, absurdity: u8) -> f32 {
        self.chance_at_100 * absurdity.min(100) as f32 / 100.0
    }
}

/// Things the director did this tick (`crate::Event::Director`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectorEvent {
    MoodChanged {
        mood: Mood,
    },
    /// The announce phase of an event started.
    Announced {
        kind: EventKind,
        target: Option<PlayerId>,
        forced: bool,
    },
    /// Its active phase started.
    Started {
        kind: EventKind,
    },
    /// It is over.
    Ended {
        kind: EventKind,
    },
}

/// The director's state (in the world; part of the state hash once it did
/// anything).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DirectorState {
    pub mood: Mood,
    /// Tick of the next mood drift (0 = not started yet).
    pub next_mood_tick: u64,
    /// Tick of the next roll (0 = not started: the quiet start).
    pub next_roll_tick: u64,
    /// A roll succeeded while an event ran or the gap lasted: announce as
    /// soon as the gap has passed.
    pub pending: bool,
    /// No announcement before this tick (the gap after the last event).
    pub gap_until: u64,
    /// The running event.
    pub current: Option<EventState>,
    /// Bit per [`EventKind`]: events announced this match.
    pub played: u32,
    /// Events announced this match.
    pub count: u32,
    /// Forced events `(tick, kind)`, sorted by tick (then insertion).
    pub schedule: Vec<(u64, EventKind)>,
}

/// Everything the director reads from the world for one step.
pub(crate) struct DirectorInput<'a> {
    pub def: &'a DirectorDef,
    pub table: &'a EventTable,
    /// Effective Absurdity (0-100).
    pub absurdity: u8,
    /// Bit per [`EventKind`]: banned by the rule set.
    pub banned: u32,
    pub mode: &'a ModeState,
    pub mode_module: &'static dyn ModeModule,
    pub mode_ref: ModeRef<'a>,
    pub mutators: &'a [&'static dyn MutatorModule],
    pub setup: &'a MatchSetup,
}

impl DirectorState {
    /// Whether the director never did anything (state hash: skipped).
    pub fn is_pristine(&self) -> bool {
        *self == DirectorState::default()
    }

    /// Whether the director has anything to do at `absurdity`.
    pub fn is_on(&self, absurdity: u8) -> bool {
        absurdity > 0 || self.current.is_some() || !self.schedule.is_empty()
    }

    /// The replicated view.
    pub fn view(&self) -> MoodView {
        match &self.current {
            None => MoodView {
                mood: self.mood,
                ..MoodView::default()
            },
            Some(e) => MoodView {
                mood: self.mood,
                event: Some(e.kind),
                phase: e.phase,
                phase_end_tick: e.phase_end_tick,
                active_tick: e.active_tick,
                target: e.target,
                params: e.params,
            },
        }
    }

    /// Queues a forced event at `tick`.
    pub fn schedule(&mut self, tick: u64, kind: EventKind) {
        let at = self.schedule.partition_point(|(t, _)| *t <= tick);
        self.schedule.insert(at, (tick, kind));
    }

    /// One director step at the end of a tick (inside a hook context, so
    /// events can queue commands). `ended`: the match ended this tick (the
    /// running event ends with it).
    pub(crate) fn step(
        &mut self,
        ctx: &mut Ctx,
        inp: &DirectorInput,
        ended: bool,
        out: &mut Vec<DirectorEvent>,
    ) {
        let now = ctx.now();
        let def = inp.def;
        if ended {
            if self.current.is_some() {
                self.end_current(ctx, inp, out);
            }
            return;
        }
        // Mood drift (only while the Absurdity allows events).
        if inp.absurdity > 0 {
            if self.next_mood_tick == 0 {
                let mut rng = mood_rng(ctx.view.seed, now, KEY_MOOD);
                self.mood = Mood::ALL[rng.below(4) as usize];
                self.next_mood_tick = now + self.mood_period(&mut rng, def);
                if self.mood != Mood::Calm {
                    out.push(DirectorEvent::MoodChanged { mood: self.mood });
                }
            } else if now >= self.next_mood_tick {
                let mut rng = mood_rng(ctx.view.seed, now, KEY_MOOD);
                let others: Vec<Mood> = Mood::ALL.into_iter().filter(|m| *m != self.mood).collect();
                self.mood = others[rng.below(others.len() as u32) as usize];
                self.next_mood_tick = now + self.mood_period(&mut rng, def);
                out.push(DirectorEvent::MoodChanged { mood: self.mood });
            }
        }
        if self.next_roll_tick == 0 {
            self.next_roll_tick = Tick::ticks_for_secs(def.quiet_start_secs).max(1);
        }
        let close = now + def.announce_ticks() + Tick::ticks_for_secs(def.quiet_end_secs);
        let may_announce = close <= inp.mode.end_tick;
        // The roll grid: every roll_secs, busy or not.
        if inp.absurdity > 0 && now >= self.next_roll_tick {
            self.next_roll_tick = now + Tick::ticks_for_secs(def.roll_secs).max(1);
            let mut rng = mood_rng(ctx.view.seed, now, KEY_ROLL);
            if may_announce && rng.chance(def.chance(inp.absurdity)) {
                self.pending = true;
            }
        }
        // The running event.
        if let Some(mut ev) = self.current.take() {
            let table_def = inp.table.get(ev.kind);
            let module = ev.kind.module();
            if ev.phase == EventPhase::Announce && now >= ev.phase_end_tick {
                ev.phase = EventPhase::Active;
                ev.active_tick = now;
                ev.phase_end_tick = now + table_def.active_ticks();
                self.with_event(ctx, inp, &mut ev, |m, e| m.on_start(e), module);
                out.push(DirectorEvent::Started { kind: ev.kind });
            }
            if ev.phase != EventPhase::Announce {
                self.with_event(ctx, inp, &mut ev, |m, e| m.on_tick(e), module);
                if now >= ev.phase_end_tick {
                    let ending = table_def.ending_ticks();
                    if ev.phase == EventPhase::Active && ending > 0 {
                        ev.phase = EventPhase::Ending;
                        ev.phase_end_tick = now + ending;
                    } else {
                        self.current = Some(ev);
                        self.end_current(ctx, inp, out);
                        return;
                    }
                }
            }
            self.current = Some(ev);
            return;
        }
        // A forced event.
        if self.schedule.first().is_some_and(|(t, _)| *t <= now) {
            let (_, kind) = self.schedule.remove(0);
            self.announce(ctx, inp, kind, true, out);
            return;
        }
        // A successful roll waiting for the gap.
        if !self.pending || now < self.gap_until {
            return;
        }
        self.pending = false;
        if !may_announce || inp.absurdity == 0 {
            return;
        }
        let mut rng = mood_rng(ctx.view.seed, now, KEY_CHOOSE);
        if let Some(kind) = self.choose(ctx, inp, &mut rng) {
            self.announce(ctx, inp, kind, false, out);
        }
    }

    fn mood_period(&self, rng: &mut Pcg32, def: &DirectorDef) -> u64 {
        let lo = Tick::ticks_for_secs(def.mood_min_secs).max(1);
        let hi = Tick::ticks_for_secs(def.mood_max_secs).max(lo);
        lo + rng.below((hi - lo + 1).min(u32::MAX as u64) as u32) as u64
    }

    /// The events a roll can pick now, with their weights.
    pub(crate) fn candidates(&self, ctx: &Ctx, inp: &DirectorInput) -> Vec<(EventKind, f32)> {
        let leader = inp.mode_module.leader(&inp.mode_ref);
        let mut all = Vec::new();
        for (kind, d) in inp.table.iter() {
            let bit = 1u32 << kind.index();
            if !d.enabled || inp.banned & bit != 0 || d.weight <= 0.0 {
                continue;
            }
            if !inp.mode_module.allows_event(&inp.mode_ref, kind)
                || !inp.mutators.iter().all(|m| m.allows_event(kind))
            {
                continue;
            }
            let r = EventRef {
                view: &ctx.view,
                mode: inp.mode,
                setup: inp.setup,
                def: d,
                mood: self.mood,
                leader,
            };
            if !kind.module().eligible(&r) {
                continue;
            }
            let w = if d.moods.contains(&self.mood) {
                d.weight * inp.def.mood_bias
            } else {
                d.weight
            };
            all.push((kind, w, self.played & bit != 0));
        }
        // No repeats while an unplayed event is eligible.
        let fresh = all.iter().any(|(_, _, played)| !played);
        all.into_iter()
            .filter(|(_, _, played)| !fresh || !played)
            .map(|(k, w, _)| (k, w))
            .collect()
    }

    fn choose(&self, ctx: &Ctx, inp: &DirectorInput, rng: &mut Pcg32) -> Option<EventKind> {
        let c = self.candidates(ctx, inp);
        let weights: Vec<f32> = c.iter().map(|(_, w)| *w).collect();
        rng.weighted_index(&weights).map(|i| c[i].0)
    }

    fn announce(
        &mut self,
        ctx: &mut Ctx,
        inp: &DirectorInput,
        kind: EventKind,
        forced: bool,
        out: &mut Vec<DirectorEvent>,
    ) {
        let now = ctx.now();
        let mut ev = EventState::new(kind, now, inp.def.announce_ticks().max(1));
        ev.forced = forced;
        self.with_event(ctx, inp, &mut ev, |m, e| m.on_announce(e), kind.module());
        self.played |= 1 << kind.index();
        self.count += 1;
        out.push(DirectorEvent::Announced {
            kind,
            target: ev.target,
            forced,
        });
        self.current = Some(ev);
    }

    fn end_current(&mut self, ctx: &mut Ctx, inp: &DirectorInput, out: &mut Vec<DirectorEvent>) {
        let Some(mut ev) = self.current.take() else {
            return;
        };
        let now = ctx.now();
        let module = ev.kind.module();
        self.with_event(ctx, inp, &mut ev, |m, e| m.on_end(e), module);
        out.push(DirectorEvent::Ended { kind: ev.kind });
        let mut gap = Tick::ticks_for_secs(inp.def.gap_secs);
        if self.mood == Mood::Calm {
            gap = Tick::ticks_for_secs(inp.def.gap_secs * inp.def.calm_gap_scale);
        }
        self.gap_until = now + gap;
    }

    fn with_event(
        &self,
        ctx: &mut Ctx,
        inp: &DirectorInput,
        ev: &mut EventState,
        f: impl FnOnce(&dyn crate::events::EventModule, &mut EventCtx),
        module: &'static dyn crate::events::EventModule,
    ) {
        let leader = inp.mode_module.leader(&inp.mode_ref);
        let def = inp.table.get(ev.kind);
        let mut e = EventCtx {
            ctx,
            event: ev,
            mode: inp.mode,
            setup: inp.setup,
            def,
            mood: self.mood,
            leader,
        };
        f(module, &mut e);
    }

    /// Hashes the state (only call it when not pristine).
    pub(crate) fn hash(&self, h: &mut StateHasher) {
        h.write_u8(0xd1);
        h.write_u8(self.mood.index());
        h.write_u64(self.next_mood_tick);
        h.write_u64(self.next_roll_tick);
        h.write_bool(self.pending);
        h.write_u64(self.gap_until);
        h.write_u32(self.played);
        h.write_u32(self.count);
        h.write_u32(self.schedule.len() as u32);
        for (t, k) in &self.schedule {
            h.write_u64(*t);
            h.write_u8(k.index());
        }
        match &self.current {
            None => h.write_u8(0),
            Some(e) => {
                h.write_u8(1);
                h.write_u8(e.kind.index());
                h.write_u8(e.phase.index());
                h.write_u64(e.announce_tick);
                h.write_u64(e.active_tick);
                h.write_u64(e.phase_end_tick);
                h.write_u16(e.target.map_or(u16::MAX, |p| p.0));
                h.write_bytes(&e.params);
                for v in e.vars {
                    h.write_u64(v as u64);
                }
                for v in e.fvars {
                    h.write_f32(v);
                }
                h.write_vec3(e.pos);
                h.write_u32(e.entities.len() as u32);
                for id in &e.entities {
                    h.write_u32(*id);
                }
                h.write_bool(e.forced);
            }
        }
    }
}

// ----- effects (pure functions of the view; sim and client predictor) ---------

/// The scalar effects of the running event at `now` (neutral without one).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventEffects {
    pub hit_radius_scale: f32,
    pub projectile_flags: u16,
    pub crater_scale: f32,
    pub scavenge_scale: f32,
    pub well_scale: f32,
}

impl EventEffects {
    pub const NONE: EventEffects = EventEffects {
        hit_radius_scale: 1.0,
        projectile_flags: 0,
        crater_scale: 1.0,
        scavenge_scale: 1.0,
        well_scale: 1.0,
    };
}

impl Default for EventEffects {
    fn default() -> Self {
        EventEffects::NONE
    }
}

fn effect_ctx<'a>(view: &'a MoodView, seed: u64, now: u64) -> Option<(EffectCtx<'a>, EventKind)> {
    let kind = view.event?;
    if !view.effects_on() {
        return None;
    }
    Some((
        EffectCtx {
            view,
            seed,
            now,
            def: kind.def(),
        },
        kind,
    ))
}

/// The scalar effects of the event in `view` at `now`.
pub fn event_effects(view: &MoodView, seed: u64, now: u64) -> EventEffects {
    let Some((fx, kind)) = effect_ctx(view, seed, now) else {
        return EventEffects::NONE;
    };
    let m = kind.module();
    EventEffects {
        hit_radius_scale: m.hit_radius_scale(&fx),
        projectile_flags: m.projectile_flags(&fx),
        crater_scale: m.crater_scale(&fx),
        scavenge_scale: m.scavenge_scale(&fx),
        well_scale: m.well_scale(&fx),
    }
}

/// What the event in `view` does to `ship`'s movement at `now`.
pub fn move_effects(view: &MoodView, seed: u64, now: u64, ship: &ShipState) -> MoveEffects {
    match effect_ctx(view, seed, now) {
        Some((fx, kind)) => kind.module().move_effects(&fx, ship),
        None => MoveEffects::NONE,
    }
}

/// Whether the event in `view` shows `ship` to everyone at `now`.
pub fn reveals(view: &MoodView, seed: u64, now: u64, ship: &ShipState) -> bool {
    effect_ctx(view, seed, now).is_some_and(|(fx, kind)| kind.module().reveal(&fx, ship))
}

/// Whether the event in `view` lets ships take pickup spawn `spawn` at `now`.
pub fn allows_pickup(view: &MoodView, seed: u64, now: u64, spawn: u16) -> bool {
    effect_ctx(view, seed, now).is_none_or(|(fx, kind)| kind.module().allows_pickup(&fx, spawn))
}

/// Applies the forces and impulses of `fx` before `simulate_movement`
/// (and removes the velocity along a lock normal).
pub fn apply_move_effects_before(ship: &mut ShipState, fx: &MoveEffects) {
    ship.vel += fx.impulse + fx.accel * DT;
    if let Some(n) = fx.lock_normal {
        ship.vel -= n * ship.vel.dot(n);
    }
}

/// After `simulate_movement`: undoes this tick's movement and velocity
/// along a lock normal (`pos_before`: the position before moving).
pub fn apply_move_effects_after(ship: &mut ShipState, fx: &MoveEffects, pos_before: Vec3) {
    if let Some(n) = fx.lock_normal {
        ship.pos -= n * (ship.pos - pos_before).dot(n);
        ship.vel -= n * ship.vel.dot(n);
    }
}

// ----- helpers for events ------------------------------------------------------

/// A random damaged living ship, weighted toward low hull (Pizza delivery).
pub fn pick_low_hull(e: &EventCtx) -> Option<PlayerId> {
    let ships = crate::events::damaged_ships(&e.ctx.view);
    let weights: Vec<f32> = ships
        .iter()
        .map(|s| {
            let max = e.ctx.view.ship_types.get(s.ship_type).hull_max.max(1.0);
            0.1 + (1.0 - s.hull / max).clamp(0.0, 1.0)
        })
        .collect();
    let mut rng = e.rng(0);
    rng.weighted_index(&weights).map(|i| ships[i].player)
}

#[cfg(test)]
mod tests;
