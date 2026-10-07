//! Cat paw (comedy event; `docs/milestone-4-plan.md` §3.2).
//!
//! A giant cat reaches into the level. The tell (3 s announce phase): the
//! paw waits outside the boundary wall nearest the densest group of ships
//! (the paw prop sits there in phase 0: the client draws its shadow and
//! the claw glow on the wall). Then, in 3 s: the paw (a 10 u pad) reaches
//! in through the wall (0.5 s), sweeps 90 u along an arc through the
//! group (2 s) and withdraws (0.5 s). Ships it touches take 30 damage
//! (environment: no kill credit, "the level" kills) and 25 u/s knockback,
//! once per ship; debris is swept along; its three claws carve grooves
//! (chains of `SubtractSphere` of radius 3, scaled by rock strength, at
//! most 36 edits, spread over the sweep where the claw tips scrape a rock
//! surface, each sphere `groove_inset` inside it, the claws `claw_gap`
//! apart so a ridge stays between the grooves), never in spawn zones, the Heist chamber or near extraction
//! zones. The paw itself passes through rock.
//!
//! - **Eligible**: a living ship within `ship_range` (60 u) of a face of
//!   the level box.
//! - **Group** (at the announcement): among the living ships within reach
//!   of a face, the one with the most living ships within `group_radius`
//!   (30 u; ties: lower id); the group's centroid `C`; the face nearest to
//!   `C` (outward normal `n`).
//! - **Claws** go toward the rock nearest `C`: the wall the paw comes
//!   through (when it is at most `pad_radius + 8` u away: the pad then
//!   sweeps 1 u off the first rock toward the face, generated rock or the
//!   uncarvable sealing wall, and the claws rake it), or either side
//!   of the sweep plane (raycasts across it; the plane shifts up to 6 u
//!   toward that rock).
//! - **Arc**: a circle in the plane of `n` and a tangent `t` (the face
//!   axis along which the box is longer, seeded sign) about a shoulder
//!   outside the wall. Its depth inside the wall at the middle is the
//!   centroid's distance from the face clamped to 12-60 u (11 u plus the
//!   rock's height above the face when the claws rake the wall); its
//!   radius is at least `depth + shoulder` and
//!   at least `sweep_length^2 / (4 depth)`, so the sweep's ends are still
//!   half that deep: a flat swipe along the wall for groups near it, a
//!   rounder one deeper in. It passes through the (shifted) centroid
//!   halfway.
//! - **State** (in [`EventState`](crate::events::EventState)): `pos` = the
//!   arc's middle, `params` = `[face, tangent, claw mode, 0, ...]` (face
//!   and tangent as axis codes `2 * axis + positive`; claw mode 0 = side
//!   `t x n`, 1 = the other side, 2 = the wall), `fvars` = `[depth, 0, 0,
//!   0]`, `vars[0..3]` = grooves carved per claw, `marks` = ships already
//!   hit.
//! - **Prop**: one `ReplicatedKind::CatPaw` (radius = pad radius), posed
//!   every tick: fingers along local -Z (into the level), sweep direction
//!   local +X; the palm with the claws faces local -Y (claw mode 0), +Y (1)
//!   or +Z (2, the wall). State `[phase, progress, face, claw mode]`:
//!   phase 0 tell, 1 reach, 2 sweep, 3 withdraw; progress 0-255 through it.
//!
//! Numbers: `data/events/cat_paw.ron`.

use ls_math::{Mat3, Quat, Tick, Vec3};

use crate::behavior::DamageSource;
use crate::entity::{EntityKind, EntitySpawn, ReplicatedKind};
use crate::events::{EventCtx, EventDef, EventKind, EventModule, EventPhase, EventRef};
use crate::ship::{PlayerId, ShipState};
use crate::world::MatchSetup;
use crate::Command;

/// Cat paw.
#[derive(Debug)]
pub struct CatPaw;

const PAW: EntityKind = EntityKind::Prop(ReplicatedKind::CatPaw);

