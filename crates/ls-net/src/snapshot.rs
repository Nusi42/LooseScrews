//! Snapshot encoding, full and delta-compressed, plus baseline history.
//!
//! # Layout
//!
//! ```text
//! tick                u32
//! has_baseline        1 bit, then (tick - baseline_tick) as u8 (1..=255)
//! ack_input           zigzag varint of (ack_input - tick) as i32
//! match_ticks_left    varint
//! edits               varint edit_base, varint count, then each edit
//! scores              [delta: 1 bit "changed"] varint count + varint each
//! ships               u32 presence mask (bit i = ship id i), then per ship
//!                     in id order: full core state, or (delta, ship in
//!                     baseline) a change bit + per-field diffs
//! projectiles         id list (below) of projectiles: full state (kind,
//!                     owner, state byte, position, velocity) or diffs
//! entities            id list of entities: full state (kind, visual, owner,
//!                     team, position, velocity and rotation with presence
//!                     bits, 4 state bytes) or diffs
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
//! exactly the packed snapshot the server encoded. Vector components are
//! sent as a 2-bit size class: unchanged, 4-bit or 8-bit zigzag difference,
//! or the raw 16-bit value.

use std::collections::VecDeque;

use crate::bits::{unzigzag16, zigzag16, BitReader, BitWriter};
use crate::error::{DecodeError, EncodeError};
use crate::quant::pack_quat;
use crate::quant::PackedVec3;
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

