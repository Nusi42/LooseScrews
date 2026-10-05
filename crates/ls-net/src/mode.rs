//! Protocol 5 mode and lobby items: the generic [`NetModeState`] every mode
//! replicates in snapshots (the wire form of `ls_sim::ModeView`), the lobby
//! state ([`LobbyStateMsg`]) and the match results ([`MatchResultsMsg`]).
//!
//! # Mode state in snapshots
//!
//! The last section of every snapshot (`docs/milestone-3-plan.md` §3.1,
//! "Generic mode state on the wire"):
//!
//! ```text
//! full    1 bit "present" (0: all defaults, e.g. practice), then
//!         header: u8 kind, u8 teams, u8 phase, u8 winner, varint end_tick,
//!           u8 stage, varint stage_end_tick, varint score_total, varint
//!           score_target, u16 flags
//!         team scores: 3 bits count (0..=4) + zigzag varint each
//!         players: varint count, per player (id order): u8 id, u8 side,
//!           zigzag varint score, u8 lives, u8 meter, u8 flags
//!         objectives: 4 bits count (0..=8), per objective (id order): u8 id,
//!           u8 kind, u8 team, u8 holder, u8 state, varint timer_end, f32
//!           radius (raw bits), 3 x u16 position
//!         mood (protocol 6): 1 bit "present" (not the default) [mood]
//!         race (protocol 6, B6): 1 bit "present" [race]
//! delta   1 bit "changed"; if changed:
//!         1 bit header changed [header]
//!         1 bit team scores changed [team scores]
//!         1 bit players changed [u32 presence mask, per present player in id
//!           order: in the baseline 1 bit "changed" + the fields after the id
//!           if changed; new: the fields after the id]
//!         1 bit objectives changed [4 bits count, per objective: u8 id; in the
//!           baseline 1 bit meta changed [the fields between id and position]
//!           + 1 bit moved [3 component diffs, 2-bit size class each]; new:
//!           the full objective]
//!         1 bit mood changed [mood]  (protocol 6)
//!         1 bit race changed [race]  (protocol 6, B6)
//! ```
//!
//! # Mood (protocol 6)
//!
//! The comedy director's replicated state (`ls_sim::MoodView`,
//! `docs/milestone-4-plan.md` §3.1) and the level's `up` (Bored level),
//! sent in full whenever any of it changes:
//!
//! ```text
//! mood    u8 mood (calm, bored, grumpy, giddy), u8 event (EventKind index,
//!         255 = none), u8 phase (mood_phase), u8 target (wire ship id, 255 =
//!         none), varint phase_start_tick, varint phase_end_tick, 8 parameter
//!         bytes, then `up`: 1 bit "default" (+Y) or 3 x f32 raw bits
//! ```
//!
//! # Race (protocol 6; day-one layout note for B6, Belt Up)
//!
//! `RaceView` goes into its own section after the mood; until B6 adds it
//! the encoder writes 0 and the decoder rejects 1. B6 fills it in this
//! layout (all of it sent again when anything in it changes; racers are
//! few and it changes at gates):
//!
//! ```text
//! race    u8 heat (0-based), u8 heats, u8 heat_phase (start lights,
//!         racing, results), varint heat_phase_end_tick, then u32 presence
//!         mask of racers (wire ship ids), per racer in id order: u8
//!         next_gate (gates passed, 8 = lap done), varint finish_tick (0 =
//!         not finished), u8 place (0 = none), varint hold_end_tick (0 = not
//!         held), u8 slot
//! ```
//!
//! # Well entities (protocol 6; day-one layout note for B7 and A4)
//!
//! A dynamic gravity well (a Paperweight stone, the Architect's Gravity
//! Chaos+ stroke) is a generic entity of kind `ReplicatedKind::Well` at the
//! well's centre whose 4 state bytes are [`crate::wire::well_state`]
//! (range, strength, end tick). The client predictor rebuilds the well from
//! them; the entity exists while the well pulls (strength 0 while a stone
//! is still flying).
//!
//! Unchanged mode state costs one bit per delta snapshot. Positions are
//! quantized like ship positions (level bounds + 2 u, 16 bits per axis).
//! Player and objective holder ids are wire ship ids.

use ls_math::Vec3;

use crate::bits::{BitReader, BitWriter};
use crate::error::{DecodeError, EncodeError};
use crate::quant::{PackedVec3, Quantizer};
use crate::wire::{MAX_PLAYER_NAME, MAX_SHIPS};

/// Most team scores in a mode state (teams 0..4).
pub const MAX_MODE_TEAMS: usize = 4;
/// Most objective markers in a mode state.
pub const MAX_OBJECTIVES: usize = 8;
/// Value for "none" in `winner`, `holder` and objective `team` fields.
pub const NONE: u8 = 255;
/// [`NetModePlayer::lives`] for modes without a lives limit.
pub const UNLIMITED_LIVES: u8 = 255;

/// Bits of [`NetModePlayer::flags`] (identical to `ls_sim::mode::player_flags`).
pub mod mode_player_flags {
    /// Out of the match (no lives or colour left); spectates.
    pub const ELIMINATED: u8 = 1 << 0;
    /// Spectating (joined an elimination mode late, or by choice).
    pub const SPECTATOR: u8 = 1 << 1;
    /// Carries an objective (Heist core, Hot Reactor).
    pub const CARRIER: u8 = 1 << 2;
    /// The Architect (protocol 6): this player is the round's architect (no
    /// ship; `meter` is the stroke budget, clamped to 255).
    pub const ARCHITECT: u8 = 1 << 3;
}