/// Claw modes (`params[2]`).
pub mod claw_mode {
    /// Claws toward `t x n` (local -Y of the paw).
    pub const SIDE: u8 = 0;
    /// Claws toward `n x t` (local +Y).
    pub const OTHER_SIDE: u8 = 1;
    /// Claws toward the wall the paw came through (local +Z).
    pub const WALL: u8 = 2;
}

/// Axis code `2 * axis + positive` as a unit vector.
pub fn axis_dir(code: u8) -> Vec3 {
    let s = if code & 1 == 1 { 1.0 } else { -1.0 };
    match (code / 2) % 3 {
        0 => Vec3::new(s, 0.0, 0.0),
        1 => Vec3::new(0.0, s, 0.0),
        _ => Vec3::new(0.0, 0.0, s),
    }
}

/// Inside distance of `p` from face `code` of `b` (negative outside).
fn face_distance(b: &ls_math::Aabb, code: u8, p: Vec3) -> f32 {
    let axis = (code / 2) as usize % 3;
    if code & 1 == 1 {
        b.max[axis] - p[axis]
    } else {
        p[axis] - b.min[axis]
    }
}

/// Smoothstep on `[0, 1]`.
fn ease(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// The sweep of one paw (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sweep {
    /// Outward normal of the face it comes through.
    pub n: Vec3,
    /// Sweep tangent.
    pub t: Vec3,
    /// `t x n`: the normal of the sweep plane.
    pub b: Vec3,
    pub claw_mode: u8,
    /// Centre of the arc (outside the wall).
    pub shoulder: Vec3,
    /// Arc radius.
    pub radius: f32,
    /// Arc angle swept (rad).
    pub span: f32,
    /// Face plane: `x . n = face`.
    pub face: f32,
    pub pad_radius: f32,
}

impl Sweep {
    /// Arc radius for a sweep `depth` inside the wall.
    pub fn arc_radius(def: &EventDef, depth: f32) -> f32 {
        let len = def.param("sweep_length", 90.0);
        let shoulder = def.param("shoulder", 30.0).max(1.0);
        (depth + shoulder).max(len * len / (4.0 * depth.max(1.0)))
    }

    /// The sweep stored in an event's state (`middle` = `EventState::pos`).
    pub fn from_state(def: &EventDef, middle: Vec3, params: &[u8; 8], fvars: &[f32; 4]) -> Sweep {
        let n = axis_dir(params[0]);
        let t = axis_dir(params[1]);
        let depth = fvars[0].max(1.0);
        let radius = Sweep::arc_radius(def, depth);
        Sweep {
            n,
            t,
            b: t.cross(n),
            claw_mode: params[2],
            shoulder: middle + n * radius,
            radius,
            span: def.param("sweep_length", 90.0) / radius,
            face: middle.dot(n) + depth,
            pad_radius: def.param("pad_radius", 10.0),
        }
    }

    /// Pad centre at arc angle `theta` (0: the middle).
    pub fn arc(&self, theta: f32) -> Vec3 {
        let (s, c) = ls_math::det::sin_cos(theta);
        self.shoulder + (-self.n * c + self.t * s) * self.radius
    }

    /// Outside the wall, opposite the arc point at `theta`.
    fn outside(&self, theta: f32) -> Vec3 {
        let p = self.arc(theta);
        p + self.n * (self.face - p.dot(self.n) + self.pad_radius)
    }

    /// Orientation at arc angle `theta` (fingers -Z into the level, sweep
    /// direction +X).
    pub fn rot(&self, theta: f32) -> Quat {
        let (s, c) = ls_math::det::sin_cos(theta);
        let fingers = -self.n * c + self.t * s;
        let along = self.n * s + self.t * c;
        let z = -fingers;
        // Right-handed: y = z x x.
        let y = z.cross(along);
        Quat::from_mat3(&Mat3::from_cols(along, y, z))
    }

