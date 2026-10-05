// Cockpit hull frames (visual-quality-plan.md VQ1): what the pilot sees of
// the own ship, one per ship type, built with the VQ4 modelling kit and drawn
// by hullFrame.ts in view space (camera at the origin, forward -Z, up +Y,
// right +X; 1 u = one ship length at the Tinker's radius).
//
// The eye sits above the hull's centre line, so the nose deck lies below the
// crosshair: the nose and its side parts (Tinker engine pods with guns,
// Hauler armour plates and floodlights, Tug grapple arms and winch) use the
// VQ4 hull's dimensions, so the frame shows the ship's real length and
// width. Canopy pillars frame the screen edges. Everything is kept out of the
// middle of the view: the nose tip stays below about 15 degrees under the
// crosshair (hullFrame.test.ts).
//
// Coordinates below are in the VQ4 builders' raw hull units (ships.ts before
// `finish` fits them to HULL_RADIUS); `frameModel` moves the eye to the
// origin and applies the same fit, so the frame matches the drawn hull.

import {
  chain,
  hullRing,
  Model,
  PART_HULL,
  PART_LAMP,
  PART_LIVERY,
  PART_METAL,
  PART_NAV_PORT,
  PART_NAV_STARBOARD,
  PART_TRIM,
  rotate,
  scale,
  translate,
  type PartFn,
  type V3,
} from "./models/kit.ts";
import { hullOf } from "./models/ships.ts";

/** Eye position (raw hull units) per ship type: above the nose, at the hull's centre plane. */
export const FRAME_EYES: readonly V3[] = [
  [0, 0.19, 0], // Skitter
  [0, 0.2, 0], // Tinker
  [0, 0.3, -0.02], // Hauler
  [0, 0.2, 0], // Tug
];

/**
 * Fit scale of the VQ4 hull (raw units -> HULL_RADIUS units): the frame's
 * hull parts include the hull's foremost point (nose, bumper or claws), so
 * the ratio of the two noses is the fit.
 */
export function hullFit(shipType: number): number {
  const t = shipType >= 0 && shipType < BUILDERS.length ? shipType : 1;
  return hullOf(t).model.bounds().min[2] / BUILDERS[t]!().hull.bounds().min[2];
}

const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (a: V3): V3 => {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};

/**
 * A bevelled beam along a polyline (eye coordinates): rectangular section
 * `w` wide (the face turned toward the eye) and `d` deep, chamfered.
 * Parts per segment: the faces toward the eye get `face`, the rest `edge`.
 */
function pillar(m: Model, path: readonly V3[], w: number, d: number, face: number, edge: number): void {
  const rings: V3[][] = [];
  const bevel = Math.min(w, d) * 0.3;
  path.forEach((p, i) => {
    const prev = path[Math.max(0, i - 1)]!;
    const next = path[Math.min(path.length - 1, i + 1)]!;
    const t = norm(sub(next, prev));
    const toEye = norm([-p[0], -p[1], -p[2]]);
    const u = norm(cross(t, toEye));
    const v = norm(cross(u, t));
    // Chamfered rectangle in (u, v): v points toward the eye.
    const hw = w / 2, hd = d / 2;
    const pts: [number, number][] = [
      [hw, hd - bevel], [hw - bevel, hd], [-hw + bevel, hd], [-hw, hd - bevel],
      [-hw, -hd + bevel], [-hw + bevel, -hd], [hw - bevel, -hd], [hw, -hd + bevel],
    ];
    rings.push(pts.map(([a, b]): V3 => [p[0] + u[0] * a + v[0] * b, p[1] + u[1] * a + v[1] * b, p[2] + u[2] * a + v[2] * b]));
  });
  // Side 1 (between the two corners toward the eye) faces the eye; its
  // chamfers (sides 0 and 2) catch the light as thin metal edges.
  m.loft(rings, {
    part: (_seg, side) => (side === 1 ? face : side === 0 || side === 2 ? PART_METAL : edge),
    group: m.smooth(50),
    capStart: true,
    capEnd: true,
  });
}

/** Both canopy pillars from a starboard path (eye coordinates). */
function pillars(m: Model, path: readonly V3[], w: number, d: number, face = PART_TRIM, edge = PART_METAL): void {
  pillar(m, path, w, d, face, edge);
  pillar(m, path.map(([x, y, z]): V3 => [-x, y, z]), w, d, face, edge);
}

/** Hull deck: the front of a VQ4 fuselage loft (raw hull units). */
function deck(m: Model, rings: V3[][], part: PartFn): void {
  m.loft(rings, { part, group: m.smooth(42), capStart: true, capEnd: true });
}

