//! Wire-level data types and their bit encodings.
//!
//! These structs are deliberately independent of `ls-sim`: the server and
//! the client convert between simulation state and these types. Two forms
//! exist for snapshot content:
//!
//! - `Net*` types ([`NetShip`], [`NetProjectile`], [`NetEntity`],
//!   [`Snapshot`]) hold `f32` values and are what game code reads and writes.
//!   [`NetPlayer`] (names, teams, ship types) needs no quantization.
//! - `Packed*` types hold the quantized integers that actually go on the
//!   wire. Delta compression works on these, so it is exact.
//!
//! [`Quantizer::pack_snapshot`] and [`Quantizer::unpack_snapshot`] convert.

use ls_math::terrain::paint_color;
use ls_math::{Quat, TerrainEdit, Vec3};

use crate::bits::{BitReader, BitWriter};
use crate::error::{DecodeError, EncodeError};
use crate::mode::{NetModeState, PackedModeState};
use crate::quant::{pack_quat, unpack_quat, PackedVec3, Quantizer};

/// Ship ids are `0..MAX_SHIPS`; a snapshot carries them as a 32-bit mask.
pub const MAX_SHIPS: u8 = 32;

/// Bit assignments of [`NetShip::flags`]. Bits 0-14 are free for game use;
/// the constants below are the layout `ls-game` uses. Bit 15 is reserved for
/// the codec (it signals that a [`ShipStatus`] follows) and is always cleared.
pub mod ship_flags {
    pub const ALIVE: u16 = 1 << 0;
    pub const BOOSTING: u16 = 1 << 1;
    pub const FIRING_PRIMARY: u16 = 1 << 2;
    pub const FIRING_SECONDARY: u16 = 1 << 3;
    pub const SCAVENGING: u16 = 1 << 4;
    pub const OVERDRIVE: u16 = 1 << 5;
    /// The ship's gadget is switched on (which gadget: `NetPlayer::gadget`).
    pub const SHIELDED: u16 = 1 << 6;
    /// Alias of [`SHIELDED`]: any gadget (Shield, Cloak) is on.
    pub const GADGET_ACTIVE: u16 = SHIELDED;
    pub const SPAWN_PROTECTED: u16 = 1 << 7;
    /// Team id (0-3) in bits 8-9.
    pub const TEAM_SHIFT: u32 = 8;
    pub const TEAM_MASK: u16 = 0b11 << TEAM_SHIFT;
    /// Selected primary weapon id (0-31) in bits 10-14.
    pub const WEAPON_SHIFT: u32 = 10;
    pub const WEAPON_MASK: u16 = 0b1_1111 << WEAPON_SHIFT;
    /// Reserved for the wire format; cleared by `Quantizer::pack_ship`.
    pub const WIRE_RESERVED: u16 = 1 << 15;

    #[inline]
    pub fn team(flags: u16) -> u8 {
        ((flags & TEAM_MASK) >> TEAM_SHIFT) as u8
    }

    #[inline]
    pub fn with_team(flags: u16, team: u8) -> u16 {
        (flags & !TEAM_MASK) | (((team as u16) << TEAM_SHIFT) & TEAM_MASK)
    }

    #[inline]
    pub fn weapon(flags: u16) -> u8 {
        ((flags & WEAPON_MASK) >> WEAPON_SHIFT) as u8
    }

    #[inline]
    pub fn with_weapon(flags: u16, slot: u8) -> u16 {
        (flags & !WEAPON_MASK) | (((slot as u16) << WEAPON_SHIFT) & WEAPON_MASK)
    }
}

/// Bits of [`NetShip::ext`] (protocol 6): ship state beyond the 16 core
/// flag bits, sent in a trailer only when non-zero (so a ship without it
/// still costs exactly 18 bytes).
pub mod ship_ext {
    /// Pigmented on a monochrome level (drawn in colour, revealed to
    /// everyone; `docs/milestone-4-plan.md` §3.4).
    pub const PIGMENTED: u8 = 1 << 0;
    /// Hats worn (Hat rain), saturating at 31, in bits 3-7.
    pub const HATS_SHIFT: u32 = 3;
    pub const HATS_MASK: u8 = 0b1_1111 << HATS_SHIFT;
    /// Most hats the wire shows on one ship.
    pub const MAX_HATS: u8 = 31;

    #[inline]
    pub fn hats(ext: u8) -> u8 {
        (ext & HATS_MASK) >> HATS_SHIFT
    }

    /// `ext` with the hat count set (saturating at [`MAX_HATS`]).
    #[inline]
    pub fn with_hats(ext: u8, hats: u32) -> u8 {
        (ext & !HATS_MASK) | ((hats.min(MAX_HATS as u32) as u8) << HATS_SHIFT)
    }
}

/// Wire bits of a projectile's kind byte (protocol 6). Weapon kinds are
/// below 128; bit 7 carries the `CHICKEN` flag (Rubber chickens: drawn as a
/// rubber chicken, same flight), so the flag costs nothing on the wire.
/// [`NetProjectile::chicken`] is the decoded flag, [`NetProjectile::kind`]
/// never has bit 7 set.
pub mod proj_wire {
    pub const CHICKEN: u8 = 1 << 7;
    pub const KIND_MASK: u8 = 0x7f;
}

