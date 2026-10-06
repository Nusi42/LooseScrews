// Props of M4 C3's spectacle events, built with the VQ4 modelling kit
// (milestone-4-plan.md §3.2, wave C "C3"): the giant rubber duck (Duck
// parade), eight hats (Hat rain), the cat paw with its arm and claws (Cat
// paw), the mirror ball and its chain (Disco inferno) and a light-trail
// segment. Materials are spectacleShading.ts's parts: glossy rubber, fur,
// felt, mirror tiles, gold, ivory.
//
// Frames (model units; game/events/*.ts scale and place them):
// - duck: faces -Z, up +Y, body centre at the origin, half width 0.76;
// - hats: sit on y = 0 with up +Y, brims about 0.5 in radius, front -Z;
// - paw: palm (toe beans) toward +X, toes toward -Y, the toe row along Z,
//   the wrist toward +Z; the pad sphere has radius 1 (instance scale = the
//   sim's pad radius); the arm runs from z 0 to 1 (stretched to the
//   shoulder); a claw hangs from its root along -Y, hooking toward +X;
// - disco ball: radius 1 about the origin; the chain runs from z 0 to 1;
// - trail segment: a capsule along Z from -0.5 to 0.5, radius 0.5.

import { Model, PART_GLOW, PART_METAL, type V3 } from "./kit.ts";
import {
  PART_BEAN,
  PART_FELT,
  PART_FUR,
  PART_GEM,
  PART_GOLD,
  PART_INK,
  PART_IVORY,
  PART_MIRROR,
  PART_ORANGE,
  PART_RUBBER,
  PART_SHINE,
  PART_WHITE,
} from "./spectacleShading.ts";

const smoothstep = (a: number, b: number, x: number): number => {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a)));
  return t * t * (3 - 2 * t);
};

/** A closed ring of `n` points around a centre in the plane spanned by `u` and `w`. */
function ringAround(c: V3, u: V3, w: V3, r: number, n: number, sx = 1): V3[] {
  const out: V3[] = [];
  for (let j = 0; j < n; j++) {
    const a = (j / n) * Math.PI * 2;
    const cu = Math.cos(a) * r * sx;
    const sw = Math.sin(a) * r;
    out.push([c[0] + u[0] * cu + w[0] * sw, c[1] + u[1] * cu + w[1] * sw, c[2] + u[2] * cu + w[2] * sw]);
  }
  return out;
}

const norm = (a: V3): V3 => {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};
const crossV = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];

/**
 * A tube along a curve `c(t)`, t 0..1, with radius `r(t)` (tapered horns,
 * claws, tassels): rings perpendicular to the curve, closed at both ends.
 */
function tube(m: Model, c: (t: number) => V3, r: (t: number) => number, steps: number, sides: number, part: number, group: number, sx = 1): void {
  const rings: V3[][] = [];
  let prevU: V3 | null = null;
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    const p = c(t);
    const q = c(Math.min(1, t + 1e-3));
    const o = c(Math.max(0, t - 1e-3));
    const tan = norm([q[0] - o[0], q[1] - o[1], q[2] - o[2]]);
    // A stable frame: the previous ring's u projected off the tangent.
    let u: V3 = prevU ?? (Math.abs(tan[2]) < 0.9 ? [0, 0, 1] : [1, 0, 0]);
    const d = u[0] * tan[0] + u[1] * tan[1] + u[2] * tan[2];
    u = norm([u[0] - tan[0] * d, u[1] - tan[1] * d, u[2] - tan[2] * d]);
    prevU = u;
    const w = crossV(tan, u);
    rings.push(ringAround(p, u, w, Math.max(r(t), 0.0004), sides, sx));
  }
  m.loft(rings, { part, group, capStart: true, capEnd: true });
}

// ----- Duck parade ---------------------------------------------------------------

