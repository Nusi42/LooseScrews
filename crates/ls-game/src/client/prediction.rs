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
use ls_sim::{
    simulate_movement, simulate_movement_env, DynamicWell, InputCmd, MoodView, MoveLimits,
    ShipState, ShipTuning, ShipType, WellField,
};

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

/// The level around the own ship that the simulation's movement reads
/// besides the terrain (M4): the comedy director's view (event movement
/// effects: impulses, forces, plane locks) and the level's up (horizon
/// assist). All of it is replicated (`NetMoodView`), so the predictor
/// moves the ship exactly as `World::step` does.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictEnv {
    /// Match seed (`ls_game::setup::match_seed`): event effects are pure
    /// functions of the view, the seed and the tick.
    pub seed: u64,
    /// The director's view (`None`: no event running, nothing to apply).
    pub mood: Option<MoodView>,
    /// The level's up (`World::up`).
    pub up: Vec3,
    /// The level's static wells (from the match setup) and the dynamic
    /// wells of the newest snapshot (`World::well_field`).
    pub wells: WellField,
}

impl Default for PredictEnv {
    fn default() -> Self {
        PredictEnv {
            seed: 0,
            mood: None,
            up: Vec3::Y,
            wells: WellField::default(),
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
    /// Ship type the local ship flies (its tuning is `tuning`).
    ship_type: ShipType,
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
    /// The mode's movement limits on the local ship (Heist carrier), from
    /// the replicated mode flags.
    limits: MoveLimits,
    /// Mood, up and wells (M4).
    env: PredictEnv,
    /// The dynamic wells changed since the last reconciliation: replay the
    /// history from the anchored state before comparing (the wells carry
    /// their exact start, so the replay matches the server).
    wells_changed: bool,
}

impl Predictor {
    pub fn new(tuning: ShipTuning, config: PredictionConfig) -> Self {
        Predictor {
            config,
            tuning,
            ship_type: ShipType::Tinker,
            entries: VecDeque::new(),
            base: None,
            pos_offset: Vec3::ZERO,
            rot_offset: Quat::IDENTITY,
            stats: PredictionStats::default(),
            spawn_poses: Vec::new(),
            freeze_after: None,
            limits: MoveLimits::NONE,
            env: PredictEnv::default(),
            wells_changed: false,
        }
    }

    /// The match seed event effects are derived from (set once per match).
    pub fn set_seed(&mut self, seed: u64) {
        self.env.seed = seed;
    }

    /// The comedy director's replicated view and the level's up (M4,
    /// protocol 6 `NetMoodView`). Applies to the ticks predicted or replayed
    /// from now on; the next reconciliation replays the ticks after the
    /// snapshot with it (the server's state at the snapshot already has
    /// the effect of the earlier ticks).
    pub fn set_mood(&mut self, mood: &MoodView, up: Vec3) {
        self.env.mood = mood.event.is_some().then_some(*mood);
        self.env.up = if up.is_finite() && up != Vec3::ZERO {
            up
        } else {
            Vec3::Y
        };
    }

    /// The level's well field: static wells from the match setup (set once
    /// per match; replaces the dynamic ones too).
    pub fn set_wells(&mut self, wells: WellField) {
        self.env.wells = wells;
    }

    /// The dynamic wells (rebuilt from the newest snapshot's well
    /// entities, `ls_game::convert::wells_from_entities`). A changed list
    /// makes the next [`Predictor::reconcile`] replay the stored inputs from
    /// the anchored state first: a well the client learns of after it
    /// started (a Paperweight landing) then pulls from its exact start tick,
    /// as on the server, instead of costing a correction.
    pub fn set_dynamic_wells(&mut self, wells: Vec<DynamicWell>) {
        if wells != self.env.wells.dynamics {
            self.env.wells.dynamics = wells;
            self.wells_changed = true;
        }
    }

    /// Mood, up and wells the prediction applies.
    pub fn env(&self) -> &PredictEnv {
        &self.env
    }

    /// The mode's movement limits for the local ship (`ModeModule::move_limits`
    /// of its replicated flags). Applies to the ticks predicted or replayed
    /// from now on.
    pub fn set_limits(&mut self, limits: MoveLimits) {
        self.limits = limits;
    }

    pub fn limits(&self) -> MoveLimits {
        self.limits
    }

    /// Switches the ship type the prediction simulates (ship choice at a
    /// respawn). Takes effect from the next reconciliation on.
    pub fn set_ship_type(&mut self, ship_type: ShipType, tuning: ShipTuning) {
        self.ship_type = ship_type;
        self.tuning = tuning;
    }

    pub fn ship_type(&self) -> ShipType {
        self.ship_type
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
        step_ship(
            s,
            cmd,
            &self.tuning,
            self.limits,
            self.freeze_after,
            &self.env,
            terrain,
        );
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
        if std::mem::take(&mut self.wells_changed) {
            if let Some((t, s)) = self.base.clone() {
                self.rebase(t, s, terrain);
            }
        }
        let player = self
            .ship()
            .map_or(ls_sim::PlayerId(server.id as u16), |s| s.player);
        let old = self.ship().map(|s| (s.pos, s.orientation));

        // Anchor state for `tick`: the prediction with the server's values.
        let predicted = self.state_at(tick).cloned();
        let result = match &predicted {
            Some(p) => {
                let mut srv = p.clone();
                srv.ship_type = self.ship_type;
                conv.apply_net_ship(&mut srv, server);
                let err = p.pos.distance(srv.pos);
                self.stats.last_error = err;
                if p.ship_type == srv.ship_type && self.matches(p, &srv) {
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
                        s.ship_type = self.ship_type;
                        conv.apply_net_ship(&mut s, server);
                        s
                    }
                    None => conv.net_to_ship_as(player, self.ship_type, server),
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
        let limits = self.limits;
        let env = &self.env;
        for e in self.entries.iter_mut() {
            step_ship(&mut s, &e.cmd, &tuning, limits, freeze, env, terrain);
            e.state = Some(s.clone());
        }
    }
}

/// One predicted tick: dead ships do not move (as in `World::step`; the
/// server respawns them), nothing moves after the match ended, and the
/// mode's limits, the running event's movement effects, the wells and the
/// level's up apply as on the server (the input for tick `k` is the
/// server step that produces tick `k`).
fn step_ship<T: TerrainQuery + ?Sized>(
    s: &mut ShipState,
    cmd: &InputCmd,
    tuning: &ShipTuning,
    limits: MoveLimits,
    freeze_after: Option<u64>,
    env: &PredictEnv,
    terrain: &T,
) {
    if !(s.alive && freeze_after.is_none_or(|f| cmd.tick <= f)) {
        return;
    }
    let now = cmd.tick;
    let fx = env.mood.as_ref().map_or(ls_sim::MoveEffects::NONE, |m| {
        ls_sim::director::move_effects(m, env.seed, now, s)
    });
    let pos_before = s.pos;
    if !fx.is_none() {
        ls_sim::director::apply_move_effects_before(s, &fx);
    }
    // Wells and the level's up exactly as `World::step` (a level without
    // wells and with +Y up runs the plain flight model).
    let plain = env.wells.is_empty() && env.up == Vec3::Y;
    let (t, c);
    let (tuning, cmd) = if limits.is_none() {
        (tuning, cmd)
    } else {
        (t, c) = limits.apply(tuning, cmd);
        (&t, &c)
    };
    if plain {
        simulate_movement(s, cmd, tuning, terrain);
    } else {
        let me = env.wells.move_env(s.pos, now, tuning, env.up);
        simulate_movement_env(s, cmd, tuning, terrain, &me);
    }
    if !fx.is_none() {
        ls_sim::director::apply_move_effects_after(s, &fx, pos_before);
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
        let conv = NetConv::for_level(
            bounds,
            &WeaponTable::builtin(),
            ls_sim::ShipTable::builtin(),
        )
        .unwrap();
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

    /// M4: a static well, a dynamic well that starts and ends mid-flight
    /// (the client learns of it from the first snapshot that shows it) and
    /// a turned level up (horizon assist) are predicted as the server moves
    /// the ship: reconciliation errors stay under 0.05 u. Without the field
    /// the prediction keeps correcting.
    #[test]
    fn wells_and_up_are_predicted() {
        use ls_sim::{simulate_movement_env, DynamicWell, GravitySettings, MoodView, WellField};
        let assist = |tick| {
            crate::convert::quantize_input(&InputCmd {
                buttons: ls_sim::Buttons::HORIZON_ASSIST,
                ..cmd(tick)
            })
        };
        for informed in [true, false] {
            let (conv, room, start) = setup();
            let settings = GravitySettings::builtin();
            let site = ls_math::level::WellSite {
                pos: start.pos + Vec3::new(12.0, 0.0, -10.0),
                core_radius: 2.0,
                hollow_radius: 8.0,
                strength: 1.0,
                range: 40.0,
                kind: Default::default(),
            };
            let mut field = WellField::new(&[site], 100, settings);
            assert!(!field.statics.is_empty());
            let mut dynamic = DynamicWell {
                key: 1,
                pos: start.pos + Vec3::new(-6.0, 4.0, -20.0),
                peak_accel: 9.0,
                range: 16.0,
                start_tick: 120,
                end_tick: 260,
                owner: None,
            };
            let up = Vec3::new(0.6, 0.8, 0.0);
            let mut server = start.clone();
            let mut truth = vec![server.clone()];
            let mut pred = Predictor::new(conv.tuning().clone(), PredictionConfig::for_conv(&conv));
            if informed {
                pred.set_wells(WellField::new(&[site], 100, settings));
                pred.set_mood(&MoodView::default(), up);
            }
            pred.reconcile(0, &conv.ship_to_net(0, &server, true), &conv, &room);
            let mut worst = 0.0f32;
            let mut dyn_pull = 0.0f32;
            for tick in 1..400u64 {
                if tick == 120 {
                    // Dropped just ahead of the ship (a Paperweight).
                    dynamic.pos = server.pos + Vec3::new(3.0, 1.0, -4.0);
                    field.dynamics.push(dynamic);
                }
                if dynamic.active(tick) {
                    let only = WellField {
                        statics: Vec::new(),
                        dynamics: vec![dynamic],
                        settings: Some(*settings),
                    };
                    dyn_pull = dyn_pull.max(only.accel_at(server.pos, tick).length());
                }
                let c = assist(tick);
                let env = field.move_env(server.pos, tick, conv.tuning(), up);
                simulate_movement_env(&mut server, &c, conv.tuning(), &room, &env);
                truth.push(server.clone());
                pred.predict(c, &room);
                if tick >= 6 && tick % 2 == 0 {
                    let t = tick - 6;
                    // The snapshot of tick `t` shows the dynamic well while
                    // it exists in the world (from its start to its end).
                    if informed && (120..260).contains(&t) {
                        pred.set_dynamic_wells(vec![dynamic]);
                    } else if informed {
                        pred.set_dynamic_wells(Vec::new());
                    }
                    let net = conv.quantize_ship(&conv.ship_to_net(0, &truth[t as usize], true));
                    pred.reconcile(t, &net, &conv, &room);
                    worst = worst.max(pred.stats().last_error);
                }
            }
            let s = pred.stats();
            eprintln!(
                "wells and up, informed {informed}: worst {worst:.5} u, dynamic pull up to {dyn_pull:.2} u/s^2, {s:?}"
            );
            assert!(
                dyn_pull > 2.0,
                "the dynamic well pulls the ship: {dyn_pull}"
            );
            if informed {
                assert!(worst < 0.05, "worst {worst}: {s:?}");
                assert!(s.corrected <= 4, "{s:?}");
                assert!(pred.ship().unwrap().pos.distance(server.pos) < 0.05);
            } else {
                assert!(s.corrected > 10, "wells unknown mispredict: {s:?}");
            }
        }
    }

    /// A Heist carrier (slower, no boost) predicts without corrections when
    /// the predictor has the mode's limits for its replicated flags; without
    /// them, boosting mispredicts.
    #[test]
    fn carrier_limits_are_predicted() {
        use ls_sim::mode::{player_flags, ModeKind};
        let rules = ls_sim::RuleSet::default_for(ModeKind::Heist).mode;
        let limits =
            ls_sim::modes::module(ModeKind::Heist).move_limits(&rules, player_flags::CARRIER);
        assert!(!limits.is_none());
        let boost = |tick| {
            crate::convert::quantize_input(&InputCmd {
                buttons: ls_sim::Buttons::BOOST,
                ..cmd(tick)
            })
        };
        for (with_limits, max_fixes) in [(true, 2u64), (false, u64::MAX)] {
            let (conv, room, start) = setup();
            let mut server = start.clone();
            server.boost = 100.0;
            let mut truth = vec![server.clone()];
            let mut pred = Predictor::new(conv.tuning().clone(), PredictionConfig::for_conv(&conv));
            if with_limits {
                pred.set_limits(limits);
            }
            pred.reconcile(0, &conv.ship_to_net(0, &server, true), &conv, &room);
            for tick in 1..300u64 {
                let c = boost(tick);
                // What `World::step` does for a carrier.
                let (t, lc) = limits.apply(conv.tuning(), &c);
                simulate_movement(&mut server, &lc, &t, &room);
                truth.push(server.clone());
                pred.predict(c, &room);
                if tick >= 6 && tick % 2 == 0 {
                    let t = tick - 6;
                    let net = conv.quantize_ship(&conv.ship_to_net(0, &truth[t as usize], true));
                    pred.reconcile(t, &net, &conv, &room);
                }
            }
            let s = pred.stats();
            let fixes = s.corrected + s.snapped;
            if with_limits {
                assert!(fixes <= max_fixes, "{s:?}");
                assert!(pred.ship().unwrap().pos.distance(server.pos) < 0.02);
            } else {
                assert!(fixes > 10, "boost without limits mispredicts: {s:?}");
            }
        }
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
