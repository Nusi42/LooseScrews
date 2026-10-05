//! The deterministic match simulation: ships, projectiles, entities, pickups,
//! scrap and the Paint the Void mode, advanced one 60 Hz tick at a time by
//! [`World::step`].
//!
//! Weapons and gadgets are data plus behaviour modules
//! ([`crate::weapons`], [`crate::gadgets`]); this file is the framework that
//! calls their hooks and applies the [`Command`]s they queue. The order of a
//! tick (see `docs/architecture/arsenal-framework.md`):
//!
//! 1. respawns (with the ship type chosen for the next life);
//! 2. per ship in id order: weapon selection, gadget key, movement (frozen
//!    while the ship guides a projectile), scavenging (and Tug scrap passing),
//!    gadget drain, then firing (primary, secondary);
//! 3. projectiles (fuse, lifetime, `update`, swept move and hit hooks);
//! 4. entities (debris physics, decoys, entity hooks, expiry);
//! 5. scrap piles and pickups;
//! 6. ship trails (Borrowed Time history);
//! 7. world physics (M4): radiation on belt levels, pigment pickups on
//!    monochrome levels, ended dynamic wells, Instability's strain decay and
//!    cave-ins;
//! 8. mode: the mode's `on_tick` hook, then `end_check` (timer, scores,
//!    eliminations; see [`crate::modes`]);
//! 9. comedy director: mood, the running event's hooks, rolls (see
//!    [`crate::director`]).
//!
//! Gravity wells act inside stages 2-4: ships through their movement
//! environment ([`crate::ship::MoveEnv`]), flagged projectiles before their
//! swept move, debris before its physics step.
//!
//! Before stage 1: the mode's `on_start` hook on the first tick, then the
//! deferred terrain edits due this tick ([`Ctx::later`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use ls_math::terrain::{RayHit, SurfaceStates, TerrainEdit, TerrainQuery};
use ls_math::{Aabb, Pcg32, Quat, Vec3, DT};
use serde::{Deserialize, Serialize};

use crate::behavior::{gadget_cue, Command, Ctx, Cue, CueKind, DamageSource, View};
use crate::director::{
    DirectorDef, DirectorEvent, DirectorInput, DirectorState, EventEffects, MoodView,
};
use crate::entity::{
    state_byte, step_debris, step_debris_pulled, Entity, EntityKind, EntitySpawn, EntityView,
    ReplicatedKind, Source, DEBRIS_PUSH, MAX_DEBRIS, NO_TEAM, WIRE_ID_MASK,
};
use crate::events::{EventTable, MoveEffects};
use crate::gadgets::{GadgetActivation, GadgetBehavior, GadgetUse, IncomingHit};
use crate::gravity::{DynamicWell, GravitySettings, WellField};
use crate::hash::StateHasher;
use crate::instability::{InstabilitySettings, InstabilityState};
use crate::mode::{
    player_flags, MatchPhase, MatchResults, ModeState, ModeView, ResultRow, MAX_FFA_SIDE, MAX_TEAMS,
};
use crate::modes::{Kill, ModeCtx, ModeHit, ModeModule, ModeMut, ModeRef};
use crate::mutators::MutatorModule;
use crate::pigment::{PigmentPickup, PigmentPickupRule, PigmentSettings, PigmentState};
use crate::ruleset::{PickupKind, ResolvedRules};
use crate::ship::{
    move_sphere, simulate_movement, simulate_movement_env, ChargeState, PlayerId, ShipState,
    WeaponState,
};
use crate::terrain::{segment_sphere, SimTerrain, TerrainLayers};
use crate::visual::{BuiltinVisual, VisualId, VisualTable};
use crate::weapons::{ChargeAction, EntityHit, HitResponse, ShipHit, Shot, WeaponBehavior};
use crate::{
    Buttons, GadgetId, GadgetTable, InputCmd, RuleSet, ShieldInteraction, ShipTable, ShipTuning,
    ShipType, SimError, WeaponCost, WeaponDef, WeaponId, WeaponSlot, WeaponTable,
};

/// Distance (added to the ship radius) within which a ship collects a pickup.
pub const PICKUP_RADIUS: f32 = 1.0;
/// Maximum number of wreck scrap piles; the oldest wreck is removed past this.
pub const MAX_WRECKS: usize = 64;
/// RNG stream for pickup rolls (see `docs/architecture/contracts.md`, RNG streams).
pub const RNG_STREAM_PICKUPS: u64 = 0x5349_4d5f_5049_434b; // "SIM_PICK"
/// Ticks of position/velocity/hull history kept per ship (3 s, Borrowed Time).
pub const TRAIL_TICKS: usize = 180;
/// Hit iterations per projectile and tick (pierce and pass-through chains).
const MAX_HITS_PER_TICK: usize = 8;
/// Energy drained per second from a ship scraping a charged surface, unless
/// the Leech Arc data sets `charged_drain_per_sec` (M2 plan: 5 energy/s).
pub const CHARGED_DRAIN_PER_SEC: f32 = 5.0;
/// Wire ids of projectiles are `id & PROJECTILE_WIRE_MASK` (16 bits).
pub const PROJECTILE_WIRE_MASK: u32 = 0xffff;

/// A team spawn point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpawnPoint {
    pub pos: Vec3,
    pub team: u8,
    pub orientation: Quat,
}

/// Level-specific placement (supplied by the generator from the seed later).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MatchSetup {
    pub spawns: Vec<SpawnPoint>,
    pub pickup_points: Vec<Vec3>,
    /// The level's Instability slider (0-100; scales Tectonic Charge
    /// collapses). Hooks read it as `ctx.view.instability`.
    #[serde(default)]
    pub instability: u8,
    /// The level's Heist site (core vein, extraction zones); Heist only.
    /// `None` (test rooms): Heist falls back to the spawn points.
    #[serde(default)]
    pub heist: Option<crate::modes::heist::HeistLevel>,
    /// The level's gravity well sites (Gravity Chaos; on belt levels the
    /// shepherd moons), from `ls_worldgen::Generator::gravity_wells`.
    #[serde(default)]
    pub wells: Vec<ls_math::level::WellSite>,
    /// The asteroid belt (belt levels only; Belt Up, radiation), from
    /// `ls_worldgen::Generator::belt`.
    #[serde(default)]
    pub belt: Option<ls_math::level::BeltSite>,
    /// The level's Absurdity slider (0-100): the comedy director runs at
    /// `min(absurdity, RuleSet::absurdity)` ([`crate::director`]). 0 (test
    /// rooms, scripted replays): no events.
    #[serde(default)]
    pub absurdity: u8,
    /// The level's Gravity Chaos slider (0-100): strength and range of the
    /// static `wells` (`data/world/gravity.ron`). M4.
    #[serde(default)]
    pub gravity_chaos: u8,
    /// A monochrome level (Palette 80 or more): pigment pickups
    /// ([`crate::pigment`]). Last Color Standing is monochrome anyway. M4.
    #[serde(default)]
    pub monochrome: bool,
}

/// Everything needed to start a match.
#[derive(Debug, Clone)]
pub struct WorldConfig {
    /// Match seed (pickup rolls, module RNG). Usually derived from the world seed.
    pub seed: u64,
    pub weapons: WeaponTable,
    /// Tuning of every ship type.
    pub ships: ShipTable,
    pub gadgets: GadgetTable,
    pub rules: RuleSet,
    pub setup: MatchSetup,
}

impl WorldConfig {
    /// Built-in data and the default Paint the Void rule set.
    pub fn paint_the_void(seed: u64, setup: MatchSetup) -> WorldConfig {
        WorldConfig {
            seed,
            weapons: WeaponTable::builtin(),
            ships: ShipTable::builtin(),
            gadgets: GadgetTable::builtin(),
            rules: RuleSet::paint_the_void(),
            setup,
        }
    }
}

/// Immutable per-match configuration shared by clones of a world.
#[derive(Debug)]
struct SimConfig {
    seed: u64,
    weapons: WeaponTable,
    ships: ShipTable,
    gadgets: GadgetTable,
    rules: RuleSet,
    resolved: ResolvedRules,
    setup: MatchSetup,
    visuals: VisualTable,
    /// Indexed by [`WeaponId`].
    behaviors: Vec<&'static dyn WeaponBehavior>,
    /// Indexed by [`GadgetId`].
    gadget_behaviors: Vec<&'static dyn GadgetBehavior>,
    /// Energy per second a charged surface drains from a ship touching it.
    charged_drain_per_sec: f32,
    /// The mode's module and the mutators' (in rule set order).
    mode_module: &'static dyn ModeModule,
    mutator_modules: Vec<&'static dyn MutatorModule>,
    /// The comedy director's effective Absurdity: `min(level, rule set)`.
    absurdity: u8,
    /// Bit per `EventKind`: events the rule set bans.
    banned_events: u32,
    /// Pigment pickups of this match (monochrome levels, Last Color
    /// Standing), `None` otherwise.
    pigment_rule: Option<PigmentPickupRule>,
}

impl SimConfig {
    fn behavior(&self, w: WeaponId) -> &'static dyn WeaponBehavior {
        self.behaviors[w.0 as usize]
    }

    fn tuning(&self, t: ShipType) -> &ShipTuning {
        self.ships.get(t)
    }
}

/// Bits of [`Projectile::flags`]. The low 8 bits are replicated to clients
/// (`Projectile::net_state`).
pub mod proj_flags {
    /// Stuck to terrain (does not move).
    pub const STUCK: u16 = 1 << 0;
    /// Armed (mines).
    pub const ARMED: u16 = 1 << 1;
    /// Can hit its owner (ricochets, returning echoes).
    pub const HITS_OWNER: u16 = 1 << 2;
    /// The owner steers it: the owner's ship gets no thrust or look input
    /// while it lives (Eyeball).
    pub const CONTROLS_OWNER: u16 = 1 << 3;
    /// On its way back (Echo Cannon).
    pub const RETURNING: u16 = 1 << 4;
    /// Passes through ships (hits only terrain and entities).
    pub const NO_SHIP_HITS: u16 = 1 << 5;
    /// Module-defined replicated bits.
    pub const CUSTOM_A: u16 = 1 << 6;
    pub const CUSTOM_B: u16 = 1 << 7;
    /// Replicated bits.
    pub const NET_MASK: u16 = 0x00ff;
    /// Drawn as a rubber chicken (Rubber chickens event; flight, damage and
    /// craters unchanged). Replicated as `ls_net::NetProjectile::chicken`
    /// (protocol 6), not in the state byte.
    pub const CHICKEN: u16 = 1 << 8;
    /// Removed by a hook (not replicated; the projectile is gone after the hook).
    pub const REMOVED: u16 = 1 << 15;
}

/// A projectile in flight (bolts, rounds, missiles, mines, charges). Its
/// velocity is the muzzle velocity only; it never inherits the shooter's
/// velocity. Fields after `knockback` are generic state for weapon modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Projectile {
    pub id: u32,
    pub owner: PlayerId,
    pub team: u8,
    pub weapon: WeaponId,
    pub pos: Vec3,
    pub vel: Vec3,
    /// The projectile is removed at the start of this tick.
    pub expires_tick: u64,
    /// Direct-hit damage (all multipliers applied).
    pub damage: f32,
    /// Splash damage at the center (all multipliers applied).
    pub splash_damage: f32,
    /// Knockback impulse (u/s) of a direct hit, scaled like damage; splash
    /// knockback scales it by the falloff.
    pub knockback: f32,
    pub spawn_tick: u64,
    /// Collision radius (the weapon's `projectile_radius` by default).
    pub radius: f32,
    pub visual: VisualId,
    /// Charge fraction the shot was fired with (0..=1).
    pub charge: f32,
    /// 0 for fired projectiles, parent + 1 for sub-projectiles.
    pub generation: u8,
    /// Bounces so far (ricochets).
    pub bounces: u8,
    /// See [`proj_flags`].
    pub flags: u16,
    /// Tick from which a mine is armed (modules decide what arming means).
    pub arm_tick: u64,
    /// `on_fuse` runs at this tick (`u64::MAX` = no fuse).
    pub fuse_tick: u64,
    /// Locked target (homing).
    pub target: Option<PlayerId>,
    /// Free per-module state.
    pub vars: [f32; 4],
    /// Recorded path points (Echo Cannon), kept small by the module.
    pub path: Vec<Vec3>,
    /// The ship (`(0, player id)`) or entity (`(1, entity id)`) a `Continue`
    /// hit last passed through: it is not hit again until the projectile
    /// has left its sphere (set by the framework).
    #[serde(default)]
    pub passed: Option<(u8, u32)>,
}

impl Projectile {
    /// Removes the projectile after the current hook.
    pub fn remove(&mut self) {
        self.flags |= proj_flags::REMOVED;
    }

    pub fn is_removed(&self) -> bool {
        self.flags & proj_flags::REMOVED != 0
    }

    pub fn is_stuck(&self) -> bool {
        self.flags & proj_flags::STUCK != 0
    }

    /// Replicated state byte (low bits of `flags`).
    pub fn net_state(&self) -> u8 {
        (self.flags & proj_flags::NET_MASK) as u8
    }
}

/// Scavengeable scrap: wrecks of destroyed ships, scrap pickups and level junk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrapPile {
    pub id: u32,
    pub pos: Vec3,
    pub amount: f32,
    /// Pickup spawn index if this pile is a scrap pickup (respawns when empty).
    pub spawn: Option<u16>,
    /// Left by a destroyed ship.
    pub wreck: bool,
}

/// A pickup spawn point and what it currently holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PickupSpawn {
    pub pos: Vec3,
    /// Pickup table entry index and content, or `None` while respawning.
    pub content: Option<(u16, PickupKind)>,
    pub respawn_tick: u64,
    /// Entity id of the current content (replication).
    pub id: u32,
    /// How far the current content was moved off the spawn point (Magnet
    /// Sneeze, [`Command::MovePickup`]); zero again when new content spawns.
    #[serde(default)]
    pub offset: Vec3,
}

impl PickupSpawn {
    /// Where the current content is (spawn point plus [`Self::offset`]).
    pub fn content_pos(&self) -> Vec3 {
        self.pos + self.offset
    }
}

/// Things that happened during a tick, for clients, the server and tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// A projectile left a gun (sub-projectiles do not count).
    Fired {
        player: PlayerId,
        weapon: WeaponId,
        projectile: u32,
    },
    /// A projectile hit a ship, an entity or terrain (or exploded).
    Impact {
        projectile: u32,
        weapon: WeaponId,
        point: Vec3,
        target: Option<PlayerId>,
    },
    Damaged {
        target: PlayerId,
        /// `None` for damage without an attacker (environment).
        attacker: Option<PlayerId>,
        /// `None` for damage not caused by a weapon (decoy return hit, ...).
        weapon: Option<WeaponId>,
        amount: f32,
        direct: bool,
    },
    Killed {
        victim: PlayerId,
        killer: Option<PlayerId>,
        weapon: Option<WeaponId>,
    },
    Respawned {
        player: PlayerId,
        pos: Vec3,
    },
    /// A terrain edit (crater, growth or paint) exactly as applied to the
    /// terrain and logged (craters already scaled by rock strength, see
    /// [`Ctx::crater`]).
    TerrainEdit(TerrainEdit),
    PickupSpawned {
        spawn: u16,
        kind: PickupKind,
    },
    PickupTaken {
        spawn: u16,
        player: PlayerId,
        kind: PickupKind,
    },
    ScoreChanged {
        team_cells: [u64; 4],
        surface_cells: u64,
    },
    /// A player is out of the match (no lives or colour left); their ship
    /// left play.
    Eliminated {
        player: PlayerId,
    },
    /// The match is over: `winner` is the winning side (team, or the
    /// winner's side id in free-for-all), `None` on a draw.
    MatchEnded {
        winner: Option<u8>,
        team_cells: [u64; 4],
    },
    /// Something for clients to show or play (warnings, blinding, chimes).
    Cue(Cue),
    /// The comedy director changed the mood or an event's phase.
    Director(DirectorEvent),
}

/// Recent history of one ship (newest last).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Trail {
    points: VecDeque<(Vec3, Vec3, f32)>,
}

impl Trail {
    /// `(pos, vel, hull)` `ticks_ago` ticks before the newest record (the
    /// oldest record if the history is shorter).
    pub(crate) fn get(&self, ticks_ago: u64) -> Option<(Vec3, Vec3, f32)> {
        let n = self.points.len();
        if n == 0 {
            return None;
        }
        let i = (n - 1).saturating_sub(ticks_ago.min(n as u64) as usize);
        self.points.get(i).copied()
    }

    fn push(&mut self, p: (Vec3, Vec3, f32)) {
        if self.points.len() > TRAIL_TICKS {
            self.points.pop_front();
        }
        self.points.push_back(p);
    }
}

/// Terrain reference usable as `&dyn TerrainQuery` for any (unsized)
/// terrain, with the mode's material override (`ModeModule::material_at`).
struct TerrainRef<'a, T: ?Sized>(&'a T, Option<ModeMaterial<'a>>);

/// What `TerrainRef` needs for the mode's material override (and the
/// rubble regions of cave-ins, which are weak rock).
struct ModeMaterial<'a> {
    module: &'a dyn ModeModule,
    state: &'a ModeState,
    rules: &'a crate::ruleset::ModeRules,
    now: u64,
    instability: &'a InstabilityState,
}

