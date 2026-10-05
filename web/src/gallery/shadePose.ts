// Gallery pose for the sun occlusion scenes (VQ9): a camera standing in
// rock's shadow that looks at rock the sun reaches through an opening, with
// as much sunlit air between them as possible (the volumetric shaft). CPU ray
// casts against the chunk meshes (raycast.ts), deterministic per world.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import * as v3 from "../math/vec3.ts";
import type { Vec3 } from "../math/vec3.ts";
import { raycast, sphereDirections, type Probe, type RayMesh } from "./raycast.ts";

/** Longest sun ray (u): beyond the 256 x 128 x 256 level's diagonal. */
const SUN_RAY = 420;

export interface ShadeOptions {
  /** Camera candidates tried (spawn, then points along the probes), at most. */
  maxCameras: number;
  /** View directions tried per camera. */
  directions: number;
  /** Target distance range (u). */
  near: number;
  far: number;
}

export const SHADE_OPTIONS: ShadeOptions = { maxCameras: 18, directions: 48, near: 10, far: 70 };

/** True when nothing blocks the way from `p` toward the sun. */
export function sunlit(meshes: readonly RayMesh[], p: Vec3, sun: Vec3): boolean {
  return raycast(meshes, p, sun, SUN_RAY) === null;
}

/**
 * Sunlit wall pose (D5, ship shadows): a sunlit camera looking at sunlit
 * rock that faces the sun, 6-14 u away, with the sun from the side and a
 * little behind, so hulls hovering in front of the rock throw their
 * shadows onto it beside them. Null
 * when no such view exists (the caller falls back to the plain pose).
 */
export function pickSunWallPose(
  meshes: readonly RayMesh[],
  probes: readonly Probe[],
  origin: Vec3,
  sun: Vec3,
  room: { min: Vec3; max: Vec3 } | null,
): { position: Vec3; orientation: Quat } | null {
  const up = { x: 0, y: 1, z: 0 };
  const inside = (p: Vec3): boolean =>
    !room || (p.x > room.min.x + 3 && p.y > room.min.y + 3 && p.z > room.min.z + 3 && p.x < room.max.x - 3 && p.y < room.max.y - 3 && p.z < room.max.z - 3);
  const cams: Vec3[] = [origin];
  for (const p of probes) {
    if (p.hit && p.hit.t > 10) cams.push(v3.addScaled(origin, p.dir, p.hit.t * 0.5));
  }
  // Toward the sun only up to the room box: the hidden walls of open levels are in the meshes too.
  const lit = (p: Vec3): boolean => {
    let t = SUN_RAY;
    if (room) {
      for (const a of ["x", "y", "z"] as const) {
        if (sun[a] > 1e-6) t = Math.min(t, (room.max[a] - p[a]) / sun[a]);
        else if (sun[a] < -1e-6) t = Math.min(t, (room.min[a] - p[a]) / sun[a]);
      }
    }
    return raycast(meshes, p, sun, Math.max(0, t - 1)) === null;
  };
  const dirs = sphereDirections(64);
  let best: { score: number; position: Vec3; dir: Vec3 } | null = null;
  let tried = 0;
  for (const cam of cams) {
    if (tried >= 24) break;
    if (!inside(cam) || !lit(cam)) continue;
    if (!sphereDirections(12).every((d) => (raycast(meshes, cam, d, 2) ?? { t: 3 }).t > 1.8)) continue;
    tried++;
    for (const dir of dirs) {
      if (v3.dot(dir, sun) > 0.1 || Math.abs(dir.y) > 0.85) continue;
      const hit = raycast(meshes, cam, dir, 14);
      if (!hit || hit.t < 6 || v3.dot(hit.normal, sun) < 0.4 || v3.dot(hit.normal, dir) > -0.5) continue;
      if (!lit(v3.addScaled(v3.addScaled(cam, dir, hit.t), hit.normal, 0.3))) continue;
      // The sun from the side (shadows fall beside the hulls, not behind them), the rock facing it.
      const score = v3.dot(hit.normal, sun) - Math.abs(v3.dot(dir, sun) + 0.3) - Math.abs(hit.t - 9) * 0.05;
      if (!best || score > best.score) best = { score, position: cam, dir };
    }
  }
  if (!best) return null;
  return { position: best.position, orientation: q.lookRotation(best.dir, Math.abs(best.dir.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up) };
}

/**
 * Shade pose: null when no camera in shadow sees sunlit rock (the caller
 * falls back to the plain pose).
 */
export function pickShadePose(
  meshes: readonly RayMesh[],
  probes: readonly Probe[],
  origin: Vec3,
  sun: Vec3,
  room: { min: Vec3; max: Vec3 } | null,
  opts: ShadeOptions = SHADE_OPTIONS,
): { position: Vec3; orientation: Quat } | null {
  const up = { x: 0, y: 1, z: 0 };
  const inside = (p: Vec3): boolean =>
    !room || (p.x > room.min.x + 3 && p.y > room.min.y + 3 && p.z > room.min.z + 3 && p.x < room.max.x - 3 && p.y < room.max.y - 3 && p.z < room.max.z - 3);
  const cams: Vec3[] = [origin];
  for (const p of probes) {
    if (p.hit && p.hit.t > 10) cams.push(v3.addScaled(origin, p.dir, p.hit.t * 0.5));
  }
  const dirs = sphereDirections(opts.directions).filter((d) => Math.abs(d.y) < 0.8);
  let best: { score: number; position: Vec3; dir: Vec3 } | null = null;
  let tried = 0;
  for (const cam of cams) {
    if (tried >= opts.maxCameras) break;
    if (!inside(cam) || sunlit(meshes, cam, sun)) continue;
    if (!sphereDirections(12).every((d) => (raycast(meshes, cam, d, 2) ?? { t: 3 }).t > 1.8)) continue;
    tried++;
    for (const dir of dirs) {
      const hit = raycast(meshes, cam, dir, opts.far);
      if (!hit || hit.t < opts.near || v3.dot(hit.normal, sun) < 0.3) continue;
      const p = v3.addScaled(v3.addScaled(cam, dir, hit.t), hit.normal, 0.3);
      if (!sunlit(meshes, p, sun)) continue;
      // Sunlit air along the view: the shaft.
      let air = 0;
      for (const k of [0.3, 0.5, 0.7, 0.85]) if (sunlit(meshes, v3.addScaled(cam, dir, hit.t * k), sun)) air++;
      const score = air + 0.75 * Math.max(0, v3.dot(dir, sun)) + 0.5 * v3.dot(hit.normal, sun);
      if (!best || score > best.score) best = { score, position: cam, dir };
    }
    if (best && best.score >= 3.5) break;
  }
  if (!best) return null;
  // Look a little above the lit spot, so the opening and the shaft show.
  const dir = v3.normalize(v3.add(best.dir, v3.scale(sun, 0.25)));
  return { position: best.position, orientation: q.lookRotation(dir, Math.abs(dir.y) > 0.95 ? { x: 1, y: 0, z: 0 } : up) };
}