/** Body rings of the duck: [z, half width, half height, centre y]. */
const DUCK_BODY: ReadonlyArray<readonly [number, number, number, number]> = [
  [-0.82, 0.04, 0.04, 0.02],
  [-0.76, 0.34, 0.3, 0.0],
  [-0.62, 0.56, 0.45, -0.02],
  [-0.4, 0.7, 0.53, -0.04],
  [-0.1, 0.76, 0.56, -0.05],
  [0.2, 0.74, 0.54, -0.03],
  [0.45, 0.64, 0.47, 0.02],
  [0.66, 0.46, 0.36, 0.1],
  [0.82, 0.26, 0.22, 0.22],
  [0.92, 0.1, 0.1, 0.33],
  [0.96, 0.02, 0.02, 0.38],
];

/** The giant rubber duck (Duck parade): body, head, beak, eyes with catchlights, wings. */
export function rubberDuckModel(): Model {
  const m = new Model();
  const g = m.smooth(60);
  // Body: superellipse rings (flat-ish bottom, round top) along Z, the tail raised.
  const rings = DUCK_BODY.map(([z, rx, ry, cy]) => {
    const pts: V3[] = [];
    const n = 28;
    for (let j = 0; j < n; j++) {
      const a = (j / n) * Math.PI * 2;
      const c = Math.cos(a), s = Math.sin(a);
      // Fuller below (a boat-like hull), rounder above.
      const yy = s < 0 ? -Math.pow(-s, 0.75) * ry * 0.9 : s * ry;
      pts.push([c * rx, cy + yy, z]);
    }
    return pts;
  });
  m.loft(rings, { part: PART_RUBBER, group: g, capStart: true, capEnd: true });
  // Head with a little cheek puff, sitting forward on the chest.
  m.sphere([0, 0.66, -0.46], [0.4, 0.38, 0.4], 2, PART_RUBBER, g);
  m.sphere([0, 0.5, -0.52], [0.34, 0.2, 0.3], 2, PART_RUBBER, g);
  // Beak: upper and lower halves, a faint smile.
  const beak = m.smooth(70);
  m.sphere([0, 0.585, -0.86], [0.22, 0.075, 0.2], 2, PART_ORANGE, beak);
  m.sphere([0, 0.52, -0.82], [0.17, 0.05, 0.15], 1, PART_ORANGE, beak);
  // Eyes: glossy black with a white catchlight each.
  const eye = m.smooth(80);
  for (const s of [-1, 1]) {
    m.sphere([s * 0.17, 0.78, -0.76], 0.075, 2, PART_INK, eye);
    m.sphere([s * 0.175 + s * 0.012, 0.815, -0.825], 0.022, 1, PART_SHINE, eye);
  }
  // Wings: flattened ellipsoids on the sides, tilted tail-up.
  for (const s of [-1, 1]) {
    const w = new Model();
    w.sphere([0, 0, 0], [0.12, 0.26, 0.44], 2, PART_RUBBER, w.smooth(60));
    m.add(w, (p) => {
      const a = -0.3;
      const y = p[1] * Math.cos(a) - p[2] * Math.sin(a);
      const z = p[1] * Math.sin(a) + p[2] * Math.cos(a);
      return [s * 0.66 + p[0] * s, 0.1 + y, 0.12 + z];
    });
  }
  return m;
}

/** Cool sunglasses for the last duck of the parade (same frame as the duck). */
export function duckShadesModel(): Model {
  const m = new Model();
  const g = m.smooth(70);
  for (const s of [-1, 1]) m.sphere([s * 0.17, 0.79, -0.815], [0.13, 0.085, 0.035], 2, PART_INK, g);
  m.box([0, 0.83, -0.83], [0.06, 0.012, 0.012], { part: PART_INK });
  for (const s of [-1, 1]) m.box([s * 0.3, 0.8, -0.62], [0.012, 0.012, 0.2], { part: PART_INK });
  return m;
}

// ----- Hat rain ------------------------------------------------------------------

