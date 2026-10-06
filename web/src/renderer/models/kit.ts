// Procedural modelling kit for the instanced meshes (ships and props).
//
// A Model is a soup of triangles. Every triangle carries a material part
// (PART_*, written into the mesh's per-vertex `team` byte, read by the
// object shader in models/shading.ts) and a smoothing group. `build()`
// computes vertex normals per group: flat groups keep face normals, smooth
// groups average the normals of coincident corners whose faces meet at less
// than the group's crease angle. Solids (loft, lathe, box) are wound
// outward automatically (their signed volume decides), so callers never
// care about vertex order.
//
// Conventions (same as primitives.ts): forward is -Z, up +Y, starboard +X;
// 1 u = one ship length.

import { MeshBuilder, type MeshData } from "../mesh.ts";
import { icosphere } from "../primitives.ts";

export type V3 = [number, number, number];

/**
 * Material parts per vertex (low byte of the instanced mesh's `team`
 * attribute). PART_PLAIN (the MeshBuilder default, 255) is the pre-VQ4 look:
 * the whole instance colour, lit, mixed toward glowing by the instance's
 * emissive amount.
 */
export const PART_HULL = 0;
export const PART_LIVERY = 1;
export const PART_TRIM = 2;
export const PART_METAL = 3;
export const PART_CANOPY = 4;
export const PART_ENGINE = 5;
export const PART_NAV_PORT = 6;
export const PART_NAV_STARBOARD = 7;
export const PART_NAV_TAIL = 8;
export const PART_GLOW = 9;
export const PART_SCORCH = 10;
/** Steady white lamp (floodlights). */
export const PART_LAMP = 11;
/**
 * M4 C2 (event visitors): carved stone in the instance colour with a
 * weathered grain (the gossip face), glossy white enamel (teeth,
 * eyeballs) and wet pink flesh (lips, tongues). Numbered from 20, so other
 * packages' parts can take 12 and up.
 */
export const PART_STONE = 20;
export const PART_ENAMEL = 21;
export const PART_FLESH = 22;
export const PART_PLAIN = 255;

/** Picks a part per loft/lathe quad: `segment` runs along the axis, `side` around it. */
export type PartFn = (segment: number, side: number) => number;
export type PartSpec = number | PartFn;

interface Tri {
  a: V3;
  b: V3;
  c: V3;
  part: number;
  group: number;
}

interface Group {
  smooth: boolean;
  /** Cosine of the crease angle: faces meeting at a smaller angle share a normal. */
  creaseCos: number;
}

export interface SolidOptions {
  part?: PartSpec;
  /** Smoothing group (Model.flat() / Model.smooth()); default flat. */
  group?: number;
  capStart?: boolean;
  capEnd?: boolean;
  /** Part of the caps (default: the part of the first quad). */
  capPart?: number;
}

const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const dot = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

export function faceNormal(a: V3, b: V3, c: V3): V3 {
  const n = cross(sub(b, a), sub(c, a));
  const l = Math.hypot(n[0], n[1], n[2]) || 1;
  return [n[0] / l, n[1] / l, n[2] / l];
}

function partOf(spec: PartSpec | undefined, segment: number, side: number): number {
  if (spec === undefined) return PART_HULL;
  return typeof spec === "number" ? spec : spec(segment, side);
}

/** Signed volume of a closed triangle set (positive when wound outward). */
export function signedVolume(tris: readonly { a: V3; b: V3; c: V3 }[]): number {
  let v = 0;
  for (const t of tris) v += dot(t.a, cross(t.b, t.c)) / 6;
  return v;
}

