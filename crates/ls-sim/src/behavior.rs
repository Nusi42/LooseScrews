//! The hook context shared by weapon modules ([`crate::weapons`]) and gadget
//! modules ([`crate::gadgets`]).
//!
//! Hooks never mutate the world directly. They read it through
//! [`Ctx::view`] and queue [`Command`]s through the helper methods on
//! [`Ctx`]; the world applies the commands in order right after the hook
//! returns (before the next projectile or ship is processed). This keeps the
//! borrow structure simple and the outcome deterministic: the same state and
//! inputs always produce the same command list in the same order.
//!
//! Randomness: modules must not share an RNG stream (the order of draws
//! would couple unrelated weapons). [`Ctx::rng`] derives a private
//! [`Pcg32`] from the match seed, the tick and a module-chosen key (usually
//! the projectile id), so a module's draws never shift anyone else's.

use std::collections::BTreeMap;

use ls_math::rng::mix64;
use ls_math::terrain::{
    material_crater_scale, RayHit, SurfaceStates, TerrainEdit, TerrainQuery, PAINT_PLAIN,
};
use ls_math::{Pcg32, Vec3};
use serde::{Deserialize, Serialize};

use crate::entity::{Entity, EntitySpawn};
use crate::ruleset::ResolvedRules;
use crate::ship::{PlayerId, ShipState};
use crate::terrain::{paint_team_id, TerrainLayers};
use crate::visual::VisualTable;
use crate::world::{PickupSpawn, Projectile, ScrapPile};
use crate::{
    Event, GadgetTable, InputCmd, ShipTable, ShipTuning, WeaponDef, WeaponId, WeaponTable,
};

/// RNG stream of [`Ctx::rng`] ("SIM_MODS").
pub const RNG_STREAM_MODULES: u64 = 0x5349_4d5f_4d4f_4453;
/// Most commands one hook call may queue; the rest are dropped (keeps a
/// buggy module from flooding the edit log).
pub const MAX_COMMANDS_PER_HOOK: usize = 256;

/// Who dealt damage, for kill credit, friendly fire and shields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageSource {
    pub attacker: Option<PlayerId>,
    /// Attacker's team (friendly-fire scaling), or `u8::MAX` for none.
    pub team: u8,
    pub weapon: Option<WeaponId>,
}

impl DamageSource {
    /// Damage caused by projectile `p`.
    pub fn projectile(p: &Projectile) -> DamageSource {
        DamageSource {
            attacker: Some(p.owner),
            team: p.team,
            weapon: Some(p.weapon),
        }
    }

    /// Damage without an attacker (environment).
    pub const ENVIRONMENT: DamageSource = DamageSource {
        attacker: None,
        team: u8::MAX,
        weapon: None,
    };
}