impl<T: SimTerrain + ?Sized> TerrainLayers for TerrainRef<'_, T> {
    fn surface_states_near(&self, center: Vec3, radius: f32, now: u64) -> SurfaceStates {
        self.0.surface_states_near(center, radius, now)
    }
    fn material_at(&self, p: Vec3) -> u8 {
        self.1
            .as_ref()
            .and_then(|m| {
                m.module
                    .material_at(m.state, m.rules, m.now, p)
                    .or_else(|| m.instability.material_at(p))
            })
            .unwrap_or_else(|| self.0.material_at(p))
    }
}

impl<T: TerrainQuery + ?Sized> TerrainQuery for TerrainRef<'_, T> {
    fn sdf(&self, p: Vec3) -> f32 {
        self.0.sdf(p)
    }
    fn normal(&self, p: Vec3) -> Vec3 {
        self.0.normal(p)
    }
    fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RayHit> {
        self.0.raycast(origin, dir, max_dist)
    }
    fn bounds(&self) -> Aabb {
        self.0.bounds()
    }
    fn sphere_overlap(&self, center: Vec3, radius: f32) -> bool {
        self.0.sphere_overlap(center, radius)
    }
    fn paint_colors_near(&self, center: Vec3, radius: f32) -> u8 {
        self.0.paint_colors_near(center, radius)
    }
}

/// The simulation state of one match. Cloning is cheap enough for client-side
/// rollback (configuration is shared).
#[derive(Debug, Clone)]
pub struct World {
    config: Arc<SimConfig>,
    tick: u64,
    ships: BTreeMap<PlayerId, ShipState>,
    projectiles: Vec<Projectile>,
    entities: Vec<Entity>,
    scrap: Vec<ScrapPile>,
    pickups: Vec<PickupSpawn>,
    mode: ModeState,
    rng: Pcg32,
    next_player: u16,
    next_projectile: u32,
    next_entity: u32,
    /// Next spawn point per team (free-for-all uses slot 0).
    spawn_cursor: [u32; MAX_TEAMS as usize],
    trails: BTreeMap<PlayerId, Trail>,
    /// This tick's sanitized inputs (hooks read them through the view).
    inputs: BTreeMap<PlayerId, InputCmd>,
    /// While projectiles are stepped: the ones not stepped yet (in reverse
    /// order) and the ones already stepped this tick. Empty otherwise.
    stepping: Vec<Projectile>,
    stepped: Vec<Projectile>,
    /// Terrain edits queued for later ticks (`Ctx::later`), in queue order.
    deferred_edits: Vec<(u64, TerrainEdit)>,
    /// The comedy director (mood, running event, rolls).
    director: DirectorState,
    /// The running event's scalar effects this tick (derived from the
    /// director at the start of each tick; not hashed).
    effects: EventEffects,
    /// The level's up (M4; horizon assist levels to it; Bored level turns it).
    up: Vec3,
    /// Static wells of the level plus the dynamic wells (M4).
    wells: WellField,
    /// Strain cells, the coming cave-in and rubble regions (M4).
    instability: InstabilityState,
    /// Pigment pickups (M4, monochrome levels).
    pigment: PigmentState,
    /// Set while a cave-in applies its own edits (they add no strain).
    no_strain: bool,
}

impl World {
    /// Validates the configuration and starts a match at tick 0. Pickups are
    /// rolled immediately.
    pub fn new(config: WorldConfig) -> Result<World, SimError> {
        let resolved = config
            .rules
            .resolve_with(&config.weapons, &config.gadgets)?;
        for s in &config.setup.spawns {
            if s.team >= MAX_TEAMS {
                return Err(SimError::InvalidTeam(s.team));
            }
        }
        if config.setup.pickup_points.len() > u16::MAX as usize {
            return Err(SimError::InvalidRuleSet("too many pickup points".into()));
        }
        // A team without spawn points of its own (3 or 4 teams on a level
        // set up for two) spawns at any spawn point.
        if config.setup.spawns.is_empty() {
            return Err(SimError::InvalidRuleSet("no spawn points".into()));
        }
        // Reject loadouts an allowed ship type cannot carry instead of
        // trimming them at respawn.
        let slots = resolved
            .allowed_ship_types
            .iter()
            .map(|t| config.ships.get(*t).secondary_slots as usize)
            .min()
            .unwrap_or(0);
        if resolved.loadout_secondaries.len() > slots {
            return Err(SimError::InvalidRuleSet(format!(
                "starting loadout has {} secondaries, an allowed ship type has {slots} slots",
                resolved.loadout_secondaries.len()
            )));
        }
        for &(w, count) in &resolved.loadout_secondaries {
            let max = resolved.max_ammo(w);
            if count > max {
                return Err(SimError::InvalidRuleSet(format!(
                    "starting loadout gives {count} {}, max_ammo is {max}",
                    config.weapons.get(w).id
                )));
            }
        }
        let mode = ModeState::new(
            config.rules.mode.kind(),
            resolved.teams,
            resolved.match_ticks,
        );
        let mode_module = crate::modes::module(config.rules.mode.kind());
        let mutator_modules = config
            .rules
            .mutators
            .iter()
            .map(|m| crate::mutators::module(*m))
            .collect();
        let pickups = config
            .setup
            .pickup_points
            .iter()
            .map(|&pos| PickupSpawn {
                pos,
                content: None,
                respawn_tick: 0,
                id: 0,
                offset: Vec3::ZERO,
            })
            .collect();
        let behaviors = config
            .weapons
            .iter()
            .map(|(_, d)| crate::weapons::behavior(&d.id))
            .collect();
        let gadget_behaviors = config
            .gadgets
            .iter()
            .map(|(_, d)| crate::gadgets::behavior(&d.id))
            .collect();
        let visuals = VisualTable::new(&config.weapons, &config.gadgets);
        let wells = WellField::new(
            &config.setup.wells,
            config.setup.gravity_chaos,
            GravitySettings::builtin(),
        );
        let pigment_rule = {
            let p = PigmentSettings::builtin();
            if config.rules.mode.kind() == crate::mode::ModeKind::LastColorStanding {
                Some(p.last_color_standing)
            } else if config.setup.monochrome {
                Some(p.monochrome)
            } else {
                None
            }
        };
        let charged_drain_per_sec = config
            .weapons
            .iter()
            .find(|(_, d)| d.id == "leech_arc")
            .map_or(CHARGED_DRAIN_PER_SEC, |(_, d)| {
                d.param("charged_drain_per_sec", CHARGED_DRAIN_PER_SEC)
            });
        let absurdity = config.setup.absurdity.min(config.rules.absurdity);
        let banned_events = config.rules.banned_events();
        let mut world = World {
            rng: Pcg32::from_seed(config.seed, RNG_STREAM_PICKUPS),
            config: Arc::new(SimConfig {
                seed: config.seed,
                weapons: config.weapons,
                ships: config.ships,
                gadgets: config.gadgets,
                rules: config.rules,
                resolved,
                setup: config.setup,
                visuals,
                behaviors,
                gadget_behaviors,
                charged_drain_per_sec,
                mode_module,
                mutator_modules,
                absurdity,
                banned_events,
                pigment_rule,
            }),
            tick: 0,
            ships: BTreeMap::new(),
            projectiles: Vec::new(),
            entities: Vec::new(),
            scrap: Vec::new(),
            pickups,
            mode,
            next_player: 0,
            next_projectile: 0,
            next_entity: 0,
            spawn_cursor: [0; MAX_TEAMS as usize],
            trails: BTreeMap::new(),
            inputs: BTreeMap::new(),
            stepping: Vec::new(),
            stepped: Vec::new(),
            deferred_edits: Vec::new(),
            director: DirectorState::default(),
            effects: EventEffects::NONE,
            up: Vec3::Y,
            wells,
            instability: InstabilityState::default(),
            pigment: PigmentState::default(),
            no_strain: false,
        };
        let mut events = Vec::new();
        world.update_pickup_spawns(&mut events);
        Ok(world)
    }

    // ----- accessors -------------------------------------------------------