/** A ring of `n` points of a superellipse |x/rx|^e + |y/ry|^e = 1 at depth z (CCW seen from +Z). */
export function superRing(z: number, rx: number, ry: number, n: number, e = 2, cx = 0, cy = 0, phase = 0): V3[] {
  const pts: V3[] = [];
  for (let i = 0; i < n; i++) {
    const a = phase + (i / n) * Math.PI * 2;
    const c = Math.cos(a), s = Math.sin(a);
    const x = Math.sign(c) * Math.pow(Math.abs(c), 2 / e) * rx;
    const y = Math.sign(s) * Math.pow(Math.abs(s), 2 / e) * ry;
    pts.push([cx + x, cy + y, z]);
  }
  return pts;
}

/** Hull cross-section: superellipse with separate upper and lower heights (a flat belly, a tall back). */
export function hullRing(z: number, rx: number, up: number, down: number, n: number, e = 2.5, cy = 0): V3[] {
  return superRing(z, rx, 1, n, e).map(([x, y]): V3 => [x, cy + (y >= 0 ? y * up : y * down), z]);
}

export class Model {
  readonly tris: Tri[] = [];
  private readonly groups: Group[] = [{ smooth: false, creaseCos: 1 }];

  /** The flat-shaded group (every face keeps its own normal). */
  flat(): number {
    return 0;
  }

  /** A new smoothing group: faces meeting at less than `creaseDeg` share vertex normals. */
  smooth(creaseDeg = 50): number {
    this.groups.push({ smooth: true, creaseCos: Math.cos((creaseDeg * Math.PI) / 180) });
    return this.groups.length - 1;
  }

  get triangleCount(): number {
    return this.tris.length;
  }

  triangle(a: V3, b: V3, c: V3, part = PART_HULL, group = 0): this {
    this.tris.push({ a, b, c, part, group });
    return this;
  }

  /** Quad a-b-c-d (counter-clockwise from the front). */
  quad(a: V3, b: V3, c: V3, d: V3, part = PART_HULL, group = 0): this {
    return this.triangle(a, b, c, part, group).triangle(a, c, d, part, group);
  }

  /**
   * Flips triangles [from, end) when, summed over their area, their normals
   * point toward the reference points (`refs[i - from]`, a point on the
   * solid's axis next to triangle i) instead of away: the solid was wound
   * inside out. Works for open tubes too.
   */
  private orient(from: number, refs: readonly V3[]): void {
    let s = 0;
    for (let i = from; i < this.tris.length; i++) {
      const t = this.tris[i]!;
      const n = cross(sub(t.b, t.a), sub(t.c, t.a));
      const g: V3 = [(t.a[0] + t.b[0] + t.c[0]) / 3, (t.a[1] + t.b[1] + t.c[1]) / 3, (t.a[2] + t.b[2] + t.c[2]) / 3];
      s += dot(n, sub(g, refs[i - from]!));
    }
    if (s >= 0) return;
    for (let i = from; i < this.tris.length; i++) {
      const t = this.tris[i]!;
      [t.b, t.c] = [t.c, t.b];
    }
  }

  /** Fan-closes a ring; `inside` is a point inside the solid behind the cap (for orientation). */
  private cap(ring: V3[], part: number, inside: V3): void {
    const c = centroid(ring);
    let s = 0;
    for (let j = 0; j < ring.length; j++) s += dot(cross(sub(ring[(j + 1) % ring.length]!, c), sub(ring[j]!, c)), sub(c, inside));
    for (let j = 0; j < ring.length; j++) {
      const a = ring[(j + 1) % ring.length]!, b = ring[j]!;
      if (s >= 0) this.triangle(c, a, b, part, 0);
      else this.triangle(c, b, a, part, 0);
    }
  }

