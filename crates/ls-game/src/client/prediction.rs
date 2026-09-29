//! Client-side prediction of the local ship with server reconciliation and
//! visual error smoothing.
//!
//! The predictor runs `ls_sim::simulate_movement` (the same code the server
//! runs) for every local input and remembers the predicted state after each
//! tick. When a snapshot arrives with the server's state of the local ship at
//! tick `T`, it compares it with the prediction for `T`:
//!
//! - within quantization tolerance: the prediction is kept (the server state
//!   is a quantized copy of the same value, so replacing it would only add
//!   jitter);
//! - otherwise: the prediction for `T` is replaced by the server state and all
//!   inputs after `T` are simulated again (reconciliation). The jump in the
//!   current state goes into a render offset that decays over a few ticks.

use std::collections::VecDeque;

use ls_math::terrain::TerrainQuery;
use ls_math::{Quat, Vec3};
use ls_net::NetShip;
use ls_sim::{simulate_movement, InputCmd, ShipState, ShipTuning};

use crate::convert::{rotation_error, NetConv};

/// Tuning of reconciliation and smoothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredictionConfig {
    /// Position error (u) above which the prediction is corrected. Default:
    /// 1.5 position quantization steps (set by [`Predictor::new`]).
    pub pos_tolerance: f32,
    /// Velocity error (u/s per component) above which to correct.
    pub vel_tolerance: f32,
    /// Orientation error (radians) above which to correct (the wire
    /// quaternion is accurate to about 0.3 degrees).
    pub rot_tolerance: f32,
    /// Boost error above which to correct (boost changes the speed cap).
    pub boost_tolerance: f32,
    /// Corrections larger than this (u) snap instead of smoothing (respawns, teleports).
    pub snap_distance: f32,
    /// Fraction of the render offset kept per tick (0.8 = about 5 ticks to 1/3).
    pub smoothing_keep_per_tick: f32,
    /// Predicted ticks kept for reconciliation.
    pub history_ticks: usize,
}

impl PredictionConfig {
    /// Defaults matched to the quantization of `conv`.
    pub fn for_conv(conv: &NetConv) -> Self {
        let q = conv.quantizer();
        PredictionConfig {
            pos_tolerance: q.position_step().max_element() * 1.5 + 1e-4,
            vel_tolerance: q.ship_velocity_step() * 1.5 + 1e-4,
            rot_tolerance: 0.012,
            boost_tolerance: conv.tuning().boost_max / 255.0 * 1.5 + 1e-4,
            snap_distance: 4.0,
            smoothing_keep_per_tick: 0.8,
            history_ticks: 256,
        }
    }
}

/// Outcome of [`Predictor::reconcile`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reconcile {
    /// The prediction for the snapshot tick matched the server.
    Accepted,
    /// The prediction was replayed from the server state; `error` is the
    /// position error (u) at the snapshot tick.
    Corrected { error: f32 },
    /// The state was replaced without smoothing (first state, respawn, large
    /// error or no prediction for that tick).
    Snapped,
    /// The snapshot was older than the prediction history and was ignored.
    Ignored,
}

/// Counters for tests and debug overlays.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PredictionStats {
    pub accepted: u64,
    pub corrected: u64,
    pub snapped: u64,
    pub ignored: u64,
    /// Position error at the snapshot tick of the last reconcile.
    pub last_error: f32,
}

#[derive(Debug, Clone)]
struct Entry {
    tick: u64,
    cmd: InputCmd,
    /// Predicted state after applying `cmd`; `None` before the first snapshot.
    state: Option<ShipState>,
}

/// Predicts the local ship. See the module docs.
#[derive(Debug, Clone)]
pub struct Predictor {
    config: PredictionConfig,
    tuning: ShipTuning,
    /// Inputs (and predicted states) in ascending, contiguous tick order.
    entries: VecDeque<Entry>,
    /// Server-anchored state before the first entry.
    base: Option<(u64, ShipState)>,
    pos_offset: Vec3,
    rot_offset: Quat,
    stats: PredictionStats,
    spawn_poses: Vec<(Vec3, Quat)>,
    /// Ticks after this one do not move the ship (the match is over and the
    /// server no longer steps the world).
    freeze_after: Option<u64>,
}