    /// Number of ticks simulated so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn weapons(&self) -> &WeaponTable {
        &self.config.weapons
    }
    pub fn gadgets(&self) -> &GadgetTable {
        &self.config.gadgets
    }
    pub fn ship_types(&self) -> &ShipTable {
        &self.config.ships
    }
    pub fn rules(&self) -> &RuleSet {
        &self.config.rules
    }
    pub fn resolved_rules(&self) -> &ResolvedRules {
        &self.config.resolved
    }
    pub fn visuals(&self) -> &VisualTable {
        &self.config.visuals
    }
    /// Tuning of the standard ship type (Tinker).
    pub fn tuning(&self) -> &ShipTuning {
        self.config.ships.get(ShipType::Tinker)
    }
    /// Tuning of a ship type.
    pub fn tuning_for(&self, t: ShipType) -> &ShipTuning {
        self.config.ships.get(t)
    }
    pub fn ships(&self) -> impl Iterator<Item = &ShipState> {
        self.ships.values()
    }
    pub fn ship(&self, id: PlayerId) -> Option<&ShipState> {
        self.ships.get(&id)
    }
    /// Direct mutable access (tests, tools, server admin commands). Changes made
    /// here bypass rule enforcement.
    pub fn ship_mut(&mut self, id: PlayerId) -> Option<&mut ShipState> {
        self.ships.get_mut(&id)
    }

    /// Moves a ship by `delta` together with the newest sample of its trail
    /// (Borrowed Time history), so the history matches the moved ship. Used
    /// by the server to undo a lag-compensation offset after a step.
    pub fn shift_ship(&mut self, id: PlayerId, delta: Vec3) {
        if let Some(s) = self.ships.get_mut(&id) {
            s.pos += delta;
        }
        if let Some(p) = self.trails.get_mut(&id).and_then(|t| t.points.back_mut()) {
            p.0 += delta;
        }
    }
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
    }
    /// Generic entities (debris, decoys, ghosts, dust clouds, markers).
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }
    pub fn scrap_piles(&self) -> &[ScrapPile] {
        &self.scrap
    }
    pub fn pickups(&self) -> &[PickupSpawn] {
        &self.pickups
    }
    pub fn mode(&self) -> &ModeState {
        &self.mode
    }
    /// The mode's terrain shell at the current tick (what the last
    /// [`World::step`] handed the terrain; The Squeeze's walls).
    pub fn terrain_shell(&self) -> Option<crate::ShellSphere> {
        self.config.mode_module.terrain_shell(&self.mode, self.tick)
    }
    /// Mutable mode state (admin commands such as ending a match early,
    /// host tests). Bypasses the rules.
    pub fn mode_mut(&mut self) -> &mut ModeState {
        &mut self.mode
    }

    /// Number of teams (0 = free-for-all: every ship is its own side, with
    /// its own side id in `ShipState::team`).
    pub fn teams(&self) -> u8 {
        self.mode.teams
    }

    /// The replicated mode state (scores, phase, timer, lives, objectives,
    /// the director's mood and event).
    pub fn mode_view(&self) -> ModeView {
        let cfg = &self.config;
        let mut view = crate::modes::base_view(
            &self.mode,
            &self.ships,
            &cfg.rules.mode,
            &cfg.rules.mutators,
        );
        cfg.mode_module.view(&self.mode, &cfg.rules.mode, &mut view);
        view.mood = self.director.view();
        view
    }

    /// The comedy director's state.
    pub fn director(&self) -> &DirectorState {
        &self.director
    }

    /// The replicated director state (also in [`World::mode_view`]).
    pub fn mood_view(&self) -> MoodView {
        self.director.view()
    }

    /// The director's effective Absurdity: `min(MatchSetup::absurdity,
    /// RuleSet::absurdity)`.
    pub fn effective_absurdity(&self) -> u8 {
        self.config.absurdity
    }

    /// The running event's scalar effects this tick.
    pub fn event_effects(&self) -> EventEffects {
        self.effects
    }

    /// Forces comedy event `kind` at `tick` (or as soon as the running event
    /// ended): practice `?event=<id>`, the server's `--event-schedule`,
    /// replays and tests. Ignores the Absurdity, bans, vetoes, `enabled`
    /// and eligibility.
    pub fn schedule_event(&mut self, tick: u64, kind: crate::events::EventKind) {
        self.director.schedule(tick, kind);
    }

    /// The level placement this match runs on (spawns, pickup points, sliders).
    pub fn setup(&self) -> &MatchSetup {
        &self.config.setup
    }

    /// The results table: winner (side) or draw, and one row per player
    /// (best mode score first, then kills, fewer deaths, player id).
    pub fn results(&self) -> MatchResults {
        let mut rows: Vec<ResultRow> = self
            .ships
            .values()
            .map(|s| ResultRow {
                player: s.player,
                side: s.team,
                kills: s.kills,
                deaths: s.deaths,
                score: self.mode.player(s.player).map_or(0, |p| p.score),
            })
            .collect();
        rows.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.kills.cmp(&a.kills))
                .then(a.deaths.cmp(&b.deaths))
                .then(a.player.cmp(&b.player))
        });
        let view = self.mode_view();
        let (ended, winner) = match self.mode.phase {
            MatchPhase::Running => (false, None),
            MatchPhase::Ended { winner } => (true, winner),
        };
        MatchResults {
            kind: self.mode.kind,
            teams: self.mode.teams,
            ended,
            winner,
            team_scores: view.team_scores,
            rows,
        }
    }

    /// Whether a player spectates (eliminated, joined late in an
    /// elimination mode, or by choice): no ship in play, no respawn.
    pub fn is_spectator(&self, id: PlayerId) -> bool {
        self.mode.player(id).is_some_and(|p| p.is_out())
    }

    /// Switches a player to spectating (the ship leaves play without a
    /// death) or back to playing (it respawns on the next tick). Leaving
    /// is refused for eliminated players and while the mode keeps late
    /// joiners out (elimination modes after the start).
    pub fn set_spectator(&mut self, id: PlayerId, spectate: bool) -> Result<(), SimError> {
        let cfg = Arc::clone(&self.config);
        if !self.ships.contains_key(&id) {
            return Err(SimError::UnknownPlayer(id));
        }
        let flags = self.mode.player(id).map_or(0, |p| p.flags);
        if spectate {
            if let Some(p) = self.mode.player_mut(id) {
                p.flags |= player_flags::SPECTATOR;
            }
            self.retire(id);
            return Ok(());
        }
        if flags & player_flags::SPECTATOR == 0 {
            return Ok(());
        }
        let late = cfg.mode_module.late_join_spectates(&self.mode_ref(&cfg));
        if flags & player_flags::ELIMINATED != 0 || late {
            return Err(SimError::InvalidRuleSet(
                "the match is in progress: spectating until the next match".into(),
            ));
        }
        if let Some(p) = self.mode.player_mut(id) {
            p.flags &= !player_flags::SPECTATOR;
        }
        if let Some(s) = self.ships.get_mut(&id) {
            s.respawn_tick = self.tick + 1;
        }
        Ok(())
    }

    /// Takes a ship out of play without a death (spectating, eliminated).
    fn retire(&mut self, id: PlayerId) {
        if let Some(s) = self.ships.get_mut(&id) {
            s.alive = false;
            s.hull = 0.0;
            s.vel = Vec3::ZERO;
            s.gadget_active = false;
            s.charge = None;
            s.respawn_tick = u64::MAX;
        }
        self.trails.remove(&id);
    }

    fn mode_ref<'a>(&'a self, cfg: &'a SimConfig) -> ModeRef<'a> {
        ModeRef {
            state: &self.mode,
            rules: &cfg.rules.mode,
            mutators: &cfg.rules.mutators,
            ships: &self.ships,
            now: self.tick,
        }
    }

    /// Adds a scrap pile (level junk placed by the generator, tests).
    pub fn add_scrap_pile(&mut self, pos: Vec3, amount: f32) -> u32 {
        let id = self.alloc_entity_id();
        self.scrap.push(ScrapPile {
            id,
            pos,
            amount,
            spawn: None,
            wreck: false,
        });
        id
    }

    #[cfg(test)]
    pub(crate) fn trail_for_test(&self, id: PlayerId, ticks_ago: u64) -> Option<(Vec3, Vec3, f32)> {
        self.trails.get(&id).and_then(|t| t.get(ticks_ago))
    }

    #[cfg(test)]
    pub(crate) fn remove_last_scrap_for_test(&mut self) {
        self.scrap.pop();
    }

    /// Spawns an entity directly (tests, tools, level setup). Returns its id.
    pub fn spawn_entity(&mut self, spawn: EntitySpawn) -> u32 {
        self.insert_entity(spawn)
    }

    // ----- players ---------------------------------------------------------

    /// Adds a player and spawns their ship. `team = None` picks the smaller team.
    pub fn add_player(
        &mut self,
        team: Option<u8>,
        ship_type: ShipType,
    ) -> Result<PlayerId, SimError> {
        if !self.config.resolved.allowed_ship_types.contains(&ship_type) {
            return Err(SimError::ShipTypeNotAllowed(ship_type));
        }
        let teams = self.mode.teams;
        let team = if teams == 0 {
            // Free-for-all: every ship is its own side (lowest free side id).
            // A departed player's side stays taken while its projectiles,
            // mines or decoys are still around: a newcomer on that side
            // would be immune to them (same side) and they to it.
            if let Some(t) = team {
                return Err(SimError::InvalidTeam(t));
            }
            let departed = |o: PlayerId| !self.ships.contains_key(&o);
            let used: BTreeSet<u8> = self
                .ships
                .values()
                .map(|s| s.team)
                .chain(
                    self.projectiles
                        .iter()
                        .filter(|p| departed(p.owner))
                        .map(|p| p.team),
                )
                .chain(
                    self.entities
                        .iter()
                        .filter(|e| e.owner.is_some_and(departed))
                        .map(|e| e.team),
                )
                .collect();
            (0..=MAX_FFA_SIDE)
                .find(|t| !used.contains(t))
                .ok_or(SimError::TooManyPlayers)?
        } else {
            match team {
                Some(t) if t >= teams => return Err(SimError::InvalidTeam(t)),
                Some(t) => t,
                // The smallest team (the lowest index on a tie).
                None => (0..teams)
                    .min_by_key(|&t| self.ships.values().filter(|s| s.team == t).count())
                    .unwrap_or(0),
            }
        };
        if self.next_player == u16::MAX {
            return Err(SimError::TooManyPlayers);
        }
        let id = PlayerId(self.next_player);
        self.next_player += 1;
        let ship = ShipState::new(
            id,
            team,
            ship_type,
            self.config.tuning(ship_type),
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        self.ships.insert(id, ship);
        let cfg = Arc::clone(&self.config);
        self.mode
            .players
            .insert(id, crate::modes::initial_player(&cfg.rules.mode));
        let module = cfg.mode_module;
        module.on_join(&mut self.mode_mut_ctx(&cfg), id);
        if module.late_join_spectates(&self.mode_ref(&cfg)) {
            if let Some(p) = self.mode.player_mut(id) {
                p.flags |= player_flags::SPECTATOR;
            }
            self.retire(id);
        } else {
            self.respawn(id);
        }
        Ok(id)
    }

    /// Removes a player. Their projectiles stay in flight.
    pub fn remove_player(&mut self, id: PlayerId) -> bool {
        self.trails.remove(&id);
        self.mode.players.remove(&id);
        self.ships.remove(&id).is_some()
    }

    fn mode_mut_ctx<'a>(&'a mut self, cfg: &'a SimConfig) -> ModeMut<'a> {
        ModeMut {
            state: &mut self.mode,
            rules: &cfg.rules.mode,
            mutators: &cfg.rules.mutators,
            ships: &self.ships,
            now: self.tick,
        }
    }

    /// Ship choice: the type `id` flies from its next respawn on (the rule
    /// set must allow it).
    pub fn set_ship_type(&mut self, id: PlayerId, ship_type: ShipType) -> Result<(), SimError> {
        if !self.config.resolved.allowed_ship_types.contains(&ship_type) {
            return Err(SimError::ShipTypeNotAllowed(ship_type));
        }
        let ship = self.ships.get_mut(&id).ok_or(SimError::UnknownPlayer(id))?;
        ship.next_ship_type = (ship.ship_type != ship_type).then_some(ship_type);
        Ok(())
    }

    /// Gives a weapon to a ship as if picked up, enforcing the rule set: banned
    /// weapons and weapons that cannot be pickups are refused.
    pub fn give_weapon(&mut self, id: PlayerId, weapon: &str) -> Result<(), SimError> {
        let cfg = Arc::clone(&self.config);
        let w = cfg.weapons.require(weapon)?;
        let avail = cfg.resolved.availability(w);
        if avail.is_banned() {
            return Err(SimError::WeaponBanned(weapon.to_string()));
        }
        let ship = self.ships.get_mut(&id).ok_or(SimError::UnknownPlayer(id))?;
        let ammo = match cfg.weapons.get(w).cost {
            WeaponCost::Count(n) => n,
            _ => 0,
        };
        if collect_weapon(ship, w, ammo, &cfg) {
            Ok(())
        } else {
            Err(SimError::WeaponNotCollectable(weapon.to_string()))
        }
    }

    /// Puts a gadget into a ship's slot as if picked up (rule set enforced).
    pub fn give_gadget(&mut self, id: PlayerId, gadget: &str) -> Result<(), SimError> {
        let cfg = Arc::clone(&self.config);
        let g = cfg.gadgets.require(gadget)?;
        if cfg.resolved.gadget_availability(g).is_banned() {
            return Err(SimError::GadgetBanned(gadget.to_string()));
        }
        let ship = self.ships.get_mut(&id).ok_or(SimError::UnknownPlayer(id))?;
        ship.gadget = Some(g);
        ship.gadget_active = false;
        Ok(())
    }

    /// Whether `viewer` may see `target` (snapshot relevance): false only for
    /// a gadget that hides it (Cloak). Unknown players are visible.
    pub fn ship_visible_to(&self, viewer: PlayerId, target: PlayerId) -> bool {
        let (Some(v), Some(t)) = (self.ships.get(&viewer), self.ships.get(&target)) else {
            return true;
        };
        if self.director.current.is_some()
            && crate::director::reveals(&self.director.view(), self.config.seed, self.tick, t)
        {
            return true;
        }
        match t.gadget {
            Some(g) if t.gadget_active && t.alive => {
                let def = self.config.gadgets.get(g);
                self.config.gadget_behaviors[g.0 as usize].visible_to(def, t, v, self.tick)
            }
            _ => true,
        }
    }

    /// Picks the next spawn point for the ship's team and resets it with the
    /// loadout, as the ship type chosen for this life.
    fn respawn(&mut self, id: PlayerId) -> Option<Vec3> {
        let cfg = Arc::clone(&self.config);
        let old = self.ships.get(&id)?;
        let team = old.team;
        let ship_type = old.next_ship_type.unwrap_or(old.ship_type);
        let (kills, deaths, horizon_assist) = (old.kills, old.deaths, old.horizon_assist);
        let (pos, orientation) = self.pick_spawn(&cfg, id, team);
        let tuning = cfg.tuning(ship_type);
        let mut ship = ShipState::new(id, team, ship_type, tuning, pos, orientation);
        ship.kills = kills;
        ship.deaths = deaths;
        ship.horizon_assist = horizon_assist;
        for &w in &cfg.resolved.loadout_primaries {
            ship.add_weapon(WeaponState {
                weapon: w,
                ammo: 0,
                level: 1,
                shots: 0,
            });
            ship.primary.get_or_insert(w);
        }
        let slots = tuning.secondary_slots as usize;
        for &(w, count) in cfg.resolved.loadout_secondaries.iter().take(slots) {
            ship.add_weapon(WeaponState {
                weapon: w,
                ammo: count.min(cfg.resolved.max_ammo(w)),
                level: 1,
                shots: 0,
            });
            ship.secondary.get_or_insert(w);
        }
        ship.gadget = cfg.resolved.loadout_gadget;
        let mut m = ModeMut {
            state: &mut self.mode,
            rules: &cfg.rules.mode,
            mutators: &cfg.rules.mutators,
            ships: &self.ships,
            now: self.tick,
        };
        cfg.mode_module.on_spawn(&mut m, &mut ship);
        let pos = ship.pos;
        self.ships.insert(id, ship);
        self.trails.remove(&id);
        Some(pos)
    }

    /// The spawn point for a ship of side `team`: team modes cycle through
    /// the team's spawn points (all of them if it has none); free-for-all
    /// takes the spawn point farthest from the nearest other living ship,
    /// scanning from a rotating start (ties keep the first).
    fn pick_spawn(&mut self, cfg: &SimConfig, id: PlayerId, team: u8) -> (Vec3, Quat) {
        let spawns = &cfg.setup.spawns;
        if spawns.is_empty() {
            return (Vec3::ZERO, Quat::IDENTITY);
        }
        if self.mode.teams == 0 {
            let others: Vec<Vec3> = self
                .ships
                .values()
                .filter(|s| s.alive && s.player != id)
                .map(|s| s.pos)
                .collect();
            let cursor = &mut self.spawn_cursor[0];
            let start = *cursor as usize % spawns.len();
            *cursor = cursor.wrapping_add(1);
            let mut best: Option<(f32, &SpawnPoint)> = None;
            for k in 0..spawns.len() {
                let sp = &spawns[(start + k) % spawns.len()];
                let d = others
                    .iter()
                    .map(|p| p.distance_squared(sp.pos))
                    .fold(f32::INFINITY, f32::min);
                if best.is_none_or(|(bd, _)| d > bd) {
                    best = Some((d, sp));
                }
            }
            let sp = best.map_or(&spawns[start], |(_, sp)| sp);
            return (sp.pos, sp.orientation);
        }
        let mut candidates: Vec<&SpawnPoint> = spawns.iter().filter(|s| s.team == team).collect();
        if candidates.is_empty() {
            candidates = spawns.iter().collect();
        }
        let cursor = &mut self.spawn_cursor[(team % MAX_TEAMS) as usize];
        let sp = candidates[*cursor as usize % candidates.len()];
        *cursor = cursor.wrapping_add(1);
        (sp.pos, sp.orientation)
    }

    // ----- simulation ------------------------------------------------------

    /// Advances the match by one tick. `inputs` holds at most one command per
    /// player (later duplicates win); ships without a command get an idle one.
    /// Deterministic: the same state, terrain and inputs give the same result.
    pub fn step<T: SimTerrain + ?Sized>(
        &mut self,
        terrain: &mut T,
        inputs: &[(PlayerId, InputCmd)],
    ) -> Vec<Event> {
        let mut events = Vec::new();
        if !self.mode.is_running() {
            return events;
        }
        self.tick += 1;
        let now = self.tick;
        let cfg = Arc::clone(&self.config);
        self.effects = if self.director.current.is_some() {
            crate::director::event_effects(&self.director.view(), cfg.seed, now)
        } else {
            EventEffects::NONE
        };

        let mut cmds: BTreeMap<PlayerId, InputCmd> = BTreeMap::new();
        for (p, c) in inputs {
            cmds.insert(*p, c.sanitized());
        }
        self.inputs = cmds;
        let ids: Vec<PlayerId> = self.ships.keys().copied().collect();

        // 0. The mode starts; terrain edits deferred to this tick.
        if now == 1 {
            self.run_mode_hook(&cfg, terrain, &mut events, |module, m| module.on_start(m));
        }
        self.apply_deferred_edits(&cfg, terrain, now, &mut events);
        // The mode's terrain shell for this tick (The Squeeze's walls).
        terrain.set_shell(cfg.mode_module.terrain_shell(&self.mode, now));

        // 1. Respawns.
        for &id in &ids {
            let due = self
                .ships
                .get(&id)
                .is_some_and(|s| !s.alive && now >= s.respawn_tick)
                && !self.mode.player(id).is_some_and(|p| p.is_out())
                && cfg.mode_module.allow_respawn(&self.mode_ref(&cfg), id);
            if due {
                if let Some(pos) = self.respawn(id) {
                    events.push(Event::Respawned { player: id, pos });
                }
            }
        }

        // Ships steering a projectile (Eyeball) get no thrust or look.
        let controlled: BTreeSet<PlayerId> = self
            .projectiles
            .iter()
            .filter(|p| p.flags & proj_flags::CONTROLS_OWNER != 0)
            .map(|p| p.owner)
            .collect();

        // 2. Ships: selection, gadget, movement, scavenging, firing.
        for &id in &ids {
            let cmd = self
                .inputs
                .get(&id)
                .copied()
                .unwrap_or_else(|| InputCmd::idle(now));
            let Some(ship) = self.ships.get_mut(&id) else {
                continue;
            };
            if !ship.alive {
                ship.prev_buttons = cmd.buttons;
                continue;
            }
            select_weapon(ship, cmd.weapon_select, &cfg.weapons);
            let tuning = cfg.tuning(ship.ship_type);
            // Mode limits (Heist carrier: slower, no boost).
            let flags = self.mode.player(id).map_or(0, |p| p.flags);
            let limits = cfg.mode_module.move_limits(&cfg.rules.mode, flags);
            let cmd = if limits.is_none() {
                cmd
            } else {
                limits.apply(tuning, &cmd).1
            };
            let move_cmd = if controlled.contains(&id) {
                InputCmd {
                    thrust: Vec3::ZERO,
                    look_pitch: 0.0,
                    look_yaw: 0.0,
                    roll: 0.0,
                    ..cmd
                }
            } else {
                cmd
            };
            // Comedy event forces, impulses and plane locks.
            let fx = if self.director.current.is_some() {
                crate::director::move_effects(&self.director.view(), cfg.seed, now, ship)
            } else {
                MoveEffects::NONE
            };
            let pos_before = ship.pos;
            if !fx.is_none() {
                crate::director::apply_move_effects_before(ship, &fx);
            }
            // Wells and the level's up (M4); a level without wells and with
            // world +Y up runs the M1-M3 flight model unchanged.
            let plain = self.wells.is_empty() && self.up == Vec3::Y;
            if limits.is_none() {
                if plain {
                    simulate_movement(ship, &move_cmd, tuning, &*terrain);
                } else {
                    let env = self.wells.move_env(ship.pos, now, tuning, self.up);
                    simulate_movement_env(ship, &move_cmd, tuning, &*terrain, &env);
                }
            } else {
                let (limited, _) = limits.apply(tuning, &move_cmd);
                if plain {
                    simulate_movement(ship, &move_cmd, &limited, &*terrain);
                } else {
                    let env = self.wells.move_env(ship.pos, now, &limited, self.up);
                    simulate_movement_env(ship, &move_cmd, &limited, &*terrain, &env);
                }
            }
            if !fx.is_none() {
                crate::director::apply_move_effects_after(ship, &fx, pos_before);
            }
            // Charged surfaces (Leech Arc) drain ships scraping them.
            if ship.in_contact
                && cfg.charged_drain_per_sec > 0.0
                && terrain
                    .surface_states_near(ship.pos, tuning.radius + 0.2, now)
                    .contains(SurfaceStates::CHARGED)
            {
                ship.energy = (ship.energy - cfg.charged_drain_per_sec * DT).max(0.0);
            }
            if cmd.buttons.contains(Buttons::SCAVENGE) {
                if self.effects.scavenge_scale == 1.0 {
                    scavenge(ship, &mut self.scrap, &mut self.entities, tuning);
                } else {
                    let mut t = tuning.clone();
                    t.hull_per_scrap *= self.effects.scavenge_scale;
                    scavenge(ship, &mut self.scrap, &mut self.entities, &t);
                }
            }
            let gadget_pressed = cmd.buttons.contains(Buttons::GADGET)
                && !ship.prev_buttons.contains(Buttons::GADGET);
            let mode_pressed = cmd.buttons.contains(Buttons::WEAPON_MODE)
                && !ship.prev_buttons.contains(Buttons::WEAPON_MODE);
            if cmd.buttons.contains(Buttons::SCAVENGE) && tuning.can_pass_scrap {
                self.pass_scrap(&cfg, id);
            }
            self.gadget_step(&cfg, terrain, id, gadget_pressed, &mut events);
            if mode_pressed {
                self.weapon_mode(&cfg, terrain, id, &mut events);
            }
            let overdrive = cmd.buttons.contains(Buttons::OVERDRIVE);
            for (slot, button) in [
                (WeaponSlot::Primary, Buttons::FIRE_PRIMARY),
                (WeaponSlot::Secondary, Buttons::FIRE_SECONDARY),
            ] {
                // A pilot guiding a missile cannot launch another secondary
                // (one Eyeball at a time).
                let guiding = slot == WeaponSlot::Secondary && controlled.contains(&id);
                let pressed = cmd.buttons.contains(button) && !guiding;
                self.trigger(&cfg, terrain, id, slot, pressed, overdrive, &mut events);
            }
            if let Some(ship) = self.ships.get_mut(&id) {
                ship.prev_buttons = cmd.buttons;
            }
        }

        // 3. Projectiles.
        self.step_projectiles(&cfg, terrain, now, &mut events);

        // 4. Entities.
        self.step_entities(&cfg, terrain, now, &mut events);

        // 5. Scrap piles and pickups.
        self.cleanup_scrap(&cfg, now);
        self.collect_pickups(&cfg, now, &mut events);
        self.update_pickup_spawns(&mut events);

        // 6. Trails.
        for s in self.ships.values() {
            if s.alive {
                self.trails
                    .entry(s.player)
                    .or_default()
                    .push((s.pos, s.vel, s.hull));
            }
        }

        // 7. World physics (M4): radiation, pigment, wells, Instability.
        self.world_physics(&cfg, terrain, now, &mut events);

        // 8. Mode: its tick, newly eliminated players leave play, then the
        //    end check.
        self.run_mode_hook(&cfg, terrain, &mut events, |module, m| module.on_tick(m));
        let out: Vec<PlayerId> = self
            .ships
            .values()
            .filter(|s| {
                s.respawn_tick != u64::MAX && self.mode.player(s.player).is_some_and(|p| p.is_out())
            })
            .map(|s| s.player)
            .collect();
        for id in out {
            self.retire(id);
            let eliminated = self
                .mode
                .player(id)
                .is_some_and(|p| p.flags & player_flags::ELIMINATED != 0);
            if eliminated {
                events.push(Event::Eliminated { player: id });
            }
        }
        let mut ended = false;
        if let Some(winner) = cfg.mode_module.end_check(&self.mode_ref(&cfg)) {
            self.mode.phase = MatchPhase::Ended { winner };
            events.push(Event::MatchEnded {
                winner,
                team_cells: self.mode.coverage.team_cells,
            });
            ended = true;
        }

        // 9. The comedy director.
        if self.director.is_on(cfg.absurdity) {
            self.run_director(&cfg, terrain, ended, &mut events);
        }
        events
    }

    /// Runs the comedy director's step with a full hook context and applies
    /// the commands its events queued.
    fn run_director<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        ended: bool,
        events: &mut Vec<Event>,
    ) {
        let mut director = std::mem::take(&mut self.director);
        let mut out = Vec::new();
        let ((), cmds) = self.collect_hook(cfg, &*terrain, |ctx| {
            let inp = DirectorInput {
                def: DirectorDef::builtin(),
                table: EventTable::builtin(),
                absurdity: cfg.absurdity,
                banned: cfg.banned_events,
                mode: &self.mode,
                mode_module: cfg.mode_module,
                mode_ref: self.mode_ref(cfg),
                mutators: &cfg.mutator_modules,
                setup: &cfg.setup,
            };
            director.step(ctx, &inp, ended, &mut out);
        });
        self.director = director;
        events.extend(out.into_iter().map(Event::Director));
        self.apply_commands(cfg, terrain, cmds, events);
    }

    /// Runs a mode hook with a full [`ModeCtx`] and applies its commands.
    fn run_mode_hook<T, R>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        events: &mut Vec<Event>,
        f: impl FnOnce(&dyn ModeModule, &mut ModeCtx) -> R,
    ) -> R
    where
        T: SimTerrain + ?Sized,
    {
        let placeholder = ModeState::new(self.mode.kind, self.mode.teams, 0);
        let mut state = std::mem::replace(&mut self.mode, placeholder);
        let coverage = || terrain.paint_coverage();
        let (r, cmds) = self.collect_hook(cfg, &*terrain, |ctx| {
            let mut m = ModeCtx {
                ctx,
                state: &mut state,
                rules: &cfg.rules.mode,
                mutators: &cfg.rules.mutators,
                setup: &cfg.setup,
                coverage: &coverage,
            };
            f(cfg.mode_module, &mut m)
        });
        self.mode = state;
        self.apply_commands(cfg, terrain, cmds, events);
        r
    }

    /// Applies the deferred terrain edits due at `now` (in queue order).
    fn apply_deferred_edits<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        if self.deferred_edits.is_empty() {
            return;
        }
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.deferred_edits)
            .into_iter()
            .partition(|(t, _)| *t <= now);
        self.deferred_edits = later;
        let cmds = due.into_iter().map(|(_, e)| Command::Edit(e)).collect();
        self.apply_commands(cfg, terrain, cmds, events);
    }

    /// Runs `f` as a hook of the current tick and applies its commands
    /// (tests of `Ctx` helpers).
    #[cfg(test)]
    pub(crate) fn hook_for_test<T: SimTerrain + ?Sized, R>(
        &mut self,
        terrain: &mut T,
        f: impl FnOnce(&mut Ctx) -> R,
    ) -> (R, Vec<Event>) {
        let cfg = Arc::clone(&self.config);
        let mut events = Vec::new();
        let r = self.run_hook(&cfg, terrain, &mut events, f);
        (r, events)
    }

    /// Runs a hook with a fresh context and applies the commands it queued.
    fn run_hook<T, R>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        events: &mut Vec<Event>,
        f: impl FnOnce(&mut Ctx) -> R,
    ) -> R
    where
        T: SimTerrain + ?Sized,
    {
        let (r, cmds) = self.collect_hook(cfg, &*terrain, f);
        self.apply_commands(cfg, terrain, cmds, events);
        r
    }

    /// Runs a hook with a fresh context; returns its result and commands
    /// (not applied yet).
    fn collect_hook<T, R>(
        &self,
        cfg: &SimConfig,
        terrain: &T,
        f: impl FnOnce(&mut Ctx) -> R,
    ) -> (R, Vec<Command>)
    where
        T: SimTerrain + ?Sized,
    {
        let tref = TerrainRef(
            terrain,
            Some(ModeMaterial {
                module: cfg.mode_module,
                state: &self.mode,
                rules: &cfg.rules.mode,
                now: self.tick,
                instability: &self.instability,
            }),
        );
        let view = View {
            now: self.tick,
            seed: cfg.seed,
            weapons: &cfg.weapons,
            gadgets: &cfg.gadgets,
            ship_types: &cfg.ships,
            rules: &cfg.resolved,
            visuals: &cfg.visuals,
            instability: cfg.setup.instability,
            effects: self.effects,
            ships: &self.ships,
            projectiles: &self.projectiles,
            projectiles_stepping: &self.stepping,
            projectiles_stepped: &self.stepped,
            entities: &self.entities,
            pickups: &self.pickups,
            scrap_piles: &self.scrap,
            terrain: &tref,
            layers: &tref,
            inputs: &self.inputs,
            trails: &self.trails,
        };
        let mut ctx = Ctx::new(view);
        let r = f(&mut ctx);
        (r, ctx.into_commands())
    }

    /// Gadget key, drain and per-tick hook for ship `id`.
    fn gadget_step<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        id: PlayerId,
        pressed: bool,
        events: &mut Vec<Event>,
    ) {
        let now = self.tick;
        let Some(ship) = self.ships.get_mut(&id) else {
            return;
        };
        let Some(g) = ship.gadget else {
            return;
        };
        if cfg.resolved.gadget_availability(g).is_banned() {
            ship.gadget_active = false;
            return;
        }
        let def = cfg.gadgets.get(g);
        let beh = cfg.gadget_behaviors[g.0 as usize];
        enum Action {
            None,
            On,
            Off,
            /// Switched off because the energy ran out (no cooldown).
            Empty,
            Use,
        }
        let mut action = Action::None;
        if pressed {
            match def.activation {
                GadgetActivation::Toggle if ship.gadget_active => {
                    ship.gadget_active = false;
                    ship.gadget_ready_tick = now + def.cooldown_ticks();
                    action = Action::Off;
                }
                _ if now >= ship.gadget_ready_tick && ship.energy >= def.energy_cost => {
                    ship.energy -= def.energy_cost;
                    match def.activation {
                        GadgetActivation::Toggle => {
                            ship.gadget_active = true;
                            action = Action::On;
                        }
                        GadgetActivation::Instant => {
                            ship.gadget_ready_tick = now + def.cooldown_ticks();
                            action = Action::Use;
                        }
                    }
                }
                _ => {}
            }
        }
        if ship.gadget_active {
            let moving = ship.vel.length() > def.param("moving_speed", 1.0);
            let mult = if moving { def.moving_drain_mult } else { 1.0 };
            ship.energy -= def.drain_per_sec * mult * DT;
            if ship.energy <= 0.0 {
                ship.energy = 0.0;
                ship.gadget_active = false;
                action = Action::Empty;
            }
        }
        let active = ship.gadget_active;
        let pos = ship.pos;
        let cue = |value: u16| {
            Event::Cue(Cue {
                kind: CueKind::Gadget,
                source: Some(id),
                target: None,
                pos,
                value,
            })
        };
        match action {
            Action::On => events.push(cue(g.0 as u16)),
            Action::Off => events.push(cue(gadget_cue::OFF)),
            Action::Empty => events.push(cue(gadget_cue::EMPTY)),
            Action::Use => events.push(cue(gadget_cue::USED | g.0 as u16)),
            Action::None => {}
        }
        let call = |w: &mut World, which: u8, terrain: &mut T, events: &mut Vec<Event>| {
            w.run_hook(cfg, terrain, events, |ctx| {
                let Some(ship) = ctx.view.ships.get(&id) else {
                    return;
                };
                let u = GadgetUse {
                    gadget: g,
                    def,
                    ship,
                };
                match which {
                    0 => beh.on_activate(ctx, &u),
                    1 => beh.on_deactivate(ctx, &u),
                    _ => beh.tick(ctx, &u),
                }
            });
        };
        match action {
            Action::On | Action::Use => call(self, 0, terrain, events),
            Action::Off | Action::Empty => call(self, 1, terrain, events),
            Action::None => {}
        }
        if active {
            call(self, 2, terrain, events);
        }
    }

    /// The weapon mode key went down: the selected primary's `on_mode` hook.
    fn weapon_mode<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        id: PlayerId,
        events: &mut Vec<Event>,
    ) {
        let Some(w) = self
            .ships
            .get(&id)
            .filter(|s| s.alive)
            .and_then(|s| s.primary)
        else {
            return;
        };
        let beh = cfg.behavior(w);
        self.run_hook(cfg, terrain, events, |ctx| {
            if let Some(s) = ctx.view.ships.get(&id) {
                beh.on_mode(ctx, s, w);
            }
        });
    }

    /// Trigger handling for one slot of ship `id` (see `crate::weapons`).
    #[allow(clippy::too_many_arguments)]
    fn trigger<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        id: PlayerId,
        slot: WeaponSlot,
        pressed: bool,
        overdrive: bool,
        events: &mut Vec<Event>,
    ) {
        let now = self.tick;
        let Some(ship) = self.ships.get_mut(&id) else {
            return;
        };
        if !ship.alive {
            return;
        }
        let selected = match slot {
            WeaponSlot::Primary => ship.primary,
            WeaponSlot::Secondary => ship.secondary,
        };
        // A charge on this slot whose weapon is no longer selected is dropped.
        if ship
            .charge
            .is_some_and(|c| c.slot == slot && Some(c.weapon) != selected)
        {
            ship.charge = None;
        }
        let Some(w) = selected else {
            return;
        };
        if now < ship.weapons_offline_until {
            if ship.charge.is_some_and(|c| c.slot == slot) {
                ship.charge = None;
            }
            return;
        }
        let def = cfg.weapons.get(w);
        let beh = cfg.behavior(w);
        let Some(charge_def) = def.charge else {
            if pressed {
                self.fire(cfg, terrain, id, slot, overdrive, 0.0, 0, events);
            }
            return;
        };
        let charging = ship.charge.filter(|c| c.slot == slot);
        match (charging, pressed) {
            (None, true) => {
                if ship.charge.is_some() || !can_fire(ship, slot, w, now, overdrive, cfg) {
                    return;
                }
                ship.charge = Some(ChargeState {
                    slot,
                    weapon: w,
                    start_tick: now,
                });
                let full = WeaponDef::charge_ticks(charge_def.max_secs);
                events.push(Event::Cue(Cue {
                    kind: CueKind::ChargeStart,
                    source: Some(id),
                    target: None,
                    pos: ship.pos,
                    value: full.min(u16::MAX as u64) as u16,
                }));
                self.run_hook(cfg, terrain, events, |ctx| {
                    if let Some(s) = ctx.view.ships.get(&id) {
                        beh.charge_start(ctx, s, w);
                    }
                });
            }
            (Some(c), true) => {
                let held = now - c.start_tick;
                let action = self.run_hook(cfg, terrain, events, |ctx| {
                    ctx.view
                        .ships
                        .get(&id)
                        .map_or(ChargeAction::Cancel, |s| beh.charge_hold(ctx, s, w, held))
                });
                match action {
                    ChargeAction::Hold => {}
                    ChargeAction::Cancel => {
                        if let Some(s) = self.ships.get_mut(&id) {
                            s.charge = None;
                        }
                    }
                    ChargeAction::Release => {
                        self.release(cfg, terrain, id, slot, overdrive, held, events)
                    }
                }
            }
            (Some(c), false) => {
                let held = now - c.start_tick;
                self.release(cfg, terrain, id, slot, overdrive, held, events);
            }
            (None, false) => {}
        }
    }

    /// Releases a charge: fires with the charge fraction if held long enough.
    #[allow(clippy::too_many_arguments)]
    fn release<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        id: PlayerId,
        slot: WeaponSlot,
        overdrive: bool,
        held: u64,
        events: &mut Vec<Event>,
    ) {
        let Some(ship) = self.ships.get_mut(&id) else {
            return;
        };
        let Some(c) = ship.charge.take() else {
            return;
        };
        let Some(cd) = cfg.weapons.get(c.weapon).charge else {
            return;
        };
        if held < ls_math::Tick::ticks_for_secs(cd.min_secs) {
            return;
        }
        let full = WeaponDef::charge_ticks(cd.max_secs);
        let charge = (held as f32 / full as f32).min(1.0);
        self.fire(cfg, terrain, id, slot, overdrive, charge, held, events);
    }

    /// Pays for and fires the selected weapon of `slot` (cooldown and cost
    /// permitting), calling the weapon's `fire` hook.
    #[allow(clippy::too_many_arguments)]
    fn fire<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        id: PlayerId,
        slot: WeaponSlot,
        overdrive: bool,
        charge: f32,
        held: u64,
        events: &mut Vec<Event>,
    ) {
        let now = self.tick;
        let Some(ship) = self.ships.get_mut(&id) else {
            return;
        };
        let Some(stats) = try_fire(ship, slot, now, overdrive, charge, cfg) else {
            return;
        };
        ship.last_fired_tick = Some(now);
        let beh = cfg.behavior(stats.weapon);
        let def = cfg.weapons.get(stats.weapon);
        self.run_hook(cfg, terrain, events, |ctx| {
            let Some(shooter) = ctx.view.ships.get(&id) else {
                return;
            };
            let shot = Shot {
                shooter,
                weapon: stats.weapon,
                def,
                level: stats.level,
                shots: stats.shots,
                damage: stats.damage,
                splash_damage: stats.splash_damage,
                knockback: stats.knockback,
                charge,
                held_ticks: held,
            };
            beh.fire(ctx, &shot);
        });
    }

    fn step_projectiles<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        // The lists live in the world while stepping so commands
        // (`ImpulseProjectile`, `RemoveProjectile`) and hooks
        // (`View::all_projectiles`) reach every projectile.
        let mut list = std::mem::take(&mut self.projectiles);
        list.reverse();
        self.stepped = Vec::with_capacity(list.len());
        self.stepping = list;
        let bounds = terrain.bounds();
        while let Some(mut p) = self.stepping.pop() {
            let beh = cfg.behavior(p.weapon);
            if now >= p.fuse_tick {
                let fuse = p.fuse_tick;
                self.run_hook(cfg, terrain, events, |ctx| beh.on_fuse(ctx, &mut p));
                if p.fuse_tick <= fuse || p.is_removed() {
                    continue;
                }
            }
            if now >= p.expires_tick {
                self.run_hook(cfg, terrain, events, |ctx| beh.on_expire(ctx, &mut p));
                continue;
            }
            self.run_hook(cfg, terrain, events, |ctx| beh.update(ctx, &mut p));
            if p.is_removed() {
                continue;
            }
            // Wells bend flagged projectiles (ordnance, mines; M4).
            if !p.is_stuck() && !self.wells.is_empty() && cfg.weapons.get(p.weapon).gravity {
                p.vel += self.wells.body_accel(p.pos, now) * DT;
            }
            if !p.is_stuck() && !self.move_projectile(cfg, terrain, &mut p, events) {
                continue;
            }
            if bounds.contains(p.pos) {
                self.stepped.push(p);
            }
        }
        // Projectiles spawned by hooks during this stage start moving next tick.
        let mut keep = std::mem::take(&mut self.stepped);
        keep.append(&mut self.projectiles);
        self.projectiles = keep;
    }

    /// Centre and radius (without the projectile's) of the ship or entity a
    /// projectile passed through (see [`Projectile::passed`]).
    fn passed_sphere(&self, cfg: &SimConfig, passed: (u8, u32)) -> Option<(Vec3, f32)> {
        match passed {
            (0, id) => {
                let s = self.ships.get(&PlayerId(id as u16))?;
                s.alive.then(|| (s.pos, cfg.tuning(s.ship_type).radius))
            }
            (_, id) => self
                .entities
                .iter()
                .find(|e| e.id == id)
                .map(|e| (e.pos, e.radius)),
        }
    }

    /// Swept move of one projectile for this tick with hit hooks. Returns
    /// false if the projectile ended.
    fn move_projectile<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        p: &mut Projectile,
        events: &mut Vec<Event>,
    ) -> bool {
        let beh = cfg.behavior(p.weapon);
        let speed = p.vel.length();
        if speed <= 0.0 {
            return true;
        }
        let mut dir = p.vel / speed;
        let mut remaining = speed * DT;
        // A ship or entity passed through on an earlier tick is skipped until
        // the projectile has left it (else a slow projectile would hit it
        // again every tick it spends inside).
        if let Some(passed) = p.passed {
            let inside = self.passed_sphere(cfg, passed).is_some_and(|(c, r)| {
                (p.pos - c).length_squared() <= (r + p.radius) * (r + p.radius)
            });
            if !inside {
                p.passed = None;
            }
        }
        let mut skip_ships: Vec<PlayerId> = Vec::new();
        let mut skip_entities: Vec<u32> = Vec::new();
        if let Some((kind, id)) = p.passed {
            if kind == 0 {
                skip_ships.push(PlayerId(id as u16));
            } else {
                skip_entities.push(id);
            }
        }
        for _ in 0..MAX_HITS_PER_TICK {
            enum Hit {
                Ship(f32, PlayerId, Vec3),
                Entity(f32, u32, EntityKind, Vec3),
                Terrain(RayHit),
            }
            let terrain_hit = terrain.raycast(p.pos, dir, remaining);
            let mut best: Option<Hit> = None;
            let mut best_t = f32::INFINITY;
            if p.flags & proj_flags::NO_SHIP_HITS == 0 {
                let hits_owner = p.flags & proj_flags::HITS_OWNER != 0;
                for (id, s) in &self.ships {
                    if !s.alive || (*id == p.owner && !hits_owner) || skip_ships.contains(id) {
                        continue;
                    }
                    let r =
                        cfg.tuning(s.ship_type).radius * self.effects.hit_radius_scale + p.radius;
                    if let Some(t) = segment_sphere(p.pos, dir, remaining, s.pos, r) {
                        if t < best_t {
                            best_t = t;
                            best = Some(Hit::Ship(t, *id, s.pos));
                        }
                    }
                }
            }
            for e in &self.entities {
                if !e.blocks_projectiles() || skip_entities.contains(&e.id) {
                    continue;
                }
                if e.kind == EntityKind::Decoy && e.owner == Some(p.owner) {
                    continue;
                }
                let r = e.radius + p.radius;
                if let Some(t) = segment_sphere(p.pos, dir, remaining, e.pos, r) {
                    if t < best_t {
                        best_t = t;
                        best = Some(Hit::Entity(t, e.id, e.kind, e.pos));
                    }
                }
            }
            // Ships win ties with terrain (M1 rule).
            let hit = match (best, terrain_hit) {
                (Some(h), None) => h,
                (Some(h), Some(th)) if best_t <= th.distance => h,
                (_, Some(th)) => Hit::Terrain(th),
                (None, None) => {
                    p.pos += dir * remaining;
                    return true;
                }
            };
            let (t, point, normal, resp) = match hit {
                Hit::Ship(t, target, center) => {
                    let point = p.pos + dir * t;
                    let normal = (point - center).try_normalize().unwrap_or(-dir);
                    let h = ShipHit {
                        target,
                        point,
                        normal,
                    };
                    let resp =
                        self.run_hook(cfg, terrain, events, |ctx| beh.on_ship_hit(ctx, p, &h));
                    if resp == HitResponse::Continue {
                        skip_ships.push(target);
                        p.passed = Some((0, target.0 as u32));
                    }
                    (t, point, normal, resp)
                }
                Hit::Entity(t, id, kind, center) => {
                    let point = p.pos + dir * t;
                    let normal = (point - center).try_normalize().unwrap_or(-dir);
                    let h = EntityHit {
                        id,
                        kind,
                        point,
                        normal,
                    };
                    let resp =
                        self.run_hook(cfg, terrain, events, |ctx| beh.on_entity_hit(ctx, p, &h));
                    if resp == HitResponse::Continue {
                        skip_entities.push(id);
                        p.passed = Some((1, id));
                    }
                    (t, point, normal, resp)
                }
                Hit::Terrain(h) => {
                    let resp =
                        self.run_hook(cfg, terrain, events, |ctx| beh.on_terrain_hit(ctx, p, &h));
                    let resp = if resp == HitResponse::Continue {
                        HitResponse::Stop
                    } else {
                        resp
                    };
                    (h.distance, h.point, h.normal, resp)
                }
            };
            if p.is_removed() {
                return false;
            }
            match resp {
                HitResponse::Stop => return false,
                HitResponse::Continue => {
                    p.pos = point;
                    remaining -= t;
                }
                HitResponse::Pierce { exit } => {
                    remaining -= t + point.distance(exit);
                    p.pos = exit;
                    if remaining <= 0.0 {
                        return true;
                    }
                }
                HitResponse::Bounce => {
                    p.vel = p.vel.reflect(normal);
                    p.pos = point + normal * (p.radius + 0.01);
                    p.bounces = p.bounces.saturating_add(1);
                    return true;
                }
                HitResponse::Stick => {
                    p.pos = point + normal * p.radius;
                    p.vel = Vec3::ZERO;
                    p.flags |= proj_flags::STUCK;
                    return true;
                }
            }
            let speed = p.vel.length();
            if speed <= 0.0 || remaining <= 0.0 {
                return true;
            }
            dir = p.vel / speed;
        }
        true
    }

    fn step_entities<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        if self.entities.is_empty() {
            return;
        }
        let mut expired: Vec<u32> = Vec::new();
        let wake = GravitySettings::builtin().wake_accel;
        for e in &mut self.entities {
            if now >= e.expires_tick {
                expired.push(e.id);
                continue;
            }
            if e.kinematic {
                // Driven by its module: no terrain collision.
                e.pos += e.vel * DT;
                continue;
            }
            match e.kind {
                EntityKind::Debris => {
                    // Wells pull debris (M4).
                    let pull = if self.wells.is_empty() {
                        Vec3::ZERO
                    } else {
                        self.wells.body_accel(e.pos, now)
                    };
                    if pull.length_squared() > wake * wake {
                        step_debris_pulled(e, &*terrain, pull);
                    } else {
                        step_debris(e, &*terrain);
                    }
                }
                EntityKind::Decoy => {
                    move_sphere(&*terrain, &mut e.pos, &mut e.vel, e.radius, 0.3);
                }
                _ => e.pos += e.vel * DT,
            }
        }
        // Ships push debris out of their way.
        for s in self.ships.values() {
            if !s.alive {
                continue;
            }
            let sr = cfg.tuning(s.ship_type).radius;
            for e in &mut self.entities {
                if e.kind != EntityKind::Debris {
                    continue;
                }
                let d = e.pos - s.pos;
                let min = sr + e.radius;
                let dist2 = d.length_squared();
                if dist2 >= min * min {
                    continue;
                }
                let n = d.try_normalize().unwrap_or(Vec3::Y);
                let dist = dist2.max(0.0).sqrt();
                e.pos += n * (min - dist);
                let closing = (s.vel - e.vel).dot(n);
                if closing > 0.0 {
                    e.vel += n * (closing * DEBRIS_PUSH);
                }
                e.resting = false;
            }
        }
        // Module hooks for entities spawned by a weapon or gadget.
        let hooked: Vec<(u32, Source, bool)> = self
            .entities
            .iter()
            .filter(|e| e.source != Source::None)
            .map(|e| (e.id, e.source, expired.contains(&e.id)))
            .collect();
        for (id, source, is_expired) in hooked {
            let Some(idx) = self.entities.iter().position(|e| e.id == id) else {
                continue;
            };
            let mut e = self.entities[idx].clone();
            let cmds = match source {
                Source::Weapon(w) if (w.0 as usize) < cfg.behaviors.len() => {
                    let beh = cfg.behavior(w);
                    self.collect_hook(cfg, &*terrain, |ctx| {
                        if is_expired {
                            beh.entity_expired(ctx, &mut e)
                        } else {
                            beh.entity_tick(ctx, &mut e)
                        }
                    })
                    .1
                }
                Source::Gadget(g) if (g.0 as usize) < cfg.gadget_behaviors.len() => {
                    let beh = cfg.gadget_behaviors[g.0 as usize];
                    let def = cfg.gadgets.get(g);
                    self.collect_hook(cfg, &*terrain, |ctx| {
                        if is_expired {
                            beh.entity_expired(ctx, def, &mut e)
                        } else {
                            beh.entity_tick(ctx, def, &mut e)
                        }
                    })
                    .1
                }
                _ => continue,
            };
            // The hook's own changes first (its id and source stay), then its
            // commands. An expiry hook may keep the entity by moving
            // `expires_tick` later; `hp <= 0` removes it.
            e.id = id;
            e.source = source;
            if is_expired && e.expires_tick > now {
                expired.retain(|x| *x != id);
            }
            if e.hp <= 0.0 {
                self.entities.remove(idx);
            } else {
                self.entities[idx] = e;
            }
            self.apply_commands(cfg, terrain, cmds, events);
        }
        self.entities.retain(|e| !expired.contains(&e.id));
    }

    // ----- commands ----------------------------------------------------------

    /// Applies commands queued by a hook, in order.
    fn apply_commands<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        cmds: Vec<Command>,
        events: &mut Vec<Event>,
    ) {
        let now = self.tick;
        for c in cmds {
            match c {
                Command::Damage {
                    target,
                    amount,
                    knockback,
                    source,
                    direct,
                } => self.apply_damage(cfg, now, target, amount, knockback, source, direct, events),
                Command::Splash {
                    center,
                    radius,
                    damage,
                    knockback,
                    source,
                    hits_owner,
                    exclude,
                    dir,
                } => {
                    if radius.is_nan() || radius <= 0.0 {
                        continue;
                    }
                    let victims: Vec<(PlayerId, Vec3)> = self
                        .ships
                        .values()
                        .filter(|s| s.alive && Some(s.player) != exclude)
                        .filter(|s| hits_owner || Some(s.player) != source.attacker)
                        .map(|s| (s.player, s.pos))
                        .collect();
                    for (id, pos) in victims {
                        let d = pos.distance(center);
                        if d >= radius {
                            continue;
                        }
                        let falloff = 1.0 - d / radius;
                        let push = (pos - center).try_normalize().unwrap_or(dir);
                        self.apply_damage(
                            cfg,
                            now,
                            id,
                            damage * falloff,
                            push * (knockback * falloff),
                            source,
                            false,
                            events,
                        );
                    }
                    self.push_debris(center, radius, knockback);
                }
                Command::Shockwave {
                    center,
                    radius,
                    strength,
                } => {
                    if radius.is_nan() || radius <= 0.0 {
                        continue;
                    }
                    for s in self.ships.values_mut() {
                        let d = s.pos.distance(center);
                        if s.alive && d < radius {
                            let push = (s.pos - center).try_normalize().unwrap_or(Vec3::Y);
                            s.vel += push * (strength * (1.0 - d / radius));
                        }
                    }
                    self.push_debris(center, radius, strength);
                    events.push(Event::Cue(Cue {
                        kind: CueKind::Shockwave,
                        source: None,
                        target: None,
                        pos: center,
                        value: radius.clamp(0.0, u16::MAX as f32) as u16,
                    }));
                }
                Command::Edit(edit) => {
                    // Craters strain the rock (Instability, M4).
                    if let TerrainEdit::SubtractSphere { center, radius } = edit {
                        if cfg.setup.instability > 0 && !self.no_strain {
                            let material = self
                                .instability
                                .material_at(center)
                                .unwrap_or_else(|| terrain.material_at(center));
                            self.instability.add_strain(
                                InstabilitySettings::builtin(),
                                center,
                                radius,
                                material,
                            );
                        }
                    }
                    terrain.apply_edit(&edit);
                    if edit.changes_geometry() {
                        let (c, r) = (edit.center(), edit.radius());
                        for e in &mut self.entities {
                            let reach = r + e.radius + 1.0;
                            if e.resting && e.pos.distance_squared(c) <= reach * reach {
                                e.resting = false;
                            }
                        }
                    }
                    events.push(Event::TerrainEdit(edit));
                }
                Command::SpawnProjectile(p) => {
                    let mut p = *p;
                    p.id = self.alloc_projectile_id();
                    p.flags &= !proj_flags::REMOVED;
                    p.flags |= self.effects.projectile_flags;
                    p.passed = None;
                    if p.generation == 0 {
                        events.push(Event::Fired {
                            player: p.owner,
                            weapon: p.weapon,
                            projectile: p.id,
                        });
                    }
                    self.projectiles.push(p);
                }
                Command::SpawnEntity(s) => {
                    self.insert_entity(*s);
                }
                Command::ImpulseShip { target, dv } => {
                    if let Some(s) = self.ships.get_mut(&target) {
                        if s.alive {
                            s.vel += dv;
                        }
                    }
                }
                Command::MovePickup { id, delta } => {
                    if let Some(pile) = self.scrap.iter_mut().find(|s| s.id == id) {
                        pile.pos += delta;
                    } else if let Some(sp) = self
                        .pickups
                        .iter_mut()
                        .find(|sp| sp.content.is_some() && sp.id == id)
                    {
                        sp.offset += delta;
                    }
                }
                Command::ImpulseEntity { id, dv } => {
                    if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
                        e.vel += dv / e.mass();
                        e.resting = false;
                    }
                }
                Command::DamageEntity { id, amount, .. } => {
                    if let Some(i) = self.entities.iter().position(|e| e.id == id) {
                        self.entities[i].hp -= amount;
                        if self.entities[i].hp <= 0.0 {
                            self.entities.remove(i);
                        }
                    }
                }
                Command::RemoveEntity { id } => self.entities.retain(|e| e.id != id),
                Command::ImpulseProjectile { id, dv } => {
                    if let Some(p) = self
                        .projectiles
                        .iter_mut()
                        .chain(self.stepping.iter_mut())
                        .chain(self.stepped.iter_mut())
                        .find(|p| p.id == id)
                    {
                        p.vel += dv;
                        if dv != Vec3::ZERO {
                            // Pulled loose (a stuck mine).
                            p.flags &= !proj_flags::STUCK;
                        }
                    }
                }
                Command::RemoveProjectile { id } => {
                    for list in [&mut self.projectiles, &mut self.stepping, &mut self.stepped] {
                        list.retain(|p| p.id != id);
                    }
                }
                Command::DrainEnergy { from, to, amount } => {
                    let Some(src) = self.ships.get_mut(&from) else {
                        continue;
                    };
                    let taken = amount.clamp(0.0, src.energy.max(0.0));
                    src.energy -= taken;
                    if let Some(dst) = to.and_then(|t| self.ships.get_mut(&t)) {
                        let max = cfg.tuning(dst.ship_type).energy_max;
                        dst.energy = (dst.energy + taken).min(max);
                    }
                }
                Command::Blind { target, ticks } => {
                    if let Some(s) = self.ships.get_mut(&target) {
                        s.blind_until = s.blind_until.max(now + ticks);
                        events.push(Event::Cue(Cue {
                            kind: CueKind::Blinded,
                            source: None,
                            target: Some(target),
                            pos: s.pos,
                            value: ticks.min(u16::MAX as u64) as u16,
                        }));
                    }
                }
                Command::Swap { a, b } => {
                    if a == b {
                        continue;
                    }
                    let (Some(sa), Some(sb)) = (self.ships.get(&a), self.ships.get(&b)) else {
                        continue;
                    };
                    let (pa, va, pb, vb) = (sa.pos, sa.vel, sb.pos, sb.vel);
                    if let Some(s) = self.ships.get_mut(&a) {
                        s.pos = pb;
                        s.vel = vb;
                    }
                    if let Some(s) = self.ships.get_mut(&b) {
                        s.pos = pa;
                        s.vel = va;
                    }
                }
                Command::Rewind { player, ticks } => {
                    let Some((pos, vel, hull)) =
                        self.trails.get(&player).and_then(|t| t.get(ticks))
                    else {
                        continue;
                    };
                    if let Some(s) = self.ships.get_mut(&player) {
                        if s.alive {
                            s.pos = pos;
                            s.vel = vel;
                            s.hull = hull.max(s.hull.min(1.0));
                        }
                    }
                    self.trails.remove(&player);
                }
                Command::DisableWeapons { player, ticks } => {
                    if let Some(s) = self.ships.get_mut(&player) {
                        s.weapons_offline_until = s.weapons_offline_until.max(now + ticks);
                        s.charge = None;
                    }
                }
                Command::AdjustHull {
                    target,
                    delta,
                    can_kill,
                } => {
                    let Some(s) = self.ships.get_mut(&target) else {
                        continue;
                    };
                    if !s.alive {
                        continue;
                    }
                    let max = cfg.tuning(s.ship_type).hull_max;
                    let floor = if can_kill {
                        f32::NEG_INFINITY
                    } else {
                        s.hull.min(1.0)
                    };
                    s.hull = (s.hull + delta).min(max).max(floor);
                    if s.hull <= 0.0 {
                        self.kill(cfg, now, target, None, None, events);
                    }
                }
                Command::SetWeaponState {
                    ship,
                    weapon,
                    value,
                } => {
                    if let Some(s) = self.ships.get_mut(&ship) {
                        if s.weapon(weapon).is_none() {
                            continue;
                        }
                        if value == 0 {
                            s.weapon_state.remove(&weapon);
                        } else {
                            s.weapon_state.insert(weapon, value);
                        }
                    }
                }
                Command::Emit(e) => events.push(e),
                Command::EditLater { tick, edit } => self.deferred_edits.push((tick, edit)),
                Command::AddWell(w) => self.add_well(w),
                Command::RemoveWell { key } => self.wells.dynamics.retain(|w| w.key != key),
            }
        }
    }

    /// Pushes debris away from `center` with linear falloff to `radius`.
    fn push_debris(&mut self, center: Vec3, radius: f32, strength: f32) {
        if strength <= 0.0 {
            return;
        }
        for e in &mut self.entities {
            if e.kind != EntityKind::Debris {
                continue;
            }
            let d = e.pos.distance(center);
            if d >= radius {
                continue;
            }
            let push = (e.pos - center).try_normalize().unwrap_or(Vec3::Y);
            e.vel += push * (strength * (1.0 - d / radius) / e.mass());
            e.resting = false;
        }
    }

    /// Applies damage and knockback to ship `target`: friendly-fire scaling
    /// between teammates, then the target's active gadget (Shield), then
    /// hull, events and kill.
    #[allow(clippy::too_many_arguments)]
    fn apply_damage(
        &mut self,
        cfg: &SimConfig,
        now: u64,
        target: PlayerId,
        damage: f32,
        knockback: Vec3,
        source: DamageSource,
        direct: bool,
        events: &mut Vec<Event>,
    ) {
        // Pigment (M4): a pigmented attacker deals +25 %.
        let damage = match source.attacker.and_then(|a| self.ships.get(&a)) {
            Some(a) if a.pigmented(now) => damage * (1.0 + PigmentSettings::builtin().damage_bonus),
            _ => damage,
        };
        let Some(ship) = self.ships.get_mut(&target) else {
            return;
        };
        if !ship.alive {
            return;
        }
        let scale = if ship.team == source.team && Some(ship.player) != source.attacker {
            cfg.resolved.friendly_fire_scale()
        } else {
            1.0
        };
        let mut kb = knockback * scale;
        let mut amount = damage * scale;
        let mut hit = ModeHit {
            target,
            attacker: source.attacker,
            weapon: source.weapon,
            amount,
            direct,
            self_inflicted: source.attacker == Some(target),
            same_side: ship.team == source.team && Some(ship.player) != source.attacker,
            ignore_gadget: false,
        };
        {
            let m = ModeRef {
                state: &self.mode,
                rules: &cfg.rules.mode,
                mutators: &cfg.rules.mutators,
                ships: &self.ships,
                now,
            };
            cfg.mode_module.on_damage(&m, &mut hit);
            for mm in &cfg.mutator_modules {
                mm.on_damage(&m, &mut hit);
            }
        }
        cfg.mode_module.on_hit(
            &mut ModeMut {
                state: &mut self.mode,
                rules: &cfg.rules.mode,
                mutators: &cfg.rules.mutators,
                ships: &self.ships,
                now,
            },
            &hit,
        );
        amount = hit.amount;
        let Some(ship) = self.ships.get_mut(&target) else {
            return;
        };
        if let Some(g) = ship
            .gadget
            .filter(|_| ship.gadget_active && !hit.ignore_gadget)
        {
            let mut hit = IncomingHit {
                amount,
                knockback: kb,
                attacker: source.attacker,
                weapon: source.weapon,
                shield: source.weapon.map_or(ShieldInteraction::Wears(1.0), |w| {
                    cfg.resolved.shield_interaction(w)
                }),
                self_inflicted: source.attacker == Some(target),
            };
            cfg.gadget_behaviors[g.0 as usize].on_damage(cfg.gadgets.get(g), ship, &mut hit);
            amount = hit.amount;
            kb = hit.knockback;
        }
        ship.vel += kb;
        if amount <= 0.0 {
            return;
        }
        ship.hull -= amount;
        if let Some(a) = source.attacker.filter(|a| *a != target) {
            ship.last_attacker = Some(a);
        }
        events.push(Event::Damaged {
            target,
            attacker: source.attacker,
            weapon: source.weapon,
            amount,
            direct,
        });
        if ship.hull <= 0.0 {
            self.kill(cfg, now, target, source.attacker, source.weapon, events);
        }
    }

    fn kill(
        &mut self,
        cfg: &SimConfig,
        now: u64,
        victim: PlayerId,
        killer: Option<PlayerId>,
        weapon: Option<WeaponId>,
        events: &mut Vec<Event>,
    ) {
        let Some(ship) = self.ships.get_mut(&victim) else {
            return;
        };
        let pigmented = ship.pigmented(now);
        ship.pigment_until = 0;
        ship.hull = 0.0;
        ship.alive = false;
        ship.deaths += 1;
        ship.respawn_tick = now + cfg.resolved.respawn_ticks.max(1);
        ship.vel = Vec3::ZERO;
        ship.gadget_active = false;
        ship.charge = None;
        let team = ship.team;
        let wreck_pos = ship.pos;
        let wreck_amount = cfg.tuning(ship.ship_type).wreck_scrap + ship.scrap * 0.5;
        let mut killer_side = None;
        if let Some(k) = killer.filter(|k| *k != victim) {
            if let Some(ks) = self.ships.get_mut(&k) {
                killer_side = Some(ks.team);
                if ks.team != team {
                    ks.kills += 1;
                }
            }
        }
        self.trails.remove(&victim);
        events.push(Event::Killed {
            victim,
            killer,
            weapon,
        });
        let kill = Kill {
            victim,
            victim_side: team,
            killer,
            killer_side,
            weapon,
        };
        let mut m = ModeMut {
            state: &mut self.mode,
            rules: &cfg.rules.mode,
            mutators: &cfg.rules.mutators,
            ships: &self.ships,
            now,
        };
        cfg.mode_module.on_kill(&mut m, &kill);
        if wreck_amount > 0.0 {
            let id = self.alloc_entity_id();
            self.scrap.push(ScrapPile {
                id,
                pos: wreck_pos,
                amount: wreck_amount,
                spawn: None,
                wreck: true,
            });
            let wrecks = self.scrap.iter().filter(|s| s.wreck).count();
            if wrecks > MAX_WRECKS {
                if let Some(i) = self.scrap.iter().position(|s| s.wreck) {
                    self.scrap.remove(i);
                }
            }
        }
        // Pigment (M4): a pigmented ship drops a splash in its colour.
        if pigmented && cfg.pigment_rule.is_some() {
            let id = self.alloc_entity_id();
            let secs = PigmentSettings::builtin().splash_secs;
            self.pigment.pickups.push(PigmentPickup {
                id,
                pos: wreck_pos,
                point: None,
                side: team,
                expires_tick: now + ls_math::Tick::ticks_for_secs(secs).max(1),
            });
        }
    }

    /// Tug support: passes scrap to the nearest teammate in reach that has
    /// room (hull first, like scavenging).
    fn pass_scrap(&mut self, cfg: &SimConfig, id: PlayerId) {
        let Some(me) = self.ships.get(&id) else {
            return;
        };
        let t = cfg.tuning(me.ship_type);
        let budget = (t.scrap_pass_rate * DT).min(me.scrap);
        if budget <= 0.0 {
            return;
        }
        let reach = t.scavenge_radius;
        let mut best: Option<(PlayerId, f32)> = None;
        for s in self.ships.values() {
            if s.player == id || !s.alive || s.team != me.team {
                continue;
            }
            let st = cfg.tuning(s.ship_type);
            let room = (st.hull_max - s.hull).max(0.0) + (st.scrap_max - s.scrap).max(0.0);
            let d = s.pos.distance_squared(me.pos);
            if room > 0.0 && d <= reach * reach && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((s.player, d));
            }
        }
        let Some((mate, _)) = best else {
            return;
        };
        let Some(m) = self.ships.get_mut(&mate) else {
            return;
        };
        let mt = cfg.tuning(m.ship_type);
        let hull_room = if mt.hull_per_scrap > 0.0 {
            (mt.hull_max - m.hull).max(0.0) / mt.hull_per_scrap
        } else {
            0.0
        };
        let store_room = (mt.scrap_max - m.scrap).max(0.0);
        let give = budget.min(hull_room + store_room);
        let to_hull = give.min(hull_room);
        m.hull = (m.hull + to_hull * mt.hull_per_scrap).min(mt.hull_max);
        m.scrap = (m.scrap + (give - to_hull)).min(mt.scrap_max);
        if let Some(me) = self.ships.get_mut(&id) {
            me.scrap -= give;
        }
    }

    /// Removes empty scrap piles; empty scrap pickups start their respawn timer.
    fn cleanup_scrap(&mut self, cfg: &SimConfig, now: u64) {
        let pickups = &mut self.pickups;
        self.scrap.retain(|pile| {
            if pile.amount > 1e-4 {
                return true;
            }
            if let Some(i) = pile.spawn {
                if let Some(sp) = pickups.get_mut(i as usize) {
                    if let Some((entry, _)) = sp.content.take() {
                        sp.respawn_tick =
                            now + cfg.resolved.pickup_table[entry as usize].respawn_ticks;
                    }
                }
            }
            false
        });
    }

    /// Ships touching pickups collect them (lowest player id first).
    fn collect_pickups(&mut self, cfg: &SimConfig, now: u64, events: &mut Vec<Event>) {
        let mode = &self.mode;
        let module = cfg.mode_module;
        let mood = self.director.current.as_ref().map(|_| self.director.view());
        for (i, sp) in self.pickups.iter_mut().enumerate() {
            let Some((entry, kind)) = sp.content else {
                continue;
            };
            if mood.is_some_and(|v| !crate::director::allows_pickup(&v, cfg.seed, now, i as u16)) {
                continue;
            }
            if matches!(kind, PickupKind::Scrap { .. }) {
                continue;
            }
            for ship in self.ships.values_mut() {
                let reach = PICKUP_RADIUS + cfg.tuning(ship.ship_type).radius;
                if !ship.alive || ship.pos.distance_squared(sp.content_pos()) > reach * reach {
                    continue;
                }
                if !module.on_pickup(mode, &cfg.rules.mode, ship, &kind) {
                    continue;
                }
                let taken = match kind {
                    PickupKind::Weapon { weapon, ammo } => collect_weapon(ship, weapon, ammo, cfg),
                    PickupKind::Upgrade { weapon } => collect_upgrade(ship, weapon, cfg),
                    PickupKind::Gadget { gadget } => collect_gadget(ship, gadget, cfg),
                    PickupKind::Scrap { .. } => false,
                };
                if taken {
                    sp.content = None;
                    sp.offset = Vec3::ZERO;
                    sp.respawn_tick = now + cfg.resolved.pickup_table[entry as usize].respawn_ticks;
                    events.push(Event::PickupTaken {
                        spawn: i as u16,
                        player: ship.player,
                        kind,
                    });
                    break;
                }
            }
        }
    }

    /// Rolls new content for empty spawn points whose timer ran out.
    fn update_pickup_spawns(&mut self, events: &mut Vec<Event>) {
        let cfg = Arc::clone(&self.config);
        let table = &cfg.resolved.pickup_table;
        for i in 0..self.pickups.len() {
            let sp = &self.pickups[i];
            if sp.content.is_some() || self.tick < sp.respawn_tick {
                continue;
            }
            // Weights depend on what is on the map (per-map caps).
            let weights: Vec<f32> = table
                .iter()
                .enumerate()
                .map(|(entry, e)| {
                    // Defensive: never spawn something the rule set does not
                    // allow as a pickup.
                    let allowed = match e.kind {
                        PickupKind::Weapon { weapon, .. } | PickupKind::Upgrade { weapon } => {
                            cfg.resolved.availability(weapon).as_pickup()
                        }
                        PickupKind::Gadget { gadget } => {
                            cfg.resolved.gadget_availability(gadget).as_pickup()
                        }
                        PickupKind::Scrap { .. } => true,
                    };
                    let capped = e.max_on_map > 0
                        && self
                            .pickups
                            .iter()
                            .filter(|p| p.content.is_some_and(|(x, _)| x as usize == entry))
                            .count()
                            >= e.max_on_map as usize;
                    if allowed && !capped {
                        e.weight
                    } else {
                        0.0
                    }
                })
                .collect();
            let Some(entry) = self.rng.weighted_index(&weights) else {
                // Nothing can spawn here. If only per-map caps are in the way,
                // retry later; otherwise never.
                let capped_only = table.iter().any(|e| e.weight > 0.0 && e.max_on_map > 0);
                self.pickups[i].respawn_tick = if capped_only {
                    let retry = table.iter().map(|e| e.respawn_ticks).min().unwrap_or(1);
                    self.tick + retry.max(1)
                } else {
                    u64::MAX
                };
                continue;
            };
            let kind = table[entry].kind;
            let pos = self.pickups[i].pos;
            let id = self.alloc_entity_id();
            self.pickups[i].content = Some((entry as u16, kind));
            self.pickups[i].id = id;
            self.pickups[i].offset = Vec3::ZERO;
            if let PickupKind::Scrap { amount } = kind {
                self.scrap.push(ScrapPile {
                    id,
                    pos,
                    amount,
                    spawn: Some(i as u16),
                    wreck: false,
                });
            }
            events.push(Event::PickupSpawned {
                spawn: i as u16,
                kind,
            });
        }
    }

    // ----- entities ----------------------------------------------------------

    /// Sets the projectile id counter (tests of id wrapping).
    #[cfg(test)]
    pub(crate) fn set_next_projectile_for_test(&mut self, next: u32) {
        self.next_projectile = next;
    }

    /// Next projectile id whose wire id (low 16 bits) no live projectile
    /// uses (a long-lived mine must not share its wire id with a new shot
    /// after the counter passed 65536).
    fn alloc_projectile_id(&mut self) -> u32 {
        if self.next_projectile <= PROJECTILE_WIRE_MASK {
            // The counter has not wrapped the wire range yet: ids are unique.
            let id = self.next_projectile;
            self.next_projectile += 1;
            return id;
        }
        let live: BTreeSet<u32> = self
            .projectiles
            .iter()
            .chain(&self.stepping)
            .chain(&self.stepped)
            .map(|p| p.id & PROJECTILE_WIRE_MASK)
            .collect();
        for _ in 0..=PROJECTILE_WIRE_MASK {
            let id = self.next_projectile;
            self.next_projectile = self.next_projectile.wrapping_add(1);
            if !live.contains(&(id & PROJECTILE_WIRE_MASK)) {
                return id;
            }
        }
        self.next_projectile
    }

    /// Next entity id whose wire id (low 15 bits) no live entity uses.
    fn alloc_entity_id(&mut self) -> u32 {
        let live: BTreeSet<u32> = self
            .entities
            .iter()
            .map(|e| e.id)
            .chain(self.scrap.iter().map(|s| s.id))
            .chain(
                self.pickups
                    .iter()
                    .filter(|p| p.content.is_some())
                    .map(|p| p.id),
            )
            .chain(self.pigment.pickups.iter().map(|p| p.id))
            .map(|id| id & WIRE_ID_MASK)
            .collect();
        for _ in 0..=WIRE_ID_MASK {
            let id = self.next_entity;
            self.next_entity = self.next_entity.wrapping_add(1);
            if !live.contains(&(id & WIRE_ID_MASK)) {
                return id;
            }
        }
        // 32768 live entities: cannot happen with the caps; reuse the id.
        self.next_entity
    }

    fn insert_entity(&mut self, s: EntitySpawn) -> u32 {
        let id = self.alloc_entity_id();
        if s.kind == EntityKind::Debris {
            let debris = self
                .entities
                .iter()
                .filter(|e| e.kind == EntityKind::Debris)
                .count();
            if debris >= MAX_DEBRIS {
                if let Some(i) = self
                    .entities
                    .iter()
                    .position(|e| e.kind == EntityKind::Debris)
                {
                    self.entities.remove(i);
                }
            }
        }
        self.entities.push(Entity {
            id,
            kind: s.kind,
            source: s.source,
            visual: s.visual,
            pos: s.pos,
            vel: s.vel,
            rot: s.rot,
            radius: s.radius,
            owner: s.owner,
            team: s.team,
            spawn_tick: self.tick,
            expires_tick: s
                .lifetime_ticks
                .map_or(u64::MAX, |t| self.tick.saturating_add(t.max(1))),
            hp: s.hp,
            scrap: s.scrap,
            resting: false,
            kinematic: s.kinematic,
            vars: s.vars,
            state: s.state,
        });
        id
    }

    /// Generic replication view of every entity-like thing (debris, decoys,
    /// ghosts, dust, markers, pickups, scrap piles and wrecks), sorted by
    /// wire id. Projectiles and ships are replicated separately.
    pub fn entity_views(&self) -> Vec<EntityView> {
        let cfg = &self.config;
        let mut out: Vec<EntityView> = Vec::new();
        for e in &self.entities {
            let state = match e.kind {
                // Radius in tenths of a unit (3..=15).
                EntityKind::Debris => [
                    state_byte(e.scrap),
                    e.resting as u8,
                    state_byte(e.radius * 10.0),
                    0,
                ],
                // Radius in whole units (the volume that cuts visibility).
                EntityKind::DustCloud => [state_byte(e.radius), e.state[1], e.state[2], e.state[3]],
                EntityKind::Decoy => {
                    let max = e
                        .owner
                        .and_then(|o| self.ships.get(&o))
                        .map_or(100.0, |s| cfg.tuning(s.ship_type).hull_max);
                    [state_byte(e.hp * 100.0 / max.max(1.0)), e.state[0], 0, 0]
                }
                _ => e.state,
            };
            out.push(EntityView {
                id: e.id,
                kind: e.kind.into(),
                visual: e.visual,
                pos: e.pos,
                vel: e.vel,
                rot: e.rot,
                owner: e.owner,
                team: e.team,
                state,
            });
        }
        for sp in &self.pickups {
            let Some((_, kind)) = sp.content else {
                continue;
            };
            let (kind, visual, state) = match kind {
                PickupKind::Weapon { weapon, ammo } => (
                    ReplicatedKind::WeaponPickup,
                    cfg.visuals.weapon_pickup(weapon),
                    [weapon.0, ammo.min(255) as u8, 0, 0],
                ),
                PickupKind::Upgrade { weapon } => (
                    ReplicatedKind::UpgradePickup,
                    BuiltinVisual::UpgradePickup.id(),
                    [weapon.0, 0, 0, 0],
                ),
                PickupKind::Gadget { gadget } => (
                    ReplicatedKind::GadgetPickup,
                    cfg.visuals.gadget_pickup(gadget),
                    [gadget.0, 0, 0, 0],
                ),
                PickupKind::Scrap { .. } => continue,
            };
            out.push(EntityView {
                id: sp.id,
                kind,
                visual,
                pos: sp.content_pos(),
                vel: Vec3::ZERO,
                rot: Quat::IDENTITY,
                owner: None,
                team: NO_TEAM,
                state,
            });
        }
        for p in &self.pigment.pickups {
            out.push(EntityView {
                id: p.id,
                kind: ReplicatedKind::PigmentPickup,
                visual: BuiltinVisual::UpgradePickup.id(),
                pos: p.pos,
                vel: Vec3::ZERO,
                rot: Quat::IDENTITY,
                owner: None,
                team: if p.point.is_none() { p.side } else { NO_TEAM },
                state: [p.point.is_none() as u8, p.side, 0, 0],
            });
        }
        for s in &self.scrap {
            let (kind, visual) = if s.wreck {
                (ReplicatedKind::Wreck, BuiltinVisual::Wreck.id())
            } else {
                (ReplicatedKind::ScrapPile, BuiltinVisual::ScrapPile.id())
            };
            out.push(EntityView {
                id: s.id,
                kind,
                visual,
                pos: s.pos,
                vel: Vec3::ZERO,
                rot: Quat::IDENTITY,
                owner: None,
                team: NO_TEAM,
                state: [state_byte(s.amount), 0, 0, 0],
            });
        }
        out.sort_by_key(|v| v.wire_id());
        out
    }

    // ----- hashing ---------------------------------------------------------

    /// Hash of the complete simulation state (not the terrain). Two worlds with
    /// equal hashes are, for all practical purposes, in the same state.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.write_u64(self.tick);
        let mut rng = self.rng.clone();
        h.write_u64(rng.next_u64());
        h.write_u64(rng.next_u64());
        h.write_u16(self.next_player);
        h.write_u32(self.next_projectile);
        h.write_u32(self.next_entity);
        h.write_u32(self.spawn_cursor[0]);
        h.write_u32(self.spawn_cursor[1]);
        if self.spawn_cursor[2..].iter().any(|&c| c != 0) {
            h.write_u32(self.spawn_cursor[2]);
            h.write_u32(self.spawn_cursor[3]);
        }

        h.write_u32(self.ships.len() as u32);
        for s in self.ships.values() {
            hash_ship(&mut h, s);
        }
        h.write_u32(self.projectiles.len() as u32);
        for p in &self.projectiles {
            hash_projectile(&mut h, p);
        }
        h.write_u32(self.entities.len() as u32);
        for e in &self.entities {
            hash_entity(&mut h, e);
        }
        h.write_u32(self.scrap.len() as u32);
        for s in &self.scrap {
            h.write_u32(s.id);
            h.write_vec3(s.pos);
            h.write_f32(s.amount);
            h.write_u32(s.spawn.map_or(u32::MAX, |x| x as u32));
            h.write_bool(s.wreck);
        }
        h.write_u32(self.pickups.len() as u32);
        for sp in &self.pickups {
            h.write_vec3(sp.pos);
            h.write_vec3(sp.offset);
            h.write_u64(sp.respawn_tick);
            h.write_u32(sp.id);
            match sp.content {
                None => h.write_u8(0),
                Some((entry, kind)) => {
                    h.write_u8(1);
                    h.write_u16(entry);
                    match kind {
                        PickupKind::Weapon { weapon, ammo } => {
                            h.write_u8(0);
                            h.write_u8(weapon.0);
                            h.write_u32(ammo);
                        }
                        PickupKind::Upgrade { weapon } => {
                            h.write_u8(1);
                            h.write_u8(weapon.0);
                        }
                        PickupKind::Gadget { gadget } => {
                            h.write_u8(2);
                            h.write_u8(gadget.0);
                        }
                        PickupKind::Scrap { amount } => {
                            h.write_u8(3);
                            h.write_f32(amount);
                        }
                    }
                }
            }
        }
        h.write_u32(self.trails.len() as u32);
        for (id, t) in &self.trails {
            h.write_u16(id.0);
            h.write_u32(t.points.len() as u32);
            for (p, v, hull) in &t.points {
                h.write_vec3(*p);
                h.write_vec3(*v);
                h.write_f32(*hull);
            }
        }
        h.write_u64(self.mode.end_tick);
        h.write_u64(self.mode.coverage.team_cells[0]);
        h.write_u64(self.mode.coverage.team_cells[1]);
        h.write_u64(self.mode.coverage.surface_cells);
        match self.mode.phase {
            MatchPhase::Running => h.write_u8(0),
            MatchPhase::Ended { winner } => {
                h.write_u8(1);
                h.write_u8(winner.map_or(u8::MAX, |w| w));
            }
        }
        // Everything else of the mode state and the deferred edits, only
        // when present (a two-team Paint the Void match without deferred
        // edits keeps its M2 hash).
        hash_mode_extras(&mut h, &self.mode);
        // The director only once it did anything (Absurdity 0 keeps the
        // M3 hashes).
        if !self.director.is_pristine() {
            self.director.hash(&mut h);
        }
        if !self.deferred_edits.is_empty() {
            h.write_u8(0xde);
            h.write_u32(self.deferred_edits.len() as u32);
            for (t, e) in &self.deferred_edits {
                h.write_u64(*t);
                hash_edit(&mut h, e);
            }
        }
        // World physics (M4), only when present (quiet matches keep their
        // M3 hashes).
        if self.up != Vec3::Y {
            h.write_u8(0x75);
            h.write_vec3(self.up);
        }
        if !self.wells.dynamics.is_empty() {
            h.write_u8(0x77);
            h.write_u32(self.wells.dynamics.len() as u32);
            for w in &self.wells.dynamics {
                h.write_u32(w.key);
                h.write_vec3(w.pos);
                h.write_f32(w.peak_accel);
                h.write_f32(w.range);
                h.write_u64(w.start_tick);
                h.write_u64(w.end_tick);
                h.write_u16(w.owner.map_or(u16::MAX, |p| p.0));
            }
        }
        if !self.instability.is_empty() {
            h.write_u8(0x1e);
            self.instability.hash_into(&mut h);
        }
        if !self.pigment.is_empty() {
            h.write_u8(0x91);
            self.pigment.hash_into(&mut h);
        }
        h.finish()
    }
}

