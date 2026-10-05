//! [`ClientSession`]: everything a client (browser via `ls-wasm`, or a bot)
//! needs after the handshake, as pure logic. The host feeds it received
//! packets and a fixed-step clock and sends the messages it returns.
//!
//! **Match transitions (protocol 5).** A `Welcome` on the open connection
//! starts the next match: the session rebuilds itself in place (new level,
//! rule set, terrain, predictor, clock, snapshot history) and reports
//! [`Incoming::MatchTransition`]; counters, ping nonces and the lobby state
//! carry over. Inputs are stamped with the `Welcome`'s epoch, and snapshots
//! of another epoch (the old match's, still in flight) are dropped.

use std::collections::VecDeque;

use ls_math::terrain::TerrainEdit;
use ls_math::{Quat, Vec3};
use ls_net::{
    mode_phase, InventoryMsg, LobbyStateMsg, MatchResultsMsg, Message, NetEntity, NetEvent,
    NetModeState, NetPlayer, NetProjectile, NetShip, Snapshot, SnapshotHistory, Welcome,
};
use ls_sim::{
    GadgetTable, GameData, InputCmd, RuleSet, ShipState, ShipTable, ShipTuning, ShipType,
    WeaponTable,
};
use ls_worldgen::{Generator, ShareCode};

use super::clock::ClientClock;
use super::input::InputSequencer;
use super::interp::{InterpolationBuffer, INTERP_DELAY_TICKS};
use super::prediction::{PredictionConfig, PredictionStats, Predictor, Reconcile};
use crate::setup::match_setup;
use crate::{GameError, GameTerrain, NetConv};

/// Ticks the first input is stamped ahead of the `Welcome` server tick; the
/// clock adds more if the server reports starvation.
pub const INITIAL_LEAD_TICKS: u64 = 3;
/// Ticks between pings that measure the input slack (see [`ClientClock`]).
pub const PING_INTERVAL_TICKS: u32 = 30;
/// Most `Input` messages one frame sends (see [`ClientSession::run_ticks`]).
pub const MAX_BURST_INPUTS: usize = 8;
/// `NetProjectile::state` bit of a projectile its owner guides
/// (`ls_sim::world::proj_flags::CONTROLS_OWNER`).
pub const GUIDED_STATE_BIT: u8 = ls_sim::world::proj_flags::CONTROLS_OWNER as u8;

/// What a received message did.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A snapshot was applied. `stale` snapshots (older than one already
    /// seen) only contributed terrain edits.
    Snapshot {
        tick: u32,
        stale: bool,
        reconcile: Option<Reconcile>,
        new_edits: usize,
    },
    /// Gameplay events (also collected in [`ClientSession::take_events`]).
    Events(usize),
    Pong {
        nonce: u32,
        client_time_ms: u32,
        server_tick: u32,
    },
    /// The server closed the session.
    Disconnected(String),
    /// The own inventory changed ([`ClientSession::inventory`]).
    Inventory,
    /// A new `Welcome` on the open connection: the session now plays the
    /// next match (new level and terrain: hosts drop their meshes and
    /// rebuild everything derived from the old match).
    MatchTransition { epoch: u8 },
    /// The lobby state changed ([`ClientSession::lobby`]).
    Lobby,
    /// The match results arrived ([`ClientSession::results`]).
    Results,
    /// Anything else (ignored).
    Ignored,
}

/// Counters for tests and debug overlays.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SessionStats {
    pub snapshots: u64,
    pub stale_snapshots: u64,
    pub full_snapshots: u64,
    pub delta_snapshots: u64,
    pub edits_applied: u64,
    /// Match transitions (`Welcome`s on the open connection) handled.
    pub transitions: u64,
    /// Snapshots of another match epoch, dropped.
    pub stale_epoch_snapshots: u64,
}