/** Hat styles (`ReplicatedKind::Hat` state[0] 0-7). */
export const HAT_STYLES = ["top hat", "cowboy hat", "party cone", "chef's toque", "crown", "propeller beanie", "fez", "viking helmet"] as const;

/** Height of each hat style (model units): where the next hat of a stack sits. */
export const HAT_HEIGHTS: readonly number[] = [0.58, 0.42, 0.82, 0.62, 0.3, 0.4, 0.32, 0.36];

/** Felt colour of each style (display; the instance colour of PART_FELT). */
export const HAT_COLORS: ReadonlyArray<readonly [number, number, number]> = [
  [0.75, 0.06, 0.1], // top hat band
  [0.34, 0.19, 0.08], // cowboy
  [0.95, 0.2, 0.55], // party cone stripes
  [0.9, 0.9, 0.88], // chef (unused: cloth)
  [0.6, 0.05, 0.12], // crown cushion
  [0.12, 0.4, 0.95], // beanie
  [0.78, 0.07, 0.07], // fez
  [0.5, 0.5, 0.5], // viking (unused: metal)
];

function topHat(): Model {
  const m = new Model();
  const profile: Array<[number, number]> = [
    [0.001, 0.0],
    [0.46, 0.0],
    [0.53, 0.025],
    [0.5, 0.045],
    [0.3, 0.045],
    [0.288, 0.06],
    [0.292, 0.16],
    [0.312, 0.55],
    [0.318, 0.575],
    [0.0015, 0.58],
  ];
  m.lathe(profile, 28, { axis: "y", group: m.smooth(45), part: (seg) => (seg === 5 ? PART_FELT : PART_INK) });
  return m;
}

function cowboyHat(): Model {
  const base = new Model();
  const profile: Array<[number, number]> = [
    [0.001, 0.0],
    [0.5, 0.0],
    [0.62, 0.02],
    [0.6, 0.045],
    [0.3, 0.05],
    [0.28, 0.065],
    [0.28, 0.12],
    [0.27, 0.3],
    [0.2, 0.42],
    [0.1, 0.4],
    [0.0015, 0.36],
  ];
  base.lathe(profile, 32, { axis: "y", group: base.smooth(55), part: (seg) => (seg === 5 ? PART_INK : PART_FELT) });
  // Curl the brim up at the sides, dip it at front and back.
  return base.transformed(([x, y, z]) => {
    const r = Math.hypot(x, z);
    const k = smoothstep(0.3, 0.62, r);
    const side = (x * x) / Math.max(r * r, 1e-6);
    return [x, y + k * (0.16 * side * (r - 0.3) * 3 - 0.03 * (1 - side)), z];
  });
}

function partyCone(): Model {
  const m = new Model();
  const profile: Array<[number, number]> = [[0.001, 0.0]];
  const steps = 9;
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    profile.push([0.3 * (1 - t) + 0.002, 0.78 * t]);
  }
  // Spiral stripes: felt and white bands winding up the cone.
  m.lathe(profile, 24, { axis: "y", group: m.smooth(40), part: (seg, side) => ((Math.floor(side / 3) + seg) % 2 === 0 ? PART_FELT : PART_WHITE) });
  // Pom-pom and a frilly rim.
  m.sphere([0, 0.8, 0], 0.085, 1, PART_GOLD, m.smooth(80));
  const frill = m.smooth(60);
  for (let i = 0; i < 12; i++) {
    const a = (i / 12) * Math.PI * 2;
    m.sphere([Math.cos(a) * 0.3, 0.02, Math.sin(a) * 0.3], [0.07, 0.04, 0.07], 1, PART_WHITE, frill);
  }
  return m;
}

