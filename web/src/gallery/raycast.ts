// Ray casts against the terrain chunk meshes on the CPU, for picking the
// gallery's fixed camera poses (the client has no SDF query; the meshes are
// deterministic per share code and generator version, so the picked poses
// are too).

import type { Vec3 } from "../math/vec3.ts";
import type { MeshData } from "../renderer/mesh.ts";

/** A mesh with its bounding box, for the per-mesh early out. */
export interface RayMesh {
  mesh: MeshData;
  min: Vec3;
  max: Vec3;
}

export function rayMesh(mesh: MeshData): RayMesh {
  const p = mesh.positions;
  const min = { x: Infinity, y: Infinity, z: Infinity };
  const max = { x: -Infinity, y: -Infinity, z: -Infinity };
  for (let i = 0; i + 2 < p.length; i += 3) {
    const x = p[i] ?? 0, y = p[i + 1] ?? 0, z = p[i + 2] ?? 0;
    min.x = Math.min(min.x, x);
    min.y = Math.min(min.y, y);
    min.z = Math.min(min.z, z);
    max.x = Math.max(max.x, x);
    max.y = Math.max(max.y, y);
    max.z = Math.max(max.z, z);
  }
  return { mesh, min, max };
}

/** True when the ray meets the box within [0, maxT] (slab test). */
function slab(o: Vec3, d: Vec3, min: Vec3, max: Vec3, maxT: number): boolean {
  let t0 = 0;
  let t1 = maxT;
  for (const k of ["x", "y", "z"] as const) {
    const inv = 1 / d[k];
    let a = (min[k] - o[k]) * inv;
    let b = (max[k] - o[k]) * inv;
    if (a > b) [a, b] = [b, a];
    if (Number.isNaN(a) || Number.isNaN(b)) {
      if (o[k] < min[k] || o[k] > max[k]) return false;
      continue;
    }
    t0 = Math.max(t0, a);
    t1 = Math.min(t1, b);
    if (t0 > t1) return false;
  }
  return true;
}

export interface RayHit {
  /** Distance along the (unit) ray. */
  t: number;
  /** Geometric normal of the hit triangle, facing the ray origin. */
  normal: Vec3;
}

/** Nearest triangle hit (both faces) within `maxT`, Möller–Trumbore. `dir` must be unit length. */
export function raycast(meshes: Iterable<RayMesh>, origin: Vec3, dir: Vec3, maxT: number): RayHit | null {
  let best: RayHit | null = null;
  let limit = maxT;
  for (const rm of meshes) {
    if (!slab(origin, dir, rm.min, rm.max, limit)) continue;
    const p = rm.mesh.positions;
    const idx = rm.mesh.indices;
    for (let i = 0; i + 2 < idx.length; i += 3) {
      const a = 3 * (idx[i] ?? 0), b = 3 * (idx[i + 1] ?? 0), c = 3 * (idx[i + 2] ?? 0);
      const ax = p[a] ?? 0, ay = p[a + 1] ?? 0, az = p[a + 2] ?? 0;
      const e1x = (p[b] ?? 0) - ax, e1y = (p[b + 1] ?? 0) - ay, e1z = (p[b + 2] ?? 0) - az;
      const e2x = (p[c] ?? 0) - ax, e2y = (p[c + 1] ?? 0) - ay, e2z = (p[c + 2] ?? 0) - az;
      const px = dir.y * e2z - dir.z * e2y, py = dir.z * e2x - dir.x * e2z, pz = dir.x * e2y - dir.y * e2x;
      const det = e1x * px + e1y * py + e1z * pz;
      if (Math.abs(det) < 1e-12) continue;
      const inv = 1 / det;
      const sx = origin.x - ax, sy = origin.y - ay, sz = origin.z - az;
      const u = (sx * px + sy * py + sz * pz) * inv;
      if (u < 0 || u > 1) continue;
      const qx = sy * e1z - sz * e1y, qy = sz * e1x - sx * e1z, qz = sx * e1y - sy * e1x;
      const v = (dir.x * qx + dir.y * qy + dir.z * qz) * inv;
      if (v < 0 || u + v > 1) continue;
      const t = (e2x * qx + e2y * qy + e2z * qz) * inv;
      if (t <= 1e-6 || t >= limit) continue;
      let nx = e1y * e2z - e1z * e2y, ny = e1z * e2x - e1x * e2z, nz = e1x * e2y - e1y * e2x;
      const len = Math.hypot(nx, ny, nz) || 1;
      nx /= len;
      ny /= len;
      nz /= len;
      if (nx * dir.x + ny * dir.y + nz * dir.z > 0) {
        nx = -nx;
        ny = -ny;
        nz = -nz;
      }
      limit = t;
      best = { t, normal: { x: nx, y: ny, z: nz } };
    }
  }
  return best;
}