/// Client state after `Welcome`: local terrain, predicted local ship,
/// interpolated remote ships, latest snapshot and outgoing inputs.
#[derive(Debug, Clone)]
pub struct ClientSession {
    terrain: GameTerrain,
    conv: NetConv,
    rules: RuleSet,
    weapons: WeaponTable,
    gadgets: GadgetTable,
    player: u8,
    team: u8,
    /// Ship type of the local ship (from `Welcome`, then from the players
    /// list of each snapshot).
    ship_type: ShipType,
    inputs: InputSequencer,
    predictor: Predictor,
    interp: InterpolationBuffer,
    received: SnapshotHistory,
    latest: Option<Snapshot>,
    clock: ClientClock,
    events: Vec<NetEvent>,
    stats: SessionStats,
    /// Outstanding pings: nonce and the newest input tick sent with it.
    pings: VecDeque<(u32, u64)>,
    next_nonce: u32,
    ticks_since_ping: u32,
    /// Scrap pickup points of the match (from the pristine level).
    pickup_points: Vec<Vec3>,
    /// Server tick at which the match timer runs out (from snapshots).
    match_end_tick: Option<u64>,
    /// Entities of the last few snapshots (tick, list), oldest first, for
    /// interpolation at the render tick.
    entity_history: VecDeque<(u32, Vec<NetEntity>)>,
    /// The own ship's inventory as the server last sent it.
    inventory: Option<InventoryMsg>,
    /// Match epoch of the `Welcome` (stamped on inputs; snapshots must match).
    epoch: u8,
    /// Room code of the `Welcome` (empty for the public room).
    room_code: String,
    /// Share code of the match's level (from the `Welcome`).
    share_code: String,
    /// Newest lobby state (`Message::LobbyState`), kept across matches.
    lobby: Option<LobbyStateMsg>,
    /// Results of this match, once it ended.
    results: Option<MatchResultsMsg>,
}

/// Snapshots of entities kept for interpolation.
const ENTITY_HISTORY: usize = 6;

impl ClientSession {
    /// Builds the session from the server's `Welcome` with the built-in
    /// ship and gadget data: `weapons` is the weapon table and `tuning`
    /// replaces the Tinker's numbers (the other ship types are built in).
    /// Prefer [`ClientSession::from_welcome_with`].
    pub fn from_welcome(
        welcome: &Welcome,
        weapons: WeaponTable,
        tuning: ShipTuning,
    ) -> Result<Self, GameError> {
        let data = GameData {
            weapons,
            ships: ShipTable::builtin().with(ShipType::Tinker, tuning),
            gadgets: GadgetTable::builtin(),
        };
        Self::from_welcome_with(welcome, data)
    }

    /// Builds the session from the server's `Welcome`: regenerates the level
    /// from the share code, replays the edit log and parses the rule set.
    /// `data` is the client's game data; with the built-in data the
    /// `Welcome`'s data hash must match `ls_sim::data_hash()`.
    pub fn from_welcome_with(welcome: &Welcome, data: GameData) -> Result<Self, GameError> {
        if data == GameData::builtin() && welcome.data_hash != ls_sim::data_hash() {
            return Err(GameError::Protocol(format!(
                "game data mismatch: server {:016x}, client {:016x}",
                welcome.data_hash,
                ls_sim::data_hash()
            )));
        }
        let GameData {
            weapons,
            ships,
            gadgets,
        } = data;
        let code = ShareCode::decode(&welcome.share_code)?;
        let terrain = GameTerrain::replay_welcome(
            Generator::from_code(&code)?,
            &welcome.edit_log,
            welcome.edit_count,
        );
        let text = std::str::from_utf8(&welcome.ruleset)
            .map_err(|_| GameError::Protocol("rule set is not UTF-8".into()))?;
        let rules = RuleSet::from_ron(&welcome.ruleset_name, text)?;
        rules.resolve_with(&weapons, &gadgets)?;
        let ship_type = ShipType::from_index(welcome.ship_type).unwrap_or(ShipType::Tinker);
        let conv = NetConv::new(welcome.quant, ships.clone())?;
        let config = PredictionConfig::for_conv(&conv);
        // Spawn points come from the pristine level, like on the server.
        let setup = match_setup(&code, &rules, &GameTerrain::from_code(&code)?)?;
        let mut predictor = Predictor::new(ships.get(ship_type).clone(), config);
        predictor.set_ship_type(ship_type, ships.get(ship_type).clone());
        predictor.set_spawn_poses(
            setup
                .spawns
                .iter()
                .map(|s| (s.pos, s.orientation))
                .collect(),
        );
        Ok(ClientSession {
            terrain,
            rules,
            weapons,
            gadgets,
            player: welcome.player_id,
            team: welcome.team,
            ship_type,
            inputs: InputSequencer::new(welcome.server_tick as u64 + 1 + INITIAL_LEAD_TICKS),
            predictor,
            interp: InterpolationBuffer::default(),
            received: SnapshotHistory::default(),
            latest: None,
            clock: ClientClock::new(welcome.server_tick),
            events: Vec::new(),
            stats: SessionStats::default(),
            pings: VecDeque::new(),
            next_nonce: 0,
            ticks_since_ping: PING_INTERVAL_TICKS,
            pickup_points: setup.pickup_points,
            match_end_tick: None,
            entity_history: VecDeque::new(),
            inventory: None,
            epoch: welcome.epoch,
            room_code: welcome.room_code.clone(),
            share_code: welcome.share_code.clone(),
            lobby: None,
            results: None,
            conv,
        })
    }

