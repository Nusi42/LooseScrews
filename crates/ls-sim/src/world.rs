//! The deterministic match simulation: ships, projectiles, pickups, scrap and
//! the Paint the Void mode, advanced one 60 Hz tick at a time by [`World::step`].

use std::collections::BTreeMap;
use std::sync::Arc;

use ls_math::terrain::TerrainEdit;
use ls_math::{Pcg32, Quat, Vec3, DT};
use serde::{Deserialize, Serialize};

use crate::hash::StateHasher;
use crate::mode::{MatchPhase, PaintTheVoid, SCORE_INTERVAL_TICKS};
use crate::ruleset::{PickupKind, ResolvedRules};
use crate::ship::{simulate_movement, PlayerId, ShipState, WeaponState};
use crate::terrain::{paint_team_id, segment_sphere, SimTerrain};
use crate::{
    Buttons, InputCmd, RuleSet, ShipTuning, ShipType, SimError, WeaponCost, WeaponId, WeaponSlot,
    WeaponTable,
};

/// Distance (added to the ship radius) within which a ship collects a weapon pickup.
pub const PICKUP_RADIUS: f32 = 1.0;
/// Maximum number of wreck scrap piles; the oldest wreck is removed past this.
pub const MAX_WRECKS: usize = 64;
/// RNG stream for pickup rolls (see `docs/architecture/contracts.md`, RNG streams).
pub const RNG_STREAM_PICKUPS: u64 = 0x5349_4d5f_5049_434b; // "SIM_PICK"
/// Number of teams in Paint the Void.
pub const TEAMS: u8 = 2;

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
}

/// Everything needed to start a match.
#[derive(Debug, Clone)]
pub struct WorldConfig {
    /// Match seed (pickup rolls). Usually derived from the world seed.
    pub seed: u64,
    pub weapons: WeaponTable,
    pub rules: RuleSet,
    /// Tuning for the Tinker (the only ship type in M1).
    pub tuning: ShipTuning,
    pub setup: MatchSetup,
}

impl WorldConfig {
    /// Built-in M1 weapons and the default Paint the Void rule set.
    pub fn paint_the_void(seed: u64, setup: MatchSetup) -> WorldConfig {
        WorldConfig {
            seed,
            weapons: WeaponTable::builtin(),
            rules: RuleSet::paint_the_void(),
            tuning: ShipTuning::tinker(),
            setup,
        }
    }
}

/// Immutable per-match configuration shared by clones of a world.
#[derive(Debug)]
struct SimConfig {
    weapons: WeaponTable,
    rules: RuleSet,
    resolved: ResolvedRules,
    tuning: ShipTuning,
    setup: MatchSetup,
}

/// A projectile in flight. Its velocity is the muzzle velocity only; it never
/// inherits the shooter's velocity.
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
    /// Direct-hit damage (level and ship multipliers applied).
    pub damage: f32,
    /// Splash damage at the center (level and ship multipliers applied).
    pub splash_damage: f32,
    /// Knockback impulse (u/s) of a direct hit, scaled like damage (level and
    /// ship multipliers applied); splash knockback scales it by the falloff.
    pub knockback: f32,
}

/// Scavengeable scrap: wrecks of destroyed ships and scrap pickups.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrapPile {
    pub id: u32,
    pub pos: Vec3,
    pub amount: f32,
    /// Pickup spawn index if this pile is a scrap pickup (respawns when empty).
    pub spawn: Option<u16>,
}

/// A pickup spawn point and what it currently holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PickupSpawn {
    pub pos: Vec3,
    /// Pickup table entry index and content, or `None` while respawning.
    pub content: Option<(u16, PickupKind)>,
    pub respawn_tick: u64,
}

