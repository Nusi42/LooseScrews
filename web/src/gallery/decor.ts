// Gallery terrain decor and the material-border pose (VQ3b). The gallery runs
// offline practice without shots, so paint and surface states are put on
// the chunk meshes directly: spheres of vertices get a paint team and effect
// colour or surface state bits, the way the sim's paint and state edits mark
// mesh vertices. Pure functions on MeshData; gallery.ts applies them after
// it picked the camera pose.

import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import {
  NO_TEAM,
  PAINT_PLAIN,
  PAINT_RED,
  PAINT_YELLOW,
  SURFACE_CHARGED,
  SURFACE_GLOW,
  SURFACE_HOT,
  SURFACE_SCORCH,
  type MeshData,
} from "../renderer/mesh.ts";
import type { Probe, RayMesh } from "./raycast.ts";

/** What a scene paints onto the terrain around the point the camera looks at. */
export type SceneDecor = "none" | "paint" | "states" | "teams4";

/** One sphere of decor: vertices within `radius` of `center` get the marks. */
export interface DecorSpot {
  center: Vec3;
  radius: number;
  /** Paint team (0-based) or NO_TEAM to leave the paint. */
  team: number;
  /** Paint effect colour (PAINT_*), with `team`. */
  color: number;
  /** Surface state bits to set (SURFACE_*). */
  surface: number;
}

/**
 * Decor spots around `target` on a wall with normal `normal`: "paint" = a
 * plain team-0 blob, a red (stripes) and a yellow (dots) team-1 blob
 * overlapping it; "states" = hot, charged, glowing and scorched patches.
 */
export function decorSpots(decor: SceneDecor, target: Vec3, normal: Vec3): DecorSpot[] {
  const up0 = Math.abs(normal.y) > 0.9 ? { x: 1, y: 0, z: 0 } : { x: 0, y: 1, z: 0 };
  const side = v3.normalize(v3.cross(normal, up0));
  const up = v3.cross(side, normal);
  const at = (s: number, u: number): Vec3 => v3.add(target, v3.add(v3.scale(side, s), v3.scale(up, u)));
  const spot = (center: Vec3, radius: number, team: number, color: number, surface = 0): DecorSpot => ({
    center,
    radius,
    team,
    color,
    surface,
  });
  if (decor === "paint") {
    return [
      spot(at(-1.6, 0.2), 1.9, 0, PAINT_PLAIN),
      spot(at(1.3, 0.7), 1.6, 1, PAINT_RED),
      spot(at(0.2, -1.7), 1.4, 1, PAINT_YELLOW),
    ];
  }
  if (decor === "teams4") {
    // M3 C1: plain paint of all four teams side by side.
    return [
      spot(at(-2.4, 0.9), 1.5, 0, PAINT_PLAIN),
      spot(at(-0.8, -0.6), 1.5, 1, PAINT_PLAIN),
      spot(at(0.8, 0.9), 1.5, 2, PAINT_PLAIN),
      spot(at(2.4, -0.6), 1.5, 3, PAINT_PLAIN),
    ];
  }
  if (decor === "states") {
    return [
      spot(at(-1.7, 0.4), 1.4, NO_TEAM, 0, SURFACE_HOT),
      spot(at(1.5, 0.6), 1.3, NO_TEAM, 0, SURFACE_CHARGED),
      spot(at(-0.2, 2.6), 1.4, NO_TEAM, 0, SURFACE_GLOW),
      spot(at(0.1, -1.6), 1.5, NO_TEAM, 0, SURFACE_SCORCH),
    ];
  }
  return [];
}

/** A copy of `mesh` with the spots applied (later spots paint over earlier ones); `mesh` itself is unchanged. */
export function applyDecor(mesh: MeshData, spots: readonly DecorSpot[]): MeshData {
  if (spots.length === 0) return mesh;
  const n = mesh.team.length;
  const team = mesh.team.slice();
  const colors = mesh.paintColors && mesh.paintColors.length === n ? mesh.paintColors.slice() : new Uint8Array(n);
  const surface = mesh.surface && mesh.surface.length === n ? mesh.surface.slice() : new Uint8Array(n);
  const p = mesh.positions;
  let touched = false;
  for (let i = 0; i < n; i++) {
    const pos = { x: p[i * 3] ?? 0, y: p[i * 3 + 1] ?? 0, z: p[i * 3 + 2] ?? 0 };
    for (const s of spots) {
      if (v3.length(v3.sub(pos, s.center)) > s.radius) continue;
      touched = true;
      if (s.team !== NO_TEAM) {
        team[i] = s.team;
        colors[i] = s.color;
      }
      surface[i] = (surface[i] ?? 0) | s.surface;
    }
  }
  return touched ? { ...mesh, team, paintColors: colors, surface } : mesh;
}

/**
 * Material border: among the probe hits `lo`..`hi` u away, the one with
 * the most rock materials in the vertices within `radius` of the hit (the
 * least common one counted, so a real border, not a stray vertex), facing
 * the ray. Ties keep the earlier probe. Null when no hit shows two materials.
 */
export function pickBorder(
  probes: readonly Probe[],
  meshes: readonly RayMesh[],
  origin: Vec3,
  lo = 4,
  hi = 14,
  radius = 2.5,
): Probe | null {
  let best: Probe | null = null;
  let bestScore = 0;
  for (const pr of probes) {
    if (!pr.hit || pr.hit.t < lo || pr.hit.t > hi) continue;
    const c = v3.addScaled(origin, pr.dir, pr.hit.t);
    const counts = [0, 0, 0];
    for (const rm of meshes) {
      const m = rm.mesh.materials;
      if (!m || m.length === 0) continue;
      if (c.x < rm.min.x - radius || c.x > rm.max.x + radius) continue;
      if (c.y < rm.min.y - radius || c.y > rm.max.y + radius) continue;
      if (c.z < rm.min.z - radius || c.z > rm.max.z + radius) continue;
      const p = rm.mesh.positions;
      for (let i = 0; i < m.length; i++) {
        const dx = (p[i * 3] ?? 0) - c.x, dy = (p[i * 3 + 1] ?? 0) - c.y, dz = (p[i * 3 + 2] ?? 0) - c.z;
        if (dx * dx + dy * dy + dz * dz > radius * radius) continue;
        const k = m[i] ?? 1;
        if (k <= 2) counts[k] = (counts[k] ?? 0) + 1;
      }
    }
    const present = counts.filter((x) => x > 0);
    if (present.length < 2) continue;
    const facing = -(pr.hit.normal.x * pr.dir.x + pr.hit.normal.y * pr.dir.y + pr.hit.normal.z * pr.dir.z);
    const score = present.length * 1000 + Math.min(...present) * facing;
    if (score > bestScore) {
      bestScore = score;
      best = pr;
    }
  }
  return best;
}
