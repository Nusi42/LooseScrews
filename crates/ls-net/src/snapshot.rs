//! Snapshot encoding, full and delta-compressed, plus baseline history.
//!
//! # Layout
//!
//! ```text
//! tick                u32
//! has_baseline        1 bit, then (tick - baseline_tick) as u8 (1..=255)
//! ack_input           zigzag varint of (ack_input - tick) as i32
//! match_ticks_left    varint
//! ship_motion         [delta: 1 bit "changed"] 3 varints (per-axis motion
//!                     scale, see below), then 3 varints projectile motion
//! edits               varint edit_base, varint count, then each edit
//! scores              [delta: 1 bit "changed"] varint count + varint each
//! ships               u32 presence mask (bit i = ship id i), [delta: 3 x 4
//!                     bits typical residual widths - 1 for position,
//!                     velocity, orientation], then per ship in id order:
//!                     full core state, or (delta, ship in baseline) a
//!                     change bit + predicted residuals (below)
//! projectiles         [delta: 4 x 4 bits typical residual widths - 1 for
//!                     position and velocity of changed projectiles,
//!                     velocity and position of new ones] id list (below)
//!                     of projectiles:
//!                     full state (kind, owner, state byte, position,
//!                     velocity), predicted diffs, or (new in a delta) the
//!                     state predicted from the owner (below)
//! entities            [delta: 4 bits typical drift width - 1] id list of
//!                     entities: full state (kind, visual, owner, team,
//!                     position, velocity and rotation with presence bits,
//!                     4 state bytes) or diffs (a drifting entity: 1 bit +
//!                     a non-zero residual group of its position)
//! players             full: varint count, then per player: u8 id, u8 team,
//!                     u8 ship type, u8 gadget, name. Delta: 1 bit
//!                     "changed"; if changed a u32 presence mask, then per
//!                     present player in id order [in baseline: 1 bit
//!                     "changed"] and, if changed or new, team, ship type,
//!                     gadget, name
//! ```
//!
//! An **id list** (projectiles, entities) in a full snapshot is a varint
//! count, then per item its id as a varint gap (the first id, then `id -
//! previous - 1`) and its full state. Against a baseline it is, per item of
//! the baseline's list in id order, 1 bit "present and unchanged" (not sent
//! again) or, if not, 1 bit "still present" followed by the item's diffs;
//! then the new items (ids not in the baseline) as a full id list. Resting
//! debris, pickups and wrecks therefore cost one bit per snapshot.
//!
//! A full ship is exactly [`SHIP_CORE_BITS`] = 144 bits (18 bytes):
//! position 3 x 16, orientation 32, velocity 3 x 16, flags 16. Flag bit 15
//! signals that a 4-byte [`ShipStatus`] follows, so the common case (other
//! players' ships) costs no extra bit.
//!
//! Delta fields are compared as packed integers, so a delta decodes to
//! exactly the packed snapshot the server encoded: compression never adds
//! error beyond the quantization in [`crate::quant`].
//!
//! # Ship deltas
//!
//! A changed ship sends residuals against predictions both ends compute in
//! integers from the baseline ship and the snapshot's `ship_motion` (packed
//! position steps per velocity step per tick, fixed point with
//! [`MOTION_SCALE_SHIFT`] fraction bits, filled by
//! [`Quantizer::pack_snapshot`](crate::Quantizer::pack_snapshot); zero
//! disables prediction):
//!
//! ```text
//! position     residual group against base.pos + base.vel x motion x age
//! velocity     1 bit predictor (0: base.vel; 1: 2 x displacement / (motion
//!              x age) - base.vel, i.e. constant acceleration), then a
//!              residual group
//! orientation  1 bit "changed"; if changed 1 bit "same dropped component":
//!              a non-zero residual group of the three 10-bit field
//!              differences, else the raw 32-bit quaternion
//! flags        1 bit "changed" [+ u16]
//! status       1 bit "changed" [+ 1 bit present + 4 bytes]
//! ```
//!
//! A **residual group** is 1 bit "non-zero", then one width for all three
//! components coded against the section's typical width `b` (`0` = b,
//! `10` = b - 1, `110` = b + 1, `1110` = b - 2, `1111` + 4 bits), then three
//! zigzag values of that width (differences modulo 2^16). The encoder picks
//! each typical width to make the snapshot's width codes shortest. A thrusting,
//! turning ship costs about 70 bits instead of about 130 with plain diffs.
//!
//! # Projectile deltas
//!
//! Projectiles fly straight most of the time, so a delta predicts each one
//! from its baseline state with the snapshot's `projectile_motion` (like
//! ship positions): "unchanged" in the id list means *exactly where the
//! ballistic prediction puts it, nothing else changed* (1 bit). Otherwise:
//!
//! ```text
//! meta         1 bit "changed" [+ kind, owner, state bytes]
//! position     residual group against the prediction
//! velocity     residual group against the baseline velocity
//! ```
//!
//! The server settles every projectile within [`SETTLE_TOLERANCE`] steps of
//! its prediction onto it before encoding ([`settle_projectiles`]), so a
//! straight shot costs one bit per snapshot. A projectile new against the
//! baseline (in a delta's id list: its id gap as `0` + 4 bits below 16,
//! else `1` + varint) is predicted from its owner:
//!
//! ```text
//! owner        0 + 5 bits (below 32), else 1 + u8
//! velocity     1 bit: residual group against the velocity of the owner's
//!              newest projectile already in the list (same kind and
//!              state); 0: kind, state, raw velocity
//! position     with the owner's ship in the snapshot: 5-bit flight time in
//!              quarter ticks and a residual group against
//!              ship.pos + (velocity - ship.vel) x flight time; else raw
//! ```
//!
//! Entity vector components are sent as a 2-bit size class: unchanged,
//! 4-bit or 8-bit zigzag difference, or the raw 16-bit value.

use std::collections::VecDeque;

use crate::bits::{unzigzag16, zigzag16, BitReader, BitWriter};
use crate::error::{DecodeError, EncodeError};
use crate::quant::pack_quat;
use crate::quant::{PackedVec3, MOTION_SCALE_SHIFT};
use crate::wire::{
    read_edit, ship_flags, write_edit, NetPlayer, PackedEntity, PackedProjectile, PackedShip,
    PackedSnapshot, ShipStatus, TerrainEditBits, EDIT_MIN_BITS, ENTITY_MIN_BITS, MAX_PLAYER_NAME,
    MAX_SHIPS, PROJECTILE_BITS,
};

/// Upper bound on edits in one snapshot (the rest wait for the next one).
pub const MAX_EDITS_PER_SNAPSHOT: usize = 1024;
/// Upper bound on projectiles in one snapshot.
pub const MAX_PROJECTILES_PER_SNAPSHOT: usize = 1024;
/// Upper bound on entities in one snapshot.
pub const MAX_ENTITIES_PER_SNAPSHOT: usize = 1024;
/// Upper bound on score entries.
pub const MAX_SCORES: usize = 64;
/// Baselines must be at most this many ticks older than the snapshot.
pub const MAX_BASELINE_AGE: u32 = 255;

/// Source of baseline snapshots for delta encoding and decoding.
pub trait Baselines {
    fn baseline(&self, tick: u32) -> Option<&PackedSnapshot>;
}

/// No baselines: only full snapshots can be encoded or decoded.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBaselines;

impl Baselines for NoBaselines {
    fn baseline(&self, _tick: u32) -> Option<&PackedSnapshot> {
        None
    }
}

/// A single snapshot serves as its own baseline store.
impl Baselines for PackedSnapshot {
    fn baseline(&self, tick: u32) -> Option<&PackedSnapshot> {
        (self.tick == tick).then_some(self)
    }
}

/// Ring buffer of recent packed snapshots, used by the server (one per
/// client: what was sent) and by the client (what was received).
#[derive(Debug, Clone)]
pub struct SnapshotHistory {
    capacity: usize,
    entries: VecDeque<PackedSnapshot>,
}

impl Default for SnapshotHistory {
    /// Holds 128 snapshots: at 30 Hz that is 256 ticks, which covers every
    /// baseline [`MAX_BASELINE_AGE`] allows. Server and client should use the
    /// same capacity, or the server may pick a baseline the client evicted.
    fn default() -> Self {
        SnapshotHistory::new(128)
    }
}

impl SnapshotHistory {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        SnapshotHistory {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    /// Stores a snapshot, replacing one with the same tick and evicting the
    /// oldest when full.
    pub fn push(&mut self, snap: PackedSnapshot) {
        if let Some(existing) = self.entries.iter_mut().find(|s| s.tick == snap.tick) {
            *existing = snap;
            return;
        }
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(snap);
    }

    pub fn get(&self, tick: u32) -> Option<&PackedSnapshot> {
        self.entries.iter().rev().find(|s| s.tick == tick)
    }