function chefHat(): Model {
  const m = new Model();
  const g = m.smooth(55);
  m.lathe(
    [
      [0.001, 0.0],
      [0.27, 0.0],
      [0.272, 0.06],
      [0.27, 0.34],
      [0.001, 0.34],
    ],
    24,
    { axis: "y", group: g, part: PART_WHITE },
  );
  // The puffed top: a ring of pleats and a dome.
  const puff = m.smooth(70);
  for (let i = 0; i < 7; i++) {
    const a = (i / 7) * Math.PI * 2;
    m.sphere([Math.cos(a) * 0.19, 0.42, Math.sin(a) * 0.19], [0.17, 0.15, 0.17], 1, PART_WHITE, puff);
  }
  m.sphere([0, 0.48, 0], [0.22, 0.14, 0.22], 2, PART_WHITE, puff);
  return m;
}

function crown(): Model {
  const m = new Model();
  const n = 48;
  const peaks = 8;
  const zig = (j: number): number => {
    const f = ((j / n) * peaks) % 1;
    return 0.11 + 0.17 * (1 - Math.abs(f * 2 - 1));
  };
  const ring = (r: number, top: boolean): V3[] =>
    Array.from({ length: n }, (_, j) => {
      const a = (j / n) * Math.PI * 2;
      return [Math.cos(a) * r, top ? zig(j) : 0, Math.sin(a) * r] as V3;
    });
  // A thin gold band with a zigzag top (outer wall, top edge, inner wall, bottom).
  m.loft([ring(0.3, false), ring(0.3, true), ring(0.27, true), ring(0.27, false), ring(0.3, false)], { part: PART_GOLD, group: m.smooth(35) });
  // Velvet cushion inside.
  m.sphere([0, 0.08, 0], [0.265, 0.17, 0.265], 2, PART_FELT, m.smooth(70));
  const jewels = m.smooth(80);
  for (let i = 0; i < peaks; i++) {
    const a = ((i + 0.5) / peaks) * Math.PI * 2;
    m.sphere([Math.cos(a) * 0.31, 0.27, Math.sin(a) * 0.31], 0.028, 1, PART_GOLD, jewels);
    if (i % 2 === 0) m.sphere([Math.cos(a + 0.4) * 0.305, 0.065, Math.sin(a + 0.4) * 0.305], [0.04, 0.035, 0.04], 1, PART_GEM, jewels);
  }
  return m;
}

function propellerBeanie(): Model {
  const m = new Model();
  m.lathe(
    [
      [0.31, 0.0],
      [0.3, 0.09],
      [0.25, 0.2],
      [0.14, 0.27],
      [0.0015, 0.29],
    ],
    24,
    { axis: "y", group: m.smooth(60), part: (_seg, side) => (Math.floor(side / 3) % 2 === 0 ? PART_FELT : PART_ORANGE), capStart: true },
  );
  // Peak at the front, a button and the propeller's stem.
  m.sphere([0, 0.02, -0.33], [0.2, 0.022, 0.12], 2, PART_FELT, m.smooth(60));
  m.lathe(
    [
      [0.018, 0.27],
      [0.018, 0.38],
    ],
    10,
    { axis: "y", part: PART_GOLD, group: m.smooth(60), capEnd: true },
  );
  return m;
}

/** The beanie's propeller (spins about +Y; placed at the stem's top, y 0.38). */
export function hatPropellerModel(): Model {
  const m = new Model();
  const g = m.smooth(60);
  for (const s of [-1, 1]) {
    const b = new Model();
    b.sphere([0, 0, 0], [0.22, 0.012, 0.055], 2, PART_ORANGE, b.smooth(60));
    // A slight pitch, opposite on each blade.
    m.add(b, ([x, y, z]) => [x + s * 0.2, y + z * 0.35 * s, z]);
  }
  m.sphere([0, 0, 0], 0.035, 1, PART_GOLD, g);
  return m;
}

