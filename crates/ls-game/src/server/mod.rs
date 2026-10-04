//! Server core (pure logic, used by `ls-server`): the authoritative [`Room`].
//!
//! The host owns sockets and timing: it calls [`Room::join`] for each accepted
//! `Hello`, feeds received packets to [`Room::handle_packet`], calls
//! [`Room::step`] at 60 Hz and sends what [`Room::take_outgoing`] returns.

pub mod lagcomp;

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

use ls_math::terrain::TerrainQuery;
use ls_math::Vec3;
use ls_net::message::MAX_EVENTS;
use ls_net::{
    check_data_hash, Channel, Hello, InputMsg, InventoryMsg, Message, NetEntity, NetEvent,
    NetInput, NetShip, NetWeapon, Reject, RejectReason, Relevance, Snapshot, SnapshotHistory,
    SnapshotMsg, Welcome, MAX_SHIPS, PROTOCOL_VERSION,
};
use ls_sim::terrain::segment_sphere;
use ls_sim::{Event, InputCmd, PlayerId, ReplicatedKind, ShipState, ShipType, World};

use crate::client::INTERP_DELAY_TICKS;
use crate::convert::{entity_to_net, events_to_net, net_to_input, projectile_to_net};
use crate::setup::{build_match, MatchConfig};
use crate::{GameError, GameTerrain, NetConv};
pub use lagcomp::{Displacement, HistoryFrame, ShipHistory};

/// Room settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomConfig {
    /// At most this many players (capped at `ls_net::MAX_SHIPS`).
    pub max_players: u8,
    /// Ticks between snapshots (2 = 30 Hz).
    pub snapshot_interval: u64,
    /// Rewind limit for lag compensation in ticks (15 = 250 ms); also the
    /// length of the position history.
    pub lag_comp_max_ticks: u64,
    /// Enables lag compensation.
    pub lag_compensation: bool,
    /// Client interpolation delay assumed when estimating what a shooter saw
    /// (see [`Room::view_tick`]).
    pub interp_delay_ticks: u64,
    /// Inputs further ahead of the server than this are dropped.
    pub max_input_lead: u64,
    /// Terrain edits per snapshot (the rest follow in later snapshots).
    pub max_edits_per_snapshot: usize,
    /// Generate every chunk in [`Room::new`] so the first paint coverage
    /// query (tick 30) does not stall a tick. Tests that never reach it can
    /// turn this off; debug builds generate slowly.
    pub pregenerate: bool,
}

impl Default for RoomConfig {
    fn default() -> Self {
        RoomConfig {
            max_players: 8,
            snapshot_interval: 2,
            lag_comp_max_ticks: ls_math::Tick::ticks_for_millis(250),
            lag_compensation: true,
            interp_delay_ticks: INTERP_DELAY_TICKS as u64,
            max_input_lead: 120,
            max_edits_per_snapshot: 64,
            pregenerate: true,
        }
    }
}

/// A packet for one client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// Wire id of the recipient.
    pub client: u8,
    /// Preferred channel (hosts may fall back to reliable for large datagrams).
    pub channel: Channel,
    pub bytes: Vec<u8>,
}

/// Counters for logs, tests and desync monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoomStats {
    /// Ticks for which a client had no input and its last one was repeated
    /// (counted from the client's first applied input on).
    pub missing_inputs: u64,
    /// Inputs that arrived after their tick was simulated.
    pub late_inputs: u64,
    /// Inputs received more than once (redundant copies).
    pub duplicate_inputs: u64,
    /// Inputs dropped for being too far ahead.
    pub dropped_inputs: u64,
    /// Lag-compensated hits (target displacements).
    pub compensated_hits: u64,
    pub snapshots_sent: u64,
    pub snapshot_bytes: u64,
    /// Messages that could not be encoded and were not sent (only reachable
    /// with inconsistent state; a non-zero value is a bug).
    pub encode_errors: u64,
}