/** Screen fraction (half-width, half-height at 100° horizontal, 16:9) at `depth` u ahead to eye coordinates. */
function screenAt(sx: number, sy: number, depth: number, spread = 1): V3 {
  const tx = Math.tan(50 * (Math.PI / 180));
  const ty = tx / (16 / 9);
  return [sx * spread * tx * depth, sy * ty * depth, -depth];
}

/**
 * Canopy pillars through fixed screen positions, so every hull frames the
 * view the same way: (sx, sy) are fractions of the half-width and
 * half-height at 100° horizontal (16:9), `depth` the distance ahead (u).
 * They rise from the bottom edge at about a fifth of the width to the top
 * corners and then curve overhead out of view. Each pillar is a dark frame
 * beam with a lighter metal rail along its inner edge (the glass seal), so
 * it reads as a machined canopy frame and not as a stray line (VQP).
 */
function canopyPillars(m: Model, w: number, d: number, spread = 1): void {
  const at = (sx: number, sy: number, depth: number): V3 => screenAt(sx, sy, depth, spread);
  const path: V3[] = [at(0.6, -1.08, 0.27), at(0.77, -0.25, 0.24), at(0.89, 0.45, 0.2), at(1.02, 1.08, 0.16), [0.13 * spread, 0.17, -0.05]];
  pillars(m, path, w, d, PART_TRIM, PART_HULL);
  // The rail: thinner, nearer the eye, on the inner (screen-centre) side.
  const rail = path.slice(0, 4).map(([x, y, z]): V3 => {
    const k = 1 - (w * 0.55) / Math.max(1e-6, Math.hypot(x, y));
    return [x * k, y * k, z * 0.985];
  });
  pillars(m, rail, w * 0.32, d * 0.5, PART_METAL, PART_METAL);
}

/**
 * The canopy bow: an arch across the top of the view at screen height
 * `sy` (its middle slightly higher), with corner gussets down to the
 * pillars, so pillars and bow read as one moulded window frame.
 */
function canopyBow(m: Model, sy: number, w: number, d: number, spread = 1): void {
  const depth = 0.18;
  const pts: V3[] = [];
  for (const sx of [-1.25, -0.9, -0.6, -0.3, 0, 0.3, 0.6, 0.9, 1.25]) pts.push(screenAt(sx, sy + 0.06 * (1 - sx * sx), depth, spread));
  pillar(m, pts, w, d, PART_TRIM, PART_HULL);
  // Inner rail along the bow's lower edge.
  const ty = Math.tan(50 * (Math.PI / 180)) / (16 / 9);
  const rail = pts.map(([x, y, z]): V3 => [x, y - w * 0.55 * (ty / ty), z * 0.985]);
  pillar(m, rail, w * 0.32, d * 0.5, PART_METAL, PART_METAL);
  // Rounded window corners where the bow meets the pillars: a quarter arc
  // from the bow down into each pillar (the window reads as one frame).
  for (const s of [-1, 1]) {
    const arc: V3[] = [];
    for (let i = 0; i <= 6; i++) {
      const a = (i / 6) * (Math.PI / 2);
      // Corner centre inside the window; the arc bulges toward the screen corner.
      const sx = 0.72 + 0.22 * Math.sin(a);
      const sy2 = sy - 0.34 + 0.34 * Math.cos(a) + 0.05;
      arc.push(screenAt(sx * s, sy2, depth * (1.0 + 0.06 * Math.sin(a)), spread));
    }
    pillar(m, arc, w * 0.9, d, PART_TRIM, PART_HULL);
  }
}

/** A heavy visor beam across the top of the view at screen height `sy` (Hauler). */
function topBow(m: Model, sy: number, w: number, d: number, part = PART_HULL): void {
  const tx = Math.tan(50 * (Math.PI / 180));
  const ty = tx / (16 / 9);
  const depth = 0.18;
  const pts: V3[] = [];
  for (const sx of [-1.25, -0.6, 0, 0.6, 1.25]) pts.push([sx * tx * depth, (sy + 0.05 * (1 - Math.abs(sx))) * ty * depth, -depth]);
  pillar(m, pts, w, d, part, PART_HULL);
}

