// Small procedural meshes for instanced objects. All face -Z (ship forward)
// and are about 1 u across, so instance scale 1 = one ship length. The
// entity renderer (game/scene.ts) maps data-driven visual shapes to these
// mesh kinds (MESH_BUILDERS) and tints them per instance.

import { MeshBuilder, NO_TEAM, type MeshData } from "./mesh.ts";

export type V3 = [number, number, number];

function faceNormal(a: V3, b: V3, c: V3): V3 {
  const ux = b[0] - a[0], uy = b[1] - a[1], uz = b[2] - a[2];
  const vx = c[0] - a[0], vy = c[1] - a[1], vz = c[2] - a[2];
  const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
  const len = Math.hypot(nx, ny, nz) || 1;
  return [nx / len, ny / len, nz / len];
}

/** Flat-shaded mesh from shared corner positions and CCW triangles. */
function flatMesh(points: V3[], tris: [number, number, number][]): MeshData {
  const b = new MeshBuilder();
  for (const [ia, ib, ic] of tris) {
    const a = points[ia], bb = points[ib], c = points[ic];
    if (!a || !bb || !c) throw new Error("primitive triangle index out of range");
    const n = faceNormal(a, bb, c);
    const i0 = b.vertex(...a, ...n);
    const i1 = b.vertex(...bb, ...n);
    const i2 = b.vertex(...c, ...n);
    b.triangle(i0, i1, i2);
  }
  return b.build();
}

/**
 * Appends a star-shaped solid (every face visible from its centroid) and
 * winds each triangle outward from that centroid, so the triangle lists
 * below need not care about their vertex order. `smooth` uses the direction
 * from the centroid as the normal (spheres), otherwise faces are flat.
 */
export function appendSolid(b: MeshBuilder, points: V3[], tris: [number, number, number][], smooth = false): void {
  let cx = 0, cy = 0, cz = 0;
  for (const p of points) {
    cx += p[0] / points.length;
    cy += p[1] / points.length;
    cz += p[2] / points.length;
  }
  const c: V3 = [cx, cy, cz];
  for (const [ia, ib, ic] of tris) {
    const a = points[ia], p1 = points[ib], p2 = points[ic];
    if (!a || !p1 || !p2) throw new Error("primitive triangle index out of range");
    let n = faceNormal(a, p1, p2);
    const g: V3 = [(a[0] + p1[0] + p2[0]) / 3 - c[0], (a[1] + p1[1] + p2[1]) / 3 - c[1], (a[2] + p1[2] + p2[2]) / 3 - c[2]];
    let order: V3[] = [a, p1, p2];
    if (n[0] * g[0] + n[1] * g[1] + n[2] * g[2] < 0) {
      order = [a, p2, p1];
      n = [-n[0], -n[1], -n[2]];
    }
    const ids = order.map((p) => {
      let vn = n;
      if (smooth) {
        const d: V3 = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
        const len = Math.hypot(...d) || 1;
        vn = [d[0] / len, d[1] / len, d[2] / len];
      }
      return b.vertex(p[0], p[1], p[2], vn[0], vn[1], vn[2]);
    });
    b.triangle(ids[0]!, ids[1]!, ids[2]!);
  }
}

/** A dart-shaped Scrapling hull: nose at -Z, fins at the back. */
export function shipMesh(): MeshData {
  const points: V3[] = [
    [0, 0, -0.55], // 0 nose
    [-0.4, 0, 0.4], // 1 left wing tip
    [0.4, 0, 0.4], // 2 right wing tip
    [0, 0.18, 0.3], // 3 top ridge
    [0, -0.12, 0.3], // 4 belly
    [0, 0.05, 0.45], // 5 tail
  ];
  const tris: [number, number, number][] = [
    [0, 1, 3], [0, 3, 2], // top
    [0, 4, 1], [0, 2, 4], // bottom
    [1, 5, 3], [3, 5, 2], // back top
    [1, 4, 5], [2, 5, 4], // back bottom
  ];
  return flatMesh(points, tris);
}

/** An elongated octahedron for bolts and rounds. */
export function projectileMesh(): MeshData {
  const w = 0.12;
  const points: V3[] = [
    [0, 0, -0.5], [0, 0, 0.5],
    [w, 0, 0], [0, w, 0], [-w, 0, 0], [0, -w, 0],
  ];
  const tris: [number, number, number][] = [
    [0, 3, 2], [0, 4, 3], [0, 5, 4], [0, 2, 5],
    [1, 2, 3], [1, 3, 4], [1, 4, 5], [1, 5, 2],
  ];
  return flatMesh(points, tris);
}

