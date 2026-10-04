//! Protocol 5 mode and lobby items: the generic [`NetModeState`] every mode
//! replicates in snapshots, the lobby state ([`LobbyStateMsg`]) and the
//! match results ([`MatchResultsMsg`]).
//!
//! # Mode state in snapshots
//!
//! The last section of every snapshot (`docs/milestone-3-plan.md` §3.1,
//! "Generic mode state on the wire"):
//!
//! ```text
//! full    1 bit "present" (0: all defaults), then phase u8, varint phase_end_tick, u8 winner,
//!         3 bits team score count (0..=4) + zigzag varint each,
//!         varint player count, per player (id order): u8 id, zigzag varint
//!         score, u8 lives, u8 meter, u8 flags,
//!         4 bits objective count (0..=8), per objective (id order): u8 id,
//!         u8 kind, u8 team, u8 holder, u16 timer, u16 value, 3 x u16 position
//! delta   1 bit "changed"; if changed:
//!         1 bit header changed [phase, phase_end_tick, winner]
//!         1 bit team scores changed [full team scores]
//!         1 bit players changed [u32 presence mask, per present player in id
//!           order: in the baseline 1 bit "changed" + score, lives, meter,
//!           flags if changed; new: score, lives, meter, flags]
//!         1 bit objectives changed [4 bits count, per objective: u8 id; in the
//!           baseline 1 bit meta changed [kind, team, holder, timer, value] +
//!           position as a component diff (1 bit + 2-bit size classes); new:
//!           the full objective]
//! ```
//!
//! Unchanged mode state costs one bit per delta snapshot. Positions are
//! quantized like ship positions (level bounds + 2 u, 16 bits per axis).

use ls_math::Vec3;

use crate::bits::{BitReader, BitWriter};
use crate::error::{DecodeError, EncodeError};
use crate::quant::{PackedVec3, Quantizer};
use crate::wire::{MAX_PLAYER_NAME, MAX_SHIPS};

/// Most team scores in a mode state (teams 0..4).
pub const MAX_MODE_TEAMS: usize = 4;
/// Most objective markers in a mode state.
pub const MAX_OBJECTIVES: usize = 8;
/// `NetModeState::winner` / `NetObjective::holder` / `team` value for none.
pub const NONE: u8 = 255;
/// `NetModePlayer::lives` value for modes without lives (unlimited).
pub const UNLIMITED_LIVES: u8 = 255;

/// Bits of [`NetModePlayer::flags`].
pub mod mode_player_flags {
    /// Out of the match (no lives or colour left); spectates.
    pub const ELIMINATED: u8 = 1 << 0;
    /// Spectating (eliminated, joined an elimination mode late, or asked to).
    pub const SPECTATOR: u8 = 1 << 1;
    /// Ready in the lobby (warm-up).
    pub const READY: u8 = 1 << 2;
    /// Carries the mode's objective (Heist core, Hot Reactor).
    pub const CARRIER: u8 = 1 << 3;
}

/// Match phases as [`NetModeState::phase`] carries them (mirrors
/// `ls_sim`'s mode phase; values not listed are mode-defined).
pub mod mode_phase {
    /// Warm-up flying in the lobby (weapons off).
    pub const WARMUP: u8 = 0;
    pub const RUNNING: u8 = 1;
    /// The match is over; `NetModeState::winner` names the winner.
    pub const ENDED: u8 = 2;
}

/// `NetModeState::winner_kind`: what `winner` refers to.
pub mod winner_kind {
    /// No result yet, or a draw once the phase is `ENDED`.
    pub const NONE: u8 = 0;
    pub const TEAM: u8 = 1;
    pub const PLAYER: u8 = 2;
}

/// One player's mode-specific state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetModePlayer {
    /// Wire ship id.
    pub id: u8,
    /// Mode score (kills, deliveries, ...).
    pub score: i32,
    /// Lives left, [`UNLIMITED_LIVES`] when the mode has none.
    pub lives: u8,
    /// Mode meter 0..=255 (Last Color Standing colour; 0 when unused).
    pub meter: u8,
    /// [`mode_player_flags`].
    pub flags: u8,
}

