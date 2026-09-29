//! The browser client's game core as plain Rust (tested natively; `lib.rs`
//! only converts to JavaScript types).
//!
//! [`GameCore`] runs in one of two modes behind one API:
//!
//! - **Online**: wraps `ls_game::client::ClientSession`. Connect-agnostic: the
//!   host feeds every received packet to [`GameCore::handle_packet`] and sends
//!   the packets [`GameCore::frame`] returns.
//! - **Practice**: a local `ls_sim::World` built from a share code (the same
//!   match setup as a server), the local player plus idle target dummies.
//!
//! Both expose the local ship pose (prediction / local simulation,
//! interpolated between ticks for smooth rendering at any frame rate), remote
//! ships and projectiles, HUD values, a kill feed and chunk meshes streamed
//! around the camera with dirty-chunk remeshing after terrain edits.

use std::collections::BTreeSet;

use ls_game::client::ClientSession;
use ls_game::setup::{match_seed, match_setup};
use ls_game::{GameError, GameTerrain, MatchConfig};
use ls_math::terrain::TerrainQuery;
use ls_math::{Aabb, Quat, Vec3};
use ls_net::{check_welcome, ship_flags, Message, NetEvent, NetShip};
use ls_sim::{
    Event, MatchPhase, PlayerId, ShipState, ShipTuning, ShipType, WeaponCost, WeaponDef, WeaponId,
    WeaponTable, World, WorldConfig,
};
use ls_worldgen::{ChunkCoord, Generator, Mesh, ShareCode};

use crate::controls::{Controls, FixedStep, LookAccumulator};
use crate::stream::ChunkStreamer;

/// Ticks per second as `f32`.
const HZ: f32 = ls_math::TICK_HZ as f32;
/// No render interpolation across jumps longer than this (respawns).
const TELEPORT: f32 = 8.0;
/// Kill-feed lines kept until the host takes them.
const MAX_FEED: usize = 32;
/// Default number of idle target dummies in practice.
pub const DEFAULT_DUMMIES: u8 = 3;

/// Local ship pose to render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub alive: bool,
}

/// Another ship, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShipView {
    /// Wire id (online) or player id (practice).
    pub id: u8,
    pub team: u8,
    pub alive: bool,
    pub boosting: bool,
    pub pos: Vec3,
    pub rot: Quat,
}

/// A projectile, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileView {
    /// Weapon id.
    pub kind: u8,
    /// Shooter's id (255 = unknown).
    pub owner: u8,
    pub pos: Vec3,
    pub vel: Vec3,
}

/// Values for the HUD.
#[derive(Debug, Clone, PartialEq)]
pub struct HudState {
    pub hull: f32,
    pub hull_max: f32,
    pub energy: f32,
    pub energy_max: f32,
    pub boost: f32,
    pub boost_max: f32,
    pub scrap: f32,
    pub scrap_max: f32,
    /// Selected primary weapon name and its ammo text.
    pub weapon: String,
    pub ammo: String,
    /// Selected secondary weapon and count (empty if none).
    pub secondary: String,
    /// Team shares of the painted surface, 0..1.
    pub paint: [f32; 2],
    pub time_left_secs: f32,
    pub team: u8,
    pub alive: bool,
    /// Short status line ("Waiting for the server", "Destroyed", "Match over").
    pub status: String,
}

/// A chunk mesh produced by [`GameCore::mesh_next`].
#[derive(Debug, Clone)]
pub struct ChunkMeshOut {
    pub coord: ChunkCoord,
    pub aabb: Aabb,
    pub mesh: Mesh,
}

#[derive(Debug)]
struct Online {
    session: ClientSession,
    /// Secondary loadout weapon and starting count (ammo is not on the wire;
    /// the count is estimated from the player's own projectiles).
    secondary: Option<(WeaponId, u32)>,
    ammo_estimate: u32,
    seen_own: BTreeSet<u16>,
    last_snapshot: Option<u32>,
    was_alive: bool,
    match_over: Option<Option<u8>>,
}

#[derive(Debug)]
struct Practice {
    world: World,
    terrain: GameTerrain,
    me: PlayerId,
    /// All ships before the latest tick (for render interpolation).
    prev: Vec<ShipState>,
}

#[derive(Debug)]
enum Mode {
    Online(Box<Online>),
    Practice(Box<Practice>),
}

