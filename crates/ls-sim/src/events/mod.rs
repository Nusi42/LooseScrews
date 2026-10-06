//! Comedy events: one module per event (`events/<id>.rs`) behind
//! [`EventModule`], one data file per event (`data/events/<id>.ron`,
//! [`EventDef`]), registered in [`MODULES`] in [`EventKind`] order (append
//! only: the index is the wire id and the rule set code bit).
//!
//! The comedy director ([`crate::director`]) decides when an event runs and
//! calls its lifecycle hooks; the world consults the effect hooks of the
//! running event (see `docs/milestone-4-plan.md` §3.1):
//!
//! | Hook | When | Context |
//! | --- | --- | --- |
//! | `eligible` | a roll: can the event run now? | [`EventRef`] |
//! | `on_announce` | the 3 s announce phase starts (pick a target, parameters) | [`EventCtx`] |
//! | `on_start` | the active phase starts | [`EventCtx`] |
//! | `on_tick` | every tick of the active and ending phases | [`EventCtx`] |
//! | `on_end` | the event is over (clean up its entities) | [`EventCtx`] |
//! | `move_effects` | a ship moves (forces, impulses, plane lock; the client predictor too) | [`EffectCtx`] + the ship |
//! | `hit_radius_scale` | projectiles test ship hit spheres | [`EffectCtx`] |
//! | `projectile_flags` | a projectile is spawned (`proj_flags::CHICKEN`) | [`EffectCtx`] |
//! | `crater_scale` | a hook digs a crater (`Ctx::crater`) | [`EffectCtx`] |
//! | `scavenge_scale` | a ship scavenges (hull repaired per scrap) | [`EffectCtx`] |
//! | `well_scale` | gravity wells pull (Gravity coffee break) | [`EffectCtx`] |
//! | `level_up` | the level's up while the event runs (Bored level) | [`EffectCtx`] |
//! | `hit_plane` | projectile hits on ships ignore depth along this normal (Wrong genre) | [`EffectCtx`] |
//! | `reveal` | snapshot relevance: is this ship shown to everyone? | [`EffectCtx`] + the ship |
//! | `allows_pickup` | a ship touches a pickup | [`EffectCtx`] + the spawn index |
//! | `obstacles` | the tick starts: kinematic obstacles that push ships and absorb projectiles (Duck parade; [`crate::obstacle`]) | [`EffectCtx`] + the [`MatchSetup`] |
//!
//! Effect hooks run only during the active and ending phases and must be
//! pure functions of the [`MoodView`], the match seed and the tick, so a
//! client derives the same effects from the replicated view (prediction).
//! A hook an event does not override does nothing. Lifecycle hooks queue
//! world changes through [`Ctx`] (damage, entities, edits, cues) like
//! weapon hooks, and keep their own state in [`EventState`].
//!
//! Wave A ships every event as a stub: it is announced and runs for its
//! duration without effects (`implemented() == false`); wave B packages
//! replace the stub bodies (B1: world and movement, B2: visitors, B3:
//! spectacles). Built by B3: Cat paw, Tired announcer, Duck parade, Hat
//! rain, Disco inferno.
//!
//! Event props (ducks, hats, the paw, the mirror ball) are entities of kind
//! [`crate::EntityKind::Prop`] that their event spawns, poses every tick
//! ([`crate::Command::PoseEntity`]) and removes in `on_end`; since one event
//! runs at a time, an event finds its props by kind. Lifecycle hooks see the
//! kills since the last director step ([`EventCtx::kills`]) and may give a
//! title for the results ([`EventState::award`]).

pub mod big_head_day;
pub mod bored_level;
pub mod cat_paw;
pub mod customer_survey;
pub mod disco_inferno;
pub mod drone;
pub mod duck_parade;
pub mod everything_is_cake;
pub mod gossip_wall;
pub mod gravity_coffee_break;
pub mod hat_rain;
pub mod level_hiccups;
pub mod mimic_pickups;
pub mod moving_day;
pub mod params;
pub mod pizza_delivery;
pub mod replays;
pub mod rubber_chickens;
pub mod tax_drone;
pub mod tired_announcer;
pub mod visitor_replays;
pub mod wrong_genre;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::OnceLock;