/// Layout of a dynamic gravity well entity (protocol 6;
/// `ReplicatedKind::Well`, built by `ls_game::convert::well_entities` from
/// `ls_sim::gravity::DynamicWell`). The client predictor rebuilds the well:
///
/// ```text
/// state[0]    range in RANGE_UNIT (0.25 u) steps (up to 63.75 u)
/// state[1]    peak pull in PEAK_UNIT (0.125 u/s^2) steps (up to 31.875)
/// state[2..4] low 16 bits of the end tick (exclusive; little-endian)
/// team        start: 0 = active at the snapshot's tick, else the ticks
///             from the snapshot's tick to the start (1-255; wells starting
///             later are not sent yet)
/// ```
///
/// Range and pull are exact when the simulation creates wells on this grid
/// ([`well_state::quantize_range`], [`well_state::quantize_peak`]); the
/// position is an entity position (about 0.03 u steps).
pub mod well_state {
    /// Range step (u).
    pub const RANGE_UNIT: f32 = 0.25;
    /// Peak pull step (u/s^2).
    pub const PEAK_UNIT: f32 = 0.125;
    /// A well starting later than this many ticks after the snapshot is not
    /// sent yet.
    pub const MAX_START_LEAD: u64 = 255;

    fn byte(v: f32, unit: f32) -> u8 {
        let q = ls_math::det::round(v / unit);
        if q.is_nan() {
            0
        } else {
            q.clamp(0.0, 255.0) as u8
        }
    }

    /// `range` rounded to the wire grid (clamped to 0..=63.75 u).
    pub fn quantize_range(range: f32) -> f32 {
        byte(range, RANGE_UNIT) as f32 * RANGE_UNIT
    }

    /// `peak` (u/s^2) rounded to the wire grid (clamped to 0..=31.875).
    pub fn quantize_peak(peak: f32) -> f32 {
        byte(peak, PEAK_UNIT) as f32 * PEAK_UNIT
    }

    /// State bytes and team byte of a well with `range` (u), peak pull
    /// `peak` (u/s^2), active from `start_tick` to `end_tick` (exclusive),
    /// sent in the snapshot of tick `now`. `None` if it starts more than
    /// [`MAX_START_LEAD`] ticks later.
    pub fn encode(
        range: f32,
        peak: f32,
        start_tick: u64,
        end_tick: u64,
        now: u64,
    ) -> Option<([u8; 4], u8)> {
        let team = if start_tick <= now {
            0
        } else if start_tick - now <= MAX_START_LEAD {
            (start_tick - now) as u8
        } else {
            return None;
        };
        let end = (end_tick as u16).to_le_bytes();
        Some((
            [
                byte(range, RANGE_UNIT),
                byte(peak, PEAK_UNIT),
                end[0],
                end[1],
            ],
            team,
        ))
    }

    /// A decoded well: range (u), peak pull (u/s^2), start and end tick.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct WellState {
        pub range: f32,
        pub peak: f32,
        pub start_tick: u64,
        pub end_tick: u64,
    }

    /// Decodes a well entity of the snapshot of tick `now`: the end tick is
    /// the one nearest to `now` with the sent low 16 bits (wells last far
    /// less than 9 minutes), the start `now + team` (`now` when active).
    pub fn decode(state: [u8; 4], team: u8, now: u64) -> WellState {
        let low = u16::from_le_bytes([state[2], state[3]]) as u64;
        let mut end = (now & !0xffff) | low;
        if end + 0x8000 < now {
            end += 0x1_0000;
        } else if end > now + 0x8000 && end >= 0x1_0000 {
            end -= 0x1_0000;
        }
        let start = now + team as u64;
        WellState {
            range: state[0] as f32 * RANGE_UNIT,
            peak: state[1] as f32 * PEAK_UNIT,
            start_tick: start,
            end_tick: end,
        }
    }
}

/// Private resource state of a ship, quantized to one byte each. The server
/// normally sends it only for the receiving player's own ship (it is not part
/// of the 18-byte core state). The simulation defines the scaling
/// (for example hull points clamped to 255, energy in percent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShipStatus {
    pub hull: u8,
    pub energy: u8,
    pub boost: u8,
    pub scrap: u8,
}

/// Ship state as game code sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetShip {
    /// `0..MAX_SHIPS`, usually the player id.
    pub id: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    /// See [`ship_flags`]; bit 15 is ignored.
    pub flags: u16,
    /// [`ship_ext`] bits (protocol 6: pigment, hats); 0 for most ships.
    pub ext: u8,
    pub status: Option<ShipStatus>,
}

/// Quantized ship: exactly 18 bytes of core state (pos 6, rot 4, vel 6,
/// flags 2) plus the optional 4-byte status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedShip {
    pub id: u8,
    pub pos: PackedVec3,
    pub rot: u32,
    pub vel: PackedVec3,
    /// Bit 15 must be clear.
    pub flags: u16,
    /// [`ship_ext`] bits.
    pub ext: u8,
    pub status: Option<ShipStatus>,
}

/// Bits of a ship's core state on the wire (18 bytes).
pub const SHIP_CORE_BITS: usize = 48 + 32 + 48 + 16;