/// [`NetMoodView::phase`] values (`ls_sim` event phases).
pub mod mood_phase {
    /// No event running.
    pub const IDLE: u8 = 0;
    /// The 3 s telegraph before the event changes anything.
    pub const ANNOUNCE: u8 = 1;
    pub const ACTIVE: u8 = 2;
    /// The event winds down (wells fade back in, ducks leave, ...).
    pub const ENDING: u8 = 3;
}

/// The comedy director's replicated state (protocol 6): the wire form of
/// `ls_sim::MoodView` plus the level's `up` (`World::up`, Bored level).
/// Everything an event does to the own ship's movement is a pure function
/// of this view and the match seed, so the client predictor reproduces it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetMoodView {
    /// Mood index (`ls_sim` moods: 0 calm, 1 bored, 2 grumpy, 3 giddy).
    pub mood: u8,
    /// Running event (`ls_sim::EventKind` index), [`NONE`] for none.
    pub event: u8,
    /// [`mood_phase`].
    pub phase: u8,
    /// Wire ship id of the player the event aims at, [`NONE`] for none.
    pub target: u8,
    /// Tick the current phase started and ends (0 = none).
    pub phase_start_tick: u32,
    pub phase_end_tick: u32,
    /// Event-defined parameters (seeded axis, route index, ...).
    pub params: [u8; 8],
    /// The level's up direction (unit vector; +Y unless Bored level turned
    /// it). Sent as raw `f32` bits, so both ends use the same value.
    pub up: Vec3,
}

impl Default for NetMoodView {
    fn default() -> Self {
        NetMoodView {
            mood: 0,
            event: NONE,
            phase: mood_phase::IDLE,
            target: NONE,
            phase_start_tick: 0,
            phase_end_tick: 0,
            params: [0; 8],
            up: Vec3::Y,
        }
    }
}

/// [`NetMoodView`] as stored in packed snapshots (`up` as raw bits, so
/// `Eq` compares bit patterns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedMoodView {
    pub mood: u8,
    pub event: u8,
    pub phase: u8,
    pub target: u8,
    pub phase_start_tick: u32,
    pub phase_end_tick: u32,
    pub params: [u8; 8],
    pub up: [u32; 3],
}

impl Default for PackedMoodView {
    fn default() -> Self {
        PackedMoodView::from(&NetMoodView::default())
    }
}

impl From<&NetMoodView> for PackedMoodView {
    fn from(m: &NetMoodView) -> Self {
        PackedMoodView {
            mood: m.mood,
            event: m.event,
            phase: m.phase,
            target: m.target,
            phase_start_tick: m.phase_start_tick,
            phase_end_tick: m.phase_end_tick,
            params: m.params,
            up: [m.up.x.to_bits(), m.up.y.to_bits(), m.up.z.to_bits()],
        }
    }
}

impl From<&PackedMoodView> for NetMoodView {
    fn from(m: &PackedMoodView) -> Self {
        NetMoodView {
            mood: m.mood,
            event: m.event,
            phase: m.phase,
            target: m.target,
            phase_start_tick: m.phase_start_tick,
            phase_end_tick: m.phase_end_tick,
            params: m.params,
            up: Vec3::new(
                f32::from_bits(m.up[0]),
                f32::from_bits(m.up[1]),
                f32::from_bits(m.up[2]),
            ),
        }
    }
}

/// [`NetModeState::phase`] values.
pub mod mode_phase {
    pub const RUNNING: u8 = 0;
    /// The match is over; `NetModeState::winner` is the winning side.
    pub const ENDED: u8 = 1;
}

/// One player's mode state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetModePlayer {
    /// Wire ship id.
    pub id: u8,
    /// Team index, or the player's own side id in free-for-all.
    pub side: u8,
    /// Mode score (kills, deliveries, ...).
    pub score: i32,
    /// Lives left, [`UNLIMITED_LIVES`] when the mode has no limit.
    pub lives: u8,
    /// Mode meter (Last Color Standing colour).
    pub meter: u8,
    /// [`mode_player_flags`].
    pub flags: u8,
}

/// An objective marker (`ls_sim::mode::objective_kind`: safe zone, core,
/// extraction zone, reactor, ...).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NetObjective {
    pub id: u8,
    pub kind: u8,
    /// Owning team, [`NONE`] for none.
    pub team: u8,
    /// Wire id of the ship holding it, [`NONE`] for none.
    pub holder: u8,
    /// Module-defined state byte.
    pub state: u8,
    /// Server tick of the objective's timer (detonation, return; 0 = none).
    pub timer_end: u32,
    pub radius: f32,
    pub pos: Vec3,
}

/// [`NetObjective`] with a quantized position (radius as raw `f32` bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PackedObjective {
    pub id: u8,
    pub kind: u8,
    pub team: u8,
    pub holder: u8,
    pub state: u8,
    pub timer_end: u32,
    pub radius: u32,
    pub pos: PackedVec3,
}

