//! [`BotCore`]: one connected bot as pure logic (no I/O). It owns the
//! [`ClientSession`] (prediction, interpolation, terrain) and the [`Brain`],
//! turns received packets into statistics and desync checks, and produces
//! the messages to send each 60 Hz tick. The WebSocket runner
//! ([`crate::net`]) and the in-process tests drive it the same way.

use ls_game::client::{ClientSession, Incoming, Reconcile};
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::{Pcg32, Vec3};
use ls_game::ls_net::{ship_flags, Hello, Message, NetEvent, Welcome};
use ls_game::ls_sim::{Buttons, CueKind, InputCmd, ShipTuning, ShipType, WeaponId, WeaponTable};
use ls_game::ls_worldgen::ShareCode;
use ls_game::{match_setup, GameError, GameTerrain};
use thiserror::Error;

use crate::brain::{Arsenal, Brain, BrainConfig, Perception};
use crate::stats::BotStats;

/// Server ticks around an own death or respawn event in which a
/// prediction snap is expected.
const LIFE_EVENT_WINDOW: u64 = 240;
/// Server ticks after a starvation report in which a snap counts as lag.
const STARVATION_WINDOW: u64 = 180;
/// Server ticks a snap waits for a teleport cue (Swap Harpoon, Borrowed
/// Time) before it counts as a desync: events arrive after the snapshot
/// that shows the new position.
const TELEPORT_GRACE: u64 = 60;

/// Stream id of the RNG that picks a bot's ship type ("BOT_SHIP").
const RNG_STREAM_SHIP: u64 = 0x424f_545f_5348_4950;

/// The ship type a bot with this seed asks for (uniform over all types;
/// the server falls back if the rule set does not allow it, and
/// [`BotCore`] then picks an allowed one).
pub fn preferred_ship(seed: u64) -> ShipType {
    let mut rng = Pcg32::from_seed(seed, RNG_STREAM_SHIP);
    ShipType::ALL[rng.below(ShipType::ALL.len() as u32) as usize]
}

/// The `Hello` a bot sends: its name, the built-in data hash and the ship
/// type [`preferred_ship`] picks for `seed` (pass the seed later given to
/// [`BotCore::from_welcome`]).
pub fn bot_hello(name: impl Into<String>, seed: u64) -> Hello {
    ls_game::client_hello(name, None, Some(preferred_ship(seed)))
}

/// Why a session has to end.
#[derive(Debug, Error)]
pub enum BotError {
    /// The server sent something this client cannot handle.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// Client and server state diverged beyond repair (edit gap).
    #[error("desync: {0}")]
    Desync(String),
}

/// What a handled packet means for the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// The server ended the session.
    Disconnected(String),
}

/// One bot in one session. See the module docs.
#[derive(Debug)]
pub struct BotCore {
    session: ClientSession,
    brain: Brain,
    prev_alive: Option<bool>,
    /// Server tick of the newest own death, respawn or teleport (Swap
    /// Harpoon swap, Borrowed Time rewind) event.
    last_life_event: Option<u64>,
    /// An unexplained snap (server tick, error) waiting [`TELEPORT_GRACE`]
    /// for a teleport cue before it counts as a desync.
    pending_snap: Option<(u64, f32)>,
    prev_starved: u64,
    last_starved_tick: Option<u64>,
    /// Terrain edits already passed to the brain's navigation grid.
    edits_seen: usize,
    /// Ship type to ask the server for (sent with the next tick).
    pending_ship: Option<ShipType>,
    /// Ids of own projectiles already counted as shots.
    own_projectiles: Vec<u16>,
    /// Alive in the previous tick (lives per ship type).
    was_alive: bool,
}

