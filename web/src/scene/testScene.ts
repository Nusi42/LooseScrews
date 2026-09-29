// Procedural test scene so the renderer and controls can be tried before the
// WASM terrain exists: a looping, partly painted tunnel of box plates, floating
// rocks, a floor and pillars, grouped into 32 u chunks exactly like terrain
// chunks will be (world-space meshes keyed by chunk coordinate).
//
// Not deterministic-critical and not the real world generator (that is
// ls-worldgen); wave 3 replaces this with chunks meshed in WASM.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import { add, cross, normalize as norm, scale, sub, type Vec3 } from "../math/vec3.ts";
import { MeshBuilder, NO_TEAM, type MeshData } from "../renderer/mesh.ts";
import { appendBox, appendOrientedBox, type FaceTeam, type V3 } from "../renderer/primitives.ts";

/** Chunk edge length in u (32 cells of 1 u, matching ls-worldgen's 32^3 chunks). */
export const CHUNK_SIZE = 32;
/** Default level bounds from the plan (256 x 128 x 256 u), origin at 0. */
export const LEVEL_SIZE: Readonly<Vec3> = { x: 256, y: 128, z: 256 };

export interface TestSceneOptions {
  seed: number;
  tunnelRadius: number;
  ringSpacing: number;
  platesPerRing: number;
  rocks: number;
  pillars: number;
}

export const DEFAULT_SCENE: TestSceneOptions = {
  seed: 0x5eed,
  tunnelRadius: 7,
  ringSpacing: 5,
  platesPerRing: 8,
  rocks: 160,
  pillars: 20,
};

export interface TestScene {
  /** Chunk meshes keyed "cx,cy,cz". */
  chunks: Map<string, MeshData>;
  spawnPosition: Vec3;
  spawnOrientation: Quat;
  /** Point on the tunnel centre line, t in [0, 1). */
  pathPoint(t: number): Vec3;
  /** Unit tangent of the centre line at t. */
  pathTangent(t: number): Vec3;
  /** Painted share of all faces per 0-based team id. */
  paintShare: number[];
  boxCount: number;
}

/** Small seeded PRNG (mulberry32); returns floats in [0, 1). */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function chunkKey(cx: number, cy: number, cz: number): string {
  return `${cx},${cy},${cz}`;
}

/** Chunk coordinate containing world point p. */
export function chunkOf(p: Vec3): [number, number, number] {
  return [Math.floor(p.x / CHUNK_SIZE), Math.floor(p.y / CHUNK_SIZE), Math.floor(p.z / CHUNK_SIZE)];
}

const TAU = Math.PI * 2;

