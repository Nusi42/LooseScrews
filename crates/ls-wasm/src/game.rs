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
//!
//! Online, the local player's shots are predicted ([`crate::shots`]) and the
//! player's own projectiles are drawn in the local (predicted) timeline, so
//! a shot leaves the muzzle on the click instead of a round trip later.
//! The core also keeps what the overlays need: ping, prediction counters, a
//! kill/death tally (from the kill events since joining) and the match
//! result ([`GameCore::debug_stats`], [`GameCore::scoreboard`],
//! [`GameCore::match_info`]).
//!
//! M2: generic entities with data-driven visuals ([`GameCore::entities`],
//! [`GameCore::visuals`], see [`crate::scene`]), player names
//! ([`GameCore::players`]), the carried weapons for the weapon HUD
//! ([`GameCore::loadout`]) and weapon selection by number key, mouse wheel
//! and X ([`crate::inventory`]); online the inventory comes from the
//! server (`Message::Inventory`).
//!
//! M3 (protocol 5): the generic mode state of every mode
//! ([`GameCore::mode_state`]), the lobby state and match results
//! ([`GameCore::lobby`], [`GameCore::results`]), lobby requests
//! ([`GameCore::ready_packet`] and friends) and match transitions on the
//! open connection: a new `Welcome` rebuilds the online core in place
//! ([`GameCore::take_match_transition`]; the old level's chunks come back
//! from [`GameCore::take_unloads`]). The Paint Thrower colour and the shot
//! phases follow the server's inventory (weapon state byte and shot counts).

use std::collections::{BTreeMap, VecDeque};

use ls_game::client::ClientSession;
use ls_game::setup::match_setup;
use ls_game::{GameError, GameTerrain, MatchConfig};
use ls_math::terrain::TerrainQuery;
use ls_math::{Aabb, Quat, Vec3};
use ls_net::{
    check_welcome, ship_flags, LobbyStateMsg, MatchResultsMsg, Message, NetEvent, NetModeState,
    NetPlayer, NetShip,
};
use ls_sim::{
    Event, GadgetTable, GameData, InputCmd, MatchPhase, PlayerId, ResolvedRules, ShipState,
    ShipTable, ShipTuning, ShipType, VisualTable, WeaponCost, WeaponDef, WeaponId, WeaponSlot,
    WeaponTable, World,
};
use ls_worldgen::{ChunkCoord, Generator, Mesh, ShareCode};

use crate::beam::{aim_beam, BeamParams, LocalBeam};
use crate::controls::{Controls, FixedStep, LookAccumulator};
use crate::inventory::{resolve_select, Inventory, LoadoutView};
use crate::mechanics::{
    allowed_or_fallback, hit_cue, push_cue, ship_choices, ChargeView, CueView, GadgetView,
    ShipChoice,
};
use crate::scene::{EntityDraw, PlayerInfo};
use crate::shots::{FireRules, Resources, ShotPredictor, ShotStats};
use crate::stream::ChunkStreamer;

/// Ticks per second as `f32`.
const HZ: f32 = ls_math::TICK_HZ as f32;
/// No render interpolation across jumps longer than this (respawns).
const TELEPORT: f32 = 8.0;
/// Kill-feed lines kept until the host takes them.
const MAX_FEED: usize = 32;
/// Paint colour before the first V press (the sim's default,
/// `paint_thrower::selected_color`).
pub const DEFAULT_PAINT_COLOR: u8 = ls_math::terrain::PAINT_RED;
/// Buttons whose press *edge* the simulation acts on (gadget G, weapon mode
/// V). A press on a frame that runs no tick is kept for the next tick, so a
/// tap shorter than a frame is not lost.
const EDGE_BUTTONS: u16 = ls_sim::Buttons::GADGET.0 | ls_sim::Buttons::WEAPON_MODE.0;
/// Default number of idle target dummies in practice.
pub const DEFAULT_DUMMIES: u8 = 3;
/// Own projectiles are extrapolated from their snapshot to the local
/// (predicted) time by at most this many ticks (0.5 s of lead plus latency).
pub const MAX_OWN_EXTRAPOLATION_TICKS: f64 = 30.0;
/// Weight of a new round-trip sample in the smoothed ping.
const PING_SMOOTHING: f64 = 0.25;
/// Outstanding pings remembered for round-trip times.
const MAX_PINGS: usize = 16;

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
    /// `ls_sim::ShipType::index`.
    pub ship_type: u8,
    /// Hitbox radius of the ship type (u).
    pub radius: f32,
    /// Its gadget (Shield, Cloak) is switched on.
    pub gadget_active: bool,
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
    /// Visual id (the weapon's projectile visual in the match's table).
    pub visual: u16,
    /// Replicated `ls_sim::proj_flags` low byte (stuck, armed, ...).
    pub state: u8,
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

/// Counters for the debug overlay.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DebugStats {
    /// Smoothed round-trip time to the server (ms), `None` before the first
    /// pong or offline.
    pub ping_ms: Option<f64>,
    /// Own-ship prediction: snapshots that matched, were corrected
    /// (reconciled) or snapped, and the last error (u).
    pub accepted: u64,
    pub corrected: u64,
    pub snapped: u64,
    pub last_error: f32,
    pub shots: ShotStats,
    pub synced: bool,
}

/// One scoreboard row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreRow {
    /// Wire id (online) or player id (practice).
    pub id: u8,
    /// Player name (online: from the snapshot's player list).
    pub name: String,
    pub team: u8,
    pub alive: bool,
    /// Online: counted from kill events since this client joined.
    pub kills: u32,
    pub deaths: u32,
    pub is_you: bool,
}

/// Match result for the end screen.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MatchInfo {
    pub over: bool,
    /// Winning team, `None` for a draw (or while running).
    pub winner: Option<u8>,
    /// Painted surface cells per team and paintable surface cells.
    pub team_cells: [u64; 2],
    pub surface_cells: u64,
    /// Seconds since the match timer ran out (0 while running).
    pub secs_since_end: f32,
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
    /// The own ship's weapons as the server reported them, with the local
    /// selection on top (see [`crate::inventory`]).
    inventory: Inventory,
    /// First input tick of the newest local weapon selection: server
    /// inventories older than it keep the local selection.
    select_tick: Option<u64>,
    match_over: Option<Option<u8>>,
    /// Resolved rule set (firing rules for shot prediction).
    resolved: ResolvedRules,
    shots: ShotPredictor,
    /// Kills and deaths per wire id, from kill events since joining.
    tally: BTreeMap<u8, (u32, u32)>,
    /// Local time (ms) for round-trip times: the host's clock once it
    /// calls [`GameCore::set_clock_ms`], otherwise the sum of frame deltas
    /// (which times a pong handled before a frame at the previous frame).
    local_ms: f64,
    host_clock: bool,
    /// Outstanding pings: nonce and local send time.
    pings: VecDeque<(u32, f64)>,
    ping_ms: Option<f64>,
    /// Server tick the match ended at, once a snapshot showed it over.
    ended_at: Option<u64>,
}

#[derive(Debug)]
struct Practice {
    world: World,
    terrain: GameTerrain,
    me: PlayerId,
    /// All ships before the latest tick (for render interpolation).
    prev: Vec<ShipState>,
    /// Seconds since the match ended (the world stops ticking then).
    over_secs: f32,
    /// The level's static gravity wells and asteroid belt (M4).
    well_sites: Vec<ls_math::level::WellSite>,
    belt: Option<ls_math::level::BeltSite>,
}

#[derive(Debug)]
enum Mode {
    Online(Box<Online>),
    Practice(Box<Practice>),
}

impl Online {
    /// Runs the shot predictor over the ticks `first..` that the session
    /// just predicted with `cmd` (only once synchronized: before that the
    /// session sends idle commands). Ticks after the match end fire nothing,
    /// like on the server.
    fn predict_shots(
        &mut self,
        first: u64,
        cmd: &InputCmd,
        weapons: &WeaponTable,
        tuning: &ShipTuning,
    ) {
        let end = self.session.match_end_tick();
        let rules = FireRules {
            weapons,
            rules: &self.resolved,
            tuning,
        };
        for tick in first..self.session.next_input_tick() {
            if end.is_some_and(|e| tick > e) || self.match_over.is_some() {
                break;
            }
            let Some(ship) = self.session.predictor().state_at(tick) else {
                continue;
            };
            self.shots
                .on_tick(tick, ship, cmd, rules, self.session.terrain());
        }
    }