/// See the module docs.
#[derive(Debug)]
pub struct GameCore {
    mode: Mode,
    step: FixedStep,
    look: LookAccumulator,
    streamer: ChunkStreamer,
    feed: Vec<String>,
    weapons: WeaponTable,
    tuning: ShipTuning,
}

impl GameCore {
    // ----- construction ----------------------------------------------------

    /// Online session from the server's first packet (`Welcome`; a `Reject`
    /// or anything else becomes an error with the reason).
    pub fn online(welcome_packet: &[u8]) -> Result<GameCore, String> {
        let msg = Message::decode(welcome_packet).map_err(|e| format!("bad welcome: {e}"))?;
        let welcome = check_welcome(msg).map_err(|e| e.to_string())?;
        let weapons = WeaponTable::builtin();
        let tuning = ShipTuning::tinker();
        let session = ClientSession::from_welcome(&welcome, weapons.clone(), tuning.clone())
            .map_err(|e| e.to_string())?;
        let secondary = session
            .rules()
            .resolve(&weapons)
            .ok()
            .and_then(|r| r.loadout_secondaries.first().copied());
        let streamer = ChunkStreamer::new(session.terrain().chunk_coords());
        Ok(GameCore {
            mode: Mode::Online(Box::new(Online {
                session,
                secondary,
                ammo_estimate: secondary.map_or(0, |s| s.1),
                seen_own: BTreeSet::new(),
                last_snapshot: None,
                was_alive: false,
                match_over: None,
            })),
            step: FixedStep::default(),
            look: LookAccumulator::default(),
            streamer,
            feed: Vec::new(),
            weapons,
            tuning,
        })
    }

    /// Offline practice on the level of `code` with `dummies` idle opponents.
    pub fn practice(code: &str, dummies: u8) -> Result<GameCore, String> {
        let code = ShareCode::decode(code).map_err(|e| e.to_string())?;
        let gen = Generator::from_code(&code).map_err(|e| e.to_string())?;
        Self::practice_with_generator(gen, dummies, true).map_err(|e| e.to_string())
    }

    /// Practice on any generator (tests use small levels). `pregenerate`
    /// builds every chunk up front, so the first paint score update (tick 30)
    /// does not stall a frame.
    pub fn practice_with_generator(
        gen: Generator,
        dummies: u8,
        pregenerate: bool,
    ) -> Result<GameCore, GameError> {
        let code = gen.code();
        let cfg = MatchConfig::paint_the_void(code);
        let mut terrain = GameTerrain::new(gen);
        let setup = match_setup(&cfg.code, &cfg.rules, &terrain)?;
        let mut world = World::new(WorldConfig {
            seed: match_seed(&cfg.code),
            weapons: cfg.weapons.clone(),
            rules: cfg.rules.clone(),
            tuning: cfg.tuning.clone(),
            setup,
        })?;
        if pregenerate {
            terrain.generate_all();
        }
        let me = world.add_player(Some(0), ShipType::Tinker)?;
        for _ in 0..dummies.min(15) {
            world.add_player(Some(1), ShipType::Tinker)?;
        }
        let prev = world.ships().cloned().collect();
        let streamer = ChunkStreamer::new(terrain.chunk_coords());
        Ok(GameCore {
            mode: Mode::Practice(Box::new(Practice {
                world,
                terrain,
                me,
                prev,
            })),
            step: FixedStep::default(),
            look: LookAccumulator::default(),
            streamer,
            feed: Vec::new(),
            weapons: cfg.weapons,
            tuning: cfg.tuning,
        })
    }

    // ----- info --------------------------------------------------------------

    pub fn is_online(&self) -> bool {
        matches!(self.mode, Mode::Online(_))
    }

    fn terrain(&self) -> &GameTerrain {
        match &self.mode {
            Mode::Online(o) => o.session.terrain(),
            Mode::Practice(p) => &p.terrain,
        }
    }

    fn terrain_mut(&mut self) -> &mut GameTerrain {
        match &mut self.mode {
            Mode::Online(o) => o.session.terrain_mut(),
            Mode::Practice(p) => &mut p.terrain,
        }
    }

    /// Printed share code of the level being played.
    pub fn share_code(&self) -> String {
        self.terrain()
            .share_code()
            .encode()
            .unwrap_or_else(|_| String::from("?"))
    }

    /// Level bounds (outside is solid).
    pub fn bounds(&self) -> Aabb {
        self.terrain().bounds()
    }

