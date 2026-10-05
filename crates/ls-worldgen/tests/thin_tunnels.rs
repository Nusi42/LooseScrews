//! Thin tunnels: the drawn rock around a tunnel narrower than two grid cells
//! must match the space a ship can fly through (visual-quality plan, VQP; the
//! VQ1 finding that a 1.2 u bore rendered about 0.65 u wide).
//!
//! A straight bore of radius `r` through solid rock (exact field, negative in
//! rock) is meshed with the game's mesher (`surface_nets_with_sdf`), then rays
//! are cast from the bore's axis outwards in its cross-section planes. The
//! distance to the first triangle is the drawn radius in that direction.
//!
//! ```text
//! cargo test -p ls-worldgen --release --test thin_tunnels -- --nocapture
//! ```

use ls_math::Vec3;
use ls_worldgen::{surface_nets_with_sdf, surface_nets_with_sdf_thin_where, Mesh, SampleGrid};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

/// Distance along the ray `o + t d` to triangle `abc`, if it hits.
fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(-1e-5..=1.0 + 1e-5).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-5 || u + v > 1.0 + 1e-5 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

fn first_hit(m: &Mesh, o: Vec3, d: Vec3, max: f32) -> Option<f32> {
    let p = |i: u32| Vec3::from_array(m.positions[i as usize]);
    let mut best: Option<f32> = None;
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        // Cheap reject: triangles far from the ray origin.
        if a.distance(o) > max + 2.0 {
            continue;
        }
        if let Some(h) = ray_triangle(o, d, a, b, c) {
            if h <= max && best.is_none_or(|b| h < b) {
                best = Some(h);
            }
        }
    }
    best
}

/// A bore of radius `r` along unit `axis` through `point`.
struct Bore {
    point: Vec3,
    axis: Vec3,
    r: f32,
}

impl Bore {
    fn field(&self, p: Vec3) -> f32 {
        let q = p - self.point;
        let along = q.dot(self.axis);
        let radial = (q - self.axis * along).length();
        // Air (positive) inside the bore.
        self.r - radial
    }
}

struct Drawn {
    min: f32,
    mean: f32,
    max: f32,
    missing: usize,
}

/// Drawn radius of the bore in `slices` cross-sections, `rays` per slice.
fn drawn_radius(bore: &Bore) -> Drawn {
    let grid = SampleGrid::from_fn([0, 0, 0], |p| bore.field(p));
    let mesh = surface_nets_with_sdf(&grid, &|p| bore.field(p));
    let up = if bore.axis.y.abs() > 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let s = bore.axis.cross(up).normalize();
    let t = bore.axis.cross(s);
    let (mut min, mut max, mut sum, mut n, mut missing) = (f32::MAX, 0.0f32, 0.0f32, 0, 0);
    for slice in 0..24 {
        // Stay well inside the chunk.
        let o = bore.point + bore.axis * (-6.0 + slice as f32 * 0.53);
        for k in 0..48 {
            let a = k as f32 / 48.0 * std::f32::consts::TAU;
            let d = s * a.cos() + t * a.sin();
            match first_hit(&mesh, o, d, 4.0) {
                Some(h) => {
                    min = min.min(h);
                    max = max.max(h);
                    sum += h;
                    n += 1;
                }
                None => missing += 1,
            }
        }
    }
    Drawn {
        min,
        mean: if n > 0 { sum / n as f32 } else { 0.0 },
        max,
        missing,
    }
}

fn bores(r: f32) -> Vec<(String, Bore)> {
    let c = Vec3::splat(16.0);
    let mut out = Vec::new();
    for (name, off) in [
        ("on a grid line", Vec3::new(0.0, 0.0, 0.0)),
        ("quarter cell", Vec3::new(0.0, 0.25, 0.1)),
        ("between lines", Vec3::new(0.0, 0.5, 0.0)),
        ("cell centre", Vec3::new(0.0, 0.5, 0.5)),
    ] {
        out.push((
            format!("x axis, {name}"),
            Bore {
                point: c + off,
                axis: Vec3::X,
                r,
            },
        ));
    }
    out.push((
        "diagonal".into(),
        Bore {
            point: c + Vec3::new(0.0, 0.3, 0.2),
            axis: Vec3::new(1.0, 0.6, 0.35).normalize(),
            r,
        },
    ));
    out.push((
        "face diagonal".into(),
        Bore {
            point: c + Vec3::new(0.1, 0.0, 0.4),
            axis: Vec3::new(1.0, 1.0, 0.0).normalize(),
            r,
        },
    ));
    // Deterministic pseudo-random offsets and directions.
    let mut state = 0x9e37_79b9u32;
    let mut rnd = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    for n in 0..8 {
        let axis = Vec3::new(1.0, rnd() * 1.6 - 0.8, rnd() * 1.6 - 0.8).normalize();
        out.push((
            format!("random {n}"),
            Bore {
                point: c + Vec3::new(rnd(), rnd(), rnd()),
                axis,
                r,
            },
        ));
    }
    out
}

