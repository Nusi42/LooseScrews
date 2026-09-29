//! [`ClientSession`]: everything a client (browser via `ls-wasm`, or a bot)
//! needs after the handshake, as pure logic. The host feeds it received
//! packets and a fixed-step clock and sends the messages it returns.

use std::collections::VecDeque;

use ls_math::terrain::TerrainEdit;
use ls_math::{Quat, Vec3};
use ls_net::{Message, NetEvent, NetProjectile, NetShip, Snapshot, SnapshotHistory, Welcome};
use ls_sim::{InputCmd, RuleSet, ShipState, ShipTuning, WeaponTable};
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
}

/// Client state after `Welcome`: local terrain, predicted local ship,
/// interpolated remote ships, latest snapshot and outgoing inputs.
#[derive(Debug, Clone)]
pub struct ClientSession {
    terrain: GameTerrain,
    conv: NetConv,
    rules: RuleSet,
    weapons: WeaponTable,
    player: u8,
    team: u8,
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
}

impl ClientSession {
    /// Builds the session from the server's `Welcome`: regenerates the level
    /// from the share code, replays the edit log and parses the rule set.
    /// `weapons` and `tuning` are the client's data tables (built-in for M1).
    pub fn from_welcome(
        welcome: &Welcome,
        weapons: WeaponTable,
        tuning: ShipTuning,
    ) -> Result<Self, GameError> {
        let code = ShareCode::decode(&welcome.share_code)?;
        let terrain = GameTerrain::replay(Generator::from_code(&code)?, &welcome.edit_log);
        let text = std::str::from_utf8(&welcome.ruleset)
            .map_err(|_| GameError::Protocol("rule set is not UTF-8".into()))?;
        let rules = RuleSet::from_ron(&welcome.ruleset_name, text)?;
        rules.resolve(&weapons)?;
        let conv = NetConv::new(welcome.quant, tuning.clone())?;
        let config = PredictionConfig::for_conv(&conv);
        // Spawn points come from the pristine level, like on the server.
        let setup = match_setup(&code, &rules, &GameTerrain::from_code(&code)?)?;
        let mut predictor = Predictor::new(tuning, config);
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
            player: welcome.player_id,
            team: welcome.team,
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
            conv,
        })
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
        self.predictor.predict(cmd, &self.terrain);
        let ack = self.newest_tick();
        Message::Input(self.inputs.message(ack).expect("a command was just added"))
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
        let msg = Message::decode_with(bytes, &self.received)?;
        self.handle_message(msg)
    }

    /// Handles one decoded message from the server.
    pub fn handle_message(&mut self, msg: Message) -> Result<Incoming, GameError> {
        match msg {
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
        let end = match self.match_end_tick {
            Some(end) if snap.match_ticks_left == 0 => Some(end.min(tick as u64)),
            Some(end) => Some(end),
            None if snap.match_ticks_left == 0 => Some(tick as u64),
            None => None,
        };
        if let Some(end) = end {
            self.predictor.freeze_after(end, &self.terrain);
        }
        let reconcile = snap.ships.iter().find(|s| s.id == self.player).map(|own| {
            self.team = ls_net::ship_flags::team(own.flags);
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