use ls_math::{Quat, Tick, Vec3};
use serde::{Deserialize, Serialize};

use crate::behavior::{Ctx, View};
use crate::director::{Mood, MoodView};
use crate::mode::ModeState;
use crate::ship::{PlayerId, ShipState};
use crate::world::MatchSetup;
use crate::SimError;

/// The comedy events, in registry order (append only: the index is the
/// wire id in `MoodView` and the bit in rule set code event bans).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventKind {
    BoredLevel = 0,
    TaxDrone = 1,
    GravityCoffeeBreak = 2,
    RubberChickens = 3,
    CatPaw = 4,
    TiredAnnouncer = 5,
    WrongGenre = 6,
    GossipWall = 7,
    PizzaDelivery = 8,
    EverythingIsCake = 9,
    DuckParade = 10,
    HatRain = 11,
    CustomerSurvey = 12,
    DiscoInferno = 13,
    BigHeadDay = 14,
    MovingDay = 15,
    MimicPickups = 16,
    LevelHiccups = 17,
}

impl EventKind {
    /// Every event in index order.
    pub const ALL: [EventKind; 18] = [
        EventKind::BoredLevel,
        EventKind::TaxDrone,
        EventKind::GravityCoffeeBreak,
        EventKind::RubberChickens,
        EventKind::CatPaw,
        EventKind::TiredAnnouncer,
        EventKind::WrongGenre,
        EventKind::GossipWall,
        EventKind::PizzaDelivery,
        EventKind::EverythingIsCake,
        EventKind::DuckParade,
        EventKind::HatRain,
        EventKind::CustomerSurvey,
        EventKind::DiscoInferno,
        EventKind::BigHeadDay,
        EventKind::MovingDay,
        EventKind::MimicPickups,
        EventKind::LevelHiccups,
    ];

    /// Stable index (wire, rule set codes).
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Option<EventKind> {
        EventKind::ALL.get(i as usize).copied()
    }

    /// Module and data file id (`events/<id>.rs`, `data/events/<id>.ron`,
    /// the key of rule set event bans).
    pub fn id(self) -> &'static str {
        MODULES[self as usize].id
    }

    /// The event with module id `id`.
    pub fn from_id(id: &str) -> Option<EventKind> {
        MODULES.iter().find(|m| m.id == id).map(|m| m.kind)
    }

    /// The event's module.
    pub fn module(self) -> &'static dyn EventModule {
        MODULES[self as usize].module
    }

    /// The event's data ([`EventTable::builtin`]).
    pub fn def(self) -> &'static EventDef {
        EventTable::builtin().get(self)
    }
}

/// One event's numbers (`data/events/<id>.ron`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventDef {
    pub id: String,
    /// Display name (announcer banner).
    pub name: String,
    /// `false` keeps the director from rolling it (forced schedules still
    /// run it): events whose look has not passed review yet.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Base weight of a roll.
    #[serde(default = "one")]
    pub weight: f32,
    /// Length of the active phase (after the 3 s announce phase).
    pub active_secs: f32,
    /// Length of the ending phase (fade-out; 0 = none).
    #[serde(default)]
    pub ending_secs: f32,
    /// Moods that multiply its weight by `DirectorDef::mood_bias`.
    #[serde(default)]
    pub moods: Vec<Mood>,
    /// The event names a player (the announce picks a target).
    #[serde(default)]
    pub targeted: bool,
    /// Free numeric parameters for the module.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

fn yes() -> bool {
    true
}

fn one() -> f32 {
    1.0
}

impl EventDef {
    pub fn from_ron(source_name: &str, text: &str) -> Result<EventDef, SimError> {
        let def: EventDef = ron::from_str(text).map_err(|e| SimError::Parse {
            source_name: source_name.to_string(),
            message: e.to_string(),
        })?;
        def.validate()?;
        Ok(def)
    }