    /// Most recently pushed snapshot.
    pub fn latest(&self) -> Option<&PackedSnapshot> {
        self.entries.back()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Server side: the baseline to delta against when sending a snapshot
    /// for `tick` to a client that last acknowledged `acked`. `None` means
    /// send a full snapshot.
    pub fn choose_baseline(&self, tick: u32, acked: Option<u32>) -> Option<u32> {
        let acked = acked?;
        let age = tick.wrapping_sub(acked);
        if (1..=MAX_BASELINE_AGE).contains(&age) && self.get(acked).is_some() {
            Some(acked)
        } else {
            None
        }
    }
}

impl Baselines for SnapshotHistory {
    fn baseline(&self, tick: u32) -> Option<&PackedSnapshot> {
        self.get(tick)
    }
}

fn check_lists(s: &PackedSnapshot) -> Result<(), EncodeError> {
    let mut prev: Option<u8> = None;
    for ship in &s.ships {
        if ship.id >= MAX_SHIPS || prev.is_some_and(|p| p >= ship.id) {
            return Err(EncodeError::BadShipList { max: MAX_SHIPS });
        }
        if ship.flags & ship_flags::WIRE_RESERVED != 0 {
            return Err(EncodeError::ReservedFlag);
        }
        prev = Some(ship.id);
    }
    let mut prev: Option<u16> = None;
    for p in &s.projectiles {
        if prev.is_some_and(|q| q >= p.id) {
            return Err(EncodeError::BadProjectileList);
        }
        prev = Some(p.id);
    }
    if s.projectiles.len() > MAX_PROJECTILES_PER_SNAPSHOT {
        return Err(EncodeError::OutOfRange("too many projectiles"));
    }
    let mut prev: Option<u16> = None;
    for e in &s.entities {
        if prev.is_some_and(|q| q >= e.id) {
            return Err(EncodeError::OutOfRange(
                "entities must be sorted by id and unique",
            ));
        }
        prev = Some(e.id);
    }
    if s.entities.len() > MAX_ENTITIES_PER_SNAPSHOT {
        return Err(EncodeError::OutOfRange("too many entities"));
    }
    let mut prev: Option<u8> = None;
    for p in &s.players {
        if p.id >= MAX_SHIPS || prev.is_some_and(|q| q >= p.id) {
            return Err(EncodeError::OutOfRange(
                "players must be sorted by id and unique",
            ));
        }
        if p.name.len() > MAX_PLAYER_NAME {
            return Err(EncodeError::OutOfRange("player name too long"));
        }
        prev = Some(p.id);
    }
    if s.edits.len() > MAX_EDITS_PER_SNAPSHOT {
        return Err(EncodeError::OutOfRange("too many edits"));
    }
    if s.scores.len() > MAX_SCORES {
        return Err(EncodeError::OutOfRange("too many scores"));
    }
    for e in &s.edits {
        check_edit(e)?;
    }
    Ok(())
}

pub(crate) fn check_edit(e: &TerrainEditBits) -> Result<(), EncodeError> {
    let finite = e
        .center
        .iter()
        .chain(core::iter::once(&e.radius))
        .all(|&b| f32::from_bits(b).is_finite());
    if e.kind > 3 || !finite || f32::from_bits(e.radius) < 0.0 {
        return Err(EncodeError::OutOfRange("invalid terrain edit"));
    }
    Ok(())
}

#[inline]
fn zigzag32(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

#[inline]
fn unzigzag32(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

fn write_vec16(w: &mut BitWriter, v: PackedVec3) {
    for c in v {
        w.write_u16(c);
    }
}

fn read_vec16(r: &mut BitReader) -> Result<PackedVec3, DecodeError> {
    Ok([r.read_u16()?, r.read_u16()?, r.read_u16()?])
}

fn write_status(w: &mut BitWriter, s: &ShipStatus) {
    w.write_u8(s.hull);
    w.write_u8(s.energy);
    w.write_u8(s.boost);
    w.write_u8(s.scrap);
}

fn read_status(r: &mut BitReader) -> Result<ShipStatus, DecodeError> {
    Ok(ShipStatus {
        hull: r.read_u8()?,
        energy: r.read_u8()?,
        boost: r.read_u8()?,
        scrap: r.read_u8()?,
    })
}

/// Writes the 144-bit core state (plus status if present).
pub fn write_ship_full(w: &mut BitWriter, s: &PackedShip) {
    write_vec16(w, s.pos);
    w.write_u32(s.rot);
    write_vec16(w, s.vel);
    let flags = s.flags
        | if s.status.is_some() {
            ship_flags::WIRE_RESERVED
        } else {
            0
        };
    w.write_u16(flags);
    if let Some(st) = &s.status {
        write_status(w, st);
    }
}

pub fn read_ship_full(r: &mut BitReader, id: u8) -> Result<PackedShip, DecodeError> {
    let pos = read_vec16(r)?;
    let rot = r.read_u32()?;
    let vel = read_vec16(r)?;
    let raw = r.read_u16()?;
    let status = if raw & ship_flags::WIRE_RESERVED != 0 {
        Some(read_status(r)?)
    } else {
        None
    };
    Ok(PackedShip {
        id,
        pos,
        rot,
        vel,
        flags: raw & !ship_flags::WIRE_RESERVED,
        status,
    })
}

/// Component diff: 2-bit class (0 same, 1 = 4-bit zigzag, 2 = 8-bit zigzag,
/// 3 = raw 16-bit value).
fn write_comp_diff(w: &mut BitWriter, cur: u16, base: u16) {
    let zz = zigzag16(cur.wrapping_sub(base) as i16);
    if zz == 0 {
        w.write_bits(0, 2);
    } else if zz < 16 {
        w.write_bits(1, 2);
        w.write_bits(zz as u64, 4);
    } else if zz < 256 {
        w.write_bits(2, 2);
        w.write_bits(zz as u64, 8);
    } else {
        w.write_bits(3, 2);
        w.write_u16(cur);
    }
}

fn read_comp_diff(r: &mut BitReader, base: u16) -> Result<u16, DecodeError> {
    Ok(match r.read_bits(2)? {
        0 => base,
        1 => base.wrapping_add(unzigzag16(r.read_bits(4)? as u16) as u16),
        2 => base.wrapping_add(unzigzag16(r.read_bits(8)? as u16) as u16),
        _ => r.read_u16()?,
    })
}

/// 1 bit "changed"; if changed, three component diffs.
fn write_vec_diff(w: &mut BitWriter, cur: PackedVec3, base: PackedVec3) {
    let changed = cur != base;
    w.write_bool(changed);
    if changed {
        for i in 0..3 {
            write_comp_diff(w, cur[i], base[i]);
        }
    }
}

fn read_vec_diff(r: &mut BitReader, base: PackedVec3) -> Result<PackedVec3, DecodeError> {
    if !r.read_bool()? {
        return Ok(base);
    }
    Ok([
        read_comp_diff(r, base[0])?,
        read_comp_diff(r, base[1])?,
        read_comp_diff(r, base[2])?,
    ])
}

/// Rounded `num / den` (halves round up) for `den > 0`.
#[inline]
fn div_round(num: i64, den: i64) -> i64 {
    let (q, r) = (num.div_euclid(den), num.rem_euclid(den));
    if 2 * r >= den {
        q + 1
    } else {
        q
    }
}

/// Signed value of a packed velocity component.
#[inline]
fn signed(v: u16) -> i64 {
    v as i16 as i64
}

/// Ballistic position prediction: the baseline position moved by the
/// baseline velocity for `age` ticks, in packed integers only (both ends
/// compute it bit for bit). Clamped to the packed range like the quantizer.
fn predict_pos(base: &PackedShip, motion: &[u32; 3], age: u32) -> PackedVec3 {
    predict_pos_from(base.pos, base.vel, motion, age)
}

/// [`predict_pos`] for any packed position and velocity (projectiles).
fn predict_pos_from(pos: PackedVec3, vel: PackedVec3, motion: &[u32; 3], age: u32) -> PackedVec3 {
    let mut out = pos;
    for i in 0..3 {
        let disp = div_round(
            signed(vel[i]) * motion[i] as i64 * age as i64,
            1 << MOTION_SCALE_SHIFT,
        );
        out[i] = (pos[i] as i64 + disp).clamp(0, u16::MAX as i64) as u16;
    }
    out
}

/// A projectile as its baseline state predicts it `age` ticks later: moved
/// ballistically, everything else unchanged (what an "unchanged" bit in a
/// projectile delta means).
fn predict_projectile(b: &PackedProjectile, motion: &[u32; 3], age: u32) -> PackedProjectile {
    PackedProjectile {
        pos: predict_pos_from(b.pos, b.vel, motion, age),
        ..*b
    }
}

/// Residuals (position against the ballistic prediction, velocity against
/// the baseline velocity) of a projectile that differs from its prediction.
#[derive(Debug, Clone, Copy)]
struct ProjectileDelta {
    pos: [i16; 3],
    vel: [i16; 3],
}

impl ProjectileDelta {
    fn new(p: &PackedProjectile, pred: &PackedProjectile) -> ProjectileDelta {
        ProjectileDelta {
            pos: residual(p.pos, pred.pos),
            vel: residual(p.vel, pred.vel),
        }
    }
}

/// The newest projectile (highest id) of `owner` among `known`.
fn sibling(known: &[PackedProjectile], owner: u8) -> Option<&PackedProjectile> {
    known
        .iter()
        .filter(|k| k.owner == owner)
        .max_by_key(|k| k.id)
}

/// Flight time steps per tick of a new projectile's position prediction.
const NEW_FLIGHT_STEPS: i64 = 4;
/// Bits of the flight time (in `1 / NEW_FLIGHT_STEPS` ticks; up to 7.75
/// ticks, enough for baselines up to 6 ticks old plus the muzzle offset).
const NEW_FLIGHT_BITS: u32 = 5;
/// Largest flight time a new projectile's position prediction can use.
const NEW_FLIGHT_MAX: u32 = (1 << NEW_FLIGHT_BITS) - 1;

/// Where a new projectile with packed velocity `vel` would be after
/// flying `steps` quarter ticks from its owner's ship, which moved with
/// its current velocity meanwhile: `ship.pos + (vel - ship.vel) x t`, in
/// packed integers only. The flight time also absorbs the muzzle offset
/// (the shot starts just outside the hull, along its velocity).
fn predict_new_pos(
    ship: &PackedShip,
    vel: PackedVec3,
    motion: (&[u32; 3], &[u32; 3]),
    steps: u32,
) -> PackedVec3 {
    let (ship_motion, proj_motion) = motion;
    let mut out = ship.pos;
    for i in 0..3 {
        let num = (signed(vel[i]) * proj_motion[i] as i64
            - signed(ship.vel[i]) * ship_motion[i] as i64)
            * steps as i64;
        let disp = div_round(num, NEW_FLIGHT_STEPS << MOTION_SCALE_SHIFT);
        out[i] = (ship.pos[i] as i64 + disp).clamp(0, u16::MAX as i64) as u16;
    }
    out
}

/// Residuals of a projectile that is new against the baseline, given the
/// items known so far and this snapshot's ships.
#[derive(Debug, Clone, Copy)]
struct NewProjectile {
    /// Velocity against the sibling's (the owner's newest projectile known
    /// so far, same kind and state), if there is one.
    sibling_vel: Option<[i16; 3]>,
    /// Flight time and position residual against [`predict_new_pos`], if
    /// the owner's ship is in the snapshot.
    flight: Option<(u32, [i16; 3])>,
}

impl NewProjectile {
    fn new(
        p: &PackedProjectile,
        known: &[PackedProjectile],
        ships: &[PackedShip],
        motion: (&[u32; 3], &[u32; 3]),
    ) -> NewProjectile {
        let sibling_vel = sibling(known, p.owner)
            .filter(|s| s.kind == p.kind && s.state == p.state)
            .map(|s| residual(p.vel, s.vel));
        let flight = ships
            .binary_search_by_key(&p.owner, |s| s.id)
            .ok()
            .map(|i| {
                (0..=NEW_FLIGHT_MAX)
                    .map(|q| {
                        (
                            q,
                            residual(p.pos, predict_new_pos(&ships[i], p.vel, motion, q)),
                        )
                    })
                    .min_by_key(|&(q, r)| (resid_width(r), q))
                    .expect("non-empty range")
            });
        NewProjectile {
            sibling_vel,
            flight,
        }
    }
}

/// Typical widths (velocity, position) of the new projectiles' residual
/// groups, from a dry run over the new items in list order.
fn new_projectile_widths(
    snap: &PackedSnapshot,
    base: &PackedSnapshot,
    motion: (&[u32; 3], &[u32; 3]),
) -> [u32; 2] {
    let mut known: Vec<PackedProjectile> = snap
        .projectiles
        .iter()
        .filter(|p| base.projectile(p.id).is_some())
        .copied()
        .collect();
    let mut counts = [[0u32; 17]; 2];
    for p in snap
        .projectiles
        .iter()
        .filter(|p| base.projectile(p.id).is_none())
    {
        let n = NewProjectile::new(p, &known, &snap.ships, motion);
        if let Some(r) = n.sibling_vel {
            counts[0][resid_width(r) as usize] += 1;
        }
        if let Some((_, r)) = n.flight {
            counts[1][resid_width(r) as usize] += 1;
        }
        known.push(*p);
    }
    [best_width_base(&counts[0]), best_width_base(&counts[1])]
}

/// A projectile that is new against the baseline (see the module docs):
/// owner (`0` + 5 bits below 32, else `1` + 8 bits), then either 1 bit + a
/// velocity residual group against its sibling, or 0 + kind, state and the
/// raw velocity; then, with the owner's ship in this snapshot, a 5-bit
/// flight time in quarter ticks and a position residual group against
/// [`predict_new_pos`] (the raw position without one). Residual groups are
/// coded against the typical widths `wb` (velocity, position).
fn write_projectile_new(w: &mut BitWriter, p: &PackedProjectile, n: &NewProjectile, wb: [u32; 2]) {
    if p.owner < 32 {
        w.write_bool(false);
        w.write_bits(p.owner as u64, 5);
    } else {
        w.write_bool(true);
        w.write_u8(p.owner);
    }
    match n.sibling_vel {
        Some(r) => {
            w.write_bool(true);
            write_resid3(w, r, wb[0]);
        }
        None => {
            w.write_bool(false);
            w.write_u8(p.kind);
            w.write_u8(p.state);
            write_vec16(w, p.vel);
        }
    }
    match n.flight {
        Some((steps, r)) => {
            w.write_bits(steps as u64, NEW_FLIGHT_BITS);
            write_resid3(w, r, wb[1]);
        }
        None => write_vec16(w, p.pos),
    }
}

fn read_projectile_new(
    r: &mut BitReader,
    id: u16,
    known: &[PackedProjectile],
    ships: &[PackedShip],
    motion: (&[u32; 3], &[u32; 3]),
    wb: [u32; 2],
) -> Result<PackedProjectile, DecodeError> {
    let owner = if r.read_bool()? {
        r.read_u8()?
    } else {
        r.read_bits(5)? as u8
    };
    let (kind, state, vel) = if r.read_bool()? {
        let s = sibling(known, owner).ok_or(DecodeError::Invalid("projectile sibling missing"))?;
        (
            s.kind,
            s.state,
            apply_residual(s.vel, read_resid3(r, wb[0])?),
        )
    } else {
        (r.read_u8()?, r.read_u8()?, read_vec16(r)?)
    };
    let pos = match ships.binary_search_by_key(&owner, |s| s.id) {
        Ok(i) => {
            let steps = r.read_bits(NEW_FLIGHT_BITS)? as u32;
            let pred = predict_new_pos(&ships[i], vel, motion, steps);
            apply_residual(pred, read_resid3(r, wb[1])?)
        }
        Err(_) => read_vec16(r)?,
    };
    Ok(PackedProjectile {
        id,
        kind,
        owner,
        state,
        pos,
        vel,
    })
}

/// Tolerance (packed position steps per axis, about 0.016 u horizontally
/// and 0.008 u vertically at the default quantization) the server settles
/// projectiles with ([`settle_projectiles`]).
pub const SETTLE_TOLERANCE: u16 = 4;

/// Moves every projectile of `snap` that lies within `tolerance` packed
/// position steps (per axis) of the ballistic prediction from `baseline`
/// onto that prediction, when nothing else about it changed. Encoded
/// against `baseline`, such a projectile then costs one bit instead of its
/// rounding residuals. The server calls it on each client's snapshot
/// before encoding and keeps the settled snapshot as that client's
/// history, so both ends stay identical; a settled position is never more
/// than `tolerance` steps per axis (one step is about 0.004 u horizontally
/// and 0.002 u vertically at the default quantization, so 4 steps are about
/// 0.016 u and 0.008 u) from the true one, because every snapshot is compared with the truth
/// again. Without it, the rounding of each packed position costs a
/// straight projectile about 10 bits per snapshot.
pub fn settle_projectiles(snap: &mut PackedSnapshot, baseline: &PackedSnapshot, tolerance: u16) {
    let age = snap.tick.wrapping_sub(baseline.tick);
    if !(1..=MAX_BASELINE_AGE).contains(&age) {
        return;
    }
    let motion = snap.projectile_motion;
    for p in &mut snap.projectiles {
        let Some(b) = baseline.projectile(p.id) else {
            continue;
        };
        let pred = predict_projectile(b, &motion, age);
        let same = p.kind == pred.kind
            && p.owner == pred.owner
            && p.state == pred.state
            && p.vel == pred.vel;
        let close = residual(p.pos, pred.pos)
            .iter()
            .all(|c| c.unsigned_abs() <= tolerance);
        if same && close {
            p.pos = pred.pos;
        }
    }
}

/// Per-snapshot typical widths of the projectile residual groups
/// (position, velocity), like [`WidthBase`] for ships.
fn projectile_widths(deltas: &[ProjectileDelta]) -> [u32; 2] {
    let mut counts = [[0u32; 17]; 2];
    for d in deltas {
        counts[0][resid_width(d.pos) as usize] += 1;
        counts[1][resid_width(d.vel) as usize] += 1;
    }
    [best_width_base(&counts[0]), best_width_base(&counts[1])]
}

/// Velocity predicted from the displacement over `age` ticks, assuming
/// constant acceleration: `2 x mean velocity - baseline velocity`. Keeps the
/// baseline velocity on an axis without a motion scale.
fn predict_vel_from_motion(
    base: &PackedShip,
    pos: PackedVec3,
    motion: &[u32; 3],
    age: u32,
) -> PackedVec3 {
    let mut out = base.vel;
    for i in 0..3 {
        if motion[i] == 0 || age == 0 {
            continue;
        }
        let d = pos[i] as i64 - base.pos[i] as i64;
        let v = div_round((2 * d) << MOTION_SCALE_SHIFT, motion[i] as i64 * age as i64)
            - signed(base.vel[i]);
        out[i] = v.clamp(-(i16::MAX as i64), i16::MAX as i64) as i16 as u16;
    }
    out
}

/// `cur - pred` per component, modulo 2^16 (exact both ways).
#[inline]
fn residual(cur: PackedVec3, pred: PackedVec3) -> [i16; 3] {
    [
        cur[0].wrapping_sub(pred[0]) as i16,
        cur[1].wrapping_sub(pred[1]) as i16,
        cur[2].wrapping_sub(pred[2]) as i16,
    ]
}

#[inline]
fn apply_residual(pred: PackedVec3, r: [i16; 3]) -> PackedVec3 {
    [
        pred[0].wrapping_add(r[0] as u16),
        pred[1].wrapping_add(r[1] as u16),
        pred[2].wrapping_add(r[2] as u16),
    ]
}

/// Bits needed for the zigzag form of the largest residual component
/// (0 for an all-zero residual).
#[inline]
fn resid_width(r: [i16; 3]) -> u32 {
    let m = r.iter().map(|&c| zigzag16(c)).max().unwrap_or(0);
    16 - m.leading_zeros()
}

/// Approximate bits of a residual (used to pick a velocity predictor).
#[inline]
fn resid_cost(r: [i16; 3]) -> u32 {
    match resid_width(r) {
        0 => 1,
        w => 1 + 4 + 3 * w,
    }
}

/// Per-snapshot typical widths of the ship residual groups (position,
/// velocity, orientation), each `1..=16`. Widths are coded relative to them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WidthBase([u32; 3]);

const W_POS: usize = 0;
const W_VEL: usize = 1;
const W_ROT: usize = 2;

impl WidthBase {
    fn write(&self, w: &mut BitWriter) {
        for b in self.0 {
            w.write_bits((b - 1) as u64, 4);
        }
    }

    fn read(r: &mut BitReader) -> Result<WidthBase, DecodeError> {
        let mut out = [0u32; 3];
        for b in &mut out {
            *b = r.read_bits(4)? as u32 + 1;
        }
        Ok(WidthBase(out))
    }

    /// For each group, the base width that makes the width codes of all
    /// changed ships in the snapshot shortest.
    fn of(deltas: &[ShipDelta]) -> WidthBase {
        let mut counts = [[0u32; 17]; 3];
        for d in deltas {
            let groups = [
                Some(d.pos),
                Some(d.vel),
                match d.rot {
                    RotDelta::Fields(f) => Some(f),
                    _ => None,
                },
            ];
            for (g, r) in groups.into_iter().enumerate() {
                if let Some(r) = r {
                    counts[g][resid_width(r) as usize] += 1;
                }
            }
        }
        WidthBase([
            best_width_base(&counts[0]),
            best_width_base(&counts[1]),
            best_width_base(&counts[2]),
        ])
    }
}

/// Bits of the width code for `width` against base `b` (see [`write_width`]).
#[inline]
fn width_code_bits(width: u32, b: u32) -> u32 {
    match width as i32 - b as i32 {
        0 => 1,
        -1 => 2,
        1 => 3,
        -2 => 4,
        _ => 8,
    }
}

/// The base width (1..=16) minimizing the width codes for `counts[width]`
/// groups of each width (index 0, all-zero groups, send no width).
fn best_width_base(counts: &[u32; 17]) -> u32 {
    let mut best = (u64::MAX, 1);
    for b in 1..=16 {
        let bits: u64 = (1..=16u32)
            .map(|w| counts[w as usize] as u64 * width_code_bits(w, b) as u64)
            .sum();
        if bits < best.0 {
            best = (bits, b);
        }
    }
    best.1
}

/// Width code against a base width `b`, shortest for the widths busy
/// snapshots show most (the largest of three residuals clusters at and
/// just below the typical width): `0` = b, `10` = b - 1, `110` = b + 1,
/// `1110` = b - 2, `1111` + 4 bits `width - 1`.
fn write_width(w: &mut BitWriter, width: u32, b: u32) {
    let prefix = match width as i32 - b as i32 {
        0 => 0,
        -1 => 1,
        1 => 2,
        -2 => 3,
        _ => 4,
    };
    for _ in 0..prefix {
        w.write_bool(true);
    }
    if prefix < 4 {
        w.write_bool(false);
    } else {
        w.write_bits((width - 1) as u64, 4);
    }
}

fn read_width(r: &mut BitReader, b: u32) -> Result<u32, DecodeError> {
    let mut prefix = 0;
    while prefix < 4 && r.read_bool()? {
        prefix += 1;
    }
    let width = match prefix {
        0 => b as i32,
        1 => b as i32 - 1,
        2 => b as i32 + 1,
        3 => b as i32 - 2,
        _ => r.read_bits(4)? as i32 + 1,
    };
    if !(1..=16).contains(&width) {
        return Err(DecodeError::Invalid("residual width out of range"));
    }
    Ok(width as u32)
}

/// Three non-zero residuals sharing one width (coded against `base`), then
/// each component as a `width`-bit zigzag value.
fn write_resid3_nz(w: &mut BitWriter, r: [i16; 3], base: u32) {
    let width = resid_width(r).max(1);
    write_width(w, width, base);
    for c in r {
        w.write_bits(zigzag16(c) as u64, width);
    }
}

fn read_resid3_nz(r: &mut BitReader, base: u32) -> Result<[i16; 3], DecodeError> {
    let width = read_width(r, base)?;
    let mut out = [0i16; 3];
    for c in &mut out {
        *c = unzigzag16(r.read_bits(width)? as u16);
    }
    Ok(out)
}

/// 1 bit "non-zero", then [`write_resid3_nz`].
fn write_resid3(w: &mut BitWriter, r: [i16; 3], base: u32) {
    let nz = r != [0; 3];
    w.write_bool(nz);
    if nz {
        write_resid3_nz(w, r, base);
    }
}

fn read_resid3(r: &mut BitReader, base: u32) -> Result<[i16; 3], DecodeError> {
    if r.read_bool()? {
        read_resid3_nz(r, base)
    } else {
        Ok([0; 3])
    }
}

/// The three 10-bit fields of a smallest-three quaternion.
#[inline]
fn quat_fields(q: u32) -> [i16; 3] {
    [
        ((q >> 20) & 0x3FF) as i16,
        ((q >> 10) & 0x3FF) as i16,
        (q & 0x3FF) as i16,
    ]
}

#[derive(Debug, Clone, Copy)]
enum RotDelta {
    Same,
    /// Same dropped component: differences of the three 10-bit fields.
    Fields([i16; 3]),
    Raw(u32),
}

/// A changed ship's residuals against its baseline state.
#[derive(Debug, Clone, Copy)]
struct ShipDelta {
    pos: [i16; 3],
    /// Velocity predicted from the displacement (else: baseline velocity).
    vel_from_motion: bool,
    vel: [i16; 3],
    rot: RotDelta,
}

impl ShipDelta {
    /// Residuals of `s` against its baseline `b` (`age` ticks older,
    /// `motion` from the snapshot header).
    fn new(s: &PackedShip, b: &PackedShip, motion: &[u32; 3], age: u32) -> ShipDelta {
        let pos = residual(s.pos, predict_pos(b, motion, age));
        // Velocity: against the baseline velocity or against the velocity
        // the displacement implies, whichever is cheaper.
        let from_base = residual(s.vel, b.vel);
        let from_motion = residual(s.vel, predict_vel_from_motion(b, s.pos, motion, age));
        let vel_from_motion = resid_cost(from_motion) < resid_cost(from_base);
        let rot = if s.rot == b.rot {
            RotDelta::Same
        } else if s.rot >> 30 == b.rot >> 30 {
            let (cf, bf) = (quat_fields(s.rot), quat_fields(b.rot));
            RotDelta::Fields([cf[0] - bf[0], cf[1] - bf[1], cf[2] - bf[2]])
        } else {
            RotDelta::Raw(s.rot)
        };
        ShipDelta {
            pos,
            vel_from_motion,
            vel: if vel_from_motion {
                from_motion
            } else {
                from_base
            },
            rot,
        }
    }
}

/// Writes a changed ship's diffs (the "changed" bit is already written).
/// See the module docs for the layout.
fn write_ship_delta(
    w: &mut BitWriter,
    s: &PackedShip,
    b: &PackedShip,
    d: &ShipDelta,
    wb: WidthBase,
) {
    write_resid3(w, d.pos, wb.0[W_POS]);
    w.write_bool(d.vel_from_motion);
    write_resid3(w, d.vel, wb.0[W_VEL]);
    match d.rot {
        RotDelta::Same => w.write_bool(false),
        RotDelta::Fields(f) => {
            w.write_bool(true);
            w.write_bool(true);
            write_resid3_nz(w, f, wb.0[W_ROT]);
        }
        RotDelta::Raw(q) => {
            w.write_bool(true);
            w.write_bool(false);
            w.write_u32(q);
        }
    }
    w.write_bool(s.flags != b.flags);
    if s.flags != b.flags {
        w.write_u16(s.flags);
    }
    w.write_bool(s.status != b.status);
    if s.status != b.status {
        w.write_bool(s.status.is_some());
        if let Some(st) = &s.status {
            write_status(w, st);
        }
    }
}

fn read_ship_delta(
    r: &mut BitReader,
    b: &PackedShip,
    motion: &[u32; 3],
    age: u32,
    wb: WidthBase,
) -> Result<PackedShip, DecodeError> {
    if !r.read_bool()? {
        return Ok(*b);
    }
    let pos = apply_residual(predict_pos(b, motion, age), read_resid3(r, wb.0[W_POS])?);
    let vel_pred = if r.read_bool()? {
        predict_vel_from_motion(b, pos, motion, age)
    } else {
        b.vel
    };
    let vel = apply_residual(vel_pred, read_resid3(r, wb.0[W_VEL])?);
    let rot = if r.read_bool()? {
        if r.read_bool()? {
            let d = read_resid3_nz(r, wb.0[W_ROT])?;
            let bf = quat_fields(b.rot);
            let mut rot = b.rot & (3 << 30);
            for (i, shift) in [20u32, 10, 0].into_iter().enumerate() {
                let f = bf[i] as i32 + d[i] as i32;
                if !(0..=0x3FF).contains(&f) {
                    return Err(DecodeError::Invalid("rotation diff out of range"));
                }
                rot |= (f as u32) << shift;
            }
            rot
        } else {
            r.read_u32()?
        }
    } else {
        b.rot
    };
    let flags = if r.read_bool()? {
        let f = r.read_u16()?;
        if f & ship_flags::WIRE_RESERVED != 0 {
            return Err(DecodeError::Invalid("reserved ship flag in delta"));
        }
        f
    } else {
        b.flags
    };
    let status = if r.read_bool()? {
        if r.read_bool()? {
            Some(read_status(r)?)
        } else {
            None
        }
    } else {
        b.status
    };
    Ok(PackedShip {
        id: b.id,
        pos,
        rot,
        vel,
        flags,
        status,
    })
}

fn write_projectile_full(w: &mut BitWriter, p: &PackedProjectile) {
    w.write_u8(p.kind);
    w.write_u8(p.owner);
    w.write_u8(p.state);
    write_vec16(w, p.pos);
    write_vec16(w, p.vel);
}

fn read_projectile_full(r: &mut BitReader, id: u16) -> Result<PackedProjectile, DecodeError> {
    Ok(PackedProjectile {
        id,
        kind: r.read_u8()?,
        owner: r.read_u8()?,
        state: r.read_u8()?,
        pos: read_vec16(r)?,
        vel: read_vec16(r)?,
    })
}

/// Diffs of a projectile that differs from its prediction `pred` (see the
/// module docs): meta bit [+ kind, owner, state], position residual group
/// against the prediction, velocity residual group against the baseline
/// velocity, coded against the section's typical widths `wb`.
fn write_projectile_delta(
    w: &mut BitWriter,
    p: &PackedProjectile,
    pred: &PackedProjectile,
    wb: [u32; 2],
) {
    let meta = p.kind != pred.kind || p.owner != pred.owner || p.state != pred.state;
    w.write_bool(meta);
    if meta {
        w.write_u8(p.kind);
        w.write_u8(p.owner);
        w.write_u8(p.state);
    }
    let d = ProjectileDelta::new(p, pred);
    write_resid3(w, d.pos, wb[0]);
    write_resid3(w, d.vel, wb[1]);
}

fn read_projectile_delta(
    r: &mut BitReader,
    pred: &PackedProjectile,
    wb: [u32; 2],
) -> Result<PackedProjectile, DecodeError> {
    let (kind, owner, state) = if r.read_bool()? {
        (r.read_u8()?, r.read_u8()?, r.read_u8()?)
    } else {
        (pred.kind, pred.owner, pred.state)
    };
    let pos = apply_residual(pred.pos, read_resid3(r, wb[0])?);
    let vel = apply_residual(pred.vel, read_resid3(r, wb[1])?);
    Ok(PackedProjectile {
        id: pred.id,
        kind,
        owner,
        state,
        pos,
        vel,
    })
}

/// Packed zero velocity (signed quantization: zero is exact).
const ZERO_VEL: PackedVec3 = [0, 0, 0];

fn write_entity_full(w: &mut BitWriter, e: &PackedEntity) {
    w.write_u8(e.kind);
    w.write_u16(e.visual);
    w.write_u8(e.owner);
    w.write_u8(e.team);
    write_vec16(w, e.pos);
    let moving = e.vel != ZERO_VEL;
    w.write_bool(moving);
    if moving {
        write_vec16(w, e.vel);
    }
    let rotated = e.rot != pack_quat(ls_math::Quat::IDENTITY);
    w.write_bool(rotated);
    if rotated {
        w.write_u32(e.rot);
    }
    for b in e.state {
        w.write_u8(b);
    }
}

fn read_entity_full(r: &mut BitReader, id: u16) -> Result<PackedEntity, DecodeError> {
    let kind = r.read_u8()?;
    let visual = r.read_u16()?;
    let owner = r.read_u8()?;
    let team = r.read_u8()?;
    let pos = read_vec16(r)?;
    let vel = if r.read_bool()? {
        read_vec16(r)?
    } else {
        ZERO_VEL
    };
    let rot = if r.read_bool()? {
        r.read_u32()?
    } else {
        pack_quat(ls_math::Quat::IDENTITY)
    };
    let mut state = [0u8; 4];
    for b in &mut state {
        *b = r.read_u8()?;
    }
    Ok(PackedEntity {
        id,
        kind,
        visual,
        owner,
        team,
        pos,
        vel,
        rot,
        state,
    })
}

/// Whether a changed entity differs from its baseline only in position
/// (debris drifting).
#[inline]
fn entity_pos_only(e: &PackedEntity, b: &PackedEntity) -> bool {
    PackedEntity { pos: e.pos, ..*b } == *e
}

/// Typical width of the position residual groups of drifting entities.
fn entity_width(cur: &[PackedEntity], base: &[PackedEntity]) -> u32 {
    let mut counts = [0u32; 17];
    for e in cur {
        if let Ok(i) = base.binary_search_by_key(&e.id, |b| b.id) {
            let b = &base[i];
            if e != b && entity_pos_only(e, b) {
                counts[resid_width(residual(e.pos, b.pos)) as usize] += 1;
            }
        }
    }
    best_width_base(&counts)
}

/// Diffs of a changed entity against its baseline state: 1 bit "only the
/// position moved" (debris drifting: a non-zero residual group coded
/// against the snapshot's typical entity width `ew`), else every field
/// group with a change bit.
fn write_entity_delta(w: &mut BitWriter, e: &PackedEntity, b: &PackedEntity, ew: u32) {
    let pos_only = entity_pos_only(e, b);
    w.write_bool(pos_only);
    if pos_only {
        write_resid3_nz(w, residual(e.pos, b.pos), ew);
        return;
    }
    let meta = e.kind != b.kind || e.visual != b.visual || e.owner != b.owner || e.team != b.team;
    w.write_bool(meta);
    if meta {
        w.write_u8(e.kind);
        w.write_u16(e.visual);
        w.write_u8(e.owner);
        w.write_u8(e.team);
    }
    write_vec_diff(w, e.pos, b.pos);
    write_vec_diff(w, e.vel, b.vel);
    w.write_bool(e.rot != b.rot);
    if e.rot != b.rot {
        w.write_u32(e.rot);
    }
    w.write_bool(e.state != b.state);
    if e.state != b.state {
        for x in e.state {
            w.write_u8(x);
        }
    }
}

fn read_entity_delta(
    r: &mut BitReader,
    b: &PackedEntity,
    ew: u32,
) -> Result<PackedEntity, DecodeError> {
    if r.read_bool()? {
        let pos = apply_residual(b.pos, read_resid3_nz(r, ew)?);
        return Ok(PackedEntity { pos, ..*b });
    }
    let (kind, visual, owner, team) = if r.read_bool()? {
        (r.read_u8()?, r.read_u16()?, r.read_u8()?, r.read_u8()?)
    } else {
        (b.kind, b.visual, b.owner, b.team)
    };
    let pos = read_vec_diff(r, b.pos)?;
    let vel = read_vec_diff(r, b.vel)?;
    let rot = if r.read_bool()? { r.read_u32()? } else { b.rot };
    let state = if r.read_bool()? {
        let mut s = [0u8; 4];
        for x in &mut s {
            *x = r.read_u8()?;
        }
        s
    } else {
        b.state
    };
    Ok(PackedEntity {
        id: b.id,
        kind,
        visual,
        owner,
        team,
        pos,
        vel,
        rot,
        state,
    })
}

fn write_player_fields(w: &mut BitWriter, p: &NetPlayer) {
    w.write_u8(p.team);
    w.write_u8(p.ship_type);
    w.write_u8(p.gadget);
    w.write_str(&p.name);
}

fn read_player_fields(r: &mut BitReader, id: u8) -> Result<NetPlayer, DecodeError> {
    Ok(NetPlayer {
        id,
        team: r.read_u8()?,
        ship_type: r.read_u8()?,
        gadget: r.read_u8()?,
        name: r.read_string(MAX_PLAYER_NAME)?,
    })
}

fn write_players(w: &mut BitWriter, players: &[NetPlayer]) {
    w.write_varint(players.len() as u64);
    for p in players {
        w.write_u8(p.id);
        write_player_fields(w, p);
    }
}

/// Players against a baseline: presence mask, then a change bit per player
/// the baseline has; only changed or new players are sent.
fn write_players_delta(w: &mut BitWriter, players: &[NetPlayer], base: &[NetPlayer]) {
    let mask = players.iter().fold(0u32, |m, p| m | (1 << p.id));
    w.write_u32(mask);
    for p in players {
        match base.iter().find(|b| b.id == p.id) {
            Some(b) => {
                w.write_bool(b != p);
                if b != p {
                    write_player_fields(w, p);
                }
            }
            None => write_player_fields(w, p),
        }
    }
}

fn read_players_delta(
    r: &mut BitReader,
    base: &[NetPlayer],
) -> Result<Vec<NetPlayer>, DecodeError> {
    let mask = r.read_u32()?;
    let mut out = Vec::with_capacity(mask.count_ones() as usize);
    for id in 0..MAX_SHIPS {
        if mask & (1 << id) == 0 {
            continue;
        }
        let p = match base.iter().find(|b| b.id == id) {
            Some(b) if !r.read_bool()? => b.clone(),
            _ => read_player_fields(r, id)?,
        };
        out.push(p);
    }
    Ok(out)
}

/// An item of an id list (see the module docs).
trait ListItem: Copy + PartialEq {
    fn id(&self) -> u16;
}

impl ListItem for PackedProjectile {
    fn id(&self) -> u16 {
        self.id
    }
}

impl ListItem for PackedEntity {
    fn id(&self) -> u16 {
        self.id
    }
}

/// Writes items as a full id list (count, id gaps, full states).
fn write_full_list<'a, T: ListItem + 'a>(
    w: &mut BitWriter,
    items: impl ExactSizeIterator<Item = &'a T>,
    full: fn(&mut BitWriter, &T),
) {
    w.write_varint(items.len() as u64);
    let mut next = 0u32;
    for item in items {
        let id = item.id() as u32;
        w.write_varint((id - next) as u64);
        next = id + 1;
        full(w, item);
    }
}

fn read_full_list<T: ListItem>(
    r: &mut BitReader,
    max: usize,
    min_bits: usize,
    out: &mut Vec<T>,
    full: fn(&mut BitReader, u16) -> Result<T, DecodeError>,
) -> Result<(), DecodeError> {
    let n = r.read_len(max, min_bits)?;
    let mut next = 0u32;
    for _ in 0..n {
        let id = next + r.read_varint_u32()?;
        if id > u16::MAX as u32 {
            return Err(DecodeError::Invalid("list id out of range"));
        }
        out.push(full(r, id as u16)?);
        next = id + 1;
    }
    Ok(())
}

/// How an id list codes its items (see [`write_list`] and [`read_list`]).
struct ListCodec<Full, Pred, Delta, New> {
    /// Full state (full snapshots).
    full: Full,
    /// Maps a baseline item to what "unchanged" means: the identity for
    /// entities, the ballistic move for projectiles.
    predict: Pred,
    /// Diffs of an item against its prediction.
    delta: Delta,
    /// An item new against the baseline, given the items known so far
    /// (those carried over from the baseline, then earlier new ones).
    new: New,
}

/// Writes `cur` (sorted, unique ids) as an id list, against `base` if given.
fn write_list<T: ListItem>(
    w: &mut BitWriter,
    cur: &[T],
    base: Option<&[T]>,
    c: ListCodec<
        fn(&mut BitWriter, &T),
        impl Fn(&T) -> T,
        impl Fn(&mut BitWriter, &T, &T),
        impl Fn(&mut BitWriter, &T, &[T]),
    >,
) {
    let Some(base) = base else {
        write_full_list(w, cur.iter(), c.full);
        return;
    };
    let mut known: Vec<T> = Vec::with_capacity(cur.len());
    for b in base {
        match cur.binary_search_by_key(&b.id(), |x| x.id()) {
            Ok(i) => {
                let pred = (c.predict)(b);
                if cur[i] == pred {
                    w.write_bool(true);
                } else {
                    w.write_bool(false);
                    w.write_bool(true);
                    (c.delta)(w, &cur[i], &pred);
                }
                known.push(cur[i]);
            }
            Err(_) => {
                w.write_bool(false);
                w.write_bool(false);
            }
        }
    }
    let new: Vec<&T> = cur
        .iter()
        .filter(|x| base.binary_search_by_key(&x.id(), |b| b.id()).is_err())
        .collect();
    w.write_varint(new.len() as u64);
    let mut next = 0u32;
    for item in new {
        let id = item.id() as u32;
        write_gap(w, id - next);
        next = id + 1;
        (c.new)(w, item, &known);
        known.push(*item);
    }
}

/// Id gap of a new item in a delta's id list: `0` + 4 bits for gaps below
/// 16 (new ids are mostly consecutive), else `1` + varint.
fn write_gap(w: &mut BitWriter, gap: u32) {
    if gap < 16 {
        w.write_bool(false);
        w.write_bits(gap as u64, 4);
    } else {
        w.write_bool(true);
        w.write_varint(gap as u64);
    }
}

fn read_gap(r: &mut BitReader) -> Result<u32, DecodeError> {
    if r.read_bool()? {
        r.read_varint_u32()
    } else {
        Ok(r.read_bits(4)? as u32)
    }
}

/// Reads an id list written by [`write_list`] (with the matching codec);
/// the result is sorted by id. `min_bits` / `new_min_bits`: smallest size
/// of a full / new item including its id gap.
#[allow(clippy::type_complexity)]
fn read_list<T: ListItem>(
    r: &mut BitReader,
    base: Option<&[T]>,
    max: usize,
    min_bits: usize,
    new_min_bits: usize,
    c: ListCodec<
        fn(&mut BitReader, u16) -> Result<T, DecodeError>,
        impl Fn(&T) -> T,
        impl Fn(&mut BitReader, &T) -> Result<T, DecodeError>,
        impl Fn(&mut BitReader, u16, &[T]) -> Result<T, DecodeError>,
    >,
) -> Result<Vec<T>, DecodeError> {
    let mut out: Vec<T> = Vec::new();
    let Some(base) = base else {
        read_full_list(r, max, min_bits, &mut out, c.full)?;
        if out.windows(2).any(|p| p[0].id() >= p[1].id()) {
            return Err(DecodeError::Invalid("list ids not ascending"));
        }
        return Ok(out);
    };
    for b in base {
        if r.read_bool()? {
            out.push((c.predict)(b));
        } else if r.read_bool()? {
            let d = (c.delta)(r, &(c.predict)(b))?;
            out.push(d);
        }
    }
    let kept = out.len();
    let n = r.read_len(max.saturating_sub(kept), new_min_bits)?;
    let mut next = 0u32;
    for _ in 0..n {
        let id = next.saturating_add(read_gap(r)?);
        if id > u16::MAX as u32 {
            return Err(DecodeError::Invalid("list id out of range"));
        }
        let item = (c.new)(r, id as u16, &out)?;
        out.push(item);
        next = id + 1;
    }
    for item in &out[kept..] {
        if base.binary_search_by_key(&item.id(), |b| b.id()).is_ok() {
            return Err(DecodeError::Invalid(
                "new list item already in the baseline",
            ));
        }
    }
    out.sort_by_key(|x| x.id());
    if out.windows(2).any(|p| p[0].id() >= p[1].id()) {
        return Err(DecodeError::Invalid("list ids not unique"));
    }
    Ok(out)
}

/// Smallest size of a full projectile in an id list (8-bit id gap).
const PROJECTILE_LIST_MIN_BITS: usize = PROJECTILE_BITS - 8;
/// Smallest size of a new projectile in a delta's id list (5-bit id gap,
/// 6-bit owner, sibling bit, empty velocity residual group, raw position
/// or flight time and empty residual group).
const PROJECTILE_NEW_MIN_BITS: usize = 5 + 6 + 1 + 1 + 6;
/// Smallest size of a new entity in a delta's id list (5-bit id gap).
const ENTITY_NEW_MIN_BITS: usize = ENTITY_MIN_BITS - 16 + 5;
/// Smallest size of a full entity in an id list (8-bit id gap).
const ENTITY_LIST_MIN_BITS: usize = ENTITY_MIN_BITS - 8;

fn read_players(r: &mut BitReader) -> Result<Vec<NetPlayer>, DecodeError> {
    let n = r.read_len(MAX_SHIPS as usize, 40)?;
    let mut out: Vec<NetPlayer> = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.read_u8()?;
        if id >= MAX_SHIPS || out.last().is_some_and(|p| p.id >= id) {
            return Err(DecodeError::Invalid("player ids not ascending"));
        }
        out.push(NetPlayer {
            id,
            team: r.read_u8()?,
            ship_type: r.read_u8()?,
            gadget: r.read_u8()?,
            name: r.read_string(MAX_PLAYER_NAME)?,
        });
    }
    Ok(out)
}

/// Encodes `snap`, delta-compressed against `baseline` if given.
///
/// The baseline must be 1..=255 ticks older than `snap` (see
/// [`SnapshotHistory::choose_baseline`]).
pub fn write_snapshot(
    w: &mut BitWriter,
    snap: &PackedSnapshot,
    baseline: Option<&PackedSnapshot>,
) -> Result<(), EncodeError> {
    // Validate everything before the first write so an error never leaves a
    // partly written `BitWriter` behind.
    check_lists(snap)?;
    let baseline_age = match baseline {
        Some(b) => {
            let age = snap.tick.wrapping_sub(b.tick);
            if !(1..=MAX_BASELINE_AGE).contains(&age) {
                return Err(EncodeError::BaselineTooOld {
                    tick: snap.tick,
                    baseline: b.tick,
                });
            }
            Some(age as u8)
        }
        None => None,
    };
    w.write_u32(snap.tick);
    match baseline_age {
        Some(age) => {
            w.write_bool(true);
            w.write_u8(age);
        }
        None => w.write_bool(false),
    }
    w.write_varint(zigzag32(snap.ack_input.wrapping_sub(snap.tick) as i32) as u64);
    w.write_varint(snap.match_ticks_left as u64);
    let motion_changed = baseline.is_none_or(|b| {
        b.ship_motion != snap.ship_motion || b.projectile_motion != snap.projectile_motion
    });
    if baseline.is_some() {
        w.write_bool(motion_changed);
    }
    if motion_changed {
        for k in snap.ship_motion.iter().chain(&snap.projectile_motion) {
            w.write_varint(*k as u64);
        }
    }

    w.write_varint(snap.edit_base as u64);
    w.write_varint(snap.edits.len() as u64);
    for e in &snap.edits {
        write_edit(w, e);
    }

    let scores_changed = baseline.is_none_or(|b| b.scores != snap.scores);
    if baseline.is_some() {
        w.write_bool(scores_changed);
    }
    if scores_changed {
        w.write_varint(snap.scores.len() as u64);
        for &s in &snap.scores {
            w.write_varint(s as u64);
        }
    }

    let age = baseline_age.unwrap_or(0) as u32;
    let mask = snap.ships.iter().fold(0u32, |m, s| m | (1 << s.id));
    w.write_u32(mask);
    // Against a baseline: the residuals of every changed ship first, so the
    // typical widths can lead the section.
    let deltas: Vec<Option<ShipDelta>> = snap
        .ships
        .iter()
        .map(|s| {
            let bs = baseline.and_then(|b| b.ship(s.id))?;
            (s != bs).then(|| ShipDelta::new(s, bs, &snap.ship_motion, age))
        })
        .collect();
    let widths = WidthBase::of(&deltas.iter().flatten().copied().collect::<Vec<_>>());
    if baseline.is_some() {
        widths.write(w);
    }
    for (s, d) in snap.ships.iter().zip(&deltas) {
        match baseline.and_then(|b| b.ship(s.id)) {
            Some(bs) => {
                w.write_bool(d.is_some());
                if let Some(d) = d {
                    write_ship_delta(w, s, bs, d, widths);
                }
            }
            None => write_ship_full(w, s),
        }
    }

    // Projectiles: residuals of every projectile that left its predicted
    // path first, so the typical widths can lead the section.
    let pm = snap.projectile_motion;
    let motion = (&snap.ship_motion, &pm);
    let (pwidths, nwidths) = match baseline {
        Some(b) => {
            let deltas: Vec<ProjectileDelta> = snap
                .projectiles
                .iter()
                .filter_map(|p| {
                    let bp = b.projectile(p.id)?;
                    let pred = predict_projectile(bp, &pm, age);
                    (*p != pred).then(|| ProjectileDelta::new(p, &pred))
                })
                .collect();
            let wb = projectile_widths(&deltas);
            let nb = new_projectile_widths(snap, b, motion);
            for x in wb.into_iter().chain(nb) {
                w.write_bits((x - 1) as u64, 4);
            }
            (wb, nb)
        }
        None => ([1; 2], [1; 2]),
    };
    write_list(
        w,
        &snap.projectiles,
        baseline.map(|b| &b.projectiles[..]),
        ListCodec {
            full: write_projectile_full,
            predict: |b: &PackedProjectile| predict_projectile(b, &pm, age),
            delta: |w: &mut BitWriter, p: &PackedProjectile, pred: &PackedProjectile| {
                write_projectile_delta(w, p, pred, pwidths)
            },
            new: |w: &mut BitWriter, p: &PackedProjectile, known: &[PackedProjectile]| {
                let n = NewProjectile::new(p, known, &snap.ships, motion);
                write_projectile_new(w, p, &n, nwidths)
            },
        },
    );
    let ew = match baseline {
        Some(b) => {
            let ew = entity_width(&snap.entities, &b.entities);
            w.write_bits((ew - 1) as u64, 4);
            ew
        }
        None => 1,
    };
    write_list(
        w,
        &snap.entities,
        baseline.map(|b| &b.entities[..]),
        ListCodec {
            full: write_entity_full,
            predict: |b: &PackedEntity| *b,
            delta: |w: &mut BitWriter, e: &PackedEntity, b: &PackedEntity| {
                write_entity_delta(w, e, b, ew)
            },
            new: |w: &mut BitWriter, e: &PackedEntity, _: &[PackedEntity]| write_entity_full(w, e),
        },
    );

    match baseline {
        None => write_players(w, &snap.players),
        Some(b) => {
            let changed = b.players != snap.players;
            w.write_bool(changed);
            if changed {
                write_players_delta(w, &snap.players, &b.players);
            }
        }
    }
    Ok(())
}

/// Decodes a snapshot. Returns it with the baseline tick it was delta
/// encoded against (`None` for a full snapshot).
pub fn read_snapshot(
    r: &mut BitReader,
    baselines: &dyn Baselines,
) -> Result<(PackedSnapshot, Option<u32>), DecodeError> {
    let tick = r.read_u32()?;
    let baseline_tick = if r.read_bool()? {
        let age = r.read_u8()?;
        if age == 0 {
            return Err(DecodeError::Invalid("baseline age 0"));
        }
        Some(tick.wrapping_sub(age as u32))
    } else {
        None
    };
    let baseline = match baseline_tick {
        Some(t) => Some(
            baselines
                .baseline(t)
                .ok_or(DecodeError::MissingBaseline(t))?,
        ),
        None => None,
    };
    let ack_input = tick.wrapping_add(unzigzag32(r.read_varint_u32()?) as u32);
    let match_ticks_left = r.read_varint_u32()?;
    let (ship_motion, projectile_motion) = match baseline {
        Some(b) if !r.read_bool()? => (b.ship_motion, b.projectile_motion),
        _ => (
            [
                r.read_varint_u32()?,
                r.read_varint_u32()?,
                r.read_varint_u32()?,
            ],
            [
                r.read_varint_u32()?,
                r.read_varint_u32()?,
                r.read_varint_u32()?,
            ],
        ),
    };
    let age = baseline_tick.map_or(0, |t| tick.wrapping_sub(t));

    let edit_base = r.read_varint_u32()?;
    let n_edits = r.read_len(MAX_EDITS_PER_SNAPSHOT, EDIT_MIN_BITS)?;
    let mut edits = Vec::with_capacity(n_edits);
    for _ in 0..n_edits {
        edits.push(read_edit(r)?);
    }

    let scores_changed = match baseline {
        Some(_) => r.read_bool()?,
        None => true,
    };
    let scores = if scores_changed {
        let n = r.read_len(MAX_SCORES, 8)?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(r.read_varint_u32()?);
        }
        v
    } else {
        baseline.map(|b| b.scores.clone()).unwrap_or_default()
    };

    let mask = r.read_u32()?;
    let widths = match baseline {
        Some(_) => WidthBase::read(r)?,
        None => WidthBase([1; 3]),
    };
    let mut ships = Vec::with_capacity(mask.count_ones() as usize);
    for id in 0..MAX_SHIPS {
        if mask & (1 << id) == 0 {
            continue;
        }
        let ship = match baseline.and_then(|b| b.ship(id)) {
            Some(bs) => read_ship_delta(r, bs, &ship_motion, age, widths)?,
            None => read_ship_full(r, id)?,
        };
        ships.push(ship);
    }

    let mut widths = [1u32; 4];
    if baseline.is_some() {
        for x in &mut widths {
            *x = r.read_bits(4)? as u32 + 1;
        }
    }
    let (pwidths, nwidths) = ([widths[0], widths[1]], [widths[2], widths[3]]);
    let projectiles = read_list(
        r,
        baseline.map(|b| &b.projectiles[..]),
        MAX_PROJECTILES_PER_SNAPSHOT,
        PROJECTILE_LIST_MIN_BITS,
        PROJECTILE_NEW_MIN_BITS,
        ListCodec {
            full: read_projectile_full,
            predict: |b: &PackedProjectile| predict_projectile(b, &projectile_motion, age),
            delta: |r: &mut BitReader, pred: &PackedProjectile| {
                read_projectile_delta(r, pred, pwidths)
            },
            new: |r: &mut BitReader, id: u16, known: &[PackedProjectile]| {
                let motion = (&ship_motion, &projectile_motion);
                read_projectile_new(r, id, known, &ships, motion, nwidths)
            },
        },
    )?;
    let ew = match baseline {
        Some(_) => r.read_bits(4)? as u32 + 1,
        None => 1,
    };
    let entities = read_list(
        r,
        baseline.map(|b| &b.entities[..]),
        MAX_ENTITIES_PER_SNAPSHOT,
        ENTITY_LIST_MIN_BITS,
        ENTITY_NEW_MIN_BITS,
        ListCodec {
            full: read_entity_full,
            predict: |b: &PackedEntity| *b,
            delta: |r: &mut BitReader, b: &PackedEntity| read_entity_delta(r, b, ew),
            new: |r: &mut BitReader, id: u16, _: &[PackedEntity]| read_entity_full(r, id),
        },
    )?;

    let players = match baseline {
        None => read_players(r)?,
        Some(b) if r.read_bool()? => read_players_delta(r, &b.players)?,
        Some(b) => b.players.clone(),
    };

    Ok((
        PackedSnapshot {
            tick,
            ack_input,
            edit_base,
            edits,
            ships,
            projectiles,
            entities,
            players,
            scores,
            match_ticks_left,
            ship_motion,
            projectile_motion,
        },
        baseline_tick,
    ))
}

// Keep the documented sizes honest at compile time.
const _: () = assert!(crate::wire::SHIP_CORE_BITS == 18 * 8);
const _: () = assert!(PROJECTILE_BITS == 17 * 8);
const _: () = assert!(ENTITY_MIN_BITS == 17 * 8 + 2);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::{pack_quat, Quantizer};
    use crate::wire::{NetEntity, NetProjectile, NetShip, Snapshot, SHIP_CORE_BITS};
    use ls_math::{Pcg32, Quat, TerrainEdit, Vec3};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn random_ship(rng: &mut Pcg32, id: u8, status: bool) -> PackedShip {
        PackedShip {
            id,
            pos: [
                rng.next_u32() as u16,
                rng.next_u32() as u16,
                rng.next_u32() as u16,
            ],
            rot: pack_quat(Quat::from_axis_angle(
                Vec3::new(rng.range_f32(-1.0, 1.0), rng.range_f32(-1.0, 1.0), 0.3),
                rng.range_f32(-3.0, 3.0),
            )),
            vel: [
                rng.next_u32() as u16,
                rng.next_u32() as u16,
                rng.next_u32() as u16,
            ],
            flags: rng.next_u32() as u16 & !ship_flags::WIRE_RESERVED,
            status: status.then(|| ShipStatus {
                hull: rng.next_u32() as u8,
                energy: rng.next_u32() as u8,
                boost: rng.next_u32() as u8,
                scrap: rng.next_u32() as u8,
            }),
        }
    }