/// Generic mode state of every mode (protocol 5): the wire form of
/// `ls_sim::ModeView` (`ls_game::convert::mode_view_to_net`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetModeState {
    /// `ls_sim::ModeKind::index`.
    pub kind: u8,
    /// Number of teams (0 = free-for-all).
    pub teams: u8,
    /// [`mode_phase`].
    pub phase: u8,
    /// Winning side once ended, [`NONE`] while running or on a draw.
    pub winner: u8,
    /// Tick the match timer runs out.
    pub end_tick: u32,
    /// Mode-specific stage (Squeeze phase, ...) and the tick it ends (0 = none).
    pub stage: u8,
    pub stage_end_tick: u32,
    /// Paint the Void: paintable surface cells (0 otherwise).
    pub score_total: u64,
    /// Score that wins (0 = none).
    pub score_target: u32,
    /// `ls_sim::mode::view_flags`.
    pub flags: u16,
    /// One score per team (empty in free-for-all).
    pub team_scores: Vec<i64>,
    /// Per-player state, sorted by id (the top 32 scores).
    pub players: Vec<NetModePlayer>,
    /// Objective markers, sorted by id.
    pub objectives: Vec<NetObjective>,
    /// The comedy director's state and the level's up (protocol 6).
    pub mood: NetMoodView,
}

impl NetModeState {
    /// The entry of ship `id`, if any.
    pub fn player(&self, id: u8) -> Option<&NetModePlayer> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn is_running(&self) -> bool {
        self.phase == mode_phase::RUNNING
    }
}

/// [`NetModeState`] with quantized objective positions (what snapshots
/// store and delta-code).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackedModeState {
    pub kind: u8,
    pub teams: u8,
    pub phase: u8,
    pub winner: u8,
    pub end_tick: u32,
    pub stage: u8,
    pub stage_end_tick: u32,
    pub score_total: u64,
    pub score_target: u32,
    pub flags: u16,
    pub team_scores: Vec<i64>,
    pub players: Vec<NetModePlayer>,
    pub objectives: Vec<PackedObjective>,
    pub mood: PackedMoodView,
}

impl PackedModeState {
    fn header(&self) -> (u8, u8, u8, u8, u32, u8, u32, u64, u32, u16) {
        (
            self.kind,
            self.teams,
            self.phase,
            self.winner,
            self.end_tick,
            self.stage,
            self.stage_end_tick,
            self.score_total,
            self.score_target,
            self.flags,
        )
    }
}

impl Quantizer {
    /// Quantizes a mode state (players and objectives sorted by id).
    pub fn pack_mode(&self, m: &NetModeState) -> PackedModeState {
        let mut players = m.players.clone();
        players.sort_by_key(|p| p.id);
        let mut objectives: Vec<PackedObjective> = m
            .objectives
            .iter()
            .map(|o| PackedObjective {
                id: o.id,
                kind: o.kind,
                team: o.team,
                holder: o.holder,
                state: o.state,
                timer_end: o.timer_end,
                radius: o.radius.to_bits(),
                pos: self.pack_position(o.pos),
            })
            .collect();
        objectives.sort_by_key(|o| o.id);
        PackedModeState {
            kind: m.kind,
            teams: m.teams,
            phase: m.phase,
            winner: m.winner,
            end_tick: m.end_tick,
            stage: m.stage,
            stage_end_tick: m.stage_end_tick,
            score_total: m.score_total,
            score_target: m.score_target,
            flags: m.flags,
            team_scores: m.team_scores.clone(),
            players,
            objectives,
            mood: PackedMoodView::from(&m.mood),
        }
    }

    pub fn unpack_mode(&self, m: &PackedModeState) -> NetModeState {
        NetModeState {
            kind: m.kind,
            teams: m.teams,
            phase: m.phase,
            winner: m.winner,
            end_tick: m.end_tick,
            stage: m.stage,
            stage_end_tick: m.stage_end_tick,
            score_total: m.score_total,
            score_target: m.score_target,
            flags: m.flags,
            team_scores: m.team_scores.clone(),
            players: m.players.clone(),
            objectives: m
                .objectives
                .iter()
                .map(|o| NetObjective {
                    id: o.id,
                    kind: o.kind,
                    team: o.team,
                    holder: o.holder,
                    state: o.state,
                    timer_end: o.timer_end,
                    radius: f32::from_bits(o.radius),
                    pos: self.unpack_position(o.pos),
                })
                .collect(),
            mood: NetMoodView::from(&m.mood),
        }
    }
}

#[inline]
fn zz32(v: i32) -> u64 {
    (((v << 1) ^ (v >> 31)) as u32) as u64
}