    /// Local player's id (wire id online, player id in practice).
    pub fn player(&self) -> u8 {
        match &self.mode {
            Mode::Online(o) => o.session.player(),
            Mode::Practice(p) => p.me.0 as u8,
        }
    }

    pub fn team(&self) -> u8 {
        match &self.mode {
            Mode::Online(o) => o.session.team(),
            Mode::Practice(p) => p.world.ship(p.me).map_or(0, |s| s.team),
        }
    }

    /// Weapon table (ids are `ProjectileView::kind` and `weapon_select - 1`).
    pub fn weapons(&self) -> &WeaponTable {
        &self.weapons
    }

    /// Terrain desync check (edit log + SDF lattice + paint); compare with the
    /// server's `GameTerrain::state_hash`. Expensive: debug use only.
    pub fn terrain_hash(&self) -> u64 {
        self.terrain().state_hash()
    }

    /// Terrain edits applied so far.
    pub fn edit_count(&self) -> u32 {
        self.terrain().edit_count()
    }

    // ----- network -------------------------------------------------------------

    /// Handles one packet from the server. Returns the reason if the server
    /// ended the session. Practice mode ignores packets.
    pub fn handle_packet(&mut self, bytes: &[u8]) -> Result<Option<String>, String> {
        let Mode::Online(o) = &mut self.mode else {
            return Ok(None);
        };
        let incoming = o.session.handle_packet(bytes).map_err(|e| e.to_string())?;
        self.collect_online_events();
        self.pull_dirty();
        Ok(match incoming {
            ls_game::client::Incoming::Disconnected(reason) => Some(reason),
            _ => None,
        })
    }

    // ----- per frame -----------------------------------------------------------

    /// Advances local time by `dt` seconds with this frame's controls.
    /// Returns the packets to send to the server (in order; empty offline).
    pub fn frame(&mut self, dt: f64, controls: Controls) -> Vec<Vec<u8>> {
        let due = self.step.advance(dt);
        let max_turn = self.tuning.max_turn_rate * ls_math::DT;
        let mut out = Vec::new();
        match &mut self.mode {
            Mode::Online(o) => {
                // The clock may run more ticks than are due (catching up) or
                // fewer (skipping); the frame's look is spread over the ticks
                // that actually run, so it is applied exactly once.
                let n = o.session.plan_ticks(due);
                let cmd = self.look.command(controls, n, max_turn);
                for msg in o.session.run_ticks(n, due, cmd) {
                    match msg.encode() {
                        Ok(bytes) => out.push(bytes),
                        Err(e) => push_feed(&mut self.feed, format!("Could not send input: {e}")),
                    }
                }
                self.collect_online_events();
                self.track_online_ammo();
            }
            Mode::Practice(p) => {
                let cmd = self.look.command(controls, due, max_turn);
                let mut events = Vec::new();
                for _ in 0..due {
                    p.prev = p.world.ships().cloned().collect();
                    let tick_cmd = ls_sim::InputCmd {
                        tick: p.world.tick() + 1,
                        ..cmd
                    };
                    events.extend(p.world.step(&mut p.terrain, &[(p.me, tick_cmd)]));
                }
                for e in events {
                    self.practice_event(e);
                }
            }
        }
        self.pull_dirty();
        out
    }

    fn pull_dirty(&mut self) {
        let dirty = self.terrain_mut().take_dirty_chunks();
        self.streamer.mark_dirty(dirty);
    }

    /// Local ship pose, interpolated between the last two ticks (plus the
    /// prediction's correction smoothing online). `None` until the first
    /// snapshot anchors the prediction.
    pub fn local_pose(&self) -> Option<Pose> {
        let alpha = self.step.alpha();
        match &self.mode {
            Mode::Online(o) => {
                let p = o.session.predictor();
                let latest = p.latest_tick()?;
                let cur = p.state_at(latest).or_else(|| p.ship())?;
                let prev = latest
                    .checked_sub(1)
                    .and_then(|t| p.state_at(t))
                    .unwrap_or(cur);
                let (pos, rot) = blend(prev, cur, alpha);
                // Correction smoothing: offset between the smoothed render pose
                // and the raw prediction.
                let render_rot = p.render_rot().unwrap_or(cur.orientation);
                let rot_offset = render_rot * cur.orientation.conjugate();
                Some(Pose {
                    pos: pos + p.pos_offset(),
                    rot: (rot_offset * rot).normalize(),
                    vel: cur.vel,
                    alive: cur.alive,
                })
            }
            Mode::Practice(p) => {
                let cur = p.world.ship(p.me)?;
                let prev = p.prev.iter().find(|s| s.player == p.me).unwrap_or(cur);
                let (pos, rot) = blend(prev, cur, alpha);
                Some(Pose {
                    pos,
                    rot,
                    vel: cur.vel,
                    alive: cur.alive,
                })
            }
        }
    }