/// An objective marker (Heist core, extraction zone, Hot Reactor, safe
/// zone). `kind`, `timer` and `value` are mode-defined.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NetObjective {
    pub id: u8,
    pub kind: u8,
    /// Owning team, [`NONE`] for none.
    pub team: u8,
    /// Ship id holding it, [`NONE`] for none.
    pub holder: u8,
    /// Ticks left on the objective's timer (fuse, return), 0 for none.
    pub timer: u16,
    /// Mode-defined value (zone radius in 1/16 u, ...).
    pub value: u16,
    pub pos: Vec3,
}

/// [`NetObjective`] with a quantized position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PackedObjective {
    pub id: u8,
    pub kind: u8,
    pub team: u8,
    pub holder: u8,
    pub timer: u16,
    pub value: u16,
    pub pos: PackedVec3,
}

/// Generic mode state of every mode (protocol 5): phase and timer, team or
/// player scores, lives, and objective markers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetModeState {
    /// [`mode_phase`].
    pub phase: u8,
    /// Server tick the phase ends at (0 = open-ended).
    pub phase_end_tick: u32,
    /// [`winner_kind`].
    pub winner_kind: u8,
    /// Winning team or ship id (see `winner_kind`).
    pub winner: u8,
    /// Scores per team (empty in free-for-all modes).
    pub team_scores: Vec<i32>,
    /// Per-player state, sorted by id (at most 32: the top scores).
    pub players: Vec<NetModePlayer>,
    /// Objective markers, sorted by id.
    pub objectives: Vec<NetObjective>,
}

impl NetModeState {
    /// The entry of ship `id`, if any.
    pub fn player(&self, id: u8) -> Option<&NetModePlayer> {
        self.players.iter().find(|p| p.id == id)
    }
}

/// [`NetModeState`] with quantized objective positions (what snapshots
/// store and delta-code).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackedModeState {
    pub phase: u8,
    pub phase_end_tick: u32,
    pub winner_kind: u8,
    pub winner: u8,
    pub team_scores: Vec<i32>,
    pub players: Vec<NetModePlayer>,
    pub objectives: Vec<PackedObjective>,
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
                timer: o.timer,
                value: o.value,
                pos: self.pack_position(o.pos),
            })
            .collect();
        objectives.sort_by_key(|o| o.id);
        PackedModeState {
            phase: m.phase,
            phase_end_tick: m.phase_end_tick,
            winner_kind: m.winner_kind,
            winner: m.winner,
            team_scores: m.team_scores.clone(),
            players,
            objectives,
        }
    }

    pub fn unpack_mode(&self, m: &PackedModeState) -> NetModeState {
        NetModeState {
            phase: m.phase,
            phase_end_tick: m.phase_end_tick,
            winner_kind: m.winner_kind,
            winner: m.winner,
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
                    timer: o.timer,
                    value: o.value,
                    pos: self.unpack_position(o.pos),
                })
                .collect(),
        }
    }
}

#[inline]
fn zz(v: i32) -> u64 {
    (((v << 1) ^ (v >> 31)) as u32) as u64
}

#[inline]
fn unzz(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
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
        prev = Some(o.id);
    }
    Ok(())
}

fn write_header(w: &mut BitWriter, m: &PackedModeState) {
    w.write_u8(m.phase);
    w.write_varint(m.phase_end_tick as u64);
    w.write_bits(m.winner_kind as u64, 2);
    w.write_u8(m.winner);
}

fn read_header(r: &mut BitReader, m: &mut PackedModeState) -> Result<(), DecodeError> {
    m.phase = r.read_u8()?;
    m.phase_end_tick = r.read_varint_u32()?;
    m.winner_kind = r.read_bits(2)? as u8;
    m.winner = r.read_u8()?;
    Ok(())
}

fn write_team_scores(w: &mut BitWriter, s: &[i32]) {
    w.write_bits(s.len() as u64, 3);
    for &v in s {
        w.write_varint(zz(v));
    }
}