impl Predictor {
    pub fn new(tuning: ShipTuning, config: PredictionConfig) -> Self {
        Predictor {
            config,
            tuning,
            entries: VecDeque::new(),
            base: None,
            pos_offset: Vec3::ZERO,
            rot_offset: Quat::IDENTITY,
            stats: PredictionStats::default(),
            spawn_poses: Vec::new(),
            freeze_after: None,
        }
    }

    /// Stops movement for every tick after `tick`: once a match is over the
    /// server stops stepping the world, so the ship stays where it was at
    /// the end tick. Already predicted later ticks are recomputed.
    pub fn freeze_after<T: TerrainQuery + ?Sized>(&mut self, tick: u64, terrain: &T) {
        if self.freeze_after.is_some_and(|f| f <= tick) {
            return;
        }
        self.freeze_after = Some(tick);
        if let Some((t, s)) = self.base.clone() {
            self.rebase(t, s, terrain);
        }
    }

    /// See [`Predictor::freeze_after`].
    pub fn frozen_after(&self) -> Option<u64> {
        self.freeze_after
    }

    pub fn config(&self) -> &PredictionConfig {
        &self.config
    }

    pub fn stats(&self) -> PredictionStats {
        self.stats
    }

    /// True once a server state anchors the prediction.
    pub fn is_anchored(&self) -> bool {
        self.base.is_some()
    }

    /// Current predicted state (after the newest input), if anchored.
    pub fn ship(&self) -> Option<&ShipState> {
        match self.entries.back().and_then(|e| e.state.as_ref()) {
            Some(s) => Some(s),
            None => self.base.as_ref().map(|(_, s)| s),
        }
    }

    /// Predicted state after tick `tick`, if still in the history.
    pub fn state_at(&self, tick: u64) -> Option<&ShipState> {
        if let Some((t, s)) = &self.base {
            if *t == tick {
                return Some(s);
            }
        }
        self.entry(tick).and_then(|e| e.state.as_ref())
    }

    /// Tick of the newest input.
    pub fn latest_tick(&self) -> Option<u64> {
        self.entries.back().map(|e| e.tick)
    }

    /// Position to render: prediction plus the decaying correction offset.
    pub fn render_pos(&self) -> Option<Vec3> {
        self.ship().map(|s| s.pos + self.pos_offset)
    }

    /// Orientation to render (prediction with the decaying rotation offset).
    pub fn render_rot(&self) -> Option<Quat> {
        self.ship()
            .map(|s| (self.rot_offset * s.orientation).normalize())
    }

    /// Current render offset (u); decays to zero when predictions hold.
    pub fn pos_offset(&self) -> Vec3 {
        self.pos_offset
    }

    fn entry(&self, tick: u64) -> Option<&Entry> {
        let first = self.entries.front()?.tick;
        let i = tick.checked_sub(first)? as usize;
        self.entries.get(i)
    }

    /// Records a (quantized) input and predicts its tick. Inputs must come in
    /// increasing tick order; skipped ticks repeat the previous command, as
    /// the server does.
    pub fn predict<T: TerrainQuery + ?Sized>(&mut self, cmd: InputCmd, terrain: &T) {
        if let Some(last) = self.entries.back() {
            if cmd.tick <= last.tick {
                return;
            }
            let repeat = last.cmd;
            for tick in last.tick + 1..cmd.tick {
                self.push(InputCmd { tick, ..repeat }, terrain);
            }
        }
        self.push(cmd, terrain);
        while self.entries.len() > self.config.history_ticks {
            self.entries.pop_front();
        }
        self.decay();
    }

    fn push<T: TerrainQuery + ?Sized>(&mut self, cmd: InputCmd, terrain: &T) {
        let state = self.ship().cloned().map(|mut s| {
            self.step(&mut s, &cmd, terrain);
            s
        });
        self.entries.push_back(Entry {
            tick: cmd.tick,
            cmd,
            state,
        });
    }

    fn step<T: TerrainQuery + ?Sized>(&self, s: &mut ShipState, cmd: &InputCmd, terrain: &T) {
        step_ship(s, cmd, &self.tuning, self.freeze_after, terrain);
    }

    fn decay(&mut self) {
        let k = self.config.smoothing_keep_per_tick;
        self.pos_offset *= k;
        if self.pos_offset.length_squared() < 1e-8 {
            self.pos_offset = Vec3::ZERO;
        }
        self.rot_offset = Quat::IDENTITY.nlerp(self.rot_offset, k);
        if rotation_error(self.rot_offset, Quat::IDENTITY) < 1e-4 {
            self.rot_offset = Quat::IDENTITY;
        }
    }