/// A projectile in flight. Projectiles never inherit shooter velocity, so
/// clients can extrapolate them between snapshots (steered ones change
/// velocity; the next snapshot corrects).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetProjectile {
    /// Server-assigned id, unique among live projectiles (wraps around).
    pub id: u16,
    /// Weapon kind index as defined by the rule set / weapon table.
    pub kind: u8,
    /// Ship id of the shooter.
    pub owner: u8,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Weapon-defined state bits (stuck, armed, returning, ...; see
    /// `ls_sim::proj_flags`).
    pub state: u8,
    /// Drawn as a rubber chicken (protocol 6, `ls_sim::proj_flags::CHICKEN`;
    /// bit 7 of the kind byte on the wire, see [`proj_wire`]).
    pub chicken: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedProjectile {
    pub id: u16,
    /// Wire kind byte: the weapon kind, plus [`proj_wire::CHICKEN`].
    pub kind: u8,
    pub owner: u8,
    pub pos: PackedVec3,
    pub vel: PackedVec3,
    pub state: u8,
}

/// Bits of a projectile in a full snapshot (17 bytes).
pub const PROJECTILE_BITS: usize = 16 + 8 + 8 + 48 + 48 + 8;

/// A generic replicated entity: debris, pickups, scrap piles, wrecks,
/// decoys, ghosts, dust clouds, markers. The game defines `kind`, `visual`
/// and the meaning of `state`; the codec only moves the numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetEntity {
    /// Unique among live entities (the server keeps it stable while the
    /// entity lives). Separate id space from projectiles.
    pub id: u16,
    pub kind: u8,
    /// Index into the match's visual table.
    pub visual: u16,
    /// Owner ship id, or 255.
    pub owner: u8,
    /// Team, or 255.
    pub team: u8,
    pub pos: Vec3,
    /// Quantized like ship velocities (`±max_ship_speed`).
    pub vel: Vec3,
    pub rot: Quat,
    pub state: [u8; 4],
}

/// Quantized [`NetEntity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackedEntity {
    pub id: u16,
    pub kind: u8,
    pub visual: u16,
    pub owner: u8,
    pub team: u8,
    pub pos: PackedVec3,
    pub vel: PackedVec3,
    pub rot: u32,
    pub state: [u8; 4],
}

/// Bits of a resting entity (zero velocity, no rotation) in a full snapshot,
/// including its 16-bit id: 17 bytes plus 2 presence bits.
pub const ENTITY_MIN_BITS: usize = 16 + 8 + 16 + 8 + 8 + 48 + 1 + 1 + 32;

/// A connected player as every client sees it (names above ships, the
/// scoreboard, the ship type for hitboxes and prediction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetPlayer {
    /// Ship id.
    pub id: u8,
    pub team: u8,
    /// Ship type index (`ls_sim::ShipType::index`).
    pub ship_type: u8,
    /// Gadget id in the gadget slot (`ls_sim::GadgetId`), 255 for none.
    pub gadget: u8,
    pub name: String,
}

/// Longest player name on the wire (bytes).
pub const MAX_PLAYER_NAME: usize = 32;

/// Decides per client which snapshot items it receives: interest
/// management and hidden ships (Cloak). The server applies it with
/// [`Snapshot::retain_relevant`] before packing each client's snapshot.
pub trait Relevance {
    /// Whether `viewer` receives `ship` (the viewer's own ship is always kept).
    fn ship(&self, _viewer: u8, _ship: &NetShip) -> bool {
        true
    }
    fn projectile(&self, _viewer: u8, _p: &NetProjectile) -> bool {
        true
    }
    fn entity(&self, _viewer: u8, _e: &NetEntity) -> bool {
        true
    }
}

/// Everything is relevant to everyone.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllRelevant;

impl Relevance for AllRelevant {}

/// Snapshot contents as game code sees them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// Server tick (60 Hz) the snapshot was taken at.
    pub tick: u32,
    /// Tick of the newest input command from this client the server applied.
    pub ack_input: u32,
    /// Sequence number (index in the match's edit log) of `edits[0]`.
    pub edit_base: u32,
    /// Terrain edits the client may not have yet, in log order.
    pub edits: Vec<TerrainEdit>,
    pub ships: Vec<NetShip>,
    pub projectiles: Vec<NetProjectile>,
    /// Generic entities, sorted by id.
    pub entities: Vec<NetEntity>,
    /// Connected players (names, teams, ship types), sorted by id.
    pub players: Vec<NetPlayer>,
    /// Mode-defined scores, e.g. per-team painted share in 1/65535 units.
    pub scores: Vec<u32>,
    /// Ticks until the match ends.
    pub match_ticks_left: u32,
    /// Generic mode state (protocol 5): phase, scores, lives, objectives.
    pub mode: NetModeState,
}

impl Snapshot {
    /// Drops the ships, projectiles and entities `viewer` should not see
    /// (its own ship is always kept).
    pub fn retain_relevant(&mut self, viewer: u8, r: &dyn Relevance) {
        self.ships.retain(|s| s.id == viewer || r.ship(viewer, s));
        self.projectiles.retain(|p| r.projectile(viewer, p));
        self.entities.retain(|e| r.entity(viewer, e));
    }
}

/// Quantized snapshot; ships and projectiles sorted by id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackedSnapshot {
    pub tick: u32,
    pub ack_input: u32,
    pub edit_base: u32,
    pub edits: Vec<TerrainEditBits>,
    pub ships: Vec<PackedShip>,
    pub projectiles: Vec<PackedProjectile>,
    pub entities: Vec<PackedEntity>,
    pub players: Vec<NetPlayer>,
    pub scores: Vec<u32>,
    pub match_ticks_left: u32,
    /// Generic mode state with quantized objective positions.
    pub mode: PackedModeState,
    /// Per-axis ship motion scale for delta prediction (see
    /// [`Quantizer::ship_motion_scale`](crate::Quantizer::ship_motion_scale)):
    /// packed position steps one packed ship velocity step moves per tick, in
    /// units of `2^-MOTION_SCALE_SHIFT`. Zero disables prediction (deltas
    /// stay exact, only larger). [`Quantizer::pack_snapshot`] fills it.
    pub ship_motion: [u32; 3],
    /// The same for projectiles (packed position steps one packed
    /// projectile velocity step moves per tick, see
    /// [`Quantizer::projectile_motion_scale`](crate::Quantizer::projectile_motion_scale));
    /// [`Quantizer::pack_snapshot`] fills it only when the snapshot carries
    /// projectiles.
    pub projectile_motion: [u32; 3],
}

