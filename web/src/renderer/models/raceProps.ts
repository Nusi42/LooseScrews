// Belt Up props (M4 C7): the drifting boulder (a lumpy, cratered rock about
// 1 u across at instance scale 1, smooth shaded: scale it by its diameter),
// the pebble (a faceted chip about 1 u across for the client-only pebbles
// in the belt) and the
// start-light gantry (a dark bar with five lamp housings, 1 u across at
// scale 1; the lamps themselves are glowing orb instances in front of it,
// game/race/raceFx.ts). Rock parts use the plain instance look (the
// instance colour, lit), the gantry the dark trim material.

import { MeshBuilder, type MeshData } from "../mesh.ts";
import { appendOrientedBox, appendSphere, icosphere, type V3 } from "../primitives.ts";
import { PART_METAL, PART_TRIM } from "./kit.ts";

/** Integer hash to 0..1 (stable across runs). */
function hash01(i: number, salt: number): number {
  let h = Math.imul(i ^ Math.imul(salt, 0x27d4eb2d), 0x9e3779b1) >>> 0;
  h ^= h >>> 15;
  h = Math.imul(h, 0x85ebca6b) >>> 0;
  h ^= h >>> 13;
  return (h >>> 0) / 4294967296;
}

/** Smooth 3D value noise (0..1) on the unit sphere's points. */
function noise3(x: number, y: number, z: number, salt: number): number {
  const xi = Math.floor(x), yi = Math.floor(y), zi = Math.floor(z);
  const xf = x - xi, yf = y - yi, zf = z - zi;
  const s = (t: number): number => t * t * (3 - 2 * t);
  const h = (a: number, b: number, c: number): number => hash01(((a * 73856093) ^ (b * 19349663) ^ (c * 83492791)) | 0, salt);
  const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;
  const u = s(xf), v = s(yf), w = s(zf);
  return lerp(
    lerp(lerp(h(xi, yi, zi), h(xi + 1, yi, zi), u), lerp(h(xi, yi + 1, zi), h(xi + 1, yi + 1, zi), u), v),
    lerp(lerp(h(xi, yi, zi + 1), h(xi + 1, yi, zi + 1), u), lerp(h(xi, yi + 1, zi + 1), h(xi + 1, yi + 1, zi + 1), u), v),
    w,
  );
}

/** Radius of the boulder surface along unit direction `p` (about 0.8-1.1). */
export function boulderRadius(p: V3, salt = 1): number {
  let r = 1;
  // Two octaves of lumps and an egg-ish stretch.
  r += 0.16 * (noise3(p[0] * 2.1 + 3, p[1] * 2.1, p[2] * 2.1, salt) - 0.5) * 2;
  r += 0.07 * (noise3(p[0] * 5.3, p[1] * 5.3 + 7, p[2] * 5.3, salt + 1) - 0.5) * 2;
  r *= 1 + 0.12 * p[0] * p[0] - 0.08 * p[1] * p[1];
  // Craters: shallow bowls with a raised rim.
  const craters: V3[] = [
    [0.6, 0.55, 0.58],
    [-0.7, -0.2, 0.68],
    [0.1, -0.9, -0.42],
    [-0.35, 0.82, -0.45],
  ];
  craters.forEach((c, i) => {
    const l = Math.hypot(...c);
    const d = Math.acos(Math.max(-1, Math.min(1, (p[0] * c[0] + p[1] * c[1] + p[2] * c[2]) / l)));
    const size = 0.32 + 0.12 * hash01(i, salt + 5);
    const k = d / size;
    if (k < 1) r -= 0.1 * (1 - k * k);
    else if (k < 1.5) r += 0.025 * Math.sin(((k - 1) / 0.5) * Math.PI);
  });
  return r;
}