// ----- world physics (M4) ---------------------------------------------------

impl World {
    /// The level's up (world +Y unless an event turned it; horizon assist
    /// levels the roll to it).
    pub fn up(&self) -> Vec3 {
        self.up
    }

    /// Turns the level's up (Bored level; normalized, ignored if zero).
    pub fn set_up(&mut self, up: Vec3) {
        if let Some(u) = up.try_normalize() {
            self.up = u;
        }
    }

    /// Every well of the match now: the level's static wells and the
    /// dynamic ones (the client predictor evaluates the same field).
    pub fn well_field(&self) -> &WellField {
        &self.wells
    }

    /// The dynamic wells (active, or starting later).
    pub fn dynamic_wells(&self) -> &[DynamicWell] {
        &self.wells.dynamics
    }

    /// Adds a dynamic well (also [`Command::AddWell`]); ignored if it ends
    /// before it starts or has no strength.
    pub fn add_well(&mut self, w: DynamicWell) {
        if w.end_tick > w.start_tick && w.peak_accel > 0.0 && w.range > 0.0 && w.pos.is_finite() {
            self.wells.dynamics.push(w);
        }
    }

    /// Removes every dynamic well (a new heat, tests).
    pub fn clear_wells(&mut self) {
        self.wells.dynamics.clear();
    }