#[inline]
fn unzz32(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

#[inline]
fn zz64(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

#[inline]
fn unzz64(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

pub(crate) fn check_mode(m: &PackedModeState) -> Result<(), EncodeError> {
    if m.team_scores.len() > MAX_MODE_TEAMS {
        return Err(EncodeError::OutOfRange("too many team scores"));
    }
    if m.objectives.len() > MAX_OBJECTIVES {
        return Err(EncodeError::OutOfRange("too many objectives"));
    }
    let mut prev: Option<u8> = None;
    for p in &m.players {
        if p.id >= MAX_SHIPS || prev.is_some_and(|q| q >= p.id) {
            return Err(EncodeError::OutOfRange(
                "mode players must be sorted by id and unique",
            ));
        }
        prev = Some(p.id);
    }
    let mut prev: Option<u8> = None;
    for o in &m.objectives {
        if prev.is_some_and(|q| q >= o.id) {
            return Err(EncodeError::OutOfRange(
                "objectives must be sorted by id and unique",
            ));
        }
        if !f32::from_bits(o.radius).is_finite() {
            return Err(EncodeError::OutOfRange("objective radius not finite"));
        }
        prev = Some(o.id);
    }
    if !m.mood.up.iter().all(|&b| f32::from_bits(b).is_finite()) {
        return Err(EncodeError::OutOfRange("mood up not finite"));
    }
    Ok(())
}

fn write_mood(w: &mut BitWriter, m: &PackedMoodView) {
    w.write_u8(m.mood);
    w.write_u8(m.event);
    w.write_u8(m.phase);
    w.write_u8(m.target);
    w.write_varint(m.phase_start_tick as u64);
    w.write_varint(m.phase_end_tick as u64);
    for b in m.params {
        w.write_u8(b);
    }
    let default_up = m.up == PackedMoodView::default().up;
    w.write_bool(default_up);
    if !default_up {
        for b in m.up {
            w.write_u32(b);
        }
    }
}

fn read_mood(r: &mut BitReader) -> Result<PackedMoodView, DecodeError> {
    let mut m = PackedMoodView {
        mood: r.read_u8()?,
        event: r.read_u8()?,
        phase: r.read_u8()?,
        target: r.read_u8()?,
        phase_start_tick: r.read_varint_u32()?,
        phase_end_tick: r.read_varint_u32()?,
        ..PackedMoodView::default()
    };
    for b in &mut m.params {
        *b = r.read_u8()?;
    }
    if !r.read_bool()? {
        for b in &mut m.up {
            *b = r.read_finite_f32()?.to_bits();
        }
    }
    Ok(m)
}

/// The race section (B6, Belt Up): not filled before B6, see the module
/// docs for its day-one layout.
fn write_race_absent(w: &mut BitWriter) {
    w.write_bool(false);
}

fn read_race_absent(r: &mut BitReader) -> Result<(), DecodeError> {
    if r.read_bool()? {
        return Err(DecodeError::Invalid("race section not supported"));
    }
    Ok(())
}

fn write_header(w: &mut BitWriter, m: &PackedModeState) {
    w.write_u8(m.kind);
    w.write_u8(m.teams);
    w.write_u8(m.phase);
    w.write_u8(m.winner);
    w.write_varint(m.end_tick as u64);
    w.write_u8(m.stage);
    w.write_varint(m.stage_end_tick as u64);
    w.write_varint(m.score_total);
    w.write_varint(m.score_target as u64);
    w.write_u16(m.flags);
}

fn read_header(r: &mut BitReader, m: &mut PackedModeState) -> Result<(), DecodeError> {
    m.kind = r.read_u8()?;
    m.teams = r.read_u8()?;
    m.phase = r.read_u8()?;
    m.winner = r.read_u8()?;
    m.end_tick = r.read_varint_u32()?;
    m.stage = r.read_u8()?;
    m.stage_end_tick = r.read_varint_u32()?;
    m.score_total = r.read_varint()?;
    m.score_target = r.read_varint_u32()?;
    m.flags = r.read_u16()?;
    Ok(())
}

fn write_team_scores(w: &mut BitWriter, s: &[i64]) {
    w.write_bits(s.len() as u64, 3);
    for &v in s {
        w.write_varint(zz64(v));
    }
}

fn read_team_scores(r: &mut BitReader) -> Result<Vec<i64>, DecodeError> {
    let n = r.read_bits(3)? as usize;
    if n > MAX_MODE_TEAMS {
        return Err(DecodeError::Invalid("too many team scores"));
    }
    (0..n).map(|_| Ok(unzz64(r.read_varint()?))).collect()
}

fn write_player_fields(w: &mut BitWriter, p: &NetModePlayer) {
    w.write_u8(p.side);
    w.write_varint(zz32(p.score));
    w.write_u8(p.lives);
    w.write_u8(p.meter);
    w.write_u8(p.flags);
}

fn read_player_fields(r: &mut BitReader, id: u8) -> Result<NetModePlayer, DecodeError> {
    Ok(NetModePlayer {
        id,
        side: r.read_u8()?,
        score: unzz32(r.read_varint_u32()?),
        lives: r.read_u8()?,
        meter: r.read_u8()?,
        flags: r.read_u8()?,
    })
}

fn objective_meta(o: &PackedObjective) -> (u8, u8, u8, u8, u32, u32) {
    (o.kind, o.team, o.holder, o.state, o.timer_end, o.radius)
}

fn write_objective_meta(w: &mut BitWriter, o: &PackedObjective) {
    w.write_u8(o.kind);
    w.write_u8(o.team);
    w.write_u8(o.holder);
    w.write_u8(o.state);
    w.write_varint(o.timer_end as u64);
    w.write_u32(o.radius);
}

fn read_objective_meta(r: &mut BitReader, id: u8) -> Result<PackedObjective, DecodeError> {
    Ok(PackedObjective {
        id,
        kind: r.read_u8()?,
        team: r.read_u8()?,
        holder: r.read_u8()?,
        state: r.read_u8()?,
        timer_end: r.read_varint_u32()?,
        radius: r.read_finite_f32()?.to_bits(),
        pos: [0; 3],
    })
}

fn write_objective_full(w: &mut BitWriter, o: &PackedObjective) {
    write_objective_meta(w, o);
    for c in o.pos {
        w.write_u16(c);
    }
}

fn read_objective_full(r: &mut BitReader, id: u8) -> Result<PackedObjective, DecodeError> {
    let mut o = read_objective_meta(r, id)?;
    o.pos = [r.read_u16()?, r.read_u16()?, r.read_u16()?];
    Ok(o)
}

/// Position component diff: 2-bit class (0 same, 1 = 4-bit zigzag, 2 =
/// 8-bit zigzag, 3 = raw 16 bits).
fn write_comp(w: &mut BitWriter, cur: u16, base: u16) {
    let d = cur.wrapping_sub(base) as i16;
    let z = ((d << 1) ^ (d >> 15)) as u16;
    if z == 0 {
        w.write_bits(0, 2);
    } else if z < 16 {
        w.write_bits(1, 2);
        w.write_bits(z as u64, 4);
    } else if z < 256 {
        w.write_bits(2, 2);
        w.write_bits(z as u64, 8);
    } else {
        w.write_bits(3, 2);
        w.write_u16(cur);
    }
}

fn read_comp(r: &mut BitReader, base: u16) -> Result<u16, DecodeError> {
    let un = |z: u16| ((z >> 1) as i16 ^ -((z & 1) as i16)) as u16;
    Ok(match r.read_bits(2)? {
        0 => base,
        1 => base.wrapping_add(un(r.read_bits(4)? as u16)),
        2 => base.wrapping_add(un(r.read_bits(8)? as u16)),
        _ => r.read_u16()?,
    })
}

/// Writes the full mode section.
pub(crate) fn write_mode_full(w: &mut BitWriter, m: &PackedModeState) {
    // A mode-less snapshot (all defaults) costs one bit.
    let present = *m != PackedModeState::default();
    w.write_bool(present);
    if !present {
        return;
    }
    write_header(w, m);
    write_team_scores(w, &m.team_scores);
    w.write_varint(m.players.len() as u64);
    for p in &m.players {
        w.write_u8(p.id);
        write_player_fields(w, p);
    }
    w.write_bits(m.objectives.len() as u64, 4);
    for o in &m.objectives {
        w.write_u8(o.id);
        write_objective_full(w, o);
    }
    let mood = m.mood != PackedMoodView::default();
    w.write_bool(mood);
    if mood {
        write_mood(w, &m.mood);
    }
    write_race_absent(w);
}

pub(crate) fn read_mode_full(r: &mut BitReader) -> Result<PackedModeState, DecodeError> {
    let mut m = PackedModeState::default();
    if !r.read_bool()? {
        return Ok(m);
    }
    read_header(r, &mut m)?;
    m.team_scores = read_team_scores(r)?;
    let n = r.read_len(MAX_SHIPS as usize, 48)?;
    let mut prev: Option<u8> = None;
    for _ in 0..n {
        let id = r.read_u8()?;
        if id >= MAX_SHIPS || prev.is_some_and(|q| q >= id) {
            return Err(DecodeError::Invalid("mode player ids not ascending"));
        }
        prev = Some(id);
        m.players.push(read_player_fields(r, id)?);
    }
    let n = r.read_bits(4)? as usize;
    if n > MAX_OBJECTIVES {
        return Err(DecodeError::Invalid("too many objectives"));
    }
    for _ in 0..n {
        let id = r.read_u8()?;
        if m.objectives.last().is_some_and(|o| o.id >= id) {
            return Err(DecodeError::Invalid("objective ids not ascending"));
        }
        m.objectives.push(read_objective_full(r, id)?);
    }
    if r.read_bool()? {
        m.mood = read_mood(r)?;
    }
    read_race_absent(r)?;
    Ok(m)
}

/// Writes the mode section against the baseline's.
pub(crate) fn write_mode_delta(w: &mut BitWriter, m: &PackedModeState, b: &PackedModeState) {
    let changed = m != b;
    w.write_bool(changed);
    if !changed {
        return;
    }
    let header = m.header() != b.header();
    w.write_bool(header);
    if header {
        write_header(w, m);
    }
    let scores = m.team_scores != b.team_scores;
    w.write_bool(scores);
    if scores {
        write_team_scores(w, &m.team_scores);
    }
    let players = m.players != b.players;
    w.write_bool(players);
    if players {
        let mask = m.players.iter().fold(0u32, |acc, p| acc | (1 << p.id));
        w.write_u32(mask);
        for p in &m.players {
            match b.players.iter().find(|x| x.id == p.id) {
                Some(bp) => {
                    w.write_bool(bp != p);
                    if bp != p {
                        write_player_fields(w, p);
                    }
                }
                None => write_player_fields(w, p),
            }
        }
    }
    let objectives = m.objectives != b.objectives;
    w.write_bool(objectives);
    if objectives {
        w.write_bits(m.objectives.len() as u64, 4);
        for o in &m.objectives {
            w.write_u8(o.id);
            match b.objectives.iter().find(|x| x.id == o.id) {
                Some(bo) => {
                    let meta = objective_meta(o) != objective_meta(bo);
                    w.write_bool(meta);
                    if meta {
                        write_objective_meta(w, o);
                    }
                    let moved = o.pos != bo.pos;
                    w.write_bool(moved);
                    if moved {
                        for i in 0..3 {
                            write_comp(w, o.pos[i], bo.pos[i]);
                        }
                    }
                }
                None => write_objective_full(w, o),
            }
        }
    }
    let mood = m.mood != b.mood;
    w.write_bool(mood);
    if mood {
        write_mood(w, &m.mood);
    }
    write_race_absent(w);
}

pub(crate) fn read_mode_delta(
    r: &mut BitReader,
    b: &PackedModeState,
) -> Result<PackedModeState, DecodeError> {
    let mut m = b.clone();
    if !r.read_bool()? {
        return Ok(m);
    }
    if r.read_bool()? {
        read_header(r, &mut m)?;
    }
    if r.read_bool()? {
        m.team_scores = read_team_scores(r)?;
    }
    if r.read_bool()? {
        let mask = r.read_u32()?;
        let mut players = Vec::with_capacity(mask.count_ones() as usize);
        for id in 0..MAX_SHIPS {
            if mask & (1 << id) == 0 {
                continue;
            }
            let p = match b.players.iter().find(|x| x.id == id) {
                Some(bp) if !r.read_bool()? => *bp,
                _ => read_player_fields(r, id)?,
            };
            players.push(p);
        }
        m.players = players;
    }
    if r.read_bool()? {
        let n = r.read_bits(4)? as usize;
        if n > MAX_OBJECTIVES {
            return Err(DecodeError::Invalid("too many objectives"));
        }
        let mut out: Vec<PackedObjective> = Vec::with_capacity(n);
        for _ in 0..n {
            let id = r.read_u8()?;
            if out.last().is_some_and(|o| o.id >= id) {
                return Err(DecodeError::Invalid("objective ids not ascending"));
            }
            let o = match b.objectives.iter().find(|x| x.id == id) {
                Some(bo) => {
                    let mut o = if r.read_bool()? {
                        read_objective_meta(r, id)?
                    } else {
                        *bo
                    };
                    o.pos = bo.pos;
                    if r.read_bool()? {
                        for i in 0..3 {
                            o.pos[i] = read_comp(r, bo.pos[i])?;
                        }
                    }
                    o
                }
                None => read_objective_full(r, id)?,
            };
            out.push(o);
        }
        m.objectives = out;
    }
    if r.read_bool()? {
        m.mood = read_mood(r)?;
    }
    read_race_absent(r)?;
    Ok(m)
}

// ----- lobby -----------------------------------------------------------------

/// Lobby phase (`docs/milestone-3-plan.md` §3.1: Lobby -> Countdown ->
/// Match -> Results -> Lobby).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LobbyPhase {
    /// Waiting for players / ready-up (warm-up flying).
    #[default]
    Lobby,
    /// The next match starts when the countdown ends.
    Countdown,
    /// A match is running.
    Match,
    /// The last match's results are shown.
    Results,
}

impl LobbyPhase {
    pub fn to_u8(self) -> u8 {
        match self {
            LobbyPhase::Lobby => 0,
            LobbyPhase::Countdown => 1,
            LobbyPhase::Match => 2,
            LobbyPhase::Results => 3,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => LobbyPhase::Lobby,
            1 => LobbyPhase::Countdown,
            2 => LobbyPhase::Match,
            3 => LobbyPhase::Results,
            _ => return None,
        })
    }
}

/// A connected player as the lobby sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LobbyPlayer {
    /// Wire ship id in the current match.
    pub id: u8,
    /// Team picked for the next match, [`NONE`] = auto-balance.
    pub team: u8,
    pub ready: bool,
    pub spectator: bool,
}