    /// Compares the server's state of the local ship at `tick` with the
    /// prediction and corrects it if needed (see the module docs).
    pub fn reconcile<T: TerrainQuery + ?Sized>(
        &mut self,
        tick: u64,
        server: &NetShip,
        conv: &NetConv,
        terrain: &T,
    ) -> Reconcile {
        let player = self
            .ship()
            .map_or(ls_sim::PlayerId(server.id as u16), |s| s.player);
        let old = self.ship().map(|s| (s.pos, s.orientation));

        // Anchor state for `tick`: the prediction with the server's values.
        let predicted = self.state_at(tick).cloned();
        let result = match &predicted {
            Some(p) => {
                let mut srv = p.clone();
                conv.apply_net_ship(&mut srv, server);
                let err = p.pos.distance(srv.pos);
                self.stats.last_error = err;
                if self.matches(p, &srv) {
                    self.stats.accepted += 1;
                    self.prune_before(tick);
                    return Reconcile::Accepted;
                }
                let r = if p.alive != srv.alive || err > self.config.snap_distance {
                    self.snap_to_spawn(&mut srv);
                    Reconcile::Snapped
                } else {
                    self.refine(p, &mut srv);
                    Reconcile::Corrected { error: err }
                };
                self.rebase(tick, srv, terrain);
                r
            }
            None => {
                let oldest = self
                    .base
                    .as_ref()
                    .map(|(t, _)| *t)
                    .or(self.entries.front().map(|e| e.tick));
                if self.base.is_some() && oldest.is_some_and(|o| tick < o) {
                    self.stats.ignored += 1;
                    return Reconcile::Ignored;
                }
                let mut srv = match self.ship() {
                    Some(s) => {
                        let mut s = s.clone();
                        conv.apply_net_ship(&mut s, server);
                        s
                    }
                    None => conv.net_to_ship(player, server),
                };
                self.snap_to_spawn(&mut srv);
                self.stats.last_error = 0.0;
                self.rebase(tick, srv, terrain);
                Reconcile::Snapped
            }
        };

        match result {
            Reconcile::Corrected { .. } => {
                self.stats.corrected += 1;
                let new = self.ship().map(|s| (s.pos, s.orientation));
                if let (Some((op, or)), Some((np, nr))) = (old, new) {
                    self.pos_offset += op - np;
                    self.rot_offset = (self.rot_offset * or * nr.inverse()).normalize();
                    if self.pos_offset.length() > self.config.snap_distance {
                        self.pos_offset = Vec3::ZERO;
                        self.rot_offset = Quat::IDENTITY;
                    }
                }
            }
            _ => {
                self.stats.snapped += 1;
                self.pos_offset = Vec3::ZERO;
                self.rot_offset = Quat::IDENTITY;
            }
        }
        result
    }

    /// Keeps the predicted (full precision) value of every component the
    /// server confirms within tolerance, so a correction of one component
    /// (say velocity after a knockback) does not inject quantization error
    /// into the others.
    fn refine(&self, p: &ShipState, s: &mut ShipState) {
        let c = &self.config;
        if p.pos.distance(s.pos) <= c.pos_tolerance {
            s.pos = p.pos;
        }
        if (p.vel - s.vel).abs().max_element() <= c.vel_tolerance {
            s.vel = p.vel;
        }
        if rotation_error(p.orientation, s.orientation) <= c.rot_tolerance {
            s.orientation = p.orientation;
        }
        if (p.boost - s.boost).abs() <= c.boost_tolerance {
            s.boost = p.boost;
        }
    }

    /// A freshly (re)spawned ship sits exactly on a spawn point, at rest. If
    /// the quantized server state is within tolerance of one, use the exact
    /// spawn pose so prediction starts bit-identical to the server.
    fn snap_to_spawn(&self, s: &mut ShipState) {
        let c = &self.config;
        if !s.alive || s.vel.abs().max_element() > c.vel_tolerance {
            return;
        }
        let exact = self.spawn_poses.iter().find(|(pos, rot)| {
            pos.distance(s.pos) <= c.pos_tolerance
                && rotation_error(*rot, s.orientation) <= c.rot_tolerance
        });
        if let Some(&(pos, rot)) = exact {
            s.pos = pos;
            s.orientation = rot;
            s.vel = Vec3::ZERO;
        }
    }