/// Client-facing cues: things a client should show or play that are not in
/// the snapshot state (warnings, blinding, chimes). Carried by
/// [`Event::Cue`] and replicated as `ls_net::NetEvent::Cue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum CueKind {
    /// Polite Laser: "excuse me" warning to `target`.
    PoliteWarning = 0,
    /// `target` is blinded for `value` ticks (Flash Pod).
    Blinded = 1,
    /// A charge started (`value` = charge ticks to full).
    ChargeStart = 2,
    /// Tectonic Charge launched (level-wide rumble).
    TectonicLaunch = 3,
    /// Tectonic Charge stuck: the ground cracks (fuse `value` ticks).
    TectonicArmed = 4,
    /// Shockwave at `pos` with radius `value` (screen shake).
    Shockwave = 5,
    /// Instant beam from `source` to `pos` (Leech Arc).
    Beam = 6,
    /// `source` and `target` swapped places (Swap Harpoon).
    Swapped = 7,
    /// `target` rewound in time; a ghost stays at `pos` (Borrowed Time).
    Rewound = 8,
    /// Muzzle flash that reveals the shooter (Railspike).
    MuzzleFlash = 9,
    /// A gadget was switched on, off or used; `value` per [`gadget_cue`].
    Gadget = 10,
    /// Decoy snapped back to its owner.
    DecoyReturn = 11,
    /// An overheld charge started burning `source`'s hull (`value` = hull
    /// per second; Fusion Charger).
    Overcharge = 12,
    /// `source` switched its Paint Thrower colour (`value` =
    /// `ls_math::terrain::PAINT_*`: 1 red, 2 blue, 3 yellow).
    PaintColor = 13,
    /// Instability: the rock at `pos` comes down in `value` ticks (rumble,
    /// dust trickling, glowing cracks).
    CaveInWarning = 14,
    /// Pigment: `source` took a pigment pickup at `pos` (`value` 1 for a
    /// dropped splash, 0 for a regular pickup).
    Pigment = 15,
    /// A comedy event's 3 s announcement (protocol 6): `value` =
    /// `EventKind` index, `target` = the player it names (255 none), `pos`
    /// = the in-world tell (paw shadow, drone entry, ...; the target's
    /// ship or zero when the event gives none).
    EventAnnounce = 16,
    /// The Architect's stroke telegraph: `source` = the architect, `pos` =
    /// centre, `value` = stroke kind << 8 | radius in u (clamped to 255).
    StrokeTelegraph = 17,
    /// A comedy event's outcome (M4 B2): `value` = `EventKind` index << 8
    /// | outcome byte (per event, see `docs/architecture/netcode.md`:
    /// Tax drone touch 1-5, delivered pizza 1, survey rating 1-5 or 0,
    /// mimic bite 2, a drone or mimic shot down 0xF0), `source` = the
    /// player who caused it (the shooter), `target` = the player it
    /// concerns, `pos` = where.
    EventOutcome = 18,
    /// Free for modules; `value` is module-defined.
    Custom = 255,
}

impl CueKind {
    pub fn from_u8(v: u8) -> CueKind {
        use CueKind::*;
        match v {
            0 => PoliteWarning,
            1 => Blinded,
            2 => ChargeStart,
            3 => TectonicLaunch,
            4 => TectonicArmed,
            5 => Shockwave,
            6 => Beam,
            7 => Swapped,
            8 => Rewound,
            9 => MuzzleFlash,
            10 => Gadget,
            11 => DecoyReturn,
            12 => Overcharge,
            13 => PaintColor,
            14 => CaveInWarning,
            15 => Pigment,
            16 => EventAnnounce,
            17 => StrokeTelegraph,
            18 => EventOutcome,
            _ => Custom,
        }
    }
}

/// `value`s of a [`CueKind::Gadget`] cue.
pub mod gadget_cue {
    /// Values below this are "switched on" with the gadget id.
    pub const ON_MAX: u16 = 253;
    /// A toggle gadget ran out of energy and switched off (no cooldown).
    pub const EMPTY: u16 = 254;
    /// A toggle gadget was switched off by its key (the cooldown starts).
    pub const OFF: u16 = 255;
    /// `USED | gadget id`: an instant gadget was used (the cooldown starts).
    pub const USED: u16 = 0x100;

    /// Whether a cue with this value starts the gadget's cooldown.
    pub fn starts_cooldown(value: u16) -> bool {
        value == OFF || value & USED != 0
    }
}

/// A client cue (see [`CueKind`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    pub kind: CueKind,
    /// Player that caused it (shooter), if any.
    pub source: Option<PlayerId>,
    /// Player it is aimed at, if any.
    pub target: Option<PlayerId>,
    pub pos: Vec3,
    /// Kind-specific number (ticks, radius, id).
    pub value: u16,
}