/// The drawn bore is as wide as the field says: every ray finds the wall
/// (the bore is drawn all along), on average at the true radius, and
/// nowhere much closer (the drawn rock never takes up much of the space a
/// ship can use). Before VQP a bore of radius 0.6 through cell centres was
/// not drawn at all, diagonal ones were drawn in pieces (rays found walls
/// 0.01 u from the axis or none), and the others as squares whose faces
/// came within 0.31 u of the axis. Now the worst of 1 152 rays per bore meets
/// rock at 0.55 r or more (0.84 r for 1.5 u bores), the mean within 0.08 u.
#[test]
fn thin_bores_render_at_their_true_width() {
    for r in [0.6f32, 0.75, 1.0, 1.5] {
        for (name, bore) in bores(r) {
            let d = drawn_radius(&bore);
            println!(
                "r {r:.2} {name:<26} drawn min {:.3} mean {:.3} max {:.3} (missing {})",
                d.min, d.mean, d.max, d.missing
            );
            assert_eq!(d.missing, 0, "r {r} {name}: bore not drawn all along");
            assert!(
                (d.mean - r).abs() <= 0.08,
                "r {r} {name}: mean drawn radius {}",
                d.mean
            );
            let min_share = if r < 1.5 { 0.55 } else { 0.8 };
            assert!(
                d.min >= min_share * r,
                "r {r} {name}: drawn rock {} u from the axis",
                d.min
            );
        }
    }
}

/// A ball of rock (radius 3.5 u, exact field) with a 1.2 u bore drilled into
/// it, like the gallery's `fit-tunnel` scene: the mouth stays on the
/// surface (the crease where the bore meets the ball keeps the grid's own
/// snap; no rock corner next to the mouth is opened).
#[test]
fn bore_mouth_stays_on_the_surface() {
    let c = Vec3::new(16.3, 15.8, 16.2);
    let axis = Vec3::new(0.3, 0.1, 1.0).normalize();
    let entry = c - axis * 5.0;
    let field = |p: Vec3| {
        let ball = p.distance(c) - 3.5;
        let q = p - entry;
        let along = q.dot(axis).clamp(0.0, 6.0);
        let bore = 0.6 - (q - axis * along).length();
        ball.max(bore)
    };
    let grid = SampleGrid::from_fn([0, 0, 0], field);
    let mesh = surface_nets_with_sdf(&grid, &field);
    let mut errs: Vec<(f32, Vec3)> = mesh
        .positions
        .iter()
        .map(|p| {
            let p = Vec3::from_array(*p);
            (field(p).abs(), p)
        })
        .collect();
    errs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (worst, at) = errs[0];
    println!(
        "bore mouth: {} vertices, worst {worst:.3} u from the surface at {at:?}",
        errs.len()
    );
    // The thin vertices sit up to the chord sag (0.11 u for this bore)
    // behind the surface; nothing may float in the bore (before: a vertex
    // 0.5 u off the surface, 0.1 u from the bore's axis).
    assert!(worst < 0.2, "vertex {worst} u off the surface at {at:?}");
    let in_bore = errs
        .iter()
        .filter(|(_, p)| {
            let q = *p - entry;
            let along = q.dot(axis);
            along > 0.0 && along < 6.0 && (q - axis * along).length() < 0.45
        })
        .count();
    assert_eq!(in_bore, 0, "vertices inside the bore");
    // Triangles facing against their vertex normals (folds).
    let p = |i: u32| Vec3::from_array(mesh.positions[i as usize]);
    let nrm = |i: u32| Vec3::from_array(mesh.normals[i as usize]);
    let mut folds = 0;
    for t in mesh.indices.chunks_exact(3) {
        let geo = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        if geo.length() < 1e-6 {
            continue;
        }
        let avg = nrm(t[0]) + nrm(t[1]) + nrm(t[2]);
        if geo.dot(avg) < 0.0 {
            folds += 1;
        }
    }
    let tris = mesh.indices.len() / 3;
    println!("folded triangles at the mouth: {folds} of {tris}");
    // A few folded (culled) triangles where the bore meets the ball: the
    // grid cannot represent the rim, every vertex there is a compromise.
    // Quads split along the diagonal that folds less (D5): 9 of 560, before
    // 14 of 560 with the fixed diagonal. M4 A2 leaves out the lid quads
    // that span the mouth between walls facing each other (`emit_quad`):
    // 5 of 552.
    assert!(folds * 100 <= tris, "{folds} of {tris} triangles folded");
}

/// The gated mesher (the game's Squeeze shell mesher) is the plain one where
/// the thin pass is allowed everywhere, and draws a bore thinner than a cell
/// only partly (or not at all) where it is allowed nowhere.
#[test]
fn thin_pass_gate() {
    let bore = Bore {
        point: Vec3::new(16.5, 16.5, 16.5),
        axis: Vec3::new(1.0, 0.0, 0.0),
        r: 0.6,
    };
    let grid = SampleGrid::from_fn([0, 0, 0], |p| bore.field(p));
    let plain = surface_nets_with_sdf(&grid, &|p| bore.field(p));
    let everywhere = surface_nets_with_sdf_thin_where(&grid, &|p| bore.field(p), &|_| true);
    assert_eq!(plain.positions, everywhere.positions);
    assert_eq!(plain.indices, everywhere.indices);
    assert!(!plain.indices.is_empty(), "the thin pass draws the bore");
    let nowhere = surface_nets_with_sdf_thin_where(&grid, &|p| bore.field(p), &|_| false);
    assert_ne!(plain.positions, nowhere.positions);
    assert!(nowhere.indices.len() < plain.indices.len());
}