impl PackedSnapshot {
    /// Sequence number one past the last edit carried.
    pub fn edit_end(&self) -> u32 {
        self.edit_base.wrapping_add(self.edits.len() as u32)
    }

    /// The edits a client that already applied `known` edits still needs.
    /// Returns `None` if there is a gap (`known < edit_base`), meaning the
    /// client missed edits and must resynchronize.
    pub fn edits_after(&self, known: u32) -> Option<&[TerrainEditBits]> {
        if known < self.edit_base {
            return None;
        }
        let skip = (known - self.edit_base) as usize;
        Some(self.edits.get(skip..).unwrap_or(&[]))
    }

    pub fn ship(&self, id: u8) -> Option<&PackedShip> {
        self.ships
            .binary_search_by_key(&id, |s| s.id)
            .ok()
            .map(|i| &self.ships[i])
    }

    pub fn projectile(&self, id: u16) -> Option<&PackedProjectile> {
        self.projectiles
            .binary_search_by_key(&id, |p| p.id)
            .ok()
            .map(|i| &self.projectiles[i])
    }

    pub fn entity(&self, id: u16) -> Option<&PackedEntity> {
        self.entities
            .binary_search_by_key(&id, |e| e.id)
            .ok()
            .map(|i| &self.entities[i])
    }
}

/// A terrain edit stored bit-exactly (edits are sent as raw `f32` bits so
/// every peer replays an identical log). `Eq` holds because comparison is on
/// bit patterns. Converts losslessly to and from [`TerrainEdit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainEditBits {
    /// 0 = subtract sphere, 1 = add sphere, 2 = paint, 3 = surface states.
    pub kind: u8,
    pub center: [u32; 3],
    pub radius: u32,
    /// Paint team for paint, state bits for surface states (0 for geometry edits).
    pub team: u8,
    /// Paint colour id for paint (`0..4`, `ls_math::terrain::PAINT_*`; 0 for
    /// every other kind).
    pub color: u8,
    /// Expiry tick of surface states (0 for every other kind).
    pub until: u32,
}

impl From<TerrainEdit> for TerrainEditBits {
    fn from(e: TerrainEdit) -> Self {
        let (kind, c, r, team, color, until) = match e {
            TerrainEdit::SubtractSphere { center, radius } => (0, center, radius, 0, 0, 0),
            TerrainEdit::AddSphere { center, radius } => (1, center, radius, 0, 0, 0),
            TerrainEdit::Paint {
                center,
                radius,
                team,
                color,
            } => (2, center, radius, team, paint_color(color), 0),
            TerrainEdit::Surface {
                center,
                radius,
                states,
                until,
            } => (3, center, radius, states, 0, until),
        };
        TerrainEditBits {
            kind,
            center: c.to_bits(),
            radius: r.to_bits(),
            team,
            color,
            until,
        }
    }
}

impl From<TerrainEditBits> for TerrainEdit {
    fn from(e: TerrainEditBits) -> Self {
        let center = Vec3::new(
            f32::from_bits(e.center[0]),
            f32::from_bits(e.center[1]),
            f32::from_bits(e.center[2]),
        );
        let radius = f32::from_bits(e.radius);
        match e.kind {
            0 => TerrainEdit::SubtractSphere { center, radius },
            1 => TerrainEdit::AddSphere { center, radius },
            3 => TerrainEdit::Surface {
                center,
                radius,
                states: e.team,
                until: e.until,
            },
            _ => TerrainEdit::Paint {
                center,
                radius,
                team: e.team,
                color: paint_color(e.color),
            },
        }
    }
}

/// Minimum encoded size of a terrain edit in bits (geometry edit).
pub(crate) const EDIT_MIN_BITS: usize = 2 + 4 * 32;

/// Terrain edit: 2-bit kind, center and radius as raw `f32`, 8-bit team and
/// 2-bit colour for paint, 8-bit states and 32-bit expiry tick for surface
/// states (130, 140 or 170 bits).
pub(crate) fn write_edit(w: &mut BitWriter, e: &TerrainEditBits) {
    w.write_bits(e.kind as u64, 2);
    for c in e.center {
        w.write_u32(c);
    }
    w.write_u32(e.radius);
    if e.kind >= 2 {
        w.write_u8(e.team);
    }
    if e.kind == 2 {
        w.write_bits(paint_color(e.color) as u64, 2);
    }
    if e.kind == 3 {
        w.write_u32(e.until);
    }
}

pub(crate) fn read_edit(r: &mut BitReader) -> Result<TerrainEditBits, DecodeError> {
    let kind = r.read_bits(2)? as u8;
    let center = [
        r.read_finite_f32()?.to_bits(),
        r.read_finite_f32()?.to_bits(),
        r.read_finite_f32()?.to_bits(),
    ];
    let radius = r.read_finite_f32()?;
    if radius < 0.0 {
        return Err(DecodeError::Invalid("negative edit radius"));
    }
    let team = if kind >= 2 { r.read_u8()? } else { 0 };
    let color = if kind == 2 { r.read_bits(2)? as u8 } else { 0 };
    let until = if kind == 3 { r.read_u32()? } else { 0 };
    Ok(TerrainEditBits {
        kind,
        center,
        radius: radius.to_bits(),
        team,
        color,
        until,
    })
}