/// What one [`Room::step`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReport {
    pub tick: u64,
    /// Simulation events of this tick.
    pub events: Vec<Event>,
    /// Lag compensation displacements applied this tick.
    pub compensated: Vec<Displacement>,
    /// True if snapshots were produced this tick.
    pub snapshot: bool,
}

#[derive(Debug, Clone)]
struct QueuedInput {
    cmd: InputCmd,
    /// Estimated shooter view delay (ticks) when the command was made.
    rewind: u64,
}

#[derive(Debug, Clone)]
struct ClientState {
    player: PlayerId,
    name: String,
    inputs: BTreeMap<u64, QueuedInput>,
    last: QueuedInput,
    /// Newest real (not repeated) input tick applied.
    ack_input: u64,
    /// True once a real input was applied. Until then the client is still
    /// setting up (level generation) and has predicted nothing, so repeated
    /// idle commands are not counted as missing inputs.
    started: bool,
    /// True once any `Input` arrived: the client has built its session
    /// (level generation can take a second after the `Welcome`) and gets
    /// snapshots from now on. Snapshots sent before that would pile up
    /// unread, all full (nothing can be acknowledged yet), and then be
    /// handled in one burst: each one a full snapshot, a starvation report
    /// and a prediction snap on the client.
    ready: bool,
    acked_snapshot: Option<u32>,
    sent: SnapshotHistory,
    /// Terrain edits the client is known to have.
    known_edits: u32,
    /// The inventory last sent to the client (`Message::Inventory`).
    inventory_sent: Option<InventoryMsg>,
}

/// Information about a connected player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    pub id: u8,
    pub player: PlayerId,
    pub name: String,
    pub team: u8,
    pub ship_type: ShipType,
}

/// One authoritative match: world, terrain, clients. See the module docs.
#[derive(Debug)]
pub struct Room {
    cfg: RoomConfig,
    match_cfg: MatchConfig,
    world: World,
    terrain: GameTerrain,
    conv: NetConv,
    share_code: String,
    ruleset_ron: String,
    /// Hash of the match's game data; `Hello`s with another hash are rejected.
    data_hash: u64,
    tick: u64,
    clients: BTreeMap<u8, ClientState>,
    history: ShipHistory,
    rewinds: BTreeMap<u32, u64>,
    pending_events: Vec<NetEvent>,
    outgoing: Vec<Outgoing>,
    stats: RoomStats,
}

impl Room {
    /// Builds the level, the setup and the world (and generates every chunk
    /// if `cfg.pregenerate`).
    pub fn new(match_cfg: MatchConfig, cfg: RoomConfig) -> Result<Room, GameError> {
        let (world, mut terrain) = build_match(&match_cfg)?;
        if cfg.pregenerate {
            terrain.generate_all();
        }
        let conv = NetConv::for_level(
            terrain.bounds(),
            &match_cfg.weapons,
            match_cfg.ships.clone(),
        )?;
        let data_hash = match_cfg.data_hash();
        let ruleset_ron = ron::to_string(&match_cfg.rules)
            .map_err(|e| GameError::Setup(format!("rule set serialization: {e}")))?;
        let share_code = match_cfg.code.encode()?;
        Ok(Room {
            history: ShipHistory::new(cfg.lag_comp_max_ticks + 1),
            cfg,
            match_cfg,
            world,
            terrain,
            conv,
            share_code,
            ruleset_ron,
            data_hash,
            tick: 0,
            clients: BTreeMap::new(),
            rewinds: BTreeMap::new(),
            pending_events: Vec::new(),
            outgoing: Vec::new(),
            stats: RoomStats::default(),
        })
    }

    // ----- accessors -------------------------------------------------------