function fez(): Model {
  const m = new Model();
  m.lathe(
    [
      [0.001, 0.0],
      [0.27, 0.0],
      [0.272, 0.02],
      [0.232, 0.32],
      [0.001, 0.32],
    ],
    24,
    { axis: "y", group: m.smooth(45), part: PART_FELT },
  );
  const g = m.smooth(60);
  // Tassel: a cord over the top and a golden tuft.
  tube(m, (t) => [0.2 * t, 0.33 - 0.12 * t * t, 0.04 * t], () => 0.01, 6, 6, PART_INK, g);
  tube(m, (t) => [0.2 + 0.01 * t, 0.21 - 0.14 * t, 0.04], (t) => 0.02 + 0.03 * t, 4, 10, PART_GOLD, m.smooth(70));
  return m;
}

function vikingHelmet(): Model {
  const m = new Model();
  m.lathe(
    [
      [0.32, 0.0],
      [0.31, 0.1],
      [0.26, 0.22],
      [0.15, 0.31],
      [0.0015, 0.335],
    ],
    28,
    { axis: "y", group: m.smooth(60), part: PART_METAL, capStart: true },
  );
  m.lathe(
    [
      [0.33, -0.005],
      [0.335, 0.03],
      [0.33, 0.065],
    ],
    28,
    { axis: "y", group: m.smooth(60), part: PART_GOLD },
  );
  const rivets = m.smooth(80);
  for (let i = 0; i < 10; i++) {
    const a = (i / 10) * Math.PI * 2;
    m.sphere([Math.cos(a) * 0.336, 0.03, Math.sin(a) * 0.336], 0.016, 0, PART_GOLD, rivets);
  }
  // Horns: tapering, curving out and up.
  const horn = m.smooth(60);
  for (const s of [-1, 1]) {
    tube(m, (t) => [s * (0.26 + 0.34 * t - 0.06 * t * t), 0.14 + 0.02 * t + 0.34 * t * t, -0.05 * t], (t) => 0.08 * (1 - t) + 0.006, 12, 12, PART_IVORY, horn);
  }
  return m;
}

/** The eight hats in style order (`HAT_STYLES`). */
export const HAT_MODELS: ReadonlyArray<() => Model> = [topHat, cowboyHat, partyCone, chefHat, crown, propellerBeanie, fez, vikingHelmet];

// ----- Cat paw -------------------------------------------------------------------

/** Toe centres of the paw (model units; the toe row along Z on the -Y side). */
export const PAW_TOES: ReadonlyArray<V3> = [
  [0.02, -0.6, -0.55],
  [0.04, -0.78, -0.19],
  [0.04, -0.78, 0.19],
  [0.02, -0.6, 0.55],
];

/** A tuft of fur: a short curved cone from `p` along `n` (cartoon fur clumps). */
function tuft(m: Model, p: V3, n: V3, len: number, rb: number, bend: V3, group: number): void {
  tube(
    m,
    (t) => [p[0] + (n[0] * len + bend[0] * len * t * 0.5) * t, p[1] + (n[1] * len + bend[1] * len * t * 0.5) * t, p[2] + (n[2] * len + bend[2] * len * t * 0.5) * t],
    (t) => rb * (1 - t) + 0.002,
    2,
    5,
    PART_FUR,
    group,
  );
}

/** Fluffy fur blob: a smooth ellipsoid with clumps of fur poking out of its surface. */
function furBlob(m: Model, c: V3, r: V3, tufts: number, seed: number, skip: (p: V3) => boolean = () => false): void {
  const g = m.smooth(70);
  m.sphere(c, r, 2, PART_FUR, g);
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < tufts; i++) {
    const y = 1 - ((i + 0.5) / tufts) * 2;
    const rr = Math.sqrt(1 - y * y);
    const a = i * golden + seed;
    const d: V3 = [Math.cos(a) * rr, y, Math.sin(a) * rr];
    const p: V3 = [c[0] + d[0] * r[0] * 0.9, c[1] + d[1] * r[1] * 0.9, c[2] + d[2] * r[2] * 0.9];
    if (skip(p)) continue;
    const nrm = norm([d[0] / r[0], d[1] / r[1], d[2] / r[2]]);
    const h = ((i * 7919 + Math.round(seed * 31)) % 13) / 13;
    // Fur lies back toward the wrist (+Z) a little.
    tuft(m, p, nrm, 0.09 + 0.06 * h, 0.085 + 0.03 * h, [0, 0.15, 0.6], g);
  }
}