impl Quantizer {
    /// Quantizes a ship. Bit 15 of `flags` is cleared.
    pub fn pack_ship(&self, s: &NetShip) -> PackedShip {
        PackedShip {
            id: s.id,
            pos: self.pack_position(s.pos),
            rot: pack_quat(s.rot),
            vel: self.pack_ship_velocity(s.vel),
            flags: s.flags & !ship_flags::WIRE_RESERVED,
            ext: s.ext,
            status: s.status,
        }
    }

    pub fn unpack_ship(&self, s: &PackedShip) -> NetShip {
        NetShip {
            id: s.id,
            pos: self.unpack_position(s.pos),
            vel: self.unpack_ship_velocity(s.vel),
            rot: unpack_quat(s.rot),
            flags: s.flags,
            ext: s.ext,
            status: s.status,
        }
    }

    pub fn pack_projectile(&self, p: &NetProjectile) -> PackedProjectile {
        debug_assert!(
            p.kind & proj_wire::CHICKEN == 0,
            "weapon kinds are below 128"
        );
        PackedProjectile {
            id: p.id,
            kind: (p.kind & proj_wire::KIND_MASK) | if p.chicken { proj_wire::CHICKEN } else { 0 },
            owner: p.owner,
            pos: self.pack_position(p.pos),
            vel: self.pack_projectile_velocity(p.vel),
            state: p.state,
        }
    }

    pub fn unpack_projectile(&self, p: &PackedProjectile) -> NetProjectile {
        NetProjectile {
            id: p.id,
            kind: p.kind & proj_wire::KIND_MASK,
            owner: p.owner,
            pos: self.unpack_position(p.pos),
            vel: self.unpack_projectile_velocity(p.vel),
            state: p.state,
            chicken: p.kind & proj_wire::CHICKEN != 0,
        }
    }

    pub fn pack_entity(&self, e: &NetEntity) -> PackedEntity {
        PackedEntity {
            id: e.id,
            kind: e.kind,
            visual: e.visual,
            owner: e.owner,
            team: e.team,
            pos: self.pack_entity_position(e.pos),
            vel: self.pack_ship_velocity(e.vel),
            rot: pack_quat(e.rot),
            state: e.state,
        }
    }

    pub fn unpack_entity(&self, e: &PackedEntity) -> NetEntity {
        NetEntity {
            id: e.id,
            kind: e.kind,
            visual: e.visual,
            owner: e.owner,
            team: e.team,
            pos: self.unpack_entity_position(e.pos),
            vel: self.unpack_ship_velocity(e.vel),
            rot: unpack_quat(e.rot),
            state: e.state,
        }
    }

    /// Quantizes a snapshot and sorts ships and projectiles by id. Duplicate
    /// or out-of-range ids are left for the encoder to reject.
    pub fn pack_snapshot(&self, s: &Snapshot) -> PackedSnapshot {
        let mut ships: Vec<PackedShip> = s.ships.iter().map(|x| self.pack_ship(x)).collect();
        ships.sort_by_key(|x| x.id);
        let mut projectiles: Vec<PackedProjectile> = s
            .projectiles
            .iter()
            .map(|x| self.pack_projectile(x))
            .collect();
        projectiles.sort_by_key(|x| x.id);
        let mut entities: Vec<PackedEntity> =
            s.entities.iter().map(|x| self.pack_entity(x)).collect();
        entities.sort_by_key(|x| x.id);
        let mut players = s.players.clone();
        players.sort_by_key(|p| p.id);
        PackedSnapshot {
            tick: s.tick,
            ack_input: s.ack_input,
            edit_base: s.edit_base,
            edits: s.edits.iter().map(|&e| e.into()).collect(),
            ships,
            projectiles,
            entities,
            players,
            scores: s.scores.clone(),
            match_ticks_left: s.match_ticks_left,
            mode: self.pack_mode(&s.mode),
            ship_motion: self.ship_motion_scale(),
            // Only snapshots with projectiles pay for the scale (a delta
            // sends it again when it changes against its baseline).
            projectile_motion: if s.projectiles.is_empty() {
                [0; 3]
            } else {
                self.projectile_motion_scale()
            },
        }
    }

    pub fn unpack_snapshot(&self, s: &PackedSnapshot) -> Snapshot {
        Snapshot {
            tick: s.tick,
            ack_input: s.ack_input,
            edit_base: s.edit_base,
            edits: s.edits.iter().map(|&e| e.into()).collect(),
            ships: s.ships.iter().map(|x| self.unpack_ship(x)).collect(),
            projectiles: s
                .projectiles
                .iter()
                .map(|x| self.unpack_projectile(x))
                .collect(),
            entities: s.entities.iter().map(|x| self.unpack_entity(x)).collect(),
            players: s.players.clone(),
            scores: s.scores.clone(),
            match_ticks_left: s.match_ticks_left,
            mode: self.unpack_mode(&s.mode),
        }
    }
}

/// Button bits of [`NetInput::buttons`].
pub mod buttons {
    pub const BOOST: u16 = 1 << 0;
    pub const FIRE_PRIMARY: u16 = 1 << 1;
    pub const FIRE_SECONDARY: u16 = 1 << 2;
    pub const SCAVENGE: u16 = 1 << 3;
    pub const OVERDRIVE: u16 = 1 << 4;
    pub const HORIZON_ASSIST: u16 = 1 << 5;
    pub const GADGET: u16 = 1 << 6;
    /// V: weapon mode of the selected primary (press edge; Paint Thrower
    /// colour).
    pub const WEAPON_MODE: u16 = 1 << 7;
}