    /// Timeline: `(phase, progress 0-1, pad centre, arc angle)` at `k`
    /// ticks into an active phase of `active` ticks with a sweep of
    /// `sweep` ticks (reach and withdraw share the rest equally).
    pub fn at(&self, k: u64, active: u64, sweep: u64) -> (u8, f32, Vec3, f32) {
        let sweep = sweep.min(active).max(1);
        let reach = ((active - sweep) / 2).max(1);
        let half = self.span * 0.5;
        if k < reach {
            let u = k as f32 / reach as f32;
            let p = self.outside(-half).lerp(self.arc(-half), ease(u));
            (1, u, p, -half)
        } else if k < reach + sweep {
            let u = (k - reach) as f32 / sweep as f32;
            let theta = (u - 0.5) * self.span;
            (2, u, self.arc(theta), theta)
        } else {
            let left = active.saturating_sub(reach + sweep).max(1);
            let u = ((k - reach - sweep) as f32 / left as f32).min(1.0);
            let p = self.arc(half).lerp(self.outside(half), ease(u));
            (3, u, p, half)
        }
    }

    /// The three claw tips at arc angle `theta` (`gap` apart, on the pad's
    /// rim toward the claw side).
    pub fn claws(&self, theta: f32, gap: f32) -> [Vec3; 3] {
        let p = self.arc(theta);
        let (dir, spread) = self.claw_frame(theta);
        let base = p + dir * self.pad_radius;
        [base - spread * gap, base, base + spread * gap]
    }

    /// The way the claws point at arc angle `theta` (toward the rock they rake).
    pub fn claw_dir(&self, theta: f32) -> Vec3 {
        self.claw_frame(theta).0
    }

    /// `(claw direction, claw row)` at arc angle `theta`.
    fn claw_frame(&self, theta: f32) -> (Vec3, Vec3) {
        let outward = (self.shoulder - self.arc(theta)) / self.radius;
        match self.claw_mode {
            claw_mode::WALL => (outward, self.b),
            claw_mode::OTHER_SIDE => (-self.b, outward),
            _ => (self.b, outward),
        }
    }
}

/// Whether a groove may be carved at `p` (not in spawn zones, the Heist
/// chamber or next to an extraction zone).
pub fn groove_allowed(def: &EventDef, setup: &MatchSetup, p: Vec3, r: f32) -> bool {
    let spawn = def.param("spawn_clearance", 24.0);
    if setup.spawns.iter().any(|s| s.pos.distance(p) < spawn + r) {
        return false;
    }
    if let Some(h) = &setup.heist {
        if h.core.distance(p) < h.chamber_radius.max(h.knot_radius) + r {
            return false;
        }
        let ex = def.param("extraction_clearance", 6.0);
        if h.extraction
            .iter()
            .any(|(c, er)| c.distance(p) < er + ex + r)
        {
            return false;
        }
    }
    true
}

fn living(ships: &std::collections::BTreeMap<PlayerId, ShipState>) -> Vec<&ShipState> {
    ships.values().filter(|s| s.alive).collect()
}

/// Ticks of the sweep itself.
fn sweep_ticks(def: &EventDef) -> u64 {
    Tick::ticks_for_secs(def.param("sweep_secs", 2.0)).max(1)
}

impl EventModule for CatPaw {
    fn kind(&self) -> EventKind {
        EventKind::CatPaw
    }

    fn implemented(&self) -> bool {
        true
    }

    fn eligible(&self, e: &EventRef) -> bool {
        let b = e.view.terrain.bounds();
        let range = e.def.param("ship_range", 60.0);
        living(e.view.ships)
            .iter()
            .any(|s| (0..6).any(|f| face_distance(&b, f, s.pos) <= range))
    }

