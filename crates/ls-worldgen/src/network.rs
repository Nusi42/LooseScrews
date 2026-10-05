//! The connection network: deterministic caverns and tunnels carved into
//! every level so both team ends, the level centre and the space between
//! them always form one connected open volume with room for every ship.
//!
//! Layout (fractions of the room, jittered by the seed):
//!
//! - **Hubs**: a 4 x 2 grid of caverns (columns at x = 0.14, 0.38, 0.62,
//!   0.86, rows at z = 0.28, 0.72, y around the middle). The outer columns
//!   sit in the team spawn regions and are bigger ([`SPAWN_HUB_RADIUS`]).
//! - **Tunnels** ([`TUNNEL_RADIUS`]): each row is a chain of hubs along x;
//!   both end columns and one middle column link the two rows.
//! - **Heist site** (generator version 3, [`HeistSite`]): the *core vein*, a
//!   chamber ([`VEIN_CHAMBER_RADIUS`]) at the bottom of the level, centred
//!   along x (the same distance from both team ends), holding a knot of rock
//!   ([`VEIN_KNOT_RADIUS`]) around the core; two tunnels link it to the two
//!   middle hubs of the nearer row. Extraction zones: below Enclosure 100 a
//!   *surface cavern* ([`EXTRACTION_RADIUS`]) at the top of the level above
//!   each team's spawn cavern, linked to it by a vertical tunnel; at
//!   Enclosure 100 (fully enclosed) the team spawn caverns themselves. The
//!   site draws from its own random stream, so the hubs and tunnels above
//!   are those of generator version 2.
//!
//! Shapes follow the Enclosure character: below [`BOXY_ENCLOSURE`] hubs are
//! spheres and tunnels are capsules bent through a jittered midpoint (cave
//! passages); from it on, hubs are box rooms and tunnels are axis-aligned
//! square corridors turning at right angles (mine shafts).
//!
//! The generator applies the network last, as `max(terrain, carve)`, so it
//! also cuts through Density pillars and rocks; the vein knot is added after
//! that (`min`), so it is rock on every level. Every primitive is an exact
//! (1-Lipschitz) distance, so the level stays a valid distance bound, and
//! every primitive keeps [`WALL_GAP`] from the sealing walls, so a tunnel's
//! centre line has at least [`TUNNEL_RADIUS`] of clearance.
//!
//! Evaluation is cheap: a grid of [`INDEX_CELL`] cells lists the primitives
//! that can matter in each cell. A primitive at least [`INDEX_CAP`] away
//! from a cell has a value of at most `-INDEX_CAP` there, so it can only
//! change points whose terrain is deeper than that; those (rare) points
//! visit every primitive instead. Both paths return the same bits.

use ls_math::rng::SplitMix64;
use ls_math::{Aabb, Vec3};

use crate::generator::floor_i32;

/// Radius of the connecting tunnels (u). The SDF on a tunnel's centre line
/// is at least this, so there is at least 3 u of clearance within 1 u of it.
pub const TUNNEL_RADIUS: f32 = 4.0;
/// Radius range of the caverns in the team spawn regions (u).
pub const SPAWN_HUB_RADIUS: [f32; 2] = [7.0, 9.0];
/// Radius range of the other caverns (u).
pub const HUB_RADIUS: [f32; 2] = [5.0, 6.5];
/// From this Enclosure on, the network uses mine-style boxes and corridors.
pub const BOXY_ENCLOSURE: u8 = 75;
/// Free space kept between any primitive and the sealing walls (u).
pub const WALL_GAP: f32 = 1.0;
/// Radius of the Heist vein chamber (u; half width of the box room in
/// boxy levels).
pub const VEIN_CHAMBER_RADIUS: f32 = 9.0;
/// Half height of the vein chamber's box room in boxy levels (u).
pub const VEIN_CHAMBER_HALF_HEIGHT: f32 = 6.0;
/// Radius of the rock knot that holds the Heist core (u).
pub const VEIN_KNOT_RADIUS: f32 = 2.0;
/// Radius of the surface caverns that hold the extraction zones (u; half
/// width of the box room in boxy levels).
pub const EXTRACTION_RADIUS: f32 = 7.0;
/// Below this Enclosure the extraction zones sit at the level's surface
/// (top); at it, in the team spawn caverns.
pub const SURFACE_ENCLOSURE_LIMIT: u8 = 100;
/// Salt of the Heist site's random stream ("HEIST_SI").
const HEIST_SALT: u64 = 0x4845_4953_545f_5349;
/// Free space between the knot and the start of the vein tunnels (u).
const VEIN_ACCESS_GAP: f32 = 0.5;
/// Hub column positions along x (fractions of the room).
pub const HUB_COLUMNS: [f32; 4] = [0.14, 0.38, 0.62, 0.86];
/// Hub row positions along z (fractions of the room).
pub const HUB_ROWS: [f32; 2] = [0.28, 0.72];
const JITTER_X: f32 = 0.04;
const JITTER_Z: f32 = 0.08;
const JITTER_Y: f32 = 0.2;
/// Sideways bend of a cave tunnel's midpoint (fraction of its length).
const BEND: f32 = 0.18;
/// Vertical bend of a cave tunnel's midpoint (fraction of the room height).
const BEND_Y: f32 = 0.12;
/// Edge length of an index cell (u).
const INDEX_CELL: f32 = 8.0;
/// Primitives further than this from an index cell are not listed in it (u).
const INDEX_CAP: f32 = 8.0;
/// Rounding margin of the per-primitive box test (u); far above the
/// rounding error of a distance at level scale.
const SKIP_MARGIN: f32 = 1e-3;