    /// Other ships: interpolated 100 ms in the past online, between the last
    /// two ticks in practice.
    pub fn remote_ships(&self) -> Vec<ShipView> {
        let alpha = self.step.alpha();
        match &self.mode {
            Mode::Online(o) => {
                let t = o.session.render_tick() + alpha as f64;
                o.session
                    .remote_ships_at(t)
                    .iter()
                    .map(net_ship_view)
                    .collect()
            }
            Mode::Practice(p) => p
                .world
                .ships()
                .filter(|s| s.player != p.me)
                .map(|cur| {
                    let prev = p
                        .prev
                        .iter()
                        .find(|s| s.player == cur.player)
                        .unwrap_or(cur);
                    let (pos, rot) = blend(prev, cur, alpha);
                    ShipView {
                        id: cur.player.0 as u8,
                        team: cur.team,
                        alive: cur.alive,
                        boosting: cur.boosting,
                        pos,
                        rot,
                    }
                })
                .collect(),
        }
    }

    /// Projectiles moved along their velocity to the current render time.
    pub fn projectiles(&self) -> Vec<ProjectileView> {
        let dt = self.step.alpha() / HZ;
        match &self.mode {
            Mode::Online(o) => o
                .session
                .projectiles()
                .iter()
                .map(|p| ProjectileView {
                    kind: p.kind,
                    owner: p.owner,
                    pos: p.pos + p.vel * dt,
                    vel: p.vel,
                })
                .collect(),
            Mode::Practice(p) => p
                .world
                .projectiles()
                .iter()
                .map(|pr| ProjectileView {
                    kind: pr.weapon.0,
                    owner: pr.owner.0.min(254) as u8,
                    // One tick behind like the ships, then forward by alpha.
                    pos: pr.pos + pr.vel * ((self.step.alpha() - 1.0) / HZ),
                    vel: pr.vel,
                })
                .collect(),
        }
    }

    /// HUD values.
    pub fn hud(&self) -> HudState {
        let t = &self.tuning;
        let mut hud = HudState {
            hull: t.hull_max,
            hull_max: t.hull_max,
            energy: t.spawn_energy,
            energy_max: t.energy_max,
            boost: t.spawn_boost,
            boost_max: t.boost_max,
            scrap: t.spawn_scrap,
            scrap_max: t.scrap_max,
            weapon: String::new(),
            ammo: String::new(),
            secondary: String::new(),
            paint: [0.0; 2],
            time_left_secs: 0.0,
            team: self.team(),
            alive: true,
            status: String::new(),
        };
        match &self.mode {
            Mode::Online(o) => self.online_hud(o, &mut hud),
            Mode::Practice(p) => self.practice_hud(p, &mut hud),
        }
        hud
    }

    fn online_hud(&self, o: &Online, hud: &mut HudState) {
        let s = &o.session;
        let Some(snap) = s.latest_snapshot() else {
            hud.status = "Waiting for the server".into();
            hud.time_left_secs = s.rules().paint_the_void_rules().match_secs;
            return;
        };
        let own = snap.ships.iter().find(|x| x.id == s.player());
        if let Some(st) = own.and_then(|o| o.status) {
            let (hull, energy, boost, scrap) = s.conv().status_values(&st);
            hud.hull = hull;
            hud.energy = energy;
            hud.boost = boost;
            hud.scrap = scrap;
        }
        // Boost is predicted (it changes the speed cap), so show the prediction.
        if let Some(ship) = s.local_ship() {
            hud.boost = ship.boost;
        }
        hud.alive = own.is_none_or(|o| o.flags & ship_flags::ALIVE != 0);
        if let Some(own) = own {
            let w = WeaponId(ship_flags::weapon(own.flags));
            if (w.0 as usize) < self.weapons.len() {
                let def = self.weapons.get(w);
                hud.weapon = def.name.clone();
                hud.ammo = ammo_text(def, hud.energy, hud.scrap, None);
            }
        }
        if let Some((w, _)) = o.secondary {
            if (w.0 as usize) < self.weapons.len() {
                hud.secondary = format!("{} ×{}", self.weapons.get(w).name, o.ammo_estimate);
            }
        }
        if let [a, b, surface, ..] = snap.scores[..] {
            if surface > 0 {
                hud.paint = [a as f32 / surface as f32, b as f32 / surface as f32];
            }
        }
        let elapsed = (s.clock().server_time() - snap.tick as f64).max(0.0);
        hud.time_left_secs = ((snap.match_ticks_left as f64 - elapsed).max(0.0) / HZ as f64) as f32;
        hud.status = match o.match_over {
            Some(winner) => match_over_text(winner, hud.team),
            None if !hud.alive => "Destroyed, respawning".into(),
            None if !s.is_synced() => "Syncing with the server".into(),
            None => String::new(),
        };
    }