    fn random_projectile(rng: &mut Pcg32, id: u16) -> PackedProjectile {
        PackedProjectile {
            id,
            kind: rng.below(8) as u8,
            owner: rng.below(32) as u8,
            state: rng.below(4) as u8,
            pos: [
                rng.next_u32() as u16,
                rng.next_u32() as u16,
                rng.next_u32() as u16,
            ],
            vel: [
                rng.next_u32() as u16,
                rng.next_u32() as u16,
                rng.next_u32() as u16,
            ],
        }
    }

    fn random_entity(rng: &mut Pcg32, id: u16) -> PackedEntity {
        let moving = rng.chance(0.3);
        PackedEntity {
            id,
            kind: rng.below(10) as u8,
            visual: rng.below(80) as u16,
            owner: if rng.chance(0.5) {
                255
            } else {
                rng.below(32) as u8
            },
            team: rng.below(3) as u8,
            pos: [
                rng.next_u32() as u16,
                rng.next_u32() as u16,
                rng.next_u32() as u16,
            ],
            vel: if moving {
                [rng.next_u32() as u16, 0, rng.next_u32() as u16]
            } else {
                ZERO_VEL
            },
            rot: if rng.chance(0.3) {
                rng.next_u32()
            } else {
                pack_quat(Quat::IDENTITY)
            },
            state: [rng.next_u32() as u8, 0, rng.below(2) as u8, 0],
        }
    }