fn read_team_scores(r: &mut BitReader) -> Result<Vec<i32>, DecodeError> {
    let n = r.read_bits(3)? as usize;
    if n > MAX_MODE_TEAMS {
        return Err(DecodeError::Invalid("too many team scores"));
    }
    (0..n).map(|_| Ok(unzz(r.read_varint_u32()?))).collect()
}

fn write_player_fields(w: &mut BitWriter, p: &NetModePlayer) {
    w.write_varint(zz(p.score));
    w.write_u8(p.lives);
    w.write_u8(p.meter);
    w.write_u8(p.flags);
}

fn read_player_fields(r: &mut BitReader, id: u8) -> Result<NetModePlayer, DecodeError> {
    Ok(NetModePlayer {
        id,
        score: unzz(r.read_varint_u32()?),
        lives: r.read_u8()?,
        meter: r.read_u8()?,
        flags: r.read_u8()?,
    })
}

fn write_objective_meta(w: &mut BitWriter, o: &PackedObjective) {
    w.write_u8(o.kind);
    w.write_u8(o.team);
    w.write_u8(o.holder);
    w.write_u16(o.timer);
    w.write_u16(o.value);
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

fn read_objective_meta(r: &mut BitReader, id: u8) -> Result<PackedObjective, DecodeError> {
    Ok(PackedObjective {
        id,
        kind: r.read_u8()?,
        team: r.read_u8()?,
        holder: r.read_u8()?,
        timer: r.read_u16()?,
        value: r.read_u16()?,
        pos: [0; 3],
    })
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
}

pub(crate) fn read_mode_full(r: &mut BitReader) -> Result<PackedModeState, DecodeError> {
    let mut m = PackedModeState::default();
    if !r.read_bool()? {
        return Ok(m);
    }
    read_header(r, &mut m)?;
    m.team_scores = read_team_scores(r)?;
    let n = r.read_len(MAX_SHIPS as usize, 40)?;
    let mut prev: Option<u8> = None;
    for _ in 0..n {
        let id = r.read_u8()?;
        if id >= MAX_SHIPS || prev.is_some_and(|q| q >= id) {
            return Err(DecodeError::Invalid("mode player ids not ascending"));
        }
        prev = Some(id);
        m.players.push(read_player_fields(r, id)?);
    }
    m.objectives = read_objectives_full(r)?;
    Ok(m)
}

fn read_objectives_full(r: &mut BitReader) -> Result<Vec<PackedObjective>, DecodeError> {
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
        out.push(read_objective_full(r, id)?);
    }
    Ok(out)
}

/// Writes the mode section against the baseline's.
pub(crate) fn write_mode_delta(w: &mut BitWriter, m: &PackedModeState, b: &PackedModeState) {
    let changed = m != b;
    w.write_bool(changed);
    if !changed {
        return;
    }
    let header = (m.phase, m.phase_end_tick, m.winner_kind, m.winner)
        != (b.phase, b.phase_end_tick, b.winner_kind, b.winner);
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
                    let meta = (o.kind, o.team, o.holder, o.timer, o.value)
                        != (bo.kind, bo.team, bo.holder, bo.timer, bo.value);
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
/// Longest mode id or rule set name in a lobby message.
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
    /// Next match: mode id (e.g. `team_brawl`), rule set name and world code.
    pub next_mode: String,
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
        (&l.next_mode, MAX_LOBBY_NAME_LEN, "mode id too long"),
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
    w.write_str(&l.next_mode);
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
    let phase = LobbyPhase::from_u8(r.read_bits(2)? as u8).expect("2 bits");
    let phase_ticks_left = r.read_varint_u32()?;
    let room_code = r.read_string(MAX_CODE_LEN)?;
    let next_mode = r.read_string(MAX_LOBBY_NAME_LEN)?;
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

/// One row of the results table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResultRow {
    pub id: u8,
    /// Team, [`NONE`] in free-for-all.
    pub team: u8,
    pub kills: u16,
    pub deaths: u16,
    /// Mode score.
    pub score: i32,
    pub name: String,
}

