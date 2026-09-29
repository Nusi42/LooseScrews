//! [`BotCore`]: one connected bot as pure logic (no I/O). It owns the
//! [`ClientSession`] (prediction, interpolation, terrain) and the [`Brain`],
//! turns received packets into statistics and desync checks, and produces
//! the messages to send each 60 Hz tick. The WebSocket runner
//! ([`crate::net`]) and the in-process tests drive it the same way.

use ls_game::client::{ClientSession, Incoming, Reconcile};
use ls_game::ls_math::terrain::TerrainQuery;
use ls_game::ls_math::Vec3;
use ls_game::ls_net::{ship_flags, Message, NetEvent, Welcome};
use ls_game::ls_sim::{Buttons, InputCmd, ShipTuning, WeaponTable};
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
    /// Server tick of the newest own death / respawn event.
    last_life_event: Option<u64>,
    prev_starved: u64,
    last_starved_tick: Option<u64>,
    /// Terrain edits already passed to the brain's navigation grid.
    edits_seen: usize,
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
        // The grid is built from the session's terrain, which already holds
        // the edits from the `Welcome`.
        let edits_seen = session.terrain().edit_log().len();
        Ok(BotCore {
            session,
            brain,
            prev_alive: None,
            last_life_event: None,
            prev_starved: 0,
            last_starved_tick: None,
            edits_seen,
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
        let cmd = match Perception::from_session(&self.session) {
            Some(p) if self.session.is_synced() => {
                self.brain
                    .think(&p, self.session.terrain(), &mut stats.behavior)
            }
            _ => InputCmd::idle(0).with_buttons(Buttons::HORIZON_ASSIST),
        };
        let msgs = self.session.frame(1, cmd);
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
            Ok(Incoming::Inventory | Incoming::Ignored) => Ok(Flow::Continue),
        }
    }

    /// Adds this session's snapshot kind counters to `stats` (call once when
    /// the session ends).
    pub fn finish(&self, stats: &mut BotStats) {
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
                        stats.desyncs += 1;
                        stats.max_reconcile_error = stats.max_reconcile_error.max(error);
                        let msg = format!(
                            "prediction snapped by {error:.2} u at tick {tick} without a respawn"
                        );
                        tracing::warn!(bot = %stats.name, "{msg}");
                        stats.note_error(msg);
                    }
                }
            }
            Some(Reconcile::Ignored) | None => {}
        }
        if alive.is_some() {
            self.prev_alive = alive;
        }
    }

    fn on_events(&mut self, stats: &mut BotStats) {
        let me = self.session.player();
        for e in self.session.take_events() {
            match e {
                NetEvent::Hit {
                    attacker, victim, ..
                } => {
                    if attacker == me && victim != me {
                        stats.hits_dealt += 1;
                    }
                    if victim == me {
                        stats.hits_taken += 1;
                    }
                }
                NetEvent::Kill {
                    tick,
                    killer,
                    victim,
                    ..
                } => {
                    if victim == me {
                        stats.deaths += 1;
                        self.last_life_event = Some(tick as u64);
                    } else if killer == me {
                        stats.kills += 1;
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
                NetEvent::Cue { .. } => {}
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