fn write_ship_delta(w: &mut BitWriter, s: &PackedShip, b: &PackedShip) {
    let changed = s != b;
    w.write_bool(changed);
    if !changed {
        return;
    }
    write_vec_diff(w, s.pos, b.pos);
    w.write_bool(s.rot != b.rot);
    if s.rot != b.rot {
        w.write_u32(s.rot);
    }
    write_vec_diff(w, s.vel, b.vel);
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

fn read_ship_delta(r: &mut BitReader, b: &PackedShip) -> Result<PackedShip, DecodeError> {
    if !r.read_bool()? {
        return Ok(*b);
    }
    let pos = read_vec_diff(r, b.pos)?;
    let rot = if r.read_bool()? { r.read_u32()? } else { b.rot };
    let vel = read_vec_diff(r, b.vel)?;
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

/// Diffs of a changed projectile against its baseline state.
fn write_projectile_delta(w: &mut BitWriter, p: &PackedProjectile, b: &PackedProjectile) {
    let meta = p.kind != b.kind || p.owner != b.owner || p.state != b.state;
    w.write_bool(meta);
    if meta {
        w.write_u8(p.kind);
        w.write_u8(p.owner);
        w.write_u8(p.state);
    }
    write_vec_diff(w, p.pos, b.pos);
    write_vec_diff(w, p.vel, b.vel);
}

fn read_projectile_delta(
    r: &mut BitReader,
    b: &PackedProjectile,
) -> Result<PackedProjectile, DecodeError> {
    let (kind, owner, state) = if r.read_bool()? {
        (r.read_u8()?, r.read_u8()?, r.read_u8()?)
    } else {
        (b.kind, b.owner, b.state)
    };
    Ok(PackedProjectile {
        id: b.id,
        kind,
        owner,
        state,
        pos: read_vec_diff(r, b.pos)?,
        vel: read_vec_diff(r, b.vel)?,
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

/// Diffs of a changed entity against its baseline state: 1 bit "only the
/// position moved" (debris drifting: three component diffs), else every
/// field group with a change bit.
fn write_entity_delta(w: &mut BitWriter, e: &PackedEntity, b: &PackedEntity) {
    let pos_only = PackedEntity { pos: e.pos, ..*b } == *e;
    w.write_bool(pos_only);
    if pos_only {
        for i in 0..3 {
            write_comp_diff(w, e.pos[i], b.pos[i]);
        }
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

fn read_entity_delta(r: &mut BitReader, b: &PackedEntity) -> Result<PackedEntity, DecodeError> {
    if r.read_bool()? {
        let pos = [
            read_comp_diff(r, b.pos[0])?,
            read_comp_diff(r, b.pos[1])?,
            read_comp_diff(r, b.pos[2])?,
        ];
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

/// Writes `cur` (sorted, unique ids) as an id list, against `base` if given.
fn write_list<T: ListItem>(
    w: &mut BitWriter,
    cur: &[T],
    base: Option<&[T]>,
    full: fn(&mut BitWriter, &T),
    delta: fn(&mut BitWriter, &T, &T),
) {
    let Some(base) = base else {
        write_full_list(w, cur.iter(), full);
        return;
    };
    for b in base {
        match cur.binary_search_by_key(&b.id(), |c| c.id()) {
            Ok(i) if cur[i] == *b => w.write_bool(true),
            Ok(i) => {
                w.write_bool(false);
                w.write_bool(true);
                delta(w, &cur[i], b);
            }
            Err(_) => {
                w.write_bool(false);
                w.write_bool(false);
            }
        }
    }
    let new: Vec<&T> = cur
        .iter()
        .filter(|c| base.binary_search_by_key(&c.id(), |b| b.id()).is_err())
        .collect();
    write_full_list(w, new.into_iter(), full);
}

/// Reads an id list written by [`write_list`]; the result is sorted by id.
fn read_list<T: ListItem>(
    r: &mut BitReader,
    base: Option<&[T]>,
    max: usize,
    min_bits: usize,
    full: fn(&mut BitReader, u16) -> Result<T, DecodeError>,
    delta: fn(&mut BitReader, &T) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    let mut out: Vec<T> = Vec::new();
    let Some(base) = base else {
        read_full_list(r, max, min_bits, &mut out, full)?;
        if out.windows(2).any(|p| p[0].id() >= p[1].id()) {
            return Err(DecodeError::Invalid("list ids not ascending"));
        }
        return Ok(out);
    };
    for b in base {
        if r.read_bool()? {
            out.push(*b);
        } else if r.read_bool()? {
            let d = delta(r, b)?;
            out.push(d);
        }
    }
    let kept = out.len();
    read_full_list(r, max.saturating_sub(kept), min_bits, &mut out, full)?;
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

    let mask = snap.ships.iter().fold(0u32, |m, s| m | (1 << s.id));
    w.write_u32(mask);
    for s in &snap.ships {
        match baseline.and_then(|b| b.ship(s.id)) {
            Some(bs) => write_ship_delta(w, s, bs),
            None => write_ship_full(w, s),
        }
    }

    write_list(
        w,
        &snap.projectiles,
        baseline.map(|b| &b.projectiles[..]),
        write_projectile_full,
        write_projectile_delta,
    );
    write_list(
        w,
        &snap.entities,
        baseline.map(|b| &b.entities[..]),
        write_entity_full,
        write_entity_delta,
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
    let mut ships = Vec::with_capacity(mask.count_ones() as usize);
    for id in 0..MAX_SHIPS {
        if mask & (1 << id) == 0 {
            continue;
        }
        let ship = match baseline.and_then(|b| b.ship(id)) {
            Some(bs) => read_ship_delta(r, bs)?,
            None => read_ship_full(r, id)?,
        };
        ships.push(ship);
    }

    let projectiles = read_list(
        r,
        baseline.map(|b| &b.projectiles[..]),
        MAX_PROJECTILES_PER_SNAPSHOT,
        PROJECTILE_LIST_MIN_BITS,
        read_projectile_full,
        read_projectile_delta,
    )?;
    let entities = read_list(
        r,
        baseline.map(|b| &b.entities[..]),
        MAX_ENTITIES_PER_SNAPSHOT,
        ENTITY_LIST_MIN_BITS,
        read_entity_full,
        read_entity_delta,
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
    use crate::wire::{NetEntity, NetShip, Snapshot, SHIP_CORE_BITS};
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
        // header (4 + 1 + 1) + ack/time/edit/score varints + mask 4 + 32 bits
        // for ships + 64 bits (unchanged) for entities + two empty new-item
        // lists + 1 bit players.
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

    /// Delta snapshots with 32 moving ships, debris at its cap (64 pieces)
    /// all drifting after an explosion, 40 pickups and 8 wrecks, against a
    /// baseline two snapshots old: one datagram and within the 17 KB/s
    /// budget at 30 Hz. Resting items cost one bit each.
    #[test]
    fn bandwidth_32_ships_64_debris() {
        const BUDGET: usize = 17 * 1024;
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
        let mut ships: Vec<NetShip> = (0..MAX_SHIPS)
            .map(|id| NetShip {
                id,
                pos: Vec3::new(
                    rng.range_f32(20.0, 236.0),
                    rng.range_f32(20.0, 108.0),
                    rng.range_f32(20.0, 236.0),
                ),
                vel: Vec3::new(
                    rng.range_f32(-20.0, 20.0),
                    rng.range_f32(-20.0, 20.0),
                    rng.range_f32(-20.0, 20.0),
                ),
                rot: Quat::from_axis_angle(Vec3::new(0.3, 1.0, -0.2), rng.range_f32(-3.0, 3.0)),
                flags: ship_flags::with_team(ship_flags::ALIVE, id % 2),
                status: None,
            })
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
        let snapshot = |tick: u32, ships: &[NetShip], debris: &[(NetEntity, Vec3)]| {
            let mut entities: Vec<NetEntity> = debris.iter().map(|(e, _)| *e).collect();
            entities.extend(fixed.iter().copied());
            quant.pack_snapshot(&Snapshot {
                tick,
                ack_input: tick + 4,
                ships: ships.to_vec(),
                entities,
                players: players.clone(),
                scores: vec![31_000, 29_500],
                match_ticks_left: 9_000,
                ..Default::default()
            })
        };
        let base = snapshot(1000, &ships, &debris);
        let dt = 2.0 / 60.0;
        for s in &mut ships {
            let accel = Vec3::new(
                rng.range_f32(-30.0, 30.0),
                rng.range_f32(-30.0, 30.0),
                rng.range_f32(-30.0, 30.0),
            );
            s.vel += accel * dt;
            s.pos += s.vel * dt;
            s.rot = Quat::from_axis_angle(Vec3::Y, 0.1) * s.rot;
        }
        for (e, v) in &mut debris {
            e.pos += *v * dt;
        }
        let next = snapshot(1002, &ships, &debris);
        let delta = encode(&next, Some(&base));
        let (back, _) = decode(&delta, &base);
        assert_eq!(back, next);
        // The entity part: the same snapshots without entities.
        let ships_only = {
            let (mut b, mut n) = (base.clone(), next.clone());
            b.entities.clear();
            n.entities.clear();
            encode(&n, Some(&b)).len()
        };
        let entity_bytes = delta.len() - ships_only;
        println!(
            "32 ships + 64 moving debris + 48 resting items: delta {} B ({} B/s at 30 Hz; ships {ships_only} B, entities {entity_bytes} B), full {} B",
            delta.len(),
            delta.len() * 30,
            encode(&next, None).len()
        );
        assert!(delta.len() <= 1200, "fits one datagram: {} B", delta.len());
        // M1 measured 377 B (11.3 KB/s) for 32 moving ships; the entities
        // must fit in the rest of the 17 KB/s budget.
        assert!(
            (377 + entity_bytes) * 30 <= BUDGET,
            "entities {entity_bytes} B per snapshot"
        );
        // Nothing moved: the whole entity list costs a bit per item.
        let same = snapshot(1002, &ships, &debris);
        let mut later = same.clone();
        later.tick = 1004;
        let idle = encode(&later, Some(&same));
        assert!(idle.len() < 60, "unchanged snapshot {} B", idle.len());
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
}