/// One carved shape; [`Primitive::value`] is positive inside (open space).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Primitive {
    Sphere {
        c: Vec3,
        r: f32,
    },
    Capsule {
        a: Vec3,
        ab: Vec3,
        len2: f32,
        r: f32,
    },
    Box(Aabb),
}

impl Primitive {
    /// Negated signed distance at `p` (positive inside the carved shape).
    #[inline(always)]
    fn value(&self, p: Vec3) -> f32 {
        match *self {
            Primitive::Sphere { c, r } => r - p.distance(c),
            Primitive::Capsule { a, ab, len2, r } => {
                let t = if len2 > 0.0 {
                    ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                r - p.distance(a + ab * t)
            }
            Primitive::Box(b) => -b.sdf(p),
        }
    }

    /// Box containing the carved shape.
    fn bounds(&self) -> Aabb {
        match *self {
            Primitive::Sphere { c, r } => Aabb::from_center_half_extents(c, Vec3::splat(r)),
            Primitive::Capsule { a, ab, r, .. } => Aabb::new(a, a + ab).expand(r),
            Primitive::Box(b) => b,
        }
    }

    /// Largest value the primitive takes anywhere.
    fn peak(&self) -> f32 {
        match *self {
            Primitive::Sphere { r, .. } | Primitive::Capsule { r, .. } => r,
            Primitive::Box(b) => b.half_extents().min_element(),
        }
    }
}

/// Uniform in `-amount..amount`.
fn jit(rng: &mut SplitMix64, amount: f32) -> f32 {
    (rng.next_f32() - 0.5) * 2.0 * amount
}

/// An extraction zone of the Heist site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtractionZone {
    /// Centre (open space).
    pub pos: Vec3,
    /// A carrier within this distance of `pos` delivers (u).
    pub radius: f32,
    /// Owning team: 0 at low x, 1 at high x.
    pub team: u8,
}

/// Where Heist happens in a level (see the module docs): the core vein and
/// one extraction zone per team. Every level has one (the generator does
/// not know the mode); only Heist uses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeistSite {
    /// The core's position: the centre of the vein knot.
    pub core: Vec3,
    /// Radius of the rock knot around the core (u).
    pub knot_radius: f32,
    /// Radius of the open chamber around the knot (u).
    pub chamber_radius: f32,
    /// A point in the chamber's open space, on the centre line of the vein
    /// tunnels (so it reaches the rest of the network).
    pub access: Vec3,
    /// Extraction zones, team 0 (low x) then team 1 (high x).
    pub extraction: [ExtractionZone; 2],
    /// Whether the zones are surface caverns (Enclosure below
    /// [`SURFACE_ENCLOSURE_LIMIT`]) rather than the team spawn caverns.
    pub surface: bool,
}

/// The carved connection network of one level (see the module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct Network {
    prims: Vec<Primitive>,
    /// Box containing each primitive's carved shape.
    boxes: Vec<Aabb>,
    /// Hub centres, column-major (`column * 2 + row`).
    hubs: Vec<Vec3>,
    /// Centre-line polylines of the tunnels.
    paths: Vec<Vec<Vec3>>,
    /// The Heist site (none for levels too small for a network).
    heist: Option<HeistSite>,
    peak: f32,
    origin: Vec3,
    dims: [usize; 3],
    /// Per index cell: range into `items`.
    cells: Vec<(u32, u32)>,
    items: Vec<u16>,
}