    pub fn validate(&self) -> Result<(), SimError> {
        let bad = |reason: &str| SimError::InvalidEvent {
            id: self.id.clone(),
            reason: reason.to_string(),
        };
        let non_neg = |v: f32| v.is_finite() && v >= 0.0;
        if EventKind::from_id(&self.id).is_none() {
            return Err(bad("no event module has this id"));
        }
        if !non_neg(self.weight) || !non_neg(self.ending_secs) {
            return Err(bad("weight and ending_secs must be >= 0"));
        }
        if !(self.active_secs.is_finite() && self.active_secs > 0.0) {
            return Err(bad("active_secs must be > 0"));
        }
        if self.params.values().any(|v| !v.is_finite()) {
            return Err(bad("params must be finite"));
        }
        Ok(())
    }

    pub fn param(&self, key: &str, default: f32) -> f32 {
        self.params.get(key).copied().unwrap_or(default)
    }

    /// Ticks of the active phase (at least 1).
    pub fn active_ticks(&self) -> u64 {
        Tick::ticks_for_secs(self.active_secs).max(1)
    }

    /// Ticks of the ending phase (0 = none).
    pub fn ending_ticks(&self) -> u64 {
        Tick::ticks_for_secs(self.ending_secs)
    }
}

/// Every event's data, indexed by [`EventKind`].
#[derive(Debug, Clone, PartialEq)]
pub struct EventTable {
    defs: Vec<EventDef>,
}

impl EventTable {
    /// The embedded `data/events/*.ron` (parsed once).
    pub fn builtin() -> &'static EventTable {
        static TABLE: OnceLock<EventTable> = OnceLock::new();
        TABLE.get_or_init(|| {
            EventTable::from_files(MODULES.iter().map(|m| (m.file, m.data)))
                .expect("built-in event data is valid")
        })
    }

    /// Parses one file per event (any order; every event needs one).
    pub fn from_files<'a>(
        files: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<EventTable, SimError> {
        let mut defs: Vec<Option<EventDef>> = vec![None; EventKind::ALL.len()];
        for (name, text) in files {
            let def = EventDef::from_ron(name, text)?;
            let kind = EventKind::from_id(&def.id).expect("validated");
            defs[kind.index() as usize] = Some(def);
        }
        let defs = defs
            .into_iter()
            .zip(EventKind::ALL)
            .map(|(d, k)| d.ok_or_else(|| SimError::UnknownEvent(k.id().to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(EventTable { defs })
    }

    pub fn get(&self, kind: EventKind) -> &EventDef {
        &self.defs[kind.index() as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = (EventKind, &EventDef)> {
        EventKind::ALL.iter().map(move |&k| (k, self.get(k)))
    }
}

/// Phase of a running event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventPhase {
    /// The 3 s telegraph: banner, sting, announcer line, in-world tell.
    /// Nothing changes yet.
    #[default]
    Announce = 0,
    /// The event does its thing.
    Active = 1,
    /// Fade-out (`EventDef::ending_secs`), then the event is over.
    Ending = 2,
}

impl EventPhase {
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Option<EventPhase> {
        match i {
            0 => Some(EventPhase::Announce),
            1 => Some(EventPhase::Active),
            2 => Some(EventPhase::Ending),
            _ => None,
        }
    }
}

/// The state of the running event (in the director, part of the state
/// hash). `params` are replicated in the [`MoodView`]; the rest is the
/// module's private scratch space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventState {
    pub kind: EventKind,
    pub phase: EventPhase,
    /// Tick the announce phase started.
    pub announce_tick: u64,
    /// Tick the active phase starts (started).
    pub active_tick: u64,
    /// Tick the current phase ends.
    pub phase_end_tick: u64,
    /// The player the event names (Tax drone, Pizza delivery, ...).
    pub target: Option<PlayerId>,
    /// Replicated parameter bytes (module-defined: axis, route, seed bits).
    pub params: [u8; 8],
    /// Module scratch values (not replicated).
    pub vars: [i64; 4],
    pub fvars: [f32; 4],
    pub pos: Vec3,
    /// Entities the event spawned (drones, ducks, hats), removed by the
    /// module in `on_end`.
    pub entities: Vec<u32>,
    /// Started by a forced schedule (`World::schedule_event`).
    pub forced: bool,
    /// Per-player module scratch (Cat paw: ships already hit; Hat rain: the
    /// tick each ship reached its hat count), kept sorted by the module.
    pub marks: Vec<(PlayerId, u64)>,
    /// A title the event awards for the results (Hat rain's Hat Baron),
    /// moved to `DirectorState::awards` when the event ends.
    pub award: Option<Award>,
}

/// A kill this tick, as the lifecycle hooks of the running event see it
/// ([`EventCtx::kills`]): Tired announcer's kill-cams, Hat rain's
/// scattered hats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KillNote {
    pub victim: PlayerId,
    /// `None`: the level (environment, an event).
    pub killer: Option<PlayerId>,
    pub weapon: Option<crate::WeaponId>,
    /// Where the victim died.
    pub pos: Vec3,
}

/// A title an event gives a player, listed in the match results (Hat
/// rain: the Hat Baron with the hat count; Customer survey: the rating).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Award {
    pub kind: EventKind,
    pub player: PlayerId,
    /// Event-defined value (hats worn, stars).
    pub value: u32,
}