    fn random_players(rng: &mut Pcg32) -> Vec<NetPlayer> {
        let mut out = Vec::new();
        for id in 0..MAX_SHIPS {
            if rng.chance(0.3) {
                out.push(NetPlayer {
                    id,
                    team: id % 2,
                    ship_type: rng.below(4) as u8,
                    gadget: rng.below(4) as u8,
                    name: format!("pilot-{}", rng.below(1000)),
                });
            }
        }
        out
    }

    fn random_snapshot(rng: &mut Pcg32, tick: u32) -> PackedSnapshot {
        let mut ships = Vec::new();
        for id in 0..MAX_SHIPS {
            if rng.chance(0.6) {
                let status = rng.chance(0.1);
                ships.push(random_ship(rng, id, status));
            }
        }
        let mut projectiles = Vec::new();
        let mut id = rng.below(1000) as u16;
        for _ in 0..rng.below(40) {
            projectiles.push(random_projectile(rng, id));
            id += 1 + rng.below(5) as u16;
        }
        let edits = (0..rng.below(4))
            .map(|i| {
                TerrainEditBits::from(TerrainEdit::Paint {
                    center: Vec3::new(i as f32, rng.range_f32(-50.0, 50.0), 3.0),
                    radius: 2.0,
                    team: rng.below(2) as u8,
                    color: rng.below(4) as u8,
                })
            })
            .collect();
        let mut entities = Vec::new();
        let mut id = rng.below(30_000) as u16;
        for _ in 0..rng.below(30) {
            entities.push(random_entity(rng, id));
            id += 1 + rng.below(9) as u16;
        }
        PackedSnapshot {
            tick,
            ack_input: tick.wrapping_add(rng.range_i32(-20, 20) as u32),
            edit_base: rng.below(10_000),
            edits,
            ships,
            projectiles,
            entities,
            players: random_players(rng),
            scores: (0..rng.below(4)).map(|_| rng.below(70_000)).collect(),
            match_ticks_left: rng.below(18_000),
            ship_motion: random_motion(rng),
            projectile_motion: random_motion(rng),
        }
    }