/** The paw: a furry mitten with four toes, pink toe beans and a heart-shaped pad on the palm (+X). */
export function catPawModel(): Model {
  const m = new Model();
  const palmSide = (p: V3): boolean => p[0] > 0.22;
  furBlob(m, [0, 0.05, 0.05], [0.55, 0.72, 0.86], 150, 0.4, palmSide);
  const toe = m.smooth(70);
  for (const t of PAW_TOES) {
    m.sphere(t, [0.44, 0.34, 0.25], 2, PART_FUR, toe);
    // Fur on the toe's back.
    for (let k = 0; k < 6; k++) {
      const a = (k / 6) * Math.PI * 1.4 - Math.PI * 0.7;
      const d = norm([-0.7, Math.sin(a) * 0.8 - 0.3, Math.cos(a) * 0.5 * Math.sign(t[2] || 1)]);
      tuft(m, [t[0] + d[0] * 0.34, t[1] + d[1] * 0.28, t[2] + d[2] * 0.2], d, 0.1, 0.05, [0, 0.2, 0.4], toe);
    }
  }
  // Toe beans and the heart-shaped pad, a little proud of the palm.
  const bean = m.smooth(80);
  for (const t of PAW_TOES) m.sphere([t[0] + 0.4, t[1] - 0.02, t[2]], [0.08, 0.15, 0.12], 2, PART_BEAN, bean);
  for (const z of [-0.22, 0, 0.22]) m.sphere([0.5, -0.16 + (z === 0 ? -0.04 : 0), z], [0.08, 0.17, 0.15], 2, PART_BEAN, bean);
  m.sphere([0.49, 0.1, 0], [0.08, 0.2, 0.34], 2, PART_BEAN, bean);
  return m;
}

/** The paw mirrored top to bottom: toes toward +Y (claw mode 1: the claws on the other side). */
export function catPawUpModel(): Model {
  return new Model().add(catPawModel(), ([x, y, z]) => [x, -y, z]);
}

/** Length of the foreleg in pad radii: from the wrist far out of any level box. */
export const PAW_ARM_LENGTH = 14;

/** The foreleg from the wrist (z 0) back toward the shoulder (z PAW_ARM_LENGTH): a furry tube. */
export function pawArmModel(): Model {
  const m = new Model();
  const g = m.smooth(70);
  m.lathe(
    [
      [0.5, -0.05],
      [0.6, 0.15],
      [0.58, 0.5],
      [0.64, 1.2],
      [0.7, 3],
      [0.78, PAW_ARM_LENGTH],
    ],
    22,
    { part: PART_FUR, group: g, capStart: true, capEnd: true },
  );
  // Fluffy tufts along the wrist and the near part of the leg.
  for (let i = 0; i < 60; i++) {
    const a = i * 2.39996;
    const z = 0.02 + (i / 60) * 2.6;
    const r = 0.58 + 0.03 * z;
    const s = 0.09 + 0.05 * (((i * 37) % 7) / 7);
    m.sphere([Math.cos(a) * r, Math.sin(a) * r, z], [s, s, s * 1.6], 1, PART_FUR, g);
  }
  return m;
}

/** A claw: a curved ivory hook from its root (origin) down -Y, hooking toward +X; 0.42 long. */
export function pawClawModel(): Model {
  const m = new Model();
  tube(m, (t) => [0.16 * t * t, -0.42 * t, 0], (t) => 0.075 * Math.pow(1 - t, 0.8) + 0.003, 10, 10, PART_IVORY, m.smooth(60), 0.6);
  return m;
}