impl EventState {
    pub fn new(kind: EventKind, announce_tick: u64, announce_ticks: u64) -> EventState {
        let active_tick = announce_tick + announce_ticks;
        EventState {
            kind,
            phase: EventPhase::Announce,
            announce_tick,
            active_tick,
            phase_end_tick: active_tick,
            target: None,
            params: [0; 8],
            vars: [0; 4],
            fvars: [0.0; 4],
            pos: Vec3::ZERO,
            entities: Vec::new(),
            forced: false,
            marks: Vec::new(),
            award: None,
        }
    }

    /// Ticks since the active phase started (0 before it).
    pub fn active_for(&self, now: u64) -> u64 {
        now.saturating_sub(self.active_tick)
    }
}

/// Read-only context of `eligible`.
pub struct EventRef<'a, 'v> {
    pub view: &'a View<'v>,
    pub mode: &'a ModeState,
    pub setup: &'a MatchSetup,
    pub def: &'a EventDef,
    pub mood: Mood,
    /// The mode's leader (`ModeModule::leader`): most mode score.
    pub leader: Option<PlayerId>,
}

impl EventRef<'_, '_> {
    pub fn now(&self) -> u64 {
        self.view.now
    }
}

/// Full context of the lifecycle hooks: the world view and command queue
/// ([`Ctx`]) plus the event's state.
pub struct EventCtx<'a, 'c> {
    pub ctx: &'c mut Ctx<'a>,
    pub event: &'c mut EventState,
    pub mode: &'c ModeState,
    pub setup: &'c MatchSetup,
    pub def: &'c EventDef,
    pub mood: Mood,
    /// The mode's leader (`ModeModule::leader`).
    pub leader: Option<PlayerId>,
    /// Kills earlier this tick (world stages 2-8), in order.
    pub kills: &'c [KillNote],
}

impl EventCtx<'_, '_> {
    pub fn now(&self) -> u64 {
        self.ctx.now()
    }

    /// The event's private RNG for this tick (stream `RNG_STREAM_MOOD`,
    /// keyed by the event kind and `key`).
    pub fn rng(&self, key: u64) -> ls_math::Pcg32 {
        crate::director::mood_rng(
            self.ctx.view.seed,
            self.now(),
            ((self.event.kind.index() as u64) << 32) ^ key,
        )
    }
}

/// Context of the effect hooks: only replicated state, so clients compute
/// the same effects.
#[derive(Debug, Clone, Copy)]
pub struct EffectCtx<'a> {
    pub view: &'a MoodView,
    /// Match seed.
    pub seed: u64,
    pub now: u64,
    pub def: &'a EventDef,
}

impl EffectCtx<'_> {
    /// Ticks since the active phase started.
    pub fn active_for(&self) -> u64 {
        self.now.saturating_sub(self.view.active_tick)
    }
}