/// Things that happened during a tick, for clients, the server and tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    Fired {
        player: PlayerId,
        weapon: WeaponId,
        projectile: u32,
    },
    /// A projectile hit a ship or terrain (or exploded).
    Impact {
        projectile: u32,
        weapon: WeaponId,
        point: Vec3,
        target: Option<PlayerId>,
    },
    Damaged {
        target: PlayerId,
        attacker: PlayerId,
        weapon: WeaponId,
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
    /// A terrain edit (crater or paint), already applied to the terrain.
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
        team_cells: [u64; 2],
        surface_cells: u64,
    },
    MatchEnded {
        winner: Option<u8>,
        team_cells: [u64; 2],
    },
}

/// The simulation state of one match. Cloning is cheap enough for client-side
/// rollback (configuration is shared).
#[derive(Debug, Clone)]
pub struct World {
    config: Arc<SimConfig>,
    tick: u64,
    ships: BTreeMap<PlayerId, ShipState>,
    projectiles: Vec<Projectile>,
    scrap: Vec<ScrapPile>,
    pickups: Vec<PickupSpawn>,
    mode: PaintTheVoid,
    rng: Pcg32,
    next_player: u16,
    next_projectile: u32,
    next_scrap: u32,
    spawn_cursor: [u32; 2],
}