    /// Strain cells, the coming cave-in and the rubble regions.
    pub fn instability(&self) -> &InstabilityState {
        &self.instability
    }

    /// Pigment pickups in the level (monochrome levels).
    pub fn pigment_pickups(&self) -> &[PigmentPickup] {
        &self.pigment.pickups
    }

    /// Radiation dose rate (hull/s, with the mode's scale) at `p`; 0 on
    /// classic levels.
    pub fn dose_rate(&self, p: Vec3) -> f32 {
        let cfg = &self.config;
        let Some(belt) = &cfg.setup.belt else {
            return 0.0;
        };
        let scale = cfg.mode_module.radiation_scale(&self.mode_ref(cfg));
        if scale <= 0.0 {
            return 0.0;
        }
        crate::radiation::dose_rate(belt, crate::radiation::RadiationSettings::builtin(), p) * scale
    }

    /// Stage 7 of a tick (see the module docs).
    fn world_physics<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        self.radiation(cfg, now, events);
        if !self.wells.dynamics.is_empty() {
            self.wells.dynamics.retain(|w| now < w.end_tick);
        }
        self.step_pigment(cfg, now, events);
        if cfg.setup.instability > 0 {
            self.step_instability(cfg, terrain, now, events);
        }
    }

    /// Radiation on belt levels: hull loss past shields, no kill credit.
    fn radiation(&mut self, cfg: &SimConfig, now: u64, events: &mut Vec<Event>) {
        let Some(belt) = &cfg.setup.belt else {
            return;
        };
        let scale = cfg.mode_module.radiation_scale(&self.mode_ref(cfg));
        if scale <= 0.0 {
            return;
        }
        let settings = crate::radiation::RadiationSettings::builtin();
        let mut dead = Vec::new();
        for s in self.ships.values_mut() {
            if !s.alive {
                continue;
            }
            let rate = crate::radiation::dose_rate(belt, settings, s.pos) * scale;
            if rate <= 0.0 {
                continue;
            }
            s.hull -= rate * DT;
            if s.hull <= 0.0 {
                dead.push(s.player);
            }
        }
        for id in dead {
            self.kill(cfg, now, id, None, None, events);
        }
    }

    /// Pigment pickups: expiry, collection, respawns (see [`crate::pigment`]).
    fn step_pigment(&mut self, cfg: &SimConfig, now: u64, events: &mut Vec<Event>) {
        let Some(rule) = cfg.pigment_rule else {
            return;
        };
        let settings = PigmentSettings::builtin();
        self.pigment.pickups.retain(|p| p.expires_tick > now);
        // Collection (lowest player id first).
        let mut i = 0;
        while i < self.pigment.pickups.len() {
            let pk = self.pigment.pickups[i];
            let taker = self
                .ships
                .values()
                .find(|s| {
                    let reach = PICKUP_RADIUS + cfg.tuning(s.ship_type).radius;
                    s.alive && s.pos.distance_squared(pk.pos) <= reach * reach
                })
                .map(|s| s.player);
            let Some(id) = taker else {
                i += 1;
                continue;
            };
            self.pigment.pickups.remove(i);
            if pk.point.is_some() {
                self.pigment
                    .respawns
                    .push(now + ls_math::Tick::ticks_for_secs(rule.respawn_secs).max(1));
            }
            let consumed = cfg.mode_module.on_pigment(
                &mut ModeMut {
                    state: &mut self.mode,
                    rules: &cfg.rules.mode,
                    mutators: &cfg.rules.mutators,
                    ships: &self.ships,
                    now,
                },
                id,
                settings.restore_colour,
            );
            if !consumed {
                if let Some(s) = self.ships.get_mut(&id) {
                    s.pigment_until =
                        now + ls_math::Tick::ticks_for_secs(settings.duration_secs).max(1);
                }
            }
            events.push(Event::Cue(Cue {
                kind: CueKind::Pigment,
                source: Some(id),
                target: None,
                pos: pk.pos,
                value: pk.point.is_none() as u16,
            }));
        }
        // How many regular pickups the players in play want.
        let points = cfg.setup.pickup_points.len();
        let players = self
            .ships
            .keys()
            .filter(|id| !self.mode.player(**id).is_some_and(|p| p.is_out()))
            .count();
        let want = rule.count(players).min(points);
        while self.pigment.regular() > want && !self.pigment.respawns.is_empty() {
            self.pigment.respawns.pop();
        }
        while self.pigment.regular() < want {
            self.pigment.respawns.push(now);
        }
        // Due respawns at free pickup points.
        if !self.pigment.respawns.iter().any(|&t| t <= now) {
            return;
        }
        let seed = ls_math::rng::mix64(
            cfg.seed ^ ls_math::rng::mix64(crate::pigment::PIGMENT_RNG_KEY ^ now.rotate_left(32)),
        );
        let mut rng = Pcg32::new(seed, crate::behavior::RNG_STREAM_MODULES);
        let mut pending = std::mem::take(&mut self.pigment.respawns);
        pending.sort_unstable();
        for t in pending {
            if t > now {
                self.pigment.respawns.push(t);
                continue;
            }
            let free: Vec<u16> = (0..points as u16)
                .filter(|i| !self.pigment.pickups.iter().any(|p| p.point == Some(*i)))
                .collect();
            if free.is_empty() {
                self.pigment.respawns.push(t);
                continue;
            }
            let point = free[rng.below(free.len() as u32) as usize];
            let id = self.alloc_entity_id();
            self.pigment.pickups.push(PigmentPickup {
                id,
                pos: cfg.setup.pickup_points[point as usize],
                point: Some(point),
                side: 0,
                expires_tick: u64::MAX,
            });
        }
    }

    /// Instability: strain decay, the warning, the cave-in.
    fn step_instability<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        let settings = InstabilitySettings::builtin();
        if now % ls_math::TICK_HZ as u64 == 0 && !self.instability.cells.is_empty() {
            self.instability.decay(settings);
        }
        if let Some(p) = self.instability.pending {
            if now >= p.due_tick {
                self.instability.pending = None;
                self.cave_in(cfg, terrain, now, p.pos, events);
            }
            return;
        }
        let interval = ls_math::Tick::ticks_for_secs(settings.min_interval_secs);
        if self
            .instability
            .last_cave_in
            .is_some_and(|t| now < t + interval)
        {
            return;
        }
        let threshold = settings.threshold(cfg.setup.instability);
        let setup = &cfg.setup;
        let excluded = |p: Vec3| {
            setup.spawns.iter().any(|s| {
                s.pos.distance_squared(p) < settings.spawn_clearance * settings.spawn_clearance
            }) || setup.heist.as_ref().is_some_and(|h| {
                p.distance(h.core) < h.chamber_radius + h.knot_radius
                    || h.extraction
                        .iter()
                        .any(|(c, r)| p.distance(*c) < r + settings.extraction_clearance)
            })
        };
        let Some(pos) = self.instability.take_over_threshold(threshold, excluded) else {
            return;
        };
        let warn = ls_math::Tick::ticks_for_secs(settings.warning_secs).max(1);
        self.instability.pending = Some(crate::instability::PendingCaveIn {
            pos,
            due_tick: now + warn,
        });
        self.instability.last_cave_in = Some(now);
        events.push(Event::Cue(Cue {
            kind: CueKind::CaveInWarning,
            source: None,
            target: None,
            pos,
            value: warn.min(u16::MAX as u64) as u16,
        }));
    }

    /// Brings the rock down at `pos` (see [`crate::instability`]).
    fn cave_in<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        pos: Vec3,
        events: &mut Vec<Event>,
    ) {
        let settings = InstabilitySettings::builtin();
        let ships: Vec<(Vec3, f32)> = self
            .ships
            .values()
            .filter(|s| s.alive)
            .map(|s| (s.pos, cfg.tuning(s.ship_type).radius))
            .collect();
        let seed = ls_math::rng::mix64(
            cfg.seed
                ^ ls_math::rng::mix64(crate::instability::CAVE_IN_RNG_KEY ^ now.rotate_left(32)),
        );
        let mut rng = Pcg32::new(seed, crate::behavior::RNG_STREAM_MODULES);
        let Some(plan) =
            crate::instability::plan_cave_in(settings, &*terrain, self.up, pos, &mut rng, &ships)
        else {
            return;
        };
        let mut cmds = Vec::new();
        if let Some(h) = plan.hollow {
            cmds.push(Command::Edit(TerrainEdit::SubtractSphere {
                center: h.center,
                radius: h.radius,
            }));
        }
        for r in &plan.rubble {
            cmds.push(Command::Edit(TerrainEdit::AddSphere {
                center: r.center,
                radius: r.radius,
            }));
        }
        for &(p, v, r, scrap) in &plan.debris {
            cmds.push(Command::SpawnEntity(Box::new(EntitySpawn::debris(
                p, v, r, scrap,
            ))));
        }
        for s in self.ships.values().filter(|s| s.alive) {
            let near = plan.rubble.iter().find(|r| {
                s.pos.distance(r.center) - r.radius - cfg.tuning(s.ship_type).radius
                    < settings.damage_radius
            });
            if let Some(r) = near {
                let push = (s.pos - r.center).normalize_or_zero() * 4.0;
                cmds.push(Command::Damage {
                    target: s.player,
                    amount: settings.damage,
                    knockback: push,
                    source: DamageSource::ENVIRONMENT,
                    direct: false,
                });
            }
        }
        self.no_strain = true;
        self.apply_commands(cfg, terrain, cmds, events);
        self.no_strain = false;
        self.instability.add_rubble(settings, &plan.rubble);
    }
}