impl Network {
    /// No carving at all.
    pub fn empty() -> Self {
        Network {
            prims: Vec::new(),
            boxes: Vec::new(),
            hubs: Vec::new(),
            paths: Vec::new(),
            heist: None,
            peak: f32::NEG_INFINITY,
            origin: Vec3::ZERO,
            dims: [0; 3],
            cells: Vec::new(),
            items: Vec::new(),
        }
    }

    /// Builds the network for a level with the given open `room`.
    pub fn build(seed: u64, enclosure: u8, room: Aabb) -> Self {
        let size = room.size();
        // Hubs need room for a tunnel plus the wall gap on every axis.
        if size.min_element() < 2.0 * (TUNNEL_RADIUS + WALL_GAP) + 2.0 {
            return Network::empty();
        }
        let boxy = enclosure >= BOXY_ENCLOSURE;
        let mut rng = SplitMix64::new(seed);
        // Largest radius that fits an axis with the wall gap.
        let fit = |r: f32| {
            let half = size * 0.5 - Vec3::splat(WALL_GAP);
            r.min(half.min_element())
        };
        let keep_in = |p: Vec3, r: f32| {
            let m = Vec3::splat(r + WALL_GAP);
            p.clamp(room.min + m, room.max - m)
        };

        let mut prims = Vec::new();
        let mut hubs = Vec::new();
        let mut radii = Vec::new();
        for (col, fx) in HUB_COLUMNS.iter().enumerate() {
            let spawn = col == 0 || col == HUB_COLUMNS.len() - 1;
            for fz in HUB_ROWS {
                let x = room.min.x + size.x * (fx + jit(&mut rng, JITTER_X));
                let z = room.min.z + size.z * (fz + jit(&mut rng, JITTER_Z));
                let y = room.min.y + size.y * (0.5 + jit(&mut rng, JITTER_Y));
                let [lo, hi] = if spawn { SPAWN_HUB_RADIUS } else { HUB_RADIUS };
                let r = fit(lo + (hi - lo) * (jit(&mut rng, 0.5) + 0.5));
                let c = keep_in(Vec3::new(x, y, z), r);
                hubs.push(c);
                radii.push(r);
                prims.push(if boxy {
                    // A mine room: flatter than wide, never lower than a tunnel.
                    let hy = (0.6 * r).max(TUNNEL_RADIUS).min(r);
                    Primitive::Box(Aabb::from_center_half_extents(c, Vec3::new(r, hy, r)))
                } else {
                    Primitive::Sphere { c, r }
                });
            }
        }
        let hub = |col: usize, row: usize| col * HUB_ROWS.len() + row;
        let mut edges = Vec::new();
        for row in 0..HUB_ROWS.len() {
            for col in 0..HUB_COLUMNS.len() - 1 {
                edges.push((hub(col, row), hub(col + 1, row)));
            }
        }
        let middle = 1 + (rng.next_u32() % 2) as usize;
        for col in [0, middle, HUB_COLUMNS.len() - 1] {
            edges.push((hub(col, 0), hub(col, 1)));
        }

        let r = TUNNEL_RADIUS;
        let mut paths = Vec::new();
        for (i, j) in edges {
            let (a, b) = (hubs[i], hubs[j]);
            let path = if boxy {
                // Axis-aligned legs, in a seeded axis order.
                let order = match rng.next_u32() % 6 {
                    0 => [0, 1, 2],
                    1 => [0, 2, 1],
                    2 => [1, 0, 2],
                    3 => [1, 2, 0],
                    4 => [2, 0, 1],
                    _ => [2, 1, 0],
                };
                let mut pts = vec![a];
                let mut cur = a;
                for axis in order {
                    let mut next = cur.to_array();
                    next[axis] = b[axis];
                    let next = Vec3::from_array(next);
                    if next != cur {
                        pts.push(next);
                        cur = next;
                    }
                }
                pts
            } else {
                let d = b - a;
                let side = Vec3::new(-d.z, 0.0, d.x).normalize_or_zero();
                let len = d.length();
                let m = (a + b) * 0.5
                    + side * (len * jit(&mut rng, BEND))
                    + Vec3::new(0.0, size.y * jit(&mut rng, BEND_Y), 0.0);
                vec![a, keep_in(m, r), b]
            };
            for w in path.windows(2) {
                prims.push(if boxy {
                    Primitive::Box(Aabb::new(w[0], w[1]).expand(r))
                } else {
                    let ab = w[1] - w[0];
                    Primitive::Capsule {
                        a: w[0],
                        ab,
                        len2: ab.dot(ab),
                        r,
                    }
                });
            }
            paths.push(path);
        }
        let mut carved = Carved {
            prims,
            paths,
            boxy,
            room,
        };
        let heist = heist_site(seed, enclosure, &hubs, &radii, &mut carved);
        Network::index(carved.prims, hubs, carved.paths, heist, room)
    }