/// A queued world change. Applied in order by the world after each hook.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Damage (and knockback) to a ship; friendly fire and shields apply.
    Damage {
        target: PlayerId,
        amount: f32,
        knockback: Vec3,
        source: DamageSource,
        /// Direct hit (as opposed to splash or area damage).
        direct: bool,
    },
    /// Damage falling off linearly to 0 at `radius`, knockback outward
    /// (scaled by the same falloff). Also pushes debris.
    Splash {
        center: Vec3,
        radius: f32,
        damage: f32,
        knockback: f32,
        source: DamageSource,
        hits_owner: bool,
        /// Ship already hit directly (not hit again).
        exclude: Option<PlayerId>,
        /// Push direction for a ship exactly at the centre (the projectile's
        /// direction for impacts; zero = no push).
        dir: Vec3,
    },
    /// Knockback only, falling off to 0 at `radius` (ships and debris).
    Shockwave {
        center: Vec3,
        radius: f32,
        strength: f32,
    },
    /// A terrain edit (crater, channel, growth, paint).
    Edit(TerrainEdit),
    /// A new projectile (its id is assigned by the world). It moves from
    /// the next tick on.
    SpawnProjectile(Box<Projectile>),
    /// A new entity (id assigned by the world; debris past the cap removes
    /// the oldest debris).
    SpawnEntity(Box<EntitySpawn>),
    /// Adds `dv` to a ship's velocity.
    ImpulseShip { target: PlayerId, dv: Vec3 },
    /// Adds `dv / mass` to an entity's velocity (wakes resting debris).
    ImpulseEntity { id: u32, dv: Vec3 },
    /// Moves a scrap pile, or the content of a pickup spawn point, whose
    /// entity id is `id` by `delta` (the spawn point itself stays; Magnet
    /// Sneeze).
    MovePickup { id: u32, delta: Vec3 },
    /// Damages an entity (debris, decoy); `hp <= 0` removes it.
    DamageEntity {
        id: u32,
        amount: f32,
        source: DamageSource,
    },
    /// Removes an entity.
    RemoveEntity { id: u32 },
    /// Adds `dv` to another projectile's velocity (Magnet Sneeze pulling
    /// mines in); a stuck projectile is pulled loose. A hook changes its own
    /// projectile through `&mut Projectile` instead.
    ImpulseProjectile { id: u32, dv: Vec3 },
    /// Removes another projectile (its own: `Projectile::remove`).
    RemoveProjectile { id: u32 },
    /// Moves energy from one ship to another (or nowhere).
    DrainEnergy {
        from: PlayerId,
        to: Option<PlayerId>,
        amount: f32,
    },
    /// Blinds a ship for `ticks` (and cues the client).
    Blind { target: PlayerId, ticks: u64 },
    /// Swaps positions and velocities of two ships.
    Swap { a: PlayerId, b: PlayerId },
    /// Restores a ship's position, velocity and hull from `ticks` ago.
    Rewind { player: PlayerId, ticks: u64 },
    /// Weapons cannot fire for `ticks`.
    DisableWeapons { player: PlayerId, ticks: u64 },
    /// Changes a ship's hull directly (overcharge burns, repairs); never
    /// kills below 1 hull unless `can_kill`.
    AdjustHull {
        target: PlayerId,
        delta: f32,
        can_kill: bool,
    },
    /// Sets the module-owned state of a weapon the ship carries
    /// (`ShipState::weapon_state`; 0 removes it): Paint Thrower colour,
    /// Leech Arc lock. Ignored if the ship does not carry the weapon.
    SetWeaponState {
        ship: PlayerId,
        weapon: WeaponId,
        value: u32,
    },
    /// Emits an event as is (impacts, cues).
    Emit(Event),
    /// A terrain edit applied at the start of tick `tick` instead of now
    /// (queued by [`Ctx::later`]; spreads big edit bursts over ticks).
    EditLater { tick: u64, edit: TerrainEdit },
    /// Adds a dynamic gravity well (Paperweight, Architect stroke; see
    /// [`crate::gravity`]).
    AddWell(crate::gravity::DynamicWell),
    /// Removes the dynamic wells with this key.
    RemoveWell { key: u32 },
    /// Sets the pose, velocity and module state of an entity (comedy event
    /// props, which the world never moves; M4 B2). Ignored for unknown ids.
    SetEntity {
        id: u32,
        pos: Vec3,
        vel: Vec3,
        rot: ls_math::Quat,
        vars: [f32; 4],
        state: [u8; 4],
    },
    /// A scrap pile of `amount` at `pos` (a drone bursting; M4 B2).
    SpawnScrap { pos: Vec3, amount: f32 },
}

