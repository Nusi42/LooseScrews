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
//! projectiles         varint count, then per projectile: u16 id,
//!                     [delta: 1 bit "in baseline"], full state or diffs
//! ```
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
use crate::quant::PackedVec3;
use crate::wire::{
    read_edit, ship_flags, write_edit, PackedProjectile, PackedShip, PackedSnapshot, ShipStatus,
    TerrainEditBits, EDIT_MIN_BITS, MAX_SHIPS, PROJECTILE_BITS,
};

/// Upper bound on edits in one snapshot (the rest wait for the next one).
pub const MAX_EDITS_PER_SNAPSHOT: usize = 1024;
/// Upper bound on projectiles in one snapshot.
pub const MAX_PROJECTILES_PER_SNAPSHOT: usize = 1024;
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
    if e.kind > 2 || !finite || f32::from_bits(e.radius) < 0.0 {
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
    write_vec16(w, p.pos);
    write_vec16(w, p.vel);
}

fn read_projectile_full(r: &mut BitReader, id: u16) -> Result<PackedProjectile, DecodeError> {
    Ok(PackedProjectile {
        id,
        kind: r.read_u8()?,
        owner: r.read_u8()?,
        pos: read_vec16(r)?,
        vel: read_vec16(r)?,
    })
}

fn write_projectile_delta(w: &mut BitWriter, p: &PackedProjectile, b: &PackedProjectile) {
    let changed = p != b;
    w.write_bool(changed);
    if !changed {
        return;
    }
    let meta = p.kind != b.kind || p.owner != b.owner;
    w.write_bool(meta);
    if meta {
        w.write_u8(p.kind);
        w.write_u8(p.owner);
    }
    write_vec_diff(w, p.pos, b.pos);
    write_vec_diff(w, p.vel, b.vel);
}

fn read_projectile_delta(
    r: &mut BitReader,
    b: &PackedProjectile,
) -> Result<PackedProjectile, DecodeError> {
    if !r.read_bool()? {
        return Ok(*b);
    }
    let (kind, owner) = if r.read_bool()? {
        (r.read_u8()?, r.read_u8()?)
    } else {
        (b.kind, b.owner)
    };
    Ok(PackedProjectile {
        id: b.id,
        kind,
        owner,
        pos: read_vec_diff(r, b.pos)?,
        vel: read_vec_diff(r, b.vel)?,
    })
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

    w.write_varint(snap.projectiles.len() as u64);
    for p in &snap.projectiles {
        w.write_u16(p.id);
        match baseline {
            Some(b) => match b.projectile(p.id) {
                Some(bp) => {
                    w.write_bool(true);
                    write_projectile_delta(w, p, bp);
                }
                None => {
                    w.write_bool(false);
                    write_projectile_full(w, p);
                }
            },
            None => write_projectile_full(w, p),
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

    // Every projectile needs at least its 16-bit id plus a bit.
    let n_proj = r.read_len(MAX_PROJECTILES_PER_SNAPSHOT, 17)?;
    let mut projectiles: Vec<PackedProjectile> = Vec::with_capacity(n_proj);
    for _ in 0..n_proj {
        let id = r.read_u16()?;
        if projectiles.last().is_some_and(|p| p.id >= id) {
            return Err(DecodeError::Invalid("projectile ids not ascending"));
        }
        let p = match baseline {
            Some(b) => {
                if r.read_bool()? {
                    let bp = b
                        .projectile(id)
                        .ok_or(DecodeError::Invalid("projectile not in baseline"))?;
                    read_projectile_delta(r, bp)?
                } else {
                    read_projectile_full(r, id)?
                }
            }
            None => read_projectile_full(r, id)?,
        };
        projectiles.push(p);
    }

    Ok((
        PackedSnapshot {
            tick,
            ack_input,
            edit_base,
            edits,
            ships,
            projectiles,
            scores,
            match_ticks_left,
        },
        baseline_tick,
    ))
}

// Keep the documented sizes honest at compile time.
const _: () = assert!(crate::wire::SHIP_CORE_BITS == 18 * 8);
const _: () = assert!(PROJECTILE_BITS == 16 * 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quant::{pack_quat, Quantizer};
    use crate::wire::{NetShip, Snapshot, SHIP_CORE_BITS};
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
        PackedSnapshot {
            tick,
            ack_input: tick.wrapping_add(rng.range_i32(-20, 20) as u32),
            edit_base: rng.below(10_000),
            edits,
            ships,
            projectiles,
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
        let mut next = base.clone();
        next.tick += 2;
        let delta = encode(&next, Some(&base));
        // header (4 + 1 + 1) + ack/time/edit/score varints + mask 4 + 32 bits.
        assert!(delta.len() <= 24, "{} bytes", delta.len());
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