/// One input command for one simulation tick, already in wire units
/// (so it round-trips exactly). Use the helpers to convert from and to
/// floating-point controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetInput {
    /// Client tick this command applies to.
    pub tick: u32,
    /// Thrust axes (right, up, back) in ship space, `-127..=127` for `-1..=1`.
    pub thrust: [i8; 3],
    /// Look deltas (yaw, pitch) this tick in units of [`NetInput::LOOK_UNIT`] radians.
    pub look: [i16; 2],
    /// Roll axis `-127..=127` for `-1..=1`.
    pub roll: i8,
    /// See [`buttons`].
    pub buttons: u16,
    /// Requested weapon slot, 0 = no change.
    pub weapon: u8,
}

impl NetInput {
    /// Radians per look unit (1/8192 rad, about 0.007 degrees; range ±4 rad per tick).
    pub const LOOK_UNIT: f32 = 1.0 / 8192.0;

    /// Builds a command from floating-point controls, clamping axes to
    /// `-1..=1` and look deltas to the representable range.
    pub fn from_controls(
        tick: u32,
        thrust: Vec3,
        look_yaw: f32,
        look_pitch: f32,
        roll: f32,
        buttons: u16,
        weapon: u8,
    ) -> Self {
        NetInput {
            tick,
            thrust: [
                axis_to_i8(thrust.x),
                axis_to_i8(thrust.y),
                axis_to_i8(thrust.z),
            ],
            look: [look_to_i16(look_yaw), look_to_i16(look_pitch)],
            roll: axis_to_i8(roll),
            buttons,
            weapon,
        }
    }

    pub fn thrust_axes(&self) -> Vec3 {
        Vec3::new(
            i8_to_axis(self.thrust[0]),
            i8_to_axis(self.thrust[1]),
            i8_to_axis(self.thrust[2]),
        )
    }

    /// `(yaw, pitch)` in radians.
    pub fn look_radians(&self) -> (f32, f32) {
        (
            self.look[0] as f32 * Self::LOOK_UNIT,
            self.look[1] as f32 * Self::LOOK_UNIT,
        )
    }

    pub fn roll_axis(&self) -> f32 {
        i8_to_axis(self.roll)
    }

    #[inline]
    pub fn pressed(&self, button: u16) -> bool {
        self.buttons & button != 0
    }

    pub(crate) fn same_body(&self, o: &NetInput) -> bool {
        NetInput { tick: 0, ..*self } == NetInput { tick: 0, ..*o }
    }

    pub(crate) fn write_body(&self, w: &mut BitWriter) {
        for t in self.thrust {
            w.write_u8(t as u8);
        }
        w.write_u16(self.look[0] as u16);
        w.write_u16(self.look[1] as u16);
        w.write_u8(self.roll as u8);
        w.write_u16(self.buttons);
        w.write_u8(self.weapon);
    }

    pub(crate) fn read_body(r: &mut BitReader, tick: u32) -> Result<Self, DecodeError> {
        Ok(NetInput {
            tick,
            thrust: [r.read_u8()? as i8, r.read_u8()? as i8, r.read_u8()? as i8],
            look: [r.read_u16()? as i16, r.read_u16()? as i16],
            roll: r.read_u8()? as i8,
            buttons: r.read_u16()?,
            weapon: r.read_u8()?,
        })
    }
}

fn axis_to_i8(v: f32) -> i8 {
    let q = ls_math::det::round(v * 127.0);
    if q.is_nan() {
        0
    } else {
        q.clamp(-127.0, 127.0) as i8
    }
}

fn i8_to_axis(v: i8) -> f32 {
    (v.max(-127) as f32) / 127.0
}

fn look_to_i16(rad: f32) -> i16 {
    let q = ls_math::det::round(rad / NetInput::LOOK_UNIT);
    if q.is_nan() {
        0
    } else {
        q.clamp(-32767.0, 32767.0) as i16
    }
}

/// Gameplay events delivered reliably (hits for feedback, kills for the
/// feed). Positions are raw `f32`; events are rare, so precision wins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NetEvent {
    Hit {
        tick: u32,
        attacker: u8,
        victim: u8,
        weapon: u8,
        damage: u16,
        pos: Vec3,
    },
    Kill {
        tick: u32,
        killer: u8,
        victim: u8,
        weapon: u8,
    },
    Respawn {
        tick: u32,
        ship: u8,
        pos: Vec3,
    },
    MatchOver {
        tick: u32,
        /// `None` for a draw.
        winner_team: Option<u8>,
    },
    /// A game cue (warning, blinding, chime, gadget switch; see
    /// `ls_sim::CueKind`). Ship ids are 255 for none.
    Cue {
        tick: u32,
        kind: u8,
        source: u8,
        target: u8,
        pos: Vec3,
        value: u16,
    },
}

pub(crate) const EVENT_MIN_BITS: usize = 8 + 32;

fn write_vec_raw(w: &mut BitWriter, v: Vec3) {
    w.write_f32(v.x);
    w.write_f32(v.y);
    w.write_f32(v.z);
}

fn read_vec_raw(r: &mut BitReader) -> Result<Vec3, DecodeError> {
    Ok(Vec3::new(
        r.read_finite_f32()?,
        r.read_finite_f32()?,
        r.read_finite_f32()?,
    ))
}