/// A blade of rock `2 h` thick with normal `n` through `c`, ending at a
/// straight edge: rock where `|n . (p - c)| < h` and `m . (p - c) < 0`
/// (exact distance field, negative in rock).
struct Blade {
    c: Vec3,
    n: Vec3,
    m: Vec3,
    h: f32,
}

impl Blade {
    fn field(&self, p: Vec3) -> f32 {
        let q = p - self.c;
        let across = q.dot(self.n).abs() - self.h;
        let along = q.dot(self.m);
        // Exact distance to the half-slab (intersection of a slab and a
        // half-space): outside both, the Euclidean distance to the edge.
        if across > 0.0 && along > 0.0 {
            (across * across + along * along).sqrt()
        } else {
            across.max(along)
        }
    }
}

/// Measures a blade: for a grid of points on the blade's mid-plane, a ray
/// from each side along the normal. Inside the blade (at least `margin`
/// from the edge) both rays must hit a face at the true distance (the blade
/// is drawn, both faces); beyond the edge by `margin` neither may hit; and
/// along the edge the drawn outline (the last point inside that is drawn)
/// must stay within `edge_tol` of the true edge.
fn blade_errors(blade: &Blade) -> (usize, usize, f32, f32) {
    let grid = SampleGrid::from_fn([0, 0, 0], |p| blade.field(p));
    let mesh = surface_nets_with_sdf(&grid, &|p| blade.field(p));
    let t = blade.n.cross(blade.m);
    let (mut holes, mut ghosts, mut worst_face, mut worst_edge) = (0, 0, 0.0f32, 0.0f32);
    for i in 0..24 {
        let s = -6.0 + i as f32 * 0.5;
        // Along the edge direction `t`, across the edge along `m`.
        let mut last_drawn = f32::NEG_INFINITY;
        for k in 0..60 {
            let a = -6.0 + k as f32 * 0.1;
            let o = blade.c + t * s + blade.m * a;
            let mut drawn = true;
            for side in [-1.0f32, 1.0] {
                let start = o + blade.n * (side * 3.0);
                match first_hit(&mesh, start, blade.n * -side, 6.0) {
                    Some(d) => {
                        if a < -0.75 {
                            worst_face = worst_face.max((d - (3.0 - blade.h)).abs());
                        }
                    }
                    None => drawn = false,
                }
            }
            if a < -0.75 && !drawn {
                holes += 1;
            }
            if a > 0.75 && drawn {
                ghosts += 1;
            }
            if drawn {
                last_drawn = a;
            }
        }
        worst_edge = worst_edge.max(last_drawn.abs());
    }
    (holes, ghosts, worst_face, worst_edge)
}

/// Thin rock (M4 A2, the M3 D5 "thin-rock jaggies"): a blade thinner than a
/// cell is drawn whole with both faces at their true place, and its edge
/// within half a cell of the true edge, at every offset and slant. Before
/// the thin-rock pass the slanted blades fell through the corners: 260 to
/// 970 of 576 x 2 rays per blade found no face (holes), the faces drawn
/// were up to 0.88 u off (a face on the wrong side of the blade, sheets
/// through the rock), and the outline wandered up to 5 u from the edge.
/// Now: no holes, faces within 0.001 u, outline within 0.5 u.
#[test]
fn thin_rock_blades_are_drawn_whole() {
    let mut state = 0x2545_f491u32;
    let mut rnd = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    for h in [0.2f32, 0.3] {
        for case in 0..6 {
            let n = match case {
                0 => Vec3::X,
                1 => Vec3::new(1.0, 0.3, 0.0).normalize(),
                _ => Vec3::new(1.0, rnd() * 1.2 - 0.6, rnd() * 1.2 - 0.6).normalize(),
            };
            // Edge direction across the blade's plane.
            let m = n.cross(Vec3::Z).normalize();
            let blade = Blade {
                c: Vec3::splat(16.0) + Vec3::new(rnd(), rnd(), rnd()),
                n,
                m,
                h,
            };
            let (holes, ghosts, face, edge) = blade_errors(&blade);
            println!(
                "blade {:.1} u case {case}: holes {holes}, ghosts {ghosts}, worst face {face:.3} u, worst edge {edge:.3} u",
                2.0 * h
            );
            assert_eq!(holes, 0, "blade {} case {case}: holes", 2.0 * h);
            assert_eq!(
                ghosts,
                0,
                "blade {} case {case}: drawn beyond the edge",
                2.0 * h
            );
            assert!(
                face <= 0.02,
                "blade {} case {case}: face off by {face}",
                2.0 * h
            );
            assert!(
                edge <= 0.5,
                "blade {} case {case}: edge off by {edge}",
                2.0 * h
            );
        }
    }
}
