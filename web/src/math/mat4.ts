// Column-major 4x4 matrices (WGSL `mat4x4<f32>` layout).
// Element (row r, column c) lives at index c * 4 + r.

import type { Quat } from "./quat.ts";
import type { Vec3 } from "./vec3.ts";

export type Mat4 = Float32Array;

export function create(): Mat4 {
  const m = new Float32Array(16);
  m[0] = m[5] = m[10] = m[15] = 1;
  return m;
}

/** out = a * b (apply b first, then a). `out` may alias neither input. */
export function multiply(a: Mat4, b: Mat4, out: Mat4 = new Float32Array(16)): Mat4 {
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += (a[k * 4 + r] ?? 0) * (b[c * 4 + k] ?? 0);
      out[c * 4 + r] = s;
    }
  }
  return out;
}

/**
 * Right-handed perspective projection mapping view-space depth to WebGPU's
 * [0, 1] clip range (near -> 0, far -> 1). The camera looks down -Z.
 */
export function perspective(fovY: number, aspect: number, near: number, far: number): Mat4 {
  const f = 1 / Math.tan(fovY / 2);
  const m = new Float32Array(16);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = far / (near - far);
  m[11] = -1;
  m[14] = (near * far) / (near - far);
  return m;
}

/** View matrix (world -> camera) for a camera at `pos` with orientation `q`. */
export function viewFromPose(pos: Vec3, q: Quat): Mat4 {
  // Inverse rotation = conjugate; rotation matrix of conj(q) is the transpose of R(q).
  const { x, y, z, w } = q;
  const xx = x * x, yy = y * y, zz = z * z;
  const xy = x * y, xz = x * z, yz = y * z;
  const wx = w * x, wy = w * y, wz = w * z;
  // R(q) rows
  const r00 = 1 - 2 * (yy + zz), r01 = 2 * (xy - wz), r02 = 2 * (xz + wy);
  const r10 = 2 * (xy + wz), r11 = 1 - 2 * (xx + zz), r12 = 2 * (yz - wx);
  const r20 = 2 * (xz - wy), r21 = 2 * (yz + wx), r22 = 1 - 2 * (xx + yy);
  const m = new Float32Array(16);
  // Column-major storage of R^T: element (r, c) = R(c, r).
  m[0] = r00; m[4] = r10; m[8] = r20;
  m[1] = r01; m[5] = r11; m[9] = r21;
  m[2] = r02; m[6] = r12; m[10] = r22;
  // translation = -R^T * pos
  m[12] = -(r00 * pos.x + r10 * pos.y + r20 * pos.z);
  m[13] = -(r01 * pos.x + r11 * pos.y + r21 * pos.z);
  m[14] = -(r02 * pos.x + r12 * pos.y + r22 * pos.z);
  m[15] = 1;
  return m;
}

/** Transforms a point (w = 1) and returns clip-space [x, y, z, w]. */
export function transformPoint(m: Mat4, p: Vec3): [number, number, number, number] {
  const e = (i: number): number => m[i] ?? 0;
  return [
    e(0) * p.x + e(4) * p.y + e(8) * p.z + e(12),
    e(1) * p.x + e(5) * p.y + e(9) * p.z + e(13),
    e(2) * p.x + e(6) * p.y + e(10) * p.z + e(14),
    e(3) * p.x + e(7) * p.y + e(11) * p.z + e(15),
  ];
}