    fn index(
        prims: Vec<Primitive>,
        hubs: Vec<Vec3>,
        paths: Vec<Vec<Vec3>>,
        heist: Option<HeistSite>,
        room: Aabb,
    ) -> Self {
        let peak = prims
            .iter()
            .map(Primitive::peak)
            .fold(f32::NEG_INFINITY, f32::max);
        // Points outside the indexed box are at least INDEX_CAP from every
        // primitive (all lie inside the room).
        let area = room.expand(INDEX_CAP);
        let origin = area.min;
        let dims = [0, 1, 2].map(|a| (floor_i32(area.size()[a] / INDEX_CELL) + 1).max(1) as usize);
        let boxes: Vec<Aabb> = prims.iter().map(Primitive::bounds).collect();
        let mut cells = Vec::with_capacity(dims[0] * dims[1] * dims[2]);
        let mut items = Vec::new();
        for z in 0..dims[2] {
            for y in 0..dims[1] {
                for x in 0..dims[0] {
                    let min = origin + Vec3::new(x as f32, y as f32, z as f32) * INDEX_CELL;
                    let cell = Aabb::new(min, min + Vec3::splat(INDEX_CELL));
                    let start = items.len() as u32;
                    for (i, b) in boxes.iter().enumerate() {
                        let gap = (cell.min - b.max).max(b.min - cell.max).max(Vec3::ZERO);
                        // Distance to the shape >= distance to its box.
                        if gap.length() < INDEX_CAP + 0.01 {
                            items.push(i as u16);
                        }
                    }
                    cells.push((start, items.len() as u32));
                }
            }
        }
        Network {
            prims,
            boxes,
            hubs,
            paths,
            heist,
            peak,
            origin,
            dims,
            cells,
            items,
        }
    }

    /// Hub (cavern) centres, `column * 2 + row` with columns along x (see
    /// [`HUB_COLUMNS`]); empty for levels too small for a network.
    pub fn hubs(&self) -> &[Vec3] {
        &self.hubs
    }

    /// Hub centres in the lowest-x column (team 0's spawn region).
    pub fn low_x_hubs(&self) -> &[Vec3] {
        &self.hubs[..self.hubs.len().min(HUB_ROWS.len())]
    }

    /// Hub centres in the highest-x column (team 1's spawn region).
    pub fn high_x_hubs(&self) -> &[Vec3] {
        &self.hubs[self.hubs.len().saturating_sub(HUB_ROWS.len())..]
    }

    /// Centre-line polylines of the tunnels: hub to hub, then the vein
    /// tunnels (from [`HeistSite::access`]) and the surface shafts.
    pub fn tunnel_paths(&self) -> &[Vec<Vec3>] {
        &self.paths
    }

    /// The Heist site (core vein and extraction zones); `None` for levels
    /// too small for a network.
    pub fn heist(&self) -> Option<&HeistSite> {
        self.heist.as_ref()
    }