    /// Takes the inventory the server sent; a local selection the server
    /// has not processed yet stays shown.
    fn apply_inventory(&mut self, weapons: &ls_sim::WeaponTable) {
        let Some(msg) = self.session.inventory() else {
            return;
        };
        let mut inv = Inventory::from_server(msg, weapons);
        self.shots.carry(inv.states().iter().map(|w| w.weapon));
        // The server's shot counts set the alternating / rotating patterns'
        // phases (no inference from projectiles).
        self.shots.on_inventory(
            msg.tick as u64,
            &inv.shot_counts(),
            Some(self.session.terrain()),
        );
        match self.select_tick {
            Some(t) if (msg.tick as u64) < t => {
                for w in [self.inventory.primary, self.inventory.secondary]
                    .into_iter()
                    .flatten()
                {
                    inv.select(w, weapons);
                }
            }
            _ => self.select_tick = None,
        }
        self.inventory = inv;
    }

    /// Feeds the newest snapshot to the shot predictor: own-ship weapon and
    /// resources, then matches the own projectiles.
    fn reconcile_shots(&mut self) {
        let Some(snap) = self.session.latest_snapshot() else {
            return;
        };
        let me = self.session.player();
        if let Some(own) = snap.ships.iter().find(|s| s.id == me) {
            let primary = WeaponId(ship_flags::weapon(own.flags));
            let mut res = Resources {
                count: self
                    .inventory
                    .secondary
                    .map_or(0, |w| self.inventory.ammo(w)),
                ..Resources::default()
            };
            if let Some(st) = own.status {
                let (hull, energy, _boost, scrap) = self.session.conv().status_values(&st);
                res.hull = hull;
                res.energy = energy;
                res.scrap = scrap;
            }
            self.shots
                .on_snapshot_state(snap.tick as u64, Some(primary), res);
        }
        let own: Vec<_> = snap
            .projectiles
            .iter()
            .filter(|p| p.owner == me)
            .copied()
            .collect();
        self.shots.set_guiding(
            own.iter()
                .any(|p| p.state & ls_game::client::GUIDED_STATE_BIT != 0),
        );
        self.shots.reconcile(snap.tick as u64, &own);
        let ended = snap.mode.phase == ls_net::mode_phase::ENDED
            && snap.mode != ls_net::NetModeState::default();
        if snap.match_ticks_left == 0 || ended {
            self.shots.clear();
            if self.ended_at.is_none() {
                let tick = snap.tick as u64;
                self.ended_at = Some(self.session.match_end_tick().map_or(tick, |e| e.min(tick)));
            }
        }
    }

    fn on_ping_sent(&mut self, nonce: u32) {
        if self.pings.len() >= MAX_PINGS {
            self.pings.pop_front();
        }
        self.pings.push_back((nonce, self.local_ms));
    }

    fn on_pong(&mut self, nonce: u32) {
        let Some(i) = self.pings.iter().position(|(n, _)| *n == nonce) else {
            return;
        };
        let rtt = (self.local_ms - self.pings[i].1).max(0.0);
        self.pings.drain(..=i);
        self.ping_ms = Some(match self.ping_ms {
            Some(p) => p + (rtt - p) * PING_SMOOTHING,
            None => rtt,
        });
    }

    /// Tick at which the local ship is drawn (see `GameCore::local_pose`).
    fn local_render_tick(&self, alpha: f32) -> Option<f64> {
        let latest = self.session.predictor().latest_tick()?;
        Some(latest as f64 - 1.0 + alpha as f64)
    }

    /// Projectiles to draw: other players' at the estimated server time;
    /// the player's own (and predicted ones) at the local predicted time,
    /// hidden once they reach terrain.
    fn projectiles(&self, alpha: f32) -> Vec<ProjectileView> {
        let s = &self.session;
        let Some(snap) = s.latest_snapshot() else {
            return Vec::new();
        };
        let me = s.player();
        let local_t = self.local_render_tick(alpha);
        let server_dt = ((s.clock().server_time() - snap.tick as f64).clamp(0.0, 8.0)
            + alpha as f64)
            / HZ as f64;
        let mut out = Vec::with_capacity(snap.projectiles.len() + self.shots.pending().len());
        for p in &snap.projectiles {
            match local_t {
                Some(t) if p.owner == me => {
                    let ext = (t - snap.tick as f64).clamp(0.0, MAX_OWN_EXTRAPOLATION_TICKS);
                    let step = p.vel * (ext / HZ as f64) as f32;
                    let len = step.length();
                    if len > 0.0 && s.terrain().raycast(p.pos, step, len).is_some() {
                        continue;
                    }
                    out.push(ProjectileView {
                        kind: p.kind,
                        owner: p.owner,
                        pos: p.pos + step + self.shots.offset(p.id),
                        vel: p.vel,
                        visual: 0,
                        state: p.state,
                    });
                }
                _ => out.push(ProjectileView {
                    kind: p.kind,
                    owner: p.owner,
                    pos: p.pos + p.vel * server_dt as f32,
                    vel: p.vel,
                    visual: 0,
                    state: p.state,
                }),
            }
        }
        if let Some(t) = local_t {
            out.extend(
                self.shots
                    .pending()
                    .iter()
                    .filter(|shot| shot.visible_at(t))
                    .map(|shot| ProjectileView {
                        kind: shot.weapon.0,
                        owner: me,
                        pos: shot.pos_at(t),
                        vel: shot.vel,
                        visual: 0,
                        state: 0,
                    }),
            );
        }
        out
    }
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
    ships: ShipTable,
    gadgets: GadgetTable,
    /// The match's visuals (derived from the data on every peer).
    visuals: VisualTable,
    /// Weapon id + 1 picked by the player and not yet sent with a tick
    /// (0 = none): a key press on a frame that runs no tick is not lost.
    pending_select: u8,
    /// Game cues since the host last took them.
    cues: Vec<CueView>,
    /// Ship type chosen for the next respawn (online: sent with
    /// `SelectShip`), until the ship flies it.
    next_ship: Option<ShipType>,
    /// Online: server tick from which the gadget is ready again (estimated
    /// from the own gadget cues and the gadget's cooldown).
    gadget_ready_tick: u64,
    /// Online: the own Paint Thrower colour (`ls_math::terrain::PAINT_*`),
    /// from the server's `PaintColor` cues, guessed ahead on a V press.
    paint_color: u8,
    /// Edge buttons pressed on frames that ran no tick (see [`EDGE_BUTTONS`]).
    pending_edges: u16,
    /// Buttons of the last command sent with a tick.
    last_buttons: u16,
    /// Online: first input tick of the newest V press guessing the next
    /// paint colour; inventories older than it keep the guess.
    paint_guess_tick: Option<u64>,
    /// Match transitions (new `Welcome`s on the open connection) handled.
    transitions: u64,
    /// A transition the host has not taken yet.
    transition_pending: bool,
    /// Chunks of the previous match's level the host still shows: reported
    /// by [`GameCore::take_unloads`] after a transition.
    unload_backlog: Vec<ChunkCoord>,
    /// Online: the ship the locally drawn Leech Arc beam is locked on
    /// ([`GameCore::local_beam`]).
    beam_lock: Option<u8>,
}

/// What the online core derives from a session's rule set: the resolved
/// rules, the loadout inventory and a fresh shot predictor (for every
/// `Welcome`).
fn online_parts(
    session: &ClientSession,
    data: &GameData,
) -> Result<(ResolvedRules, Inventory, ShotPredictor), String> {
    let resolved = session
        .rules()
        .resolve_with(&data.weapons, &data.gadgets)
        .map_err(|e| e.to_string())?;
    let slots = data.ships.get(session.ship_type()).secondary_slots;
    let inventory = Inventory::loadout(&resolved, slots);
    let shots = ShotPredictor::new(&resolved);
    Ok((resolved, inventory, shots))
}

/// Paint colour (`ls_math::terrain::PAINT_*`) of a Paint Thrower state byte
/// (unset = red, like `paint_thrower::selected_color`).
fn paint_color_of_state(state: u8) -> u8 {
    match state {
        c @ 1..=3 => c,
        _ => DEFAULT_PAINT_COLOR,
    }
}

impl GameCore {
    // ----- construction ----------------------------------------------------

