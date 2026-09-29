// Decodes ls-wasm's flat state arrays (layouts in wasm/api.ts) into poses and
// renderer instances.

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";
import type { Instance, Rgb } from "../renderer/renderer.ts";
import { POSE_LEN, PROJECTILE_STRIDE, SHIP_STRIDE } from "../wasm/api.ts";

export interface LocalPose {
  position: Vec3;
  orientation: Quat;
  velocity: Vec3;
  alive: boolean;
}

export interface ShipState {
  id: number;
  team: number;
  alive: boolean;
  boosting: boolean;
  position: Vec3;
  orientation: Quat;
}

export interface ProjectileState {
  kind: number;
  owner: number;
  position: Vec3;
  velocity: Vec3;
}

/** Hull colour per 0-based team (a darker shade of the paint colour). */
export const TEAM_HULL: readonly Rgb[] = [
  [0.95, 0.5, 0.2],
  [0.25, 0.6, 0.95],
];

/** Projectile colour and size by weapon string id. */
export const PROJECTILE_STYLE: Readonly<Record<string, { color: Rgb; scale: number }>> = {
  scrap_laser: { color: [1.0, 0.35, 0.2], scale: 1.2 },
  rivet_gun: { color: [0.95, 0.9, 0.7], scale: 0.5 },
  bolt_rocket: { color: [1.0, 0.8, 0.3], scale: 1.6 },
  // Paint blobs take the shooter's team colour (see projectileInstances).
  paint_thrower: { color: [1, 1, 1], scale: 0.9 },
};
const DEFAULT_STYLE = { color: [1, 1, 1] as Rgb, scale: 1 };

function num(a: ArrayLike<number>, i: number): number {
  return a[i] ?? 0;
}

function vec(a: ArrayLike<number>, i: number): Vec3 {
  return { x: num(a, i), y: num(a, i + 1), z: num(a, i + 2) };
}

function quat(a: ArrayLike<number>, i: number): Quat {
  return q.normalize({ x: num(a, i), y: num(a, i + 1), z: num(a, i + 2), w: num(a, i + 3) });
}

export function parsePose(a: ArrayLike<number> | undefined): LocalPose | null {
  if (!a || a.length < POSE_LEN) return null;
  return { position: vec(a, 0), orientation: quat(a, 3), velocity: vec(a, 7), alive: num(a, 10) !== 0 };
}

export function parseShips(a: ArrayLike<number>): ShipState[] {
  const out: ShipState[] = [];
  for (let i = 0; i + SHIP_STRIDE <= a.length; i += SHIP_STRIDE) {
    out.push({
      id: num(a, i),
      team: num(a, i + 1),
      alive: num(a, i + 2) !== 0,
      boosting: num(a, i + 3) !== 0,
      position: vec(a, i + 4),
      orientation: quat(a, i + 7),
    });
  }
  return out;
}

export function parseProjectiles(a: ArrayLike<number>): ProjectileState[] {
  const out: ProjectileState[] = [];
  for (let i = 0; i + PROJECTILE_STRIDE <= a.length; i += PROJECTILE_STRIDE) {
    out.push({ kind: num(a, i), owner: num(a, i + 1), position: vec(a, i + 2), velocity: vec(a, i + 5) });
  }
  return out;
}

/** Living ships as instances (dead ships are not drawn; wrecks come later). */
export function shipInstances(ships: readonly ShipState[]): Instance[] {
  return ships
    .filter((s) => s.alive)
    .map((s) => ({
      position: s.position,
      rotation: s.orientation,
      scale: 1,
      color: TEAM_HULL[s.team] ?? [0.8, 0.8, 0.8],
      emissive: s.boosting ? 0.35 : 0.1,
    }));
}

/**
 * Projectiles as instances oriented along their velocity. `weaponIds` maps
 * `kind` to a weapon string id; `teamOf` gives a shooter's team (paint blobs
 * use its colour).
 */
export function projectileInstances(
  shots: readonly ProjectileState[],
  weaponIds: readonly string[],
  teamOf: (owner: number) => number | undefined,
): Instance[] {
  return shots.map((p) => {
    const id = weaponIds[p.kind] ?? "";
    const style = PROJECTILE_STYLE[id] ?? DEFAULT_STYLE;
    let color = style.color;
    if (id === "paint_thrower") {
      const team = teamOf(p.owner);
      color = (team !== undefined ? TEAM_HULL[team] : undefined) ?? color;
    }
    const speed = Math.hypot(p.velocity.x, p.velocity.y, p.velocity.z);
    // lookRotation needs an up hint that is not parallel to the direction.
    const hint = speed > 0 && Math.abs(p.velocity.y) > 0.99 * speed ? { x: 1, y: 0, z: 0 } : { x: 0, y: 1, z: 0 };
    const rotation = speed > 1e-6 ? q.lookRotation(p.velocity, hint) : q.identity();
    return { position: p.position, rotation, scale: style.scale, color, emissive: 1 };
  });
}