/** Skitter: the needle nose with its spine stripes and chin guns, a bubble canopy's thin pillars. */
function skitterFrame(): { hull: Model; eye: Model } {
  const N = 16;
  const hull = new Model();
  deck(
    hull,
    [
      hullRing(-0.58, 0.004, 0.003, 0.003, N, 2.2),
      hullRing(-0.5, 0.026, 0.022, 0.018, N, 2.2),
      hullRing(-0.34, 0.052, 0.045, 0.034, N, 2.2),
      hullRing(-0.16, 0.07, 0.06, 0.044, N, 2.2),
      hullRing(0.02, 0.08, 0.066, 0.05, N, 2.2),
    ],
    (seg, side) => (seg === 0 ? PART_TRIM : side === 2 || side === 5 ? PART_LIVERY : side >= 10 && side <= 13 ? PART_TRIM : PART_HULL),
  );
  // Gun barrels along the chin, poking out at the sides of the nose.
  for (const x of [-0.068, 0.068]) {
    hull.lathe([[0, -0.3], [0.007, -0.29], [0.008, -0.05], [0.012, 0.0]], 8, { center: [x, -0.03, 0], part: PART_METAL, group: hull.smooth(60), capEnd: true });
  }
  const eye = new Model();
  canopyPillars(eye, 0.014, 0.012, 1.02);
  canopyBow(eye, 0.96, 0.011, 0.01, 1.02);
  return { hull, eye };
}

/** Tinker: the wedge nose, both engine pods with their guns, framed windscreen. */
function tinkerFrame(): { hull: Model; eye: Model } {
  const N = 20;
  const hull = new Model();
  deck(
    hull,
    [
      hullRing(-0.56, 0.03, 0.008, 0.008, N, 3),
      hullRing(-0.48, 0.07, 0.022, 0.018, N, 3),
      hullRing(-0.32, 0.12, 0.045, 0.034, N, 3),
      hullRing(-0.14, 0.16, 0.075, 0.05, N, 3),
      hullRing(0.04, 0.18, 0.09, 0.06, N, 3),
    ],
    (seg, side) => (seg >= 1 && (side === 3 || side === 6) ? PART_LIVERY : side >= 13 && side <= 16 ? PART_TRIM : PART_HULL),
  );
  for (const s of [-1, 1]) {
    const x = 0.255 * s;
    hull.lathe(
      [[0, -0.33], [0.03, -0.31], [0.055, -0.24], [0.066, -0.14], [0.068, 0.06]],
      14,
      { center: [x, -0.01, 0], sy: 0.9, part: (seg) => (seg === 0 ? PART_METAL : seg === 2 ? PART_LIVERY : PART_HULL), group: hull.smooth(40), capEnd: true },
    );
    hull.lathe([[0, -0.42], [0.009, -0.41], [0.01, -0.3]], 8, { center: [x, -0.055, 0], part: PART_METAL, group: hull.smooth(60), capEnd: true });
    hull.sphere([x + 0.068 * s, 0, -0.05], 0.016, 1, s > 0 ? PART_NAV_STARBOARD : PART_NAV_PORT, hull.smooth(80));
  }
  const eye = new Model();
  canopyPillars(eye, 0.02, 0.016);
  canopyBow(eye, 0.92, 0.016, 0.014);
  return { hull, eye };
}

/** Hauler: the broad armoured deck with bumper bars and floodlights, side plates, a heavy visor. */
function haulerFrame(): { hull: Model; eye: Model } {
  const N = 16;
  const block = (z: number, rx: number, up: number, down: number): V3[] => hullRing(z, rx, up, down, N, 7);
  const hull = new Model();
  deck(hull, [block(-0.5, 0.11, 0.07, 0.06), block(-0.44, 0.17, 0.12, 0.1), block(-0.36, 0.2, 0.14, 0.12), block(-0.1, 0.21, 0.145, 0.125)], (_seg, side) =>
    side >= 11 && side <= 13 ? PART_TRIM : PART_HULL,
  );
  for (const y of [-0.04, 0.03]) hull.box([0, y, -0.505], [0.08, 0.012, 0.015], { part: PART_METAL, bevel: 0.004 });
  for (const x of [-0.13, 0.13]) hull.box([x, 0.09, -0.45], [0.02, 0.014, 0.01], { part: PART_LAMP, bevel: 0.004 });
  // Livery stripes along the deck edges and a centre walkway plate.
  for (const s of [-1, 1]) hull.box([0.165 * s, 0.142, -0.25], [0.008, 0.003, 0.12], { part: PART_LIVERY, bevel: 0.002 });
  hull.box([0, 0.142, -0.27], [0.05, 0.003, 0.1], { part: PART_METAL, bevel: 0.003 });
  for (const s of [-1, 1]) {
    const piece = new Model();
    piece.box([0.235, 0.0, -0.05], [0.028, 0.115, 0.22], { part: PART_HULL, bevel: 0.016 });
    piece.box([0.266, 0.075, -0.05], [0.006, 0.016, 0.18], { part: PART_LIVERY, bevel: 0.003 });
    for (const z of [-0.2, -0.07]) piece.box([0.266, 0.0, z], [0.006, 0.012, 0.012], { part: PART_METAL });
    piece.sphere([0.27, 0.05, -0.26], 0.018, 1, PART_NAV_STARBOARD, piece.smooth(80));
    if (s > 0) hull.add(piece);
    else hull.addMirrorX(piece);
  }
  const eye = new Model();
  canopyPillars(eye, 0.026, 0.02);
  topBow(eye, 0.9, 0.024, 0.02, PART_TRIM);
  canopyBow(eye, 0.84, 0.012, 0.012);
  return { hull, eye };
}