impl NetEvent {
    pub(crate) fn write(&self, w: &mut BitWriter) -> Result<(), EncodeError> {
        match *self {
            NetEvent::Hit {
                tick,
                attacker,
                victim,
                weapon,
                damage,
                pos,
            } => {
                if !pos.is_finite() {
                    return Err(EncodeError::OutOfRange("hit position must be finite"));
                }
                w.write_u8(0);
                w.write_u32(tick);
                w.write_u8(attacker);
                w.write_u8(victim);
                w.write_u8(weapon);
                w.write_u16(damage);
                write_vec_raw(w, pos);
            }
            NetEvent::Kill {
                tick,
                killer,
                victim,
                weapon,
            } => {
                w.write_u8(1);
                w.write_u32(tick);
                w.write_u8(killer);
                w.write_u8(victim);
                w.write_u8(weapon);
            }
            NetEvent::Respawn { tick, ship, pos } => {
                if !pos.is_finite() {
                    return Err(EncodeError::OutOfRange("respawn position must be finite"));
                }
                w.write_u8(2);
                w.write_u32(tick);
                w.write_u8(ship);
                write_vec_raw(w, pos);
            }
            NetEvent::MatchOver { tick, winner_team } => {
                w.write_u8(3);
                w.write_u32(tick);
                w.write_bool(winner_team.is_some());
                if let Some(t) = winner_team {
                    w.write_u8(t);
                }
            }
            NetEvent::Cue {
                tick,
                kind,
                source,
                target,
                pos,
                value,
            } => {
                if !pos.is_finite() {
                    return Err(EncodeError::OutOfRange("cue position must be finite"));
                }
                w.write_u8(4);
                w.write_u32(tick);
                w.write_u8(kind);
                w.write_u8(source);
                w.write_u8(target);
                write_vec_raw(w, pos);
                w.write_u16(value);
            }
        }
        Ok(())
    }