    /// Online session from the server's first packet (`Welcome`; a `Reject`
    /// or anything else becomes an error with the reason).
    pub fn online(welcome_packet: &[u8]) -> Result<GameCore, String> {
        let msg = Message::decode(welcome_packet).map_err(|e| format!("bad welcome: {e}"))?;
        let welcome = check_welcome(msg).map_err(|e| e.to_string())?;
        // Built-in data: the session checks the Welcome's data hash.
        let data = GameData::builtin();
        let session =
            ClientSession::from_welcome_with(&welcome, data.clone()).map_err(|e| e.to_string())?;
        let (resolved, inventory, shots) = online_parts(&session, &data)?;
        let GameData {
            weapons,
            ships,
            gadgets,
        } = data;
        let streamer = ChunkStreamer::new(session.terrain().chunk_coords());
        let visuals = VisualTable::new(&weapons, &gadgets);
        Ok(GameCore {
            mode: Mode::Online(Box::new(Online {
                session,
                inventory,
                select_tick: None,
                match_over: None,
                resolved,
                shots,
                tally: BTreeMap::new(),
                local_ms: 0.0,
                host_clock: false,
                pings: VecDeque::new(),
                ping_ms: None,
                ended_at: None,
            })),
            step: FixedStep::default(),
            look: LookAccumulator::default(),
            streamer,
            feed: Vec::new(),
            weapons,
            ships,
            gadgets,
            visuals,
            pending_select: 0,
            cues: Vec::new(),
            next_ship: None,
            gadget_ready_tick: 0,
            paint_color: DEFAULT_PAINT_COLOR,
            pending_edges: 0,
            last_buttons: 0,
            paint_guess_tick: None,
            transitions: 0,
            transition_pending: false,
            unload_backlog: Vec::new(),
            beam_lock: None,
        })
    }

    /// Offline practice on the level of `code` with `dummies` idle opponents.
    pub fn practice(code: &str, dummies: u8) -> Result<GameCore, String> {
        Self::practice_as(code, dummies, ShipType::Tinker)
    }

    /// [`GameCore::practice`] flying `ship` (if the rule set allows it).
    pub fn practice_as(code: &str, dummies: u8, ship: ShipType) -> Result<GameCore, String> {
        let code = ShareCode::decode(code).map_err(|e| e.to_string())?;
        let gen = Generator::from_code(&code).map_err(|e| e.to_string())?;
        Self::practice_with_generator_as(gen, dummies, true, ship).map_err(|e| e.to_string())
    }

    /// A practice level without dummies whose chunks are generated lazily
    /// (see `Game::practicePreview`).
    pub fn practice_preview(code: &str) -> Result<GameCore, String> {
        let code = ShareCode::decode(code).map_err(|e| e.to_string())?;
        let gen = Generator::from_code(&code).map_err(|e| e.to_string())?;
        Self::practice_with_generator_as(gen, 0, false, ShipType::Tinker).map_err(|e| e.to_string())
    }

    /// Practice on any generator (tests use small levels). `pregenerate`
    /// builds every chunk up front, so the first paint score update (tick 30)
    /// does not stall a frame.
    pub fn practice_with_generator(
        gen: Generator,
        dummies: u8,
        pregenerate: bool,
    ) -> Result<GameCore, GameError> {
        Self::practice_with_generator_as(gen, dummies, pregenerate, ShipType::Tinker)
    }