    /// Motion scales: none, the default quantizer's, random and extreme.
    fn random_motion(rng: &mut Pcg32) -> [u32; 3] {
        match rng.below(4) {
            0 => [0; 3],
            1 => Quantizer::default().ship_motion_scale(),
            2 => [
                rng.next_u32() >> 8,
                rng.below(1 << 16),
                rng.next_u32() >> 12,
            ],
            _ => [u32::MAX, 1, u32::MAX],
        }
    }

    /// Moves a snapshot forward the way a game would: small motion, some
    /// ships and projectiles appear or disappear, occasional big jumps.
    fn evolve(rng: &mut Pcg32, base: &PackedSnapshot, dt: u32) -> PackedSnapshot {
        let mut next = base.clone();
        next.tick = base.tick.wrapping_add(dt);
        next.ack_input = next.tick.wrapping_add(3);
        next.match_ticks_left = base.match_ticks_left.saturating_sub(dt);
        next.edits.clear();
        next.edit_base = base.edit_end();
        let jitter = |rng: &mut Pcg32, v: u16, big: bool| {
            let range = if big { 30_000 } else { 200 };
            v.wrapping_add(rng.range_i32(-range, range) as u16)
        };
        next.ships.retain(|_| rng.chance(0.95));
        for s in &mut next.ships {
            if rng.chance(0.2) {
                continue; // unchanged
            }
            let big = rng.chance(0.05);
            for i in 0..3 {
                if rng.chance(0.8) {
                    s.pos[i] = jitter(rng, s.pos[i], big);
                }
                if rng.chance(0.5) {
                    s.vel[i] = jitter(rng, s.vel[i], big);
                }
            }
            if rng.chance(0.7) {
                s.rot = rng.next_u32();
            }
            if rng.chance(0.1) {
                s.flags ^= 1 << rng.below(15);
            }
            if rng.chance(0.1) {
                s.status = match s.status {
                    Some(_) => None,
                    None => Some(ShipStatus {
                        hull: 50,
                        energy: 60,
                        boost: 70,
                        scrap: 3,
                    }),
                };
            } else if let Some(st) = &mut s.status {
                st.energy = st.energy.wrapping_add(1);
            }
        }
        let mut joined = Vec::new();
        for id in 0..MAX_SHIPS {
            if next.ship(id).is_none() && rng.chance(0.05) {
                joined.push(random_ship(rng, id, false));
            }
        }
        next.ships.extend(joined);
        next.ships.sort_by_key(|s| s.id);
        next.projectiles.retain(|_| rng.chance(0.8));
        for p in &mut next.projectiles {
            if rng.chance(0.9) {
                for i in 0..3 {
                    p.pos[i] = jitter(rng, p.pos[i], false);
                }
            }
            if rng.chance(0.05) {
                p.kind ^= 1;
            }
        }
        let start = next
            .projectiles
            .last()
            .map_or(Some(0), |p| p.id.checked_add(1));
        if let Some(start) = start {
            let n = rng.below(4) as u16;
            for id in start..start.saturating_add(n) {
                next.projectiles.push(random_projectile(rng, id));
            }
        }
        if rng.chance(0.3) {
            if let Some(s) = next.scores.first_mut() {
                *s += 17;
            }
        }
        next.entities.retain(|_| rng.chance(0.9));
        for e in &mut next.entities {
            if e.vel != ZERO_VEL {
                for i in 0..3 {
                    e.pos[i] = jitter(rng, e.pos[i], false);
                }
            }
            if rng.chance(0.05) {
                e.state[0] = e.state[0].wrapping_add(1);
            }
            if rng.chance(0.02) {
                e.vel = ZERO_VEL;
            }
        }
        let start = next
            .entities
            .last()
            .map_or(Some(0), |e| e.id.checked_add(1));
        if let Some(start) = start {
            let n = rng.below(3) as u16;
            for id in start..start.saturating_add(n) {
                next.entities.push(random_entity(rng, id));
            }
        }
        if rng.chance(0.05) {
            next.players = random_players(rng);
        }
        if rng.chance(0.05) {
            next.ship_motion = random_motion(rng);
        }
        if rng.chance(0.05) {
            next.projectile_motion = random_motion(rng);
        }
        if rng.chance(0.3) {
            next.edits.push(
                TerrainEdit::SubtractSphere {
                    center: Vec3::new(1.0, 2.0, 3.0),
                    radius: 3.0,
                }
                .into(),
            );
        }
        next
    }

    fn encode(snap: &PackedSnapshot, base: Option<&PackedSnapshot>) -> Vec<u8> {
        let mut w = BitWriter::new();
        write_snapshot(&mut w, snap, base).unwrap();
        w.finish()
    }

    fn decode(bytes: &[u8], baselines: &dyn Baselines) -> (PackedSnapshot, Option<u32>) {
        let mut r = BitReader::new(bytes);
        let out = read_snapshot(&mut r, baselines).unwrap();
        r.finish().unwrap();
        out
    }