    pub fn config(&self) -> &RoomConfig {
        &self.cfg
    }
    pub fn match_config(&self) -> &MatchConfig {
        &self.match_cfg
    }
    /// Server tick (number of steps so far).
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn world(&self) -> &World {
        &self.world
    }
    /// Mutable world access (admin commands, tests). Bypasses the rules.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }
    pub fn terrain(&self) -> &GameTerrain {
        &self.terrain
    }
    /// Mutable terrain access (tests): edits made here reach clients like
    /// the simulation's.
    #[cfg(test)]
    pub(crate) fn terrain_mut(&mut self) -> &mut GameTerrain {
        &mut self.terrain
    }
    pub fn conv(&self) -> &NetConv {
        &self.conv
    }
    pub fn stats(&self) -> RoomStats {
        self.stats
    }
    /// Game data hash clients must match (`Hello::data_hash`).
    pub fn data_hash(&self) -> u64 {
        self.data_hash
    }
    pub fn history(&self) -> &ShipHistory {
        &self.history
    }
    pub fn player_count(&self) -> usize {
        self.clients.len()
    }
    pub fn is_full(&self) -> bool {
        self.clients.len() >= self.max_players() as usize
    }
    fn max_players(&self) -> u8 {
        self.cfg.max_players.min(MAX_SHIPS)
    }

    pub fn clients(&self) -> Vec<ClientInfo> {
        self.clients
            .iter()
            .map(|(&id, c)| {
                let ship = self.world.ship(c.player);
                ClientInfo {
                    id,
                    player: c.player,
                    name: c.name.clone(),
                    team: ship.map_or(0, |s| s.team),
                    ship_type: ship.map_or(ShipType::Tinker, |s| s.ship_type),
                }
            })
            .collect()
    }

    /// Simulation player of a wire id.
    pub fn player_of(&self, client: u8) -> Option<PlayerId> {
        self.clients.get(&client).map(|c| c.player)
    }

    /// Wire id of a simulation player.
    pub fn wire_id(&self, player: PlayerId) -> Option<u8> {
        wire_id(&self.clients, player)
    }

    // ----- join / leave ------------------------------------------------------