/// Hashes the mode state beyond the M2 Paint the Void fields, if any of it
/// differs from a fresh two-team Paint the Void state.
fn hash_mode_extras(h: &mut StateHasher, m: &ModeState) {
    use crate::mode::{ModeKind, PlayerMode};
    let default_players = m.players.values().all(|p| *p == PlayerMode::default());
    let plain = m.kind == ModeKind::PaintTheVoid
        && m.teams == 2
        && m.stage == 0
        && m.stage_end_tick == 0
        && m.coverage.team_cells[2..].iter().all(|&c| c == 0)
        && m.team_scores.iter().all(|&s| s == 0)
        && m.score_target == 0
        && default_players
        && m.objectives.is_empty()
        && m.vars.iter().all(|&v| v == 0)
        && m.flags == 0;
    if plain {
        return;
    }
    h.write_u8(0xa1);
    h.write_u8(m.kind.index());
    h.write_u8(m.teams);
    h.write_u8(m.stage);
    h.write_u64(m.stage_end_tick);
    h.write_u64(m.coverage.team_cells[2]);
    h.write_u64(m.coverage.team_cells[3]);
    for s in m.team_scores {
        h.write_u64(s as u64);
    }
    h.write_u32(m.score_target);
    h.write_u32(m.players.len() as u32);
    for (id, p) in &m.players {
        h.write_u16(id.0);
        h.write_u32(p.score as u32);
        h.write_u8(p.lives);
        h.write_u8(p.meter);
        h.write_u8(p.flags);
    }
    h.write_u32(m.objectives.len() as u32);
    for o in &m.objectives {
        h.write_u8(o.id);
        h.write_u8(o.kind);
        h.write_vec3(o.pos);
        h.write_vec3(o.vel);
        h.write_f32(o.radius);
        h.write_u16(o.holder.map_or(u16::MAX, |p| p.0));
        h.write_u64(o.timer_end);
        h.write_u8(o.team);
        h.write_u8(o.state);
        for v in o.vars {
            h.write_f32(v);
        }
    }
    for v in m.vars {
        h.write_u64(v as u64);
    }
    h.write_u16(m.flags);
}