    #[test]
    fn full_snapshot_round_trip() {
        let mut rng = Pcg32::new(21, 0);
        for i in 0..300 {
            let snap = random_snapshot(&mut rng, 1000 + i);
            let bytes = encode(&snap, None);
            assert_eq!(decode(&bytes, &NoBaselines), (snap, None));
        }
    }

    #[test]
    fn delta_snapshot_round_trip_is_exact() {
        let mut rng = Pcg32::new(22, 0);
        let mut total_full = 0usize;
        let mut total_delta = 0usize;
        for i in 0..200 {
            let mut base = random_snapshot(&mut rng, (u32::MAX - 100).wrapping_add(i)); // exercise tick wrap
            for _ in 0..10 {
                let dt = 1 + rng.below(MAX_BASELINE_AGE);
                let next = evolve(&mut rng, &base, dt);
                let delta = encode(&next, Some(&base));
                let full = encode(&next, None);
                let (decoded, bt) = decode(&delta, &base);
                assert_eq!(bt, Some(base.tick));
                assert_eq!(decoded, next);
                total_full += full.len();
                total_delta += delta.len();
                base = next;
            }
        }
        println!("delta {total_delta} B vs full {total_full} B over 2000 snapshots");
        // Deltas of small motion must be clearly smaller than full snapshots.
        assert!(
            total_delta * 10 < total_full * 7,
            "delta {total_delta} vs full {total_full}"
        );
    }

    #[test]
    fn unchanged_ships_cost_one_bit_each() {
        let mut rng = Pcg32::new(23, 0);
        let mut base = random_snapshot(&mut rng, 500);
        base.ships = (0..MAX_SHIPS)
            .map(|id| random_ship(&mut rng, id, false))
            .collect();
        base.projectiles.clear();
        base.edits.clear();
        base.entities = (0..64).map(|i| random_entity(&mut rng, i)).collect();
        base.players = random_players(&mut rng);
        let mut next = base.clone();
        next.tick += 2;
        let delta = encode(&next, Some(&base));
        // header (4 + 1 + 1) + ack/time/edit/score varints + motion bit +
        // mask 4 + 12 bits widths + 32 bits for ships + 4 bits entity width
        // + 64 bits (unchanged) for entities + two empty new-item lists + 1
        // bit players.
        assert!(delta.len() <= 24 + 4 + 8 + 2 + 1, "{} bytes", delta.len());
        assert_eq!(decode(&delta, &base).0, next);
    }

    #[test]
    fn missing_or_bad_baselines_are_errors() {
        let mut rng = Pcg32::new(24, 0);
        let base = random_snapshot(&mut rng, 100);
        let next = evolve(&mut rng, &base, 2);
        let delta = encode(&next, Some(&base));
        let mut r = BitReader::new(&delta);
        assert_eq!(
            read_snapshot(&mut r, &NoBaselines).unwrap_err(),
            DecodeError::MissingBaseline(100)
        );
        let mut w = BitWriter::new();
        let mut far = next.clone();
        far.tick = base.tick + 256;
        assert!(matches!(
            write_snapshot(&mut w, &far, Some(&base)),
            Err(EncodeError::BaselineTooOld { .. })
        ));
        let mut same = next.clone();
        same.tick = base.tick;
        let mut w = BitWriter::new();
        assert!(write_snapshot(&mut w, &same, Some(&base)).is_err());
        assert_eq!(w.bit_len(), 0, "a rejected snapshot writes nothing");
    }

    #[test]
    fn invalid_lists_are_rejected_by_encoder() {
        let mut rng = Pcg32::new(25, 0);
        let mut s = PackedSnapshot {
            ships: vec![
                random_ship(&mut rng, 3, false),
                random_ship(&mut rng, 3, false),
            ],
            ..Default::default()
        };
        assert!(write_snapshot(&mut BitWriter::new(), &s, None).is_err());
        s.ships = vec![random_ship(&mut rng, 32, false)];
        assert!(write_snapshot(&mut BitWriter::new(), &s, None).is_err());
        s.ships = vec![random_ship(&mut rng, 1, false)];
        s.ships[0].flags |= ship_flags::WIRE_RESERVED;
        assert_eq!(
            write_snapshot(&mut BitWriter::new(), &s, None),
            Err(EncodeError::ReservedFlag)
        );
        s.ships.clear();
        s.projectiles = vec![
            random_projectile(&mut rng, 9),
            random_projectile(&mut rng, 2),
        ];
        assert!(write_snapshot(&mut BitWriter::new(), &s, None).is_err());
    }

    #[test]
    fn truncated_and_corrupted_input_never_panics() {
        let mut rng = Pcg32::new(26, 0);
        let base = random_snapshot(&mut rng, 7000);
        let next = evolve(&mut rng, &base, 2);
        for bytes in [encode(&base, None), encode(&next, Some(&base))] {
            for cut in 0..bytes.len() {
                let _ = read_snapshot(&mut BitReader::new(&bytes[..cut]), &base);
            }
            for _ in 0..500 {
                let mut b = bytes.clone();
                let i = rng.below(b.len() as u32) as usize;
                b[i] ^= 1 << rng.below(8);
                let _ = read_snapshot(&mut BitReader::new(&b), &base);
            }
        }
    }

    /// Plan 4.4 acceptance: a ship's core state fits in 18 bytes.
    #[test]
    fn ship_core_state_is_at_most_18_bytes() {
        let quant = Quantizer::default();
        let mut rng = Pcg32::new(27, 0);
        for _ in 0..100 {
            let ship = NetShip {
                id: 5,
                pos: Vec3::new(
                    rng.range_f32(0.0, 256.0),
                    rng.range_f32(0.0, 128.0),
                    rng.range_f32(0.0, 256.0),
                ),
                vel: Vec3::new(rng.range_f32(-30.0, 30.0), 0.0, 12.0),
                rot: Quat::from_axis_angle(Vec3::Y, rng.range_f32(-3.0, 3.0)),
                flags: ship_flags::ALIVE | ship_flags::BOOSTING,
                status: None,
            };
            let mut w = BitWriter::new();
            write_ship_full(&mut w, &quant.pack_ship(&ship));
            assert_eq!(w.bit_len(), SHIP_CORE_BITS);
            assert!(w.finish().len() <= 18);
        }
        // Each additional ship grows a full snapshot by exactly 144 bits.
        let mut snap = PackedSnapshot::default();
        let mut last = {
            let mut w = BitWriter::new();
            write_snapshot(&mut w, &snap, None).unwrap();
            w.bit_len()
        };
        for id in 0..MAX_SHIPS {
            snap.ships.push(random_ship(&mut rng, id, false));
            let mut w = BitWriter::new();
            write_snapshot(&mut w, &snap, None).unwrap();
            assert_eq!(w.bit_len() - last, SHIP_CORE_BITS);
            last = w.bit_len();
        }
    }

    /// Plan 4.4 acceptance: 32 ships at 30 Hz stay within 17 KB/s
    /// (17 x 1024 B) before delta compression.
    #[test]
    fn bandwidth_32_ships_at_30_hz() {
        const BUDGET: usize = 17 * 1024;
        let quant = Quantizer::default();
        let mut rng = Pcg32::new(28, 0);
        let ships: Vec<NetShip> = (0..MAX_SHIPS)
            .map(|id| NetShip {
                id,
                pos: Vec3::new(
                    rng.range_f32(0.0, 256.0),
                    rng.range_f32(0.0, 128.0),
                    rng.range_f32(0.0, 256.0),
                ),
                vel: Vec3::new(
                    rng.range_f32(-28.5, 28.5),
                    rng.range_f32(-28.5, 28.5),
                    rng.range_f32(-28.5, 28.5),
                ),
                rot: Quat::from_axis_angle(Vec3::new(0.3, 1.0, -0.2), rng.range_f32(-3.0, 3.0)),
                flags: ship_flags::with_team(ship_flags::ALIVE, id % 2),
                status: None,
            })
            .collect();
        let snap = quant.pack_snapshot(&Snapshot {
            tick: 123_456,
            ack_input: 123_460,
            ships,
            scores: vec![31_000, 29_500],
            match_ticks_left: 9_000,
            ..Default::default()
        });

        // Ship section: presence mask + 32 core states.
        let mut w = BitWriter::new();
        w.write_u32(u32::MAX);
        for s in &snap.ships {
            write_ship_full(&mut w, s);
        }
        let ship_bytes_per_sec = w.finish().len() * 30;
        assert_eq!(ship_bytes_per_sec, (4 + 32 * 18) * 30);
        assert!(ship_bytes_per_sec <= BUDGET, "{ship_bytes_per_sec} B/s");

        // Whole snapshot including the header, tick, ack, scores.
        let full = encode(&snap, None);
        let full_per_sec = full.len() * 30;
        println!(
            "32 ships @ 30 Hz: ship data {ship_bytes_per_sec} B/s, whole snapshot {} B ({full_per_sec} B/s), budget {BUDGET} B/s",
            full.len()
        );
        // The per-snapshot header (tick, ack, timer, edit sequence, scores,
        // counts: about 18 B) comes on top of the ship data; stay under 18 KB/s.
        assert!(full_per_sec <= 18 * 1024, "{full_per_sec} B/s");
        assert!(full.len() <= 1200, "fits one datagram");
    }

    /// A dogfighting ship for the bandwidth tests, stepped at 60 Hz with the
    /// ship tuning's orders of magnitude (`data/ships`): thrust 24 to 30
    /// u/s^2 along a ship-local direction that changes every quarter to half
    /// second, per-axis speed cap 17 u/s (boost), continuous yaw and pitch at
    /// up to `turn` rad/s plus roll, bouncing off the level walls.
    struct BusyShip {
        ship: NetShip,
        thrust: Vec3,
        omega: Vec3,
        accel: f32,
        hold: u32,
    }

    impl BusyShip {
        fn new(rng: &mut Pcg32, id: u8) -> BusyShip {
            BusyShip {
                ship: NetShip {
                    id,
                    pos: Vec3::new(
                        rng.range_f32(20.0, 236.0),
                        rng.range_f32(20.0, 108.0),
                        rng.range_f32(20.0, 236.0),
                    ),
                    vel: Vec3::new(
                        rng.range_f32(-12.0, 12.0),
                        rng.range_f32(-12.0, 12.0),
                        rng.range_f32(-12.0, 12.0),
                    ),
                    rot: Quat::from_axis_angle(
                        Vec3::new(
                            rng.range_f32(-1.0, 1.0),
                            rng.range_f32(-1.0, 1.0),
                            rng.range_f32(0.1, 1.0),
                        )
                        .normalize(),
                        rng.range_f32(-3.0, 3.0),
                    ),
                    flags: ship_flags::with_team(ship_flags::ALIVE, id % 2),
                    status: None,
                },
                thrust: Vec3::ZERO,
                omega: Vec3::ZERO,
                accel: rng.range_f32(24.0, 30.0),
                hold: 0,
            }
        }

        fn tick(&mut self, rng: &mut Pcg32, turn: f32) {
            const DT: f32 = 1.0 / 60.0;
            if self.hold == 0 {
                self.hold = 15 + rng.below(16);
                self.thrust = Vec3::new(
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, -0.2),
                );
                self.omega = Vec3::new(
                    rng.range_f32(-turn, turn),
                    rng.range_f32(-turn, turn),
                    rng.range_f32(-2.0, 2.0),
                );
            }
            self.hold -= 1;
            let s = &mut self.ship;
            s.rot = (s.rot * Quat::from_scaled_axis(self.omega * DT)).normalize();
            s.vel += (s.rot * self.thrust) * (self.accel * DT);
            s.vel = s.vel.clamp(Vec3::splat(-17.0), Vec3::splat(17.0));
            s.pos += s.vel * DT;
            let (lo, hi) = ([2.0, 2.0, 2.0], [254.0, 126.0, 254.0]);
            let (mut p, mut v) = ([s.pos.x, s.pos.y, s.pos.z], [s.vel.x, s.vel.y, s.vel.z]);
            for i in 0..3 {
                if p[i] < lo[i] || p[i] > hi[i] {
                    v[i] = -v[i] * 0.2;
                    p[i] = p[i].clamp(lo[i], hi[i]);
                }
            }
            s.pos = Vec3::new(p[0], p[1], p[2]);
            s.vel = Vec3::new(v[0], v[1], v[2]);
        }
    }

    /// Runs 32 busy ships for `seconds`, snapshots every 2 ticks (30 Hz),
    /// and returns (mean, max) bytes of the ship section of a delta against
    /// the snapshot `age` ticks older. The ship section is measured as a
    /// snapshot with nothing but ships minus the same snapshot without them.
    fn busy_ship_deltas(seed: u64, turn: f32, age: u32, seconds: u32) -> (f64, usize) {
        let quant = Quantizer::default();
        let mut rng = Pcg32::new(seed, 0);
        let mut ships: Vec<BusyShip> = (0..MAX_SHIPS)
            .map(|id| BusyShip::new(&mut rng, id))
            .collect();
        let mut history: Vec<PackedSnapshot> = Vec::new();
        let (mut total, mut worst, mut n) = (0usize, 0usize, 0usize);
        for tick in 1..=seconds * 60 {
            for s in &mut ships {
                s.tick(&mut rng, turn);
            }
            if tick % 2 != 0 {
                continue;
            }
            let snap = quant.pack_snapshot(&Snapshot {
                tick,
                ack_input: tick,
                ships: ships.iter().map(|s| s.ship).collect(),
                ..Default::default()
            });
            if let Some(base) = history.iter().find(|b| b.tick + age == tick) {
                let bytes = encode(&snap, Some(base));
                assert_eq!(decode(&bytes, base).0, snap);
                let empty = {
                    let (mut b, mut s) = (base.clone(), snap.clone());
                    b.ships.clear();
                    s.ships.clear();
                    encode(&s, Some(&b)).len()
                };
                let ship_bytes = bytes.len() - empty;
                total += ship_bytes;
                worst = worst.max(ship_bytes);
                n += 1;
            }
            history.push(snap);
        }
        (total as f64 / n as f64, worst)
    }