    /// Exact spawn poses of the match (from [`crate::setup::match_setup`]),
    /// used to anchor prediction bit-exactly after (re)spawns.
    pub fn set_spawn_poses(&mut self, poses: Vec<(Vec3, Quat)>) {
        self.spawn_poses = poses;
    }

    fn matches(&self, p: &ShipState, s: &ShipState) -> bool {
        let c = &self.config;
        let dv = (p.vel - s.vel).abs().max_element();
        p.alive == s.alive
            && p.team == s.team
            && p.pos.distance(s.pos) <= c.pos_tolerance
            && dv <= c.vel_tolerance
            && rotation_error(p.orientation, s.orientation) <= c.rot_tolerance
            && (p.boost - s.boost).abs() <= c.boost_tolerance
    }

    /// Drops history before `tick` (the server will not send older states).
    fn prune_before(&mut self, tick: u64) {
        while self.entries.front().is_some_and(|e| e.tick <= tick) {
            let e = self.entries.pop_front().expect("front exists");
            if let Some(s) = e.state {
                self.base = Some((e.tick, s));
            }
        }
    }

    /// Anchors the state after `tick` to `state` and replays later inputs.
    fn rebase<T: TerrainQuery + ?Sized>(&mut self, tick: u64, state: ShipState, terrain: &T) {
        while self.entries.front().is_some_and(|e| e.tick <= tick) {
            self.entries.pop_front();
        }
        let mut s = state.clone();
        self.base = Some((tick, state));
        let tuning = self.tuning.clone();
        let freeze = self.freeze_after;
        for e in self.entries.iter_mut() {
            step_ship(&mut s, &e.cmd, &tuning, freeze, terrain);
            e.state = Some(s.clone());
        }
    }
}