fn hash_edit(h: &mut StateHasher, e: &TerrainEdit) {
    match *e {
        TerrainEdit::SubtractSphere { center, radius } => {
            h.write_u8(0);
            h.write_vec3(center);
            h.write_f32(radius);
        }
        TerrainEdit::AddSphere { center, radius } => {
            h.write_u8(1);
            h.write_vec3(center);
            h.write_f32(radius);
        }
        TerrainEdit::Paint {
            center,
            radius,
            team,
            color,
        } => {
            h.write_u8(2);
            h.write_vec3(center);
            h.write_f32(radius);
            h.write_u8(team);
            h.write_u8(color);
        }
        TerrainEdit::Surface {
            center,
            radius,
            states,
            until,
        } => {
            h.write_u8(3);
            h.write_vec3(center);
            h.write_f32(radius);
            h.write_u8(states);
            h.write_u32(until);
        }
    }
}

fn hash_ship(h: &mut StateHasher, s: &ShipState) {
    h.write_u16(s.player.0);
    h.write_u8(s.team);
    h.write_u8(s.ship_type.index());
    h.write_vec3(s.pos);
    h.write_vec3(s.vel);
    h.write_quat(s.orientation);
    h.write_vec3(s.ang_vel);
    h.write_f32(s.hull);
    h.write_f32(s.energy);
    h.write_f32(s.boost);
    h.write_f32(s.scrap);
    h.write_bool(s.alive);
    h.write_u64(s.respawn_tick);
    h.write_u32(s.weapons.len() as u32);
    for w in &s.weapons {
        h.write_u8(w.weapon.0);
        h.write_u32(w.ammo);
        h.write_u8(w.level);
        h.write_u32(w.shots);
    }
    h.write_u16(s.primary.map_or(u16::MAX, |w| w.0 as u16));
    h.write_u16(s.secondary.map_or(u16::MAX, |w| w.0 as u16));
    h.write_u64(s.primary_ready_tick);
    h.write_u64(s.secondary_ready_tick);
    h.write_bool(s.boosting);
    h.write_bool(s.in_contact);
    h.write_bool(s.horizon_assist);
    h.write_u16(s.last_attacker.map_or(u16::MAX, |p| p.0));
    h.write_u32(s.kills);
    h.write_u32(s.deaths);
    h.write_u16(s.gadget.map_or(u16::MAX, |g| g.0 as u16));
    h.write_bool(s.gadget_active);
    h.write_u64(s.gadget_ready_tick);
    match s.charge {
        None => h.write_u8(0),
        Some(c) => {
            h.write_u8(1 + (c.slot == WeaponSlot::Secondary) as u8);
            h.write_u8(c.weapon.0);
            h.write_u64(c.start_tick);
        }
    }
    h.write_u16(s.prev_buttons.0);
    h.write_u64(s.blind_until);
    h.write_u64(s.weapons_offline_until);
    h.write_u64(s.last_fired_tick.unwrap_or(u64::MAX));
    h.write_u8(s.next_ship_type.map_or(u8::MAX, |t| t.index()));
    h.write_u32(s.weapon_state.len() as u32);
    for (w, v) in &s.weapon_state {
        h.write_u8(w.0);
        h.write_u32(*v);
    }
    if s.pigment_until != 0 {
        h.write_u8(0x91);
        h.write_u64(s.pigment_until);
    }
}