    /// Delta snapshots with 32 busy ships (every ship thrusting and turning,
    /// see [`BusyShip`]), debris at its cap (64 pieces) all drifting after an
    /// explosion, 40 pickups and 8 wrecks: one datagram and within the 17
    /// KB/s budget at 30 Hz for baselines up to 6 ticks old (100 ms round
    /// trip). Resting items cost one bit each.
    #[test]
    fn bandwidth_32_ships_64_debris() {
        // 32 ships x 18 B x 30 Hz (the documented budget).
        const BUDGET: usize = 17_280;
        let quant = Quantizer::default();
        let mut rng = Pcg32::new(29, 0);
        let players: Vec<NetPlayer> = (0..MAX_SHIPS)
            .map(|id| NetPlayer {
                id,
                team: id % 2,
                ship_type: id % 4,
                gadget: 255,
                name: format!("player{id}"),
            })
            .collect();
        let mut ships: Vec<BusyShip> = (0..MAX_SHIPS)
            .map(|id| BusyShip::new(&mut rng, id))
            .collect();
        let centre = Vec3::new(128.0, 64.0, 128.0);
        // Debris: sent without velocity (clients interpolate positions).
        let mut debris: Vec<(NetEntity, Vec3)> = (0..64u16)
            .map(|i| {
                let dir = Vec3::new(
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                    rng.range_f32(-1.0, 1.0),
                )
                .normalize_or_zero();
                let e = NetEntity {
                    id: 100 + i * 3,
                    kind: 0,
                    visual: 0,
                    owner: 255,
                    team: 255,
                    pos: centre + dir * rng.range_f32(1.0, 8.0),
                    vel: Vec3::ZERO,
                    rot: Quat::IDENTITY,
                    state: [3, 0, 8, 0],
                };
                (e, dir * rng.range_f32(0.5, 6.0))
            })
            .collect();
        let fixed: Vec<NetEntity> = (0..48u16)
            .map(|i| NetEntity {
                id: 400 + i,
                kind: if i < 40 { 1 } else { 5 },
                visual: 12 + i % 20,
                owner: 255,
                team: 255,
                pos: Vec3::new(5.0 * i as f32, 30.0, 200.0),
                vel: Vec3::ZERO,
                rot: Quat::IDENTITY,
                state: [i as u8 % 27, 10, 0, 0],
            })
            .collect();
        let snapshot = |tick: u32, ships: &[BusyShip], debris: &[(NetEntity, Vec3)]| {
            let mut entities: Vec<NetEntity> = debris.iter().map(|(e, _)| *e).collect();
            entities.extend(fixed.iter().copied());
            quant.pack_snapshot(&Snapshot {
                tick,
                ack_input: tick + 4,
                ships: ships.iter().map(|s| s.ship).collect(),
                entities,
                players: players.clone(),
                scores: vec![31_000, 29_500],
                match_ticks_left: 9_000,
                ..Default::default()
            })
        };
        // Warm up so every ship is mid-manoeuvre.
        for _ in 0..30 {
            for s in &mut ships {
                s.tick(&mut rng, 4.0);
            }
        }
        let mut history = vec![snapshot(1000, &ships, &debris)];
        let mut worst = 0usize;
        for step in 1..=3u32 {
            for _ in 0..2 {
                for s in &mut ships {
                    s.tick(&mut rng, 4.0);
                }
                for (e, v) in &mut debris {
                    e.pos += *v * (1.0 / 60.0);
                }
            }
            history.push(snapshot(1000 + 2 * step, &ships, &debris));
        }
        let next = history.last().unwrap();
        for base in &history[..3] {
            let age = next.tick - base.tick;
            let delta = encode(next, Some(base));
            let (back, _) = decode(&delta, base);
            assert_eq!(&back, next);
            // The entity part: the same snapshots without entities.
            let ships_only = {
                let (mut b, mut n) = (base.clone(), next.clone());
                b.entities.clear();
                n.entities.clear();
                encode(&n, Some(&b)).len()
            };
            let entity_bytes = delta.len() - ships_only;
            println!(
                "32 busy ships + 64 moving debris + 48 resting items, baseline {age} ticks old: delta {} B ({} B/s at 30 Hz; ships and header {ships_only} B, entities {entity_bytes} B), full {} B",
                delta.len(),
                delta.len() * 30,
                encode(next, None).len()
            );
            assert!(delta.len() <= 1200, "fits one datagram: {} B", delta.len());
            worst = worst.max(delta.len());
        }
        assert!(worst * 30 <= BUDGET, "busy delta {worst} B per snapshot");
        // Nothing moved: the whole entity list costs a bit per item.
        let same = snapshot(2000, &ships, &debris);
        let mut later = same.clone();
        later.tick = 2002;
        let idle = encode(&later, Some(&same));
        assert!(idle.len() < 60, "unchanged snapshot {} B", idle.len());
    }

    /// Busy ship deltas over several seconds and baseline ages. M1's
    /// encoding (raw rotations, unpredicted 4/8/16-bit diffs) needed about
    /// 515 B per snapshot for the ship section at age 2; the predictive
    /// encoding must leave room for the entities (about 160 B) and the
    /// header in the 576 B a 17,280 B/s budget allows per 30 Hz snapshot.
    #[test]
    fn busy_ship_deltas_fit_the_budget() {
        const PER_SNAPSHOT: f64 = 17_280.0 / 30.0;
        for (age, turn, limit) in [
            (2, 4.0, 315.0),
            (4, 4.0, 350.0),
            (6, 4.0, 375.0),
            (2, 8.0, 330.0),
        ] {
            let (mean, worst) = busy_ship_deltas(31 + age as u64, turn, age, 4);
            println!(
                "32 busy ships, turning {turn} rad/s, baseline {age} ticks old: ship section mean {mean:.0} B, worst {worst} B (budget {PER_SNAPSHOT:.0} B per snapshot)"
            );
            assert!(mean <= limit, "age {age}: mean {mean:.0} B");
            assert!(
                (worst as f64) < PER_SNAPSHOT - 160.0,
                "age {age}: worst {worst} B"
            );
        }
    }

    /// Bits one changed ship adds to a delta (the ship section's mask and
    /// typical widths are there without ships too).
    fn one_ship_delta_bits(base: PackedShip, cur: PackedShip, age: u32) -> usize {
        let motion = Quantizer::default().ship_motion_scale();
        let snap = |tick: u32, ship: Option<PackedShip>| PackedSnapshot {
            tick,
            ack_input: tick,
            ships: ship.into_iter().collect(),
            ship_motion: motion,
            ..Default::default()
        };
        let (b, n) = (snap(100, Some(base)), snap(100 + age, Some(cur)));
        let bits = |b: &PackedSnapshot, n: &PackedSnapshot| {
            let mut w = BitWriter::new();
            write_snapshot(&mut w, n, Some(b)).unwrap();
            w.bit_len()
        };
        let with = bits(&b, &n);
        let bytes = encode(&n, Some(&b));
        assert_eq!(decode(&bytes, &b).0, n, "exact round trip");
        with - bits(&snap(100, None), &snap(100 + age, None))
    }

    /// The motion scale matches the quantizer: a ship cruising at constant
    /// velocity lands within a position step of the ballistic prediction,
    /// and its delta (new position only) costs a few bits.
    #[test]
    fn cruising_ship_is_predicted() {
        let quant = Quantizer::default();
        let motion = quant.ship_motion_scale();
        assert!(motion.iter().all(|&k| k > 0));
        let mut rng = Pcg32::new(41, 0);
        let mut worst_bits = 0;
        for _ in 0..200 {
            let v = Vec3::new(
                rng.range_f32(-17.0, 17.0),
                rng.range_f32(-17.0, 17.0),
                rng.range_f32(-17.0, 17.0),
            );
            let start = Vec3::new(
                rng.range_f32(40.0, 216.0),
                rng.range_f32(30.0, 100.0),
                rng.range_f32(40.0, 216.0),
            );
            let age = 1 + rng.below(8);
            let ship = |pos: Vec3| {
                quant.pack_ship(&NetShip {
                    id: 3,
                    pos,
                    vel: v,
                    rot: Quat::IDENTITY,
                    flags: ship_flags::ALIVE,
                    status: None,
                })
            };
            let base = ship(start);
            let cur = ship(start + v * (age as f32 / 60.0));
            let pred = predict_pos(&base, &motion, age);
            for (i, (&c, &p)) in cur.pos.iter().zip(&pred).enumerate() {
                let err = (c as i32 - p as i32).abs();
                assert!(err <= 1, "axis {i}: {err} steps off at age {age}");
            }
            // 1 changed + position (at most 1 + 8 + 3 x 2) + velocity 2 +
            // orientation, flags, status 3; 18 raw bytes as a full ship.
            let bits = one_ship_delta_bits(base, cur, age);
            assert!(bits <= 1 + 15 + 2 + 3, "{bits} bits");
            worst_bits = worst_bits.max(bits);
        }
        println!("cruising ship: worst {worst_bits} bits per delta");
    }

    /// Accelerating: the displacement predicts the new velocity to within
    /// the position quantization, far closer than the baseline velocity.
    #[test]
    fn accelerating_ship_velocity_is_predicted_from_motion() {
        let quant = Quantizer::default();
        let motion = quant.ship_motion_scale();
        let (v0, a) = (Vec3::new(5.0, -3.0, 9.0), Vec3::new(20.0, 12.0, -25.0));
        let age = 6;
        let t = age as f32 / 60.0;
        let p0 = Vec3::new(100.0, 60.0, 90.0);
        let ship = |pos: Vec3, vel: Vec3| {
            quant.pack_ship(&NetShip {
                id: 0,
                pos,
                vel,
                rot: Quat::IDENTITY,
                flags: ship_flags::ALIVE,
                status: None,
            })
        };
        let base = ship(p0, v0);
        let cur = ship(p0 + v0 * t + a * (0.5 * t * t), v0 + a * t);
        let d = ShipDelta::new(&cur, &base, &motion, age);
        assert!(d.vel_from_motion);
        let naive = resid_width(residual(cur.vel, base.vel));
        assert!(resid_width(d.vel) + 3 <= naive, "{d:?} vs {naive} bits");
        assert!(resid_width(d.pos) <= 6, "{d:?}");
        one_ship_delta_bits(base, cur, age);
    }

    /// Extreme packed values, ages and motion scales round-trip exactly
    /// (wrapping residuals, clamped predictions, dropped-component switches).
    #[test]
    fn ship_delta_extremes_round_trip() {
        let mut rng = Pcg32::new(42, 0);
        let edge = |rng: &mut Pcg32| match rng.below(5) {
            0 => 0u16,
            1 => u16::MAX,
            2 => 0x8001, // -32767
            3 => 0x7FFF,
            _ => rng.next_u32() as u16,
        };
        for _ in 0..3000 {
            let motion = match rng.below(3) {
                0 => [u32::MAX; 3],
                1 => Quantizer::default().ship_motion_scale(),
                _ => [0, rng.next_u32(), 1],
            };
            let mut ship = || PackedShip {
                id: 7,
                pos: [edge(&mut rng), edge(&mut rng), edge(&mut rng)],
                vel: [edge(&mut rng), edge(&mut rng), edge(&mut rng)],
                rot: if rng.chance(0.5) {
                    (rng.below(4) << 30) | (1022 << 20) | rng.below(1023)
                } else {
                    rng.next_u32() & !(0x3FF << 10) // field 1 = 0
                },
                flags: rng.next_u32() as u16 & !ship_flags::WIRE_RESERVED,
                status: None,
            };
            let (a, b) = (ship(), ship());
            let age = 1 + rng.below(MAX_BASELINE_AGE);
            let base = PackedSnapshot {
                tick: 500,
                ships: vec![a],
                ship_motion: motion,
                ..Default::default()
            };
            let next = PackedSnapshot {
                tick: 500 + age,
                ships: vec![b],
                ship_motion: motion,
                ..Default::default()
            };
            let bytes = encode(&next, Some(&base));
            assert_eq!(decode(&bytes, &base).0, next);
        }
    }

    /// Hostile deltas: an orientation field pushed out of its 10-bit range
    /// is an error, not a wrapped quaternion.
    #[test]
    fn rotation_diff_out_of_range_is_rejected() {
        let base_ship = PackedShip {
            id: 0,
            pos: [100; 3],
            vel: [0; 3],
            rot: (3 << 30) | (1000 << 20) | (511 << 10) | 511,
            flags: 0,
            status: None,
        };
        let base = PackedSnapshot {
            tick: 10,
            ships: vec![base_ship],
            ..Default::default()
        };
        // Hand-built delta: header, mask, widths, ship with field 0 + 40.
        let mut w = BitWriter::new();
        w.write_u32(12);
        w.write_bool(true);
        w.write_u8(2);
        w.write_varint(zigzag32(-12) as u64);
        w.write_varint(0);
        w.write_bool(false); // motion unchanged
        w.write_varint(0);
        w.write_varint(0);
        w.write_bool(false); // scores unchanged
        w.write_u32(1);
        WidthBase([1, 1, 7]).write(&mut w);
        w.write_bool(true); // changed
        w.write_bool(false); // position residual zero
        w.write_bool(false); // velocity predictor
        w.write_bool(false); // velocity residual zero
        w.write_bool(true); // orientation changed
        w.write_bool(true); // same dropped component
        write_resid3_nz(&mut w, [40, 0, 0], 7);
        let bytes = w.finish();
        assert_eq!(
            read_snapshot(&mut BitReader::new(&bytes), &base).unwrap_err(),
            DecodeError::Invalid("rotation diff out of range")
        );
    }

    /// The motion scale travels in full snapshots and only when it changes
    /// in deltas; a decoded snapshot carries it for later deltas.
    #[test]
    fn motion_scale_is_carried_and_inherited() {
        let quant = Quantizer::default();
        let empty = quant.pack_snapshot(&Snapshot {
            tick: 50,
            ..Default::default()
        });
        assert_eq!(empty.projectile_motion, [0; 3], "no projectiles, no scale");
        let full = quant.pack_snapshot(&Snapshot {
            tick: 50,
            projectiles: vec![NetProjectile {
                id: 3,
                kind: 1,
                owner: 0,
                pos: Vec3::splat(10.0),
                vel: Vec3::X * 30.0,
                state: 0,
            }],
            ..Default::default()
        });
        assert_eq!(full.ship_motion, quant.ship_motion_scale());
        assert_eq!(full.projectile_motion, quant.projectile_motion_scale());
        assert_ne!(full.projectile_motion, full.ship_motion);
        let (back, _) = decode(&encode(&full, None), &NoBaselines);
        assert_eq!(back.ship_motion, full.ship_motion);
        assert_eq!(back.projectile_motion, full.projectile_motion);
        let mut next = full.clone();
        next.tick = 52;
        let same = encode(&next, Some(&full));
        next.ship_motion = [1, 2, 3];
        let changed = encode(&next, Some(&full));
        assert!(changed.len() > same.len());
        assert_eq!(decode(&changed, &full).0.ship_motion, [1, 2, 3]);
        next.projectile_motion = [4, 5, 6];
        let changed = encode(&next, Some(&full));
        assert_eq!(decode(&changed, &full).0.projectile_motion, [4, 5, 6]);
    }

    #[test]
    fn history_ring_and_baseline_choice() {
        let mut h = SnapshotHistory::new(4);
        for t in [10u32, 12, 14, 16, 18] {
            h.push(PackedSnapshot {
                tick: t,
                ..Default::default()
            });
        }
        assert_eq!(h.len(), 4);
        assert!(h.get(10).is_none(), "oldest evicted");
        assert_eq!(h.latest().unwrap().tick, 18);
        assert_eq!(h.choose_baseline(20, Some(16)), Some(16));
        assert_eq!(h.choose_baseline(20, Some(10)), None, "evicted");
        assert_eq!(h.choose_baseline(20, None), None);
        assert_eq!(h.choose_baseline(16, Some(16)), None, "same tick");
        assert_eq!(h.choose_baseline(18 + 256, Some(18)), None, "too old");
        h.push(PackedSnapshot {
            tick: 18,
            scores: vec![1],
            ..Default::default()
        });
        assert_eq!(h.len(), 4);
        assert_eq!(h.get(18).unwrap().scores, vec![1]);
    }