/// Longest room or party code in bytes.
pub const MAX_CODE_LEN: usize = 32;
/// Longest mode id, rule set name or share code in a lobby message.
pub const MAX_LOBBY_NAME_LEN: usize = 64;

/// Server -> client (reliable): lobby state, sent on every change and to a
/// player when they join.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LobbyStateMsg {
    pub phase: LobbyPhase,
    /// Ticks (60 Hz) until the phase ends (countdown, results), 0 = open-ended.
    pub phase_ticks_left: u32,
    /// Room code, empty for the public room.
    pub room_code: String,
    /// Next match: mode (`ls_sim::ModeKind::index`), rule set name and world code.
    pub next_mode: u8,
    pub next_ruleset: String,
    pub next_share_code: String,
    /// Player range of the next match.
    pub min_players: u8,
    pub max_players: u8,
    /// Teams of the next match (0 = free-for-all).
    pub teams: u8,
    /// Players in the room, sorted by id.
    pub players: Vec<LobbyPlayer>,
}

pub(crate) fn write_lobby(w: &mut BitWriter, l: &LobbyStateMsg) -> Result<(), EncodeError> {
    for (s, max, what) in [
        (&l.room_code, MAX_CODE_LEN, "room code too long"),
        (
            &l.next_ruleset,
            MAX_LOBBY_NAME_LEN,
            "rule set name too long",
        ),
        (
            &l.next_share_code,
            MAX_LOBBY_NAME_LEN,
            "share code too long",
        ),
    ] {
        if s.len() > max {
            return Err(EncodeError::OutOfRange(what));
        }
    }
    if l.players.len() > MAX_SHIPS as usize {
        return Err(EncodeError::OutOfRange("too many lobby players"));
    }
    w.write_bits(l.phase.to_u8() as u64, 2);
    w.write_varint(l.phase_ticks_left as u64);
    w.write_str(&l.room_code);
    w.write_u8(l.next_mode);
    w.write_str(&l.next_ruleset);
    w.write_str(&l.next_share_code);
    w.write_u8(l.min_players);
    w.write_u8(l.max_players);
    w.write_u8(l.teams);
    w.write_varint(l.players.len() as u64);
    for p in &l.players {
        w.write_u8(p.id);
        w.write_u8(p.team);
        w.write_bool(p.ready);
        w.write_bool(p.spectator);
    }
    Ok(())
}