const T = (1 + Math.sqrt(5)) / 2;
const ICO_POINTS: V3[] = [
  [-1, T, 0], [1, T, 0], [-1, -T, 0], [1, -T, 0],
  [0, -1, T], [0, 1, T], [0, -1, -T], [0, 1, -T],
  [T, 0, -1], [T, 0, 1], [-T, 0, -1], [-T, 0, 1],
];
const ICO_TRIS: [number, number, number][] = [
  [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
  [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
  [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
  [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
];

/** Unit-direction icosphere points and triangles (`subdiv` 0-3). */
export function icosphere(subdiv: number): { points: V3[]; tris: [number, number, number][] } {
  const points: V3[] = ICO_POINTS.map((p) => {
    const l = Math.hypot(...p);
    return [p[0] / l, p[1] / l, p[2] / l];
  });
  let tris = ICO_TRIS.map((t) => [...t] as [number, number, number]);
  for (let s = 0; s < Math.min(3, Math.max(0, subdiv)); s++) {
    const mid = new Map<string, number>();
    const midpoint = (a: number, b: number): number => {
      const key = a < b ? `${a},${b}` : `${b},${a}`;
      const hit = mid.get(key);
      if (hit !== undefined) return hit;
      const pa = points[a]!, pb = points[b]!;
      const m: V3 = [(pa[0] + pb[0]) / 2, (pa[1] + pb[1]) / 2, (pa[2] + pb[2]) / 2];
      const l = Math.hypot(...m);
      points.push([m[0] / l, m[1] / l, m[2] / l]);
      mid.set(key, points.length - 1);
      return points.length - 1;
    };
    const next: [number, number, number][] = [];
    for (const [a, b, c] of tris) {
      const ab = midpoint(a, b), bc = midpoint(b, c), ca = midpoint(c, a);
      next.push([a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]);
    }
    tris = next;
  }
  return { points, tris };
}

/** Appends a sphere of `radius` at `c`. */
export function appendSphere(b: MeshBuilder, c: V3, radius: number, subdiv = 1, smooth = true): void {
  const s = icosphere(subdiv);
  appendSolid(
    b,
    s.points.map((p) => [c[0] + p[0] * radius, c[1] + p[1] * radius, c[2] + p[2] * radius]),
    s.tris,
    smooth,
  );
}

/** Sphere of diameter 1 (orbs, dust volumes, glow shells). */
export function sphereMesh(subdiv = 2): MeshData {
  const b = new MeshBuilder();
  appendSphere(b, [0, 0, 0], 0.5, subdiv);
  return b.build();
}

/** Pyramid with apex `tip` over the square base centred at `base` (half size `h`, axis Z). */
function appendPyramidZ(b: MeshBuilder, base: V3, h: number, tip: V3): void {
  const [x, y, z] = base;
  const pts: V3[] = [[x - h, y - h, z], [x + h, y - h, z], [x + h, y + h, z], [x - h, y + h, z], tip];
  appendSolid(b, pts, [[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4], [0, 2, 1], [0, 3, 2]]);
}

/** Rocket: square body, pointed nose at -Z, two fins at the back. */
export function rocketMesh(): MeshData {
  const b = new MeshBuilder();
  appendOrientedBox(b, [0, 0, 0.08], [0.1, 0, 0], [0, 0.1, 0], [0, 0, 0.32]);
  appendPyramidZ(b, [0, 0, -0.24], 0.1, [0, 0, -0.5]);
  appendOrientedBox(b, [0, 0, 0.34], [0.24, 0, 0], [0, 0.02, 0], [0, 0, 0.07]);
  appendOrientedBox(b, [0, 0, 0.34], [0.02, 0, 0], [0, 0.24, 0], [0, 0, 0.07]);
  return b.build();
}

/** Harpoon: thin shaft with a barbed head at -Z. */
export function harpoonMesh(): MeshData {
  const b = new MeshBuilder();
  appendOrientedBox(b, [0, 0, 0.1], [0.03, 0, 0], [0, 0.03, 0], [0, 0, 0.4]);
  appendPyramidZ(b, [0, 0, -0.3], 0.09, [0, 0, -0.5]);
  return b.build();
}

/** Mine: a ball with six spikes. */
export function mineMesh(): MeshData {
  const b = new MeshBuilder();
  appendSphere(b, [0, 0, 0], 0.3, 1, false);
  const axes: V3[] = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
  for (let k = 0; k < 3; k++) {
    const a = axes[k]!, p = axes[(k + 1) % 3]!, q = axes[(k + 2) % 3]!;
    appendOrientedBox(b, [0, 0, 0], [a[0] * 0.5, a[1] * 0.5, a[2] * 0.5], [p[0] * 0.04, p[1] * 0.04, p[2] * 0.04], [q[0] * 0.04, q[1] * 0.04, q[2] * 0.04]);
  }
  return b.build();
}

/** Supply crate (weapon pickups): a box with a band around it. */
export function crateMesh(): MeshData {
  const b = new MeshBuilder();
  appendBox(b, 0, 0, 0, 0.35, 0.35, 0.35);
  appendBox(b, 0, 0, 0, 0.38, 0.08, 0.38);
  return b.build();
}

/** Upgrade pickup: a diamond (octahedron). */
export function upgradeMesh(): MeshData {
  const b = new MeshBuilder();
  const w = 0.3;
  appendSolid(
    b,
    [[0, 0.5, 0], [0, -0.5, 0], [w, 0, 0], [0, 0, w], [-w, 0, 0], [0, 0, -w]],
    [[0, 2, 3], [0, 3, 4], [0, 4, 5], [0, 5, 2], [1, 2, 3], [1, 3, 4], [1, 4, 5], [1, 5, 2]],
  );
  return b.build();
}

/** Gadget pickup: a hexagonal token standing on edge (faces ±Z). */
export function gadgetMesh(): MeshData {
  const b = new MeshBuilder();
  const pts: V3[] = [];
  for (const z of [-0.1, 0.1]) {
    for (let i = 0; i < 6; i++) {
      const a = (i / 6) * Math.PI * 2;
      pts.push([Math.cos(a) * 0.42, Math.sin(a) * 0.42, z]);
    }
  }
  const tris: [number, number, number][] = [];
  for (let i = 0; i < 6; i++) {
    const j = (i + 1) % 6;
    tris.push([i, j, 6 + j], [i, 6 + j, 6 + i]);
    if (i > 0 && i < 5) tris.push([0, i, i + 1], [6, 6 + i, 6 + i + 1]);
  }
  appendSolid(b, pts, tris);
  return b.build();
}

/** Deterministic 0..1 hash for procedural jitter. */
function hash01(i: number): number {
  const x = Math.sin(i * 127.1 + 311.7) * 43758.5453;
  return x - Math.floor(x);
}

/** Debris chunk: a lumpy flat-shaded rock of diameter about 1. */
export function debrisMesh(): MeshData {
  const b = new MeshBuilder();
  const s = icosphere(1);
  const pts = s.points.map((p, i): V3 => {
    const r = 0.4 + 0.12 * hash01(i);
    return [p[0] * r, p[1] * r * 0.85, p[2] * r];
  });
  appendSolid(b, pts, s.tris);
  return b.build();
}

/** Scrap pile: a heap of bent plates and bars (about 1 u across). */
export function scrapMesh(): MeshData {
  const b = new MeshBuilder();
  for (let i = 0; i < 6; i++) {
    const a = hash01(i) * Math.PI * 2;
    const tilt = (hash01(i + 10) - 0.5) * 1.2;
    const r = 0.2 * hash01(i + 20);
    const c: V3 = [Math.cos(a) * r, (hash01(i + 30) - 0.5) * 0.2, Math.sin(a) * r];
    const len = 0.15 + 0.2 * hash01(i + 40);
    const ax: V3 = [Math.cos(a) * len, Math.sin(tilt) * len, Math.sin(a) * len];
    const ay: V3 = [-Math.sin(tilt) * Math.cos(a) * 0.04, Math.cos(tilt) * 0.04, -Math.sin(tilt) * Math.sin(a) * 0.04];
    // az = normalized(ax x ay) * width
    const cx = ax[1] * ay[2] - ax[2] * ay[1], cy = ax[2] * ay[0] - ax[0] * ay[2], cz = ax[0] * ay[1] - ax[1] * ay[0];
    const cl = Math.hypot(cx, cy, cz) || 1;
    const w = 0.06 + 0.08 * hash01(i + 50);
    appendOrientedBox(b, c, ax, ay, [(cx / cl) * w, (cy / cl) * w, (cz / cl) * w]);
  }
  return b.build();
}

/** Wreck: the broken halves of a hull with a twisted spar. */
export function wreckMesh(): MeshData {
  const b = new MeshBuilder();
  appendOrientedBox(b, [-0.15, 0, -0.15], [0.12, 0.02, 0.1], [-0.02, 0.1, 0], [0.1, 0, 0.3]);
  appendOrientedBox(b, [0.22, -0.05, 0.2], [0.25, -0.05, 0.02], [0.02, 0.06, 0], [0, 0, 0.12]);
  appendOrientedBox(b, [0.05, 0.05, 0.05], [0.03, 0, 0], [0, 0.03, 0.01], [0, -0.01, 0.35]);
  appendSolid(b, [[-0.35, 0, 0.3], [0.0, 0.0, 0.1], [-0.1, 0.12, 0.25], [-0.15, -0.08, 0.35]], [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]]);
  return b.build();
}

/** Rubber duck (Seed Grenade easter egg): body, head and beak, facing -Z. */
export function duckMesh(): MeshData {
  const b = new MeshBuilder();
  appendSphere(b, [0, 0, 0.05], 0.38, 1);
  appendSphere(b, [0, 0.36, -0.2], 0.22, 1);
  appendOrientedBox(b, [0, 0.33, -0.44], [0.08, 0, 0], [0, 0.03, 0], [0, 0, 0.07]);
  return b.build();
}

/** Paint team for a face (NO_TEAM = unpainted), either fixed or chosen from the face's outward normal. */
export type FaceTeam = number | ((faceNormal: V3) => number);

/** Axis-aligned box (half extents hx, hy, hz) appended to a builder with flat normals. */
export function appendBox(
  b: MeshBuilder,
  cx: number, cy: number, cz: number,
  hx: number, hy: number, hz: number,
  team: FaceTeam = NO_TEAM,
): void {
  appendOrientedBox(b, [cx, cy, cz], [hx, 0, 0], [0, hy, 0], [0, 0, hz], team);
}

/**
 * Oriented box: centre c and three mutually perpendicular half-axis vectors.
 * Faces are wound counter-clockwise as seen from outside.
 */
export function appendOrientedBox(b: MeshBuilder, c: V3, ax: V3, ay: V3, az: V3, team: FaceTeam = NO_TEAM): void {
  const axes = [ax, ay, az];
  for (let k = 0; k < 3; k++) {
    for (const sign of [1, -1]) {
      const a = axes[k]!;
      const n: V3 = [a[0] * sign, a[1] * sign, a[2] * sign];
      let p = axes[(k + 1) % 3]!;
      let q = axes[(k + 2) % 3]!;
      // Order the in-plane axes so p x q points along the outward normal (CCW from outside).
      const cx = p[1] * q[2] - p[2] * q[1], cy = p[2] * q[0] - p[0] * q[2], cz = p[0] * q[1] - p[1] * q[0];
      if (cx * n[0] + cy * n[1] + cz * n[2] < 0) [p, q] = [q, p];
      const len = Math.hypot(n[0], n[1], n[2]) || 1;
      const nn: V3 = [n[0] / len, n[1] / len, n[2] / len];
      const t = typeof team === "function" ? team(nn) : team;
      const corner = (sp: number, sq: number): number =>
        b.vertex(
          c[0] + n[0] + p[0] * sp + q[0] * sq,
          c[1] + n[1] + p[1] * sp + q[1] * sq,
          c[2] + n[2] + p[2] * sp + q[2] * sq,
          nn[0], nn[1], nn[2], t,
        );
      const i0 = corner(-1, -1), i1 = corner(1, -1), i2 = corner(1, 1), i3 = corner(-1, 1);
      b.quad(i0, i1, i2, i3);
    }
  }
}

/** Every instanced mesh kind the renderer knows. */
export type MeshKind =
  | "ship"
  | "bolt"
  | "orb"
  | "sphere"
  | "rocket"
  | "harpoon"
  | "mine"
  | "crate"
  | "upgrade"
  | "gadget"
  | "debris"
  | "scrap"
  | "wreck"
  | "duck";

/** Builders for every mesh kind (the renderer uploads each once). */
export const MESH_BUILDERS: Readonly<Record<MeshKind, () => MeshData>> = {
  ship: shipMesh,
  bolt: projectileMesh,
  orb: () => sphereMesh(1),
  sphere: () => sphereMesh(2),
  rocket: rocketMesh,
  harpoon: harpoonMesh,
  mine: mineMesh,
  crate: crateMesh,
  upgrade: upgradeMesh,
  gadget: gadgetMesh,
  debris: debrisMesh,
  scrap: scrapMesh,
  wreck: wreckMesh,
  duck: duckMesh,
};

export const MESH_KINDS = Object.keys(MESH_BUILDERS) as MeshKind[];