    fn on_announce(&self, e: &mut EventCtx) {
        let def = e.def;
        let b = e.ctx.view.terrain.bounds();
        let range = def.param("ship_range", 60.0);
        let group_r = def.param("group_radius", 30.0);
        let pad = def.param("pad_radius", 10.0);
        let ships = living(e.ctx.view.ships);
        let near_face = |s: &ShipState| (0..6).any(|f| face_distance(&b, f, s.pos) <= range);
        // The densest group among the ships near a face.
        let best = ships
            .iter()
            .filter(|s| near_face(s))
            .map(|s| {
                let n = ships
                    .iter()
                    .filter(|o| o.pos.distance(s.pos) <= group_r)
                    .count();
                (n, s.player, s.pos)
            })
            .min_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let center = match best {
            Some((_, _, p)) => {
                let group: Vec<Vec3> = ships
                    .iter()
                    .filter(|o| o.pos.distance(p) <= group_r)
                    .map(|o| o.pos)
                    .collect();
                group.iter().fold(Vec3::ZERO, |a, p| a + *p) / group.len().max(1) as f32
            }
            None => b.center(),
        };
        // The nearest face.
        let face = (0..6u8)
            .min_by(|x, y| face_distance(&b, *x, center).total_cmp(&face_distance(&b, *y, center)))
            .unwrap_or(0);
        let n = axis_dir(face);
        let dist = face_distance(&b, face, center);
        // Tangent: the longer face axis, seeded sign.
        let axis = face / 2;
        let others = [(axis + 1) % 3, (axis + 2) % 3];
        let size = b.size();
        let ta = if size[others[1] as usize] > size[others[0] as usize] {
            others[1]
        } else {
            others[0]
        };
        let sign = (e.rng(1).next_u32() & 1) as u8;
        let tangent = ta * 2 + sign;
        // Claws toward the nearest rock: the wall, or a side of the plane.
        let bvec = axis_dir(tangent).cross(n);
        let reach = 40.0;
        let hit = |d: Vec3| {
            e.ctx
                .view
                .terrain
                .raycast(center, d, reach)
                .map_or(reach, |h| h.distance)
        };
        let (plus, minus) = (hit(bvec), hit(-bvec));
        let wall = dist <= pad + 8.0 && dist <= plus.min(minus);
        let (mode, depth, shift) = if wall {
            // The claws rake the first rock toward the face: generated rock
            // in front of the face, else the face itself (D5: the sealing
            // walls inside the level box cannot be carved).
            let rock = hit(n).min(dist);
            (claw_mode::WALL, dist - rock + pad + 1.0, Vec3::ZERO)
        } else {
            let (mode, rock, side) = if minus < plus {
                (claw_mode::OTHER_SIDE, minus, -bvec)
            } else {
                (claw_mode::SIDE, plus, bvec)
            };
            let shift = (rock - pad - 1.0).clamp(0.0, 6.0);
            (mode, dist.clamp(12.0, range.max(12.0)), side * shift)
        };
        let middle = center + n * (dist - depth) + shift;
        e.event.pos = middle;
        e.event.params[0] = face;
        e.event.params[1] = tangent;
        e.event.params[2] = mode;
        e.event.fvars = [depth, 0.0, 0.0, 0.0];
        // The paw waits outside the wall (the tell).
        let sweep = Sweep::from_state(def, middle, &e.event.params, &e.event.fvars);
        let mut s = EntitySpawn::prop(
            ReplicatedKind::CatPaw,
            sweep.outside(0.0),
            sweep.rot(0.0),
            pad,
        );
        s.state = [0, 0, face, mode];
        e.ctx.spawn_entity(s);
    }