    /// Starts the next match from a `Welcome` received on the open
    /// connection (see the module docs): rebuilds everything from it with
    /// the same game data, keeping the counters (plus one transition), the
    /// ping nonce sequence and the lobby state. On error the session is
    /// unchanged.
    pub fn transition(&mut self, welcome: &Welcome) -> Result<(), GameError> {
        let data = GameData {
            weapons: self.weapons.clone(),
            ships: self.conv.ships().clone(),
            gadgets: self.gadgets.clone(),
        };
        let mut next = Self::from_welcome_with(welcome, data)?;
        next.stats = SessionStats {
            transitions: self.stats.transitions + 1,
            ..self.stats
        };
        next.next_nonce = self.next_nonce;
        next.lobby = self.lobby.take();
        *self = next;
        Ok(())
    }

    // ----- accessors -------------------------------------------------------

    /// This player's wire id.
    pub fn player(&self) -> u8 {
        self.player
    }
    pub fn team(&self) -> u8 {
        self.team
    }
    pub fn terrain(&self) -> &GameTerrain {
        &self.terrain
    }
    /// Mutable terrain access (meshing dirty chunks).
    pub fn terrain_mut(&mut self) -> &mut GameTerrain {
        &mut self.terrain
    }
    pub fn conv(&self) -> &NetConv {
        &self.conv
    }
    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }
    pub fn weapons(&self) -> &WeaponTable {
        &self.weapons
    }
    pub fn gadgets(&self) -> &GadgetTable {
        &self.gadgets
    }
    /// Ship type of the local ship.
    pub fn ship_type(&self) -> ShipType {
        self.ship_type
    }
    /// Match epoch of the current `Welcome`.
    pub fn epoch(&self) -> u8 {
        self.epoch
    }
    /// Share code of the current match's level.
    pub fn share_code(&self) -> &str {
        &self.share_code
    }
    /// Room code of the current `Welcome` (empty for the public room).
    pub fn room_code(&self) -> &str {
        &self.room_code
    }

    /// Mode state of the newest snapshot (phase, scores, lives, objectives;
    /// the default before the first snapshot).
    pub fn mode(&self) -> &NetModeState {
        static EMPTY: std::sync::OnceLock<NetModeState> = std::sync::OnceLock::new();
        match &self.latest {
            Some(s) => &s.mode,
            None => EMPTY.get_or_init(NetModeState::default),
        }
    }

    /// Newest lobby state from the server, if any.
    pub fn lobby(&self) -> Option<&LobbyStateMsg> {
        self.lobby.as_ref()
    }

    /// Results of this match, once the server sent them.
    pub fn results(&self) -> Option<&MatchResultsMsg> {
        self.results.as_ref()
    }

    /// The message that marks the player ready (or not) for the next match.
    pub fn ready(&self, ready: bool) -> Message {
        Message::Ready { ready }
    }

    /// The message that picks a team for the next match (`None` = auto).
    pub fn pick_team(&self, team: Option<u8>) -> Message {
        Message::PickTeam { team }
    }

    /// The message that asks to spectate (or to play again).
    pub fn spectate(&self, spectate: bool) -> Message {
        Message::Spectate { spectate }
    }

    /// Players of the newest snapshot (names, teams, ship types, gadgets).
    pub fn players(&self) -> &[NetPlayer] {
        self.latest.as_ref().map_or(&[], |s| &s.players)
    }

    /// Side of a ship id: its team index, or its own side id in
    /// free-for-all (up to `ls_sim::mode::MAX_FFA_SIDE`). Read from the
    /// players list (a full byte); the ship flags only carry the two low
    /// bits (`ship_flags::team`), which they fall back to for ships not in
    /// the list yet.
    pub fn side_of(&self, id: u8) -> Option<u8> {
        if let Some(p) = self.players().iter().find(|p| p.id == id) {
            return Some(p.team);
        }
        self.latest
            .as_ref()?
            .ships
            .iter()
            .find(|s| s.id == id)
            .map(|s| ls_net::ship_flags::team(s.flags))
    }

    /// Name of a ship id, if known.
    pub fn player_name(&self, id: u8) -> Option<&str> {
        self.players()
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.as_str())
    }

    /// Entities (debris, pickups, scrap, wrecks, decoys, ghosts, dust)
    /// interpolated at [`ClientSession::render_tick`], like remote ships:
    /// the entity set of the first snapshot at or after that tick, positions
    /// interpolated from the snapshot before it (debris is sent without
    /// velocity, so it never runs ahead of the ships that push it).
    pub fn entities(&self) -> Vec<NetEntity> {
        self.entities_at(self.render_tick())
    }

    /// [`ClientSession::entities`] at an explicit server tick.
    pub fn entities_at(&self, tick: f64) -> Vec<NetEntity> {
        let h = &self.entity_history;
        let Some(later) = h.iter().position(|(t, _)| *t as f64 >= tick) else {
            // Past the newest snapshot: hold it.
            return h.back().map(|(_, e)| e.clone()).unwrap_or_default();
        };
        let (tb, b) = &h[later];
        if later == 0 {
            return b.clone();
        }
        let (ta, a) = &h[later - 1];
        let f = ((tick - *ta as f64) / (*tb as f64 - *ta as f64)).clamp(0.0, 1.0) as f32;
        b.iter()
            .map(|e| match a.binary_search_by_key(&e.id, |x| x.id) {
                Ok(i) if a[i].kind == e.kind => NetEntity {
                    pos: a[i].pos.lerp(e.pos, f),
                    ..*e
                },
                _ => *e,
            })
            .collect()
    }

    /// The own ship's carried weapons, ammo, levels, selection and gadget as
    /// the server last reported them (`Message::Inventory`).
    pub fn inventory(&self) -> Option<&InventoryMsg> {
        self.inventory.as_ref()
    }

    /// The message that picks the ship type for the next respawn (send it
    /// reliably). The server ignores types the rule set does not allow.
    pub fn select_ship(&self, ship_type: ShipType) -> Message {
        Message::SelectShip {
            ship_type: ship_type.index(),
        }
    }
    pub fn clock(&self) -> &ClientClock {
        &self.clock
    }
    pub fn stats(&self) -> SessionStats {
        self.stats
    }
    pub fn prediction_stats(&self) -> PredictionStats {
        self.predictor.stats()
    }
    pub fn predictor(&self) -> &Predictor {
        &self.predictor
    }
    /// Tick the next input will be stamped with.
    pub fn next_input_tick(&self) -> u64 {
        self.inputs.next_tick()
    }
    /// Newest snapshot received (unpacked).
    pub fn latest_snapshot(&self) -> Option<&Snapshot> {
        self.latest.as_ref()
    }

    /// Scrap pickup points of the match (`ls_sim::MatchSetup::pickup_points`,
    /// derived from the pristine level like on the server).
    pub fn pickup_points(&self) -> &[Vec3] {
        &self.pickup_points
    }

    /// Server tick at which the match ends (movement stops after it), once
    /// a snapshot told.
    pub fn match_end_tick(&self) -> Option<u64> {
        self.match_end_tick
    }

    /// Predicted local ship (physics state), once the first snapshot arrived.
    pub fn local_ship(&self) -> Option<&ShipState> {
        self.predictor.ship()
    }

    /// Local ship pose to render (prediction plus correction smoothing).
    pub fn local_render_pose(&self) -> Option<(Vec3, Quat)> {
        Some((self.predictor.render_pos()?, self.predictor.render_rot()?))
    }

    /// Server tick at which remote ships are rendered (server time minus 100 ms).
    pub fn render_tick(&self) -> f64 {
        self.clock.server_time() - INTERP_DELAY_TICKS
    }

    /// Remote ships interpolated at [`ClientSession::render_tick`].
    pub fn remote_ships(&self) -> Vec<NetShip> {
        self.interp.sample(self.render_tick())
    }

    /// Remote ships interpolated at an explicit server tick.
    pub fn remote_ships_at(&self, tick: f64) -> Vec<NetShip> {
        self.interp.sample(tick)
    }

    /// Projectiles of the newest snapshot, moved along their (constant)
    /// velocity to the estimated server time.
    pub fn projectiles(&self) -> Vec<NetProjectile> {
        let Some(s) = &self.latest else {
            return Vec::new();
        };
        let dt = ((self.clock.server_time() - s.tick as f64).clamp(0.0, 8.0)
            / ls_math::TICK_HZ as f64) as f32;
        s.projectiles
            .iter()
            .map(|p| NetProjectile {
                pos: p.pos + p.vel * dt,
                ..*p
            })
            .collect()
    }

    /// Scores of the newest snapshot (see `docs/architecture/game-glue.md`:
    /// team 0 cells, team 1 cells, surface cells).
    pub fn scores(&self) -> &[u32] {
        self.latest.as_ref().map_or(&[], |s| &s.scores)
    }

    /// Gameplay events received since the last call.
    pub fn take_events(&mut self) -> Vec<NetEvent> {
        std::mem::take(&mut self.events)
    }

    /// Terrain edits applied so far (sequence number of the next edit).
    pub fn edit_count(&self) -> u32 {
        self.terrain.edit_count()
    }

    // ----- input and time ----------------------------------------------------

    /// Runs the ticks due this frame (`due` from the host's 60 Hz accumulator,
    /// adjusted by the clock) with the same controls. Returns one input
    /// message per simulated tick, plus a `Ping` every
    /// [`PING_INTERVAL_TICKS`] (send them in order).
    ///
    /// Same as [`ClientSession::plan_ticks`] followed by
    /// [`ClientSession::run_ticks`]. Hosts that spread per-frame deltas
    /// (mouse look) over the ticks must use those two instead, since the
    /// clock may run more or fewer ticks than `due`.
    pub fn frame(&mut self, due: u32, controls: InputCmd) -> Vec<Message> {
        let n = self.plan_ticks(due);
        self.run_ticks(n, due, controls)
    }

    /// First half of [`ClientSession::frame`]: advances the clock by `due`
    /// ticks of local time and returns how many simulation ticks to run now
    /// (`due`, plus up to 4 while catching up, minus 1 when skipping).
    pub fn plan_ticks(&mut self, due: u32) -> u32 {
        let n = self.clock.ticks_to_run(due);
        self.clock.advance(due as f64);
        n
    }

    /// Second half of [`ClientSession::frame`]: runs `n` ticks (from
    /// [`ClientSession::plan_ticks`]) with `controls` and adds a `Ping` when
    /// one is due (`due` is the local time that passed, in ticks).
    pub fn run_ticks(&mut self, n: u32, due: u32, controls: InputCmd) -> Vec<Message> {
        let mut out: Vec<Message> = (0..n).map(|_| self.tick(controls)).collect();
        // A long catch-up burst before sync (idle commands): the older inputs
        // are late on arrival anyway, so send only the tail. Once synced every
        // command counts, and an Input only repeats the newest three.
        if !self.clock.is_synced() && out.len() > MAX_BURST_INPUTS {
            out.drain(..out.len() - MAX_BURST_INPUTS);
        }
        self.ticks_since_ping += due;
        if self.ticks_since_ping >= PING_INTERVAL_TICKS && self.inputs.next_tick() > 0 {
            self.ticks_since_ping = 0;
            let nonce = self.next_nonce;
            self.next_nonce = self.next_nonce.wrapping_add(1);
            let sent_tick = self.inputs.next_tick() - 1;
            if self.pings.len() >= 16 {
                self.pings.pop_front();
            }
            self.pings.push_back((nonce, sent_tick));
            out.push(Message::Ping {
                nonce,
                client_time_ms: (sent_tick * 1000 / ls_math::TICK_HZ as u64) as u32,
            });
        }
        out
    }

    /// One local tick: stamps and quantizes `controls`, predicts the local
    /// ship and returns the input message (newest three commands).
    ///
    /// Until the clock is synchronized ([`ClientClock::is_synced`]) the
    /// command is replaced by an idle one: the server may still be simulating
    /// those ticks without our input, and look input is sent as deltas.
    pub fn tick(&mut self, controls: InputCmd) -> Message {
        let controls = if self.clock.is_synced() {
            controls
        } else {
            InputCmd::idle(0)
        };
        let cmd = self.inputs.next(controls);
        // While the own ship guides a projectile (Eyeball) the server gives
        // it no thrust or look (the look steers the missile): predict that.
        let guiding = self.latest.as_ref().is_some_and(|s| {
            s.projectiles
                .iter()
                .any(|p| p.owner == self.player && p.state & GUIDED_STATE_BIT != 0)
        });
        let predicted = if guiding {
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
        // The Squeeze's walls at the predicted tick (from the mode state).
        let shell = crate::mode::squeeze_shell(self.mode(), cmd.tick);
        self.terrain.set_shell(shell);
        self.predictor.predict(predicted, &self.terrain);
        let ack = self.newest_tick();
        let mut msg = self.inputs.message(ack).expect("a command was just added");
        msg.epoch = self.epoch;
        Message::Input(msg)
    }

    /// True once the server receives our inputs in time (see [`ClientSession::tick`]).
    pub fn is_synced(&self) -> bool {
        self.clock.is_synced()
    }

    fn newest_tick(&self) -> Option<u32> {
        self.latest.as_ref().map(|s| s.tick)
    }

    // ----- receiving ---------------------------------------------------------

    /// Decodes and handles one packet from the server.
    pub fn handle_packet(&mut self, bytes: &[u8]) -> Result<Incoming, GameError> {
        // A delta snapshot of the previous match (still in flight after a
        // new `Welcome`) refers to a baseline this session never had: drop
        // it by its epoch before decoding.
        if Message::peek_snapshot_epoch(bytes).is_some_and(|e| e != self.epoch) {
            self.stats.stale_epoch_snapshots += 1;
            return Ok(Incoming::Ignored);
        }
        let msg = Message::decode_with(bytes, &self.received)?;
        self.handle_message(msg)
    }

    /// Handles one decoded message from the server.
    pub fn handle_message(&mut self, msg: Message) -> Result<Incoming, GameError> {
        match msg {
            Message::Snapshot(sm) if sm.epoch != self.epoch => {
                // The previous match's, still in flight: its baselines and
                // edits belong to another world.
                self.stats.stale_epoch_snapshots += 1;
                Ok(Incoming::Ignored)
            }
            Message::Snapshot(sm) => {
                if sm.baseline.is_some() {
                    self.stats.delta_snapshots += 1;
                } else {
                    self.stats.full_snapshots += 1;
                }
                self.on_snapshot(sm.snapshot)
            }
            Message::Events(ev) => {
                let n = ev.len();
                self.events.extend(ev);
                Ok(Incoming::Events(n))
            }
            Message::Pong {
                nonce,
                client_time_ms,
                server_tick,
            } => {
                if let Some(i) = self.pings.iter().position(|(n, _)| *n == nonce) {
                    let (_, sent_tick) = self.pings[i];
                    self.pings.drain(..=i);
                    self.clock.on_pong(sent_tick, server_tick);
                }
                Ok(Incoming::Pong {
                    nonce,
                    client_time_ms,
                    server_tick,
                })
            }
            Message::Inventory(inv) => {
                // Reliable and in order; keep the newest anyway.
                if self
                    .inventory
                    .as_ref()
                    .is_none_or(|old| inv.tick >= old.tick)
                {
                    self.inventory = Some(inv);
                }
                Ok(Incoming::Inventory)
            }
            Message::Welcome(w) => {
                let w = ls_net::check_welcome(Message::Welcome(w))
                    .map_err(|e| GameError::Protocol(e.to_string()))?;
                self.transition(&w)?;
                Ok(Incoming::MatchTransition { epoch: w.epoch })
            }
            Message::LobbyState(l) => {
                self.lobby = Some(*l);
                Ok(Incoming::Lobby)
            }
            Message::MatchResults(r) => {
                self.results = Some(*r);
                Ok(Incoming::Results)
            }
            Message::Disconnect { reason } => Ok(Incoming::Disconnected(reason)),
            Message::Reject(r) => Ok(Incoming::Disconnected(r.message)),
            _ => Ok(Incoming::Ignored),
        }
    }

    fn on_snapshot(&mut self, packed: ls_net::PackedSnapshot) -> Result<Incoming, GameError> {
        self.stats.snapshots += 1;
        // Terrain edits first, in log order, whatever the snapshot's age.
        let edits: Vec<TerrainEdit> = packed.edits.iter().map(|e| (*e).into()).collect();
        let new_edits = self.terrain.apply_sequence(packed.edit_base, &edits)?;
        self.stats.edits_applied += new_edits as u64;
        let tick = packed.tick;
        let snap = self.conv.quantizer().unpack_snapshot(&packed);
        self.received.push(packed);

        if self.newest_tick().is_some_and(|n| tick <= n) {
            self.stats.stale_snapshots += 1;
            return Ok(Incoming::Snapshot {
                tick,
                stale: true,
                reconcile: None,
                new_edits,
            });
        }
        // After the match timer runs out the server no longer moves ships.
        if snap.match_ticks_left > 0 {
            self.match_end_tick = Some(tick as u64 + snap.match_ticks_left as u64);
        }
        let end = if snap.mode.phase == mode_phase::ENDED && snap.mode != NetModeState::default() {
            // A mode that ended early (kill target, last ship standing):
            // the room sends the last simulated tick as `end_tick`.
            let e = (snap.mode.end_tick as u64).min(tick as u64);
            self.match_end_tick = Some(self.match_end_tick.map_or(e, |m| m.min(e)));
            self.match_end_tick
        } else {
            match self.match_end_tick {
                Some(end) if snap.match_ticks_left == 0 => Some(end.min(tick as u64)),
                Some(end) => Some(end),
                None if snap.match_ticks_left == 0 => Some(tick as u64),
                None => None,
            }
        };
        if let Some(end) = end {
            self.predictor.freeze_after(end, &self.terrain);
        }
        if let Some(t) = snap
            .players
            .iter()
            .find(|p| p.id == self.player)
            .and_then(|p| ShipType::from_index(p.ship_type))
        {
            if t != self.ship_type {
                self.ship_type = t;
                self.predictor
                    .set_ship_type(t, self.conv.tuning_for(t).clone());
            }
        }
        // The mode's movement limits on the own ship (Heist carrier).
        let own_flags = snap
            .mode
            .players
            .iter()
            .find(|p| p.id == self.player)
            .map_or(0, |p| p.flags);
        self.predictor.set_limits(
            ls_sim::modes::module(self.rules.mode.kind()).move_limits(&self.rules.mode, own_flags),
        );
        let reconcile = snap.ships.iter().find(|s| s.id == self.player).map(|own| {
            // The players list carries the full side id (free-for-all
            // sides exceed the two team bits of the flags).
            self.team = snap
                .players
                .iter()
                .find(|p| p.id == self.player)
                .map_or(ls_net::ship_flags::team(own.flags), |p| p.team);
            self.predictor
                .reconcile(tick as u64, own, &self.conv, &self.terrain)
        });
        let remote: Vec<NetShip> = snap
            .ships
            .iter()
            .filter(|s| s.id != self.player)
            .copied()
            .collect();
        self.interp.push(tick, remote);
        if self.entity_history.len() >= ENTITY_HISTORY {
            self.entity_history.pop_front();
        }
        self.entity_history.push_back((tick, snap.entities.clone()));
        self.clock
            .on_snapshot(tick, snap.ack_input, self.inputs.next_tick());
        self.latest = Some(snap);
        Ok(Incoming::Snapshot {
            tick,
            stale: false,
            reconcile,
            new_edits,
        })
    }
}