  /**
   * Skins consecutive closed rings (same point count each) into a solid.
   * Quad (segment i, side j) joins ring i and i+1 between points j and j+1.
   * A ring may collapse to one point (pointed nose or tail).
   */
  loft(rings: V3[][], opts: SolidOptions = {}): this {
    const from = this.tris.length;
    const refs: V3[] = [];
    const group = opts.group ?? 0;
    const n = rings[0]?.length ?? 0;
    const centres = rings.map(centroid);
    for (let i = 0; i + 1 < rings.length; i++) {
      const r0 = rings[i]!, r1 = rings[i + 1]!;
      if (r0.length !== n || r1.length !== n) throw new Error("loft rings need equal point counts");
      const c0 = centres[i]!, c1 = centres[i + 1]!;
      const ref: V3 = [(c0[0] + c1[0]) / 2, (c0[1] + c1[1]) / 2, (c0[2] + c1[2]) / 2];
      for (let j = 0; j < n; j++) {
        const k = (j + 1) % n;
        const part = partOf(opts.part, i, j);
        const a = r0[j]!, b = r0[k]!, c = r1[k]!, d = r1[j]!;
        // Skip degenerate halves where a ring collapsed to a point.
        if (!same(a, b) && !same(b, c)) {
          this.triangle(a, b, c, part, group);
          refs.push(ref);
        }
        if (!same(c, d) && !same(a, d)) {
          this.triangle(a, c, d, part, group);
          refs.push(ref);
        }
      }
    }
    const capPart = opts.capPart ?? partOf(opts.part, 0, 0);
    const last = rings.length - 1;
    this.orient(from, refs);
    if (opts.capStart && last > 0) this.cap(rings[0]!, capPart, centres[1]!);
    if (opts.capEnd && last > 0) this.cap(rings[last]!, capPart, centres[last - 1]!);
    return this;
  }

  /**
   * Revolves a profile of [radius, z] pairs about an axis through `center`
   * (default the Z axis; `axis` "x" or "y" turns it). `sx`/`sy` squash the
   * cross-section into an ellipse.
   */
  lathe(
    profile: readonly (readonly [number, number])[],
    segments: number,
    opts: SolidOptions & { center?: V3; axis?: "x" | "y" | "z"; sx?: number; sy?: number; phase?: number } = {},
  ): this {
    const c = opts.center ?? [0, 0, 0];
    const sx = opts.sx ?? 1, sy = opts.sy ?? 1;
    const rings = profile.map(([r, z]) =>
      superRing(z, r * sx, r * sy, segments, 2, 0, 0, opts.phase ?? 0).map((p) => axisMap(p, opts.axis ?? "z", c)),
    );
    return this.loft(rings, opts);
  }

  /**
   * Box with chamfered edges (`bevel` in u, 0 = sharp) centred at `c` with
   * half extents `h`, as a loft of chamfered rectangles along Z.
   */
  box(c: V3, h: V3, opts: SolidOptions & { bevel?: number } = {}): this {
    const bv = Math.min(opts.bevel ?? 0, h[0] * 0.9, h[1] * 0.9, h[2] * 0.9);
    const rect = (z: number, inset: number): V3[] => {
      const x = h[0] - inset, y = h[1] - inset, e = bv - inset;
      if (bv <= 0) return [[c[0] + x, c[1] - y, z], [c[0] + x, c[1] + y, z], [c[0] - x, c[1] + y, z], [c[0] - x, c[1] - y, z]];
      return [
        [c[0] + x, c[1] - y + e, z], [c[0] + x, c[1] + y - e, z], [c[0] + x - e, c[1] + y, z], [c[0] - x + e, c[1] + y, z],
        [c[0] - x, c[1] + y - e, z], [c[0] - x, c[1] - y + e, z], [c[0] - x + e, c[1] - y, z], [c[0] + x - e, c[1] - y, z],
      ];
    };
    const rings = bv > 0
      ? [rect(c[2] - h[2], bv), rect(c[2] - h[2] + bv, 0), rect(c[2] + h[2] - bv, 0), rect(c[2] + h[2], bv)]
      : [rect(c[2] - h[2], 0), rect(c[2] + h[2], 0)];
    return this.loft(rings, { ...opts, capStart: true, capEnd: true });
  }