/// One predicted tick: dead ships do not move (as in `World::step`; the
/// server respawns them), and nothing moves after the match ended.
fn step_ship<T: TerrainQuery + ?Sized>(
    s: &mut ShipState,
    cmd: &InputCmd,
    tuning: &ShipTuning,
    freeze_after: Option<u64>,
    terrain: &T,
) {
    if s.alive && freeze_after.is_none_or(|f| cmd.tick <= f) {
        simulate_movement(s, cmd, tuning, terrain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::terrain::BoxRoomTerrain;
    use ls_math::Aabb;
    use ls_sim::{PlayerId, ShipType, WeaponTable};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn setup() -> (NetConv, BoxRoomTerrain, ShipState) {
        let bounds = Aabb::new(Vec3::ZERO, Vec3::new(256.0, 128.0, 256.0));
        let conv =
            NetConv::for_level(bounds, &WeaponTable::builtin(), ShipTuning::tinker()).unwrap();
        let room = BoxRoomTerrain::new(Aabb::new(Vec3::splat(10.0), Vec3::splat(120.0)));
        let ship = ShipState::new(
            PlayerId(0),
            0,
            ShipType::Tinker,
            conv.tuning(),
            Vec3::splat(60.0),
            Quat::IDENTITY,
        );
        (conv, room, ship)
    }

    fn cmd(tick: u64) -> InputCmd {
        let phase = (tick / 40) % 4;
        let thrust = match phase {
            0 => Vec3::new(0.0, 0.0, -1.0),
            1 => Vec3::new(1.0, 0.3, 0.0),
            2 => Vec3::new(-0.5, 0.0, 1.0),
            _ => Vec3::ZERO,
        };
        crate::convert::quantize_input(&InputCmd {
            tick,
            thrust,
            look_yaw: 0.01,
            look_pitch: if phase == 1 { 0.02 } else { -0.005 },
            ..InputCmd::default()
        })
    }

    /// Server truth with a 6-tick delay; snapshots every 2 ticks.
    #[test]
    fn prediction_matches_delayed_server_without_corrections() {
        let (conv, room, start) = setup();
        let mut server = start.clone();
        let mut truth = vec![server.clone()];
        let mut pred = Predictor::new(conv.tuning().clone(), PredictionConfig::for_conv(&conv));
        let first = conv.quantize_ship(&conv.ship_to_net(0, &server, true));
        assert_eq!(pred.reconcile(0, &first, &conv, &room), Reconcile::Snapped);
        for tick in 1..400u64 {
            let c = cmd(tick);
            simulate_movement(&mut server, &c, conv.tuning(), &room);
            truth.push(server.clone());
            pred.predict(c, &room);
            if tick >= 6 && tick % 2 == 0 {
                let t = tick - 6;
                let net = conv.quantize_ship(&conv.ship_to_net(0, &truth[t as usize], true));
                pred.reconcile(t, &net, &conv, &room);
            }
        }
        let s = pred.stats();
        assert!(s.corrected + s.snapped <= 2, "{s:?}");
        assert!(pred.ship().unwrap().pos.distance(server.pos) < 0.02);
    }

    /// A server-side knockback the client could not predict is corrected
    /// and smoothed; the error disappears afterwards.
    #[test]
    fn misprediction_is_corrected_and_smoothed() {
        let (conv, room, start) = setup();
        let mut server = start.clone();
        let mut truth = vec![server.clone()];
        let mut pred = Predictor::new(conv.tuning().clone(), PredictionConfig::for_conv(&conv));
        pred.reconcile(0, &conv.ship_to_net(0, &server, true), &conv, &room);
        let mut corrections = 0;
        for tick in 1..200u64 {
            let c = cmd(tick);
            simulate_movement(&mut server, &c, conv.tuning(), &room);
            if tick == 50 {
                server.vel += Vec3::new(0.0, 8.0, 0.0); // knockback
            }
            truth.push(server.clone());
            pred.predict(c, &room);
            if tick >= 4 && tick % 2 == 0 {
                let t = tick - 4;
                let net = conv.quantize_ship(&conv.ship_to_net(0, &truth[t as usize], true));
                if let Reconcile::Corrected { .. } = pred.reconcile(t, &net, &conv, &room) {
                    corrections += 1;
                    assert!(pred.pos_offset().length() > 0.0);
                }
            }
        }
        assert!(corrections >= 1);
        assert!(pred.ship().unwrap().pos.distance(server.pos) < 0.02);
        assert!(pred.pos_offset().length() < 1e-3, "{:?}", pred.stats());
    }

    #[test]
    fn inputs_before_the_first_snapshot_are_replayed() {
        let (conv, room, start) = setup();
        let mut pred = Predictor::new(conv.tuning().clone(), PredictionConfig::for_conv(&conv));
        let mut server = start.clone();
        for tick in 1..=10u64 {
            pred.predict(cmd(tick), &room);
            simulate_movement(&mut server, &cmd(tick), conv.tuning(), &room);
            if tick == 4 {
                let net = conv.ship_to_net(0, &server, true);
                assert_eq!(pred.reconcile(4, &net, &conv, &room), Reconcile::Snapped);
            }
        }
        assert!(pred.ship().unwrap().pos.distance(server.pos) < 0.02);
    }

    /// After the match ends the server stops moving ships; a frozen
    /// predictor matches that without corrections, also for ticks it had
    /// already predicted with movement.
    #[test]
    fn frozen_prediction_matches_a_finished_match() {
        let (conv, room, start) = setup();
        let end = 100u64;
        let mut server = start.clone();
        let mut truth = vec![server.clone()];
        let mut pred = Predictor::new(conv.tuning().clone(), PredictionConfig::for_conv(&conv));
        pred.reconcile(0, &conv.ship_to_net(0, &server, true), &conv, &room);
        for tick in 1..300u64 {
            let c = cmd(tick);
            if tick <= end {
                simulate_movement(&mut server, &c, conv.tuning(), &room);
            }
            truth.push(server.clone());
            pred.predict(c, &room);
            if tick == end + 3 {
                // The client learns the end tick a few ticks late.
                pred.freeze_after(end, &room);
            }
            if tick >= 4 && tick % 2 == 0 {
                let t = tick - 4;
                let net = conv.quantize_ship(&conv.ship_to_net(0, &truth[t as usize], true));
                pred.reconcile(t, &net, &conv, &room);
            }
        }
        let s = pred.stats();
        assert!(s.corrected + s.snapped <= 2, "{s:?}");
        assert_eq!(pred.frozen_after(), Some(end));
        assert!(pred.ship().unwrap().pos.distance(server.pos) < 0.02);
    }
}