pub(crate) fn read_lobby(r: &mut BitReader) -> Result<LobbyStateMsg, DecodeError> {
    let phase =
        LobbyPhase::from_u8(r.read_bits(2)? as u8).ok_or(DecodeError::Invalid("lobby phase"))?;
    let phase_ticks_left = r.read_varint_u32()?;
    let room_code = r.read_string(MAX_CODE_LEN)?;
    let next_mode = r.read_u8()?;
    let next_ruleset = r.read_string(MAX_LOBBY_NAME_LEN)?;
    let next_share_code = r.read_string(MAX_LOBBY_NAME_LEN)?;
    let min_players = r.read_u8()?;
    let max_players = r.read_u8()?;
    let teams = r.read_u8()?;
    let n = r.read_len(MAX_SHIPS as usize, 18)?;
    let mut players = Vec::with_capacity(n);
    for _ in 0..n {
        players.push(LobbyPlayer {
            id: r.read_u8()?,
            team: r.read_u8()?,
            ready: r.read_bool()?,
            spectator: r.read_bool()?,
        });
    }
    Ok(LobbyStateMsg {
        phase,
        phase_ticks_left,
        room_code,
        next_mode,
        next_ruleset,
        next_share_code,
        min_players,
        max_players,
        teams,
        players,
    })
}