/** `n` unit directions spread evenly over the sphere (Fibonacci lattice), deterministic. */
export function sphereDirections(n: number): Vec3[] {
  const out: Vec3[] = [];
  const golden = Math.PI * (3 - Math.sqrt(5));
  for (let i = 0; i < n; i++) {
    const y = 1 - (2 * (i + 0.5)) / n;
    const r = Math.sqrt(Math.max(0, 1 - y * y));
    const a = golden * i;
    out.push({ x: Math.cos(a) * r, y, z: Math.sin(a) * r });
  }
  return out;
}

/** One probe ray from the pose origin: its direction and hit (null = nothing within range). */
export interface Probe {
  dir: Vec3;
  hit: RayHit | null;
}

export function probeAround(meshes: readonly RayMesh[], origin: Vec3, dirs: readonly Vec3[], maxT: number): Probe[] {
  return dirs.map((dir) => ({ dir, hit: raycast(meshes, origin, dir, maxT) }));
}

/**
 * Close-up: the probe whose wall is nearest to `want` u (at least `minT`)
 * and faces the ray most directly. Ties keep the earlier probe, so the pick
 * is deterministic. Null when no probe hits.
 */
export function pickCloseup(probes: readonly Probe[], want: number, minT: number): Probe | null {
  let best: Probe | null = null;
  let bestScore = Infinity;
  for (const p of probes) {
    if (!p.hit || p.hit.t < minT) continue;
    const facing = -(p.hit.normal.x * p.dir.x + p.hit.normal.y * p.dir.y + p.hit.normal.z * p.dir.z);
    const score = Math.abs(p.hit.t - want) + 4 * (1 - facing);
    if (score < bestScore) {
      bestScore = score;
      best = p;
    }
  }
  return best;
}

/**
 * Corner (M4 A2): two neighbouring probes (directions at most `maxAngle`
 * radians apart) whose hits lie in [lo, hi] u and whose rock faces turn by
 * at least 60° against each other: an edge or corner of the rock between
 * them. Prefers the sharpest turn, then hits nearest `(lo + hi) / 2`; ties
 * keep the earlier pair. Returns the offset from the probes' origin to the
 * point midway between the two hits (the camera looks at it), or null when
 * no pair qualifies.
 */
export function pickCorner(probes: readonly Probe[], lo: number, hi: number, maxAngle = 0.35): Vec3 | null {
  const cosMax = Math.cos(maxAngle);
  const mid = (lo + hi) / 2;
  let best: Vec3 | null = null;
  let bestScore = Infinity;
  for (let i = 0; i < probes.length; i++) {
    const a = probes[i]!;
    if (!a.hit || a.hit.t < lo || a.hit.t > hi) continue;
    for (let j = i + 1; j < probes.length; j++) {
      const b = probes[j]!;
      if (!b.hit || b.hit.t < lo || b.hit.t > hi) continue;
      if (a.dir.x * b.dir.x + a.dir.y * b.dir.y + a.dir.z * b.dir.z < cosMax) continue;
      const turn = a.hit.normal.x * b.hit.normal.x + a.hit.normal.y * b.hit.normal.y + a.hit.normal.z * b.hit.normal.z;
      if (turn > 0.5) continue;
      const score = turn + 0.05 * (Math.abs(a.hit.t - mid) + Math.abs(b.hit.t - mid));
      if (score < bestScore) {
        bestScore = score;
        best = {
          x: (a.dir.x * a.hit.t + b.dir.x * b.hit.t) / 2,
          y: (a.dir.y * a.hit.t + b.dir.y * b.hit.t) / 2,
          z: (a.dir.z * a.hit.t + b.dir.z * b.hit.t) / 2,
        };
      }
    }
  }
  return best;
}

/**
 * Longest view: the probe that travels furthest before hitting rock (a miss
 * counts as `maxT`), preferring near-horizontal directions (|y| ≤ `maxSlope`)
 * so the frame keeps a readable horizon. Null for no probes.
 */
export function pickLongest(probes: readonly Probe[], maxT: number, maxSlope = 0.5): Probe | null {
  let best: Probe | null = null;
  let bestT = -1;
  for (const p of probes) {
    if (Math.abs(p.dir.y) > maxSlope) continue;
    const t = p.hit ? p.hit.t : maxT;
    if (t > bestT) {
      bestT = t;
      best = p;
    }
  }
  return best ?? probes[0] ?? null;
}

/**
 * Vista: the near-horizontal probe whose hit lies in [lo, hi] closest to
 * the middle of that range (islands or a far wall in view). Falls back to
 * the longest view.
 */
export function pickVista(probes: readonly Probe[], lo: number, hi: number, maxSlope = 0.5): Probe | null {
  const mid = (lo + hi) / 2;
  let best: Probe | null = null;
  let bestScore = Infinity;
  for (const p of probes) {
    if (!p.hit || Math.abs(p.dir.y) > maxSlope || p.hit.t < lo || p.hit.t > hi) continue;
    const score = Math.abs(p.hit.t - mid);
    if (score < bestScore) {
      bestScore = score;
      best = p;
    }
  }
  return best ?? pickLongest(probes, hi, maxSlope);
}