// ----- Disco inferno -------------------------------------------------------------

/** Rows of mirror tiles from pole to pole (spectacleShading.ts uses the same grid). */
export const DISCO_ROWS = 16;

/** Mirror tiles on columns per row: about 32 at the equator, fewer toward the poles. */
export function discoColumns(row: number): number {
  const lat = ((row + 0.5) / DISCO_ROWS - 0.5) * Math.PI;
  return Math.max(4, Math.round(32 * Math.cos(lat)));
}

/** The mirror ball: flat tiles with gaps over a dark core, a cap and a hook on top. */
export function discoBallModel(): Model {
  const m = new Model();
  m.sphere([0, 0, 0], 0.965, 2, PART_INK, m.smooth(60));
  const toV = (lat: number, lon: number, r: number): V3 => [Math.cos(lat) * Math.cos(lon) * r, Math.sin(lat) * r, Math.cos(lat) * Math.sin(lon) * r];
  for (let row = 0; row < DISCO_ROWS; row++) {
    const cols = discoColumns(row);
    const lat0 = (row / DISCO_ROWS - 0.5) * Math.PI;
    const lat1 = ((row + 1) / DISCO_ROWS - 0.5) * Math.PI;
    for (let c = 0; c < cols; c++) {
      const lon0 = (c / cols - 0.5) * Math.PI * 2;
      const lon1 = ((c + 1) / cols - 0.5) * Math.PI * 2;
      const gl = (lat1 - lat0) * 0.09;
      const go = (lon1 - lon0) * 0.09;
      const a = toV(lat0 + gl, lon0 + go, 1);
      const b = toV(lat0 + gl, lon1 - go, 1);
      const cc = toV(lat1 - gl, lon1 - go, 1);
      const d = toV(lat1 - gl, lon0 + go, 1);
      // Outward winding (seen from outside: a, d, c, b counter-clockwise).
      m.quad(a, d, cc, b, PART_MIRROR, 0);
    }
  }
  // Cap and hook at the top.
  m.lathe(
    [
      [0.001, 0.93],
      [0.26, 0.95],
      [0.2, 1.06],
      [0.07, 1.1],
      [0.001, 1.1],
    ],
    16,
    { axis: "y", part: PART_METAL, group: m.smooth(50) },
  );
  tube(m, (t) => [0, 1.1 + 0.16 * Math.sin(t * Math.PI), 0.12 * Math.cos(t * Math.PI)], () => 0.025, 12, 6, PART_METAL, m.smooth(60));
  return m;
}

/** The chain the ball hangs from, along +Z from 0 to 1 (stretched to the ceiling). */
export function discoChainModel(): Model {
  const m = new Model();
  const g = m.smooth(60);
  m.lathe(
    [
      [0.03, 0.0],
      [0.03, 1.0],
    ],
    8,
    { part: PART_METAL, group: g, capStart: true, capEnd: true },
  );
  return m;
}

/** A light-trail segment: a glowing capsule along Z (stretched between trail points). */
export function lightTrailModel(): Model {
  const m = new Model();
  const profile: Array<[number, number]> = [];
  const n = 6;
  for (let i = 0; i <= n; i++) {
    const a = -Math.PI / 2 + (i / n) * (Math.PI / 2);
    profile.push([Math.max(Math.cos(a) * 0.5, 0.0005), -0.5 + (Math.sin(a) + 1) * 0.12]);
  }
  for (let i = n; i >= 0; i--) {
    const a = -Math.PI / 2 + (i / n) * (Math.PI / 2);
    profile.push([Math.max(Math.cos(a) * 0.5, 0.0005), 0.5 - (Math.sin(a) + 1) * 0.12]);
  }
  m.lathe(profile, 10, { part: PART_GLOW, group: m.smooth(80) });
  return m;
}