// ----- results ---------------------------------------------------------------

/// One row of the results table (`ls_sim::ResultRow` with the wire id and
/// the player's name).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResultRow {
    /// Wire ship id.
    pub id: u8,
    /// Team index, or the player's side id in free-for-all.
    pub side: u8,
    pub kills: u32,
    pub deaths: u32,
    /// Mode score.
    pub score: i32,
    pub name: String,
}

/// Server -> client (reliable): the result of a finished match
/// (`ls_sim::MatchResults`), sent once when the mode ends.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MatchResultsMsg {
    /// Tick the match ended at.
    pub tick: u32,
    /// `ls_sim::ModeKind::index`.
    pub kind: u8,
    /// Number of teams (0 = free-for-all).
    pub teams: u8,
    /// Winning side, [`NONE`] on a draw.
    pub winner: u8,
    /// Final team scores (empty in free-for-all).
    pub team_scores: Vec<i64>,
    /// Rows best first (the mode's order).
    pub rows: Vec<ResultRow>,
}

pub(crate) fn write_results(w: &mut BitWriter, m: &MatchResultsMsg) -> Result<(), EncodeError> {
    if m.team_scores.len() > MAX_MODE_TEAMS {
        return Err(EncodeError::OutOfRange("too many team scores"));
    }
    if m.rows.len() > MAX_SHIPS as usize {
        return Err(EncodeError::OutOfRange("too many result rows"));
    }
    if m.rows.iter().any(|r| r.name.len() > MAX_PLAYER_NAME) {
        return Err(EncodeError::OutOfRange("player name too long"));
    }
    w.write_u32(m.tick);
    w.write_u8(m.kind);
    w.write_u8(m.teams);
    w.write_u8(m.winner);
    write_team_scores(w, &m.team_scores);
    w.write_varint(m.rows.len() as u64);
    for row in &m.rows {
        w.write_u8(row.id);
        w.write_u8(row.side);
        w.write_varint(row.kills as u64);
        w.write_varint(row.deaths as u64);
        w.write_varint(zz32(row.score));
        w.write_str(&row.name);
    }
    Ok(())
}