    /// A weapon of the busy-fight model: `(speed u/s, ticks between shots,
    /// bolts per shot, fan degrees, lifetime ticks)`, from `data/weapons`.
    const BUSY_WEAPONS: [(f32, u32, u32, f32, u32); 6] = [
        (30.0, 15, 1, 0.0, 180), // Scrap Laser
        (180.0, 6, 1, 0.0, 60),  // Rivet Gun
        (33.0, 6, 1, 0.0, 90),   // Plasma Repeater
        (30.0, 18, 3, 8.0, 180), // Scatter Laser
        (25.0, 45, 1, 0.0, 360), // Seeker (homing, 2.5 rad/s)
        (5.0, 30, 1, 0.0, 3600), // Prox Mine (sticks after 1 s)
    ];

    /// A projectile of the busy-fight model.
    struct BusyShot {
        p: NetProjectile,
        weapon: usize,
        /// Distance left until it hits something (wall or ship).
        reach: f32,
        age: u32,
    }

    /// A busy fight for the bandwidth tests: `ships` [`BusyShip`]s, each
    /// with one weapon of [`BUSY_WEAPONS`] (by id), holding the trigger in
    /// bursts (1 to 2 s on, 0.5 to 1.5 s off). Bolts fly straight from the
    /// nose (no inherited velocity) until they hit something 4 to 60 u away
    /// or their lifetime ends; Seekers home in on the nearest enemy at 2.5
    /// rad/s; mines slow to a stop after 1 s and stay stuck (3 per owner).
    /// Returns the snapshot at every even tick for `seconds`.
    fn busy_fight(seed: u64, ships: u8, seconds: u32) -> Vec<PackedSnapshot> {
        const DT: f32 = 1.0 / 60.0;
        let quant = Quantizer::default();
        let mut rng = Pcg32::new(seed, 0);
        let players: Vec<NetPlayer> = (0..ships)
            .map(|id| NetPlayer {
                id,
                team: id % 2,
                ship_type: id % 4,
                gadget: 255,
                name: format!("player{id}"),
            })
            .collect();
        let mut fleet: Vec<BusyShip> = (0..ships).map(|id| BusyShip::new(&mut rng, id)).collect();
        // (trigger down, ticks left in this phase, next shot tick)
        let mut triggers: Vec<(bool, u32, u32)> = (0..ships)
            .map(|_| (rng.chance(0.5), rng.below(90), 0))
            .collect();
        let mut shots: Vec<BusyShot> = Vec::new();
        let mut next_id = 1u16;
        let mut out = Vec::new();
        for tick in 1..=seconds * 60 + 60 {
            for s in &mut fleet {
                s.tick(&mut rng, 4.0);
            }
            // Fire.
            for (i, s) in fleet.iter().enumerate() {
                let t = &mut triggers[i];
                if t.1 == 0 {
                    t.0 = !t.0;
                    t.1 = if t.0 {
                        60 + rng.below(61)
                    } else {
                        30 + rng.below(61)
                    };
                }
                t.1 -= 1;
                let w = i % BUSY_WEAPONS.len();
                let (speed, interval, bolts, fan, _) = BUSY_WEAPONS[w];
                if !t.0 || tick < t.2 {
                    continue;
                }
                t.2 = tick + interval;
                let fwd = s.ship.rot * Vec3::NEG_Z;
                let right = s.ship.rot * Vec3::X;
                for b in 0..bolts {
                    let off = (b as f32 - (bolts - 1) as f32 * 0.5) * fan.to_radians();
                    let dir = (fwd + right * off).normalize();
                    shots.push(BusyShot {
                        p: NetProjectile {
                            id: next_id,
                            kind: w as u8 * 3,
                            owner: s.ship.id,
                            pos: s.ship.pos + dir * 0.7,
                            vel: dir * speed,
                            state: 0,
                        },
                        weapon: w,
                        reach: rng.range_f32(4.0, 60.0),
                        age: 0,
                    });
                    next_id = next_id.wrapping_add(1).max(1);
                }
            }
            // Move, home, stick and end.
            for sh in &mut shots {
                sh.age += 1;
                match sh.weapon {
                    4 => {
                        let target = fleet
                            .iter()
                            .filter(|s| s.ship.id % 2 != sh.p.owner % 2)
                            .min_by(|a, b| {
                                let da = a.ship.pos.distance_squared(sh.p.pos);
                                let db = b.ship.pos.distance_squared(sh.p.pos);
                                da.total_cmp(&db)
                            })
                            .map(|s| s.ship.pos);
                        if let Some(t) = target {
                            let want = (t - sh.p.pos).normalize_or_zero();
                            let cur = sh.p.vel.normalize_or_zero();
                            let turn = (want - cur) * (2.5 * DT);
                            sh.p.vel = (cur + turn).normalize_or_zero() * 25.0;
                        }
                    }
                    5 if sh.age == 60 => {
                        sh.p.vel = Vec3::ZERO;
                        sh.p.state = 1; // stuck
                    }
                    _ => {}
                }
                sh.p.pos += sh.p.vel * DT;
                sh.reach -= sh.p.vel.length() * DT;
            }
            shots.retain(|sh| {
                sh.age < BUSY_WEAPONS[sh.weapon].4 && (sh.weapon == 5 || sh.reach > 0.0)
            });
            // At most 3 mines per owner (the oldest goes).
            for owner in 0..ships {
                let mines: Vec<u16> = shots
                    .iter()
                    .filter(|sh| sh.weapon == 5 && sh.p.owner == owner)
                    .map(|sh| sh.p.id)
                    .collect();
                if mines.len() > 3 {
                    let oldest = mines[0];
                    shots.retain(|sh| sh.p.id != oldest);
                }
            }
            if tick % 2 != 0 || tick <= 60 {
                continue;
            }
            out.push(quant.pack_snapshot(&Snapshot {
                tick,
                ack_input: tick + 4,
                ships: fleet.iter().map(|s| s.ship).collect(),
                projectiles: shots.iter().map(|sh| sh.p).collect(),
                players: players.clone(),
                scores: vec![31_000, 29_500],
                match_ticks_left: 9_000,
                ..Default::default()
            }));
        }
        out
    }

    /// Bits of a projectile section as protocol 3 sent it (unpredicted
    /// 4/8/16-bit component diffs against the baseline), for comparison.
    fn protocol3_projectile_bytes(cur: &[PackedProjectile], base: &[PackedProjectile]) -> usize {
        let mut w = BitWriter::new();
        for b in base {
            match cur.binary_search_by_key(&b.id, |c| c.id) {
                Ok(i) if cur[i] == *b => w.write_bool(true),
                Ok(i) => {
                    let p = &cur[i];
                    w.write_bits(1, 2);
                    let meta = p.kind != b.kind || p.owner != b.owner || p.state != b.state;
                    w.write_bool(meta);
                    if meta {
                        w.write_bits(0, 24);
                    }
                    write_vec_diff(&mut w, p.pos, b.pos);
                    write_vec_diff(&mut w, p.vel, b.vel);
                }
                Err(_) => w.write_bits(0, 2),
            }
        }
        let new: Vec<&PackedProjectile> = cur
            .iter()
            .filter(|c| base.binary_search_by_key(&c.id, |b| b.id).is_err())
            .collect();
        write_full_list(&mut w, new.into_iter(), write_projectile_full);
        w.finish().len()
    }

    /// Protocol 4 acceptance: a busy 32-player fight (every ship dogfighting,
    /// see [`BusyShip`], and firing in bursts with a mix of fast, slow,
    /// fanned, homing and stuck projectiles, see [`busy_fight`]: about 100
    /// projectiles in flight, 4 new ones per snapshot), sent the way the
    /// server sends it (settled against the baseline, see
    /// [`settle_projectiles`]), over four seeds. Every delta is one
    /// datagram; on average the fight stays within 17,280 B/s at 30 Hz for
    /// baselines up to 6 ticks old (100 ms round trip). The busiest
    /// snapshots fit at 2 ticks, go up to about 6% over at 4 ticks and up to
    /// about 19% over at 6 ticks (worst 683 B against 576 B). New
    /// projectiles dominate the rest: a baseline 6 ticks
    /// old repeats about 12 of them at about 11 bytes each (velocity and a
    /// position against the owner's ship), while a projectile in flight
    /// mostly costs a bit. Protocol 3 needed 610 to 760 B for the
    /// projectiles alone (now 100 to 190 B), 27 to 33 KB/s in all.
    #[test]
    fn busy_fight_with_projectiles_fits_the_budget() {
        // Several fights, so the bounds are not tuned to one seed.
        for seed in [41, 7, 1234, 99_991] {
            busy_fight_budget(seed);
        }
    }

    fn busy_fight_budget(seed: u64) {
        const PER_SNAPSHOT: usize = 17_280 / 30;
        let snaps = busy_fight(seed, MAX_SHIPS, 4);
        let (mut flying, mut spawned, mut n) = (0usize, 0usize, 0usize);
        for (i, s) in snaps.iter().enumerate().skip(1) {
            flying += s.projectiles.len();
            spawned += s
                .projectiles
                .iter()
                .filter(|p| snaps[i - 1].projectile(p.id).is_none())
                .count();
            n += 1;
        }
        let (flying, spawned) = (flying as f64 / n as f64, spawned as f64 / n as f64);
        assert!(
            (80.0..=160.0).contains(&flying) && spawned > 3.0,
            "{flying:.0} projectiles in flight, {spawned:.1} new"
        );
        for (age, worst_limit, proj_limit) in [
            // Busiest snapshots: within budget at 2 ticks, a few percent
            // over at 4, up to about 25% over at 6 (a known shortfall, see
            // docs/architecture/netcode.md); the mean fits at every age.
            (2u32, PER_SNAPSHOT, 120.0),
            (4, 620, 170.0),
            (6, 720, 210.0),
        ] {
            let step = (age / 2) as usize;
            let (mut total, mut worst, mut proj, mut old, mut count) = (0, 0, 0, 0, 0);
            // What the server sent (settled): the client's baselines.
            let mut sent: Vec<PackedSnapshot> = snaps[..step].to_vec();
            for i in step..snaps.len() {
                let base = &sent[i - step];
                let mut next = snaps[i].clone();
                settle_projectiles(&mut next, base, SETTLE_TOLERANCE);
                for (p, truth) in next.projectiles.iter().zip(&snaps[i].projectiles) {
                    let off = residual(p.pos, truth.pos);
                    assert!(
                        off.iter().all(|c| c.unsigned_abs() <= SETTLE_TOLERANCE),
                        "{off:?}"
                    );
                }
                let delta = encode(&next, Some(base));
                assert_eq!(decode(&delta, base).0, next);
                let without = {
                    let (mut b, mut s) = (base.clone(), next.clone());
                    b.projectiles.clear();
                    s.projectiles.clear();
                    b.projectile_motion = s.projectile_motion;
                    encode(&s, Some(&b)).len()
                };
                total += delta.len();
                worst = worst.max(delta.len());
                proj += delta.len() - without;
                old +=
                    protocol3_projectile_bytes(&snaps[i].projectiles, &snaps[i - step].projectiles);
                count += 1;
                sent.push(next);
            }
            let mean = total as f64 / count as f64;
            let (proj, old) = (proj as f64 / count as f64, old as f64 / count as f64);
            println!(
                "seed {seed}: busy fight, 32 ships, {flying:.0} projectiles in flight, {spawned:.1} new per snapshot, baseline {age} ticks old: delta mean {mean:.0} B ({:.0} B/s), worst {worst} B ({} B/s), projectile section mean {proj:.0} B (protocol 3: {old:.0} B)",
                mean * 30.0,
                worst * 30,
            );
            assert!(worst <= 1200, "one datagram: {worst} B");
            assert!(mean <= PER_SNAPSHOT as f64, "age {age}: mean {mean:.0} B");
            assert!(worst <= worst_limit, "age {age}: worst {worst} B");
            assert!(
                proj <= proj_limit && proj < old * 0.35,
                "age {age}: {proj:.0} B vs {old:.0} B"
            );
        }
    }

    /// Straight projectiles are predicted: a bolt that flew exactly along
    /// its packed velocity costs one bit, a homing one only its residuals.
    #[test]
    fn straight_projectiles_are_predicted() {
        let quant = Quantizer::default();
        let bolt = |id, pos: Vec3, vel: Vec3| NetProjectile {
            id,
            kind: 2,
            owner: 5,
            pos,
            vel,
            state: 0,
        };
        let snap = |tick, projectiles| {
            quant.pack_snapshot(&Snapshot {
                tick,
                ack_input: tick,
                projectiles,
                ..Default::default()
            })
        };
        let p0 = Vec3::new(100.0, 50.0, 80.0);
        let v = Vec3::new(170.0, -40.0, 25.0);
        let base = snap(100, vec![bolt(7, p0, v)]);
        // Exactly on the packed prediction: one bit.
        let pb = base.projectiles[0];
        let mut exact = base.clone();
        exact.tick = 104;
        exact.projectiles[0] = predict_projectile(&pb, &base.projectile_motion, 4);
        let same = {
            let mut e = exact.clone();
            e.projectiles.clear();
            e.projectile_motion = base.projectile_motion;
            let mut b = base.clone();
            b.projectiles.clear();
            encode(&e, Some(&b)).len()
        };
        let bytes = encode(&exact, Some(&base));
        assert_eq!(decode(&bytes, &base).0, exact);
        assert!(bytes.len() <= same + 1, "{} vs {same}", bytes.len());
        // The real flight (float positions) is within a step or two.
        let flown = snap(104, vec![bolt(7, p0 + v * (4.0 / 60.0), v)]);
        let d = ProjectileDelta::new(
            &flown.projectiles[0],
            &predict_projectile(&pb, &base.projectile_motion, 4),
        );
        assert!(resid_width(d.pos) <= 2 && d.vel == [0; 3], "{d:?}");
        let bytes = encode(&flown, Some(&base));
        assert_eq!(decode(&bytes, &base).0, flown);
        assert!(bytes.len() <= same + 2, "{} vs {same}", bytes.len());
        // A turning (homing) projectile sends its velocity change too, and
        // still decodes exactly; so does a stuck one (zero velocity).
        let turned = snap(
            104,
            vec![bolt(7, p0 + v * (4.0 / 60.0), v * 0.9 + Vec3::Y * 20.0)],
        );
        let bytes = encode(&turned, Some(&base));
        assert_eq!(decode(&bytes, &base).0, turned);
        let stuck_base = snap(100, vec![bolt(9, p0, Vec3::ZERO)]);
        let mut stuck = stuck_base.clone();
        stuck.tick = 106;
        let bytes = encode(&stuck, Some(&stuck_base));
        assert_eq!(decode(&bytes, &stuck_base).0, stuck);
    }
}