/** Tug: the wide body edge, the nose winch and both grapple arms with their claws. */
function tugFrame(): { hull: Model; eye: Model } {
  const N = 24;
  const hull = new Model();
  deck(
    hull,
    [hullRing(-0.27, 0.16, 0.04, 0.035, N, 2.6), hullRing(-0.2, 0.25, 0.08, 0.06, N, 2.6), hullRing(-0.05, 0.29, 0.1, 0.075, N, 2.6)],
    (_seg, side) => (side >= 16 && side <= 20 ? PART_TRIM : side === 4 || side === 7 ? PART_LIVERY : PART_HULL),
  );
  hull.lathe([[0, -0.4], [0.03, -0.39], [0.045, -0.35], [0.06, -0.33], [0.06, -0.29], [0.07, -0.26]], 14, {
    part: (seg) => (seg === 3 ? PART_LIVERY : PART_METAL),
    group: hull.smooth(40),
    capEnd: true,
  });
  for (const x of [-0.11, 0.11]) hull.box([x, 0.04, -0.24], [0.022, 0.014, 0.012], { part: PART_LAMP, bevel: 0.004 });
  for (const s of [-1, 1]) {
    const arm = new Model();
    const shoulder: V3 = [0.2, -0.01, -0.12];
    const elbow: V3 = [0.31, -0.01, -0.3];
    const wrist: V3 = [0.2, -0.01, -0.44];
    arm.sphere(shoulder, 0.04, 1, PART_METAL, arm.smooth(60));
    beam(arm, shoulder, elbow, 0.05, 0.045, PART_HULL, 0.008);
    arm.sphere(elbow, 0.036, 1, PART_LIVERY, arm.smooth(60));
    beam(arm, elbow, wrist, 0.04, 0.038, PART_TRIM, 0.006);
    arm.sphere(wrist, 0.03, 1, PART_METAL, arm.smooth(60));
    for (const [dx, dy] of [[-0.035, 0], [0.02, 0.03], [0.02, -0.03]] as const) {
      beam(arm, wrist, [wrist[0] + dx, wrist[1] + dy, wrist[2] - 0.065], 0.014, 0.014, PART_METAL, 0);
    }
    beam(arm, [0.215, 0.03, -0.14], [0.3, 0.03, -0.27], 0.014, 0.014, PART_METAL, 0);
    if (s > 0) hull.add(arm);
    else hull.addMirrorX(arm);
  }
  const eye = new Model();
  canopyPillars(eye, 0.022, 0.018);
  canopyBow(eye, 0.94, 0.018, 0.016);
  return { hull, eye };
}

/** Box from `a` to `b` with a `w` x `h` section (ships.ts `strut`). */
function beam(m: Model, a: V3, b: V3, w: number, h: number, part: number, bevel = 0.004): void {
  const d = sub(b, a);
  const len = Math.hypot(d[0], d[1], d[2]);
  const yaw = Math.atan2(d[0], d[2]);
  const pitch = -Math.asin(d[1] / (len || 1));
  const piece = new Model().box([0, 0, 0], [w / 2, h / 2, len / 2], { part, bevel });
  m.add(piece, chain(rotate("x", pitch), rotate("y", yaw), translate([(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2])));
}

const BUILDERS = [skitterFrame, tinkerFrame, haulerFrame, tugFrame];

/**
 * The cockpit frame of a ship type in view space at the Tinker's size
 * (scale it by `radius / TINKER_RADIUS`): the hull parts moved so the eye is
 * at the origin, plus the canopy pillars (eye coordinates), both fitted like
 * the drawn hull.
 */
export function frameModel(shipType: number): Model {
  const t = shipType >= 0 && shipType < BUILDERS.length ? shipType : 1;
  const { hull, eye } = BUILDERS[t]!();
  const e = FRAME_EYES[t]!;
  const all = new Model().add(hull, translate([-e[0], -e[1], -e[2]])).add(eye);
  return all.transformed(scale(hullFit(t)));
}