    /// [`GameCore::practice_with_generator`] flying `ship` (the Tinker or
    /// the first allowed type if the rule set does not allow it).
    pub fn practice_with_generator_as(
        gen: Generator,
        dummies: u8,
        pregenerate: bool,
        ship: ShipType,
    ) -> Result<GameCore, GameError> {
        let code = gen.code();
        let cfg = MatchConfig::paint_the_void(code);
        let mut terrain = GameTerrain::new(gen);
        let setup = match_setup(&cfg.code, &cfg.rules, &terrain)?;
        let (well_sites, belt) = (setup.wells.clone(), setup.belt.clone());
        let mut world = World::new(cfg.world_config(setup))?;
        world.set_nav_graph(ls_game::setup::nav_graph(&terrain.generator()));
        if pregenerate {
            terrain.generate_all();
        }
        let ship = allowed_or_fallback(world.resolved_rules(), ship);
        let me = world.add_player(Some(0), ship)?;
        for _ in 0..dummies.min(15) {
            world.add_player(Some(1), ShipType::Tinker)?;
        }
        let prev = world.ships().cloned().collect();
        let streamer = ChunkStreamer::new(terrain.chunk_coords());
        let visuals = world.visuals().clone();
        Ok(GameCore {
            mode: Mode::Practice(Box::new(Practice {
                world,
                terrain,
                me,
                prev,
                over_secs: 0.0,
                well_sites,
                belt,
            })),
            step: FixedStep::default(),
            look: LookAccumulator::default(),
            streamer,
            feed: Vec::new(),
            weapons: cfg.weapons,
            ships: cfg.ships,
            gadgets: cfg.gadgets,
            visuals,
            pending_select: 0,
            cues: Vec::new(),
            next_ship: None,
            gadget_ready_tick: 0,
            paint_color: DEFAULT_PAINT_COLOR,
            pending_edges: 0,
            last_buttons: 0,
            paint_guess_tick: None,
            transitions: 0,
            transition_pending: false,
            unload_backlog: Vec::new(),
            beam_lock: None,
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

    /// Gadget table (ids are `PlayerInfo::gadget`).
    pub fn gadgets(&self) -> &GadgetTable {
        &self.gadgets
    }

    /// The match's visual table (ids are `EntityDraw::visual` and
    /// `ProjectileView::visual`).
    pub fn visuals(&self) -> &VisualTable {
        &self.visuals
    }

    /// Ship type of the local ship (this life).
    pub fn ship_type(&self) -> ShipType {
        match &self.mode {
            Mode::Online(o) => o.session.ship_type(),
            Mode::Practice(p) => p.world.ship(p.me).map_or(ShipType::Tinker, |s| s.ship_type),
        }
    }

    /// Tuning of the local ship's type.
    pub fn tuning(&self) -> &ShipTuning {
        self.ships.get(self.ship_type())
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

    /// Practice: generates the missing chunk nearest `near` and returns how
    /// many are left (0 when done, and online).
    pub fn generate_nearest(&mut self, near: Vec3) -> usize {
        match &mut self.mode {
            Mode::Online(_) => 0,
            Mode::Practice(p) => p.terrain.generate_nearest(near),
        }
    }

    /// Distance to the first rock along a ray (VQ1 scale cues; see
    /// [`crate::probe::raycast`]).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<f32> {
        crate::probe::raycast(self.terrain(), origin, dir, max)
    }

    /// Where a sphere swept along a ray first touches rock (VQ1 fit ring;
    /// see [`crate::probe::sweep_sphere`]).
    pub fn sweep_sphere(
        &self,
        origin: Vec3,
        dir: Vec3,
        radius: f32,
        max: f32,
    ) -> Option<crate::probe::SweepHit> {
        crate::probe::sweep_sphere(self.terrain(), origin, dir, radius, max)
    }

    /// Practice only: applies a terrain edit outside the simulation (the
    /// visual gallery carves fixed test shapes). False online.
    pub fn practice_edit(&mut self, edit: ls_math::terrain::TerrainEdit) -> bool {
        match &mut self.mode {
            Mode::Online(_) => false,
            Mode::Practice(p) => {
                p.terrain.apply(edit);
                true
            }
        }
    }

    /// Practice only: sets the terrain's shell layer (see `Game::practice_shell`).
    pub fn practice_shell(&mut self, shell: Option<ls_sim::ShellSphere>) -> bool {
        match &mut self.mode {
            Mode::Online(_) => false,
            Mode::Practice(p) => {
                p.terrain.set_shell(shell);
                true
            }
        }
    }

    // ----- network -------------------------------------------------------------

    /// Handles one packet from the server. Returns the reason if the server
    /// ended the session. Practice mode ignores packets.
    pub fn handle_packet(&mut self, bytes: &[u8]) -> Result<Option<String>, String> {
        let Mode::Online(o) = &mut self.mode else {
            return Ok(None);
        };
        let incoming = o.session.handle_packet(bytes).map_err(|e| e.to_string())?;
        match &incoming {
            ls_game::client::Incoming::Snapshot { stale: false, .. } => o.reconcile_shots(),
            ls_game::client::Incoming::Inventory => {
                o.apply_inventory(&self.weapons);
                // The Paint Thrower colour is the server's weapon state, so a
                // client that missed the colour cues (a rejoin, a transition)
                // shows the right one; a V press the server has not
                // processed yet keeps its guess.
                let pt = self.weapons.find("paint_thrower");
                if let (Some(pt), Some(msg)) = (pt, o.session.inventory()) {
                    let fresh = self.paint_guess_tick.is_none_or(|g| msg.tick as u64 >= g);
                    if let Some(x) = msg.weapons.iter().find(|x| x.weapon == pt.0) {
                        if fresh {
                            self.paint_color = paint_color_of_state(x.state);
                            self.paint_guess_tick = None;
                        }
                    }
                }
            }
            ls_game::client::Incoming::Pong { nonce, .. } => o.on_pong(*nonce),
            ls_game::client::Incoming::MatchTransition { .. } => self.on_match_transition()?,
            _ => {}
        }
        self.collect_online_events();
        self.pull_dirty();
        Ok(match incoming {
            ls_game::client::Incoming::Disconnected(reason) => Some(reason),
            _ => None,
        })
    }

    /// Sets the local clock (ms, monotonic, e.g. `performance.now()`) used to
    /// time pings. Call before handling the packets drained this frame.
    pub fn set_clock_ms(&mut self, ms: f64) {
        if let Mode::Online(o) = &mut self.mode {
            if ms.is_finite() {
                o.local_ms = ms;
                o.host_clock = true;
            }
        }
    }

    // ----- per frame -----------------------------------------------------------

    /// Advances local time by `dt` seconds with this frame's controls.
    /// Returns the packets to send to the server (in order; empty offline).
    pub fn frame(&mut self, dt: f64, controls: Controls) -> Vec<Vec<u8>> {
        let due = self.step.advance(dt);
        let tuning = self.tuning().clone();
        let max_turn = tuning.max_turn_rate * ls_math::DT;
        let mut controls = self.apply_selection(controls);
        let paint_thrower = self.weapons.find("paint_thrower");
        let mut out = Vec::new();
        match &mut self.mode {
            Mode::Online(o) => {
                // The clock may run more ticks than are due (catching up) or
                // fewer (skipping); the frame's look is spread over the ticks
                // that actually run, so it is applied exactly once.
                let n = o.session.plan_ticks(due);
                latch_edges(&mut self.pending_edges, &mut controls, n);
                let cmd = self.look.command(controls, n, max_turn);
                if n > 0 {
                    // V with the Paint Thrower selected: guess the next colour
                    // until the server's `PaintColor` cue confirms it.
                    let mode = ls_sim::Buttons::WEAPON_MODE;
                    if cmd.buttons.contains(mode)
                        && !ls_sim::Buttons(self.last_buttons).contains(mode)
                        && paint_thrower.is_some()
                        && o.inventory.primary == paint_thrower
                    {
                        self.paint_color = next_paint_color(self.paint_color);
                        self.paint_guess_tick = Some(o.session.next_input_tick());
                    }
                    self.last_buttons = cmd.buttons.0;
                }
                let first = o.session.next_input_tick();
                let synced = o.session.is_synced();
                let msgs = o.session.run_ticks(n, due, cmd);
                if n > 0 && cmd.weapon_select != 0 {
                    o.inventory
                        .select(WeaponId(cmd.weapon_select - 1), &self.weapons);
                    o.select_tick = Some(first);
                    self.pending_select = 0;
                }
                if !o.host_clock {
                    o.local_ms += dt.max(0.0) * 1000.0;
                }
                if synced {
                    o.predict_shots(first, &cmd, &self.weapons, &tuning);
                }
                o.shots.decay(n);
                for msg in msgs {
                    if let Message::Ping { nonce, .. } = &msg {
                        o.on_ping_sent(*nonce);
                    }
                    match msg.encode() {
                        Ok(bytes) => out.push(bytes),
                        Err(e) => push_feed(&mut self.feed, format!("Could not send input: {e}")),
                    }
                }
                self.collect_online_events();
            }
            Mode::Practice(p) => {
                latch_edges(&mut self.pending_edges, &mut controls, due);
                let cmd = self.look.command(controls, due, max_turn);
                if due > 0 {
                    self.last_buttons = cmd.buttons.0;
                }
                if due > 0 {
                    self.pending_select = 0;
                }
                let mut events = Vec::new();
                for _ in 0..due {
                    p.prev = p.world.ships().cloned().collect();
                    let tick_cmd = ls_sim::InputCmd {
                        tick: p.world.tick() + 1,
                        ..cmd
                    };
                    events.extend(p.world.step(&mut p.terrain, &[(p.me, tick_cmd)]));
                }
                if !p.world.mode().is_running() {
                    p.over_secs += dt.max(0.0) as f32;
                }
                // Hits on ships as client cues (particle effects), the same
                // way the server reports them online (`NetEvent::Hit`).
                let tick = match &self.mode {
                    Mode::Practice(p) => p.world.tick(),
                    Mode::Online(_) => 0,
                };
                for h in
                    ls_game::convert::events_to_net(&events, tick, |p| Some(p.0.min(254) as u8))
                {
                    if let NetEvent::Hit {
                        tick,
                        attacker,
                        victim,
                        damage,
                        pos,
                        ..
                    } = h
                    {
                        push_cue(&mut self.cues, hit_cue(attacker, victim, damage, pos, tick));
                    }
                }
                for e in events {
                    self.practice_event(e);
                }
            }
        }
        self.pull_dirty();
        out
    }

    /// Resolves this frame's number key, wheel step or X press against the
    /// carried weapons into a pending weapon selection, and puts the pending
    /// selection into the controls (a direct `weapon_select` also counts).
    fn apply_selection(&mut self, mut controls: Controls) -> Controls {
        if controls.weapon_select != 0 {
            self.pending_select = controls.weapon_select;
        }
        let req = controls.select_request();
        if !req.is_empty() {
            let view = self.loadout();
            let mut primary = view.primaries.iter().find(|e| e.selected).map(|e| e.weapon);
            let mut secondary = view
                .secondaries
                .iter()
                .find(|e| e.selected)
                .map(|e| e.weapon);
            if let Some(w) = self.pending_select.checked_sub(1).map(WeaponId) {
                if (w.0 as usize) < self.weapons.len() {
                    match self.weapons.get(w).slot {
                        WeaponSlot::Primary => primary = Some(w),
                        WeaponSlot::Secondary => secondary = Some(w),
                    }
                }
            }
            if let Some(w) = resolve_select(
                req,
                &view.primary_ids(),
                primary,
                &view.secondary_ids(),
                secondary,
            ) {
                self.pending_select = w.0 + 1;
            }
        }
        controls.weapon_select = self.pending_select;
        controls
    }

    fn pull_dirty(&mut self) {
        // Surface states (hot, charged, glow) expire: move the meshes' display
        // tick to the match time so expired ones are remeshed away.
        match &mut self.mode {
            Mode::Online(o) => {
                let now = o.session.clock().server_time().max(0.0) as u64;
                o.session.terrain_mut().advance_to(now);
            }
            Mode::Practice(p) => {
                let now = p.world.tick();
                p.terrain.advance_to(now);
            }
        }
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
                let players = o.session.players();
                o.session
                    .remote_ships_at(t)
                    .iter()
                    .map(|s| net_ship_view(s, players, &self.ships))
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
                        ship_type: cur.ship_type.index(),
                        radius: self.ships.get(cur.ship_type).radius,
                        gadget_active: cur.gadget_active,
                    }
                })
                .collect(),
        }
    }

    /// Projectiles moved along their velocity to the current render time.
    pub fn projectiles(&self) -> Vec<ProjectileView> {
        let alpha = self.step.alpha();
        let mut out = match &self.mode {
            Mode::Online(o) => o.projectiles(alpha),
            Mode::Practice(p) => p
                .world
                .projectiles()
                .iter()
                .map(|pr| ProjectileView {
                    kind: pr.weapon.0,
                    owner: pr.owner.0.min(254) as u8,
                    // One tick behind like the ships, then forward by alpha.
                    pos: pr.pos + pr.vel * ((alpha - 1.0) / HZ),
                    vel: pr.vel,
                    visual: 0,
                    state: (pr.flags & 0xff) as u8,
                })
                .collect(),
        };
        // Predicted own Paint Thrower blobs carry no state yet: give them the
        // colour the server's blobs carry in their custom bits.
        let paint = self.weapons.find("paint_thrower").map(|w| w.0);
        let me = self.player();
        for p in &mut out {
            p.visual = self.visuals.projectile(WeaponId(p.kind)).0;
            if Some(p.kind) == paint && p.owner == me && p.state >> 6 == 0 {
                p.state |= (self.paint_color & 3) << 6;
            }
        }
        out
    }

    /// Replicated entities (pickups, scrap piles, wrecks, debris, decoys,
    /// ghosts, dust clouds, markers) at the render time: online the newest
    /// snapshot moved along the velocities to the estimated server time, in
    /// practice between the last two ticks like the ships.
    pub fn entities(&self) -> Vec<EntityDraw> {
        match &self.mode {
            // Well entities carry their start tick in `visual`; wells are
            // drawn from `wells()` instead (practice has no well entities).
            Mode::Online(o) => o
                .session
                .entities()
                .iter()
                .filter(|e| e.kind != ls_sim::ReplicatedKind::Well as u8)
                .map(EntityDraw::from_net)
                .collect(),
            Mode::Practice(p) => {
                let back = (self.step.alpha() - 1.0) / HZ;
                p.world
                    .entity_views()
                    .iter()
                    .map(|v| {
                        let radius = p
                            .world
                            .entities()
                            .iter()
                            .find(|e| e.id == v.id)
                            .map_or(0.0, |e| e.radius);
                        let mut d = EntityDraw::from_view(v, radius);
                        d.pos += v.vel * back;
                        d
                    })
                    .collect()
            }
        }
    }

    /// Every player (the local one included): names, teams, ship types.
    pub fn players(&self) -> Vec<PlayerInfo> {
        match &self.mode {
            Mode::Online(o) => {
                let s = &o.session;
                let mut out: Vec<PlayerInfo> = s.players().iter().map(player_info).collect();
                if !out.iter().any(|p| p.id == s.player()) {
                    out.push(PlayerInfo {
                        id: s.player(),
                        team: s.team(),
                        ship_type: s.ship_type().index(),
                        gadget: u8::MAX,
                        name: "You".into(),
                    });
                }
                out
            }
            Mode::Practice(p) => p
                .world
                .ships()
                .map(|s| PlayerInfo {
                    id: s.player.0.min(254) as u8,
                    team: s.team,
                    ship_type: s.ship_type.index(),
                    gadget: s.gadget.map_or(u8::MAX, |g| g.0),
                    name: practice_name(s.player, p.me),
                })
                .collect(),
        }
    }

    /// Carried weapons and the gadget for the weapon HUD (online: the
    /// client's mirror, see [`crate::inventory`]).
    pub fn loadout(&self) -> LoadoutView {
        match &self.mode {
            Mode::Online(o) => {
                let s = &o.session;
                let own = s
                    .latest_snapshot()
                    .and_then(|snap| snap.ships.iter().find(|x| x.id == s.player()));
                let (energy, scrap) = own.and_then(|x| x.status).map_or((0.0, 0.0), |st| {
                    let (_, energy, _, scrap) = s.conv().status_values(&st);
                    (energy, scrap)
                });
                let inv = &o.inventory;
                let mut view = LoadoutView::build(
                    &self.weapons,
                    &inv.states(),
                    inv.primary,
                    inv.secondary,
                    |def, n| ammo_text(def, energy, scrap, Some(n)),
                );
                let gadget = s
                    .players()
                    .iter()
                    .find(|p| p.id == s.player())
                    .map_or(inv.gadget.map(|g| g.0), |p| {
                        (p.gadget != u8::MAX).then_some(p.gadget)
                    });
                view.gadget = self.gadget_name(gadget);
                view.gadget_active = own.is_some_and(|x| x.flags & ship_flags::GADGET_ACTIVE != 0);
                view.set_weapon_states(|w| inv.weapon_state(w));
                view
            }
            Mode::Practice(p) => {
                let Some(ship) = p.world.ship(p.me) else {
                    return LoadoutView::default();
                };
                let mut view = LoadoutView::build(
                    &self.weapons,
                    &ship.weapons,
                    ship.primary,
                    ship.secondary,
                    |def, n| ammo_text(def, ship.energy, ship.scrap, Some(n)),
                );
                view.gadget = self.gadget_name(ship.gadget.map(|g| g.0));
                view.gadget_active = ship.gadget_active;
                view.set_weapon_states(|w| ship.weapon_state_of(w) as u8);
                view
            }
        }
    }

    fn gadget_name(&self, g: Option<u8>) -> String {
        g.filter(|&g| (g as usize) < self.gadgets.len())
            .map_or_else(String::new, |g| {
                self.gadgets.get(ls_sim::GadgetId(g)).name.clone()
            })
    }

    /// HUD values.
    pub fn hud(&self) -> HudState {
        let t = self.tuning();
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
        let inv = &o.inventory;
        let primary = inv
            .primary
            .or_else(|| own.map(|o| WeaponId(ship_flags::weapon(o.flags))));
        if let Some(w) = primary.filter(|w| (w.0 as usize) < self.weapons.len()) {
            let def = self.weapons.get(w);
            hud.weapon = def.name.clone();
            hud.ammo = ammo_text(def, hud.energy, hud.scrap, Some(inv.ammo(w)));
        }
        if let Some(w) = inv
            .secondary
            .filter(|w| (w.0 as usize) < self.weapons.len())
        {
            hud.secondary = format!("{} ×{}", self.weapons.get(w).name, inv.ammo(w));
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

    /// Counters for the debug overlay (prediction and ping are online only).
    pub fn debug_stats(&self) -> DebugStats {
        match &self.mode {
            Mode::Online(o) => {
                let p = o.session.prediction_stats();
                DebugStats {
                    ping_ms: o.ping_ms,
                    accepted: p.accepted,
                    corrected: p.corrected,
                    snapped: p.snapped,
                    last_error: p.last_error,
                    shots: o.shots.stats(),
                    synced: o.session.is_synced(),
                }
            }
            Mode::Practice(_) => DebugStats {
                synced: true,
                ..DebugStats::default()
            },
        }
    }

    /// Scoreboard rows sorted by team, then kills (most first), deaths
    /// (fewest first) and id.
    pub fn scoreboard(&self) -> Vec<ScoreRow> {
        let mut rows: Vec<ScoreRow> = match &self.mode {
            Mode::Online(o) => {
                let me = o.session.player();
                let snap_ships = o
                    .session
                    .latest_snapshot()
                    .map_or(&[][..], |s| &s.ships[..]);
                let mut rows: Vec<ScoreRow> = snap_ships
                    .iter()
                    .map(|s| {
                        let (kills, deaths) = o.tally.get(&s.id).copied().unwrap_or_default();
                        ScoreRow {
                            id: s.id,
                            name: online_name(&o.session, s.id),
                            team: o.session.side_of(s.id).unwrap_or(ship_flags::team(s.flags)),
                            alive: s.flags & ship_flags::ALIVE != 0,
                            kills,
                            deaths,
                            is_you: s.id == me,
                        }
                    })
                    .collect();
                if !rows.iter().any(|r| r.is_you) {
                    let (kills, deaths) = o.tally.get(&me).copied().unwrap_or_default();
                    rows.push(ScoreRow {
                        id: me,
                        name: online_name(&o.session, me),
                        team: o.session.team(),
                        alive: true,
                        kills,
                        deaths,
                        is_you: true,
                    });
                }
                rows
            }
            Mode::Practice(p) => p
                .world
                .ships()
                .map(|s| ScoreRow {
                    id: s.player.0.min(254) as u8,
                    name: practice_name(s.player, p.me),
                    team: s.team,
                    alive: s.alive,
                    kills: s.kills,
                    deaths: s.deaths,
                    is_you: s.player == p.me,
                })
                .collect(),
        };
        rows.sort_by(|a, b| {
            (a.team, std::cmp::Reverse(a.kills), a.deaths, a.id).cmp(&(
                b.team,
                std::cmp::Reverse(b.kills),
                b.deaths,
                b.id,
            ))
        });
        rows
    }

    /// Match result and paint cells for the end screen.
    pub fn match_info(&self) -> MatchInfo {
        match &self.mode {
            Mode::Online(o) => {
                let s = &o.session;
                let Some(snap) = s.latest_snapshot() else {
                    return MatchInfo::default();
                };
                let (team_cells, surface_cells) = match snap.scores[..] {
                    [a, b, surface, ..] => ([a as u64, b as u64], surface as u64),
                    _ => ([0; 2], 0),
                };
                let over = o.match_over.is_some() || snap.match_ticks_left == 0;
                let winner = match o.match_over {
                    Some(w) => w,
                    None if over => leader(team_cells),
                    None => None,
                };
                let secs_since_end = match (over, o.ended_at.or(s.match_end_tick())) {
                    (true, Some(end)) => {
                        ((s.clock().server_time() - end as f64).max(0.0) / HZ as f64) as f32
                    }
                    _ => 0.0,
                };
                MatchInfo {
                    over,
                    winner,
                    team_cells,
                    surface_cells,
                    secs_since_end,
                }
            }
            Mode::Practice(p) => {
                let mode = p.world.mode();
                let over = !mode.is_running();
                MatchInfo {
                    over,
                    winner: if over { mode.leader() } else { None },
                    team_cells: [mode.coverage.team_cells[0], mode.coverage.team_cells[1]],
                    surface_cells: mode.coverage.surface_cells,
                    secs_since_end: p.over_secs,
                }
            }
        }
    }

    /// Name for the kill feed ("You" for the local player).
    fn name(&self, id: u8) -> String {
        if id == self.player() {
            "You".into()
        } else if id == u8::MAX {
            "The level".into()
        } else {
            match &self.mode {
                Mode::Online(o) => online_name(&o.session, id),
                Mode::Practice(p) => practice_name(PlayerId(id as u16), p.me),
            }
        }
    }

    fn collect_online_events(&mut self) {
        let events = match &mut self.mode {
            Mode::Online(o) => o.session.take_events(),
            Mode::Practice(_) => return,
        };
        for e in events {
            let line = match e {
                NetEvent::Cue {
                    tick,
                    kind,
                    source,
                    target,
                    pos,
                    value,
                } => {
                    let cue = CueView {
                        kind,
                        source,
                        target,
                        pos,
                        value,
                        tick: tick as u64,
                    };
                    self.on_cue(cue);
                    None
                }
                NetEvent::Kill { killer, victim, .. } => {
                    if let Mode::Online(o) = &mut self.mode {
                        o.tally.entry(victim).or_default().1 += 1;
                        if killer != u8::MAX && killer != victim {
                            o.tally.entry(killer).or_default().0 += 1;
                        }
                    }
                    Some(kill_line(
                        (killer != u8::MAX).then(|| self.name(killer)),
                        self.name(victim),
                    ))
                }
                NetEvent::MatchOver { winner_team, .. } => {
                    if let Mode::Online(o) = &mut self.mode {
                        o.match_over = Some(winner_team);
                    }
                    Some(match_over_text(winner_team, self.team()))
                }
                NetEvent::Respawn { ship, .. } => {
                    // The server clears the Paint Thrower colour on respawn.
                    if ship == self.player() {
                        self.paint_color = DEFAULT_PAINT_COLOR;
                    }
                    None
                }
                NetEvent::Hit {
                    tick,
                    attacker,
                    victim,
                    damage,
                    pos,
                    ..
                } => {
                    push_cue(&mut self.cues, hit_cue(attacker, victim, damage, pos, tick));
                    None
                }
            };
            if let Some(l) = line {
                push_feed(&mut self.feed, l);
            }
        }
    }

    fn practice_event(&mut self, e: Event) {
        let line = match e {
            Event::Cue(c) => {
                let tick = match &self.mode {
                    Mode::Practice(p) => p.world.tick(),
                    Mode::Online(_) => 0,
                };
                self.on_cue(CueView::from_sim(&c, tick));
                None
            }
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

    // ----- mechanics UI ------------------------------------------------------------

    /// Records a cue for the host; the own gadget cues also restart the
    /// gadget cooldown estimate (online).
    fn on_cue(&mut self, mut cue: CueView) {
        // A charge start carries the charging weapon in `target` (the sim
        // leaves it empty), so the host can colour the charge-up effect.
        if cue.kind() == ls_sim::CueKind::ChargeStart && cue.target == u8::MAX {
            cue.target = self.selected_primary_of(cue.source).unwrap_or(u8::MAX);
        }
        // Online, the own Leech Arc beam is drawn locally (`local_beam`):
        // the server's beam cue only corrects its lock.
        if self.is_online() && cue.kind() == ls_sim::CueKind::Beam && cue.source == self.player() {
            self.beam_lock = (cue.target != u8::MAX).then_some(cue.target);
            return;
        }
        // Borrowed Time: the rewound ship's weapons go offline for the
        // weapon's `offline_secs` (the cue's value is the rewind).
        if cue.kind() == ls_sim::CueKind::Rewound && cue.target == self.player() {
            let offline = self
                .weapons
                .find("borrowed_time")
                .map_or(u64::from(cue.value), |w| {
                    let secs = self.weapons.get(w).param("offline_secs", 3.0);
                    ls_math::Tick::ticks_for_secs(secs)
                });
            if let Mode::Online(o) = &mut self.mode {
                o.shots.set_offline_until(cue.tick + offline);
            }
        }
        // The cooldown starts when a toggle gadget is switched off by its
        // key or an instant gadget is used (not when a toggle runs out of
        // energy); see `ls_sim::gadget_cue`.
        if cue.kind() == ls_sim::CueKind::Gadget
            && cue.source == self.player()
            && ls_sim::gadget_cue::starts_cooldown(cue.value)
        {
            let g = self
                .own_gadget()
                .filter(|&g| (g as usize) < self.gadgets.len());
            if let Some(g) = g {
                let def = self.gadgets.get(ls_sim::GadgetId(g));
                self.gadget_ready_tick =
                    cue.tick + ls_math::Tick::ticks_for_secs(def.cooldown_secs);
            }
        }
        if cue.kind() == ls_sim::CueKind::PaintColor && cue.source == self.player() {
            if let Ok(c @ 1..=3) = u8::try_from(cue.value) {
                self.paint_color = c;
            }
        }
        push_cue(&mut self.cues, cue);
    }

    /// The weapon a ship is charging or has selected as its primary (online:
    /// the newest snapshot's ship flags; practice: the world's ship).
    fn selected_primary_of(&self, id: u8) -> Option<u8> {
        match &self.mode {
            Mode::Online(o) => {
                if id == o.session.player() {
                    if let Some(c) = o.shots.charge() {
                        return Some(c.weapon.0);
                    }
                }
                let snap = o.session.latest_snapshot()?;
                let ship = snap.ships.iter().find(|s| s.id == id)?;
                Some(ship_flags::weapon(ship.flags))
            }
            Mode::Practice(p) => {
                let ship = p.world.ship(ls_sim::PlayerId(u16::from(id)))?;
                ship.charge
                    .map(|c| c.weapon.0)
                    .or_else(|| ship.primary.map(|w| w.0))
            }
        }
    }

    /// Game cues since the last call (oldest first).
    pub fn take_cues(&mut self) -> Vec<CueView> {
        std::mem::take(&mut self.cues)
    }

    /// Current simulation tick (online: the estimated server time).
    pub fn now_tick(&self) -> f64 {
        match &self.mode {
            Mode::Online(o) => o.session.clock().server_time().max(0.0),
            Mode::Practice(p) => p.world.tick() as f64,
        }
    }

    /// The own gadget id, if any.
    fn own_gadget(&self) -> Option<u8> {
        match &self.mode {
            Mode::Online(o) => {
                let s = &o.session;
                s.players()
                    .iter()
                    .find(|p| p.id == s.player())
                    .map_or(o.inventory.gadget.map(|g| g.0), |p| {
                        (p.gadget != u8::MAX).then_some(p.gadget)
                    })
            }
            Mode::Practice(p) => p.world.ship(p.me).and_then(|s| s.gadget.map(|g| g.0)),
        }
    }

    /// Seconds until the own weapons come back online after a Borrowed Time
    /// rewind (0 when they are online).
    pub fn weapons_offline_secs(&self) -> f32 {
        let left = match &self.mode {
            Mode::Online(o) => o.shots.offline_until() as f64 - self.now_tick(),
            Mode::Practice(p) => p.world.ship(p.me).map_or(0.0, |s| {
                s.weapons_offline_until as f64 - p.world.tick() as f64
            }),
        };
        (left.max(0.0) / ls_math::TICK_HZ as f64) as f32
    }

    /// The own gadget for the HUD: name, on/off, cooldown left.
    pub fn gadget(&self) -> GadgetView {
        let hz = ls_math::TICK_HZ as f32;
        match &self.mode {
            Mode::Online(o) => {
                let active = o
                    .session
                    .latest_snapshot()
                    .and_then(|snap| snap.ships.iter().find(|x| x.id == o.session.player()))
                    .is_some_and(|x| x.flags & ship_flags::GADGET_ACTIVE != 0);
                let left = (self.gadget_ready_tick as f64 - self.now_tick()) as f32 / hz;
                GadgetView::new(&self.gadgets, self.own_gadget(), active, left)
            }
            Mode::Practice(p) => {
                let Some(ship) = p.world.ship(p.me) else {
                    return GadgetView::new(&self.gadgets, None, false, 0.0);
                };
                let left = ship.gadget_ready_tick.saturating_sub(p.world.tick()) as f32 / hz;
                GadgetView::new(
                    &self.gadgets,
                    ship.gadget.map(|g| g.0),
                    ship.gadget_active,
                    left,
                )
            }
        }
    }

    /// The own Leech Arc beam, predicted locally (online; practice draws
    /// the simulation's beam cues, which arrive at once): while the shot
    /// predictor fired an arc within the weapon's fire interval, the beam
    /// from the own ship to its lock or along the aim (see [`crate::beam`]).
    pub fn local_beam(&mut self) -> Option<LocalBeam> {
        let Mode::Online(o) = &self.mode else {
            return None;
        };
        let (tick, w) = o.shots.last_beam()?;
        let now = o.session.next_input_tick().saturating_sub(1);
        let def = self.weapons.get(w);
        // Arcs come every fire interval while the trigger is held; one
        // missed tick of slack keeps the beam steady between them.
        if now > tick + def.fire_interval_ticks() + 1 {
            self.beam_lock = None;
            return None;
        }
        let params = BeamParams::of(def);
        let pose = self.local_pose().filter(|p| p.alive)?;
        let ships = self.remote_ships();
        let terrain = self.terrain();
        let beam = aim_beam(
            pose.pos,
            pose.rot.forward(),
            self.team(),
            &ships,
            self.beam_lock,
            params,
            |a, b| {
                let to = b - a;
                crate::probe::raycast(terrain, a, to, to.length()).is_none()
            },
            |o, d, max| crate::probe::raycast(terrain, o, d, max),
        );
        self.beam_lock = beam.target;
        Some(beam)
    }

    /// The charge in progress of a hold-to-charge weapon, for the charge
    /// meter (online: mirrored from the local input by the shot predictor).
    pub fn charge(&self) -> Option<ChargeView> {
        match &self.mode {
            Mode::Online(o) => {
                let c = o.shots.charge()?;
                let now = o.session.next_input_tick().saturating_sub(1);
                ChargeView::new(&self.weapons, c.weapon, c.held(now))
            }
            Mode::Practice(p) => {
                let c = p.world.ship(p.me)?.charge?;
                ChargeView::new(
                    &self.weapons,
                    c.weapon,
                    p.world.tick().saturating_sub(c.start_tick),
                )
            }
        }
    }

    /// Resolved rules of the match.
    fn resolved(&self) -> &ResolvedRules {
        match &self.mode {
            Mode::Online(o) => &o.resolved,
            Mode::Practice(p) => p.world.resolved_rules(),
        }
    }

    /// The ship type chosen for the next respawn (`None` once the ship flies it).
    pub fn next_ship(&self) -> Option<ShipType> {
        self.next_ship.filter(|&t| t != self.ship_type())
    }

    /// Every ship type with its numbers, whether the rule set allows it and
    /// the loadout it spawns with (ship selection panel).
    pub fn ship_choices(&self) -> Vec<ShipChoice> {
        ship_choices(
            &self.ships,
            &self.weapons,
            &self.gadgets,
            self.resolved(),
            self.ship_type(),
            self.next_ship(),
        )
    }

    /// Picks the ship type for the next respawn. Online it returns the
    /// `SelectShip` packet to send reliably; types the rule set does not
    /// allow are refused.
    pub fn select_ship(&mut self, ship: ShipType) -> Result<Option<Vec<u8>>, String> {
        if !self.resolved().allowed_ship_types.contains(&ship) {
            return Err(format!(
                "the rule set does not allow the {}",
                crate::mechanics::ship_type_title(ship)
            ));
        }
        let packet = match &mut self.mode {
            Mode::Online(o) => Some(
                o.session
                    .select_ship(ship)
                    .encode()
                    .map_err(|e| e.to_string())?,
            ),
            Mode::Practice(p) => {
                p.world
                    .set_ship_type(p.me, ship)
                    .map_err(|e| e.to_string())?;
                None
            }
        };
        self.next_ship = Some(ship);
        Ok(packet)
    }

    /// The own Paint Thrower colour (`ls_math::terrain::PAINT_*`: 1 red,
    /// 2 blue, 3 yellow) while it is the selected primary, else 0. Practice
    /// reads the simulation's weapon state; online it follows the server's
    /// `PaintColor` cues (guessed ahead on a V press, red after a respawn).
    pub fn paint_color(&self) -> u8 {
        let Some(pt) = self.weapons.find("paint_thrower") else {
            return 0;
        };
        match &self.mode {
            Mode::Online(o) => {
                if o.inventory.primary == Some(pt) {
                    self.paint_color
                } else {
                    0
                }
            }
            Mode::Practice(p) => p
                .world
                .ship(p.me)
                .filter(|s| s.primary == Some(pt))
                .map_or(0, |s| {
                    ls_sim::weapons::specials::paint_thrower::selected_color(s, pt)
                }),
        }
    }

    // ----- modes, lobby, transitions -------------------------------------------

    /// Rebuilds the online core after the session moved to the next match:
    /// new inventory, shot predictor, chunk streamer and per-match HUD state;
    /// ping state carries over. The old level's chunks go to the unload
    /// backlog.
    fn on_match_transition(&mut self) -> Result<(), String> {
        let data = GameData {
            weapons: self.weapons.clone(),
            ships: self.ships.clone(),
            gadgets: self.gadgets.clone(),
        };
        let Mode::Online(o) = &mut self.mode else {
            return Ok(());
        };
        let (resolved, inventory, shots) = online_parts(&o.session, &data)?;
        o.resolved = resolved;
        o.inventory = inventory;
        o.shots = shots;
        o.select_tick = None;
        o.match_over = None;
        o.tally.clear();
        o.ended_at = None;
        self.unload_backlog = self.streamer.loaded();
        let radius = self.streamer.view_radius();
        self.streamer = ChunkStreamer::new(o.session.terrain().chunk_coords());
        self.streamer.set_view_radius(radius);
        self.cues.clear();
        self.next_ship = None;
        self.gadget_ready_tick = 0;
        self.paint_color = DEFAULT_PAINT_COLOR;
        self.paint_guess_tick = None;
        self.pending_select = 0;
        self.pending_edges = 0;
        self.last_buttons = 0;
        self.transitions += 1;
        self.transition_pending = true;
        Ok(())
    }

    /// True once after each match transition (the host rebuilds what it
    /// derived from the old match: meshes come back via
    /// [`GameCore::take_unloads`], HUD and scene state should reset).
    pub fn take_match_transition(&mut self) -> bool {
        std::mem::take(&mut self.transition_pending)
    }

    /// Match transitions handled since the session started.
    pub fn transitions(&self) -> u64 {
        self.transitions
    }

    /// The generic mode state (phase, timer, scores, lives, objectives):
    /// online the newest snapshot's, in practice the local world's (ids are
    /// player ids there).
    pub fn mode_state(&self) -> NetModeState {
        match &self.mode {
            Mode::Online(o) => o.session.mode().clone(),
            Mode::Practice(p) => {
                let view = p.world.mode_view();
                let mut m = ls_game::mode::mode_view_to_net(&view, |pl| u8::try_from(pl.0).ok());
                m.mood.up = p.world.up();
                m
            }
        }
    }

    /// The newest lobby state from the server (online only).
    pub fn lobby(&self) -> Option<&LobbyStateMsg> {
        match &self.mode {
            Mode::Online(o) => o.session.lobby(),
            Mode::Practice(_) => None,
        }
    }

    /// The results of the match once the server sent them (online only).
    pub fn results(&self) -> Option<&MatchResultsMsg> {
        match &self.mode {
            Mode::Online(o) => o.session.results(),
            Mode::Practice(_) => None,
        }
    }

    /// Code of the room this match runs in (empty: the public room or
    /// practice).
    pub fn room_code(&self) -> String {
        match &self.mode {
            Mode::Online(o) => o.session.room_code().to_string(),
            Mode::Practice(_) => String::new(),
        }
    }

    /// `Ready` for the next match (send reliably; `None` offline).
    pub fn ready_packet(&self, ready: bool) -> Option<Vec<u8>> {
        self.lobby_packet(Message::Ready { ready })
    }

    /// The level's static gravity well sites (M4).
    pub fn well_sites(&self) -> &[ls_math::level::WellSite] {
        match &self.mode {
            Mode::Online(o) => o.session.well_sites(),
            Mode::Practice(p) => &p.well_sites,
        }
    }

    /// The level's gravity wells now (static and dynamic; M4).
    pub fn wells(&self) -> ls_sim::WellField {
        match &self.mode {
            Mode::Online(o) => o.session.wells().clone(),
            Mode::Practice(p) => p.world.well_field().clone(),
        }
    }

    /// The match's pickup points in the world's pickup order (the spawn
    /// indices Mimic pickups names in its mood params; M4 C2).
    pub fn pickup_points(&self) -> &[Vec3] {
        match &self.mode {
            Mode::Online(o) => o.session.pickup_points(),
            Mode::Practice(p) => &p.world.setup().pickup_points,
        }
    }

    /// The level's up (Bored level turns it; M4).
    pub fn up(&self) -> Vec3 {
        match &self.mode {
            Mode::Online(o) => o.session.mood().up,
            Mode::Practice(p) => p.world.up(),
        }
    }

    /// Monochrome level (M4).
    pub fn monochrome(&self) -> bool {
        match &self.mode {
            Mode::Online(o) => o.session.monochrome(),
            Mode::Practice(p) => p.world.setup().monochrome,
        }
    }

    /// Radiation dose rate (hull/s) at `p` with the mode's scale (Belt Up:
    /// its rule set's `radiation_scale` while racing, 0 in the start lights
    /// and heat results): the world's in practice, the session's online.
    pub fn dose_rate_at(&self, p: Vec3) -> f32 {
        match &self.mode {
            Mode::Online(o) => o.session.dose_rate_at(p),
            Mode::Practice(pr) => pr.world.dose_rate(p),
        }
    }

    /// The asteroid belt of a belt level (M4).
    pub fn belt(&self) -> Option<&ls_math::level::BeltSite> {
        match &self.mode {
            Mode::Online(o) => o.session.belt(),
            Mode::Practice(p) => p.belt.as_ref(),
        }
    }

    /// `ArchitectAction` placing a stroke (online only; the server
    /// validates it).
    pub fn architect_action_packet(
        &self,
        stroke: u8,
        center: Vec3,
        radius: f32,
    ) -> Option<Vec<u8>> {
        self.lobby_packet(Message::ArchitectAction(ls_net::ArchitectActionMsg {
            stroke,
            center,
            radius,
        }))
    }

    /// Protocol 6 `ext` bits (`ls_net::ship_ext`: pigment, hats) of every
    /// ship this client knows, as `(id, ext)` (online: the newest snapshot).
    pub fn ship_ext(&self) -> Vec<(u8, u8)> {
        match &self.mode {
            Mode::Online(o) => o
                .session
                .latest_snapshot()
                .map(|s| s.ships.iter().map(|x| (x.id, x.ext)).collect())
                .unwrap_or_default(),
            Mode::Practice(p) => p
                .world
                .ships()
                .map(|s| {
                    (
                        s.player.0 as u8,
                        ls_game::convert::ship_ext(s, p.world.tick()),
                    )
                })
                .collect(),
        }
    }

    /// `PickTeam` for the next match (`None` team = auto-balance).
    pub fn pick_team_packet(&self, team: Option<u8>) -> Option<Vec<u8>> {
        self.lobby_packet(Message::PickTeam { team })
    }

    /// `Spectate` from the next match on (or play again).
    pub fn spectate_packet(&self, spectate: bool) -> Option<Vec<u8>> {
        self.lobby_packet(Message::Spectate { spectate })
    }

    fn lobby_packet(&self, msg: Message) -> Option<Vec<u8>> {
        match &self.mode {
            Mode::Online(_) => msg.encode().ok(),
            Mode::Practice(_) => None,
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
    /// After a match transition: first every chunk of the previous level.
    pub fn take_unloads(&mut self, cam: Vec3) -> Vec<ChunkCoord> {
        let mut out = std::mem::take(&mut self.unload_backlog);
        out.extend(self.streamer.take_unloads(cam));
        out
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

/// Winner by painted cells like `ls_sim::PaintTheVoid::leader` (`None` on a tie).
fn leader([a, b]: [u64; 2]) -> Option<u8> {
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => Some(0),
        std::cmp::Ordering::Less => Some(1),
        std::cmp::Ordering::Equal => None,
    }
}

fn net_ship_view(s: &NetShip, players: &[NetPlayer], ships: &ShipTable) -> ShipView {
    let ship_type = players
        .iter()
        .find(|p| p.id == s.id)
        .and_then(|p| ShipType::from_index(p.ship_type))
        .unwrap_or(ShipType::Tinker);
    ShipView {
        id: s.id,
        // Full side id from the players list (free-for-all sides exceed the
        // two team bits of the flags).
        team: players
            .iter()
            .find(|p| p.id == s.id)
            .map_or(ship_flags::team(s.flags), |p| p.team),
        alive: s.flags & ship_flags::ALIVE != 0,
        boosting: s.flags & ship_flags::BOOSTING != 0,
        pos: s.pos,
        rot: s.rot,
        ship_type: ship_type.index(),
        radius: ships.get(ship_type).radius,
        gadget_active: s.flags & ship_flags::GADGET_ACTIVE != 0,
    }
}

fn player_info(p: &NetPlayer) -> PlayerInfo {
    PlayerInfo {
        id: p.id,
        team: p.team,
        ship_type: p.ship_type,
        gadget: p.gadget,
        name: if p.name.is_empty() {
            fallback_name(p.id)
        } else {
            p.name.clone()
        },
    }
}

/// Name for ids without a known name.
pub fn fallback_name(id: u8) -> String {
    format!("Player {}", id as u32 + 1)
}

/// A player's name online (from the newest snapshot's player list).
fn online_name(s: &ClientSession, id: u8) -> String {
    s.player_name(id)
        .filter(|n| !n.is_empty())
        .map_or_else(|| fallback_name(id), str::to_string)
}

/// Practice names: the local player and numbered target dummies.
fn practice_name(id: PlayerId, me: PlayerId) -> String {
    if id == me {
        "You".into()
    } else {
        format!("Dummy {}", id.0)
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

/// Surface state bits per chunk-mesh vertex (`ChunkMesh::surface`, drawn
/// by the web client's terrain shader; M2 plan: hot, charged, glow).
pub const SURFACE_HOT: u8 = 1 << 0;
pub const SURFACE_CHARGED: u8 = 1 << 1;
pub const SURFACE_GLOW: u8 = 1 << 2;
/// Permanent scorch (charred for good: scorch trails and marks).
pub const SURFACE_SCORCH: u8 = 1 << 3;

/// Per-vertex surface state bits of a chunk mesh (`Mesh::states`, the same
/// bits as `SURFACE_*`, active at the terrain's display tick), or empty when
/// the mesh carries none (meshes built from a bare sample grid).
pub fn surface_flags(mesh: &Mesh) -> Vec<u8> {
    if mesh.states.len() != mesh.positions.len() {
        return Vec::new();
    }
    let all = SURFACE_HOT | SURFACE_CHARGED | SURFACE_GLOW | SURFACE_SCORCH;
    mesh.states.iter().map(|s| s & all).collect()
}

/// Rock material id per vertex (`ls_math::terrain::MATERIAL_*`), or empty
/// when the mesh carries none.
pub fn material_ids(mesh: &Mesh) -> Vec<u8> {
    if mesh.materials.len() != mesh.positions.len() {
        return Vec::new();
    }
    mesh.materials.clone()
}

/// Display blend weights of weak and hard rock, two bytes per vertex, or
/// empty when the mesh carries none.
pub fn material_weights(mesh: &Mesh) -> Vec<u8> {
    if mesh.material_weights.len() != mesh.positions.len() {
        return Vec::new();
    }
    mesh.material_weights.iter().flatten().copied().collect()
}

/// Paint effect colour per vertex (`ls_math::terrain::PAINT_*`, unknown
/// ids read as plain), or empty when the mesh carries none.
pub fn paint_colors(mesh: &Mesh) -> Vec<u8> {
    if mesh.colors.len() != mesh.positions.len() {
        return Vec::new();
    }
    mesh.colors
        .iter()
        .map(|&c| ls_math::terrain::paint_color(c))
        .collect()
}

// The wire bits are `ls_math::terrain::SurfaceStates`.
const _: () = assert!(SURFACE_HOT == ls_math::terrain::SurfaceStates::HOT.bits());
const _: () = assert!(SURFACE_CHARGED == ls_math::terrain::SurfaceStates::CHARGED.bits());
const _: () = assert!(SURFACE_GLOW == ls_math::terrain::SurfaceStates::GLOW.bits());
const _: () = assert!(SURFACE_SCORCH == ls_math::terrain::SurfaceStates::SCORCH.bits());

#[cfg(test)]
mod tests;

/// Keeps edge buttons pressed on a frame that runs no tick for the next
/// tick (see [`EDGE_BUTTONS`]).
fn latch_edges(pending: &mut u16, controls: &mut Controls, ticks: u32) {
    if ticks == 0 {
        *pending |= controls.buttons & EDGE_BUTTONS;
    } else {
        controls.buttons |= *pending;
        *pending = 0;
    }
}

/// The Paint Thrower colour after `color` (red, blue, yellow; the sim's
/// `paint_thrower::next_color`).
fn next_paint_color(color: u8) -> u8 {
    ls_sim::weapons::specials::paint_thrower::next_color(color)
}