    /// Adds a player for an accepted `Hello` and queues its `Welcome`
    /// (reliable). Returns the new client's wire id, or the `Reject` to send.
    ///
    /// Team: the preferred team if it would not make that team two or more
    /// players larger than the other, otherwise the smaller team. Ship type:
    /// the preferred one if the rule set allows it, otherwise the Tinker (or
    /// the first allowed type). A client built with other game data is
    /// rejected with [`RejectReason::DataMismatch`].
    pub fn join(&mut self, hello: &Hello) -> Result<u8, Reject> {
        let reject = |reason, message: &str| Reject {
            reason,
            server_version: PROTOCOL_VERSION,
            message: message.to_string(),
        };
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(reject(
                RejectReason::VersionMismatch,
                "protocol version mismatch",
            ));
        }
        if let Some(rj) = check_data_hash(hello, self.data_hash) {
            return Err(rj);
        }
        if self.is_full() {
            return Err(reject(RejectReason::ServerFull, "room is full"));
        }
        let id = (0..self.max_players())
            .find(|i| !self.clients.contains_key(i))
            .ok_or_else(|| reject(RejectReason::ServerFull, "room is full"))?;
        let team = hello.preferred_team.filter(|&t| {
            let count = |team: u8| self.world.ships().filter(|s| s.team == team).count();
            t < 2 && count(t) <= count(1 - t)
        });
        let allowed = &self.world.resolved_rules().allowed_ship_types;
        let ship_type = hello
            .ship_type
            .and_then(ShipType::from_index)
            .filter(|t| allowed.contains(t))
            .or_else(|| {
                allowed
                    .contains(&ShipType::Tinker)
                    .then_some(ShipType::Tinker)
            })
            .or_else(|| allowed.first().copied())
            .unwrap_or(ShipType::Tinker);
        let player = self
            .world
            .add_player(team, ship_type)
            .map_err(|e| reject(RejectReason::Other, &e.to_string()))?;
        let team = self.world.ship(player).map_or(0, |s| s.team);
        // Encode the Welcome before registering the client: if it cannot be
        // sent (edit log or rule set too large for the wire), the join fails
        // instead of leaving a ghost ship without a client.
        // Surface-state edits that expired before now stay out of the log
        // (they change nothing a late joiner can see).
        let (edit_log, edit_count) = self.terrain.welcome_log(self.tick);
        let welcome = Welcome {
            epoch: 0,
            room_code: String::new(),
            protocol_version: PROTOCOL_VERSION,
            player_id: id,
            team,
            ship_type: ship_type.index(),
            data_hash: self.data_hash,
            share_code: self.share_code.clone(),
            ruleset_name: self.match_cfg.rules.name.clone(),
            ruleset: self.ruleset_ron.clone().into_bytes(),
            quant: *self.conv.quantizer().config(),
            server_tick: self.tick as u32,
            edit_log,
            edit_count,
        };
        let welcome_bytes = match Message::Welcome(Box::new(welcome)).encode() {
            Ok(b) => b,
            Err(e) => {
                self.world.remove_player(player);
                self.stats.encode_errors += 1;
                return Err(reject(
                    RejectReason::Other,
                    &format!("cannot encode Welcome: {e}"),
                ));
            }
        };
        let idle = QueuedInput {
            cmd: InputCmd::idle(self.tick),
            rewind: 0,
        };
        self.clients.insert(
            id,
            ClientState {
                player,
                name: hello.name.clone(),
                inputs: BTreeMap::new(),
                last: idle,
                ack_input: self.tick,
                started: false,
                ready: false,
                acked_snapshot: None,
                sent: SnapshotHistory::default(),
                known_edits: self.terrain.edit_count(),
                inventory_sent: None,
            },
        );
        self.outgoing.push(Outgoing {
            client: id,
            channel: Channel::Reliable,
            bytes: welcome_bytes,
        });
        Ok(id)
    }

    /// Accepts or rejects the raw first packet of a connection.
    pub fn join_packet(&mut self, bytes: &[u8]) -> Result<u8, Reject> {
        let hello = ls_net::accept_hello(bytes)?;
        self.join(&hello)
    }

    /// Removes a client and its ship, and drops packets still queued for it
    /// (its wire id may be handed to the next client that joins). Returns
    /// false for unknown ids.
    pub fn leave(&mut self, client: u8) -> bool {
        match self.clients.remove(&client) {
            Some(c) => {
                self.world.remove_player(c.player);
                self.outgoing.retain(|o| o.client != client);
                true
            }
            None => false,
        }
    }

    // ----- receiving ---------------------------------------------------------

    /// Decodes and handles a packet from a joined client.
    pub fn handle_packet(&mut self, client: u8, bytes: &[u8]) -> Result<(), GameError> {
        let msg = Message::decode(bytes)?;
        self.handle_message(client, msg)
    }

    /// Handles a decoded message from a joined client.
    pub fn handle_message(&mut self, client: u8, msg: Message) -> Result<(), GameError> {
        if !self.clients.contains_key(&client) {
            return Err(GameError::Protocol(format!("unknown client {client}")));
        }
        match msg {
            Message::Input(m) => {
                self.on_input(client, &m);
                Ok(())
            }
            Message::Ping {
                nonce,
                client_time_ms,
            } => {
                let pong = Message::Pong {
                    nonce,
                    client_time_ms,
                    server_tick: self.tick as u32,
                };
                self.push_message(client, &pong);
                Ok(())
            }
            Message::Disconnect { .. } => {
                self.leave(client);
                Ok(())
            }
            Message::SelectShip { ship_type } => {
                // Types the rule set does not allow are ignored (the client
                // shows the allowed ones; a stale choice is not an error).
                if let (Some(t), Some(c)) =
                    (ShipType::from_index(ship_type), self.clients.get(&client))
                {
                    let _ = self.world.set_ship_type(c.player, t);
                }
                Ok(())
            }
            other => Err(GameError::Protocol(format!(
                "unexpected message from client {client}: {}",
                message_name(&other)
            ))),
        }
    }

    fn on_input(&mut self, client: u8, m: &InputMsg) {
        let now = self.tick;
        let max_lead = self.cfg.max_input_lead;
        let max_rewind = self.cfg.lag_comp_max_ticks as i64;
        let interp = self.cfg.interp_delay_ticks as i64;
        let snapshot_interval = self.cfg.snapshot_interval.max(1) as i64;
        let Some(c) = self.clients.get_mut(&client) else {
            return;
        };
        c.ready = true;
        if let Some(s) = m.ack_snapshot {
            if c.acked_snapshot.is_none_or(|a| s > a) {
                if let Some(snap) = c.sent.get(s) {
                    c.known_edits = c.known_edits.max(snap.edit_end());
                    c.acked_snapshot = Some(s);
                }
            }
        }
        // What the client saw when it made each command (older redundant
        // copies were made on earlier frames, so each gets its own rewind).
        let view = m
            .ack_snapshot
            .map(|s| view_tick(s, interp, snapshot_interval));
        for cmd in &m.cmds {
            let rewind = view.map_or(0, |v| (cmd.tick as i64 - v).clamp(0, max_rewind) as u64);
            let t = cmd.tick as u64;
            if t <= now {
                self.stats.late_inputs += 1;
            } else if t > now + max_lead {
                self.stats.dropped_inputs += 1;
            } else {
                match c.inputs.entry(t) {
                    Entry::Occupied(_) => self.stats.duplicate_inputs += 1,
                    Entry::Vacant(v) => {
                        v.insert(queued(cmd, rewind));
                    }
                }
            }
        }
    }

    // ----- simulation --------------------------------------------------------

    /// Advances the match by one 60 Hz tick: collects one input per client
    /// (repeating the last one if missing), applies lag compensation, steps
    /// the world and, every `snapshot_interval` ticks, queues events for
    /// every client and snapshots for every client that has sent an input
    /// (see `ClientState::ready`).
    pub fn step(&mut self) -> StepReport {
        self.tick += 1;
        let k = self.tick;
        let running = self.world.mode().is_running();

        let mut inputs: BTreeMap<PlayerId, InputCmd> = BTreeMap::new();
        let mut applied_rewind: BTreeMap<PlayerId, u64> = BTreeMap::new();
        for c in self.clients.values_mut() {
            c.inputs.retain(|&t, _| t >= k);
            match c.inputs.remove(&k) {
                Some(q) => {
                    c.last = q;
                    c.ack_input = k;
                    c.started = true;
                }
                None if c.started => self.stats.missing_inputs += 1,
                None => {}
            }
            let cmd = InputCmd {
                tick: k,
                ..c.last.cmd
            };
            inputs.insert(c.player, cmd);
            applied_rewind.insert(c.player, c.last.rewind);
        }

        let mut report = StepReport {
            tick: k,
            events: Vec::new(),
            compensated: Vec::new(),
            snapshot: false,
        };
        if running && self.world.tick() + 1 == k {
            if self.cfg.lag_compensation {
                report.compensated = lagcomp::plan_compensation(
                    &self.world,
                    &self.terrain,
                    &inputs,
                    &self.history,
                    &self.rewinds,
                );
            }
            for d in &report.compensated {
                if let Some(s) = self.world.ship_mut(d.player) {
                    s.pos += d.offset;
                }
            }
            let list: Vec<(PlayerId, InputCmd)> = inputs.into_iter().collect();
            report.events = self.world.step(&mut self.terrain, &list);
            for (player, delta) in restore_offsets(&report.compensated, &report.events) {
                self.world.shift_ship(player, delta);
            }
            self.stats.compensated_hits += report.compensated.len() as u64;
            for e in &report.events {
                if let Event::Fired {
                    player, projectile, ..
                } = e
                {
                    let r = applied_rewind.get(player).copied().unwrap_or(0);
                    self.rewinds.insert(*projectile, r);
                }
            }
            let live: std::collections::BTreeSet<u32> =
                self.world.projectiles().iter().map(|p| p.id).collect();
            self.rewinds.retain(|id, _| live.contains(id));
        }
        self.history.record(k, &self.world);
        let net_events = events_to_net(&report.events, k, |p| wire_id(&self.clients, p));
        self.pending_events.extend(net_events);

        if k % self.cfg.snapshot_interval.max(1) == 0 {
            self.send_snapshots();
            report.snapshot = true;
        }
        report
    }

    fn send_snapshots(&mut self) {
        let tick = self.tick as u32;
        let cov = self.world.mode().coverage;
        let scores: Vec<u32> = [cov.team_cells[0], cov.team_cells[1], cov.surface_cells]
            .iter()
            .map(|&v| v.min(u32::MAX as u64) as u32)
            .collect();
        let match_ticks_left = self.world.mode().remaining_ticks(self.world.tick()) as u32;
        // Wire ids (`id as u16`) are unique among live projectiles: the
        // simulation's allocator skips live ones (`World::alloc_projectile_id`).
        let mut projectiles: Vec<_> = self
            .world
            .projectiles()
            .iter()
            .map(|p| projectile_to_net(p, wire_id(&self.clients, p.owner)))
            .collect();
        projectiles.sort_by_key(|p| p.id);
        debug_assert!(projectiles.windows(2).all(|w| w[0].id < w[1].id));
        projectiles.dedup_by_key(|p| p.id);
        let entities: Vec<NetEntity> = self
            .world
            .entity_views()
            .iter()
            .map(|v| entity_to_net(v, v.owner.and_then(|o| wire_id(&self.clients, o))))
            .collect();
        let players: Vec<ls_net::NetPlayer> = self
            .clients
            .iter()
            .filter_map(|(&id, c)| {
                let s = self.world.ship(c.player)?;
                Some(NetConv::player_to_net(id, &c.name, s))
            })
            .collect();
        let log = self.terrain.edit_log();

        let mut inventories: Vec<(u8, Message)> = Vec::new();
        // Clients still setting up (no input yet) get no snapshots.
        let ids: Vec<u8> = self
            .clients
            .iter()
            .filter(|(_, c)| c.ready)
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            let ships = self
                .clients
                .iter()
                .filter_map(|(&sid, c)| {
                    let s = self.world.ship(c.player)?;
                    Some(self.conv.ship_to_net(sid, s, sid == id))
                })
                .collect();
            let c = &self.clients[&id];
            let from = (c.known_edits as usize).min(log.len());
            let to = (from + self.cfg.max_edits_per_snapshot).min(log.len());
            let mut snap = Snapshot {
                mode: Default::default(),
                tick,
                ack_input: c.ack_input as u32,
                edit_base: from as u32,
                edits: log[from..to].to_vec(),
                ships,
                projectiles: projectiles.clone(),
                entities: entities.clone(),
                players: players.clone(),
                scores: scores.clone(),
                match_ticks_left,
            };
            snap.retain_relevant(
                id,
                &SimRelevance {
                    world: &self.world,
                    clients: &self.clients,
                },
            );
            let mut packed = self.conv.quantizer().pack_snapshot(&snap);
            let baseline = c.sent.choose_baseline(tick, c.acked_snapshot);
            // Straight projectiles within a few position steps of their
            // predicted path go out as predicted (one bit each); what is
            // sent is what the client stores, so both stay identical.
            if let Some(b) = baseline.and_then(|t| c.sent.get(t)) {
                ls_net::settle_projectiles(&mut packed, b, ls_net::SETTLE_TOLERANCE);
            }
            let msg = Message::Snapshot(Box::new(SnapshotMsg {
                epoch: 0,
                baseline,
                snapshot: packed.clone(),
            }));
            match msg.encode_with(&c.sent) {
                Ok(bytes) => {
                    self.stats.snapshots_sent += 1;
                    self.stats.snapshot_bytes += bytes.len() as u64;
                    self.outgoing.push(Outgoing {
                        client: id,
                        channel: Channel::Unreliable,
                        bytes,
                    });
                }
                // Only reachable with inconsistent state; skip this client's snapshot.
                Err(_) => {
                    self.stats.encode_errors += 1;
                    continue;
                }
            }
            if let Some(c) = self.clients.get_mut(&id) {
                c.sent.push(packed);
            }
            // The own inventory, reliably, whenever it changed.
            let inv = self
                .world
                .ship(self.clients[&id].player)
                .map(|s| inventory_msg(tick, s));
            if let Some(inv) = inv {
                let c = &self.clients[&id];
                let changed = c.inventory_sent.as_ref().is_none_or(|old| {
                    InventoryMsg {
                        tick,
                        ..old.clone()
                    } != inv
                });
                if changed {
                    inventories.push((id, Message::Inventory(inv.clone())));
                    if let Some(c) = self.clients.get_mut(&id) {
                        c.inventory_sent = Some(inv);
                    }
                }
            }
        }

        for (id, msg) in inventories {
            self.push_message(id, &msg);
        }
        let events = std::mem::take(&mut self.pending_events);
        for chunk in events.chunks(MAX_EVENTS) {
            let msg = Message::Events(chunk.to_vec());
            let ids: Vec<u8> = self.clients.keys().copied().collect();
            for id in ids {
                self.push_message(id, &msg);
            }
        }
    }

    fn push_message(&mut self, client: u8, msg: &Message) {
        match msg.encode() {
            Ok(bytes) => self.outgoing.push(Outgoing {
                client,
                channel: msg.channel(),
                bytes,
            }),
            Err(_) => self.stats.encode_errors += 1,
        }
    }

    /// Server tick at which a client whose newest snapshot is `ack` saw the
    /// remote ships when it made an input (see [`view_tick`]).
    pub fn view_tick(&self, ack: u32) -> i64 {
        view_tick(
            ack,
            self.cfg.interp_delay_ticks as i64,
            self.cfg.snapshot_interval.max(1) as i64,
        )
    }

    /// Packets queued since the last call, in order.
    pub fn take_outgoing(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outgoing)
    }

    // ----- lag-compensated queries -----------------------------------------

    /// Rewind (ticks) recorded for a projectile in flight when it was fired.
    pub fn projectile_rewind(&self, projectile: u32) -> Option<u64> {
        self.rewinds.get(&projectile).copied()
    }

    /// Position of `player` after `tick`, from the lag compensation history.
    pub fn rewound_position(&self, player: PlayerId, tick: u64) -> Option<Vec3> {
        self.history.position(player, tick)
    }

    /// Hit test of a ray against ships rewound by `rewind_ticks` (capped at
    /// the lag compensation window) and the current terrain: the nearest ship
    /// (other than `shooter`) whose rewound sphere of `radius` the ray hits
    /// before any terrain, with the distance. For hitscan validation.
    pub fn lag_compensated_raycast(
        &self,
        shooter: PlayerId,
        origin: Vec3,
        dir: Vec3,
        max_dist: f32,
        radius: f32,
        rewind_ticks: u64,
    ) -> Option<(PlayerId, f32)> {
        let dir = dir.try_normalize()?;
        let wall = self
            .terrain
            .raycast(origin, dir, max_dist)
            .map_or(max_dist, |h| h.distance);
        let then = self
            .tick
            .saturating_sub(rewind_ticks.min(self.cfg.lag_comp_max_ticks));
        let frame = self.history.frame(then)?;
        frame
            .ships
            .iter()
            .filter(|(p, _, alive)| *alive && *p != shooter)
            .filter_map(|(p, pos, _)| {
                segment_sphere(origin, dir, wall, *pos, radius).map(|t| (*p, t))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

/// Where a client renders remote ships when its newest snapshot is `ack`:
/// `ClientSession` renders them at its server time estimate minus the
/// interpolation delay, and that estimate runs on from the newest snapshot
/// with local time until the next one arrives (`snapshot_interval` ticks
/// later). On average it is half a snapshot interval past `ack`, so the view
/// is `ack - interp + snapshot_interval / 2` (rounded down). The remaining
/// error is below one tick on average; sending the exact render tick would
/// need an `InputMsg` field.
pub fn view_tick(ack: u32, interp: i64, snapshot_interval: i64) -> i64 {
    ack as i64 - interp + snapshot_interval / 2
}

/// The inventory message for a ship (see [`Message::Inventory`]).
fn inventory_msg(tick: u32, s: &ShipState) -> InventoryMsg {
    InventoryMsg {
        tick,
        weapons: s
            .weapons
            .iter()
            .map(|w| NetWeapon {
                shots: 0,
                state: 0,
                weapon: w.weapon.0,
                ammo: w.ammo.min(u16::MAX as u32) as u16,
                level: w.level,
            })
            .collect(),
        primary: s.primary.map_or(u8::MAX, |w| w.0),
        secondary: s.secondary.map_or(u8::MAX, |w| w.0),
        gadget: s.gadget.map_or(u8::MAX, |g| g.0),
    }
}

/// Moving debris farther than this (u) from a viewer's ship is left out of
/// its snapshots (resting debris costs one bit and stays).
pub const DEBRIS_RELEVANCE_RADIUS: f32 = 100.0;

/// Per-client snapshot relevance from the simulation: ships hidden by a
/// gadget (Cloak) are left out for the viewers that must not see them, and
/// moving debris far from the viewer.
struct SimRelevance<'a> {
    world: &'a World,
    clients: &'a BTreeMap<u8, ClientState>,
}

impl Relevance for SimRelevance<'_> {
    fn ship(&self, viewer: u8, ship: &NetShip) -> bool {
        let (Some(v), Some(t)) = (self.clients.get(&viewer), self.clients.get(&ship.id)) else {
            return true;
        };
        self.world.ship_visible_to(v.player, t.player)
    }

    fn entity(&self, viewer: u8, e: &NetEntity) -> bool {
        // Debris state byte 1 is "resting".
        if e.kind != ReplicatedKind::Debris as u8 || e.state[1] != 0 {
            return true;
        }
        let Some(me) = self
            .clients
            .get(&viewer)
            .and_then(|v| self.world.ship(v.player))
        else {
            return true;
        };
        me.pos.distance_squared(e.pos) <= DEBRIS_RELEVANCE_RADIUS * DEBRIS_RELEVANCE_RADIUS
    }
}

fn queued(cmd: &NetInput, rewind: u64) -> QueuedInput {
    QueuedInput {
        cmd: net_to_input(cmd),
        rewind,
    }
}

fn wire_id(clients: &BTreeMap<u8, ClientState>, player: PlayerId) -> Option<u8> {
    clients
        .iter()
        .find(|(_, c)| c.player == player)
        .map(|(&id, _)| id)
}

fn message_name(m: &Message) -> &'static str {
    m.name()
}