  /** Icosphere (subdiv 0-2) with radii `r` (x, y, z). */
  sphere(c: V3, r: V3 | number, subdiv = 1, part = PART_HULL, group = 0): this {
    const rr: V3 = typeof r === "number" ? [r, r, r] : r;
    const s = icosphere(subdiv);
    const from = this.tris.length;
    const p = s.points.map((q): V3 => [c[0] + q[0] * rr[0], c[1] + q[1] * rr[1], c[2] + q[2] * rr[2]]);
    for (const [a, b, d] of s.tris) this.triangle(p[a]!, p[b]!, p[d]!, part, group);
    this.orient(from, s.tris.map(() => c));
    return this;
  }

  /** Appends `other` (its groups re-numbered), optionally transformed point by point. */
  add(other: Model, xf?: (p: V3) => V3): this {
    const base = this.groups.length - 1;
    other.groups.slice(1).forEach((g) => this.groups.push({ ...g }));
    const map = xf ?? ((p: V3) => p);
    const flip = xf ? mirrors(xf) : false;
    for (const t of other.tris) {
      const a = map(t.a), b = map(t.b), c = map(t.c);
      this.tris.push({ a, b: flip ? c : b, c: flip ? b : c, part: t.part, group: t.group === 0 ? 0 : t.group + base });
    }
    return this;
  }

  /** Appends a mirror image of `other` across the YZ plane (port <-> starboard), parts mapped by `swap`. */
  addMirrorX(other: Model, swap: (part: number) => number = swapNavParts): this {
    const m = new Model();
    m.add(other, (p) => [-p[0], p[1], p[2]]);
    for (const t of m.tris) t.part = swap(t.part);
    return this.add(m);
  }

  /** Copy transformed point by point (winding kept outward). */
  transformed(xf: (p: V3) => V3): Model {
    return new Model().add(this, xf);
  }

  /** Copy with every part remapped. */
  repainted(map: (part: number) => number): Model {
    const m = new Model().add(this);
    for (const t of m.tris) t.part = map(t.part);
    return m;
  }

  /** Largest distance of any vertex from the origin. */
  maxRadius(): number {
    let r = 0;
    for (const t of this.tris) for (const p of [t.a, t.b, t.c]) r = Math.max(r, Math.hypot(p[0], p[1], p[2]));
    return r;
  }

  /** Axis-aligned bounds of the vertices. */
  bounds(): { min: V3; max: V3 } {
    const min: V3 = [Infinity, Infinity, Infinity], max: V3 = [-Infinity, -Infinity, -Infinity];
    for (const t of this.tris) {
      for (const p of [t.a, t.b, t.c]) {
        for (let k = 0; k < 3; k++) {
          min[k] = Math.min(min[k]!, p[k]!);
          max[k] = Math.max(max[k]!, p[k]!);
        }
      }
    }
    return { min, max };
  }

  /** Builds the mesh: per-group normals, parts in the `team` byte, vertices shared where identical. */
  build(): MeshData {
    const normals = this.tris.map((t) => faceNormal(t.a, t.b, t.c));
    const areas = this.tris.map((t) => Math.hypot(...cross(sub(t.b, t.a), sub(t.c, t.a))));
    // Corners of smooth groups by quantised position.
    const corners = new Map<string, number[]>();
    this.tris.forEach((t, i) => {
      if (!this.groups[t.group]?.smooth) return;
      for (const p of [t.a, t.b, t.c]) {
        const key = `${t.group}|${posKey(p)}`;
        let list = corners.get(key);
        if (!list) corners.set(key, (list = []));
        list.push(i);
      }
    });
    const b = new MeshBuilder();
    const shared = new Map<string, number>();
    this.tris.forEach((t, i) => {
      const g = this.groups[t.group]!;
      const fn = normals[i]!;
      const ids = [t.a, t.b, t.c].map((p) => {
        let n = fn;
        if (g.smooth) {
          const acc: V3 = [0, 0, 0];
          for (const j of corners.get(`${t.group}|${posKey(p)}`) ?? []) {
            const nj = normals[j]!;
            if (dot(nj, fn) < g.creaseCos) continue;
            const w = areas[j]!;
            acc[0] += nj[0] * w;
            acc[1] += nj[1] * w;
            acc[2] += nj[2] * w;
          }
          const l = Math.hypot(...acc);
          if (l > 1e-12) n = [acc[0] / l, acc[1] / l, acc[2] / l];
        }
        const key = `${posKey(p)}|${posKey(n)}|${t.part}`;
        const hit = shared.get(key);
        if (hit !== undefined) return hit;
        const id = b.vertex(p[0], p[1], p[2], n[0], n[1], n[2], t.part);
        shared.set(key, id);
        return id;
      });
      b.triangle(ids[0]!, ids[1]!, ids[2]!);
    });
    return b.build();
  }
}