pub(crate) fn read_results(r: &mut BitReader) -> Result<MatchResultsMsg, DecodeError> {
    let tick = r.read_u32()?;
    let kind = r.read_u8()?;
    let teams = r.read_u8()?;
    let winner = r.read_u8()?;
    let team_scores = read_team_scores(r)?;
    let n = r.read_len(MAX_SHIPS as usize, 48)?;
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        rows.push(ResultRow {
            id: r.read_u8()?,
            side: r.read_u8()?,
            kills: r.read_varint_u32()?,
            deaths: r.read_varint_u32()?,
            score: unzz32(r.read_varint_u32()?),
            name: r.read_string(MAX_PLAYER_NAME)?,
        });
    }
    Ok(MatchResultsMsg {
        tick,
        kind,
        teams,
        winner,
        team_scores,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn sample() -> PackedModeState {
        PackedModeState {
            kind: 1,
            teams: 4,
            phase: mode_phase::RUNNING,
            winner: NONE,
            end_tick: 36_000,
            stage: 2,
            stage_end_tick: 4_000,
            score_total: 1 << 40,
            score_target: 50,
            flags: 0b1001,
            team_scores: vec![3, -1, 0, i64::MAX],
            players: (0..32u8)
                .map(|id| NetModePlayer {
                    id,
                    side: id % 4,
                    score: id as i32 - 3,
                    lives: if id % 3 == 0 { UNLIMITED_LIVES } else { 2 },
                    meter: 100,
                    flags: if id == 5 {
                        mode_player_flags::ELIMINATED | mode_player_flags::SPECTATOR
                    } else {
                        0
                    },
                })
                .collect(),
            objectives: vec![
                PackedObjective {
                    id: 0,
                    kind: 4,
                    team: NONE,
                    holder: 4,
                    state: 1,
                    timer_end: 1200,
                    radius: 1.5f32.to_bits(),
                    pos: [100, 30_000, 65_535],
                },
                PackedObjective {
                    id: 7,
                    kind: 3,
                    team: 1,
                    holder: NONE,
                    state: 0,
                    timer_end: 0,
                    radius: 10.0f32.to_bits(),
                    pos: [1, 2, 3],
                },
            ],
            mood: PackedMoodView::default(),
        }
    }

    fn sample_mood() -> PackedMoodView {
        PackedMoodView::from(&NetMoodView {
            mood: 3,
            event: 10,
            phase: mood_phase::ACTIVE,
            target: 7,
            phase_start_tick: 12_345,
            phase_end_tick: 13_245,
            params: [1, 2, 3, 4, 250, 0, 9, 255],
            up: Vec3::new(0.6, 0.0, -0.8),
        })
    }

    #[test]
    fn mood_round_trips_in_full_and_delta_snapshots() {
        let base = sample();
        let mut m = sample();
        m.mood = sample_mood();
        check_mode(&m).unwrap();
        full_round_trip(&m);
        // Only the mood changed: section bits plus the mood (4 bytes, two
        // varints, 8 parameter bytes, 1 + 96 bits of up).
        let bits = delta_round_trip(&m, &base);
        assert!(bits <= 7 + 32 + 32 + 64 + 97, "{bits}");
        delta_round_trip(&base, &m);
        // The default up costs one bit.
        let mut flat = m.clone();
        flat.mood.up = PackedMoodView::default().up;
        let bits_flat = delta_round_trip(&flat, &base);
        assert_eq!(bits - bits_flat, 96);
        // Unchanged mood: the whole mode section is one bit.
        assert_eq!(delta_round_trip(&m, &m), 1);
        // A default mode state with only a mood is still sent.
        let only = PackedModeState {
            mood: sample_mood(),
            ..Default::default()
        };
        assert!(full_round_trip(&only) > 1);
        // Pack and unpack keep the view exactly.
        let q = Quantizer::default();
        let net = NetModeState {
            mood: NetMoodView::from(&sample_mood()),
            ..Default::default()
        };
        assert_eq!(q.unpack_mode(&q.pack_mode(&net)).mood, net.mood);
        assert_eq!(NetMoodView::default().up, Vec3::Y);
        assert_eq!(NetMoodView::default().event, NONE);
    }

    #[test]
    fn mood_validation_and_reserved_race_section() {
        let mut m = sample();
        m.mood.up[1] = f32::INFINITY.to_bits();
        assert!(check_mode(&m).is_err());
        // A race section (B6) is rejected until it exists.
        let mut w = BitWriter::new();
        write_mode_full(&mut w, &sample());
        let len = w.bit_len();
        let mut bytes = w.finish();
        // The race bit is the last bit written.
        let i = len - 1;
        bytes[i / 8] |= 1 << (i % 8);
        assert!(read_mode_full(&mut BitReader::new(&bytes)).is_err());
    }

    fn full_round_trip(m: &PackedModeState) -> usize {
        let mut w = BitWriter::new();
        write_mode_full(&mut w, m);
        let bits = w.bit_len();
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(&read_mode_full(&mut r).unwrap(), m);
        bits
    }

    fn delta_round_trip(m: &PackedModeState, b: &PackedModeState) -> usize {
        let mut w = BitWriter::new();
        write_mode_delta(&mut w, m, b);
        let bits = w.bit_len();
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(&read_mode_delta(&mut r, b).unwrap(), m);
        bits
    }

    #[test]
    fn mode_state_full_and_delta_round_trip() {
        let m = sample();
        check_mode(&m).unwrap();
        full_round_trip(&m);
        assert_eq!(full_round_trip(&PackedModeState::default()), 1);
        // Unchanged: one bit.
        assert_eq!(delta_round_trip(&m, &m), 1);
        // One player's score: section bits + mask + 31 unchanged bits + one row.
        let mut next = m.clone();
        next.players[10].score += 1;
        let bits = delta_round_trip(&next, &m);
        assert!(bits <= 5 + 32 + 32 + 48, "{bits}");
        // The carried objective moves a little: a small position diff.
        let mut moved = m.clone();
        moved.objectives[0].pos[0] += 3;
        let bits = delta_round_trip(&moved, &m);
        assert!(bits < 64, "{bits}");
        // Every section changes; players leave and join, objectives appear.
        let mut other = sample();
        other.phase = mode_phase::ENDED;
        other.winner = 3;
        other.team_scores = vec![1, 2];
        other.players.retain(|p| p.id % 2 == 0);
        other.players[0].flags = mode_player_flags::CARRIER;
        other.objectives = vec![PackedObjective {
            id: 3,
            ..Default::default()
        }];
        delta_round_trip(&other, &m);
        delta_round_trip(&m, &other);
        delta_round_trip(&PackedModeState::default(), &m);
        delta_round_trip(&m, &PackedModeState::default());
    }

    #[test]
    fn mode_state_validation() {
        let mut m = sample();
        m.team_scores.push(5);
        assert!(check_mode(&m).is_err());
        let mut m = sample();
        m.objectives = vec![PackedObjective::default(); 9];
        assert!(check_mode(&m).is_err());
        let mut m = sample();
        m.players.swap(0, 1);
        assert!(check_mode(&m).is_err());
        let mut m = sample();
        m.objectives.reverse();
        assert!(check_mode(&m).is_err());
        let mut m = sample();
        m.objectives[0].radius = f32::NAN.to_bits();
        assert!(check_mode(&m).is_err());
    }

    #[test]
    fn pack_unpack_mode_keeps_everything_but_quantizes_positions() {
        let q = Quantizer::default();
        let m = NetModeState {
            kind: 5,
            phase: mode_phase::ENDED,
            winner: 9,
            end_tick: 99,
            players: vec![
                NetModePlayer {
                    id: 9,
                    side: 9,
                    score: 4,
                    lives: 1,
                    meter: 66,
                    flags: 0,
                },
                NetModePlayer {
                    id: 2,
                    ..Default::default()
                },
            ],
            objectives: vec![NetObjective {
                id: 1,
                kind: 4,
                team: NONE,
                holder: 9,
                state: 2,
                timer_end: 7,
                radius: 15.0,
                pos: Vec3::new(10.0, 5.0, 3.0),
            }],
            ..Default::default()
        };
        let packed = q.pack_mode(&m);
        assert_eq!(packed.players[0].id, 2, "sorted by id");
        let back = q.unpack_mode(&packed);
        assert!((back.objectives[0].pos - m.objectives[0].pos).length() < 0.01);
        assert_eq!(back.objectives[0].radius, 15.0);
        assert_eq!(back.player(9), m.player(9));
        assert!(!back.is_running());
    }
}