/// Read-only world view for hooks.
#[derive(Clone, Copy)]
pub struct View<'a> {
    pub now: u64,
    pub seed: u64,
    pub weapons: &'a WeaponTable,
    pub gadgets: &'a GadgetTable,
    pub ship_types: &'a ShipTable,
    pub rules: &'a ResolvedRules,
    pub visuals: &'a VisualTable,
    /// The level's Instability slider (0-100), see `MatchSetup::instability`.
    pub instability: u8,
    /// The running comedy event's effects this tick (crater scale, ...).
    pub effects: crate::director::EventEffects,
    pub ships: &'a BTreeMap<PlayerId, ShipState>,
    /// Projectiles in flight. While projectiles are being stepped this holds
    /// only the ones spawned during that stage; use
    /// [`View::all_projectiles`] to see every projectile.
    pub projectiles: &'a [Projectile],
    /// While projectiles are stepped: the ones not stepped yet this tick
    /// (in reverse order) ...
    pub projectiles_stepping: &'a [Projectile],
    /// ... and the ones already stepped. Both empty in other stages.
    pub projectiles_stepped: &'a [Projectile],
    pub entities: &'a [Entity],
    /// Pickup spawn points (content at [`PickupSpawn::content_pos`]).
    pub pickups: &'a [PickupSpawn],
    /// Scrap piles, scrap pickups and wrecks.
    pub scrap_piles: &'a [ScrapPile],
    pub terrain: &'a dyn TerrainQuery,
    /// Surface states and rock materials (see [`TerrainLayers`]).
    pub layers: &'a dyn TerrainLayers,
    /// This tick's input per player.
    pub inputs: &'a BTreeMap<PlayerId, InputCmd>,
    /// The level's flight network for event drones (M4 B2; empty on levels
    /// without one, see [`crate::events::drone::NavGraph`]).
    pub nav: &'a crate::events::drone::NavGraph,
    pub(crate) trails: &'a BTreeMap<PlayerId, crate::world::Trail>,
}

impl<'a> View<'a> {
    /// Every projectile in flight in any stage (except the one whose hook is
    /// running), in no particular order.
    pub fn all_projectiles(&self) -> impl Iterator<Item = &'a Projectile> + 'a {
        self.projectiles_stepped
            .iter()
            .chain(self.projectiles_stepping.iter().rev())
            .chain(self.projectiles.iter())
            .filter(|p| !p.is_removed())
    }
}

impl std::fmt::Debug for View<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("View").field("now", &self.now).finish()
    }
}