/// What an event does to one ship's movement this tick
/// ([`EventModule::move_effects`]); applied by the world around
/// `simulate_movement` and by the client predictor
/// ([`crate::director::apply_move_effects_before`] / `_after`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveEffects {
    /// Extra acceleration (u/s^2), added to the velocity before moving.
    pub accel: Vec3,
    /// Velocity change this tick (u/s), added before moving.
    pub impulse: Vec3,
    /// Plane lock: the velocity component (and this tick's movement) along
    /// this unit normal is removed (Wrong genre: +z).
    pub lock_normal: Option<Vec3>,
    /// What happens to the orientation after moving (Wrong genre).
    pub facing: Facing,
}

/// Orientation rule of [`MoveEffects`], applied after moving
/// ([`crate::director::apply_move_effects_after`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Facing {
    /// The pilot's look and roll as usual.
    #[default]
    Free,
    /// Turn `blend` (0-1) of the way toward `target` this tick (normalized
    /// linear interpolation; 1 = exactly `target`).
    Toward { target: Quat, blend: f32 },
    /// Keep the forward axis in the plane perpendicular to `normal` and the
    /// right axis along it (roll levelled to the plane; pitch turns the ship
    /// inside the plane, yaw is cancelled).
    InPlane { normal: Vec3 },
}

impl MoveEffects {
    pub const NONE: MoveEffects = MoveEffects {
        accel: Vec3::ZERO,
        impulse: Vec3::ZERO,
        lock_normal: None,
        facing: Facing::Free,
    };

    pub fn is_none(&self) -> bool {
        *self == MoveEffects::NONE
    }
}

impl Default for MoveEffects {
    fn default() -> Self {
        MoveEffects::NONE
    }
}

/// The hooks of one comedy event (see the module docs).
pub trait EventModule: Send + Sync + Debug {
    fn kind(&self) -> EventKind;

    /// Whether the module implements the event (false: an announce-only stub).
    fn implemented(&self) -> bool {
        false
    }

    /// Whether the event can run now (its needs: wells, a damaged player, a
    /// leader, pickups, ...). Bans, vetoes and `enabled` are checked by the
    /// director.
    fn eligible(&self, _e: &EventRef) -> bool {
        true
    }

    /// The announce phase starts: pick the target and parameters.
    fn on_announce(&self, _e: &mut EventCtx) {}

    /// The active phase starts.
    fn on_start(&self, _e: &mut EventCtx) {}

    /// Every tick of the active and ending phases (after `on_start` on its
    /// first tick).
    fn on_tick(&self, _e: &mut EventCtx) {}

    /// The event is over (also when the match ends while it runs).
    fn on_end(&self, _e: &mut EventCtx) {}

    // ----- effect hooks (active and ending phases; pure) ---------------------

    fn move_effects(&self, _fx: &EffectCtx, _ship: &ShipState) -> MoveEffects {
        MoveEffects::NONE
    }

    /// Scale of ship hit spheres against projectiles (Big head day: 3).
    fn hit_radius_scale(&self, _fx: &EffectCtx) -> f32 {
        1.0
    }

    /// Flags added to every new projectile (Rubber chickens: `CHICKEN`).
    fn projectile_flags(&self, _fx: &EffectCtx) -> u16 {
        0
    }

    /// Scale of crater radii (Everything is cake: 1.44).
    fn crater_scale(&self, _fx: &EffectCtx) -> f32 {
        1.0
    }

    /// Hull repaired per unit of scavenged scrap, as a multiple of normal
    /// (Everything is cake: 2).
    fn scavenge_scale(&self, _fx: &EffectCtx) -> f32 {
        1.0
    }

    /// Scale of every gravity well's pull (Gravity coffee break: 0).
    fn well_scale(&self, _fx: &EffectCtx) -> f32 {
        1.0
    }

    /// The level's up while the event runs (Bored level: turning). The
    /// world keeps the last value after the event (`World::up`).
    fn level_up(&self, _fx: &EffectCtx) -> Option<Vec3> {
        None
    }

