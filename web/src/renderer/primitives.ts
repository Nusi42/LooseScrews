// Small procedural meshes for instanced objects. All face -Z (ship forward)
// and are about 1 u long, so instance scale 1 = one ship length.

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