fn hash_projectile(h: &mut StateHasher, p: &Projectile) {
    h.write_u32(p.id);
    h.write_u16(p.owner.0);
    h.write_u8(p.team);
    h.write_u8(p.weapon.0);
    h.write_vec3(p.pos);
    h.write_vec3(p.vel);
    h.write_u64(p.expires_tick);
    h.write_f32(p.damage);
    h.write_f32(p.splash_damage);
    h.write_f32(p.knockback);
    h.write_u64(p.spawn_tick);
    h.write_f32(p.radius);
    h.write_u16(p.visual.0);
    h.write_f32(p.charge);
    h.write_u8(p.generation);
    h.write_u8(p.bounces);
    h.write_u16(p.flags);
    h.write_u64(p.arm_tick);
    h.write_u64(p.fuse_tick);
    h.write_u16(p.target.map_or(u16::MAX, |t| t.0));
    for v in p.vars {
        h.write_f32(v);
    }
    h.write_u32(p.path.len() as u32);
    for v in &p.path {
        h.write_vec3(*v);
    }
    match p.passed {
        None => h.write_u8(u8::MAX),
        Some((kind, id)) => {
            h.write_u8(kind);
            h.write_u32(id);
        }
    }
}

fn hash_entity(h: &mut StateHasher, e: &Entity) {
    h.write_u32(e.id);
    h.write_u8(ReplicatedKind::from(e.kind) as u8);
    match e.source {
        Source::None => h.write_u8(0),
        Source::Weapon(w) => {
            h.write_u8(1);
            h.write_u8(w.0);
        }
        Source::Gadget(g) => {
            h.write_u8(2);
            h.write_u8(g.0);
        }
    }
    h.write_u16(e.visual.0);
    h.write_vec3(e.pos);
    h.write_vec3(e.vel);
    h.write_quat(e.rot);
    h.write_f32(e.radius);
    h.write_u16(e.owner.map_or(u16::MAX, |p| p.0));
    h.write_u8(e.team);
    h.write_u64(e.spawn_tick);
    h.write_u64(e.expires_tick);
    h.write_f32(e.hp);
    h.write_f32(e.scrap);
    h.write_bool(e.resting);
    h.write_bool(e.kinematic);
    for v in e.vars {
        h.write_f32(v);
    }
    h.write_bytes(&e.state);
}

/// Applies a weapon selection from input (`0` = keep).
fn select_weapon(ship: &mut ShipState, select: u8, weapons: &WeaponTable) {
    if select == 0 {
        return;
    }
    let w = WeaponId(select - 1);
    if (w.0 as usize) >= weapons.len() || ship.weapon(w).is_none() {
        return;
    }
    match weapons.get(w).slot {
        WeaponSlot::Primary => ship.primary = Some(w),
        WeaponSlot::Secondary => ship.secondary = Some(w),
    }
}

/// Draws scrap from piles (then debris) within reach: repairs hull first,
/// the rest goes to the scrap store. This is the only way hull is ever
/// restored. Debris emptied by scavenging is removed.
fn scavenge(
    ship: &mut ShipState,
    piles: &mut [ScrapPile],
    entities: &mut Vec<Entity>,
    t: &ShipTuning,
) {
    let hull_room = if t.hull_per_scrap > 0.0 {
        (t.hull_max - ship.hull).max(0.0) / t.hull_per_scrap
    } else {
        0.0
    };
    let store_room = (t.scrap_max - ship.scrap).max(0.0);
    let mut budget = (t.scavenge_rate * DT).min(hull_room + store_room);
    let r2 = t.scavenge_radius * t.scavenge_radius;
    let mut gained = 0.0f32;
    for pile in piles.iter_mut() {
        if budget <= 0.0 {
            break;
        }
        if pile.amount <= 0.0 || pile.pos.distance_squared(ship.pos) > r2 {
            continue;
        }
        let take = budget.min(pile.amount);
        pile.amount -= take;
        budget -= take;
        gained += take;
    }
    if budget > 0.0 {
        let mut emptied = Vec::new();
        for e in entities.iter_mut() {
            if budget <= 0.0 {
                break;
            }
            if e.kind != EntityKind::Debris
                || e.scrap <= 0.0
                || e.pos.distance_squared(ship.pos) > r2
            {
                continue;
            }
            let take = budget.min(e.scrap);
            e.scrap -= take;
            budget -= take;
            gained += take;
            if e.scrap <= 1e-4 {
                emptied.push(e.id);
            }
        }
        if !emptied.is_empty() {
            entities.retain(|e| !emptied.contains(&e.id));
        }
    }
    if gained <= 0.0 {
        return;
    }
    let to_hull = gained.min(hull_room);
    ship.hull = (ship.hull + to_hull * t.hull_per_scrap).min(t.hull_max);
    ship.scrap = (ship.scrap + (gained - to_hull)).min(t.scrap_max);
}

/// Stats of a paid shot.
#[derive(Debug, Clone, Copy)]
struct ShotStats {
    weapon: WeaponId,
    level: u8,
    shots: u32,
    damage: f32,
    splash_damage: f32,
    knockback: f32,
}

/// Rule set, cooldown and cost checks for the selected weapon in `slot`,
/// without paying.
fn can_fire(
    ship: &ShipState,
    slot: WeaponSlot,
    w: WeaponId,
    now: u64,
    overdrive: bool,
    cfg: &SimConfig,
) -> bool {
    if cfg.resolved.availability(w).is_banned() {
        return false;
    }
    let def = cfg.weapons.get(w);
    if def.slot != slot {
        return false;
    }
    let ready = match slot {
        WeaponSlot::Primary => ship.primary_ready_tick,
        WeaponSlot::Secondary => ship.secondary_ready_tick,
    };
    if now < ready {
        return false;
    }
    let Some(st) = ship.weapon(w) else {
        return false;
    };
    if cfg.resolved.free_fire(w) {
        return true;
    }
    let t = cfg.tuning(ship.ship_type);
    match def.cost {
        WeaponCost::Energy(c) => {
            ship.energy >= c
                || (overdrive && ship.hull - (c - ship.energy) * t.overdrive_hull_per_energy >= 1.0)
        }
        WeaponCost::Scrap(c) => ship.scrap >= c,
        WeaponCost::Count(n) => st.ammo >= n,
    }
}

/// Checks rules, cooldown and cost for the selected weapon in `slot`; pays the
/// cost and starts the cooldown if the shot goes off. The rule set is
/// enforced here too: a banned weapon never fires, even if it somehow ended
/// up in the inventory.
fn try_fire(
    ship: &mut ShipState,
    slot: WeaponSlot,
    now: u64,
    overdrive: bool,
    charge: f32,
    cfg: &SimConfig,
) -> Option<ShotStats> {
    let w = match slot {
        WeaponSlot::Primary => ship.primary?,
        WeaponSlot::Secondary => ship.secondary?,
    };
    if !can_fire(ship, slot, w, now, overdrive, cfg) {
        return None;
    }
    let def = cfg.weapons.get(w);
    let level = ship.weapon(w)?.level;
    let shots = ship.weapon(w)?.shots;
    ship.weapon_mut(w)?.shots = shots.wrapping_add(1);
    let t = cfg.tuning(ship.ship_type);
    match def.cost {
        _ if cfg.resolved.free_fire(w) => {}
        WeaponCost::Energy(c) => {
            if ship.energy >= c {
                ship.energy -= c;
            } else {
                // Overdrive gamble: drain the rest of the energy, pay the
                // deficit with hull at 2:1. Never self-destructs.
                let hull_cost = (c - ship.energy) * t.overdrive_hull_per_energy;
                ship.energy = 0.0;
                ship.hull -= hull_cost;
            }
        }
        WeaponCost::Scrap(c) => ship.scrap -= c,
        WeaponCost::Count(n) => ship.weapon_mut(w)?.ammo -= n,
    }
    let tuning = cfg.resolved.tuning(w);
    let interval = if tuning.fire_rate == 1.0 {
        def.fire_interval_ticks()
    } else {
        ls_math::Tick::ticks_for_secs(def.fire_interval_secs / tuning.fire_rate).max(1)
    };
    let next = now + interval;
    match slot {
        WeaponSlot::Primary => ship.primary_ready_tick = next,
        WeaponSlot::Secondary => ship.secondary_ready_tick = next,
    }
    let charge_mult = match def.charge {
        Some(c) if charge > 0.0 => 1.0 + (c.damage_mult - 1.0) * charge,
        _ => 1.0,
    };
    let mut mult = def.level_multiplier(level) * t.damage_mult;
    if tuning.damage != 1.0 {
        mult *= tuning.damage;
    }
    if charge_mult != 1.0 {
        mult *= charge_mult;
    }
    Some(ShotStats {
        weapon: w,
        level,
        shots,
        damage: def.damage * mult,
        splash_damage: def.splash.map_or(0.0, |s| s.damage * mult),
        // "Every hit applies knockback in proportion to its damage."
        knockback: def.knockback * mult,
    })
}

/// Applies a weapon pickup to a ship. Returns false (pickup stays) if it is not
/// allowed as a pickup or would change nothing.
fn collect_weapon(ship: &mut ShipState, w: WeaponId, ammo: u32, cfg: &SimConfig) -> bool {
    if !cfg.resolved.availability(w).as_pickup() {
        return false;
    }
    let def = cfg.weapons.get(w);
    let max_ammo = cfg.resolved.max_ammo(w);
    if let Some(st) = ship.weapon_mut(w) {
        return match def.cost {
            WeaponCost::Count(_) => {
                if st.ammo >= max_ammo {
                    false
                } else {
                    st.ammo = st.ammo.saturating_add(ammo.max(1)).min(max_ammo);
                    true
                }
            }
            _ => {
                if st.level >= def.max_level {
                    false
                } else {
                    st.level += 1;
                    true
                }
            }
        };
    }
    if def.slot == WeaponSlot::Secondary {
        let slots = cfg.tuning(ship.ship_type).secondary_slots as usize;
        if slots == 0 {
            return false;
        }
        let carried = ship
            .weapons
            .iter()
            .filter(|s| cfg.weapons.get(s.weapon).slot == WeaponSlot::Secondary)
            .count();
        if carried >= slots {
            // All slots full: the selected secondary is swapped out.
            match ship.secondary {
                Some(sel) => ship.remove_weapon(sel),
                None => return false,
            }
        }
    }
    let ammo = match def.cost {
        WeaponCost::Count(_) => ammo.max(1).min(max_ammo),
        _ => 0,
    };
    ship.add_weapon(WeaponState {
        weapon: w,
        ammo,
        level: 1,
        shots: 0,
    });
    match def.slot {
        WeaponSlot::Primary => {
            ship.primary.get_or_insert(w);
        }
        WeaponSlot::Secondary => {
            ship.secondary.get_or_insert(w);
        }
    }
    true
}

/// Upgrade pickup: +1 level of a carried weapon below its max level.
fn collect_upgrade(ship: &mut ShipState, w: WeaponId, cfg: &SimConfig) -> bool {
    if !cfg.resolved.availability(w).as_pickup() {
        return false;
    }
    let max = cfg.weapons.get(w).max_level;
    match ship.weapon_mut(w) {
        Some(st) if st.level < max => {
            st.level += 1;
            true
        }
        _ => false,
    }
}

/// Gadget pickup: fills an empty slot or replaces a different gadget.
fn collect_gadget(ship: &mut ShipState, g: GadgetId, cfg: &SimConfig) -> bool {
    if !cfg.resolved.gadget_availability(g).as_pickup() || ship.gadget == Some(g) {
        return false;
    }
    ship.gadget = Some(g);
    ship.gadget_active = false;
    true
}