    fn on_tick(&self, e: &mut EventCtx) {
        if e.event.phase == EventPhase::Announce {
            return;
        }
        let def = e.def;
        let now = e.now();
        let sweep = Sweep::from_state(def, e.event.pos, &e.event.params, &e.event.fvars);
        let active = def.active_ticks();
        let sweep_t = sweep_ticks(def);
        let k = now.saturating_sub(e.event.active_tick);
        let (phase, u, pos, theta) = sweep.at(k, active, sweep_t);
        let (_, _, next, _) = sweep.at(k + 1, active, sweep_t);
        let vel = (next - pos) * ls_math::TICK_HZ as f32;
        let rot = sweep.rot(theta);
        let state = [
            phase,
            (u * 255.0).clamp(0.0, 255.0) as u8,
            e.event.params[0],
            e.event.params[2],
        ];
        let paws: Vec<u32> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == PAW)
            .map(|x| x.id)
            .collect();
        for id in paws {
            e.ctx.push(Command::PoseEntity {
                id,
                pos,
                vel,
                rot,
                state,
                vars: None,
            });
        }
        // Ships touched: damage and knockback, once each.
        let pad = sweep.pad_radius;
        let damage = def.param("damage", 30.0);
        let kb = def.param("knockback", 25.0);
        let dir = vel.try_normalize().unwrap_or(-sweep.n);
        let hits: Vec<(PlayerId, Vec3)> = e
            .ctx
            .view
            .ships
            .values()
            .filter(|s| {
                let r = e.ctx.view.ship_types.get(s.ship_type).radius + pad;
                s.alive
                    && s.pos.distance_squared(pos) <= r * r
                    && e.event
                        .marks
                        .binary_search_by_key(&s.player, |m| m.0)
                        .is_err()
            })
            .map(|s| (s.player, s.pos))
            .collect();
        for (player, spos) in hits {
            let away = (spos - pos).try_normalize().unwrap_or(dir);
            let push = (dir + away * 0.5).try_normalize().unwrap_or(dir);
            e.ctx
                .damage(player, damage, push * kb, DamageSource::ENVIRONMENT, true);
            let at = e
                .event
                .marks
                .binary_search_by_key(&player, |m| m.0)
                .unwrap_or_else(|i| i);
            e.event.marks.insert(at, (player, now));
        }
        // Debris swept along.
        let debris: Vec<(u32, Vec3)> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| {
                x.kind == EntityKind::Debris
                    && x.pos.distance_squared(pos) <= (pad + x.radius) * (pad + x.radius)
            })
            .map(|x| (x.id, (vel * 1.1 - x.vel) * x.mass()))
            .collect();
        for (id, dv) in debris {
            e.ctx.impulse_entity(id, dv);
        }
        // Claw grooves along the sweep where the tips scrape a rock surface:
        // sample points `groove_spacing` apart along the arc, the ones the
        // pad passed this tick.
        if phase == 2 {
            let reach = ((active - sweep_t.min(active)) / 2).max(1);
            let groove_r = def.param("groove_radius", 3.0);
            let gap = def.param("claw_gap", 7.0);
            let per_claw = (def.param("groove_edits", 36.0) / 3.0).max(0.0) as i64;
            let spacing = def.param("groove_spacing", 3.5).max(0.5);
            let inset = def.param("groove_inset", 0.6);
            let samples = ((def.param("sweep_length", 90.0) / spacing) as u64).max(2);
            let u0 = (k - reach) as f32 / sweep_t as f32;
            let u1 = (k + 1 - reach) as f32 / sweep_t as f32;
            for j in 0..samples {
                let uj = j as f32 / (samples - 1) as f32;
                if uj < u0 || uj >= u1 {
                    continue;
                }
                let theta = (uj - 0.5) * sweep.span;
                let dir = sweep.claw_dir(theta);
                for (c, tip) in sweep.claws(theta, gap).into_iter().enumerate() {
                    let d = e.ctx.view.terrain.sdf(tip);
                    // The groove's sphere sits `groove_inset` inside the rock
                    // the tip scrapes (along the claws' direction), so every
                    // groove has the same depth and width (D5).
                    let at = tip + dir * (d + inset);
                    if e.event.vars[c] >= per_claw
                        || d >= groove_r
                        || d <= -0.6 * groove_r
                        || !groove_allowed(def, e.setup, at, groove_r)
                    {
                        continue;
                    }
                    e.event.vars[c] += 1;
                    e.ctx.crater_material(at, groove_r);
                }
            }
        }
    }

    fn on_end(&self, e: &mut EventCtx) {
        let ids: Vec<u32> = e
            .ctx
            .view
            .entities
            .iter()
            .filter(|x| x.kind == PAW)
            .map(|x| x.id)
            .collect();
        for id in ids {
            e.ctx.push(Command::RemoveEntity { id });
        }
    }
}