export function generateTestScene(options: Partial<TestSceneOptions> = {}): TestScene {
  const opt = { ...DEFAULT_SCENE, ...options };
  const rng = mulberry32(opt.seed);
  const builders = new Map<string, MeshBuilder>();
  // Face counts for team 0, team 1 and unpainted.
  const faceCounts = [0, 0, 0];
  let boxCount = 0;

  const builderAt = (c: V3): MeshBuilder => {
    const [cx, cy, cz] = chunkOf({ x: c[0], y: c[1], z: c[2] });
    const key = chunkKey(cx, cy, cz);
    let b = builders.get(key);
    if (!b) {
      b = new MeshBuilder();
      builders.set(key, b);
    }
    return b;
  };
  /** Wraps a face-team rule so painted faces are counted for the paint stats. */
  const counted = (team: FaceTeam): FaceTeam => (n) => {
    const t = typeof team === "function" ? team(n) : team;
    faceCounts[t === NO_TEAM ? 2 : Math.min(t, 1)]! += 1;
    return t;
  };

  // --- Tunnel centre line: a tilted closed loop around the level centre.
  const cx = LEVEL_SIZE.x / 2, cy = LEVEL_SIZE.y / 2, cz = LEVEL_SIZE.z / 2;
  const rx = 88, rz = 88, ry = 26;
  const pathPoint = (t: number): Vec3 => {
    const a = t * TAU;
    return { x: cx + rx * Math.cos(a), y: cy + ry * Math.sin(2 * a), z: cz + rz * Math.sin(a) };
  };
  const pathTangent = (t: number): Vec3 => {
    const a = t * TAU;
    const d = { x: -rx * Math.sin(a), y: 2 * ry * Math.cos(2 * a), z: rz * Math.cos(a) };
    const len = Math.hypot(d.x, d.y, d.z);
    return { x: d.x / len, y: d.y / len, z: d.z / len };
  };

  // Approximate loop length for ring spacing.
  let loopLength = 0;
  const SAMPLES = 512;
  for (let i = 0; i < SAMPLES; i++) {
    const a = pathPoint(i / SAMPLES), b = pathPoint((i + 1) / SAMPLES);
    loopLength += Math.hypot(b.x - a.x, b.y - a.y, b.z - a.z);
  }
  const rings = Math.max(8, Math.round(loopLength / opt.ringSpacing));
  const segLen = loopLength / rings;
  const plates = opt.platesPerRing;
  const thick = 0.4;
  const halfWidth = opt.tunnelRadius * Math.tan(Math.PI / plates) + 0.25;
  const halfLen = segLen / 2 + 0.15; // slight overlap hides seams between rings

  for (let r = 0; r < rings; r++) {
    const t = r / rings;
    const p = pathPoint(t);
    const tan = pathTangent(t);
    const right = norm(cross(tan, { x: 0, y: 1, z: 0 }));
    const up = cross(right, tan);
    // Paint: two long team stretches plus random splats.
    let ringTeam = NO_TEAM;
    if (t >= 0.08 && t < 0.22) ringTeam = 0;
    else if (t >= 0.55 && t < 0.7) ringTeam = 1;
    const window = r % 9 === 4; // periodic openings to see out into the cavern

    for (let k = 0; k < plates; k++) {
      if (window && (k === 1 || k === 2 || k === 5)) continue;
      const theta = ((k + 0.5) / plates) * TAU;
      const d = add(scale(right, Math.cos(theta)), scale(up, Math.sin(theta)));
      const e = sub(scale(up, Math.cos(theta)), scale(right, Math.sin(theta)));
      const centre = add(p, scale(d, opt.tunnelRadius + thick));
      let team = ringTeam;
      if (team === NO_TEAM && rng() < 0.06) team = rng() < 0.5 ? 0 : 1;
      // Only the inward-facing side is painted (paint sticks to the surface you shoot).
      const faceTeam = (n: V3): number => (n[0] * d.x + n[1] * d.y + n[2] * d.z < -0.5 ? team : NO_TEAM);
      const c: V3 = [centre.x, centre.y, centre.z];
      appendOrientedBox(
        builderAt(c),
        c,
        v3(scale(d, thick)),
        v3(scale(e, halfWidth)),
        v3(scale(tan, halfLen)),
        counted(faceTeam),
      );
      boxCount++;
    }
  }

  // Coarse distance-to-tunnel test so rocks and pillars never block the loop.
  const pathSamples: Vec3[] = [];
  for (let i = 0; i < 256; i++) pathSamples.push(pathPoint(i / 256));
  const clearOfTunnel = (p: Vec3, margin: number): boolean =>
    pathSamples.every((s) => Math.hypot(s.x - p.x, s.y - p.y, s.z - p.z) > opt.tunnelRadius + margin);

  // --- Floating rocks: randomly oriented boxes.
  let placed = 0;
  for (let attempt = 0; placed < opt.rocks && attempt < opt.rocks * 20; attempt++) {
    const p = { x: 8 + rng() * (LEVEL_SIZE.x - 16), y: 10 + rng() * (LEVEL_SIZE.y - 20), z: 8 + rng() * (LEVEL_SIZE.z - 16) };
    const size = 1 + rng() * 4;
    if (!clearOfTunnel(p, size * 1.8 + 2)) continue;
    const rot = q.normalize({ x: rng() - 0.5, y: rng() - 0.5, z: rng() - 0.5, w: rng() - 0.5 });
    const sx = size * (0.6 + rng()), sy = size * (0.6 + rng()), sz = size * (0.6 + rng());
    const team = rng() < 0.15 ? (rng() < 0.5 ? 0 : 1) : NO_TEAM;
    const c: V3 = [p.x, p.y, p.z];
    appendOrientedBox(
      builderAt(c),
      c,
      v3(scale(q.right(rot), sx)),
      v3(scale(q.up(rot), sy)),
      v3(scale(q.rotate(rot, { x: 0, y: 0, z: 1 }), sz)),
      counted(team),
    );
    placed++;
    boxCount++;
  }

  // --- Floor tiles (one per chunk column) and pillars.
  for (let x = 0; x < LEVEL_SIZE.x; x += CHUNK_SIZE) {
    for (let z = 0; z < LEVEL_SIZE.z; z += CHUNK_SIZE) {
      const c: V3 = [x + CHUNK_SIZE / 2, 1, z + CHUNK_SIZE / 2];
      appendBox(builderAt(c), c[0], c[1], c[2], CHUNK_SIZE / 2, 1, CHUNK_SIZE / 2, counted(NO_TEAM));
      boxCount++;
    }
  }
  placed = 0;
  for (let attempt = 0; placed < opt.pillars && attempt < opt.pillars * 30; attempt++) {
    const px = 12 + rng() * (LEVEL_SIZE.x - 24), pz = 12 + rng() * (LEVEL_SIZE.z - 24);
    const height = 30 + rng() * 70;
    const w = 1.5 + rng() * 2.5;
    // Pillars are split into 16 u sections so each lands in the right chunk.
    const top = 2 + height;
    const sections: { c: V3; halfHeight: number }[] = [];
    for (let y = 2; y < top; y += 16) {
      const h = Math.min(16, top - y);
      sections.push({ c: [px, y + h / 2, pz], halfHeight: h / 2 });
    }
    if (!sections.every(({ c }) => clearOfTunnel({ x: c[0], y: c[1], z: c[2] }, w + 10))) continue;
    for (const { c, halfHeight } of sections) {
      appendBox(builderAt(c), c[0], c[1], c[2], w, halfHeight, w, counted(NO_TEAM));
      boxCount++;
    }
    placed++;
  }

  const chunks = new Map<string, MeshData>();
  for (const [key, b] of builders) chunks.set(key, b.build());
  const totalFaces = faceCounts[0]! + faceCounts[1]! + faceCounts[2]!;
  const share = (t: number): number => (totalFaces > 0 ? faceCounts[t]! / totalFaces : 0);

  const spawnPosition = pathPoint(0);
  const spawnOrientation = q.lookRotation(pathTangent(0), { x: 0, y: 1, z: 0 });
  return {
    chunks,
    spawnPosition,
    spawnOrientation,
    pathPoint,
    pathTangent,
    paintShare: [share(0), share(1)],
    boxCount,
  };
}

function v3(a: Vec3): V3 {
  return [a.x, a.y, a.z];
}