/// Hook context: the world view plus the command queue. See the module docs.
pub struct Ctx<'a> {
    pub view: View<'a>,
    out: Vec<Command>,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(view: View<'a>) -> Ctx<'a> {
        Ctx {
            view,
            out: Vec::new(),
        }
    }

    pub(crate) fn into_commands(self) -> Vec<Command> {
        self.out
    }

    /// Current tick.
    pub fn now(&self) -> u64 {
        self.view.now
    }

    /// Queues a command (dropped past [`MAX_COMMANDS_PER_HOOK`]).
    pub fn push(&mut self, c: Command) {
        if self.out.len() < MAX_COMMANDS_PER_HOOK {
            self.out.push(c);
        }
    }

    /// Runs `f` and defers the terrain edits it queues by `delay` ticks
    /// (at least 1): they are applied at the start of tick `now + delay`,
    /// in queue order, before respawns (`Command::EditLater`). Other
    /// commands `f` queues apply as usual. Use it to spread a burst of
    /// edits (Tectonic Charge cracks) over several ticks.
    pub fn later(&mut self, delay: u64, f: impl FnOnce(&mut Ctx<'a>)) {
        let start = self.out.len();
        f(self);
        let tick = self.now() + delay.max(1);
        for c in &mut self.out[start..] {
            if let Command::Edit(edit) = c {
                *c = Command::EditLater { tick, edit: *edit };
            }
        }
    }

    /// Commands queued so far (tests).
    pub fn commands(&self) -> &[Command] {
        &self.out
    }

    // ----- reading -----------------------------------------------------------

    pub fn ship(&self, id: PlayerId) -> Option<&'a ShipState> {
        self.view.ships.get(&id)
    }

    /// Tuning of a ship's type.
    pub fn tuning_of(&self, ship: &ShipState) -> &'a ShipTuning {
        self.view.ship_types.get(ship.ship_type)
    }

    pub fn def(&self, w: WeaponId) -> &'a WeaponDef {
        self.view.weapons.get(w)
    }

    /// This tick's input of a player (guided missiles read the owner's look).
    pub fn input(&self, player: PlayerId) -> Option<&'a InputCmd> {
        self.view.inputs.get(&player)
    }

    /// Living ships within `radius` of `pos` (sphere centres), in id order.
    pub fn ships_within(&self, pos: Vec3, radius: f32) -> Vec<&'a ShipState> {
        self.view
            .ships
            .values()
            .filter(|s| s.alive && s.pos.distance_squared(pos) <= radius * radius)
            .collect()
    }

    /// Nearest living ship to `pos` within `range` accepted by `filter`
    /// (ties go to the lower player id).
    pub fn nearest_ship(
        &self,
        pos: Vec3,
        range: f32,
        filter: impl Fn(&ShipState) -> bool,
    ) -> Option<&'a ShipState> {
        let mut best: Option<(&ShipState, f32)> = None;
        for s in self.view.ships.values() {
            if !s.alive || !filter(s) {
                continue;
            }
            let d = s.pos.distance_squared(pos);
            if d <= range * range && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((s, d));
            }
        }
        best.map(|(s, _)| s)
    }

    /// Nearest living enemy of `team` inside a cone (`half_angle_deg` around
    /// `dir`) within `range` with a clear line of sight: seekers, lock-on.
    pub fn target_in_cone(
        &self,
        origin: Vec3,
        dir: Vec3,
        range: f32,
        half_angle_deg: f32,
        team: u8,
    ) -> Option<&'a ShipState> {
        let dir = dir.try_normalize()?;
        let cos = ls_math::det::cos(half_angle_deg.to_radians());
        let mut best: Option<(&ShipState, f32)> = None;
        for s in self.view.ships.values() {
            if !s.alive || s.team == team {
                continue;
            }
            let to = s.pos - origin;
            let d = to.length();
            if d > range || d <= 1e-4 || to.dot(dir) < cos * d {
                continue;
            }
            if self.view.terrain.raycast(origin, to, d).is_some() {
                continue;
            }
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((s, d));
            }
        }
        best.map(|(s, _)| s)
    }

    /// Terrain raycast (see `TerrainQuery::raycast`).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<RayHit> {
        self.view.terrain.raycast(origin, dir, max)
    }

    /// Thickness of rock entered at `point` along `dir`: the distance to the
    /// first open point (sampled every 0.1 u), or `None` if it is thicker
    /// than `max` (Railspike pierce test, Polite Laser holes).
    pub fn thickness(&self, point: Vec3, dir: Vec3, max: f32) -> Option<f32> {
        let dir = dir.try_normalize()?;
        let step = 0.1f32;
        let mut t = step;
        while t <= max + 1e-4 {
            if self.view.terrain.sdf(point + dir * t) > 0.0 {
                return Some(t);
            }
            t += step;
        }
        None
    }

    /// Surface states (hot, charged, glow active now, and scorch) within
    /// `radius` of `center` (none on terrains without surface states).
    pub fn surface_states_near(&self, center: Vec3, radius: f32) -> SurfaceStates {
        self.view
            .layers
            .surface_states_near(center, radius, self.view.now)
    }

    /// Rock material id at `p` (`ls_math::terrain::MATERIAL_*`; ordinary
    /// rock on terrains without materials).
    pub fn material_at(&self, p: Vec3) -> u8 {
        self.view.layers.material_at(p)
    }

    /// Every projectile in flight (see [`View::all_projectiles`]).
    pub fn projectiles(&self) -> impl Iterator<Item = &'a Projectile> + 'a {
        self.view.all_projectiles()
    }

    /// Position, velocity and hull of a ship `ticks_ago` (at most 3 s; the
    /// oldest record if the ship has less history).
    pub fn trail(&self, player: PlayerId, ticks_ago: u64) -> Option<(Vec3, Vec3, f32)> {
        self.view.trails.get(&player)?.get(ticks_ago)
    }

    /// A private RNG for `key` (e.g. a projectile id) at this tick; see the
    /// module docs.
    pub fn rng(&self, key: u64) -> Pcg32 {
        let seed = mix64(self.view.seed ^ mix64(key ^ self.view.now.rotate_left(32)));
        Pcg32::new(seed, RNG_STREAM_MODULES)
    }

    // ----- writing -----------------------------------------------------------

    /// Damage with explicit knockback.
    pub fn damage(
        &mut self,
        target: PlayerId,
        amount: f32,
        knockback: Vec3,
        source: DamageSource,
        direct: bool,
    ) {
        self.push(Command::Damage {
            target,
            amount,
            knockback,
            source,
            direct,
        });
    }

    /// Damage with knockback proportional to it (`kb_per_damage` u/s per
    /// point along `dir`): "every hit applies knockback in proportion to
    /// its damage".
    pub fn damage_proportional(
        &mut self,
        target: PlayerId,
        amount: f32,
        dir: Vec3,
        kb_per_damage: f32,
        source: DamageSource,
        direct: bool,
    ) {
        let kb = dir.normalize_or_zero() * (amount * kb_per_damage);
        self.damage(target, amount, kb, source, direct);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn splash(
        &mut self,
        center: Vec3,
        radius: f32,
        damage: f32,
        knockback: f32,
        source: DamageSource,
        hits_owner: bool,
        exclude: Option<PlayerId>,
    ) {
        self.push(Command::Splash {
            center,
            radius,
            damage,
            knockback,
            source,
            hits_owner,
            exclude,
            dir: Vec3::ZERO,
        });
    }

    pub fn shockwave(&mut self, center: Vec3, radius: f32, strength: f32) {
        self.push(Command::Shockwave {
            center,
            radius,
            strength,
        });
    }

    pub fn edit(&mut self, e: TerrainEdit) {
        self.push(Command::Edit(e));
    }

    /// Subtracts a sphere (crater, pit, hole) scaled by the rock strength at
    /// `center` (weak rock 1.4x, hard rock 0.6x; see `contracts.md`) and by
    /// the level's Instability (`1 + 0.4 * instability / 100`, M4; exactly 1
    /// at 0). The scaled edit is what gets applied, logged and sent.
    pub fn crater(&mut self, center: Vec3, radius: f32) {
        let scale = material_crater_scale(self.material_at(center));
        let radius = radius * scale;
        let radius = if self.view.instability == 0 {
            radius
        } else {
            radius
                * crate::instability::InstabilitySettings::builtin()
                    .crater_scale(self.view.instability)
        };
        // The running comedy event's crater scale (Everything is cake).
        let event = self.view.effects.crater_scale;
        self.crater_exact(center, radius * event);
    }

    /// [`Ctx::crater`] scaled by the rock strength only, not by Instability
    /// or events: the Tectonic Charge's cracks, whose reach Instability
    /// already sets.
    pub fn crater_material(&mut self, center: Vec3, radius: f32) {
        let scale = material_crater_scale(self.material_at(center));
        self.crater_exact(center, radius * scale);
    }

    /// Subtracts a sphere of exactly `radius`, whatever the rock (Tectonic
    /// Charge collapse zones, scripted holes).
    pub fn crater_exact(&mut self, center: Vec3, radius: f32) {
        if radius > 0.0 {
            self.edit(TerrainEdit::SubtractSphere { center, radius });
        }
    }

    /// Adds a sphere of rock (Seed Grenade growth).
    pub fn grow(&mut self, center: Vec3, radius: f32) {
        if radius > 0.0 {
            self.edit(TerrainEdit::AddSphere { center, radius });
        }
    }

    /// Paints in a simulation team's colour, plain (no surface effect).
    pub fn paint(&mut self, center: Vec3, radius: f32, team: u8) {
        self.paint_color(center, radius, team, PAINT_PLAIN);
    }

    /// Paints in a simulation team's colour with an effect colour
    /// (`ls_math::terrain::PAINT_*`: red bouncy, blue slippery, yellow
    /// sticky; Paint Thrower).
    pub fn paint_color(&mut self, center: Vec3, radius: f32, team: u8, color: u8) {
        if radius > 0.0 {
            self.edit(TerrainEdit::Paint {
                center,
                radius,
                team: paint_team_id(team),
                color,
            });
        }
    }

    /// Scorches the surface within `radius` of `center` for good
    /// ([`SurfaceStates::SCORCH`]), hot for `hot_secs` as well (0: no heat).
    /// Skips the edit when the spot is already scorched (sustained fire on
    /// one spot does not flood the edit log). Scorch trails and marks use
    /// this instead of long-lived `HOT` edits.
    pub fn scorch(&mut self, center: Vec3, radius: f32, hot_secs: f32) {
        if radius <= 0.0
            || self
                .surface_states_near(center, radius * 0.5)
                .contains(SurfaceStates::SCORCH)
        {
            return;
        }
        let hot = ls_math::Tick::ticks_for_secs(hot_secs.max(0.0)).min(u32::MAX as u64) as u32;
        self.edit(TerrainEdit::scorch(center, radius, self.view.now, hot));
    }

    /// See [`Command::SetWeaponState`].
    pub fn set_weapon_state(&mut self, ship: PlayerId, weapon: WeaponId, value: u32) {
        self.push(Command::SetWeaponState {
            ship,
            weapon,
            value,
        });
    }

    /// A channel of craters from `a` to `b`, one every `spacing` u (at most 64).
    pub fn channel(&mut self, a: Vec3, b: Vec3, radius: f32, spacing: f32) {
        let len = a.distance(b);
        let n = if spacing > 0.0 {
            (ls_math::det::ceil(len / spacing) as usize).clamp(1, 64)
        } else {
            1
        };
        for i in 0..=n {
            self.crater(a.lerp(b, i as f32 / n as f32), radius);
        }
    }

    pub fn spawn_projectile(&mut self, p: Projectile) {
        self.push(Command::SpawnProjectile(Box::new(p)));
    }

    pub fn spawn_entity(&mut self, e: EntitySpawn) {
        self.push(Command::SpawnEntity(Box::new(e)));
    }

    /// A debris chunk (see [`EntitySpawn::debris`]).
    pub fn spawn_debris(&mut self, pos: Vec3, vel: Vec3, radius: f32, scrap: f32) {
        self.spawn_entity(EntitySpawn::debris(pos, vel, radius, scrap));
    }

    pub fn impulse_ship(&mut self, target: PlayerId, dv: Vec3) {
        self.push(Command::ImpulseShip { target, dv });
    }

    pub fn impulse_entity(&mut self, id: u32, dv: Vec3) {
        self.push(Command::ImpulseEntity { id, dv });
    }

    /// See [`Command::MovePickup`].
    pub fn move_pickup(&mut self, id: u32, delta: Vec3) {
        self.push(Command::MovePickup { id, delta });
    }

    /// See [`Command::ImpulseProjectile`].
    pub fn impulse_projectile(&mut self, id: u32, dv: Vec3) {
        self.push(Command::ImpulseProjectile { id, dv });
    }

    /// See [`Command::RemoveProjectile`].
    pub fn remove_projectile(&mut self, id: u32) {
        self.push(Command::RemoveProjectile { id });
    }

    pub fn drain_energy(&mut self, from: PlayerId, to: Option<PlayerId>, amount: f32) {
        self.push(Command::DrainEnergy { from, to, amount });
    }

    pub fn blind(&mut self, target: PlayerId, ticks: u64) {
        self.push(Command::Blind { target, ticks });
    }

    pub fn swap(&mut self, a: PlayerId, b: PlayerId) {
        self.push(Command::Swap { a, b });
    }

    pub fn rewind(&mut self, player: PlayerId, ticks: u64) {
        self.push(Command::Rewind { player, ticks });
    }

    pub fn disable_weapons(&mut self, player: PlayerId, ticks: u64) {
        self.push(Command::DisableWeapons { player, ticks });
    }

    pub fn adjust_hull(&mut self, target: PlayerId, delta: f32, can_kill: bool) {
        self.push(Command::AdjustHull {
            target,
            delta,
            can_kill,
        });
    }

    /// Adds a dynamic gravity well (see [`Command::AddWell`]).
    pub fn add_well(&mut self, well: crate::gravity::DynamicWell) {
        self.push(Command::AddWell(well));
    }

    /// Removes the dynamic wells with `key` (see [`Command::RemoveWell`]).
    pub fn remove_well(&mut self, key: u32) {
        self.push(Command::RemoveWell { key });
    }

    /// Sets an event prop's pose and state (see [`Command::SetEntity`]).
    pub fn set_entity(&mut self, e: &Entity) {
        self.push(Command::SetEntity {
            id: e.id,
            pos: e.pos,
            vel: e.vel,
            rot: e.rot,
            vars: e.vars,
            state: e.state,
        });
    }

    /// A scrap pile (see [`Command::SpawnScrap`]); nothing for `amount <= 0`.
    pub fn spawn_scrap(&mut self, pos: Vec3, amount: f32) {
        if amount > 0.0 {
            self.push(Command::SpawnScrap { pos, amount });
        }
    }

    pub fn cue(&mut self, cue: Cue) {
        self.push(Command::Emit(Event::Cue(cue)));
    }

    pub fn emit(&mut self, e: Event) {
        self.push(Command::Emit(e));
    }

    /// The default impact of projectile `p` at `point` (the M1 behaviour of
    /// every weapon): `Impact` event, direct damage and knockback to
    /// `target`, splash from the weapon data, and, on terrain, the data's
    /// crater and paint edits.
    pub fn detonate(
        &mut self,
        p: &Projectile,
        point: Vec3,
        target: Option<PlayerId>,
        terrain: bool,
    ) {
        let def = self.def(p.weapon);
        let dir = p.vel.normalize_or_zero();
        let source = DamageSource::projectile(p);
        self.emit(Event::Impact {
            projectile: p.id,
            weapon: p.weapon,
            point,
            target,
        });
        if let Some(t) = target {
            self.damage(t, p.damage, dir * p.knockback, source, true);
        }
        if let Some(splash) = def.splash {
            self.push(Command::Splash {
                center: point,
                radius: splash.radius,
                damage: p.splash_damage,
                knockback: p.knockback,
                source,
                hits_owner: splash.hits_owner,
                exclude: target,
                dir,
            });
        }
        if terrain {
            self.crater(point, def.terrain.crater_radius);
            self.paint(point, def.terrain.paint_radius, p.team);
        }
    }
}

impl std::fmt::Debug for Ctx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx")
            .field("now", &self.view.now)
            .field("queued", &self.out.len())
            .finish()
    }
}