    /// Points of the network's centre lines to try linking `p` to, nearest
    /// first (ties in hub, then tunnel order): every hub centre and the
    /// closest point of every tunnel segment. The centre lines form one
    /// connected set with at least [`TUNNEL_RADIUS`] of free space.
    pub fn link_targets(&self, p: Vec3) -> Vec<Vec3> {
        let mut t: Vec<(f32, usize, Vec3)> = Vec::new();
        for &h in &self.hubs {
            t.push((p.distance(h), t.len(), h));
        }
        for path in &self.paths {
            for w in path.windows(2) {
                let ab = w[1] - w[0];
                let len2 = ab.dot(ab);
                let s = if len2 > 0.0 {
                    ((p - w[0]).dot(ab) / len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let q = w[0] + ab * s;
                t.push((p.distance(q), t.len(), q));
            }
        }
        t.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        t.into_iter().map(|(_, _, q)| q).collect()
    }

    /// Whether a ship keeping `margin` of free space (see [`segment_clear`])
    /// can fly in a straight line from `p` to the network's centre lines
    /// (one of the `tries` nearest [`Network::link_targets`]) under the SDF
    /// `sample`. False for an empty network.
    pub fn reaches(
        &self,
        sample: impl Fn(Vec3) -> f32,
        p: Vec3,
        margin: f32,
        tries: usize,
    ) -> bool {
        if sample(p) < margin {
            return false;
        }
        self.link_targets(p)
            .into_iter()
            .take(tries)
            .any(|q| segment_clear(&sample, p, q, margin))
    }

    /// `min(max(open, value of every primitive at p), knot)`: the terrain
    /// value `open` with the network carved in, then the Heist vein knot
    /// (a rock sphere) added. Exact for every `open`.
    #[inline(always)]
    pub fn carve(&self, p: Vec3, open: f32) -> f32 {
        let carved = self.carve_open(p, open);
        match &self.heist {
            Some(h) => carved.min(p.distance(h.core) - h.knot_radius),
            None => carved,
        }
    }

    /// `max(open, value of every primitive at p)` (without the knot).
    #[inline(always)]
    fn carve_open(&self, p: Vec3, open: f32) -> f32 {
        // No primitive exceeds its peak (also covers the empty network).
        if open >= self.peak {
            return open;
        }
        let mut best = open;
        if open > -INDEX_CAP {
            for &i in self.cell_items(p) {
                best = self.carve_one(i as usize, p, best);
            }
        } else {
            for i in 0..self.prims.len() {
                best = self.carve_one(i, p, best);
            }
        }
        best
    }

    /// `best.max(value of primitive i at p)`, skipping the exact distance
    /// when the primitive's box is provably further than `-best`: outside
    /// the box the value is at most minus the largest per-axis gap `g`, so
    /// `g > 0 && -g + SKIP_MARGIN <= best` leaves `best` unchanged (the
    /// margin covers float rounding of the exact distance).
    #[inline(always)]
    fn carve_one(&self, i: usize, p: Vec3, best: f32) -> f32 {
        let b = &self.boxes[i];
        let gap = (b.min - p).max(p - b.max).max_element();
        if gap > 0.0 && SKIP_MARGIN - gap <= best {
            return best;
        }
        best.max(self.prims[i].value(p))
    }

    /// The primitives listed for the index cell of `p` (none outside).
    #[inline(always)]
    fn cell_items(&self, p: Vec3) -> &[u16] {
        let q = p - self.origin;
        let mut idx = 0usize;
        let mut stride = 1usize;
        for a in 0..3 {
            let i = floor_i32(q[a] / INDEX_CELL);
            if i < 0 || i as usize >= self.dims[a] {
                return &[];
            }
            idx += i as usize * stride;
            stride *= self.dims[a];
        }
        let (s, e) = self.cells[idx];
        &self.items[s as usize..e as usize]
    }
}

/// The primitives and tunnel paths of a network under construction.
struct Carved {
    prims: Vec<Primitive>,
    paths: Vec<Vec<Vec3>>,
    boxy: bool,
    room: Aabb,
}

impl Carved {
    /// `p` moved inside the room so a shape of half size `half` keeps
    /// [`WALL_GAP`] from the walls.
    fn keep_in(&self, p: Vec3, half: Vec3) -> Vec3 {
        let m = half + Vec3::splat(WALL_GAP);
        p.clamp(self.room.min + m, self.room.max - m)
    }

    /// A tunnel along `path` (capsules, or boxes in boxy levels).
    fn tunnel(&mut self, path: Vec<Vec3>) {
        let r = TUNNEL_RADIUS;
        for w in path.windows(2) {
            self.prims.push(if self.boxy {
                Primitive::Box(Aabb::new(w[0].min(w[1]), w[0].max(w[1])).expand(r))
            } else {
                let ab = w[1] - w[0];
                Primitive::Capsule {
                    a: w[0],
                    ab,
                    len2: ab.dot(ab),
                    r,
                }
            });
        }
        self.paths.push(path);
    }

    /// A room at `c`: a sphere of radius `r`, or a box with half extents
    /// `(r, hy, r)` in boxy levels.
    fn room_at(&mut self, c: Vec3, r: f32, hy: f32) {
        self.prims.push(if self.boxy {
            Primitive::Box(Aabb::from_center_half_extents(c, Vec3::new(r, hy, r)))
        } else {
            Primitive::Sphere { c, r }
        });
    }

    /// Axis-aligned legs from `a` to `b`, up or down first, then x, then z
    /// (boxy levels), or one straight segment.
    fn legs(&self, a: Vec3, b: Vec3) -> Vec<Vec3> {
        if !self.boxy {
            return vec![a, b];
        }
        let mut pts = vec![a];
        let mut cur = a;
        for axis in [1usize, 0, 2] {
            let mut next = cur.to_array();
            next[axis] = b[axis];
            let next = Vec3::from_array(next);
            if next != cur {
                pts.push(next);
                cur = next;
            }
        }
        pts
    }
}

/// Carves the Heist site into `carved` and describes it (see the module
/// docs). Uses its own random stream, so the hubs and tunnels drawn before
/// stay as they were.
fn heist_site(
    seed: u64,
    enclosure: u8,
    hubs: &[Vec3],
    radii: &[f32],
    carved: &mut Carved,
) -> Option<HeistSite> {
    let room = carved.room;
    let size = room.size();
    let rows = HUB_ROWS.len();
    let hub = |col: usize, row: usize| col * rows + row;
    let last_col = HUB_COLUMNS.len() - 1;
    // The vein: bottom of the level, centred along x, z near the middle.
    let chamber_r = VEIN_CHAMBER_RADIUS
        .min(size.x * 0.5 - WALL_GAP)
        .min(size.z * 0.5 - WALL_GAP);
    let half_y = if carved.boxy {
        VEIN_CHAMBER_HALF_HEIGHT
    } else {
        chamber_r
    };
    let access_up = VEIN_KNOT_RADIUS + TUNNEL_RADIUS + VEIN_ACCESS_GAP;
    // Room for the chamber, the knot and the access point above it.
    if size.y < 2.0 * (half_y + WALL_GAP) + access_up || chamber_r <= VEIN_KNOT_RADIUS + 2.0 {
        return None;
    }
    let mut rng = SplitMix64::new(seed ^ HEIST_SALT);
    let half = Vec3::new(chamber_r, half_y, chamber_r);
    let core = carved.keep_in(
        Vec3::new(
            room.center().x,
            room.min.y,
            room.min.z + size.z * (0.5 + jit(&mut rng, 0.1)),
        ),
        half,
    );
    carved.room_at(core, chamber_r, half_y);
    let access = core + Vec3::new(0.0, access_up, 0.0);
    // Two tunnels up to the middle hubs of the row nearer the vein.
    let row = if core.z < room.center().z { 0 } else { 1 };
    for col in [1, last_col - 1] {
        let target = hubs[hub(col, row)];
        let path = if carved.boxy {
            carved.legs(access, target)
        } else {
            // Bent like the other cave tunnels, never below the access
            // point (keeps the centre line clear of the knot).
            let d = target - access;
            let side = Vec3::new(-d.z, 0.0, d.x).normalize_or_zero();
            let mut m = (access + target) * 0.5
                + side * (d.length() * jit(&mut rng, BEND))
                + Vec3::new(0.0, size.y * jit(&mut rng, BEND_Y), 0.0);
            m = carved.keep_in(m, Vec3::splat(TUNNEL_RADIUS));
            m.y = m.y.max(access.y);
            vec![access, m, target]
        };
        carved.tunnel(path);
    }
    // Extraction zones above (or in) the spawn caverns of one row.
    let ex_row = (rng.next_u32() % rows as u32) as usize;
    let surface = enclosure < SURFACE_ENCLOSURE_LIMIT;
    let mut extraction = [ExtractionZone {
        pos: Vec3::ZERO,
        radius: 0.0,
        team: 0,
    }; 2];
    for (team, col) in [0, last_col].into_iter().enumerate() {
        let i = hub(col, ex_row);
        let spawn = hubs[i];
        extraction[team] = if surface {
            let r = EXTRACTION_RADIUS;
            let hy = if carved.boxy {
                (0.6 * r).max(TUNNEL_RADIUS).min(r)
            } else {
                r
            };
            let c = carved.keep_in(Vec3::new(spawn.x, room.max.y, spawn.z), Vec3::new(r, hy, r));
            carved.room_at(c, r, hy);
            let path = carved.legs(spawn, c);
            carved.tunnel(path);
            ExtractionZone {
                pos: c,
                radius: r,
                team: team as u8,
            }
        } else {
            ExtractionZone {
                pos: spawn,
                radius: radii[i],
                team: team as u8,
            }
        };
    }
    Some(HeistSite {
        core,
        knot_radius: VEIN_KNOT_RADIUS,
        chamber_radius: chamber_r,
        access,
        extraction,
        surface,
    })
}

/// Whether the straight segment from `a` to `b` keeps at least `margin` of
/// free space under the SDF `sample`, checked by sphere tracing: after a
/// sample `v`, the next `(v - margin) / 1.05` u (the chunk store's margin
/// for the noise gradient) are clear. Steps are at least
/// [`MIN_TRACE_STEP`], so between two samples the free space may dip to
/// `margin - 1.05 * MIN_TRACE_STEP / 2` (0.13 u less). Gives up (false)
/// after [`MAX_TRACE_SAMPLES`].
pub fn segment_clear(sample: impl Fn(Vec3) -> f32, a: Vec3, b: Vec3, margin: f32) -> bool {
    let d = b - a;
    let len = d.length();
    let dir = if len > 0.0 { d / len } else { Vec3::ZERO };
    let mut t = 0.0f32;
    for _ in 0..MAX_TRACE_SAMPLES {
        let v = sample(a + dir * t);
        if v < margin {
            return false;
        }
        if t >= len {
            return true;
        }
        t = (t + ((v - margin) / 1.05).max(MIN_TRACE_STEP)).min(len);
    }
    false
}

/// Smallest sphere-tracing step of [`segment_clear`] (u).
pub const MIN_TRACE_STEP: f32 = 0.25;
/// Most samples [`segment_clear`] takes.
pub const MAX_TRACE_SAMPLES: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn room() -> Aabb {
        Aabb::new(Vec3::splat(2.0), Vec3::new(254.0, 126.0, 254.0))
    }

    /// The index never changes the result: listed-only and every-primitive
    /// evaluation agree bit for bit.
    #[test]
    fn index_is_exact() {
        for (seed, e) in [(1u64, 50u8), (2, 100), (3, 0)] {
            let n = Network::build(seed, e, room());
            let mut rng = SplitMix64::new(seed);
            for _ in 0..20_000 {
                let p = Vec3::new(
                    rng.next_f32() * 280.0 - 12.0,
                    rng.next_f32() * 150.0 - 12.0,
                    rng.next_f32() * 280.0 - 12.0,
                );
                let open = rng.next_f32() * 40.0 - 25.0;
                let h = n.heist().unwrap();
                let all = n
                    .prims
                    .iter()
                    .fold(open, |b, pr| b.max(pr.value(p)))
                    .min(p.distance(h.core) - h.knot_radius);
                assert_eq!(n.carve(p, open).to_bits(), all.to_bits(), "{p:?} {open}");
            }
        }
    }

    #[test]
    fn layout_follows_the_room_and_enclosure() {
        let r = room();
        for e in [0u8, 50, 74, 75, 100] {
            let n = Network::build(9, e, r);
            assert_eq!(n.hubs().len(), 8);
            // 9 hub tunnels, 2 vein tunnels, 2 surface shafts below 100.
            let shafts = if e < SURFACE_ENCLOSURE_LIMIT { 2 } else { 0 };
            assert_eq!(n.tunnel_paths().len(), 11 + shafts);
            let boxy = n.prims.iter().all(|p| matches!(p, Primitive::Box(_)));
            let round = n.prims.iter().all(|p| !matches!(p, Primitive::Box(_)));
            assert_eq!((boxy, round), (e >= BOXY_ENCLOSURE, e < BOXY_ENCLOSURE));
            for p in &n.prims {
                let b = p.bounds();
                assert!(
                    b.min.min_element() >= r.min.x + WALL_GAP - 1e-3,
                    "{e}: {b:?}"
                );
                assert!(
                    (r.max - b.max).min_element() >= WALL_GAP - 1e-3,
                    "{e}: {b:?}"
                );
            }
            // Spawn hubs sit in the outer 30% of the level along x.
            assert!(n
                .low_x_hubs()
                .iter()
                .all(|h| h.x < r.min.x + 0.3 * r.size().x));
            assert!(n
                .high_x_hubs()
                .iter()
                .all(|h| h.x > r.max.x - 0.3 * r.size().x));
        }
        // Tiny rooms get no network; small ones still fit.
        let tiny = Aabb::new(Vec3::splat(2.0), Vec3::splat(10.0));
        assert!(Network::build(1, 50, tiny).hubs().is_empty());
        assert!(Network::build(1, 50, tiny).heist().is_none());
        assert_eq!(
            Network::build(1, 50, tiny).carve(Vec3::splat(5.0), -3.0),
            -3.0
        );
        let small = Aabb::new(Vec3::splat(2.0), Vec3::splat(62.0));
        assert_eq!(Network::build(1, 50, small).hubs().len(), 8);
    }

    /// The Heist site: the core sits in rock at the bottom of the level,
    /// centred along x, inside an open chamber that the vein tunnels reach;
    /// the extraction zones are open, one per team end, at the top of the
    /// level below Enclosure 100 and in the spawn caverns at 100.
    #[test]
    fn heist_site_layout() {
        let r = room();
        for (seed, e) in [(1u64, 0u8), (2, 50), (3, 74), (4, 75), (5, 99), (6, 100)] {
            let n = Network::build(seed, e, r);
            let h = *n.heist().expect("site");
            let open = |p: Vec3| n.carve(p, -5.0);
            assert_eq!(h.core.x, r.center().x, "{e}: same distance from both ends");
            assert!(
                h.core.y < r.min.y + 0.15 * r.size().y,
                "{e}: deep {:?}",
                h.core
            );
            assert_eq!(open(h.core), -h.knot_radius, "{e}: the core is in rock");
            assert!(
                open(h.access) >= TUNNEL_RADIUS,
                "{e}: access {}",
                open(h.access)
            );
            // The knot's surface is open space all around (a ship gets to it).
            for d in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z, Vec3::Y] {
                let p = h.core + d * (h.knot_radius + 1.5);
                assert!(open(p) >= 1.0, "{e}: {p:?} {}", open(p));
            }
            assert!(n.reaches(open, h.access, 1.5, 1));
            assert_eq!(h.surface, e < SURFACE_ENCLOSURE_LIMIT);
            let [z0, z1] = h.extraction;
            assert_eq!((z0.team, z1.team), (0, 1));
            assert!(z0.pos.x < r.min.x + 0.3 * r.size().x, "{e}: {z0:?}");
            assert!(z1.pos.x > r.max.x - 0.3 * r.size().x, "{e}: {z1:?}");
            for z in [z0, z1] {
                assert!(open(z.pos) >= TUNNEL_RADIUS - 1e-3, "{e}: {z:?}");
                assert!(z.radius >= HUB_RADIUS[0], "{e}: {z:?}");
                assert!(n.reaches(open, z.pos, 1.5, 1), "{e}: {z:?}");
                if h.surface {
                    assert!(z.pos.y > r.max.y - 0.15 * r.size().y, "{e}: {z:?}");
                } else {
                    assert!(n.hubs().contains(&z.pos), "{e}: {z:?}");
                }
            }
            // The site's own stream leaves the version 2 hubs alone, and the
            // site depends on the seed.
            let other = Network::build(seed + 100, e, r);
            assert_ne!(other.heist().unwrap().core, h.core);
        }
    }

    #[test]
    fn segment_clear_traces_free_space() {
        // Open everywhere except a slab of rock at 10 <= x <= 12.
        let sdf = |p: Vec3| (10.0 - p.x).max(p.x - 12.0);
        let a = Vec3::new(0.0, 0.0, 0.0);
        assert!(segment_clear(sdf, a, Vec3::new(8.0, 5.0, 0.0), 1.5));
        assert!(!segment_clear(sdf, a, Vec3::new(9.0, 0.0, 0.0), 1.5));
        assert!(!segment_clear(sdf, a, Vec3::new(20.0, 0.0, 0.0), 0.5));
        assert!(segment_clear(sdf, a, a, 1.0));
        // Hubs reach themselves; a network links hubs of both ends.
        let n = Network::build(4, 50, room());
        let open = |p: Vec3| n.carve(p, -5.0);
        let (lo, hi) = (n.low_x_hubs()[0], n.high_x_hubs()[0]);
        assert!(n.reaches(open, lo, 1.5, 1));
        assert!(n.reaches(open, hi, 1.5, 1));
        assert!(!n.reaches(open, Vec3::new(128.0, 3.0, 3.0), 1.5, 4));
    }
}