    /// Projectile hits on ships ignore depth along this unit normal (Wrong
    /// genre: +z; the ship's centre is moved into the projectile's plane).
    fn hit_plane(&self, _fx: &EffectCtx) -> Option<Vec3> {
        None
    }

    /// Whether `ship` is shown to everyone (Gossip wall, Hat rain).
    fn reveal(&self, _fx: &EffectCtx, _ship: &ShipState) -> bool {
        false
    }

    /// Whether pickup spawn `spawn` can be taken now (Moving day).
    fn allows_pickup(&self, _fx: &EffectCtx, _spawn: u16) -> bool {
        true
    }

    /// Kinematic obstacles at `fx.now` (Duck parade's ducks): appended to
    /// `out`; they push ships aside and absorb projectiles
    /// ([`crate::obstacle`]). Pure like every effect hook, and read from
    /// the [`MatchSetup`] that clients build from the share code too.
    fn obstacles(
        &self,
        _fx: &EffectCtx,
        _setup: &MatchSetup,
        _out: &mut Vec<crate::obstacle::Obstacle>,
    ) {
    }
}

/// One registered event: module and data file.
#[derive(Debug, Clone, Copy)]
pub struct EventEntry {
    pub kind: EventKind,
    pub id: &'static str,
    pub module: &'static dyn EventModule,
    /// Path relative to `data/` (data hash).
    pub file: &'static str,
    pub data: &'static str,
}

macro_rules! entry {
    ($kind:ident, $id:ident, $ty:ident) => {
        EventEntry {
            kind: EventKind::$kind,
            id: stringify!($id),
            module: &$id::$ty,
            file: concat!("events/", stringify!($id), ".ron"),
            data: include_str!(concat!("../../../../data/events/", stringify!($id), ".ron")),
        }
    };
}

/// Every event, in [`EventKind`] order.
pub static MODULES: [EventEntry; 18] = [
    entry!(BoredLevel, bored_level, BoredLevel),
    entry!(TaxDrone, tax_drone, TaxDrone),
    entry!(GravityCoffeeBreak, gravity_coffee_break, GravityCoffeeBreak),
    entry!(RubberChickens, rubber_chickens, RubberChickens),
    entry!(CatPaw, cat_paw, CatPaw),
    entry!(TiredAnnouncer, tired_announcer, TiredAnnouncer),
    entry!(WrongGenre, wrong_genre, WrongGenre),
    entry!(GossipWall, gossip_wall, GossipWall),
    entry!(PizzaDelivery, pizza_delivery, PizzaDelivery),
    entry!(EverythingIsCake, everything_is_cake, EverythingIsCake),
    entry!(DuckParade, duck_parade, DuckParade),
    entry!(HatRain, hat_rain, HatRain),
    entry!(CustomerSurvey, customer_survey, CustomerSurvey),
    entry!(DiscoInferno, disco_inferno, DiscoInferno),
    entry!(BigHeadDay, big_head_day, BigHeadDay),
    entry!(MovingDay, moving_day, MovingDay),
    entry!(MimicPickups, mimic_pickups, MimicPickups),
    entry!(LevelHiccups, level_hiccups, LevelHiccups),
];

/// Event ids in registry order (rule set code bits).
pub fn event_ids() -> Vec<&'static str> {
    MODULES.iter().map(|m| m.id).collect()
}

// ----- helpers for eligibility ------------------------------------------------

/// Pickup spawn points that hold something (weapons, upgrades, gadgets;
/// not scrap).
pub fn pickups_in_play(view: &View) -> usize {
    view.pickups
        .iter()
        .filter(|p| {
            p.content
                .is_some_and(|(_, k)| !matches!(k, crate::PickupKind::Scrap { .. }))
        })
        .count()
}

/// Living ships that are not at full hull, in id order.
pub fn damaged_ships<'v>(view: &View<'v>) -> Vec<&'v ShipState> {
    view.ships
        .values()
        .filter(|s| s.alive && s.hull < view.ship_types.get(s.ship_type).hull_max)
        .collect()
}

#[cfg(test)]
mod spectacle_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod visitor_tests;
#[cfg(test)]
mod world_events_tests;