impl World {
    /// Validates the configuration and starts a match at tick 0. Pickups are
    /// rolled immediately.
    pub fn new(config: WorldConfig) -> Result<World, SimError> {
        let resolved = config.rules.resolve(&config.weapons)?;
        for s in &config.setup.spawns {
            if s.team >= TEAMS {
                return Err(SimError::InvalidTeam(s.team));
            }
        }
        if config.setup.pickup_points.len() > u16::MAX as usize {
            return Err(SimError::InvalidRuleSet("too many pickup points".into()));
        }
        for team in 0..TEAMS {
            if !config.setup.spawns.iter().any(|s| s.team == team) {
                return Err(SimError::InvalidRuleSet(format!(
                    "team {team} has no spawn point"
                )));
            }
        }
        // Reject loadouts the ship cannot carry instead of trimming them at respawn.
        let slots = config.tuning.secondary_slots as usize;
        if resolved.loadout_secondaries.len() > slots {
            return Err(SimError::InvalidRuleSet(format!(
                "starting loadout has {} secondaries, the ship has {slots} slots",
                resolved.loadout_secondaries.len()
            )));
        }
        for &(w, count) in &resolved.loadout_secondaries {
            let def = config.weapons.get(w);
            if count > def.max_ammo {
                return Err(SimError::InvalidRuleSet(format!(
                    "starting loadout gives {count} {}, max_ammo is {}",
                    def.id, def.max_ammo
                )));
            }
        }
        let mode = PaintTheVoid::new(resolved.match_ticks);
        let pickups = config
            .setup
            .pickup_points
            .iter()
            .map(|&pos| PickupSpawn {
                pos,
                content: None,
                respawn_tick: 0,
            })
            .collect();
        let mut world = World {
            rng: Pcg32::from_seed(config.seed, RNG_STREAM_PICKUPS),
            config: Arc::new(SimConfig {
                weapons: config.weapons,
                rules: config.rules,
                resolved,
                tuning: config.tuning,
                setup: config.setup,
            }),
            tick: 0,
            ships: BTreeMap::new(),
            projectiles: Vec::new(),
            scrap: Vec::new(),
            pickups,
            mode,
            next_player: 0,
            next_projectile: 0,
            next_scrap: 0,
            spawn_cursor: [0; 2],
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
    pub fn rules(&self) -> &RuleSet {
        &self.config.rules
    }
    pub fn resolved_rules(&self) -> &ResolvedRules {
        &self.config.resolved
    }
    pub fn tuning(&self) -> &ShipTuning {
        &self.config.tuning
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
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
    }
    pub fn scrap_piles(&self) -> &[ScrapPile] {
        &self.scrap
    }
    pub fn pickups(&self) -> &[PickupSpawn] {
        &self.pickups
    }
    pub fn mode(&self) -> &PaintTheVoid {
        &self.mode
    }
    /// Mutable mode state (admin commands such as ending a match early,
    /// host tests). Bypasses the rules.
    pub fn mode_mut(&mut self) -> &mut PaintTheVoid {
        &mut self.mode
    }

    /// Adds a scrap pile (level junk placed by the generator, tests).
    pub fn add_scrap_pile(&mut self, pos: Vec3, amount: f32) -> u32 {
        let id = self.next_scrap;
        self.next_scrap = self.next_scrap.wrapping_add(1);
        self.scrap.push(ScrapPile {
            id,
            pos,
            amount,
            spawn: None,
        });
        id
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
        if ShipTuning::for_type(ship_type).is_none() {
            return Err(SimError::ShipTypeUnavailable(ship_type));
        }
        let team = match team {
            Some(t) if t >= TEAMS => return Err(SimError::InvalidTeam(t)),
            Some(t) => t,
            None => {
                let on_team = |t: u8| self.ships.values().filter(|s| s.team == t).count();
                if on_team(1) < on_team(0) {
                    1
                } else {
                    0
                }
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
            &self.config.tuning,
            Vec3::ZERO,
            Quat::IDENTITY,
        );
        self.ships.insert(id, ship);
        self.respawn(id);
        Ok(id)
    }

    /// Removes a player. Their projectiles stay in flight.
    pub fn remove_player(&mut self, id: PlayerId) -> bool {
        self.ships.remove(&id).is_some()
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

    /// Picks the next spawn point for the ship's team and resets it with the loadout.
    fn respawn(&mut self, id: PlayerId) -> Option<Vec3> {
        let cfg = Arc::clone(&self.config);
        let old = self.ships.get(&id)?;
        let team = old.team;
        let candidates: Vec<&SpawnPoint> =
            cfg.setup.spawns.iter().filter(|s| s.team == team).collect();
        // `World::new` guarantees every team has a spawn point; the fallback only
        // guards against a hand-built config.
        let (pos, orientation) = if candidates.is_empty() {
            (Vec3::ZERO, Quat::IDENTITY)
        } else {
            let cursor = &mut self.spawn_cursor[team as usize];
            let sp = candidates[*cursor as usize % candidates.len()];
            *cursor = cursor.wrapping_add(1);
            (sp.pos, sp.orientation)
        };
        let mut ship = ShipState::new(id, team, old.ship_type, &cfg.tuning, pos, orientation);
        ship.kills = old.kills;
        ship.deaths = old.deaths;
        ship.horizon_assist = old.horizon_assist;
        for &w in &cfg.resolved.loadout_primaries {
            ship.add_weapon(WeaponState {
                weapon: w,
                ammo: 0,
                level: 1,
            });
            ship.primary.get_or_insert(w);
        }
        let slots = cfg.tuning.secondary_slots as usize;
        for &(w, count) in cfg.resolved.loadout_secondaries.iter().take(slots) {
            let max = cfg.weapons.get(w).max_ammo;
            ship.add_weapon(WeaponState {
                weapon: w,
                ammo: count.min(max),
                level: 1,
            });
            ship.secondary.get_or_insert(w);
        }
        self.ships.insert(id, ship);
        Some(pos)
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

        let mut cmds: BTreeMap<PlayerId, InputCmd> = BTreeMap::new();
        for (p, c) in inputs {
            cmds.insert(*p, c.sanitized());
        }
        let ids: Vec<PlayerId> = self.ships.keys().copied().collect();

        // 1. Respawns.
        for &id in &ids {
            let due = self
                .ships
                .get(&id)
                .is_some_and(|s| !s.alive && now >= s.respawn_tick);
            if due {
                if let Some(pos) = self.respawn(id) {
                    events.push(Event::Respawned { player: id, pos });
                }
            }
        }

        // 2. Ships: selection, movement, scavenging, firing.
        for &id in &ids {
            let cmd = cmds
                .get(&id)
                .copied()
                .unwrap_or_else(|| InputCmd::idle(now));
            let Some(ship) = self.ships.get_mut(&id) else {
                continue;
            };
            if !ship.alive {
                continue;
            }
            select_weapon(ship, cmd.weapon_select, &cfg.weapons);
            simulate_movement(ship, &cmd, &cfg.tuning, &*terrain);
            if cmd.buttons.contains(Buttons::SCAVENGE) {
                scavenge(ship, &mut self.scrap, &cfg.tuning);
            }
            let overdrive = cmd.buttons.contains(Buttons::OVERDRIVE);
            for (slot, button) in [
                (WeaponSlot::Primary, Buttons::FIRE_PRIMARY),
                (WeaponSlot::Secondary, Buttons::FIRE_SECONDARY),
            ] {
                if !cmd.buttons.contains(button) {
                    continue;
                }
                if let Some(shot) = try_fire(ship, slot, now, overdrive, &cfg) {
                    let def = cfg.weapons.get(shot.weapon);
                    let fwd = ship.orientation.forward();
                    let id_p = self.next_projectile;
                    self.next_projectile = self.next_projectile.wrapping_add(1);
                    self.projectiles.push(Projectile {
                        id: id_p,
                        owner: ship.player,
                        team: ship.team,
                        weapon: shot.weapon,
                        pos: ship.pos + fwd * (cfg.tuning.radius + def.projectile_radius + 0.05),
                        vel: fwd * def.speed,
                        expires_tick: now + def.lifetime_ticks(),
                        damage: shot.damage,
                        splash_damage: shot.splash_damage,
                        knockback: shot.knockback,
                    });
                    events.push(Event::Fired {
                        player: ship.player,
                        weapon: shot.weapon,
                        projectile: id_p,
                    });
                }
            }
        }

        // 3. Projectiles.
        self.step_projectiles(&cfg, terrain, now, &mut events);

        // 4. Scrap piles and pickups.
        self.cleanup_scrap(&cfg, now);
        self.collect_pickups(&cfg, now, &mut events);
        self.update_pickup_spawns(&mut events);

        // 5. Mode: scores and timer.
        if now % SCORE_INTERVAL_TICKS == 0 || now >= self.mode.end_tick {
            let cov = terrain.paint_coverage().unwrap_or_default();
            if cov != self.mode.coverage {
                self.mode.coverage = cov;
                events.push(Event::ScoreChanged {
                    team_cells: cov.team_cells,
                    surface_cells: cov.surface_cells,
                });
            }
        }
        if now >= self.mode.end_tick {
            let winner = self.mode.leader();
            self.mode.phase = MatchPhase::Ended { winner };
            events.push(Event::MatchEnded {
                winner,
                team_cells: self.mode.coverage.team_cells,
            });
        }
        events
    }

    fn step_projectiles<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        let projectiles = std::mem::take(&mut self.projectiles);
        let mut keep = Vec::with_capacity(projectiles.len());
        let bounds = terrain.bounds();
        for mut p in projectiles {
            if now >= p.expires_tick {
                continue;
            }
            let def = cfg.weapons.get(p.weapon);
            let speed = p.vel.length();
            let dir = p.vel / speed;
            let len = speed * DT;

            let terrain_hit = terrain.raycast(p.pos, dir, len);
            let mut ship_hit: Option<(f32, PlayerId)> = None;
            let hit_radius = cfg.tuning.radius + def.projectile_radius;
            for (id, s) in &self.ships {
                if !s.alive || *id == p.owner {
                    continue;
                }
                if let Some(t) = segment_sphere(p.pos, dir, len, s.pos, hit_radius) {
                    if ship_hit.is_none_or(|(bt, _)| t < bt) {
                        ship_hit = Some((t, *id));
                    }
                }
            }
            match (ship_hit, terrain_hit) {
                (Some((t, target)), th) if th.is_none_or(|h| t <= h.distance) => {
                    let point = p.pos + dir * t;
                    self.impact(cfg, terrain, &p, point, Some(target), false, now, events);
                }
                (_, Some(h)) => {
                    self.impact(cfg, terrain, &p, h.point, None, true, now, events);
                }
                _ => {
                    p.pos += p.vel * DT;
                    if bounds.contains(p.pos) {
                        keep.push(p);
                    }
                }
            }
        }
        // Projectiles spawned during impacts (none in M1) would be appended here.
        keep.append(&mut self.projectiles);
        self.projectiles = keep;
    }

    /// Resolves a projectile impact: direct damage, splash, knockback and terrain edits.
    #[allow(clippy::too_many_arguments)]
    fn impact<T: SimTerrain + ?Sized>(
        &mut self,
        cfg: &SimConfig,
        terrain: &mut T,
        p: &Projectile,
        point: Vec3,
        target: Option<PlayerId>,
        hit_terrain: bool,
        now: u64,
        events: &mut Vec<Event>,
    ) {
        let def = cfg.weapons.get(p.weapon);
        let dir = p.vel.normalize_or_zero();
        events.push(Event::Impact {
            projectile: p.id,
            weapon: p.weapon,
            point,
            target,
        });
        if let Some(t) = target {
            self.hit_ship(cfg, now, t, p, p.damage, dir * p.knockback, true, events);
        }
        if let Some(splash) = def.splash {
            let victims: Vec<(PlayerId, Vec3)> = self
                .ships
                .values()
                .filter(|s| s.alive && Some(s.player) != target)
                .filter(|s| splash.hits_owner || s.player != p.owner)
                .map(|s| (s.player, s.pos))
                .collect();
            for (id, pos) in victims {
                let d = pos.distance(point);
                if d >= splash.radius {
                    continue;
                }
                let falloff = 1.0 - d / splash.radius;
                let push = (pos - point).try_normalize().unwrap_or(dir);
                self.hit_ship(
                    cfg,
                    now,
                    id,
                    p,
                    p.splash_damage * falloff,
                    push * (p.knockback * falloff),
                    false,
                    events,
                );
            }
        }
        if hit_terrain {
            if def.terrain.crater_radius > 0.0 {
                let edit = TerrainEdit::SubtractSphere {
                    center: point,
                    radius: def.terrain.crater_radius,
                };
                terrain.apply_edit(&edit);
                events.push(Event::TerrainEdit(edit));
            }
            if def.terrain.paint_radius > 0.0 {
                let edit = TerrainEdit::Paint {
                    center: point,
                    radius: def.terrain.paint_radius,
                    team: paint_team_id(p.team),
                };
                terrain.apply_edit(&edit);
                events.push(Event::TerrainEdit(edit));
            }
        }
    }

    /// Applies damage and knockback from projectile `p` to ship `target`,
    /// scaling both by the friendly-fire percentage between teammates.
    #[allow(clippy::too_many_arguments)]
    fn hit_ship(
        &mut self,
        cfg: &SimConfig,
        now: u64,
        target: PlayerId,
        p: &Projectile,
        damage: f32,
        knockback: Vec3,
        direct: bool,
        events: &mut Vec<Event>,
    ) {
        let Some(ship) = self.ships.get_mut(&target) else {
            return;
        };
        if !ship.alive {
            return;
        }
        let scale = if ship.team == p.team && ship.player != p.owner {
            cfg.resolved.friendly_fire_scale()
        } else {
            1.0
        };
        ship.vel += knockback * scale;
        let amount = damage * scale;
        if amount <= 0.0 {
            return;
        }
        ship.hull -= amount;
        if ship.player != p.owner {
            ship.last_attacker = Some(p.owner);
        }
        events.push(Event::Damaged {
            target,
            attacker: p.owner,
            weapon: p.weapon,
            amount,
            direct,
        });
        if ship.hull <= 0.0 {
            self.kill(cfg, now, target, Some(p.owner), Some(p.weapon), events);
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
        ship.hull = 0.0;
        ship.alive = false;
        ship.deaths += 1;
        ship.respawn_tick = now + cfg.resolved.respawn_ticks.max(1);
        ship.vel = Vec3::ZERO;
        let team = ship.team;
        let wreck_pos = ship.pos;
        let wreck_amount = cfg.tuning.wreck_scrap + ship.scrap * 0.5;
        if let Some(k) = killer.filter(|k| *k != victim) {
            if let Some(ks) = self.ships.get_mut(&k) {
                if ks.team != team {
                    ks.kills += 1;
                }
            }
        }
        events.push(Event::Killed {
            victim,
            killer,
            weapon,
        });
        if wreck_amount > 0.0 {
            self.add_scrap_pile(wreck_pos, wreck_amount);
            let wrecks = self.scrap.iter().filter(|s| s.spawn.is_none()).count();
            if wrecks > MAX_WRECKS {
                if let Some(i) = self.scrap.iter().position(|s| s.spawn.is_none()) {
                    self.scrap.remove(i);
                }
            }
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

    /// Ships touching weapon pickups collect them (lowest player id first).
    fn collect_pickups(&mut self, cfg: &SimConfig, now: u64, events: &mut Vec<Event>) {
        let reach = PICKUP_RADIUS + cfg.tuning.radius;
        for (i, sp) in self.pickups.iter_mut().enumerate() {
            let Some((entry, kind)) = sp.content else {
                continue;
            };
            let PickupKind::Weapon { weapon, ammo } = kind else {
                continue;
            };
            for ship in self.ships.values_mut() {
                if !ship.alive || ship.pos.distance_squared(sp.pos) > reach * reach {
                    continue;
                }
                if collect_weapon(ship, weapon, ammo, cfg) {
                    sp.content = None;
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
        let weights: Vec<f32> = table
            .iter()
            .map(|e| match e.kind {
                // Defensive: never spawn a weapon the rule set does not allow as a pickup.
                PickupKind::Weapon { weapon, .. }
                    if !cfg.resolved.availability(weapon).as_pickup() =>
                {
                    0.0
                }
                _ => e.weight,
            })
            .collect();
        for i in 0..self.pickups.len() {
            let sp = &self.pickups[i];
            if sp.content.is_some() || self.tick < sp.respawn_tick {
                continue;
            }
            let Some(entry) = self.rng.weighted_index(&weights) else {
                // Nothing can spawn here; never retry.
                self.pickups[i].respawn_tick = u64::MAX;
                continue;
            };
            let kind = table[entry].kind;
            let pos = self.pickups[i].pos;
            self.pickups[i].content = Some((entry as u16, kind));
            if let PickupKind::Scrap { amount } = kind {
                let id = self.next_scrap;
                self.next_scrap = self.next_scrap.wrapping_add(1);
                self.scrap.push(ScrapPile {
                    id,
                    pos,
                    amount,
                    spawn: Some(i as u16),
                });
            }
            events.push(Event::PickupSpawned {
                spawn: i as u16,
                kind,
            });
        }
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
        h.write_u32(self.next_scrap);
        h.write_u32(self.spawn_cursor[0]);
        h.write_u32(self.spawn_cursor[1]);

        h.write_u32(self.ships.len() as u32);
        for s in self.ships.values() {
            hash_ship(&mut h, s);
        }
        h.write_u32(self.projectiles.len() as u32);
        for p in &self.projectiles {
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
        }
        h.write_u32(self.scrap.len() as u32);
        for s in &self.scrap {
            h.write_u32(s.id);
            h.write_vec3(s.pos);
            h.write_f32(s.amount);
            h.write_u32(s.spawn.map_or(u32::MAX, |x| x as u32));
        }
        h.write_u32(self.pickups.len() as u32);
        for sp in &self.pickups {
            h.write_vec3(sp.pos);
            h.write_u64(sp.respawn_tick);
            match sp.content {
                None => h.write_u8(0),
                Some((entry, kind)) => {
                    h.write_u8(1);
                    h.write_u16(entry);
                    match kind {
                        PickupKind::Weapon { weapon, ammo } => {
                            h.write_u8(weapon.0);
                            h.write_u32(ammo);
                        }
                        PickupKind::Scrap { amount } => h.write_f32(amount),
                    }
                }
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
        h.finish()
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

/// Draws scrap from piles within reach: repairs hull first, the rest goes to
/// the scrap store. This is the only way hull is ever restored.
fn scavenge(ship: &mut ShipState, piles: &mut [ScrapPile], t: &ShipTuning) {
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
    if gained <= 0.0 {
        return;
    }
    let to_hull = gained.min(hull_room);
    ship.hull = (ship.hull + to_hull * t.hull_per_scrap).min(t.hull_max);
    ship.scrap = (ship.scrap + (gained - to_hull)).min(t.scrap_max);
}

/// A shot that was paid for.
struct Shot {
    weapon: WeaponId,
    damage: f32,
    splash_damage: f32,
    knockback: f32,
}

/// Checks rules, cooldown and cost for the selected weapon in `slot`; pays the
/// cost and starts the cooldown if the shot goes off.
fn try_fire(
    ship: &mut ShipState,
    slot: WeaponSlot,
    now: u64,
    overdrive: bool,
    cfg: &SimConfig,
) -> Option<Shot> {
    let w = match slot {
        WeaponSlot::Primary => ship.primary?,
        WeaponSlot::Secondary => ship.secondary?,
    };
    // The rule set is enforced here too: a banned weapon never fires, even if
    // it somehow ended up in the inventory.
    if cfg.resolved.availability(w).is_banned() {
        return None;
    }
    let def = cfg.weapons.get(w);
    if def.slot != slot {
        return None;
    }
    let ready = match slot {
        WeaponSlot::Primary => ship.primary_ready_tick,
        WeaponSlot::Secondary => ship.secondary_ready_tick,
    };
    if now < ready {
        return None;
    }
    let level = ship.weapon(w)?.level;
    let t = &cfg.tuning;
    match def.cost {
        WeaponCost::Energy(c) => {
            if ship.energy >= c {
                ship.energy -= c;
            } else if overdrive {
                // Overdrive gamble: drain the rest of the energy, pay the
                // deficit with hull at 2:1. Never self-destructs.
                let hull_cost = (c - ship.energy) * t.overdrive_hull_per_energy;
                if ship.hull - hull_cost < 1.0 {
                    return None;
                }
                ship.energy = 0.0;
                ship.hull -= hull_cost;
            } else {
                return None;
            }
        }
        WeaponCost::Scrap(c) => {
            if ship.scrap < c {
                return None;
            }
            ship.scrap -= c;
        }
        WeaponCost::Count(n) => {
            let st = ship.weapon_mut(w)?;
            if st.ammo < n {
                return None;
            }
            st.ammo -= n;
        }
    }
    let next = now + def.fire_interval_ticks();
    match slot {
        WeaponSlot::Primary => ship.primary_ready_tick = next,
        WeaponSlot::Secondary => ship.secondary_ready_tick = next,
    }
    let mult = def.level_multiplier(level) * t.damage_mult;
    Some(Shot {
        weapon: w,
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
    if let Some(st) = ship.weapon_mut(w) {
        return match def.cost {
            WeaponCost::Count(_) => {
                if st.ammo >= def.max_ammo {
                    false
                } else {
                    st.ammo = st.ammo.saturating_add(ammo.max(1)).min(def.max_ammo);
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
        let slots = cfg.tuning.secondary_slots as usize;
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
        WeaponCost::Count(_) => ammo.max(1).min(def.max_ammo),
        _ => 0,
    };
    ship.add_weapon(WeaponState {
        weapon: w,
        ammo,
        level: 1,
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