/// The moves that undo lag compensation after a step: each compensated ship
/// goes back by its offset, except that ships teleported by the step keep
/// their new place. A Swap Harpoon swap (`CueKind::Swapped`) exchanges the
/// two ships' present positions, so each takes the other's correction; a
/// Borrowed Time rewind (`CueKind::Rewound`) put the ship at a recorded
/// position, which needs none.
fn restore_offsets(
    compensated: &[lagcomp::Displacement],
    events: &[Event],
) -> Vec<(PlayerId, Vec3)> {
    let mut restore: BTreeMap<PlayerId, Vec3> = BTreeMap::new();
    for d in compensated {
        *restore.entry(d.player).or_insert(Vec3::ZERO) -= d.offset;
    }
    for e in events {
        let Event::Cue(c) = e else { continue };
        match (c.kind, c.source, c.target) {
            (ls_sim::CueKind::Swapped, Some(a), Some(b)) => {
                let da = restore.remove(&a).unwrap_or(Vec3::ZERO);
                let db = restore.remove(&b).unwrap_or(Vec3::ZERO);
                restore.insert(a, db);
                restore.insert(b, da);
            }
            (ls_sim::CueKind::Rewound, _, Some(t)) => {
                restore.insert(t, Vec3::ZERO);
            }
            _ => {}
        }
    }
    restore
        .into_iter()
        .filter(|(_, d)| *d != Vec3::ZERO)
        .collect()
}
#[cfg(test)]
mod tests;