/// Server -> client (reliable): the result of a finished match, once, when
/// the mode ends (also to players who join during the results).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MatchResultsMsg {
    /// Tick the match ended at.
    pub tick: u32,
    /// [`winner_kind`] and the winning team or ship id.
    pub winner_kind: u8,
    pub winner: u8,
    /// Final team scores (empty in free-for-all).
    pub team_scores: Vec<i32>,
    /// Rows sorted best first (the mode's order).
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
    w.write_bits(m.winner_kind as u64, 2);
    w.write_u8(m.winner);
    write_team_scores(w, &m.team_scores);
    w.write_varint(m.rows.len() as u64);
    for row in &m.rows {
        w.write_u8(row.id);
        w.write_u8(row.team);
        w.write_u16(row.kills);
        w.write_u16(row.deaths);
        w.write_varint(zz(row.score));
        w.write_str(&row.name);
    }
    Ok(())
}

pub(crate) fn read_results(r: &mut BitReader) -> Result<MatchResultsMsg, DecodeError> {
    let tick = r.read_u32()?;
    let winner_kind = r.read_bits(2)? as u8;
    let winner = r.read_u8()?;
    let team_scores = read_team_scores(r)?;
    let n = r.read_len(MAX_SHIPS as usize, 56)?;
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        rows.push(ResultRow {
            id: r.read_u8()?,
            team: r.read_u8()?,
            kills: r.read_u16()?,
            deaths: r.read_u16()?,
            score: unzz(r.read_varint_u32()?),
            name: r.read_string(MAX_PLAYER_NAME)?,
        });
    }
    Ok(MatchResultsMsg {
        tick,
        winner_kind,
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
            phase: mode_phase::RUNNING,
            phase_end_tick: 36_000,
            winner_kind: winner_kind::NONE,
            winner: NONE,
            team_scores: vec![3, -1, 0, 12],
            players: (0..32u8)
                .map(|id| NetModePlayer {
                    id,
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
                    kind: 1,
                    team: NONE,
                    holder: 4,
                    timer: 1200,
                    value: 0,
                    pos: [100, 30_000, 65_535],
                },
                PackedObjective {
                    id: 7,
                    kind: 2,
                    team: 1,
                    holder: NONE,
                    timer: 0,
                    value: 160,
                    pos: [1, 2, 3],
                },
            ],
        }
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
        full_round_trip(&PackedModeState::default());
        // Unchanged: one bit.
        assert_eq!(delta_round_trip(&m, &m), 1);
        // One player's score: header bits + mask + one changed player.
        let mut next = m.clone();
        next.players[10].score += 1;
        let bits = delta_round_trip(&next, &m);
        assert!(bits < 32 + 32 + 40 + 8, "{bits}");
        // The carried objective moves a little: a small position diff.
        let mut moved = m.clone();
        moved.objectives[0].pos[0] += 3;
        moved.objectives[0].timer -= 2;
        let bits = delta_round_trip(&moved, &m);
        assert!(bits < 120, "{bits}");
        // Every section changes; players leave and join, objectives appear.
        let mut other = sample();
        other.phase = mode_phase::ENDED;
        other.winner_kind = winner_kind::TEAM;
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
    }

    #[test]
    fn pack_unpack_mode_keeps_everything_but_quantizes_positions() {
        let q = Quantizer::default();
        let m = NetModeState {
            phase: mode_phase::RUNNING,
            phase_end_tick: 99,
            winner_kind: winner_kind::PLAYER,
            winner: 2,
            team_scores: vec![],
            players: vec![
                NetModePlayer {
                    id: 9,
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
                timer: 7,
                value: 8,
                pos: Vec3::new(10.0, 5.0, 3.0),
            }],
        };
        let packed = q.pack_mode(&m);
        assert_eq!(packed.players[0].id, 2, "sorted by id");
        let back = q.unpack_mode(&packed);
        assert!((back.objectives[0].pos - m.objectives[0].pos).length() < 0.01);
        assert_eq!(back.player(9), m.player(9));
        assert_eq!(back.winner, 2);
    }
}