impl BotCore {
    /// Builds the client session from the server's `Welcome` and a brain.
    /// `seed` varies the behavior.
    ///
    /// Scrap pickups spawn on the match's pickup points, which every peer
    /// derives from the pristine level (see `docs/architecture/game-glue.md`);
    /// the session computes them while it builds the match setup. The bot
    /// derives the spawn points the same way (hunting near enemy spawns).
    pub fn from_welcome(welcome: &Welcome, seed: u64) -> Result<BotCore, GameError> {
        let session =
            ClientSession::from_welcome(welcome, WeaponTable::builtin(), ShipTuning::tinker())?;
        let code = ShareCode::decode(&welcome.share_code)?;
        let setup = match_setup(&code, session.rules(), &GameTerrain::from_code(&code)?)?;
        let mut brain = Brain::new(
            BrainConfig::default(),
            Arsenal::from_table(session.weapons()),
            session.conv().tuning().clone(),
            session.terrain().bounds(),
            session.pickup_points().to_vec(),
            seed,
        );
        brain.set_spawn_points(setup.spawns.iter().map(|s| (s.team, s.pos)).collect());
        // A random allowed ship type: the one asked for in the `Hello` if
        // the rule set allows it, else another allowed one.
        let allowed = &session.rules().allowed_ship_types;
        let wanted = preferred_ship(seed);
        let ship = if allowed.is_empty() || allowed.contains(&wanted) {
            wanted
        } else {
            let mut rng = Pcg32::from_seed(seed, RNG_STREAM_SHIP ^ 1);
            allowed[rng.below(allowed.len() as u32) as usize]
        };
        let pending_ship = (ship != session.ship_type()).then_some(ship);
        // The grid is built from the session's terrain, which already holds
        // the edits from the `Welcome`.
        let edits_seen = session.terrain().edit_log().len();
        Ok(BotCore {
            session,
            brain,
            prev_alive: None,
            last_life_event: None,
            pending_snap: None,
            prev_starved: 0,
            last_starved_tick: None,
            edits_seen,
            pending_ship,
            own_projectiles: Vec::new(),
            was_alive: false,
        })
    }

    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    pub fn brain(&self) -> &Brain {
        &self.brain
    }

    /// Runs one 60 Hz tick: decides the controls and advances the session.
    /// Returns the messages to send, in order.
    pub fn tick(&mut self, stats: &mut BotStats) -> Vec<Message> {
        let ship_type = self.session.ship_type();
        self.brain
            .set_ship(ship_type, self.session.conv().tuning_for(ship_type));
        let cmd = match Perception::from_session(&self.session) {
            Some(p) if self.session.is_synced() => {
                if p.alive && !self.was_alive {
                    *stats
                        .ship_lives
                        .entry(ship_type.name().to_string())
                        .or_default() += 1;
                }
                self.was_alive = p.alive;
                self.brain
                    .think(&p, self.session.terrain(), &mut stats.behavior)
            }
            _ => InputCmd::idle(0).with_buttons(Buttons::HORIZON_ASSIST),
        };
        let mut msgs = Vec::new();
        if let Some(t) = self.pending_ship.take() {
            msgs.push(self.session.select_ship(t));
        }
        msgs.extend(self.session.frame(1, cmd));
        for m in &msgs {
            match m {
                Message::Input(_) => stats.inputs_sent += 1,
                Message::Ping { .. } => stats.pings_sent += 1,
                _ => {}
            }
        }
        msgs
    }

    /// Handles one packet from the server. `Err` means the session must end
    /// (the error has been counted in `stats`).
    pub fn handle_packet(&mut self, bytes: &[u8], stats: &mut BotStats) -> Result<Flow, BotError> {
        let result = self.session.handle_packet(bytes);
        // Bots never render: drop the remesh queue so it cannot grow.
        self.session.terrain_mut().take_dirty_chunks();
        match result {
            Err(GameError::EditGap { have, base }) => {
                let msg = format!("terrain edit gap: have {have} edits, snapshot starts at {base}");
                stats.desyncs += 1;
                stats.note_error(msg.clone());
                Err(BotError::Desync(msg))
            }
            Err(e) => {
                let msg = e.to_string();
                stats.protocol_errors += 1;
                stats.note_error(msg.clone());
                Err(BotError::Protocol(msg))
            }
            Ok(Incoming::Snapshot {
                tick,
                stale,
                reconcile,
                new_edits,
            }) => {
                stats.snapshots += 1;
                stats.edits_applied += new_edits as u64;
                self.forward_edits();
                if stale {
                    stats.stale_snapshots += 1;
                } else {
                    self.on_fresh_snapshot(tick as u64, reconcile, stats);
                    self.count_shots(stats);
                }
                Ok(Flow::Continue)
            }
            Ok(Incoming::Events(_)) => {
                stats.event_messages += 1;
                self.on_events(stats);
                Ok(Flow::Continue)
            }
            Ok(Incoming::Pong { .. }) => {
                stats.pongs += 1;
                Ok(Flow::Continue)
            }
            Ok(Incoming::Disconnected(reason)) => Ok(Flow::Disconnected(reason)),
            Ok(Incoming::Inventory | Incoming::Ignored | Incoming::Lobby | Incoming::Results) => {
                Ok(Flow::Continue)
            }
            // The session rebuilt itself for the next match, but the brain
            // (navigation grid, spawn points) belongs to the old level: until
            // bots handle transitions in place (M3 wave C), rejoin as after a
            // match restart.
            Ok(Incoming::MatchTransition { .. }) => Ok(Flow::Disconnected(
                ls_game::MATCH_RESTART_REASON.to_string(),
            )),
        }
    }