/** Swaps the port and starboard nav light parts (for mirrored halves). */
export function swapNavParts(part: number): number {
  if (part === PART_NAV_PORT) return PART_NAV_STARBOARD;
  if (part === PART_NAV_STARBOARD) return PART_NAV_PORT;
  return part;
}

function centroid(ring: readonly V3[]): V3 {
  const c: V3 = [0, 0, 0];
  for (const p of ring) for (let k = 0; k < 3; k++) c[k]! += p[k]! / ring.length;
  return c;
}

function same(a: V3, b: V3): boolean {
  return Math.abs(a[0] - b[0]) < 1e-9 && Math.abs(a[1] - b[1]) < 1e-9 && Math.abs(a[2] - b[2]) < 1e-9;
}

function posKey(p: V3): string {
  return `${Math.round(p[0] * 1e5)},${Math.round(p[1] * 1e5)},${Math.round(p[2] * 1e5)}`;
}

/** Maps a lathe point (built about +Z) onto the chosen axis through `c`. */
function axisMap(p: V3, axis: "x" | "y" | "z", c: V3): V3 {
  if (axis === "x") return [c[0] + p[2], c[1] + p[1], c[2] - p[0]];
  if (axis === "y") return [c[0] + p[0], c[1] + p[2], c[2] - p[1]];
  return [c[0] + p[0], c[1] + p[1], c[2] + p[2]];
}

/** True when a point transform mirrors (negative determinant), estimated on the unit axes. */
function mirrors(xf: (p: V3) => V3): boolean {
  const o = xf([0, 0, 0]);
  const ex = sub(xf([1, 0, 0]), o), ey = sub(xf([0, 1, 0]), o), ez = sub(xf([0, 0, 1]), o);
  return dot(ex, cross(ey, ez)) < 0;
}

/** Rotation of a point about the X, Y or Z axis by `angle` (rad). */
export function rotate(axis: "x" | "y" | "z", angle: number): (p: V3) => V3 {
  const c = Math.cos(angle), s = Math.sin(angle);
  if (axis === "x") return ([x, y, z]) => [x, y * c - z * s, y * s + z * c];
  if (axis === "y") return ([x, y, z]) => [x * c + z * s, y, -x * s + z * c];
  return ([x, y, z]) => [x * c - y * s, x * s + y * c, z];
}

/** Composes point transforms (applied left to right). */
export function chain(...xfs: ((p: V3) => V3)[]): (p: V3) => V3 {
  return (p) => xfs.reduce((q, f) => f(q), p);
}

export function translate(t: V3): (p: V3) => V3 {
  return ([x, y, z]) => [x + t[0], y + t[1], z + t[2]];
}

export function scale(s: number | V3): (p: V3) => V3 {
  const v: V3 = typeof s === "number" ? [s, s, s] : s;
  return ([x, y, z]) => [x * v[0], y * v[1], z * v[2]];
}

/** Deterministic 0..1 hash for procedural jitter (small integer inputs). */
export function hash01(i: number): number {
  const x = Math.sin(i * 127.1 + 311.7) * 43758.5453;
  return x - Math.floor(x);
}