/** Smooth-shaded mesh from displaced sphere points (shared vertices, area-weighted normals). */
function displacedSphere(subdiv: number, radius: (p: V3) => number, part: number): MeshData {
  const s = icosphere(subdiv);
  const pts = s.points.map((p): V3 => {
    const r = 0.5 * radius(p);
    return [p[0] * r, p[1] * r, p[2] * r];
  });
  const normals = pts.map((): V3 => [0, 0, 0]);
  for (const [a, b, c] of s.tris) {
    const pa = pts[a]!, pb = pts[b]!, pc = pts[c]!;
    const u: V3 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
    const v: V3 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
    let n: V3 = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    // Outward (the icosphere's winding is not guaranteed).
    const g = pa[0] + pb[0] + pc[0];
    const gy = pa[1] + pb[1] + pc[1];
    const gz = pa[2] + pb[2] + pc[2];
    if (n[0] * g + n[1] * gy + n[2] * gz < 0) n = [-n[0], -n[1], -n[2]];
    for (const i of [a, b, c]) {
      const m = normals[i]!;
      m[0] += n[0];
      m[1] += n[1];
      m[2] += n[2];
    }
  }
  const b = new MeshBuilder();
  pts.forEach((p, i) => {
    const n = normals[i]!;
    const l = Math.hypot(...n) || 1;
    b.vertex(p[0], p[1], p[2], n[0] / l, n[1] / l, n[2] / l, part);
  });
  for (const [a, bb, c] of s.tris) {
    const pa = pts[a]!, pb = pts[bb]!, pc = pts[c]!;
    const u: V3 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
    const v: V3 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
    const n: V3 = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    if (n[0] * (pa[0] + pb[0] + pc[0]) + n[1] * (pa[1] + pb[1] + pc[1]) + n[2] * (pa[2] + pb[2] + pc[2]) < 0) b.triangle(a, c, bb);
    else b.triangle(a, bb, c);
  }
  return b.build();
}

/** The drifting boulder: about 1 u across at scale 1 (instances scale it by the boulder's diameter). */
export function boulderMesh(): MeshData {
  return displacedSphere(3, (p) => boulderRadius(p, 11), 255);
}

/** A pebble: a faceted chip about 1 u across at scale 1. */
export function pebbleMesh(): MeshData {
  const s = icosphere(1);
  const b = new MeshBuilder();
  const pts = s.points.map((p, i): V3 => {
    const r = 0.5 * (0.75 + 0.35 * hash01(i, 3));
    return [p[0] * r * 1.15, p[1] * r * 0.8, p[2] * r];
  });
  for (const [a, bb, c] of s.tris) {
    const pa = pts[a]!, pb = pts[bb]!, pc = pts[c]!;
    const u: V3 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
    const v: V3 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
    let n: V3 = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let order = [pa, pb, pc];
    if (n[0] * (pa[0] + pb[0] + pc[0]) + n[1] * (pa[1] + pb[1] + pc[1]) + n[2] * (pa[2] + pb[2] + pc[2]) < 0) {
      n = [-n[0], -n[1], -n[2]];
      order = [pa, pc, pb];
    }
    const l = Math.hypot(...n) || 1;
    const ids = order.map((p) => b.vertex(p[0], p[1], p[2], n[0] / l, n[1] / l, n[2] / l));
    b.triangle(ids[0]!, ids[1]!, ids[2]!);
  }
  return b.build();
}

/** Lamp centres of the gantry at scale 1 (x across, slightly in front at -z). */
export const GANTRY_LAMPS: readonly V3[] = [-0.4, -0.2, 0, 0.2, 0.4].map((x): V3 => [x, 0, -0.07]);

/** Lamp radius at scale 1. */
export const GANTRY_LAMP_RADIUS = 0.075;

/** The start-light gantry: a dark bar with five hooded lamp sockets on a metal back plate. */
export function gantryMesh(): MeshData {
  const b = new MeshBuilder();
  appendOrientedBox(b, [0, 0, 0], [0.5, 0, 0], [0, 0.11, 0], [0, 0, 0.05], PART_TRIM);
  for (const [x] of GANTRY_LAMPS) {
    // A hood over each lamp.
    appendOrientedBox(b, [x, 0.095, -0.075], [0.085, 0, 0], [0, 0.012, 0], [0, 0, 0.05], PART_METAL);
  }
  // A back plate with a lit rim, so the gantry reads against the dark.
  appendOrientedBox(b, [0, 0, 0.06], [0.54, 0, 0], [0, 0.15, 0], [0, 0, 0.012], PART_METAL);
  // Dark sockets behind the lamps.
  for (const [x, y, z] of GANTRY_LAMPS) appendSphere(b, [x, y, z + 0.02], GANTRY_LAMP_RADIUS * 1.05, 1);
  return b.build();
}