    fn practice_hud(&self, p: &Practice, hud: &mut HudState) {
        let w = &p.world;
        if let Some(ship) = w.ship(p.me) {
            hud.hull = ship.hull;
            hud.energy = ship.energy;
            hud.boost = ship.boost;
            hud.scrap = ship.scrap;
            hud.alive = ship.alive;
            if let Some(id) = ship.primary {
                let def = self.weapons.get(id);
                hud.weapon = def.name.clone();
                hud.ammo = ammo_text(
                    def,
                    ship.energy,
                    ship.scrap,
                    ship.weapon(id).map(|s| s.ammo),
                );
            }
            if let Some(id) = ship.secondary {
                let def = self.weapons.get(id);
                let n = ship.weapon(id).map_or(0, |s| s.ammo);
                hud.secondary = format!("{} ×{}", def.name, n);
            }
        }
        let mode = w.mode();
        hud.paint = [mode.score_percent(0) / 100.0, mode.score_percent(1) / 100.0];
        hud.time_left_secs = mode.remaining_ticks(w.tick()) as f32 / HZ;
        hud.status = if mode.phase != MatchPhase::Running {
            match_over_text(mode.leader(), hud.team)
        } else if !hud.alive {
            "Destroyed, respawning".into()
        } else {
            String::new()
        };
    }

    /// Kill-feed lines since the last call.
    pub fn take_feed(&mut self) -> Vec<String> {
        std::mem::take(&mut self.feed)
    }

    fn name(&self, id: u8) -> String {
        if id == self.player() {
            "You".into()
        } else if id == u8::MAX {
            "The level".into()
        } else {
            format!("Player {}", id as u32 + 1)
        }
    }

    fn collect_online_events(&mut self) {
        let events = match &mut self.mode {
            Mode::Online(o) => o.session.take_events(),
            Mode::Practice(_) => return,
        };
        for e in events {
            let line = match e {
                NetEvent::Kill { killer, victim, .. } => Some(kill_line(
                    (killer != u8::MAX).then(|| self.name(killer)),
                    self.name(victim),
                )),
                NetEvent::MatchOver { winner_team, .. } => {
                    if let Mode::Online(o) = &mut self.mode {
                        o.match_over = Some(winner_team);
                    }
                    Some(match_over_text(winner_team, self.team()))
                }
                NetEvent::Hit { .. } | NetEvent::Respawn { .. } => None,
            };
            if let Some(l) = line {
                push_feed(&mut self.feed, l);
            }
        }
    }

    /// Online secondary ammo estimate: reset to the loadout count on spawn,
    /// minus the player's own projectiles of that weapon.
    fn track_online_ammo(&mut self) {
        let Mode::Online(o) = &mut self.mode else {
            return;
        };
        let Some(snap) = o.session.latest_snapshot() else {
            return;
        };
        if o.last_snapshot == Some(snap.tick) {
            return;
        }
        o.last_snapshot = Some(snap.tick);
        let me = o.session.player();
        let alive = snap
            .ships
            .iter()
            .find(|s| s.id == me)
            .is_some_and(|s| s.flags & ship_flags::ALIVE != 0);
        if alive && !o.was_alive {
            o.ammo_estimate = o.secondary.map_or(0, |s| s.1);
        }
        o.was_alive = alive;
        let Some((sec, _)) = o.secondary else {
            return;
        };
        let current: BTreeSet<u16> = snap
            .projectiles
            .iter()
            .filter(|p| p.owner == me)
            .map(|p| p.id)
            .collect();
        for p in snap
            .projectiles
            .iter()
            .filter(|p| p.owner == me && p.kind == sec.0)
        {
            if !o.seen_own.contains(&p.id) {
                o.ammo_estimate = o.ammo_estimate.saturating_sub(1);
            }
        }
        o.seen_own = current;
    }

