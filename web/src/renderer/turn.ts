// The level's turned up (M4 B5, Bored level: docs/milestone-4-plan.md §3.2).
// The sim keeps terrain in place and turns `World::up` 90 degrees about a
// seeded horizontal axis; the client turns everything that hangs off the
// level's up with it: the sky (sun, nebula, planet, stars), the sun's light
// and shadows, and the height fog. The turn is the shortest rotation taking
// +Y to the level's up (the sim's turn is about an axis perpendicular to
// both, so this is the same rotation).

import * as q from "../math/quat.ts";
import type { Quat } from "../math/quat.ts";
import type { Vec3 } from "../math/vec3.ts";

const Y: Vec3 = { x: 0, y: 1, z: 0 };

/** Shortest rotation taking +Y to `up` (identity for +Y or a zero vector). */
export function turnFromUp(up: Vec3): Quat {
  const len = Math.hypot(up.x, up.y, up.z);
  if (!(len > 1e-6)) return q.identity();
  const u = { x: up.x / len, y: up.y / len, z: up.z / len };
  const d = u.y; // dot(Y, u)
  if (d > 1 - 1e-9) return q.identity();
  if (d < -1 + 1e-9) return q.fromAxisAngle({ x: 1, y: 0, z: 0 }, Math.PI);
  // Half-way quaternion: (cross(Y, u), 1 + dot(Y, u)), normalised.
  const c = { x: Y.y * u.z - Y.z * u.y, y: Y.z * u.x - Y.x * u.z, z: Y.x * u.y - Y.y * u.x };
  return q.normalize({ x: c.x, y: c.y, z: c.z, w: 1 + d });
}

/** Angle (rad) between two rotations. */
export function turnAngle(a: Quat, b: Quat): number {
  const dot = Math.abs(a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w);
  return 2 * Math.acos(Math.min(1, dot));
}

/**
 * Frame uniform floats of the turn (shaders/frame.ts `up`, `sky_turn`): the
 * up with the height offset that keeps the level centre's height (height(p)
 * = dot(p, up) + w, which is p.y while up is +Y), and the quaternion from
 * world directions into the sky's own (baked, +Y up) frame.
 */
export function packTurn(turn: Quat, center: Vec3, out: Float32Array, offset: number): void {
  const up = q.rotate(turn, Y);
  const w = center.y - (center.x * up.x + center.y * up.y + center.z * up.z);
  out.set([up.x, up.y, up.z, w], offset);
  const inv = q.conjugate(turn);
  out.set([inv.x, inv.y, inv.z, inv.w], offset + 4);
}