    /// Adds this session's snapshot kind counters to `stats` (call once when
    /// the session ends).
    pub fn finish(&self, stats: &mut BotStats) {
        if let Some((tick, error)) = self.pending_snap {
            self.judge_snap(tick, error, stats);
        }
        let s = self.session.stats();
        stats.full_snapshots += s.full_snapshots;
        stats.delta_snapshots += s.delta_snapshots;
    }

    /// Passes terrain edits the brain has not seen to its navigation grid.
    fn forward_edits(&mut self) {
        let log = self.session.terrain().edit_log();
        for e in log.iter().skip(self.edits_seen) {
            if e.changes_geometry() {
                self.brain.on_terrain_edit(e);
            }
        }
        self.edits_seen = log.len();
    }

    fn on_fresh_snapshot(&mut self, tick: u64, reconcile: Option<Reconcile>, stats: &mut BotStats) {
        if let Some((t, e)) = self.pending_snap {
            if tick > t + TELEPORT_GRACE {
                self.pending_snap = None;
                self.judge_snap(t, e, stats);
            }
        }
        let starved = self.session.clock().starved;
        if starved > self.prev_starved {
            stats.starved_snapshots += starved - self.prev_starved;
            self.prev_starved = starved;
            self.last_starved_tick = Some(tick);
        }
        let player = self.session.player();
        let alive = self
            .session
            .latest_snapshot()
            .and_then(|s| s.ships.iter().find(|n| n.id == player))
            .map(|n| n.flags & ship_flags::ALIVE != 0);
        let near = |t: Option<u64>, window: u64| t.is_some_and(|t| tick.abs_diff(t) <= window);
        match reconcile {
            Some(Reconcile::Accepted) => stats.accepted += 1,
            Some(Reconcile::Corrected { error }) => {
                stats.corrections += 1;
                stats.max_reconcile_error = stats.max_reconcile_error.max(error);
            }
            Some(Reconcile::Snapped) => {
                stats.snaps += 1;
                let error = self.session.prediction_stats().last_error;
                let snap_distance = self.session.predictor().config().snap_distance;
                tracing::debug!(
                    bot = %stats.name,
                    tick,
                    error,
                    predicted_up_to = ?self.session.predictor().latest_tick(),
                    alive = ?alive,
                    prev_alive = ?self.prev_alive,
                    "prediction snapped"
                );
                let life_change = self.prev_alive.is_none()
                    || alive.is_none()
                    || self.prev_alive != alive
                    || alive == Some(false)
                    || near(self.last_life_event, LIFE_EVENT_WINDOW);
                if !life_change && error > snap_distance {
                    if near(self.last_starved_tick, STARVATION_WINDOW) {
                        stats.lag_snaps += 1;
                    } else {
                        // Judged once a teleport cue had time to arrive.
                        if let Some((t, e)) = self.pending_snap.take() {
                            self.judge_snap(t, e, stats);
                        }
                        self.pending_snap = Some((tick, error));
                    }
                }
            }
            Some(Reconcile::Ignored) | None => {}
        }
        if alive.is_some() {
            self.prev_alive = alive;
        }
    }

    /// A snap by `error` u at server tick `tick` that no death, respawn or
    /// starvation explained: a desync unless a teleport cue (or a life
    /// event) arrived for it meanwhile.
    fn judge_snap(&self, tick: u64, error: f32, stats: &mut BotStats) {
        if self
            .last_life_event
            .is_some_and(|t| tick.abs_diff(t) <= LIFE_EVENT_WINDOW)
        {
            return;
        }
        stats.desyncs += 1;
        stats.max_reconcile_error = stats.max_reconcile_error.max(error);
        let msg = format!("prediction snapped by {error:.2} u at tick {tick} without a respawn");
        tracing::warn!(bot = %stats.name, "{msg}");
        stats.note_error(msg);
    }

