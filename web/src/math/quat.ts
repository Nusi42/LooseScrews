// Unit quaternion helpers. `mul(a, b)` = rotate by b, then by a (same as ls-math).

import type { Vec3 } from "./vec3.ts";

export interface Quat {
  x: number;
  y: number;
  z: number;
  w: number;
}

export const IDENTITY: Readonly<Quat> = Object.freeze({ x: 0, y: 0, z: 0, w: 1 });

export function identity(): Quat {
  return { x: 0, y: 0, z: 0, w: 1 };
}

/** Rotation of `angle` radians around the (unit) axis. */
export function fromAxisAngle(axis: Vec3, angle: number): Quat {
  const h = angle * 0.5;
  const s = Math.sin(h);
  return { x: axis.x * s, y: axis.y * s, z: axis.z * s, w: Math.cos(h) };
}

export function mul(a: Quat, b: Quat): Quat {
  return {
    x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
    y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
    z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
    w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
  };
}

export function conjugate(q: Quat): Quat {
  return { x: -q.x, y: -q.y, z: -q.z, w: q.w };
}

export function normalize(q: Quat): Quat {
  const len = Math.hypot(q.x, q.y, q.z, q.w);
  if (len < 1e-12) return identity();
  const inv = 1 / len;
  return { x: q.x * inv, y: q.y * inv, z: q.z * inv, w: q.w * inv };
}

/** Rotates vector v by unit quaternion q. */
export function rotate(q: Quat, v: Vec3): Vec3 {
  // t = 2 * cross(q.xyz, v); v' = v + w * t + cross(q.xyz, t)
  const tx = 2 * (q.y * v.z - q.z * v.y);
  const ty = 2 * (q.z * v.x - q.x * v.z);
  const tz = 2 * (q.x * v.y - q.y * v.x);
  return {
    x: v.x + q.w * tx + (q.y * tz - q.z * ty),
    y: v.y + q.w * ty + (q.z * tx - q.x * tz),
    z: v.z + q.w * tz + (q.x * ty - q.y * tx),
  };
}

/** Local -Z axis in world space. */
export function forward(q: Quat): Vec3 {
  return rotate(q, { x: 0, y: 0, z: -1 });
}

/** Local +X axis in world space. */
export function right(q: Quat): Vec3 {
  return rotate(q, { x: 1, y: 0, z: 0 });
}

/** Local +Y axis in world space. */
export function up(q: Quat): Vec3 {
  return rotate(q, { x: 0, y: 1, z: 0 });
}

/**
 * Orientation whose forward (-Z) points along `fwd` and whose up is as close
 * to `upHint` as possible. Both need not be normalised; they must not be parallel.
 */
export function lookRotation(fwd: Vec3, upHint: Vec3): Quat {
  const f = norm3(fwd);
  // right = forward x up, up = right x forward (right-handed, forward = -Z)
  const r = norm3(cross3(f, upHint));
  const u = cross3(r, f);
  // Columns of the rotation matrix: right (+X), up (+Y), back (+Z = -forward).
  const m00 = r.x, m01 = u.x, m02 = -f.x;
  const m10 = r.y, m11 = u.y, m12 = -f.y;
  const m20 = r.z, m21 = u.z, m22 = -f.z;
  const trace = m00 + m11 + m22;
  let x: number, y: number, z: number, w: number;
  if (trace > 0) {
    const s = Math.sqrt(trace + 1) * 2;
    w = 0.25 * s;
    x = (m21 - m12) / s;
    y = (m02 - m20) / s;
    z = (m10 - m01) / s;
  } else if (m00 > m11 && m00 > m22) {
    const s = Math.sqrt(1 + m00 - m11 - m22) * 2;
    w = (m21 - m12) / s;
    x = 0.25 * s;
    y = (m01 + m10) / s;
    z = (m02 + m20) / s;
  } else if (m11 > m22) {
    const s = Math.sqrt(1 + m11 - m00 - m22) * 2;
    w = (m02 - m20) / s;
    x = (m01 + m10) / s;
    y = 0.25 * s;
    z = (m12 + m21) / s;
  } else {
    const s = Math.sqrt(1 + m22 - m00 - m11) * 2;
    w = (m10 - m01) / s;
    x = (m02 + m20) / s;
    y = (m12 + m21) / s;
    z = 0.25 * s;
  }
  return normalize({ x, y, z, w });
}

function cross3(a: Vec3, b: Vec3): Vec3 {
  return { x: a.y * b.z - a.z * b.y, y: a.z * b.x - a.x * b.z, z: a.x * b.y - a.y * b.x };
}

function norm3(a: Vec3): Vec3 {
  const len = Math.hypot(a.x, a.y, a.z) || 1;
  return { x: a.x / len, y: a.y / len, z: a.z / len };
}