    fn practice_event(&mut self, e: Event) {
        let line = match e {
            Event::Killed { victim, killer, .. } => Some(kill_line(
                killer.map(|k| self.name(k.0 as u8)),
                self.name(victim.0 as u8),
            )),
            Event::MatchEnded { winner, .. } => Some(match_over_text(winner, self.team())),
            _ => None,
        };
        if let Some(l) = line {
            push_feed(&mut self.feed, l);
        }
    }

    // ----- terrain meshes --------------------------------------------------------

    /// Meshes the next chunk around `cam` (dirty chunks first, then missing
    /// ones by distance), or `None` when everything in range is up to date.
    /// Call repeatedly until the frame's meshing budget is spent.
    pub fn mesh_next(&mut self, cam: Vec3) -> Option<ChunkMeshOut> {
        self.pull_dirty();
        let coord = self.streamer.next(cam)?;
        let mesh = self.terrain_mut().mesh_chunk(coord);
        Some(ChunkMeshOut {
            coord,
            aabb: coord.aabb(),
            mesh,
        })
    }

    /// Chunks whose meshes should be dropped (far from `cam`).
    pub fn take_unloads(&mut self, cam: Vec3) -> Vec<ChunkCoord> {
        self.streamer.take_unloads(cam)
    }

    /// Chunks waiting to be meshed around `cam`.
    pub fn pending_chunks(&mut self, cam: Vec3) -> usize {
        self.pull_dirty();
        self.streamer.pending(cam)
    }

    pub fn set_view_radius(&mut self, r: f32) {
        self.streamer.set_view_radius(r);
    }

    /// Forgets which chunks the host shows, so all are meshed again.
    pub fn reset_meshes(&mut self) {
        self.streamer.reset();
    }
}

fn push_feed(feed: &mut Vec<String>, line: String) {
    if feed.len() >= MAX_FEED {
        feed.remove(0);
    }
    feed.push(line);
}

fn kill_line(killer: Option<String>, victim: String) -> String {
    match killer {
        Some(k) if k == victim => format!("{victim} self-destructed"),
        Some(k) => format!("{k} destroyed {victim}"),
        None => format!("{victim} crashed"),
    }
}

/// Team names match the paint colours of the web client.
pub fn team_name(team: u8) -> &'static str {
    match team {
        0 => "Orange",
        1 => "Blue",
        _ => "Nobody",
    }
}

fn match_over_text(winner: Option<u8>, own_team: u8) -> String {
    match winner {
        Some(t) if t == own_team => format!("Match over: {} wins (your team)", team_name(t)),
        Some(t) => format!("Match over: {} wins", team_name(t)),
        None => "Match over: draw".into(),
    }
}

/// Ammo text for a weapon: shots left from the resource it spends.
pub fn ammo_text(def: &WeaponDef, energy: f32, scrap: f32, count: Option<u32>) -> String {
    let shots = |have: f32, cost: f32| {
        if cost > 0.0 {
            format!("{} shots", (have / cost).floor().max(0.0) as u32)
        } else {
            "∞".into()
        }
    };
    match def.cost {
        WeaponCost::Energy(c) => shots(energy, c),
        WeaponCost::Scrap(c) => shots(scrap, c),
        WeaponCost::Count(_) => count.map_or_else(|| "?".into(), |n| n.to_string()),
    }
}

fn net_ship_view(s: &NetShip) -> ShipView {
    ShipView {
        id: s.id,
        team: ship_flags::team(s.flags),
        alive: s.flags & ship_flags::ALIVE != 0,
        boosting: s.flags & ship_flags::BOOSTING != 0,
        pos: s.pos,
        rot: s.rot,
    }
}

/// Pose between two tick states (no blending across teleports or deaths).
fn blend(prev: &ShipState, cur: &ShipState, alpha: f32) -> (Vec3, Quat) {
    if prev.alive != cur.alive || prev.pos.distance(cur.pos) > TELEPORT {
        return (cur.pos, cur.orientation);
    }
    (
        prev.pos.lerp(cur.pos, alpha),
        prev.orientation.nlerp(cur.orientation, alpha).normalize(),
    )
}

#[cfg(test)]
mod tests;