    /// Data id of weapon `w`, if it is one.
    fn weapon_name(&self, w: u8) -> Option<String> {
        let table = self.session.weapons();
        ((w as usize) < table.len()).then(|| table.get(WeaponId(w)).id.clone())
    }

    /// Counts own projectiles new in the newest snapshot as shots, one per
    /// weapon per snapshot. The Railspike is counted by its muzzle flash
    /// cue instead (its spikes can hit before a snapshot shows them).
    fn count_shots(&mut self, stats: &mut BotStats) {
        let me = self.session.player();
        let Some(snap) = self.session.latest_snapshot() else {
            return;
        };
        let mut fresh: Vec<u8> = Vec::new();
        let mut ids: Vec<u16> = Vec::new();
        for q in snap.projectiles.iter().filter(|q| q.owner == me) {
            ids.push(q.id);
            if !self.own_projectiles.contains(&q.id) && !fresh.contains(&q.kind) {
                fresh.push(q.kind);
            }
        }
        self.own_projectiles = ids;
        for k in fresh {
            match self.weapon_name(k) {
                Some(name) if name != "railspike" => stats.weapon_mut(&name).fired += 1,
                _ => {}
            }
        }
    }

    fn on_events(&mut self, stats: &mut BotStats) {
        let me = self.session.player();
        let now = self.session.next_input_tick();
        for e in self.session.take_events() {
            match e {
                NetEvent::Hit {
                    attacker,
                    victim,
                    weapon,
                    ..
                } => {
                    if attacker == me && victim != me {
                        stats.hits_dealt += 1;
                        if let Some(name) = self.weapon_name(weapon) {
                            stats.weapon_mut(&name).hits += 1;
                        }
                    }
                    if victim == me {
                        stats.hits_taken += 1;
                        let w = ((weapon as usize) < self.session.weapons().len())
                            .then_some(WeaponId(weapon));
                        self.brain.on_damaged(w, now);
                    }
                }
                NetEvent::Kill {
                    tick,
                    killer,
                    victim,
                    weapon,
                } => {
                    if victim == me {
                        stats.deaths += 1;
                        self.last_life_event = Some(tick as u64);
                    } else if killer == me {
                        stats.kills += 1;
                        if let Some(name) = self.weapon_name(weapon) {
                            stats.weapon_mut(&name).kills += 1;
                        }
                    }
                    if let Some(pos) = self.ship_position(victim) {
                        let now = self.session.next_input_tick();
                        self.brain.on_wreck(pos, now);
                    }
                }
                NetEvent::Respawn { tick, ship, .. } => {
                    if ship == me {
                        self.last_life_event = Some(tick as u64);
                    }
                }
                NetEvent::MatchOver { .. } => stats.matches_over += 1,
                NetEvent::Cue {
                    tick,
                    kind,
                    source,
                    target,
                    pos,
                    value,
                } => {
                    let kind = CueKind::from_u8(kind);
                    // Server-side teleports of the own ship (Swap Harpoon,
                    // Borrowed Time) snap the prediction like a respawn.
                    let teleported = match kind {
                        CueKind::Swapped => source == me || target == me,
                        CueKind::Rewound => target == me,
                        _ => false,
                    };
                    if teleported {
                        self.last_life_event = Some(tick as u64);
                    }
                    if source == me {
                        // A beam cue covers `value` ticks of Leech Arc arcs
                        // (one every fire interval); a muzzle flash is one
                        // Railspike shot.
                        let fired = match kind {
                            CueKind::Beam => {
                                let table = self.session.weapons();
                                let arcs = table.find("leech_arc").map_or(1, |w| {
                                    let every = table.get(w).fire_interval_ticks().max(1);
                                    (u64::from(value) / every).max(1)
                                });
                                Some(("leech_arc", arcs))
                            }
                            CueKind::MuzzleFlash => Some(("railspike", 1)),
                            _ => None,
                        };
                        if let Some((name, n)) = fired {
                            stats.weapon_mut(name).fired += n;
                        }
                    }
                    self.brain.on_cue(kind, pos, value, now);
                }
            }
        }
    }

    fn ship_position(&self, id: u8) -> Option<Vec3> {
        if id == self.session.player() {
            return self.session.local_ship().map(|s| s.pos);
        }
        self.session
            .latest_snapshot()?
            .ships
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.pos)
    }
}