    pub(crate) fn read(r: &mut BitReader) -> Result<Self, DecodeError> {
        let tag = r.read_u8()?;
        let tick = r.read_u32()?;
        Ok(match tag {
            0 => NetEvent::Hit {
                tick,
                attacker: r.read_u8()?,
                victim: r.read_u8()?,
                weapon: r.read_u8()?,
                damage: r.read_u16()?,
                pos: read_vec_raw(r)?,
            },
            1 => NetEvent::Kill {
                tick,
                killer: r.read_u8()?,
                victim: r.read_u8()?,
                weapon: r.read_u8()?,
            },
            2 => NetEvent::Respawn {
                tick,
                ship: r.read_u8()?,
                pos: read_vec_raw(r)?,
            },
            3 => NetEvent::MatchOver {
                tick,
                winner_team: if r.read_bool()? {
                    Some(r.read_u8()?)
                } else {
                    None
                },
            },
            4 => NetEvent::Cue {
                tick,
                kind: r.read_u8()?,
                source: r.read_u8()?,
                target: r.read_u8()?,
                pos: read_vec_raw(r)?,
                value: r.read_u16()?,
            },
            _ => return Err(DecodeError::Invalid("event tag")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn terrain_edits_round_trip_bit_exactly() {
        let edits = [
            TerrainEdit::SubtractSphere {
                center: Vec3::new(1.25, -3.0, 1e-7),
                radius: 3.0,
            },
            TerrainEdit::AddSphere {
                center: Vec3::new(-0.0, 100.0, -128.0),
                radius: 0.5,
            },
            TerrainEdit::Paint {
                center: Vec3::new(5.0, 6.0, 7.0),
                radius: 2.0,
                team: 1,
                color: ls_math::terrain::PAINT_YELLOW,
            },
            TerrainEdit::Surface {
                center: Vec3::new(8.0, 9.0, 10.0),
                radius: 1.5,
                states: 5 | ls_math::SurfaceStates::SCORCH.bits(),
                until: 0xdead_beef,
            },
        ];
        let mut w = BitWriter::new();
        for e in edits {
            write_edit(&mut w, &e.into());
        }
        assert_eq!(w.bit_len(), 130 + 130 + 140 + 170);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        for e in edits {
            let back: TerrainEdit = read_edit(&mut r).unwrap().into();
            assert_eq!(TerrainEditBits::from(back), TerrainEditBits::from(e));
            assert_eq!(back, e);
        }
        r.finish().unwrap();
    }

    #[test]
    fn hostile_edit_is_rejected() {
        let mut w = BitWriter::new();
        write_edit(
            &mut w,
            &TerrainEdit::SubtractSphere {
                center: Vec3::new(f32::NAN, 0.0, 0.0),
                radius: 1.0,
            }
            .into(),
        );
        let bytes = w.finish();
        assert!(read_edit(&mut BitReader::new(&bytes)).is_err());
    }

    #[test]
    fn input_helpers_convert_controls() {
        let i = NetInput::from_controls(
            5,
            Vec3::new(1.0, -1.0, 0.5),
            0.01,
            -0.02,
            2.0,
            buttons::BOOST | buttons::FIRE_PRIMARY,
            3,
        );
        assert_eq!(i.thrust, [127, -127, 64]);
        assert_eq!(i.roll, 127);
        let t = i.thrust_axes();
        assert_eq!((t.x, t.y), (1.0, -1.0));
        assert!((t.z - 0.5).abs() <= 0.5 / 127.0);
        let (yaw, pitch) = i.look_radians();
        assert!((yaw - 0.01).abs() <= NetInput::LOOK_UNIT * 0.5);
        assert!((pitch + 0.02).abs() <= NetInput::LOOK_UNIT * 0.5);
        assert!(i.pressed(buttons::BOOST) && !i.pressed(buttons::SCAVENGE));
        assert_eq!(
            NetInput::from_controls(0, Vec3::splat(f32::NAN), f32::NAN, 1e9, 0.0, 0, 0).look,
            [0, 32767]
        );
    }

    #[test]
    fn flag_helpers() {
        let f = ship_flags::with_weapon(ship_flags::with_team(ship_flags::ALIVE, 2), 9);
        assert_eq!(ship_flags::team(f), 2);
        assert_eq!(ship_flags::weapon(f), 9);
        assert_eq!(f & ship_flags::WIRE_RESERVED, 0);
        // 27 weapons need 5 bits; the reserved bit stays clear.
        let f = ship_flags::with_weapon(0, 31);
        assert_eq!(ship_flags::weapon(f), 31);
        assert_eq!(f & ship_flags::WIRE_RESERVED, 0);
    }

    #[test]
    fn protocol_6_ship_ext_and_chicken_bits() {
        let e = ship_ext::with_hats(ship_ext::PIGMENTED, 7);
        assert_eq!(ship_ext::hats(e), 7);
        assert_ne!(e & ship_ext::PIGMENTED, 0);
        assert_eq!(
            ship_ext::hats(ship_ext::with_hats(0, 500)),
            ship_ext::MAX_HATS
        );
        assert_eq!(ship_ext::with_hats(e, 0), ship_ext::PIGMENTED);
        let q = Quantizer::default();
        let p = NetProjectile {
            id: 9,
            kind: 37,
            owner: 2,
            pos: Vec3::splat(5.0),
            vel: Vec3::X,
            state: 0xff,
            chicken: true,
        };
        let packed = q.pack_projectile(&p);
        assert_eq!(packed.kind, 37 | proj_wire::CHICKEN);
        let back = q.unpack_projectile(&packed);
        assert_eq!((back.kind, back.chicken, back.state), (37, true, 0xff));
        let plain = q.unpack_projectile(&q.pack_projectile(&NetProjectile {
            chicken: false,
            ..p
        }));
        assert_eq!((plain.kind, plain.chicken), (37, false));
    }

    #[test]
    fn well_state_round_trips_on_its_grid() {
        let (st, team) = well_state::encode(16.0, 9.0, 69_000, 70_000, 69_990).unwrap();
        assert_eq!(team, 0, "active at the snapshot");
        let w = well_state::decode(st, team, 69_990);
        assert_eq!((w.range, w.peak, w.end_tick), (16.0, 9.0, 70_000));
        assert!(w.start_tick <= 69_990);
        // A well that starts 120 ticks later (telegraphed) keeps its start.
        let (st, team) = well_state::encode(20.0, 12.0, 1_120, 2_920, 1_000).unwrap();
        assert_eq!(well_state::decode(st, team, 1_000).start_tick, 1_120);
        assert!(well_state::encode(20.0, 12.0, 1_256, 2_920, 1_000).is_none());
        // The end tick is recovered across the 16-bit wrap, before and after.
        for now in [65_530u64, 65_536, 70_000, 69_000, 100] {
            for end in [now, now + 360, now + 1800, now.saturating_sub(5)] {
                let (st, team) = well_state::encode(10.0, 4.0, 0, end, now).unwrap();
                let d = well_state::decode(st, team, now);
                assert_eq!(d.end_tick, end, "now {now} end {end}");
            }
        }
        // Clamped and NaN-safe; grid helpers.
        let (st, team) = well_state::encode(1e9, f32::NAN, 0, 0, 0).unwrap();
        let d = well_state::decode(st, team, 0);
        assert_eq!((d.range, d.peak), (63.75, 0.0));
        assert_eq!(well_state::quantize_range(16.1), 16.0);
        assert_eq!(well_state::quantize_peak(9.06), 9.0);
    }

    #[test]
    fn relevance_filters_snapshots() {
        struct HideShip3;
        impl Relevance for HideShip3 {
            fn ship(&self, viewer: u8, ship: &NetShip) -> bool {
                !(ship.id == 3 && viewer == 1)
            }
            fn entity(&self, _viewer: u8, e: &NetEntity) -> bool {
                e.kind != 9
            }
        }
        let ship = |id| NetShip {
            id,
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            flags: 0,
            ext: 0,
            status: None,
        };
        let entity = |id, kind| NetEntity {
            id,
            kind,
            visual: 0,
            owner: 255,
            team: 255,
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            rot: Quat::IDENTITY,
            state: [0; 4],
        };
        let base = Snapshot {
            ships: vec![ship(1), ship(2), ship(3)],
            entities: vec![entity(1, 0), entity(2, 9)],
            ..Default::default()
        };
        let mut s = base.clone();
        s.retain_relevant(1, &HideShip3);
        assert_eq!(s.ships.iter().map(|s| s.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(s.entities.len(), 1);
        let mut s = base.clone();
        s.retain_relevant(3, &HideShip3);
        assert_eq!(s.ships.len(), 3, "own ship always kept");
        let mut s = base;
        s.retain_relevant(1, &AllRelevant);
        assert_eq!(s.ships.len(), 3);
    }

    #[test]
    fn edits_after_handles_overlap_and_gaps() {
        let e: TerrainEditBits = TerrainEdit::AddSphere {
            center: Vec3::ZERO,
            radius: 1.0,
        }
        .into();
        let s = PackedSnapshot {
            edit_base: 10,
            edits: vec![e; 4],
            ..Default::default()
        };
        assert_eq!(s.edit_end(), 14);
        assert_eq!(s.edits_after(10).unwrap().len(), 4);
        assert_eq!(s.edits_after(12).unwrap().len(), 2);
        assert_eq!(s.edits_after(20).unwrap().len(), 0);
        assert!(s.edits_after(9).is_none());
    }
}
